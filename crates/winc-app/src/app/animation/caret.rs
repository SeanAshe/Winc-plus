//! 主光标的移动动画：光标从上一位置滑到新位置。
//!
//! 曲线和时长取自 IntelliJ IDEA 的平滑光标移动：Snappy 在开始的一小段时间里走过大半距离，
//! 之后缓缓停住；Gliding 沿整段距离平滑移动。移动途中光标再次移动时，从当前画面上的位置接着
//! 走向新目标；接连的移动逐次缩短时长，使连续按键时不会落后于输入。
//!
//! 位置由绘制时算出的光标矩形给出（见 `view::editor`），本模块只负责状态和插值。视口滚动、
//! 换行或布局变化时目标随之改变：光标停在原处时随文字直接移动；移动途中的目标跟着视口走，
//! 光标继续滑向新目标。

use std::time::Duration;

use winc_config::SmoothCaret;
use winc_editor::DocumentId;
use winc_render::Rect;

use super::*;

/// 一次光标移动的时长（毫秒）。
///
/// 取 IntelliJ IDEA 的 `editor.smooth.caret.duration` 默认值 120 ms：短于侧栏等过渡，
/// 连续按键时不会明显滞后。
pub(in crate::app) const CARET_MOVE: Duration = Duration::from_millis(120);

/// 接连的移动依次缩短时长，最少缩到完整时长的这一比例。
const MIN_URGENCY: f32 = 0.2;

/// 接连的移动中每一次的时长乘以这一比例。
const URGENCY_DECAY: f32 = 0.8;

/// 判断两个光标位置是否画在同一处（逻辑像素）。
///
/// 小于一个像素的差别看不出来；目标每帧重新计算，不能因此把动画当成被滚动打断。
const SAME_SPOT: f32 = 0.1;

/// 主光标所在的文档和字符偏移；它改变时开始一次新的移动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaretKey {
    doc: DocumentId,
    pos: usize,
}

/// 光标在屏幕上的位置，以及它所在的文档和字符偏移。
#[derive(Debug, Clone, Copy)]
pub(crate) struct CaretTarget {
    key: CaretKey,
    rect: Rect,
    /// 视口是否正随这次光标移动平滑滚动（跳转、居中光标）：目标会逐帧移动，移动中的
    /// 光标应跟着它走，而不是直接跳到目标。
    follow_scroll: bool,
}

impl CaretTarget {
    /// 返回文档 `doc` 中字符偏移 `pos` 处的光标矩形 `rect`。
    pub(crate) fn new(doc: DocumentId, pos: usize, rect: Rect, follow_scroll: bool) -> Self {
        Self {
            key: CaretKey { doc, pos },
            rect,
            follow_scroll,
        }
    }
}

/// 主光标的移动动画。
#[derive(Debug, Clone, Copy)]
pub(crate) struct CaretMotion {
    /// 光标所在的文档和字符偏移。
    key: CaretKey,
    /// 动画起点和当前目标（逻辑像素）。
    from: Rect,
    to: Rect,
    /// 动画时钟。
    clock: Clock,
    /// 上一次移动开始后的一段完整时长：其间开始的移动算作接连的移动。
    run: Option<Clock>,
    /// 接连移动时缩短的时长比例；隔了一段完整时长后再移动时恢复为 1。
    urgency: f32,
    /// 是否正在移动。
    moving: bool,
}

impl CaretMotion {
    /// 返回停在 `target` 处的状态。
    fn at(target: CaretTarget) -> Self {
        Self {
            key: target.key,
            from: target.rect,
            to: target.rect,
            clock: Clock::new(CARET_MOVE),
            run: None,
            urgency: 1.0,
            moving: false,
        }
    }

    /// 是否正在移动。
    pub(super) fn moving(&self) -> bool {
        self.moving && !self.clock.done()
    }

    /// 按本帧的光标目标推进，返回本帧要绘制的矩形。
    ///
    /// 位置改变时从当前画面上的位置开始新的移动；同一光标的目标随跳转的平滑滚动移动时保持
    /// 进度跟过去，光标没动而视口滚动（滚轮、滚动条）或布局变化时直接画在新位置；换了文档
    /// 时直接跳到目标。
    fn advance(&mut self, target: CaretTarget, style: SmoothCaret) -> Rect {
        self.settle();
        if self.key == target.key {
            if same_spot(self.to, target.rect) {
                return self.painted(style);
            }
            if self.moving && target.follow_scroll {
                // 跳转的平滑滚动带着目标走：保持进度跟过去，不重新计时。
                self.to = target.rect;
                return self.painted(style);
            }
            // 光标没动，或只是滚轮滚动、换行、布局变化：直接画在新位置。
            *self = CaretMotion::at(target);
            return target.rect;
        }
        if self.key.doc != target.key.doc {
            // 换了文档：直接到新位置。
            *self = CaretMotion::at(target);
            return target.rect;
        }

        // 光标移动到了新的位置：从当前画面上的位置接着滑过去。即使视口同时滚动（跳到视野之外
        // 的行），也从原来的位置滑向新位置。上一次移动开始后一段完整时长之内的移动一次比
        // 一次短，按住按键时不会一直追不上输入，也不会走完一次就回到完整时长。
        let from = self.painted(style);
        let urgency = match self.run.filter(|run| !run.done()) {
            Some(_) => (self.urgency * URGENCY_DECAY).max(MIN_URGENCY),
            None => 1.0,
        };
        self.key = target.key;
        self.to = target.rect;
        self.run = Some(Clock::new(CARET_MOVE));
        self.urgency = urgency;
        if same_spot(from, target.rect) {
            // 画面上的位置没有变（光标之前的文字变了）：不必滑动。
            self.from = target.rect;
            self.moving = false;
            return target.rect;
        }
        self.from = from;
        self.clock = Clock::new(CARET_MOVE.mul_f32(urgency));
        self.moving = true;
        from
    }

    /// 返回当前画面上的光标矩形。
    fn painted(&self, style: SmoothCaret) -> Rect {
        let t = ease(style, self.clock.linear());
        Rect::new(
            self.from.x + (self.to.x - self.from.x) * t,
            self.from.y + (self.to.y - self.from.y) * t,
            self.to.w,
            self.to.h,
        )
    }

    /// 把动画时钟和接连移动的计时固定在已过 `at` 处，仅用于测试。
    #[cfg(test)]
    pub(in crate::app) fn freeze(&mut self, at: Duration) {
        self.clock = Clock::frozen(at, self.clock.duration());
        self.run = self.run.map(|_| Clock::frozen(at, CARET_MOVE));
    }

    /// 动画已经结束时把光标停在目标处。
    pub(super) fn settle(&mut self) {
        if self.clock.done() {
            self.moving = false;
            self.from = self.to;
        }
    }
}

/// 返回时间曲线在 `progress`（0 到 1）处的进度。
///
/// 曲线取自 IntelliJ IDEA（`CaretEasing`）：Snappy 是先快后慢的解析曲线，Gliding 是
/// `cubic-bezier(0.25, 0.1, 0.25, 1.0)` 的近似。`Off` 不播放动画，因此直接到终点。
pub(in crate::app) fn ease(style: SmoothCaret, progress: f32) -> f32 {
    let t = progress.clamp(0.0, 1.0);
    let eased = match style {
        SmoothCaret::Off => 1.0,
        SmoothCaret::Snappy => {
            let root = t.cbrt();
            3.0 * root - 3.0 * root * root + t
        }
        SmoothCaret::Gliding => t * ((((-5.4 * t + 17.6) * t - 20.6) * t + 9.0) * t + 0.4),
    };
    // 近似曲线在端点上的舍入误差不应让光标越过终点。
    eased.clamp(0.0, 1.0)
}

/// 返回两个光标位置是否画在同一处。
fn same_spot(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() <= SAME_SPOT && (a.y - b.y).abs() <= SAME_SPOT
}

impl App {
    /// 返回光标移动的动画方式；`editor.smoothCaretMovement` 没写时不播放动画。
    pub(crate) fn smooth_caret(&self) -> SmoothCaret {
        self.config.smooth_caret_movement.unwrap_or_default()
    }

    /// 按本帧的光标目标推进移动动画，返回本帧要绘制的光标矩形。
    ///
    /// `target` 是绘制时算出的光标位置。位置改变时从当前画面上的位置开始新的移动；同一位置
    /// 的目标随视口滚动时，移动中的光标保持进度跟过去，停下的光标直接画在新位置；换了文档或
    /// 光标不在屏幕上（状态已被清除）时直接跳到目标。
    pub(crate) fn advance_caret(&mut self, target: CaretTarget) -> Rect {
        let style = self.smooth_caret();
        if style == SmoothCaret::Off {
            self.motion.caret_motion = None;
            return target.rect;
        }
        let Some(motion) = &mut self.motion.caret_motion else {
            self.motion.caret_motion = Some(CaretMotion::at(target));
            return target.rect;
        };
        motion.advance(target, style)
    }

    /// 本帧没有画出主光标：清除移动动画，下一次移动直接出现在新位置。
    pub(crate) fn caret_hidden(&mut self) {
        self.motion.caret_motion = None;
    }
}

#[cfg(test)]
mod tests;
