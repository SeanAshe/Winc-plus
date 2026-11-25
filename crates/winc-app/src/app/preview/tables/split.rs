//! 分栏时表格与左侧文字的联动。
//!
//! 左侧滚动时表格滚到对应的行，左侧的光标移动时表格选中光标所在的格。点击表格后键盘作用于
//! 表格，在表格中选中单元格时左侧的光标移到该格的开头；输入文字时键盘交回左侧。只有从分隔符
//! 文本读出的表（csv、tsv）知道数据行与文字行的对应（见 [`Sheet::source_line`]）。

use std::sync::Arc;

use winc_core::Lines;
use winc_editor::DocumentId;
use winc_preview::table::{Sheet, Table, field_at, field_start};
use winit::keyboard::{Key, NamedKey};

use crate::app::App;
use crate::app::preview::markdown::text_line;

/// 上次与左侧同步时的（文档，左侧的滚动位置，光标位置，文档版本，表格的 `Arc` 指针地址）。
pub(crate) type Synced = (DocumentId, f64, usize, u64, usize);

impl App {
    /// 返回分栏显示的表格及当前工作表的序号；当前标签不是分栏的 csv、tsv，或表格还没读好时
    /// 返回 `None`。
    fn split_table(&self) -> Option<(Arc<Table>, usize)> {
        let doc = self.editor.active().filter(|d| d.shows_text())?;
        let view = doc.table.as_deref()?;
        let table = Arc::clone(view.table()?);
        let sheet = table.sheets.get(view.sheet)?;
        sheet.delimiter.is_some().then_some((table, view.sheet))
    }

    /// 绘制前调用：左侧滚动后表格滚到对应的行；光标移动、文字或表格变化后选中光标所在的格。
    ///
    /// 表格持有键盘时只记下当前的状态，不跟随左侧：选区由表格决定。换了文档时键盘交回左侧。
    pub(crate) fn sync_table_split(&mut self) {
        let Some((table, index)) = self.split_table() else {
            return;
        };
        let Some(doc) = self.editor.active() else {
            return;
        };
        let address = Arc::as_ptr(&table) as usize;
        let now = (
            doc.id,
            doc.scroll_line,
            doc.selection.head,
            doc.revision,
            address,
        );
        let layout = &mut self.preview.table_layout;
        let before = layout.synced.replace(now);
        if before.is_some_and(|b| b.0 != now.0) {
            layout.focused = false;
        }
        if layout.focused || before == Some(now) {
            return;
        }
        let sheet = &table.sheets[index];
        let (scrolled, moved) = match before {
            None => (true, true),
            Some(b) => (
                b.0 != now.0 || b.1 != now.1 || b.4 != now.4,
                b.0 != now.0 || b.2 != now.2 || b.3 != now.3 || b.4 != now.4,
            ),
        };
        if scrolled {
            self.table_follow_scroll(sheet);
        }
        if moved {
            self.table_follow_caret(sheet);
        }
    }

    /// 把表格滚到左侧最上面一行文字对应的数据行。
    ///
    /// 表格的行比文字高，左侧滚到底时表格可能还没到底：余下的距离在左侧的最后一屏中按比例
    /// 补上，两侧同时到底。
    fn table_follow_scroll(&mut self, sheet: &Sheet) {
        let (row_h, max_y) = {
            let layout = &self.preview.table_layout;
            (f64::from(layout.row_h), layout.max_scroll.1)
        };
        let Some(doc) = self.editor.active() else {
            return;
        };
        if row_h <= 0.0 {
            return;
        }
        let y_of = |row: f64| table_y(sheet, text_line(doc, row), row_h);
        let max = self.max_scroll_line();
        let screen = f64::from(self.scroll_track().h / self.fonts.line_height).min(max);
        let rest = (max_y - y_of(max)).max(0.0);
        let share = match screen > 0.0 {
            true => ((doc.scroll_line - (max - screen)) / screen).clamp(0.0, 1.0),
            false => 0.0,
        };
        let y = (y_of(doc.scroll_line) + rest * share).clamp(0.0, max_y);
        if let Some(view) = self
            .editor
            .active_mut()
            .and_then(|d| d.table.as_deref_mut())
        {
            view.scroll.1 = y;
        }
    }

    /// 选中左侧光标所在的格，并滚动到该格完整可见。
    fn table_follow_caret(&mut self, sheet: &Sheet) {
        let Some(delimiter) = sheet.delimiter else {
            return;
        };
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let head = doc.selection.head;
        let row = sheet.row_at_line(rope.char_to_line(head));
        let line = sheet.source_line(row);
        if sheet.row_count() == 0 || line >= rope.len_lines() {
            return;
        }
        let start = rope.line_to_char(line);
        let Some(offset) = head.checked_sub(start) else {
            return;
        };
        let column = field_at(rope.chars_at(start), delimiter, offset);
        let column = column.min(sheet.columns.saturating_sub(1));
        self.set_table_selection((row, column), (row, column), true);
    }

    /// 分栏时在表格中选中了以 `head` 为当前格的单元格：左侧的光标移到该格在文字中的开头，并
    /// 滚动到可见；这一行数据没有那么多字段时移到这行数据的第一行末尾。
    pub(super) fn caret_follow_table(&mut self, (row, column): (usize, usize)) {
        if !self.preview.table_layout.focused {
            return;
        }
        let Some((table, index)) = self.split_table() else {
            return;
        };
        let sheet = &table.sheets[index];
        let (Some(delimiter), Some(doc)) = (sheet.delimiter, self.editor.active()) else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let line = sheet.source_line(row);
        if line >= rope.len_lines() {
            return;
        }
        let start = rope.line_to_char(line);
        let at = match field_start(rope.chars_at(start), delimiter, column) {
            Some(offset) => start + offset,
            None => doc.text().line_end(line),
        };
        self.select(at, at);
    }

    /// 分栏时表格持有键盘、按下的 `key` 是输入文字的键（字符、空格、退格、删除）：键盘交回左侧，
    /// 返回 `true`，由左侧按输入处理。
    pub(super) fn table_yields_typing(&mut self, key: &Key) -> bool {
        let typing = matches!(key, Key::Character(_))
            || matches!(
                key,
                Key::Named(NamedKey::Space | NamedKey::Backspace | NamedKey::Delete)
            );
        let layout = &mut self.preview.table_layout;
        if !(typing && layout.focused) {
            return false;
        }
        layout.focused = false;
        self.request_redraw();
        true
    }
}

/// 返回源文本第 `line` 行（小数）对应的表格纵向位置（逻辑像素）：一行数据跨几行文字时，按行在
/// 其中的位置等分该行数据的高度。
fn table_y(sheet: &Sheet, line: f32, row_h: f64) -> f64 {
    let whole = line.max(0.0) as usize;
    let row = sheet.row_at_line(whole);
    let start = sheet.source_line(row);
    let span = sheet.source_line(row + 1).saturating_sub(start).max(1);
    let into = ((f64::from(line) - start as f64) / span as f64).clamp(0.0, 1.0);
    (row as f64 + into) * row_h
}
