//! 提交视图的测试：改动列表、勾选、提交、菜单、提交信息、滚动与固定行、折叠、长文件名。

use super::*;
use std::time::Duration;

use crate::app::SyncState;
use crate::app::animation::{Clock, TREE_ANIMATION};
use crate::app::git::commit::COMMITTED_SHOWN;

/// 创建含大量深层新文件（已暂存）的仓库：`assets/pets/a`、`assets/pets/b` 下各 12 张图，
/// 另加 3 个散文件。
fn deep_changes_repo(name: &str) -> Option<PathBuf> {
    let dir = search_folder(name);
    if !make_repo(&dir) {
        return None;
    }
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "all"]);
    for sub in ["a", "b"] {
        let folder = dir.join("assets/pets").join(sub);
        std::fs::create_dir_all(&folder).unwrap();
        for i in 0..12 {
            std::fs::write(folder.join(format!("{i}.png")), "x").unwrap();
        }
    }
    for f in ["arm_L.png", "arm_R.png", "bg.png"] {
        std::fs::write(dir.join("assets/pets").join(f), "x").unwrap();
    }
    git_in(&dir, &["add", "-A"]);
    Some(dir)
}

/// 在 [`deep_changes_repo`] 的基础上新增一个名字很长的已暂存文件，返回仓库目录与文件名。
fn long_name_repo(name: &str) -> Option<(PathBuf, String)> {
    let dir = deep_changes_repo(name)?;
    let long = "a-very-long-file-name-that-never-fits-in-the-sidebar-at-all.png".to_owned();
    std::fs::write(dir.join("assets/pets/a").join(&long), "x").unwrap();
    git_in(&dir, &["add", "-A"]);
    Some((dir, long))
}

#[test]
fn commit_only_the_checked_hunks() {
    let dir = search_folder("git-partial");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    app.git_preview_changes(&file);
    paint(&mut app);
    // 两处改动各有一个勾选框；勾掉第二处（删掉 `value_50`），文件变成「部分」。
    assert!(
        app.diff_hits
            .iter()
            .any(|(_, t)| *t == DiffTarget::Include(1))
    );
    app.press_diff(DiffTarget::Include(1));
    let status = app.git.status.clone().unwrap();
    let entry = status.entries.iter().find(|e| e.path == file).unwrap();
    assert_eq!(app.commit.file_state(entry), Some(false));

    // 勾掉后又修改这一处（删掉的行换成别的内容）：在 `HEAD` 中仍是同几行，依然不提交。
    let disk = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        disk.replace(
            "let value_49 = 49;\n",
            "let value_49 = 49;\nlet value_50 = 5000;\n",
        ),
    )
    .unwrap();

    app.commit.message.set_text("only value_5");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap();
    assert!(head.contains("let value_5 = 500;"));
    assert!(head.contains("let value_50 = 50;") && !head.contains("5000"));
    // 没提交的那处还在工作区里，文件仍然有改动。
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("let value_50 = 5000;"));
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert!(status.entries.iter().any(|e| e.path == file));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn commit_list_groups_changes_by_directory() {
    let Some((mut app, root)) = git_app("git-commit-tree", false) else {
        return;
    };
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    // 几层目录：`pkg` 与 `model` 之间没有其他文件，`core/model` 并成一行；`pkg` 下另有 `z.rs`。
    for path in [
        "pkg/core/model/a.rs",
        "pkg/core/model/b.rs",
        "pkg/z.rs",
        "top.rs",
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "x\n").unwrap();
    }
    git_in(&root, &["add", "-A"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let index = |name: &str| {
        status
            .entries
            .iter()
            .position(|e| e.path.ends_with(name))
            .unwrap()
    };
    // 已暂存的新文件和改过的 `src/main.rs` 同在「更改」一组里。
    let changes: Vec<CommitRow> = app
        .commit_rows()
        .iter()
        .skip_while(|r| **r != CommitRow::Group(CommitGroup::Changes))
        .cloned()
        .collect();
    let dir = |dir: &str, label: &str, depth| CommitRow::Dir {
        group: CommitGroup::Changes,
        dir: PathBuf::from(dir),
        label: label.into(),
        depth,
    };
    let file = |name: &str, depth| CommitRow::File {
        index: index(name),
        depth,
    };
    assert_eq!(
        changes,
        [
            CommitRow::Group(CommitGroup::Changes),
            dir("pkg", "pkg", 0),
            dir("pkg/core/model", "core/model", 1),
            file("model/a.rs", 2),
            file("model/b.rs", 2),
            file("pkg/z.rs", 1),
            dir("src", "src", 0),
            file("src/main.rs", 1),
            file("src/new.rs", 1),
            file("top.rs", 0),
        ]
    );

    // 新文件默认勾着。目录的勾选框取消目录下的所有文件，再点一次全部勾上。
    let row = |app: &App, r: &CommitRow| app.commit_rows().iter().position(|x| x == r).unwrap();
    let pkg = row(&app, &dir("pkg", "pkg", 0));
    let checked = |app: &App, name: &str| app.commit.checked(&status.entries[index(name)]);
    assert!(checked(&app, "model/a.rs") && checked(&app, "top.rs"));
    app.press_commit(CommitTarget::DirCheck(pkg));
    assert!(!checked(&app, "model/a.rs") && !checked(&app, "pkg/z.rs"));
    assert!(checked(&app, "top.rs"));
    app.press_commit(CommitTarget::DirCheck(pkg));
    assert!(checked(&app, "model/a.rs") && checked(&app, "pkg/z.rs"));
    let rows = app.commit_rows();
    let members = rows.dir(CommitGroup::Changes, Path::new("pkg"));
    // `pkg` 目录这一组：合并后的 `core/model` 与 `pkg/z.rs` 里的文件都算在内。
    assert_eq!(members.len(), 3);
    assert_eq!(app.commit.members_state(&status, members), Some(true));
    // 组与目录的勾选框每帧都要查其下所有文件：结果按勾选的版本缓存，勾选不变时复用上次的结果。
    let cached = |app: &App| {
        let compute = || app.commit.members_state(&status, members);
        rows.members_state(members, app.commit.checks(), compute)
    };
    assert_eq!(cached(&app), Some(true));
    app.press_commit(CommitTarget::DirCheck(pkg));
    assert!(!checked(&app, "model/a.rs"));
    assert_eq!(cached(&app), None);
    // 勾选不重排行：已排好的行继续复用，收起或展开目录后才重新排。
    assert!(std::rc::Rc::ptr_eq(&rows, &app.commit_rows()));

    // 点击目录行收起：其下的行不再出现；「全部展开」恢复。
    app.press_commit(CommitTarget::Dir(pkg));
    assert!(!app.commit_rows().contains(&file("pkg/z.rs", 1)));
    app.run_commit_more_command(crate::app::input::context_menu::MenuCommand::CommitExpandAll);
    assert!(app.commit_rows().contains(&file("pkg/z.rs", 1)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn right_clicking_a_changed_file_offers_diff_open_and_rollback() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-commit-menu", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // `make_repo` 修改过 `src/main.rs`：右键该文件所在的行。
    let main = root.join("src/main.rs");
    let index = app
        .git
        .status
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|e| e.path == main)
        .unwrap();
    let row = app
        .commit_hits
        .iter()
        .find(|(_, t)| *t == CommitTarget::File(index))
        .unwrap()
        .0;
    right_click(&mut app, (row.x + 120.0, row.y + row.h / 2.0));
    assert_eq!(
        app.context_menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::CommitFile(main.clone()))
    );
    assert_eq!(app.commit.selected.as_deref(), Some(main.as_path()));
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [
            MenuCommand::GitShowDiff,
            MenuCommand::GitOpenFile,
            MenuCommand::GitRollbackFile,
            MenuCommand::RevealInTree,
            MenuCommand::Reveal,
            MenuCommand::CopyPath,
            MenuCommand::CopyRelativePath,
        ]
    );

    // 「回滚…」只作用于该文件（没有窗口时不弹确认框）。
    let before = std::fs::read_to_string(&main).unwrap();
    app.run_menu_command(MenuCommand::GitRollbackFile);
    for _ in 0..200 {
        app.poll_commit();
        if std::fs::read_to_string(&main).unwrap() != before {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(std::fs::read_to_string(&main).unwrap(), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn commit_view_commits_only_the_checked_files() {
    let dir = search_folder("git-commit");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    // 模块关闭时：侧栏没有 Git 按钮，⌃⇧G 不响应。
    assert_eq!(app.sidebar_views().len(), 2);
    assert!(!app.run_action(Action::ShowCommit));

    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    assert_eq!(app.sidebar_views().last(), Some(&SidebarView::Commit));
    assert!(app.run_action(Action::ShowCommit));
    assert_eq!(app.sidebar_view, SidebarView::Commit);
    assert_eq!(app.focus, Focus::CommitMessage);
    paint(&mut app);

    // 「更改」一组里是改过的 `src/main.rs` 和已暂存的新文件 `src/new.rs`，都在 `src` 目录下面，
    // 默认都勾上。
    let status = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    let src = |group| CommitRow::Dir {
        group,
        dir: PathBuf::from("src"),
        label: "src".into(),
        depth: 0,
    };
    assert_eq!(
        rows[..],
        [
            CommitRow::Group(CommitGroup::Changes),
            src(CommitGroup::Changes),
            CommitRow::File { index: 0, depth: 1 },
            CommitRow::File { index: 1, depth: 1 },
        ]
    );
    assert!(app.commit.checked(&status.entries[0]));
    assert!(app.commit.checked(&status.entries[1]));
    assert_eq!(
        app.commit.group_state(&status, CommitGroup::Changes),
        Some(true)
    );

    // 点 `src/new.rs` 的勾选框；点 `src/main.rs` 那一行打开差异。
    let target =
        |app: &App, t: CommitTarget| app.commit_hits.iter().find(|(_, h)| *h == t).unwrap().0;
    let check = target(&app, CommitTarget::Check(1));
    click(&mut app, (check.x + 4.0, check.y + 4.0));
    assert!(!app.commit.checked(&status.entries[1]));
    let row = target(&app, CommitTarget::File(0));
    click(&mut app, (row.x + row.w / 2.0, row.y + 4.0));
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("main.rs · 差异")
    );

    // 键盘：焦点在列表里，↓ 选下一个文件，差异在同一个预览标签里换内容；空格切换勾选。
    let tabs = app.editor.documents.len();
    let key = |app: &mut App, key: NamedKey| app.commit_list_key(&Key::Named(key));
    assert_eq!(app.focus, Focus::CommitList);
    key(&mut app, NamedKey::ArrowDown);
    assert_eq!(
        app.commit.selected.as_deref(),
        Some(status.entries[1].path.as_path())
    );
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("new.rs · 差异")
    );
    key(&mut app, NamedKey::Space);
    assert!(app.commit.checked(&status.entries[1]));
    key(&mut app, NamedKey::ArrowUp);
    assert_eq!(
        app.commit.selected.as_deref(),
        Some(status.entries[0].path.as_path())
    );

    // 没写提交信息：不提交。
    app.git_commit();
    assert_eq!(app.git.status.as_ref().unwrap().entries.len(), 2);

    // 只提交 `src/new.rs`：先取消 `src/main.rs` 的勾选。
    paint(&mut app);
    let check = target(&app, CommitTarget::Check(0));
    click(&mut app, (check.x + 4.0, check.y + 4.0));
    app.focus = Focus::CommitMessage;
    // 提交信息可以多行：回车换行，⌘↩ 才提交。
    app.edit_field(|f| f.insert("加上 new.rs"));
    app.commit_message_key(&Key::Named(NamedKey::Enter), false, false);
    app.edit_field(|f| f.insert("正文"));
    assert_eq!(app.commit.message.text(), "加上 new.rs\n正文");
    app.commit_message_key(&Key::Named(NamedKey::Enter), true, false);
    assert!(!app.commit.busy);
    assert_eq!(app.commit.message.text(), "");
    let status = app.git.status.clone().unwrap();
    let left: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    let root = app.git.root.clone().unwrap();
    assert_eq!(left, [root.join("src/main.rs")]);
    // 用户取消过的勾选在提交后重置：剩下的 `src/main.rs` 重新处于勾选状态。
    assert!(app.commit.checked(&status.entries[0]));

    // 修改上一次提交：信息填进输入框，修改后提交。
    app.run_commit_button_command(crate::app::input::context_menu::MenuCommand::CommitAmend);
    assert_eq!(app.commit.message.text(), "加上 new.rs\n正文");
    app.commit.message.set_text("加上 new.rs 和 main.rs");
    app.git_commit();
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());
    paint(&mut app);

    // 回滚只处理勾上的文件：已修改的文件恢复 `HEAD` 中的内容，未跟踪的文件默认不勾，保留。
    let main = root.join("src/main.rs");
    let before = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, "changed\n").unwrap();
    std::fs::write(root.join("scratch.txt"), "x").unwrap();
    app.git_refresh();
    app.run_commit_more_command(crate::app::input::context_menu::MenuCommand::CommitRollback);
    assert_eq!(std::fs::read_to_string(&main).unwrap(), before);
    assert!(root.join("scratch.txt").exists());
    // 勾上以后，未跟踪的文件移到废纸篓（测试中直接删除）。
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    app.run_commit_more_command(crate::app::input::context_menu::MenuCommand::CommitRollback);
    assert!(!root.join("scratch.txt").exists());
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hovering_a_change_offers_open_and_revert() {
    let Some((mut app, root)) = git_app("git-commit-row-actions", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let main = root.join("src/main.rs");
    let entries = |app: &App| app.git.status.as_ref().unwrap().entries.clone();
    let index = entries(&app).iter().position(|e| e.path == main).unwrap();
    let find =
        |app: &App, t: CommitTarget| app.commit_hits.iter().find(|(_, h)| *h == t).map(|h| h.0);
    // 未悬停时不显示按钮；悬停该行时右端显示「打开文件」「回滚」，两者均有悬停提示。
    assert!(find(&app, CommitTarget::OpenFile(index)).is_none());
    let row = find(&app, CommitTarget::File(index)).unwrap();
    app.on_mouse_move(row.x + 120.0, row.y + row.h / 2.0);
    paint(&mut app);
    let open = find(&app, CommitTarget::OpenFile(index)).unwrap();
    let revert = find(&app, CommitTarget::RevertFile(index)).unwrap();
    assert!(open.right() <= revert.x && revert.right() <= row.right());
    app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
    assert_eq!(app.hover, Hover::Commit(CommitTarget::RevertFile(index)));
    assert!(app.hover.has_tooltip());

    app.press_commit(CommitTarget::OpenFile(index));
    let canonical = winc_editor::canonical_path(&main);
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(canonical.as_path())
    );
    assert_eq!(app.focus, Focus::Editor);
    // 没有窗口时不弹出确认框，直接回滚。
    app.press_commit(CommitTarget::RevertFile(index));
    app.git_refresh();
    assert!(entries(&app).iter().all(|e| e.path != main));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：悬停改动行时右端的「打开文件」「回滚」，鼠标位于「回滚」上（`commit-row-actions`）。
#[test]
#[ignore]
fn shoot_commit_row_actions() {
    let Some((mut app, root)) = git_app("git-commit-row-actions-shot", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let main = root.join("src/main.rs");
    let entries = app.git.status.as_ref().unwrap().entries.clone();
    let index = entries.iter().position(|e| e.path == main).unwrap();
    let find =
        |app: &App, t: CommitTarget| app.commit_hits.iter().find(|(_, h)| *h == t).unwrap().0;
    let row = find(&app, CommitTarget::File(index));
    app.on_mouse_move(row.x + 120.0, row.y + row.h / 2.0);
    paint(&mut app);
    let revert = find(&app, CommitTarget::RevertFile(index));
    app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
    app.tooltip_since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
    shoot(&mut app, "commit-row-actions");
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：提交视图，深浅两套配色。
#[test]
#[ignore]
fn shoot_commit_view() {
    for dark in [false, true] {
        let dir = search_folder(if dark { "commit-dark" } else { "commit-light" });
        if !long_repo(&dir) {
            return;
        }
        std::fs::write(dir.join("src/config.rs"), "pub struct Config;\n").unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/notes.md"), "# notes\n").unwrap();
        std::fs::create_dir_all(dir.join("src/core/model")).unwrap();
        std::fs::write(dir.join("src/core/model/user.rs"), "pub struct User;\n").unwrap();
        std::fs::write(dir.join("src/core/model/order.rs"), "pub struct Order;\n").unwrap();
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        app.edit_field(|f| f.insert("修复配置读取\n\n启动时配置文件不存在不再错误，改用默认值。"));
        // 选中 `src/main.rs`：列表中高亮，右侧预览该文件的差异。
        let main = app.editor.root().unwrap().join("src/main.rs");
        let index = app
            .git
            .status
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.path == main);
        paint(&mut app);
        app.press_commit(CommitTarget::File(index.unwrap()));
        app.focus = Focus::CommitMessage;
        // 鼠标停在标题行的「Git 日志」按钮上，提示已显示。
        let log = app.commit_parts().tools[0].0;
        app.on_mouse_move(log.x + 4.0, log.y + 4.0);
        app.tooltip_since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, if dark { "commit-dark" } else { "commit-light" });
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 截图：没有改动的提交视图（提交信息在上、「提交 ⌄」、居中的「没有更改」），深浅两套配色。
#[test]
#[ignore]
fn shoot_commit_empty() {
    for dark in [false, true] {
        let name = if dark {
            "commit-empty-dark"
        } else {
            "commit-empty-light"
        };
        let dir = search_folder(name);
        if !make_repo(&dir) {
            return;
        }
        git_in(&dir, &["add", "-A"]);
        git_in(&dir, &["commit", "-q", "-m", "all"]);
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn switching_the_module_off_takes_focus_out_of_the_commit_view() {
    let Some((mut app, root)) = git_app("git-focus", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run_action(Action::ShowCommit);
    assert_eq!(app.focus, Focus::CommitMessage);
    app.settings_toggle(Setting::GitModule);
    assert_eq!(app.sidebar_view, SidebarView::Files);
    assert_eq!(app.focus, Focus::Editor);
    // 键入的字符不会进入已隐藏的提交信息。
    app.edit_field(|f| f.insert("x"));
    assert_eq!(app.commit.message.text(), "");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn committing_a_checked_file_saves_its_unsaved_edits_first() {
    let Some((mut app, root)) = git_app("git-commit-dirty", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run(winc_editor::Command::Insert("// 还没保存\n".into()));
    assert!(app.editor.active().unwrap().is_dirty());
    app.run_action(Action::ShowCommit);
    app.commit.message.set_text("带上没保存的修改");
    app.git_commit();
    // 提交的是编辑器中看到的内容（整个勾选的文件也会先保存），而不是磁盘上的旧内容。
    assert!(!app.editor.active().unwrap().is_dirty());
    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("// 还没保存"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn clicks_on_rows_drawn_before_a_refresh_are_ignored() {
    let Some((mut app, root)) = git_app("git-stale-click", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let before = app
        .commit
        .checked(&app.git.status.as_ref().unwrap().entries[0]);
    // 状态在绘制以后刷新过：按上一帧记下的行号点，不处理。
    git_in(&root, &["add", "src/new.rs"]);
    app.git_refresh();
    app.press_commit(CommitTarget::Check(0));
    assert_eq!(
        app.commit
            .checked(&app.git.status.as_ref().unwrap().entries[0]),
        before
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_more_menu_greys_out_what_does_not_apply() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-commit-more", false) else {
        return;
    };
    std::fs::write(root.join("new.txt"), "x").unwrap();
    git_in(&root, &["add", "-A"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let enabled = |app: &App, command| {
        app.commit_more_menu()
            .contains(&MenuEntry::Item(command, true))
    };
    // 先清空勾选：列表初始全部展开，且没有任何文件被勾选。
    for group in CommitGroup::ALL {
        for _ in 0..3 {
            let status = app.git.status.clone().unwrap();
            if app.commit.group_state(&status, group).is_none() {
                break;
            }
            app.press_commit(CommitTarget::GroupCheck(group));
        }
    }
    assert!(enabled(&app, MenuCommand::CommitRefresh));
    assert!(!enabled(&app, MenuCommand::CommitExpandAll));
    assert!(enabled(&app, MenuCommand::CommitCollapseAll));
    assert!(!enabled(&app, MenuCommand::CommitRollback));

    app.run_commit_more_command(MenuCommand::CommitCollapseAll);
    let groups = [CommitGroup::Changes];
    assert!(groups.iter().all(|g| app.commit.group_collapsed(*g)));
    assert!(enabled(&app, MenuCommand::CommitExpandAll));
    assert!(!enabled(&app, MenuCommand::CommitCollapseAll));
    app.run_commit_more_command(MenuCommand::CommitExpandAll);
    assert!(groups.iter().all(|g| !app.commit.group_collapsed(*g)));

    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    assert!(enabled(&app, MenuCommand::CommitRollback));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_message_grows_with_its_text_and_the_list_follows() {
    let Some((mut app, root)) = git_app("git-message-grow", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 自上而下依次是提交信息、「提交 ⌄」和改动列表。空信息时高度为一行，与搜索框同为 24。
    let one = app.commit_parts();
    assert_eq!(one.message.h, 24.0);
    assert_eq!(one.message.h, app.panel_parts().query.h);
    assert!(one.commit.y > one.message.bottom());
    assert_eq!(one.commit_menu.x, one.commit.right());
    assert!(one.list.y > one.commit.bottom());

    // 写入三行时框高增加两行，按钮和列表随之下移，留白不变。
    app.edit_field(|f| f.insert("修复配置读取\n\n启动时不再报错"));
    paint(&mut app);
    let three = app.commit_parts();
    assert_eq!(three.message.h, 3.0 * 20.0 + 4.0);
    assert_eq!(three.list.y - one.list.y, 2.0 * 20.0);

    // 行数超过八行后不再长高，改为在框内滚动。
    app.edit_field(|f| f.insert(&"\n一行".repeat(20)));
    paint(&mut app);
    assert_eq!(app.commit_parts().message.h, 8.0 * 20.0 + 4.0);

    // 清空后框高从八行过渡回一行：途中介于两者之间，文字仍按一行的高度排版。
    use crate::app::animation::{Clock, TREE_ANIMATION};
    app.commit.message.set_text("");
    paint(&mut app);
    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.commit.message_motion = Some((8.0 * 20.0 + 4.0, half));
    let parts = app.commit_parts();
    assert!(parts.message.h > 24.0 && parts.message.h < 8.0 * 20.0 + 4.0);
    assert_eq!(parts.message_settled.h, 24.0);
    assert!(app.animating());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_button_menu_pushes_after_committing_or_amends() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    // 与右键菜单的测试使用不同的仓库：两个测试并行运行，同一仓库会互相改写和删除文件。
    let Some((mut app, root)) = git_app("git-commit-button-menu", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitPush, true)));
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitAmend, true)));

    // 「修改上一次提交」启用时打勾，按钮写「修改提交」。
    assert!(!app.menu_checked(MenuCommand::CommitAmend));
    app.run_commit_button_command(MenuCommand::CommitAmend);
    assert!(app.commit.amend);
    assert!(app.menu_checked(MenuCommand::CommitAmend));
    app.run_commit_button_command(MenuCommand::CommitAmend);
    assert!(!app.commit.amend);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn scrolled_changes_keep_their_group_and_folders_pinned_on_top() {
    let Some(dir) = deep_changes_repo("git-sticky") else {
        return;
    };
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    paint(&mut app);
    let rows = app.commit_rows();
    // 没滚动时什么也不固定。
    assert!(app.commit_sticky(&rows).is_empty());

    // 滚到 `b` 目录中间：上面固定着组、`assets/pets`、`b` 三层。
    let b = rows
        .iter()
        .position(|r| matches!(r, CommitRow::Dir { label, .. } if label == "b"))
        .unwrap();
    app.commit.scroll = (b + 4) as f32 * COMMIT_ROW;
    paint(&mut app);
    let sticky = app.commit_sticky(&rows);
    let pinned: Vec<usize> = sticky.iter().map(|s| s.row).collect();
    assert_eq!(pinned.len(), 3);
    assert!(matches!(
        rows[pinned[0]],
        CommitRow::Group(CommitGroup::Changes)
    ));
    assert_eq!(pinned[2], b);
    let list = app.commit_parts().list;
    assert_eq!(sticky[2].rect.y, list.y + 2.0 * COMMIT_ROW);

    // 点击固定的 `b` 行：列表滚到 `b` 所在行并保持不变，`b` 收起。
    let at = sticky[2].rect;
    app.on_mouse_move(at.x + 120.0, at.y + at.h / 2.0);
    assert_eq!(app.hover, Hover::Commit(CommitTarget::Sticky(b, 2)));
    click(&mut app, (at.x + 120.0, at.y + at.h / 2.0));
    // 收起后列表变短，内容放得下时滚回顶部。
    let settled = ((b - 2) as f32 * COMMIT_ROW).min(app.commit_max_scroll());
    assert_eq!(app.commit.scroll, settled);
    let CommitRow::Dir {
        group, dir: folder, ..
    } = &rows[b]
    else {
        panic!("第 {b} 行不是目录");
    };
    assert!(folder.ends_with("assets/pets/b"));
    assert!(app.commit.dir_collapsed(*group, folder));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn option_clicking_a_changed_folder_collapses_and_expands_it_all_the_way() {
    let Some(dir) = deep_changes_repo("git-recursive") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    paint(&mut app);
    let dirs = |app: &App| -> Vec<(PathBuf, bool)> {
        app.commit_rows()
            .iter()
            .filter_map(|row| match row {
                CommitRow::Dir { group, dir, .. } => {
                    Some((dir.clone(), app.commit.dir_collapsed(*group, dir)))
                }
                _ => None,
            })
            .collect()
    };
    let first = app
        .commit_rows()
        .iter()
        .position(|row| matches!(row, CommitRow::Dir { .. }))
        .unwrap();
    let inner = dirs(&app).len();
    assert!(inner > 1);
    let alt = winit::keyboard::ModifiersState::ALT;

    // 递归收起：只剩最上面的目录行，且收起。
    app.modifiers = alt;
    app.press_commit(CommitTarget::Dir(first));
    assert_eq!(dirs(&app).len(), 1);
    assert!(dirs(&app)[0].1);

    // 普通单击只展开一层：下一级目录还是收起的。
    app.modifiers = winit::keyboard::ModifiersState::empty();
    app.press_commit(CommitTarget::Dir(first));
    assert!(dirs(&app).iter().skip(1).all(|(_, collapsed)| *collapsed));

    // 先收起再递归展开：各级全部展开，行数回到最初。
    app.press_commit(CommitTarget::Dir(first));
    app.modifiers = alt;
    app.press_commit(CommitTarget::Dir(first));
    app.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(dirs(&app).len(), inner);
    assert!(dirs(&app).iter().all(|(_, collapsed)| !collapsed));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_a_visible_change_under_the_pinned_rows_does_not_scroll() {
    let Some(dir) = deep_changes_repo("git-sticky-click") else {
        return;
    };
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let rows = app.commit_rows();
    let list = app.commit_parts().list;
    let b = rows
        .iter()
        .position(|r| matches!(r, CommitRow::Dir { label, .. } if label == "b"))
        .unwrap();
    app.commit.scroll = (b + 4) as f32 * COMMIT_ROW;
    paint(&mut app);
    let covered = app
        .commit_sticky(&rows)
        .iter()
        .map(|s| s.rect.bottom())
        .fold(list.y, f32::max);
    // 固定行下方露出的第一个文件：点击该行，列表不动，固定的行也不变。
    let before = app
        .commit_sticky(&rows)
        .iter()
        .map(|s| s.row)
        .collect::<Vec<_>>();
    let (i, _) = rows
        .iter()
        .enumerate()
        .find(|(i, r)| {
            matches!(r, CommitRow::File { .. })
                && list.y + *i as f32 * COMMIT_ROW - app.commit.scroll >= covered
        })
        .unwrap();
    let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;
    let scroll = app.commit.scroll;
    click(&mut app, (list.x + 160.0, y));
    paint(&mut app);
    assert_eq!(app.commit.scroll, scroll);
    let after = app
        .commit_sticky(&rows)
        .iter()
        .map(|s| s.row)
        .collect::<Vec<_>>();
    assert_eq!(after, before);

    // 被向上推的固定行只占自己那一格：可点击区域不伸入上一层的固定行。
    for step in 0..(rows.len() * 4) {
        app.commit.scroll = step as f32 * COMMIT_ROW / 4.0;
        paint(&mut app);
        for s in app.commit_sticky(&rows) {
            let slot = list.y + s.level as f32 * COMMIT_ROW;
            for (rect, target) in &app.commit_hits {
                if *target == CommitTarget::Sticky(s.row, s.level) {
                    assert!(rect.y >= slot - 0.01, "{step}: {rect:?} above {slot}");
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_layout_button_switches_the_changes_between_a_tree_and_a_flat_list() {
    let Some(dir) = deep_changes_repo("git-flat") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 默认平铺：没有目录行，每个文件一行，按路径排；切换按钮在「Git 日志」后面。
    assert!(app.commit.flat());
    let flat = app.commit_rows();
    assert!(!flat.iter().any(|r| matches!(r, CommitRow::Dir { .. })));
    let files = flat
        .iter()
        .filter(|r| matches!(r, CommitRow::File { depth: 0, .. }))
        .count();
    assert_eq!(files, 27);
    let tools = app.commit_parts().tools;
    assert_eq!(tools[0].1, CommitTarget::Log);
    assert_eq!(tools[1].1, CommitTarget::Layout);
    assert!(tools[1].0.x > tools[0].0.x && tools[1].0.x < tools[2].0.x);
    // 平铺时文件不缩进：勾选框和组标题的勾选框在同一列。
    let check_x = |app: &App, want: fn(&CommitTarget) -> bool| {
        let (rect, _) = app.commit_hits.iter().find(|(_, t)| want(t)).unwrap();
        rect.x + rect.w / 2.0
    };
    let group = check_x(&app, |t| matches!(t, CommitTarget::GroupCheck(_)));
    assert_eq!(
        check_x(&app, |t| matches!(t, CommitTarget::Check(_))),
        group
    );

    // 点切换按钮，按目录排成树；记进状态，以后打开的窗口、下次启动沿用。
    let button = tools[1].0;
    let toggle = |app: &mut App| click(app, (button.x + button.w / 2.0, button.y + button.h / 2.0));
    toggle(&mut app);
    assert!(!app.commit.flat());
    assert!(
        app.commit_rows()
            .iter()
            .any(|r| matches!(r, CommitRow::Dir { .. }))
    );
    assert!(app.state.commit_tree);
    // 再点回到平铺。
    toggle(&mut app);
    assert!(app.commit.flat());
    assert!(!app.state.commit_tree);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：大量深层文件的改动列表：滚到中间时固定在顶部的组、目录和缩进参考线，以及平铺列表。
#[test]
#[ignore]
fn shoot_deep_changes() {
    for dark in [false, true] {
        let name = if dark { "deep-dark" } else { "deep-light" };
        let Some(dir) = deep_changes_repo(name) else {
            return;
        };
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        app.commit.scroll = 12.0 * COMMIT_ROW;
        shoot(&mut app, &format!("{name}-tree"));
        // 固定的行被下一个目录向上推：只在自己那一格内移动。
        let rows = app.commit_rows();
        let list = app.commit_parts().list;
        let pushed = (0..rows.len() * 4)
            .map(|q| q as f32 * COMMIT_ROW / 4.0)
            .find(|&scroll| {
                app.commit.scroll = scroll;
                app.commit_sticky(&rows)
                    .iter()
                    .any(|s| s.level > 0 && s.rect.y < list.y + s.level as f32 * COMMIT_ROW - 8.0)
            });
        if let Some(scroll) = pushed {
            app.commit.scroll = scroll;
            shoot(&mut app, &format!("{name}-pushed"));
        }
        app.toggle_commit_layout();
        app.commit.scroll = 0.0;
        shoot(&mut app, &format!("{name}-flat"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn folding_a_changes_folder_slides_its_rows_like_the_file_tree() {
    use crate::app::Clock;
    use crate::app::git::commit::CommitMotion;

    let Some(dir) = deep_changes_repo("git-commit-motion") else {
        return;
    };
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let before = app.commit_rows();
    let a = before
        .iter()
        .position(|r| matches!(r, CommitRow::Dir { label, .. } if label == "a"))
        .unwrap();
    app.press_commit(CommitTarget::Dir(a));
    let after = app.commit_rows();
    assert_eq!(before.len() - after.len(), 12);

    // 收起到一半：`a` 下面的 12 行收回一半，`b` 上移 6 行。动画期间只记录 `a` 及其上方各行
    // 的命中区域，正在移动的行不记录。
    let half = Clock::frozen(
        std::time::Duration::from_millis(1000),
        std::time::Duration::from_millis(2000),
    );
    app.commit.motion = Some(CommitMotion {
        rows: before.clone(),
        parent: a,
        count: 12,
        expanding: false,
        clock: half,
    });
    let motion = app.commit.motion.as_ref().unwrap();
    let hidden = motion.hidden();
    assert!(hidden > 0.0 && hidden < 12.0 * COMMIT_ROW);
    paint(&mut app);
    assert!(
        app.commit_hits
            .iter()
            .any(|(_, t)| *t == CommitTarget::Dir(a))
    );
    assert!(
        !app.commit_hits
            .iter()
            .any(|(_, t)| matches!(t, CommitTarget::Dir(r) if *r > a))
    );
    shoot(&mut app, "commit-folding");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_changes_list_scrollbar_can_be_dragged() {
    use crate::layout::Scrollbar;

    let Some(dir) = deep_changes_repo("git-commit-scrollbar") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let max = app.commit_max_scroll();
    assert!(max > 0.0);
    let selected = app.commit.selected.clone();

    // 行的右端（状态字母所在处）就是滚动条：悬停认作滚动条，按下拖到底再拖回顶部，不选中文件。
    let list = app.commit_parts().list;
    let x = list.right() - 3.0;
    app.on_mouse_move(x, list.y + 10.0);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::Commit));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, list.bottom() + 40.0);
    assert_eq!(app.commit.scroll, max);
    app.on_mouse_move(x, list.y - 40.0);
    assert_eq!(app.commit.scroll, 0.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.commit.selected, selected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hovering_a_long_change_name_shows_it_in_full_with_its_buttons_after_it() {
    let Some((dir, long)) = long_name_repo("git-commit-overflow") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let rows = app.commit_rows();
    let status = app.git.status.clone().unwrap();
    let (i, index) = rows
        .iter()
        .enumerate()
        .find_map(|(i, r)| match r {
            CommitRow::File { index, .. } if status.entries[*index].path.ends_with(&long) => {
                Some((i, *index))
            }
            _ => None,
        })
        .unwrap();
    let list = app.commit_parts().list;
    let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;

    // 未悬停时名称截短，没有伸出的部分。
    assert!(app.commit.overflow.is_none());

    // 停在这一行上：写全名，伸出侧栏；「打开文件」「回滚」跟在全名之后，也在伸出的那一段里。
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    let overflow = app.commit.overflow.expect("long name spills");
    assert_eq!(overflow.row, i);
    let spill = overflow.spill.expect("past the sidebar");
    assert_eq!(spill.x, list.right());
    let (open, revert) = overflow.actions.expect("buttons after the name");
    assert!(open.x > list.right() && revert.right() <= spill.right());
    let open_hit = app
        .commit_hits
        .iter()
        .find(|(_, t)| *t == CommitTarget::OpenFile(index))
        .map(|(r, _)| *r);
    assert_eq!(open_hit, Some(open));

    // 移到伸出去的按钮上：仍写全名，悬停在该按钮上。
    let (ox, oy) = (open.x + open.w / 2.0, open.y + open.h / 2.0);
    app.on_mouse_move(ox, oy);
    paint(&mut app);
    assert_eq!(app.hover, Hover::Commit(CommitTarget::OpenFile(index)));
    assert!(app.commit.overflow.is_some_and(|o| o.row == i));

    // 展开收起的动画途中：该行就是被收起的行时位置不变，仍写全名；位于被收起的行下方、正在
    // 移动时恢复截短显示。
    let motion = |parent: usize| crate::app::git::commit::CommitMotion {
        rows: rows.clone(),
        parent,
        count: 1,
        expanding: false,
        clock: crate::app::animation::Clock::frozen(
            std::time::Duration::from_millis(1000),
            std::time::Duration::from_millis(2000),
        ),
    };
    app.on_mouse_move(list.x + 120.0, y);
    app.commit.motion = Some(motion(i));
    paint(&mut app);
    assert!(app.commit.overflow.is_some_and(|o| o.row == i));
    if i > 0 {
        app.commit.motion = Some(motion(i - 1));
        paint(&mut app);
        assert!(app.commit.overflow.is_none());
    }
    app.commit.motion = None;

    // 移开：恢复截短显示。
    app.on_mouse_move(list.x + 120.0, list.bottom() - 4.0);
    paint(&mut app);
    assert!(app.commit.overflow.is_none_or(|o| o.row != i));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_overflow() {
    for dark in [false, true] {
        let name = if dark {
            "overflow-dark"
        } else {
            "overflow-light"
        };
        let Some((dir, long)) = long_name_repo(name) else {
            return;
        };
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        let rows = app.commit_rows();
        let status = app.git.status.clone().unwrap();
        let i = rows
            .iter()
            .position(|r| {
                matches!(r, CommitRow::File { index, .. }
                    if status.entries[*index].path.ends_with(&long))
            })
            .unwrap();
        let list = app.commit_parts().list;
        // 滚到该行位于列表上部，悬停后截图。
        let scroll = (i as f32 - 3.0) * COMMIT_ROW;
        app.commit.scroll = scroll.clamp(0.0, app.commit_max_scroll());
        paint(&mut app);
        let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;
        app.on_mouse_move(list.x + 120.0, y);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn switching_folders_keeps_the_commit_view_while_there_is_a_repository() {
    let first = search_folder("switch-git-a");
    let second = search_folder("switch-git-b");
    let plain = search_folder("switch-git-plain");
    if !make_repo(&first) || !make_repo(&second) {
        return;
    }
    let mut app = app_with(&first, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.sidebar_view, SidebarView::Commit);

    // 换到另一个仓库：还在提交视图，焦点在提交信息里。
    app.switch_folder(&second);
    assert_eq!(app.sidebar_view, SidebarView::Commit);
    assert_eq!(app.focus, Focus::CommitMessage);

    // 换到不是仓库的目录：没有提交视图了，回到文件树。
    app.switch_folder(&plain);
    assert_eq!(app.sidebar_view, SidebarView::Files);
    assert_eq!(app.focus, Focus::Tree);
    for dir in [first, second, plain] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn change_list_keys_jump_to_the_ends_and_page() {
    let Some(dir) = deep_changes_repo("git-commit-paging") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.focus = Focus::CommitList;
    let key = |app: &mut App, key: NamedKey| app.commit_list_key(&Key::Named(key));
    let files = app.commit_files();
    let rows = app.commit_rows();
    let row_of = |app: &App| {
        let selected = app.commit.selected.clone();
        rows.iter()
            .position(|r| match r {
                CommitRow::File { index, .. } => {
                    let status = app.git.status.as_ref().unwrap();
                    Some(&status.entries[*index].path) == selected.as_ref()
                }
                _ => false,
            })
            .unwrap()
    };

    // `Home`、`End` 移到第一个、最后一个文件（跳过组、目录行），列表随之滚动到可见位置。
    key(&mut app, NamedKey::End);
    assert_eq!(app.commit.selected.as_ref(), files.last());
    assert!(app.commit.scroll > 0.0);
    key(&mut app, NamedKey::Home);
    assert_eq!(app.commit.selected.as_ref(), files.first());
    assert_eq!(app.commit.scroll, 0.0);

    // 翻页距离至少为可见整行数减一，且落在文件上；到达末尾时停在最后一个文件。
    let page = (app.commit_parts().list.h / COMMIT_ROW).floor() as usize - 1;
    let start = row_of(&app);
    key(&mut app, NamedKey::PageDown);
    let next = row_of(&app);
    assert!(next >= start + page, "{start} → {next}，一页 {page}");
    key(&mut app, NamedKey::PageUp);
    assert!(row_of(&app) <= next - page);
    for _ in 0..rows.len() {
        key(&mut app, NamedKey::PageDown);
    }
    assert_eq!(app.commit.selected.as_ref(), files.last());

    // 按住 `⇧` 翻页时扩展选区。
    key(&mut app, NamedKey::Home);
    app.modifiers = ModifiersState::SHIFT;
    key(&mut app, NamedKey::PageDown);
    assert!(app.commit_multi_selected());
    app.modifiers = ModifiersState::empty();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_commit_messages_scroll_with_the_wheel() {
    // 回归：提交信息超过最大行数时，滚轮无法滚动，每帧都回到光标所在的行。
    let Some((mut app, root)) = git_app("git-commit-message-scroll", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    let text: Vec<String> = (1..=30).map(|i| format!("第 {i} 行")).collect();
    app.commit.message.set_text(&text.join("\n"));
    paint(&mut app);
    // 光标在末尾，框内露出的是最后几行。
    let bottom = app.commit.message.view.scroll;
    assert!(bottom > 0.0);

    let message = app.commit_parts().message;
    app.mouse = (message.x + 20.0, message.y + 20.0);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, 2.0));
    paint(&mut app);
    let up = app.commit.message.view.scroll;
    assert!(up < bottom);
    // 往上滚过头停在第一行，不出现负的滚动量。
    for _ in 0..20 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, 5.0));
    }
    paint(&mut app);
    assert_eq!(app.commit.message.view.scroll, 0.0);
    // 往下滚过头停在最后一行。
    for _ in 0..20 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, -5.0));
    }
    paint(&mut app);
    assert_eq!(app.commit.message.view.scroll, bottom);

    // 键入文字后回到光标所在的行。
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, 5.0));
    paint(&mut app);
    app.focus = Focus::CommitMessage;
    let end = app.commit.message.text().len();
    app.edit_field(|f| {
        f.set_caret(end, false);
        f.insert("！")
    });
    paint(&mut app);
    assert_eq!(app.commit.message.view.scroll, bottom);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_last_local_commit_can_be_undone() {
    let Some((mut app, root)) = git_app("git-undo-commit", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.git_refresh();
    // 没有上游：上一次提交还没推送，可以撤销。
    let first = app.git.status.as_ref().unwrap().oid.clone().unwrap();
    assert_eq!(
        app.commit_undoable().map(|l| l.oid.clone()),
        Some(first.clone())
    );

    app.commit.message.set_text("加上改动\n\n正文");
    app.git_commit();
    let second = app.git.status.as_ref().unwrap().oid.clone().unwrap();
    assert_ne!(second, first);
    assert_eq!(app.commit_undoable().unwrap().subject, "加上改动");
    paint(&mut app);
    // 「上一次提交」固定在视图底部，改动列表在它上面结束。
    let parts = app.commit_parts();
    let (bar, button) = parts.undo.unwrap();
    assert_eq!(bar.bottom(), app.layout.sidebar.bottom());
    assert_eq!(parts.list.bottom(), bar.y);
    assert!(
        app.commit_hits
            .iter()
            .any(|(r, t)| *t == CommitTarget::LastCommit && *r == button)
    );

    // 撤销：分支退回上一个提交，改动回到列表里，提交信息填回输入框。
    app.press_commit(CommitTarget::LastCommit);
    app.run_menu_command(MenuCommand::CommitCopyOid);
    let oid = app.commit_undoable().unwrap().oid.clone();
    assert_eq!(app.copied.as_deref(), Some(oid.as_str()));
    app.press_commit(CommitTarget::LastCommit);
    let commands: Vec<_> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [MenuCommand::CommitUndo, MenuCommand::CommitCopyOid]
    );
    app.run_menu_command(MenuCommand::CommitUndo);
    // 先用提示框询问，确认按钮不标记为危险操作；确认后才撤销。
    assert_eq!(
        app.git.status.as_ref().unwrap().oid.as_deref(),
        Some(oid.as_str())
    );
    assert!(!app.alert.as_ref().unwrap().buttons[0].destructive);
    press_alert(&mut app, 0);
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.oid.as_deref(), Some(first.as_str()));
    assert!(!status.entries.is_empty());
    assert_eq!(app.commit.message.text(), "加上改动\n\n正文");

    // 推送到上游以后不能撤销。
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    assert!(app.commit_undoable().is_none());
    assert!(app.commit_parts().undo.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn partly_staged_files_commit_only_their_staged_part() {
    // 回归：在终端里只暂存了一部分的文件，勾上提交时整个文件都进了提交，暂存的选择被忽略。
    let Some((mut app, root)) = git_app("git-commit-partly-staged", false) else {
        return;
    };
    let main = root.join("src/main.rs");
    std::fs::write(&main, "staged\n").unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    std::fs::write(&main, "staged\nnot staged\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let index = status.entries.iter().position(|e| e.path == main).unwrap();
    let entry = &status.entries[index];
    assert!(entry.partly_staged());
    // 勾选框是「−」，悬停时说明仅提交已暂存的部分。
    assert_eq!(app.commit.file_state(entry), Some(false));
    let check = app
        .commit_hits
        .iter()
        .find(|(_, t)| *t == CommitTarget::Check(index))
        .unwrap()
        .0;
    app.on_mouse_move(check.x + 4.0, check.y + 4.0);
    assert!(app.hover.has_tooltip());
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(
        tip.as_deref(),
        Some("仅提交已暂存的部分，其余更改保留在工作区中")
    );

    app.commit.message.set_text("只提交暂存的");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout), "staged\n");
    // 没暂存的那一行还在工作区里，文件仍然有改动。
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        "staged\nnot staged\n"
    );
    let status = app.git.status.clone().unwrap();
    let entry = status.entries.iter().find(|e| e.path == main).unwrap();
    assert!(!entry.partly_staged());
    let _ = std::fs::remove_dir_all(&root);
}

/// 同步控件是一个按钮：提交数写在图标左边，运行时图标在原位转动，操作名写在提示里。
#[test]
fn the_sync_control_stays_one_button() {
    let Some((mut app, root)) = git_app("git-sync-running", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let sync = |app: &App| {
        (app.commit_hits.iter())
            .find(|(_, t)| *t == CommitTarget::Sync)
            .unwrap()
            .0
    };
    let tool = app.commit_parts().tools[2].0;
    let ahead = sync(&app);
    assert_eq!(ahead.right(), tool.right());
    assert!(ahead.w > tool.w);

    // 运行时宽度不变，分支名不会跟着动。
    app.branch.running = Some("获取".into());
    paint(&mut app);
    assert_eq!(sync(&app), ahead);
    app.on_mouse_move(ahead.x + 4.0, ahead.y + 4.0);
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(tip.as_deref(), Some("正在获取…"));

    // 不在分支上时只有图标，操作仍在同一个位置显示。
    app.branch.running = None;
    git_in(&root, &["checkout", "-q", "--detach"]);
    app.git_refresh();
    paint(&mut app);
    assert_eq!(sync(&app).w, tool.w);
    assert_eq!(sync(&app).right(), tool.right());

    let _ = std::fs::remove_dir_all(&root);
}

/// 真实的鼠标移动会开始提示的计时；提示写明要做的事和提交数。
#[test]
fn the_sync_control_explains_what_it_will_do() {
    let Some((mut app, root)) = git_app("git-sync-tips", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "a"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "b"]);
    git_in(&root, &["checkout", "-q", "-b", "other", "HEAD~1"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "c"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "d"]);
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["checkout", "-q", "main"]);
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(matches!(
        app.git_sync_state(),
        SyncState::Both {
            ahead: 1,
            behind: 2
        }
    ));
    let rect = (app.commit_hits.iter())
        .find(|(_, t)| *t == CommitTarget::Sync)
        .unwrap()
        .0;
    app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
    assert!(app.tooltip_since.is_some());
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(
        tip.as_deref(),
        Some("与 origin/main 同步：拉取 2 个提交，推送 1 个提交")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 没有远程时不显示控件；配了远程、上游被删除时都是发布。
#[test]
fn the_sync_control_follows_the_remotes() {
    let Some((mut app, root)) = git_app("git-sync-remotes", false) else {
        return;
    };
    let sync = |app: &App| {
        app.commit_hits
            .iter()
            .any(|(_, t)| *t == CommitTarget::Sync)
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "a"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git_sync_state(), SyncState::NoRemote);
    assert!(!sync(&app));

    // 操作的进度仍然显示在这里。
    app.branch.running = Some("签出".into());
    paint(&mut app);
    assert!(sync(&app));
    app.branch.running = None;

    // 刚配好的远程还没有任何远程分支，一样可以发布。
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    app.git_refresh();
    paint(&mut app);
    assert_eq!(app.git_sync_state(), SyncState::Publish);
    assert!(sync(&app));

    // 上游被删除后不再写已同步。
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    assert_eq!(app.git_sync_state(), SyncState::InSync);
    git_in(&root, &["update-ref", "-d", "refs/remotes/origin/main"]);
    app.git_refresh();
    assert_eq!(app.git_sync_state(), SyncState::Publish);

    let _ = std::fs::remove_dir_all(&root);
}

/// 上游被删除的分支：已经发布过的提交不能撤销；发布时推到它原来配置的远程，不是排在第一个的远程。
#[test]
fn a_deleted_upstream_keeps_its_commits_and_its_remote() {
    let Some((mut app, root)) = git_app("git-sync-gone", false) else {
        return;
    };
    let bare = |name: &str| root.with_extension(name);
    let (first, wanted) = (bare("gone-a.git"), bare("gone-z.git"));
    for (name, dir) in [("a-fork", &first), ("z-origin", &wanted)] {
        let _ = std::fs::remove_dir_all(dir);
        git_in(&root, &["init", "-q", "--bare", dir.to_str().unwrap()]);
        git_in(&root, &["remote", "add", name, dir.to_str().unwrap()]);
    }
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "one"]);
    git_in(&root, &["push", "-q", "-u", "z-origin", "main"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "two"]);
    git_in(&root, &["push", "-q", "z-origin", "main"]);
    git_in(&wanted, &["update-ref", "-d", "refs/heads/main"]);
    git_in(&root, &["update-ref", "-d", "refs/remotes/z-origin/main"]);
    app.git_refresh();
    paint(&mut app);
    let branch = (app.git.branches.iter()).find(|b| b.current).unwrap();
    assert!(branch.upstream.is_none() && branch.gone);
    let oid = app.git.status.as_ref().unwrap().oid.clone().unwrap();
    app.commit.last = Some(winc_editor::git::LastCommit {
        oid,
        subject: "two".to_owned(),
        message: String::new(),
        time: 0,
        local: String::new(),
    });
    assert!(app.commit_undoable().is_none());
    app.commit.last = None;

    assert_eq!(app.git_sync_state(), SyncState::Publish);
    app.press_commit(CommitTarget::Sync);
    let remote_has = |dir: &Path| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--verify", "--quiet", "refs/heads/main"])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    assert!(remote_has(&wanted));
    assert!(!remote_has(&first));

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&first);
    let _ = std::fs::remove_dir_all(&wanted);
}

/// 操作成功后同步控件显示对勾，随后回到当前状态；失败和新操作都不留对勾。
#[test]
fn the_sync_control_confirms_a_finished_operation() {
    let Some((mut app, root)) = git_app("git-sync-done", false) else {
        return;
    };
    let remote = root.with_extension("done-remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "one"]);
    git_in(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_in(&root, &["push", "-q", "-u", "origin", "main"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);

    app.press_commit(CommitTarget::Sync);
    assert!(app.branch.finished.is_some());

    // 新操作开始时不再显示上一个的结果。
    app.branch.finished = Some(Clock::stopped(COMMITTED_SHOWN, Duration::from_millis(800)));
    git_in(
        &root,
        &["remote", "set-url", "origin", "/nonexistent/repo.git"],
    );
    app.press_commit(CommitTarget::Sync);
    assert!(app.branch.finished.is_none());
    assert!(app.branch.running.is_none());

    // 中途是不透明的对勾，两端完全透明。
    let at = |ms| Clock::stopped(COMMITTED_SHOWN, Duration::from_millis(ms));
    assert_eq!(at(0).pulse(TREE_ANIMATION), 0.0);
    assert_eq!(at(800).pulse(TREE_ANIMATION), 1.0);
    assert_eq!(at(1600).pulse(TREE_ANIMATION), 0.0);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&remote);
}

/// 和上游一致时点击同步控件获取远程更新：提示写明这一点，获取到的新提交随后显示出来。
#[test]
fn the_sync_control_fetches_when_in_sync() {
    let Some((mut app, root)) = git_app("git-sync-fetch", false) else {
        return;
    };
    let remote = root.with_extension("fetch-remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "one"]);
    git_in(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_in(&root, &["push", "-q", "-u", "origin", "main"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "two"]);
    git_in(&root, &["push", "-q", "origin", "main"]);
    git_in(&root, &["reset", "-q", "--hard", "HEAD~1"]);
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git_sync_state(), SyncState::InSync);
    let rect = (app.commit_hits.iter())
        .find(|(_, t)| *t == CommitTarget::Sync)
        .unwrap()
        .0;
    app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(
        tip.as_deref(),
        Some("获取远程更新（已与 origin/main 同步）")
    );

    app.press_commit(CommitTarget::Sync);
    assert_eq!(app.git_sync_state(), SyncState::Pull(1));

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn a_clean_branch_ahead_of_its_upstream_turns_the_commit_button_into_sync() {
    let Some((mut app, root)) = git_app("git-commit-sync", false) else {
        return;
    };
    // 提交全部改动，上游停在上一个提交：领先 1 个提交，没有改动。
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git_sync_state(), SyncState::Push(1));
    let button = (app.commit_hits.iter())
        .find(|(_, t)| *t == CommitTarget::Sync)
        .unwrap()
        .0;
    app.on_mouse_move(button.x + 4.0, button.y + 4.0);
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(tip.as_deref(), Some("推送 1 个提交到 origin/main"));

    // 在修改上一次提交时不显示；有改动时还在，提交按钮不再兼做同步。
    app.commit.amend = true;
    assert_eq!(app.git_sync_state(), SyncState::Hidden);
    app.commit.amend = false;
    std::fs::write(root.join("new.txt"), "x\n").unwrap();
    app.git_refresh();
    assert_eq!(app.git_sync_state(), SyncState::Push(1));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：侧栏较窄时按目录排列的改动列表（`commit-narrow-dirs`），检查并起来的目录名截短后
/// 仍以淡色分隔符显示。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_narrow_dirs() {
    let Some((mut app, root)) = git_app("git-commit-narrow", false) else {
        return;
    };
    let deep = root.join("crates/winc-app/src/app/snapshot/git");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("commit.rs"), "a\n").unwrap();
    let view = root.join("crates/winc-app/src/view/git");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(view.join("commit.rs"), "b\n").unwrap();
    app.git_refresh();
    app.commit.set_flat(false);
    app.panel_width = 200.0;
    app.relayout();
    app.run_action(Action::ShowCommit);
    shoot(&mut app, "commit-narrow-dirs");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn untracked_files_stay_out_of_the_commit_until_checked() {
    let Some((mut app, root)) = git_app("git-commit-untracked", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let status = app.git.status.clone().unwrap();
    let entry = status.entries.iter().find(|e| e.path == new).unwrap();

    // 未跟踪的文件单独成组，默认显示但不勾，不计入提交范围。
    assert!(!app.commit.group_collapsed(CommitGroup::Untracked));
    assert!(!app.commit.checked(entry));
    assert_eq!(
        app.commit.group_state(&status, CommitGroup::Untracked),
        None
    );
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);
    assert_eq!(app.commit_scope(), Some(1));

    // 收起和展开不改变勾选；收起时切换文件跳过这一组。
    app.press_commit(CommitTarget::Group(CommitGroup::Untracked));
    assert_eq!(app.commit_files(), std::slice::from_ref(&main));
    assert!(!app.commit.checked(entry));
    app.press_commit(CommitTarget::Group(CommitGroup::Untracked));
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);

    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    assert!(app.commit.checked(entry));
    assert_eq!(app.commit_scope(), Some(2));
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    assert!(!app.commit.checked(entry));

    // 没有勾上时提交只包含已跟踪的改动，未跟踪的文件仍未跟踪。
    app.commit.message.set_text("只提交改动");
    app.git_commit();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].path, new);
    assert!(status.entries[0].untracked());

    // 勾上以后随提交加入仓库；提交后勾选回到默认。
    assert_eq!(app.commit_scope(), None);
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    app.commit.message.set_text("加入新文件");
    app.git_commit();
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());
    std::fs::write(&new, "changed\n").unwrap();
    std::fs::write(root.join("another.rs"), "\n").unwrap();
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    let another = status.entries.iter().find(|e| e.untracked()).unwrap();
    assert!(!app.commit.checked(another));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_button_names_its_scope_only_for_a_plain_commit() {
    let Some((mut app, root)) = git_app("git-commit-scope", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.commit_scope(), Some(1));

    // 没有勾选、修改上一次提交时都不写范围。
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    assert_eq!(app.commit_scope(), None);
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    app.commit.amend = true;
    assert_eq!(app.commit_scope(), None);
    app.commit.amend = false;

    // 游离 `HEAD` 时也写文件数，分支写在上方的分支行里。
    git_in(&root, &["checkout", "-q", "--detach"]);
    app.git_refresh();
    assert_eq!(app.commit_scope(), Some(1));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_rewritten_branch_is_not_force_pushed_without_confirmation() {
    let Some((mut app, root)) = git_app("git-sync-force", false) else {
        return;
    };
    let remote = root.with_extension("sync-remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "one"]);
    git_in(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_in(&root, &["push", "-q", "-u", "origin", "main"]);
    let pushed = |root: &Path| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "origin/main"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    git_in(&root, &["commit", "-q", "--amend", "-m", "one, reworded"]);
    let before = pushed(&root);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(matches!(app.git_sync_state(), SyncState::Both { .. }));

    // 同步发现需要强制推送：只用提示框询问，确认按钮标记为危险操作，上游不变。
    app.press_commit(CommitTarget::Sync);
    assert_eq!(pushed(&root), before);
    assert!(app.alert.as_ref().unwrap().buttons[0].destructive);
    // 取消后还是不推送，可以再问一次。
    press_alert(&mut app, 1);
    assert_eq!(pushed(&root), before);
    app.press_commit(CommitTarget::Sync);

    // 确认后才替换上游。
    press_alert(&mut app, 0);
    assert_ne!(pushed(&root), before);
    assert_eq!(app.git_sync_state(), SyncState::InSync);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&remote);
}

/// 所有视图共用一个侧栏宽度：切换不变，用真实的鼠标事件拖动分隔条，新窗口、下次启动也沿用。
#[test]
fn the_sidebar_keeps_one_width_across_views() {
    use crate::theme::DEFAULT_PANEL_WIDTH;

    let Some((mut app, root)) = git_app("git-commit-width", false) else {
        return;
    };
    paint(&mut app);
    assert_eq!(app.panel_width, DEFAULT_PANEL_WIDTH);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.panel_width, DEFAULT_PANEL_WIDTH);

    // 提交视图里按住分隔条拖动：列表、提交信息框旁边都能拖。
    let drag = |app: &mut App, y: f32, by: f32| {
        let x = app.layout.splitter.x + 1.0;
        app.on_mouse_move(x, y);
        assert_eq!(app.hover, Hover::Splitter);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(x + by, y);
        app.on_mouse_up(MouseButton::Left);
        paint(app);
    };
    let message = app.commit_parts().message;
    drag(&mut app, message.y + 4.0, 40.0);
    let dragged = DEFAULT_PANEL_WIDTH + 40.0;
    assert_eq!(app.panel_width, dragged);
    assert_eq!(app.state.panel_width, Some(dragged));

    // 切到文件树、搜索还是这个宽度。
    app.show_files();
    assert_eq!(app.panel_width, dragged);
    app.set_sidebar_view(SidebarView::Search);
    assert_eq!(app.panel_width, dragged);

    // 文件树里拖动，切回提交视图也是拖过的宽度。
    app.show_files();
    paint(&mut app);
    drag(&mut app, 400.0, -20.0);
    let dragged = dragged - 20.0;
    assert_eq!(app.state.panel_width, Some(dragged));
    app.run_action(Action::ShowCommit);
    assert_eq!(app.panel_width, dragged);

    // 记下的宽度比这个窗口放得下的大：从看到的宽度开始拖，一动就有反应。
    app.panel_width = 5000.0;
    paint(&mut app);
    let shown = app.layout.window.w - 320.0;
    drag(&mut app, 400.0, -20.0);
    assert_eq!(app.panel_width, shown - 20.0);

    // 新窗口、下次启动从状态恢复，太窄的值抬到最小宽度。
    app.state.panel_width = Some(10.0);
    app.restore_panel_width();
    assert_eq!(app.panel_width, crate::theme::MIN_PANEL_WIDTH);

    let _ = std::fs::remove_dir_all(&root);
}
