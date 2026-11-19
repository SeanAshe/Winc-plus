//! 十六进制视图的绘制：顶上一行列标题，下面每行一个偏移、16 个字节的十六进制和它们的 ASCII；
//! 选中的字节在两栏里同时铺底色，值为 0 的字节和不可显示的字符用淡色。
//!
//! 交互和各栏的位置见 `crate::app::editor::hex`。

use std::fmt::Write as _;

use winc_editor::hex::{ROW, ascii};

use super::*;
use crate::app::editor::hex::HexParts;
use crate::layout::editor_content_height;

/// 绘制第 `index` 个标签的十六进制视图。
pub(super) fn draw_hex(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let parts = app.hex_parts();
    let lh = app.fonts.line_height;
    let (p, size) = (app.palette, app.fonts.code);
    let doc = &app.editor.documents[index];
    let (id, rows) = (doc.id, doc.row_count());
    let scroll = drawn_scroll_line(doc.scroll_line, lh);
    let Some(view) = doc.hex.as_deref() else {
        return;
    };
    frame.rect(app.layout.editor, p.editor_bg);
    if let Some(error) = view.error.clone() {
        draw_placeholder(app, frame, error, &[]);
        return;
    }
    draw_header(app, frame, &parts);

    let body = parts.body;
    let (ascent, descent) = (app.metrics.mono.ascent, app.metrics.mono.descent);
    let selection = view.selection();
    let focused = app.focus == Focus::Editor;
    let shade = if focused {
        p.selection
    } else {
        p.selection_inactive
    };
    let mut run = String::new();
    let first = scroll.floor().max(0.0) as u64;
    let visible = (body.h / lh).ceil() as u64 + 1;
    for row in first..(first + visible).min(rows as u64) {
        let y = body.y + ((row as f64 - scroll) * f64::from(lh)) as f32;
        let band = Rect::new(body.x, y, body.w, lh);
        let baseline = baseline_in(band, ascent, descent);
        let start = row * ROW;
        let bytes = view.bytes(start, ROW);

        // 这一行里选中的字节：两栏各铺一块连续的底色。
        let from = selection.start.max(start);
        let to = selection.end.min(start + bytes.len() as u64);
        if from < to {
            let (a, b) = (from - start, to - start - 1);
            let half = parts.cell / 2.0;
            let hex = Rect::new(
                parts.hex_at(a) - half,
                y,
                parts.hex_at(b) + 2.0 * parts.cell + half - (parts.hex_at(a) - half),
                lh,
            );
            frame.rect_clipped(hex, shade, 3.0, body);
            let ascii = Rect::new(parts.ascii_at(a), y, (b - a + 1) as f32 * parts.cell, lh);
            frame.rect_clipped(ascii, shade, 2.0, body);
        }

        let offset = format!("{start:0width$X}", width = parts.digits);
        frame.text(
            FontKind::Mono,
            size,
            parts.offset_x,
            baseline,
            &offset,
            p.gutter_text,
            body,
        );
        let colors = (p.text, p.text_dim);
        draw_row(
            frame,
            &parts,
            &bytes,
            (size, baseline, body),
            colors,
            &mut run,
        );
    }

    let content = editor_content_height(rows, body.h, lh);
    let track = Rect::new(body.x, body.y, body.w, body.h);
    draw_scrollbar(
        app,
        frame,
        ScrollArea::Editor,
        id.0,
        track,
        (scroll * f64::from(lh)) as f32,
        content,
    );
}

/// 绘制一行的十六进制和 ASCII 两栏：值为 0 的字节、不可显示的字符用淡色 `colors.1`，其余用
/// `colors.0`；颜色相同的相邻字节连成一段画。`run` 是复用的缓冲。
fn draw_row(
    frame: &mut Frame<'_>,
    parts: &HexParts,
    bytes: &[u8],
    (size, baseline, clip): (f32, f32, Rect),
    (text, dim): (Color, Color),
    run: &mut String,
) {
    let half = ROW as usize / 2;
    for ascii_column in [false, true] {
        let color = |b: u8| match ascii_column {
            false if b == 0 => dim,
            true if ascii(b).is_none() => dim,
            _ => text,
        };
        let mut i = 0;
        while i < bytes.len() {
            let (start, shade) = (i, color(bytes[i]));
            run.clear();
            while i < bytes.len() && color(bytes[i]) == shade {
                let b = bytes[i];
                if ascii_column {
                    run.push(ascii(b).unwrap_or('·'));
                } else {
                    if i > start {
                        run.push_str(if i == half { "  " } else { " " });
                    }
                    let _ = write!(run, "{b:02X}");
                }
                i += 1;
            }
            let x = match ascii_column {
                true => parts.ascii_at(start as u64),
                false => parts.hex_at(start as u64),
            };
            frame.text(FontKind::Mono, size, x, baseline, run, shade, clip);
        }
    }
}

/// 绘制顶上的列标题：「偏移」，以及十六进制一栏各列的序号 `00`…`0F`；下边一条分隔线。
fn draw_header(app: &App, frame: &mut Frame<'_>, parts: &HexParts) {
    let p = app.palette;
    let header = parts.header;
    let (ascent, descent) = (app.metrics.mono.ascent, app.metrics.mono.descent);
    let baseline = baseline_in(header, ascent, descent);
    let label = t!("hex-offset");
    ui_text_small(
        frame,
        app,
        parts.offset_x,
        header,
        label,
        p.text_dim,
        header,
    );
    for i in 0..ROW {
        let text = format!("{i:02X}");
        let x = parts.hex_at(i);
        frame.text(
            FontKind::Mono,
            app.fonts.code,
            x,
            baseline,
            &text,
            p.text_dim,
            header,
        );
    }
    let line = Rect::new(header.x, header.bottom() - 1.0, header.w, 1.0);
    frame.rect(line, p.border);
}
