//! 光标移动：按字素、按词、按行，上下移动时保持目标显示列。
//!
//! 位置都是 `char` 偏移，行号从 0 起。

use unicode_segmentation::UnicodeSegmentation;

use crate::text::{CellLayout, Lines, leading_whitespace};

/// 查找字素边界时向前或向后最多读取的字符数（字符）。
///
/// 64 大于常见 emoji 序列的长度（例如家庭 emoji 为 7 个字符），用于限制单次查找的
/// 字符数。
const GRAPHEME_WINDOW: usize = 64;

/// 返回 `pos` 向左移动一个字素后的位置。
///
/// 在行首时返回上一行行末，在文本开头时返回 0。只检查 `pos` 之前至多 `GRAPHEME_WINDOW` 个
/// 字符，更长的字素簇会被拆开。
pub fn prev_grapheme(text: &impl Lines, pos: usize) -> usize {
    let (line, col) = text.pos_to_line_col(pos);

    if col == 0 {
        return if line == 0 {
            0
        } else {
            text.line_end(line - 1)
        };
    }

    let content = text.line_content(line);
    let from = col.saturating_sub(GRAPHEME_WINDOW);
    let window = content.slice().slice(from..col).to_string();
    let last = window
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(i, _)| i);

    text.line_start(line) + from + window[..last].chars().count()
}

/// 返回 `pos` 向右移动一个字素后的位置。
///
/// 在行末时返回下一行行首，在文本末尾时返回文本末尾的位置。只检查 `pos` 之后至多
/// `GRAPHEME_WINDOW` 个字符。
pub fn next_grapheme(text: &impl Lines, pos: usize) -> usize {
    let (line, col) = text.pos_to_line_col(pos);
    let content = text.line_content(line);
    let slice = content.slice();
    let len = slice.len_chars();

    if col >= len {
        return if line + 1 < text.line_count() {
            text.line_start(line + 1)
        } else {
            text.line_end(line)
        };
    }

    let window = slice
        .slice(col..(col + GRAPHEME_WINDOW).min(len))
        .to_string();
    let first = window.graphemes(true).next().map_or(window.len(), str::len);

    text.line_start(line) + col + window[..first].chars().count()
}

/// 返回 `pos` 所在字素簇的开头位置；`pos` 本身是簇边界时原样返回。
///
/// 簇指带肤色修饰的 emoji、国旗、带组合声调的字母等。按显示列换算位置（上下移动、鼠标点击）
/// 只按单个字符的宽度计算，结果可能落在簇中间，之后的编辑会拆开该簇，因此需要先移动到簇边界。
pub fn snap_to_grapheme(text: &impl Lines, pos: usize) -> usize {
    let start = prev_grapheme(text, pos);
    if next_grapheme(text, start) == pos {
        pos
    } else {
        start
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// 空白字符。
    Space,
    /// 单词字符：字母、数字和 `_`。
    Word,
    /// 除空白和单词字符以外的字符。
    Punct,
}

/// 返回字符所属的类别：空白、单词字符（字母、数字、`_`）或标点。
fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// 返回 `pos` 向左移动一个词后的位置：先跳过空白，再跳过同一类字符。
///
/// 在行首时等同于 [`prev_grapheme`]，即跳到上一行行末。
pub fn prev_word(text: &impl Lines, pos: usize) -> usize {
    let (line, col) = text.pos_to_line_col(pos);

    if col == 0 {
        return prev_grapheme(text, pos);
    }

    let content = text.line_content(line);
    let mut chars = content.slice().chars_at(col).reversed().peekable();
    let mut i = col;

    while chars.next_if(|&c| class_of(c) == Class::Space).is_some() {
        i -= 1;
    }

    if let Some(&c) = chars.peek() {
        let class = class_of(c);

        while chars.next_if(|&c| class_of(c) == class).is_some() {
            i -= 1;
        }
    }

    text.line_start(line) + i
}

/// 返回 `pos` 向右移动一个词后的位置：先跳过当前字符所属类别的字符，再跳过空白。
///
/// 在行末时等同于 [`next_grapheme`]，即跳到下一行行首。
pub fn next_word(text: &impl Lines, pos: usize) -> usize {
    let (line, col) = text.pos_to_line_col(pos);
    let content = text.line_content(line);
    let slice = content.slice();

    if col >= slice.len_chars() {
        return next_grapheme(text, pos);
    }

    let class = class_of(slice.char(col));
    let mut chars = slice.chars_at(col).peekable();
    let mut i = col;

    if class != Class::Space {
        while chars.next_if(|&c| class_of(c) == class).is_some() {
            i += 1;
        }
    }

    while chars.next_if(|&c| class_of(c) == Class::Space).is_some() {
        i += 1;
    }

    text.line_start(line) + i
}

/// 返回双击选词的位置范围 `(起点, 终点)`，终点不含。
///
/// 范围是包含 `pos` 的同类字符区间。`pos` 紧跟在单词字符之后、且当前字符不是单词字符时，选中
/// 左侧的词。
pub fn word_at(text: &impl Lines, pos: usize) -> (usize, usize) {
    let (line, col) = text.pos_to_line_col(pos);
    let content = text.line_content(line);
    let slice = content.slice();
    let len = slice.len_chars();
    let base = text.line_start(line);

    if len == 0 {
        return (base, base);
    }

    let at_word_end = col > 0 && class_of(slice.char(col - 1)) == Class::Word;
    let at = if col == len || (at_word_end && class_of(slice.char(col)) != Class::Word) {
        col - 1
    } else {
        col
    };
    let class = class_of(slice.char(at));
    let before = slice
        .chars_at(at)
        .reversed()
        .take_while(|&c| class_of(c) == class)
        .count();
    let after = slice
        .chars_at(at)
        .take_while(|&c| class_of(c) == class)
        .count();

    (base + at - before, base + at + after)
}

/// 返回上下移动 `delta` 行后的（位置，目标显示列）。
///
/// `goal` 是连续上下移动时保持的目标显示列，为 `None` 时取 `pos` 当前的显示列。`delta` 为负时
/// 向上移动；越过首行得到位置 0，越过末行得到文本末尾。结果位置会移动到字素簇边界。
pub fn vertical(
    text: &impl Lines,
    pos: usize,
    delta: isize,
    goal: Option<usize>,
    layout: impl CellLayout,
) -> (usize, usize) {
    let (line, col) = text.pos_to_line_col(pos);
    let goal = goal.unwrap_or_else(|| layout.col_to_cell(text.line_content(line).slice(), col));
    let target = line as isize + delta;

    if target < 0 {
        return (0, goal);
    }

    let target = target as usize;

    if target >= text.line_count() {
        return (text.end_pos(), goal);
    }

    let col = layout.cell_to_col(text.line_content(target).slice(), goal);
    (
        snap_to_grapheme(text, text.line_col_to_pos(target, col)),
        goal,
    )
}

/// 返回「智能行首」的目标位置（`char` 偏移）。
///
/// 光标不在首个非空白字符时移到该字符；已在该字符时移到行首（第 0 列）。
pub fn smart_home(text: &impl Lines, pos: usize) -> usize {
    let (line, col) = text.pos_to_line_col(pos);
    let indent = leading_whitespace(text.line_content(line).slice());
    let start = text.line_start(line);

    if col == indent { start } else { start + indent }
}

/// 返回光标紧邻的括号及与之配对的另一半的位置（较小者在前）。
///
/// 依次检查光标左侧和右侧的字符，只认 `()`、`[]`、`{}` 三种，不区分字符串和注释中的括号。
/// 向前或向后最多扫描 `limit` 个字符，超出时返回 `None`。
pub fn matching_bracket(text: &impl Lines, pos: usize, limit: usize) -> Option<(usize, usize)> {
    let (line, col) = text.pos_to_line_col(pos);
    let content = text.line_content(line);
    let slice = content.slice();
    let candidates = [col.checked_sub(1), (col < slice.len_chars()).then_some(col)];

    for at in candidates.into_iter().flatten() {
        let (open, close, forward) = match slice.char(at) {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => continue,
        };
        let base = text.line_start(line);

        let found = if forward {
            scan_forward(text, line, at, open, close, limit)
        } else {
            scan_backward(text, line, at, open, close, limit)
        };

        if let Some(other) = found {
            let here = base + at;
            return Some((here.min(other), here.max(other)));
        }
    }

    None
}

/// 返回从第 `line` 行第 `col` 列的左括号向后找到的配对右括号的位置。
///
/// 最多扫描 `limit` 个字符，每行的换行符也算一个，因此空行同样消耗预算；超出或找不到时返回
/// `None`。
fn scan_forward(
    text: &impl Lines,
    line: usize,
    col: usize,
    open: char,
    close: char,
    limit: usize,
) -> Option<usize> {
    let (mut depth, mut budget) = (0i32, limit);

    for l in line..text.line_count() {
        budget = budget.checked_sub(1)?;
        let content = text.line_content(l);
        let skip = if l == line { col } else { 0 };

        for (i, c) in content.slice().chars_at(skip).enumerate() {
            budget = budget.checked_sub(1)?;
            depth += i32::from(c == open) - i32::from(c == close);

            if depth == 0 {
                return Some(text.line_start(l) + skip + i);
            }
        }
    }

    None
}

/// 返回从第 `line` 行第 `col` 列的右括号向前找到的配对左括号的位置。
///
/// 预算与 [`scan_forward`] 相同，超出或找不到时返回 `None`。
fn scan_backward(
    text: &impl Lines,
    line: usize,
    col: usize,
    open: char,
    close: char,
    limit: usize,
) -> Option<usize> {
    let (mut depth, mut budget) = (0i32, limit);

    for l in (0..=line).rev() {
        budget = budget.checked_sub(1)?;
        let content = text.line_content(l);
        let slice = content.slice();
        let end = if l == line {
            col + 1
        } else {
            slice.len_chars()
        };

        for (i, c) in slice.chars_at(end).reversed().enumerate() {
            budget = budget.checked_sub(1)?;
            depth += i32::from(c == close) - i32::from(c == open);

            if depth == 0 {
                return Some(text.line_start(l) + end - 1 - i);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests;
