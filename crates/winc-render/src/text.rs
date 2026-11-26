//! 文字：加载字体、按名字查找字体文件、字形缓存，以及字形轮廓（SVG 预览里的文字使用）。

use super::*;
pub(crate) mod font;
pub(crate) mod font_list;
pub(crate) mod font_path;
#[cfg(target_os = "linux")]
pub(crate) mod fontconfig;
/// 没有 fontconfig 的平台：各查询返回空值，调用方使用平台自己的字体来源。
#[cfg(not(target_os = "linux"))]
pub(crate) mod fontconfig {
    use std::path::PathBuf;

    use super::font_path::FontFile;

    /// 一次 fontconfig 匹配的结果。
    pub(crate) struct Match {
        /// 字体文件。
        pub(crate) file: FontFile,
    }

    /// 返回 fontconfig 对 `pattern` 的最佳匹配；此平台总是返回 `None`。
    pub(crate) fn best(_pattern: &str) -> Option<Match> {
        None
    }

    /// 返回 fontconfig 为 `pattern` 排序的字体；此平台总是返回空列表。
    pub(crate) fn sorted(_pattern: &str) -> Vec<FontFile> {
        Vec::new()
    }

    /// 返回通用字族 `pattern` 的回退顺序；此平台总是返回空列表。
    pub(crate) fn chain(_pattern: &str) -> Vec<FontFile> {
        Vec::new()
    }

    /// 返回 fontconfig 列出的全部字体文件；此平台总是返回 `None`。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn files() -> Option<Vec<PathBuf>> {
        None
    }

    /// 返回 fontconfig 是否可用；此平台总是返回假。
    pub(crate) fn available() -> bool {
        false
    }

    /// 按字面字族名查询；此平台总是返回 `None`。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn lookup(_family: &str, _bold: bool, _italic: bool) -> Option<Match> {
        None
    }
}
pub(crate) mod glyphs;
pub(crate) mod outline;
pub(crate) mod shape;
pub(crate) mod system_outline;
