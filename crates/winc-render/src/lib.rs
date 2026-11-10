//! 渲染层：用 swash 光栅化字形，由 CPU 合成整帧并写入窗口的呈现缓冲。
//!
//! [`Renderer`] 负责字体加载、字形缓存和整帧的呈现；绘制通过 [`Frame`] 完成，所有坐标都是
//! 逻辑像素，呈现时按 [`Renderer::scale_factor`] 换算成物理像素。颜色是未预乘的 sRGB 分量，
//! 打包时使用 `0xAARRGGBB` 布局。macOS 用 `IOSurface` 呈现（`present/macos.rs`），
//! 其他平台用软件缓冲（`present/soft.rs`）。

mod cache;
mod draw;
mod present;
mod text;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Angle, Format, Transform, Vector};
use winit::window::Window;

pub use draw::canvas::{Covered, Stroke, rounded_outline};
pub use draw::frame::Frame;
pub use text::font::{FontKind, set_cjk_language};
pub use text::font_list::{FontFamily, installed_fonts};
pub use text::font_path::{DEFAULT_EDITOR_FONT, FontError, FontSpec};
pub use text::outline::{OutlineFonts, OutlineGlyph, OutlineMetrics};

use cache::{FastMap, Generations, StrokeCache};
use draw::canvas::{Canvas, IRect};
use present::Presenter;
use text::font::Fonts;
use text::glyphs::*;

/// 颜色，非预乘的 sRGB 分量，范围 0–1。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// 红色分量，0–1。
    pub r: f32,
    /// 绿色分量，0–1。
    pub g: f32,
    /// 蓝色分量，0–1。
    pub b: f32,
    /// 不透明度，0（全透明）–1（不透明）。
    pub a: f32,
}

impl Color {
    /// 用 `0xRRGGBB` 形式的颜色值创建不透明颜色。
    pub const fn hex(rgb: u32) -> Color {
        Color {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// 返回颜色分量不变、不透明度为 `a` 的颜色。
    pub const fn alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    /// 返回 `0xAARRGGBB` 打包值；各分量先夹取到 0–1，再四舍五入到 0–255。
    fn argb(self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        (c(self.a) << 24) | (c(self.r) << 16) | (c(self.g) << 8) | c(self.b)
    }
}

/// 逻辑像素下的矩形，原点在左上角，`y` 轴向下。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// 左上角横坐标（逻辑像素）。
    pub x: f32,
    /// 左上角纵坐标（逻辑像素）。
    pub y: f32,
    /// 宽度（逻辑像素）。
    pub w: f32,
    /// 高度（逻辑像素）。
    pub h: f32,
}

impl Rect {
    /// 用左上角坐标和尺寸创建矩形。
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// 右边界的横坐标。
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// 下边界的纵坐标。
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// 是否包含点 `(x, y)`：左闭右开，右边界和下边界不计入。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// 返回两个矩形重叠的部分；不重叠时宽或高为 0。
    pub fn intersect(&self, other: Rect) -> Rect {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (right, bottom) = (
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }

    /// 返回左右各向内收缩 `dx`、上下各向内收缩 `dy` 的矩形；传入负数则向外扩展。
    ///
    /// 结果的宽高不会小于 0。
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.x + dx,
            self.y + dy,
            (self.w - 2.0 * dx).max(0.0),
            (self.h - 2.0 * dy).max(0.0),
        )
    }

    /// 换算为物理像素矩形：四条边分别四舍五入到整数像素，因此相邻的矩形之间不留缝。
    fn to_pixels(self, scale: f32) -> IRect {
        let x0 = (self.x * scale).round() as i32;
        let y0 = (self.y * scale).round() as i32;
        let x1 = (self.right() * scale).round() as i32;
        let y1 = (self.bottom() * scale).round() as i32;
        IRect { x0, y0, x1, y1 }
    }

    /// 换算为物理像素矩形，向外取整：左、上边向下取整，右、下边向上取整。
    ///
    /// 用于裁剪框，避免边缘的一行像素被裁掉。
    fn to_pixels_outer(self, scale: f32) -> IRect {
        let x0 = (self.x * scale).floor() as i32;
        let y0 = (self.y * scale).floor() as i32;
        let x1 = (self.right() * scale).ceil() as i32;
        let y1 = (self.bottom() * scale).ceil() as i32;
        IRect { x0, y0, x1, y1 }
    }
}

/// 画面上一块区域的像素副本（物理像素），见 [`Frame::snapshot`]。
///
/// 整个窗口的副本在 2× 屏上超过 10 MB，因此不实现 `Clone`，避免被无意复制。
pub struct Snapshot {
    area: draw::canvas::IRect,
    px: Vec<u32>,
    /// 保存时的缩放系数：切换显示器后快照不再适用。
    scale: f32,
}

impl Default for Snapshot {
    /// 创建空的快照，用作 [`Frame::snapshot_into`] 的缓冲。
    fn default() -> Self {
        Snapshot {
            area: draw::canvas::IRect::new(0, 0, 0, 0),
            px: Vec::new(),
            scale: 0.0,
        }
    }
}

impl Snapshot {
    /// 是否保存的范围恰为 `rect`（逻辑像素）在缩放系数 `scale` 下的位置。
    pub fn covers(&self, rect: Rect, scale: f32) -> bool {
        self.scale == scale && self.area == rect.to_pixels(scale)
    }
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot")
            .field("area", &self.area)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

/// 字体度量，逻辑像素。
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// 基线以上的高度（逻辑像素，正数）。
    pub ascent: f32,
    /// 基线以下的高度（逻辑像素，正数）。
    pub descent: f32,
    /// 等宽字体一格的宽度（逻辑像素），取字符 `0` 的前进宽度。
    pub cell: f32,
    /// 小写字母 `x` 的高度（逻辑像素）。
    pub x_height: f32,
    /// 大写字母的高度（逻辑像素）。
    pub cap_height: f32,
}

/// 字形缓存一代的软上限（字节，2 MiB）。
///
/// 缓存分新旧两代（见 `Generations`），通常合计最多约 4 MiB；超过上限时在下一帧开始时换代，
/// 旧一代中再次用到的字形移回新一代，无需重新光栅化。一帧所需的字形多于一代时（中文密集的
/// 一屏，每个字最多 4 种亚像素位置），该帧用到的字形仍全部保留，最多达到上限的 4 倍。
const GLYPH_CACHE_BYTES: usize = 2 << 20;
/// 前进宽度缓存的最大项数（项，65536）。
///
/// 每种字号的每个字符各占一项，浏览大量中文或反复切换字号时会持续增长，因此达到上限时整体清空；
/// 重新测量单个字符只需查一次字体度量表。
const ADVANCE_CACHE_ENTRIES: usize = 1 << 16;

/// 恒等覆盖率映射表：描边蒙版按计算出的覆盖率混合，不按背景调整浓淡。
const LINEAR_COVERAGE: [u8; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = i as u8;
        i += 1;
    }
    table
};

/// 字体没有可用的斜体时，把正体斜切的角度（度）。
///
/// 常见等宽字体的斜体约倾斜 9° 至 12°。
const OBLIQUE_DEGREES: f32 = 10.0;

/// 模拟粗体时笔画加宽的量与字号之比。
///
/// 取 `FreeType` 粗体模拟所用的比例（1/24）。笔画两侧各加宽一半，前进宽度也加上同样的量，
/// 避免相邻字之间过于拥挤。
const BOLD_RATIO: f32 = 1.0 / 24.0;

/// 字体本身不提供、在绘制时模拟的字形变化：粗体加宽笔画，斜体把正体斜切。
///
/// Markdown 预览中的加粗、强调使用该模拟，因此无需另外加载粗体、斜体字体文件，后备字体（例如
/// 中文字体）同样可以加粗、倾斜。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Faux {
    /// 加粗。
    pub bold: bool,
    /// 倾斜。
    pub italic: bool,
}

/// 渲染器：持有字体、字形缓存和呈现目标，负责整帧的绘制与呈现。
pub struct Renderer {
    /// 呈现目标：窗口或内存图像。
    target: Target,
    /// 字体、字形缓存和前进宽度缓存；同一进程中的多个窗口可以共用同一缓存（[`SharedGlyphs`]）。
    pub(crate) glyphs: Rc<RefCell<GlyphCache>>,
    /// 图标描边的覆盖率蒙版缓存。
    strokes: StrokeCache,
    /// 覆盖率映射表：深色背景上的浅色文字略微加粗，避免显得单薄；随背景色调整。
    gamma: [u8; 256],
    /// `gamma` 是否为深色背景的映射表（见 [`is_dark`]）。
    dark: bool,
    /// 逻辑像素到物理像素的倍数：系统的缩放系数乘以 [`Self::zoom`]；调用 [`Self::resize`] 和
    /// [`Self::set_zoom`] 时更新。
    scale: f32,
    /// 界面缩放倍数，1 为不缩放（见 [`Self::set_zoom`]）。
    zoom: f32,
    /// 当前背景色，打包成 `0xAARRGGBB`。
    background: u32,
    /// 下一帧必然被不透明内容完全覆盖的区域（逻辑像素），填充背景色时跳过，见
    /// [`Self::cover_next_frame`]。
    cover: Option<Rect>,
    /// 下一帧的底色矩形（物理像素），见 [`Self::base_next_frame`]。
    base: Vec<(IRect, u32)>,
    /// `Canvas::fill_layered` 复用的行缓冲。
    row: Vec<u32>,
}

/// 可供多个渲染器共用的字体和字形缓存：同一进程中的多个窗口使用相同的字体，已光栅化的字形无需
/// 每个窗口各存一份（键中包含物理像素字号，不同缩放的屏幕上的窗口也能共用）。
///
/// 只在界面线程上使用。
#[derive(Clone)]
pub struct SharedGlyphs(Rc<RefCell<GlyphCache>>);

/// 绘制目标：窗口，或内存中的一张图像（用于测试截图，无需创建窗口）。
enum Target {
    Window(Arc<Window>, Presenter),
    Image(Image),
}

/// 离屏渲染的一帧，每个像素是 `0xAARRGGBB`。
pub struct Image {
    /// 像素数据，按行优先排列。
    pub pixels: Vec<u32>,
    /// 宽度（物理像素）。
    pub width: u32,
    /// 高度（物理像素）。
    pub height: u32,
}

/// 背景是否为深色：按 Rec. 709 的系数加权计算的亮度低于 0.5。
///
/// 深色与浅色背景各用一张覆盖率映射表（[`gamma_for`]）。
fn is_dark(background: Color) -> bool {
    0.2126 * background.r + 0.7152 * background.g + 0.0722 * background.b < 0.5
}

/// 按背景亮度生成覆盖率映射表。
///
/// 深色背景（[`is_dark`]）取指数 1.25，其余取 1.1；指数越大，半透明的覆盖率提升越多，
/// 文字越粗。两个指数取自调参。
fn gamma_for(background: Color) -> [u8; 256] {
    let exponent = if is_dark(background) { 1.25 } else { 1.1 };

    std::array::from_fn(|i| {
        let coverage = i as f32 / 255.0;
        ((1.0 - (1.0 - coverage).powf(exponent)) * 255.0 + 0.5) as u8
    })
}

impl Renderer {
    /// 创建渲染器，呈现到 `window`。
    ///
    /// `mono` 和 `ui` 是用户配置的字体，为 `None` 时使用平台默认字体；`terminal` 是终端字体，为
    /// `None` 时终端使用 `mono`。
    ///
    /// # Errors
    ///
    /// 字体文件不存在、无法读取或格式不支持时返回错误描述；
    /// 创建平台呈现缓冲失败时同样返回错误描述。
    pub fn new(
        window: Arc<Window>,
        background: Color,
        mono: Option<&FontSpec>,
        ui: Option<&FontSpec>,
        terminal: Option<&FontSpec>,
    ) -> Result<Renderer, String> {
        let fonts = Fonts::load(mono, ui, terminal)?;
        Self::with_glyphs(window, background, SharedGlyphs(new_glyphs(fonts)))
    }

    /// 与 [`Renderer::new`] 相同，但使用 `glyphs`（另一个窗口渲染器的字体和字形缓存，见
    /// [`Renderer::shared_glyphs`]），不再重复加载字体。
    ///
    /// # Errors
    ///
    /// 创建平台呈现缓冲失败时返回错误描述。
    pub fn with_glyphs(
        window: Arc<Window>,
        background: Color,
        glyphs: SharedGlyphs,
    ) -> Result<Renderer, String> {
        let presenter = Presenter::new(&window)?;
        let scale = window.scale_factor() as f32;

        Ok(Renderer {
            target: Target::Window(window, presenter),
            glyphs: glyphs.0,
            strokes: StrokeCache::new(),
            gamma: gamma_for(background),
            dark: is_dark(background),
            scale,
            zoom: 1.0,
            background: background.argb(),
            cover: None,
            base: Vec::new(),
            row: Vec::new(),
        })
    }

    /// 创建离屏渲染器：绘制到内存而非窗口，逻辑尺寸 `width`×`height`，按 `scale` 倍缩放。
    ///
    /// 终端使用 `mono`；需要单独的终端字体时随后调用 [`Self::set_fonts`]。
    ///
    /// # Errors
    ///
    /// 与 [`Renderer::new`] 相同：字体加载失败时返回错误描述。
    pub fn offscreen(
        width: f32,
        height: f32,
        scale: f32,
        background: Color,
        mono: Option<&FontSpec>,
        ui: Option<&FontSpec>,
    ) -> Result<Renderer, String> {
        let (w, h) = ((width * scale) as u32, (height * scale) as u32);

        Ok(Renderer {
            target: Target::Image(Image {
                pixels: vec![0; (w * h) as usize],
                width: w,
                height: h,
            }),
            glyphs: new_glyphs(Fonts::load(mono, ui, None)?),
            strokes: StrokeCache::new(),
            gamma: gamma_for(background),
            dark: is_dark(background),
            scale,
            zoom: 1.0,
            background: background.argb(),
            cover: None,
            base: Vec::new(),
            row: Vec::new(),
        })
    }

    /// 返回本渲染器的字体和字形缓存，供之后新建的窗口共用（[`Renderer::with_glyphs`]）。
    pub fn shared_glyphs(&self) -> SharedGlyphs {
        SharedGlyphs(Rc::clone(&self.glyphs))
    }

    /// 改用 `glyphs` 中的字体和字形缓存（用于在离屏渲染的测量中模拟同一进程的第二个窗口）。
    pub fn use_glyphs(&mut self, glyphs: SharedGlyphs) {
        self.glyphs = glyphs.0;
    }

    /// 离屏渲染时返回最近绘制完成的一帧；呈现到窗口时返回 `None`。
    pub fn image(&self) -> Option<&Image> {
        match &self.target {
            Target::Window(..) => None,
            Target::Image(image) => Some(image),
        }
    }

    /// 设置背景色并重新计算覆盖率映射表，例如切换深浅主题时。
    pub fn set_background(&mut self, background: Color) {
        self.background = background.argb();
        self.gamma = gamma_for(background);
        self.dark = is_dark(background);
    }

    /// 更换字体；用户在设置界面修改字体名或字体文件后调用，失败时保留原字体。
    ///
    /// 重新加载字体会重新测量度量，因此整个字形缓存清空：缓存中的字形都带有旧字体的槽位和字形
    /// id，不清空会绘制出错误的字符。共用该缓存的其他窗口同时更换字体，随后收到相同的配置。
    ///
    /// # Errors
    ///
    /// 字体文件不存在、无法读取或格式不支持时返回错误描述，此时渲染器保持原字体不变。
    pub fn set_fonts(
        &mut self,
        mono: Option<&FontSpec>,
        ui: Option<&FontSpec>,
        terminal: Option<&FontSpec>,
    ) -> Result<(), String> {
        let fonts = Fonts::load(mono, ui, terminal)?;
        let mut glyphs = self.glyphs.borrow_mut();
        glyphs.fonts = fonts;
        glyphs.clear();
        glyphs.advances.clear();
        Ok(())
    }
    /// 返回逻辑像素到物理像素的倍数：系统的缩放系数乘以界面缩放倍数。
    pub fn scale_factor(&self) -> f32 {
        self.scale
    }

    /// 设置界面缩放倍数 `zoom`（1 为不缩放）。
    ///
    /// 窗口的物理尺寸不变，逻辑尺寸随之变小或变大，界面、编辑区和终端同时按该倍数放大或
    /// 缩小；字形按新的物理像素字号光栅化，缩放后仍然清晰。
    pub fn set_zoom(&mut self, zoom: f32) {
        self.scale = self.scale / self.zoom * zoom;
        self.zoom = zoom;
    }

    /// 窗口尺寸或系统的缩放系数变化时调用。
    ///
    /// 缩放变化时不清空字形缓存和描边缓存：两者的键都按物理像素计算（字号、描边坐标），
    /// 不同缩放的结果互不冲突；字形缓存由各窗口共用，按旧缩放光栅化的条目不再使用后按代淘汰。
    /// 离屏渲染时不做处理。
    pub fn resize(&mut self) {
        let Target::Window(window, presenter) = &mut self.target else {
            return;
        };
        let size = window.inner_size();
        let scale = window.scale_factor();

        self.scale = scale as f32 * self.zoom;
        // 呈现缓冲的像素密度取决于屏幕，与界面缩放无关。
        presenter.resize(size.width, size.height, scale);
    }

    /// 返回应处理空闲呈现缓冲的时刻：macOS 上标记为可清除，其他平台上释放（Wayland 除外）；
    /// 均已处理或离屏渲染时为 `None`。
    ///
    /// 停止绘制后可能不再绘制新的一帧，因此调用方按该时刻安排唤醒，到时调用
    /// [`Self::make_idle_purgeable`]。
    pub fn purgeable_at(&self) -> Option<std::time::Instant> {
        match &self.target {
            Target::Window(_, presenter) => presenter.purgeable_at(),
            _ => None,
        }
    }

    /// 处理空闲的呈现缓冲：macOS 上把当前显示的缓冲以外的缓冲标记为可清除（不计入进程内存，系统
    /// 可以回收），其他平台上释放窗口缓冲，下一帧重建；距上次绘制未到规定时长时不做处理。
    pub fn make_idle_purgeable(&mut self) {
        if let Target::Window(_, presenter) = &mut self.target {
            presenter.make_idle_purgeable();
        }
    }

    /// 设置窗口是否在前台（有焦点且没有被完全遮挡）。
    ///
    /// 转入后台时立即把当前显示的缓冲以外的呈现缓冲标记为可清除。
    pub fn set_foreground(&mut self, foreground: bool) {
        if let Target::Window(_, presenter) = &mut self.target {
            presenter.set_foreground(foreground);
        }
    }

    /// 设置窗口是否完全不可见（被遮挡、最小化、位于其他桌面）：不可见时当前显示的呈现缓冲也
    /// 标记为可清除，重新可见时恢复为不可清除。
    ///
    /// 调用方在不可见期间不绘制，重新可见后立即重绘一帧（内容可能已被系统回收）。
    /// 离屏渲染时不做处理。
    pub fn set_occluded(&mut self, occluded: bool) {
        if let Target::Window(_, presenter) = &mut self.target {
            presenter.set_occluded(occluded);
        }
    }

    /// 返回窗口（离屏渲染时为图像）的尺寸（逻辑像素）。
    pub fn logical_size(&self) -> (f32, f32) {
        let (w, h) = match &self.target {
            Target::Window(_, presenter) => presenter.size(),
            Target::Image(image) => (image.width, image.height),
        };
        (w as f32 / self.scale, h as f32 / self.scale)
    }

    /// 返回一段文字按字体 `kind`、字号 `size` 排版后的宽度（逻辑像素），与绘制时
    /// [`Frame::measure`] 的结果相同；布局在绘制之前据此折行。
    pub fn measure(&self, kind: FontKind, size: f32, text: &str) -> f32 {
        let mut glyphs = self.glyphs.borrow_mut();
        text.chars().map(|c| glyphs.advance(kind, size, c)).sum()
    }

    /// 返回字体 `kind` 在字号 `size`（逻辑像素）下的度量。
    ///
    /// 等宽字体一格的宽度取字符 `0` 的前进宽度。
    ///
    /// # Panics
    ///
    /// 主字体在创建渲染器时必定加载成功，正常调用不会 panic。
    pub fn metrics(&self, kind: FontKind, size: f32) -> Metrics {
        self.glyphs.borrow().metrics(kind, size)
    }

    /// 声明下一帧（下一次 `Self::draw`）的 `paint` 必然以不透明的颜色完全覆盖 `rect`。
    ///
    /// 填充背景色时跳过该区域（四边各留一个像素用于抗锯齿），
    /// 避免每帧先填充编辑区等大块区域再将其覆盖。
    pub fn cover_next_frame(&mut self, rect: Rect) {
        self.cover = Some(rect);
    }

    /// 声明下一帧将由 [`Frame::base`] 绘制的底色矩形 `rects`，使其与背景色在同一趟中绘制
    /// （见 `Canvas::fill_layered`）。
    ///
    /// 存在非不透明的颜色时不声明，由 [`Frame::base`] 逐个混合。声明后该帧忽略
    /// [`Self::cover_next_frame`]。
    pub fn base_next_frame(&mut self, rects: &[(Rect, Color)]) {
        self.base.clear();
        if rects.iter().all(|(_, color)| color.a >= 1.0) {
            let scale = self.scale;
            (self.base).extend(rects.iter().map(|&(r, c)| (r.to_pixels(scale), c.argb())));
        }
    }

    /// 绘制一帧：先填充背景色（[`Self::cover_next_frame`] 声明的区域除外）和
    /// [`Self::base_next_frame`] 声明的底色矩形，再调用 `paint`，最后呈现（离屏渲染时只写入内存
    /// 图像）。
    ///
    /// 呈现缓冲都被合成器占用、无法取得空闲缓冲时不绘制（不调用 `paint`），返回 `false`：调用方
    /// 在下一次屏幕刷新时重试。绘制完成时返回 `true`。
    pub fn draw(&mut self, paint: impl FnOnce(&mut Frame<'_>)) -> bool {
        let (w, h) = self.logical_size();
        let bounds = Rect::new(0.0, 0.0, w, h);
        let mut glyphs = self.glyphs.borrow_mut();
        glyphs.new_frame();
        let glyphs = &mut *glyphs;
        let (strokes, gamma, dark) = (&mut self.strokes, &self.gamma, self.dark);
        let (scale, background) = (self.scale, self.background);
        let cover = self.cover.take().map(|rect| {
            let mut area = rect.to_pixels(scale);
            (area.x0, area.y0, area.x1, area.y1) =
                (area.x0 + 1, area.y0 + 1, area.x1 - 1, area.y1 - 1);
            Covered::new(area, 0.0)
        });
        let (based, row) = (&self.base, &mut self.row);
        let paint = |mut canvas: Canvas<'_>| {
            if based.is_empty() {
                canvas.fill_except(background, cover);
            } else {
                canvas.fill_layered(background, based, row);
            }
            paint(&mut Frame {
                canvas,
                glyphs,
                strokes,
                gamma,
                dark,
                scale,
                bounds,
                limit: bounds,
                opacity: 1.0,
                background,
                based,
            });
        };

        let drawn = match &mut self.target {
            Target::Window(window, presenter) => presenter.frame(|canvas| {
                paint(canvas);
                window.pre_present_notify();
            }),
            Target::Image(image) => {
                paint(Canvas {
                    px: &mut image.pixels,
                    width: image.width as usize,
                    height: image.height as usize,
                    stride: image.width as usize,
                });
                true
            }
        };
        self.base.clear();
        drawn
    }

    /// 在透明底上离屏绘制一张 `width`×`height`（逻辑像素）的小图，按当前缩放系数绘制，
    /// 用于生成系统菜单图标。
    ///
    /// 画布不保存透明度，因此 `paint` 会执行两次（一次黑底、一次白底），再逐像素还原透明度。
    /// 返回的像素已预乘透明度。
    pub fn draw_transparent(
        &mut self,
        width: f32,
        height: f32,
        paint: impl Fn(&mut Frame<'_>),
    ) -> Image {
        let (w, h) = (
            (width * self.scale).round() as usize,
            (height * self.scale).round() as usize,
        );
        let mut glyphs = self.glyphs.borrow_mut();
        let mut pass = |background: u32| {
            let mut px = vec![0; w * h];
            let mut canvas = Canvas {
                px: &mut px,
                width: w,
                height: h,
                stride: w,
            };
            canvas.fill(background);
            paint(&mut Frame {
                canvas,
                glyphs: &mut glyphs,
                strokes: &mut self.strokes,
                gamma: &self.gamma,
                dark: self.dark,
                scale: self.scale,
                bounds: Rect::new(0.0, 0.0, width, height),
                limit: Rect::new(0.0, 0.0, width, height),
                opacity: 1.0,
                background,
                based: &[],
            });
            px
        };
        let (black, white) = (pass(0xff00_0000), pass(0xffff_ffff));

        Image {
            pixels: black
                .iter()
                .zip(&white)
                .map(|(&b, &w)| draw::canvas::unblend(b, w))
                .collect(),
            width: w as u32,
            height: h as u32,
        }
    }
}

/// 创建字体和字形缓存。
fn new_glyphs(fonts: Fonts) -> Rc<RefCell<GlyphCache>> {
    Rc::new(RefCell::new(GlyphCache::new(fonts)))
}

#[cfg(test)]
mod tests;
