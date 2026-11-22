//! 各文件上次的位置：关闭后再打开、重新启动、没有打开目录时都回到原处。

use super::*;

/// 生成 `lines` 行 `line <行号>` 的样本。
fn numbered(lines: usize) -> String {
    (0..lines).map(|i| format!("line {i}\n")).collect()
}

/// 返回当前文档光标所在的行和列，以及视口顶边的显示行。
fn at(app: &App) -> ((usize, usize), f64) {
    let doc = app.editor.active().unwrap();
    let caret = doc.text().pos_to_line_col(doc.selection.head);
    (caret, doc.scroll_line)
}

/// 另一个实例沿用 `app` 的状态，不打开目录，只打开文件 `path`。
fn reopened(app: &App, path: &Path) -> App {
    let mut next = blank_app(false);
    next.state = app.state.clone();
    next.open_path(path);
    paint(&mut next);
    next
}

#[test]
fn a_text_file_reopens_where_it_was_left() {
    let dir = sample_folder("places-text");
    let path = dir.join("long.txt");
    std::fs::write(&path, numbered(300)).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&path);
    paint(&mut app);
    let file = app.editor.active().unwrap().path.clone().unwrap();

    app.jump_to(120, 3, 0);
    app.scroll.glide = None;
    paint(&mut app);
    let left = at(&app);
    assert_eq!(left.0, (120, 3));
    assert!(left.1 > 0.0);

    // 关闭标签时记下，再次打开回到原处；视口顶边按行记，不足一行的部分不记。
    app.close_tab(app.editor.active.unwrap());
    let saved = app.state.place_of(&file).and_then(|place| place.text);
    assert_eq!(saved, Some((120, 3, left.1 as usize)));
    app.open_path(&path);
    paint(&mut app);
    assert_eq!(at(&app).0, left.0);
    assert!((at(&app).1 - left.1).abs() < 1.0, "{:?}", at(&app));

    // 保存现场时记下打开着的各文件；重新启动后不打开目录、只打开这个文件，同样回到原处。
    app.jump_to(200, 0, 0);
    app.scroll.glide = None;
    paint(&mut app);
    app.remember_session();
    let next = reopened(&app, &path);
    assert!(next.editor.root().is_none());
    assert_eq!(at(&next).0, (200, 0));
    assert!(at(&next).1 > 0.0);

    // 打开时指定了位置的以指定的为准。
    let mut jump = blank_app(false);
    jump.state = app.state.clone();
    jump.open_path_then(&path, |app| app.go_to(7, 1, 0));
    paint(&mut jump);
    assert_eq!(at(&jump).0, (7, 1));

    // 文件在别处变短了：停在最后一行，不越界。
    std::fs::write(&path, numbered(10)).unwrap();
    let short = reopened(&app, &path);
    assert_eq!(at(&short).0, (10, 0));

    // 回到开头后去掉记录。
    let mut back = reopened(&app, &path);
    back.jump_to(0, 0, 0);
    back.scroll.glide = None;
    paint(&mut back);
    back.close_tab(back.editor.active.unwrap());
    assert_eq!(back.state.place_of(&file), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn places_follow_renames_and_skip_what_has_no_file() {
    let dir = sample_folder("places-rename");
    let path = dir.join("long.txt");
    std::fs::write(&path, numbered(100)).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&path);
    app.jump_to(40, 0, 0);
    // 未命名的文档没有文件，不记。
    app.new_file();
    app.remember_session();
    let file = app.editor.documents[0].path.clone().unwrap();
    assert_eq!(app.state.places.len(), 1);
    assert_eq!(app.state.places[0].0, file);

    let moved = file.with_file_name("renamed.txt");
    app.paths_moved(&file, &moved);
    assert_eq!(app.state.place_of(&file), None);
    assert!(
        app.state
            .place_of(&moved)
            .is_some_and(|place| place.text.is_some())
    );
    let _ = std::fs::remove_dir_all(&dir);
}
