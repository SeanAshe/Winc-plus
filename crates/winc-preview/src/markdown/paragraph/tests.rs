use super::*;

fn texts(lines: &[&str], kind: Kind) -> Vec<String> {
    change(lines, kind).into_iter().map(|l| l.text).collect()
}

#[test]
fn headings_are_set_replaced_and_toggled_back() {
    assert_eq!(texts(&["标题"], Kind::Heading(2)), ["## 标题"]);
    assert_eq!(texts(&["### 标题"], Kind::Heading(1)), ["# 标题"]);
    // 已经是这一级时改回正文；⌘0 总是改回正文。
    assert_eq!(texts(&["## 标题"], Kind::Heading(2)), ["标题"]);
    assert_eq!(texts(&["## 标题"], Kind::Heading(0)), ["标题"]);
    // 引用和列表里的标题写在标记之后；`#tag` 不是标题。
    assert_eq!(texts(&["> - 一项"], Kind::Heading(3)), ["> - ### 一项"]);
    assert_eq!(texts(&["#tag"], Kind::Heading(1)), ["# #tag"]);
    // 光标随内容移动。
    let line = &change(&["## 标题"], Kind::Heading(4))[0];
    assert_eq!((line.old_start, line.new_start), (3, 5));
}

#[test]
fn quotes_and_lists_toggle_on_every_selected_line() {
    assert_eq!(texts(&["一", "", "二"], Kind::Quote), ["> 一", "", "> 二"]);
    assert_eq!(texts(&["> 一", ">> 二"], Kind::Quote), ["一", "> 二"]);
    assert_eq!(texts(&["一", "二"], Kind::Ordered), ["1. 一", "2. 二"]);
    assert_eq!(texts(&["- 一", "* 二"], Kind::Ordered), ["1. 一", "2. 二"]);
    assert_eq!(texts(&["1. 一", "2. 二"], Kind::Ordered), ["一", "二"]);
    // 任务框跟着换记号，去掉列表时一起去掉。
    assert_eq!(texts(&["1. [x] 做完"], Kind::Bullet), ["- [x] 做完"]);
    assert_eq!(texts(&["- [ ] 待办"], Kind::Bullet), ["待办"]);
    // 引用里的列表保留引用和缩进。
    assert_eq!(texts(&[">   二"], Kind::Bullet), [">   - 二"]);
}

#[test]
fn task_boxes_join_existing_list_items() {
    let lines = ["文字", "- 项", "2. 序", "> 引", "- [x] 完"];
    assert_eq!(
        texts(&lines, Kind::Task),
        [
            "- [ ] 文字",
            "- [ ] 项",
            "2. [ ] 序",
            "> - [ ] 引",
            "- [x] 完"
        ]
    );
    // 都是任务时去掉任务框，记号保留。
    assert_eq!(texts(&["- [ ] a", "1. [x] b"], Kind::Task), ["- a", "1. b"]);
}

#[test]
fn blocks_and_links_place_the_caret_where_typing_continues() {
    assert_eq!(fenced("```", ""), ("```\n\n```".to_owned(), 3));
    assert_eq!(fenced("$$", "x^2"), ("$$\nx^2\n$$".to_owned(), 6));
    let (table, caret) = table();
    assert!(table.starts_with("|  |") && caret == 2);
    assert_eq!(link("", None, false), ("[]()".to_owned(), 1..1));
    assert_eq!(link("文字", None, false), ("[文字]()".to_owned(), 9..9));
    assert_eq!(
        link("图", Some("a b.png"), true),
        ("![图](<a b.png>)".to_owned(), 17..17)
    );
}

#[test]
fn link_targets_are_found_while_typing_a_path() {
    assert_eq!(link_target("见 [文档]("), Some((13, Target::File)));
    assert_eq!(link_target("![图](img/a"), Some((7, Target::Image)));
    assert_eq!(link_target("[x](#概"), Some((4, Target::Anchor)));
    // 网址、锚点、已经结束的地址和行内代码里不补全。
    assert_eq!(link_target("[x](https://a"), None);
    assert_eq!(link_target("[x](http:"), None);
    // 只写了网址的开头时仍然补全：`ma`、`ht` 也是文件名的开头。
    assert_eq!(link_target("[x](ma"), Some((4, Target::File)));
    assert_eq!(link_target("[x](a.md) 后面"), None);
    assert_eq!(link_target("`[x](a"), None);
    assert_eq!(link_target("没有链接"), None);
}

#[test]
fn headings_get_the_same_anchors_as_the_preview() {
    let text =
        "# 概览\n\n```\n# 不是标题\n```\n## 安装 ##\n#tag\n## 安装\n# C#\n## [链接](a.md) **粗**\n";
    let got = headings(text.lines());
    let slugs: Vec<_> = got
        .iter()
        .map(|(l, t, s)| (*l, t.as_str(), s.as_str()))
        .collect();
    assert_eq!(
        slugs,
        [
            (1, "概览", "概览"),
            (2, "安装", "安装"),
            (2, "安装", "安装-1"),
            (1, "C#", "c"),
            (2, "链接 粗", "链接-粗")
        ]
    );
}

#[test]
fn slash_opens_the_command_list_at_a_word_start() {
    assert_eq!(slash("/"), Some(0));
    assert_eq!(slash("- /标题"), Some(2));
    assert_eq!(slash("文字 /h"), Some(7));
    // 词中、路径、网址、链接地址和行内代码中的 `/` 不呼出。
    assert_eq!(slash("a/b"), None);
    assert_eq!(slash("见 /usr/bin"), None);
    assert_eq!(slash("https://"), None);
    assert_eq!(slash("[a](/"), None);
    assert_eq!(slash("`a /"), None);
    assert_eq!(slash("/h 1"), None);
}
