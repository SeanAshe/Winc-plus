//! 一直开着的外部程序：标准输出按需一段一段地读（`git log` 的提交列表）。
//!
//! 没有读者时，程序写满管道缓冲区（64 KB 左右）就阻塞在写操作上，不继续计算，也不多占内存；
//! 需要下一段时继续读取，不必像 `--skip` 那样从头重新执行；释放（`Drop`）时连同它启动的子进程
//! 一起终止并回收，不留僵尸进程。

use std::io::{self, BufRead, BufReader, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use winc_i18n::t;

use super::{MAX_OUTPUT, RunError, kill_group};

/// [`Stream::read_records`] 失败的原因。
#[derive(Debug)]
pub(crate) enum StreamError {
    /// 没能运行完：启动失败、读管道出错，或者超时被终止。
    Run(RunError),
    /// 程序退出了，退出码不是 0；内容是它的标准错误。
    Exited(Vec<u8>),
}

/// 一直开着、按需读输出的程序。
pub(crate) struct Stream {
    child: Child,
    stdout: BufReader<ChildStdout>,
    /// 在另一个线程上读完的标准错误（最多 [`MAX_OUTPUT`] 字节）：程序退出以后用来说明失败的原因。
    ///
    /// 不同时读，程序写满标准错误的管道时会停住，标准输出也就读不到了。
    stderr: Option<thread::JoinHandle<Vec<u8>>>,
    /// 标准输出读完了。
    eof: bool,
}

impl Stream {
    /// 启动 `command`：标准输入接空设备，标准输出、标准错误接管道；进程自成一个进程组
    /// （与 [`super::run`] 相同）。启动前去掉 `WSLENV` 中命令上没有设置的变量（见
    /// `wsl::drop_unset`）。
    ///
    /// # Errors
    ///
    /// 程序无法启动时返回 [`RunError::Spawn`]。
    pub(crate) fn spawn(command: &mut Command) -> Result<Self, RunError> {
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(command, 0);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(command, 0x0800_0000);
        crate::wsl::drop_unset(command);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(RunError::Spawn)?;
        let stdout =
            BufReader::new(child.stdout.take().ok_or_else(|| {
                RunError::Spawn(io::Error::other(t!("editor-process-no-stdout")))
            })?);
        let stderr = child.stderr.take().map(|pipe| {
            thread::spawn(move || {
                let mut out = Vec::new();
                let _ = pipe.take(MAX_OUTPUT as u64).read_to_end(&mut out);
                out
            })
        });
        Ok(Self {
            child,
            stdout,
            stderr,
            eof: false,
        })
    }

    /// 读取接下来 `count` 条以 `separator` 结尾的记录，最多等待 `timeout`；返回字节和是否读完。
    ///
    /// 字节含分隔符。输出读完时（不满 `count` 条）顺带等程序退出，退出码非 0 时返回标准错误的原因。
    ///
    /// # Errors
    ///
    /// 超时（终止程序）返回 [`RunError::Timeout`]，读管道出错返回 [`RunError::Spawn`]，程序失败时
    /// 返回 [`StreamError::Exited`]。
    pub(crate) fn read_records(
        &mut self,
        separator: u8,
        count: usize,
        timeout: Duration,
    ) -> Result<(Vec<u8>, bool), StreamError> {
        let pid = self.child.id();
        // 定时线程：到时间还没读够就终止程序，读管道的一端随即读到文件结尾。读完时释放 `done`，
        // 该线程立即返回；返回之前等它结束，保证程序已被回收，不会再终止一个可能被复用的进程号。
        let (done, wait) = mpsc::channel::<()>();
        let watchdog = thread::spawn(move || {
            let expired = matches!(
                wait.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Timeout)
            );
            if expired {
                kill_group(pid);
            }
            expired
        });

        let mut out = Vec::new();
        let mut read = 0;
        let mut failed = None;
        while read < count && !self.eof {
            match self.stdout.read_until(separator, &mut out) {
                Ok(0) => self.eof = true,
                Ok(_) if out.last() == Some(&separator) => read += 1,
                // 最后一条没有分隔符（只剩结尾的换行）：输出到头了。
                Ok(_) => self.eof = true,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    failed = Some(e);
                    self.eof = true;
                }
            }
        }
        drop(done);
        let expired = watchdog.join().unwrap_or(false);
        if expired {
            return Err(StreamError::Run(RunError::Timeout));
        }
        if let Some(e) = failed {
            return Err(StreamError::Run(RunError::Spawn(e)));
        }
        if self.eof {
            let status = self
                .child
                .wait()
                .map_err(|e| StreamError::Run(RunError::Spawn(e)))?;
            if !status.success() {
                let stderr = self.stderr.take().and_then(|t| t.join().ok());
                return Err(StreamError::Exited(stderr.unwrap_or_default()));
            }
        }
        Ok((out, self.eof))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // 已经退出、回收过的不再终止：进程号可能已经给了别的程序。
        if self.child.try_wait().is_ok_and(|s| s.is_none()) {
            kill_group(self.child.id());
        }
        let _ = self.child.wait();
    }
}
