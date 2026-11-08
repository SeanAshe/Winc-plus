//! 表格读取的测试：csv 的引号和分隔符、列名、zip 压缩包、xlsx 的共享字符串、内联文字、空单元格、
//! 日期和百分比、隐藏的工作表，以及损坏的文件。

use super::xlsx::{NumberFormat, classify, column_of, date_time, general};
use super::*;

/// 把 `files`（包中路径、文字内容）写成 zip，见 [`crate::zip::build`]。
fn zip(files: &[(&str, &str)], deflate: bool) -> Vec<u8> {
    let files: Vec<(&str, &[u8])> = files.iter().map(|(n, c)| (*n, c.as_bytes())).collect();
    crate::zip::build(&files, deflate)
}

const NS: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
    xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// 构造一个工作簿：两张显示的工作表（「数据」「空表」）、一张隐藏的工作表，带共享字符串和样式。
fn workbook(deflate: bool) -> Vec<u8> {
    let root_rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>"
    );
    let workbook = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <workbook {NS}><workbookPr/><sheets>\
         <sheet name=\"数据\" sheetId=\"1\" r:id=\"rId1\"/>\
         <sheet name=\"隐藏\" sheetId=\"2\" state=\"hidden\" r:id=\"rId2\"/>\
         <sheet name=\"空表\" sheetId=\"3\" r:id=\"rId3\"/>\
         </sheets></workbook>"
    );
    let workbook_rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         <Relationship Id=\"rId2\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet2.xml\"/>\
         <Relationship Id=\"rId3\" Type=\"{REL}/worksheet\" Target=\"/xl/worksheets/sheet3.xml\"/>\
         <Relationship Id=\"rId4\" Type=\"{REL}/sharedStrings\" Target=\"sharedStrings.xml\"/>\
         <Relationship Id=\"rId5\" Type=\"{REL}/styles\" Target=\"styles.xml\"/>\
         </Relationships>"
    );
    let shared = format!(
        "<sst {NS} count=\"3\" uniqueCount=\"3\">\
         <si><t>名称</t></si>\
         <si><r><t>富</t></r><r><rPr><b/></rPr><t xml:space=\"preserve\">文本 </t></r>\
         <rPh sb=\"0\" eb=\"1\"><t>フ</t></rPh></si>\
         <si><t>日期</t></si></sst>"
    );
    let styles = format!(
        "<styleSheet {NS}><numFmts count=\"1\">\
         <numFmt numFmtId=\"164\" formatCode=\"yyyy&quot;年&quot;m&quot;月&quot;d&quot;日&quot;\"/>\
         </numFmts><cellXfs count=\"4\">\
         <xf numFmtId=\"0\"/><xf numFmtId=\"14\"/><xf numFmtId=\"10\"/><xf numFmtId=\"164\"/>\
         </cellXfs></styleSheet>"
    );
    // 第 2 行缺 A 列，第 3、4 行整行为空；带公式的单元格只取缓存的结果。
    let sheet1 = format!(
        "<worksheet {NS}><sheetData>\
         <row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>2</v></c>\
         <c r=\"C1\" t=\"inlineStr\"><is><t>内联</t></is></c></row>\
         <row r=\"2\"><c r=\"B2\" s=\"1\"><v>45658</v></c><c r=\"C2\" s=\"2\"><v>0.125</v></c>\
         <c r=\"D2\"><f>0.1+0.2</f><v>0.30000000000000004</v></c></row>\
         <row r=\"5\"><c r=\"A5\" t=\"s\"><v>1</v></c><c r=\"B5\" s=\"3\"><v>45658.5</v></c>\
         <c r=\"C5\" t=\"b\"><v>1</v></c><c r=\"D5\" t=\"e\"><v>#DIV/0!</v></c></row>\
         </sheetData><mergeCells count=\"2\"><mergeCell ref=\"A1:B1\"/>\
         <mergeCell ref=\"C5:F9\"/></mergeCells></worksheet>"
    );
    let sheet2 = format!(
        "<worksheet {NS}><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData></worksheet>"
    );
    let sheet3 = format!("<worksheet {NS}><sheetData/></worksheet>");
    zip(
        &[
            ("_rels/.rels", &root_rels),
            ("xl/workbook.xml", &workbook),
            ("xl/_rels/workbook.xml.rels", &workbook_rels),
            ("xl/sharedStrings.xml", &shared),
            ("xl/styles.xml", &styles),
            ("xl/worksheets/sheet1.xml", &sheet1),
            ("xl/worksheets/sheet2.xml", &sheet2),
            ("xl/worksheets/sheet3.xml", &sheet3),
        ],
        deflate,
    )
}

#[test]
fn workbooks_give_the_visible_sheets_with_formatted_cells() {
    for deflate in [false, true] {
        let table = read_workbook(&workbook(deflate)).unwrap();
        let names: Vec<&str> = table.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["数据", "空表"]);

        let sheet = &table.sheets[0];
        // 合并区域伸到第 F 列：列数按合并区域算；伸到最后一行之外的截到最后一行。
        assert_eq!((sheet.rows.len(), sheet.columns), (5, 6));
        let wide = Area {
            top: 4,
            left: 2,
            bottom: 4,
            right: 5,
        };
        assert_eq!(sheet.merge_at(4, 4), Some(wide));
        assert_eq!(
            sheet.merge_at(0, 1).map(|m| (m.left, m.right)),
            Some((0, 1))
        );
        assert_eq!(sheet.merge_at(1, 1), None);
        assert_eq!(sheet.rows[0], ["名称", "日期", "内联"]);
        // 共享字符串连起各段文字，不含注音；没有文字的单元格和整行都没有文字的行为空。
        assert_eq!(sheet.cell(4, 0), "富文本 ");
        assert_eq!(sheet.cell(1, 0), "");
        assert!(sheet.rows[2].is_empty() && sheet.rows[3].is_empty());
        // 内置的日期格式和百分比格式，以及自定义的日期格式；带时刻的序列号按格式只写日期。
        assert_eq!(sheet.cell(1, 1), "2025-01-01");
        assert_eq!(sheet.cell(1, 2), "12.50%");
        assert_eq!(sheet.cell(1, 3), "0.3");
        assert_eq!(sheet.cell(4, 1), "2025-01-01");
        assert_eq!((sheet.cell(4, 2), sheet.cell(4, 3)), ("TRUE", "#DIV/0!"));
        assert!(!sheet.truncated);

        assert_eq!(table.sheets[1].rows.len(), 0);
    }
}

#[test]
fn broken_files_report_why() {
    // 不是 zip 压缩包。
    assert_eq!(
        read_workbook(b"not a zip"),
        Err(winc_i18n::t!("preview-table-invalid").to_owned())
    );
    // 是 zip 压缩包，但没有工作簿文件。
    let other = zip(&[("hello.txt", "hi")], false);
    assert!(read_workbook(&other).is_err());
    // 把工作表 XML 中开始标签的首个尖括号改坏，解析中断。
    let mut broken = workbook(false);
    let at = broken.windows(9).position(|w| w == b"sheetData").unwrap();
    broken[at] = b'<';
    assert!(read_workbook(&broken).is_err());
}

#[test]
fn cell_references_and_column_names_round_trip() {
    assert_eq!(column_of("A1"), Some(0));
    assert_eq!(column_of("Z9"), Some(25));
    assert_eq!(column_of("AA10"), Some(26));
    assert_eq!(column_of("xfd1"), Some(16_383));
    assert_eq!(column_of("12"), None);
    for index in [0, 25, 26, 51, 701, 702, 16_383] {
        let name = column_name(index);
        assert_eq!(column_of(&format!("{name}1")), Some(index), "{name}");
    }
    assert_eq!(column_name(0), "A");
    assert_eq!(column_name(27), "AB");
}

#[test]
fn number_formats_follow_the_format_code() {
    let date = |date, time, seconds| NumberFormat::Date {
        date,
        time,
        seconds,
    };
    assert_eq!(classify("yyyy/mm/dd"), date(true, false, false));
    assert_eq!(classify("h:mm:ss"), date(false, true, true));
    assert_eq!(classify("yyyy-mm-dd hh:mm"), date(true, true, false));
    assert_eq!(classify("0.0%"), NumberFormat::Percent(1));
    assert_eq!(classify("#,##0.00"), NumberFormat::General);
    // 引号中的文字、方括号中的颜色和币种不算。
    assert_eq!(classify("0 \"days\""), NumberFormat::General);
    assert_eq!(classify("[Red][$¥-804]#,##0"), NumberFormat::General);

    assert_eq!(general(0.1 + 0.2), "0.3");
    assert_eq!(general(42.0), "42");
    assert_eq!(general(-1.5), "-1.5");
    assert_eq!(general(1e20), "100000000000000000000");

    // 1900 年起算：1 是 1900-01-01，61 是 1900-03-01；1904 年起算：0 是 1904-01-01。
    let day = |serial, date1904| date_time(serial, date1904, true, false, false).unwrap();
    assert_eq!(day(1.0, false), "1900-01-01");
    assert_eq!(day(61.0, false), "1900-03-01");
    assert_eq!(day(45658.0, false), "2025-01-01");
    assert_eq!(day(0.0, true), "1904-01-01");
    assert_eq!(
        date_time(0.75, false, false, true, true).as_deref(),
        Some("18:00:00")
    );
    assert_eq!(date_time(-1.0, false, true, false, false), None);
}

#[test]
fn delimited_text_handles_quotes_and_delimiters() {
    let sheet = read_delimited(
        "\u{feff}名称,备注\r\n\"a,b\",\"第一行\n第二行\"\n\"说\"\"好\"\"\",\n\n",
        None,
    );
    assert_eq!(sheet.rows.len(), 3);
    assert_eq!(sheet.rows[0], ["名称", "备注"]);
    assert_eq!(sheet.rows[1], ["a,b", "第一行\n第二行"]);
    // 行末的空单元格和文件末尾的空行不计入。
    assert_eq!(sheet.rows[2], ["说\"好\""]);
    assert_eq!(sheet.columns, 2);

    // 按第一行判断分隔符，逗号是小数点时用分号；扩展名为 tsv 时总是用制表符。
    let semicolons = read_delimited("a;b;c\n1,5;2;3", None);
    assert_eq!(semicolons.rows[1], ["1,5", "2", "3"]);
    let tabs = read_delimited("a,b\tc", Some(Path::new("x.tsv")));
    assert_eq!(tabs.rows[0], ["a,b", "c"]);
    assert_eq!(read_delimited("", None).rows.len(), 0);
}

#[test]
fn rows_and_columns_beyond_the_limits_are_cut() {
    let wide = vec!["x"; MAX_COLUMNS + 5].join(",");
    let sheet = read_delimited(&wide, None);
    assert_eq!((sheet.columns, sheet.truncated), (MAX_COLUMNS, true));

    let tall = "1\n".repeat(MAX_ROWS + 3);
    let sheet = read_delimited(&tall, None);
    assert_eq!((sheet.rows.len(), sheet.truncated), (MAX_ROWS, true));
    let exact = "1\n".repeat(MAX_ROWS);
    assert!(!read_delimited(&exact, None).truncated);
}

#[test]
fn file_kinds_follow_the_extension() {
    assert!(is_workbook(Path::new("a/B.XLSX")));
    assert!(is_workbook(Path::new("macro.xlsm")));
    assert!(!is_workbook(Path::new("old.xls")));
    assert!(is_delimited(Path::new("data.CSV")));
    assert!(is_delimited(Path::new("data.tsv")));
    assert!(!is_delimited(Path::new("data.txt")));
}

#[test]
fn selections_cover_merges_and_copy_as_tab_separated_text() {
    let mut sheet = read_delimited("a,b,c\nd,\"e\tf\",\"say \"\"hi\"\"\"\n,,x", None);
    sheet.merges = vec![Area::spanning((1, 1), (2, 2))];
    // 选到合并区域的一角：扩大到整个区域；连锁重叠的也包含进来。
    assert_eq!(
        sheet.cover_merges(Area::spanning((0, 1), (1, 1))),
        Area::spanning((0, 1), (2, 2))
    );
    assert_eq!(
        sheet.cover_merges(Area::spanning((0, 0), (0, 0))),
        Area::default()
    );

    // 一格：原样；多格：制表符分隔，含制表符、双引号的格加引号。
    assert_eq!(sheet.copy_text(Area::spanning((1, 1), (1, 1))), "e\tf");
    // 正好是一个合并区域：也只是左上角那一格的文字。
    assert_eq!(sheet.copy_text(Area::spanning((1, 1), (2, 2))), "e\tf");
    assert_eq!(
        sheet.copy_text(Area::spanning((0, 0), (1, 2))),
        "a\tb\tc\nd\t\"e\tf\"\t\"say \"\"hi\"\"\"\n"
    );
    assert_eq!(sheet.copy_text(Area::spanning((2, 0), (2, 2))), "\t\tx\n");
}
