//! 模态编辑层所需接口的测试：按位置给出的修改与撤销分组、新开的行、行注释、折叠与显示行。

use winc_core::{Change, Lines, Selection};

use crate::{Command, Document, DocumentId};

fn doc(text: &str) -> Document {
    Document::from_text(DocumentId(1), text)
}

fn text(doc: &Document) -> String {
    doc.text_range(0, doc.text().end_pos()).unwrap()
}

fn insertion(start: usize, inserted: &str) -> Change {
    Change {
        start,
        removed: String::new(),
        inserted: inserted.to_owned(),
    }
}

#[test]
fn changes_apply_with_the_given_carets_and_group_into_one_unit() {
    let mut d = doc("one two\n");
    d.selection = Selection::point(4);
    d.begin_undo_group();
    let removal = Change {
        start: 4,
        removed: "two".to_owned(),
        inserted: String::new(),
    };
    assert!(d.apply_changes(vec![removal], 4, 4));
    // 分组期间经编辑命令输入的文字属于同一个撤销单元。
    d.execute(Command::Insert("x".into()), 10, 4);
    d.execute(Command::Newline, 10, 4);
    d.end_undo_group();
    assert_eq!(text(&d), "one x\n\n");

    d.execute(Command::Undo, 10, 4);
    assert_eq!(
        (text(&d).as_str(), d.selection),
        ("one two\n", Selection::point(4))
    );
    // 重做之前可以取得该单元执行之前光标所在的位置。
    assert_eq!(d.redo_origin(), Some(4));
    d.execute(Command::Redo, 10, 4);
    assert_eq!((text(&d).as_str(), d.redo_origin()), ("one x\n\n", None));

    // 修改之后的光标与撤销之后的光标各按给定的位置。
    assert!(d.apply_changes(vec![insertion(0, "ab")], 3, 1));
    assert_eq!(d.selection, Selection::point(1));
    d.execute(Command::Undo, 10, 4);
    assert_eq!(d.selection, Selection::point(3));
}

#[test]
fn read_only_documents_and_oversized_ranges_are_refused() {
    let mut d = doc("abc\n");
    assert!(d.fits_edit(0, 3));
    d.readonly = true;
    assert!(!d.apply_changes(vec![insertion(0, "x")], 0, 1));
    assert!(!d.toggle_comment_lines(0, 0, 4));
    d.open_line_at(3, 4);
    assert_eq!(text(&d), "abc\n");
}

#[test]
fn opening_a_line_indents_like_newline_and_undoes_to_the_caret() {
    let mut d = doc("    one\ntwo\n");
    d.selection = Selection::point(5);
    d.open_line_at(d.text().line_end(0), 4);
    assert_eq!(text(&d), "    one\n    \ntwo\n");
    assert_eq!(d.selection, Selection::point(12));
    d.execute(Command::Undo, 10, 4);
    assert_eq!(
        (text(&d).as_str(), d.selection),
        ("    one\ntwo\n", Selection::point(5))
    );
}

#[test]
fn comments_toggle_on_whole_lines() {
    let mut d = Document::from_text(DocumentId(1), "a\nb\nc\n");
    d.set_language("x.py");
    d.selection = Selection::point(2);
    assert!(d.toggle_comment_lines(0, 1, 4));
    assert_eq!(text(&d), "# a\n# b\nc\n");
    assert_eq!(d.selection, Selection::point(0));
    assert!(d.toggle_comment_lines(1, 1, 4));
    assert_eq!(text(&d), "# a\nb\nc\n");
    d.execute(Command::Undo, 10, 4);
    d.execute(Command::Undo, 10, 4);
    assert_eq!(
        (text(&d).as_str(), d.selection),
        ("a\nb\nc\n", Selection::point(2))
    );
}

#[test]
fn collapsed_folds_are_reported_by_any_line_inside() {
    let mut d = Document::from_text(DocumentId(1), "def f():\n    a\n    b\nc\n");
    d.set_language("x.py");
    assert!(!d.has_collapsed_folds());
    assert_eq!(d.collapsed_fold_at(1), None);
    assert!(d.toggle_fold_at(0, 4));
    assert!(d.has_collapsed_folds());
    for line in 0..=2 {
        assert_eq!(d.collapsed_fold_at(line), Some((0, 2)), "{line}");
    }
    assert_eq!(d.collapsed_fold_at(3), None);
}

#[test]
fn display_rows_follow_the_wrapping() {
    let mut d = doc("a\naaaa bbbb cccc dddd eeee\nb\n");
    let start = d.text().line_start(1);
    // 不换行时显示行就是整行，上下移动交给按文本行的移动。
    assert_eq!(d.display_row_span(start + 12), start..start + 24);
    assert_eq!(d.display_vertical(start, 1), None);

    // 宽 10 个显示列：第 1 行断成三段。
    assert!(d.ensure_wrap(Some(10), 4));
    assert_eq!(d.display_row_span(start + 12), start + 10..start + 20);
    assert_eq!(d.display_row_span(start + 22), start + 20..start + 24);
    assert_eq!(d.display_row_span(0), 0..1);
    // 上下移动保持在显示行内的同一列，越过首尾时停在边界的显示行。
    assert_eq!(d.display_vertical(start + 2, 1), Some(start + 12));
    assert_eq!(d.display_vertical(start + 12, 1), Some(start + 22));
    assert_eq!(d.display_vertical(start + 12, -2), Some(1));
    assert_eq!(d.display_vertical(start + 12, 99), Some(d.text().end_pos()));
}

#[test]
fn marks_move_with_edits_and_go_away_with_their_line() {
    let mut d = doc("one\ntwo\nthree\n");
    d.set_modal_mark('a', 5);
    d.set_modal_mark('b', 9);
    d.record_modal_change(5);
    // 标记之前的插入使标记后移；同一行上的再次修改只更新修改列表的末项。
    assert!(d.apply_changes(vec![insertion(0, "x\n")], 0, 0));
    assert_eq!((d.modal_mark('a'), d.modal_mark('b')), (Some(7), Some(11)));
    assert_eq!(d.modal_changes(), [7]);
    d.record_modal_change(8);
    assert_eq!(d.modal_changes(), [8]);
    d.record_modal_change(0);
    assert_eq!(d.modal_changes(), [8, 0]);

    // 整行被删除时该行上的标记一并删除，其余标记前移；撤销不恢复被删除的标记。
    let removal = Change {
        start: 6,
        removed: "two\n".to_owned(),
        inserted: String::new(),
    };
    assert!(d.apply_changes(vec![removal], 6, 6));
    assert_eq!((d.modal_mark('a'), d.modal_mark('b')), (None, Some(7)));
    d.execute(Command::Undo, 10, 4);
    assert_eq!((d.modal_mark('a'), d.modal_mark('b')), (None, Some(11)));

    // 行内的删除把其中的标记移到删除处。
    d.set_modal_mark('c', 3);
    let removal = Change {
        start: 2,
        removed: "one".to_owned(),
        inserted: String::new(),
    };
    assert!(d.apply_changes(vec![removal], 2, 2));
    assert_eq!(d.modal_mark('c'), Some(2));
    assert_eq!(d.modal_mark('z'), None);
}

#[test]
fn patterns_match_line_by_line_from_either_direction() {
    use crate::ModalPattern;
    let d = doc("aaaa b\n\nab ab\n");
    let find = |regex: &str, from: usize, forward: bool| {
        let pattern = ModalPattern::new(regex, false).unwrap();
        d.find_modal(&pattern, from, forward)
    };
    // 匹配互不重叠，每行从行首起依次确定。
    assert_eq!(find("aa", 0, true), Some((0, 2)));
    assert_eq!(find("aa", 1, true), Some((2, 4)));
    assert_eq!(find("aa", 3, true), None);
    assert_eq!(find("ab", 5, true), Some((8, 10)));
    assert_eq!(find("ab", 13, false), Some((11, 13)));
    assert_eq!(find("ab", 11, false), Some((8, 10)));
    assert_eq!(find("ab", 8, false), None);
    // 空的匹配：行首、行尾与空行；末尾的空行不参与查找。
    assert_eq!(find("^", 1, true), Some((7, 7)));
    assert_eq!(find("$", 0, true), Some((6, 6)));
    assert_eq!(find("$", 14, true), None);
    assert_eq!(find("a*", 5, true), Some((5, 5)));
    // 跨行的模式：起点在该行之内，可以延伸到之后的行。
    assert_eq!(find("b\\n\\n", 0, true), Some((5, 8)));
    assert_eq!(find("\\n", 7, true), Some((7, 8)));
    // 忽略大小写与无法编译的正则。
    let pattern = ModalPattern::new("AB", true).unwrap();
    assert_eq!(d.find_modal(&pattern, 0, true), Some((8, 10)));
    assert!(ModalPattern::new("(", false).is_err());
}

#[test]
fn fold_edges_come_from_the_blocks_of_the_document() {
    let mut d = Document::from_text(DocumentId(1), "a\n  b\n  c\nd\n  e\nf\n");
    assert_eq!(d.fold_edge(0, true, 4), Some(3));
    assert_eq!(d.fold_edge(3, true, 4), None);
    assert_eq!(d.fold_edge(5, false, 4), Some(4));
    assert_eq!(d.fold_edge(0, false, 4), None);
}
