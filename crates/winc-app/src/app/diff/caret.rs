//! 差异中只读一侧的光标：没有光标时的放置、窗口顶部的行，以及滚动到光标处。

use super::*;

impl App {
    /// 当前标签是差异且两侧都没有光标时，把只读光标放在窗口顶部一行的行首，返回是否已放置。
    ///
    /// 右侧有内容时放在右侧，否则放在左侧。
    pub(in crate::app) fn place_read_caret(&mut self) -> bool {
        let Some(index) = self.editor.active else {
            return false;
        };
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return false;
        };
        let side = if view.new.text.len_chars() > 0 {
            RIGHT
        } else {
            LEFT
        };
        if view.editing || view.read_selection.is_some() || view.side(side).text.len_chars() == 0 {
            return false;
        }
        let line = self.diff_top_line(index, side);
        let Some(view) = self.editor.documents[index].diff.as_deref_mut() else {
            return false;
        };
        let pos = view.side(side).text.line_to_char(line);
        view.read_selection = Some(ReadSelection {
            side,
            range: (pos, pos),
        });
        self.request_redraw();
        true
    }

    /// 返回标签 `index` 的差异中第 `side` 侧窗口顶部的文本行；顶部是折叠行时取之后的第一行。
    pub(in crate::app) fn diff_top_line(&self, index: usize, side: usize) -> usize {
        let Some(doc) = self.editor.documents.get(index) else {
            return 0;
        };
        let Some(view) = doc.diff.as_deref() else {
            return 0;
        };
        let parts = diff_parts(self.layout.editor, view, self.cell);
        let visible = f64::from(parts.body.h / self.fonts.line_height);
        let rows = view.rows[side].len();
        let top = view.row_under(side, doc.scroll_line.max(0.0), visible, 0.0);
        (top..rows)
            .find_map(|row| view.line_at(side, row))
            .or_else(|| {
                (0..top.min(rows))
                    .rev()
                    .find_map(|row| view.line_at(side, row))
            })
            .unwrap_or(0)
    }

    /// 把当前差异滚动 `lines` 行，再使只读一侧的光标可见：所在行被折叠时先展开。
    pub(in crate::app) fn diff_read_caret_visible(&mut self, lines: f64) {
        let Some(index) = self.editor.active else {
            return;
        };
        let tab_width = self.editor.active_tab_width();
        let doc = &mut self.editor.documents[index];
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        let Some((side, pos)) = view.read_caret() else {
            return;
        };
        let text = &view.side(side).text;
        let (line, col) = text.pos_to_line_col(pos);
        let cell = winc_core::text::col_to_cell(text.line(line), col, tab_width);
        doc.scroll_line = (doc.scroll_line + lines).max(0.0);
        self.diff_scroll_to(index, side, (line, col), cell);
        self.request_redraw();
    }
}
