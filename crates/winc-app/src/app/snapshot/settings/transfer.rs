//! 通用页的「设置文件」一组：导出配置文件，以及导入导出的文件替换当前的设置。

use super::*;
use crate::app::workspace::files::Pick;
use crate::keymap::Action;

/// 返回用 `dir` 下的 `config.json` 作为配置文件的窗口。
fn app_with_config(dir: &Path) -> App {
    let mut app = app_with(dir, false);
    app.store.config_path = Some(dir.join("config.json"));
    app
}

#[test]
fn the_general_page_offers_export_and_import() {
    let dir = search_folder("settings-transfer-rows");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(SettingsPage::General);
    let layout = app.settings_layout();
    for setting in [Setting::ExportSettings, Setting::ImportSettings] {
        assert!(layout.row(setting).is_some(), "{setting:?}");
        assert_eq!(setting.control(), SettingControl::Button);
        assert_eq!(setting.key(), None, "不是配置中的一项，没有「恢复默认」");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exported_settings_are_the_config_file() {
    let dir = search_folder("settings-transfer-export");
    let mut app = app_with_config(&dir);
    app.settings_toggle(Setting::WordWrap);
    let to = dir.join("exported.json");
    app.on_picked(Pick::ExportSettings, Some(to.clone()));
    let config = std::fs::read_to_string(dir.join("config.json")).unwrap();
    assert_eq!(std::fs::read_to_string(&to).unwrap(), config);
    assert_eq!(
        app.status.message.as_deref(),
        Some("已导出设置到「exported.json」")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 导入后立即改用文件中的设置和快捷键，原配置另存为备份；之后在设置页中的修改写在导入的配置上。
#[test]
fn imported_settings_replace_the_current_ones() {
    let dir = search_folder("settings-transfer-import");
    let mut app = app_with_config(&dir);
    app.settings_toggle(Setting::WordWrap);
    let old = std::fs::read_to_string(dir.join("config.json")).unwrap();
    let paste = crate::keymap::shortcut(Action::Paste);
    let from = dir.join("settings.json");
    let text = r#"{
  // 换一台设备上的设置
  "editor": { "size": 18 },
  "keyboard": { "shortcuts": { "paste": ["ctrl+alt+v"] } }
}
"#;
    std::fs::write(&from, text).unwrap();

    app.on_picked(Pick::ImportSettings, Some(from));
    assert_eq!(app.config.editor_size, Some(18.0));
    assert_eq!(app.config.word_wrap, None);
    assert_eq!(app.fonts.code, 18.0);
    assert_ne!(crate::keymap::shortcut(Action::Paste), paste);
    assert_eq!(
        std::fs::read_to_string(dir.join("config.json")).unwrap(),
        text
    );
    let backup = dir.join("config.json.bak");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), old);
    assert_eq!(app.notice.kind, NoticeKind::Info);
    let notice = app.notice.text.clone().unwrap();
    assert!(notice.contains(&backup.display().to_string()), "{notice}");

    app.settings_toggle(Setting::WordWrap);
    let (config, _) = Config::load_from(&dir.join("config.json"));
    assert_eq!(
        (config.editor_size, config.word_wrap),
        (Some(18.0), Some(true))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unrecognized_items_are_ignored_and_reported() {
    let dir = search_folder("settings-transfer-ignored");
    let mut app = app_with_config(&dir);
    let from = dir.join("settings.json");
    std::fs::write(&from, r#"{"editor": {"size": 18, "unknown": 1}}"#).unwrap();
    app.on_picked(Pick::ImportSettings, Some(from));
    assert_eq!(app.config.editor_size, Some(18.0));
    assert_eq!(app.notice.kind, NoticeKind::Warning);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("已导入设置，1 项无法识别，已忽略")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_is_not_a_settings_file_changes_nothing() {
    let dir = search_folder("settings-transfer-invalid");
    let mut app = app_with_config(&dir);
    app.settings_toggle(Setting::WordWrap);
    let config = app.config.clone();
    let old = std::fs::read_to_string(dir.join("config.json")).unwrap();
    let from = dir.join("notes.json");
    std::fs::write(&from, "[1, 2, 3]").unwrap();
    app.on_picked(Pick::ImportSettings, Some(from));
    assert_eq!(app.config, config);
    assert_eq!(
        std::fs::read_to_string(dir.join("config.json")).unwrap(),
        old
    );
    assert_eq!(app.notice.kind, NoticeKind::Error);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("「notes.json」不是有效的设置文件")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 截图：通用页末尾的「设置文件」一组。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_settings_file_group() {
    let dir = search_folder("settings-transfer-shot");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(SettingsPage::General);
    let layout = app.settings_layout();
    let row = layout.row(Setting::ImportSettings).unwrap().row;
    app.settings.scroll += (row.bottom() - layout.body.bottom() + 40.0).max(0.0);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(500));
    shoot(&mut app, "settings-file-group");
    std::fs::remove_dir_all(&dir).unwrap();
}
