//! 文本查询：在 [`Lines`] 之上提供行、字符分类、显示列的换算，以及逐字符移动的扫描位置。
//!
//! 位置都是 `char` 偏移。行内的位置分两种：字符上的位置，以及行尾（最后一个字符之后、换行符
//! 之前）。普通模式的光标只在字符上，空行除外；动作的中间结果和操作符的范围可以落在行尾。

use winc_core::text::char_cells;
use winc_core::{Line, Lines, movement};

/// 返回行数。文本以换行符结尾时，末尾的空行不算一行，与按行保存的文件一致。
pub(crate) fn line_count(text: &impl Lines) -> usize {
    let count = text.line_count();
    if count > 1 && text.line_start(count - 1) == text.end_pos() {
        count - 1
    } else {
        count
    }
}

/// 返回最后一行的行号。
pub(crate) fn last_line(text: &impl Lines) -> usize {
    line_count(text) - 1
}

/// 返回 `pos` 所在的行号，不超过最后一行。
pub(crate) fn line_of(text: &impl Lines, pos: usize) -> usize {
    text.line_of(pos).min(last_line(text))
}

/// 返回 `pos` 之后一个字素的位置，不越过行尾；`pos` 已在行尾时返回 `None`。
pub(crate) fn next_in_line(text: &impl Lines, pos: usize) -> Option<usize> {
    let end = text.line_end(text.line_of(pos));
    (pos < end).then(|| movement::next_grapheme(text, pos).min(end))
}

/// 返回 `pos` 之前一个字素的位置，不越过行首；`pos` 已在行首时返回 `None`。
pub(crate) fn prev_in_line(text: &impl Lines, pos: usize) -> Option<usize> {
    let start = text.line_start(text.line_of(pos));
    (pos > start).then(|| movement::prev_grapheme(text, pos).max(start))
}

/// 返回第 `line` 行最后一个字符的位置；空行返回行首。
pub(crate) fn last_char(text: &impl Lines, line: usize) -> usize {
    let end = text.line_end(line);
    prev_in_line(text, end).unwrap_or(end)
}

/// 把 `pos` 收拢到所在行的字符上：位于行尾之后时退到最后一个字符，并对齐到字素边界。
pub(crate) fn on_char(text: &impl Lines, pos: usize) -> usize {
    let line = line_of(text, pos);
    let pos = pos.clamp(text.line_start(line), text.line_end(line));
    movement::snap_to_grapheme(text, pos.min(last_char(text, line)))
}

/// 返回 `pos` 处字符之后的位置：行内为下一个字素，行尾为下一行行首，文本末尾不变。
pub(crate) fn after(text: &impl Lines, pos: usize) -> usize {
    match next_in_line(text, pos) {
        Some(next) => next,
        None => {
            let line = text.line_of(pos);
            if line < last_line(text) {
                text.line_start(line + 1)
            } else {
                pos
            }
        }
    }
}

/// 是否为空格或制表符。
pub(crate) fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// 返回第 `line` 行行首空白的字符数。
pub(crate) fn indent_chars(text: &impl Lines, line: usize) -> usize {
    let content = text.line_content(line);
    content.slice().chars().take_while(|c| is_blank(*c)).count()
}

/// 返回第 `line` 行第一个非空白字符的位置；整行都是空白时为最后一个字符，空行为行首。
pub(crate) fn first_non_blank(text: &impl Lines, line: usize) -> usize {
    let start = text.line_start(line);
    (start + indent_chars(text, line)).min(last_char(text, line))
}

/// 返回第 `line` 行第 `col` 个字符起始处的显示列。
pub(crate) fn cell_of(text: &impl Lines, line: usize, col: usize, tab_width: usize) -> usize {
    let content = text.line_content(line);
    let mut cell = 0;
    for c in content.slice().chars().take(col) {
        cell += char_cells(c, cell, tab_width.max(1));
    }
    cell
}

/// 返回位置 `pos` 的期望显示列：制表符取所占显示列的最后一列，其余字符取起始的显示列。
pub(crate) fn want_cell(text: &impl Lines, pos: usize, tab_width: usize) -> usize {
    let (line, col) = text.pos_to_line_col(pos);
    let cell = cell_of(text, line, col, tab_width);
    match text.char_at(pos) {
        Some('\t') if pos < text.line_end(line) => {
            cell + tab_width.max(1) - cell % tab_width.max(1) - 1
        }
        _ => cell,
    }
}

/// 返回第 `line` 行中覆盖显示列 `cell` 的字符的位置；`cell` 超过行宽时返回行尾。
pub(crate) fn pos_at_cell(text: &impl Lines, line: usize, cell: usize, tab_width: usize) -> usize {
    let content = text.line_content(line);
    let mut at = 0;
    let mut col = 0;
    for c in content.slice().chars() {
        at += char_cells(c, at, tab_width.max(1));
        if at > cell {
            break;
        }
        col += 1;
    }
    movement::snap_to_grapheme(text, text.line_start(line) + col)
}

/// 字符的类别：0 为空白，1 为标点，2 为词字符，其余各为一类文字（汉字、假名、谚文等）。
///
/// `big` 为真时只区分空白与非空白。
pub(crate) fn class(c: Option<char>, big: bool) -> u32 {
    let Some(c) = c else {
        return 0;
    };
    if is_blank(c) {
        return 0;
    }
    if big {
        return 1;
    }
    let code = u32::from(c);
    if code < 0x100 {
        // 拉丁字母补充区中 U+00C0 之后的字符与 ASCII 的字母、数字、下划线是词字符。
        return if c.is_ascii_alphanumeric() || c == '_' || code >= 0xC0 {
            2
        } else {
            1
        };
    }
    match RANGES.binary_search_by(|&(first, last, _)| {
        if code < first {
            std::cmp::Ordering::Greater
        } else if code > last {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => RANGES[index].2,
        Err(_) => 2,
    }
}

/// 表情符号所属的类别。
const EMOJI: u32 = 3;

/// 码位区间及其类别，按码位排序；未列出的字符是词字符。
///
/// 类别的划分依据录制基准 `vim` 9.1 对各 Unicode 区块的归类：空白、标点与符号、
/// 各自成类的文字。
const RANGES: &[(u32, u32, u32)] = &[
    (0x037E, 0x037E, 1),
    (0x0387, 0x0387, 1),
    (0x055A, 0x055F, 1),
    (0x0589, 0x0589, 1),
    (0x05BE, 0x05BE, 1),
    (0x05C0, 0x05C0, 1),
    (0x05C3, 0x05C3, 1),
    (0x05F3, 0x05F4, 1),
    (0x060C, 0x060C, 1),
    (0x061B, 0x061B, 1),
    (0x061F, 0x061F, 1),
    (0x066A, 0x066D, 1),
    (0x06D4, 0x06D4, 1),
    (0x0964, 0x0965, 1),
    (0x0E5A, 0x0E5B, 1),
    (0x1680, 0x1680, 0),
    (0x169B, 0x169C, 1),
    (0x16EB, 0x16ED, 1),
    (0x1735, 0x1736, 1),
    (0x17D4, 0x17DC, 1),
    (0x1800, 0x180A, 1),
    (0x2000, 0x200B, 0),
    (0x200C, 0x2027, 1),
    (0x2028, 0x2029, 0),
    (0x202A, 0x202E, 1),
    (0x202F, 0x202F, 0),
    (0x2030, 0x205E, 1),
    (0x205F, 0x205F, 0),
    (0x2060, 0x206F, 1),
    (0x2070, 0x207F, 0x2070),
    (0x2080, 0x2094, 0x2080),
    (0x20A0, 0x27FF, 1),
    (0x2800, 0x28FF, 0x2800),
    (0x2900, 0x2998, 1),
    (0x29D8, 0x29DB, 1),
    (0x29FC, 0x29FD, 1),
    (0x2E00, 0x2E7F, 1),
    (0x3000, 0x3000, 0),
    (0x3001, 0x3020, 1),
    (0x3040, 0x309F, 0x3040),
    (0x30A0, 0x30FF, 0x30A0),
    (0x3300, 0x9FFF, 0x4E00),
    (0xAC00, 0xD7A3, 0xAC00),
    (0xF900, 0xFAFF, 0x4E00),
    (0xFD3E, 0xFD3F, 1),
    (0xFE50, 0xFE6B, 1),
    (0xFF00, 0xFF0F, 1),
    (0xFF1A, 0xFF20, 1),
    (0xFF3B, 0xFF40, 1),
    (0xFF5B, 0xFF65, 1),
    (0x1D000, 0x1D24F, 1),
    (0x1D400, 0x1D7FF, 1),
    (0x1F000, 0x1F2FF, 1),
    (0x1F300, 0x1FAFF, EMOJI),
    (0x20000, 0x2A6DF, 0x4E00),
    (0x2A700, 0x2B81F, 0x4E00),
    (0x2F800, 0x2FA1F, 0x4E00),
];

/// [`Scan::inc`] 与 [`Scan::dec`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// 移到了同一行的另一个字符上。
    Char,
    /// 移到了同一行的行尾。
    LineEnd,
    /// 移到了另一行。
    Line,
    /// 已在文本的开头或末尾，没有移动。
    Edge,
}

impl Step {
    /// 是否到达了行尾或另一行。
    pub(crate) fn left_line(self) -> bool {
        matches!(self, Step::LineEnd | Step::Line)
    }
}

/// 逐字符移动的扫描位置：行号与行内的字符偏移，偏移等于行长时位于行尾。
///
/// 一次移动一个字素；向前越过行尾时先停在行尾，再到下一行行首，向后越过行首时到上一行行尾。
pub(crate) struct Scan<'a, T: Lines> {
    /// 被扫描的文本。
    text: &'a T,
    /// 行数（末尾的空行不计）。
    lines: usize,
    /// 当前行号。
    pub(crate) line: usize,
    /// 当前行内的字符偏移，不超过行长。
    pub(crate) col: usize,
    /// 当前行的内容。
    content: Line<'a>,
    /// 当前行的字符数。
    len: usize,
    /// 当前行的全部字符；只在行内容不是借用时建立，避免按偏移取字符时反复扫描。
    chars: Option<Vec<char>>,
}

impl<'a, T: Lines> Scan<'a, T> {
    /// 创建位于 `pos` 的扫描位置。
    pub(crate) fn new(text: &'a T, pos: usize) -> Self {
        let lines = line_count(text);
        let line = text.line_of(pos).min(lines - 1);
        let mut scan = Self {
            text,
            lines,
            line,
            col: 0,
            content: text.line_content(line),
            len: 0,
            chars: None,
        };
        scan.load();
        scan.col = pos.saturating_sub(text.line_start(line)).min(scan.len);
        scan
    }

    /// 读入当前行。
    fn load(&mut self) {
        self.content = self.text.line_content(self.line);
        self.chars = match &self.content {
            Line::Slice(_) => None,
            Line::Owned(text) => Some(text.chars().collect()),
        };
        self.len = match &self.chars {
            Some(chars) => chars.len(),
            None => self.content.slice().len_chars(),
        };
    }

    /// 移到第 `line` 行的第 `col` 个字符；行号与偏移超出范围时取最近的合法值。
    pub(crate) fn set(&mut self, line: usize, col: usize) {
        let line = line.min(self.lines - 1);
        if line != self.line {
            self.line = line;
            self.load();
        }
        self.col = col.min(self.len);
    }

    /// 返回当前位置（`char` 偏移）。
    pub(crate) fn pos(&self) -> usize {
        self.text.line_start(self.line) + self.col
    }

    /// 返回当前行的字符数。
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// 是否位于最后一行。
    pub(crate) fn on_last_line(&self) -> bool {
        self.line + 1 >= self.lines
    }

    /// 是否位于空行。
    pub(crate) fn on_empty_line(&self) -> bool {
        self.len == 0
    }

    /// 返回当前行第 `col` 个字符；`col` 不小于行长时返回 `None`。
    pub(crate) fn char_at(&self, col: usize) -> Option<char> {
        match &self.chars {
            Some(chars) => chars.get(col).copied(),
            None => self.content.slice().get_char(col),
        }
    }

    /// 返回当前位置的字符；位于行尾时返回 `None`。
    pub(crate) fn char(&self) -> Option<char> {
        self.char_at(self.col)
    }

    /// 返回当前位置字符的类别，行尾算作空白；`big` 的含义见 [`class`]。
    pub(crate) fn class(&self, big: bool) -> u32 {
        class(self.char(), big)
    }

    /// 向前移动一个字素。
    pub(crate) fn inc(&mut self) -> Step {
        if self.col < self.len {
            let next = match (self.char_at(self.col), self.char_at(self.col + 1)) {
                // 相邻的两个 ASCII 字符之间一定是字素边界。
                (Some(a), Some(b)) if a.is_ascii() && b.is_ascii() => self.col + 1,
                (_, None) => self.len,
                _ => {
                    movement::next_grapheme(self.text, self.pos()) - self.text.line_start(self.line)
                }
            };
            self.col = next.clamp(self.col + 1, self.len);
            return if self.col < self.len {
                Step::Char
            } else {
                Step::LineEnd
            };
        }
        if self.on_last_line() {
            return Step::Edge;
        }
        self.line += 1;
        self.load();
        self.col = 0;
        Step::Line
    }

    /// 向后移动一个字素。
    pub(crate) fn dec(&mut self) -> Step {
        if self.col > 0 {
            let prev = match (self.char_at(self.col - 1), self.col.checked_sub(2)) {
                (Some(a), Some(before)) if a.is_ascii() => match self.char_at(before) {
                    Some(b) if b.is_ascii() => self.col - 1,
                    _ => self.grapheme_before(),
                },
                (_, None) => 0,
                _ => self.grapheme_before(),
            };
            self.col = prev.min(self.col - 1);
            return Step::Char;
        }
        if self.line == 0 {
            return Step::Edge;
        }
        self.line -= 1;
        self.load();
        self.col = self.len;
        Step::Line
    }

    /// 返回当前位置之前一个字素在行内的偏移。
    fn grapheme_before(&self) -> usize {
        movement::prev_grapheme(self.text, self.pos()) - self.text.line_start(self.line)
    }

    /// 向前移动一个字素，并越过非空行的行尾。
    pub(crate) fn incl(&mut self) -> Step {
        let step = self.inc();
        if step.left_line() && self.col > 0 {
            return self.inc();
        }
        step
    }

    /// 向后移动一个字素，并越过非空行的行尾。
    pub(crate) fn decl(&mut self) -> Step {
        let step = self.dec();
        if step == Step::Line && self.col > 0 {
            return self.dec();
        }
        step
    }
}
