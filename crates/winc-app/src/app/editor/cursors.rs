//! 多光标与列选择的界面命令：在上下行添加光标、添加下一处匹配、选中全部匹配、在行尾添加光标、
//! 光标撤销、键盘列选择、把查找结果转为选区、只保留主光标，以及 `⌥` 单击添加光标、`⌥` 拖动
//! 添加选区、`⇧⌥` 拖动（或中键拖动）进行列选择。
//!
//! 多个选区的数据和在各选区上执行编辑命令见 `winc_editor` 的 `cursors` 模块。

use winc_i18n::t;

use super::*;

impl App {
    /// 在最上方光标的上一行添加光标（`⌥⌘↑`）；`down` 为真时在最下方光标的下一行添加（`⌥⌘↓`）。
    pub(in crate::app) fn add_cursor_vertically(&mut self, down: bool) {
        let tab_width = self.editor.active_tab_width();
        if let Some(doc) = self.text_document() {
            doc.add_cursor_vertically(down, tab_width);
            self.cursors_changed();
        }
    }

    /// 把下一处与主选区相同的文本加入选区（`⌘D`）；`skip` 为真时跳过主选区这一处（`⌘K ⌘D`）。
    pub(in crate::app) fn add_next_occurrence(&mut self, skip: bool) {
        let Some(doc) = self.text_document() else {
            return;
        };
        let result = if skip {
            doc.skip_to_next_occurrence()
        } else {
            doc.add_next_occurrence()
        };
        match result {
            Ok(true) => {}
            Ok(false) => self.flash(t!("editor-no-other-match")),
            Err(reason) => self.flash(reason),
        }
        self.cursors_changed();
    }

    /// 选中与主选区（或光标处的词）相同的全部文本（`⇧⌘L`）。
    pub(in crate::app) fn select_all_occurrences(&mut self) {
        let Some(doc) = self.text_document() else {
            return;
        };
        match doc.select_all_occurrences() {
            Ok(0) => {}
            Ok(count) if count >= winc_editor::MAX_MATCHES => {
                self.flash(t!("editor-too-many-matches", count = count));
            }
            Ok(_) => {}
            Err(reason) => self.flash(reason),
        }
        self.cursors_changed();
    }

    /// 在选中的每一行行尾添加光标（`⇧⌥I`）。
    pub(in crate::app) fn add_cursors_to_line_ends(&mut self) {
        if let Some(doc) = self.text_document() {
            doc.add_cursors_to_line_ends();
            self.cursors_changed();
        }
    }

    /// 撤销光标操作（`⌘U`）；`redo` 为真时重做光标操作（`⇧⌘U`）。
    pub(in crate::app) fn step_cursors(&mut self, redo: bool) {
        if let Some(doc) = self.text_document() {
            if redo {
                doc.redo_cursors();
            } else {
                doc.undo_cursors();
            }
            self.cursors_changed();
        }
    }

    /// 执行键盘列选择（`⇧⌥⌘` 加方向键）：选择框的终点横向移动 `dx` 个字符、纵向移动 `dy` 行。
    pub(in crate::app) fn extend_columns(&mut self, dx: isize, dy: isize) {
        let tab_width = self.editor.active_tab_width();
        if let Some(doc) = self.text_document() {
            doc.extend_columns(dx, dy, tab_width);
            self.cursors_changed();
        }
    }

    /// 查找条打开时（`⌥↩`）把全部查找结果转为选区，焦点回到编辑区，以便同时修改。
    ///
    /// 查找条未打开或查找内容为空时返回 `false`，按键交给编辑区。
    pub(in crate::app) fn select_all_matches(&mut self) -> bool {
        if !self.find.open || self.search.query.is_empty() || self.diff_active() {
            return false;
        }
        let Some(doc) = self
            .editor
            .active
            .and_then(|i| self.editor.documents.get_mut(i))
        else {
            return false;
        };
        if !doc.edits_text() {
            return false;
        }
        // 通常按时间片分段查找；此处需要全部结果，因此一次查找完毕。
        self.search.update(doc);
        while !self.search.is_complete() {
            self.search.update(doc);
        }
        let matches = self.search.matches();
        if matches.is_empty() {
            self.flash(t!("editor-no-match"));
            return true;
        }
        let from = doc.selection.from();
        let primary = matches.iter().rposition(|m| m.0 <= from).unwrap_or(0);
        let all = matches.iter().map(|&(a, b)| Selection::new(a, b)).collect();
        doc.record_cursors();
        doc.set_selections(all, primary);
        if self.search.is_truncated() {
            let count = winc_editor::MAX_MATCHES;
            self.flash(t!("editor-too-many-matches", count = count));
        }
        self.focus = Focus::Editor;
        self.cursors_changed();
        true
    }

    /// `⌥` 拖动：从按下处 `origin`（字符偏移）选到鼠标所在处，作为新增的选区；其余选区不变。
    pub(in crate::app) fn drag_add_selection(&mut self, x: f32, y: f32, origin: usize) {
        let t = self.layout.text;
        let scrolled = self.scroll_while_dragging(Some(x), y);
        if let Some(pos) = self.pos_at(x.clamp(t.x, t.right()), y.clamp(t.y, t.bottom() - 1.0))
            && let Some(doc) = self.editor.active_mut()
        {
            doc.set_primary(Selection::new(origin, pos));
        }
        if scrolled {
            self.clamp_scroll();
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 只保留主光标（`Esc`）；只有一个光标或改动卡片打开时不处理，返回 `false`，按键交给编辑区。
    pub(in crate::app) fn remove_secondary_cursors(&mut self) -> bool {
        if self.changes.popup.is_some() {
            return false;
        }
        let removed = self
            .editor
            .active_mut()
            .is_some_and(|doc| doc.collapse_selections());
        if removed {
            self.cursors_changed();
        }
        removed
    }

    /// `⌥` 单击：在 `pos`（字符偏移）处添加光标；点在已有的光标或选区上时移除该光标或选区。
    pub(in crate::app) fn toggle_cursor(&mut self, pos: usize) {
        if let Some(doc) = self.text_document() {
            doc.toggle_cursor(pos);
            doc.goal_cell = None;
        }
        self.cursors_changed();
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处的（行，显示列），用于列选择。
    ///
    /// 列按鼠标横坐标计算，可以超过行尾；位置在最后一行之下时取最后一行。
    /// 自动换行时按点击位置换算，列选择以整行为单位。
    pub(in crate::app) fn line_cell_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let doc = self.editor.active()?;
        let tab_width = self.editor.active_tab_width();
        if doc.is_wrapping() {
            let pos = self.pos_at(x, y)?;
            let (line, col) = doc.text().pos_to_line_col(pos);
            return Some((line, doc.col_to_cell(line, col, tab_width)));
        }
        let t = self.layout.text;
        let row = (f64::from((y - t.y) / self.fonts.line_height) + doc.scroll_line)
            .floor()
            .max(0.0) as usize;
        let line = doc.line_of_row(row.min(doc.row_count().saturating_sub(1)));
        let cell = ((x - (t.x + TEXT_PAD - doc.scroll_x)) / self.cell)
            .round()
            .max(0.0) as usize;
        Some((line, cell))
    }

    /// 处理列选择的拖动：从按下处 `anchor`（行，显示列）选到鼠标所在处；拖出边缘时自动滚动。
    pub(in crate::app) fn drag_columns(&mut self, x: f32, y: f32, anchor: (usize, usize)) {
        let t = self.layout.text;
        let scrolled = self.scroll_while_dragging(Some(x), y);
        let tab_width = self.editor.active_tab_width();
        if let Some(head) =
            self.line_cell_at(x.clamp(t.x, t.right()), y.clamp(t.y, t.bottom() - 1.0))
            && let Some(doc) = self.editor.active_mut()
        {
            doc.select_columns(anchor, head, tab_width);
        }
        self.tell_cursors_capped();
        if scrolled {
            self.clamp_scroll();
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 当前文档的选区刚因超过 [`winc_editor::MAX_CURSORS`] 被舍弃一部分时，在状态栏显示提示。
    fn tell_cursors_capped(&mut self) {
        if self
            .editor
            .active_mut()
            .is_some_and(|d| d.take_cursors_capped())
        {
            let count = winc_editor::MAX_CURSORS;
            self.flash(t!("editor-too-many-cursors", count = count));
        }
    }

    /// 返回显示文本的当前文档；没有或不显示文本（图片、差异等）时返回 `None`。
    fn text_document(&mut self) -> Option<&mut winc_editor::Document> {
        self.editor.active_mut().filter(|doc| doc.edits_text())
    }

    /// 选区变化后滚动到主光标，并使光标重新开始闪烁。
    fn cursors_changed(&mut self) {
        self.tell_cursors_capped();
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }
}
