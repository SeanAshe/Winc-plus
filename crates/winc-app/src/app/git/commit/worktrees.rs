//! 提交视图底部的工作树：一组可以收起的行，当前仓库的每个工作树一行。
//!
//! 位于储藏一组的上面，当前仓库有多个工作树时显示；展开、收起的过渡见 [`Drawer`]。
//! 列出的行数有上限，超出时最后一行显示剩余个数，点击后展开工作树选择器。

use super::*;

/// 工作树一组展开时最多占的行数（行），含「另有 N 个工作树」一行：
/// 与储藏一组的上限（`STASH_ROWS`）相同。
pub(crate) const WORKTREE_ROWS: usize = 5;

/// 工作树一组展开时标题下面的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeLine {
    /// 第几个工作树（[`App::worktree_places`] 的下标）。
    Place(usize),
    /// 没有列出的工作树个数。
    More(usize),
}

impl App {
    /// 返回工作树一组展开时标题下面的各行：最多 [`WORKTREE_ROWS`] 行，其中总包含当前工作树。
    pub(crate) fn worktree_lines(&self) -> Vec<WorktreeLine> {
        let places = self.worktree_places();
        if places.len() <= WORKTREE_ROWS {
            return (0..places.len()).map(WorktreeLine::Place).collect();
        }
        let room = WORKTREE_ROWS - 1;
        let mut shown: Vec<usize> = (0..room).collect();
        if let Some(current) = places.iter().position(|p| p.current)
            && current >= room
        {
            shown[room - 1] = current;
        }
        let mut lines: Vec<WorktreeLine> = shown.into_iter().map(WorktreeLine::Place).collect();
        lines.push(WorktreeLine::More(places.len() - room));
        lines
    }

    /// 工作树一组的终点高度变化时从当前高度开始过渡。每帧绘制提交视图之前调用。
    pub(crate) fn track_worktree_height(&mut self) {
        let (settled, clock) = (self.worktree_settled_height(), self.section_clock());
        self.git.commit.worktrees.track(settled, clock);
    }

    /// 返回底部各组高度过渡所用的时钟；上一帧尚未读到仓库的状态时为 `None`，各组直接到位。
    pub(crate) fn section_clock(&mut self) -> Option<crate::app::animation::Clock> {
        if !self.git.commit.sections_known {
            return None;
        }
        self.clock(crate::app::animation::TREE_ANIMATION)
    }

    /// 记录这一帧是否已读到仓库的状态。各组的高度跟踪之后调用。
    pub(crate) fn sections_tracked(&mut self) {
        self.git.commit.sections_known = self.git.status.is_some();
    }

    /// 返回工作树一组的高度（逻辑像素），过渡中按时钟插值；当前仓库只有一个工作树时为 0。
    pub(crate) fn worktree_section_height(&self) -> f32 {
        (self.git.commit.worktrees).height(self.worktree_settled_height())
    }

    /// 返回工作树一组不在过渡中时的高度（逻辑像素）。
    fn worktree_settled_height(&self) -> f32 {
        if !self.git_has_worktrees() {
            return 0.0;
        }
        if self.git.commit.worktrees.open {
            DRAWER_CLOSED + self.worktree_lines().len() as f32 * COMMIT_ROW
        } else {
            DRAWER_CLOSED
        }
    }

    /// 返回工作树一组标题上箭头的角度比例，见 [`Drawer::chevron`]。
    pub(crate) fn worktree_chevron(&self) -> f32 {
        let span = self.worktree_lines().len() as f32 * COMMIT_ROW;
        (self.git.commit.worktrees).chevron(self.worktree_settled_height(), span)
    }

    /// 收起或展开工作树一组。
    fn toggle_worktrees(&mut self) {
        let from = self.worktree_section_height();
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.git.commit.worktrees.toggle(from, clock);
        self.git.commit.worktrees.settled = self.worktree_settled_height();
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
    }

    /// 处理工作树一组中的目标：展开、切换、菜单和新建。
    pub(super) fn press_commit_worktree(&mut self, target: CommitTarget) {
        // 「⋯」只在悬停时绘制：没有它的位置时菜单弹在该行的右端。
        let at = |wanted: CommitTarget| {
            let hit = (self.hits.commit.iter()).find(|(_, t)| *t == wanted);
            hit.map(|(rect, _)| *rect)
        };
        let rect = at(target).or_else(|| match target {
            CommitTarget::WorktreeMenu(i) => at(CommitTarget::WorktreeRow(i)),
            _ => None,
        });
        match target {
            CommitTarget::Worktrees => self.toggle_worktrees(),
            CommitTarget::WorktreeNew => self.open_worktree_dialog(),
            CommitTarget::WorktreeMore => self.open_places(false),
            CommitTarget::WorktreeRow(i) | CommitTarget::WorktreeMenu(i) => {
                let Some(place) = self.worktree_places().get(i).cloned() else {
                    return;
                };
                // 目录已不存在的工作树无法打开：点击该行弹出菜单，其中只有清理。
                let menu = matches!(target, CommitTarget::WorktreeMenu(_)) || place.missing;
                match (menu, place.worktree, rect) {
                    (true, _, Some(rect)) => {
                        let target = MenuTarget::Location(place.path, place.worktree);
                        let at = (rect.right() - rect.h, rect.bottom());
                        self.open_context_menu(target, at);
                    }
                    (false, Some(index), _) => self.open_worktree_asking(index),
                    _ => {}
                }
            }
            // 其余目标由 [`Self::press_commit`] 交给别的函数。
            _ => {}
        }
        self.request_redraw();
    }
}
