//! 整理提示词的测试：代码块的围栏、跳过的文件、符号链接、旧编码和未保存的内容。

use super::*;

/// 返回本测试独用的空目录。
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-prompt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn file(dir: &Path, name: &str) -> PromptFile {
    PromptFile {
        path: dir.join(name),
        name: name.to_owned(),
        unsaved: None,
    }
}

/// 文件中有代码块时外层的围栏更长；名字中的换行不打断 `File:` 一行。
#[test]
fn the_fence_outgrows_fences_inside_the_file() {
    assert_eq!(
        file_block("notes\nexample.md", "md", "```rs\nfn main() {}\n```"),
        "File: notes\\nexample.md\n````md\n```rs\nfn main() {}\n```\n````\n\n"
    );
    assert_eq!(file_block("README", "", ""), "File: README\n```\n```\n\n");
}

/// 二进制文件、无法无损解码的文件和超出上限的文件跳过，其余按顺序写入。
#[test]
fn unusable_files_are_skipped_and_counted() {
    let dir = scratch("skip");
    std::fs::write(dir.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.join("image.png"), b"\x89PNG\x00\x00data").unwrap();
    std::fs::write(dir.join("big.txt"), "x".repeat(MAX_PROMPT_BYTES + 1)).unwrap();
    let names = ["a.rs", "image.png", "big.txt", "missing.txt"];
    let out = prompt(names.iter().map(|name| file(&dir, name)).collect());
    assert_eq!(out.text, "File: a.rs\n```rs\nfn a() {}\n```\n\n");
    assert_eq!(out.skipped, 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 文件树把指向文件的符号链接列为文件：读取链接指向的内容。
#[cfg(unix)]
#[test]
fn symlinks_to_files_are_read() {
    let dir = scratch("link");
    std::fs::write(dir.join("AGENTS.md"), "# Agents\n").unwrap();
    std::os::unix::fs::symlink(dir.join("AGENTS.md"), dir.join("CLAUDE.md")).unwrap();
    let out = prompt(vec![file(&dir, "CLAUDE.md")]);
    assert_eq!(out.text, "File: CLAUDE.md\n```md\n# Agents\n```\n\n");
    assert_eq!(out.skipped, 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 按打开文件的规则识别编码：GBK 的文件解码为文字，与编辑器中显示的相同。
#[test]
fn legacy_encodings_are_decoded_like_opened_files() {
    let dir = scratch("gbk");
    // 「中文注释」的 GBK 编码。
    let gbk = [0xD6, 0xD0, 0xCE, 0xC4, 0xD7, 0xA2, 0xCA, 0xCD];
    std::fs::write(dir.join("a.c"), [b"// ".as_slice(), &gbk, b"\n"].concat()).unwrap();
    let out = prompt(vec![file(&dir, "a.c")]);
    assert_eq!(out.text, "File: a.c\n```c\n// 中文注释\n```\n\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 有未保存修改的文件用编辑器中的内容，不读磁盘。
#[test]
fn unsaved_text_is_used_instead_of_the_file() {
    let dir = scratch("unsaved");
    std::fs::write(dir.join("a.rs"), "old\n").unwrap();
    let mut a = file(&dir, "a.rs");
    a.unsaved = Some(Rope::from_str("new\n"));
    assert_eq!(prompt(vec![a]).text, "File: a.rs\n```rs\nnew\n```\n\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 累计超过上限时，之后放不下的文件跳过，已写入的保留。
#[test]
fn files_past_the_total_budget_are_skipped() {
    let dir = scratch("budget");
    let half = "x".repeat(MAX_PROMPT_BYTES / 2);
    for name in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(name), &half).unwrap();
    }
    let out = prompt(vec![file(&dir, "a.txt"), file(&dir, "b.txt")]);
    assert!(out.text.starts_with("File: a.txt\n"));
    assert!(out.text.len() <= MAX_PROMPT_BYTES);
    assert_eq!(out.skipped, 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
