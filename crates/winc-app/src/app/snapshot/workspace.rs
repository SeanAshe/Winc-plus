//! 打开的目录与文件的测试：快速打开、全局搜索、侧栏与文件树、拖放、标签栏、现场的保存与
//! 恢复、目录监视。

use super::*;

pub(super) mod folders;
#[cfg(unix)]
mod links;
pub(super) mod loading;
mod new_files;
pub(super) mod outline;
mod panes;
pub(super) mod picker;
mod places;
mod project;
pub(super) mod recent;
pub(super) mod search;
mod search_dialog;
mod search_tree;
pub(super) mod selection;
pub(super) mod session;
pub(super) mod tab_selection;
pub(super) mod tabs;
pub(super) mod tree;
mod tree_drag;
pub(super) mod watch;
