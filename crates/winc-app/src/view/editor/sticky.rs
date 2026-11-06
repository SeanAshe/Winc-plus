//! 粘滞滚动：固定在编辑区顶部的几行。

use super::*;

/// 绘制固定在编辑区顶部的几行：覆盖在滚到上方的代码之上，行号仍然显示，下沿一道淡阴影把
/// 固定行与滚动的代码分开。
///
/// 被推上去的那一层压在上一层之下，因此按从内层到外层的顺序绘制。悬停的行铺悬停底色。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_sticky(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let rows = app.sticky_rows();
    app.sticky.clone_from(&rows);
    let Some(bottom) = rows.iter().map(|r| r.y).reduce(f32::max) else {
        return;
    };

    let layout = app.layout;
    let p = app.palette;
    let line_height = app.fonts.line_height;
    let text_rect = layout.text;
    // 固定行一直盖到编辑区右边缘，阴影也是整条；滚动条随后绘制在固定行之上。
    let area = Rect::new(layout.editor.x, text_rect.y, layout.editor.w, text_rect.h);
    let states: Vec<Option<LineState>> = rows
        .iter()
        .map(|row| app.editor.documents[index].highlight_state(row.line, HIGHLIGHT_BUDGET))
        .collect();

    let app = &*app;
    let doc = &app.editor.documents[index];
    let text = doc.text();
    let tab_width = doc.tab_width(app.editor.tab_width);
    let language = doc.highlighter.language();
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let origin_x = text_rect.x + TEXT_PAD - doc.scroll_x;
    let mut chars: Vec<char> = Vec::with_capacity(256);
    let mut tokens = Vec::with_capacity(32);

    frame.within(area, |frame| {
        let header = Rect::new(area.x, area.y, area.w, bottom + line_height);
        let covered = frame.covered(header, 0.0);
        frame.shadow(header, 0.0, 6.0, 0.0, p.shadow, Some(covered));

        for (row, state) in rows.iter().zip(&states).rev() {
            let y = text_rect.y + row.y;
            let band = Rect::new(area.x, y, area.w, line_height);
            frame.rect_clipped(band, p.editor_bg, 0.0, area);
            if app.hover == Hover::Sticky(row.depth) {
                let hover = Rect::new(area.x + 6.0, y, area.w - 12.0, line_height);
                frame.rect_clipped(hover, p.hover, RADIUS_ROW, area);
            }
            let baseline = baseline_in(band, ascent, descent);

            if app.config.line_numbers != Some(false) {
                let number = (row.line + 1).to_string();
                let number_x =
                    layout.gutter.right() - GUTTER_PADDING - number.len() as f32 * app.cell;
                let gutter = Rect::new(layout.gutter.x, area.y, layout.gutter.w, area.h);
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
                text.line_content(row.line)
                    .slice()
                    .chars()
                    .take(MAX_LINE_CHARS),
            );
            // 尚未推算到这一行的高亮状态时先按普通文本绘制，推算完成后的下一帧就有颜色了。
            match state {
                Some(state) => {
                    winc_syntax::highlight_line(language, &chars, *state, &mut tokens);
                }
                None => tokens.clear(),
            }
            let clip = Rect::new(text_rect.x, area.y, text_rect.w, area.h);
            draw_glyphs(
                app, frame, &chars, &tokens, origin_x, baseline, tab_width, clip, &[],
            );
        }
    });
}
