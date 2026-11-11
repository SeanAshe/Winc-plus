//! `Ctrl-A`、`Ctrl-X`：增减光标处或其后的数。
//!
//! 识别十进制（可带负号）、`0x` 开头的十六进制、`0b` 开头的二进制，以及以 `0` 开头、各位都
//! 小于 8 的八进制；结果保持原来的进制、位数与十六进制字母的大小写。

use winc_core::{Change, Lines};

use super::{Shared, Vim};
use crate::host::Host;
use crate::parser::Command;
use crate::text;

/// 数的进制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Radix {
    /// 十进制。
    Decimal,
    /// 八进制，以 `0` 开头。
    Octal,
    /// 十六进制，值为前缀中的字母。
    Hex(char),
    /// 二进制，值为前缀中的字母。
    Binary(char),
}

/// 一行中找到的数。
#[derive(Debug, Clone, Copy)]
struct Found {
    /// 起点在行内的字符偏移，含负号与前缀。
    start: usize,
    /// 终点（不含）在行内的字符偏移。
    end: usize,
    /// 进制。
    radix: Radix,
    /// 是否带负号。
    negative: bool,
    /// 绝对值；超出 64 位时取最大值。
    value: u64,
}

/// 在 `chars`（一行的全部字符）中找到下标 `col` 处或其后的数。
fn find(chars: &[char], col: usize) -> Option<Found> {
    let get = |i: usize| chars.get(i).copied();
    let digit = |i: usize, radix: u32| get(i).is_some_and(|c| c.is_digit(radix));
    let prefixed = |i: usize, letters: [char; 2], radix: u32| {
        i > 0
            && get(i).is_some_and(|c| letters.contains(&c))
            && get(i - 1) == Some('0')
            && digit(i + 1, radix)
    };

    // 先判断光标是否在十六进制或二进制数的各位上。
    let mut at = col;
    while at > 0 && digit(at, 2) {
        at -= 1;
    }
    while at > 0 && digit(at, 16) {
        at -= 1;
    }
    if !prefixed(at, ['x', 'X'], 16) {
        at = col;
        while at > 0 && digit(at, 10) {
            at -= 1;
        }
    }
    let mut start = if prefixed(at, ['x', 'X'], 16) || prefixed(at, ['b', 'B'], 2) {
        at - 1
    } else {
        // 向后找到第一个数字，再退到这串数字的开头。
        let mut at = col;
        while get(at).is_some() && !digit(at, 10) {
            at += 1;
        }
        while at > 0 && digit(at - 1, 10) {
            at -= 1;
        }
        at
    };
    if !digit(start, 10) {
        return None;
    }
    let mut negative = start > 0 && get(start - 1) == Some('-');
    if negative {
        start -= 1;
    }

    // 读出前缀与各位。
    let first = start + usize::from(negative);
    let (radix, digits) = match (get(first), get(first + 1)) {
        (Some('0'), Some(c @ ('x' | 'X'))) if digit(first + 2, 16) => (Radix::Hex(c), first + 2),
        (Some('0'), Some(c @ ('b' | 'B'))) if digit(first + 2, 2) => (Radix::Binary(c), first + 2),
        (Some('0'), Some('0'..='7')) => {
            // 之后的各位都小于 8 才是八进制。
            let mut at = first + 1;
            while digit(at, 8) {
                at += 1;
            }
            if digit(at, 10) {
                (Radix::Decimal, first)
            } else {
                (Radix::Octal, first)
            }
        }
        _ => (Radix::Decimal, first),
    };
    let base = match radix {
        Radix::Decimal => 10,
        Radix::Octal => 8,
        Radix::Hex(_) => 16,
        Radix::Binary(_) => 2,
    };
    let mut end = digits;
    let mut value = 0u64;
    while let Some(digit) = get(end).and_then(|c| c.to_digit(base)) {
        value = value
            .checked_mul(u64::from(base))
            .and_then(|value| value.checked_add(u64::from(digit)))
            .unwrap_or(u64::MAX);
        end += 1;
    }
    // 负号只对十进制数有意义。
    if radix != Radix::Decimal && negative {
        start += 1;
        negative = false;
    }
    Some(Found {
        start,
        end,
        radix,
        negative,
        value,
    })
}

/// 返回把 `found` 加上（`subtract` 为假）或减去 `by` 之后的写法；`old` 是原来的写法。
fn changed(found: Found, old: &[char], by: u64, subtract: bool) -> String {
    let mut negative = found.negative;
    let subtract = subtract != negative;
    let mut value = if subtract {
        found.value.wrapping_sub(by)
    } else {
        found.value.wrapping_add(by)
    };
    if found.radix == Radix::Decimal {
        // 越过零时变号。
        if subtract {
            if value > found.value {
                value = (!value).wrapping_add(1);
                negative = !negative;
            }
        } else if value < found.value {
            value = !value;
            negative = !negative;
        }
        if value == 0 {
            negative = false;
        }
    }

    // 十六进制字母的大小写取原来最后一个字母的大小写。
    let upper = old
        .iter()
        .rev()
        .find(|c| c.is_ascii_alphabetic())
        .is_some_and(char::is_ascii_uppercase);
    let digits = match found.radix {
        Radix::Decimal => value.to_string(),
        Radix::Octal => format!("{value:o}"),
        Radix::Hex(_) if upper => format!("{value:X}"),
        Radix::Hex(_) => format!("{value:x}"),
        Radix::Binary(_) => format!("{value:b}"),
    };
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    // 原来的位数（不含负号与前缀）；以 0 开头的数保持位数不变。
    let mut width = old.len() - usize::from(found.negative);
    match found.radix {
        Radix::Hex(letter) | Radix::Binary(letter) => {
            out.push('0');
            out.push(letter);
            width -= 2;
        }
        Radix::Octal => {
            out.push('0');
            width -= 1;
        }
        Radix::Decimal => {}
    }
    let first = old.get(usize::from(found.negative)).copied();
    if first == Some('0') && found.radix != Radix::Decimal {
        for _ in digits.len()..width {
            out.push('0');
        }
    }
    out.push_str(&digits);
    out
}

impl Vim {
    /// `Ctrl-A`、`Ctrl-X`：把光标处或其后的数加上或减去次数，光标移到结果的最后一位。
    pub(super) fn add(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        subtract: bool,
    ) {
        if self.blocked(host) {
            return;
        }
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let base = text.line_start(line);
        let chars: Vec<char> = text.line_content(line).slice().chars().collect();
        let Some(found) = find(&chars, cursor - base) else {
            return;
        };
        let by = command.count.map_or(1, u64::from);
        let old = &chars[found.start..found.end];
        let new = changed(found, old, by, subtract);
        let length = new.chars().count();
        let change = Change {
            start: base + found.start,
            removed: old.iter().collect(),
            inserted: new,
        };
        if self.edit(host, vec![change], cursor, line) {
            self.remember(shared, command, None);
        }
        self.settle(host, base + found.start + length.saturating_sub(1));
    }
}
