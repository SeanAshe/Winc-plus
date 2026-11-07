//! 新建终端时可选的 shell：「+」右侧的下拉菜单，只列出本机存在的 shell，默认 shell 排在最前。
//!
//! 首次启动终端时在后台线程检测：Windows 上列出 WSL 发行版需要启动 `wsl.exe`，不能占用界面线程。
//! 启动命令按空白分词（[`super::helper`] 的 `launch`），路径含空白的 shell 不列出。

use super::HeaderHit;
use crate::app::input::context_menu::{MenuButton, MenuTarget, menu_below};
use crate::app::{App, UserEvent};

/// 列出的 Unix shell，按这个顺序排在默认 shell 后面。
///
/// `sh`、`csh` 等虽然登记在 `/etc/shells`，但很少用作交互 shell，需要时在 `terminal.shell` 中设置。
#[cfg(any(not(windows), test))]
const UNIX_SHELLS: &[&str] = &["zsh", "bash", "fish", "nu", "pwsh"];

/// `/etc/shells` 之外还要找的目录：用包管理器安装的 shell 不一定登记在那里。
#[cfg(any(not(windows), test))]
const UNIX_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

/// 菜单里的一个 shell。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellChoice {
    /// 显示的名字。
    pub name: String,
    /// 启动命令，空白分隔程序和参数。
    pub command: String,
}

/// 返回命令 `command` 的程序名：去掉目录和 `.exe`。
fn program_name(command: &str) -> &str {
    let program = command.split_whitespace().next().unwrap_or_default();
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    file.strip_suffix(".exe").unwrap_or(file)
}

/// 返回 Unix 上的 shell：`login` 是默认 shell，`list` 是 `/etc/shells` 的内容。
#[cfg(any(not(windows), test))]
fn unix_shells(list: &str, login: Option<&str>, exists: impl Fn(&str) -> bool) -> Vec<ShellChoice> {
    let usable = |path: &str| path.starts_with('/') && !path.contains(char::is_whitespace);
    let listed = list
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && usable(line));
    let listed: Vec<&str> = listed.collect();
    let choice = |command: &str| ShellChoice {
        name: program_name(command).to_owned(),
        command: command.to_owned(),
    };
    let mut shells: Vec<ShellChoice> = login
        .filter(|path| usable(path) && exists(path))
        .map(choice)
        .into_iter()
        .collect();
    for name in UNIX_SHELLS {
        if shells.iter().any(|shell| shell.name == *name) {
            continue;
        }
        let extra = UNIX_DIRS.iter().map(|dir| format!("{dir}/{name}"));
        let found = (listed.iter().copied())
            .filter(|path| program_name(path) == *name)
            .map(str::to_owned)
            .chain(extra)
            .find(|path| exists(path));
        shells.extend(found.as_deref().map(choice));
    }
    shells
}

/// 返回 Windows 上的 shell：`has` 判断程序在不在 `PATH` 里，`distros` 是 WSL 发行版。
///
/// 顺序与辅助进程选默认 shell 的顺序相同：`pwsh`、`powershell`、`cmd`。
#[cfg(any(windows, test))]
fn windows_shells(has: impl Fn(&str) -> bool, distros: &[String]) -> Vec<ShellChoice> {
    let programs = ["pwsh", "powershell", "cmd"].into_iter();
    let programs = programs.filter(|name| has(&format!("{name}.exe")));
    let programs = programs.map(|name| ShellChoice {
        name: name.to_owned(),
        command: format!("{name}.exe"),
    });
    let distros = distros.iter().map(|distro| ShellChoice {
        name: format!("{distro} (WSL)"),
        command: format!("wsl.exe -d {distro}"),
    });
    programs.chain(distros).collect()
}

/// 解析 `wsl.exe -l -q` 的输出（UTF-16 小端）。
///
/// 不列出 Docker Desktop 内部使用的发行版，以及名字含空白、无法写入启动命令的发行版。
#[cfg(any(windows, test))]
fn parse_wsl_list(output: &[u8]) -> Vec<String> {
    let (units, _) = output.as_chunks::<2>();
    let units: Vec<u16> = units.iter().map(|&unit| u16::from_le_bytes(unit)).collect();
    let text = String::from_utf16_lossy(&units);
    let names = text
        .lines()
        .map(|line| line.trim_matches(['\u{feff}', '\0', ' ', '\r']));
    names
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
        .filter(|name| !name.starts_with("docker-desktop"))
        .map(str::to_owned)
        .collect()
}

/// 检测本机的 shell。
#[cfg(not(windows))]
fn detect() -> Vec<ShellChoice> {
    let list = std::fs::read_to_string("/etc/shells").unwrap_or_default();
    let login = std::env::var("SHELL").ok();
    let exists = |path: &str| std::path::Path::new(path).is_file();
    unix_shells(&list, login.as_deref(), exists)
}

/// 检测本机的 shell。
#[cfg(windows)]
fn detect() -> Vec<ShellChoice> {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`：图形程序启动命令行程序时不弹出控制台窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<_> = std::env::split_paths(&path).collect();
    let has = |name: &str| dirs.iter().any(|dir| dir.join(name).is_file());
    let output = std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let distros = match output {
        Ok(output) if output.status.success() => parse_wsl_list(&output.stdout),
        _ => Vec::new(),
    };
    windows_shells(has, &distros)
}

/// 记录的 shell 是否仍然可用：绝对路径的程序已卸载时，恢复现场改用默认 shell，避免启动失败。
pub(super) fn still_usable(command: &str) -> bool {
    let program = command.split_whitespace().next().unwrap_or_default();
    !program.starts_with('/') || cfg!(windows) || std::path::Path::new(program).is_file()
}

impl App {
    /// 第一次启动终端时在后台检测本机的 shell，结果由事件 [`UserEvent::TerminalShells`] 送回。
    pub(super) fn detect_terminal_shells(&mut self) {
        if std::mem::replace(&mut self.terminal.shells_detected, true) {
            return;
        }
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::TerminalShells(detect()));
        });
    }

    /// 菜单里的 shell：设置了 `terminal.shell` 时它是默认的，排在最前面。
    pub(crate) fn terminal_shell_list(&self) -> Vec<ShellChoice> {
        let configured = self.config.terminal_shell.as_deref();
        let configured = configured.filter(|command| !command.trim().is_empty());
        let first = configured.map(|command| ShellChoice {
            name: program_name(command).to_owned(),
            command: command.to_owned(),
        });
        let name = first.as_ref().map(|shell| shell.name.clone());
        let others = (self.terminal.shells.iter())
            .filter(|shell| Some(&shell.name) != name.as_ref())
            .cloned();
        first.into_iter().chain(others).collect()
    }

    /// 打开「+」右边的 shell 菜单。
    pub(in crate::app) fn open_terminal_shell_menu(&mut self) {
        let menu = (self.terminal.hits.iter()).find(|(_, hit)| *hit == HeaderHit::ShellMenu);
        let Some(&(menu, _)) = menu else {
            return;
        };
        let at = menu_below(menu);
        self.open_button_menu(MenuButton::TerminalShells, MenuTarget::TerminalShells, at);
    }

    /// 用菜单里第 `index` 个 shell 新建一个终端标签。
    pub(in crate::app) fn new_terminal_with_shell(&mut self, index: usize) {
        if let Some(shell) = self.terminal_shell_list().into_iter().nth(index) {
            self.new_terminal_tab_with(Vec::new(), Some(shell.command));
        }
    }
}

#[cfg(test)]
mod tests;
