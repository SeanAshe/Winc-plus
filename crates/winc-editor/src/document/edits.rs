//! 改动日志：最近若干次改动的版本号、第一处改动所在的行和位置，以及字节范围。
//!
//! 调用方据此判断某个版本之后哪些内容没有变：高亮、排版从未变的行之后重算，Markdown 预览换算
//! 解析之后的位置，并只重新解析改动所在的块。日志只记录经过编辑的改动；从磁盘重新读取这类
//! 变化不记录，查询时按「无法判断」处理。

use std::collections::VecDeque;
use std::ops::Range;

/// 改动日志保留的最近改动条数（条，64）。
///
/// 一次连续输入与随后的滚动查询通常只涉及几十次改动，64 条足以覆盖；记录更多只会增加内存开销。
const EDIT_LOG: usize = 64;

/// 一处连续改动在 UTF-8 文本中的字节范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteEdit {
    /// 改动前被替换的范围。
    pub old: Range<usize>,
    /// 改动后替换成的范围，起点与 `old` 相同。
    pub new: Range<usize>,
}

impl ByteEdit {
    /// 返回先做 `self`、再做 `next` 合并成的一处改动：覆盖两次改动的最小连续范围。
    ///
    /// `next` 的位置相对于 `self` 改动之后的文本。超出 `usize` 的范围时返回 `None`。
    pub(crate) fn then(&self, next: &ByteEdit) -> Option<ByteEdit> {
        let grown = self.new.end as isize - self.old.end as isize;
        let next_grown = next.new.end as isize - next.old.end as isize;
        // 把 `self` 改动之后的位置换回改动之前：位于新写入的内容中时取 `inside`。
        let back = |at: usize, inside: usize| {
            if at <= self.new.start {
                Some(at)
            } else if at >= self.new.end {
                at.checked_add_signed(-grown)
            } else {
                Some(inside)
            }
        };
        let start = self.old.start.min(back(next.old.start, self.old.start)?);
        let end = self.old.end.max(back(next.old.end, self.old.end)?);
        let new_end = end
            .checked_add_signed(grown)?
            .checked_add_signed(next_grown)?;
        Some(ByteEdit {
            old: start..end,
            new: start..new_end,
        })
    }
}

/// 日志中的一次改动。
struct Entry {
    /// 改动后的版本号。
    revision: u64,
    /// 第一处改动所在的行（从 0 起）。
    line: usize,
    /// 第一处改动的位置（字符偏移）。
    offset: usize,
    /// 改动只有一处时的字节范围；有多处，或文本不是 `rope` 时为 `None`。
    bytes: Option<ByteEdit>,
}

/// 最近 [`EDIT_LOG`] 次改动，按版本号从早到晚排列。
#[derive(Default)]
pub(crate) struct EditLog(VecDeque<Entry>);

impl EditLog {
    /// 记录版本 `revision` 的改动，第一处改动在第 `line` 行、位置 `offset`（字符偏移）。
    pub(crate) fn push(&mut self, revision: u64, line: usize, offset: usize) {
        self.0.push_back(Entry {
            revision,
            line,
            offset,
            bytes: None,
        });
        if self.0.len() > EDIT_LOG {
            self.0.pop_front();
        }
    }

    /// 为最近一次改动（版本 `revision`）记下字节范围；最近一次不是该版本时不记录。
    pub(crate) fn set_bytes(&mut self, revision: u64, bytes: ByteEdit) {
        if let Some(last) = self.0.back_mut().filter(|last| last.revision == revision) {
            last.bytes = Some(bytes);
        }
    }

    /// 返回版本 `revision` 之后、直到当前版本 `current` 的全部改动，从早到晚排列。
    ///
    /// 其间有不在日志中的版本（已移出日志，或是编辑以外的变化）时返回 `None`。
    fn since(&self, revision: u64, current: u64) -> Option<impl Iterator<Item = &Entry>> {
        let count = self
            .0
            .iter()
            .rev()
            .take_while(|e| e.revision > revision)
            .count();
        (current.checked_sub(revision) == Some(count as u64))
            .then(|| self.0.iter().skip(self.0.len() - count))
    }

    /// 返回从版本 `revision` 至今（当前版本 `current`）始终未变的（开头行数，开头字符数）；
    /// 版本未变时两者都是 `usize::MAX`，无法判断时都是 0。
    pub(crate) fn unchanged_since(&self, revision: u64, current: u64) -> (usize, usize) {
        let Some(edits) = self.since(revision, current) else {
            return (0, 0);
        };
        edits.fold((usize::MAX, usize::MAX), |(line, offset), e| {
            (line.min(e.line), offset.min(e.offset))
        })
    }

    /// 返回从版本 `revision` 至今（当前版本 `current`）的改动合并成的一处字节改动。
    ///
    /// 版本未变、其间有不在日志中的版本，或某次改动有多处时返回 `None`。
    pub(crate) fn bytes_since(&self, revision: u64, current: u64) -> Option<ByteEdit> {
        let mut edits = self.since(revision, current)?;
        let first = edits.next()?.bytes.clone()?;
        edits.try_fold(first, |merged, e| merged.then(e.bytes.as_ref()?))
    }
}

#[cfg(test)]
mod tests;
