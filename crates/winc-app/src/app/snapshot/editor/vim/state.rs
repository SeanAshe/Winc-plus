//! 写入状态文件的内容：大写标记移动后的位置、命令行历史与查找历史。

use super::*;

/// 在查找的输入条中输入 `pattern` 并回车。
fn search(app: &mut App, pattern: &str) {
    keys(app, "/");
    app.edit_field(|field| field.insert(pattern));
    app.submit_prompt();
}

#[test]
fn histories_are_read_at_start_and_written_on_exit() {
    let dir = sample_folder("vim-history");
    let path = dir.join("vim.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let mut app = app_with(&path, false);
    app.state.vim_commands = vec!["1".into(), "3".into()];
    app.state.vim_searches = vec!["one".into()];
    // 关闭 Vim 模式时不读也不写。
    app.vim_persist();
    assert_eq!(app.state.vim_commands, ["1", "3"]);

    app.config.vim_enabled = Some(true);
    app.vim_apply_config();
    paint(&mut app);
    // 开启之后读入：上下键翻出状态文件中的条目。
    let up = Key::Named(NamedKey::ArrowUp);
    keys(&mut app, ":");
    assert!(app.vim_prompt_key(&up, ""));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "3");
    app.submit_prompt();
    assert_eq!(line(&app), 2);
    keys(&mut app, "/");
    assert!(app.vim_prompt_key(&up, ""));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "one");
    app.submit_prompt();
    assert_eq!(line(&app), 0);

    // 新的条目在退出前写入，重复的条目移到最后。
    ex(&mut app, "2");
    search(&mut app, "three");
    assert_eq!(app.state.vim_commands, ["1", "3"]);
    app.prepare_exit();
    assert_eq!(app.state.vim_commands, ["1", "3", "2"]);
    assert_eq!(app.state.vim_searches, ["one", "three"]);

    // 关闭 Vim 模式之前写入一次。
    ex(&mut app, "1");
    app.config.vim_enabled = Some(false);
    app.vim_apply_config();
    assert_eq!(app.state.vim_commands, ["3", "2", "1"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn moved_marks_are_stored_when_the_document_or_the_window_closes() {
    let (mut app, dir) = vim_app("vim-mark-close", "one\ntwo\nthree\n");
    let path = app.editor.active().unwrap().path.clone().unwrap();
    keys(&mut app, "jmAgg");
    assert_eq!(app.state.vim_marks, [('A', path.clone(), 1, 0)]);
    // 标记随编辑移动；关闭文档时把新的位置写入状态。
    keys(&mut app, "Ozero<Esc>");
    ex(&mut app, "w");
    assert_eq!(app.state.vim_marks, [('A', path.clone(), 1, 0)]);
    ex(&mut app, "q");
    assert_eq!(app.editor.documents.len(), 0);
    assert_eq!(app.state.vim_marks, [('A', path.clone(), 2, 0)]);

    // 退出时同样写入，不需要先离开文档。
    app.open_path(&path);
    app.focus = Focus::Editor;
    paint(&mut app);
    keys(&mut app, "ggOtop<Esc>");
    assert_eq!(app.state.vim_marks, [('A', path.clone(), 2, 0)]);
    app.prepare_exit();
    assert_eq!(app.state.vim_marks, [('A', path, 3, 0)]);
    let _ = std::fs::remove_dir_all(&dir);
}
