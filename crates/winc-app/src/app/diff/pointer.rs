//! 差异标签中的鼠标操作：右键菜单、点击、拖选、列选择，以及判断鼠标下方是哪一侧的文字。

use super::*;

impl App {
    /// 处理差异文字上的右键点击，返回差异标签以及是否为右侧；不在文字上时返回 `None`。
    ///
    /// 点在选区外时先把光标移到点击处；点在选区内时保留选区，菜单作用于该选区。
    pub(in crate::app) fn diff_context_target(
        &mut self,
        x: f32,
        y: f32,
    ) -> Option<(DocumentId, bool)> {
        if !self.over_diff_text(x, y) || self.layout.search.contains(x, y) {
            return None;
        }
        let index = self.editor.active?;
        let id = self.editor.documents[index].id;
        self.focus = Focus::Editor;
        self.preedit.clear();
        if !self.over_diff_editable(x, y) {
            let view = self.editor.documents[index].diff.as_deref()?;
            let parts = diff_parts(self.layout.editor, view, self.cell);
            let side = match self.diff_unified_side_at(y) {
                Some(side) => side,
                None if parts.panes[RIGHT].text.contains(x, y) => RIGHT,
                None => LEFT,
            };
            let pos = self.diff_read_pos_at(side, x, y);
            let view = self.editor.documents[index].diff.as_deref_mut()?;
            view.editing = false;
            if let Some(pos) = pos {
                let inside = view.read_selection.is_some_and(|s| {
                    let (a, b) = s.range;
                    s.side == side && (a.min(b)..=a.max(b)).contains(&pos)
                });
                if !inside {
                    let range = (pos, pos);
                    view.read_selection = Some(ReadSelection { side, range });
                }
            }
            self.reset_blink();
            self.request_redraw();
            return Some((id, false));
        }
        let (source, pos) = self.diff_pos_at(x, y, true)?;
        let doc = &mut self.editor.documents[source];
        let inside = doc
            .selections()
            .0
            .iter()
            .any(|s| (s.from()..=s.to()).contains(&pos));
        if !inside {
            doc.collapse_selections();
            doc.selection = Selection::point(pos);
            doc.goal_cell = None;
        }
        let view = self.editor.documents[index].diff.as_deref_mut()?;
        view.editing = true;
        view.read_selection = None;
        self.reset_blink();
        self.request_redraw();
        Some((id, true))
    }

    /// 返回光标所在的右侧行号；光标不在右侧时为 `None`。
    pub(super) fn diff_caret_line(&self, index: usize) -> Option<usize> {
        let view = self.editor.documents.get(index)?.diff.as_deref()?;
        if !view.editing {
            return None;
        }
        let path = view.new.path.as_deref()?;
        let source = self
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path))?;
        Some(source.text().pos_to_line_col(source.selection.head).0)
    }

    /// 返回窗口坐标 (`x`, `y`) 在右侧可编辑文字上对应的右侧文档标签、行和显示列。
    ///
    /// 位于最后一行下方时取最后一行，`past_end` 为 `true`。`open` 为 `true` 时，
    /// 文件尚未打开则在后台打开。
    fn diff_line_cell_at(&mut self, x: f32, y: f32, open: bool) -> Option<DiffSpot> {
        let index = self.editor.active?;
        let doc = self.editor.documents.get(index)?;
        let view = doc.diff.as_deref()?;
        let path = view.new.path.clone()?;
        let parts = diff_parts(self.layout.editor, view, self.cell);
        if y < parts.body.y {
            return None;
        }
        let lh = self.fonts.line_height;
        let below = f64::from((y - parts.body.y) / lh);
        let visible = f64::from(parts.body.h / lh);
        let row = view.row_under(RIGHT, doc.scroll_line.max(0.0), visible, below);
        let (line, past_end) = match view.rows[RIGHT].get(row) {
            Some(SideRow::Line { line, .. }) => (*line, false),
            Some(SideRow::Fold { .. }) => return None,
            None => (usize::MAX, true),
        };
        let cell = self.diff_cell_at(view, RIGHT, row, x);
        let source = self.diff_source(&path, open)?;
        let lines = self.editor.documents[source].text().line_count();
        Some(DiffSpot {
            source,
            line: line.min(lines.saturating_sub(1)),
            cell,
            past_end,
        })
    }

    /// 返回窗口坐标 (`x`, `y`) 在右侧可编辑文字上对应的右侧文档标签和位置（字符偏移）。
    ///
    /// 位于最后一行下方时取文本末尾。
    ///
    /// `open` 同 [`App::diff_line_cell_at`]。
    fn diff_pos_at(&mut self, x: f32, y: f32, open: bool) -> Option<(usize, usize)> {
        let spot = self.diff_line_cell_at(x, y, open)?;
        let tab_width = self.editor.active_tab_width();
        let doc = &self.editor.documents[spot.source];
        if spot.past_end {
            return Some((spot.source, doc.text().end_pos()));
        }
        let col = doc.cell_to_col(spot.line, spot.cell, tab_width);
        Some((spot.source, doc.text().line_col_to_pos(spot.line, col)))
    }

    /// 处理差异文字上的点击，返回是否已处理。
    ///
    /// 行为同编辑区：单击放置光标（`⇧` 单击扩大选区），双击选词，三击或点击行号选整行，
    /// 按住后继续拖动时按相同单位扩展。左侧只读，选中后可以复制；右侧不是工作区文件时同样只读。
    /// 右侧是工作区文件时可以编辑，之后输入的文字写入右侧文件；`⌥` 单击添加或移除光标，
    /// `⌥` 拖动添加一个选区，`⇧⌥` 拖动进行列选择。点在选区外时收起选区。
    pub(in crate::app) fn click_diff(&mut self, x: f32, y: f32) -> bool {
        let Some(index) = self.editor.active else {
            return false;
        };
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return false;
        };
        let parts = diff_parts(self.layout.editor, view, self.cell);
        let editable = view.new.path.is_some();
        self.focus = Focus::Editor;
        // 点击后光标不再停在跳转到的那一段，因此清除当前段标记。
        if let Some(view) = self.editor.documents[index].diff.as_deref_mut() {
            view.focus = None;
        }
        let count = self.click_count(x, y);
        let (shift, alt) = (self.modifiers.shift_key(), self.modifiers.alt_key());

        let right = parts.panes[RIGHT];
        let on_pane = |pane: &DiffPane| pane.text.contains(x, y) || pane.gutter.contains(x, y);
        let on_left = on_pane(&parts.panes[LEFT]);
        let on_right = on_pane(&right);
        // 单栏的两列行号共用一个文字区：所属侧由该行决定，与鼠标所在的列无关。
        let (read_side, on_right) = match self.diff_unified_side_at(y) {
            Some(side) if on_left || on_right => {
                ((side == LEFT || !editable).then_some(side), true)
            }
            Some(_) => (None, false),
            None if on_left => (Some(LEFT), on_right),
            None => ((!editable && on_right).then_some(RIGHT), on_right),
        };
        let on_gutter = parts.panes[LEFT].gutter.contains(x, y) || right.gutter.contains(x, y);
        if let Some(side) = read_side {
            let unit = if on_gutter {
                SelectUnit::Line
            } else {
                SelectUnit::of(count)
            };
            let pos = self.diff_read_pos_at(side, x, y);
            if let Some(view) = self.editor.documents[index].diff.as_deref_mut() {
                view.editing = false;
                if let Some(pos) = pos {
                    let held = view.read_selection.filter(|s| s.side == side);
                    let origin = match held {
                        Some(held) if shift && unit == SelectUnit::Char => {
                            (held.range.0, held.range.0)
                        }
                        _ => unit.around(&view.side(side).text, pos),
                    };
                    let range = unit.extend(&view.side(side).text, origin, pos);
                    view.read_selection = Some(ReadSelection { side, range });
                    self.drag = crate::app::Drag::DiffSelectRead { side, unit, origin };
                }
            }
            self.reset_blink();
            self.request_redraw();
            return true;
        }
        if let Some(view) = self.editor.documents[index].diff.as_deref_mut() {
            view.read_selection = None;
        }

        if !(editable && on_right) {
            if let Some(view) = self.editor.documents[index].diff.as_deref_mut() {
                view.editing = false;
            }
            self.request_redraw();
            return true;
        }
        if alt && shift && !on_gutter {
            self.start_diff_columns(x, y);
            return true;
        }
        let Some((source, pos)) = self.diff_pos_at(x, y, true) else {
            return true;
        };
        let unit = if on_gutter {
            SelectUnit::Line
        } else {
            SelectUnit::of(count)
        };
        let doc = &mut self.editor.documents[source];
        doc.goal_cell = None;
        let mut drag = true;
        let add = alt && !on_gutter;
        if add {
            // `⌥` 单击添加或移除一个光标；点在已有光标上时移除该光标，不继续拖动。
            if count == 1 {
                let before = doc.selections().0.len();
                doc.record_cursors();
                doc.toggle_cursor(pos);
                drag = doc.selections().0.len() > before;
            } else {
                drag = false;
            }
        } else {
            // 普通点击恢复为单光标；点击前的光标可以用 ⌘U 恢复。
            doc.record_cursors();
            doc.collapse_selections();
        }
        let origin = match unit {
            SelectUnit::Char if shift && !add => (doc.selection.anchor, doc.selection.anchor),
            _ => unit.around(doc.text(), pos),
        };
        if !add {
            let (anchor, head) = unit.extend(doc.text(), origin, pos);
            doc.selection = Selection::new(anchor, head);
        }
        if let Some(view) = self.editor.documents[index].diff.as_deref_mut() {
            view.editing = true;
        }
        self.preedit.clear();
        if drag {
            self.drag = crate::app::Drag::DiffSelect { unit, origin, add };
        }
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 拖选到差异的上下边缘之外时滚动一行；返回限制在显示行范围内的纵坐标。
    fn diff_drag_scroll(&mut self, y: f32) -> f32 {
        let editor = self.layout.editor;
        let top = editor.y + DIFF_HEADER;
        if let Some(doc) = self.editor.active_mut() {
            if y < top {
                doc.scroll_line -= 1.0;
            } else if y > editor.bottom() {
                doc.scroll_line += 1.0;
            }
        }
        self.clamp_scroll();
        y.clamp(top + 1.0, editor.bottom() - 1.0)
    }

    /// 在右侧拖选：按按下时的单位从 `origin` 扩展到鼠标处；`add` 为 `true` 时只修改主选区。
    pub(in crate::app) fn drag_diff_select(
        &mut self,
        x: f32,
        y: f32,
        unit: SelectUnit,
        origin: (usize, usize),
        add: bool,
    ) {
        let y = self.diff_drag_scroll(y);
        if let Some((source, pos)) = self.diff_pos_at(x, y, false) {
            let doc = &mut self.editor.documents[source];
            let (anchor, head) = unit.extend(doc.text(), origin, pos);
            if add {
                doc.set_primary(Selection::new(anchor, head));
            } else {
                doc.selection = Selection::new(anchor, head);
            }
            self.reset_blink();
            self.request_redraw();
        }
    }

    /// 在右侧开始列选择（`⇧⌥` 拖动、中键拖动）：先在按下处放置一个光标，拖动时选择一列。
    pub(in crate::app) fn start_diff_columns(&mut self, x: f32, y: f32) {
        self.focus = Focus::Editor;
        self.preedit.clear();
        let Some(spot) = self.diff_line_cell_at(x, y, true) else {
            return;
        };
        self.editor.documents[spot.source].record_cursors();
        if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) {
            view.editing = true;
            view.read_selection = None;
        }
        let anchor = (spot.line, spot.cell);
        self.drag = crate::app::Drag::DiffColumns { anchor };
        self.drag_diff_columns(x, y, anchor);
    }

    /// 列选择拖动到 (`x`, `y`)：从 `anchor`（行，显示列）到鼠标处，每行一个选区。
    pub(in crate::app) fn drag_diff_columns(&mut self, x: f32, y: f32, anchor: (usize, usize)) {
        let y = self.diff_drag_scroll(y);
        let tab_width = self.editor.active_tab_width();
        if let Some(spot) = self.diff_line_cell_at(x, y, false) {
            self.editor.documents[spot.source].select_columns(
                anchor,
                (spot.line, spot.cell),
                tab_width,
            );
            self.reset_blink();
            self.request_redraw();
        }
    }

    /// 返回窗口坐标 (`x`, `y`) 对应第 `side` 侧文本中的位置（字符偏移）。
    ///
    /// 位于折叠的行上时为 `None`，位于最后一行下方时取文本末尾。
    fn diff_read_pos_at(&self, side: usize, x: f32, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        let view = doc.diff.as_deref()?;
        let parts = diff_parts(self.layout.editor, view, self.cell);
        let lh = self.fonts.line_height;
        let below = f64::from((y - parts.body.y) / lh);
        let visible = f64::from(parts.body.h / lh);
        let row = view.row_under(side, doc.scroll_line.max(0.0), visible, below);
        let line = match view.rows[side].get(row) {
            Some(SideRow::Line { line, .. }) => *line,
            Some(SideRow::Fold { .. }) => return None,
            None => return Some(view.side(side).text.len_chars()),
        };
        let cell = self.diff_cell_at(view, side, row, x);
        self.diff_read_pos(side, line, cell)
    }

    /// 返回窗口横坐标 `x` 在第 `side` 侧显示行 `row` 中对应的显示列（自动换行时从该行的
    /// 开头算起）。
    pub(super) fn diff_cell_at(&self, view: &DiffView, side: usize, row: usize, x: f32) -> usize {
        let scroll_x = self.editor.active().map_or(0.0, |doc| doc.scroll_x);
        let pane = diff_parts(self.layout.editor, view, self.cell).panes[side];
        let cell = ((x - pane.text.x - DIFF_TEXT_PAD + scroll_x) / self.cell)
            .round()
            .max(0.0) as usize;
        view.cell_in_row(side, row, cell)
    }

    /// 返回第 `side` 侧第 `line` 行显示列 `cell` 处的位置（字符偏移），不超过该行的末尾。
    pub(super) fn diff_read_pos(&self, side: usize, line: usize, cell: usize) -> Option<usize> {
        let view = self.editor.active()?.diff.as_deref()?;
        let text = &view.side(side).text;
        let slice = text.line(line);
        let content = slice.len_chars()
            - slice
                .chars_at(slice.len_chars())
                .reversed()
                .take_while(|c| matches!(c, '\n' | '\r'))
                .count();
        let col = winc_core::text::cell_to_col(slice, cell, self.editor.active_tab_width());
        Some(text.line_to_char(line) + col.min(content))
    }

    /// 在只读一侧拖选：按按下时的单位从 `origin` 扩展到鼠标处。
    pub(in crate::app) fn drag_diff_select_read(
        &mut self,
        x: f32,
        y: f32,
        side: usize,
        unit: SelectUnit,
        origin: (usize, usize),
    ) {
        let y = self.diff_drag_scroll(y);
        if let Some(pos) = self.diff_read_pos_at(side, x, y)
            && let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut())
        {
            let range = unit.extend(&view.side(side).text, origin, pos);
            view.read_selection = Some(ReadSelection { side, range });
            self.reset_blink();
            self.request_redraw();
        }
    }

    /// 鼠标是否位于差异两侧的文字上（只读一侧可以选中，可编辑一侧可以编辑）。
    pub(in crate::app) fn over_diff_text(&self, x: f32, y: f32) -> bool {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return false;
        };
        let parts = diff_parts(self.layout.editor, view, self.cell);
        parts.panes[LEFT].text.contains(x, y) || parts.panes[RIGHT].text.contains(x, y)
    }

    /// 鼠标是否位于差异右侧的可编辑文字上。
    pub(in crate::app) fn over_diff_editable(&self, x: f32, y: f32) -> bool {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return false;
        };
        view.new.path.is_some()
            && diff_parts(self.layout.editor, view, self.cell).panes[RIGHT]
                .text
                .contains(x, y)
            && self.diff_unified_side_at(y) != Some(LEFT)
    }

    /// 返回单栏时窗口纵坐标 `y` 处的显示行所属的一侧；并排时或该位置没有行时为 `None`。
    fn diff_unified_side_at(&self, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        let view = doc.diff.as_deref().filter(|v| v.is_unified())?;
        let body = diff_parts(self.layout.editor, view, self.cell).body;
        let index = doc.scroll_line.max(0.0) + f64::from((y - body.y) / self.fonts.line_height);
        view.unified_row(index.floor().max(0.0) as usize)
            .map(|(side, _)| side)
    }
}
