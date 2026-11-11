//! 用例文件与录制文件格式的测试。

use super::*;

/// 一个录制文件：两个用例，覆盖转义、三种寄存器类型与空行。
const RECORDED: &str = r#"# 注释
vim 9.1.1752

case delete-into-register
text "abc def"
cursor 0 1
keys "a2x
> text "a def"
> cursor 0 1
> reg " v "bc"
> reg a v "bc"

case escapes
text "\tquote \" slash \\ bell \u{7}"
text ""
text "宽字符 e\u{301}"
cursor 2 4
keys <C-v>jly<Space>
> text "x"
> cursor 0 0
> reg " b2 "ab" "cd"
> reg 1 V "line\r" ""
"#;

#[test]
fn a_recording_parses_into_cases_and_results() {
    let file = parse(RECORDED).unwrap();
    assert_eq!(file.vim.as_deref(), Some("9.1.1752"));
    assert_eq!(file.cases.len(), 2);

    let first = &file.cases[0];
    assert_eq!(first.name, "delete-into-register");
    assert_eq!(first.text, ["abc def"]);
    assert_eq!(first.cursor, Position { line: 0, col: 1 });
    assert_eq!(first.keys, "\"a2x");
    let result = first.result.as_ref().unwrap();
    assert_eq!(result.text, ["a def"]);
    assert_eq!(result.cursor, Position { line: 0, col: 1 });
    assert_eq!(
        result.registers,
        [
            Register {
                name: '"',
                kind: RegisterKind::Charwise,
                lines: vec!["bc".into()],
            },
            Register {
                name: 'a',
                kind: RegisterKind::Charwise,
                lines: vec!["bc".into()],
            },
        ]
    );

    let second = &file.cases[1];
    assert_eq!(
        second.text,
        ["\tquote \" slash \\ bell \u{7}", "", "宽字符 e\u{301}"]
    );
    assert_eq!(second.keys, "<C-v>jly<Space>");
    let registers = &second.result.as_ref().unwrap().registers;
    assert_eq!(registers[0].kind, RegisterKind::Blockwise(2));
    assert_eq!(registers[0].lines, ["ab", "cd"]);
    assert_eq!(registers[1].kind, RegisterKind::Linewise);
    assert_eq!(registers[1].lines, ["line\r", ""]);
}

#[test]
fn writing_and_parsing_round_trip() {
    let file = parse(RECORDED).unwrap();
    let written = write(&file);
    assert_eq!(parse(&written).unwrap(), file);
    // 写出的文本是规范形式：再次解析、写出后不变。
    assert_eq!(write(&parse(&written).unwrap()), written);
    assert!(written.starts_with("vim 9.1.1752\n\ncase delete-into-register\n"));
    assert!(written.contains("text \"\\tquote \\\" slash \\\\ bell \\u{7}\"\n"));
    assert!(written.contains("> reg \" b2 \"ab\" \"cd\"\n"));

    // 行尾是 CRLF 的文件解析结果相同。
    assert_eq!(parse(&RECORDED.replace('\n', "\r\n")).unwrap(), file);
}

#[test]
fn a_case_file_has_inputs_only_and_defaults() {
    let file = parse("case empty\nkeys x\n\ncase second\ntext \"a\"\nkeys dd").unwrap();
    assert_eq!(file.vim, None);
    assert_eq!(file.cases.len(), 2);
    // 没有 text 行时文本是一个空行，光标在开头。
    assert_eq!(file.cases[0].text, [""]);
    assert_eq!(file.cases[0].cursor, Position::default());
    assert!(file.cases.iter().all(|c| c.result.is_none()));
    let written = "\ncase empty\ntext \"\"\ncursor 0 0\nkeys x\n\
                   \ncase second\ntext \"a\"\ncursor 0 0\nkeys dd\n";
    assert_eq!(write(&file), written);
}

#[test]
fn malformed_files_are_rejected_with_the_line_number() {
    for (source, line) in [
        ("text \"a\"", 1),
        ("case a\ncase a", 2),
        ("case a b", 1),
        ("case", 1),
        ("case a\nbogus 1", 2),
        ("case a\ntext abc", 2),
        ("case a\ntext \"abc", 2),
        ("case a\ntext \"\\q\"", 2),
        ("case a\ntext \"\\u{110000}\"", 2),
        ("case a\ntext \"\\u{41\"", 2),
        ("case a\ncursor 1", 2),
        ("case a\ncursor 1 2 3", 2),
        ("case a\ncursor -1 0", 2),
        ("case a\nkeys", 2),
        ("case a\nkeys x ", 2),
        ("case a\nkeys x\nkeys y", 3),
        ("case a\nkeys x\ntext \"late\"", 3),
        ("case a\n> text \"early\"", 2),
        ("case a\nkeys x\n> reg a z \"x\"", 3),
        ("case a\nkeys x\n> reg a", 3),
        ("case a\nkeys x\n> reg a v \"x\" y", 3),
        ("case a\nkeys x\n> mode n", 3),
        ("case a\nkeys x\nvim 9.1.0", 3),
    ] {
        let error = parse(source).unwrap_err();
        assert!(
            error.starts_with(&format!("第 {line} 行")),
            "{source:?}：{error}"
        );
    }
    assert_eq!(parse("case a\ntext \"x\"").unwrap_err(), "用例 a 缺少 keys");
}
