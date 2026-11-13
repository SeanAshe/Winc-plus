//! 预览的解析和光栅化：图片解码成像素、SVG 画成像素、Markdown 解析成块、表格读成单元格。
//!
//! 各模块互不依赖：`image` 解码位图并缩放，`svg` 解析矢量图并按显示大小绘制，`markdown`
//! 把 Markdown 转成排版用的块和行内文字，`table` 把 xlsx、csv 读成各行的单元格文字。
//!
//! 只认字节和文本，不知道文档、标签和界面；编辑器决定什么时候预览，界面负责排版和绘制。

// 测试直接在本机的临时目录中准备和检查文件。
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod docx;
pub mod image;
pub mod markdown;
mod ooxml;
pub mod pdf;
pub mod svg;
pub mod table;
mod zip;
