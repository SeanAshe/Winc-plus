//! `crash` 的测试：日志内容的格式、信息长度上限，以及日志过大时的轮换。

use super::*;

#[test]
fn entries_name_the_version_thread_and_location() {
    let text = entry(42, "main", "src/a.rs:1:2", "boom");
    assert!(text.starts_with("time: 42\nversion: "), "{text}");
    assert!(text.contains("thread: main\nlocation: src/a.rs:1:2\nmessage: boom\n"));
    assert!(text.contains(winc_version::VERSION));
}

#[test]
fn long_messages_are_cut_on_a_character() {
    let text = entry(0, "main", "a", &"中".repeat(MAX_MESSAGE + 10));
    assert_eq!(text.matches('中').count(), MAX_MESSAGE);
}

#[test]
fn entries_are_appended_and_a_full_log_moves_aside() {
    let dir = std::env::temp_dir().join(format!("winc-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("nested").join("crash.log");

    append(&path, "one\n").unwrap();
    append(&path, "two\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");

    let full = "x".repeat(MAX_BYTES as usize + 1);
    std::fs::write(&path, &full).unwrap();
    append(&path, "three\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "three\n");
    assert_eq!(
        std::fs::read_to_string(path.with_extension("log.old")).unwrap(),
        full
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
