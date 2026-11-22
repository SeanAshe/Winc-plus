//! 解析时启用的语法扩展，以及维基链接的地址。

use pulldown_cmark::{LinkType, Options};

/// 维基链接 `[[名字]]` 的地址前缀，其后是名字（可以带 `#标题`）：按文件名查找，不是路径。
pub const WIKI: &str = "\u{1}wiki:";

/// 解析时启用的扩展：GFM 的表格、删除线、任务列表、提示块，以及脚注、文首的 YAML、公式和
/// 维基链接。
pub(super) fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_WIKILINKS
}

/// 返回 `kind` 类链接的地址：维基链接在 `dest` 之前加上 [`WIKI`]。
pub(super) fn link_url(kind: LinkType, dest: &str) -> String {
    match kind {
        LinkType::WikiLink { .. } => format!("{WIKI}{dest}"),
        _ => dest.to_owned(),
    }
}
