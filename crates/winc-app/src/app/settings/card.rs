//! 拖动设置卡片的标题行，以及滚动、拖动期间复用卡片下方的窗口画面。

use super::*;

/// 停止滚动后保留快照的时长（毫秒）。
///
/// 取值约为 60 Hz 下 9 帧的间隔，滚动事件短暂间断时不必重新保存快照。这段时间内浮层下方的变化在
/// 释放快照后统一绘制。
const SCROLL_BASE_KEEP: Duration = Duration::from_millis(150);

impl App {
    /// 拖动卡片的标题行：卡片左上角移到鼠标 (`x`, `y`) 减去按下时的 `grab`，限制在窗口内。
    pub(in crate::app) fn drag_settings_card(&mut self, x: f32, y: f32, grab: (f32, f32)) {
        let window = self.layout.window;
        let center = crate::layout::settings_card(window, (0.0, 0.0));
        let card =
            crate::layout::settings_card(window, (x - grab.0 - center.x, y - grab.1 - center.y));
        let offset = (card.x - center.x, card.y - center.y);
        if offset != self.settings.offset {
            self.settings.offset = offset;
            // 与滚动相同，拖动期间只重绘卡片。
            self.settings.scrolled_at = Some(Instant::now());
            self.request_redraw();
        }
    }

    /// 当前帧是否可以使用保存的快照：设置页正在滚动或拖动，且快照与窗口的尺寸、缩放比例相同。
    pub(crate) fn settings_reuses_base(&self, bounds: Rect, scale: f32) -> bool {
        self.settings.open
            && self.settings.motion.is_none()
            && self.settings.scrolled_at.is_some()
            && (self.settings.scroll_base.as_ref()).is_some_and(|b| b.covers(bounds, scale))
    }

    /// 是否正在拖动设置卡片的标题行。
    pub(crate) fn settings_dragging(&self) -> bool {
        matches!(self.pointer.drag, Drag::SettingsCard { .. })
    }

    /// 滚动结束后释放快照并请求重绘，显示滚动期间浮层下方的变化；返回下一次唤醒的时刻。
    pub(crate) fn settings_scroll_tick(&mut self, now: Instant) -> Option<Instant> {
        let due = self.settings.scrolled_at? + SCROLL_BASE_KEEP;
        if now < due {
            return Some(due);
        }
        self.settings.scrolled_at = None;
        self.settings.scroll_base = None;
        self.request_redraw();
        None
    }
}
