//! 崩溃日志：发布版的 panic 直接中止进程，图形界面程序也没有可见的错误输出，因此把 panic 的
//! 版本、线程、位置和信息追加写入状态目录下的 `crash.log`，反馈问题时可以附上。
//!
//! 只写在本机，不上传。panic 信息可能含有路径或文字片段，发送给别人之前可以先查看。

use std::fmt::Write as _;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 日志超过该大小（字节，经验值）时改名为 `crash.log.old`，重新开始写。
const MAX_BYTES: u64 = 64 << 10;
/// 一条 panic 信息最多保留的字符数（经验值）。
const MAX_MESSAGE: usize = 2000;

/// 安装 panic 钩子：先写日志，再执行原来的钩子；取不到状态目录时不做任何事。
pub fn install() {
    let Some(path) = log_path() else {
        return;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = append(&path, &describe(info));
        previous(info);
    }));
}

/// 返回日志文件的路径，与状态文件在同一目录。
fn log_path() -> Option<PathBuf> {
    Some(winc_config::state_path()?.parent()?.join("crash.log"))
}

/// 返回一次 panic 的日志内容。
fn describe(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(no message)");
    let location = info
        .location()
        .map_or_else(|| "(unknown)".to_owned(), ToString::to_string);
    let thread = std::thread::current();
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    entry(
        seconds,
        thread.name().unwrap_or("(unnamed)"),
        &location,
        message,
    )
}

/// 按固定格式写出一条日志；`seconds` 是 Unix 时间（秒）。
fn entry(seconds: u64, thread: &str, location: &str, message: &str) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "time: {seconds}\nversion: {}\nsystem: {} {}\nthread: {thread}\nlocation: {location}",
        winc_version::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    let shown: String = message.chars().take(MAX_MESSAGE).collect();
    let _ = writeln!(text, "message: {shown}\n");
    text
}

/// 把 `text` 追加到 `path`；文件已超过 [`MAX_BYTES`] 时先把它改名为备份。
fn append(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_BYTES) {
        std::fs::rename(path, path.with_extension("log.old"))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

#[cfg(test)]
mod tests;
