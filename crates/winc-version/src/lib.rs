//! 产品版本号。
//!
//! 版本号写在仓库根目录的 `VERSION` 文件里，编译时嵌入；发布脚本读的是同一个文件
//! （`release/common/version.mjs`）。各 crate 的包版本（`CARGO_PKG_VERSION`）不随发布变化：
//! 包版本参与编译产物的散列，一变所有 crate 都要从头重新编译，旧产物留在 `target` 里。

#[cfg(test)]
mod tests;

/// 产品版本号，例如 `0.1.592`。
pub const VERSION: &str = include_str!("../../../VERSION").trim_ascii();
