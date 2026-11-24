//! 窗格增减时的过渡：变化之前先画一帧并存下编辑区卡片，新布局画好以后它在上面淡出。新窗格随之
//! 淡入，关掉的窗格随之淡出；内容一开始就在最终的位置，不移动，也不重新断行。

use super::*;
use crate::app::animation::{Clock, SIDEBAR_ANIMATION};
use winc_render::Snapshot;

/// 窗格增减后的过渡：变化之前的编辑区卡片和时钟。
#[derive(Debug)]
pub(crate) struct PaneMotion {
    before: Snapshot,
    clock: Clock,
}

impl PaneMotion {
    /// 过渡是否已结束。
    pub(crate) fn done(&self) -> bool {
        self.clock.done()
    }
}

impl App {
    /// 窗格增减之前调用：动态效果开着时把现在的画面画一遍，存下编辑区卡片，开始过渡。
    pub(in crate::app) fn capture_panes(&mut self) {
        let Some(clock) = self.pane_clock() else {
            return;
        };
        self.relayout();
        let card = self.editor_card();
        let Some(mut renderer) = self.frame.renderer.take() else {
            return;
        };
        self.declare_base(&mut renderer);
        let scrolled = self.begin_scroll_frame();
        let mut before = None;
        renderer.draw(|frame| {
            crate::view::draw(self, frame);
            before = Some(frame.snapshot(card));
        });
        self.end_scroll_frame(scrolled);
        self.frame.renderer = Some(renderer);
        self.panes.motion = before.map(|before| PaneMotion { before, clock });
        self.request_redraw();
    }

    /// 在已画好的新布局上叠加过渡中的旧画面。
    pub(crate) fn draw_pane_motion(&self, frame: &mut winc_render::Frame<'_>) {
        if let Some(motion) = self.panes.motion.as_ref().filter(|m| !m.done()) {
            let opacity = 1.0 - motion.clock.progress();
            frame.blend_snapshot(&motion.before, 0.0, opacity, 1.0);
        }
    }

    /// 窗格过渡的时钟；动态效果关闭或没有窗口时为 `None`。
    fn pane_clock(&mut self) -> Option<Clock> {
        #[cfg(test)]
        if self.panes.animate_without_window {
            return Some(Clock::new(SIDEBAR_ANIMATION));
        }
        self.clock(SIDEBAR_ANIMATION)
    }
}

#[cfg(test)]
impl App {
    /// 把正在进行的窗格过渡停在开始后 `at` 的位置（截图用）。
    pub(crate) fn freeze_pane_motion(&mut self, at: std::time::Duration) {
        if let Some(motion) = &mut self.panes.motion {
            motion.clock = Clock::frozen(at, SIDEBAR_ANIMATION);
        }
    }
}
