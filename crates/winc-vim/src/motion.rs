//! 动作的求值：从光标位置算出目标位置与动作的类型（不含终点、含终点、按行）。
//!
//! 求值只读文本，不移动光标。目标位置可以落在行尾（最后一个字符之后），由调用方按所在的
//! 模式收拢。按词、按句、按段与括号配对的动作分别在子模块中。

pub(crate) mod block;
pub(crate) mod pair;
mod word;

use winc_core::Lines;

use crate::command::{Motion, Operator};
use crate::host::Host;
use crate::parser::Movement;
use crate::text::{self, Scan};

/// 逐步移动的动作单次执行的步数上限；次数更大时按该值执行。
///
/// 取自调参：十万步在 Apple M4 Pro 上的耗时不超过数十毫秒，远多于正常使用的次数。
pub(crate) const MAX_STEPS: usize = 100_000;

/// 行内查找字符时从光标起最多检查的字符数（字符）。
///
/// 取自调参：限制超长行上单次查找的耗时，常见源代码的行远短于该长度。
const MAX_FIND: usize = 1_000_000;

/// 动作的类型，决定操作符作用的范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Span {
    /// 按字符，不含目标位置上的字符。
    Exclusive,
    /// 按字符，包含目标位置上的字符。
    Inclusive,
    /// 按行：起止位置所在的整行。
    Linewise,
}

/// 上下移动时保持的期望列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Want {
    /// 显示列（从 0 起）。
    Cell(usize),
    /// 行尾：`$` 之后的上下移动停在各行行尾。
    End,
}

/// 动作执行之后期望列的处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WantUpdate {
    /// 保持不变（上下移动）。
    Keep,
    /// 设为给定的值。
    Set(Want),
    /// 按目标位置重新计算。
    Recompute,
}

/// 动作的求值结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Moved {
    /// 目标位置。
    pub pos: usize,
    /// 动作的类型。
    pub span: Span,
    /// 期望列的处理。
    pub want: WantUpdate,
    /// 删除时是否不论长短都进入编号寄存器（句、段、括号配对等动作）。
    pub numbered: bool,
    /// 是否禁止把止于行首的范围调整到上一行行尾。
    pub keep_end: bool,
    /// 动作是否失败：光标已经移动，但等待中的操作符不再执行。
    pub failed: bool,
}

impl Moved {
    /// 创建期望列需要重新计算的结果。
    pub(crate) fn new(pos: usize, span: Span) -> Self {
        Self {
            pos,
            span,
            want: WantUpdate::Recompute,
            numbered: false,
            keep_end: false,
            failed: false,
        }
    }
}

/// 上一次行内查找：动作与查找的字符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Find {
    /// `f`、`F`、`t`、`T` 之一。
    pub motion: Motion,
    /// 查找的字符。
    pub target: char,
}

/// 动作求值的输入。
pub(crate) struct Env<'a, H: Host> {
    /// 宿主，提供文本与视图。
    pub host: &'a H,
    /// 光标位置。
    pub from: usize,
    /// 当前的期望列。
    pub want: Want,
    /// 次数；未输入时为 `None`。
    pub count: Option<u32>,
    /// 等待该动作的操作符。
    pub operator: Option<Operator>,
    /// 是否在可视模式下：光标可以停在行尾。
    pub visual: bool,
}

impl<H: Host> Env<'_, H> {
    /// 返回次数，未输入时为 1。
    fn times(&self) -> usize {
        self.count
            .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX))
    }

    /// 返回逐步移动的步数：次数与 [`MAX_STEPS`] 中较小的一个。
    fn steps(&self) -> usize {
        self.times().min(MAX_STEPS)
    }

    /// 返回第 `line` 行上光标可以停留的最后一个位置：可视模式下为行尾，否则为最后一个字符。
    fn line_limit(&self, line: usize) -> usize {
        let text = self.host.text();
        if self.visual {
            text.line_end(line)
        } else {
            text::last_char(text, line)
        }
    }

    /// 返回第 `line` 行上期望列 `want` 对应的位置。
    fn at_want(&self, line: usize, want: Want) -> usize {
        let text = self.host.text();
        match want {
            Want::End => self.line_limit(line),
            Want::Cell(cell) => text::pos_at_cell(text, line, cell, self.host.tab_width())
                .min(self.line_limit(line)),
        }
    }

    /// 返回从第 `line` 行向下（`down` 为真）或向上移动 `count` 行之后的行号，已折叠的区域算一行。
    ///
    /// 已在最后一行或第一行、无法移动时返回 `None`；次数超出时停在边界。
    fn line_by(&self, line: usize, count: usize, down: bool) -> Option<usize> {
        let text = self.host.text();
        let last = text::last_line(text);
        let folded = |line: usize| self.host.fold(line).unwrap_or((line, line));
        let (first, end) = folded(line);
        if count == 0 {
            return Some(line);
        }
        if (down && end >= last) || (!down && first == 0) {
            return None;
        }
        if !self.host.has_folds() {
            return Some(if down {
                line.saturating_add(count).min(last)
            } else {
                line.saturating_sub(count)
            });
        }
        let mut at = line;
        for _ in 0..count {
            let (first, end) = folded(at);
            if down {
                if end >= last {
                    break;
                }
                at = end + 1;
            } else {
                if first == 0 {
                    break;
                }
                at = folded(first - 1).0;
            }
        }
        Some(at)
    }
}

/// 求值上下移动、按行号与按窗口定位的动作。
fn vertical<H: Host>(env: &Env<'_, H>, movement: Movement) -> Option<Moved> {
    let text = env.host.text();
    let from = env.from;
    let line = text::line_of(text, from);

    match movement.motion {
        Motion::Up | Motion::Down => {
            let target = env.line_by(line, env.times(), movement.motion == Motion::Down)?;
            Some(Moved {
                want: WantUpdate::Keep,
                ..Moved::new(env.at_want(target, env.want), Span::Linewise)
            })
        }
        Motion::DisplayUp | Motion::DisplayDown => {
            let down = movement.motion == Motion::DisplayDown;
            let delta = isize::try_from(env.steps()).unwrap_or(isize::MAX);
            let cell = match env.want {
                Want::Cell(cell) => cell,
                Want::End => usize::MAX,
            };
            // 可以移动的行数不足次数时，光标移到边界，等待中的操作符不再执行。
            let mut failed = false;
            let pos = match env
                .host
                .display_vertical(from, if down { delta } else { -delta }, cell)
            {
                Some(pos) => pos,
                None => {
                    let target = env.line_by(line, env.times(), down)?;
                    failed = target.abs_diff(line) < env.times() && !env.host.has_folds();
                    env.at_want(target, env.want)
                }
            };
            (pos != from).then(|| Moved {
                want: WantUpdate::Keep,
                failed,
                ..Moved::new(
                    pos.min(env.line_limit(text::line_of(text, pos))),
                    Span::Exclusive,
                )
            })
        }
        Motion::LineUp | Motion::LineDown | Motion::LineHere => {
            let (count, down) = match movement.motion {
                Motion::LineUp => (env.times(), false),
                Motion::LineDown => (env.times(), true),
                _ => (env.times() - 1, true),
            };
            let target = env.line_by(line, count, down)?;
            // `_` 用于复制时光标的列不变。
            let pos = if movement.motion == Motion::LineHere && env.operator == Some(Operator::Yank)
            {
                if target == line {
                    from
                } else {
                    env.at_want(target, env.want)
                }
            } else {
                text::first_non_blank(text, target)
            };
            Some(Moved::new(pos, Span::Linewise))
        }
        Motion::FirstLine | Motion::LastLine | Motion::LastLineEnd => {
            let last = text::last_line(text);
            let target = match (env.count, movement.motion) {
                (Some(_), _) => (env.times() - 1).min(last),
                (None, Motion::FirstLine) => 0,
                (None, _) => last,
            };
            if movement.motion == Motion::LastLineEnd {
                return Some(Moved {
                    want: WantUpdate::Set(Want::End),
                    ..Moved::new(env.line_limit(target), Span::Inclusive)
                });
            }
            Some(Moved::new(
                text::first_non_blank(text, target),
                Span::Linewise,
            ))
        }
        Motion::Percent => match env.count {
            Some(percent) => {
                let percent = usize::try_from(percent).ok().filter(|p| *p <= 100)?;
                let lines = text::line_count(text);
                let target = ((lines * percent).div_ceil(100)).clamp(1, lines) - 1;
                Some(Moved::new(
                    text::first_non_blank(text, target),
                    Span::Linewise,
                ))
            }
            None => pair::matching(text, from).map(|pos| Moved {
                numbered: true,
                ..Moved::new(pos, Span::Inclusive)
            }),
        },
        Motion::WindowTop | Motion::WindowMiddle | Motion::WindowBottom => {
            let last = text::last_line(text);
            let visible = env.host.visible_lines();
            let top = visible.start.min(last);
            let bottom = visible.end.saturating_sub(1).clamp(top, last);
            let offset = env.times() - 1;
            // 带次数时 `H` 可以超出窗口的底部；`L` 只有作为操作符的范围时才超出窗口的顶部。
            let target = match movement.motion {
                Motion::WindowTop => top.saturating_add(offset).min(last),
                Motion::WindowBottom if env.operator.is_some() => bottom.saturating_sub(offset),
                Motion::WindowBottom => bottom.saturating_sub(offset).max(top),
                _ => top + (bottom - top + 2) / 2 - 1,
            };
            Some(Moved::new(
                text::first_non_blank(text, target),
                Span::Linewise,
            ))
        }
        _ => None,
    }
}

/// 求值动作 `movement`；动作无法执行时返回 `None`。
///
/// `last_find` 是上一次行内查找，`f`、`F`、`t`、`T` 执行时更新。
pub(crate) fn evaluate<H: Host>(
    env: &Env<'_, H>,
    movement: Movement,
    last_find: &mut Option<Find>,
) -> Option<Moved> {
    let text = env.host.text();
    let from = env.from;
    let line = text::line_of(text, from);
    let start = text.line_start(line);

    match movement.motion {
        Motion::Left => {
            let mut pos = from;
            for _ in 0..env.steps() {
                match text::prev_in_line(text, pos) {
                    Some(prev) => pos = prev,
                    None => break,
                }
            }
            // 无法移动时，等待中的操作符仍然执行，范围为空。
            (pos != from || env.operator.is_some()).then(|| Moved::new(pos, Span::Exclusive))
        }
        Motion::Backspace => backspace(env),
        Motion::Right => right(env, false),
        Motion::Space => right(env, true),
        Motion::LineStart => Some(Moved::new(start, Span::Exclusive)),
        Motion::FirstNonBlank => Some(Moved::new(
            text::first_non_blank(text, line),
            Span::Exclusive,
        )),
        Motion::LineEnd => {
            let target = env.line_by(line, env.times() - 1, true)?;
            Some(Moved {
                want: WantUpdate::Set(Want::End),
                ..Moved::new(env.line_limit(target), Span::Inclusive)
            })
        }
        Motion::LastNonBlank => {
            let target = env.line_by(line, env.times() - 1, true)?;
            let mut pos = text::last_char(text, target);
            let first = text.line_start(target);
            while pos > first && text.char_at(pos).is_some_and(text::is_blank) {
                pos = text::prev_in_line(text, pos).unwrap_or(first);
            }
            Some(Moved::new(pos, Span::Inclusive))
        }
        Motion::Column => {
            let cell = env.times() - 1;
            Some(Moved {
                want: WantUpdate::Set(Want::Cell(cell)),
                ..Moved::new(env.at_want(line, Want::Cell(cell)), Span::Exclusive)
            })
        }
        Motion::DisplayStart
        | Motion::DisplayFirstNonBlank
        | Motion::DisplayEnd
        | Motion::DisplayLastNonBlank
        | Motion::DisplayMiddle => display(env, movement.motion),
        Motion::FindForward | Motion::FindBackward | Motion::TillForward | Motion::TillBackward => {
            let target = movement.arg?;
            let find = Find {
                motion: movement.motion,
                target,
            };
            *last_find = Some(find);
            find_char(env, find, false)
        }
        Motion::RepeatFind | Motion::RepeatFindReverse => {
            let mut find = (*last_find)?;
            if movement.motion == Motion::RepeatFindReverse {
                find.motion = match find.motion {
                    Motion::FindForward => Motion::FindBackward,
                    Motion::FindBackward => Motion::FindForward,
                    Motion::TillForward => Motion::TillBackward,
                    _ => Motion::TillForward,
                };
            }
            find_char(env, find, true)
        }
        Motion::Up
        | Motion::Down
        | Motion::DisplayUp
        | Motion::DisplayDown
        | Motion::LineUp
        | Motion::LineDown
        | Motion::LineHere
        | Motion::FirstLine
        | Motion::LastLine
        | Motion::LastLineEnd
        | Motion::Percent
        | Motion::WindowTop
        | Motion::WindowMiddle
        | Motion::WindowBottom => vertical(env, movement),
        Motion::WordForward | Motion::BigWordForward => {
            word::forward(env, movement.motion == Motion::BigWordForward)
        }
        Motion::WordEnd | Motion::BigWordEnd => {
            word::end(env, movement.motion == Motion::BigWordEnd)
        }
        Motion::WordBackward | Motion::BigWordBackward => {
            word::backward(env, movement.motion == Motion::BigWordBackward)
        }
        Motion::WordEndBackward | Motion::BigWordEndBackward => {
            word::end_backward(env, movement.motion == Motion::BigWordEndBackward)
        }
        Motion::SentenceBackward | Motion::SentenceForward => {
            block::sentence(env, movement.motion == Motion::SentenceForward)
        }
        Motion::ParagraphBackward | Motion::ParagraphForward => {
            block::paragraph(env, movement.motion == Motion::ParagraphForward)
        }
        Motion::SectionBackward
        | Motion::SectionForward
        | Motion::SectionEndBackward
        | Motion::SectionEndForward => block::section(env, movement.motion),
        Motion::UnmatchedParenBackward
        | Motion::UnmatchedBraceBackward
        | Motion::UnmatchedParenForward
        | Motion::UnmatchedBraceForward => {
            let (open, close, forward) = match movement.motion {
                Motion::UnmatchedParenBackward => ('(', ')', false),
                Motion::UnmatchedBraceBackward => ('{', '}', false),
                Motion::UnmatchedParenForward => ('(', ')', true),
                _ => ('{', '}', true),
            };
            let mut pos = None;
            for _ in 0..env.steps() {
                match pair::unmatched(text, pos.unwrap_or(from), open, close, forward) {
                    Some(found) => pos = Some(found),
                    None => break,
                }
            }
            pos.map(|pos| Moved::new(pos, Span::Exclusive))
        }
        // 查找与标记由状态机求值。
        Motion::SearchForward
        | Motion::SearchBackward
        | Motion::SearchNext
        | Motion::SearchPrevious
        | Motion::SearchWordForward
        | Motion::SearchWordBackward
        | Motion::SearchPartForward
        | Motion::SearchPartBackward
        | Motion::Mark
        | Motion::MarkLine => None,
    }
}

/// `<BS>`：向左移动，在行首时到上一行。
fn backspace<H: Host>(env: &Env<'_, H>) -> Option<Moved> {
    let text = env.host.text();
    let mut pos = env.from;
    let mut keep_end = false;
    for _ in 0..env.steps() {
        if let Some(prev) = text::prev_in_line(text, pos) {
            pos = prev;
            continue;
        }
        let line = text.line_of(pos);
        if line == 0 {
            break;
        }
        // 删除与修改时停在上一行的行尾，换行符因此落在范围之内。
        let deleting = matches!(env.operator, Some(Operator::Delete | Operator::Change));
        if deleting && text.line_len(line - 1) > 0 {
            pos = text.line_end(line - 1);
            keep_end = true;
        } else {
            pos = env.line_limit(line - 1);
        }
    }
    (pos != env.from || env.operator.is_some()).then(|| Moved {
        keep_end,
        ..Moved::new(pos, Span::Exclusive)
    })
}

/// `l` 与 `<Space>`：向右移动；`wrap` 为真时在行尾到下一行行首。
fn right<H: Host>(env: &Env<'_, H>, wrap: bool) -> Option<Moved> {
    let text = env.host.text();
    let last = text::last_line(text);
    let mut pos = env.from;
    let mut inclusive = false;
    for step in 0..env.steps() {
        let line = text.line_of(pos);
        let end = text.line_end(line);
        let next = text::next_in_line(text, pos).filter(|next| env.visual || *next < end);
        if let Some(next) = next {
            pos = next;
            continue;
        }
        if wrap && line < last {
            // 有等待中的操作符时，非空行的最后一个字符先算一次，换行符再算一次。
            if env.operator.is_some() && !inclusive && text.line_len(line) > 0 {
                inclusive = true;
            } else {
                pos = text.line_start(line + 1);
                inclusive = false;
            }
            continue;
        }
        if env.operator.is_none() {
            if step == 0 {
                return None;
            }
        } else if text.line_len(line) > 0 {
            inclusive = true;
        }
        break;
    }
    let span = if inclusive {
        Span::Inclusive
    } else {
        Span::Exclusive
    };
    Some(Moved::new(pos, span))
}

/// `g0`、`g^`、`g$`、`gm`：在光标所在的显示行内移动。
fn display<H: Host>(env: &Env<'_, H>, motion: Motion) -> Option<Moved> {
    let text = env.host.text();
    let tab_width = env.host.tab_width();
    let mut row = env.host.display_row(env.from);
    match motion {
        Motion::DisplayStart => Some(Moved::new(row.start, Span::Exclusive)),
        Motion::DisplayFirstNonBlank => {
            let mut pos = row.start;
            while pos < row.end && text.char_at(pos).is_some_and(text::is_blank) {
                match text::next_in_line(text, pos) {
                    Some(next) if next < row.end => pos = next,
                    _ => break,
                }
            }
            Some(Moved::new(pos, Span::Exclusive))
        }
        Motion::DisplayEnd | Motion::DisplayLastNonBlank => {
            // 带次数时先向下移动「次数减一」个显示行。
            // 无法向下移动足够的行数时仍移到显示行的末尾，等待中的操作符不再执行。
            let down = env.times() - 1;
            let mut failed = false;
            if down > 0 {
                let delta = isize::try_from(down.min(MAX_STEPS)).unwrap_or(isize::MAX);
                let from = text::line_of(text, env.from);
                let pos = match env.host.display_vertical(env.from, delta, 0) {
                    Some(pos) => pos,
                    None => match env.line_by(from, down, true) {
                        Some(line) => {
                            failed = line - from < down && !env.host.has_folds();
                            text.line_start(line)
                        }
                        None => {
                            failed = true;
                            env.from
                        }
                    },
                };
                row = env.host.display_row(pos);
            }
            let line = text::line_of(text, row.start);
            let at_line_end = row.end >= text.line_end(line);
            let mut pos = if at_line_end {
                env.line_limit(line)
            } else {
                text::prev_in_line(text, row.end).unwrap_or(row.start)
            };
            // `g<End>` 停在最后一个非空白字符上，之后的上下移动不再保持在行尾。
            let last_non_blank = motion == Motion::DisplayLastNonBlank;
            if last_non_blank {
                pos = pos.min(text::last_char(text, line));
                while pos > row.start && text.char_at(pos).is_some_and(text::is_blank) {
                    pos = text::prev_in_line(text, pos).unwrap_or(row.start);
                }
            }
            let at_line_end = at_line_end && !last_non_blank;
            Some(Moved {
                want: if at_line_end {
                    WantUpdate::Set(Want::End)
                } else {
                    WantUpdate::Recompute
                },
                failed,
                ..Moved::new(pos, Span::Inclusive)
            })
        }
        _ => {
            let (line, col) = text.pos_to_line_col(row.start);
            let cell = text::cell_of(text, line, col, tab_width) + env.host.text_width() / 2;
            Some(Moved::new(
                env.at_want(line, Want::Cell(cell)),
                Span::Exclusive,
            ))
        }
    }
}

/// 执行行内查找 `find`；`repeating` 表示由 `;` 或 `,` 触发。
fn find_char<H: Host>(env: &Env<'_, H>, find: Find, repeating: bool) -> Option<Moved> {
    let text = env.host.text();
    let forward = matches!(find.motion, Motion::FindForward | Motion::TillForward);
    let till = matches!(find.motion, Motion::TillForward | Motion::TillBackward);
    let count = env.times();
    let mut scan = Scan::new(text, env.from);
    let line = scan.line;
    // 重复 `t`、`T` 且次数为 1 时，紧邻光标的匹配不算，光标因此总会移动。
    let mut stop = !(repeating && till && count == 1);
    let mut col = scan.col;
    let mut budget = MAX_FIND;
    for _ in 0..count.min(MAX_STEPS) {
        loop {
            budget = budget.checked_sub(1)?;
            if forward {
                col += 1;
                if col >= scan.len() {
                    return None;
                }
            } else {
                col = col.checked_sub(1)?;
            }
            if scan.char_at(col) == Some(find.target) && stop {
                break;
            }
            stop = true;
        }
    }
    scan.set(line, col);
    if till {
        if forward {
            scan.dec();
        } else {
            scan.inc();
        }
    }
    let pos = winc_core::movement::snap_to_grapheme(text, scan.pos());
    let span = if forward {
        Span::Inclusive
    } else {
        Span::Exclusive
    };
    Some(Moved::new(pos, span))
}
