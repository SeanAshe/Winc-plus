//! 标题下面的一节（预览中按标题收起），以及脚注的文字（悬停在编号上时提示）。

use std::collections::HashMap;

use super::*;

/// 标题下面可以收起的一节：标题的级别、区分同名标题的键，以及这一节到哪个顶层块为止（不含）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// 标题的级别，1 至 6。
    pub level: u8,
    /// 标题的锚点（[`slug`]），同名的第二个起加上 `-1`、`-2`。
    pub key: String,
    /// 这一节之后的第一个顶层块：下一个同级或更高级的标题，或者文末的脚注。
    pub end: usize,
}

impl Markdown {
    /// 各顶层块作为标题时下面的一节；不是标题，或者下面没有内容时为 `None`。
    pub fn sections(&self) -> Vec<Option<Section>> {
        // 脚注和它前面生成的分隔线排在正文之后，不属于任何一节。
        let mut main = (self.blocks.iter())
            .position(|b| matches!(b, Block::Footnote(..)))
            .unwrap_or(self.blocks.len());
        if main > 0 && matches!(self.blocks[main - 1], Block::Rule(None)) {
            main -= 1;
        }
        let keys = self.heading_keys();
        (self.blocks.iter().zip(keys).enumerate())
            .map(|(i, (block, key))| {
                let (Block::Heading(level, _), Some(key)) = (block, key) else {
                    return None;
                };
                let next = (self.blocks[i + 1..main].iter())
                    .position(|b| matches!(b, Block::Heading(l, _) if l <= level))
                    .map_or(main, |j| i + 1 + j);
                (i < main && next > i + 1).then_some(Section {
                    level: *level,
                    key,
                    end: next,
                })
            })
            .collect()
    }

    /// 返回各顶层块作为标题时的锚点，同名的第二个起加上 `-1`、`-2`；不是标题时为 `None`。
    pub fn heading_keys(&self) -> Vec<Option<String>> {
        let mut seen: HashMap<String, usize> = HashMap::new();
        (self.blocks.iter())
            .map(|block| {
                let Block::Heading(_, inline) = block else {
                    return None;
                };
                let slug = slug(&inline.text);
                let n = seen.entry(slug.clone()).or_default();
                let key = if *n == 0 { slug } else { format!("{slug}-{n}") };
                *n += 1;
                Some(key)
            })
            .collect()
    }

    /// 返回有文字的顶层标题：级别、文字和锚点（见 [`Markdown::heading_keys`]）。
    pub fn headings(&self) -> Vec<(u8, String, String)> {
        (self.blocks.iter().zip(self.heading_keys()))
            .filter_map(|(block, key)| match block {
                Block::Heading(level, inline) if !inline.text.trim().is_empty() => {
                    Some((*level, inline.text.trim().to_owned(), key?))
                }
                _ => None,
            })
            .collect()
    }

    /// 编号为 `number` 的脚注的文字（去掉末尾跳回正文的 ↩）；没有这条脚注时为 `None`。
    pub fn footnote_text(&self, number: &str) -> Option<String> {
        let blocks = self.blocks.iter().find_map(|block| match block {
            Block::Footnote(n, blocks) if n == number => Some(blocks),
            _ => None,
        })?;
        let mut text = String::new();
        plain_text(blocks, &mut text);
        Some(text.trim_end().trim_end_matches('↩').trim_end().to_owned())
    }
}

/// 把 `blocks` 中的文字依次接到 `out` 后面，块之间隔一个空格。
fn plain_text(blocks: &[Block], out: &mut String) {
    fn push(out: &mut String, text: &str) {
        if !out.is_empty() && !text.is_empty() {
            out.push(' ');
        }
        out.push_str(text.trim());
    }
    for block in blocks {
        match block {
            Block::Paragraph(inline) | Block::Heading(_, inline) => push(out, &inline.text),
            Block::Code { text, .. } | Block::Math(text, _) => push(out, text),
            Block::Quote(_, blocks) | Block::Footnote(_, blocks) => plain_text(blocks, out),
            Block::Details {
                summary, blocks, ..
            } => {
                push(out, &summary.text);
                plain_text(blocks, out);
            }
            Block::List(_, items) => {
                for item in items {
                    plain_text(&item.blocks, out);
                }
            }
            Block::Table(_, rows, _) => {
                for cell in rows.iter().flatten() {
                    push(out, &cell.text);
                }
            }
            _ => {}
        }
    }
}
