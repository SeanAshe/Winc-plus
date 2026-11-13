//! 鼠标位置的目标判断：点击与悬停共用的目标判断、悬停高亮和指针样式。

use super::*;
use crate::app::diff::DIFF_HSCROLL;

impl App {
    /// 把顶栏当前的命中测试区域同步给窗口过程。
    ///
    /// 窗口过程只读取同步后的副本，不借用 [`App`]；系统拖动窗口的模态循环期间重入窗口过程时，
    /// 不会与应用状态的借用冲突。
    #[cfg(windows)]
    pub(crate) fn sync_window_input(&self) {
        if let Some(input) = &self.touch.window_input {
            input.update(self.window_hit_test());
        }
    }

    /// 返回提供给系统命中测试的顶栏区域划分。
    ///
    /// 系统在落指时即按命中测试的结果决定是否接管窗口拖动，因此该划分必须在落指前同步；
    /// 顶栏中的控件按客户区处理，接收常规输入事件。浮层或设置页打开、已有接触点时返回
    /// 空划分，整个窗口按客户区处理。
    #[cfg(windows)]
    pub(in crate::app) fn window_hit_test(&self) -> crate::windows::WindowHitTest {
        if self.touch_intercepted() || self.settings.open || self.touch.active() {
            return Default::default();
        }
        // 窗口过程只除以系统 DPI；布局区域需先乘应用缩放，与其输入坐标保持一致。
        let zoom = self.zoom();
        crate::windows::WindowHitTest {
            caption_height: TOP_BAR_END * zoom,
            resize_border: if self.window.controls {
                crate::app::lifecycle::title_bar::RESIZE_EDGE * zoom
            } else {
                0.0
            },
            control_gap: TAB_GAP * zoom,
            client: self
                .caption_controls()
                .into_iter()
                .map(|r| winc_render::Rect::new(r.x * zoom, r.y * zoom, r.w * zoom, r.h * zoom))
                .collect(),
        }
    }

    /// 是否位于顶栏的空白处：点 (`x`, `y`)（逻辑像素）在顶栏范围内，且不属于任何控件。
    pub(in crate::app) fn over_title_bar(&self, x: f32, y: f32) -> bool {
        matches!(self.target_at(x, y), Target::TitleBar)
    }

    /// 是否在双击顶栏空白处横坐标 `x`（逻辑像素）的位置时切换窗口的最大化。
    ///
    /// 位于某个控件所在的列时不切换，见 [`in_control_column`](crate::layout::in_control_column)；
    /// 相邻控件的间距不超过 `TAB_GAP`，按该宽度把间隙并入两侧的列。
    pub(in crate::app) fn title_bar_double_click_zooms(&self, x: f32) -> bool {
        !crate::layout::in_control_column(&self.caption_controls(), TAB_GAP, x)
    }

    /// 返回顶栏范围内按客户区处理的控件矩形（逻辑像素）。
    ///
    /// 结果为布局坐标；交给窗口过程前由 `window_hit_test` 乘以界面缩放倍数。
    /// 顶栏中其余部分由系统按标题栏处理。结果必须覆盖 `target_at` 在顶栏内返回的全部
    /// 控件目标（顶栏空白与窗口边缘除外），由测试 `caption_controls_cover_top_bar_targets`
    /// 检查。
    pub(in crate::app) fn caption_controls(&self) -> Vec<winc_render::Rect> {
        let l = &self.layout;
        let mut client = vec![
            l.sidebar_button,
            self.tabs.new_tab,
            l.terminal_divider,
            l.split_divider,
            l.search,
            self.hits.project,
            self.outline_divider(),
        ];
        // 不由应用绘制标题栏时没有窗口按钮，`window_control_rects` 返回的矩形不对应任何控件。
        if self.window.controls {
            client.extend(self.window_control_rects().map(|(r, _)| r));
        }
        client.extend(self.tabs.list);
        client.extend(
            self.hits
                .tabs
                .iter()
                .map(|h| h.tab.intersect(self.tabs.view)),
        );
        client.extend(self.sidebar_tabs().iter().map(|(r, _)| *r));
        client.extend(self.hits.tree_tools.iter().map(|(r, _)| *r));
        client.extend(self.hits.terminal.iter().map(|(r, _)| *r));
        client.extend(self.hits.merge.iter().map(|(r, _)| *r));
        client.extend(self.hits.crumbs.iter().map(|(r, _)| *r));
        client.extend(self.hits.status.iter().map(|(r, _)| *r));
        client.extend(
            [
                self.hits.changes_card,
                self.hits.notice_card,
                self.hits.notice_button,
                self.hits.merge_card,
                self.hits.conflict_card.as_ref().map(|(r, _)| *r),
                self.hits.tree_more,
            ]
            .into_iter()
            .flatten(),
        );
        if self.panels.sidebar.visible {
            client.push(l.splitter);
        }
        if self.editor.active().is_some_and(|d| d.can_preview()) {
            client.push(l.preview);
        }
        if self.browser_page().is_some() {
            client.push(l.browser);
        }
        client.retain(|r| r.w > 0.0 && r.h > 0.0 && r.y < TOP_BAR_END && r.bottom() > 0.0);
        client
    }

    /// 是否有对话框、弹出菜单等浮层打开；此时触摸由浮层接收，不开始窗口操作或控件拖动。
    fn touch_intercepted(&self) -> bool {
        self.overlays.alert.is_some()
            || self.popup().is_some()
            || self.overlays.form_dialog.is_some()
            || self.overlays.picker.open
            || self.panels.name_input.is_some()
    }

    /// 返回在 (`x`, `y`) 落指时立即开始的窗口操作：顶栏空白处移动窗口，窗口边缘调整大小。
    ///
    /// 窗口操作不参与内容滚动与长按菜单的识别。浮层或设置页打开时返回 `None`。
    pub(in crate::app) fn touch_window_at(
        &self,
        x: f32,
        y: f32,
    ) -> Option<crate::app::input::touch::WindowTouch> {
        use crate::app::input::touch::WindowTouch;
        if self.touch_intercepted() || self.settings.open {
            return None;
        }
        match self.target_at(x, y) {
            Target::TitleBar => Some(WindowTouch::Move),
            Target::ResizeEdge(direction) => Some(WindowTouch::Resize(direction)),
            _ => None,
        }
    }

    /// 是否在 (`x`, `y`) 落指后立即开始拖动，适用于滚动条、缩略图和分隔条。
    ///
    /// 列表、按钮和文字需要先区分点按与滑动，不在落指时开始拖动。
    pub(in crate::app) fn touch_drag_at(&self, x: f32, y: f32) -> bool {
        if self.touch_intercepted() {
            return false;
        }
        if self.settings.open {
            return self.settings_part_at(x, y).is_none()
                && self.settings_scrollbar_at(x, y).is_some();
        }
        if self
            .preview
            .markdown
            .diagram
            .as_ref()
            .is_some_and(|v| v.area.contains(x, y))
        {
            return false;
        }
        matches!(
            self.target_at(x, y),
            Target::Scrollbar(..)
                | Target::Minimap
                | Target::Splitter
                | Target::TerminalDivider
                | Target::OutlineDivider
                | Target::SplitDivider
                | Target::Log(crate::app::git::log::LogTarget::Divider)
        )
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处的悬停目标。
    pub(in crate::app) fn hover_at(&self, x: f32, y: f32) -> Hover {
        let l = &self.layout;
        // 窗口内的菜单位于最上层，打开期间下方的部件不产生悬停。
        if self.popup().is_some() {
            return Hover::None;
        }

        // 拖动滚动条时鼠标会经过文件树的行和编辑区，悬停保持在这根滚动条上，下方的内容不高亮。
        // 拖动文件树中的项或标签时没有悬停目标；文件树中高亮的是放置目标目录，而不是鼠标所在的行。
        match self.pointer.drag {
            Drag::Scrollbar { bar, .. } => return Hover::Scrollbar(bar),
            _ if self.tree_dragging().is_some() || self.dragged_tab().is_some() => {
                return Hover::None;
            }
            _ => {}
        }

        match self.target_at(x, y) {
            Target::WindowControl(control) => Hover::WindowControl(control),
            Target::ResizeEdge(direction) => Hover::ResizeEdge(direction),
            Target::NoticeButton => Hover::NoticeButton,
            Target::Change(target) => Hover::Change(target),
            Target::Merge(target) => Hover::Merge(target),
            Target::Conflict(action) => Hover::ConflictButton(action),
            Target::Picker
                if let Some(&(_, part)) =
                    (self.hits.picker_parts.iter()).find(|(rect, _)| rect.contains(x, y)) =>
            {
                Hover::PickerPart(part)
            }
            Target::Picker => crate::app::workspace::picker::picker_parts(self, l.picker)
                .row_at(y, self.overlays.picker.scroll)
                .filter(|&i| i < self.overlays.picker.hits.len())
                .map_or(Hover::None, Hover::PickerRow),
            Target::TreeMore => Hover::TreeMore,
            Target::Scrollbar(bar, _) => Hover::Scrollbar(bar),
            Target::Minimap => Hover::Minimap,
            Target::Panel(hover) => hover,
            Target::Search => SearchParts::new(l.search)
                .button_at(x, y)
                .map_or(Hover::None, Hover::Search),
            Target::Splitter => Hover::Splitter,
            Target::TreeTool(tool) => Hover::TreeTool(tool),
            Target::ProjectName => Hover::ProjectName,
            Target::SidebarButton => Hover::SidebarButton,
            Target::NewTab => Hover::NewTab,
            Target::TabList => Hover::TabList,
            Target::Preview(mode) => Hover::Preview(mode),
            Target::Browser => Hover::Browser,
            Target::MarkdownLink(link) => Hover::MarkdownLink(link),
            Target::MarkdownToc(target) => Hover::MarkdownToc(target),
            Target::MarkdownText | Target::PdfThumb(_) | Target::Picture => Hover::None,
            Target::PdfTool(tool) => Hover::PdfTool(tool),
            Target::Tab(index, on_close) => Hover::Tab(index, on_close),
            Target::RulerChange(hunk) => Hover::RulerChange(hunk),
            Target::Crumb(index) => Hover::Crumb(index),
            Target::Status(item) => Hover::Status(item),
            Target::Tree(row) => row.map_or(Hover::None, Hover::TreeRow),
            Target::Placeholder(action) => Hover::Placeholder(action),
            Target::Diff(target) => Hover::Diff(target),
            Target::Log(target) => Hover::Log(target),
            Target::Start(action) => Hover::Start(action),
            Target::Sticky(row) => Hover::Sticky(row.depth),
            Target::Folded(line) => Hover::Folded(line),
            Target::LongLine(line) => Hover::LongLine(line),
            Target::Blame(line) => Hover::Blame(line),
            Target::ChangeMark(hunk) => Hover::ChangeMark(hunk),
            Target::Gutter => Hover::Gutter,
            Target::TerminalDivider => Hover::TerminalDivider,
            Target::OutlineDivider => Hover::OutlineDivider,
            Target::SplitDivider => Hover::SplitDivider,
            Target::TerminalHeader(hit) => Hover::TerminalHeader(hit),
            Target::TerminalBlock(hit) => Hover::TerminalBlock(hit),
            // 遮挡下方但没有按钮的区域，以及没有悬停效果的部件：输入条、顶栏空白、编辑区文字。
            Target::Blocked
            | Target::Prompt
            | Target::TitleBar
            | Target::LogText
            | Target::Editor
            | Target::Terminal
            | Target::None => Hover::None,
        }
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处最上层的可交互目标。
    ///
    /// 悬停（[`App::hover_at`]）与左键按下（[`App::on_mouse_down`]）都以此为准。判断顺序与绘制
    /// 层次相反，上层的部件先判断；顺序只在此处定义，新增部件只需在此处放到正确的位置。
    ///
    /// 只依据位置，不区分按键；设置浮层、右键和中键由调用方先行处理。
    pub(super) fn target_at(&self, x: f32, y: f32) -> Target {
        let l = &self.layout;
        // 窗口最外圈的细边用于改变窗口大小，位于所有部件之上；窗口按钮位于顶栏右端。
        if let Some(direction) = self.resize_edge_at(x, y) {
            return Target::ResizeEdge(direction);
        }
        if let Some(control) = self.window_control_at(x, y) {
            return Target::WindowControl(control);
        }
        // 浮于编辑区之上的卡片和提示条：只响应其中的按钮，卡片的其余区域遮挡下方的部件。
        if let Some(card) = self.hits.changes_card
            && card.contains(x, y)
        {
            return hit(&self.hits.changes, x, y).map_or(Target::Blocked, Target::Change);
        }
        if self.hits.notice_button.is_some_and(|b| b.contains(x, y)) {
            return Target::NoticeButton;
        }
        if self
            .hits
            .notice_card
            .is_some_and(|card| card.contains(x, y))
        {
            return Target::Blocked;
        }
        // 合并冲突的按钮既位于卡片上，也位于冲突标记行上。
        if let Some(target) = hit(&self.hits.merge, x, y) {
            return Target::Merge(target);
        }
        if self.hits.merge_card.is_some_and(|card| card.contains(x, y)) {
            return Target::Blocked;
        }
        if let Some((card, buttons)) = &self.hits.conflict_card
            && card.contains(x, y)
        {
            return buttons
                .iter()
                .find(|(rect, _)| rect.contains(x, y))
                .map_or(Target::Blocked, |&(_, action)| Target::Conflict(action));
        }
        // 快速打开面板位于编辑区和侧栏之上。
        if self.overlays.picker.open && l.picker.contains(x, y) {
            return Target::Picker;
        }
        if self.hits.tree_more.is_some_and(|more| more.contains(x, y)) {
            return Target::TreeMore;
        }
        // 搜索结果、改动列表的滚动条位于行的右端，先于行判断。显示全名的那一改动行覆盖在滚动条
        // 之上：鼠标移到这一行的右端时全名保持显示，按下作用于这一行。
        let over_commit_row = self.over_commit_overflow_row(y);
        if let Some((bar @ (Scrollbar::Panel | Scrollbar::Commit | Scrollbar::Outline), grab)) =
            self.scrollbar_at(x, y)
            && !(over_commit_row && bar == Scrollbar::Commit)
        {
            return Target::Scrollbar(bar, grab);
        }
        // 大纲标题行的上沿跨在文件树和大纲之间，先于两者判定。
        if self.outline_divider().contains(x, y) {
            return Target::OutlineDivider;
        }
        if let Some(hover) = self.panel_hover_at(x, y) {
            return Target::Panel(hover);
        }
        // 跳转条绘制在查找条之上（窄窗口中两者会重叠）；正在输入名称的行位于文件树中。
        if (self.bars.goto_input.is_some() && self.prompt_bar().contains(x, y))
            || self.name_input_rect().is_some_and(|r| r.contains(x, y))
        {
            return Target::Prompt;
        }
        if l.search.contains(x, y) {
            return Target::Search;
        }
        // 名称显示不全而展开的行（文件树、改动列表）覆盖在分隔条上，该行优先；
        // 其余各行仍可拖动分隔条。
        let over_row = self.over_tree_overflow_row(y);
        if self.panels.sidebar.visible && !over_row && !over_commit_row && l.splitter.contains(x, y)
        {
            return Target::Splitter;
        }
        if let Some(tool) = hit(&self.hits.tree_tools, x, y) {
            return Target::TreeTool(tool);
        }
        if self.hits.project.contains(x, y) {
            return Target::ProjectName;
        }
        if l.sidebar_button.contains(x, y) {
            return Target::SidebarButton;
        }
        if self.tabs.new_tab.contains(x, y) {
            return Target::NewTab;
        }
        if self.tabs.list.is_some_and(|r| r.contains(x, y)) {
            return Target::TabList;
        }
        // 切换到不可预览的文件后，三段切换收起的动画过程中不响应点击。
        if l.preview.contains(x, y) && self.editor.active().is_some_and(|d| d.can_preview()) {
            use winc_editor::PreviewMode::*;
            let segments = crate::layout::preview_segments(l.preview);
            // 按钮之间的空隙归属于距离较近的一段，否则点击空隙时没有响应。
            let gap = |s: &winc_render::Rect| (s.x - x).max(x - s.right()).max(0.0);
            let at = (0..3)
                .min_by(|&a, &b| gap(&segments[a]).total_cmp(&gap(&segments[b])))
                .unwrap_or(0);
            return Target::Preview([Source, Split, Preview][at]);
        }
        // 切换到非网页文件后，按钮收起的动画过程中不响应点击。
        if l.browser.contains(x, y) && self.browser_page().is_some() {
            return Target::Browser;
        }
        if let Some((index, on_close)) = self.tab_at(x, y) {
            return Target::Tab(index, on_close);
        }
        // 概览标尺上的 Git 改动标记位于滑块左侧，先于滚动条判断。滚动条叠在文件树最下一行和
        // 编辑区右侧文字之上，在这些区域内按下时拖动的是滚动条。
        if let Some((hunk, _)) = self.ruler_change_at(x, y) {
            return Target::RulerChange(hunk);
        }
        // 展开的行覆盖文件树的滚动条，鼠标进入滚动条时该行不收起；展开的标题列表覆盖宽表格的
        // 水平滚动条，鼠标经过该滚动条时列表同样不收起。下方判定标题列表时沿用此处的命中结果。
        let toc = (self.preview.markdown.area.contains(x, y))
            .then(|| hit(&self.hits.markdown_toc, x, y))
            .flatten();
        let over_toc = toc.is_some();
        if let Some((bar, grab)) = self.scrollbar_at(x, y).filter(|(bar, _)| {
            !(over_row && *bar == Scrollbar::Tree
                || over_commit_row && *bar == Scrollbar::Commit
                || over_toc && *bar == Scrollbar::MarkdownTable)
        }) {
            return Target::Scrollbar(bar, grab);
        }
        if l.minimap.contains(x, y) {
            return Target::Minimap;
        }
        if let Some(index) = hit(&self.hits.crumbs, x, y) {
            return Target::Crumb(index);
        }
        if let Some(item) = hit(&self.hits.status, x, y) {
            return Target::Status(item);
        }
        // 终端面板位于右侧时，其标签栏和分隔间距位于顶栏所在的行，需要先于「按下拖动窗口」判断。
        if l.terminal_divider.contains(x, y) {
            return Target::TerminalDivider;
        }
        if l.split_divider.contains(x, y) {
            return Target::SplitDivider;
        }
        if let Some(hit) = hit(&self.hits.terminal, x, y) {
            return Target::TerminalHeader(hit);
        }
        if y < TOP_BAR_END {
            return Target::TitleBar;
        }
        if l.terminal.contains(x, y) {
            return match self.terminal_block_hit(x, y) {
                Some(hit) => Target::TerminalBlock(hit),
                None => Target::Terminal,
            };
        }
        if l.tree.contains(x, y) || self.in_tree_overflow(x, y) {
            return Target::Tree(self.tree_row_at(y));
        }
        if let Some(action) = hit(&self.hits.placeholder, x, y) {
            return Target::Placeholder(action);
        }
        if let Some(target) = hit(&self.hits.diff, x, y) {
            return Target::Diff(target);
        }
        if let Some(target) = hit(&self.hits.log, x, y) {
            return Target::Log(target);
        }
        if let Some(action) = hit(&self.hits.start, x, y) {
            return Target::Start(action);
        }
        if let Some(row) = self.sticky_at(x, y) {
            return Target::Sticky(row);
        }
        if let Some(line) = self.folded_at(x, y) {
            return Target::Folded(line);
        }
        if let Some(line) = hit(&self.hits.long_lines, x, y) {
            return Target::LongLine(line);
        }
        if let Some(line) = self.blame_at(x, y) {
            return Target::Blame(line);
        }
        if let Some(hunk) = self.change_mark_at(x, y) {
            return Target::ChangeMark(hunk);
        }
        if self.editor.active.is_some()
            && self.active_shows_text()
            && l.gutter.contains(x, y)
            && y >= l.text.y
        {
            return Target::Gutter;
        }
        if self.log_active() && self.log_parts().details.contains(x, y) {
            return Target::LogText;
        }
        // 标题指示条和展开的标题列表覆盖在预览文字之上，先于文字判断。
        if let Some(target) = toc {
            return Target::MarkdownToc(target);
        }
        // 可编辑的预览中单击文字用于放置光标，便于编辑链接文本；按住 `⌘`（其他平台为 `Ctrl`）
        // 点击才打开链接。复制按钮、折叠块的标题仍然单击生效。
        if let Some(link) = self.markdown_link_at(x, y)
            && (!self.markdown_live()
                || self.markdown_link_is_control(link)
                || if cfg!(target_os = "macos") {
                    self.pointer.modifiers.super_key()
                } else {
                    self.pointer.modifiers.control_key()
                })
        {
            return Target::MarkdownLink(link);
        }
        if let Some(tool) = hit(&self.hits.pdf_tools, x, y) {
            return Target::PdfTool(tool);
        }
        if self.preview.pdf.strip.contains(x, y) {
            let thumb = self
                .hits
                .pdf_thumbs
                .iter()
                .find(|(rect, _)| rect.contains(x, y));
            return thumb.map_or(Target::Blocked, |&(_, to)| Target::PdfThumb(to));
        }
        // 在预览上按下时选择文字并获得焦点，以便用键盘滚动和复制。
        if self.preview.markdown.area.contains(x, y) {
            return Target::MarkdownText;
        }
        if self.picture_at(x, y) {
            return Target::Picture;
        }
        if l.editor.contains(x, y) && self.editor.active.is_some() {
            return Target::Editor;
        }
        Target::None
    }

    /// 返回鼠标所在的可点击滚动条，以及此时按下后抓住滑块的位置。
    ///
    /// 抓取位置为到滑块起点的距离（逻辑像素）；点在滑块之外时抓住滑块中点，滑块跳到鼠标处。
    ///
    /// 滚动条紧贴区域的右边和下边，可点击范围为从该边向内 `SCROLLBAR_HIT`，宽于绘制的滑块。
    /// 只在区域内判断，区域外的标签和状态栏不受影响。内容无需滚动、没有滑块时返回 `None`。
    fn scrollbar_at(&self, x: f32, y: f32) -> Option<(Scrollbar, f32)> {
        let grab = |pos: f32, start: f32, len: f32| {
            if (start..start + len).contains(&pos) {
                pos - start
            } else {
                len / 2.0
            }
        };

        // 搜索视图的结果列表与文件树占用侧栏中的同一区域，先判断结果列表。
        if self.panel_shown() {
            let results = self.panel_parts().results;
            if results.contains(x, y) && x >= results.right() - SCROLLBAR_HIT {
                let content = (self.panels.global.rows().len() as f32 + 1.0) * TREE_ROW;
                let thumb = scrollbar_thumb(results, self.panels.global.scroll, content)?;
                return Some((Scrollbar::Panel, grab(y, thumb.y, thumb.h)));
            }
        }
        // 大纲的符号列表。
        if self.outline_shown() {
            let list = self.outline_list();
            if list.contains(x, y) && x >= list.right() - SCROLLBAR_HIT {
                let content = self.outline_max_scroll() + list.h;
                let thumb = scrollbar_thumb(list, self.panels.outline.scroll, content)?;
                return Some((Scrollbar::Outline, grab(y, thumb.y, thumb.h)));
            }
        }
        // Git 侧栏里正在显示的改动列表，或历史提交的文件列表。
        if self.log_diff_tree().is_some() {
            let list = self.commit_parts().list;
            if list.contains(x, y) && x >= list.right() - SCROLLBAR_HIT {
                let content = self.log_list_max_scroll() + list.h;
                let thumb = scrollbar_thumb(list, self.log_list().scroll, content)?;
                return Some((Scrollbar::Commit, grab(y, thumb.y, thumb.h)));
            }
        } else if self.commit_shown() {
            let list = self.commit_parts().list;
            if list.contains(x, y) && x >= list.right() - SCROLLBAR_HIT {
                let content = self.commit_max_scroll() + list.h;
                let thumb = scrollbar_thumb(list, self.git.commit.scroll, content)?;
                return Some((Scrollbar::Commit, grab(y, thumb.y, thumb.h)));
            }
        }

        let tree = self.layout.tree;
        if self.panels.sidebar.visible && tree.contains(x, y) {
            let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
            let content = (rows as f32 + 1.0) * TREE_ROW;

            if x >= tree.right() - SCROLLBAR_HIT
                && let Some(thumb) = scrollbar_thumb(tree, self.panels.tree.scroll, content)
            {
                return Some((Scrollbar::Tree, grab(y, thumb.y, thumb.h)));
            }
            if y >= tree.bottom() - SCROLLBAR_HIT
                && let Some(thumb) =
                    hscrollbar_thumb(tree, self.panels.tree.scroll_x, self.panels.tree.content_w)
            {
                return Some((Scrollbar::TreeHorizontal, grab(x, thumb.x, thumb.w)));
            }
            return None;
        }

        if let Some((track, scroll, content)) = self.terminal_scrollbar()
            && track.contains(x, y)
            && x >= track.right() - SCROLLBAR_HIT
        {
            let thumb = scrollbar_thumb(track, scroll, content)?;
            return Some((Scrollbar::Terminal, grab(y, thumb.y, thumb.h)));
        }

        // 预览（分栏时位于右侧，仅预览时占满编辑区）先于编辑区判断，仅预览时两者重叠。
        let preview = self.preview.markdown.area;
        if preview.contains(x, y) {
            // 比正文宽的表格的水平滚动条：位于表格显示区域的底边。
            if let Some(bar) = self.hits.markdown_table_bar
                && bar.track.contains(x, y)
                && y >= bar.track.bottom() - SCROLLBAR_HIT
                && let Some(thumb) =
                    hscrollbar_thumb(bar.track, self.markdown_table_scroll(bar.key), bar.content)
            {
                return Some((Scrollbar::MarkdownTable, grab(x, thumb.x, thumb.w)));
            }
            if x < preview.right() - SCROLLBAR_HIT {
                return None;
            }
            let scroll = self.preview.markdown.scroll();
            let thumb = scrollbar_thumb(preview, scroll, self.preview.markdown.height)?;
            return Some((Scrollbar::Markdown, grab(y, thumb.y, thumb.h)));
        }

        let strip = self.preview.pdf.strip;
        if strip.contains(x, y) {
            if x < strip.right() - SCROLLBAR_HIT {
                return None;
            }
            let (scroll, content) = (self.preview.pdf.strip_scroll, self.preview.pdf.strip_height);
            let thumb = scrollbar_thumb(strip, scroll, content)?;
            return Some((Scrollbar::PdfStrip, grab(y, thumb.y, thumb.h)));
        }
        let pdf = self.preview.pdf.area;
        if pdf.contains(x, y) {
            if x < pdf.right() - SCROLLBAR_HIT {
                return None;
            }
            let thumb = scrollbar_thumb(pdf, self.preview.pdf.scroll, self.preview.pdf.height)?;
            return Some((Scrollbar::Pdf, grab(y, thumb.y, thumb.h)));
        }

        let editor = self.scroll_track();
        let doc = self.editor.active()?;
        if editor.contains(x, y) && x >= editor.right() - SCROLLBAR_HIT {
            let scroll = (doc.scroll_line * f64::from(self.fonts.line_height)) as f32;
            let thumb = scrollbar_thumb(editor, scroll, self.content_height())?;
            return Some((Scrollbar::Editor, grab(y, thumb.y, thumb.h)));
        }

        if let Some(bars) = self.diff_hscroll() {
            return DIFF_HSCROLL
                .iter()
                .enumerate()
                .find_map(|(side, &(_, bar))| {
                    let (track, thumb) = bars.thumb(side, doc.scroll_x)?;
                    (track.contains(x, y) && y >= track.bottom() - SCROLLBAR_HIT)
                        .then(|| (bar, grab(x, thumb.x, thumb.w)))
                });
        }

        let track = self.text_hscroll_track();
        if track.contains(x, y)
            && y >= track.bottom() - SCROLLBAR_HIT
            && let Some(thumb) = hscrollbar_thumb(track, doc.scroll_x, self.text_content_w)
        {
            return Some((Scrollbar::EditorHorizontal, grab(x, thumb.x, thumb.w)));
        }

        None
    }

    /// 滚动条滑块是否以加深的颜色绘制：鼠标位于滑块上，或正在拖动滑块。
    pub(crate) fn scrollbar_active(&self, bar: Scrollbar) -> bool {
        self.pointer.hover == Hover::Scrollbar(bar)
            || matches!(self.pointer.drag, Drag::Scrollbar { bar: dragged, .. } if dragged == bar)
    }

    /// 纵坐标 `y` 是否位于正在显示完整名称的行上；该行位于最上层，覆盖分隔条和文件树的滚动条。
    ///
    /// 只有已展开的行优先。沿边缘上下移动到其他行时，该行尚未展开，滚动条正常响应。
    pub(in crate::app) fn over_tree_overflow_row(&self, y: f32) -> bool {
        self.panels
            .tree
            .overflow
            .is_some_and(|(rect, _)| (rect.y..rect.bottom()).contains(&y))
    }

    /// 窗口坐标 (`x`, `y`)（逻辑像素）是否位于文件树中显示不全的文件名
    /// 向右展开、覆盖到编辑区上方的区域内。
    pub(in crate::app) fn in_tree_overflow(&self, x: f32, y: f32) -> bool {
        self.panels
            .tree
            .overflow
            .is_some_and(|(rect, _)| rect.contains(x, y))
    }

    /// 按鼠标当前位置重新判断悬停目标和指针样式；每绘制完一帧调用一次。
    ///
    /// 悬停通常只在鼠标移动时判断。界面因按键而变化（`Esc` 关闭面板、提示条到时消失、输入使
    /// 文字滚动）而鼠标未动时，原悬停目标可能已不存在或已换成其他部件；不重新判断时，下次打开
    /// 面板会高亮上次的行。判断依据刚绘制完的一帧记录的各矩形；悬停变化时再绘制一帧后即稳定。
    pub(in crate::app) fn refresh_hover(&mut self) {
        if !self.pointer.cursor_inside || self.settings.open || self.overlays.form_dialog.is_some()
        {
            return;
        }
        let (x, y) = self.pointer.mouse;
        let hover = self.hover_at(x, y);
        if hover != self.pointer.hover {
            self.set_hover(hover);
            self.update_cursor_icon();
        }
        self.track_tree_segment();
    }

    /// `(x, y)` 是否位于编辑区右上角的卡片（冲突卡片、提示条）上。
    ///
    /// 卡片浮于文字之上，点击、右键和指针样式都不应作用于下方的文字。
    pub(in crate::app) fn over_corner_card(&self, x: f32, y: f32) -> bool {
        self.hits
            .notice_card
            .is_some_and(|card| card.contains(x, y))
            || self.hits.merge_card.is_some_and(|card| card.contains(x, y))
            || self
                .hits
                .conflict_card
                .as_ref()
                .is_some_and(|(card, _)| card.contains(x, y))
    }

    /// 根据鼠标位置和拖动状态更新鼠标指针的样式。
    pub(in crate::app) fn update_cursor_icon(&mut self) {
        let (x, y) = self.pointer.mouse;
        // 新建工作树对话框打开时只判断对话框自身：
        // 名称输入框上显示文本光标（I 形），其余位置显示箭头。
        if self.overlays.form_dialog.is_some() {
            let icon = if self.dialog_part_at(x, y) == Some(crate::layout::DialogPart::Branch) {
                CursorIcon::Text
            } else {
                CursorIcon::Default
            };
            self.set_cursor_icon(icon);
            return;
        }
        // 位于全屏查看的图片上时，工具栏按钮处显示手形指针，其余位置显示表示可拖动的抓手指针。
        if let Some(view) = &self.preview.markdown.diagram
            && (view.area.contains(x, y) || view.grab.is_some())
            && !self.settings.open
            && self.overlays.form_dialog.is_none()
        {
            let icon = if view.grab.is_some() {
                CursorIcon::Grabbing
            } else if view.hover.is_some() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Grab
            };
            self.set_cursor_icon(icon);
            return;
        }
        if self.settings.open {
            // 设置浮层打开时只判断浮层自身：字体名输入框、搜索框和正在编辑的输入框上显示文本光标，
            // 链接上显示手形，其余位置显示箭头。
            let editing = self.settings.editing.as_ref().map(|(s, _)| *s);
            let icon = match self.settings_part_at(x, y) {
                Some(SettingsPart::Field(s))
                    if s.control() == SettingControl::Font || editing == Some(s) =>
                {
                    CursorIcon::Text
                }
                Some(SettingsPart::KeySearch | SettingsPart::SearchField) => CursorIcon::Text,
                Some(SettingsPart::Link(_)) => CursorIcon::Pointer,
                _ => CursorIcon::Default,
            };
            self.set_cursor_icon(icon);
            return;
        }

        let in_text = self.editor.active.is_some()
            && self.active_shows_text()
            && !matches!(
                self.pointer.hover,
                Hover::TreeRow(_) | Hover::Sticky(_) | Hover::Folded(_) | Hover::LongLine(_)
            )
            && self.layout.text.contains(x, y)
            && !self.layout.search.contains(x, y)
            && !(self.overlays.picker.open && self.layout.picker.contains(x, y))
            && !self.over_corner_card(x, y)
            && x < self.layout.editor.right() - SCROLLBAR_HIT
            || self.over_diff_text(x, y) && x < self.layout.editor.right() - SCROLLBAR_HIT;
        // 日志详情中可选择的文字上（不含可点击的提交号和文件）显示文本光标。
        let in_field = self.field_at(x, y).is_some()
            || matches!(self.pointer.drag, Drag::LogText | Drag::MarkdownText)
            || (self.preview.markdown.area.contains(x, y) && self.markdown_link_at(x, y).is_none())
            || (self.log_active()
                && crate::layout::hit(&self.hits.log, x, y).is_none()
                && self.log_text_at(x, y, false).is_some());

        let icon = if let Hover::ResizeEdge(direction) = self.pointer.hover {
            resize_cursor(direction)
        } else if let Some(drag) = self.tree_dragging() {
            // 鼠标在文件树内但放置没有效果（目标在被拖项自身之内，或被拖项已在该目录中）时显示
            // 禁止图标；在文件树之外松开则不移动，显示普通箭头。
            if drag.target.is_none() && self.layout.tree.contains(x, y) {
                CursorIcon::NotAllowed
            } else {
                CursorIcon::Default
            }
        } else if matches!(self.pointer.drag, Drag::OutlineDivider { .. })
            || self.pointer.hover == Hover::OutlineDivider
        {
            CursorIcon::RowResize
        } else if matches!(self.pointer.drag, Drag::TerminalDivider { .. })
            || self.pointer.hover == Hover::TerminalDivider
        {
            // 面板位于下方时上下拖动，位于右侧时左右拖动。
            if self.terminal_right() {
                CursorIcon::ColResize
            } else {
                CursorIcon::RowResize
            }
        } else if matches!(
            self.pointer.drag,
            Drag::Splitter { .. } | Drag::LogDivider { .. } | Drag::SplitDivider { .. }
        ) || matches!(
            self.pointer.hover,
            Hover::Splitter
                | Hover::SplitDivider
                | Hover::Log(crate::app::git::log::LogTarget::Divider)
        ) {
            CursorIcon::ColResize
        } else if matches!(
            self.pointer.hover,
            Hover::Log(crate::app::git::log::LogTarget::Line(..))
                | Hover::TerminalBlock(_)
                | Hover::Diff(crate::app::DiffTarget::Sticky(..))
                | Hover::MarkdownLink(_)
                | Hover::MarkdownToc(crate::app::preview::markdown::TocTarget::Heading(_))
        ) || self.terminal.link.is_some()
        {
            // 提交图中的线可点击（沿线跳转）；终端中命令块的色带和吸顶的命令行同样可点击。
            CursorIcon::Pointer
        } else if matches!(self.pointer.drag, Drag::Picture { .. }) {
            CursorIcon::Grabbing
        } else if self.picture_pannable() && matches!(self.target_at(x, y), Target::Picture) {
            // 图片超出预览区域时可以按住拖动。
            CursorIcon::Grab
        } else if in_text || in_field {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };

        self.set_cursor_icon(icon);
    }

    fn set_cursor_icon(&mut self, icon: CursorIcon) {
        if icon != self.pointer.cursor_icon {
            self.pointer.cursor_icon = icon;

            if let Some(window) = &self.window.handle {
                window.set_cursor(icon);
            }
        }
    }
}

/// 窗口中某一点上最上层的可交互目标，见 [`App::target_at`]。
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    /// 遮挡下方但此处没有按钮的区域：提示条、卡片的空白处。
    ///
    /// 点击时不做任何操作，也不传给下方的部件。
    Blocked,
    NoticeButton,
    Change(crate::app::ChangeTarget),
    Merge(crate::app::git::merge::MergeTarget),
    Conflict(crate::app::editor::conflict::ConflictAction),
    /// 快速打开面板内。
    Picker,
    TreeMore,
    /// 滚动条，以及按下时抓住滑块的位置。
    Scrollbar(Scrollbar, f32),
    /// 侧栏中由视图切换、项目名、分支、搜索和提交面板处理的区域，参数为悬停目标。
    Panel(Hover),
    /// PDF 预览缩略图栏中的一页，参数为点击后正文滚动到的位置（逻辑像素）。
    PdfThumb(f32),
    /// PDF 预览右下角工具栏上的按钮。
    PdfTool(crate::app::preview::pdf::PdfTool),
    /// 跳转条，或正在输入名称的行。
    Prompt,
    Search,
    Splitter,
    TreeTool(crate::layout::TreeTool),
    ProjectName,
    SidebarButton,
    NewTab,
    /// 标签栏无法容纳全部标签时显示在最右端的「⌄」。
    TabList,
    /// 标签栏右端「源代码 / 分栏 / 预览」三段切换中的一段。
    Preview(winc_editor::PreviewMode),
    /// 标签栏右端「在浏览器中打开」。
    Browser,
    /// Markdown 预览中的链接，参数为其在本帧 `links` 中的下标。
    MarkdownLink(usize),
    MarkdownToc(crate::app::preview::markdown::TocTarget),
    /// Markdown 预览中链接以外的区域，用于选择文字。
    MarkdownText,
    /// 标签，参数为标签下标与是否位于关闭按钮上。
    Tab(usize, bool),
    /// 概览标尺上的改动，参数为改动块的序号。
    RulerChange(usize),
    /// 状态栏路径中的一段，参数为段号（从 0 起）。
    Crumb(usize),
    /// 状态栏右侧的一项。
    Status(crate::app::StatusItem),
    /// 顶栏空白处，按下后拖动窗口。
    TitleBar,
    /// 应用绘制的标题栏上的窗口按钮。
    WindowControl(WindowControl),
    /// 可拖动以改变窗口大小的边缘，参数为方向。
    ResizeEdge(winit::window::ResizeDirection),
    /// 文件树（含向右展开显示全名的区域），参数为鼠标所在的行。
    Tree(Option<usize>),
    Placeholder(crate::app::PlaceholderAction),
    Diff(crate::app::DiffTarget),
    Log(crate::app::git::log::LogTarget),
    Start(crate::app::StartAction),
    Sticky(crate::app::editor::sticky::StickyRow),
    /// 折叠处，参数为行号（从 0 起）。
    Folded(usize),
    /// 超长行截断说明中的「显示整行」，参数为行号（从 0 起）。
    LongLine(usize),
    /// 逐行追溯的标注，参数为行号（从 0 起）。
    Blame(usize),
    /// 行号栏上标出的改动，参数为改动块的序号。
    ChangeMark(usize),
    /// 行号栏（包含折叠箭头）。
    Gutter,
    /// 日志详情中的文字。
    LogText,
    Editor,
    /// 图片或 SVG 的预览区域。
    Picture,
    /// 终端面板。
    Terminal,
    /// 编辑区与终端面板之间的间距。
    TerminalDivider,
    /// 文件视图中大纲标题行的上沿。
    OutlineDivider,
    /// 分栏预览中间的分隔线。
    SplitDivider,
    /// 编辑区右侧的缩略图。
    Minimap,
    /// 终端面板标签栏上的按钮。
    TerminalHeader(crate::app::terminal::HeaderHit),
    /// 终端中命令块的色带与吸顶的命令行。
    TerminalBlock(crate::app::terminal::BlockHit),
    None,
}
