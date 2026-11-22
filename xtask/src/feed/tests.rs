//! `feed` 的测试：更新包文件名与 `sign_update` 输出的解析，以及 appcast 的内容。

use super::*;

#[test]
fn package_names_give_version_and_architecture() {
    let linux = Platform::Linux.suffix();
    let windows = Platform::Windows.suffix();
    assert_eq!(
        parse_name("winc-0.1.369-x86_64.AppImage", linux),
        Some(("0.1.369".into(), "x86_64".into()))
    );
    assert_eq!(
        parse_name("winc-1.0.0-aarch64.AppImage", linux),
        Some(("1.0.0".into(), "aarch64".into()))
    );
    assert_eq!(
        parse_name("winc-0.1.369-x86_64-setup.exe", windows),
        Some(("0.1.369".into(), "x86_64".into()))
    );
    for bad in [
        "winc-0.1.369.AppImage",
        "winc-0.1-x86.zip",
        "other-1-x86_64.AppImage",
        "winc-0.1.369-x86_64-setup.exe",
    ] {
        assert_eq!(parse_name(bad, linux), None, "{bad}");
    }
    assert_eq!(parse_name("winc-0.1.369-x86_64.exe", windows), None);
    assert_eq!(
        parse_name("Winc-1.2.3-beta.4-linux-x64.AppImage", linux),
        Some(("1.2.3-beta.4".into(), "x86_64".into()))
    );
    assert_eq!(
        parse_name("Winc-1.2.3-beta.4-windows-x64.msi", ".msi"),
        Some(("1.2.3-beta.4".into(), "x86_64".into()))
    );
    assert_eq!(
        parse_name("Winc-1.2.3-linux-arm64.AppImage", linux),
        Some(("1.2.3".into(), "aarch64".into()))
    );
}

#[test]
fn each_linux_architecture_has_a_feed_of_its_own() {
    assert_eq!(Platform::Linux.feed_file("x86_64"), "appcast-linux.xml");
    assert_eq!(
        Platform::Linux.feed_file("aarch64"),
        "appcast-linux-arm64.xml"
    );
    assert_eq!(Platform::Windows.feed_file("x86_64"), "appcast-windows.xml");
}

#[test]
fn sign_update_output_is_read() {
    let output = "sparkle:edSignature=\"AbC+/=\" length=\"12345\"\n";
    assert_eq!(parse_signature(output), Some(("AbC+/=".into(), 12345)));
    assert_eq!(parse_signature("garbage"), None);
}

#[test]
fn the_feed_lists_packages_newest_first() {
    let package = |version: &str, arch: &str| Package {
        version: version.into(),
        arch: arch.into(),
        name: format!("winc-{version}-{arch}.AppImage"),
        length: 42,
        signature: "c2ln".into(),
    };
    let feed = feed(
        Platform::Linux,
        &[package("0.1.9", "x86_64"), package("0.1.10", "aarch64")],
        "https://dl.example.com/winc",
    );
    let newer = feed.find("<title>0.1.10</title>").unwrap();
    let older = feed.find("<title>0.1.9</title>").unwrap();
    assert!(newer < older, "{feed}");
    assert!(feed.contains(
        "url=\"https://dl.example.com/winc/winc-0.1.10-aarch64.AppImage\" length=\"42\""
    ));
    assert!(feed.contains("sparkle:os=\"linux\" arch=\"aarch64\" sparkle:edSignature=\"c2ln\""));
    assert!(feed.ends_with("</rss>\n"));
    let windows = feed_for_windows();
    assert!(windows.contains("sparkle:os=\"windows\""), "{windows}");
}

/// 返回只含一个 Windows 安装包的 appcast，用于检查 `sparkle:os` 属性。
fn feed_for_windows() -> String {
    let package = Package {
        version: "0.1.9".into(),
        arch: "x86_64".into(),
        name: "winc-0.1.9-x86_64-setup.exe".into(),
        length: 42,
        signature: "c2ln".into(),
    };
    feed(Platform::Windows, &[package], "https://dl.example.com/winc")
}
