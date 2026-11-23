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
/// 只有一张工作表、内容为 `sheet` 的工作簿。
fn single_sheet(sheet: &str) -> Vec<u8> {
    let root = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>"
    );
    let workbook = format!(
        "<workbook {NS}><sheets><sheet name=\"表\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
         </workbook>"
    );
    let relations = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>"
    );
    zip(
        &[
            ("_rels/.rels", &root),
            ("xl/workbook.xml", &workbook),
            ("xl/_rels/workbook.xml.rels", &relations),
            ("xl/worksheets/sheet1.xml", sheet),
        ],
        false,
    )
}

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
        assert_eq!((sheet.row_count(), sheet.columns), (5, 6));
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
        assert_eq!(sheet.row(0).collect::<Vec<_>>(), ["名称", "日期", "内联"]);
        // 共享字符串连起各段文字，不含注音；没有文字的单元格和整行都没有文字的行为空。
        assert_eq!(sheet.cell(4, 0), "富文本 ");
        assert_eq!(sheet.cell(1, 0), "");
        assert!(sheet.row(2).next().is_none() && sheet.row(3).next().is_none());
        // 内置的日期格式和百分比格式，以及自定义的日期格式；带时刻的序列号按格式只写日期。
        assert_eq!(sheet.cell(1, 1), "2025-01-01");
        assert_eq!(sheet.cell(1, 2), "12.50%");
        assert_eq!(sheet.cell(1, 3), "0.3");
        assert_eq!(sheet.cell(4, 1), "2025-01-01");
        assert_eq!((sheet.cell(4, 2), sheet.cell(4, 3)), ("TRUE", "#DIV/0!"));
        assert!(!sheet.truncated);

        assert_eq!(table.sheets[1].row_count(), 0);
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
    // 小数位数取自文件：位数极大的格式代码按上限处理，能正常写出。
    let wide = classify(&format!("0.{}%", "0".repeat(100_000)));
    assert_eq!(wide, NumberFormat::Percent(30));
    assert!(super::xlsx::format_number(0.5, wide, false).starts_with("50.000"));
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
        false,
    );
    assert_eq!(sheet.row_count(), 3);
    assert_eq!(sheet.row(0).collect::<Vec<_>>(), ["名称", "备注"]);
    assert_eq!(sheet.row(1).collect::<Vec<_>>(), ["a,b", "第一行\n第二行"]);
    // 行末的空单元格和文件末尾的空行不计入。
    assert_eq!(sheet.row(2).collect::<Vec<_>>(), ["说\"好\""]);
    assert_eq!(sheet.columns, 2);

    // 按第一行判断分隔符，逗号是小数点时用分号；扩展名为 tsv 时总是用制表符。
    let semicolons = read_delimited("a;b;c\n1,5;2;3", false);
    assert_eq!(semicolons.row(1).collect::<Vec<_>>(), ["1,5", "2", "3"]);
    let tabs = read_delimited("a,b\tc", true);
    assert_eq!(tabs.row(0).collect::<Vec<_>>(), ["a,b", "c"]);
    assert_eq!(read_delimited("", false).row_count(), 0);
}

#[test]
fn rows_and_columns_beyond_the_limits_are_cut() {
    let wide = vec!["x"; MAX_COLUMNS + 5].join(",");
    let sheet = read_delimited(&wide, false);
    assert_eq!((sheet.columns, sheet.truncated), (MAX_COLUMNS, true));

    let tall = "1\n".repeat(MAX_ROWS + 3);
    let sheet = read_delimited(&tall, false);
    assert_eq!((sheet.row_count(), sheet.truncated), (MAX_ROWS, true));
    let exact = "1\n".repeat(MAX_ROWS);
    assert!(!read_delimited(&exact, false).truncated);
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
    let mut sheet = read_delimited("a,b,c\nd,\"e\tf\",\"say \"\"hi\"\"\"\n,,x", false);
    sheet.set_merges(vec![Area::spanning((1, 1), (2, 2))]);
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

#[test]
fn sheets_keep_empty_cells_and_rows_in_place() {
    let sheet = read_delimited("a,,c,,\n\n,,\nlast\n\n\n", false);
    assert_eq!((sheet.row_count(), sheet.columns), (4, 3));
    assert_eq!(sheet.row(0).collect::<Vec<_>>(), ["a", "", "c"]);
    assert!(sheet.row(1).next().is_none() && sheet.row(2).next().is_none());
    assert_eq!(
        (sheet.cell(0, 2), sheet.cell(0, 3), sheet.cell(3, 0)),
        ("c", "", "last")
    );
    assert_eq!((sheet.cell(1, 0), sheet.cell(9, 0)), ("", ""));
}

#[test]
fn delimited_files_are_read_from_disk_up_to_the_row_limit() {
    let dir = std::env::temp_dir().join(format!("winc-table-disk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rows.csv");
    std::fs::write(&path, "name,count\napple,3\n\"pear\npie\",5\n").unwrap();
    let table = load_delimited(&path, false).unwrap();
    let sheet = &table.sheets[0];
    assert_eq!((sheet.row_count(), sheet.truncated), (3, false));
    assert_eq!(sheet.row(2).collect::<Vec<_>>(), ["pear\npie", "5"]);

    assert!(load_delimited(&dir.join("missing.csv"), false).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 改为一段一段读入之前的解析方式：整段文本一次解析。留作对照，新的解析器必须给出相同的结果。
fn reference_delimited(text: &str, tabs: bool) -> Sheet {
    let delimiter = if tabs { '\t' } else { sniff_delimiter(text) };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut rows = Rows::default();
    let mut field = String::new();
    let mut quoted = false;
    let mut truncated = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                push_field(&mut rows, &mut field, &mut truncated);
                rows.end_row();
                if rows.len() == MAX_ROWS {
                    truncated |= chars.peek().is_some();
                    break;
                }
            }
            c if c == delimiter => push_field(&mut rows, &mut field, &mut truncated),
            _ => field.push(c),
        }
    }
    if !field.is_empty() || rows.cells() > 0 {
        push_field(&mut rows, &mut field, &mut truncated);
        rows.end_row();
    }
    rows.finish(String::new(), truncated)
}

/// 一段一段从磁盘读的结果与整段文本一次解析的相同：带引号的字段、回车换行、多字节字符和
/// 无效的字节落在两段的交界上也一样；超过行数上限的文件停在上限并记下截断。
#[test]
fn delimited_files_read_in_pieces_match_the_whole_text() {
    let dir = std::env::temp_dir().join(format!("winc-table-pieces-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let same = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let read = load_delimited(&path, false).unwrap().sheets.remove(0);
        let text = String::from_utf8_lossy(bytes);
        assert!(read == read_delimited(&text, false), "{name}");
        assert!(
            read == reference_delimited(&text, false),
            "{name}：与原来的解析不同"
        );
        read
    };

    // 约 5 MiB，每行长度不同：各种写法都会落在 1 MiB 一段的交界上。
    let mut mixed = String::from("\u{feff}");
    for i in 0..60_000 {
        let pad = "汉字é".repeat(i % 7);
        mixed.push_str(&format!("{i},\"引号\"\"里, 换\r\n行{pad}\",尾{pad}\r\n"));
        if i % 3 == 0 {
            mixed.push_str("单独的回车\r");
        }
    }
    let sheet = same("mixed.csv", mixed.as_bytes());
    assert!(!sheet.truncated && sheet.row_count() > 60_000);

    // 无效的字节，以及在段的交界上被切开的多字节字符。
    let mut broken = vec![b'a'; (1 << 20) - 1];
    broken.extend_from_slice("汉,\u{1f600}\n".as_bytes());
    broken.extend_from_slice(&[0xff, b',', 0xe6, 0xb1, b'\n', 0xe6]);
    same("broken.csv", &broken);

    // 比行数上限多：停在上限，后面的不读。
    let tall = "1,2\n".repeat(MAX_ROWS + 5000);
    let sheet = same("tall.csv", tall.as_bytes());
    assert_eq!((sheet.row_count(), sheet.truncated), (MAX_ROWS, true));
    let exact = "1,2\r\n".repeat(MAX_ROWS);
    let sheet = same("exact.csv", exact.as_bytes());
    assert_eq!((sheet.row_count(), sheet.truncated), (MAX_ROWS, false));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 行号、列号不按升序写出的工作表：各行、各单元格仍在所写的位置上。
#[test]
fn rows_and_cells_written_out_of_order_keep_their_places() {
    let sheet = format!(
        "<worksheet {NS}><sheetData>\
         <row r=\"3\"><c r=\"B3\" t=\"str\"><v>三乙</v></c><c r=\"A3\" t=\"str\"><v>三甲</v></c></row>\
         <row r=\"1\"><c r=\"A1\" t=\"str\"><v>a &amp; b &#x4E2D;</v></c></row>\
         <row r=\"5\"/><row><c t=\"inlineStr\"><is><t><![CDATA[<六>]]></t></is></c></row>\
         </sheetData><mergeCells><mergeCell ref=\"A3:B3\"/></mergeCells></worksheet>"
    );
    let table = read_workbook(&single_sheet(&sheet)).unwrap();
    let sheet = &table.sheets[0];
    assert_eq!(sheet.row_count(), 6);
    assert_eq!(sheet.row(0).collect::<Vec<_>>(), ["a & b 中"]);
    assert!(sheet.row(1).next().is_none());
    assert_eq!(sheet.row(2).collect::<Vec<_>>(), ["三甲", "三乙"]);
    assert_eq!(sheet.cell(5, 0), "<六>");
    assert_eq!(sheet.merges.len(), 1);
}

#[test]
fn the_pull_reader_reports_malformed_xml() {
    use crate::xml::{Token, Tokens, attribute};
    let xml = "<?xml version=\"1.0\"?><!-- c --><x:a k='1 > 0' r:id=\"&lt;\"><b/>t</x:a>";
    let mut tokens = Tokens::new(xml);
    let Some(Token::Open {
        name: "a",
        attributes,
        empty: false,
    }) = tokens.next()
    else {
        panic!("应为开始标签");
    };
    assert_eq!(attribute(attributes, "k").as_deref(), Some("1 > 0"));
    assert_eq!(attribute(attributes, "id").as_deref(), Some("<"));
    assert_eq!(attribute(attributes, "missing"), None);
    assert_eq!(tokens.by_ref().count(), 3);
    assert!(tokens.well_formed());
    for broken in ["<a><b></a>", "<a>", "<a", "<a></b>", "</a>"] {
        let mut tokens = Tokens::new(broken);
        tokens.by_ref().for_each(drop);
        assert!(!tokens.well_formed(), "{broken}");
    }
}

/// 只在最后一列有内容的行：空单元格的数量有上限，超出后不再读，工作表记为没有读完；有内容的
/// 单元格再多也完整读取。
#[test]
fn rows_with_one_distant_cell_are_read_up_to_the_padding_limit() {
    let sheet = |rows: usize, cell: &str| {
        let row = format!("<row>{cell}</row>");
        let xml = format!(
            "<worksheet><sheetData>{}</sheetData></worksheet>",
            row.repeat(rows)
        );
        let context = xlsx::Context::default();
        xlsx::worksheet(&xml, String::new(), &context).unwrap()
    };
    // 每行 16383 个空单元格：一千多行就超过上限。
    let sparse = sheet(2000, "<c r=\"XFD1\"><v>1</v></c>");
    assert!(sparse.truncated);
    assert!(sparse.row_count() < 1100, "{}", sparse.row_count());
    // 每行 20 个有内容的单元格，一共 100 万个：不受空单元格的上限影响。
    let cells = "<c><v>1</v></c>".repeat(20);
    let dense = sheet(50_000, &cells);
    assert!(!dense.truncated);
    assert_eq!((dense.row_count(), dense.columns), (50_000, 20));
}

/// 包里各个文件各自不大，逐个读下去累计解压量超过上限时不再解压，并记下被拒绝过；上限按
/// 包的大小成比例，不低于单个文件允许的量。
#[test]
fn an_archive_stops_inflating_past_its_budget() {
    let part = vec![b'a'; 64 << 10];
    let files: Vec<(&str, &[u8])> = vec![("a", &part), ("b", &part), ("c", &part)];
    let bytes = crate::zip::build(&files, true);
    // 三个文件各 64 KiB，压缩后整个包不到 1 KiB：比例远超正常文档。
    assert!(bytes.len() < 1024, "{}", bytes.len());

    // 默认的上限不低于 1 GiB：小的包仍能全部读出。
    let roomy = crate::zip::Archive::open(&bytes).unwrap();
    for name in ["a", "b", "c"] {
        assert_eq!(roomy.read(name).unwrap().unwrap().len(), part.len());
    }
    assert!(!roomy.exceeded());

    // 上限 150 KiB：前两个读得出，第三个被拒绝；没有的文件不算被拒绝。
    let tight = crate::zip::Archive::open(&bytes)
        .unwrap()
        .with_budget(150 << 10);
    assert!(tight.read("a").unwrap().is_ok() && tight.read("b").unwrap().is_ok());
    assert!(tight.read("missing").is_none() && !tight.exceeded());
    assert_eq!(tight.read("c").unwrap(), Err(crate::zip::Error::TooLarge));
    assert!(tight.exceeded());
}

/// 工作簿把同一个工作表文件列了两次：只读一次。
#[test]
fn a_worksheet_listed_twice_is_read_once() {
    let bytes = workbook(true);
    let archive = crate::zip::Archive::open(&bytes).unwrap();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let names = [
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/sharedStrings.xml",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet2.xml",
        "xl/worksheets/sheet3.xml",
    ];
    for name in names {
        let mut content = archive.read(name).unwrap().unwrap();
        if name == "xl/workbook.xml" {
            let text = String::from_utf8(content).unwrap().replace(
                "<sheet name=\"空表\"",
                "<sheet name=\"再来\" sheetId=\"9\" r:id=\"rId1\"/><sheet name=\"空表\"",
            );
            content = text.into_bytes();
        }
        files.push((name.to_owned(), content));
    }
    let files: Vec<(&str, &[u8])> = (files.iter())
        .map(|(name, content)| (name.as_str(), content.as_slice()))
        .collect();
    let table = read_workbook(&crate::zip::build(&files, true)).unwrap();
    let names: Vec<&str> = table.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["数据", "空表"]);
}

/// 按行查找合并区域的结果与逐个比较的相同：高的、宽的、挨着的区域混在一起，次序打乱。
#[test]
fn merges_are_found_like_a_scan_of_all_of_them() {
    let merges = vec![
        Area {
            top: 40,
            left: 0,
            bottom: 40,
            right: 3,
        },
        Area {
            top: 0,
            left: 9,
            bottom: 99,
            right: 9,
        },
        Area {
            top: 5,
            left: 0,
            bottom: 7,
            right: 2,
        },
        Area {
            top: 5,
            left: 3,
            bottom: 5,
            right: 4,
        },
        Area {
            top: 8,
            left: 0,
            bottom: 8,
            right: 8,
        },
        Area {
            top: 60,
            left: 2,
            bottom: 80,
            right: 5,
        },
        Area {
            top: 99,
            left: 0,
            bottom: 99,
            right: 1,
        },
    ];
    let mut sheet = Sheet::default();
    sheet.set_merges(merges.clone());
    for row in 0..=100 {
        for column in 0..=10 {
            let scanned = merges.iter().find(|m| m.contains(row, column)).copied();
            assert_eq!(sheet.merge_at(row, column), scanned, "{row}, {column}");
        }
    }
    // 选中合并区域的一部分即选中整个区域，连带再碰到的区域。
    let cell = |row, column| Area::spanning((row, column), (row, column));
    assert_eq!(sheet.cover_merges(cell(6, 1)), merges[2]);
    let grown = sheet.cover_merges(Area::spanning((5, 2), (5, 3)));
    assert_eq!(
        grown,
        Area {
            top: 5,
            left: 0,
            bottom: 7,
            right: 4
        }
    );
    assert_eq!(sheet.cover_merges(cell(50, 4)), cell(50, 4));
}

/// 测量用，默认不运行：一万个合并区域的工作表上，量列宽时逐格查找所在合并区域的耗时。
#[test]
#[ignore = "测量用"]
fn merge_lookup_cost() {
    let mut sheet = Sheet {
        columns: 200,
        ..Sheet::default()
    };
    // 一万个互不重叠的小合并区域，铺在 5000 行里：每行两个。
    let merges = (0..10_000)
        .map(|i| {
            let (row, left) = (i / 2, (i % 2) * 100);
            Area {
                top: row,
                left,
                bottom: row,
                right: left + 3,
            }
        })
        .collect();
    sheet.set_merges(merges);
    let start = std::time::Instant::now();
    let mut found = 0usize;
    for row in 0..1000 {
        for column in 0..200 {
            found += usize::from(sheet.merge_at(row, column).is_some());
        }
    }
    eprintln!("1000 行 × 200 列：{:?}，命中 {found}", start.elapsed());
    assert_eq!(found, 1000 * 8);
}
