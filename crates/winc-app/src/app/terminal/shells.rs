//! 新建终端时可选的 shell：「+」右侧的下拉菜单，只列出本机存在的 shell，默认 shell 排在最前。
//!
//! 首次启动终端时在后台线程检测：Windows 上列出 WSL 发行版需要启动 `wsl.exe`，不能占用界面线程。
//! 启动命令按空白分词，双引号括起的部分不拆开（[`command_words`]），含空白的路径加上双引号。

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

/// 把启动命令 `command` 按空白拆成程序和参数；双引号括起的部分（如含空格的路径）不拆开，
/// 引号去掉。
///
/// 反斜杠不作转义，Windows 路径中的 `\` 原样保留。
pub(super) fn command_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    // `started`：当前词已经开始，空的引号 `""` 也算一个词。
    let (mut quoted, mut started) = (false, false);
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if std::mem::take(&mut started) {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// 返回能写入启动命令的 `word`：含空白时加上双引号。
fn quoted(word: &str) -> String {
    if word.contains(char::is_whitespace) {
        format!("\"{word}\"")
    } else {
        word.to_owned()
    }
}

/// 返回命令 `command` 的程序名：去掉目录和 `.exe`。
fn program_name(command: &str) -> String {
    let words = command_words(command);
    let program = words.first().map_or("", String::as_str);
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    file.strip_suffix(".exe").unwrap_or(file).to_owned()
}

/// 返回 Unix 上的 shell：`login` 是默认 shell，`list` 是 `/etc/shells` 的内容。
#[cfg(any(not(windows), test))]
fn unix_shells(list: &str, login: Option<&str>, exists: impl Fn(&str) -> bool) -> Vec<ShellChoice> {
    let listed = list
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'));
    let listed: Vec<&str> = listed.collect();
    let choice = |path: &str| {
        let command = quoted(path);
        ShellChoice {
            name: program_name(&command),
            command,
        }
    };
    let mut shells: Vec<ShellChoice> = login
        .filter(|path| path.starts_with('/') && exists(path))
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

/// 返回 Windows 上的 shell：`has` 判断程序是否在 `PATH` 中，`git_bash` 是 Git Bash 的路径，
/// `distros` 是 WSL 发行版。
///
/// 前三个的顺序与辅助进程选默认 shell 的顺序相同：`pwsh`、`powershell`、`cmd`。Git Bash 以登录、
/// 交互方式启动（`--login -i`），读取 `/etc/profile` 和用户的配置。
#[cfg(any(windows, test))]
fn windows_shells(
    has: impl Fn(&str) -> bool,
    git_bash: Option<&str>,
    distros: &[String],
) -> Vec<ShellChoice> {
    let programs = ["pwsh", "powershell", "cmd"].into_iter();
    let programs = programs.filter(|name| has(&format!("{name}.exe")));
    let programs = programs.map(|name| ShellChoice {
        name: name.to_owned(),
        command: format!("{name}.exe"),
    });
    let git_bash = git_bash.map(|path| ShellChoice {
        name: "Git Bash".to_owned(),
        command: format!("{} --login -i", quoted(path)),
    });
    let distros = distros.iter().map(|distro| ShellChoice {
        name: format!("{distro} (WSL)"),
        command: format!("wsl.exe -d {}", quoted(distro)),
    });
    programs.chain(git_bash).chain(distros).collect()
}

/// Git for Windows 的默认安装目录，每项为（环境变量，其下的相对路径）：全局安装（64 位、
/// 32 位）、当前用户安装和 Scoop。
#[cfg(any(windows, test))]
const GIT_DIRS: [(&str, &str); 5] = [
    ("ProgramW6432", "Git"),
    ("ProgramFiles", "Git"),
    ("ProgramFiles(x86)", "Git"),
    ("LocalAppData", r"Programs\Git"),
    ("UserProfile", r"scoop\apps\git\current"),
];

/// 返回 Git Bash（Git for Windows 安装目录下的 `bin\bash.exe`）的路径；找不到时返回 `None`。
///
/// `path` 是 `PATH` 中的目录，`var` 读取环境变量，`exists` 判断文件是否存在。先查 `PATH` 中
/// 含 `git.exe` 的 `cmd` 目录（安装程序将 `<安装目录>\cmd` 加入 `PATH`），以找到自定义位置的
/// 安装；再查 [`GIT_DIRS`] 中的默认安装目录。
#[cfg(any(windows, test))]
fn git_bash(
    path: &[String],
    var: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let on_path = path.iter().filter_map(|dir| {
        let dir = dir.trim_end_matches('\\');
        let (parent, name) = dir.rsplit_once('\\')?;
        let git = name.eq_ignore_ascii_case("cmd") && exists(&format!(r"{dir}\git.exe"));
        git.then(|| parent.to_owned())
    });
    let defaults = GIT_DIRS
        .into_iter()
        .filter_map(|(key, dir)| Some(format!(r"{}\{dir}", var(key)?)));
    let mut bash = on_path
        .chain(defaults)
        .map(|dir| format!(r"{dir}\bin\bash.exe"));
    bash.find(|bash| exists(bash))
}

/// 解析 `wsl.exe -l -q` 的输出（UTF-16 小端）。
///
/// 不列出 `docker-desktop` 开头的内部发行版。
#[cfg(any(windows, test))]
fn parse_wsl_list(output: &[u8]) -> Vec<String> {
    let (units, _) = output.as_chunks::<2>();
    let units: Vec<u16> = units.iter().map(|&unit| u16::from_le_bytes(unit)).collect();
    let text = String::from_utf16_lossy(&units);
    let names = text
        .lines()
        .map(|line| line.trim_matches(['\u{feff}', '\0', ' ', '\r']));
    names
        .filter(|name| !name.is_empty() && !name.starts_with("docker-desktop"))
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
    let dirs: Vec<String> = dirs
        .iter()
        .map(|dir| dir.to_string_lossy().into_owned())
        .collect();
    let exists = |path: &str| std::path::Path::new(path).is_file();
    let git_bash = git_bash(&dirs, |key| std::env::var(key).ok(), exists);
    let output = std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let distros = match output {
        Ok(output) if output.status.success() => parse_wsl_list(&output.stdout),
        _ => Vec::new(),
    };
    windows_shells(has, git_bash.as_deref(), &distros)
}

/// 记录的 shell 是否仍然可用：绝对路径的程序已卸载时，恢复现场改用默认 shell，避免启动失败。
pub(super) fn still_usable(command: &str) -> bool {
    let words = command_words(command);
    let program = words.first().map_or("", String::as_str);
    !program.starts_with('/') || cfg!(windows) || std::path::Path::new(program).is_file()
}

impl App {
    /// 第一次启动终端或打开设置时在后台检测本机的 shell，结果由事件
    /// [`UserEvent::TerminalShells`] 送回。
    pub(in crate::app) fn detect_terminal_shells(&mut self) {
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

    /// 返回配置的 `terminal.shell`，未设置时返回 `None`。
    ///
    /// 命令与检测到的某个 shell 相同时沿用其名称（如「Git Bash」）。
    pub(crate) fn configured_shell(&self) -> Option<ShellChoice> {
        let command = self.config.terminal_shell.as_deref()?;
        let command = command.trim();
        if command.is_empty() {
            return None;
        }
        let known = self
            .terminal
            .shells
            .iter()
            .find(|shell| shell.command == command);
        Some(ShellChoice {
            name: known.map_or_else(|| program_name(command), |shell| shell.name.clone()),
            command: command.to_owned(),
        })
    }

    /// 菜单里的 shell：设置了 `terminal.shell` 时它是默认的，排在最前面。
    pub(crate) fn terminal_shell_list(&self) -> Vec<ShellChoice> {
        let first = self.configured_shell();
        let name = first.as_ref().map(|shell| shell.name.clone());
        let others = (self.terminal.shells.iter())
            .filter(|shell| Some(&shell.name) != name.as_ref())
            .cloned();
        first.into_iter().chain(others).collect()
    }

    /// 打开「+」右边的 shell 菜单。
    pub(in crate::app) fn open_terminal_shell_menu(&mut self) {
        let menu = (self.hits.terminal.iter()).find(|(_, hit)| *hit == HeaderHit::ShellMenu);
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
