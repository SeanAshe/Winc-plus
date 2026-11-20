//! 段落的写作命令：设为几级标题或正文，加上或去掉引用、有序和无序列表的标记，以及插入代码块、
//! 公式块、表格、链接和图片的原文。
//!
//! 只读选中的各行，不依赖解析结果，源代码和可编辑的预览共用。

use super::enter::parse;

#[cfg(test)]
mod tests;

/// 一行改成的样子：新的文字，以及改前、改后内容（标记之后的文字）的起点（字节偏移）。
///
/// 光标在内容里时随内容移动，在标记里时移到内容的起点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 改后的文字。
    pub text: String,
    /// 改前内容的起点。
    pub old_start: usize,
    /// 改后内容的起点。
    pub new_start: usize,
}

/// 段落的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 第几级标题，0 为正文。
    Heading(u8),
    /// 引用。
    Quote,
    /// 有序列表。
    Ordered,
    /// 无序列表。
    Bullet,
    /// 任务列表。
    Task,
}

/// 把各行 `lines`（不含换行符）改成 `kind`；各行已经都是这一种时改回正文（标题、列表）、去掉
/// 一层（引用）或去掉任务框（任务列表）。空行不变。
pub fn change(lines: &[&str], kind: Kind) -> Vec<Line> {
    let filled = || lines.iter().filter(|l| !l.trim().is_empty());
    let all = match kind {
        Kind::Heading(0) => false,
        Kind::Heading(level) => filled().all(|l| heading(parse(l).rest).0 == level),
        Kind::Quote => filled().all(|l| !parse(l).quote.is_empty()),
        Kind::Task => filled().all(|l| parse(l).marker.is_some_and(|m| m.task)),
        Kind::Ordered | Kind::Bullet => filled().all(|l| {
            parse(l)
                .marker
                .is_some_and(|m| m.ordered() == (kind == Kind::Ordered))
        }),
    };
    let mut number = 0;
    lines
        .iter()
        .map(|&line| {
            if line.trim().is_empty() {
                let at = line.len();
                return Line {
                    text: line.to_owned(),
                    old_start: at,
                    new_start: at,
                };
            }
            let prefix = parse(line);
            // 不是列表项时缩进留在内容前面，改动写在缩进之后。
            let body = prefix.rest.trim_start_matches([' ', '\t']);
            let lead = &line[..line.len() - body.len()];
            let (head, rest) = match kind {
                Kind::Heading(level) => {
                    let level = if all { 0 } else { level };
                    let marks = "#".repeat(usize::from(level));
                    let gap = if level > 0 { " " } else { "" };
                    (format!("{lead}{marks}{gap}"), heading(body).1)
                }
                Kind::Quote if all => (
                    unquote(prefix.quote) + &line[prefix.quote.len()..lead.len()],
                    body,
                ),
                Kind::Quote => (format!("> {lead}"), body),
                // 列表项保留原来的记号，其余的行改为无序列表项。
                Kind::Task => {
                    let item = match prefix.marker.as_ref() {
                        Some(m) => {
                            format!("{}{}{}{}", prefix.quote, prefix.indent, m.bullet, m.gap)
                        }
                        None => format!("{lead}- "),
                    };
                    let task = match prefix.marker.as_ref() {
                        _ if all => "",
                        Some(m) if m.checked => "[x] ",
                        _ => "[ ] ",
                    };
                    (format!("{item}{task}"), body)
                }
                Kind::Ordered | Kind::Bullet => {
                    let marker = prefix.marker.as_ref();
                    let base = match marker {
                        Some(_) => format!("{}{}", prefix.quote, prefix.indent),
                        None => lead.to_owned(),
                    };
                    let bullet = if all {
                        String::new()
                    } else if kind == Kind::Ordered {
                        number += 1;
                        format!("{number}. ")
                    } else {
                        "- ".to_owned()
                    };
                    let task = match marker {
                        Some(m) if m.task && !all && m.checked => "[x] ",
                        Some(m) if m.task && !all => "[ ] ",
                        _ => "",
                    };
                    (format!("{base}{bullet}{task}"), body)
                }
            };
            let old_start = line.len() - rest.len();
            Line {
                text: format!("{head}{rest}"),
                old_start,
                new_start: head.len(),
            }
        })
        .collect()
}

/// `text` 开头标题的级别（不是标题为 0）和标记之后的内容。
fn heading(text: &str) -> (u8, &str) {
    let marks = text.bytes().take_while(|&b| b == b'#').count();
    let after = &text[marks..];
    if (1..=6).contains(&marks) && (after.is_empty() || after.starts_with([' ', '\t'])) {
        (marks as u8, after.trim_start_matches([' ', '\t']))
    } else {
        (0, text)
    }
}

/// 去掉引用标记 `quote` 中最外面的一层。
fn unquote(quote: &str) -> String {
    let Some(at) = quote.find('>') else {
        return quote.to_owned();
    };
    let rest = &quote[at + 1..];
    let rest = rest.strip_prefix([' ', '\t']).unwrap_or(rest);
    format!("{}{rest}", &quote[..at])
}

/// 围栏块的原文：`fence` 是开、闭围栏（`` ``` ``、`$$`），`content` 是块中的各行（可以为空）。
///
/// 返回原文和光标的位置（字节偏移）：代码块在开围栏之后，可以接着写语言；公式块在内容末尾。
pub fn fenced(fence: &str, content: &str) -> (String, usize) {
    let text = format!("{fence}\n{content}\n{fence}");
    let caret = if fence.starts_with('`') {
        fence.len()
    } else {
        fence.len() + 1 + content.len()
    };
    (text, caret)
}

/// 两列的表格：表头一行和一行内容，单元格都空着；光标在第一个单元格里。
pub fn table() -> (String, usize) {
    ("|  |  |\n| --- | --- |\n|  |  |".to_owned(), 2)
}

/// 链接或图片（`image`）的原文：文字是 `text`，地址是 `url`。
///
/// 返回原文和之后的选区（字节偏移）：有地址时光标在末尾；没有地址时有文字则光标在括号里，
/// 没有文字则光标在方括号里。
pub fn link(text: &str, url: Option<&str>, image: bool) -> (String, std::ops::Range<usize>) {
    let bang = if image { "!" } else { "" };
    let url = url.map(|u| {
        if u.contains(['(', ')', ' ']) {
            format!("<{u}>")
        } else {
            u.to_owned()
        }
    });
    let out = format!("{bang}[{text}]({})", url.as_deref().unwrap_or(""));
    let at = match (&url, text.is_empty()) {
        (Some(_), _) => out.len(),
        (None, false) => out.len() - 1,
        (None, true) => bang.len() + 1,
    };
    (out, at..at)
}

/// 链接地址要补全的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// 链接到的文件。
    File,
    /// 图片文件。
    Image,
    /// 本文的标题（地址以 `#` 开头）。
    Anchor,
}

/// 返回光标之前的一行文字 `before` 所在链接或图片地址的开头（字节偏移）和补全对象；地址是
/// 网址或位于行内代码中时为 `None`。
///
/// 地址里还没有空白、`)` 和 `<`：写成 `<…>` 的地址和带标题的地址不补全。
pub fn link_target(before: &str) -> Option<(usize, Target)> {
    let open = before.rfind("](")?;
    let start = open + 2;
    let target = &before[start..];
    if target.contains(|c: char| c.is_whitespace() || matches!(c, ')' | '<' | '>')) {
        return None;
    }
    let url = ["http:", "https:", "mailto:", "www."];
    if url.iter().any(|p| target.starts_with(p)) {
        return None;
    }
    if before[..open].matches('`').count() % 2 == 1 {
        return None;
    }
    let bracket = before[..open].rfind('[')?;
    let image = before[..bracket].ends_with('!');
    let kind = match (target.starts_with('#'), image) {
        (true, false) => Target::Anchor,
        (true, true) => return None,
        (false, true) => Target::Image,
        (false, false) => Target::File,
    };
    Some((start, kind))
}

/// 返回光标之前的一行文字 `before` 中呼出命令列表的 `/` 的位置（字节偏移）。
///
/// `/` 位于行首或空白之后，其后到光标之间没有空白；位于链接地址或行内代码中时为 `None`。
pub fn slash(before: &str) -> Option<usize> {
    let at = before.rfind('/')?;
    let query = &before[at + 1..];
    let head = &before[..at];
    if query.contains(char::is_whitespace) || query.chars().count() > SLASH_QUERY {
        return None;
    }
    let free = head.is_empty() || head.ends_with(char::is_whitespace);
    let code = head.matches('`').count() % 2 == 1;
    (free && !code && link_target(before).is_none()).then_some(at)
}

/// `/` 之后已输入的命令名的最大长度（字符）：超过时不再视为命令。
const SLASH_QUERY: usize = 24;

/// 返回文中的 ATX 标题（不含围栏代码块中的）：级别、文字和锚点。
///
/// 文字和锚点取自只含这些标题的解析结果（见 [`super::Markdown::headings`]），与预览相同。
pub fn headings<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<(u8, String, String)> {
    let mut found = String::new();
    let mut fence: Option<(u8, usize)> = None;
    for line in lines {
        let line = line.trim_end_matches(['\n', '\r']);
        let body = line.trim_start_matches(' ');
        if line.len() - body.len() > 3 {
            continue;
        }
        let mark = body.bytes().next().filter(|b| matches!(b, b'`' | b'~'));
        if let Some(mark) = mark {
            let run = body.bytes().take_while(|&b| b == mark).count();
            if run >= 3 {
                fence = match fence {
                    Some((m, n)) if m == mark && run >= n => None,
                    None => Some((mark, run)),
                    open => open,
                };
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        if heading(body).0 > 0 {
            found.push_str(body);
            found.push_str("\n\n");
        }
    }
    super::parse(&found).headings()
}
