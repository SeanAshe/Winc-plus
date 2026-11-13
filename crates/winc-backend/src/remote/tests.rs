//! 连接的登记与取用、界面线程的限制，以及 `ssh` 命令的组成。

use super::*;

#[test]
fn unknown_targets_are_not_connected_and_blocking_can_be_forbidden() {
    let target = Target::new("no-such-connection").unwrap();
    let error = connection(&target).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
    disconnect(&target);

    // 限制只作用于调用它的线程。
    let forbidden = std::thread::spawn(move || {
        forbid_blocking();
        connection(&target).err().unwrap().kind()
    });
    assert_eq!(forbidden.join().unwrap(), io::ErrorKind::WouldBlock);
    let target = Target::new("no-such-connection").unwrap();
    let error = connection(&target).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
}

#[test]
fn the_ssh_command_passes_the_program_as_one_quoted_word() {
    let target = Target::new("alice@devbox").unwrap();
    let command = ssh_command(
        &target,
        "/home/alice/.winc-server/it's here/winc-server",
        Ssh::default(),
    );
    let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(command.get_program(), "ssh");
    assert_eq!(
        args,
        [
            "-T",
            "-o",
            "RemoteCommand=none",
            "alice@devbox",
            "--",
            r"'/home/alice/.winc-server/it'\''s here/winc-server'",
        ]
    );
    // 端口由 `-p` 给出，主机参数中不含端口。
    let with_port = Target::new("alice@devbox:22022").unwrap();
    let command = ssh_command(&with_port, "server", Ssh::default());
    let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(args[3..], ["-p", "22022", "alice@devbox", "--", "'server'"]);
    // 不向用户提问的选项写在主机之前：主机之后的参数属于远端的命令。
    let options = Ssh {
        batch: true,
        keep_alive: true,
        connect_timeout: true,
        share: false,
    };
    let command = ssh_command(&target, "server", options);
    let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(
        args,
        [
            "-T",
            "-o",
            "RemoteCommand=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "ServerAliveInterval=60",
            "-o",
            "ConnectTimeout=15",
            "alice@devbox",
            "--",
            "'server'"
        ]
    );
}

#[test]
fn a_registered_key_file_is_passed_to_ssh_as_one_argument() {
    let target = Target::new("alice@keyed-host:2200").unwrap();
    let args = || -> Vec<String> {
        let command = ssh_command(&target, "server", Ssh::default());
        (command.get_args())
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect()
    };
    set_identity(&target, Some("~/.ssh/my key"));
    assert_eq!(identity(&target).as_deref(), Some("~/.ssh/my key"));
    assert_eq!(
        args()[3..],
        [
            "-i",
            "~/.ssh/my key",
            "-p",
            "2200",
            "alice@keyed-host",
            "--",
            "'server'"
        ]
    );
    // 空的路径去掉登记。
    set_identity(&target, Some(""));
    assert_eq!(identity(&target), None);
    assert_eq!(
        args()[3..],
        ["-p", "2200", "alice@keyed-host", "--", "'server'"]
    );
}

#[test]
fn only_settings_the_user_left_unset_are_filled_in() {
    let unset = "user alice\nserveraliveinterval 0\nconnecttimeout none\n";
    let filled = Ssh {
        batch: false,
        keep_alive: true,
        connect_timeout: true,
        share: false,
    };
    assert_eq!(Ssh::missing(unset), filled);
    // 连接复用：用户没有设置 `ControlMaster`、`ControlPath` 时才补。
    let plain = format!("{unset}controlmaster false\ncontrolpersist no\n");
    assert!(Ssh::missing(&plain).share);
    let own = format!("{plain}controlpath /home/alice/.ssh/cm-%C\n");
    assert!(!Ssh::missing(&own).share);
    assert!(!Ssh::missing("controlmaster auto\n").share);
    let set = "ServerAliveInterval 30\nConnectTimeout 5\n";
    assert_eq!(Ssh::missing(set), Ssh::default());
    // 读不到的项不补：不能确定用户没有设置。
    assert_eq!(Ssh::missing(""), Ssh::default());
}

#[cfg(unix)]
#[test]
fn a_command_that_is_not_the_server_fails_the_handshake() {
    // `true` 立即退出，不应答：握手读到输入结束。
    let error = Connection::spawn(Command::new("true")).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(Connection::spawn(Command::new("winc-no-such-program")).is_err());
}

/// 回归：检查被推迟（进程被暂停、系统从睡眠中恢复）后，对端没有动静的时间已超过期限；尚未
/// 探测即记为没有响应时，探测的应答到达后随即解除。
#[test]
fn a_delayed_check_probes_before_marking_the_connection() {
    let start = Instant::now();
    let mut probed = None;
    // 检查被推迟：没有动静的时间已超过期限，尚未探测，只发出探测。
    let late = start + STALL * 3;
    assert_eq!(check(start, &mut probed, late), (false, true));
    // 探测有了应答：不记为没有响应，探测的记录清除。
    let answered = late + Duration::from_millis(1);
    assert_eq!(check(answered, &mut probed, late + PROBE), (false, false));
    assert_eq!(probed, None);
    // 探测之后始终没有动静：达到期限时记为没有响应。
    assert_eq!(check(start, &mut probed, start + PROBE), (false, true));
    assert_eq!(check(start, &mut probed, start + STALL), (true, true));
    // 之后有了动静：恢复。
    let again = start + STALL;
    assert_eq!(check(again, &mut probed, again), (false, false));
    assert_eq!(probed, None);
}

#[test]
fn a_connection_that_stops_answering_is_marked_until_it_answers_again() {
    use std::sync::atomic::AtomicUsize;

    // 对端：应答握手；探测只在 `answering` 为真时应答，其余请求一概不应答。
    let (requests_in, requests_out) = io::pipe().unwrap();
    let (replies_in, mut replies_out) = io::pipe().unwrap();
    let answering = Arc::new(AtomicBool::new(true));
    let peer = Arc::clone(&answering);
    thread::spawn(move || {
        let mut requests = BufReader::new(requests_in);
        while let Ok(Some(frame)) = wire::read_frame(&mut requests) {
            let reply = match frame.kind {
                kind::HELLO => Writer::default().u32(wire::VERSION).finish(),
                kind::PING if peer.load(Ordering::Relaxed) => Vec::new(),
                _ => continue,
            };
            if wire::write_frame(&mut replies_out, kind::OK, frame.id, &reply).is_err() {
                return;
            }
        }
    });
    let connection = Connection::over(replies_in, requests_out).unwrap();
    let changes = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&changes);
    connection.on_stall(move || {
        seen.fetch_add(1, Ordering::Relaxed);
    });
    let target = Target::new("stalls").unwrap();
    register(target.clone(), connection);
    let until = |wanted: bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while is_stalled(&target) != wanted && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        is_stalled(&target) == wanted
    };

    // 没有请求在等待：对端再安静也不算没有响应。
    thread::sleep(STALL * 2);
    assert!(!is_stalled(&target));

    // 一个迟迟没有应答的请求（如运行很久的命令），探测仍有应答：不算没有响应。
    let waiter = {
        let target = target.clone();
        thread::spawn(move || {
            let path = crate::virtual_path(&target, b"/");
            crate::fs::metadata(path).unwrap_err().kind()
        })
    };
    thread::sleep(STALL * 2);
    assert!(!is_stalled(&target));
    assert_eq!(changes.load(Ordering::Relaxed), 0);

    // 探测也没有应答：记为没有响应，连接仍然保留；恢复应答后随即解除。
    answering.store(false, Ordering::Relaxed);
    assert!(until(true));
    assert!(is_connected(&target));
    answering.store(true, Ordering::Relaxed);
    assert!(until(false));
    assert_eq!(changes.load(Ordering::Relaxed), 2);

    disconnect(&target);
    assert_eq!(waiter.join().unwrap(), io::ErrorKind::BrokenPipe);
}
