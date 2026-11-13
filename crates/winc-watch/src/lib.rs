//! 在文件所在的机器上监视目录，把变化经标准输出报告给 winc。
//!
//! 用于 winc 自己监视不到的目录：Windows 通过文件共享访问 WSL 发行版中的目录，变化通知不一定
//! 送达；远端主机上的目录则根本不在本机。在发行版内由程序 `winc-watch <目录>` 运行，在远端
//! 主机上由服务进程运行（见 [`run`]）。用 inotify 递归监视目录，不写任何文件。
//!
//! 消息以 NUL 结尾，第一个字节是类型，路径按原样的字节传输：
//!
//! ```text
//! 标准输出：  S            开始监视
//!            P<路径>       路径发生变化
//!            R            通知丢失或监视出错，需要重新扫描；两次之间至少间隔 `RESCAN_INTERVAL`
//!            E            超出系统允许的监视数量，随后退出
//! 标准输入：  W<路径>       另外监视该目录
//!            U<路径>       不再监视该目录
//! ```

use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use notify::event::EventKind;
use notify::{ErrorKind, RecursiveMode, Watcher as _};

/// 两次要求重新扫描之间的最短间隔（秒）。
///
/// 重新扫描要经文件共享遍历整个目录，持续出错时不能连续进行。未经实测，取值只决定通知丢失后界面最迟
/// 多久更新。
const RESCAN_INTERVAL: Duration = Duration::from_secs(5);

/// 一次通知要做的事。
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// 发送这些消息（可以为空）。
    Send(Vec<u8>),
    /// 要求重新扫描。
    Rescan,
    /// 超出系统允许的监视数量。
    Limit,
}

/// 重新扫描请求的限流状态。
#[derive(Default)]
struct Throttle {
    /// 上一次发送的时刻。
    last: Option<Instant>,
    /// 是否已安排延迟发送。
    scheduled: bool,
}

/// 监视 `root` 并把变化写到标准输出。
///
/// `commands` 为真时从标准输入读取命令，标准输入结束时返回（发行版内的用法：winc 退出时随之
/// 退出）。为假时不读标准输入，一直监视到进程被结束（远端主机上的用法：标准输入在启动时就已
/// 关闭，由服务进程在连接断开或不再需要时结束本进程）。
///
/// # Errors
///
/// 无法创建监视器或无法监视 `root` 时返回 `notify::Error`；超出系统允许的监视数量时先报告
/// `E` 再返回错误。
pub fn run(root: &Path, commands: bool) -> notify::Result<()> {
    let throttle = Arc::new(Mutex::new(Throttle::default()));
    let mut watcher = notify::recommended_watcher(move |event| match classify(&event) {
        Outcome::Send(messages) => send(&messages),
        Outcome::Rescan => request_rescan(&throttle),
        Outcome::Limit => {
            send(b"E\0");
            std::process::exit(1);
        }
    })?;
    if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
        if matches!(error.kind, ErrorKind::MaxFilesWatch) {
            send(b"E\0");
        }
        return Err(error);
    }
    send(b"S\0");
    if !commands {
        // 监视在 `notify` 自己的线程上进行；这里只需保持 `watcher` 存活。
        loop {
            std::thread::park();
        }
    }

    let mut input = io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(0, &mut line) {
            Ok(0) | Err(_) => return Ok(()),
            Ok(_) => {}
        }
        // 没有结尾 NUL 的是被截断的命令。
        let Some((kind, path)) = line.strip_suffix(&[0]).and_then(<[u8]>::split_first) else {
            continue;
        };
        let path = path_from_bytes(path);
        // 附加目录是可选的：监视失败时打开的目录仍然有效。
        let _ = match kind {
            b'W' => watcher.watch(&path, RecursiveMode::Recursive),
            b'U' => watcher.unwatch(&path),
            _ => Ok(()),
        };
    }
}

/// 判断一次通知要做的事；打开、读取文件不算变化，没有消息要发送。
fn classify(event: &notify::Result<notify::Event>) -> Outcome {
    match event {
        Ok(event) if event.need_rescan() => Outcome::Rescan,
        Ok(event) if matches!(event.kind, EventKind::Access(_)) => Outcome::Send(Vec::new()),
        Ok(event) => Outcome::Send(
            event
                .paths
                .iter()
                .flat_map(|path| {
                    let mut message = vec![b'P'];
                    message.extend_from_slice(&path_bytes(path));
                    message.push(0);
                    message
                })
                .collect(),
        ),
        Err(error) if matches!(error.kind, ErrorKind::MaxFilesWatch) => Outcome::Limit,
        Err(_) => Outcome::Rescan,
    }
}

/// 距离下一次可以要求重新扫描还要等多久；上一次在 `last`，现在是 `now`。
fn rescan_delay(last: Option<Instant>, now: Instant) -> Duration {
    last.map_or(Duration::ZERO, |last| {
        RESCAN_INTERVAL.saturating_sub(now.saturating_duration_since(last))
    })
}

/// 要求重新扫描：距上一次足够久时立即发送，否则安排在间隔结束时发送一次。
fn request_rescan(throttle: &Arc<Mutex<Throttle>>) {
    let lock = |throttle: &Mutex<Throttle>| {
        let mut state = throttle.lock().unwrap_or_else(PoisonError::into_inner);
        state.last = Some(Instant::now());
        state.scheduled = false;
    };
    let delay = {
        let mut state = throttle.lock().unwrap_or_else(PoisonError::into_inner);
        if state.scheduled {
            return;
        }
        let delay = rescan_delay(state.last, Instant::now());
        state.scheduled = !delay.is_zero();
        delay
    };
    if delay.is_zero() {
        lock(throttle);
        send(b"R\0");
        return;
    }
    let throttle = Arc::clone(throttle);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        lock(&throttle);
        send(b"R\0");
    });
}

/// 发送消息；winc 已退出、管道已关闭时结束进程。
fn send(message: &[u8]) {
    if message.is_empty() {
        return;
    }
    let mut output = io::stdout().lock();
    if output
        .write_all(message)
        .and_then(|()| output.flush())
        .is_err()
    {
        std::process::exit(0);
    }
}

/// 返回路径的字节；Linux 上文件名不一定是 UTF-8，按原样传输。
#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

/// 返回路径的字节；其他平台只用于开发时运行，按 UTF-8 有损转换。
#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// 把消息中的字节换成路径。
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec()).into()
}

/// 把消息中的字节换成路径；其他平台只用于开发时运行，按 UTF-8 有损转换。
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    OsString::from(String::from_utf8_lossy(bytes).into_owned()).into()
}

#[cfg(test)]
mod tests;
