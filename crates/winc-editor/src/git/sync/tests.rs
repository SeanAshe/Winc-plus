//! `sync` 的测试：本地改写已推送的提交后以本地版本推送；上游有其他人推送的提交时先拉取再推送。
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
