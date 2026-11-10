//! 粘滞滚动：固定在编辑区顶部的若干行。

use super::*;
use crate::app::StickyRow;

/// 绘制固定在编辑区顶部的行：覆盖在已滚出上方的代码之上，行号仍然显示，下沿的淡阴影把
/// 固定行与滚动的代码分开。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_sticky(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let rows = app.sticky_rows();
    app.hits.sticky.clone_from(&rows);
    if rows.is_empty() {
        return;
    }

    let layout = app.layout;
    let text_rect = layout.text;
    // 固定行覆盖到编辑区右边缘，阴影同样横贯整个宽度；滚动条之后绘制在固定行之上。
    let area = Rect::new(layout.editor.x, text_rect.y, layout.editor.w, text_rect.h);
    let states: Vec<Option<LineState>> = rows
        .iter()
        .map(|row| app.editor.documents[index].highlight_state(row.line, HIGHLIGHT_BUDGET))
        .collect();

    let app = &*app;
    let p = app.palette;
    let doc = &app.editor.documents[index];
    let text = doc.text();
    let tab_width = doc.tab_width(app.editor.tab_width);
    let language = doc.highlighter.language();
    let origin_x = text_rect.x + TEXT_PAD - doc.scroll_x;
    let gutter = Rect::new(layout.gutter.x, area.y, layout.gutter.w, area.h);
    let clip = Rect::new(text_rect.x, area.y, text_rect.w, area.h);
    let mut chars: Vec<char> = Vec::with_capacity(256);
    let mut tokens = Vec::with_capacity(32);

    let hovered = |row: &StickyRow| app.hover == Hover::Sticky(row.depth);
    draw_pinned_shadow(app, frame, area, &rows);
    draw_pinned_rows(app, frame, area, &rows, hovered, |frame, i, baseline| {
        let row = rows[i];
        if app.config.line_numbers != Some(false) {
            let number = (row.line + 1).to_string();
            let number_x = layout.gutter.right() - GUTTER_PADDING - number.len() as f32 * app.cell;
            frame.text(
                FontKind::Mono,
                app.fonts.code,
                number_x,
                baseline,
                &number,
                p.gutter_text,
                gutter,
            );
        }
        chars.clear();
        chars.extend(
            text.line_prefix(row.line, MAX_LINE_BYTES)
                .slice()
                .chars()
                .take(MAX_LINE_CHARS),
        );
        // 尚未推算到该行的高亮状态时按普通文本绘制，推算完成后的下一帧显示语法着色。
        match states[i] {
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
            clip,
            &[],
        );
    });
}

/// 返回固定行 `rows` 占用的高度（逻辑像素，从固定区域的顶部算起）；没有固定行时为 `None`。
pub(in crate::view) fn pinned_height(app: &App, rows: &[StickyRow]) -> Option<f32> {
    let bottom = rows.iter().map(|r| r.y).reduce(f32::max)?;
    Some(bottom + app.fonts.line_height)
}

/// 在区域 `area` 顶部的固定行 `rows` 下沿绘制淡阴影，把固定行与滚动的内容分开。
pub(in crate::view) fn draw_pinned_shadow(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    rows: &[StickyRow],
) {
    let Some(height) = pinned_height(app, rows) else {
        return;
    };
    frame.within(area, |frame| {
        let header = Rect::new(area.x, area.y, area.w, height);
        let covered = frame.covered(header, 0.0);
        frame.shadow(header, 0.0, 6.0, 0.0, app.palette.shadow, Some(covered));
    });
}

/// 绘制固定在区域 `area` 顶部的行 `rows`（纵坐标相对 `area` 的顶边）：底色和悬停底色，之后
/// 逐行调用 `content` 绘制行号与文字，参数为行在 `rows` 中的序号和基线。
///
/// 被向上推出的一层位于上一层之下，因此按从内层到外层的顺序绘制。编辑区与差异视图共用；
/// 下沿的阴影另由 [`draw_pinned_shadow`] 绘制。
pub(in crate::view) fn draw_pinned_rows(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    rows: &[StickyRow],
    hovered: impl Fn(&StickyRow) -> bool,
    mut content: impl FnMut(&mut Frame<'_>, usize, f32),
) {
    let p = app.palette;
    let line_height = app.fonts.line_height;
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    frame.within(area, |frame| {
        for (i, row) in rows.iter().enumerate().rev() {
            let band = Rect::new(area.x, area.y + row.y, area.w, line_height);
            frame.rect_clipped(band, p.editor_bg, 0.0, area);
            if hovered(row) {
                let hover = Rect::new(area.x + 6.0, band.y, area.w - 12.0, line_height);
                frame.rect_clipped(hover, p.hover, RADIUS_ROW, area);
            }
            content(frame, i, baseline_in(band, ascent, descent));
        }
    });
}
