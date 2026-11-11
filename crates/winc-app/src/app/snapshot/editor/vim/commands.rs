//! `Ctrl` 组合的归属、应用命令与未完成按键的配合、命令行的补全，以及保存与关闭的命令。

use winc_vim::{Key as VimKey, Mods};

use super::*;

/// 返回输入条中的文字。
fn field(app: &App) -> String {
    app.goto_input.as_ref().unwrap().text().to_owned()
}

#[test]
fn ctrl_keys_reach_vim_where_the_mode_defines_them() {
    let (mut app, dir) = vim_app("vim-ctrl", "one 7\ntwo\n");
    // 普通模式下有定义的 `Ctrl` 键由 Vim 层处理。
    assert!(app.vim_press(VimKey::ctrl('v')));
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    // 可视模式下的复制把选区放进系统剪贴板，并回到普通模式。
    keys(&mut app, "l\"a");
    app.vim_copy_selection();
    assert_eq!(
        (app.copied.as_deref(), app.vim_mode()),
        (Some("on"), Some(Mode::Normal))
    );
    assert!(app.vim_press(VimKey::ctrl('a')));
    assert_eq!(text(&app), "one 8\ntwo\n");

    // `Ctrl-C` 取消未完成的按键，没有未完成的按键时也不交给应用。
    keys(&mut app, "0d");
    assert!(app.vim_press(VimKey::ctrl('c')));
    keys(&mut app, "x");
    assert_eq!(text(&app), "ne 8\ntwo\n");
    assert!(app.vim_press(VimKey::ctrl('c')));
    keys(&mut app, "v");
    assert!(app.vim_press(VimKey::ctrl('c')));
    assert_eq!(app.vim_mode(), Some(Mode::Normal));

    // 当前模式下没有定义的组合回到应用：插入模式的粘贴、全选、查找，带 `Shift` 的组合。
    let shifted = VimKey::new(Code::Char('f'), Mods::CTRL.with(Mods::SHIFT));
    assert!(!app.vim_press(shifted));
    keys(&mut app, "i");
    for c in "vaf".chars() {
        assert!(!app.vim_press(VimKey::ctrl(c)), "{c}");
    }
    assert!(app.vim_press(VimKey::ctrl('w')));
    keys(&mut app, "<Esc>");

    // 「快捷键」页只标注普通或可视模式下有定义的组合。
    let takes = |app: &App, label: &str| app.vim_takes_keys(&[label.to_owned()]);
    assert!(takes(&app, "Ctrl+V") && takes(&app, "Ctrl+W") && takes(&app, "⌃C"));
    assert!(!takes(&app, "Ctrl+S") && !takes(&app, "Ctrl+T"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn application_commands_drop_the_unfinished_keys_first() {
    let (mut app, dir) = vim_app("vim-pending", "one\ntwo\n");
    // 等待动作时执行应用的复制：未完成的操作符取消，复制的是当前行。
    keys(&mut app, "d");
    assert!(app.run_action(Action::Copy));
    assert_eq!(
        (app.copied.as_deref(), text(&app).as_str()),
        (Some("one\n"), "one\ntwo\n")
    );
    keys(&mut app, "x");
    assert_eq!(text(&app), "ne\ntwo\n");
    // 寄存器与次数同样不带入应用的命令。
    keys(&mut app, "\"a3");
    assert!(app.run_action(Action::Undo));
    assert_eq!(text(&app), "one\ntwo\n");
    keys(&mut app, "2");
    assert!(app.run_action(Action::Paste));
    assert_eq!(text(&app), "one\none\ntwo\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn completion_restarts_when_the_text_changed_without_a_key() {
    let (mut app, dir) = vim_app("vim-complete", "one\n");
    std::fs::write(dir.join("other.txt"), "other\n").unwrap();
    let tab = Key::Named(NamedKey::Tab);
    keys(&mut app, ":");
    app.edit_field(|field| field.insert("e o"));
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(field(&app), "e other.txt");
    // 输入法上屏或鼠标粘贴换掉了文字：补全重新开始，起点不落在字符中间。
    app.goto_input.as_mut().unwrap().set_text("文");
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(field(&app), "文");
    app.goto_input.as_mut().unwrap().set_text("e 文o");
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(field(&app), "e 文o");
    app.goto_input.as_mut().unwrap().set_text("set sm");
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(field(&app), "set smartcase");
    app.close_prompt();
    app.focus = Focus::Editor;
    app.vim_prompt_cancel();

    // 候选按名字排序之后再截取。
    for index in 0..205 {
        std::fs::write(dir.join(format!("f{index:03}.txt")), "").unwrap();
    }
    let found = app.vim_paths("f");
    let wanted: Vec<String> = (0..200).map(|index| format!("f{index:03}.txt")).collect();
    assert_eq!(found, wanted);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_and_quit_saves_before_closing_without_asking() {
    let (mut app, dir) = vim_app("vim-wq", "one\ntwo\n");
    let path = dir.join("vim.txt");
    keys(&mut app, "x");
    ex(&mut app, "wq");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ne\ntwo\n");
    assert_eq!(app.editor.documents.len(), 0);
    assert!(!app.closing_pending());

    // 没有修改时 `:x` 只关闭。
    app.open_path(&path);
    app.focus = Focus::Editor;
    paint(&mut app);
    ex(&mut app, "x");
    assert_eq!(app.editor.documents.len(), 0);

    // 没有路径的文档：先选择保存的位置，选好之前不关闭，也不询问是否保存。
    app.new_file();
    app.focus = Focus::Editor;
    paint(&mut app);
    keys(&mut app, "inew<Esc>");
    ex(&mut app, "wq");
    assert_eq!(app.editor.documents.len(), 1);
    assert!(app.closing_pending() && !app.quit_pending());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn writing_to_a_path_leaves_the_document_where_it_is() {
    let (mut app, dir) = vim_app("vim-write-copy", "one\ntwo\n");
    let path = winc_editor::canonical_path(&dir.join("vim.txt"));
    let copy = dir.join("copy.txt");
    keys(&mut app, "x");
    ex(&mut app, "w copy.txt");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "ne\ntwo\n");
    assert_eq!(app.status.message.as_deref(), Some("已写出 copy.txt"));
    // 文档仍对应原来的路径，修改仍未保存。
    let doc = app.editor.active().unwrap();
    assert_eq!((doc.path.as_ref(), doc.is_dirty()), (Some(&path), true));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");

    // 目标上已有文件时要加 `!`。
    keys(&mut app, "x");
    ex(&mut app, "w copy.txt");
    assert_eq!(app.status.message.as_deref(), Some("文件已存在，加 ! 覆盖"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "ne\ntwo\n");
    ex(&mut app, "w! copy.txt");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "e\ntwo\n");
    // 写到文档自己的路径就是保存。
    ex(&mut app, "w! vim.txt");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "e\ntwo\n");
    assert!(!app.editor.active().unwrap().is_dirty());

    // 没有路径的文档以给出的路径保存。
    app.new_file();
    app.focus = Focus::Editor;
    paint(&mut app);
    keys(&mut app, "inew<Esc>");
    ex(&mut app, "w named.txt");
    let named = winc_editor::canonical_path(&dir.join("named.txt"));
    let doc = app.editor.active().unwrap();
    assert_eq!((doc.path.as_ref(), doc.is_dirty()), (Some(&named), false));
    assert_eq!(std::fs::read_to_string(&named).unwrap(), "new");
    let _ = std::fs::remove_dir_all(&dir);
}
