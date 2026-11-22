//! 多个选区同时编辑时，各选区的位置随前面的修改移动。

use super::*;

/// 返回多光标下编辑命令 `command` 的作用方式；`all` 是全部选区（按位置排序）。
pub(super) fn scope_of(command: &Command, all: &[Selection]) -> Scope {
    match command {
        Command::Indent if all.iter().all(Selection::is_empty) => Scope::Each,
        Command::Indent
        | Command::IndentLines
        | Command::Outdent
        | Command::ToggleComment
        | Command::DeleteLine
        | Command::MoveLines { .. }
        | Command::CopyLines { .. }
        | Command::JoinLines => Scope::Lines,
        Command::SortLines { .. } | Command::TrimTrailingWhitespace => Scope::Once,
        // 没有选区时转换整个文件，只执行一次。
        Command::UnescapeUnicode { whole: true } | Command::EscapeUnicode { whole: true } => {
            Scope::Once
        }
        _ => Scope::Each,
    }
}

/// 按行操作的一组选区执行完 `transaction` 后，组内选区 `selection` 的新位置。
///
/// 移动行、复制行整块平移，组内各选区移动同样的距离；删除行后都落到删除处；其余命令（缩进、
/// 切换行注释等）按修改逐处映射。
fn member_after(command: &Command, selection: Selection, transaction: &Transaction) -> Selection {
    match command {
        Command::MoveLines { .. } | Command::CopyLines { .. } => {
            let delta = transaction.after.head as isize - transaction.before.head as isize;
            let moved = |pos: usize| pos.saturating_add_signed(delta);
            Selection::new(moved(selection.anchor), moved(selection.head))
        }
        Command::DeleteLine => transaction.after,
        _ => shift_selection(selection, &transaction.changes),
    }
}

/// 一次多光标编辑中各选区的当前位置。
///
/// 每应用一处修改，修改之后的选区都要移动。逐个映射是「选区数 × 修改处数」次，上万个光标时每键入
/// 一个字就是上亿次；只有紧挨着修改的几个选区按 [`shift_selection`] 的边界规则逐个映射，更远的平移
/// 同样的距离。修改吞掉相邻选区时（如按词退格越过前一个光标）选区可能不再有序，此后改回逐个映射。
pub(super) struct Positions {
    pub(super) selections: Vec<Selection>,
    /// 差分数组：第 `j` 个选区还要平移 `pending[..=j]` 之和，比选区多一项，避免处处判断越界。
    ///
    /// 只有已处理过的选区（某一组之后的那些）上有值。
    pending: Vec<isize>,
    /// 是否从前往后处理（向上移动行）。
    ///
    /// 此时修改之后的选区还没处理，直接平移，不记差分；移动行不改变总长度，平移的距离总是 0。
    forward: bool,
    /// 选区已不再有序，改为逐个映射全部选区。
    unordered: bool,
}

impl Positions {
    /// 创建 `selections`（按位置排序）的位置记录；`forward` 为真时从前往后处理（向上移动行）。
    pub(super) fn new(selections: Vec<Selection>, forward: bool) -> Self {
        let pending = vec![0; selections.len() + 1];
        Self {
            selections,
            pending,
            forward,
            unordered: false,
        }
    }

    /// 一组选区（下标 `members`）上执行 `command` 得到的事务 `transaction` 已应用到文本：把组内
    /// 的选区放到执行后的位置，组外的选区随修改移动。
    pub(super) fn apply(
        &mut self,
        members: Range<usize>,
        command: &Command,
        transaction: &Transaction,
    ) {
        let changes = &transaction.changes;
        let (lo, hi, tail) = if self.unordered {
            for (i, selection) in self.selections.iter_mut().enumerate() {
                if !members.contains(&i) {
                    *selection = shift_selection(*selection, changes);
                }
            }
            (members.start, members.end, 0)
        } else {
            self.shift_around(members.clone(), changes)
        };
        if members.len() == 1 {
            self.selections[members.start] = transaction.after;
        } else {
            for selection in &mut self.selections[members] {
                *selection = member_after(command, *selection, transaction);
            }
        }
        if !self.unordered && !self.ordered(lo.saturating_sub(1)..hi, tail) {
            self.settle();
            self.unordered = true;
        }
    }

    /// 让组外的选区随修改 `changes` 移动。
    ///
    /// 返回逐个映射过（含组内）的选区的下标范围 `lo..hi`，和第 `hi` 个选区（如果有）还要平移
    /// 的距离；映射过的选区都已没有记着的差分。
    fn shift_around(&mut self, members: Range<usize>, changes: &[Change]) -> (usize, usize, isize) {
        let Some(earliest) = changes.iter().map(|c| c.start).min() else {
            return (members.start, members.end, self.pending[members.end]);
        };
        // 前面的选区：只有末端够到最早一处修改的才可能移动。末端是有序的，遇到第一个够不到的
        // 就可以停下。从前往后处理时这些选区已经处理过，从后往前时还没处理，两种情况下都没有
        // 记着的差分。
        let mut lo = members.start;
        while lo > 0 && self.selections[lo - 1].to() >= earliest {
            lo -= 1;
            self.selections[lo] = shift_selection(self.selections[lo], changes);
        }

        // 起点在 `reach` 之后的位置，经过每一处修改都只是平移：既不在修改之前，也不落在修改
        // 删掉的范围里或修改的起点上。依次累计每处修改的长度变化，换算回修改之前的位置。
        let (mut reach, mut delta) = (isize::MIN, 0isize);
        for change in changes {
            let removed = change.removed.chars().count() as isize;
            reach = reach.max(change.start as isize + removed - delta);
            delta += change.inserted.chars().count() as isize - removed;
        }

        // 后面的选区：起点也是有序的，紧挨着修改的逐个映射。记着的差分先加上，再从差分里扣掉，
        // 只影响这一个。
        let mut hi = members.end;
        let mut run = 0;
        while hi < self.selections.len() {
            run += self.pending[hi];
            let selection = offset(self.selections[hi], run);
            if selection.from() as isize > reach {
                break;
            }
            self.selections[hi] = shift_selection(selection, changes);
            self.pending[hi] -= run;
            self.pending[hi + 1] += run;
            run = 0;
            hi += 1;
        }
        if self.forward {
            for selection in &mut self.selections[hi..] {
                *selection = offset(*selection, delta);
            }
            (lo, hi, 0)
        } else {
            self.pending[hi] += delta;
            (lo, hi, run + delta)
        }
    }

    /// 第 `range` 个选区，连同其后第一个选区（还要平移 `tail`），是否仍按起点和末端排序。
    ///
    /// 只有逐个映射过的选区和与它们相邻的选区之间可能乱序，其余的选区要么没动，要么一起平移。
    fn ordered(&self, range: Range<usize>, tail: isize) -> bool {
        let next = self.selections.get(range.end).map(|&s| offset(s, tail));
        let window = self.selections[range].iter().copied().chain(next);
        window
            .clone()
            .zip(window.skip(1))
            .all(|(a, b)| a.from() <= b.from() && a.to() <= b.to())
    }

    /// 把记着的差分加到各选区上。
    fn settle(&mut self) {
        let mut run = 0;
        for (selection, pending) in self.selections.iter_mut().zip(&mut self.pending) {
            run += std::mem::take(pending);
            *selection = offset(*selection, run);
        }
    }

    /// 返回各选区的最终位置。
    pub(super) fn finish(mut self) -> Vec<Selection> {
        self.settle();
        self.selections
    }
}

/// 把选区的两端都平移 `delta` 个字符。
fn offset(selection: Selection, delta: isize) -> Selection {
    if delta == 0 {
        return selection;
    }
    Selection::new(
        selection.anchor.saturating_add_signed(delta),
        selection.head.saturating_add_signed(delta),
    )
}
