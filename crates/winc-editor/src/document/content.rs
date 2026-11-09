//! 文档内容的存储：普通文件整个读入 `Rope`；
//! 大文件（见 [`LargeText`]）的原文留在磁盘上，内存中只保存改动。

use std::ops::ControlFlow;

use ropey::Rope;
use winc_core::text::{EditBuffer, Line, Lines};

use crate::document::large::LargeText;

/// 文档的文本内容：整个读入内存的 `Rope`，或原文留在磁盘上的大文件 [`LargeText`]。
///
/// `LargeText` 含有原文句柄、新增缓冲、片段表和行索引，体积比 `Rope`（只含一个指针）大，
/// 因此放在堆上（`Box`）。绝大多数文档是 `Rope`，不必为大文件的字段占用内存。
pub enum Content {
    /// 整个读进内存的文本。
    Rope(Rope),
    /// 原文留在磁盘上、只把改动放在内存中的大文件。
    Large(Box<LargeText>),
}

impl Content {
    /// 返回普通文档的 `Rope`；大文件返回 `None`。
    pub fn rope(&self) -> Option<&Rope> {
        match self {
            Content::Rope(rope) => Some(rope),
            Content::Large(_) => None,
        }
    }

    /// 是否为大文件（原文留在磁盘上按需读取）。
    pub fn is_large(&self) -> bool {
        matches!(self, Content::Large(_))
    }

    /// 返回字符位置 `from..to` 之间的 UTF-8 字节数。
    ///
    /// 复制或大范围编辑之前，调用本方法判断范围是否超过限制。范围为空时直接返回 0：大文件
    /// 定位字符位置要读磁盘，而每次按键都要对光标计算一次。
    pub fn byte_len(&self, from: usize, to: usize) -> u64 {
        if from >= to {
            return 0;
        }
        match self {
            Content::Rope(rope) => {
                let to = to.min(rope.len_chars());
                (rope.char_to_byte(to) - rope.char_to_byte(from.min(to))) as u64
            }
            Content::Large(large) => large.byte_len(from, to),
        }
    }

    /// 返回字符位置 `from..to` 之间的文本；范围超过 `limit` 字节时返回 `None`。
    pub fn text_range(&self, from: usize, to: usize, limit: u64) -> Option<String> {
        (self.byte_len(from, to) <= limit).then(|| self.substring(from, to))
    }
}

impl Lines for Content {
    fn line_count(&self) -> usize {
        match self {
            Content::Rope(rope) => rope.line_count(),
            Content::Large(large) => large.line_count(),
        }
    }

    fn line_start(&self, line: usize) -> usize {
        match self {
            Content::Rope(rope) => rope.line_start(line),
            Content::Large(large) => large.line_start(line),
        }
    }

    fn line_of(&self, pos: usize) -> usize {
        match self {
            Content::Rope(rope) => rope.line_of(pos),
            Content::Large(large) => large.line_of(pos),
        }
    }

    fn line_content(&self, line: usize) -> Line<'_> {
        match self {
            Content::Rope(rope) => rope.line_content(line),
            Content::Large(large) => large.line_content(line),
        }
    }

    fn line_prefix(&self, line: usize, max_bytes: usize) -> Line<'_> {
        match self {
            Content::Rope(rope) => rope.line_prefix(line, max_bytes),
            Content::Large(large) => large.line_prefix(line, max_bytes),
        }
    }

    fn substring(&self, from: usize, to: usize) -> String {
        match self {
            Content::Rope(rope) => rope.substring(from, to),
            Content::Large(large) => large.substring(from, to),
        }
    }

    fn end_pos(&self) -> usize {
        match self {
            Content::Rope(rope) => rope.end_pos(),
            Content::Large(large) => large.end_pos(),
        }
    }

    fn line_len(&self, line: usize) -> usize {
        match self {
            Content::Rope(rope) => rope.line_len(line),
            Content::Large(large) => large.line_len(line),
        }
    }

    fn visit_lines(
        &self,
        from: usize,
        max_bytes: usize,
        f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize> {
        match self {
            Content::Rope(rope) => rope.visit_lines(from, max_bytes, f),
            Content::Large(large) => large.visit_lines(from, max_bytes, f),
        }
    }

    fn visit_short_lines(
        &self,
        from: usize,
        max_bytes: usize,
        f: impl FnMut(usize, usize, Option<&str>) -> ControlFlow<()>,
    ) -> Option<usize> {
        match self {
            Content::Rope(rope) => rope.visit_short_lines(from, max_bytes, f),
            Content::Large(large) => large.visit_short_lines(from, max_bytes, f),
        }
    }

    fn char_at(&self, pos: usize) -> Option<char> {
        match self {
            Content::Rope(rope) => rope.char_at(pos),
            Content::Large(large) => large.char_at(pos),
        }
    }
}

impl EditBuffer for Content {
    fn replace(&mut self, start: usize, removed: usize, inserted: &str) {
        match self {
            Content::Rope(rope) => rope.replace(start, removed, inserted),
            Content::Large(large) => large.replace(start, removed, inserted),
        }
    }

    fn replace_each(&mut self, changes: &[(usize, usize, &str)]) {
        match self {
            Content::Rope(rope) => rope.replace_each(changes),
            Content::Large(large) => large.replace_each(changes),
        }
    }
}
