//! 侧栏的显示与隐藏、文件树的展开、滚动与定位，以及在文件树中新建文件和目录。
//!
//! 拖动项目见 `tree_drag.rs`，工具栏按钮的收起动画见 `tool_fold.rs`；滚动时固定在顶部的各级目录
//! 由 [`StickyRow`] 描述。

use winc_editor::{EXPAND_DIRS, Workspace};
use winc_i18n::t;

use super::*;
use crate::layout::tree_row_slot;

/// 文件树中正在输入名字的那一行：新建文件或目录，或给已有的项改名。
pub struct NameInput {
    /// 新建时所在的目录。
    pub dir: PathBuf,
    /// 新建的是文件还是目录。
    pub kind: EntryKind,
    /// 绘制在文件树的第几行（从 0 起）：新建时是目标目录子树的第一行（插入的行），
    /// 改名时是原来的那一行。
    pub index: usize,
    /// 这一行的缩进层级（从 0 起）。
    pub depth: usize,
    /// 名字输入框。
    pub field: TextField,
    /// 上一次提交失败的原因，显示在名字右边。
    pub error: Option<String>,
    /// 改名时为（原路径，该项是目录时是否已展开）；新建时为 `None`。
    pub rename: Option<(PathBuf, bool)>,
    /// 把文本或图片粘贴为新文件时要写入的内容；新建空文件、目录或改名时为 `None`。
    pub contents: Option<Vec<u8>>,
}

/// 返回文件树的最大滚动位置（逻辑像素）。
///
/// 末尾多留一行：滚到底时最后一行下方还有一行高度的空白。内容不足一屏时为负数。
pub(crate) fn tree_max_scroll(tree: Rect, rows: usize) -> f32 {
    (rows as f32 + 1.0) * TREE_ROW - tree.h
}

/// 滚动时固定在文件树顶部的目录层数上限（层，经验值）。
///
/// 实际层数还受 `App::tree_sticky_max` 限制：不超过文件树高度的 40%，避免固定行占去过多的显示空间。
const STICKY_MAX: usize = 7;

/// 固定在文件树顶部的一个目录。
///
/// 往下滚动时，顶部固定的各行是当前可见文件所在的各级目录；单击即可收起该目录，无需滚动回去查找。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StickyRow {
    /// 目录在文件树条目中的序号（从 0 起）。
    pub index: usize,
    /// 嵌套层级（从 0 起）。
    ///
    /// 未被向上推时，该层就是固定区域中从上数的第几行（从 0 起）。
    pub level: usize,
    /// 绘制位置。
    ///
    /// 该目录的最后几行即将滚出可见区域时，该目录会被下面的行向上推，可能有一部分
    /// 落在文件树之外。
    pub rect: Rect,
}

/// 文件树中某个纵坐标对应的槽位内容。
enum TreeSlot {
    /// 文件树条目，参数为条目序号（从 0 起）。
    Entry(usize),
    /// 新建时插进来输入名字的那一行。
    Inserted,
    /// 收起动画里正在收回的子行。
    Removed,
    /// 没有内容（空白）。
    Empty,
}

/// 侧栏的显示动画：从露出比例 `from` 开始，朝 `panel_visible` 对应的一端过渡。
#[derive(Debug, Clone, Copy)]
pub(in crate::app) struct SidebarMotion {
    /// 起始的露出比例：0 为隐藏，1 为完全展开。
    pub(in crate::app) from: f32,
    /// 动画的时钟。
    pub(in crate::app) clock: Clock,
}

/// 目录展开或收起的动画：子行从父行下方逐渐露出（或逐渐收回），下方的行随之移动，
/// 父行的箭头同时转向。
#[derive(Debug, Clone)]
pub struct TreeMotion {
    /// 展开或收起的目录是文件树的第几个条目（从 0 起）。
    ///
    /// 该目录之前的行不受影响，序号不变。
    pub parent: usize,
    /// 子行（连同展开着的子目录里的行）的行数。
    pub count: usize,
    /// 是否为展开；`false` 表示收起。
    pub expanding: bool,
    /// 收起时移除的子行，动画期间仍然绘制。
    ///
    /// 展开时为空，子行已经在文件树里。
    pub removed: Vec<Entry>,
    /// 动画的时钟。
    pub(in crate::app) clock: Clock,
}

impl TreeMotion {
    /// 返回子行已露出的比例：0 表示完全收起，1 表示完全展开。
    pub fn reveal(&self) -> f32 {
        let eased = self.clock.progress();
        if self.expanding { eased } else { 1.0 - eased }
    }

    /// 返回子行在绘制时占用的槽位范围 `[起, 止)`，按展开后的样子计数；收起时包括正在收回的行。
    pub fn block(&self) -> (usize, usize) {
        (self.parent + 1, self.parent + 1 + self.count)
    }

    /// 返回尚未显示的子行所占的高度（逻辑像素）：其下方的行向上收起相应高度。
    pub fn hidden(&self) -> f32 {
        (1.0 - self.reveal()) * self.count as f32 * TREE_ROW
    }
}

impl App {
    /// 返回侧栏露出的比例：0 为隐藏，1 为完全展开，动画中介于两者之间。
    pub(in crate::app) fn sidebar_shown(&self) -> f32 {
        let target = if self.panel_visible { 1.0 } else { 0.0 };
        let Some(motion) = self.sidebar_motion else {
            return target;
        };
        motion.clock.between(motion.from, target)
    }

    /// 用状态文件里记下的侧栏宽度（启动、新开窗口时）。
    pub(crate) fn restore_panel_width(&mut self) {
        if let Some(width) = self.state.panel_width {
            self.panel_width = width.max(crate::theme::MIN_PANEL_WIDTH);
        }
    }

    /// 把侧栏宽度记进状态文件：以后打开的窗口、下次启动都用这个宽度。
    pub(in crate::app) fn remember_panel_width(&mut self) {
        let width = self.panel_width.round();
        self.update_state(|state| state.panel_width = Some(width));
    }

    /// 侧栏是否处于显示状态（动画的目标状态）。
    pub(crate) fn sidebar_open(&self) -> bool {
        self.panel_visible
    }

    /// 启动时直接定下侧栏，不播放展开或收起动画。
    pub(crate) fn set_initial_sidebar(&mut self, visible: bool) {
        self.panel_visible = visible;
        self.sidebar_motion = None;
    }

    /// 带动画地显示或隐藏侧栏。
    ///
    /// 动画中途反向时从当前位置继续。
    pub(in crate::app) fn set_sidebar(&mut self, visible: bool) {
        if visible == self.panel_visible {
            return;
        }

        let from = self.sidebar_shown();
        self.panel_visible = visible;
        self.sidebar_motion = self
            .clock(SIDEBAR_ANIMATION)
            .map(|clock| SidebarMotion { from, clock });
        self.relayout();
        self.request_redraw();
    }

    /// 窗口建好时按现场恢复侧栏的开关：启动时恢复的目录在窗口创建之前打开。
    ///
    /// 侧栏直接到位，没有收起动画。
    pub(in crate::app) fn restore_sidebar_for_window(&mut self) {
        let session = self
            .editor
            .root()
            .and_then(|root| self.state.session_for(root));
        if session.is_some_and(|session| session.sidebar_hidden) {
            self.panel_visible = false;
            self.sidebar_motion = None;
        }
    }

    /// 焦点是否在侧栏里（文件树、搜索面板或提交视图）。
    pub(in crate::app) fn focus_in_sidebar(&self) -> bool {
        matches!(
            self.focus,
            Focus::Tree
                | Focus::GlobalQuery
                | Focus::GlobalReplace
                | Focus::GlobalInclude
                | Focus::GlobalExclude
                | Focus::Results
                | Focus::CommitMessage
                | Focus::CommitList
        )
    }

    /// 展开或收起文件树里第 `index` 行的目录，带动画。
    ///
    /// `expand` 为 `None` 时切换。
    pub(in crate::app) fn set_dir_expanded(&mut self, index: usize, expand: Option<bool>) {
        let Some(ws) = &mut self.editor.workspace else {
            return;
        };
        let Some(entry) = ws.rows().get(index) else {
            return;
        };
        let expanding = expand.unwrap_or(!entry.expanded);

        if entry.kind != EntryKind::Dir || entry.expanded == expanding {
            return;
        }
        self.change_dir(index, expanding, |ws| ws.toggle(index));
    }

    /// 递归展开或收起文件树里第 `index` 行的目录（`⌥` 单击），带动画：收起的目录展开到底，展开
    /// 的目录连同各级子目录一起收起。目录太多只展开了一部分时提示。
    pub(in crate::app) fn toggle_dir_recursively(&mut self, index: usize) {
        let Some(entry) = self
            .editor
            .workspace
            .as_ref()
            .and_then(|w| w.rows().get(index))
        else {
            return;
        };
        if entry.kind != EntryKind::Dir {
            return;
        }
        let expanding = !entry.expanded;
        let mut limited = false;
        self.change_dir(index, expanding, |ws| {
            if expanding {
                limited = ws.expand_recursively(index);
            } else {
                ws.collapse_recursively(index);
            }
        });
        if limited {
            self.flash(t!("workspace-tree-expand-limit", count = EXPAND_DIRS));
        }
    }

    /// 用 `change` 展开（`expanding` 为真）或收起第 `index` 行的目录，播放子行露出或收起的动画。
    fn change_dir(&mut self, index: usize, expanding: bool, change: impl FnOnce(&mut Workspace)) {
        let Some(ws) = &mut self.editor.workspace else {
            return;
        };
        let Some(entry) = ws.rows().get(index) else {
            return;
        };

        // 收起前先保存子行：`index + 1..entry.end` 是它展开后的全部子孙。
        let removed: Vec<Entry> = if expanding {
            Vec::new()
        } else {
            ws.rows()[index + 1..entry.end].to_vec()
        };
        let before = ws.rows().len();
        change(ws);
        let count = ws.rows().len().abs_diff(before);

        self.tree_motion = self.clock(TREE_ANIMATION).map(|clock| TreeMotion {
            parent: index,
            count,
            expanding,
            removed,
            clock,
        });
        self.request_redraw();
    }

    /// 让文件树、提交视图的改动列表和历史提交的文件列表按设置合并或逐级显示连续的单子目录。
    pub(in crate::app) fn apply_compact_folders(&mut self) {
        let compact = self.config.compact_folders != Some(false);
        if let Some(ws) = &mut self.editor.workspace {
            ws.set_compact(compact);
        }
        self.commit.set_compact(compact);
        self.set_log_list_compact(compact);
        self.request_redraw();
    }

    /// 返回第 `index` 行里横坐标 `x` 处合并显示的目录；不在其中时返回 `None`。
    pub(in crate::app) fn tree_segment_at(&self, index: usize, x: f32) -> Option<PathBuf> {
        self.tree_segments
            .iter()
            .find(|(row, left, right, _)| *row == index && (*left..*right).contains(&x))
            .map(|(.., path)| path.clone())
    }

    /// 按鼠标现在的位置更新悬停的合并目录；同一行内换到另一级时悬停目标不变，要在这里请求重画。
    pub(in crate::app) fn track_tree_segment(&mut self) {
        if self.tree_segments.is_empty() && self.tree_hover_segment.is_none() {
            return;
        }
        let (x, y) = self.mouse;
        let segment = matches!(self.hover_at(x, y), Hover::TreeRow(_))
            .then(|| self.tree_row_at(y))
            .flatten()
            .and_then(|row| self.tree_segment_at(row, x));
        if segment != self.tree_hover_segment {
            self.tree_hover_segment = segment;
            self.request_redraw();
        }
    }

    /// 切换侧栏的显示（`⌘B`、`⌘1` 和顶栏按钮）。
    ///
    /// 显示时焦点交给文件树或搜索输入框，隐藏时还给编辑区；焦点在终端里时保持不变。
    pub(in crate::app) fn toggle_sidebar(&mut self) {
        self.set_sidebar(!self.panel_visible);
        self.focus = match (self.panel_visible, self.sidebar_view) {
            (true, SidebarView::Files) => Focus::Tree,
            (true, SidebarView::Search) => Focus::GlobalQuery,
            (true, SidebarView::Commit) => self.commit_view_focus(),
            (false, _) if self.focus == Focus::Terminal => Focus::Terminal,
            (false, _) => Focus::Editor,
        };
    }

    /// 返回可能需要展开显示全名的文件树行序号：鼠标悬停在文件树的行或分隔条上时，
    /// 鼠标纵坐标所在的行。
    ///
    /// 动画中和拖动时不展开，返回 `None`。
    pub(crate) fn overflow_candidate(&self) -> Option<usize> {
        let idle = self.sidebar_motion.is_none() && matches!(self.drag, Drag::None);
        // 拖动文件时没有悬停目标，放不下的名称同样要显示，才看得到、放得进那几级目录。
        let pointing = matches!(self.hover, Hover::TreeRow(_) | Hover::Splitter)
            || self.tree_dragging().is_some_and(|drag| drag.active);
        let (_, y) = self.mouse;
        let tree = self.layout.tree;

        if !(idle && pointing && (tree.y..tree.bottom()).contains(&y)) {
            return None;
        }

        // 展开、收起的动画中其他行在移动；正在展开、收起的这一行不动，继续显示完整名称。
        let row = self.tree_row_at(y)?;
        match &self.tree_motion {
            Some(motion) if motion.parent != row => None,
            _ => Some(row),
        }
    }

    /// 文件树中 `y` 位置对应第几个条目；被固定在顶部的目录覆盖的区域算作该目录。
    pub(in crate::app) fn tree_row_at(&self, y: f32) -> Option<usize> {
        if let Some(sticky) = self.tree_sticky_at(y) {
            return Some(sticky.index);
        }
        match self.tree_slot_at(y) {
            TreeSlot::Entry(index) => Some(index),
            _ => None,
        }
    }

    /// 固定在顶部、位于 `y` 处的目录。
    ///
    /// 被推上去的层与其上一层重叠时，绘制在上方的是上一层。
    pub(in crate::app) fn tree_sticky_at(&self, y: f32) -> Option<StickyRow> {
        if y < self.layout.tree.y {
            return None;
        }
        self.tree_sticky()
            .into_iter()
            .find(|s| (s.rect.y..s.rect.bottom()).contains(&y))
    }

    /// 纵坐标 `y` 是否在文件树最后一行下方的空白区域内。
    pub(in crate::app) fn tree_blank_at(&self, y: f32) -> bool {
        self.tree_sticky_at(y).is_none() && matches!(self.tree_slot_at(y), TreeSlot::Empty)
    }

    /// 返回文件树中纵坐标 `y` 处的槽位，不考虑固定在顶部的目录。
    ///
    /// 排布规则与绘制（`view/workspace/sidebar.rs` 的 `draw_tree`）相同：目录展开或收起的动画中，
    /// 子行位置不变，只露出已显示的那一段，下方的行向上收起尚未显示的高度；正在收回的子行
    /// 已不在文件树中，对应 [`TreeSlot::Removed`]；新建时插入的那一行也不是条目，
    /// 对应 [`TreeSlot::Inserted`]。
    fn tree_slot_at(&self, y: f32) -> TreeSlot {
        let Some(ws) = self.editor.workspace.as_ref() else {
            return TreeSlot::Empty;
        };
        let rows = ws.rows().len();
        let content_y = y - self.layout.tree.y + self.tree_scroll;

        if content_y < 0.0 {
            return TreeSlot::Empty;
        }

        let motion = self.tree_motion.as_ref().filter(|m| m.parent < rows);
        let shift = motion.map_or(0.0, |m| {
            let (_, end) = m.block();
            let hidden = m.hidden();
            if content_y >= end as f32 * TREE_ROW - hidden {
                hidden
            } else {
                0.0
            }
        });
        let v = ((content_y + shift) / TREE_ROW).floor() as usize;

        // 新建时插入的那一行不是条目，该行下方的行在数据中的序号要减 1。
        let v = match self.inserted_row() {
            Some(at) if v == at => return TreeSlot::Inserted,
            Some(at) if v > at => v - 1,
            _ => v,
        };

        let index = match motion {
            Some(m) if !m.removed.is_empty() && v >= m.block().0 => {
                if v < m.block().1 {
                    return TreeSlot::Removed;
                }
                v - m.removed.len()
            }
            _ => v,
        };
        if index < rows {
            TreeSlot::Entry(index)
        } else {
            TreeSlot::Empty
        }
    }

    /// 返回第 `index` 个条目所在行的上沿在窗口中的纵坐标（逻辑像素），排布规则与绘制相同。
    pub(crate) fn tree_row_top(&self, index: usize) -> f32 {
        let rows = self
            .editor
            .workspace
            .as_ref()
            .map_or(0, |ws| ws.rows().len());
        let motion = self.tree_motion.as_ref().filter(|m| m.parent < rows);
        let mut v = index;
        if let Some(m) = motion.filter(|m| index > m.parent) {
            v += m.removed.len();
        }
        if self.inserted_row().is_some_and(|at| v >= at) {
            v += 1;
        }
        let block = motion.map(TreeMotion::block);
        let hidden = motion.map_or(0.0, TreeMotion::hidden);
        tree_row_slot(self.layout.tree, self.tree_scroll, v, block, hidden)
            .0
            .y
    }

    /// 返回文件树顶部最多固定的层数：不超过树高的 40%，也不超过 [`STICKY_MAX`]。
    fn tree_sticky_max(&self) -> usize {
        ((self.layout.tree.h * 0.4 / TREE_ROW).floor() as usize).min(STICKY_MAX)
    }

    /// 返回固定在文件树顶部的目录，外层在前。
    ///
    /// 逐层向下查找：已经固定了 n 层时，取紧挨着这些层下方露出的那一行；该行在第 n 层的上级
    /// 目录就是下一个固定的目录；如果该行本身是已展开的目录，且上沿已滚到固定区域之下，则
    /// 固定该行自己。目录最后一行的下沿滚到固定位置之下时，该目录被向上推；被推上去的这一层
    /// 之下不再固定其他目录。
    pub(crate) fn tree_sticky(&self) -> Vec<StickyRow> {
        let Some(ws) = self.editor.workspace.as_ref() else {
            return Vec::new();
        };
        let rows = ws.rows();
        let tree = self.layout.tree;
        let mut stack: Vec<StickyRow> = Vec::new();

        while stack.len() < self.tree_sticky_max() {
            let level = stack.len();
            let top = tree.y + level as f32 * TREE_ROW;
            // 露出的是输入名字的那一行时，取它所在的目录；是正在收回的子行时，取被收起的目录。
            let first = match self.tree_slot_at(top) {
                TreeSlot::Entry(index) => index,
                TreeSlot::Inserted => match self.name_input.as_ref() {
                    Some(input) if input.index > 0 => input.index - 1,
                    _ => break,
                },
                TreeSlot::Removed => match &self.tree_motion {
                    Some(m) => m.parent,
                    None => break,
                },
                TreeSlot::Empty => break,
            };

            let entry = &rows[first];
            // 顺着上级往上走到第 `level` 层，不往回扫一整段兄弟行。
            let index = if entry.depth > level {
                let mut i = first;
                while rows[i].depth > level
                    && let Some(parent) = rows[i].parent
                {
                    i = parent;
                }
                if rows[i].depth != level {
                    break;
                }
                i
            } else if entry.depth == level
                && entry.kind == EntryKind::Dir
                && entry.expanded
                && self.tree_row_top(first) < top - 0.5
            {
                first
            } else {
                break;
            };

            let count = rows[index].end - index - 1;
            if count == 0 {
                break;
            }
            let bottom = self.tree_row_top(index + count) + TREE_ROW;
            let y = top.min(bottom - TREE_ROW);
            stack.push(StickyRow {
                index,
                level,
                rect: Rect::new(tree.x, y, tree.w, TREE_ROW),
            });
            if y < top {
                break;
            }
        }

        stack
    }

    /// 点击固定在顶部的目录时先调用：滚动文件树，使该目录的实际行（条目 `index`）停在该目录被
    /// 固定的位置（第 `level` 层，从 0 起），画面看起来没有移动；之后再展开或收起。
    pub(super) fn settle_sticky(&mut self, index: usize, level: usize) {
        let target = self.layout.tree.y + level as f32 * TREE_ROW;
        self.tree_scroll = (self.tree_scroll + self.tree_row_top(index) - target).max(0.0);
    }

    /// 返回新建时插入文件树的那一行所在的槽位序号。
    ///
    /// 改名是在原来那一行上进行，不插入行，此时返回 `None`。
    fn inserted_row(&self) -> Option<usize> {
        self.name_input
            .as_ref()
            .filter(|e| e.rename.is_none())
            .map(|e| e.index)
    }

    /// 返回第 `index` 个条目绘制在第几个槽位：位于新建时插入的那一行之后的条目向下挪一格。
    pub(crate) fn tree_slot_of(&self, index: usize) -> usize {
        match self.inserted_row() {
            Some(at) if index >= at => index + 1,
            _ => index,
        }
    }

    /// 滚动文件树，使第 `index` 个条目可见；已经可见时不动，`center` 为 `true` 时把该条目放到中间。
    ///
    /// 条目滚到上方时，该条目的各级上级目录会固定在顶部，因此要停在这些固定行之下，不能被盖住。
    pub(in crate::app) fn scroll_tree_to(&mut self, index: usize, center: bool) {
        let depth = self
            .editor
            .workspace
            .as_ref()
            .and_then(|ws| ws.rows().get(index))
            .map_or(0, |e| e.depth);
        self.scroll_tree_to_slot(self.tree_slot_of(index), depth, center);
    }

    /// 滚动文件树，使第 `slot` 个槽位可见，该槽位位于第 `depth` 层。
    ///
    /// 新建时插入的那一行还没有条目序号，用槽位定位。
    fn scroll_tree_to_slot(&mut self, slot: usize, depth: usize, center: bool) {
        let tree_h = self.layout.tree.h;
        let y = slot as f32 * TREE_ROW;
        let above = depth.min(self.tree_sticky_max()) as f32 * TREE_ROW;
        let visible = y >= self.tree_scroll + above && y + TREE_ROW <= self.tree_scroll + tree_h;

        self.tree_scroll = match (visible, center) {
            (true, _) => self.tree_scroll,
            (false, true) => (y - tree_h / 2.0).max(0.0),
            (false, false) => {
                scroll_to_show(self.tree_scroll, y - above, y + TREE_ROW, tree_h).max(0.0)
            }
        };
    }

    /// 在文件树里展开并选中当前文件。
    pub(super) fn reveal_active(&mut self) {
        if let Some(path) = self.editor.active().and_then(|d| d.path.clone()) {
            self.reveal_path(&path);
        }
    }

    /// 在文件树里展开并选中 `path`，滚到能看见的地方。
    pub(in crate::app) fn reveal_path(&mut self, path: &Path) {
        let Some(ws) = &mut self.editor.workspace else {
            return;
        };

        ws.reveal(path);
        let depth = ws.selected.map(|index| ws.rows()[index].depth);

        // 侧栏显示搜索时文件树不在屏幕上，只展开、选中，切回文件树时再滚动到该处。
        if self.sidebar_view == SidebarView::Search {
            return;
        }

        if let Some(index) = ws.selected {
            self.scroll_tree_to(index, true);
        }

        // 被展开的行可能在很深的层级，横向也滚动到能看到它的位置。
        if let Some(depth) = depth {
            self.reveal_tree_column(depth);
        }
    }

    /// 横向滚动到能看见第 `depth` 级缩进，右边至少留出 40 逻辑像素（经验值）显示名字。
    fn reveal_tree_column(&mut self, depth: usize) {
        let tree = self.layout.tree;
        let indent = depth as f32 * TREE_INDENT;

        if indent < self.tree_scroll_x {
            self.tree_scroll_x = indent;
        } else if indent + TREE_ROW > self.tree_scroll_x + tree.w - 40.0 {
            self.tree_scroll_x = indent + TREE_ROW - tree.w + 40.0;
        }

        self.tree_scroll_x = self.tree_scroll_x.max(0.0);
    }

    /// 在文件树中插入一行用于输入名字，`folder` 为 `true` 时新建目录，否则新建文件。
    ///
    /// 该行插在目标目录子树的第一行，缩进比目标目录深一级；目标目录尚未展开时先展开。
    /// 没有打开目录时在状态栏提示。
    pub(in crate::app) fn start_new_entry(&mut self, folder: bool) {
        let kind = if folder {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        self.open_new_entry(kind, TextField::default(), None);
    }

    /// 与新建文件相同，插入一行用于输入名字，新文件的内容为 `contents`（粘贴的文本或图片）。
    ///
    /// 输入框中填好建议的名字 `name`，并选中扩展名之前的部分，直接键入即替换主名；
    /// 按 `↩` 用该名字新建。
    pub(in crate::app) fn start_pasted_entry(&mut self, name: String, contents: Vec<u8>) {
        let mut field = TextField::default();
        field.set_text(&name);
        let stem = name.rfind('.').filter(|&dot| dot > 0).unwrap_or(name.len());
        field.set_caret(0, false);
        field.set_caret(stem, true);
        self.open_new_entry(EntryKind::File, field, Some(contents));
    }

    /// 插入输入名字的那一行：新建 `kind`，输入框为 `field`，新文件的内容为 `contents`。
    fn open_new_entry(&mut self, kind: EntryKind, field: TextField, contents: Option<Vec<u8>>) {
        self.goto_input = None;
        let Some(ws) = self.editor.workspace.as_mut() else {
            self.flash(winc_i18n::t!("workspace-open-folder-first"));
            return;
        };

        let dir = ws.target_dir();
        let at = (dir != ws.root).then(|| ws.row_of(&dir)).flatten();
        let (index, depth) = match at {
            Some(at) => (at + 1, ws.rows()[at].depth + 1),
            None if dir == ws.root => (0, 0),
            // 目标目录来自选中的那一行（它自己或它的上一级），一定在文件树中。
            None => return,
        };

        // 目标目录还处于收起状态时先展开：走带动画的路径，使输入行随子行一起露出。
        if let Some(at) = at.filter(|&at| !ws.rows()[at].expanded) {
            self.set_dir_expanded(at, Some(true));
        }

        self.tree_scroll_x = 0.0;
        self.name_input = Some(NameInput {
            dir,
            kind,
            index,
            depth,
            field,
            error: None,
            rename: None,
            contents,
        });
        self.focus = Focus::Prompt;
        self.field_preedit.clear();
        self.scroll_tree_to_slot(index, depth, false);
        self.relayout();
        self.request_redraw();
    }

    /// 关闭文件树中输入名字的那一行，不新建也不改名。
    pub(in crate::app) fn close_name_input(&mut self) {
        self.name_input = None;
        self.field_preedit.clear();
        // 焦点不能留在已经不存在的输入框上；跳转条同样使用 `Focus::Prompt`，
        // 跳转条打开时保持焦点不变。
        if self.focus == Focus::Prompt && self.goto_input.is_none() {
            self.focus = Focus::Tree;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 其他输入部件要获得焦点时调用（跳转条、查找条、快速打开、搜索面板、设置）。
    ///
    /// 文件树中正在输入的名字与点击其他位置时一样先提交；提交失败（名字不合法）则放弃。
    /// 跳转条与名字输入共用 `Focus::Prompt`，按键只会输入到其中一个里，因此两者不能同时打开。
    pub(in crate::app) fn finish_name_input(&mut self) {
        if self.name_input.is_some() {
            self.submit_name_input();
        }
        if self.name_input.is_some() {
            self.close_name_input();
        }
    }

    /// 返回文件树中输入名字那一行在窗口中的矩形；不在可见范围内时返回 `None`。
    pub(in crate::app) fn name_input_rect(&self) -> Option<Rect> {
        let entry = self.name_input.as_ref()?;
        let tree = self.layout.tree;
        let y = tree.y + entry.index as f32 * TREE_ROW - self.tree_scroll;
        let rect = Rect::new(tree.x, y, tree.w, TREE_ROW);
        (rect.bottom() > tree.y && rect.y < tree.bottom()).then_some(rect)
    }

    /// 提交输入的名字（新建或改名）。
    ///
    /// 失败时输入行保留，原因显示在名字右边。
    pub(in crate::app) fn submit_name_input(&mut self) {
        let Some(mut entry) = self.name_input.take() else {
            return;
        };
        if entry.rename.is_some() {
            self.submit_rename(entry);
            return;
        }
        let input = entry.field.text().to_owned();

        if input.trim().is_empty() {
            self.close_name_input();
            return;
        }

        let created = self
            .editor
            .workspace
            .as_mut()
            .map(|ws| match &entry.contents {
                Some(contents) => ws.create_file(&entry.dir, &input, contents),
                None => ws.create(&entry.dir, &input, entry.kind),
            });

        match created {
            Some(Ok(path)) => {
                if let Some(ws) = &self.editor.workspace
                    && let Some(index) = ws.selected
                {
                    self.scroll_tree_to(index, false);
                }

                self.focus = if entry.kind == EntryKind::Dir {
                    Focus::Tree
                } else {
                    Focus::Editor
                };

                if entry.kind == EntryKind::File {
                    self.open_path(&path);
                }

                // 一次可能创建多级目录（例如 `a/b/c.rs`），文件表从新建的最上一级开始更新。
                let top = input
                    .trim()
                    .trim_matches('/')
                    .split('/')
                    .next()
                    .map(|first| entry.dir.join(first));
                self.picker_touched(top.into_iter().collect());
            }
            Some(Err(e)) => {
                entry.error = Some(e.to_string());
                self.name_input = Some(entry);
                self.focus = Focus::Prompt;
            }
            None => self.flash(winc_i18n::t!("workspace-open-folder-first")),
        }

        self.relayout();
        self.request_redraw();
    }

    /// 在工具栏的「⋯」下方弹出系统菜单，列出收起的工具。
    pub(in crate::app) fn open_tree_tools_menu(&mut self) {
        let Some(more) = self.tree_more else {
            return;
        };
        self.open_button_menu(
            crate::app::input::context_menu::MenuButton::TreeMore,
            crate::app::input::context_menu::MenuTarget::TreeTools,
            crate::app::input::context_menu::menu_below(more),
        );
    }

    /// 执行文件树工具栏上的工具 `tool`。
    pub(in crate::app) fn run_tree_tool(&mut self, tool: TreeTool) {
        match tool {
            TreeTool::NewFile => self.start_new_entry(false),
            TreeTool::NewFolder => self.start_new_entry(true),
            TreeTool::Refresh => {
                if let Some(ws) = &mut self.editor.workspace {
                    ws.refresh();
                    self.tree_motion = None;
                    self.clamp_tree_scroll();
                }

                self.picker_invalidate();
            }
            TreeTool::CollapseAll => {
                if let Some(ws) = &mut self.editor.workspace {
                    ws.collapse_all();
                    self.tree_motion = None;
                    self.tree_scroll = 0.0;
                    self.tree_scroll_x = 0.0;
                }
            }
            TreeTool::Locate => {
                if self.editor.active.is_none() {
                    self.flash(winc_i18n::t!("workspace-no-open-file"));
                    return;
                }

                self.focus = Focus::Tree;
                self.reveal_active();
            }
        }

        self.request_redraw();
    }

    /// 行数变化之后（刷新、删除、切回文件树）把纵向滚动位置限制回有效范围。
    ///
    /// 绘制时也会限制（那里还要计入动画）；这里先限制一次，使下一帧绘制之前的点击和悬停
    /// 按正确的行计算。
    pub(in crate::app) fn clamp_tree_scroll(&mut self) {
        let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
        self.tree_scroll = self.tree_scroll.clamp(0.0, self.tree_max_scroll(rows));
    }

    /// 与自由函数 [`tree_max_scroll`] 相同，但结果不小于 0。
    pub(super) fn tree_max_scroll(&self, rows: usize) -> f32 {
        tree_max_scroll(self.layout.tree, rows).max(0.0)
    }

    /// 返回横向最大滚动位置（逻辑像素）：最宽的一行超出文件树宽度的部分。
    ///
    /// 滑块按同一个宽度计算，因此滑块拖到头时正好滚到头。
    pub(in crate::app) fn tree_max_scroll_x(&self) -> f32 {
        (self.tree_content_w - self.layout.tree.w).max(0.0)
    }

    /// 内容变化之后把横向滚动位置限制回有效范围；目录展开、收起和刷新后调用。
    pub(crate) fn clamp_tree_scroll_x(&mut self) {
        self.tree_scroll_x = self.tree_scroll_x.clamp(0.0, self.tree_max_scroll_x());
    }
}
