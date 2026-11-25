//! 文件按什么规则作为文本读：大小的界限、二进制的判断、超长行的截断。
//!
//! 打开文件和搜索使用同一套判断，搜出的匹配位置才与打开后的文本和行号对应。

use crate::text::utf16_text;

/// 可打开的文件大小上限（字节，4 GiB）。
///
/// 超过该大小的文件不打开。上限取自调参：`rope` 把全文放在内存中并额外维护树的节点，内存占用
/// 随文件大小线性增长，超过该值后占用达到可用内存的量级。该上限同时约束打开文件与搜索的扫描，
/// 按需读取的方式见 [`LARGE_FILE`]。
pub const OPEN_LIMIT: u64 = 4 << 30;
/// 大文件的大小下限（字节，16 MiB）。
///
/// 超过该大小的文件按大文件打开：原文留在磁盘上按需读取，内存里只记录改动。下限取自调参：按大文件
/// 打开后每次滚动都要读磁盘，只有当整个文件读入内存的占用和耗时不可忽略时，这一代价才值得承担。
pub const LARGE_FILE: u64 = 16 << 20;
/// 带 BOM 的 UTF-16 整篇读入内存的大小上限（字节，64 MiB）。
///
/// 大文件按 UTF-8 字节索引磁盘上的原文，表示不了 UTF-16。不超过该上限时整篇解码；
/// 打开和搜索使用同一个上限，行号才能对上。再大的 UTF-16 文件不打开、不搜索。上限取自调参：
/// 解码后的文本与原文件同一量级，64 MiB 的占用可以接受。
pub const UTF16_INLINE: u64 = 64 << 20;
/// 二进制检测读取的文件前缀长度（字节，8 KiB）。
///
/// 这段前缀中出现 NUL 即视为二进制文件；带 UTF-16 BOM 的文件除外。
pub const BINARY_SNIFF: usize = 8 << 10;
/// 大文件中一行最多能访问的字节数（字节，16 MiB）。
///
/// 原文件中有更长的行时只读打开，行内超出的部分不可访问，也不搜索。
pub const LINE_CAP: usize = 16 << 20;

/// 是否为二进制文件：开头 [`BINARY_SNIFF`] 字节内含 NUL。
///
/// UTF-16 文件含大量 NUL，因此是 UTF-16 的文字（带 BOM，或认得出的不带 BOM 的）时不算
/// 二进制。`bytes` 应从文件开头起，超出 [`BINARY_SNIFF`] 的部分不检查。
pub fn looks_binary(bytes: &[u8]) -> bool {
    !utf16_text(bytes) && memchr::memchr(0, &bytes[..bytes.len().min(BINARY_SNIFF)]).is_some()
}

/// 返回一行中可访问部分的字节数。
///
/// 超过 `cap` 时截到 `cap`，不切开字符；读到换行符时（`ended`）去掉行尾的 `\r`。
pub fn visible_len(bytes: &[u8], ended: bool, cap: usize) -> usize {
    if bytes.len() >= cap {
        complete_prefix(&bytes[..cap])
    } else if ended && bytes.last() == Some(&b'\r') {
        bytes.len() - 1
    } else {
        bytes.len()
    }
}

/// 返回 `bytes` 中最后一个完整字符之后的字节偏移；末尾被截断的多字节字符不计入。
pub fn complete_prefix(bytes: &[u8]) -> usize {
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
