//! 窗口内绘制的菜单（没有系统菜单时，见 `app/input/context_menu/popup.rs`）：一张和其他浮层
//! 同样的卡片，各行左边留出打勾的一列，右边是快捷键或子菜单的箭头。各级菜单的位置在这里算好，
//! 记回菜单里，悬停和点击按它判断。

use super::*;
use crate::app::{CHECK_COLUMN, ICON_COLUMN, MenuIcon, POPUP_PAD, PopupLevel, PopupRow};
use crate::icons::Direction;

/// 一行的高度（逻辑像素）。
const ROW: f32 = 24.0;
/// 标题下面还有一行小字时，一行的高度（逻辑像素）。
const ROW_TALL: f32 = 40.0;
/// 分隔线那一行的高度（逻辑像素）。
const SEPARATOR: f32 = 9.0;
/// 文字右边到快捷键、箭头的最小间距（逻辑像素）。
const KEY_GAP: f32 = 28.0;
/// 右边留白（逻辑像素）。
const RIGHT_PAD: f32 = 12.0;
/// 菜单的最小、最大宽度（逻辑像素）。
const WIDTH: (f32, f32) = (160.0, 420.0);
/// 菜单离窗口边缘至少留出的距离（逻辑像素）。
const MARGIN: f32 = 8.0;
/// 行底色左右缩进的距离（逻辑像素）。
const ROW_INSET: f32 = 4.0;
/// 选中底色上的文字颜色和次要文字颜色。
pub(super) fn band_ink(p: &Palette) -> (Color, Color) {
    let ink = p.text_on_accent;
    (ink, ink.alpha(ink.a * 0.8))
}

/// 绘制窗口内的菜单（打开着时），并把各级菜单的位置记回菜单。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let Some(mut popup) = app
        .overlays
        .context_menu
        .as_mut()
        .and_then(|menu| menu.popup.take())
    else {
        return;
    };
    let window = app.layout.window;
    for l in 0..popup.levels.len() {
        let (w, content) = measure(app, frame, &popup.levels[l].rows);
        let w = if l == 0 { w.max(popup.min_width) } else { w };
        let h = content.min(window.h - 2.0 * MARGIN).max(0.0);
        let (x, y) = if l == 0 {
            let mut y = popup.at.1;
            // 打勾的那一行对准弹出的位置。
            if popup.at_checked
                && let Some(offset) = checked_offset(&popup.levels[0].rows)
            {
                y -= offset;
            }
            let x = if popup.right_aligned {
                popup.at.0 - w
            } else {
                popup.at.0
            };
            (x, y)
        } else {
            // 子菜单贴着展开它的那一行，在上一级的右边；放不下时放到左边。
            let parent = &popup.levels[l - 1];
            let row = popup.levels[l]
                .parent
                .and_then(|r| parent.row_rects.get(r))
                .copied()
                .unwrap_or(parent.rect);
            let right = parent.rect.right() - 2.0;
            let x = if right + w <= window.right() - MARGIN {
                right
            } else {
                parent.rect.x - w + 2.0
            };
            (x, row.y - POPUP_PAD)
        };
        let x = x.clamp(
            window.x + MARGIN,
            (window.right() - MARGIN - w).max(window.x),
        );
        let y = y.clamp(
            window.y + MARGIN,
            (window.bottom() - MARGIN - h).max(window.y),
        );
        let level = &mut popup.levels[l];
        level.rect = Rect::new(x, y, w, h);
        level.content = content;
        level.scroll = level.scroll.clamp(0.0, (content - h).max(0.0));
        let mut top = y + POPUP_PAD - level.scroll;
        level.row_rects = level
            .rows
            .iter()
            .map(|row| {
                let rect = Rect::new(x, top, w, row_height(row));
                top += rect.h;
                rect
            })
            .collect();
        draw_level(app, frame, level);
    }
    if let Some(menu) = app.overlays.context_menu.as_mut() {
        menu.popup = Some(popup);
    }
}

/// 一级菜单需要的宽度和内容高度（逻辑像素）。
fn measure(app: &App, frame: &mut Frame<'_>, rows: &[PopupRow]) -> (f32, f32) {
    let size = app.fonts.ui;
    let small = app.fonts.ui_small;
    let mut w: f32 = 0.0;
    let mut h = 2.0 * POPUP_PAD;
    let lead = CHECK_COLUMN + icon_column(rows);
    for row in rows {
        h += row_height(row);
        let text = match row {
            PopupRow::Separator => continue,
            PopupRow::Item { label, key, .. } => {
                let title = frame.measure(FontKind::Ui, size, &label.title);
                let subtitle = label
                    .subtitle
                    .as_deref()
                    .map_or(0.0, |s| frame.measure(FontKind::Ui, small, s));
                let key = if key.is_empty() {
                    0.0
                } else {
                    KEY_GAP + frame.measure(FontKind::Ui, small, key)
                };
                title.max(subtitle) + key
            }
            PopupRow::Submenu { label, .. } => {
                frame.measure(FontKind::Ui, size, &label.title) + KEY_GAP
            }
            PopupRow::Entry { name, dir, .. } => {
                let arrow = if *dir { KEY_GAP } else { 0.0 };
                frame.measure(FontKind::Ui, size, name) + arrow
            }
        };
        w = w.max(lead + text + RIGHT_PAD);
    }
    (w.clamp(WIDTH.0, WIDTH.1), h)
}

/// 图标那一列的宽度：有一行带图标时为 [`ICON_COLUMN`]，否则为 0，文字紧接打勾的一列。
fn icon_column(rows: &[PopupRow]) -> f32 {
    let any = rows.iter().any(|row| {
        matches!(
            row,
            PopupRow::Item { icon: Some(_), .. } | PopupRow::Entry { .. }
        )
    });
    if any { ICON_COLUMN } else { 0.0 }
}

/// 一行的高度。
fn row_height(row: &PopupRow) -> f32 {
    match row {
        PopupRow::Separator => SEPARATOR,
        PopupRow::Item { label, .. } if label.subtitle.is_some() => ROW_TALL,
        _ => ROW,
    }
}

/// 打勾的那一行的中线离菜单顶边的距离；没有打勾的行时为 `None`。
fn checked_offset(rows: &[PopupRow]) -> Option<f32> {
    let mut top = POPUP_PAD;
    for row in rows {
        if matches!(
            row,
            PopupRow::Item { checked: true, .. } | PopupRow::Entry { checked: true, .. }
        ) {
            return Some(top + row_height(row) / 2.0);
        }
        top += row_height(row);
    }
    None
}

/// 绘制一级菜单：卡片、各行的悬停底色、勾、文字、快捷键和子菜单箭头。
fn draw_level(app: &App, frame: &mut Frame<'_>, level: &PopupLevel) {
    let p = app.palette;
    let card = level.rect;
    draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);
    let clip = card.inset(0.0, POPUP_PAD);
    let icons = icon_column(&level.rows);
    for (i, (row, rect)) in level.rows.iter().zip(&level.row_rects).enumerate() {
        if rect.bottom() < clip.y || rect.y > clip.bottom() {
            continue;
        }
        let (title, subtitle, icon, checked, enabled, key, submenu) = match row {
            PopupRow::Separator => {
                let y = (rect.y + rect.h / 2.0).round();
                let line = Rect::new(
                    rect.x + ROW_INSET + 6.0,
                    y,
                    rect.w - 2.0 * ROW_INSET - 12.0,
                    1.0,
                );
                frame.rect_clipped(line, p.separator, 0.0, clip);
                continue;
            }
            PopupRow::Item {
                label,
                icon,
                key,
                enabled,
                checked,
                ..
            } => (
                label.title.as_str(),
                label.subtitle.as_deref(),
                *icon,
                *checked,
                *enabled,
                key.as_str(),
                false,
            ),
            PopupRow::Submenu { label, checked, .. } => {
                (label.title.as_str(), None, None, *checked, true, "", true)
            }
            PopupRow::Entry {
                name, dir, checked, ..
            } => {
                let icon = if *dir {
                    MenuIcon::Folder
                } else {
                    let (glyph, colors) = crate::theme::file_kind(name);
                    MenuIcon::File(glyph, colors)
                };
                (name.as_str(), None, Some(icon), *checked, true, "", *dir)
            }
        };
        let hovered = level.hover == Some(i);
        if hovered {
            let band = rect.inset(ROW_INSET, 1.0);
            frame.rect_clipped(band, p.accent_fill, RADIUS_ROW, clip);
        }
        let (ink, dim) = if hovered {
            band_ink(p)
        } else if enabled {
            (p.text, p.text_dim)
        } else {
            (p.text_dim, p.text_dim)
        };
        let title_row = Rect::new(rect.x, rect.y, rect.w, ROW);
        if checked {
            let cy = icon_center_y(app, title_row, app.fonts.ui);
            icons::check(frame, (rect.x + 13.0, cy), 9.0, ink, clip);
        }
        if let Some(icon) = icon {
            let (w, h) = icon.size();
            let cy = icon_center_y(app, title_row, app.fonts.ui);
            let at = (
                rect.x + CHECK_COLUMN + (ICON_COLUMN - 6.0 - w) / 2.0,
                cy - h / 2.0,
            );
            app.draw_menu_icon(frame, icon, at, clip);
        }
        let text_x = rect.x + CHECK_COLUMN + icons;
        let right = rect.right() - RIGHT_PAD;
        let mut room = right - text_x;
        if !key.is_empty() {
            let w = frame.measure(FontKind::Ui, app.fonts.ui_small, key);
            ui_text_small(frame, app, right - w, title_row, key, dim, clip);
            room -= w + KEY_GAP / 2.0;
        }
        if submenu {
            let cy = icon_center_y(app, title_row, app.fonts.ui);
            icons::chevron(frame, (right - 3.0, cy), 6.0, Direction::Right, dim, clip);
            room -= KEY_GAP / 2.0;
        }
        let title = ellipsize(frame, title, app.fonts.ui, room.max(0.0));
        ui_text(frame, app, text_x, title_row, &title, ink, clip);
        if let Some(subtitle) = subtitle {
            let line = Rect::new(rect.x, rect.y + ROW - 6.0, rect.w, ROW - 4.0);
            let subtitle = ellipsize(frame, subtitle, app.fonts.ui_small, room.max(0.0));
            ui_text_small(frame, app, text_x, line, &subtitle, dim, clip);
        }
    }
}
