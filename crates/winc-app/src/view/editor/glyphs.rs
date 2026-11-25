//! 字形：按语法着色绘制一行文字，以及自动换行的分段与显示列的计算。

use winc_config::RenderWhitespace;

use super::*;

/// 标出删除位置：在 `(x, y)` 所在的行间分界上绘制一个朝右的小三角，`lit` 为真时尺寸大一号。
pub(super) fn draw_deleted_mark(
    frame: &mut Frame<'_>,
    (x, y): (f32, f32),
    color: Color,
    lit: bool,
    clip: Rect,
) {
    let half = if lit { 5.0 } else { 4.0 };
    for i in 0..half as usize {
        let h = (half - i as f32) * 2.0;
        let column = Rect::new(x + i as f32, y - h / 2.0, 1.0, h);
        frame.rect_clipped(column, color, 0.0, clip);
    }
}

/// 返回代码中 `kind` 类文字使用的字体：注释用斜体，其余用正体。
fn token_font(kind: TokenKind) -> FontKind {
    match kind {
        TokenKind::Comment => FontKind::MonoItalic,
        _ => FontKind::Mono,
    }
}

/// 绘制自动换行时一行中的一段（`range` 范围内的字符），从第 `start` 格开始（后续各段先留出缩进），
/// 返回绘制结束时的格数。
///
/// `splits` 是要断开连字的列（已排序）：光标或选区边界落在两个字符之间时，这两个字符不再连成一个字。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_glyph_run(
    app: &App,
    frame: &mut Frame<'_>,
    (chars, range, tokens): (&[char], Range<usize>, &[winc_syntax::Token]),
    (origin_x, start): (f32, usize),
    baseline: f32,
    tab_width: usize,
    clip: Rect,
    splits: &[usize],
) -> usize {
    paint_chars(
        app, frame, chars, range, tokens, origin_x, start, baseline, tab_width, clip, splits,
    )
}

/// 计算自动换行时一行中列 `from..to` 的文本所在的各段，对每段调用 `f(段号, 起点格, 终点格)`。
///
/// 格从行的左边起数（后续各段的缩进计入其中）。非最后一段绘制到该段末尾，而不是下一段的开头；
/// 终点正好落在分段处时计入前一段末尾，下一段开头不会多出一块。
pub(super) fn wrapped_spans(
    chars: &[char],
    breaks: &winc_core::wrap::Breaks,
    (from, to): (usize, usize),
    tab_width: usize,
    mut f: impl FnMut(usize, usize, usize),
) {
    let to = to.min(chars.len());
    let from = from.min(to);
    let first = breaks.row_of(from);
    let last = if to > from {
        breaks.row_of(to - 1)
    } else {
        first
    };
    for part in first..=last {
        let range = breaks.range(part, chars.len());
        let cell_at = |col: usize| {
            text::slice_cells(&chars[range.start..col], breaks.indent_of(part), tab_width)
        };
        f(
            part,
            cell_at(from.max(range.start)),
            cell_at(to.min(range.end).max(range.start)),
        );
    }
}

/// 第 `col` 个字符（空格或制表符）在 `mode` 下是否显示；`trailing` 是行尾空白开始的列，
/// `selected` 表示该列是否在选区内。
pub(in crate::view) fn whitespace_shown(
    mode: RenderWhitespace,
    col: usize,
    trailing: usize,
    selected: impl FnOnce() -> bool,
) -> bool {
    match mode {
        RenderWhitespace::None => false,
        RenderWhitespace::Selection => selected(),
        RenderWhitespace::Trailing => col >= trailing,
        RenderWhitespace::All => true,
    }
}

/// 返回 `chars` 行尾空白开始的列；行尾没有空白时是行长。
pub(in crate::view) fn trailing_whitespace(chars: &[char]) -> usize {
    chars.len()
        - chars
            .iter()
            .rev()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .count()
}

/// 绘制一个空白符：空格为单元格中央的小圆点，制表符为占满其宽度的箭头。
///
/// `x`、`y` 是该字符的左上角，`width` 是其宽度（逻辑像素）。颜色取自缩进参考线，明显淡于文字。
pub(in crate::view) fn draw_whitespace_mark(
    app: &App,
    frame: &mut Frame<'_>,
    c: char,
    (x, y): (f32, f32),
    width: f32,
    clip: Rect,
) {
    /// 空格圆点的直径（逻辑像素）。
    const DOT: f32 = 2.0;
    /// 制表符箭头两端的留白（逻辑像素）。
    const TAB_PAD: f32 = 2.0;
    let color = app.palette.indent_guide_active;
    let mid = y + app.fonts.line_height / 2.0;
    if c == ' ' {
        let dot = Rect::new(x + width / 2.0 - DOT / 2.0, mid - DOT / 2.0, DOT, DOT);
        frame.rect_clipped(dot, color, DOT / 2.0, clip);
    } else {
        let shaft = Rect::new(
            x + TAB_PAD,
            mid - 0.5,
            (width - 2.0 * TAB_PAD).max(1.0),
            1.0,
        );
        frame.rect_clipped(shaft, color, 0.0, clip);
        let head = (shaft.right() - app.metrics.cell / 4.0, mid);
        crate::icons::chevron(
            frame,
            head,
            app.metrics.cell / 2.0,
            crate::icons::Direction::Right,
            color,
            clip,
        );
    }
}

/// 绘制一行的字符，按 `tokens` 着色，返回这一行占用的格数。
///
/// `splits` 是要断开连字的列（已排序），见 [`draw_glyph_run`]。
#[allow(clippy::too_many_arguments)]
pub(in crate::view) fn draw_glyphs(
    app: &App,
    frame: &mut Frame<'_>,
    chars: &[char],
    tokens: &[winc_syntax::Token],
    origin_x: f32,
    baseline: f32,
    tab_width: usize,
    clip: Rect,
    splits: &[usize],
) -> usize {
    paint_chars(
        app,
        frame,
        chars,
        0..chars.len(),
        tokens,
        origin_x,
        0,
        baseline,
        tab_width,
        clip,
        splits,
    )
}

/// 连字向屏幕外多看的字符数：`=>` 的前一半刚被滚出时，后一半仍要用到它。
const LIGATURE_CONTEXT: usize = 16;

/// 返回列 `col` 的语法类别；没有 token 盖住时是普通文本。
fn kind_at(tokens: &[winc_syntax::Token], col: usize) -> TokenKind {
    let index = tokens.partition_point(|token| token.end <= col);
    tokens
        .get(index)
        .filter(|token| token.start <= col)
        .map_or(TokenKind::Text, |token| token.kind)
}

/// 绘制 `chars[range]`，同一字体、同一种颜色的一段一起整形，从而出现连字。
///
/// 空格和制表符、光标或选区切开的位置、以及落到后备字体上的字，都会把这一段断开。
/// 很长的行只整形视口附近的一段。返回值停在右边缘，折叠标记才不会被一行很长的文字推到屏幕外。
#[allow(clippy::too_many_arguments)]
fn paint_chars(
    app: &App,
    frame: &mut Frame<'_>,
    chars: &[char],
    range: Range<usize>,
    tokens: &[winc_syntax::Token],
    origin_x: f32,
    start_cells: usize,
    baseline: f32,
    tab_width: usize,
    clip: Rect,
    splits: &[usize],
) -> usize {
    let (cell, size) = (app.metrics.cell, app.fonts.code);
    let limit = range.end.min(chars.len());
    let mut col = range.start.min(limit);
    let mut cells = start_cells;

    let left = clip.x - cell * LIGATURE_CONTEXT as f32;
    let mut win = col;
    let mut win_cells = cells;
    let mut scan = text::Behind::before(chars, col);
    while win < limit {
        let width = scan.cells(chars[win], win_cells, tab_width);
        let edge = origin_x + (win_cells + width) as f32 * cell;
        if edge >= left {
            break;
        }
        win_cells += width;
        win += 1;
    }
    let win = win.saturating_sub(LIGATURE_CONTEXT).max(range.start);
    cells = start_cells;
    let mut behind = text::Behind::before(chars, range.start.min(limit));
    for ch in chars.iter().take(win).skip(range.start) {
        cells += behind.cells(*ch, cells, tab_width);
    }
    col = win;

    let right = clip.right() + cell * LIGATURE_CONTEXT as f32;
    let mut columns = Vec::new();
    // 第一个起点已经超出右边缘的字符：把它的宽度算进去后就用这个值当返回值。
    let mut reported = None;
    while col < limit {
        let x = origin_x + cells as f32 * cell;
        if x >= right {
            break;
        }
        let c = chars[col];
        let width = behind.cells(c, cells, tab_width);
        if reported.is_none() && x > clip.right() {
            reported = Some(cells + width);
        }
        if c == ' ' || c == '\t' || c.is_control() {
            cells += width;
            col += 1;
            continue;
        }
        // emoji 序列不进连字：由 emoji 字体合成，宽度记在首字符上。
        let next = chars.get(col + 1).copied();
        if next.is_some_and(|n| winc_emoji::may_join(c, n)) {
            let len = winc_emoji::sequence_len(chars[col..].iter().copied())
                .min(limit - col)
                .max(1);
            let mut span = width;
            for extra in 1..len {
                span += behind.cells(chars[col + extra], cells + span, tab_width);
            }
            if x < clip.right() && x + span as f32 * cell > clip.x {
                let kind = kind_at(tokens, col);
                draw_grid_char(
                    frame,
                    (token_font(kind), size, cell),
                    (x, baseline),
                    (chars, col),
                    app.palette.token(kind),
                    clip,
                );
            }
            cells += span;
            col += len;
            continue;
        }
        let kind = kind_at(tokens, col);
        let font = token_font(kind);
        if !frame.in_mono_font(font, c) {
            if x < clip.right() && x + width as f32 * cell > clip.x {
                frame.cell_char(
                    font,
                    size,
                    x,
                    baseline,
                    cell,
                    width.max(1),
                    c,
                    app.palette.token(kind),
                    clip,
                );
            }
            cells += width;
            col += 1;
            continue;
        }

        let run_col = col;
        let run_cells = cells;
        columns.clear();
        columns.push(0u32);
        let mut acc = width as u32;
        columns.push(acc);
        cells += width;
        col += 1;
        while col < limit && origin_x + cells as f32 * cell < right {
            if splits.binary_search(&col).is_ok() {
                break;
            }
            let next = chars[col];
            if next == ' ' || next == '\t' || next.is_control() {
                break;
            }
            if chars
                .get(col + 1)
                .is_some_and(|n| winc_emoji::may_join(next, *n))
            {
                break;
            }
            if kind_at(tokens, col) != kind || !frame.in_mono_font(font, next) {
                break;
            }
            let x = origin_x + cells as f32 * cell;
            let width = behind.cells(next, cells, tab_width);
            if reported.is_none() && x > clip.right() {
                reported = Some(cells + width);
            }
            acc += width as u32;
            columns.push(acc);
            cells += width;
            col += 1;
        }
        paint_run(
            app,
            frame,
            &chars[run_col..col],
            &columns,
            font,
            kind,
            origin_x,
            run_cells,
            baseline,
            clip,
        );
    }
    if let Some(cells) = reported {
        return cells;
    }
    while col < limit {
        cells += behind.cells(chars[col], cells, tab_width);
        col += 1;
    }
    cells
}

/// 绘制已经切好的一段 `run`：能连字就画连字，整形没有覆盖到的字符仍逐字画。
#[allow(clippy::too_many_arguments)]
fn paint_run(
    app: &App,
    frame: &mut Frame<'_>,
    run: &[char],
    columns: &[u32],
    font: FontKind,
    kind: TokenKind,
    origin_x: f32,
    run_cells: usize,
    baseline: f32,
    clip: Rect,
) {
    let (cell, size) = (app.metrics.cell, app.fonts.code);
    let color = app.palette.token(kind);
    let count = frame.shape_ligatures(font, size, run);
    let mut index = 0;
    let mut glyph = 0;
    while index < run.len() {
        let start_cell = columns[index];
        let x = origin_x + (run_cells + start_cell as usize) as f32 * cell;
        if glyph < count && frame.ligature_span(glyph).0 == index {
            let (_, mut end) = frame.ligature_span(glyph);
            end = end.clamp(index + 1, run.len());
            let span = (columns[end] - start_cell) as f32 * cell;
            if x < clip.right() && x + span > clip.x {
                let c = (end == index + 1).then_some(run[index]);
                while glyph < count && frame.ligature_span(glyph).0 == index {
                    frame.draw_ligature(glyph, font, size, x, baseline, span, c, color, clip);
                    glyph += 1;
                }
            } else {
                while glyph < count && frame.ligature_span(glyph).0 == index {
                    glyph += 1;
                }
            }
            index = end;
            continue;
        }
        while glyph < count && frame.ligature_span(glyph).0 <= index {
            glyph += 1;
        }
        let width = (columns[index + 1] - start_cell).max(1);
        if x < clip.right() && x + width as f32 * cell > clip.x {
            frame.cell_char(
                font,
                size,
                x,
                baseline,
                cell,
                width as usize,
                run[index],
                color,
                clip,
            );
        }
        index += 1;
    }
}

/// 绘制 CSV、TSV 的一行：每个字符绘制在对齐之后的单元格内，各列的颜色和分隔符的颜色与按原文
/// 排布时相同（见 [`winc_syntax::columns`]）。
///
/// 返回绘制结束时的格数。
pub(super) fn draw_columns(
    app: &App,
    frame: &mut Frame<'_>,
    (chars, positions, fields, separator): (&[char], &[usize], &[usize], char),
    origin_x: f32,
    baseline: f32,
    clip: Rect,
) -> usize {
    use winc_syntax::columns::COLUMN_KINDS;
    let p = app.palette;
    let cell = app.metrics.cell;
    let mut joined = 0;
    for (col, &c) in chars.iter().enumerate() {
        let (start, end) = (positions[col], positions[col + 1]);
        let x = origin_x + start as f32 * cell;
        if x > clip.right() {
            break;
        }
        if c.is_whitespace() || x + (end - start) as f32 * cell < clip.x || col < joined {
            continue;
        }
        let field = fields[col];
        // 分隔符：其后的字符属于下一列（分隔符是行内最后一个字符时，其后没有字符）。
        let is_separator = c == separator && fields.get(col + 1).is_none_or(|&next| next != field);
        let color = if is_separator {
            p.token(TokenKind::Separator)
        } else {
            p.token(COLUMN_KINDS[field % COLUMN_KINDS.len()])
        };
        // 按字符自身的宽度绘制，不计入为对齐而留出的空白。emoji 序列随首字符整体绘制。
        let font = (FontKind::Mono, app.fonts.code, cell);
        joined = col + draw_grid_char(frame, font, (x, baseline), (chars, col), color, clip);
    }
    positions.last().copied().unwrap_or(0)
}

/// 在等宽网格上绘制 `chars[col]`，返回已处理的字符数：该字符及其后属于同一 emoji 序列的字符。
///
/// 序列由 emoji 字体合成一个图形，占 [`text::sequence_cells`] 给出的列数；无法合成时只绘制首字符。
/// `(font, size, cell)` 是字体、字号和一格的宽度，`(x, baseline)` 是该字符所在格的左边缘和基线。
pub(in crate::view) fn draw_grid_char(
    frame: &mut Frame<'_>,
    (font, size, cell): (FontKind, f32, f32),
    (x, baseline): (f32, f32),
    (chars, col): (&[char], usize),
    color: Color,
    clip: Rect,
) -> usize {
    let c = chars[col];
    let next = chars.get(col + 1).copied();
    if !next.is_some_and(|next| winc_emoji::may_join(c, next)) {
        let cells = text::char_cells(c, 0, 1).max(1);
        frame.cell_char(font, size, x, baseline, cell, cells, c, color, clip);
        return 1;
    }
    let len = winc_emoji::sequence_len(chars[col..].iter().copied());
    let span = text::sequence_cells(chars[col..col + len].iter().copied(), 0, 1).max(1);
    let unit: String = chars[col..col + len].iter().collect();
    if len == 1 || !frame.cell_sequence(font, size, x, baseline, cell, span, &unit, color, clip) {
        frame.cell_char(font, size, x, baseline, cell, span, c, color, clip);
    }
    len
}

/// 返回 `chars` 中各字符在等宽网格上占的格数，制表符按 `tab_width` 计。
///
/// 与 [`text::Behind::cells`] 的区别：控制字符和孤立的组合符号占 1 格，
/// 只有 emoji 序列的后续字符占 0 格。用于输入法组字和定义列表中的预览，其中每个字符都必须可见。
pub(in crate::view) fn loose_cells(chars: &[char], tab_width: usize) -> Vec<usize> {
    let mut behind = text::Behind::default();
    let mut cells = 0;
    (chars.iter().enumerate())
        .map(|(col, &c)| {
            let before = |back: usize| col.checked_sub(back).map(|k| chars[k]);
            let member = winc_emoji::joined((before(2), before(1)), c);
            let width = behind.cells(c, cells, tab_width);
            let width = if member { width } else { width.max(1) };
            cells += width;
            width
        })
        .collect()
}

#[cfg(test)]
mod tests;

impl App {
    /// 为文档 `doc` 第 `line` 行的字符 `chars` 着色，结果写入 `tokens`，返回下一行行首的状态；
    /// 长行的结果跨帧复用（[`LineTokens`]）。
    pub(super) fn highlight_cached(
        &self,
        doc: &winc_editor::Document,
        line: usize,
        language: &'static winc_syntax::Language,
        chars: &[char],
        state: LineState,
        tokens: &mut Vec<winc_syntax::Token>,
    ) -> LineState {
        // 表格不做语法着色，按列着色；按列对齐时由 `draw_columns` 逐字符着色，颜色相同。
        if let Some(separator) = doc.delimiter() {
            winc_syntax::columns::column_tokens(chars, separator, tokens);
            return state;
        }
        if chars.len() < LONG_TOKENS_MIN {
            return winc_syntax::highlight_line(language, chars, state, tokens);
        }
        let key = (doc.id, doc.revision, line, chars.len(), state, language.id);
        let mut cache = self.caches.line_tokens.borrow_mut();
        if let Some(i) = cache.entries.iter().position(|(k, ..)| *k == key) {
            let entry = cache.entries.remove(i);
            tokens.clear();
            tokens.extend_from_slice(&entry.1);
            let next = entry.2;
            cache.entries.push(entry);
            return next;
        }
        let next = winc_syntax::highlight_line(language, chars, state, tokens);
        if cache.entries.len() >= LONG_TOKENS_LINES {
            cache.entries.remove(0);
        }
        cache.entries.push((key, tokens.clone(), next));
        next
    }
}
