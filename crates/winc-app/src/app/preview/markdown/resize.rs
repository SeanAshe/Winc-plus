//! 可编辑的预览中调整图片大小：拖动图片右侧的把手，或使用右键菜单「图片大小」。改写图片的
//! 原文（见 `winc_preview::markdown::resize`），占一步撤销，光标位置不变。

use winc_preview::markdown::resize::{self, Width};
use winc_render::Rect;
use winc_typeset::Item;

use crate::app::App;

/// 把手的宽度（逻辑像素）。
const HANDLE: f32 = 4.0;
/// 把手高度的下限和上限（逻辑像素），其间取图片高度的 30%。
const GRIP: (f32, f32) = (20.0, 32.0);
/// 把手与图片右边缘的距离（逻辑像素）。
const HANDLE_GAP: f32 = 4.0;
/// 拖动时外框与图片边缘的距离（逻辑像素）。
pub(crate) const OUTLINE_GAP: f32 = 2.0;
/// 鼠标距图片多远之内仍算停在图片上（逻辑像素），须覆盖图片外侧的把手。
const REACH: f32 = 12.0;
/// 显示把手的最小图片宽高（逻辑像素）。
const HANDLE_ROOM: f32 = 32.0;
/// 拖动时图片的最小宽度（逻辑像素）。
const MIN_WIDTH: f32 = 24.0;
/// 右键菜单「图片大小」中的比例（百分数），0 表示去掉写明的大小。
pub(crate) const PRESETS: [u32; 5] = [25, 50, 75, 100, 0];

/// 上一帧绘制的、原文可以改写的图片。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImageAt {
    /// 在窗口中的范围。
    pub(crate) rect: Rect,
    /// 所在的顶层块。
    pub(crate) block: usize,
    /// 在块的各项中的序号。
    pub(crate) item: usize,
}

impl ImageAt {
    /// 返回把手的范围（窗口坐标）；图片太小时为 `None`。
    pub(crate) fn handle(&self) -> Option<Rect> {
        let r = self.rect;
        if r.w < HANDLE_ROOM || r.h < HANDLE_ROOM {
            return None;
        }
        let h = (r.h * 0.3).clamp(GRIP.0, GRIP.1).round();
        let y = (r.y + (r.h - h) / 2.0).round();
        Some(Rect::new((r.right() + HANDLE_GAP).round(), y, HANDLE, h))
    }
}

/// 正在拖动把手的图片。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Resizing {
    /// 图片。
    pub(crate) image: ImageAt,
    /// 按下时鼠标的横坐标。
    from: f32,
    /// 图片是否在正文一栏中居中：居中时两侧同时改变宽度。
    pub(crate) centered: bool,
    /// 当前宽度（逻辑像素）。
    pub(crate) width: f32,
}

impl App {
    /// 返回可编辑的预览中 `(x, y)` 处的图片。
    pub(crate) fn image_at(&self, x: f32, y: f32) -> Option<ImageAt> {
        let area = self.preview.markdown.area;
        (self.markdown_live() && area.contains(x, y))
            .then(|| {
                self.hits
                    .markdown_images
                    .iter()
                    .rev()
                    .find(|i| i.rect.contains(x, y))
            })
            .flatten()
            .copied()
    }

    /// 返回 `(x, y)` 所在或靠近（见 [`REACH`]）的图片。
    pub(crate) fn image_hovered(&self, x: f32, y: f32) -> Option<ImageAt> {
        let area = self.preview.markdown.area;
        let images = &self.hits.markdown_images;
        (self.markdown_live() && area.contains(x, y))
            .then(|| (images.iter().rev()).find(|i| i.rect.inset(-REACH, -REACH).contains(x, y)))
            .flatten()
            .copied()
    }

    /// 返回把手位于 `(x, y)` 处的图片；把手的点击范围向外扩展 5 逻辑像素。
    pub(crate) fn image_handle_at(&self, x: f32, y: f32) -> Option<ImageAt> {
        let image = self.image_hovered(x, y)?;
        let handle = image.handle()?.inset(-5.0, -4.0);
        handle.contains(x, y).then_some(image)
    }

    /// 开始拖动 `image` 的把手。
    pub(crate) fn start_image_resize(&mut self, image: ImageAt, x: f32) {
        let width = image.rect.w;
        let centered = (self.preview.markdown.layout.as_ref()).is_some_and(|l| {
            let column = l.left + l.key.width / 2.0;
            (image.rect.x + image.rect.w / 2.0 - column).abs() < 1.0 && width < l.key.width
        });
        self.preview.markdown.resizing = Some(Resizing {
            image,
            from: x,
            centered,
            width,
        });
        self.pointer.drag = crate::app::Drag::ImageResize;
        self.request_redraw();
    }

    /// 按鼠标横坐标 `x` 更新拖动中的宽度，不超过正文一栏。
    pub(crate) fn drag_image_resize(&mut self, x: f32) {
        let column = self
            .preview
            .markdown
            .layout
            .as_ref()
            .map_or(f32::INFINITY, |l| l.key.width);
        if let Some(r) = self.preview.markdown.resizing.as_mut() {
            let pull = if r.centered { 2.0 } else { 1.0 };
            let width = r.image.rect.w + (x - r.from) * pull;
            r.width = width.clamp(MIN_WIDTH.min(column), column);
            self.request_redraw();
        }
    }

    /// 结束拖动；宽度改变时写回原文。
    pub(crate) fn finish_image_resize(&mut self) {
        if let Some(r) = self.preview.markdown.resizing.take() {
            if (r.width - r.image.rect.w).abs() >= 1.0 {
                self.resize_image(r.image, Some(Width::Px(r.width.round() as u32)));
            }
            self.request_redraw();
        }
    }

    /// 执行右键菜单「图片大小」中的一项；`percent` 为 0 时去掉写明的大小。
    pub(crate) fn resize_image_to(&mut self, percent: u32) {
        if let Some(image) = self.preview.markdown.menu_image.take() {
            let width = (percent > 0).then_some(Width::Percent(percent));
            self.resize_image(image, width);
        }
    }

    /// 把图片 `image` 的宽度改为 `width`（`None` 为去掉写明的大小），返回是否修改了原文。
    fn resize_image(&mut self, image: ImageAt, width: Option<Width>) -> bool {
        let Some((start, end, standalone)) = self.image_source(image) else {
            return false;
        };
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let end = end.min(rope.len_bytes());
        let text = rope.byte_slice(start.min(end)..end).to_string();
        let Some((range, new)) = resize::resize(&text, width, standalone) else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(start + range.start),
            rope.byte_to_char(start + range.end),
        );
        let delta = new.chars().count() as isize - (b - a) as isize;
        // 改写范围之后的位置随长度变化移动，范围之内的移到范围开头。
        let keep = |pos: usize| match pos {
            pos if pos <= a => pos,
            pos if pos >= b => pos.saturating_add_signed(delta),
            _ => a,
        };
        let old = doc.selection;
        let editing = self.preview.markdown.editing == Some(doc.id);
        doc.collapse_selections();
        doc.selection = winc_core::Selection::new(a, b);
        self.preview.markdown.slide = true;
        self.run(winc_editor::Command::Insert(new));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(keep(old.anchor), keep(old.head));
        }
        // 改写不算作在预览中编辑：未在编辑时，光标所在的块不显示原文。
        if !editing
            && let (Some(source), Some(doc)) = (self.live_caret_source(), self.editor.active())
        {
            self.preview.markdown.followed = Some((doc.id, source, doc.revision));
        }
        true
    }

    /// 返回图片原文的开头（字节偏移）、所在顶层块的结尾，以及图片是否位于顶层块开头；排版
    /// 落后于文字时为 `None`。
    fn image_source(&self, image: ImageAt) -> Option<(usize, usize, bool)> {
        let md = &self.preview.markdown;
        let revision = self.editor.active()?.revision;
        let laid = md
            .layout
            .as_ref()
            .filter(|l| md.parsed.is_some() && l.source == md.parse_serial)?;
        if md.checked != Some(revision) {
            return None;
        }
        let item = laid.blocks.get(image.block)?.1.items.get(image.item)?;
        let Item::Image {
            at: Some((_, at)), ..
        } = item
        else {
            return None;
        };
        let range = laid.ranges.get(image.block)?;
        Some((range.start + *at as usize, range.end, *at == 0))
    }
}
