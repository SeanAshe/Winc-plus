//! 改动列表一行行尾按钮的排法，布局与绘制共用。

use super::*;

/// 一行行尾的按钮：从左到右 `count` 个，同宽，相隔 [`ActionStrip::GAP`]，最左边一个在 `first`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ActionStrip {
    /// 最左边一个按钮的位置。
    pub first: Rect,
    /// 按钮的个数（1 至 3）。
    pub count: usize,
}

impl ActionStrip {
    /// 相邻两个按钮之间的空隙（逻辑像素）。
    pub const GAP: f32 = 2.0;

    /// 返回 `count` 个边长 `size` 的按钮、最右边一个的右边缘在 `right` 时的排法；`y` 是按钮的上沿。
    pub fn ending_at(right: f32, y: f32, size: f32, count: usize) -> Self {
        let w = count as f32 * (size + Self::GAP) - Self::GAP;
        ActionStrip {
            first: Rect::new(right - w, y, size, size),
            count,
        }
    }

    /// 第 `i` 个按钮（从左数，从 0 起）的位置。
    pub fn rect(&self, i: usize) -> Rect {
        let x = self.first.x + i as f32 * (self.first.w + Self::GAP);
        Rect::new(x, self.first.y, self.first.w, self.first.h)
    }

    /// 最右边一个按钮的右边缘。
    pub fn right(&self) -> f32 {
        self.rect(self.count.saturating_sub(1)).right()
    }

    /// 整体向右移 `dx`（为负时向左）。
    pub fn moved(self, dx: f32) -> Self {
        let f = self.first;
        ActionStrip {
            first: Rect::new(f.x + dx, f.y, f.w, f.h),
            ..self
        }
    }
}
