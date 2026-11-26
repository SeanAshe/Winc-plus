//! 文档：文本、选区、撤销历史、高亮状态，以及对应的磁盘文件。
//!
//! 主要类型是 [`Document`]。文本存放在 [`Content`] 中：普通文件是 `rope`，大文件按需读取磁盘。
//! 列对齐、折叠、粘滞滚动等派生数据按 `revision` 判断缓存是否失效。本模块还负责读写磁盘文件（编码、
//! 换行风格、BOM）和检查外部修改。

use std::borrow::Cow;
use std::cell::Cell;
use std::io::{self, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use winc_backend::fs::Stamp;

use ropey::Rope;
use winc_core::edit::{EditKind, Transaction};
use winc_core::selection::merge;
use winc_core::text::{Aligned, CellLayout, ColumnWidths, Columns};
use winc_core::transform::{JsonError, JsonLayout};
use winc_core::{History, Indent, LineEnding, Lines, Selection, diff, indent, transform};
use winc_i18n::t;
use winc_syntax::{Highlighter, LineState};

use crate::Command;
use crate::diff_view::DiffView;
use crate::editing::cursors::others;
use crate::editing::folding::{Folds, LineWindow, Tracked, WindowKey, WrapKey};
use crate::editing::long_lines::WholeLines;
use crate::editing::modal::Marks;
use crate::editing::scopes::{self, FoldRange, Scope, ScopeMemo};
use crate::navigation::bookmarks::Bookmarks;
use content::Content;
use editorconfig::FileConfig;
pub use edits::ByteEdit;
use edits::EditLog;
use encoding::{self as charset, Encoding, UTF8_BOM, utf16_unmarked};
use large::LargeText;
use winc_preview::image::Picture;
use winc_preview::table::Table;

pub(crate) mod content;
pub(crate) mod disk;
pub mod editorconfig;
mod edits;
pub mod encoding;
mod indentation;
mod parked;
pub(crate) use parked::Parked;
pub(crate) mod large;
mod preview;
pub(crate) mod progress;
#[cfg(test)]
mod tests;

use disk::load;
pub(crate) use winc_config::files::sync_dir;

/// 文档的标识，在同一个 [`crate::Editor`] 内唯一。
///
/// 界面按（文档、版本）缓存绘制结果，因此标识在进程内不会复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId(pub u64);

/// 打开文件失败的原因。
#[derive(Debug)]
pub enum OpenError {
    /// 读取文件时的 I/O 错误，例如文件不存在或没有读权限。
    Io(io::Error),
    /// 内容是二进制文件（开头 `BINARY_SNIFF` 字节内含 NUL），不能作为文本打开。
    ///
    /// 值为文件大小（字节）。
    Binary(u64),
    /// 文件超过 `OPEN_LIMIT`，值为文件大小（字节）。
    TooLarge(u64),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "{e}"),
            OpenError::Binary(_) => f.write_str(t!("editor-open-binary")),
            OpenError::TooLarge(size) => f.write_str(&t!(
                "editor-open-too-large",
                size = size >> 20,
                limit = OPEN_LIMIT >> 30
            )),
        }
    }
}

impl std::error::Error for OpenError {}

/// 编辑区中显示的图像的状态：解码或绘制中、已就绪，或失败原因。
#[derive(Debug, Clone)]
pub enum PictureState {
    /// 已交给后台线程解码（图片）或绘制（SVG）。
    Loading,
    /// 已生成可直接显示的位图。
    Ready(Arc<Picture>),
    /// 生成失败的原因（例如 SVG 语法错误）。
    Failed(String),
}

/// 表格读取的状态，见 [`TableView`]。
#[derive(Debug, Clone)]
pub enum TableState {
    /// 已交给后台线程读取。
    Loading,
    /// 已读取的表格。
    Ready(Arc<Table>),
    /// 读取失败的原因（例如文件损坏）。
    Failed(String),
}

/// 编辑区中只读的表格：xlsx 工作簿，或切换到预览、分栏的 csv、tsv。
#[derive(Debug, Clone)]
pub struct TableView {
    /// 读取的状态。
    pub state: TableState,
    /// 显示的表格依据的文档版本号（`revision`）：落后于文档时重新读取表格。
    pub revision: u64,
    /// 显示的是第几张工作表（从 0 起）。
    pub sheet: usize,
    /// 横向、纵向的滚动位置（逻辑像素）。纵向用 `f64`：行数很多时总高度超出 `f32` 能精确
    /// 表示的范围。
    pub scroll: (f32, f64),
    /// 选中的单元格：起点（按下处，或按住 `⇧` 之前所在的单元格）和当前格，均为（行号，列号）。
    ///
    /// 选区是以两者为对角的区域，打开时选中 A1。
    pub selection: ((usize, usize), (usize, usize)),
    /// 当前格卡片（完整显示单元格中未显示全的文字）的滚动位置（逻辑像素）。
    pub card_scroll: f32,
    /// 当前格的卡片是否已按 Esc 收起；切换到另一格时重新显示。
    pub card_closed: bool,
    /// 底部工作表标签栏的横向滚动位置（逻辑像素）。
    pub bar_scroll: f32,
    /// 当前工作表的标签是否已滚入标签栏的可见范围。
    pub bar_revealed: bool,
}

impl TableView {
    /// 待读取的表格，依据文档版本 `revision`。
    fn loading(revision: u64) -> Box<Self> {
        Box::new(Self {
            state: TableState::Loading,
            revision,
            sheet: 0,
            scroll: (0.0, 0.0),
            selection: ((0, 0), (0, 0)),
            card_scroll: 0.0,
            card_closed: false,
            bar_scroll: 0.0,
            bar_revealed: false,
        })
    }

    /// 返回已读取的表格；尚未读完或读取失败时返回 `None`。
    pub fn table(&self) -> Option<&Arc<Table>> {
        match &self.state {
            TableState::Ready(table) => Some(table),
            _ => None,
        }
    }
}

/// 阅读标签的内容：由界面在后台取回，取回之前和未能取回时编辑区显示相应的说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// 正在取回。
    Loading,
    /// 已取回的 Markdown 原文，排版后显示。
    Ready(Arc<str>),
    /// 未能取回。
    Failed,
}

/// 可预览文件的种类，见 `Document::preview_kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// SVG：预览为绘制出的图像（`Document::picture`）。
    Svg,
    /// Markdown：预览为排版后的文档，由界面解析和排版。
    Markdown,
    /// csv、tsv：预览为表格（`Document::table`）。
    Table,
    /// LaTeX：预览只读。
    Latex,
}

impl PreviewKind {
    /// 全部种类。
    pub const ALL: [PreviewKind; 4] = [
        PreviewKind::Svg,
        PreviewKind::Markdown,
        PreviewKind::Table,
        PreviewKind::Latex,
    ];

    /// 是否按 Markdown 预览排版（Markdown，以及转换成 Markdown 的 LaTeX）。
    pub fn typeset(self) -> bool {
        matches!(self, PreviewKind::Markdown | PreviewKind::Latex)
    }

    /// 返回语言 `language` 的文本可以预览成哪一种；不能预览的语言返回 `None`。
    pub fn of(language: &winc_syntax::Language) -> Option<PreviewKind> {
        match language.id {
            "svg" => Some(PreviewKind::Svg),
            "markdown" => Some(PreviewKind::Markdown),
            "csv" | "tsv" => Some(PreviewKind::Table),
            "latex" => Some(PreviewKind::Latex),
            _ => None,
        }
    }
}

/// 可预览文件（SVG、Markdown）的显示方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// 只显示源代码。
    Source,
    /// 左侧显示源代码（可以继续编辑），右侧随文字实时预览。
    Split,
    /// 只显示预览，文档暂时只读。
    Preview,
}

/// 没有文本内容的标签：图片，或无法作为文本显示的文件。
///
/// 图片在编辑区显示预览；其余文件在编辑区说明原因，并给出可执行的操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    /// 图片：编辑区显示预览图（见 `Document::picture`）。
    Image,
    /// xlsx 工作簿：编辑区显示表格（见 `Document::table`）。
    Workbook,
    /// docx 文字文档：编辑区显示转换成 Markdown 后排版的阅读视图，由界面读取和转换。
    Docx,
    /// EPUB 电子书：与 [`Unsupported::Docx`] 相同，显示转换成 Markdown 后排版的阅读视图。
    Epub,
    /// PDF（只在能预览 PDF 的平台上，见 `winc_preview::pdf::supported`）：
    /// 编辑区逐页显示，由界面读取和渲染。
    Pdf,
    /// 二进制文件或不支持的文本编码，值为文件大小（字节）。
    Binary(u64),
    /// 文件超过 `OPEN_LIMIT`，值为文件大小（字节）。
    TooLarge(u64),
    /// 无法读取，例如没有权限或文件不存在；内容是显示给用户的原因。
    Unreadable(String),
}

impl Unsupported {
    /// 是否显示转换成 Markdown 后排版的阅读视图（docx、EPUB）。
    pub fn reads_as_markdown(&self) -> bool {
        matches!(self, Unsupported::Docx | Unsupported::Epub)
    }
}

impl From<OpenError> for Unsupported {
    fn from(error: OpenError) -> Self {
        match error {
            OpenError::Binary(size) => Unsupported::Binary(size),
            OpenError::TooLarge(size) => Unsupported::TooLarge(size),
            OpenError::Io(e) => Unsupported::Unreadable(match e.kind() {
                io::ErrorKind::NotFound => t!("editor-unreadable-not-found").to_owned(),
                io::ErrorKind::PermissionDenied => t!("editor-unreadable-permission").to_owned(),
                _ => t!("editor-unreadable-failed", error = e.to_string()),
            }),
        }
    }
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsupported::Image => f.write_str(t!("editor-unsupported-image")),
            Unsupported::Workbook => f.write_str(t!("editor-unsupported-workbook")),
            Unsupported::Docx => f.write_str(t!("editor-unsupported-docx")),
            Unsupported::Epub => f.write_str(t!("editor-unsupported-epub")),
            Unsupported::Pdf => f.write_str(t!("editor-unsupported-pdf")),
            Unsupported::Binary(size) => {
                f.write_str(&t!("editor-unsupported-binary", size = size_text(*size)))
            }
            Unsupported::TooLarge(size) => f.write_str(&t!(
                "editor-unsupported-too-large",
                size = format!("{:.1}", *size as f64 / (1u64 << 30) as f64),
                limit = OPEN_LIMIT >> 30
            )),
            Unsupported::Unreadable(reason) => {
                f.write_str(&t!("editor-unsupported-unreadable", reason = reason))
            }
        }
    }
}

/// 把文件大小格式化为「2.4 MB」形式的文字：不足 1 KB 时写字节数，否则按 1000 进位并保留一位小数。
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return t!("editor-size-bytes", count = bytes);
    }
    let mut size = bytes as f64 / 1000.0;
    let mut unit = 0;
    while size >= 999.95 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Io(e)
    }
}

/// 检查外部修改后的处理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskChange {
    /// 磁盘文件没有变化，或者变化与本事件无关。
    Unchanged,
    /// 文档没有未保存的修改，已经按磁盘内容重新加载。
    Reloaded,
    /// 文件仅在末尾追加了内容（例如日志）：只把追加的部分接入文档末尾，
    /// 撤销记录、光标、折叠保持不变。
    Appended,
    /// 磁盘文件和文档都有修改，保留文档中的内容。
    Conflict,
}

/// 按（版本、Tab 宽度）缓存的列对齐结果；`None` 表示不对齐。
type ColumnsCache = ((u64, usize), Option<Arc<Columns>>);

/// 按（版本、Tab 宽度）缓存的整个文件可折叠的块，共享给每次折叠命令，不必复制。
type FoldCache = ((u64, usize), Rc<[FoldRange]>);

/// 按列对齐 CSV、TSV 的文件大小上限（字节，2 MiB）。
///
/// 输入时只重新测量改动的行（[`ColumnWidths`]），但删除某一列最宽的字段或更换文件时需要测量
/// 整个文件，耗时随文件大小线性增长，2 MiB 的文件约需数毫秒。超过上限的文件只着色、不对齐。
const ALIGN_MAX: usize = 2 << 20;

/// 返回 `old` 改为 `new` 时涉及的行 `(head, old_end, new_end)`。
///
/// `old` 的 `head..old_end` 行替换为 `new` 的 `head..new_end` 行，其余行不变。从两端逐块比较
/// 相同的字节，不逐行比较。
fn changed_lines(old: &ropey::Rope, new: &ropey::Rope) -> (usize, usize, usize) {
    let same = |a: ropey::iter::Chunks<'_>, b: ropey::iter::Chunks<'_>, back: bool| {
        let (mut a, mut b) = (a.map(str::as_bytes), b.map(str::as_bytes));
        let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
        let mut same = 0;
        loop {
            if x.is_empty() {
                match a.next() {
                    Some(c) => x = c,
                    None => return same,
                }
            }
            if y.is_empty() {
                match b.next() {
                    Some(c) => y = c,
                    None => return same,
                }
            }
            let n = x.len().min(y.len());
            let (p, q) = if back {
                (&x[x.len() - n..], &y[y.len() - n..])
            } else {
                (&x[..n], &y[..n])
            };
            if p != q {
                let pairs = p.iter().zip(q);
                return same
                    + if back {
                        pairs.rev().take_while(|(u, v)| u == v).count()
                    } else {
                        pairs.take_while(|(u, v)| u == v).count()
                    };
            }
            same += n;
            (x, y) = if back {
                (&x[..x.len() - n], &y[..y.len() - n])
            } else {
                (&x[n..], &y[n..])
            };
        }
    };
    let prefix = same(old.chunks(), new.chunks(), false);
    let head = old.byte_to_line(prefix).min(new.byte_to_line(prefix));
    fn tail_of(text: &ropey::Rope) -> ropey::iter::Chunks<'_> {
        text.chunks_at_byte(text.len_bytes()).0.reversed()
    }
    let suffix = same(tail_of(old), tail_of(new), true);
    // 完全落在相同结尾部分内的行数。
    let full = |text: &ropey::Rope| {
        let at = text.len_bytes() - suffix.min(text.len_bytes());
        let line = text.byte_to_line(at);
        let first = if text.line_to_byte(line) == at {
            line
        } else {
            line + 1
        };
        text.len_lines() - first.min(text.len_lines())
    };
    let (old_n, new_n) = (old.len_lines(), new.len_lines());
    let tail = full(old).min(full(new)).min(old_n - head).min(new_n - head);
    (head, old_n - tail, new_n - tail)
}

pub use winc_search::{LARGE_FILE, looks_binary};
pub(crate) use winc_search::{OPEN_LIMIT, UTF16_INLINE};

/// 本机上的文件 `path` 是否超过 [`LARGE_FILE`]、按大文件打开；远端的文件不查询，返回 `false`。
pub fn opens_as_large(path: &Path) -> bool {
    !winc_backend::is_remote(path)
        && winc_backend::fs::metadata(path).is_ok_and(|m| m.len > LARGE_FILE)
}
/// 一次复制的最大字节数（字节，64 MiB）。
///
/// 大文件全选后复制会把整个文件读进内存，因此需要限制。
pub const COPY_LIMIT: u64 = 64 << 20;
/// 按 `#!` 行识别语言时读取首行的最大长度（字符，256）。
///
/// 识别只匹配行首的解释器名与参数，常见的 `#!/usr/bin/env -S python3 -u` 一类行长度为几十个字符，
/// 该上限在此基础上留出余量，同时限制超长首行复制到字符串的开销。超过该长度的部分不参与识别。
const SHEBANG_CHARS: usize = 256;
/// 一次编辑涉及的选区的最大字节数（字节，16 MiB）。
///
/// 删除的内容要保存在撤销记录里，选区过大会使内存占用随之增长。
pub const EDIT_LIMIT: u64 = 16 << 20;
/// 大文件高亮的回看行数（行，256）。
///
/// 高亮状态从可见区域上方 256 行处开始推算，推算的代价因此与文件长度无关；高亮状态的传播不能
/// 跨过这一距离，起始位置以上 256 行内没有未闭合的块注释或字符串时结果才与从头推算相同。
const HIGHLIGHT_LOOKBACK: usize = 256;

/// 返回文字 `text` 中各行 `lines`（从 0 起，从小到大）行首的高亮状态，按 `language` 推算；供
/// 不打开文件、只显示其中几行的预览和搜索结果使用。
///
/// 从文件开头推算，结果与打开文件后相同，各行共用推算的结果；推算到某一行要再推算的行数超过
/// `budget` 时，该行从上方 `HIGHLIGHT_LOOKBACK` 行处推算，与打开大文件时相同。
pub fn line_highlight_states(
    text: &ropey::Rope,
    language: &'static winc_syntax::Language,
    lines: &[usize],
    budget: usize,
) -> Vec<LineState> {
    let mut highlighter = Highlighter::new(language);
    (lines.iter())
        .map(|&line| {
            (highlighter.state_at(text, line, budget))
                .unwrap_or_else(|| highlighter.state_near(text, line, HIGHLIGHT_LOOKBACK))
        })
        .collect()
}

/// 粘滞滚动每帧最多向后推算的高亮行数（行，4000）。
///
/// 与编辑区绘制时的每帧预算取同一个值：推算不完时本次结果不缓存，下一帧重新计算。该值取自调参，
/// 上限越大，跳到文件靠后位置时粘滞块能更早算出，单帧耗时也随之增长。
pub(crate) const STICKY_HIGHLIGHT_BUDGET: usize = 4_000;
pub(crate) use winc_search::BINARY_SNIFF;

/// 一个打开的文档：文本、选区、撤销历史、高亮状态，以及对应的磁盘文件。
///
/// 文本可能来自 `rope`（普通文件）或按需读取的磁盘文件（大文件），见 [`Content`]。文本每变化
/// 一次 `revision` 加 1，所有按内容缓存的派生数据（列对齐、折叠、粘滞滚动）都以它为准。
pub struct Document {
    /// 文档标识，在同一个 `Editor` 内唯一。
    pub id: DocumentId,
    /// 对应的磁盘文件；未命名文档和输出标签没有路径。
    pub path: Option<PathBuf>,
    /// 当前选区；`anchor` 与 `head` 相同时表示光标。
    ///
    /// 有多个光标时是其中的主选区，其余的见 `Document::extra`。
    pub selection: Selection,
    /// 连续上下移动光标时保持的显示列（从 0 起）；其他命令会将其重置。
    pub goal_cell: Option<usize>,
    /// 撤销与重做栈，同时记录内容是否已保存。
    pub history: History,
    /// 逐行的词法高亮状态。
    pub highlighter: Highlighter,
    /// 文件的换行风格；新插入的换行使用同一风格。
    pub line_ending: LineEnding,
    /// 文件的编码；保存时按它写回。
    pub encoding: Encoding,
    /// 从文件内容检测出的缩进；无法检测（新文件、没有缩进）时为 `None`，此时使用语言的默认值。
    pub detected_indent: Option<Indent>,
    /// `.editorconfig` 对该文件的设置，优先于检测出的缩进。
    pub editorconfig: FileConfig,
    /// 在状态栏为该文件选择的缩进，优先于 `.editorconfig` 和检测出的缩进。
    ///
    /// 仅在本次打开期间有效，不写入任何文件。
    pub indent_override: Option<Indent>,
    /// 在状态栏为该文件选择的 Tab 显示宽度（显示列）。
    pub tab_width_override: Option<usize>,
    /// 是否只读：预览图、二进制文件、含无法解码字节的文件都是只读。
    pub readonly: bool,
    /// 用户是否把文档设为只读：不接受编辑，已有的修改仍可保存。
    locked: bool,
    /// 所在的窗口是否把全部文档设为只读（见 [`Editor::set_all_locked`](crate::Editor)）。
    all_locked: bool,
    /// 无法显示的文件：标签中只显示原因。
    ///
    /// 此时文本为空，文档只读。
    pub unsupported: Option<Unsupported>,
    /// 编辑区显示图像而不是文本时的图像：图片文件，或切换到预览的 SVG。
    pub picture: Option<PictureState>,
    /// 编辑区显示的表格：xlsx 工作簿，或仅预览的 csv、tsv；分栏时表格显示在右侧。
    pub table: Option<Box<TableView>>,
    /// 编辑区显示并排差异而不是文本，差异由 `DiffView` 管理。
    ///
    /// 此类文档只读，没有对应的文件。
    pub diff: Option<Box<DiffView>>,
    /// 编辑区显示 Git 日志而不是文本，日志由 `LogView` 管理。
    ///
    /// 只读、没有对应的文件。
    pub log: Option<Box<crate::git::log::LogView>>,
    /// 编辑区按十六进制显示文件的字节，而不是文本；只读，文本为空。
    pub hex: Option<Box<crate::hex::HexView>>,
    /// 编辑区显示终端而不是文本；会话由界面层管理，没有对应的文件。
    pub terminal: bool,
    /// 切换到预览之前的 `readonly`：SVG 的预览期间强制只读，切回源代码时恢复该值。
    readonly_before_preview: bool,
    /// 分栏预览（`PreviewMode::Split`）：文本仍然显示，右侧的预览由界面管理。
    split: bool,
    /// 是否为 Markdown 仅预览模式：编辑区绘制排版后的文档，光标所在的块显示原文，可以直接编辑。
    ///
    /// SVG 的预览使用 `picture`。
    rendered: bool,
    /// 不对应文件的只读标签（格式化工具的输出、安装说明）的标题。
    pub title: Option<String>,
    /// 未命名文档在标签上显示的名字（见 [`Document::make_untitled`]）；为 `None` 时显示
    /// `untitled`。有路径时不使用。
    pub untitled_name: Option<String>,
    /// 阅读标签的内容：编辑区显示排版后的 Markdown，没有可切换的源代码，文本为空。
    pub reading: Option<Reading>,
    /// 文本每变化一次加 1，搜索结果等缓存据此判断是否失效。
    pub revision: u64,
    /// 滚动位置的首行（行，从 0 起，可以是小数，用于平滑滚动）。
    pub scroll_line: f64,
    /// 横向滚动的偏移（像素）。
    pub scroll_x: f32,
    /// 主选区以外的其余选区（多光标）。
    pub(crate) extra: crate::editing::cursors::Extra,
    /// 撤销多光标操作的历史，以及键盘列选择正在框选的区域。
    pub(crate) cursor_memory: crate::editing::cursors::CursorMemory,
    /// 扩大选区时逐层记录的（扩大前，扩大后）选区；缩小选区时据此回退。
    pub(crate) grown: Vec<(Selection, Selection)>,
    /// 已折叠的块。
    pub(crate) folds: Folds,
    /// 书签，按行号从小到大排列；文本变化时随之移动。
    pub(crate) bookmarks: Bookmarks,
    /// 模态编辑层的标记与修改列表；文本变化时随之移动。
    pub(crate) marks: Marks,
    /// 其他窗格里这个文档的视图状态；文本变化时随之移动。
    pub(crate) parked: Parked,
    /// CSV、TSV 是否按列对齐显示（见 [`Editor::set_align_columns`](crate::Editor)）。
    align_columns: bool,
    /// 表格的分隔符（[`Document::delimiter`]），按（版本、语言）缓存。
    delimiter_memo: Cell<Option<(u64, &'static str, Option<char>)>>,
    /// CSV、TSV 按列对齐时每一列的起始位置，按（版本、Tab 宽度）缓存。
    columns: Option<ColumnsCache>,
    /// 上一次测量列宽时的文字和各列宽度；文字变化时只重新测量改动的行。
    column_widths: Option<(ropey::Rope, ColumnWidths)>,
    /// 整个文件可折叠的块，按（版本、Tab 宽度）缓存。
    pub(crate) fold_cache: Option<FoldCache>,
    /// 粘滞滚动上一次向上查找的结果，键为（版本、Tab 宽度）；键不变时下一次直接复用。
    sticky_memo: ScopeMemo,
    /// 大文件需要的后台断行设置；`None` 表示当前的断行结果可用或不需要换行。
    pub(crate) wrap_wanted: Option<WrapKey>,
    /// 已交给后台、尚未收到结果的断行设置。
    pub(crate) wrap_pending: Option<WrapKey>,
    /// 最近使用的长行行内窗口（见 [`Document::line_window`]），最近使用的排在最后。
    pub(crate) windows: Mutex<Vec<(WindowKey, Arc<LineWindow>)>>,
    /// 显示整行的行及其列索引，见 [`Document::shows_whole_line`]。
    pub(crate) whole: WholeLines,
    /// 最近的若干次改动，见 [`Document::unchanged_lines_since`]、[`Document::byte_edit_since`]。
    edits: EditLog,
    /// 文档的文本：`rope`，或按需读取磁盘的大文件。
    pub(crate) text: Content,
    /// 最近一次读写时磁盘文件的修改时间。
    disk_mtime: Option<Stamp>,
    /// 最近一次读写时磁盘文件的大小（字节，含 BOM），用于判断文件是否只在末尾追加了内容。
    disk_len: u64,
    /// 磁盘文件已被其他程序修改，而文档也有未保存的修改：任何一边的内容都不能被静默覆盖。
    ///
    /// 保存或重新加载后清除。
    conflict: bool,
    /// 是否为恢复会话时尚未读取的标签（见 `Document::pending`）：文本为空、只读，切换到该标签
    /// 时由 `crate::Editor::load` 替换为已读取的文档。
    pending: bool,
    /// 尚未读取的文档在读取时是否把二进制内容也按文本读入（「以文本方式打开」）。
    pending_as_text: bool,
    /// 尚未读取的文档读取时是否打开成十六进制视图。
    pending_hex: bool,
    /// 尚未读取的文档读取时文件不存在是否成为新文件（见 `Document::new_file`）。
    pending_new: bool,
    /// 用户通过「通过编码重新打开」选的编码：之后从磁盘重新读取时按它解码，不再识别编码，也不
    /// 再判断是否二进制。
    chosen: Option<Encoding>,
    /// 按 Unicode 转义读入时各行原来的写法：保存时未修改的行按它写回。
    escape_origin: Option<std::sync::Arc<charset::EscapeOrigin>>,
}

/// 从磁盘读出的内容。
struct Loaded {
    text: Content,
    /// 检测到的编码。
    encoding: Encoding,
    /// 检测到的换行风格。
    line_ending: LineEnding,
    /// 读取时文件的修改时间。
    mtime: Option<Stamp>,
    /// 读取时文件的大小（字节）。
    len: u64,
    /// 按 Unicode 转义读入时各行原来的写法，见 [`charset::EscapeOrigin`]。
    origin: Option<std::sync::Arc<charset::EscapeOrigin>>,
}

impl Document {
    /// 创建内容为空、没有路径的文档。
    pub fn empty(id: DocumentId) -> Self {
        Self::from_text(id, "")
    }

    /// 用给定文本创建没有路径的文档，换行风格取默认值。
    pub fn from_text(id: DocumentId, text: &str) -> Self {
        Self::from_content(id, Content::Rope(Rope::from_str(text)))
    }

    /// 用已有的内容 `text` 创建没有路径的文档，换行风格取默认值。
    fn from_content(id: DocumentId, text: Content) -> Self {
        Self {
            id,
            path: None,
            selection: Selection::default(),
            goal_cell: None,
            history: History::default(),
            highlighter: Highlighter::new(winc_syntax::detect("")),
            line_ending: LineEnding::default(),
            encoding: Encoding::Utf8,
            detected_indent: None,
            editorconfig: FileConfig::default(),
            indent_override: None,
            tab_width_override: None,
            readonly: false,
            locked: false,
            all_locked: false,
            unsupported: None,
            picture: None,
            table: None,
            diff: None,
            log: None,
            hex: None,
            terminal: false,
            readonly_before_preview: false,
            split: false,
            rendered: false,
            title: None,
            untitled_name: None,
            reading: None,
            revision: 0,
            scroll_line: 0.0,
            scroll_x: 0.0,
            extra: Default::default(),
            cursor_memory: Default::default(),
            grown: Vec::new(),
            folds: Folds::default(),
            bookmarks: Bookmarks::default(),
            marks: Marks::default(),
            parked: Parked::default(),
            align_columns: true,
            delimiter_memo: Cell::new(None),
            columns: None,
            column_widths: None,
            fold_cache: None,
            sticky_memo: ScopeMemo::default(),
            wrap_wanted: None,
            wrap_pending: None,
            windows: Mutex::new(Vec::new()),
            whole: WholeLines::default(),
            edits: EditLog::default(),
            text,
            disk_mtime: None,
            disk_len: 0,
            conflict: false,
            pending: false,
            pending_as_text: false,
            pending_hex: false,
            pending_new: false,
            chosen: None,
            escape_origin: None,
        }
    }

    /// 读取 `path` 并创建文档。
    ///
    /// # Errors
    ///
    /// 文件不存在或没有读权限时返回 [`OpenError::Io`]；内容是二进制时返回
    /// [`OpenError::Binary`]；超过 `OPEN_LIMIT` 时返回 [`OpenError::TooLarge`]。
    pub fn open(id: DocumentId, path: PathBuf) -> Result<Self, OpenError> {
        Self::open_with(id, path, false)
    }

    /// 把二进制文件当作文本打开（「以文本方式打开」），只读。
    ///
    /// # Errors
    ///
    /// 与 [`Document::open`] 相同，但不会因为内容是二进制而返回 [`OpenError::Binary`]；
    /// 文件大小上限仍然生效。
    pub fn open_binary(id: DocumentId, path: PathBuf) -> Result<Self, OpenError> {
        Self::open_with(id, path, true)
    }

    fn open_with(id: DocumentId, path: PathBuf, allow_binary: bool) -> Result<Self, OpenError> {
        let loaded = load(&path, allow_binary)?;
        let mut document = Document::empty(id);
        document.set_path(path);
        document.set_content(loaded);
        Ok(document)
    }

    /// 创建只读、不对应文件的输出标签，标签上显示 `title`，内容为 `text`。
    pub fn output(id: DocumentId, title: &str, text: &str) -> Self {
        let mut document = Document::from_text(id, text);
        document.title = Some(title.to_owned());
        document.readonly = true;
        document
    }

    /// 把输出标签转为可编辑的未命名文档，标签上仍显示原来的标题。
    ///
    /// 当前内容作为保存基线：不修改时关闭不询问是否保存。不是输出标签时不做处理。
    pub fn make_untitled(&mut self) {
        if self.reading.is_some() || self.diff.is_some() || self.log.is_some() || self.terminal {
            return;
        }
        if let Some(title) = self.title.take() {
            self.untitled_name = Some(title);
            self.readonly = false;
        }
    }

    /// 把 [`Document::make_untitled`] 转出的未命名文档恢复为只读的输出标签。
    pub fn make_output(&mut self) {
        if self.path.is_none()
            && let Some(name) = self.untitled_name.take()
        {
            self.title = Some(name);
            self.readonly = true;
        }
    }

    /// 创建只读、不对应文件的阅读标签，标签上显示 `title`，内容为 `reading`。
    pub fn reading(id: DocumentId, title: &str, reading: Reading) -> Self {
        let mut document = Document::from_text(id, "");
        document.title = Some(title.to_owned());
        document.readonly = true;
        document.reading = Some(reading);
        document
    }

    /// 创建只读、不对应文件的文档，与 `text` 共用同一份文本（克隆只增加引用计数）。
    pub fn snapshot(id: DocumentId, text: Rope) -> Self {
        let mut document = Document::from_content(id, Content::Rope(text));
        document.readonly = true;
        document
    }

    /// 创建并排差异的标签，标签上显示 `title`。
    ///
    /// 文本是右侧的内容，与差异视图共用同一份 `rope`（克隆只增加引用计数，不复制文本）。文档
    /// 只读，没有对应的文件。
    pub fn diff(id: DocumentId, title: &str, view: DiffView) -> Self {
        let mut document = Document::from_content(id, Content::Rope(view.new.text.clone()));
        document.title = Some(title.to_owned());
        document.readonly = true;
        document.diff = Some(Box::new(view));
        document
    }

    /// 创建 Git 日志的标签，标签上显示 `title`。
    ///
    /// 没有文本，只读。
    pub fn log(id: DocumentId, title: &str, view: crate::git::log::LogView) -> Self {
        let mut document = Document::from_text(id, "");
        document.title = Some(title.to_owned());
        document.readonly = true;
        document.log = Some(Box::new(view));
        document
    }

    /// 创建终端标签，标题为 `title`；没有文本和文件，会话由界面层管理。
    pub fn terminal(id: DocumentId, title: &str) -> Self {
        let mut document = Self::empty(id);
        document.title = Some(title.to_owned());
        document.readonly = true;
        document.terminal = true;
        document
    }

    /// 返回 CSV、TSV 按列对齐时每一列的起始显示列；列宽由该列所有行中最宽的一格决定。
    ///
    /// 不是表格，或文件超过 `ALIGN_MAX` 时返回 `None`，此时按普通文本显示。
    /// 结果按（版本、Tab 宽度）缓存。
    pub fn columns(&mut self, tab_width: usize) -> Option<Arc<Columns>> {
        let key = (self.revision, tab_width);
        if let Some((cached, columns)) = &self.columns
            && *cached == key
        {
            return columns.clone();
        }
        let columns = self.column_widths(tab_width).map(|w| Arc::new(w.columns()));
        self.columns = Some((key, columns.clone()));
        columns
    }

    /// 返回当前文字各列的宽度；不是表格时返回 `None`。
    ///
    /// 与上一次测量时的文字比较，只重新测量改动的行。
    fn column_widths(&mut self, tab_width: usize) -> Option<&ColumnWidths> {
        let Some(separator) = self.column_separator() else {
            self.column_widths = None;
            return None;
        };
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        let widths = match self.column_widths.take() {
            Some((old, mut widths))
                if widths.separator == separator && widths.tab_width == tab_width =>
            {
                let (head, old_end, new_end) = changed_lines(&old, rope);
                let removed = old.slice(old.line_to_char(head)..old.line_to_char(old_end));
                let inserted = rope.slice(rope.line_to_char(head)..rope.line_to_char(new_end));
                if widths.update(removed.chunks(), inserted.chunks()) {
                    widths
                } else {
                    ColumnWidths::measure(separator, rope.chunks(), tab_width)
                }
            }
            _ => ColumnWidths::measure(separator, rope.chunks(), tab_width),
        };
        let (_, widths) = self.column_widths.insert((rope.clone(), widths));
        Some(widths)
    }

    /// 与 [`Document::columns`] 相同，但只借用 `&self`。
    ///
    /// 缓存对应当前版本时直接使用，否则重新测量整个文件，且不写入缓存。需要换算大量位置时，
    /// 先用 [`Document::columns`] 取得一次结果，再交给 `*_with` 系列的换算函数。
    pub fn current_columns(&self, tab_width: usize) -> Option<Arc<Columns>> {
        match &self.columns {
            Some((key, columns)) if *key == (self.revision, tab_width) => columns.clone(),
            _ => self.measure_columns(tab_width).map(Arc::new),
        }
    }

    /// 把第 `line` 行的行内列 `col`（字符偏移，从 0 起）换算为显示列；
    /// 按列对齐的 CSV 返回对齐后的位置。
    ///
    /// # Panics
    ///
    /// 第 `line` 行不存在，或 `tab_width` 为 0 且该行含制表符时 panic。
    pub fn col_to_cell(&self, line: usize, col: usize, tab_width: usize) -> usize {
        let columns = self.current_columns(tab_width);
        self.col_to_cell_with(columns.as_deref(), line, col, tab_width)
    }

    /// 同 [`Document::col_to_cell`]，按列对齐时的各列位置由调用方给出（[`Document::columns`]
    /// 的结果）。连续换算多处时不必每次都查缓存或重新测量。
    pub(crate) fn col_to_cell_with(
        &self,
        columns: Option<&Columns>,
        line: usize,
        col: usize,
        tab_width: usize,
    ) -> usize {
        if columns.is_none()
            && let Some(cells) = self.long_col_to_cell(line, col, tab_width)
        {
            return cells;
        }
        let content = self.text.line_content(line);
        match columns {
            Some(columns) => Aligned { columns, tab_width }.col_to_cell(content.slice(), col),
            None => winc_core::text::col_to_cell(content.slice(), col, tab_width),
        }
    }

    /// 把第 `line` 行的显示列 `cell` 换算为行内列；按列对齐的 CSV 按列起点查找，其余按 Tab 宽度。
    ///
    /// # Panics
    ///
    /// 第 `line` 行不存在时 panic；在到达目标列之前遇到制表符且 `tab_width` 为 0 时同样 panic。
    pub fn cell_to_col(&self, line: usize, cell: usize, tab_width: usize) -> usize {
        let columns = self.current_columns(tab_width);
        self.cell_to_col_with(columns.as_deref(), line, cell, tab_width)
    }

    /// 同 [`Document::cell_to_col`]，各列位置由调用方给出，见 [`Document::col_to_cell_with`]。
    pub(crate) fn cell_to_col_with(
        &self,
        columns: Option<&Columns>,
        line: usize,
        cell: usize,
        tab_width: usize,
    ) -> usize {
        if columns.is_none()
            && let Some(col) = self.long_cell_to_col(line, cell, tab_width)
        {
            return col;
        }
        let content = self.text.line_content(line);
        match columns {
            Some(columns) => Aligned { columns, tab_width }.cell_to_col(content.slice(), cell),
            None => winc_core::text::cell_to_col(content.slice(), cell, tab_width),
        }
    }

    fn measure_columns(&self, tab_width: usize) -> Option<Columns> {
        let separator = self.column_separator()?;
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        Some(Columns::measure(separator, rope.chunks(), tab_width))
    }

    /// 按列对齐时的分隔符（[`Document::delimiter`]）；关闭了按列对齐、不是表格、是大文件或超过
    /// `ALIGN_MAX` 时为 `None`。
    fn column_separator(&self) -> Option<char> {
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        if !self.align_columns || rope.len_bytes() > ALIGN_MAX {
            return None;
        }
        self.delimiter()
    }

    /// 返回表格（CSV、TSV）分列的字符：TSV 为制表符，CSV 为逗号，第一行中分号多于逗号时为分号。
    /// 不是表格或是大文件时返回 `None`。
    ///
    /// 按列着色和按列对齐都按该字符分列。绘制时每一行都要查询，结果按（版本、语言）缓存。
    pub fn delimiter(&self) -> Option<char> {
        let language = self.highlighter.language().id;
        if let Some((revision, id, delimiter)) = self.delimiter_memo.get()
            && revision == self.revision
            && id == language
        {
            return delimiter;
        }
        let delimiter = self.find_delimiter(language);
        (self.delimiter_memo).set(Some((self.revision, language, delimiter)));
        delimiter
    }

    /// 按语言 `language` 和第一行的内容确定表格的分隔符，规则见 [`Document::delimiter`]。
    fn find_delimiter(&self, language: &str) -> Option<char> {
        let separator = match language {
            "csv" => ',',
            "tsv" => '\t',
            _ => return None,
        };
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        let first = rope.line(0);
        let count = |c: char| first.chars().filter(|&x| x == c).count();
        Some(if separator == ',' && count(';') > count(',') {
            ';'
        } else {
            separator
        })
    }

    /// CSV、TSV 是否按列对齐显示（见 [`Document::set_align_columns`]）。
    pub(crate) fn aligns_columns(&self) -> bool {
        self.align_columns
    }

    /// 设置 CSV、TSV 是否按列对齐显示；关闭时按原文排布，仍按列着色，可以自动换行。
    pub(crate) fn set_align_columns(&mut self, align: bool) {
        if self.align_columns != align {
            self.align_columns = align;
            self.columns = None;
            self.column_widths = None;
        }
    }

    /// 创建对应文件已不在磁盘上的文档：内容为空，未保存的内容只能从备份恢复
    /// （见 [`Document::restore_unsaved`]）。
    ///
    /// `mtime` 是文件存在时的修改时间（记录在备份中）。有值时文件按已删除处理
    /// （[`Document::deleted_on_disk`]），自动保存不重建该文件，用户手动保存时才重新创建。
    pub fn missing(id: DocumentId, path: PathBuf, mtime: Option<Stamp>) -> Self {
        let mut document = Document::empty(id);
        document.set_path(path);
        document.disk_mtime = mtime;
        document
    }

    /// 向只读的输出标签（标准输入、格式化工具的输出）末尾追加文本。
    ///
    /// 追加不进入撤销记录，也不算修改。内容不是 `rope`（大文件）时不做改动。
    pub fn append_output(&mut self, text: &str) {
        let Content::Rope(rope) = &mut self.text else {
            return;
        };
        let line = rope.len_lines() - 1;
        let end = rope.len_chars();
        rope.insert(end, text);
        self.highlighter.invalidate_from(line);
        self.revision += 1;
        self.note_edit(line, end);
        self.appended(line);
    }

    /// 创建无法打开的文件对应的文档：只记录路径和原因 `reason`，文本为空，文档只读。
    pub fn unsupported(id: DocumentId, path: PathBuf, reason: Unsupported) -> Self {
        let mut document = Document::empty(id);
        document.set_path(path);
        document.readonly = true;
        document.unsupported = Some(reason);
        document
    }

    /// 返回文档的文本。
    pub fn text(&self) -> &Content {
        &self.text
    }

    /// 是否按大文件方式打开（原文留在磁盘上）。
    pub fn is_large(&self) -> bool {
        self.text.is_large()
    }

    /// 用户是否把文档设为只读。
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// 用户能否切换文档的只读状态：因自身的状态而只读的文档不能。
    pub fn can_lock(&self) -> bool {
        !self.readonly
    }

    /// 由用户把文档设为只读或恢复为可以编辑。
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// 所在的窗口是否把全部文档设为只读。
    pub(crate) fn all_locked(&self) -> bool {
        self.all_locked
    }

    /// 随窗口把文档设为只读或恢复；与文档自己的只读（[`set_locked`](Self::set_locked)）各自独立。
    pub(crate) fn set_all_locked(&mut self, locked: bool) {
        self.all_locked = locked;
    }

    /// 返回文档不能编辑的原因（显示给用户）；可以编辑时返回 `None`。
    pub fn readonly_reason(&self) -> Option<&'static str> {
        let locked = || self.locked.then(|| t!("editor-readonly-locked"));
        let all = || self.all_locked.then(|| t!("editor-readonly-all"));
        self.unwritable_reason().or_else(locked).or_else(all)
    }

    /// 应用不由用户发起的事务（磁盘上的新内容、备份中的内容），不受用户设置的只读限制。
    pub(crate) fn apply_unlocked(&mut self, transaction: Transaction) {
        let held = (self.locked, self.all_locked);
        (self.locked, self.all_locked) = (false, false);
        self.apply(transaction);
        (self.locked, self.all_locked) = held;
    }

    /// 返回文档因自身的状态而只读的原因（显示给用户），此时也不能保存。
    ///
    /// 不包括用户设置的只读。
    pub fn unwritable_reason(&self) -> Option<&'static str> {
        match &self.text {
            _ if !self.readonly => None,
            _ if self.pending => Some(t!("editor-readonly-pending")),
            _ if self.unsupported.is_some() => Some(t!("editor-readonly-unsupported")),
            _ if self.previewing() => Some(t!("editor-readonly-previewing")),
            _ if self.title.is_some() => Some(t!("editor-readonly-tab")),
            _ if self.hex.is_some() => Some(t!("editor-readonly-hex")),
            _ if self.encoding == Encoding::Binary => Some(t!("editor-readonly-binary")),
            Content::Large(large) if large.has_overlong_lines() => {
                Some(t!("editor-readonly-long-lines"))
            }
            _ => Some(t!("editor-readonly-not-utf8")),
        }
    }

    /// 返回不能执行编辑命令 `command` 的原因（显示给用户）；可以执行时返回 `None`。
    pub fn edit_blocked(&self, command: &Command) -> Option<&'static str> {
        if !command.is_edit() {
            return None;
        }

        if let Some(reason) = self.readonly_reason() {
            return Some(reason);
        }

        // 作用于整个文件的命令需要读取并改写整个文件，大文件不支持。
        let whole_file = match command {
            Command::TrimTrailingWhitespace => true,
            &Command::UnescapeUnicode { whole } | &Command::EscapeUnicode { whole } => whole,
            Command::SortLines { .. } => {
                winc_core::transform::sorts_whole_file(&self.text, self.selection)
            }
            _ => false,
        };
        if whole_file && self.is_large() {
            return Some(t!("editor-large-whole-file"));
        }

        if matches!(command, Command::Undo | Command::Redo) {
            return None;
        }
        // 多光标时每个选区都会被改写，被删除的文字都要记入撤销历史，按全部选区的总和计算。
        // 光标（空选区）不占字节，不必逐个计算。
        let mut total = 0;
        for selection in self.all_selections().filter(|s| !s.is_empty()) {
            total += self.text.byte_len(selection.from(), selection.to());
            if total > EDIT_LIMIT {
                return Some(t!("editor-selection-too-large-to-edit"));
            }
        }
        None
    }

    /// 返回按 `layout` 美化或压缩 JSON（[`Command::ReformatJson`]）时第一个无法转换的选区的原因，
    /// 先看主选区；都能转换时返回 `None`。
    pub fn json_problem(&self, layout: JsonLayout) -> Option<JsonError> {
        let (unit, line_ending) = (self.indent().unit(), self.line_ending.as_str());
        let whole = self.json_whole_file();
        self.all_selections().find_map(|selection| {
            transform::reformat_json(&self.text, selection, layout, unit, line_ending, whole).err()
        })
    }

    /// 没有选区时美化、压缩 JSON 是否检查整个文件。
    ///
    /// 大文件和超过 [`EDIT_LIMIT`] 的文件不检查，只看光标所在的行。
    pub(crate) fn json_whole_file(&self) -> bool {
        !self.is_large() && self.text.byte_len(0, self.text.end_pos()) <= EDIT_LIMIT
    }

    /// 返回第 `line` 行（从 0 起）开头的高亮状态。
    ///
    /// 普通文档从头推算并缓存，每次最多向后推算 `budget` 行；尚未推算到 `line` 时返回 `None`。
    ///
    /// 大文件不从头推算，只从 `line` 上方 `HIGHLIGHT_LOOKBACK` 行处开始推算，因此跳到文件末尾时
    /// 无需扫描全文；推算出的一段状态保留下来，滚动时继续使用
    /// （见 [`winc_syntax::Highlighter::state_near`]）。
    pub fn highlight_state(&mut self, line: usize, budget: usize) -> Option<LineState> {
        match &self.text {
            Content::Rope(rope) => self.highlighter.state_at(rope, line, budget),
            Content::Large(large) => Some(self.highlighter.state_near(
                &**large,
                line,
                HIGHLIGHT_LOOKBACK,
            )),
        }
    }

    /// 返回第 `line` 行（从 0 起）开头的近似高亮状态：从上方 `HIGHLIGHT_LOOKBACK` 行处推算。
    ///
    /// 供 [`Self::highlight_state`] 尚未推算到时绘制使用，远距离跳转后不先按普通文本着色；跨度
    /// 超过回看行数的块注释和多行字符串仍可能着色错误，直到精确的状态推算完成。
    pub fn approx_highlight_state(&mut self, line: usize) -> LineState {
        match &self.text {
            Content::Rope(rope) => self.highlighter.state_near(rope, line, HIGHLIGHT_LOOKBACK),
            Content::Large(large) => {
                self.highlighter
                    .state_near(&**large, line, HIGHLIGHT_LOOKBACK)
            }
        }
    }

    /// 返回已推算出的第 `line` 行（从 0 起）开头的高亮状态，不触发新的推算；尚未推算到时返回
    /// `None`。
    ///
    /// 供只需近似着色的调用方（缩略图）使用：大文件只保留一段推算结果，另行推算会使该段
    /// 在编辑区与缩略图之间反复重建。
    pub fn known_highlight_state(&self, line: usize) -> Option<LineState> {
        self.highlighter.known_state(line)
    }

    /// 返回编辑区显示 `rows` 各行时固定在顶部的块（粘滞滚动，从外层到内层，最多 `max` 个），
    /// 以及高亮是否已推算到位。
    ///
    /// `rows` 是编辑区从上到下显示的行号（从 0 起）；`partial` 表示滚动停在某一行的中间。
    /// 第二个返回值为 `false` 时，高亮尚未推算到需要的位置，字符串中的行可能被当作代码，
    /// 需要在下一帧重新计算。
    pub fn sticky_scopes(
        &mut self,
        rows: &[usize],
        partial: bool,
        max: usize,
        tab_width: usize,
    ) -> (Vec<Scope>, bool) {
        // 判断块是否在字符串中，需要 `rows` 及其下方 `SCAN_DOWN` 行（查找块末尾的范围）的高亮状态，
        // 因此先把高亮推算到那里；编辑区绘制时只推算到第一行。预算内推算不完时
        // （例如刚跳到长文件靠后的位置）`settled` 为 `false`，下一帧重新计算。
        let last = rows.last().copied().unwrap_or(0) + scopes::SCAN_DOWN;
        let settled = self
            .highlight_state(last, STICKY_HIGHLIGHT_BUDGET)
            .is_some();
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| starts_quoted(highlighter, text, line);
        let memo = self.sticky_memo.records(self.revision, tab_width);
        let found = scopes::sticky(
            text,
            language,
            rows,
            partial,
            max,
            tab_width,
            &mut quoted,
            memo,
        );
        // 高亮尚未推算到位时，字符串中的行可能被当作代码，本次结果不缓存到下一帧。
        if !settled {
            self.sticky_memo.discard();
        }
        (found, settled)
    }

    /// 返回包含第 `line` 行的块的层数，即粘滞滚动时顶部需要固定的行数。
    pub fn scope_depth(&mut self, line: usize, tab_width: usize) -> usize {
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| starts_quoted(highlighter, text, line);
        let memo = self.sticky_memo.get(self.revision, tab_width);
        scopes::depth(text, language, line, tab_width, &mut quoted, memo)
    }

    /// 返回位置 `from` 到 `to` 之间的文本（字符偏移），用于复制；
    /// 超过 [`COPY_LIMIT`] 字节时返回 `None`。
    pub fn text_range(&self, from: usize, to: usize) -> Option<String> {
        self.text.text_range(from, to, COPY_LIMIT)
    }

    /// 按文件名确定语言并重建高亮状态；无法按文件名识别时按首行的 `#!` 识别。
    ///
    /// 语言变化会影响列对齐和折叠，因此同时递增 `revision`，使按内容缓存的结果失效。
    pub fn set_language(&mut self, file_name: &str) {
        // `#!` 行通常很短，只取开头 `SHEBANG_CHARS` 个字符，超长的首行无需整行复制。
        let first_line: String = self
            .text
            .line_content(0)
            .slice()
            .chars()
            .take(SHEBANG_CHARS)
            .collect();
        self.choose_language(winc_syntax::detect_with_first_line(file_name, &first_line));
    }

    /// 改用状态栏中选择的语言 `language`：重建高亮状态，并使按内容缓存的结果失效，见
    /// [`Document::set_language`]。
    ///
    /// 文件重命名或另存为之后按新文件名重新识别。
    /// 预览或分栏中的文档换成预览方式不同的语言时先回到源代码。
    pub fn choose_language(&mut self, language: &'static winc_syntax::Language) {
        if self.preview_mode() != PreviewMode::Source
            && PreviewKind::of(language) != self.preview_kind()
        {
            self.set_preview_mode(PreviewMode::Source);
        }
        self.highlighter = Highlighter::new(language);
        // 列对齐、折叠、粘滞滚动的结果取决于语言，而缓存的键只有（版本、Tab 宽度）。重命名改变
        // 扩展名（`a.txt` → `a.csv`）时文本不变，因此递增 `revision` 使缓存失效。
        self.revision += 1;
        self.columns = None;
        self.follow_unicode_escapes();
    }

    /// 标签上显示的名字：输出标签用 `title`，否则用文件名；未命名文档用 `untitled_name`，没有
    /// 时是 `untitled`。
    pub fn name(&self) -> Cow<'_, str> {
        if let Some(title) = &self.title {
            return Cow::Borrowed(title);
        }
        if let Some(name) = self.path.as_deref().and_then(Path::file_name) {
            return name.to_string_lossy();
        }
        Cow::Borrowed(self.untitled_name.as_deref().unwrap_or("untitled"))
    }

    /// 是否有未保存的修改。
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// 最近一次读写时磁盘文件的修改时间。
    pub fn disk_mtime(&self) -> Option<Stamp> {
        self.disk_mtime
    }

    /// 恢复备份中上次未保存的内容，返回备份写入后磁盘文件是否被外部修改；无法恢复时返回 `None`。
    ///
    /// 调用前文档应已按磁盘上当前的内容打开。恢复时把内容替换为 `text`，记为一次修改，⌘Z 可回到
    /// 磁盘上的内容。`mtime` 与当前修改时间不同且恢复后有未保存的修改时，说明备份之后文件被其他
    /// 程序修改过，此时标记为冲突并返回 `Some(true)`；`conflict` 表示备份时已处于冲突中。只读或
    /// 大文件时不做改动并返回 `None`。
    pub fn restore_unsaved(
        &mut self,
        text: &str,
        mtime: Option<Stamp>,
        selection: Selection,
        conflict: bool,
    ) -> Option<bool> {
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        if self.readonly {
            return None;
        }

        let old = rope.to_string();
        if old != text {
            let changes = diff::line_changes(&old, text);
            let before = self.selection;
            self.apply_unlocked(Transaction::new(changes, before, before, EditKind::Other));
        }
        self.selection = selection.clamped(self.text.end_pos());

        let changed = conflict || (self.disk_mtime.is_some() && mtime != self.disk_mtime);
        if changed && self.is_dirty() {
            self.conflict = true;
        }
        Some(changed && self.is_dirty())
    }

    /// 应用一个事务并记入撤销栈。
    ///
    /// 只读文档忽略。
    pub fn apply(&mut self, transaction: Transaction) {
        self.apply_as(transaction, false);
    }

    /// 应用一个事务，并入撤销栈顶的撤销单元（见 `History::amend`）。
    ///
    /// 只读文档忽略。
    pub fn amend(&mut self, transaction: Transaction) {
        self.apply_as(transaction, true);
    }

    /// [`Document::apply`] 与 [`Document::amend`] 的实现。
    fn apply_as(&mut self, transaction: Transaction, amend: bool) {
        if self.readonly || self.locked || self.all_locked {
            return;
        }

        // 只有一处改动时记下字节范围，按改动前的文本换算。
        let bytes = match (&transaction.changes[..], self.text.rope()) {
            ([change], Some(rope)) => {
                let start = rope.char_to_byte(change.start.min(rope.len_chars()));
                Some(ByteEdit {
                    old: start..start + change.removed.len(),
                    new: start..start + change.inserted.len(),
                })
            }
            _ => None,
        };

        transaction.apply(&mut Tracked {
            text: &mut self.text,
            folds: &mut self.folds,
            bookmarks: &mut self.bookmarks,
            whole: &mut self.whole,
            marks: &mut self.marks,
            parked: &mut self.parked,
        });
        self.selection = transaction.after.clamped(self.text.end_pos());
        let extra = transaction.extra_after.clone();

        if !transaction.changes.is_empty() {
            let first = transaction.first_offset();
            if amend {
                self.history.amend(transaction);
            } else {
                self.history.push(transaction);
            }
            self.after_change(first);
            if let Some(bytes) = bytes {
                self.edits.set_bytes(self.revision, bytes);
            }
        }
        // 事务不含其余选区时，文本改动后其余选区随版本号变化自动失效；含其余选区时按事务恢复。
        if !extra.is_empty() {
            self.restore_extra(extra);
        }
    }

    /// 返回从版本 `revision` 至今的改动合并成的一处字节改动，范围覆盖其间的每次改动。
    ///
    /// 版本未变、早于最近 64 次改动、其间某次改动有多处（多个光标），或发生了编辑以外的变化
    /// （从磁盘重新读取）时返回 `None`。
    pub fn byte_edit_since(&self, revision: u64) -> Option<ByteEdit> {
        self.edits.bytes_since(revision, self.revision)
    }

    /// 记录最近一次改动（`revision` 已递增）的起始行 `line` 和起始位置 `offset`（字符偏移）。
    fn note_edit(&mut self, line: usize, offset: usize) {
        self.edits.push(self.revision, line, offset);
    }

    /// 返回从版本 `revision` 至今始终未变的开头行数：此后的每次改动都位于这些行之下。
    ///
    /// 只记录最近 64 次改动；版本更早，或期间发生了编辑以外的变化（从磁盘重新读取、
    /// 更换语言）时返回 0。版本未变时返回 `usize::MAX`。
    pub fn unchanged_lines_since(&self, revision: u64) -> usize {
        self.edits.unchanged_since(revision, self.revision).0
    }

    /// 返回从版本 `revision` 至今始终未变的开头字符数：此后的每次改动都从这些字符之后开始。
    ///
    /// 无法确定与版本未变时的返回值同 [`Document::unchanged_lines_since`]。
    pub fn unchanged_chars_since(&self, revision: u64) -> usize {
        self.edits.unchanged_since(revision, self.revision).1
    }

    /// 处理文本变化的后续工作：让变化所在行起的高亮失效，递增 `revision`，刷新折叠。
    ///
    /// `first_offset` 是第一处变化的位置（字符偏移）。
    pub(crate) fn after_change(&mut self, first_offset: usize) {
        let line = self.text.line_of(first_offset);
        self.highlighter.invalidate_from(line);
        self.sticky_memo.edited(self.revision, line);
        self.revision += 1;
        self.note_edit(line, first_offset);
        self.refresh_folds();
        // 正在预览的 SVG 内容变化（例如文件被外部修改后重新读取）时，按新内容重新绘制。
        if self.picture.is_some() {
            self.picture = Some(PictureState::Loading);
        }
    }
}

/// 是否第 `line` 行（从 0 起）的开头处于跨行的非普通代码结构中（见
/// [`LineState::is_quoted`]）。
///
/// 普通文档只使用已经推算好的高亮状态，不向后推算，尚未推算到的行视为不在其中；大文件按
/// `HIGHLIGHT_LOOKBACK` 回看推算。
pub(crate) fn starts_quoted(highlighter: &mut Highlighter, text: &Content, line: usize) -> bool {
    match text {
        Content::Rope(rope) => rope_starts_quoted(highlighter, rope, line),
        Content::Large(large) => highlighter
            .state_near(&**large, line, HIGHLIGHT_LOOKBACK)
            .is_quoted(),
    }
}

/// 同 [`starts_quoted`]，文本为整体载入内存的 `rope`。
pub(crate) fn rope_starts_quoted(highlighter: &mut Highlighter, rope: &Rope, line: usize) -> bool {
    highlighter
        .state_at(rope, line, 0)
        .is_some_and(LineState::is_quoted)
}
