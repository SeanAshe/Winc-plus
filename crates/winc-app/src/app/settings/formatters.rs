//! 设置里「格式化」一页：按语言列出的工具和命令、自动选择、保存时跳过、加减语言。

use winc_i18n::t;

use super::*;

impl App {
    /// 重新计算格式化页每一行上显示的内容；打开设置、修改命令、取到用户 shell 的环境之后调用。
    ///
    /// `list_new` 为 `true` 时先把应列出的新语言补到列表中（见 [`App::list_new_languages`]）。
    /// 项目设置只读一次，两步共用。
    pub(super) fn refresh_format_rows(&mut self, list_new: bool) {
        let file = self.format_dir().join("_");
        let project = self.project_format().unwrap_or_default();
        if list_new {
            self.list_new_languages(&file, &project);
        }
        let listed = &self.settings.languages;
        let rows = (listed.format.iter().map(|&id| (Task::Format, id)))
            .chain(listed.fix.iter().map(|&id| (Task::Fix, id)));
        self.settings.tool_rows = rows
            .filter_map(|(task, id)| {
                let command = task_commands(&project, task).get(id).cloned();
                let language = winc_syntax::find(id)?;
                let auto = self.auto_plan(task, language, &file);
                let removable = command.is_none() && !matches!(auto, Some(Ok(_)));
                let mut row = self.tool_row(task, language, &file, command, auto);
                row.removable = removable;
                Some((tool_setting_of(task, id), row))
            })
            .collect();
    }

    /// 把应列出的新语言补到格式化页的列表中。
    ///
    /// 列出应列出的语言，分为三类：设置里写过命令的语言、项目设置里写了命令的语言，以及在当前
    /// 项目中会用上工具的语言。第三类不看用户设置也会列出，且不能从列表中移除；是否会用上工具由
    /// [`App::auto_plan`] 判断，格式化看能否自动选中工具，修复看项目里有没有在用的迹象。其余语言
    /// 通过「添加语言…」加入。`file` 是判断项目工具用的虚拟路径；新语言追加在后面，已列出的不动。
    fn list_new_languages(&mut self, file: &Path, project: &Config) {
        for task in [Task::Format, Task::Fix] {
            let found: Vec<&'static str> = winc_syntax::languages()
                .iter()
                .filter(|l| {
                    task_commands(&self.config, task).contains_key(l.id)
                        || task_commands(project, task).contains_key(l.id)
                        || matches!(self.auto_plan(task, l, file), Some(Ok(_)))
                })
                .map(|l| l.id)
                .collect();
            let listed = self.settings.languages.of(task);
            for id in found {
                if !listed.contains(&id) {
                    listed.push(id);
                }
            }
        }
    }

    /// 返回不写命令时 `language` 的 `task` 自动选用的工具；`file` 是用于判断项目工具的
    /// 虚拟文件路径。
    ///
    /// 项目中看不出用什么工具时，看装了哪个工具。尚不能确定（还没取到用户 shell 的环境）时
    /// 返回 `None`，取到之后再计算一遍。
    fn auto_plan(
        &self,
        task: Task,
        language: &Language,
        file: &Path,
    ) -> Option<Result<Plan, FormatError>> {
        match Plan::for_task(task, language, file, self.editor.root(), None) {
            Ok(plan) if plan.is_guess() => self.format.formatter.peek(&plan),
            plan => Some(plan),
        }
    }

    /// 返回自动选择工具时用于判断项目在用什么工具的目录。
    ///
    /// 优先取已打开的目录；没有时取当前文件所在的目录，再没有则为 `/`。
    fn format_dir(&self) -> PathBuf {
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
    /// `file` 是用于判断项目工具的虚拟文件路径，`project` 是项目设置中写的命令，`auto` 是自动
    /// 选用的工具（见 [`App::auto_plan`]）。
    fn tool_row(
        &self,
        task: Task,
        language: &'static Language,
        file: &Path,
        project: Option<String>,
        auto: Option<Result<Plan, FormatError>>,
    ) -> FormatRow {
        let root = self.editor.root();
        let formatter = &self.format.formatter;
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
        let plan = match configured {
            Some(command) => Some(Plan::for_task(task, language, file, root, Some(command))),
            None => auto,
        };
        let (warning, install) = match plan {
            Some(Err(FormatError::Failed(message))) => (Some(message), Vec::new()),
            Some(Err(FormatError::NoTool { tools, .. })) => {
                let names: Vec<&str> = presets_for(language.id)
                    .filter(|p| tools.iter().any(|t| t == p.program()))
                    .map(|p| p.name)
                    .collect();
                // 按钮上已经写着该工具时，不再重复它的名字。
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
            Some(Ok(plan)) if formatter.installed(&plan.program) == Some(false) => {
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
            | Setting::Zoom
            | Setting::BackgroundImage
            | Setting::BackgroundFit
            | Setting::TerminalPosition
            | Setting::TerminalOptionAsMeta
            | Setting::UiFont
            | Setting::EditorFont
            | Setting::TerminalFont => {
                let target = match setting {
                    Setting::Language => MenuTarget::Locale,
                    Setting::RenderWhitespace => MenuTarget::Whitespace,
                    Setting::VimClipboard => MenuTarget::VimClipboard,
                    Setting::Zoom => MenuTarget::Zoom,
                    Setting::BackgroundImage => MenuTarget::BackgroundImage,
                    Setting::BackgroundFit => MenuTarget::BackgroundFit,
                    Setting::TerminalPosition => MenuTarget::TerminalPosition,
                    Setting::TerminalOptionAsMeta => MenuTarget::OptionAsMeta,
                    _ => MenuTarget::Font(setting),
                };
                self.settings.selected = setting;
                let (_, y) = menu_below(row.control);
                return self.open_context_menu(target, (row.control.right(), y));
            }
            Setting::FormatCommand(id) => (MenuTarget::Tool(Task::Format, id), row.control),
            Setting::FixCommand(id) => (MenuTarget::Tool(Task::Fix, id), row.control),
            Setting::TerminalShell => (MenuTarget::DefaultShell, row.control),
            Setting::Updates => return self.settings_check_updates(),
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
        let listed = self.settings.languages.of(task);
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
        let listed = self.settings.languages.of(task);
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
