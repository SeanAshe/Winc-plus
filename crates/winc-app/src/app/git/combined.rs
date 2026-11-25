//! 「未提交的更改」标签：提交视图列出的全部更改按文件依次排进一个标签，可以从头滚到尾。
//!
//! 每个文件一段：段头（路径、状态、增删行数）和这个文件的差异。比较的内容与在提交视图中单独
//! 打开它相同（[`DiffKind`]）：暂存区视图中同一个文件在「暂存的更改」「更改」两组里各成一段。
//! 各段与差异标签功能相同：编辑、按块和按行暂存、「»」、查找、换行、逐行追溯、单栏与并排。
//!
//! 段头先列出来，各段在后台比较，看得见的先比较；Git 状态刷新、编辑器中的文件有了新的修改时
//! 重新比较，正在看的位置不动。
//!
//! 绘制和鼠标操作沿用差异标签：把一段的差异临时换进标签（[`App::with_combined_section`]）。

use std::sync::mpsc::{Receiver, Sender, channel};

use winc_editor::git::Entry;

use super::diffs::{Target, prepare};
use super::*;
use crate::app::animation::{Clock, FOLD_ANIMATION};
use crate::app::diff::{DIFF_HEADER, DiffTarget};
use crate::app::git::blame::{BlameKey, blame_label};
use crate::app::git::commit::CommitGroup;
use crate::app::{DIFF_TEXT_PAD, diff_parts};
use winc_core::Selection;
use winc_editor::diff_view::{LEFT, RIGHT};
use winc_editor::git::BlameCommit;
use winc_render::Rect;
use winit::event::MouseButton;

mod blame;
mod find;

use find::FindText;

/// 同时在后台比较的段数（取自调参）：看得见的段很快出来，又不让许多文件同时读进内存。
const RUNNING: usize = 2;

/// 段头那一行的高度（逻辑像素），与提交视图的文件行相近。段头上面还有与上一段之间的间隔，
/// 高一行（见 [`App::combined_gap`]）。
pub(crate) const SECTION_ROW: f32 = 32.0;

/// 跳到一处更改后，它的第一行离视口顶边几行：与差异标签打开时的位置相同。
const LEAD: f64 = 3.0;

/// 一段的内容。
pub(crate) enum Body {
    /// 还没有比较完。
    Waiting,
    /// 这个文件的差异，排法见 [`arrange`]。
    Ready(Box<DiffView>),
    /// 不显示内容的原因：二进制、过大、子模块、无法读取，或文字没有变化。
    Skipped(String),
}

/// 一个文件的一段。
pub(crate) struct Section {
    /// 这一段的编号，在标签里不重复，不随段的增减变化：逐行追溯按它记结果。
    pub(crate) id: u32,
    /// 文件的路径。
    pub(crate) path: PathBuf,
    /// 比较的两份内容。
    pub(crate) kind: DiffKind,
    /// 段头上显示的状态，与提交视图中这一行的字母相同。
    pub(crate) change: Change,
    /// 这一段的内容。
    pub(crate) body: Body,
    /// 是否折叠：只显示段头。
    pub(crate) folded: bool,
    /// 需要（重新）比较。
    stale: bool,
    /// 正在后台比较。
    running: bool,
    /// 比较时编辑器中这个文件的版本号；比较的是磁盘内容时为 `None`。
    revision: Option<u64>,
    /// 比较磁盘内容时文件的大小和修改时间（读文件之前取）；比较的是编辑器中的内容、远端的文件
    /// 或者取不到时为 `None`。
    stamp: Option<FileStamp>,
}

/// 文件的大小和修改时间：没变时磁盘上的内容不必重新比较。
type FileStamp = (u64, Option<winc_backend::fs::Stamp>);

/// `a` 与 `b` 两侧的内容是否都相同。
fn same_text(a: &DiffView, b: &DiffView) -> bool {
    a.old.text == b.old.text && a.new.text == b.new.text
}

/// 本机文件 `path` 现在的大小和修改时间；远端的文件、取不到时为 `None`（每次都重新比较）。
fn file_stamp(path: &Path) -> Option<FileStamp> {
    if winc_backend::is_remote(path) {
        return None;
    }
    let meta = winc_backend::fs::metadata(path).ok()?;
    Some((meta.len, meta.modified))
}

impl Section {
    /// 段头以下的显示行数：折叠、不显示内容时为 0，还没比较完时留一行写「正在比较」。
    pub(crate) fn body_rows(&self) -> f64 {
        match &self.body {
            _ if self.folded => 0.0,
            Body::Waiting => 1.0,
            Body::Ready(view) => view.scroll_rows() as f64,
            Body::Skipped(_) => 0.0,
        }
    }

    /// 增删行数；还没比较完、不显示内容时为 `None`。
    pub(crate) fn stats(&self) -> Option<(usize, usize)> {
        match &self.body {
            Body::Ready(view) => Some(view.stats()),
            _ => None,
        }
    }
}

/// 后台比较完的一段。
struct Compared {
    /// 比较时的仓库；之后换了目录时丢弃。
    root: PathBuf,
    /// 文件的路径。
    path: PathBuf,
    /// 比较的两份内容。
    kind: DiffKind,
    /// 比较时编辑器中这个文件的版本号（见 [`Section::revision`]）。
    revision: Option<u64>,
    /// 比较时磁盘上文件的大小和修改时间（见 [`Section::stamp`]）。
    stamp: Option<FileStamp>,
    /// 比较结果；不能比较时为原因。
    view: Result<DiffView, String>,
}

/// 「未提交的更改」标签。
pub(crate) struct CombinedTab {
    /// 标签对应的文档（一个空的只读文档，内容由这里绘制）。
    pub(crate) doc: DocumentId,
    /// 各段，顺序同提交视图的列表。
    pub(crate) sections: Vec<Section>,
    /// 上一次列段时 `HEAD` 的提交号：变了时左侧的内容都变了，全部重新比较。
    oid: Option<String>,
    /// 后台比较的结果。
    results: (Sender<Compared>, Receiver<Compared>),
    /// 正在把段交给后台（见 [`App::pump_combined`]）。
    pumping: bool,
    /// 最近一次跳到的更改开头的位置（显示行），以及跳过去以后的滚动位置。
    ///
    /// 滚动位置没变时「上一处」「下一处」从这一处算起：内容不满一屏、最后几处滚不到顶部时，
    /// 按滚动位置算会一直停在同一处。用户滚动以后改按滚动位置算。
    focus: Option<(f64, f64)>,
    /// 下一段的编号（见 [`Section::id`]）。
    next_id: u32,
    /// 正换进标签的段（见 [`App::with_combined_section`]）。
    pub(crate) inside: Option<usize>,
    /// 当前段：最近点过的段，按键和编辑的命令交给它；没有时为 `None`。
    pub(crate) current: Option<usize>,
    /// 按着鼠标的那一段：松开之前移动、松开都交给它。
    pressed: Option<usize>,
    /// 鼠标下的那一段：它的部件按差异的悬停绘制。
    pub(crate) hovered: Option<usize>,
    /// 各段上一帧画出的差异部件（展开、勾选、「»」、逐行追溯等），换进标签时交给差异使用。
    pub(crate) section_hits: Vec<(usize, Vec<(Rect, DiffTarget)>)>,
    /// 各段内容的代数：有段换了内容、增减了段时加一，查找据此重新开始。
    generation: u64,
    /// 查找用的快照（见 [`App::update_combined_find`]）。
    find: Option<FindText>,
    /// 查找选中的匹配在快照中的位置。
    found: Option<(usize, usize)>,
    /// 正在折叠、展开的一段。
    pub(crate) fold: Option<SectionFold>,
}

/// 一段折叠或展开的过渡：正文的高度在全高和 0 之间变化，后面的段跟着移动。
pub(crate) struct SectionFold {
    /// 这一段的编号（见 [`Section::id`]）。
    id: u32,
    /// 是否为折叠；折叠在过渡结束后才写入 [`Section::folded`]。
    folding: bool,
    /// 过渡的时钟。
    pub(crate) clock: Clock,
}

impl SectionFold {
    /// 正文露出的比例：0 为折叠，1 为展开。
    fn open(&self) -> f32 {
        let progress = self.clock.progress();
        if self.folding {
            1.0 - progress
        } else {
            progress
        }
    }
}

impl App {
    /// 测试中直接改「未提交的更改」的状态（例如换掉过渡的时钟）；Git 的测试只在 Unix 上编译。
    #[cfg(all(test, unix))]
    pub(crate) fn combined_tab_mut_for_test(&mut self) -> &mut CombinedTab {
        self.git.combined.as_mut().expect("开着「未提交的更改」")
    }
}

impl CombinedTab {
    /// 是否有一段正在折叠、展开：过渡期间逐帧绘制。
    pub(crate) fn folding(&self) -> bool {
        self.fold.is_some()
    }
}

/// 「未提交的更改」标签上可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CombinedHit {
    /// 第几段的段头：折叠或展开这一段；按住 ⌥ 时全部折叠或展开。
    Header(usize),
    /// 段头上的「打开文件」。
    OpenFile(usize),
    /// 段头上的「在差异标签中打开」。
    OpenDiff(usize),
    /// 工具栏上的「上一处更改」。
    Previous,
    /// 工具栏上的「下一处更改」。
    Next,
    /// 工具栏上的「折叠未更改的行」开关，同差异标签。
    Collapse,
    /// 工具栏上的单栏、并排切换，同差异标签。
    Layout,
    /// 工具栏最右端的「合并差异视图」开关（开着）：关掉后换回单个文件的差异。
    View,
}

/// 「未提交的更改」标签的排版。
#[derive(Debug, Clone, Copy)]
pub(crate) struct CombinedParts {
    /// 顶部的工具栏：标题、文件数、增删行数和按钮。
    pub(crate) toolbar: Rect,
    /// 工具栏下面排各段的区域。
    pub(crate) body: Rect,
}

/// 文件在组 `group` 里比较哪两份内容，同单独打开它的差异（见 `App::commit_diff_kind`）。
fn group_kind(entry: &Entry, group: CommitGroup, staging: bool) -> DiffKind {
    match group {
        _ if entry.conflicted() || !staging => DiffKind::Whole,
        CommitGroup::Staged => DiffKind::Staged,
        CommitGroup::Changes => DiffKind::Unstaged,
        CommitGroup::Conflicts | CommitGroup::Untracked => DiffKind::Whole,
    }
}

/// 文件在组 `group` 里显示的状态，同提交视图中这一行的字母。
fn group_change(entry: &Entry, group: CommitGroup, staging: bool) -> Change {
    match (staging, group) {
        (true, CommitGroup::Staged) => entry.staged.unwrap_or_else(|| entry.shown()),
        (true, CommitGroup::Changes) => entry.unstaged.unwrap_or_else(|| entry.shown()),
        _ => entry.shown(),
    }
}

impl App {
    /// 「未提交的更改」标签；没有打开时为 `None`。
    pub(crate) fn combined_tab(&self) -> Option<&CombinedTab> {
        self.git.combined.as_ref()
    }

    /// 当前标签是不是「未提交的更改」。
    ///
    /// 某一段换进标签期间（[`Self::with_combined_section`]）不算：标签此时就是那一段的差异，
    /// 按差异标签处理。
    pub(crate) fn combined_active(&self) -> bool {
        let active = self.editor.active().map(|d| d.id);
        (self.combined_tab()).is_some_and(|tab| Some(tab.doc) == active && tab.inside.is_none())
    }

    /// 当前标签是不是提交视图中文件的差异：标题栏上有「合并差异视图」开关。
    pub(crate) fn diff_combinable(&self) -> bool {
        let active = self.editor.active().map(|d| d.id);
        active.is_some_and(|id| self.git.diff_tabs.contains_key(&id))
    }

    /// 「未提交的更改」标签对应的文档在第几个。
    fn combined_index(&self) -> Option<usize> {
        let id = self.combined_tab()?.doc;
        self.editor.documents.iter().position(|d| d.id == id)
    }

    /// 标签的排版：工具栏和排各段的区域。
    pub(crate) fn combined_parts(&self) -> CombinedParts {
        let editor = self.layout.editor;
        let toolbar = Rect::new(editor.x, editor.y, editor.w, DIFF_HEADER);
        let body = Rect::new(
            editor.x,
            toolbar.bottom(),
            editor.w,
            (editor.bottom() - toolbar.bottom()).max(0.0),
        );
        CombinedParts { toolbar, body }
    }

    /// 段头上面与上一段之间的间隔（逻辑像素）：一行高，画成一条斜线纹，分开两个文件。
    pub(crate) fn combined_gap(&self) -> f32 {
        self.fonts.line_height
    }

    /// 段头的高度（逻辑像素）：上面的间隔和下面的一行。
    pub(crate) fn combined_header(&self) -> f32 {
        self.combined_gap() + SECTION_ROW
    }

    /// 段头占几个显示行（可以是小数）。
    pub(crate) fn combined_header_rows(&self) -> f64 {
        f64::from(self.combined_header() / self.fonts.line_height)
    }

    /// 各段的起始位置（显示行，含段头上面的间隔），最后一项是总长。第一段没有间隔：它的起始
    /// 位置在 0 之上一个间隔处。
    ///
    /// 一段正在展开、折叠未更改的内容时，按过渡中还没露出的高度算短，后面的段跟着移动。
    pub(crate) fn combined_starts(&self) -> Vec<f64> {
        let header = self.combined_header_rows();
        let lh = self.fonts.line_height;
        let moving = self
            .motion
            .diff_fold
            .as_ref()
            .and_then(|m| Some((m.section?, m)));
        // 段与段之间才有间隔：第一段的间隔放在滚动范围的顶边之上，看不见。
        let tab_fold = self.combined_tab().and_then(|t| t.fold.as_ref());
        let mut at = -f64::from(self.combined_gap() / lh);
        let mut starts = vec![at];
        for section in self.combined_tab().map_or(&[][..], |t| &t.sections) {
            let hidden = match (moving, &section.body) {
                (Some((id, m)), Body::Ready(view)) if id == section.id && view.is_open(m.gap) => {
                    f64::from(m.hidden(lh) / lh)
                }
                _ => 0.0,
            };
            let open = match tab_fold {
                Some(fold) if fold.id == section.id => f64::from(fold.open()),
                _ => 1.0,
            };
            at += header + (section.body_rows() - hidden).max(0.0) * open;
            starts.push(at);
        }
        starts
    }

    /// 各段段头那一行的位置（显示行，间隔之下）：跳到某个文件时停在这里，顶边不留间隔。
    pub(crate) fn combined_heads(&self) -> Vec<f64> {
        let gap = f64::from(self.combined_gap() / self.fonts.line_height);
        let starts = self.combined_starts();
        let count = starts.len().saturating_sub(1);
        starts[..count].iter().map(|s| s + gap).collect()
    }

    /// 正换进标签的段的编号（见 [`Section::id`]）；没有时为 `None`。
    pub(crate) fn combined_inside_id(&self) -> Option<u32> {
        let tab = self.combined_tab()?;
        Some(tab.sections.get(tab.inside?)?.id)
    }

    /// 编号为 `id` 的段在第几段。
    pub(crate) fn combined_section_index(&self, id: u32) -> Option<usize> {
        self.combined_tab()?
            .sections
            .iter()
            .position(|s| s.id == id)
    }

    /// 标签的总长（显示行），滚动的范围按它算。
    pub(crate) fn combined_rows(&self) -> usize {
        self.combined_starts().last().copied().unwrap_or(0.0).ceil() as usize
    }

    /// 当前的滚动位置（显示行）。
    fn combined_scroll(&self) -> f64 {
        let index = self.combined_index();
        index.map_or(0.0, |i| self.editor.documents[i].scroll_line.max(0.0))
    }

    /// 视口顶边落在第几段、离这一段开头多远：重新排版后据此把这个位置放回原处。
    fn combined_anchor(&self) -> Option<(usize, f64)> {
        let scroll = self.combined_scroll();
        let starts = self.combined_starts();
        let i = starts.partition_point(|&s| s <= scroll).checked_sub(1)?;
        (i + 1 < starts.len()).then(|| (i, scroll - starts[i]))
    }

    /// 视口顶边所在那一段（重新排版之前）的段头那一行回到视口顶边。
    fn combined_restore_head(&mut self, anchor: Option<(usize, f64)>) {
        if let Some((i, _)) = anchor {
            let gap = f64::from(self.combined_gap() / self.fonts.line_height);
            self.combined_restore((i, gap));
        }
    }

    /// 让第 `i` 段的开头之后 `offset` 行处回到视口顶边（见 [`Self::combined_anchor`]）。
    fn combined_restore(&mut self, (i, offset): (usize, f64)) {
        let starts = self.combined_starts();
        let Some(&start) = starts.get(i) else {
            return;
        };
        let end = starts.get(i + 1).copied().unwrap_or(start);
        let before = self.combined_scroll();
        self.set_combined_scroll(start + offset.min(end - start));
        // 平滑滚动的起点跟着移动：屏幕上正在滚过的内容不因上面的段变高、变矮而跳开。
        let delta = self.combined_scroll() - before;
        if let Some(doc) = self.combined_tab().map(|t| t.doc)
            && delta != 0.0
        {
            self.shift_scroll_glide(doc, delta);
        }
    }

    /// 把滚动位置设为 `row`，限制在这个标签可滚动的范围内（它不是当前标签时也按它自己算）。
    fn set_combined_scroll(&mut self, row: f64) {
        let Some(index) = self.combined_index() else {
            return;
        };
        let (rows, view) = (self.combined_rows(), self.combined_parts().body.h);
        let max = crate::layout::editor_max_scroll(rows, view, self.fonts.line_height);
        let row = row.clamp(0.0, max.max(0.0));
        set_scroll_line(&mut self.editor.documents[index], row);
        self.request_redraw();
    }

    /// 打开「未提交的更改」（命令面板、提交视图），`reveal` 是要停在的文件和比较的内容。
    ///
    /// 已经打开时切换过去；否则用提交视图的预览标签（没有时新开一个），与单个文件的差异共用
    /// 一个标签。
    pub(in crate::app) fn open_combined(&mut self, reveal: Option<(PathBuf, DiffKind)>) {
        if self.git.root.is_none() {
            self.flash(t!("git-not-a-repository"));
            return;
        }
        match self.combined_index() {
            Some(index) => self.activate_tab(index),
            None => {
                let preview = (self.git.commit.preview)
                    .and_then(|id| self.editor.documents.iter().position(|d| d.id == id));
                let index = match preview {
                    Some(index) => {
                        self.activate_tab(index);
                        index
                    }
                    None => self.editor.show_output(t!("git-combined-title"), ""),
                };
                self.start_combined_in(index);
            }
        }
        self.refresh_combined();
        if let Some((path, kind)) = reveal {
            self.combined_reveal_file(&path, kind);
        }
        self.sync_combined_title();
        self.relayout();
        self.request_redraw();
    }

    /// 让第 `index` 个文档成为「未提交的更改」：原来是差异时差异不要了，标签留着。
    fn start_combined_in(&mut self, index: usize) {
        let doc = &mut self.editor.documents[index];
        let id = doc.id;
        doc.diff = None;
        self.git.diff_tabs.remove(&id);
        self.git.commit.preview = Some(id);
        self.git.combined = Some(CombinedTab {
            doc: id,
            sections: Vec::new(),
            oid: None,
            results: channel(),
            pumping: false,
            focus: None,
            next_id: 0,
            inside: None,
            current: None,
            pressed: None,
            hovered: None,
            section_hits: Vec::new(),
            generation: 0,
            find: None,
            found: None,
            fold: None,
        });
    }

    /// 标签的标题跟着视口顶边的那一段：同这个文件单独打开时的差异标题。
    fn sync_combined_title(&mut self) {
        let Some(index) = self.combined_index() else {
            return;
        };
        let title = (self.combined_anchor())
            .and_then(|(i, _)| self.combined_tab()?.sections.get(i))
            .map_or_else(
                || t!("git-combined-title").to_owned(),
                |s| super::diffs::diff_title(&s.path, s.kind),
            );
        let doc = &mut self.editor.documents[index];
        if doc.title.as_deref() != Some(title.as_str()) {
            doc.title = Some(title);
        }
    }

    /// 切换「合并差异视图」（`git.combinedDiff`）：差异标题栏上的按钮和命令面板，与设置中的
    /// 是同一项；正在看的视图随之切换（见 [`Self::apply_combined_view`]）。
    pub(in crate::app) fn toggle_combined_diff(&mut self) {
        self.settings_toggle(crate::layout::Setting::GitCombinedDiff);
    }

    /// 提交视图列出的各个文件：（路径，比较的内容，状态），顺序同列表，收起的组和目录也算。
    fn combined_entries(&self) -> Vec<(PathBuf, DiffKind, Change)> {
        let Some(status) = self.git.status.as_ref() else {
            return Vec::new();
        };
        let staging = self.git.commit.staging();
        let rows = self.commit_rows();
        rows.files()
            .filter_map(|(group, index)| {
                let entry = status.entries.get(index)?;
                let kind = group_kind(entry, group, staging);
                Some((
                    entry.path.clone(),
                    kind,
                    group_change(entry, group, staging),
                ))
            })
            .collect()
    }

    /// 按现在的 Git 状态重新列出各段：新增的文件加进来，没有了的去掉，留下的保留折叠和已有的
    /// 内容，需要时在后台重新比较。视口顶边的内容不动。
    ///
    /// 标签已经关掉时丢掉它的状态。
    pub(in crate::app) fn refresh_combined(&mut self) {
        if self.git.combined.is_some() && self.combined_index().is_none() {
            self.git.combined = None;
        }
        if self.git.combined.is_none() {
            return;
        }
        let anchor = self.combined_anchor();
        let entries = self.combined_entries();
        let oid = self.git.status.as_ref().and_then(|s| s.oid.clone());
        let revisions: Vec<Option<u64>> = (entries.iter())
            .map(|(path, _, _)| self.editor_revision(path))
            .collect();
        // 没在编辑器里打开的文件看磁盘上的大小和修改时间：没变时不重新比较。
        let stamps: Vec<Option<FileStamp>> = (entries.iter().zip(&revisions))
            .map(|((path, kind, _), revision)| {
                (*kind == DiffKind::Whole && revision.is_none())
                    .then(|| file_stamp(path))
                    .flatten()
            })
            .collect();
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let moved = tab.oid != oid;
        tab.oid = oid;
        // 按下标记的状态先换成编号：段增减、换了顺序以后仍然指向原来的文件。
        let id_of = |at: Option<usize>| Some(tab.sections.get(at?)?.id);
        let (current, pressed, hovered) =
            (id_of(tab.current), id_of(tab.pressed), id_of(tab.hovered));
        let anchor = anchor.map(|(i, offset)| (i, id_of(Some(i)), offset));
        let section_hits: Vec<(u32, Vec<(Rect, DiffTarget)>)> = (tab.section_hits.drain(..))
            .filter_map(|(at, hits)| Some((tab.sections.get(at)?.id, hits)))
            .collect();
        let before: Vec<u32> = tab.sections.iter().map(|s| s.id).collect();
        let mut old = std::mem::take(&mut tab.sections);
        let entries = entries.into_iter().zip(revisions).zip(stamps);
        for (((path, kind, change), revision), stamp) in entries {
            let found = old.iter().position(|s| s.path == path && s.kind == kind);
            let section = match found.map(|i| old.swap_remove(i)) {
                // 提交号和状态都没变，比较的是编辑器里的同一个版本、或者磁盘上没改过的文件：
                // 内容不会变，不必重新比较。有一侧是暂存区的总是重新比较：暂存区可能在别处改变，
                // 同差异标签。
                Some(mut section) => {
                    let unchanged = match revision {
                        Some(_) => section.revision == revision,
                        None => {
                            section.revision.is_none() && stamp.is_some() && section.stamp == stamp
                        }
                    };
                    let same = !moved
                        && section.kind == DiffKind::Whole
                        && section.change == change
                        && unchanged;
                    section.stale |= !same;
                    section.change = change;
                    section
                }
                None => Section {
                    id: {
                        tab.next_id += 1;
                        tab.next_id
                    },
                    path,
                    kind,
                    change,
                    body: Body::Waiting,
                    folded: false,
                    stale: true,
                    running: false,
                    revision: None,
                    stamp: None,
                },
            };
            tab.sections.push(section);
        }
        let index_of = |id: Option<u32>| tab.sections.iter().position(|s| Some(s.id) == id);
        (tab.current, tab.pressed, tab.hovered) =
            (index_of(current), index_of(pressed), index_of(hovered));
        tab.section_hits = (section_hits.into_iter())
            .filter_map(|(id, hits)| Some((index_of(Some(id))?, hits)))
            .collect();
        // 段增减、换了顺序时查找重新开始；内容变了的段比较完时另算（见 `poll_combined`）。
        if tab.sections.iter().map(|s| s.id).ne(before) {
            tab.generation += 1;
        }
        if let Some((i, id, offset)) = anchor {
            // 视口顶边所在的那一段按编号找回来（前面增减了段也不错位）；它没了时停在原来位置上
            // 现在的那一段。
            let count = self.combined_tab().map_or(0, |t| t.sections.len());
            let i = id
                .and_then(|id| self.combined_section_index(id))
                .unwrap_or_else(|| i.min(count.saturating_sub(1)));
            self.combined_restore((i, offset));
        }
        self.pump_combined();
        self.request_redraw();
    }

    /// 编辑器中打开着的 `path` 的版本号；没有打开时为 `None`。
    fn editor_revision(&self, path: &Path) -> Option<u64> {
        let canonical = self.editor.document_path(path);
        (self.editor.documents.iter())
            .find(|d| d.path.as_deref() == Some(canonical.as_path()) && d.holds_file_text())
            .map(|d| d.revision)
    }

    /// 排版时（绘制之前）整理「未提交的更改」：各段跟上编辑器中的修改、按设置断行，标题跟着
    /// 视口顶边的文件；各段高度变了时视口顶边的内容不动。
    ///
    /// 在算各段的位置、绘制之前做完：绘制中途改变高度，已经排好的段头和正文会错开。
    pub(crate) fn prepare_combined(&mut self) {
        let Some(index) = self.combined_index().filter(|_| self.combined_active()) else {
            return;
        };
        let anchor = self.combined_anchor();
        let before = self.combined_starts();
        for i in self.combined_out_of_sync() {
            self.draw_combined_section(i, |app| app.sync_diff_source(index));
        }
        self.sync_combined_wrap();
        // 折叠的过渡中标签逐帧变短：滚到底时跟着往回收，不在结束时跳一下。
        if self.combined_tab().is_some_and(CombinedTab::folding) {
            self.clamp_scroll();
        }
        if self.combined_starts() != before
            && let Some(anchor) = anchor
        {
            self.combined_restore(anchor);
        }
        self.sync_combined_title();
    }

    /// 右侧跟着的编辑器文档有了新版本、需要重新比较的段（见 `App::sync_diff_source`）：每帧
    /// 只换进这些段，不逐段换进换出。
    fn combined_out_of_sync(&self) -> Vec<usize> {
        let Some(tab) = self.combined_tab() else {
            return Vec::new();
        };
        let revision = |path: &Path| {
            (self.editor.documents.iter())
                .find(|d| d.path.as_deref() == Some(path))
                .map(|d| d.revision)
        };
        (tab.sections.iter().enumerate())
            .filter_map(|(i, section)| {
                let Body::Ready(view) = &section.body else {
                    return None;
                };
                let now = revision(view.new.path.as_deref()?)?;
                (view.source != Some(now)).then_some(i)
            })
            .collect()
    }

    /// 后台重新比较完的差异（大文件在编辑器中改了以后，见 `App::poll_rediffs`）属于「未提交的
    /// 更改」标签 `doc` 中哪一段：右侧是 `path`、左侧是 `old` 的那一段。
    pub(in crate::app) fn combined_view_mut(
        &mut self,
        doc: DocumentId,
        path: &Path,
        old: &Rope,
    ) -> Option<&mut DiffView> {
        let tab = self.git.combined.as_mut().filter(|t| t.doc == doc)?;
        tab.sections.iter_mut().find_map(|s| match &mut s.body {
            Body::Ready(view)
                if view.new.path.as_deref() == Some(path) && view.old.text == *old =>
            {
                Some(&mut **view)
            }
            _ => None,
        })
    }

    /// 绘制前按屏幕上的位置重新找鼠标下的那一段：滚动时鼠标没动，下面的段也会换。
    pub(crate) fn note_combined_hover(&mut self) {
        let (x, y) = self.pointer.mouse;
        let under = self.combined_pointer_section(x, y);
        if let Some(tab) = self.git.combined.as_mut() {
            tab.hovered = under;
        }
    }

    /// 需要比较的段交给后台，同时最多 [`RUNNING`] 段；先比较看得见的，再按顺序比较其余的。
    ///
    /// 没有窗口时（测试）后台的工作当场做完，收结果时又会回到这里：只由最外层的一次往下交。
    fn pump_combined(&mut self) {
        match self.git.combined.as_mut() {
            Some(tab) if !tab.pumping => tab.pumping = true,
            _ => return,
        }
        while let Some(i) = self.next_stale_section() {
            self.compare_section(i);
        }
        if let Some(tab) = self.git.combined.as_mut() {
            tab.pumping = false;
        }
    }

    /// 下一段要比较的：看得见的在前；已经有 [`RUNNING`] 段在比较时为 `None`。
    fn next_stale_section(&self) -> Option<usize> {
        let tab = self.combined_tab()?;
        if tab.sections.iter().filter(|s| s.running).count() >= RUNNING {
            return None;
        }
        let scroll = self.combined_scroll();
        let visible = f64::from(self.combined_parts().body.h / self.fonts.line_height);
        let starts = self.combined_starts();
        let shown = |i: &usize| starts[*i + 1] > scroll && starts[*i] < scroll + visible;
        let waiting = |i: &usize| tab.sections[*i].stale && !tab.sections[*i].running;
        let all = 0..tab.sections.len();
        (all.clone().filter(shown).find(waiting)).or_else(|| all.clone().find(waiting))
    }

    /// 在后台比较第 `i` 段；结果由 [`Self::poll_combined`] 收下。
    fn compare_section(&mut self, i: usize) {
        let (Some(root), Some(tab)) = (self.git.root.clone(), self.combined_tab()) else {
            return;
        };
        let Some(section) = tab.sections.get(i) else {
            return;
        };
        let (path, kind) = (section.path.clone(), section.kind);
        let sender = tab.results.0.clone();
        let snapshot = self.editor_snapshot(&path);
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let section = &mut tab.sections[i];
        section.stale = false;
        // 编辑器中打开的是大文件：不比较，与单独打开它的差异相同。
        let Ok(snapshot) = snapshot else {
            let name = winc_editor::display_name(&path).into_owned();
            section.body = Body::Skipped(t!("git-diff-too-large", name = name));
            section.revision = None;
            section.stamp = None;
            return;
        };
        section.running = true;
        let revision = snapshot.as_ref().map(|s| s.revision);
        let oid = tab.oid.clone();
        let links = self.editor.folder_links();
        self.spawn_git(UserEvent::GitCombined, move |tool| {
            // 先取大小和修改时间再读：读的时候文件又变了，下次刷新时仍看得出来。
            let stamp = (kind == DiffKind::Whole && revision.is_none())
                .then(|| file_stamp(&path))
                .flatten();
            let view = match &tool {
                Ok(git) => prepare(git, &root, &path, snapshot, oid.as_deref(), kind, &links),
                Err(e) => Err(unusable(e)),
            };
            let _ = sender.send(Compared {
                root,
                path,
                kind,
                revision,
                stamp,
                view,
            });
        });
    }

    /// 收下后台比较完的段：换上新的内容（已经展开的未改动内容保持展开），视口顶边的内容不动；
    /// 再交下一批给后台。
    pub(in crate::app) fn poll_combined(&mut self) {
        let mut done = Vec::new();
        if let Some(tab) = self.combined_tab() {
            while let Ok(compared) = tab.results.1.try_recv() {
                done.push(compared);
            }
        }
        if done.is_empty() {
            return;
        }
        let anchor = self.combined_anchor();
        for compared in done {
            if self.git.root.as_deref() != Some(compared.root.as_path()) {
                continue;
            }
            let Some(tab) = self.git.combined.as_mut() else {
                return;
            };
            let found = (tab.sections.iter_mut())
                .find(|s| s.path == compared.path && s.kind == compared.kind);
            let Some(section) = found else {
                continue;
            };
            section.running = false;
            section.revision = compared.revision;
            section.stamp = compared.stamp;
            let kept = matches!((&section.body, &compared.view),
                (Body::Ready(old), Ok(view)) if !view.is_identical() && same_text(old, view));
            if !kept {
                tab.generation += 1;
            }
            section.body = match compared.view {
                Ok(view) if view.is_identical() => {
                    Body::Skipped(t!("git-combined-no-text-change").to_owned())
                }
                // 两侧的内容都没变：留着原来的差异，高亮、光标、展开的内容、追溯和查找都不用重来。
                Ok(view) if kept => {
                    let mut body = std::mem::replace(&mut section.body, Body::Waiting);
                    if let Body::Ready(old) = &mut body {
                        old.source = view.source;
                        old.old.title = view.old.title;
                        old.new.title = view.new.title;
                    }
                    body
                }
                Ok(mut view) => {
                    arrange(
                        &mut view,
                        self.editor.diff_unified,
                        self.editor.diff_collapse,
                    );
                    if let Body::Ready(old) = &section.body {
                        view.inherit(old);
                    }
                    Body::Ready(Box::new(view))
                }
                Err(reason) => Body::Skipped(reason),
            };
        }
        if let Some(anchor) = anchor {
            self.combined_restore(anchor);
        }
        self.pump_combined();
        self.request_redraw();
    }

    /// 滚到 `path` 那一段（比较 `kind` 的那一段，没有时取这个文件的第一段），段头在视口顶边。
    pub(in crate::app) fn combined_reveal_file(&mut self, path: &Path, kind: DiffKind) {
        let Some(tab) = self.combined_tab() else {
            return;
        };
        let sections = &tab.sections;
        let found = (sections
            .iter()
            .position(|s| s.path == path && s.kind == kind))
        .or_else(|| sections.iter().position(|s| s.path == path));
        if let Some(i) = found {
            let head = self.combined_heads()[i];
            self.set_combined_scroll(head);
        }
    }

    /// 折叠或展开第 `i` 段（点段头），带过渡。视口顶边在这一段里时停在它的段头上。
    pub(in crate::app) fn toggle_combined_section(&mut self, i: usize) {
        let clock = self.clock(FOLD_ANIMATION);
        self.fold_combined_section(i, clock);
    }

    /// 同 [`Self::toggle_combined_section`]，过渡按 `clock` 计时；为 `None`（动态效果关闭、
    /// 没有窗口）或者这一段没有差异时直接到位。
    pub(in crate::app) fn fold_combined_section(&mut self, i: usize, clock: Option<Clock>) {
        self.finish_combined_fold(true);
        let head = self.combined_heads().get(i).copied();
        let scroll = self.combined_scroll();
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let Some(section) = tab.sections.get_mut(i) else {
            return;
        };
        let folding = !section.folded;
        match clock.filter(|_| matches!(section.body, Body::Ready(_))) {
            // 过渡中这一段按展开算，正文的高度由过渡决定；折叠在过渡结束时写入。
            Some(clock) => {
                section.folded = false;
                tab.fold = Some(SectionFold {
                    id: section.id,
                    folding,
                    clock,
                });
            }
            None => section.folded = folding,
        }
        if let Some(head) = head.filter(|&h| scroll > h) {
            self.set_combined_scroll(head);
        }
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 结束进行中的段折叠、展开过渡：`force` 为假时只在时钟走完后结束（每帧绘制之前），为真
    /// 时立即结束（改变折叠状态之前）。折叠的过渡此时写入折叠。
    pub(in crate::app) fn finish_combined_fold(&mut self, force: bool) {
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let Some(fold) = tab.fold.take_if(|f| force || f.clock.done()) else {
            return;
        };
        if fold.folding
            && let Some(section) = tab.sections.iter_mut().find(|s| s.id == fold.id)
        {
            section.folded = true;
        }
        if self.combined_active() {
            self.clamp_scroll();
        }
        self.request_redraw();
    }

    /// 第 `i` 段的正文展开的比例：0 为折叠，1 为展开，过渡中在两者之间。段头的箭头据此转动。
    pub(crate) fn combined_section_open(&self, i: usize) -> f32 {
        let Some(tab) = self.combined_tab() else {
            return 1.0;
        };
        let Some(section) = tab.sections.get(i) else {
            return 1.0;
        };
        match &tab.fold {
            Some(fold) if fold.id == section.id => fold.open(),
            _ if section.folded => 0.0,
            _ => 1.0,
        }
    }

    /// 全部折叠（`fold`）或全部展开（按住 ⌥ 点段头）；停在视口顶边所在那一段的段头上。
    pub(in crate::app) fn fold_all_combined(&mut self, fold: bool) {
        self.finish_combined_fold(true);
        let anchor = self.combined_anchor();
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        for section in &mut tab.sections {
            section.folded = fold;
        }
        self.combined_restore_head(anchor);
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 按设置给各段断行（`editor.wordWrap`），宽度按各段的文字区算；断行变了时视口顶边的内容
    /// 不动。
    fn sync_combined_wrap(&mut self) {
        let wrap = self.config.word_wrap == Some(true);
        let tab_width = self.editor.active_tab_width();
        let (body, cell) = (self.combined_parts().body, self.metrics.cell);
        let anchor = self.combined_anchor();
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let mut changed = false;
        for section in &mut tab.sections {
            let Body::Ready(view) = &mut section.body else {
                continue;
            };
            let parts = diff_parts(body, view, cell);
            let text_w = parts.panes[LEFT].text.w.min(parts.panes[RIGHT].text.w);
            let cols = ((text_w - 2.0 * DIFF_TEXT_PAD) / cell).floor().max(1.0) as usize;
            changed |= view.set_wrap(wrap.then_some((cols, tab_width)));
        }
        if changed && let Some(anchor) = anchor {
            self.combined_restore(anchor);
        }
    }

    /// 各段是否折叠未更改的行：工具栏上的开关据此换图标。没有比较完的段时按打开算。
    pub(crate) fn combined_collapses(&self) -> bool {
        let views = self.combined_tab().into_iter().flat_map(|t| &t.sections);
        let mut views = views.filter_map(|s| match &s.body {
            Body::Ready(view) => Some(view),
            _ => None,
        });
        views.next().is_none_or(|view| view.collapses())
    }

    /// 「折叠未更改的内容」变了（工具栏、差异标题栏上的开关和设置是同一项）：各段一起折叠或
    /// 展开，视口顶边所在的段停在原处。
    pub(in crate::app) fn apply_combined_collapse(&mut self, on: bool) {
        let anchor = self.combined_anchor();
        if let Some(tab) = self.git.combined.as_mut() {
            for section in &mut tab.sections {
                if let Body::Ready(view) = &mut section.body {
                    view.set_collapse(on, 0);
                }
            }
        }
        self.combined_restore_head(anchor);
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 单栏、并排变了（`editor.diffInline`）：各段跟着换，视口顶边所在的段停在原处。
    pub(in crate::app) fn apply_combined_layout(&mut self, unified: bool) {
        let anchor = self.combined_anchor();
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        for section in &mut tab.sections {
            if let Body::Ready(view) = &mut section.body {
                view.set_unified(unified);
            }
        }
        self.combined_restore_head(anchor);
    }

    /// 「合并差异视图」开关变了（标题栏的按钮、设置、命令面板）：正在看的差异原地换一种
    /// 显示，标签不变。
    ///
    /// 打开时，正在看的是提交视图中文件的差异就换成「未提交的更改」，停在这个文件；关闭时，
    /// 正在看「未提交的更改」就换回视口顶边那个文件的差异，停在原来的位置。
    pub(in crate::app) fn apply_combined_view(&mut self) {
        let on = self.config.git_combined_diff == Some(true);
        let Some(index) = self.editor.active else {
            return;
        };
        let id = self.editor.documents[index].id;
        if on {
            let Some(tab) = self.git.diff_tabs.get(&id) else {
                return;
            };
            let (path, kind) = (tab.path.clone(), tab.kind);
            // 已经有「未提交的更改」标签（这是从它打开的单独的差异）：换到那个标签，这个留着。
            if self.combined_index().is_none() {
                self.start_combined_in(index);
            }
            self.open_combined(Some((path, kind)));
            return;
        }
        if !self.combined_active() {
            return;
        }
        let Some((i, offset)) = self.combined_anchor() else {
            return;
        };
        let header = self.combined_header_rows();
        let Some(mut tab) = self.git.combined.take() else {
            return;
        };
        self.git.blame.forget(id);
        let section = tab.sections.swap_remove(i);
        let Body::Ready(mut view) = section.body else {
            // 这一段还没比较完：按单个文件的差异重新读。
            self.git.preview_seq += 1;
            let seq = self.git.preview_seq;
            self.fetch_diff(&section.path, Target::Preview(seq), section.kind);
            return;
        };
        view.aligned = false;
        let title = super::diffs::diff_title(&section.path, section.kind);
        let doc = &mut self.editor.documents[index];
        doc.diff = Some(view);
        doc.title = Some(title);
        set_scroll_line(doc, (offset - header).max(0.0));
        let tab = super::diffs::DiffTab::new(section.path, tab.oid, section.kind);
        self.git.diff_tabs.insert(id, tab);
        self.git.commit.preview = Some(id);
        self.clamp_scroll();
        self.relayout();
        self.request_redraw();
    }

    /// 各处更改：（开头的位置（显示行），第几段，段里第几处），按顺序；折叠的段不算。
    fn combined_hunks(&self) -> Vec<(f64, usize, usize)> {
        let Some(tab) = self.combined_tab() else {
            return Vec::new();
        };
        let starts = self.combined_starts();
        let header = self.combined_header_rows();
        let mut out = Vec::new();
        for (i, section) in tab.sections.iter().enumerate() {
            if let (false, Body::Ready(view)) = (section.folded, &section.body) {
                let base = starts[i] + header;
                out.extend((0..view.hunks.len()).map(|h| (base + view.hunk_pos(h), i, h)));
            }
        }
        out
    }

    /// 最近一次跳到的更改开头的位置（显示行）；之后滚动过、或者还没跳过时为 `None`。
    pub(crate) fn combined_focus(&self) -> Option<f64> {
        let (pos, at) = self.combined_tab()?.focus?;
        (at == self.combined_scroll()).then_some(pos)
    }

    /// 跳到下一处（`forward`）或上一处更改，可以跨文件；跳到的更改离视口顶边 [`LEAD`] 行。
    ///
    /// 已经是最后一处、第一处时在状态栏说明。
    pub(in crate::app) fn step_combined_change(&mut self, forward: bool) {
        let at = self
            .combined_focus()
            .unwrap_or(self.combined_scroll() + LEAD);
        let hunks = self.combined_hunks();
        let target = if forward {
            hunks.into_iter().find(|h| h.0 > at + 0.5)
        } else {
            hunks.into_iter().rev().find(|h| h.0 < at - 0.5)
        };
        let Some((pos, i, hunk)) = target else {
            let none = if forward {
                t!("git-change-no-next")
            } else {
                t!("git-change-no-previous")
            };
            self.flash(none);
            return;
        };
        // 交给那一段跳：光标、当前标记和高亮与差异标签相同。
        if let Some(tab) = self.git.combined.as_mut() {
            tab.current = Some(i);
        }
        self.with_combined_section(i, |app| app.go_to_hunk(hunk));
        let scroll = self.combined_scroll();
        if let Some(tab) = self.git.combined.as_mut() {
            tab.focus = Some((pos, scroll));
        }
    }

    /// 跳到下一个（`forward`）或上一个文件的段头。
    pub(in crate::app) fn step_combined_file(&mut self, forward: bool) {
        let scroll = self.combined_scroll();
        let heads = self.combined_heads();
        let target = if forward {
            heads.iter().copied().find(|&s| s > scroll + 0.01)
        } else {
            heads.iter().copied().rev().find(|&s| s < scroll - 0.01)
        };
        if let Some(start) = target {
            self.set_combined_scroll(start);
            self.glide_scroll(scroll);
        }
    }

    /// 打开第 `i` 段的文件（段头上的「打开文件」）。
    pub(in crate::app) fn open_combined_file(&mut self, i: usize) {
        let Some(section) = self.combined_tab().and_then(|t| t.sections.get(i)) else {
            return;
        };
        let path = section.path.clone();
        if !self.may_be_file(&path) {
            self.flash(t!("git-commit-open-deleted"));
            return;
        }
        self.open_path(&path);
        self.focus = Focus::Editor;
    }

    /// 在单独的差异标签中打开第 `i` 段（段头上的按钮）：可以在那里编辑、按块暂存和逐行追溯。
    pub(in crate::app) fn open_combined_diff(&mut self, i: usize) {
        let Some(section) = self.combined_tab().and_then(|t| t.sections.get(i)) else {
            return;
        };
        let (path, kind) = (section.path.clone(), section.kind);
        self.fetch_diff(&path, Target::Open, kind);
    }

    /// 第 `i` 段的差异露出来的部分：（交给差异排版的区域，这一部分顶边在这一段差异里的位置）。
    ///
    /// 区域的顶上留出差异标题栏的高度（不绘制），其下与这一段露出来的显示行对齐；这一段折叠、
    /// 没有差异或完全看不见时为 `None`。
    pub(crate) fn combined_section_area(&self, i: usize) -> Option<(Rect, f64)> {
        let (area, pos, shown) = self.combined_section_frame(i)?;
        shown.then_some((area, pos))
    }

    /// 同 [`Self::combined_section_area`]，但这一段看不见时也给出位置：按整个正文区算，顶边
    /// 是视口顶边在这一段里的位置（可以在这一段之外）。按键、编辑的命令据此换入。
    fn combined_section_frame(&self, i: usize) -> Option<(Rect, f64, bool)> {
        self.section_frame_in(&self.combined_starts(), i)
    }

    /// 同 [`Self::combined_section_frame`]，各段的位置 `starts` 已经算好（逐段查找时只算一次）。
    fn section_frame_in(&self, starts: &[f64], i: usize) -> Option<(Rect, f64, bool)> {
        let section = self.combined_tab()?.sections.get(i)?;
        let (false, Body::Ready(_)) = (section.folded, &section.body) else {
            return None;
        };
        let body = self.combined_parts().body;
        let lh = self.fonts.line_height;
        let start = starts[i] + self.combined_header_rows();
        // 这一段的高度按各段的位置算：展开、折叠的过渡中比差异的总行数短。
        let end = starts[i + 1];
        let scroll = self.combined_scroll();
        let top = body.y + ((start - scroll) * f64::from(lh)) as f32;
        let bottom = body.y + ((end - scroll) * f64::from(lh)) as f32;
        let (from, to) = (top.max(body.y), bottom.min(body.bottom()));
        if to <= from {
            let pos = scroll - start;
            let area = Rect::new(body.x, body.y - DIFF_HEADER, body.w, body.h + DIFF_HEADER);
            return Some((area, pos, false));
        }
        let pos = f64::from((from - top) / lh);
        let area = Rect::new(body.x, from - DIFF_HEADER, body.w, to - from + DIFF_HEADER);
        Some((area, pos, true))
    }

    /// 把第 `i` 段的差异换进标签运行 `f`，再换回来：期间标签就是这一段的差异，编辑区是它露出
    /// 来的部分（看不见时是整个正文区），差异的部件取这一段上一帧画出的；`f` 照差异标签的规则
    /// 绘制、处理鼠标和按键。`f` 里滚动了多少，换回来以后整个标签跟着滚动多少。
    pub(crate) fn with_combined_section<T>(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut App) -> T,
    ) -> Option<T> {
        self.rotate_section(i, true, f)
    }

    /// 绘制时把第 `i` 段换进标签（同 [`Self::with_combined_section`]），`f` 里对滚动位置的改动
    /// 不算：绘制时标签的滚动位置是平滑滚动中显示的位置，不能写回去。
    pub(crate) fn draw_combined_section<T>(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut App) -> T,
    ) -> Option<T> {
        self.rotate_section(i, false, f)
    }

    /// 把第 `i` 段换进标签运行 `f`；`scrolls` 为真时 `f` 里滚动了多少，整个标签跟着滚动多少。
    fn rotate_section<T>(
        &mut self,
        i: usize,
        scrolls: bool,
        f: impl FnOnce(&mut App) -> T,
    ) -> Option<T> {
        let (area, pos, _) = self.combined_section_frame(i)?;
        let index = self.combined_index()?;
        let tab = self.git.combined.as_mut()?;
        if tab.inside.is_some() {
            return None;
        }
        let Body::Ready(view) = std::mem::replace(&mut tab.sections[i].body, Body::Waiting) else {
            return None;
        };
        tab.inside = Some(i);
        let section_hits = (tab.section_hits.iter())
            .find(|(at, _)| *at == i)
            .map(|(_, hits)| hits.clone())
            .unwrap_or_default();
        let hits = std::mem::replace(&mut self.hits.diff, section_hits);
        let doc = &mut self.editor.documents[index];
        doc.diff = Some(view);
        let mut scroll = pos;
        crate::app::scroll::swap_scroll_line(doc, &mut scroll);
        let editor = std::mem::replace(&mut self.layout.editor, area);
        // 段内的平滑滚动按段内的位置计，换回来以后按整个标签重新算。
        let glide = self.scroll.glide.take();
        let result = f(self);
        self.scroll.glide = glide;
        self.layout.editor = editor;
        self.hits.diff = hits;
        let doc = &mut self.editor.documents[index];
        crate::app::scroll::swap_scroll_line(doc, &mut scroll);
        let moved = scroll - pos;
        if let Some(tab) = self.git.combined.as_mut() {
            tab.inside = None;
            if let Some(view) = self.editor.documents[index].diff.take() {
                tab.sections[i].body = Body::Ready(view);
            }
        }
        if scrolls && moved != 0.0 {
            let before = self.combined_scroll();
            self.set_combined_scroll(before + moved);
            self.glide_scroll(before);
        }
        Some(result)
    }

    /// 鼠标下那一段上一帧画出的差异部件 `target` 的位置（悬停提示用）。
    pub(crate) fn combined_hovered_hit(&self, target: DiffTarget) -> Option<Rect> {
        let tab = self.combined_tab().filter(|_| self.combined_active())?;
        let i = tab.hovered?;
        let (_, hits) = tab.section_hits.iter().find(|(at, _)| *at == i)?;
        hits.iter()
            .find(|(_, t)| *t == target)
            .map(|(rect, _)| *rect)
    }

    /// 鼠标在 `(x, y)` 时下面那一段上一帧画出的差异部件（行勾选框、展开按钮等）：悬停按它
    /// 显示，同差异标签。不在部件上时为 `None`。
    pub(in crate::app) fn combined_section_target(&self, x: f32, y: f32) -> Option<DiffTarget> {
        let i = self.combined_pointer_section(x, y)?;
        let tab = self.combined_tab()?;
        let (_, hits) = tab.section_hits.iter().find(|(at, _)| *at == i)?;
        hits.iter()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, target)| *target)
    }

    /// 鼠标在 `(x, y)` 时交给哪一段（见 [`Self::with_combined_section`]）：按着鼠标时是按下的
    /// 那一段，否则是鼠标下那一段的差异正文；在段头、工具栏上或不在正文里时为 `None`。
    pub(in crate::app) fn combined_pointer_section(&self, x: f32, y: f32) -> Option<usize> {
        if !self.combined_active() {
            return None;
        }
        let tab = self.combined_tab()?;
        if tab.pressed.is_some() {
            return tab.pressed;
        }
        let on_own = self
            .hits
            .combined
            .iter()
            .any(|(rect, _)| rect.contains(x, y));
        let body = self.combined_parts().body;
        if on_own || !body.contains(x, y) {
            return None;
        }
        // 按位置找到鼠标所在的那一段，再看是不是落在它的正文上。
        let starts = self.combined_starts();
        let row = self.combined_scroll() + f64::from((y - body.y) / self.fonts.line_height);
        let i = starts.partition_point(|&s| s <= row).checked_sub(1)?;
        let (area, _, shown) = self.section_frame_in(&starts, i)?;
        (shown && y >= area.y + DIFF_HEADER && y < area.bottom()).then_some(i)
    }

    /// 是否按着鼠标在某一段里拖动：松开之前移动都交给那一段。
    pub(in crate::app) fn combined_dragging(&self) -> bool {
        self.combined_tab().is_some_and(|t| t.pressed.is_some())
    }

    /// 按下鼠标：落在某一段的差异正文里时交给那一段，它成为当前段；返回是否交了出去。
    pub(in crate::app) fn combined_mouse_down(&mut self, button: MouseButton) -> bool {
        let (x, y) = self.pointer.mouse;
        let Some(i) = self.combined_pointer_section(x, y) else {
            return false;
        };
        if let Some(tab) = self.git.combined.as_mut() {
            tab.current = Some(i);
            tab.pressed = Some(i);
            // 别的段里的选区和光标不要了：同一时间只有当前段有选区。
            for (at, section) in tab.sections.iter_mut().enumerate() {
                if let (true, Body::Ready(view)) = (at != i, &mut section.body) {
                    view.read_selection = None;
                    view.editing = false;
                }
            }
        }
        self.with_combined_section(i, |app| app.on_mouse_down(button));
        true
    }

    /// 移动鼠标：按着时交给按下的那一段，否则交给鼠标下的那一段；返回是否交了出去。
    pub(in crate::app) fn combined_mouse_move(&mut self, x: f32, y: f32) -> bool {
        // 已经换进了某一段（交给它的移动又回到这里）：按差异处理，鼠标下的段不变。
        if !self.combined_active() {
            return false;
        }
        let section = self.combined_pointer_section(x, y);
        if let Some(tab) = self.git.combined.as_mut() {
            tab.hovered = section;
        }
        let Some(i) = section else {
            return false;
        };
        self.with_combined_section(i, |app| app.on_mouse_move(x, y));
        true
    }

    /// 松开鼠标：交给按下的那一段；返回是否交了出去。
    pub(in crate::app) fn combined_mouse_up(&mut self, button: MouseButton) -> bool {
        let pressed = (self.git.combined.as_mut()).and_then(|t| t.pressed.take());
        let Some(i) = pressed.filter(|_| self.combined_active()) else {
            return false;
        };
        self.with_combined_section(i, |app| app.on_mouse_up(button));
        true
    }

    /// 在当前段里运行 `f`（按键、编辑的命令）：没有当前段时为 `None`。
    pub(in crate::app) fn in_combined_current<T>(
        &mut self,
        f: impl FnOnce(&mut App) -> T,
    ) -> Option<T> {
        if !self.combined_active() {
            return None;
        }
        let i = self.combined_tab()?.current?;
        self.with_combined_section(i, f)
    }

    /// 当前段里有没有光标或选区：有时按键交给它（移动光标、编辑），否则按键滚动整个标签。
    pub(in crate::app) fn combined_caret(&self) -> bool {
        let Some(tab) = self.combined_tab() else {
            return false;
        };
        let section = tab.current.and_then(|i| tab.sections.get(i));
        matches!(section, Some(Section { body: Body::Ready(view), .. })
            if view.editing || view.read_selection.is_some())
    }

    /// 点了标签上的 `hit`。
    pub(in crate::app) fn press_combined(&mut self, hit: CombinedHit) {
        match hit {
            // 按住 ⌥ 点段头：全部折叠或全部展开，跟着这一段要变成的样子。
            CombinedHit::Header(i) if self.pointer.modifiers.alt_key() => {
                self.finish_combined_fold(true);
                let sections = self.combined_tab().map_or(&[][..], |t| &t.sections[..]);
                let folded = sections.get(i).is_some_and(|s| s.folded);
                self.fold_all_combined(!folded);
            }
            CombinedHit::Header(i) => self.toggle_combined_section(i),
            CombinedHit::OpenFile(i) => self.open_combined_file(i),
            CombinedHit::OpenDiff(i) => self.open_combined_diff(i),
            CombinedHit::Previous => self.step_combined_change(false),
            CombinedHit::Next => self.step_combined_change(true),
            CombinedHit::Collapse => self.settings_toggle(crate::layout::Setting::DiffCollapse),
            CombinedHit::Layout => self.settings_toggle(crate::layout::Setting::DiffInline),
            CombinedHit::View => self.toggle_combined_diff(),
        }
        self.request_redraw();
    }
}

/// 排好一段的差异：按设置单栏（`unified`）或并排，并排时两侧按改动对齐（见
/// `DiffView::aligned`），按设置折叠（`collapse`）未更改的内容。其余与单独打开这个文件的差异
/// 相同：右侧可以编辑，可以按块、按行暂存。
fn arrange(view: &mut DiffView, unified: bool, collapse: bool) {
    view.aligned = true;
    view.set_unified(unified);
    view.set_collapse(collapse, 0);
}
