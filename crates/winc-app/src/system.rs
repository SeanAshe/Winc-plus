//! 交给系统处理的操作：用默认应用打开文件，在文件管理器中显示文件，在外部终端中打开目录，
//! 以及把文件移到回收站。
//!
//! 各函数通过子进程或平台接口完成：子进程由后台线程回收，调用线程不等待其结束。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：读取本机的系统设置和已安装的程序"
)]

use std::io;
use std::path::Path;
use std::process::Command;

/// 返回「显示文件所在位置」按钮上的文字（按当前界面语言）。
///
/// 文字随平台改变：macOS 上取自 `common-reveal-in-finder`，其他平台取自
/// `common-reveal-in-folder`。
pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        winc_i18n::t!("common-reveal-in-finder")
    } else {
        winc_i18n::t!("common-reveal-in-folder")
    }
}

/// 用系统为该类文件设置的默认应用打开 `path`。
///
/// # Errors
///
/// 无法启动系统的打开命令（`open`、`rundll32`、`xdg-open`）时返回 `io::Error`。
pub fn open_with_default_app(path: &Path) -> io::Result<()> {
    // 交给本机的程序，只接受本机路径：远端主机上的文件无法由本机的程序打开。
    winc_backend::ensure_local(path)?;
    if cfg!(target_os = "macos") {
        spawn(Command::new("open").arg(path))
    } else if cfg!(windows) {
        // 不经过 `cmd /C start`：`cmd` 会解释参数中的 `&`、`|`、`%`，网址或文件名中的这些
        // 字符可以拼出另一条命令。`url.dll` 的 `FileProtocolHandler` 按默认应用打开网址和文件，
        // 参数原样交给它，不再经过任何解释。
        spawn(
            Command::new("rundll32.exe")
                .arg("url.dll,FileProtocolHandler")
                .arg(path),
        )
    } else {
        spawn(Command::new("xdg-open").arg(path))
    }
}

/// 用默认浏览器打开本地网页 `path`。
///
/// macOS 上交给系统中负责打开 `https` 链接的应用，因为 `.html` 文件的默认应用可能是编辑器；
/// 其他平台上 `.html` 的默认应用通常是浏览器，因此按默认应用打开。
///
/// # Errors
///
/// 无法启动系统的打开命令时返回 `io::Error`。
pub fn open_in_browser(path: &Path) -> io::Result<()> {
    // 交给本机的程序，只接受本机路径：远端主机上的文件无法由本机的程序打开。
    winc_backend::ensure_local(path)?;
    #[cfg(target_os = "macos")]
    if let Some(browser) = crate::macos::default_browser() {
        return spawn(Command::new("open").arg("-a").arg(browser).arg(path));
    }
    open_with_default_app(path)
}

/// 可以交给系统打开的链接协议。
///
/// 链接来自文档和终端中程序的输出，不可信，显示的文字也可以与实际地址不同。交给系统打开时，
/// `file:` 会运行 `.app`、`.command`，`smb:` 会挂载远程目录，其他协议会启动注册了该协议的
/// 应用。因此只放行网页和邮件，本地文件由调用方在编辑区中打开。
const URL_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// 链接 `url` 的协议是否在 [`URL_SCHEMES`] 中（不区分大小写）。
pub fn openable_url(url: &str) -> bool {
    url.split_once(':')
        .is_some_and(|(scheme, _)| URL_SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)))
}

/// 用系统的默认应用打开链接 `url`，只打开 [`URL_SCHEMES`] 中的协议。
///
/// # Errors
///
/// - 协议不在 [`URL_SCHEMES`] 中时返回 `io::ErrorKind::PermissionDenied`。
/// - 无法启动系统的打开命令时返回 `io::Error`。
pub fn open_url(url: &str) -> io::Result<()> {
    if !openable_url(url) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            winc_i18n::t!("common-link-scheme-not-allowed"),
        ));
    }
    open_with_default_app(Path::new(url))
}

/// 已注册的 WSL 发行版名称，按注册表中的顺序排列；未安装 WSL 时为空。
///
/// 读取注册表不启动 `wsl.exe`，也不会启动发行版。进程内只读取一次，因此可以在菜单中调用。
pub fn wsl_distributions() -> &'static [String] {
    static DISTROS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    DISTROS.get_or_init(read_wsl_distributions)
}

/// 读取当前用户注册的 WSL 发行版，略过 Docker Desktop 的内部发行版：其中没有用户的项目。
#[cfg(windows)]
#[allow(unsafe_code)] // 读取注册表只能调用系统接口
fn read_wsl_distributions() -> Vec<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
        RegOpenKeyExW,
    };

    /// 注册表项名称的缓冲区长度（UTF-16 码元，含结尾的 NUL），由注册表的限制决定：名称最长 255 个
    /// 字符。
    const KEY_NAME: usize = 256;
    /// 发行版名称的缓冲区长度（UTF-16 码元，含结尾的 NUL），与 `KEY_NAME` 相同；更长的值读取失败后
    /// 略过。
    const VALUE_NAME: usize = 256;

    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain([0])
            .collect()
    };
    let lxss = wide(r"Software\Microsoft\Windows\CurrentVersion\Lxss");
    let value = wide("DistributionName");
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: 路径是以 NUL 结尾的 UTF-16，`key` 是有效的输出位置。
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, lxss.as_ptr(), 0, KEY_READ, &mut key) }
        != ERROR_SUCCESS
    {
        return Vec::new();
    }
    let mut distros = Vec::new();
    for index in 0.. {
        let mut name = [0u16; KEY_NAME];
        let mut name_len = KEY_NAME as u32;
        // SAFETY: `key` 已打开；`name` 和 `name_len` 描述同一个缓冲区；其余可选参数为空。
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                name.as_mut_ptr(),
                &mut name_len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        let mut text = [0u16; VALUE_NAME];
        let mut bytes = (VALUE_NAME * 2) as u32;
        // SAFETY: `name` 由上面的调用写成以 NUL 结尾；`text` 和 `bytes` 描述同一个缓冲区。
        let status = unsafe {
            RegGetValueW(
                key,
                name.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                text.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status == ERROR_SUCCESS {
            // `bytes` 含结尾的 NUL。
            let len = (bytes as usize / 2).saturating_sub(1);
            let distro = String::from_utf16_lossy(&text[..len]);
            if !distro.starts_with("docker-desktop") {
                distros.push(distro);
            }
        }
    }
    // SAFETY: `key` 已打开，之后不再使用。
    unsafe { RegCloseKey(key) };
    distros
}

/// 其他平台没有 WSL。
#[cfg(not(windows))]
fn read_wsl_distributions() -> Vec<String> {
    Vec::new()
}

/// 把文件或目录移到废纸篓（回收站）。
///
/// macOS 和 Windows 上交给系统；其他 Unix 平台按 freedesktop.org 的 Trash 规范写入回收站，与
/// 文件管理器共用同一个回收站。
///
/// # Errors
///
/// 系统拒绝移动、用户取消、`path` 无法转换时返回 `io::Error`；平台没有回收站时返回
/// `io::ErrorKind::Unsupported`。
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    // 回收站在本机，只接受本机路径。
    winc_backend::ensure_local(path)?;
    #[cfg(target_os = "macos")]
    {
        crate::macos::move_to_trash(path)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        crate::trash::move_to_trash(path).map(|_| ())
    }

    #[cfg(windows)]
    {
        crate::windows::move_to_recycle_bin(path)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            winc_i18n::t!("common-trash-unsupported"),
        ))
    }
}

/// 在文件管理器中显示并选中 `path`。
///
/// macOS 以外的 Unix 平台上经 D-Bus 的 `org.freedesktop.FileManager1` 接口请桌面的文件管理器
/// 选中它；没有该服务时只打开所在的目录。`path` 不存在时打开其父目录。
///
/// # Errors
///
/// 无法启动系统的文件管理器命令时返回 `io::Error`；D-Bus 的调用在后台进行，不报告错误。
pub fn reveal_in_file_manager(path: &Path) -> io::Result<()> {
    // 交给本机的程序，只接受本机路径：远端主机上的文件无法由本机的程序打开。
    winc_backend::ensure_local(path)?;
    if !path.exists() {
        let folder = path.parent().unwrap_or(path);
        return open_with_default_app(folder);
    }

    if cfg!(target_os = "macos") {
        spawn(Command::new("open").arg("-R").arg(path))
    } else if cfg!(windows) {
        let mut select = std::ffi::OsString::from("/select,");
        select.push(path);
        spawn(Command::new("explorer").arg(select))
    } else {
        #[cfg(all(unix, not(target_os = "macos")))]
        if !cfg!(test) {
            let path = std::path::absolute(path)?;
            std::thread::spawn(move || {
                if show_items(&path).is_err() {
                    let folder = path.parent().unwrap_or(&path);
                    if let Err(e) = spawn(Command::new("xdg-open").arg(folder)) {
                        eprintln!("winc: 无法打开文件管理器：{e}");
                    }
                }
            });
            return Ok(());
        }
        let folder = path.parent().unwrap_or(path);
        spawn(Command::new("xdg-open").arg(folder))
    }
}

/// 请桌面的文件管理器打开 `path` 所在的目录并选中 `path`（接口方法 `ShowItems`）。
#[cfg(all(unix, not(target_os = "macos")))]
fn show_items(path: &Path) -> zbus::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let uri = format!("file://{}", crate::trash::encode_path(path));
    connection.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        &(vec![uri.as_str()], ""),
    )?;
    Ok(())
}

/// 未指定终端程序时依次尝试的终端程序名。
///
/// 顺序即优先级：先是发行版设置的默认终端（Debian 系的替代项），再是各桌面环境自带的，
/// 最后是 X 自带的。取第一个在环境变量 `PATH` 的目录中存在可执行文件的项；数量由这些候选项
/// 决定。
const TERMINALS: [&str; 6] = [
    "x-terminal-emulator",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "mate-terminal",
    "xterm",
];

/// 在外部终端中打开 `dir`。
///
/// `app` 是配置中指定的终端程序。未指定时 macOS 用系统自带的终端应用，Windows 用 `cmd.exe`，
/// 其他平台用环境变量 `$TERMINAL`，未设置时用 [`TERMINALS`] 中第一个已安装的。
///
/// # Errors
///
/// 无法启动终端程序时返回 `io::Error`。
pub fn open_terminal(dir: &Path, app: Option<&str>) -> io::Result<()> {
    // 交给本机的程序，只接受本机路径：远端主机上的文件无法由本机的程序打开。
    winc_backend::ensure_local(dir)?;
    if cfg!(target_os = "macos") {
        // `open -a` 按应用名或应用路径（例如 `/Applications/…`）查找应用，并把 `dir` 作为待打开的
        // 文档交给它；终端类应用据此在该目录下新建窗口。
        let app = app.unwrap_or("Terminal");
        spawn(Command::new("open").args(["-a", app]).arg(dir))
    } else if cfg!(windows) {
        let app = app.unwrap_or("cmd.exe");
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", app]).current_dir(dir);
        // 只借用 `cmd` 的 `start` 启动终端：`cmd` 自身不需要窗口，打开的终端有自己的窗口。
        // `0x0800_0000` 是 `CREATE_NO_WINDOW`，即不为 `cmd` 创建控制台窗口。
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        spawn(&mut command)
    } else {
        let app = app
            .map(str::to_owned)
            .or_else(|| std::env::var("TERMINAL").ok())
            .or_else(|| {
                let path = std::env::var_os("PATH")?;
                TERMINALS
                    .into_iter()
                    .find(|name| std::env::split_paths(&path).any(|d| d.join(name).is_file()))
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| TERMINALS[0].to_owned());
        spawn(Command::new(app).current_dir(dir))
    }
}

/// 启动子进程，并在后台线程中等待其结束，避免留下僵尸进程。
///
/// 测试中不实际启动：测试会模拟按键和菜单操作，真实启动会弹出文件管理器、浏览器或终端的窗口。
fn spawn(command: &mut Command) -> io::Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests;
