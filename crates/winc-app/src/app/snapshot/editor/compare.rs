//! 当前文档与剪贴板、与已保存的版本比较：打开的差异标签、右侧随文档编辑，以及无法比较时的提示。

use winc_editor::diff_view::DiffView;

use crate::keymap::Action;

use super::*;

/// 测试目录中 `src/main.rs` 的内容（见 `sample_folder`）。
const MAIN: &str = "fn main() {\n    println!(\"hello\");\n}\n";

/// 打开测试目录中的 `src/main.rs`，返回应用和该文档的路径。
fn main_open(name: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let file = app.editor.active().unwrap().path.clone().unwrap();
    (app, file)
}

/// 当前标签的差异视图。
fn active_view(app: &App) -> &DiffView {
    app.editor.active().unwrap().diff.as_deref().unwrap()
}

#[test]
fn the_whole_file_is_compared_with_the_clipboard_and_stays_editable() {
    let (mut app, file) = main_open("compare-clipboard");
    let source = app.editor.active().unwrap().id;

    // 剪贴板为空：只提示，不打开标签。
    let tabs = app.editor.documents.len();
    app.run_action(Action::CompareWithClipboard);
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(app.status.message.as_deref(), Some("剪贴板中没有文字"));

    // 内容相同（换行符的写法不同也算相同）：只提示。
    app.clipboard.copied = Some(MAIN.replace('\n', "\r\n"));
    app.run_action(Action::CompareWithClipboard);
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(
        app.status.message.as_deref(),
        Some("「main.rs」与剪贴板中的文字相同")
    );

    // 左侧是剪贴板，右侧连着文档：可以编辑，随文档重新比较。
    app.clipboard.copied = Some(MAIN.replace("hello", "world"));
    app.run_action(Action::CompareWithClipboard);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("剪贴板 ↔ main.rs"));
    let view = active_view(&app);
    assert_eq!(view.old.title, "剪贴板");
    // 本机路径按本机的分隔符显示。
    let relative = Path::new("src").join("main.rs");
    assert_eq!(view.new.title, relative.to_string_lossy());
    assert_eq!(view.new.path.as_deref(), Some(file.as_path()));
    assert!(!view.is_identical());

    let diff = app.editor.active.unwrap();
    let at = app.editor.index_of(source).unwrap();
    app.editor.active = Some(at);
    app.run(Command::Insert("// 新的一行\n".into()));
    app.sync_diff_source(diff);
    app.editor.active = Some(diff);
    assert!(
        active_view(&app)
            .new
            .text
            .to_string()
            .starts_with("// 新的一行")
    );

    // 再比较一次：替换同一个标签，不再新开。
    let tabs = app.editor.documents.len();
    app.editor.active = Some(at);
    app.run_action(Action::CompareWithClipboard);
    assert_eq!(app.editor.documents.len(), tabs);
}

#[test]
fn a_selection_is_compared_with_the_clipboard_as_a_snapshot() {
    let (mut app, _) = main_open("compare-selection");
    let doc = app.editor.active_mut().unwrap();
    let (from, to) = (doc.text().line_start(1), doc.text().line_start(3));
    doc.selection = winc_core::Selection::new(from, to);

    app.clipboard.copied = Some("    println!(\"hello\");\n}\n".into());
    app.run_action(Action::CompareWithClipboard);
    assert_eq!(
        app.status.message.as_deref(),
        Some("所选内容与剪贴板中的文字相同")
    );

    app.clipboard.copied = Some("    println!(\"world\");\n}\n".into());
    app.run_action(Action::CompareWithClipboard);
    let view = active_view(&app);
    assert_eq!(view.new.title, "main.rs 第 2–3 行");
    assert_eq!(view.new.path, None, "选中部分是快照，不连着文档");
    assert_eq!(view.new.text.to_string(), "    println!(\"hello\");\n}\n");
}

#[test]
fn unsaved_changes_are_compared_with_the_saved_version() {
    let (mut app, file) = main_open("compare-saved");

    // 没有修改：与已保存的版本相同。
    app.run_action(Action::CompareWithSaved);
    assert_eq!(
        app.status.message.as_deref(),
        Some("「main.rs」与已保存的版本相同")
    );

    // 有未保存的修改：左侧是磁盘上的内容，右侧连着文档。
    app.run(Command::Insert("// 未保存\n".into()));
    app.run_action(Action::CompareWithSaved);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("已保存的版本 ↔ main.rs"));
    let view = active_view(&app);
    assert_eq!(view.old.title, "已保存的版本");
    assert_eq!(view.old.text.to_string(), MAIN);
    assert_eq!(view.new.path.as_deref(), Some(file.as_path()));

    // 差异标签自己没有可比较的文本。
    app.run_action(Action::CompareWithSaved);
    assert_eq!(
        app.status.message.as_deref(),
        Some("当前标签页中没有可比较的文本")
    );

    // 未命名的文档没有已保存的版本；与剪贴板比较时右侧是快照。
    app.new_file();
    app.run(Command::Insert("草稿".into()));
    app.run_action(Action::CompareWithSaved);
    assert_eq!(
        app.status.message.as_deref(),
        Some("「untitled」尚未保存过")
    );
    app.clipboard.copied = Some("定稿".into());
    app.run_action(Action::CompareWithClipboard);
    assert_eq!(active_view(&app).new.path, None);
}

/// 截图用的源文件。
const SOURCE: &str = "use std::collections::HashMap;

/// 统计每个词出现的次数。
pub fn count(words: &[&str]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for word in words {
        *counts.entry(word.to_string()).or_insert(0) += 1;
    }
    counts
}

/// 返回出现次数最多的词。
pub fn most_common(counts: &HashMap<String, usize>) -> Option<&str> {
    counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(word, _)| word.as_str())
}
";

/// 截图：编辑区右键菜单展开「比较」子菜单，与剪贴板比较、与已保存的版本比较的差异标签
/// （`compare-menu-*`、`compare-clipboard-*`、`compare-saved-*`，浅色与深色）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_compare() {
    use crate::app::PopupRow;

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let dir = sample_folder(&format!("compare-shot-{theme}"));
        let file = dir.join("src/lib.rs");
        std::fs::write(&file, SOURCE).unwrap();
        let mut app = app_with(&dir, dark);
        app.open_path(&file);
        paint(&mut app);

        // 右键菜单：停在「比较」上展开子菜单。
        let text = app.layout.text;
        let line = app.fonts.line_height;
        right_click(&mut app, (text.x + 120.0, text.y + 4.5 * line));
        paint(&mut app);
        let top = app.popup().unwrap().levels[0].clone();
        let at = (top.rows.iter())
            .position(|row| matches!(row, PopupRow::Submenu { .. }))
            .unwrap();
        let row = top.row_rects[at];
        app.on_mouse_move(row.x + 40.0, row.y + row.h / 2.0);
        shoot(&mut app, &format!("compare-menu-{theme}"));
        app.overlays.context_menu = None;

        // 剪贴板中是改过的另一份。
        let clipped = SOURCE
            .replace("word.to_string()", "word.to_lowercase()")
            .replace(
                "    counts\n}\n",
                "    counts.retain(|_, n| *n > 1);\n    counts\n}\n",
            )
            .replace("/// 返回出现次数最多的词。\n", "");
        app.clipboard.copied = Some(clipped);
        app.run_action(Action::CompareWithClipboard);
        shoot(&mut app, &format!("compare-clipboard-{theme}"));

        // 有未保存的修改，与已保存的版本比较。
        app.open_path(&file);
        let doc = app.editor.active().unwrap();
        let at = doc.text().line_start(12);
        app.select(at, at);
        app.run(Command::Insert("#[must_use]\n".into()));
        app.run_action(Action::CompareWithSaved);
        shoot(&mut app, &format!("compare-saved-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 命令行上的 `--diff`：每对文件开一个差异标签，左侧是第一个文件；内容相同时同样打开并提示；
/// 读不了的一对说明原因，不开标签。
#[test]
fn files_given_with_diff_open_side_by_side() {
    use crate::cli::Diff;
    let dir = sample_folder("compare-cli");
    std::fs::write(dir.join("old.txt"), "a\nb\n").unwrap();
    std::fs::write(dir.join("new.txt"), "a\nc\n").unwrap();
    std::fs::write(dir.join("same.txt"), "a\nb\n").unwrap();
    std::fs::write(dir.join("binary.bin"), [0u8, 1, 2, 0]).unwrap();
    let mut app = app_with(&dir, false);
    let diff = |left: &str, right: &str| Diff {
        left: dir.join(left),
        right: dir.join(right),
    };

    app.open_diffs(&[diff("old.txt", "new.txt")]);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("old.txt ↔ new.txt"));
    assert_eq!(
        (
            active_view(&app).old.title.as_str(),
            active_view(&app).new.title.as_str()
        ),
        ("old.txt", "new.txt")
    );

    app.open_diffs(&[diff("old.txt", "same.txt")]);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("old.txt ↔ same.txt"));
    assert!(active_view(&app).is_identical());
    assert_eq!(
        app.status.message.as_deref(),
        Some("「old.txt」与「same.txt」相同")
    );

    let tabs = app.editor.documents.len();
    app.open_diffs(&[diff("old.txt", "binary.bin")]);
    assert_eq!(app.editor.documents.len(), tabs);
    let notice = app.notice.text.take().unwrap_or_default();
    assert!(notice.contains("binary.bin"), "{notice}");
    let _ = std::fs::remove_dir_all(&dir);
}
