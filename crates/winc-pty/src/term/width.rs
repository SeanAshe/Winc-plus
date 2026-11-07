//! 字符在屏幕上占几格。
//!
//! 单个字符的显示宽度来自 `unicode-width`。emoji 序列按字素簇计算，因为全屏重绘的命令行程序多数
//! 按字素簇排版：带 VS16 的 emoji、肤色修饰和 ZWJ 序列都占两格，否则程序算出的行宽比终端少，行尾
//! 折行后光标位置全部错开。代价是仍按单字符计算宽度的 shell 遇到这些序列时光标可能错位。
//!
//! 字素簇只覆盖 emoji：其余组合字符本来就不占格，无需合并。仿真器逐字符写入时用 `Cluster` 判断，
//! 读取一行文字时用 [`cells`] 判断，两者的规则相同。

use std::iter::Peekable;
use std::str::Chars;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 变体选择符 16：要求按 emoji 显示。
const VS16: char = '\u{fe0f}';
/// 零宽连接符：把前后的 emoji 连成一个字素簇。
const ZWJ: char = '\u{200d}';

/// 返回字符 `c` 单独占的格数：0（组合字符、控制字符）、1 或 2。
pub fn char_width(c: char) -> usize {
    if c.is_ascii() {
        // 可打印 ASCII 占一格；控制字符不会走到这里（解析器已经分出去了），按 0 算。
        return usize::from(c >= ' ' && c != '\x7f');
    }

    c.width().unwrap_or(0).min(2)
}

/// 一个字符写入时要做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// 占新的格子，参数是格数。
    Advance(usize),
    /// 并入前一个字，不占格。
    Combine,
    /// 并入前一个字，并把它从一格加宽到两格。
    Widen,
}

/// 逐字符判断 emoji 字素簇的状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Cluster {
    /// 前一个字不能继续组成 emoji。
    #[default]
    None,
    /// 前一个字占一格，后面跟 VS16 时可能加宽。
    Narrow(char),
    /// 正处于占两格的 emoji 之后。
    Emoji,
    /// 正处于 emoji 后的 ZWJ 之后，下一个字连入该 emoji。
    Joining,
}

impl Cluster {
    /// 返回写入 `c` 时要做的事，并更新状态。
    pub(crate) fn next(&mut self, c: char) -> Step {
        match (*self, c) {
            (Cluster::Narrow(base), VS16) if widens(base) => {
                *self = Cluster::Emoji;
                Step::Widen
            }
            (Cluster::Emoji, ZWJ) => {
                *self = Cluster::Joining;
                Step::Combine
            }
            (Cluster::Joining, _) if char_width(c) > 0 => {
                *self = Cluster::Emoji;
                Step::Combine
            }
            (Cluster::Emoji, '\u{1f3fb}'..='\u{1f3ff}') => Step::Combine,
            _ => match char_width(c) {
                0 => Step::Combine,
                1 => {
                    *self = Cluster::Narrow(c);
                    Step::Advance(1)
                }
                width => {
                    *self = if is_emoji(c) {
                        Cluster::Emoji
                    } else {
                        Cluster::None
                    };
                    Step::Advance(width)
                }
            },
        }
    }
}

/// 占一格的字 `base` 后面跟 VS16 时是否按两格显示；数据来自 `unicode-width` 的 emoji 表示序列表。
fn widens(base: char) -> bool {
    let mut buf = [0; 8];
    let len = base.encode_utf8(&mut buf).len();
    VS16.encode_utf8(&mut buf[len..]);
    std::str::from_utf8(&buf[..len + VS16.len_utf8()]).is_ok_and(|text| text.width() == 2)
}

/// 占两格的字 `c` 是否属于 emoji 所在的区段；汉字等宽字符后的 ZWJ 不连接下一个字。
fn is_emoji(c: char) -> bool {
    matches!(
        c,
        '\u{231a}'..='\u{23ff}' | '\u{25fd}'..='\u{27bf}' | '\u{2b00}'..='\u{2bff}'
            | '\u{1f000}'..='\u{1faff}'
    )
}

/// `c` 是否是字素簇里不单独绘制的部分：渲染器不合成 emoji 序列，连接符和连入的 emoji 不画。
///
/// 只对 [`cells`] 给出宽度 0 的字符有意义；普通的组合字符（重音符号）单独宽度也是 0，仍然绘制。
pub fn is_emoji_tail(c: char) -> bool {
    matches!(c, VS16 | ZWJ) || char_width(c) > 0
}

/// 文字中的每个字符和它占的格数，组合进前一个字的字符占 0 格；规则同 `Cluster`。
pub fn cells(text: &str) -> Cells<'_> {
    Cells {
        chars: text.chars().peekable(),
        cluster: Cluster::None,
    }
}

/// [`cells`] 返回的迭代器。
pub struct Cells<'a> {
    chars: Peekable<Chars<'a>>,
    cluster: Cluster,
}

impl Iterator for Cells<'_> {
    type Item = (char, usize);

    fn next(&mut self) -> Option<(char, usize)> {
        let c = self.chars.next()?;
        let width = match self.cluster.next(c) {
            Step::Advance(1) if self.chars.peek() == Some(&VS16) && widens(c) => 2,
            Step::Advance(width) => width,
            Step::Combine | Step::Widen => 0,
        };
        Some((c, width))
    }
}

#[cfg(test)]
mod tests;
