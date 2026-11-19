//! 十六进制视图的交互：各栏的位置、点按和拖选字节、键盘移动、复制和转到偏移。
//!
//! 绘制见 `view/editor/hex.rs`。每行一个编辑区行高，滚动用文档的通用逻辑（`scroll_line` 就是
//! 第几行），顶上一行是列标题，不随内容滚动。

use winc_editor::hex::{COPY_LIMIT, HexView, ROW, hex_text};
use winc_i18n::t;
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::app::scroll::set_scroll_line;

/// 列标题和各行左边的留白（逻辑像素），与编辑区文字离左边的距离相近。
const HEX_PAD: f32 = 16.0;
/// 偏移至少写的位数（4 GiB 以内）。
const OFFSET_DIGITS: usize = 8;

/// 十六进制视图各部分的位置（窗口坐标，逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HexParts {
    /// 顶上的列标题一行。
    pub header: Rect,
    /// 字节各行所在的区域。
    pub body: Rect,
    /// 偏移一栏的左边。
    pub offset_x: f32,
    /// 偏移写的位数。
    pub digits: usize,
    /// 十六进制一栏的左边。
    pub hex_x: f32,
    /// ASCII 一栏的左边。
    pub ascii_x: f32,
    /// 一个等宽字符格的宽度。
    pub cell: f32,
}

impl HexParts {
    /// 十六进制一栏里一行中第 `i` 个字节（从 0 起）的左边：每个字节占三格，前后两半之间多空一格。
    pub(crate) fn hex_at(&self, i: u64) -> f32 {
        self.hex_x + (i * 3 + u64::from(i >= ROW / 2)) as f32 * self.cell
    }

    /// ASCII 一栏里一行中第 `i` 个字节的左边。
    pub(crate) fn ascii_at(&self, i: u64) -> f32 {
        self.ascii_x + i as f32 * self.cell
    }

    /// 横坐标 `x` 处是一行中的第几个字节：在某一栏里按格算，两栏之间、超出右边的取最近的。
    fn column_at(&self, x: f32) -> u64 {
        let last = ROW - 1;
        if x >= self.ascii_x - self.cell {
            return (((x - self.ascii_x) / self.cell).floor().max(0.0) as u64).min(last);
        }
        let cells = ((x - self.hex_x) / self.cell).max(0.0);
        let half = (ROW / 2 * 3) as f32;
        let cells = if cells >= half { cells - 1.0 } else { cells };
        ((cells / 3.0).floor().max(0.0) as u64).min(last)
    }
}

impl App {
    /// 当前标签是否是十六进制视图。
    pub(crate) fn hex_active(&self) -> bool {
        self.editor.active().is_some_and(|d| d.hex.is_some())
    }

    /// 当前十六进制视图各部分的位置。
    pub(crate) fn hex_parts(&self) -> HexParts {
        let e = self.layout.editor;
        let (lh, cell) = (self.fonts.line_height, self.metrics.cell);
        let view = self.editor.active().and_then(|d| d.hex.as_deref());
        let last = view.map_or(0, |v| v.size().saturating_sub(1));
        let digits = format!("{last:X}").len().max(OFFSET_DIGITS);
        let offset_x = e.x + HEX_PAD;
        let hex_x = offset_x + (digits + 3) as f32 * cell;
        HexParts {
            header: Rect::new(e.x, e.y, e.w, lh),
            body: Rect::new(e.x, e.y + lh, e.w, (e.h - lh).max(0.0)),
            offset_x,
            digits,
            hex_x,
            ascii_x: hex_x + (ROW * 3 + 1 + 2) as f32 * cell,
            cell,
        }
    }

    /// 窗口坐标 (`x`, `y`) 处的字节；在最后一个字节之后时是最后一个，空文件时为 `None`。
    pub(in crate::app) fn hex_byte_at(&self, x: f32, y: f32) -> Option<u64> {
        let doc = self.editor.active()?;
        let view = doc.hex.as_deref().filter(|v| v.size() > 0)?;
        let parts = self.hex_parts();
        let row = (f64::from((y - parts.body.y) / self.fonts.line_height) + doc.scroll_line)
            .floor()
            .max(0.0) as u64;
        let at = row.saturating_mul(ROW).saturating_add(parts.column_at(x));
        Some(at.min(view.size() - 1))
    }

    /// 在十六进制视图的 (`x`, `y`) 处按下：选中那个字节，按住 `⇧` 时扩大选区；之后拖动继续选。
    pub(in crate::app) fn hex_press(&mut self, x: f32, y: f32) {
        self.focus = Focus::Editor;
        // 列标题一行不对应字节。
        if y < self.hex_parts().body.y {
            return self.request_redraw();
        }
        let extend = self.pointer.modifiers.shift_key();
        if let Some(at) = self.hex_byte_at(x, y) {
            self.hex_view_mut(|view| view.move_to(at, extend));
            self.pointer.drag = Drag::Hex;
        }
        self.request_redraw();
    }

    /// 拖选字节：选区的另一端跟到 (`x`, `y`) 处的字节。
    pub(in crate::app) fn drag_hex(&mut self, x: f32, y: f32) {
        if let Some(at) = self.hex_byte_at(x, y) {
            self.hex_view_mut(|view| view.move_to(at, true));
            self.reveal_hex();
            self.request_redraw();
        }
    }

    /// 十六进制视图里的按键：方向键按字节、按行移动，翻页，`Home`、`End` 到行首行尾（`cmd` 时
    /// 到文件开头结尾）；按住 `⇧` 时扩大选区。
    pub(in crate::app) fn hex_key(&mut self, key: &Key, cmd: bool) {
        let Some(view) = self.editor.active().and_then(|d| d.hex.as_deref()) else {
            return;
        };
        let (head, size) = (view.head, view.size());
        let page = self.visible_lines().saturating_sub(2).max(1) as u64 * ROW;
        let last = size.saturating_sub(1);
        let at = match key {
            Key::Named(NamedKey::ArrowLeft) => head.saturating_sub(1),
            Key::Named(NamedKey::ArrowRight) => head + 1,
            Key::Named(NamedKey::ArrowUp) if head >= ROW => head - ROW,
            Key::Named(NamedKey::ArrowUp) => head,
            Key::Named(NamedKey::ArrowDown) if head + ROW <= last => head + ROW,
            Key::Named(NamedKey::ArrowDown) => head,
            Key::Named(NamedKey::PageUp) => head.saturating_sub(page),
            Key::Named(NamedKey::PageDown) => (head + page).min(last),
            Key::Named(NamedKey::Home) if cmd => 0,
            Key::Named(NamedKey::Home) => head - head % ROW,
            Key::Named(NamedKey::End) if cmd => last,
            Key::Named(NamedKey::End) => head - head % ROW + ROW - 1,
            _ => return,
        };
        let extend = self.pointer.modifiers.shift_key();
        self.hex_view_mut(|view| view.move_to(at, extend));
        self.reveal_hex();
        self.request_redraw();
    }

    /// 选中整个文件。
    pub(in crate::app) fn select_all_hex(&mut self) {
        self.hex_view_mut(HexView::select_all);
        self.request_redraw();
    }

    /// 复制选中的字节：`text` 时按 UTF-8 文字（无法解码的字节换成 U+FFFD），否则写成十六进制。
    /// 选中的超过 [`COPY_LIMIT`] 时不复制，在状态栏说明。
    pub(in crate::app) fn copy_hex(&mut self, text: bool) {
        let Some(view) = self.editor.active().and_then(|d| d.hex.as_deref()) else {
            return;
        };
        let range = view.selection();
        let len = range.end - range.start;
        if len == 0 {
            return;
        }
        if len > COPY_LIMIT {
            let limit = winc_editor::size_text(COPY_LIMIT);
            self.flash(t!("hex-copy-too-large", limit = limit));
            return;
        }
        let bytes = view.bytes(range.start, len);
        let copied = match text {
            true => String::from_utf8_lossy(&bytes).into_owned(),
            false => hex_text(&bytes),
        };
        self.set_clipboard(copied, false);
    }

    /// 打开「转到偏移」输入条：十六进制视图里 `⌃G` 转到字节，而不是行。
    pub(in crate::app) fn open_goto_offset(&mut self) {
        let field = crate::field::TextField::with_filter(|c| c.is_ascii_hexdigit() || c == 'x');
        self.open_prompt(PromptKind::GoToOffset, field);
    }

    /// 转到偏移 `input`：`0x` 开头的按十六进制，否则按十进制；超出文件时到最后一个字节。
    /// 认不出时不动。
    pub(in crate::app) fn go_to_offset(&mut self, input: &str) {
        let Some(at) = parse_offset(input) else {
            return;
        };
        self.hex_view_mut(|view| view.move_to(at, false));
        self.reveal_hex();
        self.focus = Focus::Editor;
    }

    /// 滚动到光标所在的字节那一行可见。
    fn reveal_hex(&mut self) {
        let lh = self.fonts.line_height;
        let visible = f64::from((self.hex_parts().body.h / lh).floor().max(1.0));
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(view) = doc.hex.as_deref() else {
            return;
        };
        let row = (view.head / ROW) as f64;
        if row < doc.scroll_line {
            set_scroll_line(doc, row);
        } else if row + 1.0 > doc.scroll_line + visible {
            set_scroll_line(doc, row + 1.0 - visible);
        }
        self.clamp_scroll();
    }

    /// 用当前十六进制视图运行 `f`；当前标签不是十六进制视图时什么都不做。
    fn hex_view_mut(&mut self, f: impl FnOnce(&mut HexView)) {
        if let Some(view) = self.editor.active_mut().and_then(|d| d.hex.as_deref_mut()) {
            f(view);
        }
    }

    /// 把当前标签换成十六进制视图（「通过编码重新打开」的「十六进制」、二进制文件的占位页）。
    ///
    /// 文档有未保存的修改时，与「通过编码重新打开」一样先确认放弃。
    pub(in crate::app) fn show_hex(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;
        if !doc.is_dirty() {
            return self.show_hex_now(id);
        }
        if self.window.handle.is_some() {
            let message = t!("workspace-reopen-confirm");
            let detail = t!("workspace-reopen-confirm-detail");
            let button = t!("workspace-reopen-discard");
            self.show_confirm(message, detail, button, move |ok| {
                ok.then_some(UserEvent::ShowHex(id))
            });
            return;
        }
        self.flash(t!("workspace-reopen-dirty"));
    }

    /// 把文档 `id` 换成十六进制视图，放弃未保存的修改；文件在后台打开。
    pub(in crate::app) fn show_hex_now(&mut self, id: DocumentId) {
        let Some(index) = self.editor.index_of(id) else {
            return;
        };
        if self.editor.show_hex(index) {
            if self.editor.active == Some(index) {
                self.load_active_in_background();
            }
            self.focus = Focus::Editor;
            self.relayout();
            self.request_redraw();
        }
    }

    /// 把当前的十六进制视图换回文本，按 `encoding` 解码；文本在后台读取。
    pub(in crate::app) fn show_hex_as_text(&mut self, encoding: winc_editor::Encoding) {
        let Some(index) = self.editor.active else {
            return;
        };
        if self.editor.show_text(index, encoding) {
            self.load_active_in_background();
            self.relayout();
            self.request_redraw();
        }
    }
}

/// 解析偏移：`0x` 开头的按十六进制，否则按十进制。
fn parse_offset(input: &str) -> Option<u64> {
    let input = input.trim();
    match input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => input.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_offset;

    #[test]
    fn offsets_are_hex_with_0x_and_decimal_otherwise() {
        assert_eq!(parse_offset("0x4E"), Some(0x4E));
        assert_eq!(parse_offset(" 78 "), Some(78));
        assert_eq!(parse_offset("4E"), None);
        assert_eq!(parse_offset(""), None);
    }
}
