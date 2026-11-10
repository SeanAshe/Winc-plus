//! PDF 的逐页渲染：用系统的 PDF 渲染器（macOS 的 CoreGraphics、Windows 的
//! `Windows.Data.Pdf`）把一页画成指定大小的位图，只做预览，不解析文字和链接。
//!
//! 打开文档时只读取页数和各页大小；渲染按页、按指定大小进行，调用方只渲染视口附近的页，内存只占
//! 这几页的位图。系统的 PDF 对象不能跨线程使用：[`Document`] 只能在打开它的线程上使用，调用方
//! 通常为每个打开的 PDF 启动一个渲染线程。其他平台不支持，[`supported`] 返回 `false`。

use std::path::Path;

use crate::image::Pixels;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as platform;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as platform;

/// 一页的大小（点，1/72 英寸）：已按页面的旋转换算，宽高是显示时的方向。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    /// 宽度（点）。
    pub width: f64,
    /// 高度（点）。
    pub height: f64,
}

/// 渲染出的位图每条边的上限（像素）：超过时不渲染。
///
/// 用来限制单页位图的内存：按每像素 4 字节，16384 × 16384 的位图约占 1 GiB。按显示大小渲染时
/// 远达不到该上限。
pub const MAX_SIDE: u32 = 16384;

/// 当前平台能否预览 PDF：macOS 与 Windows 能，其他平台不能。
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

/// `path` 是否是 PDF 文件（按扩展名判断）。
pub fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// 打开的 PDF 文档；只能在打开它的线程上使用。
pub struct Document {
    #[cfg(any(target_os = "macos", windows))]
    inner: platform::Document,
    /// 各页的大小，按页序排列。
    pages: Vec<PageSize>,
}

impl Document {
    /// 打开 PDF 文件 `path`，读出页数和各页大小。
    ///
    /// # Errors
    ///
    /// 当前平台不支持、文件无法读取、不是有效的 PDF、加密或没有页时返回给用户看的原因。
    pub fn open(path: &Path) -> Result<Document, String> {
        #[cfg(any(target_os = "macos", windows))]
        {
            let (inner, pages) = platform::Document::open(path)?;
            if pages.is_empty() {
                return Err(winc_i18n::t!("preview-pdf-empty").to_owned());
            }
            Ok(Document { inner, pages })
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = path;
            Err(winc_i18n::t!("preview-pdf-unsupported").to_owned())
        }
    }

    /// 各页的大小，按页序排列；至少有一页。
    pub fn pages(&self) -> &[PageSize] {
        &self.pages
    }

    /// 把第 `index` 页（从 0 起）画成 `width × height` 像素的位图，白色底。
    ///
    /// 页面按自身比例缩放到位图里，宽高比与 [`PageSize`] 一致时恰好铺满。返回的位图大小与请求的
    /// 大小完全相同。
    ///
    /// # Errors
    ///
    /// 页号超出范围、尺寸为 0 或超过 [`MAX_SIDE`]、系统渲染失败，或渲染结果的大小与请求不符时
    /// 返回原因。
    pub fn render(&self, index: usize, width: u32, height: u32) -> Result<Pixels, String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
        }
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
        }
        #[cfg(any(target_os = "macos", windows))]
        {
            let pixels = self.inner.render(index, width, height)?;
            let size = (width as usize) * (height as usize) * 4;
            if (pixels.width, pixels.height) != (width, height) || pixels.rgba.len() != size {
                return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
            }
            Ok(pixels)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            Err(winc_i18n::t!("preview-pdf-unsupported").to_owned())
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
