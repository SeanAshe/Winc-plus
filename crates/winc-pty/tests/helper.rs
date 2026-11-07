//! `winc-pty` 的端到端测试：启动真正的 `winc-pty`，通过协议让它在伪终端里运行真实程序，
//! 再按帧还原屏幕。

#![cfg(unix)]

mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::{Client, SIZE, WAIT};
use winc_pty::protocol::{FromHelper, Line, Size, ToHelper};

#[test]
fn runs_a_program_and_reports_its_exit_code() {
    let mut c = Client::start();
    c.spawn(
        1,
        "/bin/sh",
        &["-c", r"printf 'hi\033[31mred\033[0m'; exit 3"],
    );
    assert_eq!(c.until_exit_showing(|t| t == "hired"), 3);
    assert_eq!(
        c.screen[0].runs[1].style.fg,
        winc_pty::protocol::Color::Indexed(1)
    );
}

#[test]
fn input_reaches_the_program_and_echoes() {
    let mut c = Client::start();
    c.spawn(1, "cat", &[]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"hello\r".to_vec(),
        rest: 0,
    });
    c.until_text("hello\nhello");
    c.send(ToHelper::Input {
        tab: 1,
        bytes: vec![4],
        rest: 0,
    });
    assert_eq!(c.until_exit(), 0);
}

#[test]
fn backspace_in_line_mode_erases_a_whole_character() {
    // 终端驱动按 UTF-8 解析输入：按行读（`read`）时退格删掉整个「中」，不留下半个字符。
    let mut c = Client::start();
    c.spawn(1, "/bin/sh", &["-c", r#"read l; printf '[%s]' "$l""#]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: "中\x7fa\r".as_bytes().to_vec(),
        rest: 0,
    });
    assert_eq!(c.until_exit_showing(|t| t.ends_with("[a]")), 0);
}

#[test]
fn the_environment_says_what_terminal_this_is() {
    let mut c = Client::start();
    c.spawn(
        1,
        "/bin/sh",
        &[
            "-c",
            r#"printf '%s %s %s %s' "$TERM" "$COLORTERM" "$TERM_PROGRAM" "${WINC_SECRET:-none}""#,
        ],
    );
    c.until_exit_showing(|t| t == "xterm-256color truecolor winc none");
}

#[test]
fn resizing_reaches_the_program() {
    let mut c = Client::start();
    c.spawn(1, "/bin/sh", &["-c", "stty size; read x; stty size"]);
    c.until_text("6 40");
    c.send(ToHelper::Resize {
        tab: 1,
        size: Size {
            cols: 50,
            rows: 8,
            ..SIZE
        },
    });
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"\r".to_vec(),
        rest: 0,
    });
    c.until_text("8 50");
}

#[test]
fn queries_are_answered_through_the_pty() {
    let mut c = Client::start();
    // 程序问光标位置，读回答，把回答里的数字打出来。
    c.spawn(
        1,
        "/bin/sh",
        &[
            "-c",
            r#"stty raw -echo; printf 'ab\033[6n'; r=$(dd bs=6 count=1 2>/dev/null); stty sane; printf '\r\n%s' "$r" | tr -d '\033'"#,
        ],
    );
    c.until_exit_showing(|t| t.ends_with("[1;3R"));
}

/// 启动前写进终端的内容排在程序输出之前，不会传给程序。
#[test]
fn the_banner_comes_before_the_output() {
    let mut c = Client::start();
    c.send(ToHelper::Spawn {
        tab: 1,
        program: Some(b"/bin/sh".to_vec()),
        args: vec![b"-c".to_vec(), b"echo output".to_vec()],
        login: false,
        cwd: b"/".to_vec(),
        env: vec![(b"PATH".to_vec(), b"/usr/bin:/bin".to_vec())],
        scrollback: 100,
        size: SIZE,
        integration: None,
        banner: b"> command\r\n".to_vec(),
    });
    c.until_exit_showing(|t| t.contains("output"));
    assert!(c.text().starts_with("> command\noutput"), "{}", c.text());
}

/// 停止结束忽略挂断的程序，到期强制结束；终端的内容保留，退出照常报告。
#[test]
fn stop_ends_the_program_and_keeps_the_terminal() {
    let mut c = Client::start();
    let script = "trap '' HUP; echo started; while :; do sleep 1; done";
    c.spawn(1, "/bin/sh", &["-c", script]);
    c.until_text("started");
    c.send(ToHelper::Stop { tab: 1 });
    assert_eq!(c.until_exit(), 128 + 9);
    assert!(c.text().contains("started"));
}

#[test]
fn a_missing_program_fails_to_start() {
    let mut c = Client::start();
    c.spawn(1, "/no/such/program", &[]);
    match c.recv() {
        FromHelper::Failed { tab, message } => {
            assert_eq!(tab, 1);
            assert!(message.contains("/no/such/program"), "{message}");
        }
        other => panic!("应该报启动失败：{other:?}"),
    }
}

#[test]
fn the_tab_learns_a_command_ended_even_without_more_output() {
    // 命令的输出在上一次查询 0.5 s 后才出现，并且很快结束：结束时那一帧离上次查询不到一个
    // 间隔。此后没有新输出，也须按时间再查一次，把前台已换回 shell 告诉主程序。
    let mut c = Client::start();
    c.spawn(1, "/bin/sh", &["-i"]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"perl -e 'select(undef, undef, undef, 0.5); print 1; select(undef, undef, undef, 0.1)'\r".to_vec(),
        rest: 0,
    });
    let name = |m: &FromHelper| match m {
        FromHelper::Process { name, .. } => Some(name.clone()),
        _ => None,
    };
    c.until(|_, m| name(m).as_deref() == Some("perl"));
    c.until(|_, m| name(m).is_some_and(|n| n != "perl"));
}

/// 回车的回显发帧时命令还没启动，之后没有新的输出：发帧之后再查一次，才能发现它。
#[test]
fn a_silent_command_is_still_found_as_the_foreground_process() {
    let mut c = Client::start();
    c.spawn(1, "/bin/sh", &["-i"]);
    let send = |c: &mut Client, bytes: &[u8]| {
        c.send(ToHelper::Input {
            tab: 1,
            bytes: bytes.to_vec(),
            rest: 0,
        });
    };
    send(&mut c, b"sleep 0.1; cat");
    // 等过查询间隔（300 ms）：回车的回显发帧时立即查询，这时 shell 还没有启动命令。
    thread::sleep(Duration::from_millis(500));
    send(&mut c, b"\n");
    c.until(|_, m| matches!(m, FromHelper::Process { name, .. } if name == "cat"));
}

#[test]
fn closing_the_last_tab_ends_the_helper_and_its_processes() {
    let mut c = Client::start();
    c.spawn(1, "/bin/sh", &["-c", "echo $$; exec sleep 100"]);
    c.until(|c, _| c.text().trim().parse::<i32>().is_ok());
    let pid: i32 = c.text().trim().parse().unwrap();

    c.send(ToHelper::Close { tab: 1 });
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = c.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "辅助进程没有退出");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    assert!(!alive(pid), "sleep 还活着");
}

/// 回归：关闭标签时 SIGHUP、SIGKILL 曾只发给 shell 及其进程组，开启了作业控制的前台程序
/// 在自己的进程组里收不到；忽略 SIGHUP 的程序在 shell 退出后成为孤儿并一直运行。
#[test]
fn closing_a_tab_also_ends_a_foreground_job_that_ignores_hangup() {
    let mut c = Client::start();
    let job = r#"$SIG{HUP} = "IGNORE"; $| = 1; print "$$\n"; sleep 100"#;
    c.spawn(1, "/bin/sh", &["-c", &format!("set -m; perl -e '{job}'")]);
    // 系统没有安装环境中指定的区域设置时 perl 会先输出警告，进程号在最后一行。
    let last = |c: &Client| c.text().trim().lines().last().map(str::to_owned);
    c.until(|c, _| last(c).is_some_and(|l| l.parse::<i32>().is_ok()));
    let pid: i32 = last(&c).unwrap().parse().unwrap();

    c.send(ToHelper::Close { tab: 1 });
    // 挂断之后过一段时间才发 SIGKILL；辅助进程要等它们全部退出才结束。
    let deadline = Instant::now() + WAIT + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = c.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "辅助进程没有退出");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    assert!(!alive(pid), "忽略 SIGHUP 的前台作业还活着");
}

/// 主程序关闭标签后马上退出（关闭最后一个标签、切换目录）：忽略 SIGHUP 的前台作业仍要结束。
#[test]
fn closing_the_last_tab_and_going_away_still_ends_the_job() {
    let mut c = Client::start();
    let job = r#"$SIG{HUP} = "IGNORE"; $| = 1; print "$$\n"; sleep 100"#;
    c.spawn(1, "/bin/sh", &["-c", &format!("set -m; perl -e '{job}'")]);
    let last = |c: &Client| c.text().trim().lines().last().map(str::to_owned);
    c.until(|c, _| last(c).is_some_and(|l| l.parse::<i32>().is_ok()));
    let pid: i32 = last(&c).unwrap().parse().unwrap();

    c.send(ToHelper::Close { tab: 1 });
    drop(c.stdin.take());
    let deadline = Instant::now() + WAIT + Duration::from_secs(2);
    while alive(pid) {
        assert!(Instant::now() < deadline, "忽略 SIGHUP 的前台作业还活着");
        thread::sleep(Duration::from_millis(10));
    }
}

/// 主程序直接退出时，打开着的标签里忽略 SIGHUP 的前台作业同样要结束。
#[test]
fn going_away_with_a_tab_open_also_ends_a_job_that_ignores_hangup() {
    let mut c = Client::start();
    let job = r#"$SIG{HUP} = "IGNORE"; $| = 1; print "$$\n"; sleep 100"#;
    c.spawn(1, "/bin/sh", &["-c", &format!("set -m; perl -e '{job}'")]);
    let last = |c: &Client| c.text().trim().lines().last().map(str::to_owned);
    c.until(|c, _| last(c).is_some_and(|l| l.parse::<i32>().is_ok()));
    let pid: i32 = last(&c).unwrap().parse().unwrap();

    drop(c.stdin.take());
    let deadline = Instant::now() + WAIT + Duration::from_secs(2);
    while alive(pid) {
        assert!(Instant::now() < deadline, "忽略 SIGHUP 的前台作业还活着");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_helper_hangs_up_everything_when_the_app_goes_away() {
    let mut c = Client::start();
    c.spawn(1, "/bin/sh", &["-c", "echo $$; exec sleep 100"]);
    c.until(|c, _| c.text().trim().parse::<i32>().is_ok());
    let pid: i32 = c.text().trim().parse().unwrap();

    drop(c.stdin.take());
    let status = c.child.wait().unwrap();
    assert!(status.success());
    let deadline = Instant::now() + WAIT;
    while alive(pid) {
        assert!(Instant::now() < deadline, "sleep 还活着");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn history_comes_back_on_request() {
    let mut c = Client::start();
    c.spawn(
        1,
        "/bin/sh",
        &["-c", "for i in 1 2 3 4 5 6 7 8 9 10; do echo line$i; done"],
    );
    c.until_exit();
    c.send(ToHelper::History {
        tab: 1,
        id: 42,
        start: 0,
        end: 100,
    });
    let message = c.until(|_, m| matches!(m, FromHelper::History { .. }));
    let FromHelper::History {
        id, start, lines, ..
    } = message
    else {
        unreachable!()
    };
    assert_eq!((id, start), (42, 0));
    let text: Vec<&str> = lines.iter().map(Line::trimmed).collect();
    assert_eq!(text, ["line1", "line2", "line3", "line4", "line5"]);
}

fn alive(pid: i32) -> bool {
    // 进程退出后可能留下僵尸进程（父进程是 sh 时由它回收）；kill(pid, 0) 返回失败说明进程
    // 已不存在。检查进程是否存在只能用 libc。
    #[allow(unsafe_code)]
    // SAFETY: 信号 0 只检查进程在不在，不发信号。
    let result = unsafe { libc::kill(pid, 0) };
    result == 0
}
