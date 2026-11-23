//! 很小的窗口的测试：窗口比界面的各部分还小时仍然布局、绘制和处理输入，不因几何退化而 panic。

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

/// 终端在右边最大化时编辑区宽度为零，日志按钮不能画到侧栏的 Git 工具栏上。
#[test]
fn log_stays_inside_the_editor() {
    for dark in [false, true] {
        for right in [false, true] {
            let mut app = blank_app(dark);
            app.panels.sidebar.view = SidebarView::Commit;
            app.config.terminal_enabled = Some(true);
            app.config.terminal_right = Some(right);
            app.terminal.open = true;
            app.terminal.maximized = true;
            paint(&mut app);
            let toolbar = app.commit_parts().header;
            let pixels = |app: &App| {
                let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
                let width = image.width as usize;
                let mut pixels = Vec::new();
                for y in (toolbar.y * SCALE) as usize..(toolbar.bottom() * SCALE) as usize {
                    let left = y * width + (toolbar.x * SCALE) as usize;
                    let right = y * width + (toolbar.right() * SCALE) as usize;
                    pixels.extend_from_slice(&image.pixels[left..right]);
                }
                pixels
            };
            let before = pixels(&app);
            app.editor.show_log("Git log");
            paint(&mut app);
            assert!(app.layout.editor.w == 0.0 || app.layout.editor.h == 0.0);
            assert!(pixels(&app) == before, "dark={dark}, right={right}");
        }
    }
}

/// 单独绘制一个区域，检查它不会改动画布上区域以外的像素。
fn assert_contained(app: &mut App, area: Rect, scale: f32, draw: fn(&mut App, &mut Frame<'_>)) {
    let mut renderer =
        Renderer::offscreen(SIZE.0, SIZE.1, scale, Color::hex(0x7f2359), None, None).unwrap();
    renderer.draw(|_| {});
    let background = renderer.image().unwrap().pixels[0];
    app.hits.clear();
    renderer.draw(|frame| draw(app, frame));
    let image = renderer.image().unwrap();
    for (i, &pixel) in image.pixels.iter().enumerate() {
        let (x, y) = (i % image.width as usize, i / image.width as usize);
        let inside = area.w > 0.0
            && area.h > 0.0
            && (x as f32) >= (area.x * scale).floor()
            && (x as f32) < (area.right() * scale).ceil()
            && (y as f32) >= (area.y * scale).floor()
            && (y as f32) < (area.bottom() * scale).ceil();
        assert!(
            inside || pixel == background,
            "outside {area:?} at ({x}, {y}), scale={scale}"
        );
    }
}

/// 差异和日志的固定宽度按钮，以及空编辑区的提示，都必须留在编辑区内。
#[test]
fn editor_surfaces_stay_inside_the_editor() {
    use winc_editor::diff_view::{DiffSide, DiffView};

    for dark in [false, true] {
        for scene in ["welcome", "diff", "log"] {
            let mut app = blank_app(dark);
            match scene {
                "diff" => {
                    let lang = winc_syntax::detect("sample.rs");
                    app.editor.show_diff(
                        "sample.rs",
                        DiffView::new(
                            DiffSide::new("before", "old\n", None, lang),
                            DiffSide::new("after", "new\n", None, lang),
                        ),
                    );
                }
                "log" => {
                    app.editor.show_log("Git log");
                }
                _ => {}
            }
            for scale in [1.25, 2.0] {
                for (w, h) in [(0.0, 120.0), (96.0, 0.0), (28.0, 24.0), (96.0, 120.0)] {
                    app.layout.editor = Rect::new(300.0, 80.0, w, h);
                    assert_contained(
                        &mut app,
                        Rect::new(300.0, 80.0, w, h),
                        scale,
                        view::tests::draw_editor,
                    );
                    for rect in app
                        .hits
                        .diff
                        .iter()
                        .map(|(r, _)| r)
                        .chain(app.hits.log.iter().map(|(r, _)| r))
                    {
                        assert_eq!(*rect, rect.intersect(app.layout.editor));
                    }
                }
            }
        }
    }
}

/// 预览工具栏的阴影和边框也不能跨过预览区域，或在区域隐藏后留下像素。
#[test]
fn floating_bars_stay_inside_their_preview() {
    for dark in [false, true] {
        let mut app = blank_app(dark);
        for (w, h) in [(0.0, 120.0), (96.0, 0.0), (28.0, 24.0), (96.0, 120.0)] {
            app.layout.editor = Rect::new(300.0, 80.0, w, h);
            assert_contained(
                &mut app,
                Rect::new(300.0, 80.0, w, h),
                1.25,
                view::tests::draw_floating_bar,
            );
        }
    }
}

/// Markdown 选区工具栏的卡片与阴影留在预览内。
#[test]
fn markdown_selection_tools_stay_inside_the_preview() {
    use winc_editor::PreviewMode;

    let dir = sample_folder("tiny-markdown-toolbar");
    let path = dir.join("README.md");
    std::fs::write(&path, "Intro\n\nselected words\n").unwrap();
    for dark in [false, true] {
        for width in [420.0, 820.0] {
            let mut app = app_with(&path, dark);
            app.set_preview_mode(PreviewMode::Preview);
            resize(&mut app, (width, 320.0));
            for _ in 0..4 {
                app.wait_for_markdown();
                paint(&mut app);
            }
            app.editor.active_mut().unwrap().selection = winc_core::Selection::new(7, 15);
            app.start_live_editing();
            app.offer_selection_toolbar();
            paint(&mut app);
            assert!(app.hits.selection_card.is_some());
            let area = app.layout.editor;
            assert_contained(&mut app, area, 1.25, view::tests::draw_editor);
            for (rect, _) in &app.hits.selection_tools {
                assert_eq!(*rect, rect.intersect(area));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 合并、磁盘冲突和提示条的固定宽度内容不能跨到侧栏或相邻窗格。
#[test]
fn notices_and_conflict_cards_stay_inside_their_area() {
    let dir = sample_folder("tiny-conflict-cards");
    let path = dir.join("short.rs");
    for dark in [false, true] {
        let mut app = app_with(&path, dark);
        app.config.git_enabled = Some(true);
        let index = app.editor.active.unwrap();
        let doc = &mut app.editor.documents[index];
        assert_eq!(
            doc.restore_unsaved(
                "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n",
                doc.disk_mtime(),
                winc_core::Selection::default(),
                true,
            ),
            Some(true)
        );
        assert_eq!(app.conflicts(index).len(), 1);
        app.notice.text = Some("A long message with an action".into());
        app.notice.action = Some(NoticeAction::GitOutput);
        for (w, h) in [(0.0, 120.0), (96.0, 0.0), (28.0, 24.0), (96.0, 240.0)] {
            app.layout.editor = Rect::new(300.0, 80.0, w, h);
            let area = app.overlay_area();
            assert_contained(&mut app, area, 1.25, view::tests::draw_notices);
            assert!(
                app.hits.conflict_card.is_some()
                    && app.hits.merge_card.is_some()
                    && app.hits.notice_card.is_some()
            );
            let (card, buttons) = app.hits.conflict_card.as_ref().unwrap();
            for rect in [
                *card,
                app.hits.merge_card.unwrap(),
                app.hits.notice_card.unwrap(),
                app.hits.notice_button.unwrap(),
            ]
            .into_iter()
            .chain(buttons.iter().map(|(r, _)| *r))
            .chain(app.hits.merge.iter().map(|(r, _)| *r))
            {
                assert_eq!(rect, rect.intersect(area));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 窗口缩小后，窗格工具栏不能盖到邻近窗格和侧栏。
#[test]
fn narrow_panes_keep_their_contents_inside() {
    for dark in [false, true] {
        let mut app = blank_app(dark);
        app.editor.show_log("Git log");
        app.run_action(Action::SplitRight);
        for size in [(480.0, 300.0), (340.0, 200.0)] {
            resize(&mut app, size);
            let area = app
                .panes
                .rects
                .iter()
                .find(|r| r.id == app.editor.panes.focused)
                .unwrap()
                .whole;
            assert_contained(&mut app, area, 1.25, view::tests::draw_pane);
        }
    }
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

/// 各种界面在很小的窗口里仍然工作。
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
