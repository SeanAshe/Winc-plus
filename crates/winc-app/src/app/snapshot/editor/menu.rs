//! 编辑区右键菜单的测试：菜单项随选区、书签、文件是否只读变化，菜单命令作用于选区。

use super::*;

#[test]
fn right_clicking_the_text_offers_format_cut_copy_paste() {
    use MenuCommand::{
        AddBookmark, CompareSelectionWithClipboard, CompareWithClipboard, CompareWithSaved, Copy,
        CopyReference, Cut, FindReferences, Fix, Format, FormatSelection, GoToDefinition,
        ListBookmarks, Paste, RemoveBookmark,
    };

    let dir = search_folder("editor-menu");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    paint(&mut app);
    // 在第 `line` 行第 `col` 列（均从 0 起）处按右键。
    let right_click_text = |app: &mut App, line: usize, col: usize| {
        let t = app.layout.text;
        let x = t.x + TEXT_PAD + col as f32 * app.metrics.cell;
        right_click(app, (x, t.y + (line as f32 + 0.5) * app.fonts.line_height));
    };
    let selection = |app: &App| {
        let s = app.editor.active().unwrap().selection;
        (s.from(), s.to())
    };
    let line = |app: &App, n: usize| {
        let text = app.editor.active().unwrap().text();
        text.line_content(n).slice().to_string()
    };

    // 在选区内右键：保留选区，菜单命令作用于该选区。
    app.select(20, 26);
    right_click_text(&mut app, 1, 10);
    let menu = app.overlays.context_menu.as_ref().unwrap();
    assert!(matches!(menu.target, MenuTarget::Editor(_)));
    assert_eq!(
        menu_commands(&app),
        [
            (GoToDefinition, true),
            (FindReferences, true),
            (AddBookmark, true),
            (RemoveBookmark, false),
            (ListBookmarks, true),
            (Format, true),
            (FormatSelection, true),
            (Fix, true),
            (CompareSelectionWithClipboard, true),
            (CompareWithSaved, true),
            (Cut, true),
            (Copy, true),
            (Paste, true),
            (CopyReference, true)
        ]
    );
    assert_eq!(selection(&app), (20, 26));
    app.run_menu_command(Copy);
    assert_eq!(app.clipboard.copied.as_deref(), Some("config"));

    // 该行添加书签后，菜单项变为「删除书签」。
    right_click_text(&mut app, 1, 10);
    app.run_menu_command(AddBookmark);
    right_click_text(&mut app, 1, 10);
    let enabled = |app: &App, command| {
        menu_commands(app)
            .into_iter()
            .find(|(c, _)| *c == command)
            .map(|(_, on)| on)
    };
    assert_eq!(enabled(&app, AddBookmark), Some(false));
    assert_eq!(enabled(&app, RemoveBookmark), Some(true));
    app.run_menu_command(RemoveBookmark);
    right_click_text(&mut app, 1, 10);
    assert_eq!(enabled(&app, AddBookmark), Some(true));

    // 在选区外右键：光标移到点击处。没有选区时，剪切的是整行。
    right_click_text(&mut app, 0, 3);
    assert_eq!(selection(&app), (3, 3));
    app.run_menu_command(Cut);
    assert_eq!(app.clipboard.copied.as_deref(), Some("fn main() {\n"));
    assert_eq!(line(&app, 0), "    let config = load_config();");

    // 粘贴整行：插入到光标所在行的上方。
    right_click_text(&mut app, 0, 0);
    app.run_menu_command(Paste);
    assert_eq!(line(&app, 0), "fn main() {");
    assert_eq!(line(&app, 1), "    let config = load_config();");

    // 查找条覆盖的区域不弹出编辑区菜单，右键事件由查找条处理。
    app.open_search(false);
    paint(&mut app);
    let bar = app.layout.search;
    right_click(&mut app, (bar.x + bar.w / 2.0, bar.y + bar.h / 2.0));
    assert!(app.overlays.context_menu.is_none());

    // 只读文档：格式化、修复、剪切、粘贴不可用，转到定义、查找引用、比较和复制仍可用。
    app.editor.active_mut().unwrap().readonly = true;
    right_click_text(&mut app, 0, 0);
    assert_eq!(
        menu_commands(&app),
        [
            (GoToDefinition, true),
            (FindReferences, true),
            (AddBookmark, true),
            (RemoveBookmark, false),
            (ListBookmarks, true),
            (Format, false),
            (Fix, false),
            (CompareWithClipboard, true),
            (CompareWithSaved, true),
            (Cut, false),
            (Copy, true),
            (Paste, false),
            (CopyReference, true),
        ]
    );

    let _ = std::fs::remove_dir_all(&dir);
}
