use super::*;
use crate::git::tests::real_git;

/// 一个提交：提交号 `oid`、父提交 `parents`、引用 `refs`。
fn commit(oid: &str, parents: &[&str], refs: Vec<Ref>) -> Commit {
    Commit {
        oid: oid.to_owned(),
        parents: parents.iter().map(|p| (*p).to_owned()).collect(),
        author: String::new(),
        email: String::new(),
        time: 0,
        local: String::new(),
        refs,
        subject: oid.to_owned(),
    }
}

#[test]
fn commits_split_into_outgoing_incoming_and_shared() {
    // 本地在 `b` 之上提交了 `l1`、`l2`，上游在 `b` 之上多了 `r1`；`b`、`a` 两边都有。
    let commits = [
        commit("l2", &["l1"], vec![Ref::Head("main".into())]),
        commit("r1", &["b"], vec![Ref::Remote("origin/main".into())]),
        commit("l1", &["b"], vec![]),
        commit("b", &["a"], vec![]),
        commit("a", &[], vec![]),
    ];
    use Side::*;
    assert_eq!(
        sides(&commits, Some("origin/main")),
        [Outgoing, Incoming, Outgoing, Both, Both]
    );
    // 没有上游时不区分。
    assert_eq!(sides(&commits, None), [Both; 5]);
    // 领先很多、上游不在列出的范围内：列出的都是尚未推送的。
    let ahead = [
        commit("l2", &["l1"], vec![Ref::Head("main".into())]),
        commit("l1", &["b"], vec![]),
    ];
    assert_eq!(sides(&ahead, Some("origin/main")), [Outgoing, Outgoing]);
    // 落后很多、`HEAD` 不在列出的范围内：列出的都是尚未拉取的。
    let behind = [
        commit("r2", &["r1"], vec![Ref::Remote("origin/main".into())]),
        commit("r1", &["b"], vec![]),
    ];
    assert_eq!(sides(&behind, Some("origin/main")), [Incoming, Incoming]);
}

#[test]
fn recent_reads_the_branch_and_its_upstream_with_a_graph() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-recent");
    let sh = |args: &[&str]| {
        let ok = std::process::Command::new("git")
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
    for message in ["one", "two"] {
        sh(&["commit", "-q", "--allow-empty", "-m", message]);
    }
    // 上游在「two」之上多一个提交，本地在「two」之上提交「local」。
    sh(&["switch", "-q", "-c", "theirs"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "theirs"]);
    sh(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    sh(&["switch", "-q", "main"]);
    sh(&["branch", "-q", "-D", "theirs"]);
    sh(&["commit", "-q", "--allow-empty", "-m", "local"]);

    let root = winc_config::files::canonicalize(&dir).unwrap();
    let recent = git.recent(&root, Some("origin/main")).unwrap();
    let mut subjects: Vec<_> = recent.commits.iter().map(|c| c.subject.as_str()).collect();
    // 「local」与「theirs」的提交时间可能相同，先后不定。
    subjects[..2].sort_unstable();
    assert_eq!(subjects, ["local", "theirs", "two", "one"]);
    let side = |subject: &str| {
        let i = recent.commits.iter().position(|c| c.subject == subject);
        recent.sides[i.unwrap()]
    };
    assert_eq!(side("local"), Side::Outgoing);
    assert_eq!(side("theirs"), Side::Incoming);
    assert_eq!((side("two"), side("one")), (Side::Both, Side::Both));
    assert_eq!(recent.graph.len(), 4);
    assert!(recent.graph.iter().any(|row| row.lane == 1));
    assert!(!recent.more);
    let _ = std::fs::remove_dir_all(&dir);
}
