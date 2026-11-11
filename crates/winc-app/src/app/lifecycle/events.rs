//! 事件循环：处理窗口事件和后台送回的事件，并安排下一次唤醒。

use winc_config::WindowFrame;

use super::placement::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, MIN_HEIGHT, MIN_WIDTH, place, screens, window_position,
};
use super::*;

impl ApplicationHandler<UserEvent> for App {
    /// 在事件循环结束前保存会话状态；关闭窗口、`⌘Q`、程序坞中的「退出」都经由此处。
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.prepare_exit();
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.handle.is_some() {
            return;
        }

        // 背景图层在创建窗口的同时生成，第一帧绘制时通常已经就绪。窗口大小按上次的估计，
        // 缩放系数按主屏幕的估计，图层的大小允许与实际所需的有出入。
        let expected =
            (self.state.window).map_or((DEFAULT_WIDTH, DEFAULT_HEIGHT), |w| (w.width, w.height));
        let screen = event_loop.primary_monitor();
        let scale = screen.map_or(1.0, |screen| screen.scale_factor() as f32);
        self.request_wallpaper(((expected.0 as f32, expected.1 as f32), scale));

        let title = self
            .editor
            .workspace
            .as_ref()
            .map_or(crate::layout::PRODUCT_NAME, |w| w.name.as_str());
        // 测量时窗口大小必须固定，测量结果才可比较。
        let saved = self
            .state
            .window
            .filter(|_| self.config.restore_window != Some(false) && !self.bench.silent());
        // 从已有窗口打开的新窗口使用记录的大小（没有记录时使用默认大小），位置相对来源窗口向
        // 右下方偏移。偏移后的位置与记录的位置都按当前显示器校验，不在任何显示器上时由系统决定。
        let cascade = self.cascade.take().map(|(at, scale)| WindowFrame {
            position: Some(at),
            scale,
            maximized: false,
            ..saved.unwrap_or(WindowFrame {
                position: None,
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
                scale,
                maximized: false,
            })
        });
        let frame = cascade.or(saved);
        let placed = frame.map(|frame| place(&frame, &screens(event_loop)));
        let (width, height) =
            placed.map_or((DEFAULT_WIDTH, DEFAULT_HEIGHT), |p| (p.width, p.height));
        let mut attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(width, height))
            .with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT))
            .with_theme(forced_theme(&self.theme_choice))
            .with_visible(false);
        if let (Some(frame), Some(placed)) = (frame, placed) {
            attributes = attributes.with_maximized(frame.maximized);
            if let Some(at) = placed.position {
                attributes = attributes.with_position(window_position(at, frame.scale));
            }
        }

        // 由应用绘制标题栏时不使用系统的边框和标题栏（见 `App::sync_title_bar`）。
        self.sync_title_bar();
        attributes = attributes.with_decorations(!self.window.controls);
        // Windows 上去除系统边框的窗口默认没有阴影，与其他窗口重叠时无法区分边界。
        #[cfg(windows)]
        {
            use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};
            attributes = attributes
                .with_undecorated_shadow(true)
                // 接受资源管理器拖入的文件，拖到终端上时把路径贴进正在运行的程序。
                .with_drag_and_drop(true);
            // 图标由构建脚本嵌入可执行文件；窗口类没有默认图标，要在这里指定。
            if let Ok(icon) = winit::window::Icon::from_resource(1, None) {
                attributes = attributes
                    .with_window_icon(Some(icon.clone()))
                    .with_taskbar_icon(Some(icon));
            }
        }
        // Wayland 的 app_id 和 X11 的 WM_CLASS 取自窗口名称，需与桌面项 `winc.desktop` 一致：
        // 任务栏、停靠栏据此找到图标并归组，窗口规则据此匹配。未设置时 Wayland 不上报 app_id。
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::startup_notify::WindowAttributesExtStartupNotify;
            use winit::platform::wayland::WindowAttributesExtWayland;
            attributes = attributes.with_name("winc", "winc");
            // 激活令牌：Wayland 上新窗口凭该令牌切换到前台，X11 上凭该令牌结束启动通知。
            if let Some(token) = self.activation.take() {
                attributes = attributes.with_activation_token(token);
            }
        }
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attributes = attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true);
        }

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("winc: 无法创建窗口：{e}");
                self.give_up_window(event_loop);
                return;
            }
        };

        #[cfg(target_os = "macos")]
        {
            crate::macos::keep_title_bar_drags(&window);
            crate::macos::capture_inserted_text(&window);
        }

        // 窗口初始即为最大化时无法测量还原后的大小，先使用记录的大小。
        self.window.frame = saved;

        // 测量时窗口透明且鼠标点击穿透，必须在显示之前设置，保证没有任何一帧可见。
        #[cfg(target_os = "macos")]
        if self.bench.silent() {
            crate::macos::make_invisible(&window);
        }

        // 没有固定主题时读取系统当前的外观，此后系统切换外观时会收到 `ThemeChanged`。
        let dark = match forced_theme(&self.theme_choice) {
            Some(theme) => theme == Theme::Dark,
            None => window.theme() != Some(Theme::Light),
        };
        self.palette = self.theme_palette(dark);

        let [mono, ui, terminal] = self.font_specs();

        // 配置中的字体找不到或无法绘制时仍使用默认字体启动，并提示用户；否则手动修改配置时写错
        // 字体名，用户只能看到设置未生效而不知道原因。仅在配置了字体时才解析字体。
        let missing: Vec<String> = [
            ("editor.font", &mono, &self.config.editor_font),
            ("ui.font", &ui, &self.config.ui_font),
            ("terminal.font", &terminal, &self.config.terminal_font),
        ]
        .into_iter()
        .filter(|(_, spec, _)| spec.as_ref().is_some_and(|s| s.check().is_err()))
        .map(|(key, _, name)| {
            let name = name.as_deref().unwrap_or_default();
            winc_i18n::t!("workspace-font-missing-item", key = key, name = name)
        })
        .collect();

        // 同一进程中新开的窗口复用来源窗口的字体和字形缓存，两者配置相同，无需重新加载。
        let renderer = match self.glyphs_from.take() {
            Some(glyphs) => Renderer::with_glyphs(window.clone(), self.palette.frame_bg, glyphs),
            None => Renderer::new(
                window.clone(),
                self.palette.frame_bg,
                mono.as_ref(),
                ui.as_ref(),
                terminal.as_ref(),
                self.font_weight(),
            ),
        };
        let renderer = match renderer {
            Ok(renderer) => renderer,
            Err(e) => {
                eprintln!("winc: 无法初始化渲染：{e}");
                self.give_up_window(event_loop);
                return;
            }
        };

        self.renderer = Some(renderer);
        self.measure_fonts();
        self.window.handle = Some(window.clone());
        #[cfg(windows)]
        {
            self.touch.window_input = crate::windows::WindowInput::new(&window);
        }
        self.note_window_frame();
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        {
            let proxy = self.proxy.clone();
            self.traffic_lights = crate::macos::TrafficLights::new(
                &window,
                // 这三个按钮在卡片顶部的顶栏里垂直居中：顶栏上方有 `ISLAND_MARGIN` 的留白，
                // 按钮按高度为留白两倍加顶栏高度的区域居中。
                f64::from(2.0 * crate::theme::ISLAND_MARGIN + crate::theme::TOP_BAR),
                f64::from(crate::theme::TRAFFIC_LIGHT_LEFT),
                move || {
                    if let Some(proxy) = &proxy {
                        let _ = proxy.send_event(UserEvent::TrafficLightsMoved);
                    }
                },
            );
        }
        self.sync_zoom();
        self.place_traffic_lights();
        // 在窗口显示后询问：提示框依附于窗口。
        #[cfg(target_os = "macos")]
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(UserEvent::OfferMoveToApplications);
        }
        self.install_system_hooks();
        // 启动时恢复的目录在事件循环创建之前打开，此时才能开始监视、读取运行配置和恢复终端、侧栏。
        self.watch_folder();
        self.load_run_configs_for_window();
        self.restore_terminal_for_window();
        self.restore_sidebar_for_window();

        // 开启「保存时格式化」时，启动后在后台预先读取用户 shell 的环境，避免第一次 `⌘S` 时等待；
        // 读取耗时取决于用户 shell 初始化脚本的速度。测量时不读取，避免干扰启动测量。
        if self.config.format_on_save == Some(true) && !self.bench.silent() {
            self.format.formatter.prepare(|| {});
        }

        if self.editor.active.is_none() && self.editor.workspace.is_some() && self.sidebar.visible {
            self.focus = Focus::Tree;
        }

        self.announce_readonly();

        if !missing.is_empty() {
            let fonts = missing.join(winc_i18n::t!("workspace-list-separator"));
            let message = match crate::keymap::shortcut(crate::keymap::Action::Settings) {
                Some(key) => winc_i18n::t!("workspace-font-missing-key", fonts = fonts, key = key),
                None => winc_i18n::t!("workspace-font-missing", fonts = fonts),
            };
            eprintln!("winc: {message}");
            self.notify(message);
        }

        self.relayout();
        self.ensure_caret_visible();

        // 先在尚未显示的窗口上绘制第一帧，再显示窗口，避免出现一帧白屏。
        self.redraw();
        #[cfg(windows)]
        self.sync_window_input();

        #[cfg(target_os = "macos")]
        if self.bench.silent() {
            // 测量时窗口显示但不抢占焦点（窗口已设为透明）。
            crate::macos::show_without_focus(&window);
        } else {
            window.set_visible(true);
        }
        #[cfg(not(target_os = "macos"))]
        window.set_visible(true);
        if std::mem::take(&mut self.open_fullscreen) {
            self.enter_fullscreen();
        }

        // 显示链接绑定窗口所在的屏幕，因此在窗口显示之后才创建。
        #[cfg(target_os = "macos")]
        {
            let redraw = window.clone();
            self.display_link =
                crate::macos::DisplayLink::new(&window, move || redraw.request_redraw());
        }
        #[cfg(windows)]
        {
            let redraw = window.clone();
            self.display_link = crate::windows::DisplayLink::new(
                super::window::frame_interval(&window),
                move || {
                    redraw.request_redraw();
                },
            );
        }

        window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        self.handle_event(event);
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.request_close_window(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Moved(_) => {
                // 窗口移动后，此前的按下不再参与顶栏双击的判定。触摸拖动窗口时系统接管输入，
                // 抬手事件可能不再送达，因此在此结束触摸。
                self.title_click = None;
                self.cancel_touch();
                self.note_window_frame();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.cancel_touch();
                self.note_window_frame();
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize();
                }

                // 先摆放窗口控制按钮，布局按其本次的位置计算。
                self.place_traffic_lights();
                self.relayout();
                self.clamp_scroll();
                // 拖动边缘或双击标题栏缩放时，窗口大小每改变一帧，winit 同步发出该事件，新的窗口
                // 大小随这一帧的 Core Animation 事务提交。在此同步绘制，新画面在同一事务中提交，
                // 与窗口大小同时显示。若只标记待重绘，绘制会推迟到之后的事件循环，其间显示上一帧
                // 的画面（固定在左上角），画面与窗口大小不一致。
                self.request_redraw();
                self.redraw();
            }
            // 配置中固定了主题时不跟随系统外观。
            WindowEvent::ThemeChanged(theme) => {
                if self.theme_choice == ThemeChoice::Auto {
                    self.set_theme(theme);
                }
            }
            WindowEvent::Focused(focused) => {
                self.window.focused = focused;
                self.update_foreground(focused);
                // 离开窗口时停止录制按键，菜单项的快捷键随之恢复，不影响别的窗口和应用。
                if !focused {
                    self.cancel_touch();
                    self.set_recording(false);
                }

                if focused {
                    self.check_disk();
                    self.reset_blink();
                    self.terminal_window_focused();
                    // 其他 winc 窗口可能修改了最近列表，重新读取后开始页和项目菜单显示最新内容。
                    if let Some(path) = &self.state_path {
                        self.state = State::load_from(path);
                    }
                } else {
                    let ids: Vec<DocumentId> = self.editor.documents.iter().map(|d| d.id).collect();
                    self.auto_save(&ids);
                    self.drop_drags();
                }

                self.request_redraw();
            }
            // 窗口被完全遮挡、最小化或移到其他桌面时同样视为后台，恢复时按焦点状态处理。
            // 完全不可见时不绘制，并将呈现缓冲标记为可清除（打开多个窗口时只有可见窗口占用缓冲）；
            // 重新可见时立即重绘一帧。测量时窗口透明，系统将其视为不可见，此时仍然绘制。
            WindowEvent::Occluded(occluded) => {
                if occluded {
                    self.cancel_touch();
                }
                self.update_foreground(!occluded && self.window.focused);
                self.set_occluded(occluded);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                let pressed = winit::keyboard::ModifiersKeyState::Pressed;
                self.option_side = (
                    modifiers.lalt_state() == pressed,
                    modifiers.ralt_state() == pressed,
                );
                if self.terminal.open {
                    self.update_terminal_link();
                }
                // 按住 `⌥` 时分支选择器中的「拉取」变为「拉取并变基」，提交按钮变为「提交并推送」。
                if (self.picker.open && self.picker_has_actions()) || self.commit_shown() {
                    self.request_redraw();
                }
                // 拖动文件树项时按下、松开 `⌥` 在移动与复制之间切换，放置目标和拖动卡片随之更新。
                if self.tree_dragging().is_some() {
                    let (x, y) = self.mouse;
                    self.drag_tree_item(x, y);
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                // 以输入法在按键时直接提交的文字（如拼音输入法下的中文标点）代替按键本身的字符。
                #[cfg(target_os = "macos")]
                let event = crate::macos::with_inserted_text(event);
                self.on_key(&event);
            }
            WindowEvent::Ime(ime) => self.on_ime(ime),
            WindowEvent::Touch(touch) => self.on_touch(touch),
            // 有接触点时指针位置由接触点决定；由触摸提升的鼠标事件不得再次触发点击或改变拖动目标。
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
                if self.touch_blocks_mouse() => {}
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_inside = true;
                let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
                self.on_mouse_move(position.x as f32 / scale, position.y as f32 / scale);
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_inside = false;
                if self.tabs.width_lock.take().is_some() {
                    self.request_redraw();
                }
                self.set_hover(Hover::None);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.on_mouse_down(button),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } => self.on_mouse_up(button),
            WindowEvent::MouseWheel { delta, phase, .. } => self.on_wheel_event(delta, phase),
            WindowEvent::PinchGesture { delta, .. } => {
                self.diagram_view_pinch(delta as f32);
                self.pdf_pinch(delta as f32);
                self.picture_pinch(delta as f32);
            }
            WindowEvent::HoveredFile(_) => self.hover_file(),
            WindowEvent::HoveredFileCancelled => self.end_file_hover(),
            WindowEvent::DroppedFile(path) => self.drop_file(&path),
            _ => {}
        }
    }
}

impl App {
    /// 处理送回事件循环的事件 `event`。
    ///
    /// 提示框由窗口自行绘制时，选中按钮的事件同样直接交给此处。
    pub(in crate::app) fn handle_event(&mut self, event: UserEvent) {
        match event {
            UserEvent::Picked(kind, path) => self.on_picked(kind, path),
            #[cfg(all(unix, not(target_os = "macos")))]
            UserEvent::PickFailed(kind, error) => self.pick_failed(kind, error.as_deref()),
            UserEvent::Searched => self.poll_global(),
            UserEvent::Definitions => self.poll_definitions(),
            UserEvent::FilesChanged => self.files_changed(),
            UserEvent::GitQuestion => self.poll_git_questions(),
            done @ (UserEvent::GitStatus
            | UserEvent::GitContent
            | UserEvent::GitCommit
            | UserEvent::GitBase
            | UserEvent::GitOperated
            | UserEvent::GitMergeSides
            | UserEvent::GitLog
            | UserEvent::GitBlame
            | UserEvent::Reread
            | UserEvent::TreeScanned
            | UserEvent::Saved
            | UserEvent::Loaded
            | UserEvent::BackedUp
            | UserEvent::Replaced
            | UserEvent::Wrapped) => self.finished(done),
            UserEvent::Rediffed => self.poll_rediffs(),
            UserEvent::Outline => self.poll_outline(),
            UserEvent::GitRollback(paths) => self.git_rollback(paths),
            UserEvent::GitAbort => {
                self.run_branch_command(crate::app::input::context_menu::MenuCommand::GitAbort);
            }
            UserEvent::GitSwitchBranch(branch, stash) => self.switch_branch(branch, stash),
            UserEvent::GitForcePush => self.run_git_sync(true),
            UserEvent::GitPushConfirmed(intent, never) => self.push_confirmed(intent, never),
            UserEvent::GitCommitAndPush => self.git_commit_and_push(),
            UserEvent::GitUndoCommit => self.undo_last_commit(),
            UserEvent::ReplaceAllConfirmed(total) => self.replace_all_confirmed(total),
            UserEvent::RunApproved(config) => self.approved_run(*config),
            UserEvent::TerminalClose(id) => self.confirmed_terminal_close(id),
            UserEvent::TerminalRestart(id) => self.confirmed_terminal_restart(id),
            UserEvent::ShellCommandDone(done) => self.shell_command_done(done),
            UserEvent::TerminalShells(shells) => {
                self.terminal.shells = shells;
                self.request_redraw();
            }
            UserEvent::TerminalPaste(id, text) => self.confirmed_terminal_paste(id, text),
            UserEvent::TerminalQuit(ok) => {
                if let Some(request) = self.closing.confirming.take()
                    && ok
                {
                    self.request_close(request);
                }
            }
            UserEvent::GitLogConfirmed(op) => self.run_log_confirmed(op),
            UserEvent::GitRemoveWorktree(path, branch) => self.remove_worktree_now(&path, branch),
            UserEvent::Indexed => self.picker_indexed(),
            UserEvent::QuitRequested => self.request_quit(),
            UserEvent::SaveChoice(choice) => self.answer_save_prompt(choice),
            UserEvent::Overwrite(id, choice) => self.answer_overwrite(id, choice),
            UserEvent::FolderItem(path) => self.open_path(&path),
            UserEvent::NewWindow => self.new_window(None),
            // 提示框对应的窗口可能已经关闭：找不到目标窗口时不打开。
            UserEvent::FocusFolder(path) => _ = self.focus_window_with(&path),
            // 应用层在将事件分发给窗口之前已处理。
            UserEvent::Launched(_, answer) => {
                let _ = answer.send(false);
            }
            UserEvent::Opened(_)
            | UserEvent::UpdateFound(_)
            | UserEvent::UpdateTick
            | UserEvent::UpdateState(_) => {}
            UserEvent::Move(paths, dir, never_ask) => self.confirm_move(&paths, &dir, never_ask),
            UserEvent::OpenFolderIn(path, new_window, never_ask) => {
                self.open_folder_in(&path, new_window, never_ask);
            }
            UserEvent::Pasted(result) => self.pasted_into_tree(result),
            UserEvent::Copied(copies, failures) => self.copied_into_tree(copies, failures),
            UserEvent::Reopen => self.show_window(),
            #[cfg(target_os = "macos")]
            UserEvent::OfferMoveToApplications => self.offer_move_to_applications(),
            #[cfg(target_os = "macos")]
            UserEvent::MoveToApplications(accept, never) => {
                self.answer_move_to_applications(accept, never);
            }
            // 只有 macOS 发出这两个事件。
            #[cfg(not(target_os = "macos"))]
            UserEvent::OfferMoveToApplications | UserEvent::MoveToApplications(..) => {}
            UserEvent::TrafficLightsMoved => {
                self.relayout();
                self.request_redraw();
            }
            // `⌘,` 由菜单项接管，不进入按键处理；设置已打开时再次触发则在此处关闭设置。窗口已
            // 关闭时先重新显示窗口。
            UserEvent::OpenSettings if self.settings.open && !self.window.hidden => {
                self.close_settings()
            }
            UserEvent::OpenSettings => {
                self.show_window();
                self.open_settings();
            }
            UserEvent::InstallShellCommand => self.install_shell_command(),
            // 编号为所弹出菜单中的序号，按记录的菜单查找对应的命令。
            UserEvent::MenuCommand(tag) => {
                let command = self.context_menu.as_ref().and_then(|menu| {
                    input::context_menu::MenuEntry::command_at(&menu.entries, tag)
                });
                if let Some(command) = command {
                    self.run_menu_command(command);
                }
            }
            UserEvent::Delete(paths) => self.delete_entries(&paths),
            UserEvent::Formatted => self.poll_formatting(),
            UserEvent::ToolChoice(answer) => self.answer_tool_choice(answer),
            UserEvent::GitTrust(root, fingerprint, trusted) => {
                self.answer_git_trust(&root, fingerprint, trusted);
            }
            UserEvent::FormatterReady => self.formatter_ready(),
            UserEvent::StdinRead => self.poll_stdin(),
            UserEvent::ReopenEncoding(id, encoding) => self.reopen_with_encoding(id, encoding),
            UserEvent::ImageReady => self.poll_images(),
            UserEvent::Terminal => self.poll_terminal(),
        }
    }

    /// 在事件循环空闲之前处理到期的提示、光标闪烁、动画和后台轮询。
    ///
    /// 返回本窗口下一次唤醒的时刻；无需唤醒时返回 `None`。应用层取各窗口中最早的时刻（见
    /// [`crate::winc`]）。窗口何时退出由应用层决定，本窗口只设置 `Closing::quit_requested`。
    pub(crate) fn tick(&mut self) -> Option<Instant> {
        let now = Instant::now();
        self.track_navigation();

        if self.status.until.is_some_and(|t| now >= t) {
            self.status.message = None;
            self.status.until = None;
            self.request_redraw();
        }

        if self.notice.until.is_some_and(|t| now >= t) {
            // 到期后原地淡出；淡出期间按钮不可点击。
            let (message, action) = self.clear_notice();
            let label = action.map(|a| a.label());
            self.notice.leaving = match (message, self.clock(POPUP_OUT)) {
                (Some(message), Some(clock)) => Some(NoticeGhost {
                    message,
                    label,
                    clock,
                }),
                _ => None,
            };
            self.request_redraw();
        }

        if self
            .commit
            .committed
            .as_ref()
            .is_some_and(|(_, t)| now >= *t)
        {
            self.commit.committed = None;
            self.request_redraw();
        }

        let committed = self.commit.committed.as_ref().map(|(_, t)| *t);
        let mut wake = [
            self.status.until,
            self.notice.until,
            committed,
            self.hint_tick(now),
            self.touch_tick(now),
        ]
        .into_iter()
        .flatten()
        .min();

        let resize = self.terminal_resize_tick(now);
        earliest(&mut wake, resize);
        // 当前标签正在读取且保留上一帧未绘制时，到期仍未读取完成则绘制「正在读取」。
        if self.loads.held {
            match self.holding_frame(now) {
                Some(until) => earliest(&mut wake, Some(until)),
                None => {
                    self.loads.held = false;
                    self.request_redraw();
                }
            }
        }
        earliest(&mut wake, self.terminal_settle_tick(now));

        let tooltip_at = self.tooltip.since.map(|since| since + TOOLTIP_DELAY);

        if self.hover.has_tooltip() && !self.tooltip.shown {
            let show = tooltip_at.is_none_or(|at| now >= at);

            if show {
                self.tooltip.shown = true;
                self.request_redraw();
            } else {
                earliest(&mut wake, tooltip_at);
            }
        }

        // 后台线程唤醒界面的事件经过节流，被丢弃的最后一批结果要等到下一个有匹配的文件或搜索
        // 结束才会送达。因此搜索进行中按 `GLOBAL_POLL` 的间隔主动收取结果，
        // 避免列表长时间停留在不完整的状态。
        if self.global.engine.is_running() {
            self.poll_global();
            earliest(&mut wake, Some(now + GLOBAL_POLL));
        }

        // 全局搜索的条件变化后，延迟 `SEARCH_DELAY` 到达 `global.due` 再开始搜索，合并连续的输入。
        if let Some(due) = self.global.due {
            if now >= due {
                self.run_global_search();
            } else {
                earliest(&mut wake, Some(due));
            }
        }

        // 光标闪烁的定时唤醒需要同时满足：窗口有焦点、窗口可见（被完全遮挡时不绘制）、焦点位于
        // 可输入位置、当前文档可以编辑文字（或光标在差异里）、没有进行中的输入法组字；图片预览和
        // 无法打开的文件不显示光标。离上次输入超过 `BLINK_STOP` 后光标停在亮着的状态。
        let typing = self.takes_text();

        let blinking = now.duration_since(self.blink.epoch) < BLINK_STOP;

        if !self.window.focused {
            // 窗口没有焦点时不显示光标（终端的光标改为绘制空心框），
            // 也不停留在失焦前所处的闪烁相位。
            if self.blink.visible {
                self.blink.visible = false;
                self.request_redraw();
            }
        } else if !self.unseen() && typing && blinking && self.preedit.is_empty() {
            let phase = now.duration_since(self.blink.epoch).as_millis() / BLINK.as_millis();
            let visible = phase.is_multiple_of(2);

            if visible != self.blink.visible {
                self.blink.visible = visible;
                self.request_redraw();
            }

            earliest(
                &mut wake,
                Some(self.blink.epoch + BLINK * (phase as u32 + 1)),
            );
        } else if !self.blink.visible {
            self.blink.visible = true;
            self.request_redraw();
        }

        earliest(&mut wake, self.tree_drag_tick(now));

        earliest(&mut wake, self.file_hover_tick(now));

        earliest(&mut wake, self.backup_tick(now));

        earliest(&mut wake, self.tab_drag_tick(now));

        earliest(&mut wake, self.changes_tick(now));

        earliest(&mut wake, self.sync_outline(now));

        earliest(&mut wake, self.blame_tick(now));

        // 停止绘制后处理空闲的呈现缓冲（每块与窗口大小相同）：
        // 标记为可清除或释放，无需等到下一次绘制。
        if let Some(renderer) = &mut self.renderer
            && let Some(at) = renderer.purgeable_at()
        {
            if now >= at {
                renderer.make_idle_purgeable();
            } else {
                earliest(&mut wake, Some(at));
            }
        }

        earliest(&mut wake, self.scrollbar_tick(now));

        earliest(&mut wake, self.settings_scroll_tick(now));

        earliest(&mut wake, self.bench_wheel_tick(now));

        earliest(&mut wake, self.format_progress_tick(now));

        earliest(&mut wake, self.log_tick(now));

        if let Some(at) = self.next_frame {
            if now >= at {
                self.next_frame = None;
                self.request_redraw();
            } else {
                earliest(&mut wake, Some(at));
            }
        }

        if self.dirty
            && !self.unseen()
            && let Some(window) = &self.window.handle
        {
            window.request_redraw();
        }

        wake
    }

    /// 在事件循环结束前收尾：等待后台保存完成，停止格式化，补写同意退出后的修改，记录会话状态。
    pub(crate) fn prepare_exit(&mut self) {
        // 等待后台保存的大文件写完，避免留下写入不完整的临时文件。
        self.wait_for_saves();
        self.cancel_all_formatting();
        // 同意退出后不再定时写备份，但其他窗口仍在询问期间本窗口的文档可能再被修改，因此按当前
        // 内容再写一次备份，使下次启动恢复最新内容。未经询问直接退出时（未保存的内容均已有
        // 备份，或系统注销、关机）同样按当前内容写一次，不丢失最近的修改。
        if self.backup_pending() {
            self.back_up_for_quit();
        }
        self.remember_window();
        self.save_session();
    }

    /// 退出前是否需要询问用户：存在退出时不会写入备份的未保存文档（[`App::kept_on_quit`]），
    /// 或终端中有程序在运行（[`App::terminal_busy`]）。
    ///
    /// macOS 上为假时 `⌘Q`、注销和关机直接退出，退出前在 [`App::prepare_exit`] 中写入备份；为真时
    /// 取消本次退出，回到事件循环询问，注销和关机也随之中断。
    pub(crate) fn asks_before_quit(&self) -> bool {
        self.terminal_busy()
            || (self.editor.documents.iter()).any(|d| d.is_dirty() && !self.kept_on_quit(d))
    }
}

/// 将 `at` 并入下一次唤醒的时间，取两者中较早的一个；`at` 为 `None` 时不变。
fn earliest(wake: &mut Option<Instant>, at: Option<Instant>) {
    *wake = (*wake).into_iter().chain(at).min();
}
