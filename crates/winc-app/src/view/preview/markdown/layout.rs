//! Markdown 预览的排版：把解析好的块排成一项项待绘制的内容（文字、底色、图片、链接范围）。
//!
//! [`LaidBlock`] 是排好的一块，[`Item`] 是其中一项，[`Cx`] 带着排版上下文逐块排；行内文字、
//! 公式、流程图、饼图和时序图在子模块里。坐标都是逻辑像素，相对顶层块的左上角；字号、留白
//! 等版式常量在上一级模块。

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::Arc;

use winc_preview::markdown::{Alert, Align, Block, Inline, Item as ListItem, SourceMap, Style};
use winc_render::{Color, Faux, FontKind, Frame, Rect, Stroke};

use crate::app::preview::markdown::ImageSlot;
use crate::theme::Palette;

mod diagram;
mod inline;
mod math;
mod memo;

pub(crate) use memo::Memo;
pub(super) use memo::{key as memo_key, shift};

/// 一个排好版的顶层块：要画的项，坐标相对块的左上角。
#[derive(Default)]
pub(super) struct LaidBlock {
    /// 块的高度（逻辑像素）。
    pub(super) height: f32,
    /// 要画的项，按绘制先后排列。
    pub(super) items: Vec<Item>,
    /// 标题的锚点（小写、空格换成 `-`、去掉标点）和所在高度，`[跳转](#标题)` 用。
    pub(super) anchors: Vec<(String, f32)>,
    /// 块里能选中的文字连起来：段落、单元格、代码行之间是换行，折行处不加。
    ///
    /// 选中、复制、查找都按它，位置是字符偏移。
    pub(super) text: Box<str>,
    /// `text` 有多少个字符。
    pub(super) chars: usize,
    /// 按高度找看得见的项用（见 [`LaidBlock::visible`]）。
    ///
    /// `tall` 是高的项（代码块的底色、引用的竖条、表格的竖线、大图）的序号，`short` 是矮的项
    /// 按上边缘排好的（上边缘，序号）。
    tall: Vec<u32>,
    short: Vec<(f32, u32)>,
    /// 排版这一块时使用的排版缓存条目，见 [`Memo::sweep`]。
    pub(super) memo: Box<[u64]>,
    /// 输入法组字是否排在这一块中，见 [`Cx::preedit`]。
    pub(super) preedit: bool,
    /// 比正文宽、可以横向滚动的表格，按项的先后排列。
    pub(super) scrolls: Vec<Scroller>,
}

/// 比正文宽的表格：表格的各项按自然宽度排版，绘制时横向平移、裁剪到正文一栏内。
#[derive(Debug, Clone)]
pub(super) struct Scroller {
    /// 属于该表格的项（[`LaidBlock::items`] 的序号）。
    pub(super) items: std::ops::Range<u32>,
    /// 显示表格的区域（块里的坐标）：正文一栏宽，高度含表格下方的滚动条。
    pub(super) rect: Rect,
    /// 表格的总宽度（逻辑像素）。
    pub(super) content: f32,
}

impl Scroller {
    /// 最大横向滚动距离（逻辑像素）。
    pub(super) fn max(&self) -> f32 {
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
pub(super) const SCROLL_BAR_ROOM: f32 = 10.0;

/// 高项与矮项的分界（逻辑像素，经验值）：高度超过该值的项算「高的」，每次绘制都检查一遍；
/// 矮的项按上边缘二分查找。
const TALL: f32 = 120.0;

impl LaidBlock {
    /// 创建一个排好的顶层块，同时按高度建好查找可见项的索引。
    pub(super) fn new(
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
    pub(super) fn scroller_of(&self, item: usize) -> Option<usize> {
        let item = u32::try_from(item).ok()?;
        self.scrolls.iter().position(|s| s.items.contains(&item))
    }

    /// 块里有没有用到 `paths` 里的图片（排版时还在读的也算）。
    pub(super) fn uses_any(&self, paths: &[PathBuf]) -> bool {
        !paths.is_empty()
            && self
                .items
                .iter()
                .any(|item| matches!(item, Item::Image { path, .. } if paths.contains(path)))
    }

    /// 块里有没有指向 `url` 的链接。
    pub(super) fn links_to(&self, url: &str) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, Item::Link { url: u, .. } if &**u == url))
    }

    /// 和 `y0..y1`（块里的坐标）有交集的项的序号，按原来的先后（先画的底色在下面）。
    ///
    /// 长列表、长表格、长代码块是一个顶层块，里面成千上万项，每帧只看屏上那些。
    pub(super) fn visible(&self, y0: f32, y1: f32, out: &mut Vec<u32>) {
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
    pub(super) fn span_y(&self) -> (f32, f32) {
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
    pub(super) fn span_x(&self) -> (f32, f32) {
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
pub(super) struct Span {
    /// 这一段在块文字里的起点（字符偏移）。
    pub(super) start: usize,
    /// 每个字的宽度（逻辑像素）。
    pub(super) advances: Arc<[f32]>,
    /// 第一个字在原文中的位置（字节偏移，相对所在顶层块的起点）；由原文生成的文字（提示块的
    /// 标题、图里的字）为 `None`。
    pub(super) source: Option<u32>,
    /// 原文不连续之处：（第几个字，该字相对 `source` 的字节偏移），按字的先后排列。
    ///
    /// 其余的字在原文中紧接前一个字。
    pub(super) jumps: Option<Arc<[(u32, u32)]>>,
}

impl Span {
    /// 第 `k` 个字（`k` 可以等于字数，即末尾）在原文中的位置，`text` 是这一段的文字。
    pub(super) fn source_of(&self, text: &str, k: usize) -> Option<usize> {
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
    pub(super) fn chars_before(&self, text: &str, source: usize) -> usize {
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
    pub(super) fn char_at_source(&self, text: &str, source: usize) -> Option<usize> {
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
pub(super) enum Item {
    Text {
        /// 相对顶层块左上角的横坐标。
        x: f32,
        /// 基线相对顶层块左上角的纵坐标。
        baseline: f32,
        text: Arc<str>,
        font: FontKind,
        faux: Faux,
        size: f32,
        color: Color,
        /// 能选中时在块文字里的位置；列表记号这类不算文字的为 `None`。
        span: Option<Span>,
    },
    Fill {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    /// 四个角半径各不相同的底色，`radii` 依次是左上、右上、右下、左下（逻辑像素）。
    Corners {
        rect: Rect,
        color: Color,
        radii: [f32; 4],
    },
    Image {
        rect: Rect,
        path: PathBuf,
        /// 图片在块文字中的字符序号及其在原文中的位置（相对顶层块）。
        ///
        /// 点击图片时光标移到该位置；图片在块文字中不占字符。
        at: Option<(usize, u32)>,
    },
    /// 链接能点的范围，不画。
    Link { rect: Rect, url: Arc<str> },
    /// 点击时光标移到原文 `at` 处的范围，不画：单独成行的公式中没有可定位的文字。
    ///
    /// `at` 同 [`Item::Image`] 的 `at`：块文字中的字符序号和原文中的位置（相对顶层块）。
    Spot { rect: Rect, at: (usize, u32) },
    /// 任务列表的勾选框：`source` 是 `[ ]` 在原文中的位置（相对顶层块的开头），点击时切换。
    Check {
        rect: Rect,
        checked: bool,
        source: u32,
    },
    /// 代码块的范围和内容：鼠标悬停时右上角显示复制按钮。
    ///
    /// `diagram` 为真时表示 mermaid 图的范围和源码，复制按钮左侧另显示全屏按钮。
    Code {
        rect: Rect,
        text: Arc<str>,
        diagram: bool,
    },
    /// 表格的范围和单元格。
    ///
    /// `cols`、`rows` 是各列宽度和各行高度（从 `rect` 的左上角起），点击单元格时只在该单元格内
    /// 查找文字。可编辑的预览中光标位于该表格时，`align` 是光标所在列的对齐方式，
    /// 表格上方绘制工具栏。
    Table {
        rect: Rect,
        align: Option<Align>,
        cols: Arc<[f32]>,
        rows: Arc<[f32]>,
    },
    /// 一组线（公式的根号、括号，图的边框、连线），`rect` 是外框。
    Path {
        rect: Rect,
        strokes: Arc<[Stroke]>,
        width: f32,
        color: Color,
    },
    /// 填色的凸多边形（图里的菱形、箭头）。
    Polygon {
        rect: Rect,
        points: Arc<[(f32, f32)]>,
        color: Color,
    },
}

impl Item {
    /// 返回宽 `width`（逻辑像素）的线 `strokes` 组成的一项；`rect` 取所有线段的包围盒，
    /// 四周加上一半线宽。
    pub(super) fn path(strokes: Vec<Stroke>, width: f32, color: Color) -> Item {
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
    pub(super) fn polygon(points: Vec<(f32, f32)>, color: Color) -> Item {
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
pub(super) struct Colors {
    /// 正文和标题的颜色。
    pub(super) text: Color,
    /// 次要颜色，六级标题、引用正文、提示块的说明用。
    pub(super) muted: Color,
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
    /// 从配色推出排版用的颜色，深色、浅色主题各取一组不透明度。
    pub(super) fn new(p: &Palette) -> Colors {
        let dark = p.dark;
        Colors {
            text: p.text,
            muted: p.text_muted,
            link: p.accent,
            border: p.text.alpha(if dark { 0.2 } else { 0.16 }),
            code_inline: p.text.alpha(if dark { 0.12 } else { 0.08 }),
            code_block: p.text.alpha(if dark { 0.05 } else { 0.04 }),
            table_head: p.text.alpha(if dark { 0.06 } else { 0.04 }),
            table_frame: p.text.alpha(if dark { 0.14 } else { 0.11 }),
            table_line: p.text.alpha(if dark { 0.09 } else { 0.07 }),
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
pub(super) struct Cx<'a, 'f> {
    /// 量字宽、画图用的绘制上下文。
    pub(super) frame: &'a mut Frame<'f>,
    /// 正文字号（逻辑像素），标题、代码按它缩放。
    pub(super) base: f32,
    /// 当前配色，代码高亮的颜色按它取。
    pub(super) palette: &'static Palette,
    /// 由配色推出的排版颜色。
    pub(super) colors: Colors,
    /// 是否深色主题。
    pub(super) dark: bool,
    /// 已读好的图片，按路径查。
    pub(super) images: &'a HashMap<PathBuf, ImageSlot>,
    /// 当前文档的路径，图片的相对路径按它算。
    pub(super) doc: Option<PathBuf>,
    /// 排版时遇到、还没读过的图片：本地路径和文件里写的地址。
    pub(super) wanted: Vec<(PathBuf, String)>,
    /// 网址的图片要不要联网下载（设置里可以关）。
    pub(super) remote: bool,
    /// 跨次排版留着的排版缓存。
    pub(super) memo: &'a mut Memo,
    /// 点过、和原来开合相反的折叠块（见 [`details_id`]）。
    pub(super) toggled: &'a HashSet<u64>,
    /// 正在排的块的文字（见 [`LaidBlock::text`]）和它的字符数。
    pub(super) text: String,
    /// `text` 的字符数。
    pub(super) chars: usize,
    /// 可编辑的预览中，正在排的顶层块里光标所在的叶子块：显示原文，见 [`Raw`]。
    pub(super) raw: Option<Raw>,
    /// 光标所在的表格里显示原文的那一格，见 [`Cx::live_cell`]；排这张表时取走。
    pub(super) cell: Option<LiveCell>,
    /// 这一块中是否排入了输入法组字（[`Raw::preedit`]）；为真时，绘制光标时不再重复绘制组字文字。
    pub(super) preedit: bool,
}

/// 可编辑的预览中光标所在的叶子块：不排版，显示原文。
pub(super) struct Raw {
    /// 所在顶层块现在的原文（从块的起点起，含解析之后的修改）。
    pub(super) block: Arc<str>,
    /// 叶子块在 `block` 中的范围。
    pub(super) leaf: std::ops::Range<usize>,
    /// 叶子块在解析结果中的终点（相对顶层块）。
    ///
    /// 块里记下的原文位置来自解析结果；输入之后、重新解析之前，`leaf` 的终点已按新增的字节
    /// 后移，其后的块仍是旧的位置。判断块是否属于该叶子块时与这个终点比较，其后的块不会落入
    /// 叶子块而被跳过。
    pub(super) parsed_end: usize,
    /// 已经显示过：同一个叶子块里排出的其余块（HTML 块里的几段、表格的其余单元格）不再显示。
    pub(super) done: bool,
    /// 光标在 `block` 中的位置：表格只把光标所在的单元格显示为原文。
    pub(super) caret: usize,
    /// 叶子块的种类，决定原文的显示方式。
    pub(super) kind: RawKind,
    /// 输入法正在组合的文字（仅限正文），排入光标所在的行，见 `reveal`。
    pub(super) preedit: Option<Arc<str>>,
}

/// 显示原文的叶子块的种类。
#[derive(Clone, PartialEq, Eq)]
pub(super) enum RawKind {
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
    pub(super) fn new(
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
    pub(super) fn guess(&self) -> RawKind {
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

/// 返回光标所在的段落、标题按当前原文生成的块（见 `winc_preview::markdown::reveal`），
/// 以及光标位于其后的空行上时需要排版的空行（原文起点，相对顶层块）。
fn reveal(raw: &Raw) -> (Block, Vec<usize>) {
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
    (block, blanks)
}

/// 块里文字继承的样式：颜色和嵌套深度。
#[derive(Clone, Copy)]
pub(super) struct Env {
    /// 文字颜色，标题、链接会另外覆盖。
    pub(super) color: Color,
    /// 嵌套深度（列表、引用各算一层），决定项目符号的形状和块引用里外边距是否紧凑。
    pub(super) depth: usize,
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
pub(super) struct Flow {
    /// 占用的高度（逻辑像素）。
    pub(super) height: f32,
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
    pub(super) fn block(
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
        let para = Para {
            size: base,
            bold: false,
            color: env.color,
            line: 1.5,
            align: Align::None,
        };
        match block {
            Block::Paragraph(inline) => self.flow(inline, x, y, width, para, &mut out.items),
            Block::Heading(level, inline) => {
                let scale = [2.0, 1.5, 1.25, 1.0, 0.875, 0.85][(*level as usize).clamp(1, 6) - 1];
                let size = base * scale;
                let color = if *level == 6 {
                    self.colors.muted
                } else {
                    env.color
                };
                let para = Para {
                    size,
                    bold: true,
                    color,
                    line: 1.25,
                    align: Align::None,
                };
                out.anchors.push((slug(&inline.text), y));
                let mut flow = self.flow(inline, x, y, width, para, &mut out.items);
                if *level <= 2 {
                    let line_y = y + flow.height + size * 0.3;
                    out.items.push(Item::Fill {
                        rect: Rect::new(x, line_y, width, 1.0),
                        color: self.colors.border,
                        radius: 0.0,
                    });
                    flow.height += size * 0.3 + 1.0;
                }
                flow
            }
            Block::Code { lang, text, map } => {
                let diagram = (lang == "mermaid")
                    .then(|| self.diagram(text, x, y, width, env, &mut out.items))
                    .flatten();
                diagram.unwrap_or_else(|| self.code(lang, text, map, x, y, width, &mut out.items))
            }
            Block::Quote(alert, blocks) => {
                let bar = (base * 0.25).round();
                let inner = x + bar + base;
                let room = (width - bar - base).max(1.0);
                let (bar_color, env) = match alert {
                    Some(alert) => (Colors::alert(self.dark, *alert), env),
                    None => (
                        self.colors.border,
                        Env {
                            color: self.colors.muted,
                            ..env
                        },
                    ),
                };
                let mut top = y;
                if let Some(alert) = alert {
                    let title = match alert {
                        Alert::Note => "Note",
                        Alert::Tip => "Tip",
                        Alert::Important => "Important",
                        Alert::Warning => "Warning",
                        Alert::Caution => "Caution",
                    };
                    let inline = Inline {
                        text: title.to_owned(),
                        runs: vec![winc_preview::markdown::Run {
                            end: title.len(),
                            style: Style::default(),
                        }],
                        ..Inline::default()
                    };
                    let para = Para {
                        bold: true,
                        color: bar_color,
                        ..para
                    };
                    let flow = self.flow(&inline, inner, top, room, para, &mut out.items);
                    top += flow.height + base * 0.25;
                }
                let flow = self.blocks(blocks, inner, top, room, env, out);
                let height = top + flow.height - y;
                out.items.push(Item::Fill {
                    rect: Rect::new(x, y, bar, height),
                    color: bar_color,
                    radius: 0.0,
                });
                Flow {
                    height,
                    first_baseline: flow.first_baseline,
                }
            }
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
            Block::Math(tex, source) => {
                let mut b = self.math(tex, base, true, env.color);
                // 比一行宽的缩小（最多缩到一半），还宽就让它超出去。
                if b.w > width {
                    let size = base * (width / b.w).max(0.5);
                    b = self.math(tex, size, true, env.color);
                }
                let pad = base * 0.5;
                let (dx, baseline) = (x + ((width - b.w) / 2.0).max(0.0), y + pad + b.asc);
                out.items
                    .extend(b.items.iter().map(|item| item.moved(dx, baseline, 0)));
                // 点击公式时光标移到公式的原文处，进入编辑。
                let height = pad * 2.0 + b.asc + b.desc;
                out.items.push(Item::Spot {
                    rect: Rect::new(x, y, width, height),
                    at: (self.chars, u32::try_from(*source).unwrap_or(0)),
                });
                // 复制、查找时是公式的原文。
                self.text.push_str(tex);
                self.chars += tex.chars().count();
                self.end_paragraph();
                Flow {
                    height,
                    first_baseline: Some(baseline),
                }
            }
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
            } => {
                let id = details_id(block);
                let open = *open != self.toggled.contains(&id);
                let indent = (base * 1.25).round();
                let flow = self.flow(summary, x + indent, y, width - indent, para, &mut out.items);
                let arrow = if open { "▾" } else { "▸" };
                self.marker(
                    arrow,
                    x + indent,
                    y,
                    flow.first_baseline,
                    env.color,
                    &mut out.items,
                );
                // 整行都能点；排在标题的链接后面，标题里的链接先认。
                out.items.push(Item::Link {
                    rect: Rect::new(x, y, width, flow.height),
                    url: format!("{DETAILS}{id:x}").into(),
                });
                if !open {
                    return flow;
                }
                let top = y + flow.height + base * 0.5;
                let inner = self.blocks(blocks, x, top, width, env, out);
                Flow {
                    height: top + inner.height - y,
                    first_baseline: flow.first_baseline,
                }
            }
        }
    }

    /// 块文字（[`Cx::text`]）里一段（段落、单元格、代码的一行）结束：接一个换行。
    pub(super) fn end_paragraph(&mut self) {
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

    /// `block` 是否在光标所在的叶子块里（按块里第一处有原文对照的位置判断）。
    fn raw_state(&mut self, block: &Block) -> RawState {
        let Some(raw) = &mut self.raw else {
            return RawState::Laid;
        };
        let start = match block {
            Block::Paragraph(inline) | Block::Heading(_, inline) => {
                (!inline.map.is_empty()).then_some(inline.source)
            }
            Block::Code { map, .. } => map.source(0),
            Block::Rule(at) => *at,
            Block::Table(_, rows, _) => rows
                .iter()
                .flatten()
                .find(|cell| !cell.map.is_empty())
                .map(|cell| cell.source),
            _ => None,
        };
        match start {
            Some(start) if raw.leaf.start <= start && start <= raw.parsed_end => {
                if std::mem::replace(&mut raw.done, true) {
                    // HTML 块生成的其余各块排在代码框下方，作为该块的渲染结果。
                    return match raw.kind {
                        RawKind::Html => RawState::Laid,
                        _ => RawState::Skip,
                    };
                }
                // 表格照常排，只有光标所在的那一格显示原文。
                if let Block::Table(_, rows, _) = block
                    && let Some(cell) = self.live_cell(rows)
                {
                    self.cell = Some(cell);
                    return RawState::Laid;
                }
                if matches!(block, Block::Paragraph(_) | Block::Heading(..))
                    && let Some(raw) = self.raw.as_ref().filter(|r| r.kind == RawKind::Prose)
                {
                    let (revealed, blanks) = reveal(raw);
                    self.preedit |= raw.preedit.is_some();
                    return RawState::Reveal(Box::new(revealed), blanks);
                }
                if matches!(block, Block::Rule(_)) {
                    return RawState::Rule;
                }
                RawState::Show
            }
            _ => RawState::Laid,
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

    /// 返回光标所在的表格（`rows` 是解析出的各行）里光标所在的单元格。
    ///
    /// 按现在的原文切分（见 `winc_preview::markdown::cells`）；光标在表头下面那一行上、原文的
    /// 行数或列数与解析结果对不上（加了一行、一列还没解析）时为 `None`，整张表显示原文。
    fn live_cell(&self, rows: &[Vec<Inline>]) -> Option<LiveCell> {
        use winc_preview::markdown::cells;
        let raw = self.raw.as_ref()?;
        let text = raw.block.get(raw.leaf.clone())?;
        let mut start = raw.leaf.start;
        let mut row = 0;
        let mut found = None;
        for line in text.split('\n') {
            let end = start + line.len();
            let here = (start..=end).contains(&raw.caret);
            if cells::is_delimiter(line) {
                if here {
                    return None;
                }
            } else if !line.trim().is_empty() {
                if here {
                    let cells = cells::cells(line);
                    let col = cells
                        .iter()
                        .position(|c| raw.caret - start <= c.segment.end)?;
                    let segment = &cells[col].segment;
                    found = Some((row, col, start + segment.start..start + segment.end));
                }
                row += 1;
            }
            start = end + 1;
        }
        let (r, c, range) = found?;
        (row == rows.len() && c < rows[r].len()).then_some((r, c, range))
    }

    /// 排版光标所在叶子块的原文。
    ///
    /// 公式、mermaid 图和 HTML 的下方另外绘制渲染结果，编辑时可以直接查看效果。
    /// 渲染结果来自最近一次的解析结果，可能比原文滞后一个解析周期。
    pub(super) fn raw_with_preview(
        &mut self,
        block: &Block,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let flow = self.raw(x, y, width, &mut out.items);
        let boxed = self
            .raw
            .as_ref()
            .is_some_and(|r| matches!(r.kind, RawKind::Fenced(_) | RawKind::Html));
        let gap = (self.base * 0.75).round();
        // 代码框自带底色，与下方的渲染结果之间不再绘制分隔线。
        let below = if boxed {
            y + flow.height + gap
        } else {
            y + flow.height + gap * 2.0 + 1.0
        };
        let html = self.raw.as_ref().is_some_and(|r| r.kind == RawKind::Html);
        let preview = match block {
            Block::Math(..) => self.preview(block, x, below, width, env, out),
            Block::Code { lang, text, .. } if lang == "mermaid" => {
                self.diagram(text, x, below, width, env, &mut out.items)
            }
            _ if html => self.preview(block, x, below, width, env, out),
            _ => None,
        };
        let Some(preview) = preview.filter(|p| p.height > 0.0) else {
            return flow;
        };
        if !boxed {
            // 原文与渲染结果之间绘制一条细分隔线。
            out.items.push(Item::Fill {
                rect: Rect::new(x, y + flow.height + gap, width, 1.0),
                color: self.colors.border,
                radius: 0.0,
            });
        }
        Flow {
            height: below + preview.height - y,
            first_baseline: flow.first_baseline,
        }
    }

    /// 在 `(x, y)` 处按排版显示 `block`（不显示原文），作为原文下方的渲染结果。
    fn preview(
        &mut self,
        block: &Block,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Option<Flow> {
        let raw = self.raw.take();
        let flow = self.block(block, x, y, width, env, out);
        self.raw = raw;
        Some(flow)
    }

    /// 排版光标所在叶子块的原文（见 [`Raw`]）。
    ///
    /// 代码、公式、HTML 放入代码框（见 [`Cx::raw_box`]）；其余块使用等宽字体并按 Markdown 着色，
    /// 保留原有换行，过长时在字符之间折行。每段文字的原文位置即其在原文中的位置。
    pub(super) fn raw(&mut self, x: f32, y: f32, width: f32, out: &mut Vec<Item>) -> Flow {
        let Some(raw) = &self.raw else {
            return Flow {
                height: 0.0,
                first_baseline: None,
            };
        };
        match &raw.kind {
            RawKind::Fenced(lang) => {
                let lang = lang.clone();
                return self.raw_box(&lang, x, y, width, out);
            }
            RawKind::Html => return self.raw_box("html", x, y, width, out),
            RawKind::Prose | RawKind::Plain => {}
        }
        let (block, leaf) = (Arc::clone(&raw.block), raw.leaf.clone());
        let text = block.get(leaf.clone()).unwrap_or_default();
        let size = self.base * 0.85;
        let line_h = (size * 1.45).round();
        let m = self.frame.metrics(FontKind::Mono, size);
        let row = Row {
            room: width.max(size),
            size,
            line_h,
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let (mut top, mut first_baseline) = (y, None);
        let mut start = leaf.start;
        for line in text.split('\n') {
            let source = u32::try_from(start).unwrap_or(0);
            start += line.len() + 1;
            let piece = self.code_piece(line, source, "markdown", language, &mut state, row);
            first_baseline.get_or_insert(top + piece.first_baseline.unwrap_or(0.0));
            self.paste(&piece, x, top, source, out);
            top += piece.height;
        }
        Flow {
            height: top - y,
            first_baseline,
        }
    }

    /// 排版光标所在的分隔线：原文（`---`）以次要色写在左侧，分隔线从原文之后画到右边缘。
    ///
    /// 高度与排版后的分隔线相同，原文的一行在分隔线上下的外边距中垂直居中：光标移入、移出
    /// 分隔线时其下方的内容不移动，修改原文时仍可看出这是一条分隔线。
    fn raw_rule(&mut self, x: f32, y: f32, width: f32, out: &mut Out) -> Flow {
        let h = rule_height(self.base);
        let flow = Flow {
            height: h,
            first_baseline: None,
        };
        let Some(raw) = &self.raw else {
            return flow;
        };
        let (block, leaf) = (Arc::clone(&raw.block), raw.leaf.clone());
        let text = block.get(leaf.clone()).unwrap_or_default();
        let line = text.split('\n').next().unwrap_or_default();
        let line = line.trim_end_matches('\r');
        let size = self.base * 0.85;
        let m = self.frame.metrics(FontKind::Mono, size);
        let row = Row {
            room: width.max(size),
            size,
            line_h: (size * 1.45).round(),
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let source = u32::try_from(leaf.start).unwrap_or(0);
        let piece = self.code_piece(line, source, "markdown", language, &mut state, row);
        let first = out.items.len();
        self.paste(
            &piece,
            x,
            y + (h - piece.height) / 2.0,
            source,
            &mut out.items,
        );
        let mut right = x;
        for item in &mut out.items[first..] {
            right = right.max(item.span_x().1);
            if let Item::Text { color, .. } = item {
                *color = self.colors.muted;
            }
        }
        let gap = (self.base * 0.6).round();
        if right + gap < x + width {
            out.items.push(Item::Fill {
                rect: Rect::new(right + gap, y, x + width - right - gap, h),
                color: self.colors.border,
                radius: 0.0,
            });
        }
        flow
    }

    /// 将光标所在的代码、公式或 HTML 块的原文放入代码框，内容按 `lang` 着色。
    ///
    /// 代码框的底色、边距、字号与排版后的代码块相同；框后光标所在的空行排在框外。围栏行只在
    /// 光标位于其上时显示，并以较淡的颜色绘制。因此点击进入代码块时外观不变、文字不移动；
    /// 用 ←→ 移出内容首尾时显示围栏，可以修改语言标识。
    fn raw_box(&mut self, lang: &str, x: f32, y: f32, width: f32, out: &mut Vec<Item>) -> Flow {
        let Some(raw) = &self.raw else {
            return Flow {
                height: 0.0,
                first_baseline: None,
            };
        };
        let (block, leaf) = (Arc::clone(&raw.block), raw.leaf.clone());
        let fenced = matches!(raw.kind, RawKind::Fenced(_));
        let blanks = blanks(raw);
        let text = block.get(leaf.clone()).unwrap_or_default();
        let text = &text[..text.trim_end().len()];
        let pad = self.base;
        let size = self.base * 0.85;
        let m = self.frame.metrics(FontKind::Mono, size);
        let row = Row {
            room: (width - pad * 2.0).max(size),
            size,
            line_h: (size * 1.45).round(),
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find(lang);
        let markdown = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let bg_at = out.len();
        let (mut top, mut first_baseline) = (y + pad, None);
        let mut start = leaf.start;
        let count = text.split('\n').count();
        let caret = raw.caret;
        for (i, line) in text.split('\n').enumerate() {
            let source = u32::try_from(start).unwrap_or(0);
            let here = (start..=start + line.len()).contains(&caret);
            start += line.len() + 1;
            // 第一行，以及（行数不少于两行且确为围栏时）最后一行是围栏，按 Markdown 着色。
            let fence = fenced && count > 1 && (i == 0 || (i + 1 == count && is_fence(line)));
            if fence && !here {
                continue;
            }
            let piece = if fence {
                let mut fence_state = winc_syntax::LineState::default();
                self.code_piece(line, source, "markdown", markdown, &mut fence_state, row)
            } else {
                self.code_piece(line, source, lang, language, &mut state, row)
            };
            first_baseline.get_or_insert(top + piece.first_baseline.unwrap_or(0.0));
            let at = out.len();
            self.paste(&piece, x + pad, top, source, out);
            if fence {
                // 围栏只是标记，以较淡的颜色绘制。
                let muted = self.colors.muted;
                for item in &mut out[at..] {
                    if let Item::Text { color, .. } = item {
                        *color = muted;
                    }
                }
            }
            top += piece.height;
        }
        let height = top + pad - y;
        let rect = Rect::new(x, y, width, height);
        out.insert(
            bg_at,
            Item::Fill {
                rect,
                color: self.colors.code_block,
                radius: 6.0,
            },
        );
        // 代码块在编辑时同样显示复制按钮，复制围栏之间的代码。
        if fenced && !matches!(lang, "mermaid" | "latex") {
            let lines: Vec<&str> = text.split('\n').collect();
            let close = usize::from(lines.len() > 1 && lines.last().is_some_and(|l| is_fence(l)));
            let code = lines[1.min(lines.len())..lines.len() - close].join("\n");
            out.push(Item::Code {
                rect,
                text: code.into(),
                diagram: false,
            });
        }
        let mut blank_out = Out::default();
        let extra = self.blank_lines(&blanks, x, y + height, width, &mut blank_out);
        out.append(&mut blank_out.items);
        Flow {
            height: height + extra,
            first_baseline,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn code(
        &mut self,
        lang: &str,
        text: &str,
        map: &SourceMap,
        x: f32,
        y: f32,
        width: f32,
        out: &mut Vec<Item>,
    ) -> Flow {
        let pad = self.base;
        let size = self.base * 0.85;
        let line_h = (size * 1.45).round();
        let m = self.frame.metrics(FontKind::Mono, size);
        let room = (width - pad * 2.0).max(size);
        let language = winc_syntax::find(lang);
        let mut state = winc_syntax::LineState::default();
        let bg_at = out.len();
        let mut top = y + pad;
        let mut first_baseline = None;
        let row = Row {
            room,
            size,
            line_h,
            ascent: m.ascent,
            descent: m.descent,
        };
        let mut line_start = 0;
        for line in text.split('\n') {
            let source = map.source(line_start).and_then(|s| u32::try_from(s).ok());
            line_start += line.len() + 1;
            let source = source.unwrap_or(0);
            let piece = self.code_piece(line, source, lang, language, &mut state, row);
            first_baseline.get_or_insert(top + piece.first_baseline.unwrap_or(0.0));
            self.paste(&piece, x + pad, top, source, out);
            top += piece.height;
        }
        let height = top + pad - y;
        let rect = Rect::new(x, y, width, height);
        out.insert(
            bg_at,
            Item::Fill {
                rect,
                color: self.colors.code_block,
                radius: 6.0,
            },
        );
        // 放在代码后面：按钮盖在文字上。
        out.push(Item::Code {
            rect,
            text: text.into(),
            diagram: false,
        });
        Flow {
            height,
            first_baseline,
        }
    }

    /// 排代码的一行（见 [`Cx::code_line`]），按内容、行首的词法状态和排法缓存。
    ///
    /// 长代码块改一行、在显示原文的大块里打字时，只重排改了的那一行。`source` 是这一行在原文
    /// 中的位置，取出后粘贴时按它挪；`state` 换成行末的词法状态。
    fn code_piece(
        &mut self,
        line: &str,
        source: u32,
        lang: &str,
        language: Option<&'static winc_syntax::Language>,
        state: &mut winc_syntax::LineState,
        row: Row,
    ) -> Arc<memo::Piece> {
        let key = memo::key(|h| {
            line.hash(h);
            state.hash(h);
            lang.hash(h);
            row.room.to_bits().hash(h);
            row.size.to_bits().hash(h);
            row.line_h.to_bits().hash(h);
        });
        self.memo.touch(key);
        if let Some((piece, next)) = self.memo.lines.get(&key) {
            *state = *next;
            return Arc::clone(piece);
        }
        let mut piece =
            self.isolated(|cx, out| cx.code_line(line, Some(source), language, state, row, out));
        piece.source = source;
        let piece = Arc::new(piece);
        self.memo.lines.insert(key, (Arc::clone(&piece), *state));
        piece
    }

    /// 排代码块的一行（坐标从 0 起），太长时在字之间折到下一行；`source` 是这一行在原文中的
    /// 位置（相对顶层块），`state` 是行首的词法状态，排完换成行末的。
    fn code_line(
        &mut self,
        line: &str,
        source: Option<u32>,
        language: Option<&'static winc_syntax::Language>,
        state: &mut winc_syntax::LineState,
        row: Row,
        out: &mut Vec<Item>,
    ) -> Flow {
        let chars: Vec<char> = line
            .chars()
            .map(|c| if c == '\t' { ' ' } else { c })
            .collect();
        let text_color = self.colors.text;
        // 每个字的颜色：按语法高亮。
        let mut colors = vec![text_color; chars.len()];
        if let Some(language) = language {
            let mut tokens = Vec::new();
            *state = winc_syntax::highlight_line(language, &chars, *state, &mut tokens);
            for token in &tokens {
                let color = self.palette.token(token.kind);
                for c in &mut colors[token.start.min(chars.len())..token.end.min(chars.len())] {
                    *c = color;
                }
            }
        }
        // 同一行同一颜色的字合成一段，`run` 是攒着的一段：文字、每个字的宽度、在块文字里的
        // 起点、横坐标、颜色、在原文中的位置。代码行与原文逐字节相同（制表符换成的空格也是
        // 一个字节），没有不连续之处。
        let size = row.size;
        let baseline = |top: f32| top + (row.line_h - row.ascent - row.descent) / 2.0 + row.ascent;
        let flush = |out: &mut Vec<Item>,
                     run: &mut (String, Vec<f32>, usize, f32, Option<Color>, Option<u32>),
                     top: f32| {
            if !run.0.is_empty() {
                out.push(Item::Text {
                    x: run.3,
                    baseline: baseline(top),
                    text: std::mem::take(&mut run.0).into(),
                    font: FontKind::Mono,
                    faux: Faux::default(),
                    size,
                    color: run.4.unwrap_or(text_color),
                    span: Some(Span {
                        start: run.2,
                        advances: std::mem::take(&mut run.1).into(),
                        source: run.5,
                        jumps: None,
                    }),
                });
            }
            run.4 = None;
        };
        let (mut top, mut pen) = (0.0, 0.0);
        let mut run = (String::new(), Vec::new(), 0, 0.0, None, None);
        let mut byte = 0;
        for (i, &c) in chars.iter().enumerate() {
            let w = self
                .frame
                .advance_faux(FontKind::Mono, Faux::default(), size, c);
            if pen + w > row.room && pen > 0.0 {
                flush(out, &mut run, top);
                top += row.line_h;
                pen = 0.0;
            }
            if run.4.is_some_and(|rc| rc != colors[i]) {
                flush(out, &mut run, top);
            }
            if run.0.is_empty() {
                run.2 = self.chars;
                run.3 = pen;
                run.5 = source.map(|s| s + byte);
            }
            byte += c.len_utf8() as u32;
            run.4 = Some(colors[i]);
            run.0.push(c);
            run.1.push(w);
            self.text.push(c);
            self.chars += 1;
            pen += w;
        }
        flush(out, &mut run, top);
        // 空行留一段没有字的文字：光标在空行上时也有位置可画，块文字里也占一个换行。
        if chars.is_empty() {
            out.push(Item::Text {
                x: 0.0,
                baseline: baseline(0.0),
                text: "".into(),
                font: FontKind::Mono,
                faux: Faux::default(),
                size,
                color: text_color,
                span: Some(Span {
                    start: self.chars,
                    advances: Arc::new([]),
                    source,
                    jumps: None,
                }),
            });
        }
        // 块文字里空行也占一个换行。
        self.text.push('\n');
        self.chars += 1;
        Flow {
            height: top + row.line_h,
            first_baseline: Some(baseline(0.0)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn table(
        &mut self,
        aligns: &[Align],
        rows: &[Vec<Inline>],
        head: bool,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Vec<Item>,
        scrolls: &mut Vec<Scroller>,
    ) -> Flow {
        // 表格的字号比正文小一级（正文 16 时为 14），表格内信息密度更高；内边距和行高随之缩放。
        let base = (self.base * TABLE_TEXT).round();
        let first_item = out.len();
        let (pad_x, pad_y) = ((base * 0.8125).round(), (base * 0.5).round());
        let cols = rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(aligns.len());
        if cols == 0 {
            return Flow {
                height: 0.0,
                first_baseline: None,
            };
        }
        let para = |header: bool, col: usize| Para {
            size: base,
            bold: header,
            color: env.color,
            line: 1.5,
            align: aligns.get(col).copied().unwrap_or(Align::None),
        };
        // 可编辑的预览中光标所在的单元格按排版显示，只显示光标接触到的标记（规则同正文，
        // 见 `reveal`）；空单元格保留一段空的原文，使光标有绘制位置。
        let live = self.cell.take().and_then(|(r, c, range)| {
            let raw = self.raw.as_ref()?;
            let segment = raw.block.get(range.clone())?;
            let content = segment.trim();
            // 空单元格的原文位置与解析结果一致（左侧 `|` 之后一个空格处），光标进出时不跳动。
            let start = if content.is_empty() {
                (range.start + 1).min(range.end)
            } else {
                range.start + segment.len() - segment.trim_start().len()
            };
            let cell = (!content.is_empty()).then(|| {
                let caret = raw.caret.saturating_sub(start).min(content.len());
                let preedit = raw.preedit.as_deref();
                let mut block =
                    winc_preview::markdown::reveal::reveal(content, Some(caret), preedit);
                let (Block::Paragraph(inline) | Block::Heading(_, inline)) = &mut block else {
                    unreachable!("reveal 只排出段落、标题");
                };
                inline.source = start;
                std::mem::take(inline)
            });
            self.preedit |= cell.is_some() && raw.preedit.is_some();
            Some((r, c, start, cell))
        });
        let mono = base * 0.85;
        let raw_row = |room: f32, cx: &mut Self| {
            let m = cx.frame.metrics(FontKind::Mono, mono);
            Row {
                room: room.max(mono),
                size: mono,
                line_h: (base * 1.5).round(),
                ascent: m.ascent,
                descent: m.descent,
            }
        };
        // 各列不换行要多宽、最窄能多窄。
        let mut min = vec![0f32; cols];
        let mut max = vec![0f32; cols];
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let cell = match &live {
                    Some((lr, lc, _, live)) if (*lr, *lc) == (r, c) => live.as_ref(),
                    _ => Some(cell),
                };
                let (lo, hi) = match cell {
                    Some(cell) => self.measure(cell, para(head && r == 0, c)),
                    None => (0.0, 0.0),
                };
                min[c] = min[c].max(lo + pad_x * 2.0);
                max[c] = max[c].max(hi + pad_x * 2.0);
            }
        }
        // 放得下时各列取不换行的宽度。放不下时各列仍取不换行的宽度，但每列最宽为正文一栏宽，
        // 更长的内容在列内换行；表格超出正文的部分横向滚动查看，不把各列压窄到逐字换行。
        // 按该规则排出的表格仍比正文窄（长内容的列换了行）时，按比例放宽各列，铺满正文。
        let (sum_min, sum_max): (f32, f32) = (min.iter().sum(), max.iter().sum());
        // 向上取整：向下取整会少零点几像素，最后一个字放不下而换行。
        let capped: Vec<f32> = (min.iter().zip(&max))
            .map(|(lo, hi)| hi.min(width.max(*lo)).ceil())
            .collect();
        let widths: Vec<f32> = if sum_max <= width {
            max.clone()
        } else if capped.iter().sum::<f32>() > width || sum_min >= width {
            capped
        } else {
            let t = (width - sum_min) / (sum_max - sum_min);
            min.iter()
                .zip(&max)
                .map(|(lo, hi)| (lo + (hi - lo) * t).floor())
                .collect()
        };
        let total: f32 = widths.iter().sum::<f32>() + 1.0;
        let mut top = y;
        let mut first_baseline = None;
        let mut lines = vec![top];
        for (r, row) in rows.iter().enumerate() {
            let start = out.len();
            let mut cell_x = x;
            let mut row_h: f32 = 0.0;
            for (c, w) in widths.iter().enumerate() {
                if let Some(cell) = row.get(c) {
                    let cell = match &live {
                        Some((lr, lc, source, None)) if (*lr, *lc) == (r, c) => Err(*source),
                        Some((lr, lc, _, Some(live))) if (*lr, *lc) == (r, c) => Ok(live),
                        // 空单元格同样保留一段空的原文，使其可点击，光标也有绘制位置。
                        _ if cell.text.is_empty() => Err(cell.source),
                        _ => Ok(cell),
                    };
                    let flow = match cell {
                        Err(source) => {
                            let row = raw_row(w - pad_x * 2.0, self);
                            let source = u32::try_from(source).unwrap_or(0);
                            let mut state = winc_syntax::LineState::default();
                            let piece =
                                self.code_piece("", source, "markdown", None, &mut state, row);
                            self.paste(&piece, cell_x + pad_x, top + pad_y, source, out);
                            Flow {
                                height: piece.height,
                                first_baseline: piece.first_baseline.map(|b| top + pad_y + b),
                            }
                        }
                        Ok(cell) => self.flow(
                            cell,
                            cell_x + pad_x,
                            top + pad_y,
                            w - pad_x * 2.0,
                            para(head && r == 0, c),
                            out,
                        ),
                    };
                    row_h = row_h.max(flow.height);
                    if r == 0 {
                        first_baseline = first_baseline.or(flow.first_baseline);
                    }
                }
                cell_x += w;
            }
            let row_h = row_h.max(base * 1.5) + pad_y * 2.0;
            // 表头铺底色（上方两角与外框同为圆角），与正文各行区分；正文各行之间只有横线。
            if head && r == 0 {
                out.insert(
                    start,
                    Item::Corners {
                        rect: Rect::new(x, top, total, row_h),
                        color: self.colors.table_head,
                        radii: [TABLE_RADIUS, TABLE_RADIUS, 0.0, 0.0],
                    },
                );
            }
            top += row_h;
            lines.push(top);
        }
        // 边框：圆角的外框，行之间一道横线；列之间不画竖线，按单元格的内边距区分各列。
        for &line_y in &lines[1..lines.len() - 1] {
            out.push(Item::Fill {
                rect: Rect::new(x + 1.0, line_y, total - 2.0, 1.0),
                color: self.colors.table_line,
                radius: 0.0,
            });
        }
        out.push(Item::path(
            rounded_frame(Rect::new(x, y, total, top - y + 1.0), TABLE_RADIUS),
            1.0,
            self.colors.table_frame,
        ));
        out.push(Item::Table {
            rect: Rect::new(x, y, total, top - y + 1.0),
            align: live
                .as_ref()
                .map(|(_, c, ..)| aligns.get(*c).copied().unwrap_or_default()),
            cols: widths.into(),
            rows: lines.windows(2).map(|w| w[1] - w[0]).collect(),
        });
        let mut height = top - y + 1.0;
        // 比正文宽：记下表格的各项，绘制时横向平移；下方留出滚动条的位置。
        if total > width + 0.5 {
            height += SCROLL_BAR_ROOM;
            scrolls.push(Scroller {
                items: u32::try_from(first_item).unwrap_or(u32::MAX)
                    ..u32::try_from(out.len()).unwrap_or(u32::MAX),
                rect: Rect::new(x, y, width, height),
                content: total,
            });
        }
        Flow {
            height,
            first_baseline,
        }
    }
}

/// 收集一个块里要画的项和标题锚点。
#[derive(Default)]
pub(super) struct Out {
    /// 要画的项，按绘制先后排列。
    pub(super) items: Vec<Item>,
    /// 标题锚点和所在高度（逻辑像素，相对块的左上角）。
    pub(super) anchors: Vec<(String, f32)>,
    /// 比正文宽、可以横向滚动的表格。
    pub(super) scrolls: Vec<Scroller>,
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
pub(super) fn margins(block: &Block, base: f32, tight: bool) -> (f32, f32) {
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

/// 折叠块标题的「链接」地址的开头，后面接 [`details_id`]：点了不是打开链接，是展开、收起。
pub(crate) const DETAILS: &str = "\u{1}details:";

/// 折叠块的标识：内容的哈希。
///
/// 内容相同的两个折叠块会一起开合；这种情况少见，未单独处理。
fn details_id(block: &Block) -> u64 {
    memo::key(|h| block.hash(h))
}

pub(crate) use winc_preview::markdown::slug;
