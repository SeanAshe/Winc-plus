//! 「快捷键」页：在「键位方案」菜单中切换方案和恢复全部默认，点击一行或「编辑」展开修改快捷键
//! （去掉已有的键、加上新键、冲突说明、保存与放弃），恢复默认、右键菜单，以及采用导入的快捷键。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::keymap::{Action, Context, Custom, KeyCode, Lookup, Mods, Os, Preset, Stroke};

/// 主修饰键：macOS 上是 ⌘，其他系统上是 Ctrl。
fn command() -> Mods {
    if cfg!(target_os = "macos") {
        Mods::META
    } else {
        Mods::CTRL
    }
}

fn key(key: KeyCode, mods: Mods) -> Stroke {
    Stroke { key, mods }
}

/// 在编辑区按下 `stroke` 时第一个要执行的命令。
fn runs(stroke: Stroke) -> Option<Action> {
    match crate::keymap::lookup(&crate::keymap::current(), stroke, None, Context::Editor) {
        Lookup::Run(actions) => actions.first().copied(),
        _ => None,
    }
}

/// 把「快捷键」页中命令 `action` 的一行滚进列表中间，并把鼠标移到这一行上。
pub(super) fn show_row(app: &mut App, action: Action) {
    let keys = app.keys_layout();
    let (rect, _) = keys.rows().find(|(_, row)| row.action == action).unwrap();
    let to = app.settings.scroll + rect.y - keys.list.y - keys.list.h / 2.0;
    app.settings.scroll = to.clamp(0.0, keys.max_scroll());
    let point = settings_point(app, SettingsPart::KeyRow(action));
    app.on_mouse_move(point.0, point.1);
    paint(app);
}

/// 打开设置的「快捷键」页。
fn keys_page() -> App {
    crate::keymap::install(Preset::Idea, &Custom::default());
    let mut app = blank_app(false);
    app.open_settings();
    click_part(&mut app, SettingsPart::Page(SettingsPage::Keys));
    paint(&mut app);
    app
}

/// 「键位方案」的值弹出菜单：各方案（当前的打勾）、恢复全部默认（没改过时不可选）。
#[test]
fn the_keymap_menu_switches_presets() {
    let mut app = keys_page();
    click_part(&mut app, SettingsPart::Keymap);
    let menu = app.overlays.context_menu.as_ref().expect("键位方案菜单");
    assert_eq!(menu.target, MenuTarget::Keymap);
    let entries = menu_commands(&app);
    assert_eq!(entries[0], (MenuCommand::KeymapPreset(Preset::Idea), true));
    assert_eq!(
        entries[1],
        (MenuCommand::KeymapPreset(Preset::VsCode), true)
    );
    assert_eq!(
        entries.last(),
        Some(&(MenuCommand::ResetAllShortcuts, false))
    );
    assert!(app.menu_checked(MenuCommand::KeymapPreset(Preset::Idea)));
    app.run_menu_command(MenuCommand::KeymapPreset(Preset::VsCode));
    assert_eq!(app.config.keymap.as_deref(), Some("vscode"));
    assert_eq!(crate::keymap::current_preset(), Preset::VsCode);
}

#[test]
fn shortcuts_are_edited_in_place() {
    let mut app = keys_page();

    // 单击一行展开，已有的键保留；按下的键不执行，与其他命令冲突时说明；保存后加在已有的键之后，
    // 其他命令让出该键。
    let d = key(KeyCode::Char('d'), command());
    assert_eq!(runs(d), Some(Action::CopyLinesDown));
    show_row(&mut app, Action::AddNextOccurrence);
    click_part(&mut app, SettingsPart::KeyRow(Action::AddNextOccurrence));
    let before = app.current_keys(Action::AddNextOccurrence);
    assert_eq!(app.settings.key_edit.as_ref().unwrap().keys, before);
    app.key_edit_stroke(d);
    assert_eq!(app.key_edit_conflicts(), [Action::CopyLinesDown]);
    paint(&mut app);
    click_part(&mut app, SettingsPart::KeySave);
    assert!(app.settings.key_edit.is_none());
    let mut expected = before.clone();
    expected.push((d, None));
    assert_eq!(app.current_keys(Action::AddNextOccurrence), expected);
    assert_eq!(runs(d), Some(Action::AddNextOccurrence));
    assert!(app.shortcut_customized(Action::AddNextOccurrence));

    // 悬停时点「编辑」；× 去掉已有的键，回车保存后该命令没有快捷键。Esc、「取消」、点在别处
    // 都放弃修改。
    let format = app.current_keys(Action::Format);
    show_row(&mut app, Action::Format);
    click_part(&mut app, SettingsPart::KeyEdit(Action::Format));
    paint(&mut app);
    click_part(&mut app, SettingsPart::KeyRemove(0));
    assert!(app.settings.key_edit.as_ref().unwrap().keys.is_empty());
    app.key_edit_stroke(key(KeyCode::Escape, Mods::NONE));
    assert_eq!(app.current_keys(Action::Format), format);
    click_part(&mut app, SettingsPart::KeyRow(Action::Format));
    paint(&mut app);
    click_part(&mut app, SettingsPart::KeyCancel);
    assert!(app.settings.key_edit.is_none());
    click_part(&mut app, SettingsPart::KeyRow(Action::Format));
    click_part(&mut app, SettingsPart::KeySearch);
    assert!(app.settings.key_edit.is_none());
    click_part(&mut app, SettingsPart::KeyRow(Action::Format));
    paint(&mut app);
    click_part(&mut app, SettingsPart::KeyRemove(0));
    app.key_edit_stroke(key(KeyCode::Enter, Mods::NONE));
    assert!(app.current_keys(Action::Format).is_empty());

    // 两段组合键按两下记下。
    show_row(&mut app, Action::Format);
    click_part(&mut app, SettingsPart::KeyRow(Action::Format));
    app.key_edit_stroke(key(KeyCode::Char('k'), command()));
    app.key_edit_stroke(key(KeyCode::Char('f'), command()));
    let recorded = app.settings.key_edit.as_ref().unwrap().recorded;
    assert!(recorded.is_some_and(|(_, second)| second.is_some()));
    app.cancel_key_edit();

    // 改过的行悬停时有「恢复默认」。
    show_row(&mut app, Action::Format);
    click_part(&mut app, SettingsPart::KeyReset(Action::Format));
    assert_eq!(app.current_keys(Action::Format), format);

    // 右键菜单：移除全部键；「键位方案」菜单中恢复全部默认。
    let point = settings_point(&app, SettingsPart::KeyRow(Action::Format));
    app.context_click(point.0, point.1);
    let menu = app.overlays.context_menu.as_ref().expect("右键菜单");
    assert_eq!(menu.target, MenuTarget::Shortcut(Action::Format));
    app.run_menu_command(MenuCommand::ShortcutRemove);
    assert!(app.current_keys(Action::Format).is_empty());
    click_part(&mut app, SettingsPart::Keymap);
    app.run_menu_command(MenuCommand::ResetAllShortcuts);
    assert!(app.config.shortcuts.is_empty());
    assert_eq!(runs(d), Some(Action::CopyLinesDown));

    // 采用导入的结果：换方案，改过的键整组换成导入的。
    let mut custom = Custom::default();
    let e = key(KeyCode::Char('e'), command());
    custom.set(Action::QuickOpen, vec![(e, None)]);
    let imported = crate::keymap::import::Imported {
        preset: Preset::VsCode,
        custom,
        skipped: 0,
    };
    app.adopt_imported_keymap(&imported);
    assert_eq!(app.config.keymap.as_deref(), Some("vscode"));
    assert_eq!(runs(e), Some(Action::QuickOpen));
    assert_eq!(
        app.config.shortcuts.get("quickOpen"),
        Some(&vec![crate::keymap::binding_spec(e, None, Os::CURRENT)])
    );
}

/// 保存时与新键冲突的改过的命令在配置中让出该键；不带修饰键的新键提醒会取代文字输入；展开
/// 列表底部的一行时列表滚动到整行可见。
#[test]
fn saving_a_key_takes_it_from_changed_commands() {
    let mut app = keys_page();
    let j = key(KeyCode::Char('j'), command() | Mods::SHIFT);
    app.change_shortcuts(|custom| custom.set(Action::Format, vec![(j, None)]));
    show_row(&mut app, Action::GoToLine);
    click_part(&mut app, SettingsPart::KeyRow(Action::GoToLine));
    app.key_edit_stroke(j);
    assert_eq!(app.key_edit_conflicts(), [Action::Format]);
    assert!(!app.key_edit_takes_typing());
    app.key_edit_stroke(key(KeyCode::Enter, Mods::NONE));
    assert_eq!(app.config.shortcuts.get("format"), Some(&Vec::new()));
    assert_eq!(runs(j), Some(Action::GoToLine));

    app.start_key_edit(Action::GoToLine, true);
    app.key_edit_stroke(key(KeyCode::Char('q'), Mods::NONE));
    assert!(app.key_edit_takes_typing());
    app.cancel_key_edit();

    // 列表底部的一行展开后整行可见。
    let keys = app.keys_layout();
    app.settings.scroll = keys.max_scroll();
    let last = keys.rows().last().unwrap().1.action;
    let keys = app.keys_layout();
    let (rect, _) = keys.rows().find(|(_, row)| row.action == last).unwrap();
    app.settings.scroll -= keys.list.bottom() - rect.bottom() + 1.0;
    app.start_key_edit(last, true);
    let keys = app.keys_layout();
    let (rect, _) = keys.rows().find(|(_, row)| row.action == last).unwrap();
    assert!(
        rect.bottom() <= keys.list.bottom() + 0.5,
        "{rect:?} {:?}",
        keys.list
    );
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_keys_page() {
    for dark in [false, true] {
        crate::keymap::install(Preset::Idea, &Custom::default());
        let mut app = blank_app(dark);
        app.open_settings();
        click_part(&mut app, SettingsPart::Page(SettingsPage::Keys));
        paint(&mut app);
        let mut custom = Custom::default();
        custom.set(
            Action::GoToLine,
            vec![(key(KeyCode::Char('g'), command()), None)],
        );
        app.change_shortcuts(|c| *c = custom);
        show_row(&mut app, Action::GoToLine);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("keys-hover-{theme}"));
        show_row(&mut app, Action::QuickOpen);
        click_part(&mut app, SettingsPart::KeyRow(Action::QuickOpen));
        app.key_edit_stroke(key(KeyCode::Char('d'), command()));
        shoot(&mut app, &format!("keys-edit-{theme}"));
        // 展开到一半的一帧。
        let at = std::time::Duration::from_millis(40);
        let clock =
            crate::app::animation::Clock::stopped(crate::app::animation::TREE_ANIMATION, at);
        app.settings.key_slide = Some((None, clock));
        shoot(&mut app, &format!("keys-slide-{theme}"));
        app.settings.key_slide = None;
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_setup_keys() {
    use crate::keymap::import::{Editor, Source};
    for dark in [false, true] {
        let mut app = blank_app(dark);
        app.state.setup_pending = true;
        app.setup_next();
        app.setup.sources = vec![
            Source {
                kind: Editor::Idea,
                name: "IntelliJ IDEA 2025.3".to_owned(),
                dir: std::env::temp_dir(),
            },
            Source {
                kind: Editor::VsCode,
                name: "VS Code".to_owned(),
                dir: std::env::temp_dir(),
            },
        ];
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("setup-keys-{theme}"));
    }
}

/// 回归：修改快捷键后保存或放弃，键盘焦点回到页内的搜索框，接着打字、`↓` 不跳去设置的搜索框
/// 或别的页。
#[test]
fn finishing_a_key_edit_returns_to_the_search_box() {
    let mut app = keys_page();
    assert_eq!(app.focus, Focus::KeySearch);
    app.start_key_edit(Action::GoToLine, true);
    assert_eq!(app.focus, Focus::Settings);
    app.key_edit_stroke(key(KeyCode::Escape, Mods::NONE));
    assert_eq!(app.focus, Focus::KeySearch);

    app.start_key_edit(Action::GoToLine, true);
    app.key_edit_stroke(key(KeyCode::Enter, Mods::NONE));
    assert_eq!(app.focus, Focus::KeySearch);
    assert_eq!(app.settings_layout().page, SettingsPage::Keys);
}
