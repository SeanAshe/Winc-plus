//! 表格预览的绘制：xlsx 工作簿和 csv、tsv 显示成只读的表格。
//!
//! 表格的读取（xlsx 包、数字格式、单元格文字）在 `winc_preview::table`，这一层只负责排版、
//! 绘制和命中测试。列名和行号滚动时固定；单元格只显示第一行，放不下时截短，数字靠右；
//! 合并的单元格画成一格，选中的区域有底色和边框，单选一格时浮出卡片显示完整文字；
//! 多张工作表或表格被截断时，底部一栏列出各张工作表。

use std::borrow::Cow;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use winc_editor::DocumentId;
use winc_i18n::t;
use winc_preview::table::{Area, Sheet, Table, column_name};
use winc_render::{FontKind, Frame, Rect};

use crate::app::App;
use crate::theme::{RADIUS_POPUP, RADIUS_ROW};
use crate::view::{fit, ui_baseline};

/// 单元格左右的留白（逻辑像素，经验值）。
const PAD: f32 = 8.0;
/// 列宽的下限（逻辑像素，经验值）：文字窄的列也保持这个宽度，相邻列不至于挨在一起。
const COLUMN_MIN: f32 = 64.0;
/// 列宽的上限（逻辑像素，经验值）：更长的文字截短显示，完整内容由当前格的卡片显示。
const COLUMN_MAX: f32 = 320.0;
/// 定列宽时看前多少行（行，经验值）：再往下的行不影响列宽，打开大表时不必量遍所有单元格。
const MEASURED_ROWS: usize = 200;
/// 底部工作表一栏的高度（逻辑像素，经验值）。
const BAR: f32 = 36.0;
/// 当前格卡片的最小宽度（逻辑像素，经验值）；格子更宽时按格子的宽度。
const CARD_MIN_W: f32 = 280.0;
/// 当前格卡片的最大宽度（逻辑像素，经验值）；更宽时靠换行把文字排成更多行。
const CARD_MAX_W: f32 = 480.0;
/// 当前格卡片的最大高度（逻辑像素，经验值）；文字更多时在卡片中滚动。
const CARD_MAX_H: f32 = 320.0;
/// 不放在行首的字符：换行时留在上一行末尾，含常见的中英文标点和百分号。
const NO_LINE_START: &str = "，。、；：！？）」』》】〕,.;:!?)]}%";
/// 当前格卡片的内边距（逻辑像素，经验值）。
const CARD_PAD: f32 = 12.0;
/// 卡片与格子、与表格边缘的间距（逻辑像素，经验值）。
const CARD_GAP: f32 = 4.0;

/// 在 `area`（窗口坐标，逻辑像素）中绘制第 `index` 个文档读好的表格。
///
/// 文档没有读好的表格时不绘制。
///
/// # Panics
///
/// `index` 超出 `app.editor.documents` 的范围时 panic。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, index: usize, area: Rect) {
    let doc = &app.editor.documents[index];
    let Some(view) = doc.table.as_deref() else {
        return;
    };
    let (id, sheet_index, scroll) = (doc.id, view.sheet, view.scroll);
    let (selection, card_scroll, card_closed) =
        (view.selection, view.card_scroll, view.card_closed);
    let Some(table) = view.table().map(Arc::clone) else {
        return;
    };
    let Some(sheet) = table.sheets.get(sheet_index) else {
        return;
    };

    let p = app.palette;
    app.table_layout.area = area;
    frame.rect(area, p.editor_bg);
    let bar = (table.sheets.len() > 1 || sheet.truncated)
        .then(|| Rect::new(area.x, area.bottom() - BAR, area.w, BAR));

    let head = selection.1;
    let grid = Rect::new(area.x, area.y, area.w, area.h - bar.map_or(0.0, |b| b.h));

    let size = app.fonts.ui;
    let row_h = (size * 2.0).round();
    let numbers_w =
        frame.measure(FontKind::Ui, size, &sheet.rows.len().max(1).to_string()) + 2.0 * PAD;
    let numbers_w = numbers_w.max(40.0).round();
    let widths = column_widths(
        app,
        frame,
        (id, Arc::as_ptr(&table) as usize, sheet_index),
        sheet,
    );
    let mut edges = Vec::with_capacity(widths.len() + 1);
    edges.push(0.0);
    for w in &widths {
        edges.push(edges.last().copied().unwrap_or(0.0) + w);
    }

    let body = Rect::new(
        grid.x + numbers_w,
        grid.y + row_h,
        (grid.w - numbers_w).max(0.0),
        (grid.h - row_h).max(0.0),
    );
    let header = Rect::new(body.x, grid.y, body.w, row_h);
    let numbers = Rect::new(grid.x, body.y, numbers_w, body.h);
    let content = (
        edges.last().copied().unwrap_or(0.0),
        sheet.rows.len() as f32 * row_h,
    );
    let max_scroll = ((content.0 - body.w).max(0.0), (content.1 - body.h).max(0.0));
    let (sx, sy) = (
        scroll.0.clamp(0.0, max_scroll.0),
        scroll.1.clamp(0.0, max_scroll.1),
    );
    let layout = &mut app.table_layout;
    layout.max_scroll = max_scroll;
    layout.body = body;
    layout.header = header;
    layout.numbers = numbers;
    layout.row_h = row_h;
    layout.size = (sheet.rows.len(), sheet.columns);
    // 区域在窗口中的矩形（逻辑像素）：左上角的横纵坐标由列边界和行号换算。
    let area_rect = |a: &Area| {
        let x = body.x + edges[a.left] - sx;
        let y = body.y + a.top as f32 * row_h - sy;
        Rect::new(
            x,
            y,
            edges[a.right + 1] - edges[a.left],
            (a.bottom - a.top + 1) as f32 * row_h,
        )
    };

    // 可见的列：（列号，左边的横坐标，宽度）。
    let columns: Vec<(usize, f32, f32)> = widths
        .iter()
        .enumerate()
        .map(|(c, &w)| (c, body.x + edges[c] - sx, w))
        .skip_while(|&(_, x, w)| x + w <= body.x)
        .take_while(|&(_, x, _)| x < body.right())
        .collect();
    let first = (sy / row_h).floor() as usize;
    let last = (((sy + body.h) / row_h).ceil() as usize).min(sheet.rows.len());
    let visible = match (columns.first(), columns.last()) {
        (Some(l), Some(r)) if first < last => Some(Area {
            top: first,
            left: l.0,
            bottom: last - 1,
            right: r.0,
        }),
        _ => None,
    };
    let merges: Vec<Area> = visible.map_or_else(Vec::new, |v| {
        sheet
            .merges
            .iter()
            .filter(|m| m.overlaps(&v))
            .copied()
            .collect()
    });

    // 选中的区域：底色（当前格除外）先铺上，文字画在上面。
    let selected = sheet.cover_merges(Area::spanning(selection.0, selection.1));
    let current = sheet
        .merge_at(head.0, head.1)
        .unwrap_or(Area::spanning(head, head));
    let has_cells = sheet.columns > 0 && !sheet.rows.is_empty();
    let selected_rect = has_cells.then(|| {
        area_rect(&Area {
            bottom: selected.bottom.min(sheet.rows.len().saturating_sub(1)),
            right: selected.right.min(sheet.columns - 1),
            ..selected
        })
    });
    if let Some(rect) = selected_rect
        && selected != current
    {
        frame.rect_clipped(rect, p.selection, 0.0, body);
        frame.rect_clipped(area_rect(&current), p.editor_bg, 0.0, body);
    }

    // 单元格和格线；合并区域中的格之后整个画。
    for row in first..last {
        let y = body.y + row as f32 * row_h - sy;
        for &(column, x, w) in &columns {
            let text = sheet.cell(row, column);
            if text.is_empty() || merges.iter().any(|m| m.contains(row, column)) {
                continue;
            }
            let cell = Rect::new(x, y, w, row_h);
            draw_cell(app, frame, cell, text, cell.intersect(body));
        }
        frame.rect_clipped(
            Rect::new(body.x, y + row_h - 1.0, body.w, 1.0),
            p.separator,
            0.0,
            body,
        );
    }
    for &(_, x, w) in &columns {
        let line = Rect::new(x + w - 1.0, grid.y, 1.0, row_h + body.h.min(content.1 - sy));
        frame.rect_clipped(
            line,
            p.separator,
            0.0,
            Rect::new(body.x, grid.y, body.w, grid.h),
        );
    }
    // 合并区域：盖住内部的格线（在选区中的用选区的底色），文字放在整个区域中。
    for merge in &merges {
        let rect = area_rect(merge);
        let inner = Rect::new(rect.x, rect.y, rect.w - 1.0, rect.h - 1.0);
        let picked = selected != current && *merge != current && selected.overlaps(merge);
        let fill = if picked { p.selection } else { p.editor_bg };
        frame.rect_clipped(inner, fill, 0.0, body);
        let text = sheet.cell(merge.top, merge.left);
        if !text.is_empty() {
            draw_cell(app, frame, rect, text, rect.intersect(body));
        }
    }

    // 选中的区域外面一圈边框。
    if let Some(rect) = selected_rect {
        let edge = 2.0;
        for side in [
            Rect::new(rect.x - 1.0, rect.y - 1.0, rect.w + 1.0, edge),
            Rect::new(rect.x - 1.0, rect.bottom() - 1.0, rect.w + 1.0, edge),
            Rect::new(rect.x - 1.0, rect.y - 1.0, edge, rect.h + 1.0),
            Rect::new(rect.right() - 1.0, rect.y - 1.0, edge, rect.h + 2.0),
        ] {
            frame.rect_clipped(side, p.accent, 0.0, body);
        }
    }

    // 列名一行和行号一列：底色盖住滚到下面的单元格，选中的列、行加底色。
    frame.rect(header, p.sidebar_bg);
    for &(column, x, w) in &columns {
        let cell = Rect::new(x, grid.y, w, row_h);
        let picked = (selected.left..=selected.right).contains(&column);
        draw_label(app, frame, cell, header, &column_name(column), picked, true);
    }
    frame.rect(numbers, p.sidebar_bg);
    for row in first..last {
        let y = body.y + row as f32 * row_h - sy;
        let cell = Rect::new(grid.x, y, numbers_w, row_h);
        let picked = (selected.top..=selected.bottom).contains(&row);
        draw_label(
            app,
            frame,
            cell,
            numbers,
            &(row + 1).to_string(),
            picked,
            false,
        );
    }
    frame.rect(Rect::new(grid.x, grid.y, numbers_w, row_h), p.sidebar_bg);
    frame.rect(Rect::new(grid.x, body.y - 1.0, grid.w, 1.0), p.border);
    frame.rect(Rect::new(body.x - 1.0, grid.y, 1.0, grid.h), p.border);

    if sheet.columns == 0 {
        let message = t!("preview-table-empty");
        let text_w = frame.measure(FontKind::Ui, size, message);
        let row = Rect::new(body.x, body.y + 24.0, body.w, row_h);
        let left = (body.x + (body.w - text_w) / 2.0).round();
        let baseline = ui_baseline(app, row, size);
        frame.text(
            FontKind::Ui,
            size,
            left,
            baseline,
            message,
            p.text_dim,
            body,
        );
    }

    // 当前格的卡片：只选了一格（或一个合并区域）、文字没有显示全时浮在格子旁边；拖着选的
    // 时候、按 Esc 收起以后不显示。
    app.table_layout.card = Rect::default();
    let text = if has_cells {
        sheet.cell(current.top, current.left)
    } else {
        ""
    };
    let cell = area_rect(&current);
    let show_card = selected == current
        && !card_closed
        && !text.is_empty()
        && !app.table_dragging()
        && cell.intersect(body).w > 0.0
        && cell.intersect(body).h > 0.0
        && cut_off(frame, text, size, cell);
    if show_card {
        let key = (
            Arc::as_ptr(&table) as usize,
            sheet_index,
            (current.top, current.left),
        );
        draw_cell_card(app, frame, (body, cell), (id, key), text, card_scroll);
    }

    app.table_layout.edges = edges;
    app.table_layout.sheets.clear();
    if let Some(bar) = bar {
        draw_bar(app, frame, bar, &table, sheet_index);
    }
}

/// 绘制列名或行号的一格：`picked` 时加底色；`centered` 时居中（列名），否则靠右（行号）。
fn draw_label(
    app: &App,
    frame: &mut Frame<'_>,
    cell: Rect,
    clip: Rect,
    label: &str,
    picked: bool,
    centered: bool,
) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let clip = cell.intersect(clip);
    if picked {
        frame.rect_clipped(cell, p.selected, 0.0, clip);
    }
    let text_w = frame.measure(FontKind::Ui, size, label);
    let left = if centered {
        (cell.x + (cell.w - text_w) / 2.0).round()
    } else {
        cell.right() - PAD - text_w
    };
    let baseline = ui_baseline(app, cell, size);
    let color = if picked { p.text } else { p.text_muted };
    frame.text(FontKind::Ui, size, left, baseline, label, color, clip);
    if centered {
        let line = Rect::new(cell.right() - 1.0, cell.y, 1.0, cell.h);
        frame.rect_clipped(line, p.border, 0.0, clip);
    }
}

/// 绘制当前格的卡片，显示单元格 `cell` 中完整的文字 `text`。
///
/// 卡片浮在 `cell` 下方，下方放不下时在上方，浮在表格上、不挤动表格；文字按卡片宽度换行，
/// 高过 [`CARD_MAX_H`] 时在卡片中按 `scroll` 滚动。第四个参数是（文档，（表格的 `Arc` 指针
/// 地址，工作表序号，单元格））：同一格、同一宽度下换行的结果缓存起来，不必每帧重新量文字。
fn draw_cell_card(
    app: &mut App,
    frame: &mut Frame<'_>,
    (body, cell): (Rect, Rect),
    (id, (table, sheet, at)): (DocumentId, (usize, usize, (usize, usize))),
    text: &str,
    scroll: f32,
) {
    let p = app.palette;
    let size = app.fonts.ui;
    let line_h = (size * 1.5).round();
    let room = body.inset(CARD_GAP, CARD_GAP);
    let w = cell.w.clamp(CARD_MIN_W, CARD_MAX_W).min(room.w);
    if w <= 2.0 * CARD_PAD || room.h <= 0.0 {
        return;
    }
    let key = (id, table, sheet, at, w.to_bits(), size.to_bits());
    let lines = match app.table_layout.card_lines.take() {
        Some((cached, lines)) if cached == key => lines,
        _ => wrap(frame, text, size, w - 2.0 * CARD_PAD),
    };
    let content = lines.len() as f32 * line_h;
    let max_h = CARD_MAX_H.min(room.h * 0.6).max(line_h + 2.0 * CARD_PAD);
    let h = (content + 2.0 * CARD_PAD).min(max_h).round();
    let x = cell.x.min(room.right() - w).max(room.x);
    let below = cell.bottom() + CARD_GAP;
    let above = cell.y - CARD_GAP - h;
    let y = if below + h <= room.bottom() {
        below
    } else if above >= room.y {
        above
    } else {
        (room.bottom() - h).max(room.y)
    };
    let card = Rect::new(x.round(), y.round(), w.round(), h);
    let max_scroll = (content - (h - 2.0 * CARD_PAD)).max(0.0);
    let scroll = scroll.clamp(0.0, max_scroll);
    app.table_layout.card = card;
    app.table_layout.card_max = max_scroll;

    let covered = frame.covered(card, RADIUS_POPUP);
    frame.shadow(card, RADIUS_POPUP, 12.0, 4.0, p.shadow, Some(covered));
    frame.rect_clipped(card, p.popup_border, RADIUS_POPUP, body);
    frame.rect_clipped(card.inset(1.0, 1.0), p.popup_bg, RADIUS_POPUP - 1.0, body);
    let clip = card.inset(1.0, CARD_PAD - 4.0).intersect(body);
    for (i, line) in lines.iter().enumerate() {
        let top = card.y + CARD_PAD + i as f32 * line_h - scroll;
        if top + line_h < card.y || top > card.bottom() {
            continue;
        }
        let row = Rect::new(card.x + CARD_PAD, top, w - 2.0 * CARD_PAD, line_h);
        let baseline = ui_baseline(app, row, size);
        frame.text(FontKind::Ui, size, row.x, baseline, line, p.text, clip);
    }
    // 文字比卡片高时，右边一条细的滑块表示还能滚动、滚动到的位置。
    if max_scroll > 0.0 {
        let track = card.inset(3.0, 6.0);
        let thumb_h = (track.h * (h - 2.0 * CARD_PAD) / content).max(16.0);
        let y = track.y + (track.h - thumb_h) * scroll / max_scroll;
        let thumb = Rect::new(track.right() - 3.0, y, 3.0, thumb_h);
        frame.rect_clipped(thumb, p.scrollbar, 1.5, card);
    }
    app.table_layout.card_lines = Some((key, lines));
}

/// 单元格 `cell` 中的文字 `text` 是否没有显示全：有多行，或者第一行放不下。
fn cut_off(frame: &mut Frame<'_>, text: &str, size: f32, cell: Rect) -> bool {
    let (_, over, more) = visible(frame, text, size, cell.w - 2.0 * PAD);
    over || more
}

/// 返回单元格文字的第一行中显示得到的部分。
///
/// 返回（这部分文字，是否超过了 `room`，换行之后是否还有文字）。逐个字素量到总宽超过 `room`
/// 或遇到换行为止，超过时返回的部分包含越界的那个字素。只量开头，单元格的文字再长，每帧的
/// 开销也只和格子的宽度有关。
fn visible<'a>(
    frame: &mut Frame<'_>,
    text: &'a str,
    size: f32,
    room: f32,
) -> (&'a str, bool, bool) {
    let mut width = 0.0;
    for (i, g) in text.grapheme_indices(true) {
        if g.ends_with('\n') {
            let more = !text[i..].trim_start().is_empty();
            return (&text[..i], false, more);
        }
        width += frame.measure(FontKind::Ui, size, g);
        if width > room {
            return (&text[..i + g.len()], true, false);
        }
    }
    (text, false, false)
}

/// 把 `text` 按宽度 `max`（逻辑像素）换成多行。
///
/// 先按原有的换行分段，段内在放不下的地方断开：有空格时在最后一个空格处断，没有空格（如中文）
/// 时在字与字之间断。逗号、句号、右括号这类不放在行首的标点留在上一行末尾，允许稍微超出宽度。
fn wrap(frame: &mut Frame<'_>, text: &str, size: f32, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.strip_suffix('\r').unwrap_or(paragraph);
        let mut line = String::new();
        let mut width = 0.0;
        for c in paragraph.chars() {
            let w = frame.measure(FontKind::Ui, size, c.encode_utf8(&mut [0; 4]));
            if width + w > max && !line.is_empty() && !NO_LINE_START.contains(c) {
                let rest = match line.rfind(' ') {
                    Some(at) if at > 0 && !c.is_whitespace() => line.split_off(at + 1),
                    _ => String::new(),
                };
                lines.push(std::mem::replace(&mut line, rest));
                width = frame.measure(FontKind::Ui, size, &line);
                // 断在空格处时，空格不留到下一行的开头。
                if c == ' ' {
                    continue;
                }
            }
            line.push(c);
            width += w;
        }
        lines.push(line);
    }
    lines
}

/// 绘制一个单元格的文字：只取第一行，放不下时截短；看起来是数字的靠右。
fn draw_cell(app: &App, frame: &mut Frame<'_>, cell: Rect, text: &str, clip: Rect) {
    let size = app.fonts.ui;
    let room = cell.w - 2.0 * PAD;
    if room <= 0.0 {
        return;
    }
    let (first, over, more) = visible(frame, text, size, room);
    let shown = if over || more {
        Cow::Owned(format!("{first}…"))
    } else {
        Cow::Borrowed(first)
    };
    let fitted = fit(frame, &shown, size, room, false);
    let baseline = ui_baseline(app, cell, size);
    let left = if !over && is_numeric(first) {
        cell.right() - PAD - frame.measure(FontKind::Ui, size, &fitted)
    } else {
        cell.x + PAD
    };
    frame.text(
        FontKind::Ui,
        size,
        left,
        baseline,
        &fitted,
        app.palette.text,
        clip,
    );
}

/// 绘制底部一栏：左边各张工作表的标签（当前的一张有底色），右边说明表格是否被截断。
fn draw_bar(app: &mut App, frame: &mut Frame<'_>, bar: Rect, table: &Table, current: usize) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    frame.rect(bar, p.sidebar_bg);
    frame.rect(Rect::new(bar.x, bar.y, bar.w, 1.0), p.border);

    let mut right = bar.right() - 12.0;
    if table.sheets.get(current).is_some_and(|s| s.truncated) {
        let note = t!("preview-table-truncated");
        let w = frame.measure(FontKind::Ui, size, note);
        right -= w;
        let baseline = ui_baseline(app, bar, size);
        frame.text(FontKind::Ui, size, right, baseline, note, p.text_dim, bar);
        right -= 16.0;
    }

    let mut x = bar.x + 8.0;
    let mut tabs = Vec::new();
    for (index, sheet) in table.sheets.iter().enumerate() {
        let name = if sheet.name.is_empty() {
            t!("preview-table-sheet", number = index + 1)
        } else {
            sheet.name.clone()
        };
        let name = fit(frame, &name, size, 200.0, false).into_owned();
        let w = frame.measure(FontKind::Ui, size, &name) + 2.0 * 10.0;
        let tab = Rect::new(x, bar.y + 6.0, w, bar.h - 12.0);
        let clip = Rect::new(bar.x, bar.y, (right - bar.x).max(0.0), bar.h);
        let selected = index == current;
        if selected {
            frame.rect_corners(tab, p.selected, [RADIUS_ROW; 4], clip);
        }
        let color = if selected { p.text } else { p.text_muted };
        let baseline = ui_baseline(app, tab, size);
        frame.text(
            FontKind::Ui,
            size,
            tab.x + 10.0,
            baseline,
            &name,
            color,
            clip,
        );
        tabs.push(tab);
        x += w + 4.0;
    }
    app.table_layout.sheets = tabs;
}

/// 返回各列的宽度（逻辑像素），按前 [`MEASURED_ROWS`] 行中最宽的文字确定。
///
/// 宽度限制在 [`COLUMN_MIN`] 到 [`COLUMN_MAX`] 之间；横向合并的格不参与测量（标题常跨多列）。
/// 同一张表只测量一次，结果缓存在 `table_layout` 中。
fn column_widths(
    app: &mut App,
    frame: &mut Frame<'_>,
    key: (DocumentId, usize, usize),
    sheet: &Sheet,
) -> Vec<f32> {
    if let Some((cached, widths)) = &app.table_layout.widths
        && *cached == key
    {
        return widths.clone();
    }
    let size = app.fonts.ui;
    let mut widths = vec![COLUMN_MIN; sheet.columns];
    for (r, row) in sheet.rows.iter().take(MEASURED_ROWS).enumerate() {
        for (column, text) in row.iter().enumerate() {
            let first = text.lines().next().unwrap_or_default();
            let spans = sheet.merge_at(r, column).is_some_and(|m| m.right > m.left);
            if first.is_empty() || spans {
                continue;
            }
            let w = frame.measure(FontKind::Ui, size, first) + 2.0 * PAD;
            widths[column] = widths[column].max(w.min(COLUMN_MAX).ceil());
        }
    }
    app.table_layout.widths = Some((key, widths.clone()));
    widths
}

/// 文字看起来是不是数字（整数、小数、百分比、带千分位），这样的单元格靠右。
fn is_numeric(text: &str) -> bool {
    let text = text.trim();
    let digits = text.strip_suffix('%').unwrap_or(text).replace(',', "");
    !digits.is_empty() && digits.parse::<f64>().is_ok_and(f64::is_finite)
}
