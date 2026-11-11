//! 按词移动：`w` `W` `b` `B` `e` `E` `ge` `gE`。
//!
//! 词是一串同类的字符（见 [`crate::text::class`]）；空行也算一个词。带 `big` 的形式只以空白
//! 分隔。

use winc_core::Lines;

use super::{Env, Moved, Span};
use crate::command::Operator;
use crate::host::Host;
use crate::text::{self, Scan, Step};

/// `w`、`W`：下一个词的开头。等待修改操作符且光标在非空白字符上时改为移到词尾。
pub(super) fn forward<H: Host>(env: &Env<'_, H>, big: bool) -> Option<Moved> {
    let text = env.host.text();
    let mut scan = Scan::new(text, env.from);
    let changing =
        env.operator == Some(Operator::Change) && scan.char().is_some_and(|c| !text::is_blank(c));
    if changing {
        to_end(&mut scan, env.steps(), big, true);
    } else {
        to_next(&mut scan, env.steps(), big, env.operator.is_some());
    }
    finish(env, scan, changing)
}

/// `e`、`E`：词尾。
pub(super) fn end<H: Host>(env: &Env<'_, H>, big: bool) -> Option<Moved> {
    let mut scan = Scan::new(env.host.text(), env.from);
    to_end(&mut scan, env.steps(), big, false);
    finish(env, scan, true)
}

/// 结束向前的按词移动：目标落在行尾时退到最后一个字符，并改为包含终点。
fn finish<H: Host, T: Lines>(
    env: &Env<'_, H>,
    mut scan: Scan<'_, T>,
    inclusive: bool,
) -> Option<Moved> {
    let mut inclusive = inclusive;
    if scan.pos() > env.from && scan.col > 0 && scan.char().is_none() && !env.visual {
        scan.dec();
        inclusive = true;
    }
    let pos = scan.pos();
    if pos == env.from && env.operator.is_none() {
        return None;
    }
    let span = if inclusive {
        Span::Inclusive
    } else {
        Span::Exclusive
    };
    Some(Moved::new(pos, span))
}

/// `b`、`B`：上一个词的开头。
pub(super) fn backward<H: Host>(env: &Env<'_, H>, big: bool) -> Option<Moved> {
    let mut scan = Scan::new(env.host.text(), env.from);
    let mut failed = false;
    'counts: for _ in 0..env.steps() {
        if scan.dec() == Step::Edge {
            failed = true;
            break;
        }
        // 跳过词之前的空白，停在空行上。
        while scan.class(big) == 0 {
            if scan.col == 0 && scan.on_empty_line() {
                continue 'counts;
            }
            if scan.dec() == Step::Edge {
                break 'counts;
            }
        }
        // 移到所在词的开头。
        let class = scan.class(big);
        while scan.class(big) == class {
            if scan.dec() == Step::Edge {
                break 'counts;
            }
        }
        scan.inc();
    }
    let pos = scan.pos();
    (pos != env.from).then(|| Moved {
        failed,
        ..Moved::new(pos, Span::Exclusive)
    })
}

/// `ge`、`gE`：上一个词的词尾。
pub(super) fn end_backward<H: Host>(env: &Env<'_, H>, big: bool) -> Option<Moved> {
    let mut scan = Scan::new(env.host.text(), env.from);
    let mut failed = false;
    'counts: for _ in 0..env.steps() {
        let start = scan.class(big);
        if scan.dec() == Step::Edge {
            failed = true;
            break;
        }
        // 移到当前词的开头之前。
        if start != 0 {
            while scan.class(big) == start {
                if scan.dec() == Step::Edge {
                    break 'counts;
                }
            }
        }
        // 移到上一个词的词尾，停在空行上。
        while scan.class(big) == 0 {
            if scan.col == 0 && scan.on_empty_line() {
                break;
            }
            if scan.dec() == Step::Edge {
                break 'counts;
            }
        }
    }
    let pos = scan.pos();
    (pos != env.from).then(|| Moved {
        failed,
        ..Moved::new(pos, Span::Inclusive)
    })
}

/// 把 `scan` 移到之后第 `count` 个词的开头。
///
/// `stop_at_line_end` 为真时（有等待中的操作符），最后一次移动不越过行尾：操作符不会连同
/// 换行符一起作用。
fn to_next<T: Lines>(scan: &mut Scan<'_, T>, count: usize, big: bool, stop_at_line_end: bool) {
    for remaining in (0..count).rev() {
        let start = scan.class(big);
        let last_line = scan.on_last_line();
        // 至少移动一个字符；已在文本的最后一个字符上时结束。
        let step = scan.inc();
        if step == Step::Edge || (step.left_line() && last_line) {
            return;
        }
        if step.left_line() && stop_at_line_end && remaining == 0 {
            return;
        }
        // 移到当前词的末尾之后。
        if start != 0 {
            while scan.class(big) == start {
                let step = scan.inc();
                if step == Step::Edge || (step.left_line() && stop_at_line_end && remaining == 0) {
                    return;
                }
            }
        }
        // 跳过空白，停在空行上。
        while scan.class(big) == 0 {
            if scan.col == 0 && scan.on_empty_line() {
                break;
            }
            let step = scan.inc();
            if step == Step::Edge || (step.left_line() && last_line) {
                return;
            }
            if step.left_line() && stop_at_line_end && remaining == 0 {
                return;
            }
        }
    }
}

/// 把 `scan` 移到之后第 `count` 个词尾。
///
/// `stay` 为真时（由修改操作符的 `w` 转来），已在词尾时第一次移动只算光标下的字符。
fn to_end<T: Lines>(scan: &mut Scan<'_, T>, count: usize, big: bool, stay: bool) {
    let mut stay = stay;
    for _ in 0..count {
        let start = scan.class(big);
        if scan.inc() == Step::Edge {
            return;
        }
        if scan.class(big) == start && start != 0 {
            // 在词的中间：移到所在词的末尾之后。
            if skip(scan, start, big) {
                return;
            }
        } else if !stay || start == 0 {
            // 在词尾：跳过空白，移到下一个词的末尾之后。
            while scan.class(big) == 0 {
                if scan.inc() == Step::Edge {
                    return;
                }
            }
            let class = scan.class(big);
            if skip(scan, class, big) {
                return;
            }
        }
        scan.dec();
        stay = false;
    }
}

/// 向前越过类别为 `class` 的字符；到达文本末尾时返回真。
fn skip<T: Lines>(scan: &mut Scan<'_, T>, class: u32, big: bool) -> bool {
    while scan.class(big) == class {
        if scan.inc() == Step::Edge {
            return true;
        }
    }
    false
}
