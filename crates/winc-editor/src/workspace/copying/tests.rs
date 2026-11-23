use std::fs;
use std::io;

use winc_i18n::t;

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

/// 下载目录时跳过特殊文件和无法复制的符号链接，其余内容照常复制；符号链接以复制目标内容的方式
/// 处理，与 Windows 上的下载相同。
#[cfg(unix)]
#[test]
fn downloading_skips_special_files_and_links_that_cannot_be_copied() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    // 套接字路径受 `sun_path` 长度限制（约 104 字节），不使用 `scratch` 的长路径。
    let dir = std::env::temp_dir().join(format!("winc-skip-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let root = dir.join("project");
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(root.join("main.rs"), "ok").unwrap();
    let _listener = UnixListener::bind(root.join("data/app.sock")).unwrap();
    symlink("../main.rs", root.join("data/link")).unwrap();
    symlink("missing", root.join("dangling")).unwrap();
    symlink("looped", root.join("looped")).unwrap();
    let links: Links = |from, to| winc_backend::fs::copy(from, to).map(drop);
    let mut skips = Skips::new(&root);

    download_item(
        &root,
        &dir.join("out"),
        links,
        &Copying::default(),
        &mut skips,
    )
    .unwrap();

    let mut skipped: Vec<_> = skips
        .list
        .iter()
        .map(|s| (s.name.as_str(), s.reason.as_str()))
        .collect();
    skipped.sort_unstable();
    assert_eq!(
        skipped,
        [
            (
                "project/dangling",
                t!("editor-download-skipped-link-missing")
            ),
            (
                "project/data/app.sock",
                t!("editor-download-skipped-special")
            ),
            ("project/looped", t!("editor-download-skipped-link-loop")),
        ]
    );
    assert_eq!(fs::read_to_string(dir.join("out/main.rs")).unwrap(), "ok");
    assert_eq!(fs::read_to_string(dir.join("out/data/link")).unwrap(), "ok");
    let _ = fs::remove_dir_all(&dir);
}

/// 子项出错时记下出错的子项，此前跳过的项仍保留；下载项本身出错时记下下载项。
#[cfg(unix)]
#[test]
fn a_failure_names_the_child_that_failed() {
    use std::os::unix::fs::symlink;

    let dir = scratch("download-failed-child");
    let root = dir.join("project");
    fs::create_dir_all(root.join("a")).unwrap();
    symlink("missing", root.join("a/dangling")).unwrap();
    symlink("x", root.join("a/z-locked")).unwrap();
    fs::write(root.join("a/m.txt"), "m").unwrap();
    let links: Links = |from, to| match from.file_name().and_then(|n| n.to_str()) {
        Some("z-locked") => Err(io::ErrorKind::PermissionDenied.into()),
        _ => winc_backend::fs::copy(from, to).map(drop),
    };
    let mut skips = Skips::new(&root);

    let error = download_item(
        &root,
        &dir.join("out"),
        links,
        &Copying::default(),
        &mut skips,
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let names: Vec<_> = skips.list.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.iter().all(|name| *name == "project/a/dangling"),
        "{names:?}"
    );
    assert_eq!(skips.failed_name(), "project/a/z-locked");

    let mut skips = Skips::new(&dir.join("gone"));
    download_item(
        &dir.join("gone"),
        &dir.join("out2"),
        links,
        &Copying::default(),
        &mut skips,
    )
    .unwrap_err();
    assert_eq!(skips.failed_name(), "gone");
}

#[test]
fn link_errors_that_mean_the_link_cannot_be_copied() {
    let reason = |error: io::Error| link_skip_reason(&error);
    let missing = t!("editor-download-skipped-link-missing").to_owned();
    assert_eq!(reason(io::ErrorKind::NotFound.into()), Some(missing));
    let folder = t!("editor-download-skipped-link-folder").to_owned();
    assert_eq!(reason(io::ErrorKind::IsADirectory.into()), Some(folder));
    let special = t!("editor-download-skipped-special").to_owned();
    assert_eq!(reason(io::ErrorKind::InvalidInput.into()), Some(special));
    // 远端传回的 `ELOOP` 只有描述，glibc 与 musl 的写法不同。
    let looped = Some(t!("editor-download-skipped-link-loop").to_owned());
    let linux = io::Error::other("Too many levels of symbolic links (os error 40)");
    assert_eq!(reason(linux), looped);
    assert_eq!(
        reason(io::Error::other("Symbolic link loop (os error 40)")),
        looped
    );
    assert_eq!(reason(io::ErrorKind::PermissionDenied.into()), None);
    assert_eq!(reason(io::ErrorKind::BrokenPipe.into()), None);
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
