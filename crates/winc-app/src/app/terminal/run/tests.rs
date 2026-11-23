//! `run` 的测试：覆盖 shell 的判断、引号规则和启动参数。

use super::*;

fn config() -> RunConfig {
    RunConfig {
        name: "api".into(),
        command: "cargo run".into(),
        args: vec!["--release".into(), "a b".into(), "it's".into()],
        cwd: Some("crates/api".into()),
        env: vec![("RUST_LOG".into(), "debug info".into())],
    }
}

fn texts(args: &[Vec<u8>]) -> Vec<String> {
    let args = args.iter().map(|arg| String::from_utf8_lossy(arg));
    args.map(|arg| arg.into_owned()).collect()
}

/// 按程序名判断 shell，路径和 `.exe` 后缀不影响结果。
#[test]
fn shells_are_told_apart_by_name() {
    for (program, kind) in [
        ("/bin/zsh", ShellKind::Posix),
        ("fish", ShellKind::Posix),
        ("pwsh", ShellKind::PowerShell),
        (r"C:\Windows\System32\PowerShell.exe", ShellKind::PowerShell),
        ("cmd.exe", ShellKind::Cmd),
        ("wsl.exe", ShellKind::Wsl),
    ] {
        assert_eq!(shell_kind(program.as_bytes()), kind, "{program}");
    }
}

/// 各 shell 的引号规则：普通参数不加引号，其他的按原样传给程序。
#[test]
fn arguments_are_quoted_for_the_shell() {
    assert_eq!(quote("--port=80", ShellKind::Posix), "--port=80");
    assert_eq!(quote("", ShellKind::Posix), "''");
    assert_eq!(quote("it's", ShellKind::Posix), r"'it'\''s'");
    assert_eq!(quote("$HOME", ShellKind::Wsl), "'$HOME'");
    assert_eq!(quote("it's", ShellKind::PowerShell), "'it''s'");
    assert_eq!(quote("a \"b\"", ShellKind::Cmd), "\"a \"\"b\"\"\"");
}

/// Unix 上由默认 shell 以交互方式执行命令行，相对目录基于打开的目录，环境变量覆盖继承的同名变量。
#[test]
fn posix_runs_the_line_in_the_default_shell() {
    let env = vec![
        (b"RUST_LOG".to_vec(), b"old".to_vec()),
        (b"HOME".to_vec(), b"/h".to_vec()),
    ];
    let (launch, env) = run_launch(&config(), Path::new("/p"), None, false, env);
    assert_eq!(launch.program, None);
    assert!(launch.login);
    assert_eq!(
        launch.cwd,
        os_bytes(Path::new("/p").join("crates/api").as_os_str())
    );
    assert_eq!(
        texts(&launch.args),
        ["-i", "-c", r"cargo run --release 'a b' 'it'\''s'"]
    );
    assert_eq!(
        env,
        [
            (b"HOME".to_vec(), b"/h".to_vec()),
            (b"RUST_LOG".to_vec(), b"debug info".to_vec()),
        ]
    );
}

/// 没有写目录时在打开的目录运行；配置的 shell 带自己的参数时保留在前面。
#[test]
fn configured_shells_keep_their_arguments() {
    let plain = RunConfig {
        cwd: None,
        args: Vec::new(),
        env: Vec::new(),
        ..config()
    };
    let shell = Some("/bin/bash --norc");
    let (launch, _) = run_launch(&plain, Path::new("/p"), shell, false, Vec::new());
    assert_eq!(launch.program.as_deref(), Some(&b"/bin/bash"[..]));
    assert_eq!(launch.cwd, b"/p");
    assert_eq!(texts(&launch.args), ["--norc", "-i", "-c", "cargo run"]);
}

/// Windows 上默认用 PowerShell，环境变量名不区分大小写。
#[test]
fn windows_uses_powershell_and_ignores_variable_case() {
    let env = vec![(b"rust_log".to_vec(), b"old".to_vec())];
    let root = Path::new(r"C:\p");
    let (launch, env) = run_launch(&config(), root, None, true, env);
    assert_eq!(launch.program, None);
    assert_eq!(
        texts(&launch.args),
        ["-NoLogo", "-Command", "cargo run --release 'a b' 'it''s'"]
    );
    assert_eq!(env, [(b"RUST_LOG".to_vec(), b"debug info".to_vec())]);
}

/// `wsl.exe` 里由用户的 shell 执行命令行，环境变量写在命令行中，不改 Windows 一侧的环境。
#[test]
fn wsl_sets_variables_inside_the_distribution() {
    let plain = RunConfig {
        args: Vec::new(),
        cwd: None,
        ..config()
    };
    let (launch, env) = run_launch(&plain, Path::new("/p"), Some("wsl.exe"), true, Vec::new());
    assert!(env.is_empty());
    assert_eq!(
        texts(&launch.args),
        [
            "--exec",
            "sh",
            "-c",
            r#"exec "${SHELL:-/bin/sh}" -lic "$1""#,
            "sh",
            "export RUST_LOG='debug info'; cargo run",
        ]
    );
}

/// 选中的配置不存在时使用第一个。
#[test]
fn a_missing_selection_falls_back_to_the_first() {
    let mut run = RunConfigs {
        list: vec![
            config(),
            RunConfig {
                name: "web".into(),
                ..config()
            },
        ],
        selected: Some("web".into()),
        ..RunConfigs::default()
    };
    assert_eq!(run.current().unwrap().name, "web");
    run.selected = Some("gone".into());
    assert_eq!(run.current().unwrap().name, "api");
    run.list.clear();
    assert!(run.current().is_none());
}

/// 输出之前的一行是淡色的命令行，带命令边界标记（OSC 133 `A`、`B`、`C`）；工作目录写在后面；
/// 控制字符不能进入终端。
#[test]
fn the_banner_shows_the_command() {
    let text = |config: &RunConfig| String::from_utf8(banner(config, ShellKind::Posix)).unwrap();
    assert_eq!(
        text(&config()),
        "\x1b]133;A\x07\x1b]133;B\x07\x1b[2m▶ cargo run --release 'a b' 'it'\\''s'  (crates/api)\x1b[0m\r\n\x1b]133;C\x07"
    );
    let plain = RunConfig {
        command: "run\x1b[2J\nmore".into(),
        args: Vec::new(),
        cwd: Some(".".into()),
        ..config()
    };
    assert_eq!(
        text(&plain),
        "\x1b]133;A\x07\x1b]133;B\x07\x1b[2m▶ run [2J more\x1b[0m\r\n\x1b]133;C\x07"
    );
}

/// 默认 shell 是本机的命令：项目在 WSL 发行版中时，配置的命令仍在发行版内运行。
#[test]
fn a_project_in_a_wsl_distro_runs_inside_it_despite_the_default_shell() {
    let config = RunConfig {
        name: "test".into(),
        command: "cargo".into(),
        args: vec!["test".into()],
        cwd: None,
        env: Vec::new(),
    };
    let root = Path::new(r"\\wsl.localhost\Ubuntu\home\me\project");
    let (launch, _) = run_launch(&config, root, Some("pwsh.exe"), true, Vec::new());
    assert_eq!(launch.program.as_deref(), Some(&b"wsl.exe"[..]));
    // 本机的项目按设置的 shell 运行。
    let local = Path::new(r"C:\code\project");
    let (launch, _) = run_launch(&config, local, Some("pwsh.exe"), true, Vec::new());
    assert_eq!(launch.program.as_deref(), Some(&b"pwsh.exe"[..]));
}

/// 脚本的运行方式：Windows 上按扩展名交给命令提示符或 PowerShell，写法随执行这一行的 shell
/// 而定；其他系统上直接执行带 `#!` 的可执行文件，或按扩展名交给对应的 shell。
#[test]
fn scripts_run_with_the_program_for_their_kind() {
    use script::script_config;

    let root = Path::new("/code/project");
    let command = |name: &str, kind, system| {
        let config = script_config(&root.join(name), root, kind, system);
        config.map(|config| config.command)
    };
    let powershell = "powershell -NoLogo -ExecutionPolicy Bypass -File";
    for (name, kind, expected) in [
        ("build.bat", ShellKind::PowerShell, Some("cmd /C")),
        ("build.CMD", ShellKind::Cmd, Some("call")),
        ("build.cmd", ShellKind::Posix, Some("cmd //C")),
        ("setup.ps1", ShellKind::Cmd, Some(powershell)),
        ("build.bat", ShellKind::Wsl, None),
        ("build.sh", ShellKind::PowerShell, None),
        ("notes.txt", ShellKind::PowerShell, None),
    ] {
        let found = command(name, kind, (true, false));
        assert_eq!(found.as_deref(), expected, "{name}");
    }
    for (name, expected) in [
        ("build.sh", Some("sh")),
        ("build.bash", Some("bash")),
        ("build.zsh", Some("zsh")),
        ("build.fish", Some("fish")),
        ("build.bat", None),
        ("tool.py", None),
        ("Makefile", None),
    ] {
        let found = command(name, ShellKind::Posix, (false, false));
        assert_eq!(found.as_deref(), expected, "{name}");
    }
    // 带 `#!` 的可执行文件直接执行，不看扩展名。
    for name in ["tool.py", "deploy", "build.sh"] {
        let direct = command(name, ShellKind::Posix, (false, true));
        assert_eq!(direct.as_deref(), Some("exec"), "{name}");
    }
}

/// 脚本的配置以文件名命名，路径作为参数按 shell 的规则加引号，工作目录是脚本所在的目录。
#[test]
fn a_script_runs_in_its_own_folder_under_its_file_name() {
    use script::script_config;

    let root = Path::new("/code/my project");
    let config = |path: &str| {
        script_config(Path::new(path), root, ShellKind::Posix, (false, false)).unwrap()
    };
    let nested = config("/code/my project/tools/ci/build.sh");
    assert_eq!(nested.name, "build.sh");
    assert_eq!(nested.cwd.as_deref(), Some("tools/ci"));
    assert_eq!(nested.args, ["/code/my project/tools/ci/build.sh"]);
    assert!(nested.env.is_empty());
    // 就在打开的目录里：工作目录是打开的目录；在它之外：写绝对路径。
    assert_eq!(config("/code/my project/build.sh").cwd, None);
    assert_eq!(config("/tmp/x/build.sh").cwd.as_deref(), Some("/tmp/x"));

    let (launch, _) = run_launch(&nested, root, Some("/bin/zsh"), false, Vec::new());
    let line = "sh '/code/my project/tools/ci/build.sh'";
    assert_eq!(texts(&launch.args), ["-i", "-c", line]);
}

/// 名字写成绝对路径、上级目录的配置，许可的键仍在配置文件的路径下面，不会是别的目录的键。
#[test]
fn approval_keys_stay_below_the_run_configs_file() {
    let root = Path::new(if cfg!(windows) { "C:\\work" } else { "/work" });
    let file = run_configs_path(root);
    assert_eq!(trust_key(root, &config()), file.join("api"));
    for name in ["/work", "/", "C:\\work", "../../.."] {
        let key = trust_key(
            root,
            &RunConfig {
                name: name.into(),
                ..config()
            },
        );
        assert!(key.starts_with(&file), "{name}: {}", key.display());
        assert_ne!(key, root);
    }
}
