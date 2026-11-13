//! 代码块的排版：按行切分、着色和折行。

use super::*;

impl Cx<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code(
        &mut self,
        lang: &str,
        text: &str,
        map: &SourceMap,
        x: f32,
        y: f32,
        width: f32,
        out: &mut Vec<Item>,
    ) -> Flow {
        let pad = self.base;
        let size = self.base * 0.85;
        let line_h = (size * 1.45).round();
        let m = self.frame.metrics(FontKind::Mono, size);
        let room = (width - pad * 2.0).max(size);
        let language = winc_syntax::find(lang);
        let mut state = winc_syntax::LineState::default();
        let bg_at = out.len();
        let mut top = y + pad;
        let mut first_baseline = None;
        let row = Row {
            room,
            size,
            line_h,
            ascent: m.ascent,
            descent: m.descent,
        };
        let mut line_start = 0;
        for line in text.split('\n') {
            let source = map.source(line_start).and_then(|s| u32::try_from(s).ok());
            line_start += line.len() + 1;
            let source = source.unwrap_or(0);
            let piece = self.code_piece(line, source, lang, language, &mut state, row);
            first_baseline.get_or_insert(top + piece.first_baseline.unwrap_or(0.0));
            self.paste(&piece, x + pad, top, source, out);
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
        // 放在代码后面：按钮盖在文字上。
        out.push(Item::Code {
            rect,
            text: text.into(),
            diagram: false,
        });
        Flow {
            height,
            first_baseline,
        }
    }

    /// 排代码的一行（见 [`Cx::code_line`]），按内容、行首的词法状态和排法缓存。
    ///
    /// 长代码块改一行、在显示原文的大块里打字时，只重排改了的那一行。`source` 是这一行在原文
    /// 中的位置，取出后粘贴时按它挪；`state` 换成行末的词法状态。
    pub(super) fn code_piece(
        &mut self,
        line: &str,
        source: u32,
        lang: &str,
        language: Option<&'static winc_syntax::Language>,
        state: &mut winc_syntax::LineState,
        row: Row,
    ) -> Arc<memo::Piece> {
        let key = memo::key(|h| {
            line.hash(h);
            state.hash(h);
            lang.hash(h);
            row.room.to_bits().hash(h);
            row.size.to_bits().hash(h);
            row.line_h.to_bits().hash(h);
        });
        self.memo.touch(key);
        if let Some((piece, next)) = self.memo.lines.get(&key) {
            *state = *next;
            return Arc::clone(piece);
        }
        let mut piece =
            self.isolated(|cx, out| cx.code_line(line, Some(source), language, state, row, out));
        piece.source = source;
        let piece = Arc::new(piece);
        self.memo.lines.insert(key, (Arc::clone(&piece), *state));
        piece
    }

    /// 排代码块的一行（坐标从 0 起），太长时在字之间折到下一行；`source` 是这一行在原文中的
    /// 位置（相对顶层块），`state` 是行首的词法状态，排完换成行末的。
    pub(super) fn code_line(
        &mut self,
        line: &str,
        source: Option<u32>,
        language: Option<&'static winc_syntax::Language>,
        state: &mut winc_syntax::LineState,
        row: Row,
        out: &mut Vec<Item>,
    ) -> Flow {
        let chars: Vec<char> = line
            .chars()
            .map(|c| if c == '\t' { ' ' } else { c })
            .collect();
        let text_color = self.colors.text;
        // 每个字的颜色：按语法高亮。
        let mut colors = vec![text_color; chars.len()];
        if let Some(language) = language {
            let mut tokens = Vec::new();
            *state = winc_syntax::highlight_line(language, &chars, *state, &mut tokens);
            for token in &tokens {
                let color = (self.token)(token.kind);
                for c in &mut colors[token.start.min(chars.len())..token.end.min(chars.len())] {
                    *c = color;
                }
            }
        }
        // 同一行同一颜色的字合成一段，`run` 是攒着的一段：文字、每个字的宽度、在块文字里的
        // 起点、横坐标、颜色、在原文中的位置。代码行与原文逐字节相同（制表符换成的空格也是
        // 一个字节），没有不连续之处。
        let size = row.size;
        let baseline = |top: f32| top + (row.line_h - row.ascent - row.descent) / 2.0 + row.ascent;
        let flush = |out: &mut Vec<Item>,
                     run: &mut (String, Vec<f32>, usize, f32, Option<Color>, Option<u32>),
                     top: f32| {
            if !run.0.is_empty() {
                out.push(Item::Text {
                    x: run.3,
                    baseline: baseline(top),
                    text: std::mem::take(&mut run.0).into(),
                    font: FontKind::Mono,
                    faux: Faux::default(),
                    size,
                    color: run.4.unwrap_or(text_color),
                    span: Some(Span {
                        start: run.2,
                        advances: std::mem::take(&mut run.1).into(),
                        source: run.5,
                        jumps: None,
                    }),
                });
            }
            run.4 = None;
        };
        let (mut top, mut pen) = (0.0, 0.0);
        let mut run = (String::new(), Vec::new(), 0, 0.0, None, None);
        let mut byte = 0;
        for (i, &c) in chars.iter().enumerate() {
            let w = self
                .frame
                .advance_faux(FontKind::Mono, Faux::default(), size, c);
            if pen + w > row.room && pen > 0.0 {
                flush(out, &mut run, top);
                top += row.line_h;
                pen = 0.0;
            }
            if run.4.is_some_and(|rc| rc != colors[i]) {
                flush(out, &mut run, top);
            }
            if run.0.is_empty() {
                run.2 = self.chars;
                run.3 = pen;
                run.5 = source.map(|s| s + byte);
            }
            byte += c.len_utf8() as u32;
            run.4 = Some(colors[i]);
            run.0.push(c);
            run.1.push(w);
            self.text.push(c);
            self.chars += 1;
            pen += w;
        }
        flush(out, &mut run, top);
        // 空行留一段没有字的文字：光标在空行上时也有位置可画，块文字里也占一个换行。
        if chars.is_empty() {
            out.push(Item::Text {
                x: 0.0,
                baseline: baseline(0.0),
                text: "".into(),
                font: FontKind::Mono,
                faux: Faux::default(),
                size,
                color: text_color,
                span: Some(Span {
                    start: self.chars,
                    advances: Arc::new([]),
                    source,
                    jumps: None,
                }),
            });
        }
        // 块文字里空行也占一个换行。
        self.text.push('\n');
        self.chars += 1;
        Flow {
            height: top + row.line_h,
            first_baseline: Some(baseline(0.0)),
        }
    }
}
