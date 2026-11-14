//! EPUB 各章的 XHTML 转换成 Markdown：按元素的语义转换，不应用样式表。
//!
//! 标题、段落、引用、列表、表格、预排版文字、图片和行内样式对应到 Markdown 的块与行内 HTML；
//! 其余元素只保留其中的文字。目录和书中的链接指向的元素在所在段落前写一个锚点（`<a id>`），
//! 链接改写为 `#锚点`。每章开头另有一个锚点，指向一章（不带片段标识）的链接转到这里。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use roxmltree::Node;
use winc_i18n::t;

use super::{Book, Contents, LocalName, Package, TocEntry, locate, parse_text};
use crate::convert::{escape, html_attr, link_target, previewable_image};
use crate::markdown::{is_cjk, named_entity};
use crate::zip::Archive;

/// 按结构转换的最大元素嵌套层数：更深的元素只取其中的文字，防止伪造的文件嵌套过深时递归
/// 耗尽栈。正常的书不超过二三十层。
const MAX_DEPTH: usize = 64;

/// 转换书脊中的各章，按阅读顺序连成一份 Markdown，并把目录指向的位置换成锚点。
pub(super) fn convert(archive: &Archive<'_>, package: &Package, contents: &Contents) -> Book {
    let texts: Vec<Option<String>> = (package.spine.iter())
        .map(|path| archive.read(path)?.ok().and_then(|bytes| readable(&bytes)))
        .collect();
    let docs: Vec<_> = (texts.iter())
        .map(|text| text.as_deref().and_then(parse_text))
        .collect();
    let mut targets = Targets::new(&package.spine, &docs);
    for (_, _, target) in contents {
        if let Some((path, fragment)) = target {
            targets.mark(path, fragment.as_deref());
        }
    }
    for (file, doc) in package.spine.iter().zip(&docs) {
        let links = doc.iter().flat_map(|d| d.descendants());
        for href in links
            .filter(|n| n.is_element())
            .filter_map(|n| n.attribute("href"))
        {
            let (path, fragment) = locate(file, href);
            targets.mark(&path, fragment.as_deref());
        }
    }
    let mut writer = Writer::new(archive, &targets);
    for (index, (file, doc)) in package.spine.iter().zip(&docs).enumerate() {
        writer.chapter(index, file, doc.as_ref());
    }
    let contents = (contents.iter())
        .map(|(title, level, target)| TocEntry {
            title: title.clone(),
            level: *level,
            anchor: target
                .as_ref()
                .and_then(|(path, fragment)| targets.find(path, fragment.as_deref())),
        })
        .collect();
    Book {
        markdown: writer.finish(),
        media: writer.media,
        contents,
    }
}

/// 把 XHTML 文件 `bytes` 转成可以解析的文字：按 UTF-8（或带字节序标记的 UTF-16）解码，XML
/// 未定义的 HTML 命名实体（`&nbsp;` 等）换成数字字符引用，单独的 `&` 写成 `&amp;`；无法解码时
/// 返回 `None`。
pub(super) fn readable(bytes: &[u8]) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| u16::from_le_bytes(p));
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .ok()?
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| u16::from_be_bytes(p));
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .ok()?
    } else {
        let text = std::str::from_utf8(bytes).ok()?;
        text.strip_prefix('\u{feff}').unwrap_or(text).to_owned()
    };
    if !text.contains('&') {
        return Some(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let name_len = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .filter(|&n| n > 0 && rest[n..].starts_with(';'));
        let Some(n) = name_len else {
            out.push_str(if rest.starts_with('#') { "&" } else { "&amp;" });
            continue;
        };
        let name = &rest[..n];
        match name {
            "amp" | "lt" | "gt" | "quot" | "apos" => out.push_str(&format!("&{name};")),
            _ => match named_entity(name) {
                Some(c) => out.push_str(&format!("&#{};", u32::from(c))),
                None => out.push_str(&format!("&amp;{name};")),
            },
        }
        rest = &rest[n + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// 链接和目录指向的位置与锚点。
///
/// 锚点只为被指向的元素生成：每章开头为 `c<章号>`，章中的元素为 `c<章号>-<序号>`。锚点不用
/// 元素原来的 `id`：原值可能含有空格以外的任意字符，各章之间也可能重名。
struct Targets {
    /// 各章在包中的路径到章号（从 0 起）。
    chapters: HashMap<String, usize>,
    /// 各章中元素的 `id`（和 `<a name>`）。
    ids: Vec<HashSet<String>>,
    /// 各章中被指向的元素的 `id` 到锚点。
    named: Vec<HashMap<String, String>>,
}

impl Targets {
    fn new(spine: &[String], docs: &[Option<roxmltree::Document<'_>>]) -> Targets {
        let mut chapters = HashMap::new();
        for (index, path) in spine.iter().enumerate() {
            chapters.entry(path.clone()).or_insert(index);
        }
        let ids = (docs.iter())
            .map(|doc| {
                let nodes = doc.iter().flat_map(|d| d.descendants());
                nodes.filter_map(element_id).map(str::to_owned).collect()
            })
            .collect();
        Targets {
            chapters,
            ids,
            named: vec![HashMap::new(); spine.len()],
        }
    }

    /// 记下 `path` 中片段标识为 `fragment` 的位置被指向：为该元素分配锚点。
    fn mark(&mut self, path: &str, fragment: Option<&str>) {
        let Some(&chapter) = self.chapters.get(path) else {
            return;
        };
        let Some(id) = fragment.filter(|f| self.ids[chapter].contains(*f)) else {
            return;
        };
        let named = &mut self.named[chapter];
        let next = named.len();
        named
            .entry(id.to_owned())
            .or_insert_with(|| format!("c{chapter}-{next}"));
    }

    /// `path` 中片段标识为 `fragment` 的位置的锚点：元素不存在或没有片段标识时为该章的开头；
    /// `path` 不是书中的一章时为 `None`。
    fn find(&self, path: &str, fragment: Option<&str>) -> Option<String> {
        let chapter = *self.chapters.get(path)?;
        let named = fragment.and_then(|f| self.named[chapter].get(f));
        Some(named.cloned().unwrap_or_else(|| format!("c{chapter}")))
    }
}

/// 元素 `node` 的 `id`（`<a>` 还可以是 `name`）；不是元素或没有时为 `None`。
fn element_id<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    if !node.is_element() {
        return None;
    }
    let name = || {
        node.has_tag_name_local("a")
            .then(|| node.attribute("name"))
            .flatten()
    };
    node.attribute("id")
        .or_else(name)
        .filter(|id| !id.is_empty())
}

/// 逐章写出 Markdown 的状态。
struct Writer<'b, 'a> {
    archive: &'b Archive<'a>,
    targets: &'b Targets,
    /// 正在转换的一章：章号和在包中的路径。
    chapter: usize,
    file: String,
    /// 写好的各块，最后以空行连接。
    blocks: Vec<String>,
    /// 正在收集的一段行内文字（Markdown 与行内 HTML）。
    inline: String,
    /// 这一段中是否已有可显示的内容（文字或图片）。
    visible: bool,
    /// 这一段中最后写下的字，决定之后的空白是否写成空格；段首为 `None`。
    last: Option<char>,
    /// 前面有尚未写下的空白。
    space: bool,
    /// 尚未写下的空白中有换行。
    newline: bool,
    /// 正在写标题：标题只能占一行，其中不分段。
    heading: bool,
    /// 这一段中尚未结束的行内标签：段落在标签中间结束时先关闭，下一段开头重新打开。
    open: Vec<&'static str>,
    /// 尚未落到段落或标题上的锚点。
    anchors: Vec<String>,
    /// 图片：Markdown 中引用的地址（包中的路径）和图片文件的内容。
    media: Vec<(String, Arc<[u8]>)>,
}

impl<'b, 'a> Writer<'b, 'a> {
    fn new(archive: &'b Archive<'a>, targets: &'b Targets) -> Self {
        Writer {
            archive,
            targets,
            chapter: 0,
            file: String::new(),
            blocks: Vec::new(),
            inline: String::new(),
            visible: false,
            last: None,
            space: false,
            newline: false,
            heading: false,
            open: Vec::new(),
            anchors: Vec::new(),
            media: Vec::new(),
        }
    }

    /// 写出第 `index` 章（包中的路径为 `file`）；`doc` 为 `None` 时该章无法读取，写一句说明。
    fn chapter(&mut self, index: usize, file: &str, doc: Option<&roxmltree::Document<'_>>) {
        self.chapter = index;
        self.file = file.to_owned();
        self.anchors.push(format!("c{index}"));
        let Some(doc) = doc else {
            let name = file.rsplit('/').next().unwrap_or(file).to_owned();
            let mut text = String::new();
            escape(&t!("preview-epub-chapter-damaged", name = name), &mut text);
            self.raw(&format!("<i>{text}</i>"));
            self.visible = true;
            self.flush("");
            return;
        };
        let root = doc.root_element();
        let body = root.children().find(|n| n.has_tag_name_local("body"));
        self.walk(body.unwrap_or(root), 0);
        self.flush("");
    }

    /// 以空行连接写好的各块。
    fn finish(&mut self) -> String {
        let mut markdown = self.blocks.join("\n\n");
        markdown.push('\n');
        markdown
    }

    /// 转换 `node` 的各个子节点；`depth` 为 `node` 的嵌套层数。
    fn walk(&mut self, node: Node<'_, '_>, depth: usize) {
        for child in node.children() {
            if child.is_text() {
                self.text(child.text().unwrap_or_default());
            } else if child.is_element() {
                self.element(child, depth + 1);
            }
        }
    }

    /// 转换元素 `node`。
    fn element(&mut self, node: Node<'_, '_>, depth: usize) {
        if let Some(id) = element_id(node)
            && let Some(anchor) = self.targets.named[self.chapter].get(id)
        {
            self.anchors.push(anchor.clone());
        }
        if depth > MAX_DEPTH {
            let nodes = node.descendants().filter(Node::is_text);
            let text: String = nodes.filter_map(|n| n.text()).collect();
            self.text(&text);
            return;
        }
        let name = node.tag_name().name().to_ascii_lowercase();
        match name.as_str() {
            "head" | "script" | "style" | "title" | "template" | "rt" | "rp" => {}
            "br" => {
                self.raw("<br>");
                (self.last, self.space) = (None, false);
            }
            "hr" => {
                self.flush("");
                self.blocks.push("---".to_owned());
            }
            "img" => {
                let alt = node.attribute("alt").unwrap_or_default();
                if let Some(src) = node.attribute("src") {
                    self.image(src, alt);
                }
            }
            // SVG 中的位图（常见于封面）：地址写在 `href` 或 `xlink:href`。
            "image" => {
                if let Some(href) = node.attributes().find(|a| a.name() == "href") {
                    self.image(href.value(), "");
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = usize::from(name.as_bytes()[1] - b'0');
                self.flush("");
                self.heading = true;
                self.walk(node, depth);
                self.heading = false;
                self.flush(&format!("{} ", "#".repeat(level)));
            }
            "blockquote" => self.quote(node, depth),
            "ul" | "ol" => self.list(node, depth, name == "ol"),
            "pre" => self.pre(node),
            "table" => self.table(node, depth),
            "a" => self.link(node, depth),
            "b" | "strong" => self.styled(node, depth, "b"),
            "i" | "em" | "cite" | "dfn" | "var" => self.styled(node, depth, "i"),
            "u" | "ins" => self.styled(node, depth, "u"),
            "s" | "del" | "strike" => self.styled(node, depth, "s"),
            "sup" => self.styled(node, depth, "sup"),
            "sub" => self.styled(node, depth, "sub"),
            "code" | "kbd" | "samp" | "tt" => self.styled(node, depth, "code"),
            "small" => self.styled(node, depth, "small"),
            "mark" => self.styled(node, depth, "mark"),
            other if is_block(other) => {
                self.flush("");
                self.walk(node, depth);
                self.flush("");
            }
            _ => self.walk(node, depth),
        }
    }

    /// 写一段文字：连续的空白合并成一个空格，段首的空白和两个中日韩文字之间的换行不写。
    ///
    /// 由纯文本转换来的书常把多段放在一个段落元素中，每段另起一行、以全角空格缩进：换行后
    /// 紧接全角空格时在此分段，缩进保留。
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                self.newline |= c == '\n';
                self.space = true;
                continue;
            }
            if std::mem::take(&mut self.newline) && c == '\u{3000}' && self.visible && !self.heading
            {
                self.flush("");
            }
            if std::mem::take(&mut self.space)
                && let Some(last) = self.last
                && !(is_cjk(last) && is_cjk(c))
            {
                self.inline.push(' ');
            }
            let mut buf = [0; 4];
            escape(c.encode_utf8(&mut buf), &mut self.inline);
            self.last = Some(c);
            self.visible = true;
        }
    }

    /// 原样写入行内 HTML `html`。
    fn raw(&mut self, html: &str) {
        if std::mem::take(&mut self.space) && self.last.is_some() {
            self.inline.push(' ');
        }
        self.inline.push_str(html);
    }

    /// 结束正在收集的一段，以 `prefix`（标题的 `# `，段落为空）开头写成一块，锚点写在最前面。
    ///
    /// 没有可显示内容的段落不写，其中的锚点留给下一段。
    fn flush(&mut self, prefix: &str) {
        let reopen: String = self.open.iter().map(|tag| format!("<{tag}>")).collect();
        let inline = std::mem::replace(&mut self.inline, reopen);
        let visible = std::mem::take(&mut self.visible);
        (self.last, self.space, self.newline) = (None, false, false);
        if !visible {
            return;
        }
        let mut block = prefix.to_owned();
        for anchor in self.anchors.drain(..) {
            block.push_str(&format!("<a id=\"{anchor}\"></a>"));
        }
        // 只去掉 ASCII 空白：段首的全角空格是缩进。
        let content = inline.trim_matches(|c: char| c.is_ascii_whitespace());
        // 标题只能占一行：其中的换行写成空格。
        if prefix.is_empty() {
            block.push_str(content);
        } else {
            block.push_str(&content.replace("<br>", " "));
        }
        for tag in self.open.iter().rev() {
            block.push_str(&format!("</{tag}>"));
        }
        self.blocks.push(block);
    }

    /// 在 `node` 的内容两侧写行内标签 `tag`。
    fn styled(&mut self, node: Node<'_, '_>, depth: usize, tag: &'static str) {
        self.raw(&format!("<{tag}>"));
        self.open.push(tag);
        self.walk(node, depth);
        self.open.pop();
        self.inline.push_str(&format!("</{tag}>"));
    }

    /// 写链接：书中的位置改写为 `#锚点`，网址和邮件地址原样保留，其余（书外的文件）只写文字。
    fn link(&mut self, node: Node<'_, '_>, depth: usize) {
        let target = node.attribute("href").and_then(|href| {
            let (path, fragment) = locate(&self.file, href);
            if path.is_empty() {
                let web = ["http://", "https://", "mailto:"];
                let lower = href.to_ascii_lowercase();
                web.iter()
                    .any(|p| lower.starts_with(p))
                    .then(|| href.to_owned())
            } else {
                let anchor = self.targets.find(&path, fragment.as_deref())?;
                Some(format!("#{anchor}"))
            }
        });
        let Some(url) = target else {
            self.walk(node, depth);
            return;
        };
        self.raw(&format!("<a href=\"{}\">", html_attr(&link_target(&url))));
        self.open.push("a");
        self.walk(node, depth);
        self.open.pop();
        self.inline.push_str("</a>");
    }

    /// 写包中 `src`（相对当前章）处的图片；书外的图片和无法预览的格式只写替代文字。
    fn image(&mut self, src: &str, alt: &str) {
        let (path, _) = locate(&self.file, src);
        let ext = (path.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        let known = self.media.iter().any(|(url, _)| *url == path);
        if !path.is_empty()
            && previewable_image(&ext)
            && !known
            && let Some(Ok(bytes)) = self.archive.read(&path)
        {
            self.media.push((path.clone(), bytes.into()));
        }
        if !self.media.iter().any(|(url, _)| *url == path) {
            self.text(alt);
            return;
        }
        let src = html_attr(&link_target(&path));
        self.raw(&format!("<img src=\"{src}\" alt=\"{}\">", html_attr(alt)));
        self.visible = true;
    }

    /// 把 `node` 的内容写成单独的若干块并返回，不放进当前的块列表。
    fn nested(&mut self, node: Node<'_, '_>, depth: usize) -> Vec<String> {
        self.flush("");
        let outer = std::mem::take(&mut self.blocks);
        self.walk(node, depth);
        self.flush("");
        std::mem::replace(&mut self.blocks, outer)
    }

    /// 写引用：各行前加 `> `。
    fn quote(&mut self, node: Node<'_, '_>, depth: usize) {
        let inner = self.nested(node, depth);
        if inner.is_empty() {
            return;
        }
        let text = inner.join("\n\n");
        let lines: Vec<String> = (text.lines())
            .map(|line| match line {
                "" => ">".to_owned(),
                line => format!("> {line}"),
            })
            .collect();
        self.blocks.push(lines.join("\n"));
    }

    /// 写列表：每项的第一行前加记号，其余各行缩进到记号之后；有一项多于一块时各项之间空一行。
    fn list(&mut self, node: Node<'_, '_>, depth: usize, ordered: bool) {
        self.flush("");
        let start: u64 = (node.attribute("start"))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        let mut items = Vec::new();
        let mut loose = false;
        for item in node.children().filter(Node::is_element) {
            let inner = self.nested(item, depth + 1);
            if inner.is_empty() {
                continue;
            }
            loose |= inner.len() > 1;
            let marker = match ordered {
                true => format!("{}. ", start.saturating_add(items.len() as u64)),
                false => "- ".to_owned(),
            };
            let indent = " ".repeat(marker.len());
            let text = inner.join("\n\n");
            let mut lines = text.lines();
            let mut out = format!("{marker}{}", lines.next().unwrap_or_default());
            for line in lines {
                out.push('\n');
                if !line.is_empty() {
                    out.push_str(&indent);
                    out.push_str(line);
                }
            }
            items.push(out);
        }
        if !items.is_empty() {
            self.blocks
                .push(items.join(if loose { "\n\n" } else { "\n" }));
        }
    }

    /// 写预排版文字：围栏代码块，围栏比文字中最长的一串反引号多一个（至少三个）。
    fn pre(&mut self, node: Node<'_, '_>) {
        self.flush("");
        let mut text = String::new();
        for n in node.descendants() {
            if n.has_tag_name_local("br") {
                text.push('\n');
            } else if let Some(t) = n.text().filter(|_| n.is_text()) {
                text.push_str(t);
            }
        }
        let text = text.trim_matches('\n').trim_end();
        if text.is_empty() {
            return;
        }
        let longest = (text.split(|c| c != '`')).map(str::len).max().unwrap_or(0);
        let fence = "`".repeat((longest + 1).max(3));
        self.blocks.push(format!("{fence}\n{text}\n{fence}"));
    }

    /// 写表格：第一行作表头，各格的内容连成一行（块之间用 `<br>`）；只有一格的表格（常用于
    /// 排版边框）直接写出其中的内容。
    fn table(&mut self, node: Node<'_, '_>, depth: usize) {
        self.flush("");
        let mut rows: Vec<Vec<String>> = Vec::new();
        let groups = node.children().filter(|n| {
            ["thead", "tbody", "tfoot"]
                .iter()
                .any(|g| n.has_tag_name_local(g))
        });
        let row_nodes = (node.children().chain(groups.flat_map(|g| g.children())))
            .filter(|n| n.has_tag_name_local("tr"));
        for tr in row_nodes {
            let mut row = Vec::new();
            for cell in tr
                .children()
                .filter(|n| n.has_tag_name_local("td") || n.has_tag_name_local("th"))
            {
                let span: usize = (cell.attribute("colspan"))
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                let text = self.nested(cell, depth + 1).join("<br>").replace('\n', " ");
                row.push(text);
                row.extend(std::iter::repeat_n(String::new(), span - 1));
            }
            rows.push(row);
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        if rows.len() == 1 && columns == 1 {
            let cell = rows.remove(0).remove(0);
            if !cell.is_empty() {
                self.blocks.push(cell.replace("<br>", "\n\n"));
            }
            return;
        }
        let mut out = String::new();
        for (k, row) in rows.iter().enumerate() {
            out.push('|');
            for c in 0..columns {
                out.push(' ');
                out.push_str(row.get(c).map_or("", String::as_str));
                out.push_str(" |");
            }
            out.push('\n');
            if k == 0 {
                out.push('|');
                out.push_str(&" --- |".repeat(columns));
                out.push('\n');
            }
        }
        out.pop();
        self.blocks.push(out);
    }
}

/// 名为 `name`（小写）的 HTML 元素是否是块级元素：前后分段。
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "body"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "header"
            | "hgroup"
            | "li"
            | "main"
            | "nav"
            | "p"
            | "section"
            | "summary"
    )
}
