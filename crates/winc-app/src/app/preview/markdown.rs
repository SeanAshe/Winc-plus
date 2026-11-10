//! Markdown 预览（分栏的右半边，或只看预览时的整个编辑区）：后台解析、滚动、链接和图片。
//!
//! 同一时刻只在后台解析一个版本，输入快于解析时跳过中间的版本；可编辑的预览在解析足够快时
//! 改为绘制之前直接解析（见 `INLINE_PARSE`）。排版在绘制时进行（需要测量字宽），
//! 按块缓存，见 `view::preview::markdown`。图片读取本地文件（相对文档所在目录）；网址的图片
//! （README 中的徽章、截图）在后台用系统的 `curl` 下载到临时目录缓存，关闭联网后显示替代文字。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use winc_core::Lines;
use winc_editor::{DocumentId, PreviewKind, PreviewMode};
use winc_i18n::t;
use winc_preview::image::{self, Picture, Pixels};
use winc_preview::markdown::{self, Markdown};
use winc_preview::svg;

use super::images::system_glyphs;
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, UserEvent};
use crate::system;

mod viewer;
pub(crate) use viewer::{DiagramTool, DiagramView, MAX_ZOOM, MIN_ZOOM};

/// docx 包中的图片：Markdown 中引用的地址和图片文件的内容。
type Media = Vec<(String, Arc<[u8]>)>;

/// 后台任务的结果。
enum Done {
    /// 文档、所依据的文档版本、解析结果、解析所用的时间。
    Parsed(DocumentId, u64, Markdown, std::time::Duration),
    /// docx 文档、所依据的文档版本、转换并解析的结果（含包中的图片）或失败的原因。
    Converted(DocumentId, u64, Result<(Markdown, Media), String>),
    /// 图片（解析后的本地路径）、读取结果。
    Loaded(PathBuf, Result<Arc<Picture>, String>),
    /// 图片、按显示大小（物理像素）绘制的像素。
    Drawn(PathBuf, Arc<Pixels>),
}

/// 一张图片的状态。
pub(crate) enum ImageSlot {
    /// 正在后台读取。
    Loading,
    /// 已读取；`pixels` 是按显示大小绘制的像素，最近绘制的在前，最多保留 [`SIZES`] 种
    /// （同一张图可能以多种大小出现，例如并排的截图和表格中的缩略图）；`drawing` 是正在
    /// 后台绘制的大小。
    Ready {
        picture: Arc<Picture>,
        pixels: Vec<Arc<Pixels>>,
        drawing: Vec<(u32, u32)>,
    },
    /// 无法读取（文件不存在或格式不支持）：显示替代文字。
    Failed,
}

/// 可编辑的预览中光标所在的叶子块（见 [`Markdown::leaves`]），该块显示原文。
///
/// 原文位置均按当前文字计算：解析之后输入的文字都位于该块内（光标在该块中），块的末尾
/// 按文字长度的变化移动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveLeaf {
    /// 所在的顶层块（`Markdown::blocks` 的序号）。
    pub block: usize,
    /// 所在顶层块当前的范围（字节偏移）。
    pub range: std::ops::Range<usize>,
    /// 叶子块当前的范围（字节偏移）；光标位于其后的空行上时延伸到光标所在行，位于其前的
    /// 块标记上时从光标所在行起算。
    pub leaf: std::ops::Range<usize>,
    /// 叶子块在解析结果中的终点（字节偏移）：输入之后、重新解析之前与 `leaf` 的终点不同。
    pub parsed_end: usize,
    /// 文档版本号：输入文字后该块需要重排。
    pub revision: u64,
    /// 是否为正文（段落、标题），见 `LeafRange::prose`。
    pub prose: bool,
    /// 是否为表格；表格按单元格编辑。
    pub table: bool,
    /// 查找该块所用的位置（字节偏移）。
    ///
    /// 通常是光标位置；按住鼠标期间是按下之前的光标位置。表格中该位置所在的单元格显示原文，
    /// 正文中显示该位置接触到的标记。
    pub at: usize,
    /// 光标位于代码块、公式等叶子块的第一行或最后两行时，光标所在的行号。
    ///
    /// 围栏行只在光标位于其上时显示，因此光标进出这些行时需要重排；在中间各行移动时不重排。
    pub edge: Option<usize>,
}

/// 可编辑的预览在界面线程上解析的时间预算：60 Hz 下一帧（约 16.7 ms）的四分之一。
///
/// 上一次解析不超过该预算时，输入后在绘制之前直接解析，同一帧即按最新的结构排版；超过时
/// 在后台解析，解析完成之前按上一版的结构显示。
const INLINE_PARSE: std::time::Duration = std::time::Duration::from_millis(4);

/// 一张图片最多保留的已绘制尺寸数（种）；超出时移除最早绘制的尺寸。
const SIZES: usize = 4;

/// 预览左侧的标题指示条上可以指向的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocTarget {
    /// 指示条本身：鼠标停在上面时展开标题列表。
    Bars,
    /// 标题列表里的一项，值为 [`MarkdownPreview::toc_tops`] 的序号：点击后滚动到该标题。
    Heading(usize),
}

/// 当前标签的 Markdown 预览。
/// 可横向滚动的表格的滚动条，见 [`MarkdownPreview::table_bar`]。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TableBar {
    /// 表格：（顶层块序号，块中表格的序号）。
    pub key: (usize, usize),
    /// 滚动条的轨道（窗口坐标）：表格的显示区域，滑块位于其底部。
    pub track: winc_render::Rect,
    /// 表格的总宽度（逻辑像素）。
    pub content: f32,
}

pub(crate) struct MarkdownPreview {
    /// 预览的文档；当前标签改变时重新开始。
    pub doc: Option<DocumentId>,
    /// 最近一次解析的结果。
    pub parsed: Option<Arc<Markdown>>,
    /// 最新解析过的文档版本号（`revision`）。
    checked: Option<u64>,
    /// 正在后台解析的版本号（`revision`）。
    pending: Option<u64>,
    /// 最近一次解析当前文档所用的时间（含拼接整段文字）；尚未解析过时为 `None`。
    ///
    /// 决定可编辑的预览是否在界面线程上解析，见 [`INLINE_PARSE`]。
    pub(crate) cost: Option<std::time::Duration>,
    /// 各文档预览的滚动位置（逻辑像素），切换到其他标签再切回时保持不变。
    scrolls: HashMap<DocumentId, f32>,
    /// 排版结果，见 `view::markdown`。
    pub layout: Option<crate::view::preview::markdown::Laid>,
    /// 排版缓存，见 `view::markdown::Memo`。
    pub memo: crate::view::preview::markdown::Memo,
    /// 用到的图片，键为解析后的本地路径。
    pub images: HashMap<PathBuf, ImageSlot>,
    /// docx 包中的图片：键与 `images` 相同（按文档所在目录解析后的路径），值是图片文件的
    /// 内容；读取这些图片时用这里的内容，不读磁盘。
    embedded: HashMap<PathBuf, Arc<[u8]>>,
    /// docx 读取或转换失败的原因；没有失败时为 `None`。
    pub failed: Option<String>,
    /// 图片全部作废的计数：联网设置改变时作废全部图片，整份重新排版。
    ///
    /// 每作废一次加 1。
    pub images_version: u64,
    /// 刚读取完成（或读取失败）的图片：只重排用到这些图片的块，无需整份重排。
    pub loaded: Vec<PathBuf>,
    /// 被点击后开合状态与原文相反的折叠块（`<details>`），见 `view::markdown` 的 `DETAILS`。
    pub toggled: HashSet<u64>,
    /// 刚点击的折叠块标题的地址：只重排其所在的块。
    pub toggled_now: Option<String>,
    /// 点击 `其他.md#标题` 打开的文档与目标标题：该文档排版完成后滚动到该标题。
    pub pending_anchor: Option<(DocumentId, String)>,
    /// 上一帧绘制的链接（区域和地址），点击时据此查找。
    pub links: Vec<(winc_render::Rect, String)>,
    /// 上一帧绘制了滚动条的可横向滚动的表格；没有时为 `None`。
    ///
    /// 同一时刻只显示一个表格的滚动条：鼠标所在的表格，或最近一次横向滚动的表格。
    pub table_bar: Option<TableBar>,
    /// 最近一次横向滚动的表格：（顶层块序号，块中表格的序号），见 `view::markdown` 的
    /// `Laid::hscroll`。
    pub last_table: Option<(usize, usize)>,
    /// 上一帧绘制的标题指示条和标题列表的可点击位置，见 `view::markdown` 的 `draw_toc`。
    pub toc: Vec<(winc_render::Rect, TocTarget)>,
    /// 上一帧标题指示条列出的各标题在内容里的高度（逻辑像素），与 [`TocTarget::Heading`] 的
    /// 序号对应；点击标题时滚动到该处。
    pub toc_tops: Vec<f32>,
    /// 标题列表展开时淡入的时钟；不在淡入中时为 `None`。
    pub toc_fade: Option<crate::app::animation::Clock>,
    /// 在标题列表中点击过的标题（`toc_tops` 的序号）和点击后的滚动位置。
    ///
    /// 靠近文末的标题无法滚动到预览顶部，按滚动位置无法判定其为当前一节；点击之后、再次滚动
    /// 之前，指示条将其视为当前所在的一节。
    pub toc_picked: Option<(usize, f32)>,
    /// 上一帧标题指示条标出的当前所在一节（`toc_tops` 的序号）；没有时为 `None`。
    pub toc_current: Option<usize>,
    /// 点击过的标题在正文中的底色：标题的序号与淡出的时钟。
    pub toc_flash: Option<(usize, crate::app::animation::Clock)>,
    /// 跳转到的脚注或脚注引用在正文中的底色：范围（上边缘和高度，相对内容顶部，逻辑像素）与
    /// 淡出的时钟。
    pub anchor_flash: Option<((f32, f32), crate::app::animation::Clock)>,
    /// 上一帧绘制的代码块（屏幕坐标）：鼠标移入、移出时重绘，以显示或隐藏复制按钮。
    pub codes: Vec<winc_render::Rect>,
    /// 上一帧绘制了复制按钮的代码块的内容；按钮在 `links` 中只记录 `COPY`，点击时才取这里的
    /// 文字。
    ///
    /// 与排版结果共用同一段文字：鼠标停在代码块上时每帧都要绘制按钮，无需每帧复制整块。
    pub copy: Option<std::sync::Arc<str>>,
    /// 上一帧绘制了全屏按钮的图的源码。
    ///
    /// `links` 中的按钮只记录 `EXPAND`，点击时从这里取源码，绘制时不必复制源码。
    pub expand: Option<std::sync::Arc<str>>,
    /// 全屏查看中的图；未打开全屏查看时为 `None`。
    pub diagram: Option<DiagramView>,
    /// 鼠标所在代码块在 `codes` 中的序号。
    pub code_hover: Option<usize>,
    /// 上一帧的绘制区域（分栏的右半边或整个编辑区）；不显示时为空矩形。
    pub area: winc_render::Rect,
    /// 上一帧排版的总高度（逻辑像素），滚动时据此限制范围。
    pub height: f32,
    /// 选中的文字：起点和终点（整份文字里的字符偏移，见 `view::markdown::Laid::text`）。
    pub selection: Option<(usize, usize)>,
    /// 最近一次点击是否在预览上：为真时分栏中的 ⌘C、⌘A、⌘F 作用于预览；点击左侧文字后
    /// 恢复为作用于文字。
    pub focused: bool,
    /// 右键点中的字在原文中的位置（字节偏移），供「在源代码中显示」使用；点在没有对应原文的
    /// 位置或解析结果不是当前版本时为 `None`。
    pub menu_source: Option<usize>,
    /// 查找用的文字：预览的整段文字装入一个不显示的文档，查找条按普通文档的方式查找。
    ///
    /// 重新排版时作废，需要查找时重新装入。
    pub find_doc: Option<winc_editor::Document>,
    /// `find_doc` 的装入次数：每次装入使用新的文档标识，原有的查找结果随之作废。
    find_serial: u64,
    /// 分栏时上一次两侧对齐的状态：文档、左侧文字的滚动行、预览的总高度。
    ///
    /// 左侧滚动（滚轮、光标移出屏幕、输入）或预览重排后，下一帧按左侧重新对齐右侧；
    /// 在右侧滚动时直接对齐左侧，并记录该状态。
    pub synced: Option<(DocumentId, f64, f32)>,
    /// 可编辑的预览中拖动选择文字的状态。
    ///
    /// 依次为起点（字符偏移，双击时为起始词的范围）、是否按词扩展、
    /// 按下之前光标的原文位置（字节偏移，见 [`App::live_leaf`]）。
    live_drag: Option<((usize, usize), bool, Option<usize>)>,
    /// 可编辑的预览中 ↑↓ 移动的目标横坐标（相对内容左边）和移动后光标的位置（字符偏移）：
    /// 光标仍在该位置时继续按该横坐标移动，经过短行后仍能回到原来的列。
    goal: Option<(f32, usize)>,
    /// 可编辑的预览中按 End 移到折行处时光标的位置（字符偏移）和文档版本号。
    ///
    /// 折行处的原文位置同时是上一行的末尾和下一行的开头；光标仍在该位置、文字未修改时画在
    /// 上一行的末尾，见 [`App::live_upstream`]。
    line_end: Option<(usize, u64)>,
    /// 下一次重排时各块滑动到新位置（见 [`BlockMotion`]）：表格工具增删行列后置位。
    ///
    /// 输入文字引起的重排不滑动，内容直接到位。
    pub slide: bool,
    /// 可编辑的预览中上一帧的文档、光标位置和文档版本号：发生变化表示正在编辑，见
    /// [`App::live_editing`]。
    followed: Option<(DocumentId, usize, u64)>,
    /// 在可编辑的预览中编辑过的文档：光标所在的块显示原文。
    editing: Option<DocumentId>,
    /// 刚切换为只看预览的文档：排版完成后预览滚动到文字原来的滚动位置，见
    /// [`App::align_markdown_once`]。
    align: Option<DocumentId>,
    /// 重排后的过渡动画：各块从原位置滑动到新位置，新显示的标记淡入，见 [`BlockMotion`]。
    pub motion: Option<BlockMotion>,
    /// 测试中固定的 [`BlockMotion`] 已用时间，用于截取动画中间帧。
    #[cfg(test)]
    pub frozen: Option<std::time::Duration>,
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
}

/// 淡入的项：所在顶层块的序号及块内各项的序号，按块的序号排序。
pub(crate) type Fading = Vec<(usize, Box<[usize]>)>;

/// 可编辑的预览重排后的过渡动画。
///
/// 显示原文的块改变外观或表格删除一行后，各顶层块从原绘制位置滑动到新位置，高度变化的块按
/// 过渡中的高度裁剪；仅移动光标（未修改文字）时，新显示的标记和围栏淡入。输入文字时不淡入，
/// 否则每输入一个字符，标记都会闪烁一次。
pub(crate) struct BlockMotion {
    /// 过渡动画的时钟。
    pub clock: crate::app::Clock,
    /// 各顶层块（按新排版的序号）起始时相对新位置向下的偏移（逻辑像素，向上时为负）。
    pub from: Vec<f32>,
    /// 淡入的项。
    pub fading: Fading,
}

impl BlockMotion {
    /// 返回第 `block` 块当前相对新位置向下的偏移（逻辑像素）。
    pub fn offset(&self, block: usize) -> f32 {
        self.from
            .get(block)
            .map_or(0.0, |d| d * (1.0 - self.clock.progress()))
    }

    /// 是否仍有块未到达新位置。
    pub fn moving(&self) -> bool {
        self.from.iter().any(|d| d.abs() >= 0.5)
    }

    /// 返回第 `block` 块中淡入的项。
    pub fn fading(&self, block: usize) -> &[usize] {
        self.fading
            .binary_search_by_key(&block, |(b, _)| *b)
            .map_or(&[], |i| &self.fading[i].1)
    }
}

impl Default for MarkdownPreview {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            doc: None,
            parsed: None,
            checked: None,
            pending: None,
            cost: None,
            scrolls: HashMap::new(),
            layout: None,
            table_bar: None,
            last_table: None,
            memo: Default::default(),
            images: HashMap::new(),
            embedded: HashMap::new(),
            failed: None,
            images_version: 0,
            loaded: Vec::new(),
            toggled: HashSet::new(),
            toggled_now: None,
            pending_anchor: None,
            links: Vec::new(),
            toc: Vec::new(),
            toc_tops: Vec::new(),
            toc_fade: None,
            toc_picked: None,
            toc_current: None,
            toc_flash: None,
            anchor_flash: None,
            codes: Vec::new(),
            copy: None,
            expand: None,
            diagram: None,
            code_hover: None,
            area: winc_render::Rect::default(),
            height: 0.0,
            selection: None,
            focused: false,
            menu_source: None,
            find_doc: None,
            find_serial: 0,
            synced: None,
            live_drag: None,
            goal: None,
            line_end: None,
            slide: false,
            followed: None,
            editing: None,
            align: None,
            motion: None,
            #[cfg(test)]
            frozen: None,
            sender,
            receiver,
        }
    }
}

impl MarkdownPreview {
    /// 返回当前文档预览的滚动位置（逻辑像素）。
    pub fn scroll(&self) -> f32 {
        self.doc
            .and_then(|id| self.scrolls.get(&id))
            .copied()
            .unwrap_or(0.0)
    }

    /// 滚动到 `y`（逻辑像素），限制在内容范围内。
    pub fn scroll_to(&mut self, y: f32) {
        let max = (self.height - self.area.h).max(0.0);
        if let Some(id) = self.doc {
            self.scrolls.insert(id, y.clamp(0.0, max));
        }
    }

    /// 丢弃已读取的图片（联网设置改变时调用）：下次绘制时重新排版并重新读取。
    pub fn forget_images(&mut self) {
        self.images.clear();
        self.images_version += 1;
    }

    /// 将预览切换为文档 `doc`（不再显示预览时为 `None`）。
    ///
    /// 释放上一个文档的解析结果、排版和图片，再次显示时重新解析和排版；滚动位置保留。约 1 MB 的
    /// 文档，这些合计约 10 MB；单张解码后的图片最多约 26 MB（2560 × 2560 RGBA），
    /// 网络图片再次预览时从磁盘缓存读取。
    fn reset(&mut self, doc: Option<DocumentId>) {
        self.doc = doc;
        self.parsed = None;
        self.checked = None;
        self.pending = None;
        self.cost = None;
        self.layout = None;
        self.memo = Default::default();
        self.find_doc = None;
        self.motion = None;
        self.forget_images();
        self.embedded.clear();
        self.failed = None;
        self.loaded.clear();
    }
}

/// 光标所在的围栏代码块或 `$$` 公式，见 [`App::live_fenced`]。
struct Fenced {
    /// 开围栏所在的行号。
    first: usize,
    /// 该块最后一行的行号：已闭合时为闭围栏。
    last: usize,
    /// 是否有闭围栏。
    closed: bool,
    /// 开围栏是否只有围栏字符，不带语言名。
    bare: bool,
    /// 内容是否为空（没有行，或各行均为空白）。
    empty: bool,
    /// 内容各行的行号。
    content: std::ops::Range<usize>,
}

/// 返回第 `line` 行末尾（换行符之前）的位置（字符偏移）。
fn line_end(rope: &ropey::Rope, line: usize) -> usize {
    let text = rope.line(line);
    let ending = text
        .chars_at(text.len_chars())
        .reversed()
        .take_while(|c| matches!(c, '\n' | '\r'))
        .count();
    rope.line_to_char(line) + text.len_chars() - ending
}

/// `lines` 中的各行结束后是否处于围栏代码块内。
///
/// 依据 `CommonMark` 统计开、闭围栏：行首最多 3 个空格，之后为至少 3 个反引号或波浪号；
/// 闭围栏与开围栏字符相同、长度不短于开围栏，且其后没有其他字符。
fn in_fence<'a>(lines: impl Iterator<Item = ropey::RopeSlice<'a>>) -> bool {
    let mut open: Option<(u8, usize)> = None;
    for line in lines {
        // 一行在 rope 的同一块里时不复制。
        let text = std::borrow::Cow::<str>::from(line);
        let text = text.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches(' ');
        if text.len() - trimmed.len() > 3 {
            continue;
        }
        let Some(&mark) = trimmed
            .as_bytes()
            .first()
            .filter(|b| matches!(b, b'`' | b'~'))
        else {
            continue;
        };
        let run = trimmed.bytes().take_while(|&b| b == mark).count();
        if run < 3 {
            continue;
        }
        match open {
            None if mark == b'~' || !trimmed[run..].contains('`') => open = Some((mark, run)),
            Some((m, n)) if m == mark && run >= n && trimmed[run..].trim().is_empty() => {
                open = None
            }
            _ => {}
        }
    }
    open.is_some()
}

/// 返回当前标签显示 Markdown 预览的区域（分栏的右半边或整个编辑区）；不显示时返回 `None`。
pub(crate) fn shown_area(app: &App) -> Option<winc_render::Rect> {
    let doc = app.editor.active()?;
    // docx 没有源代码可切换，总是以整个编辑区显示阅读视图。
    if doc.unsupported == Some(winc_editor::Unsupported::Docx) {
        return Some(app.layout.editor);
    }
    if doc.preview_kind() != Some(PreviewKind::Markdown) {
        return None;
    }
    match doc.preview_mode() {
        PreviewMode::Split if app.layout.split.w > 0.0 => Some(app.layout.split),
        PreviewMode::Preview => Some(app.layout.editor),
        _ => None,
    }
}

impl App {
    /// 当前标签显示 Markdown 预览且解析结果落后于文字时，在后台解析最新的文字。
    ///
    /// 不再显示预览时释放预览占用的内存，见 [`MarkdownPreview::reset`]。
    ///
    /// 每次绘制前调用。
    pub(crate) fn advance_markdown(&mut self) {
        let shown = shown_area(self).is_some();
        let Some(doc) = self.editor.active().filter(|_| shown) else {
            if self.markdown.doc.is_some() {
                self.markdown.reset(None);
            }
            return;
        };
        let (id, revision) = (doc.id, doc.revision);
        let md = &mut self.markdown;
        if md.doc != Some(id) {
            md.reset(Some(id));
        }
        if md.checked == Some(revision) {
            return;
        }
        if self.parse_markdown_inline() || self.markdown.pending.is_some() {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let md = &mut self.markdown;
        if doc.unsupported == Some(winc_editor::Unsupported::Docx) {
            let Some(path) = doc.path.clone() else {
                return;
            };
            md.pending = Some(revision);
            let (sender, proxy) = (md.sender.clone(), self.proxy.clone());
            winc_editor::background::spawn(move || {
                let result = winc_preview::docx::load(&path)
                    .map(|c| (markdown::parse(&c.markdown), c.media));
                if sender.send(Done::Converted(id, revision, result)).is_ok()
                    && let Some(proxy) = proxy
                {
                    let _ = proxy.send_event(UserEvent::ImageReady);
                }
            });
            return;
        }
        // 克隆 rope 只增加一个引用；拼接整段文字（16 MB 的文件需要数毫秒）在后台进行。
        let Some(rope) = doc.text().rope().cloned() else {
            return;
        };
        md.pending = Some(revision);
        let (sender, proxy) = (md.sender.clone(), self.proxy.clone());
        winc_editor::background::spawn(move || {
            let started = std::time::Instant::now();
            let text = rope.to_string();
            let parsed = markdown::parse(&text);
            let done = Done::Parsed(id, revision, parsed, started.elapsed());
            if sender.send(done).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 可编辑的预览中解析结果落后于文字时，在界面线程上解析最新的文字，返回是否已解析。
    ///
    /// 只在上一次解析不超过 [`INLINE_PARSE`] 时进行：输入、点击、移动光标之后的排版和按键
    /// 处理都依据当前文字的结构，不显示按上一版结构推算的中间帧。正在后台解析的旧版本作废。
    pub(crate) fn parse_markdown_inline(&mut self) -> bool {
        if !self.markdown_live() || self.markdown.cost.is_none_or(|c| c > INLINE_PARSE) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let (id, revision) = (doc.id, doc.revision);
        let md = &mut self.markdown;
        if md.doc != Some(id) {
            return false;
        }
        if md.checked == Some(revision) {
            return true;
        }
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let started = std::time::Instant::now();
        let parsed = markdown::parse(&rope.to_string());
        md.cost = Some(started.elapsed());
        md.pending = None;
        md.checked = Some(revision);
        md.parsed = Some(Arc::new(parsed));
        true
    }

    /// 接收后台完成的解析结果和图片。
    pub(crate) fn poll_markdown(&mut self) {
        let mut changed = false;
        while let Ok(done) = self.markdown.receiver.try_recv() {
            let md = &mut self.markdown;
            match done {
                Done::Parsed(id, revision, parsed, cost) => {
                    if md.doc != Some(id) || md.pending != Some(revision) {
                        continue;
                    }
                    md.cost = Some(cost);
                    md.pending = None;
                    md.checked = Some(revision);
                    md.parsed = Some(Arc::new(parsed));
                }
                Done::Converted(id, revision, result) => {
                    if md.doc != Some(id) || md.pending != Some(revision) {
                        continue;
                    }
                    md.pending = None;
                    md.checked = Some(revision);
                    md.forget_images();
                    md.embedded.clear();
                    match result {
                        Ok((parsed, media)) => {
                            let doc = self.editor.active().and_then(|d| d.path.as_deref());
                            md.embedded = media
                                .into_iter()
                                .filter_map(|(url, bytes)| {
                                    Some((image_path(doc, &url, false)?, bytes))
                                })
                                .collect();
                            md.failed = None;
                            md.parsed = Some(Arc::new(parsed));
                        }
                        Err(reason) => {
                            md.failed = Some(reason);
                            md.parsed = None;
                        }
                    }
                }
                Done::Loaded(path, result) => {
                    // 读取期间图片已作废（切换了文档或联网设置变化），丢弃结果。
                    if !matches!(md.images.get(&path), Some(ImageSlot::Loading)) {
                        continue;
                    }
                    let slot = match result {
                        Ok(picture) => ImageSlot::Ready {
                            picture,
                            pixels: Vec::new(),
                            drawing: Vec::new(),
                        },
                        Err(_) => ImageSlot::Failed,
                    };
                    md.images.insert(path.clone(), slot);
                    md.loaded.push(path);
                }
                Done::Drawn(path, drawn) => {
                    if let Some(ImageSlot::Ready {
                        pixels, drawing, ..
                    }) = md.images.get_mut(&path)
                    {
                        let size = (drawn.width, drawn.height);
                        drawing.retain(|&d| d != size);
                        pixels.retain(|p| (p.width, p.height) != size);
                        pixels.insert(0, drawn);
                        pixels.truncate(SIZES);
                    }
                }
            }
            changed = true;
        }
        if changed {
            self.request_redraw();
        }
    }

    /// 请求图片 `path`（文件中写的地址为 `url`）：尚未读取时交给后台读取，网址的图片先下载。
    pub(crate) fn request_markdown_image(&mut self, path: &Path, url: &str) {
        if self.markdown.images.contains_key(path) {
            return;
        }
        self.markdown
            .images
            .insert(path.to_path_buf(), ImageSlot::Loading);
        let (sender, proxy, path) = (
            self.markdown.sender.clone(),
            self.proxy.clone(),
            path.to_path_buf(),
        );
        let remote = is_remote(url).then(|| url.to_owned());
        let embedded = self.markdown.embedded.get(&path).cloned();
        let load = move || {
            let result = match (&remote, embedded) {
                (_, Some(bytes)) if image::is_svg(&path) => std::str::from_utf8(&bytes)
                    .map_err(|e| e.to_string())
                    .and_then(|text| svg::load(text, None, Some(system_glyphs()))),
                (_, Some(bytes)) => image::load_bytes(&bytes),
                (Some(url), None) => download(url, &path).and_then(|()| load_downloaded(&path)),
                (None, None) if image::is_svg(&path) => std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| svg::load(&text, path.parent(), Some(system_glyphs()))),
                (None, None) => image::load(&path),
            };
            if sender
                .send(Done::Loaded(path, result.map(Arc::new)))
                .is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        };
        // 网络图片由 `curl` 下载，最长等待 `REMOTE_TIMEOUT`；
        // 在单独的线程上等待，不占用共用的线程池。
        if is_remote(url) {
            winc_editor::background::spawn_blocking(load);
        } else {
            winc_editor::background::spawn(load);
        }
    }

    /// 请求把图片 `path` 按 `size`（物理像素）绘制：已有该尺寸的像素时不提交。
    ///
    /// 每张图同一时刻只在后台绘制一个尺寸：拖动改变窗口大小时每帧尺寸都不同，正在绘制时不提交，
    /// 当前尺寸完成后界面重绘，届时仍缺的尺寸再提交。
    pub(crate) fn request_markdown_pixels(&mut self, path: &Path, size: (u32, u32)) {
        let Some(ImageSlot::Ready {
            picture,
            pixels,
            drawing,
        }) = self.markdown.images.get_mut(path)
        else {
            return;
        };
        if pixels.iter().any(|p| (p.width, p.height) == size) || !drawing.is_empty() {
            return;
        }
        drawing.push(size);
        let (sender, proxy, picture, path) = (
            self.markdown.sender.clone(),
            self.proxy.clone(),
            Arc::clone(picture),
            path.to_path_buf(),
        );
        winc_editor::background::spawn(move || {
            let pixels = Arc::new(picture.render(size.0, size.1));
            if sender.send(Done::Drawn(path, pixels)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 返回预览上 `(x, y)` 处的链接序号（见 [`MarkdownPreview::links`]）。
    pub(crate) fn markdown_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.markdown.area.contains(x, y) {
            return None;
        }
        self.markdown
            .links
            .iter()
            .position(|(rect, _)| rect.contains(x, y))
    }

    /// 第 `index` 个链接是否为预览自身的控件（代码块的复制按钮、图的全屏按钮、折叠块的标题、
    /// 勾选框），而非正文中的链接。
    pub(crate) fn markdown_link_is_control(&self, index: usize) -> bool {
        use crate::view::preview::markdown::{COPY, DETAILS, EXPAND, TABLE, TASK};
        self.markdown.links.get(index).is_some_and(|(_, url)| {
            [COPY, DETAILS, EXPAND, TABLE, TASK]
                .iter()
                .any(|prefix| url.starts_with(prefix))
        })
    }

    /// 处理预览中链接的点击：网址交给浏览器，`#标题` 滚动到该标题，本地文件在编辑器中打开。
    pub(crate) fn open_markdown_link(&mut self, index: usize) {
        let Some((_, url)) = self.markdown.links.get(index).cloned() else {
            return;
        };
        if url == crate::view::preview::markdown::EXPAND {
            self.open_diagram_view();
            return;
        }
        if url == crate::view::preview::markdown::COPY {
            let Some(code) = self.markdown.copy.clone() else {
                return;
            };
            self.set_clipboard(code.to_string(), false);
            self.flash(t!("preview-markdown-code-copied"));
            return;
        }
        if let Some(op) = url.strip_prefix(crate::view::preview::markdown::TABLE) {
            self.live_table_tool(op);
            return;
        }
        if let Some(at) = url.strip_prefix(crate::view::preview::markdown::TASK) {
            if let Ok(at) = at.parse() {
                self.toggle_markdown_task(at);
            }
            return;
        }
        if let Some(id) = url.strip_prefix(crate::view::preview::markdown::DETAILS) {
            if let Ok(id) = u64::from_str_radix(id, 16)
                && !self.markdown.toggled.remove(&id)
            {
                self.markdown.toggled.insert(id);
            }
            self.markdown.toggled_now = Some(url);
            return;
        }
        if let Some(anchor) = url.strip_prefix('#') {
            self.jump_to_anchor(anchor);
            return;
        }
        let local = self
            .editor
            .active()
            .and_then(|d| d.path.as_deref())
            .and_then(|path| local_target(path, &url));
        match local {
            Some(path) if path.is_file() => self.open_markdown_file(&path, &url),
            Some(_) => self.flash(t!("preview-markdown-link-file-not-found")),
            None => {
                if let Err(e) = system::open_url(&url) {
                    self.flash(t!(
                        "preview-markdown-link-open-failed",
                        error = e.to_string()
                    ));
                }
            }
        }
    }

    /// 切换原文 `at` 处（字节偏移）的任务列表勾选框：`[ ]` 与 `[x]` 互换。
    ///
    /// 修改可以撤销；光标和选区不变，可编辑的预览也不因此进入编辑状态。
    /// 该处不是勾选框（解析结果落后于文字）时不修改。
    fn toggle_markdown_task(&mut self, at: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let mark = rope
            .get_byte_slice(at..at.saturating_add(3).min(rope.len_bytes()))
            .map(|s| s.to_string());
        let Some(mark) = mark else {
            return;
        };
        let next = match mark.as_str() {
            "[ ]" => "x",
            "[x]" | "[X]" => " ",
            _ => return,
        };
        let (id, pos) = (doc.id, rope.byte_to_char(at) + 1);
        let (selections, primary) = doc.selections();
        let editing = self.markdown.editing == Some(id);
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(pos, pos + 1);
        }
        self.run(winc_editor::Command::Insert(next.to_owned()));
        if let Some(doc) = self.editor.active_mut() {
            // 替换的只有一个字符，其前后的位置均不变。
            doc.set_selections(selections, primary);
        }
        if !editing {
            self.markdown.editing = None;
            self.markdown.followed = self
                .live_caret_source()
                .zip(self.editor.active())
                .map(|(source, d)| (d.id, source, d.revision));
        }
        self.request_redraw();
    }

    /// 在编辑器中打开预览里链接指向的本地文件。
    ///
    /// 目标同为 Markdown 时沿用当前的显示方式（分栏或只看预览）；`其他.md#标题` 在排版完成后
    /// 滚动到该标题。
    fn open_markdown_file(&mut self, path: &Path, url: &str) {
        let mode = self
            .editor
            .active()
            .map(winc_editor::Document::preview_mode);
        let anchor = url
            .split_once('#')
            .map(|(_, anchor)| anchor.to_owned())
            .filter(|a| !a.is_empty());
        self.open_path_then(path, move |app| {
            let Some(doc) = app.editor.active() else {
                return;
            };
            let id = doc.id;
            if let Some(mode) = mode
                && doc.can_preview()
                && doc.preview_mode() == PreviewMode::Source
            {
                app.set_preview_mode(mode);
            }
            if let Some(anchor) = anchor {
                app.markdown.pending_anchor = Some((id, anchor));
            }
        });
    }

    /// 排版完成后，若有属于该文档的待跳转标题（见 [`App::open_markdown_file`]），滚动到该标题。
    pub(crate) fn scroll_to_pending_anchor(&mut self) {
        let md = &mut self.markdown;
        let Some((_, anchor)) = md.pending_anchor.take_if(|(id, _)| md.doc == Some(*id)) else {
            return;
        };
        match crate::view::preview::markdown::anchor_top(self, &anchor) {
            Some(y) => self.scroll_markdown_to(y),
            None => self.flash(t!("preview-markdown-heading-not-found")),
        }
    }

    /// 键盘焦点在预览上时（只看预览，或分栏时点击了右侧）按键滚动预览，返回是否处理了该键。
    ///
    /// ↑↓ 滚动一小段，`PageUp` / `PageDown`、空格（加 ⇧ 向上）滚动一屏，Home / End、
    /// ⌘↑ / ⌘↓ 滚动到开头、末尾；Esc 清除选区。分栏时在预览上输入、回车、删除：焦点回到
    /// 左侧的文字，按普通编辑处理。可编辑的预览中按键作用于文档，见 [`App::live_key`]。
    pub(in crate::app) fn markdown_key(&mut self, key: &Key, cmd: bool) -> bool {
        let Some(area) = shown_area(self) else {
            return false;
        };
        if self.markdown_live() {
            return self.live_key(key, cmd, area);
        }
        if !self.markdown_focused() {
            return false;
        }
        if *key == Key::Named(NamedKey::Escape) {
            if self.markdown.selection.take().is_some() {
                self.request_redraw();
                return true;
            }
            return false;
        }
        let split = area != self.layout.editor;
        let typing = matches!(
            key,
            Key::Character(c) if c != " "
        ) || matches!(
            key,
            Key::Named(NamedKey::Enter | NamedKey::Backspace | NamedKey::Delete | NamedKey::Tab)
        );
        if split && typing {
            self.markdown.focused = false;
            self.markdown.selection = None;
            return false;
        }
        let step = self.fonts.line_height * 2.0;
        let page = (area.h * 0.9).max(step);
        let back = self.modifiers.shift_key();
        let y = self.markdown.scroll();
        let to = match key {
            Key::Named(NamedKey::ArrowUp) if cmd => 0.0,
            Key::Named(NamedKey::ArrowDown) if cmd => f32::MAX,
            Key::Named(NamedKey::ArrowUp) => y - step,
            Key::Named(NamedKey::ArrowDown) => y + step,
            Key::Named(NamedKey::PageUp) => y - page,
            Key::Named(NamedKey::PageDown) => y + page,
            Key::Named(NamedKey::Space) if back => y - page,
            Key::Named(NamedKey::Space) => y + page,
            Key::Character(c) if c == " " && back => y - page,
            Key::Character(c) if c == " " => y + page,
            Key::Named(NamedKey::Home) => 0.0,
            Key::Named(NamedKey::End) => f32::MAX,
            _ => return false,
        };
        self.scroll_markdown_to(to);
        true
    }

    /// 键盘与查找当前是否作用于预览排版后的文字（选中、复制、查找）。
    ///
    /// 分栏时取决于最近一次点击是否在预览上。只看预览时预览可以直接编辑，键盘作用于文档
    /// （见 [`App::markdown_live`]）；只读文档仍然作用于预览。
    pub(crate) fn markdown_focused(&self) -> bool {
        shown_area(self).is_some_and(|a| {
            if a == self.layout.editor {
                !self.markdown_live()
            } else {
                self.markdown.focused
            }
        })
    }

    /// 当前标签是否为可直接编辑的 Markdown 预览（只看预览）。
    pub(crate) fn markdown_live(&self) -> bool {
        shown_area(self).is_some_and(|a| a == self.layout.editor)
            && self
                .editor
                .active()
                .is_some_and(|d| d.edits_text() && !d.readonly)
    }

    /// 可编辑的预览中文字光标的原文位置（字节偏移，按当前文字计算）。
    pub(crate) fn live_caret_source(&self) -> Option<usize> {
        let rope = self.editor.active()?.text().rope()?;
        let head = self.editor.active()?.selection.head;
        Some(rope.char_to_byte(head.min(rope.len_chars())))
    }

    /// 返回可编辑的预览中光标所在的叶子块，见 [`LiveLeaf`]；尚未解析或没有叶子块时为 `None`。
    ///
    /// 光标在第一个叶子块之前时取第一个。拖动选择文字时取按下处（选区的起点）所在的块：
    /// 拖到其他块上时该块不切换回排版，鼠标下方的内容不移位。
    pub(crate) fn live_leaf(&self) -> Option<LiveLeaf> {
        let doc = self.editor.active()?;
        let parsed = self.markdown.parsed.as_ref()?;
        let rope = doc.text().rope()?;
        let len = rope.len_bytes();
        // 按住鼠标期间按按下之前的光标查找，显示原文的块和标记保持不变，鼠标下方的文字不移动；
        // 否则按下时该块切换为原文，文字移位，单击会变成选中一段。未开始编辑时全部按排版显示。
        let caret = match self.markdown.live_drag {
            Some((.., before)) if self.live_dragging() => before?,
            _ => self.live_caret_source()?,
        };
        let delta = len as i64 - parsed.len as i64;
        let shift = |at: usize| usize::try_from(at as i64 + delta).unwrap_or(0).min(len);
        let i = parsed.leaves.partition_point(|l| l.range.start <= caret);
        let line = rope.byte_to_line(caret);
        let line_start = rope.line_to_byte(line);
        let line_end = rope
            .try_line_to_byte(line + 1)
            .map_or(len, |b| b.saturating_sub(1).max(caret))
            .min(len);
        let block_of = |at: usize| {
            parsed
                .ranges
                .iter()
                .rposition(|r| r.start <= at && at < r.end.max(r.start + 1))
        };
        let prev = i.checked_sub(1).map(|k| &parsed.leaves[k]);
        let next = parsed.leaves.get(i);
        let (found, leaf, gap) = match (prev.map(|l| &l.range), next.map(|l| &l.range)) {
            (Some(p), _) if caret <= shift(p.end).max(p.start) => (p, p.start..shift(p.end), false),
            // 光标在块的标记上（引用的 `>`、提示块的 `[!NOTE]`、列表的序号），尚未到达其第一个
            // 叶子块：取该叶子块，从光标所在行显示原文。
            (_, Some(n)) if block_of(n.start).is_some() && block_of(n.start) == block_of(caret) => {
                (n, line_start.min(n.start)..shift(n.end).max(n.start), false)
            }
            // 光标在叶子块之后的空行上：范围延伸到光标所在行，使光标在原文的空行上有位置。
            (Some(p), _) => (p, p.start..line_end, true),
            (None, Some(n)) => (n, n.start..shift(n.end).max(n.start), false),
            (None, None) => return None,
        };
        let kind = prev
            .into_iter()
            .chain(next)
            .find(|l| std::ptr::eq(&l.range, found));
        // 块之间的空行视为正文：回车另起一段。
        let prose = gap || kind.is_some_and(|l| l.prose);
        let table = !gap && kind.is_some_and(|l| l.table);
        let block = block_of(found.start)?;
        let range = parsed.ranges[block].start..shift(parsed.ranges[block].end).max(leaf.end);
        // 输入之后、重新解析之前，范围来自上一版文档：起点可能已超出文档，也可能不在字符边界上；
        // 先将范围限制在当前文档内，之后才能按范围截取原文。
        let snap = |at: usize| rope.char_to_byte(rope.byte_to_char(at.min(len)));
        let leaf = snap(leaf.start)..snap(leaf.end).max(snap(leaf.start));
        let range = snap(range.start)..snap(range.end).max(snap(range.start));
        let last = rope.byte_to_line(leaf.end.min(len));
        let edge =
            (!prose && !table && (line <= rope.byte_to_line(leaf.start) || line + 1 >= last))
                .then_some(line);
        Some(LiveLeaf {
            block,
            range,
            leaf,
            parsed_end: found.end,
            revision: doc.revision,
            prose,
            table,
            at: caret,
            edge,
        })
    }

    /// Markdown 中选中一段文字（一行之内）后粘贴网址 `content`：返回替换选区的链接
    /// `[文字](网址)`；不应生成链接时为 `None`，按普通粘贴处理。
    ///
    /// 以下情况不生成链接：选中的文字本身是网址（视为替换网址）、选区位于代码块或行内代码中
    /// （其中的链接语法不生效）、有多个选区。
    pub(in crate::app) fn markdown_link_paste(&self, content: &str) -> Option<String> {
        let doc = self.editor.active()?;
        if doc.preview_kind() != Some(PreviewKind::Markdown) || !doc.extra_selections().is_empty() {
            return None;
        }
        let url = content.trim();
        let is_url = |s: &str| {
            ["https://", "http://", "mailto:"]
                .iter()
                .any(|p| s.len() > p.len() && s.starts_with(p))
                && !s.contains(char::is_whitespace)
        };
        if !is_url(url) {
            return None;
        }
        let rope = doc.text().rope()?;
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let selected = rope.slice(from..to).to_string();
        if selected.is_empty() || selected.contains(['\n', '`']) || is_url(selected.trim()) {
            return None;
        }
        // 位于行内代码中：该行选区之前的反引号数量为奇数。
        let (from, line) = (rope.char_to_byte(from), rope.char_to_line(from));
        let before = rope.byte_slice(rope.line_to_byte(line)..from).to_string();
        if before.matches('`').count() % 2 == 1 {
            return None;
        }
        // 判断是否位于代码块、公式、HTML 块中时使用当前版本的解析结果；没有该结果时
        // （未显示预览或尚未解析到当前版本）只检查围栏代码块，统计之前的围栏行。
        if self.markdown.checked == Some(doc.revision)
            && let Some(parsed) = self.markdown.parsed.as_ref()
        {
            let i = parsed.leaves.partition_point(|l| l.range.start <= from);
            if let Some(leaf) = i.checked_sub(1).map(|i| &parsed.leaves[i])
                && leaf.range.contains(&from)
                && !leaf.prose
            {
                return None;
            }
        } else if in_fence(rope.lines_at(0).take(line)) {
            return None;
        }
        // 网址含括号时用尖括号包裹，避免括号被视为链接的结尾。
        let url = if url.contains(['(', ')']) {
            format!("<{url}>")
        } else {
            url.to_owned()
        };
        Some(format!("[{selected}]({url})"))
    }

    /// 是否正在可编辑的预览上拖动选择文字。
    fn live_dragging(&self) -> bool {
        self.drag == crate::app::Drag::MarkdownText && self.markdown.live_drag.is_some()
    }

    /// 可编辑的预览中 ↑↓ 按显示行移动光标（加 ⇧ 扩展选区），`PageUp` / `PageDown` 移动一屏；
    /// 正文里的回车、⇧ 回车见 [`App::live_enter`]。
    ///
    /// 其余的键返回 `false`，交给编辑区按普通编辑处理（输入、删除、←→、⌘↑↓ 等都作用于文档）。
    fn live_key(&mut self, key: &Key, cmd: bool, area: winc_render::Rect) -> bool {
        self.parse_markdown_inline();
        // macOS 上 ⌘← / ⌘→ 与 Home / End 相同，移到显示行的开头、末尾。
        if cfg!(target_os = "macos") && cmd && !self.modifiers.alt_key() {
            match key {
                Key::Named(NamedKey::ArrowLeft) => return self.live_line_edge(false),
                Key::Named(NamedKey::ArrowRight) => return self.live_line_edge(true),
                _ => {}
            }
        }
        // 代码块、公式中 ⌘↩ 在块之后另起一段。
        if cmd
            && !self.modifiers.alt_key()
            && !self.modifiers.shift_key()
            && *key == Key::Named(NamedKey::Enter)
            && self.live_leave_fenced()
        {
            return true;
        }
        // 表格中 ⌘↩ 在下方插入一行，⌘⇧⌫ 删除当前行。
        if cmd && !self.modifiers.alt_key() && self.live_leaf().is_some_and(|l| l.table) {
            let shift = self.modifiers.shift_key();
            match key {
                Key::Named(NamedKey::Enter) if !shift => return self.live_table_tool("row"),
                Key::Named(NamedKey::Backspace) if shift => {
                    return self.live_table_tool("delete-row");
                }
                _ => {}
            }
        }
        if cmd || self.modifiers.alt_key() {
            return false;
        }
        use winc_preview::markdown::cells::Step;
        let shift = self.modifiers.shift_key();
        match key {
            Key::Named(NamedKey::Tab) => {
                return self.live_table_step(if shift { Step::Previous } else { Step::Next })
                    || self.live_list_shift(shift);
            }
            Key::Named(NamedKey::Enter) if !shift && self.live_table_step(Step::Down) => {
                return true;
            }
            Key::Named(NamedKey::Enter) => return self.live_enter(shift),
            Key::Named(NamedKey::Backspace) => return self.live_backspace(),
            Key::Named(NamedKey::Delete) => return self.live_delete(),
            Key::Named(NamedKey::Home) => return self.live_line_edge(false),
            Key::Named(NamedKey::End) => return self.live_line_edge(true),
            Key::Named(NamedKey::ArrowLeft) => return self.live_horizontal(false),
            Key::Named(NamedKey::ArrowRight) => return self.live_horizontal(true),
            Key::Named(NamedKey::Space) if !shift && self.live_space() => return true,
            Key::Character(c) if c == " " && !shift && self.live_space() => return true,
            Key::Character(c) if (c == "`" || c == "~") && self.live_fence_char(c) => {
                return true;
            }
            _ => {}
        }
        let page = (area.h * 0.9).max(self.fonts.line_height);
        let (down, by) = match key {
            Key::Named(NamedKey::ArrowUp) => (false, None),
            Key::Named(NamedKey::ArrowDown) => (true, None),
            Key::Named(NamedKey::PageUp) => (false, Some(page)),
            Key::Named(NamedKey::PageDown) => (true, Some(page)),
            _ => return false,
        };
        self.live_vertical(down, by);
        true
    }

    /// 重排后开始过渡动画（见 [`BlockMotion`]）。
    ///
    /// 各块从偏移 `from` 处滑动到新位置，`fading` 中的项淡入。
    ///
    /// 没有需要移动或淡入的内容、或系统开启减弱动态效果时不开始动画，直接绘制新的排版。
    pub(crate) fn start_markdown_motion(&mut self, from: Vec<f32>, fading: Fading) {
        let still = from.iter().all(|d| d.abs() < 0.5);
        if still && fading.is_empty() {
            self.markdown.motion = None;
            return;
        }
        #[cfg(test)]
        if let Some(at) = self.markdown.frozen {
            let clock = crate::app::Clock::frozen(at, crate::app::animation::MARKDOWN_MOTION);
            self.markdown.motion = Some(BlockMotion {
                clock,
                from,
                fading,
            });
            return;
        }
        self.markdown.motion = self
            .clock(crate::app::animation::MARKDOWN_MOTION)
            .map(|clock| BlockMotion {
                clock,
                from,
                fading,
            });
    }

    /// 执行表格工具栏或右键菜单「表格」中的操作 `op`。
    ///
    /// `op` 可用的值见 `view::preview::markdown` 的 `table_tools`。修改规则见
    /// `winc_preview::markdown::cells::change`，`delete` 删除整张表，一次撤销即可恢复。
    /// 光标不在表格中、或操作无效（删除表头、删除最后一列）时返回 `false`。
    pub(crate) fn live_table_tool(&mut self, op: &str) -> bool {
        use winc_preview::markdown::Align;
        use winc_preview::markdown::cells::{self, Change};
        let Some(live) = self.live_leaf().filter(|l| l.table) else {
            return false;
        };
        let Some(caret) = self.live_caret_source() else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let leaf = live.leaf.start..live.leaf.end.min(rope.len_bytes());
        let text = rope.byte_slice(leaf.clone()).to_string();
        let change = match op {
            "row-above" => Change::RowAbove,
            "row" => Change::RowBelow,
            "column-left" => Change::ColumnLeft,
            "column" => Change::ColumnRight,
            "delete-row" => Change::DeleteRow,
            "delete-column" => Change::DeleteColumn,
            "left" => Change::Align(Align::Left),
            "center" => Change::Align(Align::Center),
            "right" => Change::Align(Align::Right),
            "none" => Change::Align(Align::None),
            "delete" => {
                // 删除表格后面的换行，避免留下空行。
                let end = text.trim_end().len();
                let (new, at) = (text[end..].trim_start_matches('\n').to_owned(), 0);
                return self.replace_live_text(leaf.start, &text, &new, at);
            }
            _ => return false,
        };
        let Some((new, at)) = cells::change(&text, caret.saturating_sub(leaf.start), change) else {
            return false;
        };
        self.replace_live_text(leaf.start, &text, &new, at)
    }

    /// 将文档中从 `start` 起的原文 `old` 替换为 `new`。
    ///
    /// 只替换不同的部分，光标移到 `new` 中的 `caret` 处（字节偏移）。随后的重排中各块滑动到
    /// 新位置。
    fn replace_live_text(&mut self, start: usize, old: &str, new: &str, caret: usize) -> bool {
        self.markdown.slide = true;
        let prefix = old
            .char_indices()
            .zip(new.chars())
            .find(|((_, a), b)| a != b)
            .map_or(old.len().min(new.len()), |((i, _), _)| i);
        let suffix = old[prefix..]
            .chars()
            .rev()
            .zip(new[prefix..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum::<usize>();
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(start + prefix),
            rope.byte_to_char(start + old.len() - suffix),
        );
        doc.collapse_selections();
        doc.selection = winc_core::Selection::new(a, b);
        self.run(winc_editor::Command::Insert(
            new[prefix..new.len() - suffix].to_owned(),
        ));
        if let Some(doc) = self.editor.active_mut()
            && let Some(rope) = doc.text().rope()
        {
            let at = rope.byte_to_char((start + caret).min(rope.len_bytes()));
            doc.selection = winc_core::Selection::point(at);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 返回光标位于表格中时右键菜单的「表格」子菜单；不在表格中时为 `None`。
    pub(in crate::app) fn live_table_menu(
        &self,
    ) -> Option<Vec<crate::app::input::context_menu::MenuEntry>> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        self.live_leaf().filter(|l| l.table)?;
        let item = |op| MenuEntry::Item(MenuCommand::Table(op), true);
        Some(vec![
            item("row-above"),
            item("row"),
            item("column-left"),
            item("column"),
            MenuEntry::Separator,
            item("left"),
            item("center"),
            item("right"),
            MenuEntry::Separator,
            item("delete-row"),
            item("delete-column"),
            item("delete"),
        ])
    }

    /// 光标在表格的单元格中时，按 Tab、⇧Tab、回车切换单元格（见 `winc_preview::markdown::cells`）。
    ///
    /// 选中目标单元格的内容，随后的输入将替换该内容；在最后一行继续向下时新增一行。不在表格
    /// 中、或位于表头下方的分隔行时返回 `false`，交给编辑区处理（缩进、换行）。
    fn live_table_step(&mut self, step: winc_preview::markdown::cells::Step) -> bool {
        use winc_preview::markdown::cells;
        let Some(live) = self.live_leaf().filter(|l| l.table) else {
            return false;
        };
        let Some(caret) = self.live_caret_source() else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        if !(live.leaf.start..=live.leaf.end).contains(&caret) {
            return false;
        }
        let text = rope.byte_slice(live.leaf.clone()).to_string();
        let Some(target) = cells::step(&text, caret - live.leaf.start, step) else {
            return false;
        };
        let mut select = target.select;
        if let Some((at, row)) = target.insert {
            let before = rope.len_bytes();
            let at = live.leaf.start + at;
            if let Some(doc) = self.editor.active_mut()
                && let Some(rope) = doc.text().rope()
            {
                let pos = rope.byte_to_char(at);
                doc.collapse_selections();
                doc.selection = winc_core::Selection::point(pos);
            }
            let len = row.len();
            self.run(winc_editor::Command::Insert(row));
            // 文档的行尾为 `\r\n` 时，插入的文字比 `row` 长。
            let after = self
                .editor
                .active()
                .and_then(|d| d.text().rope())
                .map_or(before, |r| r.len_bytes());
            let grown = (after - before).saturating_sub(len);
            select = select.start + grown..select.end + grown;
        }
        if let Some(doc) = self.editor.active_mut()
            && let Some(rope) = doc.text().rope()
        {
            let char_at = |b: usize| rope.byte_to_char((live.leaf.start + b).min(rope.len_bytes()));
            let (a, b) = (char_at(select.start), char_at(select.end));
            doc.goal_cell = None;
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// ←→ 移出光标所在的块、落到块之间的空行上时，跳过这些空行，到下一块的开头或上一块的
    /// 末尾：排版后的预览不显示空行，光标不停留在不可见的位置。
    ///
    /// 后方（前方）没有块时停在空行上。落到不显示的块标记（列表记号、引用的 `>`、续行的
    /// 缩进）中时同样跳过，见 [`App::live_visible`]。其余情形返回 `false`，交给编辑区逐字移动。
    fn live_horizontal(&mut self, right: bool) -> bool {
        self.markdown.line_end = None;
        let extend = self.modifiers.shift_key();
        let Some(live) = self.live_leaf() else {
            return false;
        };
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let (sel, chars) = (doc.selection, rope.len_chars());
        if !sel.is_empty() && !extend {
            return false;
        }
        let next = match right {
            true if sel.head < chars => sel.head + 1,
            false if sel.head > 0 => sel.head - 1,
            _ => return false,
        };
        // 正文中没有空行，落到空行上即表示离开了该块；代码块中的空行是代码的一部分。
        let byte = rope.char_to_byte(next);
        let outside = live.prose
            || if right {
                byte >= live.leaf.end
            } else {
                byte < live.leaf.start
            };
        let blank = |line: usize| rope.line(line).chars().all(char::is_whitespace);
        let mut line = rope.char_to_line(next);
        let mut target = next;
        if outside && blank(line) {
            if right {
                while line < rope.len_lines() && blank(line) {
                    line += 1;
                }
                if line < rope.len_lines() {
                    target = rope.line_to_char(line);
                }
            } else {
                while blank(line) && line > 0 {
                    line -= 1;
                }
                if !blank(line) {
                    target = line_end(rope, line);
                }
            }
        }
        let target = self.live_visible(target, right);
        if target == next {
            return false;
        }
        if let Some(doc) = self.editor.active_mut() {
            doc.goal_cell = None;
            doc.collapse_selections();
            let anchor = if extend { doc.selection.anchor } else { target };
            doc.selection = winc_core::Selection::new(anchor, target);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 处理正文（段落、标题、块之间的空行）中的回车：另起一段、列表续一项、空项退出一层；
    /// 按住 ⇧ 时段内换行（见 `winc_preview::markdown::enter`）。
    ///
    /// 光标在代码块、公式、表格、HTML 块中时返回 `false`，按编辑区的方式换行并保持缩进。
    /// 有选区时先替换选区：光标之前的文字截至选区开头，之后的文字从选区末尾算起。
    fn live_enter(&mut self, shift: bool) -> bool {
        use winc_preview::markdown::enter;
        if !shift && self.live_fence() {
            return true;
        }
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let (from, to) = (rope.char_to_byte(from), rope.char_to_byte(to));
        let line_of = |at: usize| rope.byte_to_line(at);
        let line_text = |line: usize| {
            let text = rope.line(line).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let start = rope.line_to_byte(line_of(from));
        let before = rope.byte_slice(start..from).to_string();
        let to_line = line_of(to);
        let rest = line_text(to_line);
        let after = rest[(to - rope.line_to_byte(to_line)).min(rest.len())..].to_owned();
        let edit = if shift {
            enter::Edit {
                before: 0,
                after: 0,
                text: enter::line_break(&before),
            }
        } else {
            let next = (to_line + 1 < rope.len_lines()).then(|| line_text(to_line + 1));
            let first = line_of(from);
            let above: Vec<String> = (first.saturating_sub(200)..first)
                .rev()
                .map(line_text)
                .collect();
            enter::enter(
                &before,
                &after,
                next.as_deref(),
                above.iter().map(String::as_str),
            )
        };
        let (a, b) = (from - edit.before, (to + edit.after).min(rope.len_bytes()));
        // 有序列表中续出一项：其后同一层各项的序号顺延，与新增的一项在同一次修改中完成，
        // 撤销一步即可恢复。
        let item = edit.text.rsplit('\n').next().unwrap_or_default();
        let below = (to_line + 1..rope.len_lines()).map(line_text);
        let changes = if shift || !edit.text.contains('\n') {
            Vec::new()
        } else {
            enter::renumber(item, below)
        };
        let mut text = edit.text.clone();
        let mut end = b;
        for (i, range, number) in changes {
            let start = rope.line_to_byte(to_line + 1 + i);
            text.push_str(&rope.byte_slice(end..start + range.start).to_string());
            text.push_str(&number.to_string());
            end = start + range.end;
        }
        let (a, b) = (rope.byte_to_char(a), rope.byte_to_char(end));
        // 光标停在新增的一项之后；换行符在插入时按文档的行尾风格转换，长度随之变化。
        let caret = (text.len() > edit.text.len()).then(|| {
            let ending = doc.line_ending.as_str().chars().count();
            let breaks = edit.text.matches('\n').count();
            a + edit.text.chars().count() + breaks * (ending - 1)
        });
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(text));
        if let Some(caret) = caret
            && let Some(doc) = self.editor.active_mut()
        {
            doc.selection = winc_core::Selection::point(caret);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 补全闭围栏：在围栏行（```` ```rust ````、`$$`）末尾按回车时，光标移到两行围栏之间的空行。
    ///
    /// 规则见 `winc_preview::markdown::enter::fence`。光标位于代码块中间的行、有选区、
    /// 或当前行不是围栏时返回 `false`。刚输入的围栏可能已解析为未闭合的代码块，
    /// 也可能仍是正文段落，因此两种情况都按原文判断。
    fn live_fence(&mut self) -> bool {
        let leaf = self.live_leaf();
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(caret);
        if leaf.is_some_and(|l| !l.prose && rope.byte_to_line(l.leaf.start) != line) {
            return false;
        }
        let byte = rope.char_to_byte(caret);
        let start = rope.line_to_byte(line);
        let before = rope.byte_slice(start..byte).to_string();
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let after = text(line)[byte - start..].to_owned();
        if !before.contains(['`', '~', '$']) {
            return false;
        }
        let below: Vec<String> = (line + 1..rope.len_lines()).map(text).collect();
        let fence =
            winc_preview::markdown::enter::fence(&before, &after, below.iter().map(String::as_str));
        let Some((head, tail)) = fence else {
            return false;
        };
        // 分两次插入：换行符按文档的行尾风格生成，光标停在两段文字之间。
        self.run(winc_editor::Command::Insert(head));
        let at = self.editor.active().map(|d| d.selection.head);
        self.run(winc_editor::Command::Insert(tail));
        if let Some(doc) = self.editor.active_mut()
            && let Some(at) = at
        {
            doc.selection = winc_core::Selection::point(at);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 按 Tab、⇧Tab 调整列表项的层级：缩进为上一项的子项，或退出一层。
    ///
    /// 规则见 `winc_preview::markdown::enter::shift`，光标和选区随该行移动，子项和续行随该项
    /// 移动。不是列表项、有多个光标或选区跨行时返回 `false`，交给编辑区按普通缩进处理。
    fn live_list_shift(&mut self, outdent: bool) -> bool {
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let chars = rope.len_chars();
        let sel = doc.selection;
        let line = rope.char_to_line(sel.head.min(chars));
        if !doc.extra_selections().is_empty() || rope.char_to_line(sel.anchor.min(chars)) != line {
            return false;
        }
        let line_text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let above: Vec<String> = (line.saturating_sub(200)..line)
            .rev()
            .map(line_text)
            .collect();
        let text = line_text(line);
        let Some((range, indent)) =
            winc_preview::markdown::enter::shift(&text, above.iter().map(String::as_str), outdent)
        else {
            return false;
        };
        if text[range.clone()] == indent {
            return true;
        }
        // 替换的范围只含空格、Tab 和有序列表的序号，字节数等于字符数。
        let start = rope.line_to_char(line);
        let (a, b) = (start + range.start, start + range.end);
        let moved = |c: usize| {
            if c >= b {
                (c + indent.len()).saturating_sub(range.len())
            } else {
                c.min(a + indent.len())
            }
        };
        let after = winc_core::Selection::new(moved(sel.anchor), moved(sel.head));
        // 子项和续行（缩进多于该项的各行）随该项移动同样的宽度，层级关系不变。
        use winc_preview::markdown::enter::margins;
        let (quote, old) = margins(&text);
        let new = indent.len() - indent.trim_start_matches([' ', '\t']).len();
        let mut lines = vec![format!(
            "{}{indent}{}",
            &text[..range.start],
            &text[range.end..]
        )];
        let (mut last, mut blanks) = (line, 0);
        for l in line + 1..rope.len_lines() {
            let child = line_text(l);
            if child.trim().is_empty() {
                blanks += 1;
                continue;
            }
            let (q, i) = margins(&child);
            if q != quote || i <= old {
                break;
            }
            lines.extend(std::iter::repeat_n(String::new(), blanks));
            blanks = 0;
            let (head, tail) = child.split_at(q);
            lines.push(if new >= old {
                format!("{head}{}{tail}", " ".repeat(new - old))
            } else {
                format!("{head}{}", &tail[(old - new).min(i)..])
            });
            last = l;
        }
        let (a, b, indent) = if last == line {
            (a, b, indent)
        } else {
            (start, line_end(rope, last), lines.join("\n"))
        };
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(indent));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = after;
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文中的删除键：移除光标前的一层块标记（列表记号、任务框、引用的 `>`、标题的 `#`）。
    ///
    /// 规则见 `winc_preview::markdown::enter::backspace`。光标位于块的开头时删到上一块的末尾，
    /// 两段正文合并为一段。有选区、有多个光标、位于代码块等块中、或光标前还有文字时返回
    /// `false`，按普通删除处理。
    fn live_backspace(&mut self) -> bool {
        if let Some(done) = self.live_fenced_backspace() {
            return done;
        }
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let line = rope.byte_to_line(caret);
        let start = rope.line_to_byte(line);
        let before = rope.byte_slice(start..caret).to_string();
        let Some(edit) = winc_preview::markdown::enter::backspace(&before) else {
            // 光标位于块的开头：预览中看不到光标前的缩进和块之间的空行，逐个删除时光标
            // 看起来不动。一次删到上一行的末尾；隔着空行时与上一块合并。
            if line == 0 || !before.trim().is_empty() {
                return false;
            }
            let blank = |l: usize| rope.line(l).chars().all(char::is_whitespace);
            let to = rope.byte_to_char(caret);
            let from = if !blank(line - 1) {
                // 同一段的续行：没有缩进时按普通删除合并两行。
                if before.is_empty() {
                    return false;
                }
                line_end(rope, line - 1)
            } else {
                let Some(above) = (0..line).rev().find(|&l| !blank(l)) else {
                    return false;
                };
                let from = line_end(rope, above);
                let Some(prose) = self.live_prose_at(rope.char_to_byte(from)) else {
                    return false;
                };
                // 上一块是代码块、表格等时不合并：光标所在行有文字时只移动光标，移到上一块
                // 内容的末尾（闭围栏的上一行、最后一个单元格的文字之后）。
                if !prose && !blank(line) {
                    let last = rope.line(above).to_string();
                    let last = last.trim_end();
                    let from = if above > 0 && ["```", "~~~", "$$"].contains(&last.trim_start()) {
                        line_end(rope, above - 1)
                    } else if let Some(cell) = last.strip_suffix('|') {
                        from - (last.chars().count() - cell.trim_end().chars().count())
                    } else {
                        from
                    };
                    if let Some(doc) = self.editor.active_mut() {
                        doc.selection = winc_core::Selection::point(from);
                    }
                    self.markdown.goal = None;
                    self.start_live_editing();
                    self.request_redraw();
                    return true;
                }
                from
            };
            if let Some(doc) = self.editor.active_mut() {
                doc.selection = winc_core::Selection::new(from, to);
            }
            self.run(winc_editor::Command::Insert(String::new()));
            self.markdown.goal = None;
            self.start_live_editing();
            return true;
        };
        let (a, b) = (
            rope.byte_to_char(caret - edit.before),
            rope.byte_to_char(caret),
        );
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(edit.text));
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文末尾的向前删除键：光标位于块的最后一行末尾、其后隔着空行时，与下一块合并。
    ///
    /// 预览中不显示块之间的空行，逐个删除换行符时内容不变化。下一块不是正文（代码块、表格等）
    /// 时不合并，光标移到下一块的开头。有选区、有多个光标、光标不在行尾、或解析结果不是当前
    /// 版本时返回 `false`，按普通删除处理。
    fn live_delete(&mut self) -> bool {
        if self.live_fenced_delete() {
            return true;
        }
        if self.live_leaf().is_none_or(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let head = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let blank = |l: usize| rope.line(l).chars().all(char::is_whitespace);
        if head != line_end(rope, line) || line + 1 >= rope.len_lines() || !blank(line + 1) {
            return false;
        }
        let Some(below) = (line + 1..rope.len_lines()).find(|&l| !blank(l)) else {
            return false;
        };
        let to = rope.line_to_char(below) + self.live_hidden_prefix(below);
        let Some(prose) = self.live_prose_at(rope.char_to_byte(to)) else {
            return false;
        };
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = if prose {
                winc_core::Selection::new(head, to)
            } else {
                winc_core::Selection::point(to)
            };
        }
        if prose {
            self.run(winc_editor::Command::Insert(String::new()));
        }
        self.markdown.goal = None;
        self.start_live_editing();
        self.request_redraw();
        true
    }

    /// 处理正文中输入的空格：行首的任务框简写（`[]`、`-[]`、`- [`）换成任务项 `- [ ] `。
    ///
    /// 规则见 `winc_preview::markdown::enter::task`。不是简写、有选区、有多个光标、输入法
    /// 正在组字、或光标位于代码块等块中时返回 `false`，按普通输入处理。
    fn live_space(&mut self) -> bool {
        if !self.preedit.is_empty() || self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let line = rope.byte_to_line(caret);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        let at = (caret - rope.line_to_byte(line)).min(text.len());
        let Some(edit) = winc_preview::markdown::enter::task(&text[..at], &text[at..]) else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(caret - edit.before),
            rope.byte_to_char(caret + edit.after),
        );
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(edit.text));
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文中输入的 `` ` ``、`~`（`mark`）：输入后该行成为未闭合的开围栏时，立即在下一行
    /// 补上闭围栏，光标留在开围栏之后，可以继续输入语言名。
    ///
    /// 未闭合的围栏会使其后的全部内容成为代码，补全之前整篇的外观都会改变。不构成开围栏、
    /// 下方已有闭围栏、有选区、或光标位于代码块等块中时返回 `false`，按普通输入处理。
    fn live_fence_char(&mut self, mark: &str) -> bool {
        if !self.preedit.is_empty() || self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let line = rope.byte_to_line(caret);
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let current = text(line);
        let at = (caret - rope.line_to_byte(line)).min(current.len());
        let typed = format!("{}{mark}", &current[..at]);
        // 只在恰好输入第三个字符时补全：更长的围栏由回车补全。
        if typed.len() - typed.trim_end_matches(mark).len() != 3 * mark.len() {
            return false;
        }
        let below: Vec<String> = (line + 1..rope.len_lines()).map(text).collect();
        let below = below.iter().map(String::as_str);
        let fence = winc_preview::markdown::enter::fence(&typed, &current[at..], below);
        let Some((_, tail)) = fence else {
            return false;
        };
        self.run(winc_editor::Command::Insert(mark.to_owned()));
        let at = self.editor.active().map(|d| d.selection.head);
        self.run(winc_editor::Command::Insert(tail));
        if let Some(doc) = self.editor.active_mut()
            && let Some(at) = at
        {
            doc.selection = winc_core::Selection::point(at);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 返回光标所在的围栏代码块或 `$$` 公式各行的位置；光标不在这类块中时为 `None`。
    fn live_fenced(&self) -> Option<Fenced> {
        let live = self.live_leaf().filter(|l| !l.prose && !l.table)?;
        let rope = self.editor.active()?.text().rope()?;
        let len = rope.len_bytes();
        // 引用、列表中的代码块各行带有引用标记和缩进。
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_matches([' ', '\t', '>', '\n', '\r']).to_owned()
        };
        let first = rope.byte_to_line(live.leaf.start.min(len));
        let open = text(first);
        let mark = ["```", "~~~", "$$"]
            .into_iter()
            .find(|m| open.starts_with(m))?;
        let end = live
            .leaf
            .end
            .min(len)
            .saturating_sub(1)
            .max(live.leaf.start);
        let mut last = rope.byte_to_line(end);
        while last > first && text(last).is_empty() {
            last -= 1;
        }
        let close = text(last);
        let closed =
            last > first && close.starts_with(mark) && close.chars().all(|c| mark.starts_with(c));
        let content = first + 1..if closed { last } else { last + 1 };
        Some(Fenced {
            first,
            last,
            closed,
            bare: open.chars().all(|c| mark.starts_with(c)),
            empty: content.clone().all(|l| text(l).is_empty()),
            content,
        })
    }

    /// 将围栏代码块 `fenced` 的全部各行替换为 `text`，光标放在其开头。
    fn replace_fenced(&mut self, fenced: &Fenced, text: String) {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let (a, b) = (rope.line_to_char(fenced.first), line_end(rope, fenced.last));
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(text));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::point(a);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        self.request_redraw();
    }

    /// 处理围栏代码块、公式中的删除键，返回是否已处理；不属于以下情形时为 `None`。
    ///
    /// 逐字删除围栏会使代码块失去一侧的围栏，其后的全部内容成为代码。因此：
    /// - 光标在内容第一行的开头：空的代码块整块删除，有内容时不删除。
    /// - 光标在不带语言名的开围栏上：空的代码块整块删除，有内容时移除两行围栏、保留内容。
    /// - 光标在闭围栏上：空的代码块整块删除，有内容时光标移到内容的末尾。
    fn live_fenced_backspace(&mut self) -> Option<bool> {
        let fenced = self.live_fenced()?;
        let doc = self.editor.active()?;
        let rope = doc.text().rope()?;
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return None;
        }
        let head = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let col = head - rope.line_to_char(line);
        let on_close = fenced.closed && line == fenced.last;
        let on_open = line == fenced.first && fenced.bare && col > 0;
        let at_start = line == fenced.first + 1 && col == 0;
        if !on_close && !on_open && !at_start {
            return None;
        }
        if fenced.empty {
            self.replace_fenced(&fenced, String::new());
        } else if on_close {
            let end = line_end(rope, fenced.last - 1);
            if let Some(doc) = self.editor.active_mut() {
                doc.selection = winc_core::Selection::point(end);
            }
            self.request_redraw();
        } else if on_open {
            let lines: Vec<String> = fenced
                .content
                .clone()
                .map(|l| {
                    let text = rope.line(l).to_string();
                    text.trim_end_matches(['\n', '\r']).to_owned()
                })
                .collect();
            self.replace_fenced(&fenced, lines.join("\n"));
        }
        Some(true)
    }

    /// 处理围栏代码块、公式中的向前删除键，返回是否已处理。
    ///
    /// 光标在开围栏的末尾、或内容最后一行的末尾时不删除：删除其后的换行符会使内容与围栏
    /// 连成一行，代码块失去一侧的围栏。
    fn live_fenced_delete(&mut self) -> bool {
        let Some(fenced) = self.live_fenced() else {
            return false;
        };
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let head = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let before_close = fenced.closed && line + 1 == fenced.last;
        head == line_end(rope, line) && (line == fenced.first || before_close)
    }

    /// 在光标所在的围栏代码块、公式之后另起一段，光标移到新的一段；光标不在已闭合的这类
    /// 块中时返回 `false`。
    fn live_leave_fenced(&mut self) -> bool {
        let Some(fenced) = self.live_fenced().filter(|f| f.closed) else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let end = line_end(rope, fenced.last);
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::point(end);
        }
        self.run(winc_editor::Command::Insert("\n\n".to_owned()));
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 文档的最后一行属于代码块、表格、分隔线等不是正文的块时，在文末另起一段并把光标移到
    /// 该段，返回是否已处理。
    ///
    /// 文档以换行符结束时光标可以直接移到末尾的空行，不需要处理。
    fn live_append_paragraph(&mut self) -> bool {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let chars = rope.len_chars();
        if chars == 0 || rope.char(chars - 1) == '\n' {
            return false;
        }
        if self.live_prose_at(rope.len_bytes()) != Some(false) {
            return false;
        }
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::point(chars);
        }
        self.run(winc_editor::Command::Insert("\n\n".to_owned()));
        self.markdown.goal = None;
        self.markdown.line_end = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 对选区切换行内格式的标记 `mark`（`**`、`*`、`` ` ``、`~~`），规则见
    /// `winc_preview::markdown::format::toggle`。
    ///
    /// 只处理一行之内的单个选区；选区跨行、有多个光标、或光标位于代码块、公式等按原文显示的
    /// 块中时不修改。
    pub(in crate::app) fn live_format(&mut self, mark: &str) {
        self.parse_markdown_inline();
        if self.live_leaf().is_some_and(|l| !l.prose && !l.table) {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        if !doc.extra_selections().is_empty() {
            return;
        }
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let line = rope.char_to_line(from);
        if rope.char_to_line(to) != line {
            return;
        }
        let start = rope.line_to_byte(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        let local = |c: usize| (rope.char_to_byte(c) - start).min(text.len());
        let t = winc_preview::markdown::format::toggle(text, local(from)..local(to), mark);
        // 修改只涉及这一行：行首之前的字符数不变，行内的位置按修改之后的文字换算。
        let first = rope.line_to_char(line);
        let chars_to = |text: &str, byte: usize| first + text[..byte].chars().count();
        let (a, b) = (chars_to(text, t.range.start), chars_to(text, t.range.end));
        let edited = format!(
            "{}{}{}",
            &text[..t.range.start],
            t.text,
            &text[t.range.end..]
        );
        let select = (
            chars_to(&edited, t.select.start),
            chars_to(&edited, t.select.end),
        );
        let changed = a != b || !t.text.is_empty();
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        if changed {
            // 按粘贴处理：整段替换，不触发输入单个字符时的括号、引号配对。
            self.run(winc_editor::Command::Paste(t.text));
        }
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(select.0, select.1);
        }
        self.markdown.goal = None;
        self.markdown.line_end = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
    }

    /// 原文第 `at` 个字节所在的叶子块是否为正文（段落、标题）；不在任何叶子块中时为
    /// `Some(false)`，解析结果不是当前版本时为 `None`。
    fn live_prose_at(&self, at: usize) -> Option<bool> {
        let doc = self.editor.active()?;
        if self.markdown.checked != Some(doc.revision) {
            return None;
        }
        let leaves = &self.markdown.parsed.as_ref()?.leaves;
        let i = leaves.partition_point(|l| l.range.start <= at);
        Some(
            i.checked_sub(1)
                .map(|i| &leaves[i])
                .is_some_and(|l| l.prose && at <= l.range.end),
        )
    }

    /// 返回第 `line` 行开头在预览中不显示的块标记的长度（字符）。
    ///
    /// 叶子块的第一行取叶子块之前的部分（引用的 `>`、列表记号、任务框），正文的续行取引用
    /// 标记和缩进。解析结果不是当前版本时为 0。
    fn live_hidden_prefix(&self, line: usize) -> usize {
        let Some(doc) = self.editor.active() else {
            return 0;
        };
        if self.markdown.checked != Some(doc.revision) {
            return 0;
        }
        let (Some(parsed), Some(rope)) = (self.markdown.parsed.as_ref(), doc.text().rope()) else {
            return 0;
        };
        if line >= rope.len_lines() {
            return 0;
        }
        let start = rope.line_to_byte(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        let i = parsed
            .leaves
            .partition_point(|l| l.range.start <= start + text.len());
        let Some(leaf) = i.checked_sub(1).map(|i| &parsed.leaves[i]) else {
            return 0;
        };
        let hidden = if leaf.range.start >= start {
            leaf.range.start - start
        } else if leaf.prose && start <= leaf.range.end {
            winc_preview::markdown::enter::indent(text)
        } else {
            0
        };
        rope.byte_to_char((start + hidden).min(rope.len_bytes())) - rope.line_to_char(line)
    }

    /// 光标位置 `pos`（字符偏移）落在不显示的块标记中时，返回移动方向上最近的可见位置；
    /// 否则返回 `pos`。
    ///
    /// 向后（`forward`）时取该行内容的开头，向前时取上方最近一个非空行的末尾；上方没有内容时
    /// 留在该行内容的开头。
    fn live_visible(&self, pos: usize, forward: bool) -> usize {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return pos;
        };
        let line = rope.char_to_line(pos.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let hidden = self.live_hidden_prefix(line);
        if pos - start >= hidden {
            return pos;
        }
        let blank = |l: usize| rope.line(l).chars().all(char::is_whitespace);
        match (0..line).rev().find(|&l| !blank(l)) {
            Some(above) if !forward => line_end(rope, above),
            _ => start + hidden,
        }
    }

    /// 编辑区的命令移动光标之后调用：光标落在不显示的块标记中时移到可见位置。
    ///
    /// `before` 是移动之前光标的位置（字符偏移），据此判断移动方向。按词移动、移到行首等
    /// 命令按原文计算位置，结果可能落在列表记号或续行的缩进中。
    pub(in crate::app) fn live_settle_caret(&mut self, before: usize) {
        if !self.markdown_live() {
            return;
        }
        self.parse_markdown_inline();
        let Some(doc) = self.editor.active() else {
            return;
        };
        let sel = doc.selection;
        if sel.head == before || !doc.extra_selections().is_empty() {
            return;
        }
        let head = self.live_visible(sel.head, sel.head > before);
        if head == sel.head {
            return;
        }
        if let Some(doc) = self.editor.active_mut() {
            let anchor = if sel.is_empty() { head } else { sel.anchor };
            doc.selection = winc_core::Selection::new(anchor, head);
        }
        self.request_redraw();
    }

    /// 将光标移到所在显示行的开头或末尾（`end`），按住 ⇧ 时扩展选区。
    ///
    /// 正文按预览中折行后的行计算，一段原文折成多行时不会移到整段的开头、末尾。光标位于
    /// 代码块、公式等按原文显示的块中时返回 `false`，按编辑区的方式处理。
    fn live_line_edge(&mut self, end: bool) -> bool {
        if self.live_leaf().is_none_or(|l| !l.prose && !l.table) {
            return false;
        }
        let extend = self.modifiers.shift_key();
        let Some(source) = self.live_caret_source() else {
            return false;
        };
        let Some(laid) = self.markdown.layout.as_ref() else {
            return false;
        };
        let Some((_, y, h)) = laid.caret_point(source) else {
            return false;
        };
        // 横坐标取内容一栏之外：命中的是该行最左、最右的字缝。
        let x = if end {
            laid.left + laid.width() + self.markdown.area.w
        } else {
            laid.left - self.markdown.area.w
        };
        let sy = self.markdown.area.y - self.markdown.scroll() + y + h / 2.0;
        let Some(found) = crate::view::preview::markdown::source_at(self, x, sy) else {
            return false;
        };
        // 折行处的位置按下一行的开头定位：纵坐标与当前行不同时，光标应画在本行的末尾。
        laid.set_upstream(None);
        let wrapped = end
            && laid
                .caret_point(found)
                .is_some_and(|(_, fy, _)| (fy - y).abs() >= 0.5);
        if wrapped {
            laid.set_upstream(Some(found));
        }
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let pos = rope.byte_to_char(found.min(rope.len_bytes()));
        let revision = doc.revision;
        doc.goal_cell = None;
        doc.collapse_selections();
        let anchor = if extend { doc.selection.anchor } else { pos };
        doc.selection = winc_core::Selection::new(anchor, pos);
        self.markdown.line_end = wrapped.then_some((pos, revision));
        self.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 原文位置 `source` 位于折行处时，判断窗口纵坐标 `y` 是否更接近上一显示行，并让排版结果
    /// 按该结论定位光标（见 `Laid::set_upstream`）。
    ///
    /// 折行处的位置同时是上一行的末尾和下一行的开头：在上一行的右端点击、从上方按 ↓ 移到
    /// 行尾时，光标应留在上一行。
    fn live_row_affinity(&self, source: usize, y: f32) -> bool {
        let Some(laid) = self.markdown.layout.as_ref() else {
            return false;
        };
        let y = y - (self.markdown.area.y - self.markdown.scroll());
        let far = |p: Option<(f32, f32, f32)>| p.map(|(_, top, h)| (top + h / 2.0 - y).abs());
        laid.set_upstream(None);
        let below = laid.caret_point(source);
        laid.set_upstream(Some(source));
        let above = laid.caret_point(source);
        let upstream = match (far(above), far(below)) {
            (Some(a), Some(b)) => a + 0.5 < b,
            _ => false,
        };
        if !upstream {
            laid.set_upstream(None);
        }
        upstream
    }

    /// 可编辑的预览中光标应画在上一显示行末尾时，返回光标的原文位置（字节偏移），见
    /// `MarkdownPreview::line_end`。
    pub(crate) fn live_upstream(&self) -> Option<usize> {
        let doc = self.editor.active()?;
        let (pos, revision) = self.markdown.line_end?;
        (doc.selection.head == pos && doc.revision == revision)
            .then(|| self.live_caret_source())
            .flatten()
    }

    /// 将光标向下（`down`）或向上移动一个显示行，或 `by` 逻辑像素；移出第一行、最后一行时
    /// 移到文档的开头、末尾。
    fn live_vertical(&mut self, down: bool, by: Option<f32>) {
        let extend = self.modifiers.shift_key();
        let Some(source) = self.live_caret_source() else {
            return;
        };
        let Some(laid) = self.markdown.layout.as_ref() else {
            return;
        };
        let Some((x, y, h)) = laid.caret_point(source) else {
            return;
        };
        let head = self.editor.active().map_or(0, |d| d.selection.head);
        let goal = match self.markdown.goal {
            Some((goal, at)) if at == head => goal,
            _ => x,
        };
        let step = by.unwrap_or(h);
        let sign = if down { 1.0 } else { -1.0 };
        let height = laid.height();
        let screen = |laid: &crate::view::preview::markdown::Laid, v: f32| {
            (
                laid.left + goal,
                self.markdown.area.y - self.markdown.scroll() + v,
            )
        };
        // 块间留白高于一行时，下一行的目标位置落在留白中，取到的最近一行仍是原来的行；
        // 因此每次再前进半行，直到到达其他行或超出内容范围。
        let mut target = y + h / 2.0 + sign * step;
        let mut moved = None;
        let mut upstream = false;
        while (0.0..=height).contains(&target) {
            let (sx, sy) = screen(laid, target);
            let found = crate::view::preview::markdown::source_at(self, sx, sy);
            moved = found.filter(|&s| {
                upstream = self.live_row_affinity(s, sy);
                let point = self.markdown.layout.as_ref().and_then(|l| l.caret_point(s));
                point.is_none_or(|(_, py, _)| (py - y).abs() >= 0.5)
            });
            if moved.is_some() || by.is_some() {
                break;
            }
            target += sign * h / 2.0;
        }
        // 文档以代码块、表格、分隔线等结束：再向下时在其后另起一段，光标有处可去。
        if moved.is_none() && down && !extend && self.live_append_paragraph() {
            return;
        }
        // 仍在原来的行上（已是第一行或最后一行）时，移到文档的开头或末尾。
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = match moved {
            Some(s) => rope.byte_to_char(s.min(rope.len_bytes())),
            None if down => rope.len_chars(),
            None => 0,
        };
        doc.goal_cell = None;
        doc.collapse_selections();
        let anchor = if extend { doc.selection.anchor } else { pos };
        doc.selection = winc_core::Selection::new(anchor, pos);
        let revision = doc.revision;
        self.markdown.line_end = (moved.is_some() && upstream).then_some((pos, revision));
        self.markdown.goal = Some((goal, pos));
        self.markdown.selection = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
    }

    /// 处理可编辑预览上的鼠标按下：将文档的光标放到点中的字上；双击选中一个词，三击选中
    /// 原文的一行，按住 ⇧ 时将选区扩展到该处。
    fn click_live(&mut self, x: f32, y: f32) {
        // 按下之前的光标位置：未开始编辑时为 `None`，按住鼠标期间整篇按排版显示。
        let editing = self.editor.active().map(|d| d.id) == self.markdown.editing;
        let before = self.live_caret_source().filter(|_| editing);
        self.preedit.clear();
        self.markdown.selection = None;
        self.markdown.goal = None;
        self.markdown.line_end = None;
        self.request_redraw();
        let Some(source) = crate::view::preview::markdown::source_at(self, x, y) else {
            // 没有可定位的文字（空文档）：仍然进入编辑，显示光标。
            self.start_live_editing();
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.modifiers.shift_key();
        let upstream = count == 1 && self.live_row_affinity(source, y);
        // 三击选中一行时不含预览中不显示的块标记：随后输入的文字不会替换掉列表记号。
        let hidden = self
            .editor
            .active()
            .and_then(|d| d.text().rope())
            .map_or(0, |rope| {
                let line = rope.byte_to_line(source.min(rope.len_bytes()));
                self.live_hidden_prefix(line)
            });
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        self.markdown.line_end = upstream.then_some((pos, doc.revision));
        doc.goal_cell = None;
        doc.record_cursors();
        doc.collapse_selections();
        let (origin, by_word) = match count {
            2 => (winc_core::movement::word_at(doc.text(), pos), true),
            3 => {
                let line = doc.text().pos_to_line_col(pos).0;
                let span = doc.text().line_span(line);
                (
                    (span.start + hidden.min(span.end - span.start), span.end),
                    true,
                )
            }
            _ if shift => ((doc.selection.anchor, doc.selection.anchor), false),
            _ => ((pos, pos), false),
        };
        doc.selection = if by_word {
            winc_core::Selection::new(origin.0, origin.1)
        } else {
            winc_core::Selection::new(origin.0, pos)
        };
        self.markdown.live_drag = Some((origin, by_word, before));
        self.drag = crate::app::Drag::MarkdownText;
        self.start_live_editing();
        self.reset_blink();
    }

    /// 处理可编辑预览上的拖动选择：选区从按下处延伸到鼠标下方的字。
    fn drag_live(&mut self, x: f32, y: f32) {
        let Some((origin, by_word, _)) = self.markdown.live_drag else {
            return;
        };
        let Some(source) = crate::view::preview::markdown::source_at(self, x, y) else {
            return;
        };
        let upstream = !by_word && self.live_row_affinity(source, y);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        self.markdown.line_end = upstream.then_some((pos, doc.revision));
        doc.selection = if !by_word {
            winc_core::Selection::new(origin.0, pos)
        } else if pos < origin.0 {
            winc_core::Selection::new(origin.1, winc_core::movement::word_at(doc.text(), pos).0)
        } else {
            winc_core::Selection::new(origin.0, winc_core::movement::word_at(doc.text(), pos).1)
        };
        self.reset_blink();
    }

    /// 可编辑的预览在本帧是否将光标所在的块显示为原文。
    ///
    /// 在预览上点击、移动光标或输入之后才显示原文；刚打开、仅阅读时整篇均按排版显示，
    /// 不会有某一块突然切换为原文。排版之前调用。
    pub(crate) fn live_editing(&mut self) -> bool {
        if !self.markdown_live() {
            return false;
        }
        let (Some(source), Some(doc)) = (self.live_caret_source(), self.editor.active()) else {
            return false;
        };
        let now = (doc.id, source, doc.revision);
        if self
            .markdown
            .followed
            .is_some_and(|f| f.0 == doc.id && f != now)
        {
            self.markdown.editing = Some(doc.id);
        }
        self.markdown.editing == Some(doc.id)
    }

    /// 可编辑的预览中是否显示文字光标：在当前文档的预览上点击、移动光标或输入之后显示
    /// （见 [`App::live_editing`]）。
    pub(crate) fn live_caret_shown(&self) -> bool {
        let id = self.editor.active().map(|d| d.id);
        id.is_some() && self.markdown.editing == id
    }

    /// 在可编辑的预览中开始编辑（点击、移动光标）：光标所在的块显示原文。
    fn start_live_editing(&mut self) {
        self.markdown.editing = self.editor.active().map(|d| d.id);
    }

    /// 可编辑的预览中光标位置改变（输入、点击、移动、查找）后，滚动使光标可见并与边缘保留
    /// 一行的距离；光标未移动、仅滚动时不处理。
    ///
    /// 排版完成之后、绘制之前调用。
    pub(crate) fn follow_live_caret(&mut self) {
        let source = self
            .markdown_live()
            .then(|| self.live_caret_source())
            .flatten();
        let (Some(source), Some(doc)) = (source, self.editor.active()) else {
            self.markdown.followed = None;
            return;
        };
        let now = (doc.id, source, doc.revision);
        let moved = self
            .markdown
            .followed
            .is_some_and(|f| f.0 == doc.id && f != now);
        self.markdown.followed = Some(now);
        if !moved || self.markdown.editing != Some(doc.id) {
            return;
        }
        // 全选不滚动：选区的终点在文末，滚动过去会离开正在阅读的位置。
        let chars = doc.text().rope().map_or(0, ropey::Rope::len_chars);
        if doc.selection.from() == 0 && doc.selection.to() >= chars && chars > 0 {
            return;
        }
        let Some(laid) = self.markdown.layout.as_ref() else {
            return;
        };
        let Some((_, y, h)) = laid.caret_point(source) else {
            return;
        };
        let md = &mut self.markdown;
        let (scroll, view) = (md.scroll(), md.area.h);
        let margin = h.min(view / 4.0);
        if y - margin < scroll {
            md.scroll_to(y - margin);
        } else if y + h + margin > scroll + view {
            md.scroll_to(y + h + margin - view);
        }
    }

    /// 处理预览上的鼠标按下：获取焦点并从该处开始选择文字；双击选中一个词，按住 ⇧ 时扩展选区。
    ///
    /// 可编辑的预览中改为放置文档的光标，见 [`App::click_live`]。
    pub(in crate::app) fn click_markdown(&mut self, x: f32, y: f32) {
        self.focus = crate::app::Focus::Editor;
        // 内容一栏两侧的空白：分栏时将键盘焦点交给预览，但不放置光标、不开始选择，保留已有选区。
        if !crate::view::preview::markdown::in_content(self, x) {
            if !self.markdown_live() {
                self.markdown.focused = true;
                self.request_redraw();
            }
            return;
        }
        if self.markdown_live() {
            self.click_live(x, y);
            return;
        }
        self.markdown.focused = true;
        self.request_redraw();
        let Some(at) = crate::view::preview::markdown::offset_at(self, x, y) else {
            self.markdown.selection = None;
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.modifiers.shift_key();
        self.markdown.selection = match self.markdown.selection {
            _ if count >= 2 => self.markdown_word(at),
            Some((anchor, _)) if shift => Some((anchor, at)),
            _ => Some((at, at)),
        };
        self.drag = crate::app::Drag::MarkdownText;
        // 分栏时左侧的光标移到点中的字上：随后输入的文字（输入时焦点回到左侧）插入在此处。只看
        // 预览时光标不可见，不移动。
        if count == 1
            && !shift
            && shown_area(self).is_some_and(|a| a != self.layout.editor)
            && let Some(source) = self.current_source_at(x, y)
        {
            self.place_caret_at_source(source);
            // 预览不随左侧滚动：点中的字保持在鼠标下方。
            if let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active()) {
                self.markdown.synced = Some((doc.id, doc.scroll_line, laid.height()));
            }
        }
    }

    /// 文字光标在原文中的位置（字节偏移）；解析结果不是当前版本时为 `None`。
    pub(crate) fn markdown_caret_source(&self) -> Option<usize> {
        let doc = self.editor.active()?;
        if self.markdown.checked != Some(doc.revision) {
            return None;
        }
        let rope = doc.text().rope()?;
        Some(rope.char_to_byte(doc.selection.head.min(rope.len_chars())))
    }

    /// 返回预览上 `(x, y)` 处的字在原文中的位置（字节偏移）；解析结果不是当前版本时为 `None`
    /// （刚输入的文字尚未解析，位置可能已错开）。
    fn current_source_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        // docx 的预览由转换得到，位置不对应文档中的文字。
        if doc.unsupported.is_some() || self.markdown.checked != Some(doc.revision) {
            return None;
        }
        crate::view::preview::markdown::source_at(self, x, y)
    }

    /// 将文字光标放到原文第 `source` 个字节处；光标所在行不可见时滚动到距顶部三分之一处。
    fn place_caret_at_source(&mut self, source: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        doc.goal_cell = None;
        doc.record_cursors();
        doc.collapse_selections();
        doc.selection = winc_core::Selection::point(pos);
        let line = doc.text().pos_to_line_col(pos).0;
        self.scroll_to_line(line, 1.0 / 3.0, true);
        self.request_redraw();
    }

    /// 执行「在源代码中显示」：将光标放到右键点中的字在原文中的位置，键盘焦点回到文字；
    /// 只看预览时改为分栏，预览保留在旁边。
    pub(in crate::app) fn reveal_markdown_source(&mut self) {
        let Some(source) = self.markdown.menu_source.take() else {
            return;
        };
        if self.editor.active().map(|d| d.preview_mode()) == Some(PreviewMode::Preview) {
            self.set_preview_mode(PreviewMode::Split);
        }
        self.markdown.focused = false;
        self.focus = crate::app::Focus::Editor;
        self.place_caret_at_source(source);
    }

    /// 处理预览上不在链接上的右键点击，返回是否点在预览上。
    ///
    /// 预览获得焦点，点击位置在选区外时取消选区。可编辑的预览中改为将文档的光标移到该处
    /// （点在文档的选区内时不移动）。
    pub(in crate::app) fn markdown_context_target(&mut self, x: f32, y: f32) -> bool {
        if !self.markdown.area.contains(x, y) || self.markdown_link_at(x, y).is_some() {
            return false;
        }
        self.focus = crate::app::Focus::Editor;
        if self.markdown_live() {
            self.markdown.menu_source = crate::view::preview::markdown::source_at(self, x, y);
            if let Some(source) = self.markdown.menu_source
                && let Some(doc) = self.editor.active_mut()
                && let Some(rope) = doc.text().rope()
            {
                let pos = rope.byte_to_char(source.min(rope.len_bytes()));
                if !(doc.selection.from()..=doc.selection.to()).contains(&pos) {
                    doc.selection = winc_core::Selection::point(pos);
                    doc.goal_cell = None;
                }
            }
            self.request_redraw();
            return true;
        }
        self.markdown.focused = true;
        self.markdown.menu_source = self.current_source_at(x, y);
        let at = crate::view::preview::markdown::offset_at(self, x, y);
        let inside = self
            .markdown
            .selection
            .zip(at)
            .is_some_and(|((a, b), at)| (a.min(b)..=a.max(b)).contains(&at));
        if !inside {
            self.markdown.selection = None;
        }
        self.request_redraw();
        true
    }

    /// 返回双击处的词：连续的字母数字、连续的中日韩文字，或单个其他字符。
    fn markdown_word(&self, at: usize) -> Option<(usize, usize)> {
        let laid = self.markdown.layout.as_ref()?;
        const REACH: usize = 64;
        let from = at.saturating_sub(REACH);
        let text: Vec<char> = laid
            .text(from, (at + REACH).min(laid.chars()))
            .chars()
            .collect();
        let i = at - from;
        let class = |c: char| {
            if c.is_alphanumeric() && (c as u32) < 0x2E80 || c == '_' {
                1
            } else if c.is_alphanumeric() {
                2
            } else {
                0
            }
        };
        let kind = class(*text.get(i)?);
        if kind == 0 {
            return Some((at, at + 1));
        }
        let start = text[..i]
            .iter()
            .rposition(|&c| class(c) != kind)
            .map_or(0, |p| p + 1);
        let end = text[i..]
            .iter()
            .position(|&c| class(c) != kind)
            .map_or(text.len(), |p| i + p);
        Some((from + start, from + end))
    }

    /// 处理拖动选择文字：拖出上下边缘时随之滚动。
    pub(in crate::app) fn drag_markdown(&mut self, x: f32, y: f32) {
        let area = self.markdown.area;
        if y < area.y {
            self.scroll_markdown((area.y - y).min(60.0));
        } else if y > area.bottom() {
            self.scroll_markdown(-(y - area.bottom()).min(60.0));
        }
        let y = y.clamp(area.y, area.bottom() - 1.0);
        if self.markdown_live() {
            self.drag_live(x, y);
            self.request_redraw();
            return;
        }
        if let Some(at) = crate::view::preview::markdown::offset_at(self, x, y)
            && let Some((anchor, _)) = self.markdown.selection
        {
            self.markdown.selection = Some((anchor, at));
        }
        self.request_redraw();
    }

    /// 返回预览中选中的文字；没有选中时返回 `None`。
    pub(crate) fn markdown_selected_text(&self) -> Option<String> {
        let (a, b) = self.markdown.selection?;
        let text = self.markdown.layout.as_ref()?.text(a.min(b), a.max(b));
        (!text.is_empty()).then_some(text)
    }

    /// 选中预览中的全部文字（⌘A）。
    pub(crate) fn markdown_select_all(&mut self) {
        let chars = self.markdown.layout.as_ref().map_or(0, |l| l.chars());
        self.markdown.selection = Some((0, chars));
        self.request_redraw();
    }

    /// 返回查找用的预览文字（见 [`MarkdownPreview::find_doc`]），尚未装入时立即装入。
    ///
    /// 尚未排版时返回 `None`。
    pub(crate) fn markdown_find_doc(&mut self) -> Option<&winc_editor::Document> {
        if self.markdown.find_doc.is_none() {
            let text = crate::view::preview::markdown::full_text(self)?;
            let md = &mut self.markdown;
            md.find_serial += 1;
            // 与实际文档、差异所用的标识互不重叠。
            let id = DocumentId(u64::MAX - 1 - md.find_serial);
            md.find_doc = Some(winc_editor::Document::from_text(id, &text));
        }
        self.markdown.find_doc.as_ref()
    }

    /// 选中预览中第 `from` 到 `to` 个字符（查找跳转到的匹配）并滚动使其可见：已在屏幕内时
    /// 不滚动，否则滚动到距顶部三分之一屏处。
    pub(crate) fn select_in_markdown(&mut self, from: usize, to: usize) {
        self.markdown.selection = Some((from, to));
        if let Some(y) = crate::view::preview::markdown::offset_top(self, from) {
            let md = &mut self.markdown;
            let scroll = md.scroll();
            if y < scroll || y + 40.0 > scroll + md.area.h {
                let y = y - md.area.h / 3.0;
                self.scroll_markdown_to(y);
            }
        }
        self.request_redraw();
    }

    /// 在预览上滚动 `dy`（逻辑像素，向上为正，方向约定同滚轮事件）。
    /// 可横向滚动的表格 `key`（见 [`TableBar::key`]）当前的横向滚动位置（逻辑像素）。
    pub(crate) fn markdown_table_scroll(&self, key: (usize, usize)) -> f32 {
        (self.markdown.layout.as_ref()).map_or(0.0, |laid| laid.hscroll_of(key.0, key.1))
    }

    /// 拖动表格的水平滚动条：按鼠标横坐标 `x` 和按下时在滑块上的位置 `grab` 设置滚动位置。
    pub(in crate::app) fn drag_markdown_table_scrollbar(&mut self, x: f32, grab: f32) {
        let Some(bar) = self.markdown.table_bar else {
            return;
        };
        let scroll = self.markdown_table_scroll(bar.key);
        let Some(thumb) = crate::layout::hscrollbar_thumb(bar.track, scroll, bar.content) else {
            return;
        };
        let max = bar.content - bar.track.w;
        let to = crate::layout::scroll_for_thumb(bar.track.x, bar.track.w, thumb.w, x, grab, max);
        if let Some(laid) = self.markdown.layout.as_mut() {
            laid.hscroll.insert(bar.key, to.clamp(0.0, max));
        }
        self.markdown.last_table = Some(bar.key);
        self.request_redraw();
    }

    pub(crate) fn scroll_markdown(&mut self, dy: f32) {
        let y = self.markdown.scroll() - dy;
        self.scroll_markdown_to(y);
    }

    /// 处理标题指示条中第 `i` 个标题的点击：预览滚动到该标题。
    ///
    /// 滚动后将其记为当前所在的一节（见 `MarkdownPreview::toc_picked`），并在正文中为其添加
    /// 逐渐淡出的底色，标示跳转的目标。
    pub(in crate::app) fn press_toc(&mut self, target: TocTarget) {
        let TocTarget::Heading(i) = target else {
            return;
        };
        let Some(&top) = self.markdown.toc_tops.get(i) else {
            return;
        };
        self.scroll_markdown_to(top);
        self.markdown.toc_picked = Some((i, self.markdown.scroll()));
        let clock = self.clock(crate::app::animation::TOC_FLASH);
        self.markdown.toc_flash = clock.map(|clock| (i, clock));
    }

    /// 跳转到预览中的锚点 `anchor`（标题、脚注、脚注引用）；找不到时提示。
    ///
    /// 标题滚动到预览顶部。脚注和脚注引用滚动到预览上部三分之一处，上方保留一段上下文，并像
    /// 点击标题列表一样铺一层逐渐淡出的底色，标示跳转的目标：脚注集中在文末，滚动到底之后
    /// 仅凭位置看不出是哪一条。
    pub(in crate::app) fn jump_to_anchor(&mut self, anchor: &str) {
        let Some((top, height)) = crate::view::preview::markdown::anchor_box(self, anchor) else {
            self.flash(t!("preview-markdown-heading-not-found"));
            return;
        };
        let lower = anchor.to_lowercase();
        if !(lower.starts_with("fn-") || lower.starts_with("fnref-")) {
            self.scroll_markdown_to(top);
            return;
        }
        self.scroll_markdown_to(top - self.markdown.area.h / 3.0);
        let clock = self.clock(crate::app::animation::TOC_FLASH);
        self.markdown.anchor_flash = clock.map(|clock| ((top, height), clock));
    }

    /// 将预览滚动到 `y`；分栏时左侧的文字随之滚动到对应位置。
    pub(crate) fn scroll_markdown_to(&mut self, y: f32) {
        self.markdown.scroll_to(y);
        self.request_redraw();
        if shown_area(self).is_none_or(|a| a == self.layout.editor) {
            return;
        }
        let max = self.max_scroll_line();
        let y = self.markdown.scroll();
        let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active_mut())
        else {
            return;
        };
        let line = laid.line_of_y(y, doc.text().line_count());
        doc.scroll_line = text_row(doc, line).clamp(0.0, max.max(0.0));
        self.markdown.synced = Some((doc.id, doc.scroll_line, laid.height()));
    }

    /// 在切换显示方式之前调用：返回只看预览时预览顶部对应的原文行号（小数）。
    ///
    /// 切换为源代码或分栏后，文字滚动到该行（[`App::scroll_text_to`]）；不是只看预览或尚未
    /// 排版时为 `None`。切换为只看预览时，记录在排版完成后对齐一次（见
    /// [`App::align_markdown_once`]）；光标所在的块在预览上开始编辑后才显示原文。
    pub(in crate::app) fn leave_markdown_mode(&mut self, mode: PreviewMode) -> Option<f32> {
        let doc = self.editor.active()?;
        let old = doc.preview_mode();
        self.markdown.followed = None;
        self.markdown.editing = None;
        if old == mode || doc.preview_kind() != Some(PreviewKind::Markdown) {
            return None;
        }
        if mode == PreviewMode::Preview {
            self.markdown.align = Some(doc.id);
            return None;
        }
        let laid = self.markdown.layout.as_ref()?;
        (old == PreviewMode::Preview && self.markdown.doc == Some(doc.id))
            .then(|| laid.line_of_y(self.markdown.scroll(), doc.text().line_count()))
    }

    /// 滚动文字，使原文第 `line` 行（小数）位于顶部。
    pub(in crate::app) fn scroll_text_to(&mut self, line: f32) {
        let max = self.max_scroll_line();
        if let Some(doc) = self.editor.active_mut() {
            doc.scroll_line = text_row(doc, line).clamp(0.0, max.max(0.0));
        }
    }

    /// 将预览滚动到文字顶端那一行对应的位置；仅在刚切换为只显示预览后执行一次。
    ///
    /// 位置的计算使用分栏同步滚动的映射。
    ///
    /// 排版完成之后、绘制之前调用。
    pub(crate) fn align_markdown_once(&mut self) {
        let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active()) else {
            return;
        };
        if self.markdown.align != Some(doc.id) {
            return;
        }
        self.markdown.align = None;
        let y = laid.y_of_line(text_line(doc), doc.text().line_count());
        self.markdown.scroll_to(y);
    }

    /// 分栏时左侧文字滚动或预览重排后，将右侧滚动到对应位置。
    ///
    /// 绘制预览之前调用。
    pub(crate) fn sync_markdown_scroll(&mut self) {
        let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active()) else {
            return;
        };
        let now = (doc.id, doc.scroll_line, laid.height());
        if self.markdown.synced == Some(now) {
            return;
        }
        let y = laid.y_of_line(text_line(doc), doc.text().line_count());
        self.markdown.scroll_to(y);
        self.markdown.synced = Some(now);
    }
}

/// 返回文字滚动位置的第一个显示行 `doc.scroll_line` 对应的原文行号（小数：自动换行折成
/// 多段的行按段数等分）。
fn text_line(doc: &winc_editor::Document) -> f32 {
    let row = doc.scroll_line.max(0.0);
    let (line, part) = doc.line_and_part(row as usize);
    let rows = doc.rows_of(line).max(1) as f64;
    line as f32 + ((part as f64 + row.fract()) / rows) as f32
}

/// [`text_line`] 的逆运算：返回原文第 `line` 行（小数）对应的显示行。
fn text_row(doc: &winc_editor::Document, line: f32) -> f64 {
    let last = doc.text().line_count().saturating_sub(1);
    let whole = (line.max(0.0) as usize).min(last);
    let rows = doc.rows_of(whole).max(1) as f64;
    doc.row_of(whole) as f64 + f64::from((line - whole as f32).clamp(0.0, 1.0)) * rows
}

/// 网址图片的下载缓存目录，文件名为网址的哈希。
const REMOTE_CACHE: &str = "winc-markdown-images";

/// 下载一张图片的最长等待时间（秒），传给 `curl` 的 `--max-time`。
const REMOTE_TIMEOUT: &str = "20";

/// 单张图片的最大下载大小（字节，20 MB），传给 `curl` 的 `--max-filesize`。
const REMOTE_MAX: &str = "20000000";

/// `url` 是否为需要联网的地址。
fn is_remote(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// 返回文档 `doc`（未保存过的文档为 `None`）中图片地址 `url` 对应的文件：本地图片见
/// [`local_target`]；网址的图片为下载的缓存文件，`remote` 为假（不允许联网）时返回 `None`。
pub(crate) fn image_path(doc: Option<&Path>, url: &str, remote: bool) -> Option<PathBuf> {
    if is_remote(url) {
        // FNV-1a：同一网址在每次启动时都映射到同一个文件，已下载的图片无需重新下载。
        let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        // 扩展名统一按位图写，读取时按内容判断真实格式。
        return remote.then(|| {
            std::env::temp_dir()
                .join(REMOTE_CACHE)
                .join(format!("{hash:016x}.png"))
        });
    }
    local_target(doc?, url)
}

/// 缓存图片的最长未使用时间（30 天），超过时删除。
pub(in crate::app) const CACHE_AGE: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 3600);

/// 缓存目录占用的磁盘上限（字节），200 MB。
const CACHE_MAX: u64 = 200 * 1024 * 1024;

/// 把 `url` 下载到 `path`。
///
/// 已下载过时只更新修改时间（清理缓存时据此判断最久未使用的文件）。先写入同目录的临时
/// 文件，下载完成后再改名，中途失败不会留下不完整的文件。每次启动后首次下载前清理一次缓存
/// （[`clean_cache`]）。
fn download(url: &str, path: &Path) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or(t!("preview-markdown-cache-folder-invalid"))?;
    static CLEANED: std::sync::Once = std::sync::Once::new();
    CLEANED.call_once(|| clean_cache(dir, std::time::SystemTime::now()));
    if path.is_file() {
        if let Ok(file) = std::fs::File::options().write(true).open(path) {
            let _ = file.set_modified(std::time::SystemTime::now());
        }
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = path.with_extension(format!("part{}", std::process::id()));
    let mut command = std::process::Command::new("curl");
    command
        .args([
            "-fsSL",
            "--max-time",
            REMOTE_TIMEOUT,
            "--max-filesize",
            REMOTE_MAX,
            "-o",
        ])
        .arg(&part)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Windows 上 `curl.exe` 是命令行程序：不弹出控制台窗口。
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(t!("preview-markdown-download-failed").to_owned());
    }
    winc_config::files::replace_file(&part, path).map_err(|e| e.to_string())
}

/// 删除 `dir` 中超过 [`CACHE_AGE`] 未使用的图片，以及超过一小时未下载完成的临时文件；
/// 剩余文件超过 [`CACHE_MAX`] 时从最久未使用的开始删除。
pub(in crate::app) fn clean_cache(dir: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let age = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default();
        let partial = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.starts_with("part"));
        let stale = if partial {
            age > std::time::Duration::from_secs(3600)
        } else {
            age > CACHE_AGE
        };
        if stale {
            let _ = std::fs::remove_file(&path);
        } else if !partial {
            kept.push((age, meta.len(), path));
        }
    }
    let mut total: u64 = kept.iter().map(|k| k.1).sum();
    // 最久未使用的排在前面。
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, len, path) in kept {
        if total <= CACHE_MAX {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// 读取下载的图片：内容为 SVG（徽章多为 SVG）时按 SVG 读取，否则按位图读取。
///
/// SVG 中的文字（徽章上的字）用系统字体绘制，与直接预览 SVG 文件时相同。
fn load_downloaded(path: &Path) -> Result<Picture, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(4).any(|w| w == b"<svg") {
        let text = String::from_utf8_lossy(&bytes);
        return svg::load(&text, None, Some(system_glyphs()));
    }
    image::load(path)
}

/// 返回文档 `doc` 中地址 `url` 指向的本地文件；网址、`mailto:` 等返回 `None`。
///
/// 去掉 `#…`、`?…`，相对路径按文档所在目录解析，并还原 `%20` 等转义。`file://主机/路径`
/// 取其中的路径：本地文件都在编辑区中打开，不交给系统，因为系统打开 `.app`、`.command`
/// 时会运行这些文件。
pub(crate) fn local_target(doc: &Path, url: &str) -> Option<PathBuf> {
    let url = match url.strip_prefix("file://") {
        Some(rest) => &rest[rest.find('/')?..],
        None if url.is_empty()
            || url.contains("://")
            || url.starts_with("mailto:")
            || url.starts_with("data:") =>
        {
            return None;
        }
        None => url,
    };
    let path = url.split(['#', '?']).next().unwrap_or_default();
    if path.is_empty() {
        return None;
    }
    let path = percent_decode(path);
    let path = Path::new(&path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        doc.parent()?.join(path)
    })
}

/// 还原网址里 `%xx` 形式的转义；不是合法 UTF-8 时原样返回。
pub(crate) fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests;
