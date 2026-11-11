//! 滚动与折叠命令：`Ctrl-E` `Ctrl-Y` `Ctrl-D` `Ctrl-U` `Ctrl-F` `Ctrl-B`、`z` 开头的命令。
//!
//! 窗口由宿主滚动；光标在滚动之后由状态机移回窗口之内。

use winc_core::Lines;

use super::Vim;
use crate::command::{Folding, Scrolling};
use crate::host::{Fold, Host, Place, Scroll};
use crate::motion::{Want, WantUpdate};
use crate::text;

/// 一次滚动的行数或列数的上限；次数更大时按该值执行。
///
/// 取自调参：超过任何窗口的大小，换算成有符号数时不会溢出。
const MAX_SCROLL: usize = 1 << 30;

/// 返回次数，未输入时为 `default`，不超过 [`MAX_SCROLL`]。
fn amount(count: Option<u32>, default: usize) -> usize {
    count
        .map_or(default, |n| usize::try_from(n).unwrap_or(usize::MAX))
        .min(MAX_SCROLL)
}

/// 把无符号的数量换算成滚动的方向与距离。
fn signed(amount: usize, forward: bool) -> isize {
    let amount = isize::try_from(amount).unwrap_or(isize::MAX);
    if forward { amount } else { -amount }
}

impl Vim {
    /// 把光标移到第 `line` 行并保持期望列；`start` 为真时移到第一个非空白字符。
    fn move_to_line(&mut self, host: &mut impl Host, line: usize, start: bool) {
        let text = host.text();
        let line = line.min(text::last_line(text));
        if start {
            let pos = text::first_non_blank(text, line);
            self.goto(host, pos, WantUpdate::Recompute);
            return;
        }
        let limit = if self.visual.is_some() {
            text.line_end(line)
        } else {
            text::last_char(text, line)
        };
        let pos = match self.want {
            Want::End => limit,
            Want::Cell(cell) => text::pos_at_cell(text, line, cell, host.tab_width()).min(limit),
        };
        self.goto(host, pos, WantUpdate::Keep);
    }

    /// 滚动之后把光标收回窗口之内，上下各留出宿主要求的行数。
    fn keep_in_window(&mut self, host: &mut impl Host) {
        let text = host.text();
        let line = text::line_of(text, self.caret(host));
        let visible = host.visible_lines();
        let last = text::last_line(text);
        let (top, bottom) = (
            visible.start.min(last),
            visible.end.saturating_sub(1).min(last),
        );
        let margin = host.scroll_margin().min(bottom.saturating_sub(top) / 2);
        let (low, high) = (top + margin, bottom.saturating_sub(margin).max(top));
        // 窗口已到文本的开头或末尾时边缘不留行。
        let low = if top == 0 { top } else { low };
        let high = if bottom >= last { bottom } else { high };
        if line < low {
            self.move_to_line(host, low, false);
        } else if line > high {
            self.move_to_line(host, high, false);
        }
    }

    /// 执行滚动命令。
    pub(super) fn scroll(
        &mut self,
        host: &mut impl Host,
        scrolling: Scrolling,
        count: Option<u32>,
    ) {
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let last = text::last_line(text);
        let height = host.window_height().max(1);
        match scrolling {
            Scrolling::LineDown | Scrolling::LineUp => {
                let down = scrolling == Scrolling::LineDown;
                host.scroll(Scroll::Lines(signed(amount(count, 1), down)));
                self.keep_in_window(host);
            }
            Scrolling::HalfDown | Scrolling::HalfUp => {
                let down = scrolling == Scrolling::HalfDown;
                // 次数给出时成为之后半屏滚动的行数。
                if let Some(count) = count {
                    self.half = Some(amount(Some(count), 1).min(height));
                }
                let by = self.half.unwrap_or((height / 2).max(1));
                let target = if down {
                    line.saturating_add(by).min(last)
                } else {
                    line.saturating_sub(by)
                };
                let scrolled = host.scroll(Scroll::Lines(signed(by, down)));
                if target != line {
                    self.move_to_line(host, target, scrolled);
                }
            }
            Scrolling::PageDown | Scrolling::PageUp => {
                let down = scrolling == Scrolling::PageDown;
                let page = height.saturating_sub(2).max(1);
                let by = page.saturating_mul(amount(count, 1)).min(MAX_SCROLL);
                if !host.scroll(Scroll::Lines(signed(by, down))) {
                    return;
                }
                // 向下翻页之后光标在窗口的第一行，向上翻页之后在最后一行。
                let visible = host.visible_lines();
                let target = if down {
                    visible.start
                } else {
                    visible.end.saturating_sub(1)
                };
                self.move_to_line(host, target, true);
            }
            Scrolling::Top
            | Scrolling::TopStart
            | Scrolling::Center
            | Scrolling::CenterStart
            | Scrolling::Bottom
            | Scrolling::BottomStart => {
                let place = match scrolling {
                    Scrolling::Top | Scrolling::TopStart => Place::Top,
                    Scrolling::Center | Scrolling::CenterStart => Place::Center,
                    _ => Place::Bottom,
                };
                let start = matches!(
                    scrolling,
                    Scrolling::TopStart | Scrolling::CenterStart | Scrolling::BottomStart
                );
                // 次数是行号：先把光标移到该行。
                let line = match count {
                    Some(_) => (amount(count, 1) - 1).min(last),
                    None => line,
                };
                if count.is_some() || start {
                    self.move_to_line(host, line, start);
                }
                host.scroll(Scroll::Reveal { line, place });
            }
            Scrolling::Left
            | Scrolling::Right
            | Scrolling::HalfLeft
            | Scrolling::HalfRight
            | Scrolling::CaretStart
            | Scrolling::CaretEnd => self.scroll_columns(host, scrolling, count),
        }
    }

    /// 执行横向的滚动命令；自动换行时没有横向滚动，不执行。
    fn scroll_columns(&mut self, host: &mut impl Host, scrolling: Scrolling, count: Option<u32>) {
        let Some(cells) = host.visible_cells() else {
            return;
        };
        let width = cells.end.saturating_sub(cells.start).max(1);
        let tab_width = host.tab_width();
        let cursor = self.caret(host);
        let cell = text::want_cell(host.text(), cursor, tab_width);
        let by = match scrolling {
            Scrolling::Left => signed(amount(count, 1), false),
            Scrolling::Right => signed(amount(count, 1), true),
            Scrolling::HalfLeft => signed(amount(count, 1).saturating_mul(width / 2), false),
            Scrolling::HalfRight => signed(amount(count, 1).saturating_mul(width / 2), true),
            Scrolling::CaretStart => signed(cell.abs_diff(cells.start), cell > cells.start),
            _ => {
                let right = cells.end.saturating_sub(1);
                signed(cell.abs_diff(right), cell > right)
            }
        };
        host.scroll(Scroll::Columns(by));
        // 光标滚出窗口时移到窗口之内最近的一列。
        let Some(cells) = host.visible_cells() else {
            return;
        };
        let target = cell.clamp(cells.start, cells.end.saturating_sub(1).max(cells.start));
        if target != cell {
            let text = host.text();
            let line = text::line_of(text, cursor);
            let pos = text::pos_at_cell(text, line, target, tab_width);
            self.goto(host, pos, WantUpdate::Recompute);
        }
    }

    /// 执行折叠命令。
    pub(super) fn fold(&mut self, host: &mut impl Host, folding: Folding, count: Option<u32>) {
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let fold = match folding {
            Folding::Open => Fold::Open,
            Folding::Close => Fold::Close,
            Folding::Toggle => Fold::Toggle,
            Folding::OpenTree => Fold::OpenTree,
            Folding::CloseTree => Fold::CloseTree,
            Folding::ToggleTree => Fold::ToggleTree,
            Folding::OpenAll => Fold::OpenAll,
            Folding::CloseAll => Fold::CloseAll,
            Folding::Reveal => Fold::Reveal,
            Folding::Next | Folding::Previous => {
                let forward = folding == Folding::Next;
                let mut target = line;
                for _ in 0..amount(count, 1) {
                    match host.fold_edge(target, forward) {
                        Some(next) => target = next,
                        None => break,
                    }
                }
                if target != line {
                    self.move_to_line(host, target, false);
                }
                return;
            }
        };
        if host.fold_at(line, fold) {
            // 光标落在收起的区域之内时移到其首行。
            let line = host.fold(line).map_or(line, |(first, _)| first);
            self.shown = None;
            self.move_to_line(host, line, false);
        }
    }
}
