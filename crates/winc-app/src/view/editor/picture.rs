//! 图片和 SVG 的预览、分栏预览。

use winc_i18n::t;

use super::*;
use crate::app::preview::images::PictureCache;

/// SVG 小图标放大后长边的最小尺寸（逻辑像素）。
///
/// 矢量图放大后不会模糊，小图标按原大小显示时过小，因此至少把长边放大到该尺寸。
const VECTOR_MIN: f32 = 256.0;

/// 绘制分栏预览：编辑区右边那一半，左边一道分隔线，中间是跟着文字重绘的图（`App::split`）。
///
/// 写到一半解析不了时仍然绘制上一张图，底下一行写出错在哪里；一张都还没绘制出来时只写原因。
pub(in crate::view) fn draw_split(app: &mut App, frame: &mut Frame<'_>) {
    let area = app.layout.split;
    let Some(doc) = app.editor.active().filter(|_| area.w > 0.0) else {
        return;
    };
    let (id, kind) = (doc.id, doc.preview_kind());
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    // 悬停、拖动分隔线时用强调色，表示可以左右拖动。
    let active = app.split_divider_active();
    let line = if active { p.accent } else { p.border };
    frame.rect(Rect::new(area.x - 1.0, area.y, 1.0, area.h), line);
    if kind == Some(winc_editor::PreviewKind::Markdown) {
        crate::view::preview::markdown::draw(app, frame, area);
        return;
    }
    if kind == Some(winc_editor::PreviewKind::Table) {
        // 尚未读好时留空，读好后会重新绘制。
        if let Some(index) = app.editor.index_of(id) {
            crate::view::preview::table::draw(app, frame, index, area);
        }
        return;
    }
    if app.split.doc != Some(id) {
        return;
    }
    if let Some(picture) = app.split.picture.clone() {
        draw_picture(app, frame, id, &picture, area);
    }
    let Some(reason) = app.split.error.clone() else {
        return;
    };
    let message = t!("editor-preview-failed", reason = reason);
    let line = Rect::new(area.x + 16.0, area.bottom() - 32.0, area.w - 32.0, 20.0);
    let text = ellipsize(frame, &message, app.fonts.ui_small, line.w);
    ui_text_small(frame, app, line.x, line, &text, p.error, area);
}

/// 绘制图片预览：图片放在编辑区正中，四边各留 `MARGIN`；比空白区域大的按比例缩小，小的按原
/// 大小显示（一个图片像素对应一个逻辑像素）。
///
/// 有透明区域的图片底下铺棋盘格。
pub(super) fn draw_picture(
    app: &mut App,
    frame: &mut Frame<'_>,
    id: winc_editor::DocumentId,
    picture: &Arc<winc_preview::image::Picture>,
    editor: Rect,
) {
    // 编辑区四周的留白与棋盘格的边长（逻辑像素）。
    const MARGIN: f32 = 24.0;
    const CHECKER: f32 = 8.0;

    let p = app.palette;
    let room = editor.inset(MARGIN, MARGIN);
    if room.w <= 0.0 || room.h <= 0.0 {
        return;
    }
    let (w, h) = (picture.width as f32, picture.height as f32);
    // 缩小比例：位图按原大小显示，矢量图至少放大到长边 `VECTOR_MIN`。
    let natural = match picture.source {
        Source::Vector(_) => (VECTOR_MIN / w.max(h)).max(1.0),
        Source::Raster(_) => 1.0,
    };
    let fit = (room.w / w).min(room.h / h).min(natural);
    let (lw, lh) = ((w * fit).max(1.0), (h * fit).max(1.0));
    let scale = frame.scale();
    let (pw, ph) = (
        (lw * scale).round().max(1.0) as u32,
        (lh * scale).round().max(1.0) as u32,
    );
    let x = (editor.x + (editor.w - lw) / 2.0).round();
    let y = (editor.y + (editor.h - lh) / 2.0).round();
    let rect = Rect::new(x, y, pw as f32 / scale, ph as f32 / scale);

    if picture.transparent {
        // 底色上隔一格叠一层悬停色；底色不透明，叠好的颜色也不透明，整片一遍填完。
        let (base, over) = (p.selected_inactive, p.hover);
        let mix = |b: f32, o: f32| b + (o - b) * over.a;
        let tinted = Color {
            r: mix(base.r, over.r),
            g: mix(base.g, over.g),
            b: mix(base.b, over.b),
            a: base.a,
        };
        frame.checker(rect, CHECKER, (tinted, base), editor);
    }

    // 缓存中是否是这张图；是这张图时，记录其当前尺寸和是否按当前目标尺寸绘制。
    let same: Option<((u32, u32), bool)> = app
        .picture_cache
        .as_ref()
        .filter(|c| c.doc == id && Arc::ptr_eq(&c.picture, picture))
        .map(|c| ((c.pixels.width, c.pixels.height), c.exact));
    if same != Some(((pw, ph), true)) {
        match picture.source {
            // 位图缩放耗时很短，直接在当前线程缩放。
            Source::Raster(_) => {
                app.picture_cache = Some(PictureCache {
                    doc: id,
                    picture: Arc::clone(picture),
                    pixels: picture.render(pw, ph),
                    exact: true,
                });
            }
            // SVG 带滤镜时光栅化一次可能耗时较长：交给后台线程，同时先把上一次的结果缩放到
            // 当前尺寸暂用。
            Source::Vector(_) => {
                app.render_picture(id, picture, (pw, ph));
                match (same, app.picture_cache.as_mut()) {
                    (Some((size, _)), Some(cache)) if size != (pw, ph) => {
                        cache.pixels = winc_preview::image::resize(&cache.pixels, pw, ph);
                        cache.exact = false;
                    }
                    (Some(_), _) => {}
                    (None, _) => app.picture_cache = None,
                }
            }
        }
    }
    if let Some(cache) = &app.picture_cache {
        let px = &cache.pixels;
        frame.pixels((rect.x, rect.y), px.width, px.height, &px.rgba, editor);
    }
}

#[cfg(test)]
mod tests;
