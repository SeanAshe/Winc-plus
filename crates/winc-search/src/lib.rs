//! 查找的核心：匹配器、按打开文件的规则解码、遍历工作区目录，以及并行地逐块搜索文件。
//!
//! 不依赖界面。编辑器的文件内查找与全局搜索用它，远端主机上的服务进程也用它在远端搜索（见
//! [`host`]）：两边按同一套规则判断二进制、识别编码、分行和匹配，搜出的行号、列与打开后的文本
//! 一致。

mod engine;
mod file;
pub mod host;
mod lines;
mod matcher;
mod query;
mod scan;
pub mod text;
mod walk;

pub use engine::{Budget, MAX_THREADS, Scope, Searcher, search_parallel};
pub use file::{
    BINARY_SNIFF, LARGE_FILE, LINE_CAP, OPEN_LIMIT, UTF16_INLINE, complete_prefix, looks_binary,
    visible_len,
};
pub use lines::{LineMatch, for_each_line, search_lines};
pub use matcher::*;
pub use query::{Filter, GlobError, Query};
pub use scan::search_file;
pub use walk::{
    DEPENDENCY_FOLDERS, HIDDEN, MATCHER_DIRS, Probe, dependency_folders, descend, gather,
    is_listed_file, is_repository, natural_cmp, nested_repositories, probe, slashed, walker,
    walker_with_ignored,
};
