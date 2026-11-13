//! 差异的粘滞滚动：展开全部内容时，滚过函数、类的开头后把该行固定在顶部。
//!
//! 并排时两侧按各自的滚动位置分别计算，固定区域横贯两侧和中间一栏，高度取较深的一侧；单栏时
//! 按顶部可见行所属的一侧计算。块的识别、固定行数的上限、各行的位置和绘制方式与编辑区相同，
//! 见 `app::editor::sticky`。

use super::*;
use crate::app::{StickyRow, place_sticky_rows, sticky_limit};
use crate::theme::SCROLLBAR_HIT;

use super::super::editor::{draw_pinned_rows, draw_pinned_shadow, pinned_height};

/// 绘制固定在顶部的行，并把它们的可点击区域加到 `hits` 的最前面。
///
/// 可点击区域按先后匹配（见 `layout::hit`），固定行盖在正文之上，因此排在正文的区域之前。
/// 折叠了未改动内容、关闭了粘滞滚动或区域过矮时不绘制。
pub(super) fn draw(
    app: &mut App,
    frame: &mut Frame<'_>,
    view: &mut DiffView,
    parts: &DiffParts,
    scroll: Scroll,
    pos: f64,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let max = sticky_limit((parts.body.h / app.fonts.line_height) as usize);
    if view.collapses() || app.config.sticky_scroll == Some(false) || max == 0 {
        return;
    }
    let tops: Vec<(usize, f64)> = if view.is_unified() {
        let side = view.unified_row(pos.floor() as usize).map(|(side, _)| side);
        side.map(|side| (side, pos)).into_iter().collect()
    } else {
        scroll.rows.into_iter().enumerate().collect()
    };
    let sides: Vec<(usize, Vec<StickyRow>)> = tops
        .into_iter()
        .map(|(side, top)| (side, pinned_rows(app, view, side, top, max)))
        .filter(|(_, rows)| !rows.is_empty())
        .collect();
    // 两侧的固定区域合为一条：盖住中间一栏的带子，下沿取较深的一侧，阴影只有一道。
    let Some((_, deepest)) = sides.iter().max_by(|(_, a), (_, b)| {
        let height = |rows| pinned_height(app, rows).unwrap_or(0.0);
        height(a).total_cmp(&height(b))
    }) else {
        return;
    };
    let body = parts.body;
    let height = pinned_height(app, deepest).unwrap_or(0.0);
    draw_pinned_shadow(app, frame, body, deepest);
    let band = Rect::new(body.x, body.y, body.w, height);
    frame.rect_clipped(band, app.palette.editor_bg, 0.0, body);

    let lh = app.fonts.line_height;
    let mut pinned = Vec::new();
    for (side, rows) in &sides {
        draw_side(app, frame, view, parts, scroll.x, *side, rows);
        let pane = parts.panes[*side].rect;
        // 右侧留出滚动条的拖动区域。
        let width = (body.right() - SCROLLBAR_HIT - pane.x).clamp(0.0, pane.w);
        pinned.extend(rows.iter().map(|row| {
            let band = Rect::new(pane.x, pane.y + row.y, width, lh).intersect(pane);
            (band, DiffTarget::Sticky(*side, row.line))
        }));
    }
    // 分隔线贯穿固定区域，与正文中的位置一致；画在固定行的底色之上。
    draw_pane_rules(app, frame, view, parts, [body.y + height; 2]);
    hits.splice(0..0, pinned);
}

/// 返回第 `side` 侧在滚动位置 `top`（并排时为该侧的显示行，单栏时为单栏的位置）上方固定的行。
fn pinned_rows(
    app: &mut App,
    view: &mut DiffView,
    side: usize,
    top: f64,
    max: usize,
) -> Vec<StickyRow> {
    let first = top.floor() as usize;
    let lines = screen_lines(view, side, first, max + 1);
    if lines.is_empty() {
        return Vec::new();
    }
    let row = if view.is_unified() {
        view.unified_row(first).map(|(_, row)| row)
    } else {
        Some(first)
    };
    let part = match row.and_then(|row| view.rows[side].get(row)) {
        Some(&SideRow::Line { part, .. }) => part,
        _ => 0,
    };
    let partial = part > 0 || top.fract() > 0.0;
    let tab_width = app.editor.tab_width;
    let (scopes, settled) = view
        .side_mut(side)
        .sticky_scopes(&lines, partial, max, tab_width);
    app.caches.highlight_pending |= !settled;
    let lh = app.fonts.line_height;
    place_sticky_rows(&scopes, lh, |last| {
        ((first + last) as f64 - top) as f32 * lh
    })
}

/// 返回自显示行 `first` 起最多 `count` 个显示行各自对应的第 `side` 侧的文件行。
///
/// 每个显示行对应一项，自动换行时同一行重复出现：与屏幕上的位置一一对应，固定行据此判断
/// 哪些块仍在屏幕上。单栏时另一侧的行同样占用显示行：未改动的行取该侧对应的行；只在另一侧
/// 的新增或删除的行取上方最近的一行，它们在该侧的文件中位于这两行之间，所属的块相同。顶部的
/// 显示行不属于该侧时返回空表。
fn screen_lines(view: &DiffView, side: usize, first: usize, count: usize) -> Vec<usize> {
    if !view.is_unified() {
        return (first..first + count)
            .map_while(|row| view.line_at(side, row))
            .collect();
    }
    let mut lines: Vec<usize> = Vec::with_capacity(count);
    for index in first..first + count {
        let Some((at, row)) = view.unified_row(index) else {
            break;
        };
        let own = if at == side {
            view.line_at(side, row)
        } else if side == LEFT {
            unified_old_line(view, at, row, index as f64)
        } else {
            None
        };
        match own.or_else(|| lines.last().copied()) {
            Some(line) => lines.push(line),
            None => break,
        }
    }
    lines
}

/// 绘制第 `side` 侧的固定行：行号（单栏中未改动的行同时显示左侧行号）和带语法着色的文字。
fn draw_side(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    scroll_x: f32,
    side: usize,
    rows: &[StickyRow],
) {
    let pane = parts.panes[side];
    let source = view.side(side);
    let language = source.highlighter.language();
    let tab_width = app.editor.tab_width;
    let origin_x = pane.text.x + DIFF_TEXT_PAD - scroll_x;
    let (mut chars, mut tokens, mut number) = (Vec::new(), Vec::new(), String::new());
    let hovered =
        |row: &StickyRow| app.pointer.hover == Hover::Diff(DiffTarget::Sticky(side, row.line));
    draw_pinned_rows(
        app,
        frame,
        pane.rect,
        rows,
        hovered,
        |frame, i, baseline| {
            let line = rows[i].line;
            let row = view.row_of(side, line);
            if let Some(old) = unified_old_line(view, side, row, view.pos_of(side, row as f64)) {
                draw_line_number(app, frame, &parts.panes[LEFT], old, baseline, &mut number);
            }
            draw_line_number(app, frame, &pane, line, baseline, &mut number);
            line_chars(source, line, &mut chars);
            // 尚未推算到该行的高亮状态时按普通文本绘制，推算完成后的下一帧显示语法着色。
            match source.highlighter.known_state(line) {
                Some(state) => {
                    winc_syntax::highlight_line(language, &chars, state, &mut tokens);
                }
                None => tokens.clear(),
            }
            draw_glyphs(
                app,
                frame,
                &chars,
                &tokens,
                origin_x,
                baseline,
                tab_width,
                pane.text,
                &[],
            );
        },
    );
}
