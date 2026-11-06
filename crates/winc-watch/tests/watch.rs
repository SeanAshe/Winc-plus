//! `winc-watch` 进程的测试：报告目录中的变化，标准输入结束时退出，目录不存在时报错退出。

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// 等待一条消息的最长时间（秒）：通知由系统异步发送，负载高的测试机器上可能延迟数秒。
const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-watch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    winc_config::files::canonicalize(&dir).unwrap()
}

#[test]
fn reports_changes_and_exits_when_winc_goes_away() {
    let dir = scratch("reports");
    let mut child = Command::new(env!("CARGO_BIN_EXE_winc-watch"))
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, messages) = mpsc::channel();
    std::thread::spawn(move || {
        for message in BufReader::new(stdout).split(0).map_while(Result::ok) {
            let _ = sender.send(message);
        }
    });
    assert_eq!(messages.recv_timeout(WAIT).unwrap(), b"S");

    let file = dir.join("新 文件.txt");
    std::fs::write(&file, "x").unwrap();
    let want = format!("P{}", file.display()).into_bytes();
    while messages.recv_timeout(WAIT).unwrap() != want {}

    // 关闭标准输入等同于 winc 退出。
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_directory_is_an_error() {
    let dir = scratch("missing").join("none");
    let status = Command::new(env!("CARGO_BIN_EXE_winc-watch"))
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
}
