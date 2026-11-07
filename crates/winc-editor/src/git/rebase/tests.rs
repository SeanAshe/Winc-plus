//! `rebase` 的测试：待办列表的生成、输出解析，以及在真实仓库中重排、丢弃、压缩、改写和暂停。

use super::*;

fn step(oid: &str, action: RebaseAction, message: Option<&str>) -> RebaseStep {
    RebaseStep {
        oid: oid.to_owned(),
        action,
        message: message.map(str::to_owned),
    }
}

#[test]
fn every_action_maps_to_its_todo_command() {
    use RebaseAction::*;
    let todo = rebase_todo(&[
        step("a", Pick, None),
        step("b", Edit, None),
        step("c", Squash, None),
        step("d", Fixup, None),
        step("e", Drop, None),
    ]);
    assert_eq!(todo, "pick a\nedit b\nsquash c\nfixup d\ndrop e\n");
}

#[test]
fn a_new_message_stays_on_one_todo_line() {
    let todo = rebase_todo(&[step("a", RebaseAction::Reword, Some("it's\nC:\\x 100%"))]);
    let lines: Vec<&str> = todo.lines().collect();
    assert_eq!(lines[0], "pick a");
    assert_eq!(lines.len(), 2, "{todo}");
    assert!(lines[1].contains(r"'it'\''s\nC:\\x 100%'"), "{todo}");
}

#[test]
fn log_output_is_split_into_commits_and_merges_are_noticed() {
    let text = "a\x1f\x1ffirst\n\nbody\n\x1e\nb\x1fa\x1fsecond\n\x1e\n";
    let (commits, merges) = parse_commits(text);
    assert!(!merges);
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].subject, "first");
    assert_eq!(commits[0].message, "first\n\nbody");
    assert_eq!(commits[1].oid, "b");
    let (_, merges) = parse_commits("c\x1fa b\x1fmerge\n\x1e");
    assert!(merges);
}

/// 在临时目录建立仓库，依次提交 `names`：每个提交新增一个同名文件。
fn repository(name: &str, names: &[&str]) -> Option<(Git, PathBuf)> {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let git = Git::find(&shell).ok()?;
    let dir = crate::test_util::scratch(name);
    let root = winc_config::files::canonicalize(&dir).unwrap();
    run(&root, &["init", "-q", "-b", "main"]);
    run(&root, &["config", "user.name", "t"]);
    run(&root, &["config", "user.email", "t@t"]);
    run(&root, &["config", "commit.gpgsign", "false"]);
    for name in names {
        std::fs::write(root.join(name), name).unwrap();
        run(&root, &["add", "--all"]);
        run(&root, &["commit", "-q", "-m", name]);
    }
    Some((git, root))
}

fn run(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn subjects(root: &Path) -> Vec<String> {
    run(root, &["log", "--format=%s"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn oid_of(root: &Path, subject: &str) -> String {
    let log = run(root, &["log", "--format=%H %s"]);
    let line = log.lines().find(|l| l.ends_with(subject)).unwrap();
    line.split(' ').next().unwrap().to_owned()
}

#[test]
fn a_plan_covers_the_commit_and_everything_after_it() {
    let Some((git, root)) = repository("rebase-plan", &["a", "b", "c"]) else {
        return;
    };
    let plan = git.rebase_plan(&root, &oid_of(&root, "b")).unwrap();
    assert_eq!(plan.base.as_deref(), Some(oid_of(&root, "a").as_str()));
    let names: Vec<&str> = plan.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(names, ["b", "c"]);
    assert_eq!(plan.pushed, 0);

    // 从根提交开始时没有基点。
    let plan = git.rebase_plan(&root, &oid_of(&root, "a")).unwrap();
    assert_eq!(plan.base, None);
    assert_eq!(plan.commits.len(), 3);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_plan_refuses_commits_off_the_branch_and_merges() {
    let Some((git, root)) = repository("rebase-refuse", &["a", "b"]) else {
        return;
    };
    run(&root, &["switch", "-q", "-c", "side", "HEAD~1"]);
    std::fs::write(root.join("s"), "s").unwrap();
    run(&root, &["add", "--all"]);
    run(&root, &["commit", "-q", "-m", "s"]);
    let side = oid_of(&root, "s");
    run(&root, &["switch", "-q", "main"]);
    assert!(git.rebase_plan(&root, &side).is_err());
    let first = oid_of(&root, "b");
    run(&root, &["merge", "-q", "--no-edit", "--no-ff", "side"]);
    assert!(git.rebase_plan(&root, &first).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn commits_can_be_reordered_dropped_squashed_and_reworded() {
    use RebaseAction::*;
    let Some((git, root)) = repository("rebase-run", &["a", "b", "c", "d", "e"]) else {
        return;
    };
    // 未提交的更改应当保留。
    std::fs::write(root.join("a"), "changed").unwrap();
    let oid = |name: &str| oid_of(&root, name);
    let steps = [
        step(&oid("c"), Reword, Some("it's new\n\nC:\\x 100% \"q\"")),
        step(&oid("b"), Pick, None),
        step(&oid("d"), Fixup, None),
        step(&oid("e"), Drop, None),
    ];
    let stopped = git
        .interactive_rebase(&root, Some(&oid("a")), &steps)
        .unwrap();
    assert!(!stopped);
    assert_eq!(subjects(&root), ["b", "it's new", "a"]);
    let message = run(&root, &["log", "-1", "--format=%B", "HEAD~1"]);
    assert_eq!(message.trim_end(), "it's new\n\nC:\\x 100% \"q\"");
    // `fixup` 的文件并入 `b`，丢弃的文件不存在。
    assert!(root.join("d").exists() && !root.join("e").exists());
    assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "changed");
    // 待办文件不留在仓库里。
    let todo = run(&root, &["rev-parse", "--git-path", "winc-rebase-todo"]);
    assert!(!root.join(todo.trim()).exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn editing_pauses_and_a_conflict_is_reported() {
    use RebaseAction::*;
    let Some((git, root)) = repository("rebase-pause", &["a", "b", "c"]) else {
        return;
    };
    let oid = |name: &str| oid_of(&root, name);
    let steps = [step(&oid("b"), Edit, None), step(&oid("c"), Pick, None)];
    assert!(
        git.interactive_rebase(&root, Some(&oid("a")), &steps)
            .unwrap()
    );
    run(&root, &["rebase", "--abort"]);

    // 两个提交修改同一行，交换顺序产生冲突，变基停在冲突处。
    std::fs::write(root.join("a"), "one").unwrap();
    run(&root, &["commit", "-qam", "one"]);
    std::fs::write(root.join("a"), "two").unwrap();
    run(&root, &["commit", "-qam", "two"]);
    let steps = [step(&oid("two"), Pick, None), step(&oid("one"), Pick, None)];
    assert!(
        git.interactive_rebase(&root, Some(&oid("c")), &steps)
            .is_err()
    );
    assert!(
        root.join(run(&root, &["rev-parse", "--git-path", "rebase-merge"]).trim())
            .exists()
    );
    run(&root, &["rebase", "--abort"]);
    let _ = std::fs::remove_dir_all(&root);
}

/// 从根提交开始时使用 `--root`。
#[test]
fn rebasing_from_the_root_works() {
    use RebaseAction::*;
    let Some((git, root)) = repository("rebase-root", &["a", "b"]) else {
        return;
    };
    let steps = [
        step(&oid_of(&root, "b"), Pick, None),
        step(&oid_of(&root, "a"), Pick, None),
    ];
    assert!(!git.interactive_rebase(&root, None, &steps).unwrap());
    assert_eq!(subjects(&root), ["a", "b"]);
    let _ = std::fs::remove_dir_all(&root);
}
