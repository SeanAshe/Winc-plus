//! 启动：建立应用状态，连接系统菜单与退出拦截。

use super::*;

impl App {
    /// 创建应用状态：窗口、渲染器和字体度量在窗口创建后才会填入。
    ///
    /// `proxy` 为事件循环的代理，测试中传 `None`。
    pub fn new(
        mut editor: Editor,
        proxy: Option<crate::winc::WindowProxy>,
        bench: Bench,
        config: Config,
    ) -> Self {
        // 启动时打开的目录在界面创建之前读入，此处补充设置文件树的显示方式。
        let compact = config.compact_folders != Some(false);
        if let Some(ws) = &mut editor.workspace {
            ws.set_compact(compact);
        }
        // 启动时恢复的标签同样计入最近打开的文件，当前标签排在最前。
        let recent_files = {
            let active = editor.active().and_then(|d| d.path.clone());
            let mut files: Vec<PathBuf> = editor
                .documents
                .iter()
                .rev()
                .filter_map(|d| d.path.clone())
                .collect();
            if let Some(active) = active {
                files.retain(|p| *p != active);
                files.insert(0, active);
            }
            files
        };
        let fonts = ViewFonts::of(&config);
        // 实际度量值（逻辑像素）要在创建窗口、加载字体之后才能得到，此前使用占位值。
        let placeholder = Metrics {
            ascent: 12.0,
            descent: 3.0,
            cell: 8.0,
            x_height: 6.5,
            cap_height: 9.0,
        };
        // 有目录时先展开；只打开文件时由启动流程再收起（见 `Args::launch_sidebar`）。
        let panel_visible = editor.workspace.is_some();
        // 主题以配置为准；配置中没有时参考环境变量 `WINC_THEME`（见 `forced_theme`）。
        let theme_choice = config.theme.unwrap_or(ThemeChoice::Auto);

        Self {
            editor,
            terminal: terminal::TerminalPanel::default(),
            run: terminal::run::RunConfigs::default(),
            saved_config: config.clone(),
            config,
            config_path: None,
            theme_choice,
            state: State::default(),
            state_path: None,
            backup: backup::BackupState::default(),
            opened_terminal: None,
            one_off: Default::default(),
            palette: DARK_THEMES[0].palette,
            fonts,
            ui_metrics: placeholder,
            mono_metrics: placeholder,
            cell: placeholder.cell,
            terminal_metrics: placeholder,
            terminal_cell: placeholder.cell,
            focus: Focus::Editor,
            hover: Hover::None,
            modifiers: ModifiersState::empty(),
            option_side: (false, false),
            mouse: (0.0, 0.0),
            touch: Default::default(),
            drag: Drag::None,
            last_click: None,
            title_click: None,
            window: Default::default(),
            close_pressed: None,
            cursor_icon: CursorIcon::Default,
            drag_scrolled_at: None,
            blink: Default::default(),
            preedit: String::new(),
            field_preedit: String::new(),
            ime: Default::default(),
            layout: Layout::default(),
            tabs: TabBar::default(),
            diff: Default::default(),
            status: Default::default(),
            sticky_cache: None,
            wraps: Arc::default(),
            line_tokens: Default::default(),
            minimap_rows: Default::default(),
            max_scroll_x: None,
            line_widths: ((winc_editor::DocumentId(0), 0, 0), Default::default()),
            ruler_cache: None,
            picture_cache: None,
            images: preview::images::ImageJobs::default(),
            tables: preview::tables::TableJobs::default(),
            table_layout: Default::default(),
            split: preview::images::SplitPreview::default(),
            markdown: preview::markdown::MarkdownPreview::default(),
            pdf: preview::pdf::PdfPreview::default(),
            key_path: Default::default(),
            tree: Default::default(),
            tool_fold: ToolFold::default(),
            scrollbars: Scrollbars::default(),
            sidebar: workspace::Sidebar {
                visible: panel_visible,
                ..Default::default()
            },
            split_share: None,
            name_hint: None,
            bar_card: Default::default(),
            form_dialog: None,
            alert: None,
            dialog_card: Default::default(),
            alert_card: Default::default(),
            module_fx: Default::default(),
            text_content_w: 0.0,
            fold_motion: None,
            scroll_glide: None,
            wheel: Default::default(),
            preview_tab: None,
            file_hover: None,
            name_input: None,
            highlight_pending: false,
            brace_cache: None,
            #[cfg(not(test))]
            clipboard: None,
            #[cfg(test)]
            copied: None,
            #[cfg(test)]
            copied_files: Vec::new(),
            #[cfg(test)]
            copied_image: None,
            line_clipboard: None,
            search: Search::default(),
            find: Default::default(),
            word_highlights: Vec::new(),
            goto_input: None,
            global: GlobalSearch::default(),
            outline: OutlineState::default(),
            hits: Default::default(),
            picker: Default::default(),
            watcher: None,
            changes_due: None,
            rereads: Default::default(),
            saves: Default::default(),
            loads: Default::default(),
            recent_files,
            recent_commands: Vec::new(),
            navigation: Default::default(),
            pending_chord: None,
            location_toggled: Default::default(),
            branch_folders: Default::default(),
            definitions: None,
            settings: SettingsState::default(),
            format: FormatState::new(formatter()),
            git: git::GitState::default(),
            commit: {
                let mut commit = git::commit::CommitState::default();
                commit.set_compact(compact);
                commit
            },
            changes: git::changes::LineChanges::default(),
            log: git::log::LogState::default(),
            blame: git::blame::BlameState::default(),
            branch: git::branch::BranchState::default(),
            merge: git::merge::MergeState::default(),
            rediff: diff::RediffState::default(),
            prompt_kind: PromptKind::GoTo,
            context_menu: None,
            menu_button: None,
            trash: crate::system::move_to_trash,
            closing: Default::default(),
            closed: Vec::new(),
            ask_on_quit: Arc::new(AtomicBool::new(false)),
            deleted_told: HashSet::new(),
            notice: Default::default(),
            cursor_inside: false,
            tooltip: Default::default(),
            renderer: None,
            proxy,
            #[cfg(target_os = "macos")]
            traffic_lights: None,
            key: None,
            shared: Default::default(),
            config_rev: 0,
            cascade: None,
            glyphs_from: None,
            #[cfg(all(unix, not(target_os = "macos")))]
            activation: None,
            next_frame: None,
            #[cfg(any(target_os = "macos", windows))]
            display_link: None,
            frame_interval: Duration::from_micros(16_667),
            bench,
            frame_log: crate::frame_log::FrameLog::from_env(),
            presented: 0,
            dirty: true,
        }
    }

    /// 注册 macOS 的系统回调：菜单栏的「设置…」、右键菜单和目录菜单的选中项、`⌘Q` 和应用内
    /// 更新（菜单中的「检查更新」、定时检查）。
    ///
    /// 回调只向事件循环发送一个事件，由事件循环处理。没有需要询问的内容（未保存的修改、终端中
    /// 运行的程序）时，`⌘Q` 的拦截回调返回 `true`，允许退出；否则返回 `false` 取消本次退出，并
    /// 发送 `QuitRequested`，由事件循环询问。
    pub(super) fn install_system_hooks(&mut self) {
        // 其他平台：以 AppImage 运行或由安装包安装时由应用自行更新（`updates.rs`），
        // 结果同样交给应用层。
        #[cfg(not(target_os = "macos"))]
        if let Some(proxy) = self.proxy.as_ref().map(crate::winc::WindowProxy::active) {
            let (found, state, tick) = (proxy.clone(), proxy.clone(), proxy);
            crate::updates::start(
                move |version| {
                    let _ = found.send_event(UserEvent::UpdateFound(version));
                },
                move |update| {
                    let _ = state.send_event(UserEvent::UpdateState(update));
                },
                move || {
                    let _ = tick.send_event(UserEvent::UpdateTick);
                },
            );
        }
        #[cfg(target_os = "macos")]
        // 以下回调属于整个应用（菜单栏、程序坞、`⌘Q`），交由当前窗口处理。
        if let Some(proxy) = self.proxy.as_ref().map(crate::winc::WindowProxy::active) {
            let settings = proxy.clone();
            crate::macos::add_settings_menu(move || {
                let _ = settings.send_event(UserEvent::OpenSettings);
            });
            // 只有以应用包运行时才有要安装的命令（见 `shell_command::placement`）。
            if crate::shell_command::placement().is_some() {
                let install = proxy.clone();
                crate::macos::add_install_command_menu(move || {
                    let _ = install.send_event(UserEvent::InstallShellCommand);
                });
            }

            // 菜单在事件回调中弹出，选中结果不能在该回调内直接处理，因此转成用户事件。
            let menu = proxy.clone();
            crate::macos::on_menu_command(move |tag| {
                let _ = menu.send_event(UserEvent::MenuCommand(tag));
            });
            let folder = proxy.clone();
            crate::macos::on_folder_menu(status::breadcrumb::macos_folder_items, move |path| {
                let _ = folder.send_event(UserEvent::FolderItem(path));
            });

            let windows = proxy.clone();
            crate::macos::add_window_menus(move || {
                let _ = windows.send_event(UserEvent::NewWindow);
            });

            let reopen = proxy.clone();
            crate::macos::on_reopen(move || {
                let _ = reopen.send_event(UserEvent::Reopen);
            });

            // 应用内更新：后台检查的结果和定时检查都交给应用层（`Winc`），各窗口一起提示。
            let found = proxy.clone();
            let state = proxy.clone();
            let tick = proxy.clone();
            crate::macos::start_updates(
                move |version| {
                    let _ = found.send_event(UserEvent::UpdateFound(version));
                },
                move |update| {
                    let _ = state.send_event(UserEvent::UpdateState(update));
                },
                move || {
                    let _ = tick.send_event(UserEvent::UpdateTick);
                },
            );

            let ask = Arc::clone(&self.ask_on_quit);
            crate::macos::intercept_quit(move || {
                if !ask.load(Ordering::Relaxed) {
                    return true;
                }

                let _ = proxy.send_event(UserEvent::QuitRequested);
                false
            });
        }
    }
}

/// 创建运行格式化工具的 `Formatter`。
///
/// 测试中使用测试进程自身的环境变量，不运行用户的登录 shell；`PATH` 只含系统目录，使项目中
/// 无法确定格式化工具时的选择结果与本机已安装的工具无关。
fn formatter() -> winc_editor::format::Formatter {
    if cfg!(test) {
        let mut vars: Vec<_> = std::env::vars_os().filter(|(k, _)| k != "PATH").collect();
        vars.push(("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()));
        winc_editor::format::Formatter::with_vars(vars)
    } else {
        winc_editor::format::Formatter::default()
    }
}
