//! 触摸滚动的惯性：抬起手指时仍在滑动，内容按逐渐衰减的速度继续滚动。
//!
//! 抬手时的速度由 `input/touch.rs` 按最近的位置样本估计；之后每帧绘制前由
//! [`App::advance_fling`] 按经过的时间推进，滚动落指处的区域（[`ScrollTarget`]）。

use super::*;

/// 惯性速度每毫秒保留的比例。
///
/// 依据 `UIKit` 的 `UIScrollView.DecelerationRate.normal`（每毫秒 0.998）。
const DECAY_PER_MS: f64 = 0.998;

/// 开始和维持惯性滚动的最小速度（逻辑像素/秒）。
///
/// 依据 Android 的 `ViewConfiguration.getScaledMinimumFlingVelocity` 的默认值 50 dp/s，按 1 dp
/// 合 1 逻辑像素换算。低于该速度时内容已接近静止，继续逐帧推进只增加绘制量。
pub(in crate::app) const MIN_FLING_SPEED: f32 = 50.0;

/// 进行中的惯性滚动。
#[derive(Debug, Clone)]
pub(crate) struct Fling {
    /// 滚动的区域。
    target: ScrollTarget,
    /// 落指的位置（逻辑像素），见 [`App::scroll_target_by`]。
    origin: (f32, f32),
    /// 此刻的速度（横，纵）（逻辑像素/秒），符号与滚动增量相同。
    velocity: (f32, f32),
    /// 上一次推进的时刻。
    at: Instant,
}

/// 返回速度为 `velocity`（逻辑像素/秒）的惯性滚动经过 `elapsed` 后移动的距离（逻辑像素）与
/// 此时的速度。
///
/// 速度按 [`DECAY_PER_MS`] 连续衰减，距离为速度在这段时间内的积分。
fn decay(velocity: (f32, f32), elapsed: Duration) -> ((f32, f32), (f32, f32)) {
    let ms = elapsed.as_secs_f64() * 1000.0;
    let kept = DECAY_PER_MS.powf(ms);
    // 速度 v·rᵗ 从 0 积分到 t 得 v·(1 − rᵗ) / (−ln r)；速度按毫秒计，除以 1000。
    let travel = (1.0 - kept) / -DECAY_PER_MS.ln() / 1000.0;
    let scale = |v: f32, k: f64| (f64::from(v) * k) as f32;
    (
        (scale(velocity.0, travel), scale(velocity.1, travel)),
        (scale(velocity.0, kept), scale(velocity.1, kept)),
    )
}

impl App {
    /// 在 `origin`（逻辑像素）处的区域开始惯性滚动，初速度为 `velocity`（逻辑像素/秒），
    /// 只取位移较大的方向。
    pub(in crate::app) fn start_fling(&mut self, origin: (f32, f32), velocity: (f32, f32)) {
        let (vx, vy) = velocity;
        let velocity = if vy.abs() >= vx.abs() {
            (0.0, vy)
        } else {
            (vx, 0.0)
        };
        if velocity.0.hypot(velocity.1) < MIN_FLING_SPEED {
            return;
        }
        let target = self.scroll_target_at(origin.0, origin.1);
        if matches!(target, ScrollTarget::Nothing | ScrollTarget::Blocked) {
            return;
        }
        self.scroll.fling = Some(Fling {
            target,
            origin,
            velocity,
            at: Instant::now(),
        });
        self.request_redraw();
    }

    /// 把惯性滚动推进到 `now`，绘制前调用。
    ///
    /// 速度低于 [`MIN_FLING_SPEED`]、已滚动到边缘，或该区域已不可滚动（切换了标签页、关闭了
    /// 面板）时结束。
    pub(in crate::app) fn advance_fling(&mut self, now: Instant) {
        let Some(fling) = self.scroll.fling.clone() else {
            return;
        };
        let (target, origin) = (fling.target, fling.origin);
        if self.scroll_target_at(origin.0, origin.1) != target {
            self.scroll.fling = None;
            return;
        }
        let (travel, velocity) = decay(fling.velocity, now.saturating_duration_since(fling.at));
        let before = self.scroll_probe(target);
        self.scroll_target_by(target, origin, false, travel);
        let stuck = before.is_some() && self.scroll_probe(target) == before;
        let slow = velocity.0.hypot(velocity.1) < MIN_FLING_SPEED;
        self.scroll.fling = (!stuck && !slow).then_some(Fling {
            velocity,
            at: now,
            ..fling
        });
    }

    /// 返回区域 `target` 此刻的滚动位置，用于判断惯性滚动是否已到边缘；无法读取时返回 `None`。
    fn scroll_probe(&self, target: ScrollTarget) -> Option<(f64, f64)> {
        match target {
            ScrollTarget::Editor(_) => {
                let doc = self.editor.active()?;
                Some((doc.scroll_line, f64::from(doc.scroll_x)))
            }
            ScrollTarget::Tree => Some((
                f64::from(self.panels.tree.scroll),
                f64::from(self.panels.tree.scroll_x),
            )),
            ScrollTarget::Pdf => Some((
                f64::from(self.preview.pdf.scroll),
                f64::from(self.preview.pdf.scroll_x),
            )),
            _ => self.pane_scroll(target).map(|y| (f64::from(y), 0.0)),
        }
    }
}
