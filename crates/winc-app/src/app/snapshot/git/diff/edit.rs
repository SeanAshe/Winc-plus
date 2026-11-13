//! 在差异视图中编辑与移动光标的测试。

use super::*;

/// 单击右侧固定的方法头：光标放到右侧文件的该行，右侧进入编辑，该行滚到类的固定行之下。
#[test]
fn clicking_a_pinned_diff_header_puts_the_caret_on_it() {
    let (mut app, dir) = sticky_worktree_diff("diff-sticky-click");
    scroll_right_line_to_top(&mut app, 30);
    let header = app
        .hits
        .diff
        .iter()
        .find_map(|&(rect, target)| match target {
            DiffTarget::Sticky(RIGHT, 2) => Some(rect),
            _ => None,
        })
        .expect("右侧固定了方法头");
    click(
        &mut app,
        (header.x + header.w / 2.0, header.y + header.h / 2.0),
    );
    paint(&mut app);
    let source = worktree_source(&mut app);
    assert_eq!(source.text().pos_to_line_col(source.selection.head).0, 2);
    assert!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .editing
    );
    assert!((right_row_below_top(&app, 2) - 1.0).abs() < 0.01);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 光标或查找到的匹配移到被固定行遮住的位置时，滚动到固定行之下：层数按目标行计算，
/// 不取上一帧画面中的固定行数。
#[test]
fn diff_carets_and_matches_stay_below_the_pinned_headers() {
    let (mut app, dir) = sticky_worktree_diff("diff-sticky-follow");
    // 顶部是文件开头，没有固定行；目标行在方法里，外层有类和方法两层。
    scroll_right_line_to_top(&mut app, 0);
    let source = worktree_source(&mut app);
    let at = source.text().line_col_to_pos(20, 8);
    source.selection = Selection::point(at);
    scroll_right_line_to_top(&mut app, 21);
    assert!(app.diff_edit_text("x"));
    paint(&mut app);
    assert!(
        right_row_below_top(&app, 20) > 2.0 - 1e-3,
        "光标在两层固定行之下"
    );

    scroll_right_line_to_top(&mut app, 41);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let start = view.new.text.line_to_char(40) + 8;
    app.reveal_diff_match(RIGHT, start, start + 9);
    paint(&mut app);
    assert!(
        right_row_below_top(&app, 40) > 2.0 - 1e-3,
        "匹配在两层固定行之下"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn diff_text_clicks_and_cursors_work_like_the_editor() {
    use crate::app::diff::DIFF_TEXT_PAD;
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};
    use winc_editor::diff_view::LEFT;
    use winit::keyboard::ModifiersState;

    let dir = search_folder("git-diff-clicks");
    if !long_repo(&dir) {
        return;
    }
    let file = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    // 第 `side` 侧第 `line` 行（从 0 起）第 `col` 列的窗口坐标；`gutter` 为真时取该行行号处的坐标。
    let at = |app: &App, side: usize, line: usize, col: usize, gutter: bool| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
        let row = view.rows[side]
            .iter()
            .position(|r| matches!(r, SideRow::Line { line: l, .. } if *l == line))
            .unwrap();
        let lh = app.fonts.line_height;
        let pane = parts.panes[side];
        let x = if gutter {
            pane.gutter.x + pane.gutter.w / 2.0
        } else {
            pane.text.x + DIFF_TEXT_PAD + (col as f32 + 0.3) * app.metrics.cell
        };
        (x, parts.body.y + (row as f32 + 0.5) * lh)
    };
    let source = |app: &App| {
        app.editor
            .documents
            .iter()
            .position(|d| d.path.as_deref() == Some(file.as_path()))
            .unwrap()
    };
    let selected = |app: &App| {
        let doc = &app.editor.documents[source(app)];
        doc.text_range(doc.selection.from(), doc.selection.to())
            .unwrap_or_default()
    };
    // 按下、松开，按住期间鼠标轻微抖动（横向移动一个像素）。
    let press = |app: &mut App, (x, y): (f32, f32)| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(x + 1.0, y);
        app.on_mouse_up(MouseButton::Left);
    };

    // 右侧：双击选词、三击选行，按住期间轻微抖动也不会缩小选区。
    let word = at(&app, RIGHT, 5, 6, false);
    press(&mut app, word);
    press(&mut app, word);
    assert_eq!(selected(&app), "value_5");
    press(&mut app, word);
    assert_eq!(selected(&app), "let value_5 = 500;\n");

    // 点击行号选中整行，拖到下一行的行号选中两行。
    app.pointer.last_click = None;
    let (x, y) = at(&app, RIGHT, 6, 0, true);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (x, y) = at(&app, RIGHT, 7, 0, true);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(selected(&app), "let value_6 = 6;\nlet value_7 = 7;\n");

    // ⌥ 单击再添加一个光标，键入的字符出现在两处；Esc 先移除多余的光标，再移除右侧的光标。
    app.pointer.last_click = None;
    let point = at(&app, RIGHT, 6, 0, false);
    click(&mut app, point);
    app.pointer.modifiers = ModifiersState::ALT;
    let point = at(&app, RIGHT, 7, 0, false);
    click(&mut app, point);
    app.pointer.modifiers = ModifiersState::empty();
    assert_eq!(app.editor.documents[source(&app)].selections().0.len(), 2);
    assert!(app.diff_edit_text("X"));
    let text = app.editor.documents[source(&app)]
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(text.contains("Xlet value_6 = 6;\nXlet value_7"), "{text}");
    assert!(app.run_action(Action::RemoveSecondaryCursors));
    assert_eq!(app.editor.documents[source(&app)].selections().0.len(), 1);
    assert!(app.run_action(Action::Undo));
    assert!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .editing
    );

    // ⌘D：再选中下一处同样的词。
    app.pointer.last_click = None;
    let word = at(&app, RIGHT, 8, 1, false);
    press(&mut app, word);
    press(&mut app, word);
    assert_eq!(selected(&app), "let");
    assert!(app.run_action(Action::AddNextOccurrence));
    assert_eq!(app.editor.documents[source(&app)].selections().0.len(), 2);
    assert!(app.run_action(Action::RemoveSecondaryCursors));

    // 右键：右侧有剪切、复制、粘贴，光标在改动上时可以回滚该处。
    let point = at(&app, RIGHT, 5, 2, false);
    right_click(&mut app, point);
    assert!(matches!(
        app.overlays.context_menu.as_ref().unwrap().target,
        MenuTarget::Diff(_, true)
    ));
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::Paste, true)));
    assert!(commands.contains(&(MenuCommand::GitRevertHunk, true)));
    let point = at(&app, RIGHT, 8, 2, false);
    right_click(&mut app, point);
    assert!(menu_commands(&app).contains(&(MenuCommand::GitRevertHunk, false)));

    // 左侧：双击选词，点击行号选中整行，右键菜单只有复制。先关闭已打开的菜单：菜单打开时，
    // 点击菜单外只关闭菜单。
    app.dismiss_popup();
    app.pointer.last_click = None;
    let word = at(&app, LEFT, 5, 6, false);
    press(&mut app, word);
    press(&mut app, word);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.editing);
    assert_eq!(view.read_selected().unwrap(), "value_5");
    app.pointer.last_click = None;
    let point = at(&app, LEFT, 9, 0, true);
    click(&mut app, point);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selected().unwrap(), "let value_9 = 9;\n");
    let point = at(&app, LEFT, 9, 3, false);
    right_click(&mut app, point);
    assert_eq!(menu_commands(&app), vec![(MenuCommand::Copy, true)]);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selected().unwrap(), "let value_9 = 9;\n");
}

/// 只读差异快照中的查找规则：替换快捷键只打开查找；预填规则与编辑器相同；长匹配横向
/// 滚动到末端可见；单栏布局下未改动的行同样标出左侧的匹配；其他文档等待中的跳转作废。
#[test]
fn find_in_a_commit_diff_follows_the_editor_rules() {
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT, ReadSelection};

    let dir = sample_folder("diff-commit-find-rules");
    let mut app = app_with(&dir.join("short.rs"), false);
    let other = app.editor.active().unwrap().id;
    let language = winc_syntax::detect("main.rs");
    let side = |text: &str| DiffSide::new("abc1234", text, None, language);
    let wide = format!(
        "{} long_identifier_at_the_far_right_end_of_the_line\n",
        "x".repeat(400)
    );
    let old = format!("a -> b\nleft only\n{wide}");
    let new = format!("a -> b\nright only\n{wide}");
    app.editor
        .show_diff_in(None, "main.rs", DiffView::new(side(&old), side(&new)));
    paint(&mut app);
    let set_read = |app: &mut App, side: usize, range: (usize, usize)| {
        let view = app
            .editor
            .active_mut()
            .unwrap()
            .diff
            .as_deref_mut()
            .unwrap();
        view.read_selection = Some(ReadSelection { side, range });
    };

    // 替换快捷键只打开查找；光标在符号 `->` 上时不预填。
    set_read(&mut app, LEFT, (3, 3));
    app.bars.search.query.clear();
    app.open_search(true);
    assert!(app.bars.find.open && !app.bars.find.replace_open);
    assert_eq!(app.bars.search.query, "");

    // 长匹配横向滚动到能显示其末端，且不越过起点。
    app.bars
        .find
        .input
        .set_text("long_identifier_at_the_far_right_end_of_the_line");
    app.bars.search.query = "long_identifier_at_the_far_right_end_of_the_line".into();
    app.search_changed();
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let view = doc.diff.as_deref().unwrap();
    let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
    let width = parts.panes[LEFT].text.w - 2.0 * crate::app::diff::DIFF_TEXT_PAD;
    let (x, x_end) = (401.0 * app.metrics.cell, 449.0 * app.metrics.cell);
    assert!(
        doc.scroll_x <= x && x_end <= doc.scroll_x + width,
        "{}",
        doc.scroll_x
    );

    // 查找快照时，其他文档等待中的跳转作废。
    app.bars.find.pending_jump = Some((other, 0, false));
    app.advance_search();
    assert_eq!(app.bars.find.pending_jump, None);

    // 单栏布局：未改动的行按右侧绘制，查找左侧时同样标出匹配。
    app.close_search();
    let doc = app.editor.active_mut().unwrap();
    doc.scroll_x = 0.0;
    doc.scroll_line = 0.0;
    doc.diff.as_deref_mut().unwrap().set_unified(true);
    set_read(&mut app, LEFT, (0, 0));
    paint(&mut app);
    let row = |app: &App| {
        let renderer = app.frame.renderer.as_ref().unwrap();
        let (scale, image) = (renderer.scale_factor(), renderer.image().unwrap());
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
        // 单栏的第 0 行是未改动的 `a -> b`。
        let y = ((parts.body.y + app.fonts.line_height * 0.5) * scale) as usize;
        let w = image.width as usize;
        image.pixels[y * w..(y + 1) * w].to_vec()
    };
    let plain = row(&app);
    app.open_search(false);
    app.bars.find.input.set_text("->");
    app.bars.search.query = "->".into();
    app.search_changed();
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selection.map(|s| s.side), Some(LEFT));
    assert_ne!(row(&app), plain, "左侧的匹配在单栏的未改动行上标出");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_inline_layout_can_be_toggled_and_edits_only_new_lines() {
    use winc_editor::diff_view::{LEFT, RIGHT};

    let dir = search_folder("git-diff-inline");
    if !long_repo(&dir) {
        return;
    }
    std::fs::remove_file(dir.join("src/new.rs")).unwrap();
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&main, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    let view = |app: &App| {
        let doc = app.editor.active().unwrap();
        (doc.scroll_line, doc.diff.as_deref().unwrap().is_unified())
    };
    assert!(!view(&app).1);
    assert!(!app.switch_on(Setting::DiffInline));
    let layout = app.hits.diff.iter().any(|(_, t)| *t == DiffTarget::Layout);
    assert!(layout);

    // 切换：修改的是设置，文字区占满整个宽度，两侧共用一个文字区。
    assert!(app.run_action(Action::ToggleDiffLayout));
    assert!(view(&app).1);
    assert_eq!(app.config.diff_inline, Some(true));
    assert!(app.editor.diff_unified);
    let parts = |app: &App| {
        let doc = app.editor.active().unwrap();
        crate::app::diff_parts(
            app.layout.editor,
            doc.diff.as_deref().unwrap(),
            app.metrics.cell,
        )
    };
    let inline = parts(&app);
    assert_eq!(inline.panes[LEFT].text, inline.panes[RIGHT].text);
    assert!(inline.panes[RIGHT].text.w > app.layout.editor.w * 0.7);
    paint(&mut app);

    // 跳转仍然正常：第一处改动先显示旧行，其后是新行。
    assert!(app.run_action(Action::NextDifference));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let diff = doc.diff.as_deref().unwrap();
    assert_eq!(diff.focus.map(|f| f.0), Some(0));
    let start = diff.hunk_pos(0) as usize;
    assert_eq!(diff.unified_row(start).map(|r| r.0), Some(LEFT));
    assert_eq!(diff.unified_row(start + 1).map(|r| r.0), Some(RIGHT));

    // 第 `index` 个显示行的窗口坐标。
    let at = |app: &App, index: usize, col: usize| {
        let doc = app.editor.active().unwrap();
        let parts = parts(app);
        let y =
            parts.body.y + ((index as f64 - doc.scroll_line) as f32 + 0.5) * app.fonts.line_height;
        let x = parts.panes[RIGHT].text.x
            + crate::app::diff::DIFF_TEXT_PAD
            + app.metrics.cell * col as f32;
        (x, y)
    };
    // 点击旧行：只读，光标位于左侧。
    let old = at(&app, start, 4);
    assert!(!app.over_diff_editable(old.0, old.1));
    assert!(app.over_diff_text(old.0, old.1));
    click(&mut app, old);
    let diff = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!diff.editing);
    assert_eq!(diff.read_caret().map(|c| c.0), Some(LEFT));
    // 选中、复制的是旧行的内容。
    app.pointer.last_click = None;
    click(&mut app, old);
    click(&mut app, old);
    let diff = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(diff.read_selected().as_deref(), Some("value_5"));

    // 点击新行：光标进入右侧的文档，键入的字符写入文件。
    app.pointer.last_click = None;
    let new = at(&app, start + 1, 0);
    assert!(app.over_diff_editable(new.0, new.1));
    click(&mut app, new);
    let diff = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(diff.editing);
    assert!(app.diff_edit_text("X"));
    let source = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(&main));
    let text = source.unwrap().text().rope().unwrap().to_string();
    assert!(text.contains("Xlet value_5 = 500;"), "{text}");
    paint(&mut app);
    // 重新比较后仍然是单栏。
    assert!(view(&app).1);

    // 新打开的差异沿用选择；切回并排也沿用。
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    assert!(view(&app).1);
    assert!(app.run_action(Action::ToggleDiffLayout));
    assert!(!view(&app).1);
    assert_eq!(app.config.diff_inline, Some(false));
    paint(&mut app);
    assert_eq!(view(&app).0.fract(), 0.0);

    // 设置页的开关、标题栏的按钮改的是同一项，已打开的差异随之更新。
    app.settings_toggle(Setting::DiffInline);
    assert!(view(&app).1);
    app.press_diff(DiffTarget::Layout);
    assert!(!view(&app).1);
    assert!(!app.switch_on(Setting::DiffInline));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn typing_at_the_read_caret_opens_the_file_and_edits_there() {
    use crate::app::diff::edits_text;
    use winit::keyboard::{Key, NamedKey};

    let dir = search_folder("git-diff-type");
    if !long_repo(&dir) {
        return;
    }
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    assert!(app.run_action(Action::NextDifference));
    let tabs = app.editor.documents.len();
    let id = app.editor.active().unwrap().id;
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.editing);
    assert!(view.read_caret().is_some());

    // 修改文字的键才打开文件，方向键只移动。
    assert!(edits_text(&Key::Named(NamedKey::Backspace)));
    assert!(edits_text(&Key::Named(NamedKey::Enter)));
    assert!(!edits_text(&Key::Named(NamedKey::ArrowDown)));
    assert!(!edits_text(&Key::Named(NamedKey::PageDown)));

    // 输入文字：打开文件但不切换标签，文字写入右侧文档中光标所在的行。
    assert!(app.diff_edit_text("X"));
    assert_eq!(app.editor.documents.len(), tabs + 1);
    assert_eq!(app.editor.active().unwrap().id, id);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.editing);
    assert_eq!(view.read_selection, None);
    let source = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(&main));
    let text = source.unwrap().text().rope().unwrap().to_string();
    assert!(text.contains("Xlet value_5 = 500;"), "{text}");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn diffs_render_whitespace_like_the_editor() {
    use winc_config::RenderWhitespace;

    use super::super::editor::whitespace::{changed, pixels_with};

    let dir = search_folder("whitespace-diff");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    app.run(winc_editor::Command::Insert("use std::fs;\n".into()));
    app.run_action(Action::GitShowChanges);
    paint(&mut app);
    assert!(app.editor.active().unwrap().diff.is_some());

    let none = pixels_with(&mut app, RenderWhitespace::None);
    // 没有选区时，「选中时」的效果等同于「不显示」。
    assert_eq!(none, pixels_with(&mut app, RenderWhitespace::Selection));
    assert!(changed(&none, &pixels_with(&mut app, RenderWhitespace::All)) > 0);

    let _ = std::fs::remove_dir_all(&dir);
}
