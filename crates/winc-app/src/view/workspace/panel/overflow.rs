//! 搜索结果中放不下的文件行：鼠标停着时写全名，伸出侧栏的部分绘制在编辑区之上，与文件树和改动
//! 列表相同。名字的展开、收回用 [`crate::app::TreeLabels`] 过渡，悬停按钮跟在全名之后。

use std::borrow::Cow;

use winc_render::Color;

use super::*;
use crate::app::{LabelCause, LabelView, ResultOverflow};
use crate::view::workspace::sidebar::retracting_color;

/// 文件行的文字：文件名，其后淡色的所在目录（非 UTF-8 的文件改为说明）及其颜色。
pub(super) struct FileText<'a> {
    pub(super) name: &'a str,
    pub(super) note: &'a str,
    pub(super) color: Color,
}

/// 返回第 `f` 个文件的结果行显示的文字。
pub(super) fn file_text(app: &App, f: usize) -> Option<FileText<'_>> {
    let file = app.panels.global.engine.results.get(f)?;
    let (dir, name) = file
        .relative
        .rsplit_once('/')
        .unwrap_or(("", file.relative.as_str()));
    let (note, color) = if file.lossy {
        (t!("workspace-search-not-utf8"), app.palette.error)
    } else {
        (dir, app.palette.text_dim)
    };
    Some(FileText { name, note, color })
}

/// 返回文件名连同其后的目录完整绘制时的宽度（逻辑像素）。
pub(super) fn text_width(app: &App, frame: &mut Frame<'_>, text: &FileText<'_>) -> f32 {
    let name = frame.measure(FontKind::Ui, app.fonts.ui, text.name);
    if text.note.is_empty() {
        return name;
    }
    name + 8.0 + frame.measure(FontKind::Ui, app.fonts.ui_small, text.note)
}

/// 从 `x` 起完整绘制文件名和其后的目录，裁剪在 `clip` 内。
pub(super) fn draw_text(
    app: &App,
    frame: &mut Frame<'_>,
    (x, row): (f32, Rect),
    text: &FileText<'_>,
    clip: Rect,
) {
    let end = ui_text(frame, app, x, row, text.name, app.palette.text, clip);
    if !text.note.is_empty() {
        ui_text_small(frame, app, end + 8.0, row, text.note, text.color, clip);
    }
}

/// 记录第 `f` 个文件这一行本帧的名字宽度 `width`，返回绘制参数；`hovered` 是否在写全名。
pub(super) fn sync_name(
    app: &App,
    f: usize,
    text: &FileText<'_>,
    hovered: bool,
    width: f32,
) -> Option<LabelView> {
    let file = app.panels.global.engine.results.get(f)?;
    let cause = LabelCause {
        hovered,
        levels: 0,
        current: 0,
    };
    let parts = [Cow::Borrowed(text.name), Cow::Borrowed(text.note)];
    Some(
        app.panels
            .global
            .names
            .sync(&file.path, cause, &parts, width),
    )
}

/// 返回写全名的这一行按钮还差多少到终点（逻辑像素）：按钮随名字展开滑出。
pub(super) fn actions_lag(app: &App, frame: &mut Frame<'_>, f: usize) -> f32 {
    let (Some(text), Some(file)) = (file_text(app, f), app.panels.global.engine.results.get(f))
    else {
        return 0.0;
    };
    let full = text_width(app, frame, &text);
    let shown = app
        .panels
        .global
        .names
        .peek(&file.path)
        .map_or(full, |v| v.width);
    (full - shown).max(0.0)
}

/// 把按钮 `actions` 往回移 `lag`。
pub(super) fn moved_back(actions: (Option<Rect>, Rect), lag: f32) -> (Option<Rect>, Rect) {
    let back = |r: Rect| Rect::new(r.x - lag, r.y, r.w, r.h);
    (actions.0.map(back), back(actions.1))
}

/// 计算鼠标停着的文件行写全名所需的溢出区域（见 [`ResultOverflow`]）。
///
/// 按钮移到全名之后，超出窗口时向左移回，名字在按钮之前截断。名字放得下、侧栏在开合、正在
/// 拖动或文件正在折叠时返回 `None`。
pub(in crate::view) fn result_overflow(app: &App, frame: &mut Frame<'_>) -> Option<ResultOverflow> {
    if !app.panel_shown() || !app.idle_pointer() || app.panels.global.motion.is_some() {
        return None;
    }
    let index = match app.pointer.hover {
        Hover::Result(i) | Hover::RowAction(i, _) => i,
        // 鼠标经过分隔条移向全名时保持展开；在分隔条上按下仍由分隔条开始拖动。
        Hover::Splitter if app.over_result_overflow_row(app.mouse().1) => {
            app.panels.global.overflow?.index
        }
        _ => return None,
    };
    let Some(&ResultRow::File(f)) = app.panels.global.rows().get(index) else {
        return None;
    };
    let area = app.panel_parts().results;
    let row = app.result_row_rect(index);
    if row.bottom() <= area.y || row.y >= area.bottom() {
        return None;
    }
    let end = row.x + PANEL_TEXT + text_width(app, frame, &file_text(app, f)?);
    let (replace, dismiss) = row_actions(row, app.panels.global.replace_open);
    let first = replace.unwrap_or(dismiss);
    if end <= first.x - 4.0 - COVER_FADE {
        return None;
    }
    let window = app.layout.window.right() - 8.0;
    let shift = (end + 8.0 - first.x).min(window - dismiss.right()).max(0.0);
    let actions = moved_back((replace, dismiss), -shift);
    let right = actions.1.right() + 4.0;
    let spill = (right > area.right())
        .then(|| Rect::new(area.right(), row.y + 1.0, right - area.right(), row.h - 2.0));
    Some(ResultOverflow {
        index,
        spill,
        actions,
    })
}

/// 在编辑区上面继续绘制写全名的文件行：底色、文字和按钮与结果列表里的那一行连成一条。
///
/// 命中范围按全名计算，绘制时按名字展开到的宽度露出：过渡途中鼠标移过去不会落空。
pub(in crate::view) fn draw_result_overflow(app: &App, frame: &mut Frame<'_>) {
    let Some(ResultOverflow {
        index,
        spill: Some(spill),
        actions,
    }) = app.panels.global.overflow
    else {
        return;
    };
    let Some(&ResultRow::File(f)) = app.panels.global.rows().get(index) else {
        return;
    };
    let Some(text) = file_text(app, f) else {
        return;
    };
    let p = app.palette;
    let row = app.result_row_rect(index);
    let area = app.panel_parts().results;
    let (replace, dismiss) = moved_back(actions, actions_lag(app, frame, f));
    let right = (dismiss.right() + 4.0).min(spill.right());
    if right <= spill.x {
        return;
    }
    // 这一行绘制在最上层：底色从滚动条的可点击范围开始，滚动条只被这一行的高度挡住。
    let left = area.right() - SCROLLBAR_HIT;
    let band = Rect::new(left, spill.y, right - left, spill.h);
    let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
    let rows = app.panels.global.rows();
    let selected = app.panels.global.selected_row().map(|i| rows[i]) == Some(ResultRow::File(f));
    let focused = app.focus == Focus::Results;
    // 先垫一层侧栏底色，再叠半透明的悬停色，与列表里那一行的底色相同。
    frame.rect_corners(band, p.sidebar_bg, radii, band);
    if let Some(color) = row_color(p, selected, focused, true) {
        frame.rect_corners(band, color, radii, band);
    }
    let clip = Rect::new(band.x, row.y, band.w, row.h);
    let first = replace.unwrap_or(dismiss);
    let text_clip = Rect::new(clip.x, clip.y, (first.x - 4.0 - clip.x).max(0.0), clip.h);
    draw_text(app, frame, (row.x + PANEL_TEXT, row), &text, text_clip);
    let hover_on = |action| app.pointer.hover == Hover::RowAction(index, action);
    if let Some(rect) = replace {
        draw_button(frame, p, rect, clip, hover_on(RowAction::Replace), false);
        icons::replace_one(frame, center(rect), p.text_muted, clip);
    }
    draw_button(frame, p, dismiss, clip, hover_on(RowAction::Dismiss), false);
    icons::close(frame, center(dismiss), 6.5, p.text_muted, clip);
}

/// 绘制鼠标移开后正在收回、还伸在侧栏外面的文件行名字：不响应点击，收回到侧栏边缘时消失。
///
/// 底色和列表里的那一段一起淡出（[`retracting_color`]），与文件树相同。
pub(in crate::view) fn draw_result_overflow_closing(app: &App, frame: &mut Frame<'_>) {
    let global = &app.panels.global;
    if !app.panel_shown() || global.motion.is_some() || !global.names.moving() {
        return;
    }
    let p = app.palette;
    let rows = global.rows();
    let area = app.panel_parts().results;
    let open = global.overflow.map(|o| o.index);
    let window = app.layout.window.right() - 8.0;
    let focused = app.focus == Focus::Results;
    let selected = global.selected_row().map(|i| rows[i]);
    for (path, view) in global.names.moving_rows() {
        let found = rows.iter().enumerate().find_map(|(i, &r)| match r {
            ResultRow::File(f) if global.engine.results.get(f).is_some_and(|x| x.path == path) => {
                Some((i, f))
            }
            _ => None,
        });
        let Some((index, f)) = found.filter(|&(i, _)| Some(i) != open) else {
            continue;
        };
        let Some(text) = file_text(app, f) else {
            continue;
        };
        let row = app.result_row_rect(index);
        let x = row.x + PANEL_TEXT;
        let right = (x + view.width + 8.0).min(window);
        if right <= area.right() || row.bottom() <= area.y || row.y >= area.bottom() {
            continue;
        }
        let left = area.right() - SCROLLBAR_HIT;
        let rect = Rect::new(left, row.y + 1.0, right - left, row.h - 2.0);
        let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
        frame.rect_corners(rect, p.sidebar_bg, radii, rect);
        let chosen = selected == Some(ResultRow::File(f));
        if let Some(color) = retracting_color(p, chosen, focused, view.progress) {
            frame.rect_corners(rect, color, radii, rect);
        }
        let clip = Rect::new(rect.x, row.y, rect.w, row.h);
        draw_text(app, frame, (x, row), &text, clip);
    }
}
