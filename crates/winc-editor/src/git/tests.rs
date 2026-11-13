//! `git` 模块的测试：解析各条命令的输出、在真实仓库里提交与回滚、只提交一部分、信任检查，
//! 分批读取日志，以及操作的 askpass 程序和输出记录。
//!
//! 依赖本机 `git` 的测试在找不到 `git` 时跳过。

use super::*;

mod history;

fn root() -> PathBuf {
    PathBuf::from("/repo")
}

#[test]
fn commit_files_read_names_renames_and_line_counts() {
    let raw = ":100644 100644 aaa bbb M\0src/a.rs\0\
                   :100644 100644 aaa bbb R087\0old.rs\0new.rs\0\
                   :000000 100644 000 bbb A\0img.png\0\
                   :100644 000000 aaa 000 D\0gone.rs\0";
    let counts = "3\t1\tsrc/a.rs\x002\t2\t\0old.rs\0new.rs\0-\t-\timg.png\x000\t4\tgone.rs\0";
    let files = parse_diff_tree(format!("{raw}{counts}").as_bytes(), &root());
    assert_eq!(files.len(), 4);
    assert_eq!(
        (files[0].change, files[0].lines),
        (Change::Modified, Some((3, 1)))
    );
    assert_eq!(files[1].change, Change::Renamed);
    assert_eq!(files[1].from, Some(root().join("old.rs")));
    assert_eq!(files[1].lines, Some((2, 2)));
    assert_eq!(files[1].path, root().join("new.rs"));
    // 二进制文件没有行数。
    assert_eq!((files[2].change, files[2].lines), (Change::Added, None));
    assert_eq!(
        (files[3].change, files[3].lines),
        (Change::Deleted, Some((0, 4)))
    );
    // 只有文件、没有行数（输出被截断）时行数为空，不错位。
    let files = parse_diff_tree(b":100644 100644 aaa bbb M\0a.rs\0", &root());
    assert_eq!((files.len(), files[0].lines), (1, None));
}

#[test]
fn parses_branch_and_every_kind_of_entry() {
    let output = [
        "# branch.oid 1234567890abcdef",
        "# branch.head main",
        "# branch.upstream origin/main",
        "# branch.ab +2 -1",
        "1 .M N... 100644 100644 100644 aaa bbb src/main.rs",
        "1 A. N... 000000 100644 100644 000 bbb new file.rs",
        "2 R. N... 100644 100644 100644 aaa aaa R100 src/new.rs",
        "src/old.rs",
        "u UU N... 100644 100644 100644 100644 a b c conflict.rs",
        "? notes/",
        "? todo.txt",
        "! target/",
        "! src/cache.tmp",
        "",
    ]
    .join("\0");
    let status = parse_status(output.as_bytes(), &root()).unwrap();
    assert_eq!(status.head, Some(Head::Branch("main".into())));
    assert_eq!((status.ahead, status.behind), (2, 1));
    let shown: Vec<_> = status
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.staged, e.unstaged))
        .collect();
    assert_eq!(
        shown,
        [
            (root().join("src/main.rs"), None, Some(Change::Modified)),
            (root().join("new file.rs"), Some(Change::Added), None),
            (root().join("src/new.rs"), Some(Change::Renamed), None),
            (root().join("conflict.rs"), None, Some(Change::Conflicted)),
            (root().join("notes"), None, Some(Change::Untracked)),
            (root().join("todo.txt"), None, Some(Change::Untracked)),
        ]
    );
    assert_eq!(status.entries[2].from, Some(root().join("src/old.rs")));
    // 只有未跟踪的整个目录（以 `/` 结尾的那一项）记成目录。
    let dirs: Vec<bool> = status.entries.iter().map(|e| e.is_dir).collect();
    assert_eq!(dirs, [false, false, false, false, true, false]);
    // 被忽略的项不计为改动，单独列在 `ignored` 里；目录的路径不含末尾的 `/`。
    assert_eq!(
        status.ignored,
        [root().join("target"), root().join("src/cache.tmp")]
    );
}

#[test]
fn detached_head_shows_the_short_commit() {
    let output = "# branch.oid 1234567890abcdef\0# branch.head (detached)\0";
    let status = parse_status(output.as_bytes(), &root()).unwrap();
    assert_eq!(status.head, Some(Head::Detached("1234567".into())));
    assert_eq!(status.oid.as_deref(), Some("1234567890abcdef"));
    let unborn = "# branch.oid (initial)\0# branch.head main\0";
    let status = parse_status(unborn.as_bytes(), &root()).unwrap();
    assert_eq!(status.head, Some(Head::Branch("main".into())));
}

#[test]
fn status_keeps_only_the_first_changes_past_the_limit() {
    // 未被忽略的大目录：只保留前 `STATUS_LIMIT` 个，并标记为截断。
    // 超出上限后重命名的原路径仍被读取，之后的字段不错位。
    let mut output = String::from("# branch.oid (initial)\0# branch.head main\0");
    for i in 0..STATUS_LIMIT + 5 {
        output.push_str(&format!("? data/{i}.csv\0"));
    }
    output.push_str("2 R. N... 100644 100644 100644 a b R100 new.rs\0old.rs\0! target/\0");
    let status = parse_status(output.as_bytes(), &root()).unwrap();
    assert_eq!(status.entries.len(), STATUS_LIMIT);
    assert!(status.truncated);
    assert_eq!(status.ignored, [root().join("target")]);
}

#[test]
fn directories_take_the_most_visible_state_but_not_deletions() {
    let entry = |path: &str, staged, unstaged| Entry {
        path: root().join(path),
        from: None,
        staged,
        unstaged,
        is_dir: false,
    };
    let status = Status {
        entries: vec![
            // 路径有三层：各级目录都要有标记，不能只标直接父目录。
            entry("a/b/c/x.rs", None, Some(Change::Untracked)),
            entry("a/y.rs", Some(Change::Added), Some(Change::Modified)),
            entry("c/gone.rs", None, Some(Change::Deleted)),
            // 优先级低的条目在前、高的在后：上层目录的标记取优先级最高的那一个。
            entry("d/e/new.rs", None, Some(Change::Untracked)),
            entry("d/e/f/edit.rs", None, Some(Change::Modified)),
        ],
        ..Status::default()
    };
    let map = status.decorations(&root());
    let at = |p: &str| map.get(&root().join(p)).copied();
    assert_eq!(at("a/b/c"), Some(Change::Untracked));
    assert_eq!(at("a/b"), Some(Change::Untracked));
    assert_eq!(at("a"), Some(Change::Modified));
    assert_eq!(at("a/y.rs"), Some(Change::Modified));
    assert_eq!(at("c"), None);
    assert_eq!(at("d/e/f"), Some(Change::Modified));
    assert_eq!(at("d/e"), Some(Change::Modified));
    assert_eq!(at("d"), Some(Change::Modified));
    assert_eq!(map.get(&root()), None);
}

#[test]
fn branches_list_local_first_and_skip_remote_head() {
    let output = "refs/heads/main\0*\0origin/main\0aaa\nrefs/heads/dev\0 \0\0bbb\n\
                      refs/remotes/origin/HEAD\0 \0\0aaa\nrefs/remotes/origin/main\0 \0\0aaa\n";
    let branches = parse_branches(output);
    let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["dev", "main", "origin/main"]);
    assert!(branches[1].current && !branches[0].current);
    assert_eq!(branches[1].upstream.as_deref(), Some("origin/main"));
    assert_eq!(
        (branches[0].oid.as_str(), branches[1].oid.as_str()),
        ("bbb", "aaa")
    );
    assert!(branches[2].remote);
}

#[test]
fn an_upstream_that_no_longer_exists_counts_as_none() {
    let output = "refs/heads/gone\0 \0origin/gone\0a\nrefs/heads/kept\0 \0origin/kept\0b\n\
                  refs/heads/local\0 \0kept\0c\nrefs/heads/origin/kept\0 \0remotes/origin/kept\0d\n\
                  refs/remotes/origin/kept\0 \0\0b\n";
    let branches = parse_branches(output);
    let upstream = |name: &str| {
        let branch = branches.iter().find(|b| b.name == name).unwrap();
        branch.upstream.as_deref()
    };
    assert_eq!(upstream("gone"), None);
    assert!(branches.iter().any(|b| b.name == "gone" && b.gone));
    assert!(branches.iter().all(|b| b.name == "gone" || !b.gone));
    assert_eq!(upstream("kept"), Some("origin/kept"));
    // 上游可以是本地分支；和本地分支同名时 `git` 写成带 `remotes/` 的形式。
    assert_eq!(upstream("local"), Some("kept"));
    assert_eq!(upstream("origin/kept"), Some("remotes/origin/kept"));
}

#[test]
fn failures_read_utf16_messages_from_wsl() {
    let message: Vec<u8> = "There is no distribution\r\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(failure(&message), "There is no distribution");
}

#[test]
fn failures_pick_the_line_that_explains() {
    let push = b"To github.com:a/b.git\n ! [rejected]        main -> main (fetch first)\n\
                     error: failed to push some refs to 'github.com:a/b.git'\nhint: Updates were rejected\n";
    // 被拒绝的那一行说明要先拉取，比「failed to push」更能说明原因。
    assert_eq!(
        failure(push),
        "! [rejected]        main -> main (fetch first)"
    );
    assert_eq!(
        failure(b"\nfatal: not a git repository\n"),
        "not a git repository"
    );
    // 以冒号结尾的说明后面是 `git` 列出的文件，接在冒号后面，多于三个时其余写成省略号；其后
    // 不缩进的提示不算。翻译后的说明以全角冒号结尾，同样处理。
    let untracked = b"error: The following untracked working tree files would be overwritten \
                      by merge:\n\tnew.txt\n\tsrc/lib.rs\nPlease move or remove them before you merge.\n\
                      Aborting\n";
    assert_eq!(
        failure(untracked),
        "The following untracked working tree files would be overwritten by merge: new.txt, \
         src/lib.rs"
    );
    let many = "error: 工作区中下列未跟踪的文件将会因为合并操作而被覆盖：\n\ta\n\tb\n\tc\n\td\n";
    assert_eq!(
        failure(many.as_bytes()),
        "工作区中下列未跟踪的文件将会因为合并操作而被覆盖：a, b, c, …"
    );
    assert_eq!(
        summary(b"Updating a..b\nFast-forward\n file | 1 +\n"),
        "file | 1 +"
    );
}

/// 返回不依赖本机全局配置的 `git`：给定提交身份，关掉提交签名。
///
/// 找不到 `git` 时返回 `None`。
pub(super) fn real_git() -> Option<Git> {
    let mut vars: Vec<_> = std::env::vars_os().collect();
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "t"),
        ("GIT_AUTHOR_EMAIL", "t@t"),
        ("GIT_COMMITTER_NAME", "t"),
        ("GIT_COMMITTER_EMAIL", "t@t"),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "commit.gpgsign"),
        ("GIT_CONFIG_VALUE_0", "false"),
    ] {
        vars.push((k.into(), v.into()));
    }
    Git::find(&Shell::with_vars(vars)).ok()
}

/// 在临时目录里建立一个真实的仓库并运行 `git`。
///
/// 找不到 `git` 时跳过。
#[test]
fn runs_the_real_git() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("src/a.rs"), "one\n").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "first"]);
    std::fs::write(dir.join("src/a.rs"), "two\n").unwrap();
    std::fs::write(dir.join("b.rs"), "new\n").unwrap();

    let root = winc_config::files::canonicalize(&dir).unwrap();
    assert_eq!(find_root(&root.join("src")), Some(root.clone()));
    let status = git.status(&root, &Cancel::default()).unwrap();
    assert_eq!(status.head, Some(Head::Branch("main".into())));
    let paths: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(paths, [root.join("src/a.rs"), root.join("b.rs")]);

    let cancel = Cancel::default();
    let staged = git.content_at(&root, "", &root.join("src/a.rs"), &cancel);
    assert_eq!(staged, Ok(Some(b"one\n".to_vec())));
    let untracked = git.head_content(&root, &root.join("b.rs"), &cancel);
    assert_eq!(untracked, Ok(None));
    assert!(status.entries[1].untracked());

    // 另一个文件已在暂存区：只提交 a.rs 时该文件的暂存状态保持不变。
    std::fs::write(dir.join("c.rs"), "staged\n").unwrap();
    sh(&["add", "c.rs"]);
    let a = root.join("src/a.rs");
    let oid = git.commit(
        &root,
        "second\n\n正文",
        std::slice::from_ref(&a),
        &[],
        &[],
        false,
    );
    assert!(oid.as_ref().is_ok_and(|o| o.len() >= 7), "{oid:?}");
    let status = git.status(&root, &cancel).unwrap();
    let left: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(left, [root.join("c.rs"), root.join("b.rs")]);
    assert_eq!(git.last_message(&root), "second\n\n正文");

    // 提交未跟踪的文件：先加入暂存区再提交；修改上一次提交时只改提交说明。
    let b = root.join("b.rs");
    git.commit(
        &root,
        "third",
        std::slice::from_ref(&b),
        std::slice::from_ref(&b),
        &[],
        false,
    )
    .unwrap();
    git.commit(&root, "third, amended", &[], &[], &[], true)
        .unwrap();
    assert_eq!(git.last_message(&root), "third, amended");
    // 以 `#` 开头的行属于提交说明的正文，不能当作注释删除。
    git.commit(&root, "#42 修复登录\n\n# 不是注释", &[], &[], &[], true)
        .unwrap();
    assert_eq!(git.last_message(&root), "#42 修复登录\n\n# 不是注释");
    // 没有可提交的内容时返回错误，原因取自 `git` 的输出。
    assert!(
        git.commit(&root, "nothing", std::slice::from_ref(&a), &[], &[], false)
            .is_err()
    );

    // 回滚：改过的文件恢复为 `HEAD` 的内容，新加入暂存区的文件移除出暂存区（工作区里的
    // 文件保留）。
    std::fs::write(&a, "changed again\n").unwrap();
    // 名字里的 `[`、`*` 不按通配符解释：`[cd].rs` 当通配符时还会匹配上面暂存的 c.rs。
    let d = root.join("[cd].rs");
    std::fs::write(&d, "new\n").unwrap();
    sh(&["add", "--", ":(literal)[cd].rs"]);
    git.rollback(&root, std::slice::from_ref(&a), std::slice::from_ref(&d))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "two\n");
    let status = git.status(&root, &cancel).unwrap();
    // `a.rs` 不再算改动；`[cd].rs` 恢复为未跟踪；此前暂存的 `c.rs` 不受影响。
    assert!(status.entries.iter().all(|e| e.path != a));
    let d = status.entries.iter().find(|e| e.path == d).unwrap();
    assert!(d.untracked());
    let c = status.entries.iter().find(|e| e.path == root.join("c.rs"));
    assert_eq!(c.and_then(|c| c.staged), Some(Change::Added));

    // 只提交一部分：`e.rs` 有两处改动，只提交第一处。提交里只含第一处，工作区保留两处
    // 改动，暂存区里别的文件（`c.rs`）不受影响。
    let e = root.join("e.rs");
    std::fs::write(&e, "1\n2\n3\n4\n5\n6\n7\n8\n").unwrap();
    sh(&["add", "e.rs"]);
    sh(&["commit", "-q", "-m", "e", "--", "e.rs"]);
    let worktree = "1\nTWO\n3\n4\n5\n6\nSEVEN\n8\n";
    std::fs::write(&e, worktree).unwrap();
    let first = "1\nTWO\n3\n4\n5\n6\n7\n8\n";
    let part = Partial {
        path: e.clone(),
        content: first.as_bytes().to_vec(),
    };
    git.commit(
        &root,
        "only the first",
        std::slice::from_ref(&e),
        &[],
        std::slice::from_ref(&part),
        false,
    )
    .unwrap();
    let head = git.head_content(&root, &e, &cancel).unwrap().unwrap();
    assert_eq!(String::from_utf8(head).unwrap(), first);
    assert_eq!(std::fs::read_to_string(&e).unwrap(), worktree);
    let status = git.status(&root, &cancel).unwrap();
    let e_entry = status.entries.iter().find(|x| x.path == e).unwrap();
    assert_eq!(
        (e_entry.staged, e_entry.unstaged),
        (None, Some(Change::Modified))
    );
    let c = status.entries.iter().find(|x| x.path == root.join("c.rs"));
    assert_eq!(c.and_then(|c| c.staged), Some(Change::Added));
    // `core.autocrlf`：工作区里是 CRLF、提交里是 LF；取 `HEAD` 版本时做同样的转换，两边
    // 内容一致。
    sh(&["config", "core.autocrlf", "true"]);
    let crlf = root.join("crlf.txt");
    std::fs::write(&crlf, "one\r\ntwo\r\n").unwrap();
    sh(&["add", "crlf.txt"]);
    sh(&["commit", "-q", "-m", "crlf"]);
    let head = git.head_content(&root, &crlf, &cancel).unwrap().unwrap();
    assert_eq!(head, std::fs::read(&crlf).unwrap());
    // 工作区文件为 LF（由其他程序写入，不经检出）：`git` 判定文件未修改；`HEAD` 版本按检出方向
    // 转换为 CRLF，两侧只差换行符写法，逐行比较没有改动。追加一行后，改动只有这一行。
    let lf = root.join("lf.md");
    std::fs::write(&lf, "a\nb\nc\n").unwrap();
    sh(&["add", "lf.md"]);
    sh(&["commit", "-q", "-m", "lf"]);
    let clean = git.status(&root, &cancel).unwrap();
    assert!(
        !clean.entries.iter().any(|e| e.path == lf),
        "{:?}",
        clean.entries
    );
    let head = git.head_content(&root, &lf, &cancel).unwrap().unwrap();
    assert_eq!(head, b"a\r\nb\r\nc\r\n");
    let head = String::from_utf8(head).unwrap();
    assert!(winc_core::diff::line_hunks(&head, "a\nb\nc\n").is_empty());
    let hunks = winc_core::diff::line_hunks(&head, "a\nb\nc\nd\n");
    assert_eq!(hunks.len(), 1);
    assert_eq!((hunks[0].old.clone(), hunks[0].new.clone()), (3..3, 3..4));

    // 分支：新建、切换、列出。
    git.operate(&root, &["switch", "-c", "feature"]).unwrap();
    let branches = git.branches(&root, &cancel).unwrap();
    let current: Vec<_> = branches.iter().filter(|b| b.current).collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].name, "feature");
    assert!(git.operate(&root, &["switch", "no-such-branch"]).is_err());
    // 没有远程时推送返回说明原因的错误。
    assert_eq!(
        git.push(&root, false),
        Err("此仓库未配置远程仓库".to_owned())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 还没有提交过的仓库里只提交一部分；提交被拒绝时暂存区恢复原样；取内容时「没有该文件」
/// 与真正的失败可以区分开。
///
/// 找不到 `git` 时跳过。
#[test]
fn the_exit_code_of_test_decides_the_file_mode() {
    assert_eq!(mode_from_test(Some(0)), Some("100755"));
    assert_eq!(mode_from_test(Some(1)), Some("100644"));
    // 发行版不存在、`wsl.exe` 启动失败、进程被信号终止时查询失败，不按不可执行处理。
    assert_eq!(mode_from_test(Some(2)), None);
    assert_eq!(mode_from_test(Some(-1)), None);
    assert_eq!(mode_from_test(None), None);
}

#[test]
fn risks_only_count_repository_settings_that_run_commands() {
    let out = b"global\0core.pager\nless\0local\0core.fsmonitor\ntrue\0local\0user.name\nt\0\
local\0filter.a.b.smudge\ncat\0worktree\0core.hookspath\n.husky\0local\0diff.x.textconv\nsh\0";
    let risks = parse_risks(out, Path::new("/r"), None, None);
    assert_eq!(
        risks.items,
        [
            "filter.a.b.smudge = cat",
            "core.hookspath = .husky",
            "diff.x.textconv = sh"
        ]
    );
    assert_eq!(risks.filters, ["a.b"]);
    assert!(
        parse_risks(
            b"local\0core.fsmonitor\nfalse\0",
            Path::new("/r"),
            None,
            None
        )
        .is_empty()
    );
    assert_ne!(
        parse_risks(
            b"local\0core.fsmonitor\nsh x\0",
            Path::new("/r"),
            None,
            None
        )
        .fingerprint,
        parse_risks(
            b"local\0core.fsmonitor\nsh y\0",
            Path::new("/r"),
            None,
            None
        )
        .fingerprint,
        "改了命令要重新确认"
    );
}

/// 指纹记在状态文件里：同样的设置和 hooks 在各个版本里要算出同样的值，否则每个信任过的
/// 仓库都要重新确认。
///
/// 期望值是设置和 hooks 按顺序拼在一起（各段后跟 0 字节）的 FNV-1a。
#[cfg(unix)]
#[test]
fn risk_fingerprints_stay_the_same_across_versions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = crate::test_util::scratch("hooks");
    let hook = |name: &str, body: &str, mode: u32| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    hook("pre-commit", "#!/bin/sh\necho hi\n", 0o755);
    hook("post-merge", "x", 0o755);
    hook("pre-push.sample", "y", 0o755);
    hook("notes", "z", 0o644);

    let risks = parse_risks(b"local\0core.fsmonitor\nsh x\0", &dir, Some(&dir), None);
    assert_eq!(
        risks.items,
        [
            "core.fsmonitor = sh x",
            "hooks/post-merge",
            "hooks/pre-commit"
        ]
    );
    assert_eq!(risks.fingerprint, 0x1b4d_c5af_9d50_b560);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：设置 `core.hooksPath` 后仍按 `.git/hooks` 判断，配置目录里的钩子内容变化时不会
/// 要求重新确认。
#[cfg(unix)]
#[test]
fn risks_follow_the_configured_hooks_path() {
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_util::scratch("hooks-path");
    let (custom, default) = (root.join(".husky"), root.join(".git/hooks"));
    let hook = |dir: &Path, body: &str| {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("pre-commit");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    hook(&custom, "a");
    hook(&default, "x");
    let local = b"local\0core.hookspath\n.husky/\0";
    let home = root.join("home");
    let risks = |out: &[u8]| parse_risks(out, &root, Some(&default), Some(&home));

    let before = risks(local);
    assert_eq!(
        before.items,
        ["core.hookspath = .husky/", ".husky/pre-commit"]
    );
    hook(&custom, "b");
    assert_ne!(
        risks(local).fingerprint,
        before.fingerprint,
        "钩子变了要重新确认"
    );
    // `.git/hooks` 不再运行：它变了不影响。
    let after = risks(local).fingerprint;
    hook(&default, "y");
    assert_eq!(risks(local).fingerprint, after);
    // 仓库配置里 `~/` 开头的路径从仓库所在系统的主目录算起。
    hook(&home.join("hooks"), "h");
    let tilde = risks(b"local\0core.hookspath\n~/hooks\0");
    assert_eq!(
        tilde.items,
        ["core.hookspath = ~/hooks", "~/hooks/pre-commit"]
    );
    // 用户自己的配置指向的绝对路径是用户的钩子，不算仓库的风险。
    let global = format!("global\0core.hookspath\n{}\0", custom.display());
    assert!(risks(global.as_bytes()).is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

/// 仓库的配置里有过滤器和 fsmonitor 命令、hooks 目录里有钩子：信任之前读状态、取内容都
/// 不运行它们，提交被拒绝；信任之后照常运行。
///
/// 找不到 `git` 时跳过。
#[test]
fn untrusted_repository_commands_do_not_run() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("trust");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["config", "user.name", "t"]);
    sh(&["config", "user.email", "t@t"]);
    sh(&["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join(".gitattributes"), "*.t filter=x\n").unwrap();
    std::fs::write(root.join("a.t"), "hi\n").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "a"]);
    let marker = root.join("ran");
    let touch = format!("touch '{}'; cat", marker.display());
    sh(&["config", "filter.x.smudge", &touch]);
    sh(&["config", "filter.x.clean", &touch]);
    sh(&[
        "config",
        "core.fsmonitor",
        &format!("touch '{}'; false", marker.display()),
    ]);
    std::fs::write(root.join("a.t"), "changed\n").unwrap();

    let cancel = Cancel::default();
    git.status(&root, &cancel).unwrap();
    assert_eq!(
        git.head_content(&root, &root.join("a.t"), &cancel)
            .unwrap()
            .as_deref(),
        Some(&b"hi\n"[..])
    );
    assert!(!marker.exists(), "不可信的仓库里运行了配置中的命令");
    let risks = git.untrusted(&root).expect("应当不可信");
    assert_eq!(risks.filters, ["x"]);
    let refused = git.commit(&root, "m", &[root.join("a.t")], &[], &[], false);
    assert!(refused.unwrap_err().contains("信任"));
    assert!(git.operate(&root, &["switch", "-c", "b"]).is_err());

    git.trust(&root, risks.fingerprint);
    assert!(git.untrusted(&root).is_none());
    git.head_content(&root, &root.join("a.t"), &cancel).unwrap();
    assert!(marker.exists(), "信任以后照常运行过滤器");

    // 信任以后新增钩子：需要重新确认。
    let hook = root.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git.status(&root, &cancel).unwrap();
    assert!(
        git.untrusted(&root)
            .is_some_and(|r| r.items.contains(&"hooks/pre-commit".to_owned()))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 只读命令不验证签名（`gpg.program`），逐行追溯不使用 textconv：这两项配置都会运行
/// 仓库配置中的命令。
#[test]
fn reading_runs_no_signature_or_textconv_program() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("trust-read");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let sh = |args: &[&str], stdin: &str| {
        use std::io::Write;
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    sh(&["init", "-q", "-b", "main"], "");
    sh(&["config", "user.name", "t"], "");
    sh(&["config", "user.email", "t@t"], "");
    sh(&["config", "commit.gpgsign", "false"], "");
    std::fs::write(root.join(".gitattributes"), "*.t diff=x\n").unwrap();
    std::fs::write(root.join("a.t"), "hi\n").unwrap();
    sh(&["add", "."], "");
    sh(&["commit", "-q", "-m", "a"], "");
    // 带签名头的提交：签名无效，但验证时仍会运行 `gpg.program`。
    let tree = sh(&["rev-parse", "HEAD^{tree}"], "");
    let signed = format!(
        "tree {tree}\nauthor t <t@t> 1700000000 +0000\ncommitter t <t@t> 1700000000 +0000\n\
         gpgsig -----BEGIN PGP SIGNATURE-----\n \n abc\n -----END PGP SIGNATURE-----\n\nsigned\n"
    );
    let oid = sh(&["hash-object", "-t", "commit", "-w", "--stdin"], &signed);
    let marker = root.join("ran");
    // `gpg.program` 不经由 shell 启动，直接执行该脚本；textconv 经由 shell 启动。
    let script = root.join("run.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let script = script.to_str().unwrap();
    sh(&["config", "log.showSignature", "true"], "");
    sh(&["config", "gpg.program", script], "");
    sh(&["config", "diff.x.textconv", script], "");

    let cancel = Cancel::default();
    assert_eq!(git.commit_message(&root, &oid, &cancel).unwrap(), "signed");
    git.blame(&root, &root.join("a.t"), b"changed\nhi\n".to_vec(), &cancel)
        .unwrap();
    assert!(!marker.exists(), "读的命令运行了配置中的命令");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 以 `-` 开头的远程名在推送时会被当作选项（`--receive-pack=…` 会运行命令），因此跳过
/// 该远程。
#[test]
fn push_skips_remotes_that_look_like_options() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("push-remote");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["config", "user.name", "t"]);
    sh(&["config", "user.email", "t@t"]);
    sh(&["config", "commit.gpgsign", "false"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "a"]);
    let marker = root.join("ran");
    let remote = format!("--receive-pack=touch '{}';", marker.display());
    sh(&["config", &format!("remote.{remote}.url"), "."]);

    assert!(git.untrusted(&root).is_none());
    assert!(git.push(&root, false).is_err());
    assert!(!marker.exists(), "远程的名字被当成了选项");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：环境中残留的仓库变量不改变命令作用的仓库。
///
/// `git commit` 启动编辑器时传入指向临时暂存区的 `GIT_INDEX_FILE`，提交结束后该文件即被删除。
/// 从该环境启动的进程如果把这些变量原样传给 `git`，`git` 读到的是空的暂存区：所有已提交的文件
/// 既显示为删除，又显示为未跟踪。
#[test]
fn inherited_repository_variables_do_not_redirect_git() {
    let dir = crate::test_util::scratch("git-inherited-env");
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    vars.extend([
        ("GIT_INDEX_FILE".into(), dir.join(".git/index.lock").into()),
        ("GIT_DIR".into(), dir.join("elsewhere").into()),
        ("GIT_WORK_TREE".into(), dir.join("elsewhere").into()),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), "commit.gpgsign".into()),
        ("GIT_CONFIG_VALUE_0".into(), "false".into()),
    ]);
    let Ok(git) = Git::find(&Shell::with_vars(vars)) else {
        return;
    };
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("README.md"), "# x\n").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "first"]);

    let status = git.status(&dir, &Cancel::default()).unwrap();
    assert_eq!(status.entries, []);
    assert_eq!(status.head, Some(Head::Branch("main".into())));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：`WSLENV` 列出而未设置的变量进入发行版后取空字符串；启动前去掉这些变量，普通查询
/// 仍使用仓库自己的暂存区。
#[test]
fn unset_wsl_variables_do_not_select_an_empty_index() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-wsl-index");
    let cancel = Cancel::default();
    let sh = |args: &[&str]| {
        let mut command = git.command(&dir);
        command.args(args);
        git.run(&mut command, STATUS_TIMEOUT, &cancel).unwrap()
    };
    sh(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.rs"), "one\n").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "first"]);

    // 模拟 `wsl.exe`：启动前去掉未设置的变量，`WSLENV` 中剩下的变量若未设置，在发行版内
    // 取空字符串。
    let mut command = git.command(&dir);
    command.env("WSLENV", crate::wsl::GIT_ENV);
    crate::wsl::drop_unset(&mut command);
    let list = (command.get_envs())
        .find(|(key, _)| *key == "WSLENV")
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    assert!(
        !list.split(':').any(|name| name == "GIT_INDEX_FILE"),
        "{list}"
    );
    for name in list.split(':').filter(|name| !name.is_empty()) {
        let set = command
            .get_envs()
            .any(|(key, value)| key == name && value.is_some());
        if !set {
            command.env(name, "");
        }
    }
    command.env_remove("WSLENV");
    command.args(["status", "--porcelain=v2", "-z"]);
    let stdout = git.run(&mut command, STATUS_TIMEOUT, &cancel).unwrap();
    assert!(parse_status(&stdout, &dir).unwrap().entries.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：交给 `git` 的路径参数以 `/` 分隔。
///
/// Windows 上的路径以 `\` 分隔，发行版内的 `git` 无法匹配子目录中的文件：提交、回滚报告路径
/// 不存在，部分提交后同步暂存区的 `reset` 既不报错也不生效。
#[test]
fn pathspecs_separate_with_forward_slashes() {
    let root = Path::new("/repo");
    let paths = [root.join("src").join("main.rs"), root.join("README.md")];
    let specs: Vec<OsString> = relative_all(root, &paths).collect();
    assert_eq!(specs, [":(literal)src/main.rs", ":(literal)README.md"]);
    assert_eq!(slash_path(root, &paths[0]).unwrap(), "src/main.rs");
}

#[test]
fn staging_records_edits_and_deletions_literally() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-stage");
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    for name in ["app/[slug]/page.tsx", "app/s/page.tsx", "gone.txt"] {
        std::fs::create_dir_all(dir.join(name).parent().unwrap()).unwrap();
        std::fs::write(dir.join(name), "a\n").unwrap();
    }
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "first"]);
    std::fs::write(dir.join("app/[slug]/page.tsx"), "b\n").unwrap();
    std::fs::write(dir.join("app/s/page.tsx"), "b\n").unwrap();
    std::fs::remove_file(dir.join("gone.txt")).unwrap();

    let paths = [
        dir.join("app").join("[slug]").join("page.tsx"),
        dir.join("gone.txt"),
    ];
    git.stage(&dir, &paths).unwrap();
    let status = git.status(&dir, &Cancel::default()).unwrap();
    let staged = |name: &str| {
        let entry = status.entries.iter().find(|e| e.path == dir.join(name));
        entry.and_then(|e| e.staged)
    };
    assert_eq!(staged("app/[slug]/page.tsx"), Some(Change::Modified));
    assert_eq!(staged("gone.txt"), Some(Change::Deleted));
    // `[slug]` 不作为通配符：按通配符规则会匹配的 `app/s/page.tsx` 不加入暂存区。
    assert_eq!(staged("app/s/page.tsx"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：非 UTF-8 的文件名经有损转换后交给 `git`，会指向另一个（通常不存在的）文件。
#[cfg(unix)]
#[test]
fn paths_given_to_git_keep_their_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let root = Path::new("/repo");
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    let path = root.join("dir").join(name);
    assert_eq!(
        slash_path(root, &path).unwrap().into_vec(),
        b"dir/caf\xe9.txt"
    );
    let arg = joined_arg(&[OsStr::new("HEAD:"), &slash_path(root, &path).unwrap()]);
    assert_eq!(arg.into_vec(), b"HEAD:dir/caf\xe9.txt");
    assert!(slash_path(root, Path::new("/elsewhere/a")).is_err());
}

#[test]
fn undoing_a_commit_keeps_its_changes_staged() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-undo-commit");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let sh = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(&root).args(args).output();
        String::from_utf8(out.unwrap().stdout).unwrap()
    };
    sh(&["init", "-q", "-b", "main"]);
    let a = root.join("a.rs");
    std::fs::write(&a, "one\n").unwrap();
    let only = std::slice::from_ref(&a);
    git.commit(&root, "first", only, only, &[], false).unwrap();
    let first = git.last_commit(&root).unwrap();
    std::fs::write(&a, "two\n").unwrap();
    git.commit(&root, "second\n\n正文", only, &[], &[], false)
        .unwrap();
    let second = git.last_commit(&root).unwrap();
    assert_eq!(second.subject, "second");
    assert_eq!(second.message, "second\n\n正文");
    assert_eq!(second.local.len(), "2026-10-26 14:32".len());
    assert!(second.time > 0 && second.oid != first.oid);

    // 分支已经不指向该提交：不撤销。
    assert!(git.undo_commit(&root, &first.oid).is_err());
    assert_eq!(git.last_commit(&root).unwrap().oid, second.oid);

    // 退回上一个提交，改动留在暂存区，工作区的内容不变。
    git.undo_commit(&root, &second.oid).unwrap();
    assert_eq!(git.last_commit(&root).unwrap().oid, first.oid);
    assert_eq!(sh(&["diff", "--cached", "--name-only"]), "a.rs\n");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "two\n");

    // 第一个提交：分支回到还没有提交的状态，文件仍在暂存区。
    git.undo_commit(&root, &first.oid).unwrap();
    assert_eq!(git.last_commit(&root), None);
    assert_eq!(sh(&["diff", "--cached", "--name-only"]), "a.rs\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recent_branches_keep_the_latest_switch_of_each_branch() {
    let output = "checkout: moving from main to feature/login\n\
                  checkout: moving from dev to main\n\
                  checkout: moving from main to dev\n\
                  checkout: moving from dev to feature/login\n";
    assert_eq!(
        parse_recent_branches(output),
        ["feature/login", "main", "dev"]
    );
}

#[test]
fn scanning_finds_nested_repositories_within_the_depth() {
    let dir = crate::test_util::scratch("git-scan");
    let mark = |path: &str| std::fs::create_dir_all(dir.join(path).join(".git")).unwrap();
    mark("a");
    mark("group/b");
    mark("group/deep/c");
    // 找到的仓库不再往里找；依赖目录、隐藏目录都跳过。
    mark("a/inner");
    mark("node_modules/pkg");
    mark(".cache/d");

    let names = |depth| -> Vec<PathBuf> {
        scan_repositories(&dir, depth, &Cancel::default())
            .into_iter()
            .map(|path| path.strip_prefix(&dir).unwrap().to_path_buf())
            .collect()
    };
    assert_eq!(names(1), [PathBuf::from("a")]);
    assert_eq!(names(2), [PathBuf::from("a"), PathBuf::from("group/b")]);
    assert_eq!(
        names(3),
        [
            PathBuf::from("a"),
            PathBuf::from("group/b"),
            PathBuf::from("group/deep/c"),
        ]
    );

    let cancelled = Cancel::default();
    cancelled.cancel();
    assert!(scan_repositories(&dir, 3, &cancelled).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 启动要求 HTTP 基本认证的本地服务：总是返回 401，并把收到的 `Authorization` 写入通道。
#[cfg(unix)]
fn auth_server() -> (u16, std::sync::mpsc::Receiver<String>) {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let auth = request.lines().find(|l| l.starts_with("Authorization:"));
            if let Some(auth) = auth {
                let _ = sender.send(auth.to_owned());
            }
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"t\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (port, received)
}

/// `operate` 的命令参数：禁用凭据助手和代理，测试不读写本机钥匙串。
#[cfg(unix)]
const ISOLATED_REMOTE: [&str; 4] = ["-c", "credential.helper=", "-c", "http.proxy="];

#[cfg(unix)]
#[test]
fn operations_ask_through_the_askpass_program_and_keep_their_output() {
    use std::os::unix::fs::PermissionsExt;
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("askpass");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let asked = root.join("asked");
    let program = root.join("askpass.sh");
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\necho \"$WINC_GIT_ASKPASS|$1\" >> '{}'\n\
             case \"$1\" in Username*) echo me;; *) echo s3cret;; esac\n",
            asked.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let (port, received) = auth_server();
    let url = format!("http://127.0.0.1:{port}/repo.git");
    let mut args = ISOLATED_REMOTE.to_vec();
    args.extend(["ls-remote", url.as_str()]);

    // 没有 askpass 程序时直接失败，不向服务发送凭据。
    assert!(git.operate(&root, &args).is_err());
    assert!(received.try_recv().is_err());
    assert!(!asked.exists());

    let git = git.with_askpass(Some(Askpass {
        program,
        endpoint: "the-endpoint".into(),
    }));
    assert!(git.operate(&root, &args).is_err());
    let auth = received
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    // `me:s3cret` 的 Base64。
    assert!(auth.contains("Basic bWU6czNjcmV0"), "{auth}");
    let asked = std::fs::read_to_string(asked).unwrap();
    assert!(asked.starts_with("the-endpoint|Username for"), "{asked}");
    assert!(asked.contains("the-endpoint|Password for"), "{asked}");

    let output = output_text();
    assert!(output.contains(&format!(
        "$ git -c credential.helper= -c http.proxy= ls-remote {url}"
    )));
    let status = winc_i18n::t!("git-output-exit-code", code = "128");
    assert!(output.contains(&status), "{output}");
    assert!(!output.contains("s3cret"), "{output}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn publishing_prefers_configured_targets_then_origin() {
    let names = ["github-desktop-alice", "github-desktop-bob", "origin"].map(String::from);
    let remotes: Vec<&String> = names.iter().collect();
    // 没有配置推送目标：推到 `origin`，即使它按名字排在后面。
    assert_eq!(publish_remote(&remotes, &[]).unwrap(), "origin");
    // 配置的推送目标优先；配置了已不存在的远程时跳过它。
    let preferred = ["gone".to_owned(), "github-desktop-bob".to_owned()];
    assert_eq!(
        publish_remote(&remotes, &preferred).unwrap(),
        "github-desktop-bob"
    );
    // 没有 `origin` 时取第一个；没有远程时为 `None`。
    assert_eq!(
        publish_remote(&remotes[..2], &[]).unwrap(),
        "github-desktop-alice"
    );
    assert_eq!(publish_remote(&[], &[]), None);
}

/// 审阅拉取请求时添加的远程按名字排在 `origin` 之前；发布新分支时仍然推到 `origin`。
#[test]
fn publishing_a_branch_pushes_to_origin_before_other_remotes() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("publish-remote");
    let root = winc_config::files::canonicalize(&dir).unwrap().join("work");
    let sh = |cwd: &Path, args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    let base = root.parent().unwrap();
    for bare in ["fork.git", "origin.git"] {
        std::fs::create_dir_all(base.join(bare)).unwrap();
        sh(&base.join(bare), &["init", "-q", "--bare"]);
    }
    std::fs::create_dir_all(&root).unwrap();
    sh(&root, &["init", "-q", "-b", "main"]);
    sh(&root, &["config", "user.name", "t"]);
    sh(&root, &["config", "user.email", "t@t"]);
    sh(&root, &["config", "commit.gpgsign", "false"]);
    sh(&root, &["commit", "-q", "--allow-empty", "-m", "a"]);
    let fork = base.join("fork.git");
    let origin = base.join("origin.git");
    sh(
        &root,
        &[
            "remote",
            "add",
            "github-desktop-alice",
            fork.to_str().unwrap(),
        ],
    );
    sh(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    sh(&root, &["switch", "-q", "-c", "feature"]);
    assert!(git.untrusted(&root).is_none());

    git.push(&root, false).unwrap();
    let upstream = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["config", "branch.feature.remote"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&upstream.stdout).trim(), "origin");
    let pushed = |bare: &Path| {
        Command::new("git")
            .arg("-C")
            .arg(bare)
            .args(["rev-parse", "--verify", "--quiet", "refs/heads/feature"])
            .status()
            .unwrap()
            .success()
    };
    assert!(pushed(&origin));
    assert!(!pushed(&fork), "推到了按名字排在前面的远程");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 远端主机上的仓库：`git` 在远端运行，输出中的路径换回该连接上的虚拟路径。
#[cfg(unix)]
#[test]
fn a_repository_on_a_remote_host_is_read_through_its_connection() {
    use std::os::unix::ffi::OsStrExt;

    let Ok(git) = Git::find(&Shell::with_vars(std::env::vars_os().collect())) else {
        return;
    };
    let root = crate::test_util::scratch("git-remote");
    let sh = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), "one\n").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "first"]);
    std::fs::write(root.join("src/a.txt"), "two\n").unwrap();
    std::fs::write(root.join("new file.txt"), "x").unwrap();

    let target = crate::test_util::loopback("git-remote");
    let far = winc_backend::virtual_path(&target, root.as_os_str().as_bytes());
    let cancel = Cancel::default();
    assert_eq!(find_root(&far.join("src")), Some(far.clone()));

    // 状态中的路径是该连接上的虚拟路径，可以直接用于打开文件。
    let status = git.status(&far, &cancel).unwrap();
    let paths: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(paths, [far.join("src/a.txt"), far.join("new file.txt")]);
    assert!(status.oid.is_some());
    assert_eq!(
        winc_backend::fs::read(&paths[0]).unwrap(),
        b"two\n",
        "工作区的内容经文件操作读取"
    );
    // `HEAD` 中的内容：路径换成相对于仓库的写法传给远端的 `git`。
    let committed = git.head_content(&far, &far.join("src/a.txt"), &cancel);
    assert_eq!(committed.unwrap().as_deref(), Some(b"one\n".as_slice()));
    assert_eq!(
        git.head_content(&far, &far.join("new file.txt"), &cancel),
        Ok(None)
    );
    let branches = git.branches(&far, &cancel).unwrap();
    assert!(branches.iter().any(|b| b.name == "main"), "{branches:?}");
    // 提交列表经连接随到随读；仓库根的向上查找不越过远端的根目录。
    let mut log = git.log_stream(&far, &LogQuery::default()).unwrap();
    let commits = log.next(10).unwrap();
    assert_eq!(commits.len(), 1, "{commits:?}");
    let outside = winc_backend::virtual_path(&target, b"/");
    assert_eq!(find_root(&outside), None);

    // 已取消的操作不发往远端；连接断开后报错，不会转而在本机运行。
    let stopped = Cancel::default();
    stopped.cancel();
    assert!(git.status(&far, &stopped).is_err());
    winc_backend::remote::disconnect(&target);
    assert!(git.status(&far, &cancel).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}
