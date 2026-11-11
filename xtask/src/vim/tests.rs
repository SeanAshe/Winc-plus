//! 录制工具的测试：脚本的生成与结果的解析不需要 `vim`；实际录制的核对标记为 `#[ignore]`。

use super::*;

#[test]
fn keys_are_written_for_a_double_quoted_script_string() {
    let keys = |text: &str| vim_keys(&notation::parse(text));
    assert_eq!(keys("d2w").unwrap(), "d2w");
    assert_eq!(keys("\"ayy").unwrap(), "\\\"ayy");
    assert_eq!(keys("f\\").unwrap(), "f\\\\");
    assert_eq!(keys("cwx<Esc>").unwrap(), "cwx\\<Esc>");
    assert_eq!(
        keys("<C-v>j<lt><Space>|").unwrap(),
        "\\<C-v>j\\<lt>\\<Space>|"
    );
    assert_eq!(keys("<S-Left>中").unwrap(), "\\<S-Left>中");
    assert!(keys("<C-\">").is_err());
    assert!(keys("<M-Bslash>").is_err());
}

#[test]
fn cursors_convert_between_characters_and_bytes() {
    let text = ["a中b".to_owned(), String::new()];
    let at = |line, col| byte_cursor(&text, Position { line, col });
    assert_eq!(at(0, 0), Ok((1, 1)));
    assert_eq!(at(0, 1), Ok((1, 2)));
    assert_eq!(at(0, 2), Ok((1, 5)));
    assert_eq!(at(1, 0), Ok((2, 1)));
    // 光标必须在某个字符上：行尾之后、空行的第 1 列、不存在的行都不接受。
    assert!(at(0, 3).is_err());
    assert!(at(1, 1).is_err());
    assert!(at(2, 0).is_err());
    assert!(byte_cursor(&["a\nb".to_owned()], Position::default()).is_err());
}

#[test]
fn the_version_number_is_split_into_three_parts() {
    assert_eq!(version_string("9011752").as_deref(), Some("9.1.1752"));
    assert_eq!(version_string("9010000").as_deref(), Some("9.1.0"));
    assert_eq!(version_string("8024000").as_deref(), Some("8.2.4000"));
    assert_eq!(version_string("vim"), None);
}

#[test]
fn the_script_sets_up_the_case_and_reports_the_state() {
    let script = script(
        Path::new("/tmp/it's/in.txt"),
        Path::new("/tmp/out.txt"),
        (2, 5),
        "d\\<Esc>",
    );
    assert!(script.contains("call setline(1, readfile('/tmp/it''s/in.txt', 'b'))\n"));
    // 读入文本的修改不记入撤销历史，用例中的撤销不会撤回初始文本。
    assert!(script.contains("set undolevels=-1\ncall setline"));
    assert!(script.contains("call cursor(2, 5)\n"));
    assert!(script.contains("call feedkeys(\"d\\<Esc>\", 'tx')\n"));
    assert!(
        script.contains(
            "for s:name in split('\"0123456789-abcdefghijklmnopqrstuvwxyz.:/', '\\zs')\n"
        )
    );
    assert!(script.ends_with("call writefile(s:out, '/tmp/out.txt')\nqa!\n"));
}

#[test]
fn results_are_parsed_into_a_state() {
    let raw = "C 2 5\nT first\nT a中b\rc\nT \nR \" v 1\nL b\rc\n\
               R 1 V 2\nL x\nL \nR a b3 2\nL abc\nL def\nE\n";
    let state = parse_output(raw.as_bytes()).unwrap();
    assert_eq!(state.text, ["first", "a中b\rc", ""]);
    // 第 2 行的第 5 个字节是 `中` 之后的 `b`，即下标为 2 的字符。
    assert_eq!(state.cursor, Position { line: 1, col: 2 });
    assert_eq!(
        state.registers,
        [
            Register {
                name: '"',
                kind: RegisterKind::Charwise,
                lines: vec!["b\rc".into()],
            },
            Register {
                name: '1',
                kind: RegisterKind::Linewise,
                lines: vec!["x".into(), String::new()],
            },
            Register {
                name: 'a',
                kind: RegisterKind::Blockwise(3),
                lines: vec!["abc".into(), "def".into()],
            },
        ]
    );

    let empty = parse_output(b"C 1 1\nT \nE\n").unwrap();
    assert_eq!(empty.text, [""]);
    assert_eq!(empty.cursor, Position::default());
}

#[test]
fn incomplete_or_malformed_results_are_rejected() {
    for raw in [
        "",
        "T a\nE\n",
        "C 1\nT a\nE\n",
        "C 1 1\nT a\n",
        "C 1 1\nT a\nR a v 2\nL x\nE\n",
        "C 1 1\nT a\nR a q 1\nL x\nE\n",
        "C 1 1\nT a\nX\nE\n",
        "C 2 1\nT a\nE\n",
        "C 0 1\nT a\nE\n",
        "C 1 5\nT a\nE\n",
        "C 1 3\nT 中\nE\n",
        "C 1 0\nT a\nE\n",
    ] {
        assert!(parse_output(raw.as_bytes()).is_err(), "{raw:?}");
    }
    assert!(parse_output(b"C 1 1\nT \xff\nE\n").is_err());
}

#[test]
fn register_lines_that_are_not_utf8_are_recorded_as_opaque() {
    // 最近插入的文本中的功能键是内部的字节：该行记作不参与比较的内容，其余各行不变。
    let state = parse_output(b"C 1 1\nT ab\nR . v 1\nL a\x80kb\nR a v 1\nL x\nE\n").unwrap();
    assert_eq!(state.registers[0].lines, [fixture::OPAQUE]);
    assert_eq!(state.registers[1].lines, ["x"]);
}

/// 用本机的 `vim` 重新录制全部用例，核对与现有的录制文件一致。
///
/// 需要 `/usr/bin/vim` 为 9.1：`cargo test -p xtask recordings -- --ignored`。
#[test]
#[ignore = "需要本机安装 vim 9.1"]
fn recordings_are_up_to_date() {
    assert_eq!(fixtures(&["--check".to_owned()]), Ok(true));
}
