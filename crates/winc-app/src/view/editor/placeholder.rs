//! 编辑区的说明页：打不开的文件、没有打开文件时的欢迎页。

use winc_i18n::t;

use super::*;

/// 绘制编辑区不显示文本时的说明页：正中是描边的文档图标、一句原因和可用的操作按钮。
///
/// 第一个按钮是主要操作，可以没有按钮；编辑区高度不足 200 逻辑像素时不绘制图标。
pub(super) fn draw_placeholder(
    app: &mut App,
    frame: &mut Frame<'_>,
    message: String,
    actions: &[(PlaceholderAction, &str)],
) {
    // 图标、原因与按钮的尺寸及间距（逻辑像素）。
    const ICON: (f32, f32) = (30.0, 38.0);
    const MESSAGE_H: f32 = 20.0;
    const BUTTON_H: f32 = 28.0;
    const GAP: f32 = 8.0;

    let p = app.palette;
    let editor = app.layout.editor;
    let icon_h = if editor.h >= 200.0 {
        ICON.1 + 18.0
    } else {
        0.0
    };
    let buttons_h = if actions.is_empty() {
        0.0
    } else {
        18.0 + BUTTON_H
    };
    let total = icon_h + MESSAGE_H + buttons_h;
    let cx = editor.x + editor.w / 2.0;
    let mut y = (editor.y + (editor.h - total) / 2.0).round();

    if icon_h > 0.0 {
        let x = (cx - ICON.0 / 2.0).round();
        icons::document(frame, (x, y), ICON, p.text_dim, editor);
        y += icon_h;
    }

    let message = ellipsize(frame, &message, app.fonts.ui, editor.w - 48.0);
    let w = frame.measure(FontKind::Ui, app.fonts.ui, &message);
    let row = Rect::new(editor.x, y, editor.w, MESSAGE_H);
    ui_text(
        frame,
        app,
        (cx - w / 2.0).round(),
        row,
        &message,
        p.text_muted,
        editor,
    );
    y = row.bottom() + 18.0;

    let widths: Vec<f32> = actions
        .iter()
        .map(|(_, label)| frame.measure(FontKind::Ui, app.fonts.ui_small, label) + 24.0)
        .collect();
    let row_w = widths.iter().sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32;
    let mut x = (cx - row_w / 2.0).round();
    let mut hits = Vec::with_capacity(actions.len());

    for (i, (&(action, label), w)) in actions.iter().zip(widths).enumerate() {
        let rect = Rect::new(x, y, w.round(), BUTTON_H);
        let (background, color) = if i == 0 {
            (p.accent_fill, p.text_on_accent)
        } else {
            (p.selected_inactive, p.text)
        };
        frame.rect_clipped(rect, background, RADIUS_ROW, editor);

        if app.hover == Hover::Placeholder(action) {
            frame.rect_clipped(rect, p.hover, RADIUS_ROW, editor);
        }

        ui_text_small(frame, app, rect.x + 12.0, rect, label, color, editor);
        hits.push((rect, action));
        x = rect.right() + GAP;
    }

    app.hits.placeholder = hits;
}

/// 绘制打开了目录但没有标签时的欢迎页：几条常用操作，前面是图标，快捷键放在圆角键帽里。
pub(super) fn draw_welcome(app: &App, frame: &mut Frame<'_>) {
    #[derive(Clone, Copy)]
    enum Icon {
        Open,
        OpenFile,
        New,
        Find,
        Quick,
        Sidebar,
        Settings,
    }

    // 只有 macOS 的选择器能同时选文件和目录，其他平台上打开文件和打开目录是两项。
    let mut items = if cfg!(target_os = "macos") {
        vec![(Icon::Open, t!("editor-welcome-open"), Action::OpenFile)]
    } else {
        vec![
            (
                Icon::OpenFile,
                t!("editor-welcome-open-file"),
                Action::OpenFile,
            ),
            (
                Icon::Open,
                t!("editor-welcome-open-folder"),
                Action::OpenFolder,
            ),
        ]
    };
    items.extend([
        (Icon::New, t!("editor-welcome-new-file"), Action::NewFile),
        (
            Icon::Quick,
            t!("editor-welcome-go-to-file"),
            Action::QuickOpen,
        ),
        (
            Icon::Find,
            t!("editor-welcome-find-in-files"),
            Action::FindInFiles,
        ),
        (
            Icon::Sidebar,
            t!("editor-welcome-toggle-sidebar"),
            Action::ToggleSidebar,
        ),
        (
            Icon::Settings,
            t!("editor-welcome-settings"),
            Action::Settings,
        ),
    ]);
    // 一行的高度与整列的宽度（逻辑像素）。
    const ROW: f32 = 36.0;
    const WIDTH: f32 = 280.0;

    let p = app.palette;
    let editor = app.layout.editor;
    let left = (editor.x + (editor.w - WIDTH) / 2.0).round();
    let top = (editor.y + (editor.h - items.len() as f32 * ROW) / 2.0).round();

    for (i, (icon, action, key)) in items.into_iter().enumerate() {
        let row = Rect::new(left, top + i as f32 * ROW, WIDTH, ROW);
        let (cx, cy) = (row.x + 10.0, icon_center_y(app, row, app.fonts.ui));
        let color = p.text_muted;

        match icon {
            Icon::Open => icons::folder(frame, (cx - 7.5, cy - 6.0), true, color, editor),
            Icon::OpenFile => icons::files(frame, (cx, cy), 1.0, color, editor),
            Icon::New => icons::new_file(frame, (cx - 6.0, cy - 7.5), color, editor),
            Icon::Find => icons::search(frame, (cx, cy), 1.0, color, editor),
            Icon::Quick => icons::files(frame, (cx, cy), 1.0, color, editor),
            Icon::Sidebar => icons::sidebar(frame, (cx, cy), true, color, editor),
            Icon::Settings => icons::gear(frame, (cx, cy), color, editor),
        }

        ui_text(frame, app, row.x + 32.0, row, action, p.text_muted, editor);
        if let Some(key) = keymap::shortcut(key) {
            draw_keycap(app, frame, row, &key, editor);
        }
    }
}
