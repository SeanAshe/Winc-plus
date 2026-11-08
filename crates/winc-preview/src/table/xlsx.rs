//! 读 xlsx 工作簿（Office Open XML）：工作簿文件列出各张工作表，共享字符串表存放文字单元格的
//! 内容，样式表给出数字的格式。
//!
//! 只读显示的文字：公式取缓存的结果（`<v>`），不重新计算；隐藏的工作表和图表工作表不显示。

use std::collections::HashMap;

use roxmltree::{Document, Node};
use winc_i18n::t;

use super::{Area, MAX_COLUMNS, MAX_ROWS, Sheet, Table};
use crate::ooxml::{package_target, parse, part, relations};
use crate::zip::{self, Archive};

/// 工作簿文件不在包关系中时的默认位置。
const DEFAULT_WORKBOOK: &str = "xl/workbook.xml";

/// 从 `bytes` 读 xlsx 工作簿。
///
/// 只收下显示的工作表；一张都没有时按损坏处理。
///
/// # Errors
///
/// 不是 zip 压缩包、不是 xlsx，或其中的 XML 损坏时返回给用户看的原因。
pub(super) fn read(bytes: &[u8]) -> Result<Table, String> {
    let archive = Archive::open(bytes).map_err(|e| match e {
        zip::Error::Invalid => t!("preview-table-invalid").to_owned(),
        zip::Error::Unsupported => t!("preview-table-unsupported").to_owned(),
    })?;
    let workbook_path = package_target(&archive).unwrap_or_else(|| DEFAULT_WORKBOOK.to_owned());
    let workbook = part(&archive, &workbook_path)?.ok_or(t!("preview-table-invalid"))?;
    let workbook = parse(&workbook, &workbook_path)?;
    let relations = relations(&archive, &workbook_path)?;

    let date1904 = workbook
        .descendants()
        .find(|n| n.tag_name().name() == "workbookPr")
        .and_then(|n| n.attribute("date1904"))
        .is_some_and(|v| v == "1" || v == "true");
    let related = |kind: &str| {
        relations
            .values()
            .find(|(k, _)| k.ends_with(kind))
            .map(|(_, target)| target.clone())
    };
    let shared = match related("/sharedStrings") {
        Some(path) => match part(&archive, &path)? {
            Some(bytes) => shared_strings(&parse(&bytes, &path)?),
            None => Vec::new(),
        },
        None => Vec::new(),
    };
    let formats = match related("/styles") {
        Some(path) => match part(&archive, &path)? {
            Some(bytes) => number_formats(&parse(&bytes, &path)?),
            None => Vec::new(),
        },
        None => Vec::new(),
    };
    let context = Context {
        shared,
        formats,
        date1904,
    };

    let mut sheets = Vec::new();
    for node in workbook
        .descendants()
        .filter(|n| n.tag_name().name() == "sheet")
    {
        if matches!(node.attribute("state"), Some("hidden" | "veryHidden")) {
            continue;
        }
        let id = node
            .attributes()
            .find(|a| a.name() == "id")
            .map(|a| a.value());
        let Some((kind, path)) = id.and_then(|id| relations.get(id)) else {
            continue;
        };
        if !kind.ends_with("/worksheet") {
            continue;
        }
        let Some(bytes) = part(&archive, path)? else {
            continue;
        };
        let name = node.attribute("name").unwrap_or_default().to_owned();
        sheets.push(worksheet(&parse(&bytes, path)?, name, &context));
    }
    if sheets.is_empty() {
        return Err(t!("preview-table-no-sheets").to_owned());
    }
    Ok(Table { sheets })
}

/// 读工作表时用到的、工作簿级的公共数据。
struct Context {
    /// 共享字符串表。
    shared: Vec<String>,
    /// 各单元格样式（按样式序号）的数字格式。
    formats: Vec<NumberFormat>,
    /// 日期是否从 1904 年起算（旧版 macOS 上的表格软件写出的工作簿用这种起算方式）。
    date1904: bool,
}

/// 数字的显示格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NumberFormat {
    /// 常规：最多 15 位有效数字。
    General,
    /// 日期和时间：`date`、`time` 是否显示日期、时间，`seconds` 表示时间是否带秒。
    Date {
        date: bool,
        time: bool,
        seconds: bool,
    },
    /// 百分比，括号中是小数位数。
    Percent(usize),
}

/// 共享字符串表：每项是一段文字，由 `<si>` 中各段 `<t>` 连成，不含注音 `<rPh>`。
fn shared_strings(doc: &Document<'_>) -> Vec<String> {
    doc.root_element()
        .children()
        .filter(|n| n.tag_name().name() == "si")
        .map(|si| rich_text(si))
        .collect()
}

/// `node` 中各段 `<t>` 文字连起来，跳过注音 `<rPh>`。
fn rich_text(node: Node<'_, '_>) -> String {
    let mut text = String::new();
    for t in node.descendants().filter(|n| n.tag_name().name() == "t") {
        if t.ancestors().any(|a| a.tag_name().name() == "rPh") {
            continue;
        }
        text.push_str(t.text().unwrap_or_default());
    }
    text
}

/// 样式表中各单元格样式（`cellXfs` 中的 `xf`，按顺序）的数字格式。
fn number_formats(doc: &Document<'_>) -> Vec<NumberFormat> {
    let custom: HashMap<u32, &str> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "numFmt")
        .filter_map(|n| {
            let id = n.attribute("numFmtId")?.parse().ok()?;
            Some((id, n.attribute("formatCode")?))
        })
        .collect();
    let Some(xfs) = doc.descendants().find(|n| n.tag_name().name() == "cellXfs") else {
        return Vec::new();
    };
    xfs.children()
        .filter(|n| n.tag_name().name() == "xf")
        .map(|xf| {
            let id: u32 = xf
                .attribute("numFmtId")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            match custom.get(&id) {
                Some(code) => classify(code),
                None => builtin(id),
            }
        })
        .collect()
}

/// 内置的数字格式编号对应的格式。
fn builtin(id: u32) -> NumberFormat {
    let date = |date, time, seconds| NumberFormat::Date {
        date,
        time,
        seconds,
    };
    match id {
        // 9、10 是内置的百分比格式（分别为 0 位、2 位小数）。
        9 => NumberFormat::Percent(0),
        10 => NumberFormat::Percent(2),
        // 14–17 是日期；27–36、50–58 是东亚语言版本中的日期。
        14..=17 | 27..=36 | 50..=58 => date(true, false, false),
        18 | 20 => date(false, true, false),
        19 | 21 | 45..=47 => date(false, true, true),
        22 => date(true, true, false),
        _ => NumberFormat::General,
    }
}

/// 自定义格式代码 `code` 对应的格式：去掉引号中的文字、方括号中的条件和颜色、转义的字符后，
/// 有年、日或月是日期，有时、秒是时间，有 `%` 是百分比。
pub(super) fn classify(code: &str) -> NumberFormat {
    // 正数、负数、零各一段时只看第一段。
    let mut plain = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            ';' => break,
            _ => plain.push(c.to_ascii_lowercase()),
        }
    }
    let has = |c: char| plain.contains(c);
    let time = has('h') || has('s');
    let date = has('y') || has('d') || (has('m') && !time);
    if date || time {
        NumberFormat::Date {
            date,
            time,
            seconds: has('s'),
        }
    } else if has('%') {
        let decimals = plain.split_once('.').map_or(0, |(_, rest)| {
            rest.chars().take_while(|&c| c == '0').count()
        });
        NumberFormat::Percent(decimals)
    } else {
        NumberFormat::General
    }
}

/// 读一张工作表。
fn worksheet(doc: &Document<'_>, name: String, context: &Context) -> Sheet {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    let Some(data) = doc
        .descendants()
        .find(|n| n.tag_name().name() == "sheetData")
    else {
        return Sheet::from_rows(name, rows, false);
    };
    let mut next_row = 0;
    for row in data.children().filter(|n| n.tag_name().name() == "row") {
        let index = row
            .attribute("r")
            .and_then(|r| r.parse::<usize>().ok())
            .and_then(|r| r.checked_sub(1))
            .unwrap_or(next_row);
        next_row = index + 1;
        if index >= MAX_ROWS {
            truncated = true;
            break;
        }
        let mut cells = Vec::new();
        let mut next_column = 0;
        for cell in row.children().filter(|n| n.tag_name().name() == "c") {
            let column = cell
                .attribute("r")
                .and_then(column_of)
                .unwrap_or(next_column);
            next_column = column + 1;
            if column >= MAX_COLUMNS {
                truncated = true;
                break;
            }
            let value = cell_text(cell, context);
            if value.is_empty() {
                continue;
            }
            if cells.len() <= column {
                cells.resize(column + 1, String::new());
            }
            cells[column] = value;
        }
        if rows.len() <= index {
            rows.resize(index + 1, Vec::new());
        }
        rows[index] = cells;
    }
    let mut sheet = Sheet::from_rows(name, rows, truncated);
    sheet.merges = merges(doc, sheet.rows.len());
    // 合并区域伸到有文字的列之外时，列数按合并区域算，区域完整显示。
    let widest = sheet.merges.iter().map(|m| m.right + 1).max().unwrap_or(0);
    sheet.columns = sheet.columns.max(widest);
    sheet
}

/// 工作表中的合并区域（`<mergeCell ref="A1:C2"/>`）；只保留从前 `rows` 行开始的、在列数
/// 上限以内的区域，伸到 `rows` 行之外的截到最后一行。
fn merges(doc: &Document<'_>, rows: usize) -> Vec<Area> {
    doc.descendants()
        .filter(|n| n.tag_name().name() == "mergeCell")
        .filter_map(|n| {
            let (from, to) = n.attribute("ref")?.split_once(':')?;
            let (top, left) = cell_of(from)?;
            let (bottom, right) = cell_of(to)?;
            let merge = Area {
                top: top.min(bottom),
                left: left.min(right),
                bottom: top.max(bottom).min(rows.saturating_sub(1)),
                right: left.max(right).min(MAX_COLUMNS - 1),
            };
            (merge.top < rows && merge.left < MAX_COLUMNS).then_some(merge)
        })
        .collect()
}

/// 单元格引用（如 `B3`）对应的（行号，列号），都从 0 起。
fn cell_of(reference: &str) -> Option<(usize, usize)> {
    let column = column_of(reference)?;
    let digits = reference.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let row = digits.parse::<usize>().ok()?.checked_sub(1)?;
    Some((row, column))
}

/// 返回单元格引用（如 `AB12`）中的列号（从 0 起）；没有字母时返回 `None`。
pub(super) fn column_of(reference: &str) -> Option<usize> {
    let letters = reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .try_fold(0usize, |n, b| {
            n.checked_mul(26)?
                .checked_add(usize::from(b.to_ascii_uppercase() - b'A') + 1)
        })?;
    letters.checked_sub(1)
}

/// 单元格 `cell` 按 `context` 中的共享字符串和样式显示的文字。
fn cell_text(cell: Node<'_, '_>, context: &Context) -> String {
    let child = |name: &str| cell.children().find(|n| n.tag_name().name() == name);
    let value = child("v").and_then(|v| v.text()).unwrap_or_default();
    match cell.attribute("t") {
        Some("s") => value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| context.shared.get(i))
            .cloned()
            .unwrap_or_default(),
        Some("inlineStr") => child("is").map(rich_text).unwrap_or_default(),
        Some("b") => match value.trim() {
            "1" => "TRUE".to_owned(),
            "0" => "FALSE".to_owned(),
            other => other.to_owned(),
        },
        Some("str" | "e" | "d") => value.to_owned(),
        _ => {
            let format = cell
                .attribute("s")
                .and_then(|s| s.parse::<usize>().ok())
                .and_then(|s| context.formats.get(s))
                .copied()
                .unwrap_or(NumberFormat::General);
            match value.trim().parse::<f64>() {
                Ok(number) => format_number(number, format, context.date1904),
                Err(_) => value.to_owned(),
            }
        }
    }
}

/// 按 `format` 写出数字 `number`。
pub(super) fn format_number(number: f64, format: NumberFormat, date1904: bool) -> String {
    match format {
        NumberFormat::General => general(number),
        NumberFormat::Percent(decimals) => format!("{:.*}%", decimals, number * 100.0),
        NumberFormat::Date {
            date,
            time,
            seconds,
        } => date_time(number, date1904, date, time, seconds).unwrap_or_else(|| general(number)),
    }
}

/// 按常规格式写出 `number`：最多 15 位有效数字，去掉末尾的 0，`0.1 + 0.2` 写成 `0.3`。
pub(super) fn general(number: f64) -> String {
    if !number.is_finite() {
        return number.to_string();
    }
    let rounded: f64 = format!("{number:.14e}").parse().unwrap_or(number);
    rounded.to_string()
}

/// 把序列号 `serial` 写成日期或时间，按 `date`、`time` 决定显示哪一部分。
///
/// `serial` 是从 1900 年或 1904 年起算的天数，小数部分是一天中的时刻；超出范围时返回 `None`。
pub(super) fn date_time(
    serial: f64,
    date1904: bool,
    date: bool,
    time: bool,
    seconds: bool,
) -> Option<String> {
    // 序列号的范围是 0 到 9999-12-31（2958465 天）；超出时按常规数字显示。
    if !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let mut total = (serial * 86_400.0).round() as i64;
    // 1900 年起算时，序列号 60 是并不存在的 1900-02-29（xlsx 沿用的历史错误）：此后的
    // 日子从 1899-12-30 起算，之前的从 1899-12-31 起算。
    let epoch = if date1904 {
        days_from_civil(1904, 1, 1)
    } else if serial < 60.0 {
        days_from_civil(1899, 12, 31)
    } else {
        days_from_civil(1899, 12, 30)
    };
    let days = total.div_euclid(86_400);
    total = total.rem_euclid(86_400);
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    let (year, month, day) = civil_from_days(epoch + days);
    let clock = if seconds {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}")
    };
    Some(match (date, time) {
        (true, true) => format!("{year:04}-{month:02}-{day:02} {clock}"),
        (false, true) => clock,
        _ => format!("{year:04}-{month:02}-{day:02}"),
    })
}

/// 公历日期（`year` 年 `month` 月 `day` 日）到 1970-01-01 的天数。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 从 1970-01-01 起 `days` 天之后的公历日期（年，月，日）。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
