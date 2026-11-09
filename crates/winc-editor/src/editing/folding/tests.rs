//! 折叠的测试：折叠状态随修改移动、显示行换算、按行筛选、自动换行的断行与追加。

use super::*;
use crate::{Command, DocumentId};

const PY: &str = "\
class A:
    def f(self):
        if x:
            a()
        return 1

    def g(self):
        pass
x = 1
";

fn doc(text: &str) -> Document {
    let mut doc = Document::from_text(DocumentId(1), text);
    doc.set_language("a.py");
    doc
}

fn put(doc: &mut Document, line: usize) {
    let pos = doc.text().line_start(line);
    doc.selection = Selection::point(pos);
}

fn collapsed(doc: &Document) -> Vec<(usize, usize)> {
    doc.folds().iter().map(|f| (f.start, f.end)).collect()
}

#[test]
fn filtering_shows_only_the_given_lines() {
    let mut d = doc("a\nb\nc\nd\ne\nf\n");
    // 第 0 行也可以被隐藏，此时光标移到第一个显示的行上。
    assert!(d.filter_lines(Some(&[2, 4]), usize::MAX));
    assert_eq!(d.folds().hidden, [(0, 1), (3, 3), (5, 6)]);
    assert!(d.folds().is_filtered());
    assert_eq!(d.row_count(), 2);
    assert_eq!((d.line_of_row(0), d.line_of_row(1)), (2, 4));
    assert_eq!(
        (d.row_of(0), d.row_of(2), d.row_of(3), d.row_of(4)),
        (0, 0, 0, 1)
    );
    assert_eq!(d.text().line_of(d.selection.head), 2);

    // 上下移动只经过显示的行。
    d.execute(Command::MoveDown { extend: false }, 10, 4);
    assert_eq!(d.text().line_of(d.selection.head), 4);

    // 与折叠块合并计算。
    let mut d = doc("fn a() {\n    x\n    y\n}\nz\n");
    d.run_fold(FoldCommand::FoldAll, 4);
    d.filter_lines(Some(&[0, 4]), usize::MAX);
    assert_eq!(d.row_count(), 2);
    assert_eq!(d.folds().iter().count(), 1, "折起来的块还在");

    // 尚未搜索到的行仍然显示；没有任何匹配时保留最后一行。
    let mut d = doc("a\nb\nc\nd");
    d.filter_lines(Some(&[0]), 2);
    assert_eq!(d.row_count(), 3);
    d.filter_lines(Some(&[]), usize::MAX);
    assert_eq!(d.row_count(), 1);

    // 修改文本后取消筛选（待重新搜索完成后再筛选）；取消筛选后所有行都显示。
    d.filter_lines(Some(&[0]), usize::MAX);
    d.selection = Selection::point(0);
    d.execute(Command::Insert("x".into()), 10, 4);
    assert!(!d.folds().is_filtered());
    d.filter_lines(Some(&[0]), usize::MAX);
    d.filter_lines(None, usize::MAX);
    assert!(d.folds().is_empty());

    // 文件只有一行且没有匹配时该行仍然显示：行数相减不能得到负数。
    for text in ["abc", ""] {
        let mut d = doc(text);
        d.filter_lines(Some(&[]), usize::MAX);
        assert!(!d.folds().is_hidden(0));
        assert_eq!(d.row_count(), 1);
    }
}

#[test]
fn rows_skip_folded_lines() {
    let mut d = doc(PY);
    put(&mut d, 2);
    assert!(d.run_fold(FoldCommand::Fold, 4));
    // 光标在 `if` 所在的行：折叠 `if` 块。
    assert_eq!(collapsed(&d), [(2, 3)]);
    assert_eq!((d.row_of(4), d.line_of_row(3)), (3, 4));
    // 再次折叠时折叠外层的块；不显示的行合并为一段。
    d.run_fold(FoldCommand::Fold, 4);
    assert_eq!(collapsed(&d), [(1, 4), (2, 3)]);
    assert_eq!(d.folds().hidden, [(2, 4)]);
    // 光标所在的 `if` 行被隐藏：光标移到 `def` 行的行尾。
    assert_eq!(d.selection, Selection::point(d.text().line_end(1)));
    assert_eq!(d.row_count(), PY.lines().count() + 1 - 3);
    assert_eq!(d.row_of(3), 1);
    assert_eq!(d.line_of_row(2), 5);
    assert_eq!(d.folds().segments(0, 4, 10), [0..2, 5..7]);
    assert_eq!(d.folds().segments(1, 100, 10), [1..2, 5..10]);

    // 展开外层的块，内层的块仍保持折叠。
    d.run_fold(FoldCommand::Unfold, 4);
    assert_eq!(collapsed(&d), [(2, 3)]);
    d.run_fold(FoldCommand::UnfoldAll, 4);
    assert!(d.folds().is_empty());
}

#[test]
fn the_change_between_two_fold_states() {
    let mut d = doc(PY);
    let before = d.folds().clone();
    // 折叠 `if`：显示行 3（`a()`）被隐藏。
    d.toggle_fold_at(2, 4);
    let after = d.folds().clone();
    assert_eq!(Folds::changed_rows(&before, &after), Some(3..4));
    // 内层的 `if` 已折叠时再折叠外层的 `def`：`if` 行和 `return` 行被隐藏，二者按折叠前的
    // 显示行号连续（`a()` 已经隐藏）。
    let before = after;
    d.toggle_fold_at(1, 4);
    let after = d.folds().clone();
    assert_eq!(Folds::changed_rows(&before, &after), Some(2..4));
    // 展开时调用方把两个状态对调后传入，结果相同。多处同时变化（全部折叠），或两侧各自隐藏了
    // 对方未隐藏的行时，均没有结果。
    let none = Folds::default();
    d.run_fold(FoldCommand::UnfoldAll, 4);
    d.run_fold(FoldCommand::FoldLevel(2), 4);
    assert_eq!(Folds::changed_rows(&none, d.folds()), None);
    let mut other = doc(PY);
    other.toggle_fold_at(6, 4);
    let mut one = doc(PY);
    one.toggle_fold_at(2, 4);
    assert_eq!(Folds::changed_rows(one.folds(), other.folds()), None);
}

#[test]
fn fold_all_levels_and_recursion() {
    let mut d = doc(PY);
    put(&mut d, 8);
    d.run_fold(FoldCommand::FoldAll, 4);
    assert_eq!(collapsed(&d), [(0, 7), (1, 4), (2, 3), (6, 7)]);
    assert_eq!(d.row_count(), 3);
    d.run_fold(FoldCommand::UnfoldAll, 4);

    // 第二层：两个方法，光标所在的方法除外。
    put(&mut d, 7);
    d.run_fold(FoldCommand::FoldLevel(2), 4);
    assert_eq!(collapsed(&d), [(1, 4)]);
    d.run_fold(FoldCommand::UnfoldAll, 4);

    put(&mut d, 1);
    d.run_fold(FoldCommand::FoldRecursively, 4);
    assert_eq!(collapsed(&d), [(1, 4), (2, 3)]);
    d.run_fold(FoldCommand::UnfoldRecursively, 4);
    assert!(d.folds().is_empty());

    d.run_fold(FoldCommand::Toggle, 4);
    assert_eq!(collapsed(&d), [(1, 4)]);
    d.run_fold(FoldCommand::Toggle, 4);
    assert!(d.folds().is_empty());
}

#[test]
fn edits_move_folds_and_crossing_edits_unfold() {
    let mut d = doc(PY);
    d.toggle_fold_at(6, 4);
    assert_eq!(collapsed(&d), [(6, 7)]);

    // 在上方插入一行：折叠块随之下移。
    put(&mut d, 0);
    d.execute(Command::Insert("# top\n".into()), 10, 4);
    assert_eq!(collapsed(&d), [(7, 8)]);
    // 在块第一行的行尾输入：仍保持折叠。
    d.selection = Selection::point(d.text().line_end(7));
    d.execute(Command::Insert(" # ok".into()), 10, 4);
    assert_eq!(collapsed(&d), [(7, 8)]);
    // 撤销时折叠块同样随之移动。
    d.execute(Command::Undo, 10, 4);
    d.execute(Command::Undo, 10, 4);
    assert_eq!(collapsed(&d), [(6, 7)]);
    assert_eq!(d.text_range(0, 1000).unwrap(), PY);

    // 在块第一行的行首退格，与上一行合并：仍保持折叠，块随之上移一行。
    let head = d.text().line_start(6);
    d.selection = Selection::point(head);
    d.execute(Command::Backspace { word: false }, 10, 4);
    assert_eq!(collapsed(&d), [(5, 6)]);
    d.execute(Command::Undo, 10, 4);
    assert_eq!(collapsed(&d), [(6, 7)]);

    // 在块第一行的行尾删除换行符：与块内的下一行合并，块展开，块内的行重新显示。
    d.selection = Selection::point(d.text().line_end(6));
    let rows = d.row_count();
    d.execute(Command::Delete { word: false }, 10, 4);
    assert!(d.folds().is_empty());
    assert_eq!(
        d.row_count(),
        d.text().line_count(),
        "没有折起来的块了，每行都显示"
    );
    assert!(d.row_count() >= rows);
    for line in 0..d.text().line_count() {
        assert!(!d.folds().is_hidden(line), "第 {line} 行还藏着");
    }
    d.execute(Command::Undo, 10, 4);

    // 修改折叠块内部的行（全部替换、格式化）：仍保持折叠，块的末尾随之移动。
    d.toggle_fold_at(1, 4);
    let inside = d.text().line_start(3);
    d.selection = Selection::new(inside, inside + 12);
    d.apply(winc_core::edit::Transaction::new(
        vec![winc_core::edit::Change {
            start: inside,
            removed: d.text().substring(inside, inside + 12),
            inserted: "            a()\n            b()".into(),
        }],
        d.selection,
        Selection::point(0),
        winc_core::edit::EditKind::Other,
    ));
    assert_eq!(collapsed(&d), [(1, 5)]);
}

#[test]
fn moving_the_caret_steps_over_folded_lines() {
    let mut d = doc(PY);
    d.toggle_fold_at(1, 4);
    // 向下移动一行，从 `def` 跳到 `def g` 之前的空行。
    put(&mut d, 1);
    d.execute(Command::MoveDown { extend: false }, 10, 4);
    assert_eq!(d.text().line_of(d.selection.head), 5);
    d.execute(Command::MoveUp { extend: false }, 10, 4);
    assert_eq!(d.text().line_of(d.selection.head), 1);
    // 在行尾向右移动：跳到折叠范围之后一行的行首；再向左移动则回到行尾。
    d.selection = Selection::point(d.text().line_end(1));
    d.execute(
        Command::MoveRight {
            word: false,
            extend: false,
        },
        10,
        4,
    );
    assert_eq!(d.selection, Selection::point(d.text().line_start(5)));
    d.execute(
        Command::MoveLeft {
            word: false,
            extend: false,
        },
        10,
        4,
    );
    assert_eq!(d.selection, Selection::point(d.text().line_end(1)));

    // 跳到折叠的位置时展开对应的块。
    assert!(d.reveal(d.text().line_start(3)));
    assert!(d.folds().is_empty());
}

#[test]
fn wrapping_maps_rows_with_folds_and_follows_edits() {
    // 宽 10 个显示列：第 1 行断成「aaaa bbbb 」「cccc dddd 」「eeee」三段，其他行不断。
    let mut d = doc("a\naaaa bbbb cccc dddd eeee\nb\nc\n");
    assert!(d.ensure_wrap(Some(10), 4));
    assert_eq!((d.rows_of(1), d.row_count()), (3, 7));
    assert_eq!((d.row_of(1), d.row_of(2)), (1, 4));
    assert_eq!(d.line_and_part(2), (1, 1));
    assert_eq!(d.line_and_part(4), (2, 0));
    // 段的起点属于其后的一段。
    let start = d.text().line_start(1);
    assert_eq!(d.row_at(start + 10), 2);
    assert_eq!(d.pos_in_row(2, 0, 4), start + 10);

    // 在被断开的行之前加一行：被断开的行随之下移。
    d.selection = Selection::point(0);
    d.execute(Command::Insert("x\n".into()), 10, 4);
    assert_eq!((d.rows_of(2), d.row_count()), (3, 8));
    // 输入只重算受影响的行，并同步记录版本：下一帧 `ensure_wrap` 不会整体重算。
    assert_eq!(d.folds().wrap().map(|w| w.revision), Some(d.revision));
    // 把被断开的行改短：该行不再断开。
    let line = d.text().line_start(2);
    d.selection = Selection::new(line + 4, d.text().line_end(2));
    d.execute(Command::Backspace { word: false }, 10, 4);
    assert_eq!((d.rows_of(2), d.row_count()), (1, 6));
    // 把一个短行改长：记录随之更新，按记录直接返回的断行与逐字符重新计算的结果相同。
    d.selection = Selection::point(d.text().line_end(3));
    d.execute(Command::Insert(" cccc dddd eeee".into()), 10, 4);
    for line in 0..d.text().line_count() {
        assert_eq!(
            d.line_breaks(line),
            line_breaks(d.text(), line, 10, 4, MAX_LINE_CHARS)
        );
        assert_eq!(d.line_breaks(line).rows(), d.rows_of(line));
    }
    assert!(d.rows_of(3) > 1);

    // 与折叠同时使用：被折叠的行不占显示行，被断开的行仍额外占用显示行。
    let mut d = doc("class A:\n    def f(self): return the_longest_value_here\n    x = 1\ny = 2\n");
    d.ensure_wrap(Some(20), 4);
    let (wrapped, inner) = (d.row_count(), d.rows_of(1));
    assert!(inner > 1);
    d.selection = Selection::point(0);
    d.run_fold(FoldCommand::Fold, 4);
    assert_eq!(d.row_count(), wrapped - inner - 1);
    assert_eq!(d.line_of_row(1), 3);
    // 关闭自动换行。
    assert!(!d.ensure_wrap(None, 4));
    assert_eq!(d.row_count(), 3);
}

#[test]
fn whole_document_wrapping_matches_each_line() {
    // 样本覆盖按字节数即可判定可容纳的行、含 Tab 的行、宽字符、CRLF 与空行：全文断行的结果
    // 与逐行计算相同。
    let lines = [
        "short",
        "\t\tab\tcd",
        "\t\t\t\t\t\t\t\t",
        "中文中文中文中文中文中文",
        "ééééééééééééééééééé",
        "😀😀😀😀😀😀😀😀😀😀",
        "",
        "a b c d e f g h i j k l m n o p q r s t",
        "0123456789012345",
    ];
    let text = lines.join("\r\n");
    for width in [1, 4, 8, 15, 16, 17, 40] {
        let mut d = doc(&text);
        d.ensure_wrap(Some(width), 4);
        for line in 0..d.text().line_count() {
            let rows = line_breaks(d.text(), line, width, 4, MAX_LINE_CHARS).rows();
            assert_eq!(d.rows_of(line), rows, "宽 {width}，第 {line} 行");
        }
    }
}

#[test]
fn measuring_first_agrees_with_breaking() {
    // 先测量是否可容纳，结果应与直接交给 `wrap::breaks` 断行相同：样本覆盖恰好占满、
    // 超出一列、Tab 与中文。
    let rope = ropey::Rope::from_str(
        "0123456789\n0123456789a\n\tif x:\t# 注释\n中文中文中文中文中文\nshort\na b c d e f g h i j\n",
    );
    for line in 0..rope.line_count() {
        let chars: Vec<char> = rope.line_content(line).slice().chars().collect();
        for width in [1, 8, 10, 11, 20, 40] {
            assert_eq!(
                line_breaks(&rope, line, width, 4, MAX_LINE_CHARS),
                wrap::breaks(&chars, width, 4),
                "第 {line} 行，宽 {width}"
            );
        }
    }
}

#[test]
fn appending_rewraps_only_the_new_lines() {
    // 追加到末尾（标准输入、日志）：追加的长行立即断开，结果与整体重算相同，下一帧也无需
    // 重新断开整个文件。
    for (first, more) in [
        ("a\n", "aaaa bbbb cccc\nb"),
        ("x\r", "\naaaa bbbb cccc dddd"),
    ] {
        let mut d = Document::output(DocumentId(1), "输出", first);
        assert!(d.ensure_wrap(Some(10), 4));
        d.append_output(more);
        assert_eq!(d.folds().wrap().map(|w| w.revision), Some(d.revision));

        let mut fresh = Document::from_text(DocumentId(2), &format!("{first}{more}"));
        fresh.ensure_wrap(Some(10), 4);
        assert_eq!(d.row_count(), fresh.row_count(), "{first:?}{more:?}");
        for line in 0..fresh.text().line_count() {
            assert_eq!(d.rows_of(line), fresh.rows_of(line), "第 {line} 行");
        }
    }
}

#[test]
fn edits_keep_wrapped_rows_in_step_with_a_full_rewrap() {
    // 每次修改后，各行占用的显示行数、显示行的换算都应与按修改后的文本整体重算相同。样本覆盖
    // 段数不变的输入、增加一段的输入、插入换行、删除换行，以及移除折叠块的修改。
    fn check(d: &Document) {
        let text = d
            .text()
            .text_range(0, d.text().end_pos(), u64::MAX)
            .unwrap();
        let mut fresh = Document::from_text(DocumentId(2), &text);
        fresh.ensure_wrap(Some(10), 4);
        assert_eq!(d.row_count(), fresh.row_count());
        for line in 0..fresh.text().line_count() {
            assert_eq!(d.rows_of(line), fresh.rows_of(line), "第 {line} 行");
            assert_eq!(d.row_of(line), fresh.row_of(line), "第 {line} 行");
        }
        for row in 0..fresh.row_count() {
            assert_eq!(
                d.line_and_part(row),
                fresh.line_and_part(row),
                "第 {row} 个显示行"
            );
        }
    }

    let mut d = doc("aaaa bbbb cccc\nx\nclass A:\n    y = 1\naaaa bbbb cccc dddd\n");
    d.ensure_wrap(Some(10), 4);
    d.selection = Selection::point(d.text().line_start(1));
    for command in [
        Command::Insert("y".into()),
        Command::Insert("yyyyyyyyyyyy".into()),
        Command::Newline,
        Command::Backspace { word: false },
    ] {
        d.execute(command, 10, 4);
        check(&d);
    }

    // 折叠 `class A:` 后删除该行行尾的换行符：块被移除，隐藏的行重新显示。
    put(&mut d, 2);
    assert!(d.run_fold(FoldCommand::Fold, 4));
    assert!(d.folds().is_hidden(3));
    let head = d.text().line_end(2);
    d.selection = Selection::new(head, d.text().line_start(3));
    d.execute(Command::Backspace { word: false }, 10, 4);
    assert!(d.folds().is_empty());
    check(&d);
}

#[test]
fn wrapped_lines_move_by_screen_row_and_home_end_by_segment() {
    let mut d = doc("aaaa bbbb cccc dddd\nxy\n");
    d.ensure_wrap(Some(10), 4);
    // 第 0 行：「aaaa bbbb 」「cccc dddd」。
    d.selection = Selection::point(2);
    d.execute(Command::MoveDown { extend: false }, 10, 4);
    assert_eq!(d.selection.head, 12);
    d.execute(Command::MoveDown { extend: false }, 10, 4);
    assert_eq!(d.selection.head, d.text().line_start(1) + 2);
    d.execute(Command::MoveUp { extend: false }, 10, 4);
    assert_eq!(d.selection.head, 12);
    // 行尾：第一次移到当前段末尾，第二次移到整行末尾（此处当前段即最后一段）。
    d.selection = Selection::point(2);
    d.execute(Command::LineEnd { extend: false }, 10, 4);
    assert_eq!(d.selection.head, 9);
    d.execute(Command::LineEnd { extend: false }, 10, 4);
    assert_eq!(d.selection.head, 19);
    // 行首：在第二段中第一次移到当前段开头，第二次移到整行开头。
    d.selection = Selection::point(13);
    d.execute(Command::LineStart { extend: false }, 10, 4);
    assert_eq!(d.selection.head, 10);
    d.execute(Command::LineStart { extend: false }, 10, 4);
    assert_eq!(d.selection.head, 0);
}

mod measure;
