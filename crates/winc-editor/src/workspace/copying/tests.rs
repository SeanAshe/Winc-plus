use std::fs;
use std::io;

use super::*;
use crate::test_util::scratch;

#[test]
fn progress_follows_the_item_being_copied() {
    let dir = scratch("copying-progress");
    fs::write(dir.join("a.txt"), "hello").unwrap();
    fs::create_dir(dir.join("kit")).unwrap();
    fs::write(dir.join("kit/b.txt"), "abc").unwrap();
    fs::write(dir.join("kit/c.txt"), "defg").unwrap();
    let copying = Copying::default();
    let links: Links = |from, to| winc_backend::fs::copy_symlink(from, to);

    // 单个文件：名字、大小和已复制的字节数。
    copy_item(&dir.join("a.txt"), &dir.join("a2.txt"), links, &copying).unwrap();
    assert_eq!(copying.item(), ("a.txt".to_owned(), Some(5)));
    assert_eq!(copying.copied(), 5);

    // 目录：大小事先不知道，已复制的字节数从零重新累计。
    copy_item(&dir.join("kit"), &dir.join("kit2"), links, &copying).unwrap();
    assert_eq!(copying.item(), ("kit".to_owned(), None));
    assert_eq!(copying.copied(), 7);
    assert_eq!(fs::read_to_string(dir.join("kit2/c.txt")).unwrap(), "defg");
}

#[test]
fn a_cancelled_copy_leaves_nothing_behind() {
    let dir = scratch("copying-cancel");
    fs::create_dir(dir.join("kit")).unwrap();
    fs::write(dir.join("kit/b.txt"), "abc").unwrap();
    let copying = Copying::default();
    copying.cancel();
    let links: Links = |from, to| winc_backend::fs::copy_symlink(from, to);

    let error = copy_item(&dir.join("kit"), &dir.join("kit2"), links, &copying).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(!dir.join("kit2").exists());
    // 取消不影响源。
    assert!(dir.join("kit/b.txt").exists());
}
