//! 命令行入口：让 `winc .` 在终端里可用。
//!
//! Windows 安装包和 deb 包已经把 `winc` 加入 `PATH`。macOS 的程序位于 `.app` 内，AppImage 没有固定
//! 路径，由用户执行命令后创建符号链接（[`placement`]）。启动时不自动安装：写入系统目录必须由用户
//! 发起。
//!
//! 从终端启动时，窗口进程转到后台（[`detach`]），命令立即返回；macOS 的应用包由 `LaunchServices`
//! 启动。转到后台之前报告读不了的目标（[`report_unreadable`]），错误仍然回到终端。

#[cfg(any(target_os = "macos", all(test, unix)))]
use crate::app::terminal::shell_quote;
use crate::cli::Args;

/// 符号链接的位置和它指向的程序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    /// 符号链接的路径。
    pub link: std::path::PathBuf,
    /// 链接指向的程序。
    pub target: std::path::PathBuf,
}

/// 安装或卸载的结果。Windows 上没有要安装的，除 `Failed` 外不会用到。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) enum Outcome {
    /// 已创建链接。
    Installed,
    /// 已移除链接。
    Uninstalled,
    /// 卸载时没有链接。
    NotInstalled,
    /// 链接位置上是其他程序，没有改动。
    Occupied,
    /// 没有写入权限，需要授权。
    Denied,
    /// 其他失败，带系统的错误说明。
    Failed(String),
}

/// 需要授权的安装或卸载结束时送回主线程的结果。
#[derive(Debug)]
pub(crate) struct Done {
    /// 操作的链接。
    pub placement: Placement,
    /// 结果；用户取消授权时为 `None`。
    pub outcome: Option<Outcome>,
}

/// 返回当前运行方式下要创建的链接；Windows、deb 包和开发构建无需安装，返回 `None`。
pub(crate) fn placement() -> Option<Placement> {
    #[cfg(target_os = "macos")]
    {
        let exe = winc_config::files::canonicalize(&std::env::current_exe().ok()?).ok()?;
        macos_placement(&exe)
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var_os("HOME")?;
        appimage_placement(&current_appimage()?, home.as_ref())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// macOS：程序位于 `.app` 内时链接到 `/usr/local/bin/winc`，该目录在系统默认的 `PATH`
/// （`/etc/paths`）中。
#[cfg(any(target_os = "macos", all(test, unix)))]
fn macos_placement(exe: &std::path::Path) -> Option<Placement> {
    bundle_of(exe)?;
    Some(Placement {
        link: "/usr/local/bin/winc".into(),
        target: exe.to_path_buf(),
    })
}

/// 返回程序 `exe` 所在的应用包（`.app` 目录）；不在应用包内时返回 `None`。
#[cfg(any(target_os = "macos", all(test, unix)))]
fn bundle_of(exe: &std::path::Path) -> Option<&std::path::Path> {
    let dir = exe.parent()?;
    let bundle = dir.ancestors().nth(2)?;
    (dir.ends_with("Contents/MacOS") && bundle.extension().is_some_and(|e| e == "app"))
        .then_some(bundle)
}

/// 返回运行中的 AppImage 文件。
///
/// 只有程序位于 `APPDIR`（AppImage 运行时挂载的目录）内才算：这两个变量会被子进程继承，在 AppImage
/// 版的内置终端里运行其他安装方式的 `winc` 时同样存在。
#[cfg(any(target_os = "linux", all(test, unix)))]
fn appimage_of(
    exe: &std::path::Path,
    appdir: Option<&std::ffi::OsStr>,
    appimage: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    exe.starts_with(appdir?).then(|| appimage.map(Into::into))?
}

#[cfg(target_os = "linux")]
fn current_appimage() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let appdir = std::env::var_os("APPDIR");
    let appimage = std::env::var_os("APPIMAGE");
    appimage_of(&exe, appdir.as_deref(), appimage.as_deref())
}

/// AppImage：链接到用户目录的 `~/.local/bin/winc`（XDG 约定的用户可执行文件目录），不需要授权。
#[cfg(any(target_os = "linux", all(test, unix)))]
fn appimage_placement(appimage: &std::path::Path, home: &std::path::Path) -> Option<Placement> {
    (appimage.is_absolute() && home.is_absolute()).then(|| Placement {
        link: home.join(".local/bin/winc"),
        target: appimage.to_path_buf(),
    })
}

/// 链接指向的 `found` 是否由 winc 创建；只有这种链接可以替换或移除。
///
/// winc 创建的链接都指向绝对路径，相对路径的链接一定属于其他程序。目标已不存在的绝对路径链接同样
/// 视为可替换，这类链接是应用移动后的遗留。
#[cfg(unix)]
fn is_ours(placement: &Placement, found: &std::path::Path) -> bool {
    let bundled = found.ends_with("Contents/MacOS/winc");
    let appimage = found.extension().is_some_and(|e| e == "AppImage");
    found.is_absolute() && (found == placement.target || bundled || appimage || !found.exists())
}

/// 创建链接；已有 winc 创建的链接时替换。先创建临时链接再改名，失败时原链接不受影响。
#[cfg(unix)]
pub(crate) fn install(placement: &Placement) -> Outcome {
    let link = &placement.link;
    match std::fs::symlink_metadata(link) {
        Ok(meta) if meta.is_symlink() => match std::fs::read_link(link) {
            Ok(found) if is_ours(placement, &found) => {}
            _ => return Outcome::Occupied,
        },
        Ok(_) => return Outcome::Occupied,
        Err(_) => {}
    }
    let temporary = link.with_file_name(".winc.tmp");
    let create = || {
        if let Some(dir) = link.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&temporary);
        std::os::unix::fs::symlink(&placement.target, &temporary)?;
        std::fs::rename(&temporary, link)
    };
    match create() {
        Ok(()) => Outcome::Installed,
        Err(e) => {
            let _ = std::fs::remove_file(&temporary);
            failure(&e)
        }
    }
}

/// 移除链接；只移除 winc 创建的链接。
#[cfg(unix)]
pub(crate) fn uninstall(placement: &Placement) -> Outcome {
    let link = &placement.link;
    match std::fs::symlink_metadata(link) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Outcome::NotInstalled,
        Err(e) => return failure(&e),
        Ok(meta) if !meta.is_symlink() => return Outcome::Occupied,
        Ok(_) => {}
    }
    match std::fs::read_link(link) {
        Ok(found) if is_ours(placement, &found) => {}
        _ => return Outcome::Occupied,
    }
    match std::fs::remove_file(link) {
        Ok(()) => Outcome::Uninstalled,
        Err(e) => failure(&e),
    }
}

#[cfg(unix)]
fn failure(error: &std::io::Error) -> Outcome {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            Outcome::Denied
        }
        _ => Outcome::Failed(error.to_string()),
    }
}

/// 非 Unix 平台的 [`placement`] 恒为 `None`，不会调用此函数。
#[cfg(not(unix))]
pub(crate) fn install(_: &Placement) -> Outcome {
    Outcome::Failed(String::new())
}

/// 非 Unix 平台的 [`placement`] 恒为 `None`，不会调用此函数。
#[cfg(not(unix))]
pub(crate) fn uninstall(_: &Placement) -> Outcome {
    Outcome::Failed(String::new())
}

/// 返回通过系统授权对话框执行 `command` 的 `AppleScript` 脚本。
#[cfg(any(target_os = "macos", all(test, unix)))]
fn authorized_script(command: &str) -> String {
    let escaped = command.replace('\\', r"\\").replace('"', "\\\"");
    format!("do shell script \"{escaped}\" with administrator privileges")
}

/// 返回需要授权时执行的 shell 命令：`installing` 为真时创建链接，否则移除。
#[cfg(any(target_os = "macos", all(test, unix)))]
fn authorized_command(placement: &Placement, installing: bool) -> String {
    let link = shell_quote(&placement.link);
    if !installing {
        return format!("/bin/rm -f {link}");
    }
    let target = shell_quote(&placement.target);
    let dir = placement.link.parent().unwrap_or(&placement.link);
    let dir = shell_quote(dir);
    format!("/bin/mkdir -p {dir} && /bin/ln -sfn {target} {link}")
}

/// 显示系统授权对话框并执行安装或卸载。调用会阻塞到用户响应，必须在后台线程调用。
///
/// 用户取消时返回 `None`。
#[cfg(target_os = "macos")]
pub(crate) fn run_authorized(placement: &Placement, installing: bool) -> Option<Outcome> {
    /// `osascript` 报告的「用户已取消」错误码（`AppleScript` 的 `userCanceledErr`）。
    const USER_CANCELED: &str = "(-128)";
    let script = authorized_script(&authorized_command(placement, installing));
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output();
    match output {
        Ok(output) if output.status.success() => Some(if installing {
            Outcome::Installed
        } else {
            Outcome::Uninstalled
        }),
        Ok(output) => {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            (!message.contains(USER_CANCELED)).then_some(Outcome::Failed(message))
        }
        Err(e) => Some(Outcome::Failed(e.to_string())),
    }
}

/// 通过符号链接启动时切换到程序的真实路径，参数和标准输入输出不变。
///
/// macOS 报告的可执行文件路径不解析符号链接，程序要由真实路径定位应用包和同目录的组件。切换失败时
/// 继续运行。
pub(crate) fn enter_real_path() {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Ok(real) = winc_config::files::canonicalize(&exe) else {
            return;
        };
        if real != exe {
            let _ = std::process::Command::new(real)
                .args(std::env::args_os().skip(1))
                .exec();
        }
    }
}

/// 把读不了的目标从 `args` 中移除并报告到标准错误，返回是否有这样的目标。
///
/// 不存在的文件不算错误：打开时新建。临时打开（`--wait`、`-`）留在前台，由后续流程报告。
pub(crate) fn report_unreadable(args: &mut Args) -> bool {
    if args.one_off() {
        return false;
    }
    let count = args.targets.len();
    args.targets
        .retain(|target| match unreadable(&target.path) {
            Some(error) => {
                let path = target.path.display().to_string();
                let message = winc_i18n::t!("cmd-cli-open-failed", path = path, error = error);
                eprintln!("winc: {message}");
                false
            }
            None => true,
        });
    args.targets.len() != count
}

/// 返回目标 `path` 读不了的原因；可以打开或尚不存在时返回 `None`。
///
/// 只尝试打开目录和普通文件：打开没有写入端的命名管道会一直阻塞。
fn unreadable(path: &std::path::Path) -> Option<String> {
    let result = std::fs::metadata(path).and_then(|meta| {
        if meta.is_dir() {
            std::fs::read_dir(path).map(|_| ())
        } else if meta.is_file() {
            std::fs::File::open(path).map(|_| ())
        } else {
            Ok(())
        }
    });
    let error = result.err()?;
    (error.kind() != std::io::ErrorKind::NotFound).then(|| error.to_string())
}

/// 从终端启动时，用同样的参数在后台启动窗口进程；已启动时返回真，调用方应当退出。
///
/// Unix 上仅在标准输入是终端时转移；新进程的标准输入不是终端，不会再次转移。Windows 上仅在由启动
/// 脚本启动（[`Args::launcher`]）时转移；新进程的参数不带该标记，不会再次转移。`--wait` 和 `-`
/// 必须留在前台，`--new-instance` 和开发构建留在前台以便查看输出。
pub(crate) fn detach(args: &Args) -> bool {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        if cfg!(debug_assertions) || stays_in_foreground(args) || !std::io::stdin().is_terminal() {
            return false;
        }
        // 没有图形环境（例如 SSH）时窗口无法打开：留在前台，错误才能回到终端。
        #[cfg(target_os = "linux")]
        if !has_display(
            std::env::var_os("DISPLAY").as_deref(),
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        ) {
            return false;
        }
        #[cfg(target_os = "macos")]
        if let Some(launched) = launch_bundle(args) {
            return launched;
        }
        let Some(program) = background_program() else {
            return false;
        };
        // 独立的进程组：终端关闭时发给前台进程组的 SIGHUP 不会结束窗口进程。
        Command::new(program)
            .args(std::env::args_os().skip(1))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .is_ok()
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        // 图形界面程序由启动脚本（批处理文件或 shell 脚本）调用时，脚本等待程序退出，命令不会
        // 返回；只有经过启动脚本时才转移。
        if cfg!(debug_assertions) || !args.launcher || stays_in_foreground(args) {
            return false;
        }
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        // 先离开命令行的控制台，新进程因此接不上它：关闭终端时系统会结束接在控制台上的进程。
        crate::windows::detach_console();
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let child = Command::new(exe)
            .args(crate::cli::without_launcher_flag(
                std::env::args_os().skip(1),
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn();
        match child {
            Ok(child) => {
                crate::windows::allow_foreground(child.id());
                true
            }
            Err(_) => false,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = args;
        false
    }
}

/// 程序位于 `.app` 内时通过 `LaunchServices`（`open`）启动应用，返回是否成功；不在应用包内时返回
/// `None`。
///
/// 由系统启动的应用是独立的进程，窗口会切到前台，系统权限不归属于终端。新进程不继承 shell 的环境
/// 变量和当前目录，所以目标写成绝对路径；需要保留环境时使用 `--new-instance`。
///
/// `-n` 使参数一定送达：已有同一应用包的实例时，不带 `-n` 只会激活该实例，参数被丢弃；新进程启动后
/// 仍按单实例规则转交。
#[cfg(target_os = "macos")]
fn launch_bundle(args: &Args) -> Option<bool> {
    let exe = std::env::current_exe().ok()?;
    let bundle = bundle_of(&exe)?;
    let status = std::process::Command::new("/usr/bin/open")
        .args(open_arguments(bundle, args))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Some(status.is_ok_and(|status| status.success()))
}

/// 返回通过 `open` 启动应用包 `bundle` 的参数：绝对路径的目标，`文件:行:列` 的行号和列号从 1 起。
#[cfg(any(target_os = "macos", all(test, unix)))]
fn open_arguments(bundle: &std::path::Path, args: &Args) -> Vec<std::ffi::OsString> {
    let mut out: Vec<std::ffi::OsString> = vec!["-n".into(), "-a".into(), bundle.into()];
    if args.targets.is_empty() {
        return out;
    }
    out.push("--args".into());
    for target in &args.targets {
        let mut arg = std::ffi::OsString::from(crate::instance::absolute(&target.path));
        if let Some(line) = target.line {
            arg.push(format!(":{}", line + 1));
            if let Some(col) = target.col {
                arg.push(format!(":{}", col + 1));
            }
        }
        out.push(arg);
    }
    out
}

/// Linux 上是否有可以连接的图形环境。
#[cfg(any(target_os = "linux", all(test, unix)))]
fn has_display(x11: Option<&std::ffi::OsStr>, wayland: Option<&std::ffi::OsStr>) -> bool {
    [x11, wayland]
        .iter()
        .any(|value| value.is_some_and(|value| !value.is_empty()))
}

/// 这次启动是否必须留在前台。
#[cfg(any(unix, windows))]
fn stays_in_foreground(args: &Args) -> bool {
    args.one_off() || args.new_instance
}

/// 返回后台启动的程序。
///
/// AppImage 的文件系统在启动进程退出时卸载，必须重新运行 AppImage 本身。
#[cfg(unix)]
fn background_program() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = current_appimage() {
        return Some(appimage);
    }
    std::env::current_exe().ok()
}

#[cfg(all(test, unix))]
mod tests;
