//! 读 docx 文字文档（Office Open XML 的 WordprocessingML），转换成 Markdown 文本。
//!
//! 转换结果交给 Markdown 预览排版，得到便于阅读的视图，不还原分页和版式：标题、段落、行内
//! 样式、编号与项目符号、表格、图片、脚注、尾注和批注对应到 Markdown 的块与行内元素；饼图
//! 转换为 mermaid 饼图，分类较少的图表转换为表格，其余图表只列出标题和数据组。页眉、页脚、
//! 文本框的位置和修订记录中删除的文字不显示。
//!
//! 元素与属性按本地名称识别，不区分命名空间前缀，ECMA-376 的过渡与严格两种命名空间都适用。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use roxmltree::{Document, Node};
use winc_i18n::t;

use crate::ooxml::{package_target, parse, part, relations};
use crate::zip::{self, Archive};

/// 包关系中没有主文档时的默认位置。
const DEFAULT_DOCUMENT: &str = "word/document.xml";

/// 能读的 docx 文件的大小上限（字节）：与 xlsx 预览使用同一上限，两种 Office 文档包的读取
/// 方式相同。
const MAX_FILE: u64 = crate::table::MAX_FILE;

/// 转换为表格的图表最多的分类数：超过时表格过长，改为只列出标题和数据组。
///
/// 取自调参：50 行的表格约占预览的两屏，再长时逐行阅读数据已不如查看原图表。
const MAX_CHART_ROWS: usize = 50;

/// 沿样式继承链查找的最大层数：样式表中出现循环继承时在此处停止。
///
/// ECMA-376 不限制继承的层数；该上限只用于终止循环，取 16 层。
const MAX_STYLE_DEPTH: usize = 16;

/// 编号的最大层数：WordprocessingML 的编号定义有 9 层（`ilvl` 为 0 到 8）。
const LEVELS: usize = 9;

/// 转换结果：Markdown 文本，以及其中引用的、包内的图片。
#[derive(Debug, Default)]
pub struct Converted {
    /// 转换得到的 Markdown 文本。
    pub markdown: String,
    /// 图片：Markdown 中引用的地址（包中的路径）和图片文件的内容。
    pub media: Vec<(String, Arc<[u8]>)>,
}

/// `path` 是否是 docx 文字文档（按扩展名判断，含启用宏的 docm 和模板 dotx、dotm）。
pub fn is_docx(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["docx", "docm", "dotx", "dotm"]
            .iter()
            .any(|known| e.eq_ignore_ascii_case(known))
    })
}

/// 读取 docx 文件 `path` 并转换成 Markdown。
///
/// # Errors
///
/// 文件无法读取、超过大小上限（与 xlsx 预览相同，256 MiB），或内容不是有效的 docx 时返回
/// 给用户看的原因。
pub fn load(path: &Path) -> Result<Converted, String> {
    let metadata = winc_backend::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len > MAX_FILE {
        return Err(t!("preview-docx-too-large").to_owned());
    }
    let bytes = winc_backend::fs::read(path).map_err(|e| e.to_string())?;
    convert(&bytes)
}

/// 把内存中的 docx 文件 `bytes` 转换成 Markdown。
///
/// # Errors
///
/// 不是 zip 压缩包、找不到主文档，或其中的 XML 损坏时返回给用户看的原因。
pub fn convert(bytes: &[u8]) -> Result<Converted, String> {
    let archive = Archive::open(bytes).map_err(|e| match e {
        zip::Error::Invalid => t!("preview-docx-invalid").to_owned(),
        zip::Error::Unsupported => t!("preview-docx-unsupported").to_owned(),
    })?;
    let main = package_target(&archive).unwrap_or_else(|| DEFAULT_DOCUMENT.to_owned());
    let bytes = part(&archive, &main)?.ok_or(t!("preview-docx-invalid"))?;
    let document = parse(&bytes, &main)?;
    let rels = relations(&archive, &main)?;
    let related = |kind: &str| {
        rels.values()
            .find(|(k, _)| k.ends_with(kind))
            .map(|(_, target)| target.clone())
    };
    // 样式、编号、脚注等部件缺失或损坏时按没有处理：正文仍然可以显示。
    let load_part = |kind: &str| -> Option<(String, Vec<u8>)> {
        let path = related(kind)?;
        let bytes = part(&archive, &path).ok()??;
        Some((path, bytes))
    };
    let styles_part = load_part("/styles");
    let numbering_part = load_part("/numbering");
    let footnotes_part = load_part("/footnotes");
    let endnotes_part = load_part("/endnotes");
    let comments_part = load_part("/comments");
    let styles = xml(&styles_part)
        .map(|d| Styles::read(&d))
        .unwrap_or_default();
    let numbering = xml(&numbering_part)
        .map(|d| Numbering::read(&d))
        .unwrap_or_default();
    let footnotes = xml(&footnotes_part);
    let endnotes = xml(&endnotes_part);
    let comments = xml(&comments_part);

    let mut cx = Converter {
        archive: &archive,
        rels: &rels,
        styles: &styles,
        numbering: &numbering,
        footnotes: by_id(footnotes.as_ref(), "footnote"),
        endnotes: by_id(endnotes.as_ref(), "endnote"),
        comments: by_id(comments.as_ref(), "comment"),
        counters: HashMap::new(),
        blocks: Vec::new(),
        notes: Vec::new(),
        media: Vec::new(),
        pending: Vec::new(),
        list_depth: None,
    };
    let body = document
        .root_element()
        .children()
        .find(|n| n.tag_name().name() == "body")
        .ok_or(t!("preview-docx-invalid"))?;
    cx.blocks(body);
    Ok(cx.finish())
}

/// 解析可选的部件 `part`（包中的路径和内容）；没有该部件或 XML 损坏时返回 `None`。
fn xml(part: &Option<(String, Vec<u8>)>) -> Option<Document<'_>> {
    part.as_ref()
        .and_then(|(path, bytes)| parse(bytes, path).ok())
}

/// 脚注、尾注或批注部件 `doc` 中本地名称为 `tag` 的各条，按编号（`w:id`）查找。
fn by_id<'a, 'i>(doc: Option<&'a Document<'i>>, tag: &str) -> HashMap<String, Node<'a, 'i>> {
    doc.map(|d| {
        d.root_element()
            .children()
            .filter(|n| n.tag_name().name() == tag)
            .filter_map(|n| Some((attr(n, "id")?.to_owned(), n)))
            .collect()
    })
    .unwrap_or_default()
}

/// 返回 `node` 本地名称为 `name` 的属性，不区分命名空间。
fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name)
        .map(|a| a.value())
}

/// 返回 `node` 中引用关系编号的属性（`r:id`、`r:embed`）：命名空间为关系命名空间。
fn rel_attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| {
            a.name() == name
                && a.namespace()
                    .is_some_and(|ns| ns.ends_with("/relationships"))
        })
        .map(|a| a.value())
}

/// 返回 `node` 中本地名称为 `name` 的第一个子元素。
fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.tag_name().name() == name)
}

/// 开关属性（`<w:b/>`、`<w:i w:val="0"/>`）是否打开：没有 `val` 或值不为假时打开。
fn on(node: Node<'_, '_>) -> bool {
    !matches!(attr(node, "val"), Some("0" | "false" | "off" | "none"))
}

/// 把 `text` 作为 Markdown 正文写进 `out`：ASCII 标点前加反斜杠，原样显示，不被当作标记。
///
/// `CommonMark` 规定反斜杠后的任意 ASCII 标点都表示该字符本身，因此逐个转义即可，不必判断
/// 标点在上下文中是否构成标记。
fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
}

/// 返回可以放进 `<…>` 链接地址中的 `url`：去掉换行，`<`、`>` 和空格按百分号转义。
fn link_target(url: &str) -> String {
    url.chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .map(|c| match c {
            '<' => "%3C".to_owned(),
            '>' => "%3E".to_owned(),
            ' ' => "%20".to_owned(),
            c => c.to_string(),
        })
        .collect()
}

/// 返回可以放进 HTML 属性值（双引号）中的 `text`。
fn html_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 行内文字的样式：转换时写成 HTML 标签，不受 Markdown 强调标记左右两侧字符的限制。
///
/// `CommonMark` 的 `**` 前后紧挨中文标点和文字时可能不构成强调，而 `<b>` 等标签没有这一限制。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Format {
    /// 加粗。
    bold: bool,
    /// 倾斜。
    italic: bool,
    /// 删除线（单线、双线）。
    strike: bool,
    /// 上标。
    sup: bool,
    /// 下标。
    sub: bool,
}

impl Format {
    /// 读取格式属性 `rpr`（`<w:rPr>`），在 `self` 的基础上覆盖其中写明的项。
    fn apply(mut self, rpr: Node<'_, '_>) -> Format {
        for n in rpr.children().filter(Node::is_element) {
            match n.tag_name().name() {
                "b" => self.bold = on(n),
                "i" => self.italic = on(n),
                "strike" | "dstrike" => self.strike = on(n),
                "vertAlign" => {
                    let value = attr(n, "val");
                    self.sup = value == Some("superscript");
                    self.sub = value == Some("subscript");
                }
                _ => {}
            }
        }
        self
    }

    /// 写出开始标签，顺序与 [`Format::close`] 相反。
    fn open(self, out: &mut String) {
        for (set, tag) in self.tags() {
            if set {
                out.push_str(&format!("<{tag}>"));
            }
        }
    }

    /// 写出结束标签。
    fn close(self, out: &mut String) {
        for (set, tag) in self.tags().into_iter().rev() {
            if set {
                out.push_str(&format!("</{tag}>"));
            }
        }
    }

    /// 各样式是否打开及对应的 HTML 标签。
    fn tags(self) -> [(bool, &'static str); 5] {
        [
            (self.bold, "b"),
            (self.italic, "i"),
            (self.strike, "s"),
            (self.sup, "sup"),
            (self.sub, "sub"),
        ]
    }
}

/// 样式表中的一个样式。
#[derive(Debug, Default)]
struct Style {
    /// 样式名，小写（`heading 1`、`title`、`quote`）。
    name: String,
    /// 继承的样式编号。
    based: Option<String>,
    /// 大纲级别（0 起）；正文级别（9）记为 `None`。
    outline: Option<u8>,
    /// 段落样式带的编号：编号实例和层级（未写层级时为 `None`）。
    num: Option<(String, Option<usize>)>,
    /// 字符格式。
    format: Option<FormatNode>,
}

/// 样式中写明的字符格式：只记录写明的项，继承时逐项覆盖。
#[derive(Debug, Default, Clone, Copy)]
struct FormatNode {
    /// 写明的加粗。
    bold: Option<bool>,
    /// 写明的倾斜。
    italic: Option<bool>,
}

/// 样式表：样式编号到样式的映射。
#[derive(Debug, Default)]
struct Styles {
    /// 各样式，按样式编号查找。
    styles: HashMap<String, Style>,
}

impl Styles {
    /// 读取样式表 `doc`（`styles.xml`）。
    fn read(doc: &Document<'_>) -> Styles {
        let mut styles = HashMap::new();
        for s in doc
            .root_element()
            .children()
            .filter(|n| n.tag_name().name() == "style")
        {
            let Some(id) = attr(s, "styleId") else {
                continue;
            };
            let val = |name: &str| child(s, name).and_then(|n| attr(n, "val"));
            let ppr = child(s, "pPr");
            let outline = ppr
                .and_then(|p| child(p, "outlineLvl"))
                .and_then(|n| attr(n, "val")?.parse::<u8>().ok())
                .filter(|&l| l < 9);
            let num = ppr.and_then(|p| child(p, "numPr")).and_then(|n| {
                let id = child(n, "numId").and_then(|i| attr(i, "val"))?.to_owned();
                let level = child(n, "ilvl").and_then(|i| attr(i, "val")?.parse::<usize>().ok());
                Some((id, level))
            });
            let format = child(s, "rPr").map(|r| FormatNode {
                bold: child(r, "b").map(on),
                italic: child(r, "i").map(on),
            });
            styles.insert(
                id.to_owned(),
                Style {
                    name: val("name").unwrap_or_default().to_lowercase(),
                    based: val("basedOn").map(str::to_owned),
                    outline,
                    num,
                    format,
                },
            );
        }
        Styles { styles }
    }

    /// 从样式 `id` 沿继承链向上，返回第一个满足 `pick` 的结果。
    fn find<T>(&self, id: &str, mut pick: impl FnMut(&str, &Style) -> Option<T>) -> Option<T> {
        let mut at = Some(id);
        for _ in 0..MAX_STYLE_DEPTH {
            let key = at?;
            let style = self.styles.get(key)?;
            if let Some(found) = pick(key, style) {
                return Some(found);
            }
            at = style.based.as_deref();
        }
        None
    }

    /// 段落样式 `id` 的标题级别（1 起）：取大纲级别，或按样式名识别标题、文档标题样式。
    fn heading(&self, id: &str) -> Option<usize> {
        self.find(id, |_, s| {
            if let Some(level) = s.outline {
                return Some(usize::from(level) + 1);
            }
            if s.name == "title" {
                return Some(1);
            }
            s.name
                .strip_prefix("heading ")
                .and_then(|n| n.parse::<usize>().ok())
        })
    }

    /// 段落样式 `id` 是否是引用样式（`Quote`、`Intense Quote`）。
    fn quote(&self, id: &str) -> bool {
        self.find(id, |_, s| s.name.ends_with("quote").then_some(()))
            .is_some()
    }

    /// 段落样式 `id` 带的编号：编号实例、层级（未写时为 `None`）和写明编号的样式编号。
    fn num(&self, id: &str) -> Option<(String, Option<usize>, String)> {
        self.find(id, |key, s| {
            s.num
                .as_ref()
                .map(|(num, level)| (num.clone(), *level, key.to_owned()))
        })
    }

    /// 字符样式 `id` 在 `format` 基础上的格式。
    fn format(&self, id: &str, mut format: Format) -> Format {
        // 继承链上较近的样式优先：先收集再从远到近覆盖。
        let mut chain = Vec::new();
        self.find(id, |_, s| {
            chain.push(s.format);
            None::<()>
        });
        for node in chain.into_iter().rev().flatten() {
            if let Some(b) = node.bold {
                format.bold = b;
            }
            if let Some(i) = node.italic {
                format.italic = i;
            }
        }
        format
    }
}

/// 编号定义中的一层。
#[derive(Debug, Clone, Default)]
struct Level {
    /// 编号格式（`decimal`、`bullet`、`lowerLetter`……）。
    format: String,
    /// 编号文字的模板，`%1` 到 `%9` 代表各层的序号。
    text: String,
    /// 起始序号。
    start: u32,
    /// 关联的段落样式：标题样式通过它确定所在的层级。
    style: Option<String>,
}

/// 编号定义（`numbering.xml`）：编号实例到抽象编号定义各层的映射。
#[derive(Debug, Default)]
struct Numbering {
    /// 抽象编号定义：编号定义编号到各层。
    abstracts: HashMap<String, Vec<Level>>,
    /// 编号实例：实例编号到抽象编号定义编号。
    nums: HashMap<String, String>,
}

impl Numbering {
    /// 读取编号定义 `doc`。
    fn read(doc: &Document<'_>) -> Numbering {
        let root = doc.root_element();
        let mut abstracts = HashMap::new();
        for a in root
            .children()
            .filter(|n| n.tag_name().name() == "abstractNum")
        {
            let Some(id) = attr(a, "abstractNumId") else {
                continue;
            };
            let mut levels = vec![Level::default(); LEVELS];
            for l in a.children().filter(|n| n.tag_name().name() == "lvl") {
                let Some(slot) = attr(l, "ilvl")
                    .and_then(|v| v.parse::<usize>().ok())
                    .and_then(|i| levels.get_mut(i))
                else {
                    continue;
                };
                let val = |name: &str| child(l, name).and_then(|n| attr(n, "val"));
                *slot = Level {
                    format: val("numFmt").unwrap_or("decimal").to_owned(),
                    text: val("lvlText").unwrap_or_default().to_owned(),
                    start: val("start").and_then(|v| v.parse().ok()).unwrap_or(1),
                    style: val("pStyle").map(str::to_owned),
                };
            }
            abstracts.insert(id.to_owned(), levels);
        }
        let nums = root
            .children()
            .filter(|n| n.tag_name().name() == "num")
            .filter_map(|n| {
                let id = attr(n, "numId")?.to_owned();
                let abs = child(n, "abstractNumId").and_then(|a| attr(a, "val"))?;
                Some((id, abs.to_owned()))
            })
            .collect();
        Numbering { abstracts, nums }
    }

    /// 编号实例 `num` 的各层；没有该实例时返回 `None`。
    fn levels(&self, num: &str) -> Option<&[Level]> {
        self.abstracts.get(self.nums.get(num)?).map(Vec::as_slice)
    }
}

/// 把序号 `n` 按编号格式 `format` 写成文字；不认识的格式按阿拉伯数字。
fn number_text(n: u32, format: &str) -> String {
    match format {
        "upperRoman" => roman(n).to_uppercase(),
        "lowerRoman" => roman(n),
        "upperLetter" => letters(n).to_uppercase(),
        "lowerLetter" => letters(n),
        "chineseCounting"
        | "chineseCountingThousand"
        | "ideographTraditional"
        | "taiwaneseCounting"
        | "japaneseCounting" => chinese(n),
        "decimalEnclosedCircle" | "decimalEnclosedCircleChinese" if (1..=20).contains(&n) => {
            char::from_u32(0x2460 + n - 1).map_or(n.to_string(), String::from)
        }
        "decimalZero" if n < 10 => format!("0{n}"),
        _ => n.to_string(),
    }
}

/// 小写罗马数字；0 和超过 3999 的数写成阿拉伯数字。
fn roman(mut n: u32) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    const TABLE: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, text) in TABLE {
        while n >= value {
            out.push_str(text);
            n -= value;
        }
    }
    out
}

/// 小写字母序号：1 为 a，26 为 z，27 为 aa（同一字母重复，WordprocessingML 的规则）。
fn letters(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let letter = char::from(b'a' + ((n - 1) % 26) as u8);
    std::iter::repeat_n(letter, ((n - 1) / 26 + 1) as usize).collect()
}

/// 中文小写数字序号（一、十一、二十三），支持到 99；更大的数写成阿拉伯数字。
fn chinese(n: u32) -> String {
    const DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    let digit = |d: u32| DIGITS[(d % 10) as usize];
    match n {
        0..=9 => digit(n).to_owned(),
        10..=19 => format!("十{}", if n == 10 { "" } else { digit(n - 10) }),
        20..=99 => format!(
            "{}十{}",
            digit(n / 10),
            if n.is_multiple_of(10) {
                ""
            } else {
                digit(n % 10)
            }
        ),
        _ => n.to_string(),
    }
}

/// 转换后的一块。
struct Block {
    /// Markdown 文本，不含块之间的空行。
    text: String,
    /// 列表项所属的编号实例；同一编号实例的相邻列表项之间不空行，连成一个列表。
    list: Option<String>,
}

/// 转换过程中的状态。
struct Converter<'a, 'i> {
    /// docx 压缩包。
    archive: &'a Archive<'a>,
    /// 主文档的关系：关系编号到（关系类型，目标在包中的路径）。
    rels: &'a HashMap<String, (String, String)>,
    /// 样式表。
    styles: &'a Styles,
    /// 编号定义。
    numbering: &'a Numbering,
    /// 脚注，按编号查找。
    footnotes: HashMap<String, Node<'a, 'i>>,
    /// 尾注，按编号查找。
    endnotes: HashMap<String, Node<'a, 'i>>,
    /// 批注，按编号查找。
    comments: HashMap<String, Node<'a, 'i>>,
    /// 各编号实例当前的序号，每层一个。
    counters: HashMap<String, [u32; LEVELS]>,
    /// 已转换的块。
    blocks: Vec<Block>,
    /// 脚注定义：引用标签和内容（Markdown），按首次引用的顺序。
    notes: Vec<(String, String)>,
    /// 引用到的图片：地址和内容。
    media: Vec<(String, Arc<[u8]>)>,
    /// 段落中的文本框转换成的块：段落写完后接在它后面。
    pending: Vec<Block>,
    /// 上一块是列表项时的层级；嵌套层级不超过它加 1，避免缩进被当作代码块。
    list_depth: Option<usize>,
}

impl<'a, 'i> Converter<'a, 'i> {
    /// 转换 `parent` 中的块级元素：段落、表格，以及包着它们的内容控件。
    fn blocks(&mut self, parent: Node<'a, 'i>) {
        for node in parent.children().filter(Node::is_element) {
            match node.tag_name().name() {
                "p" => self.paragraph(node),
                "tbl" => {
                    let table = self.table(node);
                    self.push(table, None);
                }
                "sdt" => {
                    if let Some(content) = child(node, "sdtContent") {
                        self.blocks(content);
                    }
                }
                "customXml" | "ins" => self.blocks(node),
                _ => {}
            }
        }
    }

    /// 追加一块；`list` 为列表项所属的编号实例，不是列表项时为 `None` 并结束当前列表。
    fn push(&mut self, text: String, list: Option<String>) {
        if list.is_none() {
            self.list_depth = None;
        }
        self.blocks.push(Block { text, list });
    }

    /// 转换一个段落：标题、列表项、引用或普通段落；没有文字的段落不输出。
    fn paragraph(&mut self, p: Node<'a, 'i>) {
        let ppr = child(p, "pPr");
        let style = ppr
            .and_then(|n| child(n, "pStyle"))
            .and_then(|n| attr(n, "val"))
            .unwrap_or_default();
        let content = self.inline(p, Format::default());
        let pending = std::mem::take(&mut self.pending);
        let label = self.number_label(ppr, style);
        let outline = ppr
            .and_then(|n| child(n, "outlineLvl"))
            .and_then(|n| attr(n, "val")?.parse::<usize>().ok())
            .filter(|&l| l < 9)
            .map(|l| l + 1);
        let heading = outline.or_else(|| self.styles.heading(style));
        let text = content.trim();
        if text.is_empty() && label.is_none() {
            self.blocks.extend(pending);
            return;
        }
        if let Some(level) = heading {
            let mut out = "#".repeat(level.min(6));
            out.push(' ');
            if let Some((label, ..)) = &label {
                escape(label, &mut out);
                out.push(' ');
            }
            out.push_str(text);
            self.push(out, None);
        } else if let Some((label, level, bullet, num)) = label {
            let depth = match self.list_depth {
                Some(prev) => level.min(prev + 1),
                None => 0,
            };
            let ordered = label.strip_suffix('.').filter(|n| n.parse::<u32>().is_ok());
            if !bullet && ordered.is_none() && depth == 0 {
                // 编号的文字不是「数字加句点」（「（一）」「1.1」），Markdown 的有序列表无法
                // 表示：顶层的写成段落，编号文字写在开头。
                let mut out = String::new();
                escape(&label, &mut out);
                out.push(' ');
                out.push_str(text);
                self.push(out, None);
            } else {
                let mut out = "    ".repeat(depth);
                match ordered {
                    _ if bullet => out.push_str("- "),
                    Some(n) => out.push_str(&format!("{n}. ")),
                    // 嵌套层级中的特殊编号：写成无序列表项以保留层级，编号文字写在开头。
                    None => {
                        out.push_str("- ");
                        escape(&label, &mut out);
                        out.push(' ');
                    }
                }
                out.push_str(text);
                self.push(out, Some(num));
                self.list_depth = Some(depth);
            }
        } else if self.styles.quote(style) {
            self.push(format!("> {text}"), None);
        } else {
            self.push(text.to_owned(), None);
        }
        for block in pending {
            self.push(block.text, None);
        }
    }

    /// 段落的编号：编号文字、层级（0 起）、是否是项目符号和编号实例；没有编号时返回 `None`。
    ///
    /// 段落直接写明的编号优先，其次是段落样式带的编号；编号实例为 `0` 表示取消编号。
    fn number_label(
        &mut self,
        ppr: Option<Node<'_, '_>>,
        style: &str,
    ) -> Option<(String, usize, bool, String)> {
        let direct = ppr.and_then(|p| child(p, "numPr")).map(|n| {
            let num = child(n, "numId").and_then(|i| attr(i, "val"));
            let level = child(n, "ilvl").and_then(|i| attr(i, "val")?.parse::<usize>().ok());
            (num.map(str::to_owned), level)
        });
        let (num, level) = match direct {
            Some((Some(num), level)) => (num, level),
            // 只写了层级、没写编号实例：编号实例取自样式。
            Some((None, level)) => {
                let (num, _, _) = self.styles.num(style)?;
                (num, level)
            }
            None => {
                let (num, level, owner) = self.styles.num(style)?;
                // 样式没写层级时，取编号定义中关联该样式的那一层（标题编号常用这种写法）。
                // 先找关联段落样式本身的层，其次是写明编号的上级样式。
                let level = level.or_else(|| {
                    let levels = self.numbering.levels(&num)?;
                    let find =
                        |id: &str| levels.iter().position(|l| l.style.as_deref() == Some(id));
                    find(style).or_else(|| find(&owner))
                });
                (num, level)
            }
        };
        if num == "0" {
            return None;
        }
        let levels = self.numbering.levels(&num)?;
        let level = level.unwrap_or(0).min(LEVELS - 1);
        let def = levels.get(level)?;
        if def.format == "none" {
            return None;
        }
        let counters = self.counters.entry(num.clone()).or_insert([0; LEVELS]);
        // 首次出现的层从起始序号开始；更深的各层重新计数。
        counters[level] = if counters[level] == 0 {
            def.start.max(1)
        } else {
            counters[level] + 1
        };
        for deeper in counters.iter_mut().skip(level + 1) {
            *deeper = 0;
        }
        if def.format == "bullet" {
            return Some((String::new(), level, true, num));
        }
        let snapshot = *counters;
        let mut label = def.text.clone();
        for (k, item) in levels.iter().enumerate().take(level + 1).rev() {
            let n = snapshot[k].max(item.start.max(1));
            label = label.replace(&format!("%{}", k + 1), &number_text(n, &item.format));
        }
        Some((label, level, false, num))
    }

    /// 转换 `parent` 的行内内容（文字、链接、图片、脚注引用），样式在 `format` 基础上叠加；
    /// 返回 Markdown。
    fn inline(&mut self, parent: Node<'a, 'i>, format: Format) -> String {
        let mut runs: Vec<(String, Format)> = Vec::new();
        let mut out = String::new();
        // 相邻且样式相同的文字合并后再写标签，避免逐段开闭标签。
        let flush = |runs: &mut Vec<(String, Format)>, out: &mut String| {
            let mut i = 0;
            while i < runs.len() {
                let style = runs[i].1;
                let mut text = String::new();
                while i < runs.len() && runs[i].1 == style {
                    text.push_str(&runs[i].0);
                    i += 1;
                }
                style.open(out);
                out.push_str(&text);
                style.close(out);
            }
            runs.clear();
        };
        for node in parent.children().filter(Node::is_element) {
            match node.tag_name().name() {
                "r" => {
                    let mut style = format;
                    if let Some(rpr) = child(node, "rPr") {
                        if let Some(id) = child(rpr, "rStyle").and_then(|n| attr(n, "val")) {
                            style = self.styles.format(id, style);
                        }
                        style = style.apply(rpr);
                    }
                    for item in node.children().filter(Node::is_element) {
                        match item.tag_name().name() {
                            "t" => {
                                let mut text = String::new();
                                escape(item.text().unwrap_or_default(), &mut text);
                                runs.push((text, style));
                            }
                            "tab" | "ptab" => runs.push((" ".to_owned(), style)),
                            "br" | "cr" => {
                                if attr(item, "type") != Some("page") {
                                    runs.push(("<br>".to_owned(), Format::default()));
                                }
                            }
                            "noBreakHyphen" => runs.push(("\\-".to_owned(), style)),
                            other => {
                                flush(&mut runs, &mut out);
                                self.run_object(other, item, &mut out);
                            }
                        }
                    }
                }
                "hyperlink" => {
                    flush(&mut runs, &mut out);
                    let inner = self.inline(node, format);
                    let target = rel_attr(node, "id")
                        .and_then(|id| self.target(id))
                        .filter(|t| !t.is_empty());
                    match target {
                        Some(url) if !inner.trim().is_empty() => {
                            out.push_str(&format!("[{inner}](<{}>)", link_target(&url)));
                        }
                        _ => out.push_str(&inner),
                    }
                }
                "ins" | "smartTag" | "customXml" | "fldSimple" | "bdo" | "dir" | "moveTo" => {
                    flush(&mut runs, &mut out);
                    out.push_str(&self.inline(node, format));
                }
                "sdt" => {
                    flush(&mut runs, &mut out);
                    if let Some(content) = child(node, "sdtContent") {
                        out.push_str(&self.inline(content, format));
                    }
                }
                "oMath" | "oMathPara" => {
                    flush(&mut runs, &mut out);
                    let math: String = node
                        .descendants()
                        .filter(|n| n.tag_name().name() == "t")
                        .filter_map(|n| n.text())
                        .collect();
                    let mut text = String::new();
                    escape(&math, &mut text);
                    out.push_str(&format!("<i>{text}</i>"));
                }
                _ => {}
            }
        }
        flush(&mut runs, &mut out);
        out
    }

    /// 转换行内的非文字对象：图片、图表、脚注和批注的引用、文本框。
    fn run_object(&mut self, name: &str, node: Node<'a, 'i>, out: &mut String) {
        match name {
            "drawing" | "pict" | "object" => self.drawing(node, out),
            "AlternateContent" => {
                // 优先用兼容内容（`Fallback`），其中是所有版本都能显示的图片。
                let pick = child(node, "Fallback").or_else(|| child(node, "Choice"));
                if let Some(pick) = pick {
                    for n in pick.children().filter(Node::is_element) {
                        self.run_object(n.tag_name().name(), n, out);
                    }
                }
            }
            "footnoteReference" => {
                let note = attr(node, "id").and_then(|id| self.footnotes.get(id).copied());
                self.note_reference(note, None, out);
            }
            "endnoteReference" => {
                let note = attr(node, "id").and_then(|id| self.endnotes.get(id).copied());
                self.note_reference(note, None, out);
            }
            "commentReference" => {
                let note = attr(node, "id").and_then(|id| self.comments.get(id).copied());
                let author = note.and_then(|n| attr(n, "author")).unwrap_or_default();
                let prefix = t!("preview-docx-comment", author = author.to_owned());
                self.note_reference(note, Some(prefix), out);
            }
            _ => {}
        }
    }

    /// 写出脚注引用 `[^n]`，并把 `note` 的内容记为脚注定义；`prefix` 写在内容前面。
    fn note_reference(
        &mut self,
        note: Option<Node<'a, 'i>>,
        prefix: Option<String>,
        out: &mut String,
    ) {
        let Some(note) = note else {
            return;
        };
        let label = (self.notes.len() + 1).to_string();
        let mut text = String::new();
        if let Some(prefix) = prefix {
            escape(&prefix, &mut text);
        }
        let paragraphs: Vec<String> = note
            .children()
            .filter(|n| n.tag_name().name() == "p")
            .map(|p| self.inline(p, Format::default()).trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        text.push_str(&paragraphs.join("<br>"));
        out.push_str(&format!("[^{label}]"));
        self.notes.push((label, text));
    }

    /// 转换 `<w:drawing>`、`<w:pict>` 中的图片、图表和文本框。
    fn drawing(&mut self, node: Node<'a, 'i>, out: &mut String) {
        // 文本框：内容是段落，放在所在段落之后。
        for content in node
            .descendants()
            .filter(|n| n.tag_name().name() == "txbxContent")
        {
            let start = self.blocks.len();
            self.blocks(content);
            let inner: Vec<Block> = self.blocks.drain(start..).collect();
            for block in inner {
                let quoted = block
                    .text
                    .lines()
                    .map(|l| format!("> {l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.pending.push(Block {
                    text: quoted,
                    list: None,
                });
            }
        }
        if let Some(chart) = node.descendants().find(|n| n.tag_name().name() == "chart") {
            if let Some(path) = rel_attr(chart, "id").and_then(|id| self.target(id)) {
                let title = node
                    .descendants()
                    .find(|n| n.tag_name().name() == "docPr")
                    .and_then(|n| attr(n, "title").or_else(|| attr(n, "descr")));
                let block = self.chart(&path, title);
                self.pending.insert(0, block);
            }
            return;
        }
        let image = node
            .descendants()
            .filter(|n| matches!(n.tag_name().name(), "blip" | "imagedata"))
            .find_map(|n| rel_attr(n, "embed").or_else(|| rel_attr(n, "id")));
        let Some(path) = image.and_then(|id| self.target(id)) else {
            return;
        };
        let doc_pr = node.descendants().find(|n| n.tag_name().name() == "docPr");
        let alt = doc_pr
            .and_then(|n| attr(n, "descr"))
            .filter(|d| !d.is_empty())
            .unwrap_or_default();
        // 显示宽度取自版式中的尺寸（`cx`，单位为 EMU，1 逻辑像素 = 9525 EMU）。
        let width = node
            .descendants()
            .find(|n| n.tag_name().name() == "extent")
            .and_then(|n| attr(n, "cx")?.parse::<u64>().ok())
            .map(|cx| cx / 9525)
            .filter(|&w| w > 0);
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "bmp" | "ico" | "svg"
        ) {
            let message = t!(
                "preview-docx-image-unsupported",
                name = name.clone(),
                format = ext.to_uppercase()
            );
            let mut text = String::new();
            escape(&message, &mut text);
            out.push_str(&format!("<i>{text}</i>"));
            return;
        }
        if !self.media.iter().any(|(url, _)| *url == path)
            && let Some(Ok(bytes)) = self.archive.read(&path)
        {
            self.media.push((path.clone(), bytes.into()));
        }
        let width = width.map_or(String::new(), |w| format!(" width=\"{w}\""));
        out.push_str(&format!(
            "<img src=\"{}\" alt=\"{}\"{width}>",
            html_attr(&link_target(&path)),
            html_attr(alt)
        ));
    }

    /// 关系编号 `id` 的目标：包中的路径，外部链接为原样的网址。
    fn target(&self, id: &str) -> Option<String> {
        self.rels.get(id).map(|(_, target)| target.clone())
    }

    /// 转换包中 `path` 处的图表：饼图转换为 mermaid 饼图，分类不多的图表转换为表格，其余
    /// 只列出标题和各组数据的名称。`fallback` 是图表对象上写的标题。
    fn chart(&mut self, path: &str, fallback: Option<&str>) -> Block {
        let block = |text: String| Block { text, list: None };
        let Some(Ok(bytes)) = self.archive.read(path) else {
            return block(String::new());
        };
        let Ok(doc) = parse(&bytes, path) else {
            return block(String::new());
        };
        let root = doc.root_element();
        // 图表本身的标题是 `<c:chart>` 的直接子元素；坐标轴的标题在各坐标轴之下，不取。
        let title: String = child(root, "chart")
            .and_then(|c| child(c, "title"))
            .map(|t| {
                t.descendants()
                    .filter(|n| n.tag_name().name() == "t")
                    .filter_map(|n| n.text())
                    .collect()
            })
            .filter(|t: &String| !t.trim().is_empty())
            .or_else(|| fallback.map(str::to_owned))
            .unwrap_or_default();
        let plot = root
            .descendants()
            .find(|n| n.tag_name().name().ends_with("Chart") && n.tag_name().name() != "chart");
        let kind = plot.map(|p| p.tag_name().name()).unwrap_or_default();
        let series: Vec<Series> = plot
            .map(|p| {
                p.children()
                    .filter(|n| n.tag_name().name() == "ser")
                    .map(Series::read)
                    .collect()
            })
            .unwrap_or_default();
        let categories = series.first().map_or(&[][..], |s| s.categories.as_slice());

        // 饼图、圆环图：第一组数据画成 mermaid 饼图。
        if matches!(kind, "pieChart" | "pie3DChart" | "doughnutChart")
            && let Some(first) = series.first()
            && !first.values.is_empty()
        {
            let quote = |s: &str| s.replace('"', "'");
            let mut text = String::from("```mermaid\npie");
            if !title.is_empty() {
                text.push_str(&format!(" title {}", quote(&title)));
            }
            text.push('\n');
            for (k, value) in first.values.iter().enumerate() {
                let name = first
                    .categories
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| (k + 1).to_string());
                text.push_str(&format!("    \"{}\" : {value}\n", quote(&name)));
            }
            text.push_str("```");
            return block(text);
        }

        let mut heading = String::new();
        let caption = if title.is_empty() {
            t!("preview-docx-chart").to_owned()
        } else {
            t!("preview-docx-chart-titled", title = title.clone())
        };
        heading.push_str("<b>");
        escape(&caption, &mut heading);
        heading.push_str("</b>");

        // 散点图没有分类；分类太多时表格过长。
        let tabular = kind != "scatterChart"
            && !categories.is_empty()
            && categories.len() <= MAX_CHART_ROWS
            && !series.is_empty();
        if !tabular {
            let names: Vec<String> = series
                .iter()
                .enumerate()
                .map(|(k, s)| s.name.clone().unwrap_or_else(|| (k + 1).to_string()))
                .collect();
            let summary = t!(
                "preview-docx-chart-series",
                count = series.len().to_string(),
                names = names.join("、")
            );
            let mut text = format!("> {heading}<br>");
            escape(&summary, &mut text);
            return block(text);
        }
        let cell = |s: &str| {
            let mut out = String::new();
            escape(s, &mut out);
            out
        };
        let mut text = format!(
            "{heading}\n\n| {} |",
            cell(t!("preview-docx-chart-category"))
        );
        for (k, s) in series.iter().enumerate() {
            let name = s.name.clone().unwrap_or_else(|| (k + 1).to_string());
            text.push_str(&format!(" {} |", cell(&name)));
        }
        text.push_str("\n|");
        text.push_str(&" --- |".repeat(series.len() + 1));
        for (k, category) in categories.iter().enumerate() {
            text.push_str(&format!("\n| {} |", cell(category)));
            for s in &series {
                let value = s.values.get(k).cloned().unwrap_or_default();
                text.push_str(&format!(" {} |", cell(&value)));
            }
        }
        block(text)
    }

    /// 转换表格：合并的单元格留空，段落之间以换行分隔；只有一格的表格常用作框住一段内容，
    /// 转换为引用块。
    fn table(&mut self, tbl: Node<'a, 'i>) -> String {
        let mut rows: Vec<Vec<String>> = Vec::new();
        for tr in tbl.children().filter(|n| n.tag_name().name() == "tr") {
            let mut row = Vec::new();
            for tc in tr.children().filter(|n| n.tag_name().name() == "tc") {
                let tcpr = child(tc, "tcPr");
                let span = tcpr
                    .and_then(|p| child(p, "gridSpan"))
                    .and_then(|n| attr(n, "val")?.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                // 纵向合并的后续单元格：内容在第一格中，这里留空。
                let continued = tcpr
                    .and_then(|p| child(p, "vMerge"))
                    .is_some_and(|n| attr(n, "val") != Some("restart"));
                let text = if continued {
                    String::new()
                } else {
                    self.cell(tc)
                };
                row.push(text);
                row.extend(std::iter::repeat_n(String::new(), span - 1));
            }
            rows.push(row);
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return String::new();
        }
        if rows.len() == 1 && columns == 1 {
            return rows[0][0]
                .split("<br>")
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("<br>\n");
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
        out
    }

    /// 转换单元格：各段落（含嵌套表格中的文字）以 `<br>` 连接成一行。
    fn cell(&mut self, tc: Node<'a, 'i>) -> String {
        let mut lines = Vec::new();
        self.cell_lines(tc, &mut lines);
        lines.join("<br>")
    }

    /// 把 `parent` 中的段落逐段转换成一行文字，追加到 `lines`；嵌套表格的单元格逐格展开。
    fn cell_lines(&mut self, parent: Node<'a, 'i>, lines: &mut Vec<String>) {
        for node in parent.children().filter(Node::is_element) {
            match node.tag_name().name() {
                "p" => {
                    let mut text = String::new();
                    if let Some((label, _, bullet, _)) = self.number_label(child(node, "pPr"), "") {
                        if bullet {
                            text.push_str("• ");
                        } else {
                            escape(&label, &mut text);
                            text.push(' ');
                        }
                    }
                    text.push_str(self.inline(node, Format::default()).trim());
                    let text = text.trim().to_owned();
                    if !text.is_empty() {
                        lines.push(text);
                    }
                    // 单元格中的文本框、图表无法放进表格，放在表格之后。
                    let pending = std::mem::take(&mut self.pending);
                    self.blocks.extend(pending);
                }
                "tbl" | "tr" | "tc" | "sdt" | "sdtContent" | "customXml" => {
                    self.cell_lines(node, lines);
                }
                _ => {}
            }
        }
    }

    /// 结束转换：块之间空一行（相邻的列表项之间不空行），最后写脚注定义。
    fn finish(mut self) -> Converted {
        let pending = std::mem::take(&mut self.pending);
        self.blocks.extend(pending);
        let mut markdown = String::new();
        let mut prev_list: Option<&str> = None;
        for block in self.blocks.iter().filter(|b| !b.text.is_empty()) {
            let list = block.list.as_deref();
            if !markdown.is_empty() {
                let same = list.is_some() && list == prev_list;
                markdown.push_str(if same { "\n" } else { "\n\n" });
            }
            markdown.push_str(&block.text);
            prev_list = list;
        }
        for (label, text) in &self.notes {
            markdown.push_str(&format!("\n\n[^{label}]: {text}"));
        }
        markdown.push('\n');
        Converted {
            markdown,
            media: self.media,
        }
    }
}

/// 图表中的一组数据。
struct Series {
    /// 数据组的名称。
    name: Option<String>,
    /// 各分类（散点图为横坐标）的文字。
    categories: Vec<String>,
    /// 各分类的数值，按图表中缓存的文字。
    values: Vec<String>,
}

impl Series {
    /// 读取 `<c:ser>`：名称、分类与数值都取图表中缓存的值（`strCache`、`numCache`）。
    fn read(ser: Node<'_, '_>) -> Series {
        let points = |name: &str| -> Vec<String> {
            let Some(node) = child(ser, name) else {
                return Vec::new();
            };
            let mut points: Vec<(usize, String)> = node
                .descendants()
                .filter(|n| n.tag_name().name() == "pt")
                .filter_map(|pt| {
                    let idx = attr(pt, "idx")?.parse().ok()?;
                    Some((idx, child(pt, "v")?.text()?.to_owned()))
                })
                .collect();
            points.sort_by_key(|(idx, _)| *idx);
            points.into_iter().map(|(_, v)| v).collect()
        };
        let name = child(ser, "tx").and_then(|tx| {
            tx.descendants()
                .find(|n| n.tag_name().name() == "v")
                .and_then(|n| n.text())
                .map(str::to_owned)
        });
        let categories = match points("cat") {
            c if c.is_empty() => points("xVal"),
            c => c,
        };
        let values = match points("val") {
            v if v.is_empty() => points("yVal"),
            v => v,
        };
        Series {
            name,
            categories,
            values,
        }
    }
}

#[cfg(test)]
mod tests;
