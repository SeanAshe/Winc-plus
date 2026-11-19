//! 编辑区的窗格：按左右或上下拆分、可以嵌套的一棵树，每个窗格有自己的标签。

/// 窗格编号，在同一个 `Editor` 内唯一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u32);

/// 拆分的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// 左右并排。
    Row,
    /// 上下排列。
    Column,
}

/// 窗格树的节点。
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// 一个窗格。
    Pane(PaneId),
    /// 拆成两半：`share` 是前一半（左边或上边）所占的比例。
    Split {
        /// 拆分的方向。
        axis: Axis,
        /// 前一半所占的比例，在 0 和 1 之间。
        share: f32,
        /// 前一半（左边或上边）。
        first: Box<Node>,
        /// 后一半（右边或下边）。
        second: Box<Node>,
    },
}

impl Node {
    /// 按从左到右、从上到下的顺序返回各个窗格。
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Pane(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    /// 把窗格 `at` 拆成两半，`new` 在后（`before` 为真时在前）；返回是否找到了 `at`。
    pub(crate) fn split(&mut self, at: PaneId, axis: Axis, new: PaneId, before: bool) -> bool {
        match self {
            Node::Pane(id) if *id == at => {
                let (a, b) = if before { (new, at) } else { (at, new) };
                *self = Node::Split {
                    axis,
                    share: 0.5,
                    first: Box::new(Node::Pane(a)),
                    second: Box::new(Node::Pane(b)),
                };
                true
            }
            Node::Pane(_) => false,
            Node::Split { first, second, .. } => {
                first.split(at, axis, new, before) || second.split(at, axis, new, before)
            }
        }
    }

    /// 去掉窗格 `pane`，它所在的拆分由另一半占满；返回是否去掉了。只剩一个窗格时不去掉。
    pub(crate) fn remove(&mut self, pane: PaneId) -> bool {
        let Node::Split { first, second, .. } = self else {
            return false;
        };
        if **first == Node::Pane(pane) {
            *self = std::mem::replace(second, Node::Pane(pane));
            return true;
        }
        if **second == Node::Pane(pane) {
            *self = std::mem::replace(first, Node::Pane(pane));
            return true;
        }
        first.remove(pane) || second.remove(pane)
    }

    /// 返回第 `index` 个拆分（按前序遍历）的比例的可变引用。
    pub fn share_mut(&mut self, index: usize) -> Option<&mut f32> {
        let mut seen = 0;
        self.find_share(index, &mut seen)
    }

    fn find_share(&mut self, index: usize, seen: &mut usize) -> Option<&mut f32> {
        let Node::Split {
            share,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        if *seen == index {
            return Some(share);
        }
        *seen += 1;
        match first.find_share(index, seen) {
            Some(found) => Some(found),
            None => second.find_share(index, seen),
        }
    }
}

/// 不在前台的窗格：它的标签和当前标签，按文档编号记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTabs {
    /// 窗格编号。
    pub id: PaneId,
    /// 标签，按标签栏中的顺序。
    pub tabs: Vec<crate::DocumentId>,
    /// 当前标签。
    pub active: Option<crate::DocumentId>,
}

/// 编辑区的全部窗格：前台窗格的标签是 `Editor::documents` 的前一段，其余窗格各自记着标签。
#[derive(Debug, Clone)]
pub struct Panes {
    /// 窗格的排列。
    pub tree: Node,
    /// 前台的窗格。
    pub focused: PaneId,
    /// 其余窗格。
    pub others: Vec<PaneTabs>,
    /// `Editor::documents` 中属于前台窗格的标签个数。
    pub(crate) shown: usize,
    next: u32,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            tree: Node::Pane(PaneId(0)),
            focused: PaneId(0),
            others: Vec::new(),
            shown: 0,
            next: 1,
        }
    }
}

impl Panes {
    /// 是否拆分过（不止一个窗格）。
    pub fn is_split(&self) -> bool {
        !self.others.is_empty()
    }

    /// 分配一个新的窗格编号。
    pub(crate) fn allocate(&mut self) -> PaneId {
        let id = PaneId(self.next);
        self.next += 1;
        id
    }

    /// 返回不在前台的窗格 `id` 的标签；是前台窗格或不存在时返回 `None`。
    pub fn other(&self, id: PaneId) -> Option<&PaneTabs> {
        self.others.iter().find(|p| p.id == id)
    }

    /// 文档 `doc` 是否是前台以外某个窗格的标签。
    pub fn elsewhere(&self, doc: crate::DocumentId) -> bool {
        self.others.iter().any(|p| p.tabs.contains(&doc))
    }
}

mod switch;

#[cfg(test)]
mod tests;
