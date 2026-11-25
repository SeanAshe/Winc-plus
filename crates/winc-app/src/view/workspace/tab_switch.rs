//! 标签页切换列表（`⌃Tab`）的绘制：编辑区上方居中的卡片，每行是图标、名字和淡色的所在目录。

use std::path::Path;

use super::*;
use crate::app::{PICKER_ICON_X, PICKER_MAX_ROWS, PICKER_PAD, PICKER_ROW, PICKER_TEXT_X};

/// 绘制标签页切换列表，并把各行的位置记入 `App::hits`；列表未打开时不绘制。
///
/// 位置和宽度与快速打开卡片相同；行数超过一屏时，列表滚动到选中行可见。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let Some(list) = &app.tab_switch.list else {
        return;
    };
    let editor = app.layout.editor;
    let w = PICKER_WIDTH.min(editor.w - 48.0);
    let shown = list.rows.len().min(PICKER_MAX_ROWS);
    if w <= 0.0 || shown == 0 {
        return;
    }
    let h = shown as f32 * PICKER_ROW + 2.0 * PICKER_PAD;
    let x = (editor.x + (editor.w - w) / 2.0).round();
    let card = Rect::new(x, editor.y + 8.0, w, h);
    let p = app.palette;
    draw_card(frame, p, card, RADIUS_BAR, p.popup_bg);

    let first = (list.selected + 1).saturating_sub(shown);
    let root = app.editor.root().map(Path::to_path_buf);
    let mut hits = Vec::with_capacity(shown);
    for (slot, row_index) in (first..first + shown).enumerate() {
        let Some(doc) = (list.rows.get(row_index)).and_then(|&id| app.editor.document(id)) else {
            continue;
        };
        let row = Rect::new(
            card.x,
            card.y + PICKER_PAD + slot as f32 * PICKER_ROW,
            card.w,
            PICKER_ROW,
        );
        hits.push((row, row_index));
        if row_index == list.selected {
            frame.rect_clipped(row.inset(6.0, 1.0), p.selected_inactive, RADIUS_ROW, card);
        }
        let cy = icon_center_y(app, row, app.fonts.ui);
        if doc.terminal {
            icons::terminal(
                frame,
                (row.x + PICKER_ICON_X + icons::FILE_WIDTH / 2.0, cy),
                p.text_muted,
                card,
            );
        } else {
            let icon = tab_icon_name(doc);
            draw_file_icon(frame, p, (row.x + PICKER_ICON_X, cy), &icon, card);
        }
        let name = doc.name();
        let right = row.right() - 12.0;
        let name_x = row.x + PICKER_TEXT_X;
        let shown_name = ellipsize(frame, &name, app.fonts.ui, (right - name_x).max(0.0));
        ui_text(frame, app, name_x, row, &shown_name, p.text, card);

        // 所在目录：工作区内的写相对路径，工作区外的写完整路径。
        let parent = doc.path.as_deref().and_then(Path::parent);
        let folder = parent.map(|dir| match &root {
            Some(root) => dir.strip_prefix(root).unwrap_or(dir).display().to_string(),
            None => dir.display().to_string(),
        });
        let name_w = frame.measure(FontKind::Ui, app.fonts.ui, &shown_name);
        let dir_x = name_x + name_w + 8.0;
        if let Some(folder) = folder.filter(|f: &String| !f.is_empty())
            && right - dir_x > 20.0
        {
            let dim = ellipsize(frame, folder.as_str(), app.fonts.ui_small, right - dir_x);
            ui_text_small(frame, app, dir_x, row, &dim, p.text_dim, card);
        }
    }
    app.hits.tab_switch = hits;
}
