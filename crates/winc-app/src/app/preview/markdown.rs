//! Markdown 预览（分栏的右半边，或只看预览时的整个编辑区）：后台解析、滚动、链接和图片。
//!
//! 同一时刻只在后台解析一个版本，输入快于解析时跳过中间的版本；可编辑的预览在解析足够快时
//! 改为绘制之前直接解析（见 `INLINE_PARSE`）。排版在绘制时进行（需要测量字宽），
//! 按块缓存，见 `view::preview::markdown`。图片读取本地文件（相对文档所在目录）；网址的图片
//! （README 中的徽章、截图）在后台用系统的 `curl` 下载到临时目录缓存，关闭联网后显示替代文字。

use crate::app::ScrollTarget;
use crate::app::scroll::set_scroll_line;
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

use crate::app::{App, Focus, UserEvent};
use crate::system;

mod images;
pub(crate) mod laid;
mod live;
mod live_caret;
mod scroll;
mod viewer;
pub(crate) use viewer::{DiagramLaid, DiagramTool, DiagramView, MAX_ZOOM, MIN_ZOOM};

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
    /// 图片、未能绘制的大小（物理像素）：重新读取图片失败。
    Undrawn(PathBuf, (u32, u32)),
}

pub(crate) use winc_typeset::ImageSlot;

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
    /// 光标是否位于段落、标题之后的空行上。
    pub blank: bool,
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
    /// 上一帧的绘制区域（分栏的右半边或整个编辑区）；不显示时为空矩形。
    pub area: winc_render::Rect,
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
    /// 排版结果，见 [`laid::Laid`]。
    pub layout: Option<crate::app::preview::markdown::laid::Laid>,
    /// 排版缓存，见 `winc_typeset::Memo`。
    pub memo: crate::app::preview::markdown::laid::Memo,
    /// 用到的图片，键为解析后的本地路径。
    pub images: HashMap<PathBuf, ImageSlot>,
    /// 各图片在文件中写的地址，键同 `images`：解码后的图片释放之后据此重新读取。
    urls: HashMap<PathBuf, String>,
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
    /// 最近一次横向滚动的表格：（顶层块序号，块中表格的序号），见 `view::markdown` 的
    /// `Laid::hscroll`。
    pub last_table: Option<(usize, usize)>,
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
    /// 全屏查看中的图；未打开全屏查看时为 `None`。
    pub diagram: Option<DiagramView>,
    /// 鼠标所在代码块在 `codes` 中的序号。
    pub code_hover: Option<usize>,
    /// 上一帧排版的总高度（逻辑像素），滚动时据此限制范围。
    pub height: f32,
    /// 选中的文字：起点和终点（整份文字里的字符偏移，见 `laid::Laid::text`）。
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
            last_table: None,
            memo: Default::default(),
            images: HashMap::new(),
            urls: HashMap::new(),
            embedded: HashMap::new(),
            failed: None,
            images_version: 0,
            loaded: Vec::new(),
            toggled: HashSet::new(),
            toggled_now: None,
            pending_anchor: None,
            toc_fade: None,
            toc_picked: None,
            toc_current: None,
            toc_flash: None,
            anchor_flash: None,
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

    /// 返回最大滚动位置（逻辑像素）：内容高于预览区域的部分，不小于 0。
    pub fn max_scroll(&self) -> f32 {
        (self.height - self.area.h).max(0.0)
    }

    /// 滚动到 `y`（逻辑像素），限制在内容范围内。
    pub fn scroll_to(&mut self, y: f32) {
        self.set_scroll(y.clamp(0.0, self.max_scroll()));
    }

    /// 把当前文档预览的滚动位置设为 `y`（逻辑像素），不限制范围。
    ///
    /// 供绘制前后换入、恢复过渡中的位置使用；其他情况用 [`Self::scroll_to`]。
    pub(crate) fn set_scroll(&mut self, y: f32) {
        if let Some(id) = self.doc {
            self.scrolls.insert(id, y);
        }
    }

    /// 丢弃已读取的图片（联网设置改变时调用）：下次绘制时重新排版并重新读取。
    pub fn forget_images(&mut self) {
        self.images.clear();
        self.urls.clear();
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
    // docx 和内容已取回的阅读标签没有源代码可切换，总是以整个编辑区显示阅读视图。
    let reading = matches!(doc.reading, Some(winc_editor::Reading::Ready(_)));
    if doc.unsupported == Some(winc_editor::Unsupported::Docx) || reading {
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
            if self.preview.markdown.doc.is_some() {
                self.preview.markdown.reset(None);
            }
            return;
        };
        let (id, revision) = (doc.id, doc.revision);
        let md = &mut self.preview.markdown;
        if md.doc != Some(id) {
            md.reset(Some(id));
        }
        if md.checked == Some(revision) {
            return;
        }
        if self.parse_markdown_inline() || self.preview.markdown.pending.is_some() {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let md = &mut self.preview.markdown;
        if doc.unsupported == Some(winc_editor::Unsupported::Docx) {
            let Some(path) = doc.path.clone() else {
                return;
            };
            md.pending = Some(revision);
            let (sender, proxy) = (md.sender.clone(), self.link.proxy.clone());
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
        // 阅读标签的原文不在文档的文本中。
        let reading = match &doc.reading {
            Some(winc_editor::Reading::Ready(text)) => Some(Arc::clone(text)),
            _ => None,
        };
        md.pending = Some(revision);
        let (sender, proxy) = (md.sender.clone(), self.link.proxy.clone());
        winc_editor::background::spawn(move || {
            let started = std::time::Instant::now();
            let text = reading.map_or_else(|| rope.to_string(), |text| text.to_string());
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
        if !self.markdown_live() || self.preview.markdown.cost.is_none_or(|c| c > INLINE_PARSE) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let (id, revision) = (doc.id, doc.revision);
        let md = &mut self.preview.markdown;
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
        while let Ok(done) = self.preview.markdown.receiver.try_recv() {
            let md = &mut self.preview.markdown;
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
                            width: picture.width,
                            height: picture.height,
                            picture: Some(picture),
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
                Done::Undrawn(path, size) => {
                    if let Some(ImageSlot::Ready { drawing, .. }) = md.images.get_mut(&path) {
                        drawing.retain(|&d| d != size);
                    }
                }
            }
            changed = true;
        }
        if changed {
            self.request_redraw();
        }
    }

    /// 返回预览上 `(x, y)` 处的链接序号（见 [`MarkdownPreview::links`]）。
    pub(crate) fn markdown_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.preview.markdown.area.contains(x, y) {
            return None;
        }
        self.hits
            .markdown_links
            .iter()
            .position(|(rect, _)| rect.contains(x, y))
    }

    /// 第 `index` 个链接是否为预览自身的控件（代码块的复制按钮、图的全屏按钮、折叠块的标题、
    /// 勾选框），而非正文中的链接。
    pub(crate) fn markdown_link_is_control(&self, index: usize) -> bool {
        use crate::app::preview::markdown::laid::{COPY, DETAILS, EXPAND, TABLE, TASK};
        self.hits.markdown_links.get(index).is_some_and(|(_, url)| {
            [COPY, DETAILS, EXPAND, TABLE, TASK]
                .iter()
                .any(|prefix| url.starts_with(prefix))
        })
    }

    /// 处理预览中链接的点击：网址交给浏览器，`#标题` 滚动到该标题，本地文件在编辑器中打开。
    pub(crate) fn open_markdown_link(&mut self, index: usize) {
        let Some((_, url)) = self.hits.markdown_links.get(index).cloned() else {
            return;
        };
        if url == crate::app::preview::markdown::laid::EXPAND {
            self.open_diagram_view();
            return;
        }
        if url == crate::app::preview::markdown::laid::COPY {
            let Some(code) = self.hits.markdown_copy.clone() else {
                return;
            };
            self.set_clipboard(code.to_string(), false);
            self.flash(t!("preview-markdown-code-copied"));
            return;
        }
        if let Some(op) = url.strip_prefix(crate::app::preview::markdown::laid::TABLE) {
            self.live_table_tool(op);
            return;
        }
        if let Some(at) = url.strip_prefix(crate::app::preview::markdown::laid::TASK) {
            if let Ok(at) = at.parse() {
                self.toggle_markdown_task(at);
            }
            return;
        }
        if let Some(id) = url.strip_prefix(crate::app::preview::markdown::laid::DETAILS) {
            if let Ok(id) = u64::from_str_radix(id, 16)
                && !self.preview.markdown.toggled.remove(&id)
            {
                self.preview.markdown.toggled.insert(id);
            }
            self.preview.markdown.toggled_now = Some(url);
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
            Some(path) if winc_backend::fs::is_file(&path) => self.open_markdown_file(&path, &url),
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
        let editing = self.preview.markdown.editing == Some(id);
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
            self.preview.markdown.editing = None;
            self.preview.markdown.followed = self
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
                app.preview.markdown.pending_anchor = Some((id, anchor));
            }
        });
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
            if self.preview.markdown.selection.take().is_some() {
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
            self.preview.markdown.focused = false;
            self.preview.markdown.selection = None;
            return false;
        }
        let step = self.fonts.line_height * 2.0;
        let page = (area.h * 0.9).max(step);
        let back = self.pointer.modifiers.shift_key();
        let y = self.preview.markdown.scroll();
        // 方向键逐段滚动，直接到达；翻页和到两端的跳转平滑滚动。
        let small = matches!(key, Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown)) && !cmd;
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
        if small {
            self.scroll_markdown_to(to);
        } else {
            self.glide_pane_to(ScrollTarget::Markdown, to);
        }
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
                self.preview.markdown.focused
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

    /// 重排后开始过渡动画（见 [`BlockMotion`]）。
    ///
    /// 各块从偏移 `from` 处滑动到新位置，`fading` 中的项淡入。
    ///
    /// 没有需要移动或淡入的内容、或动态效果关闭时不开始动画，直接绘制新的排版。
    pub(crate) fn start_markdown_motion(&mut self, from: Vec<f32>, fading: Fading) {
        let still = from.iter().all(|d| d.abs() < 0.5);
        if still && fading.is_empty() {
            self.preview.markdown.motion = None;
            return;
        }
        #[cfg(test)]
        if let Some(at) = self.preview.markdown.frozen {
            let clock = crate::app::Clock::frozen(at, crate::app::animation::MARKDOWN_MOTION);
            self.preview.markdown.motion = Some(BlockMotion {
                clock,
                from,
                fading,
            });
            return;
        }
        self.preview.markdown.motion =
            self.clock(crate::app::animation::MARKDOWN_MOTION)
                .map(|clock| BlockMotion {
                    clock,
                    from,
                    fading,
                });
    }

    /// 处理预览上的鼠标按下：获取焦点并从该处开始选择文字；双击选中一个词，按住 ⇧ 时扩展选区。
    ///
    /// 可编辑的预览中改为放置文档的光标，见 [`App::click_live`]。
    pub(in crate::app) fn click_markdown(&mut self, x: f32, y: f32) {
        self.focus = crate::app::Focus::Editor;
        // 内容一栏两侧的空白：分栏时将键盘焦点交给预览，但不放置光标、不开始选择，保留已有选区。
        if !crate::app::preview::markdown::laid::in_content(self, x) {
            if !self.markdown_live() {
                self.preview.markdown.focused = true;
                self.request_redraw();
            }
            return;
        }
        if self.markdown_live() {
            self.click_live(x, y);
            return;
        }
        self.preview.markdown.focused = true;
        self.request_redraw();
        let Some(at) = crate::app::preview::markdown::laid::offset_at(self, x, y) else {
            self.preview.markdown.selection = None;
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.pointer.modifiers.shift_key();
        self.preview.markdown.selection = match self.preview.markdown.selection {
            _ if count >= 2 => self.markdown_word(at),
            Some((anchor, _)) if shift => Some((anchor, at)),
            _ => Some((at, at)),
        };
        self.pointer.drag = crate::app::Drag::MarkdownText;
        // 分栏时左侧的光标移到点中的字上：随后输入的文字（输入时焦点回到左侧）插入在此处。只看
        // 预览时光标不可见，不移动。
        if count == 1
            && !shift
            && shown_area(self).is_some_and(|a| a != self.layout.editor)
            && let Some(source) = self.current_source_at(x, y)
        {
            self.place_caret_at_source(source);
            // 预览不随左侧滚动：点中的字保持在鼠标下方。
            if let (Some(laid), Some(doc)) =
                (self.preview.markdown.layout.as_ref(), self.editor.active())
            {
                self.preview.markdown.synced = Some((doc.id, doc.scroll_line, laid.height()));
            }
        }
    }

    /// 文字光标在原文中的位置（字节偏移）；解析结果不是当前版本时为 `None`。
    pub(crate) fn markdown_caret_source(&self) -> Option<usize> {
        let doc = self.editor.active()?;
        if self.preview.markdown.checked != Some(doc.revision) {
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
        if doc.unsupported.is_some() || self.preview.markdown.checked != Some(doc.revision) {
            return None;
        }
        crate::app::preview::markdown::laid::source_at(self, x, y)
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
        let Some(source) = self.preview.markdown.menu_source.take() else {
            return;
        };
        if self.editor.active().map(|d| d.preview_mode()) == Some(PreviewMode::Preview) {
            self.set_preview_mode(PreviewMode::Split);
        }
        self.preview.markdown.focused = false;
        self.focus = crate::app::Focus::Editor;
        self.place_caret_at_source(source);
    }

    /// 处理预览上不在链接上的右键点击，返回是否点在预览上。
    ///
    /// 预览获得焦点，点击位置在选区外时取消选区。可编辑的预览中改为将文档的光标移到该处
    /// （点在文档的选区内时不移动）。
    pub(in crate::app) fn markdown_context_target(&mut self, x: f32, y: f32) -> bool {
        if !self.preview.markdown.area.contains(x, y) || self.markdown_link_at(x, y).is_some() {
            return false;
        }
        self.focus = crate::app::Focus::Editor;
        if self.markdown_live() {
            self.preview.markdown.menu_source =
                crate::app::preview::markdown::laid::source_at(self, x, y);
            if let Some(source) = self.preview.markdown.menu_source
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
        self.preview.markdown.focused = true;
        self.preview.markdown.menu_source = self.current_source_at(x, y);
        let at = crate::app::preview::markdown::laid::offset_at(self, x, y);
        let inside = self
            .preview
            .markdown
            .selection
            .zip(at)
            .is_some_and(|((a, b), at)| (a.min(b)..=a.max(b)).contains(&at));
        if !inside {
            self.preview.markdown.selection = None;
        }
        self.request_redraw();
        true
    }

    /// 返回双击处的词：连续的字母数字、连续的中日韩文字，或单个其他字符。
    fn markdown_word(&self, at: usize) -> Option<(usize, usize)> {
        let laid = self.preview.markdown.layout.as_ref()?;
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
        let area = self.preview.markdown.area;
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
        if let Some(at) = crate::app::preview::markdown::laid::offset_at(self, x, y)
            && let Some((anchor, _)) = self.preview.markdown.selection
        {
            self.preview.markdown.selection = Some((anchor, at));
        }
        self.request_redraw();
    }

    /// 返回预览中选中的文字；没有选中时返回 `None`。
    pub(crate) fn markdown_selected_text(&self) -> Option<String> {
        let (a, b) = self.preview.markdown.selection?;
        let text = self
            .preview
            .markdown
            .layout
            .as_ref()?
            .text(a.min(b), a.max(b));
        (!text.is_empty()).then_some(text)
    }

    /// 选中预览中的全部文字（⌘A）。
    pub(crate) fn markdown_select_all(&mut self) {
        let chars = self
            .preview
            .markdown
            .layout
            .as_ref()
            .map_or(0, |l| l.chars());
        self.preview.markdown.selection = Some((0, chars));
        self.request_redraw();
    }

    /// 返回查找用的预览文字（见 [`MarkdownPreview::find_doc`]），尚未装入时立即装入。
    ///
    /// 尚未排版时返回 `None`。
    pub(crate) fn markdown_find_doc(&mut self) -> Option<&winc_editor::Document> {
        if self.preview.markdown.find_doc.is_none() {
            let text = crate::app::preview::markdown::laid::full_text(self)?;
            let md = &mut self.preview.markdown;
            md.find_serial += 1;
            // 与实际文档、差异所用的标识互不重叠。
            let id = DocumentId(u64::MAX - 1 - md.find_serial);
            md.find_doc = Some(winc_editor::Document::from_text(id, &text));
        }
        self.preview.markdown.find_doc.as_ref()
    }

    /// 选中预览中第 `from` 到 `to` 个字符（查找跳转到的匹配）并滚动使其可见：已在屏幕内时
    /// 不滚动，否则滚动到距顶部三分之一屏处。
    pub(crate) fn select_in_markdown(&mut self, from: usize, to: usize) {
        self.preview.markdown.selection = Some((from, to));
        if let Some(y) = crate::app::preview::markdown::laid::offset_top(self, from) {
            let md = &mut self.preview.markdown;
            let scroll = md.scroll();
            if y < scroll || y + 40.0 > scroll + md.area.h {
                let y = y - md.area.h / 3.0;
                self.glide_pane_to(ScrollTarget::Markdown, y);
            }
        }
        self.request_redraw();
    }

    /// 在切换显示方式之前调用：返回只看预览时预览顶部对应的原文行号（小数）。
    ///
    /// 切换为源代码或分栏后，文字滚动到该行（[`App::scroll_text_to`]）；不是只看预览或尚未
    /// 排版时为 `None`。切换为只看预览时，记录在排版完成后对齐一次（见
    /// [`App::align_markdown_once`]）；光标所在的块在预览上开始编辑后才显示原文。
    pub(in crate::app) fn leave_markdown_mode(&mut self, mode: PreviewMode) -> Option<f32> {
        let doc = self.editor.active()?;
        let old = doc.preview_mode();
        self.preview.markdown.followed = None;
        self.preview.markdown.editing = None;
        if old == mode || doc.preview_kind() != Some(PreviewKind::Markdown) {
            return None;
        }
        if mode == PreviewMode::Preview {
            self.preview.markdown.align = Some(doc.id);
            return None;
        }
        let laid = self.preview.markdown.layout.as_ref()?;
        (old == PreviewMode::Preview && self.preview.markdown.doc == Some(doc.id))
            .then(|| laid.line_of_y(self.preview.markdown.scroll(), doc.text().line_count()))
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
#[allow(clippy::disallowed_methods, reason = "只访问本机：图片缓存在本机")]
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
#[allow(clippy::disallowed_methods, reason = "只访问本机：图片缓存在本机")]
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
#[allow(clippy::disallowed_methods, reason = "只访问本机：图片缓存在本机")]
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
