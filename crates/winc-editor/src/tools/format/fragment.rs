//! 只把所选内容交给命令：截出所选的文本并去掉公共的缩进，命令的输出补回缩进后放回原处。

use super::FormatRange;

/// 从文本里截出来、交给命令的所选内容。
pub(super) struct Fragment {
    /// 交给命令的文本：所选内容去掉公共的缩进，以一个换行结尾。
    pub sent: String,
    /// 所选内容在原文里的起止位置（字节）。
    span: (usize, usize),
    /// 所选各行公共的缩进，命令的输出逐行补回。
    indent: String,
    /// 第一行的缩进是否也在所选内容里：选区从行首或缩进里开始。
    whole_first: bool,
    /// 所选内容末尾换行的个数，原样留在原文里。
    tail: usize,
}

impl Fragment {
    /// 从换行已统一为 LF 的 `text` 里截出 `range` 这一区段。
    pub fn cut(text: &str, range: &FormatRange) -> Fragment {
        let mut start = byte_at(text, range.start_offset);
        let end = byte_at(text, range.end_offset).max(start);
        let line = text[..start].rfind('\n').map_or(0, |at| at + 1);
        let whole_first = lead(&text[line..start]).len() == start - line;
        if whole_first {
            start = line;
        }
        let selected = &text[start..end];
        let body = selected.trim_end_matches('\n');
        let later = body.split('\n').skip(1).filter(|l| lead(l) != *l);
        let indent = later.fold(lead(&text[line..]), |indent, l| common(indent, lead(l)));
        let lines = body.split('\n').enumerate().map(|(at, l)| {
            let indented = at > 0 || whole_first;
            indented
                .then(|| l.strip_prefix(indent))
                .flatten()
                .unwrap_or(l)
        });
        Fragment {
            sent: lines.flat_map(|l| [l, "\n"]).collect(),
            span: (start, end),
            indent: indent.to_owned(),
            whole_first,
            tail: selected.len() - body.len(),
        }
    }

    /// 返回把命令的输出 `output` 放回所选内容的位置之后的全文；`text` 同 [`Fragment::cut`]。
    pub fn join(&self, text: &str, output: &str) -> String {
        let (start, end) = self.span;
        let output = output.trim_end_matches(['\n', '\r']);
        let mut joined = String::with_capacity(text.len() + output.len());
        joined.push_str(&text[..start]);
        for (at, line) in output.split('\n').enumerate() {
            if at > 0 {
                joined.push('\n');
            }
            if (at > 0 || self.whole_first) && !line.trim_end_matches('\r').is_empty() {
                joined.push_str(&self.indent);
            }
            joined.push_str(line);
        }
        joined.push_str(&text[end - self.tail..]);
        joined
    }
}

/// 返回 `line` 开头的空格和制表符。
fn lead(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// 返回两段缩进公共的开头。
fn common<'a>(a: &'a str, b: &str) -> &'a str {
    let same = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y);
    &a[..same.count()]
}

/// 返回 `text` 中第 `units` 个 UTF-16 码元所在的字节位置，超出时返回文本的长度。
fn byte_at(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (at, c) in text.char_indices() {
        if seen >= units {
            return at;
        }
        seen += c.len_utf16();
    }
    text.len()
}
