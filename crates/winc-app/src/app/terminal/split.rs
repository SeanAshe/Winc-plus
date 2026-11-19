//! 终端标签的拆分：一个标签里并排几个终端，面板在下方时左右排，在右侧时上下排。
//!
//! 前台的终端占的那一块就是终端的内容区域（`App::terminal_content_card`），其余终端绘制、
//! 滚动时临时换到前台（[`App::with_terminal_pane`]）。

use super::*;
use crate::app::workspace::panes::DIVIDER;

/// 把整块内容区域 `card` 平分给 `n` 个终端；`column` 为真时上下排。
///
/// 上下排时每一块都在顶上留出标签栏的高度，网格从各自那一块的上沿开始。
pub(crate) fn pane_cards(card: Rect, n: usize, column: bool) -> Vec<Rect> {
    let n = n.max(1);
    let gaps = DIVIDER * (n - 1) as f32;
    if !column {
        let w = ((card.w - gaps) / n as f32).floor().max(0.0);
        return (0..n)
            .map(|i| {
                let x = card.x + i as f32 * (w + DIVIDER);
                let last = i + 1 == n;
                let w = if last { card.right() - x } else { w };
                Rect::new(x, card.y, w, card.h)
            })
            .collect();
    }
    let header = header_height(card);
    let top = card.y + header;
    let room = (card.bottom() - top - gaps).max(0.0);
    let h = (room / n as f32).floor();
    (0..n)
        .map(|i| {
            let y = top + i as f32 * (h + DIVIDER);
            let last = i + 1 == n;
            let h = if last { card.bottom() - y } else { h };
            Rect::new(card.x, y - header, card.w, h + header)
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
        let cards = pane_cards(card, tab.pane_count(), column);
        let header = header_height(card);
        (cards.windows(2))
            .map(|pair| match column {
                false => Rect::new(pair[0].right(), card.y + header, DIVIDER, card.h - header),
                true => Rect::new(card.x, pair[0].bottom(), card.w, DIVIDER),
            })
            .collect()
    }

    /// 当前标签中 `(x, y)` 所在的、不在前台的终端的位置；被浮层盖住时为 `None`。
    pub(in crate::app) fn terminal_pane_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.overlay_covers(x, y) {
            return None;
        }
        let tab = self.terminal.tab().filter(|t| t.pane_count() > 1)?;
        let card = self.terminal_whole_card();
        let header = header_height(card);
        let cards = pane_cards(card, tab.pane_count(), self.terminal_right());
        let inside = |c: &Rect| {
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

    /// 前台换成当前标签里的下一个（`forward`）或上一个终端，到头时回到另一端。
    pub(in crate::app) fn cycle_terminal_pane(&mut self, forward: bool) {
        let Some(tab) = self.terminal.tab() else {
            return;
        };
        let (n, at) = (tab.pane_count(), tab.pane_at());
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
