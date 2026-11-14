//! 目录读取的测试：测试用的 PDF 由代码生成，覆盖交叉引用表、交叉引用流和对象流、各种标题编码、
//! 按名字查目标位置，以及损坏、循环、加密的文件。

use super::*;

/// 把对象（对象号、内容）写成 PDF，交叉引用表的偏移按 `offsets` 改写后的结果（测试损坏的表）。
fn build(objects: &[(u32, &str)], trailer: &str, shift: isize) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let size = objects.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let mut offsets = vec![None; size as usize];
    for (n, body) in objects {
        offsets[*n as usize] = Some(out.len());
        out.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets[1..] {
        let line = match offset {
            Some(at) => format!("{:010} 00000 n \n", (*at as isize + shift).max(0)),
            None => "0000000000 65535 f \n".to_owned(),
        };
        out.extend_from_slice(line.as_bytes());
    }
    out.extend_from_slice(format!("trailer\n{trailer}\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

/// 三页、两层页树的文档：页对象号 10、11、12，目录从对象 20 起由 `items` 给出。
fn book(items: &[(u32, &str)], extra: &[(u32, &str)], catalog_extra: &str) -> Vec<(u32, String)> {
    let mut objects = vec![
        (
            1,
            format!("<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R {catalog_extra} >>"),
        ),
        (
            2,
            "<< /Type /Pages /Kids [4 0 R 12 0 R] /Count 3 >>".to_owned(),
        ),
        (3, "<< /Type /Outlines /First 20 0 R >>".to_owned()),
        (
            4,
            "<< /Type /Pages /Kids [10 0 R 11 0 R] /Count 2 >>".to_owned(),
        ),
        (10, "<< /Type /Page /Parent 4 0 R >>".to_owned()),
        (11, "<< /Type /Page /Parent 4 0 R >>".to_owned()),
        (12, "<< /Type /Page /Parent 2 0 R >>".to_owned()),
    ];
    objects.extend(
        items
            .iter()
            .chain(extra)
            .map(|(n, b)| (*n, (*b).to_owned())),
    );
    objects
}

fn pdf(objects: &[(u32, String)], trailer: &str) -> Vec<u8> {
    let borrowed: Vec<(u32, &str)> = objects.iter().map(|(n, b)| (*n, b.as_str())).collect();
    build(&borrowed, trailer, 0)
}

fn item(title: &str, level: usize, page: Option<usize>) -> OutlineItem {
    OutlineItem {
        title: title.to_owned(),
        level,
        page,
        top: None,
    }
}

#[test]
fn outline_reads_titles_levels_and_pages() {
    let objects = book(
        &[
            // 字面字符串、跳到第 1 页；有下级。
            (
                20,
                "<< /Title (Intro \\(1\\)) /Dest [10 0 R /XYZ 0 0 0] /First 21 0 R /Next 22 0 R >>",
            ),
            // UTF-16BE 的标题，`/A` 里的跳转。
            (
                21,
                "<< /Title <FEFF4E2D6587> /A << /S /GoTo /D [11 0 R /Fit] >> >>",
            ),
            // PDFDocEncoding（0x95 是 Ł），按名字查名字树。
            (
                22,
                "<< /Title (\\225\\363d\\274) /Dest (third) /Next 23 0 R >>",
            ),
            // 旧式的 `/Dests` 字典按名字查；网址没有页。
            (23, "<< /Title (Old) /Dest /second /Next 24 0 R >>"),
            (
                24,
                "<< /Title (Web) /A << /S /URI /URI (https://example.com) >> >>",
            ),
        ],
        &[
            (30, "<< /Kids [31 0 R] >>"),
            (
                31,
                "<< /Limits [(a) (z)] /Names [(third) << /D [12 0 R /Fit] >>] >>",
            ),
            (32, "<< /second [11 0 R /Fit] >>"),
        ],
        "/Names << /Dests 30 0 R >> /Dests 32 0 R",
    );
    let items = outline(&pdf(&objects, "<< /Size 33 /Root 1 0 R >>"));
    assert_eq!(
        items,
        [
            item("Intro (1)", 0, Some(0)),
            item("中文", 1, Some(1)),
            item("Łód\u{bc}", 0, Some(2)),
            item("Old", 0, Some(1)),
            item("Web", 0, None),
        ]
    );
}

/// 页内位置按显示的页框换算成到上沿的比例：页框由上级继承，`/CropBox` 优先；`/XYZ`、`/FitH`、
/// `/FitR` 取上沿，`/Fit` 和上沿为 null 的只到整页；横着放的页不换算。
#[test]
fn destinations_carry_the_position_within_the_page() {
    let mut objects = book(
        &[
            (
                20,
                "<< /Title (A) /Dest [10 0 R /XYZ 72 600 0] /Next 21 0 R >>",
            ),
            (
                21,
                "<< /Title (B) /Dest [10 0 R /FitH 200.0] /Next 22 0 R >>",
            ),
            (
                22,
                "<< /Title (C) /Dest [11 0 R /FitR 0 0 100 400] /Next 23 0 R >>",
            ),
            (
                23,
                "<< /Title (D) /Dest [11 0 R /XYZ 0 null 0] /Next 24 0 R >>",
            ),
            (24, "<< /Title (E) /Dest [10 0 R /Fit] /Next 25 0 R >>"),
            (25, "<< /Title (F) /Dest [12 0 R /XYZ 0 300 0] >>"),
        ],
        &[],
        "",
    );
    for (n, body) in &mut objects {
        match n {
            2 => *body = "<< /Type /Pages /Kids [4 0 R 12 0 R] /MediaBox [0 0 600 800] >>".into(),
            11 => *body = "<< /Type /Page /CropBox [0 100 600 700] >>".into(),
            12 => *body = "<< /Type /Page /Rotate 90 >>".into(),
            _ => {}
        }
    }
    let items = outline(&pdf(&objects, "<< /Size 26 /Root 1 0 R >>"));
    let places: Vec<(&str, Option<usize>, Option<f32>)> = (items.iter())
        .map(|i| (i.title.as_str(), i.page, i.top))
        .collect();
    assert_eq!(
        places,
        [
            ("A", Some(0), Some(0.25)),
            ("B", Some(0), Some(0.75)),
            ("C", Some(1), Some(0.5)),
            ("D", Some(1), None),
            ("E", Some(0), None),
            ("F", Some(2), None),
        ]
    );
}

#[test]
fn outline_reads_cross_reference_and_object_streams() {
    use miniz_oxide::deflate::compress_to_vec_zlib;
    // 目录和页放在对象流里，交叉引用流用 PNG 的 Up 预测（每行宽 4 字节）。
    let packed = [
        (1, "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>"),
        (2, "<< /Type /Pages /Kids [4 0 R] /Count 1 >>"),
        (3, "<< /First 5 0 R >>"),
        (4, "<< /Type /Page /Parent 2 0 R >>"),
        (5, "<< /Title (Packed) /Dest [4 0 R /Fit] >>"),
    ];
    let mut header = String::new();
    let mut body = String::new();
    for (n, text) in packed {
        header.push_str(&format!("{n} {} ", body.len()));
        body.push_str(text);
        body.push(' ');
    }
    let stream = compress_to_vec_zlib(format!("{header}{body}").as_bytes(), 6);
    let mut out = b"%PDF-1.7\n".to_vec();
    let objstm = out.len();
    out.extend_from_slice(
        format!(
            "6 0 obj\n<< /Type /ObjStm /N 5 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            header.len(),
            stream.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&stream);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = out.len();
    // 各行：类型、字段二（偏移或对象流号，2 字节）、字段三（序号）；对象 0 到 7。
    let mut rows: Vec<[u8; 4]> = vec![[0, 0, 0, 0]];
    rows.extend((0..5u8).map(|i| [2, 0, 6, i]));
    rows.push([1, (objstm >> 8) as u8, objstm as u8, 0]);
    rows.push([1, (xref >> 8) as u8, xref as u8, 0]);
    let mut predicted = Vec::new();
    let mut previous = [0u8; 4];
    for row in &rows {
        predicted.push(2);
        predicted.extend(row.iter().zip(previous).map(|(b, p)| b.wrapping_sub(p)));
        previous = *row;
    }
    let data = compress_to_vec_zlib(&predicted, 6);
    out.extend_from_slice(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /W [1 2 1] /Root 1 0 R /Filter /FlateDecode \
             /DecodeParms << /Predictor 12 /Columns 4 >> /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    assert_eq!(outline(&out), [item("Packed", 0, Some(0))]);
}

#[test]
fn a_broken_cross_reference_table_falls_back_to_scanning() {
    let objects = book(
        &[(20, "<< /Title (Scanned) /Dest [12 0 R /Fit] >>")],
        &[],
        "",
    );
    let borrowed: Vec<(u32, &str)> = objects.iter().map(|(n, b)| (*n, b.as_str())).collect();
    // 每个偏移都错开 7 个字节。
    let bytes = build(&borrowed, "<< /Size 21 /Root 1 0 R >>", 7);
    assert_eq!(outline(&bytes), [item("Scanned", 0, Some(2))]);
}

#[test]
fn malformed_files_yield_no_outline() {
    // 互相指向的两条只各读一次。
    let looped = book(
        &[
            (20, "<< /Title (A) /Next 21 0 R >>"),
            (21, "<< /Title (B) /Next 20 0 R /First 20 0 R >>"),
        ],
        &[],
        "",
    );
    let items = outline(&pdf(&looped, "<< /Size 22 /Root 1 0 R >>"));
    assert_eq!(items, [item("A", 0, None), item("B", 0, None)]);

    // 加密的文件不读目录。
    let encrypted = book(
        &[(20, "<< /Title (Secret) >>")],
        &[(40, "<< /Filter /Standard >>")],
        "",
    );
    let trailer = "<< /Size 41 /Root 1 0 R /Encrypt 40 0 R >>";
    assert!(outline(&pdf(&encrypted, trailer)).is_empty());

    // 不是 PDF、截断的、只有开头的文件都只是没有目录。
    let whole = pdf(
        &book(&[(20, "<< /Title (X) >>")], &[], ""),
        "<< /Size 21 /Root 1 0 R >>",
    );
    for bytes in [
        &b"not a pdf"[..],
        &whole[..whole.len() / 2],
        b"%PDF-1.7\n",
        b"%PDF-1.7\n1 0 obj << /A [[[[[",
    ] {
        assert!(outline(bytes).is_empty());
    }
}

#[test]
fn titles_decode_the_text_string_encodings() {
    assert_eq!(text(b"\xFE\xFF\x4E\x2D\x65\x87"), "中文");
    assert_eq!(text(b"\xEF\xBB\xBFUTF-8 \xE7\xAB\xA0"), "UTF-8 章");
    assert_eq!(text("第一章".as_bytes()), "第一章");
    assert_eq!(text(b"\x80 Bullet\n"), "\u{2022} Bullet");
    assert_eq!(text(b"Line\rbreak"), "Line break");
}
