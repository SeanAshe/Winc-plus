//! 文本对象：由光标位置（可视模式下连同选区的起点）算出一段文本的两端。
//!
//! 等待动作时结果是操作符的范围；可视模式下结果是新的选区，已有的选区不止一个字符时在其基础
//! 上延伸。词、句、段落、括号、引号与标签各有一个函数。

use winc_core::Lines;

use crate::command::{Shape, TextObject};
use crate::motion::block::{boundary, find_sentence};
use crate::motion::pair;
use crate::text::{self, Scan, Step};

/// 文本对象选中之后可视模式的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Becomes {
    /// 不变。
    Same,
    /// 按行的选区改为按字符，其余不变。
    CharUnlessBlock,
    /// 按字符。
    Char,
    /// 按行。
    Line,
}

/// 文本对象的求值结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selected {
    /// 范围的起点；可视模式下是选区新的起点字符。
    pub start: usize,
    /// 范围的终点；可视模式下是光标新的位置。
    pub end: usize,
    /// 终点上的字符是否在范围之内。
    pub inclusive: bool,
    /// 范围是否按行。
    pub linewise: bool,
    /// 可视模式的种类如何变化。
    pub becomes: Becomes,
    /// 次数没有用完就到达了文本的一端：可视模式下选区仍按已得到的结果更新，操作符不执行。
    pub failed: bool,
}

/// 求值的输入。
#[derive(Debug, Clone, Copy)]
pub(crate) struct At {
    /// 光标位置。
    pub cursor: usize,
    /// 可视模式下选区的起点字符；等待动作时为 `None`。
    pub anchor: Option<usize>,
    /// 可视模式是否按行。
    pub linewise: bool,
    /// 次数，不小于 1。
    pub count: usize,
    /// 制表符的宽度（显示列），供比较缩进。
    pub tab_width: usize,
}

/// 求值文本对象 `object`；光标处没有该对象时返回 `None`。
pub(crate) fn select<T: Lines>(text: &T, object: TextObject, at: At) -> Option<Selected> {
    let around = object.around;
    match object.shape {
        Shape::Word => word(text, at, around, false),
        Shape::BigWord => word(text, at, around, true),
        Shape::Sentence => sentence(text, at, around),
        Shape::Paragraph => paragraph(text, at, around),
        Shape::Paren => block(text, at, around, '(', ')'),
        Shape::Bracket => block(text, at, around, '[', ']'),
        Shape::Brace => block(text, at, around, '{', '}'),
        Shape::Angle => block(text, at, around, '<', '>'),
        Shape::Tag => tag(text, at, around),
        Shape::DoubleQuote => quote(text, at, around, '"'),
        Shape::SingleQuote => quote(text, at, around, '\''),
        Shape::Backtick => quote(text, at, around, '`'),
        Shape::Indent => indent(text, at, around),
        // 查找的匹配由状态机求值。
        Shape::NextMatch | Shape::PreviousMatch => None,
    }
}

/// `ii`、`ai`：缩进不小于光标所在行的连续各行，中间的空行算在内，两端的空行不算；`around`
/// 为真时连同其上缩进更小的一行。光标在空行上时取最近的非空行的缩进。
fn indent<T: Lines>(text: &T, at: At, around: bool) -> Option<Selected> {
    let last = text::last_line(text);
    let blank = |line: usize| text::indent_chars(text, line) == text.line_len(line);
    let cells =
        |line: usize| text::cell_of(text, line, text::indent_chars(text, line), at.tab_width);
    let here = text::line_of(text, at.cursor);
    let base_line = (here..=last)
        .chain((0..here).rev())
        .find(|line| !blank(*line))?;
    let base = cells(base_line);
    let inside = |line: &usize| blank(*line) || cells(*line) >= base;
    let mut first = (0..=base_line).rev().take_while(inside).last()?;
    let mut end = (base_line..=last).take_while(inside).last()?;
    while first < base_line && blank(first) {
        first += 1;
    }
    while end > base_line && blank(end) {
        end -= 1;
    }
    if around && let Some(above) = (0..first).rev().find(|line| !blank(*line)) {
        first = above;
    }
    Some(Selected {
        start: text.line_start(first),
        end: text.line_start(end),
        inclusive: true,
        linewise: true,
        becomes: Becomes::Line,
        failed: false,
    })
}

/// 返回 `pos` 处的字符；位于行尾时返回 `None`。
fn char_at(text: &impl Lines, pos: usize) -> Option<char> {
    let line = text::line_of(text, pos);
    (pos < text.line_end(line))
        .then(|| text.char_at(pos))
        .flatten()
}

/// `pos` 处是否为空格或制表符。
fn blank_at(text: &impl Lines, pos: usize) -> bool {
    char_at(text, pos).is_some_and(text::is_blank)
}

/// 返回 `pos` 之后一个字符的位置，越过非空行的行尾，以及是否已到文本的末尾。
///
/// 从最后一个字符出发时到达文本的末尾，位置仍移到其后的行尾。
fn incl(text: &impl Lines, pos: usize) -> (usize, bool) {
    let mut scan = Scan::new(text, pos);
    let edge = scan.incl() == Step::Edge;
    (scan.pos(), edge)
}

/// 返回 `pos` 之前一个字符的位置，越过非空行的行尾；已在文本开头时返回 `None`。
fn decl(text: &impl Lines, pos: usize) -> Option<usize> {
    let mut scan = Scan::new(text, pos);
    (scan.decl() != Step::Edge).then(|| scan.pos())
}

/// 在行内左移一个字符；已在行首时返回假。
fn one_left<T: Lines>(scan: &mut Scan<'_, T>) -> bool {
    if scan.col == 0 {
        return false;
    }
    scan.dec();
    true
}

/// 移到光标所在的一串同类字符的开头，不越过行首。
fn back_in_line<T: Lines>(scan: &mut Scan<'_, T>, big: bool) {
    let class = scan.class(big);
    while scan.col > 0 {
        scan.dec();
        if scan.class(big) != class {
            scan.inc();
            break;
        }
    }
}

/// 移到下一个词的开头；`eol` 为真时不越过行尾。已在文本的最后一个字符上时返回假。
fn fwd_word<T: Lines>(scan: &mut Scan<'_, T>, big: bool, eol: bool) -> bool {
    let start = scan.class(big);
    let last_line = scan.on_last_line();
    let step = scan.inc();
    if step == Step::Edge || (step.left_line() && last_line) {
        return false;
    }
    if step.left_line() && eol {
        return true;
    }
    if start != 0 {
        while scan.class(big) == start {
            let step = scan.inc();
            if step == Step::Edge || (step.left_line() && eol) {
                return true;
            }
        }
    }
    while scan.class(big) == 0 {
        if scan.col == 0 && scan.on_empty_line() {
            break;
        }
        let step = scan.inc();
        if step == Step::Edge || (step.left_line() && (last_line || eol)) {
            return true;
        }
    }
    true
}

/// 移到词尾；`stop` 为真时已在词尾则不动，空行算一个词。到达文本末尾时返回假。
fn end_word<T: Lines>(scan: &mut Scan<'_, T>, big: bool, stop: bool) -> bool {
    let start = scan.class(big);
    if scan.inc() == Step::Edge {
        return false;
    }
    if scan.class(big) == start && start != 0 {
        if skip(scan, start, big, true) {
            return false;
        }
    } else if !stop || start == 0 {
        while scan.class(big) == 0 {
            if scan.col == 0 && scan.on_empty_line() {
                return true;
            }
            if scan.inc() == Step::Edge {
                return false;
            }
        }
        let class = scan.class(big);
        if skip(scan, class, big, true) {
            return false;
        }
    }
    scan.dec();
    true
}

/// 移到上一个词的开头；`stop` 的含义同 [`end_word`]。已在文本开头时返回假。
fn bck_word<T: Lines>(scan: &mut Scan<'_, T>, big: bool, stop: bool) -> bool {
    let start = scan.class(big);
    if scan.dec() == Step::Edge {
        return false;
    }
    if !stop || start == scan.class(big) || start == 0 {
        while scan.class(big) == 0 {
            if scan.col == 0 && scan.on_empty_line() {
                return true;
            }
            if scan.dec() == Step::Edge {
                return true;
            }
        }
        let class = scan.class(big);
        if skip(scan, class, big, false) {
            return true;
        }
    }
    scan.inc();
    true
}

/// 移到上一个词的词尾，不越过行首。已在文本开头时返回假。
fn bckend_word<T: Lines>(scan: &mut Scan<'_, T>, big: bool) -> bool {
    let start = scan.class(big);
    match scan.dec() {
        Step::Edge => return false,
        Step::Line => return true,
        _ => {}
    }
    if start != 0 {
        while scan.class(big) == start {
            if matches!(scan.dec(), Step::Edge | Step::Line) {
                return true;
            }
        }
    }
    while scan.class(big) == 0 {
        if scan.col == 0 && scan.on_empty_line() {
            break;
        }
        if matches!(scan.dec(), Step::Edge | Step::Line) {
            return true;
        }
    }
    true
}

/// 向前（`forward` 为真）或向后越过类别为 `class` 的字符；到达文本的一端时返回真。
fn skip<T: Lines>(scan: &mut Scan<'_, T>, class: u32, big: bool, forward: bool) -> bool {
    while scan.class(big) == class {
        let step = if forward { scan.inc() } else { scan.dec() };
        if step == Step::Edge {
            return true;
        }
    }
    false
}

/// `iw`、`aw`、`iW`、`aW`。
fn word<T: Lines>(text: &T, at: At, around: bool, big: bool) -> Option<Selected> {
    let mut scan = Scan::new(text, at.cursor);
    let mut anchor = at.anchor;
    let mut start = None;
    let mut count = at.count;
    let mut inclusive = true;
    let mut include_white = false;

    if anchor.is_none_or(|anchor| anchor == at.cursor) {
        back_in_line(&mut scan, big);
        let begin = scan.pos();
        start = Some(begin);
        if (scan.class(big) == 0) == around {
            if !end_word(&mut scan, big, true) {
                return None;
            }
        } else {
            fwd_word(&mut scan, big, true);
            if scan.col == 0 {
                scan.decl();
            } else {
                one_left(&mut scan);
            }
            include_white = around;
        }
        if anchor.is_some() {
            anchor = start;
        }
        count -= 1;
    }

    let mut failed = false;
    while count > 0 {
        inclusive = true;
        if anchor.is_some_and(|anchor| scan.pos() < anchor) {
            // 可视模式下光标在选区的起点：向前文延伸。
            if scan.decl() == Step::Edge {
                failed = true;
                break;
            }
            if around != (scan.class(big) != 0) {
                if !bck_word(&mut scan, big, true) {
                    failed = true;
                    break;
                }
            } else {
                if !bckend_word(&mut scan, big) {
                    failed = true;
                    break;
                }
                scan.incl();
            }
        } else {
            if scan.incl() == Step::Edge {
                failed = true;
                break;
            }
            if around != (scan.class(big) == 0) {
                if !fwd_word(&mut scan, big, true) && count > 1 {
                    failed = true;
                    break;
                }
                // 终点落在下一行行首时不把换行符算在内。
                if !one_left(&mut scan) {
                    inclusive = false;
                }
            } else if !end_word(&mut scan, big, true) {
                failed = true;
                break;
            }
        }
        count -= 1;
    }
    if failed {
        return Some(Selected {
            start: anchor.or(start)?,
            end: scan.pos(),
            inclusive,
            linewise: false,
            becomes: Becomes::CharUnlessBlock,
            failed: true,
        });
    }

    // 末尾没有空白时改为包含词之前的空白，行首的缩进除外。
    if include_white
        && (scan.class(big) != 0 || (scan.col == 0 && !inclusive))
        && let Some(begin) = start
    {
        let mut before = Scan::new(text, begin);
        if one_left(&mut before) {
            back_in_line(&mut before, big);
            if before.class(big) == 0 && before.col > 0 {
                start = Some(before.pos());
                if anchor.is_some() {
                    anchor = start;
                }
            }
        }
    }

    Some(Selected {
        start: anchor.or(start)?,
        end: scan.pos(),
        inclusive,
        linewise: false,
        becomes: Becomes::CharUnlessBlock,
        failed: false,
    })
}

/// 返回 `from` 所在位置向后或向前一个句子开头的位置；无法移动时返回 `from`。
fn sent<T: Lines>(text: &T, from: usize, forward: bool) -> usize {
    find_sentence(text, from, forward, 1).unwrap_or(from)
}

/// 向前文越过空白，停在这段空白的第一个字符上。
fn first_blank(text: &impl Lines, pos: usize) -> usize {
    let mut pos = pos;
    while let Some(prev) = decl(text, pos) {
        pos = prev;
        if !blank_at(text, pos) {
            return incl(text, pos).0;
        }
    }
    pos
}

/// 从句子的开头（`at_start` 为真）或其后的空白出发，向后选取 `count` 个句子或空白。
fn sentences_forward<T: Lines>(text: &T, from: usize, count: usize, at_start: bool) -> usize {
    let mut pos = from;
    let mut at_start = at_start;
    for remaining in (0..count).rev() {
        pos = sent(text, pos, true);
        if at_start {
            pos = first_blank(text, pos);
        }
        if remaining == 0 || at_start {
            pos = decl(text, pos).unwrap_or(pos);
        }
        at_start = !at_start;
    }
    pos
}

/// `is`、`as`。
fn sentence<T: Lines>(text: &T, at: At, around: bool) -> Option<Selected> {
    let mut start = at.cursor;
    let mut pos = start;
    let mut cursor = sent(text, at.cursor, true);
    let mut count = at.count;

    if at.anchor.is_none_or(|anchor| anchor == at.cursor) {
        // 光标在空白上时，判断其后是否紧接着下一个句子。
        while blank_at(text, pos) {
            pos = incl(text, pos).0;
        }
        let start_blank = pos == cursor;
        if start_blank {
            start = first_blank(text, start);
        } else {
            cursor = sent(text, cursor, false);
            start = cursor;
        }
        let pieces = if around {
            count * 2
        } else {
            count - usize::from(start_blank)
        };
        cursor = if pieces > 0 {
            sentences_forward(text, cursor, pieces, true)
        } else {
            decl(text, cursor).unwrap_or(cursor)
        };
        if around {
            // 已包含句子之前的空白时不再包含之后的空白；之后没有空白时改为包含之前的。
            if start_blank {
                cursor = first_blank(text, cursor);
                if blank_at(text, cursor) {
                    cursor = decl(text, cursor).unwrap_or(cursor);
                }
            } else if !blank_at(text, cursor) {
                start = first_blank(text, start);
            }
        }
        if at.anchor.is_none() {
            // 句子之后是换行时把换行算在内；已到文本末尾时终点在范围之内。
            let (end, inclusive) = incl(text, cursor);
            return Some(Selected {
                start,
                end,
                inclusive,
                linewise: false,
                becomes: Becomes::Char,
                failed: false,
            });
        }
        if start != cursor {
            return Some(Selected {
                start,
                end: cursor,
                inclusive: true,
                linewise: false,
                becomes: Becomes::Char,
                failed: false,
            });
        }
    }

    // 可视模式下已有选区：在其基础上延伸。
    let anchor = at.anchor?;
    if start < anchor {
        let mut at_start = true;
        pos = decl(text, pos).unwrap_or(pos);
        while pos < cursor {
            if !blank_at(text, pos) {
                at_start = false;
                break;
            }
            pos = incl(text, pos).0;
        }
        if !at_start {
            cursor = sent(text, cursor, false);
            if cursor == start {
                at_start = true;
            } else {
                cursor = sent(text, cursor, true);
            }
        }
        if around {
            count *= 2;
        }
        for _ in 0..count {
            if at_start {
                cursor = first_blank(text, cursor);
            }
            if !at_start || (!around && !blank_at(text, cursor)) {
                cursor = sent(text, cursor, false);
            }
            at_start = !at_start;
        }
    } else {
        pos = incl(text, pos).0;
        let mut at_start = true;
        if pos != cursor {
            at_start = false;
            while pos < cursor {
                if !blank_at(text, pos) {
                    at_start = true;
                    break;
                }
                pos = incl(text, pos).0;
            }
            cursor = if at_start {
                sent(text, cursor, false)
            } else {
                start
            };
        }
        if around {
            count *= 2;
        }
        cursor = sentences_forward(text, cursor, count, at_start);
    }
    Some(Selected {
        start: anchor,
        end: cursor,
        inclusive: true,
        linewise: false,
        becomes: Becomes::Same,
        failed: false,
    })
}

/// 第 `line` 行是否为空行或只有空白。
fn white(text: &impl Lines, line: usize) -> bool {
    text::indent_chars(text, line) == text.line_len(line)
}

/// `ip`、`ap`。
fn paragraph<T: Lines>(text: &T, at: At, around: bool) -> Option<Selected> {
    let last = text::last_line(text);
    let starts = |line: usize| boundary(text, line, None, false);
    let cursor_line = text::line_of(text, at.cursor);
    let anchor_line = at.anchor.map(|anchor| text::line_of(text, anchor));
    let mut start = cursor_line;
    let mut extend = anchor_line.is_some_and(|line| line != cursor_line);

    if !extend {
        // 先退到段落或一段空白行的第一行。
        let white_in_front = white(text, start);
        while start > 0 {
            if white_in_front {
                if !white(text, start - 1) {
                    break;
                }
            } else if white(text, start - 1) || starts(start) {
                break;
            }
            start -= 1;
        }
        // `next` 是所选范围之后的一行。
        let mut next = start;
        while next <= last && white(text, next) {
            next += 1;
        }
        let mut remaining = at.count - usize::from(!around && white_in_front);
        let mut do_white = false;
        while remaining > 0 {
            remaining -= 1;
            if next > last {
                return None;
            }
            if !around {
                do_white = white(text, next);
            }
            if around || !do_white {
                next += 1;
                while next <= last && !white(text, next) && !starts(next) {
                    next += 1;
                }
            }
            if remaining == 0 && white_in_front && around {
                break;
            }
            if around || do_white {
                while next <= last && white(text, next) {
                    next += 1;
                }
            }
        }
        let end = next.saturating_sub(1);
        // 末尾没有空白行时改为包含段落之前的空白行。
        if !white_in_front && !white(text, end) && around {
            while start > 0 && white(text, start - 1) {
                start -= 1;
            }
        }
        // 按行的可视模式下范围的首行就是选区的起点时，改为在选区的基础上延伸。
        extend = at.linewise && anchor_line == Some(start);
        if !extend {
            return Some(Selected {
                start: text.line_start(start),
                end: text.line_start(end),
                inclusive: false,
                linewise: true,
                becomes: Becomes::Line,
                failed: false,
            });
        }
    }

    // 可视模式下已有多行的选区：向光标所在的一端延伸。
    let anchor_line = anchor_line?;
    let backward = start < anchor_line;
    let edge = if backward { 0 } else { last };
    let step = |line: usize| if backward { line - 1 } else { line + 1 };
    let mut failed = false;
    for _ in 0..at.count {
        if start == edge {
            failed = true;
            break;
        }
        let mut previous = None;
        for _ in 0..2 {
            start = step(start);
            let is_white = white(text, start);
            if previous == Some(is_white) {
                start = if backward { start + 1 } else { start - 1 };
                break;
            }
            while start != edge {
                let beyond = step(start);
                let stop = if is_white {
                    !white(text, beyond)
                } else {
                    starts(if backward { start } else { start + 1 }) || white(text, beyond)
                };
                if stop {
                    break;
                }
                start = beyond;
            }
            if !around || start == edge {
                break;
            }
            previous = Some(is_white);
        }
    }
    Some(Selected {
        start: at.anchor?,
        end: text.line_start(start),
        inclusive: false,
        linewise: true,
        becomes: Becomes::Same,
        failed,
    })
}

/// 光标是否在行首的缩进之内。
fn in_indent(text: &impl Lines, pos: usize) -> bool {
    let line = text::line_of(text, pos);
    pos - text.line_start(line) < text::indent_chars(text, line)
}

/// `i(`、`a(` 等成对括号的文本对象。
fn block<T: Lines>(text: &T, at: At, around: bool, open: char, close: char) -> Option<Selected> {
    let mut old_start = at.cursor;
    let mut old_end = at.cursor;
    let mut cursor = at.cursor;
    let visual = at.anchor.is_some();
    match at.anchor {
        Some(anchor) if anchor < at.cursor => {
            old_start = anchor;
            cursor = anchor;
        }
        Some(anchor) if anchor > at.cursor => old_end = anchor,
        _ => {
            if open == '{' {
                // 忽略缩进。
                while in_indent(text, cursor) {
                    cursor += 1;
                }
            }
            if char_at(text, cursor) == Some(open) {
                cursor += 1;
            }
        }
    }

    // 向前文找未配对的左括号；找不到时改为向后文找第一个左括号。
    let forward = pair::enclosing(text, cursor, open, close, false).is_none();
    let mut start = None;
    for _ in 0..at.count {
        let from = start.unwrap_or(cursor);
        start = Some(pair::enclosing(text, from, open, close, forward)?);
    }
    let mut start = start?;
    let mut end = pair::unmatched(text, start, open, close, true)?;

    let mut sol = false;
    // 不含括号时去掉两端；可视模式下结果不比原来的选区大时，先改选外面一层再去掉，外面一层
    // 一定比原来的选区大。
    if !around {
        loop {
            let inner = incl(text, start).0;
            // 可视模式下括号之间没有内容时不选。
            if visual && inner == end {
                return None;
            }
            let mut scan = Scan::new(text, end);
            sol = scan.col == 0;
            scan.decl();
            while in_indent(text, scan.pos()) {
                sol = true;
                if scan.decl() != Step::Char {
                    break;
                }
            }
            let inner_end = scan.pos();
            if visual && inner >= old_start && old_end >= inner_end && inner != inner_end {
                let outer = decl(text, old_start)?;
                start = pair::unmatched(text, outer, open, close, false)?;
                end = pair::unmatched(text, start, open, close, true)?;
                continue;
            }
            start = inner;
            end = inner_end;
            break;
        }
    }

    if visual {
        if sol && char_at(text, end).is_some() {
            // 把换行符算在内。
            let mut scan = Scan::new(text, end);
            scan.inc();
            end = scan.pos();
        }
        return Some(Selected {
            start,
            end,
            inclusive: true,
            linewise: false,
            becomes: Becomes::Char,
            failed: false,
        });
    }
    let (end, inclusive) = if sol {
        (incl(text, end).0, false)
    } else if start <= end {
        (end, true)
    } else {
        // 括号之间没有文本：范围为空。
        (start, false)
    };
    Some(Selected {
        start,
        end,
        inclusive,
        linewise: false,
        becomes: Becomes::Char,
        failed: false,
    })
}

/// 返回 `chars` 中从下标 `from` 起第一个引号 `quote` 的下标；`escaped` 为真时跳过反斜杠转义的
/// 字符。
fn next_quote(chars: &[char], from: usize, quote: char, escaped: bool) -> Option<usize> {
    let mut col = from;
    loop {
        let c = *chars.get(col)?;
        if escaped && c == '\\' {
            col += 1;
            chars.get(col)?;
        } else if c == quote {
            return Some(col);
        }
        col += 1;
    }
}

/// 返回 `chars` 中下标 `from` 之前最近一个引号 `quote` 的下标，找不到时返回 0；`escaped`
/// 为真时跳过反斜杠转义的引号。
fn prev_quote(chars: &[char], from: usize, quote: char, escaped: bool) -> usize {
    let mut col = from;
    while col > 0 {
        col -= 1;
        let mut backslashes = 0;
        if escaped {
            while col > backslashes && chars[col - backslashes - 1] == '\\' {
                backslashes += 1;
            }
        }
        if backslashes % 2 == 1 {
            col -= backslashes;
        } else if chars[col] == quote {
            break;
        }
    }
    col
}

/// 返回一对引号所在的列；`cursor` 是光标所在的列。
///
/// 已有非空的选区时给出 `extending`：为真表示选区的起点在光标之前。
fn quote_span(
    chars: &[char],
    quote: char,
    cursor: usize,
    extending: Option<bool>,
) -> Option<(usize, usize)> {
    let get = |col: usize| chars.get(col).copied();
    let on_quote = get(cursor) == Some(quote);
    match extending {
        // 已有选区且光标在引号上：找下一段引号。
        Some(true) if on_quote => {
            let start = next_quote(chars, cursor + 1, quote, false)?;
            Some(match next_quote(chars, start + 1, quote, true) {
                Some(end) => (start, end),
                None => (cursor, start),
            })
        }
        Some(false) if on_quote => {
            let end = prev_quote(chars, cursor, quote, false);
            if get(end) != Some(quote) {
                return None;
            }
            let start = prev_quote(chars, end, quote, true);
            Some(if get(start) == Some(quote) {
                (start, end)
            } else {
                (end, cursor)
            })
        }
        _ if on_quote || extending.is_some() => {
            let first = match extending {
                Some(true) => next_quote(chars, cursor, quote, false)?,
                Some(false) => prev_quote(chars, cursor, quote, false),
                None => cursor,
            };
            // 光标在引号上：从行首起依次配对，确定该引号是开头还是结尾。
            let mut from = 0;
            loop {
                let open = next_quote(chars, from, quote, false)?;
                if open > first {
                    return None;
                }
                let end = next_quote(chars, open + 1, quote, true)?;
                if first <= end {
                    return Some((open, end));
                }
                from = end + 1;
            }
        }
        _ => {
            // 向前文找开头的引号，找不到时向后文找。
            let mut start = prev_quote(chars, cursor, quote, true);
            if get(start) != Some(quote) {
                start = next_quote(chars, start, quote, false)?;
            }
            Some((start, next_quote(chars, start + 1, quote, true)?))
        }
    }
}

/// `i"`、`a"` 等引号的文本对象；只在一行之内。
fn quote<T: Lines>(text: &T, at: At, around: bool, quote: char) -> Option<Selected> {
    let line = text::line_of(text, at.cursor);
    let base = text.line_start(line);
    let chars: Vec<char> = text.line_content(line).slice().chars().collect();
    let get = |col: usize| chars.get(col).copied();
    let cursor_col = at.cursor - base;

    let anchor_col = match at.anchor {
        Some(anchor) if text::line_of(text, anchor) != line => return None,
        Some(anchor) => Some(anchor - base),
        None => None,
    };
    let vis_empty = anchor_col.is_none_or(|anchor| anchor == cursor_col);
    let vis_before = anchor_col.is_some_and(|anchor| anchor < cursor_col);
    let visual_col = anchor_col.unwrap_or(cursor_col);
    let mut inside_quotes = false;
    let mut selected_quote = false;

    if !vis_empty {
        // 已有的选区是否正好是引号之内的文本，以及选区中是否有引号。
        let (low, high) = if vis_before {
            (visual_col, cursor_col)
        } else {
            (cursor_col, visual_col)
        };
        inside_quotes = low > 0
            && get(low - 1) == Some(quote)
            && get(high).is_some()
            && get(high + 1) == Some(quote);
        selected_quote = (low..=high).any(|col| get(col) == Some(quote));
    }

    let extending = (!vis_empty).then_some(vis_before);
    let (mut col_start, mut col_end) = quote_span(&chars, quote, cursor_col, extending)?;

    // 包含引号时，连同结尾引号之后的空白；之后没有空白时连同开头引号之前的空白。
    if around {
        if get(col_end + 1).is_some_and(text::is_blank) {
            while get(col_end + 1).is_some_and(text::is_blank) {
                col_end += 1;
            }
        } else {
            while col_start > 0 && get(col_start - 1).is_some_and(text::is_blank) {
                col_start -= 1;
            }
        }
    }

    let widen = around || at.count > 1 || (!vis_empty && inside_quotes);
    if !around && at.count < 2 && (vis_empty || !inside_quotes) {
        col_start += 1;
    }
    // 终点：包含引号时为引号之后一个字符。
    let mut end_col = col_end;
    let mut inclusive = false;
    if widen {
        end_col += 1;
        inclusive = end_col >= chars.len();
    }

    let Some(_) = at.anchor else {
        return Some(Selected {
            start: base + col_start,
            end: base + end_col.min(chars.len()),
            inclusive,
            linewise: false,
            becomes: Becomes::Char,
            failed: false,
        });
    };
    let mut anchor = visual_col;
    if vis_empty
        || (vis_before
            && !selected_quote
            && (inside_quotes
                || (get(visual_col) != Some(quote)
                    && (visual_col == 0 || get(visual_col - 1) != Some(quote)))))
    {
        anchor = col_start;
    }
    let cursor = if vis_empty || vis_before {
        end_col.saturating_sub(1)
    } else {
        // 光标在选区的起点一端：终点成为选区的起点。
        if inside_quotes
            || (!selected_quote
                && get(visual_col) != Some(quote)
                && (get(visual_col).is_none() || get(visual_col + 1) != Some(quote)))
        {
            anchor = end_col.saturating_sub(1);
        }
        col_start
    };
    Some(Selected {
        start: base + anchor,
        end: base + cursor,
        inclusive: true,
        linewise: false,
        becomes: Becomes::CharUnlessBlock,
        failed: false,
    })
}

/// 是否为词字符，用于判断标签名的结尾。
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 判断 `chars` 从下标 `at`（一个 `<`）起是否为标签；`name` 给出时只接受该名字的标签
/// （不区分大小写）。`chars` 是从某一行起的若干行，换行记作 `\n`。
fn tag_at(chars: &[char], at: usize, name: Option<&[char]>) -> Option<bool> {
    let get = |i: usize| chars.get(i).copied();
    let space = |c: Option<char>| matches!(c, Some(' ' | '\t' | '\n'));
    if get(at + 1) == Some('/') {
        // 结束标签：`</` 之后到同一行的 `>`。
        let mut end = at + 2;
        loop {
            match get(end) {
                Some('>') => break,
                Some('\n') | None => return None,
                _ => end += 1,
            }
        }
        if let Some(name) = name {
            let found = &chars[at + 2..end];
            let same = found.len() == name.len()
                && found
                    .iter()
                    .zip(name)
                    .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()));
            if !same {
                return None;
            }
        }
        return Some(true);
    }
    // 开始标签的名字：不含空白、`>`、`/`、`!` 的一串字符。
    let mut end = at + 1;
    match name {
        Some(name) => {
            for expected in name {
                let same = get(end).is_some_and(|c| c.to_lowercase().eq(expected.to_lowercase()));
                if !same {
                    return None;
                }
                end += 1;
            }
            let boundary = get(end - 1).is_some_and(is_word) && !get(end).is_some_and(is_word);
            if !boundary {
                return None;
            }
        }
        None => {
            while get(end).is_some_and(|c| !matches!(c, ' ' | '\t' | '>' | '/' | '!' | '\n')) {
                end += 1;
            }
            if end == at + 1 {
                return None;
            }
            // 名字直到行尾的也算开始标签。
            if matches!(get(end), Some('\n') | None) {
                return Some(false);
            }
        }
    }
    // 名字之后：可选的一个空白再接 `>`；或空白、属性，再接前面不是 `/` 的 `>`。
    if get(end) == Some('>') || (space(get(end)) && get(end + 1) == Some('>')) {
        return Some(false);
    }
    if !space(get(end)) {
        return None;
    }
    let mut close = end + 1;
    loop {
        match get(close) {
            Some('>') => break,
            None => return None,
            _ => close += 1,
        }
    }
    let before = get(close - 1);
    let ok = close - 1 > end && before != Some('/') && (name.is_some() || before != Some('\n'));
    ok.then_some(false)
}

/// 标签的文本对象最多检查的字符数（字符）。
///
/// 取自调参：与括号配对的上限同一量级，限制没有配对的标签在大文件中的扫描耗时。
const MAX_TAG_SCAN: usize = 1_000_000;

/// 一个标签最多跨越的行数（行）。
///
/// 取自调参：属性分行书写的标签通常不超过数行。
const TAG_LINES: usize = 8;

/// 从 `pos` 向前文找未配对的开始标签。
fn tag_backward<T: Lines>(text: &T, pos: usize) -> Option<usize> {
    let mut nest = 1usize;
    let mut line = text::line_of(text, pos);
    let mut limit = pos - text.line_start(line);
    let mut budget = MAX_TAG_SCAN;
    loop {
        // 标签可以跨行：从该行起取到之后的若干行。
        let chars = lines_from(text, line, TAG_LINES);
        budget = budget.checked_sub(chars.len())?;
        let mut col = limit.min(text.line_len(line));
        while col > 0 {
            col -= 1;
            budget = budget.checked_sub(1)?;
            if chars.get(col) != Some(&'<') {
                continue;
            }
            match tag_at(&chars, col, None) {
                Some(true) => nest += 1,
                Some(false) => {
                    nest -= 1;
                    if nest == 0 {
                        return Some(text.line_start(line) + col);
                    }
                }
                None => {}
            }
        }
        line = line.checked_sub(1)?;
        limit = usize::MAX;
    }
}

/// 返回从第 `line` 行起至多 `count` 行的字符，行间以 `\n` 分隔。
fn lines_from<T: Lines>(text: &T, line: usize, count: usize) -> Vec<char> {
    let last = text::last_line(text).min(line + count - 1);
    let mut chars = Vec::new();
    for at in line..=last {
        chars.extend(text.line_content(at).slice().chars());
        chars.push('\n');
    }
    chars
}

/// 从 `pos` 之后向后文找与名字 `name` 配对的结束标签，返回其 `<` 的位置。
fn tag_forward<T: Lines>(text: &T, pos: usize, name: &[char]) -> Option<usize> {
    let mut nest = 1usize;
    let last = text::last_line(text);
    let mut line = text::line_of(text, pos);
    let mut from = pos - text.line_start(line) + 2;
    let mut budget = MAX_TAG_SCAN;
    loop {
        let chars = lines_from(text, line, TAG_LINES);
        budget = budget.checked_sub(chars.len())?;
        let len = text.line_len(line);
        for col in from..len {
            budget = budget.checked_sub(1)?;
            if chars.get(col) != Some(&'<') {
                continue;
            }
            match tag_at(&chars, col, Some(name)) {
                Some(false) => nest += 1,
                Some(true) => {
                    nest -= 1;
                    if nest == 0 {
                        return Some(text.line_start(line) + col);
                    }
                }
                None => {}
            }
        }
        if line >= last {
            return None;
        }
        line += 1;
        from = 0;
    }
}

/// 光标在一个标签之内时返回其 `<` 的位置：`closing` 为真时只认结束标签，否则只认不以 `/>`
/// 结尾的开始标签。
fn in_tag<T: Lines>(text: &T, pos: usize, closing: bool) -> Option<usize> {
    let line = text::line_of(text, pos);
    let base = text.line_start(line);
    let chars: Vec<char> = text.line_content(line).slice().chars().collect();
    let mut col = pos - base;
    loop {
        if chars.get(col) == Some(&'<') {
            break;
        }
        if col == 0 {
            return None;
        }
        col -= 1;
        if chars.get(col) == Some(&'>') {
            return None;
        }
    }
    let slash = chars.get(col + 1) == Some(&'/');
    if closing {
        return slash.then_some(base + col);
    }
    if slash {
        return None;
    }
    // 之后的 `>` 之前不是 `/`。
    let mut last = None;
    let mut scan = Scan::new(text, base + col);
    loop {
        if scan.inc() == Step::Edge {
            return None;
        }
        match scan.char() {
            Some('>') => break,
            c => last = c,
        }
    }
    (last != Some('/')).then_some(base + col)
}

/// 从 `pos` 起向后文找到字符 `target` 的位置。
fn seek<T: Lines>(text: &T, pos: usize, target: char) -> usize {
    let mut scan = Scan::new(text, pos);
    while scan.char() != Some(target) {
        if scan.inc() == Step::Edge {
            break;
        }
    }
    scan.pos()
}

/// `it`、`at`。
fn tag<T: Lines>(text: &T, at: At, around: bool) -> Option<Selected> {
    let visual = at.anchor.is_some();
    let mut old_start = at.cursor;
    let mut old_end = at.cursor;
    let mut cursor = at.cursor;
    if !visual {
        old_end = decl(text, old_end).unwrap_or(old_end);
    }
    match at.anchor {
        Some(anchor) if anchor < at.cursor => {
            old_start = anchor;
            cursor = anchor;
        }
        Some(anchor) if anchor > at.cursor => old_end = anchor,
        _ => {
            while in_indent(text, cursor) {
                cursor += 1;
            }
            if in_tag(text, cursor, false).is_some() {
                // 光标在开始标签上：移到其 `>`。
                cursor = seek(text, cursor, '>');
            } else if let Some(found) = in_tag(text, cursor, true) {
                // 光标在结束标签上：移到其之前。
                cursor = decl(text, found).unwrap_or(found);
                old_end = cursor;
            }
        }
    }

    let mut include = around;
    let mut count = at.count;
    let mut attempts = MAX_TAG_SCAN;
    let (start, end, inclusive) = loop {
        attempts = attempts.checked_sub(1)?;
        let mut open = cursor;
        for _ in 0..count {
            open = tag_backward(text, open)?;
        }
        // 取出标签名。
        let line = text::line_of(text, open);
        let content: Vec<char> = text.line_content(line).slice().chars().collect();
        let from = open - text.line_start(line) + 1;
        let name: Vec<char> = content
            .iter()
            .skip(from)
            .take_while(|c| !matches!(c, '>' | ' ' | '\t'))
            .copied()
            .collect();
        if name.is_empty() {
            return None;
        }
        let close = match tag_forward(text, open, &name) {
            Some(close) if close >= old_end => close,
            // 找不到配对的结束标签，或结束标签在原来的终点之前：改找更外面的开始标签。
            _ => {
                count = 1;
                cursor = open;
                continue;
            }
        };
        if include {
            break (open, seek(text, close, '>'), true);
        }
        // 去掉结束标签的 `<`；结束标签在行首时范围止于该行行首，换行符算在内。
        let mut inclusive = true;
        let end = if !visual && close == text.line_start(text::line_of(text, close)) {
            inclusive = false;
            close
        } else {
            let mut scan = Scan::new(text, close);
            scan.dec();
            scan.pos()
        };
        // 去掉开始标签。
        let after = seek(text, open, '>');
        let mut inner = Scan::new(text, after);
        inner.inc();
        let start = inner.pos();
        if visual && start == old_start && end == old_end {
            // 与原来的选区相同：改为包含标签。
            include = true;
            cursor = old_start;
            count = at.count;
            continue;
        }
        break (start, end, inclusive);
    };

    if visual {
        let end = if end < start { start } else { end };
        return Some(Selected {
            start,
            end,
            inclusive: true,
            linewise: false,
            becomes: Becomes::Char,
            failed: false,
        });
    }
    if end < start {
        // 标签之间没有文本：范围为空。
        return Some(Selected {
            start,
            end: start,
            inclusive: false,
            linewise: false,
            becomes: Becomes::Char,
            failed: false,
        });
    }
    Some(Selected {
        start,
        end,
        inclusive,
        linewise: false,
        becomes: Becomes::Char,
        failed: false,
    })
}
