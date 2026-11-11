use super::*;

/// 返回 `pattern` 转换成的正则。
fn regex(pattern: &str) -> String {
    translate(pattern, false, false).unwrap().regex
}

#[test]
fn magic_levels_decide_which_characters_are_special() {
    assert_eq!(regex("a.b*"), "a.b*");
    assert_eq!(regex(r"a\.b\*"), r"a\.b\*");
    assert_eq!(regex(r"\Ma.b*"), r"a\.b\*");
    assert_eq!(regex(r"\Ma\.b\*"), "a.b*");
    assert_eq!(regex(r"\v(a|b)+c?"), "(a|b)+c?");
    assert_eq!(regex(r"\(a\|b\)\+c\="), "(a|b)+c?");
    assert_eq!(regex("(a|b)+"), r"\(a\|b\)\+");
    assert_eq!(regex(r"\V^a.$"), r"\^a\.\$");
    assert_eq!(regex(r"\V\^a\$"), "^a$");
}

#[test]
fn anchors_are_special_only_at_the_edges() {
    assert_eq!(regex("^a$"), "^a$");
    assert_eq!(regex("a^b$c"), r"a\^b\$c");
    assert_eq!(regex(r"\(^a\|b$\)"), "(^a|b$)");
    assert_eq!(regex("*a"), r"\*a");
    assert_eq!(regex("^*a"), r"^\*a");
}

#[test]
fn classes_boundaries_and_counts_are_translated() {
    assert_eq!(regex(r"\<foo\>"), r"\b{start}foo\b{end}");
    assert_eq!(regex(r"\d\+\s"), r"[0-9]+[ \t]");
    assert_eq!(regex(r"\S"), r"[^ \t\n\r]");
    assert_eq!(regex(r"\_s"), r"[ \t\n]");
    assert_eq!(regex(r"a\{2,3}"), "a{2,3}");
    assert_eq!(regex(r"a\{-1,}"), "a{1,}?");
    assert_eq!(regex(r"a\{}"), "a*");
    assert_eq!(regex(r"\%(ab\)*"), "(?:ab)*");
    assert_eq!(regex("[a-c.]x[]]"), r"[a-c.]x[\]]");
    assert_eq!(regex("[^a]"), r"[^a\n\r]");
    assert_eq!(regex("a[b"), r"a\[b");
}

#[test]
fn lookarounds_and_match_bounds_use_groups() {
    assert_eq!(regex(r"foo\(bar\)\@="), "foo(?=(bar))");
    assert_eq!(regex(r"\(foo\)\@<!bar"), "(?<!(foo))bar");
    assert_eq!(regex(r"foo\zsbar\zebaz"), "(?<=foo)bar(?=baz)");
}

#[test]
fn case_follows_the_options_and_the_pattern() {
    let ignore = |pattern: &str, ignore: bool, smart: bool| {
        translate(pattern, ignore, smart).unwrap().ignore_case
    };
    assert!(!ignore("abc", false, false));
    assert!(ignore("abc", true, false));
    assert!(ignore("Abc", true, false));
    assert!(!ignore("Abc", true, true));
    assert!(ignore(r"\Sbc", true, true));
    assert!(ignore(r"Abc\c", false, false));
    assert!(!ignore(r"\Cabc", true, false));
}

#[test]
fn unsupported_items_report_their_position() {
    assert_eq!(translate(r"ab\%Vc", false, false), Err(2));
    assert_eq!(translate("a~", false, false), Err(1));
    assert_eq!(translate(r"\(a", false, false), Err(3));
    assert_eq!(translate(r"a\)", false, false), Err(1));
}
