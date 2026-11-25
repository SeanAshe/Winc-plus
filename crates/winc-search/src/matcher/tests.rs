//! 匹配器的测试：哪些正则跨行查找、放宽后的正则、整块查找候选位置。

use super::*;

#[test]
fn newlines_and_non_word_classes_make_a_regex_multiline() {
    assert!(is_multiline("a\\nb", true) && is_multiline("a\\Wb", true));
    assert!(!is_multiline("a\\nb", false) && !is_multiline("a\\sb", true));
}

#[test]
fn relaxed_patterns_compile_and_never_panic() {
    // 放宽后的正则必须能被 `regex` 编译；无法放宽的写法返回 `None`。任何写法都不能 panic，
    // release 构建中 panic 会使进程中止。
    for (pattern, relaxable) in [
        (r"(?<=\$)\d+", true),
        (r"\b(\w+)\s+\1\b", true),
        (r"(?i)(?<!foo)Bar(?=baz|qux)", true),
        (r"(?>a+)b\Kc\G", true),
        (r"\<word\>\B", true),
        (r"^(?:x|(?<name>y)\k<name>)+$", true),
        (r"[^\n](?=\p{L})", true),
        (r"a{2,}?(?!b)", true),
        (r"(a)(?(1)b|c)", false),
        (r"(a)\g<1>", false),
        (r"a(*FAIL)|b", false),
    ] {
        let relaxed = relaxed(pattern);
        assert_eq!(relaxed.is_some(), relaxable, "{pattern}: {relaxed:?}");
        if let Some(relaxed) = relaxed {
            assert!(Regex::new(&relaxed).is_ok(), "{pattern} → {relaxed}");
        }
    }
    // 放宽只会多匹配：原正则能匹配的位置，放宽后的正则也匹配。
    let relaxed = Regex::new(&relaxed(r"(?<=\$)(\d+)\.\1").unwrap()).unwrap();
    assert!(relaxed.is_match("$12.12") && relaxed.is_match("12.13"));
}

/// 给出必含的文字时，整块查找只找这段文字，不运行查询本身的正则；不区分大小写时同样如此。
#[test]
fn block_search_looks_for_the_required_text() {
    let options = Options {
        case_sensitive: true,
        whole_word: false,
        regex: true,
    };
    let text = "fn other\nlet name\nfn name\n";
    let query = r"fn\s+name\b";

    let full = Matcher::new(query, options).unwrap();
    assert_eq!(full.block_find(text, 0), text.find("fn name"));

    let literal = Matcher::requiring(query, options, "name").unwrap();
    assert_eq!(literal.block_find(text, 0), text.find("name"));
    assert_eq!(literal.block_find("fn other\n", 0), None);

    let options = Options {
        case_sensitive: false,
        ..options
    };
    let literal = Matcher::requiring(query, options, "name").unwrap();
    assert_eq!(literal.block_find("let NAME\n", 0), Some(4));
}

#[test]
fn lookaround_falls_back_to_fancy_regex_with_a_relaxed_block() {
    // 不含环视的正则由 `regex` 编译；含环视的交给 `fancy-regex`，可放宽时也有整块预查的 `block`。
    let kind = |q: &str| {
        let options = Options {
            case_sensitive: false,
            whole_word: false,
            regex: true,
        };
        let matcher = Matcher::new(q, options).unwrap();
        (
            matches!(matcher.kind, Kind::Regex(Pattern::Fancy(_))),
            matcher.block.is_some(),
        )
    };
    assert_eq!(kind(r"\d+"), (false, true));
    // 用到环视的正则同样有整块预查，使用放宽后的版本；无法放宽的写法没有 `block`。
    assert_eq!(kind(r"(?<!\s)\d+"), (true, true));
    assert_eq!(kind(r"(a)(?(1)b|c)"), (true, false));
}
