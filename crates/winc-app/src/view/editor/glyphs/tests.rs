//! `glyphs` 的测试：空白符在各个选项下是否显示、行尾空白从哪一列开始，以及连字整形的分段。

use super::*;

#[test]
fn whitespace_follows_the_chosen_mode() {
    use RenderWhitespace::{All, None, Selection, Trailing};
    let chars: Vec<char> = "a b \t ".chars().collect();
    let trailing = trailing_whitespace(&chars);
    assert_eq!(trailing, 3);

    let shown = |mode, col, selected| whitespace_shown(mode, col, trailing, || selected);
    // 单词之间的空格在第 1 列，行尾空白从第 3 列开始。
    assert!(!shown(None, 1, true));
    assert!(shown(All, 1, false));
    assert!(!shown(Trailing, 1, true));
    assert!(shown(Trailing, 3, false));
    assert!(shown(Selection, 1, true));
    assert!(!shown(Selection, 1, false));
}

#[test]
fn lines_without_trailing_whitespace_end_at_their_length() {
    assert_eq!(trailing_whitespace(&['a', ' ', 'b']), 3);
    assert_eq!(trailing_whitespace(&[]), 0);
    // 全是空白的行整行都算行尾。
    assert_eq!(trailing_whitespace(&[' ', '\t']), 0);
}

/// 连字只整形相邻的 ASCII 标点：至少两个，遇到字母、空格或非 ASCII 字符停止，不越过记号的边界，
/// 一段最多 `LIGATURE_RUN` 个字符。
#[test]
fn ligature_runs_cover_adjacent_punctuation() {
    let chars: Vec<char> = "a === b->c ，=".chars().collect();
    let run = |col: usize, limit: usize| ligature_run(&chars, col, limit);
    assert_eq!(run(0, chars.len()), None);
    assert_eq!(run(2, chars.len()), Some(5));
    assert_eq!(run(7, chars.len()), Some(9));
    assert_eq!(run(2, 4), Some(4), "记号在第 4 列结束");
    assert_eq!(run(2, 3), None, "记号里只剩一个字符");
    assert_eq!(run(11, chars.len()), None, "全角逗号不是 ASCII 标点");
    let long: Vec<char> = "-".repeat(40).chars().collect();
    assert_eq!(ligature_run(&long, 0, long.len()), Some(LIGATURE_RUN));
}
