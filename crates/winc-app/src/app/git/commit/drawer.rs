//! 提交视图底部可收起的一组行（储藏、工作树）共用的状态：是否展开，以及高度的过渡。
//!
//! 展开、收起的时长与文件树展开目录相同（`TREE_ANIMATION`）。

use super::*;
use crate::app::animation::Clock;

/// 一组收起时的高度（逻辑像素）：标题一行，上下各留 [`STASH_GAP`]。
pub(crate) const DRAWER_CLOSED: f32 = COMMIT_ROW + 2.0 * STASH_GAP;

/// 一组的展开状态和高度的过渡。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Drawer {
    /// 是否展开。
    pub(crate) open: bool,
    /// 高度的过渡：起始高度（逻辑像素）和时钟；不在过渡中时为 `None`。
    pub(crate) motion: Option<(f32, Clock)>,
    /// 上一帧的终点高度（逻辑像素），见 [`Drawer::track`]。
    pub(crate) settled: f32,
}

impl Drawer {
    pub(crate) fn new(open: bool) -> Drawer {
        Drawer {
            open,
            motion: None,
            settled: 0.0,
        }
    }

    /// 返回终点高度为 `settled` 时当前的高度（逻辑像素），过渡中按时钟插值。
    pub(crate) fn height(&self, settled: f32) -> f32 {
        (self.motion).map_or(settled, |(from, clock)| clock.between(from, settled))
    }

    /// 返回标题上箭头的角度比例：0 朝右（收起），1 朝下（展开），随当前高度变化。
    ///
    /// `span` 是展开时标题下面各行的总高度（逻辑像素）。
    pub(crate) fn chevron(&self, settled: f32, span: f32) -> f32 {
        if span <= 0.0 {
            return 0.0;
        }
        ((self.height(settled) - DRAWER_CLOSED) / span).clamp(0.0, 1.0)
    }

    /// 终点高度变为 `settled` 时从当前高度开始过渡；`clock` 为 `None` 时直接到位。
    pub(crate) fn track(&mut self, settled: f32, clock: Option<Clock>) {
        let old = std::mem::replace(&mut self.settled, settled);
        if (settled - old).abs() < 0.5 {
            return;
        }
        if let Some(clock) = clock {
            let from = (self.motion).map_or(old, |(from, clock)| clock.between(from, old));
            self.motion = Some((from, clock));
        }
    }

    /// 收起或展开，从高度 `from` 开始过渡。调用后须把 `settled` 设为新的终点高度。
    pub(crate) fn toggle(&mut self, from: f32, clock: Option<Clock>) {
        self.open = !self.open;
        self.motion = clock.map(|clock| (from, clock));
    }
}
