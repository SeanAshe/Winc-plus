//! 一个终端标签在主程序这边的状态：可见的一屏、看回滚时缓存的历史行、选区、查找的匹配。
//!
//! 行用绝对行号（[`Point::line`]）指：回滚缓冲里的行就是它在辅助进程里的行号，屏幕上第 `r` 行是
//! `history.1 + r`；屏幕滚出一行时那一行进回滚缓冲、行号不变，选区和查找不会因新输出错位。重排、
//! 清屏以后行号全部重来（帧的 `full`），选区和缓存一起清掉；网格上的行按「显示行」数，折起的命令块
//! 只占一行（[`ViewRow`]），`scroll` 是往上翻的显示行数。

use std::collections::BTreeMap;
use std::time::Instant;

use winc_pty::protocol::{Attrs, Block, Cursor, Frame, Line, Modes, Size};
use winc_pty::width::cells;

/// 看回滚时在可见范围上下各多缓存几屏的历史行（屏，经验值）：向上滚动时先有一段已缓存的
/// 行，不必每行都等辅助进程回答。
const CACHE_SCREENS: usize = 2;

/// 网格上的一个位置：绝对行号和第几格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Point {
    /// 绝对行号：辅助进程给这一行编的号，从会话开始算起。
    pub line: u64,
    /// 这一行里的第几格（显示格，从 0 起）。
    pub col: usize,
}

/// 选区按什么单位扩大：单击拖动按字，双击按词，三击按行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    /// 按字。
    Char,
    /// 按词（路径里的 `/`、`:` 也算词的一部分）。
    Word,
    /// 按整行。
    Line,
}

/// 选区。
///
/// `anchor` 是按下的地方，`head` 跟着鼠标走；画和复制时按 [`Unit`] 扩到整词、整行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selection {
    /// 按下的位置。
    pub anchor: Point,
    /// 跟着鼠标走的那一端。
    pub head: Point,
    /// 扩大的单位。
    pub unit: Unit,
}

/// 网格上的一行显示什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewRow {
    /// 这个绝对行号的那一行。
    Line(u64),
    /// 第几个块（[`TermTab::blocks`] 里的下标）折起来的输出，藏了几行。
    Fold(usize, u64),
}

impl ViewRow {
    /// 这一行对应的绝对行号；折起的一段取它的第一行。
    pub fn line(self, tab: &TermTab) -> u64 {
        match self {
            ViewRow::Line(line) => line,
            // 折起的那一段可能开头已经滚出了回滚缓冲：取还在的第一行。
            ViewRow::Fold(block, _) => tab.blocks[block]
                .output
                .unwrap_or_default()
                .max(tab.history.0),
        }
    }
}

/// 一段折起来的行：`start..end`（绝对行号），属于第 `block` 个块。
#[derive(Debug, Clone, Copy)]
struct Fold {
    /// 折起的第一行（绝对行号）。
    start: u64,
    /// 折起部分之后的第一行（绝对行号，不含在折起的范围里）。
    end: u64,
    /// 块在 [`TermTab::blocks`] 里的下标。
    block: usize,
}

/// 显示行的排法：[`TermTab::layout`]。
struct Layout {
    /// 折起的各段，按位置排好。
    folds: Vec<Fold>,
    /// 回滚缓冲第一行的行号（第 0 个显示行）。
    first: u64,
    /// 一共有几个显示行。
    len: u64,
    /// 看最新输出时网格最后一行的下一个显示行：一般是 `len`，见 [`TermTab::preview_end`]。
    end: u64,
}

impl Layout {
    /// 网格第一行是第几个显示行。
    fn top(&self, tab: &TermTab) -> u64 {
        self.end
            .saturating_sub(tab.grid_rows() as u64 + tab.scroll as u64)
    }

    fn display_of(&self, line: u64) -> u64 {
        let line = line.max(self.first);
        let mut d = line - self.first;
        for fold in &self.folds {
            if fold.start >= line {
                break;
            }
            if line < fold.end {
                d -= line - fold.start;
                break;
            }
            d -= fold.end - fold.start - 1;
        }
        d
    }

    fn row_at(&self, d: u64) -> Option<ViewRow> {
        if d >= self.len {
            return None;
        }
        let mut line = self.first + d;
        for fold in &self.folds {
            if fold.start == line {
                return Some(ViewRow::Fold(fold.block, fold.end - fold.start));
            }
            if fold.start > line {
                break;
            }
            line += fold.end - fold.start - 1;
        }
        Some(ViewRow::Line(line))
    }
}

/// 终端不能再用的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TabEnd {
    /// shell 退出了：退出码。
    Exited(i32),
    /// 伪终端断开了，其中的进程可能还在运行。
    Disconnected,
}

/// 一个终端标签。
pub(crate) struct TermTab {
    /// 标签编号，辅助进程用它区分各标签；面板内唯一。
    pub id: u16,
    /// 屏幕上的行。
    pub rows: Vec<Line>,
    /// 光标在屏幕上的位置。
    pub cursor: Cursor,
    /// 程序当前开着的终端模式。
    pub modes: Modes,
    /// Kitty 键盘协议的标志位。
    pub keyboard: u8,
    /// 用户起的名字（双击标签重命名），没起时为空。
    pub name: String,
    /// 程序设的标题（OSC 0、2）。
    pub title: String,
    /// 前台进程的名字。
    pub process: String,
    /// 这个终端不能再用了：为什么；还能用时为 `None`。
    pub ended: Option<TabEnd>,
    /// 从菜单选的 shell 的启动命令；`None` 是默认 shell。
    pub shell: Option<String>,
    /// 标签运行的配置；普通的 shell 标签为 `None`。
    pub run: Option<winc_config::RunConfig>,
    /// 程序退出后重新运行配置。
    pub rerun: bool,
    /// 程序是用户停止的，不是自己退出的。
    pub stopped: bool,
    /// 最近一次告诉辅助进程的尺寸。
    pub size: Size,
    /// 收到了一帧还没给额度。
    pub credit_due: bool,
    /// 回滚缓冲里的行号范围（帧里报的）。
    pub history: (u64, u64),
    /// 往上翻了几行；0 表示看着最新的输出。
    pub scroll: usize,
    /// 在 `scroll` 之外继续往上翻的不足一行的部分（行，`0.0..1.0`），滚轮和触控板按像素滚动时使用。
    pub scroll_frac: f32,
    /// 全屏程序要了鼠标、备用屏改发方向键时，攒下的不到一行的滚动量（行）。
    pub scroll_rest: f32,
    /// 面板放不下一整行的零头（行，`0.0..1.0`），见 [`TermTab::viewport`]。
    pub slack: f32,
    /// 缓存的历史行，按行号。
    cache: BTreeMap<u64, Line>,
    /// 已经向辅助进程要了、还没回来的范围，避免每帧重复要。
    pub requested: Option<(u64, u64)>,
    /// 当前的选区；没有选区时为 `None`。
    pub selection: Option<Selection>,
    /// 命令块，按位置从上到下（辅助进程发来的，见 [`Block`]）。
    pub blocks: Vec<Block>,
    /// 按还没到的那一帧的行号算的块（`Blocks` 的 `early`）：那一帧到了换上。
    pub early_blocks: Option<Vec<Block>>,
    /// 折起输出的块的编号。
    pub folded: Vec<u32>,
    /// 不在看这个标签时有命令结束了，标签上亮一个点：最后结束的那条失败了没有。
    pub unseen: Option<bool>,
    /// 面板上放得下几行（0 表示和屏幕一样）。
    ///
    /// 新的尺寸告诉了辅助进程、重排好的屏幕还没回来时，
    /// 屏幕还是原来的行数，按辅助进程重排的规则先排出来（见 [`TermTab::view_top`]），回来以后
    /// 不跳。
    pub shown: usize,
    /// 发出了新的尺寸（`size`），还在等那个尺寸的一帧：什么时候发的。
    ///
    /// 同时只发一个，拖着改大小时
    /// 不会积压一串重排。
    pub resizing: Option<Instant>,
    /// 正拖着分隔条：网格上面一屏的历史行也先取来，拖高时露出来的行已经在了，不会先出现空白。
    pub ahead: bool,
    /// 吸顶的命令那一行最近一次取到的内容（块的编号，行）。
    ///
    /// 改宽度重排以后缓存丢了、重新取回来
    /// 之前先画它，不会先变成一行灰字再变回来。
    kept: Option<(u32, Line)>,
    /// ⌘↑ ⌘↓ 刚跳到的命令（块的编号）。
    ///
    /// 敲字以后清掉。
    pub jump: Option<u32>,
    /// 超链接编号对应的地址（OSC 8）。
    links: Vec<String>,
    /// shell 最近报的当前目录（OSC 7），重新启动时从这里开始；没报过时为空。
    pub cwd: Vec<u8>,
    /// 光标刚换到一个空行的行首（回车、⌃C 以后）：什么时候。
    ///
    /// 提示符还没画出来，这时候不画光标，
    /// 避免它先在行首出现、再跳到提示符后面；行上有了字或者等过了 [`SETTLE`] 才画。
    pub settle: Option<Instant>,
}

/// 光标换到空行行首以后最多藏多久（250 ms，经验值；见 [`TermTab::settle`]）。
///
/// 慢的提示符（要查 `git` 状态的）超过它就先在行首画出来。
pub const SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

impl TermTab {
    /// 创建一个空的标签：屏幕为 `size` 行，没有内容，也没有命令块。
    pub fn new(id: u16, size: Size) -> TermTab {
        TermTab {
            id,
            rows: vec![Line::default(); usize::from(size.rows)],
            cursor: Cursor {
                visible: true,
                ..Cursor::default()
            },
            modes: Modes::default(),
            keyboard: 0,
            name: String::new(),
            title: String::new(),
            process: String::new(),
            ended: None,
            shell: None,
            run: None,
            rerun: false,
            stopped: false,
            size,
            credit_due: false,
            history: (0, 0),
            scroll: 0,
            scroll_frac: 0.0,
            scroll_rest: 0.0,
            slack: 0.0,
            cache: BTreeMap::new(),
            requested: None,
            selection: None,
            blocks: Vec::new(),
            early_blocks: None,
            folded: Vec::new(),
            unseen: None,
            shown: 0,
            resizing: None,
            ahead: false,
            kept: None,
            jump: None,
            links: Vec::new(),
            cwd: Vec::new(),
            settle: None,
        }
    }

    /// 应用一帧。
    ///
    /// 往上翻着的时候新输出进了回滚缓冲，翻的行数跟着加，看着的内容不动。
    pub fn apply(&mut self, frame: Frame) {
        let rows = usize::from(frame.rows);
        if frame.full || self.rows.len() != rows {
            self.rows = vec![Line::default(); rows];
        }
        if frame.full {
            self.cache.clear();
            self.requested = None;
            self.selection = None;
            self.scroll = 0;
            self.scroll_frac = 0.0;
        } else if frame.modes.has(Modes::ALT_SCREEN) != self.modes.has(Modes::ALT_SCREEN) {
            // 换了一屏：往上翻的位置、选区都是另一屏的，回到底部。
            self.selection = None;
            self.scroll = 0;
            self.scroll_frac = 0.0;
            self.scroll_rest = 0.0;
        } else if self.scroll > 0 || self.scroll_frac > 0.0 {
            let grown = frame.history.1.saturating_sub(self.history.1);
            self.scroll += grown as usize;
        }
        self.rows.rotate_left(usize::from(frame.scroll).min(rows));
        for (row, line) in frame.lines {
            if let Some(slot) = self.rows.get_mut(usize::from(row)) {
                *slot = line;
            }
        }
        let moved = (frame.cursor.row, frame.cursor.col) != (self.cursor.row, self.cursor.col);
        self.cursor = frame.cursor;
        // 全屏程序（例如在空行上移动光标）是有意把光标留在该位置，不隐藏。
        let blank = self.cursor.col == 0
            && !frame.modes.has(Modes::ALT_SCREEN)
            && self
                .rows
                .get(usize::from(self.cursor.row))
                .is_none_or(|line| line.text.trim().is_empty());
        self.settle = match self.settle {
            _ if !blank || frame.full => None,
            Some(at) => Some(at),
            None => moved.then(Instant::now),
        };
        self.modes = frame.modes;
        self.keyboard = frame.keyboard;
        self.history = frame.history;
        self.clamp_scroll();
        self.prune();
        if !frame.full {
            self.keep_sticky(false);
        }
    }

    /// 记下吸顶的命令那一行（见 `kept`）。
    ///
    /// `again` 为假时同一个块记过了就不再记（每一帧都会走
    /// 这里，命令行本身很少变）。
    fn keep_sticky(&mut self, again: bool) {
        let Some(block) = self.sticky().map(|i| &self.blocks[i]) else {
            return;
        };
        if !again && self.kept.as_ref().is_some_and(|(id, _)| *id == block.id) {
            return;
        }
        if let Some(line) = self.line(block.command) {
            self.kept = Some((block.id, line.clone()));
        }
    }

    /// 吸顶的块（`blocks` 里第几个）的命令那一行：缓存里有的，没有时最近一次取到的。
    ///
    /// # Panics
    ///
    /// `index` 超出 `blocks` 的范围时 panic。
    pub fn sticky_line(&self, index: usize) -> Option<&Line> {
        let block = &self.blocks[index];
        self.line(block.command).or_else(|| {
            self.kept
                .as_ref()
                .filter(|(id, _)| *id == block.id)
                .map(|(_, line)| line)
        })
    }

    /// 超链接编号对应的地址（OSC 8）。
    ///
    /// 辅助进程从 1 起按顺序编号，用列表存。
    pub fn set_link(&mut self, id: u16, uri: String) {
        let index = usize::from(id).saturating_sub(1);
        if self.links.len() <= index {
            self.links.resize(index + 1, String::new());
        }
        self.links[index] = uri;
    }

    /// 超链接编号 `id` 对应的地址（OSC 8）；该编号没有地址时返回 `None`。
    pub fn link(&self, id: u16) -> Option<&String> {
        let uri = self.links.get(usize::from(id).checked_sub(1)?)?;
        (!uri.is_empty()).then_some(uri)
    }

    /// 里面有程序在运行时返回它的名字（关标签前要确认）：shell 报了正在运行的命令，或者前台进程
    /// 不是 shell 本身。
    pub fn busy(&self) -> Option<&str> {
        const SHELLS: &[&str] = &[
            "zsh", "bash", "fish", "sh", "dash", "ksh", "tcsh", "csh", "nu",
        ];
        // 伪终端断开时程序可能还在运行，关掉标签会结束它们，仍然要确认。
        if matches!(self.ended, Some(TabEnd::Exited(_))) {
            return None;
        }
        if let Some(run) = &self.run {
            return Some(&run.name);
        }
        if let Some(block) = self.blocks.last().filter(|b| b.running()) {
            let text = block.text.trim();
            return Some(if text.is_empty() { &self.process } else { text });
        }
        let process = self.process.trim_start_matches('-');
        (!process.is_empty() && !SHELLS.contains(&process)).then_some(process)
    }

    /// 标签上显示的名字：用户起的名字、程序设的标题、前台进程名，都没有时是「终端」。
    pub fn label(&self) -> &str {
        if !self.name.is_empty() {
            &self.name
        } else if !self.title.is_empty() {
            &self.title
        } else if !self.process.is_empty() {
            &self.process
        } else {
            winc_i18n::t!("terminal-tab-default-name")
        }
    }

    /// 网格上显示几行。
    pub fn grid_rows(&self) -> usize {
        if self.shown == 0 {
            self.rows.len()
        } else {
            self.shown
        }
    }

    /// 网格第一行的行号（折起的一段取它的第一行）。
    pub fn top(&self) -> u64 {
        self.row_at(self.view_top())
            .map_or(self.history.1, |row| row.line(self))
    }

    /// 绝对行号指的那一行：屏幕上的，或者缓存了的历史行；没缓存时 `None`。
    pub fn line(&self, line: u64) -> Option<&Line> {
        if line >= self.history.1 {
            self.rows.get((line - self.history.1) as usize)
        } else {
            self.cache.get(&line)
        }
    }

    /// 最大能往上翻几个显示行。
    ///
    /// 备用屏上不能翻：辅助进程报的回滚缓冲是主屏的，翻上去会把之前
    /// 的 shell 输出接在全屏程序的画面上面。
    pub fn max_scroll(&self) -> usize {
        if self.modes.has(Modes::ALT_SCREEN) {
            return 0;
        }
        self.layout().end.saturating_sub(self.grid_rows() as u64) as usize
    }

    /// 把往上翻的显示行数限制在有效范围内。
    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        self.scroll = self.scroll.min(max);
        if self.scroll == max {
            self.scroll_frac = 0.0;
        }
    }

    /// 连续的滚动位置：（距最新输出的行数，最大值）。
    ///
    /// 最大值扣除零头：第 0 个显示行上沿到达标签栏下沿时不能再往上翻，此时与贴着标签栏排列的位置
    /// 重合，翻到顶和离开顶部时内容不跳。
    pub fn scroll_position(&self) -> (f64, f64) {
        let max = self.max_scroll();
        let limit = (max as f64 - f64::from(self.slack)).max(0.0);
        let position = if self.scroll >= max {
            limit
        } else {
            self.scroll as f64 + f64::from(self.scroll_frac)
        };
        (position, limit)
    }

    /// 翻到距最新输出 `position` 行的位置（超出范围时夹到两端）。
    pub fn set_scroll_position(&mut self, position: f64) {
        let max = self.max_scroll();
        let (_, limit) = self.scroll_position();
        let position = position.clamp(0.0, limit);
        if position >= limit {
            self.scroll = max;
            self.scroll_frac = 0.0;
        } else {
            self.scroll = position.floor() as usize;
            self.scroll_frac = (position - position.floor()) as f32;
        }
    }

    /// 视口最上面露出的显示行和它被裁掉的比例（行，`0.0..1.0`）。
    ///
    /// 网格上面还有内容时，零头和 `scroll_frac` 都使内容相对标签栏下沿往下移动；所有绘制和命中都
    /// 由此换算，滚动和拖动分隔条时每一行的位置都连续。第 0 个显示行在最上面时贴着标签栏；备用屏
    /// 同样贴着标签栏，零头留在下面。
    pub fn viewport(&self) -> (u64, f32) {
        let top = self.view_top();
        if top == 0 || self.modes.has(Modes::ALT_SCREEN) {
            return (top, 0.0);
        }
        let shift = self.scroll_frac + self.slack;
        if shift <= 0.0 {
            return (top, 0.0);
        }
        let above = shift.ceil();
        match top.checked_sub(above as u64) {
            Some(first) => (first, above - shift),
            None => (0, 0.0),
        }
    }

    /// 存下辅助进程送来的历史行。
    pub fn store_history(&mut self, start: u64, lines: Vec<Line>) {
        for (i, line) in lines.into_iter().enumerate() {
            self.cache.insert(start + i as u64, line);
        }
        self.requested = None;
        self.prune();
        self.keep_sticky(true);
    }

    /// 要用到的历史行（只算回滚缓冲里的，屏幕上的本来就有）：网格上的，上下再各加 `screens`
    /// 屏；吸顶的命令行。
    ///
    /// 按行号排好的几段（起，止）。
    fn wanted(&self, screens: u64) -> Vec<(u64, u64)> {
        let rows = self.grid_rows() as u64;
        let layout = self.layout();
        let top = layout.top(self);
        let margin = rows * screens;
        // 按像素滚动时网格上面最多露出两行（零头和不足一行的滚动量），下面多露出一行。
        let from = top.saturating_sub(margin.max(2));
        let to = (top + rows + margin.max(1)).min(layout.len);
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        let mut add = |line: u64| {
            if line < self.history.0 || line >= self.history.1 {
                return;
            }
            match ranges.last_mut() {
                Some(last) if last.1 == line => last.1 += 1,
                _ => ranges.push((line, line + 1)),
            }
        };
        for d in from..to {
            if let Some(ViewRow::Line(line)) = layout.row_at(d) {
                add(line);
            }
        }
        // 吸顶的命令行在网格上面：按位置插进去（各段是按行号排好的）。
        if let Some(block) = self.sticky() {
            let line = self.blocks[block].command;
            if (self.history.0..self.history.1).contains(&line)
                && !ranges.iter().any(|r| (r.0..r.1).contains(&line))
            {
                let at = ranges.partition_point(|r| r.0 < line);
                ranges.insert(at, (line, line + 1));
            }
        }
        ranges
    }

    /// 要用的历史行里还缺的第一段：（起，止），没缺时 `None`。
    pub fn missing(&self) -> Option<(u64, u64)> {
        // 看着最新的输出时只要网格上的（折起了一段时网格上会有回滚缓冲里的行），不先往上取；
        // 拖着分隔条时先取一屏。
        let screens = if self.scroll > 0 || self.scroll_frac > 0.0 {
            CACHE_SCREENS as u64
        } else {
            u64::from(self.ahead)
        };
        self.wanted(screens).into_iter().find_map(|(start, end)| {
            let first = (start..end).find(|l| !self.cache.contains_key(l))?;
            let last = (first..end)
                .take_while(|l| !self.cache.contains_key(l))
                .last()?;
            Some((first, last + 1))
        })
    }

    /// 移除离可见范围太远的缓存，只留可见范围上下各几屏；选区用到的行不移除（复制要用）。
    fn prune(&mut self) {
        if self.cache.is_empty() {
            return;
        }
        let keep = self.wanted(CACHE_SCREENS as u64 + 1);
        let (keep_from, keep_to) = match self.selection {
            Some(s) => (
                s.anchor.line.min(s.head.line),
                s.anchor.line.max(s.head.line),
            ),
            None => (u64::MAX, 0),
        };
        self.cache.retain(|&line, _| {
            keep.iter()
                .any(|&(start, end)| (start..end).contains(&line))
                || (keep_from..=keep_to).contains(&line)
        });
    }

    /// 当前折起来的各段，按位置排好。
    fn folds(&self) -> Vec<Fold> {
        if self.folded.is_empty() || self.modes.has(Modes::ALT_SCREEN) {
            return Vec::new();
        }
        let last = self.history.1 + self.rows.len() as u64;
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| self.folded.contains(&b.id))
            .filter_map(|(i, b)| {
                let start = b.output?.max(self.history.0);
                let end = b.end?.min(last);
                (end > start).then_some(Fold {
                    start,
                    end,
                    block: i,
                })
            })
            .collect()
    }

    /// 显示行的排法（折起的各段和总行数），一次算好，逐行查时不必每行重算。
    fn layout(&self) -> Layout {
        let folds = self.folds();
        let lines = self.history.1 + self.rows.len() as u64 - self.history.0;
        let hidden: u64 = folds.iter().map(|f| f.end - f.start - 1).sum();
        let mut layout = Layout {
            folds,
            first: self.history.0,
            len: lines - hidden,
            end: 0,
        };
        layout.end = self.preview_end(&layout);
        layout
    }

    /// 面板的行数和屏幕不一样时（新的尺寸还没换上，见 `shown`），按辅助进程重排的规则排：主屏
    /// 下面没有内容的行去掉，从剩下的最后一行往上排，光标那一行不出网格；备用屏顶着上面排。
    ///
    /// 重排好的屏幕回来时每一行都在原来的地方。
    fn preview_end(&self, layout: &Layout) -> u64 {
        let grid = self.grid_rows() as u64;
        if grid == self.rows.len() as u64 || self.rows.is_empty() {
            return layout.len;
        }
        let screen = layout.display_of(self.history.1);
        if self.modes.has(Modes::ALT_SCREEN) {
            return screen + grid;
        }
        let cursor = usize::from(self.cursor.row);
        let used = self
            .rows
            .iter()
            .rposition(|line| !line.text.is_empty() || line.wrapped)
            .map_or(0, |i| i + 1)
            .max(cursor + 1)
            .min(self.rows.len());
        let last = layout.display_of(self.history.1 + used as u64 - 1) + 1;
        let cursor = layout.display_of(self.history.1 + cursor as u64);
        last.min(cursor + grid)
    }

    /// 一共有几个显示行。
    pub fn display_len(&self) -> u64 {
        self.layout().len
    }

    /// 第 `line` 行是第几个显示行（在折起的一段里时是那一段的那一行）。
    pub fn display_of(&self, line: u64) -> u64 {
        self.layout().display_of(line)
    }

    /// 第 `d` 个显示行显示什么；超出时 `None`。
    pub fn row_at(&self, d: u64) -> Option<ViewRow> {
        self.layout().row_at(d)
    }

    /// 网格第一行是第几个显示行。
    ///
    /// 显示行比网格还少时（刚开的终端折了一段）从第一行画起。
    pub fn view_top(&self) -> u64 {
        self.layout().top(self)
    }

    /// 网格上的各行，从上到下；下面没有内容的行不在里面。
    #[cfg(test)]
    pub fn view(&self) -> Vec<ViewRow> {
        self.rows_from(self.view_top(), self.grid_rows() as u64)
    }

    /// 从第 `first` 个显示行起的 `count` 行；下面没有内容的行不在里面。
    pub fn rows_from(&self, first: u64, count: u64) -> Vec<ViewRow> {
        let layout = self.layout();
        (first..first + count)
            .map_while(|d| layout.row_at(d))
            .collect()
    }

    /// 翻到让第 `line` 行出现在网格的第 `at` 行（`at` 超出网格时夹到最后一行）。
    pub fn scroll_to(&mut self, line: u64, at: u64) {
        self.scroll_frac = 0.0;
        let rows = self.grid_rows() as u64;
        let target = self
            .display_of(line)
            .saturating_sub(at.min(rows.saturating_sub(1)));
        let bottom = self.layout().end.saturating_sub(rows);
        self.scroll = bottom.saturating_sub(target) as usize;
        self.clamp_scroll();
    }

    /// 第 `line` 行看不见时翻过去：在折起的输出里时先展开，放在网格靠上三分之一的地方。
    pub fn reveal(&mut self, line: u64) {
        let hidden: Vec<u32> = self
            .folds()
            .iter()
            .filter(|f| (f.start..f.end).contains(&line))
            .map(|f| self.blocks[f.block].id)
            .collect();
        for id in hidden {
            self.set_folded(id, false);
        }
        let rows = self.grid_rows() as u64;
        let top = self.view_top();
        // 吸顶的命令行盖着网格第一行，那一行看不见。
        let first = top + u64::from(self.sticky().is_some());
        let d = self.display_of(line);
        if d < first || d >= top + rows {
            self.scroll_to(line, rows / 3);
        }
    }

    /// 折起或展开一个块的输出。
    ///
    /// 看最新输出时还看着最新的；翻上去时网格第一行不动。网格第一行在
    /// 要折起的输出里时改为从这个块的第一行排：折起的那一行落在第一行会被吸顶的命令行盖住。
    pub fn set_folded(&mut self, id: u32, folded: bool) {
        let anchor = (self.scroll > 0).then(|| {
            let top = self.top();
            match self.blocks.iter().find(|b| b.id == id) {
                Some(b) if folded && top > b.prompt && top < self.block_end(b) => b.prompt,
                _ => top,
            }
        });
        let at = self.folded.iter().position(|&f| f == id);
        let changed = match (folded, at) {
            (true, None) => {
                self.folded.push(id);
                true
            }
            (false, Some(at)) => {
                self.folded.swap_remove(at);
                true
            }
            _ => false,
        };
        if !changed {
            return;
        }
        if let Some(anchor) = anchor {
            self.scroll_to(anchor, 0);
        }
        self.clamp_scroll();
    }

    /// 第 `line` 行在哪个块里。
    ///
    /// 备用屏（全屏程序的屏幕）上一律不在：块的行号是主屏的，备用屏
    /// 的行占着同样的行号，否则旁边会画出之前那些命令的色带和结果。
    pub fn block_at(&self, line: u64) -> Option<usize> {
        if self.modes.has(Modes::ALT_SCREEN) {
            return None;
        }
        let i = self
            .blocks
            .partition_point(|b| b.prompt <= line)
            .checked_sub(1)?;
        (line < self.block_end(&self.blocks[i])).then_some(i)
    }

    /// 块在网格上到哪一行为止（不含）：结束了的到它的结束行，还在运行的到屏幕上有内容的地方
    /// （[`TermTab::content_end`]），色带不顺着下面的空行一直画到面板底。
    ///
    /// 块的消息比屏幕先到时
    /// 也不画到还没显示出来的行上。
    pub fn block_end(&self, block: &Block) -> u64 {
        let content = self.content_end();
        block.end.map_or(content, |end| end.min(content))
    }

    /// 屏幕上有内容的地方到哪一行为止（不含）：最后一行有字的，以及光标不在行首时光标所在的行。
    pub fn content_end(&self) -> u64 {
        let text = (self.rows.iter())
            .rposition(|r| !r.trimmed().is_empty())
            .map_or(0, |r| r + 1);
        let cursor = match self.cursor.col {
            0 => 0,
            _ => usize::from(self.cursor.row) + 1,
        };
        self.history.1 + text.max(cursor) as u64
    }

    /// 要吸顶的块：上沿已经越过视口上沿的最后一条运行过的命令。
    ///
    /// 下一条命令的提示符到达视口上沿后不再吸顶，此时提示符和命令行照常可见；命令行越过上沿后
    /// 吸顶换成它，位置连续。空回车不算下一条命令。
    pub fn sticky(&self) -> Option<usize> {
        self.sticky_at().map(|(block, _)| block)
    }

    /// 吸顶行被下一条命令的提示符顶上去的比例（行，`0.0..1.0`）。
    pub fn sticky_push(&self) -> f32 {
        self.sticky_at().map_or(0.0, |(_, push)| push)
    }

    fn sticky_at(&self) -> Option<(usize, f32)> {
        if self.modes.has(Modes::ALT_SCREEN) {
            return None;
        }
        let (first, cut) = self.viewport();
        let layout = self.layout();
        let limit = first + u64::from(cut > 0.0);
        let count = self
            .blocks
            .partition_point(|b| layout.display_of(b.command) < limit);
        let i = self.blocks[..count]
            .iter()
            .rposition(|b| b.output.is_some())?;
        let next = self.blocks[i + 1..]
            .iter()
            .find(|b| b.output.is_some() || b.end.is_none())
            .map(|b| layout.display_of(b.prompt));
        match next {
            Some(prompt) if prompt <= first => None,
            Some(prompt) if prompt == first + 1 => Some((i, cut)),
            _ => Some((i, 0.0)),
        }
    }

    /// 新的命令块。
    ///
    /// 辅助进程重排、清回滚以后块的行号会变，折叠按编号记，不受影响；没了的块不再
    /// 记着折叠。
    pub fn set_blocks(&mut self, blocks: Vec<Block>) {
        let anchor = (self.scroll > 0).then(|| self.top());
        let frac = self.scroll_frac;
        self.folded.retain(|id| blocks.iter().any(|b| b.id == *id));
        self.blocks = blocks;
        if let Some(anchor) = anchor {
            self.scroll_to(anchor, 0);
            self.scroll_frac = frac;
        }
        self.clamp_scroll();
    }

    /// 选区按单位扩好的（起点，终点），终点那一格不含；空选区返回 `None`。
    pub fn selection_range(&self) -> Option<(Point, Point)> {
        let s = self.selection?;
        let (from, to) = if s.anchor <= s.head {
            (s.anchor, s.head)
        } else {
            (s.head, s.anchor)
        };
        let range = match s.unit {
            Unit::Char => (from, to),
            Unit::Word => {
                let start = self.word_at(from).map_or(from, |(start, _)| start);
                let end = self.word_at(to).map_or(
                    Point {
                        col: to.col + 1,
                        ..to
                    },
                    |(_, end)| end,
                );
                (start, end)
            }
            Unit::Line => (
                Point {
                    line: from.line,
                    col: 0,
                },
                Point {
                    line: to.line,
                    col: usize::MAX,
                },
            ),
        };
        (range.0 < range.1).then_some(range)
    }

    /// 第 `line` 行上选中的格子范围（画选区用）。
    pub fn selected_cols(&self, line: u64) -> Option<(usize, usize)> {
        let (from, to) = self.selection_range()?;
        if line < from.line || line > to.line {
            return None;
        }
        let start = if line == from.line { from.col } else { 0 };
        let end = if line == to.line { to.col } else { usize::MAX };
        (start < end).then_some((start, end))
    }

    /// 选中的文字：软换行的行接起来不加换行，其余行去掉行尾空格、用换行分开。
    ///
    /// 没有选区时返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 选区里有还没缓存的历史行时返回 `Err`，带上要先取回的范围；已经滚出回滚缓冲的行不算缺
    /// （取不回来了）。
    pub fn selected_text(&self) -> Result<Option<String>, (u64, u64)> {
        self.text_of_selection(false)
    }

    /// 同 [`TermTab::selected_text`]，但缺的行直接跳过：取过一次还缺（辅助进程那边已经不再保留）
    /// 时用，不会一直要下去。
    pub fn selected_text_lossy(&self) -> Option<String> {
        self.text_of_selection(true).ok().flatten()
    }

    fn text_of_selection(&self, lossy: bool) -> Result<Option<String>, (u64, u64)> {
        let Some((mut from, to)) = self.selection_range() else {
            return Ok(None);
        };
        if to.line < self.history.0 {
            return Ok(None);
        }
        if from.line < self.history.0 {
            from = Point {
                line: self.history.0,
                col: 0,
            };
        }
        if !lossy {
            let mut missing = (from.line..=to.line).filter(|&l| self.line(l).is_none());
            if let Some(first) = missing.next() {
                let last = missing.next_back().unwrap_or(first);
                return Err((first, last + 1));
            }
        }
        let mut text = String::new();
        for l in from.line..=to.line {
            let Some(line) = self.line(l) else { continue };
            let start = if l == from.line { from.col } else { 0 };
            let end = if l == to.line { to.col } else { usize::MAX };
            let part = cells_text(line, start, end);
            if line.wrapped && l != to.line {
                // 宽字符放不下最后一格而折到下一行时，行末留下的占位空格不属于文字。
                let filler = line
                    .runs
                    .last()
                    .is_some_and(|r| r.style.attrs.has(Attrs::FILLER));
                let part = match filler {
                    true => part.strip_suffix(' ').unwrap_or(&part),
                    false => &part,
                };
                text.push_str(part);
            } else {
                text.push_str(part.trim_end_matches(' '));
                if l != to.line {
                    text.push('\n');
                }
            }
        }
        Ok(Some(text))
    }

    /// 选中全部：回滚缓冲加屏幕上有字的部分。
    pub fn select_all(&mut self) {
        let last = (0..self.rows.len())
            .rev()
            .find(|&r| !self.rows[r].trimmed().is_empty())
            .unwrap_or(0);
        self.selection = Some(Selection {
            anchor: Point {
                line: self.history.0,
                col: 0,
            },
            head: Point {
                line: self.history.1 + last as u64,
                col: usize::MAX,
            },
            unit: Unit::Char,
        });
    }

    /// 选中第 `index` 个块的整段输出，折起的先展开。
    ///
    /// 还没运行、没有输出时返回假。
    pub fn select_output(&mut self, index: usize) -> bool {
        let Some(b) = self.blocks.get(index) else {
            return false;
        };
        let (Some(output), id) = (b.output, b.id) else {
            return false;
        };
        // 还在运行的到屏幕上有内容的地方，不连着下面的空行一起选。
        let end = self.block_end(b);
        if end <= output {
            return false;
        }
        self.set_folded(id, false);
        self.selection = Some(Selection {
            anchor: Point {
                line: output,
                col: 0,
            },
            head: Point {
                line: end - 1,
                col: usize::MAX,
            },
            unit: Unit::Char,
        });
        true
    }

    /// 双击选词：`point` 所在的那个「词」的范围。
    ///
    /// 路径、`文件:行` 里的 `/ . - _ ~ :` 也算词的
    /// 一部分，一次就能选中整条路径。点在空白上时返回 `None`。
    pub fn word_at(&self, point: Point) -> Option<(Point, Point)> {
        let line = self.line(point.line)?;
        let cells = cell_chars(&line.text);
        let at = cells.iter().position(|&(col, w, _)| point.col < col + w)?;
        if !is_word(cells[at].2) {
            return None;
        }
        let mut start = at;
        while start > 0 && is_word(cells[start - 1].2) {
            start -= 1;
        }
        let mut end = at;
        while end + 1 < cells.len() && is_word(cells[end + 1].2) {
            end += 1;
        }
        let (col, w, _) = cells[end];
        Some((
            Point {
                line: point.line,
                col: cells[start].0,
            },
            Point {
                line: point.line,
                col: col + w,
            },
        ))
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '/' | '.' | '-' | '_' | '~' | ':' | '@' | '%' | '+' | '=' | '#'
        )
}

/// 一行里每个字：（起始格，占几格，字）。
///
/// 组合字符并到前一个字里不单列。
fn cell_chars(text: &str) -> Vec<(usize, usize, char)> {
    let mut out = Vec::with_capacity(text.len());
    let mut col = 0;
    for (c, w) in cells(text) {
        if w == 0 {
            continue;
        }
        out.push((col, w, c));
        col += w;
    }
    out
}

/// 一行里从第 `start` 格到第 `end` 格（不含）的文字，组合字符跟着它的字。
pub(crate) fn cells_text(line: &Line, start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut col = 0;
    let mut inside = false;
    for (c, w) in cells(&line.text) {
        if w == 0 {
            if inside {
                out.push(c);
            }
            continue;
        }
        // 宽字符占两格：选区盖到其中一格就算选上（从它的后半格开始选时也要复制它）。
        inside = col + w > start && col < end;
        if inside {
            out.push(c);
        }
        col += w;
    }
    out
}

/// 返回一行参与查找的文字：软换行时去掉行末的占位空格（宽字符无法放入最后一格时产生）。
pub(crate) fn logical_text(line: &Line) -> &str {
    let filler =
        line.wrapped && (line.runs.last()).is_some_and(|run| run.style.attrs.has(Attrs::FILLER));
    if filler {
        line.text.strip_suffix(' ').unwrap_or(&line.text)
    } else {
        &line.text
    }
}

/// 返回字节偏移 `byte` 在文字 `text` 中对应的格。
pub(crate) fn col_at(text: &str, byte: usize) -> usize {
    let mut seen = 0;
    let mut col = 0;
    for (c, width) in cells(text) {
        if seen >= byte {
            break;
        }
        seen += c.len_utf8();
        col += width;
    }
    col
}

#[cfg(test)]
mod tests;
