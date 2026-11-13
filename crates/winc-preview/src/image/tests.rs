//! 图片预览的解码与缩放测试：各格式的解码结果、逐行缩小与整张缩小的等价性，以及异常文件
//! 头不会 panic。

use std::fs;

use super::*;

/// 把 `bytes` 写入临时目录并用 [`load`] 读取，读取后删除临时文件。
fn decoded(name: &str, bytes: &[u8]) -> Result<Picture, String> {
    let dir =
        std::env::temp_dir().join(format!("winc-preview-image-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    let result = load(&path);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    out
}

fn raster(picture: &Picture) -> &[u8] {
    match &picture.source {
        Source::Raster(p) => &p.rgba,
        Source::Vector(_) => panic!("不是像素"),
    }
}

#[test]
fn png_jpeg_gif_and_odd_names_decode_to_rgba() {
    // 2×1 的 RGB PNG：没有透明。
    let picture = decoded(
        "a.png",
        &png(2, 1, png::ColorType::Rgb, &[255, 0, 0, 0, 0, 255]),
    )
    .unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.format),
        (2, 1, "PNG")
    );
    assert_eq!(raster(&picture), [255, 0, 0, 255, 0, 0, 255, 255]);
    assert!(!picture.transparent);

    // 灰度加透明。
    let picture = decoded(
        "b.png",
        &png(1, 1, png::ColorType::GrayscaleAlpha, &[100, 50]),
    )
    .unwrap();
    assert_eq!(raster(&picture), [100, 100, 100, 50]);
    assert!(picture.transparent);

    // 扩展名是 `.jpg` 而内容是 PNG：以内容识别的格式为准。
    let picture = decoded("c.jpg", &png(1, 1, png::ColorType::Rgb, &[1, 2, 3])).unwrap();
    assert_eq!(picture.format, "PNG");

    // GIF：第一帧比画布小，画在帧自己的位置上。
    let mut gif = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut gif, 3, 2, &[0, 0, 0, 255, 255, 255]).unwrap();
        let mut frame = gif::Frame::default();
        (frame.left, frame.top, frame.width, frame.height) = (1, 1, 2, 1);
        frame.buffer = std::borrow::Cow::Owned(vec![1, 1]);
        encoder.write_frame(&frame).unwrap();
    }
    let picture = decoded("d.gif", &gif).unwrap();
    assert_eq!((picture.width, picture.height), (3, 2));
    assert_eq!(&raster(&picture)[16..20], [255, 255, 255, 255]);

    // JPEG：8×8 的红色图（由 macOS 的 ImageIO 写出；有损压缩，颜色与纯红略有偏差）。
    let jpeg = include_bytes!("../../testdata/red-8x8.jpg");
    let picture = decoded("f.jpg", jpeg).unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.format),
        (8, 8, "JPEG")
    );
    let [r, g, b, a] = raster(&picture)[..4] else {
        unreachable!()
    };
    assert!(r > 180 && g < 70 && b < 70 && a == 255, "{r} {g} {b} {a}");

    // 损坏的文件：返回错误原因。
    let broken = decoded("e.png", b"\x89PNG\r\n\x1a\nbroken").unwrap_err();
    assert!(broken.contains("无法解码"));
    assert!(!is_image(Path::new("x.svg")) && is_image(Path::new("X.JPEG")));
    assert!(is_svg(Path::new("a/X.SVG")));
}

/// 构造最小的 BMP：文件头、40 字节的信息头、调色板和像素。
fn bmp(width: i32, height: i32, bits: u16, palette: &[[u8; 4]], pixels: &[u8]) -> Vec<u8> {
    let offset = 14 + 40 + palette.len() * 4;
    let mut out = b"BM".to_vec();
    out.extend_from_slice(&((offset + pixels.len()) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(&dib_header(width, height, bits, palette.len()));
    for c in palette {
        out.extend_from_slice(c);
    }
    out.extend_from_slice(pixels);
    out
}

fn dib_header(width: i32, height: i32, bits: u16, colors: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    // 压缩方式、图像大小、横竖分辨率。
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&(colors as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out
}

#[test]
fn bmp_ico_and_webp_decode() {
    // 24 位、从下往上存：文件中的第一行是图的最后一行。每行补到 4 字节。像素按 BGR 排列：
    // 文件中的第 1 行是红、绿（图的最下行），第 2 行是蓝、白（图的最上行）。
    let pixels = [0, 0, 255, 0, 255, 0, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0];
    let picture = decoded("a.bmp", &bmp(2, 2, 24, &[], &pixels)).unwrap();
    assert_eq!(picture.format, "BMP");
    assert_eq!(
        raster(&picture),
        [
            0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255
        ]
    );
    // 8 位调色板、从上往下存（高度是负的）。
    let palette = [[0, 0, 0, 0], [0, 0, 200, 0]];
    let picture = decoded("b.bmp", &bmp(2, -1, 8, &palette, &[1, 0, 0, 0])).unwrap();
    assert_eq!(raster(&picture), [200, 0, 0, 255, 0, 0, 0, 255]);

    // ICO：一张 1×1 的 32 位位图（半透明）和一张 2×2 的 PNG，取面积大的。
    let small = {
        let mut d = dib_header(1, 2, 32, 0);
        d.extend_from_slice(&[10, 20, 30, 128]);
        d.extend_from_slice(&[0; 4]); // 遮罩
        d
    };
    let large = png(2, 2, png::ColorType::Rgb, &[9; 12]);
    let mut ico = vec![0, 0, 1, 0, 2, 0];
    let first = 6 + 32;
    for (side, data, at) in [(1u8, &small, first), (2, &large, first + small.len())] {
        ico.extend_from_slice(&[side, side, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(at as u32).to_le_bytes());
    }
    ico.extend_from_slice(&small);
    ico.extend_from_slice(&large);
    let picture = decoded("a.ico", &ico).unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.format),
        (2, 2, "ICO")
    );
    // 只有小的那张时，透明度被采用。
    let only_small = {
        let mut i = vec![0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 1, 0, 32, 0];
        i.extend_from_slice(&(small.len() as u32).to_le_bytes());
        i.extend_from_slice(&22u32.to_le_bytes());
        i.extend_from_slice(&small);
        i
    };
    let picture = decoded("b.ico", &only_small).unwrap();
    assert_eq!(raster(&picture), [30, 20, 10, 128]);
    assert!(picture.transparent);

    // WebP：16×16 的有损图，带透明（用 `cwebp` 压缩的蓝色圆）。四角透明，中央为蓝色。
    let webp = include_bytes!("../../testdata/icon-16.webp");
    let picture = decoded("a.webp", webp).unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.format),
        (16, 16, "WebP")
    );
    let rgba = raster(&picture);
    assert_eq!(rgba[3], 0);
    let center = &rgba[(4 * 16 + 4) * 4..(4 * 16 + 4) * 4 + 4];
    assert!(
        center[2] > 200 && center[0] < 120 && center[3] == 255,
        "{center:?}"
    );
    assert!(picture.transparent);
}

#[test]
fn malformed_headers_do_not_panic() {
    // GIF 的第一帧整个在画布右侧之外：什么都不画，且不能越界切片。
    let mut gif = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut gif, 3, 2, &[0, 0, 0, 255, 255, 255]).unwrap();
        let mut frame = gif::Frame::default();
        (frame.left, frame.top, frame.width, frame.height) = (10, 0, 2, 1);
        frame.buffer = std::borrow::Cow::Owned(vec![1, 1]);
        encoder.write_frame(&frame).unwrap();
    }
    let picture = decoded("a.gif", &gif).unwrap();
    assert!(raster(&picture).iter().all(|&b| b == 0));

    // 文件头声明 10000×10000 的画布，只有一个像素的帧：
    // 按缩小后的大小生成，不先分配整张画布（约 400 MB）。
    let mut gif = Vec::new();
    {
        let palette = [0, 0, 0, 255, 255, 255];
        let mut encoder = gif::Encoder::new(&mut gif, 10_000, 10_000, &palette).unwrap();
        let mut frame = gif::Frame::default();
        (frame.left, frame.top, frame.width, frame.height) = (0, 0, 1, 1);
        frame.buffer = std::borrow::Cow::Owned(vec![1]);
        encoder.write_frame(&frame).unwrap();
    }
    assert!(gif.len() < 64);
    let picture = decoded("huge.gif", &gif).unwrap();
    assert_eq!((picture.width, picture.height), (10_000, 10_000));
    assert_eq!(raster(&picture).len(), 2560 * 2560 * 4);

    // 32 位、按掩码取色，掩码宽达 32 位：换算时不能溢出。
    let mut bmp = bmp(1, 1, 32, &[], &[0xFF; 4]);
    bmp[14 + 16..14 + 20].copy_from_slice(&3u32.to_le_bytes());
    let masks: Vec<u8> = [u32::MAX, 0, 0]
        .iter()
        .flat_map(|m| m.to_le_bytes())
        .collect();
    bmp.splice(14 + 40..14 + 40, masks);
    bmp[10..14].copy_from_slice(&(14u32 + 40 + 12).to_le_bytes());
    let picture = decoded("c.bmp", &bmp).unwrap();
    assert_eq!(raster(&picture), [255, 255, 255, 255]);

    // 32 位图标的第四个字节全为 0：未使用透明度，改用后面的遮罩（位为 1 表示透明）。
    let icon = {
        let mut d = dib_header(1, 2, 32, 0);
        d.extend_from_slice(&[10, 20, 30, 0]);
        d.extend_from_slice(&[0x80, 0, 0, 0]);
        d
    };
    let mut ico = vec![0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 1, 0, 32, 0];
    ico.extend_from_slice(&(icon.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(&icon);
    let picture = decoded("c.ico", &ico).unwrap();
    assert_eq!(raster(&picture), [30, 20, 10, 0]);
}

#[test]
fn big_pngs_shrink_row_by_row_like_the_whole_picture() {
    // 长边超过 `KEEP_MAX` 且宽高都不能整除：逐行缩小的结果必须与整张解码后再缩小的结果相同。
    let (width, height) = (KEEP_MAX * 2 + 7, 13);
    let mut seed = 1u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 16) as u8
    };
    let rgba: Vec<u8> = (0..width * height * 4).map(|_| next()).collect();
    let whole = Pixels {
        width,
        height,
        rgba: rgba.clone(),
    };
    let gray: Vec<u8> = rgba.iter().step_by(4).copied().collect();
    let gray_whole = Pixels {
        width,
        height,
        rgba: gray.iter().flat_map(|&g| [g, g, g, 255]).collect(),
    };
    for (color, data, expected) in [
        (png::ColorType::Rgba, &rgba, &whole),
        (png::ColorType::Grayscale, &gray, &gray_whole),
    ] {
        let (pixels, size) = decode_png(&png(width, height, color, data)).unwrap();
        assert_eq!(size, (width, height));
        assert_eq!((pixels.width, pixels.height), (KEEP_MAX, 6));
        assert!(pixels == shrink(expected.clone(), KEEP_MAX), "{color:?}");
    }

    // 高图同样如此；数据不足时返回错误，不 panic。
    let tall = Pixels {
        width: 3,
        height: KEEP_MAX + 1,
        rgba: (0..3 * (KEEP_MAX + 1) * 4).map(|_| next()).collect(),
    };
    let bytes = png(tall.width, tall.height, png::ColorType::Rgba, &tall.rgba);
    let (pixels, _) = decode_png(&bytes).unwrap();
    assert!(pixels == shrink(tall.clone(), KEEP_MAX));
    assert!(decode_png(&bytes[..bytes.len() / 2]).is_err());
}

#[test]
fn shrinking_averages_and_keeps_transparent_edges_clean() {
    // 2×1：一个不透明的红、一个全透明的黑。缩成 1×1：颜色是红（全透明的像素不参与颜色平均），
    // 不透明度为一半。
    let pixels = Pixels {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 0, 0, 0],
    };
    assert_eq!(resize(&pixels, 1, 1).rgba, [255, 0, 0, 127]);
    // 放大取最近的像素。
    assert_eq!(
        resize(&pixels, 4, 1).rgba[..8],
        [255, 0, 0, 255, 255, 0, 0, 255]
    );
    // 长边超过上限才缩小，且比例不变；分段并行与单段处理的结果相同。
    let big = Pixels {
        width: 4000,
        height: 2000,
        rgba: (0..4000 * 2000 * 4).map(|i| (i * 7 % 251) as u8).collect(),
    };
    let small = shrink(big.clone(), KEEP_MAX);
    assert_eq!((small.width, small.height), (KEEP_MAX, KEEP_MAX / 2));
    let mut whole = vec![0; small.rgba.len()];
    resize_rows(
        &big,
        KEEP_MAX as usize,
        KEEP_MAX as usize / 2,
        0,
        &mut whole,
    );
    assert!(whole == small.rgba);
    assert_eq!(size_label(245 * 1024), "245 KB");
    assert_eq!(size_label(1_468_006), "1.4 MB");
}

#[cfg(target_os = "macos")]
#[test]
fn big_jpegs_decode_scaled_like_the_whole_picture() {
    // 2800×1600：上半左红右蓝，下半绿。JPEG 由系统的 `sips` 现场写出，测试数据里不放几十 KB
    // 的图片。
    let (width, height) = (2800u32, 1600u32);
    let rgba: Vec<u8> = (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| match (y < height / 2, x < width / 2) {
                (true, true) => [208, 32, 32, 255],
                (true, false) => [32, 64, 208, 255],
                _ => [32, 160, 48, 255],
            })
        })
        .collect();
    let dir = std::env::temp_dir().join(format!("winc-preview-jpeg-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let (source, target) = (dir.join("in.png"), dir.join("out.jpg"));
    fs::write(&source, png(width, height, png::ColorType::Rgba, &rgba)).unwrap();
    let status = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg"])
        .arg(&source)
        .arg("--out")
        .arg(&target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = fs::read(&target).unwrap();
    let _ = fs::remove_dir_all(&dir);

    // ImageIO 按目标大小解码：大小、方向和颜色都和 zune-jpeg 整张解出再缩小的一样（JPEG 有损，
    // 两边的缩放算法也不同，颜色差几级、长边差一个像素以内）。
    assert!(
        imageio::decode_scaled(&bytes, KEEP_MAX).is_some(),
        "ImageIO 解不出来"
    );
    let (scaled, size) = decode_jpeg_fitted(&bytes).unwrap();
    let whole = shrink(decode_jpeg(&bytes).unwrap(), KEEP_MAX);
    assert_eq!(size, (width, height));
    assert_eq!(scaled.width, KEEP_MAX);
    assert!(scaled.height.abs_diff(whole.height) <= 1);
    let at = |p: &Pixels, x: u32, y: u32| {
        let i = ((y * p.width + x) * 4) as usize;
        [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]]
    };
    let (w, h) = (whole.width, whole.height.min(scaled.height));
    for (x, y) in [(w / 4, h / 4), (w * 3 / 4, h / 4), (w / 2, h * 3 / 4)] {
        let (a, b) = (at(&scaled, x, y), at(&whole, x, y));
        assert!(
            a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 8),
            "({x}, {y}): {a:?} vs {b:?}"
        );
    }
}

/// 把 `rgba` 编成 Adam7 隔行扫描的 RGBA PNG（`png` 库只能写不隔行的）：各遍的行前加过滤方式
/// 0，压缩用不压缩的 deflate 块。
fn interlaced_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut raw = Vec::new();
    for (x_step, x_start, y_step, y_start) in ADAM7 {
        if w.saturating_sub(x_start) == 0 {
            continue;
        }
        for y in (y_start..h).step_by(y_step) {
            raw.push(0);
            for x in (x_start..w).step_by(x_step) {
                raw.extend_from_slice(&rgba[(y * w + x) * 4..][..4]);
            }
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xffff).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let crc = |bytes: &[u8]| {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // 8 位 RGBA，压缩、过滤方式 0，隔行扫描 1（Adam7）。
    header.extend_from_slice(&[8, 6, 0, 0, 1]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    out
}

/// 回归：隔行扫描的 PNG 逐遍累加到缩小后的图上，结果与不隔行的同一张图相同。
///
/// 按原尺寸整张解码再缩小，1 亿像素时峰值约 700 MB。
#[test]
fn interlaced_pngs_shrink_without_decoding_the_whole_picture() {
    let (width, height) = (KEEP_MAX * 2 + 7, 13);
    let mut seed = 7u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 16) as u8
    };
    let rgba: Vec<u8> = (0..width * height * 4).map(|_| next()).collect();
    let plain = decode_png(&png(width, height, png::ColorType::Rgba, &rgba)).unwrap();
    let bytes = interlaced_png(width, height, &rgba);
    let interlaced = decode_png(&bytes).unwrap();
    assert_eq!(interlaced.1, (width, height));
    assert_eq!((interlaced.0.width, interlaced.0.height), (KEEP_MAX, 6));
    assert!(interlaced.0 == plain.0);
    // 小图不缩小，仍然整张解码。
    let small: Vec<u8> = (0..5 * 3 * 4).map(|_| next()).collect();
    let (pixels, _) = decode_png(&interlaced_png(5, 3, &small)).unwrap();
    assert_eq!(pixels.rgba, small);
    // 数据不足时返回错误，不 panic。
    assert!(decode_png(&bytes[..bytes.len() / 2]).is_err());
}
