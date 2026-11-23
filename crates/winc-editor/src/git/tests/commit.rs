//! 提交与状态的测试：读取父提交、在浅克隆的边界上撤销提交、检查新增的行时不运行仓库配置的命令，
//! 以及忽略的路径的数量上限。

use super::*;

/// 在目录 `root` 里运行 `git args`，返回标准输出。
fn sh(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(root).args(args).output();
    String::from_utf8(out.unwrap().stdout).unwrap()
}

#[test]
fn parents_are_read_from_the_commit_header() {
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    let merge =
        format!("tree {a}\nparent {a}\nparent {b}\nauthor t <t@t> 1 +0000\n\nparent of nothing\n");
    assert_eq!(crate::git::commit::commit_parents(&merge), [a.clone(), b]);
    // 提交信息里以 `parent ` 开头的行不是父提交。
    let root = format!("tree {a}\nauthor t <t@t> 1 +0000\n\nparent {a}\n");
    assert!(crate::git::commit::commit_parents(&root).is_empty());
}

/// 浅克隆边界上的提交有父提交而仓库里没有它：撤销失败，分支的引用不动。
#[test]
fn undoing_the_boundary_commit_of_a_shallow_clone_keeps_the_branch() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-undo-shallow");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let (origin, shallow) = (dir.join("origin"), dir.join("shallow"));
    std::fs::create_dir_all(&origin).unwrap();
    sh(&origin, &["init", "-q", "-b", "main"]);
    let a = origin.join("a.rs");
    for text in ["one\n", "two\n"] {
        std::fs::write(&a, text).unwrap();
        git.stage(&origin, std::slice::from_ref(&a)).unwrap();
        git.commit(&origin, text.trim(), false, false).unwrap();
    }
    let url = format!("file://{}", origin.display());
    sh(&dir, &["clone", "-q", "--depth", "1", &url, "shallow"]);
    let head = git.last_commit(&shallow).unwrap();

    assert!(git.undo_commit(&shallow, &head.oid).is_err());
    assert_eq!(git.last_commit(&shallow).unwrap().oid, head.oid);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 提交前检查新增的行：仓库配置的 `diff.*.textconv` 不运行。
#[test]
fn added_lines_run_no_textconv_program() {
    let Some(git) = real_git() else {
        return;
    };
    let dir = crate::test_util::scratch("git-added-textconv");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    sh(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitattributes"), "*.t diff=x\n").unwrap();
    let file = root.join("a.t");
    std::fs::write(&file, "one\n").unwrap();
    git.stage(&root, &[file.clone(), root.join(".gitattributes")])
        .unwrap();
    git.commit(&root, "first", false, false).unwrap();
    let marker = root.join("ran");
    let script = root.join("run.sh");
    let body = format!("#!/bin/sh\ntouch '{}'\ncat \"$1\"\n", marker.display());
    winc_backend::testing::write_script(&script, &body);
    sh(
        &root,
        &["config", "diff.x.textconv", script.to_str().unwrap()],
    );
    std::fs::write(&file, "one\ntwo\n").unwrap();

    let added = git.added_lines(&root, false, std::slice::from_ref(&file));
    assert!(!marker.exists(), "检查新增的行时运行了配置中的命令");
    // 配置不可信的仓库里命令仍可读取差异，或被拒绝；两种结果都不运行配置的命令。
    if let Ok(added) = added {
        assert_eq!(added, ["two"]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn status_keeps_only_the_first_ignored_paths_past_the_limit() {
    let mut output = String::from("# branch.oid (initial)\0# branch.head main\0");
    for i in 0..STATUS_LIMIT + 5 {
        output.push_str(&format!("! out/{i}.o\0"));
    }
    output.push_str("? new.rs\0");
    let status = parse_status(output.as_bytes(), &root()).unwrap();
    assert_eq!(status.ignored.len(), STATUS_LIMIT);
    assert_eq!(status.entries.len(), 1);
}
