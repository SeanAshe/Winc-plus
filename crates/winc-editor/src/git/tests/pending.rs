//! 生成提交信息时读取的改动的测试：所选文件的补丁和最近提交的首行，以及读取时按上限截断。

use super::*;

/// 在目录 `root` 里运行 `git args`。
fn sh(root: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(root).args(args).output();
    assert!(out.is_ok_and(|o| o.status.success()), "git {args:?}");
}

/// 生成提交信息时读取的改动和最近提交：仓库尚无提交时读取暂存区的内容，最近提交为空；有提交后
/// 读取所选文件相对 `HEAD` 的改动，最近的提交在前，不含合并提交。
#[test]
fn pending_patch_and_recent_subjects_follow_the_commit() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-commit-patch");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let cancel = Cancel::default();
    let limits = PatchLimits {
        stat_files: 10,
        keep: 1 << 10,
        max_files: 10,
    };
    sh(&root, &["init", "-q", "-b", "main"]);
    let (a, b) = (root.join("a.rs"), root.join("b.rs"));
    std::fs::write(&a, "one\n").unwrap();
    git.stage(&root, std::slice::from_ref(&a)).unwrap();
    assert_eq!(git.recent_subjects(&root, 10, &cancel), Ok(Vec::new()));
    let picked = git.pending_patch(&root, false, std::slice::from_ref(&a), limits, &cancel);
    assert!(picked.unwrap().files[0].text.contains("+one"));
    git.commit(&root, "first", false, false).unwrap();

    std::fs::write(&a, "one\ntwo\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    git.stage(&root, std::slice::from_ref(&b)).unwrap();
    git.commit(&root, "second", false, false).unwrap();
    let picked = git.pending_patch(&root, false, std::slice::from_ref(&a), limits, &cancel);
    let picked = picked.unwrap();
    assert!(picked.stat.contains("a.rs | 1 +"), "{}", picked.stat);
    assert_eq!(picked.files.len(), 1);
    assert!(
        picked.files[0]
            .text
            .starts_with("diff --git a/a.rs b/a.rs\n")
    );
    assert!(picked.files[0].text.ends_with("+two\n"));
    assert_eq!(picked.files[0].size, picked.files[0].text.len());
    let staged = git
        .pending_patch(&root, true, &[], limits, &cancel)
        .unwrap();
    assert_eq!(staged, PendingPatch::default());
    let subjects = git.recent_subjects(&root, 10, &cancel).unwrap();
    assert_eq!(subjects, ["second", "first"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 补丁读取时即按上限截断：每个文件只保留开头的整行，完整大小照常计数；读满文件数后停止，统计
/// 仍列出全部文件。
#[test]
fn pending_patch_keeps_only_the_start_of_each_file() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-commit-patch-limits");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let cancel = Cancel::default();
    sh(&root, &["init", "-q", "-b", "main"]);
    let files: Vec<PathBuf> = (0..3).map(|i| root.join(format!("{i}.txt"))).collect();
    for file in &files {
        std::fs::write(file, "start\n").unwrap();
    }
    git.stage(&root, &files).unwrap();
    git.commit(&root, "first", false, false).unwrap();
    let body: String = (0..2000).map(|i| format!("line {i}\n")).collect();
    for file in &files {
        std::fs::write(file, &body).unwrap();
    }

    let limits = PatchLimits {
        stat_files: 10,
        keep: 1 << 10,
        max_files: 2,
    };
    let patch = git
        .pending_patch(&root, false, &files, limits, &cancel)
        .unwrap();
    assert!(patch.partial);
    assert_eq!(patch.files.len(), 2);
    for file in &patch.files {
        assert!(file.text.len() <= 1 << 10 && file.text.ends_with('\n'));
        assert!(file.size > 10 * file.text.len(), "{}", file.size);
    }
    assert!(patch.stat.contains("3 files changed"), "{}", patch.stat);
    let _ = std::fs::remove_dir_all(&dir);
}
