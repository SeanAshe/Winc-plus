//! 按块的可视模式：由选区的两角算出各行上的范围，以及作用于块的操作符、插入与粘贴。
//!
//! 块以显示列界定：左边界是两角字符起始列中较小的一个，右边界是两角字符末列中较大的一个；
//! 起始列落在边界之内的字符属于块。宿主以每行一个选区显示块。

use winc_core::text::char_cells;
use winc_core::{Change, Lines, Selection, movement};

use super::insert::After;
use super::{Extent, Shared, Vim, Visual, VisualKind};
use crate::command::Operator;
use crate::host::Host;
use crate::operator;
use crate::parser::Command;
use crate::register::{Content, Kind, Source};
use crate::text;

/// 块的范围。
#[derive(Debug, Clone, Copy)]
pub(super) struct Block {
    /// 首行。
    pub first: usize,
    /// 末行。
    pub last: usize,
    /// 左边界的显示列。
    pub left: usize,
    /// 右边界的显示列（含）。
    pub right: usize,
    /// 是否延伸到各行的行尾。
    pub to_end: bool,
}

/// 块在一行上的部分。
#[derive(Debug, Clone, Copy)]
pub(super) struct Row {
    /// 行号。
    pub line: usize,
    /// 起点。
    pub from: usize,
    /// 终点（不含）。
    pub to: usize,
    /// 该行是否不到块的左边界。
    pub short: bool,
}

/// 返回 `pos` 处字符占用的显示列：起始列与末列；位于行尾时占一列。
fn cells<H: Host>(host: &H, pos: usize) -> (usize, usize) {
    let text = host.text();
    let tab_width = host.tab_width().max(1);
    let (line, col) = text.pos_to_line_col(pos);
    let start = text::cell_of(text, line, col, tab_width);
    let width = match text.char_at(pos) {
        Some(c) if pos < text.line_end(line) => char_cells(c, start, tab_width).max(1),
        _ => 1,
    };
    (start, start + width - 1)
}

/// 返回可视选区 `visual` 界定的块；`to_end` 为真时块延伸到各行的行尾。
///
/// `columns` 给出时取其为左右边界的显示列，不由两角的字符决定。
pub(super) fn block_of<H: Host>(
    host: &H,
    visual: Visual,
    to_end: bool,
    columns: Option<(usize, usize)>,
) -> Block {
    let text = host.text();
    let (anchor, cursor) = (cells(host, visual.anchor), cells(host, visual.cursor));
    let (a, b) = (
        text::line_of(text, visual.anchor),
        text::line_of(text, visual.cursor),
    );
    let (left, right) = columns.unwrap_or((anchor.0.min(cursor.0), anchor.1.max(cursor.1)));
    Block {
        first: a.min(b),
        last: a.max(b),
        left,
        right,
        to_end,
    }
}

/// 返回块 `block` 在各行上的部分，按行号排列。
pub(super) fn rows_of<H: Host>(host: &H, block: Block) -> Vec<Row> {
    let text = host.text();
    let tab_width = host.tab_width().max(1);
    (block.first..=block.last)
        .map(|line| {
            let start = text.line_start(line);
            let content = text.line_content(line);
            let (mut cell, mut col) = (0, 0);
            let (mut from, mut to) = (None, None);
            for c in content.slice().chars() {
                if cell >= block.left && from.is_none() {
                    from = Some(col);
                }
                if cell > block.right && !block.to_end {
                    to = Some(col);
                    break;
                }
                cell += char_cells(c, cell, tab_width);
                col += 1;
            }
            let short = cell < block.left;
            let from = from.unwrap_or(col);
            let to = to.unwrap_or(col).max(from);
            Row {
                line,
                from: movement::snap_to_grapheme(text, start + from),
                to: movement::snap_to_grapheme(text, start + to),
                short,
            }
        })
        .collect()
}

/// 返回显示块 `visual` 的选区与其中主选区的下标；`to_end` 的含义见 [`block_of`]。
pub(super) fn selections<H: Host>(
    host: &H,
    visual: Visual,
    to_end: bool,
) -> (Vec<Selection>, usize) {
    let text = host.text();
    let cursor_line = text::line_of(text, visual.cursor);
    let leftward = cells(host, visual.cursor).0 < cells(host, visual.anchor).0;
    let mut out = Vec::new();
    let mut primary = 0;
    for row in rows_of(host, block_of(host, visual, to_end, None)) {
        // 不到左边界的行没有选区，光标所在的行除外。
        if row.short && row.line != cursor_line {
            continue;
        }
        if row.line == cursor_line {
            primary = out.len();
        }
        out.push(if leftward {
            Selection::new(row.to, row.from)
        } else {
            Selection::new(row.from, row.to)
        });
    }
    (out, primary)
}

/// 返回文本 `piece` 占用的显示列数；制表符按从第 0 列起计算。
fn width_of(piece: &str, tab_width: usize) -> usize {
    piece
        .chars()
        .map(|c| char_cells(c, 0, tab_width.max(1)))
        .sum()
}

impl Vim {
    /// 对块执行操作符；操作符不按块工作时返回假，由调用方按行处理。
    pub(super) fn block_operate(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        operator: Operator,
        visual: Visual,
        to_end: bool,
    ) -> bool {
        let by_block = matches!(
            operator,
            Operator::Yank
                | Operator::Delete
                | Operator::Change
                | Operator::ToggleCase
                | Operator::Lowercase
                | Operator::Uppercase
                | Operator::Rot13
                | Operator::Indent
                | Operator::Outdent
        );
        if !by_block {
            return false;
        }
        if operator != Operator::Yank && self.blocked(host) {
            return true;
        }
        let block = block_of(host, visual, to_end, self.columns.take());
        let rows = rows_of(host, block);
        let Some(top) = rows.first().copied() else {
            return true;
        };
        let tab_width = host.tab_width();
        let text = host.text();
        let pieces: Vec<String> = rows
            .iter()
            .map(|row| operator::normalized(text.substring(row.from, row.to)))
            .collect();
        let width = if to_end {
            pieces
                .iter()
                .map(|piece| width_of(piece, tab_width))
                .max()
                .unwrap_or(1)
                .max(1)
        } else {
            block.right - block.left + 1
        };
        let extent = Extent {
            kind: VisualKind::Block,
            lines: block.last - block.first + 1,
            cells: if to_end { usize::MAX } else { width },
        };
        // 不到左边界的行在寄存器中记作与块同宽的空格。
        let padded: Vec<String> = (rows.iter().zip(&pieces))
            .map(|(row, piece)| {
                if row.short {
                    // 延伸到行尾的块多记一列。
                    " ".repeat(width + usize::from(to_end))
                } else {
                    piece.clone()
                }
            })
            .collect();
        let content = Content {
            kind: Kind::Blockwise(width),
            text: padded.join("\n"),
        };
        let last = rows.last().map_or(top.to, |row| row.to);
        self.end_visual(host);

        match operator {
            Operator::Yank => {
                if Self::write_register(shared, host, command.register, content, Source::Yank) {
                    host.set_mark('[', top.from);
                    host.set_mark(']', last.saturating_sub(1).max(top.from));
                }
                self.settle(host, top.from);
            }
            Operator::Delete | Operator::Change => {
                let source = Source::Delete {
                    numbered: rows.len() > 1,
                };
                if !Self::write_register(shared, host, command.register, content, source) {
                    self.settle(host, top.from);
                    return true;
                }
                let changes: Vec<Change> = (rows.iter().rev())
                    .filter(|row| row.from < row.to)
                    .map(|row| operator::removal(host.text(), row.from, row.to))
                    .collect();
                // 各行删除之后，后面各行的位置前移。
                let mut removed = 0;
                let mut carets = Vec::new();
                for row in &rows {
                    if !row.short {
                        carets.push(row.from - removed);
                    }
                    removed += row.to - row.from;
                }
                if !self.edit(host, changes, top.from, block.first) {
                    self.settle(host, top.from);
                    return true;
                }
                if operator == Operator::Delete {
                    self.settle(host, top.from);
                    self.remember(shared, command, Some(extent));
                } else {
                    self.insert_at(host, command, carets, After::First, Some(extent));
                }
            }
            Operator::ToggleCase | Operator::Lowercase | Operator::Uppercase | Operator::Rot13 => {
                let changes: Vec<Change> = (rows.iter().rev())
                    .filter_map(|row| {
                        let old = host.text().substring(row.from, row.to);
                        let new = operator::recased(&old, operator);
                        (old != new).then_some(Change {
                            start: row.from,
                            removed: old,
                            inserted: new,
                        })
                    })
                    .collect();
                if self.edit(host, changes, top.from, block.first) {
                    self.remember(shared, command, Some(extent));
                }
                self.settle(host, top.from);
            }
            _ => {
                let amount = command
                    .count
                    .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
                let changes = shift(host, &rows, amount, operator == Operator::Outdent);
                if self.edit(host, changes, top.from, block.first) {
                    self.remember(shared, command, Some(extent));
                }
                let text = host.text();
                self.settle(host, top.from.min(text.line_end(top.line)));
            }
        }
        true
    }

    /// 返回按字符与按行的可视模式下 `I`、`A` 作用的起点、终点，以及供点重复使用的范围。
    ///
    /// 范围从选区起点所在行的行首到光标，光标不在行首之后时两者对调；重放时起点是光标，
    /// 终点在其下若干行的同一列。范围的列数只取 1、2 与 `usize::MAX`（由 `$` 延伸到行尾）。
    fn insert_span(&mut self, host: &impl Host, visual: Visual) -> (usize, usize, Extent) {
        let text = host.text();
        let on_char = |pos: usize| pos.min(text::last_char(text, text::line_of(text, pos)));
        if let Some(extent) = self.replayed.take().filter(|_| self.replaying) {
            let (line, col) = text.pos_to_line_col(visual.anchor);
            let last = (line + extent.lines - 1).min(text::last_line(text));
            let end = if extent.cells == usize::MAX {
                text::last_char(text, last)
            } else {
                on_char((text.line_start(last) + col).min(text.line_end(last)))
            };
            return (visual.anchor, end, extent);
        }
        let origin = text.line_start(text::line_of(text, visual.anchor));
        // 作为起点的光标可以在行尾之后，作为终点时取所在的字符。
        let (start, end) = if visual.cursor > origin {
            (origin, on_char(visual.cursor))
        } else {
            (visual.cursor, origin)
        };
        // 终点的字符只占第 0 列时列数记为 1。
        let cells = if self.want == crate::motion::Want::End {
            usize::MAX
        } else if cells(host, end).1 == 0 {
            1
        } else {
            2
        };
        let extent = Extent {
            kind: VisualKind::Line,
            lines: text::line_of(text, end) - text::line_of(text, start) + 1,
            cells,
        };
        (start, end, extent)
    }

    /// 可视模式的 `I`、`A`：按块时在各行的左边界之前或右边界之后插入，其余可视模式下在
    /// [`Self::insert_span`] 给出的起点之前或终点之后插入。
    pub(super) fn insert_selection(
        &mut self,
        host: &mut impl Host,
        command: Command,
        append: bool,
    ) {
        if self.blocked(host) {
            return;
        }
        let Some(visual) = self.visual else {
            return;
        };
        if visual.kind != VisualKind::Block {
            let (start, end, extent) = self.insert_span(host, visual);
            let text = host.text();
            // 记录的列数为 2 时在终点的字符之后插入，否则在该字符之前插入。
            let pos = if !append {
                start
            } else if extent.cells == 2 && text.line_len(text::line_of(text, end)) > 0 {
                text::next_in_line(text, end).unwrap_or(end)
            } else {
                end
            };
            self.end_visual(host);
            self.begin_insert(host, command, pos, None);
            if let Some(session) = &mut self.insert {
                session.single(After::Left, Some(extent));
            }
            return;
        }
        let to_end = self.want == crate::motion::Want::End;
        let block = block_of(host, visual, to_end, self.columns.take());
        let rows = rows_of(host, block);
        let tab_width = host.tab_width();
        let extent = Extent {
            kind: VisualKind::Block,
            lines: block.last - block.first + 1,
            cells: if to_end {
                usize::MAX
            } else {
                block.right - block.left + 1
            },
        };
        let Some(top) = rows.first().copied() else {
            return;
        };
        self.end_visual(host);
        let mut carets = Vec::new();
        if !append {
            carets.extend(rows.iter().filter(|row| !row.short).map(|row| row.from));
        } else {
            // 右边界之后；不到右边界的行先用空格补到该列，延伸到行尾的块不补。
            let text = host.text();
            let mut changes = Vec::new();
            let mut pads = Vec::new();
            for row in &rows {
                let end = text.line_end(row.line);
                let (line, col) = text.pos_to_line_col(end);
                let line_cells = text::cell_of(text, line, col, tab_width);
                let pad = if to_end || row.to < end {
                    0
                } else {
                    (block.right + 1).saturating_sub(line_cells)
                };
                pads.push(pad);
            }
            let mut added = 0;
            for (row, pad) in rows.iter().zip(&pads) {
                carets.push(row.to + added + pad);
                added += pad;
            }
            for (row, pad) in rows.iter().zip(&pads).rev() {
                if *pad > 0 {
                    changes.push(Change {
                        start: text.line_end(row.line),
                        removed: String::new(),
                        inserted: " ".repeat(*pad),
                    });
                }
            }
            if !self.edit(host, changes, top.from, block.first) {
                return;
            }
        }
        let after = After::At {
            line: block.first,
            cell: block.left,
        };
        self.insert_at(host, command, carets, after, Some(extent));
    }

    /// 在 `carets` 各处同时进入插入模式；离开时光标按 `after` 放置。
    pub(super) fn insert_at(
        &mut self,
        host: &mut impl Host,
        command: Command,
        carets: Vec<usize>,
        after: After,
        extent: Option<Extent>,
    ) {
        let Some(first) = carets.first().copied() else {
            let pos = self.caret(host);
            self.settle(host, pos);
            return;
        };
        self.begin_insert(host, command, first, None);
        if let Some(session) = &mut self.insert {
            session.single(after, extent);
        }
        if carets.len() > 1 {
            let selections = carets.into_iter().map(Selection::point).collect();
            host.set_selections(selections, 0);
            self.shown = Some(Selection::point(first));
        }
    }

    /// 把按块的寄存器内容 `content` 粘贴到光标处：各行依次插入到光标所在的显示列。
    ///
    /// `before` 为真时插入在光标处的字符之前。返回粘贴内容第一个字符的位置，以及最后一行上
    /// 粘贴内容之后的位置；修改没有应用时返回 `None`。
    pub(super) fn paste_block(
        &mut self,
        host: &mut impl Host,
        content: &Content,
        width: usize,
        count: usize,
        before: bool,
    ) -> Option<(usize, usize)> {
        let text = host.text();
        let tab_width = host.tab_width();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let last = text::last_line(text);
        let (start, end) = cells(host, cursor);
        let column = if before || text.line_len(line) == 0 {
            start
        } else {
            end + 1
        };
        let ending = host.line_ending().to_owned();
        let pieces: Vec<&str> = content.lines().collect();
        let mut changes = Vec::new();
        let mut appended = String::new();
        let mut first = cursor;
        let mut after = cursor;
        let mut grown = 0;
        for (index, piece) in pieces.iter().enumerate() {
            let target = line + index;
            let fill = width.saturating_sub(width_of(piece, tab_width));
            let body = |short: bool| {
                let mut out = String::new();
                for repeat in 0..count {
                    out.push_str(piece);
                    // 之后还有文本时才补齐到块的宽度。
                    if repeat + 1 < count || !short {
                        out.push_str(&" ".repeat(fill));
                    }
                }
                out
            };
            if target > last {
                // 超出文本末尾的部分另起新行。
                appended.push_str(&ending);
                appended.push_str(&" ".repeat(column));
                appended.push_str(&body(true));
                after = text.line_end(last) + grown + appended.chars().count();
                continue;
            }
            let end = text.line_end(target);
            let (_, col) = text.pos_to_line_col(end);
            let line_cells = text::cell_of(text, target, col, tab_width);
            let (pos, pad, short) = if line_cells < column {
                (end, column - line_cells, true)
            } else {
                let pos = text::pos_at_cell(text, target, column, tab_width);
                (pos, 0, pos == end)
            };
            let inserted = format!("{}{}", " ".repeat(pad), body(short));
            if index == 0 {
                first = pos + pad;
            }
            // 之前各行插入的文本使该行的位置后移。
            grown += inserted.chars().count();
            after = pos + grown;
            changes.push(Change {
                start: pos,
                removed: String::new(),
                inserted,
            });
        }
        // 位置靠后的修改先应用；新行接在最后一行之后。
        changes.reverse();
        if !appended.is_empty() {
            changes.insert(
                0,
                Change {
                    start: text.line_end(last),
                    removed: String::new(),
                    inserted: appended,
                },
            );
        }
        self.edit(host, changes, cursor, line)
            .then_some((first, after))
    }
}

/// 返回把块 `rows` 所在的文本右移（`left` 为假）或左移 `amount` 级缩进的修改。
///
/// 右移时在块的左边界处加入空白，连同其前后已有的空白重新排成制表符与空格；左移时去掉
/// 左边界起的空白。不到左边界的行与空行不变。
fn shift<H: Host>(host: &H, rows: &[Row], amount: usize, left: bool) -> Vec<Change> {
    let text = host.text();
    let tab_width = host.tab_width().max(1);
    let indentation = host.indentation();
    let by = indentation.width.max(1).saturating_mul(amount);
    let blank = |pos: usize| text.char_at(pos).is_some_and(text::is_blank);
    let cell = |pos: usize| {
        let (line, col) = text.pos_to_line_col(pos);
        text::cell_of(text, line, col, tab_width)
    };
    (rows.iter().rev())
        .filter(|row| !row.short && text.line_len(row.line) > 0)
        .filter_map(|row| {
            let (start, end) = (text.line_start(row.line), text.line_end(row.line));
            let mut to = row.from;
            while to < end && blank(to) {
                to += 1;
            }
            let (from, cells) = if left {
                let width = cell(to) - cell(row.from);
                (row.from, width - by.min(width))
            } else {
                let mut from = row.from;
                while from > start && blank(from - 1) {
                    from -= 1;
                }
                (from, (cell(to) - cell(from)).saturating_add(by))
            };
            let new = if left || !indentation.tabs {
                " ".repeat(cells)
            } else {
                // 从空白的起始列算起：先排满制表符，余下的用空格。
                let offset = cell(from) % tab_width;
                let tabs = (offset + cells) / tab_width;
                let spaces = if tabs > 0 {
                    (offset + cells) % tab_width
                } else {
                    cells
                };
                format!("{}{}", "\t".repeat(tabs), " ".repeat(spaces))
            };
            let old = text.substring(from, to);
            (old != new).then_some(Change {
                start: from,
                removed: old,
                inserted: new,
            })
        })
        .collect()
}
