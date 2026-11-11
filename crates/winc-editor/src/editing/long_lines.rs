//! 超长行：每行默认只显示开头的 [`MAX_LINE_CHARS`] 个字符，展开后显示整行。
//!
//! 不换行时，展开的行只读取屏幕上可见的列（[`Document::long_line_span`]）。列索引从行首起按需
//! 向后延伸，只扫描到用到的位置：开销取决于横向滚动到了哪里，与行的长度无关。文本修改时索引
//! 随行号移动，只丢弃被修改的行。换行时，展开的行按整行断开（见 `folding::wrapping`）。

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use winc_core::Lines;
use winc_core::text::{MAX_LINE_CHARS, char_cells};

use crate::document::Document;
use crate::navigation::bookmarks::Bookmarks;

/// 列索引的间隔（字符）。
///
/// 取自调参：读取屏幕上的列时最多多读两个间隔的字符；每条记录 8 字节，每 MB 文字约占 8 KB。
const COL_STRIDE: usize = 1024;

/// 列索引一次至少向后延伸的字符数（字符），是 [`COL_STRIDE`] 的整数倍。
///
/// 向右滚动时每隔这么多字符才读取一次，而不是每帧读取。
const EXTEND_MIN: usize = 64 * COL_STRIDE;

/// 列索引一次最多读取的字符数（字符），是 [`COL_STRIDE`] 的整数倍：限制读取时占用的内存。
const EXTEND_MAX: usize = 4096 * COL_STRIDE;

/// 缓存的列索引数和文字段数（条）：多于一屏可显示的行数。
const COLS_CACHE: usize = 128;

/// 显示整行的行，以及这些行的列索引。
#[derive(Debug, Default)]
pub(crate) struct WholeLines {
    /// 是否所有的行都显示整行。
    all: bool,
    /// 逐行展开的行，随文本的修改移动。
    pub lines: Bookmarks,
    /// 超长行的列索引，最近使用的排在最后。
    cols: Mutex<Vec<(ColsKey, LongCols)>>,
    /// 最近读取的文字段（见 [`Document::long_line_span`]），最近使用的排在最后。
    spans: Mutex<Vec<(SpanKey, Arc<LineSpan>)>>,
    /// 列索引每次延伸或丢弃后加一，见 [`Document::long_lines_epoch`]。
    epoch: AtomicU64,
}

impl WholeLines {
    /// 第 `line` 行是否显示整行。
    pub fn has(&self, line: usize) -> bool {
        self.all || self.lines.contains(line)
    }

    /// 是否没有显示整行的行。
    fn is_empty(&self) -> bool {
        !self.all && self.lines.is_empty()
    }

    fn cols_mut(&mut self) -> &mut Vec<(ColsKey, LongCols)> {
        self.cols.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    fn spans_mut(&mut self) -> &mut Vec<(SpanKey, Arc<LineSpan>)> {
        self.spans.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    /// 文本修改时是否需要知道修改所在的行：有逐行展开的行或列索引。
    pub fn tracks(&mut self) -> bool {
        !self.lines.is_empty() || !self.cols_mut().is_empty()
    }

    /// 文本修改后调整行号，参数同 [`Bookmarks::edit`]：被修改的行丢弃列索引，其后的行随之移动。
    pub fn edit(&mut self, start_line: usize, end_line: usize, delta: isize, whole_lines: bool) {
        self.lines.edit(start_line, end_line, delta, whole_lines);
        let keep = |line: &mut usize| {
            if (start_line..=end_line).contains(line) {
                return false;
            }
            if *line > end_line {
                *line = line.saturating_add_signed(delta);
            }
            true
        };
        self.cols_mut().retain_mut(|(key, _)| keep(&mut key.0));
        self.spans_mut().retain_mut(|(key, _)| keep(&mut key.0.0));
        *self.epoch.get_mut() += 1;
    }

    /// 文本被整体替换后丢弃全部列索引。
    pub fn forget(&mut self) {
        self.cols_mut().clear();
        self.spans_mut().clear();
        *self.epoch.get_mut() += 1;
    }
}

/// 断行时各行参与断行的字符数上限。
#[derive(Clone, Copy)]
pub(crate) struct Limits<'a> {
    /// 一般的行的上限（字符），见 [`Document::wrap_limit`]。
    pub base: usize,
    /// 显示整行的行，不设上限；`None` 表示没有。
    pub whole: Option<&'a WholeLines>,
}

impl Limits<'_> {
    /// 第 `line` 行的上限（字符）。
    pub fn of(&self, line: usize) -> usize {
        match self.whole {
            Some(whole) if whole.has(line) => usize::MAX,
            _ => self.base,
        }
    }

    /// 逐行读取时每行的字节上限：有显示整行的行时读取整行。
    pub fn cap(&self) -> usize {
        match self.whole {
            Some(whole) if !whole.is_empty() => usize::MAX,
            _ => self.base.saturating_mul(4),
        }
    }
}

/// 超长行的列索引：从行首起已扫描的部分中，每隔 [`COL_STRIDE`] 个字符记录一次显示列。
#[derive(Debug)]
struct LongCols {
    /// 该行的字符数。
    len: usize,
    /// 已扫描的字符数：[`COL_STRIDE`] 的整数倍，或等于 `len`。
    done: usize,
    /// 已扫描的部分占用的显示列数。
    cells: usize,
    /// 第 `k` 项是前 `(k + 1) * COL_STRIDE` 个字符占用的显示列数。
    stops: Vec<usize>,
}

impl LongCols {
    /// 第 `k` 处记录之前的（字符数，显示列数）；`k` 为 0 时是行首。
    fn stop(&self, k: usize) -> (usize, usize) {
        match k.checked_sub(1) {
            Some(i) => ((i + 1) * COL_STRIDE, self.stops[i]),
            None => (0, 0),
        }
    }

    /// 不晚于第 `col` 个字符的最近一处记录。
    fn before_col(&self, col: usize) -> (usize, usize) {
        self.stop((col / COL_STRIDE).min(self.stops.len()))
    }

    /// 不晚于显示列 `cell` 的最近一处记录。
    fn before_cell(&self, cell: usize) -> (usize, usize) {
        self.stop(self.stops.partition_point(|&c| c <= cell))
    }

    /// 该行占用的显示列数，以及是否准确：未扫描的部分按每个字符一列估计。
    fn width(&self) -> (usize, bool) {
        (self.cells + (self.len - self.done), self.done == self.len)
    }

    /// 扫描紧接已扫描部分的一段文字 `text`。
    fn scan(&mut self, text: &str, tab_width: usize) {
        let mut rest = text;
        while !rest.is_empty() {
            // 到下一处记录之前的一串可见 ASCII 字符各占一列，整串计入，不逐个查表。
            let room = (COL_STRIDE - self.done % COL_STRIDE).min(rest.len());
            let plain = (rest.as_bytes()[..room].iter())
                .position(|b| !(b' '..=b'~').contains(b))
                .unwrap_or(room);
            let step = match (plain, rest.chars().next()) {
                (0, Some(c)) => {
                    self.cells += char_cells(c, self.cells, tab_width);
                    self.done += 1;
                    c.len_utf8()
                }
                _ => {
                    self.cells += plain;
                    self.done += plain;
                    plain
                }
            };
            rest = &rest[step..];
            if self.done.is_multiple_of(COL_STRIDE) {
                self.stops.push(self.cells);
            }
        }
    }
}

/// 列索引要延伸到的位置。
#[derive(Clone, Copy)]
enum Until {
    /// 第几个字符。
    Col(usize),
    /// 超过这个显示列。
    Cell(usize),
}

/// 列索引缓存的键：（行号，Tab 宽度）。
type ColsKey = (usize, usize);

/// 文字段缓存的键：列索引的键，以及这一段起止处的字符序号。
type SpanKey = (ColsKey, usize, usize);

/// 超长行中相邻的一段文字，见 [`Document::long_line_span`]。
#[derive(Debug)]
pub struct LineSpan {
    /// 第一个字符在该行中的字符序号（从 0 起）。
    pub col: usize,
    /// 第一个字符之前的文字占用的显示列数。
    pub cell: usize,
    /// 这段文字。
    pub text: String,
    /// 这段文字的字符，与 `text` 相同。
    pub chars: Vec<char>,
    /// 是否延伸到行尾。
    pub ends: bool,
}

impl Document {
    /// 第 `line` 行是否显示整行，而不是只显示开头的 [`MAX_LINE_CHARS`] 个字符。
    pub fn shows_whole_line(&self, line: usize) -> bool {
        self.whole.has(line)
    }

    /// 不换行时第 `line` 行显示的字符数上限：显示整行的行不设上限，其余为 [`MAX_LINE_CHARS`]。
    pub fn line_limit(&self, line: usize) -> usize {
        if self.whole.has(line) {
            usize::MAX
        } else {
            MAX_LINE_CHARS
        }
    }

    /// 展开第 `line` 行：此后显示整行。换行时重新断开该行。
    pub fn show_whole_line(&mut self, line: usize) {
        if self.whole.has(line) {
            return;
        }
        self.whole.lines.toggle(line);
        self.folds.rewrap(&self.text, line, &self.whole);
    }

    /// 设置是否所有的行都显示整行，返回是否有变化。换行时整体重新断行。
    pub fn show_whole_lines(&mut self, all: bool) -> bool {
        if self.whole.all == all {
            return false;
        }
        self.whole.all = all;
        // 大文件换行时本来就按整行断开。
        if let (Some(wrap), false) = (self.wrap_width(), self.is_large()) {
            let limits = Limits {
                base: self.wrap_limit(),
                whole: Some(&self.whole),
            };
            self.folds
                .set_wrap(&self.text, Some(wrap), limits, self.revision);
        }
        true
    }

    /// 列索引每次延伸或丢弃后变化的计数。
    ///
    /// 超长行的宽度在扫描到行尾之前是估计值（见 [`Document::whole_line_cells`]），依赖宽度的
    /// 缓存以该计数为键。
    pub fn long_lines_epoch(&self) -> u64 {
        self.whole.epoch.load(Ordering::Relaxed)
    }

    /// 把显示整行的超长行 `line` 的列索引延伸到 `until`，再对它调用 `f`（第二个参数是行首的
    /// 字符偏移）；该行不显示整行，或不超过 [`MAX_LINE_CHARS`] 个字符时返回 `None`。
    ///
    /// 只扫描尚未扫描的部分：开销与新滚动到的范围成正比，位置不变时不读取文字。
    fn with_cols<R>(
        &self,
        key: ColsKey,
        until: Until,
        f: impl FnOnce(&LongCols, usize) -> R,
    ) -> Option<R> {
        let (line, tab_width) = key;
        if !self.whole.has(line) {
            return None;
        }
        let text = &self.text;
        let start = text.line_start(line);
        let mut cache = (self.whole.cols.lock()).unwrap_or_else(PoisonError::into_inner);
        match cache.iter().position(|(k, _)| *k == key) {
            // 最近使用的排在最后。
            Some(i) => {
                let entry = cache.remove(i);
                cache.push(entry);
            }
            None => {
                // 行的字符数由相邻两行的行首得出，不读取整行；行尾的换行符占一到两个字符。
                let last = line + 1 >= text.line_count();
                let next = match last {
                    true => text.end_pos(),
                    false => text.line_start(line + 1),
                };
                if next - start <= MAX_LINE_CHARS {
                    return None;
                }
                let ending = match last {
                    true => 0,
                    false if text.substring(next - 2, next) == "\r\n" => 2,
                    false => 1,
                };
                let len = next - start - ending;
                if len <= MAX_LINE_CHARS {
                    return None;
                }
                if cache.len() >= COLS_CACHE {
                    cache.remove(0);
                }
                let (done, cells, stops) = (0, 0, Vec::new());
                let cols = LongCols {
                    len,
                    done,
                    cells,
                    stops,
                };
                cache.push((key, cols));
            }
        }
        let cols = &mut cache.last_mut()?.1;
        let before = cols.done;
        loop {
            // 还要扫描的字符数：按显示列延伸时每个字符至少占一列，按还差的列数读取。
            let want = match until {
                Until::Col(col) => col.min(cols.len).saturating_sub(cols.done),
                Until::Cell(cell) if cols.cells <= cell => cell - cols.cells + 1,
                Until::Cell(_) => 0,
            };
            if want == 0 || cols.done == cols.len {
                break;
            }
            let more = want.clamp(EXTEND_MIN, EXTEND_MAX);
            let to = (cols.done + more.next_multiple_of(COL_STRIDE)).min(cols.len);
            cols.scan(&text.substring(start + cols.done, start + to), tab_width);
            // 读到的文字比记录的短（不应出现）时以读到的为准，避免反复读取。
            if cols.done < to {
                cols.len = cols.done;
            }
        }
        if cols.done != before {
            self.whole.epoch.fetch_add(1, Ordering::Relaxed);
        }
        Some(f(cols, start))
    }

    /// 返回显示整行的超长行 `line` 中覆盖显示列 `cells` 的一段；其他行返回 `None`。
    ///
    /// 只读取这一段所在的若干个索引间隔；最近读取的若干段有缓存，画面不动或只滚动了少许时
    /// 不重复读取。
    pub fn long_line_span(
        &self,
        line: usize,
        cells: Range<usize>,
        tab_width: usize,
    ) -> Option<Arc<LineSpan>> {
        let key = (line, tab_width);
        let until = Until::Cell(cells.end);
        let (from, to, ends, start) = self.with_cols(key, until, |cols, start| {
            let from = cols.before_cell(cells.start);
            let last = cols.stops.partition_point(|&c| c < cells.end);
            let to = ((last + 1) * COL_STRIDE).min(cols.done);
            (from, to.max(from.0), to == cols.len, start)
        })?;
        let (col, cell) = from;
        let key = (key, col, to);
        let mut cache = (self.whole.spans.lock()).unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(i);
            let span = entry.1.clone();
            cache.push(entry);
            return Some(span);
        }
        let text = self.text.substring(start + col, start + to);
        let span = Arc::new(LineSpan {
            col,
            cell,
            chars: text.chars().collect(),
            text,
            ends,
        });
        if cache.len() >= COLS_CACHE {
            cache.remove(0);
        }
        cache.push((key, span.clone()));
        Some(span)
    }

    /// 返回显示整行的超长行 `line` 占用的显示列数，以及是否准确；其他行返回 `None`。
    ///
    /// 不读取文字：列索引尚未延伸到行尾时，其后的部分按每个字符一列估计，随索引的延伸
    /// （[`Document::long_lines_epoch`] 变化）趋于准确。
    pub fn whole_line_cells(&self, line: usize, tab_width: usize) -> Option<(usize, bool)> {
        self.with_cols((line, tab_width), Until::Col(0), |cols, _| cols.width())
    }

    /// 同 [`Document::col_to_cell`]，用于显示整行的超长行；其他行返回 `None`。
    pub(crate) fn long_col_to_cell(
        &self,
        line: usize,
        col: usize,
        tab_width: usize,
    ) -> Option<usize> {
        let key = (line, tab_width);
        let (from, to, start) = self.with_cols(key, Until::Col(col), |cols, start| {
            (cols.before_col(col), col.min(cols.len), start)
        })?;
        let (from, mut cells) = from;
        for c in self.text.substring(start + from, start + to).chars() {
            cells += char_cells(c, cells, tab_width);
        }
        Some(cells)
    }

    /// 同 [`Document::cell_to_col`]，用于显示整行的超长行；其他行返回 `None`。
    pub(crate) fn long_cell_to_col(
        &self,
        line: usize,
        cell: usize,
        tab_width: usize,
    ) -> Option<usize> {
        let key = (line, tab_width);
        let (from, to, start) = self.with_cols(key, Until::Cell(cell), |cols, start| {
            let from = cols.before_cell(cell);
            (from, (from.0 + COL_STRIDE).min(cols.len), start)
        })?;
        let (from, mut at) = from;
        let text = self.text.substring(start + from, start + to);
        for (i, c) in text.chars().enumerate() {
            let width = char_cells(c, at, tab_width);
            if at + width > cell {
                return Some(from + i + usize::from(cell - at > width / 2));
            }
            at += width;
        }
        Some(to)
    }
}

#[cfg(test)]
mod tests;
