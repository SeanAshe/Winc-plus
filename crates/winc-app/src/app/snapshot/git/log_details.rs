//! Git 日志详情的测试：选中提交与改动文件、键盘走动、文件菜单、选中复制与滚动。

use super::*;

#[test]
fn refreshing_the_log_keeps_the_selection_and_its_details() {
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-refresh", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    app.select_log_row(4);
    let view = |app: &App| -> (Option<usize>, Option<String>, usize) {
        let v = app.editor.active().unwrap().log.as_deref().unwrap();
        let message = v.details.as_ref().and_then(|d| d.message.clone());
        (v.selected, message, v.commits.len())
    };
    // 选着的提交上下两个已经预取进了缓存：键盘走过去不用等。
    {
        let v = app.editor.active().unwrap().log.as_deref().unwrap();
        assert!(v.cached(&v.commits[3].oid).is_some());
        assert!(v.cached(&v.commits[5].oid).is_some());
    }
    // 详情做个记号：刷新后记号还在，说明没有清空重取，详情一栏不会闪烁。
    if let Some(d) = app
        .editor
        .active_mut()
        .unwrap()
        .log
        .as_deref_mut()
        .unwrap()
        .details
        .as_mut()
    {
        d.message = Some("记号".into());
    }
    app.press_log(LogTarget::Refresh);
    paint(&mut app);
    assert_eq!(view(&app), (Some(4), Some("记号".into()), 9));
    // 折好的说明行被缓存：再画一帧直接复用，不重新折行。
    let wrapped = |app: &App| app.log.wrapped.borrow().clone().unwrap();
    let first = wrapped(&app);
    assert_eq!(first.key.0, "记号");
    paint(&mut app);
    assert!(std::rc::Rc::ptr_eq(&first, &wrapped(&app)));
    assert!(!app.editor.active().unwrap().log.as_deref().unwrap().stale);

    // 换选一个详情还没取回来的提交：短时间内详情一栏继续显示上一条详情（此时点击旧提交号不生效），
    // 取回来一次换掉；超过等待时长还没返回才换成新的提交、显示「正在读取」。
    let copy = |app: &App| app.hits.log.iter().any(|(_, t)| *t == LogTarget::CopyOid);
    {
        let v = app.editor.active_mut().unwrap().log.as_deref_mut().unwrap();
        v.previous = v.details.clone();
        let oid = v.commits[1].oid.clone();
        v.selected = Some(1);
        v.details = Some(winc_editor::git::log::Details {
            oid,
            ..Default::default()
        });
    }
    app.log.waiting = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    paint(&mut app);
    assert!(!copy(&app));
    app.log.waiting = None;
    paint(&mut app);
    assert!(copy(&app));

    // 筛选时选中的提交还在结果里就继续选中该提交。
    let oid = |app: &App| {
        let v = app.editor.active().unwrap().log.as_deref().unwrap();
        v.selected_oid().map(str::to_owned)
    };
    app.select_log_row(0);
    let first = oid(&app);
    app.change_log_query(|q| q.text = "调试".into());
    assert_eq!(oid(&app), first);

    // 刚开始取详情时不写「正在读取」，超过等待时长还没返回才显示。
    app.log.waiting = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    assert!(app.log_quiet());
    app.log.waiting = Some(std::time::Instant::now());
    assert!(!app.log_quiet());
    app.log.list_waiting = None;
    assert_eq!(app.log_tick(std::time::Instant::now()), None);
    assert_eq!(app.log.waiting, None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_log_shows_who_wrote_each_commit_and_copies_its_hash() {
    let Some((mut app, root)) = git_app_in("git-log-avatar", false, false) else {
        return;
    };
    history_repo(&root, 40);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    use crate::app::git::log::LogTarget;
    // 每一行都有作者头像，悬停提示名字和邮箱。
    let (avatar, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::Avatar(0))
        .expect("第一行的头像");
    app.on_mouse_move(avatar.x + 4.0, avatar.y + avatar.h / 2.0);
    assert_eq!(app.hover, Hover::Log(LogTarget::Avatar(0)));
    assert!(app.hover.has_tooltip());
    // 点头像和点这一行一样：选中该提交。
    click(&mut app, (avatar.x + 4.0, avatar.y + avatar.h / 2.0));
    let oid = {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        assert_eq!(view.selected, Some(0));
        view.commits[0].oid.clone()
    };
    // ⌘C 复制选中提交的完整提交号。
    app.run_action(Action::Copy);
    assert_eq!(app.copied.as_deref(), Some(oid.as_str()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_log_files_can_be_walked_with_the_keyboard_and_commits_opened_by_double_click() {
    use crate::app::git::log::LogTarget;
    let Some((mut app, root)) = git_app_in("git-log-keys", false, false) else {
        return;
    };
    history_repo(&root, 40);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let key = |app: &mut App, k: NamedKey| {
        app.log_key(&Key::Named(k));
        paint(app);
    };
    let tabs = |app: &App| app.editor.documents.len();

    // ⌘ 点第一个和最后一个提交：比较，两者之间改动了多个文件。
    let last = app
        .editor
        .active()
        .unwrap()
        .log
        .as_deref()
        .unwrap()
        .commits
        .len()
        - 1;
    app.press_log(LogTarget::Row(0));
    app.modifiers = command_modifiers();
    app.press_log(LogTarget::Row(last));
    app.modifiers = ModifiersState::empty();
    paint(&mut app);
    let files = app.log_files_len();
    assert!(files >= 2, "{files}");

    // → 进到文件列表，↓ 选下一个，End 到最后，← 回到提交列表。
    key(&mut app, NamedKey::ArrowRight);
    assert_eq!(app.log.file, Some(0));
    key(&mut app, NamedKey::ArrowDown);
    assert_eq!(app.log.file, Some(1));
    key(&mut app, NamedKey::End);
    assert_eq!(app.log.file, Some(files - 1));
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(app.log.file, None);

    // 再进去，回车打开选中文件的差异。
    key(&mut app, NamedKey::ArrowRight);
    key(&mut app, NamedKey::ArrowDown);
    let before = tabs(&app);
    key(&mut app, NamedKey::Enter);
    app.poll_log();
    assert_eq!(tabs(&app), before + 1);
    assert!(app.editor.active().unwrap().diff.is_some());

    // 回到日志，双击一个提交：打开该提交改动的第一个文件（在刚才的差异标签中换内容）。
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let (row, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::Row(2))
        .unwrap();
    let at = (row.x + 200.0, row.y + row.h / 2.0);
    click(&mut app, at);
    click(&mut app, at);
    app.poll_log();
    let doc = app.editor.active().unwrap();
    assert!(doc.diff.is_some(), "双击打开了差异");
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：键盘进到详情的文件列表，选着第二个文件。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_git_log_file_keys() {
    use crate::app::git::log::LogTarget;
    let Some((mut app, root)) = git_app_in("git-log-file-keys", false, false) else {
        return;
    };
    history_repo(&root, 40);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let last = app
        .editor
        .active()
        .unwrap()
        .log
        .as_deref()
        .unwrap()
        .commits
        .len()
        - 1;
    app.press_log(LogTarget::Row(0));
    app.modifiers = command_modifiers();
    app.press_log(LogTarget::Row(last));
    app.modifiers = ModifiersState::empty();
    paint(&mut app);
    app.log_key(&Key::Named(NamedKey::ArrowRight));
    app.log_key(&Key::Named(NamedKey::ArrowDown));
    shoot(&mut app, "log-file-keys");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_in_the_log_details_has_a_menu_and_a_full_path_tip() {
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-log-file-menu", false) else {
        return;
    };
    history_repo(&root, 10);
    // 再提交一个藏在很深的目录里的文件：详情里目录写不下。
    let deep = "backend/internal/application/gateway/provider/quality_retry/selector";
    std::fs::create_dir_all(root.join(deep)).unwrap();
    std::fs::write(root.join(deep).join("retry.rs"), "fn retry() {}\n").unwrap();
    git_in(&root, &["add", "."]);
    git_in(
        &root,
        &[
            "-c",
            "user.name=张伟",
            "-c",
            "user.email=zhang@example.com",
            "commit",
            "-q",
            "-m",
            "重试",
        ],
    );
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let full = format!("{deep}/retry.rs");
    assert_eq!(app.log_file_tip(0).as_deref(), Some(full.as_str()));
    assert_eq!(app.log.long_files, [0]);
    assert!(Hover::Log(LogTarget::File(0)).has_tooltip());

    // 右键文件：菜单作用于该文件，键盘焦点也移到该文件。
    let (file, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::File(0))
        .unwrap();
    right_click(&mut app, (file.x + 40.0, file.y + file.h / 2.0));
    assert!(matches!(
        app.context_menu.as_ref().map(|m| &m.target),
        Some(MenuTarget::LogFile(0, path)) if path.ends_with("retry.rs")
    ));
    assert_eq!(app.log.file, Some(0));
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        &commands[..3],
        [
            MenuCommand::GitShowDiff,
            MenuCommand::GitOpenFile,
            MenuCommand::FileHistory
        ]
    );

    // 显示文件历史：打开只显示该文件历史的日志标签。
    app.run_menu_command(MenuCommand::FileHistory);
    let title = app.editor.active().unwrap().title.clone().unwrap();
    assert!(title.starts_with("retry.rs"), "{title}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn text_in_the_log_details_can_be_selected_and_copied() {
    let Some((mut app, root)) = git_app("git-log-select", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let row = {
        let v = app.editor.active().unwrap().log.as_deref().unwrap();
        v.commits
            .iter()
            .position(|c| c.subject == "登录：失败三次后锁定")
            .unwrap()
    };
    app.select_log_row(row);
    paint(&mut app);
    let runs = app.log.runs.clone();
    assert!(runs.len() >= 4, "说明、正文、作者、邮箱、时间都能选");
    assert_eq!(runs[0].text, "登录：失败三次后锁定");

    // 从说明的第一个字按下，拖到正文那一行的行尾：说明和正文之间换行。
    let (start, body) = (&runs[0], &runs[1]);
    let y0 = start.row.y + start.row.h / 2.0;
    app.on_mouse_move(start.x + 1.0, y0);
    assert_eq!(app.cursor_icon, CursorIcon::Text);
    app.on_mouse_down(MouseButton::Left);
    let end = *body.edges.last().unwrap();
    app.on_mouse_move(end + 30.0, body.row.y + body.row.h / 2.0);
    app.on_mouse_up(MouseButton::Left);
    let want = format!("{}\n{}", start.text, body.text);
    assert_eq!(app.log_selected_text().as_deref(), Some(want.as_str()));
    app.run_action(Action::Copy);
    assert_eq!(app.copied.as_deref(), Some(want.as_str()));
    paint(&mut app);

    // 双击一行选中整行（邮箱）；点空白处清掉，⌘C 回到复制提交号。
    let email = app
        .log
        .runs
        .iter()
        .find(|r| r.text.contains('@'))
        .cloned()
        .unwrap();
    let at = (email.x + 2.0, email.row.y + email.row.h / 2.0);
    click(&mut app, at);
    click(&mut app, at);
    assert_eq!(
        app.log_selected_text().as_deref(),
        Some(email.text.as_str())
    );
    let blank = app.log_parts().details;
    click(&mut app, (blank.right() - 4.0, blank.bottom() - 60.0));
    assert_eq!(app.log_selected_text(), None);
    app.run_action(Action::Copy);
    let oid = {
        let v = app.editor.active().unwrap().log.as_deref().unwrap();
        v.commits[row].oid.clone()
    };
    assert_eq!(app.copied.as_deref(), Some(oid.as_str()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_log_details_stop_scrolling_at_their_end() {
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta;

    let Some((mut app, root)) = git_app("git-log-details-scroll", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    // 详情放得下：往下滚多少都不动，不会滚得一栏空白。
    let details = app.log_parts().details;
    app.on_mouse_move(details.x + 40.0, details.y + 100.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -5000.0,
    )));
    paint(&mut app);
    let scroll = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .details_scroll
    };
    assert_eq!(scroll(&app), 0.0);
    assert!(!app.log.runs.is_empty());
    assert!(app.log.runs[0].row.y >= details.y);

    // 窗口矮、详情放不下：最多滚到最后一行可见为止。
    app.renderer =
        Some(Renderer::offscreen(1280.0, 260.0, SCALE, app.palette.frame_bg, None, None).unwrap());
    app.relayout();
    paint(&mut app);
    let details = app.log_parts().details;
    app.on_mouse_move(details.x + 40.0, details.y + 40.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -5000.0,
    )));
    paint(&mut app);
    assert!(scroll(&app) > 0.0);
    assert_eq!(scroll(&app), app.log.details_max);
    let last = app
        .hits
        .log
        .iter()
        .filter(|(_, t)| matches!(t, crate::app::git::log::LogTarget::File(_)))
        .map(|(r, _)| r.bottom())
        .fold(0.0, f32::max);
    assert!(last > details.y && last <= details.bottom() - 30.0 + 0.5);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_details_footer_does_not_fade_with_the_commit() {
    use crate::app::animation::{Clock, DETAILS_FADE};
    use std::time::Duration;

    let Some((mut app, root)) = git_app("git-log-footer-fade", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    // 详情一栏里的两块：最下面一行快捷键提示，和上面提交的内容。
    let details = app.log_parts().details;
    let footer = Rect::new(details.x, details.bottom() - 30.0, details.w, 30.0);
    let content = Rect::new(details.x, details.y, details.w, 120.0);
    let pixels = |app: &App, area: Rect| -> Vec<u32> {
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let w = image.width as usize;
        let (x0, x1) = ((area.x * SCALE) as usize, (area.right() * SCALE) as usize);
        let (y0, y1) = ((area.y * SCALE) as usize, (area.bottom() * SCALE) as usize);
        (y0..y1)
            .flat_map(|y| image.pixels[y * w + x0..y * w + x1].to_vec())
            .collect()
    };
    let (still_footer, still_content) = (pixels(&app, footer), pixels(&app, content));

    // 换了提交、刚开始淡入的那一帧：内容是淡的，提示一个像素都不变。
    app.log.fade = Some(Clock::frozen(Duration::ZERO, DETAILS_FADE));
    paint(&mut app);
    assert_ne!(pixels(&app, content), still_content, "内容在淡入");
    assert!(pixels(&app, footer) == still_footer, "提示不跟着淡入");
    let _ = std::fs::remove_dir_all(&root);
}

/// 回归：日志的「刷新」要一并刷新未提交的更改和分支，并且每次都能看出点过了。
#[test]
fn refreshing_the_log_updates_the_repository_state_and_reports_it() {
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-refresh-state", false) else {
        return;
    };
    history_repo(&root, 3);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let git = |args: &[&str]| {
        let run = Command::new("git").arg("-C").arg(&root).args(args).output();
        assert!(run.unwrap().status.success());
    };
    git(&["branch", "made-elsewhere"]);
    std::fs::write(root.join("made-elsewhere.txt"), "x").unwrap();
    let uncommitted = |app: &App| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        app.log_uncommitted(view)
    };
    let has_branch = |app: &App| (app.git.branches.iter()).any(|b| b.name == "made-elsewhere");
    assert_eq!(uncommitted(&app), 0);
    assert!(!has_branch(&app));

    app.status.message = None;
    app.press_log(LogTarget::Refresh);
    paint(&mut app);
    assert!(uncommitted(&app) > 0);
    assert!(has_branch(&app));
    assert_eq!(
        app.status.message.as_deref(),
        Some(winc_i18n::t!("git-log-refreshed"))
    );
    assert_eq!(app.log.refreshing, None);
}
