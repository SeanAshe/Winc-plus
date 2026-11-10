//! 文件里写的 HTML：按 GFM 中 README 的常见用法识别一部分标签（见 [`crate::markdown`] 的说明），
//! 其余标签移除，只保留文字。

use pulldown_cmark::TagEnd;

use super::{Align, Block, Builder, Container, Image, Inline, Leaf, Length, SourceMap, Style};

impl Builder<'_> {
    /// `</details>`：最里面未关闭的容器是 `<details>` 时结束它，生成一块。
    pub(super) fn end_details(&mut self) {
        if !matches!(self.stack.last(), Some(Container::Details(..))) {
            return;
        }
        if let Some(Container::Details(open, summary, blocks)) = self.stack.pop() {
            let summary = summary.unwrap_or_else(|| {
                let mut inline = Inline::default();
                inline.push("Details", Style::default());
                inline
            });
            self.push_container(Block::Details {
                open,
                summary,
                blocks,
            });
        }
    }

    /// 处理一段 HTML：`block` 为真时是整个 HTML 块（包括其中的文字），否则是行内的一个标签。
    ///
    /// 注释被移除；文字中的连续空白合并成一个空格，并解码 HTML 字符实体。
    pub(super) fn html(&mut self, html: &str, block: bool) {
        let mut rest = html;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix("<!--") {
                rest = after.find("-->").map_or("", |end| &after[end + 3..]);
                continue;
            }
            if block && let Some(after) = self.pre(rest) {
                rest = after;
                continue;
            }
            if rest.starts_with('<')
                && let Some(end) = rest.find('>')
            {
                self.tag(&rest[1..end], block);
                rest = &rest[end + 1..];
                continue;
            }
            // 文字到下一个 `<` 为止（行内 HTML 的文字不在这里，由 pulldown-cmark 当普通文字给出）。
            // 跳过开头的一个字（可能是 `<`，也可能是多字节的汉字）再查找。
            let first = rest.chars().next().map_or(1, char::len_utf8);
            let end = rest[first..].find('<').map_or(rest.len(), |i| i + first);
            let text = collapse(&decode_entities(&rest[..end]));
            if !text.trim().is_empty() || self.inline.is_some() {
                self.text(&text, 0);
            }
            rest = &rest[end..];
        }
    }

    /// 处理一个 HTML 标签，`tag` 是尖括号里的内容。
    fn tag(&mut self, tag: &str, block: bool) {
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        match (name.as_str(), closing) {
            ("br", _) => self.text("\n", 0),
            ("img", false) => {
                let attr = |key: &str| attribute(tag, key);
                let size =
                    |key: &str| attr(key).and_then(|v| v.trim_end_matches("px").parse().ok());
                // 图片来自 picture 时：其中的 source 声明的浅色地址优先于 img 的地址。
                let (light, dark) = &mut self.sources;
                let image = Image {
                    url: light.take().or_else(|| attr("src")).unwrap_or_default(),
                    alt: attr("alt").unwrap_or_default(),
                    width: attr("width").and_then(|v| length(&v)),
                    height: size("height"),
                    dark: dark.take(),
                };
                self.push_image(image);
            }
            ("a", false) => match attribute(tag, "href") {
                Some(href) => self.start_link(href),
                None => self.push_style(0, None),
            },
            ("source", false) => {
                // `srcset` 可能写了多张（带宽度、倍数），取第一张。
                let url = attribute(tag, "srcset").and_then(|s| {
                    s.split([',', ' '])
                        .find(|u| !u.is_empty())
                        .map(str::to_owned)
                });
                let media = attribute(tag, "media").unwrap_or_default();
                if media.contains("dark") {
                    self.sources.1 = url;
                } else if media.contains("light") {
                    self.sources.0 = url;
                }
            }
            ("picture", true) => self.sources = (None, None),
            ("sup", false) => self.push_style(Style::SUP, None),
            ("sub", false) => self.push_style(Style::SUB, None),
            ("sup" | "sub", true) => {
                self.styles.pop();
            }
            ("b" | "strong", false) => self.push_style(Style::STRONG, None),
            ("i" | "em", false) => self.push_style(Style::EMPHASIS, None),
            ("s" | "del" | "strike", false) => self.push_style(Style::STRIKE, None),
            ("code" | "kbd" | "tt" | "samp", false) => self.push_style(Style::CODE, None),
            ("u" | "ins", false) => self.push_style(Style::UNDERLINE, None),
            ("mark", false) => self.push_style(Style::MARK, None),
            ("small", false) => self.push_style(Style::SMALL, None),
            (
                "a" | "b" | "strong" | "i" | "em" | "s" | "del" | "strike" | "code" | "kbd" | "tt"
                | "samp" | "u" | "ins" | "mark" | "small",
                true,
            ) => {
                self.styles.pop();
            }
            (heading, false) if block && is_heading(heading) => {
                let level = heading.as_bytes()[1] - b'0';
                self.start_leaf(Leaf::Heading(level));
                self.push_align(tag, false);
            }
            (heading, true) if block && is_heading(heading) => {
                self.finish_html_leaf();
                self.aligns.pop();
            }
            ("summary", false) if block => self.start_leaf(Leaf::Summary),
            ("summary", true) if block => self.finish_html_leaf(),
            ("details", false) if block => {
                self.finish_html_leaf();
                let open = attribute(tag, "open").is_some() || has_flag(tag, "open");
                self.stack.push(Container::Details(open, None, Vec::new()));
            }
            ("details", true) if block => {
                self.finish_html_leaf();
                self.end_details();
            }
            ("p" | "div" | "center", closing) if block => {
                // 单元格里分段：另起一行，单元格不结束。
                if self.in_cell() {
                    if !closing && !self.inline().text.trim().is_empty() {
                        self.text("\n", 0);
                    }
                } else {
                    self.finish_html_leaf();
                }
                if closing {
                    self.aligns.pop();
                } else {
                    self.push_align(tag, name == "center");
                }
            }
            ("table", false) if block => {
                self.finish_html_leaf();
                self.stack
                    .push(Container::Table(Vec::new(), Vec::new(), Vec::new(), false));
            }
            ("table", true) if block => {
                self.end_html_row();
                if matches!(self.stack.last(), Some(Container::Table(..))) {
                    self.end(TagEnd::Table);
                }
            }
            ("tr", _) if block => self.end_html_row(),
            ("td" | "th", false) if block => {
                self.end_html_cell();
                let header = name == "th";
                let Some(Container::Table(aligns, rows, row, head)) = self.stack.last_mut() else {
                    return;
                };
                // 第一行定下各列的对齐，第一格是 `<th>` 时第一行是表头。
                if rows.is_empty() {
                    if row.is_empty() {
                        *head = header;
                    }
                    let align = match attribute(tag, "align").as_deref() {
                        Some("center") => Align::Center,
                        Some("right") => Align::Right,
                        Some("left") => Align::Left,
                        _ => Align::None,
                    };
                    aligns.resize(row.len(), Align::None);
                    aligns.push(align);
                }
                self.start_leaf(Leaf::Cell);
                if header {
                    self.push_style(Style::STRONG, None);
                    self.cell_style = true;
                }
            }
            ("td" | "th", true) if block => self.end_html_cell(),
            ("ul" | "ol", false) if block => {
                self.finish_html_leaf();
                let start = (name == "ol").then(|| {
                    attribute(tag, "start")
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(1)
                });
                self.stack.push(Container::List(start, Vec::new()));
            }
            ("ul" | "ol", true) if block => {
                self.end_html_item();
                if matches!(self.stack.last(), Some(Container::List(..))) {
                    self.end(TagEnd::List(false));
                }
            }
            ("li", false) if block => {
                self.end_html_item();
                if matches!(self.stack.last(), Some(Container::List(..))) {
                    self.stack.push(Container::Item(None, Vec::new()));
                }
            }
            ("li", true) if block => self.end_html_item(),
            ("blockquote", false) if block => {
                self.finish_html_leaf();
                self.stack.push(Container::Quote(None, Vec::new()));
            }
            ("blockquote", true) if block => {
                self.finish_html_leaf();
                if matches!(self.stack.last(), Some(Container::Quote(..))) {
                    self.end(TagEnd::BlockQuote(None));
                }
            }
            ("hr", _) if block => {
                self.finish_html_leaf();
                self.push_block(Block::Rule(None));
            }
            // 定义列表：被定义的词加粗成一段，解释另起一段。
            ("dt", false) if block => {
                self.finish_html_leaf();
                self.start_leaf(Leaf::Paragraph);
                self.push_style(Style::STRONG, None);
                self.cell_style = true;
            }
            ("dt" | "dd" | "dl", _) if block => {
                self.finish_html_leaf();
                if std::mem::take(&mut self.cell_style) {
                    self.styles.pop();
                }
            }
            _ => {}
        }
    }

    /// 进入 HTML 标签 `tag` 后内容的对齐：取 `align` 属性，`center` 标签总是居中。
    ///
    /// 没有写 `align` 属性时沿用外层的对齐。
    fn push_align(&mut self, tag: &str, center: bool) {
        let outer = self.aligns.last().copied().unwrap_or_default();
        let align = match attribute(tag, "align")
            .map(|a| a.to_ascii_lowercase())
            .as_deref()
        {
            _ if center => Align::Center,
            Some("center") => Align::Center,
            Some("right") => Align::Right,
            Some("left") => Align::Left,
            _ => outer,
        };
        self.aligns.push(align);
    }

    /// 处理 `rest` 开头的 `pre` 元素（代码块），返回其后的文本；不是代码块时返回 `None`。
    ///
    /// 代码内容不合并空白，只移除其中的标签。语言取 `pre` 或其中 `code` 的 `class` 属性。
    fn pre<'a>(&mut self, rest: &'a str) -> Option<&'a str> {
        let name = rest.get(..4)?;
        let after = rest[4..].chars().next()?;
        if !name.eq_ignore_ascii_case("<pre") || !(after == '>' || after.is_whitespace()) {
            return None;
        }
        let open = rest.find('>')?;
        let close = rest
            .to_ascii_lowercase()
            .find("</pre>")
            .unwrap_or(rest.len());
        let body = &rest[open + 1..close.max(open + 1)];
        let mut lang = attribute(&rest[1..open], "class");
        let mut code = String::new();
        let mut s = body;
        while let Some(i) = s.find('<') {
            code.push_str(&s[..i]);
            let Some(end) = s[i..].find('>') else {
                s = "";
                break;
            };
            if lang.is_none() {
                lang = attribute(&s[i + 1..i + end], "class");
            }
            s = &s[i + end + 1..];
        }
        code.push_str(s);
        let code = decode_entities(&code);
        let code = code.strip_prefix('\n').unwrap_or(&code);
        let lang = lang
            .and_then(|c| {
                c.split_whitespace()
                    .find_map(|c| c.strip_prefix("language-").or(c.strip_prefix("lang-")))
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        self.finish_html_leaf();
        self.push_block(Block::Code {
            lang,
            text: code.trim_end_matches('\n').to_owned(),
            map: SourceMap::default(),
        });
        Some(rest.get(close + "</pre>".len()..).unwrap_or_default())
    }

    /// HTML 表格的一格结束（`</td>`，或者没写 `</td>` 就开始了下一格、下一行）。
    fn end_html_cell(&mut self) {
        if self.in_cell() {
            self.finish_html_leaf();
        }
        if std::mem::take(&mut self.cell_style) {
            self.styles.pop();
        }
    }

    /// HTML 表格的一行结束（`</tr>`、下一个 `<tr>`、`</table>`）。
    fn end_html_row(&mut self) {
        self.end_html_cell();
        self.finish_html_leaf();
        if let Some(Container::Table(_, rows, row, _)) = self.stack.last_mut()
            && !row.is_empty()
        {
            rows.push(std::mem::take(row));
        }
    }

    /// HTML 列表的一项结束（`</li>`，或者没写 `</li>` 就开始了下一项、列表结束）。
    fn end_html_item(&mut self) {
        self.finish_html_leaf();
        if matches!(self.stack.last(), Some(Container::Item(..))) {
            self.end(TagEnd::Item);
        }
    }

    /// HTML 块里一段（段落、标题）结束：去掉首尾空白再收下。
    fn finish_html_leaf(&mut self) {
        if let Some((leaf, inline)) = self.inline.take() {
            self.inline = Some((leaf, inline.trim()));
        }
        self.finish_inline();
    }
}

fn is_heading(name: &str) -> bool {
    matches!(name.as_bytes(), [b'h', b'1'..=b'6'])
}

/// 取 HTML 标签 `tag`（尖括号里的内容）中属性 `key` 的值；引号可有可无。
fn attribute(tag: &str, key: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(key) {
        let at = from + i;
        from = at + key.len();
        // 属性名前面必须是空白，后面（跳过空白）必须是 `=`。
        if !lower[..at].ends_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let Some(value) = tag[from..].trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let value = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or_default(),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or_default(),
        };
        return Some(decode_entities(value));
    }
    None
}

/// 把 HTML 里的连续空白（包括换行）合成一个空格。
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// 换掉常见的 HTML 字符实体和数字实体。
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                "copy" => '©',
                "reg" => '®',
                "mdash" => '—',
                "ndash" => '–',
                "hellip" => '…',
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML 标签 `tag` 里是否写了不带值的属性 `name`（如 `<details open>`）。
fn has_flag(tag: &str, name: &str) -> bool {
    tag.split(|c: char| c.is_whitespace() || c == '/')
        .skip(1)
        .any(|word| word.eq_ignore_ascii_case(name))
}

/// HTML 的宽度属性：像素或百分比。
fn length(value: &str) -> Option<Length> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent: f32 = percent.trim().parse().ok()?;
        return (percent > 0.0)
            .then(|| Length::Percent((percent.min(100.0) * 100.0).round() as u32));
    }
    let px: f32 = value.trim_end_matches("px").trim().parse().ok()?;
    (px >= 1.0).then(|| Length::Px(px.round() as u32))
}
