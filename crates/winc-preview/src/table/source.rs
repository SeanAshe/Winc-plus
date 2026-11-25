//! 分隔符文本中的位置：数据行与源文本行的对应，以及一行数据中各字段的起点。
//!
//! 分栏时表格与左侧的文字联动（同步滚动、光标与选中的单元格互相跟随）用到这些位置。字段的
//! 划分与读表时相同（RFC 4180）：引号只在字段开头时括起字段，两个双引号表示一个双引号。

use super::Sheet;

impl Sheet {
    /// 返回第 `row` 行数据（从 0 起）在源文本中开始的行号（从 0 起）。
    ///
    /// 引号内的字段跨行时，其后各行数据的起始行号大于数据行号；工作簿中的表与数据行号相同。
    pub fn source_line(&self, row: usize) -> usize {
        let at = self.shifts.partition_point(|&(r, _)| r as usize <= row);
        let shift = at.checked_sub(1).map_or(0, |i| self.shifts[i].1);
        row + shift as usize
    }

    /// 返回源文本第 `line` 行（从 0 起）所在的数据行，不超过最后一行：一行数据跨几行文字时，
    /// 其中各行都对应这一行数据。
    pub fn row_at_line(&self, line: usize) -> usize {
        let at = (self.shifts).partition_point(|&(r, s)| (r + s) as usize <= line);
        let shift = at.checked_sub(1).map_or(0, |i| self.shifts[i].1 as usize);
        let next = self.shifts.get(at).map_or(usize::MAX, |&(r, _)| r as usize);
        let last = self.row_count().saturating_sub(1);
        line.saturating_sub(shift)
            .min(next.saturating_sub(1))
            .min(last)
    }
}

/// 返回一行数据中第 `column` 个字段（从 0 起）开头的字符偏移，相对于这行数据的开头；这行数据
/// 没有那么多字段时返回 `None`。
///
/// `chars` 从这行数据的第一个字符开始，到不在引号内的行尾为止，之后的字符不读。
pub fn field_start(
    chars: impl IntoIterator<Item = char>,
    delimiter: char,
    column: usize,
) -> Option<usize> {
    if column == 0 {
        return Some(0);
    }
    let mut found = None;
    fields(chars, delimiter, |index, start| {
        if index == column {
            found = Some(start);
            return false;
        }
        true
    });
    found
}

/// 返回字符偏移 `offset`（相对于这行数据的开头）所在的字段序号（从 0 起）；分隔符属于其前面
/// 的字段，行尾之后的偏移属于最后一个字段。规则见 [`field_start`]。
pub fn field_at(chars: impl IntoIterator<Item = char>, delimiter: char, offset: usize) -> usize {
    let mut column = 0;
    fields(chars, delimiter, |index, start| {
        if start > offset {
            return false;
        }
        column = index;
        true
    });
    column
}

/// 依次对第 1 个起的各字段调用 `f(字段序号, 起点的字符偏移)`，`f` 返回 `false` 时停止；读到
/// 不在引号内的行尾时停止。
fn fields(
    chars: impl IntoIterator<Item = char>,
    delimiter: char,
    mut f: impl FnMut(usize, usize) -> bool,
) {
    let (mut index, mut empty, mut quoted, mut closing) = (0, true, false, false);
    for (i, c) in chars.into_iter().enumerate() {
        if closing {
            closing = false;
            if c == '"' {
                continue;
            }
            quoted = false;
        }
        if quoted {
            closing = c == '"';
            continue;
        }
        match c {
            '"' if empty => quoted = true,
            '\n' | '\r' => return,
            c if c == delimiter => {
                index += 1;
                empty = true;
                if !f(index, i + 1) {
                    return;
                }
                continue;
            }
            _ => {}
        }
        empty = false;
    }
}

#[cfg(test)]
mod tests;
