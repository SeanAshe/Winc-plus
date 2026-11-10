//! 从 Git 日志打开的差异在侧栏中的文件列表：列出该次提交或比较改动的文件，点击文件在同一个
//! 差异标签中切换。
//!
//! 列表借用提交视图的改动列表（[`CommitState::browse`]）：按目录排成树或平铺、合并只有一个
//! 子目录的各级、写全名、固定在顶部的目录和各种过渡都与提交视图相同，只是不能勾选和回滚。

use super::*;
use crate::app::git::commit::{CommitRow, CommitState, rows_of};
use crate::layout::SidebarView;
use winc_editor::git::{Entry, Status};

/// 差异标签对应的文件列表：所属的提交或比较，以及当前打开的文件。
pub(crate) struct LogDiffTree {
    /// 差异所属的提交或比较，以及其中改动的文件。
    pub(super) source: Arc<LogDiffSource>,
    /// 当前打开的文件在 `source.files` 中的下标。
    pub(super) selected: usize,
    /// 是否在提交视图中显示该列表；为 `false` 时提交视图显示工作区的改动。
    pub(super) shown: bool,
}

/// 把 `files` 转成提交视图的状态：每个文件一项，变化记为已暂存，下标与 `files` 相同。
fn browse_status(files: &[CommitFile]) -> Status {
    let entries = (files.iter())
        .map(|file| Entry {
            path: file.path.clone(),
            from: file.from.clone(),
            staged: Some(file.change),
            unstaged: None,
            is_dir: false,
        })
        .collect();
    Status {
        entries,
        ..Status::default()
    }
}

impl App {
    /// 返回提交视图中显示的历史差异文件列表：当前标签是从日志打开的差异、侧栏显示提交视图，
    /// 且该列表没有被切换为工作区改动时才有。
    pub(crate) fn log_diff_tree(&self) -> Option<&LogDiffTree> {
        let id = self.editor.active()?.id;
        (self.sidebar.view == SidebarView::Commit && self.steps_log_files(id))
            .then_some(self.log.tree.as_ref())
            .flatten()
            .filter(|tree| tree.shown)
    }

    /// 返回历史差异文件列表的浏览状态。
    pub(crate) fn log_list(&self) -> &CommitState {
        &self.log.list
    }

    /// 显示历史差异文件列表时，借用提交视图执行 `f`：期间提交视图的状态换成该列表的浏览
    /// 状态（[`CommitState::browse`]），仓库状态换成该提交改动的文件，结束后换回。不显示该列表，
    /// 或已经在借用中时返回 `None`，不执行 `f`。
    ///
    /// 提交视图的绘制、悬停、点击、滚动和键盘处理由此直接用于历史差异的文件列表。
    pub(crate) fn with_log_list<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        if self.commit.browse || self.log_diff_tree().is_none() {
            return None;
        }
        self.swap_log_list();
        let result = f(self);
        self.swap_log_list();
        Some(result)
    }

    /// 交换提交视图与历史差异文件列表的状态，见 [`App::with_log_list`]。
    ///
    /// 换入的仓库状态沿用真实状态的 `HEAD`、上游差距和进行中的操作：状态栏等处读取这些信息。
    fn swap_log_list(&mut self) {
        if let (Some(real), Some(browse)) = (&self.git.status, &mut self.log.status) {
            browse.head = real.head.clone();
            browse.oid = real.oid.clone();
            (browse.ahead, browse.behind) = (real.ahead, real.behind);
            browse.operation = real.operation;
        }
        std::mem::swap(&mut self.commit, &mut self.log.list);
        std::mem::swap(&mut self.git.status, &mut self.log.status);
    }

    /// 在提交视图中切换显示历史差异文件列表与工作区改动：提交视图已经可见时切换，
    /// 否则显示文件列表。当前标签不是从日志打开的差异时不做处理。
    pub(in crate::app) fn toggle_log_diff_tree(&mut self) {
        let visible = self.sidebar.visible && self.sidebar.view == SidebarView::Commit;
        let Some(id) = self.editor.active().map(|doc| doc.id) else {
            return;
        };
        if self.steps_log_files(id)
            && let Some(tree) = &mut self.log.tree
        {
            tree.shown = !visible || !tree.shown;
        }
    }

    /// 返回侧栏标题行中切换树形与列表的按钮区域：提交视图「⋯」按钮的位置。
    pub(crate) fn log_diff_layout_button(&self) -> Rect {
        self.commit_parts().tools[2].0
    }

    /// 在树形与列表之间切换文件列表的显示方式。
    ///
    /// 与提交视图的更改列表共用同一个设置（见 [`App::toggle_commit_layout`]），两处始终一致。
    pub(in crate::app) fn toggle_log_diff_layout(&mut self) {
        self.toggle_commit_layout();
    }

    /// 按设置合并或逐级显示文件列表中连续的单子目录。
    pub(in crate::app) fn set_log_list_compact(&mut self, compact: bool) {
        self.log.list.set_compact(compact);
    }

    /// 按提交视图当前的显示方式重排文件列表，并使当前文件保持可见。
    pub(in crate::app) fn sync_log_diff_layout(&mut self) {
        let flat = self.commit.flat();
        if self.log.list.flat() != flat {
            self.log.list.set_flat(flat);
            self.reveal_log_diff_file();
            self.request_redraw();
        }
    }

    /// 在差异标签中打开当前文件列表中的第 `file` 个文件（在后台读取）。
    pub(in crate::app::git) fn open_log_diff_file(&mut self, file: usize) {
        if let Some(tree) = &self.log.tree {
            self.load_log_diff(Arc::clone(&tree.source), file);
        }
    }

    /// 记录差异标签显示的是 `source` 的第 `file` 个文件，选中它并展开所在的目录。
    ///
    /// `source` 与当前文件列表相同时保留展开状态和滚动位置，否则换成新的文件列表。
    pub(super) fn select_log_diff(&mut self, source: Arc<LogDiffSource>, file: usize) {
        let keep = (self.log.tree.as_ref())
            .is_some_and(|tree| Arc::ptr_eq(&tree.source, &source) || tree.source == source);
        if keep {
            if let Some(tree) = &mut self.log.tree {
                tree.selected = file;
                tree.shown = true;
            }
        } else {
            self.log.list = CommitState::browsing(&self.commit);
            self.log.status = Some(browse_status(&source.files));
            self.log.tree = Some(LogDiffTree {
                source,
                selected: file,
                shown: true,
            });
        }
        let Some(tree) = &self.log.tree else {
            return;
        };
        let path = tree.source.files[file].path.clone();
        if let Some(dir) = (path.parent()).and_then(|dir| dir.strip_prefix(&tree.source.root).ok())
        {
            self.log.list.expand_to(dir);
        }
        self.log.list.select_only(path);
    }

    /// 返回按文件列表的显示顺序（目录全部展开时），与当前文件相邻的文件：（下标，在显示顺序中
    /// 的位置）；`forward` 为 `true` 时取下一个，没有时返回 `None`。
    pub(super) fn log_diff_neighbor(&self, forward: bool) -> Option<(usize, usize)> {
        let tree = self.log.tree.as_ref()?;
        let status = self.log.status.as_ref()?;
        let all = CommitState::browsing(&self.log.list);
        let order: Vec<usize> = (all.build_rows(status, &tree.source.root).iter())
            .filter_map(|row| match row {
                CommitRow::File { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        let at = order.iter().position(|&i| i == tree.selected)?;
        let next = if forward { at + 1 } else { at.checked_sub(1)? };
        order.get(next).map(|&i| (i, next))
    }

    /// 返回历史差异文件列表的最大滚动距离，规则同 [`App::commit_max_scroll`]。
    pub(in crate::app) fn log_list_max_scroll(&self) -> f32 {
        let root = self
            .log
            .tree
            .as_ref()
            .map(|tree| tree.source.root.as_path());
        let status = self.log.status.as_ref();
        let rows = rows_of(&self.log.list, status, root, self.git.generation).len() as f32;
        let list = self.commit_parts().list.h;
        if rows * crate::app::git::commit::COMMIT_ROW <= list {
            return 0.0;
        }
        ((rows + 1.0) * crate::app::git::commit::COMMIT_ROW - list).max(0.0)
    }

    /// 清除历史差异文件列表中已经结束的过渡；每帧绘制之前调用。
    pub(in crate::app) fn settle_log_tree(&mut self) {
        let list = &mut self.log.list;
        list.names.settle();
        let done = |clock: crate::app::Clock| clock.done();
        if list.shift.as_ref().is_some_and(|s| done(s.clock)) {
            list.shift = None;
        }
        if list.actions_slide.is_some_and(|s| done(s.1)) {
            list.actions_slide = None;
        }
        if list.motion.as_ref().is_some_and(|m| done(m.clock)) {
            list.motion = None;
        }
        if self.log.header.2.is_some_and(|clock| clock.done()) {
            self.log.header.2 = None;
        }
    }

    /// 历史差异文件列表是否有进行中的过渡（展开收起、名字展开收回、与提交视图之间的切换）。
    pub(in crate::app) fn log_tree_animating(&self) -> bool {
        let list = &self.log.list;
        list.names.moving()
            || list.shift.is_some()
            || list.actions_slide.is_some()
            || list.motion.is_some()
            || self.log.header.2.is_some()
    }

    /// 记录提交视图与文件列表之间的切换；每帧绘制之前调用。
    ///
    /// 侧栏一直显示提交视图、其内容在工作区改动与文件列表之间切换时（切换标签、点「提交」
    /// 切换），开始一段过渡：标题行右端的按钮与提交号交替淡出淡入，项目栏随之伸缩。侧栏切换
    /// 视图或刚打开时不过渡，由侧栏自己的过渡处理。
    pub(crate) fn track_log_tree(&mut self) {
        let shown = self.log_diff_tree().is_some();
        let commit_view = self.sidebar.visible && self.sidebar.view == SidebarView::Commit;
        let (was_shown, was_commit, _) = self.log.header;
        if shown != was_shown {
            let clock = (commit_view && was_commit)
                .then(|| self.clock(crate::app::animation::TREE_ANIMATION))
                .flatten();
            self.log.header = (shown, commit_view, clock);
        } else {
            self.log.header.1 = commit_view;
        }
    }

    /// 返回文件列表在提交视图中的露出比例：0 为工作区改动，1 为文件列表，过渡中介于两者之间。
    pub(crate) fn log_tree_share(&self) -> f32 {
        let (shown, _, clock) = self.log.header;
        let progress = clock.map_or(1.0, |clock| clock.progress());
        if shown { progress } else { 1.0 - progress }
    }

    /// 返回文件列表标题行中的版本（提交号）；没有文件列表时返回 `None`。
    pub(crate) fn log_tree_version(&self) -> Option<&str> {
        self.log
            .tree
            .as_ref()
            .map(|tree| tree.source.version.as_str())
    }

    /// 暂时按 `shown` 显示或隐藏文件列表执行 `f`，结束后恢复；没有文件列表时直接执行。
    ///
    /// 过渡中标题行需要分别按提交视图和文件列表绘制，两者各自的布局由此确定。
    pub(crate) fn with_log_tree_shown<R>(
        &mut self,
        shown: bool,
        f: impl FnOnce(&mut App) -> R,
    ) -> R {
        let Some(saved) = self
            .log
            .tree
            .as_mut()
            .map(|tree| std::mem::replace(&mut tree.shown, shown))
        else {
            return f(self);
        };
        let result = f(self);
        if let Some(tree) = &mut self.log.tree {
            tree.shown = saved;
        }
        result
    }

    /// 滚动文件列表，使当前打开的文件可见。
    pub(in crate::app) fn reveal_log_diff_file(&mut self) {
        self.with_log_list(App::reveal_commit_selection);
    }
}
