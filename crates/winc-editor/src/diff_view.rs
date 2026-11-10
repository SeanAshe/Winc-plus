//! 并排差异视图的数据：两侧的文本、各段改动、两侧各自的显示行，以及每段改动内的逐词标记。
//!
//! 两侧各自排列显示行，不插入占位空行：一段改动左侧占 `a` 行、右侧占 `b` 行，界面在两侧之间绘制
//! 连接带。滚动位置是两侧共用的坐标（[`DiffView::row_at`]）：未改动的内容两侧逐行对应，每段改动
//! 按行数较多的一侧计算。长段未改动的内容默认折叠，只保留改动前后各 [`CONTEXT`] 行。

use std::ops::Range;
use std::path::PathBuf;

use ropey::Rope;
use winc_core::Lines;
use winc_core::diff::{self, LineHunk};
use winc_core::text::{self, MAX_LINE_CHARS};
use winc_core::wrap::Breaks;
use winc_syntax::{Highlighter, Language};

use crate::document::{STICKY_HIGHLIGHT_BUDGET, rope_starts_quoted};
use crate::editing::folding::line_breaks;
use crate::editing::scopes::{self, Scope, ScopeMemo};

/// 改动前后各自保留的未改动行数（行）。
///
/// 取自调参：取值过小时，改动邻接的语句和块头被折叠，无法判断改动的上下文；取值过大时，长段未改动
/// 内容显示的行数随之增加，折叠节省的显示行变少。作用范围是每段改动两侧的未改动内容，只影响折叠
/// 后显示的行数，不参与两侧对齐的计算。
pub const CONTEXT: usize = 4;
/// 一段改动任一侧超过该行数（行，200）时不再按词比较。
///
/// 改动段较长时通常整段都已改动，逐词标记不再有区分度，只标记整行即可。数值取自调参：逐词比较的
/// 耗时随改动段的行数增长，行数越多，每次重新比较所需的时间越长。
const INLINE_LIMIT: usize = 200;

/// 自动换行的行数上限（行）：任一侧的行数超过该值时不自动换行。
///
/// 依据实测：整篇断行约每 20 万行耗时 115 ms，每次编辑都要重新断行，行数更多时输入出现明显延迟。
pub const WRAP_LINES: usize = 200_000;

/// 差异的一侧。
#[derive(Debug)]
pub struct DiffSide {
    /// 显示在该侧上方的标题，例如「HEAD」「暂存区」「工作区」「磁盘上的版本」。
    pub title: String,
    /// 该侧的全部文本。
    pub text: Rope,
    /// 该侧对应的磁盘文件；内容不来自磁盘（某个提交、暂存区中的版本）时为 `None`。
    pub path: Option<PathBuf>,
    /// 该侧对应的版本中是否不存在该文件（尚未添加、已删除）。
    ///
    /// 为 `true` 时文本为空，标题已说明原因，界面不再显示「空文件」。
    pub missing: bool,
    /// 语法着色的增量状态，绘制时推进。
    pub highlighter: Highlighter,
    /// 文字的版本号：右侧随工作区文件改变时加 1，使按内容缓存的结果失效。
    revision: u64,
    /// 粘滞滚动查找外层块时记录的结果，见 [`ScopeMemo`]。
    sticky_memo: ScopeMemo,
}

impl DiffSide {
    /// 用标题、文本和对应的文件创建一侧，按 `language` 着色。
    pub fn new(
        title: impl Into<String>,
        text: &str,
        path: Option<PathBuf>,
        language: &'static Language,
    ) -> DiffSide {
        DiffSide {
            title: title.into(),
            text: Rope::from_str(text),
            path,
            missing: false,
            highlighter: Highlighter::new(language),
            revision: 0,
            sticky_memo: ScopeMemo::default(),
        }
    }

    /// 设置该侧对应的文件在该版本中是否不存在（见 [`DiffSide::missing`]）。
    pub fn with_missing(mut self, missing: bool) -> DiffSide {
        self.missing = missing;
        self
    }

    /// 返回该侧自上而下显示 `rows` 各行时固定在顶部的块（从外层到内层，最多 `max` 个），以及
    /// 高亮是否已推算到位；参数与返回值的含义同 [`crate::Document::sticky_scopes`]。
    pub fn sticky_scopes(
        &mut self,
        rows: &[usize],
        partial: bool,
        max: usize,
        tab_width: usize,
    ) -> (Vec<Scope>, bool) {
        // 判断块是否在字符串中，需要 `rows` 及其下方 `SCAN_DOWN` 行的高亮状态，先推算到那里。
        let last = rows.last().copied().unwrap_or(0) + scopes::SCAN_DOWN;
        let settled = (self.highlighter)
            .state_at(&self.text, last, STICKY_HIGHLIGHT_BUDGET)
            .is_some();
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| rope_starts_quoted(highlighter, text, line);
        let memo = self.sticky_memo.records(self.revision, tab_width);
        let found = scopes::sticky(
            text,
            language,
            rows,
            partial,
            max,
            tab_width,
            &mut quoted,
            memo,
        );
        if !settled {
            self.sticky_memo.discard();
        }
        (found, settled)
    }

    /// 返回包含第 `line` 行的块的层数，即该行显示在固定行下方时顶部固定的行数。
    pub fn scope_depth(&mut self, line: usize, tab_width: usize) -> usize {
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| rope_starts_quoted(highlighter, text, line);
        let memo = self.sticky_memo.get(self.revision, tab_width);
        scopes::depth(text, language, line, tab_width, &mut quoted, memo)
    }
}

/// 一侧的一个显示行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideRow {
    /// 该侧的一行（或自动换行后的一段）。
    ///
    /// `hunk` 是该行所在的改动（`DiffView::hunks` 的下标）；未改动的行为 `None`。
    Line {
        /// 行号（从 0 起）。
        line: usize,
        /// 所在改动的下标。
        hunk: Option<usize>,
        /// 自动换行时的段号（从 0 起）；不换行时为 0。
        part: usize,
    },
    /// 折叠的一段未改动的行。
    ///
    /// `gap` 是未改动内容的段号（第一段改动之前为 0），`lines` 是该侧被折叠的行。
    /// 两侧同一段的行数相同。
    Fold {
        /// 未改动内容的段号（从 0 起）。
        gap: usize,
        /// 该侧被折叠的行。
        lines: Range<usize>,
    },
}

/// 改动 `marks`（按位置排列）是否覆盖行 `chars` 中的全部非空白字符。
///
/// 整行都已改动时逐词标记不再有区分度，只绘制行底色即可。没有非空白字符时返回 `false`。
pub fn marks_cover_line(chars: &[char], marks: &[Range<usize>]) -> bool {
    let first = chars.iter().position(|c| !c.is_whitespace());
    let last = chars.iter().rposition(|c| !c.is_whitespace());
    let (Some(first), Some(last)) = (first, last) else {
        return false;
    };
    // 标记按位置排列，单次遍历即可完成：一行最多 `MAX_LINE_CHARS` 个字符，每次绘制都要判断。
    let mut marks = marks.iter();
    let mut mark = marks.next();
    for (i, c) in chars.iter().enumerate().take(last + 1).skip(first) {
        if c.is_whitespace() {
            continue;
        }
        while mark.is_some_and(|m| m.end <= i) {
            mark = marks.next();
        }
        if !mark.is_some_and(|m| m.contains(&i)) {
            return false;
        }
    }
    true
}

/// 一段改动中两侧逐行的逐词标记：`old[i]` 是该段改动左侧第 `i` 行中改动的字符区间。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineMarks {
    /// 左侧每一行中改动的字符区间（字符偏移，从行首起）。
    pub old: Vec<Vec<Range<usize>>>,
    /// 右侧每一行中改动的字符区间。
    pub new: Vec<Vec<Range<usize>>>,
}

/// 左侧的下标（`old` 一侧）。
pub const LEFT: usize = 0;
/// 右侧的下标（`new` 一侧）。
pub const RIGHT: usize = 1;

/// 两侧比较的结果：各段改动、逐词标记、按块提交用的 key。
///
/// 只依赖两侧的文本，可以在后台计算后交给 [`DiffView::apply`]。
#[derive(Debug, Clone)]
pub struct Compared {
    hunks: Vec<LineHunk>,
    inline: Vec<Option<InlineMarks>>,
    keys: Vec<u64>,
}

impl Compared {
    /// 比较 `old` 和 `new`。
    ///
    /// 按块提交用的 key 直接由比较时切分的行计算（切分方式同 `diff::pick_changes`），不必把每段
    /// 改动的每一行另行复制为 `String`。
    pub fn new(old: &Rope, new: &Rope) -> Compared {
        let (a, b) = (old.to_string(), new.to_string());
        let hunks = diff::line_hunks(&a, &b);
        let inline = hunks.iter().map(|h| inline_marks(old, new, h)).collect();
        let (la, lb) = (diff::lines(&a), diff::lines(&b));
        let keys = hunks
            .iter()
            .map(|h| {
                diff::hunk_key(
                    la[h.old.clone()].iter().copied(),
                    lb[h.new.clone()].iter().copied(),
                )
            })
            .collect();
        Compared {
            hunks,
            inline,
            keys,
        }
    }
}

/// 单栏时滚动位置与各侧显示行之间的换算表。
///
/// 从差异中取出后单独持有：绘制时需要同时修改差异两侧的着色状态。
#[derive(Debug, Clone)]
pub struct UnifiedMap(Vec<(f64, [f64; 2])>);

impl UnifiedMap {
    /// 返回第 `side` 侧显示行 `row` 对应的滚动位置。
    ///
    /// 每段改动先排左侧的行，再排右侧的行。该侧在一段改动中没有行时，取该段改动的结尾。
    pub fn pos_of(&self, side: usize, row: f64) -> f64 {
        let i = self.0.partition_point(|k| k.1[side] <= row);
        if i % 2 == 1 {
            let (start, from) = self.0[i - 1];
            let (_, to) = self.0[i];
            let before = if side == RIGHT {
                to[LEFT] - from[LEFT]
            } else {
                0.0
            };
            return start + before + row - from[side];
        }
        let (at, last) = i
            .checked_sub(1)
            .map_or((0.0, 0.0), |i| (self.0[i].0, self.0[i].1[side]));
        at + row - last
    }
}

/// 差异中只读一侧的选区，只支持选中和复制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadSelection {
    /// 所在的一侧：[`LEFT`] 或 [`RIGHT`]。
    pub side: usize,
    /// 起点和终点（该侧文本的字符偏移）。
    pub range: (usize, usize),
}

/// 并排差异视图。
#[derive(Debug)]
pub struct DiffView {
    /// 左侧：改动前的内容。
    pub old: DiffSide,
    /// 右侧：改动后的内容。
    pub new: DiffSide,
    /// 各段改动，按位置排列。
    pub hunks: Vec<LineHunk>,
    /// 每段改动的逐词标记，下标同 `hunks`；纯新增、纯删除、超过 `INLINE_LIMIT` 的改动为 `None`。
    pub inline: Vec<Option<InlineMarks>>,
    /// 两侧各自的显示行：`rows[LEFT]`、`rows[RIGHT]`。
    pub rows: [Vec<SideRow>; 2],
    /// 每段改动在两侧占用的显示行，下标同 `hunks`；该侧没有行时为插入位置上的空区间。
    pub hunk_rows: [Vec<Range<usize>>; 2],
    /// 各折叠行在两侧的显示行：（左侧，右侧），按位置排列。
    pub fold_rows: Vec<(usize, usize)>,
    /// 滚动位置的分段点：每段改动的开头和结尾各一个，（滚动位置，[左侧显示行，右侧显示行]），
    /// 见 [`DiffView::row_at`]。
    knots: Vec<(f64, [f64; 2])>,
    /// 是否折叠长段未改动的内容。
    collapse: bool,
    /// 是否单栏显示：未改动的行只显示一次，每段改动先排左侧的行，再排右侧的行。
    unified: bool,
    /// 自动换行的（断行宽度，Tab 宽度），单位是显示列；不换行时为 `None`。
    ///
    /// 两侧使用同一宽度：未改动的行两侧断行结果相同、行数相等，滚动位置的换算依赖这一点。
    wrap: Option<(usize, usize)>,
    /// 最近一次跳转到的改动序号，以及跳转后的滚动位置。
    ///
    /// 滚动位置未变时，「上一处」「下一处」和当前序号按该记录计算：最后几段改动无法滚动到顶部
    /// （编辑区滚过末行时只保留四成屏高），按滚动位置计算会得到前一段；用户滚动后改按滚动位置
    /// 计算当前序号，不再使用该记录。
    pub focus: Option<(usize, f64)>,
    /// 已展开的未改动内容，元素是各段的段号。
    expanded: Vec<usize>,
    /// 右侧需要保持显示的行；所在的未改动内容中只显示这些行，其余部分仍然折叠。
    ///
    /// 用「»」还原一段改动后，还原的行不再有改动，不应立即被折叠，也不应因此展开整段内容。
    pinned: Option<Range<usize>>,
    /// 已计算的最长一行的显示宽度缓存：（Tab 宽度，显示列数）。
    ///
    /// 内容不变时，滚动无需每次重新扫描全文。
    widest: std::cell::Cell<Option<(usize, usize)>>,
    /// 右侧内容所取自的文档版本号（[`crate::Document::revision`]）；右侧是磁盘内容时为
    /// `None`。
    ///
    /// 版本号变化时用 [`DiffView::set_new_text`] 重新比较。
    pub source: Option<u64>,
    /// 光标是否位于右侧：为 `true` 时输入的文字写入右侧对应的文档。
    pub editing: bool,
    /// 只读一侧的选区：左侧总是只读，右侧不是工作区中的文件（历史版本、已删除的文件）时同样只读。
    ///
    /// 没有时为 `None`。
    pub read_selection: Option<ReadSelection>,
    /// 每段改动的 [`diff::hunk_key`]，下标同 `hunks`：按块提交时据此识别取消勾选的改动。
    pub keys: Vec<u64>,
    /// 差异对应的仓库内文件路径（写法同仓库状态中的路径）；不是 Git 差异时为 `None`。
    ///
    /// 按块提交时据此找到提交视图中的文件。
    pub repo_path: Option<PathBuf>,
    /// 历史版本差异对应的文件路径，仅用于显示（见 [`DiffView::file_path`]）。
    ///
    /// 历史版本不能按块提交，因此不写入 [`DiffView::repo_path`]。
    pub snapshot_path: Option<PathBuf>,
}

impl DiffView {
    /// 比较 `old` 和 `new` 两侧，创建差异视图并排列两侧的显示行。
    ///
    /// 长段未改动的内容默认折叠。
    pub fn new(old: DiffSide, new: DiffSide) -> DiffView {
        let Compared {
            hunks,
            inline,
            keys,
        } = Compared::new(&old.text, &new.text);
        let mut view = DiffView {
            old,
            new,
            hunks,
            inline,
            rows: [Vec::new(), Vec::new()],
            hunk_rows: [Vec::new(), Vec::new()],
            fold_rows: Vec::new(),
            knots: Vec::new(),
            collapse: true,
            unified: false,
            wrap: None,
            focus: None,
            expanded: Vec::new(),
            pinned: None,
            widest: std::cell::Cell::new(None),
            source: None,
            editing: false,
            read_selection: None,
            keys,
            repo_path: None,
            snapshot_path: None,
        };
        view.layout();
        view
    }

    /// 把右侧替换为 `text`（编辑器中的修改）并重新比较。
    ///
    /// 改动的段数不变时，已展开的折叠保持展开。
    pub fn set_new_text(&mut self, text: Rope) {
        let compared = Compared::new(&self.old.text, &text);
        self.apply(text, compared);
    }

    /// 把右侧替换为 `text`；`compared` 是在后台针对同一份 `text` 计算的比较结果。
    pub fn apply(&mut self, text: Rope, compared: Compared) {
        let first = first_difference(&self.new.text, &text);
        self.new.highlighter.invalidate_from(first);
        self.new.sticky_memo.edited(self.new.revision, first);
        self.new.revision += 1;
        // 固定显示的行随上方增删的行移动：改动始于其上方时整体移动，始于其中间时只移动下边界。
        // 按一处连续的修改估算（输入、粘贴、撤销均属此类），足以使这些行保持在原来的内容上。
        if let Some(pinned) = &mut self.pinned {
            let delta = text.len_lines() as isize - self.new.text.len_lines() as isize;
            let shift = |line: usize| line.saturating_add_signed(delta).max(first);
            if first < pinned.start {
                *pinned = shift(pinned.start)..shift(pinned.end);
            } else if first < pinned.end {
                pinned.end = shift(pinned.end).max(pinned.start);
            }
        }
        self.new.text = text;
        // 右侧已替换为编辑器中的内容，文件必然存在，即使内容为空。
        self.new.missing = false;
        if compared.hunks.len() != self.hunks.len() {
            self.expanded.clear();
            self.focus = None;
        }
        self.hunks = compared.hunks;
        self.inline = compared.inline;
        self.keys = compared.keys;
        self.widest.set(None);
        self.layout();
    }

    /// 沿用 `old` 的界面状态（重新比较后替换旧视图时使用）。
    ///
    /// 沿用光标是否在右侧、是否折叠等设置；改动的段数不变时，已展开的折叠保持展开。
    pub fn inherit(&mut self, old: &DiffView) {
        self.editing = old.editing;
        self.pinned.clone_from(&old.pinned);
        // 选区所在一侧的文本未变（例如 `HEAD` 未变）时选区仍然有效。
        self.read_selection = old
            .read_selection
            .filter(|s| self.side(s.side).text == old.side(s.side).text);
        if self.repo_path.is_none() {
            self.repo_path.clone_from(&old.repo_path);
        }
        self.collapse = old.collapse;
        self.unified = old.unified;
        self.wrap = old.wrap;
        if old.hunks.len() == self.hunks.len() {
            self.expanded = old.expanded.clone();
        }
        self.layout();
    }

    /// 设置自动换行：`wrap` 是（断行宽度，Tab 宽度），单位是显示列，`None` 表示不换行。
    ///
    /// 任一侧超过 [`WRAP_LINES`] 行时不换行：每次编辑都要重新断行。设置变化时返回 `true`。
    pub fn set_wrap(&mut self, wrap: Option<(usize, usize)>) -> bool {
        let lines = self.old.text.len_lines().max(self.new.text.len_lines());
        let wrap = wrap.filter(|_| lines <= WRAP_LINES);
        if self.wrap == wrap {
            return false;
        }
        self.wrap = wrap;
        self.layout();
        true
    }

    /// 自动换行的（断行宽度，Tab 宽度），单位是显示列；不换行时为 `None`。
    pub fn wrap(&self) -> Option<(usize, usize)> {
        self.wrap
    }

    /// 是否自动换行。
    pub fn is_wrapping(&self) -> bool {
        self.wrap.is_some()
    }

    /// 返回第 `side` 侧第 `line` 行的分段；不换行时只有一段。
    pub fn breaks(&self, side: usize, line: usize) -> Breaks {
        let text = if side == LEFT {
            &self.old.text
        } else {
            &self.new.text
        };
        match self.wrap {
            Some((width, tab_width)) if line < text.len_lines() => {
                line_breaks(text, line, width, tab_width, MAX_LINE_CHARS)
            }
            _ => Breaks::default(),
        }
    }

    /// 返回第 `side` 侧第 `line` 行第 `col` 个字符所在的显示行；该行被折叠时返回所在的折叠行。
    pub fn row_of_col(&self, side: usize, line: usize, col: usize) -> usize {
        let row = self.row_of(side, line);
        match self.rows[side].get(row) {
            Some(SideRow::Line { line: at, .. }) if *at == line => {
                row + self.breaks(side, line).row_of(col)
            }
            _ => row,
        }
    }

    /// 把第 `side` 侧显示行 `row` 中的显示列 `cell` 换算为整行的显示列。
    ///
    /// 超出该段时停在该段最后一个字符之前，不进入下一段。
    pub fn cell_in_row(&self, side: usize, row: usize, cell: usize) -> usize {
        let (Some((_, tab_width)), Some(SideRow::Line { line, part, .. })) =
            (self.wrap, self.rows[side].get(row))
        else {
            return cell;
        };
        let breaks = self.breaks(side, *line);
        if breaks.starts.is_empty() {
            return cell;
        }
        let text = if side == LEFT {
            &self.old.text
        } else {
            &self.new.text
        };
        let chars: Vec<char> = text
            .line(*line)
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .take(MAX_LINE_CHARS)
            .collect();
        let prefix = text::prefix_cells(&chars, tab_width);
        let range = breaks.range(*part, chars.len());
        let inner = cell.saturating_sub(breaks.indent_of(*part));
        let end = if range.end < chars.len() {
            prefix[range.end.saturating_sub(1).max(range.start)]
        } else {
            prefix[range.end]
        };
        (prefix[range.start] + inner).min(end)
    }

    /// 返回右侧的显示行数。
    pub fn row_count(&self) -> usize {
        self.rows[RIGHT].len()
    }

    /// 返回可滚动的行数，即滚动位置的总长（见 [`DiffView::row_at`]）。
    ///
    /// 每段改动按行数较多的一侧计算，不只按右侧。
    pub fn scroll_rows(&self) -> usize {
        self.knots
            .last()
            .map_or(self.rows[RIGHT].len(), |&(pos, rows)| {
                (pos + (self.rows[RIGHT].len() as f64 - rows[RIGHT])).ceil() as usize
            })
    }

    /// 返回滚动位置 `pos` 对应的第 `side` 侧显示行（可以是小数）。
    ///
    /// 滚动位置是两侧共用的坐标：未改动的内容两侧逐行对应，每段改动占两侧中较多的行数，较短的
    /// 一侧按比例移动（为 0 行时保持不动）。如果只按右侧的行数滚动，左侧删除的长段在右侧只占
    /// 一个点，滚动时会被直接越过，超过半屏的部分无法显示。
    pub fn row_at(&self, side: usize, pos: f64) -> f64 {
        if self.unified {
            return self.unified_row_at(side, pos);
        }
        // 分段点按滚动位置排序：二分查找第一个不小于 `pos` 的分段点，在它与前一个之间插值。
        let i = self.knots.partition_point(|k| k.0 < pos);
        let from = self.knot(side, i);
        let Some(&(at, rows)) = self.knots.get(i) else {
            return from.1 + (pos - from.0);
        };
        let to = (at, rows[side]);
        let t = if to.0 > from.0 {
            (pos - from.0) / (to.0 - from.0)
        } else {
            1.0
        };
        from.1 + (to.1 - from.1) * t
    }

    /// 返回第 `i` 个分段点的前一个（滚动位置，第 `side` 侧的显示行）；`i` 为 0 时返回起点。
    fn knot(&self, side: usize, i: usize) -> (f64, f64) {
        i.checked_sub(1)
            .and_then(|i| self.knots.get(i))
            .map_or((0.0, 0.0), |&(at, rows)| (at, rows[side]))
    }

    /// 返回第 `side` 侧显示行 `row` 对应的滚动位置，是 [`DiffView::row_at`] 的逆运算。
    ///
    /// 该侧在一段改动中没有行（0 行）时取该段改动的开头。
    pub fn pos_of(&self, side: usize, row: f64) -> f64 {
        if self.unified {
            return self.unified_map().pos_of(side, row);
        }
        // 每一侧的显示行同样随分段点递增：二分查找第一个不小于 `row` 的分段点。
        let i = self.knots.partition_point(|k| k.1[side] < row);
        let from = self.knot(side, i);
        let Some(&(at, rows)) = self.knots.get(i) else {
            return from.0 + (row - from.1);
        };
        let to = (at, rows[side]);
        if to.1 > from.1 {
            return from.0 + (row - from.1) / (to.1 - from.1) * (to.0 - from.0);
        }
        from.0
    }

    /// 计算滚动位置为 `pos`、可见 `visible` 行时第 `side` 侧的滚动位置（显示行）。
    ///
    /// 两侧在同一对齐点上对齐：滚过半屏后对齐点位于视图中间；距开头不足半屏时对齐点随之上移，
    /// 位于开头时对齐点在顶边，两侧都停在第一行。
    pub fn side_scroll(&self, side: usize, pos: f64, visible: f64) -> f64 {
        // 单栏只有一列，无需对齐两侧。
        if self.unified {
            return self.row_at(side, pos);
        }
        let anchor = pos.min(visible / 2.0).max(0.0);
        let center = self.row_at(side, pos + anchor);
        (center - anchor).clamp(0.0, self.rows[side].len() as f64)
    }

    /// 返回第 `side` 侧顶部显示 `row` 时的滚动位置，是 [`DiffView::side_scroll`] 的逆运算。
    ///
    /// 距开头不足半屏时对齐点随滚动移动，没有直接的反算公式，因此改用二分查找。
    pub fn pos_of_top(&self, side: usize, row: f64, visible: f64) -> f64 {
        if self.unified {
            return self.pos_of(side, row);
        }
        let half = visible / 2.0;
        let pos = self.pos_of(side, row + half) - half;
        if pos >= half {
            return pos;
        }
        let (mut low, mut high) = (0.0, half.max(0.0));
        for _ in 0..24 {
            let mid = (low + high) / 2.0;
            if self.side_scroll(side, mid, visible) < row {
                low = mid;
            } else {
                high = mid;
            }
        }
        high
    }

    /// 是否单栏显示。
    pub fn is_unified(&self) -> bool {
        self.unified
    }

    /// 切换单栏显示与并排显示。
    ///
    /// 滚动位置的换算方式随之改变，调用方负责换算滚动位置。
    pub fn set_unified(&mut self, on: bool) {
        if self.unified != on {
            self.unified = on;
            self.layout();
        }
    }

    /// 返回单栏时滚动位置与各侧显示行之间的换算表。
    pub fn unified_map(&self) -> UnifiedMap {
        UnifiedMap(self.knots.clone())
    }

    /// 返回单栏第 `index` 个显示行（从 0 起）对应的（所在侧，该侧的显示行）。
    ///
    /// 未改动的行和折叠行计入右侧。超出最后一行时返回 `None`。
    pub fn unified_row(&self, index: usize) -> Option<(usize, usize)> {
        let pos = index as f64;
        let i = self.knots.partition_point(|k| k.0 <= pos);
        let (side, row) = if i % 2 == 1 {
            let (start, from) = self.knots[i - 1];
            let (_, to) = self.knots[i];
            let (old, offset) = (to[LEFT] - from[LEFT], pos - start);
            if offset < old {
                (LEFT, from[LEFT] + offset)
            } else {
                (RIGHT, from[RIGHT] + offset - old)
            }
        } else {
            let (at, row) = self.knot(RIGHT, i);
            (RIGHT, row + pos - at)
        };
        let row = row as usize;
        (row < self.rows[side].len()).then_some((side, row))
    }

    /// 返回单栏时滚动位置 `pos` 对应的第 `side` 侧显示行；位于另一侧的行上时取该侧的下一行。
    fn unified_row_at(&self, side: usize, pos: f64) -> f64 {
        let i = self.knots.partition_point(|k| k.0 <= pos);
        if i % 2 == 1 {
            let (start, from) = self.knots[i - 1];
            let (_, to) = self.knots[i];
            let (old, offset) = (to[LEFT] - from[LEFT], pos - start);
            return if side == LEFT {
                from[LEFT] + offset.min(old)
            } else {
                from[RIGHT] + (offset - old).max(0.0)
            };
        }
        let (at, row) = self.knot(side, i);
        row + pos - at
    }

    /// 返回滚动位置为 `pos`、可见 `visible` 行时，距顶边 `below` 行（可以是小数）处对应的
    /// 第 `side` 侧显示行。
    pub fn row_under(&self, side: usize, pos: f64, visible: f64, below: f64) -> usize {
        let row = if self.unified {
            self.row_at(side, (pos + below).floor())
        } else {
            self.side_scroll(side, pos, visible) + below
        };
        row.floor().max(0.0) as usize
    }

    /// 计算滚动位置为 `pos`、可见 `visible` 行时左侧的滚动位置，换算方式同
    /// [`DiffView::side_scroll`]。
    pub fn left_scroll(&self, pos: f64, visible: f64) -> f64 {
        self.side_scroll(LEFT, pos, visible)
    }

    /// 返回第 `side` 侧第 `line` 行所在的显示行；该行被折叠时返回所在的折叠行。
    ///
    /// 文件以换行符结尾时，末尾的空行不参与比较、没有对应的显示行，视为位于最后一个显示行之后。
    pub fn row_of(&self, side: usize, line: usize) -> usize {
        // 显示行按行号排序，因此使用二分查找：编辑时每帧都要按光标所在的行查找一次。
        self.rows[side].partition_point(|row| match row {
            SideRow::Line { line: l, .. } => *l < line,
            SideRow::Fold { lines, .. } => lines.end <= line,
        })
    }

    /// 返回右侧第 `line` 行所在的显示行，查找方式同 [`DiffView::row_of`]。
    pub fn row_of_new(&self, line: usize) -> usize {
        self.row_of(RIGHT, line)
    }

    /// 返回第 `side` 侧显示行 `row` 的行号；该显示行是折叠行或超出范围时返回 `None`。
    pub fn line_at(&self, side: usize, row: usize) -> Option<usize> {
        match self.rows[side].get(row)? {
            SideRow::Line { line, .. } => Some(*line),
            SideRow::Fold { .. } => None,
        }
    }

    /// 使右侧 `lines` 范围内的行（连同上下各 [`CONTEXT`] 行）保持显示，
    /// 所在未改动内容的其余部分仍然折叠。
    ///
    /// `lines` 为空时显示该位置上下各 [`CONTEXT`] 行。
    pub fn pin_new(&mut self, lines: Range<usize>) {
        self.pinned = Some(lines);
        self.layout();
    }

    /// 展开第 `side` 侧第 `line` 行所在的折叠；该行未被折叠时不做改动。
    pub fn reveal_line(&mut self, side: usize, line: usize) {
        let row = self.row_of(side, line);
        if let Some(SideRow::Fold { gap, .. }) = self.rows[side].get(row) {
            let gap = *gap;
            self.expand(gap);
        }
    }

    /// 返回第 `side` 侧。
    pub fn side(&self, side: usize) -> &DiffSide {
        if side == LEFT { &self.old } else { &self.new }
    }

    /// 返回第 `side` 侧（[`LEFT`] 或 [`RIGHT`]）的可变借用。
    pub fn side_mut(&mut self, side: usize) -> &mut DiffSide {
        if side == LEFT {
            &mut self.old
        } else {
            &mut self.new
        }
    }

    /// 返回只读一侧的光标：（所在侧，字符偏移），位于选区的终点。
    ///
    /// 没有选区，或该侧没有内容（文件不存在、已删除）时返回 `None`。
    pub fn read_caret(&self) -> Option<(usize, usize)> {
        let selection = self.read_selection?;
        let len = self.side(selection.side).text.len_chars();
        (len > 0).then(|| (selection.side, selection.range.1.min(len)))
    }

    /// 返回只读一侧选中的文本；没有选中内容时返回 `None`。
    pub fn read_selected(&self) -> Option<String> {
        let selection = self.read_selection?;
        let text = &self.side(selection.side).text;
        let (a, b) = selection.range;
        let (from, to) = (a.min(b), a.max(b).min(text.len_chars()));
        (from < to).then(|| text.slice(from..to).to_string())
    }

    /// 返回第 `hunk` 段改动左侧的文本（含换行符），用于把右侧的该段还原。
    pub fn old_lines(&self, hunk: usize) -> String {
        let Some(h) = self.hunks.get(hunk) else {
            return String::new();
        };
        let mut out = String::new();
        for line in h.old.clone() {
            out.extend(self.old.text.line(line).chunks());
        }
        out
    }

    /// 是否有一侧完全为空：文件不在 `HEAD` 中（新文件）、已在工作区中删除，
    /// 或该版本本身是空文件。
    ///
    /// 此时整个文件是一处改动。
    pub fn one_side_empty(&self) -> bool {
        self.old.text.len_chars() == 0 || self.new.text.len_chars() == 0
    }

    /// 返回差异所比较的文件路径，供标签的图标和提示使用；没有对应文件时（如合并冲突两侧的
    /// 文本）返回 `None`。
    ///
    /// 依次取历史版本的路径、仓库文件的路径、右侧和左侧的文件路径。
    pub fn file_path(&self) -> Option<&std::path::Path> {
        (self.snapshot_path.as_deref())
            .or(self.repo_path.as_deref())
            .or(self.new.path.as_deref())
            .or(self.old.path.as_deref())
    }

    /// 两侧是否没有改动；逐行比较时不区分换行符的写法，只有换行符不同时同样为 `true`。
    pub fn is_identical(&self) -> bool {
        self.hunks.is_empty()
    }

    /// 返回（新增行数，删除行数），即右侧改动中的行数与左侧改动中的行数。
    pub fn stats(&self) -> (usize, usize) {
        self.hunks.iter().fold((0, 0), |(add, del), h| {
            (add + h.new.len(), del + h.old.len())
        })
    }

    /// 是否折叠长段未改动的内容。
    pub fn collapses(&self) -> bool {
        self.collapse
    }

    /// 设置是否折叠长段未改动的内容，返回切换前右侧显示行 `row` 的内容
    /// 在切换后所在的显示行，使视图保持在原处。
    pub fn set_collapse(&mut self, on: bool, row: usize) -> usize {
        let anchor = match self.rows[RIGHT].get(row) {
            Some(SideRow::Line { line, .. }) => *line,
            Some(SideRow::Fold { lines, .. }) => lines.start,
            None => usize::MAX,
        };
        self.collapse = on;
        self.expanded.clear();
        self.pinned = None;
        self.layout();
        self.row_of_new(anchor)
            .min(self.row_count().saturating_sub(1))
    }

    /// 展开第 `gap` 段未改动内容的折叠。
    pub fn expand(&mut self, gap: usize) {
        if !self.expanded.contains(&gap) {
            self.expanded.push(gap);
            self.layout();
        }
    }

    /// 返回右侧显示行 `row` 之后（不含）下一段改动的起始显示行；之后没有改动时返回 `None`。
    pub fn next_hunk(&self, row: usize) -> Option<usize> {
        self.hunk_rows[RIGHT]
            .iter()
            .map(|r| r.start)
            .find(|&r| r > row)
    }

    /// 返回右侧显示行 `row` 之前（不含）上一段改动的起始显示行；之前没有改动时返回 `None`。
    pub fn previous_hunk(&self, row: usize) -> Option<usize> {
        self.hunk_rows[RIGHT]
            .iter()
            .rev()
            .map(|r| r.start)
            .find(|&r| r < row)
    }

    /// 返回滚动位置为 `scroll`（见 [`DiffView::row_at`]）时的当前改动序号。
    ///
    /// 跳转后尚未滚动时为跳转到的改动，否则取屏幕第四行所在或之前最近的改动。没有改动时返回
    /// `None`。
    pub fn current_hunk(&self, scroll: f64) -> Option<usize> {
        match self.focus {
            Some((hunk, at)) if at == scroll && hunk < self.hunks.len() => Some(hunk),
            _ => self.hunk_at_pos(scroll.max(0.0) + 3.0),
        }
    }

    /// 返回滚动位置 `pos` 所在或之前最近的改动序号（从 0 起）；之前没有改动时返回 0。
    pub fn hunk_at_pos(&self, pos: f64) -> Option<usize> {
        if self.hunks.is_empty() {
            return None;
        }
        // 分段点按每段改动的开头、结尾交替排列：不大于 `pos` 的分段点有 `n` 个时，其中开头有
        // `(n + 1) / 2` 个，最后一个开头所属的改动即为结果。
        let n = self.knots.partition_point(|k| k.0 <= pos.floor());
        Some(n.div_ceil(2).saturating_sub(1))
    }

    /// 返回第 `hunk` 段改动开头的滚动位置。
    pub fn hunk_pos(&self, hunk: usize) -> f64 {
        self.knots.get(hunk * 2).map_or(0.0, |&(pos, _)| pos)
    }

    /// 返回第 `hunk` 段改动占用的滚动位置范围。
    pub fn hunk_span(&self, hunk: usize) -> Range<f64> {
        let end = self.knots.get(hunk * 2 + 1).map_or(0.0, |&(pos, _)| pos);
        self.hunk_pos(hunk)..end
    }

    /// 返回右侧显示行 `row` 所在或之前最近的改动序号（从 0 起）；之前没有改动时返回 0。
    pub fn hunk_at(&self, row: usize) -> Option<usize> {
        if self.hunks.is_empty() {
            return None;
        }
        let i = self.hunk_rows[RIGHT].partition_point(|r| r.start <= row);
        Some(i.saturating_sub(1))
    }

    /// 返回两侧最长一行的显示宽度（显示列），用于限制横向滚动。
    ///
    /// 按 Tab 宽度缓存。
    pub fn widest(&self, tab_width: usize) -> usize {
        if let Some((tab, cells)) = self.widest.get()
            && tab == tab_width
        {
            return cells;
        }
        let cells = self.measure_widest(tab_width);
        self.widest.set(Some((tab_width, cells)));
        cells
    }

    fn measure_widest(&self, tab_width: usize) -> usize {
        let widest = |text: &Rope| {
            let mut widest = 0;
            let _ = text.visit_lines(0, text::MAX_LINE_BYTES, |_, _, content| {
                widest = widest.max(text::col_to_cell(content.into(), MAX_LINE_CHARS, tab_width));
                std::ops::ControlFlow::Continue(())
            });
            widest
        };
        widest(&self.old.text).max(widest(&self.new.text))
    }

    /// 把第 `side` 侧第 `line` 行的各段追加到 `rows`。
    fn push_line(&self, rows: &mut Vec<SideRow>, side: usize, line: usize, hunk: Option<usize>) {
        let parts = self.breaks(side, line).rows();
        rows.extend((0..parts).map(|part| SideRow::Line { line, hunk, part }));
    }

    /// 按改动和折叠状态重新排列两侧的显示行。
    fn layout(&mut self) {
        let mut rows = [Vec::new(), Vec::new()];
        let mut hunk_rows = [
            Vec::with_capacity(self.hunks.len()),
            Vec::with_capacity(self.hunks.len()),
        ];
        let (old_len, new_len) = (line_total(&self.old.text), line_total(&self.new.text));
        let (mut a, mut b) = (0, 0);

        for gap in 0..=self.hunks.len() {
            // 当前一段未改动的内容：从上一段改动之后到下一段改动之前，两侧行数相同。
            let (a_end, b_end) = match self.hunks.get(gap) {
                Some(h) => (h.old.start, h.new.start),
                None => (old_len, new_len),
            };
            let len = a_end - a;
            let first = gap == 0;
            let last = gap == self.hunks.len();
            // 文件开头只保留改动前的上下文，结尾只保留改动后的上下文。
            let head = if first { 0 } else { CONTEXT };
            let tail = if last { 0 } else { CONTEXT };
            let fold = self.collapse
                && !self.hunks.is_empty()
                && !self.expanded.contains(&gap)
                && len > head + tail + 1;
            let same = |rows: &mut [Vec<SideRow>; 2], range: Range<usize>| {
                for i in range {
                    self.push_line(&mut rows[LEFT], LEFT, a + i, None);
                    self.push_line(&mut rows[RIGHT], RIGHT, b + i, None);
                }
            };
            if fold {
                // 显示开头 `head` 行、结尾 `tail` 行，以及固定显示的行和其上下各 `CONTEXT` 行；
                // 其余每个区间折叠为一行。两个显示区间只相隔一行时直接显示该行，不单独折叠。
                let mut shown = vec![0..head, len - tail..len];
                if let Some(p) = &self.pinned
                    && p.start >= b
                    && p.start <= b_end
                {
                    let from = (p.start - b).saturating_sub(CONTEXT);
                    let to = (p.end.max(p.start) - b + CONTEXT).min(len);
                    shown.push(from..to);
                }
                shown.retain(|r| !r.is_empty());
                shown.sort_by_key(|r| r.start);
                let mut at = 0;
                for range in shown {
                    let start = range.start.max(at);
                    if start > at + 1 {
                        rows[LEFT].push(SideRow::Fold {
                            gap,
                            lines: a + at..a + start,
                        });
                        rows[RIGHT].push(SideRow::Fold {
                            gap,
                            lines: b + at..b + start,
                        });
                    } else {
                        same(&mut rows, at..start);
                    }
                    same(&mut rows, start..range.end.max(start));
                    at = at.max(range.end);
                }
                if at < len {
                    rows[LEFT].push(SideRow::Fold {
                        gap,
                        lines: a + at..a_end,
                    });
                    rows[RIGHT].push(SideRow::Fold {
                        gap,
                        lines: b + at..b_end,
                    });
                }
            } else {
                same(&mut rows, 0..len);
            }

            let Some(h) = self.hunks.get(gap) else { break };
            for (side, lines) in [(LEFT, &h.old), (RIGHT, &h.new)] {
                let start = rows[side].len();
                for line in lines.clone() {
                    self.push_line(&mut rows[side], side, line, Some(gap));
                }
                hunk_rows[side].push(start..rows[side].len());
            }
            (a, b) = (h.old.end, h.new.end);
        }
        let folds = |rows: &[SideRow]| -> Vec<usize> {
            (rows.iter().enumerate())
                .filter(|(_, row)| matches!(row, SideRow::Fold { .. }))
                .map(|(i, _)| i)
                .collect()
        };
        // 折叠行总是在两侧成对加入，按顺序一一对应。
        self.fold_rows = folds(&rows[LEFT])
            .into_iter()
            .zip(folds(&rows[RIGHT]))
            .collect();
        self.rows = rows;
        // 滚动位置：未改动的内容两侧逐行对应，每段改动占行数较多一侧的行数。
        let mut knots = Vec::with_capacity(2 * self.hunks.len());
        let (mut pos, mut prev) = (0usize, [0usize; 2]);
        for (l, r) in hunk_rows[LEFT].iter().zip(&hunk_rows[RIGHT]) {
            pos += r.start - prev[RIGHT];
            knots.push((pos as f64, [l.start as f64, r.start as f64]));
            // 并排时每段改动占行数较多一侧的行数，单栏时两侧的行上下排列。
            pos += if self.unified {
                l.len() + r.len()
            } else {
                l.len().max(r.len())
            };
            knots.push((pos as f64, [l.end as f64, r.end as f64]));
            prev = [l.end, r.end];
        }
        self.knots = knots;
        self.hunk_rows = hunk_rows;
    }
}

/// 返回 `a` 与 `b` 第一处不同所在的行号（`a` 中）；完全相同时返回 `a` 末尾所在的行号。
fn first_difference(a: &Rope, b: &Rope) -> usize {
    let mut chars = 0;
    let (mut x, mut y) = (a.chunks(), b.chunks());
    let (mut ca, mut cb) = (x.next().unwrap_or(""), y.next().unwrap_or(""));
    loop {
        if ca.is_empty() {
            match x.next() {
                Some(c) => ca = c,
                None => break,
            }
            continue;
        }
        if cb.is_empty() {
            match y.next() {
                Some(c) => cb = c,
                None => break,
            }
            continue;
        }
        let n = ca.len().min(cb.len());
        let same = ca.as_bytes()[..n]
            .iter()
            .zip(&cb.as_bytes()[..n])
            .take_while(|(p, q)| p == q)
            .count();
        // 回退到字符边界后再统计字符数。
        let mut same = same;
        while !ca.is_char_boundary(same) {
            same -= 1;
        }
        chars += ca[..same].chars().count();
        if same < n {
            break;
        }
        ca = &ca[n..];
        cb = &cb[n..];
    }
    a.char_to_line(chars.min(a.len_chars()))
}

/// 返回参与比较的行数；文本以换行符结尾时，末尾的空行不计入（差分不把该空行视为一行）。
fn line_total(text: &Rope) -> usize {
    let count = text.len_lines();
    let trailing = text.len_chars() > 0 && matches!(text.char(text.len_chars() - 1), '\n' | '\r');
    if trailing || text.len_chars() == 0 {
        count - 1
    } else {
        count
    }
}

/// 计算一段改动中两侧逐行的逐词标记；纯新增、纯删除或超过 `INLINE_LIMIT` 行时返回 `None`。
pub fn inline_marks(old: &Rope, new: &Rope, hunk: &LineHunk) -> Option<InlineMarks> {
    if hunk.old.is_empty()
        || hunk.new.is_empty()
        || hunk.old.len() > INLINE_LIMIT
        || hunk.new.len() > INLINE_LIMIT
    {
        return None;
    }
    // 两侧各拼接为一段，换行符同样参与比较，使跨行的改动也能对应。
    let joined = |text: &Rope, lines: &Range<usize>| -> (String, Vec<usize>) {
        let mut out = String::new();
        let mut starts = Vec::with_capacity(lines.len());
        let mut chars = 0;
        for line in lines.clone() {
            starts.push(chars);
            let content = text.line(line);
            chars += content.len_chars();
            out.extend(content.chunks());
        }
        (out, starts)
    };
    let (a, a_starts) = joined(old, &hunk.old);
    let (b, b_starts) = joined(new, &hunk.new);
    let (left, right) = diff::inline_changes(&a, &b);

    // 把整段中的字符区间拆分回各行，去掉落在换行符上的部分。
    let split = |ranges: &[Range<usize>], starts: &[usize], text: &Rope, lines: &Range<usize>| {
        let mut per_line = vec![Vec::new(); starts.len()];
        for (i, line) in lines.clone().enumerate() {
            let start = starts[i];
            let width = content_len(text.line(line));
            for r in ranges {
                let from = r.start.max(start);
                let to = r.end.min(start + width);
                if from < to {
                    per_line[i].push(from - start..to - start);
                }
            }
        }
        per_line
    };
    Some(InlineMarks {
        old: split(&left, &a_starts, old, &hunk.old),
        new: split(&right, &b_starts, new, &hunk.new),
    })
}

/// 返回一行去掉换行符后的字符数。
fn content_len(line: ropey::RopeSlice<'_>) -> usize {
    let mut len = line.len_chars();
    if len > 0 && line.char(len - 1) == '\n' {
        len -= 1;
    }
    if len > 0 && line.char(len - 1) == '\r' {
        len -= 1;
    }
    len
}

#[cfg(test)]
mod tests;
