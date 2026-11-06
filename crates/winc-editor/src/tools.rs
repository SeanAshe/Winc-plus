//! 外部程序：获取用户 shell 的环境、带超时和取消地运行程序，以及格式化与修复文档。
//!
//! [`shell`] 提供环境与进程运行，[`format`] 在其上实现格式化工具的自动选择、调用与结果应用。

pub mod format;
pub mod shell;
pub mod wsl;
