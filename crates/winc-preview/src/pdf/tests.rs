//! `pdf` 的测试（macOS）：打开手写的 PDF，读出各页大小和旋转，按指定大小渲染。

use super::*;

/// 写一份 PDF：每页一个 `(宽, 高, 旋转)`，内容是在左下角 (72, 72) 处填一个 100 × 100 的红色
/// 方块。交叉引用表按各对象的实际偏移写出。
pub(super) fn sample_pdf(pages: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages.len())
        .map(|k| format!("{} 0 R", 3 + k * 2))
        .collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_owned());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    ));
    let content = "1 0 0 rg 72 72 100 100 re f";
    for (k, (w, h, rotate)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Rotate {rotate} \
             /Contents {} 0 R >>",
            4 + k * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", k + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// 把 `bytes` 写进临时目录下的 `name`，返回路径。
fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-pdf-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn pages_report_their_sizes_and_rotation() {
    let path = write("sizes.pdf", &sample_pdf(&[(612, 792, 0), (612, 792, 90)]));
    let doc = Document::open(&path).unwrap();
    let pages = doc.pages();
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].width, pages[0].height), (612.0, 792.0));
    // 旋转 90 度的页面按显示方向报告宽高。
    assert_eq!((pages[1].width, pages[1].height), (792.0, 612.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_page_renders_at_the_requested_size_on_white() {
    let path = write("render.pdf", &sample_pdf(&[(200, 200, 0)]));
    let doc = Document::open(&path).unwrap();
    // 2 倍：方块占 (144, 144) 到 (344, 344)，位图的第 0 行是页面顶部。
    let pixels = doc.render(0, 400, 400).unwrap();
    assert_eq!((pixels.width, pixels.height), (400, 400));
    let at = |x: usize, y: usize| {
        let i = (y * 400 + x) * 4;
        [
            pixels.rgba[i],
            pixels.rgba[i + 1],
            pixels.rgba[i + 2],
            pixels.rgba[i + 3],
        ]
    };
    assert_eq!(at(10, 10), [255, 255, 255, 255]);
    // 方块的中心在页面坐标 (122, 122)，即位图中 (244, 400 - 244)。
    let [r, g, b, a] = at(244, 156);
    assert!(
        r > 200 && g < 60 && b < 60 && a == 255,
        "{:?}",
        [r, g, b, a]
    );
    assert!(doc.render(1, 10, 10).is_err());
    assert!(doc.render(0, 0, 10).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn broken_files_report_why() {
    let path = write("broken.pdf", b"not a pdf");
    assert_eq!(
        Document::open(&path).err().as_deref(),
        Some(winc_i18n::t!("preview-pdf-invalid"))
    );
    let _ = std::fs::remove_file(&path);
    assert!(is_pdf(Path::new("报告.PDF")));
    assert!(!is_pdf(Path::new("报告.pdfx")));
    assert!(supported());
}
