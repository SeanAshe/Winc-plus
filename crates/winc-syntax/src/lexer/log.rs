//! 日志文件的词法：时间戳、级别、地址、链接、路径、字符串、键、数字、异常和调用栈。
//!
//! 每行独立分析，不带跨行状态。日志没有统一的格式，只为上述写法着色，其余文字不着色。

use super::*;

/// 日志级别（大写）和着色类别。
const LEVELS: &[(&str, TokenKind)] = &[
    ("TRACE", TokenKind::Comment),
    ("VERBOSE", TokenKind::Comment),
    ("DEBUG", TokenKind::Builtin),
    ("INFO", TokenKind::Inserted),
    ("NOTICE", TokenKind::Inserted),
    ("WARN", TokenKind::Warning),
    ("WARNING", TokenKind::Warning),
    ("ERROR", TokenKind::Deleted),
    ("ERR", TokenKind::Deleted),
    ("FATAL", TokenKind::Deleted),
    ("CRITICAL", TokenKind::Deleted),
    ("CRIT", TokenKind::Deleted),
    ("SEVERE", TokenKind::Deleted),
    ("PANIC", TokenKind::Deleted),
    ("ALERT", TokenKind::Deleted),
    ("EMERG", TokenKind::Deleted),
];

/// 值是日志级别的键（小写），例如 `level=info`。
const LEVEL_KEYS: &[&str] = &["level", "lvl", "severity", "loglevel", "log_level"];

/// 月份的英文缩写，用于 `Nov 16 10:42:33` 和 `16/Nov/2026:10:42:33 +0000` 两种时间戳。
const MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// 可以紧跟在数字之后的单位（小写）：时长、大小、频率和百分比。
const UNITS: &[&str] = &[
    "ns", "us", "µs", "ms", "s", "m", "h", "d", "b", "kb", "mb", "gb", "tb", "kib", "mib", "gib",
    "tib", "hz", "khz", "mhz", "%",
];

impl Lexer<'_> {
    /// 为日志文件的一行着色，返回下一行开头的状态（总是 `Normal`）。
    pub(super) fn log_line(&mut self) -> LineState {
        if self.tokens.is_none() {
            return LineState::Normal;
        }
        let chars = self.chars;
        let n = chars.len();
        let mut i = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(n);
        // 调用栈的一行：缩进之后以 `at ` 开头，其后的函数名单独着色。
        let frame = i > 0 && starts_with(chars, i, "at ");
        if frame {
            self.push(i, i + 2, TokenKind::Keyword);
            i += 2;
        }
        // 本行余下部分没有收尾的单引号时不再查找，避免每个撇号都扫描到行尾。
        let mut quotes = true;
        while i < n {
            let token = match chars[i] {
                '"' => string_end(chars, i, '"').map(|end| (end, TokenKind::String)),
                '\'' if quotes => {
                    let end = string_end(chars, i, '\'');
                    quotes = end.is_some() || i > 0 && is_word(chars[i - 1]);
                    end.map(|end| (end, TokenKind::String))
                }
                '/' | '.' | '~' => path_end(chars, i).map(|end| (end, TokenKind::Link)),
                c if c.is_ascii_digit() => numeric(chars, i),
                c if is_word(c) => word(chars, i, frame),
                _ => None,
            };
            i = match token {
                Some((end, kind)) => {
                    self.push(i, end, kind);
                    end
                }
                None if is_word(chars[i]) => word_end(chars, i),
                None => i + 1,
            };
        }
        LineState::Normal
    }
}

/// 是否是单词中的字符：字母、数字或 `_`。
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 返回从 `i` 起的单词（字母、数字、`_`）的末尾之后的位置。
fn word_end(chars: &[char], i: usize) -> usize {
    i + chars[i..].iter().take_while(|&&c| is_word(c)).count()
}

/// 返回从 `i` 起连续 ASCII 数字的末尾之后的位置；`i` 超出行尾时返回 `i`。
fn digits_end(chars: &[char], i: usize) -> usize {
    let rest = chars.get(i..).unwrap_or_default();
    i + rest.iter().take_while(|c| c.is_ascii_digit()).count()
}

/// `i` 处是否不在单词中间：到了行尾，或该处不是单词中的字符。
fn ends(chars: &[char], i: usize) -> bool {
    chars.get(i).is_none_or(|&c| !is_word(c))
}

/// 从 `i` 起恰好是 `count` 位 ASCII 数字时返回其后的位置。
fn fixed_digits(chars: &[char], i: usize, count: usize) -> Option<usize> {
    (digits_end(chars, i) == i + count).then_some(i + count)
}

/// 返回 `i` 处引号 `quote` 括起的字符串的末尾之后的位置；本行内没有收尾时返回 `None`。
///
/// 单引号紧跟在单词之后（`don't`）或其收尾紧接单词时是撇号，不算字符串。
fn string_end(chars: &[char], i: usize, quote: char) -> Option<usize> {
    let apostrophe = quote == '\'';
    if apostrophe && i > 0 && is_word(chars[i - 1]) {
        return None;
    }
    let mut j = i + 1;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 1,
            c if c == quote && !(apostrophe && !ends(chars, j + 1)) => {
                return (j > i + 1).then_some(j + 1);
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// 返回 `i` 处时间（`10:42:31`、`10:42:31.123Z`、`10:42:31,123+08:00`）的末尾之后的位置。
fn time_end(chars: &[char], i: usize) -> Option<usize> {
    let mut end = fixed_digits(chars, i, 2)?;
    for _ in 0..2 {
        end = fixed_digits(chars, (chars.get(end) == Some(&':')).then_some(end + 1)?, 2)?;
    }
    if matches!(chars.get(end), Some('.' | ','))
        && chars.get(end + 1).is_some_and(char::is_ascii_digit)
    {
        end = digits_end(chars, end + 1);
    }
    Some(zone_end(chars, end).unwrap_or(end))
}

/// 返回 `i` 处时区（`Z`、`+08:00`、`-0500`）的末尾之后的位置。
fn zone_end(chars: &[char], i: usize) -> Option<usize> {
    match chars.get(i)? {
        'Z' => Some(i + 1),
        '+' | '-' => {
            let hours = fixed_digits(chars, i + 1, 2).or(fixed_digits(chars, i + 1, 4))?;
            match chars.get(hours) {
                Some(':') => fixed_digits(chars, hours + 1, 2).or(Some(hours)),
                _ => Some(hours),
            }
        }
        _ => None,
    }
}

/// 返回 `i` 处以数字开头的时间戳的末尾之后的位置：`2026-11-16T10:42:31.123Z`、
/// `2026/11/16 10:42:31`、`16/Nov/2026:10:42:33 +0000`，或只有时间的 `10:42:31`。
fn timestamp_end(chars: &[char], i: usize) -> Option<usize> {
    let at = |k: usize| chars.get(k).copied();
    let end = if let Some(year) = fixed_digits(chars, i, 4)
        && let Some(separator @ ('-' | '/')) = at(year)
        && let Some(month) = fixed_digits(chars, year + 1, 2)
        && at(month) == Some(separator)
        && let Some(day) = fixed_digits(chars, month + 1, 2)
    {
        match at(day) {
            Some('T' | ' ' | '_') => time_end(chars, day + 1).unwrap_or(day),
            _ => day,
        }
    } else if let Some(day) = fixed_digits(chars, i, 2)
        && at(day) == Some('/')
        && MONTHS.iter().any(|m| starts_with(chars, day + 1, m))
        && at(day + 4) == Some('/')
        && let Some(year) = fixed_digits(chars, day + 5, 4)
        && at(year) == Some(':')
        && let Some(time) = time_end(chars, year + 1)
    {
        match at(time) {
            Some(' ') => zone_end(chars, time + 1).unwrap_or(time),
            _ => time,
        }
    } else {
        time_end(chars, i)?
    };
    ends(chars, end).then_some(end)
}

/// 返回 `i` 处 IPv4 地址（可带 `:端口` 或 `/前缀长度`）的末尾之后的位置。
fn ip_end(chars: &[char], i: usize) -> Option<usize> {
    let mut end = i;
    for group in 0..4 {
        if group > 0 {
            end = (chars.get(end) == Some(&'.')).then_some(end + 1)?;
        }
        let stop = digits_end(chars, end);
        let value = chars[end..stop].iter().collect::<String>();
        if !(1..=3).contains(&(stop - end)) || value.parse::<u8>().is_err() {
            return None;
        }
        end = stop;
    }
    // 其后还有 `.数字` 时是版本号等更长的写法，不识别为地址。
    let longer = chars.get(end) == Some(&'.') && !ends(chars, end + 1);
    if longer || !ends(chars, end) {
        return None;
    }
    if matches!(chars.get(end), Some(':' | '/')) {
        let suffix = digits_end(chars, end + 1);
        if suffix > end + 1 && ends(chars, suffix) {
            end = suffix;
        }
    }
    Some(end)
}

/// 返回 `i` 处 UUID（`8-4-4-4-12` 位十六进制）的末尾之后的位置。
fn uuid_end(chars: &[char], i: usize) -> Option<usize> {
    let mut end = i;
    for (group, length) in [8, 4, 4, 4, 12].into_iter().enumerate() {
        if group > 0 {
            end = (chars.get(end) == Some(&'-')).then_some(end + 1)?;
        }
        let hex = chars.get(end..end + length)?;
        end = hex
            .iter()
            .all(char::is_ascii_hexdigit)
            .then_some(end + length)?;
    }
    ends(chars, end).then_some(end)
}

/// 返回 `i` 处数字（`42`、`1.5`、`0x1f`、`1532ms`、`99%`）的末尾之后的位置。
///
/// 紧跟在 `.` 之后的数字（`v1.2` 中的 `2`）和后面紧接其他字母的数字（`3f2a`）不算。
fn number_end(chars: &[char], i: usize) -> Option<usize> {
    if i > 0 && chars[i - 1] == '.' {
        return None;
    }
    if chars[i] == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
        let end = i
            + 2
            + chars[i + 2..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
        return (end > i + 2 && ends(chars, end)).then_some(end);
    }
    let mut end = digits_end(chars, i);
    while chars.get(end) == Some(&'.') && chars.get(end + 1).is_some_and(char::is_ascii_digit) {
        end = digits_end(chars, end + 1);
    }
    if chars.get(end) == Some(&'%') {
        return Some(end + 1);
    }
    let unit = end
        + chars[end..]
            .iter()
            .take_while(|c| c.is_alphabetic())
            .count();
    if unit > end && ends(chars, unit) {
        let text = chars[end..unit].iter().collect::<String>().to_lowercase();
        return UNITS.contains(&text.as_str()).then_some(unit);
    }
    ends(chars, end).then_some(end)
}

/// 识别 `i` 处以数字开头的写法：时间戳、UUID、IP 地址或数字。
fn numeric(chars: &[char], i: usize) -> Option<(usize, TokenKind)> {
    if let Some(end) = timestamp_end(chars, i) {
        return Some((end, TokenKind::Comment));
    }
    uuid_end(chars, i)
        .or_else(|| ip_end(chars, i))
        .or_else(|| number_end(chars, i))
        .map(|end| (end, TokenKind::Number))
}

/// 是否是路径中的字符。
fn is_path(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\\' | '~' | '+' | '@' | '%')
}

/// 返回 `i` 处路径的末尾之后的位置：以 `/`、`./`、`../`、`~/` 开头，可带 `:行` 或 `:行:列`。
///
/// `/` 之前是单词、`/` 或 `<` 时不算（`and/or`、`//`、`</div>`）；末尾的 `.` 是句号，不计入。
fn path_end(chars: &[char], i: usize) -> Option<usize> {
    let root = match chars[i] {
        '/' => i,
        '~' => i + 1,
        _ => i + chars[i..].iter().take_while(|&&c| c == '.').count().min(2),
    };
    let before = i.checked_sub(1).map(|k| chars[k]);
    let first = *chars.get(root + 1)?;
    if chars[root] != '/'
        || before.is_some_and(|c| is_word(c) || matches!(c, '/' | '<' | '.' | '~'))
        || !(first.is_alphanumeric() || matches!(first, '_' | '.' | '~'))
    {
        return None;
    }
    Some(path_tail(chars, root + 1))
}

/// 返回从 `i` 起的路径余下部分（含可选的 `:行:列`）的末尾之后的位置。
fn path_tail(chars: &[char], i: usize) -> usize {
    let mut end = i + chars[i..].iter().take_while(|&&c| is_path(c)).count();
    while end > i && chars[end - 1] == '.' {
        end -= 1;
    }
    for _ in 0..2 {
        let number = digits_end(chars, end + 1);
        if chars.get(end) != Some(&':') || number == end + 1 || !ends(chars, number) {
            break;
        }
        end = number;
    }
    end
}

/// 返回 `i` 处链接（`scheme://…`）的末尾之后的位置；末尾的标点不计入。
fn url_end(chars: &[char], i: usize) -> Option<usize> {
    let scheme = i + chars[i..]
        .iter()
        .take_while(|&&c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        .count();
    if !starts_with(chars, scheme, "://") {
        return None;
    }
    let stop = |c: &char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`');
    let mut end = scheme + 3;
    end += chars[end..].iter().take_while(|c| !stop(c)).count();
    while end > scheme + 3
        && matches!(
            chars[end - 1],
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'
        )
    {
        end -= 1;
    }
    (end > scheme + 3).then_some(end)
}

/// `i` 处的单词是否是值为级别的键（见 [`LEVEL_KEYS`]）之后的值：`level=info`、`level: warn`。
fn after_level_key(chars: &[char], i: usize) -> bool {
    let before = chars[..i]
        .iter()
        .rev()
        .skip_while(|c| matches!(c, ' ' | '"'));
    let mut before = before.peekable();
    if !matches!(before.next(), Some('=' | ':')) {
        return false;
    }
    let key: String = before
        .skip_while(|c| matches!(c, ' ' | '"'))
        .take_while(|&&c| is_word(c))
        .collect();
    let key: String = key.chars().rev().collect::<String>().to_ascii_lowercase();
    LEVEL_KEYS.contains(&key.as_str())
}

/// 返回 `i` 处的单词作为日志级别时的着色类别。
///
/// 全大写的级别在任何位置都识别；其他大小写只在 `[info]`、`error:` 和 `level=warn` 这些位置
/// 识别，正文中的 `error`、`info` 是普通单词。
fn level(chars: &[char], i: usize, end: usize) -> Option<TokenKind> {
    let text: String = chars[i..end].iter().collect();
    let upper = text.to_ascii_uppercase();
    let (_, kind) = LEVELS.iter().find(|(name, _)| *name == upper)?;
    let bracketed = i > 0 && chars[i - 1] == '[' && chars.get(end) == Some(&']');
    let labelled = chars.get(end) == Some(&':') && chars.get(end + 1) != Some(&':');
    (text == upper || bracketed || labelled || after_level_key(chars, i)).then_some(*kind)
}

/// 识别 `i` 处以字母或 `_` 开头的写法：时间戳、链接、盘符路径、UUID、键、级别、异常和调用栈。
///
/// `frame` 表示本行是调用栈的一行（`at 包.类.方法(文件:行)`）。
fn word(chars: &[char], i: usize, frame: bool) -> Option<(usize, TokenKind)> {
    let at = |k: usize| chars.get(k).copied();
    let end = word_end(chars, i);
    // `Nov 16 10:42:33`：月份缩写、日期和时间。
    if end == i + 3 && MONTHS.iter().any(|m| starts_with(chars, i, m)) {
        let day = i + 3 + chars[i + 3..].iter().take_while(|&&c| c == ' ').count();
        let time = digits_end(chars, day);
        if (1..=2).contains(&(day - i - 3))
            && (1..=2).contains(&(time - day))
            && at(time) == Some(' ')
            && let Some(end) = time_end(chars, time + 1)
            && ends(chars, end)
        {
            return Some((end, TokenKind::Comment));
        }
    }
    if let Some(end) = url_end(chars, i) {
        return Some((end, TokenKind::Link));
    }
    if end == i + 1
        && chars[i].is_ascii_alphabetic()
        && at(end) == Some(':')
        && matches!(at(end + 1), Some('\\' | '/'))
        && at(end + 2).is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return Some((path_tail(chars, end + 2), TokenKind::Link));
    }
    if let Some(end) = uuid_end(chars, i) {
        return Some((end, TokenKind::Number));
    }
    if let Some(kind) = level(chars, i, end) {
        return Some((end, kind));
    }
    // `Caused by:`、`Traceback (most recent call last):`、`panic:`、`panicked at`。
    let text: String = chars[i..end].iter().collect();
    let marker = match text.as_str() {
        "Caused" if starts_with(chars, end, " by:") => Some(end + 3),
        "Traceback" if starts_with(chars, end, " (most recent call last)") => Some(end),
        "panic" if at(end) == Some(':') => Some(end),
        "panicked" if starts_with(chars, end, " at ") => Some(end),
        _ => None,
    };
    if let Some(end) = marker {
        return Some((end, TokenKind::Keyword));
    }
    // 带包名的名字：`java.net.ConnectException`、`com.example.Main$Inner.run`。
    let mut dotted = end;
    let mut last = i;
    while matches!(at(dotted), Some('.' | '$')) && at(dotted + 1).is_some_and(is_word) {
        last = dotted + 1;
        dotted = word_end(chars, last);
    }
    if frame && at(dotted) == Some('(') {
        return Some((dotted, TokenKind::Function));
    }
    let name: String = chars[last..dotted].iter().collect();
    let exception = name.ends_with("Exception") || name.ends_with("Error") && name != "Error";
    if exception && name.starts_with(char::is_uppercase) {
        return Some((dotted, TokenKind::Type));
    }
    // `key=value` 的键：前面是行首、空白或分隔符，`=` 之后紧接着值。
    let mut key = end;
    while matches!(at(key), Some('.' | '-')) && at(key + 1).is_some_and(is_word) {
        key = word_end(chars, key + 1);
    }
    let starts = i == 0 || chars[i - 1].is_whitespace() || "([{,;|".contains(chars[i - 1]);
    let value = at(key + 1).is_some_and(|c| !c.is_whitespace() && c != '=');
    (starts && at(key) == Some('=') && value).then_some((key, TokenKind::Property))
}
