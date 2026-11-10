//! 位置浮窗：在名字上转到定义（多处定义）、在定义上转到定义（用法）、列出书签时，挂在光标所在
//! 名字旁边的卡片。
//!
//! 卡片分标题行（名字、发起查找的文件、数量）、列表（文件图标、文件名、行号和该行代码，名字铺
//! 查找结果的琥珀色底）和底栏（选中项的完整路径和行号）。
//!
//! 出现时从名字那一侧滑出并淡入，关掉时原地淡出（`Picker::places_in`、`Picker::places_out`）；选择、
//! 筛选、滚动和键盘操作与快速打开面板共用（`picker_*`），这里只负责绘制。

use std::fmt::Write as _;

use winc_core::text;
use winc_i18n::t;
use winc_syntax::{LineState, TokenKind, detect, highlight_line};

use super::*;
use crate::app::editor::definition::{Place, Places, PlacesKind};
use crate::app::{Focus, PICKER_ICON_X, PICKER_ROW, PICKER_TEXT_X, PlacesGhost};
use crate::layout::PickerParts;

/// 文件名与行号之后、代码之前的间距（逻辑像素）。
const CODE_GAP: f32 = 16.0;
/// 文件名和行号一栏最多占整行宽度的比例；更长的文件名截短。
const LABEL_SHARE: f32 = 0.38;
/// 代码的不透明度：代码只是上下文，比编辑区里淡，名字和选中行更突出。
const CODE_ALPHA: f32 = 0.72;
/// 出现时浮窗滑动的距离（逻辑像素）：从名字那一侧滑出，只滑过很短的距离，表明浮窗来自哪里。
const SLIDE: f32 = 4.0;

/// 绘制浮窗所需的内容：打开时取自快速打开面板的状态，淡出时取自关掉那一刻保存下来的状态。
pub(super) struct PlacesView<'a> {
    places: &'a Places,
    /// 筛选后留下的条目在 `places.items` 中的序号。
    hits: &'a [usize],
    /// 选中的是 `hits` 中的第几条。
    selected: usize,
    /// 鼠标悬停的是 `hits` 中的第几条。
    hovered: Option<usize>,
    /// 列表的滚动量（逻辑像素）。
    scroll: f32,
    filter: &'a str,
    /// 是否绘制筛选文字的光标（浮窗有焦点、光标处于亮的一拍）。
    caret: bool,
}

impl<'a> PlacesView<'a> {
    /// 打开中的浮窗。
    fn live(app: &'a App) -> Option<PlacesView<'a>> {
        Some(PlacesView {
            places: app.picker.places.as_ref()?,
            hits: &app.picker.hits,
            selected: app.picker.selected,
            hovered: match app.hover {
                Hover::PickerRow(i) => Some(i),
                _ => None,
            },
            scroll: app.picker.scroll,
            filter: app.picker.input.text(),
            caret: app.focus == Focus::Picker && app.blink.visible,
        })
    }

    /// 正在淡出的浮窗。
    fn ghost(ghost: &'a PlacesGhost) -> PlacesView<'a> {
        PlacesView {
            places: &ghost.places,
            hits: &ghost.hits,
            selected: ghost.selected,
            hovered: None,
            scroll: ghost.scroll,
            filter: &ghost.filter,
            caret: false,
        }
    }
}

/// 绘制打开中的浮窗，返回筛选文字的光标矩形（供输入法候选框定位）。
///
/// 刚打开的一百多毫秒里从名字那一侧滑出并淡入：不透明度按二次缓出（很快就看得清），位置按
/// 三次缓出（最后一段慢慢停稳）。
pub(in crate::view) fn draw_live(app: &App, frame: &mut Frame<'_>, rect: Rect) -> Option<Rect> {
    let view = PlacesView::live(app)?;
    let t = app.picker.places_in.map_or(1.0, |clock| clock.linear());
    let opacity = 1.0 - (1.0 - t).powi(2);
    let rect = slid(rect, app.places_above(), (1.0 - t).powi(3));
    frame.with_opacity(opacity, |frame| draw_places(app, frame, rect, &view))
}

/// 绘制关掉之后正在淡出的浮窗：原地淡出，不透明度按二次缓入（先慢后快，开头不会立即变淡）。
pub(in crate::view) fn draw_ghost(app: &App, frame: &mut Frame<'_>, ghost: &PlacesGhost) {
    let t = ghost.clock.linear();
    let view = PlacesView::ghost(ghost);
    frame.with_opacity(1.0 - t * t, |frame| {
        draw_places(app, frame, ghost.rect, &view)
    });
}

/// 返回向名字那一侧移动了 `away`（0–1，乘以 [`SLIDE`]）的浮窗：浮窗在名字下方时向上移，
/// 在上方（`above`）时向下移。
fn slid(rect: Rect, above: bool, away: f32) -> Rect {
    let toward_name = if above { 1.0 } else { -1.0 };
    Rect::new(rect.x, rect.y + toward_name * SLIDE * away, rect.w, rect.h)
}

/// 绘制位置浮窗的内容（标题行、分隔线、结果列表和底栏），返回筛选文字的光标矩形。
fn draw_places(app: &App, frame: &mut Frame<'_>, rect: Rect, view: &PlacesView) -> Option<Rect> {
    let p = app.palette;
    let parts = PickerParts::places(rect);
    draw_card(frame, p, rect, RADIUS_BAR, p.popup_bg);

    let caret = draw_header(app, frame, &parts, view);
    if rect.h <= BAR_HEIGHT {
        return caret;
    }
    let line = |y: f32| Rect::new(rect.x + 1.0, y, rect.w - 2.0, 1.0);
    frame.rect_clipped(line(parts.bar.bottom()), p.separator, 0.0, rect);

    let list = parts.list;
    if view.hits.is_empty() {
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        let x = list.x + PICKER_TEXT_X;
        let message = t!("editor-places-no-match");
        ui_text(frame, app, x, row, message, p.text_dim, list);
    } else {
        draw_rows(app, frame, &parts, view);
    }

    if parts.footer.h > 0.0 {
        frame.rect_clipped(line(parts.footer.y), p.separator, 0.0, rect);
        let selected = view
            .hits
            .get(view.selected)
            .and_then(|&i| view.places.items.get(i));
        if let Some(place) = selected {
            let path = format!("{}:{}", place.relative, place.line + 1);
            let room = parts.footer.w - 2.0 * PICKER_ICON_X;
            let path = ellipsize(frame, &path, app.fonts.ui_small, room);
            let x = parts.footer.x + PICKER_ICON_X;
            ui_text_small(frame, app, x, parts.footer, &path, p.text_dim, rect);
        }
    }

    caret
}

/// 绘制标题行：左侧是名字和查找的种类、发起查找的文件，右侧是数量；有筛选文字时数量左边是
/// 筛选胶囊。
///
/// 返回筛选文字的光标矩形。
fn draw_header(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
    view: &PlacesView,
) -> Option<Rect> {
    let (p, bar, places) = (app.palette, parts.bar, view.places);

    // 数量按用法计（同一行的几处合并成一项，但都算上）；筛选时显示「剩下的 / 全部」。
    let weight = |place: &Place| place.marks.len().max(1);
    let total: usize = places.items.iter().map(weight).sum();
    let count = if view.filter.is_empty() {
        places.kind.count(total)
    } else {
        let shown: usize = view
            .hits
            .iter()
            .filter_map(|&i| places.items.get(i))
            .map(weight)
            .sum();
        format!("{shown} / {}", places.kind.count(total))
    };
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count);
    let count_x = bar.right() - PICKER_ICON_X - count_w;
    ui_text_small(frame, app, count_x, bar, &count, p.text_dim, bar);

    // 筛选：有输入时是一枚胶囊（放大镜 + 文字 + 光标）。没有输入时什么也不绘制，直接打字即可。
    let mut caret = None;
    let right = if view.filter.is_empty() {
        count_x
    } else {
        let max = (bar.w * 0.3).max(60.0);
        let text = ellipsize(frame, view.filter, app.fonts.ui, max);
        let text_w = frame.measure(FontKind::Ui, app.fonts.ui, &text);
        let pill_w = 26.0 + text_w + 10.0;
        let pill = Rect::new(count_x - 12.0 - pill_w, bar.y + 7.0, pill_w, bar.h - 14.0);
        frame.rect_clipped(pill, p.selected_inactive, pill.h / 2.0, bar);
        let glass = (pill.x + 13.0, pill.y + pill.h / 2.0);
        icons::magnifier(frame, glass, p.text_dim, bar);
        let end = ui_text(frame, app, pill.x + 22.0, pill, &text, p.text, bar);
        let cursor = Rect::new(end + 1.0, pill.y + 4.0, 1.5, pill.h - 8.0);
        if view.caret {
            frame.rect_clipped(cursor, p.caret, 0.0, bar);
        }
        if app.focus == Focus::Picker {
            caret = Some(cursor);
        }
        pill.x
    };

    // 标题：名字（正文色）加种类（淡色），后面是发起查找的文件（更淡的小字）。
    let clip = Rect::new(bar.x, bar.y, (right - 16.0 - bar.x).max(0.0), bar.h);
    let mut x = bar.x + PICKER_ICON_X;
    let (title, kind) = match places.kind {
        PlacesKind::Usages => (places.name.as_str(), t!("editor-places-usages-suffix")),
        PlacesKind::Definitions => (places.name.as_str(), t!("editor-places-definitions-suffix")),
        PlacesKind::Bookmarks => (t!("editor-places-bookmarks"), ""),
    };
    let title = ellipsize(frame, title, app.fonts.ui, (clip.right() - x) * 0.5);
    x = ui_text(frame, app, x, bar, &title, p.text, clip);
    x = ui_text(frame, app, x, bar, kind, p.text_muted, clip);
    let left = (clip.right() - x - 10.0).max(0.0);
    if left > 24.0 {
        let origin = ellipsize(frame, &places.origin, app.fonts.ui_small, left);
        ui_text_small(frame, app, x + 10.0, bar, &origin, p.text_dim, clip);
    }

    caret
}

/// 绘制列表中可见的几行：文件图标、文件名与行号、那一行的代码。
fn draw_rows(app: &App, frame: &mut Frame<'_>, parts: &PickerParts, view: &PlacesView) {
    let (p, list, scroll) = (app.palette, parts.list, view.scroll);
    let rows: Vec<(usize, &Place)> = parts
        .visible(scroll, view.hits.len())
        .filter_map(|i| Some((i, view.places.items.get(*view.hits.get(i)?)?)))
        .collect();

    // 书签都在同一个文件里，只显示行号；其余显示文件名和行号。各行的代码从同一列开始，
    // 这一列按全部条目中最宽的「文件名 + 行号」定（如果只看可见行，滚动时这一列会左右跳），
    // 最多占整行的 `LABEL_SHARE`。条目和字号不变时只量一次（`Places::widest_label`）。
    let bookmarks = view.places.kind == PlacesKind::Bookmarks;
    // 行号写进同一个缓冲区，不必每条分配一个字符串。
    let mut number = String::new();
    let line_of = |number: &mut String, place: &Place| {
        number.clear();
        let _ = write!(number, "{}", place.line + 1);
    };
    let sizes = (app.fonts.ui, app.fonts.ui_small);
    let widest = view.places.widest_label(sizes, |items| {
        items.iter().fold(0.0, |widest: f32, place| {
            let name = if bookmarks { "" } else { file_name(place) };
            let name_w = frame.measure(FontKind::Ui, app.fonts.ui, name);
            let gap = if name.is_empty() { 0.0 } else { 6.0 };
            line_of(&mut number, place);
            let line_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &number);
            widest.max(name_w + gap + line_w)
        })
    });
    let code_x = list.x + PICKER_TEXT_X + widest.min(list.w * LABEL_SHARE) + CODE_GAP;

    for (i, place) in rows {
        let row = parts.row(i, scroll);
        let clip = row.intersect(list);
        let selected = i == view.selected;
        if selected {
            frame.rect_clipped(row.inset(6.0, 1.0), p.selected, RADIUS_ROW, clip);
        } else if view.hovered == Some(i) {
            frame.rect_clipped(row.inset(6.0, 1.0), p.hover, RADIUS_ROW, clip);
        }

        let cy = icon_center_y(app, row, app.fonts.ui);
        let icon = (row.x + PICKER_ICON_X, cy);
        draw_file_icon(frame, p, icon, file_name(place), clip);

        // 文件名平时用次要色，选中的那一行用正文色。
        let name = if bookmarks { "" } else { file_name(place) };
        line_of(&mut number, place);
        let line = number.as_str();
        let label_clip =
            Rect::new(row.x, row.y, code_x - CODE_GAP / 2.0 - row.x, row.h).intersect(clip);
        let room = label_clip.right() - (row.x + PICKER_TEXT_X);
        let line_w = frame.measure(FontKind::Ui, app.fonts.ui_small, line);
        let name = ellipsize(frame, name, app.fonts.ui, (room - line_w - 6.0).max(0.0));
        let color = if selected { p.text } else { p.text_muted };
        let mut x = ui_text(
            frame,
            app,
            row.x + PICKER_TEXT_X,
            row,
            &name,
            color,
            label_clip,
        );
        if !name.is_empty() {
            x += 6.0;
        }
        ui_text_small(frame, app, x, row, line, p.text_dim, label_clip);

        let code = Rect::new(code_x, row.y, row.right() - 12.0 - code_x, row.h).intersect(clip);
        draw_code(app, frame, place, code);
    }
}

/// 在 `area` 中用编辑区的等宽字体绘制 `place` 那一行的代码：语法颜色调淡（[`CODE_ALPHA`]），
/// 名字用正文色，铺上与查找结果相同的琥珀色圆角底。
///
/// 一行放不下时从左边截掉一些（前面加「…」），保证名字露在可见范围内。
fn draw_code(app: &App, frame: &mut Frame<'_>, place: &Place, area: Rect) {
    if area.w <= 0.0 {
        return;
    }
    let p = app.palette;
    let chars: Vec<char> = place.preview.chars().collect();
    let mut tokens = Vec::new();
    highlight_line(
        detect(file_name(place)),
        &chars,
        LineState::Normal,
        &mut tokens,
    );

    let cell = app.cell;
    let fit = (area.w / cell).floor() as usize;
    let skip = match place.marks.first() {
        Some(mark) if mark.end + 2 > fit => (mark.end + 4).saturating_sub(fit).min(mark.start),
        _ => 0,
    };
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let baseline = (area.y + (area.h - (ascent + descent)) / 2.0 + ascent).round();
    let size = app.fonts.code;

    // 先排好每个字符的位置（格），再绘制名字的底色，最后绘制字。
    let lead = usize::from(skip > 0);
    let mut cells = lead;
    let placed: Vec<(usize, char, usize, usize)> = chars
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(col, &c)| {
            let width = text::char_cells(c, cells, 4).max(1);
            let at = cells;
            cells += width;
            (col, c, at, width)
        })
        .take_while(|&(_, _, at, _)| area.x + at as f32 * cell <= area.right())
        .collect();
    let x_of = |cells: usize| area.x + cells as f32 * cell;

    for mark in &place.marks {
        let span: Vec<_> = placed
            .iter()
            .filter(|(col, ..)| mark.contains(col))
            .collect();
        if let (Some(first), Some(last)) = (span.first(), span.last()) {
            let x0 = x_of(first.2) - 2.0;
            let x1 = x_of(last.2 + last.3) + 2.0;
            let back = Rect::new(x0, area.y + 3.0, x1 - x0, area.h - 6.0);
            frame.rect_clipped(back, p.search_match, 3.0, area);
        }
    }

    if skip > 0 {
        let x = area.x;
        frame.cell_char(
            FontKind::Mono,
            size,
            x,
            baseline,
            cell,
            1,
            '…',
            p.text_dim,
            area,
        );
    }
    let mut token = 0;
    for (col, c, at, width) in placed {
        if c.is_whitespace() {
            continue;
        }
        while tokens.get(token).is_some_and(|t| t.end <= col) {
            token += 1;
        }
        let kind = tokens
            .get(token)
            .filter(|t| t.start <= col)
            .map_or(TokenKind::Text, |t| t.kind);
        let named = place.marks.iter().any(|m| m.contains(&col));
        let color = if named {
            p.text
        } else {
            let c = p.token(kind);
            c.alpha(c.a * CODE_ALPHA)
        };
        let font = if kind == TokenKind::Comment {
            FontKind::MonoItalic
        } else {
            FontKind::Mono
        };
        frame.cell_char(font, size, x_of(at), baseline, cell, width, c, color, area);
    }
}

fn file_name(place: &Place) -> &str {
    place
        .relative
        .rsplit_once('/')
        .map_or(place.relative.as_str(), |(_, name)| name)
}
