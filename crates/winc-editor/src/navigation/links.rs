//! 文本中的链接：找出一行中某一列所在的网址、Markdown 链接或维基链接。

use std::ops::Range;

#[cfg(test)]
mod tests;

/// 链接指向哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 网址，或 Markdown 链接括号中的地址（网址、相对路径、`#标题`）。
    Address(String),
    /// 维基链接 `[[名字]]` 中的名字，可以带 `#标题`，不含 `|` 之后的显示文字。
    Wiki(String),
}

/// 一行中的一处链接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// 链接在这一行中的范围（字符，从 0 起）。
    pub cols: Range<usize>,
    /// 链接指向哪里。
    pub target: Target,
}

/// 返回 `line` 中第 `col` 列（字符，从 0 起）所在的链接。
///
/// 各种文件中都识别 `http://`、`https://` 开头的网址；`markdown` 为真时还识别
/// `[文字](地址)` 和 `[[名字]]`，范围是整个写法。
pub fn link_at(line: &str, col: usize, markdown: bool) -> Option<Link> {
    let chars: Vec<char> = line.chars().collect();
    if col >= chars.len() {
        return None;
    }
    let found = (markdown.then(|| wiki_at(&chars, col)).flatten())
        .or_else(|| markdown.then(|| inline_at(&chars, col)).flatten())
        .or_else(|| url_at(&chars, col));
    found.filter(|link| match &link.target {
        Target::Address(address) | Target::Wiki(address) => !address.is_empty(),
    })
}

/// 返回 `chars` 中从 `from` 起第一处 `pattern` 的位置。
fn find(chars: &[char], from: usize, pattern: &[char]) -> Option<usize> {
    let tail = chars.get(from..)?;
    let at = tail.windows(pattern.len()).position(|w| w == pattern)?;
    Some(from + at)
}

/// 返回包住第 `col` 列的 `[[名字]]`。
fn wiki_at(chars: &[char], col: usize) -> Option<Link> {
    let mut from = 0;
    while let Some(open) = find(chars, from, &['[', '[']) {
        let close = find(chars, open + 2, &[']', ']'])?;
        if (open..close + 2).contains(&col) {
            let inner: String = chars[open + 2..close].iter().collect();
            let name = inner.split('|').next().unwrap_or_default().trim();
            return Some(Link {
                cols: open..close + 2,
                target: Target::Wiki(name.to_owned()),
            });
        }
        from = close + 2;
    }
    None
}

/// 返回包住第 `col` 列的 `[文字](地址)`；图片 `![说明](地址)` 的范围不含开头的 `!`。
fn inline_at(chars: &[char], col: usize) -> Option<Link> {
    let mut from = 0;
    while let Some(middle) = find(chars, from, &[']', '(']) {
        from = middle + 2;
        // 向前找与 `]` 配对的 `[`，向后找与 `(` 配对的 `)`。
        let mut depth = 0;
        let open = (0..middle).rev().find(|&i| match chars[i] {
            ']' => {
                depth += 1;
                false
            }
            '[' if depth > 0 => {
                depth -= 1;
                false
            }
            '[' => true,
            _ => false,
        });
        let mut depth = 0;
        let close = (middle + 2..chars.len()).find(|&i| match chars[i] {
            '(' => {
                depth += 1;
                false
            }
            ')' if depth > 0 => {
                depth -= 1;
                false
            }
            ')' => true,
            _ => false,
        });
        let (Some(open), Some(close)) = (open, close) else {
            continue;
        };
        if !(open..=close).contains(&col) {
            continue;
        }
        // 地址之后可以有空白和标题；尖括号中的地址可以含空格。
        let inner: String = chars[middle + 2..close].iter().collect();
        let inner = inner.trim();
        let address = match inner.strip_prefix('<') {
            Some(rest) => rest.split('>').next().unwrap_or_default(),
            None => inner.split_whitespace().next().unwrap_or_default(),
        };
        return Some(Link {
            cols: open..close + 1,
            target: Target::Address(address.to_owned()),
        });
    }
    None
}

/// 返回包住第 `col` 列的 `http://` 或 `https://` 网址。
///
/// 网址到空白、引号、尖括号、反引号或非 ASCII 字符为止；末尾的标点和没有配对的右括号不计入。
fn url_at(chars: &[char], col: usize) -> Option<Link> {
    let ends = |c: char| c.is_whitespace() || !c.is_ascii() || "\"'<>`".contains(c);
    let mut from = 0;
    while let Some(scheme) = find(chars, from, &[':', '/', '/']) {
        from = scheme + 3;
        let start = ["https", "http"].iter().find_map(|name| {
            let start = scheme.checked_sub(name.len())?;
            let word: String = chars[start..scheme].iter().collect();
            let boundary = start == 0 || !chars[start - 1].is_ascii_alphanumeric();
            (word.eq_ignore_ascii_case(name) && boundary).then_some(start)
        });
        let Some(start) = start else {
            continue;
        };
        let mut end = (scheme + 3..chars.len())
            .find(|&i| ends(chars[i]))
            .unwrap_or(chars.len());
        while end > scheme + 3 {
            let last = chars[end - 1];
            let count = |c: char| chars[start..end].iter().filter(|&&x| x == c).count();
            let extra = |open: char, close: char| last == close && count(close) > count(open);
            let unpaired = extra('(', ')') || extra('[', ']');
            if ".,;:!?*_~".contains(last) || unpaired {
                end -= 1;
            } else {
                break;
            }
        }
        if (start..end).contains(&col) && end > scheme + 3 {
            return Some(Link {
                cols: start..end,
                target: Target::Address(chars[start..end].iter().collect()),
            });
        }
    }
    None
}
