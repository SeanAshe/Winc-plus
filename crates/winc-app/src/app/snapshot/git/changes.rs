//! 改动标记的测试：行号栏与概览标尺上的标记、回滚、跳转，以及关闭标签时释放。

use super::*;

#[test]
fn closing_a_tab_lets_go_of_its_blame_and_change_marks() {
    let Some((mut app, root)) = git_app("git-close-tab", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let (id, path) = (doc.id, doc.path.clone().unwrap());
    assert!(app.blame.is_on(id) && app.blame_of(id).is_some());
    assert!(!app.changes.hunks(id).is_empty());
    assert!(app.changes.base(&path).is_some());

    // 关闭标签后逐行追溯、改动标记和 `HEAD` 中的内容都要释放，不保留到会话结束。
    app.remove_tabs(&[id]);
    assert!(!app.blame.is_on(id) && app.blame_of(id).is_none());
    assert!(app.changes.hunks(id).is_empty());
    assert!(app.changes.base(&path).is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gutter_marks_show_changes_and_revert_them() {
    let Some((mut app, root)) = git_app("git-gutter", true) else {
        return;
    };
    // `long_repo`：第 6 行改了（`value_5 = 500`），第 51 行删了。
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    let id = app.editor.active().unwrap().id;
    let hunks = app.changes.hunks(id).to_vec();
    assert_eq!(hunks.len(), 2, "{hunks:?}");
    assert_eq!(hunks[0].new, 5..6);
    assert!(hunks[1].new.is_empty());

    // 再加一行：新增的块。
    let end = app.editor.active().unwrap().text().end_pos();
    app.select(end, end);
    app.run(winc_editor::Command::Insert("let added = 1;\n".into()));
    paint(&mut app);
    let hunks = app.changes.hunks(id).to_vec();
    assert_eq!(hunks.len(), 3);
    assert!(hunks[2].old.is_empty());

    // ⌥F5 在改动之间跳转。
    app.select(0, 0);
    assert!(app.run_action(Action::NextDifference));
    let line = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.text().line_of(doc.selection.head)
    };
    assert_eq!(line(&app), 5);
    assert!(app.run_action(Action::NextDifference));
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(line(&app), 5);

    // 点行号栏的标记：打开卡片；点「回滚」：第 6 行恢复为 `HEAD` 中的内容。
    app.select(0, 0);
    app.editor.active_mut().unwrap().scroll_line = 0.0;
    paint(&mut app);
    let l = app.layout;
    let y = l.text.y + 5.5 * app.fonts.line_height;
    let x = crate::app::mark_x(l.gutter) + 1.0;
    assert_eq!(app.hover_at(x, y), Hover::ChangeMark(0));
    click(&mut app, (x, y));
    assert!(app.changes.popup.is_some());
    paint(&mut app);
    // 卡片贴在第 6 行下面，上边落在行与行的分界上，高度是行高的整数倍。
    let lh = app.fonts.line_height;
    let card = app.hits.changes_card.unwrap();
    assert_eq!(card.y - 1.0, l.text.y + 6.0 * lh);
    assert_eq!(((card.h + 2.0) / lh).fract(), 0.0);
    let revert = app
        .hits
        .changes
        .iter()
        .find(|(_, t)| *t == ChangeTarget::Revert)
        .unwrap()
        .0;
    click(&mut app, (revert.x + 4.0, revert.y + 4.0));
    assert!(app.changes.popup.is_none());
    let text = app
        .editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap();
    assert!(text.contains("let value_5 = 5;\n"), "{text}");
    paint(&mut app);
    assert_eq!(app.changes.hunks(id).len(), 2);

    // 是一次普通的修改：⌘Z 撤销。
    app.run(winc_editor::Command::Undo);
    paint(&mut app);
    assert_eq!(app.changes.hunks(id).len(), 3);

    // 命令「回滚光标所在的改动」：光标放在新加的那一行。
    let last = app.editor.active().unwrap().text().line_count() - 2;
    let pos = app.editor.active().unwrap().text().line_start(last);
    app.select(pos, pos);
    assert!(app.run_action(Action::GitRevertChange));
    paint(&mut app);
    assert_eq!(app.changes.hunks(id).len(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ruler_marks_point_at_changes_and_jump_there() {
    let Some((mut app, root)) = git_app("git-ruler", true) else {
        return;
    };
    // `long_repo`：第 6 行改了，原来的第 51 行删了。
    app.open_path(&root.join("src/main.rs"));
    app.select(0, 0);
    paint(&mut app);
    let id = app.editor.active().unwrap().id;
    let doc = app.editor.active().unwrap();
    let ruler = crate::layout::Ruler::new(app.layout.editor, doc.row_count());
    let rect = ruler.rect(
        crate::layout::GIT_LANE,
        crate::app::ruler_rows(doc, &app.changes.hunks(id)[1]),
    );
    let label = app.ruler_tip(1).expect("第二处更改有标记").1;
    assert_eq!(label, "删除 1 行 · 第 50 行之后");
    assert_eq!(app.ruler_tip(0).unwrap().1, "修改 · 第 6 行");

    // 悬停：Git 那一列（比色条略宽）算作标记，右侧滚动条一列不算。
    let (x, y) = (rect.x + 1.0, rect.y + rect.h / 2.0);
    assert_eq!(app.hover_at(x, y), Hover::RulerChange(1));
    assert_eq!(app.hover_at(x - 2.0, y), Hover::RulerChange(1));
    assert_ne!(app.hover_at(rect.right() + 3.0, y), Hover::RulerChange(1));

    // 点击：光标到那一处，滚到该处可见，不打开卡片。
    click(&mut app, (x, y));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().line_of(doc.selection.head), 50);
    assert!(
        doc.scroll_line > 0.0 && doc.scroll_line <= 47.0,
        "{}",
        doc.scroll_line
    );
    assert!(app.changes.popup.is_none());
    assert_eq!(app.focus, Focus::Editor);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：行号栏的改动标记和点开的卡片，浅色、深色各一套：两行修改（`gutter-modified`）、
/// 新增（`gutter-added`）、删除（`gutter-deleted`）。
#[test]
#[ignore]
fn shoot_gutter_marks() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-gutter-shot-{theme}"), true, dark)
        else {
            return;
        };
        app.open_path(&root.join("src/main.rs"));
        // long_repo 已经改了第 6 行、删了第 51 行；再改第 3、4 行，在第 9 行前加一行。
        let doc = app.editor.active().unwrap();
        let (start, end) = (doc.text().line_start(2), doc.text().line_start(4));
        app.select(start, end);
        app.run(winc_editor::Command::Insert(
            "let value_2 = compute(20);\nlet value_3 = value_2 * 3;\n".into(),
        ));
        let pos = app.editor.active().unwrap().text().line_start(8);
        app.select(pos, pos);
        app.run(winc_editor::Command::Insert("let extra = 8;\n".into()));
        app.editor.active_mut().unwrap().scroll_line = 0.0;
        paint(&mut app);
        app.open_change_popup(0);
        shoot(&mut app, &format!("gutter-modified-{theme}"));
        // 鼠标停在「回滚」上，提示已显示。
        let revert = app
            .hits
            .changes
            .iter()
            .find(|(_, t)| *t == ChangeTarget::Revert)
            .unwrap()
            .0;
        app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
        app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("gutter-tip-{theme}"));
        app.on_mouse_move(0.0, 0.0);
        app.open_change_popup(2);
        shoot(&mut app, &format!("gutter-added-{theme}"));
        app.editor.active_mut().unwrap().scroll_line = 40.0;
        app.open_change_popup(3);
        shoot(&mut app, &format!("gutter-deleted-{theme}"));

        // 很长的改动块：整屏都是这段新增，点中间的标记，卡片贴在点的那一行下面。
        let pos = app.editor.active().unwrap().text().line_start(20);
        app.select(pos, pos);
        let long: String = (0..80).map(|i| format!("let added_{i} = {i};\n")).collect();
        app.run(winc_editor::Command::Insert(long));
        app.changes.popup = None;
        app.editor.active_mut().unwrap().scroll_line = 30.0;
        paint(&mut app);
        let l = app.layout;
        let lh = app.fonts.line_height;
        let y = l.text.y + 12.5 * lh;
        click(&mut app, (crate::app::mark_x(l.gutter) + 1.0, y));
        paint(&mut app);
        let card = app.hits.changes_card.expect("卡片打开了");
        assert_eq!(card.y - 1.0, l.text.y + 13.0 * lh);
        shoot(&mut app, &format!("gutter-long-{theme}"));

        // 概览标尺：光标放在出现两次的 `value_2` 上，第 30 行有书签；左边一道是 Git 改动。
        app.changes.popup = None;
        let doc = app.editor.active().unwrap();
        let pos = doc.text().line_start(29);
        app.select(pos, pos);
        app.run_action(Action::ToggleBookmark);
        let pos = app.editor.active().unwrap().text().line_start(2) + 6;
        app.select(pos, pos);
        app.editor.active_mut().unwrap().scroll_line = 0.0;
        shoot(&mut app, &format!("gutter-ruler-{theme}"));
        // 鼠标停在标尺上那一长段新增上，提示已显示。
        let (rect, _) = app.ruler_tip(3).unwrap();
        app.on_mouse_move(rect.x + 1.0, rect.y + 60.0);
        app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("gutter-ruler-tip-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 截图：概览标尺在一段较长的 Python 文件中的显示：几处大小不一的改动、光标所在词的多处出现、
/// 一个书签。
///
/// `ruler-{theme}` 只有光标，`ruler-select-{theme}` 选中了该词。
#[test]
#[ignore]
fn shoot_overview_ruler() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-ruler-shot-{theme}"), false, dark)
        else {
            return;
        };
        let line = |i: usize| match i % 12 {
            0 => format!("def handler_{i}(request, context):"),
            1 => format!("    \"\"\"处理第 {i} 类请求，返回结构化的结果。\"\"\""),
            2 => "    payload = request.get(\"payload\") or {}".to_owned(),
            3 if i % 36 == 3 => "    options = config.get(\"options\", {})".to_owned(),
            3 => "    options = payload.get(\"options\", {})".to_owned(),
            4 => format!("    result = compute_{i}(payload, options=options, strict=True)"),
            5 => "    if not result:".to_owned(),
            6 => "        return None".to_owned(),
            7 => "    return {\"id\": result.id, \"items\": [str(x) for x in result.items]}"
                .to_owned(),
            _ => String::new(),
        };
        let text: String = (0..320).map(|i| line(i) + "\n").collect();
        std::fs::write(root.join("src/app.py"), &text).unwrap();
        git_in(&root, &["add", "."]);
        git_in(&root, &["commit", "-q", "-m", "app"]);
        app.git_refresh();
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        lines[40] = "    payload = normalize(request.get(\"payload\"))".to_owned();
        lines[41] = "    options = config.merge(payload.get(\"options\"))".to_owned();
        lines.splice(
            96..96,
            (0..14).map(|i| format!("    audit.record(\"step_{i}\", config)")),
        );
        lines.remove(170);
        lines[205] = "    return None  # 暂不处理".to_owned();
        lines.splice(270..270, (0..3).map(|i| format!("# TODO {i}")));
        std::fs::write(root.join("src/app.py"), lines.join("\n") + "\n").unwrap();
        app.open_path(&root.join("src/app.py"));
        // 书签在第 130 行；光标放在第 42 行的 `config` 上。
        let pos = app.editor.active().unwrap().text().line_start(129);
        app.select(pos, pos);
        app.run_action(Action::ToggleBookmark);
        let pos = app.editor.active().unwrap().text().line_start(41) + 14;
        app.select(pos, pos);
        app.editor.active_mut().unwrap().scroll_line = 30.0;
        paint(&mut app);
        shoot(&mut app, &format!("ruler-{theme}"));
        // 选中 `config`：右边一道标出该词的每一处出现。
        app.select(pos, pos + 6);
        shoot(&mut app, &format!("ruler-select-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
