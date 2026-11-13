//! 文本区的一行：行号栏中的部分（见 `gutter`）、缩进参考线、各类标注、文字、折叠标记和光标。

use winc_config::RenderWhitespace;
use winc_i18n::t;

use super::*;
use crate::app::vim::CaretShape;

/// 查找光标所在词的其他出现时，在可见列两侧额外检查的字符数（字符）。
///
/// 只需覆盖跨越屏幕边缘的一个词；更长的标识符在边缘处可能被当作不完整的词，
/// 仅影响边缘处的标记。
const OCCURRENCE_MARGIN: usize = 256;

impl Screen<'_> {
    /// 绘制行 `line`：`line_start` 是行首的字符偏移，`content` 是本行的文字，`state` 是行首的
    /// 高亮状态，绘制后更新为下一行行首的状态。
    pub(super) fn draw_line(
        &self,
        frame: &mut Frame<'_>,
        scratch: &mut Scratch,
        line: usize,
        line_start: usize,
        content: &str,
        state: &mut LineState,
    ) {
        let Scratch {
            chars,
            cells: column_cells,
            fields: column_fields,
            tokens,
            splits,
            number,
            guide_indent,
            folded_hits,
            long_hits,
            ime_cursor,
        } = scratch;
        let (text_rect, lh) = (self.text_rect, self.lh);
        // 第一段的顶边和基线按 `f64` 计算（见 `Screen::line_y`），基线的算法同 `baseline_in`。
        let y = self.line_y(line);
        let metrics = self.app.metrics.mono;
        let lead = (lh - (metrics.ascent + metrics.descent)) / 2.0 + metrics.ascent;
        let baseline = (y + f64::from(lead)).round();
        // `head` 为行首最多 `MAX_LINE_CHARS` 个字符：着色只处理这一部分，多数行的内容全在其中。
        chars.clear();
        chars.extend(content.chars().take(MAX_LINE_CHARS + 1));
        // 未读到行尾的行同样视为超长：其后的内容未知。
        let long = chars.len() > MAX_LINE_CHARS || cut_by(content, self.read_cap);
        chars.truncate(MAX_LINE_CHARS);
        let head = &chars[..];
        // 按整行断开且 `content` 不是整行时，改取屏幕上可见的若干段（行内窗口）；
        // 其余行绘制 `head`。
        let rows = self.doc.rows_of(line);
        let window = (rows > 1 && reads_head_only(content))
            .then(|| {
                let parts = visible_parts(y, rows, text_rect, lh);
                self.doc.line_window(line, parts)
            })
            .flatten();
        // 不换行时显示整行的超长行只取屏幕上可见的列。
        let span = (long && !self.doc.is_wrapping() && self.columns.is_none())
            .then(|| {
                let from = ((text_rect.x - self.origin_x) / self.cell).floor().max(0.0);
                let to = ((text_rect.right() - self.origin_x) / self.cell)
                    .ceil()
                    .max(0.0);
                let cells = from as usize..to as usize + 1;
                self.doc.long_line_span(line, cells, self.tab_width)
            })
            .flatten();
        let (chars, slice, breaks, truncated) = match (&window, &span) {
            (Some(w), _) => (&w.chars[..], &w.text[..], w.breaks.clone(), false),
            (None, Some(s)) => (&s.chars[..], &s.text[..], Default::default(), false),
            (None, None) => {
                let breaks = fit_breaks(self.doc.line_breaks(line), head.len());
                (head, content, breaks, long)
            }
        };
        let (first_part, first_col, ends) = match (&window, &span) {
            (Some(w), _) => (w.part, w.col, w.ends),
            (None, Some(s)) => (0, s.col, s.ends),
            (None, None) => (0, 0, true),
        };
        let (x0, base) = match &span {
            Some(s) => {
                let base = s.cell % self.tab_width.max(1);
                let before = (s.cell - base) as f64 * f64::from(self.cell);
                ((f64::from(self.origin_x) + before) as f32, base)
            }
            None => (self.origin_x, 0),
        };
        let slice = RopeSlice::from(slice);
        let down = first_part as f64 * f64::from(lh);
        let top = (y + down) as f32;
        let parts = visible_parts(f64::from(top), breaks.rows(), text_rect, lh);

        let start = line_start + first_col;
        let line_end = line_end(self.doc.text(), line, start, chars.len(), truncated);
        // 按列对齐时，每个字符所在的单元格位置和所属的列。
        let aligned = self.columns.map(|c| {
            let fields = Some(&mut *column_fields);
            c.layout(slice, self.tab_width, chars.len(), column_cells, fields);
            (&column_cells[..], &column_fields[..], c.separator)
        });

        // 行号、书签、折叠箭头绘制在行号栏中该行的位置。断成多段的行滚动到只显示后续各段时，
        // 第一段已超出编辑区上沿，因此下移到顶部绘制，使行号仍对应本行。下移量最多到最后一段：
        // 只有一段的行仅上半部分滚出，最后一段只显示下半部分时同理，两种情况下行号都不得与
        // 下一行的行号重叠。
        let gutter_rows = rows.max(first_part + breaks.rows());
        let gutter_room = (gutter_rows - 1) as f64 * f64::from(lh);
        let gutter_shift = (f64::from(text_rect.y) - y).clamp(0.0, gutter_room);
        let row = Row {
            line,
            origin: line_start,
            line_start: start,
            line_end,
            first_part,
            ends,
            x0,
            base,
            y: top,
            baseline: (baseline + down) as f32,
            gutter_y: (y + gutter_shift) as f32,
            gutter_baseline: (baseline + gutter_shift) as f32,
            slice,
            head: RopeSlice::from(content),
            chars,
            truncated,
            parts,
            breaks,
            aligned,
            collapsed: self.doc.folds().is_collapsed(line),
            prefix: std::cell::OnceCell::new(),
        };

        self.draw_number(frame, &row, number);
        self.draw_change_marks(frame, &row);
        self.draw_blame(frame, &row);
        self.draw_bookmark(frame, &row);
        self.draw_fold_arrow(frame, &row);
        self.draw_guides(frame, &row, guide_indent);
        self.draw_marks(frame, &row);

        // 只为前 `MAX_LINE_CHARS` 个字符着色，其后按普通文本绘制。超长行不改变下一行的高亮状态，
        // 与高亮状态推算的规则相同（截断处可能位于字符串中间）；不换行时只需着色到编辑区右边缘。
        let mut colored = head.len();
        if long && row.breaks.rows() == 1 {
            let shown = ((self.text_rect.right() - self.origin_x) / self.cell)
                .ceil()
                .max(0.0);
            colored = colored.min(shown as usize + 1);
        }
        if first_col < colored {
            let (doc, language) = (self.doc, self.language);
            let next =
                (self.app).highlight_cached(doc, line, language, &head[..colored], *state, tokens);
            if !long {
                *state = next;
            }
            // 词法单元的位置相对行首，换算为相对 `chars` 的开头。
            if first_col > 0 {
                tokens.retain_mut(|token| {
                    token.start = token.start.saturating_sub(first_col);
                    token.end = token.end.saturating_sub(first_col);
                    token.end > 0
                });
            }
        } else {
            tokens.clear();
        }
        self.ligature_splits(&row, splits);
        self.draw_whitespace(frame, &row);
        let cells = self.draw_content(frame, &row, tokens, splits);
        self.draw_tail(frame, &row, cells, (folded_hits, long_hits));
        self.draw_carets(frame, &row, ime_cursor);
    }

    /// 返回位置 `pos` 在本行中的段号（从 0 起）和 x 坐标。
    fn place(&self, row: &Row<'_>, pos: usize) -> (usize, f32) {
        let (origin_x, cell, tab_width) = (self.origin_x, self.cell, self.tab_width);
        let col = (pos - row.line_start).min(row.chars.len());
        if row.breaks.rows() > 1 {
            let part = row.breaks.row_of(col);
            let cells = winc_core::wrap::cell_in_row(row.slice, &row.breaks, col, tab_width);
            return (part, origin_x + cells as f32 * cell);
        }
        let cells = match &row.aligned {
            Some((positions, ..)) => positions[col.min(positions.len() - 1)],
            None => row.prefix.get_or_init(|| {
                let mut cells = text::prefix_cells(row.chars, tab_width);
                // 制表符的宽度取决于所在的显示列，不从 0 起时逐个计算。
                if row.base > 0 {
                    let mut at = row.base;
                    cells[0] = at;
                    for (cell, &c) in cells[1..].iter_mut().zip(row.chars) {
                        at += text::char_cells(c, at, tab_width);
                        *cell = at;
                    }
                }
                cells
            })[col],
        };
        (0, row.x0 + cells as f32 * cell)
    }

    /// 对文本范围 `start..end` 在屏幕上占据的每一块调用 `f(y, x0, x1)`；跨段时每段一块。
    fn spans(&self, row: &Row<'_>, start: usize, end: usize, f: &mut dyn FnMut(f32, f32, f32)) {
        if row.breaks.rows() == 1 {
            f(row.y, self.place(row, start).1, self.place(row, end).1);
            return;
        }
        let (origin_x, cell, lh, tab_width) = (self.origin_x, self.cell, self.lh, self.tab_width);
        let (chars, breaks) = (row.chars, &row.breaks);
        let cols = (
            start.saturating_sub(row.line_start),
            end.saturating_sub(row.line_start),
        );
        wrapped_spans(chars, breaks, cols, tab_width, |part, a, b| {
            let x = |cells: usize| origin_x + cells as f32 * cell;
            f(row.y + part as f32 * lh, x(a), x(b));
        });
    }

    /// 绘制缩进参考线：每级缩进一条，空行沿用上一行的缩进 `guide_indent`。
    fn draw_guides(&self, frame: &mut Frame<'_>, row: &Row<'_>, guide_indent: &mut usize) {
        let breaks = &row.breaks;
        if !text::is_blank(row.head) {
            *guide_indent = text::indent_cells(row.head, self.tab_width);
        }

        for level in (0..*guide_indent).step_by(self.indent_unit.max(1)) {
            let x = (self.origin_x + level as f32 * self.cell).round();
            // 折成多段且后续各段与第一段缩进相同时，参考线绘制到最后一段；后续各段不缩进时
            // 只绘制在整行的第一段上。
            let spanned = match (level < breaks.indent, row.first_part) {
                (true, _) => breaks.rows(),
                (false, 0) => 1,
                (false, _) => 0,
            };
            let height = self.lh * spanned as f32;
            let guide = Rect::new(x, row.y, 1.0, height);
            let active = self
                .active_guide
                .as_ref()
                .is_some_and(|(at, lines)| *at == level && lines.contains(&row.line));
            let p = self.app.palette;
            let color = if active {
                p.indent_guide_active
            } else {
                p.indent_guide
            };
            frame.rect_clipped(guide, color, 0.0, self.text_rect);
        }
    }

    /// 绘制查找结果、光标所在词的其他出现和配对的括号。
    fn draw_marks(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let (app, p, lh, text_rect) = (self.app, self.app.palette, self.lh, self.text_rect);
        let (line_start, line_end) = (row.line_start, row.line_end);
        let shown = row.shown_cols();
        if self.show_matches {
            let current = app.bars.search.current_match();

            // 只取可见列内的结果；可见段延伸到行尾时取到行尾，包括截断后未显示的部分。
            let to = if shown.end == row.chars.len() {
                line_end
            } else {
                line_start + shown.end
            };
            for &(start, end) in app.bars.search.matches_in(line_start + shown.start, to) {
                let color = if current == Some((start, end)) {
                    p.search_current
                } else {
                    p.search_match
                };
                self.spans(
                    row,
                    start.max(line_start),
                    end.min(line_end),
                    &mut |top, x0, x1| {
                        let rect = Rect::new(x0, top + 1.0, (x1 - x0).max(2.0), lh - 2.0);
                        frame.rect_clipped(rect, color, 3.0, text_rect);
                    },
                );
            }
        }

        if let Some(occurrences) = &self.occurrences {
            // 前后各多检查 `OCCURRENCE_MARGIN` 个字符，使跨越屏幕边缘的词按完整的词判断。
            let from = shown.start.saturating_sub(OCCURRENCE_MARGIN);
            let to = (shown.end + OCCURRENCE_MARGIN).min(row.chars.len());
            for (start, end) in occurrences.find(&row.chars[from..to]) {
                let (start, end) = (from + start, from + end);
                if end < shown.start || start > shown.end {
                    continue;
                }
                let (start, end) = (line_start + start, line_start + end);
                if occurrences.skip == Some((start, end)) {
                    continue;
                }
                self.spans(row, start, end, &mut |top, x0, x1| {
                    let rect = Rect::new(x0, top + 1.0, x1 - x0, lh - 2.0);
                    frame.rect_clipped(rect, p.occurrence, 3.0, text_rect);
                });
            }
        }

        // 固定颜色放在光标词的临时底色上方，按下快捷键时不必先移走光标。
        for (a, b, color) in marked_occurrences(app, row.chars, shown) {
            self.spans(row, line_start + a, line_start + b, &mut |top, x0, x1| {
                let rect = Rect::new(x0, top + 1.0, x1 - x0, lh - 2.0);
                frame.rect_clipped(rect, color, 3.0, text_rect);
            });
        }

        for pos in self
            .brace
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .filter(|p| (line_start..line_end).contains(p))
        {
            let (part, x) = self.place(row, pos);
            let rect = Rect::new(x, row.y + part as f32 * lh + 1.0, self.cell, lh - 2.0);
            frame.rect_clipped(rect, p.match_brace, 3.0, text_rect);
        }
    }

    /// 这一行里要断开连字的列：光标和选区的两端。
    ///
    /// 光标停在 `=` 和 `>` 之间时，`=>` 不再画成一个箭头，方便看清改的是哪一个字符。
    fn ligature_splits(&self, row: &Row<'_>, splits: &mut Vec<usize>) {
        splits.clear();
        let start = row.line_start;
        let end = start + row.chars.len();
        let mut add = |pos: usize| {
            if pos > start && pos < end {
                splits.push(pos - start);
            }
        };
        add(self.selection.from());
        add(self.selection.to());
        for selection in self.extra {
            add(selection.from());
            add(selection.to());
        }
        splits.sort_unstable();
        splits.dedup();
    }

    /// 绘制文字，`tokens` 是这一行的高亮结果；返回最后一段绘制到的单元格数。
    ///
    /// `splits` 是要断开连字的列，见 [`Self::ligature_splits`]。
    fn draw_content(
        &self,
        frame: &mut Frame<'_>,
        row: &Row<'_>,
        tokens: &[Token],
        splits: &[usize],
    ) -> usize {
        let (app, origin_x, tab_width, text_rect) =
            (self.app, self.origin_x, self.tab_width, self.text_rect);
        let (chars, breaks) = (row.chars, &row.breaks);
        if breaks.rows() > 1 {
            let len = chars.len();
            let mut last = 0;
            for part in 0..breaks.rows() {
                // 一行可以断成上百段，只绘制与编辑区相交的段。返回值仅用于在最后一段之后绘制
                // 说明；最后一段不在编辑区内时，说明同样不在编辑区内。
                let top = row.y + part as f32 * self.lh;
                if top >= text_rect.bottom() {
                    break;
                }
                if top + self.lh <= text_rect.y {
                    continue;
                }
                let range = breaks.range(part, len);
                let base = row.baseline + part as f32 * self.lh;
                last = draw_glyph_run(
                    app,
                    frame,
                    (chars, range, tokens),
                    (origin_x, breaks.indent_of(part)),
                    base,
                    tab_width,
                    text_rect,
                    splits,
                );
            }
            // 折叠标记 `⋯` 和截断说明绘制在最后一段之后。
            last
        } else {
            match &row.aligned {
                Some((positions, fields, separator)) => draw_columns(
                    app,
                    frame,
                    (chars, positions, fields, *separator),
                    origin_x,
                    row.baseline,
                    text_rect,
                ),
                None if row.base > 0 => draw_glyph_run(
                    app,
                    frame,
                    (chars, 0..chars.len(), tokens),
                    (row.x0, row.base),
                    row.baseline,
                    tab_width,
                    text_rect,
                    splits,
                ),
                None => draw_glyphs(
                    app,
                    frame,
                    chars,
                    tokens,
                    row.x0,
                    row.baseline,
                    tab_width,
                    text_rect,
                    splits,
                ),
            }
        }
    }

    /// 按设置（`renderWhitespace`）绘制本行的空格和制表符。
    ///
    /// 按列对齐的表格里空白的位置不代表文件内容，不绘制。
    fn draw_whitespace(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let mode = self.app.config.render_whitespace.unwrap_or_default();
        if mode == RenderWhitespace::None || row.aligned.is_some() {
            return;
        }
        let line_end = row.line_start + row.chars.len();
        // 多光标时选区可能很多，先筛选出与本行相交的选区。
        let selected: Vec<(usize, usize)> = std::iter::once(&self.selection)
            .chain(self.extra)
            .filter(|s| !s.is_empty() && s.from() < line_end && s.to() > row.line_start)
            .map(|s| (s.from(), s.to()))
            .collect();
        if mode == RenderWhitespace::Selection && selected.is_empty() {
            return;
        }
        let trailing = match row.ends {
            true => trailing_whitespace(row.chars),
            false => row.chars.len(),
        };
        // 断成多段时只处理可见段：按整行断开的长行可达数 MB。
        let shown = row.shown_cols();
        for (col, &c) in row
            .chars
            .iter()
            .enumerate()
            .take(shown.end)
            .skip(shown.start)
        {
            if !matches!(c, ' ' | '\t') {
                continue;
            }
            let pos = row.line_start + col;
            let in_selection = || selected.iter().any(|&(from, to)| (from..to).contains(&pos));
            if !whitespace_shown(mode, col, trailing, in_selection) {
                continue;
            }
            let (part, x) = self.place(row, pos);
            if x > self.text_rect.right() && row.breaks.rows() == 1 {
                break;
            }
            let (next_part, next) = self.place(row, pos + 1);
            let width = if next_part == part {
                next - x
            } else {
                self.cell
            };
            let y = row.y + part as f32 * self.lh;
            draw_whitespace_mark(self.app, frame, c, (x, y), width, self.text_rect);
        }
    }

    /// 在最后一段之后绘制截断说明和「显示整行」，以及折叠行的小块；两者的点击区域分别记录到
    /// `long_hits`、`folded_hits`。
    ///
    /// `cells` 是最后一段绘制到的单元格数。
    fn draw_tail(
        &self,
        frame: &mut Frame<'_>,
        row: &Row<'_>,
        cells: usize,
        (folded_hits, long_hits): (&mut LineHits, &mut LineHits),
    ) {
        let (app, p, text_rect) = (self.app, self.app.palette, self.text_rect);
        let origin_x = row.x0;
        // 最后一段的 y 坐标和基线。
        let tail_y = row.y + (row.breaks.rows() - 1) as f32 * self.lh;
        let tail_baseline = row.baseline + (row.breaks.rows() - 1) as f32 * self.lh;

        if row.truncated {
            let x = origin_x + cells as f32 * self.cell + 4.0;
            let size = app.fonts.ui_small;
            let note = t!("editor-line-truncated", count = MAX_LINE_CHARS);
            let x = frame.text(
                FontKind::Ui,
                size,
                x,
                tail_baseline,
                &note,
                p.text_dim,
                text_rect,
            );
            // 按列对齐的表格不支持显示整行。
            if self.columns.is_none() {
                let hovered = app.pointer.hover == Hover::LongLine(row.line);
                let color = if hovered { p.text } else { p.accent };
                let show = t!("editor-line-show-whole");
                let (from, clip) = (x + 8.0, text_rect);
                let to = frame.text(FontKind::Ui, size, from, tail_baseline, show, color, clip);
                long_hits.push((Rect::new(from, tail_y, to - from, self.lh), row.line));
            }
        }

        // 折叠行之后绘制一个含三个点的圆角小块，点击后展开。
        if row.collapsed && row.ends {
            let x = (origin_x + (cells + 1) as f32 * self.cell).round();
            let chip = Rect::new(x, tail_y + 3.0, 22.0, app.fonts.line_height - 6.0);
            frame.rect_clipped(chip, p.selected_inactive, 4.0, text_rect);
            if app.pointer.hover == Hover::Folded(row.line) {
                frame.rect_clipped(chip, p.hover, 4.0, text_rect);
            }
            let cy = (chip.y + chip.h / 2.0 - 1.0).round();
            for i in 0..3 {
                let dot = Rect::new(chip.x + 6.0 + i as f32 * 4.0, cy, 2.0, 2.0);
                frame.rect_clipped(dot, p.text_muted, 1.0, text_rect);
            }
            folded_hits.push((chip, row.line));
        }
    }

    /// 编辑区有焦点时绘制本行的光标；主光标的位置记录到 `ImeState::cursor`。
    fn draw_carets(&self, frame: &mut Frame<'_>, row: &Row<'_>, ime_cursor: &mut Option<Rect>) {
        let (app, lh, text_rect) = (self.app, self.lh, self.text_rect);
        // 超出 `chars` 末尾的光标绘制在末尾；所在段不在 `chars` 内时不绘制。
        let end = row.line_start + row.chars.len();
        let caret_pos = |col: usize| {
            let pos = row.origin + col;
            let pos = if row.ends { pos.min(end) } else { pos };
            (pos >= row.line_start && (pos < end || row.ends)).then_some(pos)
        };
        // 空心的块状光标在编辑区没有焦点时仍然绘制。
        let shown = self.focused || self.vim_caret == Some(CaretShape::Hollow);
        if let Some(shape) = self.vim_caret
            && let Some(pos) = caret_pos(self.caret_col).filter(|_| row.line == self.caret_line)
            && shown
        {
            self.draw_vim_caret(frame, row, pos, shape);
            return;
        }
        if let Some(pos) =
            caret_pos(self.caret_col).filter(|_| row.line == self.caret_line && self.focused)
        {
            let (part, x) = self.place(row, pos);
            let shift = part as f32 * lh;
            *ime_cursor = Some(draw_caret(
                app,
                frame,
                x,
                row.y + shift,
                row.baseline + shift,
                text_rect,
            ));
        }
        // 其余光标与主光标同步闪烁；输入法组字只显示在主光标处。
        if self.focused {
            let carets = self.extra_carets.iter().filter(|c| c.0 == row.line);
            for pos in carets.filter_map(|&(_, col)| caret_pos(col)) {
                let (part, x) = self.place(row, pos);
                caret_bar(app, frame, x, row.y + part as f32 * lh, text_rect);
            }
        }
    }
}

impl Screen<'_> {
    /// 绘制 Vim 模式的光标：覆盖位置 `pos` 上字符的块、字符下方的横线，或空心的块。
    ///
    /// 块的宽度是字符的显示宽度（宽字符两格，行尾与空行一格）；实心块之下的字符用背景色重画。
    /// 实心块与横线随光标闪烁，空心块不闪烁。
    fn draw_vim_caret(&self, frame: &mut Frame<'_>, row: &Row<'_>, pos: usize, shape: CaretShape) {
        let (app, lh, clip) = (self.app, self.lh, self.text_rect);
        let (part, x) = self.place(row, pos);
        let y = row.y + part as f32 * lh;
        let c = row.chars.get(pos - row.line_start).copied();
        let width = match c {
            Some(_) => match self.place(row, pos + 1) {
                (next, right) if next == part && right > x => right - x,
                _ => self.cell,
            },
            None => self.cell,
        };
        let baseline = row.baseline + part as f32 * lh;
        draw_modal_caret(app, frame, (x, y, width), c, baseline, shape, clip);
    }
}

/// 返回光标所在代码块的缩进参考线（所在的格与行范围），用于加深绘制以标示嵌套层级。
///
/// 光标位于块的开头一行（下一个非空行缩进更深，如 `if x {`）时取块内一层，否则取光标行
/// 所在的层；从光标向上、向下扩展，空行和缩进更深的行均属于该块。只在 `visible` 内查找：
/// 光标不在屏幕上时返回 `None`，块很长时只计算到屏幕边缘。
pub(in crate::view::editor) fn active_guide(
    text: &impl Lines,
    caret_line: usize,
    visible: &Range<usize>,
    tab_width: usize,
    unit: usize,
) -> Option<(usize, Range<usize>)> {
    if !visible.contains(&caret_line) || unit == 0 {
        return None;
    }
    let indent = |line: usize| {
        let content = text.line_prefix(line, MAX_LINE_BYTES);
        let slice = content.slice();
        (!text::is_blank(slice)).then(|| text::indent_cells(slice, tab_width))
    };
    // 空行按其下方第一个非空行计算缩进，与参考线在空行上连续绘制的规则一致。
    let own = (caret_line..visible.end).find_map(indent)?;
    let next = (caret_line + 1..visible.end).find_map(indent);
    let (level, first) = match next {
        Some(next) if next > own && indent(caret_line).is_some() => {
            (own / unit * unit, caret_line + 1)
        }
        _ if own == 0 => return None,
        _ => ((own - 1) / unit * unit, caret_line),
    };
    let inside = |line: usize| indent(line).is_none_or(|i| i > level);
    let start = (visible.start..first)
        .rev()
        .take_while(|&l| inside(l))
        .last()
        .unwrap_or(first);
    let end = (first..visible.end).take_while(|&l| inside(l)).last()? + 1;
    // 块尾的空行不属于该块：参考线只绘制到最后一个非空行。
    let end = (start..end).rev().find(|&l| indent(l).is_some())? + 1;
    Some((level, start..end))
}

#[cfg(test)]
mod tests;
