//! 遍历规则的测试：名字的自然排序，查看变化了的路径。

use super::*;

#[test]
fn natural_order() {
    let mut names = vec!["file10", "File2", "file1", "a", "B"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["a", "B", "file1", "File2", "file10"]);

    // 只差大小写或前导零的名字也有确定的先后，相同的名字才相等。
    let mut names = vec!["file01", "a.rs", "file1", "A.rs", "file001"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["A.rs", "a.rs", "file1", "file01", "file001"]);
    assert_eq!(natural_cmp("x01y2", "x1y02"), Ordering::Greater);
    assert_eq!(natural_cmp("a.rs", "a.rs"), Ordering::Equal);
}

/// 查看变化了的路径：按所在目录的忽略规则判断是否收录，目录连同其下的内容收录到限定的层数，
/// 刚收录过的目录之下的路径不再查看。
#[test]
fn changed_paths_are_probed_like_the_walk_sees_them() {
    let root = std::env::temp_dir().join(format!("winc-search-probe-{}", std::process::id()));
    for file in [
        "a.rs",
        "debug.log",
        "dir/x.rs",
        "dir/deep/y.rs",
        "dir/deep/deeper/z.rs",
    ] {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    let paths: Vec<String> = [
        "",
        ".git",
        "a.rs",
        "debug.log",
        "dir",
        "dir/x.rs",
        "gone.rs",
    ]
    .map(str::to_owned)
    .to_vec();
    let mut sorted = paths.clone();
    sorted.sort_by(|a, b| Path::new(a).cmp(Path::new(b)));
    assert_eq!(sorted, paths);

    let found = probe(&root, &paths[1..], Some(3), &AtomicBool::new(false));
    assert_eq!(found[0], Probe::Unlisted);
    assert_eq!(found[1], Probe::File);
    assert_eq!(found[2], Probe::Unlisted);
    // 目录下的顺序随文件系统而定，调用方自己排序。
    let Probe::Dir {
        mut files,
        mut dirs,
    } = found[3].clone()
    else {
        panic!("{found:?}");
    };
    files.sort();
    dirs.sort();
    assert_eq!(files, ["dir/deep/y.rs", "dir/x.rs"]);
    assert_eq!(dirs, ["dir", "dir/deep"]);
    assert_eq!(found[4], Probe::Covered);
    assert_eq!(found[5], Probe::Missing);

    // 根目录本身：全部收录的文件。
    let whole = probe(&root, &paths[..1], None, &AtomicBool::new(false));
    let Probe::Dir { mut files, .. } = whole[0].clone() else {
        panic!("{whole:?}");
    };
    files.sort();
    assert_eq!(
        files,
        [
            ".gitignore",
            "a.rs",
            "dir/deep/deeper/z.rs",
            "dir/deep/y.rs",
            "dir/x.rs"
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}
