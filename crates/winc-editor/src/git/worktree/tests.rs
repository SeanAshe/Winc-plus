//! 工作树的测试：解析 `worktree list` 的输出、默认目录与避免重名、复制被忽略的文件、移除前的
//! 检查结果。

use super::*;

#[test]
fn worktree_list_reads_branches_detached_and_states() {
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let output = format!(
        "worktree /repo\0HEAD {a}\0branch refs/heads/main\0\0\
         worktree /repo.worktrees/fix\0HEAD {b}\0detached\0locked moving disks\0\0\
         worktree /gone\0HEAD {a}\0branch refs/heads/feat/x\0prunable gitdir file points to non-existent location\0\0\
         worktree /bare.git\0bare\0\0"
    );
    let list = parse_worktrees(output.as_bytes(), b'\0');
    assert_eq!(list.len(), 4);
    assert_eq!(list[0].path, Path::new("/repo"));
    assert_eq!(list[0].branch(), Some("main"));
    assert_eq!(list[1].head, Some(Head::Detached("bbbbbbb".into())));
    assert!(list[1].locked && !list[1].prunable);
    assert_eq!(list[2].branch(), Some("feat/x"));
    assert!(list[2].prunable && !list[2].locked);
    assert_eq!(list[3].head, None);
    assert!(parse_worktrees(b"", b'\0').is_empty());
}

/// `git` 2.36 以前的输出以换行分隔，没有 `prunable`，`locked` 不带原因。
#[test]
fn worktree_list_reads_the_output_without_nul() {
    let a = "a".repeat(40);
    let output = format!(
        "worktree /repo\nHEAD {a}\nbranch refs/heads/main\n\n\
         worktree /repo.worktrees/fix\nHEAD {a}\ndetached\nlocked\n\n"
    );
    let list = parse_worktrees(output.as_bytes(), b'\n');
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].path, Path::new("/repo"));
    assert_eq!(list[0].branch(), Some("main"));
    assert_eq!(list[1].path, Path::new("/repo.worktrees/fix"));
    assert_eq!(list[1].head, Some(Head::Detached("aaaaaaa".into())));
    assert!(list[1].locked);
}

/// 只有一个工作树时不运行 `git`（返回空列表）；加了一个之后两个都列出来，路径是解析过符号
/// 链接的。
#[test]
fn lists_the_real_worktrees() {
    let Some(git) = super::super::tests::real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("worktrees");
    let main = dir.join("repo");
    std::fs::create_dir_all(&main).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "first"]);
    let cancel = Cancel::default();
    assert!(git.worktrees(&main, &cancel).unwrap().is_empty());

    let linked = dir.join("repo.worktrees").join("feat");
    sh(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feat",
        linked.to_str().unwrap(),
    ]);
    let list = git.worktrees(&main, &cancel).unwrap();
    let real = |p: &Path| winc_config::files::canonicalize(p).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0].path.clone(), list[0].branch()),
        (real(&main), Some("main"))
    );
    assert_eq!(
        (list[1].path.clone(), list[1].branch()),
        (real(&linked), Some("feat"))
    );
    // 在附属工作树里看到的是同一份列表，主工作树仍在最前面。
    assert_eq!(git.worktrees(&linked, &cancel).unwrap(), list);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_worktrees_go_next_to_the_repository_with_unique_names() {
    let dir = crate::test_util::scratch("worktree-dir");
    let main = dir.join("winc");
    assert_eq!(
        worktree_dir(&main, None, None, "feat/x"),
        dir.join("winc.worktrees").join("feat-x")
    );
    // 裸仓库去掉 `.git`；配置了目录时放在它下面按仓库分开，相对路径相对于主工作树。
    let other = dir.join("elsewhere");
    assert_eq!(
        worktree_dir(&dir.join("winc.git"), other.to_str(), None, "main"),
        other.join("winc").join("main")
    );
    assert_eq!(
        worktree_dir(&main, Some("../trees"), None, "main"),
        main.join("../trees").join("winc").join("main")
    );
    // 已有同名的目录时加 `-2`、`-3`。
    let taken = dir.join("winc.worktrees").join("fix");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::create_dir_all(taken.with_file_name("fix-2")).unwrap();
    assert_eq!(
        worktree_dir(&main, None, None, "fix"),
        taken.with_file_name("fix-3")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 配置的目录以 `~/` 开头时放在仓库所在系统的主目录下；主目录未知时用默认位置。
#[test]
fn configured_roots_starting_with_a_tilde_use_the_given_home() {
    let main = Path::new("/work/winc");
    let home = Path::new("/home/me");
    assert_eq!(
        worktree_parent(main, Some("~/trees"), Some(home)),
        home.join("trees").join("winc")
    );
    assert_eq!(
        worktree_parent(main, Some("~/trees"), None),
        Path::new("/work/winc.worktrees")
    );
}

/// 配置的目录与仓库不在同一系统中时用默认位置；对发行版中的仓库，`~/` 指发行版内的主目录。
#[cfg(windows)]
#[test]
fn configured_roots_on_another_system_are_not_used() {
    let main = Path::new(r"\\wsl.localhost\Ubuntu\home\me\winc");
    let beside = Path::new(r"\\wsl.localhost\Ubuntu\home\me\winc.worktrees");
    assert_eq!(worktree_parent(main, Some(r"D:\trees"), None), beside);
    let windows_home = Path::new(r"C:\Users\me");
    assert_eq!(
        worktree_parent(main, Some("~/trees"), Some(windows_home)),
        beside
    );
    let linux_home = Path::new(r"\\wsl.localhost\Ubuntu\home\me");
    assert_eq!(
        worktree_parent(main, Some("~/trees"), Some(linux_home)),
        linux_home.join("trees").join("winc")
    );
    // 发行版内的绝对路径接在仓库的访问路径上，仍在同一发行版中。
    assert_eq!(
        worktree_parent(main, Some("/srv/trees"), None),
        Path::new(r"\\wsl.localhost\Ubuntu\srv\trees\winc")
    );
    let local = Path::new(r"C:\work\winc");
    assert_eq!(
        worktree_parent(local, Some(r"\\wsl.localhost\Ubuntu\trees"), None),
        Path::new(r"C:\work\winc.worktrees")
    );
}

/// `git` 不认识 `worktree list -z`（2.36 以前）时改用以换行分隔的输出，结果相同。
#[cfg(unix)]
#[test]
fn old_git_lists_worktrees_without_nul() {
    use std::os::unix::fs::PermissionsExt;

    let Some(git) = super::super::tests::real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("worktrees-old-git");
    let main = dir.join("repo");
    std::fs::create_dir_all(&main).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "first"]);
    let linked = dir.join("repo.worktrees").join("feat");
    sh(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feat",
        linked.to_str().unwrap(),
    ]);

    let script = dir.join("git");
    let body = format!(
        "#!/bin/sh\n\
         case \" $* \" in *' worktree list '*' -z '*) echo 'error: unknown switch' >&2; exit 129;; esac\n\
         exec '{}' \"$@\"\n",
        git.program.display()
    );
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let rejected = Command::new(&script)
        .arg("-C")
        .arg(&main)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(129));

    let old = Git {
        program: script,
        ..git.clone()
    };
    let cancel = Cancel::default();
    let list = old.worktrees(&main, &cancel).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list, git.worktrees(&main, &cancel).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 新建的工作树复制被忽略、匹配 glob 的文件；移除前数出改动、判断分支是否已合并。
#[test]
fn copies_ignored_files_and_checks_before_removing() {
    let Some(git) = super::super::tests::real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("worktree-copy");
    let main = dir.join("repo");
    std::fs::create_dir_all(main.join("config")).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    std::fs::write(main.join(".gitignore"), ".env*\ntarget/\nconfig/\n").unwrap();
    std::fs::write(main.join("README"), "hi").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "first"]);
    std::fs::write(main.join(".env"), "KEY=1").unwrap();
    std::fs::write(main.join(".env.local"), "KEY=2").unwrap();
    std::fs::write(main.join("config").join("local.toml"), "x").unwrap();
    std::fs::create_dir_all(main.join("target")).unwrap();
    std::fs::write(main.join("target").join("big"), "x").unwrap();

    let linked = dir.join("repo.worktrees").join("feat");
    sh(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feat",
        linked.to_str().unwrap(),
    ]);
    let cancel = Cancel::default();
    let globs = [".env*".to_owned(), "config".to_owned()];
    assert_eq!(git.copy_ignored(&main, &linked, &globs, &cancel), Ok(3));
    assert_eq!(
        std::fs::read_to_string(linked.join(".env.local")).unwrap(),
        "KEY=2"
    );
    assert!(linked.join("config").join("local.toml").is_file());
    assert!(!linked.join("target").exists());
    // 再复制一次时不覆盖 `to` 里已有的同名文件。
    std::fs::write(linked.join(".env"), "mine").unwrap();
    assert_eq!(git.copy_ignored(&main, &linked, &globs, &cancel), Ok(0));
    assert_eq!(
        std::fs::read_to_string(linked.join(".env")).unwrap(),
        "mine"
    );
    assert!(
        git.copy_ignored(&main, &linked, &["a[".to_owned()], &cancel)
            .is_err()
    );
    // 新工作树位于被忽略的目录 `target/` 里面时不复制 `target/`，否则会无休止地嵌套。
    let inside = main.join("target").join("wt");
    std::fs::create_dir_all(&inside).unwrap();
    let target = ["target".to_owned()];
    assert_eq!(git.copy_ignored(&main, &inside, &target, &cancel), Ok(0));

    let list = git.worktrees(&main, &cancel).unwrap();
    // 新分支仍在 `main` 上，算已合并；改动一个文件、新增一个未跟踪文件后各计数为 1。
    std::fs::write(linked.join("README"), "changed").unwrap();
    std::fs::write(linked.join("new.txt"), "x").unwrap();
    let check = git.check_worktree(&main, &list[1], &cancel).unwrap();
    assert_eq!(
        check,
        WorktreeCheck {
            changed: 1,
            untracked: 1,
            merged: true,
            unreachable: 0,
        }
    );
    // 游离地提交两次：这两个提交不在任何分支上，移除工作树后无法再找回。
    let commit = |dir: &Path| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(["commit", "-q", "--allow-empty", "-m", "x"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
    };
    let detached = dir.join("repo.worktrees").join("detached");
    sh(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        detached.to_str().unwrap(),
    ]);
    commit(&detached);
    commit(&detached);
    let list = git.worktrees(&main, &cancel).unwrap();
    let i = list.iter().position(|w| w.branch().is_none()).unwrap();
    assert_eq!(
        git.check_worktree(&main, &list[i], &cancel)
            .unwrap()
            .unreachable,
        2
    );
    // 有上游的分支看上游：分支已合并到 `HEAD`，但上游落后时不能用 `-d` 删除。
    sh(&["remote", "add", "origin", main.to_str().unwrap()]);
    sh(&["update-ref", "refs/remotes/origin/feat", "HEAD"]);
    sh(&["branch", "--set-upstream-to=origin/feat", "feat"]);
    commit(&linked);
    sh(&["merge", "-q", "--ff-only", "feat"]);
    let list = git.worktrees(&main, &cancel).unwrap();
    let i = list
        .iter()
        .position(|w| w.branch() == Some("feat"))
        .unwrap();
    assert!(!git.check_worktree(&main, &list[i], &cancel).unwrap().merged);

    // 上游引用已不存在（远程分支被删除后获取时清掉了）：和 `git branch -d` 一样改看 `HEAD`，
    // 检查不会失败。
    sh(&["update-ref", "-d", "refs/remotes/origin/feat"]);
    let list = git.worktrees(&main, &cancel).unwrap();
    let i = list
        .iter()
        .position(|w| w.branch() == Some("feat"))
        .unwrap();
    assert!(git.check_worktree(&main, &list[i], &cancel).unwrap().merged);
    commit(&linked);
    assert!(!git.check_worktree(&main, &list[i], &cancel).unwrap().merged);
    let _ = std::fs::remove_dir_all(&dir);
}
