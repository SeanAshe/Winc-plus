//! 操作符的执行：取范围、写寄存器、生成修改并放置光标。

use winc_core::{Change, Lines};

use super::{Extent, Shared, Vim, Visual, VisualKind};
use crate::command::{Force, Motion, Operator, Shape};
use crate::host::{Host, Message};
use crate::motion::{self, Moved, Span, Want, WantUpdate};
use crate::object::{self, At};
use crate::operator::{self, Range, Region};
use crate::parser::{Command, Movement, Target};
use crate::register::{Content, Kind, Source};
use crate::text;

impl Vim {
    /// 执行操作符命令。
    pub(super) fn operate(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        operator: Operator,
        target: Target,
        force: Option<Force>,
    ) {
        let modifies = operator != Operator::Yank;
        if modifies && self.blocked(host) {
            return;
        }
        // 按块的可视模式：操作符作用于块；`S`、`R` 与只按行工作的操作符仍作用于整行。
        if let Some(visual) = self.visual
            && visual.kind == VisualKind::Block
            && force != Some(Force::Linewise)
        {
            let to_end = match target {
                Target::SelectedLines { to_end } => to_end || self.want == Want::End,
                _ => self.want == Want::End,
            };
            if self.block_operate(shared, host, command, operator, visual, to_end) {
                return;
            }
        }

        // 取不到范围即命令失败；就地执行完毕的命令自行清除。
        let failed = std::mem::replace(&mut self.failed, true);
        let Some((range, extent)) =
            self.target_range(shared, host, command, operator, target, force)
        else {
            return;
        };
        self.failed = failed;

        let (from, to) = bounds(host, range.region);
        if !host.fits(from, to) {
            self.error(host, Message::TooLarge);
            return;
        }
        let visual = self.visual.is_some();
        // 可视模式下的次数是增减缩进的级数。
        let amount = if visual {
            command
                .count
                .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX))
        } else {
            1
        };
        if visual {
            self.end_visual(host);
        }

        let done = match operator {
            Operator::Yank => self.yank(shared, host, command, range),
            Operator::Delete => self.delete(shared, host, command, range),
            Operator::Change => self.change(shared, host, command, range),
            Operator::Indent | Operator::Outdent => {
                // 增加的缩进按每行、每级一个字节估算，超过上限时不执行。
                let lines = range.last - range.first + 1;
                let by = host.indentation().width.max(1).saturating_mul(amount);
                if operator == Operator::Indent && lines.saturating_mul(by) > super::MAX_OUTPUT {
                    self.error(host, Message::TooLarge);
                    false
                } else {
                    let left = operator == Operator::Outdent;
                    let changes = operator::shift(host, range.first, range.last, amount, left);
                    let done = self.edit(host, changes, range.start, range.first);
                    let text = host.text();
                    let pos = text::first_non_blank(text, range.first.min(text::last_line(text)));
                    self.settle(host, pos);
                    done
                }
            }
            Operator::ToggleCase | Operator::Lowercase | Operator::Uppercase | Operator::Rot13 => {
                self.recase(host, operator, range)
            }
            Operator::Join | Operator::JoinRaw => {
                let count = range.last - range.first + 1;
                self.join_at(
                    host,
                    range.first,
                    count,
                    operator == Operator::Join,
                    range.start,
                )
            }
            Operator::Comment => {
                let done = host.toggle_comment(range.first, range.last);
                self.shown = None;
                let text = host.text();
                let pos = text::first_non_blank(text, range.first.min(text::last_line(text)));
                self.settle(host, pos);
                done
            }
            Operator::Reindent => {
                let done = host.format_lines(range.first, range.last);
                if !done {
                    self.error(host, Message::Unsupported);
                }
                self.shown = None;
                let text = host.text();
                let pos = text::first_non_blank(text, range.first.min(text::last_line(text)));
                self.settle(host, pos);
                done
            }
            Operator::Surround(with) => self.wrap(host, range, with),
            Operator::Format | Operator::Fold | Operator::Filter => {
                self.error(host, Message::Unsupported);
                false
            }
        };
        // 操作没有执行时，光标回到范围的起点。
        if !done && self.insert.is_none() {
            self.settle(host, range.start);
        }
        if done && modifies {
            if operator == Operator::Change {
                if let Some(session) = &mut self.insert {
                    session.extent = extent;
                }
            } else {
                self.remember(shared, command, extent);
            }
        }
    }

    /// 返回操作符作用的范围，以及可视模式下供点重复使用的范围；命令没有可作用的范围，或已
    /// 就地执行完毕时返回 `None`。
    fn target_range(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        operator: Operator,
        target: Target,
        force: Option<Force>,
    ) -> Option<(Range, Option<Extent>)> {
        let cursor = self.caret(host);
        let whole_lines = whole_lines(operator);
        let times = command
            .count
            .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let found = match (target, self.visual) {
            (Target::Selection | Target::SelectedLines { .. }, Some(visual)) => {
                let linewise = visual.kind != VisualKind::Char
                    || force == Some(Force::Linewise)
                    || matches!(target, Target::SelectedLines { .. });
                let range = operator::visual_region(
                    host,
                    visual.anchor,
                    visual.cursor,
                    linewise,
                    whole_lines,
                );
                let to_end = self.want == Want::End;
                (range, Some(extent_of(host, visual, linewise, to_end)))
            }
            (Target::Motion(movement), _) => {
                let moved = self.moved(shared, host, movement, command.count, Some(operator))?;
                if moved.failed {
                    self.settle(host, moved.pos);
                    return None;
                }
                if force == Some(Force::Blockwise) {
                    // 强制按块：以光标与目标位置为两角的块。
                    let visual = Visual {
                        kind: VisualKind::Block,
                        anchor: cursor,
                        cursor: moved.pos,
                    };
                    self.visual = Some(visual);
                    self.failed = false;
                    if !self.block_operate(shared, host, command, operator, visual, false) {
                        self.visual = None;
                        self.error(host, Message::Unsupported);
                    }
                    // 重复时重新求值动作，不沿用块的大小。
                    if !self.replaying
                        && let Some(repeat) = &mut shared.repeat
                    {
                        repeat.extent = None;
                    }
                    if let Some(session) = &mut self.insert {
                        session.extent = None;
                    }
                    return None;
                }
                (
                    operator::motion_region(host, cursor, moved, force, operator),
                    None,
                )
            }
            (Target::Lines, _) => {
                // 叠写形式：从当前行起的「次数」行。
                let down = Movement {
                    motion: Motion::Down,
                    arg: None,
                };
                let count = command
                    .count
                    .map(|n| n.saturating_sub(1))
                    .filter(|n| *n > 0);
                let mut moved = if count.is_some() {
                    self.moved(shared, host, down, count, Some(operator))?
                } else {
                    Moved {
                        want: WantUpdate::Keep,
                        ..Moved::new(cursor, Span::Linewise)
                    }
                };
                // 复制之外的操作符先把光标移到末行的第一个非空白字符。
                if operator != Operator::Yank {
                    let text = host.text();
                    moved.pos = text::first_non_blank(text, text::line_of(text, moved.pos));
                }
                (
                    operator::motion_region(host, cursor, moved, force, operator),
                    None,
                )
            }
            (Target::Object(object), _) => {
                let found = match object.shape {
                    Shape::NextMatch => self.match_target(shared, host, cursor, true, times),
                    Shape::PreviousMatch => self.match_target(shared, host, cursor, false, times),
                    _ => {
                        let at = At {
                            cursor,
                            anchor: None,
                            linewise: false,
                            count: times.min(motion::MAX_STEPS),
                            tab_width: host.tab_width(),
                        };
                        object::select(host.text(), object, at).map(|selected| {
                            let span = if selected.linewise {
                                Span::Linewise
                            } else if selected.inclusive {
                                Span::Inclusive
                            } else {
                                Span::Exclusive
                            };
                            // 标签之内的文本止于行首时，范围不调整到上一行的行尾。
                            let moved = Moved {
                                keep_end: object.shape == Shape::Tag,
                                failed: selected.failed,
                                ..Moved::new(selected.end, span)
                            };
                            (selected.start, moved)
                        })
                    }
                };
                let (start, moved) = found?;
                if moved.failed {
                    self.settle(host, moved.pos);
                    return None;
                }
                (
                    operator::motion_region(host, start, moved, force, operator),
                    None,
                )
            }
            // 没有选区时不会出现作用于选区的命令。
            (Target::Selection | Target::SelectedLines { .. }, None) => return None,
        };
        Some(found)
    }

    /// 变换 `range` 中字母的大小写，光标移到范围的起点。
    fn recase(&mut self, host: &mut impl Host, operator: Operator, range: Range) -> bool {
        let (from, to) = bounds(host, range.region);
        let text = host.text();
        // 范围为空且起点在行首时仍有字符被变换：文本开头为第一个字符，其余为整行。
        let to = match range.region {
            Region::Chars { from, to } if from == to && from == text.line_start(range.first) => {
                if range.first == 0 {
                    text::next_in_line(text, from).unwrap_or(from)
                } else {
                    text.line_end(range.first)
                }
            }
            _ => to,
        };
        let old = text.substring(from, to);
        let new = operator::recased(&old, operator);
        let changes = if old == new {
            Vec::new()
        } else {
            vec![Change {
                start: from,
                removed: old,
                inserted: new,
            }]
        };
        let done = self.edit(host, changes, range.start, range.first);
        self.settle(host, range.start);
        done
    }

    /// 复制 `range` 中的文本，光标移到范围的起点。
    pub(super) fn yank(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        range: Range,
    ) -> bool {
        let content = content_of(host, range.region);
        if !Self::write_register(shared, host, command.register, content, Source::Yank) {
            return false;
        }
        let text = host.text();
        let (from, to) = match range.region {
            Region::Chars { from, to } => (from, to.saturating_sub(1).max(from)),
            Region::Lines { first, last } => (text.line_start(first), text.line_end(last)),
        };
        host.set_mark('[', from);
        host.set_mark(']', to);
        host.yanked(from, to + 1);
        // 光标移到范围的起点；按行的范围向下延伸时起点就是光标。
        let pos = match range.region {
            Region::Lines { .. } => range.start,
            Region::Chars { from, .. } => from,
        };
        self.settle(host, pos);
        true
    }

    /// 删除 `range` 中的文本并写入寄存器。
    pub(super) fn delete(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        range: Range,
    ) -> bool {
        match range.region {
            Region::Chars { from, to } => {
                if from == to {
                    self.settle(host, from);
                    return true;
                }
                let content = content_of(host, range.region);
                let source = Source::Delete {
                    numbered: range.numbered,
                };
                if !Self::write_register(shared, host, command.register, content, source) {
                    return false;
                }
                let change = operator::removal(host.text(), from, to);
                let done = self.edit(host, vec![change], range.start, range.first);
                self.settle(host, from);
                done
            }
            Region::Lines { first, last } => {
                let content = content_of(host, range.region);
                let source = Source::Delete { numbered: true };
                if !Self::write_register(shared, host, command.register, content, source) {
                    return false;
                }
                let (from, to) = operator::lines_span(host.text(), first, last);
                let change = operator::removal(host.text(), from, to);
                let done = self.edit(host, vec![change], range.start, first);
                let text = host.text();
                let pos = text::first_non_blank(text, first.min(text::last_line(text)));
                self.settle(host, pos);
                done
            }
        }
    }

    /// 删除 `range` 中的文本并进入插入模式；按行的范围保留一个空行。
    fn change(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        range: Range,
    ) -> bool {
        let text = host.text();
        let (from, to) = match range.region {
            Region::Chars { from, to } => (from, to),
            Region::Lines { first, last } => {
                let keep = if host.keeps_indent() {
                    text::indent_chars(text, first)
                } else {
                    0
                };
                (text.line_start(first) + keep, text.line_end(last))
            }
        };
        let linewise = matches!(range.region, Region::Lines { .. });
        // 含终点的动作在空行上范围不为空，寄存器写入空的内容。
        if !range.empty {
            let content = content_of(host, range.region);
            let source = Source::Delete {
                numbered: range.numbered || linewise,
            };
            if !Self::write_register(shared, host, command.register, content, source) {
                return false;
            }
        }
        if from < to {
            let change = operator::removal(host.text(), from, to);
            if !self.edit(host, vec![change], range.start, range.first) {
                return false;
            }
        }
        self.begin_insert(host, command, from, None);
        true
    }

    /// 合并从第 `first` 行起的 `count` 行，光标放在最后一处连接的位置。
    pub(super) fn join_at(
        &mut self,
        host: &mut impl Host,
        first: usize,
        count: usize,
        spaces: bool,
        at: usize,
    ) -> bool {
        let text = host.text();
        let Some((changes, col)) = operator::join(text, first, count, spaces) else {
            return false;
        };
        let start = text.line_start(first);
        let done = self.edit(host, changes, at, first);
        self.settle(host, start + col);
        done
    }
}

/// 返回区域覆盖的位置区间：按行的区域从首行行首到末行行尾。
fn bounds<H: Host>(host: &H, region: Region) -> (usize, usize) {
    let text = host.text();
    match region {
        Region::Chars { from, to } => (from, to),
        Region::Lines { first, last } => (text.line_start(first), text.line_end(last)),
    }
}

/// 返回区域中的文本，作为寄存器的内容。
pub(super) fn content_of<H: Host>(host: &H, region: Region) -> Content {
    let text = host.text();
    match region {
        Region::Chars { from, to } => Content {
            kind: Kind::Charwise,
            text: operator::normalized(text.substring(from, to)),
        },
        Region::Lines { first, last } => Content {
            kind: Kind::Linewise,
            text: operator::lines_text(text, first, last),
        },
    }
}

/// 操作符是否只按行工作。
fn whole_lines(operator: Operator) -> bool {
    matches!(
        operator,
        Operator::Indent
            | Operator::Outdent
            | Operator::Join
            | Operator::JoinRaw
            | Operator::Comment
            | Operator::Reindent
    )
}

/// 返回可视选区覆盖的范围，供点重复使用。
///
/// `to_end` 表示选区由 `$` 延伸到行尾，重放时同样延伸到行尾。
pub(super) fn extent_of<H: Host>(host: &H, visual: Visual, linewise: bool, to_end: bool) -> Extent {
    let text = host.text();
    let (low, high) = (
        visual.anchor.min(visual.cursor),
        visual.anchor.max(visual.cursor),
    );
    let (first, last) = (text::line_of(text, low), text::line_of(text, high));
    let tab_width = host.tab_width();
    // 终点所在字符占用的最后一个显示列；终点在行尾时取行宽。
    let end_cell = {
        let (line, col) = text.pos_to_line_col(high);
        let cell = text::cell_of(text, line, col, tab_width);
        match text.char_at(high) {
            Some(c) if high < text.line_end(line) => {
                cell + winc_core::text::char_cells(c, cell, tab_width.max(1)).max(1) - 1
            }
            _ => cell,
        }
    };
    let cells = if to_end {
        usize::MAX
    } else if first == last {
        let (line, col) = text.pos_to_line_col(low);
        end_cell + 1 - text::cell_of(text, line, col, tab_width).min(end_cell)
    } else {
        end_cell
    };
    Extent {
        kind: if linewise {
            VisualKind::Line
        } else {
            VisualKind::Char
        },
        lines: last - first + 1,
        cells,
    }
}
