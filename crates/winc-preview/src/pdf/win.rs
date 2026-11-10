//! Windows 上用 `Windows.Data.Pdf`（Windows 10 起系统自带）打开和渲染 PDF。
//!
//! 页面先渲染成内存中的 PNG，再由 Windows 图像组件（WIC）的 `BitmapDecoder` 解成 RGBA 像素：
//! `Windows.Data.Pdf` 只能把页面输出成编码后的图片流。输出的图片大小由系统决定，可能与请求的
//! 大小相差一两个像素，解码时缩放到请求的大小。异步接口在渲染线程上阻塞等待结果
//! （`join`），首次调用时 `windows-core` 为该线程初始化多线程单元（MTA）。

use std::path::Path;

use winc_i18n::t;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Storage::StorageFile;
use windows::Storage::Streams::InMemoryRandomAccessStream;
use windows::core::HSTRING;

use super::PageSize;
use crate::image::Pixels;

/// 设备无关像素（DIP，1/96 英寸）换成点（1/72 英寸）的比例。
const POINTS_PER_DIP: f64 = 72.0 / 96.0;

/// 打开失败时给用户看的原因；系统给出的错误码对用户没有意义，统一说明为无效的 PDF。
fn invalid(_: windows::core::Error) -> String {
    t!("preview-pdf-invalid").to_owned()
}

/// 渲染失败时给用户看的原因，附系统的错误码，便于排查。
fn failed(error: windows::core::Error) -> String {
    let code = format!("0x{:08X}", error.code().0 as u32);
    t!("preview-pdf-render-failed-code", code = code).to_owned()
}

/// 用 `Windows.Data.Pdf` 打开的 PDF 文档。
pub(super) struct Document {
    doc: PdfDocument,
}

impl Document {
    /// 打开 `path`，返回文档和各页大小。
    pub(super) fn open(path: &Path) -> Result<(Document, Vec<PageSize>), String> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))
            .and_then(|op| op.join())
            .map_err(|e| e.message())?;
        let doc = PdfDocument::LoadFromFileAsync(&file)
            .and_then(|op| op.join())
            .map_err(invalid)?;
        if doc.IsPasswordProtected().unwrap_or(false) {
            return Err(t!("preview-pdf-encrypted").to_owned());
        }
        let count = doc.PageCount().map_err(invalid)?;
        let mut pages = Vec::with_capacity(count as usize);
        for index in 0..count {
            // `Size` 已按页面的裁切框和旋转换算成显示方向，单位为 DIP。
            let size = doc
                .GetPage(index)
                .and_then(|page| page.Size())
                .map_err(invalid)?;
            pages.push(PageSize {
                width: f64::from(size.Width) * POINTS_PER_DIP,
                height: f64::from(size.Height) * POINTS_PER_DIP,
            });
        }
        Ok((Document { doc }, pages))
    }

    /// 把第 `index` 页画成 `width × height` 像素的位图。
    pub(super) fn render(&self, index: usize, width: u32, height: u32) -> Result<Pixels, String> {
        let index = u32::try_from(index).map_err(|_| t!("preview-pdf-render-failed").to_owned())?;
        let page = self.doc.GetPage(index).map_err(failed)?;
        let options = PdfPageRenderOptions::new().map_err(failed)?;
        options.SetDestinationWidth(width).map_err(failed)?;
        options.SetDestinationHeight(height).map_err(failed)?;
        let stream = InMemoryRandomAccessStream::new().map_err(failed)?;
        page.RenderWithOptionsToStreamAsync(&stream, &options)
            .and_then(|op| op.join())
            .map_err(failed)?;
        stream.Seek(0).map_err(failed)?;
        let decoder = BitmapDecoder::CreateAsync(&stream)
            .and_then(|op| op.join())
            .map_err(failed)?;
        // 系统输出的图片可能与请求的大小相差一两个像素：缩放到请求的大小，调用方按该大小绘制。
        let transform = BitmapTransform::new().map_err(failed)?;
        transform.SetScaledWidth(width).map_err(failed)?;
        transform.SetScaledHeight(height).map_err(failed)?;
        transform
            .SetInterpolationMode(BitmapInterpolationMode::Fant)
            .map_err(failed)?;
        // 背景为不透明的白色（`PdfPageRenderOptions` 的默认值），因此按不预乘的 RGBA 取出像素。
        let provider = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Rgba8,
                BitmapAlphaMode::Straight,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .and_then(|op| op.join())
            .map_err(failed)?;
        let data = provider.DetachPixelData().map_err(failed)?;
        Ok(Pixels {
            width,
            height,
            rgba: data.to_vec(),
        })
    }
}
