//! 模态编辑层在 [`Document`] 上需要的接口：撤销分组、按位置给出的修改、折叠与显示行的查询。
//!
//! 模态编辑层自己计算修改的范围与修改之后的光标，不经 [`Command`]；这里把修改组成事务交给
//! [`Document::apply`]，并提供按文本行之外的单位（已折叠的区域、显示行）查询位置的方法。

use std::collections::VecDeque;
use std::ops::Range;
use std::time::{Duration, Instant};

use winc_core::edit::EditKind;
use winc_core::{Change, Lines, Selection, Transaction};

use crate::document::content::Content;
use crate::editing::search::Pattern;
use crate::{Command, Document, EDIT_LIMIT};

/// 修改列表保留的位置个数（个）；超出时丢弃最早的一个。
///
/// 取自调参：足以回到最近一段编辑经过的各处。
const MAX_CHANGES: usize = 100;

/// 一次查找最多占用的时间；超出时按找不到处理。
///
/// 取自调参：查找在界面线程上执行，没有匹配时要扫过整篇文本，该上限使大文件上的一次按键
/// 不会长时间停顿。
const FIND_BUDGET: Duration = Duration::from_millis(150);

/// 跨行的模式在一行上匹配时连同其后取用的行数（行）。
///
/// 取自调参：匹配可以延伸到之后的行，更长的匹配不识别。
const FIND_LINES: usize = 32;

/// 模态编辑层的标记与修改列表：位置随文本的修改移动。
#[derive(Debug, Default)]
pub(crate) struct Marks {
    /// 各标记的名字与位置。
    marks: Vec<(char, usize)>,
    /// 修改列表中的位置，从旧到新。
    changes: Vec<usize>,
    /// 逐行执行命令期间被跟踪的位置，从小到大。
    tracked: VecDeque<usize>,
}

impl Marks {
    /// 是否没有任何标记与修改记录。
    pub(crate) fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.changes.is_empty() && self.tracked.is_empty()
    }

    /// 文本 `text` 即将被修改：从 `start` 起的 `removed` 个字符换成 `inserted` 个字符。
    ///
    /// 所在的行被整行删除的标记一并删除，其余位置随修改移动。
    pub(crate) fn edit(&mut self, text: &Content, start: usize, removed: usize, inserted: usize) {
        let end = start + removed;
        let moved = |pos: usize| {
            if pos < start {
                pos
            } else if pos < end {
                start
            } else {
                pos - removed + inserted
            }
        };
        // 位置所在的行是否被整行删除。
        let deleted = |pos: usize| {
            if pos < start || pos >= end {
                return false;
            }
            let line = text.line_of(pos);
            let (from, to) = (text.line_start(line), text.line_end(line));
            start <= from && to <= end && (start < from || to < end)
        };
        self.marks.retain(|(_, pos)| !deleted(*pos));
        self.tracked.retain(|pos| !deleted(*pos));
        for (_, pos) in &mut self.marks {
            *pos = moved(*pos);
        }
        for pos in &mut self.changes {
            *pos = moved(*pos);
        }
        for pos in &mut self.tracked {
            *pos = moved(*pos);
        }
    }
}

/// 模态编辑层的查找模式：已编译的正则，`^`、`$` 按行匹配。
#[derive(Debug, Clone)]
pub struct ModalPattern {
    /// 已编译的正则。
    pattern: Pattern,
    /// 模式是否可以匹配换行：匹配时连同之后的若干行。
    multiline: bool,
}

impl ModalPattern {
    /// 编译正则 `regex`；`ignore_case` 为真时忽略大小写。
    ///
    /// # Errors
    ///
    /// 正则有误时返回错误说明。
    pub fn new(regex: &str, ignore_case: bool) -> Result<Self, String> {
        Ok(Self {
            pattern: Pattern::new(regex, ignore_case, true)?,
            multiline: regex.contains("\\n") || regex.contains("(?s"),
        })
    }

    /// 返回第 `line` 行上的全部匹配（字符偏移），按起点排序。
    ///
    /// 从行首起依次查找：非空的匹配之后从其终点继续，空的匹配之后前进一个字符；跨行的模式
    /// 可以延伸到之后的行，起点仍在该行之内。
    fn in_line(&self, text: &Content, line: usize) -> Vec<(usize, usize)> {
        let start = text.line_start(line);
        let content = text.line_content(line);
        let mut haystack = content.slice().to_string();
        let len = haystack.len();
        if self.multiline {
            let last = (line + FIND_LINES).min(text.line_count().saturating_sub(1));
            for next in line + 1..=last {
                haystack.push('\n');
                haystack.push_str(&text.line_content(next).slice().to_string());
            }
        }
        let chars = |byte: usize| haystack[..byte.min(haystack.len())].chars().count();
        let mut found = Vec::new();
        let mut from = 0;
        while let Ok(Some((s, e))) = self.pattern.find_at(&haystack, from) {
            if s > len {
                break;
            }
            found.push((start + chars(s), start + chars(e)));
            if e > len || s == len {
                break;
            }
            from = if e > s {
                e
            } else {
                s + haystack[s..].chars().next().map_or(1, char::len_utf8)
            };
        }
        found
    }

    /// 返回第 `line` 行上起点不小于字符偏移 `from` 的第一处匹配及其各分组的位置区间。
    fn groups_at(&self, text: &Content, from: usize) -> Option<Vec<Option<(usize, usize)>>> {
        let line = text.line_of(from);
        let start = text.line_start(line);
        let content = text.line_content(line);
        let mut haystack = content.slice().to_string();
        let len = haystack.len();
        if self.multiline {
            let last = (line + FIND_LINES).min(text.line_count().saturating_sub(1));
            for next in line + 1..=last {
                haystack.push('\n');
                haystack.push_str(&text.line_content(next).slice().to_string());
            }
        }
        let offset = (haystack.char_indices())
            .nth(from - start)
            .map_or(len, |(byte, _)| byte);
        let groups = self.pattern.captures_at(&haystack, offset).ok()??;
        let (whole, _) = groups.first().copied().flatten()?;
        if whole > len {
            return None;
        }
        let chars = |byte: usize| start + haystack[..byte.min(haystack.len())].chars().count();
        Some(
            groups
                .into_iter()
                .map(|group| group.map(|(a, b)| (chars(a), chars(b))))
                .collect(),
        )
    }
}

impl Document {
    /// 开始一个撤销分组：到对应的 [`Document::end_undo_group`] 为止的修改合并成一个撤销单元。
    pub fn begin_undo_group(&mut self) {
        self.history.begin_group();
    }

    /// 结束最近一次 [`Document::begin_undo_group`] 开始的分组。
    pub fn end_undo_group(&mut self) {
        self.history.end_group();
    }

    /// 返回下一次重做的撤销单元执行之前光标所在的位置；没有可重做的内容时返回 `None`。
    pub fn redo_origin(&self) -> Option<usize> {
        self.history.redo_origin().map(|selection| selection.head)
    }

    /// 是否可以读取并修改位置区间 `from..to` 内的文本：区间的字节数不超过 [`EDIT_LIMIT`]。
    pub fn fits_edit(&self, from: usize, to: usize) -> bool {
        self.text.byte_len(from.min(to), to) <= EDIT_LIMIT
    }

    /// 依次应用 `changes`，光标设在 `after`，撤销之后光标位于 `before`；返回是否已应用。
    ///
    /// 每处修改的起点是应用之前各处修改后的位置。文档只读，或被删除的文本合计超过
    /// [`EDIT_LIMIT`] 时不应用。其余选区（多光标）被收回。
    pub fn apply_changes(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool {
        let removed: usize = changes.iter().map(|change| change.removed.len()).sum();
        if self.readonly || removed as u64 > EDIT_LIMIT {
            return false;
        }
        self.collapse_selections();
        self.goal_cell = None;
        self.apply(Transaction::new(
            changes,
            Selection::point(before),
            Selection::point(after),
            EditKind::Other,
        ));
        true
    }

    /// 在位置 `at` 换行并把光标放在新行上，缩进规则与 [`Command::Newline`] 相同；撤销之后
    /// 选区回到调用之前的状态。
    pub fn open_line_at(&mut self, at: usize, tab_width: usize) {
        if self.readonly {
            return;
        }
        let original = self.selection;
        self.collapse_selections();
        self.selection = Selection::point(at.min(self.text.end_pos()));
        self.goal_cell = None;
        match self.transaction(&Command::Newline, tab_width) {
            Some(mut transaction) => {
                transaction.before = original;
                self.apply(transaction);
            }
            None => self.selection = original,
        }
    }

    /// 切换第 `first` 至 `last` 行的行注释，返回是否有修改；选区在修改之后收回为光标。
    pub fn toggle_comment_lines(&mut self, first: usize, last: usize, tab_width: usize) -> bool {
        if self.readonly {
            return false;
        }
        let original = self.selection;
        self.collapse_selections();
        let last = last.min(self.text.line_count().saturating_sub(1));
        self.selection = Selection::new(self.text.line_start(first), self.text.line_end(last));
        let Some(mut transaction) = self.transaction(&Command::ToggleComment, tab_width) else {
            self.selection = original;
            return false;
        };
        transaction.before = Selection::point(original.head);
        transaction.after = Selection::point(self.text.line_start(first));
        self.apply(transaction);
        true
    }

    /// 返回模态编辑层的标记 `name` 的位置；没有设置时返回 `None`。
    pub fn modal_mark(&self, name: char) -> Option<usize> {
        let end = self.text.end_pos();
        (self.marks.marks.iter())
            .find(|(mark, _)| *mark == name)
            .map(|(_, pos)| (*pos).min(end))
    }

    /// 删除模态编辑层的标记 `name`。
    pub fn clear_modal_mark(&mut self, name: char) {
        self.marks.marks.retain(|(mark, _)| *mark != name);
    }

    /// 返回模态编辑层的全部标记：名字与位置，按名字排序。
    pub fn modal_marks(&self) -> Vec<(char, usize)> {
        let end = self.text.end_pos();
        let mut marks: Vec<(char, usize)> = (self.marks.marks.iter())
            .map(|(name, pos)| (*name, (*pos).min(end)))
            .collect();
        marks.sort_unstable();
        marks
    }

    /// 把模态编辑层的标记 `name` 设在位置 `pos`；标记随文本的修改移动。
    pub fn set_modal_mark(&mut self, name: char, pos: usize) {
        match self.marks.marks.iter_mut().find(|(mark, _)| *mark == name) {
            Some(mark) => mark.1 = pos,
            None => self.marks.marks.push((name, pos)),
        }
    }

    /// 返回模态编辑层的修改列表，从旧到新。
    pub fn modal_changes(&self) -> &[usize] {
        &self.marks.changes
    }

    /// 把位置 `pos` 记入模态编辑层的修改列表；与上一处修改在同一行时只更新位置。
    pub fn record_modal_change(&mut self, pos: usize) {
        let line = self.text.line_of(pos.min(self.text.end_pos()));
        let text = &self.text;
        let changes = &mut self.marks.changes;
        match changes.last_mut() {
            Some(last) if text.line_of((*last).min(text.end_pos())) == line => *last = pos,
            _ => {
                changes.push(pos);
                if changes.len() > MAX_CHANGES {
                    changes.remove(0);
                }
            }
        }
    }

    /// 返回 `pattern` 起点不小于 `from`（`forward` 为真）或小于 `from` 的最近一处匹配。
    ///
    /// 每行各自从行首起依次查找，匹配互不重叠。文本以换行符结尾时末尾的空行不参与查找。
    /// 超过时间上限仍未找到时返回 `None`。
    pub fn find_modal(
        &self,
        pattern: &ModalPattern,
        from: usize,
        forward: bool,
    ) -> Option<(usize, usize)> {
        let text = &self.text;
        let count = text.line_count();
        let lines = if count > 1 && text.line_start(count - 1) == text.end_pos() {
            count - 1
        } else {
            count
        };
        let started = Instant::now();
        let at = text.line_of(from.min(text.end_pos())).min(lines - 1);
        let expired = |scanned: usize| scanned % 256 == 255 && started.elapsed() > FIND_BUDGET;
        if forward {
            for (scanned, line) in (at..lines).enumerate() {
                let found = pattern.in_line(text, line);
                if let Some(hit) = found.into_iter().find(|hit| hit.0 >= from) {
                    return Some(hit);
                }
                if expired(scanned) {
                    return None;
                }
            }
            return None;
        }
        for (scanned, line) in (0..=at).rev().enumerate() {
            let found = pattern.in_line(text, line);
            if let Some(hit) = found.into_iter().rev().find(|hit| hit.0 < from) {
                return Some(hit);
            }
            if expired(scanned) {
                return None;
            }
        }
        None
    }

    /// 返回 `pattern` 在 `from` 所在的行上起点不小于 `from` 的第一处匹配及其各分组的位置区间。
    ///
    /// 第 0 项是整个匹配，没有参与匹配的分组为 `None`；跨行的模式可以延伸到之后的行。
    pub fn match_modal(
        &self,
        pattern: &ModalPattern,
        from: usize,
    ) -> Option<Vec<Option<(usize, usize)>>> {
        pattern.groups_at(&self.text, from.min(self.text.end_pos()))
    }

    /// 开始跟踪 `positions`（从小到大），替换已在跟踪的位置：之后的修改使这些位置随文本移动，
    /// 所在的行被整行删除时作废。
    pub fn track_modal(&mut self, positions: Vec<usize>) {
        self.marks.tracked = positions.into();
    }

    /// 取出被跟踪的位置中最靠前的一个；没有时返回 `None`。
    pub fn next_tracked_modal(&mut self) -> Option<usize> {
        self.marks.tracked.pop_front()
    }

    /// 返回第 `line` 行之后下一个可折叠的块的首行（`forward` 为真），或之前上一个可折叠的块
    /// 的末行。
    pub fn fold_edge(&mut self, line: usize, forward: bool, tab_width: usize) -> Option<usize> {
        if !self.can_fold() {
            return None;
        }
        let ranges = self.fold_ranges(tab_width);
        if forward {
            (ranges.iter())
                .map(|range| range.start)
                .filter(|start| *start > line)
                .min()
        } else {
            (ranges.iter())
                .map(|range| range.end)
                .filter(|end| *end < line)
                .max()
        }
    }

    /// 是否存在已折叠的块。
    pub fn has_collapsed_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    /// 返回包含第 `line` 行的已折叠的块的首行与末行；嵌套时取最外层，不在折叠中时返回 `None`。
    pub fn collapsed_fold_at(&self, line: usize) -> Option<(usize, usize)> {
        (self.folds.iter())
            .filter(|fold| fold.start <= line && line <= fold.end)
            .map(|fold| (fold.start, fold.end))
            .reduce(|(first, last), (start, end)| (first.min(start), last.max(end)))
    }
}

impl Document {
    /// 返回位置 `pos` 所在显示行的位置范围；不换行或该行只有一个显示行时为整行。
    pub fn display_row_span(&self, pos: usize) -> Range<usize> {
        let (line, col) = self.text.pos_to_line_col(pos);
        let start = self.text.line_start(line);
        if self.folds.wrap().is_none() || self.rows_of(line) == 1 {
            return start..self.text.line_end(line);
        }
        let row = self.folds.row_of(line) + self.part_of(line, col);
        let (first, last) = self.row_bounds(row);
        first..last
    }
}

#[cfg(test)]
mod tests;
