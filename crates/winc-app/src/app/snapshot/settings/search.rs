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

/// 返回当前列出的设置项和各组标题。
fn listed(app: &App) -> (Vec<Setting>, Vec<&'static str>) {
    let layout = app.settings_layout();
    let rows = layout.rows.iter().map(|r| r.setting).collect();
    let titles = layout.headings.iter().map(|&(_, title)| title).collect();
    (rows, titles)
}

#[test]
fn search_finds_settings_on_every_page() {
    let dir = search_folder("settings-search");
    let mut app = app_with(&dir, false);
    app.config.terminal_enabled = Some(true);
    app.open_settings();

    // 名称含「字号」的设置项分布在外观、编辑器、终端三页：按页分组，以页名为标题，选中第一条。
    search(&mut app, "字号");
    assert!(app.settings_layout().searching);
    let (rows, titles) = listed(&app);
    assert_eq!(
        rows,
        [Setting::UiSize, Setting::EditorSize, Setting::TerminalSize]
    );
    assert_eq!(titles, ["外观", "编辑器", "终端"]);
    assert_eq!(app.settings.selected, Setting::UiSize);

    // 说明也参与匹配，不区分大小写：「lint」只出现在「保存时修复问题」的说明中。
    search(&mut app, "LINT");
    assert_eq!(listed(&app).0, [Setting::FixOnSave]);

    // 有标题的组保留原标题：主题缩略图仍在「浅色主题」「深色主题」之下。
    search(&mut app, "主题");
    let (rows, titles) = listed(&app);
    assert!(rows.contains(&Setting::LightTheme) && rows.contains(&Setting::DarkTheme));
    assert!(titles.contains(&"浅色主题") && titles.contains(&"深色主题"));

    // 没有匹配时正文为空，键盘移动不做任何事。
    search(&mut app, "没有这一项");
    assert!(listed(&app).0.is_empty());
    app.settings_move(true);

    // 终端模块关闭时不列出终端页的设置项。
    app.config.terminal_enabled = Some(false);
    search(&mut app, "字号");
    assert_eq!(listed(&app).0, [Setting::UiSize, Setting::EditorSize]);
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
    app.settings_move(true);
    app.settings_move(true);
    app.settings_move(true);
    assert_eq!(app.settings.selected, Setting::TerminalSize);
    // 结果就是该设置项本身，可以直接调整。
    app.settings_step(true);
    assert_eq!(app.config.terminal_size, Some(CODE_FONT + 1.0));

    // 清空搜索框：回到选中项所在的页，仍选中它。
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
