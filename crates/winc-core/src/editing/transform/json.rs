//! 美化与压缩 JSON：按 RFC 8259 校验，只改动各部分之间的空白。
//!
//! 字符串、数字和字面量与原文逐字节相同，键的顺序和重复的键都保留。转换的范围是选区；
//! 没有选区时是整个文件（文件是一个 JSON 值时），否则是光标所在行中包住光标的 JSON。

use crate::editing::edit::{Change, EditKind, Transaction};
use crate::editing::selection::Selection;
use crate::text::Lines;

/// 嵌套的层数上限。
///
/// 有约束：美化后第 n 层的行缩进 n 级，缩进的总长度随层数平方增长。
/// 512 层、每级 4 个空格时，缩进约 1 MB。
pub const JSON_MAX_DEPTH: usize = 512;

/// JSON 的排版方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonLayout {
    /// 多行排版：每个成员单独一行，按层级缩进。
    Pretty,
    /// 单行排版：去掉全部空白。
    Compact,
}

/// 无法转换 JSON 的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonError {
    /// 范围内没有 JSON：选区中只有空白，或光标所在行中没有包住光标的 JSON。
    NotFound,
    /// 不符合 JSON 语法的内容，参数是其位置（字符偏移）。
    Invalid(usize),
    /// 不完整的 JSON：内容在一个完整的值之前结束。
    Incomplete,
    /// 过深的嵌套：超过 [`JSON_MAX_DEPTH`] 层。
    TooDeep,
}

/// 返回把 `selection` 范围内的 JSON 按 `layout` 重新排版的事务；内容已是该排版时返回 `Ok(None)`。
///
/// 美化时每级缩进 `indent_unit`，换行用 `line_ending`，后续行以 JSON 起点所在行的缩进为基准。
/// 没有选区时，`whole_file` 为真才检查整个文件。转换后选中新的内容。
///
/// # Errors
///
/// 范围内没有 JSON、不是有效的 JSON，或嵌套过深时返回 [`JsonError`]。
pub fn reformat_json(
    text: &impl Lines,
    selection: Selection,
    layout: JsonLayout,
    indent_unit: &str,
    line_ending: &str,
    whole_file: bool,
) -> Result<Option<Transaction>, JsonError> {
    let found = if selection.is_empty() {
        around_caret(
            text,
            selection.head,
            layout,
            indent_unit,
            line_ending,
            whole_file,
        )?
    } else {
        let source = text.substring(selection.from(), selection.to());
        whole(
            text,
            selection.from(),
            &source,
            layout,
            indent_unit,
            line_ending,
        )?
    };
    if found.output == found.source {
        return Ok(None);
    }
    let end = found.start + found.output.chars().count();
    let change = Change {
        start: found.start,
        removed: found.source,
        inserted: found.output,
    };
    let after = Selection::new(found.start, end);
    Ok(Some(Transaction::new(
        vec![change],
        selection,
        after,
        EditKind::Other,
    )))
}

/// 已排版的一段 JSON 及其原文。
struct Found {
    /// 起点（字符偏移）。
    start: usize,
    /// 原文。
    source: String,
    /// 排版后的文字。
    output: String,
}

/// 把从 `start`（字符偏移）起的文字 `source` 整体当作一个 JSON 值排版，两端的空白不计入。
fn whole(
    text: &impl Lines,
    start: usize,
    source: &str,
    layout: JsonLayout,
    indent_unit: &str,
    line_ending: &str,
) -> Result<Found, JsonError> {
    let body = source.trim_start_matches(is_space);
    let lead = source.len() - body.len();
    let body = body.trim_end_matches(is_space);
    if body.is_empty() {
        return Err(JsonError::NotFound);
    }
    let start = start + source[..lead].chars().count();
    let style = Style::new(text, start, layout, indent_unit, line_ending);
    let to_pos = |at: usize| start + body[..at].chars().count();
    let (output, end) = reformat(body, &style).map_err(|fault| fault.at(to_pos))?;
    if end < body.len() {
        // 一个值之后还有空白以外的内容。
        let extra = end + body[end..].len() - body[end..].trim_start_matches(is_space).len();
        return Err(JsonError::Invalid(to_pos(extra)));
    }
    Ok(Found {
        start,
        source: body.to_owned(),
        output,
    })
}

/// 找到光标 `caret` 处的 JSON 并排版。
///
/// `whole_file` 为真且整个文件是一个 JSON 值时，取整个文件；
/// 否则取光标所在行中包住光标的最外层对象或数组。
fn around_caret(
    text: &impl Lines,
    caret: usize,
    layout: JsonLayout,
    indent_unit: &str,
    line_ending: &str,
    whole_file: bool,
) -> Result<Found, JsonError> {
    let file_error = if whole_file && looks_like_json(text) {
        let source = text.substring(0, text.end_pos());
        match whole(text, 0, &source, layout, indent_unit, line_ending) {
            Ok(found) => return Ok(found),
            Err(error) => Some(error),
        }
    } else {
        None
    };

    let line = text.line_of(caret);
    let line_start = text.line_start(line);
    let source = text.substring(line_start, text.line_end(line));
    let column = caret - line_start;
    let caret_byte = source
        .char_indices()
        .nth(column)
        .map_or(source.len(), |(i, _)| i);
    let mut first_error = None;
    // 外层在光标之前的 `failed_at` 处出错时，起点更早的内层也包含该处、在该处出错，不必解析。
    let mut failed_at = 0;
    for open in enclosing_brackets(&source, caret_byte) {
        if open < failed_at {
            continue;
        }
        let start = line_start + source[..open].chars().count();
        let style = Style::new(text, start, layout, indent_unit, line_ending);
        let to_pos = |at: usize| start + source[open..open + at].chars().count();
        match reformat(&source[open..], &style) {
            Ok((output, end)) if open + end >= caret_byte => {
                return Ok(Found {
                    start,
                    source: source[open..open + end].to_owned(),
                    output,
                });
            }
            Ok(_) => {}
            Err(fault) => {
                if let Fault::Invalid(at) = fault
                    && open + at <= caret_byte
                {
                    failed_at = open + at;
                }
                first_error.get_or_insert(fault.at(to_pos));
            }
        }
    }
    Err(first_error.or(file_error).unwrap_or(JsonError::NotFound))
}

/// 整个文件是否可能是一个 JSON 对象或数组：第一个和最后一个非空白字符是成对的括号。
///
/// 只读取开头和结尾第一个含非空白字符的行，不读取整个文件。
fn looks_like_json(text: &impl Lines) -> bool {
    let lines = text.line_count();
    let edge = |line: usize, last: bool| {
        let content = text.substring(text.line_start(line), text.line_end(line));
        let content = content.trim_matches(is_space);
        if last {
            content.chars().next_back()
        } else {
            content.chars().next()
        }
    };
    let first = (0..lines).find_map(|line| edge(line, false));
    let last = (0..lines).rev().find_map(|line| edge(line, true));
    matches!(
        (first, last),
        (Some('{'), Some('}')) | (Some('['), Some(']'))
    )
}

/// 返回行 `line` 中包住字节位置 `caret` 的左括号（`{`、`[`）的字节位置，外层的在前。
///
/// 从行首扫描到光标，跳过字符串中的括号。
/// 光标正好在左括号上或紧跟在右括号之后时，该对括号也算包住光标。
fn enclosing_brackets(line: &str, caret: usize) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut open: Vec<usize> = Vec::new();
    let mut just_closed = None;
    let mut string = false;
    let mut i = 0;
    while i < caret {
        let b = bytes[i];
        if string {
            match b {
                b'\\' => i += 1,
                b'"' => string = false,
                _ => {}
            }
        } else {
            just_closed = None;
            match b {
                b'"' => string = true,
                b'{' | b'[' => open.push(i),
                b'}' | b']' => just_closed = open.pop(),
                _ => {}
            }
        }
        i += 1;
    }
    let mut found = open;
    if let Some(closed) = just_closed.filter(|_| !string) {
        found.push(closed);
    }
    if !string && matches!(bytes.get(caret), Some(b'{' | b'[')) {
        found.push(caret);
    }
    found
}

/// JSON 的空白：空格、制表符、换行和回车（RFC 8259 第 2 节）。
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// 排版方式，以及美化所用的缩进和换行。
struct Style {
    /// 美化时的格式；压缩时为 `None`。
    pretty: Option<Pretty>,
}

/// 美化所用的缩进和换行。
struct Pretty {
    /// 一级缩进的文本。
    unit: String,
    /// 换行符。
    newline: String,
    /// 第一行之后各行共同的缩进：JSON 起点所在行的缩进。
    base: String,
}

impl Style {
    /// 创建从位置 `start`（字符偏移）开始的 JSON 所用的格式。
    fn new(
        text: &impl Lines,
        start: usize,
        layout: JsonLayout,
        indent_unit: &str,
        line_ending: &str,
    ) -> Style {
        let pretty = (layout == JsonLayout::Pretty).then(|| {
            let line = text.line_of(start);
            let head = text.substring(text.line_start(line), start);
            let base = head[..head.len() - head.trim_start_matches([' ', '\t']).len()].to_owned();
            Pretty {
                unit: indent_unit.to_owned(),
                newline: line_ending.to_owned(),
                base,
            }
        });
        Style { pretty }
    }

    /// 写入换行和第 `depth` 层的缩进；压缩时不写入。
    fn line(&self, out: &mut String, depth: usize) {
        if let Some(pretty) = &self.pretty {
            out.push_str(&pretty.newline);
            out.push_str(&pretty.base);
            for _ in 0..depth {
                out.push_str(&pretty.unit);
            }
        }
    }
}

/// 不符合 JSON 语法的原因，位置是在被排版的文字中的字节偏移。
enum Fault {
    /// 不符合语法的内容，参数是其位置。
    Invalid(usize),
    /// 不完整的值：文字在一个完整的值之前结束。
    Incomplete,
    /// 过深的嵌套：超过 [`JSON_MAX_DEPTH`] 层。
    TooDeep,
}

impl Fault {
    /// 转换为 [`JsonError`]，`to_pos` 把字节偏移换成文档中的字符偏移。
    fn at(self, to_pos: impl Fn(usize) -> usize) -> JsonError {
        match self {
            Fault::Invalid(at) => JsonError::Invalid(to_pos(at)),
            Fault::Incomplete => JsonError::Incomplete,
            Fault::TooDeep => JsonError::TooDeep,
        }
    }
}

/// 下一个记号应有的种类。
#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// 任意值。
    Value,
    /// 数组的第一个元素或 `]`。
    FirstItem,
    /// 对象的第一个键或 `}`。
    FirstKey,
    /// 对象的键。
    Key,
    /// 键之后的 `:`。
    Colon,
    /// 容器中一个值之后的 `,` 或右括号。
    Next,
}

/// 按 `style` 排版 `source` 开头的一个 JSON 值，返回排版结果和该值结束处的字节位置。
///
/// 开头可以有空白；值之后的内容不检查。
fn reformat(source: &str, style: &Style) -> Result<(String, usize), Fault> {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    // 尚未闭合的容器各自的右括号。
    let mut closers: Vec<u8> = Vec::new();
    let mut expect = Expect::Value;
    let mut i = 0;
    loop {
        i += skip_space(&bytes[i..]);
        let Some(&b) = bytes.get(i) else {
            return Err(Fault::Incomplete);
        };
        let depth = closers.len();
        let close = closers.last().copied();
        match expect {
            Expect::FirstItem | Expect::FirstKey if Some(b) == close => {
                closers.pop();
                out.push(char::from(b));
                i += 1;
            }
            Expect::FirstItem | Expect::FirstKey => {
                style.line(&mut out, depth);
                expect = if expect == Expect::FirstKey {
                    Expect::Key
                } else {
                    Expect::Value
                };
                continue;
            }
            Expect::Colon if b == b':' => {
                out.push(':');
                if style.pretty.is_some() {
                    out.push(' ');
                }
                i += 1;
                expect = Expect::Value;
                continue;
            }
            Expect::Next if b == b',' => {
                out.push(',');
                style.line(&mut out, depth);
                i += 1;
                expect = if close == Some(b'}') {
                    Expect::Key
                } else {
                    Expect::Value
                };
                continue;
            }
            Expect::Next if Some(b) == close => {
                closers.pop();
                style.line(&mut out, depth - 1);
                out.push(char::from(b));
                i += 1;
            }
            Expect::Key if b == b'"' => {
                let end = i + string_len(&bytes[i..]).map_err(|at| at.shifted(i))?;
                out.push_str(&source[i..end]);
                i = end;
                expect = Expect::Colon;
                continue;
            }
            Expect::Value => match b {
                b'{' | b'[' => {
                    if depth == JSON_MAX_DEPTH {
                        return Err(Fault::TooDeep);
                    }
                    closers.push(if b == b'{' { b'}' } else { b']' });
                    out.push(char::from(b));
                    i += 1;
                    expect = if b == b'{' {
                        Expect::FirstKey
                    } else {
                        Expect::FirstItem
                    };
                    continue;
                }
                _ => {
                    let len = scalar_len(&bytes[i..]).map_err(|at| at.shifted(i))?;
                    out.push_str(&source[i..i + len]);
                    i += len;
                }
            },
            _ => return Err(Fault::Invalid(i)),
        }
        // 一个值结束：顶层的值结束时排版完成。
        if closers.is_empty() {
            return Ok((out, i));
        }
        expect = Expect::Next;
    }
}

impl Fault {
    /// 把相对于某一记号起点的位置换成相对于整段文字的位置，`offset` 是记号的起点。
    fn shifted(self, offset: usize) -> Fault {
        match self {
            Fault::Invalid(at) => Fault::Invalid(at + offset),
            other => other,
        }
    }
}

/// 返回 `bytes` 开头的空白的长度。
fn skip_space(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .count()
}

/// 返回 `bytes` 开头的字符串、数字或字面量（`true`、`false`、`null`）的长度。
///
/// # Panics
///
/// `bytes` 为空时 panic；调用方在读到下一个字节之后才调用。
fn scalar_len(bytes: &[u8]) -> Result<usize, Fault> {
    match bytes[0] {
        b'"' => string_len(bytes),
        b'-' | b'0'..=b'9' => number_len(bytes),
        _ => {
            let word: &[u8] = match bytes[0] {
                b't' => b"true",
                b'f' => b"false",
                b'n' => b"null",
                _ => return Err(Fault::Invalid(0)),
            };
            match bytes.iter().zip(word).position(|(b, w)| b != w) {
                Some(at) => Err(Fault::Invalid(at)),
                None if bytes.len() < word.len() => Err(Fault::Incomplete),
                None => Ok(word.len()),
            }
        }
    }
}

/// 返回 `bytes` 开头的字符串（含两端的引号）的长度。
///
/// 控制字符必须转义。转义只能是 `\"`、`\\`、`\/`、`\b`、`\f`、`\n`、`\r`、`\t`，
/// 或 `\u` 加四位十六进制数字（RFC 8259 第 7 节）。
fn string_len(bytes: &[u8]) -> Result<usize, Fault> {
    let mut i = 1;
    loop {
        match bytes.get(i) {
            None => return Err(Fault::Incomplete),
            Some(b'"') => return Ok(i + 1),
            Some(b'\\') => match bytes.get(i + 1) {
                None => return Err(Fault::Incomplete),
                Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => i += 2,
                Some(b'u') => {
                    for k in i + 2..i + 6 {
                        match bytes.get(k) {
                            None => return Err(Fault::Incomplete),
                            Some(b) if b.is_ascii_hexdigit() => {}
                            Some(_) => return Err(Fault::Invalid(k)),
                        }
                    }
                    i += 6;
                }
                Some(_) => return Err(Fault::Invalid(i + 1)),
            },
            Some(&b) if b < 0x20 => return Err(Fault::Invalid(i)),
            Some(_) => i += 1,
        }
    }
}

/// 返回 `bytes` 开头的数字的长度。
///
/// 数字依次是可选的负号、整数部分、可选的小数部分和指数部分（RFC 8259 第 6 节）；
/// 整数部分除单独的 0 外不以 0 开头。`bytes` 不为空，见 [`scalar_len`]。
fn number_len(bytes: &[u8]) -> Result<usize, Fault> {
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    // 位置 `at` 起至少要有一位数字，返回数字之后的位置。
    let required = |at: usize| match bytes.get(at) {
        None => Err(Fault::Incomplete),
        Some(b) if b.is_ascii_digit() => Ok(at + digits(at)),
        Some(_) => Err(Fault::Invalid(at)),
    };
    let mut i = usize::from(bytes[0] == b'-');
    i = match bytes.get(i) {
        Some(b'0') => i + 1,
        _ => required(i)?,
    };
    if bytes.get(i) == Some(&b'.') {
        i = required(i + 1)?;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        i = required(i)?;
    }
    Ok(i)
}

#[cfg(test)]
mod tests;
