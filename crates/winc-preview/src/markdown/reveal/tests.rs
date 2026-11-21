//! `reveal` 的测试：只显示光标所在元素的标记，续行去掉容器的标记。

use super::*;

/// 返回光标位于 `text` 的 `|` 处时显示的文字，标记用 `‹›` 括起。
fn shown(text: &str) -> String {
    let caret = text.find('|').unwrap();
    let text = text.replace('|', "");
    let (Block::Paragraph(inline) | Block::Heading(_, inline)) = reveal(&text, Some(caret), None)
    else {
        panic!();
    };
    let mut out = String::new();
    for (range, style) in inline.runs() {
        let part = &inline.text[range];
        if style.has(Style::SYNTAX) {
            out.push_str(&format!("‹{part}›"));
        } else {
            out.push_str(part);
        }
    }
    out
}

#[test]
fn only_the_touched_element_shows_its_markers() {
    // 未接触任何元素时，显示结果即排版后的文字。
    assert_eq!(shown("|默认 **关** 和 `x`"), "默认 关 和 x");
    assert_eq!(shown("默认 **关|** 和 `x`"), "默认 ‹**›关‹**› 和 x");
    assert_eq!(shown("默认 **关** 和 |`x`"), "默认 关 和 ‹`›x‹`›");
    assert_eq!(shown("见 [文档|](a.md)。"), "见 ‹[›文档‹](a.md)›。");
    // 嵌套元素只显示被接触的各层。
    assert_eq!(shown("**粗 *斜|* 粗**"), "‹**›粗 ‹*›斜‹*› 粗‹**›");
    // 标题的 `#` 在光标接触时显示。
    assert_eq!(shown("## 标|题"), "标题");
    assert_eq!(shown("|## 标题"), "‹## ›标题");
    // 光标接触到的公式显示 LaTeX 原文。
    assert_eq!(shown("设 $x|^2$。"), "设 ‹$›x^2‹$›。");
    // 光标接触到的图片显示原文，图片另起一行排在下方。
    let mark = super::super::IMAGE_MARK;
    assert_eq!(shown("|![图](a.png)"), format!("‹![图](a.png)›\n{mark}"));
    assert_eq!(shown("见 ![图](a.png) |尾"), format!("见 {mark} 尾"));
}

#[test]
fn continuation_lines_drop_container_markers() {
    // 续行的 `>` 和缩进不显示，软换行照原文换行；文字与原文逐字对照。
    let text = "一 **二**\n> 三";
    let Block::Paragraph(inline) = reveal(text, Some(0), None) else {
        panic!();
    };
    assert_eq!(inline.text, "一 二\n三");
    let three = inline.text.find('三').unwrap();
    assert_eq!(inline.source_of(three), text.find('三'));
    let two = inline.text.find('二').unwrap();
    assert_eq!(inline.source_of(two), text.find('二'));
    // 输入法组字排在光标处，其后的文字向后移动。
    let Block::Paragraph(inline) = reveal("甲乙", Some(3), Some("丙")) else {
        panic!();
    };
    assert_eq!(inline.text, "甲丙乙");
    // 已输入为其他种类的块（尚未重新解析）时逐字显示。
    let Block::Paragraph(inline) = reveal("- 一\n- 二", Some(0), None) else {
        panic!();
    };
    assert_eq!(inline.text, "- 一\n- 二");
}

#[test]
fn highlight_markers_show_when_touched() {
    // 光标在高亮里：两个 `==` 显示出来。
    assert_eq!(shown("前面 ==高|亮== 后面"), "前面 ‹==›高亮‹==› 后面");
    assert_eq!(shown("前面 =|=高亮== 后面"), "前面 ‹==›高亮‹==› 后面");
    assert_eq!(shown("前面 ==高亮==| 后面"), "前面 ‹==›高亮‹==› 后面");
    // 光标在别处：标记不显示。
    assert_eq!(shown("前面| ==高亮== 后面"), "前面 高亮 后面");
    // 不成对的 `==` 是文字。
    assert_eq!(shown("前面 == 高|亮 后面"), "前面 == 高亮 后面");
    // 转义的 `\==` 是文字，不随光标显示为标记。
    assert_eq!(shown("前面 \\==高|亮== 后面"), "前面 ==高亮== 后面");
}

#[test]
fn trailing_spaces_show_at_the_caret() {
    assert_eq!(shown("风格 |"), "风格‹ ›");
    assert_eq!(shown("风格|  "), "风格‹  ›");
    assert_eq!(shown("|风格 "), "风格");
    assert_eq!(shown("甲 |\n乙"), "甲‹ ›\n乙");
    assert_eq!(shown("|甲 \n乙"), "甲\n乙");
}
