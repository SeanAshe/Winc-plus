//! 同名的最近目录的区分：上级目录中需要保留的末尾一段。

use super::*;

/// 从最后一级目录起逐级向前，直到与每个同名目录都不同；无法区分时保留整个路径。
#[test]
fn the_kept_tail_is_the_shortest_one_that_differs() {
    let kept = |parent: &'static str, others: &[&str]| &parent[distinct_start(parent, others)..];
    assert_eq!(kept("~/work/a/src", &["~/work/b/src"]), "a/src");
    assert_eq!(kept("~/work/a", &["~/play/b"]), "a");
    assert_eq!(kept("~/x/a/src", &["~/y/a/src", "~/x/b/src"]), "x/a/src");
    // 另一个更短、是自己结尾的一部分：多保留一级即可区分。
    assert_eq!(kept("/srv/a/src", &["a/src"]), "srv/a/src");
    // 同一路径在不同的远端主机上：只有主机不同，保留整个路径。
    assert_eq!(kept("dev:/home/me", &["box:/home/me"]), "dev:/home/me");
    assert_eq!(kept(r"C:\work\a", &[r"D:\work\b"]), "a");
}

/// 只有最近列表中另有同名目录时才标出要保留的一段。
#[test]
fn only_folders_sharing_a_name_keep_a_tail() {
    let folders = [
        PathBuf::from("/work/a/projects/winc"),
        PathBuf::from("/work/api"),
        PathBuf::from("/work/b/projects/winc"),
    ];
    let parents = parents(&folders);
    let kept = |i: usize| parents[i].keep.map(|k| &parents[i].text[k..]);
    assert_eq!(kept(0), Some("a/projects"));
    assert_eq!(kept(1), None);
    assert_eq!(kept(2), Some("b/projects"));
    assert_eq!(parents[1].text, "/work");
}
