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
///
/// 只有命令上设置了的才实际传递（见 [`drop_unset`]）：临时暂存区 `GIT_INDEX_FILE` 只在部分
/// 提交时设置。
pub(crate) const GIT_ENV: &str = "GIT_OPTIONAL_LOCKS:GIT_TERMINAL_PROMPT:GIT_EDITOR:GIT_SEQUENCE_EDITOR:GIT_INDEX_FILE:NO_COLOR:LC_ALL:\
LANGUAGE";

/// 发行版内的 `git` 经互操作运行 Windows 上的 askpass 程序时，在 [`GIT_ENV`] 之外传递的环境变量。
///
/// 程序路径按 `/p` 换算为发行版内的路径，带 `/u` 的变量只传入发行版。
/// 端点 `WINC_GIT_ASKPASS` 不带标志，由 `git` 继续传给 askpass 程序；
/// `ssh` 设置的 `SSH_ASKPASS_PROMPT` 带 `/w`，只从发行版传回 Windows。
pub(crate) const ASKPASS_ENV: &str = "GIT_ASKPASS/pu:SSH_ASKPASS/pu:SSH_ASKPASS_REQUIRE/u:\
WINC_GIT_ASKPASS:SSH_ASKPASS_PROMPT/w";

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

/// 已查询过的各发行版用户主目录：（发行版名称，发行版内的路径）；查询失败时值为 `None`。
static HOMES: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

/// 在发行版内核对并复制文件权限的脚本：`$1` 是原文件，`$2` 是临时文件。
///
/// 原文件有多个硬链接或属于其他用户时以非零状态退出，这时改名替换会改变文件的身份。
const KEEP_IDENTITY: &str = concat!(
    r#"test "$(stat -c %h "$1")" = 1 && "#,
    r#"test "$(stat -c %u "$1")" = "$(id -u)" && "#,
    r#"chmod "$(stat -c %a "$1")" "$2""#,
);

/// 在发行版内复制文件的脚本：`$1` 是源目录，`$2` 是目标目录，其余参数是相对于两者的路径。
///
/// `cp -a` 保留权限和符号链接。每复制完一项，输出该项的路径并以 NUL 结尾；
/// 某一项复制失败时以非零状态退出。
const COPY_ENTRIES: &str = concat!(
    r#"from=$1 to=$2; shift 2; for p do "#,
    r#"mkdir -p -- "$to/$(dirname -- "$p")" && cp -a -- "$from/$p" "$to/$p" || exit 1; "#,
    r#"printf '%s\0' "$p"; done"#,
);

/// 单次复制命令中路径参数的总长度上限（字节）。
///
/// Windows 命令行最长 32767 个 UTF-16 码元，路径的 UTF-8 字节数不少于其码元数；取约一半，
/// 为脚本、源目录和目标目录等其他参数留出余量。
const COPY_ARGS_LIMIT: usize = 16 << 10;

/// 发行版内复制的最长等待时间（30 分钟）。
///
/// 复制依赖目录等大型目录可能需要数分钟；超时只用于结束失去响应的进程。
const COPY_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 发行版内单次查询的最长等待时间（秒）。
///
/// 发行版尚未启动时包含其启动耗时，取值与 `git` 只读查询的超时相同。
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// 发行版中的一个位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 路径开头到发行版名称之前的部分，按原样保留（`\\wsl.localhost\`、`\\?\UNC\WSL$\` 等）。
    ///
    /// 换算回 Windows 路径时沿用该部分，同一仓库中的路径才能直接比较：Windows 上比较路径时，
    /// 前缀的种类或主机名的大小写不同即视为不同的路径。
    prefix: String,
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
    let verbatim = r"\\?\UNC\";
    let rest = text
        .strip_prefix(verbatim)
        .or_else(|| text.strip_prefix(r"\\"))?;
    let mut parts = rest.split('\\').filter(|part| !part.is_empty());
    let host = parts.next()?;
    if !HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return None;
    }
    let lead = if text.starts_with(verbatim) {
        verbatim
    } else {
        r"\\"
    };
    let distro = parts.next()?.to_owned();
    let path = parts.fold(String::new(), |path, part| path + "/" + part);
    Some(Location {
        prefix: format!(r"{lead}{host}\"),
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

/// `a` 与 `b` 是否位于同一系统中：同一个发行版，或者都不在发行版中。
///
/// 一侧的 `git` 无法使用另一侧的路径，由 `git` 记录路径的目录（例如工作树）须与仓库位于同一系统。
#[must_use]
pub fn same_system(a: &Path, b: &Path) -> bool {
    match (locate_here(a), locate_here(b)) {
        (Some(a), Some(b)) => a.distro.eq_ignore_ascii_case(&b.distro),
        (a, b) => a.is_none() && b.is_none(),
    }
}

/// 返回 `root` 所在系统中当前用户的主目录；无法取得时返回 `None`。
///
/// `root` 位于发行版中时，返回发行版内 `HOME` 在 Windows 上的访问路径；
/// 否则返回本机用户的主目录（Windows 上为 `USERPROFILE`，其他平台为 `HOME`）。
/// 发行版内的查询需要运行 `wsl.exe`，每个发行版在进程内只查询一次，界面线程应使用后台取得的结果。
#[must_use]
pub fn home_dir(root: &Path) -> Option<PathBuf> {
    let Some(place) = locate_here(root) else {
        let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        return std::env::var_os(name).map(PathBuf::from);
    };
    let home = remembered(&HOMES, &place.distro, || {
        let mut command = Command::new("wsl.exe");
        enter(&mut command, &place.distro);
        command
            .args(["--exec", "printenv", "HOME"])
            .stderr(Stdio::null());
        let output =
            process::run(&mut command, Vec::new(), QUERY_TIMEOUT, &Cancel::default()).ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        let home = text.trim_end_matches('\n');
        (output.status.success() && home.starts_with('/')).then(|| home.to_owned())
    })?;
    Some(place.windows_path(&home))
}

/// 返回 `store` 中发行版 `distro` 的查询结果。
///
/// 尚未查询时运行 `query` 并记下结果，失败的结果同样记下。运行 `query` 期间不持有锁。
fn remembered(
    store: &Mutex<Vec<(String, Option<String>)>>,
    distro: &str,
    query: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let entries = || store.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, value)) = entries().iter().find(|(name, _)| name == distro) {
        return value.clone();
    }
    let value = query();
    entries().push((distro.to_owned(), value.clone()));
    value
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

/// 从 `command` 的 `WSLENV` 中去掉命令上没有设置的变量，使发行版只收到有值的变量。
///
/// `wsl.exe` 把列出而未设置的变量在发行版内设为空字符串，与未设置的含义不同：`GIT_INDEX_FILE`
/// 为空时 `git` 读取空的暂存区，干净的仓库显示为跟踪的文件全部删除。各处按用途列出可能传递的
/// 变量（[`GIT_ENV`] 等），启动程序时去掉本次没有设置的；只从发行版传回的项（`/w`）保留。命令
/// 的环境由 `Shell::command` 整体给出（先清空再逐项设置），因此只看命令上显式设置的值。
pub(crate) fn drop_unset(command: &mut Command) {
    let Some(list) = command
        .get_envs()
        .find(|(name, _)| *name == "WSLENV")
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
    else {
        return;
    };
    let set = |name: &str| command.get_envs().any(|(k, v)| k == name && v.is_some());
    let kept: Vec<&str> = list
        .split(':')
        .filter(|entry| {
            let (name, flags) = entry.split_once('/').unwrap_or((entry, ""));
            !name.is_empty() && (flags.contains('w') || set(name))
        })
        .collect();
    let kept = kept.join(":");
    if kept != list {
        command.env("WSLENV", kept);
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
    remembered(&LOGIN_PATHS, distro, || {
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
        process::run(&mut command, Vec::new(), QUERY_TIMEOUT, &Cancel::default())
            .ok()
            .and_then(|output| environment::path_in(&output.stdout))
    })
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

/// 在发行版内把 `from` 中的各项 `entries`（相对路径）复制到同一发行版中 `to` 的相同位置。
///
/// 返回复制的项数，`to` 中已有的项由调用方排除。Windows 无法读写发行版中文件的 Linux 权限，
/// 经文件共享复制会丢失可执行位，因此在发行版内复制。
///
/// # Errors
///
/// 某一项复制失败时返回该项的路径和原因；此前已复制的项保留。
pub(crate) fn copy_entries(
    from: &Location,
    to: &Location,
    entries: &[String],
    cancel: &Cancel,
) -> Result<usize, (String, String)> {
    let mut copied = 0;
    let mut rest = entries;
    while !rest.is_empty() {
        let mut size = 0;
        let fits = rest.iter().take_while(|entry| {
            size += entry.len() + 1;
            size <= COPY_ARGS_LIMIT
        });
        let (chunk, next) = rest.split_at(fits.count().max(1));
        let mut command = Command::new("wsl.exe");
        enter(&mut command, &from.distro);
        command
            .args([
                "--exec",
                "sh",
                "-c",
                COPY_ENTRIES,
                "sh",
                &from.path,
                &to.path,
            ])
            .args(chunk)
            .stderr(Stdio::piped());
        let failed = |done: usize, error: String| (chunk[done.min(chunk.len() - 1)].clone(), error);
        let output = process::run(&mut command, Vec::new(), COPY_TIMEOUT, cancel)
            .map_err(|e| failed(0, e.to_string()))?;
        let done = output.stdout.iter().filter(|&&b| b == 0).count();
        copied += done;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(failed(done, error));
        }
        rest = next;
    }
    Ok(copied)
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
        let mut text = format!("{}{}", self.prefix, self.distro);
        for part in linux.split('/').filter(|part| !part.is_empty()) {
            text.push('\\');
            text.push_str(part);
        }
        PathBuf::from(text)
    }
}

#[cfg(test)]
mod tests;
