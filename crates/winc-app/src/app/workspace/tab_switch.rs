//! 按最近使用的顺序切换标签页（`⌃Tab`）：按住修饰键期间显示列表，松开时切到选中的标签页。
//!
//! 列表只含前台窗格的标签页，按最近成为当前标签页的先后排列。从命令面板打开时没有按住的
//! 修饰键，按回车切换。

use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use super::*;
use crate::keymap::{self, Action, Context, Lookup};

/// 记录的最近使用的标签页数（个）；更早的不再参与排序，按标签栏的顺序排在后面。
///
/// 取自调参：远多于一屏能列出的行数，同时打开的标签页一般不超过这个数。
const RECENT_TABS: usize = 100;

/// 标签页切换的状态。
#[derive(Default)]
pub(crate) struct TabSwitch {
    /// 各文档最近一次成为当前标签页的先后，最近的在前。
    recent: Vec<DocumentId>,
    /// 打开中的列表；未打开时为 `None`。
    pub(crate) list: Option<SwitchList>,
}

/// 打开中的标签页列表。
pub(crate) struct SwitchList {
    /// 列出的文档，按最近使用的先后排列。
    pub(crate) rows: Vec<DocumentId>,
    /// 选中项在 `rows` 中的序号。
    pub(crate) selected: usize,
    /// 打开时按住的修饰键（不含 `Shift`）：全部松开时切换；没有时按回车切换。
    held: ModifiersState,
}

impl App {
    /// 记下当前标签页为最近使用的；列表打开期间不记录，选择过程不改变顺序。
    ///
    /// 由 [`App::relayout`] 调用：每帧绘制前以及切换标签页之后都会执行。
    pub(in crate::app) fn track_recent_tabs(&mut self) {
        if self.tab_switch.list.is_some() {
            return;
        }
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return;
        };
        let recent = &mut self.tab_switch.recent;
        if recent.first() == Some(&id) {
            return;
        }
        let editor = &self.editor;
        recent.retain(|&other| other != id && editor.document(other).is_some());
        recent.insert(0, id);
        recent.truncate(RECENT_TABS);
    }

    /// 打开标签页列表，或在已打开的列表中移动选中项；`forward` 为 `true` 时选向更早使用的。
    ///
    /// 打开时选中上一个使用的标签页（`forward` 为 `false` 时是最早使用的），快速按下并松开
    /// 即切换到上一个标签页。
    pub(in crate::app) fn switch_tab(&mut self, forward: bool) {
        if let Some(list) = &mut self.tab_switch.list {
            let n = list.rows.len();
            list.selected = if forward {
                (list.selected + 1) % n
            } else {
                (list.selected + n - 1) % n
            };
            self.request_redraw();
            return;
        }
        self.track_recent_tabs();
        let recent = &self.tab_switch.recent;
        let rank = |id: &DocumentId| recent.iter().position(|r| r == id).unwrap_or(usize::MAX);
        let mut rows: Vec<DocumentId> = self.editor.tabs().iter().map(|d| d.id).collect();
        // 稳定排序：没有记录的标签页保持标签栏中的顺序。
        rows.sort_by_key(rank);
        let n = rows.len();
        if n == 0 {
            return;
        }
        let selected = match (forward, n) {
            (_, 1) => 0,
            (true, _) => 1,
            (false, _) => n - 1,
        };
        let held = self.pointer.modifiers - ModifiersState::SHIFT;
        self.tab_switch.list = Some(SwitchList {
            rows,
            selected,
            held,
        });
        self.request_redraw();
    }

    /// 关闭标签页列表，切到选中的标签页；该标签页已关闭时只关闭列表。
    pub(in crate::app) fn accept_tab_switch(&mut self) {
        let Some(list) = self.tab_switch.list.take() else {
            return;
        };
        let id = list.rows.get(list.selected).copied();
        if let Some(index) = id.and_then(|id| self.editor.index_of(id)) {
            self.activate_tab(index);
        }
        self.request_redraw();
    }

    /// 关闭标签页列表，不切换。
    pub(in crate::app) fn close_tab_switch(&mut self) {
        if self.tab_switch.list.take().is_some() {
            self.request_redraw();
        }
    }

    /// 修饰键变化后调用：打开列表时按住的修饰键全部松开，即切到选中的标签页。
    pub(in crate::app) fn tab_switch_modifiers(&mut self) {
        let released = (self.tab_switch.list.as_ref()).is_some_and(|list| {
            !list.held.is_empty() && !self.pointer.modifiers.intersects(list.held)
        });
        if released {
            self.accept_tab_switch();
        }
    }

    /// 列表打开时处理按键 `event`，返回是否已处理。
    ///
    /// 回车切换，`Esc` 关闭；`Tab`、下方向键选向更早使用的，`Shift+Tab`、上方向键反向。其余
    /// 按键不交给下层，避免在选择期间改动文档。
    pub(in crate::app) fn tab_switch_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        if self.tab_switch.list.is_none() {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_tab_switch(),
            Key::Named(NamedKey::Enter) => self.accept_tab_switch(),
            Key::Named(NamedKey::ArrowDown) => self.switch_tab(true),
            Key::Named(NamedKey::ArrowUp) => self.switch_tab(false),
            Key::Named(NamedKey::Tab) => self.switch_tab(!self.pointer.modifiers.shift_key()),
            key => {
                if let Some(forward) = self.switch_key(key, event) {
                    self.switch_tab(forward);
                }
            }
        }
        true
    }

    /// 按键是否为切换标签页的快捷键：是时返回方向（`true` 为选向更早使用的），否则返回 `None`。
    ///
    /// 按编辑区的范围查找：列表打开期间，从哪个区域打开的都按同一组键移动。
    fn switch_key(&self, key: &Key, event: &winit::event::KeyEvent) -> Option<bool> {
        let base = match event.key_without_modifiers() {
            Key::Character(s) => s.to_lowercase(),
            _ => String::new(),
        };
        let stroke = self.stroke_of(key, &base, event.physical_key, event.location)?;
        let table = keymap::current();
        let Lookup::Run(actions) = keymap::lookup(&table, stroke, None, Context::Editor) else {
            return None;
        };
        actions.iter().find_map(|action| match action {
            Action::SwitchTab => Some(true),
            Action::SwitchTabBack => Some(false),
            _ => None,
        })
    }

    /// 列表打开时处理鼠标按下，返回是否已处理：点在某一行上切到该标签页，点在别处关闭列表。
    pub(in crate::app) fn tab_switch_mouse_down(&mut self) -> bool {
        if self.tab_switch.list.is_none() {
            return false;
        }
        let (x, y) = self.pointer.mouse;
        let hit = (self.hits.tab_switch.iter()).find(|(rect, _)| rect.contains(x, y));
        match hit.map(|&(_, row)| row) {
            Some(row) => {
                if let Some(list) = &mut self.tab_switch.list {
                    list.selected = row;
                }
                self.accept_tab_switch();
            }
            None => self.close_tab_switch(),
        }
        true
    }
}
