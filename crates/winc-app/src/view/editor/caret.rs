//! 选区、光标和配对的括号。

use super::*;
use crate::app::vim::CaretShape;

/// 返回光标竖线的矩形：宽 2，高为一行。
pub(in crate::view) fn caret_rect(x: f32, y: f32, line_height: f32) -> Rect {
    Rect::new(x, y, 2.0, line_height)
}

/// 返回 `y` 处开始、共 `rows` 段的一行中与 `clip` 相交的段（左闭右开）；`lh` 是行高。
///
/// `y` 为 `f64`：长行的第一段可在屏幕之外数亿像素处，`f32` 分辨不出其中的各段。
pub(super) fn visible_parts(y: f64, rows: usize, clip: Rect, lh: f32) -> Range<usize> {
    let lh = f64::from(lh);
    let first = ((f64::from(clip.y) - y) / lh).floor().max(0.0) as usize;
    let end = ((f64::from(clip.bottom()) - y) / lh).ceil().max(0.0) as usize;
    first.min(rows)..end.min(rows)
}

/// 逐行读取时返回的 `content` 是否可能只是该行的开头部分（读取上限为 [`MAX_LINE_BYTES`]）。
pub(super) fn reads_head_only(content: &str) -> bool {
    cut_by(content, MAX_LINE_BYTES)
}

/// `content` 是否超出一行默认显示的部分：字符数超过 [`MAX_LINE_CHARS`]，或只读到了该行的开头（见
/// [`reads_head_only`]）。
///
/// 这样的行按整行断开时，绘制、定位和缩略图使用行内窗口（`Document::line_window`）。
pub(super) fn exceeds_head(content: &str) -> bool {
    // 字节数不超过 `MAX_LINE_CHARS` 时字符数必然不超过，无需计数。
    reads_head_only(content)
        || (content.len() > MAX_LINE_CHARS && content.chars().nth(MAX_LINE_CHARS).is_some())
}

/// 按 `cap` 字节的上限逐行读取时，返回的 `content` 是否可能只是该行的开头部分。
///
/// 截断处不切开字符，截断后的内容最多比上限少 3 个字节（UTF-8 字符最长 4 字节）。
pub(super) fn cut_by(content: &str, cap: usize) -> bool {
    content.len() + 3 >= cap
}

/// 返回绘制文档 `doc` 时逐行读取的字节上限（字节）。
///
/// `aligned` 表示是否按列对齐；`origin_x`、`right`、`cell` 为第 0 列的 x 坐标、文本区右边缘和列宽。
///
/// 大文件不换行时只需读取到右边缘：每个字符至少占 1 个显示列、最多 4 字节，另留 2 列余量；其余情况
/// 返回 `MAX_LINE_BYTES`。横向滚动到各行开头的 [`MAX_LINE_CHARS`] 个字符都在左边缘 `left`（每个字符
/// 最多占 `tab_width` 或 2 个显示列）之外时，只读取 [`HEAD_PROBE`] 字节。
pub(super) fn line_read_cap(
    doc: &winc_editor::Document,
    aligned: bool,
    (origin_x, right, cell): (f32, f32, f32),
    (left, tab_width): (f32, usize),
) -> usize {
    if doc.is_wrapping() || aligned {
        return MAX_LINE_BYTES;
    }
    let head = (MAX_LINE_CHARS * tab_width.max(2)) as f32 * cell;
    if left - origin_x > head {
        return HEAD_PROBE;
    }
    if !doc.is_large() {
        return MAX_LINE_BYTES;
    }
    let cols = ((right - origin_x) / cell).ceil().max(0.0) as usize;
    ((cols + 2) * 4).min(MAX_LINE_BYTES)
}

/// 各行的开头都已滚出屏幕时逐行读取的字节数（字节），见 [`line_read_cap`]。
const HEAD_PROBE: usize = 64;

/// 返回只保留 `len` 个字符之内的断点的 `breaks`。
///
/// 断行结果与绘制的字符不同步（等待重新断行）时，断点可能超出只取了行首部分的字符。
pub(super) fn fit_breaks(
    mut breaks: winc_core::wrap::Breaks,
    len: usize,
) -> winc_core::wrap::Breaks {
    let kept = breaks.starts.partition_point(|&start| start < len);
    breaks.starts.truncate(kept);
    breaks
}

/// 返回第 `line` 行的行末位置（字符偏移）。
///
/// 截断的行只读取了开头的 `shown` 个字符，行末位置需要由下一行的行首推算。
pub(super) fn line_end(
    text: &impl Lines,
    line: usize,
    line_start: usize,
    shown: usize,
    truncated: bool,
) -> usize {
    match truncated {
        false => line_start + shown,
        true if line + 1 < text.line_count() => text.line_start(line + 1) - 1,
        true => text.end_pos(),
    }
}

/// 返回选区 `selection` 涉及的行（左闭右开）。
///
/// 范围以外的行既没有选中的部分，也不影响块的圆角，因此绘制选区时只测量这些行：
/// 有数千个选区时，无需为每个选区测量屏幕上的所有行。
fn selection_lines(text: &impl Lines, selection: Selection) -> Range<usize> {
    text.pos_to_line_col(selection.from()).0..text.pos_to_line_col(selection.to()).0 + 1
}

/// 把行范围 `range` 限制在 `lines` 之内。
fn clamp_lines(range: &Range<usize>, lines: &Range<usize>) -> Range<usize> {
    range.start.max(lines.start)..range.end.min(lines.end)
}

/// 绘制自动换行时的选区 `selection`：每一段一块，相邻的块连成一片。
///
/// # Panics
///
/// `app` 没有当前文档时 panic。
pub(super) fn draw_wrapped_selection(
    app: &App,
    frame: &mut Frame<'_>,
    (selection, underline): (Selection, Option<Color>),
    segments: &[Range<usize>],
    origin_x: f32,
    row_y: &dyn Fn(usize) -> f64,
) {
    let doc = app.editor.active().expect("调用方保证有当前文档");
    let text = doc.text();
    let (cell, tab_width, clip, lh) = (
        app.metrics.cell,
        doc.tab_width(app.editor.tab_width),
        app.layout.text,
        app.fonts.line_height,
    );
    // 屏幕上从上到下的每一段：（y，选中部分的 x 范围）。
    let mut rows: Vec<(f32, Option<(f32, f32)>)> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    let lines = selection_lines(text, selection);
    for range in segments.iter().map(|r| clamp_lines(r, &lines)) {
        if range.is_empty() {
            continue;
        }
        text.visit_lines(range.start, MAX_LINE_BYTES, |line, line_start, content| {
            if line >= range.end {
                return ControlFlow::Break(());
            }
            let y = row_y(line);
            // 只测量可见段及上下各一段（用于判断圆角）。按整行断开的长行可达数千段，这类行只取
            // 上述各段（行内窗口），以下位置均相对窗口开头 `line_start`。
            let count = doc.rows_of(line);
            let shown = visible_parts(y, count, clip, lh);
            let parts = shown.start.saturating_sub(1)..(shown.end + 1).min(count);
            let window = (count > 1 && exceeds_head(content))
                .then(|| doc.line_window(line, parts.clone()))
                .flatten();
            let (chars, breaks, truncated) = match &window {
                Some(w) => (&w.chars[..], w.breaks.clone(), false),
                None => {
                    chars.clear();
                    chars.extend(content.chars().take(MAX_LINE_CHARS + 1));
                    let truncated = chars.len() > MAX_LINE_CHARS;
                    chars.truncate(MAX_LINE_CHARS);
                    let breaks = fit_breaks(doc.line_breaks(line), chars.len());
                    (&chars[..], breaks, truncated)
                }
            };
            let (first_part, first_col, ends) =
                (window.as_deref()).map_or((0, 0, true), |w| (w.part, w.col, w.ends));
            let line_start = line_start + first_col;
            let len = chars.len();
            let line_end = line_end(text, line, line_start, len, truncated);
            for (whole_part, part) in parts
                .filter_map(|p| Some((p, p.checked_sub(first_part)?)))
                .filter(|&(_, part)| part < breaks.rows())
            {
                let r = breaks.range(part, len);
                let r = r.start.min(len)..r.end.min(len);
                let (a, b) = (line_start + r.start, line_start + r.end);
                let last = ends && part + 1 == breaks.rows();
                // 该段的末尾：最后一段为行尾，其他段到下一段的开头（不含）。
                let end = if last { line_end } else { b };
                let x_at = |pos: usize| {
                    let col = (pos - line_start).min(len);
                    let span = &chars[r.start..col.max(r.start)];
                    origin_x
                        + text::slice_cells(span, breaks.indent_of(part), tab_width) as f32 * cell
                };
                // 最后一段按行尾判断；其他段只检查该段内的字符。
                let covered = if last {
                    selection.from() <= end && selection.to() > a
                } else {
                    selection.from() < end && selection.to() > a
                };
                let span = covered.then(|| {
                    let x0 = x_at(selection.from().max(a));
                    let mut x1 = x_at(selection.to().min(end));
                    // 选区跨过行尾时多绘制一小截（0.6 个字符宽），表示换行符也被选中。
                    if last && selection.to() > end {
                        x1 += cell * 0.6;
                    }
                    (x0, x1.max(x0 + 1.0))
                });
                rows.push(((y + whole_part as f64 * f64::from(lh)) as f32, span));
            }
            ControlFlow::Continue(())
        });
    }

    let color = selection_color(app);
    let covers = |other: Option<(f32, f32)>, x: f32| {
        other.is_some_and(|(a, b)| a <= x + 0.5 && b >= x - 0.5)
    };
    for (i, &(y, span)) in rows.iter().enumerate() {
        let Some((x0, x1)) = span else { continue };
        if let Some(color) = underline {
            frame.rect_clipped(underline_rect(x0, x1, y, lh), color, 0.0, clip);
            continue;
        }
        let above = i.checked_sub(1).and_then(|j| rows[j].1);
        let below = rows.get(i + 1).and_then(|r| r.1);
        let radius = |covered: bool| if covered { 0.0 } else { RADIUS_SELECTION };
        let radii = [
            radius(covers(above, x0 + 1.0)),
            radius(covers(above, x1 - 1.0)),
            radius(covers(below, x1 - 1.0)),
            radius(covers(below, x0 + 1.0)),
        ];
        frame.rect_corners(Rect::new(x0, y, x1 - x0, lh), color, radii, clip);
    }
}

/// 返回一行中 `x0` 到 `x1` 一段文字的下划线：线宽 1 逻辑像素，距行底 2 逻辑像素。
fn underline_rect(x0: f32, x1: f32, y: f32, line_height: f32) -> Rect {
    Rect::new(x0, y + line_height - 2.0, x1 - x0, 1.0)
}

/// 返回选区的底色；编辑区没有键盘焦点时使用较淡的 `Palette::selection_inactive`。
fn selection_color(app: &App) -> Color {
    if app.editor_focused() {
        app.palette.selection
    } else {
        app.palette.selection_inactive
    }
}

/// 绘制选区 `selection`：每行一块，跨行时相邻的块相连，只有外露的角绘制为圆角。
///
/// `columns` 是 CSV 按列对齐时各列的位置（调用方每帧取一次）。
///
/// # Panics
///
/// `app` 没有当前文档时 panic。
pub(super) fn draw_selection(
    app: &App,
    frame: &mut Frame<'_>,
    (selection, underline): (Selection, Option<Color>),
    segments: &[Range<usize>],
    columns: Option<&Columns>,
    origin_x: f32,
    row_y: &dyn Fn(usize) -> f32,
) {
    let doc = app.editor.active().expect("调用方保证有当前文档");
    let text = doc.text();
    // 选区在文本区右边缘之外的部分不可见，各行只读取到右边缘（见 `line_read_cap`）。
    let span = (origin_x, app.layout.text.right(), app.metrics.cell);
    let (cell, tab_width, clip) = (
        app.metrics.cell,
        doc.tab_width(app.editor.tab_width),
        app.layout.text,
    );
    let cap = line_read_cap(doc, columns.is_some(), span, (clip.x, tab_width));
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return;
    };

    // 上下各多取一行可见行，用于判断首尾两行的角是否需要圆角。
    let mut ranges = Vec::with_capacity(segments.len() + 2);
    // 第一个显示行之前没有可见行时（它是第 0 个显示行），不取上一行。
    if let Some(row) = doc.row_of(first.start).checked_sub(1) {
        let above = doc.line_of_row(row);
        ranges.push(above..above + 1);
    }
    ranges.extend(segments.iter().cloned());
    let below = doc.folds().visible_from(last.end);
    if below < text.line_count() {
        ranges.push(below..below + 1);
    }

    let mut spans: Vec<(usize, Option<(f32, f32)>)> = Vec::new();
    // CSV 按列对齐时各字符的显示列：各行共用一个缓冲区，只计算到选区在本行的终点为止。
    let mut cells = Vec::new();
    let lines = selection_lines(text, selection);
    for range in ranges.iter().map(|r| clamp_lines(r, &lines)) {
        if range.is_empty() {
            continue;
        }
        text.visit_lines(range.start, cap, |line, line_start, content| {
            if line >= range.end {
                return ControlFlow::Break(());
            }

            let shown = content.chars().take(MAX_LINE_CHARS + 1).count();
            let truncated = shown > MAX_LINE_CHARS || cut_by(content, cap);
            let line_end = line_end(text, line, line_start, shown.min(MAX_LINE_CHARS), truncated);

            let span = (selection.from() <= line_end && selection.to() > line_start).then(|| {
                let slice = RopeSlice::from(content);
                if let Some(c) = columns {
                    let to = selection.to().min(line_end) - line_start;
                    c.layout(slice, tab_width, to, &mut cells, None);
                }
                // 显示整行的超长行：选区可以超出读到的开头部分，按列索引换算。
                let whole = truncated && doc.shows_whole_line(line);
                let x_of = |pos: usize| {
                    let col = pos - line_start;
                    let cells = match columns {
                        Some(_) => cells[col.min(cells.len() - 1)],
                        None if whole => doc.col_to_cell(line, col, tab_width),
                        None => text::col_to_cell(slice, col, tab_width),
                    };
                    // 展开的行可达数百万列，按 `f64` 相加。
                    (f64::from(origin_x) + cells as f64 * f64::from(cell)) as f32
                };
                let x0 = x_of(selection.from().max(line_start));
                let mut x1 = x_of(selection.to().min(line_end));

                // 选区跨过行尾时多绘制一小截（0.6 个字符宽），表示换行符也被选中。
                if selection.to() > line_end {
                    x1 += cell * 0.6;
                }

                (x0, x1)
            });
            spans.push((line, span));
            ControlFlow::Continue(())
        });
    }

    // 某个角被相邻行的选区覆盖时不绘制圆角。
    let color = selection_color(app);
    let covers = |other: Option<(f32, f32)>, x: f32| {
        other.is_some_and(|(a, b)| a <= x + 0.5 && b >= x - 0.5)
    };

    for (i, &(line, span)) in spans.iter().enumerate() {
        let Some((x0, x1)) = span else { continue };

        if !segments.iter().any(|r| r.contains(&line)) {
            continue;
        }
        if let Some(color) = underline {
            let rect = underline_rect(x0, x1, row_y(line), app.fonts.line_height);
            frame.rect_clipped(rect, color, 0.0, clip);
            continue;
        }

        let above = i.checked_sub(1).and_then(|j| spans[j].1);
        let below = spans.get(i + 1).and_then(|s| s.1);
        let radius = |covered: bool| if covered { 0.0 } else { RADIUS_SELECTION };
        let radii = [
            radius(covers(above, x0 + 1.0)),
            radius(covers(above, x1 - 1.0)),
            radius(covers(below, x1 - 1.0)),
            radius(covers(below, x0 + 1.0)),
        ];
        let rect = Rect::new(x0, row_y(line), x1 - x0, app.fonts.line_height);
        frame.rect_corners(rect, color, radii, clip);
    }
}

/// 绘制光标，或输入法正在组字的文本及其中的光标；返回输入法候选框应对齐的矩形。
pub(in crate::view) fn draw_caret(
    app: &App,
    frame: &mut Frame<'_>,
    x: f32,
    y: f32,
    baseline: f32,
    clip: Rect,
) -> Rect {
    let p = app.palette;
    let (cell, tab_width) = (app.metrics.cell, app.editor.active_tab_width());
    let preedit = app.preedit_at(Focus::Editor);

    if preedit.is_empty() {
        caret_bar(app, frame, x, y, clip);
        return caret_rect(x, y, app.fonts.line_height);
    }

    let chars: Vec<char> = preedit.chars().collect();
    let cells = loose_cells(&chars, tab_width);
    let width = cells.iter().sum::<usize>() as f32 * cell;
    frame.rect_clipped(
        Rect::new(x, y, width, app.fonts.line_height),
        p.editor_bg,
        0.0,
        clip,
    );
    let mut px = x;
    let font = (FontKind::Mono, app.fonts.code, cell);
    // emoji 序列随首字符整体绘制；`joined` 之前的字符已经绘制。
    let mut joined = 0;

    for (col, &width) in cells.iter().enumerate() {
        if col >= joined {
            let at = (px, baseline);
            joined = col + draw_grid_char(frame, font, at, (&chars, col), p.text, clip);
        }
        px += width as f32 * cell;
    }

    frame.rect_clipped(
        Rect::new(x, baseline + 3.0, width, 1.0),
        p.accent,
        0.0,
        clip,
    );
    let at = preedit_cursor_x(app, preedit, &cells, x, cell);
    if let Some(at) = at {
        caret_bar(app, frame, at, y, clip);
    }
    caret_rect(at.unwrap_or(px), y, app.fonts.line_height)
}

/// 返回按格排列的组字文字 `text` 中输入法光标的横坐标（[`App::preedit_cursor`]）：`cells` 是
/// 各字所占的格数，`x` 是组字文字的左边缘，`cell` 是一格的宽度。没有光标时为 `None`。
pub(in crate::view) fn preedit_cursor_x(
    app: &App,
    text: &str,
    cells: &[usize],
    x: f32,
    cell: f32,
) -> Option<f32> {
    let before = text[..app.preedit_cursor(text)?].chars().count();
    Some(x + cells[..before].iter().sum::<usize>() as f32 * cell)
}

/// 绘制 Vim 模式的光标：覆盖字符 `c` 的块、字符下方的横线，或空心的块。
///
/// `(x, y, width)` 是字符的位置与宽度，`baseline` 是所在行的基线，
/// `under` 是光标所在的行和列（行尾之后为 `None`）。光标位于 emoji 序列上时块覆盖整个序列。
/// 实心块之下的字符用背景色重绘；实心块与横线随光标闪烁，空心块不闪烁。
pub(in crate::view) fn draw_modal_caret(
    app: &App,
    frame: &mut Frame<'_>,
    (x, y, width): (f32, f32, f32),
    under: Option<(&[char], usize)>,
    baseline: f32,
    shape: CaretShape,
    clip: Rect,
) {
    let p = app.palette;
    let cell = app.metrics.cell;
    let under = under.filter(|&(chars, col)| col < chars.len());
    // 光标位于 emoji 序列上时块覆盖整个序列，否则取调用方给出的宽度。
    let width = under.map_or(width, |(chars, col)| {
        let len = winc_emoji::sequence_len(chars[col..].iter().copied());
        let span = text::sequence_cells(chars[col..col + len].iter().copied(), 0, 1);
        if len > 1 && span > 0 {
            span as f32 * cell
        } else {
            width
        }
    });
    let block = Rect::new(x.round(), y, width.round().max(1.0), app.fonts.line_height);
    match shape {
        CaretShape::Hollow => {
            for edge in [
                Rect::new(block.x, block.y, block.w, 1.0),
                Rect::new(block.x, block.bottom() - 1.0, block.w, 1.0),
                Rect::new(block.x, block.y, 1.0, block.h),
                Rect::new(block.right() - 1.0, block.y, 1.0, block.h),
            ] {
                frame.rect_clipped(edge, p.caret, 0.0, clip);
            }
        }
        CaretShape::Underline if app.blink.visible => {
            let line = Rect::new(block.x, block.bottom() - 2.0, block.w, 2.0);
            frame.rect_clipped(line, p.caret, 0.0, clip);
        }
        CaretShape::Block if app.blink.visible => {
            frame.rect_clipped(block, p.caret, 1.0, clip);
            if let Some((chars, col)) = under.filter(|&(chars, col)| !chars[col].is_whitespace()) {
                let font = (FontKind::Mono, app.fonts.code, cell);
                let at = (x, baseline);
                draw_grid_char(frame, font, at, (chars, col), p.editor_bg, clip);
            }
        }
        CaretShape::Underline | CaretShape::Block => {}
    }
}

/// 绘制可编辑文字中的主光标，形状取自 [`App::styled_caret`]；返回输入法候选框应对齐的矩形。
///
/// `(x, y, width)` 与 `under` 见 [`draw_modal_caret`]；输入法组字时按 [`draw_caret`] 绘制。
pub(in crate::view) fn draw_text_caret(
    app: &App,
    frame: &mut Frame<'_>,
    (x, y, width): (f32, f32, f32),
    under: Option<(&[char], usize)>,
    baseline: f32,
    clip: Rect,
) -> Rect {
    match app.styled_caret() {
        Some(shape) if app.preedit_at(Focus::Editor).is_empty() => {
            draw_modal_caret(app, frame, (x, y, width), under, baseline, shape, clip);
            caret_rect(x, y, app.fonts.line_height)
        }
        _ => draw_caret(app, frame, x, y, baseline, clip),
    }
}

/// 绘制可编辑文字中主光标以外的光标，形状取自 [`App::styled_caret`]；参数见 [`draw_text_caret`]。
pub(in crate::view) fn draw_extra_caret(
    app: &App,
    frame: &mut Frame<'_>,
    (x, y, width): (f32, f32, f32),
    under: Option<(&[char], usize)>,
    baseline: f32,
    clip: Rect,
) {
    match app.styled_caret() {
        Some(shape) => draw_modal_caret(app, frame, (x, y, width), under, baseline, shape, clip),
        None => caret_bar(app, frame, x, y, clip),
    }
}

/// 在一行的顶边 `y` 处、横坐标 `x` 绘制一道光标竖线；处于闪烁周期的熄灭阶段时不绘制。
pub(in crate::view) fn caret_bar(app: &App, frame: &mut Frame<'_>, x: f32, y: f32, clip: Rect) {
    if app.blink.visible {
        let bar = Rect::new(x.round(), y + 1.0, 2.0, app.fonts.line_height - 2.0);
        frame.rect_clipped(bar, app.palette.caret, 1.0, clip);
    }
}

/// 返回光标旁括号及与之配对的括号的位置；没有时返回 `None`。
///
/// 结果按（文档、版本号、光标位置）缓存：在大文件中向前查找需要读取多行，光标不动时无需每帧重算。
pub(super) fn matching_brace(app: &mut App, index: usize) -> Option<(usize, usize)> {
    let doc = &app.editor.documents[index];
    let key = BraceKey {
        doc: doc.id,
        revision: doc.revision,
        head: doc.selection.head,
    };

    if let Some((cached, found)) = app.caches.brace_cache
        && cached == key
    {
        return found;
    }

    let found = movement::matching_bracket(doc.text(), doc.selection.head, BRACE_SCAN_LIMIT);
    app.caches.brace_cache = Some((key, found));
    found
}
