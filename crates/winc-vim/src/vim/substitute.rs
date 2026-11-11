//! 替换：`:s`、`:&`、`&`、`g&`，以及带 `c` 标志时的逐处确认。
//!
//! 匹配在修改之前的文本上一次找全，替换之后的文本一次应用；逐处确认时每确认一处应用一处，
//! 其后各处的位置按已发生的长度变化平移，整次替换仍是一个撤销单元。

use std::time::{Duration, Instant};

use winc_core::{Change, Lines, Selection};

use super::ex::Span;
use super::{Failed, Shared, Vim};
use crate::ex;
use crate::host::{Host, Message, Query};
use crate::key::{Code, Key, Named};
use crate::operator;
use crate::pattern;
use crate::text;

/// 一次替换查找匹配的时间上限。
///
/// 取自调参：十万行的文件上逐行查找约需 0.1 秒，上限留出一个数量级的余量。
const FIND_BUDGET: Duration = Duration::from_secs(2);

/// 替换的标志。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Flags {
    /// 是否替换一行中的每一处匹配。
    global: bool,
    /// 是否逐处确认。
    confirm: bool,
    /// 是否忽略大小写；`None` 表示按选项。
    ignore_case: Option<bool>,
    /// 是否只统计匹配，不替换。
    count_only: bool,
    /// 找不到匹配时是否不提示。
    quiet: bool,
    /// 重复替换时是否改用上一次查找的模式。
    searched: bool,
}

/// 上一次替换。
#[derive(Debug, Clone)]
pub(super) struct Substitution {
    /// 模式。
    pattern: String,
    /// 替换串，其中的 `~` 已展开。
    replacement: String,
    /// 标志。
    flags: Flags,
}

/// 一处匹配及其替换之后的文本。
#[derive(Debug, Clone)]
struct Hit {
    /// 起点。
    start: usize,
    /// 终点（不含）。
    end: usize,
    /// 替换之后的文本。
    text: String,
    /// 匹配是否跨行：替换使之后的行并入匹配所在的行。
    joins: bool,
}

/// 进行中的逐处确认。
#[derive(Debug)]
pub(super) struct Confirm {
    /// 尚未处理的匹配，位置是修改之前的文本中的位置。
    hits: Vec<Hit>,
    /// 当前询问的匹配在 `hits` 中的下标。
    index: usize,
    /// 已应用的替换使其后的位置平移的字符数。
    shift: isize,
    /// 已替换的处数。
    done: usize,
    /// 最近一次询问的匹配在当前文本中的起点。
    cursor: usize,
}

/// 把 `:s` 的参数分成模式、替换串与其后的标志；没有写模式时模式为 `None`。
fn split(args: &str) -> (Option<(String, String)>, &str) {
    let Some(separator) = args.chars().next() else {
        return (None, args);
    };
    if separator.is_alphanumeric() || matches!(separator, '\\' | '"' | '|' | '&' | ' ') {
        return (None, args);
    }
    let (pattern, rest) = ex::delimited(&args[separator.len_utf8()..], separator);
    // 替换串：转义的分隔符去掉反斜杠，其余的转义留待展开时处理。
    let mut replacement = String::new();
    let mut chars = rest.char_indices();
    let mut flags = "";
    while let Some((at, c)) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some((_, next)) if next == separator => replacement.push(next),
                Some((_, next)) => {
                    replacement.push(c);
                    replacement.push(next);
                }
                None => replacement.push(c),
            }
        } else if c == separator {
            flags = &rest[at + c.len_utf8()..];
            break;
        } else {
            replacement.push(c);
        }
    }
    (Some((pattern, replacement)), flags)
}

/// 把替换串中未转义的 `~` 展开成上一次的替换串 `previous`。
fn with_previous(replacement: &str, previous: &str) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('~') => out.push('~'),
                Some(next) => {
                    out.push(c);
                    out.push(next);
                }
                None => out.push(c),
            },
            '~' => out.push_str(previous),
            c => out.push(c),
        }
    }
    out
}

/// 大小写转换的状态。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    /// 不转换。
    Keep,
    /// 转成大写。
    Upper,
    /// 转成小写。
    Lower,
}

/// 按替换串 `replacement` 展开一处匹配：`groups` 是整个匹配与各分组的文本，`ending` 是换行符。
fn expand(replacement: &str, groups: &[Option<String>], ending: &str) -> String {
    let mut out = String::new();
    // 只作用于下一个字符的转换，与持续到 `\e` 的转换。
    let mut once = Case::Keep;
    let mut lasting = Case::Keep;
    let push = |out: &mut String, piece: &str, once: &mut Case, lasting: Case| {
        for c in piece.chars() {
            let case = if *once == Case::Keep { lasting } else { *once };
            *once = Case::Keep;
            match case {
                Case::Keep => out.push(c),
                Case::Upper => out.extend(c.to_uppercase()),
                Case::Lower => out.extend(c.to_lowercase()),
            }
        }
    };
    let group = |index: usize| groups.get(index).and_then(Option::as_deref).unwrap_or("");
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => push(&mut out, group(0), &mut once, lasting),
            '\r' => out.push_str(ending),
            '\\' => match chars.next() {
                Some(digit @ '0'..='9') => {
                    let index = digit.to_digit(10).unwrap_or(0) as usize;
                    push(&mut out, group(index), &mut once, lasting);
                }
                Some('n' | 'r') => out.push_str(ending),
                Some('t') => out.push('\t'),
                Some('u') => once = Case::Upper,
                Some('l') => once = Case::Lower,
                Some('U') => lasting = Case::Upper,
                Some('L') => lasting = Case::Lower,
                Some('e' | 'E') => {
                    once = Case::Keep;
                    lasting = Case::Keep;
                }
                Some(other) => push(&mut out, other.encode_utf8(&mut [0; 4]), &mut once, lasting),
                None => out.push('\\'),
            },
            c => push(&mut out, c.encode_utf8(&mut [0; 4]), &mut once, lasting),
        }
    }
    out
}

impl Shared {
    /// 解析标志与其后的行数、下一条命令；写法有误时返回 `None`。
    fn substitute_flags<'a>(&self, text: &'a str) -> Option<(Flags, Option<usize>, &'a str)> {
        let mut flags = Flags {
            global: self.options.gdefault,
            ..Flags::default()
        };
        let mut rest = text;
        if let Some(after) = rest.strip_prefix('&') {
            flags = self.substitute.as_ref().map(|last| last.flags)?;
            rest = after;
        }
        loop {
            let mut chars = rest.chars();
            match chars.next() {
                Some('g') => flags.global = !flags.global,
                Some('c') => flags.confirm = !flags.confirm,
                Some('i') => flags.ignore_case = Some(true),
                Some('I') => flags.ignore_case = Some(false),
                Some('n') => flags.count_only = true,
                Some('e') => flags.quiet = true,
                Some('r') => flags.searched = true,
                Some('p' | '#' | 'l') => {}
                _ => break,
            }
            rest = chars.as_str();
        }
        let rest = rest.trim_start_matches([' ', '\t']);
        let (count, rest) = match ex::number(rest) {
            Some((0, _)) => return None,
            Some((count, rest)) => (Some(count), rest.trim_start_matches([' ', '\t'])),
            None => (None, rest),
        };
        match rest.chars().next() {
            None => Some((flags, count, "")),
            Some('|') => Some((flags, count, &rest[1..])),
            Some(_) => None,
        }
    }
}

impl Vim {
    /// 返回逐处确认中当前这一处替换之后的文本；不在确认中时返回 `None`。
    pub fn confirming(&self) -> Option<&str> {
        let confirm = self.confirm.as_ref()?;
        confirm.hits.get(confirm.index).map(|hit| hit.text.as_str())
    }

    /// `:s`、`:&`：在范围 `span` 上替换；`repeat` 为真时参数只有标志。成功时返回下一条命令。
    pub(super) fn ex_substitute(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        span: Span,
        repeat: bool,
        args: &str,
    ) -> Result<Option<String>, Failed> {
        let (given, tail) = if repeat { (None, args) } else { split(args) };
        if given.is_none() && shared.substitute.is_none() {
            return self.fail(host, Message::NoSubstitute);
        }
        let Some((flags, count, next)) = shared.substitute_flags(tail) else {
            return self.fail(host, Message::BadArgument);
        };
        self.ex_last = next.is_empty();
        let previous = shared.substitute.clone();
        let searched = || shared.search_pattern().map(str::to_owned);
        let (pattern, replacement) = match (given, &previous) {
            (Some((pattern, replacement)), _) => {
                let last = previous
                    .as_ref()
                    .map_or("", |last| last.replacement.as_str());
                let pattern = if pattern.is_empty() {
                    searched()
                } else {
                    Some(pattern)
                };
                (pattern, with_previous(&replacement, last))
            }
            (None, Some(last)) => {
                let pattern = if flags.searched {
                    searched()
                } else {
                    Some(last.pattern.clone())
                };
                (pattern, last.replacement.clone())
            }
            (None, None) => return self.fail(host, Message::NoSubstitute),
        };
        let Some(pattern) = pattern else {
            return self.fail(host, Message::NoSearch);
        };
        if replacement.starts_with("\\=") {
            return self.fail(host, Message::Unsupported);
        }
        let ignore_case = flags.ignore_case.unwrap_or(shared.ignore_case);
        let smart = shared.smart_case && flags.ignore_case.is_none();
        let query = match pattern::translate(&pattern, ignore_case, smart) {
            Ok(query) => query,
            Err(at) => return self.fail(host, Message::BadPattern(at)),
        };
        shared.set_search_pattern(pattern.clone());
        shared.substitute = Some(Substitution {
            pattern,
            replacement: replacement.clone(),
            flags,
        });

        let (first, last) = span.lines();
        let (first, last) = match count {
            Some(count) => {
                let end = last.saturating_add(count - 1);
                (last, end.min(text::last_line(host.text())))
            }
            None => (first, last),
        };
        let Some((hits, lines)) = find_hits(host, &query, &replacement, first, last, flags.global)
        else {
            return self.fail(host, Message::Interrupted);
        };
        let next = (!next.is_empty()).then(|| next.to_owned());
        if hits.is_empty() {
            // 逐行执行的命令中找不到匹配不算出错。
            if flags.quiet || self.in_global {
                return Ok(next);
            }
            return self.fail(host, Message::NotFound);
        }
        if flags.count_only {
            host.message(Message::Matched {
                count: hits.len(),
                lines,
            });
            let text = host.text();
            let pos = text::first_non_blank(text, text::line_of(text, self.caret(host)));
            self.settle(host, pos);
            return Ok(next);
        }
        // 逐行执行的命令中不逐处确认。
        if flags.confirm && !self.in_global {
            self.confirm_end(host);
            host.begin_group();
            self.confirm = Some(Confirm {
                hits,
                index: 0,
                shift: 0,
                done: 0,
                cursor: 0,
            });
            self.confirm_show(host);
            return Ok(None);
        }
        let count = hits.len();
        let joins = hits.last().is_some_and(|hit| hit.joins);
        let Some((start, end)) = self.replace_hits(host, &hits, 0) else {
            return Err(Failed);
        };
        // 最后一处把之后的行并入时光标在连接处，否则在所在行的第一个非空白字符上。
        let text = host.text();
        let pos = if joins {
            winc_core::movement::snap_to_grapheme(text, start.min(text.end_pos()))
        } else {
            text::first_non_blank(text, text::line_of(text, end))
        };
        self.settle(host, pos);
        if count > 2 && !self.in_global {
            host.message(Message::Substituted { count, lines });
        }
        Ok(next)
    }

    /// 应用 `hits` 的替换，各处的位置先平移 `shift`；返回最后一处替换之后的文本的起点与终点。
    fn replace_hits(
        &mut self,
        host: &mut impl Host,
        hits: &[Hit],
        shift: isize,
    ) -> Option<(usize, usize)> {
        let text = host.text();
        let at = |pos: usize| pos.saturating_add_signed(shift);
        let mut grown: isize = 0;
        let mut changes = Vec::with_capacity(hits.len());
        for hit in hits {
            let (start, end) = (at(hit.start), at(hit.end));
            let removed = text.substring(start, end);
            let inserted = isize::try_from(hit.text.chars().count()).unwrap_or(0);
            grown += inserted - isize::try_from(end - start).unwrap_or(0);
            changes.push(Change {
                start,
                removed,
                inserted: hit.text.clone(),
            });
        }
        changes.reverse();
        let last = hits.last()?;
        let first_line = text::line_of(text, at(hits.first()?.start));
        // 撤销之后光标在第一处替换所在行的行首。
        let cursor = text.line_start(first_line);
        if !self.edit(host, changes, cursor, first_line) {
            return None;
        }
        let end = at(last.end).saturating_add_signed(grown);
        Some((end - last.text.chars().count(), end))
    }

    /// 把当前询问的匹配显示为选区。
    fn confirm_show(&mut self, host: &mut impl Host) {
        let Some(confirm) = &mut self.confirm else {
            return;
        };
        let Some(hit) = confirm.hits.get(confirm.index) else {
            return;
        };
        // 已应用的替换可以改变字素的划分：选区的两端收拢到字素的边界上。
        let text = host.text();
        let at = |pos: usize| {
            let pos = pos.saturating_add_signed(confirm.shift).min(text.end_pos());
            winc_core::movement::snap_to_grapheme(text, pos)
        };
        let selection = Selection::new(at(hit.start), at(hit.end));
        confirm.cursor = selection.anchor;
        self.place(host, selection);
    }

    /// 结束逐处确认：关闭撤销分组，光标留在最后询问的一处。
    pub(super) fn confirm_end(&mut self, host: &mut impl Host) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        host.end_group();
        let text = host.text();
        let pos = winc_core::movement::snap_to_grapheme(text, confirm.cursor.min(text.end_pos()));
        self.settle(host, pos);
        if confirm.done > 2 {
            host.message(Message::Substituted {
                count: confirm.done,
                lines: 0,
            });
        }
    }

    /// 逐处确认期间处理按键：`y` 替换，`n` 跳过，`a` 替换余下的全部，`l` 替换后结束，
    /// `q` 与 `Esc` 结束。
    pub(super) fn confirm_key(&mut self, host: &mut impl Host, key: Key) {
        let Some(confirm) = &self.confirm else {
            return;
        };
        let answer = match key.code() {
            Code::Named(Named::Esc) => 'q',
            Code::Char('c') if key.as_char().is_none() => 'q',
            _ => key.as_char().unwrap_or(' '),
        };
        let (index, shift) = (confirm.index, confirm.shift);
        let chosen = match answer {
            'y' | 'l' => index..index + 1,
            'a' => index..confirm.hits.len(),
            'n' => index..index,
            'q' => return self.confirm_end(host),
            _ => return,
        };
        let hits = confirm.hits[chosen.clone()].to_vec();
        let grown: isize = (hits.iter())
            .map(|hit| {
                let inserted = isize::try_from(hit.text.chars().count()).unwrap_or(0);
                inserted - isize::try_from(hit.end - hit.start).unwrap_or(0)
            })
            .sum();
        if !hits.is_empty() && self.replace_hits(host, &hits, shift).is_none() {
            return self.confirm_end(host);
        }
        let Some(confirm) = &mut self.confirm else {
            return;
        };
        confirm.shift += grown;
        confirm.done += hits.len();
        confirm.index = chosen.end.max(index + 1);
        if matches!(answer, 'l' | 'a') || confirm.index >= confirm.hits.len() {
            return self.confirm_end(host);
        }
        self.confirm_show(host);
    }

    /// `&`、`g&`：重复上一次替换；`all` 为真时作用于全部的行并沿用标志。
    pub(super) fn repeat_substitute(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        all: bool,
    ) {
        // `&` 记为上一次执行的命令行，`g&` 不记。
        if all {
            self.ex_line(shared, host, "%s//~/&");
        } else {
            self.command_line(shared, host, "s");
        }
    }
}

/// 模式是否可以匹配换行。
fn spans_lines(query: &Query) -> bool {
    query.regex.contains("\\n") || query.regex.contains("(?s")
}

/// 在第 `first` 至 `last` 行上找出 `query` 的匹配，返回各处匹配与有匹配的行数；超过时间上限时
/// 返回 `None`。
fn find_hits<H: Host>(
    host: &mut H,
    query: &Query,
    replacement: &str,
    first: usize,
    last: usize,
    global: bool,
) -> Option<(Vec<Hit>, usize)> {
    let started = Instant::now();
    let ending = host.line_ending().to_owned();
    let mut hits = Vec::new();
    let mut lines = 0;
    let mut line = first;
    while line <= last {
        if line % 256 == 255 && started.elapsed() > FIND_BUDGET {
            return None;
        }
        // 跨行的匹配把之后的行并入当前行：`reach` 是当前行延伸到的末行。
        let mut reach = line;
        let mut from = host.text().line_start(line);
        let mut previous = None;
        let before = hits.len();
        loop {
            let limit = host.text().line_end(reach);
            if from > limit {
                break;
            }
            let Some(groups) = host.find_groups(query, from) else {
                break;
            };
            let Some(Some((start, end))) = groups.first().copied() else {
                break;
            };
            let text = host.text();
            let boundary = |pos: usize| winc_core::movement::snap_to_grapheme(text, pos) == pos;
            let whole = boundary(start) && (!query.ends_literal || boundary(end));
            // 紧接在上一处匹配之后的空匹配，以及拆开字素的匹配不算。
            if !whole || (start == end && previous == Some(start)) {
                // 跳过之后已到行尾时不再查找。
                if start + 1 >= limit && !spans_lines(query) {
                    break;
                }
                if start >= limit {
                    break;
                }
                from = start + 1;
                previous = None;
                continue;
            }
            let pieces: Vec<Option<String>> = (groups.iter())
                .map(|group| group.map(|(a, b)| operator::normalized(text.substring(a, b))))
                .collect();
            // 最后一行之后的换行符不属于匹配：没有可以并入的行。
            let stop = text.line_end(text::last_line(text)).max(start);
            let (end, clamped) = (end.min(stop), end > stop);
            let joins = text::line_of(text, end) > text::line_of(text, start);
            let replaced = expand(replacement, &pieces, &ending);
            if clamped && start == end && replaced.is_empty() {
                break;
            }
            hits.push(Hit {
                start,
                end,
                text: replaced,
                joins,
            });
            reach = reach.max(text::line_of(text, end));
            // 跨行的匹配之后在并入的行上继续查找，除非已越过范围的末行。
            if (joins && reach > last) || (!joins && !global) {
                break;
            }
            previous = (end > start).then_some(end);
            from = if end > start { end } else { start + 1 };
            // 已到行尾时不再查找；可以匹配换行的模式除外。
            let line_end = text.line_end(reach);
            if from > line_end || (from == line_end && !spans_lines(query)) {
                break;
            }
        }
        lines += usize::from(hits.len() > before);
        line = reach + 1;
    }
    Some((hits, lines))
}
