//! 命令行参数的解析与用法说明，入口是 [`parse`] 与 [`usage`]。
//!
//! 不带参数时恢复上次的现场；给出目录或文件时打开它们，多个文件以第一个文件所在的目录作为项目。
//! 命令行上的 `文件:行[:列]` 跳到指定位置，行号和列号从 1 起。`ssh://主机/路径` 打开远端主机上
//! 的目录和文件（见 [`remote_open`]）。`-w`、`--wait` 在给出的文件的标签
//! 全部关闭后退出，供 `git` 等程序作为编辑器调用；`-` 读取标准输入；`-n`、`--new-instance` 不交给
//! 正在运行的 winc，由当前进程新建实例。

use std::ffi::OsString;
use std::path::PathBuf;

use winc_i18n::t;

/// 命令行上要打开的一项目录或文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 目录或文件的路径。
    pub path: PathBuf,
    /// `文件:行[:列]` 中的行号，从 0 起，由命令行上的 1 起行号减 1 得到；未指定时为 `None`。
    pub line: Option<usize>,
    /// `文件:行:列` 中的列号，从 0 起，由命令行上的 1 起列号减 1 得到；未指定时为 `None`。
    pub col: Option<usize>,
}

/// 解析后的命令行参数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// 要打开的目录和文件，顺序与命令行一致。
    pub targets: Vec<Target>,
    /// `-w`、`--wait`：所有给出的文件的标签关闭后才退出。
    pub wait: bool,
    /// 是否读取标准输入（`-`）。
    pub stdin: bool,
    /// 是否只显示用法（`-h`、`--help`）。
    pub help: bool,
    /// 是否只显示版本号（`-v`、`--version`）。
    pub version: bool,
    /// `-n`、`--new-instance`：不交给正在运行的 winc，由当前进程新建实例；也不接收之后启动的
    /// winc 交来的目录和文件。
    pub new_instance: bool,
    /// `--register-file-types`：安装程序用来登记文件类型，登记后退出，不打开窗口。
    pub register_file_types: bool,
    /// `--unregister-file-types`：卸载程序用来去掉这些文件类型，完成后退出。
    pub unregister_file_types: bool,
    /// 由命令行启动脚本（`bin/winc`、`bin/winc.cmd`）启动：窗口进程需要转到后台，命令才能立即返回
    /// （见 [`crate::shell_command::detach`]）。
    pub launcher: bool,
}

impl Args {
    /// 是否为一次性的临时打开（`--wait` 或读取标准输入）。
    ///
    /// 临时打开不恢复上次的现场，也不修改已保存的现场。
    pub fn one_off(&self) -> bool {
        self.wait || self.stdin
    }

    /// 这次启动要不要展开侧栏。
    ///
    /// 没有目标时返回 `None`：恢复上次的目录时展开，开始页收起。只打开文件时收起，
    /// 打开了目录时展开。
    pub fn launch_sidebar(&self) -> Option<bool> {
        if self.targets.is_empty() {
            None
        } else {
            Some(
                self.targets
                    .iter()
                    .any(|target| winc_backend::fs::is_dir(&target.path)),
            )
        }
    }
}

/// 解析命令行参数（不含程序名）。
///
/// `-h`、`--help`、`-v`、`--version` 立即返回，不检查其后的参数。
///
/// # Errors
///
/// 出现无法识别的选项，或使用 `--wait` 但没有给出文件、也没有读取标准输入时，返回面向用户的
/// 错误说明（含用法）。
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut options = true;
    let mut distro = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if options {
            match arg.to_str() {
                Some("--") => {
                    options = false;
                    continue;
                }
                Some("-w" | "--wait") => {
                    parsed.wait = true;
                    continue;
                }
                Some("-n" | "--new-instance") => {
                    parsed.new_instance = true;
                    continue;
                }
                Some("--register-file-types") => {
                    parsed.register_file_types = true;
                    continue;
                }
                Some("--unregister-file-types") => {
                    parsed.unregister_file_types = true;
                    continue;
                }
                // 启动脚本传入的标记，不列入用法说明。
                Some(LAUNCHER_FLAG) => {
                    parsed.launcher = true;
                    continue;
                }
                // WSL 中的启动脚本传入发行版名称，不列入用法说明。
                Some("--wsl") => {
                    let Some(name) = args.next() else {
                        let message = t!("cmd-cli-wsl-needs-name");
                        return Err(format!("{message}\n\n{}", usage()));
                    };
                    distro = Some(name.to_string_lossy().into_owned());
                    continue;
                }
                Some("-") => {
                    parsed.stdin = true;
                    continue;
                }
                Some("-h" | "--help") => {
                    parsed.help = true;
                    return Ok(parsed);
                }
                Some("-v" | "--version") => {
                    parsed.version = true;
                    return Ok(parsed);
                }
                Some(flag) if flag.starts_with('-') => {
                    let message = t!("cmd-cli-unknown-option", flag = flag);
                    return Err(format!("{message}\n\n{}", usage()));
                }
                _ => {}
            }
        }
        // 系统打开 `winc://` 链接时把整个链接作为参数传进来（Windows、Linux）。
        let text = arg.to_string_lossy();
        let scheme = text.split_once("://").map(|(scheme, _)| scheme);
        if scheme.is_some_and(|s| s.eq_ignore_ascii_case(LINK_SCHEME)) {
            let Some(target) = link(&text) else {
                return Err(t!("cmd-cli-bad-link", link = text.as_ref()).to_owned());
            };
            parsed.targets.push(target);
            continue;
        }
        if scheme.is_some_and(|s| s.eq_ignore_ascii_case(SSH_SCHEME)) {
            let Some(target) = ssh(&text) else {
                return Err(t!("cmd-cli-bad-ssh", link = text.as_ref()).to_owned());
            };
            parsed.targets.push(target);
            continue;
        }
        parsed.targets.push(target(arg));
    }
    if let Some(distro) = &distro {
        let local = parsed.targets.iter_mut();
        for target in local.filter(|t| !winc_backend::is_remote(&t.path)) {
            let path = std::mem::take(&mut target.path);
            target.path = winc_editor::wsl::from_distro(distro, path);
        }
    }
    check_remote(&parsed)?;
    if parsed.wait && parsed.targets.is_empty() && !parsed.stdin {
        return Err(format!("{}\n\n{}", t!("cmd-cli-wait-needs-file"), usage()));
    }
    Ok(parsed)
}

/// 命令行启动脚本传给程序的标记，见 [`Args::launcher`]。
pub const LAUNCHER_FLAG: &str = "--launcher";

/// 返回去掉启动脚本标记后的参数（`--` 之后的同名参数是文件名，保留）。
///
/// 窗口进程转到后台时使用：新进程收到这些参数，不会再次转移。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn without_launcher_flag(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut options = true;
    args.into_iter()
        .filter(|arg| {
            if arg == "--" {
                options = false;
            }
            !(options && arg == LAUNCHER_FLAG)
        })
        .collect()
}

/// 返回命令行用法说明。
pub fn usage() -> String {
    t!("cmd-cli-usage").to_owned()
}

/// 路径 `path` 是否以分隔符结尾（`新目录/`）：这样写的是目录，不存在时不新建同名的文件。
pub(crate) fn names_folder(path: &std::path::Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    text.ends_with('/') || (cfg!(windows) && text.ends_with('\\'))
}

/// 打开文件或目录的链接的前缀，链接为 `winc://file/<绝对路径>[:行[:列]]`。
pub const LINK_SCHEME: &str = "winc";

/// 把 `winc://file/<绝对路径>[:行[:列]]` 链接解析为要打开的 [`Target`]，行号和列号从 1 起。
///
/// 路径按百分号编码解码；查询和片段（`?`、`#` 之后）忽略。Windows 路径写作
/// `winc://file/C:/目录/文件`。不是这种链接、路径不是绝对路径、是网络路径或解码后不是 UTF-8 时
/// 返回 `None`。
pub fn link(url: &str) -> Option<Target> {
    let (scheme, rest) = url.split_once("://")?;
    let rest = rest.strip_prefix("file")?;
    if !scheme.eq_ignore_ascii_case(LINK_SCHEME) || !rest.starts_with('/') {
        return None;
    }
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let path = percent_decode(&rest[..end])?;
    // `/C:/…` 是 Windows 盘符路径，去掉开头的斜杠。
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_owned(),
        _ => path,
    };
    // 链接可以来自网页：不接受以两个分隔符开头的网络路径和设备路径（`//主机/共享`、
    // `\\?\…`），在 Windows 上查看它们是否存在就会向那台主机发起连接和身份验证。
    if winc_backend::names_host(&path) {
        return None;
    }
    let target = target(OsString::from(path));
    target.path.is_absolute().then_some(target)
}

/// 解码百分号编码（`%20` 为空格）；编码不完整或结果不是 UTF-8 时返回 `None`。
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(bytes).ok()
}

/// 远端主机上的目录和文件的参数前缀，参数为 `ssh://[用户@]主机[:端口]/绝对路径[:行[:列]]`。
pub const SSH_SCHEME: &str = "ssh";

/// 把 `ssh://[用户@]主机[:端口]/绝对路径[:行[:列]]` 解析为 [`Target`]，路径是该主机的虚拟路径，
/// 行号和列号从 1 起。
///
/// 主机按 [`winc_backend::Target`] 校验，可以是 `~/.ssh/config` 中的别名；IPv6 地址写在方括号
/// 中，`%` 写作 `%25`（RFC 6874）。路径末尾的 `:正整数` 总是作为行号和列号，路径中的 `:` 写作
/// `%3A`；之后路径按百分号编码解码，查询和片段忽略。不符合时返回 `None`。
fn ssh(url: &str) -> Option<Target> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case(SSH_SCHEME) {
        return None;
    }
    let slash = rest.find('/')?;
    let host = winc_backend::Target::new(&rest[..slash].replace("%25", "%"))?;
    let rest = &rest[slash..];
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let (path, line, col) = position(rest).unwrap_or((rest, None, None));
    let path = percent_decode(path)?;
    Some(Target {
        path: winc_backend::virtual_path(&host, path.as_bytes()),
        line,
        col,
    })
}

/// 命令行上 `ssh://` 参数要打开的远端项目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOpen {
    /// 连接目标。
    pub target: winc_backend::Target,
    /// 作为项目打开的目录（虚拟路径）。
    pub folder: PathBuf,
    /// 在项目中打开的文件，带行号时跳到该位置。
    pub files: Vec<Target>,
}

/// 返回命令行目标中的远端项目；没有 `ssh://` 参数时返回 `None`。
///
/// 远端路径在连接之前分不出目录和文件，按写法区分：带行号的、位于另一个参数之下的是文件，
/// 其余的是目录（只有一个）。只给出文件时，第一个文件所在的目录作为项目。
pub fn remote_open(targets: &[Target]) -> Option<RemoteOpen> {
    let target = winc_backend::locate(&targets.first()?.path)?.target;
    let folder = (targets.iter())
        .find(|t| !remote_file(t, targets))
        .map(|t| t.path.clone())
        .or_else(|| Some(targets.first()?.path.parent()?.to_path_buf()))?;
    let files = targets.iter().filter(|t| remote_file(t, targets));
    Some(RemoteOpen {
        target,
        folder,
        files: files.cloned().collect(),
    })
}

/// `ssh://` 参数 `target` 是否按文件打开，见 [`remote_open`]。
fn remote_file(target: &Target, targets: &[Target]) -> bool {
    target.line.is_some()
        || (targets.iter()).any(|t| t.path != target.path && target.path.starts_with(&t.path))
}

/// 检查 `ssh://` 参数：都指向同一个连接目标，不与本机路径、`--wait` 和 `-` 一起使用，其中至多
/// 一个是目录。
fn check_remote(parsed: &Args) -> Result<(), String> {
    let mut remote = parsed.targets.iter().map(|t| winc_backend::locate(&t.path));
    let Some(first) = remote.next().flatten() else {
        return match remote.flatten().next() {
            Some(_) => Err(t!("cmd-cli-ssh-mixed").to_owned()),
            None => Ok(()),
        };
    };
    if remote.any(|at| at.is_none_or(|at| at.target != first.target)) {
        return Err(t!("cmd-cli-ssh-mixed").to_owned());
    }
    if parsed.wait || parsed.stdin {
        return Err(t!("cmd-cli-ssh-one-off").to_owned());
    }
    // 同一个目录写了几次仍是一个目录。
    let targets = &parsed.targets;
    let mut folders = (targets.iter()).filter(|t| !remote_file(t, targets));
    if let Some(folder) = folders.next()
        && folders.any(|t| t.path != folder.path)
    {
        return Err(t!("cmd-cli-ssh-folders").to_owned());
    }
    Ok(())
}

/// 返回把 `target` 交给另一个 winc 进程时的参数：本机的是绝对路径，远端的是 `ssh://` 写法，
/// 有行号时加上 `:行[:列]`（从 1 起）。
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn argument(target: &Target) -> OsString {
    let mut arg = match winc_backend::locate(&target.path) {
        Some(at) => {
            let host = at.target.as_str();
            OsString::from(format!("ssh://{host}{}", percent_encode(&at.path)))
        }
        None => OsString::from(crate::instance::absolute(&target.path)),
    };
    if let Some(line) = target.line {
        arg.push(format!(":{}", line + 1));
        if let Some(col) = target.col {
            arg.push(format!(":{}", col + 1));
        }
    }
    arg
}

/// 把路径编码为 URL 的路径部分：保留字母、数字、`/` 和 `-._~`，其余字节写作 `%XX`。
fn percent_encode(path: &[u8]) -> String {
    let mut out = String::with_capacity(path.len());
    for &byte in path {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 拆出 `路径:行:列` 或 `路径:行` 末尾的行号和列号，返回时减 1 成为从 0 起；末尾不是正整数或
/// 前面的路径为空时返回 `None`。
fn position(text: &str) -> Option<(&str, Option<usize>, Option<usize>)> {
    let number = |s: &str| -> Option<usize> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<usize>().ok())
            .flatten()
            .filter(|&n| n > 0)
    };
    // 先试 `路径:行:列`，再试 `路径:行`。
    if let Some((rest, col)) = text.rsplit_once(':')
        && let Some((path, line)) = rest.rsplit_once(':')
        && let (Some(line), Some(col)) = (number(line), number(col))
        && !path.is_empty()
    {
        return Some((path, Some(line - 1), Some(col - 1)));
    }
    let (path, line) = text.rsplit_once(':')?;
    let line = number(line).filter(|_| !path.is_empty())?;
    Some((path, Some(line - 1), None))
}

/// 把一个命令行参数解析为 [`Target`]，识别 `路径:行:列` 和 `路径:行`。
///
/// 仅当整个参数作为路径不存在、末尾一段或两段是正整数且前面的路径非空时才拆分，因此名字本身
/// 带冒号和数字的已有文件仍按原名打开。
#[allow(
    clippy::disallowed_methods,
    reason = "只访问本机：命令行参数是本机路径"
)]
fn target(arg: OsString) -> Target {
    let arg = match arg.to_str().and_then(unquoted).filter(|_| cfg!(windows)) {
        Some(path) => OsString::from(path),
        None => arg,
    };
    let whole = Target {
        path: PathBuf::from(&arg),
        line: None,
        col: None,
    };
    let Some(text) = arg.to_str() else {
        return whole;
    };
    if whole.path.exists() {
        return whole;
    }
    match position(text) {
        Some((path, line, col)) => Target {
            path: PathBuf::from(path),
            line,
            col,
        },
        None => whole,
    }
}

/// 返回被 Windows 的命令行规则吃掉结尾反斜杠的路径 `text` 原来的写法；不是这种情况时返回
/// `None`。
///
/// 资源管理器把 `%V` 换成路径放在引号里传入；路径是盘符根目录时写成 `"C:\"`，其中的 `\"`
/// 按规则是一个引号，程序收到的是 `C:"`。引号不能出现在 Windows 的文件名里，结尾有引号的
/// 只能是这种情况。
fn unquoted(text: &str) -> Option<String> {
    let path = text.strip_suffix('"')?;
    (!path.is_empty() && !path.contains('"')).then(|| format!("{path}\\"))
}

#[cfg(test)]
mod tests;
