//! 提交视图多选的测试：⌘ 单击与 ⇧ 单击选择改动、目录行选中其下文件，以及一起勾选、复制路径
//! 与回滚。

use winit::keyboard::ModifiersState;

use crate::app::git::commit::CommitRow;
use crate::app::input::context_menu::MenuCommand;

use super::*;

#[test]
fn command_and_shift_clicks_select_several_changes_for_one_check_and_rollback() {
    let dir = search_folder("git-commit-multi");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    // 再修改两个已跟踪的文件：共三个改动，一个已暂存的新文件（`src/new.rs`）。
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(dir.join("src/config.rs"), "\n").unwrap();
    let mut app = app_with(&dir, false);
    app.trash = delete_for_test;
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);

    let files = app.commit_files();
    assert_eq!(files.len(), 4);
    let click_file = |app: &mut App, path: &Path, modifiers: ModifiersState| {
        let status = app.git.status.as_ref().unwrap();
        let index = status.entries.iter().position(|e| e.path == path).unwrap();
        let row = app
            .hits
            .commit
            .iter()
            .find(|(_, t)| *t == CommitTarget::File(index))
            .unwrap()
            .0;
        app.modifiers = modifiers;
        click(app, (row.x + row.w / 2.0, row.y + 4.0));
        app.modifiers = ModifiersState::empty();
        paint(app);
    };

    // ⌘ 单击追加选择：只改选区，右侧预览不换。
    click_file(&mut app, &files[0], ModifiersState::empty());
    let preview = app.editor.active().unwrap().title.clone();
    click_file(&mut app, &files[2], command_modifiers());
    assert_eq!(app.commit_selection(), [files[0].clone(), files[2].clone()]);
    assert_eq!(app.editor.active().unwrap().title, preview);

    // ⇧ 单击从上一次 ⌘ 单击的文件选到这一个。
    click_file(&mut app, &files[3], ModifiersState::SHIFT);
    assert_eq!(app.commit_selection(), files[2..]);

    // 空格：选中的文件一起切换勾选（新文件默认也勾着）：都勾着时一起取消，再按一起勾上。
    let checked = |app: &App, path: &Path| {
        let status = app.git.status.as_ref().unwrap();
        app.commit
            .checked(status.entries.iter().find(|e| e.path == path).unwrap())
    };
    assert!(checked(&app, &files[2]) && checked(&app, &files[3]));
    app.commit_list_key(&Key::Named(NamedKey::Space));
    assert!(!checked(&app, &files[2]) && !checked(&app, &files[3]));
    app.commit_list_key(&Key::Named(NamedKey::Space));
    assert!(checked(&app, &files[2]) && checked(&app, &files[3]));
    app.commit_list_key(&Key::Named(NamedKey::Space));
    assert!(!checked(&app, &files[2]) && !checked(&app, &files[3]));

    // 右键点在选区里：菜单作用于两个文件；复制路径一行一个，回滚一次作用于两个。
    let status = app.git.status.as_ref().unwrap();
    let index = status
        .entries
        .iter()
        .position(|e| e.path == files[2])
        .unwrap();
    let row = app
        .hits
        .commit
        .iter()
        .find(|(_, t)| *t == CommitTarget::File(index))
        .unwrap()
        .0;
    right_click(&mut app, (row.x + 120.0, row.y + row.h / 2.0));
    assert_eq!(app.commit_selection(), files[2..]);
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [
            MenuCommand::GitOpenFile,
            MenuCommand::CommitCheck(true),
            MenuCommand::CommitCheck(false),
            MenuCommand::GitRollbackFile,
            MenuCommand::CopyPath,
            MenuCommand::CopyRelativePath,
        ]
    );
    // 菜单里的勾选、取消勾选作用于选中的两个，没选中的不动。
    let others = |app: &App| [checked(app, &files[0]), checked(app, &files[1])];
    let before = others(&app);
    app.run_menu_command(MenuCommand::CommitCheck(true));
    assert!(checked(&app, &files[2]) && checked(&app, &files[3]));
    assert_eq!(others(&app), before);
    right_click(&mut app, (row.x + 120.0, row.y + row.h / 2.0));
    app.run_menu_command(MenuCommand::CommitCheck(false));
    assert!(!checked(&app, &files[2]) && !checked(&app, &files[3]));
    right_click(&mut app, (row.x + 120.0, row.y + row.h / 2.0));
    app.run_menu_command(MenuCommand::CopyRelativePath);
    let relative: Vec<String> = files[2..]
        .iter()
        .map(|p| {
            p.strip_prefix(app.editor.root().unwrap())
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    assert_eq!(app.copied.as_deref(), Some(relative.join("\n").as_str()));

    // 回滚后已跟踪的文件恢复原内容，未跟踪的文件被移除；未选中的两个文件保持不变。
    let edited: Vec<(PathBuf, Option<String>)> = files
        .iter()
        .map(|p| (p.clone(), std::fs::read_to_string(p).ok()))
        .collect();
    let rolled_back =
        |path: &Path, before: &Option<String>| std::fs::read_to_string(path).ok() != *before;
    right_click(&mut app, (row.x + 120.0, row.y + row.h / 2.0));
    app.run_menu_command(MenuCommand::GitRollbackFile);
    for _ in 0..200 {
        app.poll_commit();
        if edited[2..].iter().all(|(p, before)| rolled_back(p, before)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    for (path, before) in &edited[2..] {
        assert!(rolled_back(path, before), "{}", path.display());
    }
    for (path, before) in &edited[..2] {
        assert!(!rolled_back(path, before), "{}", path.display());
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_click_on_a_folder_row_selects_every_change_under_it() {
    let dir = search_folder("git-commit-multi-dir");
    if !make_repo(&dir) {
        return;
    }
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(dir.join("src/config.rs"), "\n").unwrap();
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);

    let root = app.git.root.clone().unwrap();
    let src_row = |app: &App| {
        app.commit_rows()
            .iter()
            .position(|r| matches!(r, CommitRow::Dir { dir, .. } if dir == Path::new("src")))
            .unwrap()
    };
    let under_src: Vec<PathBuf> = app.commit_dir_files(src_row(&app));
    assert!(under_src.len() >= 2 && under_src.iter().all(|p| p.starts_with(root.join("src"))));

    // ⌘ 单击目录：该目录下的文件都加入选区（`Cargo.toml` 是原来选中的），目录不收起。
    let cargo = root.join("Cargo.toml");
    app.select_commit_files(vec![cargo.clone()]);
    app.modifiers = command_modifiers();
    app.press_commit(CommitTarget::Dir(src_row(&app)));
    app.modifiers = ModifiersState::empty();
    let selection = app.commit_selection();
    assert!(selection.contains(&cargo));
    assert!(under_src.iter().all(|p| selection.contains(p)));
    assert!(
        app.commit_rows()
            .iter()
            .any(|r| matches!(r, CommitRow::File { index, .. }
        if app.git.status.as_ref().unwrap().entries[*index].path == under_src[0]))
    );

    // 收起目录以后，选中的文件还在选区里（菜单与勾选仍然作用于选中的文件）。
    app.press_commit(CommitTarget::Dir(src_row(&app)));
    let selection = app.commit_selection();
    assert!(under_src.iter().all(|p| selection.contains(p)));

    // 再次 ⌘ 单击同一目录：一起移出选区。
    app.modifiers = command_modifiers();
    app.press_commit(CommitTarget::Dir(src_row(&app)));
    app.modifiers = ModifiersState::empty();
    assert_eq!(app.commit_selection(), [cargo]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn arrow_keys_fold_the_selected_changes_folders_and_backspace_reverts_it() {
    let dir = search_folder("git-commit-fold-keys");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    app.trash = delete_for_test;
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let root = app.git.root.clone().unwrap();
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    // 「更改」组里的 `src` 目录下是改过的 `src/main.rs` 和已暂存的新文件 `src/new.rs`。
    let shown = |app: &App| app.commit_files();
    assert_eq!(shown(&app), [main.clone(), new.clone()]);

    // 在提交信息中按 `Tab` 移到列表，并选中第一个文件。
    assert_eq!(app.focus, Focus::CommitMessage);
    app.commit_message_key(&Key::Named(NamedKey::Tab), false, false);
    assert_eq!(app.focus, Focus::CommitList);
    assert_eq!(app.commit.selected.as_deref(), Some(main.as_path()));

    // `←` 收起选中文件所在的 `src`，再次按下时收起整个组；`→` 逐级展开恢复。
    let key = |app: &mut App, key: NamedKey| app.commit_list_key(&Key::Named(key));
    key(&mut app, NamedKey::ArrowLeft);
    assert!(shown(&app).is_empty());
    let rows = app.commit_rows().len();
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(app.commit_rows().len(), rows - 1);
    assert!(app.commit.group_collapsed(CommitGroup::Changes));
    key(&mut app, NamedKey::ArrowRight);
    assert!(!app.commit.group_collapsed(CommitGroup::Changes));
    assert!(shown(&app).is_empty());
    key(&mut app, NamedKey::ArrowRight);
    assert_eq!(shown(&app), [main.clone(), new.clone()]);

    // `↓` 选中 `src/new.rs`，`⌫` 回滚该文件（测试中不确认）。
    key(&mut app, NamedKey::ArrowDown);
    assert_eq!(app.commit.selected.as_deref(), Some(new.as_path()));
    key(&mut app, NamedKey::Backspace);
    assert!(!new.exists());

    // 右键点击组标题时，菜单作用于组内的各个文件。
    paint(&mut app);
    let header = app
        .hits
        .commit
        .iter()
        .find(|(_, t)| *t == CommitTarget::Group(CommitGroup::Changes))
        .unwrap()
        .0;
    right_click(&mut app, (header.x + 120.0, header.y + header.h / 2.0));
    assert!(app.context_menu.is_some());
    assert_eq!(app.commit_selection(), std::slice::from_ref(&main));
    let _ = std::fs::remove_dir_all(&dir);
}
