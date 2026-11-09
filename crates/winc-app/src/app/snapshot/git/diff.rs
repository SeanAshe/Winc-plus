//! 并排差异的测试：打开与逐处跳转、编辑右侧、从左侧取回改动、滚动、点击，以及跟随文件更新。

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
    let hint = app.status_message.clone().unwrap();
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
        .diff_hits
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
        app.diff_hits
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
    let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
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
    assert!(app.search_open);
    assert!(app.layout.search.y >= app.layout.editor.y + crate::app::diff::DIFF_HEADER);
    app.find_input.set_text("value_55 ");
    app.search.query = "value_55 ".into();
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

/// 截图：「»」替换一段改动后，该段颜色淡出到一半时的画面。
#[test]
#[ignore]
fn shoot_diff_replace_fade() {
    let dir = search_folder("git-diff-fade");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    let file = app.git.root.clone().unwrap().join("src/main.rs");
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    app.press_diff(DiffTarget::Revert(0));
    let doc = app.editor.active().unwrap().id;
    app.diff_fade = Some(crate::app::diff::DiffFade {
        doc,
        left: 5..6,
        right: 5..6,
        kind: crate::app::HunkKind::Modified,
        clock: crate::app::animation::Clock::frozen(
            std::time::Duration::from_millis(120),
            std::time::Duration::from_millis(420),
        ),
    });
    shoot(&mut app, "diff-fade");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：在差异中查找，右侧标出匹配。
#[test]
#[ignore]
fn shoot_diff_search() {
    let dir = search_folder("git-diff-search");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    app.run_action(Action::GitShowChanges);
    paint(&mut app);
    app.run_action(Action::Find);
    app.find_input.set_text("value_4");
    app.search.query = "value_4".into();
    app.search_changed();
    shoot(&mut app, "diff-search");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：当前文件改动的并排差异，深浅两套配色。
#[test]
#[ignore]
fn shoot_git_diff() {
    for dark in [false, true] {
        let dir = search_folder(if dark {
            "git-diff-dark"
        } else {
            "git-diff-light"
        });
        if !long_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        // 在开头再添加两行且不保存：三种改动（新增、修改、删除）都有。
        app.run(winc_editor::Command::Insert(
            "use std::fs;\nuse std::io;\n".into(),
        ));
        app.run_action(Action::GitShowChanges);
        // 光标放在右侧第一处改动上。
        paint(&mut app);
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
        let lh = app.fonts.line_height;
        click(
            &mut app,
            (parts.panes[1].text.x + 120.0, parts.body.y + lh * 3.5),
        );
        shoot(&mut app, if dark { "diff-dark" } else { "diff-light" });
        // 鼠标停在第二处改动的「»」上，提示已显示。
        let revert = app
            .diff_hits
            .iter()
            .find(|(_, t)| *t == DiffTarget::Revert(1))
            .unwrap()
            .0;
        app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
        app.tooltip_since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(
            &mut app,
            if dark {
                "diff-revert-dark"
            } else {
                "diff-revert-light"
            },
        );
        // 鼠标停在第二处改动的勾选框上。
        let check = app
            .diff_hits
            .iter()
            .find(|(_, t)| *t == DiffTarget::Include(1))
            .unwrap()
            .0;
        app.on_mouse_move(check.x + 4.0, check.y + 4.0);
        app.tooltip_since = None;
        shoot(
            &mut app,
            if dark {
                "diff-check-dark"
            } else {
                "diff-check-light"
            },
        );
        // 新文件（不在 `HEAD` 中）：左侧为空。
        let text: String = (0..80)
            .map(|i| format!("pub const C{i}: u32 = {i};\n"))
            .collect();
        std::fs::write(dir.join("src/new.rs"), text).unwrap();
        app.open_path(&dir.join("src/new.rs"));
        app.run_action(Action::GitShowChanges);
        paint(&mut app);
        // 滚动一次，滚动条的滑块显示。
        app.editor.active_mut().unwrap().scroll_line = 20.0;
        shoot(
            &mut app,
            if dark {
                "diff-new-dark"
            } else {
                "diff-new-light"
            },
        );
        // 滚动到底：滑块停在卡片的圆角上方，不超出卡片。
        app.editor.active_mut().unwrap().scroll_line = 78.0;
        shoot(
            &mut app,
            if dark {
                "diff-new-bottom-dark"
            } else {
                "diff-new-bottom-light"
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn diff_scrollbar_stays_below_the_header_and_new_files_have_no_replace() {
    let Some((mut app, root)) = git_app("git-diff-track", true) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    assert!(app.run_action(Action::GitShowChanges));
    paint(&mut app);
    // 滚动条的轨道不覆盖标题栏：标题栏右端（折叠开关旁边）不是滚动条。
    let editor = app.layout.editor;
    let track = app.scroll_track();
    assert_eq!(track.y, editor.y + crate::app::diff::DIFF_HEADER);
    assert_ne!(
        app.hover_at(editor.right() - 4.0, editor.y + 10.0),
        Hover::Scrollbar(crate::layout::Scrollbar::Editor)
    );
    assert!(
        app.diff_hits
            .iter()
            .any(|(_, t)| matches!(t, DiffTarget::Revert(_)))
    );

    // 新文件：左侧为空，没有「»」（用左侧替换等于清空整个文件）。
    let text: String = (0..80)
        .map(|i| format!("pub const C{i}: u32 = {i};\n"))
        .collect();
    std::fs::write(root.join("src/fresh.rs"), text).unwrap();
    app.git_refresh();
    app.open_path(&root.join("src/fresh.rs"));
    assert!(app.run_action(Action::GitShowChanges));
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.one_side_empty());
    // 左侧表示 `HEAD` 中没有该文件，不是空文件：不写「空文件」。
    assert!(view.old.missing && !view.new.missing);
    assert!(
        !app.diff_hits
            .iter()
            .any(|(_, t)| matches!(t, DiffTarget::Revert(_)))
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn status_refreshes_rediff_a_git_diff_tab_only_when_head_moves() {
    let Some((mut app, root)) = git_app("git-diff-refresh", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    assert!(app.run_action(Action::GitShowChanges));
    let view = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        std::ptr::from_ref(view) as usize
    };
    let before = view(&app);

    // 文件已在编辑器中打开且 `HEAD` 未变：右侧随文档更新，刷新状态时不再调用 git 重新比较。
    app.git_refresh();
    assert_eq!(view(&app), before);

    // `HEAD` 改变：重新读取左侧内容。
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "second"]);
    app.git_refresh();
    assert_ne!(view(&app), before);
    let _ = std::fs::remove_dir_all(&root);
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
        let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
        let row = view.rows[side]
            .iter()
            .position(|r| matches!(r, SideRow::Line { line: l, .. } if *l == line))
            .unwrap();
        let lh = app.fonts.line_height;
        let pane = parts.panes[side];
        let x = if gutter {
            pane.gutter.x + pane.gutter.w / 2.0
        } else {
            pane.text.x + DIFF_TEXT_PAD + (col as f32 + 0.3) * app.cell
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
    app.last_click = None;
    let (x, y) = at(&app, RIGHT, 6, 0, true);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (x, y) = at(&app, RIGHT, 7, 0, true);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(selected(&app), "let value_6 = 6;\nlet value_7 = 7;\n");

    // ⌥ 单击再添加一个光标，键入的字符出现在两处；Esc 先移除多余的光标，再移除右侧的光标。
    app.last_click = None;
    let point = at(&app, RIGHT, 6, 0, false);
    click(&mut app, point);
    app.modifiers = ModifiersState::ALT;
    let point = at(&app, RIGHT, 7, 0, false);
    click(&mut app, point);
    app.modifiers = ModifiersState::empty();
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
    app.last_click = None;
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
        app.context_menu.as_ref().unwrap().target,
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
    app.last_click = None;
    let word = at(&app, LEFT, 5, 6, false);
    press(&mut app, word);
    press(&mut app, word);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.editing);
    assert_eq!(view.read_selected().unwrap(), "value_5");
    app.last_click = None;
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
        let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
        let pane = parts.panes[side];
        let y = parts.body.y + app.fonts.line_height * (line as f32 + 0.5);
        let x = if gutter {
            pane.gutter.x + pane.gutter.w / 2.0
        } else {
            pane.text.x + crate::app::diff::DIFF_TEXT_PAD + app.cell * col as f32
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
    assert_eq!(app.copied.as_deref(), Some("fresh"));

    // ⌘A 选中右侧全部。
    assert!(app.run_action(Action::SelectAll));
    let all = "let fresh = 2;\nlet more = 3;\n".to_owned();
    assert_eq!(selected(&app), Some((RIGHT, all)));
    // 光标落在末尾换行符之后：没有对应的显示行，绘制在最后一行下面。
    paint(&mut app);

    // 点击行号选中整行；右键菜单只有复制，选区保留。
    app.last_click = None;
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
    app.last_click = None;
    let left = at(&app, LEFT, 0, 4, false);
    click(&mut app, left);
    click(&mut app, left);
    assert_eq!(selected(&app), Some((LEFT, "old".to_owned())));

    // 单击放置光标，没有选中内容；点击其他位置之前光标一直保留。
    app.last_click = None;
    let spot = at(&app, RIGHT, 1, 4, false);
    click(&mut app, spot);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selected(), None);
    assert_eq!(view.read_caret(), Some((RIGHT, 19)));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn find_in_a_commit_diff_searches_the_selected_snapshot() {
    use crate::keymap::{KeyCode, Mods, Os, Stroke};
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT};

    let dir = sample_folder("diff-commit-find");
    let mut app = app_with(&dir.join("short.rs"), false);
    let language = winc_syntax::detect("main.rs");
    let side = |text: &str| DiffSide::new("abc1234", text, None, language);
    let common = format!(
        "{}命中一\n{}",
        "context\n".repeat(70),
        "context\n".repeat(70)
    );
    let old = format!("左侧\n{common}old\n");
    let new = format!("右侧\n{common}命中二\n");
    let view = DiffView::new(side(&old), side(&new));
    app.editor.show_diff_in(None, "main.rs", view);
    paint(&mut app);

    let find = Stroke {
        key: KeyCode::Char('f'),
        mods: if Os::CURRENT == Os::Mac {
            Mods::META
        } else {
            Mods::CTRL
        },
    };
    let set_query = |app: &mut App, query: &str| {
        app.find_input.set_text(query);
        app.search.query = query.to_owned();
        app.search_changed();
    };
    let selection = |app: &App| {
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        assert!(!view.editing);
        view.read_selection.unwrap()
    };
    assert!(app.on_shortcut(find));
    assert!(app.search_open);
    assert_eq!(app.focus, crate::app::Focus::Search);
    set_query(&mut app, "命中");
    assert_eq!(app.search.matches().len(), 2);
    let first = selection(&app);
    assert_eq!(first.side, RIGHT);
    let doc = app.editor.active().unwrap();
    let view = doc.diff.as_deref().unwrap();
    let row = view.row_of_new(71);
    assert_eq!(view.line_at(RIGHT, row), Some(71));
    assert!(doc.scroll_line > 0.0);
    app.advance_search();
    paint(&mut app);
    assert_eq!(app.search.current_match(), Some(first.range));

    app.find_next(false);
    assert!(selection(&app).range.0 > first.range.0);
    app.find_next(true);
    assert_eq!(selection(&app), first);
    app.press_search_button(crate::layout::SearchButton::Expand);
    assert!(!app.replace_open);
    app.close_search();

    let doc = app.editor.active_mut().unwrap();
    doc.scroll_line = 0.0;
    let view = doc.diff.as_deref().unwrap();
    let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
    let point = (
        parts.panes[LEFT].text.x + crate::app::diff::DIFF_TEXT_PAD + app.cell,
        parts.body.y + 4.0,
    );
    click(&mut app, point);
    assert!(app.on_shortcut(find));
    assert_eq!(app.search.query, "左侧");
    assert_eq!(selection(&app).side, LEFT);
    set_query(&mut app, "命中");
    assert_eq!(app.search.matches().len(), 1);

    // 同一标签换成另一份差异后，同一侧的旧结果作废。
    let id = app.editor.active().unwrap().id;
    let deleted = DiffView::new(side("命中\n命中\n命中\n"), side("").with_missing(true));
    app.editor.show_diff_in(Some(id), "deleted.rs", deleted);
    app.advance_search();
    assert_eq!(app.search.matches().len(), 3);
    app.find_next(false);
    assert_eq!(selection(&app).side, LEFT);

    std::fs::remove_dir_all(&dir).unwrap();
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
    app.search.query.clear();
    app.open_search(true);
    assert!(app.search_open && !app.replace_open);
    assert_eq!(app.search.query, "");

    // 长匹配横向滚动到能显示其末端，且不越过起点。
    app.find_input
        .set_text("long_identifier_at_the_far_right_end_of_the_line");
    app.search.query = "long_identifier_at_the_far_right_end_of_the_line".into();
    app.search_changed();
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let view = doc.diff.as_deref().unwrap();
    let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
    let width = parts.panes[LEFT].text.w - 2.0 * crate::app::diff::DIFF_TEXT_PAD;
    let (x, x_end) = (401.0 * app.cell, 449.0 * app.cell);
    assert!(
        doc.scroll_x <= x && x_end <= doc.scroll_x + width,
        "{}",
        doc.scroll_x
    );

    // 查找快照时，其他文档等待中的跳转作废。
    app.pending_jump = Some((other, 0, false));
    app.advance_search();
    assert_eq!(app.pending_jump, None);

    // 单栏布局：未改动的行按右侧绘制，查找左侧时同样标出匹配。
    app.close_search();
    let doc = app.editor.active_mut().unwrap();
    doc.scroll_x = 0.0;
    doc.scroll_line = 0.0;
    doc.diff.as_deref_mut().unwrap().set_unified(true);
    set_read(&mut app, LEFT, (0, 0));
    paint(&mut app);
    let row = |app: &App| {
        let renderer = app.renderer.as_ref().unwrap();
        let (scale, image) = (renderer.scale_factor(), renderer.image().unwrap());
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
        // 单栏的第 0 行是未改动的 `a -> b`。
        let y = ((parts.body.y + app.fonts.line_height * 0.5) * scale) as usize;
        let w = image.width as usize;
        image.pixels[y * w..(y + 1) * w].to_vec()
    };
    let plain = row(&app);
    app.open_search(false);
    app.find_input.set_text("->");
    app.search.query = "->".into();
    app.search_changed();
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(view.read_selection.map(|s| s.side), Some(LEFT));
    assert_ne!(row(&app), plain, "左侧的匹配在单栏的未改动行上标出");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stepping_through_changes_marks_each_one_and_walks_the_files() {
    let dir = search_folder("git-diff-step");
    std::fs::write(dir.join("src/zeta.rs"), "a\nb\nc\n").unwrap();
    if !long_repo(&dir) {
        return;
    }
    std::fs::write(dir.join("src/zeta.rs"), "a\nB\nc\nd\n").unwrap();
    // 只保留两个有改动的文件。
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
    let tabs = app.editor.documents.len();
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };
    let caret_line = |app: &App| {
        let source = app
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(&main));
        let doc = source.unwrap();
        doc.text().pos_to_line_col(doc.selection.head).0
    };
    assert!(title(&app).starts_with("main.rs"));
    assert!(app.diff_steps_files());

    // 打开时不标记。第一次跳转标记第一处；文件已经打开，光标移到它的文档里，键入的字符写入右侧。
    assert_eq!(focus(&app), None);
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(caret_line(&app), 5);
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(1));
    assert_eq!(caret_line(&app), 50);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.editing);
    paint(&mut app);

    // 越过最后一处：第一次提示，文件不变。
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(
        app.status_message.as_deref(),
        Some("已是最后一处更改。再次执行可切换到下一个文件：zeta.rs")
    );
    // 滚动后需要重新确认：此次按视口的位置查找改动，不切换文件；遍历各处改动后才再次提示。
    app.editor.active_mut().unwrap().scroll_line += 1.0;
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(app.diff_cross, None);
    for _ in 0..3 {
        if app.diff_cross.is_none() {
            assert!(app.run_action(Action::NextDifference));
        }
    }
    assert!(title(&app).starts_with("main.rs"));
    // 提示消失后同样需要重新确认。
    let (at, cross) = app.diff_cross.unwrap();
    app.diff_cross = Some((at - std::time::Duration::from_secs(60), cross));
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("main.rs"));

    // 再按一次进入下一个文件，仍在同一个标签，停在第一处；回到上一个文件时位于其最后一处。
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("zeta.rs"), "{}", title(&app));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(focus(&app), Some(0));
    // 该文件未打开：不为它新开标签，光标是右侧的只读光标。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.editing);
    assert_eq!(view.read_caret(), Some((winc_editor::diff_view::RIGHT, 2)));
    paint(&mut app);
    // 没有更多文件时不提示再按一次。
    assert!(app.run_action(Action::NextDifference));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(app.status_message.as_deref(), Some("没有下一处更改"));
    // 该文件有两处改动：先回到第一处，再提示，第三次回到上一个文件。
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(focus(&app), Some(0));
    assert!(app.run_action(Action::PreviousDifference));
    assert!(title(&app).starts_with("zeta.rs"));
    assert!(app.run_action(Action::PreviousDifference));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(focus(&app), Some(1));

    // `F8` 切换文件时总是从第一处开始；到达末端时提示，位置不变。
    assert!(app.run_action(Action::PreviousDiffFile));
    assert_eq!(app.status_message.as_deref(), Some("没有上一个文件"));
    assert!(app.run_action(Action::NextDiffFile));
    assert!(title(&app).starts_with("zeta.rs"));
    assert!(app.run_action(Action::PreviousDiffFile));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(app.editor.documents.len(), tabs);

    // 点击差异后不再标记。
    paint(&mut app);
    let parts = crate::app::diff_parts(
        app.layout.editor,
        app.editor.active().unwrap().diff.as_deref().unwrap(),
        app.cell,
    );
    click(
        &mut app,
        (parts.panes[0].text.x + 20.0, parts.body.y + 30.0),
    );
    assert_eq!(focus(&app), None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn moving_to_a_change_clears_the_message_about_having_none() {
    let dir = search_folder("git-diff-messages");
    if !long_repo(&dir) {
        return;
    }
    std::fs::remove_file(dir.join("src/new.rs")).unwrap();
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };

    // 刚打开时向回没有改动；向下跳转到第一处后，该提示不再成立。
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.status_message.as_deref(), Some("没有上一处更改"));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(app.status_message, None);

    // 到达末端时提示方向对应的消息；反方向跳转后提示清除，不留下相反的消息。
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(1));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(app.status_message.as_deref(), Some("没有下一处更改"));
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(app.status_message, None);
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.status_message.as_deref(), Some("没有上一处更改"));

    // 隐藏状态栏时消息在提示条上，同样清除。
    app.settings_toggle(Setting::StatusBar);
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.notice.as_deref(), Some("没有上一处更改"));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(1));
    assert_eq!(app.notice, None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn diff_navigation_keys_work_with_focus_in_the_commit_list() {
    use crate::keymap::{KeyCode, Mods, Stroke};

    let dir = search_folder("git-diff-list-keys");
    if !long_repo(&dir) {
        return;
    }
    std::fs::remove_file(dir.join("src/new.rs")).unwrap();
    std::fs::write(dir.join("src/zeta.rs"), "a\n").unwrap();
    git_in(&dir, &["add", "src/zeta.rs"]);
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    let f = |n: u8, mods: Mods| Stroke {
        key: KeyCode::F(n),
        mods,
    };
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();

    // 点击列表中的文件后焦点保留在列表：跳到改动、切换文件都生效，焦点不移动。
    app.focus = Focus::CommitList;
    assert!(app.on_shortcut(f(7, Mods::NONE)));
    assert_eq!(focus(&app), Some(0));
    assert!(app.on_shortcut(f(5, Mods::ALT)));
    assert_eq!(focus(&app), Some(1));
    assert!(app.on_shortcut(f(7, Mods::SHIFT)));
    assert_eq!(focus(&app), Some(0));
    assert!(app.on_shortcut(f(5, Mods::ALT | Mods::SHIFT)));
    assert_eq!(app.focus, Focus::CommitList);
    assert!(app.on_shortcut(f(8, Mods::NONE)));
    assert!(title(&app).starts_with("zeta.rs"), "{}", title(&app));
    assert!(app.on_shortcut(f(8, Mods::SHIFT)));
    assert!(title(&app).starts_with("main.rs"), "{}", title(&app));
    assert_eq!(app.focus, Focus::CommitList);

    // 其他键仍然由列表处理；焦点在文件树时不操作差异。
    assert!(!app.on_shortcut(Stroke {
        key: KeyCode::Down,
        mods: Mods::NONE,
    }));
    app.focus = Focus::Tree;
    assert!(!app.on_shortcut(f(7, Mods::NONE)));

    // 当前标签不是差异时，提交列表不处理这些键。
    app.focus = Focus::CommitList;
    let tab = app.editor.active.unwrap();
    app.close_tab(tab);
    assert!(!app.editor.active().is_some_and(|d| d.diff.is_some()));
    assert!(!app.on_shortcut(f(7, Mods::NONE)));

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
    let layout = app.diff_hits.iter().any(|(_, t)| *t == DiffTarget::Layout);
    assert!(layout);

    // 切换：修改的是设置，文字区占满整个宽度，两侧共用一个文字区。
    assert!(app.run_action(Action::ToggleDiffLayout));
    assert!(view(&app).1);
    assert_eq!(app.config.diff_inline, Some(true));
    assert!(app.editor.diff_unified);
    let parts = |app: &App| {
        let doc = app.editor.active().unwrap();
        crate::app::diff_parts(app.layout.editor, doc.diff.as_deref().unwrap(), app.cell)
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
        let x = parts.panes[RIGHT].text.x + crate::app::diff::DIFF_TEXT_PAD + app.cell * col as f32;
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
    app.last_click = None;
    click(&mut app, old);
    click(&mut app, old);
    let diff = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert_eq!(diff.read_selected().as_deref(), Some("value_5"));

    // 点击新行：光标进入右侧的文档，键入的字符写入文件。
    app.last_click = None;
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
        !app.diff_hits
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
fn clicks_and_keys_in_a_diff_scroll_it_and_touch_nothing_else() {
    let Some((mut app, root)) = git_app("git-diff-keys", true) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run_action(Action::GitShowChanges);
    // 展开未改动的内容：超出一屏，可以滚动。
    app.press_diff(DiffTarget::Collapse);
    paint(&mut app);
    let scroll = app.editor.active().unwrap().scroll_line;

    // 点击正文：只获取焦点，不在不可见的文本中放置光标，视图不动；⌘ 点击也不转到定义。
    let body = app.layout.editor;
    click(&mut app, (body.x + body.w * 0.75, body.bottom() - 40.0));
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll);
    assert_eq!(app.editor.active().unwrap().selection.head, 0);

    // 键盘滚动。
    app.diff_key(&Key::Named(NamedKey::ArrowDown), false);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll + 1.0);
    app.diff_key(&Key::Named(NamedKey::Home), false);
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    app.diff_key(&Key::Named(NamedKey::End), false);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    app.diff_key(&Key::Named(NamedKey::PageUp), false);
    // 字符键不执行任何操作（差异只读）。
    let before = app.editor.active().unwrap().scroll_line;
    app.diff_key(&Key::Character("x".into()), false);
    assert_eq!(app.editor.active().unwrap().scroll_line, before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diffs_follow_the_file_and_skip_binaries_and_stale_previews() {
    let Some((mut app, root)) = git_app("git-diff-live", false) else {
        return;
    };
    let main = root.join("src/main.rs");
    app.git_preview_changes(&main);
    let id = app.editor.active().unwrap().id;
    let new_text = |app: &App| {
        let doc = app.editor.document(id).unwrap();
        doc.diff.as_deref().unwrap().new.text.to_string()
    };
    assert_eq!(new_text(&app), "fn main() {}\n");

    // 文件再次变化：状态刷新以后，已打开的差异原地更新，不切换标签。
    std::fs::write(&main, "fn main() { run() }\n").unwrap();
    app.open_path(&root.join("src/config.rs"));
    let active = app.editor.active().unwrap().id;
    app.apply_changes(winc_editor::Changes {
        paths: vec![main.clone()],
        rescan: false,
        git: false,
    });
    assert_eq!(new_text(&app), "fn main() { run() }\n");
    assert_eq!(app.editor.active().unwrap().id, active);

    // 被忽略的路径里的变化不触发刷新。
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("target/debug/app"), "x").unwrap();
    std::fs::write(root.join(".gitignore"), "/target\n").unwrap();
    app.apply_changes(winc_editor::Changes {
        paths: vec![root.join(".gitignore")],
        rescan: false,
        git: false,
    });
    let generation = app.git.generation;
    app.apply_changes(winc_editor::Changes {
        paths: vec![root.join("target/debug/app")],
        rescan: false,
        git: false,
    });
    assert_eq!(app.git.generation, generation);

    // 过期的预览请求返回时不打开。
    let tabs = app.editor.documents.len();
    let seq = app.git.preview_seq;
    app.git.preview_seq += 1;
    app.fetch_changes(&root.join("src/new.rs"), Target::Preview(seq));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(app.editor.active().unwrap().id, active);

    // 二进制文件不显示差异，在状态栏说明。
    std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 0, 3]).unwrap();
    app.git_preview_changes(&root.join("blob.bin"));
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("二进制")),
        "{:?}",
        app.status_message
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 测量：滚动差异标签与滚动普通文件时每帧的绘制耗时。
///
/// 运行方式：`cargo test -p winc-app measure_diff_scroll -- --ignored --nocapture`。
#[test]
#[ignore]
fn measure_diff_scroll() {
    use std::time::Instant;
    let dir = search_folder("git-diff-measure");
    let text: String = (0..3000)
        .map(|i| format!("    let value_{i} = compute(value_{i}, \"text {i}\");\n"))
        .collect();
    std::fs::write(dir.join("src/main.rs"), &text).unwrap();
    if !make_repo(&dir) {
        return;
    }
    let mut changed = String::new();
    for (i, line) in text.lines().enumerate() {
        match i % 25 {
            0 => changed.push_str(&line.replace("compute", "recompute")),
            7 => continue,
            13 => {
                changed.push_str(line);
                changed.push_str("\n    extra();");
            }
            _ => changed.push_str(line),
        }
        changed.push('\n');
    }
    std::fs::write(dir.join("src/main.rs"), changed).unwrap();
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    let frames = |app: &mut App, label: &str| {
        paint(app);
        let start = Instant::now();
        for _ in 0..60 {
            app.editor.active_mut().unwrap().scroll_line += 1.3;
            app.clamp_scroll();
            paint(app);
        }
        eprintln!("{label}: {:?} / 帧", start.elapsed() / 120);
    };
    frames(&mut app, "普通文件");
    app.run_action(Action::GitShowChanges);
    frames(&mut app, "差异");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：工作区删除了 `HEAD` 中间的一大段，滚动到该段中部：
/// 左侧逐行显示被删除的内容，右侧停在删除处。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_diff_long_deletion() {
    for dark in [false, true] {
        let name = if dark {
            "deletion-dark"
        } else {
            "deletion-light"
        };
        let dir = search_folder(name);
        if !make_repo(&dir) {
            return;
        }
        let lines: Vec<String> = (0..120).map(|i| format!("let value_{i} = {i};")).collect();
        std::fs::write(dir.join("long.rs"), lines.join("\n") + "\n").unwrap();
        git_in(&dir, &["add", "-A"]);
        git_in(&dir, &["commit", "-q", "-m", "long"]);
        let kept: Vec<&String> = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !(30..90).contains(i))
            .map(|(_, l)| l)
            .collect();
        let kept: String = kept.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(dir.join("long.rs"), kept).unwrap();
        let mut app = app_with(&dir.join("long.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::GitShowChanges);
        paint(&mut app);
        let doc = app.editor.active_mut().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let span = view.hunk_span(0);
        doc.scroll_line = span.start + 20.0;
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 创建一个仓库：`src/wrap.rs` 中有数行很长的代码，工作区修改了其中一行。
fn long_lines_repo(name: &str) -> Option<(PathBuf, PathBuf)> {
    let dir = search_folder(name);
    if !make_repo(&dir) {
        return None;
    }
    let long = |tag: &str| {
        format!(
            "    let {tag} = {};\n",
            "value + other * third, ".repeat(16)
        )
    };
    let text = format!("fn main() {{\n{}{}}}\n", long("first"), long("second"));
    let file = dir.join("src/wrap.rs");
    std::fs::write(&file, &text).unwrap();
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "long"]);
    let changed = text.replace("let second = value", "let second = changed");
    std::fs::write(&file, changed).unwrap();
    Some((dir, winc_editor::canonical_path(&file)))
}

#[test]
fn word_wrap_applies_to_the_diff_view() {
    use crate::app::diff::DIFF_TEXT_PAD;

    let Some((dir, file)) = long_lines_repo("git-diff-wrap") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    fn view(app: &App) -> &winc_editor::diff_view::DiffView {
        app.editor.active().unwrap().diff.as_deref().unwrap()
    }
    assert!(!view(&app).is_wrapping());
    let plain = view(&app).rows[RIGHT].len();
    app.editor.active_mut().unwrap().scroll_x = 80.0;

    // 开启：长行分为多段，横向不再滚动。
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    assert!(view(&app).is_wrapping());
    assert!(view(&app).rows[RIGHT].len() > plain);
    assert_eq!(app.editor.active().unwrap().scroll_x, 0.0);

    // 改动的字符只在其所在的段中标记：改动行后续各段的开头没有标记。
    {
        let view = view(&app);
        let row = view.rows[RIGHT]
            .iter()
            .position(|r| {
                matches!(
                    r,
                    SideRow::Line {
                        hunk: Some(_),
                        part: 1,
                        ..
                    }
                )
            })
            .unwrap();
        let SideRow::Line { line, .. } = view.rows[RIGHT][row] else {
            unreachable!();
        };
        let indent = view.breaks(RIGHT, line).indent as f32;
        let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
        let lh = app.fonts.line_height;
        let scroll = view.side_scroll(RIGHT, 0.0, f64::from(parts.body.h / lh));
        let x = parts.panes[RIGHT].text.x + DIFF_TEXT_PAD + indent * app.cell;
        let y = parts.body.y + (row as f64 - scroll) as f32 * lh + 3.0;
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let pixel = |x: f32, y: f32| {
            let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
            image.pixels[y * image.width as usize + x]
        };
        assert_eq!(pixel(x + 0.5, y), pixel(x - 6.0, y));
    }

    // 点击一行的第二段：光标落在该段中对应的列。
    let (row, line) = view(&app).rows[RIGHT]
        .iter()
        .enumerate()
        .find_map(|(i, r)| match r {
            SideRow::Line { line, part: 1, .. } => Some((i, *line)),
            _ => None,
        })
        .unwrap();
    let breaks = view(&app).breaks(RIGHT, line);
    let parts = crate::app::diff_parts(app.layout.editor, view(&app), app.cell);
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    let scroll = view(&app).side_scroll(RIGHT, 0.0, visible);
    let y = parts.body.y + ((row as f64 - scroll) as f32 + 0.5) * app.fonts.line_height;
    let cells = breaks.indent + 2;
    let x = parts.panes[RIGHT].text.x + DIFF_TEXT_PAD + (cells as f32 + 0.3) * app.cell;
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    let source = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    let (at, col) = source.text().pos_to_line_col(source.selection.head);
    assert_eq!((at, col), (line, breaks.start_of(1) + 2));
    paint(&mut app);

    // 关闭：每行恢复为一个显示行。
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    assert!(!view(&app).is_wrapping());
    assert_eq!(view(&app).rows[RIGHT].len(), plain);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_diff_view_scrolls_sideways_from_the_bottom_bar() {
    let Some((dir, file)) = long_lines_repo("git-diff-hscroll") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);

    // 长行超出一侧的文字区：正文下沿有横向滚动条，拖到右端能看到行尾。
    let track = app.hscroll_track();
    assert!(app.text_content_w > track.w);
    let y = track.bottom() - 4.0;
    app.on_mouse_move(track.x + 8.0, y);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::EditorHorizontal));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(track.right(), y);
    app.on_mouse_up(MouseButton::Left);
    let max = app.text_content_w - track.w;
    assert!((app.editor.active().unwrap().scroll_x - max).abs() < 1.0);

    // 自动换行后一行放得下，横向滚动条消失。
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    assert_eq!(app.editor.active().unwrap().scroll_x, 0.0);
    assert_eq!(app.text_content_w, 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn toggling_word_wrap_keeps_the_top_line_of_the_diff_in_place() {
    let Some((dir, file)) = uneven_lines_repo("git-diff-wrap-top") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    // 屏幕上右侧最上面的（行，段，行内偏移）。
    fn top(app: &App) -> (usize, usize, f64) {
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.cell);
        let visible = f64::from(parts.body.h / app.fonts.line_height);
        let row = view.side_scroll(RIGHT, doc.scroll_line, visible);
        let SideRow::Line { line, part, .. } = view.rows[RIGHT][row as usize] else {
            panic!("{row}");
        };
        (line, part, row.fract())
    }
    let start = {
        let doc = app.editor.active_mut().unwrap();
        let view = doc.diff.as_deref().unwrap();
        doc.scroll_line = view.pos_of(RIGHT, view.row_of_new(90) as f64 + 0.25);
        top(&app)
    };

    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    let wrapped = top(&app);
    assert_eq!((wrapped.0, wrapped.1), (start.0, 0));
    assert!((wrapped.2 - start.2).abs() < 0.01, "{wrapped:?} {start:?}");

    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    let plain = top(&app);
    assert_eq!(plain.0, start.0);
    assert!((plain.2 - start.2).abs() < 0.01, "{plain:?} {start:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 长短行相间、每隔数行删除一段的改动：顶边和中线之间总有两侧不等长的改动。
fn uneven_lines_repo(name: &str) -> Option<(PathBuf, PathBuf)> {
    let dir = search_folder(name);
    if !make_repo(&dir) {
        return None;
    }
    let line = |i: usize| match i % 3 {
        0 => format!("{i} {}\n", "value + other * third, ".repeat(16)),
        _ => format!("{i}\n"),
    };
    let file = dir.join("src/wrap.rs");
    std::fs::write(&file, (0..400).map(line).collect::<String>()).unwrap();
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "uneven"]);
    let kept = (0..400)
        .filter(|i| i % 12 >= 6)
        .map(line)
        .collect::<String>();
    std::fs::write(&file, kept).unwrap();
    Some((dir, winc_editor::canonical_path(&file)))
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

/// 截图：显示全部空白符的差异，深浅两套配色。
#[test]
#[ignore]
fn shoot_diff_whitespace() {
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        let dir = search_folder(&format!("whitespace-diff-shot-{theme}"));
        if !long_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        app.run(winc_editor::Command::Insert("use std::fs;  \n".into()));
        app.run_action(Action::GitShowChanges);
        app.config.render_whitespace = Some(winc_config::RenderWhitespace::All);
        shoot(&mut app, &format!("whitespace-diff-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
