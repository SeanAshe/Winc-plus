//! 打开的目录（工作区）：文件树、快速打开的文件索引、全局搜索和目录变化监视。
//!
//! 四个子模块分别维护文件树 [`tree::Workspace`]、⌘P 的文件表 [`picker::FileIndex`]、全局搜索
//! [`project_search::ProjectSearch`] 和系统通知的接收 [`watch::Watcher`]；界面层持有它们，并在
//! 用户操作与后台结果之间转发。

pub(crate) mod copying;
pub(crate) mod picker;
pub(crate) mod project_search;
mod remote_files;
pub(crate) mod tree;
pub(crate) mod watch;
