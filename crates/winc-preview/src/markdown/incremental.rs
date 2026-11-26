//! 局部重解析：一处改动只影响所在的几个顶层块时，只重新解析这些块，结果与全文解析相同。
//!
//! 重新解析的窗口向两侧扩展到与相邻块隔着空行的位置；窗口的解析结果替换其中的块，之后的块
//! 只平移位置和行号。改动可能影响窗口之外的结构时不做局部解析，由调用方解析全文：YAML 头、
//! 脚注、链接引用定义、围栏和 HTML 块、目录中的标题，以及跨空行延续的列表。

use std::ops::Range;

use super::{Block, LeafRange, Markdown, parse};

/// 返回原文中是否可能有链接引用定义（`[id]: url`，脚注除外）。
///
/// 只按行首的写法判断，可能把普通文字当作定义；误判时只是不做局部解析。
pub(super) fn has_link_ref_defs(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start_matches([' ', '\t']);
        line.starts_with('[') && !line.starts_with("[^") && line.contains("]:")
    })
}

/// 返回窗口的原文中是否有可能影响窗口之外的写法：脚注、链接引用定义、围栏、公式块、HTML 和
/// 目录。
///
/// 只按字面判断，`<` 这样的普通文字同样命中；命中时只是不做局部解析。
fn reaches_outside(text: &str) -> bool {
    ["[^", "```", "~~~", "$$", "<", "]:"]
        .iter()
        .any(|syntax| text.contains(syntax))
        || text
            .as_bytes()
            .windows(5)
            .any(|w| w.eq_ignore_ascii_case(b"[toc]"))
}

/// 返回 `bytes` 是否以至少一个空行结尾，即窗口与后一块之间仍隔着空行。
fn ends_with_blank_line(bytes: &[u8]) -> bool {
    let Some(mut i) = bytes.len().checked_sub(1).filter(|&i| bytes[i] == b'\n') else {
        return false;
    };
    if i > 0 && bytes[i - 1] == b'\r' {
        i -= 1;
    }
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    i > 0 && bytes[i - 1] == b'\n'
}

/// 返回两块之间的原文 `gap` 是否含有空行：全部是空白，且含有换行。
fn is_blank_gap(gap: &str) -> bool {
    gap.contains('\n')
        && gap
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
}

/// 返回两块之间的原文 `gap` 末尾的缩进（字节）：最后一个换行之后的空白，属于后一块所在的行。
fn indent(gap: &str) -> usize {
    gap.rfind('\n').map_or(0, |at| gap.len() - at - 1)
}

/// 返回块 `block` 的第一行是否可能缩进：顶层段落的范围从行首算起，缩进在范围之内，段落的
/// 文字因而不从范围的起点开始。
///
/// 以 `**` 这类标记开头的段落同样命中；误判时只是不做局部解析。
fn starts_indented(block: &Block) -> bool {
    matches!(block, Block::Paragraph(inline) if inline.source > 0)
}

/// 返回原文 `text` 的开头能否接在前面的列表之后：缩进（列表项的后续内容），或可能是列表标记。
///
/// 列表能跨空行延续：`- a\n\n- b` 是同一个列表，`- a\n\n  b` 中的 `b` 属于第一项。只看第一个
/// 字符，可能误判；误判时只是不做局部解析。
fn may_continue_list(text: &str) -> bool {
    text.starts_with([' ', '\t', '-', '*', '+']) || text.starts_with(|c: char| c.is_ascii_digit())
}

/// 返回位置 `at` 平移 `delta` 个字节后的位置。
///
/// 只用于窗口之后的位置：平移后不早于窗口改动后的终点，不会小于 0。
fn shift_byte(at: usize, delta: isize) -> usize {
    at.saturating_add_signed(delta)
}

/// 返回范围 `range` 平移 `delta` 个字节后的范围，见 [`shift_byte`]。
fn shift_range(range: &Range<usize>, delta: isize) -> Range<usize> {
    shift_byte(range.start, delta)..shift_byte(range.end, delta)
}

/// 返回块 `block` 的范围是否可能随相邻的原文变化：代码块、公式块、折叠块、脚注，以及正文与
/// 脚注之间的分隔线。
fn block_may_affect_boundary(block: &Block) -> bool {
    matches!(
        block,
        Block::Code { .. }
            | Block::Math(..)
            | Block::Details { .. }
            | Block::Footnote(..)
            | Block::Rule(None)
    )
}

/// 一次局部重解析的方案，见 [`Markdown::plan_local`]。
struct LocalPlan {
    /// 窗口中的顶层块在 `blocks` 中的下标范围；终点等于 `body` 时窗口延伸到正文末尾。
    blocks: Range<usize>,
    /// 正文之后第一个块（分隔线或脚注）的下标；没有脚注时为块数。
    body: usize,
    /// 窗口在改动前的原文中的字节范围。
    old: Range<usize>,
    /// 窗口之后的字节位置随改动的变化量（字节）。
    byte_delta: isize,
    /// 窗口之后的行号随改动的变化量（行）。
    line_delta: isize,
    /// 窗口起点所在的行（从 0 起）。
    start_line: usize,
    /// 窗口中的叶子块在 `leaves` 中的下标范围。
    leaves: Range<usize>,
    /// 窗口改动后的原文的解析结果，位置相对于窗口的起点。
    local: Markdown,
}

impl Markdown {
    /// 为把原文的 `old_edit` 换成 `new_edit`（改动后原文共 `new_len` 字节）制定局部重解析的方案；
    /// 改动可能影响窗口之外的结构，或窗口超过 `max_window` 字节时返回 `None`。
    ///
    /// `line_of_byte`、`slice` 读取改动后的原文：字节位置所在的行（从 0 起）和一段字节范围的文字。
    fn plan_local(
        &self,
        (old_edit, new_edit): (Range<usize>, Range<usize>),
        (new_len, max_window): (usize, usize),
        line_of_byte: &impl Fn(usize) -> usize,
        slice: &impl Fn(Range<usize>) -> String,
    ) -> Option<LocalPlan> {
        if old_edit.start != new_edit.start
            || old_edit.end > self.len
            || new_edit.end > new_len
            || self.has_link_defs
        {
            return None;
        }
        let byte_delta = new_edit.end as isize - old_edit.end as isize;
        if self.len.checked_add_signed(byte_delta) != Some(new_len) {
            return None;
        }
        let body = self
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Rule(None) | Block::Footnote(..)))
            .unwrap_or(self.blocks.len());
        if body == 0
            || self.ranges[body..]
                .iter()
                .any(|r| r.start < old_edit.end && r.end > old_edit.start)
        {
            return None;
        }

        // 窗口的起点：向前扩展到与前一块隔着空行的块。
        let mut first = self.ranges[..body].partition_point(|r| r.end < old_edit.start);
        let mut gap_before = String::new();
        while first > 0 {
            if first < body
                && !self.ranges[first - 1].is_empty()
                && !self.ranges[first].is_empty()
                && self.ranges[first].start <= old_edit.start
                && self.ranges[first - 1].end < self.ranges[first].start
            {
                gap_before = slice(self.ranges[first - 1].end..self.ranges[first].start);
                if is_blank_gap(&gap_before) {
                    break;
                }
            }
            first -= 1;
        }
        // 窗口从行首开始：块的缩进决定它是什么块，缩进的段落改成列表时范围会包含缩进。
        let old_start = if first == 0 {
            0
        } else {
            self.ranges[first].start - indent(&gap_before)
        };

        // 窗口的终点：向后扩展到与后一块仍隔着空行的块，空行按改动后的原文判断。
        let mut end = self.ranges[..body]
            .partition_point(|r| r.start <= old_edit.end)
            .max(first);
        let mut gap_after = String::new();
        while end < body {
            if end > first
                && !self.ranges[end - 1].is_empty()
                && !self.ranges[end].is_empty()
                && self.ranges[end - 1].end >= old_edit.end
                && self.ranges[end].start > old_edit.end
                && self.ranges[end - 1].end < self.ranges[end].start
            {
                let gap = shift_range(
                    &(self.ranges[end - 1].end..self.ranges[end].start),
                    byte_delta,
                );
                if gap.start < gap.end && gap.end <= new_len {
                    gap_after = slice(gap);
                    if is_blank_gap(&gap_after) {
                        break;
                    }
                }
            }
            end += 1;
        }
        // 窗口延伸到正文末尾、文末又有脚注时，脚注的位置无法局部换算。
        if end == body && body < self.blocks.len() {
            return None;
        }
        let old_end = if end < body {
            self.ranges[end].start - indent(&gap_after)
        } else {
            self.len
        };
        if old_start > old_edit.start || old_end < old_edit.end {
            return None;
        }
        let replaced = &self.blocks[first..end];
        let heading = |b: &Block| matches!(b, Block::Heading(..));
        if replaced.iter().any(block_may_affect_boundary)
            || (self.has_toc && replaced.iter().any(heading))
        {
            return None;
        }

        let new_end = old_end.checked_add_signed(byte_delta)?;
        if new_end < old_start || new_end > new_len || new_end - old_start > max_window {
            return None;
        }
        let window = slice(old_start..new_end);
        if (end < body && !ends_with_blank_line(window.as_bytes()))
            || (old_start == 0 && window.starts_with("---"))
            || reaches_outside(&window)
        {
            return None;
        }
        // 前一块是列表时，窗口的开头可能接在列表之后。
        let list = |b: &Block| matches!(b, Block::List(..));
        if first > 0 && list(&self.blocks[first - 1]) && may_continue_list(&window) {
            return None;
        }

        let mut local = parse(&window);
        if self.blank_lines {
            local = local.with_blank_lines_inner(&window, end < body);
        }
        if local.blocks.is_empty()
            || local.has_link_defs
            || local.has_toc
            || local.blocks.iter().any(block_may_affect_boundary)
            || (self.has_toc && local.blocks.iter().any(heading))
        {
            return None;
        }
        // 窗口的最后一块是列表时，后一块可能接在列表之后。
        let last = (local.ranges.iter().rposition(|r| !r.is_empty())).map(|i| &local.blocks[i]);
        if end < body
            && last.is_some_and(list)
            && (list(&self.blocks[end])
                || matches!(self.blocks[end], Block::Code { .. })
                || starts_indented(&self.blocks[end])
                || indent(&gap_after) > 0)
        {
            return None;
        }

        // 窗口中的叶子块：起点恰在窗口起点的空段落属于窗口；窗口延伸到正文末尾时其后的叶子
        // 都属于窗口（包括文末空行处的空段落）。
        let mut leaves_start = self.leaves.partition_point(|l| l.range.end < old_start);
        while let Some(leaf) = self.leaves.get(leaves_start)
            && leaf.range.end <= old_start
            && !(leaf.range.start == old_start && leaf.range.is_empty())
        {
            leaves_start += 1;
        }
        let leaves_end = if end < body {
            self.leaves.partition_point(|l| l.range.start < old_end)
        } else {
            self.leaves.len()
        };
        let crosses_start = (self.leaves.get(leaves_start))
            .is_some_and(|l| l.range.start < old_start && l.range.end > old_start);
        let crosses_end = (leaves_end.checked_sub(1).and_then(|i| self.leaves.get(i)))
            .is_some_and(|l| l.range.end > old_end);
        if crosses_start || crosses_end {
            return None;
        }

        let old_end_line = if end < body {
            self.lines[end]
        } else {
            self.lines.last().copied().unwrap_or(0)
        };
        Some(LocalPlan {
            blocks: first..end,
            body,
            old: old_start..old_end,
            byte_delta,
            line_delta: line_of_byte(new_end) as isize - old_end_line as isize,
            start_line: line_of_byte(old_start),
            leaves: leaves_start..leaves_end,
            local,
        })
    }

    /// 把原文的 `old_edit` 换成 `new_edit` 后，只重新解析改动所在的顶层块；返回是否已更新。
    ///
    /// 结果与全文解析（及 [`Markdown::with_blank_lines`]）逐项相同，其余块原样保留。改动可能影响
    /// 窗口之外的结构，或窗口超过 `max_window` 字节时不做改动并返回 `false`，调用方应解析全文。
    /// `new_len` 是改动后原文的字节数；`line_of_byte`、`slice` 读取改动后的原文：字节位置所在的
    /// 行（从 0 起）和一段字节范围的文字。
    pub fn reparse_local(
        &mut self,
        edit: (Range<usize>, Range<usize>),
        (new_len, max_window): (usize, usize),
        line_of_byte: impl Fn(usize) -> usize,
        slice: impl Fn(Range<usize>) -> String,
    ) -> bool {
        let Some(plan) = self.plan_local(edit, (new_len, max_window), &line_of_byte, &slice) else {
            return false;
        };
        let LocalPlan {
            blocks,
            body,
            old,
            byte_delta,
            line_delta,
            start_line,
            leaves,
            local,
        } = plan;
        let Markdown {
            blocks: new_blocks,
            hashes: new_hashes,
            lines: new_lines,
            ranges: new_ranges,
            leaves: new_leaves,
            ..
        } = local;

        // 窗口之后的块平移行号和位置；文末的脚注只平移位于窗口之后的。
        if line_delta != 0 {
            for line in &mut self.lines[blocks.end..] {
                *line = line.saturating_add_signed(line_delta);
            }
        }
        if byte_delta != 0 {
            for range in &mut self.ranges[blocks.end..body] {
                *range = shift_range(range, byte_delta);
            }
            for range in self.ranges[body..]
                .iter_mut()
                .filter(|r| r.start >= old.end)
            {
                *range = shift_range(range, byte_delta);
            }
            for leaf in &mut self.leaves[leaves.end..] {
                leaf.range = shift_range(&leaf.range, byte_delta);
            }
        }

        // 换上窗口的解析结果，位置换算为相对于全文。
        let at = old.start;
        let new_ranges = new_ranges.into_iter().map(|r| r.start + at..r.end + at);
        let new_lines = new_lines.into_iter().map(|l| l + start_line);
        let new_leaves = new_leaves.into_iter().map(|l| LeafRange {
            range: l.range.start + at..l.range.end + at,
            ..l
        });
        self.blocks.splice(blocks.clone(), new_blocks);
        self.hashes.splice(blocks.clone(), new_hashes);
        self.ranges.splice(blocks.clone(), new_ranges);
        self.lines.splice(blocks, new_lines);
        self.leaves.splice(leaves, new_leaves);
        self.len = new_len;
        true
    }
}

#[cfg(test)]
mod tests;
