//! 差异标题栏上的「打开文件」：在编辑区打开差异对应的工作区文件，停在差异中正在看的那一行。

use super::*;

impl App {
    /// 打开当前差异对应的工作区文件（见 [`DiffView::file_path`]）。
    ///
    /// 光标在右侧时停在光标处，否则停在右侧屏幕上第一行显示出来的内容（折叠的未改动内容不算）；
    /// 这一行在屏幕上的高度不变。历史版本的差异按右侧的行号定位。文件已不在工作区中时在状态栏
    /// 说明。
    pub(super) fn open_diff_file(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return;
        };
        let Some(path) = view.file_path().map(Path::to_path_buf) else {
            return;
        };
        if !self.may_be_file(&path) {
            self.flash(t!("git-commit-open-deleted"));
            return;
        }
        let Some((line, col, at)) = self.diff_spot(index) else {
            self.open_path(&path);
            self.focus = Focus::Editor;
            return;
        };
        self.open_path_then(&path, move |app| {
            let Some(doc) = app.editor.active() else {
                return;
            };
            let text = doc.text();
            let line = line.min(text.line_count().saturating_sub(1));
            let col = col.min(text.line_content(line).slice().len_chars());
            let pos = text.line_col_to_pos(line, col);
            app.focus = Focus::Editor;
            app.scroll_to_line(line, at, false);
            app.select(pos, pos);
        });
    }

    /// 第 `index` 个文档（差异）中正在看的位置：（右侧的行，列，这一行在屏幕上的高度占屏高的
    /// 比例）；右侧没有行时为 `None`。
    fn diff_spot(&self, index: usize) -> Option<(usize, usize, f32)> {
        let doc = self.editor.documents.get(index)?;
        let view = doc.diff.as_deref()?;
        let parts = diff_parts(self.layout.editor, view, self.metrics.cell);
        let visible = f64::from(parts.body.h / self.fonts.line_height).max(1.0);
        let pos = doc.scroll_line.max(0.0);
        let top = view.side_scroll(RIGHT, pos, visible).floor().max(0.0) as usize;
        let rows = &view.rows[RIGHT];
        let line_of = |row: &SideRow| match row {
            SideRow::Line { line, .. } => *line,
            SideRow::Fold { lines, .. } => lines.start,
        };
        let height = |row: usize| ((row as f64 - top as f64) / visible).clamp(0.0, 0.9) as f32;
        // 光标在右侧：取右侧文档里的光标，屏幕高度按它所在的显示行算。
        let caret = view.editing.then(|| {
            let path = view.new.path.as_deref()?;
            let source =
                (self.editor.documents.iter()).find(|d| d.path.as_deref() == Some(path))?;
            Some(source.text().pos_to_line_col(source.selection.head))
        });
        if let Some(Some((line, col))) = caret {
            let row = rows.iter().position(|r| line_of(r) >= line).unwrap_or(top);
            return Some((line, col, height(row)));
        }
        let shown = (rows.iter().enumerate().skip(top)).find_map(|(row, r)| match r {
            SideRow::Line { line, .. } => Some((row, *line)),
            SideRow::Fold { .. } => None,
        });
        let (row, line) = shown.or_else(|| Some((top, line_of(rows.last()?))))?;
        Some((line, 0, height(row)))
    }
}
