//! emoji 序列的测试：切分、相邻字符是否相接、按 emoji 还是文字显示、默认按 emoji 显示的字符。

use super::*;

fn split(text: &str) -> Vec<String> {
    units(text)
        .map(|unit| match unit {
            Unit::Char(c) => c.to_string(),
            Unit::Sequence(text) => text.to_owned(),
        })
        .collect()
}

#[test]
fn sequences_are_kept_whole_and_other_text_is_split_by_character() {
    for whole in [
        "❤\u{fe0f}",
        "☺\u{fe0e}",
        "👍\u{1f3fd}",
        "👨\u{200d}👩\u{200d}👧",
        "👨\u{200d}❤\u{fe0f}\u{200d}👨",
        "🧑\u{200d}💻",
        "🏳\u{fe0f}\u{200d}🌈",
        "🇨🇳",
        "1\u{fe0f}\u{20e3}",
        "#\u{20e3}",
        "🏴\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
    ] {
        assert_eq!(split(whole), [whole], "{whole:?}");
        assert_eq!(sequence_len(whole.chars()), whole.chars().count());
        // 前后的文字不并入。
        let around = format!("a{whole}中");
        assert_eq!(split(&around), ["a", whole, "中"], "{whole:?}");
    }
    assert_eq!(split("ab 中😀⭐"), ["a", "b", " ", "中", "😀", "⭐"]);
    // 三个区域指示符：前两个是一面旗，第三个单独。
    assert_eq!(split("🇨🇳🇺"), ["🇨🇳", "🇺"]);
    // 连接符只连接 emoji：文字之后的连接符、末尾的连接符、其后不是 emoji 时都不并入。
    assert_eq!(split("क\u{200d}ष"), ["क", "\u{200d}", "ष"]);
    assert_eq!(split("👨\u{200d}"), ["👨\u{200d}"]);
    assert_eq!(split("👨\u{200d}a"), ["👨\u{200d}", "a"]);
    // 肤色修饰符只跟在 emoji 之后；键帽只跟在数字、`#`、`*` 之后。
    assert_eq!(split("a\u{1f3fd}"), ["a", "\u{1f3fd}"]);
    assert_eq!(split("a\u{20e3}"), ["a", "\u{20e3}"]);
    assert_eq!(sequence_len("".chars()), 0);
    assert_eq!(sequence_len("ab".chars()), 1);
}

#[test]
fn text_presentation_and_plain_ascii_are_not_emoji() {
    for unit in [
        "❤\u{fe0f}",
        "👍\u{1f3fd}",
        "🇨🇳",
        "1\u{fe0f}\u{20e3}",
        "#\u{20e3}",
    ] {
        assert!(presents_as_emoji(unit), "{unit:?}");
    }
    for unit in ["☺\u{fe0e}", "1\u{fe0f}", "a\u{fe0f}"] {
        assert!(!presents_as_emoji(unit), "{unit:?}");
    }
    assert!(ignorable(VS16) && ignorable(ZWJ) && ignorable('\u{e007f}'));
    assert!(!ignorable('\u{1f3fd}') && !ignorable(KEYCAP) && !ignorable('a'));
}

#[test]
fn membership_is_decided_from_the_two_characters_before() {
    let at = |text: &str, index: usize| {
        let chars: Vec<char> = text.chars().collect();
        let before = (
            index.checked_sub(2).map(|i| chars[i]),
            index.checked_sub(1).map(|i| chars[i]),
        );
        joined(before, chars[index])
    };
    // 序列中除第一个字符外都相接。
    for text in [
        "❤\u{fe0f}",
        "✌\u{1f3fd}",
        "👨\u{200d}👩\u{200d}👧",
        "👩\u{200d}❤\u{fe0f}\u{200d}👨",
        "1\u{fe0f}\u{20e3}",
        "🏴\u{e0067}\u{e007f}",
    ] {
        assert!(!at(text, 0), "{text:?}");
        for index in 1..text.chars().count() {
            assert!(at(text, index), "{text:?} {index}");
        }
    }
    // 文字之后的连接符、连接符之后的文字、前面不是 emoji 的连接符之后的 emoji 都不相接。
    assert!(!at("क\u{200d}ष", 1) && !at("क\u{200d}ष", 2));
    assert!(!at("👨\u{200d}a", 2) && !at("a\u{200d}😀", 2));
    assert!(!at("a\u{1f3fd}", 1) && !at("a\u{20e3}", 1) && !at(" \u{fe0f}", 1));
    assert!(!at("😀😀", 1) && !at("ab", 1));
}

#[test]
fn the_presentation_table_is_sorted_and_covers_common_emoji() {
    assert!(
        PRESENTATION
            .windows(2)
            .all(|w| w[0].0 <= w[0].1 && w[0].1 < w[1].0)
    );
    for c in [
        '😀',
        '⭐',
        '✅',
        '❌',
        '⌚',
        '⚡',
        '🚀',
        '🇨',
        '🫠',
        '\u{1f3fb}',
    ] {
        assert!(presentation(c), "{c}");
    }
    // 默认按文字显示的符号、文字和标点不算。
    for c in [
        '☺', '✔', '❤', '⚠', '©', '←', '★', 'a', '中', '#', '\u{fe0f}',
    ] {
        assert!(!presentation(c), "{c}");
    }
}
