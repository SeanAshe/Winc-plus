//! 运行外部程序：写入标准输入，同时收集标准输出和标准错误；超时或取消时连同它启动的子进程
//! 一起终止。

use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use winc_i18n::t;

/// 标准输出、标准错误各自允许收集的最大字节数（64 MiB，经验值）。
///
/// 任一路超过该值即视为失败（[`RunError::TooLarge`]）并终止进程组，防止失控的程序耗尽内存。
pub(crate) const MAX_OUTPUT: usize = 64 << 20;
/// 程序退出后，等待其遗留的子进程释放输出管道的时长（200 毫秒，经验值）；超时后终止进程组。
const GRACE: Duration = Duration::from_millis(200);
/// 启动程序最多尝试的次数（经验值）。
const SPAWN_TRIES: u32 = 5;

/// 程序的运行结果：退出状态和捕获的输出。
#[derive(Debug)]
pub(crate) struct Output {
    /// 程序的退出状态。
    pub status: ExitStatus,
    /// 标准输出的内容（字节）。
    pub stdout: Vec<u8>,
    /// 标准错误的内容（字节）；调用方没有把它设为 `Stdio::piped()` 时为空。
    pub stderr: Vec<u8>,
}

/// 运行外部程序时的失败原因。
#[derive(Debug)]
pub(crate) enum RunError {
    /// 无法启动程序（如找不到、没有执行权限），或无法获取退出状态。
    Spawn(io::Error),
    /// 超过限定时间仍未结束。
    Timeout,
    /// 输出超过了 [`MAX_OUTPUT`]。
    TooLarge,
    /// 已被 [`Cancel`] 取消。
    Cancelled,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Spawn(e) => write!(f, "{e}"),
            RunError::Timeout => f.write_str(t!("editor-process-timeout")),
            RunError::TooLarge => f.write_str(t!("editor-process-too-large")),
            RunError::Cancelled => f.write_str(t!("editor-process-cancelled")),
        }
    }
}

/// 用于取消一次运行的句柄。
///
/// 取消在程序启动之前也有效：程序启动后立即被终止。
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<Mutex<CancelState>>);

#[derive(Debug, Default)]
struct CancelState {
    cancelled: bool,
    /// 正在运行的程序的进程号；程序自成一个进程组，进程组号与进程号相同。
    pid: Option<u32>,
}

impl Cancel {
    /// 请求取消：标记为已取消，并终止正在运行的进程组。
    pub fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        if let Some(pid) = state.pid {
            kill_group(pid);
        }
    }

    /// 是否已经请求取消。
    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// 是否成功登记了刚启动的程序；已经请求取消时，立即终止它的进程组并返回 `false`。
    fn started(&self, pid: u32) -> bool {
        let mut state = self.lock();
        if state.cancelled {
            kill_group(pid);
            return false;
        }
        state.pid = Some(pid);
        true
    }

    /// 清除登记的进程号，此后的取消不再杀进程组。
    fn finished(&self) {
        self.lock().pid = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CancelState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 启动 `command`；程序文件暂时被占用（`ETXTBSY`）时稍等再试，最多试 [`SPAWN_TRIES`] 次。
///
/// 启动前去掉 `WSLENV` 中命令上没有设置的变量（见 `wsl::drop_unset`）。
///
/// 程序文件刚写完时，同时启动的另一个进程可能在 exec 之前仍继承着它的可写描述符。这种占用
/// 只持续片刻，不能当作程序无法运行。
fn spawn(command: &mut Command) -> std::io::Result<Child> {
    /// Unix 上 `ETXTBSY` 的错误码。
    const TEXT_BUSY: i32 = 26;
    crate::wsl::drop_unset(command);
    let mut tries = 1;
    loop {
        match command.spawn() {
            Err(e) if cfg!(unix) && e.raw_os_error() == Some(TEXT_BUSY) && tries < SPAWN_TRIES => {
                std::thread::sleep(Duration::from_millis(10 * u64::from(tries)));
                tries += 1;
            }
            result => return result,
        }
    }
}

/// 运行 `command`，最多等待 `timeout`，返回退出状态和捕获的输出。
///
/// 标准输入不继承 winc 自己的；程序自成一个进程组，超时、取消、输出过多时终止整个进程组。
///
/// # Errors
///
/// 无法启动、超时、输出超过 [`MAX_OUTPUT`]、已取消时，分别返回 [`RunError::Spawn`]、
/// [`RunError::Timeout`]、[`RunError::TooLarge`]、[`RunError::Cancelled`]。
pub(crate) fn run(
    command: &mut Command,
    input: Vec<u8>,
    timeout: Duration,
    cancel: &Cancel,
) -> Result<Output, RunError> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    // Windows 上图形程序启动命令行工具时，默认会弹出控制台窗口；`CREATE_NO_WINDOW`（0x0800_0000）
    // 可避免这一点。
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(command, 0x0800_0000);
    let stdin = if input.is_empty() {
        // 不继承 winc 自己的标准输入：从终端启动 winc 时，读标准输入的 hooks、凭据助手会一直
        // 等终端输入，直到超时。
        Stdio::null()
    } else {
        Stdio::piped()
    };
    command.stdin(stdin).stdout(Stdio::piped());
    let mut child = spawn(command).map_err(RunError::Spawn)?;
    let pid = child.id();
    let deadline = Instant::now() + timeout;
    let too_large = Arc::new(AtomicBool::new(false));

    if !cancel.started(pid) {
        let _ = child.wait();
        return Err(RunError::Cancelled);
    }
    // 无论从哪条路径返回（包括等待输出超时而提前返回），都要清除登记的进程号：进程已被回收，
    // 其号码可能被其他程序复用，之后再取消时不能去杀该进程组。
    struct Finished<'a>(&'a Cancel);
    impl Drop for Finished<'_> {
        fn drop(&mut self) {
            self.0.finished();
        }
    }
    let finished = Finished(cancel);

    if let Some(mut stdin) = child.stdin.take() {
        // 输入与每路输出各占一个线程：先写完输入再读输出会互相等待而锁死。
        // 程序没读完输入就退出（报错或根本不读输入）时，写入会失败；忽略该错误，结果以退出状态
        // 为准。
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .map(|pipe| {
        let (sender, receiver) = mpsc::channel();
        let too_large = too_large.clone();
        if let Some(pipe) = pipe {
            thread::spawn(move || {
                let _ = sender.send(read_capped(pipe, pid, &too_large));
            });
        } else {
            let _ = sender.send(Vec::new());
        }
        receiver
    })
    .collect();

    let (sender, exited) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(child.wait());
    });

    let mut timed_out = false;
    let status = match exited.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(status) => status,
        Err(_) => {
            timed_out = true;
            kill_group(pid);
            exited.recv().map_err(|_| RunError::Timeout)?
        }
    };

    // 程序退出后，输出管道可能仍被它遗留在后台的子进程占用：先等待 `GRACE`，仍未释放就终止
    // 进程组，再最多等到 `deadline`（至少再等 `GRACE`）。
    let mut outputs = Vec::new();
    for receiver in &readers {
        let output = match receiver.recv_timeout(GRACE) {
            Ok(output) => output,
            Err(_) => {
                kill_group(pid);
                let left = deadline.saturating_duration_since(Instant::now());
                receiver
                    .recv_timeout(left.max(GRACE))
                    .map_err(|_| RunError::Timeout)?
            }
        };
        outputs.push(output);
    }
    drop(finished);

    if cancel.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    if timed_out {
        return Err(RunError::Timeout);
    }
    if too_large.load(Ordering::Relaxed) {
        return Err(RunError::TooLarge);
    }
    let status = status.map_err(RunError::Spawn)?;
    let stderr = outputs.pop().unwrap_or_default();
    let stdout = outputs.pop().unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// 读取管道直到关闭，最多读 [`MAX_OUTPUT`] 字节，返回已读到的内容。
///
/// 超过上限时设置 `too_large` 并终止进程组 `pid`。
fn read_capped(mut pipe: impl Read, pid: u32, too_large: &AtomicBool) -> Vec<u8> {
    let mut out = Vec::new();
    // 每次读 64 KiB（经验值），与常见的管道缓冲区大小相当，读满一次的等待即可返回。
    let mut buffer = vec![0; 64 << 10];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) if out.len() + n > MAX_OUTPUT => {
                too_large.store(true, Ordering::Relaxed);
                kill_group(pid);
                break;
            }
            Ok(n) => out.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    out
}

/// 向进程组 `pid` 发送 `SIGKILL`；进程组已不存在时没有效果。
#[cfg(unix)]
// 本文件中唯一的 `unsafe` 是下面对 `libc::kill` 的调用，因此只在此函数上放开 `unsafe_code`。
#[allow(unsafe_code)]
fn kill_group(pid: u32) {
    // SAFETY: `libc::kill` 的参数都是整数，不涉及指针、生命周期或对齐，
    // 任意参数值都不会造成内存不安全。`-pid` 表示向进程组发信号：程序以 `process_group(0)`
    // 启动，组号等于它的进程号。进程组不存在时 `kill` 只返回错误，这里忽略返回值。
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

/// 用 `taskkill /T /F` 终止进程 `pid` 及其启动的全部子进程。
#[cfg(not(unix))]
fn kill_group(pid: u32) {
    let mut command = Command::new("taskkill");
    command
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let _ = command.status();
}

mod stream;
pub(crate) use stream::{Stream, StreamError};

#[cfg(all(test, unix))]
mod tests;
