//! `icons` 的测试：提交的 ICO 与 SVG 一致，以及由它转换成的 Windows 资源文件的结构。

use super::*;

#[path = "../../../crates/winc-app/build_icon.rs"]
mod build_icon;

/// 读取提交在仓库里、嵌入 Windows 可执行文件的 ICO。
fn committed_ico() -> Vec<u8> {
    std::fs::read(root().join("crates/winc-app/assets/winc.ico")).unwrap()
}

/// 返回 ICO 中各图像的（目录中的边长字节，PNG 数据）。
fn images(ico: &[u8]) -> Vec<(u8, &[u8])> {
    let count = usize::from(u16::from_le_bytes([ico[4], ico[5]]));
    (0..count)
        .map(|i| {
            let entry = &ico[6 + 16 * i..6 + 16 * (i + 1)];
            let long = |at: usize| u32::from_le_bytes(entry[at..at + 4].try_into().unwrap());
            let (size, offset) = (long(8) as usize, long(12) as usize);
            (entry[0], &ico[offset..offset + size])
        })
        .collect()
}

#[test]
fn the_committed_icon_matches_the_svg() {
    let text = std::fs::read_to_string(root().join("crates/winc-app/assets/icon.svg")).unwrap();
    let picture = winc_preview::svg::load(&text, None, None).unwrap();
    let ico = committed_ico();
    let images = images(&ico);
    assert_eq!(images.len(), 4);
    for (dimension, bytes) in images {
        let size = if dimension == 0 {
            256
        } else {
            u32::from(dimension)
        };
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut decoded).unwrap();
        // 浮点运算在不同的处理器上有细微差别，允许每个分量差一点。
        let expected = picture.render(size, size).rgba;
        let same = decoded.len() == expected.len()
            && decoded
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.abs_diff(*b) <= 2);
        assert!(
            same,
            "重新生成 crates/winc-app/assets/winc.ico（cargo xtask icons）：{size}"
        );
    }
}

#[test]
fn the_icon_becomes_resources_the_linker_accepts() {
    let ico = committed_ico();
    let res = build_icon::icon_resources(&ico, 1).unwrap();

    // 逐条读出（类型，名称，数据）。
    let mut records = Vec::new();
    let mut at = 0;
    while at < res.len() {
        let long = |at: usize| u32::from_le_bytes(res[at..at + 4].try_into().unwrap());
        let word = |at: usize| u16::from_le_bytes(res[at..at + 2].try_into().unwrap());
        let (size, header) = (long(at) as usize, long(at + 4) as usize);
        assert_eq!(header, 32);
        assert_eq!((word(at + 8), word(at + 12)), (0xffff, 0xffff));
        let data = &res[at + header..at + header + size];
        records.push((word(at + 10), word(at + 14), data));
        at = (at + header + size).next_multiple_of(4);
    }

    // 第一条是全零的文件头，然后是各个尺寸，最后是图标组。
    assert_eq!(records[0], (0, 0, &[][..]));
    let images = images(&ico);
    assert_eq!(records.len(), images.len() + 2);
    for (i, (_, bytes)) in images.iter().enumerate() {
        assert_eq!(records[i + 1], (3, i as u16 + 1, *bytes));
    }
    let (kind, name, group) = *records.last().unwrap();
    assert_eq!((kind, name), (14, 1));
    assert_eq!(group.len(), 6 + 14 * images.len());
    assert_eq!(group[..6], [0, 0, 1, 0, images.len() as u8, 0]);
    // 最后两字节是该尺寸的编号。
    assert_eq!(group[6 + 14 - 2..6 + 14], [1, 0]);
}

#[test]
fn each_icon_group_gets_its_own_image_ids() {
    let ico = committed_ico();
    let res = build_icon::icon_groups(&[(1, &ico), (2, &ico)]).unwrap();
    let mut groups = Vec::new();
    let mut images = Vec::new();
    let mut at = 0;
    while at < res.len() {
        let long = |at: usize| u32::from_le_bytes(res[at..at + 4].try_into().unwrap());
        let word = |at: usize| u16::from_le_bytes(res[at..at + 2].try_into().unwrap());
        let (size, header) = (long(at) as usize, long(at + 4) as usize);
        let (kind, name) = (word(at + 10), word(at + 14));
        if kind == 3 {
            images.push(name);
        } else if kind == 14 {
            groups.push(name);
        }
        at = (at + header + size).next_multiple_of(4);
    }
    assert_eq!(groups, [1, 2]);
    assert_eq!(images, (1..=8).collect::<Vec<_>>());
}

#[test]
fn each_file_icon_size_keeps_its_own_image() {
    let dir = root().join("crates/winc-app/assets/file-icons");
    let load = |size: u32| std::fs::read(dir.join(format!("md.targetsize-{size}.png"))).unwrap();
    let packed: Vec<(u32, Vec<u8>)> = [16, 32, 48, 512]
        .into_iter()
        .map(|size| (size, build_icon::icon_payload(&load(size), size)))
        .collect();
    assert!(packed[0].1.starts_with(&40u32.to_le_bytes()));
    assert!(packed[1].1.starts_with(&40u32.to_le_bytes()));
    assert_ne!(packed[0].1, packed[1].1);
    assert!(packed[3].1.starts_with(b"\x89PNG"));
    let ico = build_icon::pngs_to_ico(&packed);
    assert_eq!(u16::from_le_bytes([ico[4], ico[5]]), 4);
    assert_eq!([ico[6], ico[22], ico[38], ico[54]], [16, 32, 48, 0]);
}

#[test]
fn a_file_icon_is_a_page_with_a_colored_band() {
    let pixels = build_icon::draw_file_icon(48, "MD", 0x44709E);
    assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
    assert!(pixels.chunks(4).any(|px| px == [0x44, 0x70, 0x9E, 255]));
    let plain = build_icon::draw_file_icon(48, "", 0);
    assert!(plain.chunks(4).any(|px| px[3] == 255));
    assert!(!plain.chunks(4).any(|px| px == [0x44, 0x70, 0x9E, 255]));
}

#[test]
fn a_file_that_is_not_an_icon_is_rejected() {
    assert_eq!(build_icon::icon_resources(b"not an icon", 1), None);
    // 目录项指向文件之外。
    let mut ico = vec![0, 0, 1, 0, 1, 0];
    ico.extend([0; 8]);
    ico.extend(100u32.to_le_bytes());
    ico.extend(22u32.to_le_bytes());
    assert_eq!(build_icon::icon_resources(&ico, 1), None);
}

/// macOS 图标的底板位于模板规定的位置：在 1024 × 1024 的画布上距各边约 100 像素；`ICON_TILE`
/// 与 SVG 中底板的宽度相同。
#[test]
fn the_macos_icon_follows_the_app_icon_template() {
    let text = std::fs::read_to_string(root().join("crates/winc-app/assets/icon.svg")).unwrap();
    assert!(text.contains(&format!("width=\"{}\"", crate::dmg::ICON_TILE)));
    let picture = winc_preview::svg::load(&text, None, None).unwrap();
    let pixels = crate::dmg::macos_icon(&picture, 1024);
    assert_eq!((pixels.width, pixels.height), (1024, 1024));
    let opaque = |x: usize, y: usize| pixels.rgba[(y * 1024 + x) * 4 + 3] > 128;
    let left = (0..1024).find(|&x| opaque(x, 512)).unwrap();
    let right = (0..1024).rev().find(|&x| opaque(x, 512)).unwrap();
    let top = (0..1024).find(|&y| opaque(512, y)).unwrap();
    for edge in [left, 1023 - right, top] {
        assert!((98..=102).contains(&edge), "{left} {right} {top}");
    }
}
