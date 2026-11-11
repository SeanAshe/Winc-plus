//! 测试用的宿主：以 `Rope` 保存文本，修改经撤销历史记录；插入模式的输入按原样执行，
//! 没有自动缩进与括号补全。

#![allow(dead_code, reason = "各测试文件只用到其中一部分")]

use std::collections::BTreeMap;
use std::ops::Range;

use ropey::Rope;
use winc_core::edit::{self, EditKind};
use winc_core::{Change, History, Lines, Selection, Transaction, movement};
use winc_vim::{AppCommand, Code, Fold, Host, HostOption, Indentation, Input, Key, Message, Mode};
use winc_vim::{Named, Query, Scroll, Shared, Vim, notation};

/// 窗口的高度（行），与无界面录制时相同。
const HEIGHT: usize = 23;

/// 测试用的宿主。
pub struct TestHost {
    /// 文本。
    pub rope: Rope,
    /// 主选区。
    pub selection: Selection,
    /// 撤销历史。
    pub history: History,
    /// 系统剪贴板的替身。
    pub clipboard: Option<String>,
    /// 收到的提示。
    pub messages: Vec<Message>,
    /// 窗口中显示的行；`None` 表示从光标所在的行起的 23 行，与无界面录制时的窗口相同。
    pub visible: Option<Range<usize>>,
    /// 是否只读。
    pub read_only: bool,
    /// 已折叠的区域：首行与末行。
    pub folds: Vec<(usize, usize)>,
    /// 新开的行是否保留缩进。
    pub keeps_indent: bool,
    /// 缩进的方式。
    pub indentation: Indentation,
    /// 未结束的撤销分组的层数。
    pub groups: usize,
    /// 收到的切换注释的请求：首行与末行。
    pub commented: Vec<(usize, usize)>,
    /// 自动换行的宽度（字符）；`None` 表示不换行。
    pub wrap: Option<usize>,
    /// 主选区之外的选区。
    pub others: Vec<Selection>,
    /// 标记的位置，随修改移动。
    pub marks: BTreeMap<char, usize>,
    /// 修改列表，从旧到新。
    pub changes: Vec<usize>,
    /// 跳转历史，从旧到新。
    pub jumps: Vec<usize>,
    /// 跳转历史中当前的位置；等于长度时在末尾之后。
    pub jump_index: usize,
    /// 滚动之后窗口的首行；光标移动之后窗口重新从光标所在的行算起。
    pub top: Option<usize>,
    /// 可折叠的区域：首行与末行。
    pub foldable: Vec<(usize, usize)>,
    /// 撤销与重做的次数。
    pub undone: usize,
    /// 被跟踪的位置，从小到大，随修改移动。
    pub tracked: Vec<usize>,
    /// 收到的交给宿主执行的命令。
    pub apps: Vec<AppCommand>,
    /// 由宿主保存的选项的值。
    pub options: Vec<(HostOption, usize)>,
    /// 收到的格式化请求：首行与末行。
    pub formatted: Vec<(usize, usize)>,
    /// 光标与窗口上下边缘之间保留的行数。
    pub margin: usize,
}

impl TestHost {
    /// 创建文本为 `text`、光标在开头的宿主；缩进方式与 `vim` 不加载配置时相同。
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            selection: Selection::point(0),
            history: History::default(),
            clipboard: None,
            messages: Vec::new(),
            visible: None,
            read_only: false,
            folds: Vec::new(),
            keeps_indent: false,
            indentation: Indentation {
                width: 8,
                tabs: true,
            },
            groups: 0,
            commented: Vec::new(),
            wrap: None,
            others: Vec::new(),
            marks: BTreeMap::new(),
            changes: Vec::new(),
            jumps: Vec::new(),
            jump_index: 0,
            top: None,
            foldable: Vec::new(),
            undone: 0,
            tracked: Vec::new(),
            apps: Vec::new(),
            options: Vec::new(),
            formatted: Vec::new(),
            margin: 0,
        }
    }

    /// 应用事务并记入撤销历史；标记、修改列表与其余选区随修改移动，所在的行被整行删除的
    /// 标记一并删除。
    fn commit(&mut self, transaction: Transaction) {
        // 修改按位置从后往前排列，各处的起点都是修改之前的位置。
        let rope = &self.rope;
        let removed_line = |pos: usize| {
            transaction.changes.iter().any(|change| {
                let (from, to) = (change.start, change.start + change.removed.chars().count());
                let line = rope.line_of(pos);
                let (start, end) = (rope.line_start(line), rope.line_end(line));
                // 位于删除范围终点的位置仍然有效。
                let inside = from <= pos && pos < to;
                inside && from <= start && end <= to && (from < start || end < to)
            })
        };
        self.marks.retain(|_, pos| !removed_line(*pos));
        self.tracked.retain(|pos| !removed_line(*pos));
        transaction.apply(&mut self.rope);
        let moved =
            |pos: usize| edit::shift_selection(Selection::point(pos), &transaction.changes).head;
        for pos in self.marks.values_mut() {
            *pos = moved(*pos);
        }
        for pos in &mut self.changes {
            *pos = moved(*pos);
        }
        for pos in &mut self.tracked {
            *pos = moved(*pos);
        }
        for other in &mut self.others {
            *other = edit::shift_selection(*other, &transaction.changes);
        }
        self.selection = transaction.after.clamped(self.rope.len_chars());
        self.top = None;
        if !transaction.changes.is_empty() {
            self.history.push(transaction);
        }
    }

    /// 返回文本中 `query` 的全部匹配（字符偏移），按起点排序。
    ///
    /// 每行各自从行首起依次查找：非空的匹配之后从其终点继续，空的匹配之后前进一个字符；匹配
    /// 可以延伸到之后的行。先用 `regex` 编译，其不支持的写法（环视、反向引用）改用
    /// `fancy-regex`。
    fn matches(&self, query: &Query) -> Vec<(usize, usize)> {
        let text = self.rope.to_string();
        let plain = regex::RegexBuilder::new(&query.regex)
            .case_insensitive(query.ignore_case)
            .multi_line(true)
            .build();
        let fancy = match &plain {
            Ok(_) => None,
            Err(_) => fancy_regex::RegexBuilder::new(&query.regex)
                .case_insensitive(query.ignore_case)
                .multi_line(true)
                .build()
                .ok(),
        };
        let find_at = |from: usize| match (&plain, &fancy) {
            (Ok(regex), _) => regex.find_at(&text, from).map(|m| (m.start(), m.end())),
            (_, Some(regex)) => {
                (regex.find_from_pos(&text, from).ok().flatten()).map(|m| (m.start(), m.end()))
            }
            _ => None,
        };
        let mut found = Vec::new();
        let mut start = 0;
        // 文本以换行符结尾时，末尾的空行不算一行。
        let body = text.strip_suffix('\n').unwrap_or(&text);
        for line in body.split('\n') {
            let end = start + line.len();
            let mut from = start;
            while let Some((s, e)) = find_at(from) {
                if s > end {
                    break;
                }
                found.push((s, e));
                if e > end || s == end {
                    break;
                }
                from = if e > s {
                    e
                } else {
                    s + text[s..].chars().next().map_or(1, char::len_utf8)
                };
            }
            start = end + 1;
        }
        let chars = |byte: usize| text[..byte.min(text.len())].chars().count();
        found
            .into_iter()
            .map(|(s, e)| (chars(s), chars(e)))
            .collect()
    }

    /// 返回光标所在的行：选区向后文延伸时，光标在终点之前的一个字符上。
    fn cursor_line(&self) -> usize {
        let Selection { anchor, head } = self.selection;
        (self.rope).line_of(if head > anchor { head - 1 } else { head })
    }

    /// 返回最后一行的行号；文本以换行符结尾时末尾的空行不算。
    fn last_line(&self) -> usize {
        let count = self.rope.line_count();
        if count > 1 && self.rope.line_start(count - 1) == self.rope.len_chars() {
            count - 2
        } else {
            count - 1
        }
    }

    /// 在每个选区上执行 `edit`，从后往前，合成一个事务；`edit` 返回对一个选区的修改。
    fn each(&mut self, edit: impl Fn(&Rope, Selection) -> Option<Transaction>) {
        let mut all = self.others.clone();
        all.push(self.selection);
        all.sort_by_key(|s| s.from());
        let primary = all.iter().position(|s| *s == self.selection).unwrap_or(0);
        let before = self.selection;
        let mut changes = Vec::new();
        let mut kind = EditKind::Other;
        // 先在副本上依次执行，得到全部修改与各选区之后的位置；其余选区随每次修改移动。
        let mut rope = self.rope.clone();
        for index in (0..all.len()).rev() {
            let Some(transaction) = edit(&rope, all[index]) else {
                continue;
            };
            for (other, selection) in all.iter_mut().enumerate() {
                if other != index {
                    *selection = edit::shift_selection(*selection, &transaction.changes);
                }
            }
            transaction.apply(&mut rope);
            all[index] = transaction.after;
            kind = transaction.kind;
            changes.extend(transaction.changes);
        }
        let mut results = all;
        let after = results.remove(primary);
        self.commit(Transaction::new(changes, before, after, kind));
        self.others = results;
    }

    /// 返回文本。
    pub fn text(&self) -> String {
        self.rope.to_string()
    }
}

impl Host for TestHost {
    type Text = Rope;

    fn text(&self) -> &Rope {
        &self.rope
    }

    fn selection(&self) -> Selection {
        self.selection
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
        self.top = None;
    }

    fn selections(&self) -> Vec<Selection> {
        let mut all = self.others.clone();
        all.push(self.selection);
        all.sort_by_key(|s| s.from());
        all
    }

    fn set_selections(&mut self, mut selections: Vec<Selection>, primary: usize) {
        if primary < selections.len() {
            self.selection = selections.remove(primary);
            // 与主选区重合的选区并入主选区。
            selections.retain(|other| *other != self.selection);
            selections.dedup();
            self.others = selections;
            self.top = None;
        }
    }

    fn apply(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool {
        self.others.clear();
        let transaction = Transaction::new(
            changes,
            Selection::point(before),
            Selection::point(after),
            EditKind::Other,
        );
        self.commit(transaction);
        true
    }

    fn input(&mut self, input: &Input) {
        let indentation = self.indentation;
        // 把一行的缩进改为 `cells` 个显示列。
        let reindent = |rope: &Rope, head: usize, grow: bool| {
            let line = rope.line_of(head);
            let start = rope.line_start(line);
            let content = rope.line_content(line).slice().to_string();
            let blanks = content
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            let mut cells = 0;
            for c in content.chars().take(blanks) {
                cells += if c == '\t' { 8 - cells % 8 } else { 1 };
            }
            let width = indentation.width.max(1);
            let cells = if grow {
                (cells / width + 1) * width
            } else {
                (cells.saturating_sub(1)) / width * width
            };
            let new = if indentation.tabs {
                format!("{}{}", "\t".repeat(cells / 8), " ".repeat(cells % 8))
            } else {
                " ".repeat(cells)
            };
            let after = head + new.chars().count() - blanks.min(head - start);
            let change = Change {
                start,
                removed: content.chars().take(blanks).collect(),
                inserted: new,
            };
            Some(Transaction::new(
                vec![change],
                Selection::point(head),
                Selection::point(after.max(start)),
                EditKind::Other,
            ))
        };
        let input = input.clone();
        self.each(move |rope, selection| {
            let head = selection.head;
            let delete = |from: usize, to: usize| {
                edit::replace_selection(rope, Selection::new(from, to), "", EditKind::Other)
            };
            Some(match &input {
                Input::Text(text) | Input::Paste(text) => {
                    edit::replace_selection(rope, selection, text, EditKind::Typing)
                }
                Input::Newline => edit::replace_selection(rope, selection, "\n", EditKind::Other),
                Input::Indent => edit::replace_selection(rope, selection, "\t", EditKind::Other),
                Input::Backspace { word: false } => {
                    delete(movement::prev_grapheme(rope, head), head)
                }
                Input::Backspace { word: true } => delete(movement::prev_word(rope, head), head),
                Input::Delete { word: false } => delete(head, movement::next_grapheme(rope, head)),
                Input::Delete { word: true } => delete(head, movement::next_word(rope, head)),
                Input::DeleteToLineStart => delete(rope.line_start(rope.line_of(head)), head),
                Input::IndentLine => return reindent(rope, head, true),
                Input::Outdent => return reindent(rope, head, false),
            })
        });
    }

    fn open_line(&mut self, at: usize) {
        self.others.clear();
        let mut transaction =
            edit::replace_selection(&self.rope, Selection::point(at), "\n", EditKind::Other);
        transaction.before = self.selection;
        self.commit(transaction);
    }

    fn undo(&mut self) -> bool {
        self.undone += 1;
        self.others.clear();
        match self.history.undo(&mut self.rope) {
            Some(restored) => {
                self.selection = restored.selection.clamped(self.rope.len_chars());
                true
            }
            None => false,
        }
    }

    fn redo(&mut self) -> bool {
        self.undone += 1;
        self.others.clear();
        match self.history.redo(&mut self.rope) {
            Some(restored) => {
                self.selection = restored.selection.clamped(self.rope.len_chars());
                true
            }
            None => false,
        }
    }

    fn redo_origin(&self) -> Option<usize> {
        self.history.redo_origin().map(|selection| selection.head)
    }

    fn begin_group(&mut self) {
        self.groups += 1;
        self.history.begin_group();
    }

    fn end_group(&mut self) {
        self.groups = self.groups.checked_sub(1).expect("分组的开始与结束成对");
        self.history.end_group();
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn tab_width(&self) -> usize {
        8
    }

    fn indentation(&self) -> Indentation {
        self.indentation
    }

    fn keeps_indent(&self) -> bool {
        self.keeps_indent
    }

    fn visible_lines(&self) -> Range<usize> {
        self.visible.clone().unwrap_or_else(|| {
            let top = self.top.unwrap_or_else(|| self.cursor_line());
            top..top + HEIGHT
        })
    }

    fn scroll(&mut self, scroll: Scroll) -> bool {
        // 窗口的首行跟随光标，只有按行滚动会使首行暂时离开光标所在的行。
        let Scroll::Lines(lines) = scroll else {
            return false;
        };
        let current = self.top.unwrap_or_else(|| self.cursor_line());
        let top = current.saturating_add_signed(lines).min(self.last_line());
        self.top = Some(top);
        top != current
    }

    fn scroll_margin(&self) -> usize {
        self.margin
    }

    fn find(&mut self, query: &Query, from: usize, forward: bool) -> Option<(usize, usize)> {
        let matches = self.matches(query);
        if forward {
            matches.into_iter().find(|found| found.0 >= from)
        } else {
            matches.into_iter().rev().find(|found| found.0 < from)
        }
    }

    fn mark(&self, name: char) -> Option<usize> {
        self.marks.get(&name).copied()
    }

    fn set_mark(&mut self, name: char, pos: usize) {
        self.marks.insert(name, pos);
    }

    fn changes(&self) -> Vec<usize> {
        self.changes.clone()
    }

    fn record_change(&mut self, pos: usize) {
        // 与上一处修改在同一行时只更新位置。
        let line = self.rope.line_of(pos);
        match self.changes.last_mut() {
            Some(last) if self.rope.line_of(*last) == line => *last = pos,
            _ => self.changes.push(pos),
        }
    }

    fn record_jump(&mut self, pos: usize) {
        self.jumps.push(pos);
        self.jump_index = self.jumps.len();
    }

    fn jump(&mut self, back: bool, count: usize) -> bool {
        // 同一行上只保留最新的一条；当前的位置随所指的一条移动。
        let lines: Vec<usize> = (self.jumps.iter())
            .map(|pos| self.rope.line_of(*pos))
            .collect();
        let at_end = self.jump_index >= self.jumps.len();
        let mut kept = Vec::new();
        for (index, pos) in self.jumps.iter().enumerate() {
            if self.jump_index == index {
                self.jump_index = kept.len();
            }
            if !lines[index + 1..].contains(&lines[index]) {
                kept.push(*pos);
            }
        }
        self.jumps = kept;
        if at_end {
            self.jump_index = self.jumps.len();
        }
        if self.jumps.is_empty() {
            return false;
        }
        let step = |index: usize| {
            if back {
                index.checked_sub(count)
            } else {
                index.checked_add(count)
            }
        };
        if step(self.jump_index).is_none_or(|target| target >= self.jumps.len()) {
            return false;
        }
        // 位于末尾之后时先把光标的位置记入历史。
        let here = self.selection.head;
        let at_end = self.jump_index == self.jumps.len();
        if at_end {
            self.jumps.push(here);
            self.jump_index = self.jumps.len() - 1;
        }
        let Some(target) = step(self.jump_index).filter(|target| *target < self.jumps.len()) else {
            return false;
        };
        // 光标因此移动时，原来的位置成为「上一个位置」。
        if at_end && self.jumps[target] != here {
            self.marks.insert('\'', here);
        }
        self.jump_index = target;
        self.set_selection(Selection::point(self.jumps[target]));
        true
    }

    fn fold_at(&mut self, line: usize, fold: Fold) -> bool {
        let before = self.folds.clone();
        let here: Vec<(usize, usize)> = (self.foldable.iter())
            .copied()
            .filter(|(first, last)| (*first..=*last).contains(&line))
            .collect();
        // 最内层的区域排在最后。
        let inner = here.iter().copied().max_by_key(|(first, _)| *first);
        let closed = |folds: &[(usize, usize)], fold: (usize, usize)| folds.contains(&fold);
        match fold {
            Fold::OpenAll => self.folds.clear(),
            Fold::CloseAll => self.folds = self.foldable.clone(),
            Fold::Open | Fold::Reveal => self.folds.retain(|fold| !here.contains(fold)),
            Fold::OpenTree => self.folds.retain(|fold| !here.contains(fold)),
            Fold::Close | Fold::CloseTree => {
                let targets = if fold == Fold::Close {
                    inner.into_iter().collect()
                } else {
                    here.clone()
                };
                for target in targets {
                    if !closed(&self.folds, target) {
                        self.folds.push(target);
                    }
                }
            }
            Fold::Toggle | Fold::ToggleTree => {
                if here.iter().any(|fold| closed(&self.folds, *fold)) {
                    self.folds.retain(|fold| !here.contains(fold));
                } else if let Some(inner) = inner {
                    self.folds.push(inner);
                }
            }
        }
        self.folds != before
    }

    fn fold_edge(&mut self, line: usize, forward: bool) -> Option<usize> {
        if forward {
            (self.foldable.iter())
                .map(|fold| fold.0)
                .filter(|first| *first > line)
                .min()
        } else {
            (self.foldable.iter())
                .map(|fold| fold.1)
                .filter(|last| *last < line)
                .max()
        }
    }

    fn format_lines(&mut self, first: usize, last: usize) -> bool {
        self.formatted.push((first, last));
        true
    }

    fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    fn fold(&self, line: usize) -> Option<(usize, usize)> {
        // 嵌套时取最外层。
        (self.folds.iter())
            .copied()
            .filter(|(first, last)| (*first..=*last).contains(&line))
            .min_by_key(|(first, _)| *first)
    }

    fn clipboard(&mut self) -> Option<String> {
        self.clipboard.clone()
    }

    fn set_clipboard(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }

    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }

    fn find_groups(&mut self, query: &Query, from: usize) -> Option<Vec<Option<(usize, usize)>>> {
        let text = self.rope.to_string();
        let from = from.min(self.rope.len_chars());
        let line_end = self
            .rope
            .char_to_byte(self.rope.line_end(self.rope.line_of(from)));
        let start = self.rope.char_to_byte(from);
        let plain = regex::RegexBuilder::new(&query.regex)
            .case_insensitive(query.ignore_case)
            .multi_line(true)
            .build();
        let groups: Vec<Option<(usize, usize)>> = match plain {
            Ok(regex) => {
                let captures = regex.captures_at(&text, start)?;
                (captures.iter())
                    .map(|group| group.map(|m| (m.start(), m.end())))
                    .collect()
            }
            Err(_) => {
                let regex = fancy_regex::RegexBuilder::new(&query.regex)
                    .case_insensitive(query.ignore_case)
                    .multi_line(true)
                    .build()
                    .ok()?;
                let captures = regex.captures_from_pos(&text, start).ok()??;
                (captures.iter())
                    .map(|group| group.map(|m| (m.start(), m.end())))
                    .collect()
            }
        };
        let whole = groups.first().copied().flatten()?;
        if whole.0 > line_end {
            return None;
        }
        let chars = |byte: usize| self.rope.byte_to_char(byte);
        Some(
            groups
                .into_iter()
                .map(|group| group.map(|(a, b)| (chars(a), chars(b))))
                .collect(),
        )
    }

    fn track(&mut self, positions: Vec<usize>) {
        self.tracked = positions;
    }

    fn next_tracked(&mut self) -> Option<usize> {
        (!self.tracked.is_empty()).then(|| self.tracked.remove(0))
    }

    fn option(&self, option: HostOption) -> Option<usize> {
        let stored = self.options.iter().find(|(name, _)| *name == option);
        Some(stored.map_or(0, |(_, value)| *value))
    }

    fn set_option(&mut self, option: HostOption, value: usize) -> bool {
        self.options.retain(|(name, _)| *name != option);
        self.options.push((option, value));
        true
    }

    fn app(&mut self, command: AppCommand) -> bool {
        self.apps.push(command);
        true
    }

    fn toggle_comment(&mut self, first: usize, last: usize) -> bool {
        self.commented.push((first, last));
        true
    }

    fn text_width(&self) -> usize {
        self.wrap.unwrap_or(80)
    }

    fn display_row(&self, pos: usize) -> Range<usize> {
        let (line, col) = self.rope.pos_to_line_col(pos);
        let start = self.rope.line_start(line);
        let end = self.rope.line_end(line);
        match self.wrap {
            // 每 `width` 个字符一个显示行；位于行尾时属于最后一个显示行。
            Some(width) => {
                let row = col.min(self.rope.line_len(line).saturating_sub(1)) / width;
                (start + row * width)..(start + (row + 1) * width).min(end)
            }
            None => start..end,
        }
    }

    fn display_vertical(&self, pos: usize, delta: isize, cell: usize) -> Option<usize> {
        let width = self.wrap?;
        // 把各行的显示行依次排开，按显示行的序号移动。
        let rows = |line: usize| self.rope.line_len(line).div_ceil(width).max(1);
        let (mut line, col) = self.rope.pos_to_line_col(pos);
        let mut row = col.min(self.rope.line_len(line).saturating_sub(1)) / width;
        for _ in 0..delta.unsigned_abs() {
            if delta > 0 {
                if row + 1 < rows(line) {
                    row += 1;
                } else if line + 1 < self.rope.len_lines() - 1 {
                    line += 1;
                    row = 0;
                } else {
                    break;
                }
            } else if row > 0 {
                row -= 1;
            } else if line > 0 {
                line -= 1;
                row = rows(line) - 1;
            } else {
                break;
            }
        }
        let in_row = cell % width;
        let col = (row * width + in_row).min(self.rope.line_len(line));
        Some(self.rope.line_start(line) + col)
    }
}

/// 把按键交给 Vim 层；插入模式下未被处理的按键按键入的文字执行并记录。
///
/// 状态机等待输入命令行或查找的模式时，按键交给状态机自带的行内输入。
pub fn press(vim: &mut Vim, shared: &mut Shared, host: &mut TestHost, key: Key) {
    if vim.prompt().is_some() {
        vim.prompt_key(shared, host, key);
        return;
    }
    if vim.key(shared, host, key) || vim.mode() != Mode::Insert {
        return;
    }
    let input = match key.code() {
        Code::Char(c) if key.as_char().is_some() => Input::Text(c.to_string()),
        Code::Named(Named::Enter) => Input::Newline,
        Code::Named(Named::Tab) => Input::Text("\t".into()),
        Code::Named(Named::Backspace) => Input::Backspace { word: false },
        Code::Named(Named::Delete) => Input::Delete { word: false },
        _ => return,
    };
    // 替换模式下键入的文字由状态机覆盖到文本上。
    if let (true, Input::Text(text)) = (vim.is_replace(), &input) {
        vim.overtype(host, text);
        return;
    }
    host.input(&input);
    vim.typed(input);
}

/// 依次输入按键记法 `keys` 中的按键。
pub fn feed(vim: &mut Vim, shared: &mut Shared, host: &mut TestHost, keys: &str) {
    for key in notation::parse(keys) {
        press(vim, shared, host, key);
    }
}
