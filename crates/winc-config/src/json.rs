//! 最小的 JSON 读写：用于配置和状态文件，以及格式化时读取项目里的 `package.json`。
//!
//! 只实现解析、序列化和读取，不做校验。解析失败一律返回 `None`，不报告位置：
//! 调用方都按「无法理解就视为没写」处理。字符串转义支持 `\"`、`\\`、`\/`、`\b`、`\f`、`\n`、`\r`、
//! `\t` 和 `\uXXXX`（含代理对）。用户写的配置按 JSONC 读（[`Json::parse_jsonc`]）。

use std::collections::BTreeMap;

/// JSON 值：`null`、布尔、数字（统一用 `f64`）、字符串、数组、对象。
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// 空值。
    Null,
    /// 布尔值。
    Bool(bool),
    /// 数字，统一用 `f64` 表示。
    Number(f64),
    /// 字符串。
    String(String),
    /// 数组。
    Array(Vec<Json>),
    /// 对象，键按字典序排列（[`BTreeMap`]）。
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// 解析 JSON 文本；无法解析时返回 `None`，不报告具体位置。
    pub fn parse(text: &str) -> Option<Json> {
        Parser::new(text, false).document()
    }

    /// 解析 JSONC 文本，返回值和其中是否有注释；无法解析时返回 `None`。
    ///
    /// JSONC 在 JSON 之外允许开头的 BOM、`//` 行注释和块注释，以及对象和数组最后一项后面的逗号。
    /// 用户手写的配置按 JSONC 解析。序列化不保留注释，调用方据第二个返回值决定写回前是否另存。
    pub fn parse_jsonc(text: &str) -> Option<(Json, bool)> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut parser = Parser::new(text, true);
        let value = parser.document()?;
        Some((value, parser.comments))
    }

    /// 是字符串时返回其中的文本，否则返回 `None`。
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    /// 是布尔值时返回其值，否则返回 `None`。
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// 是数字时返回其值，否则返回 `None`。
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// 是数组时返回其中的元素，否则返回 `None`。
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// 把键值对转成对象；键重复时后出现的值覆盖先出现的。
    pub(crate) fn object<K: Into<String>>(items: impl IntoIterator<Item = (K, Json)>) -> Json {
        Json::Object(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// 是对象时返回键值表，否则返回 `None`。
    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Object(map) => Some(map),
            _ => None,
        }
    }

    /// 返回多行缩进的 JSON 文本，缩进到 `indent` 层（每层 2 个空格）。
    ///
    /// 空数组和空对象写成 `[]`、`{}`。
    pub(crate) fn write(&self, indent: usize) -> String {
        let mut out = String::new();
        self.write_into(&mut out, indent);
        out
    }

    /// 同 [`Json::write`]，写到 `out` 末尾。
    ///
    /// 整棵树共用一个缓冲区，各层不必各自拼出字符串再复制一遍。
    pub(crate) fn write_into(&self, out: &mut String, indent: usize) {
        match self {
            Json::Array(items) => write_block(out, indent, ['[', ']'], items, |out, item| {
                item.write_into(out, indent + 1)
            }),
            Json::Object(map) => {
                let pairs = map.iter().map(|(k, v)| (k.as_str(), v));
                write_object(out, indent, pairs, |out, v, indent| {
                    v.write_into(out, indent)
                });
            }
            scalar => scalar.write_scalar(out),
        }
    }

    /// 返回单行 JSON（无换行和缩进），用于需要按行分隔的场合，例如备份文件的首行。
    pub fn compact(&self) -> String {
        let mut out = String::new();
        self.compact_into(&mut out);
        out
    }

    /// 同 [`Json::compact`]，写到 `out` 末尾。
    fn compact_into(&self, out: &mut String) {
        match self {
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.compact_into(out);
                }
                out.push(']');
            }
            Json::Object(map) => {
                out.push('{');
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote_into(out, key);
                    out.push(':');
                    value.compact_into(out);
                }
                out.push('}');
            }
            scalar => scalar.write_scalar(out),
        }
    }

    /// 写出标量值（`null`、布尔、数字或字符串）的 JSON 文本；数组和对象由调用方处理。
    fn write_scalar(&self, out: &mut String) {
        use std::fmt::Write;

        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            // JSON 不能表示 `NaN` 和无穷大，写出后整个文件无法解析，因此写成 `null`。
            Json::Number(n) if !n.is_finite() => out.push_str("null"),
            // 整数值不写成 `14.0`：绝对值小于 1e15 时转为 `i64` 输出，其余按浮点写法。
            Json::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => {
                let _ = write!(out, "{}", *n as i64);
            }
            Json::Number(n) => {
                let _ = write!(out, "{n}");
            }
            Json::String(text) => quote_into(out, text),
            Json::Array(_) | Json::Object(_) => unreachable!("数组和对象不是标量"),
        }
    }
}

/// 把一组键值写成对象，每项一行，缩进到 `indent` 层，键的顺序与 `pairs` 一致；`pairs` 为空时写成
/// `{}`。
///
/// 值由 `value` 写出，传给它的缩进为 `indent + 1`。值不必是 [`Json`]：配置文件按固定顺序写出各节，
/// 而 [`Json::Object`] 总按键的字典序排列。
pub(crate) fn write_object<'k, V>(
    out: &mut String,
    indent: usize,
    pairs: impl IntoIterator<Item = (&'k str, V)>,
    mut value: impl FnMut(&mut String, V, usize),
) {
    write_block(out, indent, ['{', '}'], pairs, |out, (key, v)| {
        quote_into(out, key);
        out.push_str(": ");
        value(out, v, indent + 1);
    });
}

/// 写出多行的数组或对象：`brackets` 是首尾的括号，每项单独一行，比括号深一层缩进；没有项时写成
/// `[]`、`{}`。
fn write_block<T>(
    out: &mut String,
    indent: usize,
    brackets: [char; 2],
    items: impl IntoIterator<Item = T>,
    mut item: impl FnMut(&mut String, T),
) {
    out.push(brackets[0]);
    let mut empty = true;
    for value in items {
        out.push_str(if empty { "\n" } else { ",\n" });
        empty = false;
        push_pad(out, indent + 1);
        item(out, value);
    }
    if !empty {
        out.push('\n');
        push_pad(out, indent);
    }
    out.push(brackets[1]);
}

/// 向 `out` 末尾追加 `indent` 层缩进，每层 2 个空格。
fn push_pad(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// 把字符串写成带引号的 JSON 字面量，转义 `"`、`\` 和控制字符，其他字符（含中文、emoji）原样保留。
fn quote_into(out: &mut String, text: &str) {
    use std::fmt::Write;

    out.reserve(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// 解析允许的最大嵌套层数（层）。
///
/// 超过时按格式错误处理：解析是递归的，一长串 `[` 会耗尽调用栈，导致进程中止，使未保存的内容丢失。
/// `.winc/config.json` 和 `package.json` 来自项目仓库，内容不可信。该值是经验值，
/// 高于常见配置文件的嵌套深度。
const MAX_DEPTH: usize = 256;

/// 递归下降的 JSON 解析器。
struct Parser<'a> {
    /// 待解析的 UTF-8 文本。
    bytes: &'a [u8],
    /// 当前读取位置（字节偏移）。
    pos: usize,
    /// 当前嵌套层数（层），用于限制递归深度。
    depth: usize,
    /// 是否按 JSONC 解析：允许注释和末尾的逗号（见 [`Json::parse_jsonc`]）。
    jsonc: bool,
    /// 是否读到过注释。
    comments: bool,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, jsonc: bool) -> Self {
        Parser {
            bytes: text.as_bytes(),
            pos: 0,
            depth: 0,
            jsonc,
            comments: false,
        }
    }

    /// 解析整个文本：一个值，前后只能有空白（JSONC 还可以有注释）。
    fn document(&mut self) -> Option<Json> {
        self.skip_whitespace();
        let value = self.value()?;
        self.skip_whitespace();

        // 后面还有别的内容说明文件里有多个顶层值或注释，按格式错误处理。
        (self.pos == self.bytes.len()).then_some(value)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// 跳过空白；按 JSONC 解析时连同注释一起跳过。
    ///
    /// 没有结束的块注释不跳过，由之后的解析报错。
    fn skip_whitespace(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.pos += 1;
            }
            let rest = &self.bytes[self.pos..];
            let end = if !self.jsonc {
                None
            } else if rest.starts_with(b"//") {
                Some(rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len()))
            } else if rest.starts_with(b"/*") {
                rest[2..].windows(2).position(|w| w == b"*/").map(|i| i + 4)
            } else {
                None
            };
            let Some(end) = end else {
                return;
            };
            self.pos += end;
            self.comments = true;
        }
    }

    /// 在逗号之后调用：按 JSONC 解析且后面紧跟结束符 `close`（`}` 或 `]`）时读取结束符并返回真。
    fn trailing_comma(&mut self, close: u8) -> bool {
        self.skip_whitespace();
        let trailing = self.jsonc && self.peek() == Some(close);
        if trailing {
            self.pos += 1;
        }
        trailing
    }

    /// 解析一个值；嵌套层数超过 [`MAX_DEPTH`] 时返回 `None`。
    fn value(&mut self) -> Option<Json> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let value = self.value_inner();
        self.depth -= 1;
        value
    }

    fn value_inner(&mut self) -> Option<Json> {
        self.skip_whitespace();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::String),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Option<Json> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            return Some(value);
        }
        None
    }

    fn object(&mut self) -> Option<Json> {
        self.pos += 1; // '{'
        let mut map = BTreeMap::new();
        self.skip_whitespace();

        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Some(Json::Object(map));
        }

        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return None;
            }
            self.pos += 1;
            let value = self.value()?;
            map.insert(key, value);

            self.skip_whitespace();
            match self.peek()? {
                b',' => {
                    self.pos += 1;
                    if self.trailing_comma(b'}') {
                        return Some(Json::Object(map));
                    }
                }
                b'}' => {
                    self.pos += 1;
                    return Some(Json::Object(map));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self) -> Option<Json> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_whitespace();

        if self.peek() == Some(b']') {
            self.pos += 1;
            return Some(Json::Array(items));
        }

        loop {
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => {
                    self.pos += 1;
                    if self.trailing_comma(b']') {
                        return Some(Json::Array(items));
                    }
                }
                b']' => {
                    self.pos += 1;
                    return Some(Json::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.peek() != Some(b'"') {
            return None;
        }
        self.pos += 1;

        let mut out = String::new();
        loop {
            let byte = self.peek()?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let escape = self.peek()?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return None,
                    }
                }
                // 连续复制到下一个引号或反斜杠为止。这两个字符都是 ASCII，输入又来自 `&str`，
                // 因此切分处一定在字符边界上，`from_utf8` 不会失败。
                _ => {
                    let rest = &self.bytes[self.pos..];
                    let len = rest
                        .iter()
                        .position(|&b| b == b'"' || b == b'\\')
                        .unwrap_or(rest.len());
                    out.push_str(std::str::from_utf8(&rest[..len]).ok()?);
                    self.pos += len;
                }
            }
        }
    }

    /// 解析 `\uXXXX` 转义，调用时已读过 `\u`；遇到高代理时继续读取后面的低代理，合成一个字符。
    fn unicode_escape(&mut self) -> Option<char> {
        let first = self.hex4()?;

        // 高代理（U+D800 至 U+DBFF）之后必须跟一个低代理转义（`\uDC00` 至 `\uDFFF`）。
        if (0xd800..0xdc00).contains(&first) {
            if self.peek() == Some(b'\\') {
                self.pos += 1;
                if self.peek() == Some(b'u') {
                    self.pos += 1;
                    let second = self.hex4()?;
                    if (0xdc00..0xe000).contains(&second) {
                        let code = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                        return char::from_u32(code);
                    }
                }
            }
            return None;
        }

        char::from_u32(first)
    }

    /// 读取 4 位十六进制数并前进 4 字节，不足 4 位时返回 `None`。
    fn hex4(&mut self) -> Option<u32> {
        let slice = self.bytes.get(self.pos..self.pos + 4)?;
        // `from_str_radix` 还接受前导 `+`，而 JSON 只允许十六进制数字，因此先逐字节检查。
        if !slice.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let text = std::str::from_utf8(slice).ok()?;
        let value = u32::from_str_radix(text, 16).ok()?;
        self.pos += 4;
        Some(value)
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.pos;

        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        let text = std::str::from_utf8(self.bytes.get(start..self.pos)?).ok()?;
        text.parse().ok().map(Json::Number)
    }
}

#[cfg(test)]
mod tests;
