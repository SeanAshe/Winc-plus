//! 提交视图中勾选与暂存的测试。

use super::*;

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
    assert_eq!(app.git.commit.file_state(entry), Some(false));

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

    app.git.commit.message.set_text("only value_5");
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
fn commit_view_commits_only_the_checked_files() {
    let dir = search_folder("git-commit");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.git.commit.set_flat(false);
    // 模块关闭时：侧栏没有 Git 按钮，⌃⇧G 不响应。
    assert_eq!(app.sidebar_views().len(), 2);
    assert!(!app.run_action(Action::ShowCommit));

    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    assert_eq!(app.sidebar_views().last(), Some(&SidebarView::Commit));
    assert!(app.run_action(Action::ShowCommit));
    assert_eq!(app.panels.sidebar.view, SidebarView::Commit);
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
    assert!(app.git.commit.checked(&status.entries[0]));
    assert!(app.git.commit.checked(&status.entries[1]));
    assert_eq!(
        app.git.commit.group_state(&status, CommitGroup::Changes),
        Some(true)
    );

    // 点 `src/new.rs` 的勾选框；点 `src/main.rs` 那一行打开差异。
    let target =
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let check = target(&app, CommitTarget::Check(1));
    click(&mut app, (check.x + 4.0, check.y + 4.0));
    assert!(!app.git.commit.checked(&status.entries[1]));
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
        app.git.commit.selected.as_deref(),
        Some(status.entries[1].path.as_path())
    );
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("new.rs · 差异")
    );
    key(&mut app, NamedKey::Space);
    assert!(app.git.commit.checked(&status.entries[1]));
    key(&mut app, NamedKey::ArrowUp);
    assert_eq!(
        app.git.commit.selected.as_deref(),
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
    assert_eq!(app.git.commit.message.text(), "加上 new.rs\n正文");
    app.commit_message_key(&Key::Named(NamedKey::Enter), true, false);
    assert!(!app.git.commit.busy);
    assert_eq!(app.git.commit.message.text(), "");
    let status = app.git.status.clone().unwrap();
    let left: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    let root = app.git.root.clone().unwrap();
    assert_eq!(left, [root.join("src/main.rs")]);
    // 用户取消过的勾选在提交后重置：剩下的 `src/main.rs` 重新处于勾选状态。
    assert!(app.git.commit.checked(&status.entries[0]));

    // 修改上一次提交：信息填进输入框，修改后提交。
    app.run_commit_button_command(crate::app::input::context_menu::MenuCommand::CommitAmend);
    assert_eq!(app.git.commit.message.text(), "加上 new.rs\n正文");
    app.git.commit.message.set_text("加上 new.rs 和 main.rs");
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
fn committing_a_checked_file_saves_its_unsaved_edits_first() {
    let Some((mut app, root)) = git_app("git-commit-dirty", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run(winc_editor::Command::Insert("// 还没保存\n".into()));
    assert!(app.editor.active().unwrap().is_dirty());
    app.run_action(Action::ShowCommit);
    app.git.commit.message.set_text("带上没保存的修改");
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
    assert!(!app.git.commit.group_collapsed(CommitGroup::Untracked));
    assert!(!app.git.commit.checked(entry));
    assert_eq!(
        app.git.commit.group_state(&status, CommitGroup::Untracked),
        None
    );
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);
    assert_eq!(app.commit_scope(), Some(1));

    // 收起和展开不改变勾选；收起时切换文件跳过这一组。
    app.press_commit(CommitTarget::Group(CommitGroup::Untracked));
    assert_eq!(app.commit_files(), std::slice::from_ref(&main));
    assert!(!app.git.commit.checked(entry));
    app.press_commit(CommitTarget::Group(CommitGroup::Untracked));
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);

    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    assert!(app.git.commit.checked(entry));
    assert_eq!(app.commit_scope(), Some(2));
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    assert!(!app.git.commit.checked(entry));

    // 没有勾上时提交只包含已跟踪的改动，未跟踪的文件仍未跟踪。
    app.git.commit.message.set_text("只提交改动");
    app.git_commit();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].path, new);
    assert!(status.entries[0].untracked());

    // 勾上以后随提交加入仓库；提交后勾选回到默认。
    assert_eq!(app.commit_scope(), None);
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    app.git.commit.message.set_text("加入新文件");
    app.git_commit();
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());
    std::fs::write(&new, "changed\n").unwrap();
    std::fs::write(root.join("another.rs"), "\n").unwrap();
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    let another = status.entries.iter().find(|e| e.untracked()).unwrap();
    assert!(!app.git.commit.checked(another));

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
    assert_eq!(app.git.commit.file_state(entry), Some(false));
    for other in (0..hunks).filter(|&h| h != hunk) {
        app.press_diff(DiffTarget::Include(other));
    }

    app.git.commit.message.set_text("only added_a");
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
    let inclusion = |app: &App, key| {
        app.git
            .commit
            .hunk_inclusions(&entry, &[(key, old.clone())])[0]
            .clone()
    };
    assert_eq!(inclusion(&app, key), Pick::Apply);

    // 勾掉一行：按行提交。再勾回来：整处提交。
    app.git.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    assert!(matches!(inclusion(&app, key), Pick::Lines(_)));
    app.git.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    assert_eq!(inclusion(&app, key), Pick::Apply);

    // 内容变了：行号不再对应，整处不提交。
    app.git.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    assert_eq!(inclusion(&app, key + 1), Pick::Skip);

    // 整段的勾选框把只提交部分行的改动恢复为整处提交。
    app.git.commit.toggle_hunk(&entry, 0, &all);
    assert_eq!(inclusion(&app, key), Pick::Apply);

    // 所有行都勾掉时文件也勾掉。
    for (side, line) in [(LEFT, 0), (LEFT, 1), (RIGHT, 0)] {
        app.git
            .commit
            .toggle_line(&entry, (0, 1), (side, line), &all);
    }
    assert_eq!(app.git.commit.file_state(&entry), None);

    // 文件没勾时勾上一行：文件勾上，只提交这一行。
    app.git.commit.toggle_line(&entry, (0, 1), (RIGHT, 0), &all);
    let Pick::Lines(set) = inclusion(&app, key) else {
        panic!("应当按行提交");
    };
    assert_eq!(set.old.len(), 2);
    assert!(set.new.is_empty());
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
        app.git.commit.hunk_inclusions(entry, &all)
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
    app.git.commit.message.set_text("only the second");
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
