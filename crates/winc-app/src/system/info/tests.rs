//! 版本信息的测试：Windows 的命名、os-release 的解析和桌面环境的说明。

use super::*;

#[test]
fn windows_is_named_by_its_build_number() {
    assert_eq!(
        windows_name(22631, Some("23H2"), Some(4460)),
        "Windows 11 23H2 (build 22631.4460)"
    );
    assert_eq!(windows_name(19045, None, None), "Windows 10 (build 19045)");
}

#[test]
fn os_release_prefers_the_pretty_name_and_unquotes_values() {
    let ubuntu = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n";
    assert_eq!(
        os_release_name(ubuntu).as_deref(),
        Some("Ubuntu 24.04.1 LTS")
    );
    // 没有 `PRETTY_NAME` 时由名称和版本组成；单引号和双引号内的转义都按规范处理。
    assert_eq!(
        os_release_name("NAME='Arch Linux'\nVERSION_ID=rolling\n").as_deref(),
        Some("Arch Linux rolling")
    );
    assert_eq!(
        os_release_name(r#"PRETTY_NAME="A \"quoted\" \$name""#).as_deref(),
        Some(r#"A "quoted" $name"#)
    );
    // 注释、空值和不成对的引号不算。
    assert_eq!(os_release_name("#PRETTY_NAME=x\nNAME=\"\"\n"), None);
    assert_eq!(os_release_name("PRETTY_NAME=\"broken\n"), None);
}

#[test]
fn the_desktop_names_the_environment_and_the_display_protocol() {
    assert_eq!(
        desktop(Some("ubuntu:GNOME"), Some("Wayland")).as_deref(),
        Some("GNOME, Wayland")
    );
    assert_eq!(desktop(Some("KDE"), None).as_deref(), Some("KDE"));
    assert_eq!(desktop(None, Some("X11")).as_deref(), Some("X11"));
    assert_eq!(desktop(Some(""), None), None);
}

/// 第一行是版本和架构，与「关于」页显示的相同；第二行是系统。
#[test]
fn the_report_starts_with_the_version_shown_in_about() {
    let report = report(None);
    let mut lines = report.lines();
    let first = format!("Winc {}", crate::layout::version_label());
    assert_eq!(lines.next(), Some(first.as_str()));
    let system = lines.next().unwrap();
    if cfg!(target_os = "macos") {
        assert!(
            system.starts_with("macOS ") && !system.contains("unknown"),
            "{system}"
        );
    }
    assert!(!system.is_empty());
}
