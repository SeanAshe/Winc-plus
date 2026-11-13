//! 滚动的目标：按位置确定滚轮、触控板和触摸惯性滚动的区域，并按区域滚动、限制范围。
//!
//! 判断（[`App::scroll_target_at`]）与滚动（[`App::scroll_target_by`]）分开：滚轮过渡和惯性在
//! 开始时记下目标，此后各帧按记下的目标滚动，不随鼠标的移动改变。

use super::*;

use crate::app::workspace::picker::picker_span;

/// 滚轮、触控板和触摸惯性滚动的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollTarget {
    /// 窗口内的提示框打开时不滚动任何内容。
    Blocked,
    /// 窗口内的菜单：滚动鼠标所在的一级。
    Popup,
    /// 对话框打开时只滚动交互式变基的列表，下方的内容不滚动。
    Dialog,
    /// 设置浮层的分组区；`false` 表示浮层打开而位置不在分组区上，不滚动。
    Settings(bool),
    /// 终端查找条；`true` 表示位于输入框上，文字过长时横向滚动输入框。
    TerminalFind(bool),
    /// `⌘P` 面板的列表。
    Picker,
    /// 编辑区的标签栏。
    Tabs,
    /// 日志中历史差异的文件列表（借用提交视图的列表）。
    LogList,
    /// 提交视图的提交信息框。
    CommitMessage,
    /// 提交视图的更改列表。
    Commit,
    /// 文件视图底部的大纲列表。
    Outline,
    /// 全局搜索的结果列表。
    Results,
    /// 文件树。
    Tree,
    /// 日志的详情一栏。
    LogDetails,
    /// 终端面板的标签条。
    TerminalStrip,
    /// 终端的回滚缓冲，或终端中接收滚轮的程序。
    Terminal,
    /// Markdown 预览。
    Markdown,
    /// PDF 预览的缩略图栏。
    PdfStrip,
    /// PDF 预览的正文。
    Pdf,
    /// 图片预览。
    Picture,
    /// 表格预览。
    Table,
    /// 编辑区中显示的文档。
    Editor(DocumentId),
    /// 不在任何可滚动的区域上。
    Nothing,
}

impl ScrollTarget {
    /// 是否为滚动后鼠标下方的行随之变化、需要重新判断悬停目标的列表。
    fn moves_rows(self) -> bool {
        matches!(
            self,
            Self::Picker
                | Self::Tabs
                | Self::LogList
                | Self::Commit
                | Self::Outline
                | Self::Results
                | Self::Tree
                | Self::TerminalStrip
        )
    }
}

impl App {
    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处的滚动目标。
    ///
    /// 只做判断，不改变任何状态。浮层与对话框优先于下方的区域；三栏合并的两侧按中间的文档
    /// 处理。
    pub(crate) fn scroll_target_at(&self, x: f32, y: f32) -> ScrollTarget {
        let l = self.layout;
        if self.overlays.alert.is_some() {
            return ScrollTarget::Blocked;
        }
        if self.popup().is_some() {
            return ScrollTarget::Popup;
        }
        if self.overlays.form_dialog.is_some() {
            return ScrollTarget::Dialog;
        }
        let x = if self.in_merge_pane(x, y) {
            l.text.x
        } else {
            x
        };
        if self.settings.open {
            return ScrollTarget::Settings(self.settings_layout().body.contains(x, y));
        }
        if !self.overlays.picker.open
            && self
                .terminal_find_bar()
                .is_some_and(|bar| bar.contains(x, y))
        {
            return ScrollTarget::TerminalFind(self.field_at(x, y) == Some(Focus::TerminalFind));
        }
        let in_list =
            |app: &App| app.commit_parts().list.contains(x, y) || app.in_commit_overflow(x, y);
        let terminal_header = l.terminal.y + crate::app::terminal::header_height(l.terminal);
        if self.overlays.picker.open && l.picker.contains(x, y) {
            ScrollTarget::Picker
        } else if l.tabs.contains(x, y) {
            ScrollTarget::Tabs
        } else if self.log_diff_tree().is_some() && in_list(self) {
            ScrollTarget::LogList
        } else if self.commit_shown() && self.commit_parts().message.contains(x, y) {
            ScrollTarget::CommitMessage
        } else if self.commit_shown() && in_list(self) {
            ScrollTarget::Commit
        } else if self.outline_shown() && self.outline_list().contains(x, y) {
            ScrollTarget::Outline
        } else if self.panel_shown() && l.sidebar.contains(x, y) {
            ScrollTarget::Results
        } else if l.sidebar.contains(x, y) || self.in_tree_overflow(x, y) {
            ScrollTarget::Tree
        } else if self.log_active() && self.log_parts().details.contains(x, y) {
            ScrollTarget::LogDetails
        } else if l.terminal.contains(x, y) && y < terminal_header {
            ScrollTarget::TerminalStrip
        } else if l.terminal.contains(x, y) {
            ScrollTarget::Terminal
        } else if self.preview.markdown.area.contains(x, y) {
            ScrollTarget::Markdown
        } else if self.preview.pdf.strip.contains(x, y) {
            ScrollTarget::PdfStrip
        } else if self.preview.pdf.area.contains(x, y) {
            ScrollTarget::Pdf
        } else if self.picture_at(x, y) {
            ScrollTarget::Picture
        } else if self.table_at(x, y) {
            ScrollTarget::Table
        } else if l.editor.contains(x, y)
            && let Some(doc) = self.editor.active()
        {
            ScrollTarget::Editor(doc.id)
        } else {
            ScrollTarget::Nothing
        }
    }

    /// 按目标 `target` 滚动 `dx`、`dy`（逻辑像素，向右、向下滚动为负）。
    ///
    /// `origin` 是确定目标时的位置，用于需要位置的区域（菜单的层级、表格、Markdown 中的宽表格）；
    /// `shift` 表示竖直滚动按横向处理（按住 `Shift` 转动鼠标滚轮）。滚动后按鼠标的当前位置重新
    /// 判断悬停目标。
    pub(crate) fn scroll_target_by(
        &mut self,
        target: ScrollTarget,
        origin: (f32, f32),
        shift: bool,
        (dx, dy): (f32, f32),
    ) {
        let (x, y) = origin;
        // 可以横向滚动的区域按住 `Shift` 时改为横向滚动；列表仍按竖直方向滚动。
        let (sx, sy) = if shift { (dy, 0.0) } else { (dx, dy) };
        // 竖直滚动同样用于横向排列的标签栏、标签条和输入框。
        let along = if dx.abs() > dy.abs() { dx } else { dy };
        match target {
            ScrollTarget::Blocked | ScrollTarget::Settings(false) | ScrollTarget::Nothing => {
                return;
            }
            ScrollTarget::Popup => {
                // 菜单按鼠标位置选择滚动的层级。
                let mouse = std::mem::replace(&mut self.pointer.mouse, origin);
                self.popup_wheel(dy);
                self.pointer.mouse = mouse;
            }
            ScrollTarget::Dialog => self.rebase_wheel(dy),
            ScrollTarget::Settings(true) => self.settings_scroll_by(dy),
            ScrollTarget::TerminalFind(on_field) => {
                if let Some(find) = self.terminal.find.as_mut().filter(|_| on_field) {
                    find.field.wheel_scroll(-along);
                }
            }
            ScrollTarget::Picker if self.overlays.picker.places.is_some() => self.step_places(dy),
            ScrollTarget::Tabs => self.scroll_pane_by(target, along),
            ScrollTarget::LogList => {
                self.with_log_list(|app| app.scroll_pane_by(ScrollTarget::Commit, dy));
            }
            ScrollTarget::CommitMessage => {
                // 上限在绘制时按文字的行数收紧。
                let view = &mut self.git.commit.message.view;
                view.scroll = (view.scroll - dy).max(0.0);
            }
            ScrollTarget::Tree => {
                self.scroll_pane_by(target, sy);
                // 斜向滚动只取主方向，竖直滚动时不同时产生左右位移。
                if sx != 0.0 && (sy == 0.0 || sx.abs() > sy.abs()) {
                    let max_x = self.tree_max_scroll_x();
                    self.panels.tree.scroll_x = (self.panels.tree.scroll_x - sx).clamp(0.0, max_x);
                }
            }
            ScrollTarget::TerminalStrip => self.terminal.strip.wheel(along),
            ScrollTarget::Terminal => self.terminal_wheel(dy),
            ScrollTarget::Markdown => {
                // 横向滚动落在比正文宽的表格上时滚动该表格，其余按竖直方向滚动预览。
                let table =
                    sx != 0.0 && crate::app::preview::markdown::laid::scroll_table(self, x, y, sx);
                if !table {
                    self.scroll_pane_by(target, dy);
                }
            }
            ScrollTarget::PdfStrip => self.scroll_pdf_strip(dy),
            ScrollTarget::Pdf => self.scroll_pdf_by(sx, sy),
            ScrollTarget::Picture => self.scroll_picture_by(sx, sy),
            ScrollTarget::Table => self.scroll_table(sx, sy, origin),
            ScrollTarget::Editor(id) => self.scroll_editor_by_pixels(id, sx, sy),
            ScrollTarget::Picker
            | ScrollTarget::Commit
            | ScrollTarget::Outline
            | ScrollTarget::Results
            | ScrollTarget::LogDetails => self.scroll_pane_by(target, dy),
        }
        if target.moves_rows() {
            self.set_hover(self.hover_at(self.pointer.mouse.0, self.pointer.mouse.1));
        }
        self.request_redraw();
    }

    /// 位置浮窗按整行滚动 `dy`（逻辑像素），始终只显示完整的行。
    ///
    /// 滚动量累计满一行才滚动一行，触控板的小幅滑动不会使行只显示一部分。
    fn step_places(&mut self, dy: f32) {
        self.overlays.picker.places_wheel += dy;
        let steps = (self.overlays.picker.places_wheel / PICKER_ROW).trunc();
        self.overlays.picker.places_wheel -= steps * PICKER_ROW;
        let max = (self.pane_max(ScrollTarget::Picker) / PICKER_ROW).ceil() * PICKER_ROW;
        self.overlays.picker.scroll =
            (self.overlays.picker.scroll - steps * PICKER_ROW).clamp(0.0, max);
    }

    /// 把区域 `target` 的滚动位置减少 `delta`（逻辑像素），限制在有效范围内。
    fn scroll_pane_by(&mut self, target: ScrollTarget, delta: f32) {
        if let Some(scroll) = self.pane_scroll(target) {
            self.set_pane_scroll(target, scroll - delta);
        }
    }

    /// 返回区域 `target` 的竖直滚动位置（逻辑像素）；`target` 不是只保存一个竖直滚动位置的区域
    /// （列表、文件树、标签栏、预览），或该区域当前没有内容时返回 `None`。
    pub(crate) fn pane_scroll(&self, target: ScrollTarget) -> Option<f32> {
        match target {
            ScrollTarget::Picker => Some(self.overlays.picker.scroll),
            ScrollTarget::Tabs => Some(self.tabs.scroll),
            ScrollTarget::Commit => Some(self.git.commit.scroll),
            ScrollTarget::Outline => Some(self.panels.outline.scroll),
            ScrollTarget::Results => Some(self.panels.global.scroll),
            ScrollTarget::Tree => Some(self.panels.tree.scroll),
            ScrollTarget::LogDetails => {
                let view = self.editor.active().and_then(|d| d.log.as_deref());
                view.map(|view| view.details_scroll)
            }
            ScrollTarget::Markdown => self
                .preview
                .markdown
                .doc
                .map(|_| self.preview.markdown.scroll()),
            ScrollTarget::Pdf => Some(self.preview.pdf.scroll),
            _ => None,
        }
    }

    /// 返回区域 `target` 的最大竖直滚动位置（逻辑像素），不小于 0。
    pub(crate) fn pane_max(&self, target: ScrollTarget) -> f32 {
        let max = match target {
            ScrollTarget::Picker => {
                let rows = self.overlays.picker.hits.len() as f32;
                rows * PICKER_ROW - picker_span(self)
            }
            ScrollTarget::Tabs => self.tabs.max_scroll(),
            ScrollTarget::Commit => self.commit_max_scroll(),
            ScrollTarget::Outline => self.outline_max_scroll(),
            ScrollTarget::Results => self.results_max_scroll(),
            ScrollTarget::Tree => {
                let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
                tree_max_scroll(self.layout.tree, rows)
            }
            ScrollTarget::LogDetails => self.git.log.details_max,
            ScrollTarget::Markdown => self.preview.markdown.max_scroll(),
            ScrollTarget::Pdf => self.preview.pdf.max_scroll(),
            _ => 0.0,
        };
        max.max(0.0)
    }

    /// 把区域 `target` 的竖直滚动位置设为 `scroll`（逻辑像素），限制在 0 到 [`Self::pane_max`]
    /// 之间。
    ///
    /// Markdown 预览分栏显示时，左侧文字随之滚动到对应的位置（见 `App::scroll_markdown_to`）。
    pub(crate) fn set_pane_scroll(&mut self, target: ScrollTarget, scroll: f32) {
        let scroll = scroll.clamp(0.0, self.pane_max(target));
        match target {
            ScrollTarget::Markdown => self.scroll_markdown_to(scroll),
            _ => self.set_pane_raw(target, scroll),
        }
    }

    /// 把区域 `target` 的竖直滚动位置设为 `scroll`，不限制范围，也不同步其他区域。
    ///
    /// 供绘制前换入过渡中的位置、绘制后恢复目标位置使用（见 [`glide`](super::glide)）。
    pub(crate) fn set_pane_raw(&mut self, target: ScrollTarget, scroll: f32) {
        match target {
            ScrollTarget::Picker => self.overlays.picker.scroll = scroll,
            ScrollTarget::Tabs => self.tabs.scroll = scroll,
            ScrollTarget::Commit => self.git.commit.scroll = scroll,
            ScrollTarget::Outline => self.panels.outline.scroll = scroll,
            ScrollTarget::Results => self.panels.global.scroll = scroll,
            ScrollTarget::Tree => self.panels.tree.scroll = scroll,
            ScrollTarget::LogDetails => {
                let view = self.editor.active_mut().and_then(|d| d.log.as_deref_mut());
                if let Some(view) = view {
                    view.details_scroll = scroll;
                }
            }
            ScrollTarget::Markdown => self.preview.markdown.set_scroll(scroll),
            ScrollTarget::Pdf => self.preview.pdf.scroll = scroll,
            _ => {}
        }
    }
}
