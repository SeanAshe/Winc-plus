//! `dmg` 的测试：`Info.plist` 中的文档类型、「在 Winc 中打开」服务和 `winc` 协议的声明，以及
//! 各语言的服务名。

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

/// `Info.plist` 声明 `winc` 协议，`winc://` 链接交给 Winc 打开。
#[test]
fn info_plist_declares_the_winc_url_scheme() {
    let plist = info_plist("1.2.3", &sparkle::UpdateConfig::default(), "arm64");
    let start = plist
        .find("<key>CFBundleURLTypes</key>")
        .expect("声明了协议");
    let types = &plist[start..];
    let types = &types[..types.find("</array>\n\t<key>").unwrap()];
    assert!(types.contains("<string>winc</string>"), "{types}");
    assert!(
        types.contains(&format!("<string>{BUNDLE_ID}</string>")),
        "{types}"
    );
}

/// 回归：只声明任意文件（`public.data`）时，访达的「打开方式」不列出 Winc。
#[test]
fn info_plist_declares_the_types_winc_opens() {
    let plist = info_plist("1.2.3", &sparkle::UpdateConfig::default(), "arm64");
    let start = plist.find("<key>CFBundleDocumentTypes</key>").unwrap();
    let types = &plist[start..];
    let types = &types[..types.find("\t</array>\n\t<key>").unwrap()];
    for kind in ["public.plain-text", "com.adobe.pdf", "public.folder"] {
        assert!(
            types.contains(&format!("<string>{kind}</string>")),
            "{kind}"
        );
    }
    // 扩展名同 Windows 安装包登记的类型，PDF、EPUB 在这里按系统类型声明。
    let mut listed: Vec<String> = (TEXT_EXTENSIONS.iter())
        .map(|kind| kind.to_uppercase())
        .collect();
    listed.extend(["EPUB".to_owned(), "PDF".to_owned()]);
    listed.sort();
    assert_eq!(listed, crate::file_types::nsis_types());
    for kind in TEXT_EXTENSIONS {
        assert!(
            types.contains(&format!("\t<string>{kind}</string>")),
            "{kind}"
        );
    }
    // 只有按扩展名的一项可成为默认的打开方式，Winc 的项目文件归 Winc 所有，其余都是候选。
    assert_eq!(types.matches("<string>Default</string>").count(), 1);
    assert_eq!(types.matches("<string>Owner</string>").count(), 1);
    assert_eq!(types.matches("<string>Alternate</string>").count(), 4);
    assert!(types.contains("<string>com.deeix.winc.project</string>"));
    let exported = &plist[plist.find("<key>UTExportedTypeDeclarations</key>").unwrap()..];
    assert!(exported.contains("<string>winc-project</string>"));
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

/// 项目文件的文档图标合成为 `.icns`，`Info.plist` 中这一类型和导出的类型都指向它。
#[test]
#[cfg(target_os = "macos")]
fn the_project_document_icon_is_built() {
    let work = std::env::temp_dir().join(format!("winc-project-icon-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let icns = work.join(format!("{PROJECT_ICON}.icns"));
    project_icon(&work, &icns).unwrap();
    assert!(std::fs::metadata(&icns).unwrap().len() > 1000);
    let _ = std::fs::remove_dir_all(&work);
    let plist = info_plist("1.2.3", &sparkle::UpdateConfig::default(), "arm64");
    for key in ["CFBundleTypeIconFile", "UTTypeIconFile"] {
        let entry = format!("<key>{key}</key>\n\t\t\t<string>{PROJECT_ICON}</string>");
        assert!(plist.contains(&entry), "{key}");
    }
}
