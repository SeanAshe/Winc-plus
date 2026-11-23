//! 由系统提供字形轮廓：字形只放在 swash 读不了的表中的字体，改由 CoreText 取轮廓。
//!
//! macOS 15 起，系统自带的苹方把字形放在 Apple 未公开格式的 `hvgl` 表中，没有 `glyf`、`CFF`。
//! 这类字体的字符映射、字宽和竖向尺寸仍在标准的表中，由 swash 读取，只有轮廓由 CoreText
//! 提供。`unsafe` 调用集中在 `macos` 模块。

use swash::zeno::{Point, Verb};

#[cfg(target_os = "macos")]
pub(crate) use macos::SystemOutlines;

/// 一个字形的轮廓：点和绘制命令分开存放，与 swash 的轮廓格式相同。
#[derive(Debug, Default)]
pub(crate) struct GlyphPath {
    /// 各命令用到的点，依次排列。
    pub points: Vec<Point>,
    /// 绘制命令。
    pub verbs: Vec<Verb>,
}

impl GlyphPath {
    /// 返回 zeno 能直接使用的路径数据。
    pub(crate) fn data(&self) -> (&[Point], &[Verb]) {
        (&self.points, &self.verbs)
    }
}

/// 填充好的字形：每像素 1 字节的覆盖率，自上而下逐行存放。
///
/// 其他平台没有系统轮廓，不会构造该类型。
#[derive(Debug)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct Coverage {
    /// 位图左边缘相对笔位置的横向偏移（物理像素，向右为正）。
    pub left: i32,
    /// 位图上边缘相对基线的高度（物理像素，向上为正）。
    pub top: i32,
    /// 位图宽度（物理像素）。
    pub width: usize,
    /// 位图高度（物理像素）。
    pub height: usize,
    /// 覆盖率，`width × height` 字节。
    pub data: Vec<u8>,
}

/// 其他平台没有可用的系统轮廓：该类型没有值，字形只放在 `hvgl` 表中的字体视为不能用。
#[cfg(not(target_os = "macos"))]
pub(crate) enum SystemOutlines {}

#[cfg(not(target_os = "macos"))]
impl SystemOutlines {
    /// 其他平台没有系统轮廓：始终返回 `None`。
    pub(crate) fn new(
        _path: &std::path::Path,
        _postscript: &str,
        _font: &swash::FontRef,
    ) -> Option<SystemOutlines> {
        None
    }

    /// 其他平台没有系统轮廓：该类型没有值，不会调用。
    pub(crate) fn outline(&self, _glyph: u16, _em: f32, _path: &mut GlyphPath) {
        match *self {}
    }

    /// 其他平台没有系统轮廓：该类型没有值，不会调用。
    pub(crate) fn fill(
        &self,
        _glyph: u16,
        _size: f32,
        _dx: f32,
        _slant: f32,
        _bold: f32,
    ) -> Option<Coverage> {
        match *self {}
    }
}

/// CoreText 调用的封装。
///
/// CoreText 的字体对象创建后不可变；本模块只持有并查询该对象，不修改。每个 `unsafe` 块上方的
/// `// SAFETY:` 注释写明了所依赖的前提。
#[cfg(target_os = "macos")]
// 本模块的 `unsafe` 仅用于调用 CoreText 和 CoreGraphics，不扩散到其他模块。
#[allow(unsafe_code)]
mod macos {
    use std::ffi::c_void;
    use std::path::Path;
    use std::ptr::NonNull;

    use objc2_core_foundation::{
        CFDictionary, CFRetained, CFString, CFType, CFURL, CGAffineTransform,
    };
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGContext, CGImageAlphaInfo, CGLineJoin, CGPath, CGPathDrawingMode,
        CGPathElement, CGPathElementType,
    };
    use objc2_core_text::{CTFont, CTFontDescriptor, kCTFontNameAttribute, kCTFontURLAttribute};
    use swash::FontRef;
    use swash::zeno::{Point, Verb};

    use super::{Coverage, GlyphPath};

    /// 创建 CoreText 字体时用的字号：CoreText 返回的轮廓以该字号为 1 em。
    ///
    /// 与常见字体每 em 的单位数同一量级，换算时不损失精度。
    const SIZE: f64 = 1000.0;

    /// 核对字形编号时用的字符：拉丁字母、数字和常用汉字，覆盖一个字体中的不同区段。
    const PROBES: [char; 4] = ['a', '0', '中', '。'];

    /// 一个由 CoreText 提供轮廓的字体。
    pub(crate) struct SystemOutlines {
        /// 字号为 [`SIZE`] 的 CoreText 字体。
        font: CFRetained<CTFont>,
    }

    // SAFETY: CoreText 的字体对象创建后不可变，Apple 的文档说明可以在多个线程中同时使用；
    // 本类型只持有该对象并做只读查询。
    unsafe impl Send for SystemOutlines {}
    // SAFETY: CoreText 的字体对象创建后不可变，多个线程只能通过 `&self` 做只读查询（描轮廓、取
    // 字宽），不修改共享状态。
    unsafe impl Sync for SystemOutlines {}

    impl SystemOutlines {
        /// 为 swash 从文件 `path` 中打开的字体 `font` 提供轮廓；无法提供时返回 `None`。
        ///
        /// `postscript` 是该字体（或所用命名实例）的 PostScript 名。文件中没有该名字的字体，或
        /// CoreText 的字形编号与 `font` 不一致时返回 `None`；字符到字形的映射由 swash 完成，轮廓按
        /// 字形编号向 CoreText 索取。
        ///
        /// CoreText 字体从该文件本身创建，不按名字创建，避免取到系统中同名字体的其他副本。
        pub(crate) fn new(path: &Path, postscript: &str, font: &FontRef) -> Option<SystemOutlines> {
            let descriptor = descriptor_in(path, postscript)?;
            // SAFETY: `descriptor` 是下面取到的有效描述符，调用期间保持存活；矩阵参数传 null
            // 表示使用单位矩阵，`CTFontCreateWithFontDescriptor` 接受该值。
            let system =
                unsafe { CTFont::with_font_descriptor(&descriptor, SIZE, std::ptr::null()) };

            let charmap = font.charmap();
            let same = PROBES.iter().all(|&c| {
                let mut units = [0u16; 2];
                let units = c.encode_utf16(&mut units);
                let mut glyphs = [0u16; 2];
                // SAFETY: 两个指针都指向本闭包中的局部数组，长度为 2，不小于 `units.len()`；
                // 调用期间保持有效。
                let mapped = unsafe {
                    system.glyphs_for_characters(
                        NonNull::from(&mut units[0]),
                        NonNull::from(&mut glyphs[0]),
                        units.len() as isize,
                    )
                };
                let theirs = if mapped { glyphs[0] } else { 0 };
                theirs == charmap.map(c)
            });
            same.then_some(SystemOutlines { font: system })
        }

        /// 把字形 `glyph` 的轮廓按 1 em = `em` 缩放后追加到 `path`，`y` 轴向上（与字体坐标相同）。
        ///
        /// 字形没有轮廓（如空格）时不追加任何命令。
        pub(crate) fn outline(&self, glyph: u16, em: f32, path: &mut GlyphPath) {
            // SAFETY: `self.font` 在 `self` 存活期间有效；矩阵参数传 null 表示不做变换，
            // `CTFontCreatePathForGlyph` 接受该值。没有轮廓时返回 `None`。
            let Some(outline) = (unsafe { self.font.path_for_glyph(glyph, std::ptr::null()) })
            else {
                return;
            };
            let mut sink = Sink {
                path,
                scale: em / SIZE as f32,
            };
            // SAFETY: `outline` 是上面创建的有效路径；`info` 指向本函数中的 `sink`，
            // `CGPathApply` 同步调用 `element`，返回之前不会保留该指针。
            unsafe { CGPath::apply(Some(&outline), (&raw mut sink).cast(), Some(element)) };
        }

        /// 把字形 `glyph` 填充成覆盖率位图；字形没有轮廓（如空格）时返回 `None`。
        ///
        /// `size` 是字号（物理像素），`dx` 是整体的右移量（亚像素定位），`slant` 是倾斜角的正切
        /// （0 为直立），`bold` 大于 0 时沿轮廓再描一圈该像素宽度的边。由 CoreGraphics 按面积计算
        /// 覆盖率。
        pub(crate) fn fill(
            &self,
            glyph: u16,
            size: f32,
            dx: f32,
            slant: f32,
            bold: f32,
        ) -> Option<Coverage> {
            let scale = f64::from(size) / SIZE;
            // CoreGraphics 的 y 轴向上：x 加上 y 乘以正切，越往上越靠右。
            let transform = CGAffineTransform {
                a: scale,
                b: 0.0,
                c: scale * f64::from(slant),
                d: scale,
                tx: f64::from(dx),
                ty: 0.0,
            };
            // SAFETY: `self.font` 在 `self` 存活期间有效；`transform` 是本函数的局部变量，
            // 调用期间有效。没有轮廓时返回 `None`。
            let outline = unsafe { self.font.path_for_glyph(glyph, &transform) }?;
            if CGPath::is_empty(Some(&outline)) {
                return None;
            }
            let bounds = CGPath::path_bounding_box(Some(&outline));
            // 描边向轮廓两侧各伸出一半。
            let grow = f64::from(bold) / 2.0;
            let (left, bottom) = (
                (bounds.origin.x - grow).floor(),
                (bounds.origin.y - grow).floor(),
            );
            let right = (bounds.origin.x + bounds.size.width + grow).ceil();
            let top = (bounds.origin.y + bounds.size.height + grow).ceil();
            let (width, height) = ((right - left) as usize, (top - bottom) as usize);
            if width == 0 || height == 0 {
                return None;
            }

            let mut data = vec![0u8; width * height];
            // SAFETY: `data` 有 `width × height` 字节，每行 `width` 字节，与传入的尺寸一致；
            // 只有透明度通道的位图不需要色彩空间（传 `None`）。`data` 在 `context` 释放之前
            // 一直存活且不被移动。
            let context = unsafe {
                CGBitmapContextCreate(
                    data.as_mut_ptr().cast(),
                    width,
                    height,
                    8,
                    width,
                    None,
                    CGImageAlphaInfo::Only.0,
                )
            }?;
            // 位图的原点在左下角，内存中第一行是最上面一行，与 `Coverage` 的存放顺序相同。
            CGContext::translate_ctm(Some(&context), -left, -bottom);
            CGContext::add_path(Some(&context), Some(&outline));
            if bold > 0.0 {
                CGContext::set_line_width(Some(&context), f64::from(bold));
                CGContext::set_line_join(Some(&context), CGLineJoin::Round);
                CGContext::draw_path(Some(&context), CGPathDrawingMode::FillStroke);
            } else {
                CGContext::fill_path(Some(&context));
            }
            CGContext::flush(Some(&context));
            drop(context);

            Some(Coverage {
                left: left as i32,
                top: top as i32,
                width,
                height,
                data,
            })
        }
    }

    /// 返回指向字体文件 `path` 中 PostScript 名为 `postscript` 的字体（或命名实例）的描述符。
    ///
    /// 描述符同时带文件 URL 和名字，CoreText 用该文件中的字体，不换成其他位置的同名字体；文件中
    /// 没有该名字时，创建出的字体与 swash 打开的字体对不上，由调用方核对字形编号时排除。列出
    /// 文件中的全部字体再按名字挑选也能做到，但苹方的文件中有 268 个，每次需要 2–4 ms（观测值）。
    fn descriptor_in(path: &Path, postscript: &str) -> Option<CFRetained<CTFontDescriptor>> {
        let url = CFURL::from_file_path(path)?;
        let name = CFString::from_str(postscript);
        // SAFETY: 两个键是 CoreText 导出的常量字符串，在进程生命周期内有效。
        let keys = unsafe { [kCTFontURLAttribute, kCTFontNameAttribute] };
        let values: [&CFType; 2] = [url.as_ref(), name.as_ref()];
        let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        // SAFETY: `attributes` 的键是字体描述符的属性名，值的类型与各属性要求的一致（文件 URL、
        // 字符串），调用期间保持存活。
        Some(unsafe { CTFontDescriptor::with_attributes(attributes.as_opaque()) })
    }

    /// 遍历路径时的接收端：把路径元素换算成 zeno 的点和命令。
    struct Sink<'a> {
        path: &'a mut GlyphPath,
        /// 从 CoreText 坐标到输出坐标的缩放系数。
        scale: f32,
    }

    /// `CGPathApply` 的回调：把一个路径元素追加到 `info` 指向的 [`Sink`]。
    ///
    /// # Safety
    ///
    /// `info` 必须指向有效且未被其他引用借用的 `Sink`；`element` 及其 `points` 在回调期间有效，
    /// 点数与元素种类相符（`CGPathApply` 保证后者）。
    unsafe extern "C-unwind" fn element(info: *mut c_void, element: NonNull<CGPathElement>) {
        // SAFETY: `info` 是 `CGPathApply` 传回的指针，指向调用方仍在使用的 `Sink`，回调返回前
        // 不会被释放；`element` 由 `CGPathApply` 保证非空，且在本回调返回前一直有效。
        let (sink, element) = unsafe { (&mut *info.cast::<Sink>(), element.as_ref()) };
        let scale = sink.scale;
        let point = |i: usize| {
            // SAFETY: `i` 小于该种类元素的点数（移动、直线 1 个，二次曲线 2 个，三次曲线 3 个）。
            let p = unsafe { *element.points.as_ptr().add(i) };
            Point::new(p.x as f32 * scale, p.y as f32 * scale)
        };
        let (verb, count) = match element.r#type {
            CGPathElementType::MoveToPoint => (Verb::MoveTo, 1),
            CGPathElementType::AddLineToPoint => (Verb::LineTo, 1),
            CGPathElementType::AddQuadCurveToPoint => (Verb::QuadTo, 2),
            CGPathElementType::AddCurveToPoint => (Verb::CurveTo, 3),
            CGPathElementType::CloseSubpath => (Verb::Close, 0),
            _ => return,
        };
        sink.path.verbs.push(verb);
        sink.path.points.extend((0..count).map(point));
    }

    #[cfg(test)]
    mod tests;
}
