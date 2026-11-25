//! 分栏时表格与左侧文字的联动：同步滚动、光标与选中的单元格互相跟随、键盘在两侧之间交接。

use winc_editor::PreviewMode;
use winit::keyboard::{Key, NamedKey};

use super::*;

/// 打开分栏的 csv：表头之后每行一条记录，第 3 条的备注跨三行（引号内换行）。
fn split_csv(name: &str) -> (App, PathBuf) {
    let dir = search_folder(name);
    let path = dir.join("data.csv");
    let mut text = String::from("id,name,note\n");
    for i in 0..200 {
        let note = if i == 2 {
            "\"one\ntwo\nthree\"".to_owned()
        } else {
            format!("n{i}")
        };
        text.push_str(&format!("{i},name{i},{note}\n"));
    }
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    app.set_preview_mode(PreviewMode::Split);
    app.wait_for_tables();
    frame(&mut app);
    frame(&mut app);
    (app, dir)
}

/// 按窗口的一帧推进并绘制：先做每帧绘制前的准备（其中包括与左侧同步），再绘制。
fn frame(app: &mut App) {
    app.prepare_frame();
    paint(app);
}

/// 左侧光标的（行，列），行列从 0 起。
fn caret(app: &App) -> (usize, usize) {
    let doc = app.editor.active().unwrap();
    doc.text().pos_to_line_col(doc.selection.head)
}

/// 点击表格：选中单元格，键盘作用于表格，左侧光标移到该格的开头；方向键移动时光标跟着走。
/// 输入文字时键盘交回左侧，文字写在光标处。
#[test]
fn cells_picked_in_the_split_table_move_the_caret() {
    let (mut app, dir) = split_csv("table-split-pick");
    let at = cell_center(&app, 1, 1);
    click(&mut app, at);
    assert_eq!(selection(&app), ((1, 1), (1, 1)));
    assert!(app.table_focused());
    assert_eq!(caret(&app), (1, 2), "第 1 条记录的 name 一格");

    table_key(&mut app, NamedKey::ArrowRight, false);
    assert_eq!(selection(&app).1, (1, 2));
    assert_eq!(caret(&app), (1, 8));
    // 跨行的那条记录：光标在该记录在文字中的第一行。
    table_key(&mut app, NamedKey::ArrowDown, false);
    table_key(&mut app, NamedKey::ArrowDown, false);
    assert_eq!(caret(&app), (3, 8));
    table_key(&mut app, NamedKey::ArrowDown, false);
    assert_eq!(caret(&app), (6, 8), "下一条记录在跨行的备注之后");

    assert!(!app.table_key(&Key::Character("x".into()), false));
    assert!(!app.table_focused(), "输入文字时键盘交回左侧");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 左侧的光标移动时表格选中光标所在的格；跨行的记录中任何一行都对应这条记录。
#[test]
fn the_split_table_selects_the_cell_under_the_caret() {
    let (mut app, dir) = split_csv("table-split-caret");
    let pos = |app: &App, line: usize, col: usize| {
        app.editor
            .active()
            .unwrap()
            .text()
            .line_col_to_pos(line, col)
    };
    let at = pos(&app, 4, 1);
    app.select(at, at);
    frame(&mut app);
    assert_eq!(
        selection(&app),
        ((3, 2), (3, 2)),
        "第 3 条记录的备注跨第 3 至 5 行"
    );

    let at = pos(&app, 150, 5);
    app.select(at, at);
    frame(&mut app);
    let row = selection(&app).1;
    assert_eq!(row, (148, 1));
    // 表格滚到该格可见。
    let layout = &app.preview.table_layout;
    let y = row.0 as f64 * f64::from(layout.row_h);
    let scroll = app.table_scroll().1;
    assert!(y >= scroll && y + f64::from(layout.row_h) <= scroll + f64::from(layout.body.h));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 左侧滚动时表格滚到最上面一行文字对应的记录；表格持有键盘时不跟随。点回左侧后键盘交回。
#[test]
fn the_split_table_scrolls_with_the_text() {
    let (mut app, dir) = split_csv("table-split-scroll");
    let row_h = f64::from(app.preview.table_layout.row_h);
    app.editor.active_mut().unwrap().scroll_line = 60.0;
    frame(&mut app);
    // 第 60 行文字是第 57 条记录（备注多占了两行），比表头多一行。
    let expected = 58.0 * row_h;
    assert!(
        (app.table_scroll().1 - expected).abs() < 1.0,
        "{}",
        app.table_scroll().1
    );

    // `cell_center` 不计滚动：第 2 行的位置此时是可见的某一行。
    let at = cell_center(&app, 2, 0);
    click(&mut app, at);
    assert!(app.table_focused());
    let scroll = app.table_scroll().1;
    app.editor.active_mut().unwrap().scroll_line = 10.0;
    frame(&mut app);
    assert_eq!(app.table_scroll().1, scroll, "表格持有键盘时不跟随左侧");

    let text = app.layout.text;
    click(&mut app, (text.x + 40.0, text.y + 10.0));
    assert!(!app.table_focused());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 截图：分栏时左侧光标在第 4 行的备注上，表格选中对应的格；随后点击表格中的一格。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_split_table_follows_the_caret() {
    let (mut app, dir) = split_csv("table-split-shot");
    let at = app.editor.active().unwrap().text().line_col_to_pos(4, 1);
    app.select(at, at);
    frame(&mut app);
    shoot(&mut app, "table-split-caret");
    let at = cell_center(&app, 6, 1);
    click(&mut app, at);
    frame(&mut app);
    shoot(&mut app, "table-split-picked");
    std::fs::remove_dir_all(&dir).unwrap();
}
