//! 鼠标：悬停、点击、拖动和滚轮。
//!
//! 绘制每帧把可点击的区域记入 `*_hits` 表（标签、文件树、编辑区、按钮等），鼠标事件按当前
//! 焦点和这些表找到目标。拖动包括标签排序、文件树拖动、滚动条和文本选区。

mod click;
mod drag;
mod regions;
mod target;

use target::*;

use std::time::Instant;

use winit::event::{MouseButton, MouseScrollDelta, TouchPhase};
use winit::window::CursorIcon;

use winc_core::{Lines, Selection, movement};
use winc_render::Renderer;

use crate::app::{
    App, Click, DOUBLE_CLICK, Drag, Focus, Hover, MenuCommand, TreeDrag, WindowControl,
    resize_cursor, tree_max_scroll,
};
use crate::layout::{
    RowAction, Scrollbar, SearchParts, SettingControl, SettingsPart, SidebarView, hit,
    hscrollbar_thumb, row_actions, scroll_for_thumb, scrollbar_thumb, settings_thumb,
    settings_track,
};
use crate::theme::*;

/// 触控板调整字号时，每调整一档所需的滚动距离（逻辑像素）。
///
/// 取自调参，只影响触控板调整字号的灵敏度。
pub(in crate::app) const ZOOM_WHEEL_PIXELS: f32 = 48.0;

impl App {
    /// 处理鼠标按键 `button` 松开：结束拖动，并在需要时关闭标签或执行窗口按钮。
    ///
    /// 左键按在关闭按钮上、或中键按在标签上之后，松开时鼠标仍在同一个标签上才关闭该标签；
    /// 左键按在窗口按钮上之后，松开时鼠标仍在同一个按钮上才执行该按钮。
    pub(in crate::app) fn on_mouse_up(&mut self, button: MouseButton) {
        // 拖动标签时悬停目标被清除，松开后重新判断鼠标下方的部件。
        let was_tab = self.dragged_tab().is_some();
        self.pointer.drag_scrolled_at = None;
        if button == MouseButton::Left {
            self.diagram_view_mouse_up();
        }
        if button == MouseButton::Left {
            self.release_tree_item();
            self.release_tab();
        }

        if matches!(button, MouseButton::Left | MouseButton::Middle)
            && let Some(id) = self.pointer.close_pressed.take()
            && let Some((index, _)) = self.tab_at(self.pointer.mouse.0, self.pointer.mouse.1)
            && self.editor.documents.get(index).is_some_and(|d| d.id == id)
        {
            self.tabs.width_lock = self.tabs.widths.get(index).map(|&w| (index, w));
            self.close_tab(index);
        }

        if button == MouseButton::Left
            && let Some(control) = self.pointer.control_pressed.take()
            && self.window_control_at(self.pointer.mouse.0, self.pointer.mouse.1) == Some(control)
        {
            self.press_window_control(control);
        }

        self.terminal_mouse_up();
        self.release_terminal_tab();
        let was_scrollbar = matches!(self.pointer.drag, Drag::Scrollbar { .. });
        let was_divider = matches!(
            self.pointer.drag,
            Drag::Splitter { .. }
                | Drag::TerminalDivider { .. }
                | Drag::SplitDivider { .. }
                | Drag::OutlineDivider { .. }
        );
        if matches!(self.pointer.drag, Drag::TerminalDivider { .. }) {
            self.terminal_dragging(false);
        }
        if matches!(self.pointer.drag, Drag::Splitter { .. }) {
            self.remember_panel_width();
        }
        self.pointer.drag = Drag::None;
        // 拖动分隔条期间不重新断行（`sync_wrap`）；松开后重绘，重绘前的布局按最终宽度断行。
        if was_divider {
            self.request_redraw();
        }
        if was_scrollbar || was_tab {
            self.set_hover(self.hover_at(self.pointer.mouse.0, self.pointer.mouse.1));
        }
        self.update_cursor_icon();
    }

    /// 处理窗口的滚轮事件：按住主修饰键时调整字号（见 [`Self::wheel_zoom`]），否则滚动。
    ///
    /// 触控板的手势阶段 `phase` 用于区分两次手势：调整字号与滚动的方向都在手势开始时确定。
    pub(in crate::app) fn on_wheel_event(&mut self, delta: MouseScrollDelta, phase: TouchPhase) {
        let fresh = self.note_wheel_gesture(delta, phase);
        if !self.wheel_zoom(delta, fresh) {
            self.scroll_by_wheel(delta, true);
        }
    }

    /// 只按住主修饰键滚动时，调整鼠标所在的编辑区或终端的字号，或缩放 PDF、图片，
    /// 返回事件是否已处理。
    ///
    /// 触控板的一次手势是否调整字号在手势开始时确定：滚动途中按下修饰键仍是滚动，调整途中
    /// 松开修饰键后余下的滚动量丢弃。每个事件至多调整一档。`fresh` 表示触控板开始了新的手势。
    fn wheel_zoom(&mut self, delta: MouseScrollDelta, fresh: bool) -> bool {
        let keys = self.pointer.modifiers;
        let held = self.command_held()
            && keys.control_key() != keys.super_key()
            && !keys.shift_key()
            && !keys.alt_key();
        let zoomed = match self.target_at(self.pointer.mouse.0, self.pointer.mouse.1) {
            Target::Editor if self.on_pdf_body() => Some(Zoomed::Pdf),
            Target::Picture => Some(Zoomed::Picture),
            Target::Editor | Target::Gutter => Some(Zoomed::Editor),
            Target::Terminal | Target::TerminalBlock(_) => Some(Zoomed::Terminal),
            _ => None,
        };
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                if !held || zoomed.is_none() {
                    self.scroll.wheel.zoom = 0.0;
                    return false;
                }
                // macOS 按滚动速度给出不足一格的量，每个事件按一格计。
                if cfg!(target_os = "macos") {
                    y.signum()
                } else {
                    y
                }
            }
            MouseScrollDelta::PixelDelta(delta) => {
                if fresh {
                    self.scroll.wheel.zooming = held && zoomed.is_some();
                    self.scroll.wheel.zoom = 0.0;
                }
                if !self.scroll.wheel.zooming {
                    return false;
                }
                if !held {
                    return true;
                }
                let scale = self
                    .frame
                    .renderer
                    .as_ref()
                    .map_or(1.0, Renderer::scale_factor);
                delta.y as f32 / scale / ZOOM_WHEEL_PIXELS
            }
        };
        let Some(zoomed) = zoomed else {
            return true;
        };
        self.scroll.wheel.zoom += amount;
        if self.scroll.wheel.zoom.abs() >= 1.0 {
            let up = self.scroll.wheel.zoom > 0.0;
            self.scroll.wheel.zoom = 0.0;
            self.dismiss_hint();
            // PDF 和图片缩放时鼠标处的内容保持在原位。
            match zoomed {
                Zoomed::Pdf => drop(self.zoom_pdf(Some(up), Some(self.pointer.mouse))),
                Zoomed::Picture => drop(self.zoom_picture(Some(up), Some(self.pointer.mouse))),
                Zoomed::Editor | Zoomed::Terminal => {}
            }
            if matches!(zoomed, Zoomed::Pdf | Zoomed::Picture) {
                return true;
            }
            // 调整字号会把光标滚入视野；这里保持编辑区顶部的行不变。
            let top = self.editor.active().map(|doc| doc.scroll_line);
            self.step_font_size_in(zoomed == Zoomed::Terminal, Some(up));
            if let Some(top) = top {
                self.set_active_scroll(top);
                self.clamp_scroll();
            }
        }
        true
    }
}

/// 按住主修饰键滚动时调整的对象。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Zoomed {
    Editor,
    Terminal,
    Pdf,
    Picture,
}
