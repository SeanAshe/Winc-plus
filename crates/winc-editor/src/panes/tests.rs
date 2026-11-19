//! `panes` 的测试：拆分、切换和合并窗格，各窗格的标签与视图状态，以及视图状态随修改移动。

use super::*;
use crate::{Command, Document, Editor};
use winc_core::Selection;

/// 前台窗格开着三个未命名文档，内容各是十行。
fn editor() -> Editor {
    let mut editor = Editor::default();
    for _ in 0..3 {
        let index = editor.new_untitled();
        let text = (0..10).map(|i| format!("line {i}\n")).collect::<String>();
        editor.documents[index].execute(Command::Insert(text), 10, 4);
    }
    editor
}

fn ids(editor: &Editor) -> Vec<u64> {
    editor.tabs().iter().map(|d| d.id.0).collect()
}

fn caret(doc: &Document) -> usize {
    doc.selection.head
}

#[test]
fn splitting_opens_the_current_tab_in_a_new_pane_with_its_own_view() {
    let mut editor = editor();
    editor.active = Some(1);
    editor.documents[1].selection = Selection::point(12);
    editor.documents[1].scroll_line = 2.0;
    let first = editor.panes.focused;

    let second = editor.split(Axis::Row).unwrap();
    assert_eq!(editor.panes.focused, second);
    assert_eq!(ids(&editor), [2]);
    assert_eq!(editor.panes.tree.panes(), [first, second]);
    // 新窗格从同一处开始，之后各走各的。
    assert_eq!(caret(editor.active().unwrap()), 12);
    editor.documents[0].selection = Selection::point(30);

    assert!(editor.focus_pane(first));
    assert_eq!(ids(&editor), [1, 2, 3]);
    assert_eq!(editor.active().unwrap().id.0, 2);
    assert_eq!(caret(editor.active().unwrap()), 12);
    assert_eq!(editor.active().unwrap().scroll_line, 2.0);

    editor.focus_pane(second);
    assert_eq!(caret(editor.active().unwrap()), 30);
    // 所有文档都还在，只在其他窗格里的排在后面。
    assert_eq!(editor.documents.len(), 3);
    assert_eq!(editor.tab_count(), 1);
}

#[test]
fn views_kept_by_other_panes_move_with_edits() {
    let mut editor = editor();
    editor.active = Some(0);
    let first = editor.panes.focused;
    editor.documents[0].selection = Selection::point(42);
    editor.documents[0].scroll_line = 5.0;
    editor.split(Axis::Column);

    // 在新窗格里于开头插入两行：留在原窗格里的光标和滚动位置跟着往下移。
    let doc = &mut editor.documents[0];
    doc.selection = Selection::point(0);
    doc.execute(Command::Insert("a\nb\n".into()), 10, 4);
    editor.focus_pane(first);
    let doc = editor.active().unwrap();
    assert_eq!(caret(doc), 46);
    assert_eq!(doc.scroll_line, 7.0);
}

#[test]
fn tabs_open_in_the_focused_pane_and_closing_keeps_documents_shown_elsewhere() {
    let mut editor = editor();
    editor.active = Some(0);
    let first = editor.panes.focused;
    let second = editor.split(Axis::Row).unwrap();

    // 新建的文档只进前台窗格。
    editor.new_untitled();
    assert_eq!(ids(&editor), [1, 4]);
    editor.focus_pane(first);
    assert_eq!(ids(&editor), [1, 2, 3]);

    // 关掉另一个窗格里也开着的文档：只去掉这里的标签，文档还在。
    assert!(editor.close(0).is_none());
    assert_eq!(ids(&editor), [2, 3]);
    assert!(editor.document(crate::DocumentId(1)).is_some());
    // 只在这里开着的文档真正关掉。
    assert!(editor.close(0).is_some());

    // 打开只在另一个窗格里的文档：成为这里的一个标签。
    let index = editor.index_of(crate::DocumentId(4)).unwrap();
    assert_eq!(editor.bring(index), 1);
    assert_eq!(ids(&editor), [3, 4]);
    assert_eq!(
        editor.pane_tabs(second).0,
        [crate::DocumentId(1), crate::DocumentId(4)]
    );
}

#[test]
fn an_emptied_pane_goes_away_and_unsplit_gathers_every_tab() {
    let mut editor = editor();
    editor.active = Some(0);
    let first = editor.panes.focused;
    let second = editor.split(Axis::Row).unwrap();
    editor.new_untitled();
    let third = editor.split(Axis::Column).unwrap();
    assert_eq!(editor.panes.tree.panes(), [first, second, third]);

    // 关掉第三个窗格唯一的标签：窗格去掉，前台换到它前面的窗格。
    assert!(editor.close(0).is_none());
    assert!(editor.drop_empty_pane());
    assert_eq!(editor.panes.focused, second);
    assert_eq!(editor.panes.tree.panes(), [first, second]);

    editor.focus_pane(first);
    editor.unsplit();
    assert!(!editor.panes.is_split());
    assert_eq!(ids(&editor), [1, 2, 3, 4]);
    assert_eq!(editor.panes.tree, Node::Pane(first));
}

#[test]
fn removing_a_tab_from_another_pane_closes_the_pane_when_it_empties() {
    let mut editor = editor();
    editor.active = Some(2);
    let first = editor.panes.focused;
    editor.split(Axis::Row);
    editor.focus_pane(first);
    let second = editor.panes.others[0].id;
    editor.remove_from_pane(second, crate::DocumentId(3));
    assert!(!editor.panes.is_split());
    assert_eq!(editor.panes.tree, Node::Pane(first));
}

#[test]
fn restoring_drops_closed_documents_and_empty_panes() {
    let mut editor = editor();
    let tree = Node::Split {
        axis: Axis::Row,
        share: 0.3,
        first: Box::new(Node::Pane(PaneId(4))),
        second: Box::new(Node::Split {
            axis: Axis::Column,
            share: 0.5,
            first: Box::new(Node::Pane(PaneId(5))),
            second: Box::new(Node::Pane(PaneId(6))),
        }),
    };
    let tabs = |id, tabs: &[u64], active| PaneTabs {
        id: PaneId(id),
        tabs: tabs.iter().map(|&t| crate::DocumentId(t)).collect(),
        active: Some(crate::DocumentId(active)),
    };
    let panes = vec![tabs(4, &[1, 2], 2), tabs(5, &[9], 9), tabs(6, &[3], 3)];
    editor.restore_panes(tree, panes);
    assert_eq!(editor.panes.focused, PaneId(4));
    assert_eq!(ids(&editor), [1, 2]);
    assert_eq!(editor.active().unwrap().id.0, 2);
    assert_eq!(editor.panes.tree.panes(), [PaneId(4), PaneId(6)]);
    assert_eq!(editor.panes.allocate(), PaneId(7));
}

#[test]
fn a_tab_from_another_pane_splits_off_before_the_focused_one_with_its_view() {
    let mut editor = editor();
    editor.active = Some(0);
    let first = editor.panes.focused;
    editor.documents[0].selection = Selection::point(24);
    let second = editor.split(Axis::Row).unwrap();
    editor.documents[0].selection = Selection::point(7);

    // 在右边的窗格上方拆出一个窗格，打开左边窗格里的第一个文档，光标取左边的。
    let doc = crate::DocumentId(1);
    let third = editor.split_with(Axis::Column, true, doc, first).unwrap();
    assert_eq!(editor.panes.focused, third);
    assert_eq!(ids(&editor), [1]);
    assert_eq!(caret(editor.active().unwrap()), 24);
    let tree = Node::Split {
        axis: Axis::Row,
        share: 0.5,
        first: Box::new(Node::Pane(first)),
        second: Box::new(Node::Split {
            axis: Axis::Column,
            share: 0.5,
            first: Box::new(Node::Pane(third)),
            second: Box::new(Node::Pane(second)),
        }),
    };
    assert_eq!(editor.panes.tree, tree);

    // 把左边窗格里的第二个文档也收进来，光标和滚动位置从左边来。
    assert_eq!(editor.adopt_tab(crate::DocumentId(2), first), Some(1));
    assert_eq!(ids(&editor), [1, 2]);
}

#[test]
fn wraps_kept_by_other_panes_follow_edits() {
    let mut editor = Editor::default();
    let index = editor.new_untitled();
    let long = "abcd ".repeat(6);
    let text = format!("{long}\n{long}\n");
    editor.documents[index].execute(Command::Insert(text), 10, 4);
    editor.documents[index].ensure_wrap(Some(10), 4);
    let first = editor.panes.focused;
    let rows = editor.documents[0].rows_of(0);
    assert!(rows > 1);

    // 另一个窗格不换行，在开头插入一行长的：原窗格收着的断行跟着往下移，并算上新的一行。
    editor.split(Axis::Row);
    let doc = &mut editor.documents[0];
    doc.ensure_wrap(None, 4);
    doc.selection = Selection::point(0);
    doc.execute(Command::Insert(format!("{long}\n")), 10, 4);
    editor.focus_pane(first);
    let doc = editor.active().unwrap();
    assert_eq!(doc.folds.wrap().unwrap().revision, doc.revision);
    assert_eq!(
        (doc.rows_of(0), doc.rows_of(1), doc.rows_of(2)),
        (rows, rows, rows)
    );
}

#[test]
fn switching_panes_leaves_documents_shown_in_one_pane_alone() {
    let mut editor = editor();
    editor.active = Some(0);
    let first = editor.panes.focused;
    editor.documents[2].selection = Selection::point(15);
    editor.split(Axis::Row);
    // 只在左边开着的第三个文档：切来切去都不收起它的视图状态。
    editor.focus_pane(first);
    let only_left = editor.document(crate::DocumentId(3)).unwrap();
    assert!(only_left.parked.is_empty());
    assert_eq!(caret(only_left), 15);
    // 两边都开着的第一个文档各有一份。
    assert!(!editor.documents[0].parked.is_empty());
}
