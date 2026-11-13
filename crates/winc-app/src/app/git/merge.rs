//! 在文件中解决合并冲突：每段冲突可分别采用当前、采用对方、保留两者，或打开并排对比。
//!
//! 含冲突标记的文件在编辑区打开时，每段冲突的当前一侧以蓝色、对方以绿色、共同祖先以灰色为底色，
//! `<<<<<<<` 标记行上有四个按钮。采用一侧是一次普通的编辑，可以用 `⌘Z` 撤销；右上角的卡片显示
//! 剩余的冲突数，提供上一处、下一处和全部采用一侧，全部解决后变为「标记为已解决」（`git add`）。
//!
//! 卡片上的「三栏合并」在文件两侧显示当前与对方的完整版本：中间仍是可编辑的文档，光标不在其中的
//! 冲突折叠为一行；两侧只读并随中间滚动，连接带上有采用该侧的按钮，底部为整个文件的操作。
//! 绘制由 `view/git/merge.rs` 完成，可点击的区域在绘制时记入 `merge.hits`。

use crate::app::scroll::set_scroll_line;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use winc_core::Lines;
use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::git::Change;
use winc_editor::git::merge::{self, Conflict, Pick, SideAlign};
use winc_editor::{Document, DocumentId};
use winc_i18n::t;
use winc_render::Rect;

use super::changes::replace_lines;
use crate::app::{App, UserEvent};

/// 三栏合并的标题行高度（逻辑像素），等于差异视图的标题栏高度。
pub(crate) const PANE_HEADER: f32 = crate::app::diff::DIFF_HEADER;
/// 三栏合并底部操作栏的高度（逻辑像素）：容纳一排按钮和上下留白。
pub(crate) const PANE_FOOTER: f32 = 44.0;
/// 三栏合并中栏与栏之间连接带的宽度（逻辑像素）：容纳 20 逻辑像素宽的按钮和两侧的曲线。
pub(crate) const PANE_RIBBON: f32 = 40.0;
/// 三栏合并中一侧的最小宽度（逻辑像素），约 30 列代码；窗口容纳不下三栏时不显示两侧。
const PANE_MIN: f32 = 240.0;

/// 冲突解决界面中可点击的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeTarget {
    /// 按某一侧解决指定的冲突（冲突下标，采用的一侧）。
    Pick(usize, Pick),
    /// 并排对比指定冲突的两侧（冲突下标）。
    Compare(usize),
    /// 上一段冲突。
    Previous,
    /// 下一段冲突。
    Next,
    /// 所有冲突都按某一侧解决。
    All(Pick),
    /// 冲突全部解决后运行 `git add`，标记为已解决。
    MarkResolved,
    /// 打开或关闭三栏合并。
    ThreePane,
    /// 展开指定的冲突（冲突下标），在结果中手动编辑。
    EditConflict(usize),
}

/// 三栏合并两侧的内容。
pub(crate) struct MergePanes {
    /// 对应的文档。
    pub doc: DocumentId,
    /// 当前（下标 0）与对方（下标 1）的完整版本。
    pub sides: [DiffSide; 2],
    /// 两侧与文档的行对应关系，以及计算时的文档版本号。
    pub aligned: Option<(u64, [SideAlign; 2])>,
    /// 文档中的冲突是否已由三栏合并折叠；退出或窗口容纳不下三栏时展开。
    folded: bool,
    /// 正在后台计算对齐的文档版本号。
    pub(in crate::app) aligning: Option<u64>,
}

impl MergePanes {
    /// 接收文档版本 `revision` 的对齐；比已有的结果旧时丢弃，因为后台任务不保证按提交顺序完成。
    pub(in crate::app) fn accept_alignment(&mut self, revision: u64, aligned: [SideAlign; 2]) {
        if self.aligning == Some(revision) {
            self.aligning = None;
        }
        if self.aligned.as_ref().is_none_or(|(at, _)| *at < revision) {
            self.aligned = Some((revision, aligned));
        }
    }
}

/// 后台计算对齐的结果。
struct SidesAligned {
    /// 对应的文档。
    doc: DocumentId,
    /// 计算时的文档版本号。
    revision: u64,
    /// 当前与对方的对齐。
    aligned: [SideAlign; 2],
}

/// 后台读取两侧的结果。
struct SidesLoaded {
    /// 对应的文档。
    doc: DocumentId,
    /// 文件的路径。
    path: PathBuf,
    /// 当前与对方的内容；读取失败时为原因。
    sides: Result<[Option<Vec<u8>>; 2], String>,
}

/// 冲突解决的状态。
#[derive(Default)]
pub(crate) struct MergeState {
    /// 识别出的冲突，按（文档，版本号）缓存，文档不变时无需每帧重新扫描。
    cache: Option<(DocumentId, u64, Arc<Vec<Conflict>>)>,
    /// 三栏合并两侧的内容；没有打开时为 `None`。
    pub panes: Option<MergePanes>,
    /// 三栏合并两侧的位置；不显示时宽度为 0。
    pub pane_rects: [Rect; 2],
    /// 三栏合并中两条连接带的位置。
    pub ribbons: [Rect; 2],
    /// 三栏合并的底部操作栏。
    pub footer: Rect,
    /// 后台读取两侧的结果。
    loaded: Option<(Sender<SidesLoaded>, Receiver<SidesLoaded>)>,
    /// 后台计算对齐的结果。
    aligned: Option<(Sender<SidesAligned>, Receiver<SidesAligned>)>,
}

impl App {
    /// 返回第 `index` 个标签中的冲突。
    ///
    /// Git 模块关闭、标签不显示文本或文档为大文件时没有冲突。
    pub(crate) fn conflicts(&mut self, index: usize) -> Arc<Vec<Conflict>> {
        let enabled = self.git_enabled();
        let Some(doc) = self.editor.documents.get(index) else {
            return Arc::default();
        };
        let key = (doc.id, doc.revision);
        if let Some((id, revision, found)) = &self.git.merge.cache
            && (*id, *revision) == key
        {
            return found.clone();
        }
        let found = match doc.text().rope() {
            Some(rope) if enabled && doc.shows_text() => Arc::new(merge::find_conflicts(rope)),
            _ => Arc::default(),
        };
        self.git.merge.cache = Some((key.0, key.1, found.clone()));
        found
    }

    /// 当前文件在仓库状态中是否仍标记为冲突（冲突标记已全部移除，但尚未 `git add`）。
    pub(crate) fn active_unmerged(&self) -> bool {
        let Some(path) = self.editor.active().and_then(|d| d.path.as_deref()) else {
            return false;
        };
        self.git.status.as_ref().is_some_and(|s| {
            s.entries.iter().any(|e| {
                e.shown() == Change::Conflicted && winc_editor::canonical_path(&e.path) == path
            })
        })
    }

    /// 处理对 `target` 的点击。
    pub(in crate::app) fn press_merge(&mut self, target: MergeTarget) {
        match target {
            MergeTarget::Pick(i, pick) => self.pick_conflict(i, pick),
            MergeTarget::Compare(i) => self.compare_conflict(i),
            MergeTarget::Previous => {
                self.jump_conflict(false);
            }
            MergeTarget::Next => {
                self.jump_conflict(true);
            }
            MergeTarget::All(pick) => self.pick_all_conflicts(pick),
            MergeTarget::MarkResolved => self.mark_resolved(),
            MergeTarget::ThreePane => self.toggle_merge_panes(),
            MergeTarget::EditConflict(i) => self.edit_conflict(i),
        }
        self.request_redraw();
    }

    /// 打开或关闭当前文件的三栏合并；打开时在后台读取暂存阶段中的两侧。
    fn toggle_merge_panes(&mut self) {
        if self.merge_panes_shown() {
            self.unfold_merge_conflicts();
            self.git.merge.panes = None;
            self.relayout();
            self.clamp_scroll();
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (id, path) = (doc.id, doc.path.clone());
        let (Some(path), Some(root)) = (path, self.git.root.clone()) else {
            return;
        };
        let sender = self.git.merge.loaded.get_or_insert_with(channel).0.clone();
        self.spawn_git(UserEvent::GitMergeSides, move |tool| {
            let cancel = winc_editor::format::Cancel::default();
            let sides = tool.map_err(|e| super::unusable(&e)).and_then(|git| {
                let read = |stage| git.content_at(&root, stage, &path, &cancel);
                Ok([read(":2")?, read(":3")?])
            });
            let _ = sender.send(SidesLoaded {
                doc: id,
                path,
                sides,
            });
        });
    }

    /// 接收后台读取的两侧与计算完成的对齐。
    pub(in crate::app) fn poll_merge_sides(&mut self) {
        let finished: Vec<SidesAligned> = (self.git.merge.aligned.as_ref())
            .map(|(_, results)| results.try_iter().collect())
            .unwrap_or_default();
        for result in finished {
            if let Some(panes) = self
                .git
                .merge
                .panes
                .as_mut()
                .filter(|p| p.doc == result.doc)
            {
                panes.accept_alignment(result.revision, result.aligned);
            }
        }
        let loaded: Vec<SidesLoaded> = (self.git.merge.loaded.as_ref())
            .map(|(_, results)| results.try_iter().collect())
            .unwrap_or_default();
        if !loaded.is_empty() {
            for sides in loaded {
                self.open_merge_panes(sides);
            }
            self.relayout();
            self.clamp_scroll();
        }
        self.request_redraw();
    }

    /// 用读取到的两侧打开三栏合并；非 UTF-8 文件无法逐行对应，只提示原因。
    fn open_merge_panes(&mut self, loaded: SidesLoaded) {
        let sides = match loaded.sides {
            Ok(sides) => sides,
            Err(reason) => return self.notify(reason),
        };
        let Some(doc) = self.editor.document(loaded.doc) else {
            return;
        };
        let language = doc.highlighter.language();
        let labels = self.merge_side_titles(loaded.doc);
        let [ours, theirs] = sides;
        let text = |bytes: Option<Vec<u8>>| String::from_utf8(bytes.unwrap_or_default());
        let (Ok(ours), Ok(theirs)) = (text(ours), text(theirs)) else {
            let name = winc_editor::display_name(&loaded.path);
            return self.notify(t!("git-merge-panes-not-utf8", name = name));
        };
        let [ours_title, theirs_title] = labels;
        self.git.merge.panes = Some(MergePanes {
            doc: loaded.doc,
            sides: [
                DiffSide::new(ours_title, &ours, None, language),
                DiffSide::new(theirs_title, &theirs, None, language),
            ],
            aligned: None,
            folded: false,
            aligning: None,
        });
    }

    /// 返回两侧的标题；冲突标记中带有名字时包含该名字。
    fn merge_side_titles(&mut self, doc: DocumentId) -> [String; 2] {
        let index = self.editor.documents.iter().position(|d| d.id == doc);
        let conflicts = index.map(|i| self.conflicts(i)).unwrap_or_default();
        let first = conflicts.first();
        let title = |who: &str, label: Option<&String>| match label {
            Some(label) if !label.is_empty() => format!("{who} · {label}"),
            _ => who.to_owned(),
        };
        [
            title(t!("git-merge-side-current"), first.map(|c| &c.ours_label)),
            title(
                t!("git-merge-side-incoming"),
                first.map(|c| &c.theirs_label),
            ),
        ]
    }

    /// 当前标签是否正在显示三栏合并。
    ///
    /// 文件不再是冲突状态（已标记为已解决）时两侧没有意义，不再显示。
    pub(crate) fn merge_panes_shown(&self) -> bool {
        let id = self.editor.active().map(|d| d.id);
        self.git
            .merge
            .panes
            .as_ref()
            .is_some_and(|p| Some(p.doc) == id)
            && self.active_unmerged()
    }

    /// 将编辑区分为三栏，栏之间留出连接带，底部留出操作栏；窗口容纳不下时不显示两侧。
    pub(in crate::app) fn place_merge_panes(&mut self) {
        self.git.merge.pane_rects = [Rect::default(); 2];
        self.git.merge.ribbons = [Rect::default(); 2];
        self.git.merge.footer = Rect::default();
        let needed = PANE_MIN * 3.0 + PANE_RIBBON * 2.0;
        if !self.merge_panes_shown() || self.layout.editor.w < needed {
            return;
        }
        let l = &mut self.layout;
        let whole = l.editor;
        let side = ((whole.w - PANE_RIBBON * 2.0) / 3.0).round();
        let height = (whole.h - PANE_FOOTER).max(0.0);
        let at = |x: f32, w: f32| Rect::new(x, whole.y, w, height);
        let left = at(whole.x, side);
        let right = at(whole.right() - side, side);
        self.git.merge.pane_rects = [left, right];
        self.git.merge.ribbons = [
            at(left.right(), PANE_RIBBON),
            at(right.x - PANE_RIBBON, PANE_RIBBON),
        ];
        self.git.merge.footer = Rect::new(whole.x, left.bottom(), whole.w, PANE_FOOTER);

        let shift = side + PANE_RIBBON;
        l.editor = at(left.right() + PANE_RIBBON, whole.w - shift * 2.0);
        l.gutter.x += shift;
        l.gutter.y += PANE_HEADER;
        l.gutter.h = (l.gutter.h - PANE_HEADER - PANE_FOOTER).max(0.0);
        l.text.x += shift;
        l.text.y += PANE_HEADER;
        l.text.w = (l.text.w - shift * 2.0).max(0.0);
        l.text.h = (l.text.h - PANE_HEADER - PANE_FOOTER).max(0.0);
        // 查找条紧贴结果一栏的右边。
        l.search.x -= shift;
    }

    /// 三栏合并是否正在显示两侧。
    pub(crate) fn merge_panes_placed(&self) -> bool {
        self.git.merge.pane_rects[0].w > 0.0
    }

    /// 在绘制前同步三栏合并：收起已解决的文件，将光标不在其中的冲突折叠为一行，按需重新计算对齐。
    ///
    /// 只折叠选区两端都不在其中的冲突：折叠行的文字被提示条遮挡，光标停留其上时键入的文字会
    /// 修改不可见的冲突标记，因此光标进入时立即展开。不再显示两侧时展开已折叠的冲突。
    pub(crate) fn sync_merge_panes(&mut self) {
        self.drop_finished_merge_panes();
        if !self.merge_panes_placed() {
            self.unfold_merge_conflicts();
            return;
        }
        let Some(index) = self.editor.active else {
            return;
        };
        let Some(panes) = self.git.merge.panes.as_mut() else {
            return;
        };
        panes.folded = true;
        let conflicts = self.conflicts(index);
        self.keep_view_while_folding(index, |doc| {
            let selection = doc.selection;
            let lines =
                [selection.anchor, selection.head].map(|pos| doc.text().pos_to_line_col(pos).0);
            for conflict in conflicts.iter() {
                if lines.iter().any(|line| conflict.lines().contains(line)) {
                    doc.unfold_at(conflict.start);
                } else {
                    doc.fold_lines(conflict.start, conflict.end);
                }
            }
        });
        self.align_merge_panes(index);
    }

    /// 对第 `index` 个文档执行 `fold`（折叠或展开冲突），保持屏幕上的内容位置不变。
    ///
    /// 折叠改变某一行之上的显示行数，滚动位置需要随之调整。
    /// 光标所在行可见时固定该行，否则固定最上方的一行。
    fn keep_view_while_folding(&mut self, index: usize, fold: impl FnOnce(&mut Document)) {
        let shown = (self.layout.text.h / self.fonts.line_height).ceil() as usize;
        let doc = &mut self.editor.documents[index];
        let first = doc.scroll_line.max(0.0) as usize;
        let caret = doc.text().pos_to_line_col(doc.selection.head).0;
        let anchor = if (first..first + shown).contains(&doc.row_of(caret)) {
            caret
        } else {
            doc.line_of_row(first)
        };
        let (before, rows) = (doc.row_of(anchor), doc.row_count());
        fold(doc);
        let after = doc.row_of(anchor);
        if after == before && doc.row_count() == rows {
            return;
        }
        set_scroll_line(
            doc,
            (doc.scroll_line + after as f64 - before as f64).max(0.0),
        );
        if self.editor.active == Some(index) {
            self.clamp_scroll();
        }
    }

    /// 文件不再是冲突状态（已解决、已中止）或文档已关闭时丢弃两侧的内容。
    ///
    /// 同一文件之后再次冲突时，两侧需要重新从暂存阶段读取，不能显示上一次合并的版本。
    fn drop_finished_merge_panes(&mut self) {
        let Some(doc) = self.git.merge.panes.as_ref().map(|panes| panes.doc) else {
            return;
        };
        let open = self.editor.documents.iter().any(|d| d.id == doc);
        let active = self.editor.active().map(|d| d.id) == Some(doc);
        if !open || (active && self.git.status.is_some() && !self.active_unmerged()) {
            self.unfold_merge_conflicts();
            self.git.merge.panes = None;
        }
    }

    /// 文档的版本变化后在后台重新计算对齐。
    ///
    /// 比较两侧与整个文件的耗时随文件增大而增加，因此不在界面线程上执行；结果送达之前继续使用
    /// 上一次的对齐，每个版本只计算一次。
    fn align_merge_panes(&mut self, index: usize) {
        let conflicts = self.conflicts(index);
        let doc = &self.editor.documents[index];
        let (id, revision) = (doc.id, doc.revision);
        let Some(rope) = doc.text().rope().cloned() else {
            return;
        };
        let Some(panes) = self.git.merge.panes.as_mut() else {
            return;
        };
        let current = panes
            .aligned
            .as_ref()
            .is_some_and(|(at, _)| *at == revision);
        if current || panes.aligning == Some(revision) {
            return;
        }
        panes.aligning = Some(revision);
        let sides = [panes.sides[0].text.clone(), panes.sides[1].text.clone()];
        let sender = self.git.merge.aligned.get_or_insert_with(channel).0.clone();
        self.spawn_then(UserEvent::GitMergeSides, move || {
            let [ours, theirs] = sides;
            let aligned = [
                SideAlign::new(&rope, &conflicts, &ours.to_string(), Pick::Ours),
                SideAlign::new(&rope, &conflicts, &theirs.to_string(), Pick::Theirs),
            ];
            let _ = sender.send(SidesAligned {
                doc: id,
                revision,
                aligned,
            });
        });
    }

    /// 展开三栏合并折叠的冲突。
    fn unfold_merge_conflicts(&mut self) {
        let Some(panes) = self.git.merge.panes.as_mut().filter(|p| p.folded) else {
            return;
        };
        panes.folded = false;
        let doc = panes.doc;
        let Some(index) = self.editor.documents.iter().position(|d| d.id == doc) else {
            return;
        };
        let conflicts = self.conflicts(index);
        self.keep_view_while_folding(index, |doc| {
            for conflict in conflicts.iter() {
                doc.unfold_at(conflict.start);
            }
        });
    }

    /// 展开第 `i` 段冲突并将光标移入其中，以便在结果中手动编辑。
    fn edit_conflict(&mut self, i: usize) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        let Some(conflict) = conflicts.get(i) else {
            return;
        };
        let doc = &mut self.editor.documents[index];
        doc.unfold_at(conflict.start);
        let pos = doc.text().line_start(conflict.start + 1);
        self.select(pos, pos);
        self.focus = crate::app::Focus::Editor;
    }

    /// 窗口坐标 (`x`, `y`)（逻辑像素）是否位于三栏合并的两侧或连接带上。
    pub(in crate::app) fn in_merge_pane(&self, x: f32, y: f32) -> bool {
        (self
            .git
            .merge
            .pane_rects
            .iter()
            .chain(&self.git.merge.ribbons))
        .any(|r| r.contains(x, y))
    }

    /// 按 `pick` 解决第 `i` 段冲突，将整段替换为所选的一侧。
    fn pick_conflict(&mut self, i: usize, pick: Pick) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        let Some(conflict) = conflicts.get(i) else {
            return;
        };
        let doc = &mut self.editor.documents[index];
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let text = merge::resolution(rope, conflict, pick);
        if let Err(reason) = replace_lines(doc, conflict.lines(), text) {
            self.flash(reason);
            return;
        }
        let left = self.conflicts(index).len();
        self.flash(match (left, pick) {
            (0, _) => t!("git-merge-all-resolved").to_owned(),
            (n, Pick::Ours) => t!("git-merge-picked-current", count = n),
            (n, Pick::Theirs) => t!("git-merge-picked-incoming", count = n),
            (n, Pick::Both) => t!("git-merge-picked-both", count = n),
        });
    }

    /// 按 `pick` 解决所有冲突。
    ///
    /// 从后向前替换，使前面的行号不受影响；每段是一次编辑，可以逐段撤销。
    fn pick_all_conflicts(&mut self, pick: Pick) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        if conflicts.is_empty() {
            return;
        }
        let doc = &mut self.editor.documents[index];
        let Some(rope) = doc.text().rope().cloned() else {
            return;
        };
        for conflict in conflicts.iter().rev() {
            let text = merge::resolution(&rope, conflict, pick);
            if let Err(reason) = replace_lines(doc, conflict.lines(), text) {
                self.flash(reason);
                return;
            }
        }
        let count = conflicts.len();
        self.flash(match pick {
            Pick::Ours => t!("git-merge-picked-all-current", count = count),
            Pick::Theirs => t!("git-merge-picked-all-incoming", count = count),
            Pick::Both => t!("git-merge-picked-all-both", count = count),
        });
    }

    /// 并排对比第 `i` 段冲突的两侧：左侧为当前，右侧为对方。
    fn compare_conflict(&mut self, i: usize) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        let Some(conflict) = conflicts.get(i) else {
            return;
        };
        let doc = &self.editor.documents[index];
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let lines = |range: std::ops::Range<usize>| -> String {
            range.flat_map(|l| rope.line(l).chunks()).collect()
        };
        let language = doc.highlighter.language();
        let name = doc.name().into_owned();
        let side = |label: &str, who: &str| {
            if label.is_empty() {
                who.to_owned()
            } else {
                format!("{who} · {label}")
            }
        };
        let view = DiffView::new(
            DiffSide::new(
                side(&conflict.ours_label, t!("git-merge-side-current")),
                &lines(conflict.ours()),
                None,
                language,
            ),
            DiffSide::new(
                side(&conflict.theirs_label, t!("git-merge-side-incoming")),
                &lines(conflict.theirs()),
                None,
                language,
            ),
        );
        let title = t!("git-merge-compare-title", name = name, index = i + 1);
        self.editor.show_diff(&title, view);
        self.relayout();
        self.clamp_scroll();
    }

    /// 跳转到下一段（`forward`）或上一段冲突，使其位于距顶部三行处。
    ///
    /// 当前文件没有冲突时返回 `false`。
    pub(in crate::app) fn jump_conflict(&mut self, forward: bool) -> bool {
        let Some(index) = self.editor.active else {
            return false;
        };
        let conflicts = self.conflicts(index);
        if conflicts.is_empty() {
            return false;
        }
        if self.merge_panes_placed() {
            self.scroll_to_conflict(index, &conflicts, forward);
            return true;
        }
        let doc = &self.editor.documents[index];
        let caret = doc.text().pos_to_line_col(doc.selection.head).0;
        let target = if forward {
            conflicts.iter().find(|c| c.start > caret)
        } else {
            conflicts.iter().rev().find(|c| c.end < caret)
        };
        match target {
            Some(conflict) => {
                let pos = doc.text().line_start(conflict.start);
                self.select(pos, pos);
                let doc = &mut self.editor.documents[index];
                let row = doc.row_of(conflict.start);
                set_scroll_line(doc, row.saturating_sub(3) as f64);
                self.clamp_scroll();
            }
            None if forward => self.flash(t!("git-merge-no-next")),
            None => self.flash(t!("git-merge-no-previous")),
        }
        true
    }

    /// 在三栏合并中滚动到下一段（`forward`）或上一段冲突，光标不变。
    ///
    /// 移动光标会使其落在折叠行上并展开冲突，因此只滚动。
    fn scroll_to_conflict(&mut self, index: usize, conflicts: &[Conflict], forward: bool) {
        // 冲突位于距顶部三行处，单栏跳转使用同一位置。
        const ABOVE: usize = 3;
        // 靠近文件两端的冲突无法滚动到该位置，只能停在滚动范围的端点，因此按实际可达的位置比较：
        // 相差不到半行视为没有移动，即该方向上已没有更多冲突。
        let max = self.max_scroll_line();
        let doc = &self.editor.documents[index];
        let current = doc.scroll_line;
        let stop = |c: &Conflict| (doc.row_of(c.start).saturating_sub(ABOVE) as f64).min(max);
        let target = if forward {
            conflicts.iter().map(stop).find(|s| *s > current + 0.5)
        } else {
            conflicts
                .iter()
                .rev()
                .map(stop)
                .find(|s| *s < current - 0.5)
        };
        match target {
            Some(target) => {
                set_scroll_line(&mut self.editor.documents[index], target);
                self.clamp_scroll();
            }
            None if forward => self.flash(t!("git-merge-no-next")),
            None => self.flash(t!("git-merge-no-previous")),
        }
    }

    /// 在冲突全部解决后保存文件，再运行 `git add` 将其标记为已解决。
    fn mark_resolved(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        if !self.conflicts(index).is_empty() {
            self.flash(t!("git-merge-unresolved-left"));
            return;
        }
        let doc = &self.editor.documents[index];
        let (id, dirty) = (doc.id, doc.is_dirty());
        let Some(path) = doc.path.clone() else {
            return;
        };
        if dirty && !self.write_document(id, None, true) {
            return;
        }
        let name = winc_editor::display_name(&path).into_owned();
        self.git_operate(
            t!("git-op-mark-resolved", name = name.as_str()),
            move |git, root| git.stage(root, &[path]),
        );
    }
}

/// 冲突中一行所属的部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    /// 标记行。
    Marker,
    /// 当前一侧。
    Ours,
    /// 共同祖先。
    Base,
    /// 对方。
    Theirs,
}

/// 返回 `line` 在 `conflict` 中所属的部分；不在该冲突中时返回 `None`。
pub(crate) fn part_of(conflict: &Conflict, line: usize) -> Option<Part> {
    if !conflict.lines().contains(&line) {
        None
    } else if conflict.is_marker(line) {
        Some(Part::Marker)
    } else if conflict.ours().contains(&line) {
        Some(Part::Ours)
    } else if conflict.theirs().contains(&line) {
        Some(Part::Theirs)
    } else {
        Some(Part::Base)
    }
}
