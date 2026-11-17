//! 很小的窗口的测试：窗口比界面的各部分还小时照常布局、绘制和处理输入，不因几何退化而崩溃。

use winc_render::Renderer;

use super::*;
use crate::keymap::Action;

/// 把离屏界面的窗口改为 `size`（逻辑像素）并重新布局。
fn resize(app: &mut App, size: (f32, f32)) {
    let background = app.palette.frame_bg;
    let renderer = Renderer::offscreen(size.0, size.1, SCALE, background, None, None).unwrap();
    app.frame.renderer = Some(renderer);
    app.relayout();
    app.clamp_scroll();
}

/// 在当前界面上依次绘制、移动鼠标、滚动、按下并松开左键。
fn exercise(app: &mut App, size: (f32, f32)) {
    paint(app);
    for (x, y) in [(1.0, 1.0), (size.0 / 2.0, size.1 / 2.0), (size.0, size.1)] {
        app.on_mouse_move(x, y);
        app.on_wheel(MouseScrollDelta::LineDelta(1.0, -2.0));
        paint(app);
    }
    app.on_mouse_move(size.0 / 2.0, size.1 / 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(size.0, size.1);
    app.on_mouse_up(MouseButton::Left);
    paint(app);
}

/// 回归：菜单打开时窗口缩到比菜单还小（最小化时窗口尺寸为 0）。
#[test]
fn an_open_menu_survives_a_window_smaller_than_itself() {
    let dir = sample_folder("tiny-menu");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let at = tree_row_center(&app, 1);
    right_click(&mut app, at);
    paint(&mut app);
    let wide = app.popup().unwrap().levels[0].rect;
    for size in [
        (wide.w + 12.0, 300.0),
        (120.0, 40.0),
        (12.0, 12.0),
        (0.5, 0.5),
    ] {
        resize(&mut app, size);
        paint(&mut app);
        // 菜单留在窗口内：窗口放不下时贴着左上的边距，宽高不超过窗口。
        if let Some(popup) = app.popup() {
            let rect = popup.levels[0].rect;
            assert!(rect.w <= size.0 && rect.h <= size.1, "{size:?} {rect:?}");
            assert!(rect.x >= 0.0 && rect.y >= 0.0, "{size:?} {rect:?}");
        }
    }
    resize(&mut app, SIZE);
    paint(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 各种界面在很小的窗口里照常工作。
#[test]
fn every_surface_survives_a_tiny_window() {
    type Scene = (&'static str, fn(&mut App));
    let scenes: [Scene; 11] = [
        ("preview", |app| _ = app.run_action(Action::TogglePreview)),
        ("log", |app| _ = app.run_action(Action::GitShowLog)),
        ("editor", |_| {}),
        ("menu", |app| {
            let at = tree_row_center(app, 1);
            right_click(app, at);
        }),
        ("settings", |app| app.open_settings()),
        ("find", |app| _ = app.run_action(Action::Find)),
        ("find-in-files", |app| {
            _ = app.run_action(Action::FindInFiles)
        }),
        ("outline", |app| _ = app.run_action(Action::ShowOutline)),
        ("recent", |app| _ = app.run_action(Action::OpenRecent)),
        ("commit", |app| _ = app.run_action(Action::ShowCommit)),
        ("wrap", |app| _ = app.run_action(Action::ToggleWordWrap)),
    ];
    let sizes = [
        (0.5, 0.5),
        (1.0, 1.0),
        (12.0, 12.0),
        (40.0, 300.0),
        (300.0, 40.0),
        (170.0, 120.0),
        (420.0, 200.0),
    ];
    for (name, open) in scenes {
        let dir = sample_folder(&format!("tiny-{name}"));
        let file = if name == "preview" {
            "notes.md"
        } else {
            "short.rs"
        };
        let notes = "# 标题\n\n正文 `code`\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        std::fs::write(dir.join("notes.md"), notes).unwrap();
        let mut app = app_with(&dir.join(file), false);
        paint(&mut app);
        open(&mut app);
        paint(&mut app);
        for size in sizes {
            resize(&mut app, size);
            exercise(&mut app, size);
        }
        // 打开着界面时缩小，再在小窗口里重新打开一次。
        open(&mut app);
        exercise(&mut app, sizes[4]);
        resize(&mut app, SIZE);
        paint(&mut app);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
