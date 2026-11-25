//! 编辑区的测试：编辑、多光标、折叠、格式化、导航、粘贴、滚动与滚动条、空白符、十六进制视图。

use super::*;

mod compare;
mod control_keys;
pub(super) mod cursors;
pub(super) mod editing;
pub(super) mod folding;
mod hex;
mod links;
mod menu;
mod scratch;
mod text_drag;
// 格式化的测试用 sh 脚本冒充格式化工具。
#[cfg(unix)]
pub(super) mod format;
mod http;
pub(super) mod navigation;
pub(super) mod paste;
pub(super) mod scrollbar;
pub(super) mod scrolling;
mod vim;
pub(super) mod whitespace;
