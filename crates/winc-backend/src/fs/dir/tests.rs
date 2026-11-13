//! 列目录：各项的名字、种类和路径。

use std::path::PathBuf;

use super::*;
use crate::{Target, virtual_path};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-backend-dir-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn entries_carry_their_name_kind_and_path() {
    let dir = scratch("list");
    std::fs::write(dir.join("a.txt"), "").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("sub"), dir.join("link")).unwrap();

    let mut found: Vec<(String, Kind, PathBuf)> = read_dir(&dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.name.to_string_lossy().into_owned();
            (name, entry.kind, entry.path())
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let item = |name: &str, kind| (name.to_owned(), kind, dir.join(name));
    // 符号链接不跟随：指向目录的链接仍是链接。
    #[cfg(unix)]
    let link = Some(item("link", Kind::Symlink));
    #[cfg(not(unix))]
    let link = None;
    let expected: Vec<_> = [
        Some(item("a.txt", Kind::File)),
        link,
        Some(item("sub", Kind::Dir)),
    ]
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(found, expected);

    assert_eq!(read_dir(dir.join("sub")).unwrap().count(), 0);
    assert!(read_dir(dir.join("missing")).is_err());
    assert!(read_dir(dir.join("a.txt")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn virtual_paths_need_a_connection() {
    let dir = virtual_path(&Target::new("unconnected-dir").unwrap(), b"/etc");
    assert_eq!(
        read_dir(&dir).err().unwrap().kind(),
        io::ErrorKind::NotConnected
    );
}
