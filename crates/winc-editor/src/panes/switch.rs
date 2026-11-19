//! 拆分、切换和合并窗格：标签换成另一个窗格的，各文档换上它在那个窗格里的视图状态。

use super::*;
use crate::{Document, DocumentId, Editor};

impl Editor {
    /// 前台窗格的标签，按标签栏中的顺序。
    pub fn tabs(&self) -> &[Document] {
        &self.documents[..self.panes.shown]
    }

    /// 前台窗格的标签个数。
    pub fn tab_count(&self) -> usize {
        self.panes.shown
    }

    /// 返回窗格 `pane` 的标签（文档编号，按标签栏中的顺序）和当前标签。
    pub fn pane_tabs(&self, pane: PaneId) -> (Vec<DocumentId>, Option<DocumentId>) {
        if pane == self.panes.focused {
            let tabs = self.tabs().iter().map(|d| d.id).collect();
            return (tabs, self.active().map(|d| d.id));
        }
        self.panes
            .other(pane)
            .map_or((Vec::new(), None), |p| (p.tabs.clone(), p.active))
    }

    /// 把前台窗格按 `axis` 拆开，新窗格在后（右边或下边），打开当前标签，光标和滚动位置相同，
    /// 并成为前台。没有当前标签时不拆，返回 `None`。
    pub fn split(&mut self, axis: Axis) -> Option<PaneId> {
        let doc = self.active()?.id;
        self.split_with(axis, false, doc, self.panes.focused)
    }

    /// 把前台窗格按 `axis` 拆开，新窗格在后（`before` 为真时在前），只打开文档 `doc`，光标和
    /// 滚动位置取它在窗格 `from` 里的，并成为前台。`doc` 不存在时不拆，返回 `None`。
    pub fn split_with(
        &mut self,
        axis: Axis,
        before: bool,
        doc: DocumentId,
        from: PaneId,
    ) -> Option<PaneId> {
        let index = self.index_of(doc)?;
        let new = self.panes.allocate();
        let focused = self.panes.focused;
        self.panes.tree.split(focused, axis, new, before);
        self.documents[index].copy_view(from, new);
        self.panes.others.push(PaneTabs {
            id: new,
            tabs: vec![doc],
            active: Some(doc),
        });
        self.focus_pane(new);
        Some(new)
    }

    /// 把前台窗格换成 `pane`；`pane` 已在前台或不存在时返回 `false`。
    pub fn focus_pane(&mut self, pane: PaneId) -> bool {
        let Some(at) = self.panes.others.iter().position(|p| p.id == pane) else {
            return false;
        };
        let old = self.panes.focused;
        let leaving = PaneTabs {
            id: old,
            tabs: self.tabs().iter().map(|d| d.id).collect(),
            active: self.active().map(|d| d.id),
        };
        let entering = std::mem::replace(&mut self.panes.others[at], leaving);
        self.enter(entering);
        true
    }

    /// 让窗格 `entering` 到前台：它的标签排到 `documents` 的最前面，换上它的视图状态。
    fn enter(&mut self, entering: PaneTabs) {
        let mut rest = std::mem::take(&mut self.documents);
        let mut front = Vec::with_capacity(rest.len());
        for id in &entering.tabs {
            if let Some(at) = rest.iter().position(|d| d.id == *id) {
                front.push(rest.remove(at));
            }
        }
        self.panes.shown = front.len();
        self.panes.focused = entering.id;
        front.extend(rest);
        self.documents = front;
        // 只有也开在别的窗格里的文档需要换视图状态。
        for doc in &mut self.documents[..self.panes.shown] {
            doc.show_in(entering.id, None);
        }
        let active = entering.active.and_then(|id| self.index_of(id));
        self.active = active
            .filter(|&at| at < self.panes.shown)
            .or((self.panes.shown > 0).then_some(0));
    }

    /// 让只在其他窗格里打开的第 `index` 个文档也成为前台窗格的标签（排在最后），返回它的标签
    /// 序号；已是前台窗格的标签时直接返回 `index`。
    pub fn bring(&mut self, index: usize) -> usize {
        let from = (self.documents.get(index)).and_then(|doc| {
            (self.panes.others.iter())
                .find(|p| p.tabs.contains(&doc.id))
                .map(|p| p.id)
        });
        self.bring_from(index, from)
    }

    /// 让文档 `doc` 成为前台窗格的标签，光标和滚动位置取它在窗格 `from` 里的；返回它的标签
    /// 序号，`doc` 不存在时返回 `None`。
    pub fn adopt_tab(&mut self, doc: DocumentId, from: PaneId) -> Option<usize> {
        let index = self.index_of(doc)?;
        Some(self.bring_from(index, Some(from)))
    }

    fn bring_from(&mut self, index: usize, from: Option<PaneId>) -> usize {
        let shown = self.panes.shown;
        if index < shown {
            return index;
        }
        let mut doc = self.documents.remove(index);
        doc.show_in(self.panes.focused, from);
        self.documents.insert(shown, doc);
        self.panes.shown += 1;
        shown
    }

    /// 前台窗格没有标签、又不是唯一的窗格时去掉它，前台换成它后面的窗格（没有时换成前面的）；
    /// 返回是否去掉了。
    pub fn drop_empty_pane(&mut self) -> bool {
        if self.panes.shown != 0 || !self.panes.is_split() {
            return false;
        }
        let gone = self.panes.focused;
        let order = self.panes.tree.panes();
        let at = order.iter().position(|p| *p == gone).unwrap_or(0);
        let next = order.get(at + 1).or(order.get(at.wrapping_sub(1))).copied();
        let Some(next) = next.and_then(|n| self.panes.others.iter().position(|p| p.id == n)) else {
            return false;
        };
        self.panes.tree.remove(gone);
        let entering = self.panes.others.remove(next);
        for doc in &mut self.documents {
            doc.forget_view(gone);
        }
        self.enter(entering);
        true
    }

    /// 把前台以外的窗格 `pane` 中的文档 `doc` 去掉；窗格因此没有标签时连窗格一起去掉。
    ///
    /// 只调整窗格，不关闭文档：调用方确认文档仍是某个窗格的标签。
    pub fn remove_from_pane(&mut self, pane: PaneId, doc: DocumentId) {
        let Some(at) = self.panes.others.iter().position(|p| p.id == pane) else {
            return;
        };
        let other = &mut self.panes.others[at];
        let Some(slot) = other.tabs.iter().position(|id| *id == doc) else {
            return;
        };
        other.tabs.remove(slot);
        if other.active == Some(doc) {
            other.active = other.tabs.get(slot).or(other.tabs.last()).copied();
        }
        if other.tabs.is_empty() {
            self.panes.others.remove(at);
            self.panes.tree.remove(pane);
        }
        if let Some(document) = self.documents.iter_mut().find(|d| d.id == doc) {
            document.forget_view(pane);
        }
    }

    /// 合并全部窗格：其他窗格的标签依次加到前台窗格的后面（已有的不重复）。
    pub fn unsplit(&mut self) {
        let others = std::mem::take(&mut self.panes.others);
        for pane in &others {
            for id in &pane.tabs {
                if let Some(index) = self.index_of(*id).filter(|&i| i >= self.panes.shown) {
                    let mut doc = self.documents.remove(index);
                    doc.show_in(self.panes.focused, Some(pane.id));
                    self.documents.insert(self.panes.shown, doc);
                    self.panes.shown += 1;
                }
            }
        }
        for doc in &mut self.documents {
            for pane in &others {
                doc.forget_view(pane.id);
            }
        }
        self.panes.tree = Node::Pane(self.panes.focused);
    }

    /// 按窗格树和各窗格的标签恢复拆分：`panes` 中每一项是（窗格编号，标签，当前标签）。
    ///
    /// 第一个窗格成为前台；标签中已关闭的文档略去，没有任何标签的窗格不恢复。
    pub fn restore_panes(&mut self, tree: Node, panes: Vec<PaneTabs>) {
        let ids = tree.panes();
        let mut panes: Vec<PaneTabs> = (panes.into_iter())
            .filter(|p| ids.contains(&p.id))
            .map(|mut p| {
                p.tabs.retain(|id| self.index_of(*id).is_some());
                p
            })
            .filter(|p| !p.tabs.is_empty())
            .collect();
        let mut tree = tree;
        for id in &ids {
            if !panes.iter().any(|p| p.id == *id) {
                tree.remove(*id);
            }
        }
        let Some(first) = tree.panes().first().copied() else {
            return;
        };
        let Some(at) = panes.iter().position(|p| p.id == first) else {
            return;
        };
        let entering = panes.remove(at);
        self.panes.next = ids.iter().map(|p| p.0 + 1).max().unwrap_or(1);
        self.panes.tree = tree;
        self.panes.others = panes;
        // 恢复时各窗格的视图状态都从开头开始。
        self.enter(entering);
        // 不属于任何窗格的文档（现场之外恢复的备份）放进前台窗格。
        let orphans: Vec<DocumentId> = (self.documents[self.panes.shown..].iter())
            .map(|d| d.id)
            .filter(|id| !self.panes.elsewhere(*id))
            .collect();
        for id in orphans {
            if let Some(index) = self.index_of(id) {
                self.bring(index);
            }
        }
    }
}
