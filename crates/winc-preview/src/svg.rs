//! SVG 预览：自行解析 SVG，用 zeno 绘制，不依赖完整的 SVG 渲染库。
//!
//! 支持路径与基本形状、填充、描边、变换、整组透明度、线性和径向渐变、图案、裁剪、蒙版、
//! 常用滤镜、`<use>`、`<symbol>`、嵌套的 `<svg>`、`<switch>`、位图、`<style>` 里的 CSS、
//! 文字（字形由调用方提供，见 [`Glyphs`]）。
//!
//! 不支持的内容（箭头标记、动画、少见的滤镜等）跳过并记在 [`Drawing::missing`] 中，由状态栏显示。

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use roxmltree::{Node, NodeId};
use zeno::{Cap, Command, Fill, Join};

use crate::image::{Picture, Pixels, Source};

mod bitmap;
mod css;
mod dtd;
mod filter;
mod masking;
mod paint;
mod render;
mod shapes;
mod style;
mod syntax;
mod text;

use filter::Filter;
use masking::*;
use paint::*;
use syntax::*;

pub use render::MAX_EDGE;
pub use text::{FontMetrics, Glyph, Glyphs};

/// 没写大小也没有 `viewBox` 时的默认大小（像素，宽 300、高 150）。
///
/// 依据 CSS 2.1 对没有固有尺寸的可替换元素的规定：宽取 300 px，高取 150 px。
const DEFAULT_SIZE: (f32, f32) = (300.0, 150.0);
/// `<use>` 嵌套引用的最大层数（层，经验值），防止互相引用造成循环。
const MAX_DEPTH: usize = 24;
/// 最多绘制的形状数（个，经验值）。
///
/// 超过该数量的 SVG 绘制耗时过长，因此截断。
const MAX_SHAPES: usize = 200_000;
/// 最多展开的元素数（个，经验值）。
///
/// `<use>` 每层引用十个上一层时，展开的元素数是 10 的 n 次方（n 为层数），层数稍多就会达到
/// 数十亿个。即使最内层是空的（形状数达不到 [`MAX_SHAPES`]）也展开不完，因此需要单独的上限。
const MAX_ELEMENTS: usize = 1_000_000;
/// 裁剪、蒙版、图案、滤镜一共最多展开的元素数（个，经验值），每引用一次算一个，里面的每个
/// 元素、形状、滤镜原语再各算一个。
///
/// 裁剪、蒙版、图案、滤镜每被引用一次就要重新展开、绘制一遍里面的内容：一千个形状的
/// 蒙版被一千个元素引用，要绘制一百万次蒙版里的形状，几 KB 的文件就能画上几分钟。
/// 超过时其余的效果不再生效。
const MAX_EFFECT_WORK: usize = 3_000;
/// 滤镜原语一共最多用几次（次，经验值）：每个原语都要把滤镜区域整个算一遍，模糊这类原语在大的
/// 区域上一次就要几毫秒，比展开一个形状贵得多，所以单独计数。
///
/// 超过时其余的滤镜不再生效。
const MAX_FILTER_WORK: usize = 300;
/// 图案里最多再嵌套几层图案（层）：图案每层引用十个上一层时，绘制的图块数按层数指数增长。
const MAX_PATTERN_NESTING: usize = 2;
/// 元素最多嵌套的层数（层，经验值）。
///
/// XML 解析按层递归，几千层就会耗尽后台线程的栈，导致进程中止；绘制本来也只展开
/// [`MAX_DEPTH`] 层，因此在解析之前先数一遍。
const MAX_NESTING: usize = 512;
/// `XLink` 的命名空间 URI，用于识别 `xlink:href` 属性。
const XLINK: &str = "http://www.w3.org/1999/xlink";

/// 解析好的一张 SVG，按要显示的大小现画。
#[derive(Debug)]
pub struct Drawing {
    /// SVG 自己声明的宽度（像素）。
    pub width: f32,
    /// SVG 自己声明的高度（像素）。
    pub height: f32,
    /// `viewBox`，依次是 `min-x`、`min-y`、`width`、`height`。
    view_box: [f32; 4],
    /// `preserveAspectRatio` 的解析结果。
    aspect: Aspect,
    /// 按绘制顺序排列的顶层图元。
    items: Vec<Item>,
    /// 没有画出来的内容的名称；状态栏显示为「未显示：箭头标记、动画」这样的形式。
    pub missing: Vec<&'static str>,
}

/// 读取一张 SVG 用于预览。
///
/// `base` 是 SVG 文件所在的目录，`<image>` 引用同目录的位图时从这里查找；
/// `glyphs` 提供文字所需的字形，为 `None` 时不绘制文字。
///
/// # Errors
///
/// XML 语法错误或根元素不是 `<svg>` 时返回原因。
pub fn load(
    text: &str,
    base: Option<&Path>,
    glyphs: Option<Arc<dyn Glyphs>>,
) -> Result<Picture, String> {
    let drawing = Drawing::parse(text, base, glyphs)?;
    Ok(Picture {
        width: drawing.width.round().max(1.0) as u32,
        height: drawing.height.round().max(1.0) as u32,
        format: "SVG",
        file_size: text.len() as u64,
        source: Source::Vector(Arc::new(drawing)),
        transparent: true,
    })
}

/// `preserveAspectRatio` 的解析结果：`viewBox` 如何放进视口。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Aspect {
    /// `none`：横竖各自拉伸。
    stretch: bool,
    /// 0、0.5、1：靠左（上）、居中、靠右（下）。
    align: (f32, f32),
    /// 是否为 `slice`（铺满视口并裁掉超出的部分）；否则为 `meet`（完整放进视口）。
    slice: bool,
}

/// 一个图元：形状、组或位图。
#[derive(Debug)]
enum Item {
    /// 一个形状（路径加上填充、描边）。
    Shape(Shape),
    /// 一组图元，带自己的透明度、裁剪、蒙版或滤镜。
    Group(Box<Group>),
    /// 一张位图。
    Image(ImageItem),
}

/// 一组图元：先单独画在一层上，再做滤镜、裁剪、蒙版，最后按透明度叠加到下层。
#[derive(Debug)]
struct Group {
    opacity: f32,
    /// `mix-blend-mode`：与下层已画好的内容混合。
    blend: filter::Blend,
    clip: Option<Arc<Clip>>,
    mask: Option<Arc<MaskRef>>,
    filter: Option<Arc<Filter>>,
    items: Vec<Item>,
}

#[derive(Debug)]
struct Shape {
    path: Vec<Command>,
    /// 路径在自己坐标里的外框 [x, y, w, h]。
    bounds: [f32; 4],
    /// 从形状自己的坐标到 SVG 根坐标的变换（尚未包含 viewBox 变换）。
    transform: Affine,
    fill: Option<(Paint, Fill)>,
    stroke: Option<(Paint, StrokeStyle)>,
    /// `paint-order: stroke`：先描边再填充。
    stroke_first: bool,
}

#[derive(Debug, Clone)]
struct StrokeStyle {
    width: f32,
    cap: Cap,
    join: Join,
    miter: f32,
    dashes: Arc<[f32]>,
    offset: f32,
    /// `vector-effect: non-scaling-stroke`：线宽按屏幕像素计。
    non_scaling: bool,
}

/// 填充或描边的颜料：纯色（不预乘，已含透明度）、渐变或图案。
#[derive(Debug, Clone)]
enum Paint {
    Solid([u8; 4]),
    Gradient(Arc<Gradient>),
    Pattern(Arc<Pattern>),
}

/// 图案：用一块图块（`items`，使用图块自己的坐标）平铺。
#[derive(Debug)]
struct Pattern {
    /// 图块的大小（图块坐标）。
    size: (f32, f32),
    /// 从图块坐标到形状自己的坐标的变换。
    transform: Affine,
    items: Vec<Item>,
    opacity: f32,
}

#[derive(Debug)]
struct Gradient {
    radial: bool,
    /// 线性：x1 y1 x2 y2；径向：cx cy r fx fy fr。
    geometry: [f32; 6],
    /// 从渐变的坐标到形状自己的坐标的变换。
    transform: Affine,
    spread: Spread,
    /// 渐变在 0 到 255 每一档上的颜色（已预乘透明度）；引用同一个渐变的形状共用一张。
    table: Arc<[[u8; 4]; 256]>,
}

/// 渐变的色标解析之后的结果。
#[derive(Debug, Clone)]
enum Ramp {
    /// 只有一个色标：整个渐变是这一种颜色（不预乘，已含透明度）。
    Solid([u8; 4]),
    /// 两个以上的色标：见 [`Gradient::table`]。
    Table(Arc<[[u8; 4]; 256]>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Spread {
    Pad,
    Reflect,
    Repeat,
}

/// 裁剪：这些形状合起来覆盖的区域保留，其余裁掉。
///
/// 形状的变换已是到 SVG 根的变换。
///
/// `clip` 表示裁剪路径自身又被裁剪（`<clipPath clip-path="…">`）。
#[derive(Debug)]
struct Clip {
    shapes: Vec<(Vec<Command>, Affine, Fill)>,
    clip: Option<Arc<Clip>>,
}

/// 蒙版：绘制 `items`，按亮度（或透明度）决定保留多少；`region` 之外的部分全部不保留。
#[derive(Debug)]
struct MaskRef {
    items: Vec<Item>,
    region: Arc<Clip>,
    luminance: bool,
    /// 蒙版自己也可以有蒙版。
    mask: Option<Arc<MaskRef>>,
}

/// 内嵌的位图：`transform` 是从图片的像素坐标到 SVG 根坐标的变换。
#[derive(Debug)]
struct ImageItem {
    pixels: Arc<Pixels>,
    transform: Affine,
    /// 放大时保留像素方块（`image-rendering: pixelated`）。
    pixelated: bool,
}

/// 一个元素上能继承给子元素的样式。
#[derive(Debug, Clone)]
struct Props {
    fill: PaintSpec,
    fill_opacity: f32,
    fill_rule: Fill,
    clip_rule: Fill,
    stroke: PaintSpec,
    stroke_opacity: f32,
    stroke_width: f32,
    cap: Cap,
    join: Join,
    miter: f32,
    /// 与 `text::Font` 的字族列表一样用 `Arc` 共享：每个元素都要复制一次父元素的样式。
    dashes: Arc<[f32]>,
    dash_offset: f32,
    color: [u8; 4],
    visible: bool,
    stroke_first: bool,
    /// `color-interpolation-filters`：滤镜是否在线性 RGB 中计算（默认），否则在 sRGB 中计算。
    filters_linear: bool,
    pixelated: bool,
    font: text::Font,
}

impl Default for Props {
    fn default() -> Self {
        Props {
            fill: PaintSpec::Color([0, 0, 0, 255]),
            fill_opacity: 1.0,
            fill_rule: Fill::NonZero,
            clip_rule: Fill::NonZero,
            stroke: PaintSpec::None,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter: 4.0,
            dashes: Arc::new([]),
            dash_offset: 0.0,
            color: [0, 0, 0, 255],
            visible: true,
            stroke_first: false,
            filters_linear: true,
            pixelated: false,
            font: text::Font::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PaintSpec {
    None,
    Color([u8; 4]),
    CurrentColor,
    /// `url(#id)`，后面可以跟一个引用失败时使用的备用颜色。
    Url(String, Box<PaintSpec>),
}

/// `filter` 属性：引用一个 `<filter>`，或使用 CSS 的滤镜函数（`grayscale(1) blur(2px)`）。
#[derive(Debug, Clone)]
enum FilterRef {
    Url(String),
    Functions(String),
}

/// 元素自己的、不继承的属性。
#[derive(Debug, Clone)]
struct Own {
    opacity: f32,
    shown: bool,
    transform: Affine,
    clip_path: Option<String>,
    mask: Option<String>,
    filter: Option<FilterRef>,
    /// `overflow: visible`：嵌套的 `<svg>`、`<symbol>` 不裁掉视口外的部分。
    overflow_visible: bool,
    non_scaling: bool,
    mask_alpha: bool,
    blend: filter::Blend,
}

/// 元素上的一条样式声明（属性名、值），见 [`Parser::declared`]。
///
/// 值直接引用 SVG 文本或样式表；只有 `style` 属性里含大写字母的属性名要另存一个小写的副本，
/// 因此属性名是 `Cow`。
type Declaration<'a> = (Cow<'a, str>, &'a str);

/// 遍历 SVG 元素、生成图元的解析器。
///
/// 一次解析对应一个实例：`ids`、`sheet` 在解析开始时建好，各种计数用于限制展开量。
struct Parser<'a, 'input> {
    ids: HashMap<&'a str, Node<'a, 'input>>,
    /// `<style>` 里的规则，在创建解析器之前就已收集完毕。
    sheet: &'a css::Sheet,
    /// 当前视口（viewBox）的宽高，百分比相对它计算。
    ///
    /// 进入嵌套的 `<svg>` 时替换，退出时恢复。
    viewport: (f32, f32),
    missing: Vec<&'static str>,
    /// 已生成的形状数，见 [`MAX_SHAPES`]。
    shapes: usize,
    /// 已展开的元素数，见 [`MAX_ELEMENTS`]。
    elements: usize,
    /// 裁剪、蒙版、图案、滤镜已展开的元素数，见 [`MAX_EFFECT_WORK`]。
    effect_work: usize,
    /// 已用的滤镜原语数，见 [`MAX_FILTER_WORK`]。
    filter_work: usize,
    /// 正在展开的图案和蒙版的层数：大于 0 时展开的元素计入 `effect_work`。
    inside_effect: usize,
    /// 正在展开的图案的层数，见 [`MAX_PATTERN_NESTING`]。
    patterns: usize,
    base: Option<PathBuf>,
    /// 已解码的位图，按 `href` 索引：同一张图被多次引用（`<use>`）时只解码一次，内存中也只
    /// 保留一个副本。
    images: HashMap<&'a str, Option<Arc<Pixels>>>,
    /// `images` 中已解码的位图合计占用的字节数，上限见 [`Parser::image`]。
    image_bytes: u64,
    /// 已解析的渐变色标，按（含色标的渐变元素，透明度的二进制位，`currentColor`）索引：一个渐变
    /// 常被成千上万个形状引用，色标只解析一次，颜色表只存一张。
    ///
    /// `None` 表示没有可见的颜色。
    ramps: HashMap<(NodeId, u32, [u8; 4]), Option<Ramp>>,
    /// 正在展开的引用（clip-path、mask、filter、use），用于防止循环引用。
    active: Vec<Node<'a, 'input>>,
    glyphs: Option<Arc<dyn Glyphs>>,
}

impl Drawing {
    /// 解析 SVG 文本。
    ///
    /// # Errors
    ///
    /// XML 语法错误或根元素不是 `<svg>` 时返回原因。
    pub fn parse(
        text: &str,
        base: Option<&Path>,
        glyphs: Option<Arc<dyn Glyphs>>,
    ) -> Result<Drawing, String> {
        // 先展开 DOCTYPE 中的实体，之后的 XML 解析不支持 DTD（见 `dtd`）。
        let text = dtd::expand(text)?;
        if nesting_exceeds(&text, MAX_NESTING) {
            return Err(winc_i18n::t!(
                "preview-svg-nesting-too-deep",
                max = MAX_NESTING
            ));
        }
        let document = roxmltree::Document::parse(&text)
            .map_err(|e| winc_i18n::t!("preview-svg-xml-error", error = e.to_string()))?;
        let root = document.root_element();
        if root.tag_name().name() != "svg" {
            return Err(winc_i18n::t!("preview-svg-not-svg").to_owned());
        }

        let view_box = view_box(root);
        // 百分比、无法解析和不大于 0 的宽高都按没写处理。
        let size = |name: &str| {
            root.attribute(name)
                .map(str::trim)
                .filter(|v| !v.ends_with('%'))
                .and_then(|v| length(v, 0.0))
                .filter(|l| *l > 0.0)
        };
        let (width, height) = match (size("width"), size("height"), view_box) {
            (Some(w), Some(h), _) => (w, h),
            // 只写了宽或高且有 viewBox 时：另一边按 viewBox 的宽高比推算。
            (Some(w), None, Some(v)) => (w, w * v[3] / v[2]),
            (None, Some(h), Some(v)) => (h * v[2] / v[3], h),
            (None, None, Some(v)) => (v[2], v[3]),
            (w, h, None) => (w.unwrap_or(DEFAULT_SIZE.0), h.unwrap_or(DEFAULT_SIZE.1)),
        };
        let view_box = view_box.unwrap_or([0.0, 0.0, width, height]);

        let mut ids = HashMap::new();
        let mut sheet = css::Sheet::default();
        let mut unknown_css = false;
        for node in root.descendants().filter(Node::is_element) {
            if let Some(id) = node.attribute("id") {
                ids.entry(id).or_insert(node);
            }
            if node.tag_name().name() == "style" {
                let css: String = node.children().filter_map(|c| c.text()).collect();
                unknown_css |= sheet.add(&css);
            }
        }

        let mut parser = Parser {
            ids,
            sheet: &sheet,
            viewport: (view_box[2], view_box[3]),
            missing: Vec::new(),
            shapes: 0,
            elements: 0,
            effect_work: 0,
            filter_work: 0,
            inside_effect: 0,
            patterns: 0,
            base: base.map(Path::to_path_buf),
            images: HashMap::new(),
            image_bytes: 0,
            ramps: HashMap::new(),
            active: Vec::new(),
            glyphs,
        };
        if unknown_css {
            parser.miss(winc_i18n::t!("preview-svg-missing-selectors"));
        }

        let mut items = Vec::new();
        let props = parser.props(root, &Props::default());
        let own = parser.own(root);
        parser.children(root, &props, Affine::IDENTITY, &mut items, 0);
        // 根元素上的裁剪、蒙版、滤镜、透明度同样生效。
        let mut root_items = Vec::new();
        parser.finish(&own, Affine::IDENTITY, items, &mut root_items);
        let aspect = aspect(root.attribute("preserveAspectRatio"));
        Ok(Drawing {
            width,
            height,
            view_box,
            aspect,
            items: root_items,
            missing: parser.missing,
        })
    }
}

fn view_box(node: Node) -> Option<[f32; 4]> {
    node.attribute("viewBox")
        .map(numbers)
        .filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0)
        .map(|v| [v[0], v[1], v[2], v[3]])
}

impl<'a, 'input> Parser<'a, 'input> {
    /// 裁剪、蒙版、图案、滤镜再展开一个元素：计入 [`MAX_EFFECT_WORK`]，超出时返回 `false`。
    fn effect_step(&mut self) -> bool {
        self.effect_work += 1;
        if self.effect_work > MAX_EFFECT_WORK {
            self.miss(winc_i18n::t!("preview-svg-missing-too-many-effects"));
            return false;
        }
        true
    }

    /// 滤镜再用一个原语：计入 [`MAX_FILTER_WORK`]，超出时返回 `false`。
    fn filter_step(&mut self) -> bool {
        self.filter_work += 1;
        if self.filter_work > MAX_FILTER_WORK {
            self.miss(winc_i18n::t!("preview-svg-missing-too-many-effects"));
            return false;
        }
        true
    }

    /// 记下没有画出来的内容：`what` 是已翻译的名称，重复的只记一次。
    fn miss(&mut self, what: &'static str) {
        if !self.missing.contains(&what) {
            self.missing.push(what);
        }
    }

    /// 返回视口的归一化对角线长度 `√((w² + h²) / 2)`，SVG 规范中非横纵方向的百分比长度以它为参考。
    fn diagonal(&self) -> f32 {
        let (w, h) = self.viewport;
        ((w * w + h * h) / 2.0).sqrt()
    }

    fn children(
        &mut self,
        node: Node<'a, 'input>,
        props: &Props,
        ctm: Affine,
        out: &mut Vec<Item>,
        depth: usize,
    ) {
        for child in node.children().filter(Node::is_element) {
            self.element(child, props, ctm, out, depth);
        }
    }

    fn element(
        &mut self,
        node: Node<'a, 'input>,
        parent: &Props,
        parent_ctm: Affine,
        out: &mut Vec<Item>,
        depth: usize,
    ) {
        if depth > MAX_DEPTH || self.shapes > MAX_SHAPES || self.elements > MAX_ELEMENTS {
            return;
        }
        if self.inside_effect > 0 && !self.effect_step() {
            return;
        }
        self.elements += 1;
        let tag = node.tag_name().name();
        match tag {
            // 只在被引用时才绘制，或者根本不绘制。
            "defs" | "symbol" | "linearGradient" | "radialGradient" | "stop" | "style"
            | "title" | "desc" | "metadata" | "clipPath" | "mask" | "pattern" | "marker"
            | "filter" | "script" | "font" | "font-face" | "glyph" | "missing-glyph" | "cursor"
            | "view" => {
                return;
            }
            "animate" | "animateTransform" | "animateMotion" | "animateColor" | "set" => {
                return self.miss(winc_i18n::t!("preview-svg-missing-animation"));
            }
            "foreignObject" => {
                return self.miss(winc_i18n::t!("preview-svg-missing-foreign-object"));
            }
            _ => {}
        }
        // 其他命名空间里的元素（绘图编辑器写入的私有数据）不绘制。
        if node
            .tag_name()
            .namespace()
            .is_some_and(|ns| ns != "http://www.w3.org/2000/svg")
        {
            return;
        }
        let declared = self.declared(node);
        let own = Self::own_from(&declared);
        if !own.shown || own.opacity <= 0.0 {
            return;
        }
        let props = self.props_from(&declared, parent);
        drop(declared);
        let ctm = own.transform.then(parent_ctm);

        let mut items = Vec::new();
        match tag {
            "g" | "a" => self.children(node, &props, ctm, &mut items, depth + 1),
            "switch" => {
                // 只绘制第一个条件满足的子元素。
                let chosen = node.children().filter(Node::is_element).find(|c| {
                    c.attribute("requiredExtensions").is_none_or(str::is_empty)
                        && c.attribute("requiredFeatures").is_none()
                });
                if let Some(child) = chosen {
                    self.element(child, &props, ctm, &mut items, depth + 1);
                }
            }
            "svg" => {
                let (vw, vh) = self.viewport;
                let x = len_attr(node, "x", vw);
                let y = len_attr(node, "y", vh);
                let w = node
                    .attribute("width")
                    .and_then(|v| length(v, vw))
                    .unwrap_or(vw);
                let h = node
                    .attribute("height")
                    .and_then(|v| length(v, vh))
                    .unwrap_or(vh);
                if w <= 0.0 || h <= 0.0 {
                    return;
                }
                self.viewport_children(node, &props, &own, ctm, (x, y, w, h), &mut items, depth);
            }
            "use" => self.use_element(node, &props, ctm, &mut items, depth),
            "image" => self.image(node, &props, ctm, &mut items),
            "text" => self.text(node, (&props, &own), ctm, &mut items, depth),
            _ => {
                if let Some(path) = self.geometry(node) {
                    self.shape(node, path, &props, &own, ctm, &mut items);
                }
            }
        }
        self.finish(&own, ctm, items, out);
    }

    /// 元素绘制完成后，套上它自己的裁剪、蒙版、滤镜和透明度。
    fn finish(&mut self, own: &Own, ctm: Affine, items: Vec<Item>, out: &mut Vec<Item>) {
        let has_effect = own.clip_path.is_some() || own.mask.is_some() || own.filter.is_some();
        // 没有裁剪、蒙版、滤镜时不需要计算外框。
        if !has_effect || !self.effect_step() {
            push_group(out, (own.opacity, own.blend), None, None, None, items);
            return;
        }
        let bbox = ctm.invert().and_then(|inverse| bbox_of(&items, inverse));
        let clip = match own
            .clip_path
            .as_deref()
            .and_then(|id| self.ids.get(id).copied())
        {
            Some(target) if target.tag_name().name() == "clipPath" => {
                match self.clip(target, ctm, bbox, 0) {
                    Some(clip) => Some(clip),
                    // 裁剪按外框计算而元素没有外框：什么都不剩。
                    None => return,
                }
            }
            _ => None,
        };
        let mask = match own.mask.as_deref().and_then(|id| self.ids.get(id).copied()) {
            Some(target) if target.tag_name().name() == "mask" => {
                match self.mask(target, ctm, bbox, 0) {
                    Some(mask) => Some(mask),
                    None => return,
                }
            }
            _ => None,
        };
        let parsed = match &own.filter {
            Some(FilterRef::Url(id)) => match self.ids.get(id.as_str()).copied() {
                Some(target) if target.tag_name().name() == "filter" => {
                    Some(filter::parse(self, target, ctm, bbox))
                }
                _ => None,
            },
            Some(FilterRef::Functions(text)) => Some(filter::functions(self, text, ctm, bbox)),
            None => None,
        };
        let filter = match parsed {
            Some(filter::Parsed::Filter(f)) => Some(Arc::new(f)),
            Some(filter::Parsed::Hide) => return,
            Some(filter::Parsed::Unsupported(what)) => {
                self.miss(what);
                None
            }
            None => None,
        };
        push_group(out, (own.opacity, own.blend), clip, mask, filter, items);
    }

    /// 为嵌套的 `<svg>`、`<use>` 引用的 `<symbol>` 在（`x`、`y`、`w`、`h`）区域内开一个新视口，
    /// 视口外的部分裁掉（`overflow: visible` 时不裁）。
    #[allow(clippy::too_many_arguments)]
    fn viewport_children(
        &mut self,
        node: Node<'a, 'input>,
        props: &Props,
        own: &Own,
        ctm: Affine,
        (x, y, w, h): (f32, f32, f32, f32),
        items: &mut Vec<Item>,
        depth: usize,
    ) {
        let view = view_box(node);
        let inner = match view {
            Some(v) => fit(v, aspect(node.attribute("preserveAspectRatio")), w, h)
                .then(Affine::translate(x, y)),
            None => Affine::translate(x, y),
        };
        let saved = self.viewport;
        self.viewport = view.map_or((w, h), |v| (v[2], v[3]));
        let mut inside = Vec::new();
        self.children(node, props, inner.then(ctm), &mut inside, depth + 1);
        self.viewport = saved;
        let clip = (!own.overflow_visible).then(|| rect_clip(x, y, w, h, ctm));
        push_group(
            items,
            (1.0, filter::Blend::Normal),
            clip,
            None,
            None,
            inside,
        );
    }

    fn use_element(
        &mut self,
        node: Node<'a, 'input>,
        props: &Props,
        ctm: Affine,
        items: &mut Vec<Item>,
        depth: usize,
    ) {
        let Some(target) = href(node).and_then(|id| self.ids.get(id).copied()) else {
            return;
        };
        // 引用的是自己或自己的上级（`<g id="a"><use href="#a"/></g>`）：按 SVG 规范这是错误，
        // 不绘制（`ancestors` 从自己算起）。
        if self.active.contains(&target) || node.ancestors().any(|a| a == target) {
            return;
        }
        let (vw, vh) = self.viewport;
        let x = len_attr(node, "x", vw);
        let y = len_attr(node, "y", vh);
        let ctm = Affine::translate(x, y).then(ctm);
        self.active.push(target);
        if target.tag_name().name() == "symbol" {
            let size = |name: &str, fallback: f32| {
                node.attribute(name)
                    .or_else(|| target.attribute(name))
                    .and_then(|v| length(v, fallback))
                    .unwrap_or(fallback)
            };
            let (w, h) = (size("width", vw), size("height", vh));
            let declared = self.declared(target);
            let target_own = Self::own_from(&declared);
            if target_own.shown && w > 0.0 && h > 0.0 {
                let symbol_props = self.props_from(&declared, props);
                drop(declared);
                let symbol_ctm = target_own.transform.then(ctm);
                let mut inside = Vec::new();
                self.viewport_children(
                    target,
                    &symbol_props,
                    &target_own,
                    symbol_ctm,
                    (0.0, 0.0, w, h),
                    &mut inside,
                    depth,
                );
                self.finish(
                    &Own {
                        transform: Affine::IDENTITY,
                        ..target_own
                    },
                    symbol_ctm,
                    inside,
                    items,
                );
            }
        } else {
            self.element(target, props, ctm, items, depth + 1);
        }
        self.active.pop();
    }
}

/// 返回元素的长度属性 `name`（百分比相对 `reference`）；没写或无法解析时返回 0。
fn len_attr(node: Node, name: &str, reference: f32) -> f32 {
    node.attribute(name)
        .and_then(|v| length(v, reference))
        .unwrap_or(0.0)
}

/// 返回单位取外框比例（`objectBoundingBox`）时从 0..1 到元素坐标的变换；单位取用户空间时
/// 返回恒等变换；元素没有外框时返回 `None`。
fn units(
    node: Node,
    attribute: &str,
    default_bbox: bool,
    bbox: Option<[f32; 4]>,
) -> Option<Affine> {
    let by_box = match node.attribute(attribute) {
        Some("userSpaceOnUse") => false,
        Some("objectBoundingBox") => true,
        _ => default_bbox,
    };
    if !by_box {
        return Some(Affine::IDENTITY);
    }
    let b = bbox.filter(|b| b[2] > 0.0 && b[3] > 0.0)?;
    Some(Affine([b[2], 0.0, 0.0, b[3], b[0], b[1]]))
}

/// 返回元素的 `href`（或旧写法 `xlink:href`）指向的 id；不是以 `#` 开头的片段引用时返回 `None`。
fn href<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attribute((XLINK, "href"))
        .or_else(|| node.attribute("href"))
        .and_then(|h| h.trim().strip_prefix('#'))
}

/// 解析比例：`0.5` 和 `50%` 都表示一半。
fn fraction(value: &str) -> Option<f32> {
    let v = value.trim();
    match v.strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().ok().map(|p| p / 100.0),
        None => v.parse().ok(),
    }
}

fn path_bounds(path: &[Command]) -> [f32; 4] {
    let b = zeno::bounds(path, Fill::NonZero, None);
    [b.min.x, b.min.y, b.max.x - b.min.x, b.max.y - b.min.y]
}

/// 返回一组图元在某个坐标系中的外框 `[x, y, w, h]`；`inverse` 是从 SVG 根坐标系到该坐标系的变换。
fn bbox_of(items: &[Item], inverse: Affine) -> Option<[f32; 4]> {
    // 目标坐标系中的 [x0, y0, x1, y1]。
    let mut acc: Option<[f32; 4]> = None;
    let mut add = |(x, y): (f32, f32)| {
        acc = Some(match acc {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    };
    let corners = |[x, y, w, h]: [f32; 4]| [(x, y), (x + w, y), (x, y + h), (x + w, y + h)];
    for item in items {
        let (rect, transform) = match item {
            Item::Shape(s) => (s.bounds, s.transform),
            Item::Image(i) => (
                [0.0, 0.0, i.pixels.width as f32, i.pixels.height as f32],
                i.transform,
            ),
            Item::Group(g) => {
                // 子组算出的已经是目标坐标系中的外框，直接并入。
                if let Some(b) = bbox_of(&g.items, inverse) {
                    corners(b).into_iter().for_each(&mut add);
                }
                continue;
            }
        };
        let to_target = transform.then(inverse);
        for (x, y) in corners(rect) {
            add(to_target.apply(x, y));
        }
    }
    acc.map(|[x0, y0, x1, y1]| [x0, y0, x1 - x0, y1 - y0])
}

/// 把元素展开得到的 `items` 放进 `out`：需要透明度、混合模式、裁剪、蒙版或滤镜时包成一个组，
/// 否则直接并入。
///
/// 组在绘制时要单独占一层与画布同样大小的画布，再叠加回去；导出的 SVG 常给成千上万个路径各自
/// 写上 `opacity`，逐个分层会比直接绘制慢一个数量级。因此只有一个纯色形状、只是半透明时，
/// 把透明度直接乘进颜色（见 [`fade_solid`]），不再包成组。
fn push_group(
    out: &mut Vec<Item>,
    (opacity, blend): (f32, filter::Blend),
    clip: Option<Arc<Clip>>,
    mask: Option<Arc<MaskRef>>,
    filter: Option<Arc<Filter>>,
    mut items: Vec<Item>,
) {
    // 滤镜可以凭空生成内容（例如填色），因此没有图元时也要保留。
    if items.is_empty() && filter.is_none() {
        return;
    }
    let simple =
        blend == filter::Blend::Normal && clip.is_none() && mask.is_none() && filter.is_none();
    let inline = simple
        && (opacity >= 1.0
            || match items.as_mut_slice() {
                [Item::Shape(shape)] => fade_solid(shape, opacity),
                _ => false,
            });
    if inline {
        out.extend(items);
    } else {
        out.push(Item::Group(Box::new(Group {
            opacity,
            blend,
            clip,
            mask,
            filter,
            items,
        })));
    }
}

/// 形状只有填充或只有描边、且颜料是纯色时，把透明度 `opacity` 乘进颜色的 alpha，返回 `true`；
/// 其余情况不改动形状，返回 `false`。
///
/// 单独一次填充（或描边）先算出整体的覆盖度再上色，形状自身重叠的部分也只上色一次，因此先画在
/// 单独一层上再整体乘透明度，与直接用乘过透明度的颜色绘制结果相同（只差取整的 1/255）。填充和
/// 描边都有时二者相交处会叠两次色，渐变和图案的颜色表、图块又由多个形状共用，这些情况仍按组绘制。
fn fade_solid(shape: &mut Shape, opacity: f32) -> bool {
    let paint = match (&mut shape.fill, &mut shape.stroke) {
        (Some((paint, _)), None) | (None, Some((paint, _))) => paint,
        _ => return false,
    };
    let Paint::Solid(color) = paint else {
        return false;
    };
    color[3] = (f32::from(color[3]) * opacity.clamp(0.0, 1.0)).round() as u8;
    true
}

/// `text` 里元素的嵌套是否超过 `limit` 层。
///
/// 只粗略地识别 XML 写法：注释、CDATA、`<!DOCTYPE>`、处理指令、属性值里的 `>`。多算几层不要紧，
/// 只要不会少算。
fn nesting_exceeds(text: &str, limit: usize) -> bool {
    let bytes = text.as_bytes();
    let skip_to = |from: usize, end: &[u8]| {
        memchr::memmem::find(&bytes[from..], end).map_or(bytes.len(), |i| from + i + end.len())
    };
    let (mut i, mut depth) = (0, 0usize);
    while let Some(at) = memchr::memchr(b'<', &bytes[i..]) {
        let at = i + at;
        let rest = &bytes[at + 1..];
        i = if rest.starts_with(b"!--") {
            skip_to(at, b"-->")
        } else if rest.starts_with(b"![CDATA[") {
            skip_to(at, b"]]>")
        } else if rest.starts_with(b"?") {
            skip_to(at, b"?>")
        } else if rest.starts_with(b"!") {
            // `<!DOCTYPE … [ … ]>`：内部子集里的声明也带尖括号。
            let mut j = at + 2;
            let mut brackets = 0usize;
            while j < bytes.len() {
                match bytes[j] {
                    b'[' => brackets += 1,
                    b']' => brackets = brackets.saturating_sub(1),
                    b'>' if brackets == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            j + 1
        } else {
            // 开始或结束标签：找到不在引号里的 `>`。
            let mut j = at + 1;
            let mut quote = None;
            while j < bytes.len() {
                match (quote, bytes[j]) {
                    (None, b'"' | b'\'') => quote = Some(bytes[j]),
                    (Some(q), c) if c == q => quote = None,
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            if rest.starts_with(b"/") {
                depth = depth.saturating_sub(1);
            } else if bytes.get(j.wrapping_sub(1)) != Some(&b'/') {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            j + 1
        };
        if i >= bytes.len() {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests;
