//! 解析完成后收缩解析结果的容量，见 [`shrink`]。

use super::{Block, Inline};

/// 将解析结果中每个 `Vec`、`String` 的容量收缩到恰好容纳其内容。
///
/// 逐项追加时容量按倍数增长，结果保留到下一次解析，多余容量一直闲置。一段文字的样式段和原文
/// 对照大多只有一两项，却按最少 4 项分配；收缩后约 1 MB 文档的解析结果由约 15 MB 降至约 6 MB。
pub(super) fn shrink(blocks: &mut Vec<Block>) {
    blocks.shrink_to_fit();
    for block in blocks {
        match block {
            Block::Paragraph(inline) | Block::Heading(_, inline) => shrink_inline(inline),
            Block::Code { lang, text, map } => {
                lang.shrink_to_fit();
                text.shrink_to_fit();
                map.0.shrink_to_fit();
            }
            Block::Quote(_, blocks) | Block::Footnote(_, blocks) => shrink(blocks),
            Block::List(_, items) => {
                items.shrink_to_fit();
                for item in items {
                    shrink(&mut item.blocks);
                }
            }
            Block::Table(aligns, rows, _) => {
                aligns.shrink_to_fit();
                rows.shrink_to_fit();
                for row in rows {
                    row.shrink_to_fit();
                    row.iter_mut().for_each(shrink_inline);
                }
            }
            Block::Math(tex, _) => tex.shrink_to_fit(),
            Block::Rule(_) => {}
            Block::Details {
                summary, blocks, ..
            } => {
                shrink_inline(summary);
                shrink(blocks);
            }
        }
    }
}

fn shrink_inline(inline: &mut Inline) {
    inline.ids.shrink_to_fit();
    inline.text.shrink_to_fit();
    inline.runs.shrink_to_fit();
    inline.links.shrink_to_fit();
    inline.images.shrink_to_fit();
    inline.map.0.shrink_to_fit();
}
