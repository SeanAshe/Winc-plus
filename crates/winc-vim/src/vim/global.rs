//! 逐行执行的命令：`:g`、`:v` 与 `:normal`。
//!
//! `:g` 先标记有匹配的行再逐行执行，被标记的行由宿主跟踪，行号随编辑移动；`:normal` 按行号
//! 依次执行。两者都受回放的步数与时间上限约束。

use winc_core::Lines;

use super::ex::{Span, line_range};
use super::macros::text_keys;
use super::{Failed, Shared, Vim};
use crate::command::Mode;
use crate::ex::{self, Cmd};
use crate::host::{Host, Message};
use crate::key::{Key, Named};
use crate::operator;
use crate::pattern;
use crate::register::Source;
use crate::text;

impl Vim {
    /// `:normal`：回放参数中的按键；有范围时在每一行的行首各执行一次。`remap` 为假时按键
    /// 不经过用户的映射。
    pub(super) fn ex_normal(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        given: Option<Span>,
        remap: bool,
        args: &str,
    ) -> Result<(), Failed> {
        let keys = text_keys(args);
        if keys.is_empty() {
            return self.fail(host, Message::BadArgument);
        }
        let Some(span) = given else {
            self.normal_once(shared, host, &keys, remap);
            return Ok(());
        };
        let (first, last) = span.lines();
        for line in first..=last {
            if !self.spend(host) {
                break;
            }
            // 行号超出文本时在最后一行上执行。
            let text = host.text();
            let pos = text.line_start(line.min(text::last_line(text)));
            self.settle(host, pos);
            self.failed = false;
            self.normal_once(shared, host, &keys, remap);
        }
        self.failed = false;
        Ok(())
    }

    /// 回放 `keys`，结束时未完成的命令与插入按 `Esc` 结束。
    fn normal_once(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        keys: &[Key],
        remap: bool,
    ) {
        self.play_now(shared, host, keys, remap);
        if self.prompt().is_some() {
            self.line.clear();
            self.prompt_cancel(host);
        }
        if self.confirm.is_some() {
            self.confirm_end(host);
        }
        self.parser.reset();
        if self.mode == Mode::Insert || self.oneshot.is_some() {
            self.key_raw(shared, host, Key::named(Named::Esc));
        }
    }

    /// `:g`、`:v`：在范围中有匹配（`inverse` 为真时没有匹配）的每一行上执行参数中的命令。
    pub(super) fn ex_global(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        given: Option<Span>,
        inverse: bool,
        args: &str,
    ) -> Result<(), Failed> {
        let mut chars = args.chars();
        let separator = match chars.next() {
            Some(c) if !c.is_alphanumeric() && !matches!(c, '\\' | '"' | '|') => c,
            _ => return self.fail(host, Message::BadArgument),
        };
        // 逐行执行的命令中不能再逐行执行。
        if self.in_global {
            return self.fail(host, Message::BadCommand);
        }
        let (pattern, command) = ex::delimited(chars.as_str(), separator);
        let pattern = if pattern.is_empty() {
            shared.search_pattern().map(str::to_owned)
        } else {
            Some(pattern)
        };
        let Some(pattern) = pattern else {
            return self.fail(host, Message::NoSearch);
        };
        let query = match pattern::translate(&pattern, shared.ignore_case, shared.smart_case) {
            Ok(query) => query,
            Err(at) => return self.fail(host, Message::BadPattern(at)),
        };
        shared.set_search_pattern(pattern);

        let text = host.text();
        let (first, last) = match given {
            Some(span) => span.lines(),
            None => (0, text::last_line(text)),
        };
        // 有匹配的行，从小到大。
        let mut matched = Vec::new();
        let mut pos = host.text().line_start(first);
        while let Some((start, _)) = host.find(&query, pos, true) {
            let text = host.text();
            let line = text::line_of(text, start);
            if line > last {
                break;
            }
            matched.push(line);
            if line >= text::last_line(text) {
                break;
            }
            pos = text.line_start(line + 1);
        }
        let lines: Vec<usize> = if inverse {
            let mut skip = matched.iter().copied().peekable();
            (first..=last)
                .filter(|line| skip.next_if_eq(line).is_none())
                .collect()
        } else {
            matched
        };
        if lines.is_empty() {
            return self.fail(host, Message::NotFound);
        }
        let command = command.trim();
        // 删除整行是最常见的用法，一次应用。
        let plain_delete = ex::parse(command).is_ok_and(|line| {
            line.cmd == Cmd::Delete && line.addresses.is_empty() && line.args.is_empty()
        });
        if plain_delete {
            return self.delete_lines(shared, host, &lines);
        }
        let text = host.text();
        host.track(lines.iter().map(|line| text.line_start(*line)).collect());
        self.in_global = true;
        while let Some(pos) = host.next_tracked() {
            if !self.spend(host) {
                break;
            }
            self.settle(host, pos.min(host.text().end_pos()));
            self.failed = false;
            // 没有写命令时只移动光标。
            if !command.is_empty() {
                self.ex_run(shared, host, command);
            }
        }
        host.track(Vec::new());
        self.in_global = false;
        self.failed = false;
        Ok(())
    }

    /// 删除 `lines`（从小到大）中的各行：寄存器的结果与逐行删除相同。
    fn delete_lines(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        lines: &[usize],
    ) -> Result<(), Failed> {
        if self.blocked(host) {
            return Err(Failed);
        }
        // 编号寄存器只保留最后九次删除。
        for line in &lines[lines.len().saturating_sub(9)..] {
            let content = super::operate::content_of(host, line_range(host, *line, *line).region);
            let source = Source::Delete { numbered: true };
            if !Self::write_register(shared, host, None, content, source) {
                return Err(Failed);
            }
        }
        // 相邻的行合并成一处修改，从后往前排列。
        let text = host.text();
        let mut changes = Vec::new();
        let mut end = lines.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && lines[start - 1] + 1 == lines[start] {
                start -= 1;
            }
            let (from, to) = operator::lines_span(text, lines[start], lines[end - 1]);
            changes.push(operator::removal(text, from, to));
            end = start;
        }
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
            return Ok(());
        };
        let cursor = text::first_non_blank(text, *first);
        if !self.edit(host, changes, cursor, *first) {
            return Err(Failed);
        }
        // 光标在最后一处删除的位置。
        let text = host.text();
        let line = (last + 1 - lines.len()).min(text::last_line(text));
        let pos = text::first_non_blank(text, line);
        self.settle(host, pos);
        Ok(())
    }
}
