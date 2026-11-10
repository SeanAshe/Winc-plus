//! 可编辑预览中光标所在段落或标题的排版：按样式显示（比例字体、粗体、链接），只显示光标
//! 接触到的元素的 Markdown 标记（`**`、`[文字](网址)` 的括号和网址、`` ` ``、`#`），标记以次要
//! 色排版。单击或移动光标时段落外观保持不变，阅读不受干扰；光标进入粗体后，两侧的 `**` 才显示。
//!
//! 按当前原文解析该段落，不等待后台解析，输入快于解析时结果仍然正确。显示的文字逐字对应原文，
//! 标记同样有原文位置，光标、选区和删除都作用于原文。

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};

use super::{Block, Inline, Run, SourceMap, Style, options, parse};

/// 返回光标所在的段落或标题按原文 `text` 生成的块。
///
/// `text` 是叶子块的原文，续行带有引用的 `>` 和列表的缩进；`caret` 是光标相对 `text` 的
/// 字节偏移，为 `None` 时不显示标记；`insert` 是输入法组字，排在光标处并加下划线，没有原文位置。
/// 对照以 `text` 的开头为准（`Inline::source` 为 0）；原文已不是段落时逐字显示，不带样式。
pub fn reveal(text: &str, caret: Option<usize>, insert: Option<&str>) -> Block {
    let lines = Lines::strip(text);
    let plain = &lines.plain;
    // 去掉引用标记后没有内容（只输入了 `>`）：逐字显示原文，光标仍有位置可落。
    if plain.trim().is_empty() && !text.trim().is_empty() {
        let mut out = Out::default();
        for (i, c) in text.trim_end_matches(['\n', '\r']).char_indices() {
            out.push(c, Style::default(), Some(i));
        }
        return Block::Paragraph(Inline {
            text: out.text,
            runs: out.runs,
            map: out.map,
            ..Inline::default()
        });
    }
    let parsed = parse(plain);
    let (heading, inline) = match parsed.blocks.as_slice() {
        [Block::Paragraph(inline)] => (None, inline),
        [Block::Heading(level, inline)] => (Some(*level), inline),
        _ => return Block::Paragraph(lines.verbatim()),
    };
    let base = parsed.ranges.first().map_or(0, |r| r.start) + inline.source;
    let elements = elements(plain);
    let mut out = Out {
        insert: caret.zip(insert).map(|(at, text)| (at, text.to_owned())),
        ..Out::default()
    };
    let caret = caret.map(|c| lines.to_plain(c));
    let anchors = &inline.map.0;
    // 已写入的文字和已处理的原文（相对 `plain`）的末尾位置。
    let (mut t, mut s) = (0, 0);
    let emit = |out: &mut Out, gap: Range<usize>, made: Range<usize>| {
        let generated = &inline.text[made.clone()];
        // 由原文生成的文字（emoji、脚注编号）之前的标记单独成为一处：光标接触时显示原文，
        // 不显示生成的文字。
        let own = !generated.trim().is_empty();
        let shown = out.gap(plain, gap, &elements, caret, own, &lines);
        let image = generated.contains(super::IMAGE_MARK);
        if shown && image && !out.text.ends_with('\n') {
            // 显示图片的原文时，图片另起一行排在原文下方，使原文与图片都完整可见。
            out.push('\n', Style::default(), None);
        }
        if !(shown && own) || image {
            out.copy(inline, made, None, &elements, caret);
        }
    };
    for a in anchors {
        let (at, src, len) = (a.text as usize, base + a.source as usize, a.len as usize);
        emit(&mut out, s..src.max(s), t..at.max(t));
        t = t.max(at);
        if len > 0 {
            out.copy(inline, at..at + len, Some((src, &lines)), &elements, caret);
            t = at + len;
            s = s.max(src + len);
        }
    }
    emit(&mut out, s..plain.len(), t..inline.text.len());
    out.flush();
    let inline = Inline {
        text: out.text,
        runs: out.runs,
        links: inline.links.clone(),
        images: inline.images.clone(),
        align: inline.align,
        map: out.map,
        source: 0,
    };
    match heading {
        Some(level) => Block::Heading(level, inline),
        None => Block::Paragraph(inline),
    }
}

/// 带标记的行内元素在 `plain` 中的字节范围。
///
/// 包括强调、粗体、删除线、链接、图片、行内代码、公式、HTML 标签和脚注引用。
fn elements(plain: &str) -> Vec<Range<usize>> {
    Parser::new_ext(plain, options())
        .into_offset_iter()
        .filter(|(event, _)| {
            matches!(
                event,
                Event::Start(
                    Tag::Emphasis
                        | Tag::Strong
                        | Tag::Strikethrough
                        | Tag::Link { .. }
                        | Tag::Image { .. }
                ) | Event::Code(_)
                    | Event::InlineMath(_)
                    | Event::DisplayMath(_)
                    | Event::InlineHtml(_)
                    | Event::FootnoteReference(_)
            )
        })
        .map(|(_, range)| range)
        .collect()
}

/// 返回包含位置 `at` 的最内层元素。
fn innermost(elements: &[Range<usize>], at: usize) -> Option<&Range<usize>> {
    elements
        .iter()
        .filter(|e| e.contains(&at))
        .min_by_key(|e| e.len())
}

/// 返回光标是否接触 `range`：位于其中或紧贴其两端。
fn touches(range: &Range<usize>, caret: Option<usize>) -> bool {
    caret.is_some_and(|c| range.start <= c && c <= range.end)
}

/// 去掉续行开头引用 `>` 和缩进后的原文，以及它与原文的对应关系。
struct Lines {
    plain: String,
    /// 各行在 `plain` 和原文中的起点（字节偏移）。
    starts: Vec<(usize, usize)>,
}

impl Lines {
    /// 返回去掉续行开头 `>` 和缩进后的文字，并记下各行在两个字符串中的起点。
    fn strip(text: &str) -> Lines {
        let mut plain = String::with_capacity(text.len());
        let mut starts = Vec::new();
        let mut start = 0;
        for (i, line) in text.split('\n').enumerate() {
            // 段落续行开头的 `>` 只能视为引用标记，否则该行将开始一个新的引用。
            let skip = if i == 0 {
                0
            } else {
                plain.push('\n');
                let mut at = 0;
                let bytes = line.as_bytes();
                while let Some(&b) = bytes.get(at) {
                    if !matches!(b, b' ' | b'\t' | b'>') {
                        break;
                    }
                    at += 1;
                }
                at
            };
            starts.push((plain.len(), start + skip));
            plain.push_str(&line[skip..]);
            start += line.len() + 1;
        }
        Lines { plain, starts }
    }

    /// 返回 `plain` 中的位置在原文中的位置；行间的换行对应原文的换行。
    fn to_text(&self, at: usize) -> usize {
        let i = self
            .starts
            .partition_point(|&(p, _)| p <= at)
            .saturating_sub(1);
        let (p, t) = self.starts[i];
        t + (at - p)
    }

    /// 返回原文中的位置在 `plain` 中的位置；位于已去掉的行首标记中时取该行的开头。
    fn to_plain(&self, at: usize) -> usize {
        let i = self.starts.partition_point(|&(_, t)| t <= at);
        let Some(line) = i.checked_sub(1) else {
            return 0;
        };
        let (p, t) = self.starts[line];
        let next = self.starts.get(i).map_or(usize::MAX, |&(p, _)| p);
        (p + (at - t)).min(next)
    }

    /// 返回按原文逐字显示、保留换行且不带样式的文字。
    fn verbatim(&self) -> Inline {
        let mut out = Out::default();
        let text = self.plain.trim_end_matches(['\n', '\r']);
        for (i, c) in text.char_indices() {
            if c != '\r' {
                out.push(c, Style::default(), Some(self.to_text(i)));
            }
        }
        Inline {
            text: out.text,
            runs: out.runs,
            map: out.map,
            ..Inline::default()
        }
    }
}

/// 拼接生成的文字。
#[derive(Default)]
struct Out {
    text: String,
    runs: Vec<Run>,
    map: SourceMap,
    /// 尚未写入的输入法组字及其在原文中的位置；写入到原文位置不小于该值的第一个字符之前。
    insert: Option<(usize, String)>,
}

impl Out {
    fn push(&mut self, c: char, style: Style, source: Option<usize>) {
        if source.is_some_and(|s| self.insert.as_ref().is_some_and(|(at, _)| s >= *at)) {
            self.flush();
        }
        let at = self.text.len();
        self.text.push(c);
        match self.runs.last_mut() {
            Some(run) if run.style == style => run.end = self.text.len(),
            _ => self.runs.push(Run {
                end: self.text.len(),
                style,
            }),
        }
        if let Some(source) = source {
            self.map.push(at, source, c.len_utf8());
        }
    }

    /// 写入输入法组字（加下划线，没有原文位置）。
    fn flush(&mut self) {
        if let Some((_, text)) = self.insert.take() {
            let style = Style {
                flags: Style::UNDERLINE,
                link: 0,
            };
            for c in text.chars() {
                self.push(c, style, None);
            }
        }
    }

    /// 写入原有的文字 `range`（样式不变）；`source` 是 `range` 在 `plain` 中的起点及对照。
    ///
    /// 由原文生成的文字没有起点；光标接触到的公式显示 LaTeX 原文。
    fn copy(
        &mut self,
        inline: &Inline,
        range: Range<usize>,
        source: Option<(usize, &Lines)>,
        elements: &[Range<usize>],
        caret: Option<usize>,
    ) {
        let mut runs = inline.runs().skip_while(|(r, _)| r.end <= range.start);
        let mut run = runs.next();
        for (i, c) in inline.text[range.clone()].char_indices() {
            let at = range.start + i;
            while let Some((r, _)) = &run
                && r.end <= at
            {
                run = runs.next();
            }
            let mut style = run.as_ref().map_or(Style::default(), |(_, s)| *s);
            let plain = source.map(|(s, _)| s + i);
            if style.has(Style::MATH)
                && plain
                    .and_then(|p| innermost(elements, p))
                    .is_some_and(|e| touches(e, caret))
            {
                style.flags = (style.flags & !Style::MATH) | Style::CODE;
            }
            self.push(c, style, source.map(|(s, lines)| lines.to_text(s + i)));
        }
    }

    /// 处理原文中未显示为文字的一段 `gap`（相对 `plain`），返回其中是否显示了内容。
    ///
    /// 光标接触的元素显示其标记，其余标记和换行不显示（软换行生成的空格已在文字中）。`own` 为真时，
    /// 不属于任何元素的标记单独成为一处（emoji 短码）；否则按行拆分，每段单独成为一处
    /// （标题的 `#`、转义的 `\`）。
    fn gap(
        &mut self,
        plain: &str,
        gap: Range<usize>,
        elements: &[Range<usize>],
        caret: Option<usize>,
        own: bool,
        lines: &Lines,
    ) -> bool {
        let mut shown = false;
        let text = &plain[gap.clone()];
        // 不属于任何元素的一段，截止到换行。
        let piece = |i: usize| {
            let from = text[..i].rfind('\n').map_or(0, |n| n + 1);
            let to = text[i..].find('\n').map_or(text.len(), |n| i + n);
            if own { 0..text.len() } else { from..to }
        };
        for (i, c) in text.char_indices() {
            if matches!(c, '\n' | '\r') {
                continue;
            }
            let at = gap.start + i;
            let show = match innermost(elements, at) {
                Some(e) => touches(e, caret),
                None => {
                    let p = piece(i);
                    let p = gap.start + p.start..gap.start + p.end;
                    // 行首、行尾的空白（续行的缩进、硬换行的两个空格）不视为标记。
                    !plain[p.clone()].trim().is_empty() && touches(&p, caret)
                }
            };
            if show {
                let style = Style {
                    flags: Style::SYNTAX,
                    link: 0,
                };
                self.push(c, style, Some(lines.to_text(at)));
                shown = true;
            }
        }
        shown
    }
}

#[cfg(test)]
mod tests;
