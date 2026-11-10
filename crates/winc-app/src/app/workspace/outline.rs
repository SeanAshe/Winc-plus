//! 文件视图底部的大纲：当前文档的函数、类型、标题等符号，按层级排成可以收起的树。
//!
//! 大纲是文件树下面一块可以展开、收起的区域：标题行始终在底部，展开时向上拉开符号列表，文件树
//! 随之变矮，节奏与文件树展开一个目录相同（`TREE_ANIMATION`）；拖动标题行的上沿可以调整高度。
//!
//! 符号由 [`winc_editor::symbols`] 逐行识别，不接入语言服务器。提取在后台线程进行，结果到达前
//! 列表保持原样，到达后整体换上，界面线程不因长文件停顿。切换文档时立即开始提取；同一文档的
//! 内容变化后，等 [`OUTLINE_DELAY`] 内不再变化时提取，连续输入时不逐键重算；尚未读取完的文档
//! （`Document::is_pending`）等读完再提取。光标所在的符号高亮，移到另一个符号时滚动到可见；
//! 点击一行跳转到该符号。
//!
//! 收起的符号按名字记录，已保存的文件按路径区分，状态随目录的现场（`Session::outline`）保存：
//! 关闭标签后重新打开、重新启动后均保持收起。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use winc_editor::DocumentId;
use winc_editor::symbols::{self, Symbol};
use winc_i18n::t;
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::app::animation::{Clock, TREE_ANIMATION};
use crate::layout::SidebarView;

/// 大纲标题行上下各留的空隙（逻辑像素），与侧栏里列表和上下相邻部分的间隔相同
/// （`SIDEBAR_LIST_GAP`）：上面是分隔线，下面是侧栏卡片的下沿或符号列表，悬停的底色不贴边。
pub(crate) const OUTLINE_GAP: f32 = SIDEBAR_LIST_GAP;
/// 大纲标题行的高度（逻辑像素）：一行 `TREE_ROW`，上下各留 [`OUTLINE_GAP`]。
pub(crate) const OUTLINE_HEADER: f32 = TREE_ROW + 2.0 * OUTLINE_GAP;
/// 第一次展开时符号列表的高度（逻辑像素）：十行。
const OUTLINE_HEIGHT: f32 = 10.0 * TREE_ROW;
/// 拖动调整高度时，符号列表和上面的文件树各自至少保留的高度（逻辑像素）：三行，仍能看出列表
/// 里有几项。
const OUTLINE_MIN: f32 = 3.0 * TREE_ROW;

/// 同一文档的内容变化后，到重新提取符号之间的等待时长。
///
/// 与全局搜索合并连续输入的延迟（`SEARCH_DELAY`）相同：连续键入时只在停顿之后提取一次。
pub(crate) const OUTLINE_DELAY: Duration = super::global::SEARCH_DELAY;

/// 大纲里可以点击的位置，绘制时记下矩形（见 [`OutlineState::hits`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineTarget {
    /// 一个符号所在的行，值为 [`OutlineState::items`] 的下标：跳转到该符号。
    Row(usize),
    /// 有下级的符号行首的箭头，值同上：收起或展开。
    Arrow(usize),
    /// 标题行：展开或收起大纲。
    Header,
    /// 标题行右端的「全部展开」「全部折叠」。
    FoldAll,
}

/// 一个符号的下级正在展开或收起：下级的行从它下面逐渐露出（或逐渐收回），再下面的行随之移动，
/// 它的箭头同时转向，与文件树里目录的展开、收起相同。
#[derive(Debug, Clone, Copy)]
pub struct OutlineFold {
    /// 展开或收起的符号（[`OutlineState::items`] 的下标）。
    pub symbol: usize,
    /// 是否为展开；`false` 表示收起。
    pub expanding: bool,
    /// 动画的时钟。
    pub clock: Clock,
}

impl OutlineFold {
    /// 下级已露出的比例：0 表示完全收起，1 表示完全展开。
    pub fn reveal(&self) -> f32 {
        let eased = self.clock.progress();
        if self.expanding { eased } else { 1.0 - eased }
    }
}

/// 绘制大纲列表时的各行：按展开后的样子排列，正在收起的下级仍在其中（见 [`OutlineFold`]）。
pub struct OutlineLayout {
    /// 各行的符号（[`OutlineState::items`] 的下标），按顺序排列。
    pub rows: Vec<usize>,
    /// 正在展开或收起的下级所占的行区间 `[起, 止)`；没有动画时为 `None`。
    pub block: Option<(usize, usize)>,
    /// 下级尚未露出的高度（逻辑像素）：其下方的行向上移动相应的距离。
    pub hidden: f32,
    /// 正在展开或收起的符号及其箭头转到的比例（0 朝右，1 朝下）。
    pub turning: Option<(usize, f32)>,
}

/// 大纲没有可列出的符号时的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineEmpty {
    /// 没有打开的文档。
    NoFile,
    /// 当前标签不是文本（图片、差异、日志等），没有大纲。
    NotText,
    /// 当前文档为大文件，没有读入内存，不提取符号。
    TooLarge,
    /// 文档里没有识别出符号。
    NoSymbols,
}

/// 大纲的状态。
pub struct OutlineState {
    /// 大纲是否展开。
    pub open: bool,
    /// 展开时符号列表的高度（逻辑像素），拖动标题行的上沿时改变。
    pub height: f32,
    /// 展开、收起的过渡：起点的列表高度与时钟；不在过渡中时为 `None`。
    pub motion: Option<(f32, Clock)>,
    /// 一个符号的下级正在展开或收起；没有时为 `None`。
    pub fold: Option<OutlineFold>,
    /// 最近一次开始提取的文档及其版本号（`revision`）；尚未提取时为 `None`。
    source: Option<(DocumentId, u64)>,
    /// 当前列出的符号属于哪个文档；换成另一个文档的符号时重置滚动和选中。
    shown: Option<DocumentId>,
    /// 后台正在提取的结果：（文档，版本号，符号）；没有进行中的提取时为 `None`。
    pending: Option<Receiver<(DocumentId, u64, Vec<Symbol>)>>,
    /// 最近一次看到的文档及其版本号，用于判断内容是否仍在变化。
    seen: Option<(DocumentId, u64)>,
    /// 内容变化后应当重新提取的时刻；没有等待中的提取时为 `None`。
    due: Option<Instant>,
    /// 当前文档的符号，按在文件中出现的顺序排列。
    pub items: Vec<Symbol>,
    /// 各符号的路径（见 [`symbol_paths`]），随 `items` 一起更新，绘制时不再逐帧拼接。
    paths: Vec<String>,
    /// 没有符号时的原因；有符号时为 `None`。
    pub empty: Option<OutlineEmpty>,
    /// 各文档中已收起的符号的路径（见 [`symbol_paths`]）。
    ///
    /// 按名字而不按行号记录，编辑使符号移动位置后仍保持收起。文件首次列出符号时从目录的现场
    /// 读入，此后以本字段为准。
    collapsed: HashMap<OutlineDoc, HashSet<String>>,
    /// 当前列出的符号所属的文档，作为查询 `collapsed` 的键。
    shown_doc: Option<OutlineDoc>,
    /// 列表的滚动量（逻辑像素）。
    pub scroll: f32,
    /// 键盘选中的符号（[`OutlineState::items`] 的下标）；为 `None` 时选中光标所在的符号。
    pub selected: Option<usize>,
    /// 上次滚动到可见的光标所在符号：光标移到另一个符号时才再次滚动，用户滚动列表后不被拉回。
    revealed: Option<usize>,
}

impl Default for OutlineState {
    /// 尚未提取符号的状态：与没有打开文档时相同，写「没有打开的文件」。
    fn default() -> Self {
        OutlineState {
            open: false,
            height: OUTLINE_HEIGHT,
            motion: None,
            fold: None,
            source: None,
            shown: None,
            pending: None,
            seen: None,
            due: None,
            items: Vec::new(),
            paths: Vec::new(),
            empty: Some(OutlineEmpty::NoFile),
            collapsed: HashMap::new(),
            shown_doc: None,
            scroll: 0.0,
            selected: None,
            revealed: None,
        }
    }
}

/// 记录收起状态时区分文档的键。
///
/// 已保存的文件按路径区分，关闭后重新打开、重新启动后仍能对应；未保存过的文档按文档标识区分。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum OutlineDoc {
    /// 磁盘上的文件。
    File(PathBuf),
    /// 未保存过的文档。
    Untitled(DocumentId),
}

/// 返回各符号的路径：从最外层上级到自身的名字，以 `\u{1f}`（单元分隔符）连接。
///
/// 路径用于记住收起的符号；同名的同级符号共用一个路径，一起收起、展开。
fn symbol_paths(items: &[Symbol]) -> Vec<String> {
    let mut stack: Vec<&str> = Vec::new();
    items
        .iter()
        .map(|symbol| {
            stack.truncate(symbol.depth);
            stack.push(&symbol.name);
            stack.join("\u{1f}")
        })
        .collect()
}

/// 第 `i` 个符号是否有下级：下一个符号的层级比它深。
pub(crate) fn has_children(items: &[Symbol], i: usize) -> bool {
    let depth = items.get(i).map(|s| s.depth);
    items
        .get(i + 1)
        .is_some_and(|next| Some(next.depth) > depth)
}

impl App {
    /// 符号列表是否正在显示：侧栏可见、在文件视图，且大纲展开着（含展开、收起的途中）。
    pub(crate) fn outline_shown(&self) -> bool {
        self.layout.sidebar.right() > 0.0 && self.layout.outline.h > OUTLINE_HEADER
    }

    /// 在文件树下面排出大纲，文件树让出这一段；不在文件视图时没有大纲。
    ///
    /// 展开时列表高度取 [`OutlineState::height`]，并为文件树至少留下 `OUTLINE_MIN`；过渡途中按
    /// 时钟从起点的高度变到终点。
    pub(in crate::app) fn place_outline(&mut self) {
        let l = &mut self.layout;
        l.outline = Rect::default();
        let off = self.config.outline == Some(false);
        if off || self.sidebar.view != SidebarView::Files || l.tree.h <= 0.0 {
            return;
        }
        let room = (l.tree.h - OUTLINE_HEADER - OUTLINE_MIN).max(0.0);
        let o = &self.outline;
        let target = if o.open {
            o.height.clamp(OUTLINE_MIN.min(room), room)
        } else {
            0.0
        };
        let body = o
            .motion
            .map_or(target, |(from, clock)| clock.between(from, target));
        let h = (OUTLINE_HEADER + body.min(room)).min(l.tree.h);
        l.outline = Rect::new(l.tree.x, l.tree.bottom() - h, l.tree.w, h);
        l.tree.h -= h;
    }

    /// 大纲的标题行。
    pub(crate) fn outline_header(&self) -> Rect {
        let o = self.layout.outline;
        Rect::new(o.x, o.y, o.w, OUTLINE_HEADER.min(o.h))
    }

    /// 标题行上沿可以拖动调整高度的一条（逻辑像素，上下各 3）；大纲收起或不显示时为空。
    pub(crate) fn outline_divider(&self) -> Rect {
        if !self.outline.open || self.layout.outline.h <= 0.0 {
            return Rect::default();
        }
        let o = self.layout.outline;
        Rect::new(o.x, o.y - 3.0, o.w, 6.0)
    }

    /// 拖动标题行的上沿：按下时的纵坐标为 `start`、列表高度为 `size`，鼠标移到 `y`。
    pub(in crate::app) fn drag_outline_divider(&mut self, start: f32, size: f32, y: f32) {
        self.outline.height = size - (y - start);
        self.relayout();
        // 记下排版后实际的高度：超出范围时停在边界上，再往回拖立即生效。
        self.outline.height = self.outline_list().h;
        self.request_redraw();
    }

    /// 展开或收起大纲，从现在的高度过渡过去。收起时焦点在大纲上的，交给文件树。
    pub(in crate::app) fn toggle_outline_section(&mut self) {
        let from = self.outline_list().h;
        self.outline.open = !self.outline.open;
        self.outline.motion = self.clock(TREE_ANIMATION).map(|clock| (from, clock));
        if !self.outline.open && self.focus == Focus::Outline {
            self.focus = Focus::Tree;
        }
        self.relayout();
        self.sync_outline(Instant::now());
        self.request_redraw();
    }

    /// 切到文件视图并展开大纲，焦点交给大纲（`⌘7`）；大纲在设置中关闭时只在状态栏说明。
    pub(in crate::app) fn show_outline(&mut self) {
        if self.config.outline == Some(false) {
            self.flash(t!("workspace-outline-off"));
            return;
        }
        self.set_sidebar_view(SidebarView::Files);
        if !self.sidebar.visible {
            self.set_sidebar(true);
        }
        if !self.outline.open {
            self.toggle_outline_section();
        }
        self.focus = Focus::Outline;
        self.outline.selected = None;
        self.sync_outline(Instant::now());
        self.request_redraw();
    }

    /// 按当前文档更新符号，返回下次需要检查的时刻。
    ///
    /// 文档切换后立即提取；同一文档的版本号变化后，等 [`OUTLINE_DELAY`] 内不再变化时提取。大纲
    /// 不显示时不提取，再次显示时按当时的文档更新。
    pub(crate) fn sync_outline(&mut self, now: Instant) -> Option<Instant> {
        if !self.outline_shown() {
            return None;
        }
        // 尚未读取完的文档内容为空，读完后版本号不变：等读完再提取，否则读完后不再更新。
        if self.editor.active().is_some_and(|doc| doc.is_pending()) {
            return None;
        }
        let current = self.editor.active().map(|doc| (doc.id, doc.revision));
        let o = &mut self.outline;
        if current == o.source {
            o.due = None;
            return None;
        }
        let switched = current.map(|c| c.0) != o.source.map(|s| s.0);
        if !switched {
            if current != o.seen {
                o.seen = current;
                o.due = Some(now + OUTLINE_DELAY);
            }
            if let Some(due) = o.due.filter(|&due| now < due) {
                return Some(due);
            }
        }
        self.refresh_outline();
        None
    }

    /// 重新提取当前文档的符号：文本在后台线程提取（没有事件循环的测试里直接提取），不是文本、
    /// 文件过大时直接换上说明。
    fn refresh_outline(&mut self) {
        let Some(doc) = self.editor.active() else {
            let o = &mut self.outline;
            o.source = None;
            o.seen = None;
            o.due = None;
            o.pending = None;
            self.apply_outline(None, Vec::new(), Some(OutlineEmpty::NoFile));
            return;
        };
        let (id, revision) = (doc.id, doc.revision);
        let o = &mut self.outline;
        o.source = Some((id, revision));
        o.seen = o.source;
        o.due = None;
        o.pending = None;
        // Markdown 的预览也是可以编辑的文本，同样列出标题。
        let empty = if !doc.edits_text() {
            Some(OutlineEmpty::NotText)
        } else if doc.is_large() {
            Some(OutlineEmpty::TooLarge)
        } else {
            None
        };
        let rope = doc.text().rope().filter(|_| empty.is_none());
        let Some(rope) = rope.cloned() else {
            self.apply_outline(Some(id), Vec::new(), empty.or(Some(OutlineEmpty::TooLarge)));
            return;
        };
        let language = doc.highlighter.language();
        let Some(proxy) = self.proxy.clone() else {
            let items = symbols::symbols(&rope, language);
            self.apply_outline(Some(id), items, None);
            return;
        };
        // `Rope` 的克隆只复制根节点的引用，代价与文件长度无关。
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let items = symbols::symbols(&rope, language);
            let _ = sender.send((id, revision, items));
            let _ = proxy.send_event(UserEvent::Outline);
        });
        self.outline.pending = Some(receiver);
    }

    /// 收下后台提取好的符号；不是最近一次开始的提取（期间又切换了文档或修改了内容）时丢弃。
    pub(in crate::app) fn poll_outline(&mut self) {
        let Some(receiver) = self.outline.pending.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok((id, revision, items)) if self.outline.source == Some((id, revision)) => {
                self.apply_outline(Some(id), items, None);
            }
            Ok(_) | Err(TryRecvError::Disconnected) => {}
            Err(TryRecvError::Empty) => self.outline.pending = Some(receiver),
        }
    }

    /// 换上文档 `id` 的符号 `items`；`empty` 为没有符号时的原因，为 `None` 时按 `items` 是否为空
    /// 判断。换成另一个文档的符号时，滚动、选中和正在进行的展开收起一并重置。
    fn apply_outline(
        &mut self,
        id: Option<DocumentId>,
        items: Vec<Symbol>,
        empty: Option<OutlineEmpty>,
    ) {
        let doc = id.map(|id| {
            match self
                .editor
                .documents
                .iter()
                .find(|d| d.id == id)
                .and_then(|d| d.path.clone())
            {
                Some(path) => OutlineDoc::File(path),
                None => OutlineDoc::Untitled(id),
            }
        });
        // 文件首次列出符号时，从目录的现场读入上次的收起状态。
        if let Some(key @ OutlineDoc::File(path)) = &doc
            && !self.outline.collapsed.contains_key(key)
        {
            let saved = self.saved_outline(path);
            self.outline.collapsed.insert(key.clone(), saved);
        }
        let o = &mut self.outline;
        let switched = o.shown != id;
        o.shown_doc = doc;
        o.empty = empty.or_else(|| items.is_empty().then_some(OutlineEmpty::NoSymbols));
        o.paths = symbol_paths(&items);
        o.items = items;
        o.shown = id;
        if switched {
            o.fold = None;
            o.scroll = 0.0;
            o.selected = None;
            o.revealed = None;
        } else {
            o.selected = o.selected.filter(|&i| i < o.items.len());
            o.fold = o.fold.filter(|f| f.symbol < o.items.len());
        }
        self.request_redraw();
    }

    /// 返回目录的现场中记录的、文件 `path` 的大纲里收起的符号。
    fn saved_outline(&self, path: &Path) -> HashSet<String> {
        self.editor
            .root()
            .and_then(|root| self.state.session_for(root))
            .and_then(|s| s.outline.iter().find(|(file, _)| file == path))
            .map(|(_, symbols)| symbols.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 返回目录 `root` 中各文件的大纲里收起的符号，按路径排序，用于保存现场。
    ///
    /// 以现场中已有的记录为基础，本次运行中列出过符号的文件改用当前状态；没有收起符号的文件
    /// 不记录。
    pub(in crate::app) fn outline_session(&self, root: &Path) -> Vec<(PathBuf, Vec<String>)> {
        let mut files: BTreeMap<PathBuf, Vec<String>> = self
            .state
            .session_for(root)
            .map(|s| s.outline.iter().cloned().collect())
            .unwrap_or_default();
        for (doc, set) in &self.outline.collapsed {
            let OutlineDoc::File(path) = doc else {
                continue;
            };
            if !path.starts_with(root) {
                continue;
            }
            if set.is_empty() {
                files.remove(path);
            } else {
                let mut symbols: Vec<String> = set.iter().cloned().collect();
                symbols.sort_unstable();
                files.insert(path.clone(), symbols);
            }
        }
        files.into_iter().collect()
    }

    /// 第 `i` 个符号的路径是否记为已收起。
    fn path_collapsed(&self, i: usize) -> bool {
        let o = &self.outline;
        let (Some(doc), Some(path)) = (&o.shown_doc, o.paths.get(i)) else {
            return false;
        };
        o.collapsed.get(doc).is_some_and(|set| set.contains(path))
    }

    /// 当前列出的各行：未被收起的上级挡住的符号（[`OutlineState::items`] 的下标），按顺序排列。
    pub(crate) fn outline_rows(&self) -> Vec<usize> {
        self.outline_rows_opening(None)
    }

    /// 绘制时的各行：下级正在展开或收起时，按展开后的样子排列，并给出动画中的那一段。
    pub(crate) fn outline_layout(&self) -> OutlineLayout {
        let Some(fold) = self
            .outline
            .fold
            .filter(|f| f.symbol < self.outline.items.len())
        else {
            return OutlineLayout {
                rows: self.outline_rows(),
                block: None,
                hidden: 0.0,
                turning: None,
            };
        };
        let rows = self.outline_rows_opening(Some(fold.symbol));
        let items = &self.outline.items;
        let depth = items[fold.symbol].depth;
        let block = rows.iter().position(|&i| i == fold.symbol).map(|parent| {
            let count = rows[parent + 1..]
                .iter()
                .take_while(|&&i| items[i].depth > depth)
                .count();
            (parent + 1, parent + 1 + count)
        });
        let reveal = fold.reveal();
        let count = block.map_or(0, |(start, end)| end - start);
        OutlineLayout {
            rows,
            block,
            hidden: (1.0 - reveal) * count as f32 * TREE_ROW,
            turning: Some((fold.symbol, reveal)),
        }
    }

    /// 列出的各行，`opening` 指定的符号按展开着处理（它的下级正在收起时仍要绘制）。
    fn outline_rows_opening(&self, opening: Option<usize>) -> Vec<usize> {
        let o = &self.outline;
        let mut rows = Vec::with_capacity(o.items.len());
        // 正在隐藏的下级：收起的符号的层级，比它深的符号不列出。
        let mut hidden_below: Option<usize> = None;
        for (i, symbol) in o.items.iter().enumerate() {
            if hidden_below.is_some_and(|depth| symbol.depth > depth) {
                continue;
            }
            hidden_below = None;
            rows.push(i);
            if Some(i) != opening && has_children(&o.items, i) && self.path_collapsed(i) {
                hidden_below = Some(symbol.depth);
            }
        }
        rows
    }

    /// 第 `i` 个符号是否已收起。
    pub(crate) fn outline_collapsed(&self, i: usize) -> bool {
        self.path_collapsed(i)
    }

    /// 光标所在的符号：起始行不晚于光标所在行的最后一个符号；它被收起的上级挡住时为该上级。
    pub(crate) fn outline_current(&self, rows: &[usize]) -> Option<usize> {
        let doc = self.editor.active()?;
        let line = doc.text().pos_to_line_col(doc.selection.head).0;
        let at = self.outline.items.iter().rposition(|s| s.line <= line)?;
        // 被挡住的符号属于它前面最近的那个列出的符号（收起的上级）。
        rows.iter().rev().copied().find(|&i| i <= at)
    }

    /// 高亮的行：键盘选中的符号，没有时为光标所在的符号。
    pub(crate) fn outline_highlight(&self, rows: &[usize]) -> Option<usize> {
        let selected = self.outline.selected.filter(|i| rows.contains(i));
        selected.or_else(|| self.outline_current(rows))
    }

    /// 符号列表的区域：标题行以下；大纲收起时高度为 0。
    pub(crate) fn outline_list(&self) -> Rect {
        let o = self.layout.outline;
        let header = OUTLINE_HEADER.min(o.h);
        Rect::new(o.x, o.y + header, o.w, o.h - header)
    }

    /// 列表的最大滚动量（逻辑像素）：内容比列表高时多留一行空白。
    pub(crate) fn outline_max_scroll(&self) -> f32 {
        let content = (self.outline_rows().len() as f32 + 1.0) * TREE_ROW;
        (content - self.outline_list().h).max(0.0)
    }

    /// 光标移到另一个符号时，把它所在的行滚动到可见；每帧绘制大纲之前调用。
    pub(crate) fn reveal_outline_current(&mut self) {
        let rows = self.outline_rows();
        let Some(current) = self.outline_current(&rows) else {
            return;
        };
        if self.outline.revealed == Some(current) || self.outline.selected.is_some() {
            return;
        }
        self.outline.revealed = Some(current);
        if let Some(row) = rows.iter().position(|&i| i == current) {
            self.scroll_outline_to(row);
        }
    }

    /// 滚动列表使第 `row` 行完整可见；已经可见时不滚动。
    fn scroll_outline_to(&mut self, row: usize) {
        let list = self.outline_list();
        let max = self.outline_max_scroll();
        let top = row as f32 * TREE_ROW;
        let o = &mut self.outline;
        if top < o.scroll {
            o.scroll = top;
        } else if top + TREE_ROW > o.scroll + list.h {
            o.scroll = top + TREE_ROW - list.h;
        }
        o.scroll = o.scroll.clamp(0.0, max);
    }

    /// 收起或展开第 `i` 个符号。
    pub(in crate::app) fn toggle_outline(&mut self, i: usize) {
        let o = &mut self.outline;
        let (Some(doc), Some(path)) = (o.shown_doc.clone(), o.paths.get(i)) else {
            return;
        };
        let set = o.collapsed.entry(doc).or_default();
        let expanding = set.remove(path);
        if !expanding {
            set.insert(path.clone());
        }
        let clock = self.clock(TREE_ANIMATION);
        self.outline.fold = clock.map(|clock| OutlineFold {
            symbol: i,
            expanding,
            clock,
        });
        self.request_redraw();
    }

    /// 是否有展开着的、带下级的符号：有时「全部折叠」，否则「全部展开」。
    pub(crate) fn outline_any_expanded(&self) -> bool {
        (0..self.outline.items.len())
            .any(|i| has_children(&self.outline.items, i) && !self.outline_collapsed(i))
    }

    /// 全部折叠（有展开着的符号时）或全部展开。
    pub(in crate::app) fn fold_all_outline(&mut self) {
        let collapse = self.outline_any_expanded();
        let o = &mut self.outline;
        let Some(doc) = o.shown_doc.clone() else {
            return;
        };
        let set = o.collapsed.entry(doc).or_default();
        for (i, path) in o.paths.iter().enumerate() {
            if !has_children(&o.items, i) {
                continue;
            }
            if collapse {
                set.insert(path.clone());
            } else {
                set.remove(path);
            }
        }
        o.selected = None;
        o.fold = None;
        self.outline.scroll = self.outline.scroll.min(self.outline_max_scroll());
        self.request_redraw();
    }

    /// 跳转到第 `i` 个符号并选中它的名字，焦点交给编辑区。
    pub(in crate::app) fn jump_to_outline(&mut self, i: usize) {
        let Some(symbol) = self.outline.items.get(i) else {
            return;
        };
        let (line, col, len) = (symbol.line, symbol.col, symbol.name.chars().count());
        self.outline.selected = None;
        self.outline.revealed = Some(i);
        self.jump_to(line, col, len);
    }

    /// 点击了大纲里的 `target`。
    pub(in crate::app) fn press_outline(&mut self, target: OutlineTarget) {
        match target {
            OutlineTarget::Row(i) => self.jump_to_outline(i),
            OutlineTarget::Arrow(i) => {
                self.focus = Focus::Outline;
                self.outline.selected = Some(i);
                self.toggle_outline(i);
            }
            OutlineTarget::Header => self.toggle_outline_section(),
            OutlineTarget::FoldAll => self.fold_all_outline(),
        }
    }

    /// 焦点在大纲上时按下 `key`：上下键在行之间移动，左键收起或移到上级，右键展开或移到第一个
    /// 下级，回车跳转，`Esc` 把焦点还给编辑区。
    pub(in crate::app) fn outline_key(&mut self, key: &Key) {
        let rows = self.outline_rows();
        let Some(at) = self.outline_highlight(&rows) else {
            if matches!(key, Key::Named(NamedKey::Escape)) {
                self.focus = Focus::Editor;
            }
            return;
        };
        let row = rows.iter().position(|&i| i == at).unwrap_or(0);
        let items = &self.outline.items;
        let depth = items[at].depth;
        let target = match key {
            Key::Named(NamedKey::ArrowUp) => rows.get(row.wrapping_sub(1)).copied(),
            Key::Named(NamedKey::ArrowDown) => rows.get(row + 1).copied(),
            Key::Named(NamedKey::Home) => rows.first().copied(),
            Key::Named(NamedKey::End) => rows.last().copied(),
            Key::Named(NamedKey::ArrowLeft) => {
                if has_children(items, at) && !self.outline_collapsed(at) {
                    self.toggle_outline(at);
                    return;
                }
                // 上级：它前面最近的、层级更浅的符号。
                items[..at].iter().rposition(|s| s.depth < depth)
            }
            Key::Named(NamedKey::ArrowRight) => {
                if has_children(items, at) && self.outline_collapsed(at) {
                    self.toggle_outline(at);
                    return;
                }
                has_children(items, at).then_some(at + 1)
            }
            Key::Named(NamedKey::Enter) => {
                self.jump_to_outline(at);
                return;
            }
            Key::Named(NamedKey::Escape) => {
                self.focus = Focus::Editor;
                self.request_redraw();
                return;
            }
            _ => None,
        };
        if let Some(target) = target {
            self.outline.selected = Some(target);
            let rows = self.outline_rows();
            if let Some(row) = rows.iter().position(|&i| i == target) {
                self.scroll_outline_to(row);
            }
            self.request_redraw();
        }
    }
}
