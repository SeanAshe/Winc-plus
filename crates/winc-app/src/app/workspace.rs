//! 工作区：打开的目录与其中的文件。
//!
//! 这一组模块覆盖工作区的全部功能：目录的现场（`session`）、文件树与拖动（`tree`、`tree_drag`）、
//! 工具栏收起（`tool_fold`）、两个文件的比较（`compare`）、监视磁盘变化（`watch`）、打开与保存
//! （`loading`、`files`、`background_save`）、标签切换与拖动（`tab_drag`）、快速打开（`picker`）、
//! 最近打开的目录（`recent`）、全局搜索（`global`）和拖入文件（`file_drop`）。

use super::*;

pub(super) mod background_save;
pub(super) mod compare;
pub(super) mod file_drop;
pub(super) mod files;
pub(super) mod global;
pub(super) mod loading;
pub(super) mod picker;
pub(super) mod recent;
pub(super) mod session;
pub(super) mod tab_drag;
pub(super) mod tool_fold;
pub(super) mod tree;
pub(super) mod tree_drag;
pub(super) mod tree_label;
pub(super) mod watch;
