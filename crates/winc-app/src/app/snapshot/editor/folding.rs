//! 代码折叠的测试：快捷键、行号栏的箭头、折叠行末尾的 `⋯`，以及折叠后的滚动与跳转。

use super::*;
use crate::app::snapshot::editor::navigation::{navigation_folder, press, sticky_file};
use crate::keymap::KeyCode::Char;

/// 返回当前文档的折叠范围，每项为（起始行号，结束行号），均从 0 起。
fn folded(app: &App) -> Vec<(usize, usize)> {
    let doc = app.editor.active().unwrap();
    doc.folds().iter().map(|f| (f.start, f.end)).collect()
}

fn caret_line(app: &App) -> usize {
    let doc = app.editor.active().unwrap();
    doc.text().line_of(doc.selection.head)
}

/// 返回行号栏中第 `line` 行（须未被折叠隐藏）的垂直中点。
///
/// `x` 为距行号栏左边缘的偏移（逻辑像素）。
fn gutter_point(app: &App, line: usize, x: f32) -> (f32, f32) {
    let doc = app.editor.active().unwrap();
    let row = (doc.row_of(line) as f64 - doc.scroll_line) as f32;
    let lh = app.fonts.line_height;
    (
        app.layout.gutter.x + x,
        app.layout.text.y + (row + 0.5) * lh,
    )
}

#[test]
fn folding_from_keys_the_gutter_and_the_dots() {
    let dir = navigation_folder("folding");
    let mut app = app_with(&dir, false);
    app.open_path(&sticky_file(&dir));
    let fold = if cfg!(target_os = "macos") {
        ["alt", "cmd"].as_slice()
    } else {
        ["ctrl", "shift"].as_slice()
    };

    // 光标位于 `load` 内：折叠的是 `load`，光标移到 `def` 所在行的行尾，其后的行随之上移。
    let pos = app.editor.active().unwrap().text().line_start(10);
    app.select(pos, pos);
    assert!(press(&mut app, fold, Char('[')));
    assert_eq!(folded(&app), [(3, 63)]);
    assert_eq!(caret_line(&app), 3);
    assert_eq!(app.editor.active().unwrap().row_of(65), 5);
    // ↓ 跳过被折叠隐藏的行。
    app.run(Command::MoveDown { extend: false });
    assert_eq!(caret_line(&app), 64);

    // 行末的 `⋯`：悬停时高亮，单击展开。
    paint(&mut app);
    let (chip, line) = app.hits.folded[0];
    assert_eq!(line, 3);
    let (x, y) = (chip.x + chip.w / 2.0, chip.y + chip.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::Folded(3));
    click(&mut app, (x, y));
    assert!(folded(&app).is_empty());

    // 行号栏：鼠标在其上时悬停项为 `Gutter`；点击箭头所在列折叠，再次点击展开；
    // 点击行号仍然选中整行。
    app.select(0, 0);
    paint(&mut app);
    let column = fold_column(app.layout.gutter);
    let arrow = column.x - app.layout.gutter.x + column.w / 2.0;
    let at = gutter_point(&app, 2, arrow);
    app.on_mouse_move(at.0, at.1);
    assert_eq!(app.hover, Hover::Gutter);
    click(&mut app, at);
    assert_eq!(folded(&app), [(2, 66)]);
    click(&mut app, at);
    assert!(folded(&app).is_empty());
    let number = gutter_point(&app, 0, 4.0);
    click(&mut app, number);
    assert!(folded(&app).is_empty());
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.text_range(doc.selection.from(), doc.selection.to())
            .unwrap(),
        "import os\n"
    );
    // 该行不是折叠块的起始行时，点击箭头所在列同样只是选中整行。
    let plain = gutter_point(&app, 5, arrow);
    click(&mut app, plain);
    assert!(folded(&app).is_empty());

    // ⌘K ⌘0 全部折叠，⌘K ⌘J 全部展开；⌘K ⌘2 折叠第二层，光标所在的块除外。
    assert!(press(&mut app, &["cmd"], Char('k')));
    assert!(press(&mut app, &["cmd"], Char('0')));
    assert_eq!(folded(&app), [(2, 66), (3, 63), (65, 66)]);
    assert_eq!(app.editor.active().unwrap().row_count(), 4);
    press(&mut app, &["cmd"], Char('k'));
    press(&mut app, &["cmd"], Char('j'));
    assert!(folded(&app).is_empty());
    let pos = app.editor.active().unwrap().text().line_start(66);
    app.select(pos, pos);
    press(&mut app, &["cmd"], Char('k'));
    press(&mut app, &["cmd"], Char('2'));
    assert_eq!(folded(&app), [(3, 63)]);

    // 转到被折叠隐藏的行时，先展开对应的折叠块。
    app.goto_line("30");
    assert!(folded(&app).is_empty());
    assert_eq!(caret_line(&app), 29);

    // 在上方插入一行：折叠块随之下移一行；撤销后恢复原位。
    app.run_action(crate::keymap::Action::Fold);
    assert_eq!(folded(&app), [(3, 63)]);
    app.select(0, 0);
    app.run(Command::Insert("# top\n".into()));
    assert_eq!(folded(&app), [(4, 64)]);
    app.run(Command::Undo);
    assert_eq!(folded(&app), [(3, 63)]);

    // 滚动位置按显示行计算：滚动到底时，最后一行位于编辑区高度的 60% 处，下方留出 40% 空白；
    // 折叠后内容能在一屏内显示时不滚动。
    app.editor.active_mut().unwrap().scroll_line = 1000.0;
    app.clamp_scroll();
    let doc = app.editor.active().unwrap();
    let view = f64::from(app.scroll_track().h / app.fonts.line_height);
    let rows = doc.row_count() as f64;
    let expected = if rows <= view {
        0.0
    } else {
        rows - view * (1.0 - f64::from(crate::layout::OVERSCROLL))
    };
    assert!((doc.scroll_line - expected).abs() < 1e-3);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sticky_scroll_follows_folded_rows() {
    let dir = navigation_folder("folding-sticky");
    let mut app = app_with(&dir, false);
    app.open_path(&sticky_file(&dir));
    // 折叠 `load` 后滚动到该行：置顶的是 `class`，其下显示折叠后的 `def load` 及后续的行。
    app.editor.active_mut().unwrap().toggle_fold_at(3, 4);
    app.editor.active_mut().unwrap().scroll_line = 3.0;
    paint(&mut app);
    let pinned: Vec<_> = app.hits.sticky.iter().map(|r| r.line).collect();
    assert_eq!(pinned, [2]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 记录折叠变化前的状态：折叠集合与首行位置（行，可以是小数）。
fn fold_state(app: &App) -> (winc_editor::Folds, f64) {
    let doc = app.editor.active().unwrap();
    (doc.folds().clone(), doc.scroll_line)
}

#[test]
fn folding_animates_like_the_file_tree() {
    use std::time::Duration;

    let at = |ms| Clock::frozen(Duration::from_millis(ms), FOLD_ANIMATION);
    let dir = navigation_folder("folding-motion");
    let mut app = app_with(&dir, false);
    app.open_path(&sticky_file(&dir));
    let tab_width = 4;
    paint(&mut app);

    // 折叠 `load`，方向为收起：变化的是 `def` 下方的 60 行
    // （按展开状态计，为第 4 到 63 个显示行，从 0 起）。
    let before = fold_state(&app);
    app.editor
        .active_mut()
        .unwrap()
        .toggle_fold_at(3, tab_width);
    let motion = app.fold_motion(before, at(0)).unwrap();
    assert_eq!((motion.rows.clone(), motion.expanding), (4..64, false));
    assert_eq!(motion.reveal(), 1.0);

    // 动画进行中：绘制时临时切换为展开状态，绘制后恢复，文档的折叠状态保持不变。
    app.fold_motion = Some(crate::app::FoldMotion {
        clock: at(80),
        ..motion
    });
    paint(&mut app);
    let m = app.fold_motion.as_ref().expect("动画还在");
    assert_eq!(m.reveal(), 0.125);
    assert_eq!(folded(&app), [(3, 63)]);
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);

    // 滚动位置改变时放弃动画，直接绘制最终状态。
    app.editor.active_mut().unwrap().scroll_line = 1.0;
    paint(&mut app);
    assert!(app.fold_motion.is_none());
    app.editor.active_mut().unwrap().scroll_line = 0.0;

    // 再次展开：涉及同一范围，方向为展开。
    let before = fold_state(&app);
    app.editor
        .active_mut()
        .unwrap()
        .toggle_fold_at(3, tab_width);
    let motion = app.fold_motion(before, at(0)).unwrap();
    assert_eq!((motion.rows.clone(), motion.expanding), (4..64, true));
    assert_eq!(motion.reveal(), 0.0);

    // 起始行位于屏幕上方（已滚过 `def`）时，最上面一行会变化，此时不做动画；
    // 多处同时折叠（第二层的两个方法）也不做动画。
    app.editor.active_mut().unwrap().scroll_line = 10.0;
    let before = fold_state(&app);
    let doc = app.editor.active_mut().unwrap();
    doc.toggle_fold_at(3, tab_width);
    doc.scroll_line = 3.0;
    assert!(app.fold_motion(before, at(0)).is_none());
    let doc = app.editor.active_mut().unwrap();
    doc.run_fold(winc_editor::FoldCommand::UnfoldAll, tab_width);
    doc.scroll_line = 0.0;
    let before = fold_state(&app);
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(0);
    doc.run_fold(winc_editor::FoldCommand::FoldLevel(2), tab_width);
    assert_eq!(folded(&app), [(3, 63), (65, 66)]);
    assert!(app.fold_motion(before, at(0)).is_none());

    // 动画进行中点击时直接到达终点，点击位置按折叠后的布局判定。
    let doc = app.editor.active_mut().unwrap();
    doc.run_fold(winc_editor::FoldCommand::UnfoldAll, tab_width);
    let before = fold_state(&app);
    app.editor
        .active_mut()
        .unwrap()
        .toggle_fold_at(3, tab_width);
    app.fold_motion = app.fold_motion(before, at(80));
    assert!(app.fold_motion.is_some());
    let at = (app.layout.text.x + 40.0, app.layout.text.y + 5.0);
    click(&mut app, at);
    assert!(app.fold_motion.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_arrow_of_a_wrapped_line_pinned_to_the_top_can_be_clicked() {
    let dir = navigation_folder("folding-wrapped");
    let path = dir.join("wrapped.py");
    let params: Vec<String> = (0..40).map(|i| format!("argument_{i}")).collect();
    let mut text = format!("def run({}):\n", params.join(", "));
    for i in 0..30 {
        text.push_str(&format!("    x{i} = {i}\n"));
    }
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    // 「粘滞滚动」会把 `def` 所在行固定在顶部并盖住箭头，因此这里关闭该设置。
    app.config.sticky_scroll = Some(false);
    app.run_action(crate::keymap::Action::ToggleWordWrap);
    paint(&mut app);
    assert!(app.editor.active().unwrap().rows_of(0) >= 3);

    // 滚动到 `def` 行只有后几个显示行可见的位置：该行的箭头仍绘制在可见的最上面一个显示行，
    // 点击该箭头即折叠该块。
    app.editor.active_mut().unwrap().scroll_line = 1.0;
    let column = fold_column(app.layout.gutter);
    let at = (
        column.x + column.w / 2.0,
        app.layout.text.y + app.fonts.line_height / 2.0,
    );
    app.on_mouse_move(at.0, at.1);
    paint(&mut app);
    assert_eq!(app.hover, Hover::Gutter);
    click(&mut app, at);
    assert_eq!(folded(&app), [(0, 30)]);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 视图滚离光标、光标不在屏幕上时，点击行号栏的箭头折叠其下的块：最上面一行保持不动，
/// 视图不跳回光标位置，折叠仍有动画。
///
/// 光标在屏幕上时仍保持在可见范围内。
#[test]
fn folding_below_the_top_keeps_the_view_when_the_caret_is_off_screen() {
    use std::time::Duration;

    let dir = navigation_folder("fold-off-screen");
    let mut text = String::new();
    for f in 0..40 {
        text.push_str(&format!("def f{f}():\n"));
        for i in 0..8 {
            text.push_str(&format!("    x{i} = {i}\n"));
        }
        text.push('\n');
    }
    let path = dir.join("many.py");
    std::fs::write(&path, &text).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&path);
    paint(&mut app);
    // 光标在第 0 行，视图滚到第 50 行：光标不在屏幕上。
    app.editor.active_mut().unwrap().scroll_line = 50.0;
    paint(&mut app);
    let before = fold_state(&app);
    let arrow = gutter_point(&app, 60, app.layout.gutter.w - 4.0);
    click(&mut app, arrow);
    assert_eq!(folded(&app), [(60, 68)]);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.scroll_line, 50.0);
    assert_eq!(caret_line(&app), 0);
    let at = Clock::frozen(Duration::ZERO, FOLD_ANIMATION);
    let motion = app.fold_motion(before, at).expect("折叠照常做动画");
    assert_eq!((motion.rows, motion.expanding), (61..69, false));

    std::fs::remove_dir_all(&dir).unwrap();
}
