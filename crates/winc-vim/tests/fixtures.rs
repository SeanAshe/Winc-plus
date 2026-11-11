//! 对照测试：读取录制文件，把 `vim` 的执行结果与 `winc-vim` 的结果比较。
//!
//! 录制文件（`fixtures/*.rec`）由 `cargo xtask vim-fixtures` 从同名的用例文件（`.cases`）生成，
//! 测试不需要 `vim`。文本、光标与各寄存器的内容和类型都参与比较。带 `later` 标记的用例所用的
//! 命令尚未实现，跳过；带 `known` 标记的用例已知与 `vim` 不同，核对结果确实不同；手写了期望
//! 结果的用例以手写的结果为准。

mod common;

use std::path::{Path, PathBuf};

use winc_core::{Lines, Selection};
use winc_vim::fixture::{self, Case, File, Position, Register, RegisterKind, State};
use winc_vim::{Shared, Vim, notation, register};

use common::TestHost;

/// 录制基准的版本号前缀。
const BASELINE: &str = "9.1.";

/// 返回用例文件与录制文件所在的目录。
///
/// 环境变量 `WINC_VIM_FIXTURES` 指向另一个目录时使用该目录（相对路径从 workspace 根目录算起），
/// 用于核对 `cargo xtask vim-fixtures --full` 录制的全部组合。
fn dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    match std::env::var_os("WINC_VIM_FIXTURES") {
        Some(dir) => manifest.join("../..").join(dir),
        None => manifest.join("fixtures"),
    }
}

/// 读取并解析目录中扩展名为 `extension` 的全部文件，按文件名排序；返回主文件名与内容。
fn load(extension: &str) -> Vec<(String, File)> {
    let mut files: Vec<(String, File)> = std::fs::read_dir(dir())
        .expect("fixtures 目录存在")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let file = fixture::parse(&text).unwrap_or_else(|e| panic!("{name}.{extension}：{e}"));
            (name, file)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn every_case_file_has_an_up_to_date_recording() {
    let cases = load("cases");
    let recordings = load("rec");
    assert!(!cases.is_empty(), "至少有一个用例文件");
    let names = |files: &[(String, File)]| files.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&cases),
        names(&recordings),
        "用例文件与录制文件一一对应"
    );

    for ((name, source), (_, recorded)) in cases.iter().zip(&recordings) {
        let version = recorded.vim.as_deref().unwrap_or_default();
        assert!(
            version.starts_with(BASELINE),
            "{name}.rec 的版本是 {version:?}"
        );
        assert!(!recorded.cases.is_empty(), "{name}.rec 没有用例");
        assert!(
            recorded.cases.iter().all(|c| c.result.is_some()),
            "{name}.rec 缺少结果"
        );
        // 录制文件的输入与用例文件相同；不同时需要重新运行 `cargo xtask vim-fixtures`。
        let inputs: Vec<Case> = (recorded.cases.iter())
            .map(|case| Case {
                result: None,
                ..case.clone()
            })
            .collect();
        assert_eq!(
            inputs, source.cases,
            "{name}.rec 与 {name}.cases 的输入不一致"
        );
    }
}

/// 是否有字符位于 `text` 的位置 `at`；空行只有第 0 列算作有字符。
fn on_a_character(text: &[String], at: Position) -> bool {
    (text.get(at.line)).is_some_and(|line| at.col < line.chars().count().max(1))
}

#[test]
fn recorded_cases_are_well_formed() {
    for (name, file) in load("rec") {
        for case in &file.cases {
            let label = format!("{name}.rec 的用例 {}", case.name);
            // 按键写成规范的按键记法，避免记法写错时被当作普通字符送入。
            let keys = notation::parse(&case.keys);
            assert_eq!(notation::write(&keys), case.keys, "{label}");
            assert!(on_a_character(&case.text, case.cursor), "{label}");

            let result = case.result.as_ref().unwrap();
            assert!(on_a_character(&result.text, result.cursor), "{label}");
            for register in &result.registers {
                assert!(!register.lines.is_empty(), "{label}");
            }
        }
    }
}

/// 无法录制的命令：结果取决于宿主的，以及不加载配置的 `vim` 没有的插件行为（缩进对象；
/// 成对符号的 `s`、`S` 在命令表中不是独立的命令）。
const HOSTED: [&str; 11] = [
    "gc", "=", "gd", "gf", "gt", "gT", "ZZ", "ZQ", "<C-^>", "ii", "ai",
];

#[test]
fn every_supported_command_appears_in_a_recorded_case() {
    // 标为支持的命令必须有录制用例。切换注释不是 `vim` 自带的命令，格式化所选的行交给宿主的
    // 格式化工具，转到定义、打开路径、切换标签、关闭文档、移动焦点由宿主执行，都由手写的测试
    // 覆盖。
    let recorded: Vec<String> = (load("rec").into_iter())
        .flat_map(|(_, file)| file.cases)
        .filter(|case| !case.tags.iter().any(|tag| tag == "later"))
        .map(|case| case.keys)
        .collect();
    let missing: Vec<&str> = (winc_vim::command::ENTRIES.iter())
        .filter(|entry| entry.support == winc_vim::Support::Supported)
        .filter(|entry| !HOSTED.contains(&entry.keys) && !entry.keys.starts_with("<C-w>"))
        .map(|entry| entry.keys)
        .filter(|keys| !recorded.iter().any(|case| case.contains(keys)))
        .collect();
    assert!(missing.is_empty(), "没有录制用例的命令：{missing:?}");
}

/// 在 `winc-vim` 中执行用例 `case`，返回执行之后的状态。
///
/// 文本按文件的形式建立：每行以换行符结尾。寄存器按录制时的顺序列出。
fn run(case: &Case) -> State {
    let mut text = case.text.join("\n");
    text.push('\n');
    let mut host = TestHost::new(&text);
    let start = host.rope.line_start(case.cursor.line) + case.cursor.col;
    host.selection = Selection::point(start);
    // 录制时文本由脚本写入：跳转历史中已有文本的开头，修改列表与「上一处修改」标记中已有
    // 第二行（只有一行时为第一行）的行首。
    host.jumps = vec![0];
    host.jump_index = 1;
    let second = host.rope.line_start(usize::from(case.text.len() > 1));
    host.changes = vec![second];
    host.marks.insert('.', second);
    let mut vim = Vim::default();
    let mut shared = Shared::default();
    common::feed(&mut vim, &mut shared, &mut host, &case.keys);
    // 录制结束时已离开插入模式与可视模式。
    common::feed(&mut vim, &mut shared, &mut host, "<Esc>");
    assert_eq!(host.groups, 0, "{}：撤销分组没有结束", case.name);

    let text = host.text();
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let (line, col) = host.rope.pos_to_line_col(vim.caret(&host));
    let registers = "\"0123456789-abcdefghijklmnopqrstuvwxyz.:/"
        .chars()
        .filter_map(|name| {
            let content = shared.registers.get(Some(name))?;
            // 最近插入的文本在录制中是一行，其中的换行记作 NUL。
            let lines = if name == '.' {
                vec![content.text.replace('\n', "\0")]
            } else {
                content.lines().map(str::to_owned).collect()
            };
            Some(Register {
                name,
                kind: match content.kind {
                    register::Kind::Charwise => RegisterKind::Charwise,
                    register::Kind::Linewise => RegisterKind::Linewise,
                    register::Kind::Blockwise(width) => RegisterKind::Blockwise(width),
                },
                lines,
            })
        })
        .collect();
    State {
        text: text.split('\n').map(str::to_owned).collect(),
        cursor: Position { line, col },
        registers,
    }
}

/// 把状态写成多行文本，便于在失败信息中比较；`initial` 是用例的初始文本。
fn describe(state: &State, initial: &[String]) -> String {
    let file = File {
        vim: None,
        cases: vec![Case {
            name: "state".into(),
            text: initial.to_vec(),
            keys: "x".into(),
            result: Some(state.clone()),
            ..Case::default()
        }],
    };
    (fixture::write(&file).lines())
        .filter(|line| line.starts_with("> "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn recorded_cases_match_the_recording() {
    let only = std::env::var("WINC_VIM_CASE").ok();
    let mut failures = Vec::new();
    let (mut checked, mut skipped, mut known) = (0, 0, 0);
    for (name, file) in load("rec") {
        for case in &file.cases {
            if case.tags.iter().any(|tag| tag == "later") {
                skipped += 1;
                continue;
            }
            if only
                .as_deref()
                .is_some_and(|only| !case.name.contains(only))
            {
                continue;
            }
            checked += 1;
            let expected = case.expected.as_ref().or(case.result.as_ref()).unwrap();
            let mut actual = run(case);
            // 录制结果中无法表示的寄存器内容不比较。
            let mut expected = expected.clone();
            let opaque: Vec<char> = (expected.registers.iter())
                .filter(|register| register.lines == [fixture::OPAQUE])
                .map(|register| register.name)
                .collect();
            expected.registers.retain(|r| !opaque.contains(&r.name));
            actual.registers.retain(|r| !opaque.contains(&r.name));
            let expected = &expected;
            if case.tags.iter().any(|tag| tag == "known") {
                known += 1;
                assert_ne!(
                    &actual, expected,
                    "{name}/{}：已与录制结果一致，去掉标记",
                    case.name
                );
                continue;
            }
            if &actual != expected {
                failures.push(format!(
                    "{name}/{}  keys {}\n  文本 {:?} 光标 {} {}\n期望：\n{}\n实际：\n{}",
                    case.name,
                    case.keys,
                    case.text,
                    case.cursor.line,
                    case.cursor.col,
                    describe(expected, &case.text),
                    describe(&actual, &case.text),
                ));
            }
        }
    }
    // 设置 `WINC_VIM_REPORT` 时把全部不一致的用例写入该文件，便于逐个排查。
    if let Ok(path) = std::env::var("WINC_VIM_REPORT") {
        std::fs::write(path, failures.join("\n\n")).unwrap();
    }
    let shown = failures
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");
    assert!(
        failures.is_empty(),
        "{} 个用例与录制结果不一致（共核对 {checked} 个，跳过 {skipped} 个）：\n\n{shown}",
        failures.len()
    );
    assert!(checked > 0);
    println!("核对 {checked} 个用例，其中已知不同 {known} 个，跳过 {skipped} 个");
}
