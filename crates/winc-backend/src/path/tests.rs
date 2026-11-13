//! 虚拟路径的换算，以及 `Path` 操作在虚拟路径上的行为。

use super::*;

fn target() -> Target {
    Target::new("alice@devbox").unwrap()
}

fn remote(path: &Path) -> String {
    String::from_utf8(locate(path).unwrap().path).unwrap()
}

#[test]
fn remote_paths_survive_a_round_trip() {
    let paths: [&[u8]; 6] = [
        b"/",
        b"/home/alice/project/src/main.rs",
        "/srv/文档/报告 v2.md".as_bytes(),
        br"/tmp/back\slash",
        b"/tmp/100%/a:b*c?",
        b"/tmp/.hidden/..data",
    ];
    for path in paths {
        let located = locate(&virtual_path(&target(), path)).unwrap();
        assert_eq!(located.target, target());
        assert_eq!(located.path, path, "{}", String::from_utf8_lossy(path));
    }
    // 空段和 `.` 忽略，相对路径按相对于根目录处理。
    assert_eq!(remote(&virtual_path(&target(), b"//a/./b/")), "/a/b");
    assert_eq!(remote(&virtual_path(&target(), b"a/b")), "/a/b");
}

#[cfg(unix)]
#[test]
fn names_that_are_not_utf8_are_kept_as_bytes() {
    let path = b"/data/caf\xE9/\xFF\xFE.bin";
    let located = locate(&virtual_path(&target(), path)).unwrap();
    assert_eq!(located.path, path);
}

#[test]
fn path_operations_behave_as_on_local_paths() {
    let root = virtual_path(&target(), b"/home/alice/project");
    let file = root.join("src").join("main.rs");
    assert_eq!(remote(&file), "/home/alice/project/src/main.rs");
    assert_eq!(file.file_name().unwrap(), "main.rs");
    assert_eq!(file.extension().unwrap(), "rs");
    assert_eq!(remote(file.parent().unwrap()), "/home/alice/project/src");
    assert!(file.starts_with(&root));
    assert_eq!(
        file.strip_prefix(&root).unwrap(),
        Path::new("src").join("main.rs")
    );
    assert!(file.is_absolute());

    // 不同连接、本机路径与虚拟路径之间互不为前缀。
    let other = virtual_path(&Target::new("bob@devbox").unwrap(), b"/home/alice/project");
    assert!(!other.starts_with(&root) && !root.starts_with(&other));
    assert!(!Path::new("/home/alice/project").starts_with(&root));
}

#[test]
fn parent_components_stop_at_the_remote_root() {
    let root = virtual_path(&target(), b"/");
    assert_eq!(remote(&root.join("a").join("..").join("b")), "/b");
    assert_eq!(remote(&root.join("..").join("..").join("etc")), "/etc");
    assert_eq!(remote(&root.join("a").join(".").join("b")), "/a/b");
    assert_eq!(locate(&root.join("..")).unwrap().target, target());
}

#[test]
fn local_paths_are_not_virtual() {
    for path in [
        "",
        "relative/file",
        "/home/alice/project",
        "/dev/null",
        "/dev/null/winc-remote",
        "/dev/null/winc-remote:/missing-target",
        "/dev/null/winc-remote:bad target/file",
        "/dev/null/winc-remote:-oProxyCommand/file",
        "/winc-remote:alice@devbox/file",
        "/dev/zero/winc-remote:alice@devbox/file",
        r"C:\Users\alice",
        r"\\server\share\file",
        r"\\.\winc-remote--option\file",
    ] {
        let path = Path::new(path);
        assert_eq!(locate(path), None, "{}", path.display());
        assert!(!is_remote(path));
        assert!(ensure_local(path).is_ok());
    }
}

#[test]
fn virtual_paths_are_refused_by_local_only_operations() {
    let file = virtual_path(&target(), b"/etc/hosts");
    assert!(is_remote(&file));
    assert_eq!(
        ensure_local(&file).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(display(&file).unwrap(), "alice@devbox:/etc/hosts");
    assert_eq!(display(Path::new("/etc/hosts")), None);
}

/// 虚拟路径误传给本机文件系统时立即失败：虚拟根在本机不存在，也无法创建（以 root 运行时
/// 同样如此），不产生网络访问。
#[test]
fn virtual_paths_fail_at_once_on_the_local_file_system() {
    let file = virtual_path(&target(), b"/etc/hosts");
    assert!(std::fs::metadata(&file).is_err());
    assert!(std::fs::read(&file).is_err());
    assert!(std::fs::read_dir(file.parent().unwrap()).is_err());
    assert!(std::fs::write(&file, b"x").is_err());
    assert!(std::fs::create_dir_all(file.parent().unwrap()).is_err());
    assert!(std::fs::create_dir(virtual_path(&target(), b"/")).is_err());
}

#[cfg(unix)]
#[test]
fn the_virtual_root_lies_under_a_device_file() {
    let file = virtual_path(&target(), b"/etc/hosts");
    assert_eq!(
        file,
        Path::new("/dev/null/winc-remote:alice@devbox/etc/hosts")
    );
    // 「不是目录」而非「不存在」：该错误与权限无关。
    let error = std::fs::metadata(&file).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
}

#[cfg(windows)]
#[test]
fn the_virtual_root_is_a_device_namespace_prefix() {
    use std::path::Prefix;

    let file = virtual_path(&target(), br"/tmp/back\slash");
    assert_eq!(
        file.as_os_str(),
        r"\\.\winc-remote-alice@devbox\tmp\back%5Cslash"
    );
    let mut components = file.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        panic!("虚拟路径以前缀开头");
    };
    assert!(matches!(prefix.kind(), Prefix::DeviceNS(_)));
    assert_eq!(components.next(), Some(Component::RootDir));
    // 普通的设备路径不是虚拟路径。
    assert_eq!(locate(Path::new(r"\\.\COM1")), None);
    assert_eq!(locate(Path::new(r"\\.\PhysicalDrive0\x")), None);
}

#[test]
fn native_and_relative_forms_use_the_remote_spelling() {
    let root = virtual_path(&target(), b"/home/alice/project");
    let file = root.join("src").join("main.rs");
    assert_eq!(native_lossy(&file), "/home/alice/project/src/main.rs");
    assert_eq!(relative_lossy(&file, &root).unwrap(), "src/main.rs");
    assert_eq!(relative_lossy(&root, &root).unwrap(), "");
    // 根目录之下、只是名字的前缀相同、不同连接、本机与远端之间。
    let top = virtual_path(&target(), b"/");
    assert_eq!(
        relative_lossy(&file, &top).unwrap(),
        "home/alice/project/src/main.rs"
    );
    let sibling = virtual_path(&target(), b"/home/alice/project-old/a");
    assert_eq!(relative_lossy(&sibling, &root), None);
    let elsewhere = virtual_path(
        &Target::new("bob@devbox").unwrap(),
        b"/home/alice/project/a",
    );
    assert_eq!(relative_lossy(&elsewhere, &root), None);
    assert_eq!(
        relative_lossy(Path::new("/home/alice/project/a"), &root),
        None
    );
    assert_eq!(
        relative_lossy(&file, Path::new("/home/alice/project")),
        None
    );

    // 含 `\`、`%` 的名字在各平台的虚拟路径中写法不同，交给用户时都是远端的原样。
    let odd = virtual_path(&target(), br"/tmp/a\b/100%.txt");
    assert_eq!(native_lossy(&odd), r"/tmp/a\b/100%.txt");
    assert_eq!(locate(&odd).unwrap().name(), b"100%.txt");
    assert_eq!(locate(&top).unwrap().name(), b"");

    // 本机路径原样返回。
    let local = Path::new("relative").join("file.txt");
    assert_eq!(native_lossy(&local), local.to_string_lossy());
    assert_eq!(
        relative_lossy(&local, Path::new("relative")).unwrap(),
        "file.txt"
    );
}
