//! 格式化与修复的测试：⇧⌥F、右键菜单、保存时格式化、项目自带的工具。
//!
//! 测试使用冒充格式化工具的小脚本，不依赖机器上安装了哪些工具。

use super::*;
use winc_editor::format::{Formatter, Task};

/// 创建测试项目目录：包含 `src/main.rs`、`app.js`，以及存放冒充工具的 `bin` 目录。
///
/// `bin` 目录不在 `PATH` 中。
fn format_folder(name: &str) -> PathBuf {
    let dir = sample_folder(name);
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n  run();\n  stop();\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("app.js"), "let a = 1\n").unwrap();
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    dir
}

/// 在 `dir` 中写入冒充格式化工具的 `#!/bin/sh` 脚本，设为可执行并返回脚本路径。
///
/// `body` 为脚本主体。
fn fake_tool(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// 预先运行一次 `tool`。
///
/// 新生成的程序第一次运行时 macOS 需要检查；测试并行执行或机器繁忙时，这一检查可能耗时数秒，
/// 超过保存时格式化的超时（`SAVE_TIMEOUT`，3 秒）。真正的格式化工具不是刚生成的，不受此影响。
fn warm_up(tool: &Path) {
    let _ = std::process::Command::new(tool)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 将一个把两格缩进改为四格的冒充格式化工具设为 Rust 的格式化命令。
///
/// 该工具是幂等的：对已格式化的内容再次运行，结果不变。
fn use_reindent(app: &mut App, dir: &Path) {
    let tool = fake_tool(&dir.join("bin"), "reindent", r"sed 's/^  \([^ ]\)/    \1/'");
    warm_up(&tool);
    app.config
        .format_commands
        .insert("rust".to_owned(), format!("'{}'", tool.display()));
}

fn text(app: &App) -> String {
    app.editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap()
}

#[test]
fn formatting_replaces_the_text_as_one_undo_step() {
    let dir = format_folder("format-apply");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    use_reindent(&mut app, &dir);
    let before = text(&app);
    let stop = before.find("stop").unwrap();
    app.select(stop, stop + 4);

    app.format_active();
    app.wait_formatting();
    let after = text(&app);
    assert_eq!(after, "fn main() {\n    run();\n    stop();\n}\n");
    assert_eq!(
        app.status.message.as_deref(),
        Some("已使用 reindent 格式化，更改了 2 行")
    );
    // 选区仍然落在原来的词上。
    let s = app.editor.active().unwrap().selection;
    assert_eq!(&after[s.from()..s.to()], "stop");
    assert!(app.notice.text.is_none());

    // ⌘Z 一次即可还原。
    app.run(Command::Undo);
    assert_eq!(text(&app), before);

    // 内容已经是格式化后的结果：不做修改，文档也不标记为已修改。
    app.run(Command::Redo);
    app.save_active();
    app.format_active();
    app.wait_formatting();
    assert_eq!(app.status.message.as_deref(), Some("格式正确，无需更改"));
    assert!(!app.editor.active().unwrap().is_dirty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_selection_can_be_formatted_on_its_own() {
    let dir = format_folder("format-selection");
    std::fs::write(dir.join("a.c"), "int a;\nint b;\nint c;\n").unwrap();
    // 冒充的 `clang-format`：原样输出标准输入，并把收到的参数作为注释追加到最后一行。
    let tool = fake_tool(&dir.join("bin"), "clang-format", "cat; echo \"// $*\"");
    warm_up(&tool);
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("a.c"));
    app.config
        .format_commands
        .insert("c".to_owned(), format!("'{}' --style=llvm", tool.display()));

    // 选区为第二行（结束于第三行行首）：只格式化第二行。
    app.select(7, 14);
    app.format_selection();
    app.wait_formatting();
    assert!(
        text(&app).ends_with("// --style=llvm --lines=2:2\n"),
        "{}",
        text(&app)
    );
    assert_eq!(
        app.status.message.as_deref(),
        Some("已使用 clang-format 格式化所选内容，更改了 1 行")
    );

    // 格式化文档：处理整个文件。
    app.run(Command::Undo);
    app.select(0, 0);
    app.format_active();
    app.wait_formatting();
    assert!(text(&app).ends_with("// --style=llvm\n"), "{}", text(&app));

    // 不支持格式化部分内容的工具：提示原因，不改为格式化整个文件。
    app.open_path(&dir.join("src/main.rs"));
    use_reindent(&mut app, &dir);
    let before = text(&app);
    app.select(0, 5);
    app.format_selection();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：「reindent」不支持格式化所选内容")
    );
    assert_eq!(text(&app), before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edits_made_while_formatting_win() {
    let dir = format_folder("format-stale");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let tool = fake_tool(&dir.join("bin"), "slow", "sleep 0.3; sed 's/^  /    /'");
    app.config
        .format_commands
        .insert("rust".to_owned(), format!("'{}'", tool.display()));

    app.format_active();
    // 重复触发不会再启动第二个任务。
    app.format_active();
    assert_eq!(app.formatting_count(), 1);
    app.select(0, 0);
    app.run(Command::Insert("x".into()));
    let typed = text(&app);
    app.wait_formatting();
    assert_eq!(text(&app), typed);
    assert_eq!(
        app.status.message.as_deref(),
        Some("文件在格式化期间被修改，未应用结果")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_the_tab_stops_the_tool() {
    let dir = format_folder("format-close");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let tool = fake_tool(&dir.join("bin"), "hang", "sleep 30");
    app.config
        .format_commands
        .insert("rust".to_owned(), format!("'{}'", tool.display()));

    app.format_active();
    let id = app.editor.active().unwrap().id;
    let started = Instant::now();
    app.remove_tabs(&[id]);
    assert_eq!(app.formatting_count(), 0);
    // 工具进程被终止，后台线程很快送回「已取消」的结果，且不弹出提示。
    app.wait_formatting();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(app.notice.text.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failures_explain_themselves_and_leave_the_text_alone() {
    let dir = format_folder("format-fail");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let before = text(&app);
    let tool = fake_tool(
        &dir.join("bin"),
        "broken",
        "echo 'error: expected expression' >&2; echo ' --> <stdin>:2:3' >&2; exit 1",
    );
    app.config
        .format_commands
        .insert("rust".to_owned(), format!("'{}'", tool.display()));

    app.format_active();
    app.wait_formatting();
    assert_eq!(text(&app), before);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：expected expression（2:3）")
    );

    // 点击提示条上的「查看输出」：在只读标签中打开完整输出，提示条收起。
    paint(&mut app);
    let button = app.hits.notice_button.expect("提示条上有按钮");
    app.on_mouse_move(button.x + 4.0, button.y + 4.0);
    assert_eq!(app.hover, Hover::NoticeButton);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.notice.text.is_none());
    let output = app.editor.active().unwrap();
    assert_eq!(output.name(), "格式化输出");
    let shown = text(&app);
    assert!(
        shown.contains("< main.rs") && shown.contains(" --> <stdin>:2:3"),
        "{shown}"
    );
    // 输出标签为只读：⌘S 不弹出「另存为」、不写盘，只在状态栏提示。
    app.run(Command::Insert("x".into()));
    assert_eq!(text(&app), shown);
    app.save_active();
    assert_eq!(app.status.message.as_deref(), Some("此标签页为只读"));
    app.open_path(&dir.join("src/main.rs"));

    // 看不出用哪种工具、候选工具都没安装：提示可安装哪些工具，「安装方法」打开安装说明。
    app.open_path(&dir.join("app.js"));
    app.format_active();
    app.wait_formatting();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：未安装 JavaScript 格式化工具（Prettier 或 Biome）")
    );
    assert!(matches!(
        &app.notice.action,
        Some(NoticeAction::Install { tools, .. }) if tools == &["prettier", "biome"]
    ));
    app.run_notice_action();
    assert_eq!(app.editor.active().unwrap().name(), "安装格式化工具");
    assert!(text(&app).contains("npm install --global @biomejs/biome"));

    // 这种语言没有自动使用的工具：提示条上的「去设置」打开设置的格式化一页。
    std::fs::write(dir.join("Makefile"), "all:\n\ttrue\n").unwrap();
    app.open_path(&dir.join("Makefile"));
    app.format_active();
    app.wait_formatting();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：Makefile 没有可自动使用的格式化工具，请在设置中指定命令")
    );
    assert_eq!(app.notice.action, Some(NoticeAction::FormatSettings));
    app.run_notice_action();
    assert!(app.settings.open);
    assert_eq!(app.settings.selected.page(), SettingsPage::Format);
    app.close_settings();

    // 安装了排在候选列表后面的 Biome：改用 Biome。
    let biome = fake_tool(&dir.join("bin"), "biome", "tr a-z A-Z");
    warm_up(&biome);
    let path = std::env::join_paths([dir.join("bin"), "/usr/bin".into(), "/bin".into()]).unwrap();
    app.format.formatter = Formatter::with_vars(vec![("PATH".into(), path)]);
    app.open_path(&dir.join("app.js"));
    app.format_active();
    app.wait_formatting();
    assert_eq!(text(&app), "LET A = 1\n");
    assert_eq!(
        app.status.message.as_deref(),
        Some("已使用 biome 格式化，更改了 1 行")
    );
    app.run(Command::Undo);

    // 命令中的相对路径指向不存在的程序：直接提示找不到，无需先询问是否允许运行。
    app.config
        .format_commands
        .insert("javascript".to_owned(), "./scripts/fmt.sh".to_owned());
    app.format_active();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：未找到「fmt.sh」")
    );
    assert_eq!(app.formatting_count(), 0);

    // 无法识别语言的文件和只读文档，同样提示原因。
    app.open_path(&dir.join("a.txt"));
    app.format_active();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：无法识别文件的语言")
    );
    app.open_path(&dir.join("src/main.rs"));
    app.editor.active_mut().unwrap().readonly = true;
    app.format_active();
    assert!(
        app.notice
            .text
            .as_deref()
            .unwrap()
            .starts_with("无法格式化：")
    );
    assert_eq!(app.formatting_count(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_tools_run_only_after_asking() {
    let dir = format_folder("format-project");
    std::fs::write(dir.join(".prettierrc"), "{}").unwrap();
    fake_tool(&dir.join("node_modules/.bin"), "prettier", "sed 's/$/;/'");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("app.js"));

    // 尚未询问用户：不运行项目中的工具（测试中无法弹出确认框），并提示原因。
    app.format_active();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：需要先确认是否允许运行项目中的「prettier」")
    );
    assert_eq!(text(&app), "let a = 1\n");

    // 用户选择了「运行」：记录该选择并继续格式化。
    let root = app.editor.root().unwrap().to_path_buf();
    let id = app.editor.active().unwrap().id;
    app.answer_tool_choice(ToolAnswer {
        task: Task::Format,
        id,
        save: false,
        range: None,
        folder: root.clone(),
        run: Some(true),
    });
    app.wait_formatting();
    assert_eq!(text(&app), "let a = 1;\n");
    assert_eq!(app.state.tool_choice(&root), Some(true));

    // 用户选择「使用系统中安装的」：到 `PATH` 中查找；找不到时说明项目使用的是哪种工具。
    app.state.set_tool_choice(&root, false);
    app.notice.text = None;
    app.format_active();
    app.wait_formatting();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：项目使用 Prettier（.prettierrc），但未找到「prettier」")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_settings_win_but_run_only_after_asking() {
    let dir = format_folder("format-project-config");
    let tool = fake_tool(&dir.join("bin"), "team-fmt", "tr a-z A-Z");
    warm_up(&tool);
    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::write(
        dir.join(".winc/config.json"),
        format!(
            r#"{{"format": {{"onSave": true, "onSaveExclude": ["markdown"], "commands": {{"javascript": "'{}'"}}}}}}"#,
            tool.display()
        ),
    )
    .unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("app.js"));
    // 用户在个人设置中为 JavaScript 自定义的命令，在该项目中被项目设置覆盖。
    app.config
        .format_commands
        .insert("javascript".to_owned(), "/nowhere/fmt".to_owned());

    // 项目设置中的命令来自代码仓库：未经用户确认不运行。
    app.format_active();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：需要先确认是否允许运行项目设置中的格式化命令")
    );

    // 用户已同意：记入项目的 `.winc`，并继续格式化。
    let root = app.editor.root().unwrap().to_path_buf();
    let id = app.editor.active().unwrap().id;
    app.answer_tool_choice(ToolAnswer {
        task: Task::Format,
        id,
        save: false,
        range: None,
        folder: root.join(".winc"),
        run: Some(true),
    });
    app.wait_formatting();
    assert_eq!(text(&app), "LET A = 1\n");
    assert_eq!(app.state.tool_choice(&root.join(".winc")), Some(true));

    // 项目设置开启了保存时格式化（用户自己的设置未开启），并排除了 Markdown。
    assert_eq!(app.config.format_on_save, None);
    assert!(app.formats_on_save(id));
    std::fs::write(dir.join("README.md"), "# x\n").unwrap();
    app.open_path(&dir.join("README.md"));
    assert!(!app.formats_on_save(app.editor.active().unwrap().id));

    // 设置页：JavaScript 这一行的 `project` 字段为项目设置中的命令，且没有警告。
    app.open_settings();
    let row = app.settings.format_row("javascript").unwrap();
    assert_eq!(
        row.project.as_deref(),
        Some(format!("'{}'", tool.display()).as_str())
    );
    assert_eq!(row.warning, None);

    // 用户已选择不运行：提示原因，不格式化。
    app.close_settings();
    app.state.set_tool_choice(&root.join(".winc"), false);
    app.open_path(&dir.join("app.js"));
    app.format_active();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：已选择不运行项目设置中的格式化命令")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_enabling_format_on_save_asks_first() {
    let dir = format_folder("format-project-on-save");
    let tool = fake_tool(&dir.join("bin"), "my-fmt", "tr a-z A-Z");
    warm_up(&tool);
    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::write(
        dir.join(".winc/config.json"),
        r#"{"format": {"onSave": true}}"#,
    )
    .unwrap();
    let mut app = app_with(&dir, false);
    // 命令是用户自己设置的，只有「保存时格式化」是项目打开的。
    app.config
        .format_commands
        .insert("javascript".to_owned(), format!("'{}'", tool.display()));
    app.open_path(&dir.join("app.js"));
    let id = app.editor.active().unwrap().id;
    let on_disk = || std::fs::read_to_string(dir.join("app.js")).unwrap();

    // 还没问过：不格式化（测试中无法弹出确认框），仍然保存。
    app.select(0, 0);
    app.run(Command::Insert("b\n".into()));
    app.save_active();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(on_disk(), "b\nlet a = 1\n");

    // 允许：记下来，格式化以后保存。
    let key = app.editor.root().unwrap().join(".winc/formatOnSave");
    app.run(Command::Insert("c".into()));
    app.answer_tool_choice(ToolAnswer {
        task: Task::Format,
        id,
        save: true,
        range: None,
        folder: key.clone(),
        run: Some(true),
    });
    app.wait_formatting();
    assert_eq!(on_disk(), "B\nCLET A = 1\n");
    assert_eq!(app.state.tool_choice(&key), Some(true));

    // 不允许：仍然保存，不格式化，也不提示。
    app.state.set_tool_choice(&key, false);
    app.notice.text = None;
    app.run(Command::Insert("d".into()));
    app.save_active();
    assert_eq!(app.formatting_count(), 0);
    assert!(on_disk().contains('d'), "没有格式化");

    // 用户自己在设置里也开启后：不再询问。
    app.config.format_on_save = Some(true);
    app.run(Command::Insert("e".into()));
    app.save_active();
    app.wait_formatting();
    assert!(
        !on_disk().chars().any(|c| c.is_ascii_lowercase()),
        "格式化了"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_formats_first_and_always_saves() {
    let dir = format_folder("format-save");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    use_reindent(&mut app, &dir);
    let on_disk = || std::fs::read_to_string(dir.join("src/main.rs")).unwrap();

    // 未开启「保存时格式化」：直接保存。
    app.select(0, 0);
    app.run(Command::Insert("// a\n".into()));
    app.save_active();
    assert_eq!(app.formatting_count(), 0);
    assert!(on_disk().starts_with("// a\nfn main() {\n  run();"));

    // 开启后：先格式化再保存。
    app.config.format_on_save = Some(true);
    app.run(Command::Insert("// b\n".into()));
    app.save_active();
    assert_eq!(app.formatting_count(), 1);
    app.wait_formatting();
    assert!(
        on_disk().contains("\n    run();"),
        "{:?}",
        app.status.message
    );
    assert!(!app.editor.active().unwrap().is_dirty());
    assert_eq!(
        app.status.message.as_deref(),
        Some("已保存 main.rs，使用 reindent 格式化了 2 行")
    );

    // 格式化失败仍然保存，只在状态栏提示，不弹出提示条。
    let broken = fake_tool(&dir.join("bin"), "broken", "echo 'bad input' >&2; exit 2");
    warm_up(&broken);
    app.config
        .format_commands
        .insert("rust".to_owned(), format!("'{}'", broken.display()));
    app.run(Command::Insert("// c\n".into()));
    app.save_active();
    app.wait_formatting();
    assert!(on_disk().starts_with("// a\n// b\n// c\n"));
    assert_eq!(
        app.status.message.as_deref(),
        Some("已保存 main.rs，未格式化：bad input")
    );
    assert!(app.notice.text.is_none());

    // 被排除的语言，以及项目中无法判断使用哪种工具且候选工具都未安装的语言：直接保存，
    // 不给任何提示。
    app.config.format_on_save_exclude = vec!["rust".to_owned()];
    app.run(Command::Insert("// d\n".into()));
    app.save_active();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(app.status.message.as_deref(), Some("已保存 main.rs"));
    app.open_path(&dir.join("app.js"));
    app.run(Command::Insert("x".into()));
    app.save_active();
    app.wait_formatting();
    assert_eq!(app.status.message.as_deref(), Some("已保存 app.js"));
    assert!(app.notice.text.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_from_the_close_prompt_formats_too() {
    let dir = format_folder("format-close-save");
    std::fs::write(dir.join("src/lib.rs"), "fn a() {\n  b();\n}\n").unwrap();
    let mut app = app_with(&dir, false);
    use_reindent(&mut app, &dir);
    app.config.format_on_save = Some(true);
    let on_disk = |name: &str| std::fs::read_to_string(dir.join("src").join(name)).unwrap();

    // 两个有修改的文件一起关闭，在提示框上点「保存」：逐个格式化并保存，全部保存完成后才关闭。
    app.open_path(&dir.join("src/main.rs"));
    app.run(Command::Insert("// x\n".into()));
    app.open_path(&dir.join("src/lib.rs"));
    app.run(Command::Insert("// y\n".into()));
    let ids: Vec<_> = app.editor.documents.iter().map(|d| d.id).collect();
    app.close_tabs(ids);
    assert!(app.closing.prompt.is_some());
    app.answer_save_prompt(SaveChoice::Save);
    assert_eq!(app.formatting_count(), 1);
    // 正在为关闭而格式化时再次请求关闭，不会再次弹出确认。
    app.request_quit();
    assert!(app.closing.prompt.is_none() && !app.closing.quit_requested);

    app.wait_formatting();
    assert_eq!(app.formatting_count(), 1, "第一个存好了，接着格式化第二个");
    assert_eq!(app.editor.documents.len(), 2, "都存好以前不关");
    app.wait_formatting();
    assert!(app.editor.documents.is_empty());
    assert_eq!(
        on_disk("main.rs"),
        "// x\nfn main() {\n    run();\n    stop();\n}\n"
    );
    assert_eq!(on_disk("lib.rs"), "// y\nfn a() {\n    b();\n}\n");

    // 格式化失败仍然保存并继续关闭；退出时行为相同。
    let broken = fake_tool(&dir.join("bin"), "broken", "echo nope >&2; exit 1");
    warm_up(&broken);
    app.config
        .format_commands
        .insert("rust".to_owned(), format!("'{}'", broken.display()));
    app.open_path(&dir.join("src/main.rs"));
    app.run(Command::Insert("// z\n".into()));
    app.request_quit();
    app.answer_save_prompt(SaveChoice::Save);
    app.wait_formatting();
    assert!(app.closing.quit_requested);
    assert!(on_disk("main.rs").starts_with("// z\n// x\n"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_caret_line_stays_put_on_screen() {
    let dir = format_folder("format-scroll");
    // 样本：`main` 函数内有 200 行两格缩进的语句，格式化后缩进变为四格，行数不变。
    let long: String = (0..200).map(|i| format!("  line{i}();\n")).collect();
    std::fs::write(dir.join("src/main.rs"), format!("fn main() {{\n{long}}}\n")).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    use_reindent(&mut app, &dir);
    paint(&mut app);

    let line = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.text().line_of(doc.selection.head)
    };
    let pos = text(&app).find("line120").unwrap();
    app.select(pos, pos);
    app.editor.active_mut().unwrap().scroll_line = 110.0;
    app.format_active();
    app.wait_formatting();
    let doc = app.editor.active().unwrap();
    assert_eq!(line(&app), 121);
    assert_eq!(doc.scroll_line, 110.0, "行数没变，页面不动");
    assert_eq!(text(&app)[doc.selection.head..].get(..7), Some("line120"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fixing_is_separate_and_runs_before_formatting_on_save() {
    let dir = format_folder("format-fix");
    std::fs::write(dir.join("a.py"), "import os\nx=1\n").unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("a.py"));
    let on_disk = || std::fs::read_to_string(dir.join("a.py")).unwrap();

    // 项目中无法判断使用哪种修复工具：不像格式化那样自动采用已安装的工具，而是提示原因。
    app.fix_active();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法修复：项目未配置 Python 的修复工具（Ruff），可在设置中指定")
    );
    app.notice.text = None;

    // 冒充的修复工具删除未使用的 `import`，冒充的格式化工具在等号两侧加空格。
    let fixer = fake_tool(&dir.join("bin"), "unused", "sed '/^import os$/d'");
    let formatter = fake_tool(&dir.join("bin"), "spaces", "sed 's/x=1/x = 1/'");
    warm_up(&fixer);
    warm_up(&formatter);
    app.config
        .fix_commands
        .insert("python".to_owned(), format!("'{}'", fixer.display()));
    app.config
        .format_commands
        .insert("python".to_owned(), format!("'{}'", formatter.display()));

    app.fix_active();
    app.wait_formatting();
    assert_eq!(text(&app), "x=1\n", "只修复，不格式化");
    assert_eq!(
        app.status.message.as_deref(),
        Some("已使用 unused 修复，更改了 1 行")
    );
    app.fix_active();
    app.wait_formatting();
    assert_eq!(app.status.message.as_deref(), Some("没有需要修复的问题"));
    app.run(Command::Undo);
    assert_eq!(text(&app), "import os\nx=1\n");

    // 保存时两项都开启：先修复，再格式化，最后保存；修复和格式化各算一次修改。
    app.config.fix_on_save = Some(true);
    app.config.format_on_save = Some(true);
    app.select(0, 0);
    app.run(Command::Insert("y=2\n".into()));
    app.save_active();
    app.wait_formatting();
    assert_eq!(app.formatting_count(), 1, "修复完接着格式化");
    app.wait_formatting();
    assert_eq!(on_disk(), "y=2\nx = 1\n");
    assert!(!app.editor.active().unwrap().is_dirty());
    assert_eq!(
        app.status.message.as_deref(),
        Some("已保存 a.py，使用 unused 修复了 1 行，使用 spaces 格式化了 1 行")
    );
    app.run(Command::Undo);
    assert_eq!(text(&app), "y=2\nx=1\n");
    app.run(Command::Undo);
    assert_eq!(text(&app), "y=2\nimport os\nx=1\n");

    // 只启用修复；修复失败仍然保存。
    app.config.format_on_save = Some(false);
    let broken = fake_tool(&dir.join("bin"), "broken", "echo 'bad config' >&2; exit 2");
    warm_up(&broken);
    app.config
        .fix_commands
        .insert("python".to_owned(), format!("'{}'", broken.display()));
    app.save_active();
    app.wait_formatting();
    assert_eq!(on_disk(), "y=2\nimport os\nx=1\n");
    assert_eq!(
        app.status.message.as_deref(),
        Some("已保存 a.py，未修复：bad config")
    );
    assert!(app.notice.text.is_none());

    // 项目中无法判断使用哪种修复工具：保存时不给任何提示。
    app.config.fix_commands.clear();
    app.run(Command::Insert("z=3\n".into()));
    app.save_active();
    assert_eq!(app.formatting_count(), 0);
    assert_eq!(app.status.message.as_deref(), Some("已保存 a.py"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn untitled_files_format_after_choosing_a_language_in_the_status_bar() {
    let dir = format_folder("format-untitled");
    // 冒充的 JSON 格式化工具：原样输出标准输入，再把 `--stdin-filepath` 的值追加在后面。
    let tool = fake_tool(&dir.join("bin"), "fmt-json", "cat; echo \"$2\"");
    warm_up(&tool);
    let mut app = app_with(&dir, false);
    app.config.format_commands.insert(
        "json".to_owned(),
        format!("'{}' --stdin-filepath {{file}}", tool.display()),
    );
    app.new_file();
    app.run(Command::Insert("{\"sa\": 1}\n".into()));

    // 没保存、认不出语言：提示先在状态栏选择语言。
    app.format_active();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("无法格式化：文件尚未保存，无法确定语言，请在状态栏中选择语言")
    );

    // 点状态栏上的语言：弹出语言菜单，纯文本打勾；没有文件名，没有「按文件名识别」。
    app.notice.text = None;
    paint(&mut app);
    let (rect, _) = *app
        .hits
        .status
        .iter()
        .find(|(_, i)| *i == StatusItem::Language)
        .unwrap();
    click(&mut app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
    let commands = menu_commands(&app);
    assert_eq!(commands[0], (MenuCommand::UsePlainText, true));
    assert_eq!(commands.len(), winc_syntax::languages().len() + 1);
    assert!(app.menu_checked(MenuCommand::UsePlainText));
    let titles: Vec<&str> = commands[1..].iter().map(|(c, _)| c.label()).collect();
    let mut sorted = titles.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(titles, sorted, "语言按名字排序");

    let json = (winc_syntax::languages().iter())
        .position(|l| l.id == "json")
        .unwrap();
    app.run_menu_command(MenuCommand::UseLanguage(json));
    let language = app.editor.active().unwrap().highlighter.language();
    assert_eq!(language.id, "json");

    // 这时可以格式化了：工具拿到的文件名在打开的目录里。
    app.format_active();
    app.wait_formatting();
    let expected = format!(
        "{{\"sa\": 1}}\n{}\n",
        app.editor.root().unwrap().join("untitled.json").display()
    );
    assert_eq!(text(&app), expected);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_language_menu_can_go_back_to_detection() {
    let dir = format_folder("language-detect");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let language = |app: &App| app.editor.active().unwrap().highlighter.language().id;

    // 命令面板中的「更改文件的语言」弹出的是状态栏上的同一个菜单。
    paint(&mut app);
    app.run_action(crate::keymap::Action::ChangeLanguageMode);
    let commands = menu_commands(&app);
    assert_eq!(
        commands[..2],
        [
            (MenuCommand::DetectLanguage, true),
            (MenuCommand::UsePlainText, true)
        ]
    );
    let rust = (winc_syntax::languages().iter())
        .position(|l| l.id == "rust")
        .unwrap();
    assert!(app.menu_checked(MenuCommand::UseLanguage(rust)));

    app.run_menu_command(MenuCommand::UsePlainText);
    assert_eq!(language(&app), "text");
    paint(&mut app);
    app.run_action(crate::keymap::Action::ChangeLanguageMode);
    app.run_menu_command(MenuCommand::DetectLanguage);
    assert_eq!(language(&app), "rust");

    let _ = std::fs::remove_dir_all(&dir);
}
