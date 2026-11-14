//! 读 EPUB 电子书，按阅读顺序把各章转换成一份 Markdown 文本，并读出书的目录。
//!
//! 书是一个 zip 压缩包：`META-INF/container.xml` 指向包文档（OPF），其中的清单列出各文件，书脊
//! 给出阅读顺序；目录来自 EPUB 3 的导航文档或 EPUB 2 的 NCX。各章的 XHTML 在 `xhtml` 模块中
//! 转换，不应用样式表。目录和书中的链接指向的位置转换成锚点（`<a id>`），预览中可以转到。
//! 格式依据 EPUB 3.3 与 OPF 2.0.1 规范。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use roxmltree::{Document, Node, ParsingOptions};
use winc_i18n::t;

use crate::ooxml::resolve;
use crate::zip::{self, Archive};

mod xhtml;

/// 能读的 EPUB 文件的大小上限（字节）：与 docx、xlsx 预览相同，三者都把整个压缩包读入内存。
const MAX_FILE: u64 = crate::table::MAX_FILE;

/// 目录最多列出的条数：再多在侧栏中也无法逐条查看，防止伪造的文件让列表无限增长。
const MAX_ENTRIES: usize = 10_000;

/// 目录最多的层数：正常的书不超过六七层，防止伪造的文件嵌套过深时递归耗尽栈。
const MAX_LEVELS: usize = 32;

/// 只用于混淆字体的加密算法：IDPF 与 Adobe 的字体混淆。其余算法表示内容受数字版权保护。
const FONT_OBFUSCATION: [&str; 2] = [
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

/// 转换结果：Markdown 文本、其中引用的包内图片，以及书的目录。
#[derive(Debug, Default)]
pub struct Book {
    /// 转换得到的 Markdown 文本。
    pub markdown: String,
    /// 图片：Markdown 中引用的地址（包中的路径）和图片文件的内容。
    pub media: Vec<(String, Arc<[u8]>)>,
    /// 目录，按书中的顺序排列。
    pub contents: Vec<TocEntry>,
}

/// 目录中的一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    /// 标题。
    pub title: String,
    /// 层级：顶层为 0。
    pub level: usize,
    /// 指向的锚点（Markdown 中 `<a id>` 的值）；指向书外或找不到目标时为 `None`。
    pub anchor: Option<String>,
}

/// `path` 是否是 EPUB 电子书（按扩展名判断）。
pub fn is_epub(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"))
}

/// 读取 EPUB 文件 `path` 并转换成 Markdown。应在后台线程调用：读入整个文件并逐章转换。
///
/// # Errors
///
/// 文件无法读取、超过大小上限（256 MiB）、受数字版权保护，或内容不是有效的 EPUB 时返回给
/// 用户看的原因。
pub fn load(path: &Path) -> Result<Book, String> {
    let metadata = winc_backend::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len > MAX_FILE {
        return Err(t!("preview-epub-too-large").to_owned());
    }
    let bytes = winc_backend::fs::read(path).map_err(|e| e.to_string())?;
    convert(&bytes)
}

/// 把内存中的 EPUB 文件 `bytes` 转换成 Markdown。
///
/// # Errors
///
/// 不是 zip 压缩包、找不到包文档、包文档损坏或书脊为空、内容受数字版权保护时返回给用户看的
/// 原因。单独一章损坏时不报错，该章显示无法读取的说明。
pub fn convert(bytes: &[u8]) -> Result<Book, String> {
    let invalid = || t!("preview-epub-invalid").to_owned();
    let archive = Archive::open(bytes).map_err(|e| match e {
        zip::Error::Invalid => invalid(),
        zip::Error::Unsupported => t!("preview-epub-unsupported").to_owned(),
    })?;
    if protected(&archive) {
        return Err(t!("preview-epub-drm").to_owned());
    }
    let package = Package::read(&archive).ok_or_else(invalid)?;
    if package.spine.is_empty() {
        return Err(invalid());
    }
    let contents = contents(&archive, &package);
    Ok(xhtml::convert(&archive, &package, &contents))
}

/// 包文档（OPF）中读出的清单与书脊。
struct Package {
    /// 书脊中的各章在包中的路径，按阅读顺序排列。
    spine: Vec<String>,
    /// 清单中各文件的编号到（包中的路径，媒体类型，`properties` 属性）。
    manifest: HashMap<String, (String, String, String)>,
    /// 书脊的 `toc` 属性指向的 NCX 文件在包中的路径（EPUB 2 的目录）。
    ncx: Option<String>,
}

impl Package {
    /// 按 `META-INF/container.xml` 找到包文档并读出清单与书脊；找不到或损坏时返回 `None`。
    fn read(archive: &Archive<'_>) -> Option<Package> {
        let container = archive.read("META-INF/container.xml")?.ok()?;
        let container = parse(&container)?;
        let opf = container
            .descendants()
            .find(|n| n.has_tag_name_local("rootfile"))?
            .attribute("full-path")?;
        let opf = resolve("", &percent_decode(opf));
        let bytes = archive.read(&opf)?.ok()?;
        let doc = parse(&bytes)?;
        let dir = parent(&opf);
        let manifest: HashMap<String, (String, String, String)> = doc
            .descendants()
            .filter(|n| n.has_tag_name_local("item"))
            .filter_map(|n| {
                let href = resolve(dir, &percent_decode(n.attribute("href")?));
                let kind = n.attribute("media-type").unwrap_or_default().to_owned();
                let properties = n.attribute("properties").unwrap_or_default().to_owned();
                Some((n.attribute("id")?.to_owned(), (href, kind, properties)))
            })
            .collect();
        let spine_node = doc.descendants().find(|n| n.has_tag_name_local("spine"))?;
        let spine = spine_node
            .children()
            .filter(|n| n.has_tag_name_local("itemref"))
            .filter_map(|n| manifest.get(n.attribute("idref")?))
            .filter(|(_, kind, _)| is_page(kind))
            .map(|(href, ..)| href.clone())
            .collect();
        let ncx = (spine_node.attribute("toc"))
            .and_then(|id| manifest.get(id))
            .map(|(href, ..)| href.clone());
        Some(Package {
            spine,
            manifest,
            ncx,
        })
    }
}

/// 媒体类型 `kind` 是否是书脊中能显示的页面（XHTML，少数旧书写作 HTML）。
fn is_page(kind: &str) -> bool {
    matches!(kind, "application/xhtml+xml" | "text/html")
}

/// 书中是否有受数字版权保护的内容：`META-INF/encryption.xml` 中有字体混淆以外的加密算法。
fn protected(archive: &Archive<'_>) -> bool {
    let Some(Ok(bytes)) = archive.read("META-INF/encryption.xml") else {
        return false;
    };
    let Some(doc) = parse(&bytes) else {
        return true;
    };
    doc.descendants()
        .filter(|n| n.has_tag_name_local("EncryptionMethod"))
        .any(|n| {
            !n.attribute("Algorithm")
                .is_some_and(|a| FONT_OBFUSCATION.contains(&a))
        })
}

/// 书的目录（未换成锚点）：标题、层级，以及指向的位置（包中的路径和片段标识）。
pub(super) type Contents = Vec<(String, usize, Option<(String, Option<String>)>)>;

/// 读出书的目录：优先用导航文档（清单中 `properties` 含 `nav` 的文件），没有时用 NCX。
fn contents(archive: &Archive<'_>, package: &Package) -> Contents {
    let nav =
        (package.manifest.values()).find(|(_, _, props)| props.split(' ').any(|p| p == "nav"));
    let mut out = Contents::new();
    if let Some((path, ..)) = nav
        && let Some(Ok(bytes)) = archive.read(path)
        && let Some(text) = xhtml::readable(&bytes)
        && let Some(doc) = parse_text(&text)
    {
        // 有多个 `<nav>` 时取 `epub:type="toc"` 的那个（其余是地标、页码列表）。
        let navs: Vec<Node<'_, '_>> = doc
            .descendants()
            .filter(|n| n.has_tag_name_local("nav"))
            .collect();
        let toc = (navs.iter())
            .find(|n| {
                n.attributes()
                    .any(|a| a.name() == "type" && a.value().split(' ').any(|v| v == "toc"))
            })
            .or(navs.first());
        if let Some(list) = toc.and_then(|n| n.children().find(|c| c.has_tag_name_local("ol"))) {
            nav_list(list, path, 0, &mut out);
        }
    }
    if out.is_empty()
        && let Some(path) = &package.ncx
        && let Some(Ok(bytes)) = archive.read(path)
        && let Some(doc) = parse(&bytes)
        && let Some(map) = doc.descendants().find(|n| n.has_tag_name_local("navMap"))
    {
        nav_points(map, path, 0, &mut out);
    }
    out
}

/// 读导航文档 `file` 中的列表 `list`（`<ol>`）：每个 `<li>` 是一条，其中的 `<ol>` 是下一层。
fn nav_list(list: Node<'_, '_>, file: &str, level: usize, out: &mut Contents) {
    if level >= MAX_LEVELS {
        return;
    }
    for item in list.children().filter(|n| n.has_tag_name_local("li")) {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let label = item
            .children()
            .find(|n| n.has_tag_name_local("a") || n.has_tag_name_local("span"));
        if let Some(label) = label {
            let target = label.attribute("href").map(|href| locate(file, href));
            out.push((text_of(label), level, target));
        }
        if let Some(sub) = item.children().find(|n| n.has_tag_name_local("ol")) {
            nav_list(sub, file, level + 1, out);
        }
    }
}

/// 读 NCX 文件 `file` 中 `parent` 下的各个 `<navPoint>`：标题在 `<navLabel><text>`，目标在
/// `<content src>`。
fn nav_points(parent: Node<'_, '_>, file: &str, level: usize, out: &mut Contents) {
    if level >= MAX_LEVELS {
        return;
    }
    for point in parent
        .children()
        .filter(|n| n.has_tag_name_local("navPoint"))
    {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let title = (point.children())
            .find(|n| n.has_tag_name_local("navLabel"))
            .map(text_of)
            .unwrap_or_default();
        let target = (point.children())
            .find(|n| n.has_tag_name_local("content"))
            .and_then(|n| n.attribute("src"))
            .map(|src| locate(file, src));
        out.push((title, level, target));
        nav_points(point, file, level + 1, out);
    }
}

/// 元素 `node` 中的文字，连续的空白合并成一个空格，去掉首尾空白。
fn text_of(node: Node<'_, '_>) -> String {
    let text: String = (node.descendants())
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 文件 `file` 中的链接 `href` 指向的位置：包中的路径和片段标识；只有片段标识时是 `file`
/// 本身，网址等书外的地址路径为空。
pub(super) fn locate(file: &str, href: &str) -> (String, Option<String>) {
    let (path, fragment) = match href.split_once('#') {
        Some((path, fragment)) => (path, Some(percent_decode(fragment))),
        None => (href, None),
    };
    if path.contains(':') {
        return (String::new(), fragment);
    }
    let path = if path.is_empty() {
        file.to_owned()
    } else {
        resolve(parent(file), &percent_decode(path))
    };
    (path, fragment.filter(|f| !f.is_empty()))
}

/// 包中路径 `path` 所在的目录（不带末尾的 `/`）；在包的根目录时为空。
pub(super) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// 解码地址中的百分号转义（`%20` 等）；不成对的 `%` 原样保留，结果不是 UTF-8 时按有损方式转换。
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(b) if bytes[i] == b'%' => {
                out.push(b);
                i += 3;
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 把 UTF-8 的 XML 文件 `bytes` 解析成文档；不是 UTF-8 或 XML 损坏时返回 `None`。
fn parse(bytes: &[u8]) -> Option<Document<'_>> {
    let text = std::str::from_utf8(bytes).ok()?;
    parse_text(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// 把 `text` 解析成 XML 文档；允许文档类型声明（XHTML 常带 DTD），实体的展开由解析器限制。
fn parse_text(text: &str) -> Option<Document<'_>> {
    let options = ParsingOptions {
        allow_dtd: true,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).ok()
}

/// 按本地名称（不含命名空间前缀）比较元素名。
trait LocalName {
    /// 是否是本地名称为 `name` 的元素。
    fn has_tag_name_local(&self, name: &str) -> bool;
}

impl LocalName for Node<'_, '_> {
    fn has_tag_name_local(&self, name: &str) -> bool {
        self.is_element() && self.tag_name().name() == name
    }
}

#[cfg(test)]
mod tests;
