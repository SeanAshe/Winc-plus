//! fontconfig 查询的测试：用私有配置查自定义目录、别名、字面名字和实际样式。

use super::test_fonts::{self, EmptyConfig, Isolated};
use super::*;

/// 把字族名列表写成字符串，便于测试比较。
fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_font_in_a_custom_directory_is_found() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    let found = lookup(test_fonts::DEFAULT, false, false).expect("fontconfig 认得测试字体");
    assert_eq!(found.file.path, isolated.regular(test_fonts::DEFAULT));
}

/// 空配置返回空字体集合；安全的 `FontSet::iter` 在这种情况下不会解引用空的 `fonts` 数组。
#[test]
fn an_empty_configuration_has_no_fonts() {
    let Some(_empty) = EmptyConfig::new() else {
        return;
    };
    assert!(available());
    assert_eq!(files(), Some(Vec::new()));
    assert!(best("sans-serif").is_none());
    assert!(sorted("sans-serif").is_empty());
    assert!(lookup(test_fonts::DEFAULT, false, false).is_none());
}

#[test]
fn family_names_ignore_case_and_spaces_but_not_hyphens() {
    // fontconfig 的规则：`DejaVuSans` 和 `DejaVu Sans` 是同一个名字，`DejaVu-Sans` 不是。
    assert!(names_family(&names(&["DejaVu Sans"]), "dejavusans"));
    assert!(names_family(
        &names(&["DejaVu Sans", "Nordwand Mono"]),
        "NordwandMono"
    ));
    assert!(!names_family(&names(&["DejaVu Sans"]), "DejaVu-Sans"));
    assert!(!names_family(&names(&["DejaVu_Sans"]), "DejaVu Sans"));
}

#[test]
fn an_unknown_name_is_not_substituted() {
    let Some(_isolated) = Isolated::new() else {
        return;
    };
    // fontconfig 对任何名字都会给出一个字体；这不是该名字的字体，要按找不到处理。
    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        assert!(
            lookup("Winc Missing 7f3a", bold, italic).is_none(),
            "{bold} {italic}"
        );
    }
}

#[test]
fn an_alias_configured_in_fontconfig_is_found() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    assert_ne!(isolated.other_alias(), isolated.alias());
    assert_ne!(
        isolated.regular(isolated.other_alias()).as_path(),
        isolated.fallback()
    );
    let found = lookup(test_fonts::OTHER_ALIAS, false, false).expect("配置里有别名");
    assert_eq!(found.file.path, isolated.regular(isolated.other_alias()));
}

/// 回归：别名指向的字体正好是未知名字的后备字体时，不能因为文件相同就按找不到处理。
#[test]
fn an_alias_to_the_fallback_face_is_found() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    assert_eq!(
        isolated.regular(isolated.alias()).as_path(),
        isolated.fallback()
    );
    // 结果字族里没有别名，单看查询结果分不出它和未知名字。
    let direct = best(test_fonts::ALIAS).expect("fontconfig 给出字体");
    assert!(!names_family(&direct.families, test_fonts::ALIAS));
    let found = lookup(test_fonts::ALIAS, false, false).expect("别名指向后备字体");
    assert_eq!(found.file.path, isolated.fallback());
    // fontconfig 的字族名比较忽略大小写。
    assert!(lookup(&test_fonts::ALIAS.to_lowercase(), false, false).is_some());
    // 带样式的请求同样按别名处理，即使字族里没有对应的粗体、斜体。
    assert!(lookup(test_fonts::ALIAS, true, true).is_some());
}

#[test]
fn a_collection_face_keeps_its_index() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    let found = lookup(test_fonts::COLLECTION, true, false).expect("集合里有粗体");
    assert_eq!(found.file.path, isolated.regular(test_fonts::COLLECTION));
    assert_eq!(found.file.index, Some(1), "粗体是集合中的第二个字体");
    let face = crate::text::font::Face::open(&found.file).expect("集合字体可以打开");
    assert_eq!(
        crate::text::font::postscript_name(&face.font()).as_deref(),
        Some("WincCollection-Bold")
    );
}

#[test]
fn an_unknown_alias_target_is_not_a_font() {
    let Some(_isolated) = Isolated::new() else {
        return;
    };
    // 配置里有指向 Winc Uninstalled Target 的别名，但系统里没有这个字族；不能把 fontconfig
    // 对未知名字的任意后备字体当成别名解析成功。
    assert!(lookup(test_fonts::MISSING_ALIAS, false, false).is_none());
}

#[test]
fn a_font_excluded_by_the_config_is_not_listed_or_found() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    let excluded = isolated.excluded();
    assert!(excluded.is_file());
    let files = files().expect("fontconfig 可用");
    assert!(!files.contains(&excluded));
    assert!(lookup(test_fonts::EXCLUDED, false, false).is_none());
    assert!(
        !crate::text::font_list::list()
            .iter()
            .any(|family| family.name == test_fonts::EXCLUDED)
    );
    // fontconfig 可用时不能因为按名字查不到就退回扫描固定目录，把配置排除的字体又找回来。
    assert!(
        crate::FontSpec::Name(test_fonts::EXCLUDED.into())
            .resolve()
            .is_none()
    );
}

/// 真的找不到 libfontconfig 时，进程要能启动并退回常见字体目录。
#[test]
fn a_missing_library_starts_and_uses_common_paths() {
    let dir = std::env::temp_dir().join(format!("winc-no-fontconfig-{}", std::process::id()));
    let fonts = dir.join("fonts");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    let expected = fonts.join("Winc_Default-Regular.ttf");
    test_fonts::write_font(&expected, test_fonts::DEFAULT);
    // 让动态加载器找到这个名字但加载失败，模拟系统没有 libfontconfig。
    std::fs::write(lib.join("libfontconfig.so.1"), b"not a shared library").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "text::fontconfig::tests::a_common_path_child",
            "--nocapture",
        ])
        .env("XDG_DATA_HOME", &dir)
        .env("LD_LIBRARY_PATH", &lib)
        .env("WINC_TEST_COMMON_FONT", &expected)
        .status()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(status.success());
}

/// 在子进程里确认没有 fontconfig 时各查询为空，并退回常见目录找到字体；正常测试中直接跳过。
#[test]
fn a_common_path_child() {
    let Some(expected) = std::env::var_os("WINC_TEST_COMMON_FONT") else {
        return;
    };
    assert!(!available());
    assert!(best("sans-serif").is_none());
    assert!(sorted("sans-serif").is_empty());
    assert!(files().is_none());
    assert!(lookup(test_fonts::DEFAULT, false, false).is_none());
    let found = crate::FontSpec::Name(test_fonts::DEFAULT.into())
        .resolve()
        .expect("常见字体目录里的字体");
    assert_eq!(found.path, std::path::PathBuf::from(expected));
    assert!(
        crate::text::font_list::list()
            .iter()
            .any(|family| family.name == test_fonts::DEFAULT)
    );
}

#[test]
fn a_family_name_is_taken_literally() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    // 名字里有 fontconfig 模式的语法字符：`:`、`,`、`\`、`=` 和开头表示选项的 `-`。
    for family in [test_fonts::LITERAL, test_fonts::OPTION] {
        let found = lookup(family, false, false).unwrap_or_else(|| panic!("{family:?}"));
        assert_eq!(found.file.path, isolated.regular(family), "{family:?}");
    }
}

#[test]
fn generic_family_names_are_found() {
    let Some(_isolated) = Isolated::new() else {
        return;
    };
    // 通用字族名不是任意替换，即使 fontconfig 给出的是参数相同的后备字体。
    for family in ["serif", "sans-serif", "monospace", "system-ui", "sans"] {
        assert!(lookup(family, false, false).is_some(), "{family}");
    }
}

#[test]
fn sorted_fonts_are_usable_font_files() {
    let Some(_isolated) = Isolated::new() else {
        return;
    };
    let fonts = sorted("sans-serif");
    assert!(!fonts.is_empty());
    assert!(fonts.iter().all(|font| font.path.is_file()));
}

/// `monospace` 的回退顺序按配置列出，只保留能显示前面的字体所缺字符的字体。
#[test]
fn the_monospace_chain_keeps_the_fonts_that_add_characters() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    let chain: Vec<_> = chain("monospace").into_iter().map(|f| f.path).collect();
    let expected = [test_fonts::CHAIN_FIRST, test_fonts::CHAIN_NEXT].map(|f| isolated.regular(f));
    assert_eq!(chain, expected);
}

#[test]
fn the_font_list_contains_the_custom_directory() {
    let Some(isolated) = Isolated::new() else {
        return;
    };
    let files = files().expect("fontconfig 可用");
    for family in [test_fonts::DEFAULT, test_fonts::TARGET, test_fonts::STYLE] {
        assert!(files.contains(&isolated.regular(family)), "{family}");
    }
}
