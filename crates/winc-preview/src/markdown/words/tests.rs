use super::*;

fn words(text: &str) -> usize {
    count([text])
}

#[test]
fn han_characters_count_one_each_and_latin_text_by_word() {
    assert_eq!(words("中文和 English 混排"), 6);
    assert_eq!(words("don't stop well-known"), 3);
    assert_eq!(words("v2 1.5 1,000 个"), 4);
    assert_eq!(words(""), 0);
}

#[test]
fn markup_link_addresses_and_list_numbers_are_left_out() {
    assert_eq!(words("# 标题\n\n**粗体** 和 `code`"), 6);
    assert_eq!(words("看 [文档](https://winc.si/docs (x)) 吧"), 4);
    assert_eq!(words("![图](a.png) <img src=\"x\"> <br/>"), 1);
    assert_eq!(words("1. 一\n2) 二\n> 3. 三\n4.5 版"), 5);
    // 片段的边界不影响结果。
    assert_eq!(count(["[文", "档](http", "s://x) do", "n't"]), 3);
}
