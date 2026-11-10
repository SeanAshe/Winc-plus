//! 触摸输入的集成测试：在实际布局上验证命中测试、滚动、连击和长按菜单。
//!
//! 离屏渲染的缩放比例为 `SCALE`（两倍），触摸坐标按物理像素传入。

use super::*;
use winit::event::{DeviceId, Touch, TouchPhase};

fn touch(app: &mut App, id: u64, phase: TouchPhase, x: f32, y: f32) {
    app.on_touch(Touch {
        device_id: DeviceId::dummy(),
        phase,
        location: PhysicalPosition::new(f64::from(x * SCALE), f64::from(y * SCALE)),
        force: None,
        id,
    });
}

fn document() -> App {
    let mut app = blank_app(false);
    app.new_file();
    app.run(Command::Insert("hello world\n".repeat(400)));
    app.editor.active_mut().unwrap().selection = winc_core::Selection::point(0);
    app.editor.active_mut().unwrap().scroll_line = 0.0;
    paint(&mut app);
    app
}

#[test]
fn touch_taps_position_the_caret_at_high_dpi_and_double_taps_select() {
    let mut app = document();
    let x = app.layout.text.x + TEXT_PAD + app.cell * 3.0;
    let y = app.layout.text.y + app.fonts.line_height / 2.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    assert_eq!(app.editor.active().unwrap().selection.head, 0);
    touch(&mut app, 1, TouchPhase::Ended, x, y);
    let selected = app.editor.active().unwrap().selection;
    assert!(selected.is_empty());
    assert_eq!(selected.head, 3);
    touch(&mut app, 2, TouchPhase::Started, x, y);
    touch(&mut app, 2, TouchPhase::Ended, x, y);
    assert_eq!(
        app.editor.active().unwrap().selection,
        winc_core::Selection::new(0, 5)
    );
    assert_eq!(app.drag, Drag::None);
}

#[test]
fn double_tap_tolerates_finger_offset_but_mouse_keeps_tight_tolerance() {
    let mut app = document();
    let x = app.layout.text.x + TEXT_PAD + app.cell * 2.0;
    let y = app.layout.text.y + app.fonts.line_height / 2.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    touch(&mut app, 1, TouchPhase::Ended, x, y);
    // 第二次落指在两个方向上的偏移均超过鼠标的连击容差、小于 `TAP_SLOP`，且仍在同一个词内。
    let second = x + app.cell.min(10.0);
    touch(&mut app, 2, TouchPhase::Started, second, y + 6.0);
    touch(&mut app, 2, TouchPhase::Ended, second, y + 6.0);
    assert_eq!(
        app.editor.active().unwrap().selection,
        winc_core::Selection::new(0, 5)
    );
    // 鼠标点击仍按 4 逻辑像素的容差判断连击。
    app.last_click = None;
    assert_eq!(app.click_count(x, y), 1);
    assert_eq!(app.click_count(x + 6.0, y), 1);
}

#[test]
fn stale_touch_contact_stops_blocking_the_mouse() {
    let mut app = document();
    let text = app.layout.text;
    let (x, y) = (text.x + text.w / 2.0, text.y + text.h / 2.0);
    touch(&mut app, 1, TouchPhase::Started, x, y);
    assert!(
        app.touch_blocks_mouse(),
        "接触点未结束时应忽略由触摸提升的鼠标事件"
    );
    // 模拟抬手事件丢失：超过 `STALE` 没有新的触摸事件时接触点失效，鼠标恢复响应；
    // 随后到达的抬手事件不再定位光标。
    app.touch.last_event = Some(Instant::now() - Duration::from_secs(2));
    assert!(!app.touch_blocks_mouse());
    assert!(!app.touch.active());
    touch(&mut app, 1, TouchPhase::Ended, x, y);
    assert_eq!(
        app.editor.active().unwrap().selection,
        winc_core::Selection::point(0)
    );
}

#[test]
fn caption_controls_cover_top_bar_targets() {
    use crate::app::input::touch::WindowTouch;
    let verify = |app: &App| {
        let controls = app.caption_controls();
        let size = app.layout.window;
        for y in (0..TOP_BAR_END as usize).step_by(2) {
            for x in (0..size.w as usize).step_by(2) {
                let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                // 系统命中测试先判断窗口边缘、后判断控件（`WindowHitTest::hit`），边缘处无须检查。
                let window = app.touch_window_at(x, y);
                if matches!(window, Some(WindowTouch::Resize(_))) {
                    continue;
                }
                let caption = window == Some(WindowTouch::Move);
                let control = controls.iter().any(|r| r.contains(x, y));
                assert!(caption || control, "顶栏控件未列入客户区：({x}, {y})");
                assert!(!(caption && control), "顶栏空白处被列入客户区：({x}, {y})");
            }
        }
    };
    let mut app = document();
    app.window.controls = true;
    app.relayout();
    paint(&mut app);
    verify(&app);
    app.sidebar.visible = true;
    app.relayout();
    paint(&mut app);
    verify(&app);
    app.new_file();
    app.new_file();
    paint(&mut app);
    verify(&app);
    app.config.terminal_enabled = Some(true);
    app.terminal.open = true;
    app.terminal
        .tabs
        .push(crate::app::terminal::TermTab::new(1, app.terminal_size()));
    app.config.terminal_right = Some(true);
    app.relayout();
    paint(&mut app);
    verify(&app);
}

#[test]
fn touch_scrolls_by_logical_pixels_without_moving_the_selection() {
    let mut app = document();
    let text = app.layout.text;
    let x = text.x + text.w / 2.0;
    let y = text.y + text.h / 2.0;
    let selection = app.editor.active().unwrap().selection;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    touch(&mut app, 1, TouchPhase::Moved, x, y - 100.0);
    touch(&mut app, 1, TouchPhase::Ended, x, y - 100.0);
    let doc = app.editor.active().unwrap();
    assert!((doc.scroll_line * f64::from(app.fonts.line_height) - 100.0).abs() < 0.01);
    assert_eq!(doc.selection, selection);
    assert_eq!(app.drag, Drag::None);
    assert!(app.wheel.glide.is_none());
}

#[test]
fn touch_scroll_stays_in_the_panel_where_it_started() {
    let dir = sample_folder("touch-scroll-tree");
    for i in 0..80 {
        std::fs::write(dir.join(format!("file-{i:02}.txt")), "hello").unwrap();
    }
    let mut app = app_with(&dir, false);
    paint(&mut app);
    let tree = app.layout.tree;
    // 在文件树右侧（滚动条以左）落指，竖直滑动 100 逻辑像素的同时越过边界进入编辑区；竖直位移
    // 大于横向位移，按竖直方向滚动。
    let x = tree.right() - 40.0;
    let y = tree.y + tree.h / 2.0;
    assert!(!app.touch_drag_at(x, y));
    let editor_x = app.layout.editor.x + 10.0;
    assert!(editor_x - x < 100.0);
    touch(&mut app, 1, TouchPhase::Started, x, y);
    touch(&mut app, 1, TouchPhase::Moved, editor_x, y - 100.0);
    touch(&mut app, 1, TouchPhase::Ended, x, y - 100.0);
    assert_eq!(app.tree.scroll, 100.0);
    assert!(app.editor.documents.is_empty());
    assert!(app.tree.drag.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn touch_long_press_opens_a_menu_once_and_release_keeps_it_open() {
    let mut app = document();
    let x = app.layout.text.x + 100.0;
    let y = app.layout.text.y + 100.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    let due = app.touch_tick(Instant::now()).unwrap();
    assert!(app.popup().is_none());
    assert_eq!(app.touch_tick(due), None);
    assert!(app.popup().is_some());
    touch(&mut app, 1, TouchPhase::Ended, x, y);
    assert!(app.popup().is_some());
    assert_eq!(app.drag, Drag::None);
}

#[test]
fn cancelled_or_interrupted_touch_never_places_the_caret() {
    let mut app = document();
    let x = app.layout.text.x + 100.0;
    let y = app.layout.text.y + 100.0;
    for cancel in [true, false] {
        touch(&mut app, 1, TouchPhase::Started, x, y);
        if cancel {
            touch(&mut app, 1, TouchPhase::Cancelled, x, y);
        } else {
            app.cancel_touch();
        }
        touch(&mut app, 1, TouchPhase::Ended, x, y);
        assert_eq!(app.editor.active().unwrap().selection.head, 0);
        assert_eq!(app.drag, Drag::None);
        assert!(!app.touch.active());
    }
}

#[test]
fn touch_drags_the_scrollbar_and_cancellation_releases_it() {
    let mut app = document();
    let editor = app.layout.editor;
    let x = editor.right() - 4.0;
    let y = editor.y + editor.h / 2.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    assert!(matches!(app.drag, Drag::Scrollbar { .. }));
    let before = app.editor.active().unwrap().scroll_line;
    touch(&mut app, 1, TouchPhase::Moved, x, y + 50.0);
    assert!(app.editor.active().unwrap().scroll_line > before);
    touch(&mut app, 1, TouchPhase::Cancelled, x, y + 50.0);
    assert_eq!(app.drag, Drag::None);
    let after = app.editor.active().unwrap().scroll_line;
    app.on_mouse_move(x, y + 100.0);
    assert_eq!(app.editor.active().unwrap().scroll_line, after);
    touch(&mut app, 2, TouchPhase::Started, x, y);
    assert!(matches!(app.drag, Drag::Scrollbar { .. }));
    app.cancel_touch();
    assert_eq!(app.drag, Drag::None);
    assert_eq!(app.hover, Hover::None);
}

#[test]
fn touch_scrolls_settings_without_scrolling_the_document_below() {
    let mut app = document();
    app.open_settings();
    app.settings.motion = None;
    app.settings_show_page(SettingsPage::Keys);
    paint(&mut app);
    let body = app.settings_layout().body;
    let x = body.x + body.w / 2.0;
    let y = body.y + body.h / 2.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    touch(&mut app, 1, TouchPhase::Moved, x, y - 60.0);
    touch(&mut app, 1, TouchPhase::Ended, x, y - 60.0);
    assert_eq!(app.settings.scroll, 60.0);
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    assert_eq!(app.drag, Drag::None);
}

#[test]
fn touch_closes_a_tab_only_on_tap_not_swipe_or_cancel() {
    let mut app = blank_app(false);
    app.new_file();
    paint(&mut app);
    let close = app.hits.tabs[0].close;
    let x = close.x + close.w / 2.0;
    let y = close.y + close.h / 2.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    assert_eq!(app.editor.documents.len(), 1);
    touch(&mut app, 1, TouchPhase::Cancelled, x, y);
    assert_eq!(app.editor.documents.len(), 1);
    touch(&mut app, 2, TouchPhase::Started, x, y);
    touch(&mut app, 2, TouchPhase::Moved, x - 30.0, y);
    touch(&mut app, 2, TouchPhase::Ended, x, y);
    assert_eq!(app.editor.documents.len(), 1);
    touch(&mut app, 3, TouchPhase::Started, x, y);
    touch(&mut app, 3, TouchPhase::Ended, x, y);
    assert!(app.editor.documents.is_empty());
}

#[test]
fn additional_fingers_end_a_scrollbar_drag_without_starting_a_new_action() {
    let mut app = document();
    let editor = app.layout.editor;
    let x = editor.right() - 4.0;
    let y = editor.y + editor.h / 2.0;
    touch(&mut app, 1, TouchPhase::Started, x, y);
    assert!(matches!(app.drag, Drag::Scrollbar { .. }));
    touch(&mut app, 2, TouchPhase::Started, x - 100.0, y);
    assert_eq!(app.drag, Drag::None);
    let scroll = app.editor.active().unwrap().scroll_line;
    touch(&mut app, 1, TouchPhase::Ended, x, y);
    touch(&mut app, 2, TouchPhase::Moved, x - 100.0, y - 100.0);
    touch(&mut app, 2, TouchPhase::Ended, x - 100.0, y - 100.0);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll);
    assert!(app.popup().is_none());
    assert!(!app.touch.active());
}

#[test]
fn touch_title_bar_starts_on_contact_without_scrolling_or_long_press() {
    let mut app = document();
    app.window.controls = true;
    app.relayout();
    paint(&mut app);
    let x = app.layout.window.w / 2.0;
    let y = ISLAND_MARGIN + TOP_BAR / 2.0;
    assert_eq!(
        app.touch_window_at(x, y),
        Some(crate::app::input::touch::WindowTouch::Move)
    );
    touch(&mut app, 1, TouchPhase::Started, x, y);
    assert!(
        app.title_click.is_some(),
        "顶栏按下应在落指时记录，而非等到抬手"
    );
    assert!(
        app.touch.active(),
        "抬手前应保留接触点，以忽略由触摸提升的鼠标事件"
    );
    assert_eq!(
        app.touch_tick(Instant::now() + Duration::from_secs(1)),
        None
    );
    touch(&mut app, 1, TouchPhase::Moved, x, y - 100.0);
    touch(&mut app, 1, TouchPhase::Ended, x, y);
    assert!(!app.touch.active());
    assert!(app.title_click.is_some(), "抬手不应再记录一次顶栏按下");
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    assert_eq!(app.drag, Drag::None);
    // 第二次落指构成双击，并清除前一次按下的记录。
    touch(&mut app, 2, TouchPhase::Started, x, y);
    assert!(app.title_click.is_none());
}

#[test]
fn touch_window_edges_are_distinct_from_buttons_and_overlays() {
    use crate::app::input::touch::WindowTouch;
    use winit::window::ResizeDirection;
    let mut app = document();
    app.window.controls = true;
    app.relayout();
    paint(&mut app);
    assert_eq!(
        app.touch_window_at(1.0, 1.0),
        Some(WindowTouch::Resize(ResizeDirection::NorthWest))
    );
    touch(&mut app, 1, TouchPhase::Started, 1.0, 1.0);
    assert!(app.touch.active());
    touch(&mut app, 1, TouchPhase::Cancelled, 1.0, 1.0);
    let (close, _) = app.window_control_rects()[2];
    assert_eq!(
        app.touch_window_at(close.x + close.w / 2.0, close.y + close.h / 2.0),
        None
    );
    app.open_settings();
    assert_eq!(app.touch_window_at(app.layout.window.w / 2.0, 20.0), None);
    assert_eq!(app.touch_window_at(1.0, 1.0), None);
}

#[cfg(windows)]
#[test]
fn native_touch_caption_matches_application_targets() {
    use crate::app::input::touch::WindowTouch;
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCAPTION, HTCLIENT};
    let verify = |app: &App| {
        let native = app.window_hit_test();
        let size = app.layout.window;
        for y in (0..TOP_BAR_END as usize).step_by(2) {
            for x in (0..size.w as usize).step_by(2) {
                let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                let hit = native.hit(x, y, size.w, size.h, false);
                let target = app.touch_window_at(x, y);
                assert_eq!(
                    hit == HTCAPTION,
                    target == Some(WindowTouch::Move),
                    "标题栏判定不一致：({x}, {y})"
                );
                assert_eq!(
                    hit == HTCLIENT,
                    target.is_none(),
                    "客户区判定不一致：({x}, {y})"
                );
            }
        }
    };
    let mut app = document();
    app.window.controls = true;
    app.relayout();
    paint(&mut app);
    verify(&app);
    app.sidebar.visible = true;
    app.relayout();
    paint(&mut app);
    verify(&app);
    app.new_file();
    app.new_file();
    paint(&mut app);
    verify(&app);
    app.config.terminal_enabled = Some(true);
    app.terminal.open = true;
    app.terminal
        .tabs
        .push(crate::app::terminal::TermTab::new(1, app.terminal_size()));
    app.config.terminal_right = Some(true);
    app.relayout();
    paint(&mut app);
    verify(&app);
    app.open_settings();
    verify(&app);
}
