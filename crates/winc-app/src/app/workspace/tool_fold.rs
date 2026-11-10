//! 文件树工具栏的收起动画：侧栏变窄时按钮滑进「⋯」并淡出，变宽时从「⋯」里滑出来。
//!
//! 按钮的位置和点击判断始终按终点布局（`TreeState::tools`、`TreeState::more`），这里只记每个按钮显示出来的
//! 程度，绘制时据此排布（见 [`crate::layout::folding_tools`]）。拖动分隔条时宽度连续变化，目标
//! 会在动画途中反复改变，所以不用一段固定的时钟，而是每帧把程度匀速推向目标：途中改了方向
//! 也是从当前位置继续，不会跳。

use std::time::{Duration, Instant};

use super::*;

/// 一个按钮从完全显示到完全收起所用的时长。
///
/// 取 160 ms（经验值）：更短的时长看不清滑动过程，更长的时长会让用户等待。
const TOOL_FOLD: Duration = Duration::from_millis(160);

/// 文件树工具栏的按钮数（按 [`TreeTool::ALL`] 的长度，不含「⋯」按钮）。
const TOOLS: usize = TreeTool::ALL.len();

/// 工具栏各按钮显示出来的程度：0 表示收起，1 表示完全显示。
#[derive(Debug, Clone, Default)]
pub struct ToolFold {
    /// 各工具（按 [`TreeTool::ALL`] 的顺序）当前的程度，最后一项是「⋯」按钮。
    shown: [f32; TOOLS + 1],
    /// 各按钮要到达的程度，与 `shown` 相同。
    target: [f32; TOOLS + 1],
    /// 上一次推进的时刻；没有在动时为 `None`。
    at: Option<Instant>,
    /// 是否已经摆好过；第一次（打开目录、启动）直接到位，不从「⋯」中展开。
    placed: bool,
    /// 测试截图用：停在当前程度，不随布局变化。
    #[cfg(test)]
    frozen: bool,
}

impl ToolFold {
    /// 是否正在动。
    pub fn moving(&self) -> bool {
        self.shown != self.target
    }

    /// 直接到达目标，停下动画。
    pub fn settle(&mut self) {
        self.shown = self.target;
        self.at = None;
    }

    /// 各工具和「⋯」按钮绘制用的程度：在匀速的进度上加先慢后快再慢的缓动。
    pub fn eased(&self) -> ([f32; TOOLS], f32) {
        let ease = |t: f32| t * t * (3.0 - 2.0 * t);
        let tools = std::array::from_fn(|i| ease(self.shown[i]));
        (tools, ease(self.shown[TOOLS]))
    }
}

impl App {
    /// 按这一帧的终点布局（`TreeState::tools`、`TreeState::more`）推进工具栏的收起动画；没有打开目录时
    /// 清空，下次打开时直接到位。
    pub(crate) fn step_tool_fold(&mut self) {
        if self.editor.workspace.is_none() {
            self.tool_fold = ToolFold::default();
            return;
        }

        let kept = self.hits.tree_tools.len();
        let mut target = [0.0; TOOLS + 1];
        target[..kept].fill(1.0);
        target[TOOLS] = if self.hits.tree_more.is_some() {
            1.0
        } else {
            0.0
        };

        let fold = &mut self.tool_fold;
        #[cfg(test)]
        if fold.frozen {
            return;
        }
        fold.target = target;
        if !fold.placed || !fold.moving() {
            fold.shown = target;
            fold.placed = true;
            fold.at = None;
            return;
        }

        // 刚开始动时确认可以播放动画（有窗口、动态效果开启），否则直接到位。
        if fold.at.is_none() && self.clock(TOOL_FOLD).is_none() {
            self.tool_fold.shown = target;
            return;
        }

        let fold = &mut self.tool_fold;
        let now = Instant::now();
        let elapsed = fold.at.map_or(Duration::ZERO, |at| now - at);
        fold.advance(elapsed);
        fold.at = fold.moving().then_some(now);
    }
}

impl ToolFold {
    /// 把各按钮的程度向目标推进 `elapsed` 时长的量，到目标为止。
    fn advance(&mut self, elapsed: Duration) {
        let step = elapsed.as_secs_f32() / TOOL_FOLD.as_secs_f32();
        for (shown, &target) in self.shown.iter_mut().zip(&self.target) {
            *shown = if *shown < target {
                (*shown + step).min(target)
            } else {
                (*shown - step).max(target)
            };
        }
    }
}

#[cfg(test)]
mod tests;
