//! 在文件中查找的对话框和底部面板「搜索」标签的测试：搜索、选择、打开、移动和改变大小、
//! 按设置打开、放进底部面板、文件掩码和范围、预览的缓存。

use super::*;
use crate::keymap::{Action, Context};

/// 在文件中查找的对话框：搜索、方向键选择、回车打开并选中匹配；拖动空白处移动、拖动边缘改变
/// 大小；点编辑区不关闭，Esc 关闭。
#[test]
fn the_search_dialog_finds_moves_and_stays_open() {
    let dir = search_folder("search-dialog");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);
    let search = |app: &mut App, text: &str| {
        app.search_dialog.query.set_text(text);
        app.search_dialog_changed(true);
        app.search_dialog.engine.wait();
        app.poll_search_dialog(Instant::now());
        paint(app);
        app.search_dialog.hits.len()
    };

    app.open_search_dialog();
    assert!(app.search_dialog.open);
    assert_eq!(app.focus, Focus::SearchDialog);
    assert_eq!(app.key_context(), Context::Panel);
    assert!(search(&mut app, "load_config") >= 2);
    app.toggle_search_dialog_option(crate::layout::PanelButton::Regex);
    assert!(search(&mut app, "load_conf.g") >= 2);
    app.toggle_search_dialog_option(crate::layout::PanelButton::Regex);
    assert_eq!(search(&mut app, "load_conf.g"), 0);
    assert!(search(&mut app, "load_config") >= 2);

    // 拖动标题行的空白处移动；拖动右下角改变大小，不小于最小大小。
    let parts = app.search_dialog_parts();
    let (x, y) = (parts.header.x + 300.0, parts.header.y + 14.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - 40.0, y + 30.0);
    app.on_mouse_up(MouseButton::Left);
    let moved = app.search_dialog_rect();
    assert_eq!(
        (moved.x, moved.y),
        (parts.card.x - 40.0, parts.card.y + 30.0)
    );
    let corner = (moved.right() - 2.0, moved.bottom() - 2.0);
    app.on_mouse_move(corner.0, corner.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(corner.0 - 2000.0, corner.1 - 2000.0);
    app.on_mouse_up(MouseButton::Left);
    let small = app.search_dialog_rect();
    assert_eq!((small.x, small.y), (moved.x, moved.y));
    assert!(small.w >= 480.0 && small.h >= 360.0 && small.w < moved.w);

    // 点对话框以外的编辑区：焦点离开，对话框不关闭。
    let editor = app.layout.editor;
    super::click(&mut app, (editor.right() - 20.0, editor.bottom() - 20.0));
    assert!(app.search_dialog.open);
    assert_ne!(app.focus, Focus::SearchDialog);

    // 再打开：沿用位置；回车打开选中的一处并选中匹配，对话框关闭。
    app.open_search_dialog();
    assert_eq!(app.search_dialog_rect(), small);
    app.search_dialog_select(1);
    app.search_dialog_accept();
    paint(&mut app);
    paint(&mut app);
    assert!(!app.search_dialog.open);
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.selection.head.abs_diff(doc.selection.anchor),
        "load_config".len()
    );

    // Esc 关闭。
    app.open_search_dialog();
    app.close_search_dialog();
    assert!(!app.search_dialog.open);
    assert_eq!(app.focus, Focus::Editor);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 「在文件中查找」按设置打开侧栏或对话框；没有设置时按键位方案。
#[test]
fn find_in_files_follows_the_search_style() {
    use winc_config::SearchStyle;
    let dir = search_folder("find-in-files-style");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.config.keymap = Some("vscode".into());
    assert_eq!(app.search_style(), SearchStyle::Sidebar);
    app.config.keymap = Some("idea".into());
    assert_eq!(app.search_style(), SearchStyle::Dialog);

    app.config.find_in_files = Some(SearchStyle::Sidebar);
    app.run_action(Action::FindInFiles);
    assert!(!app.search_dialog.open);
    assert_eq!(app.focus, Focus::GlobalQuery);
    app.config.find_in_files = Some(SearchStyle::Dialog);
    app.run_action(Action::FindInFiles);
    assert!(app.search_dialog.open);
    assert_eq!(app.focus, Focus::SearchDialog);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 「在底部显示」把结果放进底部面板的「搜索」标签：对话框关闭，结果和选中项留着；方向键选择、
/// 回车打开，结果仍在；关闭标签后面板里没有终端时一起收起。
#[test]
fn search_results_move_to_the_bottom_panel() {
    let dir = search_folder("search-dock");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("load_config");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    paint(&mut app);
    let count = app.search_dialog.hits.len();
    assert!(count >= 2);

    app.dock_search_results();
    paint(&mut app);
    assert!(!app.search_dialog.open);
    assert!(app.docked_search_shown() && app.terminal.open);
    assert_eq!(app.focus, Focus::SearchDock);
    assert_eq!(app.search_dialog.hits.len(), count);
    let dock = app.docked_parts().unwrap();
    let row = dock.list.row(1, app.search_dialog.scroll);
    assert_eq!(
        app.docked_part_at(row.x + 40.0, row.y + 4.0),
        Some(crate::app::workspace::search_dialog::DialogPart::Row(1))
    );

    // 选中第二条并打开：文件打开、匹配选中，结果仍在面板里。
    app.search_dialog_select(1);
    app.search_dialog_accept();
    paint(&mut app);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.selection.head.abs_diff(doc.selection.anchor),
        "load_config".len()
    );
    assert!(app.docked_search_shown());

    // 「在对话框中打开」：查询不变，选中的一行在对话框的列表中可见。
    use crate::app::workspace::search_dialog::DialogPart;
    app.search_dialog.scroll = 500.0;
    app.open_search_dialog_with(None);
    assert!(app.search_dialog.open);
    assert_eq!(app.search_dialog.query.text(), "load_config");
    let list = app.search_dialog_parts().list;
    let row = list.row(app.search_dialog.selected, app.search_dialog.scroll);
    assert!(row.y >= list.list.y && row.bottom() <= list.list.bottom());
    app.close_search_dialog();

    // 面板标题栏上有「在对话框中打开」；标签里的查询框可以直接改查询、重新搜索。
    paint(&mut app);
    let reopen = crate::app::terminal::HeaderHit::SearchDialog;
    assert!(app.hits.terminal.iter().any(|(_, h)| *h == reopen));
    let field = app.docked_parts().unwrap().query;
    app.press_docked_search(DialogPart::Query, field.x + 4.0, field.y + 4.0);
    assert!(app.docked_query_focused());
    app.edit_field(|f| {
        f.select_all();
        f.insert("struct Config")
    });
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    assert_eq!(app.search_dialog.hits.len(), 1);
    assert!(app.docked_search_shown() && !app.search_dialog.open);

    app.close_docked_search();
    assert!(!app.search_dialog.docked);
    assert!(!app.terminal.open);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 侧栏搜索面板顶部的「在对话框中打开」：带上面板的查询和搜索选项打开对话框。
#[test]
fn the_sidebar_search_opens_in_a_dialog() {
    use crate::layout::{PanelButton, PanelTool};
    let dir = search_folder("search-pop-out");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("load_config"));
    app.panels.global.whole_word = true;
    app.run_global_search();
    finish_search(&mut app);

    let parts = app.panel_parts();
    assert!(parts.tools.iter().any(|(_, t)| *t == PanelTool::OpenDialog));
    app.press_panel_button(PanelButton::Tool(PanelTool::OpenDialog));
    assert!(app.search_dialog.open);
    assert_eq!(app.focus, Focus::SearchDialog);
    assert_eq!(app.search_dialog.query.text(), "load_config");
    assert!(app.search_dialog.whole_word);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 对话框的文件掩码和范围：显示文件掩码时只搜索匹配的文件；「目录」只搜索当前文件所在的目录。
#[test]
fn the_search_dialog_filters_by_mask_and_directory() {
    use crate::app::workspace::search_dialog::DialogPart;
    let dir = search_folder("search-dialog-scope");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    paint(&mut app);
    let files = |app: &mut App| {
        app.search_dialog.engine.wait();
        app.poll_search_dialog(Instant::now());
        let mut names: Vec<String> = (app.search_dialog.engine.results.iter())
            .map(|f| f.relative.clone())
            .collect();
        names.sort();
        names
    };
    app.open_search_dialog();
    app.search_dialog.query.set_text("config");
    app.search_dialog_changed(true);
    assert!(files(&mut app).iter().any(|n| n.ends_with("Cargo.toml")));

    // 文件掩码：只搜索 `*.toml`；隐藏后恢复。
    app.search_dialog.mask.set_text("*.toml");
    app.press_search_dialog(DialogPart::MaskToggle, 0.0, 0.0);
    assert_eq!(files(&mut app), ["Cargo.toml"]);
    app.press_search_dialog(DialogPart::MaskToggle, 0.0, 0.0);
    assert!(files(&mut app).len() > 1);

    // 目录：只搜索 src，不再有 Cargo.toml。
    app.press_search_dialog(DialogPart::Scope(1), 0.0, 0.0);
    assert!(app.search_dialog.in_dir);
    let names = files(&mut app);
    assert!(
        !names.is_empty() && names.iter().all(|n| !n.ends_with("Cargo.toml")),
        "{names:?}"
    );
    app.press_search_dialog(DialogPart::Scope(0), 0.0, 0.0);
    assert!(files(&mut app).iter().any(|n| n.ends_with("Cargo.toml")));

    // 在文件掩码中输入时点范围，焦点留在文件掩码中。
    app.press_search_dialog(DialogPart::MaskToggle, 0.0, 0.0);
    assert_eq!(app.focus, Focus::SearchMask);
    app.press_search_dialog(DialogPart::Scope(1), 0.0, 0.0);
    assert_eq!(app.focus, Focus::SearchMask);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 比预览区短的文件读一次后留在缓存里，重绘时不再读盘。
#[test]
fn a_short_file_preview_is_read_once() {
    let dir = search_folder("search-dialog-cache");
    std::fs::write(dir.join("src/tiny.rs"), "fn tiny_marker() {}\n").unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("tiny_marker");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    let first = app.search_dialog_context().unwrap();
    assert_eq!(first.2, ["fn tiny_marker() {}"]);
    // 文件删掉后仍从缓存取到同样的几行。
    std::fs::remove_file(dir.join("src/tiny.rs")).unwrap();
    assert_eq!(app.search_dialog_context(), Some(first));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 预览的首行带着文件中推算出的高亮状态：Vue 的 `<script>` 中的行按脚本着色，没有打开的文件
/// 和已打开的文件相同。
#[test]
fn the_preview_highlights_lines_inside_a_vue_script() {
    let dir = search_folder("search-dialog-vue");
    // 匹配的一行在 `<script>` 中靠后的位置：预览的首行也在脚本中，从文件开头推算才知道。
    let filler = "const filler = 0\n".repeat(40);
    let text = format!(
        "<template>\n<p>hi</p>\n</template>\n<script setup>\n{filler}{}",
        "const vue_marker = 1\n</script>\n"
    );
    std::fs::write(dir.join("src/App.vue"), text).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("vue_marker");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    let closed = app.search_dialog_context().unwrap();
    assert_ne!(
        closed.3,
        winc_syntax::LineState::Normal,
        "首行在 <template> 中"
    );

    // 着色从首行起依次传下去：匹配的一行按脚本着色。
    let language = winc_syntax::detect("App.vue");
    let mut state = closed.3;
    let mut tokens = Vec::new();
    for line in &closed.2 {
        let chars: Vec<char> = line.chars().collect();
        let next = winc_syntax::highlight_line(language, &chars, state, &mut tokens);
        if line.contains("vue_marker") {
            assert!(
                tokens
                    .iter()
                    .any(|t| t.kind == winc_syntax::TokenKind::Keyword),
                "{line}"
            );
        }
        state = next;
    }

    // 列表中的一行：没有打开的文件在后台推算行首的状态，送回后按脚本着色。
    let place = &app.search_dialog.places.as_ref().unwrap().items[0];
    let (file, line) = (place.path.clone(), place.line);
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while app.place_highlight(&file, line).1 == winc_syntax::LineState::Normal {
        assert!(Instant::now() < deadline, "后台一直没有送回状态");
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.poll_search_dialog(Instant::now());
    }

    // 打开文件后用文档推算出的状态，结果相同；文档还没推算到列出的行时接着推算。
    app.open_path(&dir.join("src/App.vue"));
    app.poll_search_dialog(Instant::now());
    assert_ne!(
        app.place_highlight(&file, line).1,
        winc_syntax::LineState::Normal
    );
    app.search_dialog.context_forget();
    assert_eq!(app.search_dialog_context().unwrap().3, closed.3);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 换了查询而新的结果还没到时，列表留着上一次的结果（淡色），新的结果到了再换上并回到第一条。
#[test]
fn old_results_stay_until_new_ones_arrive() {
    let dir = search_folder("search-dialog-stale");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("config");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    let before = app.search_dialog.hits.len();
    assert!(before > 1);
    app.search_dialog_select(1);

    app.search_dialog.query.set_text("load_config");
    app.search_dialog_changed(true);
    assert!(app.search_dialog.stale);
    assert_eq!(app.search_dialog.hits.len(), before);
    // 这时回车不打开上一次的结果。
    let opened = app.editor.documents.len();
    app.search_dialog_accept();
    assert!(app.search_dialog.open);
    assert_eq!(app.editor.documents.len(), opened);

    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    assert!(!app.search_dialog.stale);
    assert!(app.search_dialog.hits.len() < before);
    assert_eq!(app.search_dialog.selected, 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 收起底部面板或关闭「搜索」标签时，焦点从标签里的查询或结果列表回到编辑区。
#[test]
fn hiding_the_search_tab_returns_focus_to_the_editor() {
    let dir = search_folder("search-dock-focus");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("config");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    app.dock_search_results();
    paint(&mut app);
    let field = app.docked_parts().unwrap().query;
    let query = crate::app::workspace::search_dialog::DialogPart::Query;

    app.press_docked_search(query, field.x + 4.0, field.y + 4.0);
    assert!(app.docked_query_focused());
    app.set_terminal_open(false);
    assert_eq!(app.focus, Focus::Editor);

    app.set_terminal_open(true);
    paint(&mut app);
    app.press_docked_search(query, field.x + 4.0, field.y + 4.0);
    app.close_docked_search();
    assert_eq!(app.focus, Focus::Editor);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 对话框的样子（浅色和深色）：四周留同样的边距，标题、输入的文字和列表的图标在同一列，预览
/// 一直排到底边；侧栏搜索标题行上是「在对话框中打开」的按钮。
///
/// 运行：`cargo test -p winc-app shoot_search_dialog -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_search_dialog() {
    for dark in [false, true] {
        let dir = search_folder(if dark {
            "dialog-shot-dark"
        } else {
            "dialog-shot-light"
        });
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        paint(&mut app);
        app.open_global_search(false);
        app.open_search_dialog();
        app.search_dialog.query.set_text("config");
        app.search_dialog_changed(true);
        app.search_dialog.engine.wait();
        app.poll_search_dialog(Instant::now());
        paint(&mut app);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("search-dialog-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 回归：开着在文件中查找的对话框时打开设置，鼠标在两者重叠处滚动：滚的是上面的设置，不是下面
/// 的对话框。
#[test]
fn the_wheel_scrolls_settings_over_the_search_dialog() {
    use winit::event::{MouseScrollDelta, TouchPhase};

    let dir = search_folder("dialog-under-settings");
    for i in 0..40 {
        std::fs::write(dir.join(format!("src/f{i}.rs")), "let config = 1;\n").unwrap();
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("config");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    paint(&mut app);
    let list = app.search_dialog_parts().list.list;
    let at = (list.x + list.w / 2.0, list.y + list.h / 2.0);
    // 先确认这里滚得动对话框。
    app.on_mouse_move(at.0, at.1);
    app.on_wheel_event(MouseScrollDelta::LineDelta(0.0, -1.0), TouchPhase::Moved);
    assert!(app.search_dialog.scroll > 0.0);
    app.search_dialog.scroll = 0.0;

    app.open_settings();
    paint(&mut app);
    assert!(
        app.settings_layout().body.contains(at.0, at.1),
        "设置盖在对话框上"
    );
    app.on_mouse_move(at.0, at.1);
    app.on_wheel_event(MouseScrollDelta::LineDelta(0.0, -1.0), TouchPhase::Moved);
    assert_eq!(app.search_dialog.scroll, 0.0, "滚到了下面的对话框");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 在文件中查找的对话框里搜 `load_config`，等结果列出来。
fn search_dialog_with_results(name: &str) -> (App, std::path::PathBuf) {
    let dir = search_folder(name);
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("load_config");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    app.poll_search_dialog(Instant::now());
    paint(&mut app);
    (app, dir)
}

/// 回归：在文件中查找的对话框盖在编辑区上，在它上面右键不弹出编辑区的菜单，光标不动。
#[test]
fn right_clicks_on_the_search_dialog_reach_no_menu() {
    let (mut app, dir) = search_dialog_with_results("search-dialog-menu");
    let list = app.search_dialog_parts().list.list;
    let at = (list.x + list.w / 2.0, list.y + list.h / 2.0);
    assert!(app.layout.editor.contains(at.0, at.1), "对话框盖在编辑区上");
    let caret = app.editor.active().unwrap().selection;
    right_click(&mut app, at);
    assert!(app.overlays.context_menu.is_none());
    assert_eq!(app.editor.active().unwrap().selection, caret);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：底部面板的「搜索」标签盖在终端上，在它上面右键不弹出终端的菜单。
#[cfg(unix)]
#[test]
fn right_clicks_on_the_search_tab_reach_no_menu() {
    let (mut app, dir) = search_dialog_with_results("search-tab-menu");
    let _term = super::super::terminal::show_terminal(&mut app, "$ ");
    app.dock_search_results();
    paint(&mut app);
    assert!(app.docked_search_shown());
    let row = app
        .docked_parts()
        .unwrap()
        .list
        .row(0, app.search_dialog.scroll);
    let at = (row.x + 40.0, row.y + 4.0);
    assert!(
        app.layout.terminal.contains(at.0, at.1),
        "「搜索」标签盖在终端上"
    );
    right_click(&mut app, at);
    assert!(app.overlays.context_menu.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：搜索 Vue 文件中 `<script>` 里的名字，列表和预览都按脚本着色。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_search_dialog_vue() {
    let dir = search_folder("search-dialog-vue-shot");
    let filler = "const filler = ref(0)\n".repeat(30);
    let template = "<template>\n  <div class=\"app\">{{ count }}</div>\n</template>\n";
    let script = "<script setup lang=\"ts\">\nimport { ref } from 'vue'\n";
    let text = format!("{template}{script}{filler}const count = ref(1) // count\n</script>\n");
    std::fs::write(dir.join("src/App.vue"), text).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    app.open_search_dialog();
    app.search_dialog.query.set_text("count");
    app.search_dialog_changed(true);
    app.search_dialog.engine.wait();
    for _ in 0..50 {
        app.poll_search_dialog(Instant::now());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.search_dialog.selected = 1;
    paint(&mut app);
    shoot(&mut app, "search-dialog-vue");
    std::fs::remove_dir_all(&dir).unwrap();
}
