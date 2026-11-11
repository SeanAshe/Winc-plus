//! [`Config`] 的测试：各节的读入、校验、问题报告、序列化，以及写回时保留不认识的键。

use super::*;

#[test]
fn empty_and_partial_configs_fall_back_to_defaults() {
    let (config, problems) = Config::parse("{}");
    assert_eq!(config, Config::default());
    assert!(problems.is_empty());

    let (config, problems) = Config::parse(r#"{"editor": {"size": 16}}"#);
    assert_eq!(config.editor_size, Some(16.0));
    assert_eq!(config.tab_width, None);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn reads_every_field() {
    let (config, problems) = Config::parse(
        r#"{
                "editor": {
                    "font": "Helvetica Neue",
                    "size": 14.5,
                    "tabWidth": 2
                },
                "ui": {
                    "font": "/System/Library/Fonts/SFNS.ttf",
                    "size": 12,
                    "zoom": 125
                },
                "tree": { "confirmDragAndDrop": false },
                "format": {
                    "onSave": true,
                    "onSaveExclude": ["Markdown"],
                    "commands": { "JSON": "jq --indent 4 .", "c++": " clang-format ", "rust": "" }
                },
                "theme": "light"
            }"#,
    );

    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.editor_font.as_deref(), Some("Helvetica Neue"));
    assert_eq!(config.editor_size, Some(14.5));
    assert_eq!(config.tab_width, Some(2));
    assert_eq!(
        config.ui_font.as_deref(),
        Some("/System/Library/Fonts/SFNS.ttf")
    );
    assert_eq!(config.ui_size, Some(12.0));
    assert_eq!(config.ui_zoom, Some(125));
    assert_eq!(config.theme, Some(ThemeChoice::Light));
    assert_eq!(config.confirm_drag_and_drop, Some(false));
    // 语言写名字（`JSON`）也能识别并存成 id；空命令视为没写，因此 `rust` 一项不出现。
    assert_eq!(config.format_on_save, Some(true));
    assert_eq!(config.format_on_save_exclude, ["markdown"]);
    assert_eq!(
        config.format_commands,
        BTreeMap::from([
            ("cpp".to_string(), "clang-format".to_string()),
            ("json".to_string(), "jq --indent 4 .".to_string()),
        ])
    );
}

#[test]
fn bad_values_are_reported_and_ignored() {
    let (config, problems) = Config::parse(
        r#"{
                "editor": {
                    "size": 99,
                    "tabWidth": "宽",
                    "fnot": "打错字了"
                },
                "theme": "blue",
                "tree": { "confirmDragAndDrop": "no" },
                "uii": {}
            }"#,
    );

    assert_eq!(config.editor_size, None);
    assert_eq!(config.tab_width, None);
    assert_eq!(config.theme, None);
    assert_eq!(config.confirm_drag_and_drop, None);
    assert_eq!(problems.len(), 6, "{problems:?}");
    let (whitespace, why) = Config::parse(r#"{"editor": {"renderWhitespace": "boundary"}}"#);
    assert_eq!(whitespace.render_whitespace, None);
    assert!(
        why[0].contains("none、selection、trailing 或 all"),
        "{why:?}"
    );
    let (fraction, why) = Config::parse(r#"{"editor": {"tabWidth": 2.5}}"#);
    assert_eq!(fraction.tab_width, None);
    assert!(why[0].contains("应为整数"), "{why:?}");
    // 界面缩放：超出范围时忽略并报告，小数取整。
    let (zoom, why) = Config::parse(r#"{"ui": {"zoom": 300}}"#);
    assert_eq!(zoom.ui_zoom, None);
    assert!(why[0].contains("超出界面缩放范围（70–200）"), "{why:?}");
    let (zoom, why) = Config::parse(r#"{"ui": {"zoom": 117.5}}"#);
    assert_eq!((zoom.ui_zoom, why.len()), (Some(118), 0));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("tree.confirmDragAndDrop 应为 true 或 false"))
    );
    assert!(problems[0].contains("无法识别 editor.fnot"), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("无法识别 editor.fnot")));
    assert!(problems.iter().any(|p| p.contains("无法识别 uii")));
}

#[test]
fn bad_format_settings_are_reported_one_by_one() {
    let (config, problems) = Config::parse(
        r#"{"format": {
                "onSave": "yes",
                "onSaveExclude": ["cobol", 3, "md", "markdown"],
                "commands": { "cobol": "cobc", "go": 1, "JSON": "jq .", "json": "jq -S ." },
                "fixCommands": { "go": true, "Python": "", "typescript": " eslint " },
                "onsave": true
            }}"#,
    );
    assert_eq!(config.format_on_save, None);
    assert_eq!(config.format_on_save_exclude, ["markdown"]);
    // 同一种语言写了两遍（`JSON` 和 `json`）：按键名排序后靠后的一条生效，并报告该问题。
    assert_eq!(
        config.format_commands,
        BTreeMap::from([("json".to_string(), "jq -S .".to_string())])
    );
    assert_eq!(
        config.fix_commands,
        BTreeMap::from([("typescript".to_string(), "eslint".to_string())])
    );
    for expected in [
        "format.onSave 应为 true 或 false",
        "format.onSaveExclude 中有无法识别的语言「cobol」",
        "format.onSaveExclude 中有无法识别的语言「md」",
        "format.onSaveExclude 中的项应为字符串",
        "format.commands 中有无法识别的语言「cobol」",
        "format.commands.go 应为字符串",
        "format.commands 中「json」与另一项指向同一种语言",
        "format.fixCommands.go 应为字符串",
        "无法识别 format.onsave",
    ] {
        assert!(
            problems.iter().any(|p| p.contains(expected)),
            "{expected}：{problems:?}"
        );
    }
    assert_eq!(problems.len(), 9, "{problems:?}");
}

#[test]
fn project_configs_only_read_the_format_section() {
    let (config, problems) = Config::parse_project(
        r#"{
                "format": { "onSave": true, "commands": { "TypeScript": "biome format --stdin-file-path={file}" } },
                "editor": { "size": 30 }
            }"#,
    );
    assert_eq!(config.format_on_save, Some(true));
    assert_eq!(
        config.format_commands.get("typescript").map(String::as_str),
        Some("biome format --stdin-file-path={file}")
    );
    assert_eq!(config.editor_size, None, "项目改不了编辑器的字号");
    assert_eq!(problems, ["项目设置仅支持 format，已忽略 editor"]);
    assert!(!Config::parse_project("[]").1.is_empty());

    let dir = std::env::temp_dir().join(format!("winc-project-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(Config::load_project(&dir).is_none());
    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::write(
        project_config_path(&dir),
        r#"{"format": {"onSave": false}}"#,
    )
    .unwrap();
    let (config, problems) = Config::load_project(&dir).unwrap();
    assert_eq!((config.format_on_save, problems.len()), (Some(false), 0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_json_does_not_panic() {
    for text in [
        "",
        "{",
        "[]",
        "null",
        r#"{"a" 1}"#,
        r#"{"a": }"#,
        r#"{"editor": {"size": 14}} 后面还有东西"#,
        // 扁平的点号键不被识别：报告中出现「无法识别 editor.size」。
        r#"{"editor.size": 14}"#,
        "{\"editor.font\": \"Menlo\n\"}",
    ] {
        let (config, problems) = Config::parse(text);
        assert!(!problems.is_empty(), "{text:?} 应该报错");
        assert_eq!(config, Config::default(), "{text:?} 应该退回默认值");
    }
}

#[test]
fn round_trips_through_json() {
    let config = Config {
        editor_font: Some("Menlo".to_string()),
        editor_size: Some(14.0),
        ui_font: Some("「带引号」的字体".to_string()),
        ui_size: Some(12.5),
        ui_zoom: Some(150),
        ui_native_title_bar: Some(true),
        ui_animations: Some(false),
        ui_font_weight: Some(500),
        ui_background_image: Some("/图片/壁纸.jpg".to_string()),
        ui_background_opacity: Some(20.0),
        ui_background_fit: Some(BackgroundFit::Tile),
        tab_width: Some(8),
        auto_save: Some(true),
        hot_exit: Some(false),
        restore_folder: Some(false),
        open_folder_in_new_window: Some(true),
        sticky_scroll: Some(false),
        word_wrap: Some(true),
        whole_lines: Some(true),
        minimap: Some(true),
        outline: Some(false),
        render_whitespace: Some(RenderWhitespace::Trailing),
        diff_inline: Some(true),
        line_numbers: Some(false),
        status_bar: Some(false),
        restore_window: Some(false),
        theme: Some(ThemeChoice::Dark),
        light_theme: Some("paper".to_string()),
        dark_theme: Some("ember".to_string()),
        language: Some(LocaleChoice::Fixed(winc_i18n::Locale::EnUs)),
        markdown_remote_images: Some(false),
        update_auto_check: Some(false),
        confirm_drag_and_drop: Some(false),
        compact_folders: Some(false),
        preview_on_click: Some(true),
        external_terminal: Some("test-terminal".to_string()),
        terminal_enabled: Some(true),
        terminal_font: Some("Menlo".to_owned()),
        terminal_size: Some(12.0),
        terminal_scrollback: Some(5000),
        option_as_meta: Some(OptionAsMeta::Both),
        terminal_minimum_contrast: Some(4.5),
        terminal_shell: Some("wsl.exe -d Ubuntu".to_owned()),
        terminal_shell_integration: Some(false),
        terminal_confirm_close: Some(false),
        terminal_prompt: Some(false),
        terminal_copy_on_select: Some(true),
        terminal_bold_is_bright: Some(true),
        terminal_right: Some(true),
        git_enabled: Some(true),
        git_confirm_push: Some(false),
        repository_scan_depth: Some(4),
        git_worktree_root: Some("~/worktrees".to_string()),
        git_worktree_copy: vec![".env*".to_string(), "config/local.toml".to_string()],
        format_on_save: Some(false),
        format_on_save_exclude: vec!["markdown".to_string(), "yaml".to_string()],
        format_commands: BTreeMap::from([(
            "python".to_string(),
            "sh -c 'ruff format - | cat'".to_string(),
        )]),
        fix_on_save: Some(true),
        fix_commands: BTreeMap::from([("python".to_string(), "ruff check --fix -".to_string())]),
    };

    let (parsed, problems) = Config::parse(&config.to_json());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(parsed, config);

    // 手写的小数写回后保持原样：`f32` 按最短十进制写法输出，不补成 `12.300000190734863`。
    let odd = Config {
        editor_size: Some(12.3),
        ..Config::default()
    };
    assert!(
        odd.to_json().contains("\"size\": 12.3\n"),
        "{}",
        odd.to_json()
    );
}

#[test]
fn escapes_round_trip() {
    let text = r#"{"editor": {"font": "a\"b\\c\nd\u4e2d\ud83d\ude00"}}"#;
    let (config, problems) = Config::parse(text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.editor_font.as_deref(), Some("a\"b\\c\nd中😀"));
}

#[test]
fn save_to_round_trips_and_creates_the_directory() {
    let dir = std::env::temp_dir().join(format!("winc-config-test-{}", std::process::id()));
    let path = dir.join("nested").join("config.json");
    let _ = std::fs::remove_dir_all(&dir);

    let config = Config {
        editor_font: Some("Menlo".to_string()),
        editor_size: Some(15.0),
        tab_width: Some(2),
        theme: Some(ThemeChoice::Dark),
        ..Config::default()
    };
    let saved = config
        .save_to(&path, &Config::default())
        .expect("写得进临时目录");
    assert_eq!(
        saved,
        Saved {
            config: config.clone(),
            backup: None
        }
    );

    let (loaded, problems) = Config::load_from(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(loaded, config);
    // 改名后不应残留临时文件。
    assert!(!path.with_extension("json.tmp").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn to_json_writes_sections_in_a_fixed_order() {
    let config = Config {
        theme: Some(ThemeChoice::Auto),
        git_enabled: Some(true),
        editor_size: Some(13.0),
        editor_font: Some("Menlo".to_string()),
        format_commands: BTreeMap::from([("json".to_string(), "jq .".to_string())]),
        ..Config::default()
    };
    assert_eq!(
        config.to_json(),
        "{\n  \"editor\": {\n    \"font\": \"Menlo\",\n    \"size\": 13\n  },\n  \"git\": {\n    \
             \"enabled\": true\n  },\n  \"format\": {\n    \"commands\": {\n      \"json\": \
             \"jq .\"\n    }\n  },\n  \"theme\": \"auto\"\n}\n"
    );
    assert_eq!(Config::default().to_json(), "{}\n");
}

#[test]
fn saving_keeps_keys_this_version_does_not_know() {
    let dir = std::env::temp_dir().join(format!("winc-config-merge-{}", std::process::id()));
    let path = dir.join("config.json");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &path,
        r#"{"editor": {"size": 99, "font": "Menlo", "future": [1]}, "ui": 3, "later": {"a": 1}}"#,
    )
    .unwrap();

    let (mut config, problems) = Config::load_from(&path);
    assert_eq!(problems.len(), 4, "{problems:?}");
    let base = config.clone();
    config.editor_font = None;
    config.tab_width = Some(2);
    assert_eq!(config.save_to(&path, &base).unwrap().backup, None);

    let text = std::fs::read_to_string(&path).unwrap();
    let root = Json::parse(&text).unwrap();
    let root = root.as_object().unwrap();
    let editor = root["editor"].as_object().unwrap();
    // 不认识的键原样保留。修改过的项以内存中的设置为准：改了值的写新值，清除的不再写出。
    // 读入时忽略的值（`size: 99`）没有修改，原样保留，下次启动时仍然报告。
    assert_eq!(editor["future"], Json::Array(vec![Json::Number(1.0)]));
    assert_eq!(editor["tabWidth"], Json::Number(2.0));
    assert!(!editor.contains_key("font"));
    assert_eq!(editor["size"], Json::Number(99.0));
    assert_eq!(root["ui"], Json::Number(3.0));
    assert_eq!(root["later"], Json::object([("a", Json::Number(1.0))]));
    assert!(!path.with_extension("json.bak").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_writes_only_what_changed_and_keeps_edits_made_meanwhile() {
    let dir = std::env::temp_dir().join(format!("winc-config-edits-{}", std::process::id()));
    let path = dir.join("config.json");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, r#"{"editor": {"size": 13, "font": "Menlo"}}"#).unwrap();
    let (mut config, _) = Config::load_from(&path);
    let base = config.clone();

    // 程序运行期间，用户在其他位置修改了文件：改了字号，删除了字体，增加了主题。
    let edited = r#"{"editor": {"size": 16}, "theme": "dark", "ui": {"size": 14}}"#;
    std::fs::write(&path, edited).unwrap();
    // 设置页同时修改了自动换行和界面字号：只写入这两项，手动修改的项保留。
    // 两边都修改的项（界面字号）以设置页的值为准。
    config.word_wrap = Some(true);
    config.ui_size = Some(12.0);
    let saved = config.save_to(&path, &base).unwrap();

    let expected = Config {
        editor_size: Some(16.0),
        ui_size: Some(12.0),
        word_wrap: Some(true),
        theme: Some(ThemeChoice::Dark),
        ..Config::default()
    };
    assert_eq!(
        saved,
        Saved {
            config: expected.clone(),
            backup: None
        }
    );
    assert_eq!(Config::load_from(&path), (expected, Vec::new()));

    // 文件已被删除：只写入修改的那一项，内存中的其余设置不再写回。
    std::fs::remove_file(&path).unwrap();
    let mut config = saved.config.clone();
    config.tab_width = Some(4);
    let saved = config.save_to(&path, &saved.config).unwrap();
    let expected = Config {
        tab_width: Some(4),
        ..Config::default()
    };
    assert_eq!(saved.config, expected);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), expected.to_json());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_over_an_unreadable_file_backs_it_up_first() {
    let dir = std::env::temp_dir().join(format!("winc-config-backup-{}", std::process::id()));
    let path = dir.join("config.json");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 缺少右括号：整份文件无法解析，读入时退回默认设置。
    let broken = "{\"editor\": {\"font\": \"Menlo\"}";
    std::fs::write(&path, broken).unwrap();

    let (mut config, problems) = Config::load_from(&path);
    assert_eq!(config, Config::default());
    assert_eq!(problems.len(), 1);
    let base = config.clone();
    config.word_wrap = Some(true);
    let backup = config.save_to(&path, &base).unwrap().backup.unwrap();

    assert_eq!(backup.path, path.with_extension("json.bak"));
    assert!(!backup.comments);
    assert_eq!(std::fs::read_to_string(&backup.path).unwrap(), broken);
    assert_eq!(Config::load_from(&path), (config.clone(), Vec::new()));
    // 新文件能读懂，之后的保存不再备份。
    assert_eq!(config.save_to(&path, &config).unwrap().backup, None);

    // 只有空白的文件没有可丢的内容，不备份。
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, " \n").unwrap();
    assert_eq!(config.save_to(&path, &base).unwrap().backup, None);
    assert!(!path.with_extension("json.bak").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn configs_are_read_as_jsonc_and_comments_are_kept_aside_when_writing() {
    let dir = std::env::temp_dir().join(format!("winc-config-jsonc-{}", std::process::id()));
    let path = dir.join("config.json");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 按 JSONC 解析：允许注释、最后一项后面的逗号和开头的 BOM。
    let text = "\u{feff}{\n  // 写代码用的字号\n  \"editor\": {\"size\": 16,},\n}\n";
    std::fs::write(&path, text).unwrap();
    let (mut config, problems) = Config::load_from(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.editor_size, Some(16.0));

    // 写出的文件不保留注释：先另存原文件，再正常写入设置。
    let base = config.clone();
    config.word_wrap = Some(true);
    let saved = config.save_to(&path, &base).unwrap();
    let backup = saved.backup.unwrap();
    assert!(backup.comments);
    assert_eq!(std::fs::read_to_string(&backup.path).unwrap(), text);
    assert_eq!(saved.config, config);
    assert_eq!(Config::load_from(&path), (config.clone(), Vec::new()));
    // 文件中已没有注释，之后不再另存；只有末尾逗号的文件同样不另存。
    assert_eq!(config.save_to(&path, &config).unwrap().backup, None);
    std::fs::write(&path, "{\"editor\": {\"size\": 16,},}").unwrap();
    assert_eq!(config.save_to(&path, &config).unwrap().backup, None);

    let (project, problems) =
        Config::parse_project("{\n  // 团队用的\n  \"format\": {\"onSave\": true,},\n}");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(project.format_on_save, Some(true));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_path_lands_in_the_platform_directory() {
    let path = config_path().expect("总能算出配置路径");
    assert!(path.ends_with("winc/config.json"), "{}", path.display());

    if cfg!(target_os = "macos") {
        assert!(path.to_string_lossy().contains("Application Support"));
    }
}
