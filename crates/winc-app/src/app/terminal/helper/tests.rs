//! 辅助进程交互的测试：接上假的辅助进程、读回发出去的消息。

use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use winc_pty::protocol::Decoder;

impl App {
    /// 测试用：接上一个什么也不做的「辅助进程」，发出去的消息不处理，标签当作已经可以用了。
    pub(crate) fn attach_idle_helper(&mut self) {
        self.terminal.client = Client::start(std::path::Path::new("/usr/bin/true"), || {}).ok();
        self.terminal.ready = true;
    }

    /// 测试用：连着辅助进程没有。
    pub(crate) fn terminal_connected(&self) -> bool {
        self.terminal.client.is_some()
    }

    /// 测试用：接上一个把收到的消息原样记进 `dir` 下的文件的「辅助进程」，标签当作已经可以用了。
    ///
    /// 返回记下的文件，交给 [`App::sent_to_helper`] 读。测试直接放进来的标签（`show_terminal`）
    /// 不占编号，这里补上，之后新建的标签不和它们重号。
    pub(crate) fn attach_recording_helper(&mut self, dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let log = dir.join("sent.bin");
        let script = dir.join("record.sh");
        let body = format!("#!/bin/sh\nexec cat > '{}'\n", log.display());
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        self.terminal.client = Client::start(&script, || {}).ok();
        self.terminal.ready = true;
        let t = &mut self.terminal;
        t.next_id = t.tabs.iter().map(|tab| tab.id).fold(t.next_id, u16::max);
        log
    }

    /// 测试用：到现在为止发给 [`App::attach_recording_helper`] 接上的辅助进程的消息。
    ///
    /// 先发一条记号，等辅助进程记下后再读，此前的消息都已经写进文件了。每次的记号都不一样：
    /// 上一次的记号已经在文件里了，如果按它判断，会读到还没写完的内容。
    pub(crate) fn sent_to_helper(&self, log: &Path) -> Vec<ToHelper> {
        static MARKS: AtomicU16 = AtomicU16::new(u16::MAX);
        let is_mark = |m: &ToHelper| matches!(m, ToHelper::Clear { tab } if *tab > u16::MAX / 2);
        let mark = ToHelper::Clear {
            tab: MARKS.fetch_sub(1, Ordering::Relaxed),
        };
        self.terminal.client.as_ref().unwrap().send(&mark);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut decoder = Decoder::default();
            decoder.push(&std::fs::read(log).unwrap_or_default());
            let mut sent = Vec::new();
            while let Ok(Some(message)) = decoder.next_message::<ToHelper>() {
                sent.push(message);
            }
            if sent.last() == Some(&mark) {
                sent.retain(|m| !is_mark(m));
                return sent;
            }
            assert!(Instant::now() < deadline, "辅助进程没有收到记号");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 测试用：到现在为止发给标签 `tab` 的输入，拼在一起（见 [`App::sent_to_helper`]）。
    pub(crate) fn input_sent_to(&self, log: &Path, tab: u16) -> Vec<u8> {
        self.sent_to_helper(log)
            .iter()
            .filter_map(|m| match m {
                ToHelper::Input { tab: t, bytes, .. } if *t == tab => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }
}

/// 以字符串表示的 [`launch`] 结果：（程序，参数，目录）。
fn launched(
    shell: Option<&str>,
    cwd: &str,
    windows: bool,
) -> (Option<String>, Vec<String>, String) {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let launch = launch(shell, cwd.as_bytes().to_vec(), windows);
    (
        launch.program.as_deref().map(text),
        launch.args.iter().map(|arg| text(arg)).collect(),
        text(&launch.cwd),
    )
}

#[test]
fn a_folder_in_wsl_opens_its_distribution() {
    let (program, args, cwd) = launched(None, r"\\wsl.localhost\Ubuntu\home\me", true);
    assert_eq!(program.as_deref(), Some("wsl.exe"));
    assert_eq!(args, ["-d", "Ubuntu", "--cd", "/home/me"]);
    // 并非所有 shell 都接受 UNC 路径作为工作目录，因此目录由 `wsl.exe` 的参数指定。
    assert_eq!(cwd, "");
}

#[test]
fn other_folders_use_the_default_shell() {
    let (program, args, cwd) = launched(None, r"C:\work", true);
    assert_eq!((program, args.len()), (None, 0));
    assert_eq!(cwd, r"C:\work");
    // 其他平台不识别 WSL 路径。
    let (program, _, _) = launched(None, r"\\wsl.localhost\Ubuntu\home", false);
    assert_eq!(program, None);
}

#[test]
fn the_shell_setting_takes_priority() {
    let (program, args, cwd) = launched(Some("wsl.exe -d Debian"), r"C:\work", true);
    assert_eq!(program.as_deref(), Some("wsl.exe"));
    assert_eq!(args, ["-d", "Debian"]);
    assert_eq!(cwd, r"C:\work");
    let (program, _, _) = launched(Some("/bin/zsh"), r"\\wsl.localhost\Ubuntu", true);
    assert_eq!(program.as_deref(), Some("/bin/zsh"));
    // 仅含空白的值视为未设置。
    let (program, _, _) = launched(Some("   "), "/work", false);
    assert_eq!(program, None);
}
