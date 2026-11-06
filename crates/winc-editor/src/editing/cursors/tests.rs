//! 多光标与列选择的测试：各命令在每个选区上执行、选区随修改移动、光标撤销与重做。

use super::*;
use crate::DocumentId;

/// 创建内容为 `text` 的 Python 文档（4 空格缩进、`#` 注释），光标放在 `cursors` 各处，
/// 最后一个为主光标。
fn doc(text: &str, cursors: &[usize]) -> Document {
    let mut doc = Document::from_text(DocumentId(1), text);
    doc.set_language("a.py");
    let all = cursors.iter().map(|&p| Selection::point(p)).collect();
    doc.set_selections(all, cursors.len() - 1);
    doc
}

fn text(doc: &Document) -> String {
    doc.text_range(0, usize::MAX).unwrap()
}

fn run(doc: &mut Document, command: Command) {
    doc.execute(command, 10, 4);
}

fn heads(doc: &Document) -> Vec<usize> {
    doc.selections().0.iter().map(|s| s.head).collect()
}

#[test]
fn typing_and_deleting_at_every_cursor_is_one_undo_step() {
    let mut d = doc("ab\nab\nab", &[1, 4, 7]);
    for c in ["x", "y"] {
        run(&mut d, Command::Insert(c.into()));
    }
    assert_eq!(text(&d), "axyb\naxyb\naxyb");
    assert_eq!(heads(&d), [3, 8, 13]);
    // 主光标仍是最后添加的那个。
    assert_eq!(d.selection, Selection::point(13));

    run(&mut d, Command::Backspace { word: false });
    run(&mut d, Command::Delete { word: false });
    assert_eq!(text(&d), "ax\nax\nax");

    // 连续键入合成一个撤销单元；退格、删除各是一个。撤销后光标也回到各自的位置。
    run(&mut d, Command::Undo);
    run(&mut d, Command::Undo);
    assert_eq!(text(&d), "axyb\naxyb\naxyb");
    run(&mut d, Command::Undo);
    assert_eq!(text(&d), "ab\nab\nab");
    assert_eq!(heads(&d), [1, 4, 7]);
    assert!(!d.is_dirty());
    run(&mut d, Command::Redo);
    assert_eq!(text(&d), "axyb\naxyb\naxyb");
    assert_eq!(heads(&d), [3, 8, 13]);
}

#[test]
fn newline_and_tab_keep_each_cursors_indentation() {
    let mut d = doc("if a:\n    b = 1\n", &[5, 15]);
    run(&mut d, Command::Newline);
    assert_eq!(text(&d), "if a:\n    \n    b = 1\n    \n");
    run(&mut d, Command::Indent);
    assert_eq!(text(&d), "if a:\n        \n    b = 1\n        \n");

    // 退格先删掉同一位置附近的光标之间的内容，碰到一起的光标合并成一个。
    let mut d = doc("abc", &[1, 2]);
    run(&mut d, Command::Backspace { word: false });
    assert_eq!(text(&d), "c");
    assert_eq!(heads(&d), [0]);
    assert!(!d.has_cursors());
}

#[test]
fn backspace_removes_the_empty_pair_at_every_caret() {
    let mut d = doc("() []", &[1, 4]);
    run(&mut d, Command::Backspace { word: false });
    assert_eq!(text(&d), " ");
}

#[test]
fn auto_pairs_wrap_every_selection() {
    let mut d = doc("a b", &[0]);
    d.set_selections(vec![Selection::new(0, 1), Selection::new(2, 3)], 1);
    run(&mut d, Command::Insert("(".into()));
    assert_eq!(text(&d), "(a) (b)");
    // 每个选区都还选着原来的文字。
    assert_eq!(
        d.selections().0,
        vec![Selection::new(1, 2), Selection::new(5, 6)]
    );
}

#[test]
fn paste_spreads_lines_when_the_counts_match() {
    let mut d = doc("a\nb\nc", &[1, 3, 5]);
    run(&mut d, Command::Paste("1\n2\n3".into()));
    assert_eq!(text(&d), "a1\nb2\nc3");

    // 行数不同时，每个光标都粘贴整段。
    let mut d = doc("a\nb", &[1, 3]);
    run(&mut d, Command::Paste("xy".into()));
    assert_eq!(text(&d), "axy\nbxy");
    run(&mut d, Command::Undo);
    assert_eq!(text(&d), "a\nb");

    // 整行粘贴：每个光标所在行的上方各插入一行。
    let mut d = doc("a\nb", &[0, 2]);
    run(&mut d, Command::InsertLinesAbove("1\n2\n".into()));
    assert_eq!(text(&d), "1\na\n2\nb");
}

#[test]
fn movement_applies_to_every_cursor_and_merges_collisions() {
    let mut d = doc("one two\nthree four\nab", &[0, 8, 19]);
    let right = |word, extend| Command::MoveRight { word, extend };
    run(&mut d, right(true, false));
    assert_eq!(heads(&d), [4, 14, 21]);
    run(&mut d, Command::LineEnd { extend: true });
    assert_eq!(
        d.selections().0,
        vec![
            Selection::new(4, 7),
            Selection::new(14, 18),
            Selection::point(21)
        ]
    );
    run(&mut d, Command::LineStart { extend: false });
    assert_eq!(heads(&d), [0, 8, 19]);

    // 上下移动时各光标保持自己的显示列；移到同一位置的光标合并。
    let mut d = doc("abcdef\nab\nabcdef\nabcdef", &[5, 15]);
    run(&mut d, Command::MoveDown { extend: false });
    assert_eq!(heads(&d), [9, 22]);
    run(&mut d, Command::MoveUp { extend: false });
    assert_eq!(heads(&d), [5, 15]);
    run(&mut d, Command::DocEnd { extend: false });
    assert_eq!(heads(&d), [23]);
    assert!(!d.has_cursors());
}

#[test]
fn add_cursors_above_and_below_keep_the_column() {
    let mut d = doc("abcdef\nab\nabcdef\nabcdef", &[4]);
    assert!(d.add_cursor_vertically(true, 4));
    assert!(d.add_cursor_vertically(true, 4));
    // 经过较短的行时停在行尾，下一行又回到原来的列。
    assert_eq!(heads(&d), [4, 9, 14]);
    assert_eq!(d.selection, Selection::point(14));
    assert!(d.add_cursor_vertically(true, 4));
    assert!(!d.add_cursor_vertically(true, 4));
    assert!(!d.add_cursor_vertically(false, 4));
    run(&mut d, Command::Insert("|".into()));
    assert_eq!(text(&d), "abcd|ef\nab|\nabcd|ef\nabcd|ef");
}

#[test]
fn next_occurrence_selects_the_word_then_each_match() {
    let mut d = doc("foo bar foo foobar\nfoo", &[1]);
    assert_eq!(d.add_next_occurrence(), Ok(true));
    assert_eq!(d.selection, Selection::new(0, 3));
    assert_eq!(d.add_next_occurrence(), Ok(true));
    // 从光标处选中的词按全字匹配：跳过 `foobar`。
    assert_eq!(d.add_next_occurrence(), Ok(true));
    assert_eq!(d.selection, Selection::new(19, 22));
    assert_eq!(d.add_next_occurrence(), Ok(false));
    assert_eq!(d.selections().0.len(), 3);
    run(&mut d, Command::Insert("baz".into()));
    assert_eq!(text(&d), "baz bar baz foobar\nbaz");

    // 跳过当前这一处：去掉主选区，选中下一处。
    let mut d = doc("a1 a2 a1 a1", &[0]);
    d.selection = Selection::new(0, 2);
    assert_eq!(d.skip_to_next_occurrence(), Ok(true));
    assert_eq!(d.selections().0, vec![Selection::new(6, 8)]);
    assert_eq!(d.add_next_occurrence(), Ok(true));
    assert_eq!(d.skip_to_next_occurrence(), Ok(true));
    assert_eq!(
        d.selections().0,
        vec![Selection::new(0, 2), Selection::new(6, 8)]
    );

    // 手动选中的文字不限全字，到末尾后从头继续查找。
    let mut d = doc("ab xab ab", &[0]);
    d.selection = Selection::new(7, 9);
    assert_eq!(d.add_next_occurrence(), Ok(true));
    assert_eq!(d.selection, Selection::new(0, 2));
    assert_eq!(d.add_next_occurrence(), Ok(true));
    assert_eq!(d.selection, Selection::new(4, 6));
}

#[test]
fn select_all_occurrences_keeps_the_one_at_the_cursor_primary() {
    let mut d = doc("let x = x + xx;", &[8]);
    assert_eq!(d.select_all_occurrences(), Ok(2));
    assert_eq!(
        d.selections().0,
        vec![Selection::new(4, 5), Selection::new(8, 9)]
    );
    assert_eq!(d.selection, Selection::new(8, 9));
    // 不在词上时什么也不选。
    let mut d = doc("a  b", &[2]);
    assert_eq!(d.select_all_occurrences(), Ok(0));
}

#[test]
fn column_selection_skips_short_lines_and_types_on_each_line() {
    let mut d = doc("abcd\na\n\nabcd", &[0]);
    d.select_columns((0, 1), (3, 3), 4);
    // 「a」正好够到框的左边，放一个光标；空行比框的左边短，跳过。
    assert_eq!(
        d.selections().0,
        vec![
            Selection::new(1, 3),
            Selection::point(6),
            Selection::new(9, 11)
        ]
    );
    assert_eq!(d.selection, Selection::new(9, 11));
    run(&mut d, Command::Insert("-".into()));
    assert_eq!(text(&d), "a-d\na-\n\na-d");

    // 宽度为 0 的框：每行一个光标，向上框选时主光标在上面一行。
    let mut d = doc("ab\nab\nab", &[0]);
    d.select_columns((2, 1), (0, 1), 4);
    assert_eq!(heads(&d), [1, 4, 7]);
    assert_eq!(d.selection, Selection::point(1));
}

#[test]
fn line_commands_run_once_per_group_of_lines() {
    // 同一行上的两个光标只注释一次，相邻的行一起处理。
    let mut d = doc("a = 1\nb = 2\nc = 3", &[0, 2, 8]);
    run(&mut d, Command::ToggleComment);
    assert_eq!(text(&d), "# a = 1\n# b = 2\nc = 3");
    assert_eq!(heads(&d), [2, 4, 12]);
    run(&mut d, Command::Undo);
    assert_eq!(text(&d), "a = 1\nb = 2\nc = 3");

    // 隔开的行各移各的，同一组里的光标跟着整块移动。
    let mut d = doc("1\n2\n3\n4\n5", &[0, 4]);
    run(&mut d, Command::MoveLines { down: true });
    assert_eq!(text(&d), "2\n1\n4\n3\n5");
    assert_eq!(heads(&d), [2, 6]);
    run(&mut d, Command::MoveLines { down: false });
    assert_eq!(text(&d), "1\n2\n3\n4\n5");
    assert_eq!(heads(&d), [0, 4]);

    let mut d = doc("1\n2\n3", &[0, 2]);
    run(&mut d, Command::MoveLines { down: true });
    assert_eq!(text(&d), "3\n1\n2");
    assert_eq!(heads(&d), [2, 4]);
    run(&mut d, Command::DeleteLine);
    assert_eq!(text(&d), "3");
    assert!(!d.has_cursors());
}

#[test]
fn alt_click_adds_and_removes_cursors() {
    let mut d = doc("abcdef", &[1]);
    d.toggle_cursor(4);
    assert_eq!(heads(&d), [1, 4]);
    assert_eq!(d.selection, Selection::point(4));
    // 点在已有的光标上：移除它；主光标被移除时由前一个接任。
    d.toggle_cursor(4);
    assert_eq!(heads(&d), [1]);
    assert_eq!(d.selection, Selection::point(1));
    // 只剩一个时不再移除。
    d.toggle_cursor(1);
    assert_eq!(heads(&d), [1]);

    d.toggle_cursor(5);
    assert!(d.collapse_selections());
    assert_eq!(heads(&d), [5]);
    assert!(!d.collapse_selections());
}

#[test]
fn cursors_go_to_the_end_of_every_selected_line() {
    let mut d = doc("ab\ncd\nef\ngh", &[0]);
    // 选区结束在第 2 行中间：前两行放在行尾，最后一行放在选区末端。
    d.selection = Selection::new(1, 7);
    assert!(d.add_cursors_to_line_ends());
    assert_eq!(heads(&d), [2, 5, 7]);
    assert_eq!(d.selection, Selection::point(7));
    run(&mut d, Command::Insert(";".into()));
    assert_eq!(text(&d), "ab;\ncd;\ne;f\ngh");

    // 结束在行首的那一行不放；都是单行光标时什么也不变。
    let mut d = doc("ab\ncd\nef", &[0]);
    d.selection = Selection::new(0, 6);
    assert!(d.add_cursors_to_line_ends());
    assert_eq!(heads(&d), [2, 5]);
    assert!(!d.add_cursors_to_line_ends());
}

#[test]
fn cursors_are_capped_at_the_limit() {
    // 两万行全选后 `⇧⌥I`：只保留 [`MAX_CURSORS`] 个，记下舍弃过，取出后清除。
    let content = "x\n".repeat(20_000);
    let mut d = doc(&content, &[0]);
    d.selection = Selection::new(0, content.chars().count());
    assert!(d.add_cursors_to_line_ends());
    assert_eq!(d.selections().0.len(), MAX_CURSORS);
    assert!(d.take_cursors_capped() && !d.take_cursors_capped());

    // 列选择拖过两万行：同样只保留上限个，主选区（`head` 所在的行）仍在其中。
    d.select_columns((0, 0), (19_999, 1), 4);
    assert_eq!(d.selections().0.len(), MAX_CURSORS);
    assert_eq!(d.text().pos_to_line_col(d.selection.head).0, 19_999);
    assert!(d.take_cursors_capped());
}

#[test]
fn cursor_undo_steps_back_through_selection_changes() {
    let mut d = doc("abc\nabc\nabc", &[1]);
    d.add_cursor_vertically(true, 4);
    d.add_cursor_vertically(true, 4);
    run(
        &mut d,
        Command::MoveRight {
            word: false,
            extend: false,
        },
    );
    assert_eq!(heads(&d), [2, 6, 10]);

    assert!(d.undo_cursors());
    assert_eq!(heads(&d), [1, 5, 9]);
    assert!(d.undo_cursors());
    assert_eq!(heads(&d), [1, 5]);
    assert!(d.redo_cursors());
    assert_eq!(heads(&d), [1, 5, 9]);
    assert!(d.undo_cursors());
    assert!(d.undo_cursors());
    assert_eq!(heads(&d), [1]);
    assert!(!d.undo_cursors());

    // 改了文字以后，改之前的选区不再退回。
    d.add_cursor_vertically(true, 4);
    run(&mut d, Command::Insert("x".into()));
    assert!(!d.undo_cursors());
    assert_eq!(heads(&d), [2, 7]);
}

#[test]
fn keyboard_column_selection_grows_from_the_cursor() {
    let mut d = doc("abcd\nab\nabcd", &[1]);
    d.extend_columns(0, 1, 4);
    d.extend_columns(0, 1, 4);
    d.extend_columns(1, 0, 4);
    d.extend_columns(1, 0, 4);
    assert_eq!(
        d.selections().0,
        vec![
            Selection::new(1, 3),
            Selection::new(6, 7),
            Selection::new(9, 11)
        ]
    );
    // 往回缩，再往上：终点跟着走，起点不动。
    d.extend_columns(-1, -1, 4);
    assert_eq!(
        d.selections().0,
        vec![Selection::new(1, 2), Selection::new(6, 7)]
    );
    // 动过别的选区之后，从主光标重新开始框选。
    run(&mut d, Command::DocEnd { extend: false });
    d.extend_columns(0, -1, 4);
    assert_eq!(heads(&d), [7, 12]);

    // 行尾之后按显示列移动：短行停在行尾，长行继续。
    let mut d = doc("ab\nabcdef", &[2]);
    d.extend_columns(0, 1, 4);
    d.extend_columns(1, 0, 4);
    d.extend_columns(1, 0, 4);
    assert_eq!(
        d.selections().0,
        vec![Selection::point(2), Selection::new(5, 7)]
    );
}

#[test]
fn positions_match_mapping_every_selection_through_every_change() {
    // 伪随机：每个选区上各一两处修改，可能删到、插到相邻选区的边上；覆盖从前往后与
    // 从后往前两种处理顺序，结果与逐个映射全部选区的结果比较。
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |bound: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % bound.max(1) as u64) as usize
    };
    for round in 0..200 {
        let forward = round % 4 == 0;
        let mut all = Vec::new();
        let mut pos = 0;
        for _ in 0..1 + next(12) {
            pos += 1 + next(4);
            let len = next(3);
            all.push(if next(2) == 0 {
                Selection::new(pos, pos + len)
            } else {
                Selection::new(pos + len, pos)
            });
            pos += len;
        }
        let mut expected = all.clone();
        let mut positions = Positions::new(all.clone(), forward);
        let order: Vec<usize> = if forward {
            (0..expected.len()).collect()
        } else {
            (0..expected.len()).rev().collect()
        };
        for i in order {
            let lo = if i > 0 { expected[i - 1].to() } else { 0 };
            let hi = match expected.get(i + 1) {
                Some(s) => s.from().max(lo),
                None => expected[i].to() + 3,
            };
            let mut changes = Vec::new();
            for _ in 0..1 + next(2) {
                let start = lo + next(hi - lo + 1);
                let removed = next(hi.saturating_sub(start) + 1);
                changes.push(Change {
                    start,
                    removed: "r".repeat(removed),
                    inserted: "i".repeat(next(4)),
                });
            }
            for s in &mut expected {
                *s = shift_selection(*s, &changes);
            }
            let transaction = Transaction::new(changes, all[i], expected[i], EditKind::Other);
            positions.apply(i..i + 1, &Command::Newline, &transaction);
        }
        assert_eq!(positions.finish(), expected, "round {round}");
    }
}

#[test]
fn alt_drag_reshapes_only_the_new_selection() {
    let mut d = doc("abc def ghi", &[0]);
    d.selection = Selection::new(0, 3);
    d.toggle_cursor(4);
    d.set_primary(Selection::new(4, 7));
    assert_eq!(
        d.selections().0,
        vec![Selection::new(0, 3), Selection::new(4, 7)]
    );
    // 拖过别的选区时合并。
    d.set_primary(Selection::new(4, 1));
    assert_eq!(d.selections().0, vec![Selection::new(4, 0)]);
}

#[test]
fn assigning_the_selection_drops_the_other_cursors() {
    let mut d = doc("abc\nabc", &[0, 4]);
    assert!(d.has_cursors());
    // 跳转、查找等代码直接给主选区赋值：其余光标随之作废。
    d.selection = Selection::point(2);
    assert!(!d.has_cursors());

    // 不带其余选区的事务改动文本后，其余光标同样作废。
    let mut d = doc("abc\nabc", &[0, 4]);
    let t = winc_core::edit::replace_selection(&d.text, Selection::point(4), "x", EditKind::Other);
    d.apply(t);
    assert!(!d.has_cursors());
}

#[test]
fn byte_matches_are_found_across_rope_chunks() {
    // 足够长，ropey 会分成很多块；有多字节字符，块的边界落在各种位置。
    let unit = "ab中文aab_x\n";
    let text = unit.repeat(2000);
    let rope = ropey::Rope::from_str(&text);
    assert!(rope.chunks().count() > 10);
    for needle in ["a", "ab", "中文a", "aab_x\nab", &unit.repeat(3)] {
        for from in [0, 1, 777, text.len() / 2] {
            let want: Vec<usize> =
                memchr::memmem::find_iter(&text.as_bytes()[from..], needle.as_bytes())
                    .map(|at| from + at)
                    .collect();
            let mut got = Vec::new();
            super::each_byte_match(&rope, needle.as_bytes(), from, |at| {
                got.push(at);
                true
            });
            assert_eq!(got, want, "{needle:?} from {from}");
        }
    }
}

/// 回归：状态栏每帧统计其余选区的长度；设置选区时算好，结果与逐个相加相同。
#[test]
fn extra_selected_chars_are_counted_when_set() {
    let mut d = doc("abcdef ghij klmn", &[0]);
    d.set_selections(
        vec![
            Selection::new(0, 3),
            Selection::new(7, 9),
            Selection::new(12, 16),
        ],
        0,
    );
    let sum: usize = d.extra_selections().iter().map(|s| s.to() - s.from()).sum();
    assert_eq!(d.extra_selected_chars(), sum);
    assert_eq!(sum, 2 + 4);
    // 其余选区作废（只剩主选区）时为 0。
    assert!(d.collapse_selections());
    assert_eq!(d.extra_selected_chars(), 0);
}
