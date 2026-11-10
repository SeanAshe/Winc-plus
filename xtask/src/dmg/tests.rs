//! `dmg` 的测试：`Info.plist` 中的「在 Winc 中打开」服务声明，以及各语言的服务名。

use super::*;

#[test]
fn info_plist_declares_the_open_service_for_text_files_and_folders() {
    let plist = info_plist("1.2.3", &sparkle::UpdateConfig::default(), "arm64");
    let start = plist.find("<key>NSServices</key>").expect("声明了服务");
    let service = &plist[start..];
    let service = &service[..service.find("</array>\n\t<key>").unwrap()];
    for want in [
        "<string>Open in Winc</string>",
        "<key>NSMessage</key>\n\t\t\t<string>openInWinc</string>",
        // 声明了 `NSRequiredContext` 的服务安装后即启用。
        "<key>NSRequiredContext</key>\n\t\t\t<dict/>",
        "<string>public.text</string>",
        "<string>public.folder</string>",
    ] {
        assert!(service.contains(want), "{want}\n{service}");
    }
    // 只针对文本文件和文件夹，不出现在图片、压缩包等其他文件上。
    assert!(!service.contains("public.data") && !service.contains("public.item"));

    // 整个文件是合法的属性列表。
    if cfg!(target_os = "macos") {
        let file = std::env::temp_dir().join(format!("winc-info-{}.plist", std::process::id()));
        std::fs::write(&file, &plist).unwrap();
        let lint = Command::new("plutil")
            .arg("-lint")
            .arg(&file)
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&file);
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stdout)
        );
    }
}

#[test]
fn services_menu_titles_are_written_for_each_language() {
    let dir = std::env::temp_dir().join(format!("winc-services-menu-{}", std::process::id()));
    write_services_menu(&dir).unwrap();
    let read = |language: &str| {
        std::fs::read_to_string(dir.join(format!("{language}.lproj/ServicesMenu.strings"))).unwrap()
    };
    assert_eq!(read("en"), "\"Open in Winc\" = \"Open in Winc\";\n");
    assert_eq!(read("zh-Hans"), "\"Open in Winc\" = \"在 Winc 中打开\";\n");
    let _ = std::fs::remove_dir_all(&dir);
}
