//! 命令行参数的解析与用法说明，入口是 [`parse`] 与 [`usage`]。
//!
//! 不带参数时恢复上次的现场；给出目录或文件时打开它们，多个文件以第一个文件所在的目录作为项目。
//! 命令行上的 `文件:行[:列]` 跳到指定位置，行号和列号从 1 起。`-w`、`--wait` 在给出的文件的标签
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
            Some(self.targets.iter().any(|target| target.path.is_dir()))
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
        parsed.targets.push(target(arg));
    }
    if let Some(distro) = &distro {
        for target in &mut parsed.targets {
            let path = std::mem::take(&mut target.path);
            target.path = winc_editor::wsl::from_distro(distro, path);
        }
    }
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

/// 把一个命令行参数解析为 [`Target`]，识别 `路径:行:列` 和 `路径:行`。
///
/// 仅当整个参数作为路径不存在、末尾一段或两段是正整数且前面的路径非空时才拆分，因此名字本身
/// 带冒号和数字的已有文件仍按原名打开。
fn target(arg: OsString) -> Target {
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
        return Target {
            path: PathBuf::from(path),
            line: Some(line - 1),
            col: Some(col - 1),
        };
    }
    match text.rsplit_once(':') {
        Some((path, line)) if !path.is_empty() && number(line).is_some() => Target {
            path: PathBuf::from(path),
            line: number(line).map(|n| n - 1),
            col: None,
        },
        _ => whole,
    }
}

#[cfg(test)]
mod tests;
