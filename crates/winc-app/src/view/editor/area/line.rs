//! 文本区的一行：行号栏里的部分（见 `gutter`）、缩进参考线、各类标注、文字、折叠标记和光标。

use winc_config::RenderWhitespace;
use winc_i18n::t;

use super::*;

impl Screen<'_> {
    /// 绘制行 `line`：`line_start` 是行首的字符偏移，`content` 是这一行的文字，`state` 是行首的
    /// 高亮状态，绘制完后换成下一行行首的状态。
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
            ime_cursor,
        } = scratch;
        let (text_rect, lh) = (self.text_rect, self.lh);
        let y = self.row_y(line);
        let baseline = baseline_in(
            Rect::new(text_rect.x, y, text_rect.w, lh),
            self.app.mono_metrics.ascent,
            self.app.mono_metrics.descent,
        );
        let slice = RopeSlice::from(content);

        chars.clear();
        chars.extend(content.chars().take(MAX_LINE_CHARS + 1));
        let truncated = chars.len() > MAX_LINE_CHARS;
        chars.truncate(MAX_LINE_CHARS);

        let line_end = line_end(self.doc.text(), line, line_start, chars.len(), truncated);
        // 按列对齐时，每个字符所在的单元格位置和所属的列。
        let aligned = self.columns.map(|c| {
            let fields = Some(&mut *column_fields);
            c.layout(slice, self.tab_width, chars.len(), column_cells, fields);
            (&column_cells[..], &column_fields[..], c.separator)
        });
        let breaks = self.doc.line_breaks(line);

        // 行号、书签、折叠箭头绘制在行号栏中该行的位置上。断成多段的行滚动到只露出后面几段时，
        // 第一段已经超出编辑区上沿，行号栏里放不下，于是下移后贴在顶部绘制，使行号仍然
        // 对应这一行。下移量最多到最后一段：普通行（只有一段）只是上半部分被滚出，
        // 最后一段只露出下半部分时同样如此，两种情况都不能与下一行的行号重叠。
        let gutter_shift = (text_rect.y - y).clamp(0.0, (breaks.rows() - 1) as f32 * lh);
        let row = Row {
            line,
            line_start,
            line_end,
            y,
            baseline,
            gutter_y: y + gutter_shift,
            gutter_baseline: baseline + gutter_shift,
            slice,
            chars: &chars[..],
            truncated,
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

        *state = winc_syntax::highlight_line(self.language, row.chars, *state, tokens);
        self.ligature_splits(&row, splits);
        self.draw_whitespace(frame, &row);
        let cells = self.draw_content(frame, &row, tokens, splits);
        self.draw_tail(frame, &row, cells, folded_hits);
        self.draw_carets(frame, &row, ime_cursor);
    }

    /// 返回位置 `pos` 在这一行中的段号（从 0 起）和 x 坐标。
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
            None => row
                .prefix
                .get_or_init(|| text::prefix_cells(row.chars, tab_width))[col],
        };
        (0, origin_x + cells as f32 * cell)
    }

    /// 文本范围 `start..end` 在屏幕上占据的几块：跨段时每段一块，对每块调用 `f(y, x0, x1)`。
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

    /// 缩进参考线：每级缩进一条，空行沿用上一行的缩进 `guide_indent`。
    fn draw_guides(&self, frame: &mut Frame<'_>, row: &Row<'_>, guide_indent: &mut usize) {
        let breaks = &row.breaks;
        if !text::is_blank(row.slice) {
            *guide_indent = text::indent_cells(row.slice, self.tab_width);
        }

        for level in (0..*guide_indent).step_by(self.indent_unit.max(1)) {
            let x = (self.origin_x + level as f32 * self.cell).round();
            // 折成多段时，参考线一直绘制到最后一段（后续各段与第一段缩进相同）。
            let height = self.lh
                * breaks
                    .rows()
                    .min(if level < breaks.indent { usize::MAX } else { 1 })
                    as f32;
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

    /// 查找结果、光标所在的词的其他出现、配对的括号。
    fn draw_marks(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let (app, p, lh, text_rect) = (self.app, self.app.palette, self.lh, self.text_rect);
        let (line_start, line_end) = (row.line_start, row.line_end);
        if self.show_matches {
            let current = app.search.current_match();

            for &(start, end) in app.search.matches_in(line_start, line_end) {
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
            for (start, end) in occurrences.find(row.chars) {
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
                // 一行可以断成上百段，只绘制与编辑区相交的段。返回值只用于在最后一段之后绘制
                // 说明：最后一段不在编辑区内时，说明同样不在编辑区内。
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
                None => draw_glyphs(
                    app,
                    frame,
                    chars,
                    tokens,
                    origin_x,
                    row.baseline,
                    tab_width,
                    text_rect,
                    splits,
                ),
            }
        }
    }

    /// 按设置（`renderWhitespace`）绘制这一行的空格和制表符。
    ///
    /// 按列对齐的表格里空白的位置不代表文件内容，不绘制。
    fn draw_whitespace(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let mode = self.app.config.render_whitespace.unwrap_or_default();
        if mode == RenderWhitespace::None || row.aligned.is_some() {
            return;
        }
        let line_end = row.line_start + row.chars.len();
        // 多光标时选区可能很多，先留下和这一行相交的。
        let selected: Vec<(usize, usize)> = std::iter::once(&self.selection)
            .chain(self.extra)
            .filter(|s| !s.is_empty() && s.from() < line_end && s.to() > row.line_start)
            .map(|s| (s.from(), s.to()))
            .collect();
        if mode == RenderWhitespace::Selection && selected.is_empty() {
            return;
        }
        let trailing = trailing_whitespace(row.chars);
        for (col, &c) in row.chars.iter().enumerate() {
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

    /// 最后一段之后：截断说明，以及折叠行的小块（点击区域收进 `folded_hits`）。
    ///
    /// `cells` 是最后一段绘制到的单元格数。
    fn draw_tail(
        &self,
        frame: &mut Frame<'_>,
        row: &Row<'_>,
        cells: usize,
        folded_hits: &mut Vec<(Rect, usize)>,
    ) {
        let (app, p, text_rect) = (self.app, self.app.palette, self.text_rect);
        let origin_x = self.origin_x;
        // 最后一段的 y 坐标和基线。
        let tail_y = row.y + (row.breaks.rows() - 1) as f32 * self.lh;
        let tail_baseline = row.baseline + (row.breaks.rows() - 1) as f32 * self.lh;

        if row.truncated {
            let x = origin_x + cells as f32 * self.cell + 4.0;
            frame.text(
                FontKind::Ui,
                app.fonts.ui_small,
                x,
                tail_baseline,
                t!("editor-line-truncated"),
                p.text_dim,
                text_rect,
            );
        }

        // 折叠行之后是一个圆角小块，里面三个点，点击后展开。
        if row.collapsed {
            let x = (origin_x + (cells + 1) as f32 * self.cell).round();
            let chip = Rect::new(x, tail_y + 3.0, 22.0, app.fonts.line_height - 6.0);
            frame.rect_clipped(chip, p.selected_inactive, 4.0, text_rect);
            if app.hover == Hover::Folded(row.line) {
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

    /// 编辑区有焦点时绘制这一行上的光标；主光标的位置记进 `ime_cursor`。
    fn draw_carets(&self, frame: &mut Frame<'_>, row: &Row<'_>, ime_cursor: &mut Option<Rect>) {
        let (app, lh, text_rect) = (self.app, self.lh, self.text_rect);
        if row.line == self.caret_line && self.focused {
            let (part, x) = self.place(row, row.line_start + self.caret_col.min(MAX_LINE_CHARS));
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
        // 其余光标与主光标一起闪烁；输入法组字只显示在主光标处。
        if self.focused {
            for &(_, col) in self.extra_carets.iter().filter(|c| c.0 == row.line) {
                let (part, x) = self.place(row, row.line_start + col.min(MAX_LINE_CHARS));
                caret_bar(app, frame, x, row.y + part as f32 * lh, text_rect);
            }
        }
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
        let content = text.line_content(line);
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
