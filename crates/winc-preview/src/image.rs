//! 图片预览：PNG、JPEG、GIF、WebP、BMP、ICO/CUR 解码为 RGBA，按窗口大小缩放显示；SVG 由
//! [`crate::svg`] 按显示大小现场绘制。
//!
//! 解码在后台线程进行，标签先打开。很大的图先缩到长边不超过 `KEEP_MAX`（2560 像素）再保留：
//! 一张 4000×3000 的照片按 RGBA8 原样保留要 48 MB，缩小后约 20 MB。PNG 边解码边缩小；macOS
//! 上要缩小的 JPEG 由 `ImageIO` 按目标大小解码（见 `image/imageio.rs`）。

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use crate::svg::Drawing;

#[cfg(target_os = "macos")]
mod imageio;

/// 解码后保留的图片最大边长（像素）。
///
/// 经验值：长边超过该值的图在解码后缩小，以限制内存占用。
const KEEP_MAX: u32 = 2560;
/// 允许解码的最大像素数（宽 × 高，像素）。
///
/// 超过时不解码：按 RGBA8 计算，解码结果约需 400 MB（1 亿像素 × 4 字节）。
pub(crate) const PIXELS_MAX: u64 = 100_000_000;
/// 允许读取的图片文件大小上限（字节，256 MiB），超过时拒绝读取。
pub(crate) const FILE_MAX: u64 = 256 << 20;
/// 缩放时启用多线程的像素数下限（像素）。
///
/// 源图与目标图像素数的较大者达到该值时，才把缩放分给多个线程；经验值，避免小图承担线程开销。
const PARALLEL_MIN: usize = 256 * 1024;

/// 解码后的位图：RGBA8，不预乘 alpha。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    /// 宽度（像素）。
    pub width: u32,
    /// 高度（像素）。
    pub height: u32,
    /// 像素数据，每 4 字节一个像素，按 R、G、B、A 排列，不预乘。
    pub rgba: Vec<u8>,
}

/// 图片内容：位图像素，或 SVG 图形（按显示大小现场绘制，任意缩放都保持清晰）。
#[derive(Debug, Clone)]
pub enum Source {
    /// 位图，已解码成 RGBA8。
    Raster(Pixels),
    /// SVG 图形，按显示大小临时绘制。
    Vector(Arc<Drawing>),
}

/// 一张用于预览的图片。
#[derive(Debug, Clone)]
pub struct Picture {
    /// 原图宽度（像素），状态栏显示；SVG 取 SVG 声明的大小。
    pub width: u32,
    /// 原图高度（像素）。
    pub height: u32,
    /// 格式名，例如 `PNG`、`JPEG`、`SVG`。
    pub format: &'static str,
    /// 文件大小（字节）。
    pub file_size: u64,
    /// 图片内容：位图或 SVG 图形。
    pub source: Source,
    /// 是否含透明像素；为 `true` 时绘制在棋盘格上。
    pub transparent: bool,
}

impl Picture {
    /// 渲染为 `width` × `height` 像素的位图。
    pub fn render(&self, width: u32, height: u32) -> Pixels {
        match &self.source {
            Source::Raster(pixels) => resize(pixels, width, height),
            Source::Vector(drawing) => drawing.render(width, height),
        }
    }

    /// 返回 SVG 中未能绘制的内容的名称（如文字、滤镜，用于状态栏提示）；位图返回空切片。
    pub fn missing(&self) -> &[&'static str] {
        match &self.source {
            Source::Vector(drawing) => &drawing.missing,
            Source::Raster(_) => &[],
        }
    }
}

/// 是否按扩展名识别为可直接预览的图片。
///
/// SVG 是文本，仍按文本打开，见 [`is_svg`]。
pub fn is_image(path: &Path) -> bool {
    format_of(path).is_some()
}

/// 是否为 SVG 文件（按扩展名判断）；SVG 按文本打开，可切换到预览。
pub fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

fn format_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "apng" => "PNG",
        "jpg" | "jpeg" | "jpe" | "jfif" => "JPEG",
        "gif" => "GIF",
        "webp" => "WebP",
        "bmp" | "dib" => "BMP",
        "ico" => "ICO",
        "cur" => "CUR",
        _ => return None,
    })
}

/// 读取并解码一张图片。
///
/// 长边超过 `KEEP_MAX`（2560 像素）的图会被缩小，但 `width`、`height` 仍是原图的大小。
///
/// # Errors
///
/// 格式不支持、文件无法读取、不是普通文件、超过 `FILE_MAX`（256 MiB）或解码失败时返回原因。
pub fn load(path: &Path) -> Result<Picture, String> {
    let named = format_of(path).ok_or(winc_i18n::t!("preview-image-format-unsupported"))?;
    let metadata = winc_backend::fs::metadata(path).map_err(|e| e.to_string())?;
    // 管道、设备文件即使有图片扩展名也不能读取：读取没有终点，会使后台线程一直阻塞。
    if !metadata.is_file() {
        return Err(winc_i18n::t!("preview-image-not-regular-file").to_owned());
    }
    let file_size = metadata.len;
    if file_size > FILE_MAX {
        return Err(winc_i18n::t!("preview-image-too-large").to_owned());
    }
    let bytes = winc_backend::fs::read(path).map_err(|e| e.to_string())?;
    // 扩展名与内容不符（`.png` 实为 JPEG）时，以内容识别的格式为准。
    let format = sniff(&bytes).unwrap_or(named);
    let (pixels, (width, height)) = decode(format, &bytes)
        .map_err(|e| winc_i18n::t!("preview-image-decode-failed", error = e.to_string()))?;
    // 在缩小后的像素上检查透明度，需要检查的像素少得多。
    let transparent = format != "JPEG" && pixels.rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255);
    Ok(Picture {
        width,
        height,
        format,
        file_size,
        source: Source::Raster(pixels),
        transparent,
    })
}

/// 按内容识别格式并解码内存中的一张图片 `bytes`（docx 等文档包中的图片）。
///
/// 与 [`load`] 相同，长边超过 `KEEP_MAX` 的图会被缩小。
///
/// # Errors
///
/// 无法识别格式或解码失败时返回原因。
pub fn load_bytes(bytes: &[u8]) -> Result<Picture, String> {
    let format = sniff(bytes).ok_or(winc_i18n::t!("preview-image-format-unknown"))?;
    let (pixels, (width, height)) = decode(format, bytes)
        .map_err(|e| winc_i18n::t!("preview-image-decode-failed", error = e.to_string()))?;
    let transparent = format != "JPEG" && pixels.rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255);
    Ok(Picture {
        width,
        height,
        format,
        file_size: bytes.len() as u64,
        source: Source::Raster(pixels),
        transparent,
    })
}

/// 按内容识别格式并解码一张图片，供 SVG 中内嵌的位图使用。
///
/// # Errors
///
/// 无法识别格式或解码失败时返回原因。
pub fn decode_bytes(bytes: &[u8]) -> Result<Pixels, String> {
    let format = sniff(bytes).ok_or(winc_i18n::t!("preview-image-format-unknown"))?;
    decode(format, bytes).map(|(pixels, _)| pixels)
}

/// 解码一张图，长边缩到不超过 [`KEEP_MAX`]；返回缩小后的图和原图的大小。
fn decode(format: &str, bytes: &[u8]) -> Result<(Pixels, (u32, u32)), String> {
    let pixels = match format {
        "PNG" => return decode_png(bytes),
        "JPEG" => return decode_jpeg_fitted(bytes),
        "GIF" => return decode_gif(bytes),
        "WebP" => decode_webp(bytes),
        "BMP" => decode_bmp(bytes),
        _ => decode_ico(bytes),
    }?;
    let size = (pixels.width, pixels.height);
    Ok((shrink(pixels, KEEP_MAX), size))
}

/// 按文件开头的魔数识别格式。
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("PNG")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("JPEG")
    } else if bytes.starts_with(b"GIF8") {
        Some("GIF")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("WebP")
    } else if bytes.starts_with(b"BM") {
        Some("BMP")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("ICO")
    } else if bytes.starts_with(&[0, 0, 2, 0]) {
        Some("CUR")
    } else {
        None
    }
}

/// 检查图片尺寸：宽高都不能为 0，且像素数不超过 [`PIXELS_MAX`]。
fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err(winc_i18n::t!("preview-image-empty").to_owned());
    }
    if u64::from(width) * u64::from(height) > PIXELS_MAX {
        return Err(winc_i18n::t!(
            "preview-image-dimensions-too-large",
            width = width,
            height = height
        ));
    }
    Ok(())
}

/// 解码 PNG：长边缩到不超过 [`KEEP_MAX`]，返回缩小后的图和原图的大小。
///
/// 需要缩小时逐行解码、边解码边缩小，内存只需缩小后的图加一行原图。若先解出整张图再缩小，
/// 一亿像素的 RGB 图需要约 700 MB（解码得到的 RGB 约 300 MB，转换后的 RGBA 约 400 MB）。
/// 隔行扫描（Adam7）的行不按顺序到达：按各遍的位置把每个像素累加进新图对应的一格
/// （[`Scatter`]），内存是新图每格 16 字节，2560 × 2560 时约 100 MB。
fn decode_png(bytes: &[u8]) -> Result<(Pixels, (u32, u32)), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // 调色板展开为 RGB(A)，低位深灰度扩展为 8 位，16 位降为 8 位，tRNS 展开为透明通道。
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    check_size(width, height)?;
    let (color, _) = reader.output_color_type();
    let kept = kept_size(width, height, KEEP_MAX);

    if info.interlaced && kept != (width, height) {
        let mut scatter = Scatter::new(width, height, kept);
        let mut row = Vec::new();
        for (x_step, x_start, y_step, y_start) in ADAM7 {
            let samples = (width as usize).saturating_sub(x_start).div_ceil(x_step);
            let lines = (height as usize).saturating_sub(y_start).div_ceil(y_step);
            if samples == 0 {
                continue;
            }
            for line in 0..lines {
                let data = reader.next_row().map_err(|e| e.to_string())?;
                let data = data.ok_or(winc_i18n::t!("preview-image-truncated"))?;
                row.clear();
                extend_rgba(color, data.data(), &mut row)?;
                if row.len() != samples * 4 {
                    return Err(winc_i18n::t!("preview-image-size-mismatch").to_owned());
                }
                scatter.put(y_start + line * y_step, x_start, x_step, &row);
            }
        }
        return Ok((scatter.finish(), (width, height)));
    }

    if kept == (width, height) {
        let mut buf = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or(winc_i18n::t!("preview-image-too-large"))?
        ];
        let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        buf.truncate(frame.buffer_size());
        let rgba = if color == png::ColorType::Rgba {
            buf
        } else {
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            extend_rgba(color, &buf, &mut rgba)?;
            rgba
        };
        let pixels = Pixels {
            width,
            height,
            rgba,
        };
        return Ok((shrink(pixels, KEEP_MAX), (width, height)));
    }

    let mut shrinker = Shrinker::new(width, height, kept);
    let mut row = Vec::with_capacity(width as usize * 4);
    while let Some(line) = reader.next_row().map_err(|e| e.to_string())? {
        row.clear();
        extend_rgba(color, line.data(), &mut row)?;
        if row.len() != width as usize * 4 {
            return Err(winc_i18n::t!("preview-image-size-mismatch").to_owned());
        }
        shrinker.push(&row);
    }
    Ok((shrinker.finish()?, (width, height)))
}

/// 把一段 8 位像素按 `color` 转换为 RGBA，追加到 `out`；WebP 解出的 RGB 也经这里转换。
fn extend_rgba(color: png::ColorType, data: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
    match color {
        png::ColorType::Rgba => out.extend_from_slice(data),
        png::ColorType::Rgb => {
            for p in data.as_chunks::<3>().0 {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in data.as_chunks::<2>().0 {
                out.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for &g in data {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::Indexed => {
            return Err(winc_i18n::t!("preview-image-palette-unexpanded").to_owned());
        }
    }
    Ok(())
}

/// 逐行接收原图并边接收边缩小，结果与 [`resize`] 缩小时相同：每个新像素是它覆盖的原像素
/// 按不透明度加权的平均，区别是不需要先有整张原图。
struct Shrinker {
    /// 原图的高度（像素）。
    height: usize,
    /// 缩小后的结果，逐行追加。
    out: Pixels,
    /// 新图每一列覆盖的原图列的范围，见 [`spans`]。
    columns: Vec<(usize, usize)>,
    /// 正在累加的一行新像素：每列的（R、G、B 各乘不透明度之和，不透明度之和）。
    sums: Vec<[u64; 4]>,
    /// 已收到的原图行数。
    row: usize,
    /// 正在累加的新图行号（从 0 起）。
    target: usize,
}

impl Shrinker {
    /// 创建缩小到 `kept`（宽、高，像素）的收集器；`kept` 的宽、高都不能大于原图。
    fn new(width: u32, height: u32, kept: (u32, u32)) -> Shrinker {
        let (width, height) = (width as usize, height as usize);
        let (w, h) = (kept.0 as usize, kept.1 as usize);
        Shrinker {
            height,
            out: Pixels {
                width: kept.0,
                height: kept.1,
                rgba: Vec::with_capacity(w * h * 4),
            },
            columns: spans(w, width),
            sums: vec![[0; 4]; w],
            row: 0,
            target: 0,
        }
    }

    /// 接收下一行原图（RGBA）。
    fn push(&mut self, line: &[u8]) {
        let h = self.out.height as usize;
        if self.target >= h {
            return;
        }
        for (sum, &(x0, x1)) in self.sums.iter_mut().zip(&self.columns) {
            accumulate(sum, &line[x0 * 4..x1 * 4]);
        }
        self.row += 1;

        // 新图当前行覆盖的原图行已收齐：输出该行，并开始累加下一行。
        let (y0, y1) = span(self.target, h, self.height);
        if self.row == y1 {
            for (sum, &(x0, x1)) in self.sums.iter_mut().zip(&self.columns) {
                let n = ((y1 - y0) * (x1 - x0)) as u64;
                self.out.rgba.extend_from_slice(&average(*sum, n));
                *sum = [0; 4];
            }
            self.target += 1;
        }
    }

    /// 返回缩小后的图；收到的原图行数不足（文件不完整）时返回错误。
    fn finish(self) -> Result<Pixels, String> {
        if self.target < self.out.height as usize {
            return Err(winc_i18n::t!("preview-image-truncated").to_owned());
        }
        Ok(self.out)
    }
}

/// Adam7 隔行扫描的七遍：（横向间隔，横向起点，纵向间隔，纵向起点），见 PNG 规范第 8.2 节。
const ADAM7: [(usize, usize, usize, usize); 7] = [
    (8, 0, 8, 0),
    (8, 4, 8, 0),
    (4, 0, 8, 4),
    (4, 2, 4, 0),
    (2, 0, 4, 2),
    (2, 1, 2, 0),
    (1, 0, 2, 1),
];

/// 隔行扫描的 PNG 边解码边缩小：像素不按行序到达，按坐标累加进新图的一格。
///
/// 分格与 [`Shrinker`] 相同（[`spans`]），结果也相同。每格的和用 `u32`：一格最多
/// `PIXELS_MAX / KEEP_MAX` 个原像素（约 3.9 万），每个乘不透明度后不超过 65025，和不超过 26 亿。
struct Scatter {
    /// 缩小后的宽、高（像素）。
    kept: (usize, usize),
    /// 原图每一列、每一行落在新图的哪一列、哪一行。
    col_of: Vec<u32>,
    row_of: Vec<u32>,
    /// 原图的宽、高（像素）。
    size: (usize, usize),
    /// 新图每格的（R、G、B 各乘不透明度之和，不透明度之和），按行排列。
    sums: Vec<[u32; 4]>,
}

impl Scatter {
    /// 创建缩小到 `kept`（宽、高，像素）的收集器；`kept` 的宽、高都不能大于原图。
    fn new(width: u32, height: u32, kept: (u32, u32)) -> Scatter {
        let (width, height) = (width as usize, height as usize);
        let kept = (kept.0 as usize, kept.1 as usize);
        let owner = |n: usize, total: usize| {
            let mut of = vec![0; total];
            for (i, (start, end)) in spans(n, total).into_iter().enumerate() {
                of[start..end].fill(i as u32);
            }
            of
        };
        Scatter {
            kept,
            col_of: owner(kept.0, width),
            row_of: owner(kept.1, height),
            size: (width, height),
            sums: vec![[0; 4]; kept.0 * kept.1],
        }
    }

    /// 累加一行隔行扫描的像素（RGBA）：原图第 `y` 行，第 `i` 个像素在原图的第
    /// `x_start + i * x_step` 列。
    fn put(&mut self, y: usize, x_start: usize, x_step: usize, pixels: &[u8]) {
        let base = self.row_of[y] as usize * self.kept.0;
        for (i, p) in pixels.as_chunks::<4>().0.iter().enumerate() {
            let x = self.col_of[x_start + i * x_step] as usize;
            let sum = &mut self.sums[base + x];
            let a = u32::from(p[3]);
            sum[0] += u32::from(p[0]) * a;
            sum[1] += u32::from(p[1]) * a;
            sum[2] += u32::from(p[2]) * a;
            sum[3] += a;
        }
    }

    /// 返回缩小后的图。
    fn finish(self) -> Pixels {
        let (w, h) = self.kept;
        let (columns, rows) = (spans(w, self.size.0), spans(h, self.size.1));
        let mut rgba = Vec::with_capacity(w * h * 4);
        for (y, &(y0, y1)) in rows.iter().enumerate() {
            for (x, &(x0, x1)) in columns.iter().enumerate() {
                let sum = self.sums[y * w + x].map(u64::from);
                rgba.extend_from_slice(&average(sum, ((y1 - y0) * (x1 - x0)) as u64));
            }
        }
        Pixels {
            width: w as u32,
            height: h as u32,
            rgba,
        }
    }
}

/// 解码 JPEG：长边缩到不超过 [`KEEP_MAX`]，返回缩小后的图和原图的大小。
///
/// macOS 上要缩小的交给 `ImageIO` 按目标大小解码（见 [`imageio`]），不先解出整张原图；`ImageIO`
/// 解不出来时改用 zune-jpeg 解出整张再缩小。
fn decode_jpeg_fitted(bytes: &[u8]) -> Result<(Pixels, (u32, u32)), String> {
    #[cfg(target_os = "macos")]
    {
        let (width, height) = jpeg_size(bytes)?;
        if width.max(height) > KEEP_MAX
            && let Some(pixels) = imageio::decode_scaled(bytes, KEEP_MAX)
        {
            return Ok((pixels, (width, height)));
        }
    }
    let pixels = decode_jpeg(bytes)?;
    let size = (pixels.width, pixels.height);
    Ok((shrink(pixels, KEEP_MAX), size))
}

/// 只读 JPEG 的头，返回图片大小；大小不合要求（见 [`check_size`]）时返回原因。
#[cfg(target_os = "macos")]
fn jpeg_size(bytes: &[u8]) -> Result<(u32, u32), String> {
    use zune_jpeg::zune_core::bytestream::ZCursor;

    let mut decoder = zune_jpeg::JpegDecoder::new(ZCursor::new(bytes));
    decoder.decode_headers().map_err(|e| e.to_string())?;
    let info = decoder
        .info()
        .ok_or(winc_i18n::t!("preview-image-size-unreadable"))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    check_size(width, height)?;
    Ok((width, height))
}

fn decode_jpeg(bytes: &[u8]) -> Result<Pixels, String> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| e.to_string())?;
    let info = decoder
        .info()
        .ok_or(winc_i18n::t!("preview-image-size-unreadable"))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    check_size(width, height)?;
    let rgba = decoder.decode().map_err(|e| e.to_string())?;
    if rgba.len() != (width * height * 4) as usize {
        return Err(winc_i18n::t!("preview-image-size-mismatch").to_owned());
    }
    Ok(Pixels {
        width,
        height,
        rgba,
    })
}

/// 解码 GIF：只取第一帧，画在整张画布上（第一帧可以比画布小）；长边缩到不超过 [`KEEP_MAX`]，
/// 返回缩小后的图和画布的大小。
///
/// 画布大小只取自文件头，数十字节的文件就能声明一亿像素的画布。需要缩小时逐行生成画布、
/// 边生成边缩小（同 PNG），内存只需缩小后的图加一行画布。
fn decode_gif(bytes: &[u8]) -> Result<(Pixels, (u32, u32)), String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|e| e.to_string())?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    check_size(width, height)?;
    let frame = decoder
        .read_next_frame()
        .map_err(|e| e.to_string())?
        .ok_or(winc_i18n::t!("preview-image-no-frames"))?;
    let (fw, fh) = (u32::from(frame.width), u32::from(frame.height));
    let (left, top) = (u32::from(frame.left), u32::from(frame.top));
    // 帧的位置可以在画布之外（解码器不检查）：整帧在画布右侧之外时目标位置已超出画布，切片会
    // panic，因此先按画布裁剪。
    let cols = fw.min(width.saturating_sub(left)) as usize;
    let rows = top..top.saturating_add(fh.min(height.saturating_sub(top)));
    // 画布第 `y` 行中帧覆盖的部分：（帧中该行的像素，在画布行中的字节偏移）。
    let frame_row = |y: u32| {
        let src = ((y - top) * fw) as usize * 4;
        let row = frame.buffer.get(src..src + cols * 4)?;
        Some((row, left as usize * 4))
    };

    let kept = kept_size(width, height, KEEP_MAX);
    if kept == (width, height) {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for y in rows.clone().filter(|_| cols > 0) {
            let Some((row, at)) = frame_row(y) else {
                break;
            };
            let dst = (y * width) as usize * 4 + at;
            rgba[dst..dst + row.len()].copy_from_slice(row);
        }
        let pixels = Pixels {
            width,
            height,
            rgba,
        };
        return Ok((pixels, (width, height)));
    }

    let mut shrinker = Shrinker::new(width, height, kept);
    let mut line = vec![0u8; width as usize * 4];
    for y in 0..height {
        line.fill(0);
        if cols > 0
            && rows.contains(&y)
            && let Some((row, at)) = frame_row(y)
        {
            line[at..at + row.len()].copy_from_slice(row);
        }
        shrinker.push(&line);
    }
    Ok((shrinker.finish()?, (width, height)))
}

/// 解码 WebP（有损、无损、带透明）；动图只取第一帧。
fn decode_webp(bytes: &[u8]) -> Result<Pixels, String> {
    let mut decoder =
        image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    check_size(width, height)?;
    let mut buf = vec![
        0;
        decoder
            .output_buffer_size()
            .ok_or(winc_i18n::t!("preview-image-too-large"))?
    ];
    decoder.read_image(&mut buf).map_err(|e| e.to_string())?;
    let rgba = if decoder.has_alpha() {
        buf
    } else {
        let mut rgba = Vec::with_capacity(buf.len() / 3 * 4);
        extend_rgba(png::ColorType::Rgb, &buf, &mut rgba)?;
        rgba
    };
    Ok(Pixels {
        width,
        height,
        rgba,
    })
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    let b = bytes
        .get(at..at + 2)
        .ok_or(winc_i18n::t!("preview-image-truncated"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    let b = bytes
        .get(at..at + 4)
        .ok_or(winc_i18n::t!("preview-image-truncated"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn decode_bmp(bytes: &[u8]) -> Result<Pixels, String> {
    let offset = u32_at(bytes, 10)? as usize;
    decode_dib(
        bytes
            .get(14..)
            .ok_or(winc_i18n::t!("preview-image-truncated"))?,
        Some(offset.saturating_sub(14)),
        false,
    )
}

/// 解码 Windows 的 DIB 位图（BMP 去掉文件头的部分，ICO 中的位图也是这种格式）：1、4、8 位为调色板，
/// 16、24、32 位为直接颜色。
///
/// `pixels_at` 是像素数据相对于 `dib` 开头的偏移：BMP 在文件头中指明，ICO 中紧跟在调色板之后
/// （传 `None`）。`icon` 为 `true` 时，信息头中的高度是实际高度的两倍，像素之后还有一张
/// 1 位的透明遮罩。
fn decode_dib(dib: &[u8], pixels_at: Option<usize>, icon: bool) -> Result<Pixels, String> {
    let header = u32_at(dib, 0)? as usize;
    let (width, raw_height, bits, compression, colors_used) = if header == 12 {
        // OS/2 的旧版信息头（12 字节）。
        let (w, h) = (u16_at(dib, 4)?, u16_at(dib, 6)?);
        (i32::from(w), i32::from(h), u16_at(dib, 10)?, 0, 0)
    } else if header >= 40 {
        (
            u32_at(dib, 4)? as i32,
            u32_at(dib, 8)? as i32,
            u16_at(dib, 14)?,
            u32_at(dib, 16)?,
            u32_at(dib, 32)? as usize,
        )
    } else {
        return Err(winc_i18n::t!("preview-image-bitmap-header-unknown").to_owned());
    };
    // 高度为负：像素从上往下存储。
    let top_down = raw_height < 0;
    let mut height = raw_height.unsigned_abs();
    if icon {
        height /= 2;
    }
    let width = u32::try_from(width).map_err(|_| winc_i18n::t!("preview-image-width-invalid"))?;
    check_size(width, height)?;
    // 压缩方式 0 为不压缩，3（BI_BITFIELDS）和 6（BI_ALPHABITFIELDS）用掩码取颜色；1、
    // 2 的游程压缩（RLE）不支持。
    if !matches!(compression, 0 | 3 | 6) {
        return Err(winc_i18n::t!("preview-image-bmp-compressed").to_owned());
    }

    // 颜色掩码：位于 V4 及以上的信息头中，或紧跟在 40 字节的信息头之后。
    let masks = if compression == 3 || compression == 6 {
        let n = if compression == 6 || header >= 56 {
            4
        } else {
            3
        };
        let mut m = [0u32; 4];
        for (i, mask) in m.iter_mut().enumerate().take(n) {
            *mask = u32_at(dib, 40 + i * 4)?;
        }
        Some(m)
    } else if bits == 16 {
        Some([0x7C00, 0x03E0, 0x001F, 0])
    } else if bits == 32 && header >= 56 {
        // V4/V5 信息头、不压缩的 32 位：存在透明掩码时使用。
        let alpha = u32_at(dib, 52)?;
        Some([0x00FF_0000, 0x0000_FF00, 0x0000_00FF, alpha])
    } else {
        None
    };
    let mask_bytes = if header == 40 && compression == 3 {
        12
    } else if header == 40 && compression == 6 {
        16
    } else {
        0
    };

    let palette_entries = match bits {
        1 | 4 | 8 if colors_used == 0 => 1 << bits,
        1 | 4 | 8 => colors_used.min(1 << bits),
        _ => 0,
    };
    let entry = if header == 12 { 3 } else { 4 };
    let palette_at = header + mask_bytes;
    let palette: Vec<[u8; 4]> = (0..palette_entries)
        .map(|i| {
            let at = palette_at + i * entry;
            dib.get(at..at + 3)
                .map(|c| [c[2], c[1], c[0], 255])
                .ok_or(winc_i18n::t!("preview-image-palette-incomplete").to_owned())
        })
        .collect::<Result<_, _>>()?;
    let start = pixels_at.unwrap_or(palette_at + palette_entries * entry);
    let stride = (width as usize * usize::from(bits)).div_ceil(32) * 4;
    let (w, h) = (width as usize, height as usize);
    if dib.len() < start + stride * h {
        return Err(winc_i18n::t!("preview-image-truncated").to_owned());
    }

    // 掩码来自文件，最宽可达 32 位：`shifted * 255` 用 `u32` 计算会溢出，因此用 `u64`。
    let channel = |value: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 255;
        }
        let shifted = u64::from((value & mask) >> mask.trailing_zeros());
        let max = u64::from(mask >> mask.trailing_zeros());
        ((shifted * 255 + max / 2) / max) as u8
    };

    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        let src = &dib[start + row * stride..start + (row + 1) * stride];
        let y = if top_down { row } else { h - 1 - row };
        let out = &mut rgba[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let px: [u8; 4] = match bits {
                1 | 4 | 8 => {
                    let bit = x * usize::from(bits);
                    let byte = src[bit / 8];
                    let shift = 8 - usize::from(bits) - bit % 8;
                    let index = (byte >> shift) & ((1u16 << bits) - 1) as u8;
                    *palette.get(usize::from(index)).unwrap_or(&[0, 0, 0, 255])
                }
                16 => {
                    let v = u32::from(u16::from_le_bytes([src[x * 2], src[x * 2 + 1]]));
                    let m = masks.unwrap_or_default();
                    [
                        channel(v, m[0]),
                        channel(v, m[1]),
                        channel(v, m[2]),
                        channel(v, m[3]),
                    ]
                }
                24 => [src[x * 3 + 2], src[x * 3 + 1], src[x * 3], 255],
                32 => {
                    let v = u32::from_le_bytes([
                        src[x * 4],
                        src[x * 4 + 1],
                        src[x * 4 + 2],
                        src[x * 4 + 3],
                    ]);
                    match masks {
                        Some(m) => [
                            channel(v, m[0]),
                            channel(v, m[1]),
                            channel(v, m[2]),
                            channel(v, m[3]),
                        ],
                        // 不压缩的 32 位：第四个字节通常未使用（全为 0），图标中则是透明度。
                        None => [src[x * 4 + 2], src[x * 4 + 1], src[x * 4], src[x * 4 + 3]],
                    }
                }
                _ => {
                    return Err(winc_i18n::t!(
                        "preview-image-bmp-depth-unsupported",
                        bits = bits
                    ));
                }
            };
            out[x * 4..x * 4 + 4].copy_from_slice(&px);
        }
    }

    let mut alpha_used = bits == 32 && masks.is_none_or(|m| m[3] != 0);
    if bits == 32 && masks.is_none() && rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
        // 第四个字节全为 0：说明该字节未用作透明度。图标此时仍使用后面的遮罩。
        rgba.as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p[3] = 255);
        alpha_used = false;
    }
    if icon && !alpha_used {
        // 图标没有透明度时使用后面的 1 位遮罩：位为 1 表示透明。
        let mask_stride = w.div_ceil(32) * 4;
        let mask_at = start + stride * h;
        if dib.len() >= mask_at + mask_stride * h {
            for row in 0..h {
                let y = if top_down { row } else { h - 1 - row };
                for x in 0..w {
                    let byte = dib[mask_at + row * mask_stride + x / 8];
                    if byte & (0x80 >> (x % 8)) != 0 {
                        rgba[(y * w + x) * 4 + 3] = 0;
                    }
                }
            }
        }
    }
    Ok(Pixels {
        width,
        height,
        rgba,
    })
}

/// 解码 ICO、CUR：文件内含多种尺寸，取面积最大的一张（面积相同时取颜色位数多的）；
/// 每张是 PNG 或 DIB 位图。
fn decode_ico(bytes: &[u8]) -> Result<Pixels, String> {
    let count = usize::from(u16_at(bytes, 4)?);
    if count == 0 {
        return Err(winc_i18n::t!("preview-image-icon-empty").to_owned());
    }
    let mut best: Option<(u64, u16, usize, usize)> = None;
    for i in 0..count {
        let at = 6 + i * 16;
        let entry = bytes
            .get(at..at + 16)
            .ok_or(winc_i18n::t!("preview-image-truncated"))?;
        let side = |b: u8| if b == 0 { 256 } else { u64::from(b) };
        let area = side(entry[0]) * side(entry[1]);
        let bits = u16_at(entry, 6)?;
        let (size, offset) = (u32_at(entry, 8)? as usize, u32_at(entry, 12)? as usize);
        if best.is_none_or(|(a, b, _, _)| (area, bits) > (a, b)) {
            best = Some((area, bits, offset, size));
        }
    }
    let (_, _, offset, size) = best.ok_or(winc_i18n::t!("preview-image-icon-empty"))?;
    let data = bytes
        .get(offset..offset.saturating_add(size).min(bytes.len()))
        .ok_or(winc_i18n::t!("preview-image-truncated"))?;
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(data).map(|(pixels, _)| pixels)
    } else {
        decode_dib(data, None, true)
    }
}

/// 把长边超过 `max` 的图按比例缩小；不超过时原样返回。
fn shrink(pixels: Pixels, max: u32) -> Pixels {
    let size = kept_size(pixels.width, pixels.height, max);
    if size == (pixels.width, pixels.height) {
        return pixels;
    }
    resize(&pixels, size.0, size.1)
}

/// 返回长边缩到不超过 `max` 之后的（宽，高）；每边至少为 1 像素。
fn kept_size(width: u32, height: u32, max: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max {
        return (width, height);
    }
    let scale = |n: u32| ((u64::from(n) * u64::from(max)) / u64::from(long)).max(1) as u32;
    (scale(width), scale(height))
}

/// 缩放到 `width` × `height` 像素；`width`、`height` 为 0 时按 1 处理。
///
/// 缩小时，每个新像素取它覆盖的原像素的平均，按不透明度加权，因此透明边缘不会出现黑边；
/// 放大时取最近的原像素，小图标放大后仍是清晰的方块。像素数较多时（达到 `PARALLEL_MIN`）按行分段，
/// 用多个线程并行处理（最多 8 个）。
pub fn resize(src: &Pixels, width: u32, height: u32) -> Pixels {
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let row_bytes = w * 4;
    let work = (src.width as usize * src.height as usize).max(w * h);
    let threads = if work < PARALLEL_MIN {
        1
    } else {
        thread::available_parallelism().map_or(1, |n| n.get().min(8))
    };
    let rows_each = h.div_ceil(threads);
    thread::scope(|scope| {
        for (i, chunk) in rgba.chunks_mut(rows_each * row_bytes).enumerate() {
            let first = i * rows_each;
            let mut job = move || resize_rows(src, w, h, first, chunk);
            if threads == 1 {
                job();
            } else {
                scope.spawn(job);
            }
        }
    });
    Pixels {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

/// 计算 [`resize`] 结果中从第 `first` 行（从 0 起）开始的若干行，填满 `out`。
fn resize_rows(src: &Pixels, w: usize, h: usize, first: usize, out: &mut [u8]) {
    let (sw, sh) = (src.width as usize, src.height as usize);
    if w >= sw && h >= sh {
        for (row, line) in out.chunks_exact_mut(w * 4).enumerate() {
            let sy = (first + row) * sh / h;
            for (x, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let s = (sy * sw + x * sw / w) * 4;
                px.copy_from_slice(&src.rgba[s..s + 4]);
            }
        }
        return;
    }
    let columns = spans(w, sw);
    for (row, line) in out.chunks_exact_mut(w * 4).enumerate() {
        let (y0, y1) = span(first + row, h, sh);
        for (px, &(x0, x1)) in line.as_chunks_mut::<4>().0.iter_mut().zip(&columns) {
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                let start = (sy * sw + x0) * 4;
                accumulate(&mut sum, &src.rgba[start..start + (x1 - x0) * 4]);
            }
            px.copy_from_slice(&average(sum, ((y1 - y0) * (x1 - x0)) as u64));
        }
    }
}

/// 把 `total` 个原像素均分给 `n` 个新像素，返回第 `i` 个新像素覆盖的原像素范围（左闭右开）。
///
/// 每个范围至少有一个原像素：放大的那一边（`n` 大于 `total`）相邻的新像素可以覆盖同一个原像素。
fn span(i: usize, n: usize, total: usize) -> (usize, usize) {
    let start = i * total / n;
    (start, ((i + 1) * total / n).max(start + 1).min(total))
}

/// 返回全部 `n` 个新像素的 [`span`]。
fn spans(n: usize, total: usize) -> Vec<(usize, usize)> {
    (0..n).map(|i| span(i, n, total)).collect()
}

/// 把一段 RGBA 像素累加到 `sum`：R、G、B 各乘不透明度后相加，不透明度直接相加。
fn accumulate(sum: &mut [u64; 4], pixels: &[u8]) {
    for p in pixels.as_chunks::<4>().0 {
        let a = u64::from(p[3]);
        sum[0] += u64::from(p[0]) * a;
        sum[1] += u64::from(p[1]) * a;
        sum[2] += u64::from(p[2]) * a;
        sum[3] += a;
    }
}

/// 由 [`accumulate`] 对 `n` 个原像素累加的结果，返回按不透明度加权的平均像素。
fn average([r, g, b, a]: [u64; 4], n: u64) -> [u8; 4] {
    let channel = |total: u64| total.checked_div(a).unwrap_or(0) as u8;
    [channel(r), channel(g), channel(b), (a / n) as u8]
}

/// 把文件大小格式化为「245 KB」「1.4 MB」这样的文字（1 KB = 1024 字节）。
pub fn size_label(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1 << 20 => format!("{} KB", (b + 512) >> 10),
        b => format!("{:.1} MB", b as f64 / f64::from(1 << 20)),
    }
}

#[cfg(test)]
mod tests;
