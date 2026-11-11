//! 编辑区的测试：编辑、多光标、折叠、格式化、导航、粘贴、滚动与滚动条、空白符。

use super::*;

pub(super) mod cursors;
pub(super) mod editing;
pub(super) mod folding;
// 格式化的测试用 sh 脚本冒充格式化工具。
#[cfg(unix)]
pub(super) mod format;
pub(super) mod navigation;
pub(super) mod paste;
pub(super) mod scrollbar;
pub(super) mod scrolling;
mod vim;
pub(super) mod whitespace;
