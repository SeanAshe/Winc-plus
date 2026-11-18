//! 读 xlsx 工作簿（Office Open XML）：工作簿文件列出各张工作表，共享字符串表存放文字单元格的
//! 内容，样式表给出数字的格式。
//!
//! 只读显示的文字：公式取缓存的结果（`<v>`），不重新计算；隐藏的工作表和图表工作表不显示。
//! 工作表和共享字符串表逐项读出（见 [`pull`](super::pull)），不建立整棵树。

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use roxmltree::Document;
use winc_i18n::t;

use super::{Area, MAX_COLUMNS, MAX_ROWS, Rows, Sheet, Table};
use crate::ooxml::{package_target, parse, part, relations, text};
use crate::xml::{Token, Tokens, attribute};
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
            Some(bytes) => shared_strings(text(&bytes, &path)?).ok_or_else(|| damaged(&path))?,
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
        let sheet = worksheet(text(&bytes, path)?, name, &context);
        sheets.push(sheet.ok_or_else(|| damaged(path))?);
    }
    if sheets.is_empty() {
        return Err(t!("preview-table-no-sheets").to_owned());
    }
    Ok(Table { sheets })
}

/// 包中的 `path` 损坏时给用户看的原因。
fn damaged(path: &str) -> String {
    t!("preview-office-damaged", part = path.to_owned())
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
fn shared_strings(xml: &str) -> Option<Vec<String>> {
    let mut strings = Vec::new();
    let mut current: Option<String> = None;
    let (mut reading, mut phonetic) = (false, 0usize);
    let mut tokens = Tokens::new(xml);
    for token in &mut tokens {
        match token {
            Token::Open {
                name: "si", empty, ..
            } => match empty {
                true => strings.push(String::new()),
                false => current = Some(String::new()),
            },
            Token::Open {
                name: "rPh",
                empty: false,
                ..
            } => phonetic += 1,
            Token::Open {
                name: "t",
                empty: false,
                ..
            } => reading = phonetic == 0,
            Token::Text(text) if reading => {
                if let Some(current) = &mut current {
                    current.push_str(&text);
                }
            }
            Token::Close("t") => reading = false,
            Token::Close("rPh") => phonetic = phonetic.saturating_sub(1),
            Token::Close("si") => strings.extend(current.take()),
            _ => {}
        }
    }
    tokens.well_formed().then_some(strings)
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

/// 正在读的一个单元格。
#[derive(Default)]
struct Cell {
    /// 列号（从 0 起）。
    column: usize,
    /// 类型（属性 `t`）；没有该属性时为空。
    kind: String,
    /// 样式序号（属性 `s`）。
    style: Option<usize>,
    /// `<v>` 中的文字。
    value: String,
    /// `<is>` 中各段 `<t>` 连成的文字。
    inline: String,
}

/// 读一张工作表。
///
/// 行、单元格通常按行号、列号升序写出，逐行追加；行号小于已有行数的行先记下，读完后放回
/// 所在的位置。
fn worksheet(xml: &str, name: String, context: &Context) -> Option<Sheet> {
    let mut rows = Rows::default();
    let mut truncated = false;
    let mut late: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut merges: Vec<Area> = Vec::new();
    // 当前行各列的文字，逐行重复使用；`used` 是当前行用到的列数。
    let (mut cells, mut used): (Vec<String>, usize) = (Vec::new(), 0);
    // 当前行的行号，不在行中或该行超出上限时为 `None`；`next` 是没有写行号、列号时的默认值。
    let (mut row, mut next_row, mut next_column) = (None, 0usize, 0usize);
    let mut cell: Option<Cell> = None;
    let (mut in_value, mut in_inline, mut in_text, mut phonetic) = (false, false, false, 0usize);
    let mut end_row = |index: usize, cells: &mut [String], rows: &mut Rows| {
        if index < rows.len() {
            late.insert(index, cells.to_vec());
            return;
        }
        while rows.len() < index {
            rows.end_row();
        }
        cells.iter().for_each(|cell| rows.push(cell));
        rows.end_row();
    };
    let mut tokens = Tokens::new(xml);
    for token in &mut tokens {
        match token {
            Token::Open {
                name: "row",
                attributes,
                empty,
            } => {
                let index = attribute(attributes, "r")
                    .and_then(|r| r.parse::<usize>().ok())
                    .and_then(|r| r.checked_sub(1))
                    .unwrap_or(next_row);
                next_row = index + 1;
                cells[..used].iter_mut().for_each(String::clear);
                (used, next_column) = (0, 0);
                row = (index < MAX_ROWS).then_some(index);
                truncated |= row.is_none();
                if let Some(index) = row.filter(|_| empty) {
                    end_row(index, &mut cells[..0], &mut rows);
                    row = None;
                }
            }
            Token::Open {
                name: "c",
                attributes,
                empty,
            } if row.is_some() => {
                let column = attribute(attributes, "r")
                    .and_then(|r| column_of(&r))
                    .unwrap_or(next_column);
                next_column = column + 1;
                if column >= MAX_COLUMNS {
                    truncated = true;
                    continue;
                }
                let style = attribute(attributes, "s").and_then(|s| s.parse().ok());
                let kind = attribute(attributes, "t")
                    .map(Cow::into_owned)
                    .unwrap_or_default();
                cell = (!empty).then(|| Cell {
                    column,
                    kind,
                    style,
                    ..Cell::default()
                });
            }
            Token::Open {
                name: "v",
                empty: false,
                ..
            } => in_value = cell.is_some(),
            Token::Open {
                name: "is",
                empty: false,
                ..
            } => in_inline = cell.is_some(),
            Token::Open {
                name: "rPh",
                empty: false,
                ..
            } => phonetic += 1,
            Token::Open {
                name: "t",
                empty: false,
                ..
            } => in_text = in_inline && phonetic == 0,
            Token::Open {
                name: "mergeCell",
                attributes,
                ..
            } => {
                let area = attribute(attributes, "ref").and_then(|range| {
                    let (from, to) = range.split_once(':')?;
                    let ((top, left), (bottom, right)) = (cell_of(from)?, cell_of(to)?);
                    Some(Area {
                        top: top.min(bottom),
                        left: left.min(right),
                        bottom: top.max(bottom),
                        right: left.max(right),
                    })
                });
                merges.extend(area);
            }
            Token::Text(text) => match &mut cell {
                Some(cell) if in_value => cell.value.push_str(&text),
                Some(cell) if in_text => cell.inline.push_str(&text),
                _ => {}
            },
            Token::Close("v") => in_value = false,
            Token::Close("t") => in_text = false,
            Token::Close("is") => in_inline = false,
            Token::Close("rPh") => phonetic = phonetic.saturating_sub(1),
            Token::Close("c") => {
                if let Some(cell) = cell.take() {
                    if cells.len() <= cell.column {
                        cells.resize(cell.column + 1, String::new());
                    }
                    used = used.max(cell.column + 1);
                    cells[cell.column].clear();
                    write_cell(&mut cells[cell.column], &cell, context);
                }
            }
            Token::Close("row") => {
                if let Some(index) = row.take() {
                    end_row(index, &mut cells[..used], &mut rows);
                }
            }
            _ => {}
        }
    }
    if !tokens.well_formed() {
        return None;
    }
    let mut sheet = rows.finish(name, truncated);
    if !late.is_empty() {
        sheet = sheet.with_rows(&late);
    }
    // 合并区域只保留从已读的行开始、在列数上限以内的，伸到最后一行之外的截到最后一行。
    let count = sheet.row_count();
    merges.retain(|m| m.top < count && m.left < MAX_COLUMNS);
    for merge in &mut merges {
        merge.bottom = merge.bottom.min(count.saturating_sub(1));
        merge.right = merge.right.min(MAX_COLUMNS - 1);
    }
    sheet.merges = merges;
    // 合并区域伸到有文字的列之外时，列数按合并区域算，区域完整显示。
    let widest = sheet.merges.iter().map(|m| m.right + 1).max().unwrap_or(0);
    sheet.columns = sheet.columns.max(widest);
    Some(sheet)
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

/// 把单元格 `cell` 按 `context` 中的共享字符串和样式显示的文字写入 `out`。
fn write_cell(out: &mut String, cell: &Cell, context: &Context) {
    let value = cell.value.as_str();
    match cell.kind.as_str() {
        "s" => {
            let shared = value.trim().parse::<usize>().ok();
            out.push_str(
                shared
                    .and_then(|i| context.shared.get(i))
                    .map_or("", String::as_str),
            );
        }
        "inlineStr" => out.push_str(&cell.inline),
        "b" => out.push_str(match value.trim() {
            "1" => "TRUE",
            "0" => "FALSE",
            other => other,
        }),
        "str" | "e" | "d" => out.push_str(value),
        _ => {
            let format = (cell.style.and_then(|s| context.formats.get(s)).copied())
                .unwrap_or(NumberFormat::General);
            match value.trim().parse::<f64>() {
                Ok(number) => out.push_str(&format_number(number, format, context.date1904)),
                Err(_) => out.push_str(value),
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
