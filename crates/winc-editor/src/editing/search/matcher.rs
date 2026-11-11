//! 匹配器：纯文本、正则和跨行的查找，替换模板的展开，以及按字节判断整词。
//!
//! `search` 的 `Matcher` 编译一次查询后供多次查找使用：`Pattern` 在 `regex` 与 `fancy-regex`
//! 之间选择实现，回溯过多时返回 `GaveUp`。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// 展开替换文字中的转义：`\n` 换成 `line_ending`，`\t` 换成制表符，`\\` 换成单个反斜杠，
/// 其余原样保留。
pub(crate) fn unescape(template: &str, line_ending: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars();

    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some('n')) => out.push_str(line_ending),
            ('\\', Some('t')) => out.push('\t'),
            ('\\', Some('\\')) => out.push('\\'),
            _ => {
                out.push(c);
                continue;
            }
        }

        chars.next();
    }

    out
}

/// 已编译的查询，文件内查找与全局搜索共用同一套匹配规则。
#[derive(Debug, Clone)]
pub(crate) struct Matcher {
    pub(super) kind: Kind,
    whole_word: bool,
    /// 是否为正则模式；为 `true` 时替换文字中的 `$1`、`\n` 需要展开。
    ///
    /// 不区分大小写的纯文本虽然也编译为正则，但其替换文字按字面处理。
    regex: bool,
    /// 是否跨行查找（见 `is_multiline`）：在整篇文本上匹配，不逐行匹配。
    multiline: bool,
    /// 在多行文本块上预先查找候选位置的正则（见 [`Self::block_find`]）。
    ///
    /// `^`、`$` 按行首、行尾匹配，换行识别 `\n`、`\r\n`、`\r`。查询含 `\A`、`\z`（无法放宽），
    /// 或使用 `Kind::Literal`、跨行查找时为 `None`。查询用到环视、反向引用时，这里是
    /// `relaxed` 放宽后的版本；调用方给出必含的文字时（见 [`Self::requiring`]），这里只找这段文字。
    pub(super) block: Option<Regex>,
    /// 是否有匹配因回溯超出 `fancy-regex` 的上限而放弃：放弃的那一处之后，同一行（跨行查找时为
    /// 整篇文本）不再查找，结果不完整。
    ///
    /// 各副本共用（并行搜索的每个线程一个副本）；由 [`Self::take_gave_up`] 读出并清除。
    gave_up: Arc<AtomicBool>,
}

/// 查询的匹配方式。
#[derive(Debug, Clone)]
pub(super) enum Kind {
    /// 区分大小写的纯文本，按字节直接查找。
    ///
    /// 放在堆上：`x86_64` 上的查找器包含几种 SIMD 实现，有近 300 字节，比正则大得多。
    Literal(Box<Finder<'static>>),
    /// 正则，以及不区分大小写的纯文本（转义后编译为正则）。
    ///
    /// 大小写按 Unicode 简单大小写折叠处理：Σ、σ、ς 视为同一字母，开尔文符号（U+212A）视为 `k`。
    /// 纯文本与正则的折叠规则相同。
    Regex(Pattern),
}

/// 正则匹配因回溯过多而放弃，见 [`Pattern::find_at`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GaveUp;

/// 一处匹配中各分组的字节范围：第 0 项是整个匹配，没有参与匹配的分组为 `None`。
pub(crate) type Groups = Vec<Option<(usize, usize)>>;

/// 用户查询对应的正则，按语法选择实现。
///
/// `regex` 不支持的写法使用 `fancy-regex` 编译（支持回溯），包括环视（`(?=…)`、`(?<=…)`、`(?!…)`、
/// `(?<!…)`）、反向引用（`\1`）和原子组；其余写法使用 `regex`，其匹配时间与输入长度成线性关系。
#[derive(Debug, Clone)]
pub(crate) enum Pattern {
    /// 由 `regex` 编译的正则。
    Plain(Regex),
    /// 由 `fancy-regex` 编译的正则。
    Fancy(fancy_regex::Regex),
}

impl Pattern {
    /// 编译 `pattern`，优先使用 `regex`，其不支持时回退到 `fancy-regex`。
    ///
    /// # Errors
    ///
    /// 两种实现都无法编译时返回错误说明。
    pub(crate) fn new(
        pattern: &str,
        case_insensitive: bool,
        multi_line: bool,
    ) -> Result<Pattern, String> {
        let error = match RegexBuilder::new(pattern)
            .case_insensitive(case_insensitive)
            .multi_line(multi_line)
            .build()
        {
            Ok(regex) => return Ok(Pattern::Plain(regex)),
            Err(error) => error.to_string(),
        };
        match fancy_regex::RegexBuilder::new(pattern)
            .case_insensitive(case_insensitive)
            .multi_line(multi_line)
            .build()
        {
            Ok(regex) => Ok(Pattern::Fancy(regex)),
            // 两种实现都编译失败。`regex` 仅因写法不受支持而失败时，报告 `fancy-regex` 的错误
            // （`fancy-regex` 支持环视和反向引用）；否则属于语法错误，报告 `regex` 的错误，
            // 其位置更准确。
            Err(fancy) if error.contains("not supported") => Err(fancy.to_string()),
            Err(_) => Err(error),
        }
    }

    /// 返回从字节偏移 `from` 起的第一个匹配（字节范围）；`from` 之前的文字仍参与环视判断。
    ///
    /// # Errors
    ///
    /// 回溯超过 `fancy-regex` 的上限（默认 100 万次）或回溯栈溢出时返回 [`GaveUp`]：放弃这一处，
    /// 不继续回溯，避免模式导致长时间阻塞。调用方据此告知用户结果不完整。
    pub(crate) fn find_at(
        &self,
        text: &str,
        from: usize,
    ) -> Result<Option<(usize, usize)>, GaveUp> {
        match self {
            Pattern::Plain(regex) => Ok(regex.find_at(text, from).map(|m| (m.start(), m.end()))),
            Pattern::Fancy(regex) => match regex.find_from_pos(text, from) {
                Ok(found) => Ok(found.map(|m| (m.start(), m.end()))),
                Err(_) => Err(GaveUp),
            },
        }
    }

    /// 返回从字节偏移 `from` 起的第一个匹配及其各分组的字节范围；第 0 项是整个匹配，没有
    /// 参与匹配的分组为 `None`。
    ///
    /// # Errors
    ///
    /// 回溯超过上限时返回 [`GaveUp`]，同 [`Pattern::find_at`]。
    pub(crate) fn captures_at(&self, text: &str, from: usize) -> Result<Option<Groups>, GaveUp> {
        match self {
            Pattern::Plain(regex) => Ok(regex.captures_at(text, from).map(|captures| {
                (captures.iter())
                    .map(|group| group.map(|m| (m.start(), m.end())))
                    .collect()
            })),
            Pattern::Fancy(regex) => match regex.captures_from_pos(text, from) {
                Ok(found) => Ok(found.map(|captures| {
                    (captures.iter())
                        .map(|group| group.map(|m| (m.start(), m.end())))
                        .collect()
                })),
                Err(_) => Err(GaveUp),
            },
        }
    }

    /// 返回恰好位于 `range`（字节范围）的匹配按 `template` 展开分组引用后的文字。
    ///
    /// 该位置已不是匹配时返回 `None`。
    pub(crate) fn expand_at(
        &self,
        text: &str,
        range: (usize, usize),
        template: &str,
    ) -> Option<String> {
        let mut out = String::new();
        match self {
            Pattern::Plain(regex) => {
                let captures = regex.captures_at(text, range.0)?;
                let whole = captures.get(0)?;
                ((whole.start(), whole.end()) == range)
                    .then(|| captures.expand(template, &mut out))?;
            }
            Pattern::Fancy(regex) => {
                let captures = regex.captures_from_pos(text, range.0).ok()??;
                let whole = captures.get(0)?;
                ((whole.start(), whole.end()) == range)
                    .then(|| captures.expand(template, &mut out))?;
            }
        }
        Some(out)
    }

    /// 返回 `text` 中第一处匹配按 `template` 展开后的文字，用于结果列表显示「替换后」；
    /// 没有匹配时返回 `None`。
    pub(crate) fn expand_first(&self, text: &str, template: &str) -> Option<String> {
        let range = self.find_at(text, 0).ok()??;
        self.expand_at(text, range, template)
    }
}

/// 跨行查找中的一处匹配。
#[derive(Debug, Clone, Copy)]
pub(crate) struct JoinedMatch {
    /// 匹配起点所在的行号（从 0 起）。
    pub line: usize,
    /// 匹配终点所在的行号（从 0 起，含）；终点恰好在换行之后时不计入下一行。
    pub last: usize,
    /// 第 `line` 行在连接文本中的起始字节偏移。
    pub line_byte: usize,
    /// 匹配在连接文本中的字节范围。
    pub bytes: (usize, usize),
    /// 匹配在文档中的位置范围（字符偏移；换行按文档中的实际写法计算）。
    pub range: (usize, usize),
}

impl Matcher {
    /// 编译查询。
    ///
    /// # Errors
    ///
    /// 正则语法错误或写法不被支持时返回错误说明。
    pub(crate) fn new(query: &str, options: Options) -> Result<Self, String> {
        Self::requiring(query, options, "")
    }

    /// 编译查询，`required` 是每处匹配都含有的一段文字，大小写规则与查询相同；为空时没有这项信息。
    ///
    /// 由多种写法合并成的长正则没有可供预筛选的公共前缀，在整块文本上只能逐字节运行状态机。有
    /// 必含的文字时，只有含这段文字的行才逐行匹配，其余内容按纯文字跳过。
    ///
    /// # Errors
    ///
    /// 正则语法错误或写法不被支持时返回错误说明。
    pub(crate) fn requiring(query: &str, options: Options, required: &str) -> Result<Self, String> {
        let multiline = is_multiline(query, options.regex);
        let mut block = None;
        let kind = if options.case_sensitive && !options.regex && !multiline {
            Kind::Literal(Box::new(Finder::new(query.as_bytes()).into_owned()))
        } else {
            let escaped;
            let pattern = if options.regex {
                query
            } else {
                escaped = regex::escape(query);
                &escaped
            };
            // 跨行查找时 `^`、`$` 仍按行首、行尾匹配（`multi_line` 标志）。
            let regex = Pattern::new(pattern, !options.case_sensitive, multiline)?;
            let plain = match regex {
                _ if !required.is_empty() => Some(Cow::Owned(regex::escape(required))),
                Pattern::Plain(_) if !pattern.contains("\\A") && !pattern.contains("\\z") => {
                    Some(Cow::Borrowed(pattern))
                }
                Pattern::Plain(_) => None,
                Pattern::Fancy(_) => relaxed(pattern).map(Cow::Owned),
            };
            if !multiline && let Some(plain) = plain {
                block = RegexBuilder::new(&plain)
                    .case_insensitive(!options.case_sensitive)
                    .multi_line(true)
                    .crlf(true)
                    .build()
                    .ok();
            }
            Kind::Regex(regex)
        };

        Ok(Self {
            kind,
            whole_word: options.whole_word,
            regex: options.regex,
            multiline,
            block,
            gave_up: Arc::default(),
        })
    }

    /// 是否可以用 [`Self::block_find`] 在整块文本上预先查找候选位置。
    pub(crate) fn has_block(&self) -> bool {
        matches!(self.kind, Kind::Literal(_)) || self.block.is_some()
    }

    /// 在由多行连接成的 `text` 中，返回从字节偏移 `from` 起第一个候选匹配的字节偏移。
    ///
    /// 单行内的匹配在整块文本的同一行上也有候选（`^`、`$` 按行匹配，`.` 不跨换行；用到环视、
    /// 反向引用时使用放宽后的正则，有必含的文字时找这段文字），因此第一个候选之前的行一定没有
    /// 匹配；候选所在的行则不一定有匹配。没有候选，或 [`Self::has_block`] 为 `false` 时返回
    /// `None`。
    pub(crate) fn block_find(&self, text: &str, from: usize) -> Option<usize> {
        match (&self.kind, &self.block) {
            (Kind::Literal(finder), _) => finder.find(&text.as_bytes()[from..]).map(|at| from + at),
            (Kind::Regex(_), Some(block)) => block.find_at(text, from).map(|m| m.start()),
            (Kind::Regex(_), None) => None,
        }
    }

    /// 是否有匹配因回溯过多而放弃（自上次 [`Self::take_gave_up`] 以来），不清除标记。
    pub(crate) fn gave_up(&self) -> bool {
        self.gave_up.load(Ordering::Relaxed)
    }

    /// 返回上次调用以来是否有匹配因回溯过多而放弃，并清除该标记。
    pub(crate) fn take_gave_up(&self) -> bool {
        self.gave_up.swap(false, Ordering::Relaxed)
    }

    /// 是否跨行查找。
    pub(crate) fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// 返回正则模式下的 [`Pattern`]，替换时用该正则展开分组引用；非正则模式返回 `None`。
    pub(super) fn template_regex(&self) -> Option<&Pattern> {
        match &self.kind {
            Kind::Regex(regex) if self.regex => Some(regex),
            _ => None,
        }
    }

    /// 在一行里依次查找所有不重叠的匹配，把每处匹配在 `line` 中的字节范围交给 `f`；`f` 返回
    /// `Break` 时停止并返回 `Break`。
    ///
    /// 空匹配和不满足全字条件的匹配不算。
    pub(crate) fn find_all(
        &self,
        line: &str,
        mut f: impl FnMut(usize, usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        // 用到环视、反向引用时先用放宽后的正则判断：该正则不匹配时该行一定没有匹配，可以跳过回溯。
        if let (Kind::Regex(Pattern::Fancy(_)), Some(block)) = (&self.kind, &self.block)
            && !block.is_match(line)
        {
            return ControlFlow::Continue(());
        }

        let bytes = line.as_bytes();
        let mut from = 0;

        while from <= bytes.len() {
            let Some((s, e)) = self.find(line, from) else {
                break;
            };

            // 跳过空匹配和不满足全字条件的匹配，从下一个字符继续查找；例如在「foofoo foo」中
            // 仍能找到最后的 `foo`。
            if s == e || (self.whole_word && !is_whole_word(bytes, s, e)) {
                from = s + char_width(bytes, s);
                continue;
            }

            f(s, e)?;
            from = e;
        }

        ControlFlow::Continue(())
    }

    /// 在一行里查找所有不重叠的匹配，位置记入 `matches`，行号记入 `lines`。
    ///
    /// `index` 是行号（从 0 起），`start` 是该行行首的位置（字符偏移）。`matches` 的长度达到
    /// `limit` 时返回 `Break`。
    pub(super) fn scan(
        &self,
        index: usize,
        start: usize,
        line: &str,
        matches: &mut Vec<(usize, usize)>,
        lines: &mut Vec<usize>,
        limit: usize,
    ) -> ControlFlow<()> {
        let bytes = line.as_bytes();
        // 查找得到的是字节偏移，增量换算为字符偏移：`byte` 处的位置是 `pos`。
        let (mut byte, mut pos) = (0, start);

        self.find_all(line, |s, e| {
            let from = pos + count_chars(&bytes[byte..s]);
            (byte, pos) = (e, from + count_chars(&bytes[s..e]));
            matches.push((from, pos));

            if lines.last() != Some(&index) {
                lines.push(index);
            }

            if matches.len() >= limit {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
    }

    /// 跨行查找：在 `joined`（见 [`joined`]）中依次查找，每处匹配交给 `f`，`f` 返回 `Break`
    /// 时停止。
    ///
    /// `starts[i]` 是第 `i` 行行首在文档中的位置（字符偏移）。
    pub(crate) fn scan_joined(
        &self,
        joined: &str,
        starts: &[usize],
        mut f: impl FnMut(JoinedMatch) -> ControlFlow<()>,
    ) {
        let bytes = joined.as_bytes();
        let mut cursor = Cursor::default();
        let mut from = 0;

        while from <= bytes.len() {
            let Some((s, e)) = self.find(joined, from) else {
                break;
            };
            if s == e || (self.whole_word && !is_whole_word(bytes, s, e)) {
                from = s + char_width(bytes, s);
                continue;
            }

            let (line, line_byte, col) = cursor.seek(bytes, s);
            let (end_line, _, end_col) = cursor.seek(bytes, e);
            // 匹配终点恰好在换行之后（列 0）时，不计入终点所在的下一行。
            let last = if end_col == 0 && end_line > line {
                end_line - 1
            } else {
                end_line
            };
            let found = JoinedMatch {
                line,
                last,
                line_byte,
                bytes: (s, e),
                range: (starts[line] + col, starts[end_line] + end_col),
            };
            if f(found).is_break() {
                return;
            }
            from = e;
        }
    }

    /// 返回从字节偏移 `from` 起的第一个匹配的字节范围。
    fn find(&self, text: &str, from: usize) -> Option<(usize, usize)> {
        match &self.kind {
            Kind::Literal(finder) => finder
                .find(&text.as_bytes()[from..])
                .map(|at| (from + at, from + at + finder.needle().len())),
            Kind::Regex(regex) => regex.find_at(text, from).unwrap_or_else(|GaveUp| {
                self.gave_up.store(true, Ordering::Relaxed);
                None
            }),
        }
    }
}

/// 把用到环视、反向引用的正则放宽为 `regex` 能编译的版本，供整块文本预先查找候选使用
/// （见 [`Matcher::block_find`]）。
///
/// 放宽后的正则只会多匹配、不会少匹配：环视和 `\b` 等断言被移除（少一个条件只会多匹配），
/// 反向引用换成当前行剩余的任意字符，原子组按普通组处理。条件分支、子程序调用无法放宽，
/// 此时返回 `None`，只能逐行查找。
pub(super) fn relaxed(pattern: &str) -> Option<String> {
    use fancy_regex::{Assertion, AstNode, Expr};

    fn write(expr: &Expr, out: &mut String) -> Option<()> {
        match expr {
            Expr::Empty
            | Expr::LookAround(..)
            | Expr::KeepOut
            | Expr::ContinueFromPreviousMatchEnd => out.push_str("(?:)"),
            // 保留行首、行尾断言（`\A`、`^` 都写成 `^`；整块文本按行匹配，语义恰好对应），
            // 其余断言移除。
            Expr::Assertion(
                Assertion::StartText
                | Assertion::EndText
                | Assertion::StartLine { .. }
                | Assertion::EndLine { .. },
            )
            | Expr::Literal { .. }
            | Expr::Any { .. }
            | Expr::Delegate { .. } => {
                out.push_str("(?:");
                expr.to_str(out, 0);
                out.push(')');
            }
            Expr::Assertion(_) => out.push_str("(?:)"),
            Expr::Backref { .. }
            | Expr::BackrefWithRelativeRecursionLevel { .. }
            | Expr::AstNode(AstNode::Backref { .. }, _) => out.push_str("[^\n]*"),
            Expr::Concat(children) => {
                out.push_str("(?:");
                for child in children {
                    write(child, out)?;
                }
                out.push(')');
            }
            Expr::Alt(children) => {
                out.push_str("(?:");
                for (i, child) in children.iter().enumerate() {
                    if i > 0 {
                        out.push('|');
                    }
                    write(child, out)?;
                }
                out.push(')');
            }
            Expr::Group(child) => write(child, out)?,
            Expr::AtomicGroup(child) | Expr::AstNode(AstNode::AstGroup { inner: child, .. }, _) => {
                write(child, out)?
            }
            Expr::Repeat { child, lo, hi, .. } => {
                out.push_str("(?:");
                write(child, out)?;
                out.push(')');
                match *hi {
                    usize::MAX => out.push_str(&format!("{{{lo},}}")),
                    hi => out.push_str(&format!("{{{lo},{hi}}}")),
                }
            }
            _ => return None,
        }
        Some(())
    }

    let tree = Expr::parse_tree(pattern).ok()?;
    let mut out = String::new();
    write(&tree.expr, &mut out)?;
    Some(out)
}

/// 是否需要跨行查找：查询含换行符，或正则模式下含 `\n`、`\r`、`\W`（`\W` 也能匹配换行符）。
pub(super) fn is_multiline(query: &str, regex: bool) -> bool {
    if query.contains('\n') {
        return true;
    }
    let mut chars = query.chars();
    while let Some(c) = chars.next() {
        if regex && c == '\\' && matches!(chars.next(), Some('n' | 'r' | 'W')) {
            return true;
        }
    }
    false
}

/// 把整篇文本的各行用 `\n` 连接，返回连接结果和每行行首的位置（字符偏移）。
///
/// 文档中的 `\r\n`、`\r` 在连接结果里都是 `\n`，使查询中的 `\n` 能匹配任意一种换行；
/// 匹配结果再按行首位置换算回文档位置。供跨行查找使用。
pub(crate) fn joined(text: &impl Lines) -> (String, Vec<usize>) {
    let (mut joined, mut starts) = (String::new(), Vec::new());
    text.visit_lines(0, usize::MAX, |_, start, line| {
        if !starts.is_empty() {
            joined.push('\n');
        }
        joined.push_str(line);
        starts.push(start);
        ControlFlow::Continue(())
    });
    (joined, starts)
}

/// 在连接文本中自前向后移动，同时统计当前行号和行内的字符数（列号）。
#[derive(Default)]
struct Cursor {
    byte: usize,
    line: usize,
    line_byte: usize,
    col: usize,
}

impl Cursor {
    /// 向后移动到字节偏移 `to`，返回所在行的行号、该行起始字节偏移和列号。
    ///
    /// 行号与列号从 0 起，列号按字符计。
    fn seek(&mut self, bytes: &[u8], to: usize) -> (usize, usize, usize) {
        while let Some(at) = memchr::memchr(b'\n', &bytes[self.byte..to]) {
            self.byte += at + 1;
            self.line += 1;
            self.line_byte = self.byte;
            self.col = 0;
        }
        self.col += count_chars(&bytes[self.byte..to]);
        self.byte = to;
        (self.line, self.line_byte, self.col)
    }
}

/// 返回 UTF-8 字节串中的字符数（个）。
pub(crate) fn count_chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b as i8) >= -0x40).count()
}

/// 按首字节判断位置 `i` 处 UTF-8 字符的字节数（字节）。
///
/// `i` 位于末尾或之后时返回 1，使调用方的循环能越过末尾而终止。
fn char_width(bytes: &[u8], i: usize) -> usize {
    match bytes.get(i) {
        Some(0xF0..) => 4,
        Some(0xE0..) => 3,
        Some(0xC0..) => 2,
        _ => 1,
    }
}

fn char_after(bytes: &[u8], i: usize) -> Option<char> {
    let end = (i + char_width(bytes, i)).min(bytes.len());
    std::str::from_utf8(bytes.get(i..end)?).ok()?.chars().next()
}

fn char_before(bytes: &[u8], i: usize) -> Option<char> {
    let start = bytes[..i].iter().rposition(|&b| (b as i8) >= -0x40)?;
    std::str::from_utf8(&bytes[start..i]).ok()?.chars().next()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 是否位置范围 `s` 到 `e`（字节偏移，左闭右开）构成一个整词。
///
/// 匹配的某一端本身不是单词字符时（例如查询「->」），该端不检查。
fn is_whole_word(bytes: &[u8], s: usize, e: usize) -> bool {
    let separated = |outside: Option<char>, inside: Option<char>| {
        !outside.is_some_and(is_word_char) || !inside.is_some_and(is_word_char)
    };

    separated(char_before(bytes, s), char_after(bytes, s))
        && separated(char_after(bytes, e), char_before(bytes, e))
}
