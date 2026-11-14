//! 日志模块的测试：提交时间的显示和本地时间的换算，改动的文件按目录分组的排法。

use super::*;

/// 日志标签的标题，测试中与 [`log_title`] 在默认语言下的结果比较。
#[cfg(unix)]
pub(crate) const LOG_TITLE: &str = "Git 日志";

fn commit(local: &str, time: i64) -> Commit {
    Commit {
        oid: "a".into(),
        parents: Vec::new(),
        author: String::new(),
        email: String::new(),
        time,
        local: local.into(),
        refs: Vec::new(),
        subject: String::new(),
    }
}

#[test]
fn times_read_like_a_person_would_say_them() {
    // 东八区：本地 2026-10-26 14:32 是 UTC 06:32。
    let at = naive_seconds("2026-10-26 14:32").unwrap() - 8 * 3600;
    let c = commit("2026-10-26 14:32", at);
    assert_eq!(display_time(&c, at + 30), "刚刚");
    assert_eq!(display_time(&c, at + 25 * 60), "25 分钟前");
    assert_eq!(display_time(&c, at + 3 * 3600), "今天 14:32");
    // 第二天早上（本地 10-27 08:00）。
    assert_eq!(display_time(&c, at + 17 * 3600 + 28 * 60), "昨天 14:32");
    assert_eq!(display_time(&c, at + 30 * 86_400), "10月26日");
    assert_eq!(display_time(&c, at + 120 * 86_400), "2026年10月26日");
    // 列表里今天、昨天的写「今天」「昨天」，今年更早的「月-日 时:分」（补零），更早的
    // 「年-月-日」；不写「几分钟前」。
    assert_eq!(list_time(&c, at + 30 * 86_400), "10-26 14:32");
    assert_eq!(list_time(&c, at + 3 * 3600), "今天 14:32");
    assert_eq!(list_time(&c, at + 30), "今天 14:32");
    assert_eq!(list_time(&c, at + 17 * 3600 + 28 * 60), "昨天 14:32");
    assert_eq!(list_time(&c, at + 120 * 86_400), "2026-10-26");
    let early = commit("2026-09-03 18:22", at - 53 * 86_400);
    assert_eq!(list_time(&early, at), "09-03 18:22");
}

#[test]
fn local_text_converts_with_the_offset() {
    let at = naive_seconds("2026-10-26 14:32").unwrap() - 8 * 3600;
    assert_eq!(local_text(at, 8 * 3600), "2026-10-26 14:32");
    assert_eq!(local_text(at, 0), "2026-10-26 06:32");
    assert_eq!(civil_from_days(days_from_civil(2000, 2, 29)), (2000, 2, 29));
}

#[test]
fn civil_dates_round_trip() {
    for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 10, 26), (2026, 12, 31)] {
        assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
    }
    assert_eq!(days_from_civil(1970, 1, 2), 1);
}

/// 当前文本的行号换算成 `HEAD` 中的行号：改动之后的按增减的行数平移，改动里的换成这段在
/// `HEAD` 中的开头，新加的行换成它前面的一行。
#[test]
fn caret_lines_map_back_to_head() {
    use super::filter::head_line;
    use winc_core::diff::LineHunk;
    // 第 2 行后插入两行（新 2..4），原第 6 行改成两行（旧 6..7，新 8..10）。
    let hunks = [
        LineHunk {
            old: 2..2,
            new: 2..4,
        },
        LineHunk {
            old: 6..7,
            new: 8..10,
        },
    ];
    assert_eq!(head_line(&hunks, 1), 1);
    assert_eq!(head_line(&hunks, 2), 1);
    assert_eq!(head_line(&hunks, 3), 1);
    assert_eq!(head_line(&hunks, 5), 3);
    assert_eq!(head_line(&hunks, 9), 6);
    assert_eq!(head_line(&hunks, 10), 7);
}

#[test]
fn changed_files_group_by_directory_in_display_order() {
    use winc_editor::git::Change;
    let root = Path::new("/r");
    let file = |path: &str| CommitFile {
        path: root.join(path),
        from: None,
        change: Change::Modified,
        lines: Some((1, 0)),
    };
    let files = [
        file("b/x.rs"),
        file("a.rs"),
        file("b/c/y.rs"),
        file("b/z.rs"),
    ];
    let rows = log_file_rows(Some(root), &files, (false, false));
    use details::{
        LogFileRow::{Dir, File},
        log_file_rows,
    };
    assert_eq!(
        rows,
        [
            File(1),
            Dir("b".into()),
            File(0),
            File(3),
            Dir("b/c".into()),
            File(2),
        ]
    );
    let flat = log_file_rows(Some(root), &files, (true, false));
    assert_eq!(flat, [File(0), File(1), File(2), File(3)]);
    // 文件历史查看的文件（第一个）所在的组排在最前，它在组里也最前。
    let pinned = log_file_rows(Some(root), &files, (false, true));
    assert_eq!(pinned[..3], [Dir("b".into()), File(0), File(3)]);
}

#[test]
fn short_cuts_only_commit_hashes() {
    assert_eq!(short("3c1848e5f0a2"), "3c1848e");
    assert_eq!(short("abc"), "abc");
    // 分支名不截短：名字里有汉字也不会截在一个字的中间。
    assert_eq!(short("feature/login"), "feature/login");
    assert_eq!(short("功能/登录"), "功能/登录");
}
