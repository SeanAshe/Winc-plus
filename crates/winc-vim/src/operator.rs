//! 操作符的范围与文本变换：由动作的结果或可视选区算出作用的区域，并生成各操作符的修改。
//!
//! 动作止于行首时范围的调整、跨行删除改为按行等特例集中在 [`motion_region`] 中。生成的修改
//! 按位置从后往前排列，依次应用时各处的位置互不影响。

use winc_core::{Change, Lines};

use crate::command::{Force, Operator};
use crate::host::{Host, Indentation};
use crate::motion::{Moved, Span};
use crate::text;

/// 操作符作用的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    /// 按字符：位置区间 `from..to`，可以为空。
    Chars {
        /// 起点。
        from: usize,
        /// 终点（不含）。
        to: usize,
    },
    /// 按行：第 `first` 至 `last` 行。
    Lines {
        /// 首行。
        first: usize,
        /// 末行（含）。
        last: usize,
    },
}

/// 操作符的范围：区域，以及只按行工作的操作符所用的首末行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
    /// 作用的区域。
    pub region: Region,
    /// 区域覆盖的首行。
    pub first: usize,
    /// 区域覆盖的末行。
    pub last: usize,
    /// 范围的起点：操作之前光标移到的位置。
    pub start: usize,
    /// 删除时是否不论长短都进入编号寄存器。
    pub numbered: bool,
    /// 范围是否为空：不含终点的动作没有移动。为空时删除与修改不写寄存器。
    pub empty: bool,
}

/// 把首末行扩展到所在的已折叠区域的边界。
fn with_folds<H: Host>(host: &H, first: usize, last: usize) -> (usize, usize) {
    if !host.has_folds() {
        return (first, last);
    }
    let first = host.fold(first).map_or(first, |(start, _)| start);
    let last = host.fold(last).map_or(last, |(_, end)| end);
    (first, last)
}

/// 返回从光标 `cursor` 到动作结果 `moved` 的范围。
pub(crate) fn motion_region<H: Host>(
    host: &H,
    cursor: usize,
    moved: Moved,
    force: Option<Force>,
    operator: Operator,
) -> Range {
    let text = host.text();
    let (start, end) = (cursor.min(moved.pos), cursor.max(moved.pos));
    let span = match (force, moved.span) {
        (Some(Force::Linewise), _) => Span::Linewise,
        (Some(Force::Charwise), Span::Linewise | Span::Inclusive) => Span::Exclusive,
        (Some(Force::Charwise), Span::Exclusive) => Span::Inclusive,
        (_, span) => span,
    };
    let (first, last) = (text::line_of(text, start), text::line_of(text, end));
    let in_indent = start - text.line_start(first) <= text::indent_chars(text, first);
    let lines = |first, last| {
        let (first, last) = with_folds(host, first, last);
        Range {
            region: Region::Lines { first, last },
            first,
            last,
            start,
            numbered: moved.numbered,
            empty: false,
        }
    };
    let chars = |to: usize, last| Range {
        region: Region::Chars { from: start, to },
        first,
        last,
        start,
        numbered: moved.numbered,
        empty: span == Span::Exclusive && start == end,
    };

    match span {
        Span::Linewise => lines(first, last),
        // 不含终点的动作止于另一行的行首时，范围改为到上一行的行尾；起点在缩进之内时改为按行。
        Span::Exclusive if !moved.keep_end && last > first && end == text.line_start(last) => {
            if in_indent {
                lines(first, last - 1)
            } else {
                chars(text.line_end(last - 1), last - 1)
            }
        }
        _ => {
            let to = match span {
                Span::Inclusive => text::next_in_line(text, end).unwrap_or(end),
                _ => end,
            };
            // 跨行的删除：终点之后只有空白、起点在缩进之内时改为按行。
            let rest_blank = || {
                let content = text.line_content(last);
                let col = to - text.line_start(last);
                content.slice().chars().skip(col).all(text::is_blank)
            };
            if operator == Operator::Delete
                && force.is_none()
                && last > first
                && in_indent
                && rest_blank()
            {
                lines(first, last)
            } else {
                chars(to, last)
            }
        }
    }
}

/// 返回可视选区的范围：`anchor` 与 `cursor` 是选区两端字符的位置，`linewise` 表示按行。
///
/// `whole_lines` 为真时（只按行工作的操作符），按字符的选区不把行尾之后的换行符算在内。
pub(crate) fn visual_region<H: Host>(
    host: &H,
    anchor: usize,
    cursor: usize,
    linewise: bool,
    whole_lines: bool,
) -> Range {
    let text = host.text();
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    let (first, last) = (text::line_of(text, start), text::line_of(text, end));
    if linewise {
        // 起点一端按所在行的行首计，范围从两端中靠前的一个算起。
        let start = text.line_start(text::line_of(text, anchor)).min(cursor);
        let (first, last) = with_folds(host, first, last);
        return Range {
            region: Region::Lines { first, last },
            first,
            last,
            start,
            numbered: false,
            empty: false,
        };
    }
    let to = match text::next_in_line(text, end) {
        Some(next) => next,
        // 光标在行尾：范围包含换行符，最后一行除外。
        None if !whole_lines && last < text::last_line(text) => text.line_start(last + 1),
        None => end,
    };
    Range {
        region: Region::Chars { from: start, to },
        first,
        last,
        start,
        numbered: false,
        empty: false,
    }
}

/// 把文本中的换行统一成 `\n`。
pub(crate) fn normalized(text: String) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}

/// 返回第 `first` 至 `last` 行的内容，行间以 `\n` 连接，不含末行之后的换行符。
pub(crate) fn lines_text(text: &impl Lines, first: usize, last: usize) -> String {
    normalized(text.substring(text.line_start(first), text.line_end(last)))
}

/// 返回删除第 `first` 至 `last` 行时移除的位置区间。
///
/// 被删除的行之后还有行时连同各行之后的换行符一起移除，否则连同之前的换行符一起移除。
pub(crate) fn lines_span(text: &impl Lines, first: usize, last: usize) -> (usize, usize) {
    if last < text::last_line(text) {
        (text.line_start(first), text.line_start(last + 1))
    } else if first > 0 {
        (text.line_end(first - 1), text.line_end(last))
    } else {
        (0, text.line_end(last))
    }
}

/// 返回删除位置区间 `from..to` 的修改。
pub(crate) fn removal(text: &impl Lines, from: usize, to: usize) -> Change {
    Change {
        start: from,
        removed: text.substring(from, to),
        inserted: String::new(),
    }
}

/// 按大小写操作符 `operator` 变换 `text`；每个字符变换前后都是一个字符。
pub(crate) fn recased(text: &str, operator: Operator) -> String {
    let single =
        |mut chars: std::char::ToUppercase, original: char| match (chars.next(), chars.next()) {
            (Some(c), None) => c,
            _ => original,
        };
    let upper = |c: char| single(c.to_uppercase(), c);
    let lower = |c: char| {
        let mut chars = c.to_lowercase();
        match (chars.next(), chars.next()) {
            (Some(l), None) => l,
            _ => c,
        }
    };
    text.chars()
        .map(|c| match operator {
            Operator::Uppercase => upper(c),
            Operator::Lowercase => lower(c),
            Operator::Rot13 => match c {
                'a'..='z' => char::from((c as u8 - b'a' + 13) % 26 + b'a'),
                'A'..='Z' => char::from((c as u8 - b'A' + 13) % 26 + b'A'),
                _ => c,
            },
            _ if c.is_uppercase() => lower(c),
            _ if c.is_lowercase() => upper(c),
            _ => c,
        })
        .collect()
}

/// 返回宽度为 `cells` 个显示列的缩进文本。
pub(crate) fn indent_text(cells: usize, indentation: Indentation, tab_width: usize) -> String {
    if indentation.tabs {
        let tab_width = tab_width.max(1);
        let mut out = "\t".repeat(cells / tab_width);
        out.push_str(&" ".repeat(cells % tab_width));
        out
    } else {
        " ".repeat(cells)
    }
}

/// 返回把第 `line` 行的缩进改为 `cells` 个显示列的修改；缩进不变时返回 `None`。
pub(crate) fn reindent<H: Host>(host: &H, line: usize, cells: usize) -> Option<Change> {
    let text = host.text();
    let start = text.line_start(line);
    let old = text.substring(start, start + text::indent_chars(text, line));
    let new = indent_text(cells, host.indentation(), host.tab_width());
    (old != new).then_some(Change {
        start,
        removed: old,
        inserted: new,
    })
}

/// 返回第 `line` 行缩进的显示宽度。
pub(crate) fn indent_cells<H: Host>(host: &H, line: usize) -> usize {
    let text = host.text();
    text::cell_of(text, line, text::indent_chars(text, line), host.tab_width())
}

/// 返回把第 `first` 至 `last` 行增加（`left` 为假）或减少 `amount` 级缩进的修改；空行不变。
pub(crate) fn shift<H: Host>(
    host: &H,
    first: usize,
    last: usize,
    amount: usize,
    left: bool,
) -> Vec<Change> {
    let text = host.text();
    let by = host.indentation().width.max(1).saturating_mul(amount);
    (first..=last)
        .rev()
        .filter(|line| text.line_len(*line) > 0)
        .filter_map(|line| {
            let cells = indent_cells(host, line);
            let cells = if left {
                cells.saturating_sub(by)
            } else {
                cells.saturating_add(by)
            };
            reindent(host, line, cells)
        })
        .collect()
}

/// 合并从第 `first` 行起的 `count` 行，返回修改与合并之后光标在首行内的字符偏移。
///
/// `spaces` 为真时去掉后续各行的行首空白，行间留一个空格（上一行以空白结尾、下一行以 `)`
/// 开头或为空时不留），句末标点之后留两个；为假时原样连接。行数不足两行时返回 `None`。
pub(crate) fn join(
    text: &impl Lines,
    first: usize,
    count: usize,
    spaces: bool,
) -> Option<(Vec<Change>, usize)> {
    let last = (first + count.max(2) - 1).min(text::last_line(text));
    if last <= first {
        return None;
    }
    let line = |line: usize| normalized(text.substring(text.line_start(line), text.line_end(line)));
    let mut joined = line(first);
    let mut cursor = 0;
    // 上一行的最后两个字符，用于决定行间的空格。
    let tail = |piece: &str| {
        let mut chars = piece.chars().rev();
        (chars.next(), chars.next())
    };
    let (mut end1, mut end2) = if spaces { tail(&joined) } else { (None, None) };
    for next in first + 1..=last {
        let content = line(next);
        let piece = if spaces {
            content.trim_start_matches([' ', '\t'])
        } else {
            &content
        };
        let mut gap = 0;
        if spaces
            && !piece.is_empty()
            && !piece.starts_with(')')
            && !joined.is_empty()
            && end1 != Some('\t')
        {
            if end1 == Some(' ') {
                end1 = end2;
            } else {
                gap += 1;
            }
            if matches!(end1, Some('.' | '?' | '!')) {
                gap += 1;
            }
        }
        cursor = joined.chars().count();
        joined.push_str(&" ".repeat(gap));
        joined.push_str(piece);
        (end1, end2) = if spaces && !piece.is_empty() {
            tail(piece)
        } else {
            (None, None)
        };
    }
    let start = text.line_start(first);
    let change = Change {
        start,
        removed: text.substring(start, text.line_end(last)),
        inserted: joined,
    };
    Some((vec![change], cursor))
}
