//! 行、列、显示列之间的换算，以及 rope 和按需读取的大文件共用的文本接口。
//!
//! [`Lines`] 是只读的行接口，位置用 `char` 偏移；[`EditBuffer`] 提供替换操作；
//! [`LineEnding`] 描述行尾风格，[`CellLayout`] 描述行内列与显示列的换算。

use std::ops::{ControlFlow, Range};

use ropey::{Rope, RopeSlice};
use unicode_width::UnicodeWidthChar;

/// 每行最多显示和高亮的字符数（字符）。
///
/// 更长的行（例如压缩过的 JS）只处理开头的这些字符。取自调参：该上限使超长行的绘制与词法分析
/// 开销有界，常见源代码的行远短于该长度，不受截断影响。
pub const MAX_LINE_CHARS: usize = 10_000;
/// 读取一行开头 [`MAX_LINE_CHARS`] 个字符所需的最大字节数（字节）。
///
/// UTF-8 单个字符最多占 4 字节（RFC 3629）。再多留 2 个字符的余量：读到的字符数超过
/// `MAX_LINE_CHARS` 时，即可判断这一行已被截断。
pub const MAX_LINE_BYTES: usize = (MAX_LINE_CHARS + 2) * 4;

/// 文件的行尾风格。
///
/// 读入时检测，新插入的换行使用同一风格，保存时原样写回。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// LF（`\n`），Unix 风格。
    #[default]
    Lf,
    /// CRLF（`\r\n`），Windows 风格。
    CrLf,
    /// CR（`\r`），经典 Mac OS 风格。
    Cr,
}

impl LineEnding {
    /// 返回行尾对应的字符串（`\n`、`\r\n` 或 `\r`）。
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }

    /// 返回显示用的行尾名称（`LF`、`CRLF`、`CR`）。
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }

    /// 统计文本前 1000 行的行尾，返回出现次数最多的一种。
    ///
    /// 同一文件的行尾风格通常一致，因此只扫描前 1000 行，不遍历全文。1000 行取自调参，混有少量
    /// 其他行尾时仍能得出多数风格。没有换行符时返回 `LF`。
    pub fn detect(text: &Rope) -> LineEnding {
        let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);

        for line in text.lines().take(1000) {
            match ending_of(line) {
                2 => crlf += 1,
                1 if line.char(line.len_chars() - 1) == '\r' => cr += 1,
                1 => lf += 1,
                _ => {}
            }
        }

        if crlf > lf && crlf >= cr {
            LineEnding::CrLf
        } else if cr > lf && cr > crlf {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        }
    }
}

/// 返回 `line` 末尾换行符占用的字符数（0、1 或 2）。
fn ending_of(line: RopeSlice<'_>) -> usize {
    let len = line.len_chars();

    if len == 0 {
        return 0;
    }

    match line.char(len - 1) {
        '\n' if len >= 2 && line.char(len - 2) == '\r' => 2,
        '\n' | '\r' => 1,
        _ => 0,
    }
}

/// 一行内容：借用 rope 中的文本，或持有从磁盘读出的副本（大文件）。
pub enum Line<'a> {
    /// 借用 rope 中的文本。
    Slice(RopeSlice<'a>),
    /// 从磁盘读出的副本（大文件）。
    Owned(String),
}

impl Line<'_> {
    /// 以 [`RopeSlice`] 的形式返回这一行的内容（不含换行符）。
    pub fn slice(&self) -> RopeSlice<'_> {
        match self {
            Line::Slice(slice) => *slice,
            Line::Owned(text) => RopeSlice::from(text.as_str()),
        }
    }
}

/// 按行只读访问的文本接口。
///
/// rope 和大文件都实现 `Lines`，光标移动、编辑、绘制和搜索只依赖该接口。位置是 `char` 偏移，
/// 行号、列号均从 0 起：第 `line` 行第 `col` 个字符的位置是 `line_start(line) + col`。
///
/// 只读打开的超长行文件例外：一行中超出可访问部分的字符仍占用位置，但光标无法到达。
pub trait Lines {
    /// 返回行数。
    ///
    /// 文本以换行符结尾时，末尾多出一个空行，光标可以停在该行。
    fn line_count(&self) -> usize;

    /// 返回第 `line` 行行首的位置；`line` 超过最后一行时按最后一行计算。
    fn line_start(&self, line: usize) -> usize;

    /// 返回 `pos` 所在的行号。
    fn line_of(&self, pos: usize) -> usize;

    /// 返回第 `line` 行的内容，不含换行符。
    fn line_content(&self, line: usize) -> Line<'_>;

    /// 返回第 `line` 行的开头部分，不含换行符。
    ///
    /// 结果至少包含前 `max_bytes` 字节，整行不足时为整行，截断处不切开字符。只需要行首（缩进、
    /// 开头的若干字符）时使用，按需读取的大文件因此不必读出整行；默认实现返回整行。
    fn line_prefix(&self, line: usize, max_bytes: usize) -> Line<'_> {
        let _ = max_bytes;
        self.line_content(line)
    }

    /// 返回位置区间 `from..to` 内的文本，包含其中的换行符。
    fn substring(&self, from: usize, to: usize) -> String;

    /// 返回文本末尾的位置，即文本的总字符数。
    fn end_pos(&self) -> usize;

    /// 返回第 `line` 行的字符数，不含换行符。
    fn line_len(&self, line: usize) -> usize {
        self.line_content(line).slice().len_chars()
    }

    /// 返回第 `line` 行行末（换行符之前）的位置。
    fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_len(line)
    }

    /// 从第 `from` 行起依次对每一行调用 `f`，直到 `f` 返回 `Break` 或到达文本末尾。
    ///
    /// `f` 的参数依次为行号、行首位置和行内容。行内容最多取 `max_bytes` 字节，截断处不切开字符。
    /// `f` 返回 `Break` 时，返回该行的行号；遍历到末尾时返回 `None`。
    fn visit_lines(
        &self,
        from: usize,
        max_bytes: usize,
        mut f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize> {
        let mut text = String::new();

        for line in from..self.line_count() {
            text.clear();
            push_prefix(&mut text, self.line_content(line).slice(), max_bytes);

            if f(line, self.line_start(line), &text).is_break() {
                return Some(line);
            }
        }

        None
    }

    /// 同 [`Lines::visit_lines`]，但超过 `max_bytes` 字节的行不交出内容，以 `None` 表示。
    ///
    /// 供对超长行另有处理的调用方使用（如推算高亮状态时保持原状态），按需读取的大文件
    /// 因此无需读取和解码超长行。
    fn visit_short_lines(
        &self,
        from: usize,
        max_bytes: usize,
        mut f: impl FnMut(usize, usize, Option<&str>) -> ControlFlow<()>,
    ) -> Option<usize> {
        // 截断处不切开字符，多取 4 字节（UTF-8 字符的最大长度）后，超过 `max_bytes` 字节的行
        // 交出的内容必然长于 `max_bytes`。
        self.visit_lines(from, max_bytes.saturating_add(4), |line, start, content| {
            f(line, start, (content.len() <= max_bytes).then_some(content))
        })
    }

    /// 返回 `pos` 处的字符；行尾的换行符一律返回 `'\n'`，`pos` 不在文本范围内时返回 `None`。
    fn char_at(&self, pos: usize) -> Option<char> {
        if pos >= self.end_pos() {
            return None;
        }

        let line = self.line_of(pos);
        let col = pos - self.line_start(line);
        Some(
            self.line_content(line)
                .slice()
                .get_char(col)
                .unwrap_or('\n'),
        )
    }

    /// 返回第 `line` 行的位置范围，包含行尾换行符；用于选中整行和复制整行。
    fn line_span(&self, line: usize) -> Range<usize> {
        let start = self.line_start(line);
        let end = if line + 1 < self.line_count() {
            self.line_start(line + 1)
        } else {
            self.end_pos()
        };
        start..end
    }

    /// 把位置转换为（行号，列号）；`pos` 超过文本末尾时按末尾计算。
    fn pos_to_line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.end_pos());
        let line = self.line_of(pos);
        let col = (pos - self.line_start(line)).min(self.line_len(line));
        (line, col)
    }

    /// 把（行号，列号）转换为位置；行号超过末行时取末行，列号超过行长时取行末。
    fn line_col_to_pos(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count() - 1);
        self.line_start(line) + col.min(self.line_len(line))
    }
}

/// 可修改的文本。
///
/// 编辑事务通过该 trait 应用到 rope 或大文件上。
pub trait EditBuffer {
    /// 删除从 `start` 起的 `removed` 个字符，再在 `start` 处插入 `inserted`。
    ///
    /// # Panics
    ///
    /// 实现方（例如 `Rope`）在 `start + removed` 超过字符数时 panic；`removed` 为 0 且
    /// `inserted` 非空时，`start` 超过字符数同样 panic。
    fn replace(&mut self, start: usize, removed: usize, inserted: &str);

    /// 依次应用 `changes` 中的每一处修改，结果为逐处调用 [`Self::replace`] 后的文本。
    ///
    /// 每一项是起点、删除的字符数和插入的文字，起点是应用之前各项修改后的位置。默认实现逐处
    /// 调用 [`Self::replace`]；逐处修改代价高的实现（大文件的片段表）可以一次完成全部修改。
    fn replace_each(&mut self, changes: &[(usize, usize, &str)]) {
        for &(start, removed, inserted) in changes {
            self.replace(start, removed, inserted);
        }
    }
}

impl Lines for Rope {
    fn line_count(&self) -> usize {
        self.len_lines()
    }

    fn line_start(&self, line: usize) -> usize {
        self.line_to_char(line.min(self.len_lines() - 1))
    }

    fn line_of(&self, pos: usize) -> usize {
        self.char_to_line(pos.min(self.len_chars()))
    }

    fn line_content(&self, line: usize) -> Line<'_> {
        Line::Slice(line_slice(self, line))
    }

    fn substring(&self, from: usize, to: usize) -> String {
        let to = to.min(self.len_chars());
        self.slice(from.min(to)..to).to_string()
    }

    fn end_pos(&self) -> usize {
        self.len_chars()
    }

    fn line_len(&self, line: usize) -> usize {
        let slice = self.line(line);
        slice.len_chars() - ending_of(slice)
    }

    fn visit_lines(
        &self,
        from: usize,
        max_bytes: usize,
        mut f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize> {
        if from >= self.len_lines() {
            return None;
        }

        let mut text = String::new();
        let mut start = self.line_start(from);

        for (index, line) in self.lines_at(from).enumerate() {
            text.clear();
            push_prefix(
                &mut text,
                line.slice(..line.len_chars() - ending_of(line)),
                max_bytes,
            );

            if f(from + index, start, &text).is_break() {
                return Some(from + index);
            }

            start += line.len_chars();
        }

        None
    }

    fn char_at(&self, pos: usize) -> Option<char> {
        self.get_char(pos)
    }
}

impl EditBuffer for Rope {
    fn replace(&mut self, start: usize, removed: usize, inserted: &str) {
        if removed > 0 {
            self.remove(start..start + removed);
        }

        if !inserted.is_empty() {
            self.insert(start, inserted);
        }
    }
}

/// 把 `slice` 追加到 `out`，使 `out` 的长度不超过 `max_bytes`（字节），截断处不切开字符。
fn push_prefix(out: &mut String, slice: RopeSlice<'_>, max_bytes: usize) {
    for chunk in slice.chunks() {
        let room = max_bytes.saturating_sub(out.len());

        if chunk.len() <= room {
            out.push_str(chunk);
            continue;
        }

        let mut cut = room;

        while !chunk.is_char_boundary(cut) {
            cut -= 1;
        }

        out.push_str(&chunk[..cut]);
        break;
    }
}

/// 返回 rope 中第 `line` 行去掉换行符后的切片。
///
/// # Panics
///
/// `line` 不小于 rope 的行数时 panic。
pub fn line_slice(text: &Rope, line: usize) -> RopeSlice<'_> {
    let slice = text.line(line);
    slice.slice(..slice.len_chars() - ending_of(slice))
}

/// 返回字符 `c` 从显示列 `cell` 起占用的显示列数。
///
/// 制表符补齐到下一个制表位；没有显示宽度的字符（如控制字符）返回 0。
///
/// # Panics
///
/// `c` 是制表符且 `tab_width` 为 0 时 panic。
#[inline(always)]
pub fn char_cells(c: char, cell: usize, tab_width: usize) -> usize {
    match c {
        '\t' => tab_width - cell % tab_width,
        // 可打印 ASCII 字符占 1 个显示列，无需查 Unicode 宽度表。断行与测宽都逐字符调用本函数，
        // 该分支随调用方内联，查表放在单独的函数中。
        ' '..='~' => 1,
        _ => table_cells(c),
    }
}

/// 按 Unicode 宽度表返回 `c` 占的显示列数；没有显示宽度的字符返回 0。
#[inline(never)]
fn table_cells(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// 返回 `line` 前 `col` 个字符占用的显示宽度（显示列数）。
///
/// # Panics
///
/// `line` 的前 `col` 个字符中含制表符且 `tab_width` 为 0 时 panic。
pub fn col_to_cell(line: RopeSlice<'_>, col: usize, tab_width: usize) -> usize {
    line.chars()
        .take(col)
        .fold(0, |cell, c| cell + char_cells(c, cell, tab_width))
}

/// 返回 `chars` 从显示列 `start` 起依次排下之后到达的显示列。
///
/// 制表符排到下一个制表位，宽字符占 2 个显示列。自动换行时后续各段从缩进处起排，`start` 即为
/// 缩进宽度；第一段从行首起排，`start` 为 0。
///
/// # Panics
///
/// `chars` 中含制表符且 `tab_width` 为 0 时 panic。
pub fn slice_cells(chars: &[char], start: usize, tab_width: usize) -> usize {
    chars
        .iter()
        .fold(start, |cell, &c| cell + char_cells(c, cell, tab_width))
}

/// 返回 `chars` 中每个字符起始处的显示列，共 `chars.len() + 1` 项。
///
/// 第 `i` 项是前 `i` 个字符排下之后到达的显示列，末项为全部字符排下之后的显示列。同一行内
/// 需要换算多个位置（查找结果、选区）时，先计算整张表，每个位置再各查表一次。
///
/// # Panics
///
/// `chars` 中含制表符且 `tab_width` 为 0 时 panic。
pub fn prefix_cells(chars: &[char], tab_width: usize) -> Vec<usize> {
    if chars.iter().all(|c| (' '..='~').contains(c)) {
        return (0..=chars.len()).collect();
    }
    let mut cell = 0;
    let mut out = Vec::with_capacity(chars.len() + 1);
    out.push(0);
    for &c in chars {
        cell += char_cells(c, cell, tab_width);
        out.push(cell);
    }
    out
}

/// 行内列与显示列的换算方式。
///
/// `usize` 的实现把自身当作制表位宽度，按 [`col_to_cell`] 换算；CSV、TSV 按列对齐时使用
/// [`Aligned`]。
pub trait CellLayout {
    /// 返回 `line` 前 `col` 个字符占用的显示宽度（显示列数）。
    fn col_to_cell(&self, line: RopeSlice<'_>, col: usize) -> usize;
    /// 把显示列 `cell` 转换为 `line` 的行内列；落在字符中间时取距离更近的一侧。
    fn cell_to_col(&self, line: RopeSlice<'_>, cell: usize) -> usize;
}

impl CellLayout for usize {
    fn col_to_cell(&self, line: RopeSlice<'_>, col: usize) -> usize {
        col_to_cell(line, col, *self)
    }

    fn cell_to_col(&self, line: RopeSlice<'_>, cell: usize) -> usize {
        cell_to_col(line, cell, *self)
    }
}

/// 按分隔符分列（CSV 的逗号、TSV 的制表符）时各列的起始显示列。
///
/// 只影响显示，不改动文本。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Columns {
    /// 分隔各列的字符，例如 CSV 的 `,` 和 TSV 的制表符。
    pub separator: char,
    /// 各列的起始显示列：第 `i` 列从 `starts[i]` 开始，`starts[0]` 为 0。
    ///
    /// 超出 `starts` 的列紧接在分隔符之后绘制。
    pub starts: Vec<usize>,
}

/// 相邻两列之间的最小空隙（显示列）。
///
/// 取能分开相邻两列内容的最小值 1：为 0 时相邻列的内容连在一起，取值更大则占用更多水平空间。
const COLUMN_GAP: usize = 1;

impl Columns {
    /// 根据文本内容计算各列的起始显示列。
    ///
    /// 每列的宽度取该列所有字段中最宽者，再加上分隔符（1 个显示列）和 `COLUMN_GAP`。
    /// 引号内的分隔符不分列，`"a, b"` 是一个字段。
    ///
    /// `chunks` 是整个文件的文本，按段给出（例如 rope 的 `chunks()`），整体只遍历一遍。
    pub fn measure<'a>(
        separator: char,
        chunks: impl IntoIterator<Item = &'a str>,
        tab_width: usize,
    ) -> Columns {
        ColumnWidths::measure(separator, chunks, tab_width).columns()
    }

    /// 计算 `line` 前 `limit` 个字符的排布，写入调用方给出的缓冲区（先清空）。
    ///
    /// `cells` 依次是每个字符起始处的显示列，末尾多一项，为这些字符绘制结束处的显示列；`fields`
    /// 依次是每个字符所属的字段序号（从 0 起）。分隔符属于其前面的字段，其后的字符从下一列的
    /// 起始显示列开始；引号内的分隔符不分列。
    ///
    /// 缓冲区由调用方复用：每帧绘制都要为每个可见行计算一次，复用后无需逐行分配内存。
    pub fn layout(
        &self,
        line: RopeSlice<'_>,
        tab_width: usize,
        limit: usize,
        cells: &mut Vec<usize>,
        mut fields: Option<&mut Vec<usize>>,
    ) {
        cells.clear();
        if let Some(fields) = fields.as_deref_mut() {
            fields.clear();
        }
        let (mut cell, mut field, mut quoted) = (0, 0, false);
        for c in line.chars().take(limit) {
            cells.push(cell);
            if let Some(fields) = fields.as_deref_mut() {
                fields.push(field);
            }
            if c == '"' {
                quoted = !quoted;
            }
            if c == self.separator && !quoted {
                cell += 1;
                field += 1;
                if let Some(&start) = self.starts.get(field) {
                    cell = cell.max(start);
                }
            } else {
                cell += char_cells(c, cell, tab_width);
            }
        }
        cells.push(cell);
    }
}

/// 逐个字段调用 `f(字段序号, 显示宽度)`，字段序号从 0 起。
///
/// 分隔符结束当前字段，其宽度不计入字段。换行（`\n`、`\r`）开始新的一行并重置引号状态；
/// 引号内的分隔符不结束字段。
fn for_each_field<'a>(
    chunks: impl IntoIterator<Item = &'a str>,
    separator: char,
    tab_width: usize,
    mut f: impl FnMut(usize, usize),
) {
    let (mut field, mut width, mut quoted) = (0, 0, false);
    for chunk in chunks {
        // ASCII 字节无需解码和查宽度表：0x20 及以上的可打印字符占 1 个显示列，其余控制字符占
        // 0 个显示列（制表符和换行在下面单独处理）。
        for (i, &b) in chunk.as_bytes().iter().enumerate() {
            if b >= 0x80 {
                // 只处理多字节字符的首字节（0xC0 及以上），续字节（0x80 至 0xBF）跳过。
                if b >= 0xC0
                    && let Some(c) = chunk[i..].chars().next()
                {
                    if c == separator && !quoted {
                        f(field, width);
                        field += 1;
                        width = 0;
                    } else {
                        width += char_cells(c, width, tab_width);
                    }
                }
                continue;
            }
            let c = b as char;
            if c == '\n' || c == '\r' {
                f(field, width);
                (field, width, quoted) = (0, 0, false);
                continue;
            }
            if c == '"' {
                quoted = !quoted;
            }
            if c == separator && !quoted {
                f(field, width);
                field += 1;
                width = 0;
            } else if c == '\t' {
                width += char_cells(c, width, tab_width);
            } else {
                width += usize::from(b >= 0x20);
            }
        }
    }
    f(field, width);
}

/// 按列对齐时各列的最大宽度，可以随改动增量更新，输入时不必重新测量整个文件。
///
/// 每列记录最宽字段的宽度，以及同为最宽的字段个数。改动后先计入新文字中的字段，再减去旧文字
/// 中的字段；某一列最宽的字段全部被删除时无法得知次宽字段的宽度，此时
/// [`ColumnWidths::update`] 返回 `false`，由调用方重新测量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnWidths {
    /// 分隔各列的字符。
    pub separator: char,
    /// 制表位宽度（显示列）。
    pub tab_width: usize,
    /// 各列最宽的字段的显示宽度。
    widths: Vec<usize>,
    /// 各列中宽度等于该列最大宽度的字段个数。
    holders: Vec<usize>,
}

impl ColumnWidths {
    /// 测量 `chunks`（整个文件的文本，按段给出）各列的宽度，规则同 [`Columns::measure`]。
    pub fn measure<'a>(
        separator: char,
        chunks: impl IntoIterator<Item = &'a str>,
        tab_width: usize,
    ) -> ColumnWidths {
        let mut widths = ColumnWidths {
            separator,
            tab_width,
            widths: Vec::new(),
            holders: Vec::new(),
        };
        widths.add(chunks);
        widths
    }

    /// 把文字 `removed` 替换为 `inserted` 后更新各列宽度。
    ///
    /// 两段都必须是完整的行：从行首开始，到换行或文件末尾结束。
    ///
    /// 某一列最宽的字段全部被删除时结果不再准确，返回 `false`，调用方应重新测量整个文件。
    pub fn update<'a, 'b>(
        &mut self,
        removed: impl IntoIterator<Item = &'a str>,
        inserted: impl IntoIterator<Item = &'b str>,
    ) -> bool {
        self.add(inserted);
        let (widths, holders) = (&self.widths, &mut self.holders);
        let mut exact = true;
        for_each_field(removed, self.separator, self.tab_width, |field, width| {
            if widths.get(field) == Some(&width) {
                holders[field] = holders[field].saturating_sub(1);
                exact &= holders[field] > 0;
            }
        });
        exact
    }

    fn add<'a>(&mut self, chunks: impl IntoIterator<Item = &'a str>) {
        let (widths, holders) = (&mut self.widths, &mut self.holders);
        for_each_field(chunks, self.separator, self.tab_width, |field, width| {
            if field >= widths.len() {
                widths.resize(field + 1, 0);
                holders.resize(field + 1, 0);
            }
            if width > widths[field] {
                (widths[field], holders[field]) = (width, 1);
            } else if width == widths[field] {
                holders[field] += 1;
            }
        });
    }

    /// 各列的起始显示列。
    pub fn columns(&self) -> Columns {
        let mut starts = Vec::with_capacity(self.widths.len().max(1));
        let mut at = 0;
        for width in &self.widths {
            starts.push(at);
            // 分隔符占 1 个显示列，之后再留出 `COLUMN_GAP` 的空隙。
            at += width + 1 + COLUMN_GAP;
        }
        if starts.is_empty() {
            starts.push(0);
        }
        Columns {
            separator: self.separator,
            starts,
        }
    }
}

/// 按列对齐的显示列换算：分隔符之后的字符从下一列的起始显示列开始绘制。
#[derive(Debug, Clone, Copy)]
pub struct Aligned<'a> {
    /// 各列的起始显示列。
    pub columns: &'a Columns,
    /// 制表位宽度（显示列）。
    pub tab_width: usize,
}

impl CellLayout for Aligned<'_> {
    fn col_to_cell(&self, line: RopeSlice<'_>, col: usize) -> usize {
        let mut cells = Vec::new();
        self.columns
            .layout(line, self.tab_width, col, &mut cells, None);
        cells[cells.len() - 1]
    }

    fn cell_to_col(&self, line: RopeSlice<'_>, cell: usize) -> usize {
        let mut cells = Vec::new();
        self.columns
            .layout(line, self.tab_width, usize::MAX, &mut cells, None);
        nearest_col(&cells, cell)
    }
}

/// 返回 `positions` 中距离显示列 `cell` 最近的字符边界（行内列）。
///
/// `positions` 是每个字符的起始显示列，末项是行尾所在的显示列。
///
/// # Panics
///
/// `positions` 为空时，debug 构建因减法溢出而 panic。
pub(crate) fn nearest_col(positions: &[usize], cell: usize) -> usize {
    let last = positions.len() - 1;
    let after = positions.partition_point(|&p| p <= cell);
    if after == 0 {
        return 0;
    }
    if after > last {
        return last;
    }
    let (left, right) = (positions[after - 1], positions[after]);
    if cell - left > (right - left) / 2 {
        after
    } else {
        after - 1
    }
}

/// 把显示列 `target` 转换为行内列；落在宽字符或制表符中间时取距离更近的一侧。
///
/// # Panics
///
/// 在到达目标列之前遇到制表符且 `tab_width` 为 0 时 panic。
pub fn cell_to_col(line: RopeSlice<'_>, target: usize, tab_width: usize) -> usize {
    let mut cell = 0;

    for (col, c) in line.chars().enumerate() {
        let width = char_cells(c, cell, tab_width);

        if cell + width > target {
            return if target - cell > width / 2 {
                col + 1
            } else {
                col
            };
        }

        cell += width;
    }

    line.len_chars()
}

/// 返回行首空白（空格和制表符）的字符数。
pub fn leading_whitespace(line: RopeSlice<'_>) -> usize {
    line.chars().take_while(|&c| c == ' ' || c == '\t').count()
}

/// 返回行首缩进的显示宽度（显示列数）。
///
/// # Panics
///
/// 行首含制表符且 `tab_width` 为 0 时 panic。
pub fn indent_cells(line: RopeSlice<'_>, tab_width: usize) -> usize {
    col_to_cell(line, leading_whitespace(line), tab_width)
}

/// 是否整行只由空格和制表符组成。
pub fn is_blank(line: RopeSlice<'_>) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

/// 是否 `slice` 以 `prefix` 开头；逐字符比较，不分配内存。
pub fn starts_with(slice: RopeSlice<'_>, prefix: &str) -> bool {
    let mut chars = slice.chars();
    prefix.chars().all(|p| chars.next() == Some(p))
}

#[cfg(test)]
mod tests;
