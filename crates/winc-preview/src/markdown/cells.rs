//! 表格原文中的单元格：可编辑的预览中光标所在的单元格显示原文，Tab、⇧Tab、回车在单元格之间
//! 移动，在最后一行继续向下时新增一行，连续录入无需切换到工具栏。
//!
//! 另外还处理插入、删除行和列，修改列的对齐方式（[`change`]）。切分只用当前原文（GFM 的
//! 规则：`|` 分格，`\|` 不分；两头的 `|` 可有可无），不依赖解析结果，输入快于后台解析时也正确。

use std::ops::Range;

/// 一行里的一个单元格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// 两个 `|` 之间的范围（字节偏移，相对这一行）。
    pub segment: Range<usize>,
    /// 内容（去掉两边的空白）的范围（字节偏移，相对这一行）。
    ///
    /// 空单元格是内容应当出现的位置（左侧空白之后）的空范围。
    pub content: Range<usize>,
}

/// 切分表格的一行 `line`（不含换行符），返回这一行的各个单元格。
pub fn cells(line: &str) -> Vec<Cell> {
    segments(line)
        .map(|segment| {
            let (start, end) = (segment.start, segment.end);
            let inner = &line[start..end];
            let left = inner.len() - inner.trim_start().len();
            let content = if inner.trim().is_empty() {
                let at = (start + 1).min(end);
                at..at
            } else {
                start + left..start + inner.trim_end().len()
            };
            Cell { segment, content }
        })
        .collect()
}

/// 是否为表头下面那一行（`|---|:--:|`）。
pub fn is_delimiter(line: &str) -> bool {
    let mut segments = segments(line).peekable();
    segments.peek().is_some()
        && segments.all(|segment| {
            let t = line[segment].trim();
            let t = t.strip_prefix(':').unwrap_or(t);
            let t = t.strip_suffix(':').unwrap_or(t);
            !t.is_empty() && t.bytes().all(|b| b == b'-')
        })
}

/// 依次返回 `line` 中各单元格两个 `|` 之间的范围（字节偏移），不分配内存。
///
/// 开头没有 `|` 时第一格从左侧空白之后起，末尾没有 `|` 时最后一格到右侧空白之前止。
fn segments(line: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let bytes = line.as_bytes();
    let lead = line.len() - line.trim_start().len();
    let trail = line.trim_end().len();
    // `piped`：当前一格的起点紧跟在 `|` 之后。
    let mut piped = bytes.get(lead) == Some(&b'|');
    let mut start = Some(lead + usize::from(piped));
    let (mut at, mut escaped) = (lead + usize::from(piped), false);
    std::iter::from_fn(move || {
        let from = start?;
        while let Some(&b) = bytes.get(at) {
            at += 1;
            if b == b'|' && !escaped {
                (start, piped) = (Some(at), true);
                return Some(from..at - 1);
            }
            escaped = b == b'\\' && !escaped;
        }
        start = None;
        (!piped || from != trail).then(|| from..trail.max(from))
    })
}

/// 光标在单元格之间移动的一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Tab 键：移到下一格，行末时移到下一行的第一格。
    Next,
    /// ⇧Tab 键：移到上一格。
    Previous,
    /// 回车键：移到下一行的同一列。
    Down,
}

/// 走一步的结果：要插入的行和要走到的单元格。
///
/// 先在 `insert` 处插入一行（在最后一行继续向下时），再选中 `select`（插入之后的偏移，空
/// 单元格是一个位置）。偏移都相对表格原文 `text` 的起点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 要插入的新行：插入位置和文字（以换行符开头）。
    pub insert: Option<(usize, String)>,
    /// 插入之后要选中的单元格内容。
    pub select: Range<usize>,
}

/// 返回光标在表格原文 `text` 的 `caret` 处（字节偏移）按 `step` 走到的单元格。
///
/// 光标在表头下面那一行或不在任何一格中时返回 `None`。
pub fn step(text: &str, caret: usize, step: Step) -> Option<Target> {
    // 各行：起点、单元格；跳过表头下面那一行和末尾的空行。
    let mut rows: Vec<(usize, Vec<Cell>)> = Vec::new();
    let mut here = None;
    let mut start = 0;
    for line in text.split('\n') {
        let end = start + line.len();
        let on_line = (start..=end).contains(&caret);
        if is_delimiter(line) {
            if on_line {
                return None;
            }
        } else if !line.trim().is_empty() {
            let cells = cells(line);
            if on_line {
                let col = cells
                    .iter()
                    .position(|c| caret - start <= c.segment.end)
                    .unwrap_or(cells.len().saturating_sub(1));
                here = Some((rows.len(), col));
            }
            rows.push((start, cells));
        }
        start = end + 1;
    }
    let (row, col) = here?;
    let at = |r: usize, c: usize| {
        let (offset, cells) = &rows[r];
        let cell = &cells[c.min(cells.len() - 1)];
        offset + cell.content.start..offset + cell.content.end
    };
    let cols = rows[row].1.len();
    let next_row = rows.get(row + 1).map(|_| row + 1);
    let target = match step {
        Step::Next if col + 1 < cols => at(row, col + 1),
        Step::Next => match next_row {
            Some(r) => at(r, 0),
            None => return Some(append(text, &rows[row], 0)),
        },
        Step::Previous if col > 0 => at(row, col - 1),
        Step::Previous if row > 0 => at(row - 1, rows[row - 1].1.len() - 1),
        Step::Previous => at(row, 0),
        Step::Down => match next_row {
            Some(r) => at(r, col),
            None => return Some(append(text, &rows[row], col)),
        },
    };
    Some(Target {
        insert: None,
        select: target,
    })
}

/// 在最后一行 `last` 之后添加一行同样多格的空行，并返回光标移到第 `col` 格的 `Target`。
fn append(text: &str, (offset, cells): &(usize, Vec<Cell>), col: usize) -> Target {
    let end = cells.last().map_or(*offset, |c| offset + c.segment.end + 1);
    let end = text[*offset..]
        .find('\n')
        .map_or(text.len(), |i| offset + i)
        .max(end.min(text.len()));
    // 新行以 `\n|` 开头，之后每格是两个空格加一个 `|`：第 `col` 格的内容从 `\n` 起第
    // `3 + 3 * col` 个字节处开始。
    let row = format!("\n|{}", "  |".repeat(cells.len().max(1)));
    let caret = end + 3 + 3 * col.min(cells.len().saturating_sub(1));
    Target {
        insert: Some((end, row)),
        select: caret..caret,
    }
}

/// 对表格的一种修改，对应表格工具栏和右键菜单中的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// 在光标所在的行上方插入一行（表头上方不能插入）。
    RowAbove,
    /// 在光标所在的行下方插入一行（光标在表头上时插入到分隔行之后）。
    RowBelow,
    /// 在光标所在的列左侧插入一列。
    ColumnLeft,
    /// 在光标所在的列右侧插入一列。
    ColumnRight,
    /// 删除光标所在的行（表头不能删除）。
    DeleteRow,
    /// 删除光标所在的列（只剩一列时不能删除）。
    DeleteColumn,
    /// 将光标所在的列设为指定的对齐方式，即修改分隔行中的 `:`。
    Align(super::Align),
}

/// 按 `change` 修改表格原文 `text`（光标位于 `caret`，字节偏移），返回修改后的原文和光标
/// 的新位置（字节偏移）。
///
/// 插入后光标位于新单元格开头，修改对齐或删除后留在同一或相邻单元格。光标位于分隔行上、
/// 不在任何单元格中、或修改无效时返回 `None`。只修改表格本身的行（至第一个空行为止）。
pub fn change(text: &str, caret: usize, change: Change) -> Option<(String, usize)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let starts: Vec<usize> = lines
        .iter()
        .scan(0, |at, line| {
            let start = *at;
            *at += line.len() + 1;
            Some(start)
        })
        .collect();
    let end = lines
        .iter()
        .position(|l| l.trim().is_empty())
        .unwrap_or(lines.len());
    let delim = lines[..end].iter().position(|l| is_delimiter(l))?;
    let here = (0..end).find(|&i| (starts[i]..=starts[i] + lines[i].len()).contains(&caret))?;
    if here == delim {
        return None;
    }
    let row = cells(lines[here]);
    let at = caret - starts[here];
    let col = row
        .iter()
        .position(|c| at <= c.segment.end)
        .unwrap_or(row.len().checked_sub(1)?);
    let content = &row.get(col)?.content;
    let offset = at.saturating_sub(content.start).min(content.len());
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    // 修改后光标所在的行号、单元格序号，以及单元格内容中的字节偏移。
    let (line, col, offset) = match change {
        Change::RowAbove | Change::RowBelow => {
            let below = change == Change::RowBelow;
            let at = match (below, here) {
                (false, 0) => return None,
                (true, 0) => delim + 1,
                (true, _) => here + 1,
                (false, _) => here,
            };
            let cols = cells(lines[delim]).len().max(row.len());
            out.insert(at, format!("|{}", "  |".repeat(cols)));
            (at, col, 0)
        }
        Change::DeleteRow => {
            if here == 0 {
                return None;
            }
            out.remove(here);
            // 光标留在下一行的同一单元格；删除的是最后一行时移到上一行（跳过分隔行）。
            let line = if here + 1 < end { here } else { here - 1 };
            (if line == delim { 0 } else { line }, col, 0)
        }
        Change::ColumnLeft | Change::ColumnRight => {
            let new = col + usize::from(change == Change::ColumnRight);
            for (i, line) in out[..end].iter_mut().enumerate() {
                let fill = if i == delim { "---" } else { "  " };
                add_column(line, new, fill);
            }
            (here, new, 0)
        }
        Change::DeleteColumn => {
            if row.len() <= 1 {
                return None;
            }
            for line in &mut out[..end] {
                let Some(cell) = cells(line).into_iter().nth(col) else {
                    continue;
                };
                let seg = cell.segment;
                // 连同右侧的 `|` 一起删除；最后一个单元格右侧没有 `|` 时删除左侧的 `|`。
                let range = if line[seg.end..].starts_with('|') {
                    seg.start..seg.end + 1
                } else {
                    seg.start.saturating_sub(1)..seg.end
                };
                line.replace_range(range, "");
            }
            (here, col.min(row.len() - 2), 0)
        }
        Change::Align(align) => {
            let line = &mut out[delim];
            let seg = cells(line).into_iter().nth(col)?.segment;
            let inner = &line[seg.clone()];
            let dashes = inner.bytes().filter(|&b| b == b'-').count().max(3);
            let lead = &inner[..inner.len() - inner.trim_start().len()];
            let trail = &inner[inner.trim_end().len()..];
            let dash = "-".repeat(dashes);
            let mark = match align {
                super::Align::None => dash,
                super::Align::Left => format!(":{dash}"),
                super::Align::Center => format!(":{dash}:"),
                super::Align::Right => format!("{dash}:"),
            };
            let cell = format!("{lead}{mark}{trail}");
            line.replace_range(seg, &cell);
            (here, col, offset)
        }
    };
    let start: usize = out[..line].iter().map(|l| l.len() + 1).sum();
    let cell = cells(&out[line]).into_iter().nth(col)?;
    let caret = start + cell.content.start + offset.min(cell.content.len());
    Some((out.join("\n"), caret))
}

/// 在表格的一行 `line` 中插入内容为 `fill` 的单元格，作为第 `col` 个单元格。
fn add_column(line: &mut String, col: usize, fill: &str) {
    let row = cells(line);
    let Some(last) = row.last() else {
        return;
    };
    match row.get(col) {
        // 第一个单元格前没有 `|` 时补上，否则新单元格会与原来的第一个单元格合并。
        Some(cell) if col == 0 && !line.trim_start().starts_with('|') => {
            line.insert_str(cell.segment.start, &format!("|{fill}|"));
        }
        Some(cell) => line.insert_str(cell.segment.start, &format!("{fill}|")),
        // 插入到末尾：最后一个单元格右侧有 `|` 时插入到该 `|` 之后。
        None if line[last.segment.end..].starts_with('|') => {
            line.insert_str(last.segment.end + 1, &format!("{fill}|"));
        }
        None => line.insert_str(last.segment.end, &format!("|{fill}")),
    }
}

/// 把制表符分隔的文字 `text` 写成 GFM 表格，第一行作表头；不符合时返回 `None`。
///
/// 要求至少两行、每行列数相同、表头没有空单元格；单元格中的 `|` 转义，不支持多行单元格。
pub fn from_tsv(text: &str) -> Option<String> {
    let rows: Vec<Vec<&str>> = text
        .trim_end_matches(['\n', '\r'])
        .lines()
        .map(|line| line.trim_end_matches('\r').split('\t').collect())
        .collect();
    let columns = rows.first()?.len();
    if rows.len() < 2 || columns < 2 || rows.iter().any(|row| row.len() != columns) {
        return None;
    }
    // 表头有空单元格时不作为表格，排除用制表符缩进的代码。
    if rows[0].iter().any(|c| c.trim().is_empty()) {
        return None;
    }
    let line = |cells: Vec<String>| format!("| {} |", cells.join(" | "));
    let row = |cells: &[&str]| line(cells.iter().map(|c| c.trim().replace('|', "\\|")).collect());
    let mut out = row(&rows[0]);
    out.push('\n');
    out.push_str(&line(vec!["---".to_owned(); columns]));
    for cells in &rows[1..] {
        out.push('\n');
        out.push_str(&row(cells));
    }
    out.push('\n');
    Some(out)
}

#[cfg(test)]
mod tests;
