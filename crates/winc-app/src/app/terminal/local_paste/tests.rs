//! 粘贴到本机终端的图片的测试：存为文件、重名不覆盖、删去旧图片。

use super::*;

/// 每次存为新的文件，内容是剪贴板中的 PNG；存放超过 [`KEEP`] 的 PNG 被删去，其他文件不动。
#[test]
fn each_image_is_saved_to_a_new_file_and_old_ones_are_removed() {
    let dir = std::env::temp_dir().join(format!("winc paste {}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old.png");
    let other = dir.join("old.txt");
    let long_ago = SystemTime::now() - KEEP - Duration::from_secs(60);
    for file in [&old, &other] {
        std::fs::write(file, b"x").unwrap();
        let handle = std::fs::File::options().write(true).open(file).unwrap();
        handle.set_modified(long_ago).unwrap();
    }

    let png = winc_editor::paste::encode_png(1, 1, &[12, 34, 56, 255]).unwrap();
    let first = save(&dir, ClipImage::Png(png.clone())).unwrap();
    let second = save(&dir, ClipImage::Png(png.clone())).unwrap();
    assert_ne!(first, second);
    for file in [&first, &second] {
        assert_eq!(file.parent(), Some(dir.as_path()));
        assert!(file.to_string_lossy().ends_with("paste.png"), "{file:?}");
        assert_eq!(std::fs::read(file).unwrap(), png);
    }
    assert!(!old.exists() && other.exists());

    // 像素数据与尺寸不符时无法编码，不留下文件。
    let broken = ClipImage::Rgba {
        width: 2,
        height: 2,
        bytes: vec![0; 3],
    };
    assert!(save(&dir, broken).is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}
