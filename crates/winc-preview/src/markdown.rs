//! Markdown 预览的文档结构：pulldown-cmark 解析（CommonMark 加 GFM 扩展：表格、任务列表、
//! 删除线、脚注、`> [!NOTE]` 提示块），转成排版要用的块和行内文字。
//!
//! 这里只描述内容，不涉及字号、颜色和坐标。解析在后台线程进行，每个顶层块带内容哈希，未变化的
//! 块沿用上次的排版结果。HTML 只认常见的标签：其余去掉标签、只留文字，HTML 注释不显示。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::Path;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

/// 行内图片在 [`Inline::text`] 里占的字符；第 n 个此字符对应 [`Inline::images`] 的第 n 项。
pub const IMAGE_MARK: char = '\u{FFFC}';

/// 容器块的最大嵌套层数（层）：引用、列表和列表项、脚注、`<details>` 各算一层。
///
/// 解析结果的处理按层递归（收缩容量、调整原文位置、比较、释放，以及预览的排版和绘制），数万层
/// `>` 会导致栈溢出、进程中止。超过上限的引用、列表、`<details>` 结束时不再嵌套一层，其中的块
/// 直接放进外层容器，文字仍然显示。依据 Markdown 解析器 markdown-it 的 `maxNesting` 默认值。
const MAX_NESTING: usize = 100;

/// 是否为 Markdown 文件（按扩展名判断）。
pub fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["md", "markdown", "mdown", "mkd", "mkdn"]
            .iter()
            .any(|m| e.eq_ignore_ascii_case(m))
    })
}

/// 一个叶子块（见 [`Markdown::leaves`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafRange {
    /// 在原文中的字节范围。
    pub range: Range<usize>,
    /// 是否为正文（段落、标题）：正文中回车另起一段、列表续一项；代码块、公式、表格、HTML 块中
    /// 回车只是换行。
    pub prose: bool,
    /// 是否为表格：可编辑的预览中按单元格编辑。
    pub table: bool,
}

/// 解析完成的 Markdown 文档。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Markdown {
    /// 顶层的块，依次排列；脚注的内容统一放在最后。
    pub blocks: Vec<Block>,
    /// 每个顶层块的内容哈希，与 `blocks` 一一对应。
    pub hashes: Vec<u64>,
    /// 每个顶层块的起始行号（从 0 起），与 `blocks` 一一对应，单调不减；分栏预览时两侧按它对齐
    /// 滚动。
    ///
    /// 移到最后的脚注算在原文末尾。
    pub lines: Vec<usize>,
    /// 每个顶层块在原文中的字节范围，与 `blocks` 一一对应；块里记下的原文位置（见
    /// [`SourceMap`]）都相对于范围的起点。
    ///
    /// 移到最后的脚注是脚注定义所在的范围，分隔线是原文末尾的空范围。
    pub ranges: Vec<Range<usize>>,
    /// 各叶子块（段落、标题、代码块、公式、分隔线、表格、HTML 块）按起点排列，互不重叠。
    ///
    /// 可编辑的预览中，光标所在的叶子块显示原文。
    pub leaves: Vec<LeafRange>,
    /// 原文的字节数：与当前文档的长度相比，得出解析之后又改了多少。
    pub len: usize,
}

/// 文字与原文的对照：文字中的字节偏移对应原文中的哪个字节偏移。
///
/// 按文字的先后记下若干段：每段文字与原文逐字节相同；不在任何一段里的文字（软换行换成的
/// 空格、emoji、脚注编号这类由原文生成的文字）对应前一段的末尾。
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SourceMap(Vec<Anchor>);

/// [`SourceMap`] 的一段：文字从 `text` 起、原文从 `source` 起的 `len` 个字节相同；`len` 为 0
/// 时只是一个对应点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Anchor {
    text: u32,
    source: u32,
    len: u32,
}

/// 把原文的字节偏移转成 [`Anchor`] 中记录的 `u32`；超过 4 GB 时返回 `None`。
///
/// 超过 4 GB 的文件按大文件打开，不生成预览，因此不记录文字与原文的对照。
fn offset(at: usize) -> Option<u32> {
    u32::try_from(at).ok()
}

impl SourceMap {
    /// 文字中第 `at` 个字节对应原文中的字节偏移；`at` 在第一段之前或没有对照时为 `None`。
    pub fn source(&self, at: usize) -> Option<usize> {
        let i = self
            .0
            .partition_point(|a| a.text as usize <= at)
            .checked_sub(1)?;
        let a = self.0[i];
        Some(a.source as usize + (at - a.text as usize).min(a.len as usize))
    }

    /// 占用的堆内存（字节，按容量计算），用于内存测量。
    pub fn heap_bytes(&self) -> usize {
        self.0.capacity() * std::mem::size_of::<Anchor>()
    }

    /// 对照的段数。
    pub fn anchors(&self) -> usize {
        self.0.len()
    }

    /// 是否没有任何对照。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 记下文字从 `text` 起的 `len` 个字节与原文从 `source` 起的字节相同；与前一段首尾相接时
    /// 合并。
    fn push(&mut self, text: usize, source: usize, len: usize) {
        let (Some(text), Some(source), Some(len)) = (offset(text), offset(source), offset(len))
        else {
            return;
        };
        if let Some(last) = self.0.last_mut() {
            if last.len > 0 && last.text + last.len == text && last.source + last.len == source {
                last.len += len;
                return;
            }
            // 同一处的对应点只留最后一个。
            if last.len == 0 && last.text == text {
                *last = Anchor { text, source, len };
                return;
            }
        }
        self.0.push(Anchor { text, source, len });
    }

    /// 文字中 `range` 内的字节换成了 `len` 个字节的新文字。
    ///
    /// 新文字对应原范围的起点，其后各段的文字偏移按长度差平移。
    fn replace(&mut self, range: Range<usize>, len: usize) {
        let Some(start) = self.source(range.start) else {
            return;
        };
        let (from, to) = (range.start as i64, range.end as i64);
        let delta = len as i64 - (to - from);
        let mut out = Vec::with_capacity(self.0.len() + 1);
        for a in &self.0 {
            let (text, end) = (a.text as i64, a.text as i64 + a.len as i64);
            if text < from {
                let len = (end.min(from) - text) as u32;
                out.push(Anchor { len, ..*a });
            }
        }
        out.push(Anchor {
            text: range.start as u32,
            source: start as u32,
            len: 0,
        });
        for a in &self.0 {
            let (text, end) = (a.text as i64, a.text as i64 + a.len as i64);
            if text >= to {
                out.push(Anchor {
                    text: (text + delta) as u32,
                    ..*a
                });
            } else if end > to {
                let cut = (to - text) as u32;
                out.push(Anchor {
                    text: (to + delta) as u32,
                    source: a.source + cut,
                    len: a.len - cut,
                });
            }
        }
        self.0 = out;
    }

    /// 最靠前的原文位置。
    fn first(&self) -> Option<usize> {
        self.0.iter().map(|a| a.source as usize).min()
    }

    /// 记录的各段原文位置都减去 `by`，改为相对于原文第 `by` 个字节。
    fn rebase(&mut self, by: usize) {
        let by = offset(by).unwrap_or(u32::MAX);
        for a in &mut self.0 {
            a.source = a.source.saturating_sub(by);
        }
    }
}

/// 在原文 `source` 的 `*cursor..end` 中找刚放进文字第 `at` 个字节起的 `text`，记到 `map`，
/// `*cursor` 移到找到处之后。
///
/// 通常 `text` 原样出现在原文中（行内代码、公式在定界符里面）；含转义、实体时逐字往后找，
/// 找不到的字对应当前位置。查找限于 `REACH` 以内，HTML 块很长时耗时也不随长度增长。
fn locate(
    map: &mut SourceMap,
    at: usize,
    text: &str,
    source: &str,
    cursor: &mut usize,
    end: usize,
) {
    let window = |from: usize, reach: usize| {
        let mut to = end.min(from.saturating_add(reach)).max(from);
        while !source.is_char_boundary(to) {
            to -= 1;
        }
        source.get(from..to).unwrap_or_default()
    };
    if text.is_empty() {
        return;
    }
    if let Some(p) = window(*cursor, text.len() + REACH).find(text) {
        map.push(at, *cursor + p, text.len());
        *cursor += p + text.len();
        return;
    }
    for (i, c) in text.char_indices() {
        match window(*cursor, REACH).find(c) {
            Some(p) => {
                map.push(at + i, *cursor + p, c.len_utf8());
                *cursor += p + c.len_utf8();
            }
            None => map.push(at + i, *cursor, 0),
        }
    }
}

/// 在原文中查找下一个字时往后找的最远距离（字节，经验值）。
///
/// 该值明显超过常见 HTML 标签（带长网址的 `<img>`、`<a>`）的长度，并避免在长 HTML 块中
/// 每次查找都扫到块尾。
const REACH: usize = 4096;

/// 在脚注 `number` 的内容末尾加一个跳回正文引用处的 ↩。
///
/// 该字符接在最后一段之后；最后一块不是段落时另起一段。
fn backlink(blocks: &mut Vec<Block>, number: &str) {
    if !matches!(blocks.last(), Some(Block::Paragraph(_))) {
        blocks.push(Block::Paragraph(Inline::default()));
    }
    let Some(Block::Paragraph(inline)) = blocks.last_mut() else {
        return;
    };
    if !inline.text.is_empty() {
        inline.push(" ", Style::default());
    }
    inline.links.push(format!("#fnref-{number}"));
    let link = u16::try_from(inline.links.len()).unwrap_or(u16::MAX);
    inline.push(
        "↩",
        Style {
            link,
            ..Style::default()
        },
    );
}

/// 一个块。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Block {
    /// 段落。
    Paragraph(Inline),
    /// 标题，级别 1 至 6。
    Heading(u8, Inline),
    /// 代码块。
    Code {
        /// 围栏后面写的第一个词；没有写语言时为空。
        lang: String,
        /// 代码，末尾不带换行。
        text: String,
        /// 代码与原文的对照，原文位置相对所在顶层块的起点；HTML 的 `<pre>` 中没有对照。
        map: SourceMap,
    },
    /// 引用；GFM 的 `> [!NOTE]` 这类提示块带上种类。
    Quote(Option<Alert>, Vec<Block>),
    /// 列表：有序列表的起始编号，无序列表为 `None`。
    List(Option<u64>, Vec<Item>),
    /// 表格：各列的对齐方式、各行单元格、第一行是不是表头（Markdown 的表格总是，HTML 写的
    /// 看第一行是不是 `<th>`）。
    Table(Vec<Align>, Vec<Vec<Inline>>, bool),
    /// 单独成行的公式（`$$…$$`、```` ```math ````）：LaTeX 原文（见 [`math`]）和公式在原文中的
    /// 起点（字节偏移，解析完成后相对所在的顶层块）。
    Math(String, usize),
    /// 分隔线和它在原文中的起点（字节偏移，解析完成后相对所在的顶层块）；HTML 的 `<hr>`、
    /// 脚注之前生成的分隔线没有对应的原文，为 `None`。
    Rule(Option<usize>),
    /// 一条脚注：编号（按正文里第一次引用的先后）和内容。
    Footnote(String, Vec<Block>),
    /// HTML 的 `<details>`：点标题（`<summary>`，没写时是「Details」）展开、收起的一块。
    Details {
        /// 是否写了 `open` 属性；带 `open` 时一开始就是展开的。
        open: bool,
        /// 标题。
        summary: Inline,
        /// 展开时显示的内容。
        blocks: Vec<Block>,
    },
}

/// 列表里的一项。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Item {
    /// 任务列表的勾选状态；不是任务时为 `None`。
    pub task: Option<bool>,
    /// 勾选框 `[ ]` 在原文中的位置（字节偏移，相对顶层块的开头）。
    ///
    /// 点击预览中的勾选框时据此修改原文；不是任务项时为 0。
    pub task_source: usize,
    /// 内容。
    pub blocks: Vec<Block>,
    /// 记号之后没有空格的空项（原文只有 `-`、`1.`）：按文字显示记号，不画圆点和序号。
    ///
    /// 输入记号后再输入空格才成为列表项；`blocks` 是记号原文构成的一个段落。
    pub bare: bool,
}

/// 表格一列的对齐方式；HTML 的 `align="center"` 等写法也归到这里的各变体。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    /// 没有指定对齐方式，按左对齐显示。
    #[default]
    None,
    /// 左对齐。
    Left,
    /// 居中。
    Center,
    /// 右对齐。
    Right,
}

/// GFM 的提示块种类（`> [!NOTE]` 等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alert {
    /// `[!NOTE]`。
    Note,
    /// `[!TIP]`。
    Tip,
    /// `[!IMPORTANT]`。
    Important,
    /// `[!WARNING]`。
    Warning,
    /// `[!CAUTION]`。
    Caution,
}

/// 一段行内文字：文字本身，加上按顺序覆盖全部文字的样式段。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Inline {
    /// 文字；软换行已换成空格，硬换行是 `\n`，图片是 [`IMAGE_MARK`]。
    pub text: String,
    /// 样式段：每段到 `end`（字节偏移，不含）为止，前一段的 `end` 是这一段的起点。
    pub runs: Vec<Run>,
    /// 链接地址，[`Style::link`] 从 1 起指向这里。
    pub links: Vec<String>,
    /// 图片，按在文字里出现的先后。
    pub images: Vec<Image>,
    /// 段落、标题的对齐：外面包着 `<p align="center">`、`<div align="center">` 这类 HTML 时
    /// 不为 [`Align::None`]（README 顶部居中的图标、徽章常用这种写法）。
    pub align: Align,
    /// 文字与原文的对照，原文位置相对于 [`Inline::source`]；由原文生成的文字（列表、表格之外
    /// 自动加的标题、front matter 的表格）为空。
    pub map: SourceMap,
    /// 这段文字最靠前的原文位置（字节偏移，相对所在顶层块的起点，见 [`Markdown::ranges`]）。
    ///
    /// 不计入哈希：排版缓存按内容取用，同样的文字挪了位置仍然命中，取出时再按记录的原文
    /// 位置偏移。
    pub source: usize,
}

impl Hash for Inline {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.runs.hash(state);
        self.links.hash(state);
        self.images.hash(state);
        self.align.hash(state);
        self.map.hash(state);
    }
}

/// 一段同样式的文字，范围见 [`Inline::runs`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Run {
    /// 这一段的终点（字节偏移，不含）。
    pub end: usize,
    /// 样式。
    pub style: Style,
}

/// 行内样式。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    /// [`Style::STRONG`] 等标志位。
    pub flags: u16,
    /// 所在链接在 [`Inline::links`] 中的序号加 1；不是链接时为 0。
    pub link: u16,
}

impl Style {
    /// 粗体。
    pub const STRONG: u16 = 1;
    /// 斜体。
    pub const EMPHASIS: u16 = 1 << 1;
    /// 删除线。
    pub const STRIKE: u16 = 1 << 2;
    /// 行内代码。
    pub const CODE: u16 = 1 << 3;
    /// 脚注引用（上标的编号）。
    pub const NOTE: u16 = 1 << 4;
    /// 上标（HTML 的 `<sup>`）。
    pub const SUP: u16 = 1 << 5;
    /// 下标（HTML 的 `<sub>`）。
    pub const SUB: u16 = 1 << 6;
    /// 行内公式：这一段文字是 LaTeX 原文，排版时按 [`math::parse`] 排成公式。
    pub const MATH: u16 = 1 << 7;
    /// 下划线（HTML 的 `<u>`、`<ins>`）。
    pub const UNDERLINE: u16 = 1 << 8;
    /// 荧光笔底色（HTML 的 `<mark>`）。
    pub const MARK: u16 = 1 << 9;
    /// 小一号的字（HTML 的 `<small>`）。
    pub const SMALL: u16 = 1 << 10;
    /// Markdown 的标记（`**`、`[`、`](网址)`、`` ` ``、`#` 等）。
    ///
    /// 可编辑的预览中，光标所在段落内光标接触到的元素显示其标记，以次要色排版（见 [`reveal`]）。
    pub const SYNTAX: u16 = 1 << 11;

    /// 是否带有标志 `flag`。
    pub fn has(self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

/// 一张图片。
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Image {
    /// 地址：相对路径、绝对路径或网址，不解析、不转义。
    pub url: String,
    /// 替代文字：图片无法显示时显示的文字。
    pub alt: String,
    /// HTML `<img>` 写明的宽度。
    pub width: Option<Length>,
    /// HTML `<img>` 写明的高度（像素）。
    pub height: Option<u32>,
    /// 深色外观时换用的地址：`<picture>` 里 `media="(prefers-color-scheme: dark)"` 的
    /// `<source>` 的地址（README 常这样为深色主题配一张图）。
    pub dark: Option<String>,
}

/// HTML 里写的宽度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Length {
    /// 像素（`width="200"`、`width="200px"`）。
    Px(u32),
    /// 占一行宽度的百分比，取值单位是万分之一（`width="49.45%"` 为 4945）。
    ///
    /// 用这种写法可以让几张图并排显示。
    Percent(u32),
}

impl Inline {
    /// 依次返回各样式段的字节范围和样式。
    pub fn runs(&self) -> impl Iterator<Item = (Range<usize>, Style)> + '_ {
        let mut start = 0;
        self.runs.iter().map(move |run| {
            let range = start..run.end;
            start = run.end;
            (range, run.style)
        })
    }

    /// 文字中第 `at` 个字节在原文中的位置（字节偏移，相对所在顶层块的起点）；没有对照时
    /// 为 `None`。
    pub fn source_of(&self, at: usize) -> Option<usize> {
        self.map.source(at).map(|s| s + self.source)
    }

    /// 是否没有可显示的内容（只有空白）。
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        match self.runs.last_mut() {
            Some(last) if last.style == style => last.end = self.text.len(),
            _ => self.runs.push(Run {
                end: self.text.len(),
                style,
            }),
        }
    }

    /// 去掉开头和末尾的空白（HTML 里常见，分段处的换行和缩进）。
    fn trim(mut self) -> Inline {
        let end = self.text.trim_end().len();
        let start = self.text.len() - self.text.trim_start().len();
        if start == 0 && end == self.text.len() {
            return self;
        }
        let text = self.text[start..end].to_owned();
        self.map.replace(0..start, 0);
        let runs = self
            .runs()
            .filter_map(|(range, style)| {
                let (from, to) = (range.start.max(start), range.end.min(end));
                (from < to).then_some(Run {
                    end: to - start,
                    style,
                })
            })
            .collect();
        self.text = text;
        self.runs = runs;
        self
    }
}

/// 解析时启用的扩展：GFM 的表格、删除线、任务列表、提示块，以及脚注、文首的 YAML 和公式。
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_MATH
}

/// 解析 `text`。
pub fn parse(text: &str) -> Markdown {
    let options = options();
    let mut builder = Builder {
        source: text,
        ..Builder::default()
    };
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        // 顶层块开始的地方：不在任何容器、段落、代码块、HTML 块里。分隔线没有开始、结束两个
        // 事件，只有一个。
        if matches!(event, Event::Start(_) | Event::Rule)
            && builder.stack.is_empty()
            && builder.inline.is_none()
            && builder.code.is_none()
            && builder.html.is_none()
        {
            builder.start_at = range.start;
        }
        (builder.start, builder.cursor, builder.end) = (range.start, range.start, range.end);
        builder.event(event);
        if builder.inline.is_some() {
            let leaf = builder.leaf.get_or_insert(range.clone());
            *leaf = leaf.start.min(range.start)..leaf.end.max(range.end);
        }
    }
    builder.finish(text)
}

/// 正在收集行内文字的块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    Paragraph,
    Heading(u8),
    /// 表格的一个单元格。
    Cell,
    /// 紧凑列表的一项里直接写的文字（没有 `Paragraph` 包着），当作段落。
    Loose,
    /// `<details>` 的标题。
    Summary,
}

/// 还没结束的容器块。
#[derive(Debug)]
enum Container {
    Quote(Option<Alert>, Vec<Block>),
    List(Option<u64>, Vec<Item>),
    /// 列表的一项：勾选状态、勾选框在原文中的位置（见 [`Item::task_source`]）和内容。
    Item(Option<(bool, usize)>, Vec<Block>),
    Footnote(String, Vec<Block>),
    /// `<details>`：有没有 `open`、标题、里面的块。
    Details(bool, Option<Inline>, Vec<Block>),
    /// 各列对齐、已经结束的行、正在填的一行、第一行是不是表头。
    Table(Vec<Align>, Vec<Vec<Inline>>, Vec<Inline>, bool),
}

/// 解析过程中逐步收集块和行内文字的状态。
#[derive(Debug, Default)]
struct Builder<'s> {
    /// 原文。
    source: &'s str,
    /// 当前事件在原文中的范围（`start..end`），以及在其中已经对照到哪里（见 [`locate`]）。
    start: usize,
    cursor: usize,
    end: usize,
    /// 最近一个软换行在原文中的位置：换成的空格对应这里。
    soft_at: usize,
    /// 正在收集的代码块与原文的对照。
    code_map: SourceMap,
    root: Vec<Block>,
    stack: Vec<Container>,
    inline: Option<(Leaf, Inline)>,
    /// 行内样式的栈：强调、链接等开始时压入，结束时弹出。
    styles: Vec<Style>,
    /// 正在收集替代文字的图片。
    image: Option<Image>,
    /// 正在收集的代码块：语言和代码。
    code: Option<(String, String)>,
    /// 正在收集的 HTML 块。
    html: Option<String>,
    /// 正文里引用过的脚注标签，按第一次引用的先后；编号是位置加 1。
    notes: Vec<String>,
    /// 脚注的内容和定义所在的原文范围，放到文档最后。
    footnotes: Vec<(String, Vec<Block>, Range<usize>)>,
    /// 包着当前内容的 HTML 标签（`<p>`、`<div>`、`<center>`、`<h1>`……）各自的对齐，一层一层。
    aligns: Vec<Align>,
    /// 刚才有一个软换行，还没决定要不要换成空格：两边都是中日韩文字时不加（中文段落里折行处
    /// 不会多出一个空格），否则加一个空格。
    soft: bool,
    /// `<picture>` 里浅色、深色外观的 `<source>` 写的地址，留给后面的 `<img>`。
    sources: (Option<String>, Option<String>),
    /// 正在填的 HTML 表格单元格是 `<th>`（或者正在写 `<dt>`），压了一层粗体，结束时弹出。
    cell_style: bool,
    /// 正在收集的代码块是文件开头的 YAML front matter。
    front: bool,
    /// 当前顶层块在原文里的起点（字节偏移）。
    start_at: usize,
    /// 各顶层块在原文中的范围，与 `root` 一一对应。
    ranges: Vec<Range<usize>>,
    /// 各叶子块，见 [`Markdown::leaves`]。
    leaves: Vec<LeafRange>,
    /// 正在收集的行内文字所在叶子块的范围：参与收集的各事件的范围合起来。
    ///
    /// 紧凑列表的一项没有段落事件，范围只能这样得到。
    leaf: Option<Range<usize>>,
    /// 正在处理 HTML 块：块里排出的段落、表格不单独记为叶子块，整个 HTML 块是一个。
    in_html: bool,
}

impl Builder<'_> {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, flags: u16, link: Option<u16>) {
        let mut style = self.style();
        style.flags |= flags;
        if let Some(link) = link {
            style.link = link;
        }
        self.styles.push(style);
    }

    /// 当前行内文字；不在段落、标题或单元格里时（紧凑列表的一项）新开一段。
    fn inline(&mut self) -> &mut Inline {
        &mut self
            .inline
            .get_or_insert_with(|| (Leaf::Loose, Inline::default()))
            .1
    }

    fn text(&mut self, text: &str, flags: u16) {
        if let Some(image) = &mut self.image {
            image.alt.push_str(text);
        } else if let Some((_, code)) = &mut self.code {
            let at = code.len();
            code.push_str(text);
            locate(
                &mut self.code_map,
                at,
                text,
                self.source,
                &mut self.cursor,
                self.end,
            );
        } else if let Some(html) = &mut self.html {
            html.push_str(text);
        } else {
            self.resolve_soft(text.chars().next());
            let mut style = self.style();
            style.flags |= flags;
            let inline = &mut self
                .inline
                .get_or_insert_with(|| (Leaf::Loose, Inline::default()))
                .1;
            let at = inline.text.len();
            inline.push(text, style);
            locate(
                &mut inline.map,
                at,
                text,
                self.source,
                &mut self.cursor,
                self.end,
            );
        }
    }

    /// 前面有软换行时，按下一个字 `next` 决定加不加空格（见 [`Builder::soft`]）。
    fn resolve_soft(&mut self, next: Option<char>) {
        if !std::mem::take(&mut self.soft) {
            return;
        }
        let style = self.style();
        let soft_at = self.soft_at;
        let inline = self.inline();
        let joined =
            inline.text.chars().next_back().is_some_and(is_cjk) && next.is_some_and(is_cjk);
        if !joined {
            inline.map.push(inline.text.len(), soft_at, 0);
            inline.push(" ", style);
        }
    }

    /// 结束一个容器块（引用、列表、`<details>`）。
    ///
    /// 已经嵌套了 [`MAX_NESTING`] 层时不再嵌套，其中的块直接放进外层容器；叶子块的位置在开始时
    /// 已经记录，不再重复记录。
    fn push_container(&mut self, block: Block) {
        if self.stack.len() < MAX_NESTING {
            return self.place(block);
        }
        let blocks = match block {
            Block::Quote(_, blocks) => blocks,
            Block::List(_, items) => items.into_iter().flat_map(|item| item.blocks).collect(),
            Block::Details {
                summary, blocks, ..
            } => std::iter::once(Block::Paragraph(summary))
                .chain(blocks)
                .collect(),
            block => vec![block],
        };
        for block in blocks {
            self.place(block);
        }
    }

    /// 将原文 `range` 逐字作为一个段落放入当前容器，不带样式。
    ///
    /// 用于后面没有空格的块标记（`-`、`1.`、`#`、`>`）：输入空格之前按文字显示。
    fn literal(&mut self, range: Range<usize>) {
        let mut inline = Inline::default();
        inline.push(&self.source[range.clone()], Style::default());
        inline.map.push(0, range.start, range.len());
        self.leaf = Some(range);
        self.push_block(Block::Paragraph(inline));
        self.leaf = None;
    }

    /// 列表项的内容只有 `[ ]`、`[x]` 时按任务项处理：勾选框之后尚未输入文字。
    ///
    /// 解析器要求勾选框之后有内容才识别任务项；输入 `- [ ]` 后即显示勾选框，无需先输入文字。
    fn bare_task(&mut self) {
        let Some(Container::Item(task @ None, blocks)) = self.stack.last_mut() else {
            return;
        };
        let [Block::Paragraph(inline)] = blocks.as_slice() else {
            return;
        };
        let checked = match inline.text.trim() {
            "[ ]" => false,
            "[x]" | "[X]" => true,
            _ => return,
        };
        let item = &self.source[self.start..self.end.max(self.start)];
        let Some(at) = item.find('[') else {
            return;
        };
        *task = Some((checked, self.start + at));
        blocks.clear();
        if !self.in_html {
            self.leaves.pop();
        }
    }

    /// 只有记号的列表项（`- `、`1. `、`- [ ] `）中放一个空段落，位于记号所在行的末尾；返回该项
    /// 是否为记号之后没有空格的空项（见 [`Item::bare`]）。
    ///
    /// 可编辑的预览中光标需要落在某个叶子块里：回车续出的新项在输入文字之前是空项，没有这个
    /// 空段落时光标只能归到下一项，该项改为显示原文，新项和下一项的原文混在一起。
    fn empty_item(&mut self) -> bool {
        let Some(Container::Item(task, blocks)) = self.stack.last() else {
            return false;
        };
        if !blocks.is_empty() || self.in_html {
            return false;
        }
        let task = task.is_some();
        let item = &self.source[self.start..self.end.max(self.start)];
        let line = item.find('\n').unwrap_or(item.len());
        let text = item[..line].trim_end_matches('\r');
        let at = self.start + text.len();
        let marker = text.trim_start();
        if !task && !marker.is_empty() && !marker.ends_with([' ', '\t']) {
            self.literal(at - marker.len()..at);
            return true;
        }
        self.anchor(at);
        false
    }

    /// 在当前容器中放一个空段落，原文位置为 `at`：没有内容的列表项、引用中光标据此定位。
    fn anchor(&mut self, at: usize) {
        let mut inline = Inline::default();
        inline.map.push(0, at, 0);
        self.leaf = Some(at..at);
        self.push_block(Block::Paragraph(inline));
        self.leaf = None;
    }

    /// 当前块的原文第一行是否只由标记字符 `mark` 组成（后面没有空格）；是则返回这些字符的范围。
    fn bare_marks(&self, mark: u8) -> Option<Range<usize>> {
        let block = &self.source[self.start..self.end.max(self.start)];
        let line = block.lines().next().unwrap_or_default();
        let marks = line.trim_start();
        let end = self.start + line.len();
        (!marks.is_empty() && marks.bytes().all(|b| b == mark) && !self.in_html)
            .then(|| end - marks.len()..end)
    }

    fn push_block(&mut self, block: Block) {
        let table = matches!(block, Block::Table(..));
        let leaf = match block {
            Block::Paragraph(_) | Block::Heading(..) => self.leaf.clone().map(|r| (r, true)),
            Block::Code { .. } | Block::Math(..) | Block::Rule(_) | Block::Table(..) => {
                Some((self.start..self.end, false))
            }
            _ => None,
        };
        if let Some((range, prose)) = leaf.filter(|_| !self.in_html) {
            self.leaves.push(LeafRange {
                range,
                prose,
                table,
            });
        }
        self.place(block);
    }

    /// 把块放进最内层的容器；没有打开的容器时作为顶层块。
    fn place(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some(
                Container::Quote(_, blocks)
                | Container::Item(_, blocks)
                | Container::Footnote(_, blocks)
                | Container::Details(_, _, blocks),
            ) => blocks.push(block),
            // 列表和表格里直接只会有项和行，不会走到这里。
            Some(Container::List(..) | Container::Table(..)) | None => {
                self.root.push(block);
                self.ranges.push(self.start_at..self.end.max(self.start_at));
            }
        }
    }

    fn finish_inline(&mut self) {
        self.soft = false;
        let Some((leaf, mut inline)) = self.inline.take() else {
            return;
        };
        emoji::shortcodes(&mut inline);
        autolink::autolink(&mut inline);
        if leaf != Leaf::Cell && inline.align == Align::None {
            inline.align = self.aligns.last().copied().unwrap_or_default();
        }
        match leaf {
            Leaf::Paragraph | Leaf::Loose => {
                if !inline.is_blank() || !inline.images.is_empty() {
                    self.push_block(Block::Paragraph(inline));
                }
            }
            // 只有 `#`、后面没有空格：按文字显示，输入空格后才成为标题。
            Leaf::Heading(level) => match self.bare_marks(b'#') {
                Some(range) if inline.is_blank() && inline.images.is_empty() => {
                    self.literal(range);
                }
                _ => self.push_block(Block::Heading(level, inline)),
            },
            Leaf::Summary => match self.stack.last_mut() {
                Some(Container::Details(_, summary, _)) => *summary = Some(inline),
                _ => self.push_block(Block::Paragraph(inline)),
            },
            Leaf::Cell => {
                if let Some(Container::Table(_, _, row, _)) = self.stack.last_mut() {
                    row.push(inline);
                }
            }
        }
        self.leaf = None;
    }

    /// 正在填表格的一个单元格。
    fn in_cell(&self) -> bool {
        matches!(self.inline, Some((Leaf::Cell, _)))
    }

    fn start_leaf(&mut self, leaf: Leaf) {
        self.finish_inline();
        self.inline = Some((leaf, Inline::default()));
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text, 0),
            Event::Code(text) => self.text(&text, Style::CODE),
            Event::InlineMath(text) => self.text(&text, Style::MATH),
            // `$$…$$` 单独成一块居中显示；写在表格里的只能留在格子里。
            Event::DisplayMath(text) if self.in_cell() => self.text(&text, Style::MATH),
            Event::DisplayMath(text) => {
                self.finish_inline();
                self.push_block(Block::Math(text.trim().to_owned(), self.start));
            }
            Event::Html(html) | Event::InlineHtml(html) => match &mut self.html {
                Some(block) => block.push_str(&html),
                None => self.html(&html, false),
            },
            Event::FootnoteReference(label) => {
                let n = match self.notes.iter().position(|l| *l == *label) {
                    Some(i) => i + 1,
                    None => {
                        self.notes.push(label.to_string());
                        self.notes.len()
                    }
                };
                // 编号是跳到脚注的链接；脚注末尾的 ↩ 跳回来（见 [`Builder::finish`]）。
                self.resolve_soft(Some('['));
                self.start_link(format!("#fn-{n}"));
                self.text(&format!("[{n}]"), Style::NOTE);
                self.styles.pop();
            }
            Event::SoftBreak if self.inline.is_some() && self.image.is_none() => {
                self.soft = true;
                self.soft_at = self.start;
            }
            Event::SoftBreak => self.text(" ", 0),
            Event::HardBreak => self.text("\n", 0),
            Event::Rule => {
                self.finish_inline();
                self.push_block(Block::Rule(Some(self.start)));
            }
            Event::TaskListMarker(checked) => {
                if let Some(Container::Item(task, _)) = self.stack.last_mut() {
                    *task = Some((checked, self.start));
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            // HTML 的 `<td>` 里空一行写的 Markdown：接在单元格里，另起一行。
            Tag::Paragraph if self.in_cell() => {
                if self.inline().text.trim().is_empty() {
                    self.inline = Some((Leaf::Cell, Inline::default()));
                } else {
                    self.text("\n", 0);
                }
            }
            Tag::Paragraph => self.start_leaf(Leaf::Paragraph),
            Tag::Heading { level, .. } => self.start_leaf(Leaf::Heading(heading_level(level))),
            // 空单元格没有可对照的文字，因此记录内容应在的位置（左侧 `|` 之后一个空格处，
            // 规则同 `cells::cells`），使可编辑的预览中可以点击进入。
            Tag::TableCell => {
                self.start_leaf(Leaf::Cell);
                self.inline().source = (self.start + 1).min(self.end);
            }
            Tag::BlockQuote(kind) => {
                self.finish_inline();
                self.stack
                    .push(Container::Quote(kind.map(alert), Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.finish_inline();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
                self.code_map = SourceMap::default();
            }
            Tag::MetadataBlock(_) => {
                self.finish_inline();
                self.code = Some(("yaml".to_owned(), String::new()));
                self.code_map = SourceMap::default();
                self.front = true;
            }
            Tag::HtmlBlock => {
                // HTML 表格的单元格可以跨几个 HTML 块（中间空一行写 Markdown），不在这里结束。
                if !self.in_cell() {
                    self.finish_inline();
                }
                self.html = Some(String::new());
                self.in_html = true;
            }
            Tag::List(start) => {
                self.finish_inline();
                self.stack.push(Container::List(start, Vec::new()));
            }
            Tag::Item => {
                self.finish_inline();
                self.stack.push(Container::Item(None, Vec::new()));
            }
            Tag::FootnoteDefinition(label) => {
                self.finish_inline();
                self.stack
                    .push(Container::Footnote(label.to_string(), Vec::new()));
            }
            Tag::Table(aligns) => {
                self.finish_inline();
                let aligns = aligns.into_iter().map(align).collect();
                self.stack
                    .push(Container::Table(aligns, Vec::new(), Vec::new(), true));
            }
            Tag::TableHead | Tag::TableRow => {}
            Tag::Emphasis => self.push_style(Style::EMPHASIS, None),
            Tag::Strong => self.push_style(Style::STRONG, None),
            Tag::Strikethrough => self.push_style(Style::STRIKE, None),
            Tag::Link { dest_url, .. } => self.start_link(dest_url.to_string()),
            Tag::Image { dest_url, .. } => {
                self.image = Some(Image {
                    url: dest_url.to_string(),
                    ..Image::default()
                });
            }
            // 没有打开的扩展。
            Tag::Superscript
            | Tag::Subscript
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph if self.in_cell() => {}
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell => self.finish_inline(),
            TagEnd::BlockQuote(_) => {
                self.finish_inline();
                let bare = self.bare_marks(b'>');
                let empty = matches!(
                    self.stack.last(),
                    Some(Container::Quote(None, blocks)) if blocks.is_empty()
                );
                // 没有内容的引用（`> `）中放一个空段落，位于第一行的末尾，光标据此定位。
                if empty && bare.is_none() && !self.in_html {
                    let block = &self.source[self.start..self.end.max(self.start)];
                    let line = block.lines().next().unwrap_or_default();
                    self.anchor(self.start + line.len());
                }
                if let Some(Container::Quote(kind, blocks)) = self.stack.pop() {
                    // 只有 `>`、后面没有空格：按文字显示，输入空格后才成为引用。
                    match bare {
                        Some(range) if empty => self.literal(range),
                        _ => self.push_container(Block::Quote(kind, blocks)),
                    }
                }
            }
            TagEnd::CodeBlock | TagEnd::MetadataBlock(_) => {
                if let Some((lang, mut text)) = self.code.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    // front matter 是简单的「键: 值」时显示成一张表。
                    let table = std::mem::take(&mut self.front)
                        .then(|| front::table(&text))
                        .flatten();
                    let block = match table {
                        Some(table) => table,
                        // 语言为 math 的围栏代码块是单独成行的公式。
                        None if lang.eq_ignore_ascii_case("math") => Block::Math(text, self.start),
                        None => Block::Code {
                            lang,
                            text,
                            map: std::mem::take(&mut self.code_map),
                        },
                    };
                    self.push_block(block);
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html.take() {
                    self.html(&html, true);
                    if !self.in_cell() {
                        self.finish_inline();
                    }
                }
                self.in_html = false;
                self.leaves.push(LeafRange {
                    range: self.start..self.end,
                    prose: false,
                    table: false,
                });
            }
            TagEnd::List(_) => {
                self.finish_inline();
                if let Some(Container::List(start, items)) = self.stack.pop() {
                    self.push_container(Block::List(start, items));
                }
            }
            TagEnd::Item => {
                self.finish_inline();
                self.bare_task();
                let bare = self.empty_item();
                if let Some(Container::Item(task, blocks)) = self.stack.pop()
                    && let Some(Container::List(_, items)) = self.stack.last_mut()
                {
                    items.push(Item {
                        task: task.map(|(checked, _)| checked),
                        task_source: task.map_or(0, |(_, at)| at),
                        blocks,
                        bare,
                    });
                }
            }
            TagEnd::FootnoteDefinition => {
                self.finish_inline();
                if let Some(Container::Footnote(label, blocks)) = self.stack.pop() {
                    self.footnotes.push((label, blocks, self.start..self.end));
                }
            }
            TagEnd::Table => {
                self.finish_inline();
                if let Some(Container::Table(aligns, rows, _, head)) = self.stack.pop() {
                    self.push_block(Block::Table(aligns, rows, head));
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                self.finish_inline();
                if let Some(Container::Table(_, rows, row, _)) = self.stack.last_mut() {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    self.push_image(image);
                }
            }
            TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    fn start_link(&mut self, url: String) {
        let inline = self.inline();
        inline.links.push(url);
        let link = u16::try_from(inline.links.len()).unwrap_or(u16::MAX);
        self.push_style(0, Some(link));
    }

    fn push_image(&mut self, image: Image) {
        self.resolve_soft(Some(IMAGE_MARK));
        let style = self.style();
        let at = self.start;
        let inline = self.inline();
        inline.images.push(image);
        inline.map.push(inline.text.len(), at, 0);
        inline.push(IMAGE_MARK.encode_utf8(&mut [0; 4]), style);
    }

    fn finish(mut self, source: &str) -> Markdown {
        self.finish_inline();
        // 没关上的容器（不会出现，pulldown-cmark 总是成对给出）按顺序收下。
        while !self.stack.is_empty() {
            self.end_any();
        }
        let main = self.root.len();
        if !self.footnotes.is_empty() {
            self.root.push(Block::Rule(None));
            self.ranges.push(source.len()..source.len());
            let mut footnotes = std::mem::take(&mut self.footnotes);
            // 按正文里第一次引用的先后排；没有被引用的放在最后。
            footnotes.sort_by_key(|(label, _, _)| {
                self.notes
                    .iter()
                    .position(|l| l == label)
                    .unwrap_or(usize::MAX)
            });
            for (label, mut blocks, range) in footnotes {
                let number = match self.notes.iter().position(|l| *l == label) {
                    Some(i) => {
                        let number = (i + 1).to_string();
                        backlink(&mut blocks, &number);
                        number
                    }
                    None => label,
                };
                self.root.push(Block::Footnote(number, blocks));
                self.ranges.push(range);
            }
        }
        contents(&mut self.root[..main]);
        for (block, range) in self.root.iter_mut().zip(&self.ranges) {
            rebase(block, range.start);
        }
        shrink(&mut self.root);
        self.ranges.shrink_to_fit();
        let mut lines = Vec::with_capacity(self.root.len());
        let (mut line, mut counted) = (0, 0);
        for i in 0..self.root.len() {
            // 脚注没有记起点：算在末尾。起点不会往回走，取不小于上一个的值。
            let at = if i < main {
                self.ranges[i].start
            } else {
                source.len()
            };
            let at = at.max(counted);
            line += memchr::memchr_iter(b'\n', &source.as_bytes()[counted..at]).count();
            counted = at;
            lines.push(line);
        }
        // 原文也计入哈希：只改了标记（列表记号、缩进）时内容不变，块里记下的原文位置却变了。
        let hashes = self
            .root
            .iter()
            .zip(&self.ranges)
            .map(|(block, range)| {
                let mut hasher = DefaultHasher::new();
                block.hash(&mut hasher);
                source.get(range.clone()).hash(&mut hasher);
                hasher.finish()
            })
            .collect();
        // 按起点排好，去掉落在前一个里面的（HTML 块里写的 Markdown 段落）。
        self.leaves
            .sort_by_key(|l| (l.range.start, std::cmp::Reverse(l.range.end)));
        let mut leaves: Vec<LeafRange> = Vec::with_capacity(self.leaves.len());
        for leaf in self.leaves {
            if leaves
                .last()
                .is_none_or(|last| leaf.range.start >= last.range.end)
            {
                leaves.push(leaf);
            }
        }
        Markdown {
            blocks: self.root,
            hashes,
            lines,
            ranges: self.ranges,
            leaves,
            len: source.len(),
        }
    }

    fn end_any(&mut self) {
        let tag = match self.stack.last() {
            Some(Container::Quote(..)) => TagEnd::BlockQuote(None),
            Some(Container::List(..)) => TagEnd::List(false),
            Some(Container::Item(..)) => TagEnd::Item,
            Some(Container::Footnote(..)) => TagEnd::FootnoteDefinition,
            Some(Container::Table(..)) => TagEnd::Table,
            Some(Container::Details(..)) => return self.end_details(),
            None => return,
        };
        self.end(tag);
    }
}

/// 把 `block` 里记下的原文位置改为相对于原文第 `start` 个字节（所在顶层块的起点）；
/// 将解析结果中每个 `Vec`、`String` 的容量收缩到恰好容纳其内容。
///
/// 逐项追加时容量按倍数增长，结果保留到下一次解析，多余容量一直闲置。一段文字的样式段和原文
/// 对照大多只有一两项，却按最少 4 项分配；收缩后约 1 MB 文档的解析结果由约 15 MB 降至约 6 MB。
fn shrink(blocks: &mut Vec<Block>) {
    blocks.shrink_to_fit();
    for block in blocks {
        match block {
            Block::Paragraph(inline) | Block::Heading(_, inline) => shrink_inline(inline),
            Block::Code { lang, text, map } => {
                lang.shrink_to_fit();
                text.shrink_to_fit();
                map.0.shrink_to_fit();
            }
            Block::Quote(_, blocks) | Block::Footnote(_, blocks) => shrink(blocks),
            Block::List(_, items) => {
                items.shrink_to_fit();
                for item in items {
                    shrink(&mut item.blocks);
                }
            }
            Block::Table(aligns, rows, _) => {
                aligns.shrink_to_fit();
                rows.shrink_to_fit();
                for row in rows {
                    row.shrink_to_fit();
                    row.iter_mut().for_each(shrink_inline);
                }
            }
            Block::Math(tex, _) => tex.shrink_to_fit(),
            Block::Rule(_) => {}
            Block::Details {
                summary, blocks, ..
            } => {
                shrink_inline(summary);
                shrink(blocks);
            }
        }
    }
}

fn shrink_inline(inline: &mut Inline) {
    inline.text.shrink_to_fit();
    inline.runs.shrink_to_fit();
    inline.links.shrink_to_fit();
    inline.images.shrink_to_fit();
    inline.map.0.shrink_to_fit();
}

/// 给标题生成锚点的规则：转成小写，空格换成 `-`，去掉其他标点（保留 `-`、`_`）。
pub fn slug(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c == ' ' {
                Some('-')
            } else if c.is_alphanumeric() || c == '-' || c == '_' {
                Some(c)
            } else {
                None
            }
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// 把只有 `[TOC]`（或 `[[TOC]]`，不分大小写）的段落换成目录：按文中标题的层级嵌套的列表，每一项
/// 链接到对应的标题（锚点见 [`slug`]）。`blocks` 是正文的顶层块，不含脚注。
///
/// 目录随标题生成，标题改变时所在块的内容随之改变；没有标题时保留原段落。
fn contents(blocks: &mut [Block]) {
    let is_marker = |block: &Block| {
        matches!(block, Block::Paragraph(inline)
            if matches!(inline.text.trim().to_ascii_lowercase().as_str(), "[toc]" | "[[toc]]"))
    };
    if !blocks.iter().any(is_marker) {
        return;
    }
    let headings: Vec<(u8, String)> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Heading(level, inline) if !inline.text.trim().is_empty() => {
                Some((*level, inline.text.clone()))
            }
            _ => None,
        })
        .collect();
    if headings.is_empty() {
        return;
    }
    let list = Block::List(None, contents_items(&headings));
    for block in blocks.iter_mut().filter(|block| is_marker(block)) {
        *block = list.clone();
    }
}

/// 把按先后排列的（级别，标题）排成嵌套的列表项：级别比前一项高的标题排在前一项下面。
fn contents_items(headings: &[(u8, String)]) -> Vec<Item> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < headings.len() {
        let (level, text) = &headings[i];
        // 级别更深的后续标题都是这一项的子项。
        let end = headings[i + 1..]
            .iter()
            .position(|(l, _)| l <= level)
            .map_or(headings.len(), |k| i + 1 + k);
        let mut inline = Inline::default();
        inline.links.push(format!("#{}", slug(text)));
        let link = u16::try_from(inline.links.len()).unwrap_or(u16::MAX);
        inline.push(
            text,
            Style {
                link,
                ..Style::default()
            },
        );
        let mut blocks = vec![Block::Paragraph(inline)];
        if end > i + 1 {
            blocks.push(Block::List(None, contents_items(&headings[i + 1..end])));
        }
        items.push(Item {
            task: None,
            task_source: 0,
            blocks,
            bare: false,
        });
        i = end;
    }
    items
}

/// 行内文字的对照再改为相对于该段文字最靠前的位置（见 [`Inline::source`]）。
fn rebase(block: &mut Block, start: usize) {
    let inline = |inline: &mut Inline| {
        if let Some(first) = inline.map.first() {
            inline.source = first.saturating_sub(start);
            inline.map.rebase(first);
        } else {
            inline.source = inline.source.saturating_sub(start);
        }
    };
    let blocks = |blocks: &mut Vec<Block>| blocks.iter_mut().for_each(|b| rebase(b, start));
    match block {
        Block::Paragraph(i) | Block::Heading(_, i) => inline(i),
        Block::Code { map, .. } => map.rebase(start),
        Block::Quote(_, b) | Block::Footnote(_, b) => blocks(b),
        Block::List(_, items) => items.iter_mut().for_each(|item| {
            item.task_source = item.task_source.saturating_sub(start);
            blocks(&mut item.blocks);
        }),
        Block::Table(_, rows, _) => rows.iter_mut().flatten().for_each(inline),
        Block::Details {
            summary, blocks: b, ..
        } => {
            inline(summary);
            blocks(b);
        }
        Block::Math(_, source) | Block::Rule(Some(source)) => {
            *source = source.saturating_sub(start);
        }
        Block::Rule(None) => {}
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// 中日韩文字和全角标点。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F)
}

fn alert(kind: BlockQuoteKind) -> Alert {
    match kind {
        BlockQuoteKind::Note => Alert::Note,
        BlockQuoteKind::Tip => Alert::Tip,
        BlockQuoteKind::Important => Alert::Important,
        BlockQuoteKind::Warning => Alert::Warning,
        BlockQuoteKind::Caution => Alert::Caution,
    }
}

fn align(align: Alignment) -> Align {
    match align {
        Alignment::None => Align::None,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
    }
}

mod autolink;
pub mod cells;
mod emoji;
pub mod enter;
pub mod format;
mod front;
mod html;
pub mod math;
pub mod mermaid;
pub mod reveal;
#[cfg(test)]
mod tests;
