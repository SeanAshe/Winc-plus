//! LaTeX 的预览：编译出的 PDF、第一次编译的说明页或阅读视图，右上角是编译状态。

use winc_render::{FontKind, Frame, Rect};

use crate::app::{App, Hover};
use crate::theme::RADIUS_POPUP;

/// 编译状态离预览右上角的距离（逻辑像素）。
const MARGIN: f32 = 12.0;
/// 编译状态的高度和左右的留白（逻辑像素）。
const HEIGHT: f32 = 28.0;
const PAD: f32 = 10.0;

/// 在 `area` 里画当前标签的 LaTeX 预览。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    if app.latex_pdf().is_some() {
        super::pdf::draw(app, frame, area);
    } else if app.latex_waiting() {
        app.hits.latex_status = Rect::default();
        app.hide_latex_status();
        frame.rect(area, app.palette.editor_bg);
        let message = app
            .latex_status()
            .map_or_else(String::new, |(text, _)| text);
        crate::view::editor::draw_waiting(app, frame, area, &message);
        return;
    } else {
        super::markdown::draw(app, frame, area);
    }
    draw_status(app, frame, area);
}

/// 在右上角画编译状态。
fn draw_status(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    app.hits.latex_status = Rect::default();
    let Some((text, error)) = app.latex_status() else {
        app.hide_latex_status();
        return;
    };
    let p = app.palette;
    let target = frame.measure(FontKind::Ui, app.fonts.ui_small, &text) + PAD * 2.0;
    let w = app.latex_status_width(target).round();
    let bar = Rect::new(area.right() - MARGIN - w, area.y + MARGIN, w, HEIGHT);
    crate::view::draw_floating_bar(frame, p, bar, area);
    if error && app.pointer.hover == Hover::LatexStatus {
        let inset = Rect::new(bar.x + 2.0, bar.y + 2.0, bar.w - 4.0, bar.h - 4.0);
        frame.rect_clipped(inset, p.hover, RADIUS_POPUP - 2.0, area);
    }
    let color = if error { p.error } else { p.text_muted };
    let clip = bar.intersect(area);
    crate::view::ui_text_small(frame, app, bar.x + PAD, bar, &text, color, clip);
    if error {
        app.hits.latex_status = bar;
    }
}
