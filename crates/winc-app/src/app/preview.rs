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
