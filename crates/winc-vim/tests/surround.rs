//! 成对符号与缩进对象的手写测试：两者是常用插件的行为，不加载配置的 `vim` 没有这些命令，
//! 无法录制。

mod common;

use winc_core::Lines;
use winc_vim::{Host, Message, Mode, Shared, Vim};

use common::{TestHost, feed};

/// 在文本 `text` 上从第 `line` 行第 `col` 列起输入 `keys`，返回结果文本与光标的行列。
fn run(text: &str, at: (usize, usize), keys: &str) -> (String, (usize, usize)) {
    let mut host = TestHost::new(&format!("{text}\n"));
    let pos = host.rope.line_start(at.0) + at.1;
    host.set_selection(winc_core::Selection::point(pos));
    let (mut vim, mut shared) = (Vim::default(), Shared::default());
    feed(&mut vim, &mut shared, &mut host, keys);
    assert_eq!(vim.mode(), Mode::Normal, "{keys}");
    let text = host.text();
    let caret = host.rope.pos_to_line_col(vim.caret(&host));
    (text.trim_end_matches('\n').to_owned(), caret)
}

#[test]
fn pairs_are_added_around_motions_objects_lines_and_selections() {
    for (keys, expected, caret) in [
        ("ysiw)", "one (two) three", (0, 4)),
        ("ysiw(", "one ( two ) three", (0, 4)),
        ("ysiw]", "one [two] three", (0, 4)),
        ("ysiwB", "one {two} three", (0, 4)),
        ("ysiw>", "one <two> three", (0, 4)),
        ("ysiw\"", "one \"two\" three", (0, 4)),
        ("ysiw*", "one *two* three", (0, 4)),
        // 末尾的空白不包在内。
        ("ysw)", "one (two) three", (0, 4)),
        ("ys$'", "one 'two three'", (0, 4)),
        ("yss)", "(one two three)", (0, 0)),
        ("viwS}", "one {two} three", (0, 4)),
        ("vllS[", "one [ two ] three", (0, 4)),
        ("VS)", "(one two three)", (0, 0)),
        ("ysiw)W.", "one (two) (three)", (0, 10)),
        ("ysiw)u", "one two three", (0, 4)),
    ] {
        assert_eq!(
            run("one two three", (0, 4), keys),
            (expected.to_owned(), caret),
            "{keys}"
        );
    }
    assert_eq!(run("  one\n  two", (0, 3), "2yss)").0, "  (one\n  two)");
}

#[test]
fn pairs_are_deleted_and_changed_around_the_cursor() {
    for (text, keys, expected) in [
        ("a (b c) d", "ds)", "a b c d"),
        ("a ( b c ) d", "ds(", "a b c d"),
        ("a ( b c ) d", "ds)", "a  b c  d"),
        ("a [b {c} d] e", "ds}", "a [b c d] e"),
        ("a [b {c} d] e", "ds]", "a b {c} d e"),
        ("a \"b c\" d", "ds\"", "a b c d"),
        ("a 'b c' d", "ds'", "a b c d"),
        ("a *b c* d", "ds*", "a b c d"),
        ("a <b c> d", "dsa", "a b c d"),
        ("<p>b <i>c</i></p>", "dst", "<p>b c</p>"),
        ("a (b c) d", "cs)]", "a [b c] d"),
        ("a (b c) d", "cs)[", "a [ b c ] d"),
        ("a \"b c\" d", "cs\"'", "a 'b c' d"),
        ("a [b c] d", "cs]B", "a {b c} d"),
        ("<p>b <i>c</i></p>", "cst\"", "<p>b \"c\"</p>"),
        ("a (b c) d", "ds)u", "a (b c) d"),
        ("(a) (b)", "ds)0.", "a b"),
    ] {
        assert_eq!(run(text, (0, 6), keys).0, expected, "{text} {keys}");
    }
    // 找不到成对的符号时不改动文本；需要输入标签名的写法提示不支持。
    let mut host = TestHost::new("plain text\n");
    let (mut vim, mut shared) = (Vim::default(), Shared::default());
    feed(&mut vim, &mut shared, &mut host, "ds)cs\"'");
    assert_eq!(
        (host.text().as_str(), host.messages.len()),
        ("plain text\n", 0)
    );
    feed(&mut vim, &mut shared, &mut host, "ysiwt");
    assert_eq!(host.messages, [Message::Unsupported]);
    feed(&mut vim, &mut shared, &mut host, "ys<Esc>ds<Esc>");
    assert_eq!(
        (host.text().as_str(), vim.pending().len()),
        ("plain text\n", 0)
    );
}

#[test]
fn indent_objects_cover_the_lines_at_least_as_deep() {
    let text = "fn a() {\n    one\n\n    if x {\n        two\n    }\n\n}\nlast";
    for (at, keys, expected) in [
        ((1, 4), "dii", "fn a() {\n\n}\nlast"),
        ((1, 4), "dai", "\n}\nlast"),
        (
            (4, 8),
            "dii",
            "fn a() {\n    one\n\n    if x {\n    }\n\n}\nlast",
        ),
        ((4, 8), "dai", "fn a() {\n    one\n\n    }\n\n}\nlast"),
        // 空行取最近的非空行的缩进。
        ((2, 0), "dii", "fn a() {\n\n}\nlast"),
        (
            (8, 0),
            "yiiP",
            "fn a() {\n    one\n\n    if x {\n        two\n    }\n\n}\nlast",
        ),
        ((3, 4), "viiy", text),
        (
            (1, 4),
            ">ii",
            "fn a() {\n\t    one\n\n\t    if x {\n\t\ttwo\n\t    }\n\n}\nlast",
        ),
    ] {
        let (result, _) = run(text, at, keys);
        if keys == "yiiP" {
            assert!(
                result.starts_with("fn a() {\n    one\n\n    if x {"),
                "{keys}"
            );
            assert_eq!(result.lines().count(), 18, "{keys}");
        } else {
            assert_eq!(result, expected, "{at:?} {keys}");
        }
    }
}
