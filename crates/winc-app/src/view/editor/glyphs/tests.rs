//! `glyphs` 的测试：空白符在各个选项下是否显示、行尾空白从哪一列开始。

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
