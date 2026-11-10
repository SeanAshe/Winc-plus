//! 侧栏全局搜索面板与查找条行筛选的测试：搜索与结果行、单个与全部替换、「只显示匹配的
//! 行」、结果菜单、滚动条，以及悬停与按下的目标一致。

use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::keymap::{Action, Context};

#[test]
fn long_queries_and_replacements_grow_their_boxes() {
    let dir = search_folder("global-grow");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.set_sidebar_view(SidebarView::Search);
    app.global.replace_open = true;
    settle_search(&mut app);
    let one = app.panel_parts();
    assert_eq!(one.query.h, crate::layout::PANEL_FIELD);

    // 长文字折成多行，框变高，下方的行跟着下移；开关留在第一行。
    let long = "word ".repeat(60);
    app.global.query.insert(&long);
    app.global.replace.insert(&long);
    settle_search(&mut app);
    let tall = app.panel_parts();
    assert!(tall.query.h > one.query.h);
    assert!(tall.replace.h > one.replace.h);
    assert_eq!(
        tall.replace.y,
        tall.query.bottom() + (one.replace.y - one.query.bottom())
    );
    assert_eq!(tall.case_toggle.y, one.case_toggle.y);
    assert!(tall.results.y > one.results.y);

    // 高度有上限，更长的文字在框内滚动。
    app.global.query.insert(&long.repeat(20));
    settle_search(&mut app);
    let capped = app.panel_parts();
    assert_eq!(capped.query.h, crate::layout::PANEL_FIELD + 5.0 * 20.0);

    // 点第二行的文字：光标落在第二行，不是第一行。
    let parts = app.panel_parts();
    app.on_mouse_move(parts.query.x + 60.0, parts.query.y + 40.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.focus, Focus::GlobalQuery);
    paint(&mut app);
    assert!(app.global.query.caret() > 0);

    // 清空后恢复一行。
    app.global.query = TextField::default();
    app.global.replace = TextField::default();
    settle_search(&mut app);
    assert_eq!(app.panel_parts().query.h, one.query.h);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 绘制到输入框高度的过渡结束。
fn settle_search(app: &mut App) {
    paint(app);
    app.global.field_motion = None;
    paint(app);
}

#[test]
fn the_exclude_field_toggles_searching_ignored_files() {
    let dir = search_folder("global-ignored");
    std::fs::write(dir.join(".gitignore"), "dist/\n").unwrap();
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(dir.join("dist/out.js"), "ignoredneedle\n").unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.set_sidebar_view(SidebarView::Search);
    app.global.details_open = true;
    app.global.query.insert("ignoredneedle");
    app.global_changed(true);
    app.global.engine.wait();
    paint(&mut app);
    assert!(app.no_search_results());

    // 默认不搜索被忽略的文件；点「排除」输入框右端的开关后能搜到，再点一次恢复。
    let toggle = app.panel_parts().ignored_toggle;
    assert!(toggle.w > 0.0);
    let at = (toggle.x + 4.0, toggle.y + 4.0);
    app.on_mouse_move(at.0, at.1);
    assert_eq!(app.hover, Hover::Panel(PanelButton::Ignored));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.global.include_ignored);
    app.global.engine.wait();
    paint(&mut app);
    assert!(!app.no_search_results());

    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(!app.global.include_ignored);
    app.global.engine.wait();
    paint(&mut app);
    assert!(app.no_search_results());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn global_search_finds_opens_and_replaces() {
    let dir = search_folder("global");
    let mut app = app_with(&dir, false);
    paint(&mut app);

    // 点击侧栏顶部的「搜索」：侧栏切换为搜索面板，焦点在查找输入框，文件树高度变为 0。
    let (search_tab, _) = app.sidebar_tabs()[1];
    app.on_mouse_move(search_tab.x + 4.0, search_tab.y + 4.0);
    assert_eq!(app.hover, Hover::ViewTab(SidebarView::Search));
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.sidebar.view, SidebarView::Search);
    assert_eq!(app.focus, Focus::GlobalQuery);
    assert_eq!(app.layout.tree.h, 0.0);

    // 输入后延迟一段时间才搜索（`due` 记录触发时间）；回车（这里直接调用）立即搜索。
    app.edit_field(|f| f.insert("config"));
    assert!(app.global.due.is_some());
    app.run_global_search();
    finish_search(&mut app);
    let files: Vec<_> = app
        .global
        .engine
        .results
        .iter()
        .map(|f| f.relative.as_str())
        .collect();
    assert_eq!(files, ["Cargo.toml", "src/config.rs", "src/main.rs"]);
    assert_eq!(app.global.engine.total(), 8);
    assert_eq!(app.global.rows().len(), 3 + 8);

    // 点击文件行收起其下的匹配，再次点击展开。
    let file_row = app.result_row_rect(0);
    app.on_mouse_move(file_row.x + 40.0, file_row.y + 10.0);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.global.rows().len(), 3 + 7);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.global.rows().len(), 3 + 8);

    // 点击一处匹配：打开文件并选中该匹配，焦点仍在结果列表。
    let index = app
        .global
        .rows()
        .iter()
        .position(|&r| r == ResultRow::Match(1, 1))
        .unwrap();
    let row = app.result_row_rect(index);
    app.on_mouse_move(row.x + 60.0, row.y + 10.0);
    assert_eq!(app.hover, Hover::Result(index));
    app.on_mouse_down(MouseButton::Left);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.name(), "config.rs");
    let selected = doc
        .text_range(doc.selection.from(), doc.selection.to())
        .unwrap();
    assert_eq!(selected, "config");
    assert_eq!(doc.text().line_of(doc.selection.head), 2);
    assert_eq!(app.focus, Focus::Results);

    // F4 跳到下一处匹配。
    app.next_result(false);
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.text_range(doc.selection.from(), doc.selection.to())
            .unwrap(),
        "Config"
    );

    // 从结果中移除 `Cargo.toml`（不改动文件），再全部替换：未打开的文件直接写盘，
    // 已打开的文件在文档中修改。
    let cargo = app.result_row_rect(0);
    paint(&mut app);
    let (_, dismiss) = row_actions(cargo, false);
    app.on_mouse_move(dismiss.x + 4.0, dismiss.y + 4.0);
    assert_eq!(app.hover, Hover::RowAction(0, RowAction::Dismiss));
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.global.engine.results.len(), 2);

    // 弹出确认框后结果发生变化（重新搜索后少了一处）时不替换，等待用户重新确认。
    app.global.replace_open = true;
    app.global.replace.set_text("settings");
    let total = app.global.engine.total();
    app.replace_all_confirmed(total + 1);
    assert_eq!(app.global.engine.total(), total);
    assert!(
        std::fs::read_to_string(dir.join("src/main.rs"))
            .unwrap()
            .contains("config")
    );

    // 没有窗口时不弹系统确认框，直接替换。
    app.ask_replace_all();
    finish_search(&mut app);

    assert_eq!(
        std::fs::read_to_string(dir.join("src/main.rs")).unwrap(),
        "fn main() {\n    let settings = load_settings();\n    println!(\"{settings:?}\");\n}\n"
    );
    assert!(
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .unwrap()
            .contains("config-demo")
    );
    // `config.rs` 已打开：替换发生在文档中，可以撤销；尚未保存，磁盘上的文件不变。
    let config = app.editor.active().unwrap();
    assert!(config.is_dirty());
    assert!(
        config
            .text()
            .substring(0, config.text().end_pos())
            .contains("load_settings")
    );
    assert!(
        std::fs::read_to_string(dir.join("src/config.rs"))
            .unwrap()
            .contains("load_config")
    );
    // 替换后重新搜索：被移除（因而未被替换）的 `Cargo.toml` 仍有匹配，其余文件已没有匹配。
    let left: Vec<_> = app
        .global
        .engine
        .results
        .iter()
        .map(|f| f.relative.as_str())
        .collect();
    assert_eq!(left, ["Cargo.toml"]);

    // 点击「文件」回到文件树，焦点在文件树。
    let (files_tab, _) = app.sidebar_tabs()[0];
    app.on_mouse_move(files_tab.x + 4.0, files_tab.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.sidebar.view, SidebarView::Files);
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.layout.tree.h > 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sidebar_top_buttons_hover_in_both_views() {
    let dir = search_folder("top-hover");
    let mut app = app_with(&dir, false);

    for view in [SidebarView::Files, SidebarView::Search] {
        app.set_sidebar_view(view);
        paint(&mut app);

        // 侧栏开关、两个视图切换按钮和分隔条，在两种视图中都有悬停效果。
        let button = app.layout.sidebar_button;
        app.on_mouse_move(button.x + 4.0, button.y + 4.0);
        assert_eq!(app.hover, Hover::SidebarButton, "{view:?}");

        for (rect, tab) in app.sidebar_tabs() {
            app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
            assert_eq!(app.hover, Hover::ViewTab(tab), "{view:?}");
        }

        let splitter = app.layout.splitter;
        app.on_mouse_move(splitter.x + 1.0, 300.0);
        assert_eq!(app.hover, Hover::Splitter, "{view:?}");
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 悬停和按下识别的是同一目标（`App::target_at`）：在窗口上按网格逐点检查，悬停在分隔条、
/// 滚动条上时按下分别进入拖动分隔条、拖动滚动条。
#[test]
fn hover_and_click_agree_on_what_is_under_the_mouse() {
    let dir = search_folder("hover-click");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));

    for view in [SidebarView::Files, SidebarView::Search] {
        app.set_sidebar_view(view);
        paint(&mut app);
        let window = app.layout.window;
        let mut checked = 0;
        // 网格再加上分隔条那一列：分隔条只有几像素宽，网格未必落在上面。
        let splitter = app.layout.splitter.x + app.layout.splitter.w / 2.0;
        let columns = (0..60)
            .map(|gx| window.x + (gx as f32 + 0.5) * window.w / 60.0)
            .chain([splitter]);
        for x in columns {
            for gy in 0..40 {
                let y = window.y + (gy as f32 + 0.5) * window.h / 40.0;
                app.on_mouse_move(x, y);
                let hover = app.hover;
                let expected = match hover {
                    Hover::Splitter => "splitter",
                    Hover::Scrollbar(_) => "scrollbar",
                    _ => continue,
                };
                app.on_mouse_down(MouseButton::Left);
                let got = match app.drag {
                    crate::app::Drag::Splitter { .. } => "splitter",
                    crate::app::Drag::Scrollbar { .. } => "scrollbar",
                    _ => "other",
                };
                app.on_mouse_up(MouseButton::Left);
                assert_eq!(got, expected, "{view:?} ({x}, {y}) hover {hover:?}");
                checked += 1;
            }
        }
        assert!(checked > 0, "{view:?}");
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn global_search_replaces_one_match_and_filters() {
    let dir = search_folder("global-one");
    let mut app = app_with(&dir, false);
    app.open_global_search(true);
    assert!(app.global.replace_open);
    paint(&mut app);

    // 只搜 `src` 目录，并排除 `main.rs`。
    app.global.details_open = true;
    app.global.include.set_text("src");
    app.global.exclude.set_text("main.rs");
    app.global.query.set_text("Config");
    app.global.case_sensitive = true;
    app.run_global_search();
    finish_search(&mut app);
    let files: Vec<_> = app
        .global
        .engine
        .results
        .iter()
        .map(|f| f.relative.as_str())
        .collect();
    assert_eq!(files, ["src/config.rs"]);
    assert_eq!(app.global.engine.total(), 3);

    // 悬停在一处匹配上并点击「替换」：只替换该处，写回磁盘，之后重新搜索该文件。
    app.global.replace.set_text("Settings");
    paint(&mut app);
    let row = app.result_row_rect(1);
    let (replace, _) = row_actions(row, true);
    let replace = replace.unwrap();
    app.on_mouse_move(replace.x + 4.0, replace.y + 4.0);
    assert_eq!(app.hover, Hover::RowAction(1, RowAction::Replace));
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(
        std::fs::read_to_string(dir.join("src/config.rs")).unwrap(),
        "pub struct Settings;\n\npub fn load_config() -> Config {\n    Config\n}\n"
    );
    assert_eq!(app.global.engine.total(), 2);

    // 替换输入框中按 ↩：先选中下一处匹配（不替换），再按一次才替换该处，焦点留在替换输入框。
    app.focus = Focus::GlobalReplace;
    app.global.selected = None;
    app.replace_from_field();
    assert_eq!(app.focus, Focus::GlobalReplace);
    assert!(
        app.global
            .selected
            .as_ref()
            .is_some_and(|(_, m)| m.is_some())
    );
    assert_eq!(app.global.engine.total(), 2);
    app.replace_from_field();
    assert_eq!(app.global.engine.total(), 1);
    assert_eq!(app.focus, Focus::GlobalReplace);

    // 查找输入框中按 ↩ / ⇧↩：在结果之间跳转，焦点留在查找输入框；搜索条件已改变但尚未搜索时，
    // 先执行搜索。
    app.focus = Focus::GlobalQuery;
    app.step_from_field(false);
    assert_eq!(app.focus, Focus::GlobalQuery);
    app.global.query.set_text("struct");
    app.step_from_field(false);
    assert_eq!(app.global.engine.query().unwrap().text, "struct");

    // 正则表达式有误：统计行显示原因，不执行搜索。
    app.global.regex = true;
    app.global.query.set_text("(");
    app.run_global_search();
    assert!(
        app.global
            .engine
            .error()
            .unwrap()
            .starts_with("正则表达式有误")
    );
    assert!(app.global.rows().is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn showing_only_matching_lines_hides_the_rest_until_turned_off() {
    let dir = std::env::temp_dir().join(format!("winc-filter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.log");
    // 样本：200 行日志，第 7、57、107、157 行（从 0 起）为 `ERROR`，其余为 `INFO`。
    let mut log = String::new();
    for i in 0..200 {
        let level = if i % 50 == 7 { "ERROR" } else { "INFO" };
        log.push_str(&format!("{i:03} {level} message\n"));
    }
    std::fs::write(&file, &log).unwrap();
    let mut app = app_with(&file, false);
    let rows = |app: &App| app.editor.active().unwrap().row_count();

    // ⌘F 搜索 `ERROR` 并开启「只显示匹配的行」：只剩四行。
    app.run_action(Action::Find);
    app.find.input.set_text("ERROR");
    app.search.query = "ERROR".into();
    app.search_changed();
    app.run_action(Action::FilterLines);
    app.advance_search();
    paint(&mut app);
    assert!(app.find.filter_lines);
    assert_eq!(rows(&app), 4);
    let doc = app.editor.active().unwrap();
    let shown: Vec<usize> = (0..4).map(|r| doc.line_of_row(r)).collect();
    assert_eq!(shown, [7, 57, 107, 157]);

    // 修改查询：显示的行随之变化；清空查询：显示全部行（200 行加末尾的空行，共 201 行）。
    app.search.query = "157".into();
    app.search_changed();
    app.advance_search();
    paint(&mut app);
    assert_eq!(rows(&app), 1);
    app.search.query.clear();
    app.search_changed();
    app.advance_search();
    paint(&mut app);
    assert_eq!(rows(&app), 201);

    // 筛选状态下修改文本：等重新搜索完成后再筛选，被修改的行仍有匹配时保持显示。
    app.search.query = "ERROR".into();
    app.search_changed();
    app.advance_search();
    paint(&mut app);
    app.focus = Focus::Editor;
    let end = app.editor.active().unwrap().text().line_end(7);
    app.select(end, end);
    app.run(Command::Insert("!".into()));
    app.advance_search();
    paint(&mut app);
    assert_eq!(rows(&app), 4);

    // 关闭查找条：取消筛选，恢复显示全部行。
    app.close_search();
    app.advance_search();
    paint(&mut app);
    assert!(!app.find.filter_lines);
    assert_eq!(rows(&app), 201);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_filter_that_hides_nothing_leaves_the_scroll_alone() {
    let dir = std::env::temp_dir().join(format!("winc-filter-all-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("all.log");
    // 每一行都含有 `x`，且最后一行之后没有换行符：筛选后没有任何行被隐藏。
    let lines: Vec<String> = (0..200).map(|i| format!("{i:03} x")).collect();
    std::fs::write(&file, lines.join("\n")).unwrap();
    let mut app = app_with(&file, false);

    app.run_action(Action::Find);
    app.find.input.set_text("x");
    app.search.query = "x".into();
    app.search_changed();
    app.run_action(Action::FilterLines);
    app.advance_search();
    paint(&mut app);
    assert!(app.find.filter_lines);
    assert_eq!(app.row_count(), 200);

    // 向下滚动（光标仍在第一处匹配上）：之后的每一帧都不应把视图拉回光标处。
    app.editor.active_mut().unwrap().scroll_line = 100.0;
    app.advance_search();
    paint(&mut app);
    app.advance_search();
    assert_eq!(app.editor.active().unwrap().scroll_line, 100.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_the_last_tab_drops_a_pending_jump() {
    let dir = sample_folder("pending-jump");
    let mut app = app_with(&dir.join("a.txt"), false);
    let id = app.editor.active().unwrap().id;
    // 大文件中按 ⌘G，一个时间片内没有搜到下一处：记录待跳转，搜到后再跳。
    app.find.pending_jump = Some((id, 0, false));
    app.run_action(Action::CloseTab);
    assert!(app.editor.documents.is_empty());
    // 已没有标签，等待中的跳转作废；否则每帧都会视为仍在搜索，导致持续重绘。
    app.advance_search();
    assert!(app.find.pending_jump.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_search_results_scrollbar_can_be_dragged() {
    let dir = search_folder("global-scrollbar");
    let many: String = (0..200).map(|i| format!("let config_{i} = 0;\n")).collect();
    std::fs::write(dir.join("src/many.rs"), many).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    let (search_tab, _) = app.sidebar_tabs()[1];
    app.on_mouse_move(search_tab.x + 4.0, search_tab.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    assert!(app.results_max_scroll() > 0.0);
    let tabs = app.editor.documents.len();

    // 结果行右端就是滚动条：悬停认滚动条，按下拖到底，列表滚到底，没有打开结果。
    let results = app.panel_parts().results;
    let x = results.right() - 3.0;
    app.on_mouse_move(x, results.y + 10.0);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::Panel));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, results.bottom() + 40.0);
    assert_eq!(app.global.scroll, app.results_max_scroll());
    app.on_mouse_move(x, results.y - 40.0);
    assert_eq!(app.global.scroll, 0.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.editor.documents.len(), tabs);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_result_dismiss_hover() {
    for dark in [false, true] {
        let name = if dark {
            "dismiss-dark"
        } else {
            "dismiss-light"
        };
        let dir = search_folder(name);
        let mut app = app_with(&dir, dark);
        paint(&mut app);
        app.open_global_search(false);
        app.edit_field(|f| f.insert("config"));
        app.run_global_search();
        finish_search(&mut app);
        let (_, dismiss) = row_actions(app.result_row_rect(1), false);
        app.on_mouse_move(dismiss.x + 4.0, dismiss.y + 4.0);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn switching_folders_keeps_the_search_view_and_searches_the_new_folder() {
    let first = search_folder("switch-keep-a");
    let second = search_folder("switch-keep-b");
    let mut app = app_with(&first, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);

    // 换目录：侧栏还在「搜索」，查的还是「config」，结果是新目录里的。
    app.switch_folder(&second);
    assert_eq!(
        app.editor.root(),
        Some(winc_config::files::canonicalize(&second).unwrap().as_path())
    );
    assert_eq!(app.sidebar.view, SidebarView::Search);
    assert_eq!(app.global.query.text(), "config");
    finish_search(&mut app);
    let root = winc_config::files::canonicalize(&second).unwrap();
    assert!(!app.global.engine.results.is_empty());
    assert!(
        app.global
            .engine
            .results
            .iter()
            .all(|r| r.path.starts_with(&root))
    );
    let _ = std::fs::remove_dir_all(&first);
    let _ = std::fs::remove_dir_all(&second);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_search_no_results() {
    for dark in [false, true] {
        let name = if dark {
            "noresult-dark"
        } else {
            "noresult-light"
        };
        let dir = search_folder(name);
        let mut app = app_with(&dir, dark);
        paint(&mut app);
        app.open_global_search(false);
        app.edit_field(|f| f.insert("1222"));
        app.run_global_search();
        finish_search(&mut app);
        assert!(app.no_search_results());
        assert_eq!(app.panel_parts().summary.h, 0.0);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn right_clicking_a_search_result_offers_its_menu() {
    use MenuCommand::{
        Copy, CopyPath, CopyRelativePath, DismissResult, OpenFiles, ReplaceResult, RevealInTree,
    };

    let dir = search_folder("result-menu");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let main = root.join("src/main.rs");
    let row_of = |app: &App, row: ResultRow| app.global.rows().iter().position(|&r| r == row);
    let click_row = |app: &mut App, index: usize| {
        let row = app.result_row_rect(index);
        right_click(app, (row.x + 60.0, row.y + row.h / 2.0));
    };
    let commands = |app: &App| -> Vec<MenuCommand> {
        menu_commands(app).into_iter().map(|(c, _)| c).collect()
    };

    // 文件行：选中它、焦点给结果列表；打开、复制路径、在文件树中显示、从结果中移除。
    let file = row_of(&app, ResultRow::File(2)).unwrap();
    click_row(&mut app, file);
    assert_eq!(
        app.context_menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::SearchResult(main.clone(), None))
    );
    assert_eq!(app.global.selected_row(), Some(file));
    assert_eq!(app.focus, Focus::Results);
    assert_eq!(
        commands(&app),
        [
            OpenFiles,
            CopyPath,
            CopyRelativePath,
            RevealInTree,
            DismissResult
        ]
    );
    app.run_menu_command(CopyRelativePath);
    // 相对路径使用当前系统的路径分隔符。
    let relative = if cfg!(windows) {
        r"src\main.rs"
    } else {
        "src/main.rs"
    };
    assert_eq!(app.copied.as_deref(), Some(relative));
    // 焦点在结果列表里时，命令面板的「复制路径」复制的也是选中的结果。macOS 上菜单不写 ⌥⌘C：
    // 在这里它是「区分大小写」开关；其他系统上开关是 Alt+C，菜单上仍写出 Ctrl+Alt+C。
    let in_panel = MenuCommand::CopyPath.shortcut(Context::Panel);
    assert_eq!(in_panel.is_none(), cfg!(target_os = "macos"));
    assert!(MenuCommand::CopyPath.shortcut(Context::Tree).is_some());
    app.run_action(Action::CopyPath);
    // 按路径比较：Windows 上复制的路径一律用 `\`。
    assert_eq!(app.copied.as_deref().map(Path::new), Some(main.as_path()));

    // 匹配行：复制的是原来的整行（预览去掉了行首空白），不带行尾。
    let found = row_of(&app, ResultRow::Match(2, 0)).unwrap();
    click_row(&mut app, found);
    assert_eq!(commands(&app), [OpenFiles, Copy, DismissResult]);
    app.run_menu_command(Copy);
    assert_eq!(
        app.copied.as_deref(),
        Some("    let config = load_config();")
    );

    // 打开：打开文件并选中这一处，焦点到编辑区。
    click_row(&mut app, found);
    app.run_menu_command(OpenFiles);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.path.as_deref(), Some(main.as_path()));
    assert_eq!(
        doc.text_range(doc.selection.from(), doc.selection.to()),
        Some("config".to_owned())
    );
    assert_eq!(app.focus, Focus::Editor);

    // 「从结果中移除」只修改结果列表，不修改文件，效果与行上的 ✕ 按钮相同。
    let total = app.global.engine.total();
    click_row(&mut app, found);
    app.run_menu_command(DismissResult);
    assert_eq!(app.global.engine.total(), total - 1);
    assert!(
        std::fs::read_to_string(&main)
            .unwrap()
            .contains("let config")
    );

    // 展开替换以后多出「替换」（匹配行）和「全部替换」（文件行，只换这个文件）。
    app.press_panel_button(PanelButton::Expand);
    app.global.replace.set_text("settings");
    paint(&mut app);
    let config = root.join("src/config.rs");
    let found = row_of(&app, ResultRow::Match(1, 0)).unwrap();
    click_row(&mut app, found);
    assert_eq!(
        commands(&app),
        [OpenFiles, Copy, ReplaceResult(false), DismissResult]
    );
    let file = row_of(&app, ResultRow::File(1)).unwrap();
    click_row(&mut app, file);
    assert_eq!(
        commands(&app),
        [
            OpenFiles,
            CopyPath,
            CopyRelativePath,
            RevealInTree,
            ReplaceResult(true),
            DismissResult
        ]
    );
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    app.run_menu_command(ReplaceResult(true));
    assert!(!std::fs::read_to_string(&config).unwrap().contains("onfig"));
    assert_eq!(
        std::fs::read_to_string(root.join("Cargo.toml")).unwrap(),
        cargo
    );

    // 在文件树中显示：切回文件树，选中这个文件。
    let file = row_of(&app, ResultRow::File(0)).unwrap();
    click_row(&mut app, file);
    app.run_menu_command(RevealInTree);
    assert_eq!(app.sidebar.view, SidebarView::Files);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(
        ws.selected_entry().map(|e| e.path.clone()),
        Some(root.join("Cargo.toml"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_result_row_ends() {
    // 结果行右端：文件行的匹配数（一位、两位）、匹配行的行号（一到三位）；鼠标分别停在文件行、
    // 匹配行上，替换收起和展开各一张。
    for dark in [false, true] {
        let tag = if dark { "dark" } else { "light" };
        let dir = search_folder(&format!("row-ends-{tag}"));
        let long: String = (1..=130)
            .map(|n| match n {
                7 | 42 | 118 => "    let config = 1;\n".to_owned(),
                _ => format!("// {n}\n"),
            })
            .collect();
        std::fs::write(dir.join("src/long.rs"), long).unwrap();
        let many = "config\n".repeat(12);
        std::fs::write(dir.join("src/many.rs"), many).unwrap();
        let mut app = app_with(&dir, dark);
        paint(&mut app);
        app.open_global_search(false);
        app.edit_field(|f| f.insert("config"));
        app.run_global_search();
        finish_search(&mut app);
        app.global.collapsed.insert(
            winc_config::files::canonicalize(&dir)
                .unwrap()
                .join("src/many.rs"),
        );
        app.global.rebuild_rows();

        for replace in [false, true] {
            if app.global.replace_open != replace {
                app.press_panel_button(PanelButton::Expand);
                app.global.replace.set_text("settings");
            }
            app.focus = Focus::Results;
            app.global.selected = None;
            paint(&mut app);
            let mode = if replace { "replace" } else { "find" };
            for (what, row) in [
                ("file", ResultRow::File(2)),
                ("match", ResultRow::Match(2, 2)),
            ] {
                let index = app.global.rows().iter().position(|&r| r == row).unwrap();
                let rect = app.result_row_rect(index);
                app.on_mouse_move(rect.x + 60.0, rect.y + rect.h / 2.0);
                shoot(&mut app, &format!("row-ends-{mode}-{what}-{tag}"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn result_keys_jump_to_the_ends_and_page() {
    let dir = search_folder("global-paging");
    let many: String = (0..80).map(|i| format!("let config_{i} = 0;\n")).collect();
    std::fs::write(dir.join("src/many.rs"), many).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    app.focus = Focus::Results;
    let key = |app: &mut App, key: NamedKey| app.results_key(&Key::Named(key));
    let rows = app.global.rows().len();
    let page = app.result_page();
    assert!(rows > 2 * page);

    // `End` 选中最后一行并滚动到可见，`Home` 回到第一行；到达两端后不绕回。
    key(&mut app, NamedKey::End);
    assert_eq!(app.global.selected_row(), Some(rows - 1));
    let last = app.result_row_rect(rows - 1);
    assert!(app.global.scroll > 0.0 && last.bottom() <= app.panel_parts().results.bottom());
    key(&mut app, NamedKey::PageDown);
    assert_eq!(app.global.selected_row(), Some(rows - 1));
    key(&mut app, NamedKey::Home);
    assert_eq!(app.global.selected_row(), Some(0));
    assert_eq!(app.global.scroll, 0.0);
    key(&mut app, NamedKey::PageUp);
    assert_eq!(app.global.selected_row(), Some(0));

    // 每次翻页移动的行数为可见整行数减一。
    key(&mut app, NamedKey::PageDown);
    assert_eq!(app.global.selected_row(), Some(page));
    key(&mut app, NamedKey::PageDown);
    assert_eq!(app.global.selected_row(), Some(2 * page));
    key(&mut app, NamedKey::PageUp);
    assert_eq!(app.global.selected_row(), Some(page));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_and_filter_rows_slide_open() {
    use crate::app::Clock;
    use crate::app::animation::TREE_ANIMATION;

    let dir = search_folder("global-rows");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    let closed = app.panel_parts();
    assert_eq!(closed.replace.h, 0.0);

    // 没有窗口时不播放动画，两行直接完全展开。
    app.global.replace_open = true;
    app.global.details_open = true;
    paint(&mut app);
    let open = app.panel_parts();
    assert!(!app.global.rows_moving());
    // 最下面的输入框获得焦点时外圈完整显示，露出范围不裁掉外圈的底边。
    app.focus = Focus::GlobalExclude;
    shoot(&mut app, "search-rows-open");
    assert_eq!(
        open.replace_band.h,
        open.replace.bottom() - closed.query.bottom()
    );

    // 展开到一半时两行各露出一部分：输入框从上一行下方滑出，下沿与露出部分的下沿对齐；
    // 结果列表位于收起与完全展开时的位置之间。
    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.global.replace_open = false;
    app.global.details_open = false;
    paint(&mut app);
    app.global.replace_open = true;
    app.global.details_open = true;
    app.global.replace_fx.track(true, Some(half));
    app.global.details_fx.track(true, Some(half));
    let parts = app.panel_parts();
    let share = app.global.replace_share();
    assert!(share > 0.0 && share < 1.0);
    assert!(parts.replace_band.h > 0.0 && parts.replace_band.h < open.replace_band.h);
    assert_eq!(parts.replace.bottom(), parts.replace_band.bottom());
    assert_eq!(parts.exclude.bottom(), parts.details_band.bottom());
    assert!(parts.results.y > closed.results.y && parts.results.y < open.results.y);
    assert!(app.animating());
    shoot(&mut app, "search-rows-sliding");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folding_a_result_file_slides_its_matches() {
    use crate::app::Clock;
    use crate::app::animation::TREE_ANIMATION;
    use crate::app::workspace::global::{ResultMotion, ResultRow};

    let dir = search_folder("global-fold");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    let before = app.global.rows().to_vec();
    let results = &app.global.engine.results;
    let f = results.iter().position(|r| r.matches.len() > 1).unwrap();
    let count = results[f].matches.len();
    let parent = before
        .iter()
        .position(|&r| r == ResultRow::File(f))
        .unwrap();

    // 没有窗口时直接完成折叠，不播放动画。
    app.focus = Focus::Results;
    app.global.select_row(parent);
    app.results_key(&Key::Named(NamedKey::ArrowLeft));
    assert_eq!(app.global.rows().len(), before.len() - count);
    assert!(app.global.motion.is_none());

    // 折叠到一半时，匹配行收回一部分，下一个文件行随之上移，箭头转过一部分。
    app.global.motion = Some(ResultMotion {
        rows: before.clone(),
        parent,
        count,
        expanding: false,
        clock: Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION),
    });
    let motion = app.global.motion.as_ref().unwrap();
    let hidden = motion.hidden();
    assert!(hidden > 0.0 && hidden < count as f32 * TREE_ROW);
    assert!(motion.turned() > 0.0 && motion.turned() < 1.0);
    shoot(&mut app, "search-result-folding");

    // 结果行重新生成后，不再绘制旧的动画。
    app.global.rebuild_rows();
    assert!(app.global.motion.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_the_query_clears_the_results_at_once() {
    let dir = search_folder("global-clear");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    assert!(!app.global.rows().is_empty());
    assert!(app.panel_parts().summary.h > 0.0);

    // 删空查找内容时，统计行与结果在同一帧清除，不等待搜索延迟结束。
    app.edit_field(|f| {
        f.select_all();
        f.delete(crate::field::Unit::Char, false)
    });
    assert!(app.global.query.text().is_empty());
    assert!(app.global.due.is_none());
    assert!(app.global.rows().is_empty());
    assert_eq!(app.panel_parts().summary.h, 0.0);

    // 查找内容非空时，仍然等待搜索延迟结束后再搜索。
    app.edit_field(|f| f.insert("c"));
    assert!(app.global.due.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 有焦点的输入框外环在各缩放倍数下四边宽度相同，且至少为 1 个物理像素。
///
/// 样本包括非整数倍数（系统缩放 125%、界面缩放 90%、110%）：按逻辑像素外扩时，四边各自取整
/// 后宽度不同，部分边宽度为 0。
#[test]
fn the_focus_ring_has_even_sides_at_any_scale() {
    let dir = search_folder("focus-ring-scale");
    let mut app = app_with(&dir, true);
    app.open_global_search(false);
    app.press_panel_button(PanelButton::Expand);
    app.focus = Focus::GlobalReplace;
    for scale in [0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.2] {
        let bg = app.palette.frame_bg;
        let renderer = winc_render::Renderer::offscreen(SIZE.0, SIZE.1, scale, bg, None, None);
        app.renderer = Some(renderer.unwrap());
        // 每轮加宽侧栏 0.3 逻辑像素，使输入框的边落在非整数的物理像素上。
        app.sidebar.width += 0.3;
        paint(&mut app);
        let field = app.panel_parts().replace;
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let accent = app.palette.accent;
        let target = [accent.r, accent.g, accent.b].map(|c| (c * 255.0).round() as i32);
        let lit = |x: i32, y: i32| {
            let px = image.pixels[(y * image.width as i32 + x) as usize];
            let rgb = [px >> 16, px >> 8, px].map(|c| (c & 0xFF) as i32);
            (0..3).all(|i| (rgb[i] - target[i]).abs() <= 8)
        };
        let s = |v: f32| (v * scale).round() as i32;
        let (cx, cy) = (s(field.x + field.w / 2.0), s(field.y + field.h / 2.0));
        let count =
            |line: &dyn Fn(i32) -> bool, at: i32| (at - 4..at + 4).filter(|&v| line(v)).count();
        let sides = [
            count(&|x| lit(x, cy), s(field.x)),
            count(&|x| lit(x, cy), s(field.right())),
            count(&|y| lit(cx, y), s(field.y)),
            count(&|y| lit(cx, y), s(field.bottom())),
        ];
        assert!(sides[0] >= 1, "{scale}: {sides:?}");
        assert!(sides.iter().all(|&n| n == sides[0]), "{scale}: {sides:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
