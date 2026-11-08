//! 预览的绘制：Markdown 预览的排版和绘制、表格预览、PDF 预览。
//!
//! `markdown` 排版并绘制 Markdown、图片和公式，`table` 绘制 xlsx、csv 工作簿的表格，`pdf` 逐页
//! 绘制渲染好的 PDF 页面；表格的
//! 读取（xlsx 包、数字格式、单元格文字）在 `winc_preview::table`，这一层只管排版、绘制和
//! 命中测试。
//!
//! 图片预览画在编辑区里，见 `editor/picture.rs`。

pub(crate) mod markdown;
pub(in crate::view) mod pdf;
pub(crate) mod table;
