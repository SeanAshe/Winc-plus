//! WSL：Windows 上通过 `\\wsl.localhost\<发行版>\...` 访问的目录。
//!
//! 这些目录中的文件由 Windows 读写，Git 和终端在对应的发行版内运行。本模块负责两种路径写法之间的
//! 换算。换算本身不依赖平台；仅在 Windows 上生效的入口在各自的文档中说明。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::shell::environment;
use crate::shell::process::{self, Cancel};

/// 访问发行版文件系统的主机名：`wsl.localhost` 是当前的写法，`wsl$` 是早期的写法，两者均可用。
const HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];

/// 通过 `WSLENV` 传给发行版内 `git` 的环境变量：`wsl.exe` 默认不向发行版传递环境变量。
pub(crate) const GIT_ENV: &str = "GIT_OPTIONAL_LOCKS:GIT_TERMINAL_PROMPT:GIT_EDITOR:GIT_SEQUENCE_EDITOR:GIT_INDEX_FILE:NO_COLOR:LC_ALL:\
LANGUAGE";

/// 通过 `WSLENV` 传给发行版内格式化工具的环境变量。
pub(crate) const FORMAT_ENV: &str = "WINC_FILE:NO_COLOR";

/// 用用户的 shell 以交互式登录方式运行 `$0` 的脚本。
///
/// 版本管理工具（如 nvm）的初始化通常写在只有交互式 shell 才读取的文件中。
const LOGIN_SHELL: &str = r#"exec "${SHELL:-/bin/sh}" -i -l -c "$0""#;

/// 运行 Windows 路径 `$0` 处的 Linux 程序的脚本：`wslpath` 按发行版的挂载配置换算路径。
const RUN_WINDOWS_FILE: &str = r#"exec "$(wslpath -u "$0")" "$@""#;

/// 已查询过的各发行版用户 `PATH`：（发行版名称，值）；查询失败时值为 `None`。
static LOGIN_PATHS: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

/// 在发行版内核对并复制文件权限的脚本：`$1` 是原文件，`$2` 是临时文件。
///
/// 原文件有多个硬链接或属于其他用户时以非零状态退出，这时改名替换会改变文件的身份。
const KEEP_IDENTITY: &str = concat!(
    r#"test "$(stat -c %h "$1")" = 1 && "#,
    r#"test "$(stat -c %u "$1")" = "$(id -u)" && "#,
    r#"chmod "$(stat -c %a "$1")" "$2""#,
);

/// 发行版内单次查询的最长等待时间（秒）。
///
/// 发行版尚未启动时包含其启动耗时，取值与 `git` 只读查询的超时相同。
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// 发行版中的一个位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 路径中的主机名。换算回 Windows 路径时沿用，使同一仓库的路径可以相互比较。
    host: String,
    /// 发行版名称。
    pub distro: String,
    /// 发行版内的绝对路径。
    pub path: String,
}

/// 返回 `path` 对应的发行版位置；不是发行版中的路径时返回 `None`。
///
/// 接受 `\\?\UNC\` 前缀和两种斜杠，主机名不区分大小写。
#[must_use]
pub fn locate(path: &Path) -> Option<Location> {
    let text = path.to_string_lossy().replace('/', "\\");
    let rest = text
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| text.strip_prefix(r"\\"))?;
    let mut parts = rest.split('\\').filter(|part| !part.is_empty());
    let host = parts.next()?;
    let host = HOSTS.iter().find(|h| h.eq_ignore_ascii_case(host))?;
    let distro = parts.next()?.to_owned();
    let path = parts.fold(String::new(), |path, part| path + "/" + part);
    Some(Location {
        host: (*host).to_owned(),
        distro,
        path: if path.is_empty() {
            "/".to_owned()
        } else {
            path
        },
    })
}

/// 同 [`locate`]，但仅在 Windows 上生效：其他平台上这种写法不表示发行版中的路径。
#[must_use]
pub fn locate_here(path: &Path) -> Option<Location> {
    if cfg!(windows) { locate(path) } else { None }
}

/// 把 `git` 输出的路径 `text` 转换为本机路径。
///
/// `root` 位于发行版中时，绝对路径按同一发行版内的路径处理；其余路径相对于 `root`。
#[must_use]
pub fn resolve(root: &Path, text: &str) -> PathBuf {
    match locate_here(root).filter(|_| text.starts_with('/')) {
        Some(place) => place.windows_path(text),
        None => root.join(text),
    }
}

/// 返回传给 `git` 的路径参数：`root` 和 `path` 都位于发行版中时使用发行版内的路径。
#[must_use]
pub fn git_arg(root: &Path, path: &Path) -> String {
    match locate_here(root).and(locate_here(path)) {
        Some(place) => place.path,
        None => path.to_string_lossy().into_owned(),
    }
}

/// 返回存放 `git` 临时文件的目录。
///
/// `root` 位于发行版中时使用发行版的 `/tmp`：发行版内的 `git` 可以直接访问，且不依赖 Windows 磁盘的
/// 自动挂载。其余情况使用系统的临时目录。
#[must_use]
pub fn temp_dir(root: &Path) -> PathBuf {
    match locate_here(root) {
        Some(place) => place.windows_path("/tmp"),
        None => std::env::temp_dir(),
    }
}

/// 把终端输出中的路径 `path` 转换为本机路径。
///
/// `root` 位于发行版中时，`/` 开头的路径按同一发行版内的路径处理；其余路径原样返回。
#[must_use]
pub fn from_terminal(root: &Path, path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match locate_here(root).filter(|_| text.starts_with('/')) {
        Some(place) => place.windows_path(&text),
        None => path,
    }
}

/// 让 `command`（`wsl.exe`）进入发行版 `distro`；`wsl.exe` 自身的信息以 UTF-8 输出。
pub(crate) fn enter(command: &mut Command, distro: &str) {
    command.args(["-d", distro]).env("WSL_UTF8", "1");
}

/// 返回发行版 `distro` 中用户配置的 `PATH`；无法取得时返回 `None`。
///
/// 运行用户的 shell 可能较慢，启动脚本还可能等到 [`QUERY_TIMEOUT`]，因此每个发行版在进程内只查询
/// 一次，失败的结果同样保留。
pub(crate) fn login_path(distro: &str) -> Option<String> {
    let paths = || LOGIN_PATHS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, path)) = paths().iter().find(|(name, _)| name == distro) {
        return path.clone();
    }
    let mut command = Command::new("wsl.exe");
    enter(&mut command, distro);
    command
        .args([
            "--exec",
            "sh",
            "-c",
            LOGIN_SHELL,
            &environment::print_script(),
        ])
        // 用户的启动脚本可据此跳过只应在打开终端时运行的内容（启动 tmux、代理进程等）。
        .env("WINC_RESOLVING_ENVIRONMENT", "1")
        .env("WSLENV", "WINC_RESOLVING_ENVIRONMENT")
        .stderr(Stdio::null());
    let path = process::run(&mut command, Vec::new(), QUERY_TIMEOUT, &Cancel::default())
        .ok()
        .and_then(|output| environment::path_in(&output.stdout));
    paths().push((distro.to_owned(), path.clone()));
    path
}

/// 让 `command`（`wsl.exe`）在 `path` 下运行随后的 shell 脚本和参数；`path` 是 `None` 时由登录
/// shell 提供 `PATH`。
fn enter_shell(command: &mut Command, path: Option<&str>) {
    match path {
        Some(path) => command.args(["--exec", "env", &format!("PATH={path}"), "sh", "-c"]),
        None => command.args(["--exec", "sh", "-lc"]),
    };
}

/// 让 `command`（`wsl.exe`）在发行版的目录 `dir` 中，以 `path` 运行随后追加的程序和参数。
pub(crate) fn enter_login(command: &mut Command, distro: &str, dir: &str, path: Option<&str>) {
    enter(command, distro);
    command.args(["--cd", dir]);
    enter_shell(command, path);
    command.arg(r#"exec "$0" "$@""#);
}

/// 返回发行版 `distro` 的根目录：优先使用 `wsl.localhost`，早期系统只有 `wsl$`。
#[must_use]
pub fn distro_root(distro: &str) -> PathBuf {
    let current = Path::new(r"\\wsl.localhost").join(distro);
    if current.exists() {
        current
    } else {
        Path::new(r"\\wsl$").join(distro)
    }
}

/// 把发行版 `distro` 中的命令行参数 `path` 转换为本机路径：`/` 开头的是发行版内的绝对路径，相对
/// 路径由启动进程的工作目录决定，原样返回。
#[must_use]
pub fn from_distro(distro: &str, path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix('/') {
        Some(rest) => rest
            .split('/')
            .fold(distro_root(distro), |dir, part| dir.join(part)),
        None => path,
    }
}

/// 在发行版 `distro` 中查找程序 `name` 的绝对路径；未找到或查询失败时返回 `None`。
pub(crate) fn find_program(distro: &str, path: Option<&str>, name: &str) -> Option<PathBuf> {
    let mut command = Command::new("wsl.exe");
    enter(&mut command, distro);
    enter_shell(&mut command, path);
    command.args([r#"command -v "$0""#, name]);
    let output = process::run(&mut command, Vec::new(), QUERY_TIMEOUT, &Cancel::default()).ok()?;
    output
        .status
        .success()
        .then(|| linux_program(&output.stdout))
        .flatten()
}

/// 返回发行版内的监视程序 `winc-watch` 的路径（在主程序旁边）；不存在时返回 `None`。
///
/// 环境变量 `WINC_WATCH` 可以指定其他路径，用于开发时运行。
pub(crate) fn watch_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WINC_WATCH") {
        return Some(path.into());
    }
    let path = std::env::current_exe().ok()?.parent()?.join("winc-watch");
    path.exists().then_some(path)
}

/// 返回在发行版内监视位置 `place` 的命令。程序直接从 Windows 的安装目录运行，不复制到发行版。
pub(crate) fn watch_command(place: &Location, helper: &Path) -> Command {
    let mut command = Command::new("wsl.exe");
    enter(&mut command, &place.distro);
    command
        .args(["--exec", "sh", "-c", RUN_WINDOWS_FILE])
        .arg(helper)
        .arg(&place.path);
    command
}

/// 从 `command -v` 的输出中取出发行版内的程序路径。
///
/// 启动脚本可能输出其他内容，结果在最后一行。`/mnt/` 下的是 WSL 从 Windows 的 `PATH` 带来的 Windows
/// 程序，它们无法处理发行版内的路径，按未找到处理。
fn linux_program(output: &[u8]) -> Option<PathBuf> {
    let text = String::from_utf8_lossy(output);
    let last = text.lines().next_back()?.trim();
    (last.starts_with('/') && !last.starts_with("/mnt/")).then(|| PathBuf::from(last))
}

/// `path` 所在的位置当前能否访问：发行版中的路径取决于发行版的根目录，其他路径总是返回 `true`。
///
/// 发行版已注销或文件共享不可用时，其中的路径同样报告不存在，这不代表文件已被删除。
#[must_use]
pub fn reachable(path: &Path) -> bool {
    locate_here(path).is_none_or(|place| place.windows_path("/").exists())
}

/// 让临时文件 `temp` 改名替换 `target` 后保持 `target` 的权限和属主。
///
/// Windows 无法读取发行版中文件的 Linux 权限，因此在发行版内处理。
///
/// # Errors
///
/// `wsl.exe` 无法运行，或 `target` 有多个硬链接、属于其他用户时返回错误。
pub(crate) fn keep_identity(target: &Path, temp: &Path) -> std::io::Result<()> {
    let refused = || std::io::Error::other(winc_i18n::t!("editor-large-identity"));
    let (Some(place), Some(from)) = (locate_here(target), locate_here(temp)) else {
        return Err(refused());
    };
    let mut command = Command::new("wsl.exe");
    enter(&mut command, &place.distro);
    command.args([
        "--exec",
        "sh",
        "-c",
        KEEP_IDENTITY,
        "sh",
        &place.path,
        &from.path,
    ]);
    match process::run(&mut command, Vec::new(), QUERY_TIMEOUT, &Cancel::default()) {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(refused()),
    }
}

/// 本机是否已安装 WSL。只检查 `wsl.exe` 是否存在，不启动它。
#[must_use]
pub fn available() -> bool {
    cfg!(windows)
        && std::env::var_os("SystemRoot")
            .is_some_and(|root| Path::new(&root).join(r"System32\wsl.exe").is_file())
}

impl Location {
    /// 返回本发行版内的绝对路径 `linux` 在 Windows 上的访问路径。
    #[must_use]
    pub fn windows_path(&self, linux: &str) -> PathBuf {
        let mut text = format!(r"\\{}\{}", self.host, self.distro);
        for part in linux.split('/').filter(|part| !part.is_empty()) {
            text.push('\\');
            text.push_str(part);
        }
        PathBuf::from(text)
    }
}

#[cfg(test)]
mod tests;
