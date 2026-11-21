//! macOS 上 `⌃` 字母键对应的编辑命令：两套方案各自的行首、行尾，删除和剪切到行尾，交换字符，
//! 插入换行，滚到中间，以及等同于方向键、删除键的命令。

use super::*;
use crate::keymap::Action;

fn text(app: &App) -> String {
    app.editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string()
}

fn head(app: &App) -> usize {
    app.editor.active().unwrap().selection.head
}

/// 准备一个只有 `content` 的文件，光标在 `at`。
fn editor_with(name: &str, content: &str, at: usize) -> App {
    let dir = sample_folder(name);
    let file = dir.join("short.rs");
    std::fs::write(&file, content).unwrap();
    let mut app = app_with(&file, false);
    app.select(at, at);
    app
}

/// 「光标移到行首」与 `Home` 相同，先停在首个非空白字符；「整行开头」「整行末尾」直接到第 0 列
/// 和行尾。
#[test]
fn line_start_and_end_follow_each_editor() {
    let mut app = editor_with("control-line-start", "    let x = 1;\n", 14);
    assert!(app.run_action(Action::CursorHome));
    assert_eq!(head(&app), 4);
    app.select(14, 14);
    assert!(app.run_action(Action::CursorLineStart));
    assert_eq!(head(&app), 0);
    assert!(app.run_action(Action::CursorLineEnd));
    assert_eq!(head(&app), 14);
}

/// 剪切到行尾把剪下的文字放进剪贴板，在行尾时剪下换行符；删除到行尾不动剪贴板。
#[test]
fn cut_and_delete_to_line_end() {
    let mut app = editor_with("control-kill", "abc def\nxyz\n", 3);
    assert!(app.run_action(Action::CutToLineEnd));
    assert_eq!(text(&app), "abc\nxyz\n");
    assert_eq!(app.clipboard_text().as_deref(), Some(" def"));
    assert!(app.run_action(Action::CutToLineEnd));
    assert_eq!(text(&app), "abcxyz\n");
    assert_eq!(app.clipboard_text().as_deref(), Some("\n"));
    assert!(app.run_action(Action::DeleteAllRight));
    assert_eq!(text(&app), "abc\n");
    assert_eq!(app.clipboard_text().as_deref(), Some("\n"));
}

/// 交换字符后光标移到两个字符之后；插入换行后光标留在原处。
#[test]
fn transpose_and_line_break() {
    let mut app = editor_with("control-transpose", "ab\n", 1);
    assert!(app.run_action(Action::TransposeLetters));
    assert_eq!((text(&app).as_str(), head(&app)), ("ba\n", 2));
    app.select(1, 1);
    assert!(app.run_action(Action::InsertLineBreak));
    assert_eq!((text(&app).as_str(), head(&app)), ("b\na\n", 1));
}

/// 等同于方向键、删除键的命令按真的按键处理：按着 `⌃` 也只移动、删除一个字符，修饰键随后恢复。
#[test]
fn arrow_and_delete_commands_act_like_the_keys() {
    let mut app = editor_with("control-arrows", "abc\ndef\n", 1);
    app.pointer.modifiers = winit::keyboard::ModifiersState::CONTROL;
    for (action, at) in [
        (Action::CursorDown, 5),
        (Action::CursorRight, 6),
        (Action::CursorUp, 2),
        (Action::CursorLeft, 1),
    ] {
        assert!(app.run_action(action));
        assert_eq!(head(&app), at, "{action:?}");
    }
    assert_eq!(
        app.pointer.modifiers,
        winit::keyboard::ModifiersState::CONTROL
    );
    assert!(app.run_action(Action::DeleteRight));
    assert_eq!(text(&app), "ac\ndef\n");
    assert!(app.run_action(Action::DeleteLeft));
    assert_eq!((text(&app).as_str(), head(&app)), ("c\ndef\n", 0));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
}

/// 滚到中间：光标所在行停在编辑区的正中，光标不动。
#[test]
fn scroll_to_center_puts_the_caret_line_in_the_middle() {
    let lines: String = (0..300).map(|n| format!("line {n}\n")).collect();
    let at = lines.find("line 150").unwrap();
    let mut app = editor_with("control-center", &lines, at);
    paint(&mut app);
    assert!(app.run_action(Action::ScrollToCenter));
    let visible = app.visible_lines() as f64;
    let doc = app.editor.active().unwrap();
    assert!(
        (doc.scroll_line - (150.5 - visible / 2.0)).abs() < 1.0,
        "{} {visible}",
        doc.scroll_line
    );
    assert_eq!(head(&app), at);
}
