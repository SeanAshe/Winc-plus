//! 单实例转交：新启动的进程把要打开的目录和文件交给已在运行的 winc，随即退出。
//!
//! 已在运行的 winc 持有 [`AppLock`]（每个用户只有一个），等待连接的位置由 `app_socket_path`
//! 或 `app_pipe_name` 决定，后者只在 Windows 上使用。Unix 套接字放在只有当前用户能创建的目录中，
//! 命名管道拒绝来自其他主机的连接。消息格式为一行版本号，第 2 版在其后另有一行激活令牌（见
//! [`LaunchTokens`]），之后每三个（路径、行、列）一组：路径、行号、列号各占一个以 NUL 结尾的
//! 字段。没有激活令牌时发送第 1 版消息。
//!
//! 持有 [`AppLock`] 的进程正在退出时回 `busy`，连接失败时新进程自己打开目标。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：本机的单实例通道和私有目录"
)]

use std::path::{Path, PathBuf};

use crate::cli::Target;

/// 消息的第一行：协议名和版本。第 1 版不带激活令牌。
const HELLO: &str = "winc-open 1\n";
/// 带激活令牌的消息（第 2 版）的第一行，下一行是令牌。
const HELLO_TOKEN: &str = "winc-open 2\n";

/// 新启动的 winc 交来的内容。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Launch {
    /// 要打开的目录和文件；为空时把当前窗口切到前台。
    pub targets: Vec<Target>,
    /// 新进程收到的激活令牌（见 [`LaunchTokens`]）；没有时为 `None`。
    pub token: Option<String>,
}

/// Wayland 合成器交给新进程的激活令牌所在的环境变量。
#[cfg(unix)]
const WAYLAND_TOKEN: &str = "XDG_ACTIVATION_TOKEN";
/// X11 启动通知的令牌所在的环境变量。
#[cfg(unix)]
const X11_TOKEN: &str = "DESKTOP_STARTUP_ID";

/// 启动本进程的程序给出的激活令牌，按窗口系统分开保存；不合法的令牌视为没有。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchTokens {
    /// `XDG_ACTIVATION_TOKEN` 中的令牌。
    wayland: Option<String>,
    /// `DESKTOP_STARTUP_ID` 中的令牌。
    x11: Option<String>,
}

impl LaunchTokens {
    /// 读出环境变量中的激活令牌，并从本进程的环境中移除，子进程不再继承已使用的令牌。
    ///
    /// 须在启动其他线程之前调用：移除环境变量时不能有其他线程读取环境。
    #[cfg(all(unix, not(target_os = "macos")))]
    #[allow(unsafe_code)] // `std::env::remove_var`
    pub fn take() -> LaunchTokens {
        let read = |name| std::env::var(name).ok().filter(|token| valid_token(token));
        let tokens = LaunchTokens {
            wayland: read(WAYLAND_TOKEN),
            x11: read(X11_TOKEN),
        };
        // SAFETY: 调用方保证此时只有本线程，没有线程同时读写环境。
        unsafe {
            std::env::remove_var(WAYLAND_TOKEN);
            std::env::remove_var(X11_TOKEN);
        }
        tokens
    }

    /// 本平台没有激活令牌，返回空的结果。
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    pub fn take() -> LaunchTokens {
        LaunchTokens::default()
    }

    /// 交给正在运行的 winc 的令牌，优先取 Wayland 的。
    pub fn forwarded(&self) -> Option<&str> {
        self.wayland.as_deref().or(self.x11.as_deref())
    }

    /// 本进程第一个窗口使用的令牌；`wayland` 表示窗口系统是否为 Wayland。
    pub fn for_window(&self, wayland: bool) -> Option<String> {
        if wayland { &self.wayland } else { &self.x11 }.clone()
    }

    /// 转到后台的窗口进程所需的环境变量，见 [`crate::shell_command::detach`]。
    #[cfg(unix)]
    pub fn env(&self) -> impl Iterator<Item = (&'static str, &str)> {
        let wayland = self.wayland.as_deref().map(|token| (WAYLAND_TOKEN, token));
        let x11 = self.x11.as_deref().map(|token| (X11_TOKEN, token));
        wayland.into_iter().chain(x11)
    }
}

/// 激活令牌能否写入消息：非空，且不含换行符和 NUL。
fn valid_token(token: &str) -> bool {
    !token.is_empty() && !token.contains(['\n', '\0'])
}

/// 返回整个应用的套接字路径（正在运行的 winc 监听的位置）；无法构造合法路径时返回 `None`。
#[cfg(unix)]
pub fn app_socket_path() -> Option<PathBuf> {
    private_file("app", "sock")
}

/// 返回当前用户私有目录中名为 `winc-{name}.{extension}` 的文件路径。
///
/// 套接字（`extension` 为 `sock`）的路径长度超出系统的限制时返回 `None`。
pub(crate) fn private_file(name: &str, extension: &str) -> Option<PathBuf> {
    // 限制取下限：macOS 的套接字路径最多 104 字节（含结尾的 NUL），Linux 为 108 字节，
    // 这里按 104 字节减去结尾 NUL 占用的 1 字节，路径本身不得超过 103 字节。
    const LIMIT: usize = 103;
    // macOS 的 `$TMPDIR` 每个用户各有一个，其他用户无法进入；Linux 使用 `$XDG_RUNTIME_DIR`。
    // 两者都可能被环境变量指到其他用户可写的目录，因此同样检查归属与权限，不合格时改用
    // 状态文件所在的目录。
    let runtime = if cfg!(target_os = "macos") {
        Some(std::env::temp_dir())
    } else {
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
    };
    let dir = runtime.filter(|dir| private_dir(dir)).or_else(|| {
        // 状态文件所在的目录在第一次启动时可能不存在，需要先以 0700 权限创建，否则无法
        // 获取锁文件，也无法创建套接字。
        let dir = winc_config::state_path()?.parent()?.to_path_buf();
        create_private_dir(&dir);
        Some(dir)
    })?;
    // 测试使用另一套名字，避免把目标交给本机正在运行的 winc 并使其切换到前台。
    let prefix = if cfg!(test) { "winc-test" } else { "winc" };
    let path = dir.join(format!("{prefix}-{name}.{extension}"));
    // 只有套接字的路径受这个限制；锁文件等其他文件不受限，Windows 上的数据目录常常更长。
    let fits = extension != "sock" || path.as_os_str().len() <= LIMIT;
    fits.then_some(path)
}

/// 是否只有当前用户能访问目录 `dir`：属主与主目录相同，且其他用户没有任何权限。
///
/// `$XDG_RUNTIME_DIR` 的规范要求权限为 0700。环境变量指向其他位置或目录权限被放宽时，
/// 该目录不被采用。
#[cfg(unix)]
fn private_dir(dir: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let (Ok(metadata), Some(home)) = (std::fs::metadata(dir), std::env::var_os("HOME")) else {
        return false;
    };
    let Ok(home) = std::fs::metadata(home) else {
        return false;
    };
    metadata.is_dir() && metadata.uid() == home.uid() && metadata.permissions().mode() & 0o077 == 0
}

/// 创建目录 `dir` 及其上级目录，新建目录的权限为 0700。
///
/// 创建失败时不报告错误，由后续使用该目录的操作失败。
fn create_private_dir(dir: &Path) {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    let _ = builder.create(dir);
}

/// 是否只有当前用户能访问目录 `dir`；不支持 Unix 权限的系统上只检查目录是否存在。
#[cfg(not(unix))]
fn private_dir(dir: &Path) -> bool {
    dir.is_dir()
}
/// 把目标 `targets` 和激活令牌 `token` 编码成消息字节。
///
/// 有合法的令牌时以 [`HELLO_TOKEN`] 和令牌一行开头，否则以 [`HELLO`] 开头；之后每项依次为
/// 路径、行号、列号三个以 NUL 结尾的字段，行号与列号从 0 起，没有指定时字段为空。
fn encode(targets: &[Target], token: Option<&str>) -> Vec<u8> {
    let mut out = match token.filter(|token| valid_token(token)) {
        Some(token) => format!("{HELLO_TOKEN}{token}\n").into_bytes(),
        None => HELLO.as_bytes().to_vec(),
    };
    for target in targets {
        out.extend_from_slice(target.path.as_os_str().as_encoded_bytes());
        out.push(0);
        for n in [target.line, target.col] {
            if let Some(n) = n {
                out.extend_from_slice(n.to_string().as_bytes());
            }
            out.push(0);
        }
    }
    out
}

/// 解码 [`encode`] 的结果（第 1 版或第 2 版消息）；格式不对时返回 `None`。
fn decode(bytes: &[u8]) -> Option<Launch> {
    let (token, body) = match bytes.strip_prefix(HELLO_TOKEN.as_bytes()) {
        Some(rest) => {
            let end = rest.iter().position(|&b| b == b'\n')?;
            let token = std::str::from_utf8(&rest[..end]).ok()?;
            (Some(token.to_owned()), &rest[end + 1..])
        }
        None => (None, bytes.strip_prefix(HELLO.as_bytes())?),
    };
    let token = token.filter(|token| valid_token(token));
    Some(Launch {
        targets: decode_targets(body)?,
        token,
    })
}

/// 解码消息头之后的目标；格式不对时返回 `None`。
fn decode_targets(body: &[u8]) -> Option<Vec<Target>> {
    if body.is_empty() {
        return Some(Vec::new());
    }
    let body = body.strip_suffix(&[0])?;
    let fields: Vec<&[u8]> = body.split(|&b| b == 0).collect();
    if !fields.len().is_multiple_of(3) {
        return None;
    }
    let number = |field: &[u8]| -> Option<Option<usize>> {
        if field.is_empty() {
            return Some(None);
        }
        std::str::from_utf8(field).ok()?.parse().ok().map(Some)
    };
    fields
        .chunks(3)
        .map(|item| {
            Some(Target {
                path: path_from_bytes(item[0])?,
                line: number(item[1])?,
                col: number(item[2])?,
            })
        })
        .collect()
}

/// 经 `send` 把目标 `targets` 和激活令牌 `token` 交出，返回对方是否已收下。
///
/// `send` 发出一条消息并返回对方答复的一行，对方未答复即断开时为空字符串。带令牌的消息未得到
/// 答复时，对方是不支持第 2 版的较早版本，改发第 1 版消息；答复 `busy` 时不重发。
fn deliver(
    targets: &[Target],
    token: Option<&str>,
    send: impl Fn(&[u8]) -> std::io::Result<String>,
) -> bool {
    let reply = |token| send(&encode(targets, token));
    match reply(token) {
        Ok(line) if line.is_empty() && token.is_some_and(valid_token) => {
            reply(None).is_ok_and(|line| line == "ok\n")
        }
        Ok(line) => line == "ok\n",
        Err(_) => false,
    }
}

/// 把 [`encode`] 写出的路径字节还原成路径。
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

/// 把 [`encode`] 写出的路径字节还原成路径；不是 UTF-8 时返回 `None`。
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(unix)]
mod imp {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Sender, channel};
    use std::time::Duration;

    use super::{Launch, app_socket_path, decode, deliver};
    use crate::cli::Target;

    /// 收到交来的目标之后等待界面线程答复的时长（毫秒）。
    ///
    /// 取值由界面线程的正常处理耗时决定，取自调参：界面线程阻塞（例如系统菜单处于打开状态）
    /// 超过该时长仍未答复时回 `busy`，由新进程自己打开目标。
    const ANSWER_TIMEOUT: Duration = Duration::from_millis(1500);
    /// 新进程等待对方回复的时长（秒）。
    ///
    /// 取值由 [`ANSWER_TIMEOUT`] 决定：比对方等待界面线程答复的时长更长，保证对方先给出答复。
    const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
    /// 单条消息可读取的最大字节数（字节），超过时按格式错误处理。
    ///
    /// 取值由系统对命令行参数总长的限制决定：macOS 为 1 MiB，Linux 默认为 2 MiB；参数中的相对
    /// 路径改写为绝对路径后长度会增加，因此留出余量。
    pub(super) const MESSAGE_LIMIT: u64 = 16 << 20;
    /// 把 `targets` 和激活令牌 `token` 交给正在运行的 winc，返回对方是否已收下。
    ///
    /// 目标投递到 [`app_socket_path`] 对应的套接字，由对方决定切换到已打开该目录的窗口还是
    /// 新建窗口；`targets` 为空时对方把当前窗口切换到前台。
    pub fn forward_to_app(targets: &[Target], token: Option<&str>) -> bool {
        app_socket_path().is_some_and(|path| forward_to(&path, targets, token))
    }

    /// 把 `targets` 和激活令牌 `token` 交给 `path` 上的 winc，返回对方是否已收下。
    ///
    /// 连接失败、写入失败或对方没有回复 `ok` 时按未收下处理。
    pub(super) fn forward_to(path: &Path, targets: &[Target], token: Option<&str>) -> bool {
        deliver(targets, token, |message| {
            let mut stream = UnixStream::connect(path)?;
            stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
            stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
            stream.write_all(message)?;
            stream.shutdown(std::net::Shutdown::Write)?;
            let mut reply = String::new();
            BufReader::new(stream).read_line(&mut reply)?;
            Ok(reply)
        })
    }

    /// 在整个应用的套接字上等待其他 winc 交来的目标，丢弃时停止监听并删除套接字文件。
    pub struct Listener {
        path: PathBuf,
        stop: Arc<AtomicBool>,
    }

    impl Listener {
        /// 开始在整个应用的套接字上等待其他 winc 交来的目标，无法创建套接字时返回 `None`。
        ///
        /// 每收到一次，就在后台线程中以交来的内容和答复通道调用 `open`：界面线程收下时向通道
        /// 发送 `true`，发送成功才打开目标，发送失败说明新进程已不再等待并自行打开；不收时发送
        /// `false` 或关闭通道。
        ///
        /// 调用方必须已持有 [`AppLock`](super::AppLock)：此时同名的套接字文件只可能由异常退出的
        /// 进程留下，先移除再创建。无法创建时返回 `None`，其他 winc 仍然自行打开目标。
        pub fn bind_app(open: impl Fn(Launch, Sender<bool>) + Send + 'static) -> Option<Listener> {
            Self::bind_at(app_socket_path()?, open)
        }

        /// 在套接字 `path` 上等待交给整个应用的目标，`open` 的约定同 [`Listener::bind_app`]。
        ///
        /// 无法创建套接字时返回 `None`。
        pub(super) fn bind_at(
            path: PathBuf,
            open: impl Fn(Launch, Sender<bool>) + Send + 'static,
        ) -> Option<Listener> {
            let _ = std::fs::remove_file(&path);
            let listener = UnixListener::bind(&path).ok()?;
            // 权限设为 0600，只允许当前用户连接。
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            std::thread::Builder::new()
                .name("winc-instance".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        let Ok(mut stream) = stream else {
                            continue;
                        };
                        let _ = stream.set_read_timeout(Some(REPLY_TIMEOUT));
                        let mut bytes = Vec::new();
                        let read = (&mut stream)
                            .take(MESSAGE_LIMIT + 1)
                            .read_to_end(&mut bytes);
                        if read.is_err() || bytes.len() as u64 > MESSAGE_LIMIT {
                            continue;
                        }
                        // 内容不是 winc 发出的或格式不正确时不答复。
                        let Some(launch) = decode(&bytes) else {
                            continue;
                        };
                        let (answer, answered) = channel();
                        open(launch, answer);
                        let reply: &[u8] = match answered.recv_timeout(ANSWER_TIMEOUT) {
                            Ok(true) => b"ok\n",
                            _ => b"busy\n",
                        };
                        let _ = stream.write_all(reply);
                    }
                })
                .ok()?;
            Some(Listener { path, stop })
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // 监听线程阻塞在 `accept` 上，需要先连接一次使其返回，线程读到停止标志后退出，
            // 之后才能移除套接字文件；先移除文件会使该连接失败，监听线程无法退出。
            self.stop.store(true, Ordering::Relaxed);
            let _ = UnixStream::connect(&self.path);
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// 返回整个应用的命名管道名，格式为 `\\.\pipe\winc-<用户名>-app`。
///
/// 便携版的管道名中另有便携数据目录的散列（见 [`path_tag`]）：便携版与安装版、不同位置的便携版
/// 各自是独立的应用，不互相转交目标。
#[cfg(windows)]
pub fn app_pipe_name() -> PathBuf {
    let user = std::env::var("USERNAME").unwrap_or_default();
    // 测试使用另一套名字，避免把目标交给本机正在运行的 winc 并使其切换到前台。
    let prefix = if cfg!(test) { "winc-test" } else { "winc" };
    let tag = (winc_config::files::portable_dir())
        .map(|dir| format!("-{:016x}", path_tag(&dir)))
        .unwrap_or_default();
    PathBuf::from(format!(r"\\.\pipe\{prefix}-{user}{tag}-app"))
}

/// 返回目录 `dir` 的散列（64 位 FNV-1a），不区分大小写：Windows 上路径不区分大小写。
///
/// 用于命名管道名；同一目录在各次运行中得到相同的值。
#[cfg(any(windows, test))]
fn path_tag(dir: &Path) -> u64 {
    let text = dir.to_string_lossy().to_lowercase();
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(windows)]
#[allow(unsafe_code)] // 建命名管道、等连接（`CreateNamedPipeW`、`ConnectNamedPipe`）
mod imp {
    use std::fs::File;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::IO::CancelIoEx;
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId,
        GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::{Launch, app_pipe_name, decode, deliver};
    use crate::cli::Target;

    /// 收到交来的目标之后等待界面线程答复的时长（毫秒）。
    ///
    /// 取值由界面线程的正常处理耗时决定，取自调参：界面线程阻塞超过该时长仍未答复时回
    /// `busy`，由新进程自己打开目标。
    const ANSWER_TIMEOUT: Duration = Duration::from_millis(1500);
    /// 管道上一次读写（发出目标并等回复、读入一条消息）最长的时间（秒）。
    ///
    /// 取值由 [`ANSWER_TIMEOUT`] 决定：比对方等待界面线程答复的时长更长，保证对方先给出答复。
    const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
    /// 读写超时之后再次取消的间隔（毫秒）：取消只对正在进行的那一次读写有效。
    const CANCEL_AGAIN: Duration = Duration::from_millis(50);
    /// 单条消息可读取的最大字节数（字节），超过时按格式错误处理。
    ///
    /// 取值由系统对命令行参数总长的限制决定：macOS 为 1 MiB，Linux 默认为 2 MiB；参数中的
    /// 相对路径改写为绝对路径后长度会增加，因此留出余量。
    pub(super) const MESSAGE_LIMIT: u64 = 16 << 20;

    /// 执行 `io`（在 `pipe` 上的读写），超过 [`REPLY_TIMEOUT`] 仍未完成时取消其中正在进行的
    /// 读写，使它以错误返回。
    ///
    /// 管道的读写没有超时：对方连上后什么也不发，或迟迟不回复时，等待的一方会一直停住。
    fn within<T>(pipe: &File, io: impl FnOnce() -> T) -> T {
        // 句柄以整数交给看守线程：它在 `pipe` 被丢弃之前结束（下面等它）。
        let handle = pipe.as_raw_handle() as usize;
        let (done, finished) = channel::<()>();
        let watchdog = std::thread::spawn(move || {
            let mut limit = REPLY_TIMEOUT;
            while finished.recv_timeout(limit) == Err(RecvTimeoutError::Timeout) {
                // SAFETY: `io` 尚未结束（通道还开着），句柄仍然有效；没有正在进行的读写时
                // 调用失败，没有其他效果。
                unsafe { CancelIoEx(handle as HANDLE, std::ptr::null()) };
                limit = CANCEL_AGAIN;
            }
        });
        let result = io();
        drop(done);
        let _ = watchdog.join();
        result
    }

    /// 返回进程 `process` 的用户（令牌中的 `TOKEN_USER`，其中的指针指向缓冲区自身）。
    fn user_of(process: HANDLE) -> Option<Vec<u64>> {
        let mut token = std::ptr::null_mut();
        // SAFETY: `process` 是调用方给出的有效进程句柄，`token` 是可写的栈变量。
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        // `TOKEN_USER` 加上最长的 SID（68 字节）不超过 128 字节；用 `u64` 保证对齐。
        let mut buffer = vec![0u64; 16];
        let (mut needed, size) = (0, (buffer.len() * 8) as u32);
        // SAFETY: `token` 是刚打开的令牌句柄；缓冲区可写、长度如实给出；用完后关闭句柄。
        let ok = unsafe {
            let ok = GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut needed,
            );
            CloseHandle(token);
            ok
        };
        (ok != 0).then_some(buffer)
    }

    /// 进程 `pid` 是否与本进程属于同一个用户；查不到时按不是处理。
    ///
    /// 管道的名字全机可见：别的用户可以抢先建一个同名的管道等着收目标，也可以连上来。
    fn same_user(pid: u32) -> bool {
        // SAFETY: 只请求查询权限；失败时返回空句柄。
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return false;
        }
        let theirs = user_of(process);
        // SAFETY: `process` 是上面打开的句柄，只关闭一次。
        unsafe { CloseHandle(process) };
        // SAFETY: `GetCurrentProcess` 返回的伪句柄总是有效，不需要关闭。
        let ours = user_of(unsafe { GetCurrentProcess() });
        let (Some(theirs), Some(ours)) = (theirs, ours) else {
            return false;
        };
        let sid = |user: &[u64]| {
            // SAFETY: 缓冲区由 `GetTokenInformation` 填好，开头是一个 `TOKEN_USER`，对齐足够。
            unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        };
        // SAFETY: 两个 SID 都指向各自仍然存活的缓冲区，由系统填写，格式有效。
        unsafe { EqualSid(sid(&theirs), sid(&ours)) != 0 }
    }

    /// 管道 `pipe` 另一端的进程编号：`server` 为真时取服务的一方，否则取连上来的一方。
    fn peer(pipe: &File, server: bool) -> Option<u32> {
        let (mut pid, handle) = (0, pipe.as_raw_handle() as HANDLE);
        // SAFETY: 句柄来自仍然存活的 `pipe`，`pid` 是可写的栈变量；调用失败时返回 0。
        let ok = unsafe {
            match server {
                true => GetNamedPipeServerProcessId(handle, &mut pid),
                false => GetNamedPipeClientProcessId(handle, &mut pid),
            }
        };
        (ok != 0).then_some(pid)
    }

    /// 把 `targets` 和激活令牌 `token` 交给正在运行的 winc，返回对方是否已收下。
    ///
    /// 目标投递到 [`app_pipe_name`] 对应的命名管道。
    pub fn forward_to_app(targets: &[Target], token: Option<&str>) -> bool {
        forward_to(&app_pipe_name(), targets, token)
    }

    /// 把 `targets` 和激活令牌 `token` 交给 `pipe` 上的 winc，返回对方是否已收下。
    ///
    /// 连接失败、对方不是本用户的进程、写入失败、超过 [`REPLY_TIMEOUT`] 或对方没有回复 `ok`
    /// 时按未收下处理。
    pub(super) fn forward_to(pipe: &Path, targets: &[Target], token: Option<&str>) -> bool {
        deliver(targets, token, |message| {
            let stream = File::options().read(true).write(true).open(pipe)?;
            // 目标和激活令牌只交给本用户的进程；系统只允许拥有前台权限的进程切换前台窗口，
            // 由用户启动的新进程有，正在运行的没有：转让给它，否则它的窗口留在后台。
            let server = peer(&stream, true).filter(|pid| same_user(*pid));
            let Some(server) = server else {
                return Err(std::io::ErrorKind::PermissionDenied.into());
            };
            crate::windows::allow_foreground(server);
            within(&stream, || {
                (&stream).write_all(&(message.len() as u64).to_le_bytes())?;
                (&stream).write_all(message)?;
                let mut reply = String::new();
                BufReader::new(&stream).read_line(&mut reply)?;
                Ok(reply)
            })
        })
    }

    /// 创建一个等待连接的管道实例；`first` 为真时要求它是该名字的第一个实例。
    ///
    /// 其他进程先创建了同名的管道时返回 `None`，目标不会被交给该管道。
    pub(crate) fn create(name: &[u16], first: bool) -> Option<File> {
        let flags = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: `name` 是以 NUL 结尾的 UTF-16 字符串；安全属性指针为空，使用默认安全描述符，
        // 只有当前用户和管理员能创建同名的管道实例。
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                flags,
                mode,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        // SAFETY: `handle` 是 `CreateNamedPipeW` 刚返回的有效句柄，所有权交给 `File`，
        // 在 `File` 被丢弃时关闭。
        Some(unsafe { File::from_raw_handle(handle) })
    }

    /// 是否已有客户端连上管道 `pipe`；调用之前已连上的也算已连接。
    pub(crate) fn connect(pipe: &File) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: 句柄有效且归 `pipe` 所有；不使用重叠 I/O，调用阻塞到有客户端连上为止。
        let ok = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), std::ptr::null_mut()) };
        // SAFETY: 紧接上一个调用读取其错误码，线程错误码在该调用返回后立即可读。
        ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED
    }

    /// 在整个应用的命名管道上等待其他 winc 交来的目标，丢弃时停止监听。
    pub struct Listener {
        pipe: PathBuf,
        stop: Arc<AtomicBool>,
    }

    impl Listener {
        /// 开始在整个应用的命名管道上等待其他 winc 交来的目标，无法创建管道时返回 `None`。
        ///
        /// 每收到一次，就在后台线程中以交来的内容和答复通道调用 `open`：界面线程收下时向通道
        /// 发送 `true`，不收时发送 `false` 或直接丢弃通道。无法创建管道时返回 `None`，其他 winc
        /// 仍然自行打开目标。
        pub fn bind_app(open: impl Fn(Launch, Sender<bool>) + Send + 'static) -> Option<Listener> {
            Self::bind_at(app_pipe_name(), open)
        }

        /// 在命名管道 `pipe` 上等待交给整个应用的目标，`open` 的约定同 [`Listener::bind_app`]。
        ///
        /// 无法创建管道时返回 `None`。
        pub(super) fn bind_at(
            pipe: PathBuf,
            open: impl Fn(Launch, Sender<bool>) + Send + 'static,
        ) -> Option<Listener> {
            let name: Vec<u16> = pipe.as_os_str().encode_wide().chain([0]).collect();
            let mut next = create(&name, true)?;
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            std::thread::Builder::new()
                .name("winc-instance".into())
                .spawn(move || {
                    loop {
                        let connected = connect(&next);
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        // 先创建下一个实例再处理当前连接：处理期间到达的连接也有实例可连。
                        let Some(waiting) = create(&name, false) else {
                            break;
                        };
                        let stream = std::mem::replace(&mut next, waiting);
                        if !connected {
                            continue;
                        }
                        // 只收本用户的进程交来的；对方连上后迟迟不发时放弃这一个，不耽误后面的。
                        if !peer(&stream, false).is_some_and(same_user) {
                            continue;
                        }
                        let Some(launch) = within(&stream, || read_message(&mut &stream)) else {
                            continue;
                        };
                        let (answer, answered) = channel();
                        open(launch, answer);
                        let reply: &[u8] = match answered.recv_timeout(ANSWER_TIMEOUT) {
                            Ok(true) => b"ok\n",
                            _ => b"busy\n",
                        };
                        within(&stream, || {
                            let _ = (&stream).write_all(reply);
                            let _ = (&stream).flush();
                        });
                    }
                })
                .ok()?;
            Some(Listener { pipe, stop })
        }
    }

    /// 读取一条带长度前缀的消息并解码，读取失败或格式错误时返回 `None`。
    ///
    /// 长度超过 [`MESSAGE_LIMIT`] 或数据不足时同样返回 `None`。
    pub(super) fn read_message(stream: &mut impl Read) -> Option<Launch> {
        let mut length = [0; 8];
        stream.read_exact(&mut length).ok()?;
        let length = u64::from_le_bytes(length);
        if length > MESSAGE_LIMIT {
            return None;
        }
        let mut bytes = vec![0; length as usize];
        stream.read_exact(&mut bytes).ok()?;
        decode(&bytes)
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // 监听线程阻塞在 `ConnectNamedPipe` 上，需要先连接一次使其返回，线程读到停止
            // 标志后退出。
            self.stop.store(true, Ordering::Relaxed);
            let _ = File::options().read(true).write(true).open(&self.pipe);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::Launch;
    use crate::cli::Target;

    /// 把 `targets` 和激活令牌交给正在运行的 winc，返回是否已收下。
    ///
    /// 不支持的平台上始终返回 `false`，由新进程自行打开目标。
    pub fn forward_to_app(_targets: &[Target], _token: Option<&str>) -> bool {
        false
    }

    /// 不支持的平台上的空实现，不会等待目标。
    pub struct Listener;

    impl Listener {
        /// 开始等待目标，不支持的平台上始终返回 `None`。
        pub fn bind_app(
            _open: impl Fn(Launch, std::sync::mpsc::Sender<bool>) + Send + 'static,
        ) -> Option<Listener> {
            None
        }
    }
}

pub use imp::{Listener, forward_to_app};
#[cfg(windows)]
pub(crate) use imp::{connect as connect_pipe, create as create_pipe};

/// 「整个应用」这一角色的持有凭据。
///
/// 每个用户同时只有一个进程能获取（文件锁，进程退出时由系统释放）；持有该凭据的进程在本地
/// 套接字（Windows 上为命名管道）上等待，其他 winc 把要打开的目录和文件交给该进程。
/// 两个 winc 几乎同时启动时，未获取到凭据的进程仍然自行打开窗口。
pub struct AppLock {
    _file: std::fs::File,
}

impl AppLock {
    /// 获取锁，失败时返回 `None`。
    ///
    /// 锁已被另一个 winc 持有时，或无法创建锁文件时返回 `None`。
    pub fn acquire() -> Option<AppLock> {
        let path = private_file("app", "lock")?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()?;
        file.try_lock().ok()?;
        Some(AppLock { _file: file })
    }
}

/// 返回 `path` 的绝对路径，写法见 [`winc_editor::given_path`]：经过符号链接时保留链接。
///
/// 交给另一个进程前需要先解析相对路径。相对路径相对于 shell 的当前目录（见 `shell_cwd`），
/// 即用户在 shell 中看到的路径；对方判断是否为同一个目录时按真实路径比较。
pub fn absolute(path: &Path) -> PathBuf {
    if winc_backend::is_remote(path) || path.is_absolute() {
        return winc_editor::given_path(path);
    }
    winc_editor::given_path(&shell_cwd().join(path))
}

/// 返回 shell 的当前目录，见 [`pick_cwd`]。
fn shell_cwd() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    pick_cwd(std::env::var_os("PWD").map(PathBuf::from), cwd)
}

/// 在 shell 记下的当前目录 `pwd`（环境变量 `PWD`）和进程的当前目录 `cwd`（真实路径）中选一个：
/// `pwd` 是绝对路径且与 `cwd` 是同一个目录时用它，保留 shell 进入时经过的符号链接。
fn pick_cwd(pwd: Option<PathBuf>, cwd: PathBuf) -> PathBuf {
    let pwd = pwd.filter(|pwd| pwd.is_absolute() && winc_editor::same_folder(pwd, &cwd));
    pwd.unwrap_or(cwd)
}

#[cfg(test)]
mod tests;
