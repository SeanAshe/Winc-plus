//! `format` 的测试：添加与移除标记、选区两端的空格、没有选区时的三种情形。

use super::*;

/// 对 `text` 中 `[` 与 `]` 之间的选区（只有 `|` 时为光标）切换 `mark`，返回同样标注的结果。
fn run(text: &str, mark: &str) -> String {
    let (line, selection) = match text.find('|') {
        Some(at) => (text.replace('|', ""), at..at),
        None => {
            let (a, b) = (text.find('[').unwrap(), text.find(']').unwrap() - 1);
            (text.replace(['[', ']'], ""), a..b)
        }
    };
    let t = toggle(&line, selection, mark);
    let mut out = format!(
        "{}{}{}",
        &line[..t.range.start],
        t.text,
        &line[t.range.end..]
    );
    if t.select.is_empty() {
        out.insert(t.select.start, '|');
    } else {
        out.insert(t.select.end, ']');
        out.insert(t.select.start, '[');
    }
    out
}

#[test]
fn selections_gain_and_lose_marks() {
    assert_eq!(run("甲[乙]丙", "**"), "甲**[乙]**丙");
    assert_eq!(run("甲**[乙]**丙", "**"), "甲[乙]丙");
    assert_eq!(run("甲[**乙**]丙", "**"), "甲[乙]丙");
    assert_eq!(run("a [code] b", "`"), "a `[code]` b");
    assert_eq!(run("a `[code]` b", "`"), "a [code] b");
    assert_eq!(run("[旧]", "~~"), "~~[旧]~~");
    // 选区两端的空格留在标记之外：`** 乙 **` 不构成粗体。
    assert_eq!(run("甲[ 乙 ]丙", "**"), "甲 **[乙]** 丙");
}

#[test]
fn italic_and_bold_marks_are_told_apart() {
    // 粗体中的文字切换斜体：再加一层 `*`，不拆掉粗体。
    assert_eq!(run("**[乙]**", "*"), "***[乙]***");
    assert_eq!(run("***[乙]***", "*"), "**[乙]**");
    assert_eq!(run("***[乙]***", "**"), "*[乙]*");
    assert_eq!(run("*[乙]*", "**"), "***[乙]***");
}

#[test]
fn a_caret_inserts_leaves_or_removes_a_pair() {
    assert_eq!(run("甲|乙", "**"), "甲**|**乙");
    assert_eq!(run("甲**|**乙", "**"), "甲|乙");
    assert_eq!(run("**粗|**后", "**"), "**粗**|后");
    assert_eq!(run("|", "`"), "`|`");
}
