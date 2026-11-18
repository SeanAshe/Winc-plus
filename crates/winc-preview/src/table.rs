//! 表格的读取：xlsx 工作簿和 csv、tsv 文本读成按行排列的单元格文字，供界面只读显示。
//!
//! xlsx 是 zip 压缩包中的一组 XML（`zip` 读包，`xlsx` 读工作表）。
//!
//! 只取每个单元格显示的文字，不读公式、样式和图表；数字按单元格的格式写成日期、时间或百分比，
//! 其余按常规格式。行数、列数超过 `MAX_ROWS`、`MAX_COLUMNS` 的部分不读，工作表记下被截断。

use std::path::Path;

mod xlsx;

/// 每张工作表最多读的行数（行）：xlsx 格式规定的行数上限，csv、tsv 取同一个值。
pub const MAX_ROWS: usize = 1_048_576;
/// 每行最多读的列数（列）：xlsx 格式规定的列数上限，csv、tsv 取同一个值。
pub const MAX_COLUMNS: usize = 16_384;
/// 能读的 xlsx 文件的大小上限（字节，256 MiB，经验值）。
///
/// 再大的工作簿解压后占用内存过多。
pub(crate) const MAX_FILE: u64 = 256 * 1024 * 1024;

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
    /// 合并的单元格区域；显示的是区域左上角那一格的文字。
    pub merges: Vec<Area>,
    /// 是否因超过 [`MAX_ROWS`]、[`MAX_COLUMNS`] 或文字总量的上限没有读完。
    pub truncated: bool,
}

/// 逐格、逐行建立工作表。
#[derive(Default)]
pub(crate) struct Rows {
    sheet: Sheet,
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

    /// 在当前行末尾加入一个单元格；文字总量超过 `u32` 能表示的范围时丢弃并记下截断。
    pub(crate) fn push(&mut self, cell: &str) {
        match u32::try_from(self.sheet.text.len() + cell.len()) {
            Ok(end) => {
                self.sheet.text.push_str(cell);
                self.sheet.ends.push(end);
            }
            Err(_) => self.sheet.truncated = true,
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

    /// 包含第 `row` 行第 `column` 列的合并区域。
    pub fn merge_at(&self, row: usize, column: usize) -> Option<Area> {
        self.merges
            .iter()
            .find(|m| m.contains(row, column))
            .copied()
    }

    /// 把 `area` 扩大到完整包含与它重叠的合并区域（选中合并区域的一部分即选中整个区域）。
    pub fn cover_merges(&self, mut area: Area) -> Area {
        loop {
            let grown = self
                .merges
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
/// 本机的文件只读取前 [`MAX_ROWS`] 行所需的部分；无法解码为 UTF-8 的字节替换为 U+FFFD。
///
/// # Errors
///
/// 文件无法读取时返回给用户看的原因。
pub fn load_delimited(path: &Path, tabs: bool) -> Result<Table, String> {
    let (bytes, more) = if winc_backend::is_remote(path) {
        (
            winc_backend::fs::read(path).map_err(|e| e.to_string())?,
            false,
        )
    } else {
        head_lines(path, MAX_ROWS + 1).map_err(|e| e.to_string())?
    };
    let mut sheet = read_delimited(&String::from_utf8_lossy(&bytes), tabs);
    sheet.truncated |= more;
    Ok(Table {
        sheets: vec![sheet],
    })
}

/// 读本机文件 `path` 开头的 `lines` 行，返回读到的字节和其后是否还有内容。
#[allow(clippy::disallowed_methods, reason = "调用方已确认路径在本机")]
fn head_lines(path: &Path, lines: usize) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut bytes = Vec::new();
    for _ in 0..lines {
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            return Ok((bytes, false));
        }
    }
    Ok((bytes, !reader.fill_buf()?.is_empty()))
}

/// 读以分隔符分列的文本 `text`。
///
/// `tabs` 为真（TSV）时分隔符总是制表符，否则取第一行中出现最多的逗号、制表符或分号。
/// 按 RFC 4180 处理引号：双引号括起的字段可以包含分隔符、换行，两个双引号表示一个双引号。
pub fn read_delimited(text: &str, tabs: bool) -> Sheet {
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
