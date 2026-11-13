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
        // 别的测试线程 fork 时会短暂继承这个脚本的写入端，此时执行得到 `ExecutableFileBusy`。
        let mut client = Client::start(&script, || {});
        for _ in 0..100 {
            if !matches!(&client, Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            client = Client::start(&script, || {});
        }
        self.terminal.client = Some(client.expect("启动记录消息的脚本"));
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
    let launch = launch(shell, cwd.as_bytes().to_vec(), windows, Integration::Off);
    (
        launch.program.as_deref().map(text),
        launch.args.iter().map(|arg| text(arg)).collect(),
        text(&launch.cwd),
    )
}

#[test]
fn the_terminal_signs_in_with_the_key_file_given_for_the_host() {
    let target = winc_backend::Target::new("me@keyed-terminal").unwrap();
    let dir = winc_backend::virtual_path(&target, b"/srv");
    winc_backend::remote::set_identity(&target, Some("~/.ssh/id_work"));
    let (_, args, _) = launched(None, dir.to_str().unwrap(), false);
    assert_eq!(args[3..6], ["-i", "~/.ssh/id_work", "me@keyed-terminal"]);
    winc_backend::remote::set_identity(&target, None);
}

#[test]
fn a_remote_shell_gets_the_integration_once_its_scripts_are_there() {
    let target = winc_backend::Target::new("me@integrated").unwrap();
    let at = winc_backend::locate(&winc_backend::virtual_path(&target, b"/srv/app")).unwrap();
    let command = |integration, ready| {
        let launch = remote_launch(&at, integration, ready);
        String::from_utf8(launch.args.last().unwrap().clone()).unwrap()
    };
    let plain = "cd '/srv/app' && exec $SHELL -l";
    assert_eq!(command(Integration::Off, true), plain);
    // 脚本还没有写到那台主机上：照常启动登录 shell。
    assert_eq!(command(Integration::On { prompt: true }, false), plain);
    let integrated = command(Integration::On { prompt: false }, true);
    assert!(
        integrated.starts_with("cd '/srv/app' && exec sh -c '"),
        "{integrated}"
    );
    // 随机数的位置留给辅助进程填，其后是「不换提示符」。
    assert!(
        integrated.ends_with("' winc @WINC_NONCE@ 0"),
        "{integrated}"
    );
}

#[test]
fn a_folder_on_a_remote_host_opens_ssh_into_it() {
    let target = winc_backend::Target::new("me@devbox:22022").unwrap();
    let dir = winc_backend::virtual_path(&target, b"/srv/it's here");
    // 设置中的 shell 是本机的命令，对远端的目录不适用。
    for shell in [None, Some("zsh -l")] {
        let (program, args, cwd) = launched(shell, dir.to_str().unwrap(), false);
        assert_eq!(program.as_deref(), Some("ssh"));
        assert_eq!(
            args,
            [
                "-t",
                "-o",
                "RemoteCommand=none",
                "-p",
                "22022",
                "me@devbox",
                "--",
                r"cd '/srv/it'\''s here' && exec $SHELL -l",
            ]
        );
        assert_eq!(cwd, "");
    }
    // 没有指定端口时不加 `-p`，由用户的配置决定。
    let plain = winc_backend::Target::new("devbox").unwrap();
    let root = winc_backend::virtual_path(&plain, b"/");
    let (_, args, _) = launched(None, root.to_str().unwrap(), false);
    assert_eq!(args[3..], ["devbox", "--", "cd '/' && exec $SHELL -l"]);
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
fn a_directory_reported_by_a_wsl_shell_is_not_a_windows_working_directory() {
    let (program, _, cwd) = launched(None, "/home/me", true);
    assert_eq!((program, cwd.as_str()), (None, ""));
    let (_, _, cwd) = launched(Some("wsl.exe -d Debian"), "/home/me", true);
    assert_eq!(cwd, "");
    // 其他平台上是普通目录。
    let (_, _, cwd) = launched(None, "/home/me", false);
    assert_eq!(cwd, "/home/me");
}

#[test]
fn working_directories_stay_as_reported_outside_a_distribution() {
    let path = b"/home/me".to_vec();
    assert_eq!(local_cwd(None, path.clone()), path);
    assert_eq!(local_cwd(Some(Path::new("/work")), path.clone()), path);
}

#[cfg(windows)]
#[test]
fn working_directories_in_a_distribution_become_windows_paths() {
    let root = Path::new(r"\\wsl.localhost\Ubuntu\work");
    assert_eq!(
        local_cwd(Some(root), b"/home/me".to_vec()),
        br"\\wsl.localhost\Ubuntu\home\me".to_vec()
    );
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
    // 双引号括起的路径不拆开。
    let shell = r#""C:\Program Files\Git\bin\bash.exe" --login -i"#;
    let (program, args, _) = launched(Some(shell), r"C:\work", true);
    assert_eq!(
        program.as_deref(),
        Some(r"C:\Program Files\Git\bin\bash.exe")
    );
    assert_eq!(args, ["--login", "-i"]);
}
