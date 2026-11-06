//! [`Document::execute`] 的测试：缩进跟随文件、括号补全和成对删除、粘贴调整缩进、撤销、选区扩大与
//! 缩小、上下移动保持显示列、只读文档。

use super::*;
use crate::DocumentId;

fn doc(file: &str, text: &str, pos: usize) -> Document {
    let mut doc = Document::from_text(DocumentId(1), text);
    doc.selection = Selection::point(pos);
    doc.set_language(file);
    doc
}

fn text(doc: &Document) -> String {
    doc.text_range(0, usize::MAX).unwrap()
}

fn type_str(doc: &mut Document, s: &str) {
    for c in s.chars() {
        doc.execute(Command::Insert(c.to_string()), 10, 4);
    }
}

#[test]
fn indentation_follows_the_file_not_the_language() {
    let dir = crate::test_util::scratch("indent");

    // 文件使用 2 空格缩进，覆盖语言的默认值（4 空格）：Tab 键和回车都按 2 空格缩进。
    let two = dir.join("two.rs");
    std::fs::write(&two, "fn a() {\n  b();\n  if c {\n    d();\n  }\n}\n").unwrap();
    let mut d = Document::open(DocumentId(1), two).unwrap();
    assert_eq!(d.indent(), winc_core::Indent::Spaces(2));
    d.selection = Selection::point(d.text().line_start(1) + 6);
    d.execute(Command::Newline, 10, 4);
    let line = d.text().line_content(2).slice().to_string();
    assert_eq!(line, "  ");
    d.execute(Command::Indent, 10, 4);
    assert_eq!(d.text().line_content(2).slice().to_string(), "    ");

    // 文件使用制表符缩进，覆盖语言的默认值（4 个空格）。
    let tabs = dir.join("tabs.py");
    std::fs::write(&tabs, "def a():\n\tif b:\n\t\tc()\n").unwrap();
    let mut d = Document::open(DocumentId(2), tabs).unwrap();
    assert_eq!(d.indent(), winc_core::Indent::Tabs);
    d.selection = Selection::point(0);
    d.execute(Command::Indent, 10, 4);
    assert!(text(&d).starts_with("\tdef"));

    // 新文件无法推断缩进，使用语言的默认值。
    assert_eq!(doc("a.rs", "", 0).indent(), winc_core::Indent::Spaces(4));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_pairs_and_skips() {
    let mut d = doc("a.py", "", 0);
    type_str(&mut d, "(x)");
    assert_eq!(text(&d), "(x)");
    assert_eq!(d.selection, Selection::point(3));
}

fn backspace(file: &str, content: &str, pos: usize) -> (String, usize) {
    let mut d = doc(file, content, pos);
    d.execute(Command::Backspace { word: false }, 10, 4);
    (text(&d), d.selection.head)
}

#[test]
fn backspace_removes_empty_pairs_together() {
    assert_eq!(backspace("a.py", "()", 1), (String::new(), 0));
    assert_eq!(backspace("a.py", "f([])", 3), ("f()".to_owned(), 2));
    assert_eq!(backspace("a.py", "{}", 1), (String::new(), 0));
    assert_eq!(backspace("a.py", "\"\"", 1), (String::new(), 0));
    assert_eq!(backspace("a.py", "x = ''", 5), ("x = ".to_owned(), 4));
    assert_eq!(backspace("a.py", "[\"\", 4]", 2), ("[, 4]".to_owned(), 1));
    assert_eq!(backspace("a.js", "``", 1), (String::new(), 0));
}

#[test]
fn backspace_keeps_what_is_not_an_empty_pair() {
    // 括号内有内容，或光标不在左右括号之间。
    assert_eq!(backspace("a.py", "(a)", 1), ("a)".to_owned(), 0));
    assert_eq!(backspace("a.py", "()", 2), ("(".to_owned(), 1));
    // 左右括号不匹配。
    assert_eq!(backspace("a.py", "(]", 1), ("]".to_owned(), 0));
    // 光标两侧的引号分属两个字符串。
    assert_eq!(
        backspace("a.py", "\"a\"\"b\"", 3),
        ("\"a\"b\"".to_owned(), 2)
    );
    // 该引号不是当前语言的字符串定界符。
    assert_eq!(backspace("a.rs", "\'\'", 1), ("\'".to_owned(), 0));
    // 三引号字符串的引号逐个删除。
    assert_eq!(backspace("a.py", "\"\"\"\"", 2), ("\"\"\"".to_owned(), 1));
}

#[test]
fn backspacing_a_word_does_not_remove_the_pair() {
    let mut d = doc("a.py", "()", 1);
    d.execute(Command::Backspace { word: true }, 10, 4);
    assert_eq!(text(&d), ")");
}

#[test]
fn undo_restores_the_pair_and_the_caret_between_it() {
    let mut d = doc("a.py", "()", 1);
    d.execute(Command::Backspace { word: false }, 10, 4);
    assert_eq!(text(&d), "");
    d.execute(Command::Undo, 10, 4);
    assert_eq!(text(&d), "()");
    assert_eq!(d.selection, Selection::point(1));
}

#[test]
fn quote_rules_follow_language() {
    let mut d = doc("a.py", "don", 3);
    type_str(&mut d, "'");
    assert_eq!(text(&d), "don'");

    let mut d = doc("a.rs", "fn f<", 5);
    type_str(&mut d, "'a");
    assert_eq!(text(&d), "fn f<'a");

    let mut d = doc("a.py", "", 0);
    type_str(&mut d, "'");
    assert_eq!(text(&d), "''");

    // 连续输入三个引号：第二个跳过自动补全的引号，第三个不再补全（代码块围栏、三引号字符串）。
    let mut d = doc("a.md", "", 0);
    type_str(&mut d, "```");
    assert_eq!(text(&d), "```");
    assert_eq!(d.selection, Selection::point(3));
    let mut d = doc("a.py", "", 0);
    type_str(&mut d, "\"\"\"");
    assert_eq!(text(&d), "\"\"\"");
}

#[test]
fn wraps_selection() {
    let mut d = doc("a.py", "abc", 0);
    d.selection = Selection::new(0, 3);
    d.execute(Command::Insert("\"".into()), 10, 4);
    assert_eq!(text(&d), "\"abc\"");
    assert_eq!(d.selection, Selection::new(1, 4));

    // Markdown 强调记号：有选区时包住选区，再按一次成为粗体；没有选区时正常输入。
    let mut d = doc("a.md", "abc", 0);
    d.selection = Selection::new(0, 3);
    type_str(&mut d, "**");
    assert_eq!(text(&d), "**abc**");
    assert_eq!(d.selection, Selection::new(2, 5));
    d.selection = Selection::point(7);
    type_str(&mut d, "_");
    assert_eq!(text(&d), "**abc**_");
    // 其他语言中输入强调记号时替换选区。
    let mut d = doc("a.py", "abc", 0);
    d.selection = Selection::new(0, 3);
    type_str(&mut d, "*");
    assert_eq!(text(&d), "*");
}

#[test]
fn undo_restores_and_clears_dirty() {
    let mut d = doc("a.py", "x", 1);
    assert!(!d.is_dirty());
    d.execute(Command::Insert("y".into()), 10, 4);
    assert!(d.is_dirty());
    d.execute(Command::Undo, 10, 4);
    assert_eq!(text(&d), "x");
    assert!(!d.is_dirty());
}

#[test]
fn paste_normalizes_line_endings() {
    let mut d = doc("a.py", "", 0);
    d.line_ending = winc_core::LineEnding::CrLf;
    d.execute(Command::Paste("a\nb\r\nc".into()), 10, 4);
    assert_eq!(text(&d), "a\r\nb\r\nc");

    // 调整缩进后粘贴：一次撤销即可还原。
    let mut d = doc("a.py", "if x:\n    ", 10);
    d.execute(Command::Paste("        a()\n        b()\n".into()), 10, 4);
    assert_eq!(text(&d), "if x:\n    a()\n    b()\n");
    d.execute(Command::Undo, 10, 4);
    assert_eq!(text(&d), "if x:\n    ");
}

#[test]
fn readonly_rejects_edits_but_moves() {
    let mut d = doc("a.py", "abc", 0);
    d.readonly = true;
    d.execute(Command::Insert("x".into()), 10, 4);
    d.execute(
        Command::MoveRight {
            word: false,
            extend: false,
        },
        10,
        4,
    );
    assert_eq!(text(&d), "abc");
    assert_eq!(d.selection, Selection::point(1));
}

#[test]
fn shrinking_retraces_the_expansion_until_the_selection_moves() {
    let mut d = doc("a.py", "f(a, bb)", 6);
    d.execute(Command::ExpandSelection, 10, 4);
    assert_eq!(d.selection, Selection::new(5, 7));
    d.execute(Command::ExpandSelection, 10, 4);
    assert_eq!(d.selection, Selection::new(2, 7));
    d.execute(Command::ShrinkSelection, 10, 4);
    assert_eq!(d.selection, Selection::new(5, 7));
    d.execute(Command::ShrinkSelection, 10, 4);
    assert_eq!(d.selection, Selection::point(6));
    // 已退回到最初的选区；移动过光标之后，也不能再缩回此前记录的层级。
    d.execute(Command::ShrinkSelection, 10, 4);
    assert_eq!(d.selection, Selection::point(6));
    d.execute(Command::ExpandSelection, 10, 4);
    d.execute(Command::LineEnd { extend: false }, 10, 4);
    d.execute(Command::ShrinkSelection, 10, 4);
    assert_eq!(d.selection, Selection::point(8));

    // 跳到配对括号是移动，只读文档中也可用；改变大小写是修改，可以撤销。
    d.readonly = true;
    d.execute(Command::JumpToBracket, 10, 4);
    assert_eq!(d.selection, Selection::point(1));
    d.readonly = false;
    d.execute(Command::ChangeCase(Case::Upper), 10, 4);
    assert_eq!(text(&d), "F(a, bb)");
    d.execute(Command::Undo, 10, 4);
    assert_eq!(text(&d), "f(a, bb)");
}

#[test]
fn vertical_movement_keeps_goal() {
    let mut d = doc("a.py", "abcdef\nab\nabcdef", 5);
    d.execute(Command::MoveDown { extend: false }, 10, 4);
    d.execute(Command::MoveDown { extend: false }, 10, 4);
    assert_eq!(d.selection, Selection::point(15));
    d.execute(
        Command::MoveLeft {
            word: false,
            extend: false,
        },
        10,
        4,
    );
    assert_eq!(d.goal_cell, None);
}
