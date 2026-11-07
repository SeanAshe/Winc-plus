//! 冲突识别的测试：两种冲突写法、标记与名字的解析、换行符保留，以及伪标记与不完整标记不算冲突。
//!
//! 跨块与按文件检查标记的样本用重复字符填充，使标记落在 rope 分块和 64 KiB 读取块的边界上。

use super::*;

const TEXT: &str = "keep\n\
<<<<<<< HEAD\n\
ours 1\n\
ours 2\n\
=======\n\
theirs\n\
>>>>>>> feature/x\n\
middle\n\
<<<<<<< HEAD\n\
a\n\
||||||| base\n\
b\n\
=======\n\
c\n\
>>>>>>> feature/x\n";

#[test]
fn finds_both_styles_of_conflict() {
    let rope = Rope::from_str(TEXT);
    let found = find_conflicts(&rope);
    assert_eq!(found.len(), 2);
    let first = &found[0];
    assert_eq!((first.start, first.middle, first.end), (1, 4, 6));
    assert_eq!(first.base, None);
    assert_eq!(first.ours(), 2..4);
    assert_eq!(first.theirs(), 5..6);
    assert_eq!(first.ours_label, "HEAD");
    assert_eq!(first.theirs_label, "feature/x");
    let second = &found[1];
    assert_eq!(second.base, Some(10));
    assert_eq!(second.ours(), 9..10);
    assert_eq!(second.middle, 12);
    assert_eq!(second.theirs(), 13..14);
    assert!(second.is_marker(10) && !second.is_marker(11));
}

#[test]
fn resolutions_keep_line_endings() {
    let rope = Rope::from_str(TEXT);
    let first = &find_conflicts(&rope)[0];
    assert_eq!(resolution(&rope, first, Pick::Ours), "ours 1\nours 2\n");
    assert_eq!(resolution(&rope, first, Pick::Theirs), "theirs\n");
    assert_eq!(
        resolution(&rope, first, Pick::Both),
        "ours 1\nours 2\ntheirs\n"
    );
    let crlf = Rope::from_str("<<<<<<< a\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\n");
    let c = &find_conflicts(&crlf)[0];
    assert_eq!(resolution(&crlf, c, Pick::Both), "x\r\ny\r\n");
}

#[test]
fn lookalikes_and_unfinished_markers_are_not_conflicts() {
    let rope = Rope::from_str("<<<<<<<< eight\n=======x\n<<<<<<< a\nx\n=======\ny\n");
    assert!(find_conflicts(&rope).is_empty());
    // 名字前后的空白去掉；`>>>>>>>` 后面不是空格的不算标记。
    let rope = Rope::from_str("<<<<<<<  HEAD \nx\n=======\ny\n>>>>>>>b\n>>>>>>>\n");
    let found = find_conflicts(&rope);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].ours_label.as_str(), found[0].end), ("HEAD", 5));
    assert_eq!(found[0].theirs_label, "");
}

#[test]
fn markers_are_found_wherever_the_rope_splits_them() {
    // 先在各块上找 `<<<<<<<`：标记落在块的边界上（rope 一块一千字节上下）时也要找到。
    let conflict = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n";
    for pad in (0..2100).step_by(7) {
        let text = format!("{}\n{conflict}", "x".repeat(pad));
        let found = find_conflicts(&Rope::from_str(&text));
        assert_eq!(found.len(), 1, "pad {pad}");
        assert_eq!((found[0].start, found[0].end), (1, 5));
    }
    // 小块拼起来的 rope：`insert` 会把小块并成一块，用 `_append_chunk` 让标记跨块。
    let mut builder = ropey::RopeBuilder::new();
    for piece in [
        "k\n<<",
        "<<",
        "<<< o",
        "urs\n",
        "a\n===",
        "====\nb\n>>>",
        ">>>> t\n",
    ] {
        builder._append_chunk(piece);
    }
    let rope = builder.finish();
    assert!(rope.chunks().count() > 1);
    assert_eq!(find_conflicts(&rope).len(), 1);
    assert!(find_conflicts(&Rope::from_str(&"<<<<<< no\n".repeat(500))).is_empty());
}

/// 回归：检查提交的文件时，整个文件先读进内存，再按有损解码复制并建 rope，同时占约三倍文件
/// 大小的内存。
///
/// 先按块找 `<<<<<<<`，没有就不建 rope；跨块的标记同样找得到。
#[test]
fn files_are_checked_for_conflicts_block_by_block() {
    let dir = crate::test_util::scratch("merge-file");
    let path = dir.join("big.txt");
    // 标记横跨 64 KiB 的读取块边界。
    let pad = "x".repeat((64 << 10) - 3);
    let conflict = "\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> b\n";
    std::fs::write(&path, format!("{pad}{conflict}")).unwrap();
    assert!(file_has_conflicts(&path));
    // 只有像标记的文字、不成对：不算。
    std::fs::write(&path, format!("{pad}\n<<<<<<< HEAD\nonly\n")).unwrap();
    assert!(!file_has_conflicts(&path));
    std::fs::write(&path, "no markers\n").unwrap();
    assert!(!file_has_conflicts(&path));
    // 不是 UTF-8 的文件按有损解码检查。
    std::fs::write(&path, b"\xff\n<<<<<<< a\n1\n=======\n2\n>>>>>>> b\n").unwrap();
    assert!(file_has_conflicts(&path));
    assert!(!file_has_conflicts(&dir.join("missing")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文件里有一段冲突，上方多出一行 `git` 从对方自动合入的内容。
#[test]
fn sides_align_with_the_conflicted_file() {
    let text = Rope::from_str(
        "auto\nkeep\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> b\nend\n",
    );
    let conflicts = find_conflicts(&text);
    // 当前一侧没有 `auto`：合入的这一行在当前一侧没有对应。
    let ours = SideAlign::new(&text, &conflicts, "keep\nours 1\nours 2\nend\n", Pick::Ours);
    assert_eq!(ours.blocks.first(), Some(&(1..3)));
    assert_eq!(ours.side_line(1), 0);
    // 冲突内的行对应这一侧冲突的开头，冲突之后的行跟在这一侧的内容后面。
    assert_eq!(ours.side_line(4), 1);
    assert_eq!(ours.side_line(8), 3);

    let theirs = SideAlign::new(&text, &conflicts, "auto\nkeep\ntheirs\nend\n", Pick::Theirs);
    assert_eq!(theirs.blocks.first(), Some(&(2..3)));
    assert_eq!(theirs.side_line(0), 0);
    assert_eq!(theirs.side_line(8), 3);
    assert!(theirs.segments().iter().all(|s| s.conflict.is_some()));
}

#[test]
fn lines_changed_on_one_side_only_are_reported() {
    let text = Rope::from_str("a\n<<<<<<< H\nx\n=======\ny\n>>>>>>> b\nC\n");
    let conflicts = find_conflicts(&text);
    let ours = SideAlign::new(&text, &conflicts, "a\nx\nc\n", Pick::Ours);
    let merged = ours.segments().iter().find(|s| s.conflict.is_none());
    assert_eq!(merged.map(|s| s.side.clone()), Some(2..3));
}

#[test]
fn a_file_without_conflicts_aligns_line_by_line() {
    let text = Rope::from_str("a\nb\n");
    let align = SideAlign::new(&text, &[], "a\nb\n", Pick::Ours);
    assert!(align.blocks.is_empty());
    assert_eq!(align.side_line(1), 1);
}

#[test]
fn segments_connect_conflicts_and_merged_changes_to_the_file() {
    let text = Rope::from_str("a\n<<<<<<< H\nx\n=======\ny\n>>>>>>> b\nm\nC\n");
    let conflicts = find_conflicts(&text);
    let ours = SideAlign::new(&text, &conflicts, "a\nx\nm\nc\n", Pick::Ours);
    let segments = ours.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].conflict, Some(0));
    assert_eq!(
        (segments[0].file.clone(), segments[0].side.clone()),
        (1..6, 1..2)
    );
    // 冲突之后的行号按文件计算：解决后的第 3 行是文件的第 7 行。
    assert_eq!(segments[1].conflict, None);
    assert_eq!(
        (segments[1].file.clone(), segments[1].side.clone()),
        (7..8, 3..4)
    );
}
