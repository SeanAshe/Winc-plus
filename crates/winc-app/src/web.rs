//! 取回网络上的小文件：运行系统的 `curl`，未安装时运行 `wget`，本程序不链接网络库。
//!
//! 应用内更新读取 appcast、读取更新日志都经过这里。

use std::io;
use std::process::{Command, Stdio};

/// 构造运行 `program` 的命令。
#[cfg(unix)]
pub(crate) fn command(program: &str) -> Command {
    Command::new(program)
}

/// 构造运行 `program` 的命令，不为它创建控制台窗口。
#[cfg(windows)]
pub(crate) fn command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`：不为控制台程序创建窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// 取回 `url` 的内容，最长等待 `timeout` 秒：优先使用 `curl`，未安装时使用 `wget`。
///
/// `limit` 是内容长度的上限（字节）：服务器声明的长度超过它时 `curl` 不下载，取回的内容超过它
/// 时丢弃。请求失败或超过上限时返回 `None`。
pub(crate) fn fetch(url: &str, timeout: u64, limit: Option<u64>) -> Option<Vec<u8>> {
    let timeout = timeout.to_string();
    let within = limit.map(|limit| limit.to_string());
    let bound = within
        .iter()
        .flat_map(|limit| ["--max-filesize", limit.as_str()]);
    let curl = command("curl")
        .args(["--fail", "--silent", "--location", "--max-time", &timeout])
        .args(bound)
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let output = match curl {
        Err(e) if e.kind() == io::ErrorKind::NotFound => command("wget")
            .args([
                "--quiet",
                "--timeout",
                &timeout,
                "--output-document",
                "-",
                url,
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?,
        result => result.ok()?,
    };
    let fits = limit.is_none_or(|limit| output.stdout.len() as u64 <= limit);
    (output.status.success() && fits).then_some(output.stdout)
}
