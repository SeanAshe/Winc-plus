//! 一行中链接的识别。

use super::*;

/// 返回 `line` 中 `^` 所在的列处的链接（`^` 不属于这一行）。
fn at(line: &str, markdown: bool) -> Option<(String, Target)> {
    let col = line.chars().position(|c| c == '^').unwrap();
    let line = line.replacen('^', "", 1);
    let link = link_at(&line, col, markdown)?;
    let text: String = line
        .chars()
        .skip(link.cols.start)
        .take(link.cols.len())
        .collect();
    Some((text, link.target))
}

fn address(text: &str, address: &str) -> Option<(String, Target)> {
    Some((text.to_owned(), Target::Address(address.to_owned())))
}

#[test]
fn urls_are_found_in_any_file() {
    let url = "https://winc.si/zh-cn/";
    assert_eq!(
        at("// 见 https://winc^.si/zh-cn/。", false),
        address(url, url)
    );
    assert_eq!(
        at("see (^https://winc.si/zh-cn/).", false),
        address(url, url)
    );
    assert_eq!(at("x = \"HTTP://a.b/c^\";", false), None);
    assert_eq!(
        at("x = \"HTTP://a.b^/c\";", false),
        address("HTTP://a.b/c", "HTTP://a.b/c")
    );
    let paired = "https://a.b/wiki/X_(y)";
    assert_eq!(
        at("https://a.b/wi^ki/X_(y), z", false),
        address(paired, paired)
    );
    // 不是网址的开头，或光标不在网址上。
    for line in [
        "xhttp://a^.b",
        "ftp://a^.b",
        "http://^ x",
        "a ^ https://a.b",
    ] {
        assert_eq!(at(line, false), None, "{line}");
    }
}

#[test]
fn markdown_links_cover_the_whole_form() {
    let link = "[Winc](https://winc.si)";
    assert_eq!(
        at("见 [Wi^nc](https://winc.si)。", true),
        address(link, "https://winc.si")
    );
    assert_eq!(
        at("见 [Winc](https://wi^nc.si)。", true),
        address(link, "https://winc.si")
    );
    assert_eq!(
        at("[a](b.md) [c^](d.md#e \"标题\")", true),
        address("[c](d.md#e \"标题\")", "d.md#e")
    );
    assert_eq!(
        at("[a](<b c^.md>)", true),
        address("[a](<b c.md>)", "b c.md")
    );
    assert_eq!(
        at("![图^](a/b.png)", true),
        address("[图](a/b.png)", "a/b.png")
    );
    assert_eq!(
        at("[a [b]^](c(d).md)", true),
        address("[a [b]](c(d).md)", "c(d).md")
    );
    // 其他语言中方括号加圆括号不是链接；空地址、光标在外面时没有链接。
    assert_eq!(at("a[i^](x)", false), None);
    assert_eq!(at("[a^]()", true), None);
    assert_eq!(at("[a](b)^ c", true), None);
}

#[test]
fn wikilinks_give_the_name_without_the_label() {
    let wiki = |name: &str| Target::Wiki(name.to_owned());
    assert_eq!(
        at("见 [[wi^nc]]。", true),
        Some(("[[winc]]".to_owned(), wiki("winc")))
    );
    assert_eq!(
        at("[[a]] [[winc.csv#表 | 显示^]]", true).map(|(_, target)| target),
        Some(wiki("winc.csv#表"))
    );
    assert_eq!(at("[[wi^nc]]", false), None);
    assert_eq!(at("[[^]]", true), None);
    assert_eq!(at("[[a]]^ b", true), None);
}
