//! 应用和窗口的生命周期。
//!
//! 启动、事件循环、窗口和绘制、外观、提示条、关闭和退出、热退出的备份、临时打开模式。

use super::*;

pub(super) mod alert;
pub(super) mod appearance;
pub(super) mod backup;
pub(super) mod closing;
pub(super) mod events;
#[cfg(target_os = "macos")]
mod install;
pub(super) mod notice;
pub(super) mod one_off;
pub(super) mod placement;
mod shell_command;
pub(super) mod startup;
pub(super) mod title_bar;
pub(super) mod wallpaper;
pub(super) mod window;
