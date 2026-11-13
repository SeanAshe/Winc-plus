//! 提交视图中同步、横幅、撤销与修改上一次提交的测试。

use super::*;

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
    let width = app.panels.sidebar.width;
    app.panels.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    assert!(app.commit_parts().amend.is_none());
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitAmend, true)));
    app.panels.sidebar.width = width;
    paint(&mut app);

    // 回归：主按钮显示没有要提交的更改时，菜单不能再提供「提交并推送」；修改上一次提交仍然可用。
    let entries = std::mem::take(&mut app.git.status.as_mut().unwrap().entries);
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitPush, false)));
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitAmend, true)));
    app.git.commit.amend = true;
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitPush, true)));
    app.git.commit.amend = false;
    app.git.status.as_mut().unwrap().entries = entries;

    // 「修改上次提交」启用时打勾，按钮写「修改提交」。
    assert!(!app.menu_checked(MenuCommand::CommitAmend));
    app.run_commit_button_command(MenuCommand::CommitAmend);
    assert!(app.git.commit.amend);
    assert!(app.menu_checked(MenuCommand::CommitAmend));
    app.run_commit_button_command(MenuCommand::CommitAmend);
    assert!(!app.git.commit.amend);
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

    app.git.commit.message.set_text("加上改动\n\n正文");
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
    assert_eq!(app.clipboard.copied.as_deref(), Some(oid.as_str()));
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
    assert!(!app.overlays.alert.as_ref().unwrap().buttons[0].destructive);
    press_alert(&mut app, 0);
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.oid.as_deref(), Some(first.as_str()));
    assert!(!status.entries.is_empty());
    assert_eq!(app.git.commit.message.text(), "加上改动\n\n正文");

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
    app.git.branch.running = Some("获取".into());
    paint(&mut app);
    assert_eq!(sync(&app), ahead);
    app.on_mouse_move(ahead.x + 4.0, ahead.y + 4.0);
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(tip.as_deref(), Some("正在获取…"));

    // 不在分支上时只有图标，操作仍在同一个位置显示。
    app.git.branch.running = None;
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
    assert!(app.overlays.tooltip.since.is_some());
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
    app.git.branch.running = Some("签出".into());
    paint(&mut app);
    assert!(sync(&app));
    app.git.branch.running = None;

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
    app.git.commit.last = Some(winc_editor::git::LastCommit {
        oid,
        subject: "two".to_owned(),
        message: String::new(),
        time: 0,
        local: String::new(),
    });
    assert!(app.commit_undoable().is_none());
    app.git.commit.last = None;

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
    assert!(app.git.branch.finished.is_some());

    // 新操作开始时不再显示上一个的结果。
    app.git.branch.finished = Some(Clock::stopped(COMMITTED_SHOWN, Duration::from_millis(800)));
    git_in(
        &root,
        &["remote", "set-url", "origin", "/nonexistent/repo.git"],
    );
    app.press_commit(CommitTarget::Sync);
    assert!(app.git.branch.finished.is_none());
    assert!(app.git.branch.running.is_none());

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
    app.git.commit.amend = true;
    assert_eq!(app.git_sync_state(), SyncState::Hidden);
    app.git.commit.amend = false;
    std::fs::write(root.join("new.txt"), "x\n").unwrap();
    app.git_refresh();
    assert_eq!(app.git_sync_state(), SyncState::Push(1));
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
    assert!(app.overlays.alert.as_ref().unwrap().buttons[0].destructive);
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

    app.git.commit.undo.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    let parts = app.commit_parts();
    let (half, ..) = parts.undo.unwrap();
    assert!(0.0 < half.h && half.h < bar.h, "{half:?}");
    assert_eq!(half.bottom(), bar.bottom());
    assert_eq!(parts.list.bottom(), half.y);
    assert_eq!(app.commit_undo_shown().unwrap().subject, "加上改动");
    paint(&mut app);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Undo));

    app.git.commit.undo.motion = Some((1.0, Clock::frozen(TREE_ANIMATION, TREE_ANIMATION)));
    app.settle_animations();
    assert!(app.git.commit.undo.motion.is_none());
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
    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    assert!(app.commit_pushes());
    paint(&mut app);
    assert!(
        tip(&app).is_some_and(|t| t.starts_with("提交并推送 main")),
        "{:?}",
        tip(&app)
    );
    app.git.commit.amend = true;
    assert!(!app.commit_pushes());
    app.git.commit.amend = false;
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
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

    app.panels.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    let settled = app.commit_parts();
    let to = settled.commit.w + settled.commit_menu.w;
    assert!(settled.amend.is_none());
    assert!(to > from);

    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.git.commit.button_motion = Some((from, half));
    app.git.commit.amend_motion = Some((false, half));
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

    app.git.commit.button_motion = None;
    app.git.commit.amend_motion = None;
    assert_eq!(app.commit_parts(), settled);
    let _ = std::fs::remove_dir_all(&root);
}
