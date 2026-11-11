//! `:sort`：对范围中的行排序。

use std::cmp::Ordering;

use winc_core::{Change, Lines};

use super::ex::Span;
use super::{Failed, Shared, Vim};
use crate::ex;
use crate::host::{Host, Message};
use crate::operator;
use crate::pattern;
use crate::text;

/// 排序的方式。
#[derive(Debug, Clone, Copy, Default)]
struct Order {
    /// 是否忽略大小写。
    ignore_case: bool,
    /// 是否按行中的第一个十进制数排序。
    numeric: bool,
    /// 是否按行中的第一个十六进制数排序。
    hex: bool,
    /// 是否只保留相同的行中的第一行。
    unique: bool,
    /// 是否按模式匹配到的文本排序；为假时按匹配之后的文本排序。
    matched: bool,
    /// 是否从大到小。
    reverse: bool,
}

/// 返回文本 `key` 中第一个数的值；`hex` 为真时按十六进制读取。没有数时返回 `None`。
fn number_in(key: &str, hex: bool) -> Option<i128> {
    let bytes = key.as_bytes();
    let digit = |b: u8| {
        if hex {
            b.is_ascii_hexdigit()
        } else {
            b.is_ascii_digit()
        }
    };
    let mut start = bytes.iter().position(|b| digit(*b))?;
    // 十六进制数可以带 `0x` 前缀。
    if hex && bytes[start] == b'0' && matches!(bytes.get(start + 1), Some(b'x' | b'X')) {
        let after = start + 2;
        if bytes.get(after).is_some_and(|b| digit(*b)) {
            start = after;
        }
    }
    let end = (start..bytes.len())
        .find(|at| !digit(bytes[*at]))
        .unwrap_or(bytes.len());
    let radix = if hex { 16 } else { 10 };
    // 位数过多时取上限。
    let value = i128::from_str_radix(&key[start..end], radix).unwrap_or(i128::MAX);
    let negative = start > 0 && bytes[start - 1] == b'-';
    Some(if negative { -value } else { value })
}

/// 比较两行的排序依据。
fn compare(a: &str, b: &str, order: Order) -> Ordering {
    if order.numeric || order.hex {
        // 没有数的行排在最前。
        number_in(a, order.hex).cmp(&number_in(b, order.hex))
    } else if order.ignore_case {
        let fold = |text: &str| {
            text.chars()
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        fold(a).cmp(&fold(b))
    } else {
        a.cmp(b)
    }
}

impl Vim {
    /// `:sort`：对范围中的行排序，没有范围时为全部的行；`reverse` 为真时从大到小。
    pub(super) fn ex_sort(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        given: Option<Span>,
        reverse: bool,
        args: &str,
    ) -> Result<(), Failed> {
        let mut order = Order {
            reverse,
            ..Order::default()
        };
        let mut query = None;
        let mut rest = args.trim();
        while let Some(c) = rest.chars().next() {
            let after = &rest[c.len_utf8()..];
            match c {
                'i' => order.ignore_case = true,
                'n' => order.numeric = true,
                'x' => order.hex = true,
                'u' => order.unique = true,
                'r' => order.matched = true,
                ' ' | '\t' => {}
                c if !c.is_alphanumeric() && c != '"' && c != '|' => {
                    let (source, after) = ex::delimited(after, c);
                    let source = if source.is_empty() {
                        shared.search_pattern().map(str::to_owned)
                    } else {
                        Some(source)
                    };
                    let Some(source) = source else {
                        return self.fail(host, Message::NoSearch);
                    };
                    match pattern::translate(&source, shared.ignore_case, shared.smart_case) {
                        Ok(translated) => query = Some(translated),
                        Err(at) => return self.fail(host, Message::BadPattern(at)),
                    }
                    rest = after;
                    continue;
                }
                _ => return self.fail(host, Message::BadArgument),
            }
            rest = after;
        }

        let text = host.text();
        let (first, last) = match given {
            Some(span) => span.lines(),
            None => (0, text::last_line(text)),
        };
        let (from, to) = (text.line_start(first), text.line_end(last));
        if !host.fits(from, to) {
            return self.fail(host, Message::TooLarge);
        }
        // 各行的内容与排序依据。
        let mut rows: Vec<(String, String)> = Vec::with_capacity(last - first + 1);
        for line in first..=last {
            let text = host.text();
            let (start, end) = (text.line_start(line), text.line_end(line));
            let content = operator::normalized(text.substring(start, end));
            let found = (query.as_ref())
                .and_then(|query| host.find_groups(query, start))
                .and_then(|groups| groups.first().copied().flatten())
                .filter(|(_, to)| *to <= end);
            // 给出了模式而没有匹配的行，排序依据为空。
            let key = match found {
                Some((a, b)) if order.matched => host.text().substring(a, b),
                Some((_, b)) => host.text().substring(b, end),
                None if query.is_some() => String::new(),
                None => content.clone(),
            };
            rows.push((content, key));
        }
        let before: Vec<String> = rows.iter().map(|(content, _)| content.clone()).collect();
        rows.sort_by(|a, b| compare(&a.1, &b.1, order));
        // 从大到小是把排好的结果倒过来，相同的行次序随之相反。
        if order.reverse {
            rows.reverse();
        }
        if order.unique {
            rows.dedup_by(|b, a| compare(&a.1, &b.1, order) == Ordering::Equal);
        }
        let sorted: Vec<String> = rows.into_iter().map(|(content, _)| content).collect();
        if sorted != before {
            let ending = host.line_ending().to_owned();
            let text = host.text();
            let change = Change {
                start: from,
                removed: text.substring(from, to),
                inserted: sorted.join(&ending),
            };
            let cursor = self.caret(host);
            if !self.edit(host, vec![change], cursor, first) {
                return Err(Failed);
            }
        }
        let text = host.text();
        let pos = text::first_non_blank(text, first.min(text::last_line(text)));
        self.settle(host, pos);
        Ok(())
    }
}
