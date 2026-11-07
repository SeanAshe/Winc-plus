//! 单实例转交的测试：消息编解码与版本兼容、套接字路径、规范路径与目录权限。

use super::*;

fn target(path: &str, line: Option<usize>, col: Option<usize>) -> Target {
    Target {
        path: PathBuf::from(path),
        line,
        col,
    }
}

#[test]
fn messages_round_trip() {
    let targets = vec![
        target("/p/a b.rs", Some(9), Some(2)),
        target("/p/中文:名字.txt", None, None),
        target("/p", None, None),
    ];
    let launch = |targets: &[Target], token: Option<&str>| Launch {
        targets: targets.to_vec(),
        token: token.map(str::to_owned),
    };
    assert_eq!(
        decode(&encode(&targets, None)),
        Some(launch(&targets, None))
    );
    assert_eq!(decode(&encode(&[], None)), Some(Launch::default()));
    let token = "winc-1234_TIME5678";
    assert_eq!(
        decode(&encode(&targets, Some(token))),
        Some(launch(&targets, Some(token)))
    );
    assert_eq!(
        decode(&encode(&[], Some(token))),
        Some(launch(&[], Some(token)))
    );
    // 没有令牌时发第 1 版消息，较早版本的 winc 也能收下。
    assert!(encode(&targets, None).starts_with(HELLO.as_bytes()));
    // 不合法的令牌不写入消息。
    for bad in ["", "a\nb", "a\0b"] {
        assert_eq!(
            encode(&targets, Some(bad)),
            encode(&targets, None),
            "{bad:?}"
        );
    }
    assert_eq!(decode(b"winc-open 2\n"), None);
    assert_eq!(decode(b"winc-open 2\ntoken"), None);
    assert_eq!(decode(b"winc-open 1\n/p\0x\0\0"), None);
    assert_eq!(decode(b"winc-open 3\n"), None);
}

#[test]
fn tokens_fall_back_to_the_first_version_for_older_wincs() {
    let sent = std::cell::RefCell::new(Vec::new());
    let targets = [target("/p", None, None)];
    // 较早版本的 winc 不认识第 2 版消息，不答复就断开；改发第 1 版消息后收下。
    let older = |message: &[u8]| {
        sent.borrow_mut().push(message.to_vec());
        let ok = message.starts_with(HELLO.as_bytes());
        Ok(if ok { "ok\n" } else { "" }.to_owned())
    };
    assert!(deliver(&targets, Some("t"), older));
    let messages = sent.take();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].starts_with(HELLO_TOKEN.as_bytes()));
    assert_eq!(messages[1], encode(&targets, None));

    // 对方答复 `busy` 时不重发；没有令牌时只发一次。
    let busy = |message: &[u8]| {
        sent.borrow_mut().push(message.to_vec());
        Ok("busy\n".to_owned())
    };
    assert!(!deliver(&targets, Some("t"), busy));
    assert_eq!(sent.take().len(), 1);
    let closed = |message: &[u8]| {
        sent.borrow_mut().push(message.to_vec());
        Ok(String::new())
    };
    assert!(!deliver(&targets, None, closed));
    assert_eq!(sent.take().len(), 1);
}

#[cfg(unix)]
#[test]
fn only_private_runtime_dirs_are_used() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("winc-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mode = |m| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o700);
    assert!(private_dir(&dir));
    // 别人能进的目录（例如大家共用的 /tmp）不用。
    mode(0o755);
    assert!(!private_dir(&dir));
    assert!(!private_dir(Path::new("/tmp")));
    assert!(!private_dir(&dir.join("missing")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn paths_are_resolved_before_handing_over() {
    let dir = winc_config::files::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("winc-absolute-{}", std::process::id()));
    let real = dir.join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("a.txt"), "").unwrap();
    let link = dir.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    // 经过符号链接的已有文件、新文件，以及所在目录也还不存在的新文件，都落到真实目录下。
    assert_eq!(absolute(&link.join("a.txt")), real.join("a.txt"));
    assert_eq!(absolute(&link.join("new.txt")), real.join("new.txt"));
    assert_eq!(
        absolute(&link.join("sub/deeper/new.txt")),
        real.join("sub/deeper/new.txt")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn only_one_winc_waits_for_launches_and_gets_them() {
    let lock = AppLock::acquire().expect("拿到整个应用的锁");
    assert!(AppLock::acquire().is_none(), "同时只有一个 winc 拿得到");

    let (tx, rx) = std::sync::mpsc::channel();
    let listener = Listener::bind_app(move |launch, answer| {
        if answer.send(true).is_ok() {
            let _ = tx.send(launch);
        }
    })
    .unwrap();
    // 没有目标也交：对方把当前窗口切到前台。激活令牌随目标一起交出。
    for (targets, token) in [
        (vec![target("/p/q", None, None)], None),
        (Vec::new(), None),
        (Vec::new(), Some("winc-token")),
    ] {
        assert!(forward_to_app(&targets, token));
        let got = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(got.targets, targets);
        assert_eq!(got.token.as_deref(), token);
    }

    drop(listener);
    assert!(!forward_to_app(&[], None), "不再等了：新进程自己开");
    drop(lock);
    // 其他测试启动外部程序时，子进程在 exec 之前短暂继承锁文件的描述符，等它释放。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let mut again = AppLock::acquire();
    while again.is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        again = AppLock::acquire();
    }
    assert!(again.is_some(), "放开以后别的 winc 能拿到");
}

#[cfg(unix)]
#[test]
fn oversized_messages_are_dropped() {
    use std::io::{Read, Write};
    let path = std::env::temp_dir().join(format!("winc-big-{}.sock", std::process::id()));
    let (tx, rx) = std::sync::mpsc::channel();
    let listener = imp::Listener::bind_at(path.clone(), move |launch: Launch, answer| {
        if answer.send(true).is_ok() {
            let _ = tx.send(launch.targets);
        }
    })
    .unwrap();

    // 回归：消息不限长度时全部读进内存。超过上限的消息不交给界面线程，也不答复。
    let mut stream = std::os::unix::net::UnixStream::connect(&path).unwrap();
    // 前 `MESSAGE_LIMIT + 1` 字节和整条消息都是格式正确的消息，只能靠长度拒绝。
    let mut message = HELLO.as_bytes().to_vec();
    message.resize(imp::MESSAGE_LIMIT as usize + 1 - 3, b'a');
    message.extend_from_slice(b"\0\0\0/b\0\0\0");
    let _ = stream.write_all(&message);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    assert!(reply.is_empty());
    // 超长消息之后的正常消息仍然收下，且是收到的第一条。
    let targets = vec![target("/p/q", Some(1), None)];
    assert!(imp::forward_to(&path, &targets, None));
    let got = rx.recv_timeout(std::time::Duration::from_secs(2));
    assert_eq!(got, Ok(targets));
    drop(listener);
}

/// 回归：消息不限长度时会全部读进内存；管道上的长度前缀超过上限时不读消息，也不交给界面线程。
#[cfg(windows)]
#[test]
fn oversized_pipe_messages_are_dropped() {
    let message = encode(&[target(r"C:\p", None, None)], None);
    let framed = |length: u64| {
        let mut bytes = length.to_le_bytes().to_vec();
        bytes.extend_from_slice(&message);
        std::io::Cursor::new(bytes)
    };
    let read = |mut bytes| imp::read_message(&mut bytes);
    assert_eq!(read(framed(message.len() as u64)).unwrap().targets.len(), 1);
    assert_eq!(read(framed(imp::MESSAGE_LIMIT + 1)), None);
    assert_eq!(read(framed(message.len() as u64 + 1)), None, "读不全");
}

#[test]
fn portable_pipe_tags_ignore_case_and_differ_by_folder() {
    let tag = |path: &str| path_tag(Path::new(path));
    assert_eq!(tag(r"D:\Tools\winc\data"), tag(r"d:\tools\WINC\data"));
    assert_ne!(tag(r"D:\Tools\winc\data"), tag(r"E:\winc\data"));
}
