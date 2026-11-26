//! `font_list` 的测试：已安装字族的排序与等宽判断、按宽松写法查找，以及读取字体文件的名字表。

use super::*;
#[cfg(target_os = "macos")]
use crate::text::font_path::FontSpec;

#[test]
fn families_are_sorted_and_deduplicated_ignoring_how_names_are_written() {
    let family = |name: &str| FontFamily {
        name: name.to_owned(),
        monospace: false,
        file: FontFile::first("a.ttf"),
    };
    let names: Vec<String> = sorted(vec![
        family("Zed"),
        family("apple"),
        family("Maple Mono"),
        family("MapleMono"),
    ])
    .into_iter()
    .map(|family| family.name)
    .collect();
    assert_eq!(names, ["apple", "Maple Mono", "Zed"]);
}

#[test]
fn an_unknown_or_blank_name_finds_no_family() {
    for name in ["", " - _", "winc 不存在的字体 42"] {
        assert_eq!(find_family(name), None, "{name:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn installed_families_tell_monospace_from_proportional() {
    let fonts = installed_fonts();
    let of = |name: &str| fonts.iter().find(|family| family.name == name).unwrap();
    assert!(of("Menlo").monospace);
    assert!(!of("Helvetica").monospace);
    assert!(of("SF Mono").monospace);
    assert!(fonts.iter().all(|family| !family.name.starts_with('.')));
}

#[cfg(target_os = "macos")]
#[test]
fn a_family_name_written_without_spaces_still_resolves() {
    for name in ["CourierNew", "couriernew", "courier-new"] {
        let found = FontSpec::Name(name.into()).resolve();
        assert_eq!(
            found,
            FontSpec::Name("Courier New".into()).resolve(),
            "{name}"
        );
        assert!(FontSpec::Name(name.into()).check().is_ok(), "{name}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn scanning_font_files_reads_families_and_prefers_the_regular_face() {
    let paths = ["/System/Library/Fonts/Menlo.ttc", "/etc/hosts"].map(std::path::PathBuf::from);
    let families = scan(paths.iter());
    assert_eq!(families.len(), 1);
    assert_eq!(families[0].name, "Menlo");
    assert!(families[0].monospace);
    assert_eq!(
        families[0].file.postscript.as_deref(),
        Some("Menlo-Regular")
    );
}

/// Linux 上字体列表来自 fontconfig，因此包含装在固定字体目录之外的字体。
#[cfg(target_os = "linux")]
#[test]
fn installed_families_include_fontconfig_directories() {
    use crate::text::fontconfig::test_fonts::{self, Isolated};

    let Some(_isolated) = Isolated::new() else {
        return;
    };
    let families = list();
    for name in [test_fonts::DEFAULT, test_fonts::TARGET, test_fonts::STYLE] {
        assert!(families.iter().any(|family| family.name == name), "{name}");
    }
}
