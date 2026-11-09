//! 选区、光标和配对的括号。

use super::*;

/// 返回 `y` 处开始、共 `rows` 段的一行中与 `clip` 相交的段（左闭右开）；`lh` 是行高。
pub(super) fn visible_parts(y: f32, rows: usize, clip: Rect, lh: f32) -> Range<usize> {
    let first = ((clip.y - y) / lh).floor().max(0.0) as usize;
    let end = ((clip.bottom() - y) / lh).ceil().max(0.0) as usize;
    first.min(rows)..end.min(rows)
}

/// 逐行读取时返回的 `content` 是否可能只是该行的开头部分（读取上限为 [`MAX_LINE_BYTES`]）。
pub(super) fn reads_head_only(content: &str) -> bool {
    cut_by(content, MAX_LINE_BYTES)
}

/// 按 `cap` 字节的上限逐行读取时，返回的 `content` 是否可能只是该行的开头部分。
///
/// 截断处不切开字符，截断后的内容最多比上限少 3 个字节（UTF-8 字符最长 4 字节）。
pub(super) fn cut_by(content: &str, cap: usize) -> bool {
    content.len() + 3 >= cap
}

/// 返回绘制文档 `doc` 时逐行读取的字节上限（字节）。
///
/// `aligned` 表示是否按列对齐；`origin_x` 是第 0 个显示列的 x 坐标，`right` 是文本区右边缘，
/// `cell` 是显示列的宽度（均为逻辑像素）。
///
/// 大文件不换行时只需读取到右边缘：每个字符至少占 1 个显示列、最多 4 字节，另留 2 列余量。
/// 一屏数十条超长行各按 [`MAX_LINE_BYTES`] 读取并解码，实测约占一帧的两成。其余情况
/// （行在内存中，或换行、按列对齐需要更多字符）返回 `MAX_LINE_BYTES`。
pub(super) fn line_read_cap(
    doc: &winc_editor::Document,
    aligned: bool,
    (origin_x, right, cell): (f32, f32, f32),
) -> usize {
    if !doc.is_large() || doc.is_wrapping() || aligned {
        return MAX_LINE_BYTES;
    }
    let cols = ((right - origin_x) / cell).ceil().max(0.0) as usize;
    ((cols + 2) * 4).min(MAX_LINE_BYTES)
}

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
    selection: Selection,
    segments: &[Range<usize>],
    origin_x: f32,
    row_y: &dyn Fn(usize) -> f32,
) {
    let doc = app.editor.active().expect("调用方保证有当前文档");
    let text = doc.text();
    let (cell, tab_width, clip, lh) = (
        app.cell,
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
            let window = (count > 1 && reads_head_only(content))
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
                rows.push((y + whole_part as f32 * lh, span));
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
    selection: Selection,
    segments: &[Range<usize>],
    columns: Option<&Columns>,
    origin_x: f32,
    row_y: &dyn Fn(usize) -> f32,
) {
    let doc = app.editor.active().expect("调用方保证有当前文档");
    let text = doc.text();
    // 选区在文本区右边缘之外的部分不可见，各行只读取到右边缘（见 `line_read_cap`）。
    let span = (origin_x, app.layout.text.right(), app.cell);
    let cap = line_read_cap(doc, columns.is_some(), span);
    let (cell, tab_width, clip) = (
        app.cell,
        doc.tab_width(app.editor.tab_width),
        app.layout.text,
    );
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
                let x_of = |pos: usize| {
                    let col = pos - line_start;
                    let cells = match columns {
                        Some(_) => cells[col.min(cells.len() - 1)],
                        None => text::col_to_cell(slice, col, tab_width),
                    };
                    origin_x + cells as f32 * cell
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

/// 绘制光标，或输入法正在组字的文本；返回输入法候选框应对齐的矩形。
pub(in crate::view) fn draw_caret(
    app: &App,
    frame: &mut Frame<'_>,
    x: f32,
    y: f32,
    baseline: f32,
    clip: Rect,
) -> Rect {
    let p = app.palette;
    let (cell, tab_width, preedit) = (app.cell, app.editor.active_tab_width(), &app.preedit);

    if preedit.is_empty() {
        caret_bar(app, frame, x, y, clip);
        return Rect::new(x, y, 2.0, app.fonts.line_height);
    }

    let cells = |c: char| text::char_cells(c, 0, tab_width).max(1);
    let width = preedit.chars().map(cells).sum::<usize>() as f32 * cell;
    frame.rect_clipped(
        Rect::new(x, y, width, app.fonts.line_height),
        p.editor_bg,
        0.0,
        clip,
    );
    let mut px = x;

    for c in preedit.chars() {
        frame.cell_char(
            FontKind::Mono,
            app.fonts.code,
            px,
            baseline,
            cell,
            cells(c),
            c,
            p.text,
            clip,
        );
        px += cells(c) as f32 * cell;
    }

    frame.rect_clipped(
        Rect::new(x, baseline + 3.0, width, 1.0),
        p.accent,
        0.0,
        clip,
    );
    Rect::new(px, y, 2.0, app.fonts.line_height)
}

/// 在一行的顶边 `y` 处、横坐标 `x` 绘制一道光标竖线；处于闪烁周期的熄灭阶段时不绘制。
pub(in crate::view) fn caret_bar(app: &App, frame: &mut Frame<'_>, x: f32, y: f32, clip: Rect) {
    if app.caret_visible {
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

    if let Some((cached, found)) = app.brace_cache
        && cached == key
    {
        return found;
    }

    let found = movement::matching_bracket(doc.text(), doc.selection.head, BRACE_SCAN_LIMIT);
    app.brace_cache = Some((key, found));
    found
}
