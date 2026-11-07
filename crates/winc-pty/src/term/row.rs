//! 屏幕上的一行：按格存放的单元格，以及与协议里的 [`Line`] 之间的转换。

use crate::protocol::{Attrs, Color, Line, Marks, Run, Style};
use crate::term::width::cells;

/// 宽字符前半个字的属性位（只在网格里用，不出现在协议的样式里）。
pub(super) const WIDE: u16 = 1 << 8;
/// 宽字符后半个字的占位属性位：占一格，不绘制，文字里没有它。
pub(super) const SPACER: u16 = 1 << 9;
/// 只在网格里用的属性位，转成 [`Line`] 时去掉。
const GRID_ONLY: u16 = WIDE | SPACER;

/// 一格。
///
/// 占 16 字节：字、前景、背景、属性；不常用的部分（组合字符、下划线颜色、链接）放在行的
/// `extras` 里，按编号引用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cell {
    /// 格里的字；宽字符的后半格是空格。
    pub ch: char,
    /// 前景色。
    pub fg: Color,
    /// 背景色。
    pub bg: Color,
    /// 属性位，另含网格用的 [`WIDE`]、[`SPACER`] 位。
    pub attrs: Attrs,
    /// `extras` 里的下标加一，0 表示没有。
    pub extra: u16,
}

impl Cell {
    /// 擦除后留下的空格：保留传入的背景色（xterm 的 bce）。
    pub fn blank(bg: Color) -> Self {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg,
            attrs: Attrs::default(),
            extra: 0,
        }
    }

    /// 是否什么都不含的空格（行尾可以省掉）。
    ///
    /// 换行留下的占位空格也算。
    fn is_empty(&self) -> bool {
        self.ch == ' '
            && self.bg == Color::Default
            && self.extra == 0
            && (self.attrs.0 & !Attrs::FILLER) == 0
    }
}

/// 一格里不常用的部分。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Extra {
    /// 紧跟在这个格的字后面的组合字符。
    pub combining: String,
    /// 下划线颜色（与前景相同时为 `Default`）。
    pub underline_color: Color,
    /// 超链接编号（0 表示没有）。
    pub link: u16,
}

/// 一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    /// 各格的字符和样式。
    pub cells: Vec<Cell>,
    /// 不常用的部分（组合字符、下划线颜色、链接），`Cell::extra` 是这里的下标加一。
    pub extras: Vec<Extra>,
    /// 写满后自动折到了下一行。
    pub wrapped: bool,
    /// shell 集成（OSC 133）记在行上的标记。
    pub marks: Marks,
}

impl Row {
    /// 创建一行 `cols` 格的空白，背景色为 `bg`。
    pub fn new(cols: usize, bg: Color) -> Self {
        Row {
            cells: vec![Cell::blank(bg); cols],
            extras: Vec::new(),
            wrapped: false,
            marks: Marks::default(),
        }
    }

    /// 清空成 `cols` 格空白，保留已经分配的内存（滚动时重复使用滚出屏幕的那一行）。
    pub fn reset(&mut self, cols: usize, bg: Color) {
        self.cells.clear();
        self.cells.resize(cols, Cell::blank(bg));
        self.extras.clear();
        self.wrapped = false;
        self.marks = Marks::default();
    }

    /// 第 `col` 格的样式。
    pub fn style(&self, col: usize) -> Style {
        let cell = &self.cells[col];
        let extra = self.extra(cell);
        Style {
            fg: cell.fg,
            bg: cell.bg,
            underline_color: extra.map_or(Color::Default, |e| e.underline_color),
            attrs: Attrs(cell.attrs.0 & !GRID_ONLY),
            link: extra.map_or(0, |e| e.link),
        }
    }

    fn extra(&self, cell: &Cell) -> Option<&Extra> {
        (cell.extra != 0).then(|| &self.extras[usize::from(cell.extra) - 1])
    }

    /// 在第 `col` 格写一个字，宽字符连同下一格的占位一起写入。
    ///
    /// 调用方保证 `col` 之后的格子放得下这个字。
    pub fn write(&mut self, col: usize, ch: char, pen: &Style, wide: bool) {
        self.cut(col);
        self.cut(col + 1 + usize::from(wide));
        let extra = if pen.underline_color != Color::Default || pen.link != 0 {
            self.add_extra(Extra {
                combining: String::new(),
                underline_color: pen.underline_color,
                link: pen.link,
            })
        } else {
            0
        };
        let mut attrs = pen.attrs;
        if wide {
            attrs.0 |= WIDE;
        }
        self.cells[col] = Cell {
            ch,
            fg: pen.fg,
            bg: pen.bg,
            attrs,
            extra,
        };
        if wide {
            self.cells[col + 1] = Cell {
                ch: ' ',
                attrs: Attrs(pen.attrs.0 | SPACER),
                extra: 0,
                ..self.cells[col]
            };
        }
    }

    /// 把组合字符接到第 `col` 格的字后面。
    ///
    /// 第 `col` 格是宽字符的后半格时，组合字符接到宽字符的前半格上。
    pub fn push_combining(&mut self, col: usize, c: char) {
        let col = if self.cells[col].attrs.has(SPACER) && col > 0 {
            col - 1
        } else {
            col
        };
        let cell = self.cells[col];
        // 单格组合字符的上限（字节）。正常情况下只有几个字符；程序反复往同一格里加时，
        // 超过这个长度就不再累积，避免这一格无限增长。
        const MAX_COMBINING: usize = 32;
        if cell.extra == 0 {
            let id = self.add_extra(Extra {
                combining: c.to_string(),
                ..Extra::default()
            });
            self.cells[col].extra = id;
        } else {
            let extra = &mut self.extras[usize::from(cell.extra) - 1];
            if extra.combining.len() < MAX_COMBINING {
                extra.combining.push(c);
            }
        }
    }

    fn add_extra(&mut self, extra: Extra) -> u16 {
        // 同一行反复重写（进度条）时，没有格再引用的旧条目会越积越多；超过格数的两倍时整理一次。
        if self.extras.len() >= (self.cells.len() * 2).max(16) {
            self.compact_extras();
        }
        if self.extras.len() >= usize::from(u16::MAX) {
            return 0;
        }
        self.extras.push(extra);
        self.extras.len() as u16
    }

    fn compact_extras(&mut self) {
        let mut kept = Vec::new();
        for cell in &mut self.cells {
            if cell.extra != 0 {
                kept.push(std::mem::take(
                    &mut self.extras[usize::from(cell.extra) - 1],
                ));
                cell.extra = kept.len() as u16;
            }
        }
        self.extras = kept;
    }

    /// 在第 `at` 格前面切一刀：切开了一个宽字符时把两半都擦掉，半个宽字符画不出来。
    fn cut(&mut self, at: usize) {
        if at == 0 || at >= self.cells.len() || !self.cells[at].attrs.has(SPACER) {
            return;
        }
        self.cells[at - 1] = Cell::blank(self.cells[at - 1].bg);
        self.cells[at] = Cell::blank(self.cells[at].bg);
    }

    /// 把第 `col` 格的字加宽到两格：后一格变成它的后半格。
    pub fn widen(&mut self, col: usize) {
        self.cut(col + 2);
        let cell = self.cells[col];
        self.cells[col].attrs.0 |= WIDE;
        self.cells[col + 1] = Cell {
            ch: ' ',
            attrs: Attrs(cell.attrs.0 | SPACER),
            extra: 0,
            ..cell
        };
    }

    /// 这一行是否没有任何内容。
    pub fn is_blank(&self) -> bool {
        self.cells.iter().all(Cell::is_empty)
    }

    /// 擦掉 `start..end` 这些格，填成背景色为 `bg` 的空格。
    pub fn erase(&mut self, start: usize, end: usize, bg: Color) {
        let end = end.min(self.cells.len());
        if start >= end {
            return;
        }
        self.cut(start);
        self.cut(end);
        self.cells[start..end].fill(Cell::blank(bg));
    }

    /// 在第 `col` 格插入 `n` 个空格，后面的往右挤，挤出行尾的丢弃（ICH）。
    pub fn insert_blanks(&mut self, col: usize, n: usize, bg: Color) {
        let len = self.cells.len();
        if col >= len {
            return;
        }
        let n = n.min(len - col);
        self.cut(col);
        // 挤出去的部分如果切开了一个宽字符，留下的前半也要擦掉。
        self.cut(len - n);
        self.cells.copy_within(col..len - n, col + n);
        self.cells[col..col + n].fill(Cell::blank(bg));
    }

    /// 删掉从第 `col` 格起的 `n` 格，后面的往左移，行尾补空格（DCH）。
    pub fn delete_chars(&mut self, col: usize, n: usize, bg: Color) {
        let len = self.cells.len();
        if col >= len {
            return;
        }
        let n = n.min(len - col);
        self.cut(col);
        self.cut(col + n);
        self.cells.copy_within(col + n..len, col);
        self.cells[len - n..].fill(Cell::blank(bg));
    }

    /// 改成 `cols` 格，不重排：多出来的截掉，不够的补空格。
    pub fn resize(&mut self, cols: usize) {
        self.cut(cols);
        self.cells.resize(cols, Cell::blank(Color::Default));
    }

    /// 最后一个非空格的下一格列号；整行都空时返回 0。
    pub fn content_len(&self) -> usize {
        self.cells
            .iter()
            .rposition(|c| !c.is_empty())
            .map_or(0, |i| i + 1)
    }

    /// 转成协议里的一行。
    ///
    /// 软换行的行保留全部格子（重排时要逐格拼回去），其余去掉行尾的空格。
    pub fn to_line(&self) -> Line {
        let mut line = Line::default();
        self.fill_line(&mut line);
        line
    }

    /// 同 [`Row::to_line`]，但写进现成的 `line` 并复用它已分配的内存。
    ///
    /// 回滚缓冲满了以后，每滚出一行就复用最旧的那一行，刷屏时不必每行分配、释放一次。
    pub fn fill_line(&self, line: &mut Line) {
        let end = if self.wrapped {
            self.cells.len()
        } else {
            self.content_len()
        };
        let text = &mut line.text;
        let runs = &mut line.runs;
        text.clear();
        runs.clear();

        // 每行进回滚缓冲都要走这里，而相邻的格样式一般相同：先比格子里的字段，变了才拼出
        // 完整的样式。
        text.reserve(end);
        let mut previous: Option<&Cell> = None;
        for (col, cell) in self.cells[..end].iter().enumerate() {
            if cell.attrs.has(SPACER) {
                continue;
            }
            let same = previous.is_some_and(|p| {
                p.fg == cell.fg
                    && p.bg == cell.bg
                    && p.extra == cell.extra
                    && (p.attrs.0 ^ cell.attrs.0) & !GRID_ONLY == 0
            });
            if !same {
                previous = Some(cell);
                let style = self.style(col);
                if runs.last().is_none_or(|run| run.style != style) {
                    runs.push(Run {
                        start: col as u16,
                        style,
                    });
                }
            }
            text.push(cell.ch);
            if let Some(extra) = self.extra(cell) {
                text.push_str(&extra.combining);
            }
        }
        // 只有一段默认样式时省略样式段，空行最常见。
        if runs.len() == 1 && runs[0].style == Style::default() {
            runs.clear();
        }
        // 复用来的内存比这一行大得多时释放多余容量，避免回滚缓冲里每一行都保留最长那一行的容量。
        if text.capacity() > text.len() * 2 + 64 {
            text.shrink_to_fit();
        }
        if runs.capacity() > runs.len() * 2 + 4 {
            runs.shrink_to_fit();
        }
        line.wrapped = self.wrapped;
        line.marks = self.marks;
    }

    /// 从协议里的一行建出 `cols` 格，放不下的部分丢弃。
    pub fn from_line(line: &Line, cols: usize) -> Self {
        let mut row = Row::new(cols, Color::Default);
        row.wrapped = line.wrapped;
        row.marks = line.marks;

        let mut run = 0;
        let mut col = 0;
        for (c, width) in cells(&line.text) {
            if width == 0 {
                if col > 0 {
                    row.push_combining(col - 1, c);
                }
                continue;
            }
            if col + width > cols {
                break;
            }
            while run + 1 < line.runs.len() && usize::from(line.runs[run + 1].start) <= col {
                run += 1;
            }
            let style = line.runs.get(run).map_or_else(Style::default, |r| r.style);
            row.write(col, c, &style, width == 2);
            col += width;
        }
        row
    }
}

/// 一行文字占几格。
pub(super) fn line_cells(line: &Line) -> usize {
    cells(&line.text).map(|(_, width)| width).sum()
}
