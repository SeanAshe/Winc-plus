//! 多光标与列选择的测试：键位表中的快捷键、`⌥` 单击、`⇧⌥` 拖动与中键拖动、多光标的复制粘贴，
//! 以及多个光标、列选择的截图。

use super::*;
use crate::app::snapshot::editor::navigation::press;
use crate::keymap::{Action, KeyCode, Os};
use winit::keyboard::ModifiersState;

/// 测试样本：6 行长短不一的 Rust 代码，用于区分列位置与行尾。
const CODE: &str = "fn main() {\n    let alpha = 1;\n    let beta = 2;\n    let gamma = 3;\n    println!(\"{alpha} {beta} {gamma}\");\n}\n";

/// 打开内容为 [`CODE`] 的 `src/main.rs` 并绘制一帧，返回界面与目录。
fn cursors_app(name: &str, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    std::fs::write(dir.join("src/main.rs"), CODE).unwrap();
    let mut app = app_with(&dir.join("src/main.rs"), dark);
    app.focus = Focus::Editor;
    paint(&mut app);
    (app, dir)
}

/// 返回第 `line` 行第 `col` 列（均从 0 起）的窗口坐标：该格的左边缘、行的垂直中心。
fn point(app: &App, line: usize, col: usize) -> (f32, f32) {
    let t = app.layout.text;
    let doc = app.editor.active().unwrap();
    (
        t.x + TEXT_PAD - doc.scroll_x + col as f32 * app.cell,
        t.y + ((line as f64 - doc.scroll_line) as f32 + 0.5) * app.fonts.line_height,
    )
}

/// 单击第 `line` 行第 `col` 列；修饰键取 `app.modifiers`。
fn click_at(app: &mut App, line: usize, col: usize) {
    let at = point(app, line, col);
    click(app, at);
}

fn text(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    doc.text_range(0, usize::MAX).unwrap()
}

/// 返回各选区（按位置排序）的（行，列）两端：（起点，光标端）。
fn carets(app: &App) -> Vec<((usize, usize), (usize, usize))> {
    let doc = app.editor.active().unwrap();
    let at = |pos| doc.text().pos_to_line_col(pos);
    doc.selections()
        .0
        .iter()
        .map(|s| (at(s.anchor), at(s.head)))
        .collect()
}

/// 当前系统上「在上下行添加光标」所用的修饰键，取自键位表。
fn add_cursor_mods() -> &'static [&'static str] {
    match Os::CURRENT {
        Os::Mac => &["cmd", "alt"],
        Os::Windows => &["ctrl", "alt"],
        Os::Linux => &["shift", "alt"],
    }
}

/// 位置 `pos`（行，列）处的光标，起点与终点相同。
fn caret(pos: (usize, usize)) -> ((usize, usize), (usize, usize)) {
    (pos, pos)
}

#[test]
fn cursors_are_added_with_the_keymap_and_removed_with_escape() {
    let (mut app, dir) = cursors_app("cursor-keys", false);
    let line = app.editor.active().unwrap().text().line_start(1);
    app.select(line + 8, line + 8);

    // ⌥⌘↓ 两次：第 2、3 行同一列各添加一个光标，键入的字出现在三处，一次撤销全部还原。
    assert!(press(&mut app, add_cursor_mods(), KeyCode::Down));
    assert!(press(&mut app, add_cursor_mods(), KeyCode::Down));
    assert_eq!(carets(&app), [caret((1, 8)), caret((2, 8)), caret((3, 8))]);
    app.run(Command::Insert("_".into()));
    assert!(text(&app).contains("let _alpha = 1;\n    let _beta = 2;\n    let _gamma = 3;"));
    // ⇧⌘→ 使各光标分别选到行尾，之后同时替换。
    app.run(Command::LineEnd { extend: true });
    app.run(Command::Insert("x;".into()));
    assert!(text(&app).contains("let _x;\n    let _x;\n    let _x;"));
    press(&mut app, &["cmd"], KeyCode::Char('z'));
    press(&mut app, &["cmd"], KeyCode::Char('z'));
    assert_eq!(text(&app), CODE);
    assert_eq!(carets(&app).len(), 3);

    // Esc 只保留主光标（最后添加的光标）；此后再按 Esc 不再触发该快捷键，由编辑区处理。
    assert!(press(&mut app, &[], KeyCode::Escape));
    assert_eq!(carets(&app), [caret((3, 8))]);
    assert!(!press(&mut app, &[], KeyCode::Escape));

    // ⌘D：先选中光标处的词，再逐个加入下一处（到达末尾后从开头查找）；全部选中后 ⌘K ⌘D
    // 没有可移往的下一处，选区不变。
    let at = app.editor.active().unwrap().text().line_start(4) + 16;
    app.select(at, at);
    assert!(press(&mut app, &["cmd"], KeyCode::Char('d')));
    assert_eq!(carets(&app), [((4, 15), (4, 20))]);
    press(&mut app, &["cmd"], KeyCode::Char('d'));
    assert_eq!(carets(&app), [((1, 8), (1, 13)), ((4, 15), (4, 20))]);
    press(&mut app, &["cmd"], KeyCode::Char('k'));
    assert!(press(&mut app, &["cmd"], KeyCode::Char('d')));
    assert_eq!(carets(&app), [((1, 8), (1, 13)), ((4, 15), (4, 20))]);
    assert_eq!(app.status.message.as_deref(), Some("没有其他匹配项"));

    // ⇧⌘L：选中全部匹配，同时改名。
    let at = app.editor.active().unwrap().text().line_start(2) + 9;
    app.select(at, at);
    assert!(press(&mut app, &["cmd", "shift"], KeyCode::Char('l')));
    assert_eq!(carets(&app), [((2, 8), (2, 12)), ((4, 23), (4, 27))]);
    app.run(Command::Insert("b".into()));
    assert!(
        text(&app)
            .contains("let b = 2;\n    let gamma = 3;\n    println!(\"{alpha} {b} {gamma}\");")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn alt_click_adds_cursors_and_shift_alt_drag_selects_a_column() {
    let (mut app, dir) = cursors_app("cursor-mouse", false);

    // 普通单击放置一个光标，⌥ 单击再添加一个；⌥ 单击已有的光标则移除该光标。
    click_at(&mut app, 1, 4);
    app.modifiers = ModifiersState::ALT;
    click_at(&mut app, 3, 4);
    assert_eq!(carets(&app), [caret((1, 4)), caret((3, 4))]);
    click_at(&mut app, 1, 4);
    assert_eq!(carets(&app), [caret((3, 4))]);
    click_at(&mut app, 2, 6);
    assert_eq!(carets(&app).len(), 2);

    // 普通单击回到单光标。
    app.modifiers = ModifiersState::empty();
    click_at(&mut app, 0, 0);
    assert_eq!(carets(&app), [caret((0, 0))]);

    // ⇧⌥ 拖动框选一列，指针最终所在的行为主选区。
    app.modifiers = ModifiersState::SHIFT | ModifiersState::ALT;
    let (x, y) = point(&app, 1, 12);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (x, y) = point(&app, 3, 8);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Left);
    app.modifiers = ModifiersState::empty();
    assert_eq!(
        carets(&app),
        [((1, 12), (1, 8)), ((2, 12), (2, 8)), ((3, 12), (3, 8))]
    );
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (3, 8));
    app.run(Command::Insert("v".into()));
    assert!(text(&app).contains("let va = 1;\n    let v = 2;\n    let va = 3;"));

    // 短于框左边界的行（只有「}」的一行）不生成选区；指针最终所在的行即使为空也生成光标。
    app.modifiers = ModifiersState::SHIFT | ModifiersState::ALT;
    let (x, y) = point(&app, 4, 30);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (x, y) = point(&app, 6, 30);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Left);
    app.modifiers = ModifiersState::empty();
    assert_eq!(carets(&app), [caret((4, 30)), caret((6, 0))]);

    // 中键拖动同样框选一列；框超出行尾时，选区止于行尾。
    let (x, y) = point(&app, 1, 4);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Middle);
    let (x, y) = point(&app, 2, 40);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Middle);
    assert_eq!(carets(&app), [((1, 4), (1, 15)), ((2, 4), (2, 14))]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn copy_and_paste_with_several_cursors() {
    let (mut app, dir) = cursors_app("cursor-clipboard", false);
    let doc = app.editor.active_mut().unwrap();
    let start = |line| doc.text().line_start(line);
    let all = vec![
        Selection::new(start(1) + 8, start(1) + 13),
        Selection::new(start(2) + 8, start(2) + 12),
        Selection::new(start(3) + 8, start(3) + 13),
    ];
    doc.set_selections(all, 2);

    // 复制：各选区的文字以换行符连接。粘贴到三个光标处时，每个光标各得一行。
    app.copy(false);
    assert_eq!(app.copied.as_deref(), Some("alpha\nbeta\ngamma"));
    app.run(Command::LineEnd { extend: false });
    app.paste();
    assert!(
        text(&app).contains("let alpha = 1;alpha\n    let beta = 2;beta\n    let gamma = 3;gamma")
    );
    app.run(Command::Undo);

    // 剪切只删除选中的文字；光标处没有选区时复制整行。
    app.run(Command::LineStart { extend: true });
    app.copy(true);
    assert_eq!(
        app.copied.as_deref(),
        Some("let alpha = 1;\nlet beta = 2;\nlet gamma = 3;")
    );
    assert!(text(&app).contains("{\n    \n    \n    \n    println!"));
    app.run(Command::Undo);
    app.run(Command::LineEnd { extend: false });
    app.copy(false);
    assert_eq!(
        app.copied.as_deref(),
        Some("    let alpha = 1;\n    let beta = 2;\n    let gamma = 3;\n")
    );
    // 整行粘贴：剪贴板的行数与光标数相同，每个光标所在行的上方各插入一行。
    app.paste();
    assert!(
        text(&app).contains(
            "    let alpha = 1;\n    let alpha = 1;\n    let beta = 2;\n    let beta = 2;"
        )
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn line_ends_cursor_undo_and_keyboard_columns() {
    let (mut app, dir) = cursors_app("cursor-more", false);
    let start = |app: &App, line| app.editor.active().unwrap().text().line_start(line);

    // ⇧⌥I：选中第 1 到第 3 行，每一行行尾各一个光标，同时插入注释。
    let (a, b) = (start(&app, 1), start(&app, 3) + 18);
    app.select(a, b);
    assert!(press(&mut app, &["shift", "alt"], KeyCode::Char('i')));
    assert_eq!(
        carets(&app),
        [caret((1, 18)), caret((2, 17)), caret((3, 18))]
    );

    // ⌘U 恢复添加光标之前的选区，⇧⌘U 重做该操作。
    assert!(press(&mut app, &["cmd"], KeyCode::Char('u')));
    assert_eq!(carets(&app), [((1, 0), (3, 18))]);
    assert!(press(&mut app, &["cmd", "shift"], KeyCode::Char('u')));
    assert_eq!(carets(&app).len(), 3);
    app.run(Command::Insert(" //".into()));
    assert!(text(&app).contains("let alpha = 1; //\n    let beta = 2; //\n    let gamma = 3; //"));
    app.run(Command::Undo);

    // ⇧⌥⌘↓ 两次、⇧⌥⌘→ 三次：从光标处框出三行三列。Linux 上这些组合键用于复制行，
    // 该命令没有默认快捷键。
    if Os::CURRENT != Os::Linux {
        let column: &[&str] = if Os::CURRENT == Os::Mac {
            &["cmd", "shift", "alt"]
        } else {
            &["ctrl", "shift", "alt"]
        };
        let at = start(&app, 1) + 8;
        app.select(at, at);
        assert!(press(&mut app, column, KeyCode::Down));
        press(&mut app, column, KeyCode::Down);
        for _ in 0..3 {
            press(&mut app, column, KeyCode::Right);
        }
        assert_eq!(
            carets(&app),
            [((1, 8), (1, 11)), ((2, 8), (2, 11)), ((3, 8), (3, 11))]
        );
    }

    // ⌥ 拖动：在已有选区之外再添加一个选区。
    app.select(start(&app, 1) + 8, start(&app, 1) + 13);
    app.modifiers = ModifiersState::ALT;
    let (x, y) = point(&app, 3, 8);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (x, y) = point(&app, 3, 13);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Left);
    app.modifiers = ModifiersState::empty();
    assert_eq!(carets(&app), [((1, 8), (1, 13)), ((3, 8), (3, 13))]);

    // 查找条打开时 ⌥↩ 把全部查找结果变成选区，焦点回到编辑区。
    app.run_action(Action::Find);
    app.find.input.set_text("let");
    app.search.query = "let".into();
    app.search_changed();
    assert!(press(&mut app, &["alt"], KeyCode::Enter));
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(
        carets(&app),
        [((1, 4), (1, 7)), ((2, 4), (2, 7)), ((3, 4), (3, 7))]
    );
    app.close_search();
    assert!(!press(&mut app, &["alt"], KeyCode::Enter));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：三行各一个光标加两处选中的词，以及从第 1 行第 4 列到第 4 行第 12 列的列选择，
/// 浅色与深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_cursors() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let (mut app, dir) = cursors_app("cursors", dark);

        // 三行各一个光标、两处选中的词。
        let line = app.editor.active().unwrap().text().line_start(1);
        app.select(line + 8, line + 8);
        press(&mut app, add_cursor_mods(), KeyCode::Down);
        press(&mut app, add_cursor_mods(), KeyCode::Down);
        let at = app.editor.active().unwrap().text().line_start(4) + 16;
        app.editor.active_mut().unwrap().toggle_cursor(at);
        app.run(Command::MoveRight {
            word: true,
            extend: true,
        });
        app.blink.visible = true;
        shoot(&mut app, &format!("cursors-multiple-{theme}"));

        // 列选择：从第 1 行第 4 列框到第 4 行第 12 列。
        app.modifiers = ModifiersState::SHIFT | ModifiersState::ALT;
        let (x, y) = point(&app, 1, 4);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        let (x, y) = point(&app, 4, 12);
        app.on_mouse_move(x, y);
        app.on_mouse_up(MouseButton::Left);
        app.modifiers = ModifiersState::empty();
        app.blink.visible = true;
        shoot(&mut app, &format!("cursors-column-{theme}"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
