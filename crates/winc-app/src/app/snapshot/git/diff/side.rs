//! 并排差异的测试：两侧的排版、滚动与点击。

use super::*;

#[test]
fn show_changes_opens_a_side_by_side_diff_and_walks_it() {
    let dir = search_folder("git-diff");
    if !long_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    let mut app = app_with(&dir.join("src/main.rs"), false);
    // 模块关闭时该命令不可用。
    assert!(!app.run_action(Action::GitShowChanges));
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    // 再修改一处且不保存：右侧显示编辑器中的内容。
    app.run(winc_editor::Command::Insert("// 新的一行\n".into()));
    assert!(app.run_action(Action::GitShowChanges));

    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("main.rs · 差异"));
    let view = doc.diff.as_deref().unwrap();
    assert!(view.old.title.starts_with("HEAD · "), "{}", view.old.title);
    assert_eq!(view.new.title, "工作区 · 未保存");
    assert_eq!(view.hunks.len(), 3);
    assert!(!view.old.missing && !view.new.missing);
    assert_eq!(view.stats(), (2, 2));
    assert!(
        view.rows[RIGHT]
            .iter()
            .any(|r| matches!(r, SideRow::Fold { .. }))
    );

    // 绘制一帧后，标题栏的按钮和折叠的行都可以点击。
    paint(&mut app);
    // 「第几处」按跳转到的改动计算：整个差异能在一屏内显示、无需滚动时，同样逐处向下跳转。
    let current = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.diff.as_deref().unwrap().current_hunk(doc.scroll_line)
    };
    // 打开时位于第一处，没有标记；第一次跳转标记第一处，之后依次向下。
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };
    assert_eq!(current(&app), Some(0));
    assert_eq!(focus(&app), None);
    app.press_diff(DiffTarget::Next);
    assert_eq!(focus(&app), Some(0));
    app.press_diff(DiffTarget::Next);
    assert_eq!(current(&app), Some(1));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(current(&app), Some(2));
    // 到达最后一处后继续向下：第一次只提示，再按一次进入下一个有改动的文件；向回同样需要
    // 按两次，回到该文件的最后一处。
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    app.press_diff(DiffTarget::Next);
    assert!(title(&app).starts_with("main.rs"), "{}", title(&app));
    let hint = app.status.message.clone().unwrap();
    assert!(hint.starts_with("已是最后一处更改"), "{hint}");
    app.press_diff(DiffTarget::Next);
    assert!(!title(&app).starts_with("main.rs"), "{}", title(&app));
    assert!(app.run_action(Action::PreviousDifference));
    assert!(app.run_action(Action::PreviousDifference));
    assert!(title(&app).starts_with("main.rs"), "{}", title(&app));
    assert_eq!(current(&app), Some(2));
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(current(&app), Some(1));

    paint(&mut app);
    let fold = app
        .hits
        .diff
        .iter()
        .find(|(_, t)| matches!(t, DiffTarget::Fold(_)))
        .map(|(r, _)| *r);
    if let Some(rect) = fold {
        let before = app
            .editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .row_count();
        click(&mut app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
        let after = app
            .editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .row_count();
        assert!(after > before);
    }
    app.press_diff(DiffTarget::Collapse);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.collapses());
    assert!(
        !view.rows[RIGHT]
            .iter()
            .any(|r| matches!(r, SideRow::Fold { .. }))
    );
    // 差异标签只读，编辑命令不修改任何内容。
    app.run(winc_editor::Command::Insert("x".into()));
    paint(&mut app);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn diff_right_side_edits_the_file_and_takes_hunks_from_the_left() {
    let dir = search_folder("git-diff-edit");
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
    let tabs = app.editor.documents.len();
    assert_eq!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .hunks
            .len(),
        2
    );

    // 文件未打开：「»」直接修改磁盘上的文件，不新开标签。
    paint(&mut app);
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::Revert(0))
    );
    app.press_diff(DiffTarget::Revert(0));
    assert_eq!(app.editor.documents.len(), tabs);
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("let value_5 = 5;\n"));
    assert_eq!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .hunks
            .len(),
        1
    );

    // 在右侧点击放置光标：文件在后台打开，差异标签仍是当前标签。
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
    let pane = parts.panes[1];
    let at = (
        pane.text.x + 30.0,
        parts.body.y + app.fonts.line_height * 0.5,
    );
    click(&mut app, at);
    assert_eq!(app.editor.documents.len(), tabs + 1);
    assert!(app.editor.active().unwrap().diff.is_some());
    assert!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .editing
    );

    // 左侧：拖选一段，⌘C 复制；右侧的光标关闭。
    let lh = app.fonts.line_height;
    let left = parts.panes[0].text;
    let from = (left.x + 14.0, parts.body.y + lh * 1.5);
    let to = (left.x + 60.0, parts.body.y + lh * 2.5);
    app.on_mouse_move(from.0, from.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(to.0, to.1);
    app.on_mouse_up(MouseButton::Left);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.editing);
    let selected = view.read_selected().unwrap();
    assert!(
        selected.contains('\n') && selected.starts_with("et value_"),
        "{selected:?}"
    );
    assert!(app.run_action(Action::Copy));
    assert!(app.run_action(Action::SelectAll));
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selected().unwrap(), view.old.text.to_string());
    click(&mut app, at);
    assert!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .read_selection
            .is_none()
    );

    // 键入的字符写入文件，差异随之重新比较；撤销也是文件的撤销。
    assert!(app.diff_edit_text("X"));
    let source = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    assert!(source.is_dirty());
    assert!(source.text().rope().unwrap().to_string().contains('X'));
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.hunks.len(), 2);
    assert_eq!(view.new.title, "工作区 · 未保存");
    assert!(app.run_action(Action::Undo));
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.hunks.len(), 1);

    // ⌘F 查找右侧的文件：跳转到的匹配是右侧文档的选区，差异随之滚动到该处；查找条位于
    // 差异的标题栏下方。
    assert!(app.run_action(Action::Find));
    assert!(app.bars.find.open);
    assert!(app.layout.search.y >= app.layout.editor.y + crate::app::diff::DIFF_HEADER);
    app.bars.find.input.set_text("value_55 ");
    app.bars.search.query = "value_55 ".into();
    app.search_changed();
    assert!(app.editor.active().unwrap().diff.is_some());
    let source = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    let selected = source.text_range(source.selection.from(), source.selection.to());
    assert_eq!(selected.as_deref(), Some("value_55 "));
    let line = source.text().pos_to_line_col(source.selection.head).0;
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let row = view.row_of_new(line);
    let scroll = app.editor.active().unwrap().scroll_line as usize;
    assert!(row >= scroll && row < scroll + app.visible_lines());
    app.close_search();

    // 光标不在改动上时「回滚光标处的改动」不生效；移到被删除行的位置再回滚，两侧内容一致。
    assert!(app.run_action(Action::GitRevertChange));
    assert_eq!(
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .hunks
            .len(),
        1
    );
    let source = app
        .editor
        .documents
        .iter_mut()
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    let pos = source.text().line_start(50);
    source.selection = winc_core::Selection::point(pos);
    assert!(app.run_action(Action::GitRevertChange));
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.is_identical());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_only_right_side_selects_and_copies_like_the_left() {
    use crate::app::input::context_menu::MenuCommand;
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT};

    let dir = sample_folder("diff-read-only-right");
    let mut app = app_with(&dir.join("short.rs"), false);
    // 从日志打开的差异：两侧都是历史版本，都没有路径。
    let language = winc_syntax::detect("main.rs");
    let side = |text: &str| DiffSide::new("abc1234", text, None, language);
    let view = DiffView::new(
        side("let old = 1;\n"),
        side("let fresh = 2;\nlet more = 3;\n"),
    );
    app.editor.show_diff_in(None, "main.rs", view);
    paint(&mut app);

    let at = |app: &App, side: usize, line: usize, col: usize, gutter: bool| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
        let pane = parts.panes[side];
        let y = parts.body.y + app.fonts.line_height * (line as f32 + 0.5);
        let x = if gutter {
            pane.gutter.x + pane.gutter.w / 2.0
        } else {
            pane.text.x + crate::app::diff::DIFF_TEXT_PAD + app.metrics.cell * col as f32
        };
        (x, y)
    };
    let selected = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        assert!(!view.editing);
        view.read_selection
            .map(|s| s.side)
            .zip(view.read_selected())
    };

    // 拖选一段。
    let (from, to) = (at(&app, RIGHT, 0, 4, false), at(&app, RIGHT, 0, 9, false));
    app.on_mouse_move(from.0, from.1);
    assert!(app.over_diff_text(from.0, from.1));
    assert!(!app.over_diff_editable(from.0, from.1));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(to.0, to.1);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(selected(&app), Some((RIGHT, "fresh".to_owned())));

    // 光标位于拖动终点；选区正常绘制，⌘C 复制右侧的内容。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_caret(), Some((RIGHT, 9)));
    paint(&mut app);
    assert!(app.run_action(Action::Copy));
    assert_eq!(app.clipboard.copied.as_deref(), Some("fresh"));

    // ⌘A 选中右侧全部。
    assert!(app.run_action(Action::SelectAll));
    let all = "let fresh = 2;\nlet more = 3;\n".to_owned();
    assert_eq!(selected(&app), Some((RIGHT, all)));
    // 光标落在末尾换行符之后：没有对应的显示行，绘制在最后一行下面。
    paint(&mut app);

    // 点击行号选中整行；右键菜单只有复制，选区保留。
    app.pointer.last_click = None;
    let number = at(&app, RIGHT, 1, 0, true);
    click(&mut app, number);
    let line = "let more = 3;\n".to_owned();
    assert_eq!(selected(&app), Some((RIGHT, line.clone())));
    let inside = at(&app, RIGHT, 1, 3, false);
    right_click(&mut app, inside);
    assert_eq!(menu_commands(&app), vec![(MenuCommand::Copy, true)]);
    assert_eq!(selected(&app), Some((RIGHT, line)));
    app.dismiss_popup();

    // 选区只有一个：双击左侧时选区切换到左侧。
    app.pointer.last_click = None;
    let left = at(&app, LEFT, 0, 4, false);
    click(&mut app, left);
    click(&mut app, left);
    assert_eq!(selected(&app), Some((LEFT, "old".to_owned())));

    // 单击放置光标，没有选中内容；点击其他位置之前光标一直保留。
    app.pointer.last_click = None;
    let spot = at(&app, RIGHT, 1, 4, false);
    click(&mut app, spot);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selected(), None);
    assert_eq!(view.read_caret(), Some((RIGHT, 19)));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn word_highlights_keep_distinct_colors_in_the_editor_and_both_diff_panes() {
    use crate::keymap::{KeyCode, Mods, Stroke};
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT, ReadSelection};

    let dir = sample_folder("word-highlights");
    let file = dir.join("short.rs");
    let text = "alpha beta alphabet alpha\n";
    std::fs::write(&file, text).unwrap();
    let mut app = app_with(&file, false);
    let source_path = app.editor.active().unwrap().path.clone();
    app.focus = Focus::Editor;
    let command = if cfg!(target_os = "macos") {
        Mods::META
    } else {
        Mods::CTRL
    };
    let toggle = Stroke {
        key: KeyCode::F(7),
        mods: command | Mods::SHIFT,
    };
    let clear = Stroke {
        key: KeyCode::Escape,
        mods: Mods::SHIFT,
    };
    let pixel = |app: &App, x: f32, y: f32| {
        let renderer = app.frame.renderer.as_ref().unwrap();
        let image = renderer.image().unwrap();
        let scale = renderer.scale_factor();
        image.pixels[(y * scale) as usize * image.width as usize + (x * scale) as usize]
    };
    let editor_colors = |app: &App| {
        let text = app.layout.text;
        [2, 7, 12, 22].map(|col| {
            pixel(
                app,
                text.x + TEXT_PAD + col as f32 * app.metrics.cell,
                text.y + 3.0,
            )
        })
    };
    // `*` 不是快捷键，在词前、词中和选区上都照常输入。
    let star = Stroke {
        key: KeyCode::Char('8'),
        mods: Mods::SHIFT,
    };
    for (a, b) in [(0, 0), (2, 2), (0, 5)] {
        app.select(a, b);
        assert!(!app.on_shortcut(star), "{a}..{b}");
    }
    // 没有高亮时 `⇧Esc` 不作为快捷键处理。
    assert!(!app.on_shortcut(clear));
    app.select(1, 1);
    paint(&mut app);
    let automatic = editor_colors(&app);
    assert!(app.on_shortcut(toggle));
    paint(&mut app);
    let pinned = editor_colors(&app);
    assert_ne!(pinned[0], automatic[0], "光标停在词上时立即显示固定颜色");
    assert_ne!(pinned[3], automatic[3], "其他出现位置也立即更新颜色");
    app.select(6, 10);
    assert!(app.on_shortcut(toggle));
    assert!(!app.bars.find.open);
    assert_eq!(
        app.editor.active().unwrap().selection,
        Selection::new(6, 10)
    );
    app.select(0, 0);
    paint(&mut app);
    let colors = editor_colors(&app);
    assert_ne!(colors[0], colors[1]);
    assert_ne!(colors[0], colors[2]);

    let language = winc_syntax::detect("short.rs");
    let old = DiffSide::new("old", text, None, language);
    let new = DiffSide::new("new", text, None, language);
    app.editor
        .show_diff_in(None, "short.rs", DiffView::new(old, new));
    let view = app
        .editor
        .active_mut()
        .unwrap()
        .diff
        .as_deref_mut()
        .unwrap();
    view.read_selection = Some(ReadSelection {
        side: LEFT,
        range: (6, 10),
    });
    // 同一个词可以在另一侧取消并重新标注，保持同一颜色槽位。
    assert!(app.on_shortcut(toggle));
    assert_eq!(app.word_highlights.len(), 1);
    assert!(app.on_shortcut(toggle));
    let diff_colors = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let parts = diff_parts(app.layout.editor, view, app.metrics.cell);
        parts.panes.map(|pane| {
            [1, 7, 12].map(|col| {
                let x = pane.text.x + DIFF_TEXT_PAD + col as f32 * app.metrics.cell;
                pixel(app, x, parts.body.y + 3.0)
            })
        })
    };
    paint(&mut app);
    let marked = diff_colors(&app);
    assert!(app.on_shortcut(clear));
    paint(&mut app);
    let plain = diff_colors(&app);
    for (marked, plain) in marked.into_iter().zip(plain) {
        assert_ne!(marked[0], plain[0]);
        assert_ne!(marked[1], plain[1], "差异选区内仍能看到固定颜色");
        assert_ne!(marked[0], marked[1]);
        assert_eq!(marked[2], plain[2], "整词匹配不标注 alphabet");
    }

    let old = DiffSide::new("old", text, None, language);
    let new = DiffSide::new("worktree", text, source_path, language);
    let mut view = DiffView::new(old, new);
    view.editing = true;
    app.editor.show_diff_in(None, "short.rs (worktree)", view);
    assert!(app.on_shortcut(toggle), "工作区差异使用右侧文件的光标");
    assert_eq!(app.word_highlights.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stepping_on_a_read_only_side_shows_the_caret_there() {
    use winc_editor::diff_view::{DiffSide, DiffView, RIGHT};

    let dir = sample_folder("diff-step-read-only");
    let mut app = app_with(&dir.join("short.rs"), false);
    let language = winc_syntax::detect("main.rs");
    let side = |text: &str| DiffSide::new("abc1234", text, None, language);
    let view = DiffView::new(side("one\ntwo\nthree\n"), side("one\nTWO\nthree\n"));
    app.editor.show_diff_in(None, "main.rs", view);
    paint(&mut app);
    // 不是日志或提交视图打开的差异：没有文件可切换，也不显示按钮。
    assert!(!app.diff_steps_files());
    assert!(
        !app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::NextFile)
    );

    assert!(app.run_action(Action::NextDifference));
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.focus.map(|f| f.0), Some(0));
    assert!(!view.editing);
    assert_eq!(view.read_caret(), Some((RIGHT, 4)));
    paint(&mut app);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn each_side_of_a_diff_has_a_horizontal_scrollbar_and_the_two_move_together() {
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT};

    let dir = sample_folder("diff-hscroll");
    let mut app = app_with(&dir.join("short.rs"), false);
    // 每行宽 300 列，远超一侧文字区的宽度。
    let old: String = (0..20)
        .map(|i| format!("line {i} {}\n", "x".repeat(300)))
        .collect();
    let new = old.replacen("line 3 ", "line three ", 1);
    let language = winc_syntax::detect("long.txt");
    let view = DiffView::new(
        DiffSide::new("old", &old, None, language),
        DiffSide::new("new", &new, None, language),
    );
    app.editor.show_diff("long.txt", view);
    paint(&mut app);
    let bars = app.diff_hscroll().unwrap();
    let [Some(left), Some(right)] = bars.tracks else {
        panic!("并排时两侧各有一条轨道");
    };
    assert!(bars.max > 0.0 && left.right() < right.x);
    let scroll_x = |app: &App| app.editor.active().unwrap().scroll_x;
    let drag_to = |app: &mut App, x: f32, y: f32| {
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(x, y);
        app.on_mouse_up(MouseButton::Left);
        paint(app);
    };

    // 按住左侧的滑块拖到右端：滚动到最右边，两侧的滑块都显示，且都位于各自轨道的右端。
    let y = left.bottom() - 4.0;
    app.on_mouse_move(left.x + 8.0, y);
    assert_eq!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::DiffLeftHorizontal)
    );
    drag_to(&mut app, left.right(), y);
    assert!((scroll_x(&app) - bars.max).abs() < 1.0);
    assert_eq!(app.scrollbar_shown(ScrollArea::DiffLeftHorizontal), 1.0);
    assert_eq!(app.scrollbar_shown(ScrollArea::DiffRightHorizontal), 1.0);
    let end = |side: usize, track: Rect| {
        let (_, thumb) = bars.thumb(side, bars.max).unwrap();
        track.right() - thumb.right()
    };
    assert_eq!(end(LEFT, left), end(RIGHT, right));

    // 右侧的滑块拖回左端：两侧一起回到行首。
    let y = right.bottom() - 4.0;
    app.on_mouse_move(right.right() - 8.0, y);
    assert_eq!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::DiffRightHorizontal)
    );
    drag_to(&mut app, right.x, y);
    assert_eq!(scroll_x(&app), 0.0);

    // 单栏时两侧显示在同一个文字区，只有一条；自动换行时没有横向滚动，也没有滑块。
    fn diff(app: &mut App) -> &mut DiffView {
        let doc = app.editor.active_mut().unwrap();
        doc.diff.as_deref_mut().unwrap()
    }
    diff(&mut app).set_unified(true);
    let bars = app.diff_hscroll().unwrap();
    assert!(bars.tracks[LEFT].is_none() && bars.thumb(RIGHT, 0.0).is_some());
    diff(&mut app).set_unified(false);
    assert!(diff(&mut app).set_wrap(Some((40, 40))));
    let bars = app.diff_hscroll().unwrap();
    assert_eq!(bars.max, 0.0);
    assert!(bars.thumb(LEFT, 0.0).is_none() && bars.thumb(RIGHT, 0.0).is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}
