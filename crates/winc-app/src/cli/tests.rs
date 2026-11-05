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
    assert!(args(&["--register-file-types"]).unwrap().register_file_types);
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
