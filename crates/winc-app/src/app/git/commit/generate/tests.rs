//! 生成提交信息的测试：篇幅的分配、补丁的写出、未跟踪文件的读取、从回答中取出提交信息、发送
//! 给模型的内容。

use super::*;

/// 小段完整保留，用不完的篇幅分给大段；总和不超过篇幅。
#[test]
fn shares_keep_small_files_whole_and_split_the_rest() {
    assert_eq!(shares(&[100, 50_000, 300], 1_000), [100, 600, 300]);
    assert_eq!(shares(&[400, 400, 400], 900), [300, 300, 300]);
    assert_eq!(shares(&[10, 20], 1_000), [10, 20]);
    assert!(shares(&[], 1_000).is_empty());
    let many = shares(&[FILE_LIMIT; MAX_FILES], PATCH_LIMIT);
    assert_eq!(many.iter().sum::<usize>(), PATCH_LIMIT);
}

/// 各文件写到分得的篇幅为止，省略的字节数写在其后；首行总是写出；未读完的文件数写在末尾。
#[test]
fn every_file_keeps_its_header_and_a_fair_part() {
    let big: String = (0..2000).map(|i| format!("+line {i}\n")).collect();
    let file = |name: &str, body: &str| FilePatch {
        text: format!("diff --git a/{name} b/{name}\n{body}"),
        size: 20 + body.len(),
    };
    let mut big_file = file("a.rs", &big[..FILE_LIMIT]);
    big_file.size = 20 + big.len();
    let patch = PendingPatch {
        stat: " a.rs | 2000 +\n b.rs |    1 +\n 2 files changed\n\n".into(),
        files: vec![big_file, file("b.rs", "+b\n")],
        partial: true,
    };
    let text = render(&[(None, patch)]);
    assert!(text.starts_with(" a.rs | 2000 +\n b.rs |    1 +\n 2 files changed\n\ndiff --git"));
    assert!(
        text.contains("more bytes omitted]\ndiff --git a/b.rs b/b.rs\n+b\n"),
        "{text}"
    );
    assert!(text.ends_with("[Patches of the remaining files omitted]\n"));

    // 文件很多时，每个文件都保留首行和一部分补丁，合计不超过篇幅。
    let files: Vec<FilePatch> = (0..MAX_FILES)
        .map(|i| {
            let mut f = file(&format!("{i}.rs"), &big[..FILE_LIMIT]);
            f.size = 20 + big.len();
            f
        })
        .collect();
    let patch = PendingPatch {
        files,
        ..PendingPatch::default()
    };
    let text = render(&[(Some("repo".into()), patch)]);
    assert!(text.starts_with("Repository repo:\ndiff --git"));
    assert_eq!(text.matches("diff --git ").count(), MAX_FILES);
    assert_eq!(text.matches("+line 0\n").count(), MAX_FILES);
    assert!(text.len() <= PATCH_LIMIT + MAX_FILES * 40, "{}", text.len());
}

/// 未跟踪的文件只读开头，最后一行不完整时不保留；二进制文件只写一行说明。
#[test]
fn new_files_read_only_their_start() {
    let dir = std::env::temp_dir().join(format!("winc-generate-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big: String = (0..4000).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("big.txt"), &big).unwrap();
    std::fs::write(dir.join("data.bin"), [0u8, 1, 2, 0, 3]).unwrap();
    let patch = new_file(&dir, &dir.join("big.txt"));
    assert!(
        patch
            .text
            .starts_with("diff --git a/big.txt b/big.txt\nnew file\n--- /dev/null\n")
    );
    assert!(patch.text.len() <= FILE_LIMIT + 4096 && patch.text.ends_with('\n'));
    assert!(patch.size >= big.len(), "{}", patch.size);
    let binary = new_file(&dir, &dir.join("data.bin"));
    assert!(binary.text.ends_with("Binary file\n") && binary.size == binary.text.len());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_reply_loses_code_fences_and_trailing_spaces() {
    assert_eq!(message_from("  fix: a  \n\nbody  \n"), "fix: a\n\nbody");
    assert_eq!(
        message_from("```text\nfix: a\n\nbody\n```"),
        "fix: a\n\nbody"
    );
    assert_eq!(message_from("```\nfix: a\n```\n"), "fix: a");
    assert_eq!(message_from("```"), "");
}

#[test]
fn the_prompt_lists_branch_subjects_and_changes() {
    let subjects = ["feat: b".to_owned(), "fix: a".to_owned()];
    let input = prompt(Some("main"), &subjects, "diff --git a/x b/x\n");
    assert_eq!(
        input,
        "Branch: main\n\nRecent commit subjects:\n- feat: b\n- fix: a\n\n\
         Changes to commit:\ndiff --git a/x b/x\n"
    );
    assert_eq!(prompt(None, &[], "d"), "Changes to commit:\nd");
    assert!(!instructions().contains("{language}"));
}
