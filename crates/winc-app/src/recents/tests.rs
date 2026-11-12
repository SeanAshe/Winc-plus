//! 跳转列表中最近目录的标题和启动参数的测试。

use super::*;

#[test]
fn labels_are_folder_names_with_the_parent_added_only_for_duplicates() {
    let folders = [
        PathBuf::from("/work/winc"),
        PathBuf::from("/work/api"),
        PathBuf::from("/backup/winc"),
        PathBuf::from("/"),
    ];
    assert_eq!(
        labels(&folders),
        ["winc — /work", "api", "winc — /backup", "/"]
    );
    assert!(labels(&[]).is_empty());
}

#[test]
fn windows_arguments_are_quoted_and_keep_a_trailing_backslash_literal() {
    assert_eq!(
        argument(Path::new(r"C:\work\my app")),
        r#""C:\work\my app""#
    );
    // 结尾的反斜杠写两遍，否则其后的引号被当作字面量。
    assert_eq!(argument(Path::new(r"C:\")), r#""C:\\""#);
    assert_eq!(
        argument(Path::new(r"\\server\share\")),
        r#""\\server\share\\""#
    );
}
