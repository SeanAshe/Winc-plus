//! 提交视图的测试：改动列表、勾选、提交、菜单、提交信息、滚动与固定行、折叠、长文件名。

use super::*;
use std::time::{Duration, Instant};

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
        app.hits
            .diff
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
        .hits
        .commit
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

    // 「回滚」只作用于该文件（没有窗口时不弹确认框）。
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
    assert_eq!(app.sidebar.view, SidebarView::Commit);
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
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
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
    app.press_commit(CommitTarget::GroupRevert(CommitGroup::Changes));
    assert_eq!(std::fs::read_to_string(&main).unwrap(), before);
    assert!(root.join("scratch.txt").exists());
    // 勾上以后，未跟踪的文件移到废纸篓（测试中直接删除）。
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    app.press_commit(CommitTarget::GroupRevert(CommitGroup::Untracked));
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
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).map(|h| h.0);
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
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let row = find(&app, CommitTarget::File(index));
    app.on_mouse_move(row.x + 120.0, row.y + row.h / 2.0);
    paint(&mut app);
    let revert = find(&app, CommitTarget::RevertFile(index));
    app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
    app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
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
        app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, if dark { "commit-dark" } else { "commit-light" });
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 截图：侧栏最窄（230）和默认宽度（300）下的提交视图，分支名很长（`commit-width-*`），检查第二、
/// 三行和提交卡片底行没有互相压住：最窄时「修改上次提交」收进按钮的菜单。另有领先上游、没有更改时
/// 的空状态和下一步按钮（`commit-width-clean-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_widths() {
    for width in [
        crate::theme::MIN_PANEL_WIDTH,
        crate::theme::DEFAULT_PANEL_WIDTH,
    ] {
        let name = format!("commit-width-{width}");
        let Some((mut app, root)) = git_app(&name, false) else {
            return;
        };
        git_in(
            &root,
            &[
                "switch",
                "-q",
                "-c",
                "feature/login-lockout-after-three-tries",
            ],
        );
        app.sidebar.width = width;
        app.git_refresh();
        app.run_action(Action::ShowCommit);
        app.edit_field(|f| f.insert("修复日志刷新没有反馈"));
        shoot(&mut app, &name);

        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "修复日志刷新没有反馈"]);
        git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
        git_in(
            &root,
            &["remote", "add", "origin", "https://example.invalid/r.git"],
        );
        let upstream = "refs/remotes/origin/feature/login-lockout-after-three-tries";
        git_in(&root, &["update-ref", upstream, "HEAD~1"]);
        let set = "--set-upstream-to=origin/feature/login-lockout-after-three-tries";
        git_in(&root, &["branch", "-q", set]);
        app.git_refresh();
        shoot(&mut app, &format!("commit-width-clean-{width}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 截图：改动列表下面展开的储藏一组（`commit-stashes`），鼠标停在第一条上（「应用」和「⋯」），
/// 储藏超过五条时最后一行是「还有 N 条」。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_stashes() {
    let Some((mut app, root)) = git_app("commit-stashes", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    for (i, note) in [
        "改登录",
        "试一下新写法",
        "调日志刷新",
        "备份配置",
        "临时打点",
        "旧方案",
    ]
    .iter()
    .enumerate()
    {
        std::fs::write(root.join("src/main.rs"), format!("fn main() {{ {i}; }}\n")).unwrap();
        git_in(&root, &["stash", "push", "-q", "-m", note]);
    }
    std::fs::write(root.join("src/main.rs"), "fn main() { now(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.press_commit(CommitTarget::Stashes);
    paint(&mut app);
    let (row, _) = *(app.hits.commit.iter())
        .find(|(_, t)| *t == CommitTarget::Stash(0))
        .unwrap();
    app.on_mouse_move(row.x + 20.0, row.y + row.h / 2.0);
    shoot(&mut app, "commit-stashes");
    // 打开第一条储藏：它的文件列在下面，正在查看的文件处于选中状态。
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "-u", "-m", "带一个新文件"]);
    app.git_refresh();
    paint(&mut app);
    app.press_commit(CommitTarget::Stash(0));
    shoot(&mut app, "commit-stash-files");
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：动画途中的三帧：储藏一组拉开到三分之一，提交完以后提交卡片滑出到一半，推送以后
/// 「上一次提交」收起到三分之一。
#[test]
#[ignore]
fn shoot_commit_motion() {
    let Some((mut app, root)) = git_app("commit-motion", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    for note in ["改登录", "试一下新写法", "调日志刷新"] {
        std::fs::write(
            root.join("src/main.rs"),
            format!("fn main() {{ {note:?}; }}\n"),
        )
        .unwrap();
        git_in(&root, &["stash", "push", "-q", "-m", note]);
    }
    std::fs::write(root.join("src/main.rs"), "fn main() { now(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let closed = app.commit_parts().stashes.unwrap().h;
    app.press_commit(CommitTarget::Stashes);
    app.commit.stash_motion = Some((closed, Clock::frozen(TREE_ANIMATION / 3, TREE_ANIMATION)));
    shoot(&mut app, "commit-stash-opening");

    app.commit.stash_motion = None;
    git_in(&root, &["commit", "-q", "-am", "now"]);
    app.git_refresh();
    app.commit.card.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    shoot(&mut app, "commit-card-leaving");

    // 推送了：「上一次提交」收起到一半。
    app.commit.card.motion = None;
    git_in(
        &root,
        &["remote", "add", "origin", "https://example.invalid/r.git"],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.track_commit_bars();
    app.git_refresh();
    app.commit.undo.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 3, TREE_ANIMATION)));
    shoot(&mut app, "commit-undo-folding");
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：没有改动的提交视图（居中的「没有更改」，没有提交卡片），深浅两套配色。
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
    assert_eq!(app.sidebar.view, SidebarView::Files);
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
    assert!(enabled(&app, MenuCommand::CommitTree));
    assert!(!enabled(&app, MenuCommand::CommitExpandAll));
    assert!(enabled(&app, MenuCommand::CommitCollapseAll));
    // 没勾文件时悬停组标题，右端仍是文件数，没有「回滚」。
    let reverts = |app: &mut App| {
        let (band, _) = *(app.hits.commit.iter())
            .find(|(_, t)| *t == CommitTarget::Group(CommitGroup::Changes))
            .unwrap();
        app.on_mouse_move(band.x + band.w / 2.0, band.y + band.h / 2.0);
        paint(app);
        (app.hits.commit.iter()).any(|(_, t)| matches!(t, CommitTarget::GroupRevert(_)))
    };
    assert!(!reverts(&mut app));

    app.run_commit_more_command(MenuCommand::CommitCollapseAll);
    let groups = [CommitGroup::Changes];
    assert!(groups.iter().all(|g| app.commit.group_collapsed(*g)));
    assert!(enabled(&app, MenuCommand::CommitExpandAll));
    assert!(!enabled(&app, MenuCommand::CommitCollapseAll));
    app.run_commit_more_command(MenuCommand::CommitExpandAll);
    assert!(groups.iter().all(|g| !app.commit.group_collapsed(*g)));

    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    paint(&mut app);
    assert!(reverts(&mut app));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_message_grows_with_its_text_and_the_list_follows() {
    let Some((mut app, root)) = git_app("git-message-grow", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 自上而下依次是改动列表和底部的提交卡片；卡片里提交信息在上，底行左边是「修改上次提交」，
    // 右边是「提交 ⌄」。空信息时高度为两行。
    let one = app.commit_parts();
    let card = one.card.unwrap();
    assert_eq!(one.message.h, 2.0 * 20.0 + 4.0);
    assert_eq!(card.bottom(), app.layout.sidebar.bottom() - 8.0);
    assert!(one.list.bottom() <= card.y);
    assert!(one.commit.y >= one.message.bottom());
    assert_eq!(one.commit_menu.x, one.commit.right());
    assert!(one.commit_menu.right() <= card.right());
    let amend = one.amend.unwrap();
    assert!(amend.right() < one.commit.x);

    // 写入三行时框高增加一行，卡片往上长，列表随之变短。
    app.edit_field(|f| f.insert("修复配置读取\n\n启动时不再报错"));
    paint(&mut app);
    let three = app.commit_parts();
    assert_eq!(three.message.h, 3.0 * 20.0 + 4.0);
    assert_eq!(three.card.unwrap().bottom(), card.bottom());
    assert_eq!(one.list.bottom() - three.list.bottom(), 20.0);

    // 行数超过八行后不再长高，改为在框内滚动。
    app.edit_field(|f| f.insert(&"\n一行".repeat(20)));
    paint(&mut app);
    assert_eq!(app.commit_parts().message.h, 8.0 * 20.0 + 4.0);

    // 清空后框高从八行过渡回两行：途中介于两者之间，文字仍按两行的高度排版。
    use crate::app::animation::{Clock, TREE_ANIMATION};
    app.commit.message.set_text("");
    paint(&mut app);
    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.commit.message_motion = Some((8.0 * 20.0 + 4.0, half));
    let parts = app.commit_parts();
    assert!(parts.message.h > 44.0 && parts.message.h < 8.0 * 20.0 + 4.0);
    assert_eq!(parts.message_settled.h, 44.0);
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
    // 卡片放得下勾选框时「修改上次提交」在卡片里，菜单里只有提交并推送；侧栏最窄时勾选框收进
    // 菜单。
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitPush, true)));
    assert!(app.commit_parts().amend.is_some());
    assert!(!menu.contains(&MenuEntry::Item(MenuCommand::CommitAmend, true)));
    let width = app.sidebar.width;
    app.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    assert!(app.commit_parts().amend.is_none());
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitAmend, true)));
    app.sidebar.width = width;
    paint(&mut app);

    // 回归：主按钮显示没有要提交的更改时，菜单不能再提供「提交并推送」；修改上一次提交仍然可用。
    let entries = std::mem::take(&mut app.git.status.as_mut().unwrap().entries);
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitPush, false)));
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitAmend, true)));
    app.commit.amend = true;
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitPush, true)));
    app.commit.amend = false;
    app.git.status.as_mut().unwrap().entries = entries;

    // 「修改上次提交」启用时打勾，按钮写「修改提交」。
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
    // `b` 目录之后再放一批文件：滚动范围足够大，下面滚到 `b` 目录中间的位置才能到达。
    let more = dir.join("assets/pets/c");
    std::fs::create_dir_all(&more).unwrap();
    for i in 0..40 {
        std::fs::write(more.join(format!("{i}.png")), "x").unwrap();
    }
    git_in(&dir, &["add", "-A"]);
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
    assert!(app.commit.scroll <= app.commit_max_scroll());
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
            for (rect, target) in &app.hits.commit {
                if *target == CommitTarget::Sticky(s.row, s.level) {
                    assert!(rect.y >= slot - 0.01, "{step}: {rect:?} above {slot}");
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn showing_as_tree_switches_the_changes_between_a_tree_and_a_flat_list() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

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
    // 默认平铺：没有目录行，每个文件一行，按路径排；「刷新」在「Git 日志」后面。
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
    assert_eq!(tools[1].1, CommitTarget::Refresh);
    assert!(tools[1].0.x > tools[0].0.x && tools[1].0.x < tools[2].0.x);
    // 平铺时文件不缩进：勾选框和组标题的勾选框在同一列。
    let check_x = |app: &App, want: fn(&CommitTarget) -> bool| {
        let (rect, _) = app.hits.commit.iter().find(|(_, t)| want(t)).unwrap();
        rect.x + rect.w / 2.0
    };
    let group = check_x(&app, |t| matches!(t, CommitTarget::GroupCheck(_)));
    assert_eq!(
        check_x(&app, |t| matches!(t, CommitTarget::Check(_))),
        group
    );

    // 「⋯」里的「以树形显示」：按目录排成树，排成树时打勾；记进状态，以后打开的窗口、下次启动
    // 沿用。
    let toggle = |app: &mut App| {
        app.open_context_menu(MenuTarget::CommitMore, (0.0, 0.0));
        app.run_menu_command(MenuCommand::CommitTree);
    };
    assert!(!app.menu_checked(MenuCommand::CommitTree));
    toggle(&mut app);
    assert!(!app.commit.flat());
    assert!(app.menu_checked(MenuCommand::CommitTree));
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
        app.hits
            .commit
            .iter()
            .any(|(_, t)| *t == CommitTarget::Dir(a))
    );
    assert!(
        !app.hits
            .commit
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
        .hits
        .commit
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

/// 改动行的长名字和文件树一样随鼠标滑出、收回：命中范围立即按全名，画出的宽度从截短的宽度
/// 缓动过去，截短的名字淡出、全名淡入；移开以后宽度缓动回去，这一行上不再唤出滚动条。
#[test]
fn a_long_change_name_slides_out_and_back_like_the_file_tree() {
    let Some((dir, long)) = long_name_repo("git-commit-name-slide") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.commit.names.animate_without_window = true;
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let entry = (status.entries.iter())
        .find(|e| e.path.ends_with(&long))
        .unwrap();
    let key = CommitName::File(entry.path.clone());
    let rows = app.commit_rows();
    let i = (rows.iter())
        .position(|r| matches!(r, CommitRow::File { index, .. } if status.entries[*index].path == entry.path))
        .unwrap();
    let rest = app.commit.names.peek(&key).unwrap();
    assert!(!app.commit.names.moving());

    let list = app.commit_parts().list;
    let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    let overflow = app.commit.overflow.expect("long name spills");
    let (_, revert) = overflow.actions.unwrap();
    let view = app.commit.names.peek(&key).unwrap();
    assert!(app.commit.names.moving());
    assert!(view.width < revert.x - list.x, "{}", view.width);
    assert!(
        (view.width - rest.width).abs() < 40.0,
        "{} {}",
        view.width,
        rest.width
    );
    assert!(!view.old.is_empty());
    assert!(app.over_commit_overflow_row(y));

    app.commit.names.finish();
    paint(&mut app);
    let out = app.commit.names.peek(&key).unwrap();
    assert!(out.width > rest.width + 40.0);

    // 移开：命中范围立即消失，宽度从全名开始缓动回去。
    app.on_mouse_move(list.x + 120.0, list.bottom() - 4.0);
    paint(&mut app);
    assert!(app.commit.overflow.is_none_or(|o| o.row != i));
    let back = app.commit.names.peek(&key).unwrap();
    assert!(app.commit.names.moving());
    assert!(
        back.width > rest.width + 40.0,
        "{} {}",
        back.width,
        rest.width
    );
    app.commit.names.finish();
    paint(&mut app);
    assert_eq!(app.commit.names.peek(&key).unwrap().width, rest.width);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 名字在侧栏里放得下、只需为「打开文件」「回滚」两个按钮让出位置时：名字不变，按钮从原位滑到
/// 名字后面，伸出侧栏的部分随按钮变宽；名字放不下、在截断处淡出的行，悬停时同样从截断处滑出
/// 全名。
#[test]
fn names_that_only_make_room_for_the_buttons_slide_the_buttons_out() {
    let Some(dir) = deep_changes_repo("git-commit-button-slide") else {
        return;
    };
    // 名字长短不一的文件放在根目录：没有可以折叠的目录，截短时只能在末尾淡出。
    let names: Vec<String> = (20..60).map(|n| format!("{}.rs", "x".repeat(n))).collect();
    for name in &names {
        std::fs::write(dir.join(name), "x").unwrap();
    }
    git_in(&dir, &["add", "-A"]);
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.commit.names.animate_without_window = true;
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let list = app.commit_parts().list;
    let away = (app.layout.editor.x + 200.0, app.layout.editor.y + 200.0);

    // 依次悬停各行，记下按钮开始时的位置、终点，以及名字露出的宽度是否在变。
    let mut slight = None;
    let mut clipped = None;
    for name in &names {
        let rows = app.commit_rows();
        let Some(i) = rows.iter().position(|r| {
            matches!(r, CommitRow::File { index, .. } if status.entries[*index].path.ends_with(name))
        }) else {
            continue;
        };
        app.on_mouse_move(away.0, away.1);
        app.commit.scroll = ((i as f32 - 3.0) * COMMIT_ROW).clamp(0.0, app.commit_max_scroll());
        paint(&mut app);
        app.commit.names.finish();
        paint(&mut app);
        let path = (status.entries.iter())
            .find(|e| e.path.ends_with(name))
            .unwrap();
        let key = CommitName::File(path.path.clone());
        let rest = app.commit.names.peek(&key).unwrap().width;
        let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;
        app.on_mouse_move(list.x + 120.0, y);
        paint(&mut app);
        let Some(overflow) = app.commit.overflow else {
            continue;
        };
        let (end, _) = overflow.actions.unwrap();
        let start = (app.hits.commit.iter())
            .find(|(_, t)| matches!(t, CommitTarget::OpenFile(_)))
            .unwrap()
            .0;
        let moving = app.commit.names.moving();
        app.commit.names.finish();
        let full = app.commit.names.peek(&key).unwrap().width;
        if rest == full && slight.is_none() {
            slight = Some((start.x, end.x, moving));
        }
        if rest < full && clipped.is_none() {
            clipped = Some(moving);
        }
    }

    // 只需让出位置：名字不动，按钮从原位出发，距终点仍有一段距离。
    let (start, end, moving) = slight.expect("a name that fits before the status letter");
    assert!(!moving);
    assert!(start < end - 1.0, "{start} {end}");
    // 截断处淡出的名字：悬停时宽度从截断处开始变化。
    assert_eq!(clipped, Some(true));
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
    assert_eq!(app.sidebar.view, SidebarView::Commit);

    // 换到另一个仓库：还在提交视图，焦点在提交信息里。
    app.switch_folder(&second);
    assert_eq!(app.sidebar.view, SidebarView::Commit);
    assert_eq!(app.focus, Focus::CommitMessage);

    // 换到不是仓库的目录：没有提交视图了，回到文件树。
    app.switch_folder(&plain);
    assert_eq!(app.sidebar.view, SidebarView::Files);
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
    // 「上一次提交」在提交卡片上面（还有没勾的未跟踪文件，卡片还在），改动列表在它上面结束；
    // 没有卡片时它在视图底部。
    let parts = app.commit_parts();
    let (bar, undo, button) = parts.undo.unwrap();
    match parts.card {
        Some(card) => assert!(bar.bottom() <= card.y),
        None => assert_eq!(bar.bottom(), app.layout.sidebar.bottom()),
    }
    assert_eq!(parts.list.bottom(), bar.y);
    assert!(undo.right() <= button.x);
    for (rect, target) in [
        (undo, CommitTarget::Undo),
        (button, CommitTarget::LastCommit),
    ] {
        assert!(
            (app.hits.commit.iter()).any(|(r, t)| *t == target && *r == rect),
            "{target:?}"
        );
    }

    // 撤销：分支退回上一个提交，改动回到列表里，提交信息填回输入框。
    app.press_commit(CommitTarget::LastCommit);
    app.run_menu_command(MenuCommand::CommitCopyOid);
    let oid = app.commit_undoable().unwrap().oid.clone();
    assert_eq!(app.copied.as_deref(), Some(oid.as_str()));
    app.press_commit(CommitTarget::LastCommit);
    let commands: Vec<_> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [
            MenuCommand::CommitUndo,
            MenuCommand::CommitAmend,
            MenuCommand::CommitCopyOid
        ]
    );
    // 「撤销」写在栏上，和菜单里的「撤销提交」是同一件事。
    app.press_commit(CommitTarget::Undo);
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

/// 在终端里只暂存了一部分的文件与其他文件的处理相同：勾选框为全选，提交的是工作区里的内容，
/// 与暂存区无关。
#[test]
fn files_staged_in_the_terminal_commit_what_is_in_the_working_tree() {
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
    let entry = status.entries.iter().find(|e| e.path == main).unwrap();
    assert!(entry.staged.is_some() && entry.unstaged.is_some());
    assert_eq!(app.commit.file_state(entry), Some(true));

    app.commit.message.set_text("提交工作区里的内容");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&head.stdout),
        "staged\nnot staged\n"
    );
    let status = app.git.status.clone().unwrap();
    assert!(status.entries.iter().all(|e| e.path != main));
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
        (app.hits.commit.iter())
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
    let rect = (app.hits.commit.iter())
        .find(|(_, t)| *t == CommitTarget::Sync)
        .unwrap()
        .0;
    app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
    assert!(app.tooltip.since.is_some());
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
        app.hits
            .commit
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
    press_alert(&mut app, 0);
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
    let rect = (app.hits.commit.iter())
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
    let button = (app.hits.commit.iter())
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
    app.sidebar.width = 200.0;
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

    // 同步发现需要强制推送：只用提示框询问，确认按钮标记为危险操作，上游不变。同步本身先确认
    // （见 `App::ask_push`）。
    app.press_commit(CommitTarget::Sync);
    press_alert(&mut app, 0);
    assert_eq!(pushed(&root), before);
    assert!(app.alert.as_ref().unwrap().buttons[0].destructive);
    // 取消后还是不推送，可以再问一次。
    press_alert(&mut app, 1);
    assert_eq!(pushed(&root), before);
    app.press_commit(CommitTarget::Sync);
    press_alert(&mut app, 0);

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
    assert_eq!(app.sidebar.width, DEFAULT_PANEL_WIDTH);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.sidebar.width, DEFAULT_PANEL_WIDTH);

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
    assert_eq!(app.sidebar.width, dragged);
    assert_eq!(app.state.panel_width, Some(dragged));

    // 切到文件树、搜索还是这个宽度。
    app.show_files();
    assert_eq!(app.sidebar.width, dragged);
    app.set_sidebar_view(SidebarView::Search);
    assert_eq!(app.sidebar.width, dragged);

    // 文件树里拖动，切回提交视图也是拖过的宽度。
    app.show_files();
    paint(&mut app);
    drag(&mut app, 400.0, -20.0);
    let dragged = dragged - 20.0;
    assert_eq!(app.state.panel_width, Some(dragged));
    app.run_action(Action::ShowCommit);
    assert_eq!(app.sidebar.width, dragged);

    // 记下的宽度比这个窗口放得下的大：从看到的宽度开始拖，一动就有反应。
    app.sidebar.width = 5000.0;
    paint(&mut app);
    let shown = app.layout.window.w - 320.0;
    drag(&mut app, 400.0, -20.0);
    assert_eq!(app.sidebar.width, shown - 20.0);

    // 新窗口、下次启动从状态恢复，太窄的值抬到最小宽度。
    app.state.panel_width = Some(10.0);
    app.restore_panel_width();
    assert_eq!(app.sidebar.width, crate::theme::MIN_PANEL_WIDTH);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn commit_only_the_checked_lines() {
    let dir = search_folder("git-partial-lines");
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
    let disk = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        disk.replace(
            "let value_5 = 500;\n",
            "let value_5 = 500;\nlet added_a = 1;\nlet added_b = 2;\n",
        ),
    )
    .unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);

    // 找到 `added_b` 所在的行并把它勾掉，同时只提交第一处改动。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    let hunk = view
        .hunks
        .iter()
        .position(|h| h.new.contains(&line))
        .unwrap();
    let hunks = view.hunks.len();
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    paint(&mut app);
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::IncludeLine(RIGHT, line))
    );
    let status = app.git.status.clone().unwrap();
    let entry = status.entries.iter().find(|e| e.path == file).unwrap();
    assert_eq!(app.commit.file_state(entry), Some(false));
    for other in (0..hunks).filter(|&h| h != hunk) {
        app.press_diff(DiffTarget::Include(other));
    }

    app.commit.message.set_text("only added_a");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap();
    assert!(head.contains("let added_a = 1;"), "{head}");
    assert!(!head.contains("added_b"), "{head}");
    // 没提交的行还在工作区里。
    assert!(std::fs::read_to_string(&file).unwrap().contains("added_b"));
}

#[test]
fn line_checks_keep_their_state_until_the_change_is_edited() {
    use winc_core::diff::Pick;
    use winc_editor::diff_view::LEFT;
    let dir = search_folder("git-line-state");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let file = app.git.root.clone().unwrap().join("src/main.rs");
    let status = app.git.status.clone().unwrap();
    let entry = status
        .entries
        .iter()
        .find(|e| e.path == file)
        .unwrap()
        .clone();
    let (key, old) = (7, 0..2);
    let all = [(key, old.clone())];
    let inclusion =
        |app: &App, key| app.commit.hunk_inclusions(&entry, &[(key, old.clone())])[0].clone();
    assert_eq!(inclusion(&app, key), Pick::Apply);

    // 勾掉一行：按行提交。再勾回来：整处提交。
    app.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    assert!(matches!(inclusion(&app, key), Pick::Lines(_)));
    app.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    assert_eq!(inclusion(&app, key), Pick::Apply);

    // 内容变了：行号不再对应，整处不提交。
    app.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    assert_eq!(inclusion(&app, key + 1), Pick::Skip);

    // 整段的勾选框把只提交部分行的改动恢复为整处提交。
    app.commit.toggle_hunk(&entry, 0, &all);
    assert_eq!(inclusion(&app, key), Pick::Apply);

    // 所有行都勾掉时文件也勾掉。
    for (side, line) in [(LEFT, 0), (LEFT, 1), (RIGHT, 0)] {
        app.commit.toggle_line(&entry, (0, 1), (side, line), &all);
    }
    assert_eq!(app.commit.file_state(&entry), None);

    // 文件没勾时勾上一行：文件勾上，只提交这一行。
    app.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    let Pick::Lines(set) = inclusion(&app, key) else {
        panic!("应当按行提交");
    };
    assert_eq!(set.old.len(), 2);
    assert!(set.new.is_empty());
}

/// 只提交部分行的改动：截图用于检查行勾选框、整段勾选框的「部分」状态和不提交行的底色。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_partial_lines() {
    let dir = search_folder("git-lines-shot");
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
    let disk = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        disk.replace(
            "let value_5 = 500;\n",
            "let value_5 = 500;\nlet added_a = 1;\nlet added_b = 2;\nlet added_c = 3;\n",
        ),
    )
    .unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    shoot(&mut app, "diff-partial-lines");
    app.press_diff(DiffTarget::Layout);
    shoot(&mut app, "diff-partial-lines-unified");
}

/// 回归：滚动到长列表下方后提交，剩下的改动比滚动位置少，列表不能一片空白。
#[test]
fn a_shorter_list_is_visible_after_scrolling_far_down() {
    let Some(dir) = deep_changes_repo("git-commit-shrink") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.show_commit();
    paint(&mut app);
    let max = app.commit_max_scroll();
    assert!(max > 0.0, "样本需要超出列表高度");
    app.commit.scroll = max;
    paint(&mut app);

    // 只留下一个改动：其余的全部提交到仓库。
    let keep = dir.join("kept-change.txt");
    git_in(&dir, &["add", "--all"]);
    git_in(
        &dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "all",
        ],
    );
    std::fs::write(&keep, "x").unwrap();
    app.git_refresh();
    paint(&mut app);

    assert!(app.commit.scroll <= app.commit_max_scroll());
    assert_eq!(app.commit.scroll, 0.0);
    assert!(
        (app.hits.commit.iter()).any(|(_, target)| matches!(target, CommitTarget::File(_))),
        "列表应当画出剩下的改动"
    );
}

/// 「⋯」的分组：储藏，远程仓库，视图，输出；同步、新建分支和工作树只在分支选择器里。
#[test]
fn the_commit_more_menu_keeps_each_action_in_one_group() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-commit-more-groups", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let menu = app.commit_more_menu();
    let groups: Vec<Vec<Option<MenuCommand>>> = (menu.split(|e| *e == MenuEntry::Separator))
        .map(|group| {
            (group.iter())
                .map(|entry| match entry {
                    MenuEntry::Item(command, _) => Some(*command),
                    _ => None,
                })
                .collect()
        })
        .collect();
    let at = |command| (groups.iter()).position(|group| group.contains(&Some(command)));
    assert_eq!(at(MenuCommand::GitStash), Some(0));
    assert_eq!(at(MenuCommand::CommitTree), Some(groups.len() - 2));
    assert_eq!(at(MenuCommand::CommitExpandAll), Some(groups.len() - 2));
    assert_eq!(at(MenuCommand::GitShowOutput), Some(groups.len() - 1));
    // 刷新是标题行上的按钮，拉取、推送、获取和新建工作树在分支选择器里，这里都不重复。
    for command in [
        MenuCommand::GitFetch,
        MenuCommand::GitPush,
        MenuCommand::NewWorktree,
    ] {
        assert_eq!(at(command), None, "{command:?}");
    }
    // 没有上游时两种拉取都写明原因。
    for command in [MenuCommand::GitPull, MenuCommand::GitPullRebase] {
        assert!(app.menu_label(command).subtitle.is_some(), "{command:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 没有更改时不显示提交卡片：空状态下面是下一步（这里是推送领先的一个提交），焦点从看不见的
/// 提交信息移到改动列表上，`Tab` 也不再把焦点交给它。
#[test]
fn a_clean_tree_hides_the_commit_card_and_offers_the_next_step() {
    let Some((mut app, root)) = git_app("git-commit-clean-card", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.commit_parts().card.is_some());
    assert_eq!(app.focus, Focus::CommitMessage);

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
    paint(&mut app);
    let parts = app.commit_parts();
    assert!(parts.card.is_none());
    assert_eq!(app.focus, Focus::CommitList);
    let next = (app.hits.commit.iter()).find(|(_, t)| *t == CommitTarget::NextStep);
    let (rect, _) = *next.unwrap();
    assert!(parts.list.contains(rect.x, rect.y));
    app.commit_list_key(&winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab));
    assert_eq!(app.focus, Focus::CommitList);

    // 打开「修改上一次提交」时卡片回来，填上上一次的信息；关掉后又收起。
    app.press_commit(CommitTarget::LastCommit);
    app.run_menu_command(crate::app::input::context_menu::MenuCommand::CommitAmend);
    paint(&mut app);
    assert!(app.commit.amend);
    assert!(app.commit_parts().card.is_some());
    assert_eq!(app.focus, Focus::CommitMessage);
    app.press_commit(CommitTarget::Amend);
    paint(&mut app);
    assert!(!app.commit.amend);
    assert!(app.commit_parts().card.is_none());
    assert_eq!(app.focus, Focus::CommitList);
    let _ = std::fs::remove_dir_all(&root);
}

/// 「刷新」点下去以后至少转满一圈：读取几十毫秒就结束，只按读取的时长转看不出点过。
#[test]
fn the_refresh_button_spins_for_at_least_one_turn() {
    let Some((mut app, root)) = git_app("git-commit-refresh-spin", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(!app.commit_refreshing());
    app.press_commit(CommitTarget::Refresh);
    // 测试里读取就地完成，这时转的只是至少一圈的时长。
    assert!(app.commit_refreshing());
    assert!(app.animating());
    let turn = Duration::from_secs_f32(1.0 / crate::theme::SPIN_SPEED);
    app.commit.refreshed = Instant::now().checked_sub(turn);
    assert!(!app.commit_refreshing());
    let _ = std::fs::remove_dir_all(&root);
}

/// 改动列表下面的储藏一组：默认收起；展开后每条一行。点一下在储藏的预览标签里打开它的第一个文件，
/// 差异里的下一个文件在这条储藏的文件之间切换，一起收进去的未跟踪文件排在后面；悬停时的「应用」
/// 把改动放回工作区。
#[test]
fn stashes_are_listed_below_the_changes_and_open_their_files() {
    let Some((mut app, root)) = git_app("git-commit-stashes", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { stashed(); }\n").unwrap();
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "-u", "-m", "试一下新写法"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let stash = app.git.stashes[0].clone();
    assert_eq!(stash.branch(), Some("main"));
    assert_eq!(stash.parents.len(), 3);

    // 收着时只有标题一行，在列表下面。
    let parts = app.commit_parts();
    let section = parts.stashes.unwrap();
    assert_eq!(
        section.h,
        COMMIT_ROW + 2.0 * crate::app::git::commit::STASH_GAP
    );
    assert!(parts.list.bottom() <= section.y);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Stash(0)));
    app.press_commit(CommitTarget::Stashes);
    paint(&mut app);
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Stash(0)));

    // 点一下：两个文件，已跟踪的在前；预览标签里是第一个，标题写着储藏的名字。
    app.press_commit(CommitTarget::Stash(0));
    let view = app.commit.stash_view.clone().unwrap();
    let names: Vec<String> = (view.files.iter())
        .map(|(f, ..)| winc_editor::display_name(&f.path).into_owned())
        .collect();
    assert_eq!(names, ["main.rs", "notes.txt"]);
    assert!(view.files[1].1.is_none());
    let doc = app.editor.active().unwrap();
    assert!(doc.diff.is_some());
    assert_eq!(app.commit.stash_preview, Some(doc.id));
    assert!(doc.name().contains("stash@{0}"));
    assert!(app.diff_steps_files());
    // 下一个文件是一起收进去的未跟踪文件。
    app.step_diff_file(true);
    assert_eq!(app.commit.stash_view.as_ref().unwrap().at, 1);
    assert!(app.editor.active().unwrap().name().contains("notes.txt"));
    assert_eq!(app.next_stash_file(true), None);

    // 储藏那一行下面列出它的两个文件，点击第一个时预览标签切回该文件。
    use crate::app::StashLine;
    let lines = [StashLine::Stash(0), StashLine::File(0), StashLine::File(1)];
    assert_eq!(app.stash_lines(), lines);
    paint(&mut app);
    let section = app.commit_parts().stashes.unwrap();
    assert_eq!(
        section.h,
        4.0 * COMMIT_ROW + 2.0 * crate::app::git::commit::STASH_GAP
    );
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::StashFile(1)));
    app.press_commit(CommitTarget::StashFile(0));
    assert_eq!(app.commit.stash_view.as_ref().unwrap().at, 0);
    assert!(app.editor.active().unwrap().name().contains("main.rs"));

    // 再次点击这条储藏：文件收起，预览标签保留；再次点击时重新展开。
    app.press_commit(CommitTarget::Stash(0));
    assert_eq!(app.stash_lines(), [StashLine::Stash(0)]);
    assert!(app.editor.active().unwrap().name().contains("main.rs"));
    app.press_commit(CommitTarget::Stash(0));
    assert_eq!(app.stash_lines(), lines);
    // 关闭预览标签：文件不再列出。
    app.run_action(Action::CloseTab);
    paint(&mut app);
    assert_eq!(app.stash_lines(), [StashLine::Stash(0)]);

    // 「应用」：改动回到工作区，储藏还在。
    app.press_commit(CommitTarget::StashApply(0));
    app.git_refresh();
    assert!(root.join("notes.txt").exists());
    assert_eq!(app.git.stashes.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

/// 储藏一组像抽屉一样拉开：途中一组的高度和箭头的角度都在两端之间，上面的改动列表跟着变矮，
/// 还在移动的行不响应点击；停稳以后各行才能点。
#[test]
fn the_stash_group_opens_like_a_drawer() {
    let Some((mut app, root)) = git_app("git-commit-stash-drawer", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { stashed(); }\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "-m", "试一下新写法"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { now(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let closed = app.commit_parts().stashes.unwrap();
    app.press_commit(CommitTarget::Stashes);
    let open = app.commit_parts().stashes.unwrap();
    assert_eq!(open.h, closed.h + COMMIT_ROW);

    app.commit.stash_motion = Some((closed.h, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    let parts = app.commit_parts();
    let half = parts.stashes.unwrap();
    assert!(closed.h < half.h && half.h < open.h, "{half:?}");
    assert_eq!(half.bottom(), open.bottom());
    assert_eq!(parts.list.bottom(), half.y);
    assert!(0.0 < app.stash_chevron() && app.stash_chevron() < 1.0);
    assert!(app.animating());
    paint(&mut app);
    let has = |app: &App, target| (app.hits.commit.iter()).any(|(_, t)| *t == target);
    assert!(has(&app, CommitTarget::Stashes));
    assert!(!has(&app, CommitTarget::Stash(0)));

    app.commit.stash_motion = Some((closed.h, Clock::frozen(TREE_ANIMATION, TREE_ANIMATION)));
    app.settle_animations();
    assert!(app.commit.stash_motion.is_none());
    paint(&mut app);
    assert!(has(&app, CommitTarget::Stash(0)));
    assert_eq!(app.stash_chevron(), 1.0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 上一次提交推送以后「上一次提交」那一栏收起：途中栏变矮、照样写着那个提交，上面的改动列表
/// 跟着变高，「撤销」不再响应点击；停稳以后栏不再占地方。
#[test]
fn the_last_commit_bar_folds_away_once_it_is_pushed() {
    let Some((mut app, root)) = git_app("git-undo-fold", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "加上改动"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.track_commit_bars();
    let (bar, ..) = app.commit_parts().undo.unwrap();

    // 推送了：远程分支指向同一个提交，不能再撤销。
    git_in(
        &root,
        &["remote", "add", "origin", "https://example.invalid/r.git"],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    assert!(app.commit_undoable().is_none());
    app.track_commit_bars();
    let gone = app.commit_parts();
    assert!(gone.undo.is_none());

    app.commit.undo.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    let parts = app.commit_parts();
    let (half, ..) = parts.undo.unwrap();
    assert!(0.0 < half.h && half.h < bar.h, "{half:?}");
    assert_eq!(half.bottom(), bar.bottom());
    assert_eq!(parts.list.bottom(), half.y);
    assert_eq!(app.commit_undo_shown().unwrap().subject, "加上改动");
    paint(&mut app);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Undo));

    app.commit.undo.motion = Some((1.0, Clock::frozen(TREE_ANIMATION, TREE_ANIMATION)));
    app.settle_animations();
    assert!(app.commit.undo.motion.is_none());
    assert_eq!(app.commit_parts().list, gone.list);
    let _ = std::fs::remove_dir_all(&root);
}

/// 改动都提交以后提交卡片滑出视图底部：途中卡片大小不变、整体往下移，上面的改动列表跟着变高，
/// 卡片上的按钮不再响应点击；停稳以后卡片不再占地方。
#[test]
fn the_commit_card_slides_away_once_nothing_is_left_to_commit() {
    let Some((mut app, root)) = git_app("git-commit-card-slide", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let rest = app.commit_parts();
    let card = rest.card.unwrap();
    // 测试里没有窗口，记下的只是「上一帧显示着卡片」，不会开始过渡。
    app.track_commit_bars();
    assert!(app.commit.card.motion.is_none());

    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    app.git_refresh();
    app.track_commit_bars();
    assert!(app.commit.card.motion.is_none());
    let gone = app.commit_parts();
    assert!(gone.card.is_none());

    app.commit.card.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    let parts = app.commit_parts();
    let half = parts.card.unwrap();
    assert_eq!((half.x, half.w, half.h), (card.x, card.w, card.h));
    assert!(
        card.y < half.y && half.y < app.layout.sidebar.bottom(),
        "{half:?}"
    );
    assert!(rest.list.bottom() < parts.list.bottom());
    assert!(parts.list.bottom() < gone.list.bottom());
    paint(&mut app);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Commit));

    app.commit.card.motion = Some((1.0, Clock::frozen(TREE_ANIMATION, TREE_ANIMATION)));
    app.settle_animations();
    assert!(app.commit.card.motion.is_none());
    assert_eq!(app.commit_parts().list, gone.list);
    let _ = std::fs::remove_dir_all(&root);
}

/// ⌥ 按着时提交按钮写「提交并推送」，点了先问再提交、推送；修改上一次提交时不换。
#[test]
fn holding_option_turns_commit_into_commit_and_push() {
    let Some((mut app, root)) = git_app("git-commit-option", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.edit_field(|f| f.insert("修复"));
    paint(&mut app);
    let button = app.commit_parts().commit;
    app.on_mouse_move(button.x + button.w / 2.0, button.y + button.h / 2.0);
    let tip = |app: &App| crate::view::tests::tooltip(app).map(|(_, text)| text);
    assert!(!app.commit_pushes());
    assert!(
        tip(&app).is_some_and(|t| t.starts_with("提交到 main")),
        "{:?}",
        tip(&app)
    );
    app.modifiers = winit::keyboard::ModifiersState::ALT;
    assert!(app.commit_pushes());
    paint(&mut app);
    assert!(
        tip(&app).is_some_and(|t| t.starts_with("提交并推送 main")),
        "{:?}",
        tip(&app)
    );
    app.commit.amend = true;
    assert!(!app.commit_pushes());
    app.commit.amend = false;
    app.modifiers = winit::keyboard::ModifiersState::empty();
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交按钮变宽（例如按住 ⌥）到放不下「修改上次提交」时：按钮宽度从原宽度过渡到占满整行；
/// 「修改上次提交」在过渡中仍绘制并淡出，不响应鼠标，按钮的菜单里已有该项；过渡结束后不再排出。
#[test]
fn the_commit_button_widens_while_the_amend_toggle_fades_out() {
    use crate::app::animation::{Clock, TREE_ANIMATION};
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-commit-button-motion", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.edit_field(|f| f.insert("修复"));
    paint(&mut app);
    let before = app.commit_parts();
    let from = before.commit.w + before.commit_menu.w;
    assert!(before.amend.is_some());

    app.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    let settled = app.commit_parts();
    let to = settled.commit.w + settled.commit_menu.w;
    assert!(settled.amend.is_none());
    assert!(to > from);

    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.commit.button_motion = Some((from, half));
    app.commit.amend_motion = Some((false, half));
    paint(&mut app);
    let parts = app.commit_parts();
    let width = parts.commit.w + parts.commit_menu.w;
    assert!(width > from && width < to, "{from} < {width} < {to}");
    assert_eq!(parts.commit_menu.right(), settled.commit_menu.right());
    assert!(parts.amend.is_some());
    assert!(
        !app.hits
            .commit
            .iter()
            .any(|(_, t)| *t == CommitTarget::Amend)
    );
    let amend = MenuEntry::Item(MenuCommand::CommitAmend, true);
    assert!(app.commit_button_menu().contains(&amend));
    assert!(app.animating());

    app.commit.button_motion = None;
    app.commit.amend_motion = None;
    assert_eq!(app.commit_parts(), settled);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：侧栏宽度刚好放得下「修改上次提交」和按钮的一种写法、放不下另一种时按下或松开 ⌥，
/// 按钮变宽：过渡前、过渡的四分之一处、过渡结束。
#[test]
#[ignore]
fn shoot_commit_button_motion() {
    use crate::app::animation::{Clock, TREE_ANIMATION};
    use winit::keyboard::ModifiersState;

    let Some((mut app, root)) = git_app("git-commit-button-motion-shot", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.edit_field(|f| f.insert("修复"));
    let fits = |app: &mut App, modifiers| {
        app.modifiers = modifiers;
        paint(app);
        app.commit_parts().amend.is_some()
    };
    let [plain, alt] = [ModifiersState::empty(), ModifiersState::ALT];
    let (from_keys, to_keys) = loop {
        match (fits(&mut app, plain), fits(&mut app, alt)) {
            (true, false) => break (plain, alt),
            (false, true) => break (alt, plain),
            _ => {}
        }
        assert!(app.sidebar.width > crate::theme::MIN_PANEL_WIDTH);
        app.sidebar.width -= 2.0;
    };
    app.modifiers = from_keys;
    shoot(&mut app, "commit-button-before");
    let before = app.commit_parts();
    let from = before.commit.w + before.commit_menu.w;
    app.modifiers = to_keys;
    paint(&mut app);
    let quarter = Clock::frozen(TREE_ANIMATION / 4, TREE_ANIMATION);
    app.commit.button_motion = Some((from, quarter));
    app.commit.amend_motion = Some((false, quarter));
    shoot(&mut app, "commit-button-quarter");
    app.commit.button_motion = None;
    app.commit.amend_motion = None;
    shoot(&mut app, "commit-button-after");
    app.modifiers = plain;
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交以后改动列表不是一下子换掉：过渡途中离开的行按刷新前的状态还画着、各行不响应点击，空状态
/// 同时淡入；只是保存了文件（改动的文件和状态都没变）时不做过渡。
#[test]
fn committed_rows_fade_out_instead_of_vanishing() {
    let Some((mut app, root)) = git_app("git-commit-shift", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let old = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    assert!(!rows.is_empty());
    let has_rows = |app: &App| {
        (app.hits.commit.iter())
            .any(|(_, t)| matches!(t, CommitTarget::File(_) | CommitTarget::Group(_)))
    };
    assert!(has_rows(&app));

    // 保存（内容变了，文件和状态都没变）：不算变化。
    assert!(!crate::app::git::commit::entries_changed(
        &old,
        &old.clone()
    ));

    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "all"]);
    app.git_refresh();
    let new = app.git.status.clone().unwrap();
    assert!(new.entries.is_empty());
    assert!(crate::app::git::commit::entries_changed(&old, &new));

    app.commit.shift = Some(crate::app::git::commit::CommitShift {
        status: old,
        rows,
        scroll: 0.0,
        clock: Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION),
    });
    assert!(app.animating());
    paint(&mut app);
    assert!(!has_rows(&app));
    assert!(app.commit.overflow.is_none());

    app.commit.shift.as_mut().unwrap().clock = Clock::frozen(TREE_ANIMATION, TREE_ANIMATION);
    app.settle_animations();
    assert!(app.commit.shift.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 显示全名的改动行覆盖列表的滚动条：鼠标移到这一行的右端时仍悬停在这一行上（全名保持显示），
/// 在伸出侧栏的部分上滚动时滚动的是改动列表。
#[test]
fn the_full_name_row_covers_the_scrollbar_like_the_file_tree() {
    use winit::event::MouseScrollDelta;
    let Some((dir, long)) = long_name_repo("git-commit-overflow-bar") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    let (i, index) = (rows.iter().enumerate())
        .find_map(|(i, r)| match r {
            CommitRow::File { index, .. } if status.entries[*index].path.ends_with(&long) => {
                Some((i, *index))
            }
            _ => None,
        })
        .unwrap();
    let list = app.commit_parts().list;
    assert!(app.commit_max_scroll() > 0.0, "list must scroll");
    let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    let spill = app.commit.overflow.and_then(|o| o.spill).unwrap();

    // 移到滚动条的可点击范围内：仍悬停在这一行上，全名保持显示。
    app.on_mouse_move(list.right() - 3.0, y);
    paint(&mut app);
    assert_eq!(app.hover, Hover::Commit(CommitTarget::File(index)));
    assert!(app.commit.overflow.is_some_and(|o| o.row == i));

    // 在伸出侧栏的部分上滚动：滚动的是改动列表。
    app.on_mouse_move(spill.x + 20.0, y);
    let before = app.commit.scroll;
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -2.0));
    assert!(app.commit.scroll > before, "{} {before}", app.commit.scroll);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 显示全名的改动行覆盖在侧栏与编辑区之间的分隔条之上，与文件树的处理相同：鼠标从名字经分隔条
/// 移到伸出侧栏的部分时，始终悬停在这一行上，全名不收回；在此处按下选中这一行，不拖动分隔条。
/// 其他行仍可拖动分隔条。
#[test]
fn the_full_name_row_covers_the_splitter_like_the_file_tree() {
    let Some((dir, long)) = long_name_repo("git-commit-overflow-splitter") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    let (i, index) = (rows.iter().enumerate())
        .find_map(|(i, r)| match r {
            CommitRow::File { index, .. } if status.entries[*index].path.ends_with(&long) => {
                Some((i, *index))
            }
            _ => None,
        })
        .unwrap();
    let list = app.commit_parts().list;
    let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll + COMMIT_ROW / 2.0;
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    assert!(app.commit.overflow.is_some_and(|o| o.row == i));

    let splitter = app.layout.splitter;
    let row = Hover::Commit(CommitTarget::File(index));
    for x in [
        splitter.x + 1.0,
        splitter.x + splitter.w / 2.0,
        splitter.right() - 1.0,
    ] {
        app.on_mouse_move(x, y);
        paint(&mut app);
        assert_eq!(app.hover, row, "x = {x}");
        assert!(app.commit.overflow.is_some_and(|o| o.row == i), "x = {x}");
    }
    click(&mut app, (splitter.x + splitter.w / 2.0, y));
    assert_eq!(
        app.commit.selected.as_deref(),
        Some(status.entries[index].path.as_path())
    );
    assert_eq!(app.layout.splitter, splitter);

    // 其他行：分隔条正常响应。
    let other = y + 3.0 * COMMIT_ROW;
    app.on_mouse_move(splitter.x + splitter.w / 2.0, other);
    paint(&mut app);
    assert_eq!(app.hover, Hover::Splitter);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 两处改动内容相同（加入了同样的一行）时各自勾选：勾掉其中一处只有这一处不提交，再点一次能
/// 勾回来；提交时也只少了勾掉的那一处。
#[test]
fn identical_changes_are_checked_one_by_one() {
    use winc_core::diff::Pick;
    let dir = search_folder("git-identical-hunks");
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
    let disk = std::fs::read_to_string(&file).unwrap();
    let disk = disk
        .replace(
            "let value_20 = 20;\n",
            "let value_20 = 20;\nlet same = 1;\n",
        )
        .replace(
            "let value_40 = 40;\n",
            "let value_40 = 40;\nlet same = 1;\n",
        );
    std::fs::write(&file, &disk).unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);
    let picks = |app: &App| -> Vec<Pick> {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let entry = app.diff_entry(view).unwrap();
        let all: Vec<_> = (view.keys.iter().zip(&view.hunks))
            .map(|(&k, h)| (k, h.old.clone()))
            .collect();
        app.commit.hunk_inclusions(entry, &all)
    };
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let same: Vec<usize> = (0..view.hunks.len())
        .filter(|&h| (0..view.hunks.len()).any(|o| o != h && view.keys[o] == view.keys[h]))
        .collect();
    assert_eq!(same.len(), 2, "two changes with the same content");
    let (first, second) = (same[0], same[1]);

    // 勾掉第二处：只有它不勾；再点一次勾回来。
    app.press_diff(DiffTarget::Include(second));
    let now = picks(&app);
    assert_eq!((&now[first], &now[second]), (&Pick::Apply, &Pick::Skip));
    app.press_diff(DiffTarget::Include(second));
    assert!(picks(&app).iter().all(|p| *p == Pick::Apply));

    // 勾掉第一处后提交：只少了第一处加的那一行。
    app.press_diff(DiffTarget::Include(first));
    let now = picks(&app);
    assert_eq!((&now[first], &now[second]), (&Pick::Skip, &Pick::Apply));
    app.commit.message.set_text("only the second");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap();
    assert_eq!(head.matches("let same = 1;").count(), 1, "{head}");
    assert!(head.contains("let value_40 = 40;\nlet same = 1;"), "{head}");
    let _ = std::fs::remove_dir_all(&dir);
}
