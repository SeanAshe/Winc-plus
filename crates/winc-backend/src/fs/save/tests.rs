//! 保存：内容与返回的文件信息、已有文件属性的保留、退回原地写入的条件，以及失败时不留痕迹。

use std::path::PathBuf;

use super::*;
use crate::{Target, virtual_path};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-backend-save-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn in_memory() -> SaveOptions<'static> {
    SaveOptions {
        in_memory: true,
        ..SaveOptions::default()
    }
}

/// 目录中的文件名，用于确认没有留下临时文件。
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn new_and_existing_files_are_written_and_described() {
    let dir = scratch("basic");
    // 所在的目录不存在时先创建。
    let file = dir.join("nested/deep/note.txt");
    let written = save(&file, &in_memory(), |out| out.write_all(b"one")).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"one");
    assert_eq!(written, crate::fs::metadata(&file).unwrap());

    let options = SaveOptions {
        bom: true,
        ..in_memory()
    };
    let written = save(&file, &options, |out| out.write_all("二".as_bytes())).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"\xEF\xBB\xBF\xE4\xBA\x8C");
    assert_eq!(written.len, 6);
    assert_eq!(written, crate::fs::metadata(&file).unwrap());
    assert_eq!(names(file.parent().unwrap()), ["note.txt"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failed_write_leaves_the_file_and_the_folder_untouched() {
    let dir = scratch("failed");
    let file = dir.join("note.txt");
    std::fs::write(&file, "old").unwrap();
    let error = save(&file, &SaveOptions::default(), |out| {
        out.write_all(b"partial")?;
        Err(io::Error::other("stop"))
    })
    .unwrap_err();
    assert!(matches!(error, SaveError::Io(_)), "{error}");
    assert_eq!(std::fs::read(&file).unwrap(), b"old");
    assert_eq!(names(&dir), ["note.txt"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn permissions_are_kept_and_read_only_files_are_refused() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("mode");
    let file = dir.join("run.sh");
    std::fs::write(&file, "old").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o750)).unwrap();
    let written = save(&file, &in_memory(), |out| out.write_all(b"new")).unwrap();
    assert_eq!(written.unix.unwrap().mode & 0o777, 0o750);
    assert_eq!(std::fs::read(&file).unwrap(), b"new");

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    let error = save(&file, &in_memory(), |out| out.write_all(b"x")).unwrap_err();
    assert!(matches!(error, SaveError::ReadOnly), "{error}");
    assert_eq!(std::fs::read(&file).unwrap(), b"new");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 有多个硬链接的文件：改名只替换一个名字。内存中的内容原地写入，各个名字都看到新内容；
/// 无法原地写入的内容报错，文件不变。
#[cfg(unix)]
#[test]
fn hard_linked_files_are_written_in_place_or_refused() {
    let dir = scratch("links");
    let file = dir.join("a.txt");
    let other = dir.join("b.txt");
    std::fs::write(&file, "old").unwrap();
    std::fs::hard_link(&file, &other).unwrap();

    let written = save(&file, &in_memory(), |out| out.write_all(b"new")).unwrap();
    assert_eq!(std::fs::read(&other).unwrap(), b"new");
    assert_eq!(written.unix.unwrap().nlink, 2);

    let error = save(&file, &SaveOptions::default(), |out| out.write_all(b"x")).unwrap_err();
    assert!(matches!(error, SaveError::IdentityWouldChange), "{error}");
    assert_eq!(std::fs::read(&file).unwrap(), b"new");
    assert_eq!(names(&dir), ["a.txt", "b.txt"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_in_place_preference_and_the_callback_apply_to_existing_files_only() {
    let dir = scratch("options");
    let file = dir.join("note.txt");
    let called = std::cell::Cell::new(0);
    let callback = |target: &Path, temp: &Path| {
        assert!(target.ends_with("note.txt") && temp.exists() && temp != target);
        called.set(called.get() + 1);
        Ok(())
    };
    let options = SaveOptions {
        before_replace: Some(&callback),
        ..SaveOptions::default()
    };
    // 新文件没有可复制的属性，不调用回调。
    save(&file, &options, |out| out.write_all(b"one")).unwrap();
    assert_eq!(called.get(), 0);
    save(&file, &options, |out| out.write_all(b"two")).unwrap();
    assert_eq!(called.get(), 1);

    // 回调失败时放弃保存，不留临时文件。
    let failing = |_: &Path, _: &Path| Err(io::Error::other("stop"));
    let options = SaveOptions {
        before_replace: Some(&failing),
        ..SaveOptions::default()
    };
    assert!(save(&file, &options, |out| out.write_all(b"three")).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"two");
    assert_eq!(names(&dir), ["note.txt"]);

    // 原地写入不经过临时文件，也不调用回调。
    let options = SaveOptions {
        prefer_in_place: true,
        before_replace: Some(&callback),
        ..in_memory()
    };
    save(&file, &options, |out| out.write_all(b"four")).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"four");
    assert_eq!(called.get(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn virtual_paths_need_a_connection() {
    let file = virtual_path(&Target::new("unconnected-save").unwrap(), b"/tmp/note.txt");
    let error = save(&file, &in_memory(), |out| out.write_all(b"x")).unwrap_err();
    let SaveError::Io(error) = error else {
        panic!("{error}");
    };
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
}
