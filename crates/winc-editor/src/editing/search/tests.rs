//! 文件内查找与替换的测试：大小写折叠、字符偏移、跨行匹配、整词、回退到 `fancy-regex`，
//! 以及文档改动与结果达到上限之后继续搜索。

use super::*;
use crate::Command;

fn query(text: &str) -> Search {
    Search {
        query: text.into(),
        ..Default::default()
    }
}

#[test]
fn case_insensitive_and_wraps() {
    let doc = Document::from_text(DocumentId(1), "Foo foo\nFOO");
    let mut search = query("foo");
    search.update(&doc);
    assert!(search.is_complete());
    assert_eq!(search.matches(), &[(0, 3), (4, 7), (8, 11)]);
    assert_eq!(search.lines(), &[0, 1]);
    assert_eq!(search.next_from(5, false), Some((8, 11)));
    assert_eq!(search.next_from(9, false), Some((0, 3)));
    assert_eq!(search.next_from(4, true), Some((0, 3)));
    assert_eq!(search.next_from(0, true), Some((8, 11)));

    search.case_sensitive = true;
    search.update(&doc);
    assert_eq!(search.matches(), &[(4, 7)]);
}

#[test]
fn char_offsets_with_multibyte_text() {
    let doc = Document::from_text(DocumentId(1), "中文 abc 中文\n文");
    let mut search = query("文");
    search.update(&doc);
    assert_eq!(search.matches(), &[(1, 2), (8, 9), (10, 11)]);
    assert_eq!(search.matches_in(2, 10), &[(8, 9)]);
    assert_eq!(search.matches_in(0, 100).len(), 3);
    assert!(search.matches_in(3, 8).is_empty());

    let doc = Document::from_text(DocumentId(1), "ÄRGER ärger");
    let mut search = query("Ärger");
    search.update(&doc);
    assert_eq!(search.matches(), &[(0, 5), (6, 11)]);
}

#[test]
fn no_early_wrap_while_incomplete() {
    let mut search = query("x");
    search.matches = vec![(10, 11), (50, 51)];
    search.resume = Some(Resume { line: 3, pos: 60 });

    assert_eq!(search.next_from(20, false), Some((50, 51)));
    assert_eq!(search.next_from(55, false), None);
    assert_eq!(search.next_from(55, true), Some((50, 51)));
    // 断点是 60，早于 100：100 之前可能还有更近的匹配，不能绕回开头。
    assert_eq!(search.next_from(100, true), None);

    search.resume = None;
    assert_eq!(search.next_from(55, false), Some((10, 11)));
    assert_eq!(search.next_from(5, true), Some((50, 51)));
}

#[test]
fn plain_text_folds_case_like_regex() {
    // 不区分大小写时，纯文本与正则按相同的 Unicode 简单大小写折叠：Σ、σ、ς 视为同一字母，
    // 开尔文符号视为 k，长 s（ſ）视为 s；İ、ß 没有一对一的折叠，因此不视为 i、ss。
    let text = "Σσς Kk K ſsS İi ß ss 中文 FOO";
    let doc = Document::from_text(DocumentId(1), text);
    for q in ["σ", "k", "s", "i", "ß", "中文 foo", "FOO", "a.b"] {
        let mut plain = query(q);
        plain.update(&doc);
        let mut regex = query(&regex::escape(q));
        regex.regex = true;
        regex.update(&doc);
        assert_eq!(plain.matches(), regex.matches(), "{q}");
    }

    let count = |q: &str| {
        let mut search = query(q);
        search.update(&doc);
        search.matches().len()
    };
    assert_eq!(count("σ"), 3);
    assert_eq!(count("k"), 3);
    assert_eq!(count("s"), 5);
    assert_eq!(count("i"), 1);
    assert_eq!(count("中文 foo"), 1);
    // 区分大小写的纯文本按字节直接查找，`FOO` 只与原文的大写形式匹配。
    let mut exact = query("FOO");
    exact.case_sensitive = true;
    exact.update(&doc);
    assert_eq!(exact.matches().len(), 1);
}

#[test]
fn newlines_in_a_regex_match_across_lines() {
    // 正则里含 `\n`、`\r`、`\W` 时跨行查找，`\n` 能匹配任意一种换行符。
    assert!(is_multiline("a\\nb", true) && is_multiline("a\\Wb", true));
    assert!(!is_multiline("a\\nb", false) && !is_multiline("a\\sb", true));

    let doc = Document::from_text(DocumentId(1), "a\r\nb\nc\rd\nx");
    let mut search = query("a\\nb|b\\nc|c\\nd");
    search.regex = true;
    search.update(&doc);
    assert!(search.is_complete());
    assert_eq!(search.matches(), &[(0, 4), (5, 8)]);
    assert_eq!(search.lines(), &[0, 1, 2, 3]);

    // `^`、`$` 仍按行首、行尾匹配；`\W` 也能跨过换行；终点落在换行之后的匹配不计入下一行。
    let mut search = query("^d$|c\\Wd|x\\n?");
    search.regex = true;
    search.update(&doc);
    assert_eq!(search.matches(), &[(5, 8), (9, 10)]);
    let mut search = query("b\\n");
    search.regex = true;
    search.update(&doc);
    assert_eq!(search.matches(), &[(3, 5)]);
    assert_eq!(search.lines(), &[1]);

    // 替换时 `\n` 换成文档的换行符，分组引用同样展开。
    let mut doc = Document::from_text(DocumentId(1), "key\r\nvalue\r\nkey\r\nother");
    doc.line_ending = winc_core::LineEnding::CrLf;
    let mut search = query("(key)\\n(value)");
    search.regex = true;
    search.replacement = "$2\\n$1".into();
    assert_eq!(search.replace_all(&mut doc), Ok(1));
    assert_eq!(
        doc.text().rope().unwrap().to_string(),
        "value\r\nkey\r\nkey\r\nother"
    );
}

#[test]
fn whole_word() {
    let doc = Document::from_text(DocumentId(1), "foofoo foo foo_bar 中foo a->b");
    let mut search = query("foo");
    search.whole_word = true;
    search.update(&doc);
    // 「foofoo」中第二个 foo 的前一个字符是 o，不满足全字匹配的条件。
    assert_eq!(search.matches(), &[(7, 10)]);

    // 查询文字的两端不是单词字符时，该端不检查。
    search.query = "->".into();
    search.update(&doc);
    assert_eq!(search.matches(), &[(25, 27)]);

    search.query = "FOO".into();
    search.regex = true;
    search.update(&doc);
    assert_eq!(search.matches(), &[(7, 10)]);
}

#[test]
fn lookaround_and_backreferences_fall_back_to_fancy_regex() {
    let doc = Document::from_text(
        DocumentId(1),
        "price: $100, cost: $200\nfoo foo bar baz baz\n中文中文",
    );
    let found = |q: &str| {
        let mut search = query(q);
        search.regex = true;
        search.update(&doc);
        assert!(search.error().is_none(), "{q}: {:?}", search.error());
        search.matches().to_vec()
    };
    // 覆盖后顾、前瞻（含否定形式）、反向引用和长度不定的后顾；仍然不区分大小写。
    assert_eq!(found(r"(?<=\$)\d+"), [(8, 11), (20, 23)]);
    assert_eq!(found(r"(?<=PRICE: \$|cost: \$)\d+"), [(8, 11), (20, 23)]);
    assert_eq!(found(r"(?<=\w+: \$)2"), [(20, 21)]);
    assert_eq!(found(r"foo(?! bar)"), [(24, 27)]);
    assert_eq!(found(r"\b(\w+) \1\b"), [(24, 31), (36, 43)]);
    assert_eq!(found(r"(中文)\1"), [(44, 48)]);
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

    // 语法有误时，错误不能只是 `regex` 给出的「not supported」，应当带上 `fancy-regex` 的具体原因。
    let mut search = query(r"(?<=a");
    search.regex = true;
    search.update(&doc);
    assert!(search.error().is_some_and(|e| !e.contains("not supported")));

    // 含反向引用的查询在替换时同样展开分组引用，两个重复的词都换成第一个分组。
    let mut doc = Document::from_text(DocumentId(1), "foo foo bar baz baz");
    let mut search = query(r"(\w+) \1");
    search.regex = true;
    search.replacement = "$1".into();
    assert_eq!(search.replace_all(&mut doc), Ok(2));
    assert_eq!(doc.text().rope().unwrap().to_string(), "foo bar baz");
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

#[test]
fn regex_search() {
    let doc = Document::from_text(DocumentId(1), "fn main() {}\nFN Helper()\nlet x = 中文12;");
    let mut search = query(r"fn (\w+)\(");
    search.regex = true;
    search.update(&doc);
    assert_eq!(search.matches(), &[(0, 8), (13, 23)]);

    search.case_sensitive = true;
    search.update(&doc);
    assert_eq!(search.matches(), &[(0, 8)]);

    // 位置按字符偏移计，不按字节。
    search.query = r"\d+".into();
    search.update(&doc);
    assert_eq!(search.matches(), &[(35, 37)]);

    // 空匹配不计入结果，搜索也能正常结束。
    search.query = "z*".into();
    search.update(&doc);
    assert!(search.matches().is_empty());
    assert!(search.is_complete());

    search.query = "(".into();
    search.update(&doc);
    assert!(search.error().is_some());
    assert!(search.matches().is_empty() && search.is_complete());

    search.query = "^let".into();
    search.update(&doc);
    assert!(search.error().is_none());
    assert_eq!(search.matches(), &[(25, 28)]);
}

#[test]
fn replace_current_then_all() {
    let mut doc = Document::from_text(DocumentId(1), "a = 1\nb = 2\nc = 3");
    let mut search = query("=");
    search.replacement = ":".into();

    // 选区不是匹配时不替换。
    assert_eq!(search.replace_current(&mut doc), Ok(false));

    doc.selection = Selection::new(2, 3);
    assert_eq!(search.replace_current(&mut doc), Ok(true));
    assert_eq!(doc.text().substring(0, 17), "a : 1\nb = 2\nc = 3");
    assert_eq!(doc.selection, Selection::point(3));

    assert_eq!(search.replace_all(&mut doc), Ok(2));
    assert_eq!(doc.text().substring(0, 17), "a : 1\nb : 2\nc : 3");

    // 全部替换是一次修改，撤销一次即可恢复。
    doc.execute(Command::Undo, 10, 4);
    assert_eq!(doc.text().substring(0, 17), "a : 1\nb = 2\nc = 3");
}

#[test]
fn regex_replace_expands_groups_and_escapes() {
    let mut doc = Document::from_text(DocumentId(1), "key=value\nname=中文");
    let mut search = query(r"(\w+)=(\w+)");
    search.regex = true;
    search.replacement = r"$2 <- $1\t\\".into();
    assert_eq!(search.replace_all(&mut doc), Ok(2));
    assert_eq!(
        doc.text().substring(0, 100),
        "value <- key\t\\\n中文 <- name\t\\"
    );

    search.query = " <- ".into();
    search.regex = false;
    search.replacement = r"\n$1".into();
    assert_eq!(search.replace_all(&mut doc), Ok(2));
    // 纯文本模式不解析转义，替换文字原样插入。
    assert_eq!(
        doc.text().substring(0, 100),
        "value\\n$1key\t\\\n中文\\n$1name\t\\"
    );

    search.query = "key".into();
    search.regex = true;
    search.replacement = r"a\nb".into();
    assert_eq!(search.replace_all(&mut doc), Ok(1));
    assert!(doc.text().substring(0, 100).contains("$1a\nb\t"));
}

#[test]
fn replacing_after_a_query_change_restarts_the_search() {
    let mut doc = Document::from_text(DocumentId(1), "foo bar");
    let mut search = query("foo");
    search.update(&doc);
    assert_eq!(search.matches(), &[(0, 3)]);
    // 更换查询后直接替换（选区不是匹配，因此不替换）：下次更新按新查询从头搜索。
    search.query = "bar".into();
    assert_eq!(search.replace_current(&mut doc), Ok(false));
    search.update(&doc);
    assert_eq!(search.matches(), &[(4, 7)]);
}

/// 一直搜索到底（每次 `update` 只搜一个时间片）。
fn search_all(search: &mut Search, doc: &Document) {
    search.update(doc);
    while !search.is_complete() {
        search.update(doc);
    }
}

#[test]
fn edits_keep_the_matches_above_them() {
    let text: String = (0..500).map(|i| format!("foo {i} foo\n")).collect();
    let mut doc = Document::from_text(DocumentId(1), &text);
    let mut search = query("foo");
    search_all(&mut search, &doc);
    let revision = doc.revision;
    // 在第 300 行插入一处、在第 400 行删除一处：之前的结果保留，之后的从第 300 行继续搜索。
    for (line, insert) in [(300, "foo "), (400, "")] {
        let at = doc.text().line_start(line);
        let to = if insert.is_empty() { at + 4 } else { at };
        doc.set_selections(vec![Selection::new(at, to)], 0);
        doc.execute(Command::Insert(insert.into()), 10, 4);
    }
    assert_eq!(doc.unchanged_lines_since(revision), 300);
    search.update(&doc);
    assert!(search.matches().len() >= 600, "更改之上的结果还在");
    search_all(&mut search, &doc);
    let mut fresh = query("foo");
    search_all(&mut fresh, &doc);
    assert_eq!(search.matches(), fresh.matches());
    assert_eq!(search.lines(), fresh.lines());
}

#[test]
fn edits_below_a_truncated_search_resume_where_it_stopped() {
    // 结果达到上限就停：停下之后、改动之前的行没有搜过，继续搜索时不能跳过。
    let text = "x\n".repeat(MAX_MATCHES + 50);
    let mut doc = Document::from_text(DocumentId(1), &text);
    let mut search = query("x");
    search_all(&mut search, &doc);
    assert!(search.is_truncated());
    let at = doc.text().line_start(MAX_MATCHES + 40);
    doc.set_selections(vec![Selection::point(at)], 0);
    doc.execute(Command::Insert("y".into()), 10, 4);
    search.update(&doc);
    assert_eq!(search.searched_lines(), MAX_MATCHES);
    search_all(&mut search, &doc);
    let mut fresh = query("x");
    search_all(&mut fresh, &doc);
    assert_eq!(search.matches(), fresh.matches());
}

/// 回归：正则回溯超过上限时，那一行按没有匹配处理，计数和「全部替换」都当作结果完整。
#[test]
fn regex_backtracking_limits_are_reported() {
    let mut doc = Document::from_text(DocumentId(1), &format!("{}bc\naac", "a".repeat(40)));
    // 反向引用把它交给 `fancy-regex`；第一行有 `c`，整块预查放行，`(a|aa)+` 在那一行上的
    // 回溯次数随长度指数增长。
    let mut search = query(r"(a|aa)+\1c");
    search.regex = true;
    search.update(&doc);
    assert!(search.is_complete());
    assert_eq!(search.matches(), &[(43, 46)]);
    assert!(search.gave_up() && !search.is_truncated());
    search.replacement = "x".into();
    assert_eq!(
        search.replace_all(&mut doc),
        Err(t!("editor-replace-all-gave-up"))
    );
    // 查询变了，从头查找，标记随之清除。
    search.query = "aac".into();
    search.regex = false;
    search.update(&doc);
    assert!(!search.gave_up());
    assert_eq!(search.matches(), &[(43, 46)]);
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
fn text_search_uses_the_same_options_without_a_document() {
    let find = |query, case, word, regex| {
        TextSearch::new(query, case, word, regex)
            .unwrap()
            .find_all("Foo foo foobar")
    };
    assert_eq!(find("foo", false, false, false).len(), 3);
    assert_eq!(find("foo", true, false, false), [(4, 7), (8, 11)]);
    assert_eq!(find("foo", false, true, false).len(), 2);
    assert_eq!(find("fo+b", false, false, true), [(8, 12)]);
    assert!(TextSearch::new("(", false, false, true).is_err());
}
