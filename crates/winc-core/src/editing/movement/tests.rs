//! `movement` 模块的测试：字素边界（含 emoji 簇与长行）、按词移动与选词、上下移动保持目标显示列、
//! 括号配对与智能行首。

use ropey::Rope;

use super::*;

#[test]
fn graphemes_cross_lines_and_clusters() {
    // `👍🏽` 是由 2 个字符组成的字素簇；`\r\n` 占一个位置，跨行时跳到行首或行末。
    let text = Rope::from_str("a👍🏽b\r\nc");
    assert_eq!(next_grapheme(&text, 0), 1);
    assert_eq!(next_grapheme(&text, 1), 3);
    assert_eq!(prev_grapheme(&text, 3), 1);
    assert_eq!(next_grapheme(&text, 4), 6);
    assert_eq!(prev_grapheme(&text, 6), 4);
}

#[test]
fn graphemes_on_long_line() {
    // 长行超过 `GRAPHEME_WINDOW` 个字符：只读出窗口内的字符，邻近位置仍逐字前进。
    let line = "x".repeat(10_000);
    let text = Rope::from_str(&line);
    assert_eq!(next_grapheme(&text, 5_000), 5_001);
    assert_eq!(prev_grapheme(&text, 5_000), 4_999);
    assert_eq!(next_grapheme(&text, 10_000), 10_000);
}

#[test]
fn words() {
    let text = Rope::from_str("let foo_bar = baz(1);");
    assert_eq!(next_word(&text, 0), 4);
    assert_eq!(next_word(&text, 4), 12);
    assert_eq!(next_word(&text, 12), 14);
    assert_eq!(prev_word(&text, 12), 4);
    assert_eq!(prev_word(&text, 4), 0);
    assert_eq!(prev_word(&text, 17), 14);
    assert_eq!(word_at(&text, 6), (4, 11));
    assert_eq!(word_at(&text, 11), (4, 11));
    assert_eq!(word_at(&text, 21), (19, 21));
}

#[test]
fn vertical_keeps_goal_cell() {
    let text = Rope::from_str("abcdef\n中文\nabcdef");
    let (pos, goal) = vertical(&text, 3, 1, None, 4);
    assert_eq!(goal, 3);
    // 目标显示列 3 落在「文」内部（该字符占显示列 2 至 3），取其左侧的列 1。
    assert_eq!(text.pos_to_line_col(pos), (1, 1));
    let (pos, _) = vertical(&text, pos, 1, Some(goal), 4);
    assert_eq!(text.pos_to_line_col(pos), (2, 3));
    assert_eq!(vertical(&text, pos, 5, Some(goal), 4).0, text.len_chars());
}

#[test]
fn vertical_moves_never_stop_inside_a_grapheme_cluster() {
    // `é` 写成字母加组合符号时由两个字符组成一个字素；目标显示列换算出的位置落在两个字符
    // 之间时回到簇边界。
    let text = Rope::from_str("abcd\nae\u{301}b");
    assert_eq!(snap_to_grapheme(&text, 7), 6);
    assert_eq!(snap_to_grapheme(&text, 8), 8);
    assert_eq!(snap_to_grapheme(&text, 5), 5);
    // emoji 序列占 2 个显示列，其中的修饰符、选择符、连接符及其后的字符不占列：目标显示列 3
    // 落在序列之后的 `b` 上。
    for unit in ["👍\u{1f3fd}", "❤\u{fe0f}", "👨\u{200d}👩"] {
        let text = Rope::from_str(&format!("abcd\na{unit}b"));
        let (pos, _) = vertical(&text, 3, 1, None, 4);
        assert_eq!(
            text.pos_to_line_col(pos),
            (1, 1 + unit.chars().count()),
            "{unit:?}"
        );
    }
}

#[test]
fn brackets_match_across_lines() {
    let text = Rope::from_str("fn f() {\n    g(a[1]);\n}\n");
    assert_eq!(matching_bracket(&text, 7, 100), Some((7, 22)));
    assert_eq!(matching_bracket(&text, 23, 100), Some((7, 22)));
    assert_eq!(matching_bracket(&text, 15, 100), Some((14, 19)));
    assert_eq!(matching_bracket(&text, 18, 100), Some((16, 18)));
    assert_eq!(matching_bracket(&text, 7, 5), None);
    assert_eq!(matching_bracket(&text, 1, 100), None);
}

#[test]
fn smart_home_toggles() {
    let text = Rope::from_str("    code");
    assert_eq!(smart_home(&text, 6), 4);
    assert_eq!(smart_home(&text, 4), 0);
    assert_eq!(smart_home(&text, 0), 4);
}

/// 回归：括号查找的预算曾不计空行，空行很多时一次查找会读完整个文件的所有行。
#[test]
fn bracket_scans_count_empty_lines() {
    let text = Rope::from_str(&format!("({})", "\n".repeat(100)));
    assert_eq!(matching_bracket(&text, 0, 50), None);
    let end = text.len_chars() - 1;
    assert_eq!(matching_bracket(&text, end + 1, 50), None);
    assert_eq!(matching_bracket(&text, 0, 300), Some((0, end)));
}
