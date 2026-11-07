//! `width` 的测试：各类字符占用的格数，以及 emoji 字素簇的宽度。

use super::*;

#[test]
fn widths_follow_wcwidth() {
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('中'), 2);
    assert_eq!(char_width('😀'), 2);
    assert_eq!(char_width('\u{301}'), 0); // 组合重音，U+0301
    assert_eq!(char_width('\u{200d}'), 0); // ZWJ
    assert_eq!(char_width('\x07'), 0);
}

fn text_width(text: &str) -> usize {
    cells(text).map(|(_, width)| width).sum()
}

#[test]
fn emoji_sequences_take_two_cells() {
    for text in [
        "⚠\u{fe0f}", // VS16
        "❤\u{fe0f}",
        "👍",
        "👍\u{1f3fd}",  // 肤色修饰
        "🧑\u{200d}💻", // ZWJ
        "👨\u{200d}👩\u{200d}👧",
        "👩\u{1f3fb}\u{200d}🚀",
        "❤\u{fe0f}\u{200d}🔥", // VS16 之后接 ZWJ
    ] {
        assert_eq!(text_width(text), 2, "{text}");
    }
}

#[test]
fn only_emoji_clusters_are_merged() {
    // 没有 VS16 时仍是文字样式，占一格。
    assert_eq!(text_width("⚠"), 1);
    // 不能按 emoji 显示的字加 VS16 不变宽。
    assert_eq!(text_width("a\u{fe0f}"), 1);
    // 汉字之间的 ZWJ 不连接。
    assert_eq!(text_width("中\u{200d}文"), 4);
    // 普通文字后的肤色修饰符单独占格。
    assert_eq!(text_width("a\u{1f3fd}"), 3);
    // 组合重音照常不占格。
    assert_eq!(text_width("e\u{301}"), 1);
    // 两个独立的 emoji 各占两格。
    assert_eq!(text_width("👍👍"), 4);
}

#[test]
fn the_cells_of_a_cluster_put_the_width_on_its_first_character() {
    let parts: Vec<_> = cells("⚠\u{fe0f}x").collect();
    assert_eq!(parts, [('⚠', 2), ('\u{fe0f}', 0), ('x', 1)]);
    let parts: Vec<_> = cells("🧑\u{200d}💻").collect();
    assert_eq!(parts, [('🧑', 2), ('\u{200d}', 0), ('💻', 0)]);
}

#[test]
fn only_parts_of_emoji_clusters_count_as_tails() {
    assert!(is_emoji_tail('\u{fe0f}'));
    assert!(is_emoji_tail('\u{200d}'));
    assert!(is_emoji_tail('💻'));
    assert!(!is_emoji_tail('\u{301}'));
}
