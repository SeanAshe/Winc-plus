//! Markdown 预览的排版：把解析好的块排成一项项待绘制的内容（文字、底色、图片、链接范围）。
//!
//! [`LaidBlock`] 是排好的一块，[`Item`] 是其中一项，[`Cx`] 带着排版上下文逐块排；行内文字、
//! 公式、流程图、饼图和时序图在子模块中。坐标都是逻辑像素，相对顶层块的左上角。排版不依赖
//! 应用层：配色、图片的状态和图片地址的解析由调用方通过 [`Cx`] 传入。

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::Arc;

use winc_preview::image::{Picture, Pixels};
use winc_preview::markdown::{Alert, Align, Block, Inline, Item as ListItem, SourceMap, Style};
use winc_render::{Color, Faux, FontKind, Frame, Rect, Stroke};
use winc_syntax::TokenKind;

/// 一张图片的状态。
pub enum ImageSlot {
    /// 正在后台读取。
    Loading,
    /// 已读取；`pixels` 是按显示大小绘制的像素，最近绘制的在前（同一张图可能以多种大小出现，
    /// 例如并排的截图和表格中的缩略图）；`drawing` 是正在后台绘制的大小。
    Ready {
        /// 原图的宽度（像素）。
        width: u32,
        /// 原图的高度（像素）。
        height: u32,
        /// 解码后的图片；不再需要绘制时释放，为 `None`，需要绘制时重新读取。
        picture: Option<Arc<Picture>>,
        /// 按显示大小绘制的像素。
        pixels: Vec<Arc<Pixels>>,
        /// 正在后台绘制的大小（物理像素）。
        drawing: Vec<(u32, u32)>,
    },
    /// 无法读取（文件不存在或格式不支持）：显示替代文字。
    Failed,
}

mod block;
mod code;
mod diagram;
mod inline;
mod math;
mod memo;
mod raw;
mod table;

pub use memo::Memo;
pub use memo::{key as memo_key, shift};

/// 一个排好版的顶层块：要画的项，坐标相对块的左上角。
#[derive(Default)]
pub struct LaidBlock {
    /// 块的高度（逻辑像素）。
    pub height: f32,
    /// 要画的项，按绘制先后排列。
    pub items: Vec<Item>,
    /// 标题的锚点（小写、空格换成 `-`、去掉标点）和所在高度，`[跳转](#标题)` 用。
    pub anchors: Vec<(String, f32)>,
    /// 块里能选中的文字连起来：段落、单元格、代码行之间是换行，折行处不加。
    ///
    /// 选中、复制、查找都按它，位置是字符偏移。
    pub text: Box<str>,
    /// `text` 有多少个字符。
    pub chars: usize,
    /// 按高度找看得见的项用（见 [`LaidBlock::visible`]）。
    ///
    /// `tall` 是高的项（代码块的底色、引用的竖条、表格的竖线、大图）的序号，`short` 是矮的项
    /// 按上边缘排好的（上边缘，序号）。
    tall: Vec<u32>,
    short: Vec<(f32, u32)>,
    /// 排版这一块时使用的排版缓存条目，见 [`Memo::sweep`]。
    pub memo: Box<[u64]>,
    /// 输入法组字是否排在这一块中，见 [`Cx::preedit`]。
    pub preedit: bool,
    /// 比正文宽、可以横向滚动的表格，按项的先后排列。
    pub scrolls: Vec<Scroller>,
}

/// 比正文宽的表格：表格的各项按自然宽度排版，绘制时横向平移、裁剪到正文一栏内。
#[derive(Debug, Clone)]
pub struct Scroller {
    /// 属于该表格的项（[`LaidBlock::items`] 的序号）。
    pub items: std::ops::Range<u32>,
    /// 显示表格的区域（块里的坐标）：正文一栏宽，高度含表格下方的滚动条。
    pub rect: Rect,
    /// 表格的总宽度（逻辑像素）。
    pub content: f32,
}

impl Scroller {
    /// 最大横向滚动距离（逻辑像素）。
    pub fn max(&self) -> f32 {
        (self.content - self.rect.w).max(0.0)
    }
}

/// 表格外框的圆角半径（逻辑像素），与代码块的圆角相近。
const TABLE_RADIUS: f32 = 6.0;

/// 返回沿 `rect` 内侧描一圈、圆角半径为 `radius` 的外框的线段和圆弧：线宽 1 时恰好落在 `rect` 的
/// 最外一圈像素上。
fn rounded_frame(rect: Rect, radius: f32) -> Vec<Stroke> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let (l, t) = (rect.x + 0.5, rect.y + 0.5);
    let (r, b) = (rect.right() - 0.5, rect.bottom() - 0.5);
    let k = radius.min((r - l) / 2.0).min((b - t) / 2.0).max(0.0);
    vec![
        Stroke::Line(l + k, t, r - k, t),
        Stroke::Line(r, t + k, r, b - k),
        Stroke::Line(r - k, b, l + k, b),
        Stroke::Line(l, b - k, l, t + k),
        Stroke::Arc(l + k, t + k, k, -PI, -FRAC_PI_2),
        Stroke::Arc(r - k, t + k, k, -FRAC_PI_2, 0.0),
        Stroke::Arc(r - k, b - k, k, 0.0, FRAC_PI_2),
        Stroke::Arc(l + k, b - k, k, FRAC_PI_2, PI),
    ]
}

/// 表格文字与正文字号之比：表格比正文小一级。
const TABLE_TEXT: f32 = 0.875;

/// 可横向滚动的表格下方为滚动条留出的高度（逻辑像素）。
pub const SCROLL_BAR_ROOM: f32 = 10.0;

/// 高项与矮项的分界（逻辑像素，经验值）：高度超过该值的项算「高的」，每次绘制都检查一遍；
/// 矮的项按上边缘二分查找。
const TALL: f32 = 120.0;

impl LaidBlock {
    /// 创建一个排好的顶层块，同时按高度建好查找可见项的索引。
    pub fn new(
        height: f32,
        items: Vec<Item>,
        anchors: Vec<(String, f32)>,
        text: Box<str>,
        chars: usize,
        memo: Box<[u64]>,
    ) -> LaidBlock {
        let (mut tall, mut short) = (Vec::new(), Vec::new());
        for (i, item) in items.iter().enumerate() {
            let (top, bottom) = item.span_y();
            if bottom - top > TALL {
                tall.push(i as u32);
            } else {
                short.push((top, i as u32));
            }
        }
        short.sort_by(|a, b| a.0.total_cmp(&b.0));
        LaidBlock {
            height,
            items,
            anchors,
            text,
            chars,
            tall,
            short,
            memo,
            preedit: false,
            scrolls: Vec::new(),
        }
    }

    /// 第 `item` 项所在的可横向滚动的表格的序号（[`LaidBlock::scrolls`]）；不在其中时为 `None`。
    pub fn scroller_of(&self, item: usize) -> Option<usize> {
        let item = u32::try_from(item).ok()?;
        self.scrolls.iter().position(|s| s.items.contains(&item))
    }

    /// 块里有没有用到 `paths` 里的图片（排版时还在读的也算）。
    pub fn uses_any(&self, paths: &[PathBuf]) -> bool {
        !paths.is_empty()
            && self
                .items
                .iter()
                .any(|item| matches!(item, Item::Image { path, .. } if paths.contains(path)))
    }

    /// 块里有没有指向 `url` 的链接。
    pub fn links_to(&self, url: &str) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, Item::Link { url: u, .. } if &**u == url))
    }

    /// 和 `y0..y1`（块里的坐标）有交集的项的序号，按原来的先后（先画的底色在下面）。
    ///
    /// 长列表、长表格、长代码块是一个顶层块，里面成千上万项，每帧只看屏上那些。
    pub fn visible(&self, y0: f32, y1: f32, out: &mut Vec<u32>) {
        out.clear();
        let hit = |i: u32| {
            let (top, bottom) = self.items[i as usize].span_y();
            bottom >= y0 && top <= y1
        };
        out.extend(self.tall.iter().copied().filter(|&i| hit(i)));
        let from = self.short.partition_point(|&(top, _)| top < y0 - TALL);
        let to = self.short.partition_point(|&(top, _)| top <= y1);
        out.extend(
            self.short[from..to.max(from)]
                .iter()
                .map(|&(_, i)| i)
                .filter(|&i| hit(i)),
        );
        out.sort_unstable();
    }
}

impl Item {
    /// 上下边缘（块里的坐标）；文字算上选中时铺底色的范围。
    pub fn span_y(&self) -> (f32, f32) {
        match self {
            Item::Text { baseline, size, .. } => (baseline - size * 1.2, baseline + size * 0.5),
            Item::Fill { rect, .. }
            | Item::Corners { rect, .. }
            | Item::Image { rect, .. }
            | Item::Link { rect, .. }
            | Item::Spot { rect, .. }
            | Item::Check { rect, .. }
            | Item::Code { rect, .. }
            | Item::Table { rect, .. }
            | Item::Path { rect, .. }
            | Item::Polygon { rect, .. } => (rect.y, rect.bottom()),
        }
    }

    /// 左右边缘（块内坐标）；文字按各字符的宽度计算，宽度未知时只计算起点。
    pub fn span_x(&self) -> (f32, f32) {
        match self {
            Item::Text { x, span, .. } => {
                let w = span.as_ref().map_or(0.0, |s| s.advances.iter().sum());
                (*x, x + w)
            }
            Item::Fill { rect, .. }
            | Item::Corners { rect, .. }
            | Item::Image { rect, .. }
            | Item::Link { rect, .. }
            | Item::Spot { rect, .. }
            | Item::Check { rect, .. }
            | Item::Code { rect, .. }
            | Item::Table { rect, .. }
            | Item::Path { rect, .. }
            | Item::Polygon { rect, .. } => (rect.x, rect.right()),
        }
    }
}

/// 一段能选中的文字在块文字（[`LaidBlock::text`]）里从哪个字符开始、每个字的宽度，以及在
/// 原文中的位置。
#[derive(Clone)]
pub struct Span {
    /// 这一段在块文字里的起点（字符偏移）。
    pub start: usize,
    /// 每个字的宽度（逻辑像素）。
    pub advances: Arc<[f32]>,
    /// 第一个字在原文中的位置（字节偏移，相对所在顶层块的起点）；由原文生成的文字（提示块的
    /// 标题、图里的字）为 `None`。
    pub source: Option<u32>,
    /// 原文不连续之处：（第几个字，该字相对 `source` 的字节偏移），按字的先后排列。
    ///
    /// 其余的字在原文中紧接前一个字。
    pub jumps: Option<Arc<[(u32, u32)]>>,
}

impl Span {
    /// 第 `k` 个字（`k` 可以等于字数，即末尾）在原文中的位置，`text` 是这一段的文字。
    pub fn source_of(&self, text: &str, k: usize) -> Option<usize> {
        let at = self.source? as usize;
        let (from, base) = self
            .jumps
            .iter()
            .flat_map(|j| j.iter())
            .rev()
            .find(|&&(i, _)| i as usize <= k)
            .map_or((0, 0), |&(i, d)| (i as usize, d as usize));
        let bytes: usize = text
            .chars()
            .skip(from)
            .take(k - from)
            .map(char::len_utf8)
            .sum();
        Some(at + base + bytes)
    }

    /// 原文位置在 `source`（相对顶层块）之前的字有几个：选区的两端换成这一段里的字。
    pub fn chars_before(&self, text: &str, source: usize) -> usize {
        match self.char_at_source(text, source) {
            Some(k) if self.source_of(text, k).is_some_and(|s| s < source) => {
                (k + 1).min(self.advances.len())
            }
            Some(k) => k,
            None => 0,
        }
    }

    /// 原文位置 `source`（相对顶层块）落在第几个字之前：取原文位置不超过它的最后一个字；
    /// 比第一个字还靠前时为 `None`。
    pub fn char_at_source(&self, text: &str, source: usize) -> Option<usize> {
        let at = self.source? as usize;
        if at > source {
            return None;
        }
        let jumps = self.jumps.as_deref().unwrap_or_default();
        let (mut next, mut pos) = (0, at);
        for (k, c) in text.chars().enumerate() {
            while let Some(&(i, d)) = jumps.get(next)
                && i as usize <= k
            {
                pos = at + d as usize;
                next += 1;
            }
            // 第一个字的位置就是 `at`，不超过 `source`，所以这里的 `k` 至少是 1。
            if pos > source {
                return Some(k - 1);
            }
            pos += c.len_utf8();
        }
        let count = self.advances.len();
        Some(if pos <= source {
            count
        } else {
            count.saturating_sub(1)
        })
    }
}

/// 要画的一项。
///
/// 文字、宽度用 `Arc` 存，排版缓存（[`Memo`]）里取出来挪个位置就能用，
/// 不用再分配。坐标都是逻辑像素，相对顶层块的左上角。
#[derive(Clone)]
pub enum Item {
    /// 一段文字。
    Text {
        /// 相对顶层块左上角的横坐标。
        x: f32,
        /// 基线相对顶层块左上角的纵坐标。
        baseline: f32,
        /// 文字内容。
        text: Arc<str>,
        /// 字体。
        font: FontKind,
        /// 模拟的粗体与斜体。
        faux: Faux,
        /// 字号（逻辑像素）。
        size: f32,
        /// 颜色。
        color: Color,
        /// 能选中时在块文字里的位置；列表记号这类不算文字的为 `None`。
        span: Option<Span>,
    },
    /// 圆角半径为 `radius`（逻辑像素）的底色。
    Fill {
        /// 范围。
        rect: Rect,
        /// 颜色。
        color: Color,
        /// 圆角半径。
        radius: f32,
    },
    /// 四个角半径各不相同的底色，`radii` 依次是左上、右上、右下、左下（逻辑像素）。
    Corners {
        /// 范围。
        rect: Rect,
        /// 颜色。
        color: Color,
        /// 四个角的半径。
        radii: [f32; 4],
    },
    /// 一张图片。
    Image {
        /// 范围。
        rect: Rect,
        /// 图片对应的本地文件。
        path: PathBuf,
        /// 图片在块文字中的字符序号及其在原文中的位置（相对顶层块）。
        ///
        /// 点击图片时光标移到该位置；图片在块文字中不占字符。
        at: Option<(usize, u32)>,
    },
    /// 链接能点的范围，不画。
    Link {
        /// 范围。
        rect: Rect,
        /// 链接的地址。
        url: Arc<str>,
    },
    /// 点击时光标移到原文 `at` 处的范围，不画：单独成行的公式中没有可定位的文字。
    ///
    /// `at` 同 [`Item::Image`] 的 `at`：块文字中的字符序号和原文中的位置（相对顶层块）。
    Spot {
        /// 范围。
        rect: Rect,
        /// 块文字中的字符序号和原文中的位置。
        at: (usize, u32),
    },
    /// 任务列表的勾选框：`source` 是 `[ ]` 在原文中的位置（相对顶层块的开头），点击时切换。
    Check {
        /// 范围。
        rect: Rect,
        /// 是否已勾选。
        checked: bool,
        /// `[ ]` 在原文中的位置。
        source: u32,
    },
    /// 代码块的范围和内容：鼠标悬停时右上角显示复制按钮。
    ///
    /// `diagram` 为真时表示 mermaid 图的范围和源码，复制按钮左侧另显示全屏按钮。
    Code {
        /// 范围。
        rect: Rect,
        /// 代码或图的源码。
        text: Arc<str>,
        /// 是否为 mermaid 图。
        diagram: bool,
    },
    /// 表格的范围和单元格。
    ///
    /// `cols`、`rows` 是各列宽度和各行高度（从 `rect` 的左上角起），点击单元格时只在该单元格内
    /// 查找文字。可编辑的预览中光标位于该表格时，`align` 是光标所在列的对齐方式，
    /// 表格上方绘制工具栏。
    Table {
        /// 范围。
        rect: Rect,
        /// 光标所在列的对齐方式。
        align: Option<Align>,
        /// 各列的宽度。
        cols: Arc<[f32]>,
        /// 各行的高度。
        rows: Arc<[f32]>,
    },
    /// 一组线（公式的根号、括号，图的边框、连线），`rect` 是外框。
    Path {
        /// 外框。
        rect: Rect,
        /// 各段线。
        strokes: Arc<[Stroke]>,
        /// 线宽（逻辑像素）。
        width: f32,
        /// 颜色。
        color: Color,
    },
    /// 填色的凸多边形（图里的菱形、箭头）。
    Polygon {
        /// 外框。
        rect: Rect,
        /// 各顶点。
        points: Arc<[(f32, f32)]>,
        /// 颜色。
        color: Color,
    },
}

impl Item {
    /// 返回宽 `width`（逻辑像素）的线 `strokes` 组成的一项；`rect` 取所有线段的包围盒，
    /// 四周加上一半线宽。
    pub fn path(strokes: Vec<Stroke>, width: f32, color: Color) -> Item {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut grow = |x: f32, y: f32, r: f32| {
            (x0, y0, x1, y1) = (x0.min(x - r), y0.min(y - r), x1.max(x + r), y1.max(y + r));
        };
        for &stroke in &strokes {
            match stroke {
                Stroke::Line(ax, ay, bx, by) => {
                    grow(ax, ay, 0.0);
                    grow(bx, by, 0.0);
                }
                Stroke::Circle(cx, cy, r) | Stroke::Arc(cx, cy, r, ..) => grow(cx, cy, r),
            }
        }
        let half = width / 2.0;
        Item::Path {
            rect: Rect::new(x0 - half, y0 - half, x1 - x0 + width, y1 - y0 + width),
            strokes: strokes.into(),
            width,
            color,
        }
    }

    /// 返回填色的凸多边形 `points` 组成的一项；`rect` 取各点的包围盒。
    pub fn polygon(points: Vec<(f32, f32)>, color: Color) -> Item {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in &points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        Item::Polygon {
            rect: Rect::new(x0, y0, x1 - x0, y1 - y0),
            points: points.into(),
            color,
        }
    }
}

/// 排版用到的颜色，由配色推出来，换主题时跟着变。
///
/// 边框、底色都是正文色的低不透明度值：深色主题取 20%/12%/5%/3%，浅色主题取
/// 16%/8%/4%/2.5%。
#[derive(Clone, Copy)]
pub struct Colors {
    /// 正文和标题的颜色。
    pub text: Color,
    /// 次要颜色，六级标题、引用正文、提示块的说明用。
    pub muted: Color,
    /// 链接颜色，取配色的强调色。
    link: Color,
    /// 边框、分隔线、表格线的颜色。
    border: Color,
    /// 行内代码的底色。
    code_inline: Color,
    /// 代码块的底色，编辑时显示原文的代码框同样使用。
    code_block: Color,
    /// 表格表头的底色。
    table_head: Color,
    /// 表格外框的颜色：比分隔线（`border`）淡，表格不显得沉重。
    table_frame: Color,
    /// 表格行之间横线的颜色：比外框更淡。
    table_line: Color,
}

impl Colors {
    /// 由正文色 `text`、次要色 `muted`、强调色 `accent` 推出排版用的颜色；`dark` 为深色主题，
    /// 深色、浅色主题各取一组不透明度。
    pub fn new(text: Color, muted: Color, accent: Color, dark: bool) -> Colors {
        Colors {
            text,
            muted,
            link: accent,
            border: text.alpha(if dark { 0.2 } else { 0.16 }),
            code_inline: text.alpha(if dark { 0.12 } else { 0.08 }),
            code_block: text.alpha(if dark { 0.05 } else { 0.04 }),
            table_head: text.alpha(if dark { 0.06 } else { 0.04 }),
            table_frame: text.alpha(if dark { 0.14 } else { 0.11 }),
            table_line: text.alpha(if dark { 0.09 } else { 0.07 }),
        }
    }

    /// 提示块（`[!NOTE]` 这类）的竖条和标题颜色，按种类区分。
    fn alert(dark: bool, alert: Alert) -> Color {
        let (light, night) = match alert {
            Alert::Note => (0x0969da, 0x4493f8),
            Alert::Tip => (0x1a7f37, 0x3fb950),
            Alert::Important => (0x8250df, 0xab7df8),
            Alert::Warning => (0x9a6700, 0xd29922),
            Alert::Caution => (0xd1242f, 0xf85149),
        };
        Color::hex(if dark { night } else { light })
    }
}

/// 排版时的上下文。
pub struct Cx<'a, 'f> {
    /// 量字宽、画图用的绘制上下文。
    pub frame: &'a mut Frame<'f>,
    /// 正文字号（逻辑像素），标题、代码按它缩放。
    pub base: f32,
    /// 返回语法标记对应的颜色，用于代码的高亮。
    pub token: &'a dyn Fn(TokenKind) -> Color,
    /// 编辑区的底色，图的填充色由它与强调色混合得到。
    pub surface: Color,
    /// 强调色。
    pub accent: Color,
    /// 由配色推出的排版颜色。
    pub colors: Colors,
    /// 是否深色主题。
    pub dark: bool,
    /// 已读好的图片，按路径查。
    pub images: &'a HashMap<PathBuf, ImageSlot>,
    /// 返回图片地址对应的本地文件；不显示该图片时返回 `None`。
    pub image_path: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// 排版时遇到、还没读过的图片：本地路径和文件里写的地址。
    pub wanted: Vec<(PathBuf, String)>,
    /// 跨次排版留着的排版缓存。
    pub memo: &'a mut Memo,
    /// 点过、和原来开合相反的折叠块（见 `details_id`）。
    pub toggled: &'a HashSet<u64>,
    /// 正在排的块的文字（见 [`LaidBlock::text`]）和它的字符数。
    pub text: String,
    /// `text` 的字符数。
    pub chars: usize,
    /// 可编辑的预览中，正在排的顶层块里光标所在的叶子块：显示原文，见 [`Raw`]。
    pub raw: Option<Raw>,
    /// 光标所在的表格里显示原文的那一格，见 `Cx::live_cell`；排这张表时取走。
    pub cell: Option<LiveCell>,
    /// 这一块中是否排入了输入法组字（[`Raw::preedit`]）；为真时，绘制光标时不再重复绘制组字文字。
    pub preedit: bool,
}

/// 可编辑的预览中光标所在的叶子块：不排版，显示原文。
pub struct Raw {
    /// 所在顶层块现在的原文（从块的起点起，含解析之后的修改）。
    pub block: Arc<str>,
    /// 叶子块在 `block` 中的范围。
    pub leaf: std::ops::Range<usize>,
    /// 叶子块在解析结果中的终点（相对顶层块）。
    ///
    /// 块里记下的原文位置来自解析结果；输入之后、重新解析之前，`leaf` 的终点已按新增的字节
    /// 后移，其后的块仍是旧的位置。判断块是否属于该叶子块时与这个终点比较，其后的块不会落入
    /// 叶子块而被跳过。
    pub parsed_end: usize,
    /// 已经显示过：同一个叶子块里排出的其余块（HTML 块里的几段、表格的其余单元格）不再显示。
    pub done: bool,
    /// 光标在 `block` 中的位置：表格只把光标所在的单元格显示为原文。
    pub caret: usize,
    /// 叶子块的种类，决定原文的显示方式。
    pub kind: RawKind,
    /// 输入法正在组合的文字（仅限正文），排入光标所在的行，见 `reveal`。
    pub preedit: Option<Arc<str>>,
}

/// 显示原文的叶子块的种类。
#[derive(Clone, PartialEq, Eq)]
pub enum RawKind {
    /// 段落、标题：按排版显示，只显示光标接触到的标记，见 `reveal`。
    Prose,
    /// 围栏代码块、公式（`$$`）：放入代码框按语言着色，围栏行只在光标位于其上时显示；
    /// 公式和 mermaid 图的下方另外绘制渲染结果。
    Fenced(Box<str>),
    /// HTML 块：放入代码框按 HTML 着色，下方另外绘制渲染结果。
    Html,
    /// 其余块：使用等宽字体，按 Markdown 着色。
    Plain,
}

impl Raw {
    /// 光标位于 `block` 的 `caret` 处，所在的叶子块为 `leaf`，在解析结果中的终点为
    /// `parsed_end`；`prose` 表示正文。
    pub fn new(
        block: Arc<str>,
        leaf: std::ops::Range<usize>,
        parsed_end: usize,
        caret: usize,
        prose: bool,
        preedit: Option<Arc<str>>,
    ) -> Raw {
        let mut raw = Raw {
            block,
            parsed_end: parsed_end.max(leaf.start),
            leaf,
            done: false,
            caret,
            kind: RawKind::Plain,
            preedit,
        };
        raw.kind = if prose { RawKind::Prose } else { raw.guess() };
        raw
    }

    /// 按叶子块原文的第一行识别其种类（仅用于非正文的块）。
    pub fn guess(&self) -> RawKind {
        let text = self.block.get(self.leaf.clone()).unwrap_or_default();
        let first = text.split('\n').next().unwrap_or_default();
        let line = first.trim_start_matches(' ');
        if first.len() - line.len() > 3 {
            return RawKind::Plain;
        }
        if let Some(info) = ["```", "~~~"].iter().find_map(|f| line.strip_prefix(f)) {
            let info = info.trim_start_matches(['`', '~']).trim();
            let lang = info.split_whitespace().next().unwrap_or_default();
            return RawKind::Fenced(lang.into());
        }
        if line.starts_with("$$") {
            return RawKind::Fenced("latex".into());
        }
        if line.starts_with('<') {
            return RawKind::Html;
        }
        RawKind::Plain
    }
}

/// 可编辑的预览中光标所在的单元格：第几行（解析出的行，不含表头下面那一行）、第几列，和这一格
/// 两个 `|` 之间的原文在顶层块中的范围。
type LiveCell = (usize, usize, std::ops::Range<usize>);

/// [`Cx::raw_state`] 的结果：块不在光标所在的叶子块里、显示原文、或者原文已经显示过。
enum RawState {
    Laid,
    Show,
    Skip,
    /// 光标所在的段落、标题：按排版显示，只显示光标接触到的标记。
    ///
    /// 规则见 `winc_preview::markdown::reveal`。光标位于段落之后的空行上时，
    /// 这些空行（原文起点，相对顶层块）各排一行，供光标定位。
    Reveal(Box<Block>, Vec<usize>),
    /// 光标所在的分隔线，见 [`Cx::raw_rule`]。
    Rule,
}

/// `line` 是否为代码块或公式的围栏（三个以上的反引号或波浪号，或 `$$`）。
fn is_fence(line: &str) -> bool {
    let line = line.trim();
    ["```", "~~~", "$$"].iter().any(|f| line.starts_with(f))
}

/// 返回光标位于叶子块内容之后的空行上时需要排版的空行（原文起点，相对顶层块）。
///
/// 范围从内容之后的第一行到光标所在的行；在块末尾按回车后，光标位于这些行上。
fn blanks(raw: &Raw) -> Vec<usize> {
    let text = raw.block.get(raw.leaf.clone()).unwrap_or_default();
    let caret = raw.caret.saturating_sub(raw.leaf.start).min(text.len());
    let content = text.trim_end().len();
    match text[content..].find('\n').map(|n| content + n + 1) {
        Some(first) if caret >= first => std::iter::once(first)
            .chain(
                text[first..caret]
                    .match_indices('\n')
                    .map(|(n, _)| first + n + 1),
            )
            .map(|at| raw.leaf.start + at)
            .collect(),
        _ => Vec::new(),
    }
}

/// 返回光标所在的段落、标题按当前原文生成的块（见 `winc_preview::markdown::reveal`）、
/// 光标位于其后的空行上时需要排版的空行（原文起点，相对顶层块），以及组字是否已排入该块。
fn reveal(raw: &Raw) -> (Block, Vec<usize>, bool) {
    let text = raw.block.get(raw.leaf.clone()).unwrap_or_default();
    let caret = raw.caret.saturating_sub(raw.leaf.start).min(text.len());
    let content = text.trim_end().len();
    let blanks = blanks(raw);
    let at = (caret <= content).then_some(caret);
    let preedit = raw.preedit.as_deref().filter(|_| at.is_some());
    let mut block = winc_preview::markdown::reveal::reveal(&text[..content], at, preedit);
    if let Block::Paragraph(inline) | Block::Heading(_, inline) = &mut block {
        inline.source = raw.leaf.start;
    }
    (block, blanks, preedit.is_some())
}

/// 块里文字继承的样式：颜色和嵌套深度。
#[derive(Clone, Copy)]
pub struct Env {
    /// 文字颜色，标题、链接会另外覆盖。
    pub color: Color,
    /// 嵌套深度（列表、引用各算一层），决定项目符号的形状和块引用里外边距是否紧凑。
    pub depth: usize,
}

/// 一段行内文字的样式。
#[derive(Clone, Copy)]
struct Para {
    /// 字号（逻辑像素）。
    size: f32,
    /// 是否加粗（行内代码不加粗）。
    bold: bool,
    color: Color,
    /// 行高与字号之比。
    line: f32,
    align: Align,
}

/// 代码块里一行的排法。
#[derive(Clone, Copy)]
struct Row {
    /// 一行最宽多少（逻辑像素），再宽就折。
    room: f32,
    /// 字号（逻辑像素）。
    size: f32,
    /// 行高（逻辑像素）。
    line_h: f32,
    /// 基线以上的高度（逻辑像素）。
    ascent: f32,
    /// 基线以下的深度（逻辑像素）。
    descent: f32,
}

/// 排一段行内文字的结果。
#[derive(Clone, Copy)]
pub struct Flow {
    /// 占用的高度（逻辑像素）。
    pub height: f32,
    /// 第一行的基线（逻辑像素，相对 `y`）；没有文字时为 `None`。
    first_baseline: Option<f32>,
}

impl Cx<'_, '_> {
    /// 排一串块，相邻两块的间距取两者外边距的较大值（上下外边距合并），第一块上面、最后
    /// 一块下面不留空。
    fn blocks(
        &mut self,
        blocks: &[Block],
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let mut top = y;
        let mut first_baseline = None;
        let mut below: Option<f32> = None;
        for block in blocks {
            let (margin_top, margin_bottom) = margins(block, self.base, env.depth > 0);
            if let Some(below) = below {
                top += below.max(margin_top);
            }
            let laid = self.block(block, x, top, width, env, out);
            if first_baseline.is_none() {
                first_baseline = laid.first_baseline;
            }
            top += laid.height;
            below = Some(margin_bottom);
        }
        Flow {
            height: top - y,
            first_baseline,
        }
    }

    /// 排一个块，左上角在 `(x, y)`、宽 `width`（逻辑像素），要画的项追加到 `out`。
    ///
    /// 返回块的高度和第一行的基线（相对 `y`）。
    pub fn block(
        &mut self,
        block: &Block,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let base = self.base;
        match self.raw_state(block) {
            RawState::Laid => {}
            RawState::Show => return self.raw_with_preview(block, x, y, width, env, out),
            RawState::Reveal(revealed, blanks) => {
                let raw = self.raw.take();
                let mut flow = self.block(&revealed, x, y, width, env, out);
                self.raw = raw;
                flow.height += self.blank_lines(&blanks, x, y + flow.height, width, out);
                return flow;
            }
            RawState::Skip => {
                return Flow {
                    height: 0.0,
                    first_baseline: None,
                };
            }
            RawState::Rule => return self.raw_rule(x, y, width, out),
        }
        let para = self.body_para(env);
        match block {
            Block::Paragraph(inline) => self.flow(inline, x, y, width, para, &mut out.items),
            Block::Heading(level, inline) => self.heading(*level, inline, x, y, width, env, out),
            Block::Code { lang, text, map } => {
                let diagram = (lang == "mermaid")
                    .then(|| self.diagram(text, x, y, width, env, &mut out.items))
                    .flatten();
                diagram.unwrap_or_else(|| self.code(lang, text, map, x, y, width, &mut out.items))
            }
            Block::Quote(alert, blocks) => self.quote(*alert, blocks, x, y, width, env, out),
            Block::List(start, items) => self.list(*start, items, x, y, width, env, out),
            Block::Table(aligns, rows, head) => self.table(
                aligns,
                rows,
                *head,
                x,
                y,
                width,
                env,
                &mut out.items,
                &mut out.scrolls,
            ),
            Block::Math(tex, source) => self.math_block(tex, *source, x, y, width, env, out),
            Block::Rule(at) => {
                let h = rule_height(base);
                out.items.push(Item::Fill {
                    rect: Rect::new(x, y, width, h),
                    color: self.colors.border,
                    radius: 0.0,
                });
                // 分隔线上没有文字：点击分隔线及其上下各半个外边距时，光标移到它的原文处。
                if let Some(at) = at.and_then(|a| u32::try_from(a).ok()) {
                    let reach = base * 0.75;
                    out.items.push(Item::Spot {
                        rect: Rect::new(x, y - reach, width, h + reach * 2.0),
                        at: (self.chars, at),
                    });
                }
                Flow {
                    height: h,
                    first_baseline: None,
                }
            }
            Block::Footnote(label, blocks) => {
                let env = Env {
                    color: self.colors.muted,
                    ..env
                };
                let indent = base * 2.0;
                // 正文里的编号链到这里（见 `winc_preview::markdown` 的脚注引用）。
                out.anchors.push((format!("fn-{label}"), y));
                let flow = self.blocks(blocks, x + indent, y, width - indent, env, out);
                let marker = format!("{label}.");
                self.marker(
                    &marker,
                    x + indent,
                    y,
                    flow.first_baseline,
                    env.color,
                    &mut out.items,
                );
                flow
            }
            Block::Details {
                open,
                summary,
                blocks,
            } => self.details(
                details_id(block),
                *open,
                summary,
                blocks,
                x,
                y,
                width,
                env,
                out,
            ),
        }
    }

    /// 块文字（[`Cx::text`]）里一段（段落、单元格、代码的一行）结束：接一个换行。
    pub fn end_paragraph(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
            self.chars += 1;
        }
    }

    /// 在内容左边 `content_x` 前面画列表的记号（右对齐），基线和内容第一行对齐。
    fn marker(
        &mut self,
        marker: &str,
        content_x: f32,
        y: f32,
        baseline: Option<f32>,
        color: Color,
        out: &mut Vec<Item>,
    ) {
        let size = self.base;
        let baseline = baseline.unwrap_or_else(|| {
            let m = self.frame.metrics(FontKind::Ui, size);
            y + (size * 1.5 - m.ascent - m.descent) / 2.0 + m.ascent
        });
        let w = self.frame.measure(FontKind::Ui, size, marker);
        out.push(Item::Text {
            x: content_x - size * 0.4 - w,
            baseline,
            text: marker.into(),
            font: FontKind::Ui,
            faux: Faux::default(),
            size,
            color,
            span: None,
        });
    }

    /// 画无序列表项的记号：第一层实心圆点，第二层空心圆圈，更深的层实心方块。
    ///
    /// 记号画成图形而不是文字：符号字形的大小、位置随字体变化，中文字体中的 `•` 常常偏小、
    /// 偏低。记号的中心与该项第一行文字的中部对齐，位于缩进的中间偏右。
    fn bullet(
        &mut self,
        content_x: f32,
        top: f32,
        baseline: Option<f32>,
        env: Env,
        out: &mut Vec<Item>,
    ) {
        let base = self.base;
        let mid = baseline.map_or(top + base * 0.75, |b| b - base * 0.35);
        let cx = content_x - base * 0.75;
        let color = env.color;
        match env.depth {
            0 => {
                let r = base * 0.17;
                out.push(Item::Fill {
                    rect: Rect::new(cx - r, mid - r, r * 2.0, r * 2.0),
                    color,
                    radius: r,
                });
            }
            1 => {
                let r = base * 0.16;
                out.push(Item::path(
                    vec![Stroke::Circle(cx, mid, r)],
                    (base * 0.08).max(1.0),
                    color,
                ));
            }
            _ => {
                let side = base * 0.3;
                out.push(Item::Fill {
                    rect: Rect::new(cx - side / 2.0, mid - side / 2.0, side, side),
                    color,
                    radius: 1.0,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn list(
        &mut self,
        start: Option<u64>,
        items: &[ListItem],
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let base = self.base;
        let indent = base * 2.0;
        let content_x = x + indent;
        let room = (width - indent).max(1.0);
        let inner = Env {
            depth: env.depth + 1,
            ..env
        };
        let mut top = y;
        let mut first_baseline = None;
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                top += base * LIST_GAP;
            }
            // 记号之后没有空格的空项：记号按文字排在列表的左边缘，不画圆点和序号。
            let (item_x, item_room) = if item.bare {
                (x, width)
            } else {
                (content_x, room)
            };
            let flow = self.blocks(&item.blocks, item_x, top, item_room, inner, out);
            first_baseline = first_baseline.or(flow.first_baseline);
            let baseline = flow.first_baseline;
            match (item.task, start) {
                _ if item.bare => {}
                (Some(checked), _) => {
                    let size = (base * 0.85).round();
                    let line_mid = baseline.map_or(top + base * 0.75, |b| b - base * 0.35);
                    out.items.push(Item::Check {
                        rect: Rect::new(
                            content_x - size - base * 0.45,
                            line_mid - size / 2.0,
                            size,
                            size,
                        ),
                        checked,
                        source: u32::try_from(item.task_source).unwrap_or(u32::MAX),
                    });
                }
                (None, Some(start)) => {
                    let marker = format!("{}.", start + i as u64);
                    self.marker(&marker, content_x, top, baseline, env.color, &mut out.items);
                }
                (None, None) => self.bullet(content_x, top, baseline, env, &mut out.items),
            }
            // 空的一项也占一行。
            top += flow.height.max(if item.blocks.is_empty() {
                base * 1.5
            } else {
                0.0
            });
        }
        Flow {
            height: top - y,
            first_baseline,
        }
    }

    /// 为光标所在的空行排一行，返回排出的总高度；`blanks` 是段落之后直到光标所在行的各空行
    /// （原文起点，相对顶层块）。
    ///
    /// 光标所在的行高度等于正文的行高，之前的空行合计占一个段落间距：该行的位置与在此输入
    /// 文字后新段落的位置相同，输入第一个字时内容不移动。
    fn blank_lines(&mut self, blanks: &[usize], x: f32, y: f32, width: f32, out: &mut Out) -> f32 {
        let size = self.base;
        let m = self.frame.metrics(FontKind::Ui, size);
        let row = Row {
            room: width.max(size),
            size,
            line_h: (size * 1.5).round(),
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let Some((&source, before)) = blanks.split_last() else {
            return 0.0;
        };
        let gap = if before.is_empty() { 0.0 } else { size };
        let source = u32::try_from(source).unwrap_or(0);
        let piece = self.code_piece("", source, "markdown", language, &mut state, row);
        self.paste(&piece, x, y + gap, source, &mut out.items);
        gap + piece.height
    }
}

/// 收集一个块里要画的项和标题锚点。
#[derive(Default)]
pub struct Out {
    /// 要画的项，按绘制先后排列。
    pub items: Vec<Item>,
    /// 标题锚点和所在高度（逻辑像素，相对块的左上角）。
    pub anchors: Vec<(String, f32)>,
    /// 比正文宽、可以横向滚动的表格。
    pub scrolls: Vec<Scroller>,
}

/// 列表相邻两项之间的间距与正文字号之比。
const LIST_GAP: f32 = 0.25;

/// 分隔线的粗细（逻辑像素）：正文字号的四分之一。
fn rule_height(base: f32) -> f32 {
    (base * 0.25).round()
}

/// 块上下的外边距（逻辑像素）。
///
/// `tight` 为真时在列表项里：段落之间不额外留空，嵌套的列表之前只留出列表项之间的间距。
pub fn margins(block: &Block, base: f32, tight: bool) -> (f32, f32) {
    match block {
        // 项里的文字之后接嵌套的列表时，间距与相邻两项之间相同（见 [`LIST_GAP`]）。
        Block::List(..) if tight => (base * LIST_GAP, 0.0),
        Block::Paragraph(_) if tight => (0.0, 0.0),
        Block::Heading(..) => (base * 1.5, base),
        Block::Rule(_) => (base * 1.5, base * 1.5),
        Block::Footnote(..) => (0.0, base * 0.5),
        _ => (0.0, base),
    }
}

/// 折叠块标题的「链接」地址的开头，后面接 `details_id`：点了不是打开链接，是展开、收起。
pub const DETAILS: &str = "\u{1}details:";

/// 折叠块的标识：内容的哈希。
///
/// 内容相同的两个折叠块会一起开合；这种情况少见，未单独处理。
fn details_id(block: &Block) -> u64 {
    memo::key(|h| block.hash(h))
}

pub(crate) use winc_preview::markdown::slug;

#[cfg(test)]
mod tests;
