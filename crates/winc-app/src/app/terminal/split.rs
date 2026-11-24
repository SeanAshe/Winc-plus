//! 终端标签的拆分：一个标签里并排几个终端，面板在下方时左右排，在右侧时上下排。
//!
//! 前台的终端占的那一块就是终端的内容区域（`App::terminal_content_card`），其余终端绘制、
//! 滚动时临时换到前台（[`App::with_terminal_pane`]）。

use super::*;
use crate::app::Hover;
use crate::app::workspace::panes::{DIVIDER, GRIP};

/// 拆分的终端最小的宽度（上下排时为终端区的高度，逻辑像素）：拖动分隔线时不再缩小。
///
/// 取自调参：约容纳十个字符或四行，窄到这里已无法正常使用。
const PANE_MIN: f32 = 80.0;

/// 按份额 `shares`（见 `TermTab::share`）把整块内容区域 `card` 分给各终端；`column` 为真时上下排。
///
/// 上下排时分的是标签栏以下的部分，每一块往上多出一截，使 `header_height` 量出的标签栏正好
/// 盖住这一截，网格从各自那一块的终端区开始。
pub(crate) fn pane_cards(card: Rect, shares: &[f32], column: bool) -> Vec<Rect> {
    let n = shares.len().max(1);
    let gaps = DIVIDER * (n - 1) as f32;
    let header = if column { header_height(card) } else { 0.0 };
    let (start, length) = match column {
        false => (card.x, card.w),
        true => (card.y + header, card.h - header),
    };
    let room = (length - gaps).max(0.0);
    let total: f32 = shares.iter().sum();
    let fraction = |s: f32| {
        if total > 0.0 {
            s / total
        } else {
            1.0 / n as f32
        }
    };
    let mut at = start;
    (0..n)
        .map(|i| {
            let last = i + 1 == n;
            let size = match last {
                true => (start + length - at).max(0.0),
                false => (room * fraction(shares.get(i).copied().unwrap_or(1.0))).floor(),
            };
            let rect = match column {
                false => Rect::new(at, card.y, size, card.h),
                true if i == 0 => Rect::new(card.x, card.y, card.w, size + header),
                true => {
                    // 下面几块离窗口顶栏远，标签栏是 `HEADER` 高；离得近时两种算法都取顶栏的高度。
                    let lift = header_height(Rect::new(card.x, at - HEADER, card.w, 0.0));
                    Rect::new(card.x, at - lift, card.w, size + lift)
                }
            };
            at += size + DIVIDER;
            rect
        })
        .collect()
}

impl App {
    /// 当前标签各终端之间的分隔线；没有拆分时为空。
    pub(crate) fn terminal_pane_dividers(&self) -> Vec<Rect> {
        let Some(tab) = self.terminal.tab().filter(|t| t.pane_count() > 1) else {
            return Vec::new();
        };
        let card = self.terminal_whole_card();
        let column = self.terminal_right();
        let cards = pane_cards(card, &tab.pane_shares(), column);
        let header = header_height(card);
        (cards.windows(2))
            .map(|pair| match column {
                false => Rect::new(pair[0].right(), card.y + header, DIVIDER, card.h - header),
                true => Rect::new(card.x, pair[0].bottom(), card.w, DIVIDER),
            })
            .collect()
    }

    /// 当前标签中 `(x, y)` 所在的分隔线：第几条（第 `n` 个与第 `n + 1` 个终端之间）。可拖动的范围
    /// 是分隔线两侧各放宽 [`GRIP`]；被浮层遮住时为 `None`。
    pub(in crate::app) fn terminal_split_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.overlay_covers(x, y) {
            return None;
        }
        let column = self.terminal_right();
        self.terminal_pane_dividers().iter().position(|line| {
            let grip = match column {
                false => Rect::new(line.x - GRIP, line.y, line.w + 2.0 * GRIP, line.h),
                true => Rect::new(line.x, line.y - GRIP, line.w, line.h + 2.0 * GRIP),
            };
            grip.contains(x, y)
        })
    }

    /// 正被悬停或拖动的分隔线（见 [`Self::terminal_split_at`]）。
    pub(crate) fn active_terminal_split(&self) -> Option<usize> {
        match (self.pointer.hover, &self.pointer.drag) {
            (_, crate::app::Drag::TerminalSplit { index }) => Some(*index),
            (Hover::TerminalSplit(index), _) => Some(index),
            _ => None,
        }
    }

    /// 把第 `index` 条分隔线拖到 `(x, y)`：两侧的终端重新分配它们合起来的大小，各自不小于
    /// [`PANE_MIN`]，其余终端不变。
    pub(in crate::app) fn drag_terminal_split(&mut self, index: usize, (x, y): (f32, f32)) {
        let Some(mut shares) = self.terminal.tab().map(TermTab::pane_shares) else {
            return;
        };
        if index + 1 >= shares.len() {
            return;
        }
        let (card, column) = (self.terminal_whole_card(), self.terminal_right());
        let cards = pane_cards(card, &shares, column);
        let (start, end, at) = match column {
            false => (cards[index].x, cards[index + 1].right(), x),
            true => {
                let top = cards[index].y + header_height(cards[index]);
                (top, cards[index + 1].bottom(), y)
            }
        };
        let size = end - start - DIVIDER;
        if size < PANE_MIN * 2.0 {
            return;
        }
        let first = (at - start).clamp(PANE_MIN, size - PANE_MIN);
        let pair = shares[index] + shares[index + 1];
        shares[index] = pair * first / size;
        shares[index + 1] = pair - shares[index];
        self.set_terminal_shares(&shares);
    }

    /// 双击第 `index` 条分隔线：两侧的终端平分它们合起来的大小。
    pub(in crate::app) fn reset_terminal_split(&mut self, index: usize) {
        let Some(mut shares) = self.terminal.tab().map(TermTab::pane_shares) else {
            return;
        };
        if index + 1 >= shares.len() {
            return;
        }
        let half = (shares[index] + shares[index + 1]) / 2.0;
        shares[index] = half;
        shares[index + 1] = half;
        self.set_terminal_shares(&shares);
    }

    /// 按排列顺序设置当前标签各终端的份额 `shares`，并重新排版。
    fn set_terminal_shares(&mut self, shares: &[f32]) {
        let index = self.terminal.active;
        if index >= self.terminal.tabs.len() {
            return;
        }
        let (mut all, at) = self.terminal.take_group(index);
        for (tab, &share) in all.iter_mut().zip(shares) {
            tab.share = share;
        }
        self.terminal.put_group(index, all, at);
        self.relayout();
        self.request_redraw();
    }

    /// 当前标签中 `(x, y)` 所在的、不在前台的终端的位置；被浮层遮住时为 `None`。
    pub(in crate::app) fn terminal_pane_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.overlay_covers(x, y) {
            return None;
        }
        let tab = self.terminal.tab().filter(|t| t.pane_count() > 1)?;
        let card = self.terminal_whole_card();
        let cards = pane_cards(card, &tab.pane_shares(), self.terminal_right());
        let inside = |c: &Rect| {
            let header = header_height(*c);
            let body = Rect::new(c.x, c.y + header, c.w, c.h - header);
            body.contains(x, y)
        };
        let at = cards.iter().position(inside)?;
        (at != tab.pane_at()).then_some(at)
    }

    /// 临时把当前标签中第 `at` 个终端换到前台，运行 `f`，再换回原来的。
    pub(crate) fn with_terminal_pane<T>(&mut self, at: usize, f: impl FnOnce(&mut App) -> T) -> T {
        let index = self.terminal.active;
        let Some(back) = self.terminal.tab().map(TermTab::pane_at) else {
            return f(self);
        };
        if at == back || index >= self.terminal.tabs.len() {
            return f(self);
        }
        self.rotate_terminal_pane(index, at);
        let result = f(self);
        self.rotate_terminal_pane(index, back);
        result
    }

    /// 把第 `index` 个标签中第 `at` 个终端换到前台，滚动条的显示状态跟着换。
    fn rotate_terminal_pane(&mut self, index: usize, at: usize) {
        let area = [crate::app::ScrollArea::Terminal];
        if let Some(tab) = self.terminal.tabs.get_mut(index) {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
        let (all, _) = self.terminal.take_group(index);
        self.terminal.put_group(index, all, at);
        if let Some(tab) = self.terminal.tabs.get_mut(index) {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
    }

    /// 把当前标签中第 `at` 个终端换到前台。
    pub(in crate::app) fn focus_terminal_pane(&mut self, at: usize) {
        let index = self.terminal.active;
        let Some(tab) = self.terminal.tab() else {
            return;
        };
        if at == tab.pane_at() || at >= tab.pane_count() {
            return;
        }
        self.blur_terminal_tab();
        self.rotate_terminal_pane(index, at);
        self.terminal.focus_sent = false;
        self.focus = Focus::Terminal;
        self.relayout();
        self.request_redraw();
    }

    /// 点在当前标签里不在前台的终端上：把它换到前台；返回是否换了。
    pub(in crate::app) fn focus_terminal_pane_at(&mut self, x: f32, y: f32) -> bool {
        match self.terminal_pane_at(x, y) {
            Some(at) => {
                self.focus_terminal_pane(at);
                true
            }
            None => false,
        }
    }

    /// 前台换成当前标签里的下一个（`forward`）或上一个终端；到了一端时分栏的进相邻一栏，没有分栏
    /// 的回到另一端。
    pub(in crate::app) fn cycle_terminal_pane(&mut self, forward: bool) {
        let Some(tab) = self.terminal.tab() else {
            return;
        };
        let (n, at) = (tab.pane_count(), tab.pane_at());
        let inside = if forward { at + 1 < n } else { at > 0 };
        let groups = self.terminal.group_count();
        if !inside && groups > 1 {
            let now = self.terminal.group_at;
            let next = if forward {
                (now + 1) % groups
            } else {
                (now + groups - 1) % groups
            };
            self.focus_terminal_group(next);
            let n = self.terminal.tab().map_or(1, TermTab::pane_count);
            self.focus_terminal_pane(if forward { 0 } else { n - 1 });
            return;
        }
        if n < 2 {
            return;
        }
        self.focus_terminal_pane(if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        });
    }

    /// 编号为 `id` 的标签能不能拆分：它前台的终端还在运行。
    pub(in crate::app) fn terminal_splittable(&self, id: u16) -> bool {
        let tab = self.terminal.tabs.iter().find(|t| t.id == id);
        !self.terminal.crashed && tab.is_some_and(|t| t.ended.is_none())
    }

    /// 拆分当前的终端标签：新终端在当前终端的目录里启动，排在它后面；没有终端时新建一个。
    pub(in crate::app) fn split_terminal(&mut self) {
        let Some(tab) = self.terminal.tab().filter(|t| t.ended.is_none()) else {
            return self.new_terminal_tab();
        };
        let (id, cwd) = (tab.id, tab.cwd.clone());
        self.terminal.split_into = Some(id);
        self.new_terminal_tab_in(cwd);
    }
}
