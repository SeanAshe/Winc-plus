//! 按文件内容识别缩进：用制表符还是空格，是空格时每级几格。

use std::ops::ControlFlow;

use crate::text::Lines;

/// 识别缩进时最多扫描的行数（行）。
///
/// 缩进风格在文件开头已足以判断，大文件无需读取更多行。2000 取自调参，超过这个行数的部分不参与
/// 判断。
const SCAN_LINES: usize = 2000;
/// 识别缩进时每行最多读取的前缀长度（字节）。
///
/// 缩进只出现在行首，行的其余部分不影响判断。128 大于常见文件的缩进深度。
const PREFIX_BYTES: usize = 128;

/// 一级缩进的写法：制表符或固定数量的空格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    /// 用制表符缩进。
    Tabs,
    /// 用空格缩进，值为每级的空格数（个）。
    ///
    /// [`detect`] 只返回 2 至 8；其他取值由调用方构造，[`Indent::unit`] 会夹取。
    Spaces(usize),
}

impl Indent {
    /// 返回按一次 Tab 键插入的缩进文本。
    ///
    /// `Spaces(n)` 的空格数夹取到 1 至 8。
    pub fn unit(self) -> &'static str {
        const SPACES: &str = "        ";
        match self {
            Indent::Tabs => "\t",
            Indent::Spaces(n) => &SPACES[..n.clamp(1, SPACES.len())],
        }
    }

    /// 由缩进文本反推缩进写法，用于把语言定义的缩进单位转换成 [`Indent`]。
    ///
    /// 以制表符开头时返回 `Tabs`；否则按字节数取空格数，夹取到 1 至 8。
    pub fn of_unit(unit: &str) -> Indent {
        if unit.starts_with('\t') {
            Indent::Tabs
        } else {
            Indent::Spaces(unit.len().clamp(1, 8))
        }
    }
}

/// 返回识别出的缩进写法；无法判断时返回 `None`。
///
/// 只扫描开头的 `SCAN_LINES` 行，每行只读取前 `PREFIX_BYTES` 字节。以制表符缩进的行多于以空格
/// 缩进的行时返回 `Tabs`。
pub fn detect(text: &impl Lines) -> Option<Indent> {
    let (mut tab_lines, mut space_lines) = (0usize, 0usize);
    // `deltas[d]` 是相邻两个非空行的缩进相差 `d` 个空格的次数，`d` 取 2 至 8。
    let mut deltas = [0usize; 9];
    // 上一个以空格缩进（或没有缩进）的非空行的缩进空格数。
    let mut previous: Option<usize> = None;
    let mut smallest = usize::MAX;

    text.visit_lines(0, PREFIX_BYTES, |line, _, content| {
        if line >= SCAN_LINES {
            return ControlFlow::Break(());
        }

        let rest = content.trim_start_matches([' ', '\t']);
        // 空行不参与统计，也不打断前后两个非空行的比较。
        if rest.is_empty() {
            return ControlFlow::Continue(());
        }
        let lead = &content[..content.len() - rest.len()];

        if lead.starts_with('\t') {
            tab_lines += 1;
            previous = None;
            return ControlFlow::Continue(());
        }
        if lead.contains('\t') {
            // 前导空白混用空格和 Tab 时无法判断，跳过该行，并中断与下一行的比较。
            previous = None;
            return ControlFlow::Continue(());
        }

        let width = lead.len();
        // 块注释中 ` * ` 之前的奇数个对齐空格不是缩进，跳过。
        if rest.starts_with('*') && width % 2 == 1 {
            return ControlFlow::Continue(());
        }
        if width > 0 {
            space_lines += 1;
            smallest = smallest.min(width);
        }
        if let Some(before) = previous {
            let delta = width.abs_diff(before);
            if (2..=8).contains(&delta) {
                deltas[delta] += 1;
            }
        }
        previous = Some(width);
        ControlFlow::Continue(())
    });

    if tab_lines == 0 && space_lines == 0 {
        return None;
    }
    if tab_lines > space_lines {
        return Some(Indent::Tabs);
    }

    // 取出现次数最多的差值；次数相同时取较小者，因为每级 `n` 个空格的文件中，
    // 一次退两级的差值是 `2n`。
    let best = (2..=8).max_by_key(|&d| (deltas[d], std::cmp::Reverse(d)))?;
    if deltas[best] > 0 {
        return Some(Indent::Spaces(best));
    }

    // 没有可用的差值（例如所有缩进行都是 4 个空格）时，取最小的缩进宽度。
    (2..=8)
        .contains(&smallest)
        .then_some(Indent::Spaces(smallest))
}

#[cfg(test)]
mod tests;
