//! `askpass` 的测试：提示的解析、经真实端点的请求与答复、取消和格式错误的消息。

use std::time::Duration;

use super::*;

const HOST_KEY: &str = "The authenticity of host 'h (1.2.3.4)' can't be established.\n\
    ED25519 key fingerprint is SHA256:abc.\n\
    This key is not known by any other names.\n\
    Are you sure you want to continue connecting (yes/no/[fingerprint])? ";

#[test]
fn known_prompts_are_parsed() {
    let parse = |text| Prompt::parse(text, false);
    assert_eq!(
        parse("Username for 'https://github.com': "),
        Prompt::Username {
            server: "https://github.com".into()
        }
    );
    assert_eq!(
        parse("Password for 'https://me@github.com/a.git': "),
        Prompt::Password {
            server: "https://github.com/a.git".into(),
            account: Some("me".into()),
        }
    );
    assert_eq!(
        parse("Enter passphrase for key '/k/id': "),
        Prompt::Passphrase {
            key: "/k/id".into()
        }
    );
    assert_eq!(
        parse("git@host.example's password: "),
        Prompt::SshPassword {
            account: "git".into(),
            host: "host.example".into(),
        }
    );
    assert_eq!(
        parse(HOST_KEY),
        Prompt::HostKey {
            host: "h (1.2.3.4)".into(),
            key_type: "ED25519".into(),
            fingerprint: "SHA256:abc".into(),
        }
    );
}

#[test]
fn unknown_prompts_stay_verbatim_with_a_hidden_input_by_default() {
    let other = |text: &str, input| Prompt::Other {
        text: text.trim_end().into(),
        input,
    };
    let code = "(u@h) Verification code: ";
    assert_eq!(Prompt::parse(code, false), other(code, InputKind::Secret));
    assert_eq!(
        Prompt::parse("Sure?", true),
        other("Sure?", InputKind::None)
    );
    assert_eq!(
        Prompt::parse("Username: ", false),
        other("Username: ", InputKind::Visible)
    );
}

/// 取自本机 OpenSSH 的实际输出：指纹前有冒号，结尾没有句号。
#[test]
fn a_host_key_prompt_from_a_newer_openssh_is_parsed() {
    let text = "The authenticity of host '[127.0.0.1]:53885 ([127.0.0.1]:53885)' can't be \
        established.\nED25519 key fingerprint is: SHA256:EcsfrjDiZE3k2nSxhHEJR+yDlulL0Zn0KsQX+lN75Q0\n\
        This key is not known by any other names.\n\
        Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
    assert_eq!(
        Prompt::parse(text, false),
        Prompt::HostKey {
            host: "[127.0.0.1]:53885 ([127.0.0.1]:53885)".into(),
            key_type: "ED25519".into(),
            fingerprint: "SHA256:EcsfrjDiZE3k2nSxhHEJR+yDlulL0Zn0KsQX+lN75Q0".into(),
        }
    );
}

/// 主机的其他已知名称有助于判断主机是否可信，存在时不能只显示解析出的字段。
#[test]
fn a_host_key_with_extra_lines_stays_verbatim() {
    let text = HOST_KEY.replace(
        "This key is not known by any other names.",
        "This host key is known by the following other names/addresses:\n    ~/.ssh/known:1: a",
    );
    let prompt = Prompt::parse(&text, false);
    assert!(prompt.is_confirmation());
    assert!(matches!(prompt, Prompt::Other { .. }), "{prompt:?}");
}

#[test]
fn hosts_come_from_addresses() {
    assert_eq!(host_of("https://github.com/a/b.git"), "github.com");
    assert_eq!(host_of("github.com"), "github.com");
}

/// 经真实端点发送一次请求；`answer` 为 `None` 时模拟关闭对话框。
fn ask_through_a_listener(answer: Option<&'static str>) -> (Prompt, Option<String>) {
    let (asked, questions) = channel();
    let listener = Listener::bind(move |question: Question| {
        let _ = asked.send(question.prompt.clone());
        if let Some(answer) = answer {
            let _ = question.reply.send(answer.to_owned());
        }
    })
    .unwrap();
    let mut stream = imp::connect(&listener.endpoint).unwrap();
    write_question(&mut stream, "Enter passphrase for key '/k': ", false).unwrap();
    let answer = read_answer(&mut stream).unwrap();
    let prompt = questions.recv_timeout(Duration::from_secs(5)).unwrap();
    (prompt, answer)
}

#[test]
fn an_answer_reaches_the_askpass_program() {
    let (prompt, answer) = ask_through_a_listener(Some("秘密 with space"));
    assert_eq!(prompt, Prompt::Passphrase { key: "/k".into() });
    assert_eq!(answer.as_deref(), Some("秘密 with space"));
}

#[test]
fn a_closed_dialog_counts_as_canceled() {
    assert_eq!(ask_through_a_listener(None).1, None);
}

#[cfg(unix)]
#[test]
fn the_socket_is_private_and_removed_with_the_listener() {
    use std::os::unix::fs::PermissionsExt;
    let listener = Listener::bind(|_| {}).unwrap();
    let path = std::path::PathBuf::from(&listener.endpoint);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0);
    drop(listener);
    assert!(!path.exists());
}

#[test]
fn an_oversized_message_is_rejected() {
    let length = (MESSAGE_LIMIT + 1).to_le_bytes();
    let mut bytes = length.as_slice();
    assert!(read_text(&mut bytes, MESSAGE_LIMIT).is_err());
}

#[test]
fn a_message_cut_short_is_rejected() {
    let mut bytes: &[u8] = &[5, 0, 0, 0, b'a'];
    assert!(read_text(&mut bytes, MESSAGE_LIMIT).is_err());
}
