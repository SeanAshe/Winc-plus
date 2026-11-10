//! 开始页：没有打开目录时，编辑区内显示的应用名称、开始操作和最近目录。

use winc_i18n::t;
use winc_render::{Frame, Rect};

use super::*;
use crate::app::StartAction;

/// 内容列的宽度（逻辑像素）。
const WIDTH: f32 = 420.0;
/// 一行的高度（逻辑像素）。
const ROW: f32 = 32.0;
/// 小标题的高度（逻辑像素）。
const SECTION_TITLE: f32 = 28.0;
/// 两段之间的空隙（逻辑像素）。
const GAP: f32 = 20.0;
/// 最近列表最多显示的目录数（个）。
const MAX_SHOWN: usize = 8;

/// 「开始」一段的操作：操作、用于显示快捷键的键位表动作；行上的文字见 [`action_label`]。
///
/// 只有 macOS 的选择器能同时选文件和目录，其他平台上打开文件和打开目录是两项。
const ACTIONS: &[(StartAction, Action)] = if cfg!(target_os = "macos") {
    &[
        (StartAction::Settings, Action::Settings),
        (StartAction::NewFile, Action::NewFile),
        (StartAction::Open, Action::OpenFile),
    ]
} else {
    &[
        (StartAction::Settings, Action::Settings),
        (StartAction::NewFile, Action::NewFile),
        (StartAction::Open, Action::OpenFile),
        (StartAction::OpenFolder, Action::OpenFolder),
    ]
};

/// 返回「开始」一段中操作 `action` 所在行上的文字。
fn action_label(action: StartAction) -> &'static str {
    match action {
        StartAction::Settings => t!("workspace-start-settings"),
        StartAction::NewFile => t!("workspace-start-new-file"),
        StartAction::OpenFolder => t!("workspace-start-open-folder"),
        _ if cfg!(target_os = "macos") => t!("workspace-start-open"),
        _ => t!("workspace-start-open-file"),
    }
}

/// 绘制开始页，并登记各行的点击区域（`start_hits`）。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let editor = app.layout.editor;
    let recent = app.state.recent.len().min(MAX_SHOWN);

    // 先计算总高度，使整块内容在编辑区内垂直居中，并整体上移 24 逻辑像素
    // （上边距至少 16 逻辑像素）。
    // 标题块的高度（逻辑像素）：应用名 34、版本号 20，再留 2 的间隔。
    let title_h = 56.0;
    let actions_h = SECTION_TITLE + ACTIONS.len() as f32 * ROW;
    let recent_h = if recent > 0 {
        GAP + SECTION_TITLE + recent as f32 * ROW
    } else {
        0.0
    };
    let total = title_h + GAP + actions_h + recent_h;
    let w = WIDTH.min(editor.w - 48.0).max(0.0);
    let x = (editor.x + (editor.w - w) / 2.0).round();
    let mut y = (editor.y + ((editor.h - total) / 2.0 - 24.0).max(16.0)).round();

    let title = Rect::new(x + 10.0, y, w, 34.0);
    crate::view::label(
        frame,
        app,
        app.fonts.ui + 13.0,
        (title.x, title),
        crate::layout::PRODUCT_NAME,
        p.text,
        editor,
    );
    let version = Rect::new(title.x, title.bottom(), w, 20.0);
    let text = t!(
        "workspace-start-version",
        version = crate::release_version::display_version()
    );
    ui_text_small(frame, app, version.x, version, &text, p.text_dim, editor);
    y += title_h + GAP;

    y = section_title(app, frame, t!("workspace-start-section-start"), x, y, w);
    for &(action, key) in ACTIONS {
        let label = action_label(action);
        let row = Rect::new(x, y, w, ROW);
        draw_row_bg(app, frame, row, action);
        action_icon(app, frame, action, row);
        ui_text(frame, app, row.x + 36.0, row, label, p.text, editor);
        if let Some(key) = keymap::shortcut(key) {
            draw_keycap(app, frame, row.inset(6.0, 0.0), &key, editor);
        }
        app.hits.start.push((row, action));
        y += ROW;
    }

    if recent == 0 {
        return;
    }

    y += GAP;
    y = section_title(app, frame, t!("workspace-start-section-recent"), x, y, w);

    for index in 0..recent {
        let row = Rect::new(x, y, w, ROW);
        let open = StartAction::Recent(index);
        let forget = StartAction::Forget(index);
        let hovered = matches!(app.hover, Hover::Start(a) if a == open || a == forget);
        draw_row_bg(app, frame, row, open);

        let folder = app.state.recent[index].clone();
        let name = winc_editor::display_name(&folder).into_owned();

        let cy = icon_center_y(app, row, app.fonts.ui);
        icons::folder(frame, (row.x + 11.0, cy - 6.0), false, p.folder, editor);

        // ✕ 只在悬停时显示，但其位置始终预留，名称和路径不会因 ✕ 出现而移动。
        let close = Rect::new(
            row.right() - 4.0 - BUTTON_SMALL,
            row.y + (ROW - BUTTON_SMALL) / 2.0,
            BUTTON_SMALL,
            BUTTON_SMALL,
        );
        let room = (close.x - 8.0 - (row.x + 36.0)).max(0.0);
        let name = ellipsize(frame, &name, app.fonts.ui, room);
        let end = ui_text(frame, app, row.x + 36.0, row, &name, p.text, editor);

        // 名称之后是淡色的上级目录（用户主目录缩写为 `~`），放不下时从中间截短。
        let parent = folder
            .parent()
            .map(|p| winc_editor::display_path(p, None))
            .unwrap_or_default();
        let room = close.x - 8.0 - (end + 10.0);
        if !parent.is_empty() && room > 40.0 {
            let parent = ellipsize(frame, &parent, app.fonts.ui_small, room);
            ui_text_small(frame, app, end + 10.0, row, &parent, p.text_dim, editor);
        }

        if hovered {
            draw_button(
                frame,
                p,
                close,
                editor,
                app.hover == Hover::Start(forget),
                false,
            );
            icons::close(frame, center(close), 6.5, p.text_muted, editor);
        }

        // ✕ 的命中区域排在整行之前：二者都包含鼠标时，先匹配到的是 ✕。
        app.hits.start.push((close, forget));
        app.hits.start.push((row, open));
        y += ROW;
    }
}

/// 绘制一段的小标题，返回其下方内容起始位置的 y 坐标。
fn section_title(app: &App, frame: &mut Frame<'_>, text: &str, x: f32, y: f32, w: f32) -> f32 {
    let rect = Rect::new(x + 10.0, y, w, SECTION_TITLE);
    let p = app.palette;
    ui_text_small(
        frame,
        app,
        rect.x,
        rect,
        text,
        p.text_dim,
        app.layout.editor,
    );
    y + SECTION_TITLE
}

/// 绘制悬停时的圆角底色，样式与文件树的行相同。
fn draw_row_bg(app: &App, frame: &mut Frame<'_>, row: Rect, action: StartAction) {
    let hovered = match app.hover {
        Hover::Start(StartAction::Forget(i)) => action == StartAction::Recent(i),
        Hover::Start(a) => a == action,
        _ => false,
    };
    if hovered {
        frame.rounded(row, app.palette.hover, RADIUS_ROW);
    }
}

/// 绘制操作行前面的图标，与欢迎页上同一操作的图标相同。
fn action_icon(app: &App, frame: &mut Frame<'_>, action: StartAction, row: Rect) {
    let p = app.palette;
    let (cx, cy) = (row.x + 18.0, icon_center_y(app, row, app.fonts.ui));
    let clip = app.layout.editor;
    let color = p.text_muted;

    // 图标与空编辑区页上同一操作的图标相同。
    match action {
        StartAction::Settings => icons::gear(frame, (cx, cy), color, clip),
        StartAction::NewFile => icons::new_file(frame, (cx - 6.0, cy - 7.5), color, clip),
        StartAction::Open if !cfg!(target_os = "macos") => {
            icons::files(frame, (cx, cy), 1.0, color, clip);
        }
        StartAction::Open | StartAction::OpenFolder => {
            icons::folder(frame, (cx - 7.5, cy - 6.0), true, color, clip);
        }
        StartAction::Recent(_) | StartAction::Forget(_) => {}
    }
}
