//! 运行外部程序的测试：大输入输出不会死锁、超时与取消会终止整个进程组、遗留的子进程不阻塞
//! 返回、输出超过上限即停止。

use super::*;
use crate::test_util::scratch;
use crate::tools::format::tests::fake_tool;

fn sh(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]).stderr(Stdio::piped());
    command
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// 进程是否在 1 秒内消失：被终止的进程需要等系统回收，每 20 毫秒检查一次，最多 50 次。
fn gone(pid: &str) -> bool {
    (0..50).any(|_| {
        thread::sleep(Duration::from_millis(20));
        !alive(pid)
    })
}

#[test]
fn large_input_and_output_do_not_deadlock() {
    let input = "0123456789\n".repeat(1 << 20);
    let output = run(
        &mut sh("cat"),
        input.clone().into_bytes(),
        Duration::from_secs(20),
        &Cancel::default(),
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), input.len());

    let output = run(
        &mut sh("echo oops >&2; exit 3"),
        Vec::new(),
        Duration::from_secs(5),
        &Cancel::default(),
    )
    .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(output.stderr, b"oops\n");
}

#[test]
fn timeouts_kill_the_whole_group() {
    let dir = scratch("timeout");
    let pid = dir.join("pid");
    let script = format!("sleep 30 & echo $! > '{}'; wait", pid.display());
    let started = Instant::now();
    let result = run(
        &mut sh(&script),
        Vec::new(),
        Duration::from_millis(300),
        &Cancel::default(),
    );
    assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        gone(&std::fs::read_to_string(&pid).unwrap()),
        "孙进程也要杀掉"
    );
}

#[test]
fn leftover_children_do_not_hold_the_output() {
    // 程序已退出，遗留在后台的子进程仍占用输出管道：仍应很快（3 秒内）返回结果，且该子进程
    // 被终止。
    let dir = scratch("leftover");
    let pid = dir.join("pid");
    let script = format!("cat; sleep 30 & echo $! > '{}'", pid.display());
    let started = Instant::now();
    let output = run(
        &mut sh(&script),
        b"abc".to_vec(),
        Duration::from_secs(10),
        &Cancel::default(),
    )
    .unwrap();
    assert_eq!(output.stdout, b"abc");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(gone(&std::fs::read_to_string(&pid).unwrap()));
}

#[test]
fn output_held_by_an_escaped_process_times_out_and_forgets_the_pid() {
    // 遗留的子进程自己新建了会话（终止进程组时影响不到它），并仍占用输出管道：到期限即放弃
    // 等待，并且清除已被回收的进程号（之后取消不会去终止可能已被复用的进程组）。
    if !std::path::Path::new("/usr/bin/perl").exists() {
        return;
    }
    let dir = scratch("escaped");
    let pid = dir.join("pid");
    // 子进程新建会话后才写进程号，`sh` 等到进程号才退出：否则负载高时子进程还没脱离进程组就被
    // 终止，管道随之释放，`run` 正常返回。
    let script = format!(
        "perl -e 'use POSIX qw(setsid); setsid(); open(my $f, \">\", $ARGV[0]); print $f $$; \
         close $f; exec \"sleep\", \"30\"' '{0}' & until [ -s '{0}' ]; do sleep 0.01; done",
        pid.display()
    );
    let cancel = Cancel::default();
    let result = run(
        &mut sh(&script),
        Vec::new(),
        Duration::from_secs(3),
        &cancel,
    );
    assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
    assert_eq!(cancel.lock().pid, None);
    let escaped = std::fs::read_to_string(&pid).unwrap();
    let _ = Command::new("kill").args(["-9", escaped.trim()]).status();
}

#[test]
fn cancel_stops_a_running_program() {
    let cancel = Cancel::default();
    let canceller = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        canceller.cancel();
    });
    let started = Instant::now();
    let result = run(
        &mut sh("sleep 30"),
        Vec::new(),
        Duration::from_secs(20),
        &cancel,
    );
    assert!(matches!(result, Err(RunError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(3));

    // 启动之前就已取消：程序启动后立即被终止，结果为 `Cancelled`。
    let cancel = Cancel::default();
    cancel.cancel();
    let result = run(
        &mut sh("sleep 30"),
        Vec::new(),
        Duration::from_secs(20),
        &cancel,
    );
    assert!(matches!(result, Err(RunError::Cancelled)), "{result:?}");
}

#[test]
fn runaway_output_is_cut_off() {
    let dir = scratch("runaway");
    let tool = fake_tool(&dir, "yes", "exec yes winc");
    let result = run(
        &mut Command::new(&tool),
        Vec::new(),
        Duration::from_secs(30),
        &Cancel::default(),
    );
    assert!(matches!(result, Err(RunError::TooLarge)), "{result:?}");
}

#[cfg(unix)]
#[test]
fn remote_questions_are_answered_by_the_local_askpass_program() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("winc-remote-askpass-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("askpass");
    let script = "#!/bin/sh\n[ \"$1\" = refuse ] && exit 1\n\
                  printf '%s|%s|%s\\n' \"$WINC_GIT_ASKPASS\" \"$SSH_ASKPASS_PROMPT\" \"$1\"\n";
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, PermissionsExt::from_mode(0o755)).unwrap();

    let mut command = Command::new("git");
    command.arg("fetch").env("LC_ALL", "C");
    assert!(Askpass::of(&command).is_none());
    command
        .env("GIT_ASKPASS", &program)
        .env("SSH_ASKPASS", &program)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(crate::git::ASKPASS_ENDPOINT, "the-endpoint");
    let askpass = Askpass::of(&command).unwrap();
    let ask = |mode: &str, prompt: &str| winc_backend::remote::Ask {
        mode: mode.into(),
        prompt: prompt.into(),
    };
    let answer = |mode, prompt| askpass.answer(&ask(mode, prompt));
    assert_eq!(
        answer("", "Password: ").as_deref(),
        Some(&b"the-endpoint||Password: "[..])
    );
    assert_eq!(
        answer("confirm", "Trust?").as_deref(),
        Some(&b"the-endpoint|confirm|Trust?"[..])
    );
    assert_eq!(answer("", "refuse"), None);
    // 本机的 askpass 程序和端点不发到远端，其余显式设置的变量照常传递。
    let mut plain = Command::new("git");
    plain.arg("fetch").env("LC_ALL", "C");
    let sent = |command: &Command| format!("{:?}", remote_exec(command, Vec::new()));
    assert_eq!(sent(&command), sent(&plain));
    std::fs::remove_dir_all(&dir).unwrap();
}
