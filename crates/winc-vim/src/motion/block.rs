//! 按句、按段、按节移动：`(` `)` `{` `}` `[[` `]]` `[]` `][`。
//!
//! 段落以空行分隔；节以第一列是 `{` 或 `}` 的行分隔，换页符开头的行同时是段落与节的边界。
//! 句子止于 `.`、`!`、`?`，其后可以跟 `)`、`]`、`"`、`'`，再后是空白或行尾。

use winc_core::Lines;

use super::{Env, Moved, Span};
use crate::command::Motion;
use crate::host::Host;
use crate::text::{self, Scan, Step};

/// 第 `line` 行是否为段落或节的边界：空行（`mark` 为 `None`）或以 `mark` 开头的行，以及换页符
/// 开头的行；`both` 为真时第一列是 `}` 的行也算。
pub(crate) fn boundary(text: &impl Lines, line: usize, mark: Option<char>, both: bool) -> bool {
    let content = text.line_prefix(line, 4);
    let first = content.slice().get_char(0);
    match first {
        None => mark.is_none(),
        Some('\u{c}') => true,
        Some('}') if both => true,
        Some(c) => mark == Some(c),
    }
}

/// `{`、`}`：上一个或下一个段落的边界。
pub(super) fn paragraph<H: Host>(env: &Env<'_, H>, forward: bool) -> Option<Moved> {
    let (pos, inclusive) = boundary_line(env, forward, None, false)?;
    let span = if inclusive {
        Span::Inclusive
    } else {
        Span::Exclusive
    };
    Some(Moved {
        numbered: true,
        ..Moved::new(pos, span)
    })
}

/// `[[`、`]]`、`[]`、`][`：上一个或下一个节的边界。
pub(super) fn section<H: Host>(env: &Env<'_, H>, motion: Motion) -> Option<Moved> {
    let forward = matches!(motion, Motion::SectionForward | Motion::SectionEndForward);
    let mark = match motion {
        Motion::SectionBackward | Motion::SectionForward => '{',
        _ => '}',
    };
    // 有等待中的操作符时，`]]` 也停在第一列是 `}` 的行上。
    let both = env.operator.is_some() && forward && mark == '{';
    let (pos, inclusive) = boundary_line(env, forward, Some(mark), both)?;
    let text = env.host.text();
    if env.operator.is_none() {
        let line = text.line_of(pos);
        return Some(Moved::new(
            text::first_non_blank(text, line),
            Span::Exclusive,
        ));
    }
    let span = if inclusive {
        Span::Inclusive
    } else {
        Span::Exclusive
    };
    Some(Moved::new(pos, span))
}

/// 从光标所在的行出发，按次数找到边界行，返回目标位置与是否包含终点。
///
/// 向下到达最后一行仍未找到时，目标是最后一行的最后一个字符，并包含终点（节的结尾 `][` 除外）；
/// 次数未用完就到达文本的开头或末尾时返回 `None`。
fn boundary_line<H: Host>(
    env: &Env<'_, H>,
    forward: bool,
    mark: Option<char>,
    both: bool,
) -> Option<(usize, bool)> {
    let text = env.host.text();
    let last = text::last_line(text);
    let mut line = text::line_of(text, env.from);
    let count = env.steps();
    for remaining in (0..count).rev() {
        let mut seen_text = false;
        let mut first = true;
        loop {
            if text.line_len(line) > 0 {
                seen_text = true;
            }
            // 折叠起来的区域整体跳过。
            if first && let Some((start, end)) = env.host.fold(line) {
                line = if forward { end } else { start };
            } else if !first && seen_text && boundary(text, line, mark, both) {
                break;
            }
            first = false;
            let next = if forward {
                (line < last).then(|| line + 1)
            } else {
                line.checked_sub(1)
            };
            match next {
                Some(next) => line = next,
                None if remaining > 0 => return None,
                None => break,
            }
        }
    }
    if both && text.char_at(text.line_start(line)) == Some('}') && text.line_len(line) > 0 {
        line = (line + 1).min(last);
    }
    if line == last && mark != Some('}') && forward {
        let end = text::last_char(text, line);
        return Some((end, text.line_len(line) > 0));
    }
    Some((text.line_start(line), false))
}

/// 是否为句末标点。
fn ends_sentence(c: Option<char>) -> bool {
    matches!(c, Some('.' | '!' | '?'))
}

/// 是否为可以跟在句末标点之后的右括号或引号。
fn closes(c: Option<char>) -> bool {
    matches!(c, Some(')' | ']' | '"' | '\''))
}

/// `(`、`)`：上一个或下一个句子的开头。
pub(super) fn sentence<H: Host>(env: &Env<'_, H>, forward: bool) -> Option<Moved> {
    let text = env.host.text();
    let mut scan = Scan::new(text, find_sentence(text, env.from, forward, env.steps())?);
    // 目标落在行尾时退到最后一个字符，并改为包含终点。
    let mut span = Span::Exclusive;
    if scan.col > 0 && scan.char().is_none() && !env.visual {
        scan.dec();
        span = Span::Inclusive;
    }
    Some(Moved {
        numbered: true,
        ..Moved::new(scan.pos(), span)
    })
}

/// 返回从 `from` 向后（`forward` 为真）或向前第 `count` 个句子开头的位置，可以落在行尾。
///
/// 次数未用完就到达文本的开头或末尾时返回 `None`。
pub(crate) fn find_sentence<T: Lines>(
    text: &T,
    from: usize,
    forward: bool,
    count: usize,
) -> Option<usize> {
    let mut scan = Scan::new(text, from);
    let step = |scan: &mut Scan<'_, T>| {
        if forward { scan.incl() } else { scan.decl() }
    };
    for remaining in (0..count).rev() {
        let mut skip_blanks = true;
        'found: {
            if scan.char().is_none() {
                // 在空行上：移到相邻的非空行。
                loop {
                    if step(&mut scan) == Step::Edge {
                        break;
                    }
                    if scan.char().is_some() {
                        break;
                    }
                }
                if forward {
                    break 'found;
                }
            } else if forward && scan.col == 0 && boundary(text, scan.line, None, false) {
                // 在段落边界的行首：移到下一行。
                if scan.on_last_line() {
                    return None;
                }
                scan.set(scan.line + 1, 0);
                break 'found;
            } else if !forward {
                scan.decl();
            }

            // 退回到上一个既不是空白也不是句末标点的字符。
            let mut found_dot = false;
            loop {
                let c = scan.char();
                if !(c.is_some_and(text::is_blank) || ends_sentence(c) || closes(c)) {
                    break;
                }
                let mut before = Scan::new(text, scan.pos());
                if before.decl() == Step::Edge || (before.on_empty_line() && forward) {
                    break;
                }
                if found_dot {
                    break;
                }
                if ends_sentence(c) {
                    found_dot = true;
                }
                let prev = before.char();
                if closes(c) && !(ends_sentence(prev) || closes(prev)) {
                    break;
                }
                scan.decl();
            }

            // 找到句子的结尾。
            let start_line = scan.line;
            loop {
                let c = scan.char();
                if c.is_none() || (scan.col == 0 && boundary(text, scan.line, None, false)) {
                    if !forward && scan.line != start_line {
                        scan.set(scan.line + 1, 0);
                    }
                    break;
                }
                if ends_sentence(c) {
                    let mut ahead = Scan::new(text, scan.pos());
                    let mut edge = false;
                    loop {
                        if ahead.inc() == Step::Edge {
                            edge = true;
                            break;
                        }
                        if !closes(ahead.char()) {
                            break;
                        }
                    }
                    let next = ahead.char();
                    if edge || next.is_none() || next.is_some_and(text::is_blank) {
                        scan.set(ahead.line, ahead.col);
                        if scan.char().is_none() {
                            scan.inc();
                        }
                        break;
                    }
                }
                if step(&mut scan) == Step::Edge {
                    if remaining > 0 {
                        return None;
                    }
                    skip_blanks = false;
                    break;
                }
            }
        }
        // 跳过句子之后的空白。
        while skip_blanks && scan.char().is_some_and(text::is_blank) {
            if scan.incl() == Step::Edge {
                break;
            }
        }
    }

    Some(scan.pos())
}
