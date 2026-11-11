//! 编辑：编辑命令、多光标与列选择、折叠与块识别、文件内查找、粘贴。
//!
//! `command` 把按键映射成 [`Command`] 并在 [`Document`] 上执行；`cursors` 负责多光标与列选择；
//! `folding` 与 `scopes` 提供折叠状态、显示行换算和块识别；`search` 是文件内查找与替换；
//! `paste` 处理粘贴文件与图片。

pub(crate) mod command;
pub(crate) mod cursors;
pub(crate) mod folding;
pub(crate) mod long_lines;
pub mod paste;
pub mod scopes;
pub(crate) mod search;
