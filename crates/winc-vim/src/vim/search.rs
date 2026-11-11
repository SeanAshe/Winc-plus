//! 查找：`/` `?` 的输入、`n` `N`、`*` `#`、`gn` `gN`，以及查找之后的偏移。
//!
//! 模式的输入由宿主的输入框完成：`/`、`?` 解析成命令之后状态机进入等待输入的状态
//! （[`Vim::prompt`]），宿主在输入过程中调用 [`Vim::prompt_preview`]，结束时调用
//! [`Vim::prompt_submit`] 或 [`Vim::prompt_cancel`]。匹配由宿主的正则引擎查找。

use winc_core::Lines;

use super::{Shared, Vim, Visual, VisualKind};
use crate::command::{Mode, Motion};
use crate::host::{Host, Message, Query};
use crate::motion::{Moved, Span, WantUpdate};
use crate::parser::{Body, Command, Movement, Target};
use crate::pattern;
use crate::text::{self, Scan, Step};

/// 查找历史与命令行的历史各自保留的条数（条）。
///
/// 取自调参：与输入框的历史同一量级。
const MAX_HISTORY: usize = 100;

/// 一次查找中依次检查的匹配数上限（处）；超过时按找不到处理。
///
/// 取自调参：限制次数很大或同一行上匹配极多时的耗时。
const MAX_MATCHES: usize = 10_000;

/// 查找之后光标相对匹配的偏移。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Offset {
    /// 是否按行偏移：光标位于匹配所在行之后若干行的行首。
    line: bool,
    /// 是否相对匹配的终点偏移；为假时相对起点。
    end: bool,
    /// 偏移的行数或字符数。
    amount: isize,
}

/// 上一次查找。
#[derive(Debug, Clone)]
pub(super) struct Searched {
    /// 模式。
    pattern: String,
    /// 是否向后文查找。
    forward: bool,
    /// 偏移。
    offset: Offset,
    /// 是否不考虑「模式中有大写字母时区分大小写」的选项。
    plain_case: bool,
}

/// 状态机等待宿主提供的输入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// 查找的模式；`forward` 为真时向后文查找。
    Search {
        /// 是否向后文查找。
        forward: bool,
    },
    /// 命令行；预填的文本见 [`Vim::prompt_prefill`]。
    Command,
}

/// 等待输入期间保存的状态。
#[derive(Debug, Clone, Copy)]
pub(super) struct Pending {
    /// 输入完成之后执行的命令。
    command: Command,
    /// 是否向后文查找。
    forward: bool,
    /// 输入开始时光标的位置。
    origin: usize,
    /// 输入开始时的可视选区。
    visual: Option<Visual>,
}

/// 返回命令 `command` 中需要输入模式的查找动作的方向；没有时返回 `None`。
fn prompted(command: Command) -> Option<bool> {
    let motion = match command.body {
        Body::Motion(movement) => movement.motion,
        Body::Operator {
            target: Target::Motion(movement),
            ..
        } => movement.motion,
        _ => return None,
    };
    match motion {
        Motion::SearchForward => Some(true),
        Motion::SearchBackward => Some(false),
        _ => None,
    }
}

/// 把输入的文本分成模式与偏移：以未转义的分隔符 `separator` 为界。
fn split(text: &str, separator: char) -> (String, Option<Offset>) {
    let mut pattern = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            pattern.push(c);
            if let Some(next) = chars.next() {
                // 向前文查找时，转义的 `?` 在模式中不带反斜杠。
                if next == separator && separator == '?' {
                    pattern.pop();
                }
                pattern.push(next);
            }
        } else if c == separator {
            return (pattern, Some(offset(chars.as_str())));
        } else {
            pattern.push(c);
        }
    }
    (pattern, None)
}

/// 解析偏移的写法：`e`、`s`、`b` 加带符号的字符数，或带符号的行数。
fn offset(text: &str) -> Offset {
    let mut chars = text.chars().peekable();
    let mut offset = Offset::default();
    match chars.peek() {
        Some('e') => {
            offset.end = true;
            chars.next();
        }
        Some('s' | 'b') => {
            chars.next();
        }
        Some('+' | '-' | '0'..='9') => offset.line = true,
        _ => return offset,
    }
    let sign = match chars.peek() {
        Some('-') => {
            chars.next();
            Some(-1)
        }
        Some('+') => {
            chars.next();
            Some(1)
        }
        _ => None,
    };
    let digits: String = chars.take_while(char::is_ascii_digit).collect();
    let amount = digits.parse::<isize>().ok();
    offset.amount = match (sign, amount) {
        (Some(sign), Some(amount)) => sign * amount,
        (Some(sign), None) => sign,
        (None, Some(amount)) => amount,
        (None, None) => 0,
    };
    offset
}

/// 返回匹配 `found` 用于比较先后的位置：按终点比较时为最后一个字符，否则为起点，位于非空行
/// 行尾的起点算作该行的最后一个字符。
fn key<H: Host>(host: &H, found: (usize, usize), by_end: bool) -> usize {
    let text = host.text();
    if by_end {
        return found.0.max(found.1.saturating_sub(1));
    }
    let line = text::line_of(text, found.0);
    if found.0 == text.line_end(line) && text.line_len(line) > 0 {
        found.0 - 1
    } else {
        found.0
    }
}

/// 返回起点不小于 `from`（`forward` 为真）或小于 `from` 的最近一处匹配；起点不在字素边界上，
/// 或以字面字符结尾而终点不在字素边界上的匹配不算（组合字符属于其前的字符）。
fn find<H: Host>(
    host: &mut H,
    query: &Query,
    from: usize,
    forward: bool,
) -> Option<(usize, usize)> {
    let mut from = from;
    for _ in 0..MAX_MATCHES {
        let found = host.find(query, from, forward)?;
        let boundary = |pos: usize| winc_core::movement::snap_to_grapheme(host.text(), pos) == pos;
        if boundary(found.0) && (!query.ends_literal || boundary(found.1)) {
            return Some(found);
        }
        from = if forward { found.0 + 1 } else { found.0 };
    }
    None
}

/// 返回位置在 `at` 之后（`forward` 为真）或之前最近的一处匹配；`wrap` 为真时到达文本的一端
/// 之后从另一端继续。
fn step<H: Host>(
    host: &mut H,
    query: &Query,
    at: usize,
    forward: bool,
    by_end: bool,
    wrap: bool,
) -> Option<(usize, usize)> {
    let end = host.text().end_pos();
    if forward {
        let mut from = if by_end {
            let text = host.text();
            text.line_start(text::line_of(text, at))
        } else {
            at + 1
        };
        for _ in 0..MAX_MATCHES {
            let Some(found) = find(host, query, from, true) else {
                break;
            };
            if key(host, found, by_end) > at {
                return Some(found);
            }
            from = found.0 + 1;
        }
        return wrap.then(|| find(host, query, 0, true)).flatten();
    }
    if let Some(found) = find(host, query, at, false) {
        if !by_end || key(host, found, true) < at {
            return Some(found);
        }
        if let Some(previous) = find(host, query, found.0, false) {
            return Some(previous);
        }
    }
    wrap.then(|| find(host, query, end + 1, false)).flatten()
}

/// 返回从 `pos` 向后（`amount` 为正）或向前移动若干字符之后的位置，越过行尾。
fn shifted<T: Lines>(text: &T, pos: usize, amount: isize) -> usize {
    let mut scan = Scan::new(text, pos);
    for _ in 0..amount.unsigned_abs().min(MAX_MATCHES) {
        let step = if amount > 0 { scan.incl() } else { scan.decl() };
        if step == Step::Edge {
            break;
        }
    }
    scan.pos()
}

impl Shared {
    /// 把查找历史与命令行的历史换成 `searches` 与 `commands`（从旧到新），超过保留的条数时
    /// 留下最新的。
    pub fn restore_history(&mut self, searches: Vec<String>, commands: Vec<String>) {
        let latest = |mut entries: Vec<String>| {
            entries.retain(|entry| !entry.is_empty());
            entries.split_off(entries.len().saturating_sub(MAX_HISTORY))
        };
        self.history = latest(searches);
        self.commands = latest(commands);
    }

    /// 返回查找历史，从旧到新排列。
    pub fn search_history(&self) -> &[String] {
        &self.history
    }

    /// 把 `text` 记入查找历史；已有相同的一条时移到最后。
    fn remember_search(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.history.retain(|entry| entry != text);
        self.history.push(text.to_owned());
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }

    /// 返回上一次查找的模式。
    pub(super) fn search_pattern(&self) -> Option<&str> {
        self.search
            .as_ref()
            .map(|searched| searched.pattern.as_str())
    }

    /// 把上一次查找的模式换成 `pattern`，方向不变，偏移清除。
    pub(super) fn set_search_pattern(&mut self, pattern: String) {
        let forward = self.search.as_ref().is_none_or(|searched| searched.forward);
        self.set_search(pattern, forward, Offset::default(), false);
    }

    /// 返回命令行的历史，从旧到新排列。
    pub fn command_history(&self) -> &[String] {
        &self.commands
    }

    /// 把 `text` 记入命令行的历史；已有相同的一条时移到最后。
    fn remember_command(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.commands.retain(|entry| entry != text);
        self.commands.push(text.to_owned());
        if self.commands.len() > MAX_HISTORY {
            self.commands.remove(0);
        }
    }

    /// 把模式 `pattern` 记为上一次查找。
    fn set_search(&mut self, pattern: String, forward: bool, offset: Offset, plain_case: bool) {
        self.registers.set_special('/', pattern.clone());
        self.search = Some(Searched {
            pattern,
            forward,
            offset,
            plain_case,
        });
    }

    /// 转换上一次查找的模式；模式有误时提示并返回 `None`。
    fn query(&self, host: &mut impl Host) -> Option<(Query, Searched)> {
        let Some(searched) = self.search.clone() else {
            host.message(Message::NoSearch);
            return None;
        };
        let smart = self.smart_case && !searched.plain_case;
        match pattern::translate(&searched.pattern, self.ignore_case, smart) {
            Ok(query) => Some((query, searched)),
            Err(at) => {
                host.message(Message::BadPattern(at));
                None
            }
        }
    }
}

impl Vim {
    /// 返回状态机正在等待的输入；不在等待时返回 `None`。
    pub fn prompt(&self) -> Option<Prompt> {
        if self.cmdline {
            return Some(Prompt::Command);
        }
        self.prompt.map(|pending| Prompt::Search {
            forward: pending.forward,
        })
    }

    /// 返回光标处的词，供命令行的输入框插入；光标所在的行上没有词时返回 `None`。
    pub fn word_at_cursor(&self, host: &impl Host) -> Option<String> {
        let text = host.text();
        let (start, end) = word_at(text, self.caret(host))?;
        Some(text.substring(start, end))
    }

    /// 返回寄存器 `name` 的文本，供命令行的输入框插入；寄存器为空时返回 `None`。
    pub fn register_text(shared: &mut Shared, host: &mut impl Host, name: char) -> Option<String> {
        Self::read_register(shared, host, Some(name)).map(|content| content.text)
    }

    /// 返回等待命令行时预填的文本：可视模式下是选区的范围，带次数时是从当前行起的若干行。
    pub fn prompt_prefill(&self) -> &str {
        if self.cmdline { &self.line } else { "" }
    }

    /// 命令 `command` 需要先输入命令行或查找的模式时，进入等待输入的状态并返回真。
    pub(super) fn ask(&mut self, host: &mut impl Host, command: Command) -> bool {
        let is_command_line = matches!(
            command.body,
            Body::Action {
                action: crate::command::Action::CommandLine,
                ..
            }
        );
        if is_command_line {
            self.line = match (self.visual, command.count) {
                (Some(_), _) => "'<,'>".to_owned(),
                (None, Some(1)) => ".".to_owned(),
                (None, Some(count)) => format!(".,.+{}", count - 1),
                (None, None) => String::new(),
            };
            // 输入命令行时已离开可视模式。
            if let Some(visual) = self.visual {
                self.end_visual(host);
                self.settle(host, visual.cursor);
            }
            self.cmdline = true;
            return true;
        }
        let Some(forward) = prompted(command) else {
            return false;
        };
        self.line.clear();
        self.prompt = Some(Pending {
            command,
            forward,
            origin: self.caret(host),
            visual: self.visual,
        });
        true
    }

    /// 把光标放回输入开始时的位置。
    fn restore(&mut self, host: &mut impl Host, pending: Pending) {
        self.visual = pending.visual;
        self.show(host, pending.origin);
    }

    /// 输入过程中调用：把光标移到 `text` 的第一处匹配上；没有匹配时回到输入开始时的位置。
    pub fn prompt_preview(&mut self, shared: &Shared, host: &mut impl Host, text: &str) {
        let Some(pending) = self.prompt else {
            return;
        };
        if !shared.options.incsearch {
            return;
        }
        let separator = if pending.forward { '/' } else { '?' };
        let (pattern, _) = split(text, separator);
        let found = (!pattern.is_empty())
            .then(|| pattern::translate(&pattern, shared.ignore_case, shared.smart_case).ok())
            .flatten()
            .and_then(|query| {
                let wrap = shared.options.wrapscan;
                step(host, &query, pending.origin, pending.forward, false, wrap)
            });
        match found {
            Some((start, _)) => {
                let text = host.text();
                let pos = match &mut self.visual {
                    Some(visual) => {
                        visual.cursor = start;
                        start
                    }
                    None => text::on_char(text, start),
                };
                self.show(host, pos);
            }
            None => self.restore(host, pending),
        }
    }

    /// 放弃输入：光标回到输入开始时的位置，命令不执行。
    pub fn prompt_cancel(&mut self, host: &mut impl Host) {
        if self.prompt.is_some() || self.cmdline {
            self.record_line(None);
        }
        self.cmdline = false;
        self.line.clear();
        if let Some(pending) = self.prompt.take() {
            self.restore(host, pending);
        }
    }

    /// 放弃输入，光标留在当前的位置；输入框被其他操作关闭时调用。
    pub fn prompt_abandon(&mut self) {
        self.prompt = None;
        self.cmdline = false;
        self.line.clear();
    }

    /// 输入完成：以 `text` 为命令行或查找的模式，执行等待中的命令。
    ///
    /// 查找时 `text` 中未转义的 `/`（向前文查找时为 `?`）之后是偏移；模式为空时沿用上一次的
    /// 模式。
    pub fn prompt_submit(&mut self, shared: &mut Shared, host: &mut impl Host, text: &str) {
        self.begin_key(shared);
        self.record_line(Some(text));
        self.submit(shared, host, text);
        self.finish_key(shared, host);
    }

    /// 执行命令行 `text`，并记为上一次执行的命令行；以 `|` 分隔的命令没有执行到最后一条时
    /// 不记。
    pub(super) fn command_line(&mut self, shared: &mut Shared, host: &mut impl Host, text: &str) {
        self.ex_last = true;
        self.ex_line(shared, host, text);
        if !text.trim().is_empty() && self.ex_last {
            shared.last_command = Some(text.to_owned());
            shared.registers.set_special(':', text.to_owned());
        }
    }

    /// [`Vim::prompt_submit`] 的实现，不记录按键。
    pub(super) fn submit(&mut self, shared: &mut Shared, host: &mut impl Host, text: &str) {
        self.line.clear();
        if std::mem::take(&mut self.cmdline) {
            shared.remember_command(text);
            self.command_line(shared, host, text);
            self.resume_if_done(host);
            return;
        }
        let Some(pending) = self.prompt.take() else {
            return;
        };
        self.restore(host, pending);
        let separator = if pending.forward { '/' } else { '?' };
        let (pattern, offset) = split(text, separator);
        let previous = shared.search.as_ref();
        let (pattern, offset) = if pattern.is_empty() {
            match previous {
                Some(searched) => (
                    searched.pattern.clone(),
                    offset.unwrap_or(if text.is_empty() {
                        searched.offset
                    } else {
                        Offset::default()
                    }),
                ),
                None => {
                    self.error(host, Message::NoSearch);
                    return;
                }
            }
        } else {
            (pattern, offset.unwrap_or_default())
        };
        shared.remember_search(text);
        shared.set_search(pattern, pending.forward, offset, false);

        // 模式已经记为上一次查找：命令中的查找动作改为重复上一次查找。
        let again = Movement {
            motion: Motion::SearchNext,
            arg: None,
        };
        let mut command = pending.command;
        command.body = match command.body {
            Body::Operator {
                operator, force, ..
            } => Body::Operator {
                operator,
                target: Target::Motion(again),
                force,
            },
            _ => Body::Motion(again),
        };
        self.run(shared, host, command);
        self.resume_if_done(host);
    }

    /// 返回从第 `line` 行之后（`forward` 为真）或之前第一个匹配 `pattern` 的行，供命令行的
    /// 范围使用；模式为空时沿用上一次查找的模式。找不到或模式有误时提示并返回 `None`。
    pub(super) fn search_line(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        pattern: &str,
        line: usize,
        forward: bool,
    ) -> Option<usize> {
        if !pattern.is_empty() {
            shared.set_search_pattern(pattern.to_owned());
        }
        let Some((query, _)) = shared.query(host) else {
            self.failed = true;
            return None;
        };
        let text = host.text();
        let last = text::last_line(text);
        let wrap = shared.options.wrapscan;
        let found = if forward {
            let next = (line < last).then(|| text.line_start(line + 1));
            next.and_then(|from| find(host, &query, from, true))
                .or_else(|| wrap.then(|| find(host, &query, 0, true)).flatten())
        } else {
            let end = text.end_pos();
            find(host, &query, text.line_start(line), false)
                .or_else(|| wrap.then(|| find(host, &query, end + 1, false)).flatten())
        };
        match found {
            Some((start, _)) => Some(text::line_of(host.text(), start).min(last)),
            None => {
                self.error(host, Message::NotFound);
                None
            }
        }
    }

    /// `n`、`N`：重复上一次查找；`reverse` 为真时方向相反。
    pub(super) fn search_again(
        &mut self,
        shared: &Shared,
        host: &mut impl Host,
        from: usize,
        count: Option<u32>,
        reverse: bool,
    ) -> Option<Moved> {
        let (query, searched) = shared.query(host)?;
        let forward = searched.forward != reverse;
        let offset = searched.offset;
        // 带字符偏移时先把起点移回偏移之前，避免停在同一处匹配上。
        let mut at = from;
        if !offset.line && offset.amount != 0 {
            at = shifted(host.text(), at, -offset.amount);
        }
        let times = count.map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let mut found = None;
        for _ in 0..times.min(MAX_MATCHES) {
            match step(
                host,
                &query,
                at,
                forward,
                offset.end,
                shared.options.wrapscan,
            ) {
                Some(next) => {
                    at = key(host, next, offset.end);
                    found = Some(next);
                }
                None => break,
            }
        }
        let Some((start, end)) = found else {
            self.error(host, Message::NotFound);
            return None;
        };
        let text = host.text();
        let (pos, span) = if offset.line {
            let line = text::line_of(text, start)
                .saturating_add_signed(offset.amount)
                .min(text::last_line(text));
            (text.line_start(line), Span::Linewise)
        } else {
            // 起点在行尾的匹配算作位于该行的最后一个字符上。
            let base = key(host, (start, end), offset.end);
            let span = if offset.end {
                Span::Inclusive
            } else {
                Span::Exclusive
            };
            (shifted(text, base, offset.amount), span)
        };
        Some(Moved {
            numbered: true,
            ..Moved::new(pos, span)
        })
    }

    /// `*`、`#`、`g*`、`g#`：查找光标处的词。
    pub(super) fn search_word(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        from: usize,
        count: Option<u32>,
        motion: Motion,
    ) -> Option<Moved> {
        let forward = matches!(
            motion,
            Motion::SearchWordForward | Motion::SearchPartForward
        );
        let whole = matches!(
            motion,
            Motion::SearchWordForward | Motion::SearchWordBackward
        );
        let text = host.text();
        let (start, end) = word_at(text, from)?;
        let word = text.substring(start, end);
        // 模式中有特殊含义的字符按字面查找。
        let special = if forward { "/.*~[^$\\" } else { "?.*~[^$\\" };
        let is_word = |c: Option<char>| text::class(c, false) >= 2;
        let mut pattern = String::new();
        if whole && is_word(word.chars().next()) {
            pattern.push_str("\\<");
        }
        for c in word.chars() {
            if special.contains(c) {
                pattern.push('\\');
            }
            pattern.push(c);
        }
        if whole && is_word(word.chars().next_back()) {
            pattern.push_str("\\>");
        }
        shared.remember_search(&pattern);
        shared.set_search(pattern, forward, Offset::default(), true);
        self.search_again(shared, host, start, count, false)
    }

    /// 返回 `gn`（`forward` 为真）或 `gN` 选中的匹配：起点与终点（不含）。
    ///
    /// `extending` 表示在已有的可视选区上延伸：光标处的匹配不算。
    fn matched(
        &mut self,
        shared: &Shared,
        host: &mut impl Host,
        cursor: usize,
        forward: bool,
        count: usize,
        extending: bool,
    ) -> Option<(usize, usize)> {
        let (query, _) = shared.query(host)?;
        let end = host.text().end_pos();
        let mut found = None;
        if forward {
            let at = if extending { cursor + 1 } else { cursor };
            let text = host.text();
            let mut from = text.line_start(text::line_of(text, at));
            for _ in 0..MAX_MATCHES {
                let Some(next) = find(host, &query, from, true) else {
                    break;
                };
                if key(host, next, true) >= at {
                    found = Some(next);
                    break;
                }
                from = next.0 + 1;
            }
            found = found.or_else(|| find(host, &query, 0, true));
        } else {
            let at = if extending { cursor } else { cursor + 1 };
            found = host
                .find(&query, at, false)
                .or_else(|| find(host, &query, end + 1, false));
        }
        for _ in 1..count.min(MAX_MATCHES) {
            let at = found?.0;
            found = step(host, &query, at, forward, false, shared.options.wrapscan);
        }
        let found = found.filter(|(start, end)| end > start);
        if found.is_none() {
            self.error(host, Message::NotFound);
        }
        found
    }

    /// 返回等待动作时 `gn`、`gN` 的范围：起点，以及终点上的字符在范围之内的动作结果。
    pub(super) fn match_target(
        &mut self,
        shared: &Shared,
        host: &mut impl Host,
        cursor: usize,
        forward: bool,
        count: usize,
    ) -> Option<(usize, Moved)> {
        let (start, end) = self.matched(shared, host, cursor, forward, count, false)?;
        let last = winc_core::movement::prev_grapheme(host.text(), end).max(start);
        Some((
            start,
            Moved {
                want: WantUpdate::Recompute,
                ..Moved::new(last, Span::Inclusive)
            },
        ))
    }

    /// `gn`、`gN`：选中上一次查找的模式的一处匹配；已在可视模式时把选区延伸到该匹配。
    pub(super) fn select_match(
        &mut self,
        shared: &Shared,
        host: &mut impl Host,
        forward: bool,
        count: usize,
    ) {
        let cursor = self.caret(host);
        // 可视模式下光标处的匹配不算，选区延伸到下一处匹配。
        let extending = self.visual.is_some();
        let Some((start, end)) = self.matched(shared, host, cursor, forward, count, extending)
        else {
            return;
        };
        let last = winc_core::movement::prev_grapheme(host.text(), end).max(start);
        let (anchor, cursor) = match self.visual {
            Some(visual) if extending => {
                // 光标在选区起点一端时移到匹配的起点，否则移到匹配的最后一个字符。
                let target = if !forward || cursor < visual.anchor {
                    start
                } else {
                    last
                };
                (visual.anchor, target)
            }
            _ if forward => (start, last),
            _ => (last, start),
        };
        self.visual = Some(Visual {
            kind: VisualKind::Char,
            anchor,
            cursor,
        });
        self.mode = Mode::Visual;
        self.goto(host, cursor, WantUpdate::Recompute);
    }
}

/// 返回光标处或其后的词的位置区间：先找词字符组成的词，没有时取一串非空白字符。
fn word_at<T: Lines>(text: &T, cursor: usize) -> Option<(usize, usize)> {
    let line = text::line_of(text, cursor);
    let base = text.line_start(line);
    let chars: Vec<char> = text.line_content(line).slice().chars().collect();
    let class = |col: usize| text::class(chars.get(col).copied(), false);
    let from = cursor - base;
    for pass in 0..2 {
        // 第一遍只找词字符，第二遍接受任何非空白字符。
        let mut col = from;
        while col < chars.len() && (class(col) == 0 || (pass == 0 && class(col) == 1)) {
            col += 1;
        }
        if col >= chars.len() {
            continue;
        }
        let this = class(col);
        while col > 0 && class(col - 1) == this {
            col -= 1;
        }
        if pass == 0 && this < 2 {
            continue;
        }
        let mut end = col;
        while end < chars.len()
            && if pass == 0 {
                class(end) == this
            } else {
                class(end) != 0
            }
        {
            end += 1;
        }
        return Some((base + col, base + end));
    }
    None
}
