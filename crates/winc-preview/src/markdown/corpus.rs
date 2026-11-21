//! 解析的回归语料：逐条断言 CommonMark、GFM 中易错写法的解析结果。
//!
//! 结果写成提纲（[`outline`]）：每块一行，行内样式写回 Markdown 标记，嵌套的块缩进两格。

use super::*;

/// 把解析结果写成提纲。
fn outline(markdown: &str) -> String {
    let doc = parse(markdown);
    let mut out = String::new();
    blocks(&doc.blocks, 0, &mut out);
    out
}

fn blocks(blocks: &[Block], depth: usize, out: &mut String) {
    for block in blocks {
        let pad = "  ".repeat(depth);
        match block {
            Block::Paragraph(inline) => out.push_str(&format!("{pad}P {}\n", inline_text(inline))),
            Block::Heading(level, inline) => {
                out.push_str(&format!("{pad}H{level} {}\n", inline_text(inline)));
            }
            Block::Code { lang, text, .. } => {
                out.push_str(&format!("{pad}Code({lang}) {text:?}\n"));
            }
            Block::Quote(alert, inner) => {
                out.push_str(&format!(
                    "{pad}Quote{}\n",
                    alert.map_or(String::new(), |a| format!("({a:?})"))
                ));
                self::blocks(inner, depth + 1, out);
            }
            Block::List(start, items) => {
                out.push_str(&format!(
                    "{pad}List({})\n",
                    start.map_or("-".to_owned(), |n| n.to_string())
                ));
                for item in items {
                    let task = match item.task {
                        Some(true) => "[x] ",
                        Some(false) => "[ ] ",
                        None => "",
                    };
                    out.push_str(&format!("{pad}  Item {task}\n"));
                    self::blocks(&item.blocks, depth + 2, out);
                }
            }
            Block::Table(aligns, rows, head) => {
                let cells: Vec<String> = (rows.iter())
                    .map(|r| r.iter().map(inline_text).collect::<Vec<_>>().join(" | "))
                    .collect();
                out.push_str(&format!("{pad}Table{aligns:?} head={head} {cells:?}\n"));
            }
            Block::Math(tex, _) => out.push_str(&format!("{pad}Math {tex:?}\n")),
            Block::Rule(_) => out.push_str(&format!("{pad}Rule\n")),
            Block::Footnote(n, inner) => {
                out.push_str(&format!("{pad}Footnote {n}\n"));
                self::blocks(inner, depth + 1, out);
            }
            Block::Details {
                open,
                summary,
                blocks: inner,
            } => {
                out.push_str(&format!(
                    "{pad}Details open={open} {}\n",
                    inline_text(summary)
                ));
                self::blocks(inner, depth + 1, out);
            }
        }
    }
}

/// 行内文字，样式写回 Markdown 的标记。
fn inline_text(inline: &Inline) -> String {
    let mut out = String::new();
    let mut images = inline.images.iter();
    for (range, style) in inline.runs() {
        let mut text: String = inline.text[range].to_owned();
        if text.contains(IMAGE_MARK) {
            text = text
                .chars()
                .map(|c| match c {
                    IMAGE_MARK => images
                        .next()
                        .map_or(String::new(), |i| format!("![{}]({})", i.alt, i.url)),
                    c => c.to_string(),
                })
                .collect();
        }
        for (flag, mark) in [
            (Style::CODE, "`"),
            (Style::STRIKE, "~~"),
            (Style::EMPHASIS, "*"),
            (Style::STRONG, "**"),
        ] {
            if style.has(flag) {
                text = format!("{mark}{text}{mark}");
            }
        }
        if style.link > 0
            && let Some(url) = inline.links.get(usize::from(style.link) - 1)
        {
            text = format!("[{text}]({url})");
        }
        out.push_str(&text);
    }
    out
}

/// 逐条比对：每条是原文和期望的提纲。
fn check(cases: &[(&str, &str)]) {
    for (markdown, expected) in cases {
        assert_eq!(outline(markdown), *expected, "原文：{markdown:?}");
    }
}

#[test]
fn block_structure() {
    check(&[
        // 引用中的懒惰续行属于同一段；中文之间的软换行不加空格。
        ("> 一\n二\n", "Quote\n  P 一\n二\n"),
        // Setext 标题、行尾的 `#`。
        ("标题\n===\n\n## 二级 ##\n", "H1 标题\nH2 二级\n"),
        // 缩进四格为代码；未闭合的围栏延续到文末。
        ("    code\n", "Code() \"code\"\n"),
        ("~~~ rust ignore\nfn a() {}\n", "Code(rust) \"fn a() {}\"\n"),
        // 分隔线的几种写法。
        ("* * *\n\n___\n", "Rule\nRule\n"),
        // 有序列表的起始编号；松散列表的项里是段落。
        (
            "3. 甲\n4. 乙\n",
            "List(3)\n  Item \n    P 甲\n  Item \n    P 乙\n",
        ),
        // 嵌套列表和任务项。
        (
            "- 一\n  - [x] 二\n  - [ ] 三\n",
            "List(-)\n  Item \n    P 一\n    List(-)\n      Item [x] \n        P 二\n      Item [ ] \n        P 三\n",
        ),
        // GFM 提示块。
        ("> [!NOTE]\n> 注意\n", "Quote(Note)\n  P 注意\n"),
    ]);
}

#[test]
fn inline_markup() {
    check(&[
        // 反斜杠转义、实体。
        (
            "\\*不是斜体\\* &amp; &copy; &#35;\n",
            "P *不是斜体* & © #\n",
        ),
        // 词中的下划线不表示强调，星号表示强调。
        ("snake_case_name 和 a*b*c\n", "P snake_case_name 和 a*b*c\n"),
        (
            "**粗** __粗__ *斜* ~~删~~ `码`\n",
            "P **粗** **粗** *斜* ~~删~~ `码`\n",
        ),
        // 链接引用定义；尖括号自动链接和裸网址。
        ("[文档][d]\n\n[d]: /docs \"标题\"\n", "P [文档](/docs)\n"),
        (
            "<https://a.dev> 和 www.b.dev\n",
            "P [https://a.dev](https://a.dev) 和 [www.b.dev](http://www.b.dev)\n",
        ),
        // 图片的标题不计入替代文字。
        ("![图](a.png \"标题\")\n", "P ![图](a.png)\n"),
        // 行尾两个空格和反斜杠是硬换行。
        ("一  \n二\\\n三\n", "P 一\n二\n三\n"),
    ]);
}

#[test]
fn gfm_tables() {
    check(&[
        // 转义的竖线保留在单元格中；缺少的单元格补为空。
        (
            "| a | b |\n|:-|-:|\n| x \\| y | z |\n| 只有一格 |\n",
            "Table[Left, Right] head=true [\"a | b\", \"x | y | z\", \"只有一格 | \"]\n",
        ),
    ]);
}
