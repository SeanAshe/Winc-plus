//! `transform` 模块的测试：大小写、排序、合并行、行尾空白、缩进与换行的转换，以及括号跳转与
//! 选区扩大。

use ropey::Rope;

use super::*;
use crate::editing::edit::History;

/// 执行一次操作并撤销，返回操作后的文本和选区。
///
/// 撤销后必须还原为原文，否则断言失败，因此每个用例同时验证撤销。
fn run(
    text: &str,
    selection: Selection,
    f: impl Fn(&Rope, Selection) -> Option<Transaction>,
) -> Option<(String, Selection)> {
    let mut rope = Rope::from_str(text);
    let transaction = f(&rope, selection)?;
    transaction.apply(&mut rope);
    let after = rope.to_string();
    let mut history = History::default();
    history.push(transaction.clone());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), text, "撤销后应恢复原文");
    Some((after, transaction.after))
}

#[test]
fn case_changes_the_selection_or_the_word_at_the_caret() {
    let upper = |t: &Rope, s| change_case(t, s, Case::Upper);
    assert_eq!(
        run("let foo_bar = 1;", Selection::point(6), upper),
        Some(("let FOO_BAR = 1;".into(), Selection::point(6)))
    );
    assert_eq!(
        run("abc def", Selection::new(7, 4), |t, s| change_case(
            t,
            s,
            Case::Upper
        )),
        Some(("abc DEF".into(), Selection::new(7, 4)))
    );
    // `ß` 转换为大写后变成 `SS`（两个字符），选区终点随之增加 1。
    assert_eq!(
        run("straße", Selection::new(0, 6), upper),
        Some(("STRASSE".into(), Selection::new(0, 7)))
    );
    // 光标在空白上、或文本已经是大写时没有内容需要改变，返回 `None`。
    assert!(run("a  b", Selection::point(2), upper).is_none());
    assert!(run("ABC", Selection::point(1), upper).is_none());

    assert_eq!(Case::Lower.apply("HeLLo 世界"), "hello 世界");
    assert_eq!(
        Case::Title.apply("hello WORLD, don't 'quote' x-ray 2nd"),
        "Hello World, Don't 'Quote' X-Ray 2nd"
    );
}

#[test]
fn sorting_sorts_the_selected_lines_or_the_whole_file() {
    let sort = |t: &Rope, s| sort_lines(t, s, false);
    // 选区在一行之内：排整个文件，末尾的空行不动；比较时忽略大小写，相同时小写在前。
    let (out, sel) = run("pear\nBanana\napple\nbanana\n", Selection::point(1), sort).unwrap();
    assert_eq!(out, "apple\nbanana\nBanana\npear\n");
    assert_eq!(sel, Selection::point(1));
    // 只排选中的行；选区结束在下一行行首时该行不计入，换行符留在原处。
    let (out, _) = run("z\r\nc\r\nb\r\na\r\n", Selection::new(3, 9), sort).unwrap();
    assert_eq!(out, "z\r\nb\r\nc\r\na\r\n");
    let (out, _) = run("b\na\nc", Selection::point(0), |t, s| {
        sort_lines(t, s, true)
    })
    .unwrap();
    assert_eq!(out, "c\nb\na");
    // 已经有序、只有一行：不产生事务。
    assert!(run("a\nb\n", Selection::point(0), sort).is_none());
    assert!(run("only", Selection::point(0), sort).is_none());
    assert!(sorts_whole_file(
        &Rope::from_str("a\nb"),
        Selection::point(0)
    ));
    // 各行长短变了：光标留在原来的行和列上，不落进 CRLF 的两个字符之间。
    let (out, sel) = run("b\r\naa\r\n", Selection::point(3), sort).unwrap();
    assert_eq!(out, "aa\r\nb\r\n");
    assert_eq!(sel, Selection::point(4));
    let (_, sel) = run("bbb\na\n", Selection::point(3), sort).unwrap();
    assert_eq!(sel, Selection::point(1), "列超过新行的长度时停在行尾");
}

#[test]
fn joining_lines_drops_indentation_and_keeps_one_space() {
    let (out, sel) = run("if x {\n    a();\n}", Selection::point(2), join_lines).unwrap();
    assert_eq!(out, "if x { a();\n}");
    assert_eq!(sel, Selection::point(6));
    // 前一行末尾有空白时替换为一个空格；空行丢弃；并入全是空白的行时不加空格。
    let (out, _) = run("a  \n  \n\tb\nc", Selection::new(0, 8), join_lines).unwrap();
    assert_eq!(out, "a b\nc");
    let (out, _) = run("\nx", Selection::point(0), join_lines).unwrap();
    assert_eq!(out, "x");
    // 第一行的缩进保留，只去掉并入行开头的空白。
    let (out, _) = run("    a\n      b", Selection::point(0), join_lines).unwrap();
    assert_eq!(out, "    a b");
    // 并入的全是空行：光标停在第一行末尾，不回到行首。
    let (out, sel) = run("ab  \n  \nc", Selection::point(0), join_lines).unwrap();
    assert_eq!((out.as_str(), sel), ("ab\nc", Selection::point(2)));
    // 最后一行没有下一行可并入。
    assert!(run("a\nb", Selection::point(3), join_lines).is_none());
    // 光标停在最后一次并入的段落之前（空格之前）；选中多行时一次合并完。
    let (out, sel) = run("a\n  b\n  c", Selection::point(0), join_lines).unwrap();
    assert_eq!((out.as_str(), sel), ("a b\n  c", Selection::point(1)));
    let (out, sel) = run("a\n  b\n  c", Selection::new(0, 8), join_lines).unwrap();
    assert_eq!((out.as_str(), sel), ("a b c", Selection::new(0, 5)));
    // 选区起点落在第一行行尾被移除的空白里：退到去掉空白后的行尾，不越界、不落进 CRLF 中间。
    let (out, sel) = run("ab   \r\nc\r\nz", Selection::new(5, 8), join_lines).unwrap();
    assert_eq!((out.as_str(), sel), ("ab c\r\nz", Selection::new(2, 4)));
    let (out, sel) = run("          \n\n", Selection::new(9, 2), join_lines).unwrap();
    assert_eq!((out.as_str(), sel), ("\n", Selection::new(0, 0)));
}

#[test]
fn trailing_whitespace_is_trimmed_everywhere() {
    let (out, sel) = run(
        "a  \nb\t\n  \nc",
        Selection::point(9),
        trim_trailing_whitespace,
    )
    .unwrap();
    assert_eq!(out, "a\nb\n\nc");
    assert_eq!(sel, Selection::point(4));
    assert!(run("a\nb", Selection::point(0), trim_trailing_whitespace).is_none());
    // 全角空格不属于空格和制表符，视为内容保留。
    assert!(
        run(
            "标题\u{3000}\nb",
            Selection::point(0),
            trim_trailing_whitespace
        )
        .is_none()
    );
}

#[test]
fn indentation_converts_between_tabs_and_spaces() {
    let to_spaces = |t: &Rope, s| convert_indentation(t, s, false, 4);
    let (out, sel) = run("a\n\tb\n\t  c\n  \td", Selection::point(5), to_spaces).unwrap();
    assert_eq!(out, "a\n    b\n      c\n    d");
    assert_eq!(sel, Selection::point(8));
    // 转换成制表符时，凑不满一个制表符的空格保留。
    let to_tabs = |t: &Rope, s| convert_indentation(t, s, true, 4);
    let (out, _) = run("    a\n      b\n  c", Selection::point(0), to_tabs).unwrap();
    assert_eq!(out, "\ta\n\t  b\n  c");
    assert!(run("a\n    b", Selection::point(0), to_spaces).is_none());
    assert!(run("\t\t  a\n   b", Selection::point(0), to_tabs).is_none());
    // 制表符之前的空格、凑满一个制表符的空格也要转换。
    let (out, _) = run(" \ta\n\t    b", Selection::point(0), to_tabs).unwrap();
    assert_eq!(out, "\ta\n\t\tb");
}

#[test]
fn line_endings_change_everywhere() {
    let crlf = |t: &Rope, s| change_line_endings(t, s, "\r\n");
    let (out, sel) = run("a\nb\r\nc\rd\n", Selection::point(6), crlf).unwrap();
    assert_eq!(out, "a\r\nb\r\nc\r\nd\r\n");
    // 光标在 `c` 的行尾；其之前第一个 `\n` 变成 `\r\n`，多了 1 个字符。
    assert_eq!(sel, Selection::point(7));
    let (out, _) = run("a\r\nb", Selection::point(0), |t, s| {
        change_line_endings(t, s, "\n")
    })
    .unwrap();
    assert_eq!(out, "a\nb");
    assert!(run("a\r\nb", Selection::point(0), crlf).is_none());
    // 目标换行符都是一个字符时，仍需分清 LF 和 CR。
    let cr = |t: &Rope, s| change_line_endings(t, s, "\r");
    let (out, _) = run("a\nb\rc\r\n", Selection::point(0), cr).unwrap();
    assert_eq!(out, "a\rb\rc\r");
    assert!(run("a\rb\r", Selection::point(0), cr).is_none());
}

#[test]
fn jumping_between_brackets() {
    let text = Rope::from_str("fn f() {\n    g(a);\n}\n");
    // 紧邻左括号时跳到右括号之前，在右括号之前再次触发则跳回左括号。
    assert_eq!(jump_to_bracket(&text, 7), Some(19));
    assert_eq!(jump_to_bracket(&text, 8), Some(19));
    assert_eq!(jump_to_bracket(&text, 19), Some(7));
    // 不紧邻括号时跳到包住光标的左括号；外面没有括号时跳到之后的第一个括号。
    assert_eq!(jump_to_bracket(&text, 11), Some(7));
    assert_eq!(jump_to_bracket(&text, 0), Some(4));
    assert_eq!(jump_to_bracket(&Rope::from_str("abc"), 1), None);
    // 空括号：光标在括号之间时跳到左括号，在左括号之前时跳到右括号。
    let empty = Rope::from_str("f()");
    assert_eq!(jump_to_bracket(&empty, 2), Some(1));
    assert_eq!(jump_to_bracket(&empty, 1), Some(2));
}

#[test]
fn selection_grows_from_word_to_string_to_brackets_to_line_to_file() {
    let text = Rope::from_str("x = 1\n    call(\"hello world\", [a, b])\n");
    let grow = |s: Selection| expand_selection(&text, s).unwrap();
    let slice = |s: Selection| text.substring(s.from(), s.to());

    let mut sel = Selection::point(17);
    let mut steps = Vec::new();
    for _ in 0..7 {
        sel = grow(sel);
        steps.push(slice(sel));
    }
    assert_eq!(
        steps,
        [
            "hello",
            "hello world",
            "\"hello world\"",
            "\"hello world\", [a, b]",
            "(\"hello world\", [a, b])",
            "call(\"hello world\", [a, b])",
            "    call(\"hello world\", [a, b])",
        ]
    );
    sel = grow(sel);
    assert_eq!(slice(sel), "    call(\"hello world\", [a, b])\n");
    sel = grow(sel);
    assert_eq!(sel, Selection::new(0, text.len_chars()));
    assert!(expand_selection(&text, sel).is_none());

    // 方括号内的一个字母：依次是词、括号内的内容、含括号的整体。
    let mut sel = Selection::point(32);
    let mut steps = Vec::new();
    for _ in 0..3 {
        sel = grow(sel);
        steps.push(slice(sel));
    }
    assert_eq!(steps, ["a", "a, b", "[a, b]"]);

    // 前面单词中的撇号不是引号，因此不影响后面的字符串。
    let text = Rope::from_str("don't say \"hi there\"");
    let sel = expand_selection(&text, Selection::point(12)).unwrap();
    assert_eq!(text.substring(sel.from(), sel.to()), "hi");
    let sel = expand_selection(&text, sel).unwrap();
    assert_eq!(text.substring(sel.from(), sel.to()), "hi there");

    // 外层括号尚未闭合：不把它和里层的右括号配成一对（`(ab`），跳过它向外找。
    let text = Rope::from_str("((ab)");
    let sel = expand_selection(&text, Selection::new(1, 3)).unwrap();
    assert_eq!(text.substring(sel.from(), sel.to()), "((ab)");
    let text = Rope::from_str("x = ((a, b)");
    let sel = expand_selection(&text, Selection::new(6, 7)).unwrap();
    assert_eq!(text.substring(sel.from(), sel.to()), "a, b");
    let sel = expand_selection(&text, sel).unwrap();
    assert_eq!(text.substring(sel.from(), sel.to()), "(a, b)");
}

/// 回归：选区超出内层括号对时，每向外一层都从新的左括号重新扫描，嵌套很深时总扫描量随层数
/// 平方增长。
///
/// 各层共用 `SCAN_LIMIT` 个字符的预算，用完以后不再向外找。
#[test]
fn expanding_through_deep_nesting_shares_one_budget() {
    let n = 1100;
    let text = Rope::from_str(&format!("({}{}Z)", "(".repeat(n), ")".repeat(n)));
    // 选区从最内层括号之间到 `Z` 之后：内层的每一对都在选区里结束，要一层层向外找。
    let sel = Selection::new(n + 1, 2 * n + 2);
    let grown = expand_selection(&text, sel).unwrap();
    // 预算在找到最外层之前耗尽：直接扩大到整行，不选最外层括号内的内容。
    assert_eq!((grown.from(), grown.to()), (0, 2 * n + 3));
    // 层数少时找得到最外层，选中其中的内容。
    let text = Rope::from_str("(((()))Z)");
    let wider = expand_selection(&text, Selection::new(4, 8)).unwrap();
    assert_eq!((wider.from(), wider.to()), (1, 8));
}

#[test]
fn transpose_swaps_graphemes_around_the_caret() {
    // 行中：光标前后的两个字素交换，光标移到两者之后。
    assert_eq!(
        run("abcd", Selection::point(2), transpose),
        Some(("acbd".into(), Selection::point(3)))
    );
    // 行尾：交换行末的两个字素，光标留在行尾。
    assert_eq!(
        run("abc", Selection::point(3), transpose),
        Some(("acb".into(), Selection::point(3)))
    );
    // 行首：与前面的换行符交换，本行的第一个字素接到上一行末尾。
    assert_eq!(
        run("a\nbc", Selection::point(2), transpose),
        Some(("ab\nc".into(), Selection::point(3)))
    );
    // 两个字素相同：文字不变，光标仍然前移。
    assert_eq!(
        run("aab", Selection::point(1), transpose),
        Some(("aab".into(), Selection::point(2)))
    );
    // 文档开头、只有一个字素、有选区：不处理。
    assert_eq!(run("abcd", Selection::point(0), transpose), None);
    assert_eq!(run("x", Selection::point(1), transpose), None);
    assert_eq!(run("abcd", Selection::new(1, 3), transpose), None);
    // 一个字素可以是多个码点：emoji 整体交换；CRLF 换行也作为一个整体。
    assert_eq!(
        run("a👍b", Selection::point(2), transpose),
        Some(("ab👍".into(), Selection::point(3)))
    );
    assert_eq!(
        run("a\r\nb", Selection::point(3), transpose),
        Some(("ab\r\n".into(), Selection::point(4)))
    );
}

#[test]
fn unescape_unicode_turns_escapes_into_characters() {
    // 没有选区时转换整个文件；光标随文本调整。
    assert_eq!(
        run(
            "greeting=\\u4F60\\u597D\n",
            Selection::point(21),
            unescape_unicode
        ),
        Some(("greeting=你好\n".into(), Selection::point(11)))
    );
    // 代理项成对时合成一个字符，十六进制大小写都认。
    assert_eq!(
        run(
            "a=\\uD83D\\uDE00 \\u4f60",
            Selection::point(0),
            unescape_unicode
        ),
        Some(("a=😀 你".into(), Selection::point(0)))
    );
    // 两个反斜杠是一对；落单的代理项、控制字符、行分隔符、残缺的转义保留原文。
    for kept in [
        "a=\\\\u4F60",
        "\\uD83D",
        "\\uDE00x",
        "a=\\u000A",
        "a=\\u2028",
        "\\u12G4",
        "\\u12",
        "\\U4F60",
        "a=1\\tb",
    ] {
        assert_eq!(
            run(kept, Selection::point(0), unescape_unicode),
            None,
            "{kept}"
        );
    }
    assert_eq!(
        run("a=\\\\\\u4F60", Selection::point(0), unescape_unicode),
        Some(("a=\\\\你".into(), Selection::point(0)))
    );
    // 有选区时只转换选区，选区包住转换后的文字。
    assert_eq!(
        run("\\u4F60=\\u597D", Selection::new(7, 13), unescape_unicode),
        Some(("\\u4F60=好".into(), Selection::new(7, 8)))
    );
}

#[test]
fn escape_unicode_writes_characters_as_escapes() {
    assert_eq!(
        run("greeting=你好\n", Selection::point(11), escape_unicode),
        Some(("greeting=\\u4F60\\u597D\n".into(), Selection::point(21)))
    );
    // 补充平面的字符写成一对代理项；ASCII 和已有的转义不动。
    assert_eq!(
        run("a=😀 \\u4F60", Selection::point(0), escape_unicode),
        Some(("a=\\uD83D\\uDE00 \\u4F60".into(), Selection::point(0)))
    );
    assert_eq!(
        run("key=value\t\\n", Selection::point(0), escape_unicode),
        None
    );
    // 有选区时只转换选区。
    assert_eq!(
        run("你=好", Selection::new(2, 3), escape_unicode),
        Some(("你=\\u597D".into(), Selection::new(2, 8)))
    );
    // 落单的反斜杠写成 `\u005C`，转回字符时不变；成对的反斜杠不动。
    for source in ["a\\你", "\\\\\\好\\\\好"] {
        let escaped = run(source, Selection::point(0), escape_unicode).unwrap().0;
        let back = run(&escaped, Selection::point(0), unescape_unicode);
        assert_eq!(back.unwrap().0, source, "{escaped}");
    }
    // 两个方向互为逆运算。
    let source = "名称=值 😀\n# 注释\n";
    let escaped = run(source, Selection::point(0), escape_unicode).unwrap().0;
    let back = run(&escaped, Selection::point(0), unescape_unicode)
        .unwrap()
        .0;
    assert_eq!(back, source);
}

#[test]
fn unicode_escape_text_helpers_round_trip() {
    let source = "a=\\u4F60\\u597D \\\\u4F60 \\u000A \\uD83D\\uDE00";
    let shown = unescape_unicode_text(source);
    assert_eq!(shown, "a=你好 \\\\u4F60 \\u000A 😀");
    assert_eq!(escape_unicode_text(&shown, false, false), source);
    assert_eq!(escape_unicode_text("你", true, false), "\\u4f60");
    // ISO-8859-1 的字符原样保留；落单的反斜杠后面的字符是字符本身，去掉反斜杠。
    assert_eq!(escape_unicode_text("ü你", false, true), "ü\\u4F60");
    let written = escape_unicode_text("a\\你 b\\\\你", false, false);
    assert_eq!(written, "a\\u4F60 b\\\\\\u4F60");
    // 看不见的字符和 ASCII 字符保留转义；DEL 是 ASCII，不转义。
    assert_eq!(unescape_unicode_text("\\uFEFFa\\u200B"), "\\uFEFFa\\u200B");
    let ascii = "my\\u0020key=\\u003D\\u005C\\u0041";
    assert_eq!(unescape_unicode_text(ascii), ascii);
    assert_eq!(escape_unicode_text("\u{7F}", false, false), "\u{7F}");
    assert_eq!(unescape_unicode_text("plain"), "plain");
}

/// 按 `java.util.Properties` 读出的值（UTF-16 码元）；转义残缺或末尾是反斜杠时返回 `None`。
fn java_value(raw: &str) -> Option<Vec<u16>> {
    let chars: Vec<char> = raw.chars().collect();
    let (mut out, mut units, mut i) = (Vec::new(), [0; 2], 0);
    while i < chars.len() {
        let mut c = chars[i];
        i += 1;
        if c == '\\' {
            c = *chars.get(i)?;
            i += 1;
            let control = match c {
                'u' => {
                    let hex = chars.get(i..i + 4)?;
                    i += 4;
                    hex.iter()
                        .try_fold(0, |v, c| Some(v * 16 + c.to_digit(16)? as u16))
                }
                't' => Some(9),
                'n' => Some(10),
                'r' => Some(13),
                'f' => Some(12),
                _ => None,
            };
            if let Some(unit) = control {
                out.push(unit);
                continue;
            }
            if c == 'u' {
                return None;
            }
        }
        out.extend_from_slice(c.encode_utf16(&mut units));
    }
    Some(out)
}

/// 换成字符再写回转义后，`.properties` 读出的值不变；只有 ASCII、没有可换转义的文字原样写回。
#[test]
fn unicode_escape_text_keeps_properties_values() {
    let tokens = [
        "\\", "u", "t", "a", "\\u4F60", "\\u0041", "\\u00E9", "\\u005C", "\\u200B", "\\uD83D",
        "\\uDE00", "你", "\u{e9}", "\u{200B}", "😀",
    ];
    let mut sources = vec![String::new()];
    for _ in 0..4 {
        let longer: Vec<String> = (sources.iter())
            .flat_map(|s| tokens.iter().map(move |t| format!("{s}{t}")))
            .collect();
        sources.extend(longer);
        sources.dedup();
    }
    for raw in &sources {
        let Some(value) = java_value(raw) else {
            continue;
        };
        let shown = unescape_unicode_text(raw);
        for latin1 in [false, true] {
            if latin1 && raw.chars().any(|c| u32::from(c) > 0xFF) {
                continue;
            }
            let written = escape_unicode_text(&shown, false, latin1);
            assert_eq!(
                java_value(&written).as_ref(),
                Some(&value),
                "{raw:?} → {written:?}"
            );
            if raw.is_ascii() && shown == *raw {
                assert_eq!(&written, raw);
            }
        }
    }
}
