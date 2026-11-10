//! 字体加载的测试：字形表的选择、可变字体的命名实例、斜体候选，以及注释的 ASCII 快速路径。

use super::*;
#[cfg(target_os = "macos")]
use crate::text::system_outline::GlyphPath;

/// 返回只含表目录的字体文件数据：列出 `tags` 中的表，每个表的内容为空。
pub(crate) fn bare_font(tags: &[&[u8; 4]]) -> Vec<u8> {
    let count = tags.len() as u16;
    let mut data = Vec::new();
    data.extend(0x0001_0000u32.to_be_bytes());
    data.extend(count.to_be_bytes());
    data.extend([0; 6]);
    let end = 12 + 16 * u32::from(count);
    for tag in tags {
        data.extend(*tag);
        data.extend(0u32.to_be_bytes());
        data.extend(end.to_be_bytes());
        data.extend(0u32.to_be_bytes());
    }
    data
}

#[test]
fn only_fonts_with_drawable_glyphs_are_selected() {
    // 有任何一种 swash 能绘制的字形表即可选用；只有 `hvgl` 表时，macOS 上改由系统提供轮廓，
    // 其他平台上不能选用；一种字形表都没有时不能选用。
    let system = |tags: &[&[u8; 4]]| select(&bare_font(tags), None).map(|s| s.system);
    for tag in GLYPH_TABLES {
        assert_eq!(system(&[b"cmap", tag]), Some(false), "{tag:?}");
    }
    let hvgl = system(&[b"cmap", b"hmtx", b"hvgl"]);
    assert_eq!(hvgl, cfg!(target_os = "macos").then_some(true));
    assert_eq!(system(&[b"cmap", b"hmtx"]), None);
}

/// 返回已打开字体的 PostScript 名。
#[cfg(target_os = "macos")]
fn postscript(face: &Face) -> String {
    let font = face.font();
    let name = font
        .localized_strings()
        .find_by_id(StringId::PostScript, None);
    name.expect("系统字体都有 PostScript 名").chars().collect()
}

#[cfg(target_os = "macos")]
#[test]
fn named_instances_of_variable_fonts_are_selected_by_postscript_name() {
    // macOS 15 起的苹方是可变字体：`PingFangSC-Regular` 只是 `PingFangSC-Medium` 字体
    // 中的一个命名实例，按字体自身的名字找不到，要比对命名实例。
    let file = FontSpec::Name("苹方-简".into())
        .resolve()
        .expect("苹方是 macOS 自带字体");
    let face = Face::open(&file).expect("苹方能用");
    let font = face.font();
    if font.instances().len() == 0 {
        return; // 旧版 macOS 的苹方不是可变字体，每个字重是单独的字体。
    }
    let names: Vec<_> = (0..font.instances().len())
        .filter_map(|i| instance_postscript(&font, i))
        .collect();
    assert!(
        names.iter().any(|n| n == "PingFangSC-Semibold"),
        "{names:?}"
    );
    let regular = font
        .instances()
        .find(|i| instance_postscript(&font, i.index()).as_deref() == Some("PingFangSC-Regular"))
        .expect("命名实例中有常规体");
    assert_eq!(face.coords, regular.normalized_coords().collect::<Vec<_>>());
}

#[cfg(target_os = "macos")]
#[test]
fn hvgl_only_fonts_get_outlines_from_the_system() {
    let file = FontSpec::Name("苹方-简".into()).resolve().unwrap();
    let face = Face::open(&file).expect("苹方能用");
    let font = face.font();
    let Some(system) = &face.system else {
        // 不只有 `hvgl` 表的苹方由 swash 读取：旧版 macOS 的有 `glyf` 表，macOS 27 的有 `CFF ` 表。
        let drawable = GLYPH_TABLES.map(|t| font.table(swash::tag_from_bytes(t)).is_some());
        assert!(drawable.contains(&true));
        return;
    };
    // 字宽由 swash 按命名实例的坐标读取，必须与提供轮廓的 CoreText 字体一致，否则字形与排版
    // 位置对不上。CoreText 的字宽另加了 `trak` 表中按字号给出的字距（每个字形相同），因此比较
    // 两边的差值是否对所有字形都相同：实例不对时拉丁字母的差值与汉字不同。
    let gaps: Vec<f32> = ['中', 'a', '0', '。']
        .into_iter()
        .map(|c| {
            let glyph = font.charmap().map(c);
            assert_ne!(glyph, 0, "{c}");
            let ours = font
                .glyph_metrics(&face.coords)
                .scale(1000.0)
                .advance_width(glyph);
            system.advance(glyph, 1000.0) - ours
        })
        .collect();
    assert!(gaps.iter().all(|g| (g - gaps[0]).abs() < 0.5), "{gaps:?}");
    assert!(gaps[0].abs() < 10.0, "{gaps:?}");
    let mut path = GlyphPath::default();
    system.outline(font.charmap().map('中'), 16.0, &mut path);
    assert!(path.verbs.len() > 10, "{path:?}");
    let mut space = GlyphPath::default();
    system.outline(font.charmap().map(' '), 16.0, &mut space);
    assert!(space.verbs.is_empty(), "空格没有轮廓");
}

#[cfg(target_os = "macos")]
#[test]
fn a_collection_opens_the_face_named_by_its_postscript_name() {
    let path = "/System/Library/Fonts/Menlo.ttc";
    let bold = FontFile {
        path: path.into(),
        postscript: Some("Menlo-Bold".into()),
    };
    let face = Face::open(&bold).expect("Menlo 是 macOS 自带字体");
    assert_eq!(postscript(&face), "Menlo-Bold");

    // 未指定 PostScript 名，或集合中没有该名字时，取第一个字体。
    let first = Face::open(&FontFile::first(path)).unwrap();
    assert_eq!(postscript(&first), "Menlo-Regular");
    let unknown = FontFile {
        path: path.into(),
        postscript: Some("Menlo-Nothing".into()),
    };
    assert_eq!(postscript(&Face::open(&unknown).unwrap()), "Menlo-Regular");
}

#[test]
#[cfg(target_os = "macos")]
fn italics_are_found_next_to_the_regular_face() {
    // 系统等宽字体：`SFNSMono.ttf` 旁边的 `SFNSMonoItalic.ttf`。
    let sf = FontFile::first("/System/Library/Fonts/SFNSMono.ttf");
    let face = Face::open(&sf).expect("SF Mono 是 macOS 自带字体");
    let found = italic_candidates(&sf, &face);
    assert_eq!(
        found[0].path,
        PathBuf::from("/System/Library/Fonts/SFNSMonoItalic.ttf")
    );

    // 字体集合：同一个文件里的 `Menlo-Italic`。
    let menlo = FontFile::first("/System/Library/Fonts/Menlo.ttc");
    let face = Face::open(&menlo).unwrap();
    let found = italic_candidates(&menlo, &face);
    assert_eq!(found[0].postscript.as_deref(), Some("Menlo-Italic"));
    assert_eq!(postscript(&Face::open(&found[0]).unwrap()), "Menlo-Italic");
}

#[cfg(target_os = "macos")]
#[test]
fn ascii_comments_take_the_fast_path_without_an_italic() {
    // Monaco 没有斜体：注释里的 ASCII 字符落到正体上，查 ASCII 表即可，不进哈希表。
    let monaco = FontSpec::Path("/System/Library/Fonts/Monaco.ttf".into());
    let mut fonts = Fonts::load(Some(&monaco), None, None, 400).expect("Monaco 是 macOS 自带字体");
    let regular = fonts.resolve(FontKind::Mono, 'a');
    assert_eq!(fonts.resolve(FontKind::MonoItalic, 'a'), regular);
    assert!(matches!(fonts.slots[MONO_ITALIC as usize], Slot::Missing));
    assert_eq!(
        fonts.ascii[FontKind::MonoItalic as usize]['a' as usize],
        Some(regular)
    );
    assert!(fonts.resolved.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn the_terminal_uses_its_own_font_or_the_editor_font() {
    // 没有单独的终端字体：终端的正体、斜体与编辑区的相同。
    let mut fonts = Fonts::load(None, None, None, 400).expect("系统里有等宽字体");
    assert_eq!(
        fonts.resolve(FontKind::Terminal, 'a'),
        fonts.resolve(FontKind::Mono, 'a')
    );
    let italic = fonts.resolve(FontKind::MonoItalic, 'a');
    assert_eq!(fonts.resolve(FontKind::TerminalItalic, 'a'), italic);

    // 单独设置了终端字体：用终端自己的槽位。Monaco 没有斜体，终端的斜体落到终端的正体上。
    let monaco = FontSpec::Path("/System/Library/Fonts/Monaco.ttf".into());
    let mut fonts = Fonts::load(None, None, Some(&monaco), 400).expect("Monaco 是 macOS 自带字体");
    let (slot, glyph) = fonts.resolve(FontKind::Terminal, 'a');
    assert_eq!(slot, TERMINAL);
    assert_ne!(glyph, 0);
    assert_eq!(fonts.resolve(FontKind::TerminalItalic, 'a'), (slot, glyph));
    assert_ne!(fonts.resolve(FontKind::Mono, 'a').0, TERMINAL);

    // 指定的终端字体打不开时用编辑区的字体，不报错。
    let missing = FontSpec::Path("/nonexistent/font.ttf".into());
    let mut fonts = Fonts::load(None, None, Some(&missing), 400).expect("系统里有等宽字体");
    assert_eq!(fonts.resolve(FontKind::Terminal, 'a').0, MONO);
}

#[test]
fn italic_comments_fall_back_to_the_regular_face() {
    let mut fonts = Fonts::load(None, None, None, 400).expect("系统里有等宽字体");
    // 有斜体时用斜体槽位，没有时用正体；两种情况下注释里的字符都画得出来。
    let (slot, glyph) = fonts.resolve(FontKind::MonoItalic, 'a');
    assert!(slot == MONO_ITALIC || slot == MONO);
    assert_ne!(glyph, 0);
    // 斜体中没有的字符（中文）仍然落到后备字体上，与正体相同；系统中没有中文字体（只装了
    // 西文字体的精简系统）时两者都找不到，不做比较。
    let (_, regular) = fonts.resolve(FontKind::Mono, '中');
    if regular != 0 {
        let (slot, glyph) = fonts.resolve(FontKind::MonoItalic, '中');
        assert!(slot >= FALLBACK || slot == UI);
        assert_ne!(glyph, 0);
    }
}

/// 回归：用户目录里的字体读进内存，使用中被截断时不会让整个程序收到 SIGBUS 退出。
#[cfg(unix)]
#[test]
fn only_files_users_cannot_change_are_mapped() {
    let path = std::env::temp_dir().join(format!("winc-font-{}.ttf", std::process::id()));
    std::fs::write(&path, b"not really a font").unwrap();
    let data = load(&File::open(&path).unwrap()).unwrap();
    assert_eq!(&*data, b"not really a font");
    // 以 root 运行时（例如在容器中运行测试）自己建的文件也属于 root，按系统字体映射，此时
    // 截断会使映射失效，因此不做检查。
    let root = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&path).unwrap()) == 0;
    assert_eq!(matches!(data, FontData::Owned(_)), !root);
    if !root {
        // 截断文件不影响已经读进来的内容。
        std::fs::write(&path, b"").unwrap();
        assert_eq!(&*data, b"not really a font");
    }
    drop(data);
    std::fs::remove_file(&path).unwrap();
    // 属于 root 的文件仍然映射。
    let system = load(&File::open("/etc/hosts").unwrap()).unwrap();
    assert!(matches!(system, FontData::Mapped(_)));
}

/// 回归：旧版 OS/2 表（版本 1）没有 x 字高和大写字高，按 0 处理时界面文字的视觉中心算成
/// 基线，整体偏上。
#[test]
fn fonts_without_stated_heights_measure_their_letters() {
    let fonts = Fonts::load(None, None, None, 400).expect("系统中有等宽字体");
    let face = fonts.primary_face(FontKind::Mono).unwrap();
    let font = face.font();
    let stated = crate::text::glyphs::heights(font, &face.coords, 100.0);

    // 同一字体的副本，OS/2 表的版本改为 1。
    let mut data = font.data.to_vec();
    let offset = font.offset as usize;
    let count = usize::from(u16::from_be_bytes([data[offset + 4], data[offset + 5]]));
    let record = (0..count)
        .map(|i| offset + 12 + 16 * i)
        .find(|&r| &data[r..r + 4] == b"OS/2")
        .expect("字体有 OS/2 表");
    let table = u32::from_be_bytes(data[record + 8..record + 12].try_into().unwrap()) as usize;
    data[table..table + 2].copy_from_slice(&1u16.to_be_bytes());
    let old = FontRef {
        data: &data,
        offset: font.offset,
        key: font.key,
    };
    assert_eq!(old.metrics(&face.coords).x_height, 0.0);

    let measured = crate::text::glyphs::heights(old, &face.coords, 100.0);
    // 字母的轮廓含过冲，与字体声明的值相差几个百分点。
    assert!(
        (measured.0 - stated.0).abs() < 4.0,
        "{measured:?} {stated:?}"
    );
    assert!(
        (measured.1 - stated.1).abs() < 4.0,
        "{measured:?} {stated:?}"
    );
    assert!(measured.0 > 30.0 && measured.1 > measured.0, "{measured:?}");
}

#[test]
fn sorted_fontconfig_results_keep_order_and_skip_missing_files() {
    let dir = std::env::temp_dir().join(format!("winc-sorted-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("a.ttf"), dir.join("b.ttc"));
    std::fs::write(&a, "").unwrap();
    std::fs::write(&b, "").unwrap();
    let text = format!(
        "{}\nFirst\n/no/such/file.ttf\nGone\n{}\n\n",
        a.display(),
        b.display()
    );

    let fonts = parse_sorted_fonts(&text);
    assert_eq!(fonts.len(), 2);
    assert_eq!(fonts[0].path, a);
    assert_eq!(fonts[0].postscript.as_deref(), Some("First"));
    assert_eq!(fonts[1].path, b);
    assert_eq!(fonts[1].postscript, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 回归：fontconfig 把不含汉字的字体排在最前时，中文后备槽位继续往后找含汉字的字体。
#[cfg(target_os = "linux")]
#[test]
fn the_cjk_slot_skips_fonts_without_chinese() {
    let query = "sans-serif:lang=zh-cn";
    let covers =
        |file: &FontFile| Face::open(file).is_some_and(|face| face.font().charmap().map('中') != 0);
    let sorted = fontconfig_sorted(query);
    let Some(chinese) = sorted.iter().find(|file| covers(file)) else {
        return;
    };
    let latin = sorted
        .iter()
        .find(|file| Face::open(file).is_some() && !covers(file));
    let Some(latin) = latin else {
        return;
    };

    // 不含汉字的字体排在前面，也选用后面含汉字的字体。
    let choices = Choices {
        query: None,
        files: vec![latin.clone(), chinese.clone()],
        probe: Some('中'),
    };
    let face = choices.open(0).unwrap();
    assert_ne!(face.font().charmap().map('中'), 0);

    // 查询结果本身也是如此，无论 fontconfig 把什么字体排在第一个。
    let choices = Choices {
        query: Some(query.to_owned()),
        files: Vec::new(),
        probe: Some('中'),
    };
    assert_ne!(choices.open(0).unwrap().font().charmap().map('中'), 0);

    // 都不含时仍用第一个能打开的字体。
    let choices = Choices {
        query: None,
        files: vec![latin.clone()],
        probe: Some('中'),
    };
    assert!(choices.open(0).is_some());
}

/// 回归：Linux 上默认字体能绘制中文。字体不在常见位置的发行版靠 fontconfig 找到中文字体；系统里
/// 没有中文字体时不检查。
#[cfg(target_os = "linux")]
#[test]
fn chinese_text_finds_a_font_on_linux() {
    let has_chinese = std::process::Command::new("fc-list")
        .arg(":lang=zh")
        .output()
        .is_ok_and(|output| !output.stdout.is_empty());
    if !has_chinese {
        return;
    }
    let mut fonts = Fonts::load(None, None, None, 400).expect("系统里有等宽字体");
    for kind in [FontKind::Ui, FontKind::Mono] {
        let (_, glyph) = fonts.resolve(kind, '中');
        assert_ne!(glyph, 0, "{kind:?}");
    }
}

#[test]
fn chinese_japanese_and_korean_locales_pick_their_own_glyph_forms() {
    for (locale, language) in [
        ("zh_TW.UTF-8", "zh-tw"),
        ("zh_HK.UTF-8", "zh-hk"),
        ("zh_CN.UTF-8", "zh-cn"),
        ("ja_JP.UTF-8", "ja-jp"),
        ("ko_KR.UTF-8", "ko-kr"),
        ("zh", "zh-cn"),
        ("en_US.UTF-8", "zh-cn"),
        ("C", "zh-cn"),
        ("", "zh-cn"),
    ] {
        assert_eq!(cjk_language_of(locale), language, "{locale}");
    }
}

#[test]
fn the_app_language_overrides_the_system_language_for_cjk_glyphs() {
    assert!(set_cjk_language(Some("zh-cn")));
    assert_eq!(cjk_language(), "zh-cn");
    // 同一种语言再设一次不算变化，调用方不必重新加载字体。
    assert!(!set_cjk_language(Some("zh-cn")));
    // 不再指定时回到按系统语言选。
    assert!(set_cjk_language(None));
    assert_eq!(
        cjk_language(),
        cjk_language_of(
            &["LC_ALL", "LC_CTYPE", "LANG"]
                .iter()
                .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
                .unwrap_or_default()
        )
    );
}

#[cfg(target_os = "macos")]
#[test]
fn weight_follows_the_axis_and_the_rest_is_emboldened() {
    // 有字重轴：只调整坐标，不加宽笔画。
    let sf = FontFile::first("/System/Library/Fonts/SFNSMono.ttf");
    let regular = Face::open(&sf).expect("SF Mono 是 macOS 自带字体");
    let (_, max) = weight_range(&regular.font()).expect("SF Mono 有字重轴");
    let bold = Face::open_at(&sf, 300).unwrap();
    assert_ne!(bold.coords, regular.coords);
    assert_eq!((regular.embolden, bold.embolden), (0.0, 0.0));
    assert_eq!(Face::open_at(&sf, 0).unwrap().coords, regular.coords);
    // 超出轴的范围：不足的部分加宽笔画。
    let past = Face::open_at(&sf, (max - REGULAR) as i16 + 150).unwrap();
    assert!((past.embolden - crate::BOLD_RATIO / 2.0).abs() < 1e-6);

    // Menlo 没有字重轴，同一文件中有粗体（700）：加 300 换用粗体；加 200 仍用常规体，加 450
    // 用粗体，差额都加宽笔画。
    let menlo = FontFile::first("/System/Library/Fonts/Menlo.ttc");
    let bold = Face::open_at(&menlo, 300).unwrap();
    assert_eq!(
        (postscript(&bold).as_str(), bold.embolden),
        ("Menlo-Bold", 0.0)
    );
    let medium = Face::open_at(&menlo, 200).unwrap();
    assert_eq!(postscript(&medium), "Menlo-Regular");
    assert!((medium.embolden - crate::BOLD_RATIO * 2.0 / 3.0).abs() < 1e-6);
    let black = Face::open_at(&menlo, 450).unwrap();
    assert_eq!(postscript(&black), "Menlo-Bold");
    assert!((black.embolden - crate::BOLD_RATIO / 2.0).abs() < 1e-6);
    // 斜体只在斜体中找。
    let italic = FontFile {
        postscript: Some("Menlo-Italic".to_owned()),
        ..menlo.clone()
    };
    assert_eq!(
        postscript(&Face::open_at(&italic, 300).unwrap()),
        "Menlo-BoldItalic"
    );
    // 调细时换用同族中更细的字体。
    let heavy = FontFile {
        postscript: Some("Menlo-Bold".to_owned()),
        ..menlo
    };
    assert_eq!(
        postscript(&Face::open_at(&heavy, -300).unwrap()),
        "Menlo-Regular"
    );

    // Monaco 没有同族的其他字体：调粗靠加宽笔画，调细不处理。
    let monaco = FontFile::first("/System/Library/Fonts/Monaco.ttf");
    assert_eq!(
        Face::open_at(&monaco, 300).unwrap().embolden,
        crate::BOLD_RATIO
    );
    assert_eq!(Face::open_at(&monaco, -100).unwrap().embolden, 0.0);
}
