//! 改尺寸时重排主屏：按软换行把行连成逻辑行，再按新的宽度重新折开。
//!
//! 屏幕变矮时顶上的行进回滚缓冲，变高时从回滚缓冲拉回；光标跟着所在的字走。没有折行、
//! 也比新宽度短的行原样留下，只有需要重新折行的逻辑行才逐格处理。

use std::collections::VecDeque;

use super::history::History;
use super::row::{Cell, Extra, Row, SPACER, WIDE, line_cells};
use crate::protocol::{Attrs, Color, Line, Marks};

/// 重排的结果。
pub(super) struct Reflowed {
    /// 重排后放进回滚缓冲的行。
    pub history: Vec<Line>,
    /// 正好 `rows` 行。
    pub rows: Vec<Row>,
    /// 光标在新屏幕里的行号和列号，均从 0 起。
    ///
    /// 列等于 `cols` 表示光标停在写满的行尾、等着折行（见 [`reflow`] 的 `pending`）。
    pub cursor: (usize, usize),
    /// `history` 中开头有多少行属于清屏前的内容（见 [`reflow`] 的 `cleared`）。
    pub cleared: usize,
}

/// 重排中的一项：原样留下的回滚行，或一行格子。
enum Item {
    /// 回滚缓冲里原样留下的行。
    Line(Line),
    /// 屏幕上的行（重排前或折行后的）。
    Row(Row),
}

/// 按新宽度重排主屏，返回新的回滚缓冲、屏幕和光标。
///
/// `screen` 是屏幕上从第一行到最后一个有内容的行（光标所在行更靠下时到光标所在行），`cursor`
/// 是光标在其中的行号和列号；返回的屏幕正好 `rows` 行。`pending` 为真时光标停在写满的行尾
/// 等待折行，重排后该字仍在行尾时保持等待。`history` 开头的 `cleared` 行属于清屏之前的内容，
/// 不拉回屏幕。
#[allow(clippy::too_many_arguments)]
pub(super) fn reflow(
    history: VecDeque<Line>,
    screen: Vec<Row>,
    cursor: (usize, usize),
    pending: bool,
    cleared: usize,
    old_cols: usize,
    cols: usize,
    rows: usize,
) -> Reflowed {
    let mut items: Vec<Item> = Vec::with_capacity(history.len() + screen.len());
    let mut group: Vec<Row> = Vec::new();
    let mut group_cursor = None;
    let mut cursor_at = None;
    // 清屏前的内容重排后占 `items` 开头的多少项；跨过分界的逻辑行算在分界之后。
    let mut floor = 0;

    let sources = history.into_iter().map(Source::Line).chain(
        screen
            .into_iter()
            .enumerate()
            .map(|(i, row)| Source::Row(i, row)),
    );

    for (index, source) in sources.enumerate() {
        if index == cleared {
            floor = items.len();
        }
        let row = match source {
            Source::Line(line) => {
                if group.is_empty() && !line.wrapped && line_cells(&line) <= cols {
                    items.push(Item::Line(line));
                    continue;
                }
                Row::from_line(&line, old_cols)
            }
            Source::Row(index, row) => {
                if index == cursor.0 {
                    group_cursor = Some((group.len(), cursor.1));
                }
                row
            }
        };
        let wrapped = row.wrapped;
        group.push(row);
        if !wrapped {
            flush(
                &mut group,
                &mut group_cursor,
                pending,
                cols,
                &mut items,
                &mut cursor_at,
            );
        }
    }
    flush(
        &mut group,
        &mut group_cursor,
        pending,
        cols,
        &mut items,
        &mut cursor_at,
    );

    // 屏幕放最后 `rows` 行，光标必须在里面。
    let total = items.len();
    let (cursor_item, cursor_col) = cursor_at.unwrap_or((total.saturating_sub(1), 0));
    let floor = floor.min(cursor_item);
    let mut start = total.saturating_sub(rows).min(cursor_item).max(floor);
    if cursor_item >= start + rows {
        start = cursor_item + 1 - rows;
    }

    let mut items = items.into_iter();
    let history = items
        .by_ref()
        .take(start)
        .map(|item| match item {
            Item::Line(line) => line,
            Item::Row(row) => row.to_line(),
        })
        .collect();
    let mut screen: Vec<Row> = items
        .take(rows)
        .map(|item| match item {
            Item::Line(line) => Row::from_line(&line, cols),
            Item::Row(row) => row,
        })
        .collect();
    screen.resize_with(rows, || Row::new(cols, Color::Default));

    Reflowed {
        history,
        rows: screen,
        cursor: (cursor_item - start, cursor_col.min(cols)),
        cleared: floor,
    }
}

/// 宽度不变、只改行数时，按 [`reflow`] 的规则在屏幕和回滚缓冲之间移动行。
///
/// 回滚缓冲里其余的行不动，面板拖动时每次调整都走这条路径，不必把整份回滚缓冲重新过一遍。
/// 返回正好 `rows` 行的屏幕和光标，光标列等于 `cols` 表示等着折行。
///
/// 绝对行号小于 `cleared` 的行属于清屏前的内容，不拉回屏幕。
pub(super) fn resize_rows(
    history: &mut History,
    mut screen: Vec<Row>,
    cursor: (usize, usize),
    cleared: u64,
    cols: usize,
    rows: usize,
) -> (Vec<Row>, (usize, usize)) {
    let (first, last) = history.range();
    let kept = (last - first) as usize;
    let cursor_item = kept + cursor.0;
    let floor = cleared.clamp(first, last) - first;
    let start = (kept + screen.len())
        .saturating_sub(rows)
        .min(cursor_item)
        .max(floor as usize);
    if start < kept {
        let mut pulled: Vec<Row> = (start..kept)
            .map_while(|_| history.pop())
            .map(|line| Row::from_line(&line, cols))
            .collect();
        pulled.reverse();
        pulled.append(&mut screen);
        screen = pulled;
    } else {
        for row in screen.drain(..start - kept) {
            history.push_row(&row);
        }
    }
    screen.truncate(rows);
    screen.resize_with(rows, || Row::new(cols, Color::Default));
    (screen, (cursor_item - start, cursor.1.min(cols)))
}

/// 重排的输入项：回滚缓冲里的一行，或屏幕上的一行。
enum Source {
    /// 回滚缓冲里的一行。
    Line(Line),
    /// 屏幕上的行，带行号（从 0 起）。
    Row(usize, Row),
}

/// 把一个逻辑行（`group`，最后一行没有软换行）按 `cols` 重新折开，放进 `items`。
///
/// 逻辑行的标记合到折开后的第一行上。光标在逻辑行末尾且 `pending` 为真时停在最后一行的行尾
/// （列等于 `cols`），不另起一行。
fn flush(
    group: &mut Vec<Row>,
    group_cursor: &mut Option<(usize, usize)>,
    pending: bool,
    cols: usize,
    items: &mut Vec<Item>,
    cursor_at: &mut Option<(usize, usize)>,
) {
    if group.is_empty() {
        return;
    }
    // 长命令折了行时，`B`、`C` 等标记可能记在后面的行上，因此标记先全部合到一起，再统一放到
    // 折开后的第一行。
    let mut marks = group[0].marks;
    for row in &group[1..] {
        marks.flags |= row.marks.flags;
        if row.marks.flags & Marks::DONE != 0 {
            marks.exit = row.marks.exit;
        }
        if row.marks.flags & Marks::PROMPT != 0 && marks.block == 0 {
            marks.block = row.marks.block;
        }
    }
    let cursor = group_cursor.take();

    // 逐格拼起来：去掉换行时留下的占位空格，最后一行去掉行尾的空白（光标停在空白里时留到光标）。
    let mut cells: Vec<(Cell, Option<Extra>)> = Vec::new();
    let mut cursor_offset = None;
    let last = group.len() - 1;
    for (i, row) in group.iter_mut().enumerate() {
        let cursor_col = cursor.filter(|&(r, _)| r == i).map(|(_, c)| c);
        let mut end = if i == last {
            row.content_len()
        } else {
            row.cells.len()
        };
        if let Some(col) = cursor_col {
            end = end.max(col.min(row.cells.len()));
        }
        for col in 0..end {
            if cursor_col == Some(col) {
                cursor_offset = Some(cells.len());
            }
            let mut cell = row.cells[col];
            if cell.attrs.has(Attrs::FILLER) && i != last {
                continue;
            }
            let extra = (cell.extra != 0)
                .then(|| std::mem::take(&mut row.extras[usize::from(cell.extra) - 1]));
            cell.extra = 0;
            cells.push((cell, extra));
        }
        if cursor_col.is_some_and(|c| c >= end) {
            cursor_offset = Some(cells.len());
        }
    }
    group.clear();

    let first = items.len();
    let mut row = Row::new(cols, Color::Default);
    let mut col: usize = 0;
    let mut out_cursor = None;

    for (k, (cell, extra)) in cells.into_iter().enumerate() {
        if cell.attrs.has(SPACER) {
            if cursor_offset == Some(k) {
                out_cursor = Some((items.len() - first, col.saturating_sub(1)));
            }
            continue;
        }
        let width = if cell.attrs.has(WIDE) { 2 } else { 1 };
        if width > cols {
            continue;
        }
        if col + width > cols {
            if col < cols {
                row.cells[col] = Cell {
                    attrs: Attrs(Attrs::FILLER),
                    ..Cell::blank(Color::Default)
                };
            }
            row.wrapped = true;
            items.push(Item::Row(std::mem::replace(
                &mut row,
                Row::new(cols, Color::Default),
            )));
            col = 0;
        }
        if cursor_offset == Some(k) {
            out_cursor = Some((items.len() - first, col));
        }
        let mut cell = cell;
        if let Some(extra) = extra {
            row.extras.push(extra);
            cell.extra = row.extras.len() as u16;
        }
        row.cells[col] = cell;
        if width == 2 {
            row.cells[col + 1] = Cell {
                ch: ' ',
                attrs: Attrs((cell.attrs.0 & !WIDE) | SPACER),
                extra: 0,
                ..cell
            };
        }
        col += width;
    }
    if cursor_offset.is_some() && out_cursor.is_none() {
        // 光标在逻辑行的末尾。
        if col >= cols && !pending {
            row.wrapped = true;
            items.push(Item::Row(std::mem::replace(
                &mut row,
                Row::new(cols, Color::Default),
            )));
            col = 0;
        }
        out_cursor = Some((items.len() - first, col.min(cols)));
    }
    items.push(Item::Row(row));

    if let Some(Item::Row(head)) = items.get_mut(first) {
        head.marks = marks;
    }
    if let Some((r, c)) = out_cursor {
        *cursor_at = Some((first + r, c));
    }
}
