//! 设置里「格式化」一页的测试：列出用到的语言，加语言、选工具。

use super::*;

#[test]
fn format_settings_list_the_languages_in_use() {
    let (mut app, dir) = format_settings_app("settings-format", false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Format);
    assert_eq!(app.settings_layout().page, SettingsPage::Format);

    // 列出的语言：项目在用的（Python 配置了 Ruff）、总是自动选择的（Rust、Go）、
    // 写过命令的（JSON），按语言表的顺序排列。
    assert_eq!(
        app.settings.languages.format,
        ["python", "rust", "go", "json"]
    );
    // 按钮上显示工具名，名称下方说明工具是如何确定的；指定了命令时，写命令本身。
    let row = |app: &App, id: &'static str| app.settings.format_row(id).unwrap().clone();
    let python = row(&app, "python");
    assert_eq!(python.tool, "Ruff");
    assert_eq!(python.detail, "自动 · 根据 pyproject.toml");
    assert_eq!(python.auto.as_deref(), Some("Ruff（pyproject.toml）"));
    assert_eq!(python.warning.as_deref(), Some("未安装"));
    let rust = row(&app, "rust");
    assert_eq!(
        (rust.tool.as_str(), rust.detail.as_str()),
        ("rustfmt", "自动")
    );
    assert_eq!(rust.warning, None);
    let json = row(&app, "json");
    assert_eq!(
        (json.tool.as_str(), json.detail.as_str()),
        ("自定义", "jq --indent 4 .")
    );
    assert!(json.warning.is_none());

    // 修复工具一组：只列出有修复工具预设的语言。项目配置了 Ruff，修复也使用 Ruff。
    assert_eq!(app.settings.languages.fix, ["python"]);
    let fix = app
        .settings
        .tool_row(Setting::FixCommand("python"))
        .unwrap();
    assert_eq!(
        (fix.tool.as_str(), fix.detail.as_str()),
        ("Ruff", "自动 · 根据 pyproject.toml")
    );
    assert_eq!(fix.warning.as_deref(), Some("未安装"));
    // 这一行位于页面下方、滚动区域之外，点击不到，因此直接弹出该行的菜单。
    app.settings.selected = Setting::FixCommand("python");
    app.settings_open_menu(Setting::FixCommand("python"));
    let fix_preset = PRESETS
        .iter()
        .position(|p| p.task == Task::Fix && p.name == "Ruff")
        .unwrap();
    assert_eq!(
        menu_commands(&app)
            .into_iter()
            .map(|(c, _)| c)
            .collect::<Vec<_>>(),
        [
            MenuCommand::ToolAuto,
            MenuCommand::ToolPreset(fix_preset),
            MenuCommand::ToolCustom,
            MenuCommand::ToolInstall,
        ]
    );
    app.run_menu_command(MenuCommand::ToolPreset(fix_preset));
    assert_eq!(
        app.config.fix_commands.get("python").map(String::as_str),
        Some(PRESETS[fix_preset].command)
    );
    assert!(
        !app.config.format_commands.contains_key("python"),
        "格式化命令不受影响"
    );
    app.settings_choose_tool(Setting::FixCommand("python"), MenuCommand::ToolAuto);
    assert!(app.config.fix_commands.is_empty());

    // 开关：单击开启，再次单击关闭，并写入配置。
    click_part(&mut app, SettingsPart::Switch(Setting::FormatOnSave));
    assert_eq!(app.config.format_on_save, Some(true));
    app.settings_activate();
    assert_eq!(app.config.format_on_save, Some(false));
    click_part(&mut app, SettingsPart::Switch(Setting::FixOnSave));
    assert_eq!(app.config.fix_on_save, Some(true));
    assert_eq!(app.config.format_on_save, Some(false));
    app.settings_activate();
    assert_eq!(app.config.fix_on_save, Some(false));

    // 点击 Rust 的命令：弹出菜单，包含「自动」（打勾，下方显示自动选中的工具）、该语言的预设、
    // 自定义命令。
    let preset = |name: &str| PRESETS.iter().position(|p| p.name == name).unwrap();
    click_part(
        &mut app,
        SettingsPart::Field(Setting::FormatCommand("rust")),
    );
    let commands = |app: &App| -> Vec<MenuCommand> {
        menu_commands(app).into_iter().map(|(c, _)| c).collect()
    };
    assert_eq!(
        commands(&app),
        [
            MenuCommand::ToolAuto,
            MenuCommand::ToolPreset(preset("rustfmt")),
            MenuCommand::ToolCustom
        ]
    );
    assert!(app.menu_checked(MenuCommand::ToolAuto));
    let label = app.menu_label(MenuCommand::ToolAuto);
    assert_eq!(label.subtitle.as_deref(), Some("rustfmt"));

    // 自定义命令：即使找不到程序也会保存，警告显示在名称下方；清空后恢复为自动选择。
    app.run_menu_command(MenuCommand::ToolCustom);
    assert_eq!(app.focus, Focus::Setting);
    let set_command = |app: &mut App, text: &str| {
        app.edit_field(|f| {
            f.select_all();
            f.delete(Unit::Char, false);
            f.insert(text)
        });
        app.settings_commit_field();
    };
    set_command(&mut app, "my-rustfmt --edition 2024");
    assert_eq!(
        app.config.format_commands.get("rust").map(String::as_str),
        Some("my-rustfmt --edition 2024")
    );
    assert_eq!(app.focus, Focus::Settings);
    assert_eq!(
        row(&app, "rust").warning.as_deref(),
        Some("未找到「my-rustfmt」")
    );
    app.settings_start_editing(Setting::FormatCommand("rust"));
    set_command(&mut app, "rustfmt {file}");
    assert_eq!(
        row(&app, "rust").warning.as_deref(),
        Some("该命令读取磁盘上的文件，不包含未保存的修改")
    );
    app.settings_start_editing(Setting::FormatCommand("rust"));
    set_command(&mut app, "");
    assert!(!app.config.format_commands.contains_key("rust"));
    assert_eq!(row(&app, "rust").warning, None);

    // 选择预设：写入配置，菜单中该项打勾；再选「自动」则移除该配置。
    app.settings_open_menu(Setting::FormatCommand("rust"));
    app.run_menu_command(MenuCommand::ToolPreset(preset("rustfmt")));
    assert_eq!(
        app.config.format_commands.get("rust").map(String::as_str),
        Some("rustfmt --edition {rust-edition}")
    );
    app.settings.selected = Setting::FormatCommand("rust");
    app.settings_activate();
    assert!(app.menu_checked(MenuCommand::ToolPreset(preset("rustfmt"))));
    assert!(!app.menu_checked(MenuCommand::ToolAuto));
    app.run_menu_command(MenuCommand::ToolAuto);
    assert!(!app.config.format_commands.contains_key("rust"));

    // 语言行不会因配置改动而消失：Rust 恢复为自动后仍在原位置。
    assert_eq!(app.settings.languages.format[1], "rust");

    // 「排除的语言」：菜单列出页面上已有的语言，点击一项打勾，再次点击则取消，并写入配置。
    assert_eq!(app.skipped_on_save(), None);
    paint(&mut app);
    click_part(&mut app, SettingsPart::Action(Setting::SkipOnSave));
    let index = |id: &str| {
        winc_syntax::languages()
            .iter()
            .position(|l| l.id == id)
            .unwrap()
    };
    let listed: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        listed,
        ["python", "rust", "go", "json"].map(|id| MenuCommand::SkipOnSave(index(id)))
    );
    app.run_menu_command(MenuCommand::SkipOnSave(index("json")));
    assert_eq!(app.config.format_on_save_exclude, ["json"]);
    assert!(app.menu_checked(MenuCommand::SkipOnSave(index("json"))));
    assert!(!app.menu_checked(MenuCommand::SkipOnSave(index("go"))));
    app.settings.selected = Setting::SkipOnSave;
    app.settings_activate();
    app.run_menu_command(MenuCommand::SkipOnSave(index("go")));
    assert_eq!(app.skipped_on_save().as_deref(), Some("Go、JSON"));
    app.settings_open_menu(Setting::SkipOnSave);
    app.run_menu_command(MenuCommand::SkipOnSave(index("json")));
    assert_eq!(app.config.format_on_save_exclude, ["go"]);

    // 项目使用 Ruff 但 Ruff 未安装：菜单中注明未安装，最后一项是「安装方法…」，
    // 点击后关闭设置并打开安装说明。
    assert_eq!(row(&app, "python").install, ["ruff"]);
    app.settings_open_menu(Setting::FormatCommand("python"));
    assert_eq!(commands(&app).last(), Some(&MenuCommand::ToolInstall));
    let ruff = app.menu_label(MenuCommand::ToolPreset(preset("Ruff")));
    assert_eq!(ruff.title, "Ruff（未安装）");
    assert_eq!(
        ruff.subtitle.as_deref(),
        Some("ruff format --stdin-filename {file} -")
    );
    app.run_menu_command(MenuCommand::ToolInstall);
    assert!(!app.settings.open);
    assert_eq!(app.editor.active().unwrap().name(), "安装格式化工具");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn format_settings_add_a_language_and_pick_a_tool() {
    let (mut app, dir) = format_settings_app("settings-add-language", false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Format);
    paint(&mut app);

    // 「添加语言…」弹出尚未列出的语言，有预设工具的排在前面。
    click_part(&mut app, SettingsPart::Action(Setting::AddLanguage));
    let menu = menu_commands(&app);
    let index = |id: &str| {
        winc_syntax::languages()
            .iter()
            .position(|l| l.id == id)
            .unwrap()
    };
    let position = |id: &str| {
        menu.iter()
            .position(|&(c, _)| c == MenuCommand::Language(index(id)))
    };
    assert!(position("rust").is_none(), "已经列出来的不再出现");
    // Shell 有预设工具而 Kotlin 没有，因此 Shell 排在 Kotlin 之前。
    assert!(position("shell").unwrap() < position("kotlin").unwrap());

    // 选择 `Shell`：多出一行，随后弹出该行的菜单选择工具。只有 `shfmt` 会被自动使用，按钮上就写它；
    // 该工具未安装，名称下方提示「未安装」，菜单中有「安装方法…」。
    app.run_menu_command(MenuCommand::Language(index("shell")));
    assert_eq!(app.settings.languages.format.last(), Some(&"shell"));
    assert_eq!(app.settings.selected, Setting::FormatCommand("shell"));
    let shfmt = PRESETS.iter().position(|p| p.name == "shfmt").unwrap();
    let commands = |app: &App| -> Vec<MenuCommand> {
        menu_commands(app).into_iter().map(|(c, _)| c).collect()
    };
    // 自己加的语言最后一项是「移除」。
    assert_eq!(
        commands(&app),
        [
            MenuCommand::ToolAuto,
            MenuCommand::ToolPreset(shfmt),
            MenuCommand::ToolCustom,
            MenuCommand::ToolInstall,
            MenuCommand::ToolRemove
        ]
    );
    let shell = app.settings.format_row("shell").unwrap();
    assert_eq!(
        (shell.tool.as_str(), shell.detail.as_str()),
        ("shfmt", "自动")
    );
    assert_eq!(shell.warning.as_deref(), Some("未安装"));
    app.run_menu_command(MenuCommand::ToolPreset(shfmt));
    assert_eq!(
        app.config.format_commands.get("shell").map(String::as_str),
        Some("shfmt -filename {file}")
    );

    // 自动列出的语言不能移除：Rust 总是用 rustfmt，Python 是项目在用 Ruff。只因写过命令才列出的
    // JSON 可以。
    for (id, removable) in [("rust", false), ("python", false), ("json", true)] {
        app.settings_open_menu(Setting::FormatCommand(id));
        assert_eq!(
            commands(&app).contains(&MenuCommand::ToolRemove),
            removable,
            "{id}"
        );
        app.overlays.context_menu = None;
    }

    // 没有预设工具的语言只能自定义命令：直接进入输入框。
    app.settings_add_language(Task::Format, winc_syntax::find("makefile").unwrap());
    assert!(matches!(
        &app.settings.editing,
        Some((Setting::FormatCommand("makefile"), field)) if field.text().is_empty()
    ));
    // Esc 放弃：配置中没有该语言的命令，但设置保持打开期间这一行仍然保留，说明只能自己写命令。
    app.settings_cancel_field();
    assert!(!app.config.format_commands.contains_key("makefile"));
    assert!(app.settings.languages.format.contains(&"makefile"));
    let makefile = app.settings.format_row("makefile").unwrap();
    assert_eq!(
        (makefile.tool.as_str(), makefile.detail.as_str()),
        ("未设置", "无预设的工具，请选择「自定义命令…」")
    );
    assert!(makefile.unset);
    // 菜单里没有「自动」（没有工具可选），只有自定义命令和移除。移除后这一行立刻消失，
    // 选中项落到上一行（Makefile 是最后一行）。
    app.settings_open_menu(Setting::FormatCommand("makefile"));
    assert_eq!(
        commands(&app),
        [MenuCommand::ToolCustom, MenuCommand::ToolRemove]
    );
    app.run_menu_command(MenuCommand::ToolRemove);
    assert!(!app.settings.languages.format.contains(&"makefile"));
    assert!(app.settings.format_row("makefile").is_none());
    assert_eq!(app.settings.selected, Setting::FormatCommand("shell"));

    // 预设都不自动使用的语言（XML 只有 xmllint）同样没有「自动」，提示选一个工具；选了以后
    // 再移除，配置中的命令一起删掉。
    app.settings_add_language(Task::Format, winc_syntax::find("xml").unwrap());
    let xmllint = PRESETS.iter().position(|p| p.name == "xmllint").unwrap();
    assert_eq!(
        commands(&app),
        [
            MenuCommand::ToolPreset(xmllint),
            MenuCommand::ToolCustom,
            MenuCommand::ToolRemove
        ]
    );
    assert_eq!(
        app.settings.format_row("xml").unwrap().detail,
        "不自动选择工具，请选择要使用的工具"
    );
    app.run_menu_command(MenuCommand::ToolPreset(xmllint));
    assert!(app.config.format_commands.contains_key("xml"));
    app.settings_open_menu(Setting::FormatCommand("xml"));
    app.run_menu_command(MenuCommand::ToolRemove);
    assert!(!app.config.format_commands.contains_key("xml"));
    assert!(!app.settings.languages.format.contains(&"xml"));

    // 修复工具一组只列项目里在用的（Python 的 Ruff），加了格式化工具的 JavaScript 不跟着多出一行
    // 「无」。要给它指定修复工具，用修复工具一组自己的「添加语言…」：有修复预设的排在前面，
    // 选了以后弹出这一行的菜单，同样能移除，移除时只删修复命令。
    app.settings_add_language(Task::Format, winc_syntax::find("javascript").unwrap());
    app.overlays.context_menu = None;
    assert_eq!(app.settings.languages.fix, ["python"]);
    app.settings_open_menu(Setting::AddFixLanguage);
    let menu = commands(&app);
    let at = |id: &str| {
        menu.iter()
            .position(|&c| c == MenuCommand::Language(index(id)))
    };
    assert!(at("python").is_none(), "已经列出来的不再出现");
    assert!(at("javascript").unwrap() < at("go").unwrap());
    app.run_menu_command(MenuCommand::Language(index("javascript")));
    assert_eq!(app.settings.languages.fix, ["python", "javascript"]);
    assert_eq!(app.settings.selected, Setting::FixCommand("javascript"));
    let row = app
        .settings
        .tool_row(Setting::FixCommand("javascript"))
        .unwrap();
    assert_eq!(
        (row.tool.as_str(), row.detail.as_str()),
        ("未设置", "项目未配置修复工具，请选择要使用的工具")
    );
    assert!(row.unset);
    let eslint = PRESETS
        .iter()
        .position(|p| p.task == Task::Fix && p.name == "ESLint")
        .unwrap();
    assert!(commands(&app).ends_with(&[MenuCommand::ToolRemove]));
    app.run_menu_command(MenuCommand::ToolPreset(eslint));
    assert!(app.config.fix_commands.contains_key("javascript"));
    app.settings_choose_tool(Setting::FixCommand("javascript"), MenuCommand::ToolRemove);
    assert_eq!(app.settings.languages.fix, ["python"]);
    assert!(!app.config.fix_commands.contains_key("javascript"));
    assert!(app.settings.languages.format.contains(&"javascript"));
    assert_eq!(app.settings.selected, Setting::FixCommand("python"));
    // 项目在用的 Python 不能移除。
    app.settings_open_menu(Setting::FixCommand("python"));
    assert!(!commands(&app).contains(&MenuCommand::ToolRemove));
    app.overlays.context_menu = None;

    // 再次打开：写过命令的 Shell 仍在，没写命令的 Makefile、移除了的 XML 不再列出。
    app.close_settings();
    app.open_settings();
    assert!(app.settings.languages.format.contains(&"shell"));
    assert!(!app.settings.languages.format.contains(&"makefile"));
    assert!(!app.settings.languages.format.contains(&"xml"));

    let _ = std::fs::remove_dir_all(&dir);
}
