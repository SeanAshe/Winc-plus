//! 在 `PATH` 中查找程序的测试：只考虑绝对路径的目录、Windows 上按 `PATHEXT` 补扩展名。

use super::*;
#[cfg(unix)]
use crate::test_util::scratch;
#[cfg(unix)]
use crate::tools::format::tests::fake_tool;

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn programs_are_found_in_absolute_path_directories() {
    let root = scratch("find-in-path");
    let tool = fake_tool(&root.join("bin"), "prettier", "cat");
    let path = std::env::join_paths([root.join("nope"), root.join("bin")]).unwrap();
    assert_eq!(find_in_path("prettier", Some(&path)), Some(tool.clone()));
    assert!(find_in_path("jq", Some(&path)).is_none());
    assert!(find_in_path("prettier", None).is_none());
    // 相对路径的目录不算：其含义随当前目录而变。
    assert!(find_in_path("prettier", Some(OsStr::new("bin"))).is_none());

    // `Shell::find` 在给定环境的 `PATH` 里找，不运行登录 shell。
    let shell = Shell::with_vars(vec![("PATH".into(), path)]);
    assert!(shell.is_ready());
    assert_eq!(shell.find("prettier"), Some(tool));
    assert_eq!(shell.find("jq"), None);
    assert_eq!(Shell::with_vars(Vec::new()).find("prettier"), None);
}

#[test]
fn windows_finds_the_path_variable_whatever_its_case() {
    let vars: Vars = vec![
        ("Other".into(), "x".into()),
        ("Path".into(), "C:\\Git\\cmd".into()),
    ];
    assert_eq!(path_value(&vars, true), Some(OsStr::new("C:\\Git\\cmd")));
    assert_eq!(path_value(&vars, false), None);
}

#[test]
fn windows_programs_are_looked_up_with_their_extensions() {
    assert_eq!(
        windows_names("prettier", ".COM;.EXE;.BAT;.CMD;.VBS"),
        [
            "prettier.com",
            "prettier.exe",
            "prettier.bat",
            "prettier.cmd",
            "prettier.vbs"
        ]
    );
    assert_eq!(windows_names("x", ";;EXE;.Cmd"), ["x.cmd"]);
    // 其他系统上就是程序名本身。
    if cfg!(not(windows)) {
        assert_eq!(file_names("prettier", None), ["prettier"]);
    }
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn the_listing_answers_like_a_search_of_the_path() {
    use std::os::unix::fs::PermissionsExt;

    let root = scratch("path-listing");
    let (first, second) = (root.join("first"), root.join("second"));
    let tool = fake_tool(&second, "prettier", "cat");
    // 排在前面的目录里有同名但不可执行的文件：跳过，使用后面目录里的。
    std::fs::create_dir_all(&first).unwrap();
    std::fs::write(first.join("prettier"), "").unwrap();
    // 不存在的目录、重复的目录、相对路径的目录都不影响结果。
    let dirs = [
        &root.join("nope"),
        &first,
        &second,
        &first,
        Path::new("bin"),
    ];
    let path = std::env::join_paths(dirs).unwrap();
    let shell = Shell::with_vars(vec![("PATH".into(), path)]);
    for name in ["prettier", "jq", "Prettier"] {
        assert_eq!(shell.peek(name), Some(shell.find(name)), "{name}");
    }
    assert_eq!(shell.peek("prettier"), Some(Some(tool.clone())));

    // 列出之后才安装的程序：重新列出后才能找到，清单没有变化时 `list` 返回假。
    let ruff = fake_tool(&first, "ruff", "cat");
    assert_eq!(shell.peek("ruff"), Some(None));
    assert!(shell.list());
    assert!(!shell.list());
    assert_eq!(shell.peek("ruff"), Some(Some(ruff)));
    // 在后台找到清单里没有的程序时，清单随之更新。
    let black = fake_tool(&second, "black", "cat");
    assert_eq!(shell.find("black"), Some(black.clone()));
    assert_eq!(shell.peek("black"), Some(Some(black)));

    // 卸载的程序不必重新列出：命中清单后仍检查文件本身。
    std::fs::remove_file(&tool).unwrap();
    assert_eq!(shell.peek("prettier"), Some(None));

    // 无法列出的目录（没有读取权限）：直接检查其中的文件。
    let sealed = root.join("sealed");
    let jq = fake_tool(&sealed, "jq", "cat");
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o111)).unwrap();
    let shell = Shell::with_vars(vec![("PATH".into(), sealed.clone().into())]);
    let listable = std::fs::read_dir(&sealed).is_ok(); // 以 root 运行时仍能列出
    assert_eq!(shell.peek("jq"), Some(Some(jq)));
    assert_eq!(shell.peek("black"), Some(None));
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(listable || shell.list());

    // 尚未取到环境时不知道，也不等待。
    assert_eq!(Shell::default().peek("prettier"), None);
}
