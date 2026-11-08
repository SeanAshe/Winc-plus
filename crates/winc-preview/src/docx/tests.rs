//! `docx` 的测试：标题与编号、行内样式、列表、表格、图片、链接、脚注与批注、图表、修订记录、
//! Markdown 标点的转义，以及损坏的文件。

use super::*;

const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
    xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
    xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
    xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
    xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// 样式表：`H1`、`H2` 为标题样式（带编号实例 1），`Q` 为引用样式，`Strong` 为加粗的字符样式。
const STYLES: &str = "<w:style w:styleId=\"H1\"><w:name w:val=\"heading 1\"/>\
    <w:pPr><w:numPr><w:numId w:val=\"1\"/></w:numPr><w:outlineLvl w:val=\"0\"/></w:pPr></w:style>\
    <w:style w:styleId=\"H2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"H1\"/>\
    <w:pPr><w:outlineLvl w:val=\"1\"/></w:pPr></w:style>\
    <w:style w:styleId=\"Q\"><w:name w:val=\"Quote\"/></w:style>\
    <w:style w:styleId=\"Strong\"><w:name w:val=\"Strong\"/><w:rPr><w:b/></w:rPr></w:style>";

/// 编号定义：实例 1 是标题编号（`1`、`1.1`），实例 2 是项目符号，实例 3 是「1.」「a.」两层
/// 的有序列表，实例 4 是「（一）」式的中文编号。
const NUMBERING: &str = "<w:abstractNum w:abstractNumId=\"10\">\
    <w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/>\
    <w:pStyle w:val=\"H1\"/><w:lvlText w:val=\"%1\"/></w:lvl>\
    <w:lvl w:ilvl=\"1\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/>\
    <w:pStyle w:val=\"H2\"/><w:lvlText w:val=\"%1.%2\"/></w:lvl></w:abstractNum>\
    <w:abstractNum w:abstractNumId=\"20\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/>\
    <w:lvlText w:val=\"•\"/></w:lvl><w:lvl w:ilvl=\"1\"><w:numFmt w:val=\"bullet\"/>\
    <w:lvlText w:val=\"◦\"/></w:lvl></w:abstractNum>\
    <w:abstractNum w:abstractNumId=\"30\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/>\
    <w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/></w:lvl>\
    <w:lvl w:ilvl=\"1\"><w:start w:val=\"1\"/><w:numFmt w:val=\"lowerLetter\"/>\
    <w:lvlText w:val=\"%2.\"/></w:lvl></w:abstractNum>\
    <w:abstractNum w:abstractNumId=\"40\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/>\
    <w:numFmt w:val=\"chineseCounting\"/><w:lvlText w:val=\"（%1）\"/></w:lvl></w:abstractNum>\
    <w:num w:numId=\"1\"><w:abstractNumId w:val=\"10\"/></w:num>\
    <w:num w:numId=\"2\"><w:abstractNumId w:val=\"20\"/></w:num>\
    <w:num w:numId=\"3\"><w:abstractNumId w:val=\"30\"/></w:num>\
    <w:num w:numId=\"4\"><w:abstractNumId w:val=\"40\"/></w:num>";

/// 段落：样式 `style`（空串为不带样式）、编号（实例、层级），内容为 `runs`。
fn p(style: &str, num: Option<(u32, u32)>, runs: &str) -> String {
    let style = if style.is_empty() {
        String::new()
    } else {
        format!("<w:pStyle w:val=\"{style}\"/>")
    };
    let num = num.map_or(String::new(), |(id, level)| {
        format!("<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{id}\"/></w:numPr>")
    });
    format!("<w:p><w:pPr>{style}{num}</w:pPr>{runs}</w:p>")
}

/// 一段不带格式的文字。
fn r(text: &str) -> String {
    format!("<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
}

/// 构造 docx：正文为 `body`，另加 `parts`（包中路径、内容）；主文档的关系为 `rels`。
fn docx(body: &str, rels: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
    let root = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"word/document.xml\"/>\
         </Relationships>"
    );
    let document = format!("<w:document {W}><w:body>{body}</w:body></w:document>");
    let doc_rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rS\" Type=\"{REL}/styles\" Target=\"styles.xml\"/>\
         <Relationship Id=\"rN\" Type=\"{REL}/numbering\" Target=\"numbering.xml\"/>\
         {rels}</Relationships>"
    );
    let styles = format!("<w:styles {W}>{STYLES}</w:styles>");
    let numbering = format!("<w:numbering {W}>{NUMBERING}</w:numbering>");
    let mut files: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
        ("word/numbering.xml", numbering.as_bytes()),
    ];
    files.extend_from_slice(parts);
    crate::zip::build(&files, true)
}

/// 转换只有正文 `body` 的 docx，返回 Markdown。
fn markdown(body: &str) -> String {
    convert(&docx(body, "", &[])).unwrap().markdown
}

#[test]
fn headings_are_numbered_and_lists_follow_their_definitions() {
    let body = [
        p("H1", None, &r("概述")),
        p("H2", None, &r("功能")),
        p("H2", None, &r("思路")),
        p("H1", None, &r("接口")),
        p("", Some((2, 0)), &r("甲")),
        p("", Some((2, 1)), &r("乙")),
        p("", Some((3, 0)), &r("第一步")),
        p("", Some((3, 1)), &r("细节")),
        p("", Some((3, 0)), &r("第二步")),
        p("", Some((4, 0)), &r("说明")),
        p("Q", None, &r("引文")),
    ]
    .concat();
    assert_eq!(
        markdown(&body),
        "# 1 概述\n\n## 1\\.1 功能\n\n## 1\\.2 思路\n\n# 2 接口\n\n\
         - 甲\n    - 乙\n\n1. 第一步\n    - a\\. 细节\n2. 第二步\n\n（一） 说明\n\n> 引文\n"
    );
}

#[test]
fn inline_styles_become_tags_and_punctuation_stays_literal() {
    let runs = "<w:r><w:rPr><w:b/></w:rPr><w:t>粗体。</w:t></w:r>\
        <w:r><w:rPr><w:rStyle w:val=\"Strong\"/><w:i/></w:rPr><w:t>强调</w:t></w:r>\
        <w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:t>2</w:t></w:r>\
        <w:r><w:t>行</w:t><w:br/><w:t>首 # 1. *x* | y</w:t></w:r>";
    assert_eq!(
        markdown(&p("", None, runs)),
        "<b>粗体。</b><b><i>强调</i></b><sup>2</sup>行<br>首 \\# 1\\. \\*x\\* \\| y\n"
    );
}

#[test]
fn deleted_text_and_field_codes_are_left_out() {
    let runs = "<w:ins><w:r><w:t>新增</w:t></w:r></w:ins>\
        <w:del><w:r><w:delText>删去</w:delText></w:r></w:del>\
        <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>\
        <w:r><w:instrText> PAGE </w:instrText></w:r>\
        <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>3</w:t></w:r>\
        <w:r><w:fldChar w:fldCharType=\"end\"/></w:r>";
    assert_eq!(markdown(&p("", None, runs)), "新增3\n");
}

#[test]
fn tables_keep_merged_cells_empty_and_boxes_become_quotes() {
    let cell = |attrs: &str, text: &str| {
        format!(
            "<w:tc><w:tcPr>{attrs}</w:tcPr>{}</w:tc>",
            p("", None, &r(text))
        )
    };
    let table = format!(
        "<w:tbl><w:tr>{}{}</w:tr><w:tr>{}{}{}</w:tr><w:tr>{}{}{}</w:tr></w:tbl>",
        cell("<w:gridSpan w:val=\"2\"/>", "标题"),
        cell("", "c"),
        cell("<w:vMerge w:val=\"restart\"/>", "合并"),
        cell("", "b|1"),
        cell("", "c1"),
        cell("<w:vMerge/>", "不显示"),
        cell("", "b2"),
        cell("", "c2"),
    );
    let boxed = format!(
        "<w:tbl><w:tr><w:tc>{}{}</w:tc></w:tr></w:tbl>",
        p("", None, &r("int a;")),
        p("", None, &r("int b;"))
    );
    assert_eq!(
        markdown(&(table + &boxed)),
        "| 标题 |  | c |\n| --- | --- | --- |\n| 合并 | b\\|1 | c1 |\n|  | b2 | c2 |\n\n\
         > int a\\;<br>\n> int b\\;\n"
    );
}

#[test]
fn images_links_notes_and_comments_are_carried_over() {
    let image = "<w:r><w:drawing><wp:inline><wp:extent cx=\"952500\" cy=\"476250\"/>\
        <wp:docPr id=\"1\" name=\"图片 1\" descr=\"示意图\"/><a:graphic><a:graphicData>\
        <a:blip r:embed=\"rI\"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>";
    let link = "<w:hyperlink r:id=\"rL\"><w:r><w:t>官网</w:t></w:r></w:hyperlink>";
    let notes = "<w:r><w:footnoteReference w:id=\"2\"/></w:r>\
        <w:r><w:commentReference w:id=\"0\"/></w:r>";
    let body = p("", None, &format!("{image}{link}{notes}"));
    let rels = format!(
        "<Relationship Id=\"rI\" Type=\"{REL}/image\" Target=\"media/image1.png\"/>\
         <Relationship Id=\"rL\" Type=\"{REL}/hyperlink\" Target=\"https://example.com/a b\" \
         TargetMode=\"External\"/>\
         <Relationship Id=\"rF\" Type=\"{REL}/footnotes\" Target=\"footnotes.xml\"/>\
         <Relationship Id=\"rC\" Type=\"{REL}/comments\" Target=\"comments.xml\"/>"
    );
    let footnotes = format!(
        "<w:footnotes {W}><w:footnote w:id=\"2\">{}</w:footnote></w:footnotes>",
        p("", None, &r("脚注内容"))
    );
    let comments = format!(
        "<w:comments {W}><w:comment w:id=\"0\" w:author=\"审阅人\">{}</w:comment></w:comments>",
        p("", None, &r("请补充"))
    );
    let png = b"\x89PNG fake";
    let parts: [(&str, &[u8]); 3] = [
        ("word/media/image1.png", png),
        ("word/footnotes.xml", footnotes.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
    ];
    let converted = convert(&docx(&body, &rels, &parts)).unwrap();
    let comment = t!("preview-docx-comment", author = "审阅人".to_owned());
    let mut escaped = String::new();
    escape(&comment, &mut escaped);
    assert_eq!(
        converted.markdown,
        format!(
            "<img src=\"word/media/image1.png\" alt=\"示意图\" width=\"100\">\
             [官网](<https://example.com/a%20b>)[^1][^2]\n\n[^1]: 脚注内容\n\n\
             [^2]: {escaped}请补充\n"
        )
    );
    assert_eq!(converted.media.len(), 1);
    assert_eq!(converted.media[0].0, "word/media/image1.png");
    assert_eq!(&*converted.media[0].1, png);
}

/// 构造一张图表：类型为 `kind`，各组数据为（名称，分类，数值）。
fn chart(kind: &str, series: &[(&str, &[&str], &[&str])]) -> String {
    let points = |values: &[&str]| {
        values
            .iter()
            .enumerate()
            .map(|(k, v)| format!("<c:pt idx=\"{k}\"><c:v>{v}</c:v></c:pt>"))
            .collect::<String>()
    };
    let series: String = series
        .iter()
        .map(|(name, cats, vals)| {
            format!(
                "<c:ser><c:tx><c:strRef><c:strCache><c:pt idx=\"0\"><c:v>{name}</c:v></c:pt>\
                 </c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache>{}</c:strCache>\
                 </c:strRef></c:cat><c:val><c:numRef><c:numCache>{}</c:numCache></c:numRef>\
                 </c:val></c:ser>",
                points(cats),
                points(vals)
            )
        })
        .collect();
    format!(
        "<c:chartSpace {W}><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>销量</a:t></a:r>\
         </a:p></c:rich></c:tx></c:title><c:plotArea><c:{kind}>{series}</c:{kind}>\
         </c:plotArea></c:chart></c:chartSpace>"
    )
}

#[test]
fn charts_become_pies_tables_or_summaries() {
    let drawing = "<w:r><w:drawing><wp:inline><wp:docPr id=\"1\" name=\"图表 1\"/>\
        <a:graphic><a:graphicData><c:chart r:id=\"rC\"/></a:graphicData></a:graphic>\
        </wp:inline></w:drawing></w:r>";
    let convert_chart = |xml: &str| {
        let rels =
            format!("<Relationship Id=\"rC\" Type=\"{REL}/chart\" Target=\"charts/c.xml\"/>");
        let parts: [(&str, &[u8]); 1] = [("word/charts/c.xml", xml.as_bytes())];
        convert(&docx(&p("", None, drawing), &rels, &parts))
            .unwrap()
            .markdown
    };
    let cats: &[&str] = &["春", "夏"];
    assert_eq!(
        convert_chart(&chart("pieChart", &[("占比", cats, &["3", "5"])])),
        "```mermaid\npie title 销量\n    \"春\" : 3\n    \"夏\" : 5\n```\n"
    );
    let caption = t!("preview-docx-chart-titled", title = "销量".to_owned());
    let category = t!("preview-docx-chart-category");
    let bar = convert_chart(&chart(
        "barChart",
        &[("甲", cats, &["1", "2"]), ("乙", cats, &["3", "4"])],
    ));
    assert_eq!(
        bar,
        format!(
            "<b>{caption}</b>\n\n| {category} | 甲 | 乙 |\n| --- | --- | --- |\n\
             | 春 | 1 | 3 |\n| 夏 | 2 | 4 |\n"
        )
    );
    let scatter = convert_chart(&chart("scatterChart", &[("甲", cats, &["1", "2"])]));
    let summary = t!(
        "preview-docx-chart-series",
        count = "1".to_owned(),
        names = "甲".to_owned()
    );
    assert_eq!(scatter, format!("> <b>{caption}</b><br>{summary}\n"));
}

#[test]
fn numbers_follow_their_formats() {
    assert_eq!(number_text(4, "upperRoman"), "IV");
    assert_eq!(number_text(1994, "lowerRoman"), "mcmxciv");
    assert_eq!(number_text(28, "lowerLetter"), "bb");
    assert_eq!(number_text(23, "chineseCounting"), "二十三");
    assert_eq!(number_text(10, "chineseCounting"), "十");
    assert_eq!(number_text(3, "decimalEnclosedCircle"), "③");
    assert_eq!(number_text(7, "unknownFormat"), "7");
}

#[test]
fn broken_files_report_why() {
    assert_eq!(
        convert(b"not a zip").unwrap_err(),
        t!("preview-docx-invalid")
    );
    let other = crate::zip::build(&[("hello.txt", b"hi")], false);
    assert_eq!(convert(&other).unwrap_err(), t!("preview-docx-invalid"));
    assert!(is_docx(Path::new("报告.DOCX")));
    assert!(is_docx(Path::new("模板.dotx")));
    assert!(!is_docx(Path::new("旧版.doc")));
}
