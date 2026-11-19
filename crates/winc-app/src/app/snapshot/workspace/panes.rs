//! 编辑区窗格的测试：拆分、点击换前台、拖动分隔线、关掉窗格最后一个标签时合并，以及随现场保存
//! 和恢复。

use super::*;
use crate::app::workspace::panes::TabDrop;
use crate::keymap::Action;
use winc_editor::panes::{Axis, PaneId};

#[test]
fn panes_take_focus_on_click_resize_and_close_with_their_last_tab() {
    let dir = sample_folder("panes-split");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let whole = app.layout.editor;

    app.run_action(Action::SplitRight);
    paint(&mut app);
    let [left, right] = app.panes.rects[..] else {
        panic!("{:?}", app.panes.rects);
    };
    assert_eq!(app.editor.panes.focused, right.id);
    assert_eq!(app.layout.editor, right.editor);
    assert!(left.editor.right() < right.editor.x);
    assert_eq!(app.editor.tab_count(), 1);
    // 两个窗格的标签栏都在顶栏上，各占自己那一列。
    assert_eq!(left.tabs.y, right.tabs.y);
    assert!(left.tabs.right() <= right.tabs.x);

    // 点另一个窗格：它换到前台。
    click(&mut app, (left.editor.x + 120.0, left.editor.y + 60.0));
    assert_eq!(app.editor.panes.focused, left.id);
    assert_eq!(app.layout.editor, left.editor);

    // 拖动分隔线：左边变窄。
    let line = app.panes.dividers[0].line;
    let y = line.y + 80.0;
    app.on_mouse_move(line.x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(line.x - 120.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.panes.rects[0].whole.w < whole.w / 2.0 - 100.0);

    // 关掉右边窗格唯一的标签：文件在左边还开着，不询问，右边的窗格合并掉。
    app.focus_pane(right.id);
    app.run_action(Action::CloseTab);
    assert!(!app.editor.panes.is_split());
    assert!(app.panes.rects.is_empty());
    assert_eq!(app.editor.tab_count(), 1);
    assert_eq!(app.layout.editor, whole);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn splits_are_saved_with_the_session_and_restored() {
    let dir = sample_folder("panes-session");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_path(&dir.join("src/main.rs"));
    app.run_action(Action::SplitDown);
    app.save_session();
    let root = app.editor.root().unwrap().to_path_buf();
    let session = app.state.session_for(&root).cloned().unwrap();
    let panes = session.panes.clone().unwrap();
    assert_eq!(panes.panes.len(), 2);
    assert_eq!(panes.focused, 1);
    assert_eq!(panes.panes[1].tabs.len(), 1);

    let mut editor = winc_editor::Editor::default();
    editor.open_folder(&root).unwrap();
    crate::app::lifecycle::backup::restore_tabs(&mut editor, Some(&session), None);
    assert!(editor.panes.is_split());
    assert_eq!(editor.panes.focused, PaneId(1));
    assert_eq!(editor.tab_count(), 1);
    assert!(
        editor
            .active()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("src/main.rs")
    );
    editor.focus_pane(PaneId(0));
    assert_eq!(editor.tab_count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drawing_other_panes_leaves_the_focused_panes_layout_alone() {
    let dir = sample_folder("panes-parts");
    std::fs::write(dir.join("notes.md"), "# Notes\n\nSome text.\n").unwrap();
    let mut app = app_with(&dir.join("notes.md"), false);
    app.config.minimap = Some(true);
    app.run_action(Action::SplitRight);
    app.relayout();
    let parts = |app: &App| {
        let l = &app.layout;
        (l.tabs, l.text, l.preview, l.minimap)
    };
    let placed = parts(&app);
    assert!(placed.2.w > 0.0);
    // 画的时候其他窗格临时换到前台；画完前台窗格的标签栏、按钮和文字区域不变。
    paint(&mut app);
    assert_eq!(parts(&app), placed);
    // 其他窗格按它自己的位置放这些部分。
    let left = app.panes.rects[0];
    let (minimap, preview) = app.with_pane(left.id, |app| (app.layout.minimap, app.layout.preview));
    assert_eq!(minimap.right() + SCROLLBAR_HIT, left.editor.right());
    assert!(preview.right() <= left.tabs.right() && preview.x > left.tabs.x);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 按住前台窗格的当前标签，拖到 `to` 松开；`alt` 为真时松开前按着 `⌥`。
fn drag_active_tab(app: &mut App, to: (f32, f32), alt: bool) -> Option<TabDrop> {
    paint(app);
    let index = app.editor.active.unwrap();
    let tab = app.hits.tabs.iter().find(|h| h.index == index).unwrap().tab;
    let (x, y) = (tab.x + tab.w / 2.0, tab.y + tab.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y + 30.0);
    // 拖出了标签栏：标签回到原位，卡片跟着鼠标。
    assert!(app.dragged_tab().is_none());
    assert_eq!(app.tabs_dragged_out().len(), 1);
    if alt {
        app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    }
    app.on_mouse_move(to.0, to.1);
    let drop = app.panes.drop.map(|(d, _)| d);
    app.on_mouse_up(MouseButton::Left);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    drop
}

#[test]
fn dragging_a_tab_to_an_edge_splits_and_onto_another_pane_moves_it() {
    let dir = sample_folder("panes-drag");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_path(&dir.join("src/main.rs"));
    let first = app.editor.panes.focused;
    let name = |app: &App| app.editor.active().unwrap().name().into_owned();

    // 拖到编辑区右边：拆出右边的窗格，标签从原窗格移过去。
    let editor = app.layout.editor;
    let to = (editor.right() - 20.0, editor.y + editor.h / 2.0);
    let drop = drag_active_tab(&mut app, to, false);
    let split = TabDrop::Split {
        pane: first,
        axis: Axis::Row,
        before: false,
    };
    assert_eq!(drop, Some(split));
    let right = app.editor.panes.focused;
    assert_ne!(right, first);
    assert_eq!(app.editor.panes.tree.panes(), [first, right]);
    assert_eq!(name(&app), "main.rs");
    assert_eq!(app.editor.tab_count(), 1);
    assert_eq!(app.editor.pane_tabs(first).0.len(), 1);

    // 只有这一个标签的窗格，拖到它自己的边上什么也不变。
    let rects = app.panes.rects[1];
    let to = (rects.editor.x + 10.0, rects.editor.y + rects.editor.h / 2.0);
    assert_eq!(drag_active_tab(&mut app, to, false), None);
    assert_eq!(app.editor.panes.tree.panes(), [first, right]);

    // 按着 ⌥ 拖到左边窗格的下半部：左边上下拆开，右边仍留着这个标签。
    let left = app.panes.rects[0];
    let to = (
        left.editor.x + left.editor.w / 2.0,
        left.editor.bottom() - 10.0,
    );
    let drop = drag_active_tab(&mut app, to, true);
    assert!(matches!(
        drop,
        Some(TabDrop::Split {
            axis: Axis::Column,
            before: false,
            ..
        })
    ));
    let below = app.editor.panes.focused;
    assert_eq!(app.editor.panes.tree.panes(), [first, below, right]);
    assert_eq!(app.editor.pane_tabs(right).0.len(), 1);

    // 拖到右边窗格的标签栏上：放进那个窗格，下面的窗格空了就合并掉。
    let tabs = app.panes.rects.iter().find(|r| r.id == right).unwrap().tabs;
    let drop = drag_active_tab(
        &mut app,
        (tabs.x + tabs.w / 2.0, tabs.y + tabs.h / 2.0),
        false,
    );
    assert!(matches!(drop, Some(TabDrop::Into { pane, at: Some(_) }) if pane == right));
    assert_eq!(app.editor.panes.focused, right);
    assert_eq!(app.editor.panes.tree.panes(), [first, right]);
    assert_eq!(app.editor.tab_count(), 1);
    assert_eq!(name(&app), "main.rs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_panes_load_their_current_file_and_hover_only_where_the_mouse_is() {
    let dir = sample_folder("panes-load");
    let mut app = app_with(&dir.join("short.rs"), false);
    let index = app.editor.open_tab(&dir.join("src/main.rs"));
    app.editor.active = Some(index);
    let id = app.editor.documents[index].id;
    app.run_action(Action::SplitRight);
    app.open_path(&dir.join("short.rs"));
    // 左边窗格的当前标签还没读：排版时交给后台读取（测试中没有窗口，就地读完）。
    assert!(!app.editor.document(id).unwrap().is_pending());
    assert_eq!(app.editor.document(id).unwrap().text().line_count(), 4);

    // 鼠标在右边（前台）窗格的新建按钮上：只有右边显示悬停。
    paint(&mut app);
    let hovers = |app: &mut App| {
        let mut seen = Vec::new();
        app.draw_panes(|app, _| seen.push((app.editor.panes.focused, app.pointer.hover)));
        seen
    };
    let [left, right] = [app.panes.rects[0].id, app.panes.rects[1].id];
    let plus = app.tabs.new_tab;
    app.on_mouse_move(plus.x + plus.w / 2.0, plus.y + plus.h / 2.0);
    assert_eq!(app.pointer.hover, Hover::NewTab);
    assert_eq!(
        hovers(&mut app),
        [(left, Hover::None), (right, Hover::NewTab)]
    );

    // 鼠标移到左边窗格的新建按钮上：左边显示悬停，右边不显示。
    let plus = app.with_pane(left, |app| app.tabs.new_tab);
    app.on_mouse_move(plus.x + plus.w / 2.0, plus.y + plus.h / 2.0);
    assert_eq!(
        hovers(&mut app),
        [(left, Hover::NewTab), (right, Hover::None)]
    );
    assert_eq!(app.pointer.hover, Hover::NewTab);
    assert_eq!(app.panes.hover, Some(left));

    // 左边窗格的文字上是文本光标。
    let text = app.with_pane(left, |app| app.layout.text);
    app.on_mouse_move(text.x + 40.0, text.y + 10.0);
    assert_eq!(app.pointer.cursor_icon, winit::window::CursorIcon::Text);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn buttons_appearing_in_the_focused_pane_do_not_show_in_other_panes() {
    use crate::app::animation::{Clock, MODULE_FADE};
    let dir = sample_folder("panes-buttons");
    std::fs::write(dir.join("notes.md"), "# Notes\n").unwrap();
    let mut app = app_with(&dir.join("a.txt"), false);
    app.run_action(Action::SplitRight);
    app.open_path(&dir.join("notes.md"));
    // 右边（前台）切到 Markdown，「源代码 / 分栏 / 预览」正在展开。
    let fx = &mut app.motion.module_fx.preview;
    fx.track(false, None);
    fx.track(true, Some(Clock::frozen(MODULE_FADE / 3, MODULE_FADE)));
    app.relayout();
    let mut seen = Vec::new();
    app.draw_panes(|app, front| {
        // 不在前台的窗格画得淡一些（当前标签）。
        assert_eq!(app.panes.drawing_background, !front);
        seen.push(app.layout.preview.w);
    });
    let [left, right] = seen[..] else {
        panic!("{seen:?}");
    };
    assert_eq!(left, 0.0);
    assert!(
        right > 0.0 && right < crate::layout::PREVIEW_GROUP_W,
        "{right}"
    );
    assert!(app.motion.module_fx.preview.moving());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_pane_keeps_its_own_markdown_preview() {
    use winc_editor::PreviewMode;
    let dir = sample_folder("panes-preview");
    std::fs::write(dir.join("a.md"), "# Alpha\n").unwrap();
    std::fs::write(dir.join("b.md"), "# Beta\n").unwrap();
    let mut app = app_with(&dir.join("a.md"), false);
    app.editor
        .active_mut()
        .unwrap()
        .set_preview_mode(PreviewMode::Preview);
    let a = app.editor.active().unwrap().id;
    app.run_action(Action::SplitRight);
    app.open_path(&dir.join("b.md"));
    app.editor
        .active_mut()
        .unwrap()
        .set_preview_mode(PreviewMode::Preview);
    let b = app.editor.active().unwrap().id;
    let left = app.panes.rects[0].id;
    for _ in 0..2 {
        paint(&mut app);
        app.wait_for_markdown();
        app.with_pane(left, |app| app.wait_for_markdown());
    }
    paint(&mut app);
    // 左边画的是 a.md 的预览，右边（前台）是 b.md 的，各自解析好了。
    let shown = |app: &App| {
        let md = &app.preview.markdown;
        (md.doc, md.parsed.is_some(), md.area.w > 0.0)
    };
    assert_eq!(shown(&app), (Some(b), true, true));
    assert_eq!(app.with_pane(left, |app| shown(app)), (Some(a), true, true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_a_document_removes_it_from_every_pane() {
    let dir = sample_folder("panes-close");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_path(&dir.join("src/main.rs"));
    let main = app.editor.active().unwrap().id;
    // 左边留着 short.rs 和 main.rs，右边只开 short.rs。
    app.editor.active = app.editor.index_of(app.editor.tabs()[0].id);
    app.run_action(Action::SplitRight);
    let right = app.editor.panes.focused;
    // 文件删掉了：只在左边开着的 main.rs 也关掉。
    app.close_documents(&[main]);
    assert!(app.editor.document(main).is_none());
    assert!(app.editor.panes.is_split());
    assert_eq!(app.editor.panes.focused, right);
    // 左边唯一剩下的 short.rs 也关掉：两边都去掉，只剩一个空的窗格。
    let short = app.editor.active().unwrap().id;
    app.close_documents(&[short]);
    assert!(app.editor.documents.is_empty());
    assert!(!app.editor.panes.is_split());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wheel_glide_over_another_pane_scrolls_that_pane() {
    use crate::app::animation::{Clock, MODULE_FADE};
    let dir = sample_folder("panes-wheel");
    let text: String = (0..300).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("long.txt"), text).unwrap();
    let mut app = app_with(&dir.join("long.txt"), false);
    app.run_action(Action::SplitRight);
    paint(&mut app);
    let left = app.panes.rects[0];
    app.pointer.mouse = (left.editor.x + 100.0, left.editor.y + 100.0);
    // 鼠标滚轮在左边（不在前台）开始一段平滑滚动，之后逐帧推进。
    let clock = Clock::frozen(MODULE_FADE, MODULE_FADE);
    app.with_pane(left.id, |app| app.start_wheel_glide((0.0, -400.0), clock));
    app.advance_wheel_glide();
    let scrolled = |app: &mut App| app.editor.active().unwrap().scroll_line;
    assert!(app.with_pane(left.id, scrolled) > 0.0);
    assert_eq!(scrolled(&mut app), 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pane_commands_are_unavailable_without_a_split() {
    let dir = sample_folder("panes-available");
    let mut app = app_with(&dir.join("short.rs"), false);
    for action in [Action::Unsplit, Action::NextPane, Action::MoveToSplitRight] {
        assert!(!app.action_available(action), "{action:?}");
    }
    assert!(!app.action_available(Action::NextTerminalPane));
    assert!(app.action_available(Action::SplitRight));
    app.run_action(Action::SplitRight);
    assert!(app.action_available(Action::NextPane));
    assert!(app.action_available(Action::Unsplit));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_dropped_at_a_pane_edge_open_in_a_new_split() {
    let dir = sample_folder("panes-file-drop");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let first = app.editor.panes.focused;
    let editor = app.layout.editor;
    let at = (editor.x + editor.w / 2.0, editor.bottom() - 20.0);
    // 同一次拖进来两个文件，落在下边：下面拆出一个窗格，两个文件都开在里面。
    app.hover_file();
    app.drop_file_at(&dir.join("a.txt"), at.0, at.1);
    app.drop_file_at(&dir.join("Cargo.toml"), at.0, at.1);
    let below = app.editor.panes.focused;
    assert_eq!(app.editor.panes.tree.panes(), [first, below]);
    let names: Vec<String> = (app.editor.tabs().iter())
        .map(|d| d.name().into_owned())
        .collect();
    assert_eq!(names, ["a.txt", "Cargo.toml"]);
    assert_eq!(app.editor.pane_tabs(first).0.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tab_dropped_after_the_last_tab_of_a_pane_goes_last() {
    let dir = sample_folder("panes-slot");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_path(&dir.join("src/main.rs"));
    app.open_path(&dir.join("a.txt"));
    let left = app.editor.panes.focused;
    let short = app.editor.tabs()[0].id;
    app.run_action(Action::SplitRight);
    app.open_path(&dir.join("short.rs"));
    // 右边的 short.rs 复制到左边标签栏的最后：左边原有的 short.rs 挪到最后。
    let into = TabDrop::Into {
        pane: left,
        at: Some(3),
    };
    app.drop_tabs(&[short], short, into, true);
    assert_eq!(app.editor.panes.focused, left);
    let names: Vec<String> = (app.editor.tabs().iter())
        .map(|d| d.name().into_owned())
        .collect();
    assert_eq!(names, ["main.rs", "a.txt", "short.rs"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 手动运行：截一张三个窗格（右边再上下拆）加上拆分过的终端的图。
#[cfg(unix)]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_split_panes() {
    let dir = sample_folder("panes-shot");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_path(&dir.join("src/main.rs"));
    app.run_action(Action::SplitRight);
    app.open_path(&dir.join("short.rs"));
    app.run_action(Action::SplitDown);
    let _term = super::super::terminal::show_terminal(&mut app, "$ cargo test\r\nok\r\n");
    let size = app.terminal_size();
    let (mut all, at) = app.terminal.take_group(0);
    all.push(crate::app::terminal::TermTab::new(99, size));
    app.terminal.put_group(0, all, at);
    app.focus = Focus::Editor;
    app.relayout();
    shoot(&mut app, "split-panes");
    let _ = std::fs::remove_dir_all(&dir);
}
