//! EPUB 转换的测试：测试用的书由代码生成，覆盖书脊顺序、导航文档与 NCX 目录、锚点与链接、
//! 各种块与行内元素、HTML 命名实体、受保护与损坏的书。

use super::*;
use crate::markdown::{Block, parse as parse_markdown};

const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

/// 生成一本书：`manifest` 是清单中 `<item>` 的属性，`spine` 是书脊的 `idref`，`files` 是
/// `OEBPS/` 下的文件，`toc` 是书脊的 `toc` 属性。
fn book(manifest: &[&str], spine: &[&str], toc: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let items: String = manifest.iter().map(|m| format!("<item {m}/>")).collect();
    let refs: String = spine
        .iter()
        .map(|id| format!("<itemref idref=\"{id}\"/>"))
        .collect();
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\">\
         <manifest>{items}</manifest><spine {toc}>{refs}</spine></package>"
    );
    let mut all: Vec<(String, Vec<u8>)> = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            CONTAINER.as_bytes().to_vec(),
        ),
        ("OEBPS/content.opf".into(), opf.into_bytes()),
    ];
    all.extend(
        files
            .iter()
            .map(|(n, b)| (format!("OEBPS/{n}"), b.as_bytes().to_vec())),
    );
    let borrowed: Vec<(&str, &[u8])> = all
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    crate::zip::build(&borrowed, true)
}

/// 一章 XHTML：`body` 是 `<body>` 中的内容。
fn page(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\
         <head><title>t</title><style>p {{ color: red }}</style></head><body>{body}</body></html>"
    )
}

/// 第一章有标题、段落、行内样式与各种块，第二章的小节被目录和第一章的链接指向。
fn sample() -> Vec<u8> {
    let one = page(
        "<section><h1 id=\"top\">第一章&nbsp;开端</h1>\n\
         <p>第一行\n第二行，<em>强调</em>与<b>粗体</b> &amp; 1. 不是列表</p>\
         <p>见<a href=\"two.xhtml#sec\">第二节</a>、<a href=\"https://example.com\">网站</a>和\
         <a href=\"missing.xhtml\">书外</a>。</p>\
         <blockquote><p>引用</p></blockquote>\
         <ol start=\"3\"><li>甲</li><li><p>乙</p><ul><li>丙</li></ul></li></ol>\
         <pre>let x = 1;\n```\n</pre>\
         <table><tr><th>名</th><th>值</th></tr><tr><td>a|b</td><td colspan=\"1\">2</td></tr></table>\
         <p><img src=\"images/a.png\" alt=\"图\"/><img src=\"images/b.tiff\" alt=\"不支持\"/></p>\
         <p>注音<ruby>漢<rt>かん</rt></ruby></p></section>",
    );
    let two = page("<h2>第二章</h2><p>前文</p><div id=\"sec\"><p>小节正文</p></div>");
    let nav = page(
        "<nav epub:type=\"landmarks\"><ol><li><a href=\"one.xhtml\">地标</a></li></ol></nav>\
         <nav epub:type=\"toc\"><ol><li><a href=\"one.xhtml\">第一章</a>\
         <ol><li><a href=\"two.xhtml#sec\">小节</a></li></ol></li>\
         <li><span>无链接</span></li></ol></nav>",
    );
    book(
        &[
            "id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"",
            "id=\"one\" href=\"one.xhtml\" media-type=\"application/xhtml+xml\"",
            "id=\"two\" href=\"two.xhtml\" media-type=\"application/xhtml+xml\"",
            "id=\"img\" href=\"images/a.png\" media-type=\"image/png\"",
        ],
        &["one", "two"],
        "",
        &[
            ("one.xhtml", &one),
            ("two.xhtml", &two),
            ("nav.xhtml", &nav),
            ("images/a.png", "png"),
        ],
    )
}

#[test]
fn chapters_convert_in_spine_order() {
    let book = convert(&sample()).unwrap();
    let md = &book.markdown;
    assert!(
        md.starts_with("# <a id=\"c0\"></a>第一章\u{a0}开端\n\n"),
        "{md}"
    );
    // 中日韩文字之间的换行不写成空格；ASCII 标点转义，不成为 Markdown 标记。
    assert!(md.contains("第一行第二行，<i>强调</i>与<b>粗体</b> \\& 1\\. 不是列表"));
    assert!(md.contains("> 引用"));
    assert!(md.contains("3. 甲\n\n4. 乙\n\n   - 丙"), "{md}");
    assert!(md.contains("```\nlet x = 1;\n```\n````"), "{md}");
    assert!(md.contains("| 名 | 值 |\n| --- | --- |\n| a\\|b | 2 |"));
    assert!(md.contains("<img src=\"OEBPS/images/a.png\" alt=\"图\">不支持"));
    assert!(md.contains("注音漢") && !md.contains("かん") && !md.contains("color"));
    assert_eq!(book.media.len(), 1);
    assert_eq!(&*book.media[0].1, b"png");
}

#[test]
fn links_and_contents_point_at_anchors() {
    let book = convert(&sample()).unwrap();
    let md = &book.markdown;
    // 书中的链接改写为锚点，网址保留，书外的文件只留文字。
    assert!(md.contains(
        "见<a href=\"#c1-0\">第二节</a>、<a href=\"https://example.com\">网站</a>和书外。"
    ));
    assert!(md.contains("## <a id=\"c1\"></a>第二章"));
    assert!(md.contains("<a id=\"c1-0\"></a>小节正文"));
    assert_eq!(
        book.contents,
        [
            TocEntry {
                title: "第一章".into(),
                level: 0,
                anchor: Some("c0".into())
            },
            TocEntry {
                title: "小节".into(),
                level: 1,
                anchor: Some("c1-0".into())
            },
            TocEntry {
                title: "无链接".into(),
                level: 0,
                anchor: None
            },
        ]
    );
    // 解析后锚点落在段落上，预览可以转到。
    let parsed = parse_markdown(md);
    let has = |id: &str| {
        parsed.blocks.iter().any(|b| match b {
            Block::Paragraph(i) | Block::Heading(_, i) => i.ids.iter().any(|x| x == id),
            _ => false,
        })
    };
    assert!(has("c0") && has("c1") && has("c1-0"));
}

#[test]
fn epub2_books_read_the_ncx_contents() {
    let ncx = r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
        <navPoint id="a"><navLabel><text> 开篇 </text></navLabel><content src="text/one.html"/>
          <navPoint id="b"><navLabel><text>细节</text></navLabel><content src="text/one.html#d%20e"/></navPoint>
        </navPoint></navMap></ncx>"#;
    let one = page("<p>甲</p><p id=\"d e\">乙</p>");
    let bytes = book(
        &[
            "id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"",
            "id=\"one\" href=\"text/one.html\" media-type=\"application/xhtml+xml\"",
        ],
        &["one"],
        "toc=\"ncx\"",
        &[("toc.ncx", ncx), ("text/one.html", &one)],
    );
    let book = convert(&bytes).unwrap();
    let titles: Vec<(&str, usize, Option<&str>)> = (book.contents.iter())
        .map(|e| (e.title.as_str(), e.level, e.anchor.as_deref()))
        .collect();
    assert_eq!(titles, [("开篇", 0, Some("c0")), ("细节", 1, Some("c0-0"))]);
    assert!(book.markdown.contains("<a id=\"c0-0\"></a>乙"));
}

#[test]
fn protected_damaged_and_invalid_books() {
    let encryption = |algorithm: &str| {
        format!(
            "<encryption xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\" \
             xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\"><enc:EncryptedData>\
             <enc:EncryptionMethod Algorithm=\"{algorithm}\"/></enc:EncryptedData></encryption>"
        )
    };
    let one = page("<p>正文</p>");
    let with = |extra: &[(&str, &str)]| {
        let mut files = vec![("one.xhtml", one.as_str())];
        files.extend_from_slice(extra);
        let mut bytes = book(
            &["id=\"one\" href=\"one.xhtml\" media-type=\"application/xhtml+xml\""],
            &["one"],
            "",
            &files,
        );
        // `META-INF/encryption.xml` 在 `OEBPS/` 之外，另行加入。
        if let Some((_, xml)) = extra.iter().find(|(n, _)| *n == "enc") {
            let archive = crate::zip::Archive::open(&bytes).unwrap();
            let mut entries: Vec<(String, Vec<u8>)> = [
                "mimetype",
                "META-INF/container.xml",
                "OEBPS/content.opf",
                "OEBPS/one.xhtml",
            ]
            .iter()
            .map(|n| (n.to_string(), archive.read(n).unwrap().unwrap()))
            .collect();
            entries.push(("META-INF/encryption.xml".into(), xml.as_bytes().to_vec()));
            let borrowed: Vec<(&str, &[u8])> = entries
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect();
            bytes = crate::zip::build(&borrowed, true);
        }
        convert(&bytes)
    };
    let fonts = encryption("http://www.idpf.org/2008/embedding");
    assert!(with(&[("enc", &fonts)]).is_ok());
    let aes = encryption("http://www.w3.org/2001/04/xmlenc#aes128-cbc");
    assert_eq!(with(&[("enc", &aes)]).unwrap_err(), t!("preview-epub-drm"));

    // 损坏的一章写一句说明，其余照常。
    let bytes = book(
        &[
            "id=\"one\" href=\"one.xhtml\" media-type=\"application/xhtml+xml\"",
            "id=\"bad\" href=\"bad.xhtml\" media-type=\"application/xhtml+xml\"",
        ],
        &["bad", "one"],
        "",
        &[("one.xhtml", &one), ("bad.xhtml", "<html><body><p>未闭合")],
    );
    let md = convert(&bytes).unwrap().markdown;
    assert!(md.starts_with("<a id=\"c0\"></a><i>"), "{md}");
    assert!(md.contains("bad\\.xhtml") && md.ends_with("<a id=\"c1\"></a>正文\n"));

    for bytes in [
        &b"not a zip"[..],
        &crate::zip::build(&[("mimetype", b"x")], false),
    ] {
        assert_eq!(convert(bytes).unwrap_err(), t!("preview-epub-invalid"));
    }
}

/// 由纯文本转换来的书：一个段落元素中有多段，每段换行后以全角空格缩进。换行后的全角空格处
/// 分段并保留缩进；行内的全角空格、标题中的换行不分段。
#[test]
fn indented_lines_start_new_paragraphs() {
    let one = page(
        "<h1>第一回\n\u{3000}\u{3000}标题</h1>\
         <p>\u{3000}\u{3000}甲段第一行\n甲段第二行\u{3000}中间\n\u{3000}\u{3000}乙段<b>粗\n\u{3000}丙段</b></p>",
    );
    let bytes = book(
        &["id=\"one\" href=\"one.xhtml\" media-type=\"application/xhtml+xml\""],
        &["one"],
        "",
        &[("one.xhtml", &one)],
    );
    let md = convert(&bytes).unwrap().markdown;
    assert_eq!(
        md,
        "# <a id=\"c0\"></a>第一回\u{3000}\u{3000}标题\n\n\
         \u{3000}\u{3000}甲段第一行甲段第二行\u{3000}中间\n\n\
         \u{3000}\u{3000}乙段<b>粗</b>\n\n<b>\u{3000}丙段</b>\n"
    );
    let parsed = parse_markdown(&md);
    assert_eq!(parsed.blocks.len(), 4);
    assert!(matches!(&parsed.blocks[1], Block::Paragraph(i) if i.text.starts_with('\u{3000}')));
}

#[test]
fn unknown_html_entities_still_parse() {
    let text = xhtml::readable("<p>a&nbsp;b&hellip;&amp;&lt;&foo;&#65;& c</p>".as_bytes());
    assert_eq!(
        text.as_deref(),
        Some("<p>a&#160;b&#8230;&amp;&lt;&amp;foo;&#65;&amp; c</p>")
    );
    assert_eq!(
        locate("OEBPS/a/x.xhtml", "#n"),
        ("OEBPS/a/x.xhtml".into(), Some("n".into()))
    );
    assert_eq!(
        locate("OEBPS/a/x.xhtml", "../b%20c.xhtml"),
        ("OEBPS/b c.xhtml".into(), None)
    );
    assert_eq!(locate("x.xhtml", "https://e.com/#a").0, "");
}
