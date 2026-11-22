//! 绘制在文件中查找的对话框（标题行、查询、各处匹配和选中处的上下文）和底部面板的「搜索」标签。

use winc_i18n::t;

use super::*;
use crate::app::editor::definition::Place;
use crate::app::workspace::search_dialog::{DialogPart, DialogParts, PREVIEW_PAD};
use crate::app::{Focus, Hover, PICKER_ICON_X, PICKER_TEXT_X};
use crate::view::editor::places::{PlacesView, draw_code_with, draw_rows};

/// 新的搜索还没有结果时，上一次的结果的不透明度。
const STALE_ALPHA: f32 = 0.45;

/// 绘制对话框，与其他对话框一样淡入；关闭后淡出期间绘制最后一帧。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    app.track_card(Card::Search, app.search_dialog.open);
    if !app.search_dialog.open {
        draw_card_ghost(app, frame, Card::Search);
        return;
    }
    let card = app.search_dialog_rect();
    draw_moving_card(app, frame, Card::Search, card, draw_dialog);
}

/// 绘制打开中的对话框。
///
/// 样式与其他对话框相同：标题行右端是范围、「在底部显示」和关闭按钮，输入框、分段控件和按钮
/// 沿用侧栏搜索、设置和表单对话框的样式，结果列表与跳转到定义的列表相同。
fn draw_dialog(app: &mut App, frame: &mut Frame<'_>) {
    let context = app.search_dialog_context();
    let parts = app.search_dialog_parts();
    let p = app.palette;
    let card = parts.card;
    draw_card(frame, p, card, RADIUS_CARD, p.editor_bg);
    let hovered = |app: &App, part| app.pointer.hover == Hover::SearchDialog(part);
    let line = |y: f32| Rect::new(card.x + 1.0, y, card.w - 2.0, 1.0);

    // 标题行：标题、结果统计，右端是范围、「在底部显示」和关闭按钮。
    let header = parts.header;
    let x = card.x + SETTINGS_PAD;
    let end = ui_text(frame, app, x, header, t!("cmd-find-in-files"), p.text, card);
    let (count, count_color) = summary(app);
    let room = (parts.track.x - 12.0 - end - 10.0).max(0.0);
    let count = ellipsize(frame, &count, app.fonts.ui_small, room);
    ui_text_small(frame, app, end + 10.0, header, &count, count_color, card);
    draw_scopes(app, frame, &parts);
    let dock = parts.dock;
    draw_button(frame, p, dock, card, hovered(app, DialogPart::Dock), false);
    icons::panel(frame, center(dock), false, p.text_muted, dock);
    let close = parts.close;
    draw_button(
        frame,
        p,
        close,
        card,
        hovered(app, DialogPart::Close),
        false,
    );
    icons::close(frame, center(close), 7.0, p.text_muted, close);

    // 查询框：右端是三个开关和「过滤」；过滤打开时下面是文件掩码。
    let focused = app.focus == Focus::SearchDialog;
    let mask_focused = app.focus == Focus::SearchMask;
    let d = &app.search_dialog;
    let settled = d.due.is_none();
    let error = d.engine.error().is_some() && settled;
    let mask_error = error && d.engine.is_filter_error();
    field_box(
        app,
        frame,
        parts.query_row,
        p.editor_bg,
        focused,
        error && !mask_error,
    );
    if parts.mask.h > 0.0 {
        field_box(
            app,
            frame,
            parts.mask,
            p.editor_bg,
            mask_focused,
            mask_error,
        );
    }
    let caret = {
        let mut query = std::mem::take(&mut app.search_dialog.query);
        let mut mask = std::mem::take(&mut app.search_dialog.mask);
        let placeholder = t!("workspace-search-placeholder");
        let a = draw_bar_field(app, frame, parts.query, &mut query, placeholder, focused);
        let b = (parts.mask.h > 0.0).then(|| {
            let row = parts.mask;
            let title = t!("workspace-search-include");
            let end = ui_text_small(frame, app, row.x + 10.0, row, title, p.text_muted, row);
            let area = Rect::new(
                end + 8.0,
                row.y,
                (row.right() - 10.0 - end - 8.0).max(0.0),
                row.h,
            );
            let example = t!("workspace-search-include-example");
            draw_bar_field(app, frame, area, &mut mask, example, mask_focused)
        });
        app.search_dialog.query = query;
        app.search_dialog.mask = mask;
        a.or(b.flatten())
    };
    if caret.is_some() {
        app.ime.cursor = caret;
    }
    let app = &*app;
    let d = &app.search_dialog;
    let on = [d.case_sensitive, d.whole_word, d.regex];
    let toggles = [0, 1, 2].map(|i| (parts.options[i], on[i], hovered(app, DialogPart::Option(i))));
    draw_search_options(app, frame, toggles, card);
    let filter = parts.mask_toggle;
    draw_button(
        frame,
        p,
        filter,
        card,
        hovered(app, DialogPart::MaskToggle),
        d.mask_on,
    );
    let active = d.mask_on && !d.mask.text().is_empty();
    let color = if active { p.accent } else { p.text_muted };
    icons::filter(frame, center(filter), color, card);

    // 各处匹配：与跳转到定义的列表相同；没有结果时在列表和预览的位置居中显示提示。
    let top = parts.list.bar.y;
    frame.rect_clipped(line(top), p.separator, 0.0, card);
    let Some(places) = d.places.as_ref().filter(|_| !d.hits.is_empty()) else {
        let body = Rect::new(card.x, top, card.w, parts.preview.bottom() - top);
        return draw_empty(app, frame, body, true);
    };
    let fade = if d.stale { STALE_ALPHA } else { 1.0 };
    frame.with_opacity(fade, |frame| {
        let hover = match app.pointer.hover {
            Hover::SearchDialog(DialogPart::Row(i)) => Some(i),
            _ => None,
        };
        let view = PlacesView::of(places, &d.hits, d.selected, hover, d.scroll);
        let list = parts.list.list;
        frame.within(list, |frame| draw_rows(app, frame, &parts.list, &view));

        // 预览：标题行与列表的行对齐，是文件图标、文件名和所在目录；下面是选中处周围的代码。
        let head = parts.preview_head;
        frame.rect_clipped(line(head.y), p.separator, 0.0, card);
        let Some(place) = places.items.get(d.selected) else {
            return;
        };
        let (dir, name) = match place.relative.rsplit_once('/') {
            Some((dir, name)) => (dir, name),
            None => ("", place.relative.as_str()),
        };
        let icon = (
            card.x + PICKER_ICON_X,
            icon_center_y(app, head, app.fonts.ui),
        );
        draw_file_icon(frame, p, icon, name, card);
        let x = card.x + PICKER_TEXT_X;
        let end = ui_text(frame, app, x, head, name, p.text, card);
        let room = (card.right() - SETTINGS_PAD - end - 8.0).max(0.0);
        let dir = ellipsize(frame, dir, app.fonts.ui_small, room);
        ui_text_small(frame, app, end + 8.0, head, &dir, p.text_dim, card);
        if let Some((at, first, lines)) = &context {
            frame.within(parts.preview, |frame| {
                draw_context(app, frame, parts.preview, (*at, *first), lines, place);
            });
        }
    });
}

/// 在标题行绘制范围的分段控件，样式与设置中的分段控件相同。
fn draw_scopes(app: &App, frame: &mut Frame<'_>, parts: &DialogParts) {
    let p = app.palette;
    let (card, d) = (parts.card, &app.search_dialog);
    let selected = usize::from(d.in_dir);
    let slot = Some(parts.scopes[selected]);
    controls::segment_track(frame, p, parts.track, p.editor_bg, slot, card);
    let labels = [
        t!("workspace-search-scope-project"),
        t!("workspace-search-scope-directory"),
    ];
    for (i, (rect, label)) in parts.scopes.iter().zip(labels).enumerate() {
        let enabled = i == 0 || d.dir.is_some();
        let hover = enabled && app.pointer.hover == Hover::SearchDialog(DialogPart::Scope(i));
        if hover && i != selected {
            frame.rect_clipped(*rect, p.hover, RADIUS_ROW, card);
        }
        let color = match (enabled, i == selected || hover) {
            (false, _) => p.text_dim,
            (true, true) => p.text,
            (true, false) => p.text_muted,
        };
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, label);
        let at = (rect.x + (rect.w - w) / 2.0).round();
        ui_text_small(frame, app, at, *rect, label, color, card);
    }
}

/// 返回对话框和底部面板「搜索」标签中悬停处的提示：按钮的名称和快捷键，「目录」是目录的相对
/// 路径。
pub(in crate::view) fn tooltip(app: &App) -> Option<(Rect, String)> {
    use crate::keymap::{KeyCode, Mods, Os, Stroke};
    use crate::layout::{SearchButton, TIP_KEY};
    let part = match app.pointer.hover {
        Hover::SearchDock(DialogPart::Option(i)) => {
            let button = [SearchButton::Case, SearchButton::Word, SearchButton::Regex][i];
            return Some((app.docked_parts()?.options[i], button.label()));
        }
        Hover::SearchDialog(part) => part,
        _ => return None,
    };
    let parts = app.search_dialog_parts();
    let d = &app.search_dialog;
    let stroke = |key, mods| Stroke { key, mods }.label(Os::CURRENT);
    match part {
        DialogPart::Option(i) => {
            let button = [SearchButton::Case, SearchButton::Word, SearchButton::Regex][i];
            Some((parts.options[i], button.label()))
        }
        DialogPart::MaskToggle => {
            let label = if d.mask_on {
                t!("workspace-search-hide-filters")
            } else {
                t!("workspace-search-filter-files")
            };
            Some((parts.mask_toggle, label.to_owned()))
        }
        DialogPart::Dock => {
            let mods = if cfg!(target_os = "macos") {
                Mods::META
            } else {
                Mods::CTRL
            };
            let name = t!("workspace-search-dock-button");
            let key = stroke(KeyCode::Enter, mods);
            Some((parts.dock, format!("{name}{TIP_KEY}{key}")))
        }
        DialogPart::Close => {
            let key = stroke(KeyCode::Escape, Mods::NONE);
            Some((parts.close, format!("{}{TIP_KEY}{key}", t!("common-close"))))
        }
        DialogPart::Scope(1) => {
            let dir = d.dir.as_ref()?;
            let root = app.editor.root();
            let relative = root.and_then(|root| dir.strip_prefix(root).ok());
            let shown = relative
                .filter(|r| !r.as_os_str().is_empty())
                .unwrap_or(dir);
            Some((parts.scopes[1], shown.display().to_string()))
        }
        _ => None,
    }
}

/// 绘制底部面板的「搜索」标签：顶部一行是查询和结果统计，左边是各处匹配，右边是选中处的上下文。
pub(in crate::view) fn draw_dock(app: &mut App, frame: &mut Frame<'_>) {
    let Some(dock) = app.docked_parts() else {
        return;
    };
    let context = app.search_dialog_context();
    let p = app.palette;
    let card = app.layout.terminal;

    // 顶部一行与快速打开的输入行相同：放大镜、查询，右端是结果统计和搜索选项，下面一条分隔线。
    let row = dock.summary;
    let glass = (
        row.x + PICKER_ICON_X + icons::FILE_WIDTH / 2.0,
        row.y + row.h / 2.0,
    );
    icons::magnifier(frame, glass, p.text_dim, card);
    let (summary, color) = summary(app);
    let summary_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &summary);
    let room = (dock.query.w - summary_w - 16.0).max(0.0);
    let area = Rect::new(dock.query.x, row.y, room, row.h);
    let focused = app.docked_query_focused();
    let mut query = std::mem::take(&mut app.search_dialog.query);
    let placeholder = t!("workspace-search-placeholder");
    let caret = draw_bar_field(app, frame, area, &mut query, placeholder, focused);
    app.search_dialog.query = query;
    if focused && caret.is_some() {
        app.ime.cursor = caret;
    }
    let app = &*app;
    let d = &app.search_dialog;
    let x = dock.query.right() - summary_w;
    ui_text_small(frame, app, x, row, &summary, color, card);
    let on = [d.case_sensitive, d.whole_word, d.regex];
    let hovered = |i| app.pointer.hover == Hover::SearchDock(DialogPart::Option(i));
    let toggles = [0, 1, 2].map(|i| (dock.options[i], on[i], hovered(i)));
    draw_search_options(app, frame, toggles, card);
    let line = Rect::new(card.x, row.bottom(), card.w, 1.0);
    frame.rect_clipped(line, p.separator, 0.0, card);

    // 没有结果时在列表和预览的位置居中显示提示。
    let top = dock.summary.bottom();
    let Some(places) = d.places.as_ref().filter(|_| !d.hits.is_empty()) else {
        let body = Rect::new(card.x, top, card.w, card.bottom() - top);
        return draw_empty(app, frame, body, false);
    };
    let fade = if d.stale { STALE_ALPHA } else { 1.0 };
    frame.with_opacity(fade, |frame| {
        let hover = match app.pointer.hover {
            Hover::SearchDock(DialogPart::Row(i)) => Some(i),
            _ => None,
        };
        let view = PlacesView::of(places, &d.hits, d.selected, hover, d.scroll);
        let list = dock.list.list;
        frame.within(list, |frame| draw_rows(app, frame, &dock.list, &view));
        let divider = Rect::new(dock.preview.x, dock.preview.y, 1.0, dock.preview.h);
        frame.rect_clipped(divider, p.separator, 0.0, card);
        let (Some(place), Some((at, first, lines))) = (places.items.get(d.selected), &context)
        else {
            return;
        };
        // 第一行与列表的第一行在同一高度上居中。
        let lead = crate::app::PICKER_PAD + (crate::app::PICKER_ROW - app.fonts.line_height) / 2.0;
        let area = Rect::new(
            dock.preview.x + 1.0,
            dock.preview.y + lead - PREVIEW_PAD,
            dock.preview.w - 1.0,
            (dock.preview.h - lead).max(0.0),
        );
        frame.within(dock.preview, |frame| {
            draw_context(app, frame, area, (*at, *first), lines, place);
        });
    });
}

/// 在 `area` 中居中绘制没有结果时的提示：没有查询时是一句说明和几个按键，查过没有匹配时是
/// 「没有匹配」，搜索途中和查询有误时不绘制。`dialog` 为真时按键中有「在底部显示」。
fn draw_empty(app: &App, frame: &mut Frame<'_>, area: Rect, dialog: bool) {
    use crate::keymap::{KeyCode, Mods, Os, Stroke};
    let p = app.palette;
    let d = &app.search_dialog;
    let searching = d.engine.is_running() || d.due.is_some();
    // 查询有误时标题行已经显示错误，这里不再说「没有匹配」。
    let failed = d.engine.error().is_some();
    let message = if d.query.text().is_empty() {
        t!("workspace-search-empty")
    } else if !searching && !failed {
        t!("editor-places-no-match")
    } else {
        return;
    };
    let hints = d.query.text().is_empty();
    let line_h = 24.0;
    let block_h = if hints { line_h * 2.0 + 8.0 } else { line_h };
    let top = (area.y + (area.h - block_h) / 2.0).round();
    let row = Rect::new(area.x, top, area.w, line_h);
    let w = frame.measure(FontKind::Ui, app.fonts.ui, message);
    ui_text(
        frame,
        app,
        (area.x + (area.w - w) / 2.0).round(),
        row,
        message,
        p.text_dim,
        area,
    );
    if !hints {
        return;
    }

    // 第二行：按键和它的作用，键帽与开始页上的相同。
    let primary = if cfg!(target_os = "macos") {
        Mods::META
    } else {
        Mods::CTRL
    };
    let key = |key, mods| Stroke { key, mods }.label(Os::CURRENT);
    let up_down = format!(
        "{}{}",
        key(KeyCode::Up, Mods::NONE),
        key(KeyCode::Down, Mods::NONE)
    );
    let mut items = vec![
        (up_down, t!("workspace-search-hint-select")),
        (
            key(KeyCode::Enter, Mods::NONE),
            t!("workspace-search-hint-open"),
        ),
    ];
    if dialog {
        items.push((
            key(KeyCode::Enter, primary),
            t!("workspace-search-dock-button"),
        ));
    }
    let size = app.fonts.ui_small;
    let widths: Vec<(f32, f32)> = (items.iter())
        .map(|(k, label)| {
            (
                frame.measure(FontKind::Ui, size, k) + 12.0,
                frame.measure(FontKind::Ui, size, label),
            )
        })
        .collect();
    let total =
        widths.iter().map(|(k, l)| k + 6.0 + l).sum::<f32>() + 20.0 * (items.len() - 1) as f32;
    let row = Rect::new(area.x, row.bottom() + 8.0, area.w, line_h);
    let mut x = (area.x + (area.w - total) / 2.0).round();
    for ((k, label), (key_w, label_w)) in items.iter().zip(widths) {
        let chip = Rect::new(x, (row.y + (row.h - 20.0) / 2.0).round(), key_w, 20.0);
        frame.rect_clipped(chip, p.selected_inactive, RADIUS_ROW, area);
        caps_label(
            frame,
            app,
            size,
            (chip.x + 6.0, chip),
            k,
            p.text_muted,
            area,
        );
        x = chip.right() + 6.0;
        ui_text_small(frame, app, x, row, label, p.text_dim, area);
        x += label_w + 20.0;
    }
}

/// 返回底栏的结果统计和颜色：「3 个文件中有 12 个结果」，或搜索中、出错的状态。
fn summary(app: &App) -> (String, Color) {
    let p = app.palette;
    let d = &app.search_dialog;
    let engine = &d.engine;
    let pending = d.due.is_some();
    if let Some(error) = engine.error().filter(|_| !pending) {
        return (error.to_owned(), p.error);
    }
    if d.query.text().is_empty() {
        return (String::new(), p.text_dim);
    }
    let searching = engine.is_running() || pending;
    if searching && engine.total() == 0 {
        return (t!("workspace-search-searching").to_owned(), p.text_dim);
    }
    let more = if engine.is_truncated() || engine.gave_up() {
        "+"
    } else {
        ""
    };
    let (files, total) = (engine.results.len(), engine.total());
    let text = if searching {
        t!(
            "workspace-search-summary-searching",
            files = files,
            total = total,
            more = more
        )
    } else {
        t!(
            "workspace-search-summary",
            files = files,
            total = total,
            more = more
        )
    };
    (text, p.text_muted)
}

/// 在预览区 `area` 中绘制选中处周围的几行：左侧是行号，选中的一行铺底色、匹配铺琥珀色底。
///
/// `at` 是（选中的行号，首行的行号），均从 0 起。
fn draw_context(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    (at, first): (usize, usize),
    lines: &[String],
    place: &Place,
) {
    let p = app.palette;
    let line_h = app.fonts.line_height;
    let top = area.y + PREVIEW_PAD;
    // 取来的行可能多于这里放得下的行数：从中取一段，让选中的一行居中。
    let fit = (((area.h - 2.0 * PREVIEW_PAD) / line_h).floor().max(1.0)) as usize;
    let skip = (at.saturating_sub(first))
        .saturating_sub(fit / 2)
        .min(lines.len().saturating_sub(fit));
    let (first, lines) = (first + skip, &lines[skip..]);
    let (size, cell) = (app.fonts.code, app.metrics.cell);
    let digits = (first + lines.len()).max(1).to_string().len();
    // 行号等宽、靠右；代码与列表中的文件名对齐，行号太宽时右移。
    let code_x = (area.x + PICKER_TEXT_X).max(area.x + 24.0 + digits as f32 * cell);
    let gutter = code_x - 12.0;
    let (ascent, descent) = (app.metrics.mono.ascent, app.metrics.mono.descent);
    for (k, text) in lines.iter().enumerate() {
        let n = first + k;
        let row = Rect::new(area.x, top + k as f32 * line_h, area.w, line_h);
        let current = n == at;
        if current {
            frame.rect_clipped(row.inset(6.0, 0.0), p.selected_inactive, RADIUS_ROW, area);
        }
        let number = (n + 1).to_string();
        let x = gutter - number.len() as f32 * cell;
        let baseline = (row.y + (row.h - (ascent + descent)) / 2.0 + ascent).round();
        let color = if current { p.text_muted } else { p.text_dim };
        frame.text(FontKind::Mono, size, x, baseline, &number, color, area);
        // 列表中的文字去掉了行首空白，这里按缩进移动匹配的范围。
        let marks = if current {
            let indent = text.chars().take_while(|c| c.is_whitespace()).count();
            let shift = |r: &std::ops::Range<usize>| r.start + indent..r.end + indent;
            place.marks.iter().map(shift).collect()
        } else {
            Vec::new()
        };
        let line = Place {
            path: place.path.clone(),
            relative: place.relative.clone(),
            line: n,
            col: None,
            preview: text.replace('\t', "    "),
            marks,
        };
        let code = Rect::new(code_x, row.y, row.right() - 12.0 - code_x, row.h);
        frame.within(area, |frame| draw_code_with(app, frame, &line, code, 1.0));
    }
}
