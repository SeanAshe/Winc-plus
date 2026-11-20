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
fn changed_files_nest_like_the_file_tree() {
    use details::{LogFileRow, RowsShape, log_file_rows};
    use std::collections::HashSet;
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
        file("b/c/d/y.rs"),
        file("b/z.rs"),
        file("e/f/w.rs"),
    ];
    let shape = RowsShape {
        flat: false,
        pinned: false,
        compact: true,
    };
    let dir = |dir: &str, label: &str, depth, open| LogFileRow::Dir {
        dir: dir.into(),
        label: label.into(),
        depth,
        open,
    };
    let file = |index, depth| LogFileRow::File { index, depth };
    let none = HashSet::new();
    let sep = std::path::MAIN_SEPARATOR_STR;
    let cd = format!("c{sep}d");
    let bcd = format!("b{sep}c{sep}d");
    let ef = format!("e{sep}f");
    // 先目录、后文件，各按名字排；只有一个子目录的几级合成一行。
    assert_eq!(
        log_file_rows(Some(root), &files, shape, &none),
        [
            dir("b", "b", 0, true),
            dir(&bcd, &cd, 1, true),
            file(2, 2),
            file(0, 1),
            file(3, 1),
            dir(&ef, &ef, 0, true),
            file(4, 1),
            file(1, 0),
        ]
    );
    // 收起的目录里的文件不列出。
    let folded = HashSet::from(["b".to_owned()]);
    let rows = log_file_rows(Some(root), &files, shape, &folded);
    assert_eq!(rows[..2], [dir("b", "b", 0, false), dir(&ef, &ef, 0, true)]);
    // 不合并时每级一行。
    let loose = RowsShape {
        compact: false,
        ..shape
    };
    let rows = log_file_rows(Some(root), &files, loose, &none);
    assert_eq!(rows[1], dir(&format!("b{sep}c"), "c", 1, true));
    let flat = RowsShape {
        flat: true,
        ..shape
    };
    let order: Vec<_> = log_file_rows(Some(root), &files, flat, &none)
        .iter()
        .filter_map(LogFileRow::file)
        .collect();
    assert_eq!(order, [0, 1, 2, 3, 4]);
    // 文件历史查看的文件（第一个）和装着它的目录排在各自那一层的最前。
    let pinned = RowsShape {
        pinned: true,
        ..shape
    };
    let viewed = [files[4].clone(), files[0].clone()];
    let rows = log_file_rows(Some(root), &viewed, pinned, &none);
    let first = [dir(&ef, &ef, 0, true), file(0, 1), dir("b", "b", 0, true)];
    assert_eq!(rows[..3], first);
}

#[test]
fn short_cuts_only_commit_hashes() {
    assert_eq!(short("3c1848e5f0a2"), "3c1848e");
    assert_eq!(short("abc"), "abc");
    // 分支名不截短：名字里有汉字也不会截在一个字的中间。
    assert_eq!(short("feature/login"), "feature/login");
    assert_eq!(short("功能/登录"), "功能/登录");
}
