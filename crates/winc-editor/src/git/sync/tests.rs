//! `sync` 的测试：本地改写已推送的提交后以本地版本推送；上游有其他人推送的提交时先拉取再推送；
//! 拉取、变基拉取、同步时未提交的更改的去向。
//!
//! 远程是同一台机器上的裸仓库；依赖本机 `git`，找不到时跳过。

use super::super::tests::real_git;
use super::*;

/// 在 `dir` 中运行 `git args`，失败时 panic。
fn sh(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 建一个裸仓库作为远程，以及推送过一个提交、上游为 `origin/main` 的工作仓库。
///
/// 返回的工作仓库路径已规范化，与 `git` 输出的路径形式一致。
fn cloned(name: &str) -> (PathBuf, PathBuf) {
    let remote = crate::test_util::scratch(&format!("{name}-remote"));
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let work = crate::test_util::scratch(name);
    sh(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a.txt"), "one\n").unwrap();
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "one"]);
    sh(
        &work,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    sh(&work, &["push", "-q", "-u", "origin", "main"]);
    let work = winc_config::files::canonicalize(&work).unwrap();
    (work, remote)
}

#[test]
fn an_amended_pushed_commit_is_pushed_over_its_old_version() {
    let Some(git) = real_git() else {
        return;
    };
    let (work, remote) = cloned("sync-amend");
    sh(&work, &["commit", "-q", "--amend", "-m", "one, reworded"]);
    // 领先、落后各 1 个提交：上游多出的是修改前的版本。
    assert_eq!(
        sh(&work, &["rev-list", "--count", "HEAD..@{upstream}"]),
        "1"
    );

    // 未确认时不推送：上游保持原样，并返回将被替换的提交数。
    let before = sh(&remote, &["rev-parse", "main"]);
    assert_eq!(git.sync(&work, false, None).unwrap(), Synced::NeedsForce(1));
    assert_eq!(sh(&remote, &["rev-parse", "main"]), before);

    git.sync(&work, true, None).unwrap();
    let head = sh(&work, &["rev-parse", "HEAD"]);
    assert_eq!(sh(&remote, &["rev-parse", "main"]), head);
    assert_eq!(sh(&work, &["rev-list", "--count", "HEAD"]), "1");
}

#[test]
fn commits_pushed_by_others_are_pulled_before_pushing() {
    let Some(git) = real_git() else {
        return;
    };
    let (work, remote) = cloned("sync-others");
    // 另一个工作仓库推送了本地从未有过的提交，本地同时有自己的新提交。
    let other = crate::test_util::scratch("sync-others-peer");
    sh(&other, &["clone", "-q", remote.to_str().unwrap(), "."]);
    std::fs::write(other.join("b.txt"), "theirs\n").unwrap();
    sh(&other, &["add", "."]);
    sh(&other, &["commit", "-q", "-m", "theirs"]);
    sh(&other, &["push", "-q"]);
    std::fs::write(work.join("c.txt"), "ours\n").unwrap();
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "ours"]);

    git.sync(&work, false, None).unwrap();
    // 两边的提交都在远程上，没有提交被覆盖。
    let subjects = sh(&remote, &["log", "--format=%s", "main"]);
    assert!(
        subjects.contains("theirs") && subjects.contains("ours"),
        "{subjects}"
    );
    assert_eq!(
        sh(&work, &["rev-parse", "HEAD"]),
        sh(&remote, &["rev-parse", "main"])
    );
}

/// 从 `remote` 克隆一份，把 `file` 写成 `text` 并推送：模拟别人推送了本地还没有的提交。
fn push_from_peer(name: &str, remote: &Path, file: &str, text: &str) {
    let peer = crate::test_util::scratch(name);
    sh(&peer, &["clone", "-q", remote.to_str().unwrap(), "."]);
    std::fs::write(peer.join(file), text).unwrap();
    sh(&peer, &["add", "."]);
    sh(&peer, &["commit", "-q", "-m", "theirs"]);
    sh(&peer, &["push", "-q"]);
}

/// [`cloned`] 之后把 `a.txt` 改成三行并推送，各测试在不同的行上改动。
fn three_lines(name: &str) -> (PathBuf, PathBuf) {
    let (work, remote) = cloned(name);
    std::fs::write(work.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    sh(&work, &["commit", "-q", "-am", "three lines"]);
    sh(&work, &["push", "-q"]);
    (work, remote)
}

#[test]
fn uncommitted_changes_are_carried_through_a_pull() {
    let Some(git) = real_git() else {
        return;
    };
    // 没有未提交的更改：拉取后无需恢复，也不留下储藏。
    let (work, remote) = three_lines("pull-carry-clean");
    push_from_peer(
        "pull-carry-clean-peer",
        &remote,
        "a.txt",
        "ONE\ntwo\nthree\n",
    );
    let before = git.stash_tip(&work);
    git.pull(&work, None).unwrap();
    assert_eq!(git.carried(&work, before.as_deref()), Carried::Restored);
    assert_eq!(git.stash_tip(&work), None);

    // 改动的文件也被上游改了（不加 `--autostash` 时 git 拒绝拉取）：拉取后两边的修改都在。
    let (work, remote) = three_lines("pull-carry-dirty");
    push_from_peer(
        "pull-carry-dirty-peer",
        &remote,
        "a.txt",
        "ONE\ntwo\nthree\n",
    );
    std::fs::write(work.join("a.txt"), "one\ntwo\nTHREE\n").unwrap();
    let before = git.stash_tip(&work);
    git.pull(&work, None).unwrap();
    assert_eq!(git.carried(&work, before.as_deref()), Carried::Restored);
    let text = std::fs::read_to_string(work.join("a.txt")).unwrap();
    assert_eq!(text, "ONE\ntwo\nTHREE\n");
    assert_eq!(git.stash_tip(&work), None);
    // 更改仍是未提交的，没有被提交进合并。
    assert_eq!(sh(&work, &["status", "--porcelain"]), "M a.txt");
}

#[test]
fn a_conflicting_restore_is_marked_and_keeps_the_stash() {
    let Some(git) = real_git() else {
        return;
    };
    let (work, remote) = three_lines("pull-carry-conflict");
    push_from_peer(
        "pull-carry-conflict-peer",
        &remote,
        "a.txt",
        "theirs\ntwo\nthree\n",
    );
    std::fs::write(work.join("a.txt"), "mine\ntwo\nthree\n").unwrap();
    let before = git.stash_tip(&work);
    git.pull(&work, None).unwrap();
    // 拉取完成，冲突标记写在工作区里，原更改仍在储藏中。
    assert_eq!(git.carried(&work, before.as_deref()), Carried::Conflicted);
    let text = std::fs::read_to_string(work.join("a.txt")).unwrap();
    assert!(text.contains("<<<<<<<") && text.contains("mine"), "{text}");
    assert!(git.stash_tip(&work).is_some());
    assert_eq!(
        sh(&work, &["rev-parse", "HEAD"]),
        sh(&remote, &["rev-parse", "main"])
    );
}

#[test]
fn rebase_pulls_and_syncs_carry_uncommitted_changes() {
    let Some(git) = real_git() else {
        return;
    };
    // 拉取并变基：本地与上游各有新提交，工作区还有改动（不加 `--autostash` 时 `git` 拒绝变基）。
    let (work, remote) = three_lines("pull-carry-rebase");
    push_from_peer("pull-carry-rebase-peer", &remote, "b.txt", "theirs\n");
    std::fs::write(work.join("c.txt"), "ours\n").unwrap();
    sh(&work, &["add", "c.txt"]);
    sh(&work, &["commit", "-q", "-m", "ours"]);
    std::fs::write(work.join("a.txt"), "one\ntwo\nTHREE\n").unwrap();
    let before = git.stash_tip(&work);
    git.pull(&work, Some(true)).unwrap();
    assert_eq!(git.carried(&work, before.as_deref()), Carried::Restored);
    // 变基而不是合并：历史是一条线，本地提交在上游之上。
    assert_eq!(sh(&work, &["rev-list", "--merges", "--count", "HEAD"]), "0");
    assert_eq!(sh(&work, &["log", "-1", "--format=%s"]), "ours");
    assert_eq!(sh(&work, &["status", "--porcelain"]), "M a.txt");

    // 同步：落后时先拉取，工作区的改动同样保留。
    let (work, remote) = three_lines("sync-carry");
    push_from_peer("sync-carry-peer", &remote, "a.txt", "ONE\ntwo\nthree\n");
    std::fs::write(work.join("a.txt"), "one\ntwo\nTHREE\n").unwrap();
    let before = git.stash_tip(&work);
    assert!(matches!(
        git.sync(&work, false, None).unwrap(),
        Synced::Done(_)
    ));
    assert_eq!(git.carried(&work, before.as_deref()), Carried::Restored);
    let text = std::fs::read_to_string(work.join("a.txt")).unwrap();
    assert_eq!(text, "ONE\ntwo\nTHREE\n");
}

#[test]
fn an_untracked_file_in_the_way_stops_the_pull_without_losing_changes() {
    let Some(git) = real_git() else {
        return;
    };
    // 上游新增的文件在本地是未跟踪的文件：拉取失败，原因说明了是哪个文件；工作区原样不动，
    // 也不留下储藏。
    let (work, remote) = three_lines("pull-carry-untracked");
    push_from_peer("pull-carry-untracked-peer", &remote, "new.txt", "theirs\n");
    std::fs::write(work.join("new.txt"), "mine\n").unwrap();
    std::fs::write(work.join("a.txt"), "one\ntwo\nTHREE\n").unwrap();
    let error = git.pull(&work, None).unwrap_err();
    assert!(error.contains("new.txt"), "{error}");
    assert_eq!(
        std::fs::read_to_string(work.join("new.txt")).unwrap(),
        "mine\n"
    );
    let text = std::fs::read_to_string(work.join("a.txt")).unwrap();
    assert_eq!(text, "one\ntwo\nTHREE\n");
    assert_eq!(git.stash_tip(&work), None);
}

/// 拉取方式：指明合并或变基时不看 Git 的配置，`None` 时按 `pull.rebase`；同步的拉取相同。
#[test]
fn an_explicit_pull_mode_overrides_the_git_config() {
    let Some(git) = real_git() else {
        return;
    };
    let diverge = |name: &str| {
        let (work, remote) = three_lines(name);
        push_from_peer(&format!("{name}-peer"), &remote, "b.txt", "theirs\n");
        std::fs::write(work.join("c.txt"), "ours\n").unwrap();
        sh(&work, &["add", "c.txt"]);
        sh(&work, &["commit", "-q", "-m", "ours"]);
        sh(&work, &["config", "pull.rebase", "true"]);
        work
    };
    let merges = |work: &Path| sh(work, &["rev-list", "--merges", "--count", "HEAD"]);

    let work = diverge("pull-mode-config");
    git.pull(&work, None).unwrap();
    assert_eq!(merges(&work), "0");

    let work = diverge("pull-mode-merge");
    git.pull(&work, Some(false)).unwrap();
    assert_eq!(merges(&work), "1");

    let work = diverge("pull-mode-sync");
    sh(&work, &["config", "pull.rebase", "false"]);
    assert!(matches!(
        git.sync(&work, false, Some(true)).unwrap(),
        Synced::Done(_)
    ));
    assert_eq!(merges(&work), "0");
    assert_eq!(sh(&work, &["log", "-1", "--format=%s"]), "ours");
}

/// 回归：2.27 以前的 `git` 合并方式拉取时不接受 `--autostash`，拉取总是失败。这时不带它：
/// 工作区干净、改动不冲突时照常拉取。
#[cfg(unix)]
#[test]
fn merging_pulls_work_with_git_before_merge_autostash() {
    use super::super::tests::real_git_with;
    let (work, remote) = three_lines("pull-old-git");
    // 按 2.25 的样子：报出旧版本号，合并方式的拉取遇到 `--autostash` 时退出。
    let fake = work.join("../pull-old-git-program");
    let body = "#!/bin/sh\n\
                case \" $* \" in *' version '*) echo 'git version 2.25.1'; exit 0;; esac\n\
                case \" $* \" in *' pull '*' --autostash '*) case \" $* \" in *' --rebase '*) ;; \
                *) echo 'fatal: --[no-]autostash option is only valid with --rebase.' >&2; \
                exit 128;; esac;; esac\n\
                exec git \"$@\"\n";
    winc_backend::testing::write_script(&fake, body);
    let Some(git) = real_git_with(Some(&fake.to_string_lossy())) else {
        return;
    };
    push_from_peer("pull-old-git-peer", &remote, "b.txt", "theirs\n");
    std::fs::write(work.join("a.txt"), "one\ntwo\nTHREE\n").unwrap();
    git.pull(&work, Some(false)).unwrap();
    assert!(work.join("b.txt").exists());
    assert_eq!(sh(&work, &["status", "--porcelain"]), "M a.txt");

    push_from_peer("pull-old-git-peer-2", &remote, "c.txt", "theirs\n");
    git.pull_from(&work, "origin", "main", false).unwrap();
    assert!(work.join("c.txt").exists());
    // 变基方式的拉取一直接受 `--autostash`。
    push_from_peer("pull-old-git-peer-3", &remote, "d.txt", "theirs\n");
    git.pull(&work, Some(true)).unwrap();
    assert!(work.join("d.txt").exists());
    assert_eq!(sh(&work, &["status", "--porcelain"]), "M a.txt");
}

/// 上游与当前分支不同名（从别人的合并请求签出的分支常常这样）：推送、同步写明推到上游；
/// 推送去另一个远程或设置了 `push.default` 时交给 `git` 按配置推送。
#[test]
fn a_differently_named_upstream_is_pushed_to_explicitly() {
    let config = |lines: &[(&str, &str)]| {
        lines
            .iter()
            .map(|(k, v)| format!("{k}\n{v}\0"))
            .collect::<String>()
    };
    let tracking = [
        ("branch.pr/316.remote", "fork"),
        ("branch.pr/316.merge", "refs/heads/feat/x"),
        ("branch.main.remote", "origin"),
    ];
    assert_eq!(
        upstream_target(&config(&tracking), "pr/316"),
        Some(("fork".to_owned(), "refs/heads/feat/x".to_owned()))
    );
    // 同名、上游在本仓库、远程名像选项：交给 `git`。
    assert_eq!(upstream_target(&config(&tracking), "main"), None);
    let local = [("branch.a.remote", "."), ("branch.a.merge", "refs/heads/b")];
    assert_eq!(upstream_target(&config(&local), "a"), None);
    let option = [
        ("branch.a.remote", "-x"),
        ("branch.a.merge", "refs/heads/b"),
    ];
    assert_eq!(upstream_target(&config(&option), "a"), None);
    // 推送去另一个远程、设置了 `push.default`：交给 `git`。推送去的就是上游所在的远程时照样
    // 写明目标。
    for (extra, expected) in [
        (("push.default", "upstream"), false),
        (("remote.pushdefault", "mine"), false),
        (("branch.pr/316.pushremote", "mine"), false),
        (("remote.pushdefault", "fork"), true),
    ] {
        let lines = [tracking[0], tracking[1], extra];
        let found = upstream_target(&config(&lines), "pr/316");
        assert_eq!(found.is_some(), expected, "{extra:?}");
    }

    let Some(git) = real_git() else {
        return;
    };
    let (work, remote) = cloned("push-renamed");
    sh(
        &work,
        &["switch", "-q", "-c", "pr/1", "--track", "origin/main"],
    );
    std::fs::write(work.join("b.txt"), "two\n").unwrap();
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "two"]);
    git.push(&work, true).unwrap();
    assert_eq!(
        sh(&remote, &["rev-parse", "main"]),
        sh(&work, &["rev-parse", "HEAD"])
    );
    assert_eq!(sh(&remote, &["branch", "--list", "pr/1"]), "");

    // 同步：先拉取别人推送的提交，再推回不同名的上游。
    push_from_peer("push-renamed-peer", &remote, "c.txt", "three\n");
    std::fs::write(work.join("d.txt"), "four\n").unwrap();
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "four"]);
    assert!(matches!(
        git.sync(&work, false, Some(false)).unwrap(),
        Synced::Done(_)
    ));
    assert_eq!(
        sh(&remote, &["rev-parse", "main"]),
        sh(&work, &["rev-parse", "HEAD"])
    );

    // 推送去另一个远程（从一个仓库拉取、推到自己的仓库）：按 `git` 的规则推到那里同名的
    // 分支，不改上游。
    let mine = crate::test_util::scratch("push-renamed-mine");
    sh(&mine, &["init", "-q", "--bare", "-b", "main"]);
    sh(&work, &["remote", "add", "mine", mine.to_str().unwrap()]);
    sh(&work, &["config", "remote.pushDefault", "mine"]);
    std::fs::write(work.join("e.txt"), "five\n").unwrap();
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "five"]);
    let before = sh(&remote, &["rev-parse", "main"]);
    git.push(&work, true).unwrap();
    assert_eq!(sh(&remote, &["rev-parse", "main"]), before);
    assert_eq!(
        sh(&mine, &["rev-parse", "pr/1"]),
        sh(&work, &["rev-parse", "HEAD"])
    );
}
