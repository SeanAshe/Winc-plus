//! 格式化与修复的测试：换行符转换与 BOM、占位符与工作目录、各类失败原因的提示、超时、
//! 工具的选择与部分格式化，以及结果作为一次可撤销的修改应用。

use super::*;
use crate::test_util::scratch;

/// 不测试超时的地方给模拟工具的时间（60 秒）。
///
/// 新生成的脚本第一次运行时 macOS 要先检查一遍，空闲时约 0.5 秒，同时运行多套测试时排队会超过
/// 10 秒，因此留出足够的余量。
#[cfg_attr(not(unix), allow(dead_code))] // 只有 Unix 上的测试用到
pub(crate) const TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// 在 `dir` 里写一个模拟格式化工具的 `#!/bin/sh` 脚本。
pub(crate) fn fake_tool(dir: &Path, name: &str, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    winc_backend::testing::write_script(&path, &format!("#!/bin/sh\n{body}\n"));
    path
}

fn language(id: &str) -> &'static Language {
    winc_syntax::find(id).unwrap()
}

/// 创建 `PATH` 依次为 `bin` 和系统目录的 `Formatter`（模拟工具的脚本里要用 `tr`、`cat`）。
#[cfg_attr(not(unix), allow(dead_code))] // 只有 Unix 上的测试用到
fn formatter(bin: &Path) -> Formatter {
    let path = std::env::join_paths([bin, Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    Formatter::with_vars(vec![
        (OsString::from("PATH"), path),
        (OsString::from("HOME"), OsString::from("/nowhere")),
    ])
}

#[cfg_attr(not(unix), allow(dead_code))] // 只有 Unix 上的测试用到
fn format(
    formatter: &Formatter,
    file: &Path,
    command: &str,
    text: &str,
) -> Result<String, FormatError> {
    let plan = Plan::new(language("python"), file, None, Some(command))?;
    let line_ending = LineEnding::detect(&Rope::from_str(text));
    let job = Job {
        plan,
        text: Rope::from_str(text),
        line_ending,
        timeout: TOOL_TIMEOUT,
    };
    let formatted = formatter.run(job, &Cancel::default())?;
    let mut rope = Rope::from_str(text);
    for change in &formatted.changes {
        rope.remove(change.start..change.start + change.removed.chars().count());
        rope.insert(change.start, &change.inserted);
    }
    Ok(rope.to_string())
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn output_replaces_the_text_with_the_files_own_newlines() {
    let dir = scratch("run");
    let bin = dir.join("bin");
    fake_tool(&bin, "upper", "tr a-z A-Z");
    let formatter = formatter(&bin);
    let file = dir.join("a.py");

    assert_eq!(
        format(&formatter, &file, "upper", "ab\ncd\n").unwrap(),
        "AB\nCD\n"
    );
    // CRLF、CR 的文件送进工具的是 LF，输出再转换回原有的换行符；混用时统一成占多数的那一种。
    assert_eq!(
        format(&formatter, &file, "upper", "ab\r\ncd\r\n").unwrap(),
        "AB\r\nCD\r\n"
    );
    assert_eq!(
        format(&formatter, &file, "upper", "a\rb\r").unwrap(),
        "A\rB\r"
    );
    assert_eq!(
        format(&formatter, &file, "upper", "a\r\nb\r\nc\n").unwrap(),
        "A\r\nB\r\nC\r\n"
    );

    // 输出开头的 BOM 去掉；与原内容相同时没有修改。
    fake_tool(&bin, "bom", "printf '\\357\\273\\277'; cat");
    let job = |text: &str| Job {
        plan: Plan::new(language("python"), &file, None, Some("bom")).unwrap(),
        text: Rope::from_str(text),
        line_ending: LineEnding::Lf,
        timeout: TOOL_TIMEOUT,
    };
    let formatted = formatter.run(job("x = 1\n"), &Cancel::default()).unwrap();
    assert_eq!(
        formatted,
        Formatted {
            changes: Vec::new(),
            lines: 0,
            tool: "bom".to_owned(),
        }
    );

    // 空文件格式化出空内容，不算「没有输出」。
    fake_tool(&bin, "nothing", "cat > /dev/null");
    assert_eq!(format(&formatter, &file, "nothing", "").unwrap(), "");
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn the_file_name_is_one_argument_and_the_tool_runs_beside_it() {
    let dir = scratch("args");
    let bin = dir.join("bin");
    fake_tool(
        &bin,
        "show",
        "printf '%s|%s|%s|%s|%s\\n' \"$1\" \"$WINC_FILE\" \"$PWD\" \"$NO_COLOR\" \"$#\"",
    );
    let sub = dir.join("有 空格");
    std::fs::create_dir_all(&sub).unwrap();
    let file = sub.join("it's.py");
    let out = format(&formatter(&bin), &file, "show --path={file}", "x\n").unwrap();
    let path = file.display();
    assert_eq!(out, format!("--path={path}|{path}|{}|1|1\n", sub.display()));
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn failures_leave_the_text_alone_and_say_why() {
    let dir = scratch("fail");
    let bin = dir.join("bin");
    let formatter = formatter(&bin);
    let file = dir.join("main.py");
    let error = |command: &str, body: &str| {
        fake_tool(&bin, command, body);
        format(&formatter, &file, command, "x = 1\n").unwrap_err()
    };
    let message = |error: FormatError| match error {
        FormatError::Tool { message, .. } => message,
        other => panic!("{other:?}"),
    };

    assert_eq!(
        message(error(
            "gofmt",
            "echo '<standard input>:2:20: expected operand' >&2; exit 2"
        )),
        "main.py:2:20: expected operand"
    );
    // 前缀和颜色控制符去掉；位置在第二行时补在末尾。
    assert_eq!(
        message(error(
            "rustfmt",
            "printf '\\033[1;31merror\\033[0m: expected expression\\n --> <stdin>:1:18\\n' >&2; exit 1"
        )),
        "expected expression（1:18）"
    );
    assert_eq!(
        message(error(
            "jq",
            "echo 'jq: error: parse error at line 2' >&2; exit 5"
        )),
        "parse error at line 2"
    );
    assert_eq!(
        message(error("quiet", "exit 3")),
        "「quiet」运行失败（退出码 3）"
    );

    // 完整记录：运行的命令、工作目录、退出码，以及原样的标准错误。
    let FormatError::Tool { output, .. } = error(
        "verbose",
        "echo 'line one' >&2; echo '  line two' >&2; exit 4",
    ) else {
        panic!()
    };
    assert_eq!(
        output,
        "$ verbose < main.py\n（工作目录：文件所在目录，退出码 4）\n\nline one\n  line two\n"
    );
    // 没有输出、标准错误里说明了原因（例如被配置排除的文件）：完整记录中也能看到该说明。
    assert!(matches!(
        error("silent", "echo 'skipped: excluded' >&2"),
        FormatError::Tool { message, output }
            if message == "「silent」未返回任何输出" && output.contains("skipped: excluded")
    ));
    assert_eq!(
        error("empty", "cat > /dev/null"),
        FormatError::Failed("「empty」未返回任何输出".to_owned())
    );
    assert_eq!(
        error("binary", "printf '\\377\\376'"),
        FormatError::Failed("「binary」的输出不是 UTF-8 文本".to_owned())
    );
    assert_eq!(
        format(&formatter, &file, "nope", "x\n").unwrap_err(),
        FormatError::Missing {
            message: "未找到「nope」".to_owned(),
            tools: Vec::new()
        }
    );

    // 标准错误里有内容、退出码是 0：仍然算成功。
    fake_tool(&bin, "noisy", "echo 'All done!' >&2; tr x y");
    assert_eq!(format(&formatter, &file, "noisy", "x\n").unwrap(), "y\n");
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn timeouts_are_reported() {
    let dir = scratch("slow");
    let bin = dir.join("bin");
    fake_tool(&bin, "slow", "sleep 30");
    let plan = Plan::new(language("go"), &dir.join("a.go"), None, Some("slow")).unwrap();
    let job = Job {
        plan,
        text: Rope::from_str("x\n"),
        line_ending: LineEnding::Lf,
        timeout: Duration::from_secs(1),
    };
    assert_eq!(
        formatter(&bin).run(job, &Cancel::default()),
        Err(FormatError::Failed(
            "「slow」超过 1 秒未完成。新安装的工具首次运行可能较慢，请重试".to_owned()
        ))
    );
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn plans_prefer_the_projects_own_programs() {
    let root = scratch("plan");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = src.join("app.ts");
    let local = fake_tool(&root.join("node_modules/.bin"), "prettier", "cat");
    std::fs::write(root.join(".prettierrc"), "{}").unwrap();

    let plan = Plan::new(language("typescript"), &file, Some(&root), None).unwrap();
    assert_eq!(
        plan.program,
        Program::Project {
            path: local,
            folder: root.clone(),
            searchable: true
        }
    );
    assert_eq!(
        plan.auto,
        Some(("Prettier", Some(".prettierrc".to_owned())))
    );
    assert_eq!(plan.tool(), "prettier");

    // 选了「使用系统中安装的」：到 `PATH` 里找同名程序，找不到时说明项目用的是什么。
    let mut system = plan.clone();
    system.use_system();
    assert_eq!(system.program, Program::Search("prettier".to_owned()));
    let job = Job {
        plan: system,
        text: Rope::from_str("x\n"),
        line_ending: LineEnding::Lf,
        timeout: Duration::from_secs(5),
    };
    assert_eq!(
        formatter(&root.join("empty-bin")).run(job, &Cancel::default()),
        Err(FormatError::Missing {
            message: "项目使用 Prettier（.prettierrc），但未找到「prettier」".to_owned(),
            tools: vec!["prettier".to_owned()]
        })
    );

    // 命令里写的相对路径从打开的目录起算，同样是项目自带的程序，不能改到 `PATH` 里找。
    let mut relative =
        Plan::new(language("typescript"), &file, Some(&root), Some("./fmt.sh")).unwrap();
    relative.use_system();
    assert_eq!(
        relative.program,
        Program::Project {
            path: root.join("./fmt.sh"),
            folder: root.clone(),
            searchable: false
        }
    );
    assert_eq!(
        Plan::new(language("typescript"), &file, None, Some("/opt/x/fmt"))
            .unwrap()
            .program,
        Program::Path(PathBuf::from("/opt/x/fmt"))
    );
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn without_a_sign_the_first_installed_tool_is_used() {
    let root = scratch("plan-guess");
    let bin = root.join("bin");
    let file = root.join("a.py");
    let plan = Plan::new(language("python"), &file, None, None).unwrap();
    assert!(plan.is_guess());
    let job = |plan: Plan| Job {
        plan,
        text: Rope::from_str("x\n"),
        line_ending: LineEnding::Lf,
        timeout: TOOL_TIMEOUT,
    };

    // 一个都没安装：提示安装候选工具中的任意一个。
    let error = formatter(&bin).run(job(plan.clone()), &Cancel::default());
    assert_eq!(
        error,
        Err(FormatError::NoTool {
            message: "未安装 Python 格式化工具（Black 或 Ruff）".to_owned(),
            tools: vec!["black".to_owned(), "ruff".to_owned()]
        })
    );

    // 只安装了排在后面的工具：使用它，命令取自它的预设。
    fake_tool(&bin, "ruff", "echo \"$@\" | tr ' ' '|'");
    let formatter = formatter(&bin);
    let resolved = formatter.resolve(&plan).unwrap();
    assert_eq!(resolved.auto, Some(("Ruff", None)));
    assert_eq!(resolved.program, Program::Search("ruff".to_owned()));
    assert_eq!(formatter.peek(&plan).unwrap().unwrap().tool(), "ruff");
    let formatted = formatter
        .run(job(plan.clone()), &Cancel::default())
        .unwrap();
    assert_eq!(formatted.tool, "ruff");
    assert_eq!(
        formatted.changes[0].inserted,
        format!("format|--stdin-filename|{}|-\n", file.display())
    );

    // 只格式化一部分：运行前确定工具之后，再检查该工具是否支持。
    let mut partial = plan.clone();
    let text = Rope::from_str("a\nb\n");
    partial.restrict(&FormatRange::new(&text, 2, 3)).unwrap();
    let formatted = formatter.run(job(partial), &Cancel::default()).unwrap();
    assert!(formatted.changes[0].inserted.contains("|--range=2:1-2:2"));

    // 项目里有迹象时不尝试其他工具：项目用的工具没安装就报错。
    std::fs::write(root.join("pyproject.toml"), "[tool.black]\n").unwrap();
    let plan = Plan::new(language("python"), &file, None, None).unwrap();
    assert!(!plan.is_guess());
    assert!(matches!(
        formatter.run(job(plan), &Cancel::default()),
        Err(FormatError::Missing { tools, .. }) if tools == ["black"]
    ));
}

#[test]
fn plans_explain_why_nothing_can_run() {
    let root = scratch("plan-none");
    let file = root.join("a.js");
    let unavailable = |id: &str| match Plan::new(language(id), &file, None, None) {
        Err(FormatError::Unset(message)) => message,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        unavailable("makefile"),
        "Makefile 无可自动使用的格式化工具，请在设置中指定命令"
    );
    let plain = winc_syntax::detect("README");
    assert!(matches!(
        Plan::new(plain, &file, None, None),
        Err(FormatError::Unavailable(_))
    ));
    assert!(matches!(
        Plan::new(language("rust"), &file, None, Some("rustfmt 'oops")),
        Err(FormatError::Failed(_))
    ));
}

#[test]
fn ranges_cover_whole_lines_and_count_the_way_each_tool_does() {
    let text = Rope::from_str("ab\r\n中😀d\r\nxyz\r\n");
    let pos = |line: usize, col: usize| text.line_to_char(line) + col;
    let range = FormatRange::new(&text, pos(1, 1), pos(2, 0));
    // 选区止于第三行行首：只包含第二行。
    assert_eq!((range.start_line, range.end_line), (2, 2));
    assert_eq!(range.end_column, 4);
    // 去掉了一个 `\r`，😀 在 UTF-16 中占两个码元。
    assert_eq!((range.start_offset, range.end_offset), (4, 8));
    assert_eq!(
        range.expand("--lines={start-line}:{end-line}"),
        "--lines=2:2"
    );

    let empty = FormatRange::new(&text, pos(2, 1), pos(2, 1));
    assert_eq!((empty.start_line, empty.end_line), (3, 3));
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn only_tools_that_can_format_part_of_a_file_are_restricted() {
    let dir = scratch("range");
    let bin = dir.join("bin");
    fake_tool(&bin, "clang-format", "printf '%s ' \"$@\"; echo");
    let formatter = formatter(&bin);
    let file = dir.join("a.c");
    let text = Rope::from_str("int a;\nint b;\nint c;\n");
    let range = FormatRange::new(&text, 7, 10);

    let mut plan = Plan::new(
        language("c"),
        &file,
        None,
        Some("clang-format --style=llvm"),
    )
    .unwrap();
    plan.restrict(&range).unwrap();
    assert!(plan.is_restricted());
    let job = Job {
        plan,
        text: text.clone(),
        line_ending: LineEnding::Lf,
        timeout: TOOL_TIMEOUT,
    };
    let out = formatter.run(job, &Cancel::default()).unwrap();
    assert_eq!(out.changes[0].inserted, "--style=llvm --lines=2:2 \n");

    // 自动选择的工具没有区段参数：提示原因，不改为格式化整个文件。
    let mut gofmt = Plan::new(language("go"), &dir.join("a.go"), None, None).unwrap();
    assert_eq!(
        gofmt.restrict(&range),
        Err(FormatError::Unavailable(
            "「gofmt」不支持格式化所选内容".to_owned()
        ))
    );
    assert!(!gofmt.is_restricted());

    // 区段参数只属于格式化的子命令：`ruff check` 不认识 `--range`。
    let ruff = |command: &str| {
        let file = dir.join("a.py");
        let mut plan = Plan::new(language("python"), &file, None, Some(command)).unwrap();
        plan.restrict(&range).unwrap();
        plan.fragment
    };
    assert!(!ruff("ruff format --stdin-filename {file} -"));
    assert!(ruff(
        "ruff check --select I --fix --stdin-filename {file} -"
    ));
}

/// 用设置里的命令 `command` 只格式化 `text` 中 `from..to`（字符）的内容，返回之后的全文。
#[cfg(unix)]
fn format_part(
    formatter: &Formatter,
    (file, root): (&Path, Option<&Path>),
    command: &str,
    text: &str,
    (from, to): (usize, usize),
) -> Result<String, FormatError> {
    let rope = Rope::from_str(text);
    let mut plan = Plan::new(language("python"), file, root, Some(command))?;
    plan.restrict(&FormatRange::new(&rope, from, to))?;
    let job = Job {
        plan,
        line_ending: LineEnding::detect(&rope),
        text: rope.clone(),
        timeout: TOOL_TIMEOUT,
    };
    let formatted = formatter.run(job, &Cancel::default())?;
    let mut rope = rope;
    for change in &formatted.changes {
        rope.remove(change.start..change.start + change.removed.chars().count());
        rope.insert(change.start, &change.inserted);
    }
    Ok(rope.to_string())
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn settings_commands_without_range_options_get_only_the_selection() {
    let dir = scratch("fragment");
    let bin = dir.join("bin");
    fake_tool(&bin, "mark", "sed 's/^/>/'");
    fake_tool(&bin, "same", "cat");
    fake_tool(&bin, "json", r#"cat >/dev/null; printf '{\n  "a": 1\n}\n'"#);
    let formatter = formatter(&bin);
    let at = (dir.join("a.py"), None);
    let part = |command: &str, text: &str, range| {
        format_part(&formatter, (&at.0, at.1), command, text, range).unwrap()
    };

    // 只有选中的第二行交给了命令，其余各行原样。
    assert_eq!(part("mark", "a\nb\nc\n", (2, 4)), "a\n>b\nc\n");
    assert_eq!(part("mark", "a\r\nb\r\nc\r\n", (3, 6)), "a\r\n>b\r\nc\r\n");
    // 回归：选中没有换行结尾的最后一行，文件末尾多出一个换行。
    assert_eq!(part("same", "a\nb", (2, 3)), "a\nb");
    // 选区在一行中间：两侧的文字不动，输出的后续各行与这一行对齐。
    let text = "    x = {\"a\":1};\n";
    let (from, to) = (text.find('{').unwrap(), text.find(';').unwrap());
    assert_eq!(
        part("json", text, (from, to)),
        "    x = {\n      \"a\": 1\n    };\n"
    );
}

/// 回归：所选内容原样交给命令、输出原样放回，整理行首空白的工具使选中的各行丢了缩进。
#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn a_formatted_selection_keeps_its_indentation() {
    let dir = scratch("fragment-indent");
    let bin = dir.join("bin");
    // 冒充的工具把收到的内容另存一份，再把 `x` 换成 `y`。
    fake_tool(&bin, "swap", r#"tee "$WINC_FILE.in" | sed 's/x/y/'"#);
    let formatter = formatter(&bin);
    let file = dir.join("a.py");
    let text = "def f():\n    if x:\n        return 1\n    return 2\n";
    let (from, to) = (
        text.find("    if").unwrap(),
        text.find("    return 2").unwrap(),
    );
    let expected = "def f():\n    if y:\n        return 1\n    return 2\n";

    // 选区从行首开始，或从缩进里开始：命令收到的都是去掉公共缩进的两行。
    for from in [from, from + 2] {
        let out = format_part(&formatter, (&file, None), "swap", text, (from, to));
        assert_eq!(out.as_deref(), Ok(expected));
        let sent = std::fs::read_to_string(dir.join("a.py.in")).unwrap();
        assert_eq!(sent, "if x:\n    return 1\n");
    }
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn a_selection_runs_the_same_program_as_the_whole_file() {
    let root = scratch("fragment-program");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    fake_tool(&root.join("node_modules/.bin"), "sqlfmt", "tr a-z A-Z");
    fake_tool(&root, "fmt.sh", "tr a-z A-Z");
    // `PATH` 里没有这两个程序：项目自带的和按相对路径写的都从打开的目录找。
    let formatter = formatter(&root.join("empty-bin"));
    let at = (src.join("a.sql"), Some(root.as_path()));
    for command in ["sqlfmt", "./fmt.sh"] {
        let out = format_part(
            &formatter,
            (&at.0, at.1),
            command,
            "one\ntwo\nthree\n",
            (4, 8),
        );
        assert_eq!(out.as_deref(), Ok("one\nTWO\nthree\n"), "{command}");
    }
}

#[test]
fn commands_that_cannot_take_only_the_selection_say_why() {
    let dir = scratch("fragment-refused");
    let range = FormatRange::new(&Rope::from_str("a\nb\nc\n"), 2, 4);
    let refused = |language_id: &str, command: &str| {
        let file = dir.join("a.txt");
        let mut plan = Plan::new(language(language_id), &file, None, Some(command)).unwrap();
        let refused = plan.restrict(&range).unwrap_err();
        assert!(!plan.is_restricted());
        refused.to_string()
    };
    // 回归：程序写成绝对路径时没有拒绝，命令输出的整个文件被放进了所选内容的位置。
    for command in ["gofmt {file}", "/opt/tools/gofmt {file}"] {
        assert_eq!(
            refused("go", command),
            "「gofmt」直接读取磁盘上的文件，不支持格式化所选内容"
        );
    }
    assert_eq!(
        refused("javascript", "eslint --stdin --fix-dry-run --format json"),
        "「eslint」不支持格式化所选内容"
    );
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn a_selection_without_any_installed_tool_asks_to_install_one() {
    let root = scratch("fragment-guess");
    let text = Rope::from_str("a\nb\nc\n");
    let mut plan = Plan::new(language("python"), &root.join("a.py"), None, None).unwrap();
    assert!(plan.is_guess());
    plan.restrict(&FormatRange::new(&text, 2, 4)).unwrap();
    let job = Job {
        plan,
        text,
        line_ending: LineEnding::Lf,
        timeout: TOOL_TIMEOUT,
    };
    let error = formatter(&root.join("empty-bin")).run(job, &Cancel::default());
    assert!(
        matches!(error, Err(FormatError::NoTool { .. })),
        "{error:?}"
    );
}

#[test]
fn commands_that_read_the_file_from_disk_are_spotted() {
    assert!(Plan::reads_file_from_disk("gofmt {file}"));
    assert!(Plan::reads_file_from_disk("prettier {file}"));
    assert!(Plan::reads_file_from_disk("rustfmt --edition 2021 {file}"));
    assert!(!Plan::reads_file_from_disk(
        "prettier --stdin-filepath {file}"
    ));
    assert!(!Plan::reads_file_from_disk(
        "clang-format --assume-filename={file}"
    ));
    assert!(!Plan::reads_file_from_disk("jq ."));
}

#[test]
fn results_apply_as_one_undo_step_and_follow_the_text() {
    use crate::{Command, DocumentId};

    let old = "fn a() {\n  call(x)\n}\n";
    let new = "fn a() {\n    call(x);\n}\n";
    let mut doc = Document::from_text(DocumentId(1), old);
    doc.set_language("a.rs");
    doc.detected_indent = winc_core::indent::detect(doc.text());
    doc.selection = Selection::new(old.find("call").unwrap(), old.find("(x)").unwrap());
    let sent = doc.format_source().unwrap();

    let top = doc.apply_formatted(&sent, diff::line_changes(old, new), old.find('}').unwrap());
    assert_eq!(top, new.find('}'), "窗口第一行跟着一起换算");
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), new);
    assert_eq!(
        doc.selection,
        Selection::new(new.find("call").unwrap(), new.find("(x)").unwrap())
    );
    assert_eq!(
        doc.indent(),
        winc_core::Indent::Spaces(4),
        "缩进跟着重新识别"
    );

    doc.execute(Command::Undo, 10, 4);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), old, "⌘Z 一次就还原");
    assert!(!doc.is_dirty());

    // 送去格式化之后文档又被修改：不改动。
    let sent = doc.format_source().unwrap();
    doc.execute(Command::Insert("y".into()), 10, 4);
    let typed = doc.text_range(0, usize::MAX).unwrap();
    assert_eq!(
        doc.apply_formatted(&sent, diff::line_changes(old, new), 0),
        None
    );
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), typed);

    // 只读文档不提供格式化的内容。
    doc.readonly = true;
    assert!(doc.format_source().is_err());
}

#[test]
fn fixers_are_only_picked_when_the_project_uses_them() {
    let dir = scratch("fix-auto");
    let file = dir.join("a.py");
    let fix = |file: &Path| Plan::for_task(Task::Fix, language("python"), file, None, None);

    // 项目里没有迹象：修复工具不像格式化工具那样装了就使用。
    let Err(FormatError::Unset(message)) = fix(&file) else {
        panic!("没有迹象时不该选中");
    };
    assert_eq!(
        message,
        "项目未配置 Python 的修复工具（Ruff），可在设置中指定"
    );

    std::fs::write(
        dir.join("pyproject.toml"),
        "[tool.ruff]\nline-length = 100\n",
    )
    .unwrap();
    let plan = fix(&file).unwrap();
    assert_eq!(plan.auto, Some(("Ruff", Some("pyproject.toml".to_owned()))));
    assert_eq!(plan.task(), Task::Fix);
    assert_eq!(plan.args[..3], ["ruff", "check", "--fix"]);
    // 格式化仍使用 `ruff format`。
    let plan = Plan::new(language("python"), &file, None, None).unwrap();
    assert_eq!(plan.args[..2], ["ruff", "format"]);

    let Err(FormatError::Unset(message)) =
        Plan::for_task(Task::Fix, language("go"), &dir.join("a.go"), None, None)
    else {
        panic!("Go 没有修复工具的预设");
    };
    assert_eq!(message, "Go 无可自动使用的修复工具，请在设置中指定命令");

    std::fs::write(dir.join("eslint.config.mjs"), "export default [];\n").unwrap();
    let plan = Plan::for_task(
        Task::Fix,
        language("typescript"),
        &dir.join("a.ts"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        plan.auto,
        Some(("ESLint", Some("eslint.config.mjs".to_owned())))
    );
}

/// 为每种语言选择格式化工具和修复工具，返回各次选择结果的完整描述。
fn plan_everything(
    mut plan: impl FnMut(Task, &Language) -> Result<Plan, FormatError>,
) -> Vec<String> {
    let tasks = [Task::Format, Task::Fix];
    (tasks.into_iter())
        .flat_map(|task| winc_syntax::languages().iter().map(move |l| (task, l)))
        .map(|(task, language)| format!("{task:?} {}: {:?}", language.id, plan(task, language)))
        .collect()
}

#[test]
fn one_survey_for_every_language_chooses_what_separate_lookups_choose() {
    let root = scratch("survey-same");
    let deep = root.join("packages/app/src");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(root.join(".prettierrc"), "{}").unwrap();
    std::fs::write(root.join("pyproject.toml"), "[tool.ruff]\n").unwrap();
    // 更近一级的 Biome 遮蔽上层的 Prettier；ESLint 只在上层的依赖中声明。
    std::fs::write(root.join("packages/app/biome.json"), "{}").unwrap();
    let package = r#"{"devDependencies": {"eslint": "^9"}}"#;
    std::fs::write(root.join("packages/package.json"), package).unwrap();
    std::fs::write(root.join("packages/app/.clang-format"), "").unwrap();
    // 存放程序的目录存在、其中没有程序：覆盖先检查目录、再检查程序的情形。
    std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
    let file = deep.join("_");
    let separately =
        |task, language: &Language| Plan::for_task(task, language, &file, Some(&root), None);

    let mut survey = Survey::default();
    let shared =
        plan_everything(|task, language| survey.plan(task, language, &file, Some(&root), None));
    assert_eq!(shared, plan_everything(separately));

    // 同一轮选择沿用已查询的结果，其间新建、删除的文件由下一轮反映。
    let typescript = language("typescript");
    std::fs::write(deep.join(".prettierrc.json"), "{}").unwrap();
    std::fs::remove_file(root.join("pyproject.toml")).unwrap();
    let auto = |plan: Result<Plan, FormatError>| plan.unwrap().auto;
    let stale = survey.plan(Task::Format, typescript, &file, Some(&root), None);
    assert_eq!(auto(stale), Some(("Biome", Some("biome.json".to_owned()))));
    let fresh = Survey::default().plan(Task::Format, typescript, &file, Some(&root), None);
    assert_eq!(
        auto(fresh),
        Some(("Prettier", Some(".prettierrc.json".to_owned())))
    );
    let python = language("python");
    let stale = survey.plan(Task::Fix, python, &file, Some(&root), None);
    assert_eq!(
        auto(stale),
        Some(("Ruff", Some("pyproject.toml".to_owned())))
    );
    let fresh = Survey::default().plan(Task::Fix, python, &file, Some(&root), None);
    assert!(matches!(fresh, Err(FormatError::Unset(_))));
}

#[test]
fn choosing_tools_for_every_language_asks_about_each_file_once_per_level() {
    let deep = scratch("survey-count").join("a/b/c/d");
    std::fs::create_dir_all(&deep).unwrap();
    let file = deep.join("_");
    let mut survey = Survey::default();
    let _ = plan_everything(|task, language| survey.plan(task, language, &file, None, None));

    // 每级目录询问各个迹象文件、`package.json`、`pyproject.toml` 和三个存放程序的目录。存放
    // 程序的目录不存在时不询问其中的程序，次数与 `PATHEXT` 中扩展名的个数无关。
    let signs = PRESETS.iter().flat_map(|preset| match preset.auto {
        Auto::When(signs) | Auto::OnlyWhen(signs) => signs,
        _ => &[],
    });
    let names: std::collections::HashSet<&str> = signs
        .filter_map(|sign| match sign {
            tools::Sign::File(name) => Some(*name),
            _ => None,
        })
        .collect();
    let per_level = names.len() + 5;
    let levels = deep.ancestors().count();
    let asked = survey.asked();
    assert!(
        asked <= levels * per_level,
        "{asked} > {levels} × {per_level}"
    );

    let _ = plan_everything(|task, language| survey.plan(task, language, &file, None, None));
    assert_eq!(survey.asked(), asked);
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn eslint_fixes_come_from_its_json_results() {
    let dir = scratch("fix-eslint");
    let bin = dir.join("bin");
    let formatter = formatter(&bin);
    let file = dir.join("a.js");
    let fix = |text: &str| {
        let command = "eslint --stdin --stdin-filename {file} --fix-dry-run --format json";
        let job = Job {
            plan: Plan::for_task(
                Task::Fix,
                language("javascript"),
                &file,
                None,
                Some(command),
            )
            .unwrap(),
            text: Rope::from_str(text),
            line_ending: LineEnding::Lf,
            timeout: TOOL_TIMEOUT,
        };
        formatter.run(job, &Cancel::default())
    };

    // 已修复，但仍有无法自动修复的问题（退出码 1）：使用 `output`。
    fake_tool(
        &bin,
        "eslint",
        r#"cat >/dev/null; printf '[{"filePath":"a.js","messages":[{"ruleId":"no-undef","severity":2}],"output":"let a = 1;\\n"}]'; exit 1"#,
    );
    let fixed = fix("var a = 1;\n").unwrap();
    assert_eq!((fixed.lines, fixed.tool.as_str()), (1, "eslint"));
    assert_eq!(fixed.changes[0].inserted, "let a = 1;\n");

    // 没有可修复的问题：没有 `output`。
    fake_tool(
        &bin,
        "eslint",
        r#"cat >/dev/null; printf '[{"filePath":"a.js","messages":[]}]'"#,
    );
    assert!(fix("let a = 1;\n").unwrap().changes.is_empty());

    // 文件无法解析（语法错误）。
    fake_tool(
        &bin,
        "eslint",
        r#"cat >/dev/null; printf '[{"messages":[{"fatal":true,"message":"Parsing error: Unexpected token","line":3,"column":7}]}]'; exit 1"#,
    );
    assert_eq!(
        fix("let = ;\n").unwrap_err(),
        FormatError::Failed("Parsing error: Unexpected token（3:7）".to_owned())
    );

    // 配置出错（退出码 2）仍然是失败。
    fake_tool(
        &bin,
        "eslint",
        "cat >/dev/null; echo 'Oops! Something went wrong!' >&2; exit 2",
    );
    assert!(matches!(fix("let a;\n"), Err(FormatError::Tool { .. })));
}

#[test]
fn distribution_files_get_linux_install_commands() {
    assert!(install::fits_this_system("apt", true));
    assert!(install::fits_this_system("brew", true));
    assert!(!install::fits_this_system("winget", true));
    assert!(!install::fits_this_system("scoop", true));
    assert!(!install::fits_this_system("xcode-select", true));
    assert!(install::fits_this_system("npm", true));
}

#[test]
fn tools_found_in_a_wsl_distro_are_remembered_until_forgotten() {
    let formatter = Formatter::default();
    // 没有查过：设置页还不知道，等后台查询。
    assert_eq!(formatter.wsl_asked("Ubuntu", "ruff"), None);
    formatter.wsl_found("Ubuntu", "ruff", Some(Path::new("/usr/bin/ruff")));
    formatter.wsl_found("Ubuntu", "black", None);
    assert_eq!(
        formatter.wsl_asked("Ubuntu", "ruff"),
        Some(Some(PathBuf::from("/usr/bin/ruff")))
    );
    assert_eq!(formatter.wsl_asked("Ubuntu", "black"), Some(None));
    // 各发行版分别记录；再次查到时替换。
    assert_eq!(formatter.wsl_asked("Debian", "ruff"), None);
    formatter.wsl_found(
        "Ubuntu",
        "black",
        Some(Path::new("/home/me/.local/bin/black")),
    );
    assert!(formatter.wsl_asked("Ubuntu", "black").unwrap().is_some());
    // 重新打开设置时丢弃，新安装的工具得以发现。
    formatter.forget_wsl();
    assert_eq!(formatter.wsl_asked("Ubuntu", "ruff"), None);
}
