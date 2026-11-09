//! 编辑区右侧的缩略图：每行代码绘制为一道彩色小块，半透明的滑块标出当前显示的范围。
//!
//! 只绘制缩略图容纳得下的行；文档长于缩略图时，缩略图随滚动位置按比例移动。自动换行时每个显示行
//! 各占一道；按整行断开的长行只读取缩略图范围内的若干段。

use std::collections::HashMap;
use std::ops::ControlFlow;

use winc_syntax::Token;

use super::*;
use crate::theme::MINIMAP_ROW;

/// 一列字符的宽度（逻辑像素）。
///
/// 取自调参：缩略图可容纳约 100 列代码，长行不会在右边缘整齐截断。
const CELL: f32 = 0.75;
/// 小块左边留出的空隙（逻辑像素）。
const PAD: f32 = 4.0;
/// 滑块的圆角半径（逻辑像素）。
const SLIDER_RADIUS: f32 = 3.0;
/// 一道小块的高度（逻辑像素），略低于行高，相邻两行之间留出间隙。
const STROKE: f32 = 1.5;
/// 小块的不透明度（0 到 1）。
///
/// 取自调参：比正文淡，缩略图不分散对代码的注意力。
const OPACITY: f32 = 0.75;
/// 注释小块的不透明度（0 到 1）。
///
/// 取自调参：低于 `OPACITY`，使代码与注释易于区分。
const COMMENT_OPACITY: f32 = 0.4;

/// 缩略图中各行的小块，跨帧复用。
///
/// 缩略图同时显示数百行，相邻两帧通常只相差数行。不换行时按行号缓存，每帧只读取并着色
/// 新进入范围的行；文档、版本或显示条件（[`RowsKey`]）变化时全部失效。
#[derive(Default)]
pub(crate) struct MinimapRows {
    key: Option<RowsKey>,
    rows: HashMap<usize, CachedRow>,
}

/// [`MinimapRows`] 的有效条件：（文档，版本号，列数，Tab 宽度，语言的标识）。
type RowsKey = (winc_editor::DocumentId, u64, usize, usize, &'static str);

/// 缓存的一行：行首与下一行行首的高亮状态，以及各小块的（起始列，结束列，类别）。
struct CachedRow {
    state: LineState,
    next: LineState,
    strokes: Vec<(u16, u16, TokenKind)>,
}

/// [`MinimapRows`] 最多缓存的行数（行）；超出时全部清空。
///
/// 远大于缩略图可同时显示的行数（按 [`MINIMAP_ROW`] 计，2000 逻辑像素高时为 1000 行），
/// 连续滚动时不会反复清空。
const ROWS_MAX: usize = 4096;

/// 绘制缩略图；当前布局没有缩略图时不执行任何操作。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_minimap(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let area = app.layout.minimap;
    if area.w <= 0.0 || area.h <= 0.0 {
        return;
    }
    let p = app.palette;
    let (first, slider, active) = app.minimap_view();
    let shown = (area.h / MINIMAP_ROW).ceil() as usize;
    let tab_width = app.editor.documents[index].tab_width(app.editor.tab_width);
    let doc = &app.editor.documents[index];
    let segments: Vec<_> = doc
        .folds()
        .segments(first, shown, doc.text().line_count())
        .into_iter()
        .collect();
    let language = doc.highlighter.language();

    let color = if active { p.selected_inactive } else { p.hover };
    frame.rect_clipped(slider, color, SLIDER_RADIUS, area);

    // 光标所在行的底色绘制在代码下方，标出光标的位置。
    let caret = doc.row_of(doc.text().line_of(doc.selection.head));
    let caret = Rect::new(
        area.x,
        area.y + (caret as f32 - first as f32) * MINIMAP_ROW,
        area.w,
        STROKE,
    );
    frame.rect_clipped(caret, p.accent.alpha(p.accent.a * 0.5), 0.0, area);

    let columns = ((area.w - PAD) / CELL) as usize;
    // 大文件不换行时每行只读取 `columns` 列所需的字节（每个字符至少 1 列、最多 4 字节）。
    // 缩略图同时显示数百行，在每行约 1 MB 的文件中按 `MAX_LINE_BYTES` 读取，实测约占一帧的
    // 七成（约 15 ms）。普通文档仍读取 `MAX_LINE_BYTES`，使高亮状态的顺推与编辑区一致。
    let cap = if doc.is_large() && !doc.is_wrapping() {
        (columns + 1) * 4
    } else {
        MAX_LINE_BYTES
    };
    let (mut chars, mut tokens) = (Vec::new(), Vec::new());
    let row_y = |row: usize| area.y + (row as f32 - first as f32) * MINIMAP_ROW;
    let stroke = |frame: &mut Frame<'_>, y: f32, cells: std::ops::Range<usize>, kind| {
        let color = p.token(kind);
        let rect = Rect::new(
            area.x + PAD + cells.start as f32 * CELL,
            y,
            cells.len() as f32 * CELL,
            STROKE,
        );
        let opacity = if kind == TokenKind::Comment {
            COMMENT_OPACITY
        } else {
            OPACITY
        };
        frame.rect_clipped(rect, color.alpha(color.a * opacity), 0.0, area);
    };

    // 不换行时每行一道，按行缓存。
    if !doc.is_wrapping() {
        let mut cache = app.minimap_rows.borrow_mut();
        let key = (doc.id, doc.revision, columns, tab_width, language.id);
        if cache.key != Some(key) || cache.rows.len() > ROWS_MAX {
            cache.key = Some(key);
            cache.rows.clear();
        }
        for range in &segments {
            let mut state = LineState::Normal;
            for line in range.clone() {
                let y = row_y(doc.row_of(line));
                if y >= area.bottom() {
                    break;
                }
                // 不额外推算高亮：已推算出的行首状态直接采用，其余的行由上一行顺推。
                state = doc.known_highlight_state(line).unwrap_or(state);
                if cache.rows.get(&line).is_none_or(|row| row.state != state) {
                    let content = doc.text().line_prefix(line, cap);
                    chars.clear();
                    chars.extend(content.slice().chars().take(MAX_LINE_CHARS + 1));
                    // 超过 `MAX_LINE_CHARS` 个字符或只读取了前若干列（见 `cap`）的行
                    // 不改变下一行的状态；后者只需着色已读取的列。
                    let cut = cap < MAX_LINE_BYTES && chars.len() > columns;
                    let long = cut || chars.len() > MAX_LINE_CHARS;
                    chars.truncate(if cut { columns } else { MAX_LINE_CHARS });
                    let next =
                        app.highlight_cached(doc, line, language, &chars, state, &mut tokens);
                    let mut row = CachedRow {
                        state,
                        next: if long { state } else { next },
                        strokes: Vec::new(),
                    };
                    let whole = (0..chars.len(), 0);
                    strokes(
                        (&chars, &tokens),
                        whole,
                        tab_width,
                        columns,
                        |cells, kind| {
                            row.strokes
                                .push((cells.start as u16, cells.end as u16, kind));
                        },
                    );
                    cache.rows.insert(line, row);
                }
                let row = &cache.rows[&line];
                state = row.next;
                if y + STROKE > area.y {
                    for &(from, to, kind) in &row.strokes {
                        stroke(frame, y, usize::from(from)..usize::from(to), kind);
                    }
                }
            }
        }
        return;
    }

    for range in &segments {
        let mut state = LineState::Normal;
        doc.text()
            .visit_lines(range.start, cap, |line, _, content| {
                let top = doc.row_of(line);
                if line >= range.end || row_y(top) >= area.bottom() {
                    return ControlFlow::Break(());
                }
                chars.clear();
                chars.extend(content.chars().take(MAX_LINE_CHARS + 1));
                // 不额外推算高亮：已推算出的行首状态直接采用，其余的行由上一行顺推。
                // 超过 `MAX_LINE_CHARS` 个字符或没有读到行尾的行不改变下一行开头的状态。
                let long = chars.len() > MAX_LINE_CHARS || content.len() + 3 >= cap;
                chars.truncate(MAX_LINE_CHARS);
                state = doc.known_highlight_state(line).unwrap_or(state);
                let next = app.highlight_cached(doc, line, language, &chars, state, &mut tokens);
                if !long {
                    state = next;
                }
                // 按整行断开且 `content` 不是整行时，只取缩略图范围内的若干段（行内窗口）；
                // 着色只覆盖前 `MAX_LINE_CHARS` 个字符，窗口在其后时按普通文本绘制。
                let rows = doc.rows_of(line);
                let window = (rows > 1 && reads_head_only(content))
                    .then(|| {
                        let from = first.saturating_sub(top).min(rows - 1);
                        let to = (first + shown).saturating_sub(top).min(rows);
                        doc.line_window(line, from..to)
                    })
                    .flatten();
                let (shown_chars, breaks) = match &window {
                    Some(w) => (&w.chars[..], w.breaks.clone()),
                    None => (&chars[..], fit_breaks(doc.line_breaks(line), chars.len())),
                };
                let (first_part, first_col) = window.as_deref().map_or((0, 0), |w| (w.part, w.col));
                if first_col > 0 {
                    tokens.retain_mut(|token| {
                        token.start = token.start.saturating_sub(first_col);
                        token.end = token.end.saturating_sub(first_col);
                        token.end > 0
                    });
                }
                for part in 0..breaks.rows() {
                    let y = row_y(top + first_part + part);
                    if y + STROKE <= area.y {
                        continue;
                    }
                    if y >= area.bottom() {
                        break;
                    }
                    let len = shown_chars.len();
                    let range = breaks.range(part, len);
                    let segment = (range.start..range.end.min(len), breaks.indent_of(part));
                    strokes(
                        (shown_chars, &tokens),
                        segment,
                        tab_width,
                        columns,
                        |cells, kind| stroke(frame, y, cells, kind),
                    );
                }
                ControlFlow::Continue(())
            });
    }
}

/// 把一个显示行切分为连续的小块，对每块调用 `f(列范围, 类别)`：空白不绘制，同一类别的相邻字符
/// 合并为一块；只处理前 `columns` 列。
///
/// `line` 是整行的字符和词法单元，`segment` 是该显示行的字符范围和缩进（列）。
fn strokes(
    (chars, tokens): (&[char], &[Token]),
    (range, indent): (std::ops::Range<usize>, usize),
    tab_width: usize,
    columns: usize,
    mut f: impl FnMut(std::ops::Range<usize>, TokenKind),
) {
    let mut run: Option<(std::ops::Range<usize>, TokenKind)> = None;
    let (mut cell, mut next) = (indent, 0);
    for (i, &c) in chars.iter().enumerate().take(range.end).skip(range.start) {
        if cell >= columns {
            break;
        }
        while next < tokens.len() && tokens[next].end <= i {
            next += 1;
        }
        let kind = tokens
            .get(next)
            .filter(|token| token.start <= i)
            .map_or(TokenKind::Text, |token| token.kind);
        let width = match c {
            '\t' => tab_width.max(1) - cell % tab_width.max(1),
            c if c.is_ascii() => 1,
            _ => 2,
        };
        let blank = c.is_whitespace();
        match &mut run {
            Some((cells, same)) if !blank && *same == kind && cells.end == cell => {
                cells.end = cell + width;
            }
            _ => {
                if let Some((cells, kind)) = run.take() {
                    f(cells, kind);
                }
                if !blank {
                    run = Some((cell..cell + width, kind));
                }
            }
        }
        cell += width;
    }
    if let Some((cells, kind)) = run {
        f(cells.start..cells.end.min(columns), kind);
    }
}

#[cfg(test)]
mod tests;
