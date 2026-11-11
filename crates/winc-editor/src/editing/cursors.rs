//! 多光标与列选择：主选区之外的其余选区，以及在每个选区上同时执行编辑命令。
//!
//! 文档的 `selection` 始终是主选区（视图跟着它滚动），其余选区记在 [`Extra`] 里，只在主选区和
//! `revision` 仍是记录时的值时有效：直接给 `selection` 赋值或改动文本之后，其余选区自动作废，
//! 回到单光标。
//!
//! 多光标下的编辑逐个生成事务、立即应用，全部修改合成一个事务记入撤销栈。

mod positions;

use positions::*;

use std::ops::Range;

use memchr::memmem::Finder;
use winc_core::edit::{Change, EditKind, Transaction, selected_lines, shift_selection};
use winc_core::selection::merge;
use winc_core::{Lines, Selection, movement};
use winc_i18n::t;

use crate::Document;
use crate::document::content::Content;
use crate::editing::command::Command;
use crate::editing::folding::Tracked;
use crate::editing::search::MAX_MATCHES;

/// 主选区以外的其余选区（多光标）。
#[derive(Debug, Clone, Default)]
pub(crate) struct Extra {
    /// 其余选区，按位置排序，互不重叠，也不与主选区重叠。
    selections: Vec<Selection>,
    /// 各选区连续上下移动时保持的显示列，与 `selections` 一一对应。
    goals: Vec<Option<usize>>,
    /// `selections` 一共选中的字符数，设置选区时算好：状态栏每帧都要显示。
    chars: usize,
    /// 记录时的主选区；与文档当前的主选区不同时，其余选区作废。
    primary: Selection,
    /// 记录时文档的 `revision`；文本改动过（版本号变了）时，其余选区作废。
    revision: u64,
    /// 「添加下一处匹配」（`⌘D`）是否按全字匹配：从光标处选中一个词开始时为真，
    /// 此后只找完整的同一个词。
    whole_word: bool,
}

/// 光标撤销（`⌘U`）最多记下的选区状态个数（个）。
///
/// 经验值：足以覆盖一段连续的光标移动，同时限制内存占用。
const CURSOR_HISTORY: usize = 50;

/// 最多的选区数（个）：超过时只保留其中包含主选区的 [`MAX_CURSORS`] 个。
///
/// 每个选区都要单独执行编辑命令、记入撤销记录；全选后在两千万行的文件上放行尾光标会生成
/// 两千万个选区，占用数 GB 内存。经验值，与查找结果的上限 [`MAX_MATCHES`] 相同。
pub const MAX_CURSORS: usize = 10_000;

/// 光标撤销记下的一组选区：全部选区、主选区的下标和记录时文档的 `revision`。
#[derive(Debug, Clone, PartialEq, Eq)]
struct CursorState {
    selections: Vec<Selection>,
    primary: usize,
    revision: u64,
}

/// 键盘列选择（`⇧⌥⌘` 加方向键）正在框选的一块：起点和终点的（行号，显示列），均从 0 起。
///
/// 与其余选区相同，只在主选区和 `revision` 仍是记录时的值时有效，否则从主光标重新开始框选。
#[derive(Debug, Clone, Copy)]
struct ColumnBox {
    anchor: (usize, usize),
    head: (usize, usize),
    primary: Selection,
    revision: u64,
}

/// 选区的历史：光标撤销、重做（`⌘U` / `⇧⌘U`），以及正在框选的列。
///
/// 撤销栈只记文本没变时的选区变化：文本一改，此前记下的状态随 `revision` 不同而跳过。
#[derive(Debug, Clone, Default)]
pub(crate) struct CursorMemory {
    undo: Vec<CursorState>,
    redo: Vec<CursorState>,
    column: Option<ColumnBox>,
    /// 上次设置选区时是否因为超过 [`MAX_CURSORS`] 而舍弃了一部分，见
    /// [`Document::take_cursors_capped`]。
    capped: bool,
}

/// 多光标下编辑命令的作用方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// 在每个选区上各执行一次（键入、删除、粘贴……）。
    Each,
    /// 按行操作：行范围重叠或相邻的选区并成一组，每组执行一次。
    Lines,
    /// 作用于整个文件或只对主选区有意义：只在主选区上执行一次，其余选区随修改移动。
    Once,
}

/// 一次多光标编辑中执行一次命令的单位：一个选区，或按行操作时并成一组的几个选区。
struct Unit {
    /// 组内选区在全部选区中的下标：组内的选区总是位置相邻的一段。
    members: Range<usize>,
}

impl Document {
    /// 返回主选区以外的其余选区（按位置排序）；单光标时为空。
    pub fn extra_selections(&self) -> &[Selection] {
        if self.extra_valid() {
            &self.extra.selections
        } else {
            &[]
        }
    }

    /// [`Self::extra_selections`] 一共选中的字符数（不含主选区）。
    pub fn extra_selected_chars(&self) -> usize {
        if self.extra_valid() {
            self.extra.chars
        } else {
            0
        }
    }

    /// 是否有多个光标或选区。
    pub fn has_cursors(&self) -> bool {
        !self.extra_selections().is_empty()
    }

    /// 返回全部选区（按位置排序）和主选区在其中的下标。
    pub fn selections(&self) -> (Vec<Selection>, usize) {
        let extra = self.extra_selections();
        let primary = extra.partition_point(|s| s.from() < self.selection.from());
        let mut all = extra.to_vec();
        all.insert(primary, self.selection);
        (all, primary)
    }

    /// 依次返回主选区和其余选区（不按位置排序），不必像 [`Self::selections`] 那样复制整个列表。
    pub(crate) fn all_selections(&self) -> impl Iterator<Item = Selection> + '_ {
        std::iter::once(self.selection).chain(self.extra_selections().iter().copied())
    }

    /// 把选区设为 `selections`，`primary` 是主选区的下标。
    ///
    /// 选区先限制在文本范围内，再排序、合并重叠的部分；连续上下移动保持的显示列全部重置。
    /// `selections` 为空时不做任何事。超过 [`MAX_CURSORS`] 个时只保留其中包含主选区的
    /// [`MAX_CURSORS`] 个。
    pub fn set_selections(&mut self, selections: Vec<Selection>, primary: usize) {
        let goals = vec![None; selections.len()];
        self.set_selections_with_goals(selections, goals, primary);
    }

    /// 同 [`Self::set_selections`]，并给每个选区记下连续上下移动时保持的显示列。
    fn set_selections_with_goals(
        &mut self,
        selections: Vec<Selection>,
        goals: Vec<Option<usize>>,
        primary: usize,
    ) {
        if selections.is_empty() {
            return;
        }
        let (mut selections, mut goals, mut primary) = (selections, goals, primary);
        self.cursor_memory.capped = selections.len() > MAX_CURSORS;
        if self.cursor_memory.capped {
            let start = primary.saturating_sub(MAX_CURSORS - 1);
            selections = selections.split_off(start);
            selections.truncate(MAX_CURSORS);
            goals = goals.split_off(start);
            goals.truncate(MAX_CURSORS);
            primary -= start;
        }
        let end = self.text.end_pos();
        let selections: Vec<Selection> = selections.iter().map(|s| s.clamped(end)).collect();
        // 合并前记下每个选区的显示列（按位置排好，供二分查找），合并后按位置找回；被并掉的
        // 选区的显示列丢弃。全都没有显示列时（`set_selections`）不必记。
        let keyed: Vec<(Selection, Option<usize>)> = if goals.iter().any(Option::is_some) {
            let mut keyed: Vec<_> = selections.iter().copied().zip(goals).collect();
            keyed.sort_by_key(|(s, _)| (s.from(), s.to()));
            keyed
        } else {
            Vec::new()
        };
        let (merged, primary) = merge(selections, primary);
        // 位置相同的选区按原来的先后排在一起（稳定排序），取第一个方向也相同的条目。
        let goal_of = |s: &Selection| {
            let key = (s.from(), s.to());
            let start = keyed.partition_point(|(k, _)| (k.from(), k.to()) < key);
            keyed[start..]
                .iter()
                .take_while(|(k, _)| (k.from(), k.to()) == key)
                .find(|(k, _)| k == s)
                .and_then(|&(_, g)| g)
        };

        let whole_word = self.extra_alive() && self.extra.whole_word;
        self.selection = merged[primary];
        self.goal_cell = goal_of(&merged[primary]);
        let mut extra = Extra {
            primary: self.selection,
            revision: self.revision,
            whole_word,
            ..Extra::default()
        };
        for (i, s) in merged.iter().enumerate() {
            if i != primary {
                extra.selections.push(*s);
                extra.goals.push(goal_of(s));
                extra.chars += s.to() - s.from();
            }
        }
        self.extra = extra;
    }

    /// 上次设置选区时是否因为超过 [`MAX_CURSORS`] 而舍弃了一部分；取出后清除。
    pub fn take_cursors_capped(&mut self) -> bool {
        std::mem::take(&mut self.cursor_memory.capped)
    }

    /// 只保留主选区（`Esc`）；原本就只有一个选区时返回 `false`。
    pub fn collapse_selections(&mut self) -> bool {
        let had = self.has_cursors();
        if had {
            self.record_cursors();
        }
        self.extra = Extra::default();
        had
    }

    /// 只把主选区换成 `selection`，其余选区不变（`⌥` 拖动时选区随鼠标变化）；与其余选区
    /// 重叠时合并。
    pub fn set_primary(&mut self, selection: Selection) {
        let (mut all, primary) = self.selections();
        all[primary] = selection;
        self.set_selections(all, primary);
    }

    /// 记下当前的选区，供光标撤销（`⌘U`）退回；与上一次记下的相同时不重复记。
    ///
    /// 新记一次就清空光标重做。最多记 `CURSOR_HISTORY` 个，超出时丢弃最早的一个。
    pub fn record_cursors(&mut self) {
        let (selections, primary) = self.selections();
        let state = CursorState {
            selections,
            primary,
            revision: self.revision,
        };
        let memory = &mut self.cursor_memory;
        if memory.undo.last() == Some(&state) {
            return;
        }
        if memory.undo.len() >= CURSOR_HISTORY {
            memory.undo.remove(0);
        }
        memory.undo.push(state);
        memory.redo.clear();
    }

    /// 光标撤销（`⌘U`）：退回上一次记下的选区；文本改过之后，改之前的选区不再退回。
    ///
    /// 没有可退回的时返回 `false`。
    pub fn undo_cursors(&mut self) -> bool {
        self.step_cursors(true)
    }

    /// 光标重做（`⇧⌘U`）：重新做一次被光标撤销退回的选区变化；没有时返回 `false`。
    pub fn redo_cursors(&mut self) -> bool {
        self.step_cursors(false)
    }

    /// 从撤销栈（`undo` 为假时从重做栈）取出一组仍然有效、且与现在不同的选区换上，
    /// 现在的选区放进另一个栈。
    fn step_cursors(&mut self, undo: bool) -> bool {
        let (selections, primary) = self.selections();
        let current = CursorState {
            selections,
            primary,
            revision: self.revision,
        };
        let memory = &mut self.cursor_memory;
        let (from, to) = if undo {
            (&mut memory.undo, &mut memory.redo)
        } else {
            (&mut memory.redo, &mut memory.undo)
        };
        while let Some(state) = from.pop() {
            if state.revision != current.revision || state == current {
                continue;
            }
            to.push(current);
            self.set_selections(state.selections, state.primary);
            return true;
        }
        false
    }

    /// `⇧⌥I`：在每个选区覆盖的每一行行尾放一个光标。
    ///
    /// 选区的最后一行放在选区末端（不在行尾）；跨行选区结束在某行行首时，那一行不放。一行之内
    /// 的选区收成末端的光标。折叠隐藏的行跳过。返回是否改变了选区。
    pub fn add_cursors_to_line_ends(&mut self) -> bool {
        let (all, _) = self.selections();
        let mut cursors = Vec::new();
        for selection in &all {
            let (first, _) = self.text.pos_to_line_col(selection.from());
            let (last, last_col) = self.text.pos_to_line_col(selection.to());
            for line in first..last {
                // 超过上限的部分最终要舍弃（见 [`Self::set_selections`]）：多生成一个以便记下
                // 舍弃过，之后不再生成。
                if cursors.len() > MAX_CURSORS {
                    break;
                }
                if !self.folds.is_hidden(line) {
                    cursors.push(Selection::point(self.text.line_end(line)));
                }
            }
            if first == last || last_col > 0 {
                cursors.push(Selection::point(selection.to()));
            }
        }
        if cursors == all {
            return false;
        }
        self.record_cursors();
        let last = cursors.len() - 1;
        self.set_selections(cursors, last);
        true
    }

    /// `⇧⌥⌘` 加方向键：用键盘框选一列。
    ///
    /// 第一次从主选区开始框选（起点是选区的起点，终点是光标端），之后沿用上一次的框，终点上下
    /// 移 `dy` 行、左右移 `dx` 个显示列（行尾之后同样按显示列移动）。
    pub fn extend_columns(&mut self, dx: isize, dy: isize, tab_width: usize) {
        let columns = self.columns(tab_width);
        let columns = columns.as_deref();
        let at = |doc: &Self, pos: usize| {
            let (line, col) = doc.text.pos_to_line_col(pos);
            (line, doc.col_to_cell_with(columns, line, col, tab_width))
        };
        let current = self
            .cursor_memory
            .column
            .filter(|b| b.primary == self.selection && b.revision == self.revision);
        let (anchor, mut head) = match current {
            Some(b) => (b.anchor, b.head),
            None => (
                at(self, self.selection.anchor),
                at(self, self.selection.head),
            ),
        };

        let last = self.text.line_count().saturating_sub(1);
        head.0 = head.0.saturating_add_signed(dy).min(last);
        if dx != 0 {
            let line = head.0;
            let len = self.text.line_len(line);
            let target = head.1.saturating_add_signed(dx);
            let end = self.col_to_cell_with(columns, line, len, tab_width);
            head.1 = if target.max(head.1) > end {
                // 在行尾之后：按显示列移动。
                target
            } else {
                // 在行内：按字符移动；落在同一个制表符或宽字符里时再移动一个字符。
                let mut col = self
                    .cell_to_col_with(columns, line, target, tab_width)
                    .min(len);
                if self.col_to_cell_with(columns, line, col, tab_width) == head.1 {
                    col = col.saturating_add_signed(dx).min(len);
                }
                self.col_to_cell_with(columns, line, col, tab_width)
            };
        }

        self.record_cursors();
        self.select_columns(anchor, head, tab_width);
    }

    /// `⌥` 单击：在位置 `pos`（`char` 偏移）处添加一个光标，成为主选区；`pos` 落在已有的某个
    /// 选区上（含两端）且不止一个选区时，改为移除该选区。
    pub fn toggle_cursor(&mut self, pos: usize) {
        let pos = pos.min(self.text.end_pos());
        self.record_cursors();
        let (mut all, primary) = self.selections();
        if let Some(i) = all.iter().position(|s| s.from() <= pos && pos <= s.to()) {
            if all.len() > 1 {
                all.remove(i);
                // 移除的是主选区时，由它前面一个（没有时后面一个）接任。
                let primary = if i < primary || (i == primary && i > 0) {
                    primary - 1
                } else {
                    primary
                };
                let primary = primary.min(all.len() - 1);
                self.set_selections(all, primary);
            }
            return;
        }
        all.push(Selection::point(pos));
        let last = all.len() - 1;
        self.set_selections(all, last);
    }

    /// `⌥⌘↑` / `⌥⌘↓`：在最上面（`down` 为真时最下面）一个光标的上一行（下一行）同一显示列
    /// 添加光标，成为主选区。
    ///
    /// 已在第一行（最后一行）时不做任何事。
    ///
    /// 新光标沿用来源光标保持的显示列，连续添加经过较短的行后仍回到原来的列。
    pub fn add_cursor_vertically(&mut self, down: bool, tab_width: usize) -> bool {
        let (all, primary) = self.selections();
        let goals = self.goals(primary);
        let edge = if down { all.len() - 1 } else { 0 };
        let source = all[edge];

        let (saved, saved_goal) = (self.selection, self.goal_cell);
        self.selection = Selection::point(source.head);
        self.goal_cell = goals[edge];
        self.move_vertically(if down { 1 } else { -1 }, false, tab_width);
        let (added, goal) = (self.selection, self.goal_cell);
        self.selection = saved;
        self.goal_cell = saved_goal;

        if self.text.line_of(added.head) == self.text.line_of(source.head) {
            return false;
        }
        self.record_cursors();
        let mut all = all;
        let mut goals = goals;
        all.push(added);
        goals.push(goal);
        let last = all.len() - 1;
        self.set_selections_with_goals(all, goals, last);
        true
    }

    /// `⌘D`：把下一处与主选区相同的文本加入选区，成为主选区；主选区是光标时先选中光标处的词。
    ///
    /// 从主选区之后向后找，到文件末尾后从头继续，跳过已选中的位置。返回是否改变了选区；
    /// 光标不在词上、或已没有其他匹配时返回 `Ok(false)`。
    ///
    /// # Errors
    ///
    /// 大文件（文本不在内存中）不支持，返回说明原因的文字。
    pub fn add_next_occurrence(&mut self) -> Result<bool, &'static str> {
        self.next_occurrence(true)
    }

    /// `⌘K ⌘D`：跳过主选区，改为选中下一处相同的文本；其余同 [`Self::add_next_occurrence`]。
    ///
    /// # Errors
    ///
    /// 同 [`Self::add_next_occurrence`]。
    pub fn skip_to_next_occurrence(&mut self) -> Result<bool, &'static str> {
        self.next_occurrence(false)
    }

    /// 找到下一处与主选区相同的文本，作为新的主选区；`keep` 为假时去掉原来的主选区。
    fn next_occurrence(&mut self, keep: bool) -> Result<bool, &'static str> {
        self.record_cursors();
        if self.selection.is_empty() {
            return Ok(self.select_word_at_cursors());
        }
        let needle = self
            .text
            .substring(self.selection.from(), self.selection.to());
        let whole_word = self.extra_alive() && self.extra.whole_word;
        let (mut all, primary) = self.selections();
        // 各选区互不重叠，按起点排好后终点也是递增的：二分找到第一个终点在 `a` 之后的选区，
        // 该选区的起点不早于 `b` 就说明没有选区与 `a..b` 相交，不必把所有选区比一遍。
        let mut spans: Vec<(usize, usize)> = all.iter().map(|s| (s.from(), s.to())).collect();
        spans.sort_unstable();
        let free = |&(a, b): &(usize, usize)| {
            let i = spans.partition_point(|&(_, to)| to <= a);
            spans.get(i).is_none_or(|&(from, _)| from >= b)
        };
        // 先往后找，到末尾再从头找到主选区为止；找到第一处没被选中的就停。
        let after = self.selection.to();
        let mut next = None;
        self.each_match(&needle, whole_word, after, |m| {
            next = Some(m).filter(free);
            next.is_none()
        })?;
        if next.is_none() {
            self.each_match(&needle, whole_word, 0, |m| {
                next = Some(m).filter(|m| m.0 < after && free(m));
                next.is_none() && m.0 < after
            })?;
        }
        let Some((a, b)) = next else {
            return Ok(false);
        };
        if !keep {
            all.remove(primary);
        }
        all.push(Selection::new(a, b));
        let last = all.len() - 1;
        self.set_selections(all, last);
        Ok(true)
    }

    /// `⇧⌘L`：选中与主选区相同的全部文本；主选区是光标时按光标处的词（全字匹配）查找。
    ///
    /// 返回选中的处数；光标不在词上时返回 0。最多选中 [`MAX_MATCHES`] 处。
    ///
    /// # Errors
    ///
    /// 大文件（文本不在内存中）不支持，返回说明原因的文字。
    pub fn select_all_occurrences(&mut self) -> Result<usize, &'static str> {
        let (needle, whole_word, around) = if self.selection.is_empty() {
            let Some((a, b)) = self.word_around(self.selection.head) else {
                return Ok(0);
            };
            (self.text.substring(a, b), true, a)
        } else {
            let whole_word = self.extra_alive() && self.extra.whole_word;
            let from = self.selection.from();
            (
                self.text.substring(from, self.selection.to()),
                whole_word,
                from,
            )
        };
        let found = self.find_all(&needle, whole_word)?;
        if found.is_empty() {
            return Ok(0);
        }
        self.record_cursors();
        let primary = found.iter().rposition(|m| m.0 <= around).unwrap_or(0);
        let count = found.len();
        let all = found
            .into_iter()
            .map(|(a, b)| Selection::new(a, b))
            .collect();
        self.set_selections(all, primary);
        self.extra.whole_word = whole_word;
        Ok(count)
    }

    /// 列选择（`⇧⌥` 拖动）：从第 `anchor.0` 行的显示列 `anchor.1` 到第 `head.0` 行的显示列
    /// `head.1` 框选一块，每行各一个选区，`head` 所在行的选区为主选区。
    ///
    /// 行号与显示列均从 0 起。
    ///
    /// 比框的左边还短的行不放选区（框的宽度为 0 时同样跳过），但 `head` 所在的行总会有一个
    /// 选区；折叠隐藏的行跳过。
    pub fn select_columns(
        &mut self,
        anchor: (usize, usize),
        head: (usize, usize),
        tab_width: usize,
    ) {
        let last = self.text.line_count().saturating_sub(1);
        let (anchor_line, head_line) = (anchor.0.min(last), head.0.min(last));
        let left = anchor.1.min(head.1);
        let columns = self.columns(tab_width);
        let columns = columns.as_deref();
        let mut all = Vec::new();
        let mut primary = 0;
        for line in anchor_line.min(head_line)..=anchor_line.max(head_line) {
            let is_head = line == head_line;
            // 超过上限的部分最终要舍弃（见 [`Self::set_selections`]），只留下 `head` 所在的行。
            if all.len() > MAX_CURSORS && !is_head {
                continue;
            }
            if !is_head && self.folds.is_hidden(line) {
                continue;
            }
            let len = self.text.line_len(line);
            if !is_head && self.col_to_cell_with(columns, line, len, tab_width) < left {
                continue;
            }
            let start = self.text.line_start(line);
            let at = |cell: usize| {
                start
                    + self
                        .cell_to_col_with(columns, line, cell, tab_width)
                        .min(len)
            };
            if is_head {
                primary = all.len();
            }
            all.push(Selection::new(at(anchor.1), at(head.1)));
        }
        self.set_selections(all, primary);
        self.cursor_memory.column = Some(ColumnBox {
            anchor,
            head: (head_line, head.1),
            primary: self.selection,
            revision: self.revision,
        });
    }

    /// 在多个选区上执行 `command`；调用方已确认有多个选区、命令不是撤销或重做。
    pub(crate) fn execute_each(&mut self, command: Command, page_lines: usize, tab_width: usize) {
        if command.is_edit() {
            self.edit_each(command, tab_width);
            return;
        }

        // 移动类命令：逐个把选区换成主选区，按单光标的规则移动（自动换行、折叠、按列对齐都一样
        // 处理），再收回结果。
        let (all, primary) = self.selections();
        let goals = self.goals(primary);
        let mut moved = Vec::with_capacity(all.len());
        let mut moved_goals = Vec::with_capacity(all.len());
        for (selection, goal) in all.into_iter().zip(goals) {
            self.selection = selection;
            self.goal_cell = goal;
            self.execute_one(command.clone(), page_lines, tab_width);
            moved.push(self.selection);
            moved_goals.push(self.goal_cell);
        }
        self.grown.clear();
        self.set_selections_with_goals(moved, moved_goals, primary);
    }

    /// 在每个选区（或每组行）上执行编辑命令 `command`，全部修改合成一个撤销单元。
    fn edit_each(&mut self, command: Command, tab_width: usize) {
        let (all, primary) = self.selections();
        let scope = scope_of(&command, &all);
        let command = match command {
            // 有选区时 Tab 是整行缩进；各选区混在一起时统一按整行缩进。
            Command::Indent if scope == Scope::Lines => Command::IndentLines,
            command => command,
        };
        let units = self.units(&all, primary, scope, &command);
        let pieces = self.pieces(&command, all.len());

        let before = all[primary];
        let extra_before: Vec<Selection> = others(&all, primary);
        // 从后往前处理，前面的选区通常不受后面修改的影响；向上移动行时反过来，否则前一组
        // 移上去时会换到已经处理过的那一组上。
        let upward = matches!(command, Command::MoveLines { down: false });
        // 各选区的当前位置：每应用一处修改，其余选区都随之移动。
        let mut current = Positions::new(all, upward);
        let mut changes = Vec::new();
        let mut typing = true;
        let mut first = usize::MAX;

        let order: Vec<&Unit> = if upward {
            units.iter().collect()
        } else {
            units.iter().rev().collect()
        };
        for unit in order {
            let members = unit.members.clone();
            let selections = &current.selections;
            self.selection = if members.len() == 1 {
                selections[members.start]
            } else {
                self.line_span(selections[members.clone()].iter().copied())
            };
            let piece;
            let command = match &pieces {
                Some(pieces) => {
                    let text = pieces[members.start].clone();
                    piece = match &command {
                        Command::Paste(_) => Command::Paste(text),
                        _ => Command::InsertLinesAbove(text),
                    };
                    &piece
                }
                None => &command,
            };
            let Some(transaction) = self.transaction(command, tab_width) else {
                continue;
            };
            transaction.apply(&mut Tracked {
                text: &mut self.text,
                folds: &mut self.folds,
                bookmarks: &mut self.bookmarks,
                whole: &mut self.whole,
                marks: &mut self.marks,
            });
            typing &= transaction.kind == EditKind::Typing;
            first = first.min(transaction.first_offset());

            current.apply(members, command, &transaction);
            changes.extend(transaction.changes);
        }

        // 只移动了光标而没有改动文本的修改（例如跳过已有的右括号）不记入撤销栈。
        changes.retain(|c| c.removed != c.inserted);
        let end = self.text.end_pos();
        let current: Vec<Selection> = current
            .finish()
            .into_iter()
            .map(|s| s.clamped(end))
            .collect();
        if changes.is_empty() {
            self.set_selections(current, primary);
            return;
        }

        let (merged, primary) = merge(current, primary);
        let mut transaction = Transaction::new(
            changes,
            before,
            merged[primary],
            if typing {
                EditKind::Typing
            } else {
                EditKind::Other
            },
        );
        transaction.extra_before = extra_before;
        transaction.extra_after = others(&merged, primary);
        self.history.push(transaction);
        self.after_change(first);
        self.set_selections(merged, primary);
    }

    /// 撤销、重做或应用事务之后，按事务记录的其余选区恢复多光标。
    pub(crate) fn restore_extra(&mut self, extra: Vec<Selection>) {
        if extra.is_empty() {
            self.extra = Extra::default();
            return;
        }
        let primary = self.selection;
        let mut all = extra;
        let index = all.partition_point(|s| s.from() < primary.from());
        all.insert(index, primary);
        self.set_selections(all, index);
    }

    /// 返回把编辑命令 `command` 分给各选区执行的单位，按位置排序。
    fn units(
        &self,
        all: &[Selection],
        primary: usize,
        scope: Scope,
        command: &Command,
    ) -> Vec<Unit> {
        match scope {
            Scope::Each => (0..all.len()).map(|i| Unit { members: i..i + 1 }).collect(),
            Scope::Once => vec![Unit {
                members: primary..primary + 1,
            }],
            Scope::Lines => {
                // 行范围重叠的选区并成一组；相邻的也并成一组，使移动行、复制行时整块一起移动。
                // 合并行例外：相邻的几组各自合并。
                let touching = usize::from(!matches!(command, Command::JoinLines));
                let mut units: Vec<(usize, Unit)> = Vec::new();
                for (i, &selection) in all.iter().enumerate() {
                    let (first, last) = selected_lines(&self.text, selection);
                    match units.last_mut() {
                        Some((end, unit)) if first <= *end + touching => {
                            *end = (*end).max(last);
                            unit.members.end = i + 1;
                        }
                        _ => units.push((last, Unit { members: i..i + 1 })),
                    }
                }
                units.into_iter().map(|(_, u)| u).collect()
            }
        }
    }

    /// 多光标粘贴：剪贴板的行数与选区个数相同时，按顺序每个选区分到一行，返回分好的各段；
    /// 否则返回 `None`，每个选区都粘贴整段文字。
    ///
    /// 整行粘贴（`InsertLinesAbove`）的每段保留行尾的换行符。
    fn pieces(&self, command: &Command, count: usize) -> Option<Vec<String>> {
        let (text, whole_lines) = match command {
            Command::Paste(text) => (text, false),
            Command::InsertLinesAbove(text) => (text, true),
            _ => return None,
        };
        let pieces: Vec<String> = if whole_lines {
            text.split_inclusive('\n').map(str::to_owned).collect()
        } else {
            let text = text.strip_suffix('\n').unwrap_or(text);
            let text = text.strip_suffix('\r').unwrap_or(text);
            text.split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
                .collect()
        };
        (pieces.len() == count && count > 1).then_some(pieces)
    }

    /// 返回覆盖 `selections` 所在各行的选区：从第一行行首到最后一行行尾。
    ///
    /// 不能直接从最前一处选到最后一处：最后一个光标在行首时，按行操作会把那一行当作没选中。
    fn line_span(&self, selections: impl Iterator<Item = Selection>) -> Selection {
        let (first, last) = selections.fold((usize::MAX, 0), |(a, b), s| {
            let (first, last) = selected_lines(&self.text, s);
            (a.min(first), b.max(last))
        });
        Selection::new(self.text.line_start(first), self.text.line_end(last))
    }

    /// 返回各选区连续上下移动时保持的显示列，与 [`Self::selections`] 返回的全部选区一一
    /// 对应；`primary` 是其中主选区的下标。
    ///
    /// 全部选区就是其余选区在 `primary` 处插入主选区，显示列也照此排列。
    fn goals(&self, primary: usize) -> Vec<Option<usize>> {
        let mut goals = if self.extra_valid() {
            self.extra.goals.clone()
        } else {
            Vec::new()
        };
        goals.insert(primary, self.goal_cell);
        goals
    }

    /// 主选区是光标时，把每个光标都扩成所在的词（`⌘D` 的第一步）；返回是否选中了词。
    fn select_word_at_cursors(&mut self) -> bool {
        let Some(_) = self.word_around(self.selection.head) else {
            return false;
        };
        let (all, primary) = self.selections();
        let all = all
            .into_iter()
            .map(|s| match self.word_around(s.head) {
                Some((a, b)) if s.is_empty() => Selection::new(a, b),
                _ => s,
            })
            .collect();
        self.set_selections(all, primary);
        // 单光标时没有其余选区，全字匹配也会记下：`set_selections` 已记下主选区和版本号。
        self.extra.whole_word = true;
        true
    }

    /// 返回 `pos`（`char` 偏移）处的词（字母、数字、下划线组成）的范围（`char` 偏移，左闭右开）；
    /// 不在词上时返回 `None`。
    fn word_around(&self, pos: usize) -> Option<(usize, usize)> {
        let (a, b) = movement::word_at(&self.text, pos);
        let word = a < b && self.text.substring(a, b).chars().all(is_word_char);
        word.then_some((a, b))
    }

    /// 返回全文中 `needle` 的每一处出现（字符偏移，左闭右开），区分大小写，按位置排序，
    /// 最多 [`MAX_MATCHES`] 处。
    ///
    /// `whole_word` 为真时两端不能紧挨着字母、数字或下划线。
    fn find_all(
        &self,
        needle: &str,
        whole_word: bool,
    ) -> Result<Vec<(usize, usize)>, &'static str> {
        let mut found = Vec::new();
        self.each_match(needle, whole_word, 0, |m| {
            found.push(m);
            found.len() < MAX_MATCHES
        })?;
        Ok(found)
    }

    /// 从字符偏移 `from` 起按位置把 `needle` 的每一处出现交给 `f`，`f` 返回假时停下；匹配规则同
    /// [`Self::find_all`]。
    ///
    /// 直接在文本的各个块里找，不把全文拼成字符串：`⌘D` 找到下一处就停。
    fn each_match(
        &self,
        needle: &str,
        whole_word: bool,
        from: usize,
        mut f: impl FnMut((usize, usize)) -> bool,
    ) -> Result<(), &'static str> {
        let Content::Rope(rope) = &self.text else {
            return Err(t!("editor-large-no-multi-cursor"));
        };
        if needle.is_empty() {
            return Ok(());
        }
        let len = needle.chars().count();
        // 重叠的匹配（如在 `aaa` 中找 `aa`）只取前一处，见 [`each_byte_match`]。
        each_byte_match(rope, needle.as_bytes(), rope.char_to_byte(from), |at| {
            let start = rope.byte_to_char(at);
            if whole_word {
                let before = start.checked_sub(1).and_then(|c| rope.get_char(c));
                let after = rope.get_char(start + len);
                if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
                    return true;
                }
            }
            f((start, start + len))
        });
        Ok(())
    }

    /// 其余选区是否仍对应当前的主选区和文本（不论是否为空）。
    fn extra_alive(&self) -> bool {
        self.extra.primary == self.selection && self.extra.revision == self.revision
    }

    /// 是否有仍然有效的其余选区。
    fn extra_valid(&self) -> bool {
        !self.extra.selections.is_empty() && self.extra_alive()
    }
}

/// 返回 `all` 中除第 `primary` 个以外的选区。
pub(crate) fn others(all: &[Selection], primary: usize) -> Vec<Selection> {
    all.iter()
        .enumerate()
        .filter(|&(i, _)| i != primary)
        .map(|(_, &s)| s)
        .collect()
}

/// 是否是词的组成字符：字母、数字或下划线。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests;

/// 把 `rope` 里从字节偏移 `from` 起 `needle` 的每一处出现（字节偏移，按位置，互不重叠，和对整段
/// 文字做 `memmem::find_iter` 一样）交给 `f`，`f` 返回假时停下。
///
/// 块内的直接查找；跨块的取前面最后 `needle.len() - 1` 个字节与这一块的开头拼接后查找，
/// 块再短也不会漏（未拼完的部分留到下一块）。
fn each_byte_match(
    rope: &ropey::Rope,
    needle: &[u8],
    from: usize,
    mut f: impl FnMut(usize) -> bool,
) {
    let finder = Finder::new(needle);
    let keep = needle.len() - 1;
    let (chunks, mut start, _, _) = rope.chunks_at_byte(from);
    // 下一处最早可以从哪里开始：上一处的末尾（不重叠）。
    let mut allowed = from;
    // 前面最后 `keep` 个字节。
    let mut tail: Vec<u8> = Vec::new();
    let mut joined = Vec::new();
    for chunk in chunks {
        let bytes = chunk.as_bytes();
        let base = start - tail.len();
        let skip = allowed.saturating_sub(base);
        if skip < tail.len() {
            joined.clear();
            joined.extend_from_slice(&tail);
            joined.extend_from_slice(&bytes[..bytes.len().min(keep)]);
            // 只要从前面开始、落到这一块里的；整个在这一块里的下面找。
            for at in finder.find_iter(&joined[skip..]) {
                let at = skip + at;
                if at >= tail.len() {
                    break;
                }
                if !f(base + at) {
                    return;
                }
                allowed = base + at + needle.len();
            }
        }
        let skip = allowed.saturating_sub(start).min(bytes.len());
        for at in finder.find_iter(&bytes[skip..]) {
            let at = start + skip + at;
            if !f(at) {
                return;
            }
            allowed = at + needle.len();
        }
        if bytes.len() >= keep {
            tail.clear();
            tail.extend_from_slice(&bytes[bytes.len() - keep..]);
        } else {
            tail.extend_from_slice(bytes);
            tail.drain(..tail.len().saturating_sub(keep));
        }
        start += bytes.len();
    }
}
