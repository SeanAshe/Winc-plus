//! UTF-8 字节处理的辅助函数：统计字符数、查找字符位置、截断超长行、解码时替换非法字节。

use super::*;

/// 把 `bytes` 追加到 `line`，使 `line` 的长度不超过 `cap` 字节；超出上限的字节丢弃。
pub(super) fn push_capped(line: &mut Vec<u8>, bytes: &[u8], cap: usize) {
    let room = cap.saturating_sub(line.len());
    line.extend_from_slice(&bytes[..bytes.len().min(room)]);
}

/// 返回一行中可访问部分的字节数。
///
/// 超过 `cap` 时截到 `cap`，不切开字符；读到换行符时去掉行尾的 `\r`。
pub(crate) fn visible_len(bytes: &[u8], ended: bool, cap: usize) -> usize {
    if bytes.len() >= cap {
        complete_prefix(&bytes[..cap])
    } else if ended && bytes.last() == Some(&b'\r') {
        bytes.len() - 1
    } else {
        bytes.len()
    }
}

/// 把一行的字节解码为文字写入 `out`，非法字节替换为 U+FFFD。
fn decode_into(bytes: &[u8], ended: bool, cap: usize, out: &mut String) {
    let bytes = &bytes[..visible_len(bytes, ended, cap)];
    out.clear();

    match std::str::from_utf8(bytes) {
        Ok(text) => out.push_str(text),
        Err(_) => out.push_str(&String::from_utf8_lossy(bytes)),
    }
}

/// 把一行解码后的内容交给 `f` 处理。
///
/// 已知是合法 UTF-8 时直接借用 `text`；否则先校验，不合法则有损解码到 `scratch`。
pub(super) fn emit_line(
    f: &mut impl FnMut(usize, usize, Option<&str>) -> ControlFlow<()>,
    (line, start): (usize, usize),
    bytes: &[u8],
    text: Option<&str>,
    ended: bool,
    cap: usize,
    scratch: &mut String,
) -> ControlFlow<()> {
    let len = visible_len(bytes, ended, cap);

    match text
        .map(|t| &t[..len])
        .or_else(|| std::str::from_utf8(&bytes[..len]).ok())
    {
        Some(text) => f(line, start, Some(text)),
        None => {
            decode_into(bytes, ended, cap, scratch);
            f(line, start, Some(scratch))
        }
    }
}

/// 返回合法 UTF-8 的字符数：统计不是后续字节（二进制 `10xxxxxx`）的字节。
pub(super) fn count_valid(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|&&b| (b as i8) >= -0x40).count() as u64
}

/// 返回按有损解码计算的字符数，每段连续的非法字节算一个字符。
pub(crate) fn char_count(bytes: &[u8]) -> u64 {
    if std::str::from_utf8(bytes).is_ok() {
        return count_valid(bytes);
    }

    bytes
        .utf8_chunks()
        .map(|chunk| chunk.valid().chars().count() as u64 + u64::from(!chunk.invalid().is_empty()))
        .sum()
}

/// 返回第 `n` 个字符（从 0 起）在字节串中的字节偏移。
///
/// 字符的计数规则同 [`char_count`]；`n` 超出字符总数时返回字节串的长度。
pub(super) fn byte_of_char(bytes: &[u8], n: usize) -> usize {
    let mut remaining = n;
    let mut offset = 0;

    for chunk in bytes.utf8_chunks() {
        for (i, _) in chunk.valid().char_indices() {
            if remaining == 0 {
                return offset + i;
            }

            remaining -= 1;
        }

        offset += chunk.valid().len();

        if !chunk.invalid().is_empty() {
            if remaining == 0 {
                return offset;
            }

            remaining -= 1;
            offset += chunk.invalid().len();
        }
    }

    offset
}

/// 返回 `bytes` 中最后一个完整字符之后的字节偏移；末尾被截断的多字节字符不计入。
pub(super) fn complete_prefix(bytes: &[u8]) -> usize {
    let n = bytes.len();

    for back in 1..=n.min(4) {
        let b = bytes[n - back];

        // 从末尾向前遇到的第一个非后续字节（`10xxxxxx`）即为最后一个字符的首字节。
        if b & 0xC0 != 0x80 {
            let width = match b {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            return if back >= width { n } else { n - back };
        }
    }

    n
}
