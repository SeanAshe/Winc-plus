//! 终端分栏及其归属：停靠栏有各自的标签栏，编辑器栏对应一个编辑器标签。
//!
//! 面板在下方时停靠栏左右排，在右侧时上下排；编辑器栏由编辑器窗格确定位置。正在用的一栏的
//! 标签放在 [`TerminalPanel`] 的 `tabs`、`active`、`strip` 中，其余各栏存在 `groups` 中。绘制或
//! 处理其他栏时临时换入（[`App::with_terminal_group`]）；归属和可见性始终保留在原始栏槽位中。
//! 栏内终端仍可拆分（[`super::split`]）。

use super::strip::Strip;
use super::*;
use crate::app::Hover;
use crate::app::workspace::panes::{DIVIDER, DROP_EDGE, GRIP};

/// 一栏最小的宽度（上下排时为高度，逻辑像素）：拖动分隔线时不再缩小。
///
/// 取自调参：放得下一个收窄的标签、「+」和几个按钮，或者标签栏加四五行。
const GROUP_MIN: f32 = 160.0;

/// 不在用的一栏。正在用的一栏在 `TerminalPanel::groups` 里只占位置、记份额。
#[derive(Default)]
pub(crate) struct TermGroup {
    /// 各标签。
    pub tabs: Vec<TermTab>,
    /// 这一栏正在看的标签。
    pub active: usize,
    /// 这一栏标签栏的动画和拖动。
    pub strip: Strip,
    /// 这一栏占面板的份额：各栏按份额的比例分面板，新的分栏让各栏平分。
    pub share: f32,
    /// 编辑器中的终端标签；为 `None` 时这一栏停靠在终端面板里。
    pub editor: Option<winc_editor::DocumentId>,
    /// 编辑器标签正在某个窗格中显示；面板栏的可见性由 `TerminalPanel::open` 决定。
    pub shown: bool,
}

impl TermGroup {
    /// 空的一栏，份额为 1。
    fn new() -> TermGroup {
        TermGroup::empty(1.0)
    }

    /// 空的一栏，份额为 `share`。
    pub(crate) fn empty(share: f32) -> TermGroup {
        TermGroup {
            share,
            ..TermGroup::default()
        }
    }
}

/// 拖动标签、移到新分栏时标签去哪里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupDrop {
    /// 放进第 `group` 栏的第 `at` 个位置（`None` 是最后）。
    Into {
        /// 第几栏。
        group: usize,
        /// 放在这一栏的第几个位置。
        at: Option<usize>,
    },
    /// 在第几个位置新加一栏，放进去。
    New(usize),
}

/// 按份额 `shares` 把面板 `card` 分给各栏；`column` 为真时上下排。每一栏都是完整的一块，
/// 有自己的标签栏。
pub(crate) fn group_cards(card: Rect, shares: &[f32], column: bool) -> Vec<Rect> {
    let n = shares.len().max(1);
    let (start, length) = match column {
        false => (card.x, card.w),
        true => (card.y, card.h),
    };
    let room = (length - DIVIDER * (n - 1) as f32).max(0.0);
    let total: f32 = shares.iter().sum();
    let mut at = start;
    (0..n)
        .map(|i| {
            let size = if i + 1 == n {
                (start + length - at).max(0.0)
            } else if total > 0.0 {
                (room * shares[i] / total).floor()
            } else {
                (room / n as f32).floor()
            };
            let rect = match column {
                false => Rect::new(at, card.y, size, card.h),
                true => Rect::new(card.x, at, card.w, size),
            };
            at += size + DIVIDER;
            rect
        })
        .collect()
}

impl TerminalPanel {
    /// 是否临时换进了不在用的一栏（[`App::with_terminal_group`]）。
    pub(crate) fn borrowed(&self) -> bool {
        self.borrowed
    }

    /// 面板分成了几栏；没有分栏时为 1。
    pub(crate) fn group_count(&self) -> usize {
        self.groups.len().max(1)
    }

    /// 第 `at` 栏所在的编辑器标签；面板栏没有编辑器标签。
    pub(crate) fn editor_at(&self, at: usize) -> Option<winc_editor::DocumentId> {
        self.groups.get(at).and_then(|group| group.editor)
    }

    /// 正在用的终端是否位于编辑器标签里。
    pub(crate) fn editor_document(&self) -> Option<winc_editor::DocumentId> {
        self.editor_at(self.group_at)
    }

    /// 第 `at` 栏的内容是否正在显示。
    pub(crate) fn group_shown(&self, at: usize) -> bool {
        match self.groups.get(at).filter(|group| group.editor.is_some()) {
            Some(group) => group.shown,
            None => self.open,
        }
    }

    /// 停靠在终端面板里的栏，按排列顺序。
    pub(crate) fn panel_groups(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.group_count()).filter(|&at| self.editor_at(at).is_none())
    }

    /// 停靠在终端面板里的标签数量。
    pub(crate) fn panel_tab_count(&self) -> usize {
        self.panel_groups()
            .map(|at| self.group_tabs(at).len())
            .sum()
    }

    /// 停靠栏的份额，按面板上的排列顺序，不包含编辑器终端。
    pub(crate) fn panel_shares(&self) -> Vec<f32> {
        self.panel_groups()
            .map(|at| self.groups.get(at).map_or(1.0, |group| group.share))
            .collect()
    }

    /// 第 `at` 栏的标签。
    pub(crate) fn group_tabs(&self, at: usize) -> &[TermTab] {
        match self.groups.get(at) {
            Some(group) if at != self.group_at => &group.tabs,
            _ => &self.tabs,
        }
    }

    /// 各栏的全部标签，按排列顺序。
    pub(crate) fn all_tabs(&self) -> impl Iterator<Item = &TermTab> {
        (0..self.group_count()).flat_map(|at| self.group_tabs(at))
    }

    /// 各栏的标签一共有几个。
    pub(crate) fn tab_count(&self) -> usize {
        (0..self.group_count())
            .map(|at| self.group_tabs(at).len())
            .sum()
    }

    /// 各栏正在看的标签：都在眼前，都要收新的帧。
    pub(crate) fn shown_tabs(&self) -> impl Iterator<Item = &TermTab> {
        (0..self.group_count())
            .filter(|&at| self.group_shown(at))
            .filter_map(|at| match self.groups.get(at) {
                Some(group) if at != self.group_at => group.tabs.get(group.active),
                _ => self.tab(),
            })
    }

    /// 编号为 `id` 的终端（可以是拆分出的）在第几栏。
    pub(crate) fn group_of(&self, id: u16) -> Option<usize> {
        (0..self.group_count()).find(|&at| {
            (self.group_tabs(at).iter()).any(|tab| tab.panes().any(|pane| pane.id == id))
        })
    }

    /// 把第 `at` 栏换成正在用的一栏，原来正在用的一栏放回它的位置。
    ///
    /// 只换数据，不管焦点、滚动条和重绘（见 [`App::focus_terminal_group`]）。
    pub(crate) fn swap_group(&mut self, at: usize) {
        if at == self.group_at || at >= self.groups.len() {
            return;
        }
        for to in [self.group_at, at] {
            let group = &mut self.groups[to];
            std::mem::swap(&mut self.tabs, &mut group.tabs);
            std::mem::swap(&mut self.active, &mut group.active);
            std::mem::swap(&mut self.strip, &mut group.strip);
        }
        self.group_at = at;
        if self.editor_at(at).is_none() {
            self.panel_at = at;
        }
    }

    /// 在第 `at` 个位置加一个空的栏；加了以后各栏平分面板。
    pub(crate) fn insert_group(&mut self, at: usize) {
        if self.groups.is_empty() {
            self.groups.push(TermGroup::new());
        }
        let at = at.min(self.groups.len());
        self.groups.insert(at, TermGroup::new());
        if at <= self.group_at {
            self.group_at += 1;
        }
        if at <= self.panel_at {
            self.panel_at += 1;
        }
        self.groups
            .iter_mut()
            .filter(|group| group.editor.is_none())
            .for_each(|group| group.share = 1.0);
    }

    /// 去掉不在用的空栏；正在用的一栏空了、也没有等着建的标签时同样去掉，改用它前面一栏
    /// （它是第一栏时用后面一栏）。返回是否去掉了。
    pub(crate) fn drop_empty_groups(&mut self) -> bool {
        // 还有等着建的标签：它们可能要放进现在空着的栏（恢复现场时）。
        if !self.want.is_empty() || !self.run_queue.is_empty() {
            return false;
        }
        if self.groups.len() > 1 && self.tabs.is_empty() {
            let next = if self.group_at > 0 {
                self.group_at - 1
            } else {
                1
            };
            self.swap_group(next);
        }
        let before = self.groups.len();
        let focused = self.group_at;
        let panel = self.panel_at;
        let mut panel_at = None;
        let mut kept = Vec::with_capacity(before);
        for (at, group) in std::mem::take(&mut self.groups).into_iter().enumerate() {
            if at == focused {
                self.group_at = kept.len();
            }
            if at == focused || !group.tabs.is_empty() {
                if at == panel {
                    panel_at = Some(kept.len());
                }
                kept.push(group);
            }
        }
        self.groups = kept;
        if self.groups.len() == 1 && self.groups[0].editor.is_none() {
            self.groups.clear();
            self.group_at = 0;
        }
        self.panel_at = if self.groups.is_empty() {
            0
        } else {
            panel_at.or_else(|| self.panel_groups().next()).unwrap_or(0)
        };
        self.groups.len() != before
    }

    /// 把标签 `tab` 放进第 `at` 栏的第 `slot` 个位置（`None` 是最后），并让那一栏看它。没有
    /// 这一栏时先加上。
    pub(crate) fn place_tab(&mut self, at: usize, tab: TermTab, slot: Option<usize>) {
        while at >= self.group_count() {
            let n = self.group_count();
            self.insert_group(n);
        }
        let editor = self.editor_at(at).is_some();
        let (tabs, active) = match self.groups.get_mut(at) {
            Some(group) if at != self.group_at => (&mut group.tabs, &mut group.active),
            _ => (&mut self.tabs, &mut self.active),
        };
        if editor && !tabs.is_empty() {
            super::insert_pane(tabs, 0, tab);
            *active = 0;
        } else {
            let slot = slot.unwrap_or(tabs.len()).min(tabs.len());
            tabs.insert(slot, tab);
            *active = slot;
        }
    }

    /// 按保存现场时各 shell 的顺序恢复前台终端，包含编辑器标签中合并恢复的拆分终端。
    pub(crate) fn focus_shell(&mut self, index: usize) {
        let mut remaining = index;
        let mut target = None;
        for at in 0..self.group_count() {
            for (root, tab) in self.group_tabs(at).iter().enumerate() {
                if remaining < tab.pane_count() {
                    target = Some((at, root, remaining));
                    break;
                }
                remaining -= tab.pane_count();
            }
            if target.is_some() {
                break;
            }
        }
        if let Some((at, root, pane)) = target {
            self.swap_group(at);
            self.active = root;
            let (all, _) = self.take_group(root);
            self.put_group(root, all, pane);
            self.focus_sent = false;
        }
    }

    /// 关闭尚未启动的编辑器终端时，移除该栏的请求，其他栏的启动顺序不变。
    pub(crate) fn cancel_group_wants(&mut self, group: usize) {
        let mut removed = false;
        for i in (0..self.want.len()).rev() {
            if self.want_groups.get(i).copied().unwrap_or(self.group_at) != group {
                continue;
            }
            self.want.remove(i);
            if i < self.want_groups.len() {
                self.want_groups.remove(i);
            }
            if i < self.want_names.len() {
                self.want_names.remove(i);
            }
            if i < self.want_shells.len() {
                self.want_shells.remove(i);
            }
            if i < self.want_into.len() {
                self.want_into.remove(i);
            }
            removed = true;
        }
        if removed {
            self.want_active = None;
            if self.want.is_empty() {
                self.pending_input = None;
            }
        }
    }
}

impl App {
    /// 面板栏的位置按全局分栏编号存放；编辑器终端的槽位为空矩形。
    pub(crate) fn terminal_group_cards_in(&self, card: Rect) -> Vec<Rect> {
        let t = &self.terminal;
        let shares = t.panel_shares();
        if shares.is_empty() {
            return vec![Rect::default(); t.group_count()];
        }
        let mut cards = group_cards(card, &shares, self.terminal_right());
        let mut dock = cards.len();
        cards.resize(t.group_count(), Rect::default());
        for at in (0..t.group_count()).rev() {
            if t.editor_at(at).is_none() {
                dock -= 1;
                cards[at] = cards[dock];
            } else {
                cards[at] = Rect::default();
            }
        }
        cards
    }

    /// 当前终端的卡片：停靠栏使用面板的位置，编辑器终端使用所在窗格的位置。
    pub(crate) fn terminal_group_card_in(&self, card: Rect) -> Rect {
        if let Some(document) = self.terminal.editor_document() {
            return self.editor_terminal_card(document);
        }
        let cards = self.terminal_group_cards_in(card);
        cards.get(self.terminal.group_at).copied().unwrap_or(card)
    }

    /// 鼠标所在的可见终端栏：面板标签栏也算，编辑器终端只包含内容区。
    pub(crate) fn terminal_group_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.overlay_covers(x, y) {
            return None;
        }
        if self.docked_search_shown() && self.layout.terminal.contains(x, y) {
            if y < self.layout.terminal.y + header_height(self.layout.terminal) {
                return self.terminal.panel_groups().next();
            }
            return None;
        }
        if self.layout.terminal.contains(x, y) && !self.docked_search_shown() {
            let cards = self.terminal_group_cards_in(self.layout.terminal);
            if let Some(at) = cards.iter().position(|card| card.contains(x, y)) {
                return Some(at);
            }
        }
        self.terminal
            .groups
            .iter()
            .enumerate()
            .find_map(|(at, group)| {
                let document = group.editor.filter(|_| group.shown)?;
                self.editor_terminal_pane(document)
                    .filter(|pane| pane.editor.contains(x, y))
                    .map(|_| at)
            })
    }

    /// 面板级按钮只放在面板右上角的停靠栏里，不放在编辑器终端中。
    pub(crate) fn terminal_panel_actions_here(&self) -> bool {
        let t = &self.terminal;
        if t.editor_document().is_some() {
            return false;
        }
        let corner = if self.terminal_right() {
            t.panel_groups().next()
        } else {
            t.panel_groups().last()
        };
        corner == Some(t.group_at)
    }

    /// 鼠标所在那一栏标签栏上 `hit` 的位置（悬停提示用）。
    pub(crate) fn terminal_hovered_hit(&self, hit: HeaderHit) -> Option<Rect> {
        let (x, y) = self.pointer.mouse;
        let hits = self.terminal_hits_at(x, y);
        hits.iter().find(|(_, h)| *h == hit).map(|(rect, _)| *rect)
    }

    /// 鼠标所在那一栏的第 `index` 个标签（悬停提示用）。
    pub(crate) fn terminal_hovered_tab(&self, index: usize) -> Option<&TermTab> {
        let at = self.terminal_group_under_pointer();
        let tabs = at.map_or(&self.terminal.tabs[..], |at| self.terminal.group_tabs(at));
        tabs.get(index)
    }

    /// `(x, y)` 在正在用的一栏里：悬停的链接和上报给程序的鼠标移动只看这一栏。
    pub(crate) fn terminal_group_focused_at(&self, x: f32, y: f32) -> bool {
        self.terminal_group_at(x, y) == Some(self.terminal.group_at)
    }

    /// 鼠标在第几栏里（见 [`Self::terminal_group_at`]）。
    pub(crate) fn terminal_group_under_pointer(&self) -> Option<usize> {
        let (x, y) = self.pointer.mouse;
        self.terminal_group_at(x, y)
    }

    /// 临时把第 `at` 栏换成正在用的一栏，运行 `f`，再换回原来的。
    ///
    /// 其间不整理空栏、不建等着的标签（见 [`App::sync_terminal`]），换回来以后再做。
    pub(crate) fn with_terminal_group<T>(&mut self, at: usize, f: impl FnOnce(&mut App) -> T) -> T {
        let back = self.terminal.group_at;
        let panel = self.terminal.panel_at;
        if at == back || at >= self.terminal.groups.len() {
            return f(self);
        }
        let borrowed = std::mem::replace(&mut self.terminal.borrowed, true);
        self.rotate_terminal_group(at);
        let result = f(self);
        self.rotate_terminal_group(back);
        self.terminal.borrowed = borrowed;
        self.terminal.panel_at = panel;
        if !borrowed && std::mem::take(&mut self.terminal.resync) {
            self.sync_terminal();
        }
        result
    }

    /// 把第 `at` 栏换成正在用的一栏，滚动条的显示状态跟着正在看的标签换。
    pub(in crate::app) fn rotate_terminal_group(&mut self, at: usize) {
        if at == self.terminal.group_at {
            return;
        }
        let area = [crate::app::ScrollArea::Terminal];
        if let Some(tab) = self.terminal.tab_mut() {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
        self.terminal.swap_group(at);
        if let Some(tab) = self.terminal.tab_mut() {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
    }

    /// 改用第 `at` 栏：焦点放进该栏正在看的标签；当前栏保持原有焦点。
    pub(in crate::app) fn focus_terminal_group(&mut self, at: usize) {
        if at >= self.terminal.group_count() || at == self.terminal.group_at {
            return;
        }
        if let Some(document) = self.terminal.editor_at(at) {
            self.focus_editor_terminal_document(document);
            return;
        }
        self.finish_terminal_rename(true);
        self.blur_terminal_tab();
        self.rotate_terminal_group(at);
        // 点下去的这一下还要按这一栏标签栏上的位置处理：换上它上一帧记下的按钮。
        if let Some(hits) = self.hits.terminal_groups.get(at) {
            self.hits.terminal.clone_from(hits);
        }
        self.terminal_activated();
        self.focus = Focus::Terminal;
        self.relayout();
        self.request_redraw();
    }

    /// 点在不在用的一栏上：先改用那一栏，再按平常处理这一下。
    pub(in crate::app) fn focus_terminal_group_at(&mut self, x: f32, y: f32) {
        if self.terminal_group_split_at(x, y).is_some() {
            return;
        }
        if let Some(at) = self.terminal_group_at(x, y) {
            self.focus_terminal_group(at);
        }
    }

    /// `(x, y)` 所在那一栏标签栏上的按钮：不在用的一栏取它上一帧记下的。
    pub(in crate::app) fn terminal_hits_at(&self, x: f32, y: f32) -> &[(Rect, HeaderHit)] {
        match self.terminal_group_at(x, y) {
            Some(at) if at != self.terminal.group_at || self.terminal.editor_at(at).is_some() => {
                self.hits
                    .terminal_groups
                    .get(at)
                    .map_or(&[][..], Vec::as_slice)
            }
            Some(_) => &self.hits.terminal,
            None => &[],
        }
    }

    /// 正在看的标签能不能移到新的一栏：这一栏里不止它一个标签，否则移过去原来那一栏就空了。
    pub(crate) fn terminal_movable_to_group(&self) -> bool {
        !self.terminal_in_editor() && self.terminal.tabs.len() > 1 && !self.terminal.crashed
    }

    /// 把正在看的标签移到正在用的一栏后面新加的一栏（标签和终端里的右键菜单、命令面板）：面板
    /// 在下方时加在右边，在右侧时加在下面。这一栏只有它一个标签时不动。
    pub(in crate::app) fn move_terminal_to_new_group(&mut self) {
        let Some(id) = self.terminal.tab().map(|tab| tab.id) else {
            return;
        };
        if self.terminal_movable_to_group() {
            let at = self.terminal.group_at + 1;
            self.move_terminal_tab(id, GroupDrop::New(at));
        }
    }

    /// 把编号为 `id` 的标签（连同拆分出的终端）挪到 `drop` 处，并改用那一栏。
    ///
    /// 原来那一栏空了时去掉。
    pub(in crate::app) fn move_terminal_tab(&mut self, id: u16, drop: GroupDrop) {
        let Some(mut from) = self.terminal.group_of(id) else {
            return;
        };
        if let Some(document) = self.terminal.editor_at(from) {
            self.move_editor_terminal_tab_to_panel(document, drop);
            return;
        }
        let to = match drop {
            GroupDrop::Into { group, .. } if group == from => return,
            GroupDrop::Into { group, .. } if group >= self.terminal.group_count() => return,
            GroupDrop::Into { group, .. } if self.terminal.editor_at(group).is_some() => return,
            GroupDrop::Into { group, .. } => group,
            GroupDrop::New(at) => {
                let at = at.min(self.terminal.group_count());
                self.terminal.insert_group(at);
                if at <= from {
                    from += 1;
                }
                at
            }
        };
        self.finish_terminal_rename(true);
        let tab = self.with_terminal_group(from, |app| {
            let index = app.terminal.tabs.iter().position(|tab| tab.id == id)?;
            app.blur_terminal_tab();
            if app.terminal.tabs.len() > 1 {
                app.animate_terminal_close(index);
            }
            let t = &mut app.terminal;
            let tab = t.tabs.remove(index);
            if t.active > index || t.active >= t.tabs.len() {
                t.active = t.active.saturating_sub(1);
            }
            Some(tab)
        });
        let Some(tab) = tab else {
            self.tidy_terminal_groups();
            return;
        };
        let area = [crate::app::ScrollArea::Terminal];
        if let Some(tab) = self.terminal.tab_mut() {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
        let slot = match drop {
            GroupDrop::Into { at, .. } => at,
            GroupDrop::New(_) => None,
        };
        self.terminal.place_tab(to, tab, slot);
        self.terminal.swap_group(to);
        if let Some(tab) = self.terminal.tab_mut() {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
        self.tidy_terminal_groups();
        self.terminal_activated();
        self.focus = Focus::Terminal;
        self.relayout();
        self.request_redraw();
    }

    /// 拖出标签栏的标签松开时放到哪里，以及标出的区域；放下后什么也不变时为 `None`。
    ///
    /// 落在别的栏的标签栏那一行：插在鼠标所在的位置，标出一道竖线。落在一栏中部：放进那一栏，
    /// 标出整栏。离一栏的两端（面板在下方时是左右，在右侧时是上下）不足它的三分之一：在那一边
    /// 拆出新的一栏，标出那一半；一栏里只有这一个标签时拖到它自己的边上不变。
    pub(crate) fn terminal_tab_drop(&self) -> Option<(GroupDrop, Rect)> {
        if !self.terminal_tab_dragged_out() {
            return None;
        }
        self.terminal_panel_drop_at(self.pointer.mouse, Some(self.terminal.group_at))
    }

    /// 面板中的放置目标；来自编辑器时不受原栏中只有一个标签的限制。
    pub(crate) fn terminal_panel_drop_at(
        &self,
        (x, y): (f32, f32),
        from: Option<usize>,
    ) -> Option<(GroupDrop, Rect)> {
        if !self.layout.terminal.contains(x, y)
            || self.overlay_covers(x, y)
            || self.docked_search_shown()
        {
            return None;
        }
        let cards = self.terminal_group_cards_in(self.layout.terminal);
        let group = cards.iter().position(|card| card.contains(x, y))?;
        let card = cards[group];
        let focused = Some(group) == from;
        let top = card.y + header_height(card);
        if y < top {
            return (!focused).then(|| self.terminal_tab_slot(group, card, x));
        }
        let column = self.terminal_right();
        let r = match column {
            false => (x - card.x) / card.w.max(1.0),
            true => (y - top) / (card.bottom() - top).max(1.0),
        };
        if r.min(1.0 - r) >= DROP_EDGE {
            return (!focused).then_some((GroupDrop::Into { group, at: None }, card));
        }
        if focused && self.terminal.group_tabs(group).len() < 2 {
            return None;
        }
        let before = r < 0.5;
        let half = |n: f32| (n / 2.0).round();
        let area = match (column, before) {
            (false, true) => Rect::new(card.x, card.y, half(card.w), card.h),
            (false, false) => Rect::new(card.right() - half(card.w), card.y, half(card.w), card.h),
            (true, true) => Rect::new(card.x, card.y, card.w, half(card.h)),
            (true, false) => Rect::new(card.x, card.bottom() - half(card.h), card.w, half(card.h)),
        };
        let at = if before { group } else { group + 1 };
        Some((GroupDrop::New(at), area))
    }

    /// 拖到第 `group` 栏（在 `card`）的标签栏那一行横坐标 `x` 处：插在哪个位置，以及那里的竖线。
    /// 这一栏是竖排的标签列表时放在最后，标出整栏。
    fn terminal_tab_slot(&self, group: usize, card: Rect, x: f32) -> (GroupDrop, Rect) {
        let hits = self
            .hits
            .terminal_groups
            .get(group)
            .map_or(&[][..], Vec::as_slice);
        let tabs: Vec<Rect> = (hits.iter())
            .filter_map(|(rect, hit)| matches!(hit, HeaderHit::Tab(_)).then_some(*rect))
            .collect();
        let (Some(first), Some(last)) = (tabs.first(), tabs.last()) else {
            return (GroupDrop::Into { group, at: None }, card);
        };
        let at = tabs.iter().filter(|r| r.x + r.w / 2.0 < x).count();
        let gap = super::strip::TAB_GAP;
        let line_x = tabs
            .get(at)
            .map_or(last.right() + gap / 2.0, |r| r.x - gap / 2.0);
        let line = Rect::new(line_x - 1.0, first.y, 2.0, first.h);
        (
            GroupDrop::Into {
                group,
                at: Some(at),
            },
            line,
        )
    }

    /// 去掉空了的栏（见 [`TerminalPanel::drop_empty_groups`]）；临时换进别的栏时不做。
    pub(in crate::app) fn tidy_terminal_groups(&mut self) {
        if self.terminal.borrowed {
            return;
        }
        if self.terminal.want.is_empty() && self.terminal.run_queue.is_empty() {
            while let Some((at, document)) =
                (self.terminal.groups.iter().enumerate()).find_map(|(at, group)| {
                    group
                        .editor
                        .filter(|_| self.terminal.group_tabs(at).is_empty())
                        .map(|document| (at, document))
                })
            {
                self.terminal.groups[at].editor = None;
                self.terminal.groups[at].shown = false;
                self.detach_terminal_editor_document(document);
            }
        }
        let area = [crate::app::ScrollArea::Terminal];
        if let Some(tab) = self.terminal.tab_mut() {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
        let dropped = self.terminal.drop_empty_groups();
        if let Some(tab) = self.terminal.tab_mut() {
            self.scroll.bars.exchange(&mut tab.scrollbars, &area);
        }
        if dropped {
            self.terminal_activated();
            self.relayout();
            self.request_redraw();
        }
    }

    /// 面板中相邻停靠栏之间的分隔线，不包含编辑器终端。
    pub(crate) fn terminal_group_dividers(&self) -> Vec<Rect> {
        let card = self.layout.terminal;
        let cards = self.terminal_group_cards_in(card);
        let mut panels = self.terminal.panel_groups();
        let Some(mut previous) = panels.next() else {
            return Vec::new();
        };
        panels
            .map(|at| {
                let line = match self.terminal_right() {
                    false => Rect::new(cards[previous].right(), card.y, DIVIDER, card.h),
                    true => Rect::new(card.x, cards[previous].bottom(), card.w, DIVIDER),
                };
                previous = at;
                line
            })
            .collect()
    }

    /// `(x, y)` 所在的分栏分隔线：第几条。可拖动的范围是两侧各放宽 [`GRIP`]；被浮层遮住时为
    /// `None`。
    pub(in crate::app) fn terminal_group_split_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.overlay_covers(x, y) {
            return None;
        }
        let column = self.terminal_right();
        self.terminal_group_dividers().iter().position(|line| {
            let grip = match column {
                false => Rect::new(line.x - GRIP, line.y, line.w + 2.0 * GRIP, line.h),
                true => Rect::new(line.x, line.y - GRIP, line.w, line.h + 2.0 * GRIP),
            };
            grip.contains(x, y)
        })
    }

    /// 正被悬停或拖动的分栏分隔线（见 [`Self::terminal_group_split_at`]）。
    pub(crate) fn active_terminal_group_split(&self) -> Option<usize> {
        match (self.pointer.hover, &self.pointer.drag) {
            (_, crate::app::Drag::TerminalGroupSplit { index }) => Some(*index),
            (Hover::TerminalGroupSplit(index), _) => Some(index),
            _ => None,
        }
    }

    /// 把第 `index` 条分栏分隔线拖到 `(x, y)`：两侧的栏重新分配它们合起来的大小，各自不小于
    /// [`GROUP_MIN`]，其余栏不变。
    pub(in crate::app) fn drag_terminal_group_split(&mut self, index: usize, (x, y): (f32, f32)) {
        let mut shares = self.terminal.panel_shares();
        if index + 1 >= shares.len() {
            return;
        }
        let column = self.terminal_right();
        let cards = group_cards(self.layout.terminal, &shares, column);
        let (start, end, at) = match column {
            false => (cards[index].x, cards[index + 1].right(), x),
            true => (cards[index].y, cards[index + 1].bottom(), y),
        };
        let size = end - start - DIVIDER;
        if size < GROUP_MIN * 2.0 {
            return;
        }
        let first = (at - start).clamp(GROUP_MIN, size - GROUP_MIN);
        let pair = shares[index] + shares[index + 1];
        shares[index] = pair * first / size;
        shares[index + 1] = pair - shares[index];
        self.set_terminal_group_shares(&shares);
    }

    /// 双击第 `index` 条分栏分隔线：两侧的栏平分它们合起来的大小。
    pub(in crate::app) fn reset_terminal_group_split(&mut self, index: usize) {
        let mut shares = self.terminal.panel_shares();
        if index + 1 >= shares.len() {
            return;
        }
        let half = (shares[index] + shares[index + 1]) / 2.0;
        shares[index] = half;
        shares[index + 1] = half;
        self.set_terminal_group_shares(&shares);
    }

    /// 按排列顺序设置各栏的份额 `shares`，并重新排版。
    fn set_terminal_group_shares(&mut self, shares: &[f32]) {
        for (group, &share) in self
            .terminal
            .groups
            .iter_mut()
            .filter(|group| group.editor.is_none())
            .zip(shares)
        {
            group.share = share;
        }
        self.relayout();
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests;
