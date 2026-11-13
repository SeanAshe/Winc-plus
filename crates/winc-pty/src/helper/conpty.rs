//! Windows 上的伪终端：ConPTY（`CreatePseudoConsole`）和在其中运行的进程。
//!
//! 每个终端的 shell 放进一个作业对象，关掉时整个作业一起结束，不留孤儿进程；辅助进程意外退出时
//! 句柄随之关闭，系统同样结束其中的进程（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`）。ConPTY 的输出
//! 管道和 stdin 都不能 `poll`：各由一个后台线程读进缓冲区并唤醒事件循环（[`wait`]），事件循环从
//! 缓冲区取（[`Pty::read`]、[`read_stdin`]），没有数据时得到 `WouldBlock`。关闭伪终端可能要等
//! 控制台进程退出才返回，因此在后台线程上进行，事件循环不等它完成。

// ConPTY、进程和作业对象只有 Win32 的接口。
#![allow(unsafe_code)]

#[path = "conpty/api.rs"]
mod api;

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{COORD, HPCON};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

use winc_pty::protocol::Size;

/// 进程号（Windows 上即进程 ID）。
///
/// Windows 上没有前台进程组，[`Pty::foreground`] 总是返回 `None`；该类型只用于与其他平台的
/// 实现保持相同的签名。
pub type Pid = u32;

/// 找不到 PowerShell 时启动的 shell（`cmd.exe`）。
pub const FALLBACK_SHELL: &[u8] = b"cmd.exe";

/// 家目录对应的环境变量名（Windows 上没有 `HOME`，用 `USERPROFILE`）。
pub const HOME: &[u8] = b"USERPROFILE";

/// 启动一个进程需要的信息。
///
/// 字段与 Unix 上的实现相同；Windows 的命令行没有单独的 `argv[0]`，`argv0` 不使用。
pub struct Command<'a> {
    /// 程序的路径（已经在 `PATH` 中找好）。
    pub program: &'a [u8],
    /// 不用；命令行由 `program` 和 `args` 拼出。
    #[allow(dead_code)]
    pub argv0: &'a [u8],
    /// 参数，不含程序本身（`argv[0]`）。
    pub args: &'a [Vec<u8>],
    /// 工作目录。
    pub cwd: &'a [u8],
    /// 环境变量，每项是（键，值）。
    pub env: &'a [(Vec<u8>, Vec<u8>)],
    /// 伪终端的初始尺寸。
    pub size: Size,
}

/// 后台线程读进来、等事件循环取走的字节。
#[derive(Default)]
struct Pending {
    /// 读线程读到的、还没取走的字节。
    bytes: Vec<u8>,
    /// 读到了结尾（管道的另一端都关了）或出错。
    closed: bool,
}

/// 一个伪终端和在其中运行的进程。
pub struct Pty {
    /// 创建伪终端的接口。
    api: &'static api::Api,
    /// 伪终端；关掉以后为 `None`。
    console: Mutex<Option<HPCON>>,
    /// 写给伪终端的输入（管道的写入端）。
    input: File,
    /// 读线程放进来的输出。
    output: Arc<Mutex<Pending>>,
    /// shell 的进程。
    process: OwnedHandle,
    /// shell 和它启动的所有进程所在的作业。
    job: OwnedHandle,
}

impl Pty {
    /// 新建伪终端，在其中启动 `command`，并开始在后台读它的输出。
    ///
    /// # Errors
    ///
    /// 建管道、伪终端、作业对象或启动进程失败时返回对应的 `std::io::Error`。
    pub fn spawn(command: &Command<'_>) -> io::Result<Pty> {
        let (input_read, input) = pipe()?;
        let (output, output_write) = pipe()?;
        // SAFETY: `input_read`、`output_write` 是刚建好的管道句柄，在下面的 `drop` 之前一直有效。
        // 伪终端复制了这两个句柄，所以返回后关掉本进程的两端。
        let created = unsafe {
            api::create(
                coord(command.size),
                input_read.as_raw_handle() as HANDLE,
                output_write.as_raw_handle() as HANDLE,
            )
        };
        drop((input_read, output_write));
        let (api, console) = created.map_err(io::Error::from_raw_os_error)?;
        let pty = start(command, console).map(|(process, job)| Pty {
            api,
            console: Mutex::new(Some(console)),
            input: File::from(input),
            output: Arc::new(Mutex::new(Pending::default())),
            process,
            job,
        });
        let pty = match pty {
            Ok(pty) => pty,
            Err(error) => {
                // SAFETY: `console` 是上面的 `api` 刚建好、还没有交给任何线程或本结构体的伪终端，
                // 这里只关闭一次，之后不再使用。
                unsafe { close_in_background(api, console) };
                return Err(error);
            }
        };
        read_in_background(File::from(output), pty.output.clone());
        watch_exit(&pty.process);
        Ok(pty)
    }

    /// 取走读线程读到的输出；还没有时返回 `WouldBlock`，读到结尾时返回 `Ok(0)`。
    ///
    /// # Errors
    ///
    /// 暂时没有数据时返回 `io::ErrorKind::WouldBlock`。
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        take(&self.output, buf)
    }

    /// 写给伪终端；伪终端一直不读时会阻塞。
    ///
    /// # Errors
    ///
    /// 伪终端已经关了时返回对应的 `std::io::Error`。
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        (&self.input).write(buf)
    }

    /// 改伪终端的尺寸。
    pub fn resize(&self, size: Size) {
        if let Some(console) = *self.console.lock().unwrap_or_else(PoisonError::into_inner) {
            // SAFETY: 互斥锁保证同一时刻只有一个线程读这个值；值来自 `self.api`，而关闭它的
            // `hang_up` 会先把值从 `Option` 中取走，所以这里拿到的句柄仍然有效。
            unsafe { (self.api.resize)(console, coord(size)) };
        }
    }

    /// 挂断：关掉伪终端，其中的程序收到关闭控制台的通知。
    ///
    /// 关闭在后台线程上进行，本函数不等它完成；已经挂断过时什么也不做。
    pub fn hang_up(&self) {
        let console = self
            .console
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(console) = console {
            // SAFETY: 伪终端由 `self.api` 创建；值已经从 `Mutex` 中取走，只关闭一次，
            // `resize` 之后拿不到它。
            unsafe { close_in_background(self.api, console) };
        }
    }

    /// shell 退出以后挂断它留下的进程：同 [`Self::hang_up`]。
    pub fn hang_up_group(&self) {
        self.hang_up();
    }

    /// 挂断以后到期仍未退出时结束整个作业里的所有进程。
    ///
    /// `reaped` 在 Windows 上没有用：作业里的进程一起结束，不区分 shell 是否已经回收。
    pub fn kill(&self, _reaped: bool) {
        // SAFETY: 作业句柄由本结构体持有且没有关闭；作业里已经没有进程时调用没有效果。
        unsafe { TerminateJobObject(self.job.as_raw_handle() as HANDLE, 1) };
    }

    /// 返回 shell 的退出码；还在运行、或者它的输出尚未读到结尾时返回 `None`。
    ///
    /// shell 退出时，控制台可能尚未把它最后的输出写进管道。这时挂断伪终端并结束作业中剩下的
    /// 进程：控制台写完剩余的输出后退出，管道随之关闭，读线程读到结尾并唤醒事件循环，此后才
    /// 返回退出码。调用方因此能在报告退出之前取到全部输出。
    pub fn try_wait(&self) -> Option<i32> {
        let handle = self.process.as_raw_handle() as HANDLE;
        // SAFETY: 进程句柄有效；超时为 0，只查询不等待。
        if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
            return None;
        }
        let closed = (self.output.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .closed;
        if !closed {
            // 管道关闭之前每次查询都执行：两个操作在已经做过时都没有效果。
            self.hang_up();
            self.kill(true);
            return None;
        }
        let mut code = 0;
        // SAFETY: 进程句柄由本结构体持有、仍然有效，`code` 指向本地变量；进程已经退出，只读取
        // 退出码，不等待。
        unsafe { GetExitCodeProcess(handle, &mut code) };
        Some(code as i32)
    }

    /// 返回前台作业：Windows 上没有前台进程组，总是 `None`；关掉时整个作业一起结束。
    pub fn foreground_group(&self) -> Option<Pid> {
        None
    }

    /// 返回前台程序：Windows 上不查询，总是 `None`，标签名跟随标题。
    pub fn foreground(&self) -> Option<(Pid, String)> {
        None
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // 作业的句柄随后关闭，系统结束作业中剩下的进程。
        self.hang_up();
    }
}

/// 挂断前台作业（进程组）：Windows 上没有前台进程组，调用时什么也不做。
pub fn hang_up_job(_job: Pid) {}

/// 结束前台作业（进程组）：Windows 上没有前台进程组，调用时什么也不做。
pub fn kill_job(_job: Pid) {}

/// 返回前台作业（进程组）中是否还有进程：Windows 上没有前台进程组，总是返回 `false`。
pub fn job_alive(_job: Pid) -> bool {
    false
}

/// 返回用户的 shell 的路径：`PATH` 中有 PowerShell 7（`pwsh.exe`）时用它，否则用系统自带的
/// Windows PowerShell。
///
/// 两者都找不到时返回 `None`。
pub fn user_shell(env: &[(Vec<u8>, Vec<u8>)]) -> Option<Vec<u8>> {
    let var = |name: &str| {
        env.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
    };
    let path = var("PATH").unwrap_or_default();
    let pwsh = (path.split(';'))
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join("pwsh.exe"))
        .find(|candidate| candidate.is_file());
    let root = var("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let builtin = Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    pwsh.or_else(|| builtin.is_file().then_some(builtin))
        .map(|found| found.to_string_lossy().into_owned().into_bytes())
}

/// 唤醒事件循环用的标志：`bool` 为真表示有新的事情，等待中的线程由条件变量叫醒。
fn waker() -> &'static (Mutex<bool>, Condvar) {
    static WAKER: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
    WAKER.get_or_init(|| (Mutex::new(false), Condvar::new()))
}

/// 唤醒等在 [`wait`] 上的事件循环；没有线程在等待时只记下标志。
fn wake() {
    let (flag, condvar) = waker();
    *flag.lock().unwrap_or_else(PoisonError::into_inner) = true;
    condvar.notify_one();
}

/// 等到后台线程有了新的东西（stdin、某个终端的输出、进程退出），或者过了 `timeout`。
///
/// `timeout` 为 `None` 时一直等待，直到有新的东西。
pub fn wait(timeout: Option<Duration>) {
    let (flag, condvar) = waker();
    let mut woken = flag.lock().unwrap_or_else(PoisonError::into_inner);
    match timeout {
        Some(timeout) => {
            woken = condvar
                .wait_timeout_while(woken, timeout, |woken| !*woken)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        None => {
            woken = condvar
                .wait_while(woken, |woken| !*woken)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
    *woken = false;
}

/// 主程序发来的、还没取走的字节。
fn stdin() -> &'static Arc<Mutex<Pending>> {
    static STDIN: OnceLock<Arc<Mutex<Pending>>> = OnceLock::new();
    STDIN.get_or_init(Arc::default)
}

/// 开始在后台读 stdin（主程序的消息）。
pub fn read_stdin_in_background() {
    read_in_background(io::stdin(), stdin().clone());
}

/// 返回 stdin 是否有未读的字节，或者是否已经读到结尾。
pub fn stdin_pending() -> bool {
    let pending = stdin().lock().unwrap_or_else(PoisonError::into_inner);
    !pending.bytes.is_empty() || pending.closed
}

/// 取走 stdin 读到的字节；读到结尾时返回 `Ok(0)`。
///
/// # Errors
///
/// 暂时没有数据时返回 `io::ErrorKind::WouldBlock`。
pub fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    take(stdin(), buf)
}

/// 从 `pending` 取走最多 `buf.len()` 个字节，返回取到的个数。
fn take(pending: &Mutex<Pending>, buf: &mut [u8]) -> io::Result<usize> {
    let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
    if pending.bytes.is_empty() {
        return if pending.closed {
            Ok(0)
        } else {
            Err(io::ErrorKind::WouldBlock.into())
        };
    }
    let n = buf.len().min(pending.bytes.len());
    buf[..n].copy_from_slice(&pending.bytes[..n]);
    pending.bytes.drain(..n);
    Ok(n)
}

/// 在后台线程上一直读 `source`，读到的放进 `pending` 并唤醒事件循环。
///
/// 读到结尾或出错时标记结束，唤醒事件循环后线程退出。
fn read_in_background(mut source: impl Read + Send + 'static, pending: Arc<Mutex<Pending>>) {
    std::thread::spawn(move || {
        // 一次读 64 KiB：块大时取锁的次数少，块过大时读线程占用的内存多。
        let mut buf = vec![0; 64 << 10];
        loop {
            let read = source.read(&mut buf);
            let mut shared = pending.lock().unwrap_or_else(PoisonError::into_inner);
            match read {
                Ok(n) if n > 0 => shared.bytes.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                _ => {
                    shared.closed = true;
                    drop(shared);
                    wake();
                    break;
                }
            }
            drop(shared);
            wake();
        }
    });
}

/// 在后台线程上关闭伪终端 `console`，不等关闭完成；无法新建线程时在当前线程上关闭。
///
/// Windows 11 24H2 之前的系统接口要等控制台进程退出才返回，控制台进程又要等其中的程序响应关闭
/// 通知。事件循环不能停在这里：否则所有终端的输出、输入和退出通知都被推迟，到期结束进程
/// （[`Pty::kill`]）也无法执行。读线程一直读输出管道，控制台进程不会因管道写满而无法退出。
///
/// # Safety
///
/// `console` 必须是 `api` 创建、尚未关闭的伪终端，并且已经从保存它的位置取走：调用后任何线程
/// 都不再使用它。
unsafe fn close_in_background(api: &'static api::Api, console: HPCON) {
    let close = move || {
        // SAFETY: 调用方保证 `console` 由 `api` 创建、尚未关闭，并且不再有别的使用者。
        unsafe { (api.close)(console) };
    };
    if std::thread::Builder::new().spawn(close).is_err() {
        close();
    }
}

/// 进程 `process` 退出时唤醒事件循环。
fn watch_exit(process: &OwnedHandle) {
    // 复制句柄：调用方可能在等完之前丢弃 `Pty`，复制出的句柄由本线程持有到等待结束。
    let Ok(process) = process.try_clone() else {
        return;
    };
    std::thread::spawn(move || {
        // SAFETY: 句柄是本函数复制来的，本线程持有到等完（`INFINITE` 不超时），期间不会被关闭。
        unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, INFINITE) };
        wake();
    });
}

/// 建一对匿名管道，返回（读取端，写入端）。
///
/// 句柄不可被子进程继承。
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: 两个出参指向本地变量，`CreatePipe` 只写入两个新句柄；不给安全属性，句柄不可被
    // 子进程继承。
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 两个值都是 `CreatePipe` 刚写入的有效句柄，各交给一个 `OwnedHandle` 持有并在丢弃时
    // 关闭，之后不再使用原始值。
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

/// 在伪终端 `console` 中启动 `command` 并放进新建的作业，返回（进程，作业）。
fn start(command: &Command<'_>, console: HPCON) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut line = command_line(command);
    let env = environment_block(command.env);
    let cwd = wide(&String::from_utf8_lossy(command.cwd));

    // 进程的属性表：把伪终端交给新进程。先问要多大，再在这块内存上初始化。
    let mut size = 0;
    // SAFETY: 第一次调用只查所需大小：属性表指针为空，属性个数与调用一致，`size` 由调用写入。
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    let mut attributes = vec![0u8; size];
    let list = attributes.as_mut_ptr().cast::<c_void>();
    // SAFETY: `attributes` 正好有第一次调用要到的 `size` 字节，并活到函数结束，`list` 指向它的
    // 首字节；属性值按 `UpdateProcThreadAttribute` 的要求直接给伪终端句柄，值的大小是 `HPCON`。
    let ready = unsafe {
        InitializeProcThreadAttributeList(list, 1, 0, &mut size) != 0
            && UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const c_void,
                size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) != 0
    };
    if !ready {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: 全零是 `STARTUPINFOEXW` 的合法初值，随后填上大小和属性表。
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = list;
    // 辅助进程自己的 stdin、stdout 是连着主程序的管道：标明不传给新进程，否则新进程可能拿到
    // 这两个管道（写出的东西混进与主程序之间的消息），标准输入输出应当是伪终端。
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    // SAFETY: 全零是 `PROCESS_INFORMATION` 的合法初值，调用成功时由 `CreateProcessW` 填满。
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 命令行可写、以 NUL 结尾；环境块按 `CREATE_UNICODE_ENVIRONMENT` 的格式；目录以 NUL
    // 结尾；`startup` 和属性表在调用期间有效。先挂起进程，放进作业以后再运行，第一行代码就受
    // 作业约束。
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
            env.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    };
    // SAFETY: 属性表在上面初始化成功（否则已经返回），`CreateProcessW` 返回以后不再使用。
    unsafe { DeleteProcThreadAttributeList(list) };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `CreateProcessW` 写入的进程句柄和主线程句柄都归本函数所有，各交给一个
    // `OwnedHandle` 持有，之后不再使用 `info` 里的原始值。
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    let job = new_job();
    let assigned = job.as_ref().is_ok_and(|job| {
        // SAFETY: 两个句柄都由 `OwnedHandle` 持有、都没有关闭。
        unsafe {
            AssignProcessToJobObject(
                job.as_raw_handle() as HANDLE,
                process.as_raw_handle() as HANDLE,
            ) != 0
        }
    });
    // SAFETY: 线程句柄由 `thread` 持有、有效；只恢复一次，放不进作业时进程也照常运行。
    unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) };
    if !assigned {
        // 放不进作业（例如所在的作业不允许嵌套）：进程仍然运行，只是不受作业约束，关掉时只能靠
        // 关闭伪终端让它退出。
        eprintln!("winc-pty: 无法把终端里的进程放进作业");
    }
    Ok((process, job?))
}

/// 创建一个关掉句柄时结束其中所有进程的作业。
fn new_job() -> io::Result<OwnedHandle> {
    // SAFETY: 不给安全属性和名字，两个参数都是空指针。
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 句柄是刚建好的，交给 `OwnedHandle` 持有，之后不再用原始值。
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    // SAFETY: 全零是 `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` 的合法初值，随后只设 `LimitFlags`。
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: 作业句柄有效；结构的大小如实给出，指针指向本地变量，调用期间一直有效。
    let set = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// 返回伪终端的尺寸。
///
/// `COORD` 的两个字段是 `i16`，超出上限的列数、行数按上限截断。
fn coord(size: Size) -> COORD {
    COORD {
        X: size.cols.min(i16::MAX as u16) as i16,
        Y: size.rows.min(i16::MAX as u16) as i16,
    }
}

/// 返回以 NUL 结尾的 UTF-16 字符串。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// 返回以 NUL 结尾的命令行：程序和各参数按 `CommandLineToArgvW` 的规则加引号后用空格连起来。
fn command_line(command: &Command<'_>) -> Vec<u16> {
    let mut line = quote(&String::from_utf8_lossy(command.program));
    for arg in command.args {
        line.push(' ');
        line.push_str(&quote(&String::from_utf8_lossy(arg)));
    }
    wide(&line)
}

/// 按 `CommandLineToArgvW` 的规则给参数加引号。
///
/// 不含空白和引号的原样返回；否则用引号括起来，引号前的反斜杠加倍，引号本身前面再加一个
/// 反斜杠。
pub(crate) fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// 返回 `CREATE_UNICODE_ENVIRONMENT` 要的环境块。
///
/// `名=值` 各以 NUL 结尾，按名字（不分大小写）排序，最后再一个 NUL。
fn environment_block(env: &[(Vec<u8>, Vec<u8>)]) -> Vec<u16> {
    let mut pairs: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| {
            (
                String::from_utf8_lossy(k).into_owned(),
                String::from_utf8_lossy(v).into_owned(),
            )
        })
        .filter(|(k, _)| !k.is_empty() && !k.contains('='))
        .collect();
    pairs.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in pairs {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    // 空的环境块也要两个 NUL。
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
#[path = "conpty/tests.rs"]
mod tests;
