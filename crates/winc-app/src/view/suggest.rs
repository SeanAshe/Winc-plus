//! 链接地址和写作命令的补全列表：光标下方的卡片，文件行为图标、文件名和所在目录，命令行为
//! 名字和快捷键；下方放不下时显示在光标上方。

use super::popup_menu::{MARGIN, ROW_INSET, band_ink};
use super::*;
use crate::app::preview::markdown::suggest::Hit;
use crate::app::preview::markdown::suggest::SUGGEST_VISIBLE;
use crate::app::{MenuIcon, POPUP_PAD, ScrollArea};

/// 列表的最小、最大宽度（逻辑像素）。
const WIDTH: (f32, f32) = (240.0, 420.0);
/// 每行的高度（逻辑像素）：比菜单的行高一些，带图标的各行之间留出间隙。
const LIST_ROW: f32 = 28.0;
/// 每行底色上下各缩进的距离（逻辑像素）：相邻两行的底色之间相隔两倍于此。
const BAND_INSET: f32 = 2.0;
/// 行内左右两侧的留白（逻辑像素）：图标离底色左边、快捷键离底色右边的距离相同。
const PAD: f32 = 8.0;
/// 图标所占的宽度（逻辑像素），等于写作命令图标与文件图标的宽度。
const ICON: f32 = 12.0;
/// 图标与文字之间的距离（逻辑像素）。
const ICON_GAP: f32 = 8.0;
/// 文件名之后到目录的间距（逻辑像素）。
const DIR_GAP: f32 = 8.0;

/// 绘制补全列表（开着、有文件时），记下卡片和各行的位置。
pub(super) fn draw_suggest(app: &mut App, frame: &mut Frame<'_>) {
    let suggest = &app.preview.markdown.suggest;
    let Some(caret) = app.ime.cursor else {
        return;
    };
    if suggest.at.is_none() || suggest.hits.is_empty() {
        return;
    }
    // 文件显示文件名和所在目录，标题显示文字和级别（`H2`）。
    let rows: Vec<(String, String)> = (suggest.hits.iter())
        .map(|hit| match hit {
            Hit::File(path, _) => match path.rsplit_once('/') {
                Some((dir, name)) => (name.to_owned(), dir.to_owned()),
                None => (path.clone(), String::new()),
            },
            Hit::Heading(level, text, _) => (text.clone(), format!("H{level}")),
            Hit::Command(command) => {
                let key = command.action().and_then(crate::keymap::shortcut);
                (command.label().to_owned(), key.unwrap_or_default())
            }
        })
        .collect();
    let commands = matches!(suggest.hits.first(), Some(Hit::Command(_)));
    let headings = matches!(suggest.hits.first(), Some(Hit::Heading(..)));
    let (size, small) = (app.fonts.ui, app.fonts.ui_small);
    let text_x = ROW_INSET + PAD + ICON + ICON_GAP;
    let widest = (rows.iter())
        .map(|(name, dir)| {
            let dir = if dir.is_empty() {
                0.0
            } else {
                DIR_GAP + frame.measure(FontKind::Ui, small, dir)
            };
            frame.measure(FontKind::Ui, size, name) + dir
        })
        .fold(0.0, f32::max);
    let area = app.layout.editor;
    let w = (text_x + widest + PAD + ROW_INSET)
        .clamp(WIDTH.0, WIDTH.1)
        .min(area.w - 2.0 * MARGIN);
    let shown = rows.len().min(SUGGEST_VISIBLE);
    let scrolls = rows.len() > shown;
    let h = 2.0 * POPUP_PAD + shown as f32 * LIST_ROW;
    let x = (caret.x - text_x)
        .min(area.right() - MARGIN - w)
        .max(area.x + MARGIN);
    let below = caret.bottom() + 4.0;
    let y = if below + h <= area.bottom() - MARGIN {
        below
    } else {
        (caret.y - 4.0 - h).max(area.y + MARGIN)
    };
    let card = Rect::new(x.round(), y.round(), w.round(), h);
    let p = app.palette;
    draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);
    let clip = card.inset(0.0, POPUP_PAD);
    let suggest = &mut app.preview.markdown.suggest;
    let selected = suggest.selected;
    let first = suggest.first.min(rows.len() - shown);
    suggest.first = first;
    let mut hits = Vec::with_capacity(shown);
    for (i, (name, dir)) in rows.iter().enumerate().skip(first).take(shown) {
        let top = card.y + POPUP_PAD + (i - first) as f32 * LIST_ROW;
        let row = Rect::new(card.x, top, card.w, LIST_ROW);
        let band = row.inset(ROW_INSET, BAND_INSET);
        // 不同类的命令之间画一道分隔线，位于两行之间，不占高度。
        if i > first && group_of(app, i) != group_of(app, i - 1) {
            let line = Rect::new(band.x + PAD, row.y - 0.5, band.w - PAD * 2.0, 1.0);
            frame.rect_clipped(line, p.separator, 0.0, clip);
        }
        let (ink, dim) = if i == selected {
            frame.rect_clipped(band, p.accent_fill, RADIUS_ROW, clip);
            band_ink(p)
        } else {
            if app.pointer.hover == Hover::Suggest(i) {
                frame.rect_clipped(band, p.hover, RADIUS_ROW, clip);
            }
            (p.text, p.text_dim)
        };
        let cy = icon_center_y(app, row, size);
        let column = band.x + PAD;
        match suggest_hit(app, i) {
            // 命令使用写作命令的图标。
            Some(Hit::Command(command)) => {
                let center = (column + ICON / 2.0, cy);
                crate::icons::writing(frame, center, command.glyph(), dim, clip);
            }
            // 标题使用 Markdown 文件的图标。
            _ => {
                let (glyph, colors) = crate::theme::file_kind(if headings { "a.md" } else { name });
                let icon = MenuIcon::File(glyph, colors);
                let (iw, ih) = icon.size();
                let at = (column + (ICON - iw) / 2.0, cy - ih / 2.0);
                app.draw_menu_icon(frame, icon, at, clip);
            }
        }
        let x = row.x + text_x;
        let right = band.right() - PAD;
        let room = right - x;
        let shown = ellipsize(frame, name, size, room.max(0.0));
        let end = ui_text(frame, app, x, row, &shown, ink, clip);
        let rest = right - end - DIR_GAP;
        if !dir.is_empty() && rest > 24.0 {
            let dir = ellipsize(frame, dir, small, rest);
            // 命令的快捷键靠右对齐。
            let at = match commands {
                true => right - frame.measure(FontKind::Ui, small, &dir),
                false => end + DIR_GAP,
            };
            ui_text_small(frame, app, at, row, &dir, dim, clip);
        }
        hits.push((row, i));
    }
    // 项数多于可见行数时画滚动条，样式与其他列表相同。
    if scrolls {
        let track = Rect::new(card.x, card.y + POPUP_PAD, card.w, shown as f32 * LIST_ROW);
        let (scroll, content) = (first as f32 * LIST_ROW, rows.len() as f32 * LIST_ROW);
        let owner = app.editor.active().map_or(0, |doc| doc.id.0);
        draw_scrollbar(
            app,
            frame,
            ScrollArea::Suggest,
            owner,
            track,
            scroll,
            content,
        );
    }
    app.hits.suggest = hits;
    app.hits.suggest_card = Some(card);
}

/// 返回补全列表第 `i` 项所属的类；不是命令时为 `None`。
fn group_of(app: &App, i: usize) -> Option<u8> {
    match app.preview.markdown.suggest.hits.get(i) {
        Some(Hit::Command(command)) => Some(command.group()),
        _ => None,
    }
}

/// 返回补全列表的第 `i` 项。
fn suggest_hit(app: &App, i: usize) -> Option<Hit> {
    app.preview.markdown.suggest.hits.get(i).cloned()
}
