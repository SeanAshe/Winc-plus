//! 滚动：滚轮与触控板的分发、编辑区与各区域的过渡、触摸的惯性，以及绘制时的像素对齐。
//!
//! [`ScrollTarget`] 在事件发生时确定滚动哪个区域，之后的过渡与惯性都作用于该区域。编辑区的
//! 滚动位置（`Document::scroll_line`）只经 [`set_scroll_line`] 写入，各区域的滚动位置经
//! `App::set_pane_scroll` 写入并限制范围。过渡期间字段保存目标位置，绘制时换入插值位置
//! （见 [`glide`]）。

use super::*;

pub(crate) mod editor;
pub(crate) mod fling;
pub(crate) mod glide;
pub(crate) mod target;
pub(crate) mod wheel;

pub use editor::ScrollGlide;
pub(crate) use editor::{set_scroll_line, swap_scroll_line};
pub(crate) use fling::Fling;
pub(crate) use glide::PaneGlide;
pub(crate) use target::ScrollTarget;
pub use wheel::Wheel;

/// 滚动的状态：滚轮与触控板的输入、平滑滚动的过渡、惯性滚动和滚动条的显示。
pub(crate) struct ScrollState {
    /// 各区域滚动条的显示状态：平时隐藏，滚动后或鼠标移到滚动条上时显示。
    pub(in crate::app) bars: Scrollbars,
    /// 编辑区跳转式滚动（翻页、跳到行、查找跳转）的平滑滚动状态。
    pub(in crate::app) glide: Option<ScrollGlide>,
    /// 各区域（文件树、列表、预览）跳转式滚动的过渡，每个区域至多一项。
    pub(in crate::app) pane_glides: Vec<scroll::PaneGlide>,
    /// 触摸滚动抬手后的惯性滚动；没有时为 `None`。
    pub(in crate::app) fling: Option<scroll::Fling>,
    /// 滚轮与触控板滚动的状态。
    pub(in crate::app) wheel: scroll::Wheel,
}
