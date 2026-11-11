//! 撤销与重做栈：记录事务、合并连续的输入并限制占用的内存。

use super::*;

/// 撤销与重做栈。
///
/// 每个事务有唯一的版本号，用来判断当前内容是否已保存。撤销栈占用的内存上限为 64 MB
/// （`MAX_UNDO_BYTES`），超过时丢弃最早的事务；最近一次事务总会保留。
#[derive(Debug, Default)]
pub struct History {
    /// 撤销栈，栈顶是最近一次事务。
    pub(super) undo: Vec<Transaction>,
    /// 重做栈，栈顶是最近一次被撤销的事务。
    redo: Vec<Transaction>,
    /// 撤销栈中各事务占用内存之和（字节），即各事务 `bytes` 之和。
    pub(super) undo_bytes: usize,
    /// 撤销栈撤到底时的版本号：打开时的原始内容为 0；丢弃过最早的事务后，是被丢弃的最后一个事务
    /// 的版本号，撤到底时的内容正是应用该事务之后的内容。
    base_version: u64,
    /// 已分配的最大版本号；下一个事务取该值加 1。
    next_version: u64,
    /// 最近一次保存（或重新加载）时的版本号；与 [`History::version`] 相等表示内容未修改。
    ///
    /// 保存时的内容已无法撤回到时为 `u64::MAX`，不会与任何版本号相等。
    saved_version: u64,
    /// 封口标记：为真时，下一次键入不与栈顶事务合并。
    ///
    /// 保存、撤销、重做和清空之后置为真。
    sealed: bool,
    /// 尚未结束的分组的嵌套层数；大于 0 时处于分组期间（见 [`History::begin_group`]）。
    groups: u32,
    /// 分组期间栈顶事务是否属于当前分组；为真且未封口时，下一个事务并入栈顶事务。
    ///
    /// 最外层分组开始时为假，分组内的第一个事务另起一个撤销单元。
    grouped: bool,
}

/// 连续键入合并为一个撤销单元的最长间隔（毫秒）。
///
/// 两次键入的间隔不超过 1000 ms 时合并；间隔超过该时长后，下一次键入另起一个撤销单元。
/// 1000 ms 为经验值：它长于连续输入单个词或短句的停顿，短于阅读、思考后的停顿。
pub(super) const MERGE_WINDOW: Duration = Duration::from_millis(1000);

/// 撤销栈占用内存的上限（字节，估算方法见 [`changes_size`]）。
///
/// 整份文件的操作（格式化、全部替换、排序、转换换行符）把全文记进撤销栈，大文件上做几次就是
/// 几百 MB，且要到关闭标签页才释放。超过上限时从最早的事务开始丢弃，最近一次事务总会保留。
/// 该值为经验值：64 MB 足以撤回普通文件上很长一段时间的编辑。
const MAX_UNDO_BYTES: usize = 64 << 20;

impl History {
    /// 返回当前内容对应的版本号；撤到底时为 `base_version`（打开时的原始内容为 0）。
    pub fn version(&self) -> u64 {
        self.undo.last().map_or(self.base_version, |t| t.version)
    }

    /// 是否有过修改：存在可撤销或可重做的事务。撤销至原始内容后仍为真，清空（[`History::clear`]）
    /// 后为假。
    pub fn has_changes(&self) -> bool {
        !self.undo.is_empty() || !self.redo.is_empty()
    }

    /// 是否有未保存的修改，即当前版本号与保存时的版本号不一致。
    pub fn is_dirty(&self) -> bool {
        self.version() != self.saved_version
    }

    /// 把当前版本记为已保存，并封口，使下一次键入另起一个撤销单元。
    pub fn mark_saved(&mut self) {
        self.saved_version = self.version();
        self.sealed = true;
    }

    /// 封口并返回当前版本号，使下一次键入另起一个撤销单元。
    ///
    /// 后台保存发起时调用；写完时把这个版本记为已保存
    /// （[`History::mark_saved_version`]），撤销能正好回到写出的那一刻。
    pub fn seal(&mut self) -> u64 {
        self.sealed = true;
        self.version()
    }

    /// 把之前某一刻 [`History::seal`] 返回的版本 `version` 记为已保存。
    ///
    /// 后台保存写出的是发起保存那一刻的内容，写完时内容可能已经又改过了。
    pub fn mark_saved_version(&mut self, version: u64) {
        self.saved_version = version;
    }

    /// 开始一个分组：到对应的 [`History::end_group`] 为止，入栈的事务合并成一个撤销单元。
    ///
    /// 合并不受 [`EditKind`] 与键入间隔的限制，分组内的第一个事务不并入分组之前的事务。
    /// 分组期间封口（[`History::seal`]、保存、撤销、重做）之后入栈的事务另起一个撤销单元。
    /// 分组可以嵌套，最外层的分组结束时整组结束。
    pub fn begin_group(&mut self) {
        if self.groups == 0 {
            self.grouped = false;
        }
        self.groups = self.groups.saturating_add(1);
    }

    /// 结束最近一次 [`History::begin_group`] 开始的分组；没有未结束的分组时不做任何事。
    ///
    /// 最外层的分组结束时封口，之后的键入不并入该分组的撤销单元。
    pub fn end_group(&mut self) {
        if self.groups == 0 {
            return;
        }
        self.groups -= 1;
        if self.groups == 0 && self.grouped {
            self.grouped = false;
            self.sealed = true;
        }
    }

    /// 丢弃全部撤销和重做记录，并把当前内容视为已保存；重新加载文件时调用。
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.base_version = 0;
        self.saved_version = 0;
        self.sealed = true;
    }

    /// 记录一个已经应用到文本上的事务。
    ///
    /// 同时清空重做栈并分配新的版本号；符合合并条件的键入并入栈顶事务，分组期间的事务并入
    /// 该分组的撤销单元（[`History::begin_group`]），撤销栈超过上限时丢弃最早的事务。
    pub fn push(&mut self, mut transaction: Transaction) {
        self.redo.clear();
        self.next_version += 1;
        transaction.version = self.next_version;

        let grouping = self.groups > 0;
        if !self.sealed
            && let Some(last) = self.undo.last_mut()
            && (if grouping {
                self.grouped
            } else {
                can_merge(last, &transaction)
            })
        {
            // 紧邻的插入直接追加到上一处修改的 `inserted` 末尾：逐字符各记一处修改，会使每个
            // 字符多占一个 `Change` 和一次堆分配，撤销栈随键入量线性增大。
            // 多光标的修改各在一处，只能整组追加。分组期间不经 `can_merge`，在此判断是否紧邻。
            let single = transaction.extra_after.is_empty();
            let added = match (last.changes.last_mut(), transaction.changes.as_slice()) {
                (Some(prev), [next]) if single && (!grouping || extends(prev, next)) => {
                    let before = heap(&prev.inserted);
                    prev.inserted.push_str(&next.inserted);
                    heap(&prev.inserted) - before
                }
                _ => {
                    let added = changes_size(&transaction.changes);
                    last.changes.extend(transaction.changes);
                    added
                }
            };
            last.bytes += added;
            last.after = transaction.after;
            last.extra_after = transaction.extra_after;
            last.time = transaction.time;
            last.version = transaction.version;
            self.undo_bytes += added;
        } else {
            transaction.bytes = changes_size(&transaction.changes);
            self.undo_bytes += transaction.bytes;
            self.sealed = false;
            self.grouped = grouping;
            self.undo.push(transaction);
        }
        self.trim(MAX_UNDO_BYTES);
    }

    /// 撤销栈超过 `limit` 字节时，从最早的事务开始丢弃，直到不超过 `limit`；最近一次事务总会保留。
    pub(super) fn trim(&mut self, limit: usize) {
        let mut dropped = 0;
        while self.undo_bytes > limit && dropped + 1 < self.undo.len() {
            self.undo_bytes -= self.undo[dropped].bytes;
            dropped += 1;
        }
        if dropped == 0 {
            return;
        }

        self.base_version = self.undo[dropped - 1].version;
        self.undo.drain(..dropped);
        // 撤销栈中的版本号自底向上递增：保存时的版本比撤到底时的还早，说明那时的内容已撤回不到。
        if self.saved_version < self.base_version {
            self.saved_version = u64::MAX;
        }
    }

    /// 撤销栈顶的事务，返回撤销后的选区和修改涉及的最小位置；撤销栈为空时返回 `None`。
    pub fn undo(&mut self, text: &mut impl EditBuffer) -> Option<Restored> {
        let transaction = self.undo.pop()?;
        if let [change] = &transaction.changes[..] {
            change.revert(text);
        } else {
            let changes: Vec<_> = (transaction.changes.iter().rev())
                .map(|c| (c.start, c.inserted.chars().count(), c.removed.as_str()))
                .collect();
            text.replace_each(&changes);
        }
        let result = Restored {
            first: transaction.first_offset(),
            selection: transaction.before,
            extra: transaction.extra_before.clone(),
        };
        self.undo_bytes -= transaction.bytes;
        self.redo.push(transaction);
        self.sealed = true;
        Some(result)
    }

    /// 返回下一次重做的撤销单元执行之前的主选区；重做栈为空时返回 `None`。
    pub fn redo_origin(&self) -> Option<Selection> {
        self.redo.last().map(|transaction| transaction.before)
    }

    /// 重做最近撤销的事务；返回值含义同 [`Self::undo`]，重做栈为空时返回 `None`。
    pub fn redo(&mut self, text: &mut impl EditBuffer) -> Option<Restored> {
        let transaction = self.redo.pop()?;
        transaction.apply(text);
        let result = Restored {
            selection: transaction.after,
            extra: transaction.extra_after.clone(),
            first: transaction.first_offset(),
        };
        self.undo_bytes += transaction.bytes;
        self.undo.push(transaction);
        self.sealed = true;
        Some(result)
    }
}

/// 估算一组修改在撤销栈中占用的内存（字节）：每处修改本身的大小，加上删除和插入的文本占用的
/// 堆内存（见 [`heap`]）。
pub(super) fn changes_size(changes: &[Change]) -> usize {
    changes
        .iter()
        .map(|c| size_of::<Change>() + heap(&c.removed) + heap(&c.inserted))
        .sum()
}

/// 估算 `text` 占用的堆内存（字节）：按容量计，并向上取整到 16 字节。
///
/// macOS 的分配器以 16 字节为单位分配小块内存。整个文件转换行尾这类操作每行记一处修改，每处只有
/// 一两个字符，只按文本长度计算时会把撤销栈的占用低估一半以上，[`MAX_UNDO_BYTES`] 不起作用。
fn heap(text: &String) -> usize {
    text.capacity().next_multiple_of(16)
}

/// 撤销或重做之后的状态：选区和修改涉及的最小位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// 主选区。
    pub selection: Selection,
    /// 主选区以外的其余选区（多光标），按位置排序；单光标时为空。
    pub extra: Vec<Selection>,
    /// 修改涉及的最小位置（`char` 偏移），从这一行起重新高亮。
    pub first: usize,
}

/// 是否可以把 `next` 插入的文本追加到 `prev.inserted` 末尾：`next` 是纯插入，且紧接在 `prev`
/// 插入的文本之后。
fn extends(prev: &Change, next: &Change) -> bool {
    next.removed.is_empty() && next.start == prev.start + prev.inserted.chars().count()
}

/// 是否可以把 `next` 并入 `last`：两者都是键入，间隔不超过 [`MERGE_WINDOW`]，且插入位置首尾相接。
fn can_merge(last: &Transaction, next: &Transaction) -> bool {
    if last.kind != EditKind::Typing || next.kind != EditKind::Typing {
        return false;
    }

    if next.time.duration_since(last.time) > MERGE_WINDOW {
        return false;
    }

    // 多光标键入：各光标都只插入了不含空白的文字，且选区接在上一次键入之后继续输入。
    // 修改按顺序应用，直接接在上一次的修改之后即可，不必逐处检查是否相邻。
    if !last.extra_after.is_empty() || !next.extra_before.is_empty() {
        return last.after == next.before
            && last.extra_after == next.extra_before
            && next
                .changes
                .iter()
                .all(|c| c.removed.is_empty() && !c.inserted.contains(char::is_whitespace));
    }

    let (Some(prev), Some(change)) = (last.changes.last(), next.changes.first()) else {
        return false;
    };

    // 只合并紧邻的纯插入；插入内容含空白时不合并，使撤销以词为单位。
    change.removed.is_empty()
        && prev.removed.is_empty()
        && change.start == prev.start + prev.inserted.chars().count()
        && !change.inserted.contains(char::is_whitespace)
}
