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

/// 边运行边记录：命令一开始就写下仓库和命令两行；输出按整行写入，没凑成一行的部分等后面的
/// 输出；结束时补上剩下的输出和退出码。记录是全局的，别的测试同时在写，这里只按包含判断。
#[test]
fn a_running_command_is_recorded_line_by_line() {
    let root = Path::new("/live-record-test");
    let live = begin(root, &["push", "https://me:secret@host/a.git"]);
    let header = "/live-record-test\n$ git push https://***@host/a.git\n";
    assert!(output_text().contains(header));

    // 半行不写入；凑成整行后一起写入，其中的凭据已经替换。
    feed(&live, b"remote: live-record https://me:se");
    assert!(!output_text().contains("remote: live-record"));
    feed(&live, b"cret@host/a.git\nlive-record-tail");
    assert!(output_text().contains("remote: live-record https://***@host/a.git\n"));
    assert!(!output_text().contains("live-record-tail"));

    // 结束：剩下的半行和退出码一次写入。
    finish(&live, Some(0), b"ignored when lines were streamed");
    let status = winc_i18n::t!("git-output-exit-code", code = "0");
    assert!(output_text().contains(&format!("live-record-tail\n{status}\n")));
    assert!(!output_text().contains("ignored when lines were streamed"));
    assert!(!output_text().contains("me:secret"));
}

/// 从某个位置以后写入的内容可以单独取回，接在已经显示的文本后面；位置为 0 时取不到，应当
/// 换上全文。
#[test]
fn what_was_written_after_a_position_can_be_fetched_alone() {
    record(Path::new("/since-test"), &["status"], Some(0), b"");
    let (_, at) = output_snapshot();
    record(Path::new("/since-test"), &["since-marker"], Some(0), b"");
    let (more, written) = output_since(at).unwrap();
    assert!(more.contains("$ git since-marker\n"));
    assert!(written > at);
    assert!(output_since(0).is_none());
}

/// 运行途中一点输出都没有取到（在远端运行）时，结束时写全部输出的末尾。记录是全局的，
/// 别的测试同时在写，命令行与输出之间可能夹着别的记录，这里分别按包含判断。
#[test]
fn a_command_without_live_output_records_it_at_the_end() {
    let live = begin(Path::new("/late-record-test"), &["fetch"]);
    assert!(output_text().contains("/late-record-test\n$ git fetch\n"));
    assert!(!output_text().contains("late-record-output"));
    finish(&live, Some(1), b"fatal: late-record-output\n");
    let status = winc_i18n::t!("git-output-exit-code", code = "1");
    let expected = format!("fatal: late-record-output\n{status}\n");
    assert!(output_text().contains(&expected), "{}", output_text());
}

/// 超出上限时从开头丢弃整行：超出的位置落在汉字中间也不出错，留下的从下一行开始。
#[test]
fn trimming_the_front_keeps_whole_lines_of_any_text() {
    let mut text = "提交说明\n第二行\n末行\n".to_owned();
    let full = text.len();
    // 超出 1 字节：位置落在「提」的第二个字节上。
    let cut = trim_front(&mut text, full - 1);
    assert_eq!(text, "第二行\n末行\n");
    assert_eq!(cut, "提交说明\n".len());
    assert_eq!(trim_front(&mut text, full), 0);
    // 最后一行也放不下时全部丢弃。
    let mut text = "没有换行的一整段".to_owned();
    assert_eq!(trim_front(&mut text, 3), "没有换行的一整段".len());
    assert!(text.is_empty());
}
