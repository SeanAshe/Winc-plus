//! 表格的读取：xlsx 工作簿和 csv、tsv 文本读成按行排列的单元格文字，供界面只读显示。
//!
//! xlsx 是 zip 压缩包中的一组 XML（`zip` 读包，`xlsx` 读工作表）。
//!
//! 只取每个单元格显示的文字，不读公式、样式和图表；数字按单元格的格式写成日期、时间或百分比，
//! 其余按常规格式。行数、列数超过 `MAX_ROWS`、`MAX_COLUMNS` 的部分不读，工作表记下被截断。

use std::path::Path;

mod source;
mod xlsx;

pub use source::{field_at, field_start};

/// 每张工作表最多读的行数（行）：xlsx 格式规定的行数上限，csv、tsv 取同一个值。
pub const MAX_ROWS: usize = 1_048_576;
/// 每行最多读的列数（列）：xlsx 格式规定的列数上限，csv、tsv 取同一个值。
pub const MAX_COLUMNS: usize = 16_384;
/// 能读的 xlsx 文件的大小上限（字节，256 MiB）。
///
/// 取自调参：再大的工作簿解压后占用内存过多。
pub(crate) const MAX_FILE: u64 = 256 * 1024 * 1024;
/// 一张工作表最多保存的单元格数（个，含行内的空单元格）。
///
/// 每个单元格记一个 4 字节的结束位置，取 2²⁸ 个，结束位置共占 1 GiB，与 [`MAX_TEXT`] 相同；
/// 行数、列数上限以内的表格（例如 100 万行、200 列）都在其内。
pub(crate) const MAX_CELLS: usize = 1 << 28;
/// 一张工作表的文字总量的上限（字节，1 GiB）。
///
/// 共享字符串在每个引用它的单元格里各存一份，文字总量可以远超文件的大小；结束位置以 `u32`
/// 保存，该上限同时保证位置不溢出。
pub(crate) const MAX_TEXT: usize = 1 << 30;

/// 单元格区域，用于合并的单元格和选中的单元格。
///
/// 由上、左、下、右四边的行号和列号围成（都从 0 起，包含两端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Area {
    /// 第一行。
    pub top: usize,
    /// 第一列。
    pub left: usize,
    /// 最后一行。
    pub bottom: usize,
    /// 最后一列。
    pub right: usize,
}

impl Area {
    /// 创建以 `a`、`b` 两格为对角的区域；两格用（行号，列号）表示。
    pub fn spanning(a: (usize, usize), b: (usize, usize)) -> Self {
        Self {
            top: a.0.min(b.0),
            left: a.1.min(b.1),
            bottom: a.0.max(b.0),
            right: a.1.max(b.1),
        }
    }

    /// 是否包含第 `row` 行第 `column` 列。
    pub fn contains(&self, row: usize, column: usize) -> bool {
        (self.top..=self.bottom).contains(&row) && (self.left..=self.right).contains(&column)
    }

    /// 是否与 `other` 有重叠的单元格。
    pub fn overlaps(&self, other: &Area) -> bool {
        self.top <= other.bottom
            && other.top <= self.bottom
            && self.left <= other.right
            && other.left <= self.right
    }
}

/// 一张工作表。
///
/// 单元格的文字连续存放在一个缓冲区中，每个单元格另占 4 字节的偏移：单元格很多时不逐个分配。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    /// 工作表的名称；csv、tsv 为空。
    pub name: String,
    /// 全部单元格的文字，按行、行内按列依次相接。
    text: String,
    /// 各单元格在 `text` 中的结束位置（字节偏移），按行、行内按列排列；行内不含末尾的空单元格。
    ends: Vec<u32>,
    /// 各行在 `ends` 中的结束位置；空行与上一行的值相同。
    row_ends: Vec<u32>,
    /// 最长一行的单元格数（合并区域伸到更右边时按合并区域算）。
    pub columns: usize,
    /// 合并的单元格区域，按第一行、第一列排序；显示的是区域左上角那一格的文字。
    merges: Vec<Area>,
    /// 与 `merges` 等长：到这一项为止各区域最后一行的最大值，查找时据此知道前面不必再看。
    reach: Vec<usize>,
    /// 是否因超过 [`MAX_ROWS`]、[`MAX_COLUMNS`] 或文字总量的上限没有读完。
    pub truncated: bool,
    /// 从分隔符文本读出时分列的字符；工作簿中的表为 `None`。
    pub delimiter: Option<char>,
    /// 数据行与源文本行号之差变化的地方：（数据行，该行起的行号之差），按数据行排序。
    ///
    /// 引号内的字段跨行时，其后各行数据的起始行号比数据行号大；没有跨行的字段时为空。见
    /// [`Sheet::source_line`]。
    shifts: Vec<(u32, u32)>,
}

/// 逐格、逐行建立工作表。
#[derive(Default)]
pub(crate) struct Rows {
    sheet: Sheet,
    /// 是否已到上限：之后的单元格一律不收，不让靠后的单元格填进被丢弃的单元格的位置。
    full: bool,
}

impl Rows {
    /// 当前行已加入的单元格数。
    pub(crate) fn cells(&self) -> usize {
        self.sheet.ends.len() - self.sheet.row_ends.last().map_or(0, |&end| end as usize)
    }

    /// 已结束的行数。
    pub(crate) fn len(&self) -> usize {
        self.sheet.row_ends.len()
    }

    /// 在当前行末尾加入一个单元格；单元格数达到 [`MAX_CELLS`] 或文字总量超过 [`MAX_TEXT`] 时
    /// 丢弃并记下截断。
    pub(crate) fn push(&mut self, cell: &str) {
        let end = self.sheet.text.len() + cell.len();
        self.full |= self.sheet.ends.len() >= MAX_CELLS || end > MAX_TEXT;
        if self.full {
            self.sheet.truncated = true;
            return;
        }
        self.sheet.text.push_str(cell);
        self.sheet.ends.push(end as u32);
    }

    /// 记下下一行数据从源文本的第 `line` 行开始；与按行号顺推的不同时才记。
    fn starts_at(&mut self, line: u32) {
        let row = self.sheet.row_ends.len() as u32;
        let shift = line - row;
        if self.sheet.shifts.last().map_or(0, |&(_, s)| s) != shift {
            self.sheet.shifts.push((row, shift));
        }
    }

    /// 结束当前行，去掉行末的空单元格。
    pub(crate) fn end_row(&mut self) {
        let sheet = &mut self.sheet;
        let start = sheet.row_ends.last().map_or(0, |&end| end as usize);
        let before = |ends: &[u32]| ends.len().checked_sub(2).map_or(0, |i| ends[i]);
        while sheet.ends.len() > start && sheet.ends.last().copied() == Some(before(&sheet.ends)) {
            sheet.ends.pop();
        }
        sheet.columns = sheet.columns.max(sheet.ends.len() - start);
        sheet.row_ends.push(sheet.ends.len() as u32);
    }

    /// 返回建好的工作表，去掉末尾的空行；`truncated` 表示来源没有读完。
    pub(crate) fn finish(mut self, name: String, truncated: bool) -> Sheet {
        let sheet = &mut self.sheet;
        while let Some(&end) = sheet.row_ends.last() {
            let above = sheet.row_ends.len().checked_sub(2);
            if end != above.map_or(0, |i| sheet.row_ends[i]) {
                break;
            }
            sheet.row_ends.pop();
        }
        sheet.text.shrink_to_fit();
        sheet.ends.shrink_to_fit();
        sheet.row_ends.shrink_to_fit();
        sheet.name = name;
        sheet.truncated |= truncated;
        self.sheet
    }
}

impl Sheet {
    /// 返回行数。
    pub fn row_count(&self) -> usize {
        self.row_ends.len()
    }

    /// 返回第 `row` 行（从 0 起）各单元格的文字，不含行末的空单元格；没有这一行时为空。
    pub fn row(&self, row: usize) -> impl Iterator<Item = &str> {
        self.span(row).map(|i| self.text_at(i))
    }

    /// 返回把 `rows` 中各行（键为行号）换成给出的内容后的工作表；合并区域不保留。
    pub(crate) fn with_rows(&self, rows: &std::collections::BTreeMap<usize, Vec<String>>) -> Sheet {
        let last = rows.keys().next_back().map_or(0, |&row| row + 1);
        let mut out = Rows::default();
        for row in 0..self.row_count().max(last) {
            match rows.get(&row) {
                Some(cells) => cells.iter().for_each(|cell| out.push(cell)),
                None => self.row(row).for_each(|cell| out.push(cell)),
            }
            out.end_row();
        }
        out.finish(self.name.clone(), self.truncated)
    }

    /// 返回第 `row` 行的单元格在 `ends` 中的范围；没有这一行时为空。
    fn span(&self, row: usize) -> std::ops::Range<usize> {
        let end = self.row_ends.get(row).map_or(0, |&end| end as usize);
        let start = match row.checked_sub(1) {
            Some(above) => self.row_ends.get(above).map_or(end, |&end| end as usize),
            None => 0,
        };
        start..end
    }

    /// 返回 `ends` 中第 `i` 个单元格的文字。
    fn text_at(&self, i: usize) -> &str {
        let start = i.checked_sub(1).map_or(0, |i| self.ends[i] as usize);
        &self.text[start..self.ends[i] as usize]
    }

    /// 合并的单元格区域，按第一行、第一列排序。
    pub fn merges(&self) -> &[Area] {
        &self.merges
    }

    /// 设置合并的单元格区域。
    pub fn set_merges(&mut self, mut merges: Vec<Area>) {
        merges.sort_by_key(|m| (m.top, m.left));
        let mut lowest = 0;
        self.reach = (merges.iter())
            .map(|m| {
                lowest = lowest.max(m.bottom);
                lowest
            })
            .collect();
        self.merges = merges;
    }

    /// 可能与第 `top` 行到第 `bottom` 行有交集的合并区域：第一行不在 `bottom` 之后，且往前
    /// 直到再没有区域伸到 `top`。
    ///
    /// 区域按第一行排序，通常只有几行高：要看的只是这几行附近的那些，不是全部。
    fn merges_near(&self, top: usize, bottom: usize) -> &[Area] {
        let end = self.merges.partition_point(|m| m.top <= bottom);
        let start = self.reach[..end].partition_point(|&lowest| lowest < top);
        &self.merges[start..end]
    }

    /// 包含第 `row` 行第 `column` 列的合并区域。
    pub fn merge_at(&self, row: usize, column: usize) -> Option<Area> {
        let near = self.merges_near(row, row);
        near.iter().find(|m| m.contains(row, column)).copied()
    }

    /// 把 `area` 扩大到完整包含与它重叠的合并区域（选中合并区域的一部分即选中整个区域）。
    pub fn cover_merges(&self, mut area: Area) -> Area {
        loop {
            let grown = self
                .merges_near(area.top, area.bottom)
                .iter()
                .filter(|m| m.overlaps(&area))
                .fold(area, |a, m| Area {
                    top: a.top.min(m.top),
                    left: a.left.min(m.left),
                    bottom: a.bottom.max(m.bottom),
                    right: a.right.max(m.right),
                });
            if grown == area {
                return area;
            }
            area = grown;
        }
    }

    /// 复制 `area` 得到的文字：一行一行，格与格之间用制表符分开，粘贴到表格软件中仍是这些格。
    ///
    /// 只有一格、或者正好是一个合并区域时就是该格的文字。多格时，含制表符、换行或双引号的格用
    /// 双引号括起，其中的双引号写两遍；合并区域中左上角以外的格为空。
    pub fn copy_text(&self, area: Area) -> String {
        let single = area.top == area.bottom && area.left == area.right;
        if single || self.merges.contains(&area) {
            return self.cell(area.top, area.left).to_owned();
        }
        let mut out = String::new();
        for row in area.top..=area.bottom {
            for column in area.left..=area.right {
                if column > area.left {
                    out.push('\t');
                }
                let text = self.cell(row, column);
                if text.contains(['\t', '\n', '\r', '"']) {
                    out.push('"');
                    out.push_str(&text.replace('"', "\"\""));
                    out.push('"');
                } else {
                    out.push_str(text);
                }
            }
            out.push('\n');
        }
        out
    }

    /// 第 `row` 行第 `column` 列（都从 0 起）的文字；没有时为空。
    pub fn cell(&self, row: usize, column: usize) -> &str {
        let span = self.span(row);
        let cell = span.start.checked_add(column).filter(|i| *i < span.end);
        cell.map_or("", |i| self.text_at(i))
    }
}

/// 一个表格文件中的各张工作表；csv、tsv 只有一张。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// 各张工作表，顺序与文件中的一致。
    pub sheets: Vec<Sheet>,
}

/// `path` 是否是 xlsx 工作簿（按扩展名，不区分大小写）。
///
/// 这类文件以表格打开。
pub fn is_workbook(path: &Path) -> bool {
    extension(path).is_some_and(|e| matches!(e.as_str(), "xlsx" | "xlsm"))
}

/// `path` 是否是以分隔符分列的文本（csv、tsv）。
///
/// 这类文件按文本打开，可以切换到表格预览。
pub fn is_delimited(path: &Path) -> bool {
    extension(path).is_some_and(|e| matches!(e.as_str(), "csv" | "tsv" | "tab"))
}

/// 小写的扩展名。
fn extension(path: &Path) -> Option<String> {
    Some(path.extension()?.to_str()?.to_ascii_lowercase())
}

/// 读 xlsx 工作簿 `path`。
///
/// # Errors
///
/// 文件无法读取、过大、不是 xlsx，或者内容损坏时返回给用户看的原因。
pub fn load_workbook(path: &Path) -> Result<Table, String> {
    let metadata = winc_backend::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len > MAX_FILE {
        return Err(winc_i18n::t!("preview-table-too-large").to_owned());
    }
    let bytes = winc_backend::fs::read(path).map_err(|e| e.to_string())?;
    read_workbook(&bytes)
}

/// 读内存中的 xlsx 工作簿 `bytes`。
///
/// # Errors
///
/// 不是 xlsx，或者内容损坏时返回给用户看的原因。
pub fn read_workbook(bytes: &[u8]) -> Result<Table, String> {
    xlsx::read(bytes)
}

/// 读磁盘上以分隔符分列的文件 `path`，用于原文不在内存中的大文件；`tabs` 的含义同
/// [`read_delimited`]。
///
/// 一段一段地读、边读边解析，到 [`MAX_ROWS`] 行或文字总量的上限就不再往下读：文件再大、
/// 一行再长，读进内存的只有显示得了的部分。无法解码为 UTF-8 的字节替换为 U+FFFD。
///
/// # Errors
///
/// 文件无法读取时返回给用户看的原因。
pub fn load_delimited(path: &Path, tabs: bool) -> Result<Table, String> {
    let mut parser: Option<Delimited> = None;
    let (mut offset, mut carry) = (0u64, Vec::new());
    loop {
        let (chunk, end) =
            winc_backend::fs::read_range(path, offset, READ_PIECE).map_err(|e| e.to_string())?;
        // 未到末尾却没有数据：再读下去不会结束。
        if chunk.is_empty() && !end {
            return Err(winc_i18n::t!("preview-table-invalid").to_owned());
        }
        offset += chunk.len() as u64;
        carry.extend_from_slice(&chunk);
        let text = decode(&mut carry, end);
        let parser = parser.get_or_insert_with(|| {
            Delimited::new(if tabs { '\t' } else { sniff_delimiter(&text) })
        });
        if !parser.feed(&text) || end {
            break;
        }
    }
    let sheet = parser.map_or_else(Sheet::default, Delimited::finish);
    Ok(Table {
        sheets: vec![sheet],
    })
}

/// 读分隔符文件时一次读取的字节数（字节，1 MiB）：远端的文件一次请求最多取回该字节数。
const READ_PIECE: u32 = 1 << 20;

/// 取出 `bytes` 中能解码为 UTF-8 的部分并返回；无效的字节换成 U+FFFD。结尾不完整的一个字符
/// 留在 `bytes` 里等下一段接上，`end` 为真（后面没有了）时同样换成 U+FFFD。
fn decode(bytes: &mut Vec<u8>, end: bool) -> String {
    let mut text = String::new();
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        let error = match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                rest = &[];
                break;
            }
            Err(error) => error,
        };
        let (valid, after) = rest.split_at(error.valid_up_to());
        text.push_str(&String::from_utf8_lossy(valid));
        match error.error_len() {
            Some(invalid) => {
                text.push('\u{fffd}');
                rest = &after[invalid..];
            }
            None if end => {
                text.push('\u{fffd}');
                rest = &[];
            }
            None => {
                rest = after;
                break;
            }
        }
    }
    *bytes = rest.to_vec();
    text
}

/// 读以分隔符分列的文本 `text`。
///
/// `tabs` 为真（TSV）时分隔符总是制表符，否则取第一行中出现最多的逗号、制表符或分号。
/// 按 RFC 4180 处理引号：双引号括起的字段可以包含分隔符、换行，两个双引号表示一个双引号。
pub fn read_delimited(text: &str, tabs: bool) -> Sheet {
    let mut parser = Delimited::new(if tabs { '\t' } else { sniff_delimiter(text) });
    parser.feed(text);
    parser.finish()
}

/// 一段一段读入分隔符文本的解析器：各段依次交给 [`Delimited::feed`]，最后 [`Delimited::finish`]。
///
/// 同时记下各行数据在源文本中的起始行号（[`Sheet::source_line`]）。行尾与文本编辑区相同：换行、
/// 回车和回车换行各算一个。
struct Delimited {
    rows: Rows,
    delimiter: char,
    /// 正在读的字段。
    field: String,
    /// 是否在双引号括起的字段里。
    quoted: bool,
    /// 引号里刚读到一个双引号：下一个字符也是双引号时是写两遍的引号，否则字段的引号到此结束。
    closing: bool,
    /// 刚读到结束一行的回车：紧跟的换行属于同一个行尾。
    after_cr: bool,
    /// 还没有读过任何字符：文本开头的字节序标记不算内容。
    fresh: bool,
    /// 已经读满 [`MAX_ROWS`] 行：此后只看后面还有没有内容。
    done: bool,
    truncated: bool,
    /// 已读过的行尾数，即当前字符所在的源文本行号（从 0 起）。
    line: u32,
    /// 引号里刚读到回车：紧跟的换行属于同一个行尾，不再计数。
    quoted_cr: bool,
}

impl Delimited {
    fn new(delimiter: char) -> Self {
        Self {
            rows: Rows::default(),
            delimiter,
            field: String::new(),
            quoted: false,
            closing: false,
            after_cr: false,
            fresh: true,
            done: false,
            truncated: false,
            line: 0,
            quoted_cr: false,
        }
    }

    /// 读入下一段 `text`；返回 `false` 时已到行数或文字总量的上限，后面的不必再读。
    fn feed(&mut self, text: &str) -> bool {
        let mut text = text;
        if std::mem::take(&mut self.fresh) {
            text = text.strip_prefix('\u{feff}').unwrap_or(text);
        }
        for c in text.chars() {
            if self.closing {
                self.closing = false;
                if c == '"' {
                    self.field.push('"');
                    continue;
                }
                self.quoted = false;
            }
            if self.quoted {
                let quoted_cr = std::mem::take(&mut self.quoted_cr);
                match c {
                    '"' => self.closing = true,
                    _ => self.field.push(c),
                }
                if c == '\r' || (c == '\n' && !quoted_cr) {
                    self.line += 1;
                    self.quoted_cr = c == '\r';
                }
                continue;
            }
            let after_cr = std::mem::take(&mut self.after_cr);
            if after_cr && c == '\n' {
                continue;
            }
            if self.done {
                self.truncated = true;
                return false;
            }
            match c {
                '"' if self.field.is_empty() => self.quoted = true,
                '\n' | '\r' => {
                    self.after_cr = c == '\r';
                    push_field(&mut self.rows, &mut self.field, &mut self.truncated);
                    self.rows.end_row();
                    self.done = self.rows.len() == MAX_ROWS;
                    self.line += 1;
                    self.rows.starts_at(self.line);
                }
                c if c == self.delimiter => {
                    push_field(&mut self.rows, &mut self.field, &mut self.truncated);
                }
                _ => self.field.push(c),
            }
            // 文字总量到了上限：后面的单元格都收不下，不再读。
            if self.rows.full {
                return false;
            }
        }
        true
    }

    /// 结束读入，返回工作表。
    fn finish(mut self) -> Sheet {
        if !self.done && (!self.field.is_empty() || self.rows.cells() > 0) {
            push_field(&mut self.rows, &mut self.field, &mut self.truncated);
            self.rows.end_row();
        }
        let mut sheet = self.rows.finish(String::new(), self.truncated);
        sheet.delimiter = Some(self.delimiter);
        sheet
    }
}

/// 把 `field` 加到 `rows` 当前行的末尾并清空；超过 [`MAX_COLUMNS`] 时丢弃并记下截断。
fn push_field(rows: &mut Rows, field: &mut String, truncated: &mut bool) {
    if rows.cells() < MAX_COLUMNS {
        rows.push(field);
    } else {
        *truncated = true;
    }
    field.clear();
}

/// 第一行（引号之外）中出现最多的逗号、制表符或分号；都没有时为逗号。
fn sniff_delimiter(text: &str) -> char {
    let mut counts = [(',', 0usize), ('\t', 0), (';', 0)];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' | '\r' if !quoted => break,
            _ if !quoted => {
                if let Some(slot) = counts.iter_mut().find(|(d, _)| *d == c) {
                    slot.1 += 1;
                }
            }
            _ => {}
        }
    }
    // 次数相同时取靠前的：逗号优先。
    counts
        .iter()
        .fold(
            (',', 0),
            |best, &(d, n)| if n > best.1 { (d, n) } else { best },
        )
        .0
}

/// 第 `index` 列（从 0 起）的列名：A、B、…、Z、AA、AB、…。
pub fn column_name(index: usize) -> String {
    let mut name = Vec::new();
    let mut n = index + 1;
    while n > 0 {
        n -= 1;
        name.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

#[cfg(test)]
mod tests;
