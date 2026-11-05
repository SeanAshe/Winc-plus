//! 渲染层：swash 光栅化字形，CPU 合成整帧，写进窗口的呈现缓冲。
//!
//! [`Renderer`] 负责字体加载、字形缓存和整帧的呈现；绘制通过 [`Frame`] 完成，所有坐标都是逻辑像素，
//! 呈现时按 [`Renderer::scale_factor`] 换算成物理像素。颜色是未预乘的 sRGB 分量，
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

pub use draw::canvas::{Covered, Stroke};
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

/// 画面上一块的像素拷贝（物理像素），见 [`Frame::snapshot`]。
///
/// 整个窗口的拷贝在 2× 屏上超过 10 MB，因此不实现 `Clone`，避免被无意复制。
pub struct Snapshot {
    area: draw::canvas::IRect,
    px: Vec<u32>,
    /// 存下来时的缩放系数：换了显示器以后不再适用。
    scale: f32,
}

impl Default for Snapshot {
    /// 空的快照，用来给 [`Frame::snapshot_into`] 当缓冲。
    fn default() -> Self {
        Snapshot {
            area: draw::canvas::IRect::new(0, 0, 0, 0),
            px: Vec::new(),
            scale: 0.0,
        }
    }
}

impl Snapshot {
    /// 存下来的范围是否就是 `rect`（逻辑像素）在缩放系数 `scale` 下的位置。
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
/// 缓存分新旧两代（见 `Generations`），平时合起来最多约 4 MiB；超过时在下一帧开头换代，旧一代
/// 里又用到的字形搬回来，不必重新光栅化。一帧要的字形多于一代时（中文密集的一屏，每个字最多
/// 4 种子像素位置），这一帧用到的字形仍然全部保留，最多到上限的 4 倍。该值是经验值。
const GLYPH_CACHE_BYTES: usize = 2 << 20;
/// 前进宽度缓存的最大项数（项，65536）。
///
/// 每种字号的每个字符各占一项，浏览大量中文或反复切换字号时会持续增长，因此满了就整体清空；
/// 重新测量单个字符只需查一次字体度量表。
const ADVANCE_CACHE_ENTRIES: usize = 1 << 16;

/// 原样不变的覆盖率映射表：描边蒙版按算出的覆盖率混合，不按背景调整浓淡。
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

/// 字体本身没有、绘制时模拟出来的字形变化：粗体加宽笔画，斜体把正体斜切。
///
/// Markdown 预览里的加粗、强调用它，因此不用另外加载粗体、斜体字体文件，后备字体（例如中文）
/// 也一样能加粗、倾斜。
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
    /// 字体、字形缓存和前进宽度缓存；同一个进程里的几个窗口可以共用同一个缓存（[`SharedGlyphs`]）。
    pub(crate) glyphs: Rc<RefCell<GlyphCache>>,
    /// 图标描边的覆盖率蒙版缓存。
    strokes: StrokeCache,
    /// 覆盖率映射表：深色背景上的浅色文字略微加粗，避免显得单薄；随背景色调整。
    gamma: [u8; 256],
    /// 屏幕缩放系数，逻辑像素到物理像素的倍数；[`Self::resize`] 时更新。
    scale: f32,
    /// 当前背景色，打包成 `0xAARRGGBB`。
    background: u32,
    /// 下一帧画的内容一定会不透明地整个盖住的一块（逻辑像素），铺背景色时跳过，见
    /// [`Self::cover_next_frame`]。
    cover: Option<Rect>,
}

/// 可以给几个渲染器共用的字体和字形缓存：同一个进程里的几个窗口用同样的字体，光栅化好的字形
/// 不必每个窗口各存一遍（键里带着物理像素字号，不同缩放的屏幕上的窗口也能共用）。
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

/// 按背景亮度生成覆盖率映射表。
///
/// 亮度按 Rec. 709 的系数加权计算。亮度低于 0.5 的深色背景取指数 1.25，其余取 1.1；指数越大，
/// 半透明的覆盖率被抬得越高，文字越粗。这两个指数都是经验值。
fn gamma_for(background: Color) -> [u8; 256] {
    let luminance = 0.2126 * background.r + 0.7152 * background.g + 0.0722 * background.b;
    let exponent = if luminance < 0.5 { 1.25 } else { 1.1 };

    std::array::from_fn(|i| {
        let coverage = i as f32 / 255.0;
        ((1.0 - (1.0 - coverage).powf(exponent)) * 255.0 + 0.5) as u8
    })
}

impl Renderer {
    /// 创建渲染器，呈现到 `window`。
    ///
    /// `mono` 和 `ui` 是用户配置的字体，为 `None` 时使用平台默认字体。
    ///
    /// # Errors
    ///
    /// 字体文件不存在、无法读取或格式不支持时返回错误描述；创建平台呈现缓冲失败时同样返回
    /// 错误描述。
    pub fn new(
        window: Arc<Window>,
        background: Color,
        mono: Option<&FontSpec>,
        ui: Option<&FontSpec>,
    ) -> Result<Renderer, String> {
        let fonts = Fonts::load(mono, ui)?;
        Self::with_glyphs(window, background, SharedGlyphs(new_glyphs(fonts)))
    }

    /// 同 [`Renderer::new`]，字体和字形缓存用 `glyphs`（另一个窗口的渲染器的，见
    /// [`Renderer::shared_glyphs`]），不再加载一遍字体。
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
            scale,
            background: background.argb(),
            cover: None,
        })
    }

    /// 创建离屏渲染器：绘制到内存而非窗口，逻辑尺寸 `width`×`height`，按 `scale` 倍缩放。
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
            glyphs: new_glyphs(Fonts::load(mono, ui)?),
            strokes: StrokeCache::new(),
            gamma: gamma_for(background),
            scale,
            background: background.argb(),
            cover: None,
        })
    }

    /// 本渲染器的字体和字形缓存，供之后新建的窗口共用（[`Renderer::with_glyphs`]）。
    pub fn shared_glyphs(&self) -> SharedGlyphs {
        SharedGlyphs(Rc::clone(&self.glyphs))
    }

    /// 改用 `glyphs` 这份字体和字形缓存（离屏渲染的测量里模拟同一个进程的第二个窗口）。
    pub fn use_glyphs(&mut self, glyphs: SharedGlyphs) {
        self.glyphs = glyphs.0;
    }

    /// 离屏渲染时返回最近画好的一帧；呈现到窗口时返回 `None`。
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
    }

    /// 更换字体；用户在设置界面修改字体名或字体文件后调用，失败时保留原字体。
    ///
    /// 重新加载字体会重新测量度量，因此整个字形缓存清空：缓存中的字形都带着旧字体的槽位和字形
    /// id，不清空会画出错误的字符。共用这份缓存的其他窗口也一起换，它们随后收到同样的配置。
    ///
    /// # Errors
    ///
    /// 字体文件不存在、无法读取或格式不支持时返回错误描述，此时渲染器保持原字体不变。
    pub fn set_fonts(
        &mut self,
        mono: Option<&FontSpec>,
        ui: Option<&FontSpec>,
    ) -> Result<(), String> {
        let fonts = Fonts::load(mono, ui)?;
        let mut glyphs = self.glyphs.borrow_mut();
        glyphs.fonts = fonts;
        glyphs.clear();
        glyphs.advances.clear();
        Ok(())
    }
    /// 返回屏幕缩放系数：逻辑像素乘以该值得到物理像素。
    pub fn scale_factor(&self) -> f32 {
        self.scale
    }

    /// 窗口尺寸或缩放系数变化时调用。
    ///
    /// 缩放变化时不清空字形缓存和描边缓存：两者的键都按物理像素计算（字号、描边坐标），
    /// 不同缩放的结果互不冲突；字形缓存由各窗口共用，按旧缩放光栅化的条目不再使用后按代淘汰。
    /// 离屏渲染时不做任何事。
    pub fn resize(&mut self) {
        let Target::Window(window, presenter) = &mut self.target else {
            return;
        };
        let size = window.inner_size();
        let scale = window.scale_factor();

        self.scale = scale as f32;
        presenter.resize(size.width, size.height, scale);
    }

    /// 返回应当把空闲的呈现缓冲标成可清除的时刻；都已标好或离屏渲染时为 `None`。
    ///
    /// 停止绘制后可能不再画新的一帧，因此调用方按该时刻安排唤醒，到时调用
    /// [`Self::make_idle_purgeable`]。
    pub fn purgeable_at(&self) -> Option<std::time::Instant> {
        match &self.target {
            Target::Window(_, presenter) => presenter.purgeable_at(),
            _ => None,
        }
    }

    /// 把屏幕上显示着的那块以外的呈现缓冲标成可清除（不计入进程内存，系统可以收回）；
    /// 刚画过、尚未到时间时什么也不做。
    pub fn make_idle_purgeable(&mut self) {
        if let Target::Window(_, presenter) = &mut self.target {
            presenter.make_idle_purgeable();
        }
    }

    /// 设置窗口是否在前台（有焦点且没有被完全遮住）。
    ///
    /// 转入后台时马上把屏幕上显示着的那块以外的呈现缓冲标成可清除。
    pub fn set_foreground(&mut self, foreground: bool) {
        if let Target::Window(_, presenter) = &mut self.target {
            presenter.set_foreground(foreground);
        }
    }

    /// 设置窗口是否完全看不见（被挡住、最小化、在别的桌面上）：看不见时屏幕上显示着的那块
    /// 呈现缓冲也标成可清除，重新可见时标回不可清除。
    ///
    /// 调用方在看不见期间不绘制，重新可见后马上重画一帧（内容可能已被系统收回）。离屏渲染时
    /// 不做任何事。
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

    /// 声明下一帧（下一次 `Self::draw`）的 `paint` 一定会用不透明的颜色整个盖住 `rect`；
    /// 铺背景色时跳过这一块（四边各留一个像素给抗锯齿），避免每帧先把编辑区这样的大块铺一遍，
    /// 再盖掉。
    pub fn cover_next_frame(&mut self, rect: Rect) {
        self.cover = Some(rect);
    }

    /// 绘制一帧：先铺满背景色（[`Self::cover_next_frame`] 说明的那一块除外），再调用 `paint`，
    /// 最后呈现（离屏渲染时只写入内存图像）。
    ///
    /// 呈现缓冲都被合成器占着、等不到空闲的一块时不画（不调用 `paint`），返回 `false`：调用方
    /// 在下一次屏幕刷新时再画。画了返回 `true`。
    pub fn draw(&mut self, paint: impl FnOnce(&mut Frame<'_>)) -> bool {
        let (w, h) = self.logical_size();
        let bounds = Rect::new(0.0, 0.0, w, h);
        let mut glyphs = self.glyphs.borrow_mut();
        glyphs.new_frame();
        let glyphs = &mut *glyphs;
        let (strokes, gamma) = (&mut self.strokes, &self.gamma);
        let (scale, background) = (self.scale, self.background);
        let cover = self.cover.take().map(|rect| {
            let mut area = rect.to_pixels(scale);
            (area.x0, area.y0, area.x1, area.y1) =
                (area.x0 + 1, area.y0 + 1, area.x1 - 1, area.y1 - 1);
            Covered::new(area, 0.0)
        });
        let paint = |mut canvas: Canvas<'_>| {
            canvas.fill_except(background, cover);
            paint(&mut Frame {
                canvas,
                glyphs,
                strokes,
                gamma,
                scale,
                bounds,
                limit: bounds,
                opacity: 1.0,
            });
        };

        match &mut self.target {
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
        }
    }

    /// 在透明底上离屏绘制一张 `width`×`height`（逻辑像素）的小图，按当前缩放系数绘制，
    /// 用于生成系统菜单图标。
    ///
    /// 画布不保存透明度，因此 `paint` 会画两遍（一次黑底、一次白底），再逐像素还原透明度。
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
                scale: self.scale,
                bounds: Rect::new(0.0, 0.0, width, height),
                limit: Rect::new(0.0, 0.0, width, height),
                opacity: 1.0,
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
