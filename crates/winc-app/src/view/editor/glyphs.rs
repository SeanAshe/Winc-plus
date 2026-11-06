//! 字形：按语法着色绘制一行文字，自动换行的分段，显示列。

use winc_config::RenderWhitespace;

use super::*;

/// 删掉的地方：在 `(x, y)` 这道行间的分界上绘制一个朝右的小三角，`lit` 时大一号。
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
pub(super) fn draw_glyph_run(
    app: &App,
    frame: &mut Frame<'_>,
    (chars, range, tokens): (&[char], Range<usize>, &[winc_syntax::Token]),
    (origin_x, start): (f32, usize),
    baseline: f32,
    tab_width: usize,
    clip: Rect,
) -> usize {
    let p = app.palette;
    let cell = app.cell;
    let mut cells = start;
    let mut token = tokens.partition_point(|t| t.end <= range.start);
    for col in range {
        let c = chars[col];
        let width = text::char_cells(c, cells, tab_width);
        let x = origin_x + cells as f32 * cell;
        cells += width;
        if c == ' ' || c == '\t' || x > clip.right() {
            continue;
        }
        while tokens.get(token).is_some_and(|t| t.end <= col) {
            token += 1;
        }
        let kind = tokens
            .get(token)
            .filter(|t| t.start <= col)
            .map_or(TokenKind::Text, |t| t.kind);
        frame.cell_char(
            token_font(kind),
            app.fonts.code,
            x,
            baseline,
            cell,
            width.max(1),
            c,
            p.token(kind),
            clip,
        );
    }
    cells
}

/// 计算自动换行时一行中列 `from..to` 的文本落在哪几段，对每段调用 `f(段号, 起点格, 终点格)`。
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
/// `selected` 表示这一列在选区里。
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

/// 绘制一个空白符：空格是格子中央的小圆点，制表符是占满它的宽度的箭头。
///
/// `x`、`y` 是这个字符的左上角，`width` 是它的宽度（逻辑像素）。颜色取自缩进参考线，比文字淡得多。
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
        let head = (shaft.right() - app.cell / 4.0, mid);
        crate::icons::chevron(
            frame,
            head,
            app.cell / 2.0,
            crate::icons::Direction::Right,
            color,
            clip,
        );
    }
}

/// 绘制一行的字符，按 `tokens` 着色，返回这一行占用的格数。
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
) -> usize {
    let p = app.palette;
    let cell = app.cell;
    let mut cells = 0;
    let mut token = 0;

    for (col, &c) in chars.iter().enumerate() {
        let width = text::char_cells(c, cells, tab_width);
        let x = origin_x + cells as f32 * cell;
        cells += width;

        if x > clip.right() {
            break;
        }

        if c == ' ' || c == '\t' || x + width as f32 * cell < clip.x {
            continue;
        }

        while tokens.get(token).is_some_and(|t| t.end <= col) {
            token += 1;
        }

        let kind = tokens
            .get(token)
            .filter(|t| t.start <= col)
            .map_or(TokenKind::Text, |t| t.kind);
        frame.cell_char(
            token_font(kind),
            app.fonts.code,
            x,
            baseline,
            cell,
            width.max(1),
            c,
            p.token(kind),
            clip,
        );
    }
    cells
}

/// 绘制 CSV、TSV 的一行：每个字符绘制在对齐之后的单元格内，每一列使用一种颜色（循环使用
/// `COLORS`），分隔符颜色较淡。
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
    const COLORS: [TokenKind; 6] = [
        TokenKind::Text,
        TokenKind::Keyword,
        TokenKind::String,
        TokenKind::Function,
        TokenKind::Number,
        TokenKind::Builtin,
    ];
    let p = app.palette;
    let cell = app.cell;
    for (col, &c) in chars.iter().enumerate() {
        let (start, end) = (positions[col], positions[col + 1]);
        let x = origin_x + start as f32 * cell;
        if x > clip.right() {
            break;
        }
        if c.is_whitespace() || x + (end - start) as f32 * cell < clip.x {
            continue;
        }
        let field = fields[col];
        // 分隔符：其后的字符属于下一列（分隔符是行内最后一个字符时，其后没有字符）。
        let is_separator = c == separator && fields.get(col + 1).is_none_or(|&next| next != field);
        let color = if is_separator {
            p.text_dim
        } else {
            p.token(COLORS[field % COLORS.len()])
        };
        // 按字符自身的宽度绘制，不计入为对齐而留出的空白。
        let width = text::char_cells(c, 0, 1).max(1);
        frame.cell_char(
            FontKind::Mono,
            app.fonts.code,
            x,
            baseline,
            cell,
            width,
            c,
            color,
            clip,
        );
    }
    positions.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests;
