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
        diffs: Vec::new(),
        token: token.map(str::to_owned),
    };
    let round_trip = |launch: Launch| assert_eq!(decode(&encode(&launch)), Some(launch));
    round_trip(launch(&targets, None));
    round_trip(Launch::default());
    let token = "winc-1234_TIME5678";
    round_trip(launch(&targets, Some(token)));
    round_trip(launch(&[], Some(token)));
    // 没有令牌时发第 1 版消息，较早版本的 winc 也能收下。
    assert!(encode(&launch(&targets, None)).starts_with(HELLO.as_bytes()));
    // 不合法的令牌不写入消息。
    for bad in ["", "a\nb", "a\0b"] {
        assert_eq!(
            encode(&launch(&targets, Some(bad))),
            encode(&launch(&targets, None)),
            "{bad:?}"
        );
    }
    // 有要比较的文件时发第 3 版消息，目标和要比较的文件都还原；令牌可以没有。
    let diffs = vec![Diff {
        left: PathBuf::from("/p/旧 a.rs"),
        right: PathBuf::from("/p/b.rs"),
    }];
    for token in [None, Some(token)] {
        let compared = Launch {
            diffs: diffs.clone(),
            ..launch(&targets, token)
        };
        assert!(encode(&compared).starts_with(HELLO_DIFF.as_bytes()));
        round_trip(compared);
    }
    round_trip(Launch {
        diffs,
        ..Launch::default()
    });
    assert_eq!(decode(b"winc-open 2\n"), None);
    assert_eq!(decode(b"winc-open 2\ntoken"), None);
    assert_eq!(decode(b"winc-open 1\n/p\0x\0\0"), None);
    assert_eq!(decode(b"winc-open 3\n"), None);
    assert_eq!(decode(b"winc-open 3\n\nmove\0/p\0"), None);
    assert_eq!(decode(b"winc-open 3\n\ndiff\0/p\0"), None);
}

#[test]
fn tokens_fall_back_to_the_first_version_for_older_wincs() {
    let sent = std::cell::RefCell::new(Vec::new());
    let targets = [target("/p", None, None)];
    let launch = |token: Option<&str>| Launch {
        targets: targets.to_vec(),
        diffs: Vec::new(),
        token: token.map(str::to_owned),
    };
    // 较早版本的 winc 不认识第 2 版消息，不答复就断开；改发第 1 版消息后收下。
    let older = |message: &[u8]| {
        sent.borrow_mut().push(message.to_vec());
        let ok = message.starts_with(HELLO.as_bytes());
        Ok(if ok { "ok\n" } else { "" }.to_owned())
    };
    assert!(deliver(&launch(Some("t")), older));
    let messages = sent.take();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].starts_with(HELLO_TOKEN.as_bytes()));
    assert_eq!(messages[1], encode(&launch(None)));

    // 对方答复 `busy` 时不重发；没有令牌时只发一次。
    let busy = |message: &[u8]| {
        sent.borrow_mut().push(message.to_vec());
        Ok("busy\n".to_owned())
    };
    assert!(!deliver(&launch(Some("t")), busy));
    assert_eq!(sent.take().len(), 1);
    let closed = |message: &[u8]| {
        sent.borrow_mut().push(message.to_vec());
        Ok(String::new())
    };
    assert!(!deliver(&launch(None), closed));
    assert_eq!(sent.take().len(), 1);
    // 第 3 版消息没有可退回的版本：较早版本不答复时只发一次，由新进程自己打开。
    let compared = Launch {
        diffs: vec![Diff {
            left: PathBuf::from("/a"),
            right: PathBuf::from("/b"),
        }],
        ..launch(Some("t"))
    };
    assert!(!deliver(&compared, older));
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
fn paths_keep_symlinks_when_handed_over() {
    let dir = winc_config::files::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("winc-absolute-{}", std::process::id()));
    let real = dir.join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("a.txt"), "").unwrap();
    let link = dir.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    // 经过符号链接的已有文件、新文件，以及所在目录也还不存在的新文件，都保留链接；`.` 和
    // `..` 按字面消去。
    assert_eq!(absolute(&link.join("a.txt")), link.join("a.txt"));
    assert_eq!(absolute(&link.join("new.txt")), link.join("new.txt"));
    assert_eq!(
        absolute(&link.join("sub/./deeper/../new.txt")),
        link.join("sub/new.txt")
    );

    // 相对路径相对于 shell 记下的当前目录，它与进程的当前目录是同一个目录时才用。
    assert_eq!(pick_cwd(Some(link.clone()), real.clone()), link);
    assert_eq!(pick_cwd(Some(dir.clone()), real.clone()), real);
    assert_eq!(pick_cwd(Some("relative".into()), real.clone()), real);
    assert_eq!(pick_cwd(None, real.clone()), real);
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
    // 没有目标也交：对方把当前窗口切到前台。激活令牌和要比较的文件随目标一起交出。
    let diff = Diff {
        left: PathBuf::from("/p/a"),
        right: PathBuf::from("/p/b"),
    };
    for (targets, diffs, token) in [
        (vec![target("/p/q", None, None)], Vec::new(), None),
        (Vec::new(), Vec::new(), None),
        (Vec::new(), Vec::new(), Some("winc-token")),
        (Vec::new(), vec![diff], Some("winc-token")),
    ] {
        let launch = Launch {
            targets,
            diffs,
            token: token.map(str::to_owned),
        };
        assert!(forward_to_app(&launch));
        let got = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(got, launch);
    }

    drop(listener);
    assert!(
        !forward_to_app(&Launch::default()),
        "不再等了：新进程自己开"
    );
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
    let launch = Launch {
        targets: targets.clone(),
        ..Launch::default()
    };
    assert!(imp::forward_to(&path, &launch));
    let got = rx.recv_timeout(std::time::Duration::from_secs(2));
    assert_eq!(got, Ok(targets));
    drop(listener);
}

/// 回归：消息不限长度时会全部读进内存；管道上的长度前缀超过上限时不读消息，也不交给界面线程。
#[cfg(windows)]
#[test]
fn oversized_pipe_messages_are_dropped() {
    let message = encode(&Launch {
        targets: vec![target(r"C:\p", None, None)],
        ..Launch::default()
    });
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

/// 转交时优先使用 Wayland 的令牌；第一个窗口按窗口系统取对应的令牌；转到后台的进程拿回两者。
#[test]
fn launch_tokens_follow_the_window_system() {
    let tokens = LaunchTokens {
        wayland: Some("w".to_owned()),
        x11: Some("x".to_owned()),
    };
    assert_eq!(tokens.forwarded(), Some("w"));
    assert_eq!(tokens.for_window(true).as_deref(), Some("w"));
    assert_eq!(tokens.for_window(false).as_deref(), Some("x"));
    #[cfg(unix)]
    {
        let env: Vec<_> = tokens.env().collect();
        assert_eq!(env, [(WAYLAND_TOKEN, "w"), (X11_TOKEN, "x")]);
        assert_eq!(LaunchTokens::default().env().count(), 0);
    }

    let x11 = LaunchTokens {
        wayland: None,
        x11: Some("x".to_owned()),
    };
    assert_eq!(x11.forwarded(), Some("x"));
    assert_eq!(x11.for_window(true), None);
}
