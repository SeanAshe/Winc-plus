//! 更新日志的测试：按版本取舍、格式不符的内容，以及图片和 HTML 的失效。

use super::*;

/// 三个版本的更新日志，从新到旧。
const NOTES_TEXT: &str = "## 0.3.0 · 2026-03-01\n\n### 新功能\n\n- 丙\n\n\
## 0.2.0 · 2026-02-01\n\n- 乙（[#7](https://example.com/7)）\n\n\
## 0.1.0 · 2026-01-01\n\n- 甲\n";

/// 返回 `text` 中各版本标题里的版本号。
fn versions(text: &str) -> Vec<&str> {
    text.lines().filter_map(version_of).collect()
}

#[test]
fn releases_are_chosen_by_the_versions_run_before_and_now() {
    let chosen =
        |scope: Scope, current: &str| select(NOTES_TEXT, &scope, current, "（暂缺）").unwrap();
    assert_eq!(
        versions(&chosen(Scope::All, "0.2.0")),
        ["0.3.0", "0.2.0", "0.1.0"]
    );
    // 从 0.1.0 更新到 0.3.0：中间跳过的版本一并列出，上次运行的版本不列出。
    let since = |previous: &str| Scope::Since(Some(previous.to_owned()));
    assert_eq!(
        versions(&chosen(since("0.1.0"), "0.3.0")),
        ["0.3.0", "0.2.0"]
    );
    // 比当前版本新的不列出。
    assert_eq!(versions(&chosen(since("0.1.0"), "0.2.0")), ["0.2.0"]);
    // 上次运行的版本没有记录：只有当前版本。
    assert_eq!(versions(&chosen(Scope::Since(None), "0.2.0")), ["0.2.0"]);
    // 日志尚未包含当前版本：显示全部，最前面说明这一点；其余情况没有这段说明。
    let missing = chosen(since("0.3.0"), "0.4.0");
    assert_eq!(versions(&missing).len(), 3);
    assert!(missing.starts_with("（暂缺）\n\n## 0.3.0"));
    assert!(!chosen(since("0.1.0"), "0.3.0").contains("（暂缺）"));
    assert!(!chosen(Scope::All, "0.4.0").contains("（暂缺）"));
    // 各版的正文原样保留。
    let all = chosen(Scope::All, "0.3.0");
    assert!(all.contains("### 新功能\n\n- 丙\n") && all.contains("[#7](https://example.com/7)"));
}

#[test]
fn text_that_is_not_a_changelog_is_refused() {
    let refused = |text: &str| select(text, &Scope::All, "0.1.0", "").is_none();
    // 登录页、错误页之类的应答。
    assert!(refused("<!doctype html>\n<html></html>\n"));
    assert!(refused("Not Found\n## 0.1.0 · 2026-01-01\n"));
    assert!(refused(""));
    assert!(refused("## 更新日志\n\n- 甲\n"));
    // 超过长度上限。
    let long = format!("## 0.1.0 · 2026-01-01\n{}", "- 甲\n".repeat(MAX_LENGTH / 4));
    assert!(refused(&long));
    // 标题之前的空行不影响。
    assert!(!refused("\n\n## 0.1.0 · 2026-01-01\n- 甲\n"));
}

#[test]
fn images_and_html_are_made_inert_outside_code() {
    let made = |text: &str| inert(text);
    assert_eq!(
        made("- ![图](https://a.example/x.png)\n"),
        "- \\![图](https://a.example/x.png)\n"
    );
    assert_eq!(
        made("<img src=\"https://a.example/x\">\n"),
        "&lt;img src=\"https://a.example/x\">\n"
    );
    // 行内代码和围栏代码块中的内容不变。
    assert_eq!(
        made("输入 `- [ ` 和 `<kbd>`！\n"),
        "输入 `- [ ` 和 `<kbd>`！\n"
    );
    assert_eq!(made("``a ` <b>`` <i>\n"), "``a ` <b>`` &lt;i>\n");
    assert_eq!(
        made("```\n<b> ![x](y)\n```\n<b>\n"),
        "```\n<b> ![x](y)\n```\n&lt;b>\n"
    );
    // 没有结束的反引号是普通文字，其后的内容照常处理。
    assert_eq!(made("a ` <b>\n"), "a ` &lt;b>\n");
    // 链接、加粗和感叹号不受影响。
    let plain = "- **加粗**、[链接](https://winc.si)，完成！\n";
    assert_eq!(made(plain), plain);
}

#[test]
fn each_language_has_its_own_changelog_and_page() {
    assert_eq!(url(Locale::ZhCn), "https://winc.si/releases/notes/zh-cn.md");
    assert_eq!(url(Locale::EnUs), "https://winc.si/releases/notes/en.md");
    assert!(page(Locale::ZhCn).contains("/zh-cn/"));
    assert!(!page(Locale::EnUs).contains("/zh-cn/"));
}
