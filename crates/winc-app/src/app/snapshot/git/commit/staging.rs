//! 暂存区视图的测试：各项的状态取自暂存区，提交取暂存区的内容。
//!
//! 按段、按行暂存的测试打开整体的差异（上次提交 ↔ 工作区，编辑区中「显示当前文件的改动」
//! 打开的就是它），勾选框显示各段在暂存区中的状态；从两组打开的分开显示的差异见 `split`。

use super::*;
use crate::app::git::Target;

/// 返回 `root` 中 `HEAD` 里的 `src/main.rs`。
fn head_main(root: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// 返回 `root` 的暂存区相对 `HEAD` 的差异（`git diff --cached`）。
fn staged_diff(root: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--cached"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// 返回文件 `path` 的勾选框状态（见 [`CommitState::file_state`]）。
fn file_state(app: &App, path: &Path) -> Option<bool> {
    let status = app.git.status.as_ref().unwrap();
    let entry = status.entries.iter().find(|e| e.path == path).unwrap();
    app.git.commit.file_state(entry)
}

/// 返回当前差异标签中各段改动的暂存状态。
fn picks(app: &App) -> Vec<winc_core::diff::Pick> {
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let entry = app.diff_entry(view).unwrap();
    app.git
        .commit
        .hunk_inclusions(entry, view, app.active_diff_kind())
}

#[test]
fn only_the_staged_hunks_are_committed() {
    use winc_core::diff::Pick;
    let dir = search_folder("git-partial-staging");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.git.commit.set_staging(true);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    // 两处改动各有一个勾选框，都没有暂存；勾上第一处（`value_5` 改成 500），文件变成「部分」。
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::Include(1))
    );
    assert_eq!(picks(&app), [Pick::Skip, Pick::Skip]);
    assert_eq!(file_state(&app, &file), None);
    app.press_diff(DiffTarget::Include(0));
    assert_eq!(picks(&app), [Pick::Apply, Pick::Skip]);
    assert_eq!(file_state(&app, &file), Some(false));
    // 暂存区里只有这一处：与在终端里看到的一致。
    let staged = staged_diff(&root);
    assert!(staged.contains("+let value_5 = 500;"), "{staged}");
    assert!(!staged.contains("-let value_50 = 50;"), "{staged}");

    // 暂存之后又修改了别处：新的修改不在暂存区里，不进入提交。
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
    let head = head_main(&root);
    assert!(head.contains("let value_5 = 500;"));
    assert!(head.contains("let value_50 = 50;") && !head.contains("5000"));
    // 没提交的那处还在工作区里，文件仍然有改动，不再有暂存的内容。
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("let value_50 = 5000;"));
    app.git_refresh();
    assert_eq!(file_state(&app, &file), None);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 暂存区视图：已暂存的文件列在「暂存的更改」一组，其余都在「更改」中；没有勾选框，行尾和
/// 组标题上是加号、减号，组标题保留回滚；只暂存了一部分的文件两组都有。提交取暂存区的内容。
#[test]
fn the_staging_view_groups_staged_changes_and_has_plus_and_minus() {
    let dir = search_folder("git-staging-view");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    let mut app = app_with(&dir, false);
    app.git.commit.set_staging(true);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let root = app.git.root.clone().unwrap();
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let file = |index| CommitRow::File { index, depth: 0 };
    let staged = CommitRow::Group(CommitGroup::Staged);
    let changes = CommitRow::Group(CommitGroup::Changes);
    assert_eq!(
        app.commit_rows()[..],
        [staged.clone(), file(1), changes.clone(), file(0)]
    );
    let boxes =
        |t: &CommitTarget| matches!(t, CommitTarget::Check(_) | CommitTarget::GroupCheck(_));
    assert!(!app.hits.commit.iter().any(|(_, t)| boxes(t)));

    // 鼠标移到「更改」中的文件上，行尾出现加号；点它即暂存，文件移到「已暂存的更改」。
    let hit = |app: &App, t: CommitTarget| {
        let found = app.hits.commit.iter().find(|(_, h)| *h == t);
        found.map(|(rect, _)| *rect)
    };
    let hover = |app: &mut App, t: CommitTarget| {
        let row = hit(app, t).unwrap();
        app.on_mouse_move(row.x + 40.0, row.y + 4.0);
        paint(app);
    };
    hover(&mut app, CommitTarget::File(0));
    assert!(hit(&app, CommitTarget::StageFile(0, false)).is_none());
    let plus = hit(&app, CommitTarget::StageFile(0, true)).unwrap();
    click(&mut app, (plus.x + 4.0, plus.y + 4.0));
    assert_eq!(file_state(&app, &main), Some(true));
    assert_eq!(app.commit_rows()[..], [staged.clone(), file(0), file(1)]);
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(2)));

    // 暂存之后又修改：这个文件只暂存了一部分，两组都列着它。
    std::fs::write(&main, "fn main() { let later = 1; }\n").unwrap();
    app.git_refresh();
    paint(&mut app);
    assert_eq!(
        app.commit_rows()[..],
        [staged.clone(), file(0), file(1), changes.clone(), file(0)]
    );

    // 「已暂存的更改」标题上的减号：全部取消暂存，新文件恢复为未跟踪。
    hover(&mut app, CommitTarget::Group(CommitGroup::Staged));
    let minus = hit(&app, CommitTarget::GroupStage(CommitGroup::Staged)).unwrap();
    click(&mut app, (minus.x + 4.0, minus.y + 4.0));
    assert_eq!(file_state(&app, &main), None);
    let status = app.git.status.clone().unwrap();
    assert!(
        status
            .entries
            .iter()
            .any(|e| e.path == new && e.untracked())
    );
    assert_eq!(app.commit_scope(), Some(CommitScope::All(1)));
    assert_eq!(app.commit_rows()[..], [changes.clone(), file(0), file(1)]);

    // 键盘：选中文件后按空格暂存。只提交暂存区里的内容，未跟踪的文件仍未跟踪。
    paint(&mut app);
    app.press_commit(CommitTarget::File(0));
    app.commit_list_key(&Key::Named(NamedKey::Space));
    assert_eq!(file_state(&app, &main), Some(true));
    app.git.commit.message.set_text("只有 main.rs");
    app.git_commit();
    let status = app.git.status.clone().unwrap();
    let left: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(left, std::slice::from_ref(&new));
    assert!(head_main(&root).contains("let later = 1;"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn untracked_files_stay_out_of_the_commit_until_staged() {
    let Some((mut app, root)) = git_app("git-commit-untracked-staging", false) else {
        return;
    };
    app.git.commit.set_staging(true);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let status = app.git.status.clone().unwrap();

    // 未跟踪的文件列在「更改」中；没有暂存任何内容时「提交全部」仍不含未跟踪的文件。
    assert_eq!(file_state(&app, &new), None);
    assert_eq!(
        app.git.commit.group_state(&status, CommitGroup::Changes),
        None
    );
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);
    assert_eq!(app.commit_scope(), Some(CommitScope::All(1)));

    // 收起和展开「更改」不改变暂存状态；收起时不在这组的文件之间切换。
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    assert!(app.commit_files().is_empty());
    assert_eq!(file_state(&app, &new), None);
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);

    // 单独暂存新文件：成为新增的文件，移到「暂存的更改」；这时只提交暂存区里的它。
    app.press_commit(CommitTarget::StageFile(1, true));
    assert_eq!(file_state(&app, &new), Some(true));
    let status = app.git.status.clone().unwrap();
    assert!(status.entries.iter().all(|e| !e.untracked()));
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));
    app.stage_paths(std::slice::from_ref(&new), false);
    assert_eq!(file_state(&app, &new), None);

    // 没有暂存时提交只包含已跟踪的改动，未跟踪的文件仍未跟踪。
    app.git.commit.message.set_text("只提交改动");
    app.git_commit();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].path, new);
    assert!(status.entries[0].untracked());

    // 只剩未跟踪的文件时没有可以提交的内容；暂存以后随提交加入仓库。
    assert_eq!(app.commit_scope(), None);
    app.git.commit.message.set_text("加入新文件");
    assert!(app.commit_blocker(&status).is_some());
    app.press_commit(CommitTarget::GroupStage(CommitGroup::Changes));
    app.git_commit();
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_the_staged_lines_are_committed() {
    let dir = search_folder("git-partial-lines-staging");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.git.commit.set_staging(true);
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
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);

    // 暂存 `added_b` 所在的那处改动，再把 `added_b` 这一行取消暂存；另一处改动不暂存。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    let hunk = view
        .hunks
        .iter()
        .position(|h| h.new.contains(&line))
        .unwrap();
    app.press_diff(DiffTarget::Include(hunk));
    assert_eq!(picks(&app)[hunk], winc_core::diff::Pick::Apply);
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    paint(&mut app);
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::IncludeLine(RIGHT, line))
    );
    assert!(matches!(picks(&app)[hunk], winc_core::diff::Pick::Lines(_)));
    assert_eq!(file_state(&app, &file), Some(false));
    let staged = staged_diff(&root);
    assert!(staged.contains("+let added_a = 1;") && !staged.contains("added_b"));

    app.git.commit.message.set_text("only added_a");
    app.git_commit();
    let head = head_main(&root);
    assert!(head.contains("let added_a = 1;"), "{head}");
    assert!(!head.contains("added_b"), "{head}");
    assert!(head.contains("let value_50 = 50;"), "{head}");
    // 没提交的行还在工作区里。
    assert!(std::fs::read_to_string(&file).unwrap().contains("added_b"));

    // 再暂存这一行：这处改动全部暂存。
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    let hunk = view
        .hunks
        .iter()
        .position(|h| h.new.contains(&line))
        .unwrap();
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    assert_eq!(picks(&app)[hunk], winc_core::diff::Pick::Apply);
    assert!(staged_diff(&root).contains("+let added_b = 2;"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 暂存之后又修改的文件变为「部分」：改过的那处显示为部分暂存，新的改动未暂存，提交取暂存时
/// 的内容。
#[test]
fn a_file_edited_after_staging_is_partly_staged() {
    use winc_core::diff::Pick;
    let Some((mut app, root)) = git_app("git-stage-then-edit", true) else {
        return;
    };
    app.git.commit.set_staging(true);
    let file = root.join("src/main.rs");
    app.run_action(Action::ShowCommit);
    app.stage_paths(std::slice::from_ref(&file), true);
    assert_eq!(file_state(&app, &file), Some(true));

    let disk = std::fs::read_to_string(&file).unwrap();
    let edited = disk
        .replace("let value_5 = 500;", "let value_5 = 555;")
        .replace("let value_30 = 30;", "let value_30 = 3000;");
    std::fs::write(&file, edited).unwrap();
    app.git_refresh();
    assert_eq!(file_state(&app, &file), Some(false));

    // 三处改动：`value_5` 暂存后又改过，`value_30` 是新的改动，删掉的 `value_50` 已暂存。
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    let now = picks(&app);
    assert!(matches!(now[0], Pick::Lines(_)), "{now:?}");
    assert_eq!(now[1..], [Pick::Skip, Pick::Apply]);

    app.git.commit.message.set_text("暂存时的内容");
    app.git_commit();
    let head = head_main(&root);
    assert!(head.contains("let value_5 = 500;") && head.contains("let value_30 = 30;"));
    assert!(!head.contains("let value_50 = 50;"));
    assert_eq!(file_state(&app, &file), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// 两处改动内容相同（加入了同样的一行）时各自暂存：暂存其中一处只有这一处进入暂存区，再点
/// 一次取消；提交时也只有暂存的那一处。
#[test]
fn identical_changes_are_staged_one_by_one() {
    use winc_core::diff::Pick;
    let dir = search_folder("git-identical-hunks-staging");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.git.commit.set_staging(true);
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
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let same: Vec<usize> = (0..view.hunks.len())
        .filter(|&h| (0..view.hunks.len()).any(|o| o != h && view.keys[o] == view.keys[h]))
        .collect();
    assert_eq!(same.len(), 2, "two changes with the same content");
    let (first, second) = (same[0], same[1]);

    // 暂存第一处：只有它暂存；再点一次取消。
    app.press_diff(DiffTarget::Include(first));
    let now = picks(&app);
    assert_eq!((&now[first], &now[second]), (&Pick::Apply, &Pick::Skip));
    app.press_diff(DiffTarget::Include(first));
    assert!(picks(&app).iter().all(|p| *p == Pick::Skip));
    assert_eq!(staged_diff(&root), "");

    // 暂存第二处后提交：提交里只有第二处加的那一行。
    app.press_diff(DiffTarget::Include(second));
    let now = picks(&app);
    assert_eq!((&now[first], &now[second]), (&Pick::Skip, &Pick::Apply));
    app.git.commit.message.set_text("only the second");
    app.git_commit();
    let head = head_main(&root);
    assert_eq!(head.matches("let same = 1;").count(), 1, "{head}");
    assert!(head.contains("let value_40 = 40;\nlet same = 1;"), "{head}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文件树和标签的右键菜单对有更改的文件提供暂存、取消暂存与回滚；没有更改的文件没有这几项。
/// 暂存区视图中回滚只放弃未暂存的更改，与提交视图相同；勾选式下回滚到上次提交。
///
/// 回归：暂存区视图中从文件树、标签回滚，已暂存的更改也被放弃。
#[test]
fn the_tree_and_tab_menus_stage_and_roll_back_a_changed_file() {
    use crate::app::input::context_menu::MenuCommand::{CommitCheck, FileHistory, GitRollbackFile};
    use crate::app::input::context_menu::MenuTarget;
    let Some((mut app, root)) = git_app("git-tree-menu-stage", false) else {
        return;
    };
    app.git.commit.set_staging(true);
    let file = root.join("Cargo.toml");
    let before = std::fs::read_to_string(&file).unwrap();
    let staged = format!("{before}# staged\n");
    std::fs::write(&file, &staged).unwrap();
    app.git_refresh();
    let commands = |app: &mut App, target: MenuTarget| -> Vec<_> {
        app.open_context_menu(target, (0.0, 0.0));
        menu_commands(app).into_iter().map(|(c, _)| c).collect()
    };
    let index = |root: &Path| {
        let out = std::process::Command::new("git")
            .args(["show", ":Cargo.toml"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };

    let listed = commands(&mut app, MenuTarget::Tree(Some(file.clone())));
    assert!(
        listed.ends_with(&[FileHistory, CommitCheck(true), GitRollbackFile]),
        "{listed:?}"
    );
    app.run_menu_command(CommitCheck(true));
    assert_eq!(file_state(&app, &file), Some(true));

    // 已全部暂存的文件只有「取消暂存」，没有可回滚的未暂存更改；标签的菜单相同。
    app.open_path(&file);
    let id = app.editor.active().unwrap().id;
    let listed = commands(&mut app, MenuTarget::Tab(id));
    assert!(
        listed.ends_with(&[FileHistory, CommitCheck(false)]),
        "{listed:?}"
    );

    // 暂存后又修改：回滚只放弃未暂存的更改，工作区恢复为暂存区的内容，暂存区不变。
    std::fs::write(&file, format!("{staged}# unstaged\n")).unwrap();
    app.git_refresh();
    let listed = commands(&mut app, MenuTarget::Tab(id));
    assert!(
        listed.ends_with(&[
            FileHistory,
            CommitCheck(true),
            CommitCheck(false),
            GitRollbackFile
        ]),
        "{listed:?}"
    );
    app.run_menu_command(GitRollbackFile);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), staged);
    assert_eq!(index(&root), staged);

    // 勾选式下回滚到上次提交，暂存区一起恢复；之后该文件没有更改，菜单里也没有这几项。
    app.git.commit.set_staging(false);
    std::fs::write(&file, format!("{staged}# unstaged\n")).unwrap();
    app.git_refresh();
    let _ = commands(&mut app, MenuTarget::Tree(Some(file.clone())));
    app.run_menu_command(GitRollbackFile);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    assert_eq!(index(&root), before);
    let listed = commands(&mut app, MenuTarget::Tree(Some(file)));
    assert!(listed.ends_with(&[FileHistory]), "{listed:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 在终端里只暂存了一部分的文件，勾选框为「部分」；提交的是暂存区里的内容，其余的改动留在
/// 工作区。
#[test]
fn files_staged_in_the_terminal_show_as_checked_and_commit_the_index() {
    let Some((mut app, root)) = git_app("git-commit-partly-staged-staging", false) else {
        return;
    };
    app.git.commit.set_staging(true);
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
    assert_eq!(app.git.commit.file_state(entry), Some(false));

    app.git.commit.message.set_text("提交暂存区里的内容");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout), "staged\n");
    let status = app.git.status.clone().unwrap();
    let entry = status.entries.iter().find(|e| e.path == main).unwrap();
    assert_eq!(app.git.commit.file_state(entry), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// 暂存区视图中暂存区为空时提交按钮显示「提交全部」，否则显示暂存的文件数；修改上次提交时不显示
/// 范围，游离 `HEAD` 时照常显示。
#[test]
fn the_commit_button_says_commit_all_until_something_is_staged() {
    let Some((mut app, root)) = git_app("git-commit-scope-staging", false) else {
        return;
    };
    app.git.commit.set_staging(true);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.commit_scope(), Some(CommitScope::All(1)));
    app.press_commit(CommitTarget::StageFile(0, true));
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));

    app.git.commit.amend = true;
    assert_eq!(app.commit_scope(), None);
    app.git.commit.amend = false;

    git_in(&root, &["checkout", "-q", "--detach"]);
    app.git_refresh();
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));

    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：暂存区视图（`commit-staging-*`），有已暂存、只暂存了一部分、未暂存和未跟踪的文件，鼠标
/// 停在「更改」中的一行上，行尾是「回滚」「暂存」。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_staging_view() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let dir = search_folder(&format!("commit-staging-{theme}"));
        if !long_repo(&dir) {
            return;
        }
        std::fs::write(dir.join("src/config.rs"), "pub struct Config;\n").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        std::fs::write(dir.join("notes.md"), "# notes\n").unwrap();
        git_in(&dir, &["add", "src/config.rs", "src/main.rs"]);
        let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
        let later = main.replace("let value_30 = 30;", "let value_30 = 3000;");
        std::fs::write(dir.join("src/main.rs"), later).unwrap();
        let mut app = app_with(&dir, dark);
        app.git.commit.set_staging(true);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        app.edit_field(|f| f.insert("调整配置"));
        paint(&mut app);
        let rows = app.commit_rows();
        let changes = rows
            .iter()
            .position(|r| *r == CommitRow::Group(CommitGroup::Changes));
        let index = match rows.get(changes.unwrap() + 1) {
            Some(CommitRow::File { index, .. }) => *index,
            _ => return,
        };
        let listed = |(_, t): &&(Rect, CommitTarget)| *t == CommitTarget::File(index);
        let row = app.hits.commit.iter().rfind(listed).unwrap().0;
        app.on_mouse_move(row.x + 60.0, row.y + 4.0);
        shoot(&mut app, &format!("commit-staging-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 「暂存全部更改」在暂存区视图中包含未跟踪的文件，勾选式下不包含。
#[test]
fn stage_all_matches_the_changes_group() {
    let Some((mut app, root)) = git_app("git-stage-all-untracked", false) else {
        return;
    };
    let new = root.join("src/new.rs");
    app.run_action(Action::GitStageAll);
    assert!(staged_diff(&root).contains("src/main.rs"));
    assert!(!staged_diff(&root).contains("src/new.rs"));
    app.run_action(Action::GitUnstageAll);

    app.git.commit.set_staging(true);
    app.run_action(Action::GitStageAll);
    assert_eq!(file_state(&app, &new), Some(true));
    let status = app.git.status.as_ref().unwrap();
    assert!(status.entries.iter().all(|e| e.unstaged.is_none()));
    let _ = std::fs::remove_dir_all(&root);
}

/// 设置中的「暂存区布局」与提交视图「⋯」菜单中的同名项是同一开关，切换后提交视图立即更新。
#[test]
fn the_staging_area_setting_and_the_menu_item_are_one_switch() {
    use crate::app::input::context_menu::MenuCommand::CommitStagingArea;
    use crate::layout::{Setting, SettingsPage};
    let Some((mut app, root)) = git_app("git-staging-setting", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    assert_eq!(Setting::GitStagingArea.page(), SettingsPage::Git);
    assert!(!app.switch_on(Setting::GitStagingArea));
    assert!(!app.git.commit.staging() && !app.menu_checked(CommitStagingArea));

    app.run_commit_more_command(CommitStagingArea);
    assert!(app.git.commit.staging() && app.menu_checked(CommitStagingArea));
    assert_eq!(app.config.git_staging_area, Some(true));
    let groups = |app: &App| -> Vec<CommitRow> {
        let rows = app.commit_rows();
        let titles = rows.iter().filter(|r| matches!(r, CommitRow::Group(_)));
        titles.cloned().collect()
    };
    // 暂存区为空：只有「更改」一组（含未跟踪的文件），按钮为「提交全部」。
    assert_eq!(app.commit_scope(), Some(CommitScope::All(1)));
    assert_eq!(groups(&app), [CommitRow::Group(CommitGroup::Changes)]);

    app.settings_toggle(Setting::GitStagingArea);
    assert!(!app.git.commit.staging() && !app.menu_checked(CommitStagingArea));
    assert_eq!(app.config.git_staging_area, Some(false));
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));
    let _ = std::fs::remove_dir_all(&root);
}

/// 暂存区视图的树形显示：目录行悬停时行尾是加号（「已暂存的更改」一组里是减号），暂存或取消
/// 暂存这个目录下的全部文件。
#[test]
fn folders_stage_and_unstage_everything_under_them() {
    let dir = search_folder("git-staging-folder");
    if !make_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.git.commit.set_staging(true);
    app.git.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let root = app.git.root.clone().unwrap();
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let src = |app: &App, group| {
        (app.commit_rows().iter())
            .position(|r| matches!(r, CommitRow::Dir { group: g, dir, .. } if *g == group && dir == Path::new("src")))
            .unwrap()
    };
    let hit = |app: &App, t: CommitTarget| {
        let found = app.hits.commit.iter().find(|(_, h)| *h == t);
        found.map(|(rect, _)| *rect)
    };
    let press = |app: &mut App, row: usize, on: bool| {
        let band = hit(app, CommitTarget::Dir(row)).unwrap();
        app.on_mouse_move(band.x + 40.0, band.y + 4.0);
        paint(app);
        assert!(hit(app, CommitTarget::DirStage(row, !on)).is_none());
        let button = hit(app, CommitTarget::DirStage(row, on)).unwrap();
        click(app, (button.x + 4.0, button.y + 4.0));
        paint(app);
    };

    // 「更改」里的 `src`：加号同时暂存改过的 `main.rs` 和未跟踪的 `new.rs`。
    let row = src(&app, CommitGroup::Changes);
    // 不悬停时没有按钮。
    assert!(hit(&app, CommitTarget::DirStage(row, true)).is_none());
    let band = hit(&app, CommitTarget::Dir(row)).unwrap();
    app.on_mouse_move(band.x + 40.0, band.y + 4.0);
    paint(&mut app);
    shoot(&mut app, "git-staging-folder");
    press(&mut app, row, true);
    assert_eq!(file_state(&app, &main), Some(true));
    assert_eq!(file_state(&app, &new), Some(true));
    // 「暂存的更改」里的 `src`：减号把两个都取消暂存。
    let row = src(&app, CommitGroup::Staged);
    press(&mut app, row, false);
    assert_eq!(file_state(&app, &main), None);
    let status = app.git.status.clone().unwrap();
    assert!(
        status
            .entries
            .iter()
            .any(|e| e.path == new && e.untracked())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 暂存区视图中回滚「更改」只放弃未暂存的更改，未跟踪的文件移到废纸篓；「暂存的更改」没有回滚
/// 按钮。
#[test]
fn rolling_back_changes_keeps_what_is_staged() {
    let Some((mut app, root)) = git_app("git-staging-group-rollback", false) else {
        return;
    };
    app.settings_toggle(Setting::GitStagingArea);
    app.trash = |path| std::fs::remove_file(path);
    let main = root.join("src/main.rs");
    let added = root.join("src/added.rs");
    let config = root.join("src/config.rs");
    let new = root.join("src/new.rs");
    let before_config = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&main, "staged\n").unwrap();
    std::fs::write(&added, "first\n").unwrap();
    git_in(&root, &["add", "src/main.rs", "src/added.rs"]);
    std::fs::write(&main, "staged\nnot staged\n").unwrap();
    std::fs::write(&added, "first\nsecond\n").unwrap();
    std::fs::write(&config, "pub struct Updated;\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let hover = |app: &mut App, group: CommitGroup| {
        let target = CommitTarget::Group(group);
        let rect = app
            .hits
            .commit
            .iter()
            .find(|(_, t)| *t == target)
            .unwrap()
            .0;
        app.on_mouse_move(rect.x + 40.0, rect.y + 4.0);
        paint(app);
    };
    let shown = |app: &App, target: CommitTarget| app.hits.commit.iter().any(|(_, t)| *t == target);

    hover(&mut app, CommitGroup::Staged);
    assert!(shown(&app, CommitTarget::GroupStage(CommitGroup::Staged)));
    assert!(!shown(&app, CommitTarget::GroupRevert(CommitGroup::Staged)));
    hover(&mut app, CommitGroup::Changes);
    assert!(shown(&app, CommitTarget::GroupStage(CommitGroup::Changes)));
    assert!(shown(&app, CommitTarget::GroupRevert(CommitGroup::Changes)));
    app.press_commit(CommitTarget::GroupRevert(CommitGroup::Changes));

    assert_eq!(std::fs::read_to_string(&main).unwrap(), "staged\n");
    assert_eq!(std::fs::read_to_string(&added).unwrap(), "first\n");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before_config);
    assert!(!new.exists());
    let diff = staged_diff(&root);
    assert!(
        diff.contains("+staged") && diff.contains("+first"),
        "{diff}"
    );
    let status = app.git.status.as_ref().unwrap();
    assert!(status.entries.iter().all(|e| e.unstaged.is_none()));
    let _ = std::fs::remove_dir_all(&root);
}

mod split;
