//! 文件系统的访问入口：区分本机路径与远端主机上的路径。
//!
//! 远端主机上的文件用虚拟路径表示（见 [`virtual_path`]），与本机路径共用 `Path` 类型，上层
//! 代码不区分两者。虚拟路径只在本 crate 的边界换算为连接目标和远端路径（见 [`locate`]），
//! 不对应本机上的任何文件；只能访问本机文件的操作用 [`ensure_local`] 拒绝虚拟路径。
//!
//! 文件操作在 [`fs`] 中，上层代码经它访问文件，不直接调用 `std::fs`。

#[cfg(any(windows, test))]
mod escape;
pub mod fs;
pub mod local;
mod path;
pub mod remote;
mod target;

pub use path::{
    Location, display, ensure_local, is_remote, join, locate, native_lossy, relative_lossy,
    virtual_path,
};
pub use target::Target;
