//! 输入：键盘和输入法、鼠标、执行键位表里的命令、右键菜单、命令面板。
//!
//! 快捷键与命令面板的内容都来自键位表；鼠标事件按绘制时记下的可点击区域找到目标。

use super::*;

pub(super) mod actions;
pub(super) mod context_menu;
pub(super) mod keyboard;
pub(super) mod mouse;
pub(super) mod palette;
pub(super) mod touch;
