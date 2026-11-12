//! `lexer` 模块的测试：各语言的 token、跨行状态、原始字符串与反引号转义、注释规则与键识别。

use super::*;
use crate::lang::detect;

fn lex(file: &str, line: &str, state: LineState) -> (Vec<(String, TokenKind)>, LineState) {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let next = highlight_line(detect(file), &chars, state, &mut tokens);
    assert_eq!(
        scan_line(detect(file), &chars, state),
        next,
        "只推算状态时结果应一致"
    );

    let out = tokens
        .iter()
        .map(|t| (chars[t.start..t.end].iter().collect(), t.kind))
        .collect();
    (out, next)
}

#[test]
fn python_basics() {
    let (tokens, next) = lex("a.py", "def foo(x): return 'hi' # 注释", LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("def".into(), TokenKind::Keyword),
            ("foo".into(), TokenKind::Function),
            ("return".into(), TokenKind::Keyword),
            ("'hi'".into(), TokenKind::String),
            ("# 注释".into(), TokenKind::Comment),
        ]
    );
    assert_eq!(next, LineState::Normal);
}

#[test]
fn config_keys_are_properties() {
    let keys = |file: &str, line: &str| -> Vec<String> {
        lex(file, line, LineState::Normal)
            .0
            .into_iter()
            .filter(|(_, k)| *k == TokenKind::Property)
            .map(|(s, _)| s)
            .collect()
    };
    assert_eq!(keys("a.yaml", "  version: 1.0"), ["version"]);
    assert_eq!(keys("a.yaml", "  - name: x"), ["name"]);
    assert_eq!(keys("a.yaml", "  '@base-ui/react':"), ["'@base-ui/react'"]);
    assert_eq!(keys("a.yaml", "  .:"), ["."]);
    assert_eq!(keys("a.toml", "name = \"winc\""), ["name"]);
    assert_eq!(keys("a.json", "  \"name\": \"winc\","), ["\"name\""]);
    assert_eq!(keys("a.css", "  color: red;"), ["color"]);
    // 冒号后面紧跟其他字符的不是键：选择器、网址、注释。
    assert!(keys("a.css", "a:hover {").is_empty());
    assert!(keys("a.yaml", "- https://example.com").is_empty());
    assert!(keys("a.yaml", "# key: value").is_empty());
    assert!(keys(".gitignore", "a=b").is_empty());
    // 值是带引号的字符串时，键之后的字符串仍然识别。
    let (tokens, _) = lex("a.yaml", "a: 'b'", LineState::Normal);
    assert_eq!(tokens[1], ("'b'".into(), TokenKind::String));
}

#[test]
fn versions_are_not_numbers() {
    let numbers = |line: &str| -> Vec<String> {
        lex("a.yaml", line, LineState::Normal)
            .0
            .into_iter()
            .filter(|(_, k)| *k == TokenKind::Number)
            .map(|(s, _)| s)
            .collect()
    };
    assert!(numbers("version: 1.4.1(@types/react@19.2.14)").is_empty());
    assert!(numbers("host: 127.0.0.1").is_empty());
    assert_eq!(numbers("retries: 3"), ["3"]);
    assert_eq!(numbers("ratio: 0.75"), ["0.75"]);
}

#[test]
fn ranges_and_method_calls_do_not_join_numbers() {
    let (tokens, _) = lex(
        "a.rs",
        "for i in 0..10 { x[1..=n]; 2.pow(3); 1.5e3; 0xFFu8 }",
        LineState::Normal,
    );
    let numbers: Vec<&str> = tokens
        .iter()
        .filter(|(_, k)| *k == TokenKind::Number)
        .map(|(s, _)| s.as_str())
        .collect();
    assert_eq!(numbers, ["0", "10", "1", "2", "3", "1.5e3", "0xFFu8"]);
    assert!(tokens.contains(&("pow".into(), TokenKind::Function)));
}

#[test]
fn apostrophes_in_markup_text_do_not_open_strings() {
    let (tokens, next) = lex(
        "a.html",
        "<p>Don't stop</p> <a title='x'>",
        LineState::Normal,
    );
    let strings: Vec<&str> = tokens
        .iter()
        .filter(|(_, k)| *k == TokenKind::String)
        .map(|(s, _)| s.as_str())
        .collect();
    assert_eq!(strings, ["'x'"]);
    assert_eq!(next, LineState::Normal);
    // 其他语言中紧跟字母的单引号仍然开始字符串（Python 的 `f'…'`），插值按代码着色。
    let (tokens, _) = lex("a.py", "s = f'{x}'", LineState::Normal);
    assert_eq!(
        tokens,
        [
            ("'{".into(), TokenKind::String),
            ("}'".into(), TokenKind::String),
        ]
    );
    // 忽略规则文件里的撇号和 `yes` 是普通文本，`#` 开头的是注释。
    let (tokens, _) = lex(".gitignore", "it's/yes", LineState::Normal);
    assert!(tokens.is_empty(), "{tokens:?}");
    let (tokens, _) = lex(".gitignore", "# build", LineState::Normal);
    assert_eq!(tokens, [("# build".into(), TokenKind::Comment)]);
}

/// 回归：TOML、YAML、INI 中的撇号被当成字符串的开头；TOML 的字面字符串按反斜杠转义
/// 处理，行内其后部分都被当作字符串。
#[test]
fn config_single_quotes_are_literal_and_apostrophes_are_text() {
    let strings = |file: &str, line: &str| -> (Vec<String>, LineState) {
        let (tokens, next) = lex(file, line, LineState::Normal);
        let strings = tokens
            .into_iter()
            .filter(|(_, k)| *k == TokenKind::String)
            .map(|(s, _)| s)
            .collect();
        (strings, next)
    };
    // 字面字符串里没有转义：末尾的反斜杠不转义收尾的单引号。
    let (found, _) = strings("a.toml", r"path = 'C:\dir\' # 目录");
    assert_eq!(found, [r"'C:\dir\'"]);
    let (found, next) = strings("a.toml", r"re = '''\d\'''");
    assert_eq!(found, [r"'''\d\'''"]);
    assert_eq!(next, LineState::Normal);
    // 值里紧跟字母的单引号是撇号。
    let (found, _) = strings("a.yaml", "title: Don't panic # 注释");
    assert!(found.is_empty(), "{found:?}");
    let (found, _) = strings("a.ini", "name = it's 'quoted'");
    assert_eq!(found, ["'quoted'"]);
    // 双引号字符串仍然有转义，转义序列单独着色。
    let (tokens, _) = lex("a.toml", r#"s = "a\"b""#, LineState::Normal);
    assert_eq!(
        tokens[1..],
        [
            ("\"a".into(), TokenKind::String),
            (r#"\""#.into(), TokenKind::Escape),
            ("b\"".into(), TokenKind::String),
        ]
    );
}

#[test]
fn types_and_macros() {
    let (tokens, _) = lex(
        "a.rs",
        "let c: Config = vec![MAX]; println!(\"x\");",
        LineState::Normal,
    );
    assert!(tokens.contains(&("Config".into(), TokenKind::Type)));
    assert!(tokens.contains(&("vec".into(), TokenKind::Function)));
    assert!(tokens.contains(&("println".into(), TokenKind::Function)));
    // 全大写的常量不是类型；`a != b` 中的 `a` 不是宏。
    assert!(!tokens.iter().any(|(s, _)| s == "MAX"));
    let (tokens, _) = lex("a.rs", "if a != b {}", LineState::Normal);
    assert!(!tokens.iter().any(|(s, _)| s == "a"));
    // Go 中以大写字母开头的是导出的函数。
    let (tokens, _) = lex("a.go", "fmt.Println(x)", LineState::Normal);
    assert!(tokens.contains(&("Println".into(), TokenKind::Function)));
}

#[test]
fn python_docstring_spans_lines() {
    let (_, next) = lex("a.py", "    \"\"\"文档开始", LineState::Normal);
    assert_eq!(next, LineState::TripleDouble);
    let (tokens, next) = lex("a.py", "结束\"\"\" + x", next);
    assert_eq!(tokens[0], ("结束\"\"\"".into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);
}

#[test]
fn rust_comments_strings_and_lifetimes() {
    let (_, next) = lex("a.rs", "let x = 1; /* start", LineState::Normal);
    assert_eq!(next, LineState::BlockComment { index: 0, depth: 1 });

    let (tokens, next) = lex("a.rs", "end */ fn f<'a>(c: char) { 'x' }", next);
    assert_eq!(tokens[0], ("end */".into(), TokenKind::Comment));
    assert!(tokens.contains(&("'x'".into(), TokenKind::String)));
    assert!(!tokens.iter().any(|(s, _)| s.contains("'a")));
    assert_eq!(next, LineState::Normal);
    // 转义的字符字面量整段着色，被转义的单引号不收尾。
    let (tokens, _) = lex("a.rs", r"let q = '\''; let b = '\\';", LineState::Normal);
    assert!(tokens.contains(&(r"'\''".into(), TokenKind::String)));
    assert!(tokens.contains(&(r"'\\'".into(), TokenKind::String)));

    let (_, next) = lex("a.rs", r#"let s = "first \"line"#, LineState::Normal);
    assert_eq!(next, LineState::DoubleQuote);
    let (tokens, next) = lex("a.rs", "second\"; x", next);
    assert_eq!(tokens[0], ("second\"".into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);
}

#[test]
fn swift_attributes_directives_and_multiline_strings() {
    let (tokens, _) = lex(
        "a.swift",
        "@MainActor func load() async throws -> String? { nil }",
        LineState::Normal,
    );
    assert_eq!(tokens[0], ("@MainActor".into(), TokenKind::Decorator));
    assert!(tokens.contains(&("func".into(), TokenKind::Keyword)));
    assert!(tokens.contains(&("load".into(), TokenKind::Function)));
    assert!(tokens.contains(&("nil".into(), TokenKind::Builtin)));

    let (tokens, _) = lex("a.swift", "#if DEBUG", LineState::Normal);
    assert_eq!(tokens.len(), 1);

    let (_, next) = lex("a.swift", "let text = \"\"\"", LineState::Normal);
    assert_eq!(next, LineState::TripleDouble);
    let (tokens, next) = lex("a.swift", "    \"\"\" + suffix // done", next);
    assert_eq!(tokens[0].1, TokenKind::String);
    assert_eq!(tokens.last().unwrap().1, TokenKind::Comment);
    assert_eq!(next, LineState::Normal);
}

#[test]
fn arkts_structs_decorators_and_components() {
    assert_eq!(detect("Index.ets").name, "ArkTS");
    let (tokens, _) = lex(
        "Index.ets",
        "@Entry @Component struct Index {",
        LineState::Normal,
    );
    assert_eq!(tokens[0], ("@Entry".into(), TokenKind::Decorator));
    assert!(tokens.contains(&("@Component".into(), TokenKind::Decorator)));

    // 状态管理装饰器沿用 `@` 装饰器的着色，`struct` 按关键字着色。
    let (tokens, _) = lex(
        "Index.ets",
        "@ComponentV2 struct Counter { @Local @Trace count: number = 0",
        LineState::Normal,
    );
    assert!(tokens.contains(&("@ComponentV2".into(), TokenKind::Decorator)));
    assert!(tokens.contains(&("@Local".into(), TokenKind::Decorator)));
    assert!(tokens.contains(&("@Trace".into(), TokenKind::Decorator)));
    assert!(tokens.contains(&("struct".into(), TokenKind::Keyword)));
    assert!(tokens.contains(&("number".into(), TokenKind::Builtin)));

    // `constructor` 在 ArkTS 里是硬关键字；TypeScript 中它是普通标识符。
    let (tokens, _) = lex("Index.ets", "  constructor() {}", LineState::Normal);
    assert!(tokens.contains(&("constructor".into(), TokenKind::Keyword)));
    let (tokens, _) = lex("a.ts", "  constructor() {}", LineState::Normal);
    assert!(!tokens.contains(&("constructor".into(), TokenKind::Keyword)));

    // 声明式界面：`build` 和属性方法按函数名着色，`Text` 这类大写开头的组件按命名习惯算类型。
    let (tokens, next) = lex(
        "Index.ets",
        "  build() { Text(this.message).fontSize(50) }",
        LineState::Normal,
    );
    assert!(tokens.contains(&("build".into(), TokenKind::Function)));
    assert!(tokens.contains(&("Text".into(), TokenKind::Type)));
    assert!(tokens.contains(&("fontSize".into(), TokenKind::Function)));
    assert!(tokens.contains(&("this".into(), TokenKind::Keyword)));
    assert_eq!(next, LineState::Normal);
}

#[test]
fn raw_strings_have_no_escapes_and_close_on_their_hashes() {
    // 反斜杠不转义：`r"C:\"` 在第二个引号处结束，之后仍按代码处理。
    let (tokens, next) = lex("a.rs", r#"let p = r"C:\"; x"#, LineState::Normal);
    assert!(tokens.contains(&(r#"r"C:\""#.into(), TokenKind::String)));
    assert_eq!(next, LineState::Normal);

    // `r#"…"#` 里的双引号不收尾，跨行时状态带到下一行。
    let (tokens, next) = lex("a.rs", r##"let j = r#"{"a": 1"##, LineState::Normal);
    assert_eq!(tokens.last().unwrap().1, TokenKind::String);
    assert_eq!(next, LineState::Raw1);
    let (tokens, next) = lex("a.rs", r##"  "b": 2}"#; y"##, next);
    assert_eq!(tokens[0], (r##"  "b": 2}"#"##.into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);

    // 名字里的 `r` 不是原始字符串的开头。
    let (_, next) = lex("a.rs", r#"let bar"#, LineState::Normal);
    assert_eq!(next, LineState::Normal);
}

#[test]
fn backslashes_escape_inside_backticks_only_where_they_do() {
    // Markdown 行内代码和 Go 原始字符串中的反斜杠不是转义，收尾的反引号仍然收尾。
    let (_, next) = lex("a.md", r"按 `\` 转义", LineState::Normal);
    assert_eq!(next, LineState::Normal);
    let (_, next) = lex("a.go", r"strings.TrimSuffix(p, `\`)", LineState::Normal);
    assert_eq!(next, LineState::Normal);
    // JS 模板字符串中的反斜杠是转义。
    let (_, next) = lex("a.js", r"const s = `a\`", LineState::Normal);
    assert_eq!(next, LineState::Backtick);
}

#[test]
fn comment_rules() {
    let (tokens, _) = lex("a.sh", "echo $# # 参数个数", LineState::Normal);
    assert_eq!(
        tokens.last(),
        Some(&("# 参数个数".into(), TokenKind::Comment))
    );

    let (tokens, _) = lex(
        "a.css",
        "a { background: url(http://x.y/z.png); }",
        LineState::Normal,
    );
    assert!(!tokens.iter().any(|(_, k)| *k == TokenKind::Comment));

    let (tokens, _) = lex("a.sql", "SELECT id FROM t", LineState::Normal);
    assert_eq!(tokens[0], ("SELECT".into(), TokenKind::Keyword));
    assert_eq!(tokens[1], ("FROM".into(), TokenKind::Keyword));
}

#[test]
fn hash_lines_and_tokens_are_ordered() {
    let (tokens, _) = lex("a.c", "  #include <stdio.h>", LineState::Normal);
    assert_eq!(
        tokens,
        vec![("#include <stdio.h>".into(), TokenKind::Decorator)]
    );

    let chars: Vec<char> = "x = foo(1, \"a\")  # c".chars().collect();
    let mut tokens = Vec::new();
    highlight_line(detect("a.py"), &chars, LineState::Normal, &mut tokens);
    assert!(tokens.windows(2).all(|w| w[0].end <= w[1].start));
    assert_eq!(tokens.last().map(|t| t.end), Some(chars.len()));
}

#[test]
fn diff_lines_are_colored_as_a_whole() {
    let kind = |line: &str| {
        let (tokens, state) = lex("a.diff", line, LineState::Normal);
        assert_eq!(state, LineState::Normal);
        tokens.first().map(|(_, kind)| *kind)
    };
    assert_eq!(kind("--- a/main.rs"), Some(TokenKind::Keyword));
    assert_eq!(kind("+++ b/main.rs"), Some(TokenKind::Keyword));
    assert_eq!(kind("@@ -1,3 +1,4 @@"), Some(TokenKind::Function));
    assert_eq!(kind("+let x = \"a // b\";"), Some(TokenKind::Inserted));
    assert_eq!(kind("-/* gone"), Some(TokenKind::Deleted));
    assert_eq!(
        kind("\\ No newline at end of file"),
        Some(TokenKind::Comment)
    );
    // 上下文行不着色，其中的引号与注释符不影响下一行。
    assert_eq!(kind(" \"unclosed /* "), None);
}

#[test]
fn prose_formats_do_not_color_code_tokens() {
    // 纯文本、表格和 Markdown 中的双引号、数字、字符字面量与函数名都按普通文字处理。
    for file in ["notes.txt", "a.csv", "a.md"] {
        let (tokens, next) = lex(
            file,
            "He said \"hi, call foo(1) at 3.5 'x'",
            LineState::Normal,
        );
        assert!(tokens.is_empty(), "{file}: {tokens:?}");
        assert_eq!(next, LineState::Normal);
    }
}

#[test]
fn markdown_code_spans_and_fences() {
    // 没有配对的反引号是普通文本，不影响下一行。
    let (tokens, next) = lex("a.md", "按下 ` 键", LineState::Normal);
    assert!(tokens.is_empty());
    assert_eq!(next, LineState::Normal);

    // 收尾的反引号串与开头等长：内部较短的反引号串属于行内代码。
    let (tokens, _) = lex("a.md", "用 `` a ` b `` 和 `c`", LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("`` a ` b ``".into(), TokenKind::String),
            ("`c`".into(), TokenKind::String),
        ]
    );

    // 没有指明语言的代码块整块按字符串着色：以 `#` 开头的行不是标题，孤立的反引号
    // 结束代码块。
    let (_, open) = lex("a.md", "```", LineState::Normal);
    assert_eq!(
        open,
        LineState::Fence {
            tilde: false,
            language: u8::MAX,
            inner: 0
        }
    );
    let (tokens, next) = lex("a.md", "# 注释 `x", open);
    assert_eq!(tokens, vec![("# 注释 `x".into(), TokenKind::String)]);
    assert_eq!(next, open);
    // 以 `~` 开头的行不结束反引号代码块。
    let (_, next) = lex("a.md", "~~~", next);
    assert_eq!(next, open);
    let (_, next) = lex("a.md", "  ```  ", next);
    assert_eq!(next, LineState::Normal);

    let (_, next) = lex("a.md", "~~~python", LineState::Normal);
    assert!(matches!(next, LineState::Fence { tilde: true, .. }));
    let (_, next) = lex("a.md", "~~~", next);
    assert_eq!(next, LineState::Normal);

    // 信息字符串中有反引号的是行内代码，不是代码块。
    let (_, next) = lex("a.md", "``` a ```", LineState::Normal);
    assert_eq!(next, LineState::Normal);
}

#[test]
fn js_template_interpolation_and_regex() {
    // 嵌套的模板字符串不会提前结束外层的字符串，插值内部按代码着色。
    let (tokens, next) = lex("a.js", "s = `a ${b ? `c` : 'd'} e`; x", LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("`a ${".into(), TokenKind::String),
            ("`c`".into(), TokenKind::String),
            ("'d'".into(), TokenKind::String),
            ("} e`".into(), TokenKind::String),
        ]
    );
    assert_eq!(next, LineState::Normal);

    // 本行内没有闭合的插值按字符串内容处理，字符串跨行时状态带到下一行。
    let (_, next) = lex("a.js", "s = `a ${", LineState::Normal);
    assert_eq!(next, LineState::Backtick);
    let (tokens, next) = lex("a.js", "b} c` + f(1)", LineState::Backtick);
    assert_eq!(tokens[0], ("b} c`".into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);

    // 正则中的反引号、引号和字符类中的斜杠不影响之后的代码。
    let (tokens, next) = lex("a.js", "s.replace(/`'[/]/g, '')", LineState::Normal);
    assert_eq!(tokens[1], ("/`'[/]/g".into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);
    let (tokens, _) = lex("a.js", "return /a\\/b/.test(s)", LineState::Normal);
    assert_eq!(tokens[1], ("/a\\/b/".into(), TokenKind::String));

    // 值之后的斜杠是除号。
    for line in ["x = a / b / c", "x = f(a) / 2 / g(b)", "x = n[0] / 2 / 3"] {
        let (tokens, _) = lex("a.js", line, LineState::Normal);
        assert!(
            !tokens.iter().any(|(_, k)| *k == TokenKind::String),
            "{line}"
        );
    }

    // JSX：结束标签不是正则，正文中的撇号不开始字符串。
    let (tokens, _) = lex("a.jsx", "<li><a>Don't</a></li>", LineState::Normal);
    assert!(
        !tokens.iter().any(|(_, k)| *k == TokenKind::String),
        "{tokens:?}"
    );
}

#[test]
fn single_quote_rules_follow_each_language() {
    // Groovy 的单引号界定字符串。
    let (tokens, _) = lex(
        "build.gradle",
        "implementation 'a:b:1.0'",
        LineState::Normal,
    );
    assert_eq!(tokens, vec![("'a:b:1.0'".into(), TokenKind::String)]);

    // Shell 单引号内的反斜杠不转义，字符串在第二个单引号处结束。
    let (tokens, _) = lex("a.sh", r"echo 'C:\dir\' && ls # c", LineState::Normal);
    assert_eq!(tokens[1], (r"'C:\dir\'".into(), TokenKind::String));
    assert_eq!(tokens[2], ("# c".into(), TokenKind::Comment));
}

#[test]
fn ini_accepts_both_comment_markers() {
    for line in ["; it's a comment", "# it's a comment"] {
        let (tokens, _) = lex("a.ini", line, LineState::Normal);
        assert_eq!(tokens, vec![(line.into(), TokenKind::Comment)]);
    }
    // 值中间紧跟其他字符的 `;` 不是注释。
    let (tokens, _) = lex("a.ini", "path = a;b", LineState::Normal);
    assert!(!tokens.iter().any(|(_, k)| *k == TokenKind::Comment));
    assert_eq!(detect("a.ini").comment_prefix(), Some("# "));
}

// 代码和双引号字符串中的变量都单独着色；单引号字符串里的 `$` 是字面内容。
#[test]
fn shell_variables_in_code_and_strings() {
    let (tokens, _) = lex(
        "a.sh",
        "echo $HOME ${PATH} $1 $? \"$USER/x\" '$NO' $(pwd)",
        LineState::Normal,
    );
    assert_eq!(
        tokens,
        vec![
            ("echo".into(), TokenKind::Builtin),
            ("$HOME".into(), TokenKind::Variable),
            ("${PATH}".into(), TokenKind::Variable),
            ("$1".into(), TokenKind::Variable),
            ("$?".into(), TokenKind::Variable),
            ("\"".into(), TokenKind::String),
            ("$USER".into(), TokenKind::Variable),
            ("/x\"".into(), TokenKind::String),
            ("'$NO'".into(), TokenKind::String),
            ("pwd".into(), TokenKind::Builtin),
        ]
    );
}

// 单双引号字符串都可以跨行：未结束的状态带到下一行，下一行从字符串内部继续。
#[test]
fn shell_strings_span_lines() {
    let (_, next) = lex("a.sh", "awk '{", LineState::Normal);
    assert_eq!(next, LineState::SingleQuote);
    let (tokens, next) = lex("a.sh", "  print \"$1\" }' file", next);
    assert_eq!(tokens[0], ("  print \"$1\" }'".into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);

    let (_, next) = lex("a.sh", "echo \"a", LineState::Normal);
    assert_eq!(next, LineState::DoubleQuote);
}

#[test]
fn shell_heredocs() {
    let (tokens, next) = lex("a.sh", "cat <<EOF | grep x # c", LineState::Normal);
    assert_eq!(tokens[0], ("<<EOF".into(), TokenKind::Keyword));
    assert_eq!(tokens.last(), Some(&("# c".into(), TokenKind::Comment)));
    let body = next;
    assert!(matches!(body, LineState::Heredoc(_)));

    // 内容中的撇号与反引号不影响之后的行，变量单独着色。
    let (tokens, next) = lex("a.sh", "Don't `run` $HOME", body);
    assert_eq!(
        tokens,
        vec![
            ("Don't `run` ".into(), TokenKind::String),
            ("$HOME".into(), TokenKind::Variable),
        ]
    );
    assert_eq!(next, body);
    // 只有与结束标记完全相同的行才结束 here 文档。
    let (_, next) = lex("a.sh", " EOF", body);
    assert_eq!(next, body);
    let (tokens, next) = lex("a.sh", "EOF", body);
    assert_eq!(tokens, vec![("EOF".into(), TokenKind::Keyword)]);
    assert_eq!(next, LineState::Normal);

    // 带引号的结束标记：内容中不展开变量；`<<-` 允许结束行用制表符缩进。
    let (_, body) = lex("a.sh", "cat <<-'END'", LineState::Normal);
    let (tokens, _) = lex("a.sh", "$HOME", body);
    assert_eq!(tokens, vec![("$HOME".into(), TokenKind::String)]);
    let (_, next) = lex("a.sh", "\t\tEND", body);
    assert_eq!(next, LineState::Normal);

    // here 字符串与算术左移不是 here 文档。
    for line in ["cat <<< \"$x\"", "echo $((1 << 2))"] {
        let (_, next) = lex("a.sh", line, LineState::Normal);
        assert_eq!(next, LineState::Normal, "{line}");
    }
}

// 行首的目标名是键，`$(…)`、`$@` 等引用是变量，`$(shell …)` 只把函数名按函数着色。
#[test]
fn makefile_targets_variables_and_directives() {
    let (tokens, _) = lex("Makefile", "all: $(OUT) build", LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("all".into(), TokenKind::Property),
            ("$(OUT)".into(), TokenKind::Variable),
        ]
    );
    let (tokens, _) = lex(
        "Makefile",
        "\t$(CC) -o $@ $^ $(SRC:.c=.o) $$HOME",
        LineState::Normal,
    );
    assert_eq!(
        tokens,
        vec![
            ("$(CC)".into(), TokenKind::Variable),
            ("$@".into(), TokenKind::Variable),
            ("$^".into(), TokenKind::Variable),
            ("$(SRC:.c=.o)".into(), TokenKind::Variable),
            ("$$HOME".into(), TokenKind::Variable),
        ]
    );
    let (tokens, _) = lex("Makefile", "FILES := $(shell ls)", LineState::Normal);
    assert_eq!(tokens, vec![("$(shell".into(), TokenKind::Function)]);
    let (tokens, _) = lex("Makefile", "ifeq ($(OS),Darwin)", LineState::Normal);
    assert_eq!(tokens[0], ("ifeq".into(), TokenKind::Keyword));
    let (tokens, _) = lex("Makefile", "%.o: %.c", LineState::Normal);
    assert_eq!(tokens, vec![("%.o".into(), TokenKind::Property)]);
}

// 跨行时收尾的定界符必须完全一致；C 不支持原始字符串，按普通字符串处理。
#[test]
fn cpp_raw_strings() {
    let (tokens, next) = lex(
        "a.cpp",
        r#"auto s = R"(say "hi")"; f();"#,
        LineState::Normal,
    );
    assert_eq!(tokens[1], (r#"R"(say "hi")""#.into(), TokenKind::String));
    assert_eq!(tokens[2], ("f".into(), TokenKind::Function));
    assert_eq!(next, LineState::Normal);

    // 跨行时只有定界符相同的收尾才结束字符串。
    let (_, next) = lex("a.cpp", r#"auto s = u8R"sql(SELECT ")""#, LineState::Normal);
    assert!(matches!(next, LineState::CppRaw(_)));
    let (_, next) = lex("a.cpp", r#"  )" )x""#, next);
    assert!(matches!(next, LineState::CppRaw(_)));
    let (tokens, next) = lex("a.cpp", r#")sql"; int x;"#, next);
    assert_eq!(tokens[0], (r#")sql""#.into(), TokenKind::String));
    assert_eq!(next, LineState::Normal);

    // C 没有原始字符串。
    let (tokens, _) = lex("a.c", r#"R"(a)""#, LineState::Normal);
    assert_eq!(tokens, vec![(r#""(a)""#.into(), TokenKind::String)]);
}

#[test]
fn shell_quotes_nest_inside_substitutions() {
    // 代码中转义的引号不开始字符串。
    let (tokens, next) = lex("a.sh", r#"sed "s/a/b/" \" x"#, LineState::Normal);
    assert_eq!(tokens, vec![(r#""s/a/b/""#.into(), TokenKind::String)]);
    assert_eq!(next, LineState::Normal);

    // 命令替换中嵌套的引号不结束外层的字符串，命令替换内部按代码着色。
    let (tokens, next) = lex("a.sh", r#"v="$(cat "$f") end""#, LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("\"$(".into(), TokenKind::String),
            ("\"".into(), TokenKind::String),
            ("$f".into(), TokenKind::Variable),
            ("\"".into(), TokenKind::String),
            (") end\"".into(), TokenKind::String),
        ]
    );
    assert_eq!(next, LineState::Normal);

    // `${…}` 中嵌套的引号和花括号不提前结束变量。
    let (tokens, next) = lex("a.sh", r#"echo "${d#"${t}"/}" x"#, LineState::Normal);
    assert_eq!(tokens[2], (r#"${d#"${t}"/}"#.into(), TokenKind::Variable));
    assert_eq!(next, LineState::Normal);
}

/// 返回 `file` 中 `line` 里类别为 `kind` 的 token 文本。
fn texts(file: &str, line: &str, state: LineState, kind: TokenKind) -> Vec<String> {
    lex(file, line, state)
        .0
        .into_iter()
        .filter(|(_, k)| *k == kind)
        .map(|(s, _)| s)
        .collect()
}

#[test]
fn markup_tags_attributes_and_text() {
    let line = r#"<div class="a" id=x>Tom &amp; "Jerry" don't</div>"#;
    let (tokens, next) = lex("a.html", line, LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("div".into(), TokenKind::Tag),
            ("class".into(), TokenKind::Attribute),
            ("\"a\"".into(), TokenKind::String),
            ("id".into(), TokenKind::Attribute),
            ("x".into(), TokenKind::String),
            ("&amp;".into(), TokenKind::Escape),
            ("div".into(), TokenKind::Tag),
        ]
    );
    assert_eq!(next, LineState::Normal);

    // 属性跨行时状态停在标签内部。
    let (_, next) = lex("a.html", "<a", LineState::Normal);
    assert_eq!(next, LineState::Tag(0));
    let (tokens, next) = lex("a.html", "  href=\"/\">x</a>", next);
    assert_eq!(tokens[0], ("href".into(), TokenKind::Attribute));
    assert_eq!(next, LineState::Normal);

    // XML 中没有内嵌脚本。
    let (_, next) = lex("a.xml", "<script>", LineState::Normal);
    assert_eq!(next, LineState::Normal);
}

#[test]
fn html_embeds_script_and_style() {
    let (_, next) = lex("a.vue", "<script setup lang=\"ts\">", LineState::Normal);
    assert!(matches!(next, LineState::Embedded { script: true, .. }));
    let (tokens, next) = lex("a.vue", "const s = `a ${b}`; /* c", next);
    assert_eq!(tokens[0], ("const".into(), TokenKind::Keyword));
    assert_eq!(
        next,
        LineState::Embedded {
            script: true,
            inner: 0x41
        }
    );
    let (tokens, next) = lex("a.vue", "*/ </script><p>", next);
    assert_eq!(
        tokens,
        vec![
            ("*/".into(), TokenKind::Comment),
            ("script".into(), TokenKind::Tag),
            ("p".into(), TokenKind::Tag),
        ]
    );
    assert_eq!(next, LineState::Normal);

    let (tokens, _) = lex(
        "a.html",
        "<style>a { color: #fff }</style>",
        LineState::Normal,
    );
    assert!(tokens.contains(&("color".into(), TokenKind::Property)));
    assert!(tokens.contains(&("#fff".into(), TokenKind::Number)));
}

#[test]
fn markdown_blocks_and_inlines() {
    let headings = |line: &str| texts("a.md", line, LineState::Normal, TokenKind::Heading);
    assert_eq!(headings("## Title"), ["## Title"]);
    // `#` 之后必须是空白，最多 6 个。
    assert!(headings("#hashtag").is_empty());
    assert!(headings("####### seven").is_empty());

    let (tokens, _) = lex(
        "a.md",
        "> - [x] *em* **bold** [a](http://b) <br> https://c.d.",
        LineState::Normal,
    );
    assert_eq!(
        tokens,
        vec![
            (">".into(), TokenKind::Keyword),
            ("-".into(), TokenKind::Keyword),
            ("[x]".into(), TokenKind::Keyword),
            ("*em*".into(), TokenKind::Emphasis),
            ("**bold**".into(), TokenKind::Strong),
            ("[a]".into(), TokenKind::Link),
            ("(http://b)".into(), TokenKind::String),
            ("<br>".into(), TokenKind::Tag),
            ("https://c.d".into(), TokenKind::Link),
        ]
    );
    // 单词内部的 `_` 与两侧有空格的 `*` 不是强调。
    let (tokens, _) = lex("a.md", "snake_case_name and 2 * 3 * 4", LineState::Normal);
    assert!(tokens.is_empty(), "{tokens:?}");
    assert_eq!(
        texts("a.md", "---", LineState::Normal, TokenKind::Keyword),
        ["---"]
    );
    // HTML 注释可以跨行。
    let (_, next) = lex("a.md", "<!-- a", LineState::Normal);
    assert_eq!(next, LineState::BlockComment { index: 0, depth: 1 });
}

#[test]
fn config_sections_keys_and_literals() {
    let heading = |file: &str, line: &str| texts(file, line, LineState::Normal, TokenKind::Heading);
    assert_eq!(heading("a.toml", "[package]"), ["[package]"]);
    assert_eq!(heading("a.toml", "[[bin]] # c"), ["[[bin]]"]);
    assert_eq!(heading("a.ini", "[core]"), ["[core]"]);
    // 多行数组中以 `[` 开头的元素不是表头。
    assert!(heading("a.toml", "  [1, 2],").is_empty());

    let builtin = |file: &str, line: &str| texts(file, line, LineState::Normal, TokenKind::Builtin);
    assert_eq!(builtin("a.toml", "x = inf"), ["inf"]);
    assert!(builtin("a.toml", "x = yes").is_empty());
    assert_eq!(builtin("a.yaml", "x: True"), ["True"]);

    // JSON 中紧跟 `:` 的字符串是键，不限于行首。
    assert_eq!(
        texts(
            "a.json",
            r#"[{"a": 1, "b": "c"}]"#,
            LineState::Normal,
            TokenKind::Property
        ),
        ["\"a\"", "\"b\""]
    );
    // SQL 中双引号与反引号括起的是标识符；单引号字符串中 `''` 是一个引号、反斜杠不转义。
    let (tokens, _) = lex(
        "a.sql",
        r#"SELECT "c", `t` FROM x WHERE a = 'it''s' AND b = 'C:\'"#,
        LineState::Normal,
    );
    assert!(tokens.contains(&("\"c\"".into(), TokenKind::Property)));
    assert!(tokens.contains(&("`t`".into(), TokenKind::Property)));
    assert!(tokens.contains(&("'it''s'".into(), TokenKind::String)));
    assert!(tokens.contains(&(r"'C:\'".into(), TokenKind::String)));
}

#[test]
fn stylesheet_selectors_and_declarations() {
    let (tokens, _) = lex(
        "a.css",
        ".a, #b:hover::before, div > p {",
        LineState::Normal,
    );
    assert_eq!(
        tokens,
        vec![
            (".a".into(), TokenKind::Attribute),
            ("#b".into(), TokenKind::Attribute),
            (":hover".into(), TokenKind::Builtin),
            ("::before".into(), TokenKind::Builtin),
            ("div".into(), TokenKind::Tag),
            ("p".into(), TokenKind::Tag),
        ]
    );
    let (tokens, _) = lex("a.css", "a { margin:0; color: #1e90ff }", LineState::Normal);
    assert!(tokens.contains(&("margin".into(), TokenKind::Property)));
    assert!(tokens.contains(&("color".into(), TokenKind::Property)));
    assert!(tokens.contains(&("#1e90ff".into(), TokenKind::Number)));
    // `url(http://…)` 中的 `http` 不是属性名。
    let (tokens, _) = lex(
        "a.css",
        "  background: url(http://x/y.png);",
        LineState::Normal,
    );
    assert!(!tokens.contains(&("http".into(), TokenKind::Property)));
}

#[test]
fn nested_comments_attributes_and_escapes() {
    let (tokens, next) = lex("a.rs", "/* a /* b */ c */ fn f() {}", LineState::Normal);
    assert_eq!(tokens[0], ("/* a /* b */ c */".into(), TokenKind::Comment));
    assert_eq!(next, LineState::Normal);
    let (_, next) = lex("a.rs", "/* a /* b */", LineState::Normal);
    assert_eq!(next, LineState::BlockComment { index: 0, depth: 1 });
    // C 的块注释不嵌套。
    let (tokens, _) = lex("a.c", "/* a /* b */ int x;", LineState::Normal);
    assert_eq!(tokens[0], ("/* a /* b */".into(), TokenKind::Comment));

    assert_eq!(
        texts(
            "a.rs",
            "#[derive(Debug)]",
            LineState::Normal,
            TokenKind::Decorator
        ),
        ["#[derive"]
    );
    assert_eq!(
        texts(
            "a.rs",
            r#"let s = "a\u{1F600}\n\x41";"#,
            LineState::Normal,
            TokenKind::Escape
        ),
        [r"\u{1F600}", r"\n", r"\x41"]
    );
    // 原始字符串中没有转义序列。
    assert!(texts("a.py", r#"r"\d+""#, LineState::Normal, TokenKind::Escape).is_empty());
}

#[test]
fn string_interpolation_by_language() {
    let strings = |file: &str, line: &str| texts(file, line, LineState::Normal, TokenKind::String);
    assert_eq!(strings("a.py", r#"f"a{x}b {{c}}""#), ["\"a{", "}b {{c}}\""]);
    assert_eq!(strings("a.py", r#""a{x}""#), ["\"a{x}\""]);
    assert_eq!(strings("a.swift", r#""a\(x)b""#), ["\"a\\(", ")b\""]);
    assert_eq!(
        strings("a.rb", r##""a#{x}b" 'c#{d}'"##),
        ["\"a#{", "}b\"", "'c#{d}'"]
    );
    assert_eq!(
        strings("a.cs", r#"$"a{x}" + "b{y}""#),
        ["\"a{", "}\"", "\"b{y}\""]
    );
    assert_eq!(
        texts(
            "a.kt",
            r#""Hi $name""#,
            LineState::Normal,
            TokenKind::Variable
        ),
        ["$name"]
    );
    // Kotlin 代码中的 `$` 不是变量。
    assert!(texts("a.kt", "val $x = 1", LineState::Normal, TokenKind::Variable).is_empty());
    assert_eq!(
        texts(
            "a.ps1",
            r#""Hi $env:USER""#,
            LineState::Normal,
            TokenKind::Variable
        ),
        ["$env:USER"]
    );
}

#[test]
fn language_specific_strings_and_comments() {
    // Lua 的长字符串与长注释。
    let (_, next) = lex("a.lua", "local s = [==[a", LineState::Normal);
    assert_eq!(next, LineState::LongString(2));
    let (_, next) = lex("a.lua", "]] ]==] --[[ c", next);
    assert_eq!(next, LineState::LongComment(0));
    // CMake 的括号注释。
    let (_, next) = lex("a.cmake", "#[=====[.rst:", LineState::Normal);
    assert_eq!(next, LineState::LongComment(5));
    // Ruby、Perl 的块注释只在行首开始和结束。
    let (_, next) = lex("a.rb", "=begin", LineState::Normal);
    assert_eq!(next, LineState::BlockComment { index: 0, depth: 1 });
    let (_, next) = lex("a.rb", " =end", next);
    assert_eq!(next, LineState::BlockComment { index: 0, depth: 1 });
    let (_, next) = lex("a.rb", "=end", next);
    assert_eq!(next, LineState::Normal);
    // Ruby 的 here 文档：结束标记之后可以紧跟方法调用；`a << b` 是运算符。
    let (_, next) = lex("a.rb", "sql = <<~SQL.strip", LineState::Normal);
    assert!(matches!(next, LineState::Heredoc(_)));
    let (_, next) = lex("a.rb", "  SQL", next);
    assert_eq!(next, LineState::Normal);
    let (_, next) = lex("a.rb", "arr << item", LineState::Normal);
    assert_eq!(next, LineState::Normal);
    // Ruby 的百分号字面量可以跨行，内部括号配对。
    let (_, next) = lex("a.rb", "re = %r{a{2}", LineState::Normal);
    assert_eq!(
        next,
        LineState::Percent {
            open: b'{',
            depth: 1
        }
    );
    let (_, next) = lex("a.rb", "}x", next);
    assert_eq!(next, LineState::Normal);
    assert_eq!(
        texts(
            "a.rb",
            "f(:ok, :`, @a, $`)",
            LineState::Normal,
            TokenKind::Variable
        ),
        [":ok", ":`", "@a", "$`"]
    );
    // PHP 的 here 文档：结束行可以缩进，之后紧跟 `;`。
    let (_, next) = lex("a.php", "echo <<<EOT", LineState::Normal);
    let (_, next) = lex("a.php", "  EOT;", next);
    assert_eq!(next, LineState::Normal);
    // PowerShell 的转义字符是反引号，here 字符串可以跨行。
    assert_eq!(
        texts("a.ps1", r#""a`"b""#, LineState::Normal, TokenKind::Escape),
        ["`\""]
    );
    let (_, next) = lex("a.ps1", "$h = @\"", LineState::Normal);
    assert_eq!(next, LineState::BlockString(0));
    // C# 的逐字字符串：`""` 是引号，反斜杠不转义。
    assert_eq!(
        texts(
            "a.cs",
            r#"@"C:\d""q" x"#,
            LineState::Normal,
            TokenKind::String
        ),
        [r#"@"C:\d""q""#]
    );
    // Zig 的多行字符串行、Haskell 的嵌套块注释与批处理的变量。
    assert_eq!(
        texts("a.zig", r"    \\line", LineState::Normal, TokenKind::String),
        [r"\\line"]
    );
    let (_, next) = lex("a.hs", "{- a {- b -}", LineState::Normal);
    assert_eq!(next, LineState::BlockComment { index: 0, depth: 1 });
    assert_eq!(
        texts(
            "a.bat",
            "echo %~dp0 %%i !x! %PATH%",
            LineState::Normal,
            TokenKind::Variable
        ),
        ["%~dp0", "%%i", "!x!", "%PATH%"]
    );
    // LaTeX 的命令、`\%` 转义和数学公式。
    let (tokens, _) = lex("a.tex", r"\emph{a} 50\% $x$ % c", LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            (r"\emph".into(), TokenKind::Keyword),
            ("$x$".into(), TokenKind::String),
            ("% c".into(), TokenKind::Comment),
        ]
    );
}

/// 回归：算术展开中的左移被当成 here 文档，文件剩下的部分都按字符串着色。
#[test]
fn shell_arithmetic_shift_is_not_a_heredoc() {
    for line in ["echo $((a<<b))", "(( x <<= 2 ))", "y=$(( (a << b) + 1 ))"] {
        let (_, next) = lex("a.sh", line, LineState::Normal);
        assert_eq!(next, LineState::Normal, "{line}");
    }
    // 算术括号之后的 here 文档仍然识别。
    let (_, next) = lex("a.sh", "echo $((1+2)); cat <<EOF", LineState::Normal);
    assert!(matches!(next, LineState::Heredoc(_)));
}

/// 回归：跨行声明值中以逗号结尾的续行被当成选择器。
#[test]
fn stylesheet_values_span_lines() {
    let (_, next) = lex("a.css", "  font-family:", LineState::Normal);
    assert_eq!(next, LineState::StyleValue);
    let (tokens, next) = lex("a.css", "    -apple-system, Helvetica,", next);
    assert!(tokens.is_empty(), "{tokens:?}");
    assert_eq!(next, LineState::StyleValue);
    let (tokens, next) = lex("a.css", "    sans-serif;", next);
    assert!(tokens.is_empty(), "{tokens:?}");
    assert_eq!(next, LineState::Normal);
    // 最后一条声明没有分号时，值延续到收尾的 `}` 为止。
    let (_, next) = lex("a.css", "  color: red", LineState::Normal);
    let (_, next) = lex("a.css", "}", next);
    assert_eq!(next, LineState::Normal);
    // 选择器列表的续行仍是选择器。
    let (tokens, _) = lex("a.css", ".a,", LineState::Normal);
    assert_eq!(tokens, [(".a".into(), TokenKind::Attribute)]);
    // HTML 中内嵌的样式同样延续。
    let (_, next) = lex("a.html", "<style>a { font:", LineState::Normal);
    assert_eq!(
        next,
        LineState::Embedded {
            script: false,
            inner: 2
        }
    );
}

#[test]
fn comment_prefixes_end_with_a_space() {
    for (file, prefix) in [
        ("a.rs", "// "),
        ("a.py", "# "),
        ("a.sql", "-- "),
        ("a.clj", "; "),
        ("a.erl", "% "),
        ("a.tex", "% "),
        ("a.bat", ":: "),
    ] {
        assert_eq!(detect(file).comment_prefix(), Some(prefix), "{file}");
    }
}

/// 回归：`if (…)` 之后的正则被当成除号，其中的引号开始字符串。
#[test]
fn regex_after_condition_parens() {
    let (tokens, _) = lex(
        "a.js",
        r#"if (ok) /a"b/.test(s); x = 1;"#,
        LineState::Normal,
    );
    assert!(tokens.contains(&(r#"/a"b/"#.into(), TokenKind::String)));
    assert!(tokens.contains(&("1".into(), TokenKind::Number)));
    // 其他右括号之后仍然是除号。
    let (tokens, _) = lex("a.js", "x = f(a) / 2 / g(b)", LineState::Normal);
    assert!(!tokens.iter().any(|(_, k)| *k == TokenKind::String));
}

/// 回归：Python 的矩阵乘法 `a @ b` 中的 `@` 被当成装饰器。
#[test]
fn bare_at_is_an_operator() {
    let (tokens, _) = lex("a.py", "c = a @ b", LineState::Normal);
    assert!(tokens.is_empty(), "{tokens:?}");
    let (tokens, _) = lex("a.py", "@cache", LineState::Normal);
    assert_eq!(tokens, [("@cache".into(), TokenKind::Decorator)]);
}

#[test]
fn markdown_fences_highlight_by_language() {
    let (_, next) = lex("a.md", "```rust", LineState::Normal);
    let (tokens, next) = lex("a.md", "fn main() { /* a", next);
    assert_eq!(
        tokens,
        vec![
            ("fn".into(), TokenKind::Keyword),
            ("main".into(), TokenKind::Function),
            ("/* a".into(), TokenKind::Comment),
        ]
    );
    // 代码中的块注释跨行，代码块本身不结束。
    let (tokens, next) = lex("a.md", "b */ let x = 1; }", next);
    assert_eq!(tokens[0], ("b */".into(), TokenKind::Comment));
    let (_, next) = lex("a.md", "```", next);
    assert_eq!(next, LineState::Normal);

    // 扩展名、别名与带属性的信息字符串。
    for info in [
        "```py",
        "~~~python",
        "```js {1,3}",
        "```golang",
        "```c++",
        "```yml",
    ] {
        let (_, next) = lex("a.md", info, LineState::Normal);
        assert!(
            matches!(next, LineState::Fence { language, .. } if language != u8::MAX),
            "{info}"
        );
    }
    // HTML 代码块中按标签着色。
    let (_, next) = lex("a.md", "```html", LineState::Normal);
    let (tokens, _) = lex("a.md", "<p class=\"a\">x</p>", next);
    assert_eq!(tokens[0], ("p".into(), TokenKind::Tag));
}

#[test]
fn markdown_tables_and_setext_underlines() {
    let (tokens, _) = lex("a.md", "| a | `b|c` | **d** |", LineState::Normal);
    assert_eq!(
        tokens,
        vec![
            ("|".into(), TokenKind::Keyword),
            ("|".into(), TokenKind::Keyword),
            ("`b|c`".into(), TokenKind::String),
            ("|".into(), TokenKind::Keyword),
            ("**d**".into(), TokenKind::Strong),
            ("|".into(), TokenKind::Keyword),
        ]
    );
    assert_eq!(
        texts("a.md", "|:--|--:|", LineState::Normal, TokenKind::Keyword),
        ["|:--|--:|"]
    );
    assert_eq!(
        texts("a.md", "=====  ", LineState::Normal, TokenKind::Heading),
        ["=====  "]
    );
    // 正文中的竖线不是列分隔符。
    let (tokens, _) = lex("a.md", "a | b", LineState::Normal);
    assert!(tokens.is_empty(), "{tokens:?}");
}

#[test]
fn yaml_block_scalars() {
    let (_, next) = lex("a.yaml", "    run: |  # 脚本", LineState::Normal);
    assert_eq!(next, LineState::BlockScalar(4));
    let (tokens, next) = lex("a.yaml", "      echo \"it's\" # c", next);
    assert_eq!(
        tokens,
        vec![("echo \"it's\" # c".into(), TokenKind::String)]
    );
    let (tokens, next) = lex("a.yaml", "      key: value", next);
    assert_eq!(tokens, vec![("key: value".into(), TokenKind::String)]);
    // 空行属于内容；与键同列的下一个键结束块标量。
    let (_, next) = lex("a.yaml", "", next);
    assert_eq!(next, LineState::BlockScalar(4));
    let (tokens, next) = lex("a.yaml", "    env: x", next);
    assert_eq!(tokens[0], ("env".into(), TokenKind::Property));
    assert_eq!(next, LineState::Normal);

    // 列表项中的键、直接写在列表项上的块标量，以及带指示符的写法。
    let (_, next) = lex("a.yaml", "  - run: >-", LineState::Normal);
    assert_eq!(next, LineState::BlockScalar(4));
    let (_, next) = lex("a.yaml", "  - |2", LineState::Normal);
    assert_eq!(next, LineState::BlockScalar(2));
    // 值中含 `|` 的不是块标量。
    let (_, next) = lex("a.yaml", "a: b|", LineState::Normal);
    assert_eq!(next, LineState::Normal);
}

#[test]
fn jsx_prose_quotes() {
    let (tokens, _) = lex(
        "a.jsx",
        r#"<p>He said "hi" and 'yo</p>; y = 2;"#,
        LineState::Normal,
    );
    assert!(
        !tokens.iter().any(|(_, k)| *k == TokenKind::String),
        "{tokens:?}"
    );
    assert!(tokens.contains(&("2".into(), TokenKind::Number)));
    // 关键字和标点之后仍然开始字符串。
    for line in [
        r#"import x from "y";"#,
        r#"return "s";"#,
        r#"case "k":"#,
        r#"f("x")"#,
        r#"const o = { a: "b" }"#,
        r#"declare module "m" {}"#,
    ] {
        let lang = if line.starts_with("declare") {
            "a.ts"
        } else {
            "a.js"
        };
        assert!(
            lex(lang, line, LineState::Normal)
                .0
                .iter()
                .any(|(_, k)| *k == TokenKind::String),
            "{line}"
        );
    }
}
