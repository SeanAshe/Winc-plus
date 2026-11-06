//! 文字：加载字体、按名字查找字体文件、字形缓存，以及字形轮廓（SVG 预览里的文字使用）。

use super::*;
pub(crate) mod font;
pub(crate) mod font_list;
pub(crate) mod font_path;
pub(crate) mod glyphs;
pub(crate) mod outline;
pub(crate) mod shape;
pub(crate) mod system_outline;
