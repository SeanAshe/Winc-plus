//! 设置中的「快捷键」页。
//!
//! 页面顶部是搜索框（右端是「录制按键」按钮），其下按分组列出每个命令及其快捷键。
//! 快捷键都来自键位表，与按键处理使用同一份数据。

use winc_i18n::t;
use winc_render::{FontKind, Frame, Rect};

use super::*;
use crate::app::Focus;
use crate::keymap::Action;
use crate::layout::{KeysLayout, SettingsPart, settings_thumb};

/// 绘制「快捷键」页，返回搜索框内光标的矩形（供输入法候选框定位）。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>) -> Option<Rect> {
    let keys = app.keys_layout();
    let caret = draw_search(app, frame, &keys);
    let page = app.settings.selected.page() as u64;
    let shown = (keys.max_scroll() > 0.0).then_some(keys.list);
    let look = app.scrollbar_look(ScrollArea::Settings, page, app.settings.scroll, shown);
    frame.within(keys.list, |frame| draw_list(app, frame, &keys, look));
    caret
}

/// 绘制搜索框，返回搜索框内光标的矩形。
///
/// 样式与设置页中正在编辑的输入框相同（编辑区底色，有焦点时外圈为强调色），左侧是放大镜图标，
/// 右端是「录制按键」按钮。录制时按钮铺强调色底色，占位文本换成录制提示。
fn draw_search(app: &mut App, frame: &mut Frame<'_>, keys: &KeysLayout) -> Option<Rect> {
    let p = app.palette;
    let search = keys.search;
    let focused = app.focus == Focus::KeySearch;
    if focused {
        frame.rounded(search.inset(-1.5, -1.5), p.accent, RADIUS_ROW + 1.5);
    }
    frame.rounded(search, p.editor_bg, RADIUS_ROW);
    if !focused {
        outline(frame, p.border, search);
    }
    let cy = search.y + search.h / 2.0;
    icons::magnifier(frame, (search.x + 16.0, cy), p.text_dim, search);

    let recording = app.settings.recording;
    let record = keys.record;
    let hovered = app.settings.hover == Some(SettingsPart::Record);
    if recording {
        frame.rounded(record, p.accent.alpha(0.18), RADIUS_ROW);
    } else if hovered {
        frame.rounded(record, p.hover, RADIUS_ROW);
    }
    let color = if recording { p.accent } else { p.text_muted };
    icons::keyboard(frame, center(record), color, record);

    let area = Rect::new(
        search.x + 30.0,
        search.y,
        (record.x - 6.0 - search.x - 30.0).max(0.0),
        search.h,
    );
    // 右端的按钮只有图标，因此在占位文本里写出该按钮的快捷键。
    let placeholder = match keymap::shortcut(Action::RecordKeys) {
        _ if recording => t!("settings-keys-recording").to_owned(),
        Some(key) => t!("settings-keys-search-record", key = key),
        None => t!("settings-keys-search").to_owned(),
    };
    let mut field = std::mem::take(&mut app.settings.keys_query);
    let caret = crate::view::draw_bar_field(app, frame, area, &mut field, &placeholder, focused);
    app.settings.keys_query = field;
    caret
}

/// 绘制输入框外沿 1 逻辑像素的细边。
pub(super) fn outline(frame: &mut Frame<'_>, color: Color, rect: Rect) {
    let shapes = icons::rounded_outline(rect.x, rect.y, rect.w, rect.h, RADIUS_ROW);
    frame.stroke(&shapes, 1.0, color, rect.inset(-1.0, -1.0));
}

/// 绘制快捷键的分组列表：每组上方有小标题，组内一行一个命令，名称之后以淡色说明在哪种上下文
/// 可用，右侧是键帽；`look` 是滚动条滑块的显示状态。
fn draw_list(app: &App, frame: &mut Frame<'_>, keys: &KeysLayout, look: ThumbLook) {
    let p = app.palette;
    let list = keys.list;

    if keys.rows.is_empty() {
        let row = Rect::new(
            list.x + SETTINGS_PAD + SETTINGS_INSET,
            list.y,
            list.w,
            KEYS_ROW,
        );
        let message = if app.settings.keys_recorded {
            t!("settings-keys-no-command")
        } else {
            t!("settings-keys-no-match")
        };
        ui_text(frame, app, row.x, row, message, p.text_dim, list);
        return;
    }

    for &(rect, title) in &keys.headings {
        ui_text_small(frame, app, rect.x, rect, title, p.text_dim, list);
    }
    for &group in &keys.groups {
        frame.rect_clipped(group, p.group_bg, RADIUS_BAR, list);
    }
    for &line in &keys.separators {
        frame.rect_clipped(line, p.separator, 0.0, list);
    }

    for (rect, row) in &keys.rows {
        if rect.bottom() < list.y || rect.y > list.bottom() {
            continue;
        }
        // 被 Vim 层占用的键在生效范围之后标注。
        let taken = app.vim_takes_keys(&row.keys).then(|| t!("cmd-when-vim"));
        let note = match (row.when.note(), taken) {
            (Some(when), Some(taken)) => Some(format!("{when} · {taken}")),
            (when, taken) => when.or(taken).map(str::to_owned),
        };
        draw_row(
            app,
            frame,
            *rect,
            row.action,
            note.as_deref(),
            &row.keys,
            list,
        );
    }

    if let Some(thumb) = settings_thumb(list, app.settings.scroll, keys.max_scroll()) {
        let color = look.color(p.scrollbar, p.scrollbar_active);
        frame.rounded(thumb, color, 2.0);
    }
}

/// 绘制一行：名称，其后是淡色的说明（在哪种上下文可用），右侧每个快捷键一个键帽，从右向左排列。
///
/// 名称放不下时截短。
fn draw_row(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    action: Action,
    note: Option<&str>,
    keys: &[String],
    clip: Rect,
) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let mut right = rect.right() - SETTINGS_INSET;
    for key in keys.iter().rev() {
        let w = frame.measure(FontKind::Ui, size, key) + 14.0;
        let chip = Rect::new(right - w, (rect.y + (rect.h - 20.0) / 2.0).round(), w, 20.0);
        frame.rect_clipped(chip, p.selected_inactive, RADIUS_ROW, clip);
        let size = app.fonts.ui_small;
        caps_label(
            frame,
            app,
            size,
            (chip.x + 7.0, chip),
            key,
            p.text_muted,
            clip,
        );
        right = chip.x - 6.0;
    }

    let x = rect.x + SETTINGS_INSET;
    let room = (right - 10.0 - x).max(0.0);
    let title = ellipsize(frame, action.title(), app.fonts.ui, room);
    let end = ui_text(frame, app, x, rect, &title, p.text, clip);
    if let Some(note) = note {
        let room = (right - 10.0 - end - 8.0).max(0.0);
        if room > 24.0 {
            let note = ellipsize(frame, note, size, room);
            ui_text_small(frame, app, end + 8.0, rect, &note, p.text_dim, clip);
        }
    }
}
