//! 设置中的「快捷键」页。
//!
//! 标题行右侧是搜索框（右端是「录制按键」按钮）。页面顶部是「键位方案」一行（点击值弹出方案、
//! 导入和恢复全部默认的菜单），其下按分组列出每个命令及其快捷键。悬停在一行上时出现「编辑」，
//! 改过的行还有「恢复默认」；点击一行或「编辑」后该行展开：已有的键各带 ×，按下的新键加在后面，
//! 与其他命令冲突时说明。快捷键都来自当前的键位表，与按键处理使用同一份数据。

use std::borrow::Cow;

use winc_i18n::t;
use winc_render::{FontKind, Frame, Rect};

use super::*;
use crate::app::Focus;
use crate::keymap::{Action, Binding, Keys, Os, When};
use crate::layout::{KeysHeader, KeysLayout, SettingsPart, settings_thumb};

/// 绘制「快捷键」页，`body` 是标题下方的区域；返回搜索框内光标的矩形（供输入法候选框定位）。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>, body: Rect) -> Option<Rect> {
    let keys = app.keys_layout();
    let caret = draw_search(app, frame, &keys);
    frame.within(body, |frame| draw_keymap_row(app, frame, &keys.header));
    let page = app.settings.selected.page() as u64;
    let shown = (keys.max_scroll() > 0.0).then_some(keys.list);
    let look = app.scrollbar_look(ScrollArea::Settings, page, app.settings.scroll, shown);
    let mut parts = Vec::new();
    frame.within(keys.list, |frame| {
        draw_list(app, frame, &keys, look, &mut parts)
    });
    app.hits.keys_parts = parts;
    caret
}

/// 绘制搜索框，返回搜索框内光标的矩形。
///
/// 样式与分类栏顶部的搜索框相同，右端是「录制按键」按钮。录制时按钮铺强调色底色，占位文本
/// 换成录制提示。
fn draw_search(app: &mut App, frame: &mut Frame<'_>, keys: &KeysLayout) -> Option<Rect> {
    let p = app.palette;
    let search = keys.header.search;
    let focused = app.focus == Focus::KeySearch;
    if focused {
        frame.rounded(search.inset(-1.5, -1.5), p.accent, RADIUS_ROW + 1.5);
    }
    frame.rounded(search, p.editor_bg, RADIUS_ROW);
    if !focused {
        outline(frame, p.border, search);
    }
    let cy = search.y + search.h / 2.0;
    icons::magnifier(frame, (search.x + 14.0, cy), p.text_dim, search);

    let recording = app.settings.recording;
    let record = keys.header.record;
    let hovered = app.settings.hover == Some(SettingsPart::Record);
    if recording {
        frame.rounded(record, p.accent.alpha(0.18), RADIUS_ROW);
    } else if hovered {
        frame.rounded(record, p.hover, RADIUS_ROW);
    }
    let color = if recording { p.accent } else { p.text_muted };
    icons::keyboard(frame, center(record), color, record);

    let x = search.x + 26.0;
    let area = Rect::new(x, search.y, (record.x - 4.0 - x).max(0.0), search.h);
    let placeholder = if recording {
        t!("settings-keys-recording")
    } else {
        t!("settings-keys-search")
    };
    let mut field = std::mem::take(&mut app.settings.keys_query);
    let caret = crate::view::draw_bar_field(app, frame, area, &mut field, placeholder, focused);
    app.settings.keys_query = field;
    caret
}

/// 绘制「键位方案」一行，样式与设置页中带说明的一行相同：名称下方是说明（改过快捷键时是改过
/// 的命令数），右边是当前方案，点击后弹出菜单。
fn draw_keymap_row(app: &App, frame: &mut Frame<'_>, header: &KeysHeader) {
    let p = app.palette;
    let row = header.keymap;
    frame.rect_clipped(row, p.group_bg, RADIUS_BAR, row);
    let x = row.x + SETTINGS_INSET;
    let (top, bottom) = super::two_lines(row);
    ui_text(frame, app, x, top, t!("settings-keys-keymap"), p.text, row);
    let changed = app.config.shortcuts.len();
    let description: Cow<'_, str> = if changed > 0 {
        Cow::Owned(t!("settings-keys-keymap-changed", count = changed))
    } else {
        Cow::Borrowed(t!("settings-keys-keymap-description"))
    };
    let room = crate::layout::description_room(row, header.value);
    let description = truncate_end(frame, &description, app.fonts.ui_small, room);
    ui_text_small(frame, app, x, bottom, &description, p.text_muted, row);
    let hovered = app.settings.hover == Some(SettingsPart::Keymap);
    let preset = crate::keymap::current_preset().title();
    super::controls::popup_value(app, frame, header.value, preset, false, hovered, row);
}

/// 绘制输入框外沿 1 逻辑像素的细边。
pub(super) fn outline(frame: &mut Frame<'_>, color: Color, rect: Rect) {
    let shapes = icons::rounded_outline(rect.x, rect.y, rect.w, rect.h, RADIUS_ROW);
    frame.stroke(&shapes, 1.0, color, rect.inset(-1.0, -1.0));
}

/// 绘制快捷键的分组列表：每组上方有小标题，组内一行一个命令，名称之后以淡色说明在哪种上下文
/// 可用、是否改过，右侧是键帽；`look` 是滚动条滑块的显示状态。绘制出的链接、按钮和 × 的位置
/// 记入 `parts`。
fn draw_list(
    app: &App,
    frame: &mut Frame<'_>,
    keys: &KeysLayout,
    look: ThumbLook,
    parts: &mut Vec<(Rect, SettingsPart)>,
) {
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

    let editing = app.settings.key_edit.as_ref();
    for (rect, row) in keys.rows() {
        if rect.bottom() < list.y || rect.y > list.bottom() {
            continue;
        }
        // 被 Vim 层占用的键、改过的快捷键在生效范围之后标注。
        let taken = app.vim_takes_keys(&row.keys).then(|| t!("cmd-when-vim"));
        let changed = (app.shortcut_customized(row.action)).then(|| t!("settings-keys-modified"));
        let notes: Vec<&str> = [row.when.note(), taken, changed]
            .into_iter()
            .flatten()
            .collect();
        let note = (!notes.is_empty()).then(|| notes.join(" · "));
        if let Some(edit) = editing.filter(|edit| edit.action == row.action) {
            draw_editing_row(app, frame, rect, edit, note.as_deref(), list, parts);
            continue;
        }
        // 整行可点击：悬停时铺浅色，键帽左边出现「编辑」，改过的行还有「恢复默认」。收起途中
        // 的一行内容留在顶端。
        let action = row.action;
        let hovered = matches!(
            app.settings.hover,
            Some(SettingsPart::KeyRow(a) | SettingsPart::KeyEdit(a) | SettingsPart::KeyReset(a))
                if a == action
        );
        if hovered {
            frame.rect_clipped(rect, p.hover, RADIUS_ROW, list);
        }
        let top = Rect::new(rect.x, rect.y, rect.w, KEYS_ROW);
        let mut right = draw_keys(app, frame, top, &row.keys, hovered, list);
        if hovered {
            let mut links = vec![(t!("settings-keys-edit"), SettingsPart::KeyEdit(action))];
            if app.shortcut_customized(action) {
                links.push((t!("settings-keys-reset"), SettingsPart::KeyReset(action)));
            }
            for (label, part) in links {
                let link = draw_link(app, frame, top, right, label, part, list);
                parts.push((link, part));
                right = link.x - 4.0;
            }
        }
        draw_name(app, frame, top, action, note.as_deref(), right, list);
    }

    if let Some(thumb) = settings_thumb(list, app.settings.scroll, keys.max_scroll()) {
        let color = look.color(p.scrollbar, p.scrollbar_active);
        frame.rounded(thumb, color, 2.0);
    }
}

/// 键帽和文字链接的高度。
const CHIP_H: f32 = 20.0;

/// 返回一行 `row` 中右端位于 `right`、宽 `w` 的键帽或链接的位置，各个都按同一中线居中。
fn chip_rect(row: Rect, right: f32, w: f32) -> Rect {
    let y = (row.y + (row.h - CHIP_H) / 2.0).round();
    Rect::new(right - w, y, w, CHIP_H)
}

/// 在一行 `rect` 的右侧绘制各快捷键的键帽，从右向左排列；返回最左一个键帽左边的位置。
///
/// `hovered` 时键帽再叠一层悬停色，以免与该行的悬停底色混在一起。
fn draw_keys(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    keys: &[String],
    hovered: bool,
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let mut right = rect.right() - SETTINGS_INSET;
    for key in keys.iter().rev() {
        let w = frame.measure(FontKind::Ui, size, key) + 14.0;
        let chip = chip_rect(rect, right, w);
        frame.rect_clipped(chip, p.selected_inactive, RADIUS_ROW, clip);
        if hovered {
            frame.rect_clipped(chip, p.hover, RADIUS_ROW, clip);
        }
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
    right
}

/// 在一行 `row` 中右端位于 `right` 处绘制强调色的文字链接 `label`，悬停时铺浅色；返回位置。
fn draw_link(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    right: f32,
    label: &str,
    part: SettingsPart,
    clip: Rect,
) -> Rect {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let w = frame.measure(FontKind::Ui, size, label) + 12.0;
    let link = chip_rect(row, right, w);
    if app.settings.hover == Some(part) {
        frame.rect_clipped(link, p.hover, RADIUS_ROW, clip);
    }
    ui_text_small(frame, app, link.x + 6.0, link, label, p.accent, clip);
    link
}

/// 绘制一行的名称和其后淡色的说明，不越过 `right`；名称放不下时截短。
fn draw_name(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    action: Action,
    note: Option<&str>,
    right: f32,
    clip: Rect,
) {
    let p = app.palette;
    let size = app.fonts.ui_small;
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

/// 绘制正在修改快捷键的一行 `rect`：铺选中底色；第一行是名称；第二行是保留的已有键（各带 ×）、
/// 新键的录入框和「取消」「保存」；第三行说明怎样操作，新键与其他命令冲突时改为说明冲突。
///
/// 展开途中第二、三行裁在该行之内并随之淡入。
fn draw_editing_row(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    edit: &crate::app::KeyEdit,
    note: Option<&str>,
    clip: Rect,
    parts: &mut Vec<(Rect, SettingsPart)>,
) {
    let p = app.palette;
    // 底色与悬停时的同一块，展开时只向下长，名称不动。
    frame.rect_clipped(rect, p.selected_inactive, RADIUS_ROW, clip);
    let t = ((rect.h - KEYS_ROW) / (KEYS_EDIT_H - KEYS_ROW)).clamp(0.0, 1.0);
    let first = Rect::new(rect.x, rect.y, rect.w, KEYS_ROW);
    draw_name(app, frame, first, edit.action, note, rect.right(), clip);
    let full = Rect::new(rect.x, rect.y, rect.w, KEYS_EDIT_H);
    frame.within(rect, |frame| {
        frame.with_opacity(t, |frame| {
            draw_edit_controls(app, frame, full, edit, clip, parts)
        });
    });
}

/// 绘制展开的一行 `rect` 中的第二、三行：保留的键、录入框、按钮，以及说明或冲突。
fn draw_edit_controls(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    edit: &crate::app::KeyEdit,
    clip: Rect,
    parts: &mut Vec<(Rect, SettingsPart)>,
) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let line = |top: f32, h: f32| Rect::new(rect.x, rect.y + top, rect.w, h);
    let left = rect.x + SETTINGS_INSET;

    // 第二行：保留的键各一个键帽，右端的 × 去掉该键；其后是录入框，右侧是两个按钮。
    let second = line(KEYS_ROW, 38.0);
    let y = (second.y + (second.h - KEYS_FIELD_H) / 2.0).round();
    let mut x = left;
    for (i, &keys) in edit.keys.iter().enumerate() {
        let text = keys_label(edit.action, keys);
        let w = frame.measure(FontKind::Ui, size, &text) + 14.0 + 18.0;
        let chip = Rect::new(x, y, w, KEYS_FIELD_H);
        frame.rect_clipped(chip, p.editor_bg, RADIUS_ROW, clip);
        caps_label(
            frame,
            app,
            size,
            (chip.x + 7.0, chip),
            &text,
            p.text_muted,
            clip,
        );
        let remove = Rect::new(chip.right() - 20.0, chip.y + 3.0, 18.0, chip.h - 6.0);
        let part = SettingsPart::KeyRemove(i);
        if app.settings.hover == Some(part) {
            frame.rect_clipped(remove, p.hover, RADIUS_ROW, clip);
        }
        icons::close(frame, center(remove), 7.0, p.text_dim, clip);
        parts.push((remove, part));
        x = chip.right() + 6.0;
    }
    let text = match edit.recorded {
        Some(keys) => keys_label(edit.action, keys),
        None => t!("settings-keys-press").to_owned(),
    };
    let w = (frame.measure(FontKind::Ui, size, &text) + 20.0).max(KEYS_FIELD_W);
    let field = Rect::new(x, y, w, KEYS_FIELD_H);
    // 外圈用描边而不是铺底后再盖住，淡入途中不透出强调色。
    frame.rect_clipped(field, p.editor_bg, RADIUS_ROW, clip);
    let ring = field.inset(-0.75, -0.75);
    let r = RADIUS_ROW + 0.75;
    let shapes = icons::rounded_outline(ring.x, ring.y, ring.w, ring.h, r);
    frame.stroke(&shapes, 1.5, p.accent, clip);
    let color = if edit.recorded.is_some() {
        p.text
    } else {
        p.text_dim
    };
    caps_label(
        frame,
        app,
        size,
        (field.x + 10.0, field),
        &text,
        color,
        clip,
    );

    let mut right = rect.right() - SETTINGS_INSET;
    for (label, role, part) in [
        (
            t!("settings-keys-save"),
            ButtonRole::Primary,
            SettingsPart::KeySave,
        ),
        (
            t!("settings-keys-cancel"),
            ButtonRole::Normal,
            SettingsPart::KeyCancel,
        ),
    ] {
        let w = (frame.measure(FontKind::Ui, app.fonts.ui, label) + 24.0).round();
        let button = Rect::new(right - w, y, w, KEYS_FIELD_H);
        let hovered = app.settings.hover == Some(part);
        draw_action_button(app, frame, button, label, role, true, hovered);
        parts.push((button, part));
        right = button.x - 8.0;
    }

    // 第三行：操作说明，或者新键与其他命令冲突时的说明。
    let third = line(KEYS_ROW + 38.0, KEYS_EDIT_H - KEYS_ROW - 38.0 - 4.0);
    let conflicts = app.key_edit_conflicts();
    let (message, color) = match edit.recorded {
        Some(keys) if app.key_edit_takes_typing() => {
            let key = keys_label(edit.action, keys);
            (t!("settings-keys-takes-typing", key = key), p.warning)
        }
        Some(keys) if !conflicts.is_empty() => {
            let names: Vec<String> = (conflicts.iter())
                .map(|a| t!("workspace-quoted-name", name = a.title()))
                .collect();
            let names = names.join(t!("workspace-list-separator"));
            let key = keys_label(edit.action, keys);
            (
                t!("settings-keys-conflict", key = key, names = names),
                p.warning,
            )
        }
        _ => (t!("settings-keys-edit-hint").to_owned(), p.text_muted),
    };
    let room = (rect.right() - SETTINGS_INSET - left).max(0.0);
    let message = truncate_end(frame, &message, size, room);
    ui_text_small(frame, app, left, third, &message, color, clip);
}

/// 返回按键 `keys` 的显示文字。
fn keys_label(action: Action, (first, second): Keys) -> String {
    let binding = Binding {
        action,
        first,
        second,
        when: When::Global,
    };
    binding.label(Os::CURRENT)
}
