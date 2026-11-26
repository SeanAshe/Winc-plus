//! 编辑器与终端之间命令的测试：路径的引号处理，以及拖进终端时插入的路径。

use super::*;

#[test]
fn paths_are_quoted_only_when_needed() {
    assert_eq!(shell_quote(Path::new("/a/b-c_d.rs")), "/a/b-c_d.rs");
    assert_eq!(shell_quote(Path::new("/a b/c")), "'/a b/c'");
    assert_eq!(shell_quote(Path::new("it's")), r"'it'\''s'");
    assert_eq!(shell_quote(Path::new("$HOME;rm")), "'$HOME;rm'");
    // Windows 的路径：反斜杠和盘符不需要引号，空格、括号、`&` 需要，用双引号。
    assert_eq!(windows_quote(r"C:\a\b-c_d.png"), r"C:\a\b-c_d.png");
    assert_eq!(
        windows_quote(r"C:\Users\A B\x.png"),
        r#""C:\Users\A B\x.png""#
    );
    assert_eq!(windows_quote(r"C:\a&b (1)\x"), r#""C:\a&b (1)\x""#);
    assert_eq!(windows_quote("截图.png"), "截图.png");
}

/// 本机的文件按本机命令行的规则加引号；远端的文件插入在远端主机上的路径，不是本机的虚拟路径，
/// 按 shell 的规则加引号。
#[test]
fn dropped_files_type_the_path_on_their_own_host() {
    if cfg!(windows) {
        assert_eq!(dropped_path_text(Path::new(r"C:\a b\c")), r#""C:\a b\c" "#);
    } else {
        assert_eq!(dropped_path_text(Path::new("/a b/c")), "'/a b/c' ");
    }
    let target = winc_backend::Target::new("terminal-drop").unwrap();
    let remote = winc_backend::virtual_path(&target, b"/srv/notes.md");
    assert_eq!(dropped_path_text(&remote), "/srv/notes.md ");
}
