//! 目录：只有 `[TOC]`（或 `[[TOC]]`）的段落换成按标题层级嵌套的列表。

use super::{Block, Inline, Item, Style, slug, syntax};

/// 把只有 `[TOC]`（或 `[[TOC]]`，不分大小写）的段落换成目录：按文中标题的层级嵌套的列表，每一项
/// 链接到对应的标题（锚点见 [`slug`]）。`blocks` 是正文的顶层块，不含脚注。
///
/// 目录随标题生成，标题改变时所在块的内容随之改变；没有标题时保留原段落。返回是否有这样的
/// 段落，有时改动标题须重新解析全文。
pub(super) fn contents(blocks: &mut [Block]) -> bool {
    // `[[TOC]]` 解析为名字是 `toc` 的维基链接。
    let is_marker = |block: &Block| {
        let Block::Paragraph(inline) = block else {
            return false;
        };
        let wiki = matches!(&inline.links[..], [url] if url.strip_prefix(syntax::WIKI)
            .is_some_and(|name| name.eq_ignore_ascii_case("toc")));
        wiki || inline.text.trim().eq_ignore_ascii_case("[toc]")
    };
    if !blocks.iter().any(is_marker) {
        return false;
    }
    let headings: Vec<(u8, String)> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Heading(level, inline) if !inline.text.trim().is_empty() => {
                Some((*level, inline.text.clone()))
            }
            _ => None,
        })
        .collect();
    if headings.is_empty() {
        return true;
    }
    let list = Block::List(None, contents_items(&headings));
    for block in blocks.iter_mut().filter(|block| is_marker(block)) {
        *block = list.clone();
    }
    true
}

/// 把按先后排列的（级别，标题）排成嵌套的列表项：级别比前一项高的标题排在前一项下面。
fn contents_items(headings: &[(u8, String)]) -> Vec<Item> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < headings.len() {
        let (level, text) = &headings[i];
        // 级别更深的后续标题都是这一项的子项。
        let end = headings[i + 1..]
            .iter()
            .position(|(l, _)| l <= level)
            .map_or(headings.len(), |k| i + 1 + k);
        let mut inline = Inline::default();
        inline.links.push(format!("#{}", slug(text)));
        let link = u16::try_from(inline.links.len()).unwrap_or(u16::MAX);
        inline.push(
            text,
            Style {
                link,
                ..Style::default()
            },
        );
        let mut blocks = vec![Block::Paragraph(inline)];
        if end > i + 1 {
            blocks.push(Block::List(None, contents_items(&headings[i + 1..end])));
        }
        items.push(Item {
            task: None,
            task_source: 0,
            blocks,
            bare: false,
        });
        i = end;
    }
    items
}
