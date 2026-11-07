//! `shell_command` 的测试：链接的位置、安装和卸载的边界情况、授权脚本的转义、前台和后台的判断。

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// 新建测试目录，返回目录和其中的链接位置。
fn sandbox() -> (std::path::PathBuf, Placement) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("winc-shell-{}-{serial}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("Winc.app/Contents/MacOS/winc");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "").unwrap();
    let placement = Placement {
        link: dir.join("bin/winc"),
        target,
    };
    (dir, placement)
}

#[test]
fn only_the_app_bundle_gets_a_link_on_macos() {
    let bundled = Path::new("/Applications/My Apps/winc.app/Contents/MacOS/winc");
    let placement = macos_placement(bundled).unwrap();
    assert_eq!(placement.link, Path::new("/usr/local/bin/winc"));
    assert_eq!(placement.target, bundled);
    // 开发构建不在应用包内。
    assert_eq!(macos_placement(Path::new("/repo/target/debug/winc")), None);
    assert_eq!(macos_placement(Path::new("/x/Contents/MacOS/winc")), None);
}

#[test]
fn the_appimage_links_into_the_home_directory() {
    let placement =
        appimage_placement(Path::new("/opt/winc.AppImage"), Path::new("/home/a")).unwrap();
    assert_eq!(placement.link, Path::new("/home/a/.local/bin/winc"));
    assert_eq!(placement.target, Path::new("/opt/winc.AppImage"));
    assert_eq!(
        appimage_placement(Path::new("winc"), Path::new("/home/a")),
        None
    );
}

#[test]
fn install_creates_the_directory_and_replaces_its_own_link() {
    let (dir, placement) = sandbox();
    assert_eq!(install(&placement), Outcome::Installed);
    assert_eq!(
        std::fs::read_link(&placement.link).unwrap(),
        placement.target
    );
    assert!(!placement.link.with_file_name(".winc.tmp").exists());

    // 应用移动后再次安装：指向旧位置的链接被替换。
    std::fs::remove_file(&placement.link).unwrap();
    let old = dir.join("Old.app/Contents/MacOS/winc");
    std::os::unix::fs::symlink(&old, &placement.link).unwrap();
    assert_eq!(install(&placement), Outcome::Installed);
    assert_eq!(
        std::fs::read_link(&placement.link).unwrap(),
        placement.target
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn another_program_at_the_link_is_left_alone() {
    let (dir, placement) = sandbox();
    std::fs::create_dir_all(placement.link.parent().unwrap()).unwrap();

    // 普通文件。
    std::fs::write(&placement.link, "other").unwrap();
    assert_eq!(install(&placement), Outcome::Occupied);
    assert_eq!(uninstall(&placement), Outcome::Occupied);
    assert_eq!(std::fs::read_to_string(&placement.link).unwrap(), "other");
    std::fs::remove_file(&placement.link).unwrap();

    // 指向其他存在的程序的链接。
    let other = dir.join("other-tool");
    std::fs::write(&other, "").unwrap();
    std::os::unix::fs::symlink(&other, &placement.link).unwrap();
    assert_eq!(install(&placement), Outcome::Occupied);
    assert_eq!(uninstall(&placement), Outcome::Occupied);
    assert_eq!(std::fs::read_link(&placement.link).unwrap(), other);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn uninstall_removes_only_its_own_link() {
    let (dir, placement) = sandbox();
    assert_eq!(uninstall(&placement), Outcome::NotInstalled);
    assert_eq!(install(&placement), Outcome::Installed);
    assert_eq!(uninstall(&placement), Outcome::Uninstalled);
    assert!(std::fs::symlink_metadata(&placement.link).is_err());
    assert!(placement.target.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_read_only_directory_asks_for_authorization() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, placement) = sandbox();
    let bin = placement.link.parent().unwrap();
    std::fs::create_dir_all(bin).unwrap();
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = install(&placement);
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    // root 不受目录权限限制（容器里的测试以 root 运行）。
    assert!(
        matches!(outcome, Outcome::Denied | Outcome::Installed),
        "{outcome:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_authorized_script_quotes_paths() {
    let placement = Placement {
        link: "/usr/local/bin/winc".into(),
        target: "/Applications/Bob's \"Apps\"/winc.app/Contents/MacOS/winc".into(),
    };
    let command = authorized_command(&placement, true);
    assert_eq!(
        command,
        "/bin/mkdir -p /usr/local/bin && /bin/ln -sfn \
         '/Applications/Bob'\\''s \"Apps\"/winc.app/Contents/MacOS/winc' /usr/local/bin/winc"
    );
    assert_eq!(
        authorized_command(&placement, false),
        "/bin/rm -f /usr/local/bin/winc"
    );
    // AppleScript 字符串内的反斜杠和双引号要转义。
    assert_eq!(
        authorized_script(r#"a "b" \c"#),
        r#"do shell script "a \"b\" \\c" with administrator privileges"#
    );
}

#[test]
fn waiting_and_new_instances_stay_in_the_foreground() {
    assert!(!stays_in_foreground(&Args::default()));
    let wait = Args {
        wait: true,
        ..Args::default()
    };
    assert!(stays_in_foreground(&wait));
    let stdin = Args {
        stdin: true,
        ..Args::default()
    };
    assert!(stays_in_foreground(&stdin));
    let new_instance = Args {
        new_instance: true,
        ..Args::default()
    };
    assert!(stays_in_foreground(&new_instance));
}

#[test]
fn launching_a_bundle_passes_absolute_targets() {
    use crate::cli::Target;
    let target = |path: &str, line, col| Target {
        path: path.into(),
        line,
        col,
    };
    let bundle = Path::new("/Applications/My Apps/winc.app");
    let none = open_arguments(bundle, &Args::default());
    assert_eq!(none, ["-n", "-a", "/Applications/My Apps/winc.app"]);

    let args = Args {
        targets: vec![
            target("/p", None, None),
            target("/a b.rs", Some(11), Some(4)),
            target("/c.rs", Some(0), None),
            target("relative", None, None),
        ],
        ..Args::default()
    };
    let cwd = std::env::current_dir().unwrap().join("relative");
    let out = open_arguments(bundle, &args);
    assert_eq!(&out[3..7], ["--args", "/p", "/a b.rs:12:5", "/c.rs:1"]);
    assert_eq!(out[7], cwd.as_os_str());
}

#[test]
fn only_unreadable_targets_are_reported() {
    use crate::cli::Target;
    use std::os::unix::fs::PermissionsExt;
    let (dir, placement) = sandbox();
    let locked = dir.join("locked.txt");
    std::fs::write(&locked, "").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let target = |path: std::path::PathBuf| Target {
        path,
        line: None,
        col: None,
    };
    let mut args = Args {
        targets: vec![
            target(locked.clone()),
            target(placement.target.clone()),
            target(dir.join("not-yet-created.rs")),
            target(dir.clone()),
        ],
        ..Args::default()
    };
    let readable_as_root = std::fs::File::open(&locked).is_ok();
    let failed = report_unreadable(&mut args);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    // root 不受文件权限限制（容器里的测试以 root 运行）。
    assert_eq!(failed, !readable_as_root);
    assert_eq!(args.targets.len(), if readable_as_root { 4 } else { 3 });

    // 临时打开留在前台，由后续流程报告。
    let mut wait = Args {
        wait: true,
        targets: vec![target(dir.join("x"))],
        ..Args::default()
    };
    assert!(!report_unreadable(&mut wait));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 相对路径的链接属于其他程序，目标是否存在都不改动；判断与当前工作目录无关。
#[test]
fn relative_links_belong_to_other_programs() {
    let (dir, placement) = sandbox();
    let bin = placement.link.parent().unwrap();
    std::fs::create_dir_all(bin).unwrap();
    std::os::unix::fs::symlink("../Cellar/other/1.0/bin/winc", &placement.link).unwrap();
    assert_eq!(install(&placement), Outcome::Occupied);
    assert_eq!(uninstall(&placement), Outcome::Occupied);
    assert!(std::fs::symlink_metadata(&placement.link).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 安装失败时不留下临时链接。
#[test]
fn a_failed_install_leaves_no_temporary_link() {
    let (dir, placement) = sandbox();
    let bin = placement.link.parent().unwrap();
    std::fs::create_dir_all(bin).unwrap();
    // 目录不可写时临时链接创建不出来；root 不受限时安装成功，同样没有临时链接。
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o555)).unwrap();
    let _ = install(&placement);
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(std::fs::symlink_metadata(placement.link.with_file_name(".winc.tmp")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 命名管道不会被打开：没有写入端时打开会一直阻塞。
#[test]
fn a_named_pipe_is_not_opened() {
    let (dir, _) = sandbox();
    let pipe = dir.join("pipe");
    let made = std::process::Command::new("mkfifo").arg(&pipe).status();
    if made.is_ok_and(|status| status.success()) {
        assert_eq!(unreadable(&pipe), None);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_display_is_needed_to_go_to_the_background() {
    use std::ffi::OsStr;
    assert!(!has_display(None, None));
    assert!(!has_display(Some(OsStr::new("")), None));
    assert!(has_display(Some(OsStr::new(":0")), None));
    assert!(has_display(None, Some(OsStr::new("wayland-0"))));
}

/// 只有程序位于 `APPDIR` 内才算 AppImage：内置终端里运行其他安装方式的 `winc` 时变量同样存在。
#[test]
fn inherited_appimage_variables_are_ignored() {
    use std::ffi::OsStr;
    let image = Some(OsStr::new("/opt/winc.AppImage"));
    let mount = Some(OsStr::new("/tmp/.mount_winc"));
    assert_eq!(
        appimage_of(Path::new("/tmp/.mount_winc/usr/bin/winc"), mount, image),
        Some("/opt/winc.AppImage".into())
    );
    assert_eq!(appimage_of(Path::new("/usr/bin/winc"), mount, image), None);
    assert_eq!(appimage_of(Path::new("/usr/bin/winc"), None, image), None);
}
