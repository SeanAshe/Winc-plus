//! 十六进制视图的测试：跨块读取、文件变化后重新打开、选区与复制的写法。

use super::*;
use crate::test_util::scratch;

#[test]
fn bytes_are_read_across_blocks_and_files_reload() {
    let dir = scratch("hex-read");
    let path = dir.join("data.bin");
    let data: Vec<u8> = (0..BLOCK * 2 + 10).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &data).unwrap();
    let mut view = HexView::open(&path);
    assert_eq!(view.size(), data.len() as u64);
    assert_eq!(view.rows(), data.len().div_ceil(ROW as usize));
    let from = BLOCK - 3;
    assert_eq!(view.bytes(from, 8), data[from as usize..from as usize + 8]);
    // 读到文件末尾为止。
    let tail = data.len() as u64 - 4;
    assert_eq!(view.bytes(tail, 100), data[tail as usize..]);
    assert!(view.bytes(data.len() as u64, 4).is_empty());

    view.move_to(tail + 2, false);
    std::fs::write(&path, b"MZ\x90\x00").unwrap();
    let metadata = winc_backend::fs::metadata(&path).unwrap();
    assert!(view.outdated(&metadata));
    view.refresh(&metadata);
    assert_eq!((view.size(), view.rows()), (4, 1));
    assert_eq!(view.bytes(0, 16), b"MZ\x90\x00");
    assert_eq!(view.selection(), 3..4);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn selection_copy_and_missing_files() {
    let dir = scratch("hex-select");
    let path = dir.join("a.bin");
    std::fs::write(&path, b"MZ\x90\x00This").unwrap();
    let mut view = HexView::open(&path);
    view.move_to(1, false);
    view.move_to(5, true);
    assert_eq!(view.selection(), 1..6);
    assert_eq!(hex_text(&view.bytes(1, 5)), "5A 90 00 54 68");
    view.move_to(100, true);
    assert_eq!(view.selection(), 1..8);
    view.select_all();
    assert_eq!(view.selection(), 0..8);
    assert_eq!(
        (ascii(b'M'), ascii(0), ascii(b' ')),
        (Some('M'), None, Some(' '))
    );

    let gone = HexView::open(&dir.join("missing.bin"));
    assert!(gone.error.is_some());
    assert_eq!((gone.size(), gone.rows(), gone.selection()), (0, 1, 0..0));
    std::fs::remove_dir_all(&dir).unwrap();
}
