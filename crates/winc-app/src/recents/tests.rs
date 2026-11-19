//! 程序坞菜单列出的最近项目，以及跳转列表中最近目录的标题和启动参数的测试。

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

/// 目录最多 7 个，其余名额给文件，合计 10 个；不存在的跳过，与目录相同的文件不重复。
#[test]
fn the_dock_keeps_room_for_folders_and_fills_the_rest_with_files() {
    let paths = |prefix: &str, n: usize| -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("/{prefix}{i}")))
            .collect()
    };
    let (folders, files) = (paths("d", 9), paths("f", 9));
    let gone = Path::new("/d1");
    let (shown, picked) = dock_entries(&folders, &files, |p| p != gone);
    assert_eq!(shown.len(), DOCK_FOLDERS);
    assert!(!shown.iter().any(|p| p == gone));
    assert_eq!(picked, files[..DOCK_TOTAL - DOCK_FOLDERS]);

    let (shown, picked) = dock_entries(
        &folders[..2],
        &[folders[0].clone(), files[0].clone()],
        |_| true,
    );
    assert_eq!((shown.len(), picked), (2, vec![files[0].clone()]));
}
