//! 设置分类栏顶部搜索框的测试：找出各页中名称或说明含有关键字的设置项，键盘和点击进出搜索结果。

use super::*;

/// 把焦点移到搜索框，把其中的文字换成 `query`。
fn search(app: &mut App, query: &str) {
    app.focus_settings_search();
    app.edit_field(|f| {
        f.set_text(query);
        true
    });
}

/// 返回当前列出的设置项和各组标题，不含「快捷键」组（见
/// [`search_reaches_the_keymap_and_command_shortcuts`]）。
fn listed(app: &App) -> (Vec<Setting>, Vec<String>) {
    let layout = app.settings_layout();
    let keys = |s: &Setting| s.page() == SettingsPage::Keys;
    let rows = (layout.rows.iter()).map(|r| r.setting).filter(|s| !keys(s));
    let titles = (layout.headings.iter())
        .map(|(_, title)| title.to_string())
        .filter(|title| title != SettingsPage::Keys.title());
    (rows.collect(), titles.collect())
}

#[test]
fn search_finds_settings_on_every_page() {
    let dir = search_folder("settings-search");
    let mut app = app_with(&dir, false);
    app.config.terminal_enabled = Some(true);
    app.open_settings();

    // 名称含「字号」的设置项分布在外观、编辑器、Markdown、终端四页：每页一组，以页名为标题，
    // 选中第一条。
    search(&mut app, "字号");
    assert!(app.settings_layout().searching);
    let (rows, titles) = listed(&app);
    let sizes = [
        Setting::UiSize,
        Setting::EditorSize,
        Setting::MarkdownSize,
        Setting::TerminalSize,
    ];
    assert_eq!(rows, sizes);
    assert_eq!(titles, ["外观", "编辑器", "Markdown", "终端"]);
    assert_eq!(app.settings.selected, Setting::UiSize);

    // 说明也参与匹配，不区分大小写：「lint」只出现在「保存时修复问题」的说明中。
    search(&mut app, "LINT");
    assert_eq!(listed(&app).0, [Setting::FixOnSave]);

    // 主题的各项都在「外观」之下。
    search(&mut app, "主题");
    let (rows, titles) = listed(&app);
    assert!(rows.contains(&Setting::LightTheme) && rows.contains(&Setting::DarkTheme));
    assert_eq!(titles, ["外观"]);

    // 组标题也参与匹配：按「Vim」一次搜到这一组的全部设置项。
    search(&mut app, "vim");
    let (rows, titles) = listed(&app);
    assert_eq!(
        rows,
        [
            Setting::VimMode,
            Setting::VimClipboard,
            Setting::VimRelativeNumber,
            Setting::VimYankHighlight
        ]
    );
    assert_eq!(titles, ["编辑器"]);
    // 页名也参与匹配：搜到的是该页的全部设置项。
    search(&mut app, "markdown");
    let (rows, titles) = listed(&app);
    let page = SettingsPage::Markdown.sections(&Default::default());
    let all: Vec<Setting> = page.into_iter().flat_map(|section| section.items).collect();
    assert_eq!(rows, all);
    assert_eq!(titles, ["Markdown"]);

    // 没有匹配时正文为空，键盘移动不做任何事。
    search(&mut app, "没有这一项");
    assert!(listed(&app).0.is_empty());
    app.settings_move(true);

    // 终端模块关闭时不列出终端页的设置项。
    app.config.terminal_enabled = Some(false);
    search(&mut app, "字号");
    assert_eq!(
        listed(&app).0,
        [Setting::UiSize, Setting::EditorSize, Setting::MarkdownSize]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keyboard_and_clicks_move_in_and_out_of_search() {
    let dir = search_folder("settings-search-keys");
    let mut app = app_with(&dir, false);
    app.config.terminal_enabled = Some(true);
    app.open_settings();

    // 点击搜索框：焦点移到搜索框。
    click_part(&mut app, SettingsPart::SearchField);
    assert_eq!(app.focus, Focus::SettingsSearch);

    // ↓ 进入结果：焦点回到浮层，选中第一条，上下在结果之间移动，到头后不再移动。
    search(&mut app, "字号");
    app.settings_enter_results();
    assert_eq!(app.focus, Focus::Settings);
    assert!(app.settings.keyboard);
    for _ in 0..5 {
        app.settings_move(true);
    }
    assert_eq!(app.settings.selected, Setting::KeyMatches);
    app.settings_move(false);
    assert_eq!(app.settings.selected, Setting::TerminalSize);
    // 结果就是该设置项本身，可以直接调整。
    app.settings_step(true);
    assert_eq!(app.config.terminal_size, Some(CODE_FONT + 1.0));

    // 清空搜索框：回到选中项所在的页，仍选中它。
    app.settings_move(false);
    app.settings_move(false);
    search(&mut app, "");
    let layout = app.settings_layout();
    assert!(!layout.searching);
    assert_eq!(layout.page, SettingsPage::Editor);
    assert_eq!(app.settings.selected, Setting::EditorSize);

    // 搜索时点击分类栏中的一页：离开搜索结果，显示该页。在快捷键页搜索时正文同样是结果。
    app.settings_show_page(SettingsPage::Keys);
    search(&mut app, "字号");
    assert!(app.settings_layout().searching);
    click_part(&mut app, SettingsPart::Page(SettingsPage::Format));
    let layout = app.settings_layout();
    assert!(!layout.searching);
    assert_eq!(layout.page, SettingsPage::Format);
    assert!(app.settings.search.text().is_empty());

    // 重新打开设置时搜索框为空。
    search(&mut app, "字号");
    app.close_settings();
    app.open_settings();
    assert!(!app.settings_layout().searching);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 取值与默认不同的项在名称后面有「恢复默认」，点击后去掉配置文件中的这一项；写了值但与默认
/// 相同的项没有。
#[test]
fn changed_settings_can_be_reset() {
    let dir = search_folder("settings-reset");
    let mut app = app_with(&dir, false);
    app.config.editor_size = Some(17.0);
    app.config.minimap = Some(false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Editor);
    assert_eq!(app.settings.changed, [Setting::EditorSize]);
    app.settings_toggle(Setting::WordWrap);
    assert_eq!(
        app.settings.changed,
        [Setting::EditorSize, Setting::WordWrap]
    );
    paint(&mut app);
    click_part(&mut app, SettingsPart::Reset(Setting::WordWrap));
    assert!(!app.switch_on(Setting::WordWrap) && app.config.word_wrap.is_none());
    // 其余各项不变。
    assert_eq!(app.config.editor_size, Some(17.0));
    assert_eq!(app.settings.changed, [Setting::EditorSize]);
    paint(&mut app);
    assert_eq!(app.hits.reset_links.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 一组里的各项要么都写说明，要么都不写：各行等高。
#[test]
fn rows_of_a_group_are_all_described_or_none() {
    for page in SettingsPage::ALL {
        for section in page.sections(&Default::default()) {
            let rows = (section.items.iter()).filter(|s| s.control() != SettingControl::Action);
            let described = rows.filter(|s| s.description().is_some()).count();
            let all = (section.items.iter())
                .filter(|s| s.control() != SettingControl::Action)
                .count();
            assert!(
                described == 0 || described == all,
                "{page:?} {:?}",
                section.title
            );
        }
    }
}

/// 每个开关在配置文件中的名字与它读写的是同一项：打开或关闭后，这个名字在配置里有值。
#[test]
fn every_switch_is_named_after_the_config_entry_it_writes() {
    let modules = crate::layout::Modules {
        git: true,
        terminal: true,
    };
    for setting in Setting::all(&Default::default(), modules) {
        let Some(key) = setting.key() else { continue };
        if setting.control() == SettingControl::Switch {
            let mut app = blank_app(false);
            app.settings_toggle(setting);
            assert!(app.config.is_set(key), "{setting:?} 写的不是 {key}");
        }
    }
}

/// 回归：搜索设置不列出快捷键。搜「快捷键」列出键位方案，搜命令名列出相符的命令，点击后在
/// 「快捷键」页中查看，该页的搜索框换成同一段文字。
#[test]
fn search_reaches_the_keymap_and_command_shortcuts() {
    crate::keymap::install(
        crate::keymap::Preset::Idea,
        &crate::keymap::Custom::default(),
    );
    let mut app = blank_app(false);
    app.open_settings();

    let rows = |app: &App| -> Vec<Setting> {
        let layout = app.settings_layout();
        layout.rows.iter().map(|r| r.setting).collect()
    };
    search(&mut app, "快捷键");
    assert!(rows(&app).contains(&Setting::Keymap), "{:?}", rows(&app));
    let layout = app.settings_layout();
    assert!(layout.headings.iter().any(|(_, t)| *t == "快捷键"));

    search(&mut app, "转到行");
    assert_eq!(rows(&app), [Setting::KeyMatches]);
    assert!(crate::keymap::count_matches("转到行") > 0);
    app.settings_open_menu(Setting::KeyMatches);
    assert!(!app.settings_searching());
    assert_eq!(app.settings_layout().page, SettingsPage::Keys);
    assert_eq!(app.settings.keys_query.text(), "转到行");
    assert_eq!(app.focus, Focus::KeySearch);
}

/// 回归：只是输入关键字预览结果，清空搜索框后回到原来的页；搜索框为空时按 `↓` 留在当前页，
/// 不跳到第一页的第一项。
#[test]
fn clearing_a_search_returns_to_the_page_it_started_from() {
    let mut app = blank_app(false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Editor);
    let start = app.settings.selected;

    search(&mut app, "字号");
    assert_eq!(app.settings.selected, Setting::UiSize);
    search(&mut app, "");
    assert_eq!(app.settings.selected, start);
    assert_eq!(app.settings_layout().page, SettingsPage::Editor);

    app.focus_settings_search();
    app.settings_enter_results();
    assert_eq!(app.settings_layout().page, SettingsPage::Editor);
    assert_eq!(app.focus, Focus::Settings);

    // 进入结果后再清空：停在选中的结果所在的页。
    search(&mut app, "字号");
    app.settings_enter_results();
    search(&mut app, "");
    assert_eq!(app.settings_layout().page, SettingsPage::Appearance);
}

/// 搜索结果中「快捷键」组的截图：`cargo test -p winc-app shoot_search_shortcuts -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_search_shortcuts() {
    crate::keymap::install(
        crate::keymap::Preset::Idea,
        &crate::keymap::Custom::default(),
    );
    let mut app = blank_app(false);
    app.open_settings();
    for (query, name) in [
        ("保存", "settings-search-save"),
        ("快捷键", "settings-search-keys"),
    ] {
        search(&mut app, query);
        shoot(&mut app, name);
    }
}
