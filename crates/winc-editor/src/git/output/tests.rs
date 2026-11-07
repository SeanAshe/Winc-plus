//! `output` 的测试：命令的排版、输出的截断和地址中凭据的替换。

use std::path::Path;

use super::*;

#[test]
fn an_entry_lists_the_repository_command_output_and_exit_code() {
    let text = entry(Path::new("/repo"), &["push"], Some(1), "fatal: no\n");
    let status = winc_i18n::t!("git-output-exit-code", code = "1");
    assert_eq!(text, format!("/repo\n$ git push\nfatal: no\n{status}\n"));
}

#[test]
fn an_entry_without_output_or_exit_code_stays_readable() {
    let text = entry(Path::new("/repo"), &["fetch"], None, "");
    let status = winc_i18n::t!("git-output-no-exit-code");
    assert_eq!(text, format!("/repo\n$ git fetch\n{status}\n"));
}

#[test]
fn credentials_in_urls_are_replaced() {
    assert_eq!(
        redact("fatal: https://me:secret@host/a.git and http://token@h/b"),
        "fatal: https://***@host/a.git and http://***@h/b",
    );
}

#[test]
fn addresses_without_credentials_stay_unchanged() {
    for text in ["git@host:a/b.git", "https://host/a@b", "ssh://host", "a://"] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn a_command_line_is_redacted_too() {
    let text = entry(Path::new("/r"), &["clone", "https://a:b@h/c"], Some(0), "");
    assert!(text.contains("$ git clone https://***@h/c\n"), "{text}");
}

#[test]
fn long_output_keeps_its_end_on_a_character_boundary() {
    assert_eq!(tail("甲乙丙", 7), "乙丙");
    assert_eq!(tail("甲乙丙", 100), "甲乙丙");
}

/// 回归：截断点落在地址中间时，开头剩下的凭据没有 `://`，必须在截断之前替换。
#[test]
fn truncation_never_exposes_credentials() {
    let url = "https://me:secret@host/a.git\n";
    let cut = url.find("secret").unwrap() + 2;
    let output = format!("{url}{}", "x".repeat(TEXT_LIMIT - (url.len() - cut)));
    let text = entry(Path::new("/r"), &["push"], Some(1), &output);
    assert!(!text.contains("cret"), "{}", &text[..80]);
}
