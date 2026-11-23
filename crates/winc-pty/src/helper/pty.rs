//! 伪终端和子进程：打开伪终端、fork 出 shell、读写、改尺寸、挂断、回收，以及 `poll` 和 SIGCHLD。
//!
//! 整个 crate 中 Unix 上只有本模块用到 `unsafe`。辅助进程是单线程的，fork 之后子进程里只调用异步
//! 信号安全的函数（`setsid`、`ioctl`、`dup2`、`chdir`、`execve`、`write`、`_exit`），要用的字符串
//! 都在 fork 之前准备好。

// 伪终端、fork 和信号处理只能调用 libc。
#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};

use winc_pty::protocol::Size;

/// 进程号；shell 是会话首进程，进程组号与 shell 的进程号相同。
pub type Pid = libc::pid_t;

/// 没有 `$SHELL`、用户数据库中也没有登录 shell 时使用的 shell（`/bin/sh`）。
pub const FALLBACK_SHELL: &[u8] = b"/bin/sh";

/// 家目录对应的环境变量名（`HOME`；Windows 上改用 `USERPROFILE`）。
pub const HOME: &[u8] = b"HOME";

/// 一个伪终端和在其中的进程组。
pub struct Pty {
    /// 主端的描述符，读写和改尺寸都通过它。
    master: OwnedFd,
    /// 子进程（shell）的进程号；该子进程也是会话和进程组的首进程。
    pid: libc::pid_t,
}

/// 要启动的程序及其参数、工作目录、环境和伪终端尺寸。
pub struct Command<'a> {
    /// 程序的路径（已经在 `PATH` 里找好）。
    pub program: &'a [u8],
    /// 程序名（`argv[0]`），登录 shell 前面带 `-`。
    pub argv0: &'a [u8],
    /// 参数，不含 `argv[0]`（程序名）。
    pub args: &'a [Vec<u8>],
    /// 工作目录。
    pub cwd: &'a [u8],
    /// 环境变量，每项是（键，值）。
    pub env: &'a [(Vec<u8>, Vec<u8>)],
    /// 伪终端的初始尺寸。
    pub size: Size,
}

fn cstring(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "参数中包含 NUL 字符"))
}

/// 系统调用返回负数时转成 `std::io::Error`，否则原样返回。
fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

/// 把协议里的尺寸转换成 `winsize`：`width`、`height` 对应 `ws_xpixel`、`ws_ypixel`（像素）。
fn winsize(size: Size) -> libc::winsize {
    libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.width,
        ws_ypixel: size.height,
    }
}

/// 告诉终端驱动输入是 UTF-8（`IUTF8`）。
///
/// 按行输入（`cat`、`read`、密码提示）时退格删掉整个字，不会只删最后一个字节、留下半个汉字。
/// 系统默认的设置里没有这一项。
fn set_utf8(fd: RawFd) -> io::Result<()> {
    // SAFETY: termios 是普通的 C 结构体，全零是合法的值；tcgetattr 写满它。
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: fd 是打开的终端，termios 指向本地变量。
    check(unsafe { libc::tcgetattr(fd, &mut termios) })?;
    termios.c_iflag |= libc::IUTF8;
    // SAFETY: `fd` 仍是上一步读到的打开的终端，`termios` 指向本地变量，内容已由上一步填满。
    check(unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) })?;
    Ok(())
}

/// 设置描述符的 close-on-exec 标志和非阻塞标志。
fn set_flags(fd: RawFd, cloexec: bool, nonblock: bool) -> io::Result<()> {
    // SAFETY: `fd` 是调用方传入的、已经打开的伪终端描述符，`fcntl` 只读写它的标志位，
    // 不访问其他内存。
    unsafe {
        if cloexec {
            let flags = check(libc::fcntl(fd, libc::F_GETFD))?;
            check(libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC))?;
        }
        if nonblock {
            let flags = check(libc::fcntl(fd, libc::F_GETFL))?;
            check(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
        }
    }
    Ok(())
}

/// 启动失败且错误码是内核内部错误码时的最多重试次数（次，取自调参）。
const SPAWN_RETRIES: u32 = 5;

impl Pty {
    /// 打开伪终端，在其中启动 `command`。
    ///
    /// 程序启动失败（不存在、没有执行权限）也在这里报错：子进程 exec 失败时通过一条 CLOEXEC 管道
    /// 把 errno 告诉父进程，exec 成功时管道随之关闭，父进程读到 0 字节。
    ///
    /// # Errors
    ///
    /// 打开伪终端、创建管道或 fork 失败，程序不存在或没有执行权限时返回对应的 `std::io::Error`。
    pub fn spawn(command: &Command<'_>) -> io::Result<Pty> {
        let mut tries = 0;
        loop {
            match Self::spawn_once(command) {
                // 负数是系统内核内部的错误码：macOS 上多个进程同时打开伪终端时偶尔返回
                // `EREDRIVEOPEN`（-6），它要求内核重新打开，重试即可。
                Err(e) if tries < SPAWN_RETRIES && e.raw_os_error().is_some_and(|c| c < 0) => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(5 * u64::from(tries)));
                }
                result => return result,
            }
        }
    }

    /// 打开伪终端并启动 `command` 一次，见 [`Pty::spawn`]。
    fn spawn_once(command: &Command<'_>) -> io::Result<Pty> {
        let program = cstring(command.program)?;
        let mut argv = vec![cstring(command.argv0)?];
        for arg in command.args {
            argv.push(cstring(arg)?);
        }
        let mut env = Vec::with_capacity(command.env.len());
        for (key, value) in command.env {
            let mut entry = key.clone();
            entry.push(b'=');
            entry.extend_from_slice(value);
            env.push(cstring(&entry)?);
        }
        let cwd = cstring(command.cwd)?;
        let argv_ptrs: Vec<*const libc::c_char> = argv
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let env_ptrs: Vec<*const libc::c_char> = env
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        let mut master: RawFd = -1;
        let mut slave: RawFd = -1;
        let mut size = winsize(command.size);
        // SAFETY: 两个出参指向本地变量；终端名不要（null），termios 用默认（null），
        // 尺寸指向本地变量。
        check(unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                // macOS 的声明是可变指针，其他平台是只读指针，原始指针两边都接受。
                &raw mut size,
            )
        })?;
        let (master, slave) =
            // SAFETY: openpty 成功后两个描述符都是新打开的，归这里所有，只接管一次。
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        set_flags(master.as_raw_fd(), true, false)?;
        set_flags(slave.as_raw_fd(), true, false)?;
        set_utf8(slave.as_raw_fd())?;

        let mut pipe = [0; 2];
        // SAFETY: `pipe` 指向长度为 2 的本地数组，只写入两个新描述符，不读其他内存。
        check(unsafe { libc::pipe(pipe.as_mut_ptr()) })?;
        let (error_read, error_write) =
            // SAFETY: pipe 成功后两个描述符都是新打开的，归这里所有，只接管一次。
            unsafe { (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])) };
        set_flags(error_read.as_raw_fd(), true, false)?;
        set_flags(error_write.as_raw_fd(), true, false)?;

        // SAFETY: 辅助进程是单线程的；子进程里只调用异步信号安全的函数，用到的指针都在 fork 前
        // 准备好，exec 或 _exit 之前一直有效。
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // SAFETY: 这里是 fork 出来的子进程，描述符和字符串都继承自父进程且在 fork 前准备好；
            // 函数不返回。
            unsafe {
                child(
                    slave.as_raw_fd(),
                    error_write.as_raw_fd(),
                    &cwd,
                    &program,
                    &argv_ptrs,
                    &env_ptrs,
                )
            }
        }

        drop(slave);
        drop(error_write);
        let mut errno = [0u8; 4];
        let read = read_full(error_read.as_raw_fd(), &mut errno);
        if read == 4 {
            // exec 失败，子进程已经 _exit，回收掉。
            let mut status = 0;
            // SAFETY: `pid` 是上面 `fork` 成功返回的子进程，还没有被回收；`status` 指向本地变量。
            unsafe { libc::waitpid(pid, &mut status, 0) };
            return Err(io::Error::from_raw_os_error(i32::from_ne_bytes(errno)));
        }
        set_flags(master.as_raw_fd(), false, true)?;
        Ok(Pty { master, pid })
    }

    /// 读程序的输出。
    ///
    /// 没有数据时返回 `WouldBlock`；从端都关了时返回 `Ok(0)`，macOS 上这时读到的是 EIO，一并
    /// 当作结束。
    ///
    /// # Errors
    ///
    /// 没有数据（`WouldBlock`）或读取失败时返回对应的 `std::io::Error`。
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: 缓冲区来自切片，长度如实给出。
        let n = unsafe { libc::read(self.fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EIO) => Ok(0),
            _ => Err(error),
        }
    }

    /// 写入按键、粘贴和对查询的回答。
    ///
    /// 伪终端的缓冲很小，写不进去时返回 `WouldBlock`，剩下的等可写时再写。
    ///
    /// # Errors
    ///
    /// 缓冲写满（`WouldBlock`）或写入失败时返回对应的 `std::io::Error`。
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: 缓冲区来自切片，长度如实给出。
        let n = unsafe { libc::write(self.fd(), buf.as_ptr().cast(), buf.len()) };
        if n >= 0 {
            Ok(n as usize)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// 改尺寸，内核给前台进程组发 SIGWINCH。
    pub fn resize(&self, size: Size) {
        let size = winsize(size);
        // SAFETY: TIOCSWINSZ 读一个 winsize，指向本地变量。
        unsafe { libc::ioctl(self.fd(), libc::TIOCSWINSZ, &size) };
    }

    /// 挂断：给 shell 和它的进程组发 SIGHUP。
    pub fn hang_up(&self) {
        self.signal(libc::SIGHUP);
    }

    /// 挂断以后到期仍未退出：发 SIGKILL。
    ///
    /// `reaped` 为真（shell 已回收）时 shell 的 pid 可能已经给了别的进程，只发给进程组。
    ///
    /// 这时进程号又有了进程（[`taken_over`]）就不发：进程组已经散了，号码给了别的程序。
    pub fn kill(&self, reaped: bool) {
        if reaped {
            if !taken_over(self.pid, None) {
                self.signal_group_only(libc::SIGKILL);
            }
        } else {
            self.signal(libc::SIGKILL);
        }
    }

    /// 给整个进程组和 shell 自身发信号（shell 是会话首进程，进程组号就是它的 pid）。
    pub fn signal(&self, signal: libc::c_int) {
        // SAFETY: `self.pid` 是 `fork` 返回的子进程号，为正数，取负后表示发给整个进程组，不会是
        // 发给所有进程的 -1；`kill` 只发信号、不访问内存。进程组已经不存在时调用失败，可以忽略。
        unsafe {
            libc::kill(-self.pid, signal);
            libc::kill(self.pid, signal);
        }
    }

    /// shell 已经回收以后，挂断它留下的进程组。
    ///
    /// 只发给进程组：shell 的 pid 回收后可能已经给了别的进程，不能再单独发给它。
    pub fn hang_up_group(&self) {
        self.signal_group_only(libc::SIGHUP);
    }

    /// 只给 shell 的进程组发信号，不单独发给 shell（用于 shell 已经回收以后，原因同
    /// [`Self::hang_up_group`]）。
    pub fn signal_group_only(&self, signal: libc::c_int) {
        signal_group(self.pid, signal);
    }

    /// 返回 shell 的退出码并回收子进程；还在运行时返回 `None`。
    ///
    /// 被信号杀掉时退出码是 128 加信号编号。
    pub fn try_wait(&self) -> Option<i32> {
        let mut status = 0;
        // SAFETY: 只回收 `fork` 返回的这一个子进程，`status` 指向本地变量。
        let pid = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
        if pid != self.pid {
            return None;
        }
        Some(if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else if libc::WIFSIGNALED(status) {
            128 + libc::WTERMSIG(status)
        } else {
            0
        })
    }

    /// 返回前台作业的进程组号。
    ///
    /// shell 开着作业控制时前台程序在自己的进程组里，[`Self::signal`] 发不到，因此单独查询；
    /// 前台就是 shell 自己、或查不到时返回 `None`。
    pub fn foreground_group(&self) -> Option<libc::pid_t> {
        // SAFETY: tcgetpgrp 只查询，主端描述符有效。
        let pgid = unsafe { libc::tcgetpgrp(self.fd()) };
        (pgid > 0 && pgid != self.pid).then_some(pgid)
    }

    /// 返回前台进程组的组长（进程号，名字）；没有前台进程组或查不到名字时返回 `None`。
    pub fn foreground(&self) -> Option<(libc::pid_t, String)> {
        // SAFETY: tcgetpgrp 只查询，主端描述符有效。
        let pgid = unsafe { libc::tcgetpgrp(self.fd()) };
        if pgid <= 0 {
            return None;
        }
        process_name(pgid).map(|name| (pgid, name))
    }

    /// 返回这个终端的会话号：shell 是会话首进程，会话号就是它的进程号。
    pub fn session(&self) -> Pid {
        self.pid
    }

    /// 返回主端的描述符，交给 `poll` 等待读写。
    pub fn fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }
}

/// 挂断前台作业 `job`（进程组号）。
pub fn hang_up_job(job: Pid) {
    signal_group(job, libc::SIGHUP);
}

/// 结束前台作业 `job`（进程组号）；`session` 是它所在终端的会话号，不知道时为 `None`。
///
/// 挂断之后到期才调用：这个号码已经给了别的会话里的程序（[`taken_over`]）时不发信号；不知道
/// 会话号时无从分辨，仍然发送。
pub fn kill_job(job: Pid, session: Option<Pid>) {
    if session.is_none_or(|session| !taken_over(job, Some(session))) {
        signal_group(job, libc::SIGKILL);
    }
}

/// 进程组号 `pgid` 是否已经给了别的程序：现在有进程号是它的进程，而该进程不在会话 `session`
/// 里（`session` 为 `None` 表示原来的组长已经回收，有这个进程就是别的程序）。
///
/// 进程组里还有进程时它的号码不会再分配；组散了以后号码可以给新的进程，延迟发出的信号就会
/// 落到无关的程序上。查不到这个进程（组长已退出、组员还在）时按没有被占用处理。
fn taken_over(pgid: libc::pid_t, session: Option<libc::pid_t>) -> bool {
    // SAFETY: `getsid` 只查询；没有这个进程时返回 -1。
    let sid = unsafe { libc::getsid(pgid) };
    sid > 0 && session != Some(sid)
}

/// 返回前台作业 `job`（进程组号）中是否还有进程。
pub fn job_alive(job: Pid) -> bool {
    group_alive(job)
}

/// 给进程组 `pgid` 发信号 `signal`；组里没有进程时调用失败，不产生其他影响。
pub fn signal_group(pgid: libc::pid_t, signal: libc::c_int) {
    // SAFETY: kill 只发信号；`pgid` 为正，取负数表示整个进程组，不会变成发给所有进程的 -1。
    unsafe {
        libc::kill(-pgid, signal);
    }
}

/// 返回进程组 `pgid` 里是否还有进程（0 号信号只检查、不发送）。
pub fn group_alive(pgid: libc::pid_t) -> bool {
    // SAFETY: `pgid` 为正，取负后表示整个进程组；信号 0 只检查进程组是否存在，不发信号，也不
    // 访问内存。
    unsafe { libc::kill(-pgid, 0) == 0 }
}

/// 返回用户的登录 shell（用户数据库里的那一项）；查不到时返回 `None`。
pub fn user_shell(_env: &[(Vec<u8>, Vec<u8>)]) -> Option<Vec<u8>> {
    // SAFETY: getpwuid 返回指向静态区的指针，用之前检查空指针，内容立即复制出来；辅助进程是
    // 单线程的，不会有别的调用覆盖它。
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() || (*entry).pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr((*entry).pw_shell).to_bytes();
        (!shell.is_empty()).then(|| shell.to_vec())
    }
}

/// 返回进程的短名字（macOS 用 `proc_name`，其他平台读 `/proc/<pid>/comm`）。
#[cfg(target_os = "macos")]
fn process_name(pid: libc::pid_t) -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: 缓冲区来自本地数组，长度如实给出。
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

/// 返回进程的短名字（macOS 用 `proc_name`，其他平台读 `/proc/<pid>/comm`）。
#[cfg(not(target_os = "macos"))]
fn process_name(pid: libc::pid_t) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(name.trim_end().to_string())
}

/// 读满 `buf` 或读到结束，返回读到的字节数。
fn read_full(fd: RawFd, buf: &mut [u8]) -> usize {
    let mut done = 0;
    while done < buf.len() {
        // SAFETY: 缓冲区来自切片，长度如实给出。
        let n = unsafe { libc::read(fd, buf[done..].as_mut_ptr().cast(), buf.len() - done) };
        if n > 0 {
            done += n as usize;
        } else if n == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break;
        }
    }
    done
}

/// fork 出来的子进程：成为新会话的首进程，把从端设成控制终端和 0、1、2，再 exec 程序。
///
/// # Safety
///
/// 只能在 fork 之后的子进程里调用。调用者必须保证：`slave`、`error` 是有效且未关闭的描述符；
/// `cwd`、`program` 在 exec 之前一直有效；`argv`、`env` 非空且以 null 结尾。调用后不返回，
/// 子进程只能调用异步信号安全的函数，不能分配内存。
unsafe fn child(
    slave: RawFd,
    error: RawFd,
    cwd: &CString,
    program: &CString,
    argv: &[*const libc::c_char],
    env: &[*const libc::c_char],
) -> ! {
    // SAFETY: 都是异步信号安全的函数，参数见各自的说明。
    unsafe {
        // Rust 程序启动时把 SIGPIPE 设成了忽略，fork 会继承下去；shell 里的 `yes | head` 要靠
        // SIGPIPE 结束，所以把信号的处理方式和屏蔽字都恢复成默认。
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
        for signal in [
            libc::SIGPIPE,
            libc::SIGCHLD,
            libc::SIGINT,
            libc::SIGQUIT,
            libc::SIGTERM,
            libc::SIGHUP,
            libc::SIGTSTP,
            libc::SIGTTIN,
            libc::SIGTTOU,
        ] {
            libc::signal(signal, libc::SIG_DFL);
        }

        libc::setsid();
        libc::ioctl(slave, libc::TIOCSCTTY as _, 0);
        for fd in 0..3 {
            libc::dup2(slave, fd);
        }
        if slave > 2 {
            libc::close(slave);
        }
        // 目录不在了时留在辅助进程的目录里，shell 仍然可用。
        libc::chdir(cwd.as_ptr());
        libc::execve(program.as_ptr(), argv.as_ptr(), env.as_ptr());

        let errno = *errno_location();
        let bytes = errno.to_ne_bytes();
        libc::write(error, bytes.as_ptr().cast(), bytes.len());
        libc::_exit(127);
    }
}

/// 当前线程的 `errno` 地址。
///
/// # Safety
///
/// 返回的指针指向线程本地的 `errno`，只能在本线程读写，且只在下一次系统调用之前有效。
#[cfg(target_os = "macos")]
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: 返回当前线程的 errno 地址，不需要其他前提。
    unsafe { libc::__error() }
}

/// 当前线程的 `errno` 地址。
///
/// # Safety
///
/// 返回的指针指向线程本地的 `errno`，只能在本线程读写，且只在下一次系统调用之前有效。
#[cfg(not(target_os = "macos"))]
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: 返回当前线程的 errno 地址，不需要其他前提。
    unsafe { libc::__errno_location() }
}

/// 给从主程序继承来的描述符（0、1、2 以外）都加上 close-on-exec，返回加上的个数。
///
/// 主程序里系统库打开的描述符不一定带该标志，启动辅助进程时原样传过来；不加时会一路传进每个
/// 终端里的 shell 和它启动的程序。辅助进程自己打开的描述符都带着该标志。启动时调用一次，按
/// `/dev/fd`（Linux 上是 `/proc/self/fd`）列出实际打开的描述符，不按上限逐个试。
pub fn close_inherited_on_exec() -> usize {
    let dir = if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    // 先列完再设置：列目录用的描述符本身也在列表里，读完以后就关掉了，对它设置会失败，忽略。
    let fds: Vec<RawFd> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .filter(|&fd| fd > 2)
        .collect();
    fds.into_iter()
        .filter(|&fd| set_flags(fd, true, false).is_ok())
        .count()
}

/// SIGCHLD 写进去的管道的写端，信号处理函数里用。
static CHILD_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_child(_: libc::c_int) {
    let fd = CHILD_PIPE.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write 是异步信号安全的；管道是非阻塞的，满了就丢弃；读的一边只关心是否有数据。
        unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) };
    }
}

/// 返回子进程退出时变得可读的描述符（SIGCHLD 的自写管道），放进 `poll` 里等待，不用定时去查。
///
/// # Errors
///
/// 创建管道、设置标志或安装 SIGCHLD 处理方式失败时返回对应的 `std::io::Error`。
pub fn child_signals() -> io::Result<OwnedFd> {
    let mut pipe = [0; 2];
    // SAFETY: pipe 写入两个描述符到本地数组。
    check(unsafe { libc::pipe(pipe.as_mut_ptr()) })?;
    for fd in pipe {
        set_flags(fd, true, true)?;
    }
    CHILD_PIPE.store(pipe[1], Ordering::Relaxed);
    // SAFETY: 处理函数只调用 write；SA_RESTART 让被打断的系统调用自动重来。
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        // `SA_SIGINFO` 不设，内核按 `sa_handler` 解释 `sa_sigaction` 字段，处理函数的签名与它相符。
        action.sa_sigaction = on_child as *const () as usize;
        action.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;
        libc::sigemptyset(&mut action.sa_mask);
        check(libc::sigaction(
            libc::SIGCHLD,
            &action,
            std::ptr::null_mut(),
        ))?;
        // 写端一直开着（进程退出时由系统关闭）；读端交给调用方放进 `poll`。
        Ok(OwnedFd::from_raw_fd(pipe[0]))
    }
}

/// 把 SIGCHLD 管道里攒下的字节读空。
pub fn drain(fd: RawFd) {
    let mut buf = [0u8; 64];
    // SAFETY: 缓冲区来自本地数组，长度如实给出；描述符是非阻塞的。
    while unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
}

/// 等到这些描述符有事可做，最多等 `timeout` 毫秒（负数表示一直等待）。
///
/// 被信号打断时返回 0。
///
/// # Errors
///
/// `poll` 失败时返回对应的 `std::io::Error`。
pub fn poll(fds: &mut [libc::pollfd], timeout: i32) -> io::Result<usize> {
    // SAFETY: 数组来自切片，个数如实给出。
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
    if n >= 0 {
        return Ok(n as usize);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::Interrupted {
        Ok(0)
    } else {
        Err(error)
    }
}

/// 从 stdin 读一次（`poll` 报告可读之后才调用，不会阻塞）。
///
/// # Errors
///
/// 读取失败时返回对应的 `std::io::Error`；stdin 结束时返回 `Ok(0)`。
pub fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: 缓冲区来自切片，长度如实给出。
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
