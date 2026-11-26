//! 字体名解析的测试：路径与名字的检查、系统字体查询（CoreText 和 fontconfig），以及系统保留名
//! 的处理。

use super::*;

#[test]
fn path_spec_requires_an_existing_file() {
    let existing = std::env::current_exe().expect("测试进程自己的路径总是存在");
    assert_eq!(
        FontSpec::Path(existing.clone()).resolve(),
        Some(FontFile::first(existing))
    );

    assert_eq!(
        FontSpec::Path(PathBuf::from("/nope/nowhere.ttf")).resolve(),
        None
    );
}

#[test]
fn check_tells_a_missing_font_from_one_that_cannot_be_drawn() {
    use crate::text::font::tests::bare_font;

    let dir = std::env::temp_dir().join(format!("winc-font-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hvgl = dir.join("hvgl.ttf");
    std::fs::write(&hvgl, bare_font(&[b"cmap", b"hmtx", b"hvgl"])).unwrap();
    let glyf = dir.join("glyf.ttf");
    std::fs::write(&glyf, bare_font(&[b"cmap", b"glyf"])).unwrap();

    assert_eq!(FontSpec::Path(glyf).check(), Ok(()));
    assert_eq!(FontSpec::Path(hvgl).check(), Err(FontError::Unsupported));
    assert_eq!(
        FontSpec::Path(dir.join("missing.ttf")).check(),
        Err(FontError::NotFound)
    );
    assert_eq!(
        FontSpec::Name("winc 不存在的字体 42".into()).check(),
        Err(FontError::NotFound)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_font_name_resolves_to_nothing() {
    // 名字中带随机因素，不会被真实字体使用，任何平台上都不应解析出文件。
    assert_eq!(
        FontSpec::Name("winc 不存在的字体 42".into()).resolve(),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn coretext_finds_the_system_monospace_font() {
    let found = FontSpec::Name("Menlo".into())
        .resolve()
        .expect("Menlo 是 macOS 自带字体");
    assert!(found.path.to_string_lossy().contains("Menlo"));
    // 字族名取常规体；PostScript 名指定的是集合中的哪一个字体。
    assert_eq!(found.postscript.as_deref(), Some("Menlo-Regular"));
    let bold = FontSpec::Name("Menlo-Bold".into()).resolve().unwrap();
    assert_eq!(bold.postscript.as_deref(), Some("Menlo-Bold"));
}

#[cfg(target_os = "macos")]
#[test]
fn coretext_does_not_report_a_substitute_as_a_hit() {
    // 按名字创建字体时，CoreText 对不认识的名字会改用 Helvetica，前缀也会被当成 Helvetica；
    // 这两种结果都不算找到。以 `.` 开头的系统保留名同理。
    for name in ["Winc 绝不会装的字体", "H", "Helvet", ".SF NS Mono", ""] {
        assert_eq!(FontSpec::Name(name.into()).resolve(), None, "{name:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn coretext_accepts_localized_family_names() {
    let found = FontSpec::Name("苹方-简".into())
        .resolve()
        .expect("苹方是 macOS 自带字体");
    assert_eq!(found.postscript.as_deref(), Some("PingFangSC-Regular"));
}

#[cfg(target_os = "macos")]
#[test]
fn font_files_with_spaces_in_their_names_are_found() {
    let found = FontSpec::Name("Hiragino Sans GB".into()).resolve();
    let found = found.expect("冬青黑体是 macOS 自带字体");
    assert!(
        found.path.ends_with("Hiragino Sans GB.ttc"),
        "{:?}",
        found.path
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_font_names_map_to_the_system_font_files() {
    let mono = FontSpec::Name("SF Mono".into()).resolve().unwrap();
    assert!(mono.path.ends_with("SFNSMono.ttf"));
    let ui = FontSpec::Name("sf pro".into()).resolve().unwrap();
    assert!(ui.path.ends_with("SFNS.ttf"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn linux_and_windows_scan_font_directories() {
    // 只验证调用不会 panic：这两个平台上能否找到字体取决于系统安装了哪些字体。
    let _ = FontSpec::Name("DejaVu Sans Mono".into()).resolve();
}

/// Linux 上查询隔离的 fontconfig 配置，不依赖系统装了哪些字体。
#[cfg(target_os = "linux")]
mod fontconfig {
    use super::*;
    use crate::text::fontconfig::test_fonts::{self, Isolated};

    #[test]
    fn a_font_in_a_custom_directory_resolves() {
        let Some(isolated) = Isolated::new() else {
            return;
        };
        let spec = FontSpec::Name(test_fonts::DEFAULT.into());
        let found = spec.resolve().expect("fontconfig 认得测试字体");
        assert_eq!(found.path, isolated.regular(test_fonts::DEFAULT));
        // 解析出的文件要能打开，才能用来显示文字。
        assert_eq!(spec.check(), Ok(()));
    }

    #[test]
    fn a_missing_name_is_not_replaced_by_a_substitute() {
        let Some(_isolated) = Isolated::new() else {
            return;
        };
        let spec = FontSpec::Name("Winc Missing 7f3a".into());
        assert_eq!(spec.resolve(), None);
        assert_eq!(spec.check(), Err(FontError::NotFound));
        // 字体回退列表里每个名字都要能各自判断，不能让一个后备字体占住所有名字。
        assert!(crate::text::font_path::resolve_face("Winc Missing 7f3a", true, true).is_none());
    }

    #[test]
    fn a_configured_alias_resolves_to_its_target() {
        let Some(isolated) = Isolated::new() else {
            return;
        };
        // 别名指向的字体和未知名字的后备字体是同一个文件，仍要按别名找到它。
        assert_eq!(
            isolated.regular(isolated.alias()).as_path(),
            isolated.fallback()
        );
        let found = FontSpec::Name(test_fonts::ALIAS.into())
            .resolve()
            .expect("配置里把 Winc Alias 指向了测试字族");
        assert_eq!(found.path, isolated.fallback());
    }

    #[test]
    fn a_family_name_with_pattern_syntax_is_taken_literally() {
        let Some(isolated) = Isolated::new() else {
            return;
        };
        for family in [test_fonts::LITERAL, test_fonts::OPTION] {
            let found = FontSpec::Name(family.into())
                .resolve()
                .unwrap_or_else(|| panic!("{family:?}"));
            assert_eq!(found.path, isolated.regular(family), "{family:?}");
        }
    }

    #[test]
    fn a_request_for_italic_does_not_invent_an_italic_face() {
        let Some(isolated) = Isolated::new() else {
            return;
        };
        let found = crate::text::font_path::resolve_face(test_fonts::TARGET, false, true)
            .expect("字族存在");
        assert_eq!(found.file.path, isolated.regular(test_fonts::TARGET));
        assert!(!found.italic, "字族里没有斜体，取到的是正体");
        assert!(!found.bold);
    }

    #[test]
    fn a_real_italic_face_is_reported_as_italic() {
        let Some(isolated) = Isolated::new() else {
            return;
        };
        let found = crate::text::font_path::resolve_face(test_fonts::ITALIC, false, true)
            .expect("字族存在");
        assert_eq!(found.file.path, isolated.italic(test_fonts::ITALIC));
        assert!(found.italic);
        assert!(!found.bold);

        // 不要斜体时取常规体。
        let found = crate::text::font_path::resolve_face(test_fonts::ITALIC, false, false)
            .expect("字族存在");
        assert_eq!(found.file.path, isolated.regular(test_fonts::ITALIC));
        assert!(!found.italic);
    }

    #[test]
    fn the_style_flags_describe_the_face_that_was_found() {
        let Some(isolated) = Isolated::new() else {
            return;
        };
        let found = crate::text::font_path::resolve_face(test_fonts::TARGET, true, false)
            .expect("字族存在");
        assert_eq!(found.file.path, isolated.bold(test_fonts::TARGET));
        assert!(found.bold);
        assert!(!found.italic);

        // 同时请求粗体和斜体，字族里没有斜体：取粗体，斜体标志为假，由调用方合成斜体。
        let found =
            crate::text::font_path::resolve_face(test_fonts::TARGET, true, true).expect("字族存在");
        assert!(found.bold);
        assert!(!found.italic);
    }
}
