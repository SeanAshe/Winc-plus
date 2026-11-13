//! 原样显示的块：实时编辑时光标所在的块按源文字排版，可以带预览、分隔线或边框。

use super::*;

impl Cx<'_, '_> {
    /// `block` 是否在光标所在的叶子块里（按块里第一处有原文对照的位置判断）。
    pub(super) fn raw_state(&mut self, block: &Block) -> RawState {
        let Some(raw) = &mut self.raw else {
            return RawState::Laid;
        };
        let start = match block {
            Block::Paragraph(inline) | Block::Heading(_, inline) => {
                (!inline.map.is_empty()).then_some(inline.source)
            }
            Block::Code { map, .. } => map.source(0),
            Block::Rule(at) => *at,
            Block::Table(_, rows, _) => rows
                .iter()
                .flatten()
                .find(|cell| !cell.map.is_empty())
                .map(|cell| cell.source),
            _ => None,
        };
        match start {
            Some(start) if raw.leaf.start <= start && start <= raw.parsed_end => {
                if std::mem::replace(&mut raw.done, true) {
                    // HTML 块生成的其余各块排在代码框下方，作为该块的渲染结果。
                    return match raw.kind {
                        RawKind::Html => RawState::Laid,
                        _ => RawState::Skip,
                    };
                }
                // 表格照常排，只有光标所在的那一格显示原文。
                if let Block::Table(_, rows, _) = block
                    && let Some(cell) = self.live_cell(rows)
                {
                    self.cell = Some(cell);
                    return RawState::Laid;
                }
                if matches!(block, Block::Paragraph(_) | Block::Heading(..))
                    && let Some(raw) = self.raw.as_ref().filter(|r| r.kind == RawKind::Prose)
                {
                    let (revealed, blanks, placed) = reveal(raw);
                    self.preedit |= placed;
                    return RawState::Reveal(Box::new(revealed), blanks);
                }
                if matches!(block, Block::Rule(_)) {
                    return RawState::Rule;
                }
                RawState::Show
            }
            _ => RawState::Laid,
        }
    }

    /// 返回光标所在的表格（`rows` 是解析出的各行）里光标所在的单元格。
    ///
    /// 按现在的原文切分（见 `winc_preview::markdown::cells`）；光标在表头下面那一行上、原文的
    /// 行数或列数与解析结果对不上（加了一行、一列还没解析）时为 `None`，整张表显示原文。
    pub(super) fn live_cell(&self, rows: &[Vec<Inline>]) -> Option<LiveCell> {
        use winc_preview::markdown::cells;
        let raw = self.raw.as_ref()?;
        let text = raw.block.get(raw.leaf.clone())?;
        let mut start = raw.leaf.start;
        let mut row = 0;
        let mut found = None;
        for line in text.split('\n') {
            let end = start + line.len();
            let here = (start..=end).contains(&raw.caret);
            if cells::is_delimiter(line) {
                if here {
                    return None;
                }
            } else if !line.trim().is_empty() {
                if here {
                    let cells = cells::cells(line);
                    let col = cells
                        .iter()
                        .position(|c| raw.caret - start <= c.segment.end)?;
                    let segment = &cells[col].segment;
                    found = Some((row, col, start + segment.start..start + segment.end));
                }
                row += 1;
            }
            start = end + 1;
        }
        let (r, c, range) = found?;
        (row == rows.len() && c < rows[r].len()).then_some((r, c, range))
    }

    /// 排版光标所在叶子块的原文。
    ///
    /// 公式、mermaid 图和 HTML 的下方另外绘制渲染结果，编辑时可以直接查看效果。
    /// 渲染结果来自最近一次的解析结果，可能比原文滞后一个解析周期。
    pub fn raw_with_preview(
        &mut self,
        block: &Block,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Flow {
        let flow = self.raw(x, y, width, &mut out.items);
        let boxed = self
            .raw
            .as_ref()
            .is_some_and(|r| matches!(r.kind, RawKind::Fenced(_) | RawKind::Html));
        let gap = (self.base * 0.75).round();
        // 代码框自带底色，与下方的渲染结果之间不再绘制分隔线。
        let below = if boxed {
            y + flow.height + gap
        } else {
            y + flow.height + gap * 2.0 + 1.0
        };
        let html = self.raw.as_ref().is_some_and(|r| r.kind == RawKind::Html);
        let preview = match block {
            Block::Math(..) => self.preview(block, x, below, width, env, out),
            Block::Code { lang, text, .. } if lang == "mermaid" => {
                self.diagram(text, x, below, width, env, &mut out.items)
            }
            _ if html => self.preview(block, x, below, width, env, out),
            _ => None,
        };
        let Some(preview) = preview.filter(|p| p.height > 0.0) else {
            return flow;
        };
        if !boxed {
            // 原文与渲染结果之间绘制一条细分隔线。
            out.items.push(Item::Fill {
                rect: Rect::new(x, y + flow.height + gap, width, 1.0),
                color: self.colors.border,
                radius: 0.0,
            });
        }
        Flow {
            height: below + preview.height - y,
            first_baseline: flow.first_baseline,
        }
    }

    /// 在 `(x, y)` 处按排版显示 `block`（不显示原文），作为原文下方的渲染结果。
    pub(super) fn preview(
        &mut self,
        block: &Block,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Out,
    ) -> Option<Flow> {
        let raw = self.raw.take();
        let flow = self.block(block, x, y, width, env, out);
        self.raw = raw;
        Some(flow)
    }

    /// 排版光标所在叶子块的原文（见 [`Raw`]）。
    ///
    /// 代码、公式、HTML 放入代码框（见 `Cx::raw_box`）；其余块使用等宽字体并按 Markdown 着色，
    /// 保留原有换行，过长时在字符之间折行。每段文字的原文位置即其在原文中的位置。
    pub fn raw(&mut self, x: f32, y: f32, width: f32, out: &mut Vec<Item>) -> Flow {
        let Some(raw) = &self.raw else {
            return Flow {
                height: 0.0,
                first_baseline: None,
            };
        };
        match &raw.kind {
            RawKind::Fenced(lang) => {
                let lang = lang.clone();
                return self.raw_box(&lang, x, y, width, out);
            }
            RawKind::Html => return self.raw_box("html", x, y, width, out),
            RawKind::Prose | RawKind::Plain => {}
        }
        let (block, leaf) = (Arc::clone(&raw.block), raw.leaf.clone());
        let text = block.get(leaf.clone()).unwrap_or_default();
        let size = self.base * 0.85;
        let line_h = (size * 1.45).round();
        let m = self.frame.metrics(FontKind::Mono, size);
        let row = Row {
            room: width.max(size),
            size,
            line_h,
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let (mut top, mut first_baseline) = (y, None);
        let mut start = leaf.start;
        for line in text.split('\n') {
            let source = u32::try_from(start).unwrap_or(0);
            start += line.len() + 1;
            let piece = self.code_piece(line, source, "markdown", language, &mut state, row);
            first_baseline.get_or_insert(top + piece.first_baseline.unwrap_or(0.0));
            self.paste(&piece, x, top, source, out);
            top += piece.height;
        }
        Flow {
            height: top - y,
            first_baseline,
        }
    }

    /// 排版光标所在的分隔线：原文（`---`）以次要色写在左侧，分隔线从原文之后画到右边缘。
    ///
    /// 高度与排版后的分隔线相同，原文的一行在分隔线上下的外边距中垂直居中：光标移入、移出
    /// 分隔线时其下方的内容不移动，修改原文时仍可看出这是一条分隔线。
    pub(super) fn raw_rule(&mut self, x: f32, y: f32, width: f32, out: &mut Out) -> Flow {
        let h = rule_height(self.base);
        let flow = Flow {
            height: h,
            first_baseline: None,
        };
        let Some(raw) = &self.raw else {
            return flow;
        };
        let (block, leaf) = (Arc::clone(&raw.block), raw.leaf.clone());
        let text = block.get(leaf.clone()).unwrap_or_default();
        let line = text.split('\n').next().unwrap_or_default();
        let line = line.trim_end_matches('\r');
        let size = self.base * 0.85;
        let m = self.frame.metrics(FontKind::Mono, size);
        let row = Row {
            room: width.max(size),
            size,
            line_h: (size * 1.45).round(),
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let source = u32::try_from(leaf.start).unwrap_or(0);
        let piece = self.code_piece(line, source, "markdown", language, &mut state, row);
        let first = out.items.len();
        self.paste(
            &piece,
            x,
            y + (h - piece.height) / 2.0,
            source,
            &mut out.items,
        );
        let mut right = x;
        for item in &mut out.items[first..] {
            right = right.max(item.span_x().1);
            if let Item::Text { color, .. } = item {
                *color = self.colors.muted;
            }
        }
        let gap = (self.base * 0.6).round();
        if right + gap < x + width {
            out.items.push(Item::Fill {
                rect: Rect::new(right + gap, y, x + width - right - gap, h),
                color: self.colors.border,
                radius: 0.0,
            });
        }
        flow
    }

    /// 将光标所在的代码、公式或 HTML 块的原文放入代码框，内容按 `lang` 着色。
    ///
    /// 代码框的底色、边距、字号与排版后的代码块相同；框后光标所在的空行排在框外。围栏行只在
    /// 光标位于其上时显示，并以较淡的颜色绘制。因此点击进入代码块时外观不变、文字不移动；
    /// 用 ←→ 移出内容首尾时显示围栏，可以修改语言标识。
    pub(super) fn raw_box(
        &mut self,
        lang: &str,
        x: f32,
        y: f32,
        width: f32,
        out: &mut Vec<Item>,
    ) -> Flow {
        let Some(raw) = &self.raw else {
            return Flow {
                height: 0.0,
                first_baseline: None,
            };
        };
        let (block, leaf) = (Arc::clone(&raw.block), raw.leaf.clone());
        let fenced = matches!(raw.kind, RawKind::Fenced(_));
        let blanks = blanks(raw);
        let text = block.get(leaf.clone()).unwrap_or_default();
        let text = &text[..text.trim_end().len()];
        let pad = self.base;
        let size = self.base * 0.85;
        let m = self.frame.metrics(FontKind::Mono, size);
        let row = Row {
            room: (width - pad * 2.0).max(size),
            size,
            line_h: (size * 1.45).round(),
            ascent: m.ascent,
            descent: m.descent,
        };
        let language = winc_syntax::find(lang);
        let markdown = winc_syntax::find("markdown");
        let mut state = winc_syntax::LineState::default();
        let bg_at = out.len();
        let (mut top, mut first_baseline) = (y + pad, None);
        let mut start = leaf.start;
        let count = text.split('\n').count();
        let caret = raw.caret;
        for (i, line) in text.split('\n').enumerate() {
            let source = u32::try_from(start).unwrap_or(0);
            let here = (start..=start + line.len()).contains(&caret);
            start += line.len() + 1;
            // 第一行，以及（行数不少于两行且确为围栏时）最后一行是围栏，按 Markdown 着色。
            let fence = fenced && count > 1 && (i == 0 || (i + 1 == count && is_fence(line)));
            if fence && !here {
                continue;
            }
            let piece = if fence {
                let mut fence_state = winc_syntax::LineState::default();
                self.code_piece(line, source, "markdown", markdown, &mut fence_state, row)
            } else {
                self.code_piece(line, source, lang, language, &mut state, row)
            };
            first_baseline.get_or_insert(top + piece.first_baseline.unwrap_or(0.0));
            let at = out.len();
            self.paste(&piece, x + pad, top, source, out);
            if fence {
                // 围栏只是标记，以较淡的颜色绘制。
                let muted = self.colors.muted;
                for item in &mut out[at..] {
                    if let Item::Text { color, .. } = item {
                        *color = muted;
                    }
                }
            }
            top += piece.height;
        }
        let height = top + pad - y;
        let rect = Rect::new(x, y, width, height);
        out.insert(
            bg_at,
            Item::Fill {
                rect,
                color: self.colors.code_block,
                radius: 6.0,
            },
        );
        // 代码块在编辑时同样显示复制按钮，复制围栏之间的代码。
        if fenced && !matches!(lang, "mermaid" | "latex") {
            let lines: Vec<&str> = text.split('\n').collect();
            let close = usize::from(lines.len() > 1 && lines.last().is_some_and(|l| is_fence(l)));
            let code = lines[1.min(lines.len())..lines.len() - close].join("\n");
            out.push(Item::Code {
                rect,
                text: code.into(),
                diagram: false,
            });
        }
        let mut blank_out = Out::default();
        let extra = self.blank_lines(&blanks, x, y + height, width, &mut blank_out);
        out.append(&mut blank_out.items);
        Flow {
            height: height + extra,
            first_baseline,
        }
    }
}
