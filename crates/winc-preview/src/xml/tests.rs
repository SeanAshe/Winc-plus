//! `xml` 的测试：嵌套层数的统计、带标签的实体，以及实体的还原。

use super::*;

#[test]
fn nesting_counts_elements_only() {
    // 不计入注释、CDATA、DOCTYPE、处理指令、自闭合标签和属性值中的大于号。
    assert!(!nesting_exceeds(
        r#"<!DOCTYPE svg [<!ENTITY a "<g>">]><svg><!-- <g><g> --><![CDATA[<g>]]><?x <g>?><g a="<>"/><g></g></svg>"#,
        2
    ));
    assert!(nesting_exceeds("<svg><g><g></g></g></svg>", 2));
    assert!(!nesting_exceeds("<svg><g/><g></g><g></g></svg>", 2));
}

#[test]
fn entities_that_expand_to_tags_are_found() {
    assert!(!declares_markup("<p>a &amp; b</p>"));
    assert!(!declares_markup(
        r#"<!DOCTYPE html [<!ENTITY nbsp "&#160;"><!ENTITY name 'Winc'>]><html/>"#
    ));
    assert!(declares_markup(
        r#"<!DOCTYPE html [<!ENTITY a "<b>&a0;</b>">]><html/>"#
    ));
    assert!(declares_markup(
        r#"<!DOCTYPE html [<!ENTITY ok "x"><!ENTITY a '&#60;b>&#x3c;/b>'>]><html/>"#
    ));
    // 声明没有写完：没有可以展开的值。
    assert!(!declares_markup("<!DOCTYPE html [<!ENTITY a "));
}

#[test]
fn entities_are_restored_and_unknown_ones_kept() {
    assert_eq!(unescape("a &lt;b&gt; &#x4E2D;&#10;"), "a <b> 中\n");
    assert_eq!(unescape("&unknown; & &;"), "&unknown; & &;");
    // 没有分号的一长串 `&`：逐个原样保留。
    let many = "&".repeat(10_000);
    assert_eq!(unescape(&many), many);
    assert_eq!(unescape("&#x10FFFF;"), "\u{10FFFF}");
}
