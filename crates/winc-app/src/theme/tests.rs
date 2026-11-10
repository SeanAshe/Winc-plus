//! `theme` 的测试：文件类型图标的识别顺序、各套配色的对比度底线、对比度保护与项目徽章。

use super::*;

impl Palette {
    /// 返回默认的深色（`dark` 为 `true`）或浅色主题，供测试和截图使用。
    pub fn for_dark(dark: bool) -> &'static Palette {
        themes(dark)[0].palette
    }
}

#[test]
fn file_kinds_follow_whole_names_then_extensions() {
    use FileGlyph::*;
    let glyph = |name: &str| file_kind(name).0;
    assert_eq!(glyph("photo.HEIC"), Image);
    assert_eq!(glyph("logo.svg"), Image);
    assert_eq!(glyph("song.flac"), Audio);
    assert_eq!(glyph("clip.mov"), Video);
    assert_eq!(glyph("Inter.woff2"), Font);
    assert_eq!(glyph("dist.tar.gz"), Archive);
    assert_eq!(glyph("libfoo.dylib"), Binary);
    assert_eq!(glyph("Cargo.lock"), Lock);
    assert_eq!(glyph("id_ed25519.pub"), Lock);
    // 整个名字先认：没有扩展名的、点开头的、带点的。
    assert_eq!(glyph("Dockerfile"), Docker);
    assert_eq!(glyph("Dockerfile.dev"), Docker);
    assert_eq!(glyph("docker-compose.yml"), Docker);
    assert_eq!(glyph(".dockerignore"), Docker);
    assert_eq!(glyph("Makefile"), Shell);
    assert_eq!(glyph("go.mod"), Package);
    assert_eq!(glyph("package.json"), Package);
    assert_eq!(glyph("Cargo.toml"), Package);
    assert_eq!(glyph(".env.local"), Key);
    assert_eq!(glyph(".gitignore"), Git);
    assert_eq!(glyph(".gitattributes"), Git);
    assert_eq!(glyph("LICENSE"), License);
    assert_eq!(glyph("LICENSE.md"), License);
    assert_eq!(glyph("README.zh-CN.md"), Info);
    assert_eq!(glyph("NOTES.md"), Markdown);
    assert_eq!(glyph("turbo.json"), Json);
    assert_eq!(glyph("config.yaml"), Config);
    assert_eq!(glyph("ruff.toml"), Config);
    assert_eq!(glyph(".editorconfig"), Config);
    // NOTICE 是普通文档，LICENSE 才是许可证。
    assert_eq!(glyph("NOTICE.txt"), Text);
    assert_eq!(glyph("NOTICE"), Text);
    assert_eq!(glyph("COPYING"), License);
    // 锁文件按整个名字认，不当成 JSON、YAML。
    assert_eq!(glyph("pnpm-lock.yaml"), Lock);
    assert_eq!(glyph("package-lock.json"), Lock);
    assert_eq!(glyph("users.csv"), Table);
    assert_eq!(glyph("schema.sql"), Database);
    assert_eq!(glyph("main.rb"), Letters("RB"));
    // 常见语言是缩写，JSX、TSX 和 JS、TS 同一个缩写、换个颜色。
    assert_eq!(glyph("app.ts"), Letters("TS"));
    assert_eq!(glyph("App.tsx"), Letters("TS"));
    assert_ne!(file_kind("app.ts").1, file_kind("App.tsx").1);
    assert_eq!(glyph("index.mjs"), Letters("JS"));
    assert_eq!(glyph("main.py"), Letters("PY"));
    assert_eq!(glyph("main.go"), Letters("GO"));
    assert_eq!(glyph("lib.rs"), Letters("RS"));
    assert_eq!(glyph("main.c"), Letters("C"));
    assert_eq!(glyph("main.cpp"), Letters("C++"));
    assert_eq!(glyph("Main.java"), Java);
    assert_eq!(glyph("App.vue"), Code);
    // `file_kind` 给出的每种记号都在 `FileGlyph::ALL` 里：目录菜单按它事先画图标。
    let samples = [
        "a.ts",
        "a.js",
        "a.py",
        "a.go",
        "a.rs",
        "a.c",
        "a.cpp",
        "a.cs",
        "a.kt",
        "a.rb",
        "a.swift",
        "a.java",
        "a.json",
        "a.yaml",
        "a.toml",
        "a.md",
        "Dockerfile",
        ".gitignore",
        "LICENSE",
        "README",
        ".env",
        "package.json",
        "a.csv",
        "a.sql",
        "a.txt",
        "a.bin",
        "a.png",
        "a.zip",
        "a.mp3",
        "a.mp4",
        "a.ttf",
        "a.lock",
        "a.sh",
        "a.vue",
        "a.xyz",
    ];
    for name in samples {
        assert!(FileGlyph::ALL.contains(&glyph(name)), "{name}");
    }
    assert_eq!(glyph("whatever.xyz"), Plain);
}

#[test]
fn light_syntax_colors_stand_out_from_the_card() {
    for theme in &LIGHT_THEMES {
        let p = theme.palette;
        for (name, color) in [
            ("keyword", p.syntax_keyword),
            ("builtin", p.syntax_builtin),
            ("string", p.syntax_string),
            ("number", p.syntax_number),
            ("function", p.syntax_function),
            ("decorator", p.syntax_decorator),
            ("property", p.syntax_property),
        ] {
            let ratio = contrast(color, p.editor_bg);
            assert!(ratio >= 5.95, "{} {name}: {ratio:.2}", theme.id);
        }
        let ratio = contrast(p.syntax_comment, p.editor_bg);
        assert!(ratio >= 4.45, "{} comment: {ratio:.2}", theme.id);
    }
}

/// 「幕」系列在背景图最不利时仍然清晰：不透明度 30%，深色主题下方是纯白的图，浅色主题下方是
/// 纯黑的图，正文至少 6.5:1，语法色至少 4.5:1，注释和次要文字至少 3:1。浮层的底色与卡片不同。
#[test]
fn veil_themes_stay_readable_over_a_background_image() {
    let veils = (LIGHT_THEMES.iter().chain(&DARK_THEMES)).filter(|t| t.id.starts_with("veil-"));
    let mut count = 0;
    for theme in veils {
        count += 1;
        let p = theme.palette;
        let image = if p.dark { 1.0 } else { 0.0 };
        let part = |c: f32| c * 0.7 + image * 0.3;
        let (c, mut worst) = (p.editor_bg, p.editor_bg);
        (worst.r, worst.g, worst.b) = (part(c.r), part(c.g), part(c.b));
        let check = |name: &str, color: Color, least: f32| {
            let ratio = contrast(color, worst);
            assert!(ratio >= least, "{} {name}: {ratio:.2}", theme.id);
        };
        check("text", p.text, 6.45);
        for (name, color) in [
            ("keyword", p.syntax_keyword),
            ("builtin", p.syntax_builtin),
            ("string", p.syntax_string),
            ("number", p.syntax_number),
            ("function", p.syntax_function),
            ("decorator", p.syntax_decorator),
            ("property", p.syntax_property),
        ] {
            check(name, color, 4.45);
        }
        check("comment", p.syntax_comment, 2.95);
        check("muted", p.text_muted, 2.95);
        assert_ne!(p.popup_bg, p.editor_bg, "{}", theme.id);
    }
    assert_eq!(count, 4);
}

#[test]
fn terminal_colors_stand_out_from_the_card() {
    for theme in LIGHT_THEMES.iter().chain(&DARK_THEMES) {
        let p = theme.palette;
        for (i, &color) in p.ansi.iter().enumerate() {
            // 深色主题的黑、浅色主题的亮白本来就接近底色（程序拿它们铺底），不要求。
            if (p.dark && i == 0) || (!p.dark && i == 15) {
                continue;
            }
            let ratio = contrast(color, p.editor_bg);
            assert!(ratio >= 3.0, "{} ansi {i}: {ratio:.2}", theme.id);
        }
    }
}

#[test]
fn unreadable_colors_get_just_enough_contrast() {
    let dark_bg = DARK.editor_bg;
    // 深蓝字在深色底上：调亮到刚好 3:1，色相不变（还是蓝的）。
    let navy = Color::hex(0x000080);
    let fixed = readable(navy, dark_bg, 3.0);
    assert!((contrast(fixed, dark_bg) - 3.0).abs() < 0.1);
    assert!(fixed.b > fixed.r && fixed.b > fixed.g);
    // 本来就够的不动；同色（用来藏字）不动；最低对比度为 0 时不动。
    let white = Color::hex(0xFFFFFF);
    assert_eq!(readable(white, dark_bg, 3.0), white);
    assert_eq!(readable(dark_bg, dark_bg, 3.0), dark_bg);
    assert_eq!(readable(navy, dark_bg, 0.0), navy);
    // 浅色底上调暗。
    let pale = Color::hex(0xFFFF99);
    let fixed = readable(pale, Color::hex(0xFFFFFF), 3.0);
    assert!(contrast(fixed, Color::hex(0xFFFFFF)) >= 2.9);
    // 终端程序常在绿底上写黑字：底色按明度算偏深，黑字也只往更黑调，不翻成白字。
    let (black, green) = (WINC_DAY.ansi[0], WINC_DAY.ansi[2]);
    let fixed = readable(black, green, 3.0);
    assert!(contrast(fixed, green) >= 2.9);
    assert!(luminance(fixed) < luminance(black));
    // 往本来那头到底也不够时翻到另一头：中灰底上的浅灰字，要 4.5:1 只能用黑字。
    let gray = Color::hex(0x777777);
    let fixed = readable(Color::hex(0x888888), gray, 4.5);
    assert!(contrast(fixed, gray) >= 4.4);
    assert!(luminance(fixed) < luminance(gray));
}

#[test]
fn project_badges_take_the_initials_and_the_given_color() {
    let badge = LIGHT.project_badge("grok2api", 3);
    assert_eq!(badge.initials, "GR");
    assert_eq!((badge.background, badge.text), LIGHT.badges[3]);
    assert_eq!(LIGHT.project_badge("DEEIX-Chat", 0).initials, "DE");
    assert_eq!(LIGHT.project_badge("中文项目", 0).initials, "中文");
    assert_eq!(LIGHT.project_badge("--", 0).initials, "W");

    // 十种颜色互不相同，深色、浅色两套配色都是如此。
    for palette in LIGHT_THEMES.iter().chain(&DARK_THEMES).map(|t| t.palette) {
        for (i, a) in palette.badges.iter().enumerate() {
            assert!(palette.badges[i + 1..].iter().all(|b| b.0 != a.0), "{i}");
        }
    }
}

#[test]
fn winc_themes_are_comfortable_and_readable() {
    for (id, p) in [
        ("winc-night", &WINC_NIGHT),
        ("winc-slate", &WINC_SLATE),
        ("winc-day", &WINC_DAY),
        ("winc-paper", &WINC_PAPER),
    ] {
        // 设置里选得到。
        assert!(themes(p.dark).iter().any(|t| t.id == id), "{id}");
        // 底色不用纯黑纯白。
        let bg = luminance(p.editor_bg);
        assert!(bg > 0.005 && bg < 0.97, "{id} card: {bg:.3}");
        let ratio = |c| contrast(c, p.editor_bg);
        assert!(ratio(p.text) >= 7.0, "{id} text: {:.2}", ratio(p.text));
        assert!(ratio(p.syntax_text) >= 7.0, "{id} syntax text");
        assert!(ratio(p.text_muted) >= 4.5, "{id} muted");
        assert!(ratio(p.syntax_comment) >= 4.5, "{id} comment");
        assert!(
            contrast(p.text_on_accent, p.accent_fill) >= 4.5,
            "{id} accent fill"
        );
        assert!(contrast(p.text, p.selection) >= 4.5, "{id} selection");
        let syntax = [
            p.syntax_keyword,
            p.syntax_builtin,
            p.syntax_string,
            p.syntax_number,
            p.syntax_function,
            p.syntax_decorator,
            p.syntax_property,
        ];
        for (i, &c) in syntax.iter().enumerate() {
            assert!(ratio(c) >= 5.9, "{id} syntax {i}: {:.2}", ratio(c));
            // 每种语法色各不相同，也不和正文同色。
            assert!(
                c != p.syntax_text && !syntax[i + 1..].contains(&c),
                "{id} {i}"
            );
        }
    }
}

/// 朝原来的方向调到极端也不够、只能翻到另一头时用纯色，不停在刚好达标的灰色上。
#[test]
fn flipped_colors_go_all_the_way() {
    // 深色主题的 ANSI 白写在 ANSI 蓝上：纯白也只有约 2.5:1。
    let (white, blue) = (Color::hex(0xC9CED6), Color::hex(0x6FA6EE));
    assert!(contrast(Color::hex(0xFFFFFF), blue) < 3.0);
    assert_eq!(readable(white, blue, 3.0), Color::hex(0x000000));
}
