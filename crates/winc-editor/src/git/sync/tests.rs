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
    assert_eq!(git.sync(&work, false).unwrap(), Synced::NeedsForce(1));
    assert_eq!(sh(&remote, &["rev-parse", "main"]), before);

    git.sync(&work, true).unwrap();
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

    git.sync(&work, false).unwrap();
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
    git.pull(&work).unwrap();
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
    git.pull(&work).unwrap();
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
    git.pull(&work).unwrap();
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
    git.pull_rebase(&work).unwrap();
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
    assert!(matches!(git.sync(&work, false).unwrap(), Synced::Done(_)));
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
    let error = git.pull(&work).unwrap_err();
    assert!(error.contains("new.txt"), "{error}");
    assert_eq!(
        std::fs::read_to_string(work.join("new.txt")).unwrap(),
        "mine\n"
    );
    let text = std::fs::read_to_string(work.join("a.txt")).unwrap();
    assert_eq!(text, "one\ntwo\nTHREE\n");
    assert_eq!(git.stash_tip(&work), None);
}
