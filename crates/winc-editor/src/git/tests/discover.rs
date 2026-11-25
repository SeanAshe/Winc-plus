//! 查找仓库的测试：按层数查找目录中的仓库，读状态时发现嵌套的仓库。

use super::*;

#[test]
fn scanning_finds_nested_repositories_within_the_depth() {
    let dir = crate::test_util::scratch("git-scan");
    let mark = |path: &str| std::fs::create_dir_all(dir.join(path).join(".git")).unwrap();
    mark("a");
    mark("group/b");
    mark("group/deep/c");
    // 找到仓库后继续查找其中的目录，嵌套的仓库同样找到；依赖目录、隐藏目录都跳过。
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
    assert_eq!(
        names(2),
        [
            PathBuf::from("a"),
            PathBuf::from("a/inner"),
            PathBuf::from("group/b"),
        ]
    );
    assert_eq!(
        names(3),
        [
            PathBuf::from("a"),
            PathBuf::from("a/inner"),
            PathBuf::from("group/b"),
            PathBuf::from("group/deep/c"),
        ]
    );

    let cancelled = Cancel::default();
    cancelled.cancel();
    assert!(scan_repositories(&dir, 3, &cancelled).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 仓库里嵌套的另一个仓库（未被跟踪）不列为未跟踪的目录，记在 `nested` 中；普通的未跟踪目录照常
/// 列出。
#[test]
fn nested_repositories_are_not_untracked_files() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-nested-status");
    let sh = |dir: &Path, args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    std::fs::create_dir_all(dir.join("libs/b")).unwrap();
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    std::fs::write(dir.join("a.rs"), "a\n").unwrap();
    sh(&dir, &["init", "-q", "-b", "main"]);
    sh(&dir, &["add", "a.rs"]);
    sh(&dir, &["commit", "-q", "-m", "first"]);
    std::fs::write(dir.join("libs/b/b.rs"), "b\n").unwrap();
    sh(&dir.join("libs/b"), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("notes/n.md"), "n\n").unwrap();

    let root = winc_config::files::canonicalize(&dir).unwrap();
    let status = git.status(&root, &Cancel::default()).unwrap();
    let paths: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(paths, [root.join("notes/n.md")]);
    assert_eq!(status.nested, [root.join("libs/b")]);
    let _ = std::fs::remove_dir_all(&dir);
}
