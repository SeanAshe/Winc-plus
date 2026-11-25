//! 改动列表各行行尾的按钮：同一套顺序、目录的回滚，以及其他仓库的行就地操作、不切换仓库。

use super::others::two_repositories;
use super::*;
use crate::app::OtherGroup;

/// 返回 `git -C repo args` 的标准输出（去掉末尾的换行）。
fn git_output(repo: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
}

/// 返回命中目标 `target` 的区域；这一帧没有画出时返回 `None`。
fn hit(app: &App, target: CommitTarget) -> Option<Rect> {
    let found = app.hits.commit.iter().find(|(_, t)| *t == target);
    found.map(|(rect, _)| *rect)
}

/// 第 `repo` 个仓库中路径为 `path` 的文件在其状态各项中的下标。
fn other_entry(app: &App, repo: usize, path: &Path) -> usize {
    let find = |_: &Path, status: &winc_editor::git::Status| {
        status.entries.iter().position(|e| e.path == path)
    };
    app.other_status(repo, find).flatten().unwrap()
}

/// 暂存区视图中未暂存的文件：回滚在最左，暂存在最右，中间是打开；状态字母照常显示在最右那一列。
#[test]
fn row_buttons_keep_one_order_and_the_status_letter() {
    let Some((mut app, root)) = git_app("git-row-buttons-order", false) else {
        return;
    };
    app.settings_toggle(Setting::GitStagingArea);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let main = root.join("src/main.rs");
    let entries = app.git.status.as_ref().unwrap().entries.clone();
    let index = entries.iter().position(|e| e.path == main).unwrap();
    let row = hit(&app, CommitTarget::File(index)).unwrap();
    app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
    paint(&mut app);
    let revert = hit(&app, CommitTarget::RevertFile(index)).unwrap();
    let open = hit(&app, CommitTarget::OpenFile(index)).unwrap();
    let stage = hit(&app, CommitTarget::StageFile(index, true)).unwrap();
    assert!(revert.right() <= open.x && open.right() <= stage.x);
    assert!(
        stage.right() + ACTION_SLOT <= row.right(),
        "状态字母那一列让出来"
    );

    // 组标题：回滚、暂存整组，与文件行的按钮靠同一条右边缘。
    let group = hit(&app, CommitTarget::Group(CommitGroup::Changes)).unwrap();
    app.on_mouse_move(group.x + 60.0, group.y + group.h / 2.0);
    paint(&mut app);
    let revert_all = hit(&app, CommitTarget::GroupRevert(CommitGroup::Changes)).unwrap();
    let stage_all = hit(&app, CommitTarget::GroupStage(CommitGroup::Changes)).unwrap();
    assert!(revert_all.right() <= stage_all.x);
    assert_eq!(stage_all.right(), stage.right());
    let _ = std::fs::remove_dir_all(&root);
}

/// 目录行的「回滚」回滚该目录下这一组的全部文件；未跟踪的文件在另一组，不受影响。
#[test]
fn a_folder_rolls_back_the_files_under_it() {
    let Some((mut app, root)) = git_app("git-folder-revert", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    if app.git.commit.flat() {
        app.toggle_commit_layout();
    }
    paint(&mut app);
    let rows = app.commit_rows();
    let row = (rows.iter())
        .position(|r| {
            matches!(
                r,
                CommitRow::Dir {
                    group: CommitGroup::Changes,
                    ..
                }
            )
        })
        .unwrap();
    app.press_commit(CommitTarget::DirRevert(row));
    app.poll_commit();
    app.git_refresh();
    assert_eq!(
        git_output(&root, &["status", "--porcelain"]),
        "?? src/new.rs"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 其他仓库的行：按钮作用于该仓库，当前仓库不变。选中其中的文件后按 `⌫` 回滚该文件，打开文件、
/// 回滚整组同样不切换仓库。
#[test]
fn other_repository_rows_act_in_place() {
    let Some((mut app, a, b)) = two_repositories("git-other-in-place", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    if !app.commit_shows_others() {
        app.run_commit_more_command(MenuCommand::CommitAllRepositories);
    }
    app.git_read_summaries();
    app.poll_git();
    paint(&mut app);
    let main = other_entry(&app, 1, &b.join("src/main.rs"));
    let row = hit(&app, CommitTarget::OtherFile(1, main)).unwrap();
    app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
    paint(&mut app);
    let revert = hit(&app, CommitTarget::OtherRevert(1, main)).unwrap();
    let open = hit(&app, CommitTarget::OtherOpen(1, main)).unwrap();
    assert!(revert.right() <= open.x);

    // 选中 `b` 的文件，按 `⌫`：回滚的是 `b` 的文件，当前仓库仍是 `a`。
    app.press_commit(CommitTarget::OtherFile(1, main));
    assert_eq!(app.git.root.as_ref(), Some(&a));
    app.commit_list_key(&Key::Named(NamedKey::Backspace));
    app.poll_commit();
    app.poll_git();
    assert_eq!(git_output(&b, &["status", "--porcelain"]), "?? src/new.rs");
    assert_eq!(
        git_output(&a, &["status", "--porcelain"]).lines().count(),
        2
    );

    // 打开 `b` 中的文件、回滚 `b` 的「未跟踪的文件」一组：都不切换仓库。
    app.git_read_summaries();
    app.poll_git();
    let new = other_entry(&app, 1, &b.join("src/new.rs"));
    app.press_commit(CommitTarget::OtherOpen(1, new));
    let opened = app.editor.active().and_then(|d| d.path.clone());
    assert_eq!(
        opened,
        Some(winc_editor::canonical_path(&b.join("src/new.rs")))
    );
    app.press_commit(CommitTarget::OtherGroupRevert(1, OtherGroup::Untracked));
    app.poll_commit();
    assert_eq!(git_output(&b, &["status", "--porcelain"]), "");
    assert_eq!(app.git.root.as_ref(), Some(&a));
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 截图：暂存区视图中悬停改动行（`commit-actions-*`）和另一个仓库的行（`commit-actions-other-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_row_buttons() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let name = format!("commit-actions-{theme}");
        let Some((mut app, a, b)) = two_repositories(&name, dark) else {
            return;
        };
        app.settings_toggle(Setting::GitStagingArea);
        app.run_action(Action::ShowCommit);
        if !app.commit_shows_others() {
            app.run_commit_more_command(MenuCommand::CommitAllRepositories);
        }
        app.git_read_summaries();
        app.poll_git();
        paint(&mut app);
        let main = a.join("src/main.rs");
        let entries = app.git.status.as_ref().unwrap().entries.clone();
        let index = entries.iter().position(|e| e.path == main).unwrap();
        let row = hit(&app, CommitTarget::File(index)).unwrap();
        app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
        paint(&mut app);
        shoot(&mut app, &name);
        let other = other_entry(&app, 1, &b.join("src/main.rs"));
        let row = hit(&app, CommitTarget::OtherFile(1, other)).unwrap();
        app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
        paint(&mut app);
        shoot(&mut app, &format!("commit-actions-other-{theme}"));
        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }
}
