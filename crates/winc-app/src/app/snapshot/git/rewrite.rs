//! 日志里提交菜单上改写历史的操作的测试：编辑提交说明、删除提交、撤销提交、推送此前所有提交。

use crate::app::git::form::FormKind;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::layout::DialogPart;

use super::rebase::{four_commits, rev, subjects};
use super::*;

/// 打开日志，在提交 `oid` 上弹出右键菜单，返回菜单里的命令及是否可用。
fn commit_menu(app: &mut App, oid: &str) -> Vec<(MenuCommand, bool)> {
    if app.editor.active().and_then(|d| d.log.as_deref()).is_none() {
        app.run_action(Action::GitShowLog);
    }
    paint(app);
    app.open_context_menu(MenuTarget::LogCommit(oid.to_owned()), (0.0, 0.0));
    menu_commands(app)
}

/// 编辑提交说明的对话框里信息框的内容；不是这个对话框时 panic。
fn reword_text(app: &App) -> String {
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    match &dialog.kind {
        FormKind::Rebase(form) => assert!(form.reword),
        _ => panic!("应当是编辑提交说明的对话框"),
    }
    dialog.name.text().to_owned()
}

/// 编辑 `HEAD` 的说明只改信息：暂存区里的更改留着；说明没变时不能保存。
#[test]
fn editing_the_head_message_keeps_staged_changes() {
    let Some((mut app, dir)) = four_commits("rewrite-reword-head") else {
        return;
    };
    std::fs::write(dir.join("a.txt"), "staged").unwrap();
    git_in(&dir, &["add", "a.txt"]);
    let head = rev(&dir, "HEAD");
    commit_menu(&mut app, &head);
    app.run_menu_command(MenuCommand::LogReword);
    paint(&mut app);
    assert_eq!(reword_text(&app), "d");
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.focus, DialogPart::Branch);
    assert!(!app.form_plan(dialog).enabled);

    let field = &mut app.overlays.form_dialog.as_mut().unwrap().name;
    field.insert(" 改");
    // 回车换行，⌘回车保存。
    assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", false));
    app.overlays
        .form_dialog
        .as_mut()
        .unwrap()
        .name
        .insert("\n正文");
    assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", true));
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(subjects(&dir), ["d 改", "c", "b", "a"]);
    let staged = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["diff", "--cached", "--name-only"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&staged.stdout).trim(), "a.txt");
}

/// 编辑更早的提交的说明：之后的提交重新应用，只有这一个的说明变了。
#[test]
fn editing_an_older_message_rebases_the_commits_after_it() {
    let Some((mut app, dir)) = four_commits("rewrite-reword-old") else {
        return;
    };
    commit_menu(&mut app, &rev(&dir, "HEAD~2"));
    app.run_menu_command(MenuCommand::LogReword);
    paint(&mut app);
    assert_eq!(reword_text(&app), "b");
    let field = &mut app.overlays.form_dialog.as_mut().unwrap().name;
    field.set_text("b 新的说明");
    app.submit_form_dialog();
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(subjects(&dir), ["d", "c", "b 新的说明", "a"]);
}

/// 删除提交：确认后去掉它，之后的提交保留；仓库里唯一的提交不能删。
#[test]
fn dropping_a_commit_keeps_the_ones_after_it() {
    let Some((mut app, dir)) = four_commits("rewrite-drop") else {
        return;
    };
    let c = rev(&dir, "HEAD~1");
    commit_menu(&mut app, &c);
    app.run_menu_command(MenuCommand::LogDropCommit);
    paint(&mut app);
    assert_eq!(subjects(&dir), ["d", "b", "a"]);
    assert!(!dir.join("c.txt").exists() && dir.join("d.txt").exists());
}

/// 删除提交的确认写明之后有几个提交重新应用，已推送时说明要强制推送。
#[test]
fn the_drop_prompt_counts_later_and_pushed_commits() {
    use winc_editor::git::{RebaseCommit, RebasePlan};
    let commit = |subject: &str| RebaseCommit {
        oid: format!("{subject}000000"),
        subject: subject.to_owned(),
        message: subject.to_owned(),
    };
    let mut plan = RebasePlan {
        base: Some("a".into()),
        commits: vec![commit("b"), commit("c"), commit("d")],
        pushed: 0,
    };
    let (message, detail, _) = crate::app::git::rewrite::drop_prompt(&plan);
    assert!(message.contains('b'), "{message}");
    assert!(
        detail.contains('2') && !detail.contains("强制推送"),
        "{detail}"
    );
    plan.pushed = 3;
    let (_, detail, _) = crate::app::git::rewrite::drop_prompt(&plan);
    assert!(detail.contains("强制推送"), "{detail}");
}

/// 撤销提交只对可以撤销的上一次提交可用：分支退回上一个提交，更改留在暂存区。
#[test]
fn undo_commit_is_offered_on_head_only() {
    let Some((mut app, dir)) = four_commits("rewrite-undo") else {
        return;
    };
    app.git_refresh();
    paint(&mut app);
    let older = commit_menu(&mut app, &rev(&dir, "HEAD~1"));
    assert!(older.contains(&(MenuCommand::LogUndoCommit, false)));
    app.overlays.context_menu = None;
    let head = commit_menu(&mut app, &rev(&dir, "HEAD"));
    assert!(head.contains(&(MenuCommand::LogUndoCommit, true)));
    app.run_menu_command(MenuCommand::LogUndoCommit);
    press_alert(&mut app, 0);
    paint(&mut app);
    assert_eq!(subjects(&dir), ["c", "b", "a"]);
    assert!(dir.join("d.txt").exists());
}

/// 推送此前所有提交：推送对话框列出到这个提交为止的提交，推送后远程分支停在这个提交上。
#[test]
fn pushing_up_to_a_commit_leaves_later_commits_local() {
    let Some((mut app, dir)) = four_commits("rewrite-push") else {
        return;
    };
    let remote = search_folder("rewrite-push-remote");
    git_in(&remote, &["init", "-q", "--bare"]);
    git_in(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git_in(&dir, &["push", "-q", "origin", "HEAD~3:refs/heads/main"]);
    git_in(&dir, &["fetch", "-q", "origin"]);
    git_in(&dir, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    paint(&mut app);

    let c = rev(&dir, "HEAD~1");
    let menu = commit_menu(&mut app, &c);
    assert!(menu.contains(&(MenuCommand::LogPushUpTo, true)));
    app.run_menu_command(MenuCommand::LogPushUpTo);
    paint(&mut app);
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    let FormKind::Transfer(form) = &dialog.kind else {
        panic!("应当是推送对话框");
    };
    assert_eq!(form.upto.as_deref(), Some(c.as_str()));
    assert_eq!(form.subjects, ["c", "b"]);
    assert_eq!(form.total, 2);
    app.submit_form_dialog();
    app.poll_branch_ops();
    assert_eq!(rev(&remote, "main"), c);
    assert_eq!(rev(&dir, "HEAD"), rev(&dir, "main"));

    // 已经推送过的提交：没有会送出的提交，不能推送，也没有「不再询问」。
    git_in(&dir, &["fetch", "-q", "origin"]);
    app.git_refresh();
    app.start_push_up_to(c);
    paint(&mut app);
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert!(!app.form_plan(dialog).enabled);
    let options = app.dialog_layout().options;
    assert!(
        !options
            .iter()
            .any(|(part, _)| *part == DialogPart::NewWindow)
    );
}

/// 日志里按 F2 编辑选中提交的说明，菜单里的「编辑提交说明…」旁写着 F2；不在日志里时 F2
/// 不做这件事。
#[test]
fn f2_edits_the_selected_commit_message() {
    use crate::keymap::{self, Context};
    let Some((mut app, dir)) = four_commits("rewrite-f2") else {
        return;
    };
    let f2 = keymap::single_stroke(Action::GitRewordCommit, Context::Editor);
    assert!(f2.is_some());
    assert_eq!(MenuCommand::LogReword.shortcut(Context::Editor), f2);
    assert!(!app.run_action(Action::GitRewordCommit));

    commit_menu(&mut app, &rev(&dir, "HEAD~1"));
    app.overlays.context_menu = None;
    app.select_log_row(1);
    assert!(app.on_shortcut(f2.unwrap()));
    paint(&mut app);
    assert_eq!(reword_text(&app), "c");
}

/// 显示全部分支时，别的分支上的提交不能改写、推送，当前分支上的提交不用拣选；新工作树的
/// 位置写在悬停提示里，不占第二行。
#[test]
fn the_commit_menu_follows_where_the_commit_is() {
    let Some((mut app, dir)) = four_commits("rewrite-reach") else {
        return;
    };
    git_in(&dir, &["switch", "-q", "-c", "side", "HEAD~1"]);
    std::fs::write(dir.join("s.txt"), "s").unwrap();
    git_in(&dir, &["add", "s.txt"]);
    git_in(&dir, &["commit", "-q", "-m", "s"]);
    let side = rev(&dir, "HEAD");
    git_in(&dir, &["switch", "-q", "main"]);
    app.git_refresh();
    assert!(show_all_log(&mut app));
    paint(&mut app);

    let menu = commit_menu(&mut app, &side);
    for command in [
        MenuCommand::LogReword,
        MenuCommand::LogDropCommit,
        MenuCommand::LogInteractiveRebase,
    ] {
        assert!(menu.contains(&(command, false)), "{command:?}");
    }
    assert!(menu.contains(&(MenuCommand::LogCherryPick, true)));
    app.overlays.context_menu = None;

    let menu = commit_menu(&mut app, &rev(&dir, "HEAD~1"));
    assert!(menu.contains(&(MenuCommand::LogReword, true)));
    assert!(menu.contains(&(MenuCommand::LogCherryPick, false)));
    let label = app.menu_label(MenuCommand::LogNewWorktree);
    assert_eq!(label.subtitle, None);
    assert!(label.tooltip.is_some_and(|t| t.contains(".worktrees")));
}

/// 修正到此提交：提交卡片写好 `fixup! 说明`，按钮是「提交并变基」；提交以后更改并入目标，
/// 说明不变，之后的提交还在。
#[test]
fn fixup_folds_the_changes_into_the_commit() {
    use crate::app::git::commit::CommitTarget;
    let Some((mut app, dir)) = four_commits("rewrite-fixup") else {
        return;
    };
    std::fs::write(dir.join("b.txt"), "b2").unwrap();
    app.git_refresh();
    paint(&mut app);
    let b = rev(&dir, "HEAD~2");
    let menu = commit_menu(&mut app, &b);
    assert!(menu.contains(&(MenuCommand::LogFixup, true)));
    app.run_menu_command(MenuCommand::LogFixup);
    paint(&mut app);
    assert_eq!(app.git.commit.message.text(), "fixup! b");
    assert_eq!(app.focus, Focus::CommitMessage);

    // 取消勾选那一行：恢复原来写的说明；再来一次后提交。
    app.press_commit(CommitTarget::Amend);
    assert!(app.git.commit.into.is_none());
    assert_eq!(app.git.commit.message.text(), "");
    app.overlays.context_menu = None;
    commit_menu(&mut app, &b);
    app.run_menu_command(MenuCommand::LogFixup);
    paint(&mut app);
    app.git_commit();
    app.poll_commit();
    app.poll_branch_ops();
    assert!(app.git.commit.into.is_none());
    assert_eq!(subjects(&dir), ["d", "c", "b", "a"]);
    let shown = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["show", "HEAD~2:b.txt"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&shown.stdout), "b2");
}

/// 压缩到此提交：补充的说明写在 `squash!` 下面，提交以后两段说明都留在目标里；工作区干净时
/// 两项都不能用。
#[test]
fn squash_into_keeps_both_messages() {
    let Some((mut app, dir)) = four_commits("rewrite-squash") else {
        return;
    };
    let menu = commit_menu(&mut app, &rev(&dir, "HEAD~1"));
    assert!(menu.contains(&(MenuCommand::LogSquashInto, false)));
    app.overlays.context_menu = None;
    std::fs::write(dir.join("c.txt"), "c2").unwrap();
    app.git_refresh();
    paint(&mut app);
    commit_menu(&mut app, &rev(&dir, "HEAD~1"));
    app.run_menu_command(MenuCommand::LogSquashInto);
    paint(&mut app);
    assert_eq!(app.git.commit.message.text(), "squash! c\n\n");
    app.git.commit.message.insert("补充说明");
    app.git_commit();
    app.poll_commit();
    app.poll_branch_ops();
    assert_eq!(subjects(&dir), ["d", "c", "b", "a"]);
    let message = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["log", "-1", "--format=%B", "HEAD~1"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&message.stdout).trim_end(),
        "c\n\n补充说明"
    );
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_history_edits() {
    let Some((mut app, dir)) = four_commits("rewrite-shot") else {
        return;
    };
    let remote = search_folder("rewrite-shot-remote");
    git_in(&remote, &["init", "-q", "--bare"]);
    git_in(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git_in(&dir, &["push", "-q", "origin", "HEAD~2:refs/heads/main"]);
    git_in(&dir, &["fetch", "-q", "origin"]);
    git_in(&dir, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    commit_menu(&mut app, &rev(&dir, "HEAD"));
    app.overlays.context_menu = None;
    let row = app.log_parts().list;
    let at = (row.x + 200.0, row.y + 12.0);
    app.open_context_menu(MenuTarget::LogCommit(rev(&dir, "HEAD")), at);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-history-menu");
    app.overlays.context_menu = None;

    app.start_reword(rev(&dir, "HEAD~2"));
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-reword");
    app.close_form_dialog();

    app.start_push_up_to(rev(&dir, "HEAD~1"));
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-push-up-to");
    app.close_form_dialog();

    app.run_action(Action::GitGoToRevision);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-compare-revision");
    app.close_form_dialog();

    app.start_interactive_rebase(rev(&dir, "HEAD~2"));
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-rebase-ref");
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_fixup_card() {
    let Some((mut app, dir)) = four_commits("rewrite-fixup-shot") else {
        return;
    };
    git_in(&dir, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    std::fs::write(dir.join("b.txt"), "b2").unwrap();
    app.git_refresh();
    commit_menu(&mut app, &rev(&dir, "HEAD~2"));
    app.run_menu_command(MenuCommand::LogFixup);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(600));
    shoot(&mut app, "git-fixup-card");
}
