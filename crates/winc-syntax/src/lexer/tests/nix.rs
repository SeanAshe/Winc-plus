//! Nix 与 HCL 字符串的词法测试：缩进字符串的转义、`$$`、跨行的字符串和插值、带 `'` 的名字。

use super::*;

/// 缩进字符串中内嵌 shell 脚本：`''${` 是转义，其后的 `/*` 仍在字符串中，`'';` 后恢复为代码。
#[test]
fn nix_indented_strings_keep_escapes_inside() {
    let sample = [
        "let",
        "  s = ''",
        "    x=\"''${FOO}\"",
        "    \"$dir\"/*)",
        "  '';",
        "in s",
    ];
    let mut state = LineState::Normal;
    let mut lines = Vec::new();
    for line in sample {
        let (tokens, next) = lex("a.nix", line, state);
        lines.push(tokens);
        state = next;
    }
    assert_eq!(
        lines[2],
        [
            ("    x=\"".into(), TokenKind::String),
            ("''$".into(), TokenKind::Escape),
            ("{FOO}\"".into(), TokenKind::String),
        ]
    );
    assert_eq!(lines[3], [("    \"$dir\"/*)".into(), TokenKind::String)]);
    assert_eq!(lines[5], [("in".into(), TokenKind::Keyword)]);
    assert_eq!(state, LineState::Normal);

    let inside = LineState::BlockString(0);
    let escapes = texts("a.nix", r"'''a''' ''\n ''\'", inside, TokenKind::Escape);
    assert_eq!(escapes, ["'''", "'''", r"''\n", r"''\'"]);
    let (_, next) = lex("a.nix", r"'''a''' ''\n ''\'", inside);
    assert_eq!(next, inside);
}

/// `$$` 是字面字符：其后的 `{` 不开始插值；缩进字符串中的 `''$` 先作为转义，其后的 `${` 仍是
/// 插值。HCL 的 `$${` 同样是字面的 `${`。
#[test]
fn doubled_dollars_do_not_interpolate() {
    let strings = |file: &str, line: &str| texts(file, line, LineState::Normal, TokenKind::String);
    assert_eq!(
        strings("a.nix", r#"a = "$${x}" + ''$${y}'';"#),
        ["\"$${x}\"", "''$${y}''"]
    );
    assert_eq!(strings("a.nix", "b = ''''$${x}'';"), ["''", "${", "}''"]);
    assert_eq!(
        strings("main.tf", r#"a = "$${x} ${y}""#),
        ["\"$${x} ${", "}\""]
    );
}

/// Nix 的双引号字符串可以跨行；名字中的 `'` 不开始字符串（`x''` 是一个名字）。
#[test]
fn nix_strings_span_lines_and_names_take_primes() {
    let (_, next) = lex("a.nix", r#"c = "line1"#, LineState::Normal);
    assert_eq!(next, LineState::DoubleQuote);
    let (tokens, next) = lex("a.nix", r#"line2"; d = true;"#, next);
    assert_eq!(
        tokens,
        [
            ("line2\"".into(), TokenKind::String),
            ("true".into(), TokenKind::Builtin),
        ]
    );
    assert_eq!(next, LineState::Normal);

    let (tokens, next) = lex("a.nix", "f = x: x'' + inputs'.foo;", LineState::Normal);
    assert!(
        tokens.iter().all(|(_, kind)| *kind != TokenKind::String),
        "{tokens:?}"
    );
    assert_eq!(next, LineState::Normal);
}

/// 依次分析 `lines`，返回每行的 token 和行末状态。
fn lex_lines(file: &str, lines: &[&str]) -> Vec<(Vec<(String, TokenKind)>, LineState)> {
    let mut state = LineState::Normal;
    lines
        .iter()
        .map(|line| {
            let (tokens, next) = lex(file, line, state);
            state = next;
            (tokens, next)
        })
        .collect()
}

/// 回归：双引号字符串里跨行的 `${…}` 曾退回字符串扫描：插值里的缩进被当成字符串，
/// 插值里的字符串被当成代码。
#[test]
fn nix_interpolation_spans_lines_in_double_quoted_string() {
    let lines = lex_lines(
        "a.nix",
        &[
            r#"ExecStart = "${pkgs.foo}/bin/foo ${lib.escapeShellArgs ["#,
            r#"  "--flag""#,
            r#"]}";"#,
            "x = 1;",
        ],
    );
    assert_eq!(
        lines[0].0,
        vec![
            ("\"${".into(), TokenKind::String),
            ("}/bin/foo ${".into(), TokenKind::String),
        ]
    );
    // 插值里是代码，不算在字符串中。
    assert!(matches!(
        lines[0].1,
        LineState::Interpolation { top: 0, .. }
    ));
    assert!(!lines[0].1.is_quoted());
    assert_eq!(lines[1].0, vec![("\"--flag\"".into(), TokenKind::String)]);
    assert_eq!(lines[1].1, lines[0].1);
    assert_eq!(lines[2].0, vec![("}\"".into(), TokenKind::String)]);
    assert_eq!(lines[2].1, LineState::Normal);
    assert_eq!(lines[3].0, vec![("1".into(), TokenKind::Number)]);
}

/// 回归：跨行的 `${…}` 里嵌套的缩进字符串曾被当成外层字符串的收尾，其中 shell 通配符的 `/*`
/// 成了一直延续到文件末尾的块注释。
#[test]
fn nix_indented_string_nested_in_multiline_interpolation() {
    let lines = lex_lines(
        "a.nix",
        &[
            "text = ''",
            "  ${lib.optionalString cond ''",
            r#"    rm -rf "$dir"/* && sed 's/"//'"#,
            "  ''}",
            "'';",
            "x = 1;",
        ],
    );
    assert_eq!(lines[0].1, LineState::BlockString(0));
    assert_eq!(
        lines[1].0,
        vec![
            ("  ${".into(), TokenKind::String),
            ("''".into(), TokenKind::String),
        ]
    );
    assert!(lines[1].1.is_quoted());
    // 内层字符串的内容整行是字符串，奇数个双引号不影响之后的行。
    assert_eq!(
        lines[2].0,
        vec![(
            r#"    rm -rf "$dir"/* && sed 's/"//'"#.into(),
            TokenKind::String
        )]
    );
    assert_eq!(lines[2].1, lines[1].1);
    // 内层字符串收尾后回到插值，插值收尾后回到外层字符串。
    assert_eq!(lines[3].1, LineState::BlockString(0));
    assert_eq!(lines[4].1, LineState::Normal);
    assert_eq!(lines[5].0, vec![("1".into(), TokenKind::Number)]);
}

/// 跨行的插值里成对的花括号不收尾插值，字符串里的 `}` 不计数。
#[test]
fn nix_multiline_interpolation_counts_braces() {
    let lines = lex_lines("a.nix", &[r#"a = "${ {"#, r#"  x = "}";"#, r#"}.x } b";"#]);
    assert_eq!(lines[1].0, vec![("\"}\"".into(), TokenKind::String)]);
    assert!(matches!(
        lines[1].1,
        LineState::Interpolation { top: 0, .. }
    ));
    assert_eq!(lines[2].0, vec![("} b\"".into(), TokenKind::String)]);
    assert_eq!(lines[2].1, LineState::Normal);
}

/// Markdown 代码块的内层状态放不下插值的嵌套，退回外层的字符串，字符串收尾后恢复。
#[test]
fn nix_multiline_interpolation_in_markdown_fence_falls_back_to_string() {
    let lines = lex_lines("a.md", &["```nix", r#"a = "${f ["#, r#"]}";"#, "x = 1;"]);
    let inner = |state| match state {
        LineState::Fence { inner, .. } => inner,
        other => panic!("{other:?}"),
    };
    assert_eq!(expand(inner(lines[1].1)), LineState::DoubleQuote);
    assert_eq!(expand(inner(lines[2].1)), LineState::Normal);
    assert_eq!(lines[3].0, vec![("1".into(), TokenKind::Number)]);
}

/// 跨行的插值里注释照常识别：注释中的括号不计数，引号和 `''` 不打开字符串。
#[test]
fn nix_multiline_interpolation_reads_comments() {
    let lines = lex_lines(
        "a.nix",
        &[
            "text = ''",
            "  ${",
            r#"    # fallback when empty (e.g. "foo" or {"#,
            "    /* note: handle } */ pkgs.hello",
            r#"    /* open " }"#,
            "     still } '' */",
            "  }",
            "'';",
            "x = 1;",
        ],
    );
    assert_eq!(
        lines[2].0,
        vec![(
            r#"# fallback when empty (e.g. "foo" or {"#.into(),
            TokenKind::Comment
        )]
    );
    assert_eq!(lines[2].1, lines[1].1);
    assert_eq!(
        lines[3].0,
        vec![("/* note: handle } */".into(), TokenKind::Comment)]
    );
    assert_eq!(lines[3].1, lines[1].1);
    // 没有结束的块注释连同外层的字符串带到下一行，结束后回到插值的代码。
    assert!(lines[4].1.is_quoted());
    assert_eq!(
        lines[5].0,
        vec![("     still } '' */".into(), TokenKind::Comment)]
    );
    assert_eq!(lines[5].1, lines[1].1);
    assert_eq!(lines[6].1, LineState::BlockString(0));
    assert_eq!(lines[7].1, LineState::Normal);
    assert_eq!(lines[8].0, vec![("1".into(), TokenKind::Number)]);
}

/// 回归：名字中的 `'` 曾在词法分析时被拆开：末尾的 `''` 被当成缩进字符串的开头，`id' ''` 中的
/// `' '` 被当成字符字面量、吞掉缩进字符串开头的一个引号。
#[test]
fn nix_names_include_primes() {
    let lines = lex_lines(
        "a.nix",
        &[
            r#"s = "${let x'' = 1; in toString 3}";"#,
            "let pkgs'' = pkgs'.extend overlay; in pkgs''",
            "a = ''str''; b = [1''x''];",
            "s = id' ''",
            r#"  "$dir"/*)"#,
            "'';",
            "n = 42;",
        ],
    );
    assert_eq!(
        lines[0].0,
        vec![
            ("\"${".into(), TokenKind::String),
            ("let".into(), TokenKind::Keyword),
            ("1".into(), TokenKind::Number),
            ("in".into(), TokenKind::Keyword),
            ("toString".into(), TokenKind::Builtin),
            ("3".into(), TokenKind::Number),
            ("}\"".into(), TokenKind::String),
        ]
    );
    assert_eq!(lines[0].1, LineState::Normal);
    assert_eq!(
        lines[1].0,
        vec![
            ("let".into(), TokenKind::Keyword),
            ("in".into(), TokenKind::Keyword),
        ]
    );
    // 不紧跟在名字之后的 `''` 仍然是字符串，数字之后的也是。
    assert_eq!(
        lines[2].0,
        vec![
            ("''str''".into(), TokenKind::String),
            ("1".into(), TokenKind::Number),
            ("''x''".into(), TokenKind::String),
        ]
    );
    assert_eq!(lines[3].0, vec![("''".into(), TokenKind::String)]);
    assert_eq!(lines[3].1, LineState::BlockString(0));
    assert_eq!(lines[4].1, LineState::BlockString(0));
    assert_eq!(lines[5].1, LineState::Normal);
    assert_eq!(lines[6].0, vec![("42".into(), TokenKind::Number)]);
}

/// 回归：`\u{` 之后不是十六进制数时，转义曾一直找到之后的 `}`，把收尾的引号也算进去。
#[test]
fn nix_unicode_brace_escape_does_not_swallow_the_closing_quote() {
    let (tokens, state) = lex("a.nix", r#"a = "\u{"; b = { x = 1; };"#, LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("\"".into(), TokenKind::String),
            ("\\u".into(), TokenKind::Escape),
            ("{\"".into(), TokenKind::String),
            ("1".into(), TokenKind::Number),
        ]
    );
    assert_eq!(state, LineState::Normal);
}

#[test]
fn nix_path_ends_before_an_indented_string() {
    for path in ["./example", "dir/sub-dir/file.nix", "/tmp/test+1", "~/file"] {
        let (tokens, state) = lex("a.nix", &format!("[ {path}''"), LineState::Normal);
        assert_eq!(tokens, [("''".into(), TokenKind::String)]);
        assert_eq!(state, LineState::BlockString(0));
        let (tokens, state) = lex("a.nix", r#"  "$dir"/*)"#, state);
        assert_eq!(tokens, [(r#"  "$dir"/*)"#.into(), TokenKind::String)]);
        assert_eq!(lex("a.nix", "'' ]", state).1, LineState::Normal);
    }
    assert_eq!(
        lex("a.nix", "attrs //more''", LineState::Normal).1,
        LineState::Normal
    );
}
