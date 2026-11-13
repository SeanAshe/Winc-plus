//! 文件操作：本机路径的结果与 `std::fs` 一致，虚拟路径被拒绝。

use std::time::Duration;

use super::*;
use crate::{Target, virtual_path};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-backend-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn local_files_report_the_same_information_as_the_standard_library() {
    let dir = scratch("metadata");
    let file = dir.join("note.txt");
    std::fs::write(&file, "héllo").unwrap();

    let found = metadata(&file).unwrap();
    let expected = std::fs::metadata(&file).unwrap();
    assert_eq!((found.kind, found.len), (Kind::File, 6));
    assert_eq!(found.modified, expected.modified().ok().map(Stamp::from));
    assert!(found.is_file() && !found.is_dir() && !found.readonly);
    assert!(metadata(&dir).unwrap().is_dir());
    assert_eq!(read(&file).unwrap(), "héllo".as_bytes());
    assert_eq!(read_to_string(&file).unwrap(), "héllo");
    assert!(is_file(&file) && !is_dir(&file) && exists(&file));
    assert!(is_dir(&dir) && !is_file(&dir));

    let missing = dir.join("missing");
    assert_eq!(
        metadata(&missing).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(!exists(&missing) && !is_file(&missing) && !is_dir(&missing));

    // 路径上没有该项、路径中间的一段是文件：都表示不存在；其他错误不表示不存在。
    assert!(is_absent(&metadata(&missing).unwrap_err()));
    assert!(is_absent(&metadata(file.join("below")).unwrap_err()));
    assert!(!is_absent(&io::Error::from(
        io::ErrorKind::PermissionDenied
    )));
    assert!(!is_absent(&io::Error::from(io::ErrorKind::Unsupported)));

    // 批量取信息：结果与路径的顺序一一对应，各项互不影响。
    let found = metadata_many(&[file.clone(), missing, dir.clone()]);
    assert_eq!(found.len(), 3);
    assert!(found[0].as_ref().is_ok_and(Metadata::is_file));
    assert!(found[1].as_ref().is_err_and(is_absent));
    assert!(found[2].as_ref().is_ok_and(Metadata::is_dir));
    assert!(metadata_many::<PathBuf>(&[]).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn symbolic_links_and_unix_attributes_are_reported() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let dir = scratch("unix");
    let file = dir.join("script.sh");
    let link = dir.join("link");
    std::fs::write(&file, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o750)).unwrap();
    std::os::unix::fs::symlink(&file, &link).unwrap();

    let expected = std::fs::metadata(&file).unwrap();
    let unix = metadata(&file).unwrap().unix.unwrap();
    assert_eq!(unix.mode & 0o777, 0o750);
    assert_eq!((unix.uid, unix.nlink), (expected.uid(), 1));
    assert_eq!((unix.dev, unix.ino), (expected.dev(), expected.ino()));

    // 跟随链接时得到目标的信息，不跟随时得到链接本身。
    assert!(metadata(&link).unwrap().is_file());
    assert!(symlink_metadata(&link).unwrap().is_symlink());
    assert!(is_symlink(&link) && !is_symlink(&file));
    assert_eq!(read_link(&link).unwrap(), file);
    // 指向不存在目标的链接：本身存在，跟随后不存在。
    std::fs::remove_file(&file).unwrap();
    assert!(is_symlink(&link) && !exists(&link));

    let readonly = dir.join("readonly");
    std::fs::write(&readonly, "").unwrap();
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o444)).unwrap();
    assert!(metadata(&readonly).unwrap().readonly);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stamps_keep_the_nanoseconds_on_both_sides_of_the_epoch() {
    // 纳秒部分取 100 的整数倍：Windows 的系统时间以 100 纳秒为单位，更细的部分存不下。
    let after = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_700);
    assert_eq!(
        Stamp::from(after),
        Stamp {
            secs: 1_700_000_000,
            nanos: 123_456_700
        }
    );
    assert_eq!(Stamp::from(UNIX_EPOCH), Stamp { secs: 0, nanos: 0 });
    // 纪元之前 1.25 秒：秒数向下取整为 -2，纳秒部分为 0.75 秒。
    let before = UNIX_EPOCH - Duration::new(1, 250_000_000);
    assert_eq!(
        Stamp::from(before),
        Stamp {
            secs: -2,
            nanos: 750_000_000
        }
    );
    assert_eq!(
        Stamp::from(UNIX_EPOCH - Duration::from_secs(3)),
        Stamp { secs: -3, nanos: 0 }
    );
    assert!(Stamp::from(before) < Stamp::from(UNIX_EPOCH));
}

#[test]
fn virtual_paths_need_a_connection_and_never_touch_the_local_file_system() {
    let file = virtual_path(&Target::new("unconnected-fs").unwrap(), b"/etc/hosts");
    let unconnected = |result: io::Result<()>| {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotConnected);
    };
    unconnected(metadata(&file).map(drop));
    unconnected(symlink_metadata(&file).map(drop));
    unconnected(read(&file).map(drop));
    unconnected(read_to_string(&file).map(drop));
    unconnected(read_link(&file).map(drop));
    // 取不到信息时各项判断为否：无法确定不等于存在。
    assert!(!exists(&file) && !is_file(&file) && !is_dir(&file) && !is_symlink(&file));
    for result in metadata_many(&[&file]) {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotConnected);
    }
}
