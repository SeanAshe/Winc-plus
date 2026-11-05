//! 把 ICO 文件转换成 Windows 资源文件（`.res`）：链接器直接接受这种格式，不需要资源编译器。

/// 资源类型：单个图标。
const RT_ICON: u16 = 3;
/// 资源类型：图标组，列出同一个图标的各个尺寸。
const RT_GROUP_ICON: u16 = 14;
/// 资源的内存标志：可移动、用时才加载、可丢弃。
const MEMORY_FLAGS: u16 = 0x1030;
/// 资源的语言：与语言无关。
const NEUTRAL_LANGUAGE: u16 = 0;
/// ICO 文件头的长度。
const ICO_HEADER: usize = 6;
/// ICO 目录项的长度。
const ICO_ENTRY: usize = 16;

/// 把 ICO 文件的内容 `ico` 转换成 `.res` 文件的内容；图标组的编号是 `group`。
///
/// 各尺寸的编号从 1 起。文件头不合法或目录项超出文件范围时返回 `None`。
pub fn icon_resources(ico: &[u8], group: u16) -> Option<Vec<u8>> {
    icon_groups(&[(group, ico)])
}

/// 把多组 ICO 合成一个 `.res`：每组一个图标（应用图标、各文件类型图标）。
///
/// 图像编号在整个文件里连续，不随组重新从 1 起，否则两组会抢同一个 `RT_ICON`。
/// 某一组的文件头不合法或目录项超出文件范围时返回 `None`。
pub fn icon_groups(groups: &[(u16, &[u8])]) -> Option<Vec<u8>> {
    // `.res` 文件以一条类型和名称都是 0 的空记录开头，用来和 16 位格式区分。
    let mut res = Vec::new();
    push_resource(&mut res, 0, 0, 0, 0, &[]);
    let mut image_id = 1u16;
    for &(group, ico) in groups {
        let word = |at: usize| Some(u16::from_le_bytes(ico.get(at..at + 2)?.try_into().ok()?));
        let long = |at: usize| Some(u32::from_le_bytes(ico.get(at..at + 4)?.try_into().ok()?));
        if word(0)? != 0 || word(2)? != 1 {
            return None;
        }
        let count = word(4)?;
        let mut directory = vec![0, 0, 1, 0];
        directory.extend(count.to_le_bytes());
        for index in 0..count {
            let entry = ICO_HEADER + ICO_ENTRY * usize::from(index);
            let size = long(entry + 8)?;
            let offset = long(entry + 12)? as usize;
            let image = ico.get(offset..offset.checked_add(size as usize)?)?;
            let id = image_id;
            image_id = image_id.checked_add(1)?;
            push_resource(&mut res, RT_ICON, id, MEMORY_FLAGS, NEUTRAL_LANGUAGE, image);
            // 前 12 字节（尺寸、颜色数、位面、位深、数据长度）两种目录相同，最后的文件偏移换成编号。
            directory.extend(ico.get(entry..entry + 12)?);
            directory.extend(id.to_le_bytes());
        }
        push_resource(
            &mut res,
            RT_GROUP_ICON,
            group,
            MEMORY_FLAGS,
            NEUTRAL_LANGUAGE,
            &directory,
        );
    }
    Some(res)
}

/// 把已经编码好的 PNG（`(边长, 文件内容)`）包成 ICO。边长 256 在目录里写成 0。
pub fn pngs_to_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut ico = vec![0, 0, 1, 0];
    ico.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, bytes) in images {
        let dimension = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend([dimension, dimension, 0, 0]);
        ico.extend(1u16.to_le_bytes());
        ico.extend(32u16.to_le_bytes());
        ico.extend((bytes.len() as u32).to_le_bytes());
        ico.extend((offset as u32).to_le_bytes());
        offset += bytes.len();
    }
    for (_, bytes) in images {
        ico.extend(bytes);
    }
    ico
}

/// 画一张 Notepads 那种文件图标：浅色折角纸，下面一条写着 `label` 的色带。
///
/// `label` 为空时不画色带，只在纸上画三行，当作没有专门图标的类型。`color` 是色带的
/// `0xRRGGBB`。返回 `size`×`size` 的 RGBA，直线向下，每像素 4 字节。
pub fn draw_file_icon(size: u32, label: &str, color: u32) -> Vec<u8> {
    let size = size.max(1);
    let mut pixels = vec![0u8; size as usize * size as usize * 4];
    let s = size as f32;
    let page = (s * 0.08, s * 0.05, s * 0.84, s * 0.90);
    let radius = s * 0.10;
    let fold = s * 0.26;
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let at = (y as usize * size as usize + x as usize) * 4;
            let page_cover = round_rect(px, py, page.0, page.1, page.2, page.3, radius);
            if page_cover > 0.0 {
                blend(&mut pixels[at..at + 4], [230, 232, 236, 255], page_cover);
            }
            // 右上角折下来的一角。
            let (ax, ay) = (page.0 + page.2 - fold, page.1);
            let (bx, by) = (page.0 + page.2, page.1);
            let (cx, cy) = (page.0 + page.2, page.1 + fold);
            if page_cover > 0.0 && in_triangle(px, py, ax, ay, bx, by, cx, cy) {
                blend(&mut pixels[at..at + 4], [186, 192, 200, 255], 1.0);
            }
            if label.is_empty() {
                for row in 0..3 {
                    let line = (
                        page.0 + page.2 * 0.16,
                        page.1 + page.3 * (0.38 + row as f32 * 0.16),
                        page.2 * 0.52,
                        (s * 0.035).max(1.0),
                    );
                    let cover = round_rect(px, py, line.0, line.1, line.2, line.3, line.3 * 0.5);
                    if cover > 0.0 {
                        blend(&mut pixels[at..at + 4], [188, 194, 202, 255], cover);
                    }
                }
            } else {
                let banner = (
                    page.0 + page.2 * 0.06,
                    page.1 + page.3 * 0.58,
                    page.2 * 0.88,
                    page.3 * 0.28,
                );
                let cover = round_rect(
                    px,
                    py,
                    banner.0,
                    banner.1,
                    banner.2,
                    banner.3,
                    banner.3 * 0.28,
                );
                if cover > 0.0 {
                    blend(&mut pixels[at..at + 4], [r, g, b, 255], cover);
                }
            }
        }
    }
    if !label.is_empty() {
        let banner = (
            page.0 + page.2 * 0.06,
            page.1 + page.3 * 0.58,
            page.2 * 0.88,
            page.3 * 0.28,
        );
        blit_label(&mut pixels, size, label, banner);
    }
    pixels
}

/// 圆角矩形在点 `(px, py)` 的覆盖率，边缘大约一个像素宽。
fn round_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, radius: f32) -> f32 {
    let radius = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let qx = (px - cx).abs() - (w * 0.5 - radius);
    let qy = (py - cy).abs() - (h * 0.5 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    (0.5 - (outside + inside - radius)).clamp(0.0, 1.0)
}

/// `(px, py)` 是否在三角形 `(ax, ay)-(bx, by)-(cx, cy)` 内（含边界）。
fn in_triangle(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> bool {
    let side = |x1: f32, y1: f32, x2: f32, y2: f32| (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2);
    let (ab, bc, ca) = (
        side(ax, ay, bx, by),
        side(bx, by, cx, cy),
        side(cx, cy, ax, ay),
    );
    (ab >= 0.0 && bc >= 0.0 && ca >= 0.0) || (ab <= 0.0 && bc <= 0.0 && ca <= 0.0)
}

/// 把 `src` 按覆盖率 `cover` 叠到 `dst`（RGBA）上。
fn blend(dst: &mut [u8], src: [u8; 4], cover: f32) {
    let alpha = f32::from(src[3]) / 255.0 * cover;
    if alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let under = f32::from(dst[channel]);
        dst[channel] = (f32::from(src[channel]) * alpha + under * (1.0 - alpha)).round() as u8;
    }
    let under = f32::from(dst[3]) / 255.0;
    dst[3] = ((alpha + under * (1.0 - alpha)) * 255.0).round() as u8;
}

/// 把 `label` 居中画进色带 `banner`（`x, y, w, h`），白色、5×7 点阵、按色带能放下的大小缩放。
fn blit_label(pixels: &mut [u8], size: u32, label: &str, banner: (f32, f32, f32, f32)) {
    let chars: Vec<char> = label.chars().collect();
    if chars.is_empty() {
        return;
    }
    let (x, y, w, h) = banner;
    let scale = (w * 0.86 / (chars.len() as f32 * 6.0)).min(h * 0.62 / 7.0);
    if scale <= 0.2 {
        return;
    }
    let text_w = chars.len() as f32 * 6.0 * scale - scale;
    let text_h = 7.0 * scale;
    let origin_x = x + (w - text_w) * 0.5;
    let origin_y = y + (h - text_h) * 0.5;
    for (index, ch) in chars.iter().enumerate() {
        let glyph = glyph(*ch);
        let gx = origin_x + index as f32 * 6.0 * scale;
        for row in 0..7 {
            for col in 0..5 {
                if glyph[row] & (1u8 << (4 - col)) == 0 {
                    continue;
                }
                fill_rect(
                    pixels,
                    size,
                    gx + col as f32 * scale,
                    origin_y + row as f32 * scale,
                    scale,
                    scale,
                );
            }
        }
    }
}

/// 用白色盖住轴对齐的矩形，边缘按覆盖率。
fn fill_rect(pixels: &mut [u8], size: u32, x: f32, y: f32, w: f32, h: f32) {
    let x1 = x.floor().max(0.0) as u32;
    let y1 = y.floor().max(0.0) as u32;
    let x2 = ((x + w).ceil() as u32).min(size);
    let y2 = ((y + h).ceil() as u32).min(size);
    for py in y1..y2 {
        for px in x1..x2 {
            let cover = (px as f32 + 1.0).min(x + w) - (px as f32).max(x);
            let cover_y = (py as f32 + 1.0).min(y + h) - (py as f32).max(y);
            let cover = (cover.max(0.0) * cover_y.max(0.0)).min(1.0);
            let at = (py as usize * size as usize + px as usize) * 4;
            blend(&mut pixels[at..at + 4], [255, 255, 255, 255], cover);
        }
    }
}

/// 5×7 点阵，每行低 5 位，最高位在左。没有的字符画成空的。
fn glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '+' => [
            0b00100, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00100,
        ],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        _ => [0; 7],
    }
}

/// 向 `res` 追加一条资源记录；类型和名称都是编号，记录按 4 字节对齐。
fn push_resource(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, language: u16, data: &[u8]) {
    res.extend((data.len() as u32).to_le_bytes());
    res.extend(32u32.to_le_bytes());
    res.extend([0xff, 0xff]);
    res.extend(kind.to_le_bytes());
    res.extend([0xff, 0xff]);
    res.extend(name.to_le_bytes());
    // 数据版本。
    res.extend(0u32.to_le_bytes());
    res.extend(flags.to_le_bytes());
    res.extend(language.to_le_bytes());
    // 版本和特征。
    res.extend([0; 8]);
    res.extend(data);
    res.resize(res.len().next_multiple_of(4), 0);
}
