//! 改动日志的测试：未变的行数和合并后的字节范围。

use super::*;

/// 把 `text` 的 `range` 换成 `insert`，返回改动后的文本和改动。
fn replace(text: &str, range: Range<usize>, insert: &str) -> (String, ByteEdit) {
    let changed = format!("{}{insert}{}", &text[..range.start], &text[range.end..]);
    let new = range.start..range.start + insert.len();
    (changed, ByteEdit { old: range, new })
}

#[test]
fn merged_edits_cover_every_edit_in_between() {
    // 依次改动后，把合并的范围套回最初的文本，应得到同样的结果。
    let cases: [&[(Range<usize>, &str)]; 6] = [
        // 连续输入。
        &[(3..3, "a"), (4..4, "b"), (5..5, "c")],
        // 先输入再退格。
        &[(3..3, "ab"), (4..5, "")],
        // 后一处在前一处之前、之后。
        &[(6..8, "xyz"), (1..2, "")],
        &[(1..1, "q"), (8..9, "rr")],
        // 后一处跨过前一处新写入的内容。
        &[(2..4, "abcd"), (4..9, "Z")],
        // 删除后在原处输入。
        &[(2..5, ""), (2..2, "new")],
    ];
    for edits in cases {
        let start = "0123456789".to_owned();
        let (mut text, mut merged) = (start.clone(), None::<ByteEdit>);
        for (range, insert) in edits {
            let (changed, edit) = replace(&text, range.clone(), insert);
            text = changed;
            merged = Some(match merged {
                Some(merged) => merged.then(&edit).unwrap(),
                None => edit,
            });
        }
        let merged = merged.unwrap();
        let rebuilt = format!(
            "{}{}{}",
            &start[..merged.old.start],
            &text[merged.new.clone()],
            &start[merged.old.end..]
        );
        assert_eq!(rebuilt, text, "{edits:?} → {merged:?}");
    }
}

#[test]
fn a_gap_in_the_log_means_nothing_is_known() {
    let mut log = EditLog::default();
    log.push(1, 4, 9);
    log.set_bytes(
        1,
        ByteEdit {
            old: 2..2,
            new: 2..3,
        },
    );
    log.push(2, 2, 3);
    log.set_bytes(
        2,
        ByteEdit {
            old: 3..3,
            new: 3..4,
        },
    );
    assert_eq!(log.unchanged_since(0, 2), (2, 3));
    assert_eq!(
        log.bytes_since(0, 2),
        Some(ByteEdit {
            old: 2..2,
            new: 2..4
        })
    );
    assert_eq!(log.unchanged_since(2, 2), (usize::MAX, usize::MAX));
    assert_eq!(log.bytes_since(2, 2), None);
    // 版本 3 不在日志中（例如从磁盘重新读取）。
    assert_eq!(log.unchanged_since(0, 3), (0, 0));
    assert_eq!(log.bytes_since(0, 3), None);
    // 有多处改动的版本没有字节范围。
    log.push(3, 0, 0);
    assert_eq!(log.unchanged_since(0, 3), (0, 0));
    assert_eq!(log.bytes_since(0, 3), None);
    assert_eq!(log.bytes_since(3, 3), None);
}
