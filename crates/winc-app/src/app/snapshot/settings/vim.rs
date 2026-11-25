//! 设置里 Vim 的「快捷键冲突」一组：列出 Vim 与编辑器都要用的 `Ctrl` 键，逐键改归属，写回
//! `vim.handleKeys`。

use super::*;
use crate::app::NoticeAction;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::keymap::Action;

/// 打开设置的编辑器页；编辑区的「粘贴」改为 `Ctrl+V`，使这个键在各系统上都与 Vim 冲突（macOS
/// 上的粘贴默认是 `⌘V`）。
fn vim_settings(name: &str) -> (App, PathBuf) {
    let dir = search_folder(name);
    let mut app = app_with(&dir, false);
    let paste = Action::Paste.id();
    app.config
        .shortcuts
        .insert(paste, vec!["ctrl+v".to_owned()]);
    let problems = crate::keymap::install_from(&app.config);
    assert!(problems.is_empty(), "{problems:?}");
    app.open_settings();
    app.settings_show_page(SettingsPage::Editor);
    (app, dir)
}

/// 编辑器页列出的「快捷键冲突」各行。
fn conflict_rows(app: &App) -> Vec<char> {
    let rows = app.settings_layout().rows;
    (rows.iter())
        .filter_map(|row| match row.setting {
            Setting::VimKey(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn conflicting_keys_are_listed_while_vim_mode_is_on() {
    let (mut app, dir) = vim_settings("settings-vim-keys-listed");
    assert!(conflict_rows(&app).is_empty(), "Vim 模式关闭时不列出");

    app.settings_toggle(Setting::VimMode);
    let rows = conflict_rows(&app);
    assert!(rows.contains(&'v'), "{rows:?}");
    // 说明这个键在编辑器中的命令；没有改过时由 Vim 处理。
    let paste = Setting::VimKey('v');
    let description = paste.description().unwrap();
    assert!(description.contains(Action::Paste.title()), "{description}");
    assert_eq!(app.choice_text(paste).0, "Vim");

    app.settings_toggle(Setting::VimMode);
    assert!(conflict_rows(&app).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_key_is_handed_to_the_editor_from_its_menu_and_reset() {
    let (mut app, dir) = vim_settings("settings-vim-keys-owner");
    app.settings_toggle(Setting::VimMode);
    let paste = Setting::VimKey('v');

    // 从菜单把 `Ctrl+V` 交给编辑器：写入 `vim.handleKeys`，这一行显示「恢复默认」。
    app.open_context_menu(MenuTarget::VimKey('v'), (0.0, 0.0));
    assert!(app.menu_checked(MenuCommand::VimKeyOwner(true)));
    app.run_menu_command(MenuCommand::VimKeyOwner(false));
    assert_eq!(app.config.vim_handle_keys, [("ctrl+v".to_owned(), false)]);
    assert!(!app.vim_owns_key('v'));
    assert_eq!(app.choice_text(paste).0, app_owner_label());
    assert!(app.settings.changed.contains(&paste));

    // 恢复默认：去掉这一项，又由 Vim 处理。
    app.settings_reset(paste);
    assert!(app.config.vim_handle_keys.is_empty());
    assert!(app.vim_owns_key('v'));
    assert!(!app.settings.changed.contains(&paste));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Vim 第一次接管编辑区也在用的键时提示一次，「打开设置」定位到这个键的一行；改写过归属的键
/// 不提示，之后也不再提示。
#[test]
fn the_first_conflicting_key_is_explained_once() {
    let (mut app, dir) = vim_settings("settings-vim-keys-notice");
    app.close_settings();
    app.settings_toggle(Setting::VimMode);
    let ctrl_v = winc_vim::Key::ctrl('v');

    // 归属写在配置里：用户自己定的，不提示。
    app.config.vim_handle_keys = vec![("ctrl+v".to_owned(), true)];
    app.vim_notice_key(ctrl_v);
    assert_eq!(app.notice.action, None);

    app.config.vim_handle_keys.clear();
    app.vim_notice_key(ctrl_v);
    assert_eq!(app.notice.action, Some(NoticeAction::VimKey('v')));
    assert!(app.state.vim_keys_noticed);
    app.run_notice_action();
    assert!(app.settings.open);
    assert_eq!(app.settings.selected, Setting::VimKey('v'));

    app.close_settings();
    app.vim_notice_key(ctrl_v);
    assert_eq!(app.notice.action, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 「快捷键」页中被 Vim 占用的一行有「查看冲突」，点击后定位到这个键在「快捷键冲突」中的一行。
#[test]
fn the_keys_page_links_a_taken_key_to_its_conflict_row() {
    let (mut app, dir) = vim_settings("settings-vim-keys-link");
    app.settings_toggle(Setting::VimMode);
    let label = crate::layout::ctrl_label('v').to_owned();
    assert_eq!(app.vim_taken_key(&[label]), Some('v'));

    // 悬停在「粘贴」一行上，出现链接后点击。
    app.settings_show_page(SettingsPage::Keys);
    paint(&mut app);
    super::shortcuts::show_row(&mut app, Action::Paste);
    click_part(&mut app, SettingsPart::KeyVim(Action::Paste, 'v'));
    assert_eq!(app.settings.selected, Setting::VimKey('v'));
    assert_eq!(app.settings.selected.page(), SettingsPage::Editor);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 「编辑器」一项的文字。
fn app_owner_label() -> &'static str {
    crate::layout::vim_key_owner_label(false)
}

/// 截图：编辑器页的 Vim 分组和下面的「快捷键冲突」、其中一行的菜单，以及「快捷键」页中被 Vim
/// 占用的一行悬停时的链接。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_vim_key_conflicts() {
    let (mut app, dir) = vim_settings("settings-vim-keys-shot");
    app.settings_toggle(Setting::VimMode);
    let layout = app.settings_layout();
    let top = layout.row(Setting::VimMode).unwrap().row.y - layout.body.y;
    app.settings.scroll = (app.settings.scroll + top - 40.0).max(0.0);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(500));
    shoot(&mut app, "settings-vim-keys");
    let layout = app.settings_layout();
    let row = layout.row(Setting::VimKey('v')).unwrap().row;
    app.settings.scroll += row.y - layout.body.y - layout.body.h / 2.0;
    paint(&mut app);
    let control = app
        .settings_layout()
        .row(Setting::VimKey('v'))
        .unwrap()
        .control;
    app.on_mouse_move(control.x + 4.0, control.y + 4.0);
    app.on_mouse_down(winit::event::MouseButton::Left);
    app.on_mouse_up(winit::event::MouseButton::Left);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(300));
    shoot(&mut app, "settings-vim-keys-menu");
    app.overlays.context_menu = None;
    app.settings_show_page(SettingsPage::Keys);
    paint(&mut app);
    super::shortcuts::show_row(&mut app, Action::Paste);
    std::thread::sleep(std::time::Duration::from_millis(300));
    shoot(&mut app, "settings-vim-keys-link");
    std::fs::remove_dir_all(&dir).unwrap();
}
