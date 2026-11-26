//! 项目的运行配置（`.winc/run.json`）：在终端面板的标签栏上选择、运行、重新运行和停止，输出显示在
//! 终端标签中。
//!
//! 每个配置最多对应一个标签：再次运行时新标签替换旧标签。程序退出后标签保留，便于查看输出；运行
//! 配置的标签不随现场恢复。没有配置文件时列出从项目文件识别出的命令，不写文件。配置和项目文件都来自
//! 仓库，内容不可信：每个配置第一次运行前和内容改变后都要用户确认（[`RunConfig::approval`]），
//! 只由用户的操作触发运行。

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use winc_config::{
    RunConfig, affects_detected_configs, create_run_configs, detected_run_configs,
    load_run_configs, run_configs_path,
};
use winc_i18n::t;
use winc_pty::protocol::ToHelper;

use super::helper::{Integration, Launch, default_shell_applies, environment, launch, os_bytes};
use super::{HeaderHit, TermTab};
use crate::app::NoticeKind;
use crate::app::input::context_menu::menu_below;
use crate::app::input::context_menu::{MenuButton, MenuTarget};
use crate::app::lifecycle::alert::AlertButton;
use crate::app::{App, UserEvent};

/// 传给辅助进程的环境变量，每项是（键，值）。
type Env = Vec<(Vec<u8>, Vec<u8>)>;

/// 终端面板标签栏上运行配置的三个按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunButton {
    /// 配置名和下拉箭头。
    Menu,
    /// 运行；运行中是重新运行。
    Start,
    /// 停止。
    Stop,
}

/// 当前目录的运行配置。
pub(crate) struct RunConfigs {
    /// 文件中的配置。
    pub list: Vec<RunConfig>,
    /// 用户选中的配置名；为 `None` 或已不存在时使用第一个。
    pub selected: Option<String>,
    /// 配置是从项目文件识别出来的，不是写在配置文件里的。
    pub detected: bool,
    /// 上一次报告的问题：文件没有变化时不重复提示。
    problems: Vec<String>,
    /// 向后台发出过几次读取；只采用最后一次的结果。
    loads: u64,
    /// 后台读好的配置的通道：发送端交给工作线程，接收端由界面线程轮询。
    results: (mpsc::Sender<Loaded>, mpsc::Receiver<Loaded>),
    /// 等配置读好后在界面线程上继续做的事（运行、打开菜单）。
    after: Vec<AfterLoad>,
}

impl Default for RunConfigs {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            selected: None,
            detected: false,
            problems: Vec::new(),
            loads: 0,
            results: mpsc::channel(),
            after: Vec::new(),
        }
    }
}

/// 运行配置读好后在界面线程上继续做的事。
type AfterLoad = Box<dyn FnOnce(&mut App)>;

/// 后台读好的运行配置。
struct Loaded {
    /// 第几次读取，和读的是哪个目录的。
    load: u64,
    root: PathBuf,
    list: Vec<RunConfig>,
    problems: Vec<String>,
    detected: bool,
}

impl RunConfigs {
    /// 当前选中的配置。
    pub fn current(&self) -> Option<&RunConfig> {
        let name = self.selected.as_deref();
        (self.list.iter().find(|c| Some(c.name.as_str()) == name)).or_else(|| self.list.first())
    }
}

/// 等辅助进程准备好、知道面板尺寸以后要启动的配置。
pub(super) struct RunStart {
    pub(super) config: RunConfig,
    pub(super) launch: Launch,
    pub(super) env: Env,
    /// 写在输出之前的命令行。
    pub(super) banner: Vec<u8>,
    /// 启动后替换的旧标签（编号）。
    pub(super) replaces: Option<u16>,
}

/// 运行命令行用的 shell 属于哪一类：决定启动参数和引号规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
    /// `wsl.exe`：命令在发行版内由用户的 shell 执行。
    Wsl,
}

/// 按程序名判断 shell 的类别，不认识的按 POSIX shell 处理。
fn shell_kind(program: &[u8]) -> ShellKind {
    let program = String::from_utf8_lossy(program).to_ascii_lowercase();
    let name = program.rsplit(['/', '\\']).next().unwrap_or_default();
    match name.strip_suffix(".exe").unwrap_or(name) {
        "pwsh" | "powershell" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        "wsl" => ShellKind::Wsl,
        _ => ShellKind::Posix,
    }
}

/// 给参数加上 `kind` 对应的 shell 的引号，使它按原样传给程序。
fn quote(arg: &str, kind: ShellKind) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c);
    if !arg.is_empty() && arg.chars().all(plain) {
        return arg.to_owned();
    }
    match kind {
        ShellKind::Posix | ShellKind::Wsl => format!("'{}'", arg.replace('\'', r"'\''")),
        ShellKind::PowerShell => format!("'{}'", arg.replace('\'', "''")),
        ShellKind::Cmd => format!("\"{}\"", arg.replace('"', "\"\"")),
    }
}

/// 拼出交给 shell 的命令行：命令加上加了引号的参数。
fn command_line(config: &RunConfig, kind: ShellKind) -> String {
    let mut line = config.command.clone();
    for arg in &config.args {
        line.push(' ');
        line.push_str(&quote(arg, kind));
    }
    line
}

/// 返回启动方式 `launch` 使用的 shell 的类别。
fn launch_kind(launch: &Launch, windows: bool) -> ShellKind {
    match &launch.program {
        Some(program) => shell_kind(program),
        None if windows => ShellKind::PowerShell,
        None => ShellKind::Posix,
    }
}

/// 返回写在输出之前的一行：淡色的命令行，工作目录不是打开的目录时写在后面。
///
/// 这一行带 shell 集成的命令边界标记（OSC 133 `A`、`B`，行尾 `C`），与终端里输入的命令一样
/// 成为一个命令块：滚动时命令行吸顶，程序退出后由辅助进程写入结束标记和退出码。配置来自项目
/// 仓库，控制字符换成空格，不能借此改写终端的内容。
fn banner(config: &RunConfig, kind: ShellKind) -> Vec<u8> {
    let mut line = format!("▶ {}", command_line(config, kind));
    if let Some(cwd) = config.cwd.as_deref().filter(|cwd| *cwd != ".") {
        line.push_str(&format!("  ({cwd})"));
    }
    let line: String = (line.chars())
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!("\x1b]133;A\x07\x1b]133;B\x07\x1b[2m{line}\x1b[0m\r\n\x1b]133;C\x07").into_bytes()
}

/// 返回运行 `config` 的启动方式和完整的环境变量；`env` 是继承的环境。
///
/// 命令由用户的 shell 执行，和在终端里输入一样可以使用别名、管道和配置文件设置的 `PATH`。环境变量
/// 不经 `wsl.exe` 传递，在发行版内的命令行中设置。
fn run_launch(
    config: &RunConfig,
    root: &Path,
    shell: Option<&str>,
    windows: bool,
    mut env: Env,
) -> (Launch, Env) {
    let dir = config
        .cwd
        .as_ref()
        .map_or(root.to_path_buf(), |cwd| root.join(cwd));
    let dir = os_bytes(dir.as_os_str());
    // 默认 shell 是本机的命令：目录在 WSL 发行版中时命令在发行版内运行。
    let shell = shell.filter(|_| default_shell_applies(&dir, windows));
    let mut launch = launch(shell, dir, windows, Integration::Off);
    let kind = launch_kind(&launch, windows);
    let mut line = command_line(config, kind);
    let args: Vec<String> = match kind {
        ShellKind::Posix => vec!["-i".into(), "-c".into(), line],
        ShellKind::PowerShell => vec!["-NoLogo".into(), "-Command".into(), line],
        ShellKind::Cmd => vec!["/C".into(), line],
        ShellKind::Wsl => {
            for (key, value) in config.env.iter().rev() {
                line.insert_str(0, &format!("export {key}={}; ", quote(value, kind)));
            }
            let script = r#"exec "${SHELL:-/bin/sh}" -lic "$1""#;
            let args = ["--exec", "sh", "-c", script, "sh"];
            args.into_iter().map(str::to_owned).chain([line]).collect()
        }
    };
    launch.args.extend(args.into_iter().map(String::into_bytes));
    if kind != ShellKind::Wsl {
        for (key, value) in &config.env {
            env.retain(|(old, _)| {
                if windows {
                    !old.eq_ignore_ascii_case(key.as_bytes())
                } else {
                    old != key.as_bytes()
                }
            });
            env.push((key.clone().into_bytes(), value.clone().into_bytes()));
        }
    }
    (launch, env)
}

/// 记录用户同意运行 `config` 时使用的键：同一个文件中的各配置分别记录。
///
/// 名字接在配置文件的路径后面，不用 `Path::join`：名字写成绝对路径时它会取代整个路径，键就
/// 成了别的目录的。
fn trust_key(root: &Path, config: &RunConfig) -> PathBuf {
    let mut key = run_configs_path(root).into_os_string();
    key.push(std::path::MAIN_SEPARATOR_STR);
    key.push(&config.name);
    PathBuf::from(key)
}

impl App {
    /// 在后台重新读取当前目录的运行配置；终端模块关着或没有打开目录时清空。
    pub(in crate::app) fn reload_run_configs(&mut self) {
        self.reload_run_configs_then(|_| {});
    }

    /// 同 [`App::reload_run_configs`]，配置读好换上后在界面线程上调用 `after`；期间换了目录时
    /// 不调用。
    fn reload_run_configs_then(&mut self, after: impl FnOnce(&mut App) + 'static) {
        self.run.loads += 1;
        self.run.after.push(Box::new(after));
        let load = self.run.loads;
        let root = self.editor.root().filter(|_| self.terminal_enabled());
        let Some(root) = root.map(Path::to_path_buf) else {
            self.run_configs_loaded(Vec::new(), Vec::new(), false);
            return;
        };
        let sender = self.run.results.0.clone();
        self.spawn_then(UserEvent::RunConfigsLoaded, move || {
            let (mut list, problems) = load_run_configs(&root);
            // 配置文件优先；没有文件时列出识别到的命令。文件存在但为空时尊重用户的选择。
            let detected = !winc_backend::fs::exists(run_configs_path(&root));
            if detected {
                list = detected_run_configs(&root);
            }
            let _ = sender.send(Loaded {
                load,
                root,
                list,
                problems,
                detected,
            });
        });
    }

    /// 收取后台读好的运行配置：只采用最后一次读取的，期间换了目录的丢弃。
    pub(in crate::app) fn poll_run_configs(&mut self) {
        while let Ok(loaded) = self.run.results.1.try_recv() {
            if loaded.load != self.run.loads {
                continue;
            }
            if self.editor.root() == Some(loaded.root.as_path()) {
                self.run_configs_loaded(loaded.list, loaded.problems, loaded.detected);
            } else {
                self.run.after.clear();
            }
        }
    }

    /// 换上读好的运行配置，报告其中新出现的问题，再做等着它的事。
    fn run_configs_loaded(&mut self, list: Vec<RunConfig>, problems: Vec<String>, detected: bool) {
        self.run.detected = detected;
        let changed = self.run.list != list;
        self.run.list = list;
        if self.run.problems != problems {
            if let Some(problem) = problems.first() {
                self.notify(
                    NoticeKind::Error,
                    t!("terminal-run-config-problem", problem = problem.as_str()),
                );
            }
            self.run.problems = problems;
        }
        if changed {
            self.relayout();
            self.request_redraw();
        }
        for after in std::mem::take(&mut self.run.after) {
            after(self);
        }
    }

    /// 打开另一个目录时换上它的运行配置和上次选中的配置；读好之前没有配置。
    pub(in crate::app) fn switch_run_configs(&mut self, selected: Option<String>) {
        self.run.selected = selected;
        self.run.problems.clear();
        self.run.list.clear();
        self.run.after.clear();
        self.reload_run_configs();
    }

    /// 窗口建好时读取运行配置和上次选中的配置：启动时恢复的目录不经过 `open_folder_now`。
    pub(in crate::app) fn load_run_configs_for_window(&mut self) {
        let session = self.current_session();
        let selected = session.and_then(|session| session.run.clone());
        self.switch_run_configs(selected);
    }

    /// 一批文件变化里有运行配置文件时重新读取。
    pub(in crate::app) fn run_configs_changed(&mut self, rescan: bool, paths: &[PathBuf]) {
        let Some(root) = self.editor.root() else {
            return;
        };
        let file = run_configs_path(root);
        let affected = |path: &PathBuf| *path == file || affects_detected_configs(root, path);
        if rescan || paths.iter().any(affected) {
            self.reload_run_configs();
        }
    }

    /// 配置 `name` 的标签中程序还在运行时返回（标签在第几栏，在那一栏中的下标）。
    fn running_config(&self, name: &str) -> Option<(usize, usize)> {
        if self.terminal.crashed {
            return None;
        }
        let running =
            |tab: &TermTab| tab.ended.is_none() && tab.run.as_ref().is_some_and(|r| r.name == name);
        (0..self.terminal.group_count()).find_map(|at| {
            let index = self.terminal.group_tabs(at).iter().position(running)?;
            Some((at, index))
        })
    }

    /// 当前选中的配置是否正在运行。
    pub(crate) fn run_config_running(&self) -> bool {
        (self.run.current()).is_some_and(|config| self.running_config(&config.name).is_some())
    }

    /// 运行当前选中的配置，已在运行时结束后重新运行；还没有配置时打开配置文件。
    pub(in crate::app) fn run_current_config(&mut self) {
        if self.remote_has_no_run_configs() {
            return;
        }
        self.reload_run_configs_then(Self::run_loaded_config);
    }

    /// 运行刚读好的配置里当前选中的一个：第一次运行或内容变了时先询问。
    fn run_loaded_config(&mut self) {
        let Some(config) = self.run.current().cloned() else {
            self.edit_run_configs();
            return;
        };
        let Some(root) = self.editor.root() else {
            return;
        };
        let key = trust_key(root, &config);
        let trusted = self.state.command_approved(&key, &config.approval());
        // 没有窗口（测试）时无法询问，也不会有用户的操作触发运行。
        if trusted || self.window.handle.is_none() {
            self.start_run(config);
            return;
        }
        let message = t!("terminal-run-confirm", name = config.name.as_str());
        let mut detail = if self.run.detected {
            t!("terminal-run-confirm-detected").to_owned()
        } else {
            t!("terminal-run-confirm-detail").to_owned()
        };
        let command = command_line(&config, ShellKind::Posix);
        detail.push_str("\n\n");
        detail.push_str(&t!("terminal-run-confirm-command", command = command));
        if let Some(cwd) = config.cwd.as_deref().filter(|cwd| *cwd != ".") {
            detail.push('\n');
            detail.push_str(&t!("terminal-run-confirm-cwd", cwd = cwd));
        }
        for (key, value) in &config.env {
            detail.push('\n');
            let variable = format!("{key}={value}");
            detail.push_str(&t!("terminal-run-confirm-env", variable = variable));
        }
        // 运行不是破坏性操作，按钮用普通样式。
        let buttons = vec![
            AlertButton::new(t!("terminal-run-confirm-action")),
            AlertButton::cancel(t!("common-cancel")),
        ];
        self.show_alert(&message, &detail, buttons, None, move |index, _| {
            (index == 0).then(|| UserEvent::RunApproved(Box::new(config.clone())))
        });
    }

    /// 确认框上同意运行 `config`：记下来，内容不变时不再询问。
    pub(crate) fn approved_run(&mut self, config: RunConfig) {
        if let Some(root) = self.editor.root() {
            let key = trust_key(root, &config);
            let approval = config.approval();
            self.update_state(move |state| state.approve_command_only(&key, &approval));
        }
        self.start_run(config);
    }

    /// 启动 `config`：还在运行时先停止，退出后再启动；已退出的标签被新标签替换。
    pub(in crate::app) fn start_run(&mut self, config: RunConfig) {
        // 辅助进程退出了：先恢复其他标签，运行配置的旧标签随之消失。
        if self.terminal.crashed {
            self.restart_terminal_tab();
        }
        // 这个配置的标签在别的栏里：改用那一栏，在原来的标签里重新运行。
        let has = |tab: &TermTab| tab.run.as_ref().is_some_and(|run| run.name == config.name);
        let column = (0..self.terminal.group_count())
            .find(|&at| self.terminal.group_tabs(at).iter().any(has));
        if let Some(at) = column {
            self.focus_terminal_group(at);
        }
        let t = &mut self.terminal;
        let old = (t.tabs.iter().enumerate())
            .find(|(_, tab)| tab.run.as_ref().is_some_and(|run| run.name == config.name))
            .map(|(index, tab)| (index, tab.id, tab.ended.is_none()));
        if let Some((index, id, true)) = old {
            t.active = index;
            t.tabs[index].rerun = true;
            // 退出后按这一次的配置启动：配置文件在运行期间可能已修改。
            t.tabs[index].run = Some(config);
            if let Some(client) = &t.client {
                client.send(&ToHelper::Stop { tab: id });
            }
            self.drop_other_tab_state();
            self.reveal_run_panel();
            return;
        }
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        // 运行配置不经过交互式 shell 的集成脚本，不涉及 Python 环境的加载。
        let env = environment(self.config.terminal_prompt != Some(false), true);
        let shell = self.config.terminal_shell.as_deref();
        let (launch, env) = run_launch(&config, &root, shell, cfg!(windows), env);
        let banner = banner(&config, launch_kind(&launch, cfg!(windows)));
        self.terminal.run_queue.push(RunStart {
            config,
            launch,
            env,
            banner,
            replaces: old.map(|(_, id, _)| id),
        });
        self.reveal_run_panel();
    }

    /// 显示终端面板并启动辅助进程。焦点留在原处，不打断正在进行的编辑。
    fn reveal_run_panel(&mut self) {
        let focus = self.focus;
        if !self.terminal.open {
            let empty = self.terminal.tab_count() == 0 && self.terminal.want.is_empty();
            self.open_terminal_panel();
            // 打开面板时放的默认标签不需要：要建的就是运行配置的标签。
            if empty {
                self.terminal.want.clear();
                self.terminal.want_names.clear();
                self.terminal.want_shells.clear();
                self.terminal.want_into.clear();
                self.terminal.want_editor.clear();
            }
        }
        self.focus = focus;
        self.start_terminal();
        self.terminal_activated();
        self.relayout();
        self.request_redraw();
    }

    /// 停止当前选中的配置，标签保留。
    pub(in crate::app) fn stop_current_config(&mut self) {
        let Some((at, index)) =
            (self.run.current()).and_then(|config| self.running_config(&config.name))
        else {
            return;
        };
        self.with_terminal_group(at, |app| {
            let t = &mut app.terminal;
            t.tabs[index].rerun = false;
            t.tabs[index].stopped = true;
            if let Some(client) = &t.client {
                client.send(&ToHelper::Stop {
                    tab: t.tabs[index].id,
                });
            }
        });
    }

    /// 按下运行配置的按钮。
    pub(crate) fn press_run_button(&mut self, button: RunButton) {
        match button {
            RunButton::Menu => self.reload_run_configs_then(|app| {
                let hit = HeaderHit::Run(RunButton::Menu);
                let menu = app.hits.terminal.iter().find(|(_, h)| *h == hit);
                let Some(&(menu, _)) = menu else {
                    return;
                };
                let at = menu_below(menu);
                app.open_button_menu(MenuButton::RunConfigs, MenuTarget::RunConfigs, at);
            }),
            RunButton::Start => self.run_current_config(),
            RunButton::Stop => self.stop_current_config(),
        }
    }

    /// 选中第 `index` 个配置。
    pub(in crate::app) fn select_run_config(&mut self, index: usize) {
        if let Some(config) = self.run.list.get(index) {
            self.run.selected = Some(config.name.clone());
            self.remember_session();
        }
    }

    /// 打开的目录在远端主机上时提示没有运行配置并返回 `true`：配置中的命令只在本机的终端里运行。
    fn remote_has_no_run_configs(&mut self) -> bool {
        let remote = self.editor.root().is_some_and(winc_backend::is_remote);
        if remote {
            self.notify(NoticeKind::Warning, t!("terminal-run-remote"));
        }
        remote
    }

    /// 打开运行配置文件，不存在时先用示例新建：这是唯一往项目里写文件的地方，由用户的操作触发。
    pub(in crate::app) fn edit_run_configs(&mut self) {
        if self.remote_has_no_run_configs() {
            return;
        }
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        match create_run_configs(&root) {
            Ok(created) => {
                self.open_path(&run_configs_path(&root));
                // 刚生成的命令来自 Winc，文件也已打开在用户面前。
                self.reload_run_configs_then(move |app| {
                    if created {
                        app.trust_run_configs();
                    }
                });
            }
            Err(error) => {
                self.notify(
                    NoticeKind::Error,
                    t!("terminal-run-create-failed", error = error.to_string()),
                );
            }
        }
    }

    /// 保存了文档 `id`：是运行配置文件时重新读取，并视为用户已经看过其中的命令，运行时不再询问。
    pub(in crate::app) fn run_file_saved(&mut self, id: winc_editor::DocumentId) {
        let path = self.editor.root().map(run_configs_path);
        let doc = self.editor.document(id).and_then(|doc| doc.path.as_ref());
        if doc.is_some_and(|doc| Some(doc) == path.as_ref()) {
            self.reload_run_configs_then(Self::trust_run_configs);
        }
    }

    /// 记下当前文件中所有配置的内容，运行时不再询问。
    fn trust_run_configs(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let trusted: Vec<(PathBuf, String)> = (self.run.list.iter())
            .map(|config| (trust_key(&root, config), config.approval()))
            .collect();
        self.update_state(move |state| {
            for (key, approval) in &trusted {
                state.approve_command_only(key, approval);
            }
        });
    }

    /// 配置 `config` 的程序是否正在运行。
    pub(in crate::app) fn config_running(&self, config: &RunConfig) -> bool {
        self.running_config(&config.name).is_some()
    }

    /// 编号是 `id` 的运行配置标签的程序退出了：不在眼前时用标签上的小点提醒，失败时再提示一声。
    pub(super) fn run_tab_exited(&mut self, id: u16, code: i32) {
        let t = &mut self.terminal;
        let seen = t.open && t.index_of(id) == Some(t.active);
        let Some(tab) = t.by_id(id).filter(|tab| !tab.stopped && !seen) else {
            return;
        };
        let Some(name) = tab.run.as_ref().map(|run| run.name.clone()) else {
            return;
        };
        tab.unseen = Some(code != 0);
        if code != 0 {
            self.notify(
                NoticeKind::Error,
                t!("terminal-run-failed", name = name.as_str(), code = code),
            );
        }
    }

    /// 替换已退出的旧标签：新标签放回旧标签的位置，旧标签的终端随之关闭。
    pub(super) fn retire_run_tabs(&mut self, replaced: Vec<(u16, u16)>) {
        for (new, old) in replaced {
            let t = &mut self.terminal;
            let (Some(from), Some(to)) = (t.index_of(new), t.index_of(old)) else {
                continue;
            };
            if let Some(client) = &t.client {
                client.send(&ToHelper::Close { tab: old });
            }
            let tab = t.tabs.remove(from);
            let to = if from < to { to - 1 } else { to };
            t.tabs[to] = tab;
            t.active = to;
            self.forget_terminal_tab(old);
        }
    }
}

#[cfg(all(test, unix))]
impl super::TerminalPanel {
    /// 测试用：等着启动的配置数量。
    pub(crate) fn run_waiting(&self) -> usize {
        self.run_queue.len()
    }
}

mod script;

#[cfg(test)]
mod tests;
