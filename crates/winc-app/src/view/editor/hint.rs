//! 名字旁边的提示气泡：一张小卡片，带一个指向名字的小三角。
//!
//! 气泡在名字所在行的上方，上方放不下时在下方；出现时向名字那一侧滑出并淡入，消失时原地淡出，
//! 与位置浮窗的过渡一致。

use winc_render::Stroke;

use super::*;

/// 气泡的高度（逻辑像素）。
const HEIGHT: f32 = 34.0;
/// 文字左右的留白（逻辑像素）。
const PAD: f32 = 14.0;
/// 小三角的高度（逻辑像素），底边宽为它的两倍。
const POINTER: f32 = 6.0;
/// 小三角尖端与名字所在行之间的距离（逻辑像素）。
const GAP: f32 = 2.0;
/// 出现时滑动的距离（逻辑像素）。
const SLIDE: f32 = 3.0;

/// 绘制名字旁的提示气泡；没有气泡、它挂的文档不是当前文档，或名字不在屏幕上时不绘制。
pub(in crate::view) fn draw_name_hint(app: &App, frame: &mut Frame<'_>) {
    let Some(hint) = &app.jumps.name_hint else {
        return;
    };
    if app.editor.active().map(|d| d.id) != Some(hint.doc) {
        return;
    }
    let editor = app.layout.editor;
    let line_h = app.fonts.line_height;
    let Some((name_x, top)) = app
        .anchor_point(hint.anchor)
        .filter(|&(_, y)| y >= editor.y && y + line_h <= editor.bottom())
    else {
        return;
    };

    let p = app.palette;
    let w = frame.measure(FontKind::Ui, app.fonts.ui, &hint.text) + 2.0 * PAD;
    let right = editor.right() - SCROLLBAR_HIT - 8.0;
    let x = (name_x - 20.0).min(right - w).max(editor.x + 8.0).round();
    let above = top - GAP - POINTER - HEIGHT >= editor.y + 4.0;
    let y = if above {
        top - GAP - POINTER - HEIGHT
    } else {
        top + line_h + GAP + POINTER
    };

    // 出现：不透明度二次缓出，位置三次缓出；消失：原地淡出，二次缓入。
    let (opacity, away) = match (hint.fade_in, hint.fade_out) {
        (_, Some(out)) => {
            let t = out.linear();
            (1.0 - t * t, 0.0)
        }
        (Some(fade_in), None) => {
            let t = fade_in.linear();
            (1.0 - (1.0 - t).powi(2), (1.0 - t).powi(3))
        }
        (None, None) => (1.0, 0.0),
    };
    let toward_name = if above { 1.0 } else { -1.0 };
    let card = Rect::new(x, (y + toward_name * SLIDE * away).round(), w, HEIGHT);
    let tip_x = (name_x + 6.0)
        .clamp(card.x + 16.0, card.right() - 16.0)
        .round();

    frame.with_opacity(opacity, |frame| {
        draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);
        draw_pointer(frame, p, card, tip_x, above);
        ui_text(frame, app, card.x + PAD, card, &hint.text, p.text, card);
    });
}

/// 在卡片 `card` 的下边（`down` 为真）或上边绘制一个尖端横坐标为 `tip_x` 的小三角，填充与
/// 卡片相同，两条斜边描上卡片的边框色；三角的底边盖住卡片在那一段的边框，与卡片连成一体。
fn draw_pointer(frame: &mut Frame<'_>, p: &Palette, card: Rect, tip_x: f32, down: bool) {
    let edge = if down { card.bottom() } else { card.y };
    let dir = if down { 1.0 } else { -1.0 };
    // 每半个逻辑像素一条，自底边向尖端逐条收窄；第一条压住卡片的边框。
    let steps = (POINTER * 2.0) as usize;
    for i in 0..=steps {
        let d = i as f32 * 0.5;
        let half = POINTER - d;
        let y = if down { edge + d - 1.0 } else { edge - d + 0.5 };
        let strip = Rect::new(tip_x - half, y, 2.0 * half, 0.5);
        frame.rect(strip, p.popup_bg);
    }
    let tip = (tip_x, edge + dir * POINTER);
    let lines = [
        Stroke::Line(tip_x - POINTER, edge, tip.0, tip.1),
        Stroke::Line(tip.0, tip.1, tip_x + POINTER, edge),
    ];
    frame.stroke(&lines, 1.0, p.popup_border, frame.bounds());
}
