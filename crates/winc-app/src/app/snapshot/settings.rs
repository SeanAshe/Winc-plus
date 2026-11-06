//! 设置浮层与字号缩放的测试：模态行为、键盘遍历、鼠标命中、字体名校验、滚动、动画。

// 格式化设置页的测试用 sh 脚本冒充格式化工具。
#[cfg(unix)]
mod format;

use super::*;
#[cfg(unix)]
use winc_editor::format::{PRESETS, Task};

#[test]
fn scrolling_settings_reuses_the_window_below_and_looks_the_same() {
    use std::time::{Duration, Instant};

    let dir = sample_folder("settings-scroll-base");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_settings();
    app.settings.motion = None;
    paint(&mut app);
    assert!(app.settings.scroll_base.is_none());

    // 第一帧完整绘制并保存快照。
    app.settings_scroll_by(-10.0);
    paint(&mut app);
    assert!(app.settings.scroll_base.is_some());
    let window = app.layout.window;
    let scale = app.renderer.as_ref().unwrap().scale_factor();
    assert!(app.settings_reuses_base(window, scale));

    // 之后的帧使用快照，画面与完整绘制相同。
    app.settings_scroll_by(-30.0);
    paint(&mut app);
    let reused = app
        .renderer
        .as_ref()
        .unwrap()
        .image()
        .unwrap()
        .pixels
        .clone();

    // 停止滚动后释放快照并重绘。
    let now = Instant::now();
    assert!(app.settings_scroll_tick(now).is_some());
    assert!(app.settings.scroll_base.is_some());
    assert_eq!(app.settings_scroll_tick(now + Duration::from_secs(1)), None);
    assert!(app.settings.scroll_base.is_none());
    assert!(!app.settings_reuses_base(window, scale));
    paint(&mut app);
    let full = &app.renderer.as_ref().unwrap().image().unwrap().pixels;
    let differ = (reused.iter().zip(full)).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "使用快照的画面与完整绘制不同");

    // 关闭设置时释放快照。
    app.settings_scroll_by(-10.0);
    paint(&mut app);
    app.close_settings();
    assert!(app.settings.scroll_base.is_none());
    assert_eq!(app.settings.scrolled_at, None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn zoom_changes_the_editor_font_size_and_resets() {
    let dir = search_folder("zoom");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let ui = app.fonts.ui;

    app.zoom(Some(true));
    assert_eq!(app.config.editor_size, Some(CODE_FONT + 1.0));
    assert_eq!(app.fonts.code, CODE_FONT + 1.0);
    // 界面字号保持不变。
    assert_eq!(app.fonts.ui, ui);

    app.zoom(Some(false));
    app.zoom(Some(false));
    assert_eq!(app.fonts.code, CODE_FONT - 1.0);

    // ⌘0 还原：配置中不再记录字号。
    app.zoom(None);
    assert_eq!(app.config.editor_size, None);
    assert_eq!(app.fonts.code, CODE_FONT);

    // 已达上限时再放大：字号不变，并在状态栏提示。
    app.config.editor_size = Some(*Config::EDITOR_SIZE_RANGE.end());
    app.zoom(Some(true));
    assert_eq!(
        app.config.editor_size,
        Some(*Config::EDITOR_SIZE_RANGE.end())
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("无法继续调整"))
    );
    // 测试中没有配置文件路径，不会写入用户的配置。
    assert!(app.config_path.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_is_a_modal_over_the_whole_window() {
    let dir = search_folder("settings-modal");
    let mut app = app_with(&dir.join("a.txt"), false);
    let tabs = app.editor.documents.len();

    app.open_settings();
    assert!(app.settings.open);
    // 设置不是标签：标签数量不变，当前文档仍是原来的文档。
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(app.editor.active().unwrap().name(), "a.txt");

    // 卡片相对整个窗口居中，而不是相对编辑区居中（遮罩覆盖侧栏和编辑区）。
    let layout = app.settings_layout();
    let window = app.layout.window;
    let editor = app.layout.editor;
    assert!((layout.card.x + layout.card.w / 2.0 - window.w / 2.0).abs() <= 1.0);
    assert!((layout.card.x + layout.card.w / 2.0 - (editor.x + editor.w / 2.0)).abs() > 50.0);

    // 模态：下层的文件树和编辑区不响应悬停；按键不会修改文档。
    let tree = app.layout.tree;
    app.on_mouse_move(tree.x + 4.0, tree.y + 4.0);
    assert_eq!(app.hover, Hover::None);
    let before = app.editor.active().unwrap().revision;
    app.edit_field(|f| f.insert("x"));
    assert_eq!(app.editor.active().unwrap().revision, before);

    // 点击遮罩关闭设置，焦点回到编辑区；该次点击不传递到下层。
    click(&mut app, (window.x + 4.0, window.bottom() - 4.0));
    assert!(!app.settings.open);
    assert_eq!(app.focus, Focus::Editor);

    // 关闭按钮同样可以关闭设置。
    app.open_settings();
    let close = settings_point(&app, SettingsPart::Close);
    click(&mut app, close);
    assert!(!app.settings.open);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_keyboard_walks_every_item_and_adjusts() {
    let dir = search_folder("settings-keys");
    let mut app = app_with(&dir, false);
    app.open_settings();

    // 从第一页走到最后一页：依次经过外观、编辑器、格式化、模块各页的每一行、快捷键页的
    // 搜索框，最后是关于页的各行，到达末尾后不再移动。越过一页的末项时自动换页，右侧显示该页
    // 内容。
    // 格式化页列出的语言随机器上装了哪些工具而变，步数按全部设置项计算。
    let mut seen = vec![app.settings.selected];
    for _ in 0..Setting::all(&app.settings.languages).len() + 2 {
        app.settings_move(true);
        if seen.last() != Some(&app.settings.selected) {
            seen.push(app.settings.selected);
        }
    }
    assert_eq!(seen, Setting::all(&app.settings.languages));
    assert_eq!(app.settings.selected, Setting::CheckUpdates);
    assert_eq!(app.settings_layout().page, SettingsPage::About);
    // 往回走过关于页的三行，回到快捷键页的搜索框。
    for _ in 0..3 {
        app.settings_move(false);
    }
    assert_eq!(app.settings.selected, Setting::KeySearch);
    assert_eq!(app.settings_layout().page, SettingsPage::Keys);
    assert_eq!(app.focus, Focus::KeySearch);
    // 向回走一步，回到模块页最后的终端开关，再一步是 Git 开关；再走一步，回到格式化页的最后
    // 一项（修复工具一组的「添加语言…」），焦点回到浮层。
    app.settings_move(false);
    assert_eq!(app.settings.selected, Setting::TerminalModule);
    assert_eq!(app.focus, Focus::Settings);
    app.settings_move(false);
    assert_eq!(app.settings.selected, Setting::GitModule);
    app.settings_move(false);
    assert_eq!(app.settings.selected, Setting::AddFixLanguage);

    // 主题：← → 在三个选项之间切换，到头后不再变化。步数按全部设置项计算，原因同上。
    for _ in 0..Setting::all(&app.settings.languages).len() {
        app.settings_move(false);
    }
    assert_eq!(app.settings.selected, Setting::Theme);
    app.set_theme_choice(ThemeChoice::Auto);
    app.settings_step(true);
    assert_eq!(app.theme_choice, ThemeChoice::Light);
    assert!(!app.palette.dark);
    app.settings_step(true);
    assert_eq!(app.theme_choice, ThemeChoice::Dark);
    assert!(app.palette.dark);
    app.settings_step(true);
    assert_eq!(app.theme_choice, ThemeChoice::Dark);

    // 语言：← → 不改变选择。
    app.settings_move(true);
    assert_eq!(app.settings.selected, Setting::Language);
    app.settings_step(true);
    assert_eq!(app.config.language, None);

    // 浅色主题（语言下面是界面字体、字号，macOS 以外的平台还有「使用系统标题栏」，然后是状态栏和
    // 记住窗口两个开关，再下面是两组主题）：外观为深色时选一个浅色主题，外观随之切换。深色主题：
    // → 换到下一个，外观回到深色；到头后不再变化。
    let title_bar = usize::from(!cfg!(target_os = "macos"));
    for _ in 0..5 + title_bar {
        app.settings_move(true);
    }
    assert_eq!(app.settings.selected, Setting::LightTheme);
    app.settings_step(true);
    assert_eq!(app.config.light_theme.as_deref(), Some("winc-paper"));
    assert_eq!(app.theme_choice, ThemeChoice::Light);
    assert!(std::ptr::eq(app.palette, &crate::theme::WINC_PAPER));
    app.settings_move(true);
    assert_eq!(app.settings.selected, Setting::DarkTheme);
    app.settings_step(true);
    assert_eq!(app.config.dark_theme.as_deref(), Some("winc-slate"));
    assert_eq!(app.theme_choice, ThemeChoice::Dark);
    assert!(std::ptr::eq(app.palette, &crate::theme::WINC_SLATE));
    for _ in 0..10 {
        app.settings_step(true);
    }
    let last = crate::theme::DARK_THEMES.last().unwrap().palette;
    assert!(std::ptr::eq(app.palette, last));
    // 跟随系统时只换那一种外观的主题，现在的配色不变。
    app.set_theme_choice(ThemeChoice::Auto);
    let before = app.palette;
    app.set_named_theme(!before.dark, 3);
    assert!(std::ptr::eq(app.palette, before));
    for _ in 0..10 {
        app.settings_step(false);
    }
    assert!(std::ptr::eq(app.palette, &crate::theme::WINC_NIGHT));

    // 编辑器字号：→ 加 1；到达上限后不再变化，对应一侧的按钮变淡。
    for _ in 0..2 {
        app.settings_move(true);
    }
    assert_eq!(app.settings.selected, Setting::EditorSize);
    let before = app.fonts.code;
    app.settings_step(true);
    assert_eq!(app.fonts.code, before + 1.0);
    for _ in 0..40 {
        app.settings_step(true);
    }
    assert_eq!(
        app.config.editor_size,
        Some(*Config::EDITOR_SIZE_RANGE.end())
    );
    assert!(app.setting_value(Setting::EditorSize).2);

    // 测试环境没有配置路径，这些改动都不会写入磁盘。
    assert!(app.config_path.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_mouse_controls_use_the_same_geometry_as_drawing() {
    let dir = search_folder("settings-mouse");
    let mut app = app_with(&dir, false);
    app.open_settings();
    paint(&mut app);

    // 编辑器的设置在第二页：点击分类栏切换过去。
    click_part(&mut app, SettingsPart::Page(SettingsPage::Editor));
    assert_eq!(app.settings_layout().page, SettingsPage::Editor);
    let before = app.fonts.code;
    click_part(&mut app, SettingsPart::Increase(Setting::EditorSize));
    assert_eq!(app.fonts.code, before + 1.0);
    click_part(&mut app, SettingsPart::Decrease(Setting::EditorSize));
    assert_eq!(app.fonts.code, before);

    // 悬停在加号上：记录悬停的是哪个控件。
    let (x, y) = settings_point(&app, SettingsPart::Increase(Setting::TabWidth));
    app.on_mouse_move(x, y);
    assert_eq!(
        app.settings.hover,
        Some(SettingsPart::Increase(Setting::TabWidth))
    );

    click_part(&mut app, SettingsPart::Page(SettingsPage::Appearance));
    click_part(&mut app, SettingsPart::Theme(ThemeChoice::Dark));
    assert_eq!(app.theme_choice, ThemeChoice::Dark);

    // 点击界面字体的输入框进入编辑，焦点交给该输入框。
    click_part(&mut app, SettingsPart::Field(Setting::UiFont));
    assert_eq!(app.focus, Focus::Setting);
    assert!(matches!(&app.settings.editing, Some((Setting::UiFont, _))));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_font_name_is_checked_and_can_be_abandoned() {
    let dir = search_folder("settings-font");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_start_editing(Setting::EditorFont);
    assert_eq!(app.focus, Focus::Setting);

    app.edit_field(|f| {
        f.select_all();
        f.insert("绝对不存在的字体 ZZZ")
    });
    app.settings_commit_field();

    // 字体找不到：按 ↩ 提交时停留在输入框并显示错误，配置退回原值。
    assert!(app.settings.editing.is_some());
    assert_eq!(
        app.settings.error,
        Some((
            Setting::EditorFont,
            "未安装字体「绝对不存在的字体 ZZZ」".to_owned()
        ))
    );

    // 找得到但画不出字形的字体（例如字形只放在 `hvgl` 表中）同样不能提交：渲染层会静默改用
    // 默认字体，界面字体设成这种字体时所有文字都是空白。
    let font = dir.join("hvgl.ttf");
    let mut data = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    data.extend(b"hvgl");
    data.extend([0, 0, 0, 0, 0, 0, 0, 28, 0, 0, 0, 0]);
    std::fs::write(&font, data).unwrap();
    let path = font.to_string_lossy().into_owned();
    app.edit_field(|f| {
        f.select_all();
        f.insert(&path)
    });
    app.settings_commit_field();
    assert!(app.settings.editing.is_some());
    assert_eq!(
        app.settings.error,
        Some((
            Setting::EditorFont,
            format!("不支持字体「{path}」的字形格式")
        ))
    );

    // 配置中原本就是用不了的字体（例如手动改了配置文件）：打开设置、换到该页时在该行写明原因。
    app.settings_cancel_field();
    app.close_settings();
    app.config.editor_font = Some(path.clone());
    app.open_settings();
    assert_eq!(app.settings.error, None, "第一页上没有编辑器字体");
    app.settings_show_page(SettingsPage::Editor);
    assert!(matches!(
        &app.settings.error,
        Some((Setting::EditorFont, _))
    ));
    app.close_settings();
    app.config.editor_font = None;
    app.open_settings();
    app.settings_start_editing(Setting::EditorFont);
    app.edit_field(|f| {
        f.select_all();
        f.insert("绝对不存在的字体 ZZZ")
    });

    // Esc 只放弃本次输入，浮层保持打开，焦点回到浮层。
    app.settings_cancel_field();
    assert!(app.settings.open);
    assert!(app.settings.editing.is_none());
    assert_eq!(app.focus, Focus::Settings);
    assert_eq!(app.config.editor_font, None);

    // 输入到一半就关闭浮层：不完整的名称不保留在配置中。
    app.settings_start_editing(Setting::EditorFont);
    app.edit_field(|f| {
        f.select_all();
        f.insert("JetB")
    });
    assert_eq!(app.config.editor_font.as_deref(), Some("JetB"));
    app.close_settings();
    assert_eq!(app.config.editor_font, None);
    assert_eq!(app.focus, Focus::Editor);

    // 再次打开：从第一页的顶部开始。
    app.open_settings();
    assert_eq!(app.settings.selected, Setting::Theme);
    assert_eq!(app.settings_layout().page, SettingsPage::Appearance);
    assert_eq!(app.settings.scroll, 0.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_pages_are_listed_on_the_left() {
    let dir = search_folder("settings-pages");
    let mut app = app_with(&dir, false);
    app.open_settings();
    paint(&mut app);

    // 分类栏列出每一页，每页只放属于该页的设置项；所有页合起来恰好是全部设置，不遗漏也不重复。
    let layout = app.settings_layout();
    assert_eq!(layout.pages.len(), SettingsPage::ALL.len());
    for &(_, row) in &layout.pages {
        assert!(layout.nav.contains(row.x, row.y) && row.right() <= layout.nav.right());
    }
    let mut listed = Vec::new();
    for page in SettingsPage::ALL {
        let languages = &app.settings.languages;
        let layout = SettingsLayout::compute(app.layout.window, page, languages, None, 0.0);
        for row in &layout.rows {
            assert_eq!(row.setting.page(), page);
            // 分组下方的文字按钮（添加语言）不在分组内，其余设置项都在分组内。
            let in_group = layout
                .groups
                .iter()
                .any(|g| g.contains(row.row.x + 1.0, row.row.y + 1.0));
            assert_eq!(in_group, row.setting.control() != SettingControl::Action);
            assert!(row.row.x >= layout.nav.right(), "设置项不压到分类栏上");
        }
        listed.extend(layout.rows.iter().map(|r| r.setting));
        // 快捷键页没有设置项，只有搜索框。
        if page == SettingsPage::Keys {
            listed.push(Setting::KeySearch);
        }
    }
    assert_eq!(listed, Setting::all(&app.settings.languages));

    // 悬停在分类栏的一项上时记录该项；点击后换页，选中该页的第一项，滚动位置回到顶部。
    let (x, y) = settings_point(&app, SettingsPart::Page(SettingsPage::Editor));
    app.on_mouse_move(x, y);
    assert_eq!(
        app.settings.hover,
        Some(SettingsPart::Page(SettingsPage::Editor))
    );
    app.settings.scroll = 30.0;
    click_part(&mut app, SettingsPart::Page(SettingsPage::Editor));
    assert_eq!(app.settings.selected, Setting::EditorFont);
    assert_eq!(app.settings.scroll, 0.0);
    assert!(app.settings.open);

    // 字体名无效时点击同一页的其他控件：输入退回原值，出错原因留在该行，这次点击仍然生效。
    let missing = |app: &mut App| {
        app.settings_start_editing(Setting::EditorFont);
        app.edit_field(|f| {
            f.select_all();
            f.insert("绝对不存在的字体 ZZZ")
        });
    };
    missing(&mut app);
    let tab_width = app.editor.tab_width;
    click_part(&mut app, SettingsPart::Increase(Setting::TabWidth));
    assert_eq!(app.editor.tab_width, tab_width + 1);
    assert!(app.settings.editing.is_none());
    assert_eq!(app.config.editor_font, None);
    assert!(matches!(
        &app.settings.error,
        Some((Setting::EditorFont, _))
    ));
    assert_eq!(app.focus, Focus::Settings);

    // `Tab` 同样离开输入框，移到下一项。
    missing(&mut app);
    app.settings_move(true);
    assert!(app.settings.editing.is_none());
    assert_eq!(app.config.editor_font, None);
    assert_ne!(app.settings.selected, Setting::EditorFont);

    // 点击其他页：仍然换页，出错原因随之清除。
    missing(&mut app);
    click_part(&mut app, SettingsPart::Page(SettingsPage::Appearance));
    assert_eq!(app.settings_layout().page, SettingsPage::Appearance);
    assert!(app.settings.editing.is_none());
    assert_eq!(app.config.editor_font, None);
    assert_eq!(app.settings.error, None);
    app.settings_show_page(SettingsPage::Editor);

    // 换页时卡片大小不变。
    let editor = app.settings_layout().card;
    app.settings_show_page(SettingsPage::Appearance);
    assert_eq!(app.settings_layout().card, editor);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_body_scrolls_when_the_window_is_short() {
    let dir = search_folder("settings-scroll");
    let mut app = app_with(&dir, false);
    app.open_settings();
    // 内容放得下时，滚轮不滚动内容。用「模块」页：它的行数固定，格式化页列出的语言则随机器
    // 上装了哪些工具而变。
    app.settings_show_page(SettingsPage::Modules);
    let body = app.settings_layout().body;
    app.on_mouse_move(body.x + 20.0, body.y + 20.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -200.0,
    )));
    assert_eq!(app.settings.scroll, 0.0);

    // 窗口变矮后可以滚动，到底即停；文档本身不动。外观页的两组主题各有两排，默认窗口下
    // 也需要滚动，这里用格式化页。
    app.settings_show_page(SettingsPage::Format);
    app.layout.window.h = 280.0;
    let body = app.settings_layout().body;
    app.on_mouse_move(body.x + 20.0, body.y + 20.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -2000.0,
    )));
    let max = app.settings_layout().max_scroll();
    assert!(max > 0.0);
    assert_eq!(app.settings.scroll, max);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_open_from_the_shortcut_without_a_tab() {
    let dir = search_folder("settings-shortcut");
    let mut app = app_with(&dir, false);
    assert!(app.editor.active.is_none(), "只开了目录，没有标签");
    paint(&mut app);

    // 顶栏右端没有按钮：标签栏一直延伸到右端。
    assert_eq!(app.layout.tabs.right(), app.layout.top_bar.right() - 8.0);

    // ⌘,（其他系统上是 Ctrl+,）在任何地方都能打开设置；macOS 的菜单栏「设置…」走同一个动作。
    assert!(app.run_action(crate::keymap::Action::Settings));
    assert!(app.settings.open);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 创建格式化设置页测试用的应用，返回应用及其项目目录。
///
/// 项目的 `pyproject.toml` 中配置了 Ruff；`PATH` 中只有 `bin` 目录，其中有 `rustfmt`、`gofmt`、
/// `jq`，没有 `ruff`。因此哪些工具能找到，与本机实际安装的工具无关。
#[cfg(unix)]
pub(super) fn format_settings_app(name: &str, dark: bool) -> (App, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let dir = search_folder(name);
    std::fs::write(
        dir.join("pyproject.toml"),
        "[tool.ruff]\nline-length = 100\n",
    )
    .unwrap();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for tool in ["rustfmt", "gofmt", "jq"] {
        std::fs::write(bin.join(tool), "#!/bin/sh\ncat\n").unwrap();
        std::fs::set_permissions(bin.join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut app = app_with(&dir, dark);
    app.format.formatter = winc_editor::format::Formatter::with_vars(vec![(
        std::ffi::OsString::from("PATH"),
        bin.into_os_string(),
    )]);
    app.config
        .format_commands
        .insert("json".to_owned(), "jq --indent 4 .".to_owned());
    (app, dir)
}

#[cfg(unix)]
#[test]
fn scrolled_rows_stay_under_the_header() {
    let (mut app, dir) = format_settings_app("settings-scroll-clip", false);
    app.renderer =
        Some(Renderer::offscreen(820.0, 420.0, SCALE, app.palette.frame_bg, None, None).unwrap());
    app.config.format_commands.insert(
        "python".to_owned(),
        "black --quiet --stdin-filename {file} -".to_owned(),
    );
    app.open_settings();
    app.settings_show_page(SettingsPage::Format);
    app.relayout();

    // 滚动到 Python 一行压在标题下沿上：名称已滚到标题之后，其下的说明行和右侧的工具名
    // 有一半在标题区域内。
    let layout = app.settings_layout();
    let python = layout.row(Setting::FormatCommand("python")).unwrap().row;
    app.settings.scroll = python.y - layout.body.y + 35.0;
    assert!(app.settings.scroll <= app.settings_layout().max_scroll());
    paint(&mut app);

    // 标题下沿的一条像素带（标题文字和关闭按钮下方）只应有底色。
    let layout = app.settings_layout();
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let px = |x: f32, y: f32| {
        let (x, y) = ((x * SCALE) as u32, (y * SCALE) as u32);
        image.pixels[(y * image.width + x) as usize]
    };
    let body = layout.body;
    let first = px(body.x + 1.0, body.y - 7.0);
    for y in [body.y - 7.0, body.y - 4.0, body.y - 1.5] {
        for x in (body.x as i32 + 1..body.right() as i32 - 1).step_by(3) {
            assert_eq!(px(x as f32, y), first, "({x}, {y}) 画到了标题里");
        }
    }
    // 不可见的「安装方法…」链接不会被点击命中。
    let hidden = app
        .settings
        .install_links
        .iter()
        .find(|(s, _)| *s == Setting::FormatCommand("python"));
    if let Some(&(_, link)) = hidden {
        let (x, y) = (link.x + 4.0, body.y - 3.0);
        assert_ne!(
            app.settings_part_at(x, y),
            Some(SettingsPart::Install(Setting::FormatCommand("python")))
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_keys_page_lists_searches_and_records() {
    use crate::keymap::{Action, KeyCode, Mods, Stroke};

    let mut app = blank_app(false);
    app.open_settings();
    click_part(&mut app, SettingsPart::Page(SettingsPage::Keys));
    // 进入该页时焦点就在搜索框。
    assert_eq!(app.focus, Focus::KeySearch);
    paint(&mut app);
    let all = app.keys_layout().rows.len();
    assert!(all > 50, "{all}");
    let actions = |app: &App| {
        app.keys_layout()
            .rows
            .iter()
            .map(|(_, r)| r.action)
            .collect::<Vec<_>>()
    };

    // 按名称搜索，中文名和英文名都可以。
    app.edit_field(|f| f.insert("格式化"));
    assert_eq!(actions(&app), [Action::Format, Action::FormatSelection]);
    // Esc 先清空搜索框，搜索框为空时才关闭设置。
    app.keys_page_stroke(Stroke {
        key: KeyCode::Escape,
        mods: Mods::NONE,
    });
    assert_eq!(app.keys_layout().rows.len(), all);
    assert!(app.settings.open);

    // 录制按键：按下的键写入搜索框并据此查找对应命令，按下的键本身不执行。
    click_part(&mut app, SettingsPart::Record);
    assert!(app.settings.recording);
    let cmd = if cfg!(target_os = "macos") {
        Mods::META
    } else {
        Mods::CTRL
    };
    for key in ['k', 's'] {
        app.keys_page_stroke(Stroke {
            key: KeyCode::Char(key),
            mods: cmd,
        });
    }
    assert_eq!(actions(&app), [Action::KeyboardShortcuts]);
    assert!(app.settings.open);
    app.keys_page_stroke(Stroke {
        key: KeyCode::Escape,
        mods: Mods::NONE,
    });
    assert!(!app.settings.recording);
    // 停止录制后，搜索框中仍保留录下的按键。
    assert_eq!(actions(&app), [Action::KeyboardShortcuts]);

    // 滚动到底：列表的最后一行完全露出。
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    app.settings_scroll_by(-1e6);
    let keys = app.keys_layout();
    let last = keys.rows.last().unwrap().0;
    assert!(last.bottom() <= keys.list.bottom());
    assert!(app.settings.scroll > 0.0);
}

#[test]
fn settings_scales_in_and_fades_out() {
    use crate::app::animation::{Clock, DIALOG_IN, DIALOG_OUT};

    let dir = search_folder("settings-motion");
    let mut app = app_with(&dir, true);
    app.open_settings();
    let rest = app.settings_layout().card;

    // 出现途中：卡片整张淡入、放大（缩放只作用于绘制），点击仍按原位置命中。第一帧照常绘制
    // 并保存快照，之后的帧直接使用快照。
    app.settings.motion = Some((true, Clock::frozen(DIALOG_IN / 3, DIALOG_IN)));
    app.settings.cache = None;
    assert_eq!(app.settings_layout().card, rest);
    assert!(app.animating());
    paint(&mut app);
    assert!(app.settings.cache.is_some());
    let mut renderer = app.renderer.take().unwrap();
    let mut cached = false;
    renderer.draw(|frame| cached = crate::view::tests::settings_draw_cached(&mut app, frame));
    app.renderer = Some(renderer);
    assert!(cached);

    // 关闭：键盘和鼠标立即回到主界面，浮层继续绘制淡出，结束后才移除。
    app.close_settings();
    assert!(!app.settings.open);
    app.settings.motion = Some((false, Clock::frozen(DIALOG_OUT / 2, DIALOG_OUT)));
    assert_eq!(app.settings_layout().card, rest);
    assert!(app.animating());
    paint(&mut app);
    app.settings.motion = Some((false, Clock::frozen(DIALOG_OUT, DIALOG_OUT)));
    app.settle_animations();
    assert!(app.settings.motion.is_none());
    assert!(app.settings.cache.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：设置浮层出现到四分之一（整张淡入、放大）；「模式」从跟随系统换到深色，
/// 凸起的那一段滑到一半。
#[test]
#[ignore]
fn shoot_settings_motion() {
    use crate::app::animation::{Clock, DIALOG_IN, VIEW_SLIDE};

    let dir = search_folder("settings-motion-shot");
    let mut app = app_with(&dir, true);
    app.open_settings();
    app.settings.motion = Some((true, Clock::frozen(DIALOG_IN / 4, DIALOG_IN)));
    shoot(&mut app, "settings-opening");
    app.settings.motion = None;
    app.theme_choice = ThemeChoice::Dark;
    app.settings.mode_slide = Some((ThemeChoice::Auto, Clock::frozen(VIEW_SLIDE / 4, VIEW_SLIDE)));
    shoot(&mut app, "settings-mode-slide");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 测量：设置浮层出现动画途中绘制一帧的耗时（与不在动画中时对比）。
///
/// `paint` 每次绘制两帧，实际一帧的耗时是打印值的一半。运行
/// `cargo test --release -- --ignored measure_settings_motion --nocapture` 查看结果。
#[test]
#[ignore]
fn measure_settings_motion() {
    use crate::app::animation::{Clock, DIALOG_IN};
    use std::time::Instant;

    let dir = search_folder("settings-motion-measure");
    let mut app = app_with(&dir, true);
    app.open_settings();
    let time = |app: &mut App| {
        paint(app);
        let start = Instant::now();
        for _ in 0..20 {
            paint(app);
        }
        start.elapsed() / 20
    };
    let still = time(&mut app);
    app.settings.motion = Some((true, Clock::frozen(DIALOG_IN / 3, DIALOG_IN)));
    let moving = time(&mut app);
    println!("静止 {still:?}，动画中 {moving:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_about_page_shows_the_version_and_the_update_state() {
    use crate::winc::UpdateState;

    let dir = sample_folder("settings-about");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_settings();
    paint(&mut app);
    // 最后一页是「关于」：版本号、更新状态和「检查更新」、定时检查的开关（从「编辑器」页移过来）。
    assert_eq!(SettingsPage::ALL.last(), Some(&SettingsPage::About));
    click_part(&mut app, SettingsPart::Page(SettingsPage::About));
    paint(&mut app);
    let layout = app.settings_layout();
    for setting in [Setting::Version, Setting::Updates, Setting::CheckUpdates] {
        assert!(layout.row(setting).is_some(), "{setting:?}");
    }
    assert_eq!(Setting::CheckUpdates.page(), SettingsPage::About);
    // 版本号只读，点不到。
    let version = layout.row(Setting::Version).unwrap().control;
    let center = (version.x + version.w / 2.0, version.y + version.h / 2.0);
    assert_eq!(layout.part_at(center.0, center.1), None);

    // 当前版本不检查更新时按钮点了没有反应；能检查时点了交给 Sparkle（测试里不启用）。
    assert!(!app.can_check_updates());
    click_part(&mut app, SettingsPart::Action(Setting::Updates));
    assert_eq!(app.settings.selected, Setting::Updates);
    assert!(app.context_menu.is_none());
    *app.shared.update_state.borrow_mut() = UpdateState::Found("0.1.400".into());
    assert!(app.can_check_updates());
    *app.shared.update_state.borrow_mut() = UpdateState::Checking;
    assert!(!app.can_check_updates());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_font_can_be_chosen_from_the_menu() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};
    use crate::layout::{SettingsPart, font_menu_button};

    let dir = search_folder("settings-font-menu");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(Setting::EditorFont.page());
    paint(&mut app);

    // 点输入框右端的按钮弹出字体菜单；点其余位置仍是输入框。
    let row = *app.settings_layout().row(Setting::EditorFont).unwrap();
    let button = font_menu_button(row.control);
    let (x, y) = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    assert_eq!(
        app.settings_part_at(x, y),
        Some(SettingsPart::FontMenu(Setting::EditorFont))
    );
    assert_eq!(
        app.settings_part_at(button.x - 4.0, y),
        Some(SettingsPart::Field(Setting::EditorFont))
    );
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(
        app.context_menu.as_ref().map(|m| &m.target),
        Some(&MenuTarget::Font(Setting::EditorFont))
    );
    assert!(app.settings.editing.is_none());
    assert!(app.menu_checked(MenuCommand::FontDefault));
    app.context_menu = None;
    paint(&mut app);

    // 编辑器字体和界面字体都直接列出本机字族；编辑器把等宽字体排在前面。
    let fonts = winc_render::installed_fonts();
    let listed = |setting| {
        crate::app::input::context_menu::font_menu(setting)
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    crate::app::input::context_menu::MenuEntry::Item(MenuCommand::Font(_), _)
                )
            })
            .count()
    };
    assert_eq!(listed(Setting::UiFont), fonts.len());
    assert_eq!(listed(Setting::EditorFont), fonts.len());

    #[cfg(target_os = "macos")]
    {
        // 选中后立刻生效并写入配置，再选「系统默认」清除。
        let menlo = fonts.iter().position(|f| f.name == "Menlo").unwrap();
        app.settings_open_menu(Setting::EditorFont);
        app.run_menu_command(MenuCommand::Font(menlo));
        assert_eq!(app.config.editor_font.as_deref(), Some("Menlo"));
        assert_eq!(app.settings.error, None);

        app.settings_open_menu(Setting::EditorFont);
        assert!(app.menu_checked(MenuCommand::Font(menlo)));
        app.run_menu_command(MenuCommand::FontDefault);
        assert_eq!(app.config.editor_font, None);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_language_and_ui_font_controls_line_up() {
    let dir = search_folder("settings-language-width");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(Setting::Language.page());
    let layout = app.settings_layout();
    let control = |setting| layout.row(setting).unwrap().control;
    let (language, font) = (control(Setting::Language), control(Setting::UiFont));
    assert_eq!((language.x, language.w), (font.x, font.w));
    assert_eq!(Setting::Language.description(), None);
    // 与模式分段控件同宽。
    let theme = control(Setting::Theme);
    assert_eq!((language.x, language.w), (theme.x, theme.w));

    // 语言菜单比控件短，打开后与控件同宽并对齐。
    app.settings_open_menu(Setting::Language);
    paint(&mut app);
    let popup = app.context_menu.as_ref().unwrap().popup.as_ref().unwrap();
    let menu = popup.levels[0].rect;
    assert_eq!((menu.x, menu.w), (language.x, language.w));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn line_numbers_and_the_status_bar_can_be_hidden() {
    let dir = sample_folder("settings-chrome");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let (gutter, text, editor_bottom) = (
        app.layout.gutter.w,
        app.layout.text.w,
        app.layout.editor.bottom(),
    );
    assert!(!app.status_hits.is_empty());

    // 行号：行号栏变窄，文字区域变宽，折叠箭头和书签的空白还在。
    assert!(app.switch_on(Setting::LineNumbers));
    assert!(app.run_action(crate::keymap::Action::ToggleLineNumbers));
    assert!(!app.switch_on(Setting::LineNumbers));
    assert_eq!(app.config.line_numbers, Some(false));
    assert_eq!(app.layout.gutter.w, 2.0 * crate::theme::GUTTER_PADDING);
    assert!(app.layout.gutter.w < gutter);
    assert!(app.layout.text.w > text);
    paint(&mut app);

    // 状态栏：底部只留下边距，卡片变高，点不到状态栏上的项。
    assert!(app.switch_on(Setting::StatusBar));
    assert!(app.run_action(crate::keymap::Action::ToggleStatusBar));
    assert_eq!(app.config.status_bar, Some(false));
    assert_eq!(app.layout.status.h, crate::theme::ISLAND_MARGIN);
    assert!(app.layout.editor.bottom() > editor_bottom);
    paint(&mut app);
    assert!(app.status_hits.is_empty());
    assert!(app.crumb_hits.is_empty());

    // 短提示本来显示在状态栏上，隐藏后改到右上角。
    app.flash("copied");
    assert_eq!(app.status_message, None);
    assert_eq!(app.notice.as_deref(), Some("copied"));

    // 命令面板能把它们打开回来。
    assert!(app.run_action(crate::keymap::Action::ToggleStatusBar));
    assert!(app.run_action(crate::keymap::Action::ToggleLineNumbers));
    assert_eq!(app.layout.gutter.w, gutter);
    assert_eq!(app.layout.editor.bottom(), editor_bottom);
    paint(&mut app);
    assert!(!app.status_hits.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn turning_off_window_restore_forgets_the_saved_frame() {
    let dir = sample_folder("settings-window");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.state.window = Some(winc_config::WindowFrame {
        position: Some((10.0, 20.0)),
        width: 900.0,
        height: 600.0,
        scale: 1.0,
        maximized: false,
    });
    assert!(app.switch_on(Setting::RestoreWindow));
    app.settings_toggle(Setting::RestoreWindow);
    assert_eq!(app.config.restore_window, Some(false));
    assert_eq!(app.state.window, None);

    std::fs::remove_dir_all(&dir).unwrap();
}
