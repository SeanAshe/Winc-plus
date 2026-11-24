//! 拖放编辑：在选中的文字上按下并拖动，松开时把文字移到松开处；松开时按住复制键则复制。
//!
//! 按下后没有移动超过 [`App::repeat_slop`] 就松开，按普通单击在按下处放置光标。设置项
//! `editor.dragAndDrop` 关闭时，在选区上按下与在别处按下相同。

use winc_editor::Command;

use super::*;

impl App {
    /// 是否可以从窗口坐标 (`x`, `y`)（逻辑像素）开始拖动选中的文字：指针压在主选区的文字上，
    /// 文档可编辑，只有一个选区，且没有关闭拖放编辑。
    pub(in crate::app) fn press_on_selection(&self, x: f32, y: f32) -> bool {
        if self.config.drag_and_drop == Some(false) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let selection = doc.selection;
        if selection.is_empty()
            || !doc.extra_selections().is_empty()
            || doc.readonly_reason().is_some()
        {
            return false;
        }
        // `pos_at` 取离指针最近的字符边界；左移半个字符宽，取到的是指针所压字符的起点。
        let Some(at) = self.pos_at(x - self.metrics.cell / 2.0, y) else {
            return false;
        };
        selection.from() <= at && at < selection.to()
    }

    /// 拖动选中的文字时鼠标移到 (`x`, `y`)：越过 [`App::repeat_slop`] 后开始拖动，贴近编辑区
    /// 边缘时滚动；`press` 是按下的位置，`at` 是没有拖动时放置光标的位置。
    pub(in crate::app) fn drag_text(&mut self, x: f32, y: f32, press: (f32, f32), at: usize) {
        let slop = self.repeat_slop();
        let moved = (x - press.0).abs() >= slop || (y - press.1).abs() >= slop;
        if !matches!(self.pointer.drag, Drag::MoveText { moving: true, .. }) {
            if !moved {
                return;
            }
            self.pointer.drag = Drag::MoveText {
                press,
                at,
                moving: true,
            };
        }
        if self.scroll_while_dragging(Some(x), y) {
            self.clamp_scroll();
        }
        self.update_cursor_icon();
        self.request_redraw();
    }

    /// 拖动选中的文字时文字将放下的位置（字符偏移）；没有在拖动时返回 `None`。
    pub(crate) fn text_drop(&self) -> Option<usize> {
        if !matches!(self.pointer.drag, Drag::MoveText { moving: true, .. }) {
            return None;
        }
        let (t, (x, y)) = (self.layout.text, self.pointer.mouse);
        self.pos_at(
            x.clamp(t.x, t.right()),
            y.clamp(t.y, (t.bottom() - 1.0).max(t.y)),
        )
    }

    /// 松开时是否复制选中的文字，而不是移动：macOS 上按住 `⌥` 或 `⌘`，其他系统上按住 `Ctrl`。
    pub(in crate::app) fn drop_copies(&self) -> bool {
        let m = self.pointer.modifiers;
        if cfg!(target_os = "macos") {
            m.alt_key() || m.super_key()
        } else {
            m.control_key()
        }
    }

    /// 松开鼠标，结束拖动选中的文字：拖动过时（`dropped` 是放下的位置）把文字移到或复制到
    /// 那里，否则在按下处 `at` 放置光标。
    pub(in crate::app) fn release_text_drag(&mut self, at: usize, dropped: Option<usize>) {
        let Some(to) = dropped else {
            if let Some(doc) = self.editor.active_mut() {
                doc.goal_cell = None;
                doc.record_cursors();
                doc.collapse_selections();
                doc.selection = Selection::point(at);
            }
            self.reset_blink();
            self.request_redraw();
            return;
        };
        let copy = self.drop_copies();
        self.run(Command::DropText { at: to, copy });
        self.update_cursor_icon();
    }
}
