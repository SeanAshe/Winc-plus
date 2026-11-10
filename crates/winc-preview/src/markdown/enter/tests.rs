//! `enter` 的测试：回车拆分段落、续写和结束列表与引用、闭合围栏，退格逐个删除标记，Tab
//! 嵌套列表项。

use super::*;

/// 在 `text` 的光标处（用 `|` 标出）按回车，返回改完的文字（光标仍用 `|` 标出）。
fn press(text: &str) -> String {
    let at = text.find('|').unwrap();
    let text = text.replace('|', "");
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let next = text.get(end + 1..).and_then(|t| t.split('\n').next());
    let above = text[..start.saturating_sub(1)].rsplit('\n');
    let above: Vec<&str> = if start == 0 {
        Vec::new()
    } else {
        above.collect()
    };
    let edit = enter(&text[start..at], &text[at..end], next, above.into_iter());
    let from = at - edit.before;
    let to = at + edit.after;
    format!("{}{}|{}", &text[..from], edit.text, &text[to..])
}

#[test]
fn paragraphs_split() {
    assert_eq!(press("一段|文字"), "一段\n\n|文字");
    assert_eq!(press("# 标题|"), "# 标题\n\n|");
    // 光标位于多行段落的行尾：下一行成为新的一段。
    assert_eq!(press("第一行|\n第二行"), "第一行\n\n|第二行");
    assert_eq!(press("a\n|\nb"), "a\n\n|\nb");
}

#[test]
fn lists_continue_and_end() {
    assert_eq!(press("- 一|"), "- 一\n- |");
    assert_eq!(press("* 一|二"), "* 一\n* |二");
    assert_eq!(press("9. 九|"), "9. 九\n10. |");
    assert_eq!(press("1) 一|"), "1) 一\n2) |");
    assert_eq!(press("- [x] 做完|"), "- [x] 做完\n- [ ] |");
    // 空项：结束列表，另起一段。
    assert_eq!(press("- 一\n- |"), "- 一\n\n|");
    assert_eq!(press("- 一\n- [ ] |"), "- 一\n\n|");
    // 空的嵌套项：退到上一层，延续上一层的序号。
    assert_eq!(press("1. 一\n   - 甲\n   - |"), "1. 一\n   - 甲\n2. |");
    assert_eq!(
        press("- 一\n  - 甲\n    - 子\n    - |"),
        "- 一\n  - 甲\n    - 子\n  - |"
    );
    // 分隔线、粗体开头不是列表。
    assert_eq!(press("**粗**|"), "**粗**\n\n|");
}

#[test]
fn quotes_continue_and_end() {
    assert_eq!(press("> 引用|"), "> 引用\n>\n> |");
    assert_eq!(press("> 引用\n> |"), "> 引用\n\n|");
    assert_eq!(press("> > 两层\n> > |"), "> > 两层\n>\n> |");
    assert_eq!(press("> - 一|"), "> - 一\n> - |");
    assert_eq!(press("> - 一\n> - |"), "> - 一\n>\n> |");
}

/// 在 `before` 末尾按删除键，返回修改后的行；按普通删除处理时返回 `None`。
fn delete(before: &str) -> Option<String> {
    let edit = backspace(before)?;
    Some(format!(
        "{}{}",
        &before[..before.len() - edit.before],
        edit.text
    ))
}

#[test]
fn backspace_removes_one_marker() {
    assert_eq!(delete("- [ ] ").as_deref(), Some("- "));
    assert_eq!(delete("- [x]").as_deref(), Some("- "));
    assert_eq!(delete("- ").as_deref(), Some("  "));
    assert_eq!(delete("  10. ").as_deref(), Some("      "));
    assert_eq!(delete("> - ").as_deref(), Some(">   "));
    assert_eq!(delete("> ").as_deref(), Some(""));
    assert_eq!(delete("> > ").as_deref(), Some("> "));
    assert_eq!(delete("## ").as_deref(), Some(""));
    assert_eq!(delete("> # ").as_deref(), Some("> "));
    assert_eq!(delete("- ### ").as_deref(), Some("- "));
    assert_eq!(delete("#"), None);
    assert_eq!(delete("####### "), None);
    assert_eq!(delete("# 标"), None);
    // 光标前还有文字、只有缩进、或为空行时按普通删除处理。
    assert_eq!(delete("- 甲"), None);
    assert_eq!(delete("  "), None);
    assert_eq!(delete(""), None);
    assert_eq!(delete("**"), None);
}

/// 在 `text` 的最后一行上按 Tab（`outdent` 为真时为 ⇧Tab），返回修改后的文字。
fn tab(text: &str, outdent: bool) -> Option<String> {
    let (above, line) = text.rsplit_once('\n').unwrap_or(("", text));
    let lines: Vec<&str> = if above.is_empty() {
        Vec::new()
    } else {
        above.rsplit('\n').collect()
    };
    let (range, indent) = shift(line, lines.into_iter(), outdent)?;
    let mut text = text.to_owned();
    let start = text.len() - line.len();
    text.replace_range(start + range.start..start + range.end, &indent);
    Some(text)
}

#[test]
fn enter_after_a_fence_closes_it() {
    let close = |line: &str, below: &[&str]| fence(line, "", below.iter().copied());
    assert_eq!(
        close("```rust", &[]),
        Some(("\n".to_owned(), "\n```".to_owned()))
    );
    assert_eq!(
        close("> ~~~~", &["后文"]),
        Some(("\n> ".to_owned(), "\n> ~~~~".to_owned()))
    );
    assert_eq!(
        close("$$", &["", "$$"]),
        Some(("\n".to_owned(), "\n$$".to_owned()))
    );
    // 下一个代码块的开围栏不视为闭围栏。
    assert!(close("```", &["文字", "```js", "x", "```"]).is_some());
    // 已闭合时按普通换行处理。
    assert_eq!(close("```rust", &["x", "```"]), None);
    assert_eq!(close("$$", &["x^2", "$$"]), None);
    assert_eq!(close("`` `a` ``", &[]), None);
    assert_eq!(close("正文", &[]), None);
    assert_eq!(fence("```", "x", std::iter::empty()), None);
}

#[test]
fn tab_nests_list_items() {
    assert_eq!(tab("- 一\n- 二", false).as_deref(), Some("- 一\n  - 二"));
    // 有序列表项成为第一个子项时从 1 开始编号，之后的子项接着上一项编号。
    assert_eq!(
        tab("1. 一\n2. 二", false).as_deref(),
        Some("1. 一\n   1. 二")
    );
    assert_eq!(
        tab("1. 一\n   1. 甲\n2. 乙", false).as_deref(),
        Some("1. 一\n   1. 甲\n   2. 乙")
    );
    assert_eq!(
        tab("- 一\n  - 甲\n  - 乙", false).as_deref(),
        Some("- 一\n  - 甲\n    - 乙")
    );
    // 第一项没有上一项，保持不变。
    assert_eq!(tab("段落\n- 一", false).as_deref(), Some("段落\n- 一"));
    assert_eq!(tab("- 一\n  - 甲", false).as_deref(), Some("- 一\n  - 甲"));
    assert_eq!(
        tab("> - 一\n> - 二", false).as_deref(),
        Some("> - 一\n>   - 二")
    );
    // ⇧Tab 退出一层。
    assert_eq!(
        tab("- 一\n  - 甲\n    - 子", true).as_deref(),
        Some("- 一\n  - 甲\n  - 子")
    );
    assert_eq!(tab("1. 一\n   - 甲", true).as_deref(), Some("1. 一\n- 甲"));
    // 有序列表项退出一层后接在上一层的那一项之后编号。
    assert_eq!(
        tab("1. 一\n2. 二\n   1. 甲\n   2. 乙", true).as_deref(),
        Some("1. 一\n2. 二\n   1. 甲\n3. 乙")
    );
    assert_eq!(tab("- 一", true).as_deref(), Some("- 一"));
    // 不是列表项时按普通缩进处理。
    assert_eq!(tab("- 一\n段落", false), None);
}

#[test]
fn line_breaks_stay_in_the_block() {
    assert_eq!(line_break("一段"), "\n");
    assert_eq!(line_break("- 一"), "\n  ");
    assert_eq!(line_break("  1. 一"), "\n     ");
    assert_eq!(line_break("> - [ ] 一"), "\n>       ");
    assert_eq!(line_break("> 引用"), "\n> ");
}

#[test]
fn items_after_a_new_one_are_renumbered() {
    let below = ["2. 乙", "   - 子项", "", "3. 丙", "", "后文", "4. 丁"];
    assert_eq!(
        renumber("2. ", below.iter()),
        vec![(0, 0..1, 3), (3, 0..1, 4)]
    );
    // 序号已经正确、不是有序列表项时不改。
    assert!(renumber("2. ", ["3. 丙"].iter()).is_empty());
    assert!(renumber("- ", ["- 乙"].iter()).is_empty());
    // 引用中的列表、位数变化的序号。
    assert_eq!(renumber("> 10. ", ["> 10. 乙"].iter()), vec![(0, 2..4, 11)]);
    // 上一层的项不属于这个列表。
    assert!(renumber("   2. ", ["2. 外层"].iter()).is_empty());
}

#[test]
fn task_shorthands_become_task_items_on_space() {
    let run = |before: &str, after: &str| {
        task(before, after).map(|e| {
            format!(
                "{}{}|{}",
                &before[..before.len() - e.before],
                e.text,
                &after[e.after..]
            )
        })
    };
    assert_eq!(run("[]", "").as_deref(), Some("- [ ] |"));
    assert_eq!(run("-[]", "").as_deref(), Some("- [ ] |"));
    assert_eq!(run("- []", "").as_deref(), Some("- [ ] |"));
    assert_eq!(run("1. []", " 文字").as_deref(), Some("1. [ ] | 文字"));
    // 完整的 `- [ ]` 之后输入空格即为任务项，不需要替换。
    assert_eq!(run("- [ ]", ""), None);
    assert_eq!(run("> 【】", "").as_deref(), Some("> - [ ] |"));
    // 括号配对把右括号补在光标之后：连同右括号一起替换。
    assert_eq!(run("- [", "]").as_deref(), Some("- [ ] |"));
    assert_eq!(run("  *[", "]后").as_deref(), Some("  * [ ] |后"));
    // 已经是任务项、不在行首、后面是链接时不处理。
    assert_eq!(run("- [ ] ", ""), None);
    assert_eq!(run("文字 []", ""), None);
    assert_eq!(run("[x]", "(网址)"), None);
}

#[test]
fn margins_split_quote_marks_from_indentation() {
    assert_eq!(margins("> - 项"), (2, 0));
    assert_eq!(margins(">   续行"), (2, 2));
    assert_eq!(margins("    - 子项"), (0, 4));
    assert_eq!(indent("  续行"), 2);
}
