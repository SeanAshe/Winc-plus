//! 括号配对：`%`、`[(`、`[{`、`])`、`]}`。
//!
//! 配对时跳过双引号字符串之内的括号（起点在字符串之内时只在字符串之内配对）、单引号括起的
//! 单个字符，以及转义状态与起点不同的括号。只识别 `()`、`[]`、`{}`。

use winc_core::Lines;

use crate::text;

/// 配对时最多检查的字符数（字符），每行的换行符也算一个。
///
/// 取自调参：与跳到配对括号的命令所用的上限同一量级，限制没有配对的括号在大文件中的扫描耗时。
const MAX_SCAN: usize = 1_000_000;

/// 返回与 `open` 或 `close` 配对的另一半，以及查找的方向是否向后（朝文本开头）。
fn partner(c: char) -> Option<(char, bool)> {
    Some(match c {
        '(' => (')', false),
        '[' => (']', false),
        '{' => ('}', false),
        ')' => ('(', true),
        ']' => ('[', true),
        '}' => ('{', true),
        _ => return None,
    })
}

/// `%`：返回光标处或其右侧第一个括号所配对的括号的位置。
pub(super) fn matching(text: &impl Lines, from: usize) -> Option<usize> {
    let (line, col) = text.pos_to_line_col(from);
    let chars = line_chars(text, line);
    let (at, start, (target, backward)) = chars
        .iter()
        .enumerate()
        .skip(col)
        .find_map(|(i, c)| partner(*c).map(|p| (i, *c, p)))?;
    let escaped = backslashes_before(&chars, at) % 2 == 1;
    search(text, line, at, start, target, backward, escaped, false)
}

/// `[(` 等：返回从 `from` 向后（`forward` 为假）或向前找到的未配对的 `open` 或 `close` 的位置。
pub(crate) fn unmatched(
    text: &impl Lines,
    from: usize,
    open: char,
    close: char,
    forward: bool,
) -> Option<usize> {
    let (line, col) = text.pos_to_line_col(from);
    let (nested, target) = if forward {
        (open, close)
    } else {
        (close, open)
    };
    search(text, line, col, nested, target, !forward, false, false)
}

/// 返回从 `from` 向后（`forward` 为假）找到的未配对的 `open`，或向前找到的第一个不在嵌套中的
/// `open`；引号之内的括号同样参与配对。
pub(crate) fn enclosing(
    text: &impl Lines,
    from: usize,
    open: char,
    close: char,
    forward: bool,
) -> Option<usize> {
    let (line, col) = text.pos_to_line_col(from);
    search(text, line, col, close, open, !forward, false, true)
}

/// 返回第 `line` 行的全部字符。
fn line_chars(text: &impl Lines, line: usize) -> Vec<char> {
    text.line_content(line).slice().chars().collect()
}

/// 返回 `chars` 中下标 `at` 之前连续的反斜杠的个数。
fn backslashes_before(chars: &[char], at: usize) -> usize {
    chars[..at.min(chars.len())]
        .iter()
        .rev()
        .take_while(|c| **c == '\\')
        .count()
}

/// 从第 `line` 行的第 `col` 个字符出发查找 `target`，遇到 `nested` 时嵌套加深一层。
///
/// 起点上的字符不参与配对。`escaped` 是起点的括号之前是否有奇数个反斜杠，只有转义状态相同的
/// 括号参与配对。`plain` 为真时不识别引号。
#[allow(clippy::too_many_arguments)]
fn search(
    text: &impl Lines,
    line: usize,
    col: usize,
    nested: char,
    target: char,
    backward: bool,
    escaped: bool,
    plain: bool,
) -> Option<usize> {
    let last = text::last_line(text);
    let mut line = line;
    let mut chars = line_chars(text, line);
    // 行尾的位置用行长表示。
    let mut col = col;
    let mut depth = 0usize;
    let mut budget = MAX_SCAN;
    // 该行的引号是否需要识别：`None` 表示尚未统计。
    let mut quotes: Option<bool> = None;
    let mut in_quote = false;
    // 起点是否在字符串之内：`None` 表示尚未判断。
    let mut start_in_quote: Option<bool> = None;

    loop {
        budget = budget.checked_sub(1)?;
        // 移到下一个位置。
        if backward {
            if col == 0 {
                line = line.checked_sub(1)?;
                chars = line_chars(text, line);
                col = chars.len();
                quotes = None;
            } else {
                col -= 1;
            }
        } else if col >= chars.len() {
            if line >= last {
                return None;
            }
            line += 1;
            chars = line_chars(text, line);
            col = 0;
            quotes = None;
        } else {
            col += 1;
        }

        if plain {
            quotes = Some(false);
        }
        if quotes.is_none() {
            // 统计该行中未转义的双引号：个数为偶数时才识别字符串；个数为奇数时，只有该行或
            // 上一行以反斜杠结尾（字符串跨行）才识别。
            let mut count = 0usize;
            // 起点之前有偶数个引号：上一行的字符串延续到该行时，起点仍在字符串之内。
            let mut at_start = true;
            let mut i = 0;
            while i < chars.len() {
                if i == col + usize::from(backward) {
                    at_start = count.is_multiple_of(2);
                }
                let quoted_char = i > 0 && chars[i - 1] == '\'' && chars.get(i + 1) == Some(&'\'');
                if chars[i] == '"' && !quoted_char {
                    count += 1;
                }
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                i += 1;
            }
            let mut known = count.is_multiple_of(2);
            if !known {
                in_quote = false;
                if chars.last() == Some(&'\\') {
                    known = true;
                    if start_in_quote.is_none() {
                        in_quote = true;
                        start_in_quote = Some(true);
                    } else if backward {
                        in_quote = true;
                    }
                }
                if line > 0 && line_chars(text, line - 1).last() == Some(&'\\') {
                    known = true;
                    if start_in_quote.is_none() {
                        in_quote = at_start;
                        if in_quote {
                            start_in_quote = Some(true);
                        }
                    } else if !backward {
                        in_quote = true;
                    }
                }
            }
            quotes = Some(known);
        }
        if start_in_quote.is_none() {
            start_in_quote = Some(false);
        }

        match chars.get(col).copied() {
            None => {
                // 行尾：该行不以反斜杠结尾时，字符串到此结束。
                if col == 0 || chars[col - 1] != '\\' {
                    in_quote = false;
                    start_in_quote = Some(false);
                }
            }
            Some('"') => {
                if quotes == Some(true) && backslashes_before(&chars, col).is_multiple_of(2) {
                    in_quote = !in_quote;
                    start_in_quote = Some(false);
                }
            }
            Some(c) => {
                // 单引号括起的单个字符（`'x'`、`'\x'`）整体跳过。
                if c == '\'' && !plain {
                    if backward {
                        if col > 1 && chars[col - 2] == '\'' {
                            col -= 2;
                            continue;
                        }
                        if col > 2 && chars[col - 2] == '\\' && chars[col - 3] == '\'' {
                            col -= 3;
                            continue;
                        }
                    } else if col + 1 < chars.len() {
                        if chars[col + 1] == '\\' && chars.get(col + 3) == Some(&'\'') {
                            col += 3;
                            continue;
                        }
                        if chars.get(col + 2) == Some(&'\'') {
                            col += 2;
                            continue;
                        }
                    }
                }
                let counted = !in_quote || start_in_quote == Some(true);
                if counted
                    && (c == nested || c == target)
                    && (backslashes_before(&chars, col) % 2 == 1) == escaped
                {
                    if c == nested {
                        depth += 1;
                    } else if depth == 0 {
                        return Some(text.line_start(line) + col);
                    } else {
                        depth -= 1;
                    }
                }
            }
        }
    }
}
