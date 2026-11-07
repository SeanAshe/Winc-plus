//! `diff_view` 模块的测试：两侧各自排出的显示行、滚动位置与显示行的换算、长段折叠与展开、
//! 逐词标记、重新比较以后沿用状态。

use super::*;

fn view(old: &str, new: &str) -> DiffView {
    let language = winc_syntax::detect("a.rs");
    DiffView::new(
        DiffSide::new("old", old, None, language),
        DiffSide::new("new", new, None, language),
    )
}

fn numbered(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

fn line(line: usize, hunk: Option<usize>) -> SideRow {
    SideRow::Line {
        line,
        hunk,
        part: 0,
    }
}

#[test]
fn a_long_deletion_in_the_middle_is_scrolled_through_line_by_line() {
    // HEAD 第 30 到 80 行在工作区里删掉了：滚动位置给这段留 50 行，滚过去时左侧一行行走，
    // 右侧停在删除的地方不动。
    let old = numbered(100);
    let new: String = old
        .lines()
        .enumerate()
        .filter(|(i, _)| !(30..80).contains(i))
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    let mut v = view(&old, &new);
    v.set_collapse(false, 0);
    assert_eq!(v.row_count(), 50);
    assert_eq!(v.scroll_rows(), 100);
    assert_eq!(v.hunk_span(0), 30.0..80.0);

    // 一屏 20 行。
    let at = |side, pos| v.side_scroll(side, pos, 20.0);
    // 对齐点（视图中间）在删掉的那段里：左侧每次走一行，右侧不动。
    assert!((at(LEFT, 41.0) - at(LEFT, 40.0) - 1.0).abs() < 0.01);
    assert_eq!(at(RIGHT, 40.0), at(RIGHT, 41.0));
    assert!((at(RIGHT, 40.0) - 20.0).abs() < 0.01, "{}", at(RIGHT, 40.0));
    // 过了这段：两侧又一行对一行，右侧少 50 行。
    assert!((at(RIGHT, 90.0) - 40.0).abs() < 0.01);
    assert!((at(LEFT, 90.0) - 90.0).abs() < 0.01);
    // 换回来：右侧删除点在这段的开头，左侧的行原样。
    assert_eq!(v.pos_of(RIGHT, 30.0), 30.0);
    assert_eq!(v.pos_of(RIGHT, 40.0), 90.0);
    assert_eq!(v.pos_of(LEFT, 55.0), 55.0);
    // 屏幕第四行落在删掉的那段里，算这一段。
    assert_eq!(v.current_hunk(40.0), Some(0));
}

#[test]
fn a_long_deletion_at_the_end_can_still_be_scrolled_through() {
    // HEAD 末尾删掉了 80 行：右侧只有 20 行，滚动范围按左侧算，左侧能一直滚到末尾。
    let mut v = view(&numbered(100), &numbered(20));
    v.set_collapse(false, 0);
    assert_eq!(v.row_count(), 20);
    assert_eq!(v.scroll_rows(), 100);
    // 一屏 30 行、滚到底（末尾留四成）：左侧也停在末尾，最后一行在六成高的地方。
    let left = v.left_scroll(100.0 - 0.6 * 30.0, 30.0);
    assert!((left - 82.0).abs() < 0.01, "{left}");
    // 滚动一行，左侧也走一行（不按比例压缩成一次跳过去）。
    let a = v.left_scroll(40.0, 30.0);
    let b = v.left_scroll(41.0, 30.0);
    assert!((b - a - 1.0).abs() < 0.01, "{a} {b}");

    // 反过来：末尾新增的行在右侧，本来就能滚到，不多加。
    let v = view(&numbered(20), &numbered(100));
    assert_eq!(v.scroll_rows(), v.row_count());
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn each_side_lists_its_own_lines_without_padding() {
    let v = view("a\nb\nc\n", "a\nB\nX\nc\n");
    assert_eq!(
        v.rows[LEFT],
        [line(0, None), line(1, Some(0)), line(2, None)]
    );
    assert_eq!(
        v.rows[RIGHT],
        [
            line(0, None),
            line(1, Some(0)),
            line(2, Some(0)),
            line(3, None)
        ]
    );
    assert_eq!(v.hunk_rows, [vec![1..2], vec![1..3]]);
    assert_eq!(v.stats(), (2, 1));
    // 纯删除：右侧是插入位置上的空区间。
    let v = view("a\nb\nc\n", "a\nc\n");
    assert_eq!(v.hunk_rows, [vec![1..2], vec![1..1]]);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn long_unchanged_runs_fold_and_expand() {
    let old = numbered(40);
    let new = old.replace("line 20\n", "line twenty\n");
    let mut v = view(&old, &new);
    // 开头：折起 0..16，留 16..20；改动一行；结尾：留 21..25，折起 25..40。
    assert_eq!(
        v.rows[RIGHT][0],
        SideRow::Fold {
            gap: 0,
            lines: 0..16
        }
    );
    assert_eq!(v.row_count(), 1 + CONTEXT + 1 + CONTEXT + 1);
    assert_eq!(v.hunk_rows[RIGHT], [1 + CONTEXT..2 + CONTEXT]);

    v.expand(0);
    assert_eq!(v.rows[RIGHT][0], line(0, None));
    assert_eq!(v.hunk_rows[RIGHT], [20..21]);

    let row = v.set_collapse(false, 20);
    assert_eq!(v.row_count(), 40);
    assert_eq!(row, 20);
    assert!(!v.collapses());
}

#[test]
fn navigation_walks_hunks_in_order() {
    let old = numbered(30);
    let new = old
        .replace("line 2\n", "two\n")
        .replace("line 25\n", "twenty-five\n");
    let v = view(&old, &new);
    let first = v.next_hunk(0).unwrap();
    let second = v.next_hunk(first).unwrap();
    assert_eq!(v.next_hunk(second), None);
    assert_eq!(v.previous_hunk(second), Some(first));
    assert_eq!(v.previous_hunk(first), None);
    assert_eq!(v.hunk_at(second), Some(1));
}

#[test]
fn left_side_follows_the_right_around_hunks() {
    // 右侧在第 10 行插了 20 行。
    let old = numbered(60);
    let mut new = String::new();
    for (i, l) in old.lines().enumerate() {
        if i == 10 {
            new.push_str(&"added\n".repeat(20));
        }
        new.push_str(l);
        new.push('\n');
    }
    let mut v = view(&old, &new);
    v.set_collapse(false, 0);
    // 改动之前两侧一样；插入的那段滚过去时左侧停在插入点上；之后左侧少 20 行。
    assert_eq!(v.left_scroll(0.0, 10.0), 0.0);
    assert_eq!(v.left_scroll(2.0, 10.0), 2.0);
    assert_eq!(v.left_scroll(10.0, 10.0), 5.0);
    assert_eq!(v.left_scroll(40.0, 10.0), 20.0);
}

#[test]
fn pinned_lines_stay_visible_with_only_their_context() {
    let old = numbered(80);
    let mut v = view(&old, &old.replace("line 40\n", "forty\n"));
    // 改回去以后整个文件没有改动，不折叠；再改两处，中间那段很长。
    v.set_new_text(Rope::from_str(
        &old.replace("line 5\n", "five\n")
            .replace("line 75\n", "seventy-five\n"),
    ));
    v.pin_new(40..41);
    let row = v.row_of_new(40);
    assert_eq!(v.line_at(RIGHT, row), Some(40));
    // 钉住的那一行上下各露 CONTEXT 行，两边仍然折着。
    assert!(matches!(
        v.rows[RIGHT][row - CONTEXT - 1],
        SideRow::Fold { .. }
    ));
    assert!(matches!(
        v.rows[RIGHT][row + CONTEXT + 1],
        SideRow::Fold { .. }
    ));
    assert_eq!(v.line_at(LEFT, v.row_of(LEFT, 40)), Some(40));
}

#[test]
fn pinned_lines_follow_edits_above_them() {
    let old = numbered(80);
    let changed = old
        .replace("line 5\n", "five\n")
        .replace("line 75\n", "seventy-five\n");
    let mut v = view(&old, &changed);
    v.pin_new(40..41);
    // 在上面加两行：钉住的那一行跟着往下挪两行，仍然露在外面。
    let added = changed.replace("line 10\n", "line 10\nnew a\nnew b\n");
    v.set_new_text(Rope::from_str(&added));
    let row = v.row_of_new(42);
    assert_eq!(v.line_at(RIGHT, row), Some(42));
    assert!(matches!(
        v.rows[RIGHT][row + CONTEXT + 1],
        SideRow::Fold { .. }
    ));
}

#[test]
fn both_sides_start_at_the_top_when_the_file_starts_with_a_change() {
    // 开头一段左边 1 行、右边 5 行：右侧在开头时左侧也在开头。
    let old = numbered(60);
    let new = format!("a\nb\nc\nd\n{}", old.replacen("line 0\n", "zero\n", 1));
    let v = view(&old, &new);
    assert_eq!(v.left_scroll(0.0, 20.0), 0.0);
}

#[test]
fn identical_texts_have_no_hunks_and_no_folds() {
    let text = numbered(50);
    let v = view(&text, &text);
    assert!(v.is_identical());
    assert_eq!(v.row_count(), 50);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn inline_marks_split_per_line() {
    let v = view("let a = 1;\nlet b = 2;\n", "let a = 10;\nlet c = 2;\n");
    let marks = v.inline[0].as_ref().unwrap();
    assert_eq!(marks.old, [vec![8..9], vec![4..5]]);
    assert_eq!(marks.new, [vec![8..10], vec![4..5]]);
    // 纯新增没有逐词标记。
    let v = view("a\n", "a\nb\n");
    assert_eq!(v.inline, [None]);
}

#[test]
fn files_without_a_trailing_newline_line_up() {
    let v = view("a\nb", "a\nc");
    assert_eq!(v.row_count(), 2);
    assert_eq!(v.hunks.len(), 1);
}

#[test]
fn editing_the_right_side_rediffs_and_maps_lines() {
    let old = numbered(40);
    let mut v = view(&old, &old.replace("line 20\n", "line twenty\n"));
    assert_eq!(v.hunks.len(), 1);
    // 右侧第 2 行位于开头折起的一段里：先展开才看得见。
    assert!(matches!(
        v.rows[RIGHT][v.row_of_new(2)],
        SideRow::Fold { .. }
    ));
    v.reveal_new(2);
    assert_eq!(v.line_at(RIGHT, v.row_of_new(2)), Some(2));

    // 改回去：两侧相同。
    v.set_new_text(Rope::from_str(&old));
    assert!(v.is_identical());
    // 再改一处别的。
    v.set_new_text(Rope::from_str(&old.replace("line 3\n", "line three\n")));
    assert_eq!(v.hunks.len(), 1);
    assert_eq!(v.hunks[0].new, 3..4);
    assert_eq!(v.old_lines(0), "line 3\n");
}

#[test]
fn comparing_elsewhere_then_applying_matches_comparing_in_place() {
    let old = numbered(50);
    let new = old.replace("line 3\n", "three\n").replace("line 40\n", "");
    let mut a = view(&old, &old);
    let mut b = view(&old, &old);
    a.set_new_text(Rope::from_str(&new));
    let compared = Compared::new(&b.old.text, &Rope::from_str(&new));
    b.apply(Rope::from_str(&new), compared);
    assert_eq!(
        (a.hunks.clone(), a.keys.clone()),
        (b.hunks.clone(), b.keys.clone())
    );
    assert_eq!(a.rows, b.rows);
    // 只读一侧的选区：取出选中的文本。
    let range = (b.old.text.line_to_char(2), b.old.text.line_to_char(4));
    b.read_selection = Some(ReadSelection { side: LEFT, range });
    assert_eq!(b.read_selected().as_deref(), Some("line 2\nline 3\n"));
    let range = (b.new.text.line_to_char(0), b.new.text.line_to_char(1));
    b.read_selection = Some(ReadSelection { side: RIGHT, range });
    assert_eq!(b.read_selected(), Some(b.new.text.line(0).to_string()));
    // 光标在选区的终点；没有选中内容时也有光标。
    assert_eq!(b.read_caret(), Some((RIGHT, range.1)));
    b.read_selection = Some(ReadSelection {
        side: RIGHT,
        range: (range.0, range.0),
    });
    assert_eq!(b.read_selected(), None);
    assert_eq!(b.read_caret(), Some((RIGHT, range.0)));
}

#[test]
fn first_difference_counts_lines_across_chunks() {
    let a = Rope::from_str(&"汉字\n".repeat(2000));
    let mut b = a.clone();
    b.insert(a.line_to_char(1500) + 1, "x");
    assert_eq!(first_difference(&a, &b), 1500);
    assert_eq!(first_difference(&a, &a.clone()), a.len_lines() - 1);
}

mod measure {
    //! 重新比较耗时的测量：右侧每改一个字就重新比较一次。

    use super::*;

    /// 测量：右侧改一个字以后重新比较要多久。
    ///
    /// `cargo test -p winc-editor measure_rediff -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_rediff() {
        let language = winc_syntax::detect("a.rs");
        for n in [2_000, 10_000, 50_000] {
            let old: String = (0..n)
                .map(|i| format!("    let value_{i} = compute(value_{i}, \"text {i}\");\n"))
                .collect();
            let mut new = old.replace("value_7 ", "value_seven ");
            new = new.replace("value_1500 ", "value_x ");
            let mut v = DiffView::new(
                DiffSide::new("a", &old, None, language),
                DiffSide::new("b", &new, None, language),
            );
            let mut rope = Rope::from_str(&new);
            let start = std::time::Instant::now();
            for i in 0..20 {
                rope.insert(
                    rope.line_to_char(n / 2) + 4,
                    if i % 2 == 0 { "x" } else { "y" },
                );
                v.set_new_text(rope.clone());
            }
            eprintln!("{n} 行：{:?} / 次", start.elapsed() / 20);
        }
    }
}

#[test]
fn wrapping_gives_each_part_of_a_long_line_its_own_row() {
    let long = "word ".repeat(10);
    let text = format!("short\n{long}\nend\n");
    let mut view = view(&text, &format!("{text}added\n"));
    assert_eq!(view.rows[LEFT].len(), 3);
    assert!(view.set_wrap(Some((20, 4))));
    assert!(view.is_wrapping());
    assert!(!view.set_wrap(Some((20, 4))));

    // 没改的行两侧断法相同，行数相等。
    let parts = view.breaks(LEFT, 1).rows();
    assert!(parts > 1);
    assert_eq!(view.rows[LEFT].len(), 2 + parts);
    assert_eq!(view.rows[RIGHT].len(), 3 + parts);
    assert_eq!(
        view.rows[LEFT][2],
        SideRow::Line {
            line: 1,
            hunk: None,
            part: 1
        }
    );

    // 改动的显示行和滚动位置跟着多出来的段走。
    assert_eq!(view.hunk_rows[RIGHT][0], 2 + parts..3 + parts);
    assert_eq!(view.row_at(RIGHT, (2 + parts) as f32), (2 + parts) as f32);

    // 行号查到的是第一段，字符查到它所在的那一段。
    assert_eq!(view.row_of(LEFT, 1), 1);
    let second = view.breaks(LEFT, 1).start_of(1);
    assert_eq!(view.row_of_col(LEFT, 1, second), 2);
    assert_eq!(view.row_of_col(LEFT, 1, 0), 1);

    assert!(view.set_wrap(None));
    assert_eq!(view.rows[LEFT].len(), 3);
}

#[test]
fn a_cell_in_a_wrapped_row_maps_to_the_whole_line() {
    let long = format!("    {}", "word ".repeat(10));
    let mut view = view(&format!("{long}\n"), &format!("{long}\nmore\n"));
    view.set_wrap(Some((20, 4)));
    let breaks = view.breaks(LEFT, 0);
    assert_eq!(breaks.indent, 4);
    let start = breaks.start_of(1);

    // 第一段不变；第二段先去掉缩进，再加上这一段开头的列。
    assert_eq!(view.cell_in_row(LEFT, 0, 3), 3);
    assert_eq!(view.cell_in_row(LEFT, 1, 4), start);
    assert_eq!(view.cell_in_row(LEFT, 1, 6), start + 2);
    assert_eq!(view.cell_in_row(LEFT, 1, 0), start);
    // 超出这一段时不落到下一段。
    let next = breaks.start_of(2);
    assert!(view.cell_in_row(LEFT, 1, 1000) < next);

    // 不换行时不换算。
    view.set_wrap(None);
    assert_eq!(view.cell_in_row(LEFT, 0, 7), 7);
}

#[test]
fn the_top_row_maps_back_to_its_scroll_position() {
    // 一段删除让两侧不等长：开头半屏内、停在删除处、越过删除之后都能反算回来。
    let old = numbered(100);
    let new: String = old
        .lines()
        .enumerate()
        .filter(|(i, _)| !(30..80).contains(i))
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    let mut v = view(&old, &new);
    v.set_collapse(false, 0);
    for row in [0.0, 5.0, 19.5, 20.0, 30.0, 40.25] {
        let pos = v.pos_of_top(RIGHT, row, 20.0);
        assert!(
            (v.side_scroll(RIGHT, pos, 20.0) - row).abs() < 0.01,
            "{row}"
        );
    }
}

#[test]
fn folded_rows_are_not_wrapped_and_a_rebuilt_view_keeps_wrapping() {
    let long = "word ".repeat(10);
    let lines: String = (0..40).map(|i| format!("{long}{i}\n")).collect();
    let mut first = view(&lines, &format!("{lines}added\n"));
    first.set_wrap(Some((20, 4)));
    assert!(matches!(first.rows[LEFT][0], SideRow::Fold { .. }));

    let mut next = view(&lines, &format!("{lines}other\n"));
    next.inherit(&first);
    assert!(next.is_wrapping());
    assert_eq!(next.rows[LEFT].len(), first.rows[LEFT].len());
}

#[test]
fn the_unified_layout_lists_old_rows_before_new_rows() {
    let old = "same\nold 1\nold 2\nsame\ngone\nsame\n";
    let new = "same\nnew 1\nsame\nsame\nadded\n";
    let mut v = view(old, new);
    v.set_unified(true);
    assert!(v.is_unified());
    let rows: Vec<_> = (0..v.scroll_rows())
        .map(|i| v.unified_row(i).unwrap())
        .collect();
    assert_eq!(v.unified_row(rows.len()), None);
    // 每一行只出现一次，两侧各自按顺序排列；改动里左侧的行在右侧的行前面。
    for side in [LEFT, RIGHT] {
        let of_side: Vec<_> = rows.iter().filter(|r| r.0 == side).map(|r| r.1).collect();
        assert!(of_side.windows(2).all(|w| w[0] < w[1]), "{of_side:?}");
    }
    let first = &v.hunk_rows;
    let at = v.hunk_pos(0) as usize;
    let old_rows = first[LEFT][0].len();
    assert_eq!(rows[at], (LEFT, first[LEFT][0].start));
    assert_eq!(rows[at + old_rows], (RIGHT, first[RIGHT][0].start));
    // 没改的行算右侧，只显示一次：总行数是右侧的行数加上左侧在改动里的行数。
    let changed: usize = first[LEFT].iter().map(|r| r.len()).sum();
    assert_eq!(rows.len(), v.rows[RIGHT].len() + changed);

    // 滚动位置和显示行互相换算。
    let map = v.unified_map();
    for (index, &(side, row)) in rows.iter().enumerate() {
        assert_eq!(v.pos_of(side, row as f32), index as f32, "{side} {row}");
        assert_eq!(map.pos_of(side, row as f32), index as f32, "{side} {row}");
        assert_eq!(v.row_at(side, index as f32), row as f32, "{side} {row}");
        assert_eq!(v.row_under(side, 0.0, 10.0, index as f32), row);
    }

    // 切回并排，排布恢复。
    let side_by_side = view(old, new);
    v.set_unified(false);
    assert_eq!(v.scroll_rows(), side_by_side.scroll_rows());
    assert_eq!(v.rows, side_by_side.rows);
}

#[test]
fn a_unified_view_stays_unified_after_comparing_again() {
    let mut a = view("one\ntwo\n", "one\nTWO\n");
    a.set_unified(true);
    let mut b = view("one\ntwo\n", "one\ntwo!\n");
    b.inherit(&a);
    assert!(b.is_unified());
    assert_eq!(b.scroll_rows(), 3);
}

#[test]
fn marks_covering_every_visible_character_cover_the_line() {
    let chars: Vec<char> = "  let a = 1;  ".chars().collect();
    // 缩进和行尾空白不必标记，中间的空格可以分属多段标记。
    assert!(marks_cover_line(&chars, std::slice::from_ref(&(2..12))));
    assert!(marks_cover_line(&chars, &[2..5, 6..7, 8..9, 10..12]));
    assert!(!marks_cover_line(&chars, std::slice::from_ref(&(10..11))));
    assert!(!marks_cover_line(&chars, &[]));
    assert!(!marks_cover_line(
        &[' ', ' '],
        std::slice::from_ref(&(0..2))
    ));
}

#[test]
fn fold_rows_pair_the_same_gap_on_both_sides() {
    let old: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let new = old.replace("line 20\n", "line 20\nextra\nmore\n");
    let view = view(&old, &new);
    assert_eq!(view.fold_rows.len(), 2);
    let (left, right) = view.fold_rows[1];
    assert!(matches!(view.rows[LEFT][left], SideRow::Fold { .. }));
    assert!(matches!(view.rows[RIGHT][right], SideRow::Fold { .. }));
    // 第二段折叠行在新增的两行之后：右侧比左侧靠后两行。
    assert_eq!(right, left + 2);
}
