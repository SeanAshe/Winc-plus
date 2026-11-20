//! 改写图片原文中写明的大小：`![alt](src)` 改为带宽度的 `<img>`，`<img>` 改写宽度属性。

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::convert::html_attr;

/// 写入 `<img>` 的宽度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// 逻辑像素（等于 CSS 像素）。
    Px(u32),
    /// 占行宽的百分比。
    Percent(u32),
}

/// 把 `text` 开头的图片改为宽度 `width`；`width` 为 `None` 时去掉写明的大小。
///
/// `text` 从 `![` 或 `<img` 起，到所在顶层块的末尾；`standalone` 表示图片位于顶层块开头、
/// 不在 HTML 块中，此时去掉大小后的 `<img>` 改为 `![alt](src)`。返回要替换的字节范围（相对
/// `text`）和新文字；无法识别或无需修改时返回 `None`。
pub fn resize(
    text: &str,
    width: Option<Width>,
    standalone: bool,
) -> Option<(Range<usize>, String)> {
    if text.starts_with("![") {
        return markdown(text, width);
    }
    let at = img_tags(text).next().filter(|at| at.start == 0)?;
    html(text, at, width, standalone)
}

/// 改写 `![alt](src "title")`：写宽度时改为 `<img>`，去掉大小时只去掉替代文字末尾的 `|W`。
fn markdown(text: &str, width: Option<Width>) -> Option<(Range<usize>, String)> {
    let (mut alt, mut range, mut found) = (String::new(), 0..0, None);
    for (event, at) in Parser::new_ext(text, super::options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) if at.start == 0 => {
                range = at;
                found = Some((dest_url.into_string(), title.into_string()));
            }
            Event::Text(t) | Event::Code(t) if found.is_some() => alt.push_str(&t),
            Event::End(TagEnd::Image) if found.is_some() => break,
            _ if found.is_none() && at.start > 0 => return None,
            _ => {}
        }
    }
    let (src, title) = found?;
    let raw_alt = alt_source(&text[range.clone()])?;
    let width = match width {
        Some(width) => width,
        None => {
            let (kept, ..) = split_alt_size(&text[raw_alt.clone()])?;
            return Some((raw_alt.start..raw_alt.end, kept.to_owned()));
        }
    };
    let alt = split_alt_size(&alt).map_or(alt.as_str(), |(alt, ..)| alt);
    let mut out = format!(
        "<img src=\"{}\" alt=\"{}\"",
        html_attr(&src),
        html_attr(alt)
    );
    if !title.is_empty() {
        out.push_str(&format!(" title=\"{}\"", html_attr(&title)));
    }
    out.push_str(&format!(" width=\"{}\">", value(width)));
    Some((range, out))
}

/// 返回 `![alt](…)` 中替代文字的字节范围。
fn alt_source(image: &str) -> Option<Range<usize>> {
    let mut depth = 0usize;
    let mut chars = image.char_indices().skip(2);
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => depth += 1,
            ']' if depth == 0 => return Some(2..i),
            ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// 依次返回 `text` 中各个 `<img>` 标签的字节范围（含尖括号）。
fn img_tags(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    std::iter::from_fn(move || {
        loop {
            let at = from + lower[from..].find("<img")?;
            from = at + 4;
            if !lower[from..].starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
                continue;
            }
            let end = tag_end(text, from)?;
            from = end;
            return Some(at..end);
        }
    })
}

/// 返回从 `from` 起引号之外第一个 `>` 之后的位置。
fn tag_end(text: &str, from: usize) -> Option<usize> {
    let mut quote = None;
    for (i, c) in text[from..].char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(from + i + 1),
            _ => {}
        }
    }
    None
}

/// 改写 `<img>` 标签 `text[tag]`：去掉 `width`、`height` 属性和 `style` 中的大小，写宽度时
/// 加上 `width` 属性；只剩 `src`、`alt`、`title` 且 `standalone` 时改为 `![alt](src)`。
fn html(
    text: &str,
    tag: Range<usize>,
    width: Option<Width>,
    standalone: bool,
) -> Option<(Range<usize>, String)> {
    let raw = &text[tag.clone()];
    let inner_end = raw.len() - 1 - usize::from(raw[..raw.len() - 1].trim_end().ends_with('/'));
    let inner = &raw[4..inner_end];
    let close = &raw[inner_end..];
    let mut kept = String::new();
    let (mut src, mut alt, mut title, mut plain) = (None, String::new(), None, true);
    for attr in attributes(inner) {
        let name = attr.name.to_ascii_lowercase();
        match name.as_str() {
            "width" | "height" => continue,
            "style" => {
                let style = without_size(attr.value.as_deref().unwrap_or_default());
                if !style.is_empty() {
                    kept.push_str(&format!(" style=\"{}\"", html_attr(&style)));
                    plain = false;
                }
                continue;
            }
            "src" => src = attr.value.clone(),
            "alt" => alt = attr.value.clone().unwrap_or_default(),
            "title" => title = attr.value.clone(),
            _ => plain = false,
        }
        kept.push(' ');
        kept.push_str(attr.raw.trim());
    }
    let new = match width {
        Some(width) => format!(
            "<img{kept} width=\"{}\"{}",
            value(width),
            close.trim_start()
        ),
        None if plain && standalone => {
            let src = super::html::decode_entities(&src?);
            let alt = super::html::decode_entities(&alt);
            let title = title.map(|t| super::html::decode_entities(&t));
            image_markdown(&alt, &src, title.as_deref())
        }
        None => format!("<img{kept}{}", close.trim_start()),
    };
    (new != raw).then_some((tag, new))
}

/// HTML 属性：原文、名字和值（已去掉引号，实体未解码）。
struct Attribute<'a> {
    raw: &'a str,
    name: &'a str,
    value: Option<String>,
}

/// 依次读出 `<img` 与 `>` 之间 `inner` 中的各个属性。
fn attributes(inner: &str) -> Vec<Attribute<'_>> {
    let bytes = inner.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !b"=/>".contains(&bytes[i]) {
            i += 1;
        }
        let name = &inner[name_start..i];
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = None;
        if bytes.get(j) == Some(&b'=') {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let (v, end) = match bytes.get(j) {
                Some(&q @ (b'"' | b'\'')) => {
                    let end = inner[j + 1..]
                        .find(q as char)
                        .map_or(inner.len(), |k| j + 1 + k);
                    (&inner[j + 1..end], (end + 1).min(inner.len()))
                }
                _ => {
                    let end = inner[j..]
                        .find(|c: char| c.is_whitespace())
                        .map_or(inner.len(), |k| j + k);
                    (&inner[j..end], end)
                }
            };
            value = Some(v.to_owned());
            i = end;
        }
        if name.is_empty() {
            // 跳过不构成属性名的字符（如单独的 `/`）。
            i = i.max(start + 1);
            continue;
        }
        out.push(Attribute {
            raw: &inner[start..i],
            name,
            value,
        });
    }
    out
}

/// 去掉 CSS 声明 `style` 中的 `width`、`height`、`zoom`。
fn without_size(style: &str) -> String {
    let kept: Vec<&str> = style
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .filter(|d| {
            let name = d
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            !matches!(name.as_str(), "width" | "height" | "zoom")
        })
        .collect();
    kept.join("; ")
}

fn value(width: Width) -> String {
    match width {
        Width::Px(px) => px.to_string(),
        Width::Percent(p) => format!("{p}%"),
    }
}

/// 写成 `![alt](src "title")`，`alt` 中的方括号和反斜杠、`title` 中的引号和反斜杠转义。
fn image_markdown(alt: &str, src: &str, title: Option<&str>) -> String {
    let mut out = String::from("![");
    for c in alt.chars() {
        if matches!(c, '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push_str("](");
    out.push_str(&crate::convert::destination(src));
    if let Some(title) = title {
        let title = title.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(" \"{title}\""));
    }
    out.push(')');
    out
}

/// 取出替代文字末尾的尺寸（`|300`、`|300x200`），并从替代文字中去掉。
pub(super) fn alt_size(image: &mut super::Image) {
    if let Some((alt, w, h)) = split_alt_size(&image.alt) {
        image.width = Some(super::Length::Px(w));
        image.height = h;
        image.alt = alt.to_owned();
    }
}

/// 拆分替代文字末尾的 `|W` 或 `|WxH`，返回其前的文字、宽和高；
/// 没有尺寸或宽、高为 0 时返回 `None`。
pub(super) fn split_alt_size(alt: &str) -> Option<(&str, u32, Option<u32>)> {
    let (alt, size) = alt.rsplit_once('|')?;
    let (w, h) = match size.split_once('x') {
        Some((w, h)) => (w, Some(h)),
        None => (size, None),
    };
    let w = w.trim().parse::<u32>().ok().filter(|&w| w > 0)?;
    let h = match h.map(|h| h.trim().parse::<u32>()) {
        Some(Ok(h)) if h > 0 => Some(h),
        Some(_) => return None,
        None => None,
    };
    Some((alt.trim_end(), w, h))
}

#[cfg(test)]
mod tests;
