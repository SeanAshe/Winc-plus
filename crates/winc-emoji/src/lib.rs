//! emoji 序列的规则：哪些字符合成一个图形，哪些字符默认按 emoji 显示。
//!
//! 序列包括带变体选择符的符号（`❤` 加 U+FE0F）、带肤色的手势、用零宽连接符连起来的组合、
//! 国旗（两个区域指示符）和键帽。文本内核按它计算显示列和折行，渲染层按它把整个序列交给
//! emoji 字体合成，终端按它判断哪些字符并入前一格；三者共用这一份规则。

/// 变体选择符 15：要求按文字显示。
pub const VS15: char = '\u{fe0e}';
/// 变体选择符 16：要求按 emoji 显示。
pub const VS16: char = '\u{fe0f}';
/// 零宽连接符：把前后的 emoji 连成一个图形。
pub const ZWJ: char = '\u{200d}';
/// 键帽：与前面的数字、`#`、`*` 合成按键的图形。
pub const KEYCAP: char = '\u{20e3}';

/// 默认按 emoji 显示的字符所在的码点范围（含两端），按码点从小到大排列。
///
/// 取自 Unicode 16 的 `Emoji_Presentation` 属性；U+1FA70 至 U+1FAFF 整段列入，其中尚未分配的
/// 码点在任何字体中都没有字形。
const PRESENTATION: &[(u32, u32)] = &[
    (0x231A, 0x231B),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1E6, 0x1F1FF),
    (0x1F201, 0x1F201),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F236),
    (0x1F238, 0x1F23A),
    (0x1F250, 0x1F251),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D7),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FAFF),
];

/// 字符 `c` 是否默认按 emoji 显示；`☺`、`✔` 等默认按文字显示的符号返回假。
pub fn presentation(c: char) -> bool {
    let code = c as u32;
    code >= PRESENTATION[0].0
        && PRESENTATION
            .binary_search_by(|&(start, end)| {
                if code < start {
                    std::cmp::Ordering::Greater
                } else if code > end {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
}

/// 是否是肤色修饰符（U+1F3FB 至 U+1F3FF）。
pub fn modifier(c: char) -> bool {
    matches!(c, '\u{1f3fb}'..='\u{1f3ff}')
}

/// 是否是区域指示符（U+1F1E6 至 U+1F1FF）：两个连写表示一面国旗。
pub fn regional(c: char) -> bool {
    matches!(c, '\u{1f1e6}'..='\u{1f1ff}')
}

/// 是否是标签字符（U+E0020 至 U+E007F）：跟在旗帜之后表示地区旗。
fn tag(c: char) -> bool {
    matches!(c, '\u{e0020}'..='\u{e007f}')
}

/// 是否是键帽的基本字符：数字、`#`、`*`。
fn keycap_base(c: char) -> bool {
    matches!(c, '0'..='9' | '#' | '*')
}

/// 字符 `c` 是否是图形符号：默认按 emoji 显示的字符，以及符号区中默认按文字显示、加上变体
/// 选择符或肤色后按 emoji 显示的符号（`♂`、`⚕`、`❤`、`✌`）。
pub fn pictographic(c: char) -> bool {
    presentation(c) || matches!(c, '\u{2190}'..='\u{2bff}' | '\u{1f000}'..='\u{1faff}')
}

/// 字符 `c` 是否可以结束一个 emoji：其后的零宽连接符把下一个图形连进来。
fn ends_emoji(c: char) -> bool {
    presentation(c) || c == VS16 || modifier(c) || c == '\u{e007f}'
}

/// 字符 `c` 是否接在它前面的两个字符 `before`（较远的在前，没有时为 `None`）之后，与它们属于
/// 同一个 emoji 序列。
///
/// 判断只依赖前两个字符，无需从头扫描：变体选择符接在任何可见字符之后；肤色修饰符接在图形符号之后；
/// 键帽接在数字、`#`、`*` 之后；零宽连接符接在 emoji 之后，并连入其后的图形符号（印度系文字中的
/// 连接符除外）。成对的区域指示符见 [`sequence_len`]。
pub fn joined(before: (Option<char>, Option<char>), c: char) -> bool {
    let (Some(previous), earlier) = (before.1, before.0) else {
        return false;
    };
    // 序列的后续字符均不小于零宽连接符，据此排除普通文字。
    if c < ZWJ {
        return false;
    }
    match c {
        VS15 | VS16 => !(previous.is_whitespace() || previous.is_control() || previous == ZWJ),
        ZWJ => ends_emoji(previous),
        KEYCAP => keycap_base(previous) || (previous == VS16 && earlier.is_some_and(keycap_base)),
        c if modifier(c) => pictographic(previous) || previous == VS16,
        c if tag(c) => tag(previous) || previous == '\u{1f3f4}',
        c => previous == ZWJ && pictographic(c) && earlier.is_some_and(ends_emoji),
    }
}

/// 返回 `chars` 开头的 emoji 序列的字符数；开头不是多字符序列时返回 1，`chars` 为空时返回 0。
///
/// 两个连写的区域指示符是一面国旗；其余按 [`joined`] 逐个接上。
pub fn sequence_len(chars: impl IntoIterator<Item = char>) -> usize {
    let mut chars = chars.into_iter();
    let Some(base) = chars.next() else {
        return 0;
    };
    if regional(base) {
        return 1 + usize::from(chars.next().is_some_and(regional));
    }
    let mut before = (None, Some(base));
    let mut len = 1;
    for c in chars {
        if !joined(before, c) {
            break;
        }
        before = (before.1, Some(c));
        len += 1;
    }
    len
}

/// 字符 `c` 紧跟 `previous` 时是否可能属于同一个 emoji 序列。
///
/// 用于在逐字符的循环中排除普通文字；为真时再用 [`sequence_len`] 或 [`joined`] 确认。
#[inline]
pub fn may_join(previous: char, c: char) -> bool {
    c >= ZWJ
        && (previous == ZWJ
            || matches!(c, VS15 | VS16 | ZWJ | KEYCAP)
            || modifier(c)
            || tag(c)
            || (regional(previous) && regional(c)))
}

/// 序列 `unit`（见 [`sequence_len`]）是否按 emoji 显示，由 emoji 字体合成。
///
/// 带变体选择符 15 的按文字显示；ASCII 字符只有组成键帽时才是 emoji。
pub fn presents_as_emoji(unit: &str) -> bool {
    let base = unit.chars().next().unwrap_or(' ');
    !unit.contains(VS15) && (!base.is_ascii() || unit.contains(KEYCAP))
}

/// 序列中不单独绘制的字符：变体选择符、零宽连接符和标签字符。
pub fn ignorable(c: char) -> bool {
    matches!(c, VS15 | VS16 | ZWJ) || tag(c)
}

/// 一段文字中的一个绘制单位：单个字符，或一个 emoji 序列。
pub enum Unit<'a> {
    /// 单个字符。
    Char(char),
    /// 由几个字符组成的 emoji 序列。
    Sequence(&'a str),
}

/// 返回 `text` 的各个绘制单位。
pub fn units(text: &str) -> impl Iterator<Item = Unit<'_>> {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        let (start, c) = chars.next()?;
        if !chars.peek().is_some_and(|&(_, next)| may_join(c, next)) {
            return Some(Unit::Char(c));
        }
        let len = sequence_len(text[start..].chars());
        let mut end = start + c.len_utf8();
        for _ in 1..len {
            end += chars.next().map_or(0, |(_, c)| c.len_utf8());
        }
        Some(match len {
            1 => Unit::Char(c),
            _ => Unit::Sequence(&text[start..end]),
        })
    })
}

#[cfg(test)]
mod tests;
