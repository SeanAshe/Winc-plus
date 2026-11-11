//! 差异视图中只读的一侧：移动、可视选择、复制、查找与标记，修改类命令提示只读，以及差异中
//! 光标的形状。

use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT, ReadSelection};

use super::*;

/// 打开两侧都不对应文件的差异，左侧为 `old`，右侧为 `new`；返回界面与样本目录。
fn history_diff(name: &str, old: &str, new: &str) -> (App, PathBuf) {
    let (mut app, dir) = vim_app(name, "source\n");
    let language = winc_syntax::detect("vim.txt");
    let view = DiffView::new(
        DiffSide::new("old", old, None, language),
        DiffSide::new("new", new, None, language),
    );
    app.editor.show_diff_in(None, "vim.txt (history)", view);
    app.focus = Focus::Editor;
    paint(&mut app);
    (app, dir)
}

/// 返回当前差异。
fn view(app: &mut App) -> &mut DiffView {
    let doc = app.editor.active_mut().unwrap();
    doc.diff.as_deref_mut().unwrap()
}

/// 返回只读一侧的选区：所在侧、起点与终点。
fn read(app: &mut App) -> Option<(usize, usize, usize)> {
    let selection = view(app).read_selection?;
    Some((selection.side, selection.range.0, selection.range.1))
}

/// 返回差异中 Vim 模式的光标。
fn caret(app: &App) -> Option<(usize, usize, CaretShape)> {
    let doc = app.editor.active().unwrap();
    app.vim_diff_caret(doc.diff.as_deref().unwrap())
}

/// 像鼠标那样把只读一侧的选区设为第 `side` 侧的 `range`。
fn point(app: &mut App, side: usize, range: (usize, usize)) {
    view(app).read_selection = Some(ReadSelection { side, range });
    paint(app);
}

const OLD: &str = "one two\nthree\nfour two\n";
const NEW: &str = "one two\nTHREE\nfour two\nfive\n";

#[test]
fn a_read_only_side_takes_motions_selections_yanks_searches_and_marks() {
    let (mut app, dir) = history_diff("vim-diff-read", OLD, NEW);
    // 没有光标时不适用；放上光标之后位于右侧窗口顶部一行的行首。
    assert_eq!(app.vim_mode(), None);
    assert!(app.place_read_caret());
    assert!(!app.place_read_caret());
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    assert_eq!(read(&mut app), Some((RIGHT, 0, 0)));

    // 移动。
    keys(&mut app, "wj");
    assert_eq!(read(&mut app), Some((RIGHT, 12, 12)));
    keys(&mut app, "G");
    assert_eq!(read(&mut app), Some((RIGHT, 23, 23)));
    // 可视选择显示为该侧的选区；复制之后回到普通模式。
    keys(&mut app, "ggve");
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    assert_eq!(read(&mut app), Some((RIGHT, 0, 3)));
    keys(&mut app, "\"+y");
    assert_eq!(app.copied.as_deref(), Some("one"));
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    assert_eq!(read(&mut app), Some((RIGHT, 0, 0)));
    keys(&mut app, "j\"+yy");
    assert_eq!(app.copied.as_deref(), Some("THREE\n"));

    // 查找：输入条、`n` 与 `*`。
    keys(&mut app, "gg/");
    assert_eq!(app.focus, Focus::Prompt);
    app.edit_field(|field| field.insert("two"));
    app.submit_prompt();
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(read(&mut app), Some((RIGHT, 4, 4)));
    keys(&mut app, "n");
    assert_eq!(read(&mut app), Some((RIGHT, 19, 19)));
    keys(&mut app, "*");
    assert_eq!(read(&mut app), Some((RIGHT, 4, 4)));
    // 标记。
    keys(&mut app, "maG`a");
    assert_eq!(read(&mut app), Some((RIGHT, 4, 4)));
    // 命令行的行号。
    ex(&mut app, "3");
    assert_eq!(read(&mut app), Some((RIGHT, 14, 14)));

    // 修改类命令提示只读，文本不变，也不进入插入模式。
    for change in ["x", "dd", "p", "ix", "cw", "J", "~", "r1", ">>"] {
        app.status.message = None;
        keys(&mut app, change);
        let message = app.status.message.as_deref();
        assert_eq!(message, Some("差异的这一侧是只读的"), "{change}");
        assert_eq!(app.vim_mode(), Some(Mode::Normal), "{change}");
    }
    ex(&mut app, "s/four/4/");
    assert_eq!(app.status.message.as_deref(), Some("差异的这一侧是只读的"));
    assert_eq!(view(&mut app).new.text.to_string(), NEW);
    assert_eq!(view(&mut app).old.text.to_string(), OLD);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_left_side_keeps_its_own_cursor_and_marks_and_follows_the_mouse() {
    let (mut app, dir) = history_diff("vim-diff-left", OLD, NEW);
    assert!(app.place_read_caret());
    keys(&mut app, "jmb");
    // 鼠标把光标放到左侧：之后的按键作用于左侧。
    point(&mut app, LEFT, (8, 8));
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    keys(&mut app, "e");
    assert_eq!(read(&mut app), Some((LEFT, 12, 12)));
    keys(&mut app, "\"+yiw");
    assert_eq!(app.copied.as_deref(), Some("three"));
    keys(&mut app, "`b");
    assert_eq!(app.status.message.as_deref(), Some("标记 b 没有设置"));
    keys(&mut app, "x");
    assert_eq!(app.status.message.as_deref(), Some("差异的这一侧是只读的"));
    // 回到右侧时标记仍在。
    point(&mut app, RIGHT, (0, 0));
    keys(&mut app, "`b");
    assert_eq!(read(&mut app), Some((RIGHT, 8, 8)));

    // 鼠标拖出的选区进入可视模式，点击行尾之后光标退到最后一个字符。
    point(&mut app, LEFT, (0, 3));
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    // 应用的复制由 Vim 层执行，随后回到普通模式。
    assert!(app.run_action(Action::Copy));
    assert_eq!(app.copied.as_deref(), Some("one"));
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    point(&mut app, LEFT, (7, 7));
    keys(&mut app, "<Esc>");
    paint(&mut app);
    assert_eq!(read(&mut app), Some((LEFT, 6, 6)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_caret_in_a_diff_follows_the_mode() {
    let (mut app, dir) = history_diff("vim-diff-caret", OLD, NEW);
    assert_eq!(caret(&app), None);
    point(&mut app, LEFT, (0, 0));
    assert_eq!(caret(&app), Some((LEFT, 0, CaretShape::Block)));
    // 块覆盖一个字符格，横线高 2 逻辑像素、宽一个字符格。
    let scale = (SCALE * SCALE) as usize;
    let cell = app.cell.round() as usize;
    let line = app.fonts.line_height as usize;
    let block = caret_pixels(&mut app);
    assert!(block > cell * (line - 4) * scale, "{block}");
    keys(&mut app, "d");
    assert_eq!(caret(&app), Some((LEFT, 0, CaretShape::Underline)));
    assert_eq!(caret_pixels(&mut app), cell * 2 * scale);
    keys(&mut app, "<Esc>");
    // 可视模式的光标在选区的光标一端所含的字符上。
    keys(&mut app, "ve");
    assert_eq!(caret(&app), Some((LEFT, 2, CaretShape::Block)));
    keys(&mut app, "<Esc>");
    // 编辑区没有焦点时是空心的块，不闪烁。
    app.focus = Focus::Tree;
    assert_eq!(caret(&app), Some((LEFT, 2, CaretShape::Hollow)));
    assert_eq!(caret_pixels(&mut app), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_editable_side_shows_the_modal_caret_and_saves_its_document() {
    let (mut app, dir) = vim_app("vim-diff-edit", "one\ntwo\nthree\n");
    let path = app.editor.active().unwrap().path.clone().unwrap();
    let language = winc_syntax::detect("vim.txt");
    let mut diff = DiffView::new(
        DiffSide::new("old", "one\nthree\n", None, language),
        DiffSide::new(
            "worktree",
            "one\ntwo\nthree\n",
            Some(path.clone()),
            language,
        ),
    );
    diff.editing = true;
    app.editor.show_diff_in(None, "vim.txt (worktree)", diff);
    app.focus = Focus::Editor;
    paint(&mut app);
    keys(&mut app, "ggj");
    assert_eq!(caret(&app), Some((RIGHT, 4, CaretShape::Block)));
    // 插入模式用竖线光标。
    keys(&mut app, "cw");
    assert!(app.diff_edit_text("TWO"));
    assert_eq!(caret(&app), None);
    keys(&mut app, "<Esc>");
    assert_eq!(caret(&app), Some((RIGHT, 6, CaretShape::Block)));
    // `:w` 保存的是可编辑一侧对应的文档。
    ex(&mut app, "w");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\nTWO\nthree\n");

    // 光标移到只读的左侧：修改被拒绝，提示来自该侧。
    view(&mut app).editing = false;
    point(&mut app, LEFT, (0, 0));
    keys(&mut app, "x");
    assert_eq!(app.status.message.as_deref(), Some("差异的这一侧是只读的"));
    assert_eq!(caret(&app), Some((LEFT, 0, CaretShape::Block)));

    // `:wq` 保存可编辑一侧对应的文档，关闭的是差异的标签。
    view(&mut app).read_selection = None;
    view(&mut app).editing = true;
    paint(&mut app);
    keys(&mut app, "ggx");
    ex(&mut app, "wq");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ne\nTWO\nthree\n");
    let open = &app.editor.documents;
    assert_eq!((open.len(), open[0].diff.is_none()), (1, true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_diff_scrolls_with_the_read_only_cursor() {
    let old: String = (0..300).map(|line| format!("line {line}\n")).collect();
    let new = old.replace("line 150\n", "LINE 150\n");
    let (mut app, dir) = history_diff("vim-diff-scroll", &old, &new);
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;
    let line = |app: &mut App| {
        let (side, _, head) = read(app).unwrap();
        view(app).side(side).text.char_to_line(head)
    };
    // 窗口顶部是折叠的未改动内容：光标放在其后的第一行上。
    assert!(app.place_read_caret());
    assert!((140..150).contains(&line(&mut app)));
    // 光标移进折叠的内容时该段展开；移出窗口时差异随之滚动。
    keys(&mut app, "gg");
    assert_eq!((line(&mut app), scroll(&app)), (0, 0.0));
    keys(&mut app, "100j");
    assert_eq!(line(&mut app), 100);
    assert!(scroll(&app) > 0.0);
    keys(&mut app, "gg");
    assert_eq!((line(&mut app), scroll(&app)), (0, 0.0));
    // 滚动命令滚动差异，光标留在窗口之内。
    keys(&mut app, "<C-e><C-e>");
    assert_eq!(scroll(&app), 2.0);
    assert!(line(&mut app) >= 2);
    keys(&mut app, "<C-d>");
    assert!(scroll(&app) > 2.0);
    // 放上的光标在窗口顶部的一行。
    view(&mut app).read_selection = None;
    paint(&mut app);
    let top = scroll(&app);
    assert!(app.place_read_caret());
    assert!((line(&mut app) as f64 - top).abs() <= 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}
