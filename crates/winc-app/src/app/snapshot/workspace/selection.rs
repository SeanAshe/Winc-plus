//! 文件树多选的测试：⌘ 单击、⇧ 单击、⇧↑↓ 选中多项，对选中的各项删除、复制、复制路径、
//! 拖动移动（按着 ⌥ 时复制），剪切后粘贴移动，一起打开，比较两个文件。

use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::*;

/// 返回文件树里选中的各项的名字，按显示顺序。
fn selected_names(app: &App) -> Vec<String> {
    app.editor
        .workspace
        .as_ref()
        .unwrap()
        .selection()
        .iter()
        .map(|p| winc_editor::display_name(p).into_owned())
        .collect()
}

/// 按着 `modifiers` 单击文件树第 `index` 行。
fn click_row_with(app: &mut App, index: usize, modifiers: ModifiersState) {
    app.modifiers = modifiers;
    let at = tree_row_center(app, index);
    click(app, at);
    app.modifiers = ModifiersState::empty();
}

/// 返回文件树第 `from` 到第 `to` 行（含）的名字。
fn names_between(app: &App, from: usize, to: usize) -> Vec<String> {
    app.editor.workspace.as_ref().unwrap().rows()[from..=to]
        .iter()
        .map(|e| e.name.clone())
        .collect()
}

#[test]
fn command_and_shift_clicks_build_a_selection_and_a_plain_click_resets_it() {
    let dir = sample_folder("tree-multi-select");
    let mut app = app_with(&dir, false);
    paint(&mut app);

    let a = row_of(&app, "a.txt");
    let docker = row_of(&app, "Dockerfile");
    let short = row_of(&app, "short.rs");
    click_row_with(&mut app, a, ModifiersState::empty());
    assert_eq!(app.editor.active().unwrap().name(), "a.txt");

    // ⌘ 单击加选、再点一次移出；只改选区，不打开文件。
    click_row_with(&mut app, docker, command_modifiers());
    assert_eq!(selected_names(&app), ["a.txt", "Dockerfile"]);
    assert_eq!(app.editor.active().unwrap().name(), "a.txt");
    click_row_with(&mut app, a, command_modifiers());
    assert_eq!(selected_names(&app), ["Dockerfile"]);

    // ⇧ 单击从上一次单击的那一行选到这一行。
    click_row_with(&mut app, short, ModifiersState::SHIFT);
    assert_eq!(selected_names(&app), names_between(&app, a, short));

    // 普通单击只选这一行。
    click_row_with(&mut app, docker, ModifiersState::empty());
    assert_eq!(selected_names(&app), ["Dockerfile"]);
    assert_eq!(app.editor.active().unwrap().name(), "Dockerfile");

    // ⇧↓ 往下扩展，Esc 退回只选当前行，再按 Esc 才回到编辑区。
    app.modifiers = ModifiersState::SHIFT;
    app.on_key_tree(&Key::Named(NamedKey::ArrowDown));
    app.on_key_tree(&Key::Named(NamedKey::ArrowDown));
    app.modifiers = ModifiersState::empty();
    assert_eq!(
        selected_names(&app),
        names_between(&app, docker, docker + 2)
    );
    app.on_key_tree(&Key::Named(NamedKey::Escape));
    assert_eq!(
        selected_names(&app),
        names_between(&app, docker + 2, docker + 2)
    );
    assert_eq!(app.focus, Focus::Tree);
    app.on_key_tree(&Key::Named(NamedKey::Escape));
    assert_eq!(app.focus, Focus::Editor);

    // 焦点在文件树时 ⌘A 选中所有看得见的行。
    app.focus = Focus::Tree;
    app.run_action(crate::keymap::Action::SelectAll);
    let rows = app.editor.workspace.as_ref().unwrap().rows().len();
    assert_eq!(selected_names(&app).len(), rows);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_menu_on_a_selection_copies_and_deletes_every_selected_item() {
    let dir = sample_folder("tree-multi-menu");
    let mut app = app_with(&dir, false);
    app.trash = delete_for_test;
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    let src = row_of(&app, "src");
    let a = row_of(&app, "a.txt");
    let cargo = row_of(&app, "Cargo.toml");
    click_row_with(&mut app, a, ModifiersState::empty());
    click_row_with(&mut app, cargo, command_modifiers());
    click_row_with(&mut app, src, command_modifiers());
    assert_eq!(selected_names(&app), ["src", "a.txt", "Cargo.toml"]);

    // 右键点在选区里：选区不变，菜单只有对多项有意义的几项。
    let at = tree_row_center(&app, cargo);
    right_click(&mut app, at);
    assert_eq!(selected_names(&app), ["src", "a.txt", "Cargo.toml"]);
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [
            MenuCommand::OpenFiles,
            MenuCommand::CompareFiles,
            MenuCommand::Cut,
            MenuCommand::Copy,
            MenuCommand::CopyPath,
            MenuCommand::CopyRelativePath,
            MenuCommand::Delete,
        ]
    );

    // 复制路径一行一个，复制的文件按显示顺序。
    app.run_menu_command(MenuCommand::CopyRelativePath);
    assert_eq!(app.copied.as_deref(), Some("src\na.txt\nCargo.toml"));
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::Copy);
    let expected: Vec<PathBuf> = ["src", "a.txt", "Cargo.toml"]
        .iter()
        .map(|n| root.join(n))
        .collect();
    assert_eq!(app.copied_files, expected);

    // 删除一次删掉全部。
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::Delete);
    for path in &expected {
        assert!(!path.exists(), "{}", path.display());
    }
    let moved = format!("已将 3 项移到{}", crate::app::snapshot::trash_name());
    assert_eq!(app.status.message.as_deref(), Some(moved.as_str()));
    assert!(!app.editor.workspace.as_ref().unwrap().multi_selected());

    // 右键点在选区外：只选那一行，菜单是平常的。
    let docker = row_of(&app, "Dockerfile");
    click_row_with(&mut app, docker, ModifiersState::empty());
    click_row_with(&mut app, docker + 1, command_modifiers());
    let at = tree_row_center(&app, docker + 2);
    right_click(&mut app, at);
    assert_eq!(selected_names(&app).len(), 1);
    assert!(
        menu_commands(&app)
            .iter()
            .any(|(c, _)| *c == MenuCommand::Rename)
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn delete_key_trashes_the_selection_and_a_selected_folder_covers_its_children() {
    let dir = sample_folder("tree-multi-delete-key");
    let mut app = app_with(&dir, false);
    app.trash = delete_for_test;
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    // 展开 `src`，连同其中的 `main.rs` 一起选中：只删除 `src` 一次，不会再删除已不存在的
    // `main.rs`。
    let src = row_of(&app, "src");
    click_row_with(&mut app, src, ModifiersState::empty());
    let main = row_of(&app, "main.rs");
    click_row_with(&mut app, main, ModifiersState::SHIFT);
    let a = row_of(&app, "a.txt");
    click_row_with(&mut app, a, command_modifiers());
    assert_eq!(selected_names(&app), ["src", "main.rs", "a.txt"]);
    assert_eq!(app.tree_targets(), [root.join("src"), root.join("a.txt")]);

    app.run_action(crate::keymap::Action::DeleteFile);
    assert!(!root.join("src").exists());
    assert!(!root.join("a.txt").exists());
    let moved = format!("已将 2 项移到{}", crate::app::snapshot::trash_name());
    assert_eq!(app.status.message.as_deref(), Some(moved.as_str()));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dragging_a_selected_row_moves_the_whole_selection() {
    let dir = sample_folder("tree-multi-drag");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    let a = row_of(&app, "a.txt");
    let short = row_of(&app, "short.rs");
    click_row_with(&mut app, a, ModifiersState::empty());
    click_row_with(&mut app, short, command_modifiers());

    // 按在选区里的一行上拖到 lib：两项一起移过去，移过去以后仍选中这两项。
    let lib = row_of(&app, "lib");
    let (x, y) = tree_row_center(&app, short);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(selected_names(&app), ["a.txt", "short.rs"]);
    let (_, lib_y) = tree_row_center(&app, lib);
    app.on_mouse_move(x + 10.0, lib_y);
    assert_eq!(app.tree_dragging().unwrap().paths.len(), 2);
    app.on_mouse_up(MouseButton::Left);
    assert!(root.join("lib/a.txt").is_file());
    assert!(root.join("lib/short.rs").is_file());
    assert!(!root.join("a.txt").exists());
    assert_eq!(selected_names(&app), ["a.txt", "short.rs"]);

    // 在选区里的一行上按下又松开（没有拖动）：只选这一行并打开它。
    let moved = row_of(&app, "short.rs");
    click_row_with(&mut app, moved, ModifiersState::empty());
    assert_eq!(selected_names(&app), ["short.rs"]);
    assert_eq!(app.editor.active().unwrap().name(), "short.rs");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cut_and_paste_in_the_tree_moves_the_selection_once() {
    let dir = sample_folder("tree-multi-cut");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    let a = row_of(&app, "a.txt");
    let short = row_of(&app, "short.rs");
    click_row_with(&mut app, a, ModifiersState::empty());
    click_row_with(&mut app, short, command_modifiers());
    app.focus = Focus::Tree;
    app.run_action(crate::keymap::Action::Cut);
    assert_eq!(
        *app.shared.cut_files.borrow(),
        [root.join("a.txt"), root.join("short.rs")]
    );
    assert_eq!(app.copied_files, *app.shared.cut_files.borrow());
    assert_eq!(app.status.message.as_deref(), Some("已剪切 2 项"));

    // 粘贴到 lib：两项移过去，剪切的记录清掉，移过去的两项选中。
    let lib = row_of(&app, "lib");
    click_row_with(&mut app, lib, ModifiersState::empty());
    app.focus = Focus::Tree;
    app.run_action(crate::keymap::Action::Paste);
    assert!(root.join("lib/a.txt").is_file());
    assert!(root.join("lib/short.rs").is_file());
    assert!(!root.join("a.txt").exists() && !root.join("short.rs").exists());
    assert!(app.shared.cut_files.borrow().is_empty());
    assert_eq!(selected_names(&app), ["a.txt", "short.rs"]);

    // 先前剪切过的文件不参与这次复制：按普通复制处理。
    let docker = row_of(&app, "Dockerfile");
    click_row_with(&mut app, docker, ModifiersState::empty());
    app.focus = Focus::Tree;
    app.run_action(crate::keymap::Action::Cut);
    app.run_action(crate::keymap::Action::Copy);
    assert!(app.shared.cut_files.borrow().is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opening_and_comparing_a_selection() {
    let dir = sample_folder("tree-multi-open");
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("short.rs"), "one\nthree\n").unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);

    // ⌘↓ 打开选中的各个文件（目录跳过），选区留着。
    let src = row_of(&app, "src");
    let a = row_of(&app, "a.txt");
    let short = row_of(&app, "short.rs");
    click_row_with(&mut app, a, ModifiersState::empty());
    click_row_with(&mut app, short, command_modifiers());
    click_row_with(&mut app, src, command_modifiers());
    app.focus = Focus::Tree;
    app.run_action(crate::keymap::Action::OpenSelected);
    let open: Vec<String> = app
        .editor
        .documents
        .iter()
        .map(|d| d.name().into_owned())
        .collect();
    assert!(open.contains(&"a.txt".to_owned()) && open.contains(&"short.rs".to_owned()));
    assert_eq!(selected_names(&app), ["src", "a.txt", "short.rs"]);
    assert_eq!(app.focus, Focus::Tree);

    // 正好两个文件：菜单里有「比较所选文件」，打开两边的差异。
    let src = row_of(&app, "src");
    click_row_with(&mut app, src, command_modifiers());
    assert_eq!(selected_names(&app), ["a.txt", "short.rs"]);
    let at = tree_row_center(&app, row_of(&app, "a.txt"));
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CompareFiles);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("a.txt ↔ short.rs"));
    let view = doc.diff.as_deref().unwrap();
    assert_eq!(view.old.title, "a.txt");
    assert_eq!(view.new.title, "short.rs");

    // 两个一样的文件只提示。
    std::fs::write(
        dir.join("same.txt"),
        std::fs::read(dir.join("a.txt")).unwrap(),
    )
    .unwrap();
    let tabs = app.editor.documents.len();
    app.compare_files(&dir.join("a.txt"), &dir.join("same.txt"));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(
        app.status.message.as_deref(),
        Some("「a.txt」与「same.txt」相同")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn option_dragging_copies_the_selection_instead_of_moving_it() {
    let dir = sample_folder("tree-multi-copy-drag");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    let a = row_of(&app, "a.txt");
    let short = row_of(&app, "short.rs");
    click_row_with(&mut app, a, ModifiersState::empty());
    click_row_with(&mut app, short, command_modifiers());

    let lib = row_of(&app, "lib");
    let (x, y) = tree_row_center(&app, short);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (_, lib_y) = tree_row_center(&app, lib);
    app.modifiers = ModifiersState::ALT;
    app.on_mouse_move(x + 10.0, lib_y);
    app.on_mouse_up(MouseButton::Left);
    app.modifiers = ModifiersState::empty();
    for name in ["a.txt", "short.rs"] {
        assert!(root.join(name).is_file(), "{name}");
        assert!(root.join("lib").join(name).is_file(), "{name}");
    }
    assert_eq!(selected_names(&app), ["a.txt", "short.rs"]);
    assert!(app.editor.workspace.as_ref().unwrap().selection()[0].starts_with(root.join("lib")));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn typing_jumps_to_names_and_home_end_page_keys_move() {
    use std::time::{Duration, Instant};

    let dir = sample_folder("tree-typing");
    let mut app = app_with(&dir, false);
    app.focus = Focus::Tree;
    paint(&mut app);
    let type_key = |app: &mut App, c: &str| app.on_key_tree(&Key::Character(c.into()));
    let selected = |app: &App| {
        let ws = app.editor.workspace.as_ref().unwrap();
        ws.rows()[ws.selected.unwrap()].name.clone()
    };
    // 把上次键入的时刻提前到 `TYPE_RESET` 之前，使下一个字符开始新一轮查找。
    let pause = |app: &mut App| {
        if let Some((_, at)) = &mut app.tree.typed {
            *at = Instant::now() - Duration::from_secs(1);
        }
    };

    // 键入单个字符时跳到下一个以该字符开头的条目（不区分大小写），再按一次跳到再下一个。
    type_key(&mut app, "d");
    assert_eq!(selected(&app), "data.bin");
    pause(&mut app);
    type_key(&mut app, "D");
    assert_eq!(selected(&app), "Dockerfile");
    pause(&mut app);
    type_key(&mut app, "d");
    assert_eq!(selected(&app), "data.bin");

    // 连续键入多个字符时按整个前缀查找。
    pause(&mut app);
    for c in ["i", "m", "g", "_", "w"] {
        type_key(&mut app, c);
    }
    assert!(selected(&app).starts_with("img_WNoi"), "{}", selected(&app));
    // 没有匹配的条目时选中项不变。
    type_key(&mut app, "z");
    assert!(selected(&app).starts_with("img_WNoi"));

    // `Home`、`End` 选中首行和末行；`⇧End` 选中从当前行到最后一行的范围。
    let count = app.editor.workspace.as_ref().unwrap().rows().len();
    app.on_key_tree(&Key::Named(NamedKey::Home));
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(0));
    app.on_key_tree(&Key::Named(NamedKey::End));
    assert_eq!(
        app.editor.workspace.as_ref().unwrap().selected,
        Some(count - 1)
    );
    app.on_key_tree(&Key::Named(NamedKey::Home));
    app.modifiers = ModifiersState::SHIFT;
    app.on_key_tree(&Key::Named(NamedKey::End));
    app.modifiers = ModifiersState::empty();
    assert_eq!(selected_names(&app).len(), count);

    // 文件树比一屏短时，翻页停在首行或末行。
    app.on_key_tree(&Key::Named(NamedKey::Home));
    app.on_key_tree(&Key::Named(NamedKey::PageDown));
    assert_eq!(
        app.editor.workspace.as_ref().unwrap().selected,
        Some(count - 1)
    );
    app.on_key_tree(&Key::Named(NamedKey::PageUp));
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(0));

    let _ = std::fs::remove_dir_all(&dir);
}
