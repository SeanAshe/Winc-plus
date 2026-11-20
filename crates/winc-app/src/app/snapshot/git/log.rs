//! Git 日志标签的测试：提交图、长说明的悬停、筛选、提交菜单、比较、标签与点选提交图。

use super::*;

mod measure;
mod menu;

/// 在列表中无法完整显示的长说明，用于检查截短与悬停提示。
const LONG_SUBJECT: &str = "重构：把登录、注册、找回密码三处各写一遍的表单校验收拢到一个模块里，错误提示统一走本地化表，顺带删掉早就不用的旧接口和它们的测试";

/// 在 `history_repo` 上追加一次使用 [`LONG_SUBJECT`] 的提交，返回打开日志后该提交所在的行。
fn long_subject_row(app: &mut App, root: &Path) -> usize {
    git_in(
        root,
        &[
            "-c",
            "user.name=张伟",
            "-c",
            "user.email=zhang@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            LONG_SUBJECT,
        ],
    );
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(app);
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    view.commits
        .iter()
        .position(|c| c.subject == LONG_SUBJECT)
        .unwrap()
}

/// 将鼠标移到日志列表第 `row` 行的说明上，并重新绘制一帧。
fn hover_log_row(app: &mut App, row: usize) {
    let (rect, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == crate::app::git::log::LogTarget::Row(row))
        .unwrap();
    app.on_mouse_move(rect.x + 200.0, rect.y + rect.h / 2.0);
    paint(app);
}

#[test]
fn a_new_log_shows_only_the_current_branch() {
    let Some((mut app, root)) = git_app("git-log-current", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let log = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(log.query.branches, ["main"]);
    // 仅在 `dev` 上领先的提交不在列表中，最上方是 `main` 上最新的提交。
    assert_eq!(log.commits[0].subject, "启动日志写到文件里");
    assert!(
        log.commits
            .iter()
            .all(|c| c.subject != "配置：加入调试开关")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn log_lists_commits_with_their_graph_and_opens_a_files_diff() {
    use crate::app::git::log::LogTarget;
    use crate::app::git::log::tests::LOG_TITLE;

    let Some((mut app, root)) = git_app("git-log", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    assert!(show_all_log(&mut app));
    paint(&mut app);
    let view = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .map(|v| (v.commits.len(), v.graph.len(), v.selected, v.complete))
    };
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some(LOG_TITLE));
    // `history_repo` 的 7 次提交、1 次合并，加上创建仓库时的第一次提交，共 9 次。
    let (count, graph, selected, complete) = view(&app).unwrap();
    assert_eq!((count, graph), (9, 9));
    assert!(complete);
    // 打开时选中 `HEAD`（`main` 上最新的一次：「启动日志写到文件里」），`dev` 在其后。
    let log = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(log.commits[selected.unwrap()].subject, "启动日志写到文件里");
    assert_eq!(log.commits[0].subject, "配置：加入调试开关");
    // 合并提交绘制在两条车道上方。
    let merge = log
        .commits
        .iter()
        .position(|c| c.parents.len() == 2)
        .unwrap();
    assert!(log.graph[merge].down.len() >= 2);
    let files = log.details.as_ref().and_then(|d| d.files.clone()).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].path.ends_with("src/main.rs"));

    // ↓ 选中下一个提交；点击文件打开该提交的差异。
    app.log_key(&Key::Named(NamedKey::ArrowDown));
    assert_eq!(view(&app).unwrap().2, Some(selected.unwrap() + 1));
    app.log_key(&Key::Named(NamedKey::ArrowUp));
    paint(&mut app);
    app.press_log(LogTarget::File(0));
    let doc = app.editor.active().unwrap();
    assert!(doc.title.as_deref().unwrap().starts_with("main.rs · "));
    assert!(doc.diff.is_some());
    let diff_index = app.editor.active.unwrap();
    let title = doc.name().into_owned();

    // ⌘9 回到日志标签，不新开。
    let tabs = app.editor.documents.len();
    assert!(app.run_action(Action::GitShowLog));
    assert_eq!(app.editor.documents.len(), tabs);
    assert!(app.log_active());

    // 差异标签不在前台时，提示仍按差异记录的文件路径显示完整名称（含提交号）和所在目录；
    // 鼠标移出标签后提示消失。
    paint(&mut app);
    let hit = app
        .hits
        .tabs
        .iter()
        .find(|hit| hit.index == diff_index)
        .unwrap()
        .tab;
    app.on_mouse_move(hit.x + 20.0, hit.y + hit.h / 2.0);
    let folder = winc_editor::display_path(&root.join("src"), None);
    assert_eq!(
        crate::view::tests::tooltip(&app),
        Some((hit, format!("{title}\n{folder}")))
    );
    app.on_mouse_move(
        app.layout.editor.x + 20.0,
        app.layout.editor.bottom() - 10.0,
    );
    assert_eq!(crate::view::tests::tooltip(&app), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stepping_past_the_last_change_opens_the_commits_next_file() {
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-step", false) else {
        return;
    };
    history_repo(&root, 3);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    std::fs::write(root.join("b.txt"), "two\n").unwrap();
    git_in(&root, &["add", "a.txt", "b.txt"]);
    git_in(&root, &["commit", "-q", "-m", "两个文件"]);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    app.press_log(LogTarget::File(0));
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    assert!(title(&app).starts_with("a.txt · "));
    // 每个文件只有一处改动：按 `F7` 跳到该处，再按提示「再按一次」，第三次切换到下一个文件，仍在
    // 同一标签中；在第一个文件中向前跳转时没有上一个文件。
    let tabs = app.editor.documents.len();
    app.run_action(Action::NextDifference);
    app.run_action(Action::NextDifference);
    assert!(title(&app).starts_with("a.txt · "));
    app.run_action(Action::NextDifference);
    assert!(title(&app).starts_with("b.txt · "));
    assert_eq!(app.editor.documents.len(), tabs);
    app.run_action(Action::NextDifference);
    app.run_action(Action::NextDifference);
    assert!(title(&app).starts_with("b.txt · "));
    assert_eq!(app.status.message.as_deref(), Some("已是最后一处更改"));
    app.run_action(Action::PreviousDifference);
    app.run_action(Action::PreviousDifference);
    assert!(title(&app).starts_with("a.txt · "));
    let _ = std::fs::remove_dir_all(&root);
}

/// 点击当前差异中 `target` 的位置（按上一帧画出的位置找）。
fn click_diff(app: &mut App, target: crate::app::DiffTarget) {
    paint(app);
    let rect = (app.hits.diff.iter())
        .find(|(_, t)| *t == target)
        .unwrap()
        .0;
    click(app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
    app.poll_log();
    paint(app);
}

/// 从日志打开的差异：侧栏仍是工作区的改动；差异上方写明提交，左侧文件栏只列出这个提交改动的
/// 文件，点击在同一个标签里切换；日志改选其他提交后仍对应原提交；「返回日志」和 Esc 回到日志，
/// 关掉差异标签后文件列表作废。
#[test]
fn a_history_diff_lists_its_files_beside_the_diff_and_keeps_the_sidebar() {
    use crate::app::DiffTarget;
    use crate::app::git::log::LogTarget;
    use crate::layout::SidebarView;

    let Some((mut app, root)) = git_app("git-log-file-panel", false) else {
        return;
    };
    std::fs::create_dir_all(root.join("src/nested")).unwrap();
    std::fs::write(root.join("src/nested/one.txt"), "first\n").unwrap();
    std::fs::write(root.join("src/two.txt"), "second\n").unwrap();
    git_in(&root, &["add", "src/nested/one.txt", "src/two.txt"]);
    git_in(&root, &["commit", "-q", "-m", "Add nested files"]);
    std::fs::write(root.join("untracked.txt"), "working tree only\n").unwrap();
    app.git_refresh();
    show_all_log(&mut app);
    let log_index = app.editor.active.unwrap();
    app.set_sidebar_view(SidebarView::Commit);
    app.press_log(LogTarget::File(0));
    app.poll_log();
    paint(&mut app);

    // 侧栏没换：仍是提交视图里工作区的改动（未跟踪的文件在其中）。
    assert_eq!(app.panels.sidebar.view, SidebarView::Commit);
    assert!(app.commit_shown());
    let entries = app.git.status.as_ref().unwrap().entries.iter();
    assert!(
        entries
            .map(|e| &e.path)
            .any(|p| p.ends_with("untracked.txt"))
    );
    // 差异上方写明提交；两个文件，文件栏默认展开。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let banner = view.banner.as_ref().unwrap();
    assert_eq!(banner.subject, "Add nested files");
    assert!(view.panel > 0.0);
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    assert!(title(&app).starts_with("one.txt · "));

    let (diff_index, doc, tabs) = (
        app.editor.active.unwrap(),
        app.editor.active().unwrap().id,
        app.editor.documents.len(),
    );
    click_diff(&mut app, DiffTarget::File(1));
    assert!(title(&app).starts_with("two.txt · "));
    assert_eq!(app.editor.active().unwrap().id, doc);
    assert_eq!(app.editor.documents.len(), tabs);

    // 日志标签改选另一个提交后，差异的文件栏仍是原提交的文件。
    app.activate_tab(log_index);
    app.select_log_row(1);
    app.activate_tab(diff_index);
    click_diff(&mut app, DiffTarget::File(0));
    assert!(title(&app).starts_with("one.txt · "));

    // 「返回日志」和 Esc 都回到日志。
    click_diff(&mut app, DiffTarget::BannerBack);
    assert!(app.log_active());
    app.activate_tab(diff_index);
    app.diff_key(&Key::Named(NamedKey::Escape), false);
    assert!(app.log_active());

    // 关掉差异标签后文件列表作废。
    app.close_tab(diff_index);
    assert!(app.next_log_file(true).is_none() && app.next_log_file(false).is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 平铺与按目录分组各处分开记：提交视图按设置，日志详情、差异的文件栏切换过以后各自记住。
/// 文件栏可以收起、拖宽，‹ › 按文件栏的顺序切换，按钮的提示写明点了以后怎样。
#[test]
fn each_file_list_keeps_its_own_layout_and_the_panel_folds_and_resizes() {
    use crate::app::DiffTarget;
    use crate::app::git::log::{LogFileRow, LogTarget};

    let Some((mut app, root)) = git_app("git-log-file-panel-layout", false) else {
        return;
    };
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("src/b.txt"), "b\n").unwrap();
    git_in(&root, &["add", "a.txt", "src/b.txt"]);
    git_in(&root, &["commit", "-q", "-m", "Two files"]);
    app.git_refresh();
    show_all_log(&mut app);
    app.update_state(|state| {
        state.commit_tree = false;
        state.log_files_tree = None;
        state.diff_files_tree = None;
    });
    app.press_log(LogTarget::File(0));
    app.poll_log();
    paint(&mut app);

    // 没切换过时同提交视图（平铺）；切换文件栏只改它自己。
    assert!(app.diff_files_flat() && app.log_details_flat());
    click_diff(&mut app, DiffTarget::FilesLayout);
    assert!(!app.diff_files_flat());
    assert_eq!(app.state.diff_files_tree, Some(true));
    assert!(app.log_details_flat() && !app.state.commit_tree);
    app.toggle_log_details_layout();
    assert!(!app.log_details_flat());
    assert_eq!(app.state.log_files_tree, Some(true));
    assert!(!app.diff_files_flat());

    // 同文件视图先目录后文件：`src` 下的 `b.txt` 在 `a.txt` 前面。
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    assert!(title(&app).starts_with("a.txt · "));
    assert!(!app.step_log_file(true));
    assert!(app.step_log_file(false));
    app.poll_log();
    assert!(title(&app).starts_with("b.txt · "));

    // 点目录收起，其中的文件不再列出；转到收起的目录里的文件时目录展开。
    let files = |app: &App| {
        app.log_files_rows()
            .iter()
            .filter_map(LogFileRow::file)
            .count()
    };
    paint(&mut app);
    click_diff(&mut app, DiffTarget::FilesDir(0));
    assert_eq!(files(&app), 1);
    assert!(app.step_log_file(true));
    app.poll_log();
    assert_eq!(files(&app), 1);
    assert!(app.step_log_file(false));
    app.poll_log();
    assert_eq!(files(&app), 2);

    // 提示写明点了以后怎样。
    let rect = (app.hits.diff.iter())
        .find(|(_, t)| *t == DiffTarget::FilesLayout)
        .unwrap()
        .0;
    app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
    paint(&mut app);
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(
        tip.as_deref(),
        Some(winc_i18n::t!("workspace-tip-show-as-list"))
    );

    // 收起、展开文件栏；拖宽以后双击右边缘恢复默认宽度。
    let width = |app: &App| app.log_files_width();
    let open = width(&app);
    click_diff(&mut app, DiffTarget::FilesToggle);
    assert_eq!(width(&app), 0.0);
    click_diff(&mut app, DiffTarget::FilesToggle);
    assert_eq!(width(&app), open);
    app.drag_log_files_edge(500.0, (400.0, open));
    assert_eq!(width(&app), open + 100.0);
    app.reset_log_files_width();
    assert_eq!(width(&app), open);

    // 地方不够时文件栏先变窄，比最窄的宽度还窄时才不显示。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let parts = |w: f32| crate::app::diff_parts(Rect::new(0.0, 0.0, w, 600.0), view, 8.0);
    assert_eq!(parts(2000.0).files.w, open);
    assert_eq!(parts(720.0).files.w, 240.0);
    assert_eq!(parts(600.0).files.w, 0.0);
    // 展开、收起途中只露出一部分，各行仍按展开时的宽度排。
    let shown = |app: &mut App, f: f32| {
        let view = app
            .editor
            .active_mut()
            .unwrap()
            .diff
            .as_deref_mut()
            .unwrap();
        view.panel_shown = f;
        crate::app::diff_parts(Rect::new(0.0, 0.0, 2000.0, 600.0), view, 8.0)
    };
    let sliding = shown(&mut app, 0.5);
    assert_eq!(sliding.files.w, (open / 2.0).round());
    assert_eq!(sliding.files_w, open);
    shown(&mut app, 1.0);

    // 详情里的目录与文件栏各收各的；收起的目录里的文件不能用键盘选，回车仍打开最前面的文件。
    app.git_show_log();
    paint(&mut app);
    assert_eq!(app.log_file_order(), [1, 0]);
    app.press_log(LogTarget::FilesDir(0));
    assert_eq!(app.log_file_order(), [0]);
    assert_eq!(app.log_first_file(), Some(1));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn uncommitted_changes_sit_above_head_in_the_log() {
    use crate::app::git::log::{LOG_LIST_PAD, LogTarget};

    let Some((mut app, root)) = git_app("git-log-uncommitted", false) else {
        return;
    };
    history_repo(&root, 5);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let list = app.log_parts().list;
    let row_y = |app: &App, target: LogTarget| {
        app.hits
            .log
            .iter()
            .filter(|(_, t)| *t == target)
            .map(|(r, _)| r.y)
            .reduce(f32::min)
    };
    // 工作区没有改动时不显示该行，第一个提交位于最上方。
    assert_eq!(row_y(&app, LogTarget::Uncommitted), None);
    assert_eq!(row_y(&app, LogTarget::Row(0)), Some(list.y + LOG_LIST_PAD));

    // 修改一个文件后最上方增加一行，各提交下移一行；筛选时不显示该行。
    std::fs::write(root.join("README.md"), "changed\n").unwrap();
    app.git_refresh();
    paint(&mut app);
    let lh = app.fonts.line_height;
    assert_eq!(
        row_y(&app, LogTarget::Uncommitted),
        Some(list.y + LOG_LIST_PAD)
    );
    assert_eq!(
        row_y(&app, LogTarget::Row(0)),
        Some(list.y + LOG_LIST_PAD + lh)
    );
    app.change_log_query(|q| q.text = "登录".into());
    paint(&mut app);
    assert_eq!(row_y(&app, LogTarget::Uncommitted), None);
    app.change_log_query(|q| q.text.clear());
    paint(&mut app);

    // 点击该行打开提交视图。
    app.press_log(LogTarget::Uncommitted);
    assert!(app.commit_shown());
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：日志标签，浅色、深色各一张（`log-light`、`log-dark`）。
#[test]
#[ignore]
fn shoot_git_log() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-log-shot-{theme}"), false, dark)
        else {
            return;
        };
        history_repo(&root, 40);
        app.git_refresh();
        app.run_action(Action::GitShowLog);
        paint(&mut app);
        // 选中带说明正文的提交。
        let row = app
            .editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .commits
            .iter()
            .position(|c| c.subject.starts_with("登录：失败"))
            .unwrap();
        app.select_log_row(row);
        shoot(&mut app, &format!("log-{theme}"));

        // 在详情中选中正文的一段（可以选中并复制）。
        paint(&mut app);
        let body = app.git.log.runs[1].clone();
        let point = |ch| crate::app::git::log::TextPoint { run: 1, ch };
        app.git.log.text_sel = Some((point(2), point(body.edges.len() - 3)));
        shoot(&mut app, &format!("log-select-{theme}"));
        app.git.log.text_sel = None;

        // ⌘ 点击另一个提交：比较两者。
        let newer = app
            .editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .commits
            .iter()
            .position(|c| c.subject == "启动日志写到文件里")
            .unwrap();
        app.pointer.modifiers = command_modifiers();
        app.press_log(crate::app::git::log::LogTarget::Row(newer));
        app.pointer.modifiers = ModifiersState::empty();
        shoot(&mut app, &format!("log-compare-{theme}"));
        app.clear_log_compare();

        // 按说明筛选「登录」，再按作者筛选：不绘制提交图，下拉框使用淡强调色背景，
        // 末尾是「清除筛选」。
        app.press_log(crate::app::git::log::LogTarget::Search);
        app.edit_field(|f| f.insert("登录"));
        app.change_log_query(|q| q.authors = vec!["Alice Chen".into()]);
        app.select_log_row(0);
        shoot(&mut app, &format!("log-filter-{theme}"));

        // 筛选结果为空：不绘制详情一栏，提示在整个标签中居中。
        app.change_log_query(|q| q.text = "没有这样的说明".into());
        shoot(&mut app, &format!("log-empty-{theme}"));
        app.change_log_query(|q| q.text = "登录".into());

        // 工作区有改动时，最上方增加一行「未提交的更改」，并在 `HEAD` 的车道上绘制空心圆。
        app.git.log.filter.set_text("");
        app.change_log_query(|q| {
            q.text.clear();
            q.authors.clear();
        });
        std::fs::write(root.join("README.md"), "changed\n").unwrap();
        app.git_refresh();
        shoot(&mut app, &format!("log-uncommitted-{theme}"));

        // 文件历史。
        app.focus = Focus::Editor;
        app.git_file_history(&root.join("src/main.rs"));
        paint(&mut app);
        shoot(&mut app, &format!("log-history-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn log_spills_long_subjects_on_hover_and_drags_the_divider() {
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-long", false) else {
        return;
    };
    history_repo(&root, 10);
    let long = long_subject_row(&mut app, &root);

    // 悬停在说明无法完整显示的行上：作者、时间两列让出位置，说明向右展开后仍显示不全时记入
    // `clipped`，悬停提示显示整句。
    hover_log_row(&mut app, long);
    assert_eq!(app.git.log.clipped, Some(long));
    assert!(
        !app.hits
            .log
            .iter()
            .any(|(_, t)| *t == LogTarget::Avatar(long))
    );
    assert!(Hover::Log(LogTarget::Row(long)).has_tooltip());
    // 提示从开始悬停时鼠标的位置开始，在这一行下方；之后鼠标在同一行里移动，提示不跟着动。
    {
        use crate::app::animation::TipPlace;
        let since = std::time::Instant::now() - std::time::Duration::from_secs(1);
        app.overlays.tooltip.since = Some(since);
        let (x, y) = app.mouse();
        app.on_mouse_move(x + 30.0, y);
        app.overlays.tooltip.since = Some(since);
        paint(&mut app);
        let last = app.overlays.tooltip.last.borrow().clone();
        let (anchor, _, place) = last.unwrap();
        assert_eq!((place, anchor.x), (TipPlace::Pointer, x));
    }
    // 能完整显示的行不提示。
    hover_log_row(&mut app, long + 1);
    assert_eq!(app.git.log.clipped, None);

    // 向左拖动分隔线：详情一栏随之变宽；双击恢复默认宽度。
    let before = app.log_parts().details.w;
    let (divider, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::Divider)
        .unwrap();
    let (x, y) = (divider.x + divider.w / 2.0, divider.y + 200.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.pointer.hover, Hover::Log(LogTarget::Divider));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - 100.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.log_parts().details.w, before + 100.0);
    assert_eq!(app.git.log.details_w, Some(before + 100.0));
    // 拖动距离再大，列表也保留最小宽度。
    app.on_mouse_move(x - 100.0, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(-5000.0, y);
    app.on_mouse_up(MouseButton::Left);
    let parts = app.log_parts();
    assert!(parts.list.w >= 360.0 - 0.5);
    paint(&mut app);
    let (divider, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::Divider)
        .unwrap();
    let x = divider.x + divider.w / 2.0;
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.git.log.details_w, None);
    assert_eq!(app.log_parts().details.w, before);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：提交图、悬停时展开的长说明、拖宽的详情（`log-graph-light`、`log-graph-dark`）。
#[test]
#[ignore]
fn shoot_log_graph() {
    use crate::app::git::log::LogTarget;

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-log-graph-{theme}"), false, dark)
        else {
            return;
        };
        history_repo(&root, 40);
        let long = long_subject_row(&mut app, &root);
        hover_log_row(&mut app, long);
        app.overlays.tooltip.since = Some(
            std::time::Instant::now()
                - crate::theme::TOOLTIP_DELAY
                - crate::app::animation::TOOLTIP_FADE,
        );
        shoot(&mut app, &format!("log-graph-{theme}"));
        app.overlays.tooltip.since = None;

        // 指针位于合并提交分出的那段线上：这条线高亮并显示手形指针，提示点击后跳转到的提交。
        let (merge, line) = {
            let v = app.editor.active().unwrap().log.as_deref().unwrap();
            let merge = v.commits.iter().position(|c| c.parents.len() == 2).unwrap();
            let tip = v
                .commits
                .iter()
                .position(|c| c.subject == "登录：失败三次后锁定")
                .unwrap();
            (merge, v.graph[tip].line)
        };
        let (rect, _) = *app
            .hits
            .log
            .iter()
            .find(|(_, t)| *t == LogTarget::Line(merge, line))
            .unwrap();
        app.on_mouse_move(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        paint(&mut app);
        app.overlays.tooltip.since = Some(
            std::time::Instant::now()
                - crate::theme::TOOLTIP_DELAY
                - crate::app::animation::TOOLTIP_FADE,
        );
        shoot(&mut app, &format!("log-line-{theme}"));
        app.overlays.tooltip.since = None;
        app.set_hover(Hover::Log(LogTarget::Divider));
        app.git.log.details_w = Some(460.0);
        shoot(&mut app, &format!("log-divider-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn log_filters_by_text_branch_and_author_and_shows_a_files_history() {
    use crate::app::git::branch::picker::BranchRow;
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-log-filter", false) else {
        return;
    };
    history_repo(&root, 10);
    git_in(
        &root,
        &[
            "update-ref",
            "refs/remotes/origin/feature/login",
            "feature/login",
        ],
    );
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let subjects = |app: &App| -> Vec<String> {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits.iter().map(|c| c.subject.clone()).collect()
    };
    assert_eq!(subjects(&app).len(), 9);

    // ⌘F 到筛选输入框（不打开编辑区的查找条）。
    assert!(app.run_action(Action::Find));
    assert_eq!(app.focus, Focus::LogFilter);
    assert_eq!(app.layout.search.h, 0.0);

    // 按说明筛选：只剩两次「登录」，不绘制提交图。
    app.press_log(LogTarget::Search);
    assert_eq!(app.focus, Focus::LogFilter);
    app.edit_field(|f| f.insert("登录"));
    assert_eq!(
        subjects(&app),
        ["登录：失败三次后锁定", "登录：账号密码表单"]
    );
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert!(view.query.graphless());
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    assert_eq!(subjects(&app).len(), 9);

    // 按分支名搜索本地和远程分支。选择后只修改日志，工作树留在 main。
    paint(&mut app);
    let (button, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, target)| *target == LogTarget::Branch)
        .unwrap();
    click(&mut app, (button.x + 5.0, button.y + button.h / 2.0));
    assert_eq!(app.focus, Focus::Picker);
    app.edit_field(|f| f.insert("no-such-branch"));
    assert!(app.overlays.picker.hits.is_empty());
    app.picker_accept();
    assert!(app.overlays.picker.open);
    assert_eq!(app.log_branch_filter(), None);
    app.edit_field(|f| {
        f.set_text("LOGIN");
        true
    });
    // 与分支选择器同样分组：本地、远程各一组，搜索时目录都展开；筛选不新建分支。
    let rows = app.overlays.picker.branch_rows.clone().unwrap();
    let at = |name: &str| {
        rows.iter().position(|r| {
            matches!(r, BranchRow::Branch { index, .. } if app.git.branches[*index].name == name)
        })
    };
    assert!(at("feature/login").is_some());
    assert!(matches!(rows[0], BranchRow::Locals { recent: false, .. }));
    assert!(
        rows.iter()
            .any(|r| matches!(r, BranchRow::Remotes { open: true, .. }))
    );
    assert!(!rows.iter().any(|r| matches!(r, BranchRow::Create(_))));
    app.overlays.picker.selected = at("origin/feature/login").unwrap();
    paint(&mut app);
    app.picker_accept();
    assert!(!app.overlays.picker.open);
    assert_eq!(app.log_branch_filter(), Some("origin/feature/login"));
    assert!(
        app.git
            .branches
            .iter()
            .any(|b| b.current && b.name == "main")
    );

    // 点击搜索结果也能选回本地分支；分支筛选后仍然绘制提交图。
    paint(&mut app);
    app.press_log(LogTarget::Branch);
    app.edit_field(|f| f.insert("feature/login"));
    paint(&mut app);
    let parts = crate::app::workspace::picker::picker_parts(&app, app.layout.picker);
    let row = parts.row(app.overlays.picker.selected, app.overlays.picker.scroll);
    click(&mut app, (row.x + 40.0, row.y + row.h / 2.0));
    assert_eq!(app.log_branch_filter(), Some("feature/login"));
    assert_eq!(subjects(&app).len(), 5);
    assert!(
        !app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .query
            .graphless()
    );

    // 再按作者筛选（经作者菜单）：feature/login 上 Alice 的提交。
    paint(&mut app);
    app.press_log(LogTarget::Author);
    let alice = app
        .git
        .log
        .authors
        .iter()
        .position(|a| a == "Alice Chen")
        .unwrap();
    app.open_context_menu(MenuTarget::LogAuthor, (0.0, 0.0));
    app.run_menu_command(MenuCommand::LogAuthor(alice + 1));
    assert_eq!(subjects(&app), ["登录：失败三次后锁定", "启动时先加载配置"]);

    // 清除筛选：回到全部。
    app.press_log(LogTarget::ClearFilters);
    assert_eq!(subjects(&app).len(), 9);

    // 文件历史：只列出修改过 `src/login.rs` 的提交，该文件排在详情文件列表的最前面，
    // 回车打开其差异。
    app.git_file_history(&root.join("src/login.rs"));
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("login.rs · 历史"));
    assert_eq!(
        subjects(&app),
        ["登录：失败三次后锁定", "登录：账号密码表单"]
    );
    app.log_key(&Key::Named(NamedKey::Enter));
    let doc = app.editor.active().unwrap();
    assert!(doc.title.as_deref().unwrap().starts_with("login.rs · "));
    assert!(doc.diff.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// 日志分支筛选的分组与选中项：多个远程按远程分组；命中规则与分支选择器相同；重新打开时选中
/// 已应用的筛选，查询只有空白字符时视为没有查询；筛选的分支已不在列表中时选中「全部」。
#[test]
fn log_branch_filter_selects_the_applied_branch_or_all() {
    use crate::app::git::branch::picker::BranchRow;
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-branch-filter", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let selected = |app: &App| -> BranchRow {
        let rows = app.overlays.picker.branch_rows.as_ref().unwrap();
        rows[app.overlays.picker.selected].clone()
    };
    let label = |row: BranchRow| match row {
        BranchRow::Branch { label, .. } => label,
        other => panic!("{other:?}"),
    };

    // 有多个远程时远程一组里按远程名分出目录，与分支选择器相同。
    for remote in ["fork", "origin"] {
        let name = format!("refs/remotes/{remote}/feature/login");
        git_in(&root, &["update-ref", &name, "feature/login"]);
    }
    app.git_refresh();
    app.press_log(LogTarget::Branch);
    app.edit_field(|f| f.insert("login"));
    let rows = app.overlays.picker.branch_rows.as_deref().unwrap();
    for remote in ["fork", "origin"] {
        // 只有一层子目录的目录合成一行写（`fork/feature`）。
        let folder = |r: &BranchRow| match r {
            BranchRow::Folder { label, .. } => label.starts_with(remote),
            _ => false,
        };
        assert!(rows.iter().any(folder));
    }
    app.close_picker();
    paint(&mut app);

    // 字符分散命中：`ftlg` 按顺序出现在 feature/login 中。
    app.press_log(LogTarget::Branch);
    app.edit_field(|f| f.insert("ftlg"));
    // 分支写在 `feature` 目录下，行里只写 `login`。
    assert_eq!(label(selected(&app)), "login");
    app.picker_accept();
    assert_eq!(app.log_branch_filter(), Some("feature/login"));

    // 重新打开时：筛选的分支在收起的目录里看不见（默认只展开当前分支所在的目录），选中「全部」；
    // 只输入空白字符时视为没有查询，同样如此。
    paint(&mut app);
    app.press_log(LogTarget::Branch);
    assert_eq!(selected(&app), BranchRow::All);
    app.edit_field(|f| f.insert(" "));
    assert_eq!(selected(&app), BranchRow::All);

    // 「全部」清除筛选。
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    for _ in 0..app.overlays.picker.hits.len() {
        app.picker_move(false);
    }
    assert_eq!(selected(&app), BranchRow::All);
    app.picker_accept();
    assert_eq!(app.log_branch_filter(), None);

    // 筛选的分支已删除：选中「全部」，回车清除筛选。
    app.change_log_query(|q| q.branches = vec!["deleted/branch".to_owned()]);
    paint(&mut app);
    app.press_log(LogTarget::Branch);
    assert_eq!(selected(&app), BranchRow::All);
    app.picker_accept();
    assert_eq!(app.log_branch_filter(), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：一个提交上有多个长名称的分支和标签：
/// 徽章截短并合并为「+N」，说明仍然可见；详情中显示全名。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_git_log_crowded_refs() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-log-refs-{theme}"), false, dark)
        else {
            return;
        };
        history_repo(&root, 40);
        for args in [
            &["tag", "v3.1.8-fs-proxy-auth"][..],
            &["branch", "github-desktop-fengwm64/v3.1.8-fs-proxy-auth"],
            &["branch", "github-desktop-Sakuralaaa/main"],
            &["tag", "v3.1.8"],
        ] {
            git_in(&root, args);
        }
        app.git_refresh();
        app.run_action(Action::GitShowLog);
        paint(&mut app);
        // 选中带有多个引用的提交（`HEAD`）。
        let row = app
            .editor
            .active()
            .and_then(|d| d.log.as_deref())
            .and_then(|v| v.commits.iter().position(|c| c.refs.len() >= 4))
            .unwrap();
        app.select_log_row(row);
        shoot(&mut app, &format!("log-refs-{theme}"));
    }
}

#[test]
fn a_tag_can_be_created_on_a_commit_from_the_log() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};
    let Some((mut app, root)) = git_app_in("git-log-tag", false, false) else {
        return;
    };
    history_repo(&root, 40);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let oid = app.editor.active().unwrap().log.as_deref().unwrap().commits[2]
        .oid
        .clone();
    // 右键菜单里有「新建标签…」和「复制短提交号」。
    use crate::app::input::context_menu::{CommitActions, log_commit_menu};
    let actions = CommitActions {
        enabled: true,
        undo: false,
        push: false,
        on_head: None,
        child: false,
        parent: true,
        changes: true,
    };
    let menu = log_commit_menu(actions, "main", Vec::new(), &[]);
    for command in [MenuCommand::LogNewTag, MenuCommand::LogCopyShortOid] {
        assert!(
            menu.contains(&MenuEntry::Item(command, true)),
            "{command:?}"
        );
    }
    app.run_log_commit_command(MenuCommand::LogCopyShortOid, &oid);
    assert_eq!(app.clipboard.copied.as_deref(), Some(&oid[..7]));

    // 输入标签名后回车：标签创建在该提交上。
    app.run_log_commit_command(MenuCommand::LogNewTag, &oid);
    assert_eq!(app.focus, Focus::Prompt);
    paint(&mut app);
    app.edit_field(|f| f.insert("v9 .0"));
    assert_eq!(app.bars.goto_input.as_ref().unwrap().text(), "v9.0");
    app.submit_prompt();
    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-list", "-n", "1", "v9.0"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), oid);
    // 打开着的日志随之重新读取：该提交上出现这个标签。
    let mut logs = app.editor.documents.iter().filter_map(|d| d.log.as_deref());
    let commit = logs.next().unwrap().commits.iter().find(|c| c.oid == oid);
    let refs = &commit.unwrap().refs;
    assert!(
        refs.contains(&winc_editor::git::Ref::Tag("v9.0".into())),
        "{refs:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：`WINC_LOG_REPO` 指向的真实仓库的日志，只读，用于检查分支较多时的提交图。
///
/// `WINC_LOG_SCROLL` 指定滚动到的提交序号（默认 0），
/// `WINC_LOG_SELECT` 指定选中的提交序号（默认 `HEAD`）。
#[test]
#[ignore]
fn shoot_log_repo() {
    let Ok(repo) = std::env::var("WINC_LOG_REPO") else {
        return;
    };
    let number = |key: &str| {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
    };
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut app = app_with(Path::new(&repo), dark);
        app.settings_toggle(Setting::GitModule);
        if git_unusable(&app) {
            return;
        }
        app.git_refresh();
        app.run_action(Action::GitShowLog);
        paint(&mut app);
        if let Some(row) = number("WINC_LOG_SELECT") {
            app.select_log_row(row);
        }
        app.editor.active_mut().unwrap().scroll_line =
            number("WINC_LOG_SCROLL").unwrap_or(0) as f64;
        shoot(&mut app, &format!("log-repo-{theme}"));
    }
}

#[test]
fn graph_lines_and_dots_can_be_clicked() {
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-lines", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let row_of = |app: &App, subject: &str| {
        let v = app.editor.active().unwrap().log.as_deref().unwrap();
        v.commits.iter().position(|c| c.subject == subject).unwrap()
    };
    let merge = row_of(&app, "合并分支 feature/login");
    let tip = row_of(&app, "登录：失败三次后锁定");
    let feature = app.editor.active().unwrap().log.as_deref().unwrap().graph[tip].line;

    // 合并提交所在行分出、连接到 `feature/login` 的那段线：悬停时高亮并显示手形指针，
    // 提示点击后跳转到的提交。
    let (rect, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::Line(merge, feature))
        .expect("合并提交那一行有 feature/login 的线");
    let at = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    app.on_mouse_move(at.0, at.1);
    assert_eq!(
        app.pointer.hover,
        Hover::Log(LogTarget::Line(merge, feature))
    );
    assert_eq!(app.pointer.cursor_icon, CursorIcon::Pointer);
    assert!(app.pointer.hover.has_tooltip());
    assert_eq!(app.log_line_target(merge, feature), Some(tip));
    click(&mut app, at);
    let selected = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .selected
    };
    assert_eq!(selected(&app), Some(tip));

    // 后退回到跳转之前选中的提交（打开时选中的 `HEAD`），前进则返回；没有可后退的记录时交给编辑区。
    let head = row_of(&app, "启动日志写到文件里");
    assert!(app.run_action(Action::GoBack));
    assert_eq!(selected(&app), Some(head));
    assert!(app.run_action(Action::GoForward));
    assert_eq!(selected(&app), Some(tip));
    assert!(!app.log_navigate(true));

    // `feature/login` 的线在该分支最早的提交下方合回主线：点击该处跳转到合入的提交。
    let first = row_of(&app, "登录：账号密码表单");
    let base = row_of(&app, "启动时先加载配置");
    assert_eq!(app.log_line_target(first, feature), Some(base));

    // 点击圆点选中该提交。
    paint(&mut app);
    let (dot, _) = *app
        .hits
        .log
        .iter()
        .find(|(r, t)| *t == LogTarget::Row(merge) && r.w <= 12.0)
        .expect("合并提交的圆点");
    click(&mut app, (dot.x + dot.w / 2.0, dot.y + dot.h / 2.0));
    assert_eq!(selected(&app), Some(merge));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_log_list_has_room_on_top_and_the_author_column_eases_in() {
    use crate::app::git::log::{LOG_LIST_PAD, LogTarget};

    let Some((mut app, root)) = git_app("git-log-pad", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    // 第一行上方留出间距，不紧贴标题栏的分隔线。
    let list = app.log_parts().list;
    // 该行的底边（取最宽的 `Row(0)`，圆点同样记为 `Row(0)`）。
    let (first, _) = *app
        .hits
        .log
        .iter()
        .filter(|(_, t)| *t == LogTarget::Row(0))
        .max_by(|a, b| a.0.w.total_cmp(&b.0.w))
        .unwrap();
    assert_eq!(first.y, list.y + LOG_LIST_PAD);
    // 详情的第一行（说明）和列表的第一行在同一条线上：两边垂直居中对齐。
    let subject = &app.git.log.runs[0].row;
    let center = |r: &Rect| r.y + r.h / 2.0;
    assert!((center(subject) - center(&first)).abs() < 0.01);

    // 拖动分隔线使列表逐渐变窄：作者一列（头像左边缘到时间之间）连续变窄，不发生跳变。
    let avatar_x = |app: &App| {
        app.hits
            .log
            .iter()
            .find(|(_, t)| *t == LogTarget::Avatar(0))
            .map(|(r, _)| r.x)
            .unwrap()
    };
    let mut last: Option<(f32, f32)> = None;
    for details in (300..=900).step_by(20) {
        app.git.log.details_w = Some(details as f32);
        paint(&mut app);
        let list = app.log_parts().list;
        let gap = list.right() - avatar_x(&app);
        if let Some((w, g)) = last
            && list.w < w
        {
            assert!(gap <= g + 0.01, "列表变窄时作者一列不会变宽");
            // 每一步的宽度变化小于 25 像素，作者一列不会突然消失。
            assert!(g - gap < 25.0, "作者一列一下子窄了一大截：{g} → {gap}");
        }
        last = Some((list.w, gap));
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn log_rows_left_behind_do_not_swallow_editor_clicks() {
    use crate::app::git::log::LogTarget;

    let dir = sample_folder("log-stale-hits");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    // 日志标签绘制过的提交行：切换到代码标签后不再绘制日志，这些位置不再识别为提交。
    let text = app.layout.text;
    app.hits.log.push((text, LogTarget::Row(0)));
    paint(&mut app);
    let (x, y) = (text.x + 40.0, text.y + 5.0);
    assert_ne!(app.hover_at(x, y), Hover::Log(LogTarget::Row(0)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_log_follows_new_commits_and_moved_remote_branches() {
    use winc_editor::git::Ref;

    let Some((mut app, root)) = git_app("git-log-follow", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let first = |app: &App| {
        let log = app.editor.active().unwrap().log.as_deref().unwrap();
        log.commits.first().cloned().unwrap()
    };

    // 提交后无需手动刷新，新提交出现在最上方。
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "新的提交"]);
    app.git_refresh();
    paint(&mut app);
    assert_eq!(first(&app).subject, "新的提交");

    // 推送只移动远程分支、`HEAD` 不变：日志同样重新读取，远程分支标在新提交上。
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    app.git_refresh();
    paint(&mut app);
    let remote = Ref::Remote("origin/main".into());
    assert!(first(&app).refs.contains(&remote), "{:?}", first(&app).refs);
    let _ = std::fs::remove_dir_all(&root);
}

/// 修改上次提交之后，原来的提交不在日志中：按缩写的提交号转到它时固定在最前面并选中，再
/// `⌘` 单击修改后的提交即可比较两者，固定的提交作为旧的一侧。转到已列出的提交时直接选中；
/// 无法解析时提示原因，格式不对时无法提交。
#[test]
fn going_to_an_amended_away_commit_pins_it_for_comparison() {
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-go-to-amended", false) else {
        return;
    };
    let rev = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() { before(); }\n").unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    git_in(&root, &["commit", "-m", "before amend"]);
    let old = rev(&["rev-parse", "HEAD"]);
    std::fs::write(&file, "fn main() { after(); }\n").unwrap();
    git_in(&root, &["commit", "--amend", "-am", "after amend"]);
    let new = rev(&["rev-parse", "HEAD"]);
    let branch = rev(&["rev-parse", "--abbrev-ref", "HEAD"]);

    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    fn log(app: &App) -> &winc_editor::git::log::LogView {
        app.editor.active().unwrap().log.as_deref().unwrap()
    }
    assert!(log(&app).commits.iter().all(|c| c.oid != old));

    let go_to = |app: &mut App, text: &str| {
        app.run_action(Action::GitGoToRevision);
        assert_eq!(app.focus, Focus::Dialog);
        app.edit_field(|f| f.insert(text));
        assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", false));
        paint(app);
    };
    // 以 `-` 开头的文字不能提交。
    app.run_action(Action::GitGoToRevision);
    app.edit_field(|f| f.insert("--all"));
    assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", false));
    assert!(app.overlays.form_dialog.is_some());
    app.close_form_dialog();

    // 不存在的提交：提示原因，日志不变。
    go_to(&mut app, &"0".repeat(12));
    assert!(
        app.notice
            .text
            .as_deref()
            .unwrap()
            .contains(&"0".repeat(12))
    );
    assert_eq!(log(&app).query.pin, None);

    // 修改之前的提交：固定在最前面并选中。
    go_to(&mut app, &old[..10].to_ascii_uppercase());
    let view = log(&app);
    assert_eq!(view.query.pin.as_deref(), Some(old.as_str()));
    assert_eq!(view.commits[0].oid, old);
    assert_eq!(view.selected, Some(0));
    assert!(view.query.filtered());

    // `⌘` 单击修改后的提交：比较两者，旧的在左。
    let row = view.commits.iter().position(|c| c.oid == new).unwrap();
    app.pointer.modifiers = command_modifiers();
    app.press_log(LogTarget::Row(row));
    app.pointer.modifiers = Default::default();
    paint(&mut app);
    let view = log(&app);
    let compare = view.compare.as_ref().unwrap();
    assert_eq!(
        (compare.from.as_str(), compare.to.as_str()),
        (old.as_str(), new.as_str())
    );
    let files = compare.files.as_ref().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, file);

    // 转到分支：分支所指的提交已列出，直接选中，固定的提交保留。
    go_to(&mut app, &branch);
    let view = log(&app);
    assert_eq!(view.commits[view.selected.unwrap()].oid, new);
    assert_eq!(view.query.pin.as_deref(), Some(old.as_str()));

    // 清除筛选时取消固定；之后转到分支不固定。
    app.press_log(LogTarget::ClearFilters);
    paint(&mut app);
    assert_eq!(log(&app).query.pin, None);
    go_to(&mut app, &branch);
    let view = log(&app);
    assert_eq!(view.query.pin, None);
    assert_eq!(view.commits[view.selected.unwrap()].oid, new);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn log_compares_with_a_revision_missing_from_the_list() {
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

    let Some((mut app, root)) = git_app("git-compare-revision", false) else {
        return;
    };
    let head = || {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let file = root.join("src/main.rs");
    let before = "fn main() { before(); }\n";
    let after = "fn main() { after(); }\n";
    std::fs::write(&file, before).unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    git_in(&root, &["commit", "-m", "before amend"]);
    let old = head();
    std::fs::write(&file, after).unwrap();
    git_in(&root, &["commit", "--amend", "-am", "after amend"]);
    let new = head();
    git_in(&root, &["commit", "--amend", "-m", "amended again"]);
    let tip = head();

    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    fn log(app: &App) -> &winc_editor::git::log::LogView {
        app.editor.active().unwrap().log.as_deref().unwrap()
    }
    assert!(
        log(&app)
            .commits
            .iter()
            .all(|c| c.oid != old && c.oid != new)
    );
    let enter = |app: &mut App| assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", false));
    app.run_action(Action::GitGoToRevision);
    app.edit_field(|f| f.insert(&old[..12]));
    enter(&mut app);
    paint(&mut app);
    assert_eq!(log(&app).query.pin.as_deref(), Some(old.as_str()));

    let open_compare = |app: &mut App| {
        let command = MenuCommand::LogCompareRevision;
        app.open_context_menu(MenuTarget::LogCommit(old.clone()), (0.0, 0.0));
        assert!(
            app.overlays
                .context_menu
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .any(|entry| matches!(entry, MenuEntry::Item(item, true) if *item == command))
        );
        app.run_menu_command(command);
        assert_eq!(app.focus, Focus::Dialog);
    };
    open_compare(&mut app);
    app.edit_field(|f| f.insert(&"0".repeat(12)));
    enter(&mut app);
    // 无法解析的版本：只提示原因，不进入比较。
    paint(&mut app);
    assert!(log(&app).compare.is_none());
    let notice = app.notice.text.as_deref().unwrap_or_default();
    assert!(notice.contains(&"0".repeat(12)), "{notice}");

    open_compare(&mut app);
    app.edit_field(|f| f.insert(&new[..12]));
    enter(&mut app);
    paint(&mut app);
    let view = log(&app);
    assert_eq!(view.query.pin.as_deref(), Some(old.as_str()));
    assert!(view.commits.iter().all(|c| c.oid != new));
    let compare = view.compare.as_ref().unwrap();
    assert_eq!((&compare.from, &compare.to), (&old, &new));
    let files = compare.files.as_ref().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, file);
    app.press_log(LogTarget::File(0));
    let diff = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(diff.old.text, before);
    assert_eq!(diff.new.text, after);
    assert_eq!(head(), tip);

    // 输入的标签是所选提交的祖先：排在旧的一侧，显示标签名，取数据用解析出的提交号。
    git_in(&root, &["tag", "base", "HEAD~1"]);
    git_in(&root, &["checkout", "--quiet", "HEAD~1"]);
    let base = head();
    git_in(&root, &["checkout", "--quiet", "-"]);
    app.run_action(Action::GitShowLog);
    open_compare(&mut app);
    app.edit_field(|f| f.insert("base"));
    enter(&mut app);
    paint(&mut app);
    let compare = log(&app).compare.as_ref().unwrap();
    assert_eq!((&compare.from, &compare.to), (&base, &old));
    assert_eq!(compare.from_name.as_deref(), Some("base"));
    assert_eq!(compare.to_name, None);
    assert_eq!(compare.files.as_ref().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}
