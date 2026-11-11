//! 成对符号：`ys` 与可视模式的 `S` 加上，`ds` 删除，`cs` 替换。
//!
//! 左括号（`(` `[` `{`）加上时括号之内留一个空格，删除时连同括号之内的空白；右括号与 `b` `B`
//! `r` `a` 不带空白。引号与其余的标点两侧相同。

use winc_core::{Change, Lines};

use super::{Shared, Vim};
use crate::command::{Shape, TextObject};
use crate::host::{Host, Message};
use crate::object::{self, At};
use crate::operator::{self, Range, Region};
use crate::parser::Command;
use crate::text;

/// 返回符号 `with` 对应的一对文字；字母、数字与需要输入标签名的写法返回 `None`。
fn pair_of(with: char) -> Option<(String, String)> {
    let plain = |open: &str, close: &str| Some((open.to_owned(), close.to_owned()));
    match with {
        '(' => plain("( ", " )"),
        ')' | 'b' => plain("(", ")"),
        '[' => plain("[ ", " ]"),
        ']' | 'r' => plain("[", "]"),
        '{' => plain("{ ", " }"),
        '}' | 'B' => plain("{", "}"),
        '>' | 'a' => plain("<", ">"),
        '<' | 't' => None,
        c if c.is_alphanumeric() || c.is_control() => None,
        c => Some((c.to_string(), c.to_string())),
    }
}

/// 两侧的符号在文本中的位置区间：左侧与右侧各一段。
#[derive(Debug, Clone, Copy)]
struct Pair {
    /// 左侧的符号。
    open: (usize, usize),
    /// 右侧的符号。
    close: (usize, usize),
}

impl Vim {
    /// 在 `range` 的两端加上符号 `with`；末尾的空白不包在内，按行的范围从首行的第一个非空白
    /// 字符到末行的行尾。光标放在左侧的符号上。
    pub(super) fn wrap(&mut self, host: &mut impl Host, range: Range, with: char) -> bool {
        let Some((open, close)) = pair_of(with) else {
            self.error(host, Message::Unsupported);
            return false;
        };
        let text = host.text();
        let (from, mut to) = match range.region {
            Region::Chars { from, to } => (from, to),
            Region::Lines { first, last } => {
                (text::first_non_blank(text, first), text.line_end(last))
            }
        };
        while to > from && text.char_at(to - 1).is_some_and(text::is_blank) {
            to -= 1;
        }
        let insert = |start: usize, inserted: String| Change {
            start,
            removed: String::new(),
            inserted,
        };
        let changes = vec![insert(to, close), insert(from, open)];
        let done = self.edit(host, changes, range.start, range.first);
        self.settle(host, from);
        done
    }

    /// `ds{符号}`、`cs{符号}{符号}`：删除光标两侧的符号 `old`，`new` 给出时换成该符号。
    pub(super) fn unwrap(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        old: Option<char>,
        new: Option<char>,
    ) {
        if self.blocked(host) {
            return;
        }
        let cursor = self.caret(host);
        let Some(pair) = old.and_then(|old| self.pair_at(host, cursor, old)) else {
            self.failed = true;
            return;
        };
        let (open, close) = match new.map(pair_of) {
            Some(Some(pair)) => pair,
            Some(None) => return self.error(host, Message::Unsupported),
            None => (String::new(), String::new()),
        };
        let text = host.text();
        let replace = |(from, to): (usize, usize), inserted: String| Change {
            start: from,
            removed: operator::normalized(text.substring(from, to)),
            inserted,
        };
        let changes = vec![replace(pair.close, close), replace(pair.open, open)];
        let line = text::line_of(text, pair.open.0);
        if self.edit(host, changes, cursor, line) {
            self.settle(host, pair.open.0);
            self.remember(shared, command, None);
        }
    }

    /// 返回光标 `cursor` 两侧的符号 `with` 的位置；找不到时返回 `None`。
    fn pair_at(&self, host: &impl Host, cursor: usize, with: char) -> Option<Pair> {
        let text = host.text();
        let shape = match with {
            '(' | ')' | 'b' => Shape::Paren,
            '[' | ']' | 'r' => Shape::Bracket,
            '{' | '}' | 'B' => Shape::Brace,
            '<' | '>' | 'a' => Shape::Angle,
            't' => Shape::Tag,
            '"' => Shape::DoubleQuote,
            '\'' => Shape::SingleQuote,
            '`' => Shape::Backtick,
            c if c.is_alphanumeric() => return None,
            // 其余的符号：同一行上光标之前与之后最近的一处；光标在符号上时，之前还有一处则
            // 光标处是右侧，否则是左侧。
            c => {
                let line = text::line_of(text, cursor);
                let (start, end) = (text.line_start(line), text.line_end(line));
                let is = |pos: &usize| text.char_at(*pos) == Some(c);
                let before = (start..cursor).rev().find(is);
                let after = (cursor + 1..end).find(is);
                let (open, close) = match (is(&cursor), before) {
                    (true, Some(before)) => (before, cursor),
                    (true, None) => (cursor, after?),
                    (false, before) => (before?, after?),
                };
                return Some(Pair {
                    open: (open, open + 1),
                    close: (close, close + 1),
                });
            }
        };
        let at = At {
            cursor,
            anchor: None,
            linewise: false,
            count: 1,
            tab_width: host.tab_width(),
        };
        let object = TextObject {
            shape,
            around: true,
        };
        let selected = object::select(text, object, at)?;
        let last = if selected.inclusive {
            selected.end
        } else {
            selected.end.checked_sub(1)?
        };
        let (left, right) = match with {
            '(' | ')' | 'b' => ('(', ')'),
            '[' | ']' | 'r' => ('[', ']'),
            '{' | '}' | 'B' => ('{', '}'),
            '<' | '>' | 'a' | 't' => ('<', '>'),
            quote => (quote, quote),
        };
        let is = |c: char| move |pos: &usize| text.char_at(*pos) == Some(c);
        let open = (selected.start..=last).find(is(left))?;
        let close = (open + 1..=last).rev().find(is(right))?;
        if shape == Shape::Tag {
            // 标签：左侧到第一个 `>` 为止，右侧从最后一个 `<` 起。
            let open_end = (open..=last).find(is('>'))?;
            let close_start = (open_end..=close).rev().find(is('<'))?;
            return Some(Pair {
                open: (open, open_end + 1),
                close: (close_start, close + 1),
            });
        }
        let (mut open_end, mut close_start) = (open + 1, close);
        if matches!(with, '(' | '[' | '{') {
            while open_end < close_start && text.char_at(open_end).is_some_and(text::is_blank) {
                open_end += 1;
            }
            while close_start > open_end
                && text.char_at(close_start - 1).is_some_and(text::is_blank)
            {
                close_start -= 1;
            }
        }
        Some(Pair {
            open: (open, open_end),
            close: (close_start, close + 1),
        })
    }
}
