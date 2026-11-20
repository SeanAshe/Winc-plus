//! 改写图片大小的测试。

use super::*;

fn apply(text: &str, width: Option<Width>) -> String {
    let (range, new) = resize(text, width, true).expect("应当改写");
    format!("{}{new}{}", &text[..range.start], &text[range.end..])
}

#[test]
fn markdown_images_become_html_with_a_width() {
    let text = "![截图](shot.png) 后文";
    assert_eq!(
        apply(text, Some(Width::Px(320))),
        "<img src=\"shot.png\" alt=\"截图\" width=\"320\"> 后文"
    );
    // 标题、需要转义的字符和替代文字末尾的 `|300` 都正确处理。
    let text = "![a \"b\" |300](<my shot.png> \"标题\")";
    assert_eq!(
        apply(text, Some(Width::Percent(50))),
        "<img src=\"my shot.png\" alt=\"a &quot;b&quot;\" title=\"标题\" width=\"50%\">"
    );
    // 去掉大小：没有写明大小时不修改，`|300` 被去掉。
    assert_eq!(resize("![a](b.png)", None, true), None);
    assert_eq!(apply("![a |300x200](b.png)", None), "![a](b.png)");
    // 高为 0 的不算写明的大小。
    assert_eq!(resize("![a |300x0](b.png)", None, true), None);
    // 无法识别的写法不修改。
    assert_eq!(resize("![a][ref]", Some(Width::Px(10)), true), None);
}

#[test]
fn html_images_keep_their_other_attributes() {
    let text = "<img src=\"a.png\" alt=\"A\" width=\"100\" height=50 class=x>";
    assert_eq!(
        apply(text, Some(Width::Px(240))),
        "<img src=\"a.png\" alt=\"A\" class=x width=\"240\">"
    );
    let text = "<img src='a.png' style=\"zoom: 50%; border: 0\"/>";
    assert_eq!(
        apply(text, Some(Width::Px(80))),
        "<img src='a.png' style=\"border: 0\" width=\"80\"/>"
    );
    // HTML 块中的图片从自身的 `<img` 起改写，去掉大小后仍为 HTML。
    let text = "<img src=\"b.png\" width=\"9\">\n</p>";
    assert_eq!(
        apply(text, Some(Width::Percent(25))),
        "<img src=\"b.png\" width=\"25%\">\n</p>"
    );
    let (range, new) = resize(text, None, false).unwrap();
    assert_eq!((range, new.as_str()), (0..27, "<img src=\"b.png\">"));
    // 图片不在开头时不改写。
    assert_eq!(resize("<p><img src=\"a.png\"></p>", None, true), None);
}

#[test]
fn removing_the_size_turns_plain_html_back_into_markdown() {
    let text = "<img src=\"my shot.png\" alt=\"a [b]\" width=\"320\">";
    assert_eq!(apply(text, None), "![a \\[b\\]](<my%20shot.png>)");
    let text = "<img src=\"a.png\" alt=\"&amp;\" title=\"t\" width=\"1\">";
    assert_eq!(apply(text, None), "![&](a.png \"t\")");
    let text = "<img src=\"a.png\" title='a\\ \"b\"' width=\"1\">";
    assert_eq!(apply(text, None), "![](a.png \"a\\\\ \\\"b\\\"\")");
    // 带其他属性时保留 HTML，只去掉大小。
    let text = "<img src=\"a.png\" class=x width=\"1\">";
    assert_eq!(apply(text, None), "<img src=\"a.png\" class=x>");
    // 写入宽度后再去掉，恢复原文。
    let text = "![图](a.png)";
    let sized = apply(text, Some(Width::Px(200)));
    assert_eq!(apply(&sized, None), text);
}
