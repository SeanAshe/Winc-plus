//! 块之间多出的空行：可编辑的预览中每一行显示为一个空段落，占一行高度，光标可以停在上面。

use std::hash::{DefaultHasher, Hash, Hasher};

use super::{Block, Inline, LeafRange, Markdown};

impl Markdown {
    /// 在块之间多出的空行处各插入一个空段落，`source` 是解析所用的原文。
    ///
    /// 块之间的第一个空行只是分隔，不插入；其后连续的空行、文首的空行各插入一个。块的范围
    /// 末尾的空行（列表之后）不算在块里。空段落的范围是该行开头的空范围。
    pub fn with_blank_lines(self, source: &str) -> Self {
        self.with_blank_lines_inner(source, false)
    }

    /// [`Self::with_blank_lines`] 的内部实现：`has_next_block` 为真时表示 `source` 是文档中间的
    /// 局部切片（以分隔空行结尾，紧随其后还有下一块），忽略末尾换行符之后的虚空行。
    pub(super) fn with_blank_lines_inner(mut self, source: &str, has_next_block: bool) -> Self {
        self.blank_lines = true;
        let starts: Vec<usize> = std::iter::once(0)
            .chain(memchr::memchr_iter(b'\n', source.as_bytes()).map(|i| i + 1))
            .collect();
        let line_of = |at: usize| starts.partition_point(|&s| s <= at) - 1;
        let text = |l: usize| {
            let end = starts.get(l + 1).map_or(source.len(), |&e| e - 1);
            &source[starts[l]..end]
        };
        let blank = |l: usize| text(l).trim().is_empty();
        let mut covered = vec![false; starts.len()];
        for range in self.ranges.iter().filter(|r| r.end > r.start) {
            let first = line_of(range.start);
            let mut last = line_of(range.end - 1).max(first);
            while last > first && blank(last) {
                last -= 1;
            }
            covered[first..=last].fill(true);
        }
        let limit = if has_next_block {
            starts.len().saturating_sub(1)
        } else {
            starts.len()
        };
        let slots: Vec<usize> = (0..limit)
            .filter(|&l| !covered[l] && blank(l) && (l == 0 || blank(l - 1)))
            .collect();
        if slots.is_empty() {
            return self;
        }
        let hash = {
            let mut hasher = DefaultHasher::new();
            Block::Paragraph(Inline::default()).hash(&mut hasher);
            hasher.finish()
        };
        let count = self.blocks.len() + slots.len();
        let (mut blocks, mut hashes) = (Vec::with_capacity(count), Vec::with_capacity(count));
        let (mut lines, mut ranges) = (Vec::with_capacity(count), Vec::with_capacity(count));
        let old =
            (self.blocks.into_iter().zip(self.hashes)).zip(self.lines.into_iter().zip(self.ranges));
        let mut pending = slots.iter().peekable();
        for ((block, h), (line, range)) in old {
            while let Some(&&slot) = pending.peek().filter(|&&&s| s <= line) {
                blocks.push(Block::Paragraph(Inline::default()));
                hashes.push(hash);
                lines.push(slot);
                ranges.push(starts[slot]..starts[slot]);
                pending.next();
            }
            blocks.push(block);
            hashes.push(h);
            lines.push(line);
            ranges.push(range);
        }
        for &slot in pending {
            blocks.push(Block::Paragraph(Inline::default()));
            hashes.push(hash);
            lines.push(slot);
            ranges.push(starts[slot]..starts[slot]);
        }
        let empty = slots.iter().map(|&l| LeafRange {
            range: starts[l]..starts[l],
            prose: true,
            table: false,
        });
        self.leaves.extend(empty);
        self.leaves.sort_by_key(|l| l.range.start);
        self.blocks = blocks;
        self.hashes = hashes;
        self.lines = lines;
        self.ranges = ranges;
        self
    }
}
