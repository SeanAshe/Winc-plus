//! `run` 的测试：覆盖配置的解析、校验、指纹和按项目生成的文件。

use super::*;

/// 所有字段都能读出，环境变量按键排序，文件可以带注释。
#[test]
fn every_field_is_read() {
    let (configs, problems) = parse(
        r#"{
            // 后端
            "configurations": [{
                "name": "api",
                "command": "cargo run",
                "args": ["--release", "a b"],
                "cwd": "crates/api",
                "env": {"B": "2", "A": "1"}
            }]
        }"#,
    );
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(
        configs,
        [RunConfig {
            name: "api".into(),
            command: "cargo run".into(),
            args: vec!["--release".into(), "a b".into()],
            cwd: Some("crates/api".into()),
            env: vec![("A".into(), "1".into()), ("B".into(), "2".into())],
        }]
    );
}

/// 无效的项和重名的项被跳过并报告，其余照常可用。
#[test]
fn bad_entries_are_skipped_and_reported() {
    let (configs, problems) = parse(
        r#"{"configurations": [
            {"name": "ok", "command": "a"},
            {"name": "ok", "command": "b"},
            {"name": "", "command": "c"},
            {"name": "no command"},
            {"name": "bad args", "command": "d", "args": [1]},
            {"name": "bad env", "command": "e", "env": {"A=B": "1"}}
        ]}"#,
    );
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].command, "a");
    assert_eq!(problems.len(), 5);
}

/// 不认识的键被报告，便于发现拼写错误，不影响其他内容。
#[test]
fn unknown_keys_are_reported() {
    let (configs, problems) =
        parse(r#"{"version": 1, "configurations": [{"name": "a", "command": "b", "arg": []}]}"#);
    assert_eq!(configs.len(), 1);
    assert_eq!(problems.len(), 2);
}

/// 不是 JSON 对象时报告一次，没有配置。
#[test]
fn invalid_files_are_reported() {
    for text in ["", "[]", "{", r#"{"configurations": {}}"#] {
        let (configs, problems) = parse(text);
        assert!(configs.is_empty(), "{text}");
        assert_eq!(problems.len(), 1, "{text}");
    }
}

/// 改变运行内容时指纹改变，改名字不影响；内容在相邻字段间移动也算改变。
#[test]
fn the_fingerprint_follows_what_runs() {
    let base = RunConfig {
        name: "a".into(),
        command: "run".into(),
        args: vec!["x".into()],
        cwd: None,
        env: Vec::new(),
    };
    let print = base.fingerprint();
    let renamed = RunConfig {
        name: "b".into(),
        ..base.clone()
    };
    assert_eq!(renamed.fingerprint(), print);
    let changes = [
        RunConfig {
            command: "other".into(),
            ..base.clone()
        },
        RunConfig {
            args: vec!["y".into()],
            ..base.clone()
        },
        RunConfig {
            cwd: Some("sub".into()),
            ..base.clone()
        },
        RunConfig {
            env: vec![("A".into(), "1".into())],
            ..base.clone()
        },
        RunConfig {
            command: "runx".into(),
            args: Vec::new(),
            ..base.clone()
        },
    ];
    for change in changes {
        assert_ne!(change.fingerprint(), print, "{change:?}");
    }
}

/// 识别到的配置可以直接使用：只有名字和命令，读取时不创建文件。
#[test]
fn detection_writes_nothing() {
    let root = temporary("detect");
    assert!(detected_run_configs(&root).is_empty());
    std::fs::write(root.join("go.mod"), "").unwrap();
    let configs = detected_run_configs(&root);
    assert_eq!(configs[0].name, "build");
    assert_eq!(configs[0].command, "go build ./...");
    assert!(!root.join(".winc").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

fn temporary(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("winc-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// 没有项目文件时写出全部字段的示例；文件已存在时不覆盖，没有文件时什么也不报告。
#[test]
fn the_template_is_valid_and_never_overwrites() {
    let root = temporary("template");
    assert_eq!(load_run_configs(&root), (Vec::new(), Vec::new()));

    assert!(create_run_configs(&root).unwrap());
    let (configs, problems) = load_run_configs(&root);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].cwd.as_deref(), Some("."));
    assert_eq!(configs[0].command, "echo \"Hello from Winc\"");
    assert!(problems.is_empty());

    std::fs::write(run_configs_path(&root), "{}").unwrap();
    assert!(!create_run_configs(&root).unwrap());
    assert_eq!(
        std::fs::read_to_string(run_configs_path(&root)).unwrap(),
        "{}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// 生成的文件列出项目的常用脚本，常用的在前，按锁文件选包管理器，数量有上限；结果可以读回。
#[test]
fn package_scripts_become_configurations() {
    let root = temporary("package");
    let scripts = [
        r#"{"scripts": {"a": "x", "b": "x", "c": "x", "d": "x","#,
        r#""e": "x", "test": "x", "dev": "x"}}"#,
    ]
    .concat();
    std::fs::write(root.join("package.json"), scripts).unwrap();
    std::fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
    create_run_configs(&root).unwrap();
    let (configs, problems) = load_run_configs(&root);
    assert!(problems.is_empty(), "{problems:?}");
    let names: Vec<&str> = configs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["dev", "test", "a", "b", "c", "d"]);
    assert_eq!(configs[0].command, "pnpm run dev");
    std::fs::remove_dir_all(&root).unwrap();
}

/// 生成的文件每项都写明全部字段。
#[test]
fn generated_entries_spell_out_every_field() {
    let root = temporary("fields");
    std::fs::write(root.join("go.mod"), "").unwrap();
    let text = template(&root);
    for field in [
        "\"name\"",
        "\"command\"",
        "\"args\": []",
        "\"cwd\": \".\"",
        "\"env\": {}",
    ] {
        assert_eq!(text.matches(field).count(), 2, "{field}\n{text}");
    }
    let (configs, problems) = parse(&text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(configs.len(), 2);
    std::fs::remove_dir_all(&root).unwrap();
}
