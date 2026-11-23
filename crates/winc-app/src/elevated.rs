//! 以管理员身份保存：保存被系统拒绝时，经系统的授权对话框以管理员身份运行 winc 自身，只把一个
//! 临时文件的内容写进目标文件，写完即退出。
//!
//! 写入进程（[`helper`]）只接受 [`ARG`]、内容文件、目标文件和目标的身份（Unix），不执行其他
//! 程序。目标文件原地覆盖，不新建再改名：系统文件（如 hosts）的属主、权限和硬链接保持不变。
//! 授权每次都由用户确认，不常驻、不保留权限。Unix 上只写未提权的进程看到的那个文件，见
//! [`Identity`]。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：写入的是本机的文件，远端和 WSL 中的文件不以管理员身份保存"
)]

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use winc_i18n::t;

/// 写入进程的第一个参数。
const ARG: &str = "--elevated-write";

/// 作为以管理员身份启动的写入进程运行时（参数为 `ARG <内容文件> <目标文件>`，Unix 上再加
/// [`Identity`]），写入并返回退出码：0 为写好，1 为写入失败（原因写到标准错误），2 为参数不对。
/// 不是写入进程时返回 `None`。
pub fn helper() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != ARG {
        return None;
    }
    let (Some(copy), Some(target)) = (args.next(), args.next()) else {
        return Some(2);
    };
    #[cfg(unix)]
    let Some(identity) = args.next().as_deref().and_then(Identity::parse) else {
        return Some(2);
    };
    if args.next().is_some() {
        return Some(2);
    }
    #[cfg(unix)]
    let written = write_in_place(Path::new(&copy), Path::new(&target), identity);
    #[cfg(not(unix))]
    let written = write_in_place(Path::new(&copy), Path::new(&target));
    Some(match written {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    })
}

/// 未提权的进程看到的目标的身份：设备号和 inode 号。
///
/// 写入进程打开目标后与之核对，不相符时不写：路径在两次解析之间可以被换成别的文件。
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Identity {
    /// 已有的普通文件。
    File(u64, u64),
    /// 目标尚不存在：它所在的目录，文件在其中新建。
    Dir(u64, u64),
}

#[cfg(unix)]
impl Identity {
    /// 返回 `target` 现在的身份；`target` 不存在时是它所在目录的身份。
    ///
    /// # Errors
    ///
    /// `target` 是符号链接、设备等不是普通文件的对象，或读不到信息时返回错误。
    fn of(target: &Path) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        match std::fs::symlink_metadata(target) {
            Ok(file) if file.is_file() => Ok(Self::File(file.dev(), file.ino())),
            Ok(_) => Err(io::Error::from(io::ErrorKind::InvalidInput)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let dir = std::fs::metadata(target.parent().unwrap_or(Path::new("/")))?;
                Ok(Self::Dir(dir.dev(), dir.ino()))
            }
            Err(e) => Err(e),
        }
    }

    /// 写成命令行参数：`file:<设备号>:<inode 号>` 或 `dir:…`。
    fn argument(self) -> OsString {
        let (kind, dev, ino) = match self {
            Self::File(dev, ino) => ("file", dev, ino),
            Self::Dir(dev, ino) => ("dir", dev, ino),
        };
        format!("{kind}:{dev}:{ino}").into()
    }

    /// 读回 [`Identity::argument`] 写出的参数；格式不对时返回 `None`。
    fn parse(argument: &std::ffi::OsStr) -> Option<Self> {
        let mut parts = argument.to_str()?.split(':');
        let (kind, dev, ino) = (parts.next()?, parts.next()?, parts.next()?);
        let (dev, ino) = (dev.parse().ok()?, ino.parse().ok()?);
        match (kind, parts.next()) {
            ("file", None) => Some(Self::File(dev, ino)),
            ("dir", None) => Some(Self::Dir(dev, ino)),
            _ => None,
        }
    }

    /// 是否打开的文件或目录 `opened` 就是这个身份。
    fn matches(self, opened: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        let same = |dev, ino| opened.dev() == dev && opened.ino() == ino;
        match self {
            Self::File(dev, ino) => opened.is_file() && same(dev, ino),
            Self::Dir(dev, ino) => opened.is_dir() && same(dev, ino),
        }
    }
}

/// 把 `copy` 的内容原地写进身份为 `identity` 的 `target`，写完刷到磁盘；写到一半失败时写回
/// 原来的内容。
///
/// 目标不跟随符号链接地打开，打开后核对身份，之后的读写都经这个已打开的文件，不再按路径访问。
///
/// # Errors
///
/// 读不到内容文件、目标已不是 `identity` 所指的文件或目录、写入失败时返回错误。
#[cfg(unix)]
fn write_in_place(copy: &Path, target: &Path, identity: Identity) -> io::Result<()> {
    use std::io::{Read, Seek};
    use std::os::unix::fs::OpenOptionsExt;

    let content = std::fs::read(copy)?;
    let changed = || io::Error::new(io::ErrorKind::InvalidInput, "the file was replaced");
    let mut file = match identity {
        Identity::File(..) => {
            // 不阻塞地打开：目标被换成命名管道时不等待另一端。
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(target)?;
            if !identity.matches(&file.metadata()?) {
                return Err(changed());
            }
            file
        }
        Identity::Dir(..) => {
            let parent = target.parent().unwrap_or(Path::new("/"));
            let dir = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(parent)?;
            let name = target.file_name().ok_or_else(changed)?;
            if !identity.matches(&dir.metadata()?) {
                return Err(changed());
            }
            create_in(&dir, name)?
        }
    };
    let mut original = Vec::new();
    file.read_to_end(&mut original)?;
    let mut rewrite = |bytes: &[u8]| {
        file.rewind()?;
        file.set_len(0)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    rewrite(&content).inspect_err(|_| {
        let _ = rewrite(&original);
    })
}

/// 在已打开的目录 `dir` 中新建文件 `name` 并以读写打开；已有同名的项（包括符号链接）时失败。
#[cfg(unix)]
// 本函数只有对 `libc::openat` 的调用和接收其返回的描述符需要 `unsafe`，因此只在此处放开。
#[allow(unsafe_code)]
fn create_in(dir: &std::fs::File, name: &std::ffi::OsStr) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let name = std::ffi::CString::new(name.as_bytes())?;
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `dir` 在调用期间保持打开，`name` 是以 NUL 结尾、调用期间有效的字符串；带
    // `O_CREAT` 时第四个参数是权限。失败时返回 -1，不产生描述符。
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o644 as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` 是 `openat` 刚返回的有效描述符，没有其他持有者，所有权交给 `File`。
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// 把 `copy` 的内容原地写进 `target`（不存在时新建），写完刷到磁盘；写到一半失败时写回原来的
/// 内容。
#[cfg(not(unix))]
fn write_in_place(copy: &Path, target: &Path) -> io::Result<()> {
    let content = std::fs::read(copy)?;
    let original = std::fs::read(target).ok();
    let rewrite = |bytes: &[u8]| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(target)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    rewrite(&content).inspect_err(|_| {
        if let Some(original) = &original {
            let _ = rewrite(original);
        }
    })
}

/// 保存 `path` 失败、原因为 `error` 时能否以管理员身份重试：是本机的文件，且是系统拒绝了写入。
///
/// 文档只读等 winc 自己的拒绝不算；远端主机上和 WSL 发行版中的文件不经本机的管理员权限。
pub fn can_retry(path: &Path, error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        && error.raw_os_error().is_some()
        && !winc_backend::is_remote(path)
        && winc_editor::wsl::locate(path).is_none()
}

/// 新建一个只有当前用户能访问的临时目录，用来放交给写入进程的内容。
pub fn private_dir() -> io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("winc-admin-save-{}-{n}", std::process::id()));
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700).create(&dir)?;
    // Windows 上的临时目录本来只有当前用户能访问。
    #[cfg(not(unix))]
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

/// 授权对话框所属的窗口（Windows 的窗口句柄），在界面线程上取得，交给后台线程。
#[derive(Clone, Copy)]
pub struct Owner(#[cfg_attr(not(windows), allow(dead_code))] isize);

impl Owner {
    /// 返回窗口 `window` 作为授权对话框的所属窗口；取不到时对话框不属于任何窗口。
    pub fn of(window: Option<&winit::window::Window>) -> Self {
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle = window.and_then(|w| w.window_handle().ok());
            if let Some(RawWindowHandle::Win32(handle)) = handle.map(|h| h.as_raw()) {
                return Self(handle.hwnd.get());
            }
        }
        let _ = window;
        Self(0)
    }
}

/// 经系统的授权对话框以管理员身份把 `copy` 的内容写进 `target`，阻塞到写完或用户取消；在后台
/// 线程上调用。对话框属于窗口 `owner`。
///
/// # Errors
///
/// 用户取消时返回 `Interrupted`；拿不到授权或写入失败时返回说明原因的错误。
pub fn write(copy: &Path, target: &Path, owner: Owner) -> io::Result<()> {
    #[cfg(unix)]
    let identity = Identity::of(target)?;
    // 测试中没有授权对话框：直接写入，验证之后的流程。
    if cfg!(test) {
        #[cfg(unix)]
        return write_in_place(copy, target, identity);
        #[cfg(not(unix))]
        return write_in_place(copy, target);
    }
    let exe = helper_program()?;
    #[cfg(unix)]
    let args = [ARG.into(), copy.into(), target.into(), identity.argument()];
    #[cfg(not(unix))]
    let args = [OsString::from(ARG), copy.into(), target.into()];
    // 授权对话框写明完整的路径：同名的文件可以在别的目录里。
    let name = target.to_string_lossy();
    match run_as_admin(&exe, &args, owner, &name) {
        Ok(()) => Ok(()),
        Err(Refusal::Cancelled) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            t!("workspace-save-admin-cancelled"),
        )),
        Err(Refusal::Failed(reason)) if reason.is_empty() => {
            Err(io::Error::other(t!("workspace-save-admin-failed")))
        }
        Err(Refusal::Failed(reason)) => Err(io::Error::other(reason)),
    }
}

/// 写入进程的程序：winc 自身。
///
/// 以 AppImage 运行时是 AppImage 文件：程序本身在只有当前用户能访问的挂载目录里，管理员身份
/// 的进程打不开它。
fn helper_program() -> io::Result<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(appimage) = crate::shell_command::current_appimage() {
        return Ok(appimage);
    }
    std::env::current_exe()
}

/// 没有以管理员身份写成的原因。
enum Refusal {
    /// 用户在授权对话框中取消。
    Cancelled,
    /// 拿不到授权，或写入进程报告失败；说明为空时用通用的说法。
    Failed(String),
}

/// 以管理员身份运行 `exe` 并等它结束（UAC 对话框）。
#[cfg(windows)]
#[allow(unsafe_code)] // ShellExecuteExW、WaitForSingleObject
fn run_as_admin(exe: &Path, args: &[OsString], owner: Owner, _: &str) -> Result<(), Refusal> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let wide = |text: &std::ffi::OsStr| text.encode_wide().chain([0]).collect::<Vec<u16>>();
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let mut parameters = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            parameters.push(u16::from(b' '));
        }
        quote_argument(arg, &mut parameters);
    }
    parameters.push(0);

    // SAFETY: `SHELLEXECUTEINFOW` 只含整数和指针字段，全零是有效值；其中的字符串指针指向本函数中
    // 以 NUL 结尾的缓冲区，在调用期间有效。
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI;
    // 对话框属于 winc 的窗口，出现在它前面，不只在任务栏上闪烁。
    info.hwnd = owner.0 as _;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;
    // SAFETY: `info` 已按上面的说明填写，`cbSize` 与结构体大小一致。
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: 没有参数，读取本线程上一次调用的错误码。
        return Err(match unsafe { GetLastError() } {
            ERROR_CANCELLED => Refusal::Cancelled,
            _ => Refusal::Failed(io::Error::last_os_error().to_string()),
        });
    }
    let mut code = 1;
    // SAFETY: 设置了 `SEE_MASK_NOCLOSEPROCESS`，`hProcess` 是新进程的句柄，由本函数关闭一次；
    // `code` 是可写的栈变量。
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    match code {
        0 => Ok(()),
        _ => Err(Refusal::Failed(String::new())),
    }
}

/// 把 `arg` 按 Windows 拆分命令行的规则加上引号，追加到 `out`：引号前和结尾的反斜杠加倍，引号
/// 写成 `\"`。
#[cfg(any(windows, test))]
fn quote_argument(arg: &std::ffi::OsStr, out: &mut Vec<u16>) {
    #[cfg(windows)]
    let units: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(arg).collect();
    #[cfg(not(windows))]
    let units: Vec<u16> = arg.to_string_lossy().encode_utf16().collect();
    let (quote, backslash) = (u16::from(b'"'), u16::from(b'\\'));
    out.push(quote);
    let mut backslashes = 0;
    for unit in units {
        if unit == backslash {
            backslashes += 1;
            continue;
        }
        let repeat = if unit == quote {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        out.extend(std::iter::repeat_n(backslash, repeat));
        out.push(unit);
        backslashes = 0;
    }
    out.extend(std::iter::repeat_n(backslash, backslashes * 2));
    out.push(quote);
}

/// 以管理员身份运行 `exe` 并等它结束（系统的授权对话框）。
#[cfg(target_os = "macos")]
fn run_as_admin(exe: &Path, args: &[OsString], _: Owner, name: &str) -> Result<(), Refusal> {
    /// `osascript` 报告的「用户已取消」错误码（`AppleScript` 的 `userCanceledErr`）。
    const USER_CANCELED: &str = "(-128)";
    let quote = |text: &std::ffi::OsStr| crate::app::terminal::shell_quote(Path::new(text));
    let mut command = quote(exe.as_os_str());
    for arg in args {
        command.push(' ');
        command.push_str(&quote(arg));
    }
    // 授权对话框里说明是谁、要做什么，而不是只说 osascript 要做出更改。
    let prompt = t!("workspace-save-admin-prompt", name = name);
    let script = crate::shell_command::authorized_script(&command, Some(&prompt));
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| Refusal::Failed(e.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(match message.contains(USER_CANCELED) {
        true => Refusal::Cancelled,
        false => Refusal::Failed(message),
    })
}

/// 以管理员身份运行 `exe` 并等它结束（经 `pkexec` 由桌面环境的 polkit 授权代理询问）。
#[cfg(all(unix, not(target_os = "macos")))]
fn run_as_admin(exe: &Path, args: &[OsString], _: Owner, _: &str) -> Result<(), Refusal> {
    /// `pkexec` 的退出码：用户关掉了授权对话框。
    const DISMISSED: i32 = 126;
    /// `pkexec` 的退出码：没有授权，或拿不到授权（例如没有授权代理）。
    const NOT_AUTHORIZED: i32 = 127;
    let output = std::process::Command::new("pkexec")
        .arg(exe)
        .args(args)
        .output()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => {
                Refusal::Failed(t!("workspace-save-admin-unavailable").to_owned())
            }
            _ => Refusal::Failed(e.to_string()),
        })?;
    let reason = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match output.status.code() {
        Some(0) => Ok(()),
        Some(DISMISSED) => Err(Refusal::Cancelled),
        // 没有授权、拿不到授权（没有授权代理）或运行不了程序：`pkexec` 写下的原因能分清这几种。
        Some(NOT_AUTHORIZED) if reason.is_empty() => Err(Refusal::Failed(
            t!("workspace-save-admin-unavailable").to_owned(),
        )),
        _ => Err(Refusal::Failed(reason)),
    }
}

/// 其他系统上没有以管理员身份运行的办法。
#[cfg(not(any(unix, windows)))]
fn run_as_admin(_: &Path, _: &[OsString], _: Owner, _: &str) -> Result<(), Refusal> {
    Err(Refusal::Failed(String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_is_written_in_place_and_keeps_the_file() {
        let dir = private_dir().unwrap();
        let (copy, target) = (dir.join("content"), dir.join("hosts"));
        std::fs::write(&copy, "127.0.0.1 example.test\n").unwrap();
        std::fs::write(&target, "old and much longer content\n").unwrap();
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&target).unwrap());
        write(&copy, &target, Owner(0)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "127.0.0.1 example.test\n"
        );
        // 原地覆盖：还是原来那个文件，不是新建再改名的。
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&target).unwrap()),
            inode
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_puts_the_original_back() {
        let dir = private_dir().unwrap();
        let target = dir.join("hosts");
        std::fs::write(&target, "original\n").unwrap();
        // 内容文件不存在：读取失败，原文件一个字节也不动。
        assert!(write(&dir.join("missing"), &target, Owner(0)).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 授权期间目标被换成别的文件或符号链接：写入进程按路径打开到的不是原来的文件，不写。
    #[cfg(unix)]
    #[test]
    fn a_target_replaced_after_it_was_identified_is_not_written() {
        let dir = private_dir().unwrap();
        let (copy, target, other) = (dir.join("content"), dir.join("hosts"), dir.join("other"));
        std::fs::write(&copy, "new\n").unwrap();
        std::fs::write(&target, "original\n").unwrap();
        std::fs::write(&other, "elsewhere\n").unwrap();
        let identity = Identity::of(&target).unwrap();
        assert_eq!(Identity::parse(&identity.argument()), Some(identity));

        // 换成指向别处的符号链接。原来的文件改名留着：删掉后它的 inode 号可以立刻分给新建的
        // 文件，下面换上的普通文件就无从区分。
        std::fs::rename(&target, dir.join("moved")).unwrap();
        std::os::unix::fs::symlink(&other, &target).unwrap();
        assert!(write_in_place(&copy, &target, identity).is_err());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "elsewhere\n");
        // 换成另一个普通文件。
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, "another\n").unwrap();
        assert!(write_in_place(&copy, &target, identity).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "another\n");
        // 符号链接本身不能作为目标。
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(&other, &target).unwrap();
        assert!(Identity::of(&target).is_err());

        // 目标不存在时在核对过的目录里新建；已有同名的项时不覆盖。
        let fresh = dir.join("fresh");
        let identity = Identity::of(&fresh).unwrap();
        assert!(matches!(identity, Identity::Dir(..)));
        write_in_place(&copy, &fresh, identity).unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "new\n");
        assert!(write_in_place(&copy, &fresh, identity).is_err());
        assert!(Identity::parse("file:1".as_ref()).is_none());
        assert!(Identity::parse("link:1:2".as_ref()).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_local_files_the_system_refused_can_retry() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        let os_denied = io::Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 });
        let local = Path::new("/etc/hosts");
        assert!(can_retry(local, &os_denied));
        // winc 自己的拒绝（文档只读）不是系统的拒绝。
        assert!(!can_retry(local, &denied));
        assert!(!can_retry(local, &io::Error::other("full")));
        let target = winc_backend::Target::new("host").unwrap();
        let remote = winc_backend::virtual_path(&target, b"/etc/hosts");
        assert!(!can_retry(&remote, &os_denied));
    }

    #[test]
    fn windows_arguments_are_quoted_for_the_command_line() {
        let quoted = |arg: &str| {
            let mut out = Vec::new();
            quote_argument(arg.as_ref(), &mut out);
            String::from_utf16(&out).unwrap()
        };
        assert_eq!(
            quoted(r"C:\Program Files\Winc\winc.exe"),
            r#""C:\Program Files\Winc\winc.exe""#
        );
        assert_eq!(quoted(r"C:\dir\"), r#""C:\dir\\""#);
        assert_eq!(quoted(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
    }
}
