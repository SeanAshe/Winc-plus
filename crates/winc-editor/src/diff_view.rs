//! 并排差异视图的数据：两侧的文本、各段改动、两侧各自的显示行、每段改动里具体改了哪些字。
//!
//! 两侧各排各的行，不插占位的空行：一段改动左侧占 `a` 行、右侧占 `b` 行，界面在两侧之间画一条带子
//! 把它们连起来。滚动位置是两侧共用的一个坐标（[`DiffView::row_at`]）：没改的内容一行对一行，每段
//! 改动按长的那一侧算。长段没改的内容默认折起来，只留改动前后各 [`CONTEXT`] 行。

use std::ops::Range;
use std::path::PathBuf;

use ropey::Rope;
use winc_core::Lines;
use winc_core::diff::{self, LineHunk};
use winc_core::text::{self, MAX_LINE_CHARS};
use winc_core::wrap::Breaks;
use winc_syntax::{Highlighter, Language};

use crate::editing::folding::line_breaks;

/// 改动前后各自保留的未改动行数（行）。
///
/// 取自调参：取值过小时，改动邻接的语句和块头被折起，看不出改动的上下文；取值过大时，长段未改动
/// 内容露出的行数随之增加，折叠省下的显示行变少。作用范围是每段改动两侧的未改动内容，只影响折叠
/// 后露出的行数，不参与两侧对齐的计算。
pub const CONTEXT: usize = 4;
/// 一段改动任一侧超过该行数（行，200）时不再按词比较。
///
/// 整段都已改动时，逐词标记不再有区分度，只标出整行即可，因此内容上的依据不构成约束；数值的大小
/// 取自调参：逐词比较的耗时与改动段的行数成正比，行数越多，重新比较所需的时间越长。
const INLINE_LIMIT: usize = 200;

/// 自动换行的行数上限（行，经验值）：整篇断行约每 20 万行 115 ms，再多打字时会明显卡顿。
pub const WRAP_LINES: usize = 200_000;

/// 差异的一侧。
#[derive(Debug)]
pub struct DiffSide {
    /// 标题，写在这一侧的上方（「HEAD」「暂存区」「工作区」「磁盘上的版本」……）。
    pub title: String,
    /// 这一侧的全部文本。
    pub text: Rope,
    /// 这一侧对应磁盘上的哪个文件；不是磁盘上的内容（某个提交、暂存区里的版本）时为 `None`。
    pub path: Option<PathBuf>,
    /// 这一侧对应的版本里没有该文件（还没加进来、已经删掉）：文本为空，标题已经写明，
    /// 界面不再写「空文件」。
    pub missing: bool,
    /// 语法着色的增量状态，绘制时推进。
    pub highlighter: Highlighter,
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
        }
    }

    /// 设置 `missing`：这一侧对应的文件在该版本中是否不存在（见 [`DiffSide::missing`]）。
    pub fn with_missing(mut self, missing: bool) -> DiffSide {
        self.missing = missing;
        self
    }
}

/// 一侧的一个显示行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideRow {
    /// 这一侧的一行。
    ///
    /// `hunk` 是这一行所在的改动（`DiffView::hunks` 的下标）；没改的行为 `None`。
    Line {
        /// 行号（从 0 起）。
        line: usize,
        /// 所在的改动。
        hunk: Option<usize>,
        /// 自动换行时这一行的第几段（从 0 起）；不换行时为 0。
        part: usize,
    },
    /// 折起来的一段没改的行：`gap` 是第几段没改的内容（第一段改动之前是 0），`lines` 是这一侧
    /// 折起来的行。
    ///
    /// 两侧同一段的行数相同。
    Fold {
        /// 第几段没改的内容。
        gap: usize,
        /// 这一侧折起来的行。
        lines: Range<usize>,
    },
}

/// 一段改动里两侧逐行的逐词标记：`old[i]` 是这段改动左侧第 `i` 行里改了的字符区间。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineMarks {
    /// 左侧每一行改了的字符区间（按字符，从行首起）。
    pub old: Vec<Vec<Range<usize>>>,
    /// 右侧每一行改了的字符区间。
    pub new: Vec<Vec<Range<usize>>>,
}

/// 左侧的下标（`old` 一侧）。
pub const LEFT: usize = 0;
/// 右侧的下标（`new` 一侧）。
pub const RIGHT: usize = 1;

/// 两侧比较的结果：各段改动、逐词标记、按块提交用的 key。
///
/// 只依赖两侧的文本，可以在后台算好再交给 [`DiffView::apply`]。
#[derive(Debug, Clone)]
pub struct Compared {
    hunks: Vec<LineHunk>,
    inline: Vec<Option<InlineMarks>>,
    keys: Vec<u64>,
}

impl Compared {
    /// 比较 `old` 和 `new`。
    ///
    /// 按块提交用的 key 直接从比较时切好的行算（与 `diff::pick_hunks` 的切法相同），不再把每段
    /// 改动的每一行另外复制成 `String`。
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

/// 单栏时滚动位置和各侧显示行之间的换算表。
///
/// 从差异里取出来单独持有：绘制时要同时修改差异两侧的着色状态。
#[derive(Debug, Clone)]
pub struct UnifiedMap(Vec<(f32, [f32; 2])>);

impl UnifiedMap {
    /// 第 `side` 侧显示行 `row` 在滚动位置上的哪里。
    ///
    /// 每段改动先是左侧的行，后面接右侧的行。这一侧在一段改动里没有行时，取这段改动的结尾。
    pub fn pos_of(&self, side: usize, row: f32) -> f32 {
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

/// 差异只读一侧的选区：只能选中、复制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadSelection {
    /// 在哪一侧：[`LEFT`] 或 [`RIGHT`]。
    pub side: usize,
    /// 起点和终点，按这一侧文本的字符偏移。
    pub range: (usize, usize),
}

/// 并排差异视图。
#[derive(Debug)]
pub struct DiffView {
    /// 左侧：改之前。
    pub old: DiffSide,
    /// 右侧：改之后。
    pub new: DiffSide,
    /// 各段改动，按位置排列。
    pub hunks: Vec<LineHunk>,
    /// 每段改动的逐词标记，下标同 `hunks`；纯新增、纯删除、太大的改动为 `None`。
    pub inline: Vec<Option<InlineMarks>>,
    /// 两侧各自的显示行：`rows[LEFT]`、`rows[RIGHT]`。
    pub rows: [Vec<SideRow>; 2],
    /// 每段改动在两侧占的显示行，下标同 `hunks`；这一侧没有行时是插入位置上的空区间。
    pub hunk_rows: [Vec<Range<usize>>; 2],
    /// 滚动位置的分段点：每段改动的开头和结尾各一个，（滚动位置，[左侧显示行，右侧显示行]），
    /// 见 [`DiffView::row_at`]。
    knots: Vec<(f32, [f32; 2])>,
    /// 是否折起长段没改的内容。
    collapse: bool,
    /// 单栏显示：没改的行只显示一次，每段改动先是左侧的行，后面接右侧的行。
    unified: bool,
    /// 自动换行的（断行宽度，Tab 宽度），单位是显示列；不换行时为 `None`。
    ///
    /// 两侧用同一个宽度：没改的行两侧断法相同、行数相等，滚动位置的换算才成立。
    wrap: Option<(usize, usize)>,
    /// 上一次跳到的是第几段改动，和跳过去以后的滚动位置。
    ///
    /// 滚动位置没变时，「上一处」「下一处」和「第几处」按它算：最后几段改动滚不到顶上（编辑区滚过
    /// 最后一行只留四成屏），按滚动位置算会停在前面那段；用户自己滚动过之后改按滚动位置算。
    pub focus: Option<(usize, f32)>,
    /// 已经点开的没改内容，元素是各段的下标。
    expanded: Vec<usize>,
    /// 右侧要一直露出来的几行；没改内容里只露出这几行，其余部分仍然折起。
    ///
    /// 用「»」换掉一段改动以后，换上去的几行没有改动了，不能立即被折进去，也不应因此把整段
    /// 都展开。
    pinned: Option<Range<usize>>,
    /// 算过的最长一行的显示宽度：（Tab 宽度，格数）。
    ///
    /// 内容不变，滚动时不必每次重新扫描全文。
    widest: std::cell::Cell<Option<(usize, usize)>>,
    /// 右侧取自编辑器里哪个版本的文档（[`crate::Document::revision`]）；右侧是磁盘内容时为
    /// `None`。
    ///
    /// 版本变了就用 [`DiffView::set_new_text`] 重新比较。
    pub source: Option<u64>,
    /// 光标在右侧：键入的字进右侧对应的文档。
    pub editing: bool,
    /// 只读一侧的选区：左侧总是只读，右侧不是工作区里的文件时也只读（历史版本、已删除的文件）。
    ///
    /// 没有时为 `None`。
    pub read_selection: Option<ReadSelection>,
    /// 每段改动的 [`diff::hunk_key`]，下标同 `hunks`：按块提交时按它认出勾掉的改动。
    pub keys: Vec<u64>,
    /// 这份差异对应仓库里的哪个文件（和仓库状态里的路径同一种写法）；不是 Git 的差异时为
    /// `None`。
    ///
    /// 按块提交时用它找到提交视图里的文件。
    pub repo_path: Option<PathBuf>,
}

impl DiffView {
    /// 比较 `old` 和 `new` 两侧，算出改动并排好两侧的行。
    ///
    /// 没改的长段默认折起来。
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
        };
        view.layout();
        view
    }

    /// 右侧换成 `text`（编辑器里改了），重新比较。
    ///
    /// 改动的段数没变时已经点开的折叠保持点开。
    pub fn set_new_text(&mut self, text: Rope) {
        let compared = Compared::new(&self.old.text, &text);
        self.apply(text, compared);
    }

    /// 把右侧换成 `text`，`compared` 是在后台对着同一份 `text` 算好的结果。
    pub fn apply(&mut self, text: Rope, compared: Compared) {
        let first = first_difference(&self.new.text, &text);
        self.new.highlighter.invalidate_from(first);
        // 钉住的几行跟着上面增删的行挪：改动从它们上面开始时整体挪，从中间开始时只挪下沿。
        // 按一处连续的修改估算（打字、粘贴、撤销都是），足以让钉住的几行停在原来的内容上。
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
        // 右侧换成了编辑器里的内容：文件是存在的，哪怕是空的。
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

    /// 沿用 `old` 的状态（重新比较以后换上来）：光标是否在右侧、是否折起，改动的段数没变时
    /// 已经点开的折叠保持点开。
    pub fn inherit(&mut self, old: &DiffView) {
        self.editing = old.editing;
        self.pinned.clone_from(&old.pinned);
        // 选区所在的一侧没变（例如 `HEAD` 没变）时选区仍然有效。
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
    /// 两侧合起来超过 [`WRAP_LINES`] 行时不换行：每次编辑都要重新断行。设置变了时返回 `true`。
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

    /// 第 `side` 侧第 `line` 行的分段；不换行时只有一段。
    pub fn breaks(&self, side: usize, line: usize) -> Breaks {
        let text = if side == LEFT {
            &self.old.text
        } else {
            &self.new.text
        };
        match self.wrap {
            Some((width, tab_width)) if line < text.len_lines() => {
                line_breaks(text, line, width, tab_width)
            }
            _ => Breaks::default(),
        }
    }

    /// 第 `side` 侧第 `line` 行第 `col` 个字符所在的显示行；折起来时是折起来的那一行。
    pub fn row_of_col(&self, side: usize, line: usize, col: usize) -> usize {
        let row = self.row_of(side, line);
        match self.rows[side].get(row) {
            Some(SideRow::Line { line: at, .. }) if *at == line => {
                row + self.breaks(side, line).row_of(col)
            }
            _ => row,
        }
    }

    /// 把第 `side` 侧显示行 `row` 里第 `cell` 个显示列换算成这一整行的显示列。
    ///
    /// 超出这一段时停在这一段的最后一个字符之前，不落到下一段。
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

    /// 右侧的显示行数。
    pub fn row_count(&self) -> usize {
        self.rows[RIGHT].len()
    }

    /// 能滚动的行数（滚动位置的总长，见 [`DiffView::row_at`]）：每段改动按长的那一侧算，
    /// 不只按右侧。
    pub fn scroll_rows(&self) -> usize {
        self.knots
            .last()
            .map_or(self.rows[RIGHT].len(), |&(pos, rows)| {
                (pos + (self.rows[RIGHT].len() as f32 - rows[RIGHT])).ceil() as usize
            })
    }

    /// 滚动位置 `pos` 上第 `side` 侧对着的显示行（可以有小数）。
    ///
    /// 滚动位置是两侧共用的一把尺：没改的内容两侧一行对一行，每段改动占两侧里长的那一侧的
    /// 行数，短的一侧按比例跟着走（一侧是 0 行时停在原地）。只按右侧的行数滚时，左侧删掉的一大段
    /// 在右侧只是一个点，一滚就跳过去，比半屏长的部分永远看不到。
    pub fn row_at(&self, side: usize, pos: f32) -> f32 {
        if self.unified {
            return self.unified_row_at(side, pos);
        }
        // 分段点按滚动位置排好了：二分找到第一个不在 `pos` 之前的，在它和前一个之间插值。
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

    /// 第 `i` 个分段点之前的那一个（滚动位置，第 `side` 侧的显示行）；`i` 是 0 时是开头。
    fn knot(&self, side: usize, i: usize) -> (f32, f32) {
        i.checked_sub(1)
            .and_then(|i| self.knots.get(i))
            .map_or((0.0, 0.0), |&(at, rows)| (at, rows[side]))
    }

    /// 第 `side` 侧显示行 `row` 在滚动位置上的哪里（[`DiffView::row_at`] 反过来）。
    ///
    /// 这一侧在一段改动里停着不动（0 行）时取这段改动的开头。
    pub fn pos_of(&self, side: usize, row: f32) -> f32 {
        if self.unified {
            return self.unified_map().pos_of(side, row);
        }
        // 每一侧的显示行也随分段点递增：二分找到第一个不在 `row` 之前的。
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
    /// 两侧在同一个对齐点上对齐：滚过半屏以后，对齐点位于视图中间；离开头不到半屏时，对齐点随之
    /// 向上移动，在开头时位于顶边，两侧一起停在第一行。
    pub fn side_scroll(&self, side: usize, pos: f32, visible: f32) -> f32 {
        // 单栏只有一列，不需要对齐两侧。
        if self.unified {
            return self.row_at(side, pos);
        }
        let anchor = pos.min(visible / 2.0).max(0.0);
        let center = self.row_at(side, pos + anchor);
        (center - anchor).clamp(0.0, self.rows[side].len() as f32)
    }

    /// 第 `side` 侧最上面显示到 `row` 时的滚动位置（[`DiffView::side_scroll`] 反过来）。
    ///
    /// 离开头不到半屏时对齐点随滚动移动，没有直接的反算公式，改用二分查找。
    pub fn pos_of_top(&self, side: usize, row: f32, visible: f32) -> f32 {
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

    /// 切换单栏显示和并排显示。
    ///
    /// 滚动位置的换算随之改变，调用方负责换算滚动位置。
    pub fn set_unified(&mut self, on: bool) {
        if self.unified != on {
            self.unified = on;
            self.layout();
        }
    }

    /// 单栏时滚动位置和各侧显示行之间的换算表。
    pub fn unified_map(&self) -> UnifiedMap {
        UnifiedMap(self.knots.clone())
    }

    /// 单栏第 `index` 个显示行（从 0 起）属于哪一侧的哪个显示行：（哪一侧，显示行）。
    ///
    /// 没改的行和折起来的一段算右侧。超出最后一行时为 `None`。
    pub fn unified_row(&self, index: usize) -> Option<(usize, usize)> {
        let pos = index as f32;
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

    /// 单栏时滚动位置 `pos` 上第 `side` 侧对着的显示行：在另一侧的行上时是这一侧接下来的行。
    fn unified_row_at(&self, side: usize, pos: f32) -> f32 {
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

    /// 滚动位置为 `pos`、可见 `visible` 行时，从顶边往下第 `below` 行（可以有小数）对着的第
    /// `side` 侧的显示行。
    pub fn row_under(&self, side: usize, pos: f32, visible: f32, below: f32) -> usize {
        let row = if self.unified {
            self.row_at(side, (pos + below).floor())
        } else {
            self.side_scroll(side, pos, visible) + below
        };
        row.floor().max(0.0) as usize
    }

    /// 计算滚动位置为 `pos`、可见 `visible` 行时左侧的滚动位置，换算方式同
    /// [`DiffView::side_scroll`]。
    pub fn left_scroll(&self, pos: f32, visible: f32) -> f32 {
        self.side_scroll(LEFT, pos, visible)
    }

    /// 第 `side` 侧第 `line` 行所在的显示行；那一行折起来了时是折起来的那一行。
    ///
    /// 文件以换行符结尾时末尾的空行不参与比较、没有自己的显示行，算作最后一个显示行之后。
    pub fn row_of(&self, side: usize, line: usize) -> usize {
        // 显示行按行号排好了，二分查找：编辑时每一帧都要按光标所在的行找一次。
        self.rows[side].partition_point(|row| match row {
            SideRow::Line { line: l, .. } => *l < line,
            SideRow::Fold { lines, .. } => lines.end <= line,
        })
    }

    /// 返回右侧第 `line` 行所在的显示行，查找方式同 [`DiffView::row_of`]。
    pub fn row_of_new(&self, line: usize) -> usize {
        self.row_of(RIGHT, line)
    }

    /// 第 `side` 侧显示行 `row` 的行号；是折起来的行、或者超出了时为 `None`。
    pub fn line_at(&self, side: usize, row: usize) -> Option<usize> {
        match self.rows[side].get(row)? {
            SideRow::Line { line, .. } => Some(*line),
            SideRow::Fold { .. } => None,
        }
    }

    /// 让右侧 `lines` 这几行（连同上下各 [`CONTEXT`] 行）一直露出来，所在那段没改的内容其余
    /// 部分仍然折起。
    ///
    /// `lines` 为空时露出该位置上下的几行。
    pub fn pin_new(&mut self, lines: Range<usize>) {
        self.pinned = Some(lines);
        self.layout();
    }

    /// 展开右侧第 `line` 行所在的折叠；该行没有折起来时不做改动。
    pub fn reveal_new(&mut self, line: usize) {
        let row = self.row_of_new(line);
        if let Some(SideRow::Fold { gap, .. }) = self.rows[RIGHT].get(row) {
            let gap = *gap;
            self.expand(gap);
        }
    }

    /// 第 `side` 侧的内容。
    pub fn side(&self, side: usize) -> &DiffSide {
        if side == LEFT { &self.old } else { &self.new }
    }

    /// 只读一侧的光标：（哪一侧，字符偏移），在选区的终点。
    ///
    /// 没有选区、这一侧没有内容（文件不存在、已删除）时为 `None`。
    pub fn read_caret(&self) -> Option<(usize, usize)> {
        let selection = self.read_selection?;
        let len = self.side(selection.side).text.len_chars();
        (len > 0).then(|| (selection.side, selection.range.1.min(len)))
    }

    /// 只读一侧选中的文本；没有选中内容时为 `None`。
    pub fn read_selected(&self) -> Option<String> {
        let selection = self.read_selection?;
        let text = &self.side(selection.side).text;
        let (a, b) = selection.range;
        let (from, to) = (a.min(b), a.max(b).min(text.len_chars()));
        (from < to).then(|| text.slice(from..to).to_string())
    }

    /// 第 `hunk` 段改动左侧的文本（含换行符），用来把右侧的这一段换回去。
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

    /// 一侧整个是空的：文件不在 `HEAD` 里（新文件）、在工作区里删掉了，或者该版本本来就是
    /// 空文件。
    ///
    /// 这时整个文件是一处改动。
    pub fn one_side_empty(&self) -> bool {
        self.old.text.len_chars() == 0 || self.new.text.len_chars() == 0
    }

    /// 两侧内容是否完全相同。
    pub fn is_identical(&self) -> bool {
        self.hunks.is_empty()
    }

    /// 新增、删除的行数（右侧多出的行，左侧去掉的行）。
    pub fn stats(&self) -> (usize, usize) {
        self.hunks.iter().fold((0, 0), |(add, del), h| {
            (add + h.new.len(), del + h.old.len())
        })
    }

    /// 长段没改的内容是否折起来。
    pub fn collapses(&self) -> bool {
        self.collapse
    }

    /// 切换是否折起长段没改的内容，返回切换后右侧显示行 `row` 那一行所在的显示行，使视图停在
    /// 原处。
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

    /// 展开第 `gap` 段折起来的内容。
    pub fn expand(&mut self, gap: usize) {
        if !self.expanded.contains(&gap) {
            self.expanded.push(gap);
            self.layout();
        }
    }

    /// 右侧显示行 `row` 之后（不含）下一段改动的位置；后面没有改动时返回 `None`。
    pub fn next_hunk(&self, row: usize) -> Option<usize> {
        self.hunk_rows[RIGHT]
            .iter()
            .map(|r| r.start)
            .find(|&r| r > row)
    }

    /// 右侧显示行 `row` 之前（不含）上一段改动的位置；前面没有改动时返回 `None`。
    pub fn previous_hunk(&self, row: usize) -> Option<usize> {
        self.hunk_rows[RIGHT]
            .iter()
            .rev()
            .map(|r| r.start)
            .find(|&r| r < row)
    }

    /// 滚动到 `scroll`（滚动位置，见 [`DiffView::row_at`]）时「现在在第几段改动」：刚跳过去、
    /// 还没滚动过时是跳到的那一段，否则按屏幕上第四行所在或之前最近的那一段算。
    ///
    /// 没有改动时为 `None`。
    pub fn current_hunk(&self, scroll: f32) -> Option<usize> {
        match self.focus {
            Some((hunk, at)) if at == scroll && hunk < self.hunks.len() => Some(hunk),
            _ => self.hunk_at_pos(scroll.max(0.0) + 3.0),
        }
    }

    /// 滚动位置 `pos` 所在或之前最近的改动是第几段（从 0 起）；前面没有改动时是第一段。
    pub fn hunk_at_pos(&self, pos: f32) -> Option<usize> {
        if self.hunks.is_empty() {
            return None;
        }
        // 分段点是每段改动的开头、结尾交替排着的：不在 `pos` 之后的有 `n` 个时，其中开头有
        // `(n + 1) / 2` 个，最后一个就是要找的那段。
        let n = self.knots.partition_point(|k| k.0 <= pos.floor());
        Some(n.div_ceil(2).saturating_sub(1))
    }

    /// 第 `hunk` 段改动的开头在滚动位置上的哪里。
    pub fn hunk_pos(&self, hunk: usize) -> f32 {
        self.knots.get(hunk * 2).map_or(0.0, |&(pos, _)| pos)
    }

    /// 第 `hunk` 段改动在滚动位置上占的范围。
    pub fn hunk_span(&self, hunk: usize) -> Range<f32> {
        let end = self.knots.get(hunk * 2 + 1).map_or(0.0, |&(pos, _)| pos);
        self.hunk_pos(hunk)..end
    }

    /// 右侧显示行 `row` 所在或之前最近的改动是第几段（从 0 起）；前面没有改动时是第一段。
    pub fn hunk_at(&self, row: usize) -> Option<usize> {
        if self.hunks.is_empty() {
            return None;
        }
        let i = self.hunk_rows[RIGHT].partition_point(|r| r.start <= row);
        Some(i.saturating_sub(1))
    }

    /// 两侧最长一行的显示宽度（格），用于限制横向滚动。
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

    /// 按改动和折叠状态重新排出两侧的显示行。
    fn layout(&mut self) {
        let mut rows = [Vec::new(), Vec::new()];
        let mut hunk_rows = [
            Vec::with_capacity(self.hunks.len()),
            Vec::with_capacity(self.hunks.len()),
        ];
        let (old_len, new_len) = (line_total(&self.old.text), line_total(&self.new.text));
        let (mut a, mut b) = (0, 0);

        for gap in 0..=self.hunks.len() {
            // 这一段没改的内容：上一段改动之后到下一段改动之前，两侧行数相同。
            let (a_end, b_end) = match self.hunks.get(gap) {
                Some(h) => (h.old.start, h.new.start),
                None => (old_len, new_len),
            };
            let len = a_end - a;
            let first = gap == 0;
            let last = gap == self.hunks.len();
            // 文件开头只留改动前的上下文，结尾只留改动后的上下文。
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
                // 露出来的：开头 `head` 行、结尾 `tail` 行，钉住的几行和它们上下各 `CONTEXT`
                // 行；其余每一截各折成一行。只隔一行的两截连起来，不为一行折一次。
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
        self.rows = rows;
        // 滚动位置：没改的两侧一行对一行，每段改动占长的那一侧的行数。
        let mut knots = Vec::with_capacity(2 * self.hunks.len());
        let (mut pos, mut prev) = (0usize, [0usize; 2]);
        for (l, r) in hunk_rows[LEFT].iter().zip(&hunk_rows[RIGHT]) {
            pos += r.start - prev[RIGHT];
            knots.push((pos as f32, [l.start as f32, r.start as f32]));
            // 并排时每段改动占长的那一侧的行数，单栏时两侧的行上下排开。
            pos += if self.unified {
                l.len() + r.len()
            } else {
                l.len().max(r.len())
            };
            knots.push((pos as f32, [l.end as f32, r.end as f32]));
            prev = [l.end, r.end];
        }
        self.knots = knots;
        self.hunk_rows = hunk_rows;
    }
}

/// `a` 和 `b` 从第几行开始不同；完全相同时是最后一行之后。
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
        // 退到字符边界上再数字符。
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

/// 参与比较的行数：文本以换行符结尾时，末尾的空行不算（它在差分里不是一行）。
fn line_total(text: &Rope) -> usize {
    let count = text.len_lines();
    let trailing = text.len_chars() > 0 && matches!(text.char(text.len_chars() - 1), '\n' | '\r');
    if trailing || text.len_chars() == 0 {
        count - 1
    } else {
        count
    }
}

/// 一段改动里两侧逐行的逐词标记；纯新增、纯删除或者太大时返回 `None`。
pub fn inline_marks(old: &Rope, new: &Rope, hunk: &LineHunk) -> Option<InlineMarks> {
    if hunk.old.is_empty()
        || hunk.new.is_empty()
        || hunk.old.len() > INLINE_LIMIT
        || hunk.new.len() > INLINE_LIMIT
    {
        return None;
    }
    // 两侧各拼成一段，换行符也参与比较，这样跨行的改动也能对上。
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

    // 把整段里的字符区间拆回到各行，去掉落在换行符上的部分。
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

/// 一行去掉换行符以后的字符数。
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
