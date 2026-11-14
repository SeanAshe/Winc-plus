//! 「显示全部仓库」的测试：其他仓库的更改列在当前仓库之后，点击即切换过去。

use super::*;
use crate::app::OtherRow;
use crate::app::input::context_menu::{MenuCommand, MenuEntry};

/// 建一个放着两个仓库 `a`、`b` 的目录并打开，各有一个改过的文件和一个未跟踪的文件。
fn two_repositories(name: &str, dark: bool) -> Option<(App, PathBuf, PathBuf)> {
    let dir = search_folder(name);
    for repo in ["a", "b"] {
        let repo = dir.join(repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() { }\n").unwrap();
        if !make_repo(&repo) {
            return None;
        }
    }
    let mut app = app_with(&dir, dark);
    app.config.repository_scan_depth = Some(1);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return None;
    }
    let dir = app.editor.root().unwrap().to_path_buf();
    Some((app, dir.join("a"), dir.join("b")))
}

#[test]
fn other_repositories_are_listed_after_the_current_one_and_a_click_switches() {
    let Some((mut app, a, b)) = two_repositories("git-all-repositories", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git.root.as_ref(), Some(&a));

    // 没有选过时随样式：暂存区视图只显示当前仓库，勾选式列出其他仓库。有多个仓库时「⋯」菜单
    // 里有开关，关掉以后只显示当前仓库。
    let toggle = MenuCommand::CommitAllRepositories;
    assert!(app.commit_more_menu().iter().any(|entry| match entry {
        MenuEntry::Submenu(_, items) => items.contains(&MenuEntry::Item(toggle, true)),
        _ => false,
    }));
    app.git.commit.set_staging(true);
    assert!(!app.menu_checked(toggle) && app.commit_other_rows().is_empty());
    app.git.commit.set_staging(false);
    assert!(app.menu_checked(toggle) && !app.commit_other_rows().is_empty());
    app.run_commit_more_command(toggle);
    assert!(!app.menu_checked(toggle) && app.commit_other_rows().is_empty());
    let alone = app.commit_max_scroll();
    app.run_commit_more_command(toggle);
    assert!(app.menu_checked(toggle));

    // 按仓库列表的顺序每个仓库一节：当前仓库的标题行下面是它的改动列表，另一个仓库的标题行
    // 下面按组列出它的文件，一起滚动。
    use crate::app::OtherGroup::{Changes, Untracked};
    let listed = [
        OtherRow::Current,
        OtherRow::Repository(1),
        OtherRow::Group(1, Changes),
        OtherRow::File(1, 0),
        OtherRow::Group(1, Untracked),
        OtherRow::File(1, 1),
    ];
    assert_eq!(app.commit_other_rows(), listed);
    assert_eq!(app.commit_rows_before(), 1);
    app.layout.sidebar.h = 200.0;
    assert!(app.commit_max_scroll() > alone);
    app.relayout();
    paint(&mut app);
    let rows = app.commit_rows().len();
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let current = hit(&app, CommitTarget::CurrentRepository);
    let first = hit(&app, CommitTarget::Group(CommitGroup::Changes));
    assert_eq!(first.y, current.y + COMMIT_ROW);
    let header = hit(&app, CommitTarget::OtherRepository(1));
    let last = hit(&app, CommitTarget::File(1));
    assert_eq!(header.y, last.y + COMMIT_ROW);
    assert_eq!(rows, 4);

    // 折叠：另一个仓库只剩标题行；当前仓库一节也只剩标题行，下面的仓库紧跟在它后面。
    app.press_commit(CommitTarget::OtherFold(1));
    assert_eq!(app.commit_other_rows(), listed[..2]);
    app.press_commit(CommitTarget::OtherFold(1));
    app.press_commit(CommitTarget::CurrentRepository);
    assert!(app.current_section_folded() && app.commit_block_len() == 0);
    paint(&mut app);
    assert_eq!(
        hit(&app, CommitTarget::OtherRepository(1)).y,
        current.y + COMMIT_ROW
    );
    let has = |app: &App, t: CommitTarget| app.hits.commit.iter().any(|(_, h)| *h == t);
    assert!(!has(&app, CommitTarget::Group(CommitGroup::Changes)));

    // 收起到一半：只记录当前仓库标题行及其上方的命中区域，正在移动的行不记录。
    let half = crate::app::Clock::frozen(
        std::time::Duration::from_millis(1000),
        std::time::Duration::from_millis(2000),
    );
    app.git.commit.fold_motion = Some(crate::app::git::commit::FoldMotion {
        rows: listed.to_vec(),
        parent: 0,
        count: 0,
        height: rows as f32 * COMMIT_ROW,
        expanding: false,
        clock: half,
    });
    paint(&mut app);
    assert!(has(&app, CommitTarget::CurrentRepository));
    assert!(!has(&app, CommitTarget::OtherRepository(1)));
    app.git.commit.fold_motion = None;
    app.press_commit(CommitTarget::CurrentRepository);
    assert!(!app.current_section_folded());
    paint(&mut app);

    // 点另一个仓库的文件：切换到那个仓库，选中该文件并预览差异；列表的顺序不变，✓ 移过去。
    let file = hit(&app, CommitTarget::OtherFile(1, 0));
    click(&mut app, (file.x + 60.0, file.y + 4.0));
    assert_eq!(app.git.root.as_ref(), Some(&b));
    let main = b.join("src/main.rs");
    assert_eq!(app.git.commit.selected.as_deref(), Some(main.as_path()));
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("main.rs · 差异")
    );
    assert_eq!(app.commit_other_rows()[0], OtherRow::Repository(0));
    assert_eq!(app.commit_other_rows().last(), Some(&OtherRow::Current));

    // 当前仓库没有更改时不画空状态：照样列出各个仓库，当前仓库的标题行下面没有行。
    git_in(&b, &["checkout", "-q", "--", "."]);
    std::fs::remove_file(b.join("src/new.rs")).unwrap();
    app.git_refresh();
    assert_eq!(app.commit_other_rows()[0], OtherRow::Repository(0));
    assert_eq!(app.commit_other_rows().last(), Some(&OtherRow::Current));
    paint(&mut app);
    let header = hit(&app, CommitTarget::OtherRepository(0));
    click(&mut app, (header.x + 60.0, header.y + 4.0));
    assert_eq!(app.git.root.as_ref(), Some(&a));
    // 没有更改的仓库只有一行。
    app.git_read_summaries();
    app.poll_git();
    assert_eq!(
        app.commit_other_rows(),
        [OtherRow::Current, OtherRow::Quiet(1)]
    );

    // 关闭后只剩当前仓库。
    app.run_commit_more_command(toggle);
    assert!(app.commit_other_rows().is_empty());
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 在列表里换仓库只换当前仓库，不换要提交的内容：各文件的勾选和提交信息照样留着。修改上一次
/// 提交时不一并提交其他仓库，它们的勾选留着；提交按钮只算列表里看得到的勾选。
#[test]
fn switching_repositories_keeps_checks_and_the_message() {
    let Some((mut app, a, b)) = two_repositories("git-commit-switch-keeps", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 当前仓库 `a`：改过的文件默认勾上，未跟踪的不勾；`b` 的文件默认都不勾，这里只勾一个。
    app.git.commit.message.set_text("一起提交");
    app.press_commit(CommitTarget::OtherCheck(1, 0));
    let a_state = app.current_check_state();
    assert_eq!(a_state, Some(false));

    // 点 `b` 的文件：换到 `b`，勾选和提交信息不变。
    app.press_commit(CommitTarget::OtherFile(1, 0));
    assert_eq!(app.git.root.as_ref(), Some(&b));
    assert_eq!(app.git.commit.message.text(), "一起提交");
    assert_eq!(app.current_check_state(), Some(false));
    assert_eq!(app.other_group_state(0, None), a_state);
    assert_eq!(app.other_checked_count(), 1);

    // 换回 `a`：照样。
    app.press_commit(CommitTarget::OtherRepository(0));
    assert_eq!(app.git.root.as_ref(), Some(&a));
    assert_eq!(app.current_check_state(), a_state);
    assert_eq!(app.other_group_state(1, None), Some(false));
    assert_eq!(app.git.commit.message.text(), "一起提交");

    // 关掉「显示全部仓库」、当前仓库一个都不勾：只剩看不到的勾选时提交按钮不可用。
    // 部分勾选时先全部勾上，再点一次全部取消。
    app.press_commit(CommitTarget::CurrentCheck);
    app.press_commit(CommitTarget::CurrentCheck);
    assert_eq!(app.current_check_state(), None);
    let status = app.git.status.clone().unwrap();
    assert!(app.commit_blocker(&status).is_none());
    app.run_commit_more_command(MenuCommand::CommitAllRepositories);
    assert!(app.commit_blocker(&status).is_some());
    app.run_commit_more_command(MenuCommand::CommitAllRepositories);

    // 修改上一次提交：只改当前仓库，`b` 勾上的文件不提交，勾选留着。
    app.press_commit(CommitTarget::CurrentCheck);
    app.git.commit.amend = true;
    app.git.commit.message.set_text("改过的说明");
    app.git_commit();
    assert_eq!(app.other_checked_count(), 1);
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 勾选式下其他仓库的文件有勾选框，默认不勾；勾上的文件随当前仓库的一起提交，各自提交到所在
/// 的仓库，用同一条提交信息。点勾选框不切换仓库。
#[test]
fn checked_files_of_other_repositories_are_committed_alongside() {
    let Some((mut app, a, b)) = two_repositories("git-commit-other-repositories", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.other_group_state(1, None), None);
    assert_eq!(app.other_checked_count(), 0);
    let subject = |repo: &Path| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["log", "-1", "--format=%s"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let before = subject(&b);

    // 勾上另一个仓库改过的文件：仓库的勾选框成为部分勾选，当前仓库不变，按钮的文件数加一。
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let own = app.commit_scope();
    let check = hit(&app, CommitTarget::OtherCheck(1, 0));
    click(&mut app, (check.x + 8.0, check.y + 8.0));
    assert_eq!(app.git.root.as_ref(), Some(&a));
    assert_eq!(app.other_group_state(1, None), Some(false));
    assert_eq!(app.other_checked_count(), 1);
    assert_ne!(app.commit_scope(), own);
    // 「提交并推送」的确认框写明另外推送哪个仓库。
    assert_eq!(app.other_commit_names(), ["b"]);

    // 仓库标题行的勾选框：全部勾上，再点一次全部取消。
    paint(&mut app);
    let group = hit(&app, CommitTarget::OtherGroupCheck(1));
    click(&mut app, (group.x + 8.0, group.y + 8.0));
    assert_eq!(app.other_group_state(1, None), Some(true));
    click(&mut app, (group.x + 8.0, group.y + 8.0));
    assert_eq!(app.other_group_state(1, None), None);
    app.press_commit(CommitTarget::OtherCheck(1, 0));

    app.git.commit.message.set_text("两个仓库一起提交");
    app.git_commit();
    assert_eq!(subject(&a), "两个仓库一起提交");
    assert_eq!(subject(&b), "两个仓库一起提交");
    assert_ne!(before, subject(&b));
    assert_eq!(app.other_checked_count(), 0);

    // 没勾的文件留在另一个仓库里，没有被提交。
    assert!(b.join("src/new.rs").exists());
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&b)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "?? src/new.rs");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 暂存区视图中其他仓库的文件分为已暂存、未暂存两组，行尾的按钮暂存或取消暂存，不切换仓库；
/// 已暂存的内容随当前仓库一起提交，此时当前仓库没有暂存的更改不会被一并提交。
#[test]
fn staged_files_of_other_repositories_are_committed_alongside() {
    let Some((mut app, a, b)) = two_repositories("git-stage-other-repositories", false) else {
        return;
    };
    app.settings_toggle(Setting::GitStagingArea);
    app.run_action(Action::ShowCommit);
    app.run_commit_more_command(MenuCommand::CommitAllRepositories);
    app.poll_git();
    paint(&mut app);
    let git = |repo: &Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    use crate::app::OtherGroup::{Changes, Staged};
    let unstaged = [
        OtherRow::Current,
        OtherRow::Repository(1),
        OtherRow::Group(1, Changes),
        OtherRow::File(1, 0),
        OtherRow::File(1, 1),
    ];
    assert_eq!(app.commit_other_rows(), unstaged);
    assert_eq!(app.other_staged_count(), 0);

    // 暂存另一个仓库改过的文件：写入那个仓库的暂存区，当前仓库不变，按钮写明提交 1 个文件。
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    // 行尾的按钮悬停时才出现，取代状态字母。
    let has = |app: &App, t: CommitTarget| app.hits.commit.iter().any(|(_, h)| *h == t);
    assert!(!has(&app, CommitTarget::OtherStage(1, 0, true)));
    let row = hit(&app, CommitTarget::OtherFile(1, 0));
    app.on_mouse_move(row.x + 60.0, row.y + 4.0);
    paint(&mut app);
    let plus = hit(&app, CommitTarget::OtherStage(1, 0, true));
    click(&mut app, (plus.x + 8.0, plus.y + 8.0));
    app.poll_commit();
    app.poll_git();
    assert_eq!(app.git.root.as_ref(), Some(&a));
    assert_eq!(
        git(&b, &["status", "--porcelain"]),
        "M  src/main.rs\n?? src/new.rs"
    );
    let staged = [
        OtherRow::Current,
        OtherRow::Repository(1),
        OtherRow::Group(1, Staged),
        OtherRow::Staged(1, 0),
        OtherRow::Group(1, Changes),
        OtherRow::File(1, 1),
    ];
    assert_eq!(app.commit_other_rows(), staged);
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));

    // 分组行的按钮作用于整组：全部取消暂存，再全部暂存，然后把未跟踪的文件移出。
    app.press_commit(CommitTarget::OtherGroupStage(1, false));
    app.poll_commit();
    app.poll_git();
    assert_eq!(app.commit_other_rows(), unstaged);
    app.press_commit(CommitTarget::OtherGroupStage(1, true));
    app.poll_commit();
    app.poll_git();
    assert_eq!(app.other_staged_count(), 2);
    app.press_commit(CommitTarget::OtherStage(1, 1, false));
    app.poll_commit();
    app.poll_git();
    assert_eq!(app.commit_other_rows(), staged);

    // 提交：只有另一个仓库暂存的文件进入提交，当前仓库未暂存的更改原样保留。
    let status = app.git.status.clone().unwrap();
    assert!(app.commit_blocker(&status).is_some());
    app.git.commit.message.set_text("提交另一个仓库暂存的内容");
    assert_eq!(app.commit_blocker(&status), None);
    app.git_commit();
    app.poll_commit();
    assert_eq!(
        git(&b, &["log", "-1", "--format=%s"]),
        "提交另一个仓库暂存的内容"
    );
    assert_eq!(git(&b, &["status", "--porcelain"]), "?? src/new.rs");
    assert_eq!(git(&a, &["log", "-1", "--format=%s"]), "first");
    assert_eq!(
        git(&a, &["status", "--porcelain"]),
        " M src/main.rs\n?? src/new.rs"
    );
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 其他仓库标题行上的分支可以直接点：切换到那个仓库，并展开它的分支选择面板。
#[test]
fn the_branch_on_another_repository_opens_its_branch_picker() {
    let Some((mut app, a, b)) = two_repositories("git-other-branch", false) else {
        return;
    };
    git_in(&b, &["branch", "-q", "topic"]);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git.root.as_ref(), Some(&a));
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let chip = hit(&app, CommitTarget::OtherBranch(1));
    let header = hit(&app, CommitTarget::OtherRepository(1));
    assert!(chip.x > header.x && chip.right() < header.right());
    click(&mut app, (chip.x + 4.0, chip.y + 4.0));
    assert_eq!(app.git.root.as_ref(), Some(&b));
    assert!(app.picker_lists_branches());
    assert!(app.git.branches.iter().any(|branch| branch.name == "topic"));
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 截图：开着「显示全部仓库」的提交视图（`commit-all-repositories-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_all_repositories() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let name = format!("commit-all-repositories-{theme}");
        let Some((mut app, a, _)) = two_repositories(&name, dark) else {
            return;
        };
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        app.press_commit(CommitTarget::OtherCheck(1, 0));
        shoot(&mut app, &name);
        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }
}

/// 截图：暂存区视图中列出其他仓库，另一个仓库有一个已暂存的文件（`commit-staging-others-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_staging_others() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let name = format!("commit-staging-others-{theme}");
        let Some((mut app, a, _)) = two_repositories(&name, dark) else {
            return;
        };
        app.settings_toggle(Setting::GitStagingArea);
        app.run_action(Action::ShowCommit);
        app.run_commit_more_command(MenuCommand::CommitAllRepositories);
        app.poll_git();
        app.press_commit(CommitTarget::OtherStage(1, 0, true));
        app.poll_commit();
        app.poll_git();
        // 鼠标停在未暂存的那一行上：行尾的状态字母换成「＋」。
        paint(&mut app);
        let rows = app.hits.commit.iter();
        let row = rows
            .clone()
            .find(|(_, h)| *h == CommitTarget::OtherFile(1, 1));
        if let Some((row, _)) = row.copied() {
            app.on_mouse_move(row.x + 60.0, row.y + 4.0);
        }
        shoot(&mut app, &name);
        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }
}

/// 打开另一个仓库里的文件（提交视图开着时当前仓库不跟着换）：当前行追溯按文件所在的仓库跑；
/// 未跟踪的文件不跑，也不提示失败。
#[test]
fn inline_blame_follows_the_repository_of_the_file() {
    let Some((mut app, a, b)) = two_repositories("git-blame-other-repository", false) else {
        return;
    };
    // `b` 的 main.rs 恢复成提交里的样子：各行都有提交可以点。
    git_in(&b, &["checkout", "-q", "--", "src/main.rs"]);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.git_read_summaries();
    app.poll_git();
    let blamed = |app: &mut App, path: &Path| {
        app.open_path(path);
        let id = app.editor.active().unwrap().id;
        app.update_blame(id);
        app.poll_blame();
        app.blame_of(id).is_some_and(|b| !b.lines.is_empty())
    };
    assert!(blamed(&mut app, &b.join("src/main.rs")));
    assert_eq!(app.git.root.as_ref(), Some(&a));

    // 点追溯的一行：切换到文件所在的仓库，在它的日志里选中那个提交。
    let id = app.editor.active().unwrap().id;
    let blame = app.blame_of(id).unwrap();
    let line = (blame.lines.iter())
        .position(|&c| !blame.commits[c as usize].uncommitted)
        .unwrap();
    let oid = blame.commits[blame.lines[line] as usize].oid.clone();
    app.click_blame(line);
    assert_eq!(app.git.root.as_ref(), Some(&b));
    let log = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(
        log.details.as_ref().map(|d| d.oid.as_str()),
        Some(oid.as_str())
    );
    app.git_switch_repository(0);
    assert!(!blamed(&mut app, &b.join("src/new.rs")));
    assert!(app.status.message.is_none());
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}
