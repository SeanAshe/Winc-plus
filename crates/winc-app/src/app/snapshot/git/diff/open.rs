//! 差异标题栏上的「打开文件」：打开差异对应的工作区文件，停在差异中正在看的那一行。

use super::*;
use crate::app::DiffTarget;
use winc_editor::diff_view::{DiffSide, DiffView};

/// 一百行的文件提交以后改动第 61 行（下标 60），返回文件路径。
fn long_change(root: &Path) -> PathBuf {
    let file = root.join("long.txt");
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &text).unwrap();
    git_in(root, &["add", "long.txt"]);
    git_in(root, &["commit", "-q", "-m", "long"]);
    std::fs::write(&file, text.replace("line 60\n", "line sixty\n")).unwrap();
    file
}

/// 标题栏上是否有「打开文件」。
fn has_open_button(app: &mut App) -> bool {
    paint(app);
    app.hits
        .diff
        .iter()
        .any(|(_, t)| *t == DiffTarget::OpenFile)
}

/// 当前文档的路径、光标所在的（行，列）和第 `line` 行是否在屏幕上。
fn opened(app: &App, line: usize) -> (Option<PathBuf>, (usize, usize), bool) {
    let doc = app.editor.active().unwrap();
    let caret = doc.text().pos_to_line_col(doc.selection.head);
    let row = doc.row_of(line) as f64;
    let shown = row >= doc.scroll_line && row < doc.scroll_line + app.visible_lines() as f64;
    (doc.path.clone(), caret, shown)
}

#[test]
fn the_diff_header_opens_the_file_where_the_diff_is_looking() {
    let Some((mut app, root)) = git_app("git-diff-open-file", false) else {
        return;
    };
    let file = long_change(&root);
    app.git_refresh();
    app.git_preview_changes(&file);
    assert!(has_open_button(&mut app));

    // 差异滚到改动处：打开后改动的那一行仍在屏幕上，光标停在右侧最上面的一行。
    app.press_diff(DiffTarget::OpenFile);
    let (path, (line, col), shown) = opened(&app, 60);
    assert_eq!(path.as_deref(), Some(file.as_path()));
    assert!(line <= 60 && col == 0, "{line}:{col}");
    assert!(shown);
    assert_eq!(app.focus, Focus::Editor);

    // 光标在右侧：停在光标处。
    let doc = app.editor.active_mut().unwrap();
    let at = doc.text().line_col_to_pos(70, 3);
    app.select(at, at);
    app.git_preview_changes(&file);
    let index = app.editor.active.unwrap();
    app.editor.documents[index]
        .diff
        .as_deref_mut()
        .unwrap()
        .editing = true;
    app.press_diff(DiffTarget::OpenFile);
    let (path, caret, shown) = opened(&app, 70);
    assert_eq!(path.as_deref(), Some(file.as_path()));
    assert_eq!(caret, (70, 3));
    assert!(shown);

    // 文件已不在工作区中：说明原因，仍停在差异上。
    app.git_preview_changes(&file);
    let diff = app.editor.active().unwrap().id;
    std::fs::remove_file(&file).unwrap();
    app.press_diff(DiffTarget::OpenFile);
    assert_eq!(app.editor.active().unwrap().id, diff);
    let message = app.status.message.as_deref().or(app.notice.text.as_deref());
    assert_eq!(message, Some(winc_i18n::t!("git-commit-open-deleted")));

    // 没有对应文件的差异（两段文字的比较）没有这个按钮。
    let language = winc_syntax::detect("texts.txt");
    let side = |title: &str, text: &str| DiffSide::new(title, text, None, language);
    let view = DiffView::new(side("a", "one\n"), side("b", "two\n"));
    app.editor.show_diff("texts", view);
    assert!(!has_open_button(&mut app));
    let _ = std::fs::remove_dir_all(&root);
}

/// 从日志打开的历史版本差异：打开工作区中的同一个文件。
#[test]
fn a_history_diff_opens_the_file_in_the_workspace() {
    use crate::app::git::log::LogTarget;
    let Some((mut app, root)) = git_app("git-diff-open-history", false) else {
        return;
    };
    let file = long_change(&root);
    git_in(&root, &["commit", "-q", "-am", "sixty"]);
    app.git_refresh();
    show_all_log(&mut app);
    app.press_log(LogTarget::File(0));
    app.poll_log();
    assert!(app.editor.active().unwrap().diff.is_some());
    assert!(has_open_button(&mut app));
    app.press_diff(DiffTarget::OpenFile);
    let (path, _, shown) = opened(&app, 60);
    assert_eq!(path.as_deref(), Some(file.as_path()));
    assert!(shown);
    let _ = std::fs::remove_dir_all(&root);
}
