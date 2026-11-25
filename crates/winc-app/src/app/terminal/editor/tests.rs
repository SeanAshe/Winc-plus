//! 编辑器与终端之间命令的测试：路径的引号处理，以及拖进终端时插入的路径。

use super::*;

#[test]
fn paths_are_quoted_only_when_needed() {
    assert_eq!(shell_quote(Path::new("/a/b-c_d.rs")), "/a/b-c_d.rs");
    assert_eq!(shell_quote(Path::new("/a b/c")), "'/a b/c'");
    assert_eq!(shell_quote(Path::new("it's")), r"'it'\''s'");
    assert_eq!(shell_quote(Path::new("$HOME;rm")), "'$HOME;rm'");
}

/// 远端的文件插入在远端主机上的路径，不是本机的虚拟路径。
#[test]
fn dropped_files_type_the_path_on_their_own_host() {
    assert_eq!(dropped_path_text(Path::new("/a b/c")), "'/a b/c' ");
    let target = winc_backend::Target::new("terminal-drop").unwrap();
    let remote = winc_backend::virtual_path(&target, b"/srv/notes.md");
    assert_eq!(dropped_path_text(&remote), "/srv/notes.md ");
}
