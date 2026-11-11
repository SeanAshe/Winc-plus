//! 图片和 SVG 的预览、分栏预览。

use winc_i18n::t;

use super::*;
use crate::app::preview::images::{PictureCache, Raster};
use winc_preview::image::Pixels;
use winc_preview::svg;

/// 棋盘格的边长（逻辑像素）。
const CHECKER: f32 = 8.0;
/// SVG 光栅化的范围在可见部分四周多出的宽度（逻辑像素）。
///
/// 平移不超过该宽度时沿用已有的位图，不重新光栅化。取自调参：64 时位图比可见部分每边多
/// 64 逻辑像素，1400 × 900 的预览区域中内存约增加四分之一；更大的值使内存按面积增长。
const VECTOR_OVERSCAN: f32 = 64.0;
/// SVG 整张图的低分辨率位图长边的上限（物理像素）。
///
/// 该位图只在清晰的位图尚未覆盖的位置短暂显示。取自调参：1024 时位图不超过 4 MB，放大到
/// 4 倍屏幕宽时每个像素约占 8 个物理像素，仍能辨认内容。
const BASE_EDGE: u32 = 1024;

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

/// 绘制图片预览：位置与大小由缩放倍数和滚动位置决定（`PictureView::rect`），
/// 超出 `editor` 的部分不绘制。
///
/// 有透明区域的图片底下铺棋盘格。位图和缓存的大小与缩放倍数无关，见 [`draw_raster`]、
/// [`draw_vector`]。缩放过渡期间每帧的大小都不同：光栅化只按过渡结束后的大小进行一次，
/// 过渡中把已有的位图缩放到这一帧的位置。
pub(super) fn draw_picture(
    app: &mut App,
    frame: &mut Frame<'_>,
    id: winc_editor::DocumentId,
    picture: &Arc<winc_preview::image::Picture>,
    editor: Rect,
) {
    let view = &app.picture_view;
    let (Some(shown), Some(goal)) = (view.rect(picture, editor), view.settled(picture, editor))
    else {
        return;
    };
    let p = app.palette;
    let scale = frame.scale();
    let shown = Placed::new(shown, scale, editor);
    let goal = Placed::new(goal, scale, editor);

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
        frame.checker(shown.whole(), CHECKER, (tinted, base), editor);
    }
    match &picture.source {
        Source::Raster(pixels) => draw_raster(app, frame, (id, picture), pixels, (&shown, &goal)),
        Source::Vector(_) => draw_vector(app, frame, (id, picture), (&shown, &goal)),
    }
}

/// 图片在窗口中的位置，对齐到物理像素。
struct Placed {
    /// 图片左上角在窗口中的位置（物理像素）。
    origin: (i32, i32),
    /// 图片的显示大小（宽、高，物理像素）。
    size: (u32, u32),
    /// 每个逻辑像素对应的物理像素数。
    scale: f32,
    /// 预览区域（窗口坐标，逻辑像素）；超出的部分不绘制。
    clip: Rect,
}

impl Placed {
    /// 把位置 `rect`（窗口坐标，逻辑像素）对齐到物理像素。
    ///
    /// 位置和大小分别取整：大小不随滚动变化，滚动时沿用同一份位图。
    fn new(rect: Rect, scale: f32, clip: Rect) -> Placed {
        Placed {
            origin: (
                (rect.x * scale).round() as i32,
                (rect.y * scale).round() as i32,
            ),
            size: (
                (rect.w * scale).round().max(1.0) as u32,
                (rect.h * scale).round().max(1.0) as u32,
            ),
            scale,
            clip,
        }
    }

    /// 返回图片中自 `at` 起、大小为 `size` 的部分（物理像素，相对图片左上角）在窗口中的位置
    /// （逻辑像素）。
    fn rect(&self, at: (u32, u32), size: (u32, u32)) -> Rect {
        Rect::new(
            (self.origin.0 as f32 + at.0 as f32) / self.scale,
            (self.origin.1 as f32 + at.1 as f32) / self.scale,
            size.0 as f32 / self.scale,
            size.1 as f32 / self.scale,
        )
    }

    /// 返回整张图片在窗口中的位置（逻辑像素）。
    fn whole(&self) -> Rect {
        self.rect((0, 0), self.size)
    }

    /// 返回图片位于预览区域内的部分：左上角和右下角（物理像素，相对图片左上角）；图片不在
    /// 预览区域内时返回 `None`。
    fn seen(&self) -> Option<[u32; 4]> {
        let within = |from: f32, to: f32, origin: i32, size: u32| {
            let first = (from * self.scale).floor() as i64 - i64::from(origin);
            let last = (to * self.scale).ceil() as i64 - i64::from(origin);
            let limit = i64::from(size);
            (first.clamp(0, limit) as u32, last.clamp(0, limit) as u32)
        };
        let clip = self.clip;
        let (x0, x1) = within(clip.x, clip.right(), self.origin.0, self.size.0);
        let (y0, y1) = within(clip.y, clip.bottom(), self.origin.1, self.size.1);
        (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
    }

    /// 返回光栅化的结果 `raster` 在窗口中的位置（逻辑像素）：光栅化的大小与显示大小不同时
    /// 按比例换算，两端向外取整到物理像素。
    fn part(&self, raster: &Raster) -> Rect {
        let (px, of) = (&raster.pixels, raster.size);
        if of == self.size {
            return self.rect(raster.origin, (px.width, px.height));
        }
        let at = (
            convert(raster.origin.0, of.0, self.size.0, false),
            convert(raster.origin.1, of.1, self.size.1, false),
        );
        let end = (
            convert(raster.origin.0 + px.width, of.0, self.size.0, true),
            convert(raster.origin.1 + px.height, of.1, self.size.1, true),
        );
        self.rect(at, (end.0 - at.0, end.1 - at.1))
    }
}

/// 把整张图为 `of` 像素时的位置 `at` 换算到整张图为 `to` 像素时的位置，向上（`up`）或向下
/// 取整，不超过 `to`。
fn convert(at: u32, of: u32, to: u32, up: bool) -> u32 {
    let at = f64::from(at) * f64::from(to) / f64::from(of.max(1));
    (if up { at.ceil() } else { at.floor() } as u32).min(to)
}

/// 把位图 `pixels` 绘制到 `rect`（逻辑像素）：大小相同时逐像素绘制，否则取最近的像素缩放。
fn blit(frame: &mut Frame<'_>, rect: Rect, size: (u32, u32), pixels: &Pixels, clip: Rect) {
    if size == (pixels.width, pixels.height) {
        frame.pixels((rect.x, rect.y), size.0, size.1, &pixels.rgba, clip);
    } else {
        frame.pixels_scaled(rect, pixels.width, pixels.height, &pixels.rgba, clip);
    }
}

/// 绘制位图；`shown` 是这一帧的位置，`goal` 是缩放过渡结束后的位置。
///
/// 显示大小不小于解码后保留的像素时直接取最近的像素绘制，只处理预览区域内的部分，不另存
/// 位图。缩小时使用按面积平均缩小后的缓存，大小不超过保留的像素；每个缩放倍数缩小一次，
/// 耗时很短，直接在当前线程进行。
fn draw_raster(
    app: &mut App,
    frame: &mut Frame<'_>,
    (id, picture): (winc_editor::DocumentId, &Arc<winc_preview::image::Picture>),
    pixels: &Pixels,
    (shown, goal): (&Placed, &Placed),
) {
    let (size, clip) = (goal.size, shown.clip);
    let rect = shown.whole();
    let cached = (app.picture_cache.as_ref())
        .filter(|c| c.doc == id && Arc::ptr_eq(&c.picture, picture))
        .and_then(|c| c.sharp.as_ref())
        .map(|sharp| sharp.size);
    // 大小是否不小于保留的像素，可以直接绘制。
    let direct = |size: (u32, u32)| size.0 >= pixels.width && size.1 >= pixels.height;
    if direct(shown.size) || (direct(size) && cached.is_none()) {
        // 过渡结束后也直接绘制时释放缩小后的缓存；向缩小过渡时留给之后的各帧。
        if direct(size) {
            app.picture_cache = None;
        }
        return blit(frame, rect, shown.size, pixels, clip);
    }
    // 这一帧小于保留的像素：绘制按面积平均缩小后的缓存，直接缩小原图会出现锯齿。向放大过渡
    // 时沿用过渡开始前的缓存，不为中间的大小重新缩小。
    if !direct(size) && cached != Some(size) {
        app.picture_cache = Some(PictureCache {
            doc: id,
            picture: Arc::clone(picture),
            sharp: Some(Raster {
                pixels: picture.render(size.0, size.1),
                size,
                origin: (0, 0),
            }),
            base: None,
        });
    }
    if let Some(sharp) = app.picture_cache.as_ref().and_then(|c| c.sharp.as_ref()) {
        blit(frame, rect, shown.size, &sharp.pixels, clip);
    }
}

/// 返回大小 `size`（宽、高，像素）按比例缩小到长边不超过 `limit` 的大小；未超过时原样返回。
fn bounded(size: (u32, u32), limit: u32) -> (u32, u32) {
    let long = size.0.max(size.1);
    if long <= limit {
        return size;
    }
    let side = |length: u32| convert(length, long, limit, false).max(1);
    (side(size.0), side(size.1))
}

/// 绘制 SVG；`shown` 是这一帧的位置，`goal` 是缩放过渡结束后的位置。
///
/// 按 `goal` 的大小光栅化预览区域内的部分，四周多出 `VECTOR_OVERSCAN`：位图的大小取决于预览
/// 区域，与缩放倍数无关。光栅化的长边不超过 [`svg::MAX_EDGE`]，超过时按比例缩小后放大显示；
/// 缩放的各档已受该上限限制，只有放大后又增大窗口时才会超过。
///
/// 带滤镜的 SVG 光栅化一次可能耗时较长，交给后台线程。完成之前、以及平移超出已光栅化的范围
/// 时，把已有的位图缩放到这一帧的位置，其余位置显示整张图的低分辨率位图
/// （`PictureCache::base`）；长边不超过 `BASE_EDGE` 的图整张光栅化，不需要低分辨率位图。
fn draw_vector(
    app: &mut App,
    frame: &mut Frame<'_>,
    (id, picture): (winc_editor::DocumentId, &Arc<winc_preview::image::Picture>),
    (shown, goal): (&Placed, &Placed),
) {
    let Some(seen) = goal.seen() else {
        return;
    };
    let raster = bounded(goal.size, svg::MAX_EDGE);
    let small = raster.0.max(raster.1) <= BASE_EDGE;
    let [x0, y0, x1, y1] = if small {
        [0, 0, raster.0, raster.1]
    } else {
        [
            convert(seen[0], goal.size.0, raster.0, false),
            convert(seen[1], goal.size.1, raster.1, false),
            convert(seen[2], goal.size.0, raster.0, true),
            convert(seen[3], goal.size.1, raster.1, true),
        ]
    };
    let same = app
        .picture_cache
        .as_ref()
        .filter(|c| c.doc == id && Arc::ptr_eq(&c.picture, picture));
    let covered = same.and_then(|c| c.sharp.as_ref()).is_some_and(|r| {
        let (right, bottom) = (r.origin.0 + r.pixels.width, r.origin.1 + r.pixels.height);
        r.size == raster && r.origin.0 <= x0 && r.origin.1 <= y0 && right >= x1 && bottom >= y1
    });
    let based = small || same.is_some_and(|c| c.base.is_some());
    if same.is_none() {
        app.picture_cache = None;
    }
    // 低分辨率位图先于可见的部分绘制：它的像素少，完成得早，等待期间不留空白。
    if !based {
        let size = bounded(raster, BASE_EDGE);
        app.render_picture(id, picture, size, [0, 0, size.0, size.1], true);
    } else if !covered {
        let over = if small {
            0
        } else {
            (VECTOR_OVERSCAN * goal.scale).round() as u32
        };
        let (left, top) = (x0.saturating_sub(over), y0.saturating_sub(over));
        let (right, bottom) = ((x1 + over).min(raster.0), (y1 + over).min(raster.1));
        let region = [left, top, right - left, bottom - top];
        app.render_picture(id, picture, raster, region, false);
    }

    let Some(cache) = &app.picture_cache else {
        return;
    };
    let visible = shown.whole().intersect(shown.clip);
    let sharp = cache.sharp.as_ref().map(|r| (r, shown.part(r)));
    if let Some(base) = &cache.base {
        // 只画在清晰的位图之外：半透明的内容叠画两次会变深。
        for strip in outside(visible, sharp.as_ref().map(|(_, at)| *at)) {
            let whole = shown.whole();
            frame.pixels_scaled(whole, base.width, base.height, &base.rgba, strip);
        }
    }
    if let Some((raster, at)) = sharp {
        let px = &raster.pixels;
        if raster.size == shown.size {
            frame.pixels((at.x, at.y), px.width, px.height, &px.rgba, shown.clip);
        } else {
            frame.pixels_scaled(at, px.width, px.height, &px.rgba, shown.clip);
        }
    }
}

/// 返回 `area` 中位于 `hole` 之外的部分：`hole` 上方、下方各一条，左右各一条；宽或高为 0 的
/// 不返回。`hole` 为 `None` 或与 `area` 不相交时返回整个 `area`。
fn outside(area: Rect, hole: Option<Rect>) -> Vec<Rect> {
    let hole = hole.map(|hole| hole.intersect(area));
    let Some(hole) = hole.filter(|hole| hole.w > 0.0 && hole.h > 0.0) else {
        return vec![area];
    };
    let strips = [
        Rect::new(area.x, area.y, area.w, hole.y - area.y),
        Rect::new(area.x, hole.bottom(), area.w, area.bottom() - hole.bottom()),
        Rect::new(area.x, hole.y, hole.x - area.x, hole.h),
        Rect::new(hole.right(), hole.y, area.right() - hole.right(), hole.h),
    ];
    strips
        .into_iter()
        .filter(|strip| strip.w > 0.0 && strip.h > 0.0)
        .collect()
}

#[cfg(test)]
mod tests;
