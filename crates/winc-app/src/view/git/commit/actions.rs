//! 改动列表各行行尾的按钮：各类行有哪些按钮、排在哪里，以及绘制。
//!
//! 按钮从左到右排，最右边一个是最常用、不丢内容的操作（暂存、取消暂存、打开），回滚总在最左，
//! 与常用操作隔开。按钮排在行尾状态字母或文件数那一列的左边，悬停时那一列照常显示；各类行的
//! 按钮靠同一条右边缘对齐，同一种操作在相邻的行上处在同一列。

use super::*;
use crate::app::{ActionStrip, OtherGroup};

/// 一行行尾的按钮，从左到右，至多三个。
#[derive(Debug, Clone, Copy)]
pub(super) struct RowActions {
    targets: [CommitTarget; 3],
    len: usize,
}

impl RowActions {
    /// 由 `targets`（从左到右，至多三个）组成；为空时返回 `None`。
    fn of(targets: &[CommitTarget]) -> Option<Self> {
        let first = *targets.first()?;
        let mut all = [first; 3];
        let len = targets.len().min(3);
        all[..len].copy_from_slice(&targets[..len]);
        Some(RowActions { targets: all, len })
    }

    /// 各按钮的目标，从左到右。
    pub(super) fn targets(&self) -> &[CommitTarget] {
        &self.targets[..self.len]
    }

    /// 按钮的个数。
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// 是否有按钮的目标是 `target`。
    pub(super) fn holds(&self, target: CommitTarget) -> bool {
        self.targets().contains(&target)
    }
}

/// 返回 `band` 一行行尾那一列的位置：状态字母、文件数以该列为准，储藏和工作树行的「⋯」也在
/// 这里。中心离行底色右边 `END_CENTER`。
pub(super) fn end_slot(band: Rect) -> Rect {
    let y = band.y + (band.h - ACTION) / 2.0;
    Rect::new(band.right() - END_PAD - ACTION, y, ACTION, ACTION)
}

/// 返回 `count` 个按钮在 `band` 一行中的原位：最右边一个紧挨行尾那一列的左边；行尾的文件数宽
/// `tail`（逻辑像素，从行尾那一列的右边缘量起），比那一列宽时按钮再往左让。
pub(super) fn actions_home(band: Rect, count: usize, tail: f32) -> ActionStrip {
    let slot = end_slot(band);
    let right = slot.x.min(band.right() - END_PAD - tail) - ActionStrip::GAP;
    ActionStrip::ending_at(right, slot.y, ACTION, count)
}

/// 返回行尾写着文件数 `count` 时占的宽度（逻辑像素），见 [`draw_count_pill`]。
pub(super) fn count_tail(app: &App, frame: &mut Frame<'_>, count: &str) -> f32 {
    frame.measure(FontKind::Ui, app.fonts.ui_small, count) + 12.0
}

/// 当前仓库文件行的按钮：`group` 是所在的组，`index` 是文件在状态中的下标。
///
/// 冲突的文件只有「打开」：一键回滚会丢掉解决到一半的内容，需要时从右键菜单回滚。已暂存的文件
/// 不回滚，取消暂存后再回滚。
pub(super) fn file_row_actions(app: &App, group: Option<CommitGroup>, index: usize) -> RowActions {
    let (open, revert) = (
        CommitTarget::OpenFile(index),
        CommitTarget::RevertFile(index),
    );
    let targets: &[CommitTarget] = match (app.git.commit.staging(), group) {
        (_, Some(CommitGroup::Conflicts)) => &[open],
        (true, Some(CommitGroup::Staged)) => &[open, CommitTarget::StageFile(index, false)],
        (true, _) => &[revert, open, CommitTarget::StageFile(index, true)],
        (false, _) => &[revert, open],
    };
    RowActions::of(targets).unwrap_or(RowActions {
        targets: [open; 3],
        len: 1,
    })
}

/// 当前仓库目录行的按钮：`row` 是目录行的下标；冲突的一组没有按钮。
pub(super) fn dir_row_actions(app: &App, group: CommitGroup, row: usize) -> Option<RowActions> {
    let revert = CommitTarget::DirRevert(row);
    match (app.git.commit.staging(), group) {
        (_, CommitGroup::Conflicts) => None,
        (true, CommitGroup::Staged) => RowActions::of(&[CommitTarget::DirStage(row, false)]),
        (true, _) => RowActions::of(&[revert, CommitTarget::DirStage(row, true)]),
        (false, _) => RowActions::of(&[revert]),
    }
}

/// 当前仓库组标题的按钮；冲突的一组没有按钮。
pub(super) fn group_row_actions(group: CommitGroup, staging: bool) -> Option<RowActions> {
    let (revert, stage) = (
        CommitTarget::GroupRevert(group),
        CommitTarget::GroupStage(group),
    );
    match (staging, group) {
        (_, CommitGroup::Conflicts) => None,
        (true, CommitGroup::Staged) => RowActions::of(&[stage]),
        (true, _) => RowActions::of(&[revert, stage]),
        (false, _) => RowActions::of(&[revert]),
    }
}

/// 另一个仓库中文件行的按钮，规则同 [`file_row_actions`]：`side` 是暂存区视图中这一行所在的
/// 部分（`Some(true)` 为已暂存），勾选式下为 `None`；`conflicted` 为文件是否有冲突。
pub(super) fn other_file_actions(
    (repo, entry): (usize, usize),
    side: Option<bool>,
    conflicted: bool,
) -> RowActions {
    let open = CommitTarget::OtherOpen(repo, entry);
    let revert = CommitTarget::OtherRevert(repo, entry);
    let targets: &[CommitTarget] = match side {
        _ if conflicted => &[open],
        Some(true) => &[open, CommitTarget::OtherStage(repo, entry, false)],
        Some(false) => &[revert, open, CommitTarget::OtherStage(repo, entry, true)],
        None => &[revert, open],
    };
    RowActions::of(targets).unwrap_or(RowActions {
        targets: [open; 3],
        len: 1,
    })
}

/// 另一个仓库里一组的标题行的按钮，规则同 [`group_row_actions`]。
pub(super) fn other_group_actions(repo: usize, group: OtherGroup, staging: bool) -> RowActions {
    let revert = CommitTarget::OtherGroupRevert(repo, group);
    let targets: &[CommitTarget] = match (staging, group) {
        (true, OtherGroup::Staged) => &[CommitTarget::OtherGroupStage(repo, false)],
        (true, _) => &[revert, CommitTarget::OtherGroupStage(repo, true)],
        (false, _) => &[revert],
    };
    RowActions::of(targets).unwrap_or(RowActions {
        targets: [revert; 3],
        len: 1,
    })
}

/// 另一个仓库树形显示时目录行的按钮，规则同 [`dir_row_actions`]：`row` 是目录行在
/// `App::commit_other_rows` 中的下标，`staged` 为暂存区视图中目录是否在「暂存的更改」一组。
pub(super) fn other_dir_actions(row: usize, staged: Option<bool>) -> RowActions {
    let revert = CommitTarget::OtherDirRevert(row);
    let targets: &[CommitTarget] = match staged {
        Some(true) => &[CommitTarget::OtherDirStage(row, false)],
        Some(false) => &[revert, CommitTarget::OtherDirStage(row, true)],
        None => &[revert],
    };
    RowActions::of(targets).unwrap_or(RowActions {
        targets: [revert; 3],
        len: 1,
    })
}

/// 在 `strip` 处画按钮 `actions`，只画在 `clip` 之内；悬停的按钮铺底色，图标用正文色。
///
/// 有 `hits` 时记下各按钮的命中区域：`spill` 为真（跟在伸出侧栏的全名之后）时不裁到 `clip` 内，
/// 伸出去的按钮同样可以点击。
pub(super) fn draw_row_actions(
    app: &App,
    frame: &mut Frame<'_>,
    (strip, actions): (ActionStrip, RowActions),
    (clip, spill): (Rect, bool),
    hits: Option<&mut Vec<(Rect, CommitTarget)>>,
) {
    let p = app.palette;
    let mut found = Vec::with_capacity(actions.len());
    for (i, &target) in actions.targets().iter().enumerate() {
        let rect = strip.rect(i);
        let on = app.pointer.hover == Hover::Commit(target);
        draw_button(frame, p, rect, clip, on, false);
        let color = if on { p.text } else { p.text_muted };
        draw_action_icon(frame, target, center(rect), color, clip);
        found.push((if spill { rect } else { rect.intersect(clip) }, target));
    }
    if let Some(hits) = hits {
        hits.extend(found);
    }
}

/// 画按钮 `target` 的图标：打开文件、加号（暂存）、减号（取消暂存）或回滚。
fn draw_action_icon(
    frame: &mut Frame<'_>,
    target: CommitTarget,
    (cx, cy): (f32, f32),
    color: Color,
    clip: Rect,
) {
    match target {
        CommitTarget::OpenFile(_) | CommitTarget::OtherOpen(..) => {
            icons::open_file(frame, (cx, cy), color, clip);
        }
        CommitTarget::StageFile(_, true)
        | CommitTarget::DirStage(_, true)
        | CommitTarget::OtherStage(_, _, true)
        | CommitTarget::OtherGroupStage(_, true)
        | CommitTarget::OtherDirStage(_, true) => icons::plus(frame, (cx, cy), 8.0, color, clip),
        CommitTarget::StageFile(_, false)
        | CommitTarget::DirStage(_, false)
        | CommitTarget::OtherStage(_, _, false)
        | CommitTarget::OtherGroupStage(_, false)
        | CommitTarget::OtherDirStage(_, false)
        | CommitTarget::GroupStage(CommitGroup::Staged) => {
            icons::minus(frame, (cx, cy), 8.0, color, clip);
        }
        CommitTarget::GroupStage(_) => icons::plus(frame, (cx, cy), 8.0, color, clip),
        _ => icons::rollback(frame, (cx, cy), color, clip),
    }
}
