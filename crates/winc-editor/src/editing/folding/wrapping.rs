//! 自动换行：各行的断行位置、显示行与文本位置的换算、按显示行上下移动，以及行首、行尾命令。

use super::*;

use std::sync::Arc;

use crate::LargeText;
use crate::document::content::Content;

/// 返回第 `line` 行按 `width` 个显示列断开的结果。
///
/// 只处理开头 `limit` 个字符（见 [`Document::wrap_limit`]）。
pub(crate) fn line_breaks(
    text: &impl Lines,
    line: usize,
    width: usize,
    tab_width: usize,
    limit: usize,
) -> Breaks {
    let content = text.line_content(line);
    let slice = content.slice();
    breaks_of(slice.len_chars(), || slice.chars(), width, tab_width, limit)
}

/// 返回一行文字按 `width` 个显示列断开的结果：`chars` 每次调用都从行首起依次给出该行的字符，
/// `len` 是字符数的上界。
///
/// 只处理开头 `limit` 个字符。
pub(super) fn breaks_of<I: Iterator<Item = char>>(
    len: usize,
    chars: impl Fn() -> I,
    width: usize,
    tab_width: usize,
    limit: usize,
) -> Breaks {
    // 先做粗略估计：每个字符最宽 2 个显示列（Tab 按 Tab 宽度计），按该上界估算仍可容纳的行
    // 无需逐字符测量。
    if len.saturating_mul(tab_width.max(2)) <= width {
        return Breaks::default();
    }
    // 该上界偏保守（Tab 宽度越大，越容易判为不可容纳），而多数行实际可以容纳：因此遍历时累计
    // 宽度，确认不可容纳时才取出字符精确计算，避免为同一行建立两次字符表。
    let mut cells = 0;
    let fits = chars().take(limit).all(|c| {
        cells += winc_core::text::char_cells(c, cells, tab_width);
        cells <= width.max(1)
    });
    if fits {
        return Breaks::default();
    }
    let chars: Vec<char> = chars().take(limit).collect();
    wrap::breaks(&chars, width, tab_width)
}

/// 返回第 `line` 行按 `width` 个显示列断开的结果，以及参与断行的字符数（字符）。
pub(super) fn measure_line(
    text: &impl Lines,
    line: usize,
    width: usize,
    tab_width: usize,
    limit: usize,
) -> (Breaks, usize) {
    let content = text.line_content(line);
    let slice = content.slice();
    let len = slice.len_chars();
    let breaks = breaks_of(len, || slice.chars(), width, tab_width, limit);
    (breaks, len.min(limit))
}

/// 返回 `lines` 范围内（超出末尾的部分忽略）各行的断行结果：被断成多段的行各自额外占用的显示行数，
/// 以及长行的行内断点索引。
///
/// 按顺序遍历各行：逐行按行号定位的开销与行数成正比，10 万行约 20 ms。
/// 每行最多取 `limits` 给出的字符数，与 [`line_breaks`] 处理的字符相同；不设上限
/// （`usize::MAX`，整行断开）的行为长行建立行内索引。
pub(crate) fn wrapped_lines(
    text: &impl Lines,
    lines: std::ops::Range<usize>,
    width: usize,
    tab_width: usize,
    limits: Limits<'_>,
) -> WrapRows {
    let mut rows = WrapRows::default();
    if lines.start >= text.line_count() {
        return rows;
    }
    text.visit_lines(lines.start, limits.cap(), |line, _, content| {
        if line >= lines.end {
            return ControlFlow::Break(());
        }
        let limit = limits.of(line);
        // 每个字符占的显示列数不超过它的 UTF-8 字节数（Tab 除外），
        // 按字节数可容纳的行无需逐字符测量。
        let tabs = content.bytes().filter(|&b| b == b'\t').count();
        let cells = content.len() + tabs * tab_width.saturating_sub(1);
        if cells <= width {
            return ControlFlow::Continue(());
        }
        // 只统计段数并记录行内索引需要的断点，不为整行建立字符表和全部断点。
        let whole = limit == usize::MAX;
        let text = match whole {
            true => content,
            false => {
                (content.char_indices().nth(limit)).map_or(content, |(end, _)| &content[..end])
            }
        };
        let (mut count, mut stops) = (1, Vec::new());
        let indent = wrap::each_break(text, width, tab_width, |start| {
            if whole && count % ROW_STRIDE == 0 {
                stops.push(start);
            }
            count += 1;
        });
        if count > 1 {
            rows.extra.push((line, count - 1));
        }
        if whole && count > ROW_STRIDE {
            rows.long.push(LongRows {
                line,
                len: text.chars().count(),
                indent,
                stops,
            });
        }
        ControlFlow::Continue(())
    });
    rows
}

/// 大文件后台断行的设置：结果只在文档仍按同一设置换行、版本也未变时采用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrapKey {
    /// 断行宽度（显示列）。
    width: usize,
    /// 一个 Tab 的显示宽度（显示列）。
    tab_width: usize,
    /// 开始计算时的文档版本号。
    revision: u64,
}

/// 大文件的断行任务：在后台把全文逐行断开（[`WrapJob::run`]），完成后交给
/// [`Document::finish_wrap`]。
pub struct WrapJob {
    /// 文本的副本（[`LargeText::try_clone`]），与文档共用文件句柄；后台读取不影响界面上的编辑。
    text: LargeText,
    key: WrapKey,
}

/// [`WrapJob`] 的结果。
pub struct WrapResult {
    key: WrapKey,
    rows: WrapRows,
}

impl WrapJob {
    /// 把全文分成若干份并行断行，返回各行的断行结果。
    ///
    /// 各份共用同一份文本，按位置读取，互不影响。单线程实测每 MB 文本约需 4 ms（含解码）。
    pub fn run(self) -> WrapResult {
        let WrapKey {
            width, tab_width, ..
        } = self.key;
        let lines = self.text.line_count();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        // 份数多于线程数，线程完成一份后再取下一份：性能核与能效核速度不同，
        // 每个线程只分配一份时总耗时取决于最慢的核心。
        let parts = (threads * WRAP_JOB_SHARES).min(lines.div_ceil(WRAP_JOB_LINES).max(1));
        // 按字符位置等分，使各段的文字量相近：超长行可达数 MB，按行数等分会使各段相差悬殊。
        let total = self.text.end_pos();
        let mut bounds: Vec<usize> = (0..parts)
            .map(|k| self.text.line_of(total / parts * k))
            .collect();
        bounds[0] = 0;
        bounds.push(lines);
        bounds.dedup();
        let count = bounds.len() - 1;
        let text = std::sync::Arc::new(self.text);
        let pieces = crate::background::split(count, threads, move |i| {
            let range = bounds[i]..bounds[i + 1];
            wrapped_lines(&*text, range, width, tab_width, WHOLE)
        });
        let mut rows = WrapRows::default();
        for piece in pieces {
            rows.extra.extend(piece.extra);
            rows.long.extend(piece.long);
        }
        WrapResult {
            key: self.key,
            rows,
        }
    }
}

/// 大文件换行时各行都按整行断开。
const WHOLE: Limits<'static> = Limits {
    base: usize::MAX,
    whole: None,
};

/// 后台断行时每个线程平均分到的份数（份）。
///
/// 份越小负载越均衡，但每份都要定位一次段首；取 8 时收尾阶段的不均衡不超过总量的
/// 1/(8 × 线程数)。
const WRAP_JOB_SHARES: usize = 8;

/// 后台断行时每段至少包含的行数（行）：行数很少时只用一个线程。
///
/// 每段都要按行号定位一次段首；该值只用于避免把很少的行分给多个线程，
/// 分段的耗时与断行本身相比可以忽略。
const WRAP_JOB_LINES: usize = 64;

/// 后台断行完成之前预先断开的行数上限（行），见 [`Document::preview_wrap`]。
///
/// 预先断开在界面线程上进行，每 MB 文字约需 4 ms。短行时该行数足以覆盖两屏；
/// 长行时通常一至两行即可填满所需的显示行。
const PREVIEW_LINES: usize = 256;

/// 缓存的行内窗口数（个）。
///
/// 一帧内编辑区、选区、光标和缩略图各读取屏幕上长行的一至两个窗口；缓存最近使用的窗口，
/// 使同一帧内和相邻两帧之间不重复读取。
const WINDOW_CACHE: usize = 8;

/// 一行中相邻若干段的文字与断点，见 [`Document::line_window`]。
#[derive(Debug)]
pub struct LineWindow {
    /// 窗口的第一段在该行中的段号（从 0 起）。
    pub part: usize,
    /// 窗口的第一个字符在该行中的字符序号（从 0 起）。
    pub col: usize,
    /// 窗口内的文字。
    pub text: String,
    /// 窗口内的字符，与 `text` 相同。
    pub chars: Vec<char>,
    /// 窗口内各段的断点，位置相对窗口的开头。
    pub breaks: Breaks,
    /// 窗口是否延伸到该行参与断行的最后一个字符。
    pub ends: bool,
}

impl LineWindow {
    /// 返回该行第 `part` 段在窗口中的字符范围（相对窗口的开头）和缩进宽度（显示列）；
    /// `part` 不在窗口内时取窗口中最近的一段。
    pub fn row(&self, part: usize) -> (std::ops::Range<usize>, usize) {
        let local = part.saturating_sub(self.part).min(self.breaks.rows() - 1);
        let range = self.breaks.range(local, self.chars.len());
        (range, self.breaks.indent_of(local))
    }

    /// 是否第 `part` 段为该行的最后一段。
    pub fn is_last(&self, part: usize) -> bool {
        self.ends && part + 1 >= self.part + self.breaks.rows()
    }
}

/// 行内窗口缓存的键：（行号，版本号，断行宽度，Tab 宽度，第一块，最后一块）。
pub(crate) type WindowKey = (usize, u64, usize, usize, usize, usize);

impl Document {
    /// 设置自动换行并在需要时重新计算，返回设置后是否换行。
    ///
    /// `width` 是一行可容纳的显示列数，`None` 表示不换行。宽度、Tab 宽度变化，或重新读取文件、
    /// 改名后整体重新计算。按列对齐的 CSV 不换行。
    ///
    /// 大文件的全文断行在后台计算（[`Document::take_wrap_job`]）：计算完成之前沿用原有的断行结果，
    /// 原先未换行时暂不换行，可由 [`Document::preview_wrap`] 预先断开屏幕上的行。
    pub fn ensure_wrap(&mut self, width: Option<usize>, tab_width: usize) -> bool {
        let width = width.filter(|_| self.shows_text() && self.columns(tab_width).is_none());
        let fresh = match (self.folds.wrap(), width) {
            (None, None) => true,
            (Some(w), Some(width)) => {
                let same = w.width == width && w.tab_width == tab_width;
                !w.partial && same && w.revision == self.revision
            }
            _ => false,
        };
        if fresh {
            self.wrap_wanted = None;
        } else if let (Some(width), true) = (width, self.is_large()) {
            self.wrap_wanted = Some(WrapKey {
                width,
                tab_width,
                revision: self.revision,
            });
        } else {
            self.wrap_wanted = None;
            let revision = self.revision;
            let limits = Limits {
                base: self.wrap_limit(),
                whole: Some(&self.whole),
            };
            self.folds
                .set_wrap(&self.text, width.map(|w| (w, tab_width)), limits, revision);
        }
        self.is_wrapping()
    }

    /// 返回需要在后台运行的断行任务；没有需要计算的断行，或已有任务尚未完成时返回 `None`。
    ///
    /// 同一文档同一时间只运行一个任务：全文断行需要数秒，编辑期间每次按键都会改变版本号，
    /// 为每个版本各启动一个任务会使多个任务同时读取整个文件。上一个任务完成
    /// （[`Document::finish_wrap`]）后，下一次调用按最新的设置和版本返回新的任务。
    pub fn take_wrap_job(&mut self) -> Option<WrapJob> {
        if self.wrap_pending.is_some() {
            return None;
        }
        let key = self.wrap_wanted?;
        let Content::Large(large) = &self.text else {
            return None;
        };
        let text = large.try_clone().ok()?;
        self.wrap_pending = Some(key);
        Some(WrapJob { text, key })
    }

    /// 采用后台计算完成的断行结果，返回是否采用。
    ///
    /// 结果对应的设置或文档版本已经过时（期间宽度或内容已修改）时不采用；
    /// 需要时由下一次 [`Document::ensure_wrap`] 重新安排任务。
    pub fn finish_wrap(&mut self, result: WrapResult) -> bool {
        self.wrap_pending = None;
        if self.wrap_wanted != Some(result.key) || result.key.revision != self.revision {
            return false;
        }
        self.wrap_wanted = None;
        self.folds.install_wrap(Wrap {
            width: result.key.width,
            tab_width: result.key.tab_width,
            limit: self.wrap_limit(),
            extra: result.rows.extra,
            long: result.rows.long,
            partial: false,
            revision: result.key.revision,
        });
        true
    }

    /// 在后台断行完成前预先断开第 `first` 行起的若干行，使屏幕上的文字立即换行；
    /// 返回是否替换了断行结果。
    ///
    /// 只断开从 `first` 起合计至少 `rows` 个显示行的行，最多 `PREVIEW_LINES` 行；其余行暂按
    /// 一段显示，后台任务完成后替换为全文结果（[`Document::finish_wrap`]）。已有其他宽度的全文
    /// 结果时不替换：各行段数相近，避免滚动范围先缩小再恢复。
    pub fn preview_wrap(&mut self, first: usize, rows: usize) -> bool {
        let Some(key) = self.wrap_wanted else {
            return false;
        };
        let same = |w: &Wrap| {
            (w.width, w.tab_width, w.revision) == (key.width, key.tab_width, key.revision)
        };
        if self.folds.wrap().is_some_and(|w| !w.partial || same(w)) {
            return false;
        }
        let end = (first + PREVIEW_LINES).min(self.text.line_count());
        let mut preview = WrapRows::default();
        let mut shown = 0;
        for line in first..end {
            if shown >= rows {
                break;
            }
            let part = wrapped_lines(&self.text, line..line + 1, key.width, key.tab_width, WHOLE);
            shown += 1 + part.extra.first().map_or(0, |&(_, extra)| extra);
            preview.extra.extend(part.extra);
            preview.long.extend(part.long);
        }
        self.folds.install_wrap(Wrap {
            width: key.width,
            tab_width: key.tab_width,
            limit: self.wrap_limit(),
            extra: preview.extra,
            long: preview.long,
            partial: true,
            revision: key.revision,
        });
        true
    }

    /// 是否正在等待后台断行的结果。
    pub fn wrap_pending(&self) -> bool {
        self.wrap_wanted.is_some()
    }

    /// 返回按整行断开的第 `line` 行中覆盖第 `parts` 段的窗口。
    ///
    /// 只用于按整行断开（[`Document::wrap_limit`] 不设上限）的文档中被断开的行；其他情况，以及
    /// 断行结果尚未与当前文字同步时返回 `None`，调用方按行首的 [`MAX_LINE_CHARS`] 个字符和
    /// [`Document::line_breaks`] 处理。
    ///
    /// 有行内索引的长行，窗口从 `parts` 之前最近的索引记录处开始，到 `parts` 之后最近的记录处
    /// 或行尾结束，只读取并断开该部分文字：开销取决于 `parts` 的段数，与行的长度无关。
    /// 没有行内索引的行段数不多，窗口为整行。
    pub fn line_window(
        &self,
        line: usize,
        parts: std::ops::Range<usize>,
    ) -> Option<Arc<LineWindow>> {
        let wrap = self.folds.wrap().filter(|w| w.revision == self.revision)?;
        if self.wrap_limit_of(line) != usize::MAX || !wrap.breaks_line(line) {
            return None;
        }
        let long = wrap.long_rows(line);
        let stops = long.map_or(&[][..], |l| &l.stops[..]);
        let blocks = stops.len();
        let last = (parts.end.max(parts.start + 1) - 1) / ROW_STRIDE;
        let (first, last) = ((parts.start / ROW_STRIDE).min(blocks), last.min(blocks));
        let key = (line, self.revision, wrap.width, wrap.tab_width, first, last);
        let mut cache = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(i);
            let window = entry.1.clone();
            cache.push(entry);
            return Some(window);
        }
        let from = first.checked_sub(1).map_or(0, |i| stops[i]);
        let text = match long {
            Some(long) => {
                let to = stops.get(last).copied().unwrap_or(long.len);
                let start = self.text.line_start(line);
                self.text.substring(start + from, start + to)
            }
            None => self.text.line_content(line).slice().to_string(),
        };
        let chars: Vec<char> = text.chars().collect();
        let breaks = match long {
            Some(long) if first > 0 => {
                wrap::breaks_from(&chars, wrap.width, wrap.tab_width, long.indent)
            }
            _ => wrap::breaks(&chars, wrap.width, wrap.tab_width),
        };
        let window = Arc::new(LineWindow {
            part: first * ROW_STRIDE,
            col: from,
            text,
            chars,
            breaks,
            ends: last == blocks,
        });
        if cache.len() >= WINDOW_CACHE {
            cache.remove(0);
        }
        cache.push((key, window.clone()));
        Some(window)
    }

    /// 返回第 `line` 行中含第 `part` 段的窗口：长行只含相邻的若干段，其他行为整行（最多
    /// [`Document::wrap_limit`] 个字符）。
    fn window_at(&self, line: usize, part: usize) -> Arc<LineWindow> {
        if let Some(window) = self.line_window(line, part..part + 1) {
            return window;
        }
        let content = self.text.line_content(line);
        let limit = self.wrap_limit_of(line);
        let text: String = content.slice().chars().take(limit).collect();
        Arc::new(LineWindow {
            part: 0,
            col: 0,
            chars: text.chars().collect(),
            text,
            breaks: self.line_breaks(line),
            ends: true,
        })
    }

    /// 返回第 `line` 行的第 `col` 个字符所在的段号（从 0 起）。
    pub fn part_of(&self, line: usize, col: usize) -> usize {
        let long = (self.folds.wrap())
            .filter(|w| w.revision == self.revision)
            .and_then(|w| w.long_rows(line));
        if let Some(long) = long {
            let part = long.stops.partition_point(|&s| s <= col) * ROW_STRIDE;
            if let Some(window) = self.line_window(line, part..part + 1) {
                let local = col.saturating_sub(window.col).min(window.chars.len());
                return window.part + window.breaks.row_of(local);
            }
        }
        self.line_breaks(line).row_of(col)
    }

    /// 每行参与断行和绘制的字符数上限（字符）。
    ///
    /// 普通文档只处理每行开头 [`MAX_LINE_CHARS`] 个字符，限制压缩过的代码等超长行的开销。
    /// 大文件中的超长行（如逐条拼接的 `INSERT`）是需要阅读的内容，按整行断开；
    /// 绘制与定位只读取屏幕上的若干段（[`Document::line_window`]）。
    pub fn wrap_limit(&self) -> usize {
        if self.is_large() {
            usize::MAX
        } else {
            MAX_LINE_CHARS
        }
    }

    /// 第 `line` 行参与断行和绘制的字符数上限：显示整行的行不设上限，其余同
    /// [`Document::wrap_limit`]。
    pub fn wrap_limit_of(&self, line: usize) -> usize {
        if self.shows_whole_line(line) {
            usize::MAX
        } else {
            self.wrap_limit()
        }
    }

    /// 是否已开启自动换行。
    pub fn is_wrapping(&self) -> bool {
        self.folds.is_wrapping()
    }

    /// 返回自动换行的（断行宽度，Tab 宽度），单位为显示列；不换行时返回 `None`。
    pub fn wrap_width(&self) -> Option<(usize, usize)> {
        self.folds.wrap().map(|w| (w.width, w.tab_width))
    }

    /// 返回第 `line` 行的断行结果；不换行或可以容纳时只有一段。
    ///
    /// 绘制时每个可见行会调用多次：记录中该行未被断开时直接返回，不再逐字符测量。记录与文字
    /// 不同步（版本不一致，等待 `ensure_wrap` 重算）时仍按当前文字计算。
    ///
    /// 结果含整行的全部断点，开销与行的长度成正比；按整行断开的长行应改用
    /// [`Document::line_window`]、[`Document::part_of`]。记录与文字不同步时只处理行首的
    /// [`MAX_LINE_CHARS`] 个字符：此时没有可用的行内索引，不为屏幕上的每条长行计算全部断点。
    pub fn line_breaks(&self, line: usize) -> Breaks {
        let (text, count) = (&self.text, self.text.line_count());
        match self.folds.wrap() {
            Some(w) if w.revision == self.revision && !w.breaks_line(line) => Breaks::default(),
            Some(w) if w.revision == self.revision && line < count => {
                line_breaks(text, line, w.width, w.tab_width, self.wrap_limit_of(line))
            }
            Some(w) if line < count => line_breaks(
                text,
                line,
                w.width,
                w.tab_width,
                w.limit.min(MAX_LINE_CHARS),
            ),
            _ => Breaks::default(),
        }
    }

    /// 第 `line` 行是否为有行内索引的长行（见 [`Document::line_window`]）。
    pub fn is_long_line(&self, line: usize) -> bool {
        (self.folds.wrap())
            .filter(|w| w.revision == self.revision)
            .is_some_and(|w| w.long_rows(line).is_some())
    }

    /// 返回第 `line` 行占用的显示行数。
    pub fn rows_of(&self, line: usize) -> usize {
        self.folds.rows_of(line)
    }

    /// 返回第 `row` 个显示行对应的（行号，段号），均从 0 起；超出时返回最后一行的最后一段。
    pub fn line_and_part(&self, row: usize) -> (usize, usize) {
        let last = self.text.line_count().saturating_sub(1);
        match self.folds.line_and_part(row) {
            (line, part) if line <= last => (line, part),
            _ => (last, self.folds.rows_of(last).saturating_sub(1)),
        }
    }

    /// 返回第 `row` 个显示行的位置范围：第一个字符的位置，以及最后一个字符之后的位置。
    pub(crate) fn row_bounds(&self, row: usize) -> (usize, usize) {
        let (line, part) = self.line_and_part(row);
        let window = self.window_at(line, part);
        let (range, _) = window.row(part);
        let start = self.text.line_start(line) + window.col;
        (start + range.start, start + range.end)
    }

    /// 返回从位置 `pos` 上下移动 `delta` 个显示行之后的位置，保持在显示行内的同一显示列；
    /// 越过首尾时停在第一个或最后一个显示行。不换行时返回 `None`。
    pub fn display_vertical(&self, pos: usize, delta: isize) -> Option<usize> {
        let tab_width = self.folds.wrap()?.tab_width;
        let (line, col) = self.text.pos_to_line_col(pos);
        let part = self.part_of(line, col);
        let window = self.window_at(line, part);
        let (range, indent) = window.row(part);
        let end = col.saturating_sub(window.col).clamp(range.start, range.end);
        let cell = winc_core::text::slice_cells(&window.chars[range.start..end], indent, tab_width);
        let last = self.row_count().saturating_sub(1);
        let row = (self.folds.row_of(line) + part)
            .saturating_add_signed(delta)
            .min(last);
        Some(self.pos_in_row(row, cell, tab_width))
    }

    /// 返回位置 `pos`（字符偏移）所在的显示行号（从 0 起）。
    pub fn row_at(&self, pos: usize) -> usize {
        let (line, col) = self.text.pos_to_line_col(pos);
        self.folds.row_of(line) + self.part_of(line, col)
    }

    /// 返回第 `row` 个显示行中距行首 `cell` 个显示列处的位置（字符偏移）；
    /// 落在一个字符中间时取较近的一侧。
    ///
    /// 不是该行最后一段时，结果最多取到该段最后一个字符之前，否则光标会绘制在下一段的开头。
    pub fn pos_in_row(&self, row: usize, cell: usize, tab_width: usize) -> usize {
        let (line, part) = self.line_and_part(row);
        let window = self.window_at(line, part);
        let (range, indent) = window.row(part);
        let chars = &window.chars;
        let last = window.is_last(part);
        let mut at = indent;
        let mut col = range.start;
        while col < range.end {
            let w = winc_core::text::char_cells(chars[col], at, tab_width);
            if at + w > cell {
                if cell - at.min(cell) > (at + w - cell) && (last || col + 1 < range.end) {
                    col += 1;
                }
                break;
            }
            at += w;
            col += 1;
        }
        if !last {
            col = col.min(range.end.saturating_sub(1)).max(range.start);
        }
        let pos = self.text.line_col_to_pos(line, window.col + col);
        winc_core::movement::snap_to_grapheme(&self.text, pos)
    }

    /// 自动换行时上下移动 `delta` 个显示行，返回（新位置（字符偏移），目标显示列 `goal`），
    /// 保持在同一显示列上；不换行时返回 `None`。
    pub(crate) fn wrapped_vertical(&self, delta: isize) -> Option<(usize, usize)> {
        let wrap = self.folds.wrap()?;
        let tab_width = wrap.tab_width;
        let head = self.selection.head;
        let (line, col) = self.text.pos_to_line_col(head);
        let part = self.part_of(line, col);
        let goal = self.goal_cell.unwrap_or_else(|| {
            let window = self.window_at(line, part);
            let (range, indent) = window.row(part);
            let end = col.saturating_sub(window.col).clamp(range.start, range.end);
            winc_core::text::slice_cells(&window.chars[range.start..end], indent, tab_width)
        });
        let row = self.folds.row_of(line) as isize + part as isize + delta;
        if row < 0 {
            return Some((0, goal));
        }
        if row as usize >= self.row_count() {
            return Some((self.text.end_pos(), goal));
        }
        Some((self.pos_in_row(row as usize, goal, tab_width), goal))
    }

    /// 返回自动换行时「行首」「行尾」命令（`end` 为 `true` 时是行尾）的目标位置（字符偏移）。
    ///
    /// 目标先取当前段的首尾，已位于该处时取整行的首尾；应由普通的行首、行尾命令处理时返回
    /// `None`。
    pub(crate) fn wrapped_home_end(&self, end: bool) -> Option<usize> {
        self.folds.wrap()?;
        let head = self.selection.head;
        let (line, col) = self.text.pos_to_line_col(head);
        if self.rows_of(line) == 1 {
            return None;
        }
        let part = self.part_of(line, col);
        let window = self.window_at(line, part);
        let (range, _) = window.row(part);
        let range = window.col + range.start..window.col + range.end;
        let start = self.text.line_start(line);
        if end {
            // 最后一段按普通行尾处理。其他段停在最后一个字符之前（再向后光标会绘制在下一段开头），
            // 已位于该处时才移到行尾。
            if window.is_last(part) {
                return None;
            }
            let stop = range.end - 1;
            return Some(if col == stop {
                self.text.line_end(line)
            } else {
                start + stop
            });
        }
        (part > 0 && col != range.start).then_some(start + range.start)
    }
}
