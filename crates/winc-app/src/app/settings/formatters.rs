//! 设置里「格式化」一页：按语言列出的工具和命令、自动选择、保存时跳过、加减语言。

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use winc_editor::format::Survey;
use winc_i18n::t;

use super::*;

/// 格式化页一行的工具选择所对应的键：（任务，语言 ID，设置中写的命令）。没有写命令时为自动选择。
type PlanKey = (Task, &'static str, Option<String>);

/// 后台送回的一批工具选择：发出计算时的轮次（见 [`ToolPlans::round`]），以及各项结果。
type Computed = (u64, Vec<(PlanKey, Chosen)>);

/// 格式化页各行所用的工具选择：在后台线程上计算，界面线程只读取，尚未算出的按未知显示。
///
/// 打开设置时清空，重新检查项目中的文件；设置打开期间只补算缺少的选择。
#[derive(Debug)]
pub struct ToolPlans {
    /// 选择所依据的目录（见 [`App::format_dir`]）。
    dir: PathBuf,
    /// 当前的轮次：每次清空时加 1，此前发出的计算送回的结果作废。
    round: u64,
    /// 已算出的选择。
    plans: HashMap<PlanKey, Chosen>,
    /// 是否有一次计算正在后台进行；同时只进行一次。
    running: bool,
    /// 后台送回结果的通道。
    channel: (Sender<Computed>, Receiver<Computed>),
}

impl Default for ToolPlans {
    fn default() -> Self {
        ToolPlans {
            dir: PathBuf::new(),
            round: 0,
            plans: HashMap::new(),
            running: false,
            channel: mpsc::channel(),
        }
    }
}

impl ToolPlans {
    /// 丢弃已算出的选择，此后按目录 `dir` 重新计算。
    pub(super) fn restart(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.round += 1;
        self.plans.clear();
    }

    /// 是否已经算出各语言的自动选择：第一次计算总是包含全部语言的自动选择。
    fn has_auto(&self) -> bool {
        self.plans.keys().any(|(_, _, command)| command.is_none())
    }

    /// 返回 `language_id` 的 `task` 使用命令 `command` 时的选择；尚未算出时返回 `None`。
    ///
    /// `command` 为 `None` 表示自动选择。
    fn get(&self, task: Task, language_id: &'static str, command: Option<&str>) -> Option<&Chosen> {
        let key = (task, language_id, command.map(str::to_owned));
        self.plans.get(&key)
    }
}

#[cfg(test)]
impl ToolPlans {
    /// 标记为有一次计算正在后台进行，使缺少的选择暂不交给后台（测试用：模拟结果尚未送回）。
    #[cfg_attr(not(unix), allow(dead_code))] // 只有 Unix 上的格式化测试用到
    pub(in crate::app) fn hold(&mut self) {
        self.running = true;
    }
}

/// 后台为格式化页的一行算出的工具选择。
#[derive(Debug, Clone)]
struct Chosen {
    /// 选择的结果。
    plan: Result<Plan, FormatError>,
    /// 项目自带的程序或以路径指定的程序是否存在；在 `PATH` 中查找的程序为 `None`，由
    /// [`winc_editor::format::Formatter::installed`] 回答。
    present: Option<bool>,
}

impl App {
    /// 重新计算格式化页每一行上显示的内容；打开设置、修改命令和后台送回结果之后调用。
    ///
    /// `list_new` 为 `true` 时先把应列出的新语言补到列表中（见 [`App::list_new_languages`]）。
    /// 项目设置只读一次，两步共用。工具选择取自后台计算的结果，缺少的交给后台补算。
    pub(super) fn refresh_format_rows(&mut self, list_new: bool) {
        let dir = self.format_dir();
        if self.settings.tool_plans.dir != dir {
            self.settings.tool_plans.restart(dir.clone());
        }
        let project = self.project_format().unwrap_or_default();
        if list_new {
            self.list_new_languages(&project);
        }
        let listed = &self.settings.listed;
        let rows = (listed.format.iter().map(|&id| (Task::Format, id)))
            .chain(listed.fix.iter().map(|&id| (Task::Fix, id)));
        self.settings.tool_rows = rows
            .filter_map(|(task, id)| {
                let command = task_commands(&project, task).get(id).cloned();
                let language = winc_syntax::find(id)?;
                let auto = self.auto_plan(task, language);
                let removable = command.is_none() && !matches!(auto, Some(Ok(_)));
                let mut row = self.tool_row(task, language, command, auto);
                row.removable = removable;
                Some((tool_setting_of(task, id), row))
            })
            .collect();
        self.plan_tools(&project);
        self.probe_wsl_tools(&dir);
    }

    /// 在后台计算格式化页缺少的工具选择：每种语言的自动选择，以及已列出的语言设置中写的命令。
    ///
    /// 选择工具要逐级检查项目中的文件，耗时与目录的层数和文件系统的延迟成正比，不能在界面线程上
    /// 进行。同时只进行一次计算，结束后（[`App::poll_tool_plans`]）再补算其间新缺少的。
    fn plan_tools(&mut self, project: &Config) {
        let plans = &self.settings.tool_plans;
        if plans.running {
            return;
        }
        let listed = &self.settings.listed;
        let mut wanted: Vec<PlanKey> = Vec::new();
        for task in [Task::Format, Task::Fix] {
            let languages = winc_syntax::languages().iter();
            wanted.extend(languages.map(|language| (task, language.id, None)));
            let ids = match task {
                Task::Format => &listed.format,
                Task::Fix => &listed.fix,
            };
            let configured = ids.iter().filter_map(|&id| {
                let command = (task_commands(project, task).get(id))
                    .or_else(|| task_commands(&self.config, task).get(id))?;
                Some((task, id, Some(command.clone())))
            });
            wanted.extend(configured);
        }
        wanted.retain(|key| !plans.plans.contains_key(key));
        if wanted.is_empty() {
            return;
        }

        let file = plans.dir.join("_");
        let round = plans.round;
        let sender = plans.channel.0.clone();
        let root = self.editor.root().map(Path::to_path_buf);
        let formatter = self.format.formatter.clone();
        self.settings.tool_plans.running = true;
        self.spawn_blocking_then(UserEvent::ToolsPlanned, move || {
            // 各语言共用一次查询：同一级目录中的同一个文件只检查一次。
            let mut survey = Survey::default();
            let chosen = (wanted.into_iter())
                .filter_map(|key| {
                    let (task, id, command) = &key;
                    let language = winc_syntax::find(id)?;
                    let plan =
                        survey.plan(*task, language, &file, root.as_deref(), command.as_deref());
                    let present = match &plan {
                        Ok(plan) if !matches!(plan.program, Program::Search(_)) => {
                            Some(formatter.locate(&plan.program).is_some())
                        }
                        _ => None,
                    };
                    Some((key, Chosen { plan, present }))
                })
                .collect();
            let _ = sender.send((round, chosen));
        });
    }

    /// 接收后台算出的工具选择，并重新计算格式化页的各行；属于已作废的轮次的结果丢弃。
    ///
    /// 第一次算出自动选择之前，列表中只有写过命令的语言；算出后补上项目在用的语言，
    /// 并按语言表的顺序排列一次。
    pub(in crate::app) fn poll_tool_plans(&mut self) {
        let plans = &mut self.settings.tool_plans;
        let first = !plans.has_auto();
        while let Ok((round, chosen)) = plans.channel.1.try_recv() {
            if round == plans.round {
                plans.plans.extend(chosen);
            }
        }
        plans.running = false;
        let first = first && plans.has_auto();
        if self.settings.open {
            self.refresh_format_rows(true);
            if first {
                let order = |id: &&str| winc_syntax::languages().iter().position(|l| l.id == *id);
                self.settings.listed.format.sort_by_key(order);
                self.settings.listed.fix.sort_by_key(order);
            }
            self.request_redraw();
        }
    }

    /// 打开的目录位于 WSL 发行版中时，在后台查询还不知道是否安装的工具，查完后重新计算各行。
    ///
    /// 查询发行版要启动进程，不能在界面线程上等待；同时只进行一轮。
    /// `dir` 是选择工具所依据的目录。
    fn probe_wsl_tools(&mut self, dir: &Path) {
        if self.settings.probing || winc_editor::wsl::locate_here(dir).is_none() {
            return;
        }
        let formatter = &self.format.formatter;
        let listed = &self.settings.listed;
        let rows = (listed.format.iter().map(|&id| (Task::Format, id)))
            .chain(listed.fix.iter().map(|&id| (Task::Fix, id)));
        let unknown: Vec<Plan> = rows
            .filter_map(|(task, id)| {
                let chosen = self.settings.tool_plans.get(task, id, None)?;
                let plan = chosen.plan.as_ref().ok()?;
                (plan.is_guess() && formatter.peek(plan).is_none()).then(|| plan.clone())
            })
            .collect();
        let Some(proxy) = self.link.proxy.clone().filter(|_| !unknown.is_empty()) else {
            return;
        };
        self.settings.probing = true;
        let formatter = formatter.clone();
        std::thread::spawn(move || {
            for plan in &unknown {
                let _ = formatter.resolve(plan);
            }
            let _ = proxy.send_event(crate::app::UserEvent::FormatterReady);
        });
    }

    /// 把应列出的新语言补到格式化页的列表中。
    ///
    /// 列出应列出的语言，分为三类：设置里写过命令的语言、项目设置里写了命令的语言，以及在当前
    /// 项目中会用上工具的语言。第三类不看用户设置也会列出，且不能从列表中移除；是否会用上工具由
    /// [`App::auto_plan`] 判断，格式化看能否自动选中工具，修复看项目里有没有在用的迹象。其余语言
    /// 通过「添加语言…」加入。新语言追加在后面，已列出的不动。
    fn list_new_languages(&mut self, project: &Config) {
        for task in [Task::Format, Task::Fix] {
            let found: Vec<&'static str> = winc_syntax::languages()
                .iter()
                .filter(|l| {
                    task_commands(&self.config, task).contains_key(l.id)
                        || task_commands(project, task).contains_key(l.id)
                        || matches!(self.auto_plan(task, l), Some(Ok(_)))
                })
                .map(|l| l.id)
                .collect();
            let listed = self.settings.listed.of(task);
            for id in found {
                if !listed.contains(&id) {
                    listed.push(id);
                }
            }
        }
    }

    /// 返回不写命令时 `language` 的 `task` 自动选用的工具。
    ///
    /// 项目中没有迹象时按已安装的工具选择。后台尚未算出选择或尚未取得用户 shell 的环境时
    /// 无法确定，返回 `None`，结果送回后重新计算。不访问磁盘。
    fn auto_plan(&self, task: Task, language: &Language) -> Option<Result<Plan, FormatError>> {
        match &self.settings.tool_plans.get(task, language.id, None)?.plan {
            Ok(plan) if plan.is_guess() => self.format.formatter.peek(plan),
            plan => Some(plan.clone()),
        }
    }

    /// 返回 `plan` 要运行的程序是否已安装；不知道时返回 `None`。
    ///
    /// 不访问磁盘：在 `PATH` 中查找的程序按清单判断，其余取后台检查的结果 `chosen`。
    fn tool_installed(&self, plan: &Plan, chosen: Option<&Chosen>) -> Option<bool> {
        match plan.program {
            Program::Search(_) => self.format.formatter.installed(&plan.program),
            _ => chosen?.present,
        }
    }

    /// 返回自动选择工具时用于判断项目在用什么工具的目录。
    ///
    /// 优先取已打开的目录；没有时取当前文件所在的目录，再没有则为 `/`。
    pub(super) fn format_dir(&self) -> PathBuf {
        self.editor
            .root()
            .map(Path::to_path_buf)
            .or_else(|| {
                let path = self.editor.active()?.path.as_ref()?;
                path.parent().map(Path::to_path_buf)
            })
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// 计算 `language` 的 `task` 工具行上显示的内容。
    ///
    /// `project` 是项目设置中写的命令，`auto` 是自动选用的工具（见 [`App::auto_plan`]）。
    fn tool_row(
        &self,
        task: Task,
        language: &'static Language,
        project: Option<String>,
        auto: Option<Result<Plan, FormatError>>,
    ) -> FormatRow {
        let presets_for = |id| presets(task, id);
        let auto_tool = match &auto {
            Some(Ok(plan)) => plan.auto.as_ref(),
            _ => None,
        };

        let user = task_commands(&self.config, task);
        let configured = project.as_ref().or_else(|| user.get(language.id));
        let preset = configured.and_then(|c| presets_for(language.id).find(|p| p.command == c));
        let always = |name: &str| {
            presets_for(language.id).any(|p| p.name == name && matches!(p.auto, Auto::Always))
        };
        // 能自动选用的工具都没装时，这里列出候选预设；只有一个时按钮上直接写它，不写「自动」。
        let missing: Vec<&str> = match &auto {
            Some(Err(FormatError::NoTool { tools, .. })) => presets_for(language.id)
                .filter(|p| tools.iter().any(|t| t == p.program()))
                .map(|p| p.name)
                .collect(),
            _ => Vec::new(),
        };
        // 没指定命令、也选不出工具，且不是因为还没找过或没装：只能请用户自己选。
        let unset = configured.is_none()
            && auto_tool.is_none()
            && matches!(auto, Some(Err(FormatError::Unset(_))));
        let (tool, detail) = match (configured, auto_tool) {
            (Some(_), _) if project.is_some() => (
                preset
                    .map_or(t!("format-tool-custom"), |p| p.name)
                    .to_owned(),
                t!("format-tool-from-project").to_owned(),
            ),
            (Some(c), _) => (
                preset
                    .map_or(t!("format-tool-custom"), |p| p.name)
                    .to_owned(),
                c.clone(),
            ),
            (None, Some((name, reason))) => {
                let how = match reason {
                    Some(reason) => t!("format-tool-auto-reason", reason = reason.as_str()),
                    None if always(name) => t!("format-tool-auto").to_owned(),
                    None => t!("format-tool-auto-installed").to_owned(),
                };
                ((*name).to_owned(), how)
            }
            (None, None) => match &auto {
                // 修复工具一组只自动列出项目在用的，这样的行是用户自己加的，请他选一个。
                Some(Err(FormatError::Unset(_)))
                    if task == Task::Fix && presets_for(language.id).next().is_some() =>
                {
                    (
                        t!("format-tool-unset").to_owned(),
                        t!("format-tool-fix-unset").to_owned(),
                    )
                }
                // 有预设但都不自动使用（例如 xmllint）时请用户选一个；没有预设时只能自己写命令。
                Some(Err(FormatError::Unset(_))) => (
                    t!("format-tool-unset").to_owned(),
                    if presets_for(language.id).next().is_some() {
                        t!("format-tool-no-auto")
                    } else {
                        t!("format-tool-no-preset")
                    }
                    .to_owned(),
                ),
                None => (
                    t!("format-tool-auto").to_owned(),
                    t!("format-tool-searching").to_owned(),
                ),
                Some(Err(FormatError::NoTool { .. })) if missing.len() == 1 => {
                    (missing[0].to_owned(), t!("format-tool-auto").to_owned())
                }
                _ => (
                    t!("format-tool-auto").to_owned(),
                    t!("format-tool-auto").to_owned(),
                ),
            },
        };
        let auto_text = auto_tool.map(|(name, reason)| match reason {
            Some(reason) => t!(
                "format-tool-auto-with-reason",
                name = *name,
                reason = reason.as_str()
            ),
            None => (*name).to_owned(),
        });
        let plans = &self.settings.tool_plans;
        let chosen = plans.get(task, language.id, configured.map(String::as_str));
        let plan = match configured {
            Some(_) => chosen.map(|chosen| chosen.plan.clone()),
            None => auto,
        };
        let (warning, install) = match plan {
            Some(Err(FormatError::Failed(message))) => (Some(message), Vec::new()),
            Some(Err(FormatError::NoTool { tools, .. })) => {
                let names: Vec<&str> = presets_for(language.id)
                    .filter(|p| tools.iter().any(|t| t == p.program()))
                    .map(|p| p.name)
                    .collect();
                // 按钮上已经显示该工具时，不再重复它的名字。
                let warning = if names == [tool.as_str()] {
                    t!("format-tool-not-installed").to_owned()
                } else {
                    let names = names.join(t!("format-tool-list-separator"));
                    t!("format-tool-not-installed-names", names = names)
                };
                (Some(warning), tools)
            }
            Some(Ok(_)) if configured.is_some_and(|c| Plan::reads_file_from_disk(c)) => {
                (Some(t!("format-tool-reads-disk").to_owned()), Vec::new())
            }
            Some(Ok(plan)) if self.tool_installed(&plan, chosen) == Some(false) => {
                let tool = plan.tool().into_owned();
                let known =
                    matches!(plan.program, Program::Search(_)) && knows_how_to_install(&tool);
                let install = if known {
                    vec![tool.clone()]
                } else {
                    Vec::new()
                };
                let warning = if known {
                    t!("format-tool-not-installed").to_owned()
                } else {
                    t!("format-tool-not-found", tool = tool.as_str())
                };
                (Some(warning), install)
            }
            _ => (None, Vec::new()),
        };

        FormatRow {
            tool,
            unset,
            detail,
            auto: auto_text,
            project,
            warning,
            install,
            removable: false,
        }
    }

    /// 用户 shell 的环境取到之后调用：此时才能确定能否找到工具，因此补上提醒，
    /// 并列出装了可用工具的语言（追加在后面，已经列出的不改变位置）。
    pub(in crate::app) fn formatter_ready(&mut self) {
        self.settings.probing = false;
        if self.settings.open {
            self.refresh_format_rows(true);
            self.request_redraw();
        }
    }

    /// 在 `setting` 所在行的下方弹出该设置项的菜单。
    ///
    /// 「添加语言…」的菜单列出还没有列出的语言；「排除的语言」的菜单列出已列出的语言
    /// （被排除的打勾）；某种语言的工具行的菜单列出该语言可用的格式化（修复）工具。
    pub(in crate::app) fn settings_open_menu(&mut self, setting: Setting) {
        let Some(row) = self.settings_layout().row(setting).copied() else {
            return;
        };
        let (target, anchor) = match setting {
            Setting::AddLanguage => (
                MenuTarget::AddLanguage(Task::Format),
                row.row.inset(4.0, 4.0),
            ),
            Setting::AddFixLanguage => {
                (MenuTarget::AddLanguage(Task::Fix), row.row.inset(4.0, 4.0))
            }
            Setting::SkipOnSave => (MenuTarget::SkipOnSave, row.control),
            // 右边对齐控件：不长于控件时与控件同宽，字体名更长时向左展开。
            Setting::Language
            | Setting::RenderWhitespace
            | Setting::VimClipboard
            | Setting::VimKey(_)
            | Setting::GitOpenWorktree
            | Setting::GitLogPlacement
            | Setting::GitPullMode
            | Setting::AiProtocol
            | Setting::DropFolder
            | Setting::FindInFiles
            | Setting::AutoSave
            | Setting::CursorStyle
            | Setting::SmoothCaret
            | Setting::Zoom
            | Setting::BackgroundImage
            | Setting::BackgroundFit
            | Setting::TextWidth
            | Setting::DefaultLanguage
            | Setting::TerminalPosition
            | Setting::TerminalTabPosition
            | Setting::TerminalOptionAsMeta
            | Setting::TerminalRightClick
            | Setting::UiFont
            | Setting::EditorFont
            | Setting::TerminalFont
            | Setting::MarkdownFont => {
                let target = match setting {
                    Setting::Language => MenuTarget::Locale,
                    Setting::RenderWhitespace => MenuTarget::Whitespace,
                    Setting::VimClipboard => MenuTarget::VimClipboard,
                    Setting::VimKey(c) => MenuTarget::VimKey(c),
                    Setting::GitOpenWorktree => MenuTarget::OpenWorktree,
                    Setting::GitPullMode => MenuTarget::PullMode,
                    Setting::AiProtocol => MenuTarget::AiProtocol,
                    Setting::GitLogPlacement => MenuTarget::LogPlacement,
                    Setting::DropFolder => MenuTarget::DropFolder,
                    Setting::FindInFiles => MenuTarget::SearchStyle,
                    Setting::AutoSave => MenuTarget::AutoSave,
                    Setting::CursorStyle => MenuTarget::CursorStyle,
                    Setting::SmoothCaret => MenuTarget::SmoothCaret,
                    Setting::Zoom => MenuTarget::Zoom,
                    Setting::BackgroundImage => MenuTarget::BackgroundImage,
                    Setting::BackgroundFit => MenuTarget::BackgroundFit,
                    Setting::TextWidth => MenuTarget::TextWidth,
                    Setting::DefaultLanguage => MenuTarget::DefaultLanguage,
                    Setting::TerminalPosition => MenuTarget::TerminalPosition,
                    Setting::TerminalTabPosition => MenuTarget::TerminalTabPosition,
                    Setting::TerminalOptionAsMeta => MenuTarget::OptionAsMeta,
                    Setting::TerminalRightClick => MenuTarget::RightClick,
                    _ => MenuTarget::Font(setting),
                };
                self.settings.selected = setting;
                let (_, y) = menu_below(row.control);
                return self.open_context_menu(target, (row.control.right(), y));
            }
            Setting::FormatCommand(id) => (MenuTarget::Tool(Task::Format, id), row.control),
            Setting::FixCommand(id) => (MenuTarget::Tool(Task::Fix, id), row.control),
            Setting::TerminalShell => (MenuTarget::DefaultShell, row.control),
            _ if setting.is_text_field() => return self.settings_start_editing(setting),
            Setting::Updates => return self.settings_check_updates(),
            Setting::ExportSettings => return self.pick(Pick::ExportSettings),
            Setting::ImportSettings => return self.pick(Pick::ImportSettings),
            Setting::Keymap => return self.open_keymap_menu_at(row.control),
            Setting::KeyMatches => return self.show_key_matches(),
            _ => return,
        };
        self.settings.selected = setting;
        self.open_context_menu(target, menu_below(anchor));
    }

    /// 处理「排除的语言」菜单中对语言 `id` 的点击：已排除的恢复，未排除的加入排除。
    pub(in crate::app) fn settings_toggle_skip(&mut self, id: &str) {
        let skipped = &mut self.config.format_on_save_exclude;
        match skipped.iter().position(|l| l == id) {
            Some(i) => {
                skipped.remove(i);
            }
            None => skipped.push(id.to_owned()),
        }
        self.save_config();
        self.request_redraw();
    }

    /// 返回「排除的语言」右侧显示的文字：被排除的语言名称，以「、」分隔。
    ///
    /// 没有被排除的语言时返回 `None`。
    pub(crate) fn skipped_on_save(&self) -> Option<String> {
        let names: Vec<&str> = winc_syntax::languages()
            .iter()
            .filter(|l| {
                self.config
                    .format_on_save_exclude
                    .iter()
                    .any(|id| id == l.id)
            })
            .map(|l| l.name)
            .collect();
        (!names.is_empty()).then(|| names.join(t!("settings-list-separator")))
    }

    /// 处理 `task` 那一组「添加语言…」菜单中选中的 `language`：加一行，并弹出这一行的工具菜单。
    ///
    /// 没有预设工具的语言只能自己写命令，直接进入输入框。
    pub(in crate::app) fn settings_add_language(
        &mut self,
        task: Task,
        language: &'static Language,
    ) {
        let listed = self.settings.listed.of(task);
        if !listed.contains(&language.id) {
            listed.push(language.id);
            self.refresh_format_rows(false);
        }
        let setting = tool_setting_of(task, language.id);
        self.settings.selected = setting;
        self.settings_scroll_into_view();
        if presets(task, language.id).next().is_some() {
            self.settings_open_menu(setting);
        } else {
            self.settings_start_editing(setting);
        }
        self.request_redraw();
    }

    /// 把语言 `id` 从 `task` 那一组中移除（工具菜单中的「移除」），并删掉用户为它写的这一组的
    /// 命令；另一组不受影响。
    ///
    /// 选中项移到原位置上的下一行（已是最后一行时为上一行），组里没有语言了就选中这一组的
    /// 「添加语言…」。
    pub(super) fn settings_remove_language(&mut self, task: Task, id: &'static str) {
        let setting = tool_setting_of(task, id);
        let listed = self.settings.listed.of(task);
        let Some(index) = listed.iter().position(|&l| l == id) else {
            return;
        };
        listed.remove(index);
        self.settings.selected = match listed.get(index).or(listed.last()) {
            Some(&next) => tool_setting_of(task, next),
            None => match task {
                Task::Format => Setting::AddLanguage,
                Task::Fix => Setting::AddFixLanguage,
            },
        };
        if matches!(&self.settings.editing, Some((s, _)) if *s == setting) {
            self.settings_cancel_field();
        }
        if matches!(&self.settings.error, Some((s, _)) if *s == setting) {
            self.settings.error = None;
        }

        if task_commands_mut(&mut self.config, task)
            .remove(id)
            .is_some()
        {
            self.save_config();
        }
        self.focus = Focus::Settings;
        self.refresh_format_rows(false);
        let max = self.settings_layout().max_scroll();
        self.settings.scroll = self.settings.scroll.min(max);
        self.request_redraw();
    }

    /// 处理某种语言的格式化（修复）工具菜单中选中的一项 `command`。
    ///
    /// 可选项：自动、某个预设（写入配置）、自定义命令（进入输入框）、安装方法（关闭设置并
    /// 打开安装说明）、移除（把这种语言从列表中去掉）。
    pub(in crate::app) fn settings_choose_tool(&mut self, setting: Setting, command: MenuCommand) {
        let Some((task, id)) = tool_setting(setting) else {
            return;
        };
        self.settings.selected = setting;
        match command {
            MenuCommand::ToolAuto => self.settings_commit_command(task, id, None),
            MenuCommand::ToolPreset(i) => {
                if let Some(preset) = PRESETS.get(i) {
                    self.settings_commit_command(task, id, Some(preset.command.to_owned()));
                }
            }
            MenuCommand::ToolCustom => self.settings_start_editing(setting),
            MenuCommand::ToolRemove => self.settings_remove_language(task, id),
            MenuCommand::ToolInstall => {
                let tools = self
                    .settings
                    .tool_row(setting)
                    .map(|r| r.install.clone())
                    .unwrap_or_default();
                let dir = self.format_dir();
                self.close_settings();
                self.show_install_guide(&tools, &dir);
            }
            _ => {}
        }
    }
}
