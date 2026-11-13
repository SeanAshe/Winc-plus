//! 并排差异标签的几何、命中区与交互。
//!
//! 负责标题栏按钮、折叠的未改动段、上一处与下一处改动之间的跳转，以及把左侧的一段内容
//! 替换到右侧。绘制由 `view` 子模块完成；可点击区域在绘制时记录到 `DiffState::hits`，
//! 悬停与点击均据此判定目标。
//!
//! 右侧对应工作区文件时可直接编辑：右侧即编辑器中该文件的文档，光标、选区与撤销记录都属于该
//! 文档。在右侧点击放置光标后，按键与编辑命令转给该文档执行；文件未打开时先在后台打开一个
//! 标签，执行后重新比较。左侧（`HEAD`）只读。

mod caret;
mod edit;
mod files;
mod find;
mod fold;
mod pointer;

use crate::app::scroll::set_scroll_line;
use std::path::Path;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;
use winc_core::{Lines, Selection, movement};

use winc_editor::background;
use winc_editor::diff_view::{Compared, DiffView, LEFT, RIGHT, ReadSelection, SideRow};
use winc_editor::{Document, DocumentId, FoldCommand};
use winc_i18n::t;
use winc_render::Rect;
use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use super::animation::{Clock, FOLD_ANIMATION};
use super::git::changes::replace_lines;
use super::lifecycle::notice::FLASH_DURATION;
use super::{App, Focus, ScrollArea, UserEvent, sticky_limit};
use crate::keymap::Action;
use crate::layout::{Scrollbar, hscrollbar_thumb};
use crate::theme::SCROLLBAR_HIT;

/// 差异标签顶部标题栏的高度（逻辑像素）；标题栏包含两侧的标题，右端为统计和按钮。
pub(crate) const DIFF_HEADER: f32 = 36.0;
/// 左侧行号左边放置「»」的列宽（逻辑像素）。
const DIFF_ARROWS: f32 = 22.0;
/// 右侧行号右边放置勾选框的列宽（逻辑像素），用于按块提交时勾选每处改动。
const DIFF_CHECKS: f32 = 22.0;
/// 并排时行号栏中行勾选框的宽度（逻辑像素）：容纳边长 11 的勾选框和两侧各 3 的留白。
const DIFF_LINE_CHECK: f32 = 18.0;
/// 两侧行号之间连接改动的连接带宽度（逻辑像素）。
const DIFF_RIBBON: f32 = 48.0;
/// 连接带上的按钮中心到连接带边缘的距离（逻辑像素）：两个宽 20 的悬停底色之间留出 4 的间隙。
const DIFF_RIBBON_BUTTON_INSET: f32 = 12.0;
/// 行号左右各留的空白（逻辑像素）。
pub(crate) const DIFF_GUTTER_PAD: f32 = 10.0;
/// 单栏时加减号列的宽度（字符宽度的倍数）：一个字符宽度加少量间距。
const DIFF_SIGN_CELLS: f32 = 1.5;
/// 文字和文字区边缘之间的空白（逻辑像素）。
pub(crate) const DIFF_TEXT_PAD: f32 = 10.0;

/// 「»」替换一段改动后，该段改动的颜色在原处淡出的时长（毫秒）。
///
/// 取自调参：该时长足以分辨被替换的行，又不会使下一步操作等待。
const REPLACE_FADE: std::time::Duration = std::time::Duration::from_millis(420);

/// 一段改动的种类，决定改动的颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HunkKind {
    /// 只在右侧有行。
    Added,
    /// 只在左侧有行。
    Deleted,
    /// 两侧都有行。
    Modified,
}

impl HunkKind {
    /// 返回 `hunk` 的种类。
    pub(crate) fn of(hunk: &winc_core::diff::LineHunk) -> HunkKind {
        match (hunk.old.is_empty(), hunk.new.is_empty()) {
            (true, _) => HunkKind::Added,
            (_, true) => HunkKind::Deleted,
            _ => HunkKind::Modified,
        }
    }
}

/// 正在淡出的替换颜色。
///
/// 记录两侧替换后对应的行号区间，以及改动原先的种类。
#[derive(Debug, Clone)]
pub(crate) struct DiffFade {
    /// 所属的差异标签。
    pub doc: DocumentId,
    /// 左侧的行。
    pub left: std::ops::Range<usize>,
    /// 右侧替换后的行，行数与 `left` 相同。
    pub right: std::ops::Range<usize>,
    /// 被替换的改动原先的种类。
    pub kind: HunkKind,
    /// 淡出的进度。
    pub clock: super::animation::Clock,
}

/// 同步重新比较的右侧行数上限（行）；超过时在后台比较。
///
/// 取自调参：debug 构建中实测 2000 行的比较约 4 ms；文档更大时若每次按键都同步比较，
/// 连续输入会明显滞后。
const REDIFF_IN_PLACE: usize = 3000;

/// 后台重新比较的一次结果。
pub(crate) struct Rediffed {
    /// 所属的差异标签。
    id: DocumentId,
    /// 比较时的左侧；采用结果前据此确认左侧未变。
    old: ropey::Rope,
    /// 比较时的右侧，即工作区文件当时的内容。
    new: ropey::Rope,
    /// 比较时右侧文档的版本号（`revision`），用于丢弃过时的结果。
    revision: u64,
    /// 比较时右侧文档是否有未保存的修改，决定右侧标题是否标出「未保存」。
    dirty: bool,
    /// 两侧的比较结果。
    compared: Compared,
}

/// 后台重新比较的状态。
pub(crate) struct RediffState {
    /// 正在后台比较的差异标签。
    running: std::collections::HashSet<DocumentId>,
    /// 后台比较结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    results: (Sender<Rediffed>, Receiver<Rediffed>),
}

impl Default for RediffState {
    fn default() -> Self {
        RediffState {
            running: Default::default(),
            results: channel(),
        }
    }
}

/// 差异导航到达首尾时等待再次按键的状态：第一次按键只提示，再次按键后才切换文件。
///
/// 再次按键时各字段均未变化且提示仍在显示才切换，否则重新提示。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffCross {
    /// 差异标签。
    doc: DocumentId,
    /// 方向：`true` 表示下一个文件。
    forward: bool,
    /// 提示时的当前段。
    hunk: Option<usize>,
    /// 提示时的滚动位置。
    scroll: f64,
}

/// 差异标签中的可点击目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTarget {
    /// 上一处改动。
    Previous,
    /// 下一处改动。
    Next,
    /// 上一个文件。
    PreviousFile,
    /// 下一个文件。
    NextFile,
    /// 切换单栏显示和并排显示。
    Layout,
    /// 折叠或展开所有未改动的长段。
    Collapse,
    /// 折叠的一段，参数为未改动段的序号；点击后展开。
    Fold(usize),
    /// 已展开的一段未改动内容的收起标记，参数为未改动段的序号；点击后重新折叠。
    Refold(usize),
    /// 固定在顶部的块头：（侧，行号）。点击后光标放到该行，该行滚到其外层的固定行之下。
    Sticky(usize, usize),
    /// 用左侧替换右侧的一段改动（参数为改动序号），即把该处恢复为 `HEAD` 中的内容。
    Revert(usize),
    /// 切换提交时是否包含一段改动（参数为改动序号）。
    Include(usize),
    /// 切换提交时是否包含一行：（侧，行号，从 0 起）。
    IncludeLine(usize, usize),
}

/// 差异中点击、拖选的单位：单击按字，双击按词，三击（或点击行号）按行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectUnit {
    Char,
    Word,
    Line,
}

impl SelectUnit {
    /// 返回连击 `count` 次对应的单位。
    fn of(count: u8) -> SelectUnit {
        match count {
            2 => SelectUnit::Word,
            3 => SelectUnit::Line,
            _ => SelectUnit::Char,
        }
    }

    /// 返回 `pos` 处按该单位选中的范围（起点，终点）。
    fn around(self, text: &impl Lines, pos: usize) -> (usize, usize) {
        match self {
            SelectUnit::Char => (pos, pos),
            SelectUnit::Word => movement::word_at(text, pos),
            SelectUnit::Line => {
                let span = text.line_span(text.pos_to_line_col(pos).0);
                (span.start, span.end)
            }
        }
    }

    /// 返回从按下时选中的 `origin` 拖到 `pos` 后的（锚点，光标）。
    ///
    /// 向前拖动时锚点位于 `origin` 的末尾。按下时选中的词、行始终完整保留，
    /// 鼠标在该范围内小幅移动时选区也不会缩小。
    fn extend(self, text: &impl Lines, origin: (usize, usize), pos: usize) -> (usize, usize) {
        let (from, to) = self.around(text, pos);
        if from < origin.0 {
            (origin.1, from)
        } else {
            (origin.0, to.max(origin.1))
        }
    }
}

/// 右侧显示行上的一点，对应右侧文档的标签下标、行号与显示列。
struct DiffSpot {
    /// 右侧文档在 `documents` 中的下标。
    source: usize,
    /// 右侧文档中的行号（从 0 起）。
    line: usize,
    /// 显示列（从 0 起），制表符按当前 Tab 宽度展开。
    cell: usize,
    /// 是否位于最后一行下方。
    past_end: bool,
}

/// 差异的一侧在屏幕上的位置。
///
/// 左侧为文字在左、行号在右，右侧为行号在左、文字在右，两侧的行号都紧邻中间的连接带。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffPane {
    /// 该侧的整体区域（文字与行号，左侧还包括「»」列）。
    pub rect: Rect,
    /// 行号栏。
    pub gutter: Rect,
    /// 行号右端的横坐标（逻辑像素），行号据此右对齐。
    pub number_end: f32,
    /// 文字区。
    pub text: Rect,
}

/// 差异标签的几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffParts {
    /// 顶部的标题栏：两侧的标题各自与文字区对齐，右端依次为当前改动序号、增删行数、
    /// 上一处、下一处和折叠开关。
    pub header: Rect,
    /// 标题栏下方显示行的区域。
    pub body: Rect,
    /// 左侧（修改前）和右侧（修改后）。
    pub panes: [DiffPane; 2],
    /// 两侧行号之间连接改动的连接带。
    pub ribbon: Rect,
    /// 单栏时右侧行号右边放置改动勾选框的列；并排时宽度为 0，勾选框位于连接带上。
    pub checks: Rect,
    /// 两侧放置行勾选框的列：并排时位于行号栏靠文字的一侧，单栏时均为加减号列。
    pub line_checks: [Rect; 2],
    /// 「»」按钮的中心横坐标（逻辑像素）。
    pub revert_x: f32,
    /// 改动勾选框的中心横坐标（逻辑像素）。
    pub include_x: f32,
}

/// 差异标签中一段未改动内容展开或折叠的过渡。
///
/// 过渡期间差异保持该段展开的排版：该段的行自上而下逐行露出，其后的行向上收起尚未露出的高度。
/// 折叠在过渡结束时才写入差异（见 [`App::finish_diff_fold`]）。
#[derive(Debug, Clone)]
pub(crate) struct DiffFoldMotion {
    /// 过渡所属的文档。
    pub doc: DocumentId,
    /// 未改动内容的段号。
    pub gap: usize,
    /// 展开状态下，该段折叠后隐藏的行在左侧、右侧占用的显示行；两侧行数相同。
    pub rows: [std::ops::Range<usize>; 2],
    /// 是否为展开；`false` 表示折叠。
    pub expanding: bool,
    /// 过渡开始时的滚动位置（显示行）；滚动位置改变后过渡不再绘制。
    pub scroll: f64,
    /// 过渡的时钟。
    pub clock: Clock,
}

impl DiffFoldMotion {
    /// 返回该段尚未露出的高度（逻辑像素），其后的行向上收起相应高度。
    ///
    /// 折叠状态下该段显示为一行，因此完全折叠时保留一行的高度。
    pub(crate) fn hidden(&self, line_height: f32) -> f32 {
        let progress = self.clock.progress();
        let reveal = if self.expanding {
            progress
        } else {
            1.0 - progress
        };
        (1.0 - reveal) * self.rows[RIGHT].len().saturating_sub(1) as f32 * line_height
    }
}

impl App {
    /// 处理差异标签中 `target` 的点击。
    pub(super) fn press_diff(&mut self, target: DiffTarget) {
        match target {
            DiffTarget::Previous => self.jump_difference(false),
            DiffTarget::Next => self.jump_difference(true),
            DiffTarget::PreviousFile => self.step_diff_file(false),
            DiffTarget::NextFile => self.step_diff_file(true),
            DiffTarget::Layout => self.settings_toggle(crate::layout::Setting::DiffInline),
            DiffTarget::Collapse => self.refold_diff(|view, row| {
                let on = !view.collapses();
                view.set_collapse(on, row)
            }),
            DiffTarget::Fold(gap) => {
                self.finish_diff_fold(true);
                if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) {
                    view.expand(gap);
                }
                self.clamp_scroll();
                self.start_diff_fold(gap, true);
            }
            DiffTarget::Refold(gap) => {
                self.finish_diff_fold(true);
                // 有过渡时折叠在过渡结束后写入；没有时立即折叠。
                if !self.start_diff_fold(gap, false) {
                    self.refold_diff(|view, row| view.fold(gap, row));
                }
            }
            DiffTarget::Sticky(side, line) => self.click_diff_sticky(side, line),
            DiffTarget::Revert(hunk) => self.revert_diff_hunk(hunk),
            DiffTarget::Include(hunk) => self.toggle_diff_hunk(hunk),
            DiffTarget::IncludeLine(side, line) => self.toggle_diff_line(side, line),
        }
        self.request_redraw();
    }

    /// 处理固定行上的点击：光标放到该行上点击处的列，并滚动使该行位于固定行之下（行为同
    /// 编辑区的固定行）。
    ///
    /// 右侧是工作区文件时光标放在右侧文档中，否则在该侧选中这一位置（只读）。
    fn click_diff_sticky(&mut self, side: usize, line: usize) {
        let max = self.max_scroll_line();
        let x = self.pointer.mouse.0;
        let Some(index) = self.editor.active else {
            return;
        };
        let depth = self.diff_sticky_depth(index, side, line);
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return;
        };
        let row = view.row_of(side, line);
        let cell = self.diff_cell_at(view, side, row, x);
        let source = view
            .new
            .path
            .clone()
            .filter(|_| side == RIGHT)
            .and_then(|path| self.diff_source(&path, true));
        if let Some(source) = source {
            let tab_width = self.editor.active_tab_width();
            let doc = &mut self.editor.documents[source];
            let col = doc.cell_to_col(line, cell, tab_width);
            let pos = doc.text().line_col_to_pos(line, col);
            doc.collapse_selections();
            doc.selection = Selection::point(pos);
            doc.goal_cell = None;
        }
        let read = source
            .is_none()
            .then(|| self.diff_read_pos(side, line, cell))
            .flatten();
        let doc = &mut self.editor.documents[index];
        let Some(view) = doc.diff.as_deref_mut() else {
            return;
        };
        view.editing = source.is_some();
        view.read_selection = read.map(|pos| ReadSelection {
            side,
            range: (pos, pos),
        });
        let parts = diff_parts(self.layout.editor, view, self.metrics.cell);
        let visible = f64::from(parts.body.h / self.fonts.line_height);
        let row = row as f64;
        let top = if view.is_unified() {
            view.pos_of(side, row) - depth
        } else {
            view.pos_of_top(side, (row - depth).max(0.0), visible)
        };
        let before = doc.scroll_line;
        set_scroll_line(doc, top.clamp(0.0, max.max(0.0)));
        self.focus = Focus::Editor;
        self.glide_scroll(before);
        self.reset_blink();
    }

    /// 返回第 `side` 侧第 `line` 行显示在固定行下方时，顶部固定区域的高度（显示行）。
    ///
    /// 光标或查找到的匹配移到该行时，滚动需要再让出这些行；折叠了未改动内容或关闭了粘滞
    /// 滚动时为 0。高度按目标行计算，不依赖当前画面。并排时固定区域横贯两侧，高度取较深的
    /// 一侧：另一侧取与该行显示在同一高度的行。
    fn diff_sticky_depth(&mut self, index: usize, side: usize, line: usize) -> f64 {
        if self.config.sticky_scroll == Some(false) {
            return 0.0;
        }
        let tab_width = self.editor.tab_width;
        let lh = self.fonts.line_height;
        let editor = self.layout.editor;
        let cell = self.metrics.cell;
        let Some(view) = self.editor.documents[index].diff.as_deref_mut() else {
            return 0.0;
        };
        if view.collapses() {
            return 0.0;
        }
        let body = diff_parts(editor, view, cell).body.h / lh;
        let max = sticky_limit(body as usize);
        let own = view.side_mut(side).scope_depth(line, tab_width).min(max);
        if view.is_unified() {
            return own as f64;
        }
        let other = if side == LEFT { RIGHT } else { LEFT };
        let visible = f64::from(body);
        let row = view.row_of(side, line) as f64 - own as f64;
        let top = view.side_scroll(other, view.pos_of_top(side, row.max(0.0), visible), visible);
        let beside = view.line_at(other, (top + own as f64).floor() as usize);
        let deeper = beside.map_or(0, |l| view.side_mut(other).scope_depth(l, tab_width));
        own.max(deeper.min(max)) as f64
    }

    /// 返回差异标签在提交视图中对应的条目；不在提交视图中（没有改动、模块关闭）时为 `None`。
    pub(crate) fn diff_entry(&self, view: &DiffView) -> Option<&winc_editor::git::Entry> {
        let path = view.repo_path.as_deref()?;
        self.git
            .status
            .as_ref()?
            .entries
            .iter()
            .find(|e| e.path == path && !e.is_dir)
    }

    /// 切换第 `hunk` 处改动的勾选状态；提交时只提交已勾选的改动。
    fn toggle_diff_hunk(&mut self, hunk: usize) {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return;
        };
        let all: Vec<(u64, std::ops::Range<usize>)> = view
            .keys
            .iter()
            .zip(&view.hunks)
            .map(|(&k, h)| (k, h.old.clone()))
            .collect();
        let Some(entry) = self.diff_entry(view).cloned() else {
            return;
        };
        self.git.commit.toggle_hunk(&entry, hunk, &all);
        self.request_redraw();
    }

    /// 切换 `side` 侧第 `line` 行的勾选状态；提交时只提交已勾选的行。
    fn toggle_diff_line(&mut self, side: usize, line: usize) {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return;
        };
        let range = |h: &winc_core::diff::LineHunk| {
            if side == LEFT {
                h.old.clone()
            } else {
                h.new.clone()
            }
        };
        let Some(hunk) = view.hunks.iter().position(|h| range(h).contains(&line)) else {
            return;
        };
        let target = &view.hunks[hunk];
        let (new_len, start) = (target.new.len(), range(target).start);
        let all: Vec<(u64, std::ops::Range<usize>)> = view
            .keys
            .iter()
            .zip(&view.hunks)
            .map(|(&k, h)| (k, h.old.clone()))
            .collect();
        let Some(entry) = self.diff_entry(view).cloned() else {
            return;
        };
        let offset = line - start;
        self.git
            .commit
            .toggle_line(&entry, (hunk, new_len), (side, offset), &all);
        self.request_redraw();
    }

    /// 返回右侧对应的文档下标；未打开且 `open` 为 `true` 时就地打开，但不切换当前标签。
    fn diff_source(&mut self, path: &Path, open: bool) -> Option<usize> {
        let found = self
            .editor
            .documents
            .iter()
            .position(|d| d.path.as_deref() == Some(path));
        if let Some(index) = found {
            // 右侧随文档编辑，尚未读取的文档需要先读取。
            self.editor.load(index);
            return Some(index);
        }
        if !open {
            return None;
        }
        let active = self.editor.active;
        // 右侧随文档编辑，需要立即有内容，因此就地读取。
        let index = self.editor.open(path);
        self.editor.active = active;
        self.editor.documents[index]
            .text_editable()
            .then_some(index)
    }

    /// 使差异标签 `index` 的右侧与编辑器中的文档同步。
    ///
    /// 文档发生变化（在差异中编辑、在文件的标签中编辑、撤销）时重新比较。
    pub(crate) fn sync_diff_source(&mut self, index: usize) {
        self.sync_diff(index, false);
    }

    /// 同步差异标签 `index` 的右侧，`now` 为 `true` 时无论文件大小都同步比较。
    ///
    /// 「»」替换完成后需要立即定位，因此要求结果立即可用。
    fn sync_diff(&mut self, index: usize, now: bool) {
        let Some(path) = self
            .editor
            .documents
            .get(index)
            .and_then(|d| d.diff.as_deref())
            .and_then(|v| v.new.path.clone())
        else {
            return;
        };
        let Some(source) = self.diff_source(&path, false) else {
            return;
        };
        let source = &self.editor.documents[source];
        let (revision, dirty) = (source.revision, source.is_dirty());
        let Some(rope) = source.text().rope().cloned() else {
            return;
        };
        let id = self.editor.documents[index].id;
        let Some(view) = self.editor.documents[index].diff.as_deref_mut() else {
            return;
        };
        if view.source == Some(revision) {
            return;
        }
        // 不超过 `REDIFF_IN_PLACE` 行的文档同步比较；更大的文档在后台比较，同一标签同一时刻
        // 只比较一次，完成后若文档再次变化则重新比较，连续输入时任务不排队。
        let Some(proxy) = self
            .link
            .proxy
            .clone()
            .filter(|_| !now && rope.len_lines() > REDIFF_IN_PLACE)
        else {
            view.set_new_text(rope);
            view.source = Some(revision);
            view.new.title = worktree_title(dirty).to_owned();
            return;
        };
        if !self.diff.rediff.running.insert(id) {
            return;
        }
        let old = view.old.text.clone();
        let sender = self.diff.rediff.results.0.clone();
        background::spawn(move || {
            let compared = Compared::new(&old, &rope);
            let _ = sender.send(Rediffed {
                id,
                old,
                new: rope,
                revision,
                dirty,
                compared,
            });
            let _ = proxy.send_event(UserEvent::Rediffed);
        });
    }

    /// 接收后台重新比较的结果并应用到差异标签。
    ///
    /// 文档在比较期间再次变化时，下一帧重新比较。
    pub(super) fn poll_rediffs(&mut self) {
        while let Ok(done) = self.diff.rediff.results.1.try_recv() {
            self.diff.rediff.running.remove(&done.id);
            let Some(view) = self
                .editor
                .document_mut(done.id)
                .and_then(|d| d.diff.as_deref_mut())
            else {
                continue;
            };
            // 比较期间左侧已变化（`HEAD` 变化、标签内容被替换），或已应用更新的结果时，丢弃该结果。
            if view.source.is_some_and(|s| s >= done.revision) || view.old.text != done.old {
                continue;
            }
            view.apply(done.new, done.compared);
            view.source = Some(done.revision);
            view.new.title = worktree_title(done.dirty).to_owned();
            self.clamp_scroll();
            self.request_redraw();
        }
    }

    /// 用左侧内容替换右侧第 `hunk` 段改动。
    ///
    /// 文件已打开时修改编辑器中的文档，⌘Z 可撤销，保存仍然可用；未打开时直接写回磁盘，
    /// 不为这一处修改额外打开标签。
    pub(super) fn revert_diff_hunk(&mut self, hunk: usize) {
        let Some(index) = self.editor.active else {
            return;
        };
        self.sync_diff(index, true);
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return;
        };
        let (Some(path), Some(h)) = (view.new.path.clone(), view.hunks.get(hunk).cloned()) else {
            return;
        };
        let inserted = view.old_lines(hunk);
        // 记录该段改动首行与视图顶部的距离，替换后使该行停留在原位置。
        let offset = view.hunk_rows[RIGHT][hunk].start as f64
            - view.row_at(RIGHT, self.editor.documents[index].scroll_line);
        let doc_id = self.editor.documents[index].id;
        let fade = DiffFade {
            doc: doc_id,
            left: h.old.clone(),
            right: h.new.start..h.new.start + h.old.len(),
            kind: HunkKind::of(&h),
            clock: super::animation::Clock::new(REPLACE_FADE),
        };
        match self.diff_source(&path, false) {
            Some(source) => {
                if let Err(reason) =
                    replace_lines(&mut self.editor.documents[source], h.new, inserted)
                {
                    self.flash(reason);
                    return;
                }
                self.sync_diff(index, true);
            }
            None => {
                let written = Document::open(DocumentId(u64::MAX), path.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|mut doc| {
                        replace_lines(&mut doc, h.new, inserted).map_err(str::to_owned)?;
                        doc.save().map_err(|e| e.to_string())?;
                        Ok(doc.text().rope().cloned())
                    });
                match written {
                    Ok(rope) => {
                        if let Some((view, rope)) =
                            self.editor.documents[index].diff.as_deref_mut().zip(rope)
                        {
                            view.set_new_text(rope);
                        }
                    }
                    Err(e) => {
                        self.notify(t!("git-diff-revert-failed", error = e));
                        return;
                    }
                }
            }
        }
        // 替换后这些行不再有改动，会被折入「n 行未改动」：只保持这些行及其上下相邻的行展开，
        // 其余仍然折叠；视图位置不变，替换后的行仍在原处。
        if let Some(doc) = self.editor.documents.get_mut(index)
            && let Some(view) = doc.diff.as_deref_mut()
        {
            view.pin_new(fade.right.clone());
            let row = view.row_of_new(fade.right.start) as f64 - offset;
            let row = view.pos_of(RIGHT, row.max(0.0));
            set_scroll_line(doc, row);
        }
        self.clamp_scroll();
        // 被替换的改动不立即失去颜色，而是在原处淡出；动态效果关闭时不显示淡出。
        self.diff.fade = self
            .clock(REPLACE_FADE)
            .map(|clock| DiffFade { clock, ..fade });
        self.flash(t!("git-change-reverted"));
        self.request_redraw();
    }

    /// 在差异标签中执行「回滚光标处的改动」。
    ///
    /// 光标在右侧时回滚光标所在的改动，否则回滚视口第四行所在的改动。
    pub(super) fn revert_diff_hunk_here(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        self.sync_diff_source(index);
        let hunk = match self.diff_caret_line(index) {
            Some(_) => self.diff_caret_hunk(),
            None => {
                let doc = &self.editor.documents[index];
                doc.diff
                    .as_deref()
                    .and_then(|v| v.hunk_at_pos(doc.scroll_line.max(0.0) + 3.0))
            }
        };
        match hunk {
            Some(hunk) => self.revert_diff_hunk(hunk),
            None => self.flash(t!("git-diff-none-at-caret")),
        }
    }

    /// 返回右侧光标所在改动的序号；光标不在右侧或不在改动上时为 `None`。
    pub(super) fn diff_caret_hunk(&self) -> Option<usize> {
        let index = self.editor.active?;
        let line = self.diff_caret_line(index)?;
        let view = self.editor.documents[index].diff.as_deref()?;
        view.hunks
            .iter()
            .position(|h| h.new.contains(&line) || (h.new.is_empty() && h.new.start == line))
    }

    /// 当前标签是差异、其右侧对应的文档被改动或光标移动之后调用：重新比较并滚动到光标处。
    pub(in crate::app) fn diff_source_changed(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        if self.editor.documents[index].diff.is_none() {
            return;
        }
        self.relayout();
        self.sync_diff_source(index);
        self.diff_caret_visible(index);
    }

    /// 光标在差异右侧时由右侧文档执行 `f`（按键、编辑命令），执行后重新比较并滚动到光标处。
    ///
    /// 光标不在右侧时返回 `None`，不做任何改动。
    fn in_diff_source<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        let index = self.editor.active?;
        let view = self.editor.documents[index].diff.as_deref()?;
        if !view.editing {
            return None;
        }
        let id = self.editor.documents[index].id;
        let path = view.new.path.clone()?;
        let source = self.diff_source(&path, true)?;
        // 编辑后光标不一定仍在跳转到的那一段，因此清除当前段标记。
        if let Some(view) = self.editor.documents[index].diff.as_deref_mut() {
            view.focus = None;
        }
        self.editor.active = Some(source);
        let result = f(self);
        // `f` 可能关闭了标签：按标识找回差异标签。
        if let Some(index) = self.editor.index_of(id) {
            self.editor.active = Some(index);
            self.relayout();
            self.sync_diff_source(index);
            self.diff_caret_visible(index);
        }
        self.reset_blink();
        self.request_redraw();
        Some(result)
    }

    /// 使当前差异标签按设置和文字区宽度自动换行，视图停留在原先的内容上。
    ///
    /// 当前标签不是差异时返回 `false`。整篇断行的耗时与行数成正比：宽度逐帧变化时沿用已有的宽度，
    /// 变化结束后再断行一次。
    pub(in crate::app) fn sync_diff_wrap(&mut self) -> bool {
        let wrap = self.config.word_wrap == Some(true);
        let tab_width = self.editor.active_tab_width();
        let resizing = self.text_width_changing();
        let (editor, cell) = (self.layout.editor, self.metrics.cell);
        let line_height = self.fonts.line_height;
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(view) = doc.diff.as_deref_mut() else {
            return false;
        };
        if resizing && wrap && view.is_wrapping() {
            return true;
        }
        let parts = diff_parts(editor, view, cell);
        let text_w = parts.panes[LEFT].text.w.min(parts.panes[RIGHT].text.w);
        let cols = ((text_w - 2.0 * DIFF_TEXT_PAD) / cell).floor().max(1.0) as usize;

        // 按屏幕上右侧最上方的显示行定位：两侧在视图中线对齐，该行不一定是由滚动位置换算出的行。
        let visible = f64::from(parts.body.h / line_height);
        let row = view.side_scroll(RIGHT, doc.scroll_line.max(0.0), visible);
        let (top, part) = match view.rows[RIGHT].get(row.floor() as usize) {
            Some(SideRow::Line { line, part, .. }) => (*line, *part),
            Some(SideRow::Fold { lines, .. }) => (lines.start, 0),
            None => (usize::MAX, 0),
        };
        if view.set_wrap(wrap.then_some((cols, tab_width))) {
            let part = part.min(view.breaks(RIGHT, top).rows().saturating_sub(1));
            let first = view.row_of_new(top) + part;
            let row = view.pos_of_top(RIGHT, first as f64 + row.fract(), visible);
            let wrapping = view.is_wrapping();
            set_scroll_line(doc, row);
            if wrapping {
                doc.scroll_x = 0.0;
            }
            self.clamp_scroll();
            self.request_redraw();
        }
        true
    }

    /// 使右侧光标可见：所在行被折叠时先展开，之后滚动到视口内。
    fn diff_caret_visible(&mut self, index: usize) {
        let Some(line) = self.diff_caret_line(index) else {
            return;
        };
        let tab_width = self.editor.active_tab_width();
        let path = self.editor.documents[index]
            .diff
            .as_deref()
            .and_then(|v| v.new.path.clone());
        let (caret_col, caret_cell) = path
            .and_then(|p| self.diff_source(&p, false))
            .map(|s| {
                let doc = &self.editor.documents[s];
                let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
                (col, doc.col_to_cell(line, col, tab_width))
            })
            .unwrap_or((0, 0));
        self.diff_scroll_to(index, RIGHT, (line, caret_col), caret_cell);
    }

    /// 滚动标签 `index` 中的差异，使第 `side` 侧（行，列）处的光标可见；所在行被折叠时先
    /// 展开。`caret_cell` 是光标所在的显示列。
    fn diff_scroll_to(
        &mut self,
        index: usize,
        side: usize,
        (line, caret_col): (usize, usize),
        caret_cell: usize,
    ) {
        let visible = self.visible_lines().saturating_sub(2).max(1);
        let cell = self.metrics.cell;
        let sticky = self.diff_sticky_depth(index, side, line);
        let (width, pane_height) = {
            let Some(view) = self.editor.documents[index].diff.as_deref() else {
                return;
            };
            let parts = diff_parts(self.layout.editor, view, cell);
            (
                parts.panes[side].text.w - DIFF_TEXT_PAD,
                f64::from(parts.body.h / self.fonts.line_height),
            )
        };
        let doc = &mut self.editor.documents[index];
        let Some(view) = doc.diff.as_deref_mut() else {
            return;
        };
        view.reveal_line(side, line);
        // 按滚动位置比较：另一侧较长的改动中本侧不前进，按本侧的行数比较会产生偏差。光标所在
        // 行的外层块固定在顶部时，光标需要位于这些固定行之下。
        let row = view.row_of_col(side, line, caret_col) as f64;
        let at = view.pos_of(side, row);
        let covered = sticky > 0.0
            && if view.is_unified() {
                at < doc.scroll_line + sticky
            } else {
                row < view.side_scroll(side, doc.scroll_line, pane_height) + sticky
            };
        let wrapping = view.is_wrapping();
        if covered {
            let row = if view.is_unified() {
                (at - sticky).max(0.0)
            } else {
                view.pos_of_top(side, (row - sticky).max(0.0), pane_height)
            };
            set_scroll_line(doc, row);
        } else if at < doc.scroll_line {
            set_scroll_line(doc, at);
        } else if at >= doc.scroll_line + visible as f64 {
            set_scroll_line(doc, at - visible as f64 + 1.0);
        }
        let x = if wrapping {
            doc.scroll_x
        } else {
            caret_cell as f32 * cell
        };
        if x < doc.scroll_x {
            doc.scroll_x = (x - cell * 4.0).max(0.0);
        } else if x > doc.scroll_x + width - cell * 2.0 {
            doc.scroll_x = x - width + cell * 6.0;
        }
        self.clamp_scroll();
    }

    /// 在差异标签中执行查找 `f`，查找范围是右侧的工作区文件。
    ///
    /// 查找转给右侧文档执行，光标放到右侧，跳到的匹配即右侧的选区，差异随之滚动。
    ///
    /// 右侧不可编辑时（文件已删除，或比较的不是 Git 差异）返回 `None` 且不做任何改动。
    pub(super) fn in_diff_search<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        let view = self.editor.active_mut()?.diff.as_deref_mut()?;
        view.new.path.as_ref()?;
        view.editing = true;
        view.read_selection = None;
        self.in_diff_source(f)
    }

    /// 当前标签是否为差异。
    pub(super) fn diff_active(&self) -> bool {
        self.editor.active().is_some_and(|d| d.diff.is_some())
    }

    /// 处理差异标签中的滚动按键。
    ///
    /// `↑` `↓` 滚动一行，翻页键滚动一屏，`⌘↑` `⌘↓`（其他系统为 `Ctrl+Home` `Ctrl+End`）
    /// 与 `Home` `End` 跳到开头和结尾，`←` `→` 横向滚动。其他按键不处理。
    pub(super) fn diff_key(&mut self, key: &Key, cmd: bool) {
        let page = self.visible_lines().saturating_sub(1).max(1) as f64;
        let cell = self.metrics.cell;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        match key {
            Key::Named(NamedKey::ArrowUp) if cmd => set_scroll_line(doc, 0.0),
            Key::Named(NamedKey::ArrowDown) if cmd => set_scroll_line(doc, f64::MAX),
            Key::Named(NamedKey::Home) => set_scroll_line(doc, 0.0),
            Key::Named(NamedKey::End) => set_scroll_line(doc, f64::MAX),
            Key::Named(NamedKey::ArrowUp) => set_scroll_line(doc, doc.scroll_line - 1.0),
            Key::Named(NamedKey::ArrowDown) => set_scroll_line(doc, doc.scroll_line + 1.0),
            Key::Named(NamedKey::PageUp) => set_scroll_line(doc, doc.scroll_line - page),
            Key::Named(NamedKey::PageDown) => set_scroll_line(doc, doc.scroll_line + page),
            Key::Named(NamedKey::ArrowLeft) => doc.scroll_x -= cell * 4.0,
            Key::Named(NamedKey::ArrowRight) => doc.scroll_x += cell * 4.0,
            _ => return,
        }
        set_scroll_line(doc, doc.scroll_line.floor());
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 跳转到第 `hunk` 段改动。
    ///
    /// 滚动到使该段距视口顶部三行的位置，距离较远时平滑滚动；光标移到该段开头，
    /// 该段显示当前标记和一次短暂的高亮。
    fn go_to_hunk(&mut self, hunk: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        let before = doc.scroll_line;
        set_scroll_line(doc, (view.hunk_pos(hunk) - 3.0).max(0.0));
        self.clamp_scroll();
        self.glide_scroll(before);
        self.place_diff_caret(hunk);
        self.diff.cross = None;
        self.diff.pulse = self.clock(crate::app::animation::DIFF_PULSE);
        if let Some(doc) = self.editor.active_mut() {
            let scroll = doc.scroll_line;
            if let Some(view) = doc.diff.as_deref_mut() {
                view.focus = Some((hunk, scroll));
            }
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 把光标放到第 `hunk` 段改动的开头：右侧文件已打开时放在该文档中，否则使用只读光标。
    ///
    /// 不为此打开文件：打开文件会新增标签，连续查看时标签数会不断增加；点击右侧时再打开。
    /// 右侧没有内容时（文件已删除）只读光标放在左侧。
    fn place_diff_caret(&mut self, hunk: usize) {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return;
        };
        let Some(lines) = view.hunks.get(hunk).map(|h| (h.old.start, h.new.start)) else {
            return;
        };
        let path = view.new.path.clone();
        let source = path.and_then(|path| self.diff_source(&path, false));
        if let Some(source) = source {
            let doc = &mut self.editor.documents[source];
            let line = lines.1.min(doc.text().line_count().saturating_sub(1));
            let pos = doc.text().line_col_to_pos(line, 0);
            doc.record_cursors();
            doc.collapse_selections();
            doc.selection = Selection::point(pos);
            doc.goal_cell = None;
        }
        let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) else {
            return;
        };
        view.editing = source.is_some();
        view.read_selection = None;
        if source.is_none() {
            let (side, line) = if view.new.text.len_chars() > 0 {
                (RIGHT, lines.1)
            } else {
                (LEFT, lines.0)
            };
            let text = &view.side(side).text;
            let pos = text.line_to_char(line.min(text.len_lines().saturating_sub(1)));
            let range = (pos, pos);
            view.read_selection = Some(ReadSelection { side, range });
        }
    }

    /// 使所有已打开和之后打开的差异按设置（`diffInline`）显示。
    ///
    /// 各标签顶部的行保持在顶部；已跳转到的改动仍为当前段。
    pub(in crate::app) fn apply_diff_layout(&mut self) {
        let on = self.config.diff_inline == Some(true);
        self.editor.diff_unified = on;
        let visible = f64::from(self.layout.editor.h / self.fonts.line_height);
        for doc in &mut self.editor.documents {
            let Some(view) = doc.diff.as_deref_mut() else {
                continue;
            };
            let row = view.side_scroll(RIGHT, doc.scroll_line.max(0.0), visible);
            view.set_unified(on);
            let scroll = view.pos_of_top(RIGHT, row, visible);
            if let Some((hunk, _)) = view.focus {
                view.focus = Some((hunk, scroll));
            }
            set_scroll_line(doc, scroll);
        }
        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
    }
}

/// 按键 `key` 是否修改文字；方向键、翻页键只移动光标，不为此打开文件。
pub(crate) fn edits_text(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(NamedKey::Enter | NamedKey::Backspace | NamedKey::Delete | NamedKey::Tab)
    )
}

/// 返回右侧（工作区）的标题。
pub(crate) fn worktree_title(dirty: bool) -> &'static str {
    if dirty {
        t!("git-diff-side-worktree-unsaved")
    } else {
        t!("git-diff-side-worktree")
    }
}

/// 差异两侧的水平滚动条，依次为左侧和右侧。
pub(crate) const DIFF_HSCROLL: [(ScrollArea, Scrollbar); 2] = [
    (
        ScrollArea::DiffLeftHorizontal,
        Scrollbar::DiffLeftHorizontal,
    ),
    (
        ScrollArea::DiffRightHorizontal,
        Scrollbar::DiffRightHorizontal,
    ),
];

/// 差异的横向滚动范围，以及两侧水平滚动条的轨道。
///
/// 两侧共用一个横向滚动位置，两条滚动条的滑块同步移动，拖动任意一条效果相同。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffHScroll {
    /// 最大横向滚动量（逻辑像素）；自动换行或内容不超出文字区时为 0。
    pub max: f32,
    /// 左侧和右侧的轨道：各侧的文字区，右侧让出竖直滚动条。单栏时两侧显示在同一个文字区，
    /// 只有右侧的一条，左侧为 `None`。
    pub tracks: [Option<Rect>; 2],
}

impl DiffHScroll {
    /// 计算差异 `view` 在布局 `parts` 中的横向滚动范围和轨道；`cell` 是字符宽度（逻辑像素）。
    pub(crate) fn of(parts: &DiffParts, view: &DiffView, cell: f32, tab_width: usize) -> Self {
        let [left, right] = [parts.panes[LEFT].text, parts.panes[RIGHT].text];
        let max = if view.is_wrapping() {
            0.0
        } else {
            // 按两侧最长的一行、右侧文字区的宽度限制，末尾留出 8 个字符的空白。
            let widest = view.widest(tab_width) as f32 * cell;
            (widest - right.w + DIFF_TEXT_PAD + cell * 8.0).max(0.0)
        };
        let right = Rect::new(
            right.x,
            right.y,
            (right.w - SCROLLBAR_HIT).max(0.0),
            right.h,
        );
        Self {
            max,
            tracks: [(!view.is_unified()).then_some(left), Some(right)],
        }
    }

    /// 返回第 `side` 侧的轨道和横向滚动到 `scroll` 时的滑块；该侧没有滚动条或内容不超出时
    /// 返回 `None`。
    pub(crate) fn thumb(&self, side: usize, scroll: f32) -> Option<(Rect, Rect)> {
        let track = self.tracks[side]?;
        let thumb = hscrollbar_thumb(track, scroll, track.w + self.max)?;
        Some((track, thumb))
    }
}

impl App {
    /// 返回当前差异标签的横向滚动范围和轨道；当前标签不是差异时返回 `None`。
    pub(crate) fn diff_hscroll(&self) -> Option<DiffHScroll> {
        let view = self.editor.active()?.diff.as_deref()?;
        let parts = diff_parts(self.layout.editor, view, self.metrics.cell);
        let tab_width = self.editor.active_tab_width();
        Some(DiffHScroll::of(&parts, view, self.metrics.cell, tab_width))
    }
}

/// 在编辑区 `editor` 中计算差异 `view` 的几何布局。
///
/// 中间从左到右依次是「»」、左侧行号栏、连接带、右侧行号栏、勾选框，两侧文字区平分剩余宽度；
/// 行号栏按两侧最大行号的位数定宽。
pub(crate) fn diff_parts(editor: Rect, view: &DiffView, cell: f32) -> DiffParts {
    let header = Rect::new(editor.x, editor.y, editor.w, DIFF_HEADER);
    let body = Rect::new(
        editor.x,
        header.bottom(),
        editor.w,
        (editor.h - DIFF_HEADER).max(0.0),
    );
    let lines = view.old.text.len_lines().max(view.new.text.len_lines());
    let digits = lines.to_string().len().max(2) as f32;
    let gutter_w = (digits * cell + 2.0 * DIFF_GUTTER_PAD).round();
    let column = |x: f32, w: f32| Rect::new(x, body.y, w, body.h);
    if view.is_unified() {
        // 单栏：「»」、修改前的行号、修改后的行号、勾选框、加减号（行勾选框与之同列）和文字。
        let arrows = column(editor.x, DIFF_ARROWS);
        let left_gutter = column(arrows.right(), gutter_w);
        let right_gutter = column(left_gutter.right(), gutter_w);
        let checks = column(right_gutter.right(), DIFF_CHECKS);
        let sign = (DIFF_SIGN_CELLS * cell).round();
        let lines = column(checks.right(), sign);
        let text = column(lines.right(), (editor.right() - lines.right()).max(0.0));
        let rect = column(editor.x, editor.w);
        return DiffParts {
            header,
            body,
            panes: [
                DiffPane {
                    rect,
                    gutter: left_gutter,
                    number_end: left_gutter.right() - DIFF_GUTTER_PAD,
                    text,
                },
                DiffPane {
                    rect,
                    gutter: right_gutter,
                    number_end: right_gutter.right() - DIFF_GUTTER_PAD,
                    text,
                },
            ],
            ribbon: column(lines.right(), 0.0),
            checks,
            line_checks: [lines, lines],
            revert_x: arrows.x + arrows.w / 2.0,
            include_x: checks.x + checks.w / 2.0,
        };
    }
    // 可按行勾选的差异：行号栏靠文字一侧留出行勾选框的位置，勾选框宽于行号留白的部分计入栏宽。
    let line_w = if view.repo_path.is_some() {
        DIFF_LINE_CHECK
    } else {
        0.0
    };
    let extra = (line_w - DIFF_GUTTER_PAD).max(0.0);
    let gutter_w = gutter_w + extra;
    let middle = 2.0 * gutter_w + DIFF_RIBBON;
    let text_w = ((editor.w - middle) / 2.0).max(0.0).floor();

    let left_text = column(editor.x, text_w);
    let left_gutter = column(left_text.right(), gutter_w);
    let ribbon = column(left_gutter.right(), DIFF_RIBBON);
    let right_gutter = column(ribbon.right(), gutter_w);
    let right_text = column(
        right_gutter.right(),
        (editor.right() - right_gutter.right()).max(0.0),
    );
    DiffParts {
        header,
        body,
        panes: [
            DiffPane {
                rect: column(editor.x, left_gutter.right() - editor.x),
                gutter: left_gutter,
                number_end: left_gutter.right() - DIFF_GUTTER_PAD,
                text: left_text,
            },
            DiffPane {
                rect: column(right_gutter.x, editor.right() - right_gutter.x),
                gutter: right_gutter,
                number_end: right_gutter.right() - DIFF_GUTTER_PAD - extra,
                text: right_text,
            },
        ],
        ribbon,
        checks: column(right_gutter.right(), 0.0),
        line_checks: [
            column(left_gutter.x, line_w),
            column(right_gutter.right() - line_w, line_w),
        ],
        // 改动级的两个按钮都位于中间的连接带上。
        revert_x: ribbon.x + DIFF_RIBBON_BUTTON_INSET,
        include_x: ribbon.right() - DIFF_RIBBON_BUTTON_INSET,
    }
}

/// 差异视图的界面状态：切换、定位时的过渡，以及后台的重新比较。
#[derive(Default)]
pub struct DiffState {
    /// 差异中用「»」替换一段改动后正在淡出的颜色。
    pub(crate) fade: Option<crate::app::diff::DiffFade>,
    /// 跳转到一处改动时该段改动的短暂高亮；没有时为 `None`。
    pub(crate) pulse: Option<crate::app::animation::Clock>,
    /// 差异切换文件后新文件的停留位置：`true` 为最后一处改动，`false` 为第一处；
    /// 不在切换文件时为 `None`。
    pub(crate) land: Option<bool>,
    /// 差异导航到达首尾时等待再次按键的状态与提示时刻；不在等待时为 `None`。
    pub(crate) cross: Option<(Instant, crate::app::diff::DiffCross)>,
    /// 大文件的差异在后台重新比较。
    pub(crate) rediff: RediffState,
}
