//! 命令行参数的测试：选项、`--` 之后的参数、`文件:行[:列]` 的拆分与用法。

use super::*;

fn args(list: &[&str]) -> Result<Args, String> {
    parse(list.iter().map(OsString::from))
}

fn at(path: &str, line: Option<usize>, col: Option<usize>) -> Target {
    Target {
        path: PathBuf::from(path),
        line,
        col,
    }
}

#[test]
fn files_lines_and_options() {
    assert_eq!(args(&[]).unwrap(), Args::default());
    let parsed = args(&["-w", "/no/such/a.rs:12", "/no/such/b.rs:3:7", "c"]).unwrap();
    assert!(parsed.wait && !parsed.stdin && parsed.one_off());
    assert_eq!(
        parsed.targets,
        [
            at("/no/such/a.rs", Some(11), None),
            at("/no/such/b.rs", Some(2), Some(6)),
            at("c", None, None),
        ]
    );
    assert!(args(&["-"]).unwrap().stdin);
    // 另开一个进程不是临时打开：仍然恢复现场、记录最近列表。
    let parsed = args(&["-n", "."]).unwrap();
    assert!(parsed.new_instance && !parsed.one_off());
    assert_eq!(parsed.targets, [at(".", None, None)]);
    assert!(args(&["--new-instance"]).unwrap().new_instance);
    assert!(
        args(&["--register-file-types"])
            .unwrap()
            .register_file_types
    );
    assert!(
        args(&["--unregister-file-types"])
            .unwrap()
            .unregister_file_types
    );
    // `--` 之后的参数都按文件处理，包括 `-x`。
    assert_eq!(args(&["--", "-x"]).unwrap().targets, [at("-x", None, None)]);
    // 行号为 0、行号不是数字、冒号前的路径为空时，不拆分。
    assert_eq!(args(&["a:0"]).unwrap().targets, [at("a:0", None, None)]);
    assert_eq!(args(&["a:b"]).unwrap().targets, [at("a:b", None, None)]);
    assert_eq!(args(&[":3"]).unwrap().targets, [at(":3", None, None)]);
}

#[test]
fn existing_files_with_colons_open_as_they_are() {
    let dir = std::env::temp_dir().join(format!("winc-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let odd = dir.join("note:12");
    std::fs::write(&odd, "").unwrap();
    let parsed = parse([odd.clone().into_os_string()]).unwrap();
    assert_eq!(parsed.targets[0].path, odd);
    assert_eq!(parsed.targets[0].line, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_hides_the_sidebar_and_a_folder_shows_it() {
    assert_eq!(args(&[]).unwrap().launch_sidebar(), None);
    assert_eq!(
        args(&["no-such-file-winc.rs"]).unwrap().launch_sidebar(),
        Some(false)
    );
    assert_eq!(args(&["."]).unwrap().launch_sidebar(), Some(true));
    assert_eq!(
        args(&["no-such-file-winc.rs", "."])
            .unwrap()
            .launch_sidebar(),
        Some(true)
    );
}

#[test]
fn bad_options_explain_themselves() {
    assert!(
        args(&["--frobnicate"])
            .unwrap_err()
            .contains("--frobnicate")
    );
    assert!(args(&["--wait"]).unwrap_err().contains("--wait"));
    assert!(args(&["-h", "--frobnicate"]).unwrap().help);
    assert!(usage().starts_with("用法"));
}

#[test]
fn wsl_resolves_absolute_paths_inside_the_distribution() {
    let parsed =
        parse(["--wsl", "Ubuntu", "/home/me/a.rs:3:2", "b.rs"].map(OsString::from)).unwrap();
    let place = winc_editor::wsl::locate(&parsed.targets[0].path).unwrap();
    assert_eq!(
        (place.distro.as_str(), place.path.as_str()),
        ("Ubuntu", "/home/me/a.rs")
    );
    assert_eq!(
        (parsed.targets[0].line, parsed.targets[0].col),
        (Some(2), Some(1))
    );
    assert_eq!(parsed.targets[1].path, PathBuf::from("b.rs"));
}

#[test]
fn wsl_needs_a_distribution_name() {
    assert!(parse([OsString::from("--wsl")]).is_err());
}

#[test]
fn the_version_flag_returns_at_once() {
    assert!(args(&["-v"]).unwrap().version);
    assert!(args(&["--version", "--no-such-option"]).unwrap().version);
    assert!(!args(&["."]).unwrap().version);
}

#[test]
fn the_launcher_flag_is_recorded_and_dropped_when_relaunching() {
    let parsed = args(&["--launcher", "-n", "a"]).unwrap();
    assert!(parsed.launcher && parsed.new_instance);
    assert_eq!(parsed.targets, [at("a", None, None)]);
    // 启动脚本传入的标记不列入用法说明。
    assert!(!usage().contains(LAUNCHER_FLAG));

    // 转到后台的新进程不带该标记；`--` 之后的同名参数是文件名，保留。
    let relaunched = without_launcher_flag(
        ["--launcher", "--wsl", "Ubuntu", "a", "--", "--launcher"].map(OsString::from),
    );
    assert_eq!(
        relaunched,
        ["--wsl", "Ubuntu", "a", "--", "--launcher"].map(OsString::from)
    );
    assert!(!args(&["a"]).unwrap().launcher);
}
