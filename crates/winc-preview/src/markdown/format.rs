//! 可编辑的预览中行内格式的切换：为选中的文字添加或移除粗体、斜体、行内代码、删除线的标记。
//!
//! 只读取光标所在的一行，不依赖解析结果。

use std::ops::Range;

#[cfg(test)]
mod tests;

/// 切换格式要做的修改：将一行中的 `range` 替换为 `text`，之后选中 `select`。
///
/// 位置均为该行中的字节偏移；`select` 按修改之后的文字计算。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toggle {
    /// 被替换的范围。
    pub range: Range<usize>,
    /// 替换成的文字。
    pub text: String,
    /// 修改之后的选区；起止相同时为光标。
    pub select: Range<usize>,
}

/// 返回在 `line`（不含换行符）中对选区 `selection` 切换标记 `mark` 要做的修改。
///
/// `mark` 是 `**`、`*`、`` ` `` 或 `~~`。选中的文字两侧或内部两端已有该标记时移除，否则添加；
/// 添加时标记紧贴文字，选区两端的空格留在标记之外。没有选区时：光标在一对空标记中间则移除
/// 这对标记，光标之后紧跟标记则移到标记之后，否则插入一对标记并将光标放在中间。
///
/// # Panics
///
/// `mark` 为空，或 `selection` 不在 `line` 的字符边界上时 panic。
pub fn toggle(line: &str, selection: Range<usize>, mark: &str) -> Toggle {
    let (c, n) = (mark.as_bytes()[0], mark.len());
    let run = |text: &str, back: bool| {
        let same = |b: &u8| *b == c;
        if back {
            text.bytes().rev().take_while(same).count()
        } else {
            text.bytes().take_while(same).count()
        }
    };
    // 连续的标记字符是否含有该格式：一个 `*` 是斜体，两个是粗体，三个两者都有。
    let marked = |count: usize| if n == 1 { count % 2 == 1 } else { count >= n };
    let (start, end) = (selection.start, selection.end);
    let before = marked(run(&line[..start], true));
    let after = marked(run(&line[end..], false));
    if start == end {
        return if before && after {
            Toggle {
                range: start - n..end + n,
                text: String::new(),
                select: start - n..start - n,
            }
        } else if after {
            Toggle {
                range: start..start,
                text: String::new(),
                select: start + n..start + n,
            }
        } else {
            Toggle {
                range: start..start,
                text: mark.repeat(2),
                select: start + n..start + n,
            }
        };
    }
    let inner = &line[start..end];
    if before && after {
        return Toggle {
            range: start - n..end + n,
            text: inner.to_owned(),
            select: start - n..end - n,
        };
    }
    if inner.len() >= 2 * n && marked(run(inner, false)) && marked(run(inner, true)) {
        let text = &inner[n..inner.len() - n];
        return Toggle {
            range: start..end,
            text: text.to_owned(),
            select: start..start + text.len(),
        };
    }
    let lead = inner.len() - inner.trim_start().len();
    let core = inner.trim();
    if core.is_empty() {
        return Toggle {
            range: start..end,
            text: format!("{mark}{inner}{mark}"),
            select: start + n..end + n,
        };
    }
    let tail = &inner[lead + core.len()..];
    Toggle {
        range: start..end,
        text: format!("{}{mark}{core}{mark}{tail}", &inner[..lead]),
        select: start + lead + n..start + lead + n + core.len(),
    }
}
