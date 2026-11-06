//! 编辑事务与撤销栈，以及生成事务的各种编辑操作。
//!
//! 编辑操作（输入、删除、缩进、移动行等）只读文本，返回一个 [`Transaction`]，由调用方应用到
//! 文本，再交给 [`History`] 记录。撤销时按相反顺序逐条撤回事务中的修改：删去插入的文本，
//! 恢复删除的文本。

mod history;

pub use history::*;

use std::time::{Duration, Instant};

use ropey::RopeSlice;

use crate::editing::movement;
use crate::editing::selection::Selection;
use crate::text::{self, EditBuffer, Lines};

/// 一处修改：在 `start` 处删除 `removed`，再插入 `inserted`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// 修改的起点（位置，从 0 起的 `char` 偏移）。
    pub start: usize,
    /// 被删除的文本。
    pub removed: String,
    /// 插入的文本。
    pub inserted: String,
}

impl Change {
    /// 应用这条修改：在 `start` 处删除 `removed` 并插入 `inserted`。
    pub fn apply(&self, text: &mut impl EditBuffer) {
        text.replace(self.start, self.removed.chars().count(), &self.inserted);
    }

    /// 撤回这条修改：在 `start` 处删除 `inserted` 并放回 `removed`。
    fn revert(&self, text: &mut impl EditBuffer) {
        text.replace(self.start, self.inserted.chars().count(), &self.removed);
    }
}

/// 撤销单元的类别，决定相邻的编辑能否合并成一个撤销单元。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// 连续键入的单个字符，可以和上一次合并成一个撤销单元。
    Typing,
    /// 其他编辑，各自是独立的撤销单元。
    Other,
}

/// 一次用户操作产生的全部修改，以及操作前后的选区。
#[derive(Debug, Clone)]
pub struct Transaction {
    /// 本次操作产生的全部修改，按应用顺序排列；撤销时按相反顺序逐条撤回。
    pub changes: Vec<Change>,
    /// 操作前的选区。
    pub before: Selection,
    /// 操作后的选区。
    pub after: Selection,
    /// 多光标编辑时，操作前主选区以外的其余选区（按位置排序）；单光标时为空。
    pub extra_before: Vec<Selection>,
    /// 多光标编辑时，操作后主选区以外的其余选区（按位置排序）；单光标时为空。
    pub extra_after: Vec<Selection>,
    /// 撤销单元的类别，决定能否与上一次键入合并。
    pub kind: EditKind,
    /// 操作发生的时间，用于判断两次键入是否落在 [`MERGE_WINDOW`] 之内。
    time: Instant,
    /// 本次操作的版本号，入栈时由 [`History::push`] 分配；0 表示尚未入栈。
    version: u64,
    /// 在撤销栈中占用的内存（字节，按 [`changes_size`] 估算），入栈时由 [`History::push`]
    /// 计算。
    bytes: usize,
}

impl Transaction {
    /// 创建记录一组修改和操作前后选区的事务。
    ///
    /// 时间取当前时刻，版本号由 [`History::push`] 在入栈时分配。其余选区（多光标）为空，由多光标
    /// 编辑另行填入 `extra_before`、`extra_after`。
    pub fn new(changes: Vec<Change>, before: Selection, after: Selection, kind: EditKind) -> Self {
        Self {
            changes,
            before,
            after,
            extra_before: Vec::new(),
            extra_after: Vec::new(),
            kind,
            time: Instant::now(),
            version: 0,
            bytes: 0,
        }
    }

    /// 按顺序应用全部修改。
    pub fn apply(&self, text: &mut impl EditBuffer) {
        if let [change] = &self.changes[..] {
            change.apply(text);
            return;
        }
        let changes: Vec<_> = (self.changes.iter())
            .map(|c| (c.start, c.removed.chars().count(), c.inserted.as_str()))
            .collect();
        text.replace_each(&changes);
    }

    /// 返回修改涉及的最小位置（`char` 偏移），用于决定从哪一行开始重新高亮；没有修改时返回 0。
    pub fn first_offset(&self) -> usize {
        self.changes.iter().map(|c| c.start).min().unwrap_or(0)
    }
}

/// 编辑操作需要的语言相关参数。
#[derive(Debug, Clone, Copy)]
pub struct EditOptions<'a> {
    /// 制表位宽度（显示列）。
    pub tab_width: usize,
    /// 一级缩进的文本，例如 4 个空格或一个制表符。
    pub indent_unit: &'a str,
    /// 插入换行时使用的换行符，取自文档的行尾风格。
    pub line_ending: &'a str,
    /// 换行后多缩进一级的触发字符：光标之前最近的非空白字符属于其中时多缩进一级。
    ///
    /// 例如 Python 的 `:`、C 系语言的 `{`。为空时不做多缩进，括号也不参与判断。
    pub indent_after: &'a [char],
    /// 是否在光标之前以开始标签结尾时多缩进一级（HTML、XML），规则见 [`newline`]。
    pub tag_indent: bool,
    /// 行注释前缀（不含缩进，例如 `//`、`#`）；语言没有行注释时为 `None`。
    pub line_comment: Option<&'a str>,
    /// 单引号是否界定字符串（Python、JS 为真）。
    ///
    /// Rust 的单引号多用于生命周期，为假。
    pub single_quote_string: bool,
}

/// 返回用 `inserted` 替换选区的事务，光标落在插入文本之后。
pub fn replace_selection(
    text: &impl Lines,
    selection: Selection,
    inserted: &str,
    kind: EditKind,
) -> Transaction {
    let range = selection.range();
    let removed = text.substring(range.start, range.end);
    let after = Selection::point(range.start + inserted.chars().count());

    Transaction::new(
        vec![Change {
            start: range.start,
            removed,
            inserted: inserted.to_owned(),
        }],
        selection,
        after,
        kind,
    )
}

/// 返回删除位置区间 `from..to` 的事务，光标落在 `from`；区间为空时返回 `None`。
pub(crate) fn delete_range(
    text: &impl Lines,
    selection: Selection,
    from: usize,
    to: usize,
) -> Option<Transaction> {
    if from >= to {
        return None;
    }

    let removed = text.substring(from, to);

    Some(Transaction::new(
        vec![Change {
            start: from,
            removed,
            inserted: String::new(),
        }],
        selection,
        Selection::point(from),
        EditKind::Other,
    ))
}

/// 返回退格的事务；没有可删内容时返回 `None`。
///
/// 有选区时删除选区；`word` 为真时删到上一个词的开头。光标位于行首缩进内、且缩进全是空格时，
/// 按缩进单位退到上一个缩进位置。
pub fn backspace(
    text: &impl Lines,
    selection: Selection,
    word: bool,
    options: &EditOptions<'_>,
) -> Option<Transaction> {
    if !selection.is_empty() {
        return delete_range(text, selection, selection.from(), selection.to());
    }

    let pos = selection.head;
    let (line, col) = text.pos_to_line_col(pos);
    let content = text.line_content(line);
    let slice = content.slice();
    let indent = text::leading_whitespace(slice);

    let from = if word {
        movement::prev_word(text, pos)
    } else if col > 0 && col <= indent && slice.chars().take(col).all(|c| c == ' ') {
        // 在缩进里：删到上一个缩进位置。
        let unit = options.indent_unit.chars().count().max(1);
        let target = (col - 1) / unit * unit;
        text.line_start(line) + target
    } else {
        movement::prev_grapheme(text, pos)
    };

    delete_range(text, selection, from, pos)
}

/// 返回删除光标前后各一个字符的事务，用于删除空的括号或引号对；光标位于文档开头时返回 `None`。
///
/// 撤销后光标恢复到两个字符之间。
pub fn delete_pair(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    let from = selection.head.checked_sub(1)?;
    delete_range(text, selection, from, selection.head + 1)
}

/// 返回删除到行首（`⌘⌫`，macOS）的事务。
///
/// 有选区时删除选区；光标已在行首时等同于退格，即删除前一个字素，与上一行相接。
pub fn delete_to_line_start(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    if !selection.is_empty() {
        return delete_range(text, selection, selection.from(), selection.to());
    }
    let pos = selection.head;
    let start = text.line_start(text.line_of(pos));
    let from = if pos == start {
        movement::prev_grapheme(text, pos)
    } else {
        start
    };
    delete_range(text, selection, from, pos)
}

/// 返回向后删除一个字素（`word` 为真时删除一个词）的事务；有选区时删除选区。
pub fn delete_forward(text: &impl Lines, selection: Selection, word: bool) -> Option<Transaction> {
    if !selection.is_empty() {
        return delete_range(text, selection, selection.from(), selection.to());
    }

    let pos = selection.head;
    let to = if word {
        movement::next_word(text, pos)
    } else {
        movement::next_grapheme(text, pos)
    };
    delete_range(text, selection, pos, to)
}

/// 返回回车（插入换行）的事务。
///
/// 新行沿用当前行的缩进。光标之前最近的非空白字符属于 `indent_after`、这一行含未闭合的括号
/// （例如 `foo(a,`），或开启 `tag_indent` 时以开始标签结尾，则多缩进一级。结束标签、自闭合
/// 标签、注释、声明和空元素不算。光标前后最近的非空白字符是一对括号时，右括号移到再下一行。
pub fn newline(text: &impl Lines, selection: Selection, options: &EditOptions<'_>) -> Transaction {
    let from = selection.from();
    let (line, col) = text.pos_to_line_col(from);
    let content = text.line_content(line);
    let slice = content.slice();
    let indent: String = slice
        .chars()
        .take_while(|&c| c == ' ' || c == '\t')
        .take(col)
        .collect();
    let before = slice.chars_at(col).reversed().find(|c| !c.is_whitespace());
    // 选区内的文本会被替换，因此向后取字符要从选区末端开始。
    let (end_line, end_col) = text.pos_to_line_col(selection.to());
    let end_content = text.line_content(end_line);
    let rest = end_content.slice().slice(end_col..);
    let after = rest.chars().find(|c| !c.is_whitespace());

    // 纯文本和 Markdown 的 `indent_after` 为空，括号也不参与判断：行文中的括号不是代码结构。
    let opens = before.is_some_and(|c| options.indent_after.contains(&c))
        || (!options.indent_after.is_empty() && unclosed_bracket(slice.slice(..col), options))
        || (options.tag_indent && before == Some('>') && opens_tag(slice.slice(..col), rest));
    let between_pair = matches!(
        (before, after),
        (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
    );

    let mut inserted = String::from(options.line_ending);
    inserted.push_str(&indent);

    if opens {
        inserted.push_str(options.indent_unit);
    }

    let caret = from + inserted.chars().count();

    if between_pair {
        inserted.push_str(options.line_ending);
        inserted.push_str(&indent);
    }

    let mut transaction = replace_selection(text, selection, &inserted, EditKind::Other);
    transaction.after = Selection::point(caret);
    transaction
}

/// 不需要结束标签的 HTML 空元素：其后换行不多缩进。
///
/// 依据 HTML 规范中不允许有内容、因此省略结束标签的元素列表。
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// `before`（光标之前的一段行内容，最后一个非空白字符是 `>`）是否以开始标签结尾，且光标之后的
/// `rest`（同一行余下的内容）里没有该开始标签的结束标签；规则见 [`newline`]。
///
/// 只看这一行：属性写成多行的标签，最后一行里找不到 `<`，不算。
fn opens_tag(before: RopeSlice<'_>, rest: RopeSlice<'_>) -> bool {
    // 从光标往回找最后一个 `>` 和它前面最近的 `<`，只扫描这一段，很长的一行也不整行取出。
    let mut back = before.chars_at(before.len_chars()).reversed();
    let Some(close) = back
        .position(|c| c == '>')
        .map(|n| before.len_chars() - 1 - n)
    else {
        return false;
    };
    let Some(open) = back.position(|c| c == '<').map(|n| close - 1 - n) else {
        return false;
    };
    // 标签名紧跟 `<`：`</div>`、`<!-- -->`、`<!DOCTYPE>`、`<?xml ?>` 和 `a < b > c` 都没有。
    let tag = before.slice(open + 1..close);
    let last = tag.len_chars().checked_sub(1).and_then(|i| tag.get_char(i));
    if !tag.get_char(0).is_some_and(|c| c.is_alphabetic()) || last == Some('/') {
        return false;
    }
    let name: String = tag
        .chars()
        .take_while(|&c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if VOID_ELEMENTS.contains(&name.as_str()) {
        return false;
    }
    // 光标之后有没有 `</name`（不分大小写）：逐个位置比，不把余下的整段拼成字符串。
    let end: Vec<char> = format!("</{name}").chars().collect();
    let mut at = rest.chars();
    loop {
        let here = at.clone().map(|c| c.to_ascii_lowercase()).take(end.len());
        if here.eq(end.iter().copied()) {
            return false;
        }
        if at.next().is_none() {
            return true;
        }
    }
}

/// 是否存在未闭合的括号；`before` 是光标之前的一段行内容。
///
/// 字符串内和行注释之后的括号不计入。逐个字符扫描，不复制这一段。
fn unclosed_bracket(before: RopeSlice<'_>, options: &EditOptions<'_>) -> bool {
    let comment = options.line_comment.map_or("", str::trim_end);
    let starts_comment = |c: char, rest: &ropey::iter::Chars<'_>| {
        !comment.is_empty()
            && comment.chars().eq(std::iter::once(c)
                .chain(rest.clone())
                .take(comment.chars().count()))
    };
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    let mut chars = before.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if starts_comment(c, &chars) => break,
            None => match c {
                '"' | '`' => quote = Some(c),
                '\'' if options.single_quote_string => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = (depth - 1).max(0),
                _ => {}
            },
        }
    }
    depth > 0
}

/// 返回选区覆盖的行范围（起止行号，从 0 起，含两端）。
///
/// 选区跨越多行且结束在某行的第 0 列时，该行不计入。
pub fn selected_lines(text: &impl Lines, selection: Selection) -> (usize, usize) {
    let (first, _) = text.pos_to_line_col(selection.from());
    let (mut last, last_col) = text.pos_to_line_col(selection.to());

    if last > first && last_col == 0 {
        last -= 1;
    }

    (first, last)
}

/// 返回 Tab 键的事务。
///
/// 没有选区时在光标处插入一级缩进（缩进单位是空格时补齐到下一个缩进位）；有选区时对选区
/// 覆盖的行整体缩进。
pub fn indent(text: &impl Lines, selection: Selection, options: &EditOptions<'_>) -> Transaction {
    let multi_line = {
        let (a, b) = selected_lines(text, selection);
        a != b
    };

    if !multi_line && selection.is_empty() {
        let (line, col) = text.pos_to_line_col(selection.head);

        let inserted = if options.indent_unit == "\t" {
            "\t".to_owned()
        } else {
            let unit = options.indent_unit.chars().count().max(1);
            let cell = text::col_to_cell(text.line_content(line).slice(), col, options.tab_width);
            " ".repeat(unit - cell % unit)
        };

        return replace_selection(text, selection, &inserted, EditKind::Other);
    }

    indent_lines(text, selection, options)
}

/// 返回增加缩进（`⌘]`）的事务：选区覆盖的行（没有选区时为光标所在行）各加一级缩进。
///
/// 结果与光标在行内的位置无关；空行和只含空白的行不加。
pub fn indent_lines(
    text: &impl Lines,
    selection: Selection,
    options: &EditOptions<'_>,
) -> Transaction {
    let (first, last) = selected_lines(text, selection);
    let unit_len = options.indent_unit.chars().count();
    let mut changes = Vec::new();

    for line in first..=last {
        if text::is_blank(text.line_content(line).slice()) {
            continue;
        }

        // 修改按顺序应用：前面各行已插入缩进单位，后面行的起点要加上这部分偏移。
        let shift = changes.len() * unit_len;
        changes.push(Change {
            start: text.line_start(line) + shift,
            removed: String::new(),
            inserted: options.indent_unit.to_owned(),
        });
    }

    let after = shift_selection(selection, &changes);
    Transaction::new(changes, selection, after, EditKind::Other)
}

/// 返回减少缩进（`⇧⇥`）的事务：选区覆盖的各行最多去掉一级缩进。
///
/// 没有行可以去掉缩进时返回 `None`。
pub fn outdent(
    text: &impl Lines,
    selection: Selection,
    options: &EditOptions<'_>,
) -> Option<Transaction> {
    let (first, last) = selected_lines(text, selection);
    // 一级缩进折合的空格数：缩进单位是制表符时按 `tab_width` 计算，使用空格缩进的行也能一次退一级。
    let spaces = if options.indent_unit == "\t" {
        options.tab_width.max(1)
    } else {
        options.indent_unit.chars().count().max(1)
    };
    let mut changes = Vec::new();
    let mut removed_total = 0usize;
    // 末行去掉缩进后，剩余缩进末尾的位置（修改后的偏移）；末行没有去掉缩进时为 `None`。
    let mut last_indent_end = None;

    for line in first..=last {
        let content = text.line_content(line);
        let slice = content.slice();
        let count = match slice.chars().next() {
            Some('\t') => 1,
            Some(' ') => slice.chars().take(spaces).take_while(|c| *c == ' ').count(),
            _ => 0,
        };

        if count == 0 {
            continue;
        }

        let start = text.line_start(line) - removed_total;
        if line == last {
            last_indent_end = Some(start + text::leading_whitespace(slice) - count);
        }
        changes.push(Change {
            start,
            removed: slice.slice(..count).to_string(),
            inserted: String::new(),
        });
        removed_total += count;
    }

    if changes.is_empty() {
        return None;
    }

    let mut after = shift_selection(selection, &changes);

    // 跨行选区的结尾落在末行被去掉的缩进里时，按位置换算会移到行首，使末行不再计入选区，
    // 再按一次只减少前面几行的缩进。改为停在剩余缩进的末尾，选区仍覆盖同样的行。
    if first != last
        && let Some(end) = last_indent_end
    {
        let to = selection.to();
        let (line, col) = text.pos_to_line_col(to);
        let removed = changes.last().map_or(0, |c| c.removed.chars().count());
        if line == last && col > 0 && col <= removed {
            if selection.head == to {
                after.head = end;
            } else {
                after.anchor = end;
            }
        }
    }

    Some(Transaction::new(changes, selection, after, EditKind::Other))
}

/// 返回切换行注释的事务；语言没有行注释，或覆盖的行都是空行时返回 `None`。
///
/// 所有非空行都已注释时取消注释，否则在这些行的最小缩进处加上注释前缀。
pub fn toggle_comment(
    text: &impl Lines,
    selection: Selection,
    options: &EditOptions<'_>,
) -> Option<Transaction> {
    let prefix = options.line_comment?;
    let (first, last) = selected_lines(text, selection);
    let trimmed = prefix.trim_end();

    let lines: Vec<usize> = (first..=last)
        .filter(|&l| !text::is_blank(text.line_content(l).slice()))
        .collect();

    if lines.is_empty() {
        return None;
    }

    let all_commented = lines.iter().all(|&l| {
        let content = text.line_content(l);
        let slice = content.slice();
        text::starts_with(slice.slice(text::leading_whitespace(slice)..), trimmed)
    });

    let mut changes = Vec::new();
    let mut delta: isize = 0;

    if all_commented {
        for &line in &lines {
            let content = text.line_content(line);
            let slice = content.slice();
            let indent = text::leading_whitespace(slice);
            let remove = if text::starts_with(slice.slice(indent..), prefix) {
                prefix
            } else {
                trimmed
            };
            let count = remove.chars().count();
            let start = (text.line_start(line) as isize + indent as isize + delta) as usize;
            changes.push(Change {
                start,
                removed: remove.to_owned(),
                inserted: String::new(),
            });
            delta -= count as isize;
        }
    } else {
        let column = lines
            .iter()
            .map(|&l| text::leading_whitespace(text.line_content(l).slice()))
            .min()
            .unwrap_or(0);

        for &line in &lines {
            let start = (text.line_start(line) as isize + column as isize + delta) as usize;
            changes.push(Change {
                start,
                removed: String::new(),
                inserted: prefix.to_owned(),
            });
            delta += prefix.chars().count() as isize;
        }
    }

    let after = shift_selection(selection, &changes);
    Some(Transaction::new(changes, selection, after, EditKind::Other))
}

/// 返回粘贴的事务；多行代码在满足条件时先调整缩进。
///
/// 光标之前只有缩进（空格或制表符）时，把整块代码移到光标所在的缩进上，块内各行的相对缩进不变。
/// 以下情况原样粘贴：
///
/// - 第一行不带缩进。从行中间开始复制时，第一行的缩进已丢失，无法确定其余各行该移动多少。
/// - 光标在行首。空行没有缩进，粘贴通常是为了保留代码原有的缩进，而不是顶格。
/// - 光标之前有非空白字符，或粘贴内容只有一行。
pub fn paste(text: &impl Lines, selection: Selection, pasted: &str) -> Transaction {
    let (line, col) = text.pos_to_line_col(selection.from());
    let content = text.line_content(line);
    let slice = content.slice();
    // 光标之前有非空白字符时原样粘贴：先数缩进，很长的一行上不必把光标之前的整段取出。
    let fitted = (text::leading_whitespace(slice) >= col)
        .then(|| fit_indent(pasted, &slice.slice(..col).to_string()))
        .flatten();
    replace_selection(
        text,
        selection,
        fitted.as_deref().unwrap_or(pasted),
        EditKind::Other,
    )
}

/// 返回把 `pasted` 移到缩进 `target` 上的结果；不需要调整时返回 `None`。
///
/// 按行切分时各行保留自己的换行符，LF、CRLF 和单独的 CR 都能认出（粘贴的内容已换成文档的换行符）。
fn fit_indent(pasted: &str, target: &str) -> Option<String> {
    let is_indent = |c: char| c == ' ' || c == '\t';
    let lines = crate::diff::lines(pasted);
    let first = lines.first()?;
    let indented = !target.is_empty() && target.chars().all(is_indent);
    if !pasted.contains(['\n', '\r']) || !indented || !first.starts_with(is_indent) {
        return None;
    }

    let blank = |line: &str| line.trim_end_matches(['\r', '\n']).chars().all(is_indent);
    let base = lines
        .iter()
        .filter(|l| !blank(l))
        .map(|l| &l[..l.len() - l.trim_start_matches(is_indent).len()])
        .reduce(|a, b| {
            let common = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            &a[..common]
        })?;

    let mut fitted = String::with_capacity(pasted.len() + lines.len() * target.len());
    for (i, line) in lines.iter().enumerate() {
        match line.strip_prefix(base) {
            _ if blank(line) => fitted.push_str(line.trim_start_matches(is_indent)),
            Some(rest) if i == 0 => fitted.push_str(rest),
            Some(rest) => {
                fitted.push_str(target);
                fitted.push_str(rest);
            }
            None => fitted.push_str(line),
        }
    }
    Some(fitted)
}

/// 返回在光标所在行上方插入整行内容的事务，光标随原来的行下移。
///
/// `lines` 以换行符结尾。没有选区时复制和剪切的是整行，粘贴也按整行处理，因此只用于空选区。
pub fn insert_lines_above(text: &impl Lines, selection: Selection, lines: &str) -> Transaction {
    let (line, _) = text.pos_to_line_col(selection.head);
    let start = text.line_start(line);
    let after = Selection::point(selection.head + lines.chars().count());

    Transaction::new(
        vec![Change {
            start,
            removed: String::new(),
            inserted: lines.to_owned(),
        }],
        selection,
        after,
        EditKind::Other,
    )
}

/// 返回把当前行（或选区覆盖的行）与上一行（`down` 为真时与下一行）对调的事务，选区随之移动。
///
/// 行与行之间原有的换行符原样保留，不会把 CRLF 改成 LF。已在首行（向上）或末行（向下）时
/// 返回 `None`。
pub fn move_lines(text: &impl Lines, selection: Selection, down: bool) -> Option<Transaction> {
    let (first, last) = selected_lines(text, selection);

    // `neighbor` 是被对调的相邻行（上一行或下一行），`start..end` 是两段合起来的位置范围。
    let (start, end, inserted, shift) = if down {
        if last + 1 >= text.line_count() {
            return None;
        }
        let block = text.substring(text.line_start(first), text.line_end(last));
        let separator = text.substring(text.line_end(last), text.line_start(last + 1));
        let neighbor = text.substring(text.line_start(last + 1), text.line_end(last + 1));
        let shift = neighbor.chars().count() + separator.chars().count();
        (
            text.line_start(first),
            text.line_end(last + 1),
            format!("{neighbor}{separator}{block}"),
            shift as isize,
        )
    } else {
        if first == 0 {
            return None;
        }
        let neighbor = text.substring(text.line_start(first - 1), text.line_end(first - 1));
        let separator = text.substring(text.line_end(first - 1), text.line_start(first));
        let block = text.substring(text.line_start(first), text.line_end(last));
        let shift = neighbor.chars().count() + separator.chars().count();
        (
            text.line_start(first - 1),
            text.line_end(last),
            format!("{block}{separator}{neighbor}"),
            -(shift as isize),
        )
    };

    // 选区停在下一行行首（向下移动、下一行是最后一行）时，平移后会越过文本末尾一个换行符的
    // 长度。文本总长度不变，限制在末尾之内。
    let moved = |pos: usize| pos.saturating_add_signed(shift).min(text.end_pos());
    Some(Transaction::new(
        vec![Change {
            start,
            removed: text.substring(start, end),
            inserted,
        }],
        selection,
        Selection::new(moved(selection.anchor), moved(selection.head)),
        EditKind::Other,
    ))
}

/// 返回复制当前行（或选区覆盖的行）的事务：副本放在原行下方（`down` 为真）或上方，选区落在
/// 副本上。
pub fn copy_lines(
    text: &impl Lines,
    selection: Selection,
    down: bool,
    options: &EditOptions<'_>,
) -> Transaction {
    let (first, last) = selected_lines(text, selection);
    let block = text.substring(text.line_start(first), text.line_end(last));

    let (start, inserted, after) = if down {
        // 插在块末尾：插入「换行 + 块」，选区后移到副本上。
        let inserted = format!("{}{block}", options.line_ending);
        let offset = inserted.chars().count();
        let after = Selection::new(selection.anchor + offset, selection.head + offset);
        (text.line_end(last), inserted, after)
    } else {
        // 插在块之前：插入「块 + 换行」，原行下移；选区位置不变，因此落在上方的副本上。
        let inserted = format!("{block}{}", options.line_ending);
        (text.line_start(first), inserted, selection)
    };

    Transaction::new(
        vec![Change {
            start,
            removed: String::new(),
            inserted,
        }],
        selection,
        after,
        EditKind::Other,
    )
}

/// 返回删除当前行（或选区覆盖的行，含换行符）的事务；没有可删内容时返回 `None`。
///
/// 光标保持原来的列号（不超过目标行的长度），落到删除后占据该位置的行上。
pub fn delete_lines(text: &impl Lines, selection: Selection) -> Option<Transaction> {
    let (first, last) = selected_lines(text, selection);
    let (_, col) = text.pos_to_line_col(selection.head);
    let start = text.line_start(first);

    // 删除后光标所在的行：后面还有行时是原来的下一行，否则是上一行；全部删除后文本为空。
    let (from, to, after) = if last + 1 < text.line_count() {
        (
            start,
            text.line_start(last + 1),
            start + col.min(text.line_len(last + 1)),
        )
    } else if first > 0 {
        let above = text.line_start(first - 1) + col.min(text.line_len(first - 1));
        (text.line_end(first - 1), text.end_pos(), above)
    } else {
        (start, text.end_pos(), 0)
    };

    let mut transaction = delete_range(text, selection, from, to)?;
    transaction.after = Selection::point(after);
    Some(transaction)
}

/// 按顺序把选区两端映射过每一处修改，返回新选区。
///
/// 修改位于位置之前时，位置按增减量平移；位置在被删除的区间内时，移到修改点。修改点正好
/// 落在位置上时，非空选区的起点留在原地（整行缩进后选区仍从行首开始），其余情况随修改平移。
pub fn shift_selection(selection: Selection, changes: &[Change]) -> Selection {
    let map = |mut pos: usize, stays_at_start: bool| {
        for change in changes {
            let removed = change.removed.chars().count();
            let inserted = change.inserted.chars().count();

            if pos < change.start || (pos == change.start && stays_at_start) {
                continue;
            }

            pos = if pos < change.start + removed {
                change.start
            } else {
                pos - removed + inserted
            };
        }

        pos
    };

    let anchor_is_from = selection.anchor <= selection.head && !selection.is_empty();
    let head_is_from = selection.head < selection.anchor;

    Selection::new(
        map(selection.anchor, anchor_is_from),
        map(selection.head, head_is_from),
    )
}

#[cfg(test)]
mod tests;
