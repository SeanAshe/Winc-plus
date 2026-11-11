//! 按键记法：按键序列与文本之间的转换。
//!
//! 可见字符记作字符本身；功能键和带修饰键的按键记作尖括号中的名字（`<Esc>`、`<C-d>`、
//! `<S-Left>`），`<` 记作 `<lt>`，空格记作 `<Space>`。名字与修饰键前缀不区分大小写；
//! 不构成记法的 `<` 按字符本身处理。[`write()`] 的输出经 [`parse`] 得到原来的按键序列。

use std::fmt;

use super::{Code, Key, MAX_FUNCTION_KEY, Mods, Named};

/// 尖括号记法中名字的最大长度（字符），等于最长的名字 `BackSpace` 的长度。
///
/// 查找右尖括号时最多向后读取这一长度，解析的耗时与文本长度成正比。
const MAX_NAME: usize = 9;

/// 名字与按键的对应；同一个按键有多个名字时，第一个用于输出。
const NAMES: &[(&str, Code, Mods)] = &[
    ("Esc", Code::Named(Named::Esc), Mods::NONE),
    ("CR", Code::Named(Named::Enter), Mods::NONE),
    ("Return", Code::Named(Named::Enter), Mods::NONE),
    ("Enter", Code::Named(Named::Enter), Mods::NONE),
    ("Tab", Code::Named(Named::Tab), Mods::NONE),
    ("BS", Code::Named(Named::Backspace), Mods::NONE),
    ("BackSpace", Code::Named(Named::Backspace), Mods::NONE),
    ("Del", Code::Named(Named::Delete), Mods::NONE),
    ("Delete", Code::Named(Named::Delete), Mods::NONE),
    ("Insert", Code::Named(Named::Insert), Mods::NONE),
    ("Up", Code::Named(Named::Up), Mods::NONE),
    ("Down", Code::Named(Named::Down), Mods::NONE),
    ("Left", Code::Named(Named::Left), Mods::NONE),
    ("Right", Code::Named(Named::Right), Mods::NONE),
    ("Home", Code::Named(Named::Home), Mods::NONE),
    ("End", Code::Named(Named::End), Mods::NONE),
    ("PageUp", Code::Named(Named::PageUp), Mods::NONE),
    ("PageDown", Code::Named(Named::PageDown), Mods::NONE),
    ("Space", Code::Char(' '), Mods::NONE),
    ("lt", Code::Char('<'), Mods::NONE),
    ("Bar", Code::Char('|'), Mods::NONE),
    ("Bslash", Code::Char('\\'), Mods::NONE),
    ("Nul", Code::Char('@'), Mods::CTRL),
    ("NL", Code::Char('j'), Mods::CTRL),
    ("NewLine", Code::Char('j'), Mods::CTRL),
    ("LineFeed", Code::Char('j'), Mods::CTRL),
];

/// 修饰键前缀的字母与修饰键，按输出的顺序排列；`A` 是 `M` 的另一种写法，只用于解析。
const MOD_LETTERS: &[(char, Mods)] = &[
    ('M', Mods::ALT),
    ('C', Mods::CTRL),
    ('S', Mods::SHIFT),
    ('D', Mods::CMD),
    ('A', Mods::ALT),
];

/// 把按键记法的文本解析成按键序列。
///
/// 文本中的控制字符按 [`Key::new`] 归并（换行符即 `<NL>`）。不构成记法的 `<`（名字未知、
/// 缺少右尖括号）按字符本身处理，因此任何文本都能解析。
pub fn parse(text: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some((key, len)) = special(rest)
        {
            keys.push(key);
            rest = &rest[len..];
        } else {
            keys.push(Key::char(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    keys
}

/// 把按键序列写成按键记法的文本。
pub fn write(keys: &[Key]) -> String {
    let mut text = String::new();
    for key in keys {
        // 向 `String` 写入不会失败。
        let _ = write_key(&mut text, *key);
    }
    text
}

/// 解析以 `<` 开头的 `text` 起始处的尖括号记法，返回按键与记法占用的字节数。
///
/// 起始处不是合法的记法时返回 `None`。
fn special(text: &str) -> Option<(Key, usize)> {
    let body = text.strip_prefix('<')?;
    let (mods, rest) = split_mods(body);
    // 修饰键之后紧跟 `>` 时，该 `>` 是按键的字符，其后还需要一个右尖括号（`<C->>`）。
    let name_len = if !mods.is_empty() && rest.starts_with('>') {
        1
    } else {
        rest.char_indices()
            .take(MAX_NAME + 1)
            .find_map(|(i, c)| (c == '>').then_some(i))?
    };
    let (name, tail) = rest.split_at(name_len);
    if !tail.starts_with('>') {
        return None;
    }

    let mut chars = name.chars();
    let (code, extra) = match (chars.next(), chars.next()) {
        // 单个字符只有带修饰键时才是记法：`<C-d>` 是按键，`<d>` 是三个字符。
        (Some(c), None) if !mods.is_empty() => (Code::Char(c), Mods::NONE),
        _ => named(name)?,
    };
    let len = text.len() - tail.len() + 1;
    Some((Key::new(code, mods.with(extra)), len))
}

/// 拆出 `text` 开头的修饰键前缀（`C-`、`S-`、`M-`、`A-`、`D-`），返回修饰键与余下的文本。
///
/// 前缀之后没有内容时不算前缀：`<C-->` 中第二个 `-` 是按键的字符。
fn split_mods(mut text: &str) -> (Mods, &str) {
    let mut mods = Mods::NONE;
    loop {
        let mut chars = text.chars();
        let (Some(letter), Some('-')) = (chars.next(), chars.next()) else {
            break;
        };
        let rest = chars.as_str();
        let found = (MOD_LETTERS.iter()).find(|(l, _)| l.eq_ignore_ascii_case(&letter));
        let Some((_, modifier)) = found else {
            break;
        };
        if rest.is_empty() {
            break;
        }
        mods = mods.with(*modifier);
        text = rest;
    }
    (mods, text)
}

/// 按名字（不区分大小写）查找按键；名字未知时返回 `None`。
fn named(name: &str) -> Option<(Code, Mods)> {
    if let Some((_, code, mods)) = NAMES.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name)) {
        return Some((*code, *mods));
    }
    // 功能键：`F` 后跟不带前导零的序号。
    let digits = name.strip_prefix(['F', 'f'])?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u8 = digits.parse().ok()?;
    (1..=MAX_FUNCTION_KEY)
        .contains(&number)
        .then_some((Code::Named(Named::F(number)), Mods::NONE))
}

/// 把 `key` 写成按键记法。
pub(super) fn write_key(f: &mut impl fmt::Write, key: Key) -> fmt::Result {
    let mods = key.mods();
    match key.code() {
        Code::Char('<') if mods.is_empty() => f.write_str("<lt>"),
        Code::Char(' ') if mods.is_empty() => f.write_str("<Space>"),
        Code::Char(c) if mods.is_empty() => f.write_char(c),
        code => {
            f.write_char('<')?;
            write_mods(f, mods)?;
            match code {
                Code::Char('<') => f.write_str("lt")?,
                Code::Char(' ') => f.write_str("Space")?,
                Code::Char(c) => f.write_char(c)?,
                Code::Named(Named::F(number)) => write!(f, "F{number}")?,
                Code::Named(_) => {
                    let name = (NAMES.iter())
                        .find(|(_, c, m)| *c == code && m.is_empty())
                        .map_or("", |(name, ..)| name);
                    f.write_str(name)?;
                }
            }
            f.write_char('>')
        }
    }
}

/// 按 `M-`、`C-`、`S-`、`D-` 的顺序写出修饰键前缀。
pub(super) fn write_mods(f: &mut impl fmt::Write, mods: Mods) -> fmt::Result {
    for (letter, modifier) in &MOD_LETTERS[..4] {
        if mods.contains(*modifier) {
            write!(f, "{letter}-")?;
        }
    }
    Ok(())
}
