//! `markdown` 解析的测试：块与行内样式、表格与行内文字、HTML、脚注、数学与 mermaid 代码块。

use super::*;

fn paragraph(block: &Block) -> &Inline {
    match block {
        Block::Paragraph(inline) => inline,
        other => panic!("不是段落：{other:?}"),
    }
}

/// 各样式段的文字和样式标志。
fn spans(inline: &Inline) -> Vec<(&str, u16)> {
    inline
        .runs()
        .map(|(range, style)| (&inline.text[range], style.flags))
        .collect()
}

#[test]
fn blocks_come_out_in_order() {
    let doc = parse("# 标题\n\n第一段\n第二行\n\n---\n\n```rust title\nfn main() {}\n```\n");
    assert_eq!(doc.blocks.len(), 4);
    assert_eq!(doc.hashes.len(), 4);
    assert!(matches!(&doc.blocks[0], Block::Heading(1, h) if h.text == "标题"));
    // 中文之间的软换行不加空格。
    assert_eq!(paragraph(&doc.blocks[1]).text, "第一段第二行");
    assert_eq!(doc.blocks[2], Block::Rule(Some(0)));
    assert!(matches!(
        &doc.blocks[3],
        Block::Code { lang, text, .. } if lang == "rust" && text == "fn main() {}"
    ));
    // 西文之间、中西文之间照常是空格。
    let doc = parse("one\ntwo\n中文\nthree");
    assert_eq!(paragraph(&doc.blocks[0]).text, "one two 中文 three");
}

#[test]
fn inline_styles_and_links_are_runs() {
    let doc = parse("a **b *c*** ~~d~~ `e` [f](https://x.dev)  \ng");
    let inline = paragraph(&doc.blocks[0]);
    assert_eq!(
        spans(inline),
        vec![
            ("a ", 0),
            ("b ", Style::STRONG),
            ("c", Style::STRONG | Style::EMPHASIS),
            (" ", 0),
            ("d", Style::STRIKE),
            (" ", 0),
            ("e", Style::CODE),
            (" ", 0),
            ("f", 0),
            ("\ng", 0),
        ]
    );
    assert_eq!(inline.links, ["https://x.dev"]);
    let link = inline
        .runs()
        .find(|(r, _)| &inline.text[r.clone()] == "f")
        .unwrap();
    assert_eq!(link.1.link, 1);
}

#[test]
fn lists_tasks_quotes_and_alerts_nest() {
    let doc = parse("- [x] 完成\n- 待办\n  1. 子项\n\n> [!NOTE]\n> 注意\n");
    let Block::List(None, items) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks[0]);
    };
    assert_eq!(items[0].task, Some(true));
    assert_eq!(paragraph(&items[0].blocks[0]).text, "完成");
    assert_eq!(items[1].task, None);
    assert!(matches!(&items[1].blocks[1], Block::List(Some(1), sub) if sub.len() == 1));
    let Block::Quote(Some(Alert::Note), inner) = &doc.blocks[1] else {
        panic!("{:?}", doc.blocks[1]);
    };
    assert_eq!(paragraph(&inner[0]).text, "注意");

    // 勾选框在原文中的位置相对顶层块的开头，嵌套列表中的勾选框同样如此。
    let text = "段落\n\n- [ ] 甲\n  - [x] 乙\n";
    let doc = parse(text);
    let Block::List(None, items) = &doc.blocks[1] else {
        panic!("{:?}", doc.blocks[1]);
    };
    let start = doc.ranges[1].start;
    assert_eq!(&text[start + items[0].task_source..][..3], "[ ]");
    let Block::List(None, sub) = &items[0].blocks[1] else {
        panic!("{:?}", items[0].blocks);
    };
    assert_eq!(&text[start + sub[0].task_source..][..3], "[x]");
}

#[test]
fn tables_keep_alignment_and_header() {
    let doc = parse("| a | b |\n|:-|-:|\n| 1 | **2** |\n");
    let Block::Table(aligns, rows, true) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks[0]);
    };
    assert_eq!(aligns, &[Align::Left, Align::Right]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1].text, "b");
    assert_eq!(spans(&rows[1][1]), vec![("2", Style::STRONG)]);
}

#[test]
fn empty_table_cells_know_where_they_are() {
    // 引用中的表格：位置相对顶层块（引用）的起点。
    let text = "前\n\n> | a |  |\n> |---|---|\n> |  | b |\n";
    let doc = parse(text);
    let Block::Quote(_, blocks) = &doc.blocks[1] else {
        panic!("{:?}", doc.blocks[1]);
    };
    let Block::Table(_, rows, _) = &blocks[0] else {
        panic!("{:?}", blocks[0]);
    };
    let start = doc.ranges[1].start;
    // 空单元格位于左侧 `|` 之后一个空格处（规则同 `cells::cells`）。
    let at = |cell: &Inline| &text[start + cell.source - 2..start + cell.source];
    assert_eq!(at(&rows[0][1]), "| ");
    assert_eq!(at(&rows[1][0]), "| ");
    assert_eq!(text[start + rows[1][1].source..].chars().next(), Some('b'));
}

#[test]
fn images_sit_in_the_text() {
    let doc = parse("前 ![图 *一*](a.png) 后");
    let inline = paragraph(&doc.blocks[0]);
    assert_eq!(inline.text, format!("前 {IMAGE_MARK} 后"));
    assert_eq!(inline.images[0].url, "a.png");
    assert_eq!(inline.images[0].alt, "图 一");
}

#[test]
fn readme_html_becomes_headings_images_and_text() {
    let html = "<p align=\"center\">\n  <img src='logo.png' width=\"120\" alt=\"logo\">\n</p>\n\
                <h1 align=\"center\">Winc &amp; co</h1>\n<!-- 注释 -->\n";
    let doc = parse(html);
    let image = &paragraph(&doc.blocks[0]).images[0];
    assert_eq!(
        (image.url.as_str(), image.width),
        ("logo.png", Some(Length::Px(120)))
    );
    assert_eq!(image.alt, "logo");
    assert_eq!(paragraph(&doc.blocks[0]).align, Align::Center);
    assert!(
        matches!(&doc.blocks[1], Block::Heading(1, h) if h.text == "Winc & co" && h.align == Align::Center)
    );
    assert_eq!(doc.blocks.len(), 2);

    // `<div align="center">` 里面写的 Markdown 也居中，出了 `</div>` 就不再居中。
    let doc = parse("<div align=\"center\">\n\n# 标题\n\n</div>\n\n正文\n");
    assert!(matches!(&doc.blocks[0], Block::Heading(1, h) if h.align == Align::Center));
    assert_eq!(paragraph(&doc.blocks[1]).align, Align::None);

    // 行内的 `<br>` 换行，`<b>` 加粗。
    let doc = parse("一<br>二 <b>粗</b>");
    assert_eq!(
        spans(paragraph(&doc.blocks[0])),
        vec![("一\n二 ", 0), ("粗", Style::STRONG)]
    );
}

#[test]
fn footnotes_are_numbered_and_moved_to_the_end() {
    let doc = parse("甲[^b] 乙[^a]\n\n[^a]: A\n\n[^b]: B\n\n尾\n");
    let inline = paragraph(&doc.blocks[0]);
    assert_eq!(inline.text, "甲[1] 乙[2]");
    let n = doc.blocks.len();
    assert_eq!(paragraph(&doc.blocks[1]).text, "尾");
    assert_eq!(doc.blocks[n - 3], Block::Rule(None));
    assert!(
        matches!(&doc.blocks[n - 2], Block::Footnote(l, b) if l == "1" && paragraph(&b[0]).text == "B ↩")
    );
    assert!(matches!(&doc.blocks[n - 1], Block::Footnote(l, _) if l == "2"));
}

#[test]
fn unchanged_blocks_keep_their_hash() {
    let before = parse("# A\n\npara one\n\npara two\n");
    let after = parse("# A\n\npara one!\n\npara two\n");
    assert_eq!(before.hashes[0], after.hashes[0]);
    assert_ne!(before.hashes[1], after.hashes[1]);
    assert_eq!(before.hashes[2], after.hashes[2]);
}

#[test]
fn markdown_files_are_recognised_by_extension() {
    assert!(is_markdown(Path::new("README.md")));
    assert!(is_markdown(Path::new("a/b.MarkDown")));
    assert!(!is_markdown(Path::new("a.txt")));
}

#[test]
fn bare_urls_become_links() {
    let doc = parse(
        "见 https://x.dev/a_(b)。和 www.rust-lang.org, 还有 `https://code` 与 [t](https://t.co)\n",
    );
    let inline = paragraph(&doc.blocks[0]);
    assert_eq!(
        inline.links,
        [
            "https://t.co",
            "https://x.dev/a_(b)",
            "http://www.rust-lang.org"
        ]
    );
    // 写明的链接先记下，裸网址排在后面。
    let linked: Vec<&str> = inline
        .runs()
        .filter(|(_, s)| s.link != 0)
        .map(|(r, _)| &inline.text[r])
        .collect();
    assert_eq!(linked, ["https://x.dev/a_(b)", "www.rust-lang.org", "t"]);
    // 末尾的句号、没配对的括号不算；单词中间的不算；主机名不像样的不算。
    let doc = parse("(see https://a.io/x). foohttps://b.io www.nodot\n");
    assert_eq!(paragraph(&doc.blocks[0]).links, ["https://a.io/x"]);
}

/// 回归：自动链接对每个候选都从头扫到这一段文字的末尾，去末尾的 `)` 时每去一个都重数括号，
/// `(www.(www.…` 和一长串 `)` 都是平方级，数十 KB 的一段文字能让预览长时间无响应。
#[test]
fn autolinking_long_runs_stays_linear() {
    let nested = format!("{}x.com\n", "(www.".repeat(20_000));
    let doc = parse(&nested);
    assert_eq!(paragraph(&doc.blocks[0]).links, ["http://www.x.com"]);
    let closes = format!("www.a.com{}\n", ")".repeat(50_000));
    let doc = parse(&closes);
    assert_eq!(paragraph(&doc.blocks[0]).links, ["http://www.a.com"]);
}

#[test]
fn blocks_remember_their_source_lines() {
    let doc = parse("# A\n\npara\nmore\n\n- x\n- y\n\n脚注[^1]\n\n[^1]: 注\n\n尾\n");
    // 依次是标题、段落、列表、段落、尾，末尾是挪过去的分隔线和两条脚注。
    assert_eq!(doc.lines, [0, 2, 5, 8, 12, 13, 13]);
}

#[test]
fn html_blocks_may_start_with_chinese_text() {
    // HTML 块里 `<img>` 后面紧跟中文：按字节切会切在「简」的中间，解析线程 panic。
    let doc = parse(
        "<p align=\"center\"><img src=\"a.png\">简体中文 | <a href=\"en.md\">English</a></p>\n",
    );
    let inline = paragraph(&doc.blocks[0]);
    assert_eq!(inline.text, format!("{IMAGE_MARK}简体中文 | English"));
    assert_eq!(inline.links, ["en.md"]);
}

#[test]
fn emoji_shortcodes_become_emoji() {
    let doc = parse("完成 :tada: :+1: 时间 12:30:45 `:tada:` :nope:\n");
    assert_eq!(
        paragraph(&doc.blocks[0]).text,
        "完成 🎉 👍 时间 12:30:45 :tada: :nope:"
    );
    // 样式段跟着挪：行内代码仍是原文的 6 个字符。
    let inline = paragraph(&doc.blocks[0]);
    let code: Vec<_> = spans(inline)
        .into_iter()
        .filter(|(_, f)| *f == Style::CODE)
        .collect();
    assert_eq!(code, [(":tada:", Style::CODE)]);
}

#[test]
fn sup_sub_and_picture_come_from_html() {
    let doc = parse("H<sub>2</sub>O 和 x<sup>2</sup>\n");
    assert_eq!(
        spans(paragraph(&doc.blocks[0])),
        vec![
            ("H", 0),
            ("2", Style::SUB),
            ("O 和 x", 0),
            ("2", Style::SUP)
        ]
    );
    let doc = parse(
        "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"dark.png\">\n  \
         <source media=\"(prefers-color-scheme: light)\" srcset=\"light.png 1x, light@2x.png 2x\">\n  \
         <img alt=\"logo\" src=\"fallback.png\">\n</picture>\n",
    );
    let image = &paragraph(&doc.blocks[0]).images[0];
    assert_eq!(image.url, "light.png");
    assert_eq!(image.dark.as_deref(), Some("dark.png"));
}

#[test]
fn front_matter_is_a_table_when_flat() {
    let doc = parse("---\ntitle: \"Hello\"\ntags: [a, b]\n---\n\n正文\n");
    let Block::Table(_, rows, _) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks[0]);
    };
    let texts = |row: &Vec<Inline>| row.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
    assert_eq!(texts(&rows[0]), ["title", "tags"]);
    assert_eq!(texts(&rows[1]), ["Hello", "[a, b]"]);
    // 有嵌套的照常是代码块。
    let doc = parse("---\nauthor:\n  name: x\n---\n");
    assert!(matches!(&doc.blocks[0], Block::Code { lang, .. } if lang == "yaml"));
}

#[test]
fn details_collapse_their_markdown() {
    let doc = parse(
        "<details open>\n<summary>点开看 <b>更多</b></summary>\n\n- 一\n- 二\n\n</details>\n\n<details><summary>x</summary>\n\n正文\n</details>\n\n尾\n",
    );
    let Block::Details {
        open,
        summary,
        blocks,
    } = &doc.blocks[0]
    else {
        panic!("{:?}", doc.blocks[0]);
    };
    assert!(*open);
    assert_eq!(summary.text, "点开看 更多");
    assert!(matches!(&blocks[0], Block::List(None, items) if items.len() == 2));
    assert!(matches!(&doc.blocks[1], Block::Details { open: false, .. }));
    assert_eq!(paragraph(&doc.blocks[2]).text, "尾");
    assert_eq!(doc.lines.len(), 3);
}

#[test]
fn footnotes_link_both_ways() {
    let doc = parse("正文[^a]。\n\n[^a]: 说明\n");
    let body = paragraph(&doc.blocks[0]);
    assert_eq!(body.text, "正文[1]。");
    assert_eq!(body.links, ["#fn-1"]);
    let Block::Footnote(number, blocks) = &doc.blocks[2] else {
        panic!("{:?}", doc.blocks);
    };
    assert_eq!(number, "1");
    let note = paragraph(&blocks[0]);
    assert_eq!(note.text, "说明 ↩");
    assert_eq!(note.links, ["#fnref-1"]);
}

#[test]
fn html_tables_lists_and_pre_become_blocks() {
    let doc = parse(
        "<table>\n<tr><th>名字</th><th align=\"right\">数量</th></tr>\n<tr><td>苹果</td><td>3</td></tr>\n<tr><td>梨\n<td><b>5</b></td>\n</table>\n\n<ul>\n<li>一</li>\n<li>二\n<ol start=\"3\"><li>三</li></ol>\n</li>\n</ul>\n\n<blockquote>引用</blockquote>\n<hr>\n<pre lang=\"x\"><code class=\"language-rust\">fn a() {\n    1 &lt; 2\n}\n</code></pre>\n\n尾\n",
    );
    let Block::Table(aligns, rows, true) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks[0]);
    };
    assert_eq!(aligns, &[Align::None, Align::Right]);
    let texts: Vec<Vec<&str>> = rows
        .iter()
        .map(|r| r.iter().map(|c| c.text.as_str()).collect())
        .collect();
    assert_eq!(texts, [["名字", "数量"], ["苹果", "3"], ["梨", "5"]]);
    let Block::List(None, items) = &doc.blocks[1] else {
        panic!("{:?}", doc.blocks[1]);
    };
    assert_eq!(items.len(), 2);
    assert_eq!(paragraph(&items[1].blocks[0]).text, "二");
    assert!(matches!(&items[1].blocks[1], Block::List(Some(3), inner) if inner.len() == 1));
    assert!(matches!(&doc.blocks[2], Block::Quote(None, b) if paragraph(&b[0]).text == "引用"));
    assert_eq!(doc.blocks[3], Block::Rule(None));
    assert!(matches!(
        &doc.blocks[4],
        Block::Code { lang, text, .. } if lang == "rust" && text == "fn a() {\n    1 < 2\n}"
    ));
    assert_eq!(paragraph(&doc.blocks[5]).text, "尾");
    // 没有 `<th>` 的表格没有表头；单元格里空一行写的 Markdown 接在单元格里。
    let doc = parse("<table><tr><td>\n\n**粗**\n\n</td><td>b</td></tr></table>\n");
    let Block::Table(_, rows, false) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks);
    };
    assert_eq!(rows[0][0].text, "粗");
    assert_eq!(doc.blocks.len(), 1);
}

#[test]
fn html_images_keep_percent_widths() {
    let doc = parse(
        "<p align=\"center\">\n  <img src=\"a.png\" width=\"49.45%\" />\n  <img src=\"b.png\" width=\"120px\" />\n  <img src=\"c.png\" width=\"x\" />\n</p>\n\n<u>下</u> <mark>亮</mark> <small>小</small>\n",
    );
    let inline = paragraph(&doc.blocks[0]);
    assert_eq!(inline.align, Align::Center);
    let widths: Vec<_> = inline.images.iter().map(|i| i.width).collect();
    assert_eq!(
        widths,
        [Some(Length::Percent(4945)), Some(Length::Px(120)), None]
    );
    let styles = spans(paragraph(&doc.blocks[1]));
    assert!(styles.contains(&("下", Style::UNDERLINE)));
    assert!(styles.contains(&("亮", Style::MARK)));
    assert!(styles.contains(&("小", Style::SMALL)));
}

/// 第 `block` 个顶层块的行内文字 `inline` 中，`shown` 第一次出现处在原文中的字节偏移。
fn source_of(doc: &Markdown, block: usize, inline: &Inline, shown: &str) -> usize {
    let at = inline.text.find(shown).expect("文字中没有");
    doc.ranges[block].start + inline.source_of(at).expect("没有对照")
}

#[test]
fn inline_text_maps_back_to_the_source() {
    let cases = [
        // 跳过样式标记、链接地址和转义；中文之间的软换行不生成空格，其后的文字仍能对应到原文。
        ("# 标题\n\n第一段 **粗体** 文字", 1, "文字"),
        ("第一段\n第二行", 0, "第二行"),
        ("> 引用\n> 第二行", 0, "第二行"),
        ("[链接](https://example.com/a) 之后", 0, "之后"),
        ("a\\*b* 星号之后", 0, "星号之后"),
        ("前 :tada: 后", 0, "后"),
        ("`code` 与 $x^2$ 之后", 0, "之后"),
        ("前面\n\n- 一\n- 二\n  续行", 1, "续行"),
    ];
    for (text, block, shown) in cases {
        let doc = parse(text);
        let inline = match &doc.blocks[block] {
            Block::Paragraph(inline) | Block::Heading(_, inline) => inline,
            Block::Quote(_, blocks) => paragraph(&blocks[0]),
            Block::List(_, items) => paragraph(items.last().unwrap().blocks.last().unwrap()),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            source_of(&doc, block, inline, shown),
            text.find(shown).unwrap(),
            "{text}"
        );
    }
}

#[test]
fn code_lines_map_back_to_the_source() {
    let text = "段落\n\n> ```rust\n> let a = 1;\n> let b = 2;\n> ```\n";
    let doc = parse(text);
    let Block::Quote(_, blocks) = &doc.blocks[1] else {
        panic!("{:?}", doc.blocks);
    };
    let Block::Code {
        text: code, map, ..
    } = &blocks[0]
    else {
        panic!("{blocks:?}");
    };
    let at = code.find("let b").unwrap();
    let source = doc.ranges[1].start + map.source(at).unwrap();
    assert_eq!(source, text.find("let b").unwrap());
}

#[test]
fn unchanged_list_items_keep_their_hash_when_an_earlier_item_grows() {
    // 排版缓存按行内文字的哈希取用：前面一项变长后，后面一项的哈希不变，只是原文位置挪了。
    let item = |doc: &Markdown, i: usize| match &doc.blocks[0] {
        Block::List(_, items) => paragraph(&items[i].blocks[0]).clone(),
        other => panic!("{other:?}"),
    };
    let hash = |inline: &Inline| {
        let mut hasher = DefaultHasher::new();
        inline.hash(&mut hasher);
        hasher.finish()
    };
    let (a, b) = (parse("- 一\n- 二\n"), parse("- 一一一\n- 二\n"));
    assert_eq!(hash(&item(&a, 1)), hash(&item(&b, 1)));
    assert_eq!(item(&b, 1).source, item(&a, 1).source + "一一".len());
    // 只改了列表记号：内容不变，块的哈希仍然要变，原文位置才跟得上。
    assert_ne!(parse("- 一\n").hashes, parse("*  一\n").hashes);
}

#[test]
fn block_ranges_cover_their_own_source() {
    let text = "> 引用\n\n---\n\n段落[^1]\n\n[^1]: 脚注\n";
    let doc = parse(text);
    let shown: Vec<&str> = doc
        .ranges
        .iter()
        .map(|r| text[r.clone()].trim_end())
        .collect();
    // 引用、分隔线、段落、脚注前自动加的分隔线（原文末尾的空范围）、脚注。
    assert_eq!(shown, ["> 引用", "---", "段落[^1]", "", "[^1]: 脚注"]);
}

#[test]
fn leaves_cover_each_editable_block() {
    let text = "# 标题\n\n段落一\n续行\n\n- 一\n- 二\n  - 三\n\n```\ncode\n```\n\n| a |\n|---|\n| b |\n\n---\n\n<div>\nhtml\n</div>\n";
    let doc = parse(text);
    let shown: Vec<&str> = doc
        .leaves
        .iter()
        .map(|l| text[l.range.clone()].trim_end())
        .collect();
    assert_eq!(
        shown,
        [
            "# 标题",
            "段落一\n续行",
            "一",
            "二",
            "三",
            "```\ncode\n```",
            "| a |\n|---|\n| b |",
            "---",
            "<div>\nhtml\n</div>",
        ]
    );
    assert_eq!(doc.len, text.len());
}

#[test]
fn deep_containers_are_flattened() {
    fn depth(blocks: &[Block]) -> usize {
        let inner = |b: &Block| match b {
            Block::Quote(_, blocks)
            | Block::Footnote(_, blocks)
            | Block::Details { blocks, .. } => 1 + depth(blocks),
            Block::List(_, items) => 1 + items.iter().map(|i| depth(&i.blocks)).max().unwrap_or(0),
            _ => 0,
        };
        blocks.iter().map(inner).max().unwrap_or(0)
    }
    // 回归：两万层 `>`、数百层缩进的列表、两万层 HTML 的 `<blockquote>`，在收缩容量、
    // 调整原文位置、比较和释放时使后台线程（2 MB）栈溢出。超过上限的部分不再嵌套，文字仍然保留。
    let lists: String = (0..300)
        .map(|i| format!("{}- x{i}\n", "  ".repeat(i)))
        .collect();
    for text in [
        format!("{} deep\n", ">".repeat(20_000)),
        lists,
        format!("{}\n\ndeep\n", "<blockquote>".repeat(20_000)),
    ] {
        let (depth, same) = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let doc = parse(&text);
                (depth(&doc.blocks), doc.blocks.clone() == doc.blocks)
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(depth <= MAX_NESTING && same, "{depth}");
    }
    let doc = parse(&format!("{} deep\n", ">".repeat(200)));
    let mut blocks = &doc.blocks;
    while let [Block::Quote(_, inner)] = blocks.as_slice() {
        blocks = inner;
    }
    assert_eq!(paragraph(&blocks[0]).text, "deep");
}

#[test]
fn toc_marker_becomes_a_nested_list_of_heading_links() {
    let md = parse("# 概述\n\n[TOC]\n\n## 安装 winc\n\n### 下载\n\n## 使用\n\n正文\n");
    let Block::List(None, items) = &md.blocks[1] else {
        panic!("[TOC] 没有换成目录：{:?}", md.blocks[1]);
    };
    // 「概述」一项，其下是「安装 winc」（含「下载」）和「使用」。
    assert_eq!(items.len(), 1);
    let link = |item: &Item| paragraph(&item.blocks[0]).links.clone();
    assert_eq!(link(&items[0]), ["#概述"]);
    let Block::List(None, children) = &items[0].blocks[1] else {
        panic!("没有子项");
    };
    assert_eq!(children.len(), 2);
    assert_eq!(link(&children[0]), ["#安装-winc"]);
    assert_eq!(link(&children[1]), ["#使用"]);
    let Block::List(None, grand) = &children[0].blocks[1] else {
        panic!("没有第三级");
    };
    assert_eq!(link(&grand[0]), ["#下载"]);

    // 没有标题时保留原文，不分大小写的 `[[toc]]` 同样识别。
    let none = parse("[TOC]\n\n正文\n");
    assert_eq!(paragraph(&none.blocks[0]).text, "[TOC]");
    let wiki = parse("# 标题\n\n[[toc]]\n");
    assert!(matches!(&wiki.blocks[1], Block::List(None, _)));
}

/// 只有记号的列表项里有一个空段落，位于记号之后，是一个叶子块：可编辑的预览中光标停在这里。
#[test]
fn empty_list_items_hold_an_empty_paragraph() {
    let source = "1. 一\n2. \n3. 三\n";
    let doc = parse(source);
    let Block::List(Some(1), items) = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks[0]);
    };
    let empty = paragraph(&items[1].blocks[0]);
    assert!(empty.text.is_empty());
    let at = source.find("2. ").unwrap() + 3;
    assert_eq!(empty.source_of(0), Some(at));
    assert!(doc.leaves.iter().any(|l| l.range == (at..at) && l.prose));
    // 分隔线记下在原文中的起点。
    let doc = parse("段落\n\n1. 甲\n2. ---\n");
    let Block::List(Some(1), items) = &doc.blocks[1] else {
        panic!("{:?}", doc.blocks[1]);
    };
    assert_eq!(items[1].blocks[0], Block::Rule(Some("1. 甲\n2. ".len())));
}

#[test]
fn block_markers_need_a_space_to_take_effect() {
    // 只有记号、后面没有空格：按文字显示，不成为列表项、标题、引用。
    for text in ["-", "1.", "#", "##", ">"] {
        let doc = parse(text);
        let block = match &doc.blocks[0] {
            Block::List(_, items) => {
                assert!(items[0].bare, "{text}");
                &items[0].blocks[0]
            }
            block => block,
        };
        assert_eq!(paragraph(block).text, text);
        assert_eq!(doc.leaves[0].range, 0..text.len(), "{text}");
    }
    // 输入空格之后生效。
    assert!(matches!(&parse("- ").blocks[0], Block::List(None, items) if !items[0].bare));
    assert!(matches!(&parse("# ").blocks[0], Block::Heading(1, _)));
    assert!(matches!(&parse("> ").blocks[0], Block::Quote(..)));
    // 有内容的项不受影响。
    assert!(matches!(&parse("- 甲\n- 乙").blocks[0], Block::List(None, items) if !items[1].bare));
}

#[test]
fn a_task_box_without_text_is_already_a_task_item() {
    for (text, checked) in [("- [ ]", false), ("- [ ] ", false), ("1. [x]", true)] {
        let doc = parse(text);
        let Block::List(_, items) = &doc.blocks[0] else {
            panic!("{text}");
        };
        assert_eq!(items[0].task, Some(checked), "{text}");
        assert_eq!(items[0].task_source, text.find('[').unwrap(), "{text}");
        assert!(paragraph(&items[0].blocks[0]).text.is_empty(), "{text}");
    }
}
