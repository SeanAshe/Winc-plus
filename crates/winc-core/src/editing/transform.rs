//! 整理文字的命令：改大小写、排序行、合并行、删行尾空白、转换缩进和行尾，以及跳到配对的括号、
//! 扩大选区。
//!
//! 每个命令只生成 [`Transaction`]，不直接修改文本。

use crate::editing::edit::{Change, EditKind, Transaction, selected_lines, shift_selection};
use crate::editing::movement;
use crate::editing::selection::Selection;
use crate::text::Lines;

/// 大小写转换的目标形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// 全部大写。
    Upper,
    /// 全部小写。
    Lower,
    /// 每个词的首字母大写，其余小写。
    Title,
}

impl Case {
    /// 返回 `s` 转换为该大小写形式后的字符串。
    pub fn apply(self, s: &str) -> String {
        match self {
            Case::Upper => s.to_uppercase(),
            Case::Lower => s.to_lowercase(),
            Case::Title => title_case(s),
        }
    }
}

/// 把 `s` 转为每个词首字母大写、其余小写的形式。
///
/// 字母的前一个字符不是字母、数字、撇号时，该字母是词首。撇号之前不是字母时，撇号也算分隔符，
/// 因此 `'hello'` 中的 `h` 大写，而 `don't` 不会变成 `Don'T`。
fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let (mut prev, mut before) = (None::<char>, None::<char>);
    for c in s.chars() {
        let starts = match prev {
            None => true,
            Some('\'') => !before.is_some_and(char::is_alphabetic),
            Some(p) => !p.is_alphanumeric(),
        };
        if c.is_alphabetic() && starts {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        before = prev;
        prev = Some(c);
    }
    out
}

/// 返回改变大小写的事务；没有内容需要改变时返回 `None`。
///
/// 有选区时转换选区；没有选区时转换光标所在的词，光标位置不变，转换成更短的文本时夹取到词尾。
pub fn change_case(text: &impl Lines, selection: Selection, case: Case) -> Option<Transaction> {
    let (from, to) = if selection.is_empty() {
        let (start, end) = movement::word_at(text, selection.head);
        let word = text.substring(start, end);
        if !word.chars().any(char::is_alphanumeric) {
            return None;
        }
        (start, end)
    } else {
        (selection.from(), selection.to())
    };

    let old = text.substring(from, to);
    let new = case.apply(&old);
    if new == old {
        return None;
    }

    // 转换后长度可能变化（如 `ß` → `SS`），选区随新文本调整。
    let len = new.chars().count();
    let after = if selection.is_empty() {
        Selection::point(selection.head.min(from + len))
    } else if selection.anchor <= selection.head {
        Selection::new(from, from + len)
    } else {
        Selection::new(from + len, from)
    };
    Some(Transaction::new(
        vec![Change {
            start: from,
            removed: old,
            inserted: new,
        }],
        selection,
        after,
        EditKind::Other,
    ))
}

/// 返回排序涉及的行范围（起止行号，含两端）。
///
/// 选区覆盖多行时取这些行；选区在一行之内时取整个文件。
fn lines_to_sort(text: &impl Lines, selection: Selection) -> (usize, usize) {
    if sorts_whole_file(text, selection) {
        (0, text.line_count() - 1)
    } else {
        selected_lines(text, selection)
    }
}

/// 是否排序会作用于整个文件，即选区是否在一行之内。
///
/// 调用方据此在文件过大时先拦截，不执行排序。
pub fn sorts_whole_file(text: &impl Lines, selection: Selection) -> bool {
    let (first, last) = selected_lines(text, selection);
    first == last
}

/// 返回按 `descending` 排序选中行的事务；顺序已符合要求时返回 `None`。
///
/// 选区在一行之内时排整个文件。行与行之间的换行符留在原处，只调换行的内容。比较时先忽略大小写，
/// 相同时小写在前。
pub fn sort_lines(
    text: &impl Lines,
    selection: Selection,
    descending: bool,
) -> Option<Transaction> {
    let (first, mut last) = lines_to_sort(text, selection);
    // 文件以换行符结尾时，末尾的空行不参加排序，否则该空行会排到最前面。
    if last > first && last + 1 == text.line_count() && text.line_len(last) == 0 {
        last -= 1;
    }
    if first >= last {
        return None;
    }

    let lines: Vec<String> = (first..=last)
        .map(|l| text.line_content(l).slice().to_string())
        .collect();
    // 每行的小写形式只算一次。若每次比较都重新计算，n 行约需 2·n·log₂n 次字符串分配
    // （估算，10 万行约 330 万次）。
    let mut sorted: Vec<(String, &str)> = lines
        .iter()
        .map(|line| (line.to_lowercase(), line.as_str()))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(a.1)));
    if descending {
        sorted.reverse();
    }
    if sorted
        .iter()
        .map(|&(_, line)| line)
        .eq(lines.iter().map(String::as_str))
    {
        return None;
    }

    let (start, end) = (text.line_start(first), text.line_end(last));
    let mut inserted = String::new();
    // 排序后各行的（开头位置，字符数），用于换算选区的新位置。
    let mut placed = Vec::with_capacity(sorted.len());
    let mut at = start;
    for (i, &(_, line)) in sorted.iter().enumerate() {
        let len = line.chars().count();
        placed.push((at, len));
        inserted.push_str(line);
        at += len;
        if i + first < last {
            let l = first + i;
            let separator = text.substring(text.line_end(l), text.line_start(l + 1));
            at += separator.chars().count();
            inserted.push_str(&separator);
        }
    }
    // 各行长短变了，但总长度不变：选区两端留在原来的行号和列上，列不超过该行新的长度。
    // 直接沿用原来的位置可能落进 CRLF 的两个字符之间。
    let keep = |pos: usize| {
        let (line, col) = text.pos_to_line_col(pos);
        match line.checked_sub(first).and_then(|i| placed.get(i)) {
            Some(&(line_start, len)) => line_start + col.min(len),
            None => pos,
        }
    };
    let after = Selection::new(keep(selection.anchor), keep(selection.head));
    Some(Transaction::new(
        vec![Change {
            start,
            removed: text.substring(start, end),
            inserted,
        }],
        selection,
        after,
        EditKind::Other,
    ))
}

/// 返回合并行（`⌃J`）的事务；没有下一行可并入时返回 `None`。
///
/// 没有选区或选区在一行之内时并入下一行，否则把选中的多行合成一行。并入的行去掉开头的空白，与前文
/// 之间隔一个空格；前文末尾已有空白时替换为一个空格，只含空白的行丢弃。光标停在两段文本相接处。
pub fn join_lines(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    let (first, last) = selected_lines(text, selection);
    let last = if first == last { first + 1 } else { last };
    if last >= text.line_count() {
        return None;
    }

    // 第一行保留缩进，只去掉行尾空白；并入的行去掉开头的空白。
    let mut joined = text.line_content(first).slice().to_string();
    joined.truncate(joined.trim_end().len());
    // `length` 是 `joined` 的字符数，`seam` 是两段文本相接的位置，二者随拼接增量维护。每并入一行都
    // 重新统计字符数会使合并 n 行的开销为 O(n²)。没有并入内容时接缝就是第一行的末尾。
    let mut length = joined.chars().count();
    let mut seam = length;
    // 第一行去掉行尾空白后的长度：选区起点落在被去掉的空白里时退到这里。
    let kept = length;
    for line in first + 1..=last {
        let content = text.line_content(line).slice().to_string();
        let rest = content.trim_start();
        if rest.is_empty() {
            continue;
        }
        // 上一段末尾的空白替换为一个空格；`joined` 为空时不加空格。
        let cut = joined.trim_end().len();
        length -= joined[cut..].chars().count();
        joined.truncate(cut);
        seam = length;
        if !joined.is_empty() {
            joined.push(' ');
            length += 1;
        }
        joined.push_str(rest);
        length += rest.chars().count();
    }

    let (start, end) = (text.line_start(first), text.line_end(last));
    let after = if selection.is_empty() {
        Selection::point(start + seam)
    } else {
        Selection::new(selection.from().min(start + kept), start + length)
    };
    Some(Transaction::new(
        vec![Change {
            start,
            removed: text.substring(start, end),
            inserted: joined,
        }],
        selection,
        after,
        EditKind::Other,
    ))
}

/// 返回删除每一行末尾空格和制表符的事务；没有可删内容时返回 `None`。
///
/// 全角空格视为内容，不删除。选区随文本调整。
pub fn trim_trailing_whitespace(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    let mut changes = Vec::new();
    let mut removed_total = 0;
    for line in 0..text.line_count() {
        let content = text.line_content(line);
        let slice = content.slice();
        let len = slice.len_chars();
        let blank = slice
            .chars_at(len)
            .reversed()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .count();
        if blank == 0 {
            continue;
        }
        changes.push(Change {
            start: text.line_start(line) + len - blank - removed_total,
            removed: slice.slice(len - blank..).to_string(),
            inserted: String::new(),
        });
        removed_total += blank;
    }
    if changes.is_empty() {
        return None;
    }
    let after = shift_selection(selection, &changes);
    Some(Transaction::new(changes, selection, after, EditKind::Other))
}

/// 返回转换每行缩进的事务；缩进已符合要求时返回 `None`。
///
/// 默认把缩进全部换成空格；`to_tabs` 为真时尽量换成制表符，凑不满一个制表符的空格保留。制表符按
/// `tab_width` 个显示列折算，`tab_width` 小于 1 时按 1 处理。
pub fn convert_indentation(
    text: &impl Lines,
    selection: Selection,
    to_tabs: bool,
    tab_width: usize,
) -> Option<Transaction> {
    let tab_width = tab_width.max(1);
    let mut changes = Vec::new();
    let mut shift: isize = 0;
    for line in 0..text.line_count() {
        let content = text.line_content(line);
        let slice = content.slice();
        let lead = crate::text::leading_whitespace(slice);
        if lead == 0 {
            continue;
        }
        // 缩进已是目标写法时跳过，不必先拼出新旧两份缩进再比较。缩进只含空格和制表符：转成空格
        // 时没有制表符即可；转成制表符时，应是若干制表符之后跟凑不满一个制表符的空格。
        let done = if to_tabs {
            let tabs = slice.chars().take_while(|&c| c == '\t').count();
            lead - tabs < tab_width && slice.chars().skip(tabs).take(lead - tabs).all(|c| c == ' ')
        } else {
            slice.chars().take(lead).all(|c| c == ' ')
        };
        if done {
            continue;
        }
        let cells = crate::text::indent_cells(slice, tab_width);
        let new = if to_tabs {
            "\t".repeat(cells / tab_width) + &" ".repeat(cells % tab_width)
        } else {
            " ".repeat(cells)
        };
        let delta = new.chars().count() as isize - lead as isize;
        changes.push(Change {
            start: text.line_start(line).saturating_add_signed(shift),
            removed: slice.slice(..lead).to_string(),
            inserted: new,
        });
        shift += delta;
    }
    if changes.is_empty() {
        return None;
    }
    let after = shift_selection(selection, &changes);
    Some(Transaction::new(changes, selection, after, EditKind::Other))
}

/// 返回把所有换行符替换为 `ending` 的事务；换行符已全部是 `ending` 时返回 `None`。
///
/// `ending` 取 `"\n"`、`"\r\n"` 或 `"\r"` 之一。
pub fn change_line_endings(
    text: &impl Lines,
    selection: Selection,
    ending: &str,
) -> Option<Transaction> {
    let mut changes = Vec::new();
    let mut shift: isize = 0;
    let target = ending.chars().count();
    for line in 0..text.line_count().saturating_sub(1) {
        let (end, next) = (text.line_end(line), text.line_start(line + 1));
        let len = next - end;
        // 换行符只有 LF、CRLF、CR 三种：两个字符的一定是 CRLF，不必取出原文比较；都是一个字符
        // 时必须取出来才分得清 LF 和 CR（`char_at` 的默认实现把换行符一律报告为 `\n`）。
        let old = match (len, target) {
            (2, 2) => continue,
            (1, 1) => match text.substring(end, next) {
                old if old == ending => continue,
                old => old,
            },
            _ => text.substring(end, next),
        };
        let delta = target as isize - len as isize;
        changes.push(Change {
            start: end.saturating_add_signed(shift),
            removed: old,
            inserted: ending.to_owned(),
        });
        shift += delta;
    }
    if changes.is_empty() {
        return None;
    }
    let after = shift_selection(selection, &changes);
    Some(Transaction::new(changes, selection, after, EditKind::Other))
}

/// 查找括号时最多扫描的字符数（字符）。
///
/// 100 万为经验值，用于避免在超大文件中扫描整个文件。
const SCAN_LIMIT: usize = 1_000_000;

/// 返回右括号 `close` 对应的左括号；`close` 不是右括号时返回 `None`。
fn opener_of(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// 返回 `pos` 之前最近的、尚未闭合的左括号的位置；超出扫描预算时返回 `None`。
///
/// 按行倒序扫描，不逐字符调用 `char_at`：大文件的每一行都要从磁盘读取并解码，逐字符调用会为
/// 每个字符重新读取并解码所在行。每行的换行符同样计入 `SCAN_LIMIT` 的字符预算。
fn enclosing_open(text: &impl Lines, pos: usize) -> Option<usize> {
    let mut closers: Vec<char> = Vec::new();
    let mut budget = SCAN_LIMIT;
    let (line, col) = text.pos_to_line_col(pos);
    for l in (0..=line).rev() {
        budget = budget.checked_sub(1)?;
        let start = text.line_start(l);
        let content = text.line_content(l);
        let slice = content.slice();
        // 光标所在行只扫描光标之前的部分，其余行扫描全行。
        let end = if l == line {
            col.min(slice.len_chars())
        } else {
            slice.len_chars()
        };
        for (i, c) in (0..end).rev().zip(slice.chars_at(end).reversed()) {
            budget = budget.checked_sub(1)?;
            if opener_of(c).is_some() {
                closers.push(c);
            } else if matches!(c, '(' | '[' | '{') {
                match closers.last() {
                    Some(&close) if opener_of(close) == Some(c) => {
                        closers.pop();
                    }
                    Some(_) => {}
                    None => return Some(start + i),
                }
            }
        }
    }
    None
}

/// 返回「转到括号」（`⇧⌘\`）的目标位置；找不到括号时返回 `None`。
///
/// 光标紧邻括号时跳到配对的括号之前；不紧邻时跳到包住光标的左括号；外面没有括号时跳到之后的
/// 第一个括号。
pub fn jump_to_bracket(text: &impl Lines, pos: usize) -> Option<usize> {
    if let Some((open, close)) = movement::matching_bracket(text, pos, SCAN_LIMIT) {
        let target = if pos == open || pos == open + 1 {
            close
        } else {
            open
        };
        // 空括号 `{}` 之间：光标两侧都紧邻括号，目标会与当前位置重合，此时改为跳到左括号。
        return Some(if target == pos { open } else { target });
    }
    if let Some(open) = enclosing_open(text, pos) {
        return Some(open);
    }
    // 向后查找同样按行读取，理由与 `enclosing_open` 相同。
    let mut budget = SCAN_LIMIT;
    let (line, col) = text.pos_to_line_col(pos);
    for l in line..text.line_count() {
        budget = budget.checked_sub(1)?;
        let content = text.line_content(l);
        let slice = content.slice();
        let skip = if l == line {
            col.min(slice.len_chars())
        } else {
            0
        };
        for (i, c) in slice.chars_at(skip).enumerate() {
            budget = budget.checked_sub(1)?;
            if matches!(c, '(' | '[' | '{' | ')' | ']' | '}') {
                return Some(text.line_start(l) + skip + i);
            }
        }
    }
    None
}

/// 返回扩大选区（`⌃⇧⌘→`）后的选区；已经是整个文件时返回 `None`。
///
/// 依次扩大到光标所在的词、引号内的字符串、括号内的内容、所在的行和整个文件。字符串与括号各分
/// 不含、含两侧符号两级，行分不含缩进、整行、含换行符三级。只按文本写法判断，不依赖语言服务。
pub fn expand_selection(text: &impl Lines, selection: Selection) -> Option<Selection> {
    let (from, to) = (selection.from(), selection.to());
    let wider = |&(a, b): &(usize, usize)| a <= from && to <= b && (a, b) != (from, to);

    let mut candidates = Vec::new();
    if selection.is_empty() {
        let (start, end) = movement::word_at(text, from);
        if text
            .substring(start, end)
            .chars()
            .any(|c| c.is_alphanumeric() || c == '_')
        {
            candidates.push((start, end));
        }
    }
    candidates.extend(strings_around(text, from, to));
    // 包住选区的括号对：选区超出内层括号对时（例如 `[a], [b]` 中选了中间的 `], [`），继续向外
    // 查找。各层共用 `SCAN_LIMIT` 个字符的预算，按两个括号之间的距离扣除。
    let mut at = from;
    let mut budget = SCAN_LIMIT;
    while let Some(open) = enclosing_open(text, at) {
        let Some((first, close)) = movement::matching_bracket(text, open + 1, budget) else {
            break;
        };
        budget = budget.saturating_sub(close.abs_diff(first));
        // `matching_bracket` 先看左边、再看右边的字符：`open` 本身没有配对（括号还没写完）时，
        // 该函数会改用右边紧邻的括号，配出来的不是 `open` 这一对，跳过它继续向外找。
        if first != open {
            at = open;
            continue;
        }
        if close >= to {
            candidates.push((open + 1, close));
            candidates.push((open, close + 1));
            break;
        }
        at = open;
    }
    let (first, last) = (text.line_of(from), text.line_of(to));
    let indent = crate::text::leading_whitespace(text.line_content(first).slice());
    candidates.push((text.line_start(first) + indent, text.line_end(last)));
    candidates.push((text.line_start(first), text.line_end(last)));
    candidates.push((text.line_span(first).start, text.line_span(last).end));
    candidates.push((0, text.end_pos()));

    // 在所有比当前选区更宽的候选中取最窄的一个。
    candidates
        .into_iter()
        .filter(wider)
        .min_by_key(|&(a, b)| b - a)
        .map(|(a, b)| Selection::new(a, b))
}

/// 返回交换光标两侧两个字素（`⌃T`）的事务。
///
/// 光标在行尾时交换它前面的两个字素，光标不动；否则交换光标前后的两个字素，光标移到两者之后。
/// 在行首时前一个字素是换行符，交换后本行的第一个字素接到上一行的末尾。两个字素相同时不改动
/// 文字，光标照样前移，连按可以把一个字素一路往后挪。选区非空、光标前不足两个字素时返回 `None`。
pub fn transpose(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    if !selection.is_empty() {
        return None;
    }
    let pos = selection.head;
    let end = if pos == text.line_end(text.line_of(pos)) {
        pos
    } else {
        movement::next_grapheme(text, pos)
    };
    let middle = movement::prev_grapheme(text, end);
    let start = movement::prev_grapheme(text, middle);
    if start == middle {
        return None;
    }
    let (head, tail) = (text.substring(start, middle), text.substring(middle, end));
    let changes = if head == tail {
        Vec::new()
    } else {
        vec![Change {
            start,
            removed: text.substring(start, end),
            inserted: format!("{tail}{head}"),
        }]
    };
    Some(Transaction::new(
        changes,
        selection,
        Selection::point(end),
        EditKind::Other,
    ))
}

/// 返回把 `\uXXXX` 转义换成对应字符的事务；没有可换的转义时返回 `None`。
///
/// 有选区时只转换选区，否则转换整个文件。两个反斜杠是一对，后面的 `u` 不算转义；成对的高、低
/// 代理项合成一个字符。落单的代理项、控制字符、行分隔符和零宽字符等看不见的字符保留转义写法。
pub fn unescape_unicode(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    convert_runs(text, selection, |chars, i| {
        if chars[i] != '\\' {
            return Run::Keep(1);
        }
        if chars.get(i + 1) == Some(&'\\') {
            return Run::Keep(2);
        }
        match unicode_escape_at(chars, i, true) {
            Some((c, len)) => Run::Replace(len, c.to_string()),
            None => Run::Keep(1),
        }
    })
}

/// 返回把 ASCII 以外的字符写成 `\uXXXX` 转义的事务（十六进制大写，补充平面的字符写成一对代理项）；
/// 没有这样的字符时返回 `None`。
///
/// 有选区时只转换选区，否则转换整个文件；已有的转义原样保留。转义前面是落单的反斜杠时，这个
/// 反斜杠写成 `\u005C`，再转回字符时仍是原来的文字。
pub fn escape_unicode(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    convert_runs(text, selection, |chars, i| {
        let c = chars[i];
        if c == '\\' {
            let n = chars[i..].iter().take_while(|&&c| c == '\\').count();
            let lone = n % 2 == 1 && chars.get(i + n).is_some_and(|c| !c.is_ascii());
            return match n {
                1 if lone => Run::Replace(1, "\\u005C".to_owned()),
                _ if lone => Run::Keep(n - 1),
                _ => Run::Keep(n),
            };
        }
        if c.is_ascii() {
            return Run::Keep(1);
        }
        let mut escaped = String::new();
        push_escape(&mut escaped, c, false);
        Run::Replace(1, escaped)
    })
}

/// 把 `text`（`.properties` 的内容）中的 `\uXXXX` 转义换成对应的字符，规则同
/// [`unescape_unicode`]，但 ASCII 字符的转义保留：`\u003D`、`\u005C` 等换成字符会改变含义。
pub fn unescape_unicode_text(text: &str) -> String {
    if !text.contains("\\u") {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'\\') {
            out.push_str("\\\\");
            i += 2;
        } else if let Some((c, len)) = unicode_escape_at(&chars, i, false) {
            out.push(c);
            i += len;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// 把 `text` 中 ASCII 以外的字符写成 `\uXXXX` 转义，是 [`unescape_unicode_text`] 的逆运算。
///
/// `latin1` 为真时 ISO-8859-1 能表示的字符（U+00FF 以内）原样保留。`lowercase` 为真时十六进制
/// 用小写。落单的反斜杠后面的字符在 `.properties` 中是这个字符本身，转义时去掉这个反斜杠。
pub fn escape_unicode_text(text: &str, lowercase: bool, latin1: bool) -> String {
    let limit = if latin1 { 0xFF } else { 0x7F };
    let mut out = String::with_capacity(text.len());
    // `out` 末尾连续的反斜杠数。
    let mut slashes = 0;
    for c in text.chars() {
        if u32::from(c) <= limit {
            out.push(c);
            slashes = if c == '\\' { slashes + 1 } else { 0 };
            continue;
        }
        if slashes % 2 == 1 {
            out.pop();
        }
        slashes = 0;
        push_escape(&mut out, c, lowercase);
    }
    out
}

/// 把 `c` 写成 `\uXXXX` 转义接到 `out` 后面，补充平面的字符写成一对代理项。
fn push_escape(out: &mut String, c: char, lowercase: bool) {
    let mut units = [0; 2];
    for unit in c.encode_utf16(&mut units).iter() {
        if lowercase {
            out.push_str(&format!("\\u{unit:04x}"));
        } else {
            out.push_str(&format!("\\u{unit:04X}"));
        }
    }
}

/// [`convert_runs`] 中一处的处理：原样保留若干字符，或把若干字符换成新的文字。
enum Run {
    Keep(usize),
    Replace(usize, String),
}

/// 逐处扫描选区（没有选区时为整个文件）中的文字，按 `step` 的结果生成事务；相邻的替换合成一处
/// 修改。没有替换时返回 `None`，选区随文本调整。
fn convert_runs(
    text: &impl Lines,
    selection: Selection,
    step: impl Fn(&[char], usize) -> Run,
) -> Option<Transaction> {
    let (from, to) = if selection.is_empty() {
        (0, text.end_pos())
    } else {
        (selection.from(), selection.to())
    };
    let chars: Vec<char> = text.substring(from, to).chars().collect();
    let mut changes: Vec<Change> = Vec::new();
    // 已排入的修改造成的长度变化（字符）：后面的修改位置按应用前面的修改之后计。
    let mut delta = 0isize;
    let mut open: Option<(usize, String, String)> = None;
    let mut i = 0;
    while i < chars.len() {
        match step(&chars, i) {
            Run::Keep(len) => {
                if let Some(run) = open.take() {
                    push_run(&mut changes, &mut delta, from, run);
                }
                i += len;
            }
            Run::Replace(len, inserted) => {
                let run = open.get_or_insert_with(|| (i, String::new(), String::new()));
                run.1.extend(&chars[i..i + len]);
                run.2.push_str(&inserted);
                i += len;
            }
        }
    }
    if let Some(run) = open {
        push_run(&mut changes, &mut delta, from, run);
    }
    if changes.is_empty() {
        return None;
    }
    let after = shift_selection(selection, &changes);
    Some(Transaction::new(changes, selection, after, EditKind::Other))
}

/// 把一段相邻的替换（在扫描文字中的起点、原文、新文字）排入 `changes`。
fn push_run(
    changes: &mut Vec<Change>,
    delta: &mut isize,
    from: usize,
    (start, removed, inserted): (usize, String, String),
) {
    let removed_len = removed.chars().count() as isize;
    let inserted_len = inserted.chars().count() as isize;
    changes.push(Change {
        start: (from + start).saturating_add_signed(*delta),
        removed,
        inserted,
    });
    *delta += inserted_len - removed_len;
}

/// 读取 `chars[i]` 起的 `\uXXXX` 转义，返回对应的字符和转义占的字符数；不是可以显示的转义，或
/// `ascii` 为假而字符是 ASCII 时返回 `None`（见 [`unescape_unicode`]）。
fn unicode_escape_at(chars: &[char], i: usize, ascii: bool) -> Option<(char, usize)> {
    let unit = |at: usize| -> Option<u32> {
        if chars.get(at) != Some(&'\\') || chars.get(at + 1) != Some(&'u') {
            return None;
        }
        let digits = chars.get(at + 2..at + 6)?;
        digits
            .iter()
            .try_fold(0, |value, c| Some(value * 16 + c.to_digit(16)?))
    };
    let first = unit(i)?;
    let (scalar, len) = if (0xD800..=0xDBFF).contains(&first) {
        let low = unit(i + 6).filter(|low| (0xDC00..=0xDFFF).contains(low))?;
        (0x1_0000 + ((first - 0xD800) << 10) + (low - 0xDC00), 12)
    } else {
        (first, 6)
    };
    let c = char::from_u32(scalar)?;
    (!invisible(c) && (ascii || !c.is_ascii())).then_some((c, len))
}

/// 是否为显示出来看不见、或会断行的字符：控制字符、行和段落分隔符、零宽字符、方向控制符、BOM。
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// 返回包住选区的引号字符串的位置区间，依次是不含引号、含引号两种。
///
/// 只在选区所在的一行内查找，选区跨行时返回空表。
fn strings_around(text: &impl Lines, from: usize, to: usize) -> Vec<(usize, usize)> {
    let line = text.line_of(from);
    if text.line_of(to) != line {
        return Vec::new();
    }
    let base = text.line_start(line);
    let chars: Vec<char> = text.line_content(line).slice().chars().collect();
    let mut found = Vec::new();
    let mut open: Option<(usize, char)> = None;
    let mut escaped = false;
    for (i, &c) in chars.iter().enumerate() {
        match open {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some((start, q)) if c == q => {
                let (a, b) = (base + start, base + i + 1);
                if a <= from && to <= b {
                    found.push((a + 1, b - 1));
                    found.push((a, b));
                }
                open = None;
            }
            Some(_) => {}
            // 字母后面的撇号不是引号：`don't`、`it's`。
            None if c == '\'' && i > 0 && chars[i - 1].is_alphanumeric() => {}
            None if matches!(c, '"' | '\'' | '`') => open = Some((i, c)),
            None => {}
        }
    }
    found
}

#[cfg(test)]
mod tests;
