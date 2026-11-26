//! 提交视图中提交历史一组的测试。

use super::*;
use crate::app::HistoryTarget;
use winc_editor::git::recent::Side;

/// 在 `root` 中建立一段历史：一个合并进来的分支、一个标签；上游 `origin/main` 停在两个提交
/// 之前，另有一个只在上游的提交。之后的两个本地提交尚未推送。
fn history_repo(root: &Path) {
    git_in(root, &["add", "-A"]);
    git_in(root, &["commit", "-q", "-m", "Add entry point"]);
    git_in(root, &["switch", "-q", "-c", "feature"]);
    std::fs::write(root.join("src/new.rs"), "pub fn parse() {}\n").unwrap();
    git_in(root, &["commit", "-q", "-am", "Parse the config file"]);
    git_in(root, &["switch", "-q", "main"]);
    std::fs::write(root.join("README.md"), "# Demo\n").unwrap();
    git_in(root, &["add", "-A"]);
    git_in(root, &["commit", "-q", "-m", "Write the readme"]);
    git_in(
        root,
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature",
            "-m",
            "Merge branch 'feature'",
        ],
    );
    git_in(root, &["branch", "-q", "-D", "feature"]);
    git_in(root, &["tag", "v0.1"]);
    // 上游：在合并提交之上多一个提交。
    git_in(root, &["switch", "-q", "-c", "upstream"]);
    git_in(
        root,
        &["commit", "-q", "--allow-empty", "-m", "Fix a typo upstream"],
    );
    git_in(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/demo.git",
        ],
    );
    git_in(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(root, &["switch", "-q", "main"]);
    git_in(root, &["branch", "-q", "-D", "upstream"]);
    git_in(root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { run(); }\n").unwrap();
    git_in(root, &["commit", "-q", "-am", "Run the app on start"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { run(); log(); }\n").unwrap();
    git_in(root, &["commit", "-q", "-am", "Log startup"]);
}

/// 检查不排出各行的行数与排出的各行一致。
fn check_line_count(app: &App) {
    assert_eq!(app.history_line_count(), app.history_lines().len());
}

/// 返回提交历史中说明为 `subject` 的提交的下标和位置。
fn find(app: &App, subject: &str) -> (usize, Side) {
    let recent = app.git.commit.history.recent.as_ref().unwrap();
    let i = (recent.commits.iter()).position(|c| c.subject == subject);
    let i = i.unwrap_or_else(|| panic!("{subject}"));
    (i, recent.sides[i])
}

/// 提交历史在改动列表下面，默认展开；列出当前分支和上游的提交，尚未推送、尚未拉取的各有
/// 区分。点击一个提交在这一行下面列出它的文件并预览第一个，再点击收起；右键点在提交上弹出
/// 提交菜单。
#[test]
fn history_lists_the_branch_and_its_upstream_below_the_changes() {
    let Some((mut app, root)) = git_app("git-commit-history", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    history_repo(&root);
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    paint(&mut app);

    assert_eq!(find(&app, "Log startup").1, Side::Outgoing);
    assert_eq!(find(&app, "Run the app on start").1, Side::Outgoing);
    assert_eq!(find(&app, "Fix a typo upstream").1, Side::Incoming);
    assert_eq!(find(&app, "Merge branch 'feature'").1, Side::Both);
    assert_eq!(find(&app, "first").1, Side::Both);
    let parts = app.commit_parts();
    let history = parts.history.unwrap();
    assert_eq!(parts.list.bottom(), history.y);
    assert!(history.bottom() <= parts.card.unwrap().y);
    let (i, _) = find(&app, "Write the readme");
    let target = CommitTarget::History(HistoryTarget::Commit(i));
    let (row, _) = *(app.hits.commit.iter())
        .find(|(_, t)| *t == target)
        .unwrap();
    let menu = app.history_context_target(row.x + 40.0, row.y + row.h / 2.0);
    assert_eq!(
        menu.as_deref(),
        Some(app.git.commit.history.commit(i).unwrap().0.oid.as_str())
    );

    // 单击：文件列在这一行下面，预览标签里是第一个文件。
    app.pointer.last_click = None;
    app.press_commit(target);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(doc.diff.is_some());
    assert_eq!(app.git.commit.peek_preview, Some(doc.id));
    check_line_count(&app);
    let lines = app.history_lines();
    let at = lines
        .iter()
        .position(|l| *l == HistoryLine::Commit(i))
        .unwrap();
    assert_eq!(lines[at + 1], HistoryLine::File(0));
    let file = CommitTarget::History(HistoryTarget::File(0));
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == file));

    // 再点击同一个提交：文件收起，预览标签保留。
    app.pointer.last_click = None;
    app.press_commit(target);
    paint(&mut app);
    assert!(!app.history_lines().contains(&HistoryLine::File(0)));
    check_line_count(&app);

    // 后台重新读取到新的结果、还没重新绘制时，绘制时记下的下标已对不上：点击和右键不处理。
    app.git.commit.history.outdate();
    app.track_history();
    let state = (app.git.commit.peek_request, app.git.commit.peek_folded);
    app.pointer.last_click = None;
    app.press_commit(target);
    assert_eq!(
        (app.git.commit.peek_request, app.git.commit.peek_folded),
        state
    );
    assert!(
        app.history_context_target(row.x + 40.0, row.y + row.h / 2.0)
            .is_none()
    );
    paint(&mut app);

    // 双击：在日志标签中选中该提交。第一次点击的查看此后才到达时不再换下日志标签。
    let oid = app.git.commit.history.commit(i).unwrap().0.oid.clone();
    app.pointer.last_click = None;
    app.on_mouse_move(row.x + 40.0, row.y + row.h / 2.0);
    app.press_commit(target);
    let files = app.git.commit.peek.as_ref().unwrap().files.clone();
    let late = app.git.commit.peek_request;
    app.press_commit(target);
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.selected_oid(), Some(oid.as_str()));
    app.peek_files_arrived(late, oid.clone(), "late".into(), Ok(files));
    assert!(app.editor.active().unwrap().log.is_some());
    app.run_action(Action::ShowCommit);
    paint(&mut app);

    // 悬停提交：提示完整的说明，以及作者、时间和短提交号。
    let tip = app.history_tip(i).unwrap();
    let (subject, detail) = tip.split_once('\n').unwrap();
    assert_eq!(subject, "Write the readme");
    assert!(
        detail.starts_with("t · ") && detail.ends_with(&oid[..7]),
        "{detail}"
    );
    assert!(Hover::Commit(target).has_tooltip());

    // 悬停标题时右端是「在日志中显示」。
    let title = CommitTarget::History(HistoryTarget::Title);
    let (head, _) = *(app.hits.commit.iter()).find(|(_, t)| *t == title).unwrap();
    app.on_mouse_move(head.x + 20.0, head.y + head.h / 2.0);
    paint(&mut app);
    let log = CommitTarget::History(HistoryTarget::Log);
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == log));
    app.press_commit(log);
    assert!(app.editor.active().unwrap().log.is_some());
    app.run_action(Action::ShowCommit);
    paint(&mut app);

    // 收起整组：只剩标题一行。
    app.press_commit(CommitTarget::History(HistoryTarget::Title));
    app.settle_animations();
    paint(&mut app);
    let closed = app.commit_parts().history.unwrap();
    assert_eq!(closed.h, crate::app::git::commit::DRAWER_CLOSED);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == target));
    let _ = std::fs::remove_dir_all(&root);
}

/// 右键点在提交历史的提交上弹出提交菜单：第一项是「在日志中显示」，没有依赖日志标签的比较和
/// 转到；尚未拉取的提交不能改写。命令作用于提交视图所在的仓库。
#[test]
fn history_commits_offer_the_commit_menu() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

    let Some((mut app, root)) = git_app("git-commit-history-menu", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    history_repo(&root);
    git_in(
        &root,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Tidy up\n\nWith a body.",
        ],
    );
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    paint(&mut app);
    let right_click_on = |app: &mut App, subject: &str| {
        let (i, _) = find(app, subject);
        let target = CommitTarget::History(HistoryTarget::Commit(i));
        let (row, _) = *(app.hits.commit.iter())
            .find(|(_, t)| *t == target)
            .unwrap();
        right_click(app, (row.x + 60.0, row.y + row.h / 2.0));
        app.git.commit.history.commit(i).unwrap().0.oid.clone()
    };

    let oid = right_click_on(&mut app, "Run the app on start");
    let menu = app.overlays.context_menu.as_ref().unwrap();
    assert_eq!(menu.target, MenuTarget::HistoryCommit(oid.clone()));
    assert_eq!(
        menu.entries[0],
        MenuEntry::Item(MenuCommand::LogShowInLog, true)
    );
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::LogReword, true)));
    assert!(commands.contains(&(MenuCommand::LogPushUpTo, true)));
    for absent in [MenuCommand::LogGoTo, MenuCommand::LogCompareRevision] {
        assert!(!commands.iter().any(|(c, _)| *c == absent), "{absent:?}");
    }
    app.run_menu_command(MenuCommand::LogCopyShortOid);
    assert_eq!(app.clipboard.copied.as_deref(), Some(&oid[..7]));

    // 完整的说明在后台读取后复制。
    right_click_on(&mut app, "Tidy up");
    app.run_menu_command(MenuCommand::LogCopyMessage);
    assert_eq!(
        app.clipboard.copied.as_deref(),
        Some("Tidy up\n\nWith a body.")
    );

    // 尚未拉取的提交不在当前分支上：不能改写。
    right_click_on(&mut app, "Fix a typo upstream");
    assert!(menu_commands(&app).contains(&(MenuCommand::LogReword, false)));

    // 「在日志中显示」：打开日志标签并选中该提交。
    right_click_on(&mut app, "Run the app on start");
    app.run_menu_command(MenuCommand::LogShowInLog);
    paint(&mut app);
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.selected_oid(), Some(oid.as_str()));
    let _ = std::fs::remove_dir_all(&root);
}

/// 行数多于露出的行数时在组内滚动，滚轮落在这一组上时只滚动这一组；超过读取上限时最后一行在
/// 日志中查看更早的提交。
#[test]
fn history_scrolls_within_its_group() {
    use crate::app::scroll::ScrollTarget;
    use winc_editor::git::recent::RECENT_COMMITS;

    let Some((mut app, root)) = git_app("git-commit-history-scroll", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    for i in 0..RECENT_COMMITS {
        git_in(
            &root,
            &["commit", "-q", "--allow-empty", "-m", &format!("step {i}")],
        );
    }
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let lines = app.history_lines();
    assert_eq!(lines.len(), RECENT_COMMITS + 1);
    assert_eq!(lines.last(), Some(&HistoryLine::More));
    check_line_count(&app);
    let rows = app.history_rows_area().unwrap();
    let shown = (rows.h / COMMIT_ROW) as usize;
    assert!(shown < RECENT_COMMITS);
    let max = app.history_max_scroll();
    assert_eq!(max, (RECENT_COMMITS + 1 - shown) as f32 * COMMIT_ROW);

    let at = (rows.x + 40.0, rows.y + 10.0);
    let target = app.scroll_target_at(at.0, at.1);
    assert_eq!(target, ScrollTarget::CommitHistory);
    let list = app.git.commit.scroll;
    app.scroll_target_by(target, at, false, (0.0, -3.0 * COMMIT_ROW));
    assert_eq!(app.git.commit.history.scroll, 3.0 * COMMIT_ROW);
    assert_eq!(app.git.commit.scroll, list);
    app.scroll_target_by(target, at, false, (0.0, -1e6));
    assert_eq!(app.git.commit.history.scroll, max);

    // 各行的右端是滚动条：拖到顶部再拖到底部，不打开提交。
    use crate::layout::Scrollbar;
    paint(&mut app);
    let x = rows.right() - 3.0;
    app.on_mouse_move(x, rows.bottom() - 10.0);
    assert_eq!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::CommitHistory)
    );
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, rows.y - 40.0);
    assert_eq!(app.git.commit.history.scroll, 0.0);
    app.on_mouse_move(x, rows.bottom() + 40.0);
    assert_eq!(app.git.commit.history.scroll, max);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.git.commit.peek.is_none());
    paint(&mut app);
    let more = CommitTarget::History(HistoryTarget::More);
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == more));
    // 在日志中查看更早的提交：选中列出的最后一个提交，从该提交起继续查看。
    app.press_commit(more);
    paint(&mut app);
    let last = app
        .git
        .commit
        .history
        .recent
        .as_ref()
        .unwrap()
        .commits
        .last()
        .unwrap();
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.selected_oid(), Some(last.oid.as_str()));
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交历史默认关闭：设置中打开以后才读取并显示，关掉以后随之消失。
#[test]
fn history_shows_only_when_enabled_in_settings() {
    let Some((mut app, root)) = git_app("git-commit-history-setting", false) else {
        return;
    };
    history_repo(&root);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.git.commit.history.recent.is_none());
    assert!(app.commit_parts().history.is_none());
    app.settings_toggle(Setting::GitCommitHistory);
    assert_eq!(app.config.git_commit_history, Some(true));
    paint(&mut app);
    assert!(app.git.commit.history.recent.is_some());
    assert!(app.commit_parts().history.is_some());
    app.settings_toggle(Setting::GitCommitHistory);
    paint(&mut app);
    assert!(app.commit_parts().history.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 收起、展开像抽屉一样过渡：途中高度在两端之间、各行不响应点击；过渡结束后清除，各行恢复
/// 可点击。收起时组内的滚动位置不变，再展开时停在原处。
#[test]
fn the_history_group_opens_and_closes_like_a_drawer() {
    use crate::app::animation::{Clock, TREE_ANIMATION};

    let Some((mut app, root)) = git_app("git-commit-history-drawer", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    history_repo(&root);
    for i in 0..10 {
        git_in(
            &root,
            &["commit", "-q", "--allow-empty", "-m", &format!("step {i}")],
        );
    }
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.git.commit.history.scroll = 2.0 * COMMIT_ROW;
    let open = app.commit_parts().history.unwrap();
    app.press_commit(CommitTarget::History(HistoryTarget::Title));
    let closed = app.commit_parts().history.unwrap();
    assert!(closed.h < open.h);
    paint(&mut app);
    assert_eq!(app.git.commit.history.scroll, 2.0 * COMMIT_ROW);

    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.git.commit.history.drawer.motion = Some((closed.h, half));
    app.press_commit(CommitTarget::History(HistoryTarget::Title));
    app.git.commit.history.drawer.motion = Some((closed.h, half));
    let middle = app.commit_parts().history.unwrap();
    assert!(closed.h < middle.h && middle.h < open.h, "{middle:?}");
    assert!(app.animating());
    paint(&mut app);
    let row = |app: &App| {
        (app.hits.commit.iter())
            .any(|(_, t)| matches!(t, CommitTarget::History(HistoryTarget::Commit(_))))
    };
    assert!(!row(&app));

    let done = Clock::frozen(TREE_ANIMATION, TREE_ANIMATION);
    app.git.commit.history.drawer.motion = Some((closed.h, done));
    app.settle_animations();
    assert!(app.git.commit.history.drawer.motion.is_none());
    paint(&mut app);
    assert!(row(&app));
    assert_eq!(app.git.commit.history.scroll, 2.0 * COMMIT_ROW);
    let _ = std::fs::remove_dir_all(&root);
}

/// 侧栏较矮（下方是很高的 Git 面板、改动很多）时，提交历史让出高度：改动列表至少留四行，
/// 组内露出的行变少、可以滚动，整组不越过列表伸到标题行上。
#[test]
fn history_yields_to_the_changes_in_a_short_sidebar() {
    use crate::app::git::commit::HISTORY_LIST_KEEP;

    let Some((mut app, root)) = git_app("git-commit-history-short", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    history_repo(&root);
    for i in 0..40 {
        std::fs::write(root.join(format!("note-{i}.txt")), "x\n").unwrap();
    }
    app.config.git_log_placement = Some(winc_config::LogPlacement::Panel);
    app.git.panel.docked = true;
    app.git.panel.height = Some(520.0);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    paint(&mut app);
    assert!(app.git_panel_shown());

    // 很矮时连标题都放不下：不显示，不压到列表和标题行上。
    let parts = app.commit_parts();
    assert!(parts.history.is_none());
    assert!(parts.list.y >= parts.branch.bottom());

    app.git.panel.height = Some(380.0);
    app.relayout();
    paint(&mut app);
    let parts = app.commit_parts();
    let history = parts.history.unwrap();
    assert!(parts.list.h >= HISTORY_LIST_KEEP, "{:?}", parts.list);
    assert_eq!(parts.list.bottom(), history.y);
    // 露出的行少于八行，组内可以滚到最后一行。
    let rows = app.history_rows_area().unwrap();
    assert!(rows.h < 8.0 * COMMIT_ROW);
    let lines = app.history_line_count() as f32;
    assert_eq!(app.history_max_scroll(), lines * COMMIT_ROW - rows.h);
    let _ = std::fs::remove_dir_all(&root);
}

/// 新的提交和移动的引用使提交历史重新读取；换了仓库或仓库还没有提交时不显示这一组。
#[test]
fn history_follows_new_commits_and_hides_without_any() {
    let Some((mut app, root)) = git_app("git-commit-history-follow", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let count = |app: &App| (app.git.commit.history.recent.as_ref()).map_or(0, |r| r.commits.len());
    assert_eq!(count(&app), 1);
    assert!(
        app.git
            .commit
            .history
            .recent
            .as_ref()
            .unwrap()
            .sides
            .iter()
            .all(|s| *s == Side::Both)
    );

    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "second"]);
    app.git_refresh();
    paint(&mut app);
    assert_eq!(count(&app), 2);
    // 只移动标签也重新读取：提交上的徽章随之更新。
    git_in(&root, &["tag", "v1"]);
    app.git_refresh();
    paint(&mut app);
    let recent = app.git.commit.history.recent.as_ref().unwrap();
    assert!(
        recent.commits[0]
            .refs
            .contains(&winc_editor::git::Ref::Tag("v1".into()))
    );

    let empty = search_folder("git-commit-history-empty");
    git_in(&empty, &["init", "-q", "-b", "main"]);
    // 换到另一个仓库：此前仓库的提交立即不再显示，收起的状态保留。
    app.press_commit(CommitTarget::History(HistoryTarget::Title));
    app.git_set_root(Some(empty.clone()));
    app.track_history();
    assert!(app.git.commit.history.recent.is_none());
    assert!(!app.git.commit.history.drawer.open);
    let mut app = app_with(&empty, false);
    app.settings_toggle(Setting::GitModule);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.git.status.is_some());
    assert!(app.git.commit.history.recent.is_none());
    assert!(app.commit_parts().history.is_none());
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&empty);
}

/// 截图：提交历史一组（`commit-history`、`commit-history-dark`），鼠标停在一个提交上、提示完整的
/// 说明；悬停标题时右端的「在日志中显示」（`commit-history-title`）；滚动到中间时上下两端渐隐、
/// 右边是滚动条（`commit-history-scrolled`）；点开一个提交，它的文件列在下面
/// （`commit-history-files`）。
///
/// 运行：`WINC_SNAPSHOT_DIR=… cargo test -p winc-app shoot_commit_history -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_history() {
    let rested = || {
        Some(
            std::time::Instant::now()
                - crate::theme::TOOLTIP_DELAY
                - crate::app::animation::TOOLTIP_FADE,
        )
    };
    for dark in [false, true] {
        let name = if dark {
            "commit-history-dark"
        } else {
            "commit-history"
        };
        let Some((mut app, root)) = git_app_in(name, false, dark) else {
            return;
        };
        app.config.git_commit_history = Some(true);
        for subject in ["Set up the build", "Add the license"] {
            git_in(&root, &["commit", "-q", "--allow-empty", "-m", subject]);
        }
        history_repo(&root);
        std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
        app.git_refresh();
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        paint(&mut app);
        let (i, _) = find(&app, "Write the readme");
        let target = CommitTarget::History(HistoryTarget::Commit(i));
        let (row, _) = *(app.hits.commit.iter())
            .find(|(_, t)| *t == target)
            .unwrap();
        app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
        app.overlays.tooltip.since = rested();
        shoot(&mut app, name);
        if dark {
            let _ = std::fs::remove_dir_all(&root);
            continue;
        }
        let title = CommitTarget::History(HistoryTarget::Title);
        let (head, _) = *(app.hits.commit.iter()).find(|(_, t)| *t == title).unwrap();
        app.on_mouse_move(head.x + 20.0, head.y + head.h / 2.0);
        paint(&mut app);
        let log = CommitTarget::History(HistoryTarget::Log);
        let (button, _) = *(app.hits.commit.iter()).find(|(_, t)| *t == log).unwrap();
        app.on_mouse_move(button.x + 4.0, button.y + 4.0);
        app.overlays.tooltip.since = rested();
        shoot(&mut app, "commit-history-title");
        app.on_mouse_move(0.0, 0.0);
        app.git.commit.history.scroll = 1.5 * COMMIT_ROW;
        shoot(&mut app, "commit-history-scrolled");
        app.git.commit.history.scroll = 0.0;
        paint(&mut app);
        app.pointer.last_click = None;
        app.press_commit(target);
        shoot(&mut app, "commit-history-files");
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 截图：侧栏较矮（下方是 Git 面板、改动很多）时的提交历史（`commit-history-short`）：改动列表
/// 留出四行以上，提交历史露出的行变少。
///
/// 运行：`WINC_SNAPSHOT_DIR=… cargo test -p winc-app shoot_commit_history_short -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_history_short() {
    let Some((mut app, root)) = git_app("commit-history-short", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    history_repo(&root);
    for i in 0..40 {
        std::fs::write(root.join(format!("note-{i}.txt")), "x\n").unwrap();
    }
    app.config.git_log_placement = Some(winc_config::LogPlacement::Panel);
    app.git.panel.docked = true;
    app.git.panel.height = Some(360.0);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    shoot(&mut app, "commit-history-short");
    let _ = std::fs::remove_dir_all(&root);
}
