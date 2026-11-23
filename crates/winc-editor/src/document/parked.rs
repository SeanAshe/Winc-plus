//! 同一文档在其他窗格里的视图状态：光标、选区、滚动位置和自动换行，随文本修改移动。

use super::*;
use crate::editing::cursors::{CursorMemory, Extra};
use crate::editing::folding::Wrap;
use crate::panes::PaneId;

/// 文档在一个窗格里的视图状态。
#[derive(Debug, Default)]
pub(crate) struct View {
    selection: Selection,
    goal_cell: Option<usize>,
    extra: Extra,
    cursor_memory: CursorMemory,
    grown: Vec<(Selection, Selection)>,
    scroll_line: f64,
    scroll_x: f32,
    wrap: Option<Wrap>,
}

/// 文档在各窗格里的视图状态：文档当前的字段属于 `live`，其余窗格的收在 `views` 里。
#[derive(Debug, Default)]
pub(crate) struct Parked {
    views: Vec<(PaneId, View)>,
    /// 当前字段所属的窗格；刚打开、或所属的窗格不再有这个文档时为 `None`。
    live: Option<PaneId>,
}

impl Parked {
    /// 是否没有保存任何窗格的视图状态。
    pub(crate) fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// 文本即将被修改：从 `start` 起的 `removed` 个字符换成 `inserted` 个字符，`lines` 是修改
    /// 开始的行和增加的行数（减少时为负）。
    pub(crate) fn edit(
        &mut self,
        start: usize,
        removed: usize,
        inserted: usize,
        lines: (usize, isize),
    ) {
        let end = start + removed;
        let moved = |pos: usize| {
            if pos < start {
                pos
            } else if pos < end {
                start
            } else {
                pos - removed + inserted
            }
        };
        let (first, delta) = lines;
        for (_, view) in &mut self.views {
            view.selection =
                Selection::new(moved(view.selection.anchor), moved(view.selection.head));
            view.extra = Extra::default();
            view.grown.clear();
            if (first as f64) < view.scroll_line {
                view.scroll_line = (view.scroll_line + delta as f64).max(first as f64);
            }
        }
    }
}

impl Parked {
    /// 文本修改以后，按改动重新计算各窗格保存的断行（见 `Wrap::edit`）。
    pub(crate) fn edit_wraps(
        &mut self,
        text: &impl winc_core::Lines,
        lines: (usize, usize, usize),
        whole: &crate::editing::long_lines::WholeLines,
    ) {
        for (_, view) in &mut self.views {
            if let Some(wrap) = &mut view.wrap {
                wrap.edit(text, lines, whole);
            }
        }
    }

    /// 文本修改并重新计算断行以后，各窗格保存的断行记为与文档版本 `revision` 一致。
    pub(crate) fn wraps_follow(&mut self, revision: u64) {
        for (_, view) in &mut self.views {
            if let Some(wrap) = &mut view.wrap {
                wrap.revision = revision;
            }
        }
    }
}

impl Document {
    /// 让文档显示在窗格 `pane` 里：当前的视图状态属于另一个窗格时先存入那个窗格，再换上
    /// `pane` 保存的；`pane` 没有保存时沿用 `from` 的副本，都没有时停在开头。当前的视图状态
    /// 不属于任何窗格时，`pane` 和 `from` 都没有保存就直接接过来。
    pub(crate) fn show_in(&mut self, pane: PaneId, from: Option<PaneId>) {
        let has = |p: &Parked, id: PaneId| p.views.iter().any(|(v, _)| *v == id);
        match self.parked.live {
            Some(live) if live == pane => return,
            Some(live) => {
                self.park(live);
                self.unpark(pane, from);
            }
            None if has(&self.parked, pane) || from.is_some_and(|f| has(&self.parked, f)) => {
                self.unpark(pane, from);
            }
            None => {}
        }
        self.parked.live = Some(pane);
    }

    /// 把当前的视图状态存入窗格 `pane`，覆盖它原先保存的。
    fn park(&mut self, pane: PaneId) {
        let view = View {
            selection: self.selection,
            goal_cell: self.goal_cell,
            extra: std::mem::take(&mut self.extra),
            cursor_memory: std::mem::take(&mut self.cursor_memory),
            grown: std::mem::take(&mut self.grown),
            scroll_line: self.scroll_line,
            scroll_x: self.scroll_x,
            wrap: self.folds.take_wrap(),
        };
        self.wrap_wanted = None;
        self.parked.views.retain(|(id, _)| *id != pane);
        self.parked.views.push((pane, view));
    }

    /// 换上窗格 `pane` 保存的视图状态；它没有保存时沿用 `from` 保存的副本（光标和滚动位置），
    /// 都没有时停在开头。
    fn unpark(&mut self, pane: PaneId, from: Option<PaneId>) {
        let at = self.parked.views.iter().position(|(id, _)| *id == pane);
        let view = match at {
            Some(at) => self.parked.views.remove(at).1,
            None => {
                let copy =
                    from.and_then(|from| self.parked.views.iter().find(|(id, _)| *id == from));
                copy.map_or_else(View::default, |(_, v)| View {
                    selection: v.selection,
                    goal_cell: v.goal_cell,
                    scroll_line: v.scroll_line,
                    scroll_x: v.scroll_x,
                    ..View::default()
                })
            }
        };
        let end = self.text.end_pos();
        self.selection = view.selection.clamped(end);
        self.goal_cell = view.goal_cell;
        self.extra = view.extra;
        self.cursor_memory = view.cursor_memory;
        self.grown = view.grown;
        let last = self.text.line_count().saturating_sub(1) as f64;
        self.scroll_line = view.scroll_line.clamp(0.0, last.max(0.0));
        self.scroll_x = view.scroll_x;
        self.folds.put_wrap(view.wrap);
    }

    /// 复制窗格 `from` 的视图状态给窗格 `pane`，不含自动换行的结果，之后它从这里开始。
    pub(crate) fn copy_view(&mut self, from: PaneId, pane: PaneId) {
        let parked = (self.parked.live != Some(from))
            .then(|| self.parked.views.iter().find(|(id, _)| *id == from))
            .flatten();
        let view = match parked {
            Some((_, v)) => View {
                selection: v.selection,
                goal_cell: v.goal_cell,
                scroll_line: v.scroll_line,
                scroll_x: v.scroll_x,
                ..View::default()
            },
            None => View {
                selection: self.selection,
                goal_cell: self.goal_cell,
                scroll_line: self.scroll_line,
                scroll_x: self.scroll_x,
                ..View::default()
            },
        };
        self.parked.views.retain(|(id, _)| *id != pane);
        self.parked.views.push((pane, view));
    }

    /// 丢掉窗格 `pane` 保存的视图状态（窗格关掉了，或文档不再是它的标签）。
    pub(crate) fn forget_view(&mut self, pane: PaneId) {
        self.parked.views.retain(|(id, _)| *id != pane);
        if self.parked.live == Some(pane) {
            self.parked.live = None;
        }
    }

    /// 让当前的视图状态属于前台窗格 `pane`（刚打开、刚加进这个窗格的文档）。
    pub(crate) fn claim_view(&mut self, pane: PaneId) {
        self.parked.live = Some(pane);
    }

    /// 接过 `old`（同一个标签换成了新读取的文档）保存的各窗格视图状态。
    pub(crate) fn adopt_parked(&mut self, old: &mut Document) {
        self.parked = std::mem::take(&mut old.parked);
    }
}
