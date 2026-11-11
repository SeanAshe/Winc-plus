//! 差异视图中只读的一侧：Vim 层在该侧文本的只读副本上移动、选择、复制与查找，选区写回差异
//! 视图；修改类命令因副本只读而被拒绝。

use ropey::Rope;
use winc_core::{Lines, Selection};
use winc_editor::diff_view::{DiffView, RIGHT, ReadSelection};
use winc_editor::{Document, DocumentId};

use super::{CaretShape, VimState};
use crate::app::{App, Focus};

/// 差异中只读一侧的只读副本。
pub(super) struct Side {
    /// 差异所在的标签。
    tab: DocumentId,
    /// 所在的一侧。
    side: usize,
    /// 创建副本时该侧的文本，用于发现差异视图换了文本。
    text: Rope,
    /// 文本与该侧共用的只读文档，其选区即该侧的选区。
    pub(super) doc: Document,
}

impl std::fmt::Debug for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Side")
            .field("tab", &self.tab)
            .field("side", &self.side)
            .finish_non_exhaustive()
    }
}

impl App {
    /// 返回当前标签的差异中只读光标所在的位置：（标签的序号，所在侧）；光标在可编辑的一侧或
    /// 没有光标时返回 `None`。
    pub(super) fn vim_side(&self) -> Option<(usize, usize)> {
        let index = self.editor.active?;
        let view = self.editor.documents[index].diff.as_deref()?;
        let (side, _) = view.read_caret().filter(|_| !view.editing)?;
        Some((index, side))
    }

    /// 光标在差异中只读的一侧时由 Vim 层执行应用的复制，可视模式随之结束；返回是否已执行。
    pub(in crate::app) fn vim_side_copy(&mut self) -> bool {
        let state = self.vim.as_ref();
        if self.focus != Focus::Editor || !state.is_some_and(|state| state.side.is_some()) {
            return false;
        }
        let keys = self.vim_equivalent(crate::keymap::Action::Copy);
        keys.is_some_and(|keys| {
            self.vim_feed(keys);
            true
        })
    }

    /// 副本 `held` 是否仍对应只读光标所在的一侧：同一个标签、同一侧，且该侧的文本没有换过。
    pub(super) fn vim_side_holds(&self, held: &Side) -> bool {
        self.vim_side().is_some_and(|(index, side)| {
            let tab = &self.editor.documents[index];
            let text = tab.diff.as_deref().map(|view| &view.side(side).text);
            tab.id == held.tab
                && side == held.side
                && text.is_some_and(|t| held.text.is_instance(t))
        })
    }

    /// 换成只读光标所在一侧的副本；光标不在只读的一侧时不留副本。
    ///
    /// 离开的副本留在 `state.parked` 中，回到该侧时沿用，其中的标记得以保留；所属的标签已经
    /// 关闭时丢弃。
    pub(super) fn vim_side_switch(&self, state: &mut VimState) {
        let left = state.side.take();
        let mut spare = state.parked.take();
        if let Some((index, side)) = self.vim_side() {
            let tab = &self.editor.documents[index];
            let text = tab.diff.as_deref().map(|view| &view.side(side).text);
            let fits = |held: &Side| {
                held.tab == tab.id
                    && held.side == side
                    && text.is_some_and(|t| held.text.is_instance(t))
            };
            state.side = match text {
                _ if spare.as_deref().is_some_and(fits) => spare.take(),
                Some(text) => Some(Box::new(Side {
                    tab: tab.id,
                    side,
                    text: text.clone(),
                    doc: Document::snapshot(tab.id, text.clone()),
                })),
                None => None,
            };
        }
        let parked = left.or(spare);
        state.parked = parked.filter(|held| self.editor.index_of(held.tab).is_some());
    }

    /// 把差异视图中只读一侧的选区取到副本上；鼠标改动选区之后两者不同。
    pub(super) fn vim_side_pull(&self, state: &mut VimState) {
        let Some(held) = &mut state.side else {
            return;
        };
        let index = self.editor.index_of(held.tab);
        let view = index.and_then(|index| self.editor.documents[index].diff.as_deref());
        let Some(read) = view.and_then(|view| view.read_selection) else {
            return;
        };
        let end = held.doc.text().end_pos();
        let selection = Selection::new(read.range.0.min(end), read.range.1.min(end));
        if held.doc.selection != selection {
            held.doc.collapse_selections();
            held.doc.selection = selection;
        }
    }

    /// 把副本的滚动位置设为该侧窗口顶部的行，返回该位置；Vim 层的滚动命令在其上增减。
    pub(super) fn vim_side_top(&self, held: &mut Side) -> f64 {
        let index = self.editor.index_of(held.tab);
        let top = index.map_or(0, |index| self.diff_top_line(index, held.side)) as f64;
        held.doc.scroll_line = top;
        top
    }

    /// Vim 层处理之后把副本的选区写回差异视图，并返回 Vim 层滚动的行数。
    pub(super) fn vim_side_push(&mut self, held: &Side, top: f64) -> f64 {
        let index = self.editor.index_of(held.tab);
        let view = index.and_then(|index| self.editor.documents[index].diff.as_deref_mut());
        if let Some(view) = view.filter(|view| !view.editing) {
            let selection = held.doc.selection;
            view.read_selection = Some(ReadSelection {
                side: held.side,
                range: (selection.anchor, selection.head),
            });
        }
        held.doc.scroll_line - top
    }

    /// 返回差异 `view` 中 Vim 模式的光标：所在侧、位置与形状；使用竖线光标时（关闭、插入
    /// 模式、没有光标）返回 `None`。
    ///
    /// 绘制期间差异视图不在标签中，因此由调用方给出。
    pub(crate) fn vim_diff_caret(&self, view: &DiffView) -> Option<(usize, usize, CaretShape)> {
        let state = self.vim.as_ref()?;
        if view.editing {
            let path = view.new.path.as_ref()?;
            let mut documents = self.editor.documents.iter();
            let doc =
                documents.find(|doc| doc.diff.is_none() && doc.path.as_ref() == Some(path))?;
            let own = state.doc == Some(doc.id) && state.side.is_none();
            let (pos, shape) = self.vim_caret_in(state, doc).filter(|_| own)?;
            return Some((RIGHT, pos, shape));
        }
        let held = state.side.as_deref()?;
        let (side, _) = view.read_caret()?;
        let fits = Some(held.tab) == self.editor.active().map(|tab| tab.id)
            && held.side == side
            && held.text.is_instance(&view.side(side).text);
        let (pos, shape) = self.vim_caret_in(state, &held.doc).filter(|_| fits)?;
        Some((side, pos, shape))
    }
}
