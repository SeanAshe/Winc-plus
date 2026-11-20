//! 可编辑的预览中调整图片大小的绘制：悬停时的把手，拖动时的新大小与尺寸。

use winc_render::{Color, FontKind, Frame, Rect};

use crate::app::preview::markdown::MarkdownPreview;
use crate::app::preview::markdown::resize::{ImageAt, OUTLINE_GAP};
use crate::icons;
use crate::theme::Palette;

/// 绘制悬停图片的把手或拖动中的新大小；`images` 是本帧绘制的可调整图片，
/// `size` 是尺寸文字的字号。
pub(super) fn draw_image_resize(
    md: &MarkdownPreview,
    images: &[ImageAt],
    p: &Palette,
    size: f32,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    if let Some(r) = md.resizing {
        let image = r.image.rect;
        let h = (image.h * r.width / image.w.max(1.0)).round();
        let w = r.width.round();
        let x = if r.centered {
            (image.x + (image.w - w) / 2.0).round()
        } else {
            image.x
        };
        let shape = Rect::new(x, image.y, w, h);
        frame.rect_clipped(shape, p.accent.alpha(p.accent.a * 0.06), 0.0, area);
        draw_outline(frame, p.accent, shape, area);
        let label = format!("{} × {}", w as u32, h as u32);
        let text_w = frame.measure(FontKind::Ui, size, &label) + 12.0;
        let m = frame.metrics(FontKind::Ui, size);
        let pill_h = (m.ascent + m.descent + 6.0).round();
        // 尺寸位于框底部正中，避开右侧的把手。
        let pill = Rect::new(
            (shape.x + (shape.w - text_w) / 2.0).round(),
            shape.bottom() - pill_h - 6.0,
            text_w,
            pill_h,
        );
        frame.rect_clipped(pill, p.accent, pill_h / 2.0, area);
        let baseline = pill.y + (pill_h + m.ascent - m.descent) / 2.0;
        let x = pill.x + 6.0;
        frame.text(
            FontKind::Ui,
            size,
            x,
            baseline,
            &label,
            p.text_on_accent,
            area,
        );
        let moved = ImageAt {
            rect: shape,
            ..r.image
        };
        if let Some(handle) = moved.handle() {
            draw_handle(frame, p, handle, area);
        }
        return;
    }
    let Some(hovered) = md.image_hover else {
        return;
    };
    let Some(image) = images.iter().find(|i| i.rect == hovered) else {
        return;
    };
    if let Some(handle) = image.handle() {
        draw_handle(frame, p, handle, area);
    }
}

/// 绘制图片外 1 逻辑像素宽、颜色为 `color` 的外框。
fn draw_outline(frame: &mut Frame<'_>, color: Color, image: Rect, area: Rect) {
    let r = image.inset(-OUTLINE_GAP, -OUTLINE_GAP).inset(0.5, 0.5);
    let lines = icons::rounded_outline(r.x, r.y, r.w, r.h, 2.0);
    frame.stroke(&lines, 1.0, color, area);
}

/// 绘制把手。
fn draw_handle(frame: &mut Frame<'_>, p: &Palette, handle: Rect, area: Rect) {
    frame.rect_clipped(handle, p.text.alpha(p.text.a * 0.35), handle.w / 2.0, area);
}
