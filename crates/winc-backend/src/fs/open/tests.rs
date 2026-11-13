//! 打开：开头的一段、可重复的顺序读取、按偏移读取，以及文件被替换后句柄的指向。

use std::io::Read;
use std::path::PathBuf;

use super::*;
use crate::{Target, virtual_path};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-backend-open-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_head_and_the_whole_content_come_from_one_opening() {
    let dir = scratch("read");
    let file = dir.join("note.txt");
    std::fs::write(&file, "0123456789").unwrap();

    let mut opened = open(&file, 4).unwrap();
    assert_eq!(opened.head, b"0123");
    assert_eq!((opened.metadata.len, opened.metadata.is_file()), (10, true));
    // 顺序读取每次都从开头开始，与已读出的开头无关。
    for _ in 0..2 {
        let mut all = Vec::new();
        opened.reader().unwrap().read_to_end(&mut all).unwrap();
        assert_eq!(all, b"0123456789");
    }
    // 文件比要求的开头短时，开头就是全部内容。
    assert_eq!(open(&file, 64).unwrap().head, b"0123456789");
    assert!(open(&file, 0).unwrap().head.is_empty());
    assert_eq!(
        open(dir.join("missing"), 4).err().unwrap().kind(),
        io::ErrorKind::NotFound
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn handles_read_at_offsets_and_keep_the_file_they_opened() {
    let dir = scratch("handle");
    let file = dir.join("data.bin");
    std::fs::write(&file, "abcdefgh").unwrap();
    let handle = open(&file, 0).unwrap().into_handle();
    let copy = handle.try_clone().unwrap();

    let mut buf = [0u8; 3];
    assert_eq!(handle.read_at(2, &mut buf).unwrap(), 3);
    assert_eq!(&buf, b"cde");
    assert_eq!(copy.read_at(6, &mut buf).unwrap(), 2);
    assert_eq!(&buf[..2], b"gh");
    assert_eq!(handle.read_at(8, &mut buf).unwrap(), 0);
    // 读满缓冲区，或在文件末尾提前结束。
    let mut wide = [0u8; 6];
    assert_eq!(handle.read_full(1, &mut wide).unwrap(), 6);
    assert_eq!(&wide, b"bcdefg");
    assert_eq!(handle.read_full(5, &mut wide).unwrap(), 3);
    assert_eq!(&wide[..3], b"fgh");
    assert_eq!(handle.read_full(20, &mut wide).unwrap(), 0);
    assert_eq!(handle.metadata().unwrap().len, 8);

    // 文件被改名替换：句柄仍指向打开时的内容，路径上已是新文件。
    let fresh = dir.join("fresh");
    std::fs::write(&fresh, "REPLACED-CONTENT").unwrap();
    crate::local::replace_file(&fresh, &file).unwrap();
    assert_eq!(handle.read_at(0, &mut buf).unwrap(), 3);
    assert_eq!(&buf, b"abc");
    assert_eq!(handle.metadata().unwrap().len, 8);
    assert_eq!(crate::fs::metadata(&file).unwrap().len, 16);
    drop((handle, copy));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn virtual_paths_need_a_connection() {
    let file = virtual_path(&Target::new("unconnected-open").unwrap(), b"/etc/hosts");
    assert_eq!(
        open(&file, 16).err().unwrap().kind(),
        io::ErrorKind::NotConnected
    );
}
