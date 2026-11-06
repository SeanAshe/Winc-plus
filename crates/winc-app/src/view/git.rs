//! Git 模块的绘制：提交视图、日志标签、行号栏改动的卡片、文件里的合并冲突。
//!
//! 提交视图在侧栏，日志标签占一个编辑器标签页，改动卡片浮在编辑区上，合并冲突画在文件内容里。
//! 要画的数据（状态、日志、选中的提交、冲突位置）都在 `crate::app::git` 的字段里，这里只读取
//! 和绘制，并把可点击的范围写回 `App`。

pub(super) mod changes;
pub(super) mod commit;
pub(super) mod log;
pub(super) mod merge;
pub(super) mod picker;
