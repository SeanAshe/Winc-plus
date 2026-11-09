//! 代码折叠：折叠后的块只显示第一行，其余行不显示。
//!
//! 块按缩进划分，Markdown 按标题划分（见 [`scopes::fold_ranges`]）。折叠块记录第一行和最后一行的
//! 行尾（字符偏移），文本变化时随之移动（[`Tracked`]）；修改位于块内部时保持折叠，跨越块的边界时
//! 展开该块，避免修改用户看不到的文本。
//!
//! 编辑区按显示行排列：折叠的行不占显示行，自动换行时一行可以占多个显示行（见 [`Wrap`]）。

mod commands;
mod wrapping;

use wrapping::*;

pub(crate) use wrapping::WindowKey;
pub(crate) use wrapping::line_breaks;
pub use wrapping::{LineWindow, WrapJob, WrapKey, WrapResult};

use std::ops::{ControlFlow, Range};
use std::rc::Rc;

use winc_core::text::{EditBuffer, MAX_LINE_CHARS};
use winc_core::wrap::{self, Breaks};
use winc_core::{Lines, Selection};

use crate::document::Document;
use crate::document::content::Content;
use crate::editing::scopes::{self, FoldRange};
use crate::navigation::bookmarks::Bookmarks;

/// 一个折叠块：`start` 行保持显示，`start + 1..=end` 行不显示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    /// 折叠块的第一行（从 0 起），始终显示。
    pub start: usize,
    /// 折叠块的最后一行（从 0 起），不显示。
    pub end: usize,
    /// `start` 行的行尾位置（字符偏移）；文本变化时随之移动。
    head: usize,
    /// `end` 行的行尾位置（字符偏移）；文本变化时随之移动。
    tail: usize,
}

impl Fold {
    fn new(text: &impl Lines, start: usize, end: usize) -> Self {
        Fold {
            start,
            end,
            head: text.line_end(start),
            tail: text.line_end(end),
        }
    }
}

/// 一个文档的折叠块，以及由折叠块与筛选得出的不显示的行。
#[derive(Debug, Clone)]
pub struct Folds {
    /// 按开头行排序；块可以嵌套（外层展开后内层仍保持折叠）。
    list: Vec<Fold>,
    /// 「只显示匹配的行」隐藏的行：若干段连续的行（含两端，行号从 0 起），按顺序排列，互不相交。
    ///
    /// 与折叠块不同，可以从第 0 行开始，之前没有保持显示的行。
    filter: Vec<(usize, usize)>,
    /// 不显示的行：若干段连续的行（含两端，行号从 0 起），按顺序排列，互不相交。
    hidden: Vec<(usize, usize)>,
    /// `before[i]` 是 `hidden[..=i]` 共包含的行数（个）。
    ///
    /// 块数很多时（例如大文件全部折叠后），行号与显示行的换算仍然只需二分查找。
    before: Vec<usize>,
    /// 自动换行的设置与各行的段数；`None` 表示未开启。
    wrap: Option<Wrap>,
    /// 处于显示状态且被断成多段的行，按行号排序，用于换算显示行。
    wrapped: Vec<Wrapped>,
    /// 文本修改后 `Wrap::extra` 是否有变化、`wrapped` 尚未随之重算。
    ///
    /// 多数按键只修改一行内的文字，该行的段数不变，`wrapped` 仍然可用。自动换行的大文件中被断开的
    /// 行可达数十万行，每次按键都重新计算会占用输入处理的大部分时间。
    wrapped_stale: bool,
}

/// 自动换行的设置：断行宽度，以及被断成多段的行各自额外占用的显示行数。
#[derive(Debug, Clone)]
pub(crate) struct Wrap {
    /// 断行宽度（显示列）。
    pub width: usize,
    /// 一个 Tab 的显示宽度（显示列）。
    pub tab_width: usize,
    /// 每行参与断行的字符数上限（字符），见 [`Document::wrap_limit`]。
    pub limit: usize,
    /// 被断开的行及各自额外占用的显示行数：（行号，额外显示行数），按行号排序。
    extra: Vec<(usize, usize)>,
    /// 长行的行内断点索引，按行号排序；只在整行断开（`limit` 不设上限）时建立。
    long: Vec<LongRows>,
    /// 是否只断开了屏幕附近的行（见 [`Document::preview_wrap`]）：其余行暂按一段显示，
    /// 等待后台任务给出全文的结果。
    pub partial: bool,
    /// 计算时对应的文档版本；与文档当前版本不一致时整体重算
    /// （例如改名后 CSV 的列对齐可能随之变化）。
    ///
    /// 文本修改、末尾追加时只重算受影响的行，并同步记录新的版本。
    pub revision: u64,
}

impl Wrap {
    /// 第 `line` 行是否被断成了多段（二分查找，不读文字）。
    pub fn breaks_line(&self, line: usize) -> bool {
        self.extra.binary_search_by_key(&line, |&(l, _)| l).is_ok()
    }

    /// 返回第 `line` 行的行内断点索引；该行不是长行时返回 `None`。
    pub fn long_rows(&self, line: usize) -> Option<&LongRows> {
        let i = self.long.binary_search_by_key(&line, |l| l.line).ok()?;
        Some(&self.long[i])
    }
}

/// 行内断点索引的间隔（段）：每隔该段数记录一个段的开头。
///
/// 读取长行中的任意一段时，从该段之前最近的记录处开始断行，最多额外计算 `ROW_STRIDE - 1` 段；
/// 按每段 100 到 200 个字符计，约为 1 万到 3 万个字符。每条记录占 8 字节，每 MB 文字约占 0.5 KB。
pub(crate) const ROW_STRIDE: usize = 128;

/// 长行的行内断点索引：段数超过 [`ROW_STRIDE`] 的行每隔 `ROW_STRIDE` 段记录一个段的开头。
///
/// 这类行可达数 MB、数千段。借助索引，绘制与定位只需读取可见段之前最近一处记录起的文字
/// （见 [`Document::line_window`]），开销与行的长度无关。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LongRows {
    /// 行号（从 0 起）。
    pub line: usize,
    /// 该行参与断行的字符数（字符）。
    pub len: usize,
    /// 后续各段的缩进宽度（显示列）。
    pub indent: usize,
    /// 第 `k` 项是第 `(k + 1) * ROW_STRIDE` 段开头的字符序号（行内，从 0 起）。
    pub stops: Vec<usize>,
}

impl LongRows {
    /// 由第 `line` 行（共 `len` 个字符）的断行结果建立索引；段数不超过 [`ROW_STRIDE`] 时返回
    /// `None`。
    fn of(line: usize, len: usize, breaks: &Breaks) -> Option<Self> {
        (breaks.rows() > ROW_STRIDE).then(|| LongRows {
            line,
            len,
            indent: breaks.indent,
            // `starts[i]` 是第 `i + 1` 段的开头。
            stops: (breaks.starts.iter().copied())
                .skip(ROW_STRIDE - 1)
                .step_by(ROW_STRIDE)
                .collect(),
        })
    }
}

/// 一批行的断行结果：被断开的行各自额外占用的显示行数，以及其中长行的行内断点索引。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WrapRows {
    /// （行号，额外占用的显示行数），按行号排序。
    pub extra: Vec<(usize, usize)>,
    /// 长行的行内断点索引，按行号排序。
    pub long: Vec<LongRows>,
}

impl WrapRows {
    /// 记入第 `line` 行（共 `len` 个字符）的断行结果；`index` 表示是否为长行建立行内索引。
    fn push(&mut self, line: usize, len: usize, breaks: &Breaks, index: bool) {
        if breaks.rows() > 1 {
            self.extra.push((line, breaks.rows() - 1));
        }
        if index {
            self.long.extend(LongRows::of(line, len, breaks));
        }
    }
}

/// 一个处于显示状态且被断成多段的行。
#[derive(Debug, Clone, Copy)]
struct Wrapped {
    /// 行号（从 0 起）。
    line: usize,
    /// 不计换行时该行的显示行号（从 0 起，见 [`Folds::visible_index`]）。
    vis: usize,
    /// 该行之前处于显示状态的行共额外占用的显示行数（显示行）。
    before: usize,
    /// 该行自身额外占用的显示行数（显示行）。
    extra: usize,
}

impl Default for Folds {
    fn default() -> Self {
        Folds {
            list: Vec::new(),
            filter: Vec::new(),
            hidden: Vec::new(),
            before: vec![0],
            wrap: None,
            wrapped: Vec::new(),
            wrapped_stale: false,
        }
    }
}

impl Folds {
    /// 是否既没有折叠块也没有筛选，即所有行都显示。
    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.filter.is_empty()
    }

    /// 是否处于「只显示匹配的行」状态。
    pub fn is_filtered(&self) -> bool {
        !self.filter.is_empty()
    }

    /// 返回按开头行排序的所有折叠块。
    pub fn iter(&self) -> impl Iterator<Item = &Fold> {
        self.list.iter()
    }

    /// 返回包含 `line` 的一段不显示的行（含两端）；`line` 处于显示状态时返回 `None`。
    fn hidden_span(&self, line: usize) -> Option<(usize, usize)> {
        let i = self.hidden.partition_point(|&(_, b)| b < line);
        self.hidden.get(i).copied().filter(|&(a, _)| a <= line)
    }

    /// 是否不显示第 `line` 行：在折叠块内部或被筛选掉时为 `true`。
    pub fn is_hidden(&self, line: usize) -> bool {
        self.hidden_span(line).is_some()
    }

    /// 是否有以第 `line` 行为第一行的折叠块（不论该行本身是否显示）。
    fn has(&self, line: usize) -> bool {
        self.list.binary_search_by_key(&line, |f| f.start).is_ok()
    }

    /// 是否有以第 `line` 行为第一行的折叠块，且该行处于显示状态。
    pub fn is_collapsed(&self, line: usize) -> bool {
        !self.is_hidden(line) && self.has(line)
    }

    /// 返回不计换行时第 `line` 行所在的显示行号（即在显示的行中的序号，从 0 起）。
    fn visible_index(&self, line: usize) -> usize {
        let line = match self.hidden_span(line) {
            Some((0, _)) => return 0,
            Some((a, _)) => a - 1,
            None => line,
        };
        line - self.before[self.hidden.partition_point(|&(a, _)| a < line)]
    }

    /// 返回第 `line` 行之前处于显示状态、被断开的行共额外占用的显示行数。
    fn extra_before_line(&self, line: usize) -> usize {
        let i = self.wrapped.partition_point(|w| w.line < line);
        i.checked_sub(1)
            .map_or(0, |j| self.wrapped[j].before + self.wrapped[j].extra)
    }

    /// 返回第 `line` 行的第一段所在的显示行号（从 0 起）。
    ///
    /// 不显示的行计入其前最近一个显示的行；从第 0 行起不显示的行计入第 0 个显示行。
    pub fn row_of(&self, line: usize) -> usize {
        let shown = match self.hidden_span(line) {
            Some((0, _)) => return 0,
            Some((a, _)) => a - 1,
            None => line,
        };
        self.visible_index(shown) + self.extra_before_line(shown)
    }

    /// 返回第 `line` 行（应为处于显示状态的行）占用的显示行数。
    pub fn rows_of(&self, line: usize) -> usize {
        let i = self.wrapped.partition_point(|w| w.line < line);
        1 + self
            .wrapped
            .get(i)
            .filter(|w| w.line == line)
            .map_or(0, |w| w.extra)
    }

    /// 返回第 `row` 个显示行对应的（行号，段号），均从 0 起。
    pub fn line_and_part(&self, row: usize) -> (usize, usize) {
        // 最后一处起点不晚于 `row` 的被断开的行。
        let i = self.wrapped.partition_point(|w| w.vis + w.before <= row);
        if let Some(w) = i.checked_sub(1).map(|j| self.wrapped[j]) {
            let start = w.vis + w.before;
            if row <= start + w.extra {
                return (w.line, row - start);
            }
            return (self.line_of_visible(row - w.before - w.extra), 0);
        }
        (self.line_of_visible(row), 0)
    }

    /// 返回第 `row` 个显示行对应的行号（从 0 起）。
    pub fn line_of_row(&self, row: usize) -> usize {
        self.line_and_part(row).0
    }

    /// 返回不计换行时第 `row` 个显示行对应的行号（从 0 起）。
    fn line_of_visible(&self, row: usize) -> usize {
        // 第 `i` 段之后的行位于第 `a - before[i]` 个显示行；该行不在 `row` 之后时跳过第 `i` 段。
        let (mut lo, mut hi) = (0, self.hidden.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.hidden[mid].0 - self.before[mid] <= row {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        row + self.before[lo]
    }

    /// 返回共 `line_count` 行的文件的显示行数（至少 1 个）。
    pub fn row_count(&self, line_count: usize) -> usize {
        let hidden = self.before.last().copied().unwrap_or(0);
        let extra = self.wrapped.last().map_or(0, |w| w.before + w.extra);
        line_count.saturating_sub(hidden).max(1) + extra
    }

    /// 是否已开启自动换行。
    pub fn is_wrapping(&self) -> bool {
        self.wrap.is_some()
    }

    /// 返回从 `line` 起向下第一个显示的行的行号；`line` 本身显示时返回 `line`。
    pub fn visible_from(&self, line: usize) -> usize {
        self.hidden_span(line).map_or(line, |(_, b)| b + 1)
    }

    /// 返回从第 `first_row` 个显示行起向下 `rows` 个显示行对应的行：若干段连续的行（左闭右开，
    /// 行号从 0 起）；第一行可以只显示其后部的若干段。
    pub fn segments(&self, first_row: usize, rows: usize, line_count: usize) -> Vec<Range<usize>> {
        if self.wrapped.is_empty() {
            return self.visible_segments(first_row, rows, line_count);
        }
        // 换算成不计换行的显示行：从第一行到最后一行（含）。
        let first = self.line_of_row(first_row);
        let last = self.line_of_row(first_row + rows.max(1) - 1);
        let (v0, v1) = (self.visible_index(first), self.visible_index(last));
        self.visible_segments(v0, v1 - v0 + 1, line_count)
    }

    fn visible_segments(
        &self,
        first_row: usize,
        rows: usize,
        line_count: usize,
    ) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut line = self.line_of_visible(first_row);
        let mut left = rows;
        while left > 0 && line < line_count {
            let next = self
                .hidden
                .get(self.hidden.partition_point(|&(a, _)| a <= line))
                .copied();
            let stop = next
                .map_or(line_count, |(a, _)| a)
                .min(line + left)
                .min(line_count);
            out.push(line..stop);
            left -= stop - line;
            line = match next {
                Some((a, b)) if stop == a => b + 1,
                _ => stop,
            };
        }
        out
    }

    /// 返回折叠前后差异所在的显示行范围；差异不是单个连续范围时返回 `None`。
    ///
    /// `open` 是展开状态，`closed` 是折叠状态；差异指在 `open` 中显示、在 `closed` 中折叠的行，
    /// 范围按 `open` 的显示行号（从 0 起）计。仅有一处且范围连续时（折叠或展开一个块及其内部的块）
    /// 才有结果，动画作用于该范围；差异分为多处（例如全部折叠）时返回 `None`。
    pub fn changed_rows(open: &Folds, closed: &Folds) -> Option<Range<usize>> {
        let inside = |(a, b): (usize, usize)| {
            closed
                .hidden_span(a)
                .is_some_and(|(ca, cb)| ca <= a && b <= cb)
        };
        if !open.hidden.iter().all(|&span| inside(span)) {
            return None;
        }

        // `closed` 的每一段减去 `open` 中已经不显示的行，剩余的行在 `open` 中均显示。
        let mut found: Option<Range<usize>> = None;
        for &(ca, cb) in &closed.hidden {
            let mut at = ca;
            let first = open.hidden.partition_point(|&(_, b)| b < ca);
            let mut gaps = Vec::new();
            for &(a, b) in open.hidden[first..].iter().take_while(|&&(a, _)| a <= cb) {
                if a > at {
                    gaps.push((at, a - 1));
                }
                at = at.max(b + 1);
            }
            if at <= cb {
                gaps.push((at, cb));
            }
            for (a, b) in gaps {
                let rows = open.row_of(a)..open.row_of(b) + open.rows_of(b);
                found = match found {
                    None => Some(rows),
                    Some(prev) if prev.end == rows.start => Some(prev.start..rows.end),
                    Some(_) => return None,
                };
            }
        }
        found
    }

    /// 折叠 `folds` 中的块并跳过已折叠的块，返回是否有新折叠的块。
    ///
    /// 全部折叠时一次插入的块可达数万个，因此全部插入后只重算一次。
    fn insert(&mut self, folds: impl IntoIterator<Item = Fold>) -> bool {
        let new: Vec<Fold> = folds
            .into_iter()
            .filter(|f| f.end > f.start && !self.has(f.start))
            .collect();
        if new.is_empty() {
            return false;
        }
        self.list.extend(new);
        self.list.sort_by_key(|f| f.start);
        self.list.dedup_by_key(|f| f.start);
        self.rebuild();
        true
    }

    /// 移除满足 `drop` 的块，返回是否有块被移除。
    fn remove(&mut self, drop: impl Fn(&Fold) -> bool) -> bool {
        let before = self.list.len();
        self.list.retain(|f| !drop(f));
        let changed = self.list.len() != before;
        if changed {
            self.rebuild();
        }
        changed
    }

    fn clear(&mut self) {
        self.list.clear();
        self.filter.clear();
        self.rebuild();
    }

    /// 只显示 `shown` 列出的行（按顺序），第 `end` 行及之后尚未搜索的行全部显示；`shown` 为
    /// `None` 时取消筛选。
    fn set_filter(&mut self, shown: Option<&[usize]>, line_count: usize, end: usize) {
        self.filter.clear();
        if let Some(shown) = shown {
            let end = end.min(line_count);
            let mut next = 0;
            for &line in shown.iter().filter(|&&l| l < end) {
                if line > next {
                    self.filter.push((next, line - 1));
                }
                next = next.max(line + 1);
            }
            if next < end {
                self.filter.push((next, end - 1));
            }
            // 没有任何行显示时保留最后一行，避免编辑区空白。文件只有一行时无法保留该行之外的行，
            // 因此不筛选，该行仍然显示。
            if shown.is_empty() && end == line_count && end <= 1 {
                self.filter.clear();
            } else if shown.is_empty() && end == line_count {
                if let Some(last) = self.filter.last_mut() {
                    last.1 -= 1;
                }
                if let Some(&(a, b)) = self.filter.last()
                    && b < a
                {
                    self.filter.pop();
                }
            }
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.hidden.clear();
        let mut spans: Vec<(usize, usize)> = self
            .list
            .iter()
            .map(|fold| (fold.start + 1, fold.end))
            .chain(self.filter.iter().copied())
            .collect();
        if !self.filter.is_empty() {
            spans.sort_unstable();
        }
        for (a, b) in spans {
            match self.hidden.last_mut() {
                // 块的第一行已在上一段中（嵌套的块）或与上一段相邻：合并入上一段。
                Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
                _ => self.hidden.push((a, b)),
            }
        }
        self.before.clear();
        self.before.push(0);
        let mut total = 0;
        for &(a, b) in &self.hidden {
            total += b - a + 1;
            self.before.push(total);
        }
        self.rebuild_wrapped();
    }

    /// 按当前的折叠和换行重新计算 `wrapped`。
    fn rebuild_wrapped(&mut self) {
        self.wrapped.clear();
        self.wrapped_stale = false;
        let Some(wrap) = self.wrap.take() else {
            return;
        };
        let mut before = 0;
        for &(line, extra) in &wrap.extra {
            if self.is_hidden(line) {
                continue;
            }
            let vis = self.visible_index(line);
            self.wrapped.push(Wrapped {
                line,
                vis,
                before,
                extra,
            });
            before += extra;
        }
        self.wrap = Some(wrap);
    }

    /// 开启、关闭自动换行或更换宽度：按 `text` 整体重新计算。
    ///
    /// `wrap` 是（断行宽度，Tab 宽度），单位均为显示列，`None` 表示关闭；`limit` 是每行参与断行的
    /// 字符数上限，`revision` 是此时的文档版本号。
    pub(crate) fn set_wrap(
        &mut self,
        text: &impl Lines,
        wrap: Option<(usize, usize)>,
        limit: usize,
        revision: u64,
    ) {
        self.wrap = wrap.map(|(width, tab_width)| {
            let rows = wrapped_lines(text, 0..usize::MAX, width, tab_width, limit);
            Wrap {
                width,
                tab_width,
                limit,
                extra: rows.extra,
                long: rows.long,
                partial: false,
                revision,
            }
        });
        self.rebuild();
    }

    /// 采用已完成的自动换行结果，例如后台完成的大文件断行。
    pub(crate) fn install_wrap(&mut self, wrap: Wrap) {
        self.wrap = Some(wrap);
        self.rebuild();
    }

    /// 返回自动换行的设置；未开启时返回 `None`。
    pub(crate) fn wrap(&self) -> Option<&Wrap> {
        self.wrap.as_ref()
    }

    /// 返回自动换行设置的可变引用；未开启时返回 `None`。
    pub(crate) fn wrap_mut(&mut self) -> Option<&mut Wrap> {
        self.wrap.as_mut()
    }

    /// 按改动重新计算断行：改动前是第 `first..=old_last` 行，改动后是第 `first..=new_last` 行。
    fn edit_wrap(&mut self, text: &impl Lines, first: usize, old_last: usize, new_last: usize) {
        let Some(wrap) = &mut self.wrap else {
            return;
        };
        let delta = new_last as isize - old_last as isize;
        let from = wrap.extra.partition_point(|&(l, _)| l < first);
        let to = wrap.extra.partition_point(|&(l, _)| l <= old_last);
        let mut fresh = WrapRows::default();
        for line in first..=new_last.min(text.line_count().saturating_sub(1)) {
            let (breaks, len) = measure_line(text, line, wrap.width, wrap.tab_width, wrap.limit);
            fresh.push(line, len, &breaks, wrap.limit == usize::MAX);
        }
        let long_from = wrap.long.partition_point(|l| l.line < first);
        let long_to = wrap.long.partition_point(|l| l.line <= old_last);
        // 行数不变（行内输入、同一行内的全部替换）时其后的行号无需移动：全部替换一次可能修改
        // 上万处，每处都遍历其后所有被断开的行会显著增加耗时。
        if delta != 0 && to < wrap.extra.len() {
            for entry in &mut wrap.extra[to..] {
                entry.0 = entry.0.saturating_add_signed(delta);
            }
            self.wrapped_stale = true;
        }
        if delta != 0 {
            for entry in &mut wrap.long[long_to..] {
                entry.line = entry.line.saturating_add_signed(delta);
            }
        }
        if wrap.extra[from..to] != fresh.extra[..] {
            wrap.extra.splice(from..to, fresh.extra);
            self.wrapped_stale = true;
        }
        // 行内文字变化后断点随之移动，即使段数不变也要替换索引。
        wrap.long.splice(long_from..long_to, fresh.long);
    }

    /// 按一处修改调整折叠块的位置和行号，并移除修改跨越边界的块。
    ///
    /// 修改是在 `start`（字符偏移）处删除 `removed` 个字符并插入 `inserted`，行数增加 `lines`
    /// 行（减少时为负）。不按位置重新查找行号：全部折叠后块数可达数万个，
    /// 输入一个字符时不能遍历全部块。
    fn edit(&mut self, start: usize, removed: usize, inserted: &str, lines: isize) {
        // 文本修改后筛选出的行号不再对应，待重新搜索完成后再按新的结果筛选。
        self.filter.clear();
        let end = start + removed;
        let delta = inserted.chars().count() as isize - removed as isize;
        let shift = |pos: usize| pos.saturating_add_signed(delta);
        let breaks = inserted.contains(['\n', '\r']);
        self.list.retain_mut(|f| {
            if end < f.head || (end == f.head && !breaks) {
                // 修改位于块的第一行或其上方。
                f.head = shift(f.head);
                f.tail = shift(f.tail);
                // 修改都在第一行的行尾之前，增减的行都在该行之上。
                f.start = f.start.saturating_add_signed(lines);
                f.end = f.end.saturating_add_signed(lines);
                true
            } else if start > f.head && end <= f.tail {
                // 修改位于被折叠的行内部。
                f.tail = shift(f.tail);
                f.end = f.end.saturating_add_signed(lines);
                f.end > f.start
            } else {
                start > f.tail
            }
        });
    }

    /// 文本修改后重新计算不显示的行和 `wrapped`。
    ///
    /// 折叠块可能在本次修改中全部被移除（修改跨越边界时移除），修改也会取消筛选，因此修改前存在
    /// 不显示的行时必须整体重算，否则 `hidden` 保留旧的范围，导致行被错误地隐藏。修改前后都没有
    /// 不显示的行时 `hidden` 不变，`wrapped` 只在被断开的行有变化时重算。
    fn refresh(&mut self) {
        if self.is_empty() && self.hidden.is_empty() {
            if self.wrapped_stale {
                self.rebuild_wrapped();
            }
        } else {
            self.rebuild();
        }
    }
}

/// 修改文本时一并移动折叠块和书签的位置。
pub(crate) struct Tracked<'a> {
    /// 被修改的文本。
    pub text: &'a mut Content,
    /// 需要随修改移动的折叠状态。
    pub folds: &'a mut Folds,
    /// 需要随修改移动的书签。
    pub bookmarks: &'a mut Bookmarks,
}

impl EditBuffer for Tracked<'_> {
    fn replace(&mut self, start: usize, removed: usize, inserted: &str) {
        let before = self.text.line_count();
        // 书签需要修改所在的行范围，该范围必须在修改之前计算。
        let (start_line, end_line, whole_lines) = if self.bookmarks.is_empty() {
            (0, 0, false)
        } else {
            let end = (start + removed).min(self.text.end_pos());
            (
                self.text.line_of(start),
                self.text.line_of(end),
                removed > 0
                    && self.text.line_start(self.text.line_of(start)) == start
                    && self.text.line_start(self.text.line_of(end)) == end,
            )
        };
        // 自动换行需要重新计算受影响的行。
        let wrap_lines = self.folds.wrap.is_some().then(|| {
            let end = (start + removed).min(self.text.end_pos());
            (self.text.line_of(start), self.text.line_of(end))
        });
        self.text.replace(start, removed, inserted);
        let after = self.text.line_count();
        if !self.folds.is_empty() {
            self.folds
                .edit(start, removed, inserted, after as isize - before as isize);
        }
        if let Some((first, old_last)) = wrap_lines {
            let new_last = old_last.saturating_add_signed(after as isize - before as isize);
            self.folds.edit_wrap(&*self.text, first, old_last, new_last);
        }
        if !self.bookmarks.is_empty() {
            self.bookmarks.edit(
                start_line,
                end_line,
                after as isize - before as isize,
                whole_lines,
            );
        }
    }

    /// 没有需要随之移动的折叠、自动换行和书签时，整批修改交给文本一次完成（大文件的片段表一次
    /// 处理）；否则逐处修改，每处先计算其所在的行范围。
    fn replace_each(&mut self, changes: &[(usize, usize, &str)]) {
        if self.folds.is_empty() && self.folds.wrap.is_none() && self.bookmarks.is_empty() {
            self.text.replace_each(changes);
            return;
        }
        for &(start, removed, inserted) in changes {
            self.replace(start, removed, inserted);
        }
    }
}

/// 折叠命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldCommand {
    /// 折叠光标所在的、仍然展开的最内层块（⌥⌘[）。
    Fold,
    /// 展开光标所在的、已折叠的最内层块（⌥⌘]）。
    Unfold,
    /// 切换光标所在的最内层块：已折叠则展开，已展开则折叠（⌘K ⌘L）。
    Toggle,
    /// 折叠光标所在的块及其内部的所有块（⌘K ⌘[）。
    FoldRecursively,
    /// 展开光标所在的块及其内部的所有块（⌘K ⌘]）。
    UnfoldRecursively,
    /// 折叠所有块（⌘K ⌘0）。
    FoldAll,
    /// 展开所有块（⌘K ⌘J）。
    UnfoldAll,
    /// 折叠第 `n` 层（从 1 起）的所有块，光标所在的块除外（⌘K ⌘1 到 ⌘K ⌘7）。
    FoldLevel(usize),
}

impl Document {}

#[cfg(test)]
mod tests;
