//! 格式工具栏的绘制：选区上方（放不下时在下方）的一行按钮。

use winc_render::{Frame, Rect};

use crate::app::App;
use crate::app::Hover;
use crate::app::preview::markdown::toolbar::Tool;
use crate::icons;
use crate::theme::RADIUS_POPUP;

/// 按钮的边长（逻辑像素）。
const BUTTON: f32 = 28.0;
/// 按钮之间、按钮与卡片边缘的距离（逻辑像素）。
const GAP: f32 = 4.0;
/// 按钮组之间分隔线所占的宽度（逻辑像素）。
const SEPARATOR: f32 = 1.0;
/// 工具栏与选区之间的距离（逻辑像素）。
const OFFSET: f32 = 6.0;

/// 绘制格式工具栏，记录卡片和各按钮的位置；选区已改变或不在视口中时不绘制。
pub(in crate::view) fn draw_selection_toolbar(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let Some(selection) = app.selection_toolbar() else {
        app.preview.markdown.toolbar = None;
        return;
    };
    let Some(rope) = app.editor.active().and_then(|d| d.text().rope()) else {
        return;
    };
    let chars = rope.len_chars();
    let byte = |pos: usize| rope.char_to_byte(pos.min(chars));
    let (from, to) = (byte(selection.from()), byte(selection.to()));
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return;
    };
    let (Some(start), Some(end)) = (laid.caret_point(from), laid.caret_point(to)) else {
        return;
    };
    let top = area.y - app.preview.markdown.scroll();
    let groups = Tool::ALL.iter().filter(|t| t.starts_group()).count() as f32;
    let (left, n) = (laid.left, Tool::ALL.len() as f32);
    let w = n * BUTTON + (n + 1.0) * GAP + groups * (SEPARATOR + GAP);
    let h = BUTTON + GAP * 2.0;
    let x = if (start.1 - end.1).abs() < 0.5 {
        left + (start.0 + end.0) / 2.0 - w / 2.0
    } else {
        left + start.0
    };
    let x = x.min(area.right() - w - 8.0).max(area.x + 8.0).round();
    let above = top + start.1 - OFFSET - h;
    let y = if above >= area.y + 4.0 {
        above
    } else {
        top + end.1 + end.2 + OFFSET
    };
    if y > area.bottom() || y + h < area.y {
        return;
    }
    let card = Rect::new(x, y.round(), w, h);
    let p = app.palette;
    crate::view::draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);
    let active = app.active_tools();
    let mut x = card.x + GAP;
    for tool in Tool::ALL {
        if tool.starts_group() {
            let line = Rect::new(x + GAP / 2.0 - 0.5, card.y + h / 2.0 - 8.0, 1.0, 16.0);
            frame.rect_clipped(line, p.separator, 0.0, card);
            x += SEPARATOR + GAP;
        }
        let button = Rect::new(x, card.y + GAP, BUTTON, BUTTON);
        let hovered = app.pointer.hover == Hover::SelectionTool(tool);
        let on = active.contains(&tool);
        // 选区已有的格式：按钮铺强调色的淡底，图标用强调色。
        if on {
            frame.rect_clipped(button, p.accent.alpha(p.accent.a * 0.14), 6.0, card);
        } else if hovered {
            frame.rect_clipped(button, p.hover, 6.0, card);
        }
        let color = match (on, hovered) {
            (true, _) => p.accent,
            (false, true) => p.text,
            (false, false) => p.text_muted,
        };
        let center = (button.x + BUTTON / 2.0, button.y + BUTTON / 2.0);
        icons::writing(frame, center, tool.glyph(), color, card);
        app.hits
            .selection_tools
            .push((button.intersect(area), tool));
        x += BUTTON + GAP;
    }
    app.hits.selection_card = Some(card.intersect(area));
}
