//! macOS 上用 CoreGraphics（`CGPDFDocument`）打开和渲染 PDF。
//!
//! 页面按裁切框（CropBox）显示并按页面的旋转角度转正；画进 sRGB 的位图，先铺白色底。

#![allow(unsafe_code)] // 位图上下文直接写入 Rust 分配的像素缓冲区，需要 `CGBitmapContextCreate`

use std::path::Path;

use objc2_core_foundation::{CFRetained, CFString, CFURL, CFURLPathStyle, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGInterpolationQuality,
    CGPDFBox, CGPDFDocument, CGPDFPage, kCGColorSpaceSRGB,
};
use winc_i18n::t;

use super::PageSize;
use crate::image::Pixels;

/// 用 CoreGraphics 打开的 PDF 文档。
pub(super) struct Document {
    doc: CFRetained<CGPDFDocument>,
}

/// 页面 `page` 的显示区域：裁切框和顺时针旋转的角度（0、90、180、270）。
fn page_box(page: &CGPDFPage) -> (CGRect, i32) {
    let rect = CGPDFPage::box_rect(Some(page), CGPDFBox::CropBox);
    let rotation = CGPDFPage::rotation_angle(Some(page)).rem_euclid(360);
    (rect, rotation)
}

impl Document {
    /// 打开 `path`，返回文档和各页大小。
    pub(super) fn open(path: &Path) -> Result<(Document, Vec<PageSize>), String> {
        let invalid = || t!("preview-pdf-invalid").to_owned();
        let text = path.to_str().ok_or_else(invalid)?;
        let text = CFString::from_str(text);
        let url = CFURL::with_file_system_path(
            None,
            Some(&text),
            CFURLPathStyle::CFURLPOSIXPathStyle,
            false,
        )
        .ok_or_else(invalid)?;
        let doc = CGPDFDocument::with_url(Some(&url)).ok_or_else(invalid)?;
        if CGPDFDocument::is_encrypted(Some(&doc)) && !CGPDFDocument::is_unlocked(Some(&doc)) {
            return Err(t!("preview-pdf-encrypted").to_owned());
        }
        let count = CGPDFDocument::number_of_pages(Some(&doc));
        let mut pages = Vec::with_capacity(count);
        // CoreGraphics 的页号从 1 起。
        for number in 1..=count {
            let Some(page) = CGPDFDocument::page(Some(&doc), number) else {
                return Err(invalid());
            };
            let (rect, rotation) = page_box(&page);
            let (w, h) = (rect.size.width.abs(), rect.size.height.abs());
            let (width, height) = if rotation % 180 == 0 { (w, h) } else { (h, w) };
            pages.push(PageSize { width, height });
        }
        Ok((Document { doc }, pages))
    }

    /// 把第 `index` 页画成 `width × height` 像素的位图。
    pub(super) fn render(&self, index: usize, width: u32, height: u32) -> Result<Pixels, String> {
        let failed = || t!("preview-pdf-render-failed").to_owned();
        let page = CGPDFDocument::page(Some(&self.doc), index + 1).ok_or_else(failed)?;
        let (w, h) = (width as usize, height as usize);
        // SAFETY: `kCGColorSpaceSRGB` 是 CoreGraphics 导出的常量字符串。
        let space =
            CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).ok_or_else(failed)?;
        let len = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(failed)?;
        let mut rgba = vec![0u8; len];
        // SAFETY: `rgba` 有 `w × h × 4` 字节，每行 `w × 4` 字节，与传入的尺寸一致；每像素 R、G、B
        // 和一个不用的字节（`NoneSkipLast`），每个分量 8 位。`rgba` 在 `context` 释放之前一直存活
        // 且不被移动。
        let context = unsafe {
            CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                w,
                h,
                8,
                w * 4,
                Some(&space),
                CGImageAlphaInfo::NoneSkipLast.0,
            )
        }
        .ok_or_else(failed)?;
        let full = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64));
        CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
        CGContext::fill_rect(Some(&context), full);
        CGContext::set_interpolation_quality(Some(&context), CGInterpolationQuality::High);

        // 把裁切框的中心移到位图中心，按旋转角度转正，再按比例缩放到位图大小。
        // `CGPDFPageGetDrawingTransform` 只缩小、不放大，因此在此自行计算变换。
        let (rect, rotation) = page_box(&page);
        let (pw, ph) = (rect.size.width.abs(), rect.size.height.abs());
        let (shown_w, shown_h) = if rotation % 180 == 0 {
            (pw, ph)
        } else {
            (ph, pw)
        };
        if shown_w <= 0.0 || shown_h <= 0.0 {
            return Err(failed());
        }
        let scale = (w as f64 / shown_w).min(h as f64 / shown_h);
        CGContext::translate_ctm(Some(&context), w as f64 / 2.0, h as f64 / 2.0);
        // PDF 的 `Rotate` 是顺时针，CoreGraphics 的角度是逆时针。
        CGContext::rotate_ctm(Some(&context), -(rotation as f64).to_radians());
        CGContext::scale_ctm(Some(&context), scale, scale);
        let (cx, cy) = (
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        );
        CGContext::translate_ctm(Some(&context), -cx, -cy);
        CGContext::clip_to_rect(Some(&context), rect);
        CGContext::draw_pdf_page(Some(&context), Some(&page));
        CGContext::flush(Some(&context));
        drop(context);
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        Ok(Pixels {
            width,
            height,
            rgba,
        })
    }
}
