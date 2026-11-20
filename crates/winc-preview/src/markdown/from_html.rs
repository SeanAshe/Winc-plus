//! 把剪贴板中的 HTML 转换为 Markdown：按元素语义转换，不应用样式表。
//!
//! 容忍不配对的标签、不关闭的 `<br>`、`<img>` 和不加引号的属性；未识别的元素只保留文字，
//! `script`、`style`、`head` 等元素的内容不输出。

use crate::convert::destination;

/// 转换结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    /// Markdown 原文。
    pub text: String,
    /// 是否含有文字以外的结构（标题、列表、链接、强调、表格、代码、图片、引用）。
    pub rich: bool,
}

/// 把 HTML `html` 转换为 Markdown。
pub fn from_html(html: &str) -> Converted {
    let mut w = Writer::default();
    for token in Tokens(html) {
        match token {
            Token::Open(name, attrs) => w.open(&name, attrs),
            Token::Close(name) => w.close(&name),
            Token::Text(text) => w.text(&super::html::decode_entities(text)),
        }
    }
    w.finish()
}

/// HTML 中的一项：开始标签（小写名字与属性原文）、结束标签或文字（实体未解码）。
enum Token<'a> {
    Open(String, &'a str),
    Close(String),
    Text(&'a str),
}

/// 依次读出 HTML 中的各项；跳过注释、文档类型声明和处理指令，不构成标签的 `<` 作为文字。
struct Tokens<'a>(&'a str);

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let rest = self.0;
            if rest.is_empty() {
                return None;
            }
            let Some(at) = rest.find('<') else {
                self.0 = "";
                return Some(Token::Text(rest));
            };
            if at > 0 {
                self.0 = &rest[at..];
                return Some(Token::Text(&rest[..at]));
            }
            if let Some(comment) = rest.strip_prefix("<!--") {
                self.0 = comment.find("-->").map_or("", |end| &comment[end + 3..]);
                continue;
            }
            let close = rest.starts_with("</");
            let body = &rest[1 + usize::from(close)..];
            let name_len = body
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(body.len());
            if body.starts_with(['!', '?']) {
                self.0 = body.find('>').map_or("", |end| &body[end + 1..]);
                continue;
            }
            if name_len == 0 || !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
                self.0 = &rest[1..];
                return Some(Token::Text("<"));
            }
            let end = tag_end(body).unwrap_or(body.len());
            self.0 = body.get(end + 1..).unwrap_or("");
            let name = body[..name_len].to_ascii_lowercase();
            return Some(match close {
                true => Token::Close(name),
                false => Token::Open(name, &body[name_len..end]),
            });
        }
    }
}

/// 标签里引号之外的第一个 `>` 的位置。
fn tag_end(body: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in body.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// 返回属性 `key` 的值（实体已解码）。
fn attr(attrs: &str, key: &str) -> Option<String> {
    super::html::attribute(&format!(" {attrs}"), key)
}

/// 包含后续内容的块：引用，或列表中的一项（续行缩进 `indent` 个空格）。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Frame {
    Quote,
    List { ordered: bool, next: u32 },
    Item { indent: usize },
}

/// 行内的强调、代码或链接。
#[derive(Debug, Clone, PartialEq)]
enum Span {
    /// 强调：结束标签名和标记（`**`、`*`、`~~`）。
    Mark(String, &'static str),
    Code,
    Link(String),
    /// 不改变样式的标签（如 `<b style="font-weight:normal">`），仅用于配对。
    Plain(String),
}

#[derive(Default)]
struct Writer {
    out: String,
    rich: bool,
    stack: Vec<Frame>,
    /// 列表项第一块的记号，写出后清空。
    marker: Option<String>,
    /// 上一块是否位于列表项中：是时下一项不加空行。
    after_item: bool,
    /// 上一块续行的前缀；块之间的空行只保留两块共有的前缀。
    last_prefix: String,
    /// 尚未写出的行内文字。
    inline: String,
    spans: Vec<(Span, usize)>,
    heading: Option<usize>,
    /// 预排版文字的内容和语言。
    pre: Option<(String, String)>,
    table: Option<Table>,
    /// 位于内容不输出的元素中的层数。
    skip: usize,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    /// 当前单元格的内容在行内缓冲中的开始位置。
    cell: Option<usize>,
    /// 嵌套表格的层数：嵌套的表格只保留文字。
    nested: usize,
}

impl Writer {
    fn open(&mut self, name: &str, attrs: &str) {
        // 只按同类的开始、结束标签计层数，其中的其他标签不计入。
        if is_skipped(name) {
            self.skip += 1;
            return;
        }
        if self.skip > 0 {
            return;
        }
        if let Some((text, lang)) = self.pre.as_mut() {
            match name {
                "br" => text.push('\n'),
                "code" if lang.is_empty() => *lang = code_lang(attrs),
                _ => {}
            }
            return;
        }
        if let Some(table) = self.table.as_mut() {
            match name {
                "table" => table.nested += 1,
                "td" | "th" if table.nested == 0 => {
                    let at = self.inline.len();
                    if let Some(table) = self.table.as_mut() {
                        table.cell = Some(at);
                    }
                    return;
                }
                "br" => {
                    self.inline.push_str("<br>");
                    return;
                }
                // 单元格中的块元素只作为分隔，内容合并为一行。
                _ if is_block(name) => {
                    self.inline.push(' ');
                    return;
                }
                _ => {}
            }
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.heading = Some(usize::from(name.as_bytes()[1] - b'0'));
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "figure"
            | "figcaption" | "dt" | "dd" | "address" | "center" | "nav" | "aside" => self.flush(),
            "blockquote" => {
                self.flush();
                self.stack.push(Frame::Quote);
            }
            "ul" | "ol" => {
                self.flush();
                // 有序列表的序号最多 9 位。
                let next = attr(attrs, "start")
                    .and_then(|s| s.trim().parse().ok())
                    .filter(|&n| n <= 999_999_999)
                    .unwrap_or(1);
                let ordered = name == "ol";
                self.stack.push(Frame::List { ordered, next });
            }
            "li" => {
                self.flush();
                self.end_item();
                let list = self.stack.iter_mut().rev().find_map(|f| match f {
                    Frame::List { ordered, next } => Some((ordered, next)),
                    _ => None,
                });
                let marker = match list {
                    Some((true, next)) => {
                        let number = *next;
                        *next = (number + 1).min(999_999_999);
                        format!("{number}. ")
                    }
                    _ => "- ".to_owned(),
                };
                self.stack.push(Frame::Item {
                    indent: marker.len(),
                });
                self.marker = Some(marker);
            }
            "pre" => {
                self.flush();
                self.pre = Some((String::new(), code_lang(attrs)));
            }
            "table" => {
                self.flush();
                self.table = Some(Table::default());
            }
            "hr" => {
                self.flush();
                self.emit("---");
                self.rich = true;
            }
            "br" => self.inline.push_str("\\\n"),
            "img" => {
                let alt = attr(attrs, "alt").unwrap_or_default();
                match attr(attrs, "src").filter(|s| absolute(s)) {
                    Some(src) => {
                        let alt = escape(alt.trim());
                        self.inline
                            .push_str(&format!("![{alt}]({})", destination(&src)));
                        self.rich = true;
                    }
                    None => self.inline.push_str(&escape(&alt)),
                }
            }
            "input" if attrs.to_ascii_lowercase().contains("checkbox") => {
                let checked = attrs.to_ascii_lowercase().contains("checked");
                self.inline.push_str(if checked { "[x] " } else { "[ ] " });
            }
            "a" => {
                let href = attr(attrs, "href").unwrap_or_default();
                let at = self.inline.len();
                self.spans.push((Span::Link(href), at));
            }
            "code" | "kbd" | "samp" | "tt" => {
                let at = self.inline.len();
                self.spans.push((Span::Code, at));
            }
            "strong" | "b" | "em" | "i" | "del" | "s" | "strike" | "span" => {
                let style = attr(attrs, "style")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let style = style.replace(' ', "");
                let normal =
                    style.contains("font-weight:normal") || style.contains("font-weight:400");
                let bold = ["font-weight:bold", "font-weight:600", "font-weight:700"];
                let mark = match name {
                    "strong" | "b" if normal => None,
                    "strong" | "b" => Some("**"),
                    "em" | "i" => Some("*"),
                    "del" | "s" | "strike" => Some("~~"),
                    _ if bold.iter().any(|b| style.contains(b)) => Some("**"),
                    _ if style.contains("font-style:italic") => Some("*"),
                    _ => None,
                };
                let at = self.inline.len();
                let span = match mark {
                    Some(mark) => Span::Mark(name.to_owned(), mark),
                    None => Span::Plain(name.to_owned()),
                };
                self.spans.push((span, at));
                if let Some(mark) = mark {
                    self.inline.push_str(mark);
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        if self.skip > 0 {
            if is_skipped(name) {
                self.skip -= 1;
            }
            return;
        }
        if name == "pre"
            && let Some((text, lang)) = self.pre.take()
        {
            let text = text.trim_matches('\n');
            let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
            self.emit(&format!("{fence}{lang}\n{text}\n{fence}"));
            self.rich = true;
            return;
        }
        if self.pre.is_some() {
            return;
        }
        if let Some(table) = self.table.as_mut() {
            match name {
                "table" if table.nested > 0 => table.nested -= 1,
                "td" | "th" if table.nested == 0 => {
                    self.close_spans(0);
                    let Some(table) = self.table.as_mut() else {
                        return;
                    };
                    if let Some(at) = table.cell.take() {
                        let cell = self.inline.split_off(at.min(self.inline.len()));
                        let cell = cell.trim().replace('|', "\\|").replace("\\\n", "<br>");
                        match table.rows.last_mut() {
                            Some(row) => row.push(cell),
                            None => table.rows.push(vec![cell]),
                        }
                    }
                    return;
                }
                "tr" if table.nested == 0 => {
                    table.rows.push(Vec::new());
                    return;
                }
                "table" => {
                    let table = self.table.take().unwrap_or_default();
                    self.inline.clear();
                    self.write_table(table.rows);
                    return;
                }
                _ if is_block(name) => return,
                _ => {}
            }
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article"
            | "header" | "footer" | "main" | "figure" | "figcaption" | "dt" | "dd" | "address"
            | "center" | "nav" | "aside" => self.flush(),
            "blockquote" => {
                self.flush();
                self.end_item();
                if let Some(i) = self.stack.iter().rposition(|f| *f == Frame::Quote) {
                    self.stack.truncate(i);
                }
                self.rich = true;
                self.after_item = false;
            }
            "ul" | "ol" => {
                self.flush();
                self.end_item();
                if let Some(i) = (self.stack.iter()).rposition(|f| matches!(f, Frame::List { .. }))
                {
                    self.stack.truncate(i);
                }
                self.rich = true;
                self.after_item = self.stack.iter().any(|f| matches!(f, Frame::Item { .. }));
            }
            "li" => {
                self.flush();
                self.end_item();
            }
            _ => {
                if let Some(i) = self
                    .spans
                    .iter()
                    .rposition(|(span, _)| span_name(span) == name)
                {
                    self.close_spans(i);
                }
            }
        }
    }

    fn text(&mut self, text: &str) {
        if self.skip > 0 {
            return;
        }
        if let Some((pre, _)) = self.pre.as_mut() {
            pre.push_str(text);
            return;
        }
        let code = self.spans.iter().any(|(s, _)| *s == Span::Code);
        let mut collapsed = String::with_capacity(text.len());
        let mut space = self.inline.is_empty() || self.inline.ends_with([' ', '\n']);
        for c in text.chars() {
            if c.is_whitespace() {
                if !space {
                    collapsed.push(' ');
                    space = true;
                }
            } else {
                collapsed.push(c);
                space = false;
            }
        }
        if code {
            self.inline.push_str(&collapsed);
        } else {
            self.inline.push_str(&escape(&collapsed));
        }
    }

    /// 关闭第 `from` 层及更内层的行内样式。
    fn close_spans(&mut self, from: usize) {
        while self.spans.len() > from {
            let Some((span, at)) = self.spans.pop() else {
                break;
            };
            let at = at.min(self.inline.len());
            let inner = self.inline.split_off(at);
            match span {
                Span::Plain(_) => self.inline.push_str(&inner),
                Span::Mark(_, mark) => {
                    let inner = inner.strip_prefix(mark).unwrap_or(&inner);
                    self.wrap(inner, mark, mark);
                }
                Span::Code => {
                    let ticks = "`".repeat(longest_run(&inner, '`') + 1);
                    let pad = if inner.starts_with('`') || inner.ends_with('`') {
                        " "
                    } else {
                        ""
                    };
                    let inner = format!("{pad}{}{pad}", inner.trim());
                    if inner.trim().is_empty() {
                        continue;
                    }
                    self.wrap(&inner, &ticks, &ticks);
                }
                Span::Link(href) => {
                    let usable = absolute(&href);
                    if usable && !inner.trim().is_empty() {
                        let close = format!("]({})", destination(&href));
                        self.wrap(&inner, "[", &close);
                    } else {
                        self.inline.push_str(&inner);
                    }
                }
            }
        }
    }

    /// 用 `open`、`close` 包住 `inner`，两端空白移到标记外；只有空白时不加标记。
    fn wrap(&mut self, inner: &str, open: &str, close: &str) {
        let body = inner.trim();
        if body.is_empty() {
            self.inline.push_str(inner);
            return;
        }
        let lead = &inner[..inner.len() - inner.trim_start().len()];
        let trail = &inner[inner.trim_end().len()..];
        self.inline
            .push_str(&format!("{lead}{open}{body}{close}{trail}"));
        self.rich = true;
    }

    /// 写出行内缓冲中的文字。
    fn flush(&mut self) {
        self.close_spans(0);
        let text = std::mem::take(&mut self.inline);
        let text = trim_breaks(&text);
        let heading = self.heading.take();
        if text.is_empty() {
            return;
        }
        match heading {
            Some(level) => {
                let text = text.replace("\\\n", " ");
                self.emit(&format!("{} {text}", "#".repeat(level)));
                self.rich = true;
            }
            None => {
                let text = text.to_owned();
                self.emit(&escape_start(&text));
            }
        }
    }

    /// 结束当前列表项；没有内容的项只写出记号。
    fn end_item(&mut self) {
        if let Some(i) = self
            .stack
            .iter()
            .rposition(|f| matches!(f, Frame::Item { .. }))
            && !self.stack[i + 1..]
                .iter()
                .any(|f| matches!(f, Frame::List { .. }))
        {
            if self.marker.is_some() {
                self.emit("");
            }
            self.stack.truncate(i);
        }
    }

    /// 写出一块 `text`，每行加上所在引用、列表项的前缀。
    fn emit(&mut self, text: &str) {
        let marker = self.marker.take();
        let (mut first, mut rest) = (String::new(), String::new());
        let innermost = self
            .stack
            .iter()
            .rposition(|f| matches!(f, Frame::Item { .. }));
        for (i, frame) in self.stack.iter().enumerate() {
            match frame {
                Frame::Quote => {
                    first.push_str("> ");
                    rest.push_str("> ");
                }
                Frame::Item { indent } => {
                    let pad = " ".repeat(*indent);
                    match (&marker, Some(i) == innermost) {
                        (Some(marker), true) => first.push_str(marker),
                        _ => first.push_str(&pad),
                    }
                    rest.push_str(&pad);
                }
                Frame::List { .. } => {}
            }
        }
        if !self.out.is_empty() {
            self.out.push('\n');
            if !(marker.is_some() && self.after_item) {
                let shared = (self.last_prefix.chars().zip(rest.chars()))
                    .take_while(|(a, b)| a == b)
                    .count();
                let blank: String = rest.chars().take(shared).collect();
                self.out.push_str(blank.trim_end());
                self.out.push('\n');
            }
        }
        for (k, line) in text.split('\n').enumerate() {
            if k > 0 {
                self.out.push('\n');
            }
            let prefix = if k == 0 { &first } else { &rest };
            self.out.push_str(format!("{prefix}{line}").trim_end());
        }
        if marker.is_some() {
            self.rich = true;
        }
        self.after_item = innermost.is_some();
        self.last_prefix = rest;
    }

    fn write_table(&mut self, rows: Vec<Vec<String>>) {
        let rows: Vec<Vec<String>> = rows.into_iter().filter(|r| !r.is_empty()).collect();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        if rows.is_empty() || columns == 0 {
            return;
        }
        // 只有一格的表格作为一段文字。
        if rows.len() == 1 && columns == 1 {
            self.emit(&rows[0][0]);
            return;
        }
        let line = |row: &[String]| {
            let cells: Vec<&str> = (0..columns)
                .map(|c| row.get(c).map_or("", String::as_str))
                .collect();
            format!("| {} |", cells.join(" | "))
        };
        let mut text = line(&rows[0]);
        text.push('\n');
        text.push_str(&line(&vec!["---".to_owned(); columns]));
        for row in &rows[1..] {
            text.push('\n');
            text.push_str(&line(row));
        }
        self.emit(&text);
        self.rich = true;
    }

    fn finish(mut self) -> Converted {
        if self.pre.is_some() {
            self.close("pre");
        }
        if let Some(table) = self.table.take() {
            self.write_table(table.rows);
        }
        self.flush();
        let mut text = self.out;
        text.push('\n');
        Converted {
            text,
            rich: self.rich,
        }
    }
}

/// 是否为块元素（段落、标题、列表、引用等）。
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "h1" | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "p"
            | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "figure"
            | "figcaption"
            | "dt"
            | "dd"
            | "address"
            | "center"
            | "nav"
            | "aside"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "pre"
            | "hr"
    )
}

/// 是否为内容不输出的元素。
fn is_skipped(name: &str) -> bool {
    matches!(name, "script" | "style" | "head" | "title" | "template")
}

/// 地址 `url` 是否为绝对地址（带协议或 `mailto:`）；相对地址不写成链接和图片。
fn absolute(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    url.contains("://") || url.starts_with("mailto:")
}

fn span_name(span: &Span) -> &str {
    match span {
        Span::Mark(name, _) | Span::Plain(name) => name,
        Span::Code => "code",
        Span::Link(_) => "a",
    }
}

/// 返回 `class` 中 `language-`、`lang-` 之后的语言名。
fn code_lang(attrs: &str) -> String {
    let class = attr(attrs, "class").unwrap_or_default();
    (class.split_whitespace())
        .find_map(|c| {
            c.strip_prefix("language-")
                .or_else(|| c.strip_prefix("lang-"))
        })
        .unwrap_or_default()
        .to_owned()
}

fn longest_run(text: &str, c: char) -> usize {
    let (mut best, mut run) = (0, 0);
    for ch in text.chars() {
        run = if ch == c { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

/// 转义会被解析为标记的字符：反斜杠、`*`、反引号、方括号、`<`、`~`、`$`，以及不在词中的 `_`。
fn escape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let inside_word = |k: Option<&char>| k.is_some_and(|c| c.is_alphanumeric());
        let underscore = c == '_'
            && !(inside_word(i.checked_sub(1).and_then(|k| chars.get(k)))
                && inside_word(chars.get(i + 1)));
        if matches!(c, '\\' | '*' | '`' | '[' | ']' | '<' | '~' | '$') || underscore {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 去掉 `text` 两端的空白和换行（`\` 加换行）。
fn trim_breaks(text: &str) -> &str {
    let mut text = text.trim();
    loop {
        let trimmed = (text.strip_prefix("\\\n").map(str::trim_start)).or_else(|| {
            text.strip_suffix('\\')
                .filter(|t| !escaped(t))
                .map(str::trim_end)
        });
        match trimmed {
            Some(t) => text = t,
            None => return text,
        }
    }
}

/// `text` 末尾是否有奇数个反斜杠，即紧随其后的字符被转义。
fn escaped(text: &str) -> bool {
    text.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1
}

/// 段首像标题、引用或列表记号时在前面加反斜杠。
fn escape_start(text: &str) -> String {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let listy = (digits > 0 && text[digits..].starts_with(". "))
        || ["- ", "+ ", "> ", "#"].iter().any(|p| text.starts_with(p));
    match listy {
        true => format!("\\{text}"),
        false => text.to_owned(),
    }
}

#[cfg(test)]
mod tests;
