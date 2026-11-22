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

/// `winc://file/<绝对路径>[:行[:列]]` 链接：百分号编码解码，忽略查询和片段，Windows 盘符路径去掉
/// 开头的斜杠；没有盘符的 Windows 路径、编码不完整的链接报错，不当作文件名。
#[test]
fn winc_links_open_files_at_a_line() {
    // 绝对路径：Windows 上带盘符，链接写作 `winc://file/C:/…`。
    let (url, dir) = if cfg!(windows) {
        ("winc://file/C:/no/such", "C:/no/such")
    } else {
        ("winc://file/no/such", "/no/such")
    };
    assert_eq!(
        link(&format!("{url}/a%20b.rs:12:3?windowId=1#x")),
        Some(at(&format!("{dir}/a b.rs"), Some(11), Some(2)))
    );
    let upper = url.replacen("winc", "WINC", 1);
    assert_eq!(
        link(&format!("{upper}/dir")),
        Some(at(&format!("{dir}/dir"), None, None))
    );
    if cfg!(windows) {
        assert_eq!(link("winc://file/no/drive"), None);
    }
    for bad in [
        "winc://file".to_owned(),
        "winc://open/no/such/a.rs".to_owned(),
        format!("{url}/%e4"),
        format!("{url}/%zz"),
        "vscode://file/no/such/a.rs".to_owned(),
    ] {
        assert_eq!(link(&bad), None, "{bad}");
    }
    let parsed = args(&[&format!("{url}/a.rs:2")]).unwrap();
    assert_eq!(parsed.targets, [at(&format!("{dir}/a.rs"), Some(1), None)]);
    assert!(args(&[&format!("{url}/%zz")]).is_err());
}

/// `ssh://` 参数：主机按 `ssh` 的写法校验，路径按百分号编码解码，末尾的行号和列号拆出；按写法
/// 分出项目目录和文件；写回参数后解析出同样的目标。
#[test]
fn ssh_arguments_name_a_remote_folder_and_files() {
    let host = winc_backend::Target::new("me@devbox:2222").unwrap();
    let at = |path: &str, line, col| Target {
        path: winc_backend::virtual_path(&host, path.as_bytes()),
        line,
        col,
    };
    let parsed = args(&[
        "ssh://me@devbox:2222/home/me/project",
        "SSH://me@devbox:2222/home/me/project/src/a%20b.py:12:3?x#y",
    ])
    .unwrap();
    let (folder, file) = (
        at("/home/me/project", None, None),
        at("/home/me/project/src/a b.py", Some(11), Some(2)),
    );
    assert_eq!(parsed.targets, [folder.clone(), file.clone()]);
    let open = remote_open(&parsed.targets).unwrap();
    assert_eq!(
        open,
        RemoteOpen {
            target: host.clone(),
            folder: folder.path.clone(),
            files: vec![file.clone()],
        }
    );
    for target in [&folder, &file] {
        let again = parse([argument(target)]).unwrap();
        assert_eq!(again.targets, std::slice::from_ref(target));
    }

    // IPv6 地址。
    let scoped = winc_backend::Target::new("me@[fe80::1%eth0]:2222").unwrap();
    let parsed = args(&["ssh://me@[fe80::1%25eth0]:2222/srv/app/a.rs:3"]).unwrap();
    assert_eq!(
        parsed.targets[0].path,
        winc_backend::virtual_path(&scoped, b"/srv/app/a.rs")
    );
    assert_eq!(parsed.targets[0].line, Some(2));

    // 只有一个不带行号的参数是目录；只给出文件时项目是它所在的目录；`%3A` 不当作行号。
    let one = |text: &str| remote_open(&args(&[text]).unwrap().targets).unwrap();
    let open = one("ssh://me@devbox:2222/srv/app");
    assert_eq!(
        (open.folder, open.files),
        (at("/srv/app", None, None).path, vec![])
    );
    let open = one("ssh://me@devbox:2222/srv/app/main.rs:3");
    let main = at("/srv/app/main.rs", Some(2), None);
    assert_eq!(
        (open.folder, open.files),
        (at("/srv/app", None, None).path, vec![main])
    );
    let twice = args(&[
        "ssh://me@devbox:2222/srv/app",
        "ssh://me@devbox:2222/srv/app",
    ])
    .unwrap();
    assert_eq!(remote_open(&twice.targets).unwrap().files, []);
    let colon = args(&["ssh://me@devbox:2222/srv/a%3A3"]).unwrap();
    assert_eq!(colon.targets, [at("/srv/a:3", None, None)]);

    // 主机以 `-` 开头会被 `ssh` 当作选项，不接受；没有路径、编码不完整时同样报错。
    for bad in [
        "ssh://-oProxyCommand=x/srv",
        "ssh://devbox",
        "ssh://dev box/srv",
        "ssh://devbox/srv/%zz",
    ] {
        assert!(args(&[bad]).is_err(), "{bad}");
    }
    // 不能与本机路径、其他主机、`--wait` 一起使用；两个互不包含的目录无法决定项目。
    for list in [
        &["ssh://devbox/srv", "/no/such/local"][..],
        &["/no/such/local", "ssh://devbox/srv"],
        &["ssh://devbox/srv", "ssh://other/srv/a.rs:1"],
        &["-w", "ssh://devbox/srv/a.rs:1"],
        &["ssh://devbox/srv/a", "ssh://devbox/srv/b"],
    ] {
        assert!(args(list).is_err(), "{list:?}");
    }
}
