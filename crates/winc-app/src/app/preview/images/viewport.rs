//! 图片预览的缩放与滚动：倍数、滚动位置，以及由二者决定的图片位置。
//!
//! 操作方式与 PDF 预览相同：滚动平移；按住主修饰键滚动、触控板捏合或 `⌘+`、`⌘−` 按档缩放，
//! `⌘0` 还原，缩放时鼠标处的内容保持在原位。图片超出预览区域时还可以按住左键拖动，双击还原。
//! 绘制见 `view::editor::picture`。

use std::sync::Arc;

use winc_editor::{DocumentId, PreviewMode};
use winc_i18n::t;
use winc_preview::image::{Picture, Source};
use winc_preview::svg;
use winc_render::{Rect, Renderer};

use crate::app::animation::{Clock, PDF_MOTION};
use crate::app::preview::pdf::PINCH_STEP;
use crate::app::{App, Drag, Focus};

/// 图片四周的留白（逻辑像素）：适应预览区域时图片与区域边缘的最小距离，
/// 放大后滚动到边缘时同样留出。
const MARGIN: f32 = 24.0;
/// SVG 适应预览区域时长边的最小尺寸（逻辑像素）。
///
/// 矢量图放大后不会模糊，小图标按原大小显示时过小，因此至少把长边放大到该尺寸。
const VECTOR_MIN: f32 = 256.0;
/// 可选的缩放倍数，1 为适应预览区域。
///
/// 不超过 2 的各档与 PDF 预览相同。上限为 32：按原大小显示的位图在该倍数下每个图片像素占
/// 32 个逻辑像素，足以分辨单个像素。SVG 另受光栅化大小的限制，见 [`App::zoom_picture`]。
const ZOOMS: [f32; 15] = [
    0.5, 0.67, 0.8, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0,
];

/// 缩放的过渡：从倍数 `from` 到 [`PictureView::zoom`]，期间图片中相对位置 `at`（宽、高的
/// 比例）处保持在 `anchor`（窗口坐标）；内容的范围不允许时取最接近的位置。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoomMotion {
    from: f32,
    pub(crate) clock: Clock,
    at: (f32, f32),
    anchor: (f32, f32),
}

/// 当前图片预览的缩放与滚动位置。
pub(crate) struct PictureView {
    /// 预览的文档；当前标签显示其他文档的图片时还原为适应预览区域。
    doc: Option<DocumentId>,
    /// 缩放倍数（见 `ZOOMS`），1 为适应预览区域；过渡期间为过渡结束后的倍数。
    zoom: f32,
    /// 滚动位置（逻辑像素）：内容左上角到预览区域左上角的横向、竖直距离；
    /// 过渡期间为过渡结束后的位置。
    ///
    /// 内容为图片加四周的留白，不小于预览区域；读取时限制在内容的范围内（[`Self::settled`]）。
    scroll: (f32, f32),
    /// 触控板捏合累计的缩放比例，见 `PINCH_STEP`。
    pinch: f32,
    /// 进行中的缩放过渡。
    pub(crate) zoom_motion: Option<ZoomMotion>,
}

impl Default for PictureView {
    fn default() -> Self {
        Self {
            doc: None,
            zoom: 1.0,
            scroll: (0.0, 0.0),
            pinch: 0.0,
            zoom_motion: None,
        }
    }
}

/// 返回图片 `picture` 适应区域 `area` 时的显示大小（逻辑像素）；`area` 容不下留白时返回 `None`。
///
/// 比可用区域大的图片按比例缩小；小的位图按原大小显示（一个图片像素对应一个逻辑像素），小的
/// SVG 长边至少放大到 `VECTOR_MIN`。
fn fitted(picture: &Picture, area: Rect) -> Option<(f32, f32)> {
    let room = area.inset(MARGIN, MARGIN);
    if room.w <= 0.0 || room.h <= 0.0 {
        return None;
    }
    let (w, h) = (picture.width.max(1) as f32, picture.height.max(1) as f32);
    let natural = match picture.source {
        Source::Vector(_) => (VECTOR_MIN / w.max(h)).max(1.0),
        Source::Raster(_) => 1.0,
    };
    let fit = (room.w / w).min(room.h / h).min(natural);
    Some(((w * fit).max(1.0), (h * fit).max(1.0)))
}

/// 返回倍数为 `zoom` 时横向、竖直的最大滚动位置（逻辑像素）：内容超出 `area` 的宽度和高度。
fn range(zoom: f32, fit: (f32, f32), area: Rect) -> (f32, f32) {
    let beyond = |size: f32, room: f32| (size * zoom + MARGIN * 2.0 - room).max(0.0);
    (beyond(fit.0, area.w), beyond(fit.1, area.h))
}

/// 返回倍数为 `zoom`、滚动位置为 `scroll` 时图片在 `area` 中的位置（窗口坐标，逻辑像素）。
///
/// 图片在内容中居中；内容超出 `area` 的方向按滚动位置移动（限制在内容的范围内），
/// 不超出的方向图片在 `area` 中居中。
fn placed(zoom: f32, scroll: (f32, f32), fit: (f32, f32), area: Rect) -> Rect {
    let (w, h) = (fit.0 * zoom, fit.1 * zoom);
    let max = range(zoom, fit, area);
    let along = |size: f32, room: f32, scroll: f32, max: f32| {
        (room + max - size) / 2.0 - scroll.clamp(0.0, max)
    };
    Rect::new(
        area.x + along(w, area.w, scroll.0, max.0),
        area.y + along(h, area.h, scroll.1, max.1),
        w,
        h,
    )
}

/// 返回倍数为 `zoom` 时，使图片中相对位置 `at` 处位于 `anchor`（窗口坐标）的滚动位置，
/// 限制在内容的范围内。
fn hold(zoom: f32, at: (f32, f32), anchor: (f32, f32), fit: (f32, f32), area: Rect) -> (f32, f32) {
    // 滚动位置为 0 时图片的位置；滚动 s 使图片向左、向上移动 s。
    let origin = placed(zoom, (0.0, 0.0), fit, area);
    let max = range(zoom, fit, area);
    (
        (origin.x + at.0 * origin.w - anchor.0).clamp(0.0, max.0),
        (origin.y + at.1 * origin.h - anchor.1).clamp(0.0, max.1),
    )
}

impl PictureView {
    /// 返回图片 `picture` 这一帧在预览区域 `area` 中的位置（窗口坐标，逻辑像素）；`area` 容不下
    /// 留白时返回 `None`。缩放过渡期间倍数逐帧变化，过渡的锚点处保持不动。
    pub(crate) fn rect(&self, picture: &Picture, area: Rect) -> Option<Rect> {
        fitted(picture, area).map(|fit| self.place(fit, area))
    }

    /// 返回缩放过渡结束后图片的位置，没有过渡时同 [`Self::rect`]；光栅化按这一位置和大小进行。
    pub(crate) fn settled(&self, picture: &Picture, area: Rect) -> Option<Rect> {
        fitted(picture, area).map(|fit| placed(self.zoom, self.scroll, fit, area))
    }

    /// 同 [`Self::rect`]，`fit` 为图片适应 `area` 时的大小。
    fn place(&self, fit: (f32, f32), area: Rect) -> Rect {
        match self.zoom_motion {
            Some(m) => {
                let zoom = m.clock.between(m.from, self.zoom);
                placed(zoom, hold(zoom, m.at, m.anchor, fit, area), fit, area)
            }
            None => placed(self.zoom, self.scroll, fit, area),
        }
    }

    /// 横向滚动 `dx`、竖直滚动 `dy`（逻辑像素，向右、向下滚动为负），限制在内容的范围内。
    ///
    /// 缩放过渡立即结束：过渡期间的位置由锚点决定，无法同时跟随滚动。
    fn scroll_by(&mut self, (dx, dy): (f32, f32), fit: (f32, f32), area: Rect) {
        self.zoom_motion = None;
        let max = range(self.zoom, fit, area);
        self.scroll = (
            (self.scroll.0.clamp(0.0, max.0) - dx).clamp(0.0, max.0),
            (self.scroll.1.clamp(0.0, max.1) - dy).clamp(0.0, max.1),
        );
    }

    /// 把倍数改为 `zoom`，`anchor`（窗口坐标）处的图片内容保持在原位；内容的范围不允许时
    /// 取最接近的滚动位置。`clock` 不为 `None` 时从这一帧的位置过渡到新的位置。
    fn zoom_to(
        &mut self,
        zoom: f32,
        anchor: (f32, f32),
        (fit, area): ((f32, f32), Rect),
        clock: Option<Clock>,
    ) {
        let before = self.place(fit, area);
        let at = (
            (anchor.0 - before.x) / before.w,
            (anchor.1 - before.y) / before.h,
        );
        let from = before.w / fit.0;
        self.zoom = zoom;
        self.scroll = hold(zoom, at, anchor, fit, area);
        self.zoom_motion = clock.map(|clock| ZoomMotion {
            from,
            clock,
            at,
            anchor,
        });
    }
}

impl App {
    /// 返回当前标签显示的图片及其预览区域：只看预览时为编辑区，SVG 分栏预览时为右侧一栏；
    /// 没有显示图片时返回 `None`。
    pub(crate) fn shown_picture(&self) -> Option<(&Arc<Picture>, Rect)> {
        let doc = self.editor.active()?;
        if let Some(picture) = doc.picture() {
            return Some((picture, self.layout.editor));
        }
        let split = doc.preview_mode() == PreviewMode::Split && self.split.doc == Some(doc.id);
        let picture = self.split.picture.as_ref().filter(|_| split)?;
        Some((picture, self.layout.split))
    }

    /// 返回当前显示的图片适应预览区域时的大小（逻辑像素）和预览区域；没有显示图片或区域容不下
    /// 留白时返回 `None`。
    fn picture_fit(&self) -> Option<((f32, f32), Rect)> {
        let (picture, area) = self.shown_picture()?;
        Some((fitted(picture, area)?, area))
    }

    /// 返回当前显示的图片可以清晰显示的最大长边（逻辑像素）：SVG 为光栅化长边的上限
    /// [`svg::MAX_EDGE`]，位图不限。
    fn picture_edge_limit(&self) -> f32 {
        let vector = self
            .shown_picture()
            .is_some_and(|(picture, _)| matches!(picture.source, Source::Vector(_)));
        if !vector {
            return f32::INFINITY;
        }
        let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
        svg::MAX_EDGE as f32 / scale
    }

    /// 当前标签预览的文档改变时还原缩放与滚动位置，并结束拖动；每次绘制前调用。
    ///
    /// 同一文档的图片重新加载（文件被修改，或分栏预览随文字重绘）时保留缩放与滚动位置。
    pub(super) fn advance_picture_view(&mut self) {
        let doc = self
            .editor
            .active()
            .filter(|d| {
                d.picture.is_some()
                    || (d.preview_mode() == PreviewMode::Split && self.split.doc == Some(d.id))
            })
            .map(|d| d.id);
        if self.picture_view.doc == doc {
            return;
        }
        self.picture_view = PictureView {
            doc,
            ..PictureView::default()
        };
        if matches!(self.drag, Drag::Picture { .. }) {
            self.drag = Drag::None;
            self.update_cursor_icon();
        }
    }

    /// 是否正在显示图片，且 (`x`, `y`)（窗口坐标）位于其预览区域内。
    pub(crate) fn picture_at(&self, x: f32, y: f32) -> bool {
        self.shown_picture()
            .is_some_and(|(_, area)| area.contains(x, y))
    }

    /// 图片是否超出预览区域，可以滚动或拖动。
    pub(in crate::app) fn picture_pannable(&self) -> bool {
        self.picture_fit().is_some_and(|(fit, area)| {
            let (across, down) = range(self.picture_view.zoom, fit, area);
            across > 0.0 || down > 0.0
        })
    }

    /// 把图片横向滚动 `dx`、竖直滚动 `dy`（逻辑像素，向右、向下滚动为负）。
    pub(crate) fn scroll_picture_by(&mut self, dx: f32, dy: f32) {
        if let Some((fit, area)) = self.picture_fit() {
            self.picture_view.scroll_by((dx, dy), fit, area);
            self.request_redraw();
        }
    }

    /// 把图片放大（`up` 为真）、缩小一档，或还原为适应预览区域（`None`），返回当前标签是否
    /// 在预览图片。
    ///
    /// `anchor`（窗口坐标）处的内容保持在原位，为 `None` 时取预览区域的中心。SVG 的显示大小
    /// 超过光栅化的上限后只能放大位图，这样的倍数不可选。
    pub(in crate::app) fn zoom_picture(
        &mut self,
        up: Option<bool>,
        anchor: Option<(f32, f32)>,
    ) -> bool {
        let Some((fit, area)) = self.picture_fit() else {
            return self.picture_view.doc.is_some();
        };
        let old = self.picture_view.zoom;
        let limit = self.picture_edge_limit();
        let fits = |zoom: &f32| *zoom <= 1.0 || fit.0.max(fit.1) * zoom <= limit;
        let next = match up {
            Some(true) => ZOOMS.into_iter().find(|zoom| *zoom > old).filter(fits),
            Some(false) => ZOOMS.into_iter().rev().find(|zoom| *zoom < old),
            None => Some(1.0),
        };
        let zoom = next.unwrap_or(old);
        if zoom != old {
            let anchor = anchor.unwrap_or((area.x + area.w / 2.0, area.y + area.h / 2.0));
            // 有过渡时由每帧的绘制推进；没有时立即到位。
            let clock = self.clock(PDF_MOTION);
            self.picture_view.zoom_to(zoom, anchor, (fit, area), clock);
        }
        let percent = (zoom * 100.0).round().to_string();
        self.flash(t!("preview-picture-zoom", zoom = percent));
        self.update_cursor_icon();
        self.request_redraw();
        true
    }

    /// 处理触控板捏合 `delta`（放大为正）：鼠标在图片预览上时，累计满一档缩放一次。
    pub(crate) fn picture_pinch(&mut self, delta: f32) {
        let (x, y) = self.mouse;
        if !self.picture_at(x, y) {
            return;
        }
        self.picture_view.pinch += delta;
        if self.picture_view.pinch.abs() >= PINCH_STEP {
            let up = self.picture_view.pinch > 0.0;
            self.picture_view.pinch = 0.0;
            self.zoom_picture(Some(up), Some(self.mouse));
        }
    }

    /// 处理在图片预览的 (`x`, `y`)（窗口坐标）按下左键：双击还原为适应预览区域；图片超出预览
    /// 区域时开始拖动。
    pub(in crate::app) fn click_picture(&mut self, x: f32, y: f32) {
        self.focus = Focus::Editor;
        if self.click_count(x, y) == 2 {
            self.zoom_picture(None, None);
        } else if self.picture_pannable() {
            self.drag = Drag::Picture { at: (x, y) };
            self.update_cursor_icon();
        }
    }

    /// 处理拖动图片时鼠标从 `at` 移到 (`x`, `y`)（窗口坐标）：图片随鼠标移动相同的距离，到达
    /// 内容的边缘后不再移动。
    pub(in crate::app) fn drag_picture(&mut self, x: f32, y: f32, at: (f32, f32)) {
        self.scroll_picture_by(x - at.0, y - at.1);
        self.drag = Drag::Picture { at: (x, y) };
    }
}

#[cfg(test)]
mod tests;
