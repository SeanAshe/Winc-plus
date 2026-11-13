//! `files` 的测试：原子写入、符号链接与权限、并发写入、临时文件名、Windows 路径前缀，以及便携版的
//! 数据目录和首次运行时的导入。

use super::*;

/// 测试用的空目录，每个测试一个。
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::files::canonicalize(dir).unwrap()
}

/// 回归：保存配置曾把符号链接替换成普通文件，权限也恢复成默认值。
#[cfg(unix)]
#[test]
fn writes_follow_symlinks_and_keep_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("link");
    let real = dir.join("dotfiles-config.json");
    std::fs::write(&real, "{}").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
    let link = dir.join("config.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    write_atomic(&link, "{\"a\": 1}").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"a\": 1}");
    let mode = std::fs::metadata(&real).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
    // 目录里只有目标文件和链接，没有遗留的临时文件。
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 回归：临时文件名固定时，同时写同一个文件的多个进程会互相覆盖或删掉对方的临时文件。
#[test]
fn concurrent_writes_use_their_own_temporary_files() {
    use std::io::ErrorKind;

    let dir = scratch("concurrent");
    let path = dir.join("state.json");
    // Windows 上改名换掉一个别人正在换的文件会被拒绝，靠重试等对方换完；十几个写入方一刻不停地
    // 抢同一个文件时会超出重试的时限。这与临时文件名无关，Windows 上少用几个写入方。
    let (writers, rounds) = if cfg!(windows) { (4, 5) } else { (16, 20) };
    let writers: Vec<_> = (0..writers)
        .map(|n| {
            let path = path.clone();
            std::thread::spawn(move || {
                for _ in 0..rounds {
                    // 每个写入方一个字母，各次写入的字节数相同：据此判断读到的是哪一次完整的写入。
                    let letter = char::from(b'a' + n);
                    match write_atomic(&path, &letter.to_string().repeat(1000)) {
                        Ok(()) => {}
                        // Windows 上竞争激烈时可能超出重试的时限而放弃这次写入。放弃不损坏文件，
                        // 结尾的检查仍然成立。
                        Err(e) if cfg!(windows) && e.kind() == ErrorKind::PermissionDenied => {}
                        Err(e) => panic!("{e}"),
                    }
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    // 内容是某一次完整的写入，不是几次混在一起。
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.len(), 1000);
    assert!(text.chars().all(|c| c == text.chars().next().unwrap()));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Windows 的 `\\?\` 前缀：能不带前缀表示的路径去掉前缀，网络路径换回 `\\服务器\共享`。
///
/// 去掉后含义会变的路径（以点或空格结尾的段、超过 `MAX_PATH`、不是盘符开头）原样保留。
#[test]
fn verbatim_prefixes_are_dropped_when_safe() {
    let simple = |s: &str| simplify_verbatim(PathBuf::from(s));
    assert_eq!(
        simple(r"\\?\C:\Users\me\code"),
        PathBuf::from(r"C:\Users\me\code")
    );
    assert_eq!(
        simple(r"\\?\UNC\server\share\a"),
        PathBuf::from(r"\\server\share\a")
    );
    assert_eq!(simple(r"C:\plain"), PathBuf::from(r"C:\plain"));
    for kept in [
        r"\\?\C:\Users\dot.\x",
        r"\\?\C:\Users\space \x",
        r"\\?\Volume{1234}\x",
        r"\\?\C:",
    ] {
        assert_eq!(simple(kept), PathBuf::from(kept), "{kept}");
    }
    let long = format!(r"\\?\C:\{}", "a".repeat(300));
    assert_eq!(simple(&long), PathBuf::from(&long));
}

#[test]
fn a_data_folder_beside_the_program_makes_it_portable() {
    let dir = scratch("portable");
    let exe = dir.join("winc.exe");
    assert_eq!(portable_dir_beside(&exe), None);
    // 同名的文件不算：只认目录。
    std::fs::write(dir.join("data"), "").unwrap();
    assert_eq!(portable_dir_beside(&exe), None);
    std::fs::remove_file(dir.join("data")).unwrap();
    std::fs::create_dir(dir.join("data")).unwrap();
    assert_eq!(portable_dir_beside(&exe), Some(dir.join("data")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_first_portable_run_imports_existing_files_once() {
    let dir = scratch("import");
    let (user, state, data) = (dir.join("user"), dir.join("state"), dir.join("data"));
    for d in [&user, &state, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(user.join("config.json"), "{\"a\":1}").unwrap();
    let sources = || {
        [
            (Some(user.clone()), "config.json"),
            (Some(state.clone()), "state.json"),
        ]
    };
    // 数据目录中两个文件都没有：复制存在的那个，不存在的跳过。
    assert!(import_missing(&data, sources()));
    assert_eq!(
        std::fs::read_to_string(data.join("config.json")).unwrap(),
        "{\"a\":1}"
    );
    assert!(!data.join("state.json").exists());
    // 数据目录中已有其中一个：不再导入，已有的内容不被覆盖。
    std::fs::write(state.join("state.json"), "{}").unwrap();
    std::fs::write(user.join("config.json"), "{\"a\":2}").unwrap();
    assert!(!import_missing(&data, sources()));
    assert!(!data.join("state.json").exists());
    assert_eq!(
        std::fs::read_to_string(data.join("config.json")).unwrap(),
        "{\"a\":1}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
