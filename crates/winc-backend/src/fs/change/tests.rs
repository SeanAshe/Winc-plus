//! 改动文件系统的操作：结果与 `std::fs` 一致，虚拟路径被拒绝。

use std::path::PathBuf;

use super::*;
use crate::{Target, virtual_path};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-backend-change-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn folders_and_files_are_created_renamed_copied_and_removed() {
    let dir = scratch("basic");
    assert!(create_dir(dir.join("a/b")).is_err(), "上级目录不存在");
    create_dir_all(dir.join("a/b")).unwrap();
    create_dir_all(dir.join("a/b")).unwrap();
    create_dir(dir.join("a/c")).unwrap();

    let file = dir.join("a/b/note.txt");
    create_new(&file, b"one").unwrap();
    // 已有条目时不覆盖，内容不变。
    let error = create_new(&file, b"two").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&file).unwrap(), b"one");

    write(&file, "three").unwrap();
    let moved = dir.join("a/c/moved.txt");
    rename(&file, &moved).unwrap();
    assert!(!file.exists());
    assert_eq!(copy(&moved, &file).unwrap(), 5);
    assert_eq!(std::fs::read(&file).unwrap(), b"three");
    remove_file(&moved).unwrap();
    assert!(!moved.exists() && remove_file(&moved).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn links_are_copied_as_links_to_the_same_target() {
    let dir = scratch("links");
    std::fs::write(dir.join("target.txt"), "x").unwrap();
    // 相对的目标原样保留；指向不存在目标的链接同样可以复制。
    std::os::unix::fs::symlink("target.txt", dir.join("link")).unwrap();
    std::os::unix::fs::symlink("gone", dir.join("dangling")).unwrap();

    copy_symlink(dir.join("link"), dir.join("copy")).unwrap();
    copy_symlink(dir.join("dangling"), dir.join("copy2")).unwrap();
    assert_eq!(
        std::fs::read_link(dir.join("copy")).unwrap(),
        PathBuf::from("target.txt")
    );
    assert_eq!(
        std::fs::read_link(dir.join("copy2")).unwrap(),
        PathBuf::from("gone")
    );
    assert!(copy_symlink(dir.join("target.txt"), dir.join("copy3")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn virtual_paths_need_a_connection_and_both_paths_on_one_host() {
    let dir = scratch("virtual");
    let local = dir.join("local.txt");
    std::fs::write(&local, "x").unwrap();
    let remote = virtual_path(
        &Target::new("unconnected-change").unwrap(),
        b"/tmp/remote.txt",
    );
    let other = virtual_path(&Target::new("another-host").unwrap(), b"/tmp/remote.txt");
    let fails = |kind: io::ErrorKind, result: io::Result<()>| {
        assert_eq!(result.unwrap_err().kind(), kind);
    };
    let unconnected = io::ErrorKind::NotConnected;
    fails(unconnected, create_dir(&remote));
    fails(unconnected, create_dir_all(&remote));
    fails(unconnected, create_new(&remote, b"x"));
    fails(unconnected, write(&remote, "x"));
    fails(unconnected, remove_file(&remote));
    fails(unconnected, rename(&remote, &remote));
    fails(unconnected, copy(&remote, &remote).map(drop));
    fails(unconnected, copy_symlink(&remote, &remote));
    // 一个在本机、一个在远端，或分属两台主机：改名不支持，本机的文件不受影响。
    let unsupported = io::ErrorKind::Unsupported;
    fails(unsupported, rename(&local, &remote));
    fails(unsupported, rename(&remote, &local));
    fails(unsupported, rename(&remote, &other));
    // 复制可以跨主机，内容经本进程中转：这里没有连接，在访问远端的一侧时失败。
    fails(unconnected, copy(&local, &remote).map(drop));
    fails(unconnected, copy(&remote, &local).map(drop));
    fails(unconnected, copy(&remote, &other).map(drop));
    fails(unconnected, copy_symlink(&remote, &other));
    fails(unconnected, set_mode(&remote, 0o644));
    assert_eq!(std::fs::read(&local).unwrap(), b"x");
    std::fs::remove_dir_all(&dir).unwrap();
}
