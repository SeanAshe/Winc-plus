//! 截图的测试：设置浮层、快捷键一页、格式化一页，格式化出错和安装说明。

use super::*;

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_settings() {
    let dir = search_folder("settings-shot");

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.theme_choice = ThemeChoice::Auto;
        app.config.editor_font = Some("Menlo".to_owned());
        app.open_settings();
        paint(&mut app);

        // 第一页「外观」，鼠标停在分类栏的「编辑器」上。
        let (x, y) = settings_point(&app, SettingsPart::Page(SettingsPage::Editor));
        app.on_mouse_move(x, y);
        shoot(&mut app, &format!("settings-{tag}"));

        // 切换到「编辑器」页，鼠标停在字号的加号上。
        app.settings_show_page(SettingsPage::Editor);
        let (x, y) = settings_point(&app, SettingsPart::Increase(Setting::EditorSize));
        app.on_mouse_move(x, y);
        shoot(&mut app, &format!("settings-editor-{tag}"));

        // 用键盘移到 Tab 宽度一项上。
        for _ in 0..2 {
            app.settings_move(true);
        }
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("settings-keyboard-{tag}"));

        // 修改字体名，输入系统中不存在的名称。
        app.settings_start_editing(Setting::EditorFont);
        app.edit_field(|f| {
            f.select_all();
            f.insert("Comic Sanz")
        });
        app.settings_commit_field();
        shoot(&mut app, &format!("settings-error-{tag}"));

        // 离开输入框：输入退回原值，出错原因仍留在该行。
        app.settings_leave_field();
        shoot(&mut app, &format!("settings-error-left-{tag}"));

        // 窗口很矮时，卡片缩小到窗口以内，内容区域改为滚动。
        app.settings.error = None;
        app.renderer = Some(
            Renderer::offscreen(820.0, 420.0, SCALE, app.palette.frame_bg, None, None).unwrap(),
        );
        app.relayout();
        shoot(&mut app, &format!("settings-small-{tag}"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_keys_page() {
    use crate::keymap::{KeyCode, Mods, Stroke};

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = blank_app(dark);
        app.open_settings();
        app.settings_show_page(SettingsPage::Keys);
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("keys-{tag}"));

        // 按名称搜索。
        app.edit_field(|f| f.insert("路径"));
        shoot(&mut app, &format!("keys-search-{tag}"));

        // 录制按键：按下 ⌘K，列出以 ⌘K 开头的组合键。
        app.toggle_recording();
        app.keys_page_stroke(Stroke {
            key: KeyCode::Char('k'),
            mods: if cfg!(target_os = "macos") {
                Mods::META
            } else {
                Mods::CTRL
            },
        });
        shoot(&mut app, &format!("keys-record-{tag}"));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_settings_format() {
    for (tag, dark) in [("light", false), ("dark", true)] {
        let (mut app, dir) =
            crate::app::snapshot::settings::format_settings_app("settings-format-shot", dark);
        app.config.format_on_save = Some(true);
        app.open_settings();
        app.settings_show_page(SettingsPage::Format);
        paint(&mut app);

        // 鼠标停在「添加语言…」上。
        let (x, y) = settings_point(&app, SettingsPart::Action(Setting::AddLanguage));
        app.on_mouse_move(x, y);
        shoot(&mut app, &format!("settings-format-{tag}"));

        // Python 选择「自定义命令…」：按钮换成较宽的输入框，输入为空时以淡色显示填写提示。
        app.settings_open_menu(Setting::FormatCommand("python"));
        app.run_menu_command(MenuCommand::ToolCustom);
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("settings-format-edit-{tag}"));

        // 滚动到底：显示修复工具一组。
        app.settings_cancel_field();
        app.settings_scroll_by(-1000.0);
        shoot(&mut app, &format!("settings-fix-{tag}"));

        // 窗口很矮且滚动到中间：压在标题下沿和卡片底边上的行被裁剪，不会画到这两个边界之外。
        app.renderer = Some(
            Renderer::offscreen(820.0, 420.0, SCALE, app.palette.frame_bg, None, None).unwrap(),
        );
        app.relayout();
        app.settings.scroll = 0.0;
        app.settings_scroll_by(-150.0);
        shoot(&mut app, &format!("settings-format-scrolled-{tag}"));

        // 装了 Prettier：它能格式化的语言都自动列出来；再手动加 TOML（只有 Taplo，未安装）和
        // Groovy（没有预设）。修复工具一组仍只有项目在用的 Python，不为每种语言列一行「无」。
        use std::os::unix::fs::PermissionsExt;
        let prettier = dir.join("bin/prettier");
        std::fs::write(&prettier, "#!/bin/sh\ncat\n").unwrap();
        std::fs::set_permissions(&prettier, std::fs::Permissions::from_mode(0o755)).unwrap();
        app.renderer = Some(
            Renderer::offscreen(SIZE.0, SIZE.1, SCALE, app.palette.frame_bg, None, None).unwrap(),
        );
        app.close_settings();
        app.open_settings();
        app.settings_show_page(SettingsPage::Format);
        app.settings_add_language(Task::Format, winc_syntax::find("toml").unwrap());
        app.context_menu = None;
        app.settings_add_language(Task::Format, winc_syntax::find("groovy").unwrap());
        app.settings_cancel_field();
        app.settings_scroll_by(-10_000.0);
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("settings-format-many-{tag}"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_settings_terminal() {
    use crate::app::terminal::shells::ShellChoice;

    let dir = search_folder("settings-terminal-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.terminal_enabled = Some(true);
        app.open_settings();
        // 检测到的 shell 换成固定的几个，截图不随机器变化。
        let shell = |name: &str, command: &str| ShellChoice {
            name: name.to_owned(),
            command: command.to_owned(),
        };
        app.terminal.shells = vec![
            shell("zsh", "/bin/zsh"),
            shell("bash", "/opt/homebrew/bin/bash"),
            shell(
                "Git Bash",
                r#""C:\Program Files\Git\bin\bash.exe" --login -i"#,
            ),
        ];
        app.config.terminal_size = Some(13.0);
        app.settings_show_page(SettingsPage::Terminal);
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("settings-terminal-{tag}"));

        // 「默认 Shell」的菜单：系统默认和检测到的 shell，副标题是启动命令。
        app.settings_open_menu(Setting::TerminalShell);
        shoot(&mut app, &format!("settings-terminal-shell-{tag}"));
        app.context_menu = None;

        // 自定义命令：没有设置时输入框为空，以淡色显示填写提示。
        app.settings_start_editing(Setting::TerminalShell);
        shoot(&mut app, &format!("settings-terminal-custom-{tag}"));
        app.settings_cancel_field();

        // 滚动到底：开关一组和 Meta 键。
        app.settings_scroll_by(-1000.0);
        shoot(&mut app, &format!("settings-terminal-bottom-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_format_error() {
    let dir = search_folder("format-error-shot");
    std::fs::write(dir.join("Makefile"), "all:\n\techo hi\n").unwrap();
    for (tag, dark) in [("light", false), ("dark", true)] {
        // Makefile 没有可自动选用的格式化工具：编辑区右上角提示后续的处理办法。
        let mut app = app_with(&dir.join("Makefile"), dark);
        app.format_active();
        shoot(&mut app, &format!("format-error-{tag}"));

        // 窗口最窄时，提示文字从中间截短，不超出编辑区。
        app.renderer = Some(
            Renderer::offscreen(640.0, 420.0, SCALE, app.palette.frame_bg, None, None).unwrap(),
        );
        app.relayout();
        shoot(&mut app, &format!("format-error-small-{tag}"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_format_output() {
    use std::os::unix::fs::PermissionsExt;

    let dir = search_folder("format-output-shot");
    let tool = dir.join("broken-fmt");
    std::fs::write(
        &tool,
        "#!/bin/sh\necho 'error: expected expression, found `;`' >&2\n\
         echo ' --> <stdin>:2:13' >&2\necho '  |' >&2\n\
         echo '2 |     let x = ;' >&2\necho '  |             ^ expected expression' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

    for (tag, dark) in [("light", false), ("dark", true)] {
        // 出错的提示条右侧有「查看输出」，鼠标停在其上。
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.config
            .format_commands
            .insert("rust".to_owned(), format!("'{}'", tool.display()));
        app.format_active();
        app.wait_formatting();
        paint(&mut app);
        let button = app.hits.notice_button.unwrap();
        app.on_mouse_move(button.x + 10.0, button.y + 10.0);
        shoot(&mut app, &format!("format-output-{tag}"));

        // 点击之后：打开只读的「格式化输出」标签。
        app.run_notice_action();
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("format-output-tab-{tag}"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_format_install() {
    let dir = search_folder("format-install-shot");
    std::fs::write(dir.join("tool.py"), "def main():\n  print('hi')\n").unwrap();

    for (tag, dark) in [("light", false), ("dark", true)] {
        // 项目中无法判断 Python 使用哪种工具，候选工具也都未安装：提示条右侧是「安装方法」。
        let mut app = app_with(&dir.join("tool.py"), dark);
        app.format_active();
        app.wait_formatting();
        paint(&mut app);
        let button = app.hits.notice_button.unwrap();
        app.on_mouse_move(button.x + 10.0, button.y + 10.0);
        shoot(&mut app, &format!("format-install-{tag}"));

        // 点击之后：打开只读的「安装格式化工具」标签。
        app.run_notice_action();
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("format-install-tab-{tag}"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_english() {
    // 切换的是整个进程的界面语言，只能单独运行，结束时恢复源语言。
    winc_i18n::set_locale(winc_i18n::Locale::EnUs);
    let dir = search_folder("english-shot");

    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.editor_font = Some("Menlo".to_owned());
    shoot(&mut app, "english-editor");

    app.config.terminal_enabled = Some(true);
    app.open_settings();
    shoot(&mut app, "english-settings");
    app.settings_open_menu(Setting::Language);
    shoot(&mut app, "english-settings-language");
    app.context_menu = None;
    for page in [
        SettingsPage::Editor,
        SettingsPage::Format,
        SettingsPage::Terminal,
        SettingsPage::Keys,
    ] {
        app.settings_show_page(page);
        shoot(
            &mut app,
            &format!("english-settings-{}", page.title().to_lowercase()),
        );
    }

    winc_i18n::set_locale(winc_i18n::Locale::SOURCE);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_about_page() {
    let dir = search_folder("settings-about-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.open_settings();
        app.settings_show_page(SettingsPage::About);
        *app.shared.update_state.borrow_mut() = crate::winc::UpdateState::Found("0.1.400".into());
        paint(&mut app);
        let (x, y) = settings_point(&app, SettingsPart::Action(Setting::Updates));
        app.on_mouse_move(x, y);
        shoot(&mut app, &format!("settings-about-{tag}"));

        // 不支持应用内更新：只写明原因，没有按钮；鼠标停在一个链接上。
        *app.shared.update_state.borrow_mut() = crate::winc::UpdateState::Off;
        let (x, y) = settings_point(&app, SettingsPart::Link(Setting::Repository));
        app.on_mouse_move(x, y);
        shoot(&mut app, &format!("settings-about-off-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：界面缩放为 125%、150%、200% 时的窗口和设置（`zoom-<百分比>`、`zoom-<百分比>-settings`）。
///
/// 窗口的物理尺寸不变，界面、编辑区和终端一同放大。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_zoom() {
    let dir = search_folder("zoom-shot");
    for zoom in [125, 150, 200] {
        let mut app = app_with(&dir.join("short.rs"), false);
        app.config.terminal_enabled = Some(true);
        app.set_zoom(zoom);
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("zoom-{zoom}"));
        app.open_settings();
        shoot(&mut app, &format!("zoom-{zoom}-settings"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：分类栏顶部的搜索框，搜索「字号」时各页的结果（`settings-search-*`），以及没有匹配时
/// 的说明（`settings-search-empty-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_settings_search() {
    let dir = search_folder("settings-search-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.terminal_enabled = Some(true);
        app.open_settings();
        app.on_mouse_move(4.0, 4.0);
        shoot(&mut app, &format!("settings-search-idle-{tag}"));

        app.focus_settings_search();
        app.edit_field(|f| f.insert("字号"));
        shoot(&mut app, &format!("settings-search-{tag}"));

        app.edit_field(|f| {
            f.set_text("没有这一项");
            true
        });
        shoot(&mut app, &format!("settings-search-empty-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
