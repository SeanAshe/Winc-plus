//! 设置里「终端」一页的测试：默认 shell、位置、字体和字号、开关、Meta 键。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::app::terminal::shells::ShellChoice;

/// Git Bash 的启动命令，与检测到的写法相同。
const GIT_BASH: &str = r#""C:\Program Files\Git\bin\bash.exe" --login -i"#;

/// 打开设置的终端页；检测到的 shell 换成固定的两个，结果不随测试机器变化。
fn terminal_settings(name: &str) -> (App, PathBuf) {
    let dir = search_folder(name);
    let mut app = app_with(&dir, false);
    app.config.terminal_enabled = Some(true);
    app.open_settings();
    app.terminal.shells = vec![
        ShellChoice {
            name: "zsh".to_owned(),
            command: "/bin/zsh".to_owned(),
        },
        ShellChoice {
            name: "Git Bash".to_owned(),
            command: GIT_BASH.to_owned(),
        },
    ];
    app.settings_show_page(SettingsPage::Terminal);
    paint(&mut app);
    (app, dir)
}

#[test]
fn the_default_shell_is_chosen_from_the_detected_shells_or_typed() {
    let (mut app, dir) = terminal_settings("settings-terminal-shell");
    assert_eq!(app.settings_layout().page, SettingsPage::Terminal);
    assert_eq!(app.settings.selected, Setting::TerminalShell);

    // 关闭再打开终端模块后，检测到的 shell 仍然列出。
    app.settings_toggle(Setting::TerminalModule);
    app.settings_toggle(Setting::TerminalModule);
    assert_eq!(app.terminal.shells.len(), 2);

    // 没有设置时是系统默认；检测到的 shell 以启动命令为副标题。
    app.settings_open_menu(Setting::TerminalShell);
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| &m.target),
        Some(&MenuTarget::DefaultShell)
    );
    assert!(app.menu_checked(MenuCommand::DefaultShell(None)));
    let label = app.menu_label(MenuCommand::DefaultShell(None));
    assert_eq!(
        (label.title.as_str(), label.subtitle.as_deref()),
        ("系统默认", None)
    );
    let label = app.menu_label(MenuCommand::DefaultShell(Some(1)));
    assert_eq!(label.title, "Git Bash");
    assert_eq!(label.subtitle.as_deref(), Some(GIT_BASH));

    // 选中 Git Bash：写入启动命令，显示检测到的名字，「+」菜单里排在最前且不重复。
    app.run_menu_command(MenuCommand::DefaultShell(Some(1)));
    assert_eq!(app.config.terminal_shell.as_deref(), Some(GIT_BASH));
    assert_eq!(app.configured_shell().unwrap().name, "Git Bash");
    let names: Vec<String> = (app.terminal_shell_list().into_iter())
        .map(|shell| shell.name)
        .collect();
    assert_eq!(names, ["Git Bash", "zsh"]);
    app.settings_open_menu(Setting::TerminalShell);
    assert!(app.menu_checked(MenuCommand::DefaultShell(Some(1))));
    assert!(!app.menu_checked(MenuCommand::DefaultShellCustom));
    app.overlays.context_menu = None;

    // 自定义命令：输入框里是当前的命令，确认后写入；不在检测到的列表里时「自定义命令…」打勾。
    app.settings_open_menu(Setting::TerminalShell);
    app.run_menu_command(MenuCommand::DefaultShellCustom);
    let text = app.settings.editing.as_ref().map(|(_, field)| field.text());
    assert_eq!(text, Some(GIT_BASH));
    app.edit_field(|f| {
        f.select_all();
        f.insert("/usr/local/bin/fish -l")
    });
    paint(&mut app);
    app.settings_commit_field();
    assert_eq!(
        app.config.terminal_shell.as_deref(),
        Some("/usr/local/bin/fish -l")
    );
    assert!(app.settings.editing.is_none());
    app.settings_open_menu(Setting::TerminalShell);
    assert!(app.menu_checked(MenuCommand::DefaultShellCustom));

    // 选「系统默认」清除设置。
    app.run_menu_command(MenuCommand::DefaultShell(None));
    assert_eq!(app.config.terminal_shell, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_terminal_font_size_and_switches_write_the_config() {
    let (mut app, dir) = terminal_settings("settings-terminal-font");

    // 字号：未设置时显示默认字号；调整后只改终端，编辑区不变。
    let code = app.fonts.code;
    assert_eq!(app.setting_value(Setting::TerminalSize).0, code.to_string());
    app.settings_adjust(Setting::TerminalSize, true);
    assert_eq!(app.config.terminal_size, Some(code + 1.0));
    assert_eq!((app.fonts.terminal, app.fonts.code), (code + 1.0, code));
    assert_eq!(app.config.editor_size, None);

    // 字体：菜单的第一项是「跟随编辑器」，和编辑器字体一样先列出等宽字体。
    app.settings_open_menu(Setting::TerminalFont);
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| &m.target),
        Some(&MenuTarget::Font(Setting::TerminalFont))
    );
    assert_eq!(app.menu_label(MenuCommand::FontDefault).title, "跟随编辑器");
    assert!(app.menu_checked(MenuCommand::FontDefault));
    app.overlays.context_menu = None;
    // 找不到的字体名：原因写在该行，取消后配置退回原值。
    app.settings_start_editing(Setting::TerminalFont);
    app.edit_field(|f| f.insert("Comic Sanz"));
    app.settings_commit_field();
    let error = app.settings.error.as_ref().map(|(setting, _)| *setting);
    assert_eq!(error, Some(Setting::TerminalFont));
    app.settings_cancel_field();
    assert_eq!(app.config.terminal_font, None);

    // 开关：选中时复制默认关，其余默认开。
    for (setting, on) in [
        (Setting::TerminalCopyOnSelect, false),
        (Setting::TerminalConfirmClose, true),
        (Setting::TerminalPrompt, true),
        (Setting::TerminalShellIntegration, true),
    ] {
        assert_eq!(app.switch_on(setting), on, "{setting:?}");
        app.settings_toggle(setting);
        assert_eq!(app.switch_on(setting), !on, "{setting:?}");
    }
    assert_eq!(app.config.terminal_copy_on_select, Some(true));
    assert_eq!(app.config.terminal_confirm_close, Some(false));
    assert_eq!(app.config.terminal_prompt, Some(false));
    assert_eq!(app.config.terminal_shell_integration, Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_position_and_meta_key_are_chosen_from_menus() {
    let (mut app, dir) = terminal_settings("settings-terminal-choices");

    app.settings_open_menu(Setting::TerminalPosition);
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| &m.target),
        Some(&MenuTarget::TerminalPosition)
    );
    assert!(app.menu_checked(MenuCommand::TerminalPosition(false)));
    app.run_menu_command(MenuCommand::TerminalPosition(true));
    assert_eq!(app.config.terminal_right, Some(true));
    assert!(app.terminal_right());

    // 「Meta 键」只在 macOS 上列出。
    let listed = (app.settings_layout().row(Setting::TerminalOptionAsMeta)).is_some();
    assert_eq!(listed, cfg!(target_os = "macos"));
    if listed {
        app.settings_open_menu(Setting::TerminalOptionAsMeta);
        assert!(app.menu_checked(MenuCommand::OptionAsMeta(0)));
        app.run_menu_command(MenuCommand::OptionAsMeta(2));
        assert_eq!(
            app.config.option_as_meta,
            Some(winc_config::OptionAsMeta::Both)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_terminal_page_is_listed_only_when_the_module_is_on() {
    let (mut app, dir) = terminal_settings("settings-terminal-module");
    let listed = |app: &App| {
        (app.settings_layout().pages.iter()).any(|&(page, _)| page == SettingsPage::Terminal)
    };
    assert!(listed(&app));

    // 配置文件中关闭终端模块（另一个窗口或手动修改）：不再列出该页，停在该页时改到模块页。
    let mut config = app.config.clone();
    config.terminal_enabled = Some(false);
    app.adopt_config(config);
    assert!(!listed(&app));
    assert_eq!(app.settings_layout().page, SettingsPage::Modules);
    // 键盘移动同样跳过该页：模块页第一项的上一项在格式化页。
    app.settings_move(false);
    assert_eq!(app.settings.selected.page(), SettingsPage::Format);

    // 在模块页重新开启后立即列出。
    app.settings_toggle(Setting::TerminalModule);
    assert!(listed(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 把「终端」一项的出现（`on`）或消失动画停在 `at`；消失时 `at` 从内容开始淡出时计时。
///
/// 测试中没有窗口，`track_modules` 不做动画，因此直接给时钟。
fn freeze_terminal_page(app: &mut App, on: bool, at: std::time::Duration) {
    use crate::app::animation::{Clock, MODULE_FADE};

    let fx = &mut app.motion.module_fx.terminal;
    fx.track(!on, None);
    fx.track(on, Some(Clock::frozen(at, MODULE_FADE)));
}

#[test]
fn the_terminal_page_slides_in_and_out_with_the_module() {
    use crate::app::animation::{EXIT_FADE, MODULE_FADE};
    use crate::theme::SETTINGS_NAV_ROW;

    let dir = search_folder("settings-terminal-slide");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Modules);
    let row = |app: &App, page: SettingsPage| {
        let pages = app.settings_layout().pages;
        pages.into_iter().find(|&(p, _)| p == page).map(|(_, r)| r)
    };
    let modules = row(&app, SettingsPage::Modules).unwrap();
    assert!(row(&app, SettingsPage::Terminal).is_none());

    // 开启：「终端」一项从零高度展开，模块页随之下移，内容尚未完全显示；结束时是整行高度。
    app.settings_toggle(Setting::TerminalModule);
    freeze_terminal_page(&mut app, true, MODULE_FADE / 2);
    let terminal = row(&app, SettingsPage::Terminal).unwrap();
    assert!(terminal.h > 0.0 && terminal.h < SETTINGS_NAV_ROW);
    let moved = row(&app, SettingsPage::Modules).unwrap();
    assert!(moved.y > modules.y && moved.y < modules.y + SETTINGS_NAV_ROW + 2.0);
    assert!(app.motion.module_fx.terminal.opacity(true) < 1.0);
    freeze_terminal_page(&mut app, true, MODULE_FADE);
    assert_eq!(
        row(&app, SettingsPage::Terminal).unwrap().h,
        SETTINGS_NAV_ROW
    );

    // 关闭：内容先在整行高度内淡出，之后高度收起；淡出途中点击该项不切换过去。
    app.settings_toggle(Setting::TerminalModule);
    freeze_terminal_page(&mut app, false, EXIT_FADE / 2);
    assert_eq!(
        row(&app, SettingsPage::Terminal).unwrap().h,
        SETTINGS_NAV_ROW
    );
    let opacity = app.motion.module_fx.terminal.opacity(false);
    assert!(opacity > 0.0 && opacity < 1.0);
    app.settings_show_page(SettingsPage::Terminal);
    assert_eq!(app.settings_layout().page, SettingsPage::Modules);
    freeze_terminal_page(&mut app, false, EXIT_FADE + MODULE_FADE);
    assert!(row(&app, SettingsPage::Terminal).is_none());
    assert_eq!(row(&app, SettingsPage::Modules).unwrap(), modules);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：设置停在模块页，开启终端模块时「终端」一项展开、内容淡入途中
/// （`settings-terminal-slide-in-*`），以及关闭时内容淡出后高度收起途中
/// （`settings-terminal-slide-out-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_terminal_page_slide() {
    use crate::app::animation::{EXIT_FADE, MODULE_FADE};

    for (theme, dark) in [("light", false), ("dark", true)] {
        let dir = search_folder(&format!("settings-terminal-slide-{theme}"));
        let mut app = app_with(&dir, dark);
        app.open_settings();
        app.settings_show_page(SettingsPage::Modules);
        app.on_mouse_move(4.0, 4.0);
        app.settings_toggle(Setting::TerminalModule);
        freeze_terminal_page(&mut app, true, MODULE_FADE / 3);
        shoot(&mut app, &format!("settings-terminal-slide-in-{theme}"));
        app.settings_toggle(Setting::TerminalModule);
        freeze_terminal_page(&mut app, false, EXIT_FADE + MODULE_FADE / 2);
        shoot(&mut app, &format!("settings-terminal-slide-out-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
