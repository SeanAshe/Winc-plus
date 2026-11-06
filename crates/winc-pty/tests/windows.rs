//! Windows 的端到端测试：启动真正的 `winc-pty` 在 ConPTY 中运行 `cmd.exe`，按帧还原屏幕。

#![allow(unsafe_code)] // 查询是否运行在 Wine 中
#![cfg(windows)]

mod common;

use std::time::{Duration, Instant};

use common::{Client, SIZE, WAIT};
use winc_pty::protocol::{FromHelper, ToHelper};

/// 是否运行在 Wine 中。
///
/// Wine 的伪终端不转发程序的输出，这些测试需要在真实的 Windows 上运行。
fn under_wine() -> bool {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    // SAFETY: 两个参数都是以 NUL 结尾的字符串常量，在整个调用期间有效；只查询模块句柄和
    // 函数地址，不调用找到的函数。
    unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
        !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
    }
}

/// 运行在 Wine 中时输出提示并返回 `true`。
fn skip_under_wine() -> bool {
    let wine = under_wine();
    if wine {
        eprintln!("跳过：Wine 的伪终端不转发程序的输出，需要在 Windows 上运行");
    }
    wine
}

/// 系统 `cmd.exe` 的路径。
fn cmd() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into())
}

#[test]
fn a_program_runs_in_a_pseudo_console_and_reports_its_exit_code() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    c.spawn(1, &cmd(), &["/c", "echo hello from conpty& exit 3"]);
    assert_eq!(c.until_exit_showing(|t| t.contains("hello from conpty")), 3);
}

#[test]
fn input_reaches_the_shell() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    c.spawn(1, &cmd(), &["/q", "/k"]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"echo typed %COMSPEC%\r".to_vec(),
        rest: 0,
    });
    c.until_text("typed ");
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"exit 5\r".to_vec(),
        rest: 0,
    });
    assert_eq!(c.until_exit(), 5);
}

/// 结束辅助进程 `helper` 自己建的伪终端进程，返回结束了几个。
///
/// 按父进程号和 `--headless` 筛选，不影响其他伪终端，也不影响辅助进程自己的控制台。
fn kill_pseudo_console(helper: u32) -> usize {
    let filter = format!("ParentProcessId={helper} AND CommandLine LIKE '%--headless%'");
    let script = format!(
        "$found = @(Get-CimInstance Win32_Process -Filter \"{filter}\"); \
         $found | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}; \
         $found.Count"
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .expect("启动 PowerShell");
    let count = String::from_utf8_lossy(&output.stdout);
    count.trim().parse().expect("PowerShell 输出个数")
}

/// 等到辅助进程退出，返回退出状态。
fn until_helper_exits(c: &mut Client) -> std::process::ExitStatus {
    let deadline = Instant::now() + WAIT + Duration::from_secs(2);
    loop {
        if let Some(status) = c.child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "辅助进程没有退出");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 伪终端的进程不在了而 shell 还在运行：报告断开，不当作退出；关闭标签时结束剩下的进程。
#[test]
fn losing_the_pseudo_console_is_reported_while_the_shell_runs() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    // `ping` 不读写控制台，伪终端不在了也继续运行；它要用到完整的环境才能初始化网络。
    c.send(ToHelper::Spawn {
        tab: 1,
        program: Some(cmd().into()),
        args: ["/c", "echo ready& ping -n 60 127.0.0.1 >NUL"]
            .map(|arg| arg.as_bytes().to_vec())
            .to_vec(),
        login: false,
        cwd: b"/".to_vec(),
        env: std::env::vars()
            .map(|(k, v)| (k.into_bytes(), v.into_bytes()))
            .collect(),
        scrollback: 100,
        size: SIZE,
        integration: None,
    });
    c.until_text("ready");

    assert_eq!(kill_pseudo_console(c.child.id()), 1);
    let ended = c.until(|_, m| {
        matches!(
            m,
            FromHelper::Disconnected { .. } | FromHelper::Exited { .. }
        )
    });
    assert_eq!(ended, FromHelper::Disconnected { tab: 1 });

    // 伪终端的进程已经不在，关闭它不能卡住：作业结束，辅助进程随后退出。
    c.send(ToHelper::Close { tab: 1 });
    assert!(until_helper_exits(&mut c).success());
}

/// 关闭标签：伪终端随之关闭，其中的程序以及它启动的程序一同结束，辅助进程随后退出。
#[test]
fn closing_a_tab_ends_everything_in_it() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    // `cmd` 中再启动一个等待输入的 `cmd`：关闭标签时两层都要结束。
    c.spawn(1, &cmd(), &["/q", "/k", "cmd /q /k"]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"echo ready\r".to_vec(),
        rest: 0,
    });
    c.until_text("ready");
    c.send(ToHelper::Close { tab: 1 });
    assert!(until_helper_exits(&mut c).success());
}
