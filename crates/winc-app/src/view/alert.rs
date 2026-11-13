//! 窗口里画出来的提示框（没有系统提示框时，见 `app/lifecycle/alert.rs`）：窗口上铺一层遮罩，
//! 中间一张卡片，上面是主句和说明，下面左边是「不再询问」这样的勾选框，右边一排按钮。
//! 第一个按钮是默认按钮（↩），排在最右边、铺强调色；危险操作的按钮文字用错误色。卡片的宽度、
//! 内边距，按钮和勾选框的样式，以及出现、消失时连同遮罩一起淡入淡出，都与功能对话框
//! （`view/dialog.rs`）相同。各部分的位置记回提示框，悬停和点击按它判断。

use super::*;
use crate::app::{AlertPart, AlertParts};

/// 卡片的内边距（逻辑像素）。
const PAD: f32 = SETTINGS_PAD;
/// 按钮的高度（逻辑像素）。
const BUTTON_H: f32 = SETTINGS_CONTROL_H;
/// 按钮文字两边的留白（逻辑像素）；按钮至少与对话框的按钮一样宽（`DIALOG_BUTTON_W`）。
const BUTTON_PAD: f32 = 14.0;
/// 相邻两个按钮的间距（逻辑像素）。
const BUTTON_GAP: f32 = 8.0;
/// 主句、说明一行的高度（逻辑像素）。
const LINE: f32 = 20.0;

/// 绘制窗口内的提示框（打开着时），并把各部分的位置记回提示框；关闭后淡出期间绘制最后一帧。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    app.track_card(Card::Alert, app.overlays.alert.is_some());
    let Some(alert) = app.overlays.alert.as_ref() else {
        if let Some((clock, _, _)) = app.motion.alert_card.closing() {
            let fade = dialog_curve(false, clock).0;
            let scrim = app.palette.scrim;
            frame.rect(app.layout.window, scrim.alpha(scrim.a * fade));
        }
        draw_card_ghost(app, frame, Card::Alert);
        return;
    };
    let p = app.palette;
    let window = app.layout.window;
    let width = DIALOG_W.min(window.w - 32.0).max(160.0);
    let text_w = width - 2.0 * PAD;
    let (size, small) = (app.fonts.ui, app.fonts.ui_small);

    let message: Vec<String> = lines(frame, size, &alert.message, text_w);
    let detail: Vec<String> = if alert.detail.is_empty() {
        Vec::new()
    } else {
        lines(frame, small, &alert.detail, text_w)
    };
    let text_h = message.len() as f32 * LINE
        + if detail.is_empty() {
            0.0
        } else {
            6.0 + detail.len() as f32 * (LINE - 2.0)
        };
    let height = PAD + text_h + PAD + BUTTON_H + PAD;
    let card = Rect::new(
        (window.x + (window.w - width) / 2.0).round(),
        (window.y + (window.h - height) / 2.0).round(),
        width,
        height,
    );

    let fade = match app.motion.alert_card.motion {
        Some((true, clock)) => dialog_curve(true, clock).0,
        _ => 1.0,
    };
    frame.rect(window, p.scrim.alpha(p.scrim.a * fade));
    draw_moving_card(app, frame, Card::Alert, card, |app, frame| {
        draw_contents(app, frame, card, (&message, &detail));
    });
}

/// 绘制卡片 `card` 和其中的主句、说明、按钮、勾选框，并把各部分的位置记回提示框。
fn draw_contents(
    app: &mut App,
    frame: &mut Frame<'_>,
    card: Rect,
    (message, detail): (&[String], &[String]),
) {
    let Some(alert) = app.overlays.alert.as_ref() else {
        return;
    };
    let p = app.palette;
    let size = app.fonts.ui;
    let text_w = card.w - 2.0 * PAD;
    draw_card(frame, p, card, RADIUS_CARD, p.editor_bg);

    // 主句和说明。
    let mut y = card.y + PAD;
    for line in message {
        let row = Rect::new(card.x + PAD, y, text_w, LINE);
        ui_text(frame, app, row.x, row, line, p.text, card);
        y += LINE;
    }
    if !detail.is_empty() {
        y += 6.0;
        for line in detail {
            let row = Rect::new(card.x + PAD, y, text_w, LINE - 2.0);
            ui_text_small(frame, app, row.x, row, line, p.text_muted, card);
            y += LINE - 2.0;
        }
    }

    // 按钮：第一个在最右边，其余依次向左。
    let row_y = card.bottom() - PAD - BUTTON_H;
    let mut right = card.right() - PAD;
    let mut buttons = Vec::with_capacity(alert.buttons.len());
    for (i, button) in alert.buttons.iter().enumerate() {
        let w = (frame.measure(FontKind::Ui, size, &button.title) + 2.0 * BUTTON_PAD)
            .max(DIALOG_BUTTON_W);
        let rect = Rect::new(right - w, row_y, w, BUTTON_H);
        right = rect.x - BUTTON_GAP;
        let hovered = alert.hover == Some(AlertPart::Button(i));
        let role = if i == 0 && button.key.is_none() {
            ButtonRole::Primary
        } else if button.destructive {
            ButtonRole::Destructive
        } else {
            ButtonRole::Normal
        };
        draw_action_button(app, frame, rect, &button.title, role, true, hovered);
        buttons.push(rect);
    }

    // 勾选框在按钮那一行的左边。
    let checkbox = alert.suppression.as_ref().map(|text| {
        let w = frame.measure(FontKind::Ui, size, text);
        let row = Rect::new(
            card.x + PAD,
            row_y,
            (w + 20.0).min(right - card.x - PAD),
            BUTTON_H,
        );
        draw_check_option(app, frame, row, text, alert.checked);
        row
    });

    if let Some(alert) = app.overlays.alert.as_mut() {
        alert.parts = AlertParts { buttons, checkbox };
    }
}

/// 按宽度 `width` 折好 `text`：先按换行符分段，每段再折行；空行保留。
fn lines(frame: &mut Frame<'_>, size: f32, text: &str, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        out.extend(
            git::log::wrap(frame, size, paragraph, width)
                .into_iter()
                .map(str::to_owned),
        );
    }
    out
}
