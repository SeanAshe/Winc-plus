//! 作用于大量文件的命令：路径参数分批，储藏由标准输入读取路径。

use super::*;
use crate::git::pathspec::{PATHS_PER_CALL, merge_folders, relative_all};

/// 新建仓库：`.gitignore` 忽略 `ignored/`，此外有 `count` 个名字很长的源文件、名字含 `[` 的
/// `[ab].rs`，以及按通配符会被 `[ab].rs` 匹配上的 `a.rs`。都未跟踪，返回仓库目录和各源文件。
fn many_files(name: &str, count: usize) -> (PathBuf, Vec<PathBuf>) {
    let dir = crate::test_util::scratch(name);
    let ok = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .arg(&dir)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok);
    std::fs::write(dir.join(".gitignore"), "ignored/\n").unwrap();
    std::fs::create_dir_all(dir.join("ignored")).unwrap();
    std::fs::write(dir.join("ignored/local.txt"), "local\n").unwrap();
    let deep = dir.join("src/main/java/example/application/service/impl");
    std::fs::create_dir_all(&deep).unwrap();
    let files: Vec<PathBuf> = (0..count)
        .map(|i| deep.join(format!("Feature{i:04}ServiceImplementation.java")))
        .collect();
    for file in &files {
        std::fs::write(file, "class Example {}\n").unwrap();
    }
    for name in ["[ab].rs", "a.rs"] {
        std::fs::write(dir.join(name), "fn a() {}\n").unwrap();
    }
    (dir, files)
}

/// 暂存区中的文件（相对路径）。
fn staged_names(git: &Git, dir: &Path) -> Vec<PathBuf> {
    let status = git.status(dir, &Cancel::default()).unwrap();
    let staged = status.entries.iter().filter(|e| e.staged.is_some());
    staged
        .map(|e| e.path.strip_prefix(dir).unwrap().to_path_buf())
        .collect()
}

#[test]
fn path_batches_stay_under_the_command_line_limit_in_order() {
    let root = Path::new("/repo");
    let paths: Vec<PathBuf> = (0..2000)
        .map(|i| root.join(format!("src/very/long/folder/name/file{i:04}.rs")))
        .collect();
    let batches = path_batches(root, &paths);
    assert!(batches.len() > 1);
    for batch in &batches {
        let length: usize = batch.iter().map(|s| s.len() + 3).sum();
        assert!(length <= PATHS_PER_CALL, "{length}");
    }
    let all: Vec<OsString> = batches.into_iter().flatten().collect();
    let expected: Vec<OsString> = relative_all(root, &paths).collect();
    assert_eq!(all, expected);
    // 没有路径时仍运行一次，参数与原来相同。
    assert_eq!(path_batches(root, &[]), [Vec::<OsString>::new()]);
}

#[test]
fn whole_selected_folders_merge_into_one_path() {
    let root = Path::new("/repo");
    let p = |s: &str| root.join(s);
    let changed = [
        p("a/x/1.rs"),
        p("a/x/2.rs"),
        p("a/y.rs"),
        p("b/1.rs"),
        p("b/2.rs"),
        p("top.rs"),
    ];
    // `a` 下的改动全部选中，合并到最上层的 `a`；`b` 下有一个没选中，逐个保留。
    let chosen = [
        p("a/x/1.rs"),
        p("a/y.rs"),
        p("a/x/2.rs"),
        p("b/1.rs"),
        p("top.rs"),
    ];
    assert_eq!(
        merge_folders(root, &chosen, &changed),
        [p("a"), p("b/1.rs"), p("top.rs")]
    );
    // 仓库根目录本身不合并。
    assert_eq!(merge_folders(root, &changed, &changed).len(), 3);
}

/// 回归：一次暂存上千个文件时，全部路径拼进一条命令，Windows 拒绝启动 `git`（os error 206）。
/// 分批暂存后只加入选中的文件：被忽略的文件不加入，`[ab].rs` 不当通配符。
#[test]
fn staging_many_files_keeps_to_the_selection() {
    let Some(git) = real_git() else {
        return;
    };
    let (dir, files) = many_files("git-stage-many", 1500);
    let mut chosen = files.clone();
    chosen.push(dir.join("[ab].rs"));
    chosen.push(dir.join(".gitignore"));
    assert!(path_batches(&dir, &chosen).len() > 1);
    git.stage(&dir, &chosen).unwrap();
    let staged = staged_names(&git, &dir);
    assert_eq!(staged.len(), chosen.len());
    assert!(staged.contains(&PathBuf::from("[ab].rs")));
    assert!(!staged.contains(&PathBuf::from("a.rs")));
    assert!(!staged.iter().any(|p| p.starts_with("ignored")));

    // 移出暂存区同样分批。
    git.unstage(&dir, &chosen).unwrap();
    assert!(staged_names(&git, &dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：储藏大量文件时只生成一份储藏；整个选中的目录合并成一项，Windows 上 `git` 内部的
/// 命令不因路径过多而无法运行（`Filename too long`）。
#[test]
fn stashing_many_files_makes_one_stash() {
    let Some(git) = real_git() else {
        return;
    };
    let (dir, files) = many_files("git-stash-many", 1500);
    git.stage(&dir, &[dir.join(".gitignore"), dir.join("a.rs")])
        .unwrap();
    git.commit(&dir, "first", false, false).unwrap();
    git.stash_paths(&dir, "many", &files).unwrap();
    let list = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["stash", "list"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&list.stdout).lines().count(), 1);
    assert!(files.iter().all(|f| !f.exists()));
    // 未选中的文件留在工作区。
    assert!(dir.join("[ab].rs").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
