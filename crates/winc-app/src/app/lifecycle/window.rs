//! 窗口：标题栏、前后台、显示和隐藏，各区域的排布，以及每帧的绘制。

use super::*;

impl App {
    /// 处理双击顶栏空白处。
    ///
    /// macOS 上执行系统偏好中的标题栏双击动作（缩放或最小化），其他平台切换最大化。
    pub(in crate::app) fn title_bar_double_click(&self) {
        let Some(window) = &self.window else {
            return;
        };

        #[cfg(target_os = "macos")]
        crate::macos::title_bar_double_click(window);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            self.toggle_maximized();
        }
    }

    /// 告诉渲染器窗口是否在前台（有焦点且没有被完全遮住）：转入后台时马上把空闲的呈现缓冲
    /// 标成可清除。
    ///
    /// 测量模式下窗口不激活、完全透明，系统把它当作在后台；仍按前台处理，测得的内存与平时
    /// 使用时可比。
    pub(super) fn update_foreground(&mut self, foreground: bool) {
        if let Some(renderer) = &mut self.renderer {
            renderer.set_foreground(foreground || self.bench.silent());
        }
    }

    /// 设置窗口标题（程序坞和「窗口」菜单中显示的名字）。
    ///
    /// `AppKit` 修改标题时会重排标题栏并把那三个控制按钮恢复到默认位置，因此随后重新摆放
    /// 一次。
    pub(in crate::app) fn set_window_title(&self, title: &str) {
        if let Some(window) = &self.window {
            window.set_title(title);
        }
        self.place_traffic_lights();
    }

    /// 隐藏窗口（macOS 上关闭窗口时）：应用留在程序坞中，点击图标时由 [`App::show_window`]
    /// 重新显示。
    pub(super) fn hide_window(&mut self) {
        if let Some(window) = &self.window {
            window.set_visible(false);
        }
        self.window_hidden = true;
    }

    /// 打开 `targets` 里的文件（目录跳过：已经是本窗口的目录），`文件:行:列` 跳到该位置，再把
    /// 窗口切到前台。
    pub(crate) fn open_files(&mut self, targets: &[crate::cli::Target]) {
        for target in targets.iter().filter(|t| !t.path.is_dir()) {
            self.open_path(&target.path);
            if let Some(line) = target.line {
                self.jump_to(line, target.col.unwrap_or(0), 0);
            }
        }
        self.raise();
    }

    /// 窗口是否完全看不见（见 `occluded`）；看得见时，或者测量时（窗口透明，系统当它看不见）
    /// 为假。
    pub(in crate::app) fn unseen(&self) -> bool {
        self.occluded && !self.bench.silent()
    }

    /// 窗口变得完全看不见或又看得见（`WindowEvent::Occluded`）。
    pub(super) fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
        let unseen = self.unseen();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_occluded(unseen);
        }
        if !occluded {
            self.request_redraw();
        }
    }

    /// 本窗口的字体和字形缓存，给之后新开的窗口共用；还没有渲染器时为 `None`。
    pub(crate) fn shared_glyphs(&self) -> Option<winc_render::SharedGlyphs> {
        self.renderer.as_ref().map(Renderer::shared_glyphs)
    }

    /// 把窗口切到前台：关着的重新显示，最小化的还原。
    pub(crate) fn raise(&mut self) {
        self.show_window();
        if let Some(window) = &self.window {
            window.set_minimized(false);
            window.focus_window();
        }
        self.request_redraw();
    }

    /// 是否显示状态栏（`statusBar`）。
    pub(crate) fn status_bar_shown(&self) -> bool {
        self.config.status_bar != Some(false)
    }

    /// 新窗口放在哪里：比本窗口往右下错开，形成层叠。
    pub(crate) fn cascade_from(&self) -> Option<winit::dpi::Position> {
        // 错开的距离（逻辑像素），和 macOS 标题栏的高度接近。
        const CASCADE: f64 = 28.0;
        let window = self.window.as_ref()?;
        let at = window.outer_position().ok()?;
        let at = at.to_logical::<f64>(window.scale_factor());
        Some(winit::dpi::LogicalPosition::new(at.x + CASCADE, at.y + CASCADE).into())
    }

    /// 窗口建不起来（系统窗口或渲染器）：还有别的窗口时只放弃这一个（标成关掉，应用层去掉它），
    /// 否则结束。
    pub(super) fn give_up_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.shared.others() {
            self.window = None;
            self.window_hidden = true;
        } else {
            event_loop.exit();
        }
    }

    /// 重新显示隐藏的窗口，并让它成为前台窗口；窗口没有隐藏时不做任何事。
    ///
    /// 先在隐藏的窗口上画好一帧（开始页）再显示，不会先显示关闭前的画面。
    pub(in crate::app) fn show_window(&mut self) {
        if !self.window_hidden {
            return;
        }
        self.window_hidden = false;
        // 关着时系统当它看不见；显示之前先画好一帧，不等 `Occluded(false)`。
        self.set_occluded(false);
        self.relayout();
        self.request_redraw();
        self.redraw();
        if let Some(window) = &self.window {
            window.set_visible(true);
            window.focus_window();
        }
    }

    /// 把窗口左上角的三个控制按钮放进顶栏并垂直居中，并按它们的实际位置调整右侧按钮的
    /// 布局。
    ///
    /// 切换窗口外观或修改标题后 `AppKit` 会把它们恢复到默认位置，因此需要再次摆放。
    pub(super) fn place_traffic_lights(&self) {
        #[cfg(target_os = "macos")]
        if let Some(window) = &self.window
            && let Some(end) = crate::macos::place_traffic_lights(
                window,
                // 这三个按钮在卡片顶部的顶栏里垂直居中：顶栏上方有 `ISLAND_MARGIN` 的留白，
                // 按钮按高度为留白两倍加顶栏高度的区域居中。
                (2.0 * ISLAND_MARGIN + TOP_BAR) as f64,
                TRAFFIC_LIGHT_LEFT as f64,
            )
            && crate::layout::set_traffic_lights_end(end as f32)
        {
            // 每次重绘都会重新计算布局。
            window.request_redraw();
        }
    }

    /// 重新计算各区域的矩形。
    ///
    /// 窗口尺寸、面板宽度、标签或预览状态变化后调用；面板跟着焦点关闭也在这一步完成。
    pub(in crate::app) fn relayout(&mut self) {
        // 面板打开以后换了标签（⌘W、切换标签、后退前进、新建文件）：面板里的符号、书签、用法
        // 都属于原来的文档，回车会在新标签里跳到错误的位置，所以关掉。`close_picker` 会再次
        // 调用本函数，那时面板已经关了。
        if self.picker_open && self.editor.active().map(|d| d.id) != self.picker_doc {
            self.close_picker();
            return;
        }
        // 浮层和焦点不分家：快捷键、点击把焦点挪到了其他位置（⌘F、⇧⌘F、⌃G、⌃Tab、点编辑区……）
        // 时面板、输入条跟着关掉。否则 Esc 和回车都到不了面板，面板会一直留在屏幕上。
        if self.picker_open && self.focus != Focus::Picker {
            self.close_picker();
            return;
        }
        if self.goto_input.is_some() && self.focus != Focus::Prompt {
            self.goto_input = None;
        }
        let Some(renderer) = &self.renderer else {
            return;
        };
        let (w, h) = renderer.logical_size();

        // 隐藏状态栏时留下和窗口其他三边一样的边距，卡片不贴着窗口底边。
        let status_height = if self.status_bar_shown() {
            STATUS_HEIGHT
        } else {
            ISLAND_MARGIN
        };
        let rows = match (
            self.search_open && self.editor.active.is_some(),
            self.replace_open,
        ) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        };
        // ⌘P 面板高度随结果数变化：至少一行（显示「正在整理文件…」「没有匹配」），最多
        // `PICKER_MAX_ROWS` 行，且不能延伸到状态栏。
        let picker_rows = if self.picker_open {
            let room =
                (h - status_height - TOP_BAR_END - BAR_HEIGHT - 2.0 * PICKER_PAD - 16.0).max(0.0);
            let most = ((room / PICKER_ROW).floor() as usize).clamp(1, PICKER_MAX_ROWS);
            self.picker_hits.len().clamp(1, most)
        } else {
            0
        };
        let (panel, shown) = (self.panel_width, self.sidebar_shown());
        self.layout = Layout::compute(
            w,
            h,
            panel,
            shown,
            rows,
            picker_rows,
            self.gutter_width(),
            status_height,
        );
        self.place_places();
        self.animate_picker((w, h));
        if let Some(bottom) = self.places_bottom {
            self.layout.picker.y = bottom - self.layout.picker.h;
        }

        // 差异标签：查找条放在差异的标题栏下面，不盖住标题栏右端的按钮。
        if self.layout.search.h > 0.0 && self.editor.active().is_some_and(|d| d.diff.is_some()) {
            self.layout.search.y += crate::app::diff::DIFF_HEADER;
        }

        self.place_terminal();
        self.place_window_controls();
        self.place_preview_button();
        self.place_browser_button();
        self.place_split();
        self.place_minimap();

        // 侧栏显示搜索面板时没有文件树，点击、悬停和滚动不能落在文件树原来的区域上。
        if self.sidebar_view == SidebarView::Search {
            let tree = self.layout.tree;
            self.layout.tree = Rect::new(tree.x, tree.y, 0.0, 0.0);
        }
        self.sync_wrap();
        self.sync_terminal();
    }

    /// 当前文件能预览（SVG）时，在标签栏右端留出「源代码 / 分栏 / 预览」三个按钮，标签栏
    /// 让出这一块。
    ///
    /// 在可预览与不可预览的文件之间切换时，按钮组按比例从右侧展开或收起，标签栏随之让出或
    /// 收回空间。
    fn place_preview_button(&mut self) {
        let present = self.editor.active().is_some_and(|d| d.can_preview());
        let share = self.module_fx.preview.share(present);
        let l = &mut self.layout;
        l.preview = Rect::default();
        let full = crate::layout::PREVIEW_GROUP_W;
        if share <= 0.0 || l.tabs.w < full * 2.0 {
            return;
        }
        let (tabs, w) = (l.tabs, full * share);
        l.preview = Rect::new(
            tabs.right() - w,
            tabs.y + (tabs.h - BUTTON) / 2.0,
            w,
            BUTTON,
        );
        l.tabs.w -= (full + TAB_GAP) * share;
    }

    /// 当前文件是网页（`App::browser_page`）时，在标签栏右端留出「在浏览器中打开」按钮，标签栏
    /// 让出该处；出现、消失的动画同预览切换。
    fn place_browser_button(&mut self) {
        let share = self.module_fx.browser.share(self.browser_page().is_some());
        let l = &mut self.layout;
        l.browser = Rect::default();
        if share <= 0.0 || l.tabs.w < BUTTON * 4.0 {
            return;
        }
        let (tabs, w) = (l.tabs, BUTTON * share);
        l.browser = Rect::new(
            tabs.right() - w,
            tabs.y + (tabs.h - BUTTON) / 2.0,
            w,
            BUTTON,
        );
        l.tabs.w -= (BUTTON + TAB_GAP) * share;
    }

    /// 分栏预览中间的分隔线是否正被悬停或拖动。
    pub(crate) fn split_divider_active(&self) -> bool {
        self.hover == Hover::SplitDivider || matches!(self.drag, Drag::SplitDivider { .. })
    }

    /// 分栏预览：编辑区左右分开，左边是行号栏和文字，右边画预览，中间一道分隔线。
    ///
    /// 左边占的比例按 `split_share`（默认各占一半），两边都至少留 `SPLIT_MIN`；编辑区太窄、
    /// 两边都放不下 `SPLIT_MIN` 时不分，只显示源代码。
    fn place_minimap(&mut self) {
        // 编辑区窄于该宽度时不显示（逻辑像素，经验值）：优先留给代码。
        const MINIMAP_MIN: f32 = 480.0;
        // 缩略图占的宽度（逻辑像素，经验值），右边一段让给滚动条。
        const MINIMAP_W: f32 = 84.0;
        self.layout.minimap = Rect::default();
        let shows_code = self.editor.active().is_some_and(|doc| {
            doc.shows_text()
                && doc.diff.is_none()
                && doc.log.is_none()
                && doc.preview_mode() != winc_editor::PreviewMode::Preview
        });
        let l = &mut self.layout;
        if self.config.minimap != Some(true) || !shows_code || l.editor.w < MINIMAP_MIN {
            return;
        }
        l.text.w = (l.text.w - MINIMAP_W).max(0.0);
        l.minimap = Rect::new(
            l.text.right(),
            l.editor.y,
            MINIMAP_W - SCROLLBAR_HIT,
            l.editor.h,
        );
    }

    fn place_split(&mut self) {
        // 分栏时每一半的最小宽度（逻辑像素，经验值）。
        const SPLIT_MIN: f32 = 240.0;
        // 分隔线两侧各放宽的可拖动范围（逻辑像素，经验值）。
        const GRIP: f32 = 4.0;
        let share = self.split_share.unwrap_or(0.5);
        let l = &mut self.layout;
        l.split = Rect::default();
        l.split_divider = Rect::default();
        let split = self.editor.active().map(|d| d.preview_mode());
        if split != Some(winc_editor::PreviewMode::Split) || l.editor.w < SPLIT_MIN * 2.0 {
            return;
        }
        let left = (l.editor.w * share)
            .round()
            .clamp(SPLIT_MIN, l.editor.w - SPLIT_MIN);
        l.split_divider = Rect::new(
            l.editor.x + left - GRIP,
            l.editor.y,
            2.0 * GRIP + 1.0,
            l.editor.h,
        );
        let shrink = l.editor.w - left;
        l.split = Rect::new(
            l.editor.x + left + 1.0,
            l.editor.y,
            shrink - 1.0,
            l.editor.h,
        );
        l.editor.w = left;
        l.text.w = (l.text.w - shrink).max(0.0);
        // 查找条贴着编辑区右边。
        l.search.x -= shrink;
    }

    /// 终端面板开着时从编辑区卡片下面切出一张卡片：编辑区变矮，两张卡片之间留间距，间距里
    /// 可以上下拖动改面板高度。
    ///
    /// 放在右边时（`terminal.position`）从编辑区右边切出整个一栏，间距里左右拖动改面板宽度；
    /// 窗口放不下最小尺寸时面板不显示。动画里面板只露出一部分，这时间距不能拖。
    fn place_terminal(&mut self) {
        let (shown, right) = self.terminal_shown();
        let l = &mut self.layout;
        l.terminal = Rect::default();
        l.terminal_divider = Rect::default();
        if shown <= 0.0 {
            return;
        }
        let gap = if shown < 1.0 { 0.0 } else { ISLAND_GAP };
        if right {
            let room = l.editor.w - terminal::EDITOR_MIN_WIDTH - ISLAND_GAP;
            if room < terminal::MIN_WIDTH {
                return;
            }
            let w = match self.terminal.width {
                _ if self.terminal.maximized => room,
                w if w > 0.0 => w,
                _ => terminal::DEFAULT_WIDTH,
            };
            let w = w.clamp(terminal::MIN_WIDTH, room);
            let w = self
                .terminal
                .grow
                .map_or(w, |(from, c)| c.between(from, w))
                .round();
            let shrink = ((w + ISLAND_GAP) * shown).round();
            for rect in [&mut l.top_bar, &mut l.tabs, &mut l.editor, &mut l.text] {
                rect.w = (rect.w - shrink).max(0.0);
            }
            // 查找条贴着编辑区右边；⌘P 面板在编辑区里居中，放不下时变窄。
            l.search.x -= shrink;
            if l.picker.w > 0.0 {
                l.picker.w = l.picker.w.min(l.editor.w - 48.0).max(0.0);
                l.picker.x = (l.editor.x + (l.editor.w - l.picker.w) / 2.0).round();
            }
            let top = l.top_bar.y;
            let h = l.editor.bottom() - top;
            l.terminal = Rect::new(l.editor.right() + ISLAND_GAP, top, w, h);
            l.terminal_divider = Rect::new(l.editor.right(), top, gap, h);
            return;
        }
        let room = l.editor.h - terminal::EDITOR_MIN - ISLAND_GAP;
        if room < terminal::MIN_HEIGHT {
            return;
        }
        let h = match self.terminal.maximized {
            true => room,
            false => self.terminal.height.clamp(terminal::MIN_HEIGHT, room),
        };
        let h = self
            .terminal
            .grow
            .map_or(h, |(from, c)| c.between(from, h))
            .round();
        let shrink = ((h + ISLAND_GAP) * shown).round();
        for rect in [&mut l.editor, &mut l.gutter, &mut l.text] {
            rect.h = (rect.h - shrink).max(0.0);
        }
        l.terminal = Rect::new(l.editor.x, l.editor.bottom() + ISLAND_GAP, l.editor.w, h);
        l.terminal_divider = Rect::new(l.editor.x, l.editor.bottom(), l.editor.w, gap);
    }

    /// 编辑区每帧都先用不透明的 `editor_bg` 整个铺一遍（`view::editor::draw_editor`），告诉渲染器
    /// 那一块不必先铺窗口背景色。
    pub(in crate::app) fn cover_frame(&self, renderer: &mut winc_render::Renderer) {
        let (window, scale) = (self.layout.window, renderer.scale_factor());
        if self.settings_reuses_base(window, scale) {
            // 滚动设置页时由保存的快照覆盖整个窗口，无需先填充背景。
            renderer.cover_next_frame(window);
        } else if self.palette.editor_bg.a >= 1.0 {
            renderer.cover_next_frame(self.layout.editor);
        }
    }

    /// 绘制一帧并提交。
    ///
    /// 窗口看不见时跳过绘制；当前标签还在后台读取时保留上一帧；绘制后重新判断悬停目标，
    /// 并安排下一帧。
    pub(super) fn redraw(&mut self) {
        // 看不见时不画（`dirty` 留着），又看得见时再画。
        if self.unseen() {
            return;
        }
        // 切换标签的路径很多，个别没经过 `activate_tab` 的也在绘制之前开始读（见
        // `Document::pending`）。
        self.load_active_in_background();
        // 当前标签在读：先不画，屏幕上留着上一帧，读好了或到时再画（见 `App::holding_frame`）。
        if self.holding_frame(Instant::now()).is_some() {
            self.loads.held = true;
            self.dirty = false;
            return;
        }
        self.settle_animations();
        self.advance_wheel_glide();
        self.continue_drag_scroll();
        self.track_modules();
        self.track_search_rows();
        self.git_follow_active();
        self.relayout();
        self.run_when_loaded();
        self.advance_search();
        self.advance_images();

        self.highlight_pending = false;
        self.ime_cursor = None;
        // 此前的重画请求由这一帧满足；绘制之后（例如 `refresh_hover`）再提出的请求留到下一帧。
        self.dirty = false;

        let Some(mut renderer) = self.renderer.take() else {
            return;
        };
        let started = Instant::now();
        self.cover_frame(&mut renderer);
        let glide = self.begin_glide_frame();
        let drawn = renderer.draw(|frame| view::draw(self, frame));
        self.end_glide_frame(glide);
        let elapsed = started.elapsed();
        self.renderer = Some(renderer);
        if !drawn {
            self.retry_frame(started);
            return;
        }
        self.refresh_hover();

        // 高亮状态尚未推算完、大文件尚未搜完时，马上绘制下一帧，不按刷新率等：每帧本身有预算
        // （高亮 `HIGHLIGHT_BUDGET` 行、查找一个 8 ms 的时间片），已经不比刷新间隔短，按刷新率
        // 等反而常常错过一次刷新。
        let searching = self.search_open || self.pending_jump.is_some();
        self.dirty |= self.highlight_pending || (searching && !self.search.is_complete());
        // 终端查找同样按时间片进行：这一帧之后再找一个时间片，没找完就马上绘制下一帧。
        self.dirty |= self.terminal_find_tick();
        // 动画帧按显示器的刷新率安排。winit 在 macOS 上请求重画后立即绘制，不等待屏幕刷新，
        // 因此不能直接置 `dirty`，否则绘制频率会远高于刷新率。
        self.schedule_frame(started);
        self.update_ime_area();
        self.after_terminal_frame();
        self.presented += 1;

        match self.bench.on_present(self.presented, elapsed) {
            Next::Continue => {}
            // 应用层看到以后结束事件循环（测量时只有一个窗口）。
            Next::Exit => self.quit_requested = true,
            Next::Scroll => {
                let max = self.max_scroll_line();

                if let Some(doc) = self.editor.active_mut() {
                    doc.scroll_line = if doc.scroll_line + 3.0 > max {
                        0.0
                    } else {
                        doc.scroll_line + 3.0
                    };
                }

                self.request_redraw();
            }
            Next::Search(query) => {
                self.open_global_search(false);
                self.global.query.set_text(&query);
                self.global_changed(true);
            }
            Next::QuickOpen => self.open_picker(),
            Next::Find(query) => {
                self.open_search(false);
                self.find_input.set_text(&query);
                self.search.query = query;
                self.search_changed();
            }
            Next::Highlight => {
                let max = self.max_scroll_line();
                if let Some(doc) = self.editor.active_mut() {
                    doc.scroll_line = max;
                }
                self.request_redraw();
            }
        }
        let searching = self.search_open || self.pending_jump.is_some();
        let done = !self.highlight_pending && (!searching || self.search.is_complete());
        if self.bench.on_progress(done) {
            self.quit_requested = true;
        }
    }
}

/// 返回窗口所在显示器相邻两帧的间隔。
///
/// 查询不到刷新率，或刷新率低于 30 Hz 时按 60 Hz 计算。
pub(in crate::app) fn frame_interval(window: &Window) -> Duration {
    let millihertz = window
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .filter(|&mhz| mhz >= 30_000)
        .unwrap_or(60_000);
    Duration::from_micros(1_000_000_000 / u64::from(millihertz))
}
