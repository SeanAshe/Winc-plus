//! 不带范围的编辑命令：替换字符、切换大小写、粘贴、合并行。

use winc_core::{Change, Lines};

use super::operate::content_of;
use super::{MAX_OUTPUT, Shared, Vim, VisualKind};
use crate::command::{Action, Operator};
use crate::host::{Host, Message};
use crate::operator::{self, Region};
use crate::parser::Command;
use crate::register::{Content, Kind, Source};
use crate::text;

/// 返回次数，未输入时为 1。
fn times(command: Command) -> usize {
    command
        .count
        .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX))
}

impl Vim {
    /// `r{字符}`：把光标起的「次数」个字符替换为 `arg`；可视模式下替换选区中的每个字符。
    pub(super) fn replace_char(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        arg: Option<char>,
    ) {
        let Some(with) = arg else {
            return;
        };
        if self.blocked(host) {
            return;
        }
        let cursor = self.caret(host);
        if let Some(visual) = self.visual {
            // 选区中换行符之外的每个字符各替换一次。
            let linewise = visual.kind == VisualKind::Line;
            let to_end = self.want == crate::motion::Want::End;
            if visual.kind == VisualKind::Block {
                // 按块：块在各行上的字符各替换一次。
                let block = super::block::block_of(host, visual, to_end, self.columns.take());
                let rows = super::block::rows_of(host, block);
                let Some(top) = rows.first().copied() else {
                    return;
                };
                if with == '\r' {
                    self.error(host, Message::Unsupported);
                    return;
                }
                let changes: Vec<Change> = (rows.iter().rev())
                    .filter(|row| row.from < row.to)
                    .map(|row| {
                        let old = host.text().substring(row.from, row.to);
                        Change {
                            start: row.from,
                            inserted: replaced(&old, with),
                            removed: old,
                        }
                    })
                    .collect();
                let extent = super::Extent {
                    kind: VisualKind::Block,
                    lines: block.last - block.first + 1,
                    cells: if to_end {
                        usize::MAX
                    } else {
                        block.right - block.left + 1
                    },
                };
                self.end_visual(host);
                if self.edit(host, changes, top.from, block.first) {
                    self.remember(shared, command, Some(extent));
                }
                self.settle(host, top.from);
                return;
            }
            let range = operator::visual_region(host, visual.anchor, visual.cursor, linewise, true);
            let text = host.text();
            let (from, to) = match range.region {
                Region::Chars { from, to } => (from, to),
                Region::Lines { first, last } => (text.line_start(first), text.line_end(last)),
            };
            if with == '\r' || !host.fits(from, to) {
                self.error(host, Message::Unsupported);
                return;
            }
            let old = text.substring(from, to);
            let new = replaced(&old, with);
            let extent = super::operate::extent_of(host, visual, linewise, to_end);
            self.end_visual(host);
            let change = Change {
                start: from,
                removed: old,
                inserted: new,
            };
            if self.edit(host, vec![change], range.start, range.first) {
                self.remember(shared, command, Some(extent));
            }
            self.settle(host, range.start);
            return;
        }

        let text = host.text();
        let count = times(command);
        // 光标起必须有「次数」个字符，否则不替换。
        let mut end = cursor;
        for _ in 0..count {
            match text::next_in_line(text, end) {
                Some(next) => end = next,
                None => return,
            }
        }
        let (inserted, after) = if with == '\r' {
            // 「次数」个字符换成一个换行。
            (host.line_ending().to_owned(), None)
        } else {
            let inserted: String = std::iter::repeat_n(with, count).collect();
            (inserted, Some(cursor + count - 1))
        };
        let line = text::line_of(text, cursor);
        let ending = inserted.chars().count();
        let change = Change {
            start: cursor,
            removed: text.substring(cursor, end),
            inserted,
        };
        if self.edit(host, vec![change], cursor, line) {
            self.remember(shared, command, None);
            // 替换用的字符同时记为最近插入的文本。
            let typed = match with {
                '\r' => "\n".to_owned(),
                // 制表符按字面插入时带有前导的控制字符。
                '\t' => "\u{16}\t".to_owned(),
                other => other.to_string(),
            };
            shared.registers.set_special('.', typed);
        }
        self.settle(host, after.unwrap_or(cursor + ending));
    }

    /// `~`：切换光标起「次数」个字符的大小写，光标移到其后。
    pub(super) fn toggle_case_char(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
    ) {
        if self.blocked(host) {
            return;
        }
        let text = host.text();
        let cursor = self.caret(host);
        let mut end = cursor;
        for _ in 0..times(command).min(crate::motion::MAX_STEPS) {
            match text::next_in_line(text, end) {
                Some(next) => end = next,
                None => break,
            }
        }
        if end == cursor {
            return;
        }
        let old = text.substring(cursor, end);
        let new = operator::recased(&old, Operator::ToggleCase);
        let line = text::line_of(text, cursor);
        let changes = if old == new {
            Vec::new()
        } else {
            vec![Change {
                start: cursor,
                removed: old,
                inserted: new,
            }]
        };
        if self.edit(host, changes, cursor, line) {
            self.remember(shared, command, None);
        }
        self.settle(host, end);
    }

    /// `J`、`gJ`：合并光标所在行起的「次数」行，至少两行。
    pub(super) fn join_lines(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        spaces: bool,
    ) {
        if self.blocked(host) {
            return;
        }
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let count = times(command).max(2);
        let available = text::last_line(text) - line + 1;
        // 最后一行上不能合并：次数大于 2 时光标移到行首。次数超出时合并到最后一行。
        if count > available && (count <= 2 || available < 2) {
            if count > 2 {
                self.settle(host, text.line_start(line));
            }
            return;
        }
        if self.join_at(host, line, count.min(available), spaces, cursor) {
            self.remember(shared, command, None);
        }
    }

    /// 粘贴寄存器的内容；可视模式下替换选区。
    pub(super) fn paste(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        action: Action,
    ) {
        if self.blocked(host) {
            return;
        }
        let Some(content) = Self::read_register(shared, host, command.register) else {
            self.error(
                host,
                Message::EmptyRegister(command.register.unwrap_or('"')),
            );
            // 可视模式下寄存器为空时，选区仍被删除。
            if self.visual.is_some() {
                let command = Command {
                    register: None,
                    ..command
                };
                let target = crate::parser::Target::Selection;
                self.operate(shared, host, command, Operator::Delete, target, None);
            }
            return;
        };
        // 每次重复至少占一个换行符，空的内容同样受次数的限制。
        let count = times(command);
        if (content.text.len() + 1).saturating_mul(count) > MAX_OUTPUT {
            self.error(host, Message::TooLarge);
            return;
        }
        let ending = host.line_ending().to_owned();
        let piece = content.text.replace('\n', &ending);
        let before = matches!(action, Action::PasteBefore | Action::PasteBeforeMove);
        let stay_after = matches!(action, Action::PasteAfterMove | Action::PasteBeforeMove);

        if let Some(visual) = self.visual {
            self.paste_over(
                shared,
                host,
                command,
                &content,
                visual,
                action == Action::PasteBefore,
            );
            return;
        }

        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        match content.kind {
            Kind::Blockwise(width) => {
                let Some((first, after)) = self.paste_block(host, &content, width, count, before)
                else {
                    return;
                };
                self.settle(host, if stay_after { after } else { first });
            }
            Kind::Charwise => {
                let at = if before {
                    cursor
                } else {
                    text::next_in_line(text, cursor).unwrap_or(cursor)
                };
                let inserted = piece.repeat(count);
                let length = inserted.chars().count();
                let multiline = content.text.contains('\n');
                let change = Change {
                    start: at,
                    removed: String::new(),
                    inserted,
                };
                if !self.edit(host, vec![change], cursor, line) {
                    return;
                }
                let pos = if stay_after {
                    at + length
                } else if multiline || length == 0 {
                    at
                } else {
                    winc_core::movement::prev_grapheme(host.text(), at + length)
                };
                self.settle(host, pos);
            }
            Kind::Linewise => {
                let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();
                if action == Action::PasteIndented {
                    self.match_indent(host, &mut lines, line);
                }
                let block = lines.join(&ending);
                let block = vec![block; count].join(&ending);
                let added = lines.len() * count;
                let (_, fold_end) = host
                    .fold(line)
                    .filter(|_| host.has_folds())
                    .unwrap_or((line, line));
                let (start, inserted, first) = if before {
                    (text.line_start(line), format!("{block}{ending}"), line)
                } else {
                    (
                        text.line_end(fold_end),
                        format!("{ending}{block}"),
                        fold_end + 1,
                    )
                };
                let change = Change {
                    start,
                    removed: String::new(),
                    inserted,
                };
                if !self.edit(host, vec![change], cursor, first) {
                    return;
                }
                let (from, to) = {
                    let text = host.text();
                    let last = (first + added - 1).min(text::last_line(text));
                    (text.line_start(first), text::last_char(text, last))
                };
                host.set_mark('[', from);
                host.set_mark(']', to);
                let text = host.text();
                let pos = if stay_after {
                    // 光标放在粘贴内容之后一行的行首；粘贴内容在末尾时放在其最后一行。
                    text.line_start((first + added).min(text::last_line(text)))
                } else {
                    text::first_non_blank(text, first)
                };
                self.settle(host, pos);
            }
        }
        self.remember(shared, command, None);
    }

    /// 把 `lines` 各行的缩进调整到第 `line` 行的缩进：第一行与之对齐，其余各行保持相对关系。
    fn match_indent(&self, host: &impl Host, lines: &mut [String], line: usize) {
        let tab_width = host.tab_width().max(1);
        let cells = |text: &str| {
            let mut cell = 0;
            for c in text.chars().take_while(|c| text::is_blank(*c)) {
                cell += winc_core::text::char_cells(c, cell, tab_width);
            }
            cell
        };
        let target = operator::indent_cells(host, line);
        let mut difference: Option<isize> = None;
        for text in lines.iter_mut().filter(|text| !text.is_empty()) {
            let current = isize::try_from(cells(text)).unwrap_or(isize::MAX);
            let wanted = match difference {
                None => {
                    let target = isize::try_from(target).unwrap_or(isize::MAX);
                    difference = Some(target - current);
                    target
                }
                Some(difference) => (current + difference).max(0),
            };
            let indent =
                operator::indent_text(wanted.unsigned_abs(), host.indentation(), tab_width);
            let body = text.trim_start_matches([' ', '\t']);
            *text = format!("{indent}{body}");
        }
    }

    /// 可视模式下的粘贴：以寄存器的内容替换选区，被替换的文本写入寄存器（`P` 不写入）。
    fn paste_over(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        content: &Content,
        visual: super::Visual,
        keep_registers: bool,
    ) {
        // 按块的选区按其覆盖的整行处理。
        let linewise = visual.kind != VisualKind::Char;
        let range = operator::visual_region(host, visual.anchor, visual.cursor, linewise, false);
        let text = host.text();
        let (from, to) = match range.region {
            Region::Chars { from, to } => (from, to),
            Region::Lines { first, last } => (text.line_start(first), text.line_end(last)),
        };
        if !host.fits(from, to) {
            self.error(host, Message::TooLarge);
            return;
        }
        let replaced = content_of(host, range.region);
        let ending = host.line_ending().to_owned();
        let count = times(command);
        let piece = content.text.replace('\n', &ending);
        // 选区与寄存器的类型不同时，寄存器的内容单独成行。
        let (inserted, cursor) = match (linewise, content.kind) {
            (false, Kind::Charwise | Kind::Blockwise(_)) => {
                let inserted = piece.repeat(count);
                let length = inserted.chars().count();
                let cursor = if content.text.contains('\n') || length == 0 {
                    from
                } else {
                    from + length - 1
                };
                (inserted, Some(cursor))
            }
            (false, Kind::Linewise) => {
                let block = vec![piece; count].join(&ending);
                (format!("{ending}{block}{ending}"), None)
            }
            (true, _) => (vec![piece; count].join(&ending), None),
        };
        let change = Change {
            start: from,
            removed: text.substring(from, to),
            inserted,
        };
        self.end_visual(host);
        if !self.edit(host, vec![change], range.start, range.first) {
            return;
        }
        if !keep_registers {
            let numbered = replaced.kind != Kind::Charwise;
            let source = Source::Delete { numbered };
            Self::write_register(shared, host, None, replaced, source);
        }
        let text = host.text();
        let pos = match cursor {
            Some(pos) => winc_core::movement::snap_to_grapheme(text, pos),
            None if linewise => text::first_non_blank(text, range.first),
            None => text::first_non_blank(text, range.first + 1),
        };
        self.settle(host, pos);
        self.remember(shared, command, None);
    }
}

/// 把 `text` 中换行符之外的每个字素替换为 `with`。
fn replaced(text: &str, with: char) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '\n' || c == '\r' {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        out.push(with);
        rest = &rest[c.len_utf8()..];
        // 组合字符与前一个字符合为一个字素，一并替换。
        while let Some(next) = rest.chars().next() {
            if !is_combining(next) {
                break;
            }
            rest = &rest[next.len_utf8()..];
        }
    }
    out
}

/// 是否为组合用字符（常见的组合附加符号区块、变体选择符与零宽连接符）。
fn is_combining(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F | 0x200D
    )
}
