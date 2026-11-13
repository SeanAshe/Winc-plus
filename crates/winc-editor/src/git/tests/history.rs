//! 读取历史的测试：日志、提交、追溯、贮藏和版本的解析。

use super::*;

#[test]
fn log_records_split_into_commits_with_their_refs() {
    let (a, b, p1, p2) = (
        "a".repeat(40),
        "b".repeat(64),
        "1".repeat(40),
        "2".repeat(40),
    );
    let text = format!(
        "{a}\x1f{p1} {p2}\x1fAnn\x1fann@x\x1f1700000000\x1f2023-11-15 06:13\x1fHEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD, tag: refs/tags/v1\x1fMerge it\x1e\n\
         {b}\x1f\x1fBob\x1fbob@x\x1f1600000000\x1f2020-09-13 20:26\x1f\x1fRoot: a \u{1f}b\x1e\n"
    );
    let commits = parse_log(&text);
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].oid, a);
    assert_eq!(commits[0].parents, [p1, p2]);
    assert_eq!(
        commits[0].refs,
        [
            Ref::Head("main".into()),
            Ref::Remote("origin/main".into()),
            Ref::Tag("v1".into()),
        ]
    );
    assert_eq!(commits[0].subject, "Merge it");
    assert!(commits[1].parents.is_empty() && commits[1].refs.is_empty());
    assert_eq!(commits[1].time, 1_600_000_000);
    assert_eq!(commits[1].local, "2020-09-13 20:26");
    assert_eq!(
        parse_refs("HEAD, refs/heads/dev"),
        [Ref::Detached, Ref::Branch("dev".into())]
    );
}

#[test]
fn log_subject_cannot_forge_a_record() {
    let (a, p) = ("a".repeat(40), "1".repeat(40));
    // 提交说明中含有 `0x1e`，之后是一条提交号为选项的伪造记录。
    let text = format!(
        "{a}\x1f{p}\x1fAnn\x1fann@x\x1f1700000000\x1f2023-11-15 06:13\x1f\x1fx\x1e--output=/tmp/x\x1f{p}\x1fa\x1fe\x1f0\x1fl\x1f\x1fs\x1e\n\
         {p}\x1f--all\x1fBob\x1fbob@x\x1f1600000000\x1f2020-09-13 20:26\x1f\x1fbad parent\x1e\n"
    );
    let commits = parse_log(&text);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].oid, a);
}

#[test]
fn blame_maps_every_line_to_its_commit() {
    let a = "a".repeat(40);
    let zero = "0".repeat(40);
    let text = format!(
        "{a} 1 1 2\nauthor Ann\nauthor-mail <ann@x>\nauthor-time 1700000000\nauthor-tz +0800\nsummary First\nfilename f\n\tline one\n\
             {a} 2 2\n\tline two\n\
             {zero} 3 3 1\nauthor Not Committed Yet\nauthor-time 1800000000\nauthor-tz -0530\nsummary Version of f from -\nfilename f\n\tnew line\n"
    );
    let blame = parse_blame(&text);
    assert_eq!(blame.lines, [0, 0, 1]);
    assert_eq!(blame.commits[0].author, "Ann");
    assert_eq!(blame.commits[0].summary, "First");
    assert_eq!(blame.commits[0].offset, 8 * 3600);
    assert!(!blame.commits[0].uncommitted);
    assert!(blame.commits[1].uncommitted);
    assert_eq!(blame.commits[1].offset, -(5 * 3600 + 30 * 60));
}

/// 合并冲突的处理：状态里识别出「合并中」和冲突的文件，文件里识别出冲突标记；在文件里
/// 解决并提交之后合并完成。
///
/// 找不到 `git` 时跳过。
#[test]
fn merge_conflicts_resolve_and_commit() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("merge");
    let git_in = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "merge.conflictStyle=merge",
            ])
            .args(args)
            .output()
            .unwrap()
            .status
            .success()
    };
    let sh = |args: &[&str]| assert!(git_in(args), "git {args:?}");
    sh(&["init", "-q", "-b", "main"]);
    sh(&["config", "user.name", "t"]);
    sh(&["config", "user.email", "t@t"]);
    sh(&["config", "commit.gpgsign", "false"]);
    let m = dir.join("m.rs");
    std::fs::write(&m, "top\nbase\nbottom\n").unwrap();
    sh(&["add", "."]);
    sh(&["commit", "-q", "-m", "base"]);
    sh(&["switch", "-q", "-c", "other"]);
    std::fs::write(&m, "top\ntheirs\nbottom\n").unwrap();
    sh(&["commit", "-q", "-am", "theirs"]);
    sh(&["switch", "-q", "main"]);
    std::fs::write(&m, "top\nours\nbottom\n").unwrap();
    sh(&["commit", "-q", "-am", "ours"]);
    assert!(!git_in(&["merge", "other"]), "应当出冲突");

    let root = winc_config::files::canonicalize(&dir).unwrap();
    let m = root.join("m.rs");
    let cancel = Cancel::default();
    let status = git.status(&root, &cancel).unwrap();
    assert_eq!(status.operation, Some(Operation::Merge));
    let entry = status.entries.iter().find(|e| e.path == m).unwrap();
    assert_eq!(entry.shown(), Change::Conflicted);
    assert!(
        git.merge_message(&root)
            .is_some_and(|m| m.contains("other"))
    );

    let rope = ropey::Rope::from_str(&std::fs::read_to_string(&m).unwrap());
    let conflicts = merge::find_conflicts(&rope);
    assert_eq!(conflicts.len(), 1);
    let both = merge::resolution(&rope, &conflicts[0], merge::Pick::Both);
    assert_eq!(both, "ours\ntheirs\n");
    std::fs::write(&m, format!("top\n{both}bottom\n")).unwrap();

    // 合并中不能只提交一部分；整个提交以后合并完成。
    let part = Partial {
        path: m.clone(),
        content: b"x\n".to_vec(),
    };
    assert!(
        git.commit(&root, "m", std::slice::from_ref(&m), &[], &[part], false)
            .is_err()
    );
    git.commit(
        &root,
        "merge other",
        std::slice::from_ref(&m),
        &[],
        &[],
        false,
    )
    .unwrap();
    let status = git.status(&root, &cancel).unwrap();
    assert_eq!(status.operation, None);
    assert!(status.entries.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn partial_commits_keep_other_staged_files_and_the_execute_bit() {
    use std::os::unix::fs::PermissionsExt;

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
    let shell = Shell::with_vars(vars);
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("git-partial-index");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let sh = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["config", "user.name", "t"]);
    sh(&["config", "user.email", "t@t"]);
    sh(&["config", "commit.gpgsign", "false"]);
    let script = root.join("run.sh");
    std::fs::write(&script, "1\n2\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    sh(&["add", "run.sh"]);
    sh(&["commit", "-q", "-m", "first"]);

    // `staged.txt` 已暂存；只提交 `run.sh` 的一部分。
    std::fs::write(root.join("staged.txt"), "s\n").unwrap();
    sh(&["add", "staged.txt"]);
    std::fs::write(&script, "1\n2\n3\n4\n").unwrap();
    let part = Partial {
        path: script.clone(),
        content: b"1\n2\n3\n".to_vec(),
    };
    git.commit(
        &root,
        "part",
        std::slice::from_ref(&script),
        &[],
        std::slice::from_ref(&part),
        false,
    )
    .unwrap();

    assert_eq!(sh(&["diff", "--cached", "--name-only"]), "staged.txt");
    assert!(sh(&["ls-tree", "HEAD", "run.sh"]).starts_with("100755 "));
    assert_eq!(sh(&["show", "HEAD:run.sh"]), "1\n2\n3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn partial_commits_failed_commits_and_missing_contents() {
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
    let shell = Shell::with_vars(vars);
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("git-unborn");
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
    let cancel = Cancel::default();

    // 还没有提交过：文件不在 `HEAD` 里，不算失败；只提交一部分时同样可以提交。
    let a = root.join("a.txt");
    std::fs::write(&a, "1\n2\n").unwrap();
    assert_eq!(git.head_content(&root, &a, &cancel), Ok(None));
    let part = Partial {
        path: a.clone(),
        content: b"1\n".to_vec(),
    };
    git.commit(
        &root,
        "first",
        std::slice::from_ref(&a),
        &[],
        std::slice::from_ref(&part),
        false,
    )
    .unwrap();
    let head = git.head_content(&root, &a, &cancel).unwrap();
    assert_eq!(head.as_deref(), Some(&b"1\n"[..]));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "1\n2\n");

    // 版本不存在不是「没有该文件」。
    assert!(git.content_at(&root, "no-such-rev", &a, &cancel).is_err());
    assert_eq!(
        git.content_at(&root, "HEAD", &root.join("none.txt"), &cancel),
        Ok(None)
    );

    // 提交被 hooks 拒绝：未跟踪的文件不留在暂存区。
    let hook = root.join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho 不许提交 >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let risks = git.inspect(&root).unwrap();
    git.trust(&root, risks.fingerprint);
    let b = root.join("b.txt");
    std::fs::write(&b, "new\n").unwrap();
    let refused = git.commit(
        &root,
        "second",
        std::slice::from_ref(&b),
        std::slice::from_ref(&b),
        &[],
        false,
    );
    assert!(refused.unwrap_err().contains("不许提交"));
    let status = git.status(&root, &cancel).unwrap();
    let entry = status.entries.iter().find(|e| e.path == b).unwrap();
    assert!(entry.untracked(), "{entry:?}");

    // 必需的过滤器失败：返回错误，不能当成文件不在 `HEAD` 里。
    std::fs::remove_file(&hook).unwrap();
    std::fs::write(root.join(".gitattributes"), "*.t filter=x\n").unwrap();
    let t = root.join("a.t");
    std::fs::write(&t, "hi\n").unwrap();
    sh(&["add", "a.t", ".gitattributes"]);
    sh(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-q",
        "-m",
        "t",
    ]);
    sh(&["config", "filter.x.smudge", "false"]);
    sh(&["config", "filter.x.required", "true"]);
    let risks = git.inspect(&root).unwrap();
    git.trust(&root, risks.fingerprint);
    assert!(git.head_content(&root, &t, &cancel).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 转到：提交号（可以缩写）、分支名和标签名都解析成完整的提交号；修改上次提交之后，原来的
/// 提交仍能解析，但不在日志中，固定后排在最前面。
#[test]
fn revisions_resolve_to_commits_and_amended_ones_are_outside_the_log() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-resolve-revision");
    let sh = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "first"]);
    sh(&["tag", "-a", "v1", "-m", "v1"]);
    let first = sh(&["rev-parse", "HEAD"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "second"]);
    let old = sh(&["rev-parse", "HEAD"]);
    sh(&[
        "commit",
        "-q",
        "--amend",
        "--allow-empty",
        "-m",
        "second, amended",
    ]);
    let new = sh(&["rev-parse", "HEAD"]);
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let cancel = Cancel::default();
    let resolve = |text: &str| git.resolve_revision(&root, text, &cancel);

    assert_eq!(resolve("main").unwrap(), new);
    // 附注标签解析到它指向的提交，不是标签对象本身。
    assert_eq!(resolve("v1").unwrap(), first);
    assert_eq!(resolve(&old[..8].to_ascii_uppercase()).unwrap(), old);
    for bad in ["", "-h", "main v1", "no-such-branch", &"0".repeat(12)] {
        assert!(resolve(bad).is_err(), "{bad:?}");
    }

    assert!(git.in_log(&root, &new, &cancel).unwrap());
    assert!(git.in_log(&root, &first, &cancel).unwrap());
    assert!(!git.in_log(&root, &old, &cancel).unwrap());
    let query = LogQuery {
        pin: Some(old.clone()),
        ..LogQuery::default()
    };
    let commits = git.log_stream(&root, &query).unwrap().next(10).unwrap();
    let oids: Vec<_> = commits.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(oids, [old.as_str(), new.as_str(), first.as_str()]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 筛选文字形如提交号时同时按提交号查找：解析出的提交排在最前面，说明同样匹配时不重复。
#[test]
fn log_filter_also_finds_a_commit_by_its_hash() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-log-hash");
    let sh = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    sh(&["init", "-q", "-b", "main"]);
    // 提交说明使用十六进制数以外的字母，查找文字不会碰巧出现在别的提交的提交号里。
    for i in 0..4 {
        sh(&["commit", "-q", "--allow-empty", "-m", &format!("x{i}")]);
    }
    let first = sh(&["rev-parse", "HEAD~3"]);
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let subjects = |text: &str, batch: usize| {
        let query = LogQuery {
            text: text.to_owned(),
            ..LogQuery::default()
        };
        let mut stream = git.log_stream(&root, &query).unwrap();
        let mut subjects = Vec::new();
        loop {
            let commits = stream.next(batch).unwrap();
            let done = commits.len() < batch;
            subjects.extend(commits.into_iter().map(|c| c.subject));
            if done {
                return subjects;
            }
        }
    };
    assert_eq!(subjects(&first[..7], 10), ["x0"]);
    assert_eq!(subjects(&first[..7].to_uppercase(), 10), ["x0"]);
    // 提交说明里包含该提交号时，该提交只出现一次并排在最前面；分批读取时结果相同。
    sh(&[
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        &format!("revert {}", &first[..7]),
    ]);
    sh(&["commit", "-q", "--allow-empty", "-m", "x5"]);
    assert_eq!(
        subjects(&first[..7], 10),
        ["x0", &format!("revert {}", &first[..7])]
    );
    assert_eq!(
        subjects(&first[..7], 1),
        ["x0", &format!("revert {}", &first[..7])]
    );
    // 不形如提交号、或无法解析的文字仍按提交说明查找。
    // 无法解析的提交号取 40 位：没有提交以它开头，7 位提交号的提交说明里也不含这 40 位。
    assert_eq!(subjects("x3", 10), ["x3"]);
    assert_eq!(subjects(&"f".repeat(40), 10), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 日志从同一个运行中的 `git log` 分批读取，顺序与一次读取全部时相同；
/// 中途丢弃日志时 `git log` 进程随之结束。
#[test]
fn log_streams_read_batches_from_one_git() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-log-stream");
    let sh = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
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
    for i in 0..5 {
        sh(&["commit", "-q", "--allow-empty", "-m", &format!("c{i}")]);
    }
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let query = LogQuery::default();

    let mut stream = git.log_stream(&root, &query).unwrap();
    let mut subjects = Vec::new();
    for expected in [2, 2, 1, 0] {
        let batch = stream.next(2).unwrap();
        assert_eq!(batch.len(), expected);
        subjects.extend(batch.into_iter().map(|c| c.subject));
    }
    assert_eq!(subjects, ["c4", "c3", "c2", "c1", "c0"]);

    // 读了一半就丢弃：结束 `git` 进程时不等它写完输出。
    let mut half = git.log_stream(&root, &query).unwrap();
    assert_eq!(half.next(1).unwrap().len(), 1);
    drop(half);

    // `git` 自己报的错（分支不存在）在读第一批时返回。
    let missing = LogQuery {
        branch: Some("no-such-branch".into()),
        ..LogQuery::default()
    };
    let mut stream = git.log_stream(&root, &missing).unwrap();
    assert!(stream.next(10).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 储藏的说明开头写着储藏时的分支：「On 分支:」是带说明的，「WIP on 分支:」是没写说明的。
#[test]
fn a_stash_knows_the_branch_it_was_made_on() {
    let stash = |message: &str| Stash {
        name: "stash@{0}".into(),
        oid: String::new(),
        parents: Vec::new(),
        time: 0,
        local: String::new(),
        message: message.into(),
    };
    assert_eq!(stash("On main: 试一下").branch(), Some("main"));
    assert_eq!(
        stash("WIP on feature/a: 1a2b3c4 改登录").branch(),
        Some("feature/a")
    );
    assert_eq!(
        stash("WIP on (no branch): 1a2b3c4 x").branch(),
        Some("(no branch)")
    );
    assert_eq!(stash("autostash").branch(), None);
}
