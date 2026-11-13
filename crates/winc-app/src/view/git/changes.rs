//! 点击行号栏改动标记后打开的卡片：上方是标题栏，下方是该处改动在 `HEAD` 中的原内容。
//!
//! 标题栏左侧是与标记同色的竖条、「修改 2 行」（或「新增」「删除」）和基准版本，右侧是「1 / 4」、
//! 上一处、下一处、回滚、在差异视图中打开、关闭。原内容使用与差异视图左侧相同的改动底色，每行
//! 前有「−」及该行在 `HEAD` 中的行号。卡片位于 `ChangePopup::anchor` 指定的各行下方，空间不足时
//! 位于上方，上下边落在行的分界上。坐标均为逻辑像素，原点在窗口左上角。

use std::fmt::Write as _;

use winc_core::{Lines, text};
use winc_i18n::t;

use crate::app::{ChangeTarget, fold_column};
use crate::icons::Direction;
use crate::view::diff::hunk_shade;
use crate::view::editor::draw_glyphs;
use crate::view::*;

/// 标题栏的最小高度（逻辑像素）；绘制时向上取整到行高的整数倍，使卡片覆盖整数行。
const HEADER: f32 = 30.0;
/// 标题栏中按钮的边长（逻辑像素），与差异标签工具栏按钮的边长相同。
const BUTTON: f32 = 24.0;

/// 绘制已打开的改动卡片，并把按钮的可点击范围记录到 `app.hits.changes`。
pub(in crate::view) fn draw_change_popup(app: &mut App, frame: &mut Frame<'_>) {
    let Some(popup) = app.git.changes.popup else {
        return;
    };
    if app.editor.active().map(|d| d.id) != Some(popup.doc) {
        return;
    }
    // 原内容按文档版本缓存，避免每帧重新读取基准版本、逐词比较和着色。
    let Some(preview) = app.change_preview() else {
        return;
    };
    let Some(doc) = app.editor.active() else {
        return;
    };
    let hunks = app.git.changes.hunks(doc.id);
    let Some(hunk) = hunks.get(popup.hunk).cloned() else {
        return;
    };
    let count = hunks.len();
    let tab = doc.tab_width(app.editor.tab_width);
    let p = app.palette;
    let shade = hunk_shade(p, &hunk);
    let lh = app.fonts.line_height;
    let cell = app.metrics.cell;
    let (text_rect, gutter) = (app.layout.text, app.layout.gutter);
    // 卡片中的行号、文字分别与编辑区的行号和文字对齐；「−」位于行号左侧。
    let code_x = text_rect.x + TEXT_PAD;
    let number_right = fold_column(gutter).x;
    let shown = preview.lines.len();
    let more = preview.more;

    let lines = if hunk.old.is_empty() {
        hunk.new.len()
    } else {
        hunk.old.len()
    };
    let title = match (hunk.old.is_empty(), hunk.new.is_empty()) {
        (true, _) => t!("git-change-popup-added", count = lines),
        (_, true) => t!("git-change-popup-deleted", count = lines),
        _ => t!("git-change-popup-modified", count = lines),
    };
    // 说明下方内容所属的版本；新增的块没有原内容，说明该内容不在 `HEAD` 中。
    let note = match (hunk.old.is_empty(), app.git.changes.short_head()) {
        (true, _) => t!("git-change-popup-not-in-head").to_owned(),
        (false, Some(oid)) => format!("HEAD · {oid}"),
        (false, None) => "HEAD".to_owned(),
    };
    let mut counter = String::new();
    let _ = write!(counter, "{} / {count}", popup.hunk + 1);
    let measure =
        |frame: &mut Frame<'_>, s: &str| frame.measure(FontKind::Ui, app.fonts.ui_small, s);
    let (title_w, note_w, counter_w) = (
        measure(frame, &title),
        measure(frame, &note),
        measure(frame, &counter),
    );

    // 宽度取最长一行与标题栏中的较大者，不超出编辑区。
    let body_rows = shown + usize::from(more > 0);
    let header_rows = (HEADER / lh).ceil() as usize;
    let rows = header_rows + body_rows;
    // 逐行追溯打开时从追溯栏右侧开始，不遮挡追溯栏。
    let blame_w = app.blame_width();
    let x = gutter.x + blame_w + 2.0;
    let longest = preview
        .lines
        .iter()
        .map(|l| cells(&l.chars, l.chars.len(), tab))
        .max()
        .unwrap_or(0);
    let content_w = code_x + longest as f32 * cell + 24.0 - x;
    let header_w = 12.0 + 11.0 + title_w + 10.0 + note_w + 24.0 + counter_w + 6.0 * BUTTON + 30.0;
    let room = text_rect.right() - 16.0 - x;
    let w = content_w.max(header_w).max(360.0).min(room);

    // 位置（以显示行计）：优先位于改动各行的下方，空间不足时位于上方；两侧都放不下时限制在
    // 可见范围内。
    let line_count = doc.text().line_count();
    let (from, to) = (
        popup.anchor.0.min(line_count - 1),
        popup.anchor.1.min(line_count),
    );
    let top_row = doc.row_of(from) as f64;
    let bottom_row = if to > from {
        (doc.row_of(to - 1) + doc.rows_of(to - 1)) as f64
    } else if popup.anchor.0 >= line_count {
        // 删除位于文件末尾：分界在最后一行的下沿。
        (doc.row_of(from) + doc.rows_of(from)) as f64
    } else {
        top_row
    };
    let first = doc.scroll_line.ceil();
    let last = (doc.scroll_line + f64::from(text_rect.h / lh)).floor();
    let span = rows as f64;
    let start = if bottom_row + span <= last {
        bottom_row
    } else if top_row - span >= first {
        top_row - span
    } else {
        bottom_row.min(last - span).max(first)
    };
    let top = text_rect.y + (start - doc.scroll_line) as f32 * lh;
    let card = Rect::new(x, top + 1.0, w, rows as f32 * lh - 2.0);
    draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);

    // 标题栏：与行号栏标记同色的小色条、标题和淡色说明。
    let header = Rect::new(card.x, top, card.w, header_rows as f32 * lh);
    let (_, cy) = center(header);
    let chip = Rect::new(card.x + 12.0, cy - 6.0, 3.0, 12.0);
    frame.rect_clipped(chip, shade.mark, 1.5, card);
    let mut tx = chip.right() + 8.0;
    ui_text_small(frame, app, tx, header, &title, p.text, card);
    tx += title_w + 10.0;
    ui_text_small(frame, app, tx, header, &note, p.text_dim, card);

    // 按钮从右往左排列，各组之间以竖线分隔。
    let mut right = card.right() - 6.0;
    let mut hits = Vec::new();
    let mut button = |frame: &mut Frame<'_>, right: &mut f32, target: ChangeTarget| {
        let rect = Rect::new(*right - BUTTON, cy - BUTTON / 2.0, BUTTON, BUTTON);
        *right = rect.x - 2.0;
        let enabled = match target {
            ChangeTarget::Previous => popup.hunk > 0,
            ChangeTarget::Next => popup.hunk + 1 < count,
            _ => true,
        };
        let hovered = enabled && app.pointer.hover == Hover::Change(target);
        draw_button(frame, p, rect, card, hovered, false);
        let color = match (enabled, hovered) {
            (false, _) => p.text_dim.alpha(0.5),
            (true, true) => p.text,
            (true, false) => p.text_muted,
        };
        let c = center(rect);
        match target {
            ChangeTarget::Previous => icons::chevron(frame, c, 8.0, Direction::Up, color, card),
            ChangeTarget::Next => icons::chevron(frame, c, 8.0, Direction::Down, color, card),
            ChangeTarget::Revert => icons::rollback(frame, c, color, card),
            ChangeTarget::ShowDiff => icons::compare(frame, c, color, card),
            ChangeTarget::Close => icons::close(frame, c, 7.0, color, card),
        }
        if enabled {
            hits.push((rect, target));
        }
    };
    let divider = |frame: &mut Frame<'_>, right: &mut f32| {
        let line = Rect::new(*right - 4.0, cy - 7.0, 1.0, 14.0);
        frame.rect_clipped(line, p.separator, 0.0, card);
        *right -= 10.0;
    };
    button(frame, &mut right, ChangeTarget::Close);
    divider(frame, &mut right);
    button(frame, &mut right, ChangeTarget::ShowDiff);
    button(frame, &mut right, ChangeTarget::Revert);
    divider(frame, &mut right);
    button(frame, &mut right, ChangeTarget::Next);
    button(frame, &mut right, ChangeTarget::Previous);
    ui_text_small(
        frame,
        app,
        right - counter_w - 4.0,
        header,
        &counter,
        p.text_muted,
        card,
    );

    let body = Rect::new(card.x, header.bottom(), card.w, body_rows as f32 * lh);
    let (ascent, descent) = (app.metrics.mono.ascent, app.metrics.mono.descent);
    if !hunk.old.is_empty() {
        // 底色延伸到卡片底边时，下方两角随卡片绘制为圆角。
        let tint = Rect::new(body.x, body.y, body.w, shown as f32 * lh);
        if more == 0 {
            let tint = Rect::new(tint.x, tint.y, tint.w, card.bottom() - tint.y);
            frame.rect_clipped(tint, shade.line, RADIUS_POPUP, card);
            let square = Rect::new(tint.x, tint.y, tint.w, RADIUS_POPUP.min(tint.h));
            frame.rect_clipped(square, shade.line, 0.0, card);
        } else {
            frame.rect_clipped(tint, shade.line, 0.0, card);
        }
        let mut number = String::new();
        let clip = Rect::new(code_x, card.y, card.right() - 8.0 - code_x, card.h);
        for (i, line) in preview.lines.iter().enumerate() {
            let row = Rect::new(body.x, body.y + i as f32 * lh, body.w, lh);
            let baseline = baseline_in(row, ascent, descent);
            number.clear();
            let _ = write!(number, "{}", hunk.old.start + i + 1);
            let nx = number_right - number.len() as f32 * cell;
            frame.text(
                FontKind::Mono,
                app.fonts.code,
                nx,
                baseline,
                &number,
                p.gutter_text,
                card,
            );
            let sign = (gutter.x + blame_w + 10.0, center(row).1);
            icons::minus(frame, sign, 6.0, shade.mark, card);
            for range in &line.words {
                let x0 = code_x + cells(&line.chars, range.start, tab) as f32 * cell;
                let x1 = code_x + cells(&line.chars, range.end, tab) as f32 * cell;
                let word = Rect::new(x0, row.y + 2.0, (x1 - x0).max(2.0), lh - 4.0);
                frame.rect_clipped(word, shade.word, 3.0, clip);
            }
            draw_glyphs(
                app,
                frame,
                &line.chars,
                &line.tokens,
                code_x,
                baseline,
                tab,
                clip,
                &[],
            );
        }
        if more > 0 {
            let row = Rect::new(body.x, body.y + shown as f32 * lh, body.w, lh);
            let note = t!("git-change-popup-more-lines", count = more);
            ui_text_small(frame, app, code_x, row, &note, p.text_dim, card);
        }
    }

    app.hits.changes = hits;
    app.hits.changes_card = Some(card);
}

/// 返回 `chars` 的前 `col` 个字符占用的显示列数（制表符按 `tab` 列宽展开）。
fn cells(chars: &[char], col: usize, tab: usize) -> usize {
    text::slice_cells(&chars[..col.min(chars.len())], 0, tab)
}
