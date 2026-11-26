//! 终端与编辑器标签之间的移动：会话仍由终端分栏持有，编辑器只管理标签和窗格的位置。

use winc_editor::DocumentId;

use super::groups::{GroupDrop, TermGroup};
use super::*;
use crate::app::workspace::panes::{PaneRects, TabDrop};

impl App {
    /// 编辑器中的终端标签所在的终端栏。
    pub(in crate::app) fn editor_terminal_group(&self, document: DocumentId) -> Option<usize> {
        self.terminal
            .groups
            .iter()
            .position(|group| group.editor == Some(document))
    }

    /// 当前终端位于编辑器标签里。
    pub(crate) fn terminal_in_editor(&self) -> bool {
        self.terminal.editor_document().is_some()
    }

    /// 在编辑器窗格中正在显示的终端标签所占的区域。
    pub(crate) fn editor_terminal_pane(&self, document: DocumentId) -> Option<PaneRects> {
        if self.panes.rects.is_empty() {
            return (self.editor.active().is_some_and(|doc| doc.id == document)).then_some(
                PaneRects {
                    id: self.editor.panes.focused,
                    whole: self.editor_card(),
                    tabs: self.layout.tabs,
                    editor: self.layout.editor,
                },
            );
        }
        self.panes
            .rects
            .iter()
            .find(|pane| {
                let active = if pane.id == self.editor.panes.focused {
                    self.editor.active().map(|doc| doc.id)
                } else {
                    self.editor
                        .panes
                        .other(pane.id)
                        .and_then(|pane| pane.active)
                };
                active == Some(document)
            })
            .copied()
    }

    /// 编辑器终端的内容卡片；向上多出的部分沿用终端拆分的标签栏补偿，不重复绘制标签栏。
    pub(crate) fn editor_terminal_card(&self, document: DocumentId) -> Rect {
        let body = if self.editor.tabs().iter().any(|doc| doc.id == document) {
            self.layout.editor
        } else {
            let pane = self
                .editor
                .panes
                .others
                .iter()
                .find(|pane| pane.tabs.contains(&document));
            let Some(body) = pane
                .and_then(|pane| self.panes.rects.iter().find(|rects| rects.id == pane.id))
                .map(|rects| rects.editor)
            else {
                return Rect::default();
            };
            body
        };
        lifted(body)
    }

    /// 返回当前窗格中新的编辑器终端的网格大小（见 [`App::grid_size`]）。
    fn editor_terminal_size(&self) -> Size {
        self.grid_size(lifted(self.layout.editor))
    }

    /// 鼠标位于某个可见终端的内容或面板标签栏上。
    pub(in crate::app) fn terminal_contains(&self, x: f32, y: f32) -> bool {
        self.terminal_group_at(x, y).is_some()
    }

    /// 按各编辑器窗格的当前标签更新终端可见性，隐藏标签不消耗帧额度。
    pub(in crate::app) fn sync_terminal_editor_visibility(&mut self) {
        let active = self.editor.active().map(|doc| doc.id);
        for group in &mut self.terminal.groups {
            group.shown = group.editor.is_some_and(|document| {
                active == Some(document)
                    || self
                        .editor
                        .panes
                        .others
                        .iter()
                        .any(|pane| pane.active == Some(document))
            });
        }
    }

    /// 编辑区中的终端标签显示终端的名字（[`TermTab::label`]）：自定名称、程序设的标题或前台
    /// 进程变化后调用；返回是否有标签的名字变了。
    pub(in crate::app) fn sync_terminal_editor_titles(&mut self) -> bool {
        let mut changed = false;
        for at in 0..self.terminal.group_count() {
            let Some(document) = self.terminal.editor_at(at) else {
                continue;
            };
            let Some(tab) = self.terminal.group_tabs(at).first() else {
                continue;
            };
            let Some(doc) = self.editor.document_mut(document) else {
                continue;
            };
            let name = tab.label();
            if doc.title.as_deref() != Some(name) {
                doc.title = Some(name.to_owned());
                changed = true;
            }
        }
        changed
    }

    /// 按编辑区的当前标签调整焦点：焦点在编辑区、当前标签是终端时把按键交给该终端；焦点在
    /// 终端、而编辑区当前显示的已不是该终端时，焦点回到编辑区。
    ///
    /// 结果只取决于当前标签和焦点，重复调用结果相同，因此排版（[`App::relayout`]）时调用，
    /// 切换、关闭、移动标签之后都会经过这里。
    pub(in crate::app) fn sync_editor_terminal_focus(&mut self) {
        if self.terminal.borrowed() {
            return;
        }
        if self.focus == Focus::Editor {
            if !self.activate_editor_terminal() {
                self.leave_editor_terminal();
            }
        } else if self.focus.in_terminal()
            && (self.terminal.editor_document())
                .is_some_and(|id| self.editor.active().is_none_or(|doc| doc.id != id))
        {
            self.focus = Focus::Editor;
            self.leave_editor_terminal();
        }
        self.sync_terminal_editor_visibility();
    }

    /// 将当前编辑器终端换入终端运行上下文，不改变真实焦点。
    pub(in crate::app) fn bind_editor_terminal(&mut self) -> bool {
        let Some(document) = self
            .editor
            .active()
            .filter(|doc| doc.terminal)
            .map(|doc| doc.id)
        else {
            return false;
        };
        let Some(at) = self.editor_terminal_group(document) else {
            return false;
        };
        if at != self.terminal.group_at {
            self.blur_terminal_tab();
            self.rotate_terminal_group(at);
        }
        true
    }

    /// 激活编辑器终端标签：按键仍交给原有的终端会话。
    pub(in crate::app) fn activate_editor_terminal(&mut self) -> bool {
        if !self.bind_editor_terminal() {
            return false;
        }
        self.focus = Focus::Terminal;
        self.terminal_activated();
        true
    }

    /// 离开编辑器终端时释放该终端的焦点；有停靠栏时换回上一次使用的面板上下文。
    pub(in crate::app) fn leave_editor_terminal(&mut self) {
        if !self.terminal_in_editor() {
            return;
        }
        self.blur_terminal_tab();
        let panel = self
            .terminal
            .panel_groups()
            .find(|&at| at == self.terminal.panel_at)
            .or_else(|| self.terminal.panel_groups().next());
        if let Some(at) = panel {
            self.rotate_terminal_group(at);
        }
    }

    /// 激活指定的编辑器终端及其所属窗格，不把会话复制到别的窗格。
    pub(in crate::app) fn focus_editor_terminal_document(&mut self, document: DocumentId) {
        let pane = if self.editor.tabs().iter().any(|doc| doc.id == document) {
            Some(self.editor.panes.focused)
        } else {
            self.editor
                .panes
                .others
                .iter()
                .find(|pane| pane.tabs.contains(&document))
                .map(|pane| pane.id)
        };
        let Some(pane) = pane else { return };
        self.focus_pane(pane);
        if let Some(index) = self.editor.index_of(document) {
            self.activate_tab(index);
        }
    }

    /// 保证有停靠栏，返回上一次使用的栏；只有编辑器终端时新加一个空的面板栏。
    pub(in crate::app) fn ensure_terminal_panel_group(&mut self) -> usize {
        let t = &mut self.terminal;
        let at = t
            .panel_groups()
            .find(|&at| at == t.panel_at)
            .or_else(|| t.panel_groups().next());
        if let Some(at) = at {
            return at;
        }
        let at = t.groups.len();
        t.groups.push(TermGroup::empty(1.0));
        t.panel_at = at;
        at
    }

    /// 把正在看的面板终端移到当前编辑器窗格，shell 和拆分状态不变。
    pub(in crate::app) fn move_terminal_to_editor(&mut self) {
        if self.terminal_in_editor() {
            return;
        }
        let Some(id) = self.terminal.tab().map(|tab| tab.id) else {
            return;
        };
        let drop = TabDrop::Into {
            pane: self.editor.panes.focused,
            at: None,
        };
        self.move_terminal_tab_to_editor(id, drop);
    }

    /// 在当前编辑器窗格新建一个终端标签，`shell` 为 `None` 时用默认 shell；不打开终端面板。
    ///
    /// 新终端排进待建的队列，辅助进程建好后从面板的栏移到编辑区（见 `App::sync_terminal`）。
    pub(in crate::app) fn new_editor_terminal(&mut self, shell: Option<String>) {
        if !self.terminal_enabled() {
            return;
        }
        if self.terminal.crashed {
            self.restart_terminal_tab();
        }
        let drop = TabDrop::Into {
            pane: self.editor.panes.focused,
            at: None,
        };
        let group = self.ensure_terminal_panel_group();
        let size = self.editor_terminal_size();
        let t = &mut self.terminal;
        t.want_shells.resize(t.want.len(), None);
        t.want_shells.push(shell);
        t.want_into.resize(t.want.len(), None);
        t.want_into.push(None);
        t.want_editor.resize(t.want.len(), None);
        t.want_editor.push(Some((drop, size)));
        t.want_groups.resize(t.want.len(), t.group_at);
        t.want_groups.push(group);
        t.want.push(Vec::new());
        t.want_active = None;
        self.start_terminal();
        self.relayout();
        self.request_redraw();
    }

    /// 把面板标签 `id` 放进指定编辑器窗格或边缘拆分目标，不发送关闭或新建 shell 的消息。
    pub(in crate::app) fn move_terminal_tab_to_editor(&mut self, id: u16, drop: TabDrop) {
        let Some(from) = self
            .terminal
            .group_of(id)
            .filter(|&at| self.terminal.editor_at(at).is_none())
        else {
            return;
        };
        self.finish_terminal_rename(true);
        let tab = self.with_terminal_group(from, |app| {
            let index = app.terminal.tabs.iter().position(|tab| tab.id == id)?;
            app.blur_terminal_tab();
            let tab = app.terminal.tabs.remove(index);
            if app.terminal.active > index || app.terminal.active >= app.terminal.tabs.len() {
                app.terminal.active = app.terminal.active.saturating_sub(1);
            }
            Some(tab)
        });
        let Some(tab) = tab else { return };
        self.make_room_for_editor();
        let index = self.editor.new_terminal(tab.label());
        let document = self.editor.documents[index].id;
        if self.terminal.groups.is_empty() {
            self.terminal.groups.push(TermGroup::empty(1.0));
        }
        self.terminal.groups.push(TermGroup {
            tabs: vec![tab],
            editor: Some(document),
            shown: true,
            ..TermGroup::empty(1.0)
        });
        self.place_terminal_editor_document(document, drop);
        self.tidy_terminal_groups();
        if self.terminal.panel_tab_count() == 0 && self.terminal.want.is_empty() {
            self.set_terminal_open(false);
        }
        self.activate_editor_terminal();
        self.relayout();
        self.remember_session();
        self.request_redraw();
    }

    /// 把刚建的终端标签 `document` 放到 `drop` 指定的位置并切换过去：放进当前窗格时挪到指定的
    /// 序号，其余（其他窗格、边缘拆分）与拖动文件标签相同。
    fn place_terminal_editor_document(&mut self, document: DocumentId, drop: TabDrop) {
        match drop {
            TabDrop::Into { pane, at } if pane == self.editor.panes.focused => {
                if let (Some(index), Some(at)) = (self.editor.index_of(document), at) {
                    let to = at.min(self.editor.tab_count().saturating_sub(1));
                    self.editor.move_document(index, to);
                }
                if let Some(index) = self.editor.index_of(document) {
                    self.activate_tab(index);
                }
            }
            _ => self.drop_tabs(&[document], document, drop, false),
        }
    }

    /// 将当前编辑器终端移回面板。
    pub(in crate::app) fn move_terminal_to_panel(&mut self) {
        let document = self
            .editor
            .active()
            .filter(|doc| doc.terminal)
            .map(|doc| doc.id)
            .or_else(|| self.terminal.editor_document());
        if let Some(document) = document {
            self.move_editor_terminal_to_panel(document);
        }
    }

    /// 将编辑器标签 `document` 的终端移回上一次使用的面板栏。
    pub(in crate::app) fn move_editor_terminal_to_panel(&mut self, document: DocumentId) {
        let group = self.ensure_terminal_panel_group();
        self.move_editor_terminal_tab_to_panel(document, GroupDrop::Into { group, at: None });
    }

    /// 将编辑器终端放进面板中的标签位置或新拆出的栏，移除标签但不关闭会话。
    pub(in crate::app) fn move_editor_terminal_tab_to_panel(
        &mut self,
        document: DocumentId,
        drop: GroupDrop,
    ) {
        let Some(from) = self.editor_terminal_group(document) else {
            return;
        };
        self.finish_terminal_rename(true);
        let tab = self.with_terminal_group(from, |app| {
            app.blur_terminal_tab();
            (!app.terminal.tabs.is_empty()).then(|| app.terminal.tabs.remove(0))
        });
        let Some(tab) = tab else { return };
        self.terminal.groups[from].editor = None;
        self.terminal.groups[from].shown = false;
        self.detach_terminal_editor_document(document);
        let (to, slot) = match drop {
            GroupDrop::Into { group, at } => (group, at),
            GroupDrop::New(at) => {
                self.terminal.insert_group(at);
                (at.min(self.terminal.groups.len() - 1), None)
            }
        };
        self.terminal.place_tab(to, tab, slot);
        self.rotate_terminal_group(to);
        self.tidy_terminal_groups();
        self.set_terminal_open(true);
        self.terminal.drag_opened = false;
        self.focus = Focus::Terminal;
        self.terminal_activated();
        self.relayout();
        self.remember_session();
        self.request_redraw();
    }

    /// 去掉终端的编辑器标签及空窗格；调用前会话已经移走或结束，不发送 PTY 消息。
    pub(in crate::app) fn detach_terminal_editor_document(&mut self, document: DocumentId) {
        self.editor.close_everywhere(document);
        self.tabs.marked.retain(|id| *id != document);
        self.drop_empty_pane();
        self.forget_closed_panes();
    }

    /// 关闭编辑器标签前结束该标签持有的终端；文件关闭流程负责去掉文档本身。
    pub(in crate::app) fn close_editor_terminal_session(&mut self, document: DocumentId) {
        let Some(at) = self.editor_terminal_group(document) else {
            return;
        };
        self.terminal.cancel_group_wants(at);
        let ids = self.with_terminal_group(at, |app| {
            let ids: Vec<u16> = app
                .terminal
                .tabs
                .iter()
                .flat_map(TermTab::panes)
                .map(|tab| tab.id)
                .collect();
            if let Some(client) = &app.terminal.client {
                for &tab in &ids {
                    client.send(&ToHelper::Close { tab });
                }
            }
            app.terminal.tabs.clear();
            ids
        });
        self.terminal.groups[at].editor = None;
        self.terminal.groups[at].shown = false;
        for id in ids {
            self.forget_terminal_tab(id);
        }
        self.tidy_terminal_groups();
        if self.terminal.tab_count() == 0 && self.terminal.want.is_empty() {
            self.terminal.client = None;
            self.terminal.ready = false;
            self.terminal.crashed = false;
        }
    }

    /// 关闭指定编辑器终端标签前确认其中正在运行的程序，不影响其他终端。
    pub(in crate::app) fn confirm_editor_terminal_tabs(
        &mut self,
        documents: &[DocumentId],
    ) -> bool {
        if self.window.handle.is_none() || self.config.terminal_confirm_close == Some(false) {
            return false;
        }
        let busy = self.terminal.groups.iter().enumerate().any(|(at, group)| {
            group.editor.is_some_and(|id| documents.contains(&id))
                && self
                    .terminal
                    .group_tabs(at)
                    .iter()
                    .flat_map(TermTab::panes)
                    .any(|tab| tab.busy().is_some())
        });
        busy && self.confirm_terminal_quit(CloseRequest::Tabs(documents.to_vec()))
    }

    /// 面板终端拖入编辑器的目标，与文件标签使用相同的边缘拆分和标签插入位置。
    pub(crate) fn terminal_editor_drop(&self) -> Option<(TabDrop, Rect)> {
        if !self.terminal_tab_dragged_out() {
            return None;
        }
        let (x, y) = self.pointer.mouse;
        let (pane, edge) = self.pane_drop_at((x, y))?;
        if y < pane.editor.y {
            let (at, marker) = self.tab_slot_at(pane, x);
            return Some((
                TabDrop::Into {
                    pane: pane.id,
                    at: Some(at),
                },
                marker,
            ));
        }
        match edge {
            Some((axis, before)) => Some((
                TabDrop::Split {
                    pane: pane.id,
                    axis,
                    before,
                },
                self.drop_area(pane.id, edge)?,
            )),
            None => Some((
                TabDrop::Into {
                    pane: pane.id,
                    at: None,
                },
                pane.whole,
            )),
        }
    }

    /// 只有终端标签的拖动才能放进终端面板，文件标签继续使用编辑器的放置目标。
    pub(crate) fn editor_terminal_panel_drop(
        &self,
        documents: &[DocumentId],
    ) -> Option<(GroupDrop, Rect)> {
        if documents.is_empty()
            || !documents
                .iter()
                .all(|id| self.editor.document(*id).is_some_and(|doc| doc.terminal))
        {
            return None;
        }
        self.terminal_panel_drop_at(self.pointer.mouse, None)
    }

    /// 拖动编辑器终端到窗口底边时临时展开面板，允许把最后一个终端直接拖回。
    pub(in crate::app) fn reveal_terminal_drag_panel(&mut self, documents: &[DocumentId]) {
        if self.terminal.open
            || documents.is_empty()
            || !documents
                .iter()
                .all(|id| self.editor.document(*id).is_some_and(|doc| doc.terminal))
        {
            return;
        }
        let (x, y) = self.pointer.mouse;
        let edge = Rect::new(
            self.layout.editor.x,
            self.layout.editor.bottom() - 24.0,
            self.layout.editor.w,
            24.0,
        );
        if !edge.contains(x, y) {
            return;
        }
        self.ensure_terminal_panel_group();
        self.set_terminal_open(true);
        self.terminal.motion = None;
        self.terminal.drag_opened = true;
        self.relayout();
    }

    /// 未把终端放回临时展开的空面板时恢复原来的收起状态。
    pub(in crate::app) fn finish_terminal_drag_panel(&mut self) {
        if std::mem::take(&mut self.terminal.drag_opened) && self.terminal.panel_tab_count() == 0 {
            self.set_terminal_open(false);
            self.terminal.motion = None;
            self.relayout();
        }
    }
}

/// 返回编辑区中的终端占用的范围：窗格的内容 `body` 向上延伸一个终端标签栏的高度，终端的内容
/// 仍从 `body` 的上沿开始。
fn lifted(body: Rect) -> Rect {
    let lift = header_height(Rect::new(body.x, body.y - HEADER, body.w, 0.0));
    Rect::new(body.x, body.y - lift, body.w, body.h + lift)
}
