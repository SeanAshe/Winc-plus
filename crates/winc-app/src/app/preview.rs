//! 预览：Markdown（后台解析、滚动、链接和图片）、图片和 SVG（后台解码、分栏预览）、表格（后台
//! 读取 xlsx、csv）、PDF（渲染线程逐页渲染）。
//!
//! 各种预览都在后台线程完成解码、解析和绘制，界面只收结果；排版与绘制在 `view` 模块中完成。

use std::path::Path;

use winc_i18n::t;

use crate::app::App;

pub(crate) mod images;
pub(crate) mod markdown;
pub(crate) mod pdf;
pub(crate) mod tables;

/// 能在浏览器中打开的网页文件的扩展名（小写）。
const PAGE_EXTENSIONS: &[&str] = &["html", "htm", "xhtml"];

impl App {
    /// 当前标签是磁盘上的网页文件时返回它的路径（标签栏右端显示「在浏览器中打开」）。
    pub(crate) fn browser_page(&self) -> Option<&Path> {
        let doc = self.editor.active()?;
        if doc.unsupported.is_some() || doc.title.is_some() {
            return None;
        }
        let path = doc.path.as_deref()?;
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        PAGE_EXTENSIONS
            .contains(&extension.as_str())
            .then_some(path)
    }

    /// 用默认浏览器打开当前的网页文件。
    ///
    /// 浏览器读的是磁盘上的文件：有未保存的修改时在状态栏提示。
    pub(in crate::app) fn open_in_browser(&mut self) {
        let Some(path) = self.browser_page().map(Path::to_path_buf) else {
            return;
        };
        let dirty = self.editor.active().is_some_and(|d| d.is_dirty());
        match crate::system::open_in_browser(&path) {
            Ok(()) if dirty => self.flash(t!("preview-browser-unsaved")),
            Ok(()) => {}
            Err(e) => self.flash(t!("preview-browser-failed", error = e.to_string())),
        }
    }
}

/// 各类预览的状态：图片、表格、分栏、Markdown 与 PDF。
pub(crate) struct PreviewState {
    /// 图片预览缩放后的图片缓存。
    pub(crate) picture_cache: Option<images::PictureCache>,
    /// 当前图片预览的缩放倍数与滚动位置。
    pub(crate) picture_view: images::PictureView,
    /// 后台的图片解码和 SVG 绘制任务。
    pub(in crate::app) images: images::ImageJobs,
    /// 后台的表格读取任务。
    pub(in crate::app) tables: tables::TableJobs,
    /// 表格预览上次绘制的几何：工作表标签、最大滚动距离和列宽缓存。
    pub(crate) table_layout: tables::TableLayout,
    /// 分栏预览右侧的图片。
    pub(crate) split: images::SplitPreview,
    /// Markdown 预览（分栏右侧或仅预览模式）。
    pub(crate) markdown: markdown::MarkdownPreview,
    /// 当前标签的 PDF 预览。
    pub(crate) pdf: pdf::PdfPreview,
    /// 分栏预览时源代码一侧占编辑区宽度的比例，取值 0 到 1；`None` 表示各占一半。
    ///
    /// 拖动中间的分隔线时改变，所有分栏预览的标签共用该值。
    pub(in crate::app) split_share: Option<f32>,
}
