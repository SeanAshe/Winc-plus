//! Git 日志标签的测试：提交图、长说明的悬停、筛选、提交菜单、比较、标签与点选提交图。

use super::*;

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
    assert_eq!(log.query.branch.as_deref(), Some("main"));
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
    assert_eq!(app.status.message.as_deref(), Some("没有下一处更改"));
    app.run_action(Action::PreviousDifference);
    app.run_action(Action::PreviousDifference);
    assert!(title(&app).starts_with("a.txt · "));
    let _ = std::fs::remove_dir_all(&root);
}

/// 返回历史差异文件列表当前的各行（借用提交视图的列表排出）。
fn log_rows(app: &mut App) -> std::rc::Rc<crate::app::git::commit::CommitRows> {
    app.with_log_list(|app| app.commit_rows()).unwrap()
}

/// 点击历史差异文件列表的第 `row` 行（列表没有滚动）。
fn click_log_row(app: &mut App, row: usize) {
    let list = app.commit_parts().list;
    let y = list.y + (row as f32 + 0.5) * crate::app::git::commit::COMMIT_ROW;
    click(app, (list.x + 40.0, y));
}

/// 历史差异的侧栏借用提交视图的改动列表，只列出该提交改动的文件：目录可以收起、展开，
/// 日志改选其他提交后仍对应原提交，点击文件在同一个差异标签中打开。
#[test]
fn commit_diff_sidebar_lists_only_that_commits_changed_files() {
    use crate::app::git::commit::CommitRow;
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-file-tree", false) else {
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
    app.set_sidebar(false);
    // 显示方式与提交视图共用，默认平铺；该测试检查树形。
    app.commit.set_flat(false);
    app.press_log(LogTarget::File(0));
    paint(&mut app);

    let diff_index = app.editor.active.unwrap();
    let doc = app.editor.active().unwrap().id;
    let tabs = app.editor.documents.len();
    assert!(app.sidebar_open() && app.log_diff_tree().is_some());
    assert!(!app.commit_shown());
    // 组标题、`src`、`src/nested`、`one.txt`、`two.txt`：工作区里未跟踪的文件不在其中。
    let rows = log_rows(&mut app);
    assert_eq!(rows.len(), 5);
    assert!(matches!(&rows[1], CommitRow::Dir { label, .. } if label == "src"));
    assert!(matches!(&rows[2], CommitRow::Dir { label, .. } if label == "nested"));
    click_log_row(&mut app, 2);
    assert_eq!(log_rows(&mut app).len(), 4);
    click_log_row(&mut app, 2);
    assert_eq!(log_rows(&mut app).len(), 5);

    // 日志标签改选另一个提交后，原差异的侧栏仍然可以打开原提交的其他文件。
    app.activate_tab(log_index);
    app.select_log_row(1);
    assert!(app.log_diff_tree().is_none());
    app.activate_tab(diff_index);
    paint(&mut app);
    click_log_row(&mut app, 4);
    let active = app.editor.active().unwrap();
    assert_eq!(active.id, doc);
    assert_eq!(app.editor.documents.len(), tabs);
    assert!(active.title.as_deref().unwrap().starts_with("two.txt · "));

    let _ = std::fs::remove_dir_all(&root);
}

/// 历史差异的文件列表：逐个切换文件按列表的显示顺序进行；收起的目录在打开其中的文件时
/// 展开；侧栏只在新建差异标签时打开，之后保留用户的选择；提交视图可以在文件列表与工作区
/// 改动之间切换；关闭差异标签后文件列表随之作废。
#[test]
fn commit_diff_tree_follows_its_order_and_the_users_sidebar() {
    use crate::app::git::log::LogTarget;
    use crate::layout::SidebarView;

    let Some((mut app, root)) = git_app("git-log-file-tree-order", false) else {
        return;
    };
    // Git 列出的顺序是 `a.txt`、`src/b.txt`；树形中目录在前，顺序是 `src/b.txt`、`a.txt`。
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("src/b.txt"), "b\n").unwrap();
    git_in(&root, &["add", "a.txt", "src/b.txt"]);
    git_in(&root, &["commit", "-q", "-m", "Two files"]);
    app.git_refresh();
    show_all_log(&mut app);
    let log_index = app.editor.active.unwrap();
    app.commit.set_flat(false);
    app.press_log(LogTarget::File(0));
    paint(&mut app);
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    assert!(title(&app).starts_with("a.txt · "));
    assert!(
        app.next_log_file(true).is_none(),
        "a.txt 是列表中最后一个文件"
    );
    let (_, at, name, total) = app.next_log_file(false).unwrap();
    assert_eq!((at, name.as_str(), total), (0, "b.txt", 2));

    // 收起 `src` 后切换到其中的文件：目录随之展开。行依次是组标题、`src`、`b.txt`、`a.txt`。
    assert_eq!(log_rows(&mut app).len(), 4);
    click_log_row(&mut app, 1);
    assert_eq!(log_rows(&mut app).len(), 3);
    assert!(app.step_log_file(false));
    assert!(title(&app).starts_with("b.txt · "));
    assert_eq!(log_rows(&mut app).len(), 4);

    // 提交视图在文件列表与工作区改动之间切换。
    app.show_commit();
    assert!(app.log_diff_tree().is_none() && app.commit_shown());
    app.show_commit();
    assert!(app.log_diff_tree().is_some() && !app.commit_shown());

    // 差异标签已存在时，从日志再打开文件不改动用户选择的侧栏视图。
    let diff_index = app.editor.active.unwrap();
    app.set_sidebar_view(SidebarView::Files);
    app.activate_tab(log_index);
    app.press_log(LogTarget::File(1));
    paint(&mut app);
    assert_eq!(app.sidebar.view, SidebarView::Files);

    // 关闭差异标签后文件列表作废。
    app.activate_tab(diff_index);
    app.set_sidebar_view(SidebarView::Commit);
    assert!(app.log_diff_tree().is_some());
    app.close_tab(diff_index);
    assert!(app.next_log_file(true).is_none() && app.next_log_file(false).is_none());

    let _ = std::fs::remove_dir_all(&root);
}

/// 历史差异侧栏标题行的按钮在树形与列表之间切换文件列表，与提交视图的更改列表共用同一个
/// 设置；树形中只有一个子目录的各级按设置「紧凑文件夹」合成一行（「src/nested」）。
#[test]
fn commit_diff_tree_switches_between_tree_and_list() {
    use crate::app::git::commit::CommitRow;
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-file-tree-layout", false) else {
        return;
    };
    std::fs::create_dir_all(root.join("src/nested")).unwrap();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("src/nested/b.txt"), "b\n").unwrap();
    git_in(&root, &["add", "a.txt", "src/nested/b.txt"]);
    git_in(&root, &["commit", "-q", "-m", "Two files"]);
    app.git_refresh();
    show_all_log(&mut app);
    app.commit.set_flat(false);
    app.press_log(LogTarget::File(0));
    paint(&mut app);
    let press = |app: &mut App| {
        let button = app.log_diff_layout_button();
        click(app, (button.x + 4.0, button.y + 4.0));
    };
    // 树形：组标题、`src/nested`、`b.txt`、`a.txt`。
    let rows = log_rows(&mut app);
    assert_eq!(rows.len(), 4);
    assert!(matches!(&rows[1], CommitRow::Dir { label, .. } if label == "src/nested"));

    // 关闭设置「紧凑文件夹」后逐级显示：组标题、`src`、`nested`、`b.txt`、`a.txt`。
    app.config.compact_folders = Some(false);
    app.apply_compact_folders();
    let rows = log_rows(&mut app);
    assert_eq!(rows.len(), 5);
    assert!(matches!(&rows[1], CommitRow::Dir { label, .. } if label == "src"));
    app.config.compact_folders = Some(true);
    app.apply_compact_folders();

    press(&mut app);
    assert!(app.commit.flat());
    assert_eq!(log_rows(&mut app).len(), 3);

    // 悬停按钮时提示点击后切换成哪一种显示方式。
    let button = app.log_diff_layout_button();
    app.on_mouse_move(button.x + 4.0, button.y + 4.0);
    paint(&mut app);
    assert!(app.hover.has_tooltip(), "停留满提示延迟后显示提示");
    let tip = crate::view::tests::tooltip(&app).map(|(_, text)| text);
    assert_eq!(
        tip.as_deref(),
        Some(winc_i18n::t!("workspace-tip-show-as-tree"))
    );

    // 在提交视图中切换显示方式，文件列表随之重排。
    app.toggle_commit_layout();
    assert!(!app.commit.flat());
    assert_eq!(log_rows(&mut app).len(), 4);

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
        let body = app.log.runs[1].clone();
        let point = |ch| crate::app::git::log::TextPoint { run: 1, ch };
        app.log.text_sel = Some((point(2), point(body.edges.len() - 3)));
        shoot(&mut app, &format!("log-select-{theme}"));
        app.log.text_sel = None;

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
        app.modifiers = command_modifiers();
        app.press_log(crate::app::git::log::LogTarget::Row(newer));
        app.modifiers = ModifiersState::empty();
        shoot(&mut app, &format!("log-compare-{theme}"));
        app.clear_log_compare();

        // 按说明筛选「登录」，再按作者筛选：不绘制提交图，下拉框使用淡强调色背景，
        // 末尾是「清除筛选」。
        app.press_log(crate::app::git::log::LogTarget::Search);
        app.edit_field(|f| f.insert("登录"));
        app.change_log_query(|q| q.author = Some("Alice Chen".into()));
        app.select_log_row(0);
        shoot(&mut app, &format!("log-filter-{theme}"));

        // 筛选结果为空：不绘制详情一栏，提示在整个标签中居中。
        app.change_log_query(|q| q.text = "没有这样的说明".into());
        shoot(&mut app, &format!("log-empty-{theme}"));
        app.change_log_query(|q| q.text = "登录".into());

        // 工作区有改动时，最上方增加一行「未提交的更改」，并在 `HEAD` 的车道上绘制空心圆。
        app.log.filter.set_text("");
        app.change_log_query(|q| {
            q.text.clear();
            q.author = None;
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
    assert_eq!(app.log.clipped, Some(long));
    assert!(
        !app.hits
            .log
            .iter()
            .any(|(_, t)| *t == LogTarget::Avatar(long))
    );
    assert!(Hover::Log(LogTarget::Row(long)).has_tooltip());
    // 能完整显示的行不提示。
    hover_log_row(&mut app, long + 1);
    assert_eq!(app.log.clipped, None);

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
    assert_eq!(app.hover, Hover::Log(LogTarget::Divider));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - 100.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.log_parts().details.w, before + 100.0);
    assert_eq!(app.log.details_w, Some(before + 100.0));
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
    assert_eq!(app.log.details_w, None);
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
        app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("log-graph-{theme}"));
        app.tooltip.since = None;

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
        app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("log-line-{theme}"));
        app.tooltip.since = None;
        app.set_hover(Hover::Log(LogTarget::Divider));
        app.log.details_w = Some(460.0);
        shoot(&mut app, &format!("log-divider-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn log_filters_by_text_branch_and_author_and_shows_a_files_history() {
    use crate::app::git::branch::picker::{BranchRow, BranchSection};
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
    assert!(app.picker.hits.is_empty());
    app.picker_accept();
    assert!(app.picker.open);
    assert_eq!(app.log_branch_filter(), None);
    app.edit_field(|f| {
        f.set_text("LOGIN");
        true
    });
    // 只有一个远程：远程分支一组的标题为「远程分支」，各行省去远程名。
    let label = |name: &str| BranchRow::Branch {
        index: app
            .git
            .branches
            .iter()
            .position(|b| b.name == name)
            .unwrap(),
        depth: 0,
        label: "feature/login".to_owned(),
    };
    assert_eq!(
        app.picker.branch_rows.as_deref().unwrap(),
        [
            BranchRow::Heading(BranchSection::Local),
            label("feature/login"),
            BranchRow::Heading(BranchSection::Remote(None)),
            label("origin/feature/login"),
        ]
    );
    paint(&mut app);
    assert!(app.hits.picker_parts.is_empty());
    app.picker_move(true);
    app.picker_accept();
    assert!(!app.picker.open);
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
    let row = parts.row(app.picker.selected, app.picker.scroll);
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
    use crate::app::git::branch::picker::{BranchRow, BranchSection};
    use crate::app::git::log::LogTarget;

    let Some((mut app, root)) = git_app("git-log-branch-filter", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let selected = |app: &App| -> BranchRow {
        let rows = app.picker.branch_rows.as_ref().unwrap();
        rows[app.picker.selected].clone()
    };
    let label = |row: BranchRow| match row {
        BranchRow::Branch { label, .. } => label,
        other => panic!("{other:?}"),
    };

    // 有多个远程时按远程分组，组标题写远程名。
    for remote in ["fork", "origin"] {
        let name = format!("refs/remotes/{remote}/feature/login");
        git_in(&root, &["update-ref", &name, "feature/login"]);
    }
    app.git_refresh();
    app.press_log(LogTarget::Branch);
    app.edit_field(|f| f.insert("login"));
    let headings: Vec<_> = (app.picker.branch_rows.as_deref().unwrap().iter())
        .filter_map(|row| match row {
            BranchRow::Heading(section) => Some(section.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        headings,
        [
            BranchSection::Local,
            BranchSection::Remote(Some("fork".to_owned())),
            BranchSection::Remote(Some("origin".to_owned())),
        ]
    );
    app.close_picker();
    paint(&mut app);

    // 字符分散命中：`ftlg` 按顺序出现在 feature/login 中。
    app.press_log(LogTarget::Branch);
    app.edit_field(|f| f.insert("ftlg"));
    assert_eq!(label(selected(&app)), "feature/login");
    app.picker_accept();
    assert_eq!(app.log_branch_filter(), Some("feature/login"));

    // 重新打开时选中已应用的筛选，只输入空白字符时同样如此。
    paint(&mut app);
    app.press_log(LogTarget::Branch);
    assert_eq!(label(selected(&app)), "feature/login");
    app.edit_field(|f| f.insert(" "));
    assert_eq!(label(selected(&app)), "feature/login");

    // 「全部」清除筛选。
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    for _ in 0..app.picker.hits.len() {
        app.picker_move(false);
    }
    assert_eq!(selected(&app), BranchRow::All);
    app.picker_accept();
    assert_eq!(app.log_branch_filter(), None);

    // 筛选的分支已删除：选中「全部」，回车清除筛选。
    app.change_log_query(|q| q.branch = Some("deleted/branch".to_owned()));
    paint(&mut app);
    app.press_log(LogTarget::Branch);
    assert_eq!(selected(&app), BranchRow::All);
    app.picker_accept();
    assert_eq!(app.log_branch_filter(), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn log_commit_menu_checks_out_branches_picks_reverts_and_resets() {
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

    let Some((mut app, root)) = git_app("git-log-ops", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let row_of = |app: &App, subject: &str| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits
            .iter()
            .position(|c| c.subject == subject)
            .unwrap()
    };
    let oid_of = |app: &App, subject: &str| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits[row_of(app, subject)].oid.clone()
    };
    let head = |root: &Path| {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["log", "-1", "--format=%H %s"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let right_click_row = |app: &mut App, subject: &str| {
        let row = row_of(app, subject);
        let rect = app
            .hits
            .log
            .iter()
            .find(|(_, t)| *t == LogTarget::Row(row))
            .unwrap()
            .0;
        right_click(app, (rect.x + 200.0, rect.y + rect.h / 2.0));
    };
    let refresh = |app: &mut App| {
        app.git_refresh();
        paint(app);
    };

    // 右键一个提交：菜单中的命令作用于该提交，
    // 包括签出、新建分支、拣选、还原、重置（子菜单）与复制。
    right_click_row(&mut app, "补充使用说明");
    let readme = oid_of(&app, "补充使用说明");
    assert_eq!(
        app.context_menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::LogCommit(readme.clone()))
    );
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.selected_oid(), Some(readme.as_str()));
    let menu = &app.context_menu.as_ref().unwrap().entries;
    assert!(
        menu.iter().any(
            |e| matches!(e, MenuEntry::Submenu(title, _) if title == "将「main」重置到此提交")
        )
    );
    assert!(menu_commands(&app).contains(&(MenuCommand::LogResetHard, true)));

    // 还原：新建一次逆向改动的提交，`README.md` 被删除；日志随之刷新。
    app.run_menu_command(MenuCommand::LogRevert);
    assert!(head(&root).contains("Revert"), "{}", head(&root));
    assert!(!root.join("README.md").exists());
    refresh(&mut app);
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert!(view.commits[0].subject.starts_with("Revert"));

    // 拣选 `dev` 上的提交到 `main`。
    right_click_row(&mut app, "配置：加入调试开关");
    app.run_menu_command(MenuCommand::LogCherryPick);
    assert!(
        head(&root).ends_with("配置：加入调试开关"),
        "{}",
        head(&root)
    );
    refresh(&mut app);

    // 硬重置回「启动日志写到文件里」（没有窗口时不弹确认框）。
    let tip = oid_of(&app, "启动日志写到文件里");
    right_click_row(&mut app, "启动日志写到文件里");
    app.run_menu_command(MenuCommand::LogResetHard);
    assert!(head(&root).starts_with(&tip));
    assert!(root.join("README.md").exists());
    refresh(&mut app);

    // 有本地分支指向的提交：签出该分支，不进入分离的 HEAD。
    right_click_row(&mut app, "配置：加入调试开关");
    app.run_menu_command(MenuCommand::LogCheckout);
    refresh(&mut app);
    let current = app
        .git
        .branches
        .iter()
        .find(|b| b.current)
        .map(|b| b.name.clone());
    assert_eq!(current.as_deref(), Some("dev"));

    // 从提交新建分支：对话框里「基于」固定为该提交且不能修改，输入名称后基于该提交创建并签出。
    right_click_row(&mut app, "启动时先加载配置");
    let base = oid_of(&app, "启动时先加载配置");
    app.run_menu_command(MenuCommand::LogNewBranch);
    assert_eq!(app.focus, Focus::Dialog);
    app.edit_field(|f| f.insert("hotfix"));
    let plan = app.form_plan(app.form_dialog.as_ref().unwrap());
    assert_eq!(plan.base, format!("提交 {}", &base[..7]));
    assert!(!plan.base_enabled);
    app.submit_form_dialog();
    refresh(&mut app);
    let current = app
        .git
        .branches
        .iter()
        .find(|b| b.current)
        .map(|b| b.name.clone());
    assert_eq!(current.as_deref(), Some("hotfix"));
    assert!(head(&root).starts_with(&base));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn log_compares_two_commits_or_a_branch_and_the_branch_menu_stashes() {
    use crate::app::git::branch::StashOp;
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::MenuCommand;
    use winc_editor::git::Change;

    let Some((mut app, root)) = git_app("git-compare", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let row_of = |app: &App, subject: &str| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits
            .iter()
            .position(|c| c.subject == subject)
            .unwrap()
    };
    let compare = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .compare
            .clone()
    };

    // 选中「补充使用说明」，⌘ 点击「登录：账号密码表单」：较旧的提交（登录）在左，较新的在右。
    let readme = row_of(&app, "补充使用说明");
    let login = row_of(&app, "登录：账号密码表单");
    app.press_log(LogTarget::Row(readme));
    app.modifiers = command_modifiers();
    app.press_log(LogTarget::Row(login));
    app.modifiers = ModifiersState::empty();
    let c = compare(&app).expect("在比较");
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(c.from, view.commits[login].oid);
    assert_eq!(c.to, view.commits[readme].oid);
    let files = c.files.unwrap();
    let change = |name: &str| {
        files
            .iter()
            .find(|f| f.path.ends_with(name))
            .map(|f| f.change)
    };
    assert_eq!(change("README.md"), Some(Change::Added));
    assert_eq!(change("src/login.rs"), Some(Change::Deleted));

    // 点击文件打开两次提交之间的差异；回到日志按 Esc 退出比较。
    paint(&mut app);
    let i = files
        .iter()
        .position(|f| f.path.ends_with("README.md"))
        .unwrap();
    app.press_log(LogTarget::File(i));
    let doc = app.editor.active().unwrap();
    assert!(doc.title.as_deref().unwrap().contains('…'));
    assert!(doc.diff.is_some());
    app.run_action(Action::GitShowLog);
    app.log_key(&Key::Named(NamedKey::Escape));
    assert!(compare(&app).is_none());

    // 分支菜单「与当前分支比较」：从 `main` 到 `feature/login`。
    let feature = app
        .git
        .branches
        .iter()
        .position(|b| b.name == "feature/login")
        .unwrap();
    let compare_feature =
        MenuCommand::GitBranch(crate::app::git::branch::BranchOp::Compare, feature);
    choose_in_branch_menu(&mut app, compare_feature);
    let c = compare(&app).expect("在比较分支");
    assert_eq!((c.from.as_str(), c.to.as_str()), ("main", "feature/login"));
    assert!(!c.files.unwrap().is_empty());

    // 储藏：修改一个文件、新增一个文件后储藏（附说明），工作区恢复干净；菜单中列出该储藏。
    std::fs::write(root.join("src/main.rs"), "fn main() { changed(); }\n").unwrap();
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    app.git_refresh();
    app.run_branch_command(MenuCommand::GitStash);
    assert_eq!(app.focus, Focus::Prompt);
    app.edit_field(|f| f.insert("试一下新写法"));
    app.submit_prompt();
    app.git_refresh();
    assert_eq!(app.git.stashes.len(), 1);
    assert!(app.git.stashes[0].message.ends_with("试一下新写法"));
    assert!(!root.join("notes.txt").exists());
    assert_ne!(
        std::fs::read_to_string(root.join("src/main.rs")).unwrap(),
        "fn main() { changed(); }\n"
    );
    // 改动列表下方新增储藏分组，展开后列出该储藏。
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.commit_parts().stashes.is_some());
    app.press_commit(CommitTarget::Stashes);
    paint(&mut app);
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Stash(0)));

    // 应用并删除：改动恢复，储藏被移除。
    choose_in_branch_menu(&mut app, MenuCommand::GitStashOp(StashOp::Pop, 0));
    app.git_refresh();
    assert!(app.git.stashes.is_empty());
    assert!(root.join("notes.txt").exists());

    // 再储藏一次（不附说明），之后删除（没有窗口时不弹确认框）。
    app.run_branch_command(MenuCommand::GitStash);
    app.submit_prompt();
    app.git_refresh();
    assert_eq!(app.git.stashes.len(), 1);
    choose_in_branch_menu(&mut app, MenuCommand::GitStashOp(StashOp::Drop, 0));
    app.git_refresh();
    assert!(app.git.stashes.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_log_opens_from_the_commit_view_button_and_the_branch_menu_starts_with_fetch() {
    use crate::app::git::branch::branch_menu;
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-log-entry", false) else {
        return;
    };
    // 提交视图标题行最左边的工具按钮是「Git 日志」，悬停提示带 ⌘9。
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let (rect, target) = app.commit_parts().tools[0];
    assert_eq!(target, CommitTarget::Log);
    click(&mut app, (rect.x + 4.0, rect.y + 4.0));
    assert!(app.log_active());

    // 「⋯」不重复列出日志入口和同步，最上方是储藏；获取在分支选择器里，悬停提示写明作用。
    let menu = branch_menu(&[], None, false);
    assert_eq!(
        menu.first(),
        Some(&MenuEntry::Item(MenuCommand::GitStash, true))
    );
    let label = app.menu_label(MenuCommand::GitFetch);
    assert_eq!(
        label.subtitle.as_deref(),
        Some("下载远程更新，不改动本地分支")
    );
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
    let menu = crate::app::input::context_menu::log_commit_menu(true, "main", &[]);
    for command in [MenuCommand::LogNewTag, MenuCommand::LogCopyShortOid] {
        assert!(
            menu.contains(&MenuEntry::Item(command, true)),
            "{command:?}"
        );
    }
    app.run_log_commit_command(MenuCommand::LogCopyShortOid, &oid);
    assert_eq!(app.copied.as_deref(), Some(&oid[..7]));

    // 输入标签名后回车：标签创建在该提交上。
    app.run_log_commit_command(MenuCommand::LogNewTag, &oid);
    assert_eq!(app.focus, Focus::Prompt);
    paint(&mut app);
    app.edit_field(|f| f.insert("v9 .0"));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "v9.0");
    app.submit_prompt();
    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-list", "-n", "1", "v9.0"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), oid);
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
    assert_eq!(app.hover, Hover::Log(LogTarget::Line(merge, feature)));
    assert_eq!(app.cursor_icon, CursorIcon::Pointer);
    assert!(app.hover.has_tooltip());
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
    let subject = &app.log.runs[0].row;
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
        app.log.details_w = Some(details as f32);
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
