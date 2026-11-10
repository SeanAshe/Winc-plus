//! 只读差异快照的查找：在差异某一侧的文本中查找，不涉及工作区文件。

use super::*;

impl App {
    /// 返回当前标签中右侧没有对应工作区文件的差异，例如从 Git 日志打开的提交；
    /// 当前标签不是这类差异时返回 `None`。
    pub(crate) fn readonly_diff(&self) -> Option<&DiffView> {
        self.editor
            .active()?
            .diff
            .as_deref()
            .filter(|view| view.new.path.is_none())
    }

    /// 是否可以在当前标签中替换：只读的差异快照不能替换。
    pub(crate) fn can_replace(&self) -> bool {
        self.readonly_diff().is_none()
    }

    /// 当前标签是否为可编辑的差异：查找与替换在右侧对应的工作区文件中进行（见
    /// [`App::in_diff_search`]）。
    pub(in crate::app) fn editable_diff_active(&self) -> bool {
        self.diff_active() && self.can_replace()
    }

    /// 在只读差异快照中推进一个时间片的查找，返回（差异标签的标识，所查一侧的选区）；
    /// 当前标签不是只读差异时返回 `None`。
    ///
    /// 查找光标或选区所在的一侧；尚未在任一侧放置光标时查找右侧，右侧不存在
    /// （文件已删除）时查找左侧。所查的一侧变化时，等待中的跳转作废。
    pub(in crate::app) fn update_diff_find(&mut self) -> Option<(DocumentId, Selection)> {
        let doc = self.editor.active()?;
        let view = doc.diff.as_deref().filter(|view| view.new.path.is_none())?;
        let read = view.read_selection.filter(|s| !view.side(s.side).missing);
        let side = read.map_or(if view.new.missing { LEFT } else { RIGHT }, |s| s.side);
        let selection =
            read.map_or_else(Selection::default, |s| Selection::new(s.range.0, s.range.1));
        if self.search.snapshot_side(doc) != Some(side) {
            self.pending_jump = None;
        }
        self.search
            .update_snapshot(doc, side, &view.side(side).text);
        Some((doc.id, selection))
    }

    /// 选中第 `side` 侧位置 `start..end`（字符偏移）的匹配并滚动到可见位置；匹配位于折叠的
    /// 未改动段内时先展开该段。
    pub(in crate::app) fn reveal_diff_match(&mut self, side: usize, start: usize, end: usize) {
        let tab_width = self.editor.tab_width;
        let Some(index) = self.editor.active else {
            return;
        };
        let Some(line) = (self.editor.documents[index].diff.as_deref())
            .map(|view| view.side(side).text.char_to_line(start))
        else {
            return;
        };
        // 匹配所在行的外层块固定在顶部时，匹配需要位于这些固定行之下。
        let sticky = self.diff_sticky_depth(index, side, line);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(view) = doc.diff.as_deref_mut() else {
            return;
        };
        let text = &view.side(side).text;
        let (line, col) = text.pos_to_line_col(start);
        let last_line = text.line_of(end.saturating_sub(1).max(start));
        let parts = diff_parts(self.layout.editor, view, self.cell);
        let pane = parts.panes[side].text;
        let cells = |col: usize| winc_core::text::col_to_cell(text.line(line), col, tab_width);
        let x = cells(col) as f32 * self.cell;
        // 跨行的匹配只按第一行计算右端。
        let end_col = match text.pos_to_line_col(end) {
            (end_line, end_col) if end_line == line => end_col,
            _ => text.line(line).len_chars(),
        };
        let x_end = (cells(end_col) as f32 * self.cell).max(x + self.cell);
        let visible = f64::from(parts.body.h / self.fonts.line_height);
        let covered = if self.layout.search.x < pane.right() && self.layout.search.h > 0.0 {
            f64::from((self.layout.search.bottom() - parts.body.y) / self.fonts.line_height)
        } else {
            0.0
        };

        view.editing = false;
        view.read_selection = Some(ReadSelection {
            side,
            range: (start, end),
        });
        view.reveal_line(side, line);
        view.reveal_line(side, last_line);
        let row = view.row_of_col(side, line, col) as f64;
        let (row, top) = if view.is_unified() {
            (view.pos_of(side, row), doc.scroll_line)
        } else {
            (row, view.side_scroll(side, doc.scroll_line, visible))
        };
        if row < top + covered.max(sticky) || row + 1.0 > top + visible {
            let top = (row - visible / 2.0).max(0.0);
            doc.scroll_line = if view.is_unified() {
                top
            } else {
                view.pos_of_top(side, top, visible)
            };
            view.focus = None;
        }
        if !view.is_wrapping() {
            let width = (pane.w - 2.0 * DIFF_TEXT_PAD).max(self.cell);
            if x < doc.scroll_x || x_end > doc.scroll_x + width {
                // 匹配的起点放在中间；匹配较长时右移到能显示其末端，但不越过起点。
                let centered = (x - width / 2.0).max(0.0);
                doc.scroll_x = centered.max(x_end - width).min(x);
            }
        }
        self.clamp_scroll();
        self.request_redraw();
    }
}
