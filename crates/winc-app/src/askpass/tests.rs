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
    let listener = Listener::bind(false, move |question: Question| {
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
    let listener = Listener::bind(false, |_| {}).unwrap();
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

/// 经端点 `listener` 发送一次凭据助手的请求，返回答复。
fn credential_through(listener: &Listener, request: &str) -> Option<String> {
    let mut stream = imp::connect(&listener.endpoint).unwrap();
    stream.write_all(&[CREDENTIAL]).unwrap();
    write_text(&mut stream, request).unwrap();
    stream.flush().unwrap();
    read_answer(&mut stream).unwrap()
}

/// 经端点 `listener` 询问 `prompt`，返回答复。
fn ask(listener: &Listener, prompt: &str) -> Option<String> {
    let mut stream = imp::connect(&listener.endpoint).unwrap();
    write_question(&mut stream, prompt, false).unwrap();
    read_answer(&mut stream).unwrap()
}

/// 只有用户选择记住的地址才保存：`git` 也会把其他助手提供的凭据交来保存。
#[test]
fn a_credential_is_stored_only_when_the_user_chose_to_remember() {
    let listener = Listener::bind(true, |question: Question| {
        let remember = question.remember.expect("HTTP(S) 的登录请求提供记住密码");
        // 只记住 kept.askpass.test。
        let chosen = question.prompt.origin().unwrap().contains("kept");
        remember.store(chosen, Ordering::Relaxed);
        let _ = question.reply.send("typed".to_owned());
    })
    .unwrap();
    for host in ["kept.askpass.test", "other.askpass.test"] {
        let attributes = format!("protocol=https\nhost={host}\n");
        assert_eq!(
            credential_through(&listener, &format!("get\n{attributes}")),
            None
        );
        let prompt = format!("Password for 'https://alice@{host}': ");
        assert_eq!(ask(&listener, &prompt).as_deref(), Some("typed"));
        let stored = format!("store\n{attributes}username=alice\npassword=typed\n");
        assert_eq!(credential_through(&listener, &stored), None);
    }
    let get =
        |host: &str| credential_through(&listener, &format!("get\nprotocol=https\nhost={host}\n"));
    assert_eq!(
        get("kept.askpass.test").as_deref(),
        Some("username=alice\npassword=typed\n")
    );
    assert_eq!(get("other.askpass.test"), None);

    // 服务器拒绝后 `git` 要求删除：之后不再提供。
    let erase = "erase\nprotocol=https\nhost=kept.askpass.test\nusername=alice\n";
    assert_eq!(credential_through(&listener, erase), None);
    assert_eq!(get("kept.askpass.test"), None);
}

/// 不记住密码的端点不处理凭据助手的请求，登录请求也不提供该选项。
#[test]
fn a_listener_that_does_not_remember_ignores_credential_requests() {
    let listener = Listener::bind(false, |question: Question| {
        assert!(question.remember.is_none());
        let _ = question.reply.send("typed".to_owned());
    })
    .unwrap();
    let prompt = "Password for 'https://alice@off.askpass.test': ";
    assert_eq!(ask(&listener, prompt).as_deref(), Some("typed"));
    let stored = "store\nprotocol=https\nhost=off.askpass.test\nusername=alice\npassword=x\n";
    assert_eq!(credential_through(&listener, stored), None);
    assert!(credentials::find("https://off.askpass.test", None).is_none());
}

/// 私钥口令、SSH 密码和非 HTTP(S) 的地址不提供记住密码。
#[test]
fn only_http_sign_ins_can_be_remembered() {
    let origin = |text| Prompt::parse(text, false).origin();
    assert_eq!(
        origin("Username for 'https://github.com/a/b.git': ").as_deref(),
        Some("https://github.com")
    );
    assert_eq!(
        origin("Password for 'http://alice@example.com:8080': ").as_deref(),
        Some("http://example.com:8080")
    );
    assert_eq!(origin("Enter passphrase for key '/k': "), None);
    assert_eq!(origin("alice@example.com's password: "), None);
    assert_eq!(origin("Password for 'ftp://example.com': "), None);
}

/// 用真实的 `git` 和已构建的 winc 程序走完凭据助手协议：没有保存时询问，验证通过后保存，再次
/// 需要时直接提供，被拒绝后删除。
///
/// 使用测试程序同一构建目录中的 winc，需要先运行 `cargo build`；没有 `git` 或 winc 时跳过。
#[test]
#[ignore = "依赖已构建的 winc 程序，手动运行"]
fn git_stores_and_reuses_a_credential_through_the_helper() {
    use std::process::{Command, Stdio};

    let name = format!("winc{}", std::env::consts::EXE_SUFFIX);
    let Some(program) = std::env::current_exe()
        .ok()
        .and_then(|test| Some(test.parent()?.parent()?.join(name)))
        .filter(|program| program.is_file())
    else {
        return;
    };
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let asked = Arc::new(AtomicU64::new(0));
    let listener = Listener::bind(true, {
        let asked = asked.clone();
        move |question: Question| {
            asked.fetch_add(1, Ordering::Relaxed);
            if let Some(remember) = &question.remember {
                remember.store(true, Ordering::Relaxed);
            }
            let answer = match question.prompt {
                Prompt::Username { .. } => "alice",
                _ => "token-123",
            };
            let _ = question.reply.send(answer.to_owned());
        }
    })
    .unwrap();
    let helper = format!("!'{}' {CREDENTIAL_HELPER_ARG}", program.display());
    let git = |action: &str, input: &str| {
        let mut child = Command::new("git")
            .args(["credential", action])
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "credential.helper")
            .env("GIT_CONFIG_VALUE_0", &helper)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", &program)
            .env(ASKPASS_ENDPOINT, &listener.endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        String::from_utf8(output.stdout).unwrap()
    };
    let request = "protocol=https\nhost=e2e.askpass.test\n\n";

    // 没有保存：依次询问用户名和密码。
    let filled = git("fill", request);
    assert!(filled.contains("username=alice") && filled.contains("password=token-123"));
    assert_eq!(asked.load(Ordering::Relaxed), 2);
    // 验证通过：保存；再次需要时由助手提供，不再询问。
    git("approve", &filled);
    let again = git("fill", request);
    assert!(again.contains("username=alice") && again.contains("password=token-123"));
    assert_eq!(asked.load(Ordering::Relaxed), 2);
    // 被服务器拒绝：删除，下次重新询问。
    git("reject", &again);
    git("fill", request);
    assert_eq!(asked.load(Ordering::Relaxed), 4);
}
