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
