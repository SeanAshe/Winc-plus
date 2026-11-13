//! 在文件所在的机器上运行的监视程序：启动、接收它报告的变化，以及另外监视公共 Git 目录。
//!
//! WSL 发行版内是程序 `winc-watch`，标准输入结束时自行退出，因此 winc 异常退出时不会留下
//! 进程。远端主机上由服务进程运行同样的监视，经连接读取它的输出，连接断开时由服务进程结束。
//! 消息格式见 `winc_watch` 的说明。

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::Pending;

/// 正常退出的最长等待时间（秒），超过时强制结束。
///
/// 程序读到标准输入结束后立即退出，这段时间用于 `wsl.exe` 转发。未经实测；等待在后台线程上进行，
/// 取值只决定异常情况下强制结束的时机。
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// 等待退出时检查的间隔（毫秒）。
const EXIT_POLL: Duration = Duration::from_millis(20);

/// 已启动，尚未开始监视。
const STARTING: u8 = 0;
/// 正在监视。
const LIVE: u8 = 1;
/// 监视过，之后退出了；或已由 [`Helper`] 释放。
const EXITED: u8 = 2;
/// 未开始监视就退出了（发行版配置不允许运行等），重试也不会成功。
const FAILED: u8 = 3;
/// 超出系统允许的监视数量，重试也不会成功。
const LIMITED: u8 = 4;

/// 运行中的监视程序。释放时结束它。
pub(super) struct Helper {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    /// 远端主机上的监视：释放时经它结束。
    remote: Option<winc_backend::remote::Stopper>,
    state: Arc<AtomicU8>,
}

impl Helper {
    /// 运行 `command`，在后台线程上把报告的路径（经 `to_local` 转换）交给 `record`。
    ///
    /// `command` 只需设置程序和参数。程序开始监视时要求重新扫描一次，弥补打开目录到开始监视之间的
    /// 变化；监视之后意外结束时同样要求一次。
    ///
    /// # Errors
    ///
    /// `command` 无法启动时返回错误。
    pub(super) fn spawn(
        mut command: Command,
        to_local: impl Fn(&str) -> PathBuf + Send + 'static,
        record: impl Fn(Report) + Send + 'static,
    ) -> io::Result<Helper> {
        // Windows 上图形程序启动命令行程序时默认弹出控制台窗口：`CREATE_NO_WINDOW`（0x0800_0000）
        // 可避免这一点。
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin 是管道");
        let stdout = child.stdout.take().expect("stdout 是管道");
        Ok(Helper {
            child: Some(child),
            stdin: Some(stdin),
            remote: None,
            state: read(stdout, to_local, record)?,
        })
    }

    /// 读取远端主机上的监视 `running` 的输出，其余同 [`Helper::spawn`]。不能另外监视其他目录。
    ///
    /// # Errors
    ///
    /// 无法创建读取的线程时返回错误。
    pub(super) fn remote(
        running: winc_backend::remote::Running,
        to_local: impl Fn(&str) -> PathBuf + Send + 'static,
        record: impl Fn(Report) + Send + 'static,
    ) -> io::Result<Helper> {
        let stopper = running.stopper();
        Ok(Helper {
            child: None,
            stdin: None,
            remote: Some(stopper),
            state: read(running, to_local, record)?,
        })
    }

    /// 程序是否正在监视。
    pub(super) fn live(&self) -> bool {
        self.state.load(Ordering::Relaxed) == LIVE
    }

    /// 程序是否在监视过之后退出了：发行版恢复后重新启动可能成功。
    pub(super) fn exited(&self) -> bool {
        self.state.load(Ordering::Relaxed) == EXITED
    }

    /// 让程序另外监视发行版内的目录 `path`。
    pub(super) fn watch(&mut self, path: &str) {
        self.send(b'W', path);
    }

    /// 让程序不再监视发行版内的目录 `path`。
    pub(super) fn unwatch(&mut self, path: &str) {
        self.send(b'U', path);
    }

    /// 向代替程序的测试脚本发送一行，推进脚本。
    #[cfg(all(test, unix))]
    pub(super) fn send_line(&mut self) {
        if let Some(stdin) = &mut self.stdin {
            let _ = stdin.write_all(b"go\n").and_then(|()| stdin.flush());
        }
    }

    /// 发送一条命令；程序已退出时什么也不做，退出由读线程发现。
    fn send(&mut self, kind: u8, path: &str) {
        let Some(stdin) = &mut self.stdin else {
            return;
        };
        let mut message = vec![kind];
        message.extend_from_slice(path.as_bytes());
        message.push(0);
        let _ = stdin.write_all(&message).and_then(|()| stdin.flush());
    }
}

impl Drop for Helper {
    /// 关闭标准输入让程序自行退出，超时后才强制结束：结束 `wsl.exe` 不一定能结束发行版内的进程。
    /// 远端主机上的监视由服务进程结束。
    fn drop(&mut self) {
        self.state.store(EXITED, Ordering::Relaxed);
        if let Some(remote) = self.remote.take() {
            remote.stop();
        }
        drop(self.stdin.take());
        let Some(mut child) = self.child.take() else {
            return;
        };
        thread::spawn(move || {
            let deadline = Instant::now() + EXIT_GRACE;
            while Instant::now() < deadline {
                if !matches!(child.try_wait(), Ok(None)) {
                    return;
                }
                thread::sleep(EXIT_POLL);
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

/// 在后台线程上读取监视程序的输出 `stdout`，把报告的路径（经 `to_local` 转换）交给 `record`，
/// 返回程序的状态。
///
/// 程序开始监视时要求重新扫描一次，弥补打开目录到开始监视之间的变化；监视之后意外结束时同样
/// 要求一次。
fn read(
    stdout: impl io::Read + Send + 'static,
    to_local: impl Fn(&str) -> PathBuf + Send + 'static,
    record: impl Fn(Report) + Send + 'static,
) -> io::Result<Arc<AtomicU8>> {
    let state = Arc::new(AtomicU8::new(STARTING));
    let shared = Arc::clone(&state);
    thread::Builder::new()
        .name("winc-watch 读".into())
        .spawn(move || {
            for message in BufReader::new(stdout).split(0).map_while(Result::ok) {
                match parse(&message, &to_local) {
                    Some(Report::Started) => {
                        let _ = shared.compare_exchange(
                            STARTING,
                            LIVE,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        );
                        record(Report::Rescan);
                    }
                    Some(Report::Limit) => {
                        shared.store(LIMITED, Ordering::Relaxed);
                        record(Report::Limit);
                    }
                    Some(report) => record(report),
                    None => {}
                }
            }
            // 由 `Helper` 释放时状态已经是 `EXITED`，不再报告。
            let exchange =
                |from, to| shared.compare_exchange(from, to, Ordering::Relaxed, Ordering::Relaxed);
            if exchange(LIVE, EXITED).is_ok() {
                record(Report::Rescan);
            } else {
                let _ = exchange(STARTING, FAILED);
            }
        })?;
    Ok(state)
}

/// 监视程序报告的一项内容。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Report {
    /// 开始监视。
    Started,
    /// 路径发生变化。
    Path(PathBuf),
    /// 需要重新扫描整个目录。
    Rescan,
    /// 超出系统允许的监视数量。
    Limit,
}

/// 解析一条不含结尾 NUL 的消息；未知的消息返回 `None`。
fn parse(message: &[u8], to_local: &impl Fn(&str) -> PathBuf) -> Option<Report> {
    match message.split_first()? {
        (b'S', _) => Some(Report::Started),
        (b'P', path) => Some(Report::Path(to_local(&String::from_utf8_lossy(path)))),
        (b'R', _) => Some(Report::Rescan),
        (b'E', _) => Some(Report::Limit),
        _ => None,
    }
}

/// 把 `report` 记入 `pending`，需要唤醒界面线程时返回 `true`。
pub(super) fn apply(pending: &Mutex<Pending>, base: &Path, report: Report) -> bool {
    let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
    match report {
        Report::Started => {}
        Report::Path(path) => pending.add_paths(base, [path]),
        Report::Rescan => pending.rescan = true,
        Report::Limit => pending.limit = true,
    }
    pending.wake_due()
}
