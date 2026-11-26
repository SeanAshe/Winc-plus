//! Git 面板的测试：面板模式下日志在编辑区下方的面板中，与编辑区的标签之间整体移动。

use super::*;
use crate::app::git::log::LogTarget;
use crate::app::git::panel::PanelHit;

/// 面板显示、隐藏过渡的一半和全程（与 `SIDEBAR_ANIMATION` 相同，200 毫秒）。
const SIDEBAR_HALF: std::time::Duration = std::time::Duration::from_millis(100);
const SIDEBAR_FULL: std::time::Duration = std::time::Duration::from_millis(200);

/// 打开一个有历史的仓库，切到面板模式（测试默认是编辑区模式），按 ⌘9 显示面板。
fn panel_app(name: &str, dark: bool) -> Option<(App, PathBuf)> {
    let (mut app, root) = git_app_in(name, false, dark)?;
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    history_repo(&root, 6);
    app.config.git_log_placement = Some(winc_config::LogPlacement::Panel);
    app.git.panel.docked = true;
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    Some((app, root))
}

/// ⌘9 在窗口底部显示 Git 面板，横跨侧栏和编辑区，日志在其中、不占编辑区的标签；焦点在面板上
/// 时再按隐藏，再按显示原来的日志。
#[test]
fn the_log_opens_in_a_panel_below_the_editor() {
    let Some((mut app, root)) = panel_app("git-panel", false) else {
        return;
    };
    assert!(app.git_panel_shown() && app.log_active());
    assert_eq!(app.focus, Focus::GitPanel);
    let (panel, editor, sidebar) = (app.layout.git_panel, app.layout.editor, app.layout.sidebar);
    assert!(panel.h > 0.0 && editor.bottom() < panel.y && sidebar.bottom() < panel.y);
    assert_eq!((panel.x, panel.right()), (sidebar.x, editor.right()));
    assert_eq!(panel.bottom(), app.layout.status.y);
    assert!(!app.editor.documents.iter().any(|d| d.log.is_some()));
    let view = app.log_view().unwrap();
    assert!(view.commits.len() > 3);
    // 日志画在面板里：提交的行在面板中，点击、按键都作用于面板里的日志。
    let (row, _) = *(app.hits.log.iter())
        .find(|(_, t)| *t == LogTarget::Row(2))
        .unwrap();
    assert!(panel.contains(row.x + 4.0, row.y + 4.0));
    app.press_log(LogTarget::Row(2));
    app.log_key(&Key::Named(NamedKey::ArrowDown));
    assert_eq!(app.log_view().unwrap().selected, Some(3));
    // 编辑文字的命令不作用于编辑区的文档。
    assert!(!app.action_available(Action::SelectAll));
    assert!(app.action_available(Action::Copy));

    let id = app.log_doc().unwrap().id;
    app.run_action(Action::GitShowLog);
    assert!(!app.git_panel_shown());
    assert_ne!(app.focus, Focus::GitPanel);
    app.run_action(Action::GitShowLog);
    assert!(app.git_panel_shown());
    assert_eq!(app.log_doc().unwrap().id, id);
    assert_eq!(app.log_view().unwrap().selected, Some(3));
    let _ = std::fs::remove_dir_all(&root);
}

/// 「移至编辑区」把日志变成编辑区的标签、关闭面板；标签栏右端的「移至面板」再移回去，选中的
/// 提交不变。
#[test]
fn the_log_moves_between_the_panel_and_an_editor_tab() {
    let Some((mut app, root)) = panel_app("git-panel-move", false) else {
        return;
    };
    app.press_log(LogTarget::Row(1));
    app.press_git_panel(PanelHit::ToEditor);
    assert!(!app.git.panel.docked && !app.git_panel_shown());
    assert_eq!(app.layout.git_panel.h, 0.0);
    let doc = app.editor.active().unwrap();
    assert!(doc.log.is_some());
    assert_eq!(app.log_view().unwrap().selected, Some(1));
    paint(&mut app);
    assert!(app.layout.terminal_return.w > 0.0);

    app.press_git_panel(PanelHit::ToPanel);
    assert!(app.git.panel.docked && app.git_panel_shown());
    assert!(!app.editor.documents.iter().any(|d| d.log.is_some()));
    assert_eq!(app.log_view().unwrap().selected, Some(1));
    let _ = std::fs::remove_dir_all(&root);
}

/// 文件历史同样在面板中打开，标题行各有一个标签；关闭一个回到另一个，全部关闭时面板隐藏。
#[test]
fn file_histories_open_as_panel_tabs() {
    let Some((mut app, root)) = panel_app("git-panel-history", false) else {
        return;
    };
    app.git_file_history(&root.join("src/main.rs"));
    paint(&mut app);
    assert_eq!(app.git.panel.docs.len(), 2);
    assert_eq!(app.git.panel.at, 1);
    assert!(app.log_view().unwrap().query.path.is_some());
    assert!((app.hits.git_panel.iter()).any(|(_, h)| *h == PanelHit::Tab(1)));
    app.press_git_panel(PanelHit::Tab(0));
    assert!(app.log_view().unwrap().query.path.is_none());
    app.press_git_panel(PanelHit::Close(1));
    assert_eq!(app.git.panel.docs.len(), 1);
    // 焦点在面板上时 ⌘W 关闭面板中的日志，编辑区的标签不受影响。
    app.open_path(&root.join("src/main.rs"));
    let tabs = app.editor.documents.len();
    app.git_file_history(&root.join("src/main.rs"));
    assert_eq!(app.focus, Focus::GitPanel);
    app.run_action(Action::CloseTab);
    assert_eq!(app.git.panel.docs.len(), 1);
    assert_eq!(app.editor.documents.len(), tabs);
    app.press_git_panel(PanelHit::Close(0));
    assert!(!app.git_panel_shown() && app.git.panel.docs.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// 滚轮在面板的提交列表上只滚动面板里的日志；拖动上沿改变面板的高度，双击恢复默认；最大化时
/// 铺满编辑区一栏。
#[test]
fn the_panel_scrolls_resizes_and_maximizes() {
    let Some((mut app, root)) = panel_app("git-panel-size", false) else {
        return;
    };
    // 最矮的面板放不下全部提交。
    app.git.panel.height = Some(crate::app::git::panel::MIN_HEIGHT);
    app.relayout();
    paint(&mut app);
    assert!(app.log_max_scroll() > 0.0);
    let list = app.log_parts().list;
    let at = (list.x + 40.0, list.y + 40.0);
    let target = app.scroll_target_at(at.0, at.1);
    assert_eq!(target, crate::app::scroll::ScrollTarget::GitPanel);
    app.scroll_target_by(target, at, false, (0.0, -60.0));
    assert!(app.log_doc().unwrap().scroll_line > 0.0);

    app.reset_git_panel_height();
    let before = app.layout.git_panel;
    app.drag_git_panel(before.h + 80.0);
    assert_eq!(app.layout.git_panel.h, before.h + 80.0);
    assert_eq!(app.layout.git_panel.bottom(), before.bottom());
    app.reset_git_panel_height();
    assert_eq!(app.layout.git_panel, before);
    // 用鼠标拖动上沿：面板随鼠标移动的距离变高，与按在间距的哪一处无关。
    let divider = app.layout.git_divider;
    let (x, y) = (divider.x + 100.0, divider.y + 1.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y - 50.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.layout.git_panel.h, before.h + 50.0);
    app.reset_git_panel_height();

    app.press_git_panel(PanelHit::Maximize);
    assert_eq!(app.layout.git_panel.y, app.layout.top_bar.bottom());
    app.press_git_panel(PanelHit::Maximize);
    assert_eq!(app.layout.git_panel, before);
    let _ = std::fs::remove_dir_all(&root);
}

/// 最大化的面板盖在侧栏、编辑区和终端上面：其中的悬停、滚轮和右键都归面板里的日志，不落到
/// 下面的文件树或编辑区。
#[test]
fn a_maximized_panel_takes_the_input_over_the_sidebar() {
    use crate::app::input::context_menu::MenuTarget;
    use crate::app::scroll::ScrollTarget;

    let Some((mut app, root)) = panel_app("git-panel-full", false) else {
        return;
    };
    app.press_git_panel(PanelHit::Maximize);
    paint(&mut app);
    // 分支面板在侧栏的位置上。
    let branches = app.log_parts().branches;
    assert!(
        app.layout
            .sidebar
            .contains(branches.x + 20.0, branches.y + 20.0)
    );
    let at = (branches.x + 20.0, branches.y + 60.0);
    app.on_mouse_move(at.0, at.1);
    assert!(matches!(
        app.pointer.hover,
        Hover::Log(_) | Hover::GitPanel(_)
    ));
    assert_eq!(app.scroll_target_at(at.0, at.1), ScrollTarget::LogBranches);
    // 分支面板的一行在文件树的位置上：右键是分支的菜单，不是文件树的菜单。
    let sidebar = app.layout.sidebar;
    let (row, _) = *(app.hits.log.iter())
        .find(|(r, t)| {
            let (x, y) = (r.x + 20.0, r.y + r.h / 2.0);
            matches!(t, LogTarget::BranchRow(_))
                && sidebar.contains(x, y)
                && app.log_branch_context_target(x, y).is_some()
        })
        .unwrap();
    assert!(app.layout.tree.contains(row.x + 20.0, row.y + row.h / 2.0));
    right_click(&mut app, (row.x + 20.0, row.y + row.h / 2.0));
    let menu = app.overlays.context_menu.as_ref().map(|m| m.target.clone());
    assert!(matches!(menu, Some(MenuTarget::BranchOps(_))), "{menu:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 日志是某个窗格仅有的标签时，移到面板以后该窗格随之去掉。
#[test]
fn moving_a_split_log_to_the_panel_drops_its_pane() {
    let Some((mut app, root)) = panel_app("git-panel-split", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.press_git_panel(PanelHit::ToEditor);
    app.run_action(Action::MoveToSplitRight);
    assert!(app.editor.panes.is_split());
    assert!(app.editor.active().unwrap().log.is_some());
    app.press_git_panel(PanelHit::ToPanel);
    assert!(!app.editor.panes.is_split());
    assert!(app.git_panel_shown());
    assert!(!app.editor.documents.iter().any(|d| d.log.is_some()));
    let _ = std::fs::remove_dir_all(&root);
}

/// 在筛选输入框里输入时关掉另一个日志：输入框里的文字不变。
#[test]
fn closing_another_log_keeps_the_filter_being_typed() {
    let Some((mut app, root)) = panel_app("git-panel-filter", false) else {
        return;
    };
    app.git_file_history(&root.join("src/main.rs"));
    app.press_git_panel(PanelHit::Tab(0));
    app.focus = Focus::LogFilter;
    app.git.log.filter.set_text("登录");
    app.press_git_panel(PanelHit::Close(1));
    assert_eq!(app.git.log.filter.text(), "登录");
    assert_eq!(app.focus, Focus::LogFilter);
    let _ = std::fs::remove_dir_all(&root);
}

/// 设置里的「日志位置」：改为编辑区时已打开的日志变成编辑区的标签，并写入配置；面板上的
/// 「移至面板」改的是同一项；启动时按配置决定日志在哪里。
#[test]
fn the_log_placement_setting_moves_the_logs_and_is_remembered() {
    use winc_config::LogPlacement;

    let Some((mut app, root)) = panel_app("git-panel-setting", false) else {
        return;
    };
    app.set_log_placement(LogPlacement::Editor);
    assert!(!app.git.panel.docked && !app.git_panel_shown());
    assert!(app.editor.active().unwrap().log.is_some());
    app.press_git_panel(PanelHit::ToPanel);
    assert_eq!(app.config.git_log_placement, Some(LogPlacement::Panel));
    assert!(app.git_panel_shown());
    // 另一个窗口改了配置文件：本窗口随之移动日志。
    let mut config = app.config.clone();
    config.git_log_placement = Some(LogPlacement::Editor);
    app.adopt_config(config);
    assert!(!app.git.panel.docked);
    // 另一个窗口改回面板：日志移回面板，但不抢本窗口的焦点。
    app.focus = Focus::Editor;
    let mut config = app.config.clone();
    config.git_log_placement = Some(LogPlacement::Panel);
    app.adopt_config(config);
    assert!(app.git_panel_shown());
    assert_eq!(app.focus, Focus::Editor);
    app.set_log_placement(LogPlacement::Editor);
    assert!(app.editor.active().unwrap().log.is_some());

    let config = Config {
        git_log_placement: Some(LogPlacement::Editor),
        ..Config::default()
    };
    let fresh = App::new(Editor::default(), None, Bench::from_env(), config);
    assert!(!fresh.git.panel.docked);
    let fresh = App::new(
        Editor::default(),
        None,
        Bench::from_env(),
        Config::default(),
    );
    assert!(fresh.git.panel.docked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 状态栏右端在终端开关左边有 Git 面板的开关：点击显示、隐藏面板；日志在编辑区时没有该开关。
#[test]
fn the_status_bar_toggles_the_git_panel() {
    use crate::app::StatusItem;

    let Some((mut app, root)) = panel_app("git-panel-status", false) else {
        return;
    };
    paint(&mut app);
    let has = |app: &App| (app.hits.status.iter()).any(|(_, i)| *i == StatusItem::GitPanel);
    assert!(has(&app));
    app.open_status_menu(StatusItem::GitPanel);
    assert!(!app.git_panel_shown());
    app.open_status_menu(StatusItem::GitPanel);
    assert!(app.git_panel_shown());
    assert_eq!(app.focus, Focus::GitPanel);
    assert!(Hover::Status(StatusItem::GitPanel).has_tooltip());

    app.set_log_placement(winc_config::LogPlacement::Editor);
    app.settle_animations();
    paint(&mut app);
    assert!(!has(&app));
    let _ = std::fs::remove_dir_all(&root);
}

/// 显示、隐藏面板有过渡：途中面板从底部滑上来，编辑区随露出的高度变矮，上沿不能拖；隐藏的
/// 过渡中仍然画着但不响应点击；过渡结束后清除。
#[test]
fn the_panel_slides_in_and_out() {
    use crate::app::animation::Clock;

    let Some((mut app, root)) = panel_app("git-panel-slide", false) else {
        return;
    };
    let (full, editor) = (app.layout.git_panel, app.layout.editor);
    app.hide_git_panel();
    let hidden = app.layout.editor;
    assert!(hidden.h > editor.h);

    app.run_action(Action::GitShowLog);
    let half = Clock::frozen(SIDEBAR_HALF, SIDEBAR_FULL);
    app.git.panel.motion = Some((0.0, half));
    app.relayout();
    let middle = app.layout.git_panel;
    assert_eq!(middle.h, full.h);
    assert!(
        full.y < middle.y && middle.y < app.layout.status.y,
        "{middle:?}"
    );
    assert!(editor.h < app.layout.editor.h && app.layout.editor.h < hidden.h);
    assert_eq!(app.layout.git_divider.h, 0.0);
    assert!(app.animating());

    app.git.panel.motion = Some((0.0, Clock::frozen(SIDEBAR_FULL, SIDEBAR_FULL)));
    app.settle_animations();
    assert!(app.git.panel.motion.is_none());
    app.relayout();
    assert_eq!(app.layout.git_panel, full);

    // 隐藏途中：画着，但点击不落在面板上。
    app.hide_git_panel();
    app.git.panel.motion = Some((1.0, half));
    app.relayout();
    paint(&mut app);
    assert!(app.layout.git_panel.h > 0.0);
    assert!(app.hits.git_panel.is_empty() && app.hits.log.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// 显示面板时提交视图中展开着的提交历史收起，隐藏时恢复；期间手动展开、收起过就不再自动恢复。
#[test]
fn opening_the_panel_folds_the_commit_history() {
    use crate::app::HistoryTarget;

    let Some((mut app, root)) = panel_app("git-panel-history-fold", false) else {
        return;
    };
    app.config.git_commit_history = Some(true);
    app.hide_git_panel();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.git.commit.history.drawer.open);

    app.run_action(Action::GitShowLog);
    assert!(app.git_panel_shown());
    assert!(!app.git.commit.history.drawer.open);
    app.hide_git_panel();
    assert!(app.git.commit.history.drawer.open);

    // 面板开着时手动展开：隐藏面板后保持展开，不再动它。
    app.run_action(Action::GitShowLog);
    app.press_commit(CommitTarget::History(HistoryTarget::Title));
    assert!(app.git.commit.history.drawer.open);
    app.hide_git_panel();
    assert!(app.git.commit.history.drawer.open);
    let _ = std::fs::remove_dir_all(&root);
}

/// 焦点不在面板上时，编辑区的后退、前进不被面板中的日志接走；焦点在面板上时 `⌘W` 关的是面板
/// 中的日志，没有编辑区标签时也不改为关闭窗口。
#[test]
fn editor_keys_stay_with_the_editor_unless_the_panel_has_focus() {
    let Some((mut app, root)) = panel_app("git-panel-keys", false) else {
        return;
    };
    let oid = app.log_view().unwrap().commits[2].oid.clone();
    app.git.log.back.push(oid);
    app.focus = Focus::Editor;
    assert!(!app.log_navigate(false));
    assert_eq!(app.git.log.back.len(), 1);
    app.focus = Focus::GitPanel;
    assert!(app.log_navigate(false));

    assert!(app.editor.documents.is_empty());
    assert_eq!(app.keyboard_action(Action::CloseTab), Action::CloseTab);
    app.focus = Focus::LogFilter;
    app.run_action(Action::CloseTab);
    assert!(app.git.panel.docs.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// 关闭目录时面板中的日志一同关闭，面板隐藏。
#[test]
fn closing_the_folder_closes_the_panel_logs() {
    let Some((mut app, root)) = panel_app("git-panel-close-folder", false) else {
        return;
    };
    app.git_file_history(&root.join("src/main.rs"));
    assert_eq!(app.git.panel.docs.len(), 2);
    app.close_folder_now();
    assert!(app.git.panel.docs.is_empty());
    assert!(!app.git_panel_shown() && !app.git.panel.open);
    let _ = std::fs::remove_dir_all(&root);
}

/// 最大化的面板盖住编辑区：编辑区中冲突标记行的按钮等在面板下面，点击落在面板里的日志上。
#[test]
fn a_maximized_panel_covers_the_editor_buttons() {
    use crate::app::git::merge::MergeTarget;

    let Some((mut app, root)) = panel_app("git-panel-cover", false) else {
        return;
    };
    app.press_git_panel(PanelHit::Maximize);
    paint(&mut app);
    let (row, _) = *(app.hits.log.iter())
        .find(|(_, t)| *t == LogTarget::Row(1))
        .unwrap();
    // 编辑区里画在鼠标所在处的冲突按钮（面板之前绘制，记在命中表里）。
    let (x, y) = (row.x + 4.0, row.y + row.h / 2.0);
    let pick = MergeTarget::Pick(0, winc_editor::git::merge::Pick::Ours);
    app.hits
        .merge
        .push((Rect::new(x - 4.0, y - 4.0, 8.0, 8.0), pick));
    app.on_mouse_move(x, y);
    assert!(
        matches!(app.pointer.hover, Hover::Log(_)),
        "{:?}",
        app.pointer.hover
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 面板中的标签放不下时，显示着的那个总在标题行里，可以点到。
#[test]
fn the_shown_panel_tab_stays_visible_when_tabs_overflow() {
    let Some((mut app, root)) = panel_app("git-panel-tabs", false) else {
        return;
    };
    for i in 0..14 {
        let name = format!("a-rather-long-module-name-number-{i}.rs");
        app.git_file_history(&root.join(name));
    }
    paint(&mut app);
    let at = app.git.panel.at;
    assert_eq!(at, app.git.panel.docs.len() - 1);
    let tab = (app.hits.git_panel.iter()).find(|(_, h)| *h == PanelHit::Tab(at));
    let (rect, _) = tab.unwrap();
    assert!(app.layout.git_panel.contains(rect.x + 4.0, rect.y + 4.0));
    assert!(!(app.hits.git_panel.iter()).any(|(_, h)| *h == PanelHit::Tab(0)));
    let _ = std::fs::remove_dir_all(&root);
}

/// 在提交视图的提交历史里「在日志中显示」：面板模式下在面板中选中该提交。
#[test]
fn show_in_log_selects_the_commit_in_the_panel() {
    let Some((mut app, root)) = panel_app("git-panel-show", false) else {
        return;
    };
    app.hide_git_panel();
    let oid = app.log_view().unwrap().commits[3].oid.clone();
    app.git_show_log_at(&oid);
    paint(&mut app);
    assert!(app.git_panel_shown());
    assert_eq!(app.log_view().unwrap().selected_oid(), Some(oid.as_str()));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：窗口底部的 Git 面板（`git-panel`、`git-panel-dark`），选中一个提交，打开着一个
/// 文件历史的标签；状态栏的面板开关（`git-panel-status`）；设置中的「日志位置」
/// （`git-panel-settings`）。
///
/// 运行：`WINC_SNAPSHOT_DIR=… cargo test -p winc-app shoot_git_panel -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_git_panel() {
    for dark in [false, true] {
        let name = if dark { "git-panel-dark" } else { "git-panel" };
        let Some((mut app, root)) = panel_app(name, dark) else {
            return;
        };
        app.open_path(&root.join("src/main.rs"));
        app.git_file_history(&root.join("src/main.rs"));
        app.press_git_panel(PanelHit::Tab(0));
        app.change_log_query(|q| q.branches.clear());
        paint(&mut app);
        app.press_log(LogTarget::Row(2));
        paint(&mut app);
        shoot(&mut app, name);
        if !dark {
            // 状态栏右端的 Git 面板开关和它的提示。
            let item = crate::app::StatusItem::GitPanel;
            let (pill, _) = *(app.hits.status.iter()).find(|(_, i)| *i == item).unwrap();
            app.on_mouse_move(pill.x + 4.0, pill.y + 4.0);
            app.overlays.tooltip.since = Some(
                std::time::Instant::now()
                    - crate::theme::TOOLTIP_DELAY
                    - crate::app::animation::TOOLTIP_FADE,
            );
            shoot(&mut app, "git-panel-status");
            // 设置 → Git 中的「日志位置」。
            app.open_settings();
            app.settings_show_page(crate::layout::SettingsPage::Git);
            app.settle_animations();
            shoot(&mut app, "git-panel-settings");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
