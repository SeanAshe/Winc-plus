//! `wsl` 的测试：识别发行版中的路径、两种路径写法之间的换算、进入发行版的命令，以及保存时保持
//! 文件权限的脚本。

use super::*;

fn place(text: &str) -> Option<(String, String)> {
    locate(Path::new(text)).map(|p| (p.distro, p.path))
}

fn found(distro: &str, path: &str) -> Option<(String, String)> {
    Some((distro.to_owned(), path.to_owned()))
}

#[test]
fn both_host_names_are_recognized() {
    let want = found("Ubuntu", "/home/me/app");
    assert_eq!(place(r"\\wsl.localhost\Ubuntu\home\me\app"), want);
    assert_eq!(place(r"\\wsl$\Ubuntu\home\me\app"), want);
    assert_eq!(place(r"\\WSL.LOCALHOST\Ubuntu\home\me\app"), want);
    assert_eq!(place("//wsl.localhost/Ubuntu/home/me/app"), want);
    assert_eq!(place(r"\\?\UNC\wsl.localhost\Ubuntu\home\me\app"), want);
}

#[test]
fn the_root_of_a_distribution_is_slash() {
    assert_eq!(place(r"\\wsl.localhost\Ubuntu"), found("Ubuntu", "/"));
    assert_eq!(place(r"\\wsl.localhost\Ubuntu\"), found("Ubuntu", "/"));
    assert_eq!(place(r"\\wsl.localhost"), None);
}

#[test]
fn other_paths_are_not_in_a_distribution() {
    for text in [
        r"C:\Users\me",
        r"\\server\share\dir",
        "/home/me",
        "relative/wsl.localhost",
        "",
    ] {
        assert_eq!(place(text), None, "{text}");
    }
}

#[test]
fn paths_convert_back_with_the_same_host() {
    for root in [r"\\wsl.localhost\Debian\work", r"\\wsl$\Debian\work"] {
        let location = locate(Path::new(root)).unwrap();
        let back = location.windows_path(&location.path);
        assert_eq!(back, PathBuf::from(root));
        assert_eq!(locate(&back), Some(location));
    }
}

#[test]
fn names_with_spaces_stay_whole() {
    let path = r"\\wsl.localhost\Ubuntu-22.04\home\my files";
    assert_eq!(place(path), found("Ubuntu-22.04", "/home/my files"));
}

/// 其他平台不把这种写法视为发行版中的路径，`git` 的路径按普通路径处理。
#[cfg(not(windows))]
#[test]
fn only_windows_uses_the_conversion() {
    let root = Path::new(r"\\wsl.localhost\Ubuntu\repo");
    assert_eq!(locate_here(root), None);
    assert_eq!(resolve(Path::new("/repo"), ".git"), Path::new("/repo/.git"));
    assert_eq!(git_arg(Path::new("/repo"), Path::new("/repo/a")), "/repo/a");
    assert!(!available());
    assert_eq!(temp_dir(Path::new("/repo")), std::env::temp_dir());
    assert_eq!(temp_dir(root), std::env::temp_dir());
    let linux = PathBuf::from("/home/me/a.rs");
    assert_eq!(from_terminal(root, linux.clone()), linux);
    assert!(reachable(root));
}

#[test]
fn programs_run_through_a_login_shell_in_the_directory() {
    let mut command = Command::new("wsl.exe");
    enter_login(&mut command, "Ubuntu", "/home/me/app", None);
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(
        args,
        [
            "-d",
            "Ubuntu",
            "--cd",
            "/home/me/app",
            "--exec",
            "sh",
            "-lc",
            r#"exec "$0" "$@""#,
        ]
    );
    let utf8 = command.get_envs().find(|(name, _)| *name == "WSL_UTF8");
    assert_eq!(utf8.and_then(|(_, value)| value), Some("1".as_ref()));
}

#[test]
fn programs_use_the_path_of_the_login_shell_when_known() {
    let mut command = Command::new("wsl.exe");
    enter_login(
        &mut command,
        "Ubuntu",
        "/app",
        Some("/usr/bin:/home/me/.nvm/bin"),
    );
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(
        args[4..],
        [
            "--exec",
            "env",
            "PATH=/usr/bin:/home/me/.nvm/bin",
            "sh",
            "-c",
            r#"exec "$0" "$@""#,
        ]
    );
}

#[test]
fn the_watcher_runs_from_the_windows_installation() {
    let place = locate(Path::new(r"\\wsl.localhost\Ubuntu\home\me\my app")).unwrap();
    let command = watch_command(&place, Path::new(r"C:\Program Files\winc\winc-watch"));
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(
        args,
        [
            "-d",
            "Ubuntu",
            "--exec",
            "sh",
            "-c",
            RUN_WINDOWS_FILE,
            r"C:\Program Files\winc\winc-watch",
            "/home/me/my app",
        ]
    );
}

// `wslpath` 只存在于发行版中，这里用同名脚本代替，验证空格和参数的传递。
#[cfg(unix)]
#[test]
fn the_run_script_converts_the_path_and_keeps_the_arguments() {
    use std::os::unix::fs::PermissionsExt;

    let dir = crate::test_util::scratch("wsl-run-file");
    let write_script = |name: &str, text: String| {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let program = dir.join("my helper");
    write_script("my helper", "#!/bin/sh\nprintf '%s|' \"$@\"\n".to_owned());
    write_script(
        "wslpath",
        format!("#!/bin/sh\nprintf '%s' '{}'\n", program.display()),
    );
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
    let output = Command::new("sh")
        .args([
            "-c",
            RUN_WINDOWS_FILE,
            r"C:\winc\winc-watch",
            "/home/my app",
        ])
        .env("PATH", path)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "/home/my app|");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn windows_programs_on_the_windows_path_do_not_count() {
    let found = |text: &[u8]| linux_program(text);
    assert_eq!(
        found(b"/usr/bin/prettier\n"),
        Some(PathBuf::from("/usr/bin/prettier"))
    );
    // 启动脚本的输出排在结果之前。
    assert_eq!(
        found(b"hello\n/usr/bin/git\n"),
        Some(PathBuf::from("/usr/bin/git"))
    );
    assert_eq!(
        found(b"/mnt/c/Users/me/AppData/Roaming/npm/prettier\n"),
        None
    );
    assert_eq!(found(b"prettier\n"), None);
    assert_eq!(found(b""), None);
}

#[test]
fn absolute_paths_given_in_a_distribution_stay_inside_it() {
    let path = from_distro("Ubuntu", PathBuf::from("/home/me/a.rs"));
    let place = locate(&path).unwrap();
    assert_eq!(
        (place.distro.as_str(), place.path.as_str()),
        ("Ubuntu", "/home/me/a.rs")
    );
    assert_eq!(
        from_distro("Ubuntu", PathBuf::from("a.rs")),
        Path::new("a.rs")
    );
}

// 交互式登录 shell 读取的配置决定 `PATH`；`bash` 是发行版中最常见的默认 shell。
#[cfg(target_os = "linux")]
#[test]
fn the_login_shell_reports_the_path_from_the_user_profile() {
    let dir = crate::test_util::scratch("wsl-login-shell");
    std::fs::write(
        dir.join(".profile"),
        "echo 欢迎\nexport PATH=/from/profile:$PATH\n",
    )
    .unwrap();
    let output = Command::new("sh")
        .args(["-c", LOGIN_SHELL, &environment::print_script()])
        .env("SHELL", "bash")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let path = environment::path_in(&output.stdout).unwrap();
    assert!(path.starts_with("/from/profile:"), "{path}");
    let _ = std::fs::remove_dir_all(&dir);
}

// `stat -c` 是 Linux 上 `stat` 的写法，与发行版内运行脚本的环境相同。
#[cfg(target_os = "linux")]
#[test]
fn the_identity_script_copies_the_mode_and_refuses_hard_links() {
    use std::os::unix::fs::PermissionsExt;

    let dir = crate::test_util::scratch("wsl-keep-identity");
    let target = dir.join("run.sh");
    let temp = dir.join("run.sh.tmp");
    let run = || {
        Command::new("sh")
            .args(["-c", KEEP_IDENTITY, "sh"])
            .args([&target, &temp])
            .status()
            .unwrap()
            .success()
    };
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    std::fs::write(&target, "a").unwrap();
    std::fs::write(&temp, "b").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o751)).unwrap();
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).unwrap();

    assert!(run());
    assert_eq!(mode(&temp), 0o751);

    // 有多个硬链接时改名替换会使其他名字指向旧内容，因此拒绝，权限也不改动。
    std::fs::hard_link(&target, dir.join("other")).unwrap();
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!run());
    assert_eq!(mode(&temp), 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_temporary_index_reaches_git_in_the_distribution() {
    assert!(GIT_ENV.split(':').any(|name| name == "GIT_INDEX_FILE"));
}

#[cfg(windows)]
#[test]
fn windows_converts_paths_inside_a_distribution() {
    let root = Path::new(r"\\wsl.localhost\Ubuntu\repo");
    assert_eq!(
        resolve(root, "/home/me/repo/.git"),
        PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\repo\.git")
    );
    assert_eq!(resolve(root, ".git"), root.join(".git"));
    let other = Path::new(r"\\wsl.localhost\Ubuntu\repo.worktrees\feat");
    assert_eq!(git_arg(root, other), "/repo.worktrees/feat");
    assert_eq!(
        from_terminal(root, PathBuf::from("/home/me/a.rs")),
        PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\a.rs")
    );
    assert_eq!(
        from_terminal(root, PathBuf::from("a.rs")),
        PathBuf::from("a.rs")
    );
    let local = Path::new(r"C:\repo");
    assert_eq!(
        from_terminal(local, PathBuf::from("/a")),
        PathBuf::from("/a")
    );
    let temp = temp_dir(root);
    assert_eq!(temp, PathBuf::from(r"\\wsl.localhost\Ubuntu\tmp"));
    assert_eq!(
        locate_here(&temp).map(|place| place.path).as_deref(),
        Some("/tmp")
    );
}
