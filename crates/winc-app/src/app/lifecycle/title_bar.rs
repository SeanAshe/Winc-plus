//! 应用自己绘制的标题栏（macOS 以外的平台，配置 `ui.titleBarStyle` 不是 `native` 时）：窗口
//! 不要系统的边框和标题栏，顶栏右端放最小化、最大化（还原）、关闭三个按钮，顶栏空白处仍然可以
//! 拖动窗口、双击最大化；窗口没有最大化时，四边和四角各留一条细边，按下拖动改变窗口大小。

use winit::window::ResizeDirection;

use super::*;

/// 顶栏右端的一个窗口按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControl {
    /// 最小化。
    Minimize,
    /// 最大化；已经最大化时还原。
    Maximize,
    /// 关闭窗口。
    Close,
}

impl WindowControl {
    /// 从左到右的顺序。
    pub const ALL: [WindowControl; 3] = [
        WindowControl::Minimize,
        WindowControl::Maximize,
        WindowControl::Close,
    ];
}

/// 窗口按钮一组的宽度（逻辑像素）：三个按钮，彼此相隔 `BUTTON_GAP`。
pub(crate) const WINDOW_CONTROLS_W: f32 = 3.0 * BUTTON + 2.0 * BUTTON_GAP;

/// 窗口边缘可以拖动改变大小的宽度（逻辑像素）。
pub(in crate::app) const RESIZE_EDGE: f32 = 4.0;

/// 改变窗口大小的方向对应的鼠标指针。
pub(crate) fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::NResize,
        ResizeDirection::South => CursorIcon::SResize,
        ResizeDirection::East => CursorIcon::EResize,
        ResizeDirection::West => CursorIcon::WResize,
        ResizeDirection::NorthEast => CursorIcon::NeResize,
        ResizeDirection::NorthWest => CursorIcon::NwResize,
        ResizeDirection::SouthEast => CursorIcon::SeResize,
        ResizeDirection::SouthWest => CursorIcon::SwResize,
    }
}

impl App {
    /// 按配置决定是否由应用绘制标题栏，并相应地打开或关闭系统的边框和标题栏。
    pub(in crate::app) fn sync_title_bar(&mut self) {
        self.window.controls =
            !cfg!(target_os = "macos") && self.config.ui_native_title_bar != Some(true);
        if let Some(window) = &self.window.handle {
            window.set_decorations(!self.window.controls);
        }
    }

    /// 在顶栏右端放置窗口按钮，标签栏让出这一块；终端面板在右边时它的标签栏也让出。
    ///
    /// 不由应用绘制标题栏时为空矩形。
    pub(in crate::app) fn place_window_controls(&mut self) {
        let l = &mut self.layout;
        l.controls = Rect::default();
        if !self.window.controls {
            return;
        }
        let top = ISLAND_MARGIN + (TOP_BAR - BUTTON) / 2.0;
        let right = l.window.right() - ISLAND_MARGIN - 6.0;
        l.controls = Rect::new(right - WINDOW_CONTROLS_W, top, WINDOW_CONTROLS_W, BUTTON);
        let end = l.controls.x - TAB_GAP;
        if l.tabs.right() > end {
            l.tabs.w = (end - l.tabs.x).max(0.0);
        }
    }

    /// 每个窗口按钮的位置。
    pub(crate) fn window_control_rects(&self) -> [(Rect, WindowControl); 3] {
        let c = self.layout.controls;
        std::array::from_fn(|i| {
            let x = c.x + i as f32 * (BUTTON + BUTTON_GAP);
            (Rect::new(x, c.y, BUTTON, BUTTON), WindowControl::ALL[i])
        })
    }

    /// `(x, y)` 处的窗口按钮。
    pub(in crate::app) fn window_control_at(&self, x: f32, y: f32) -> Option<WindowControl> {
        if !self.layout.controls.contains(x, y) {
            return None;
        }
        crate::layout::hit(&self.window_control_rects(), x, y)
    }

    /// `(x, y)` 处可以拖动改变窗口大小的方向：只在应用绘制标题栏、窗口没有最大化时有。
    pub(in crate::app) fn resize_edge_at(&self, x: f32, y: f32) -> Option<ResizeDirection> {
        if !self.window.controls
            || self
                .window
                .handle
                .as_ref()
                .is_some_and(|w| w.is_maximized())
        {
            return None;
        }
        let w = self.layout.window;
        let (left, right) = (x < RESIZE_EDGE, x >= w.right() - RESIZE_EDGE);
        let (top, bottom) = (y < RESIZE_EDGE, y >= w.bottom() - RESIZE_EDGE);
        Some(match (top, bottom, left, right) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (true, _, _, true) => ResizeDirection::NorthEast,
            (_, true, true, _) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::North,
            (_, true, ..) => ResizeDirection::South,
            (_, _, true, _) => ResizeDirection::West,
            (_, _, _, true) => ResizeDirection::East,
            _ => return None,
        })
    }

    /// 按下窗口边缘：交给系统拖动改变窗口大小。
    pub(in crate::app) fn start_resize(&self, direction: ResizeDirection) {
        if let Some(window) = &self.window.handle {
            let _ = window.drag_resize_window(direction);
        }
    }

    /// 点击窗口按钮。
    pub(in crate::app) fn press_window_control(&mut self, control: WindowControl) {
        match control {
            WindowControl::Minimize => {
                if let Some(window) = &self.window.handle {
                    window.set_minimized(true);
                }
            }
            WindowControl::Maximize => self.toggle_maximized(),
            WindowControl::Close => self.request_close_window(),
        }
    }

    /// 窗口是否最大化（没有窗口时为否）。
    pub(crate) fn window_maximized(&self) -> bool {
        self.window
            .handle
            .as_ref()
            .is_some_and(|w| w.is_maximized())
    }

    /// 切换窗口的最大化。
    pub(in crate::app) fn toggle_maximized(&self) {
        if let Some(window) = &self.window.handle {
            window.set_maximized(!window.is_maximized());
        }
    }
}
