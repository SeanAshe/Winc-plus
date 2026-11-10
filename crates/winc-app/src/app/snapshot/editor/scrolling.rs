//! 编辑区滚动与焦点的测试：跳转时的平滑滚动、鼠标滚轮的过渡和每格的距离、查找把远处的匹配滚到
//! 正中、拖选停在编辑区外时的持续滚动，以及有无焦点时选区与当前行的绘制。

use std::time::Duration;

use super::*;
use crate::app::Clock;
use crate::app::animation::SCROLL_GLIDE;

/// 打开一个 480 行的文件并绘制一帧，返回界面与目录。
///
/// 文件内容是同构的：60 个 8 行的小函数，第 `i` 个从第 `i * 8` 行开始，每行都短于一屏的宽度。
/// 期望的行号可由序号直接算出；480 行远超一屏高度，跳转需要滚过若干屏才能到达。
fn long_file(name: &str) -> (App, PathBuf) {
    let dir = search_folder(name);
    let mut src = String::new();
    for i in 0..60 {
        src.push_str(&format!(
            "fn item_{i}(config: &Config) -> usize {{\n    if config.enabled {{\n        config.value + {i}\n    }} else {{\n        0\n    }}\n}}\n\n"
        ));
    }
    std::fs::write(dir.join("src/long.rs"), src).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/long.rs"));
    paint(&mut app);
    (app, dir)
}

fn scroll(app: &App) -> f64 {
    app.editor.active().unwrap().scroll_line
}

/// 切换标签的跳转直接到达目标：跳转时开始的平滑滚动被取消；留在同一标签时保留。
#[test]
fn opening_another_tab_does_not_glide() {
    fn jump(app: &mut App) {
        app.scroll_glide = Some(ScrollGlide {
            doc: app.editor.active().unwrap().id,
            from: 0.0,
            clock: Clock::frozen(SCROLL_GLIDE / 2, SCROLL_GLIDE),
        });
    }
    let (mut app, dir) = long_file("scroll-glide-tab");
    std::fs::write(dir.join("src/other.rs"), "fn other() {}\n").unwrap();

    app.open_path_then(&dir.join("src/long.rs"), jump);
    assert!(app.scroll_glide.is_some());

    app.open_path_then(&dir.join("src/other.rs"), jump);
    assert_eq!(app.editor.active().unwrap().name(), "other.rs");
    paint(&mut app);
    assert!(app.scroll_glide.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn far_jumps_glide_and_near_ones_do_not() {
    let (mut app, dir) = long_file("scroll-glide");

    // 跳转到第 200 行（居中）：距离超过一屏，滚动需要一段动画。
    // 没有窗口时不做动画，滚动位置直接等于目标。
    app.scroll_to_line(200, 0.5, false);
    assert!(app.scroll_glide.is_none());
    let target = scroll(&app);

    // 平滑滚动进行到一半：绘制的帧位于中间位置，绘制后恢复为目标，计算始终按目标进行。
    app.scroll_glide = Some(ScrollGlide {
        doc: app.editor.active().unwrap().id,
        from: 0.0,
        clock: Clock::frozen(SCROLL_GLIDE / 2, SCROLL_GLIDE),
    });
    let glide = app.begin_glide_frame();
    let shown = scroll(&app);
    assert_eq!(glide, Some(target));
    assert!(shown > 0.0 && shown < target, "{shown} / {target}");
    app.end_glide_frame(glide);
    assert_eq!(scroll(&app), target);
    assert!(app.animating());

    // 滚轮事件中止平滑滚动，滚动位置停在目标上。
    app.on_wheel(winit::event::MouseScrollDelta::LineDelta(0.0, -1.0));
    assert!(app.scroll_glide.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_centres_far_matches_and_keeps_near_ones_still() {
    let (mut app, dir) = long_file("find-centre");
    app.open_search(false);
    app.edit_field(|f| f.insert("item_40"));
    paint(&mut app);
    let visible = app.visible_lines() as f64;

    // 远处的匹配（`item_40` 在第 320 行）滚到编辑区正中，而不是刚刚露出的下边缘：按 0.4 到 0.6
    // 的屏内位置断言，留出预览留白与行高的余量。
    app.find_next(false);
    let line = 40 * 8;
    let row = scroll(&app);
    let at = (line as f64 - row) / visible;
    assert!((0.4..0.6).contains(&at), "匹配在 {at} 高处");

    // 匹配已可见时不滚动：`item_41` 在第 328 行，紧接上一处，仍在同一屏内。
    app.edit_field(|f| {
        f.select_all();
        f.insert("item_41")
    });
    let before = scroll(&app);
    app.find_next(false);
    assert_eq!(scroll(&app), before);

    // 首个可见位置设成第 328 行：下一处 `-> usize`（在 `item_41` 的开头）正好贴到编辑区顶部、
    // 被查找条盖住，视为不可见，编辑区向下滚一行使其露出。光标放在上一块（`item_40`）内，
    // 命中位置因此与光标分开。
    let doc = app.editor.active_mut().unwrap();
    doc.scroll_line = (41 * 8) as f64;
    doc.scroll_x = 0.0;
    app.edit_field(|f| {
        f.select_all();
        f.insert("-> usize")
    });
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(doc.text().line_start(40 * 8 + 4));
    app.find_next(false);
    let doc = app.editor.active().unwrap();
    let (found, _) = doc.text().pos_to_line_col(doc.selection.from());
    assert_eq!(found, 41 * 8);
    assert!(scroll(&app) < found as f64 - 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drag_select_keeps_scrolling_while_the_mouse_rests_outside() {
    let (mut app, dir) = long_file("drag-autoscroll");
    // 从第一行开始按住向下拖，鼠标停在编辑区下缘外 60 逻辑像素：开始持续滚动。
    let t = app.layout.text;
    app.on_mouse_move(t.x + 40.0, t.y + 40.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(t.x + 40.0, t.bottom() + 60.0);
    assert!(app.drag_autoscrolling());
    let first = scroll(&app);
    assert!(first > 0.0);

    // 鼠标不动但时间推移（距上次滚动 50 毫秒）：编辑区继续滚动，选区随之向下扩展。
    app.drag_scrolled_at = Some(std::time::Instant::now() - Duration::from_millis(50));
    app.continue_drag_scroll();
    let second = scroll(&app);
    assert!(second > first, "{first} → {second}");
    let head = app.editor.active().unwrap().selection.head;
    let (line, _) = app.editor.active().unwrap().text().pos_to_line_col(head);
    assert!(line as f64 >= second);

    // 鼠标回到编辑区内时停止滚动。
    app.on_mouse_move(t.x + 40.0, t.y + 100.0);
    assert!(!app.drag_autoscrolling());
    assert!(app.drag_scrolled_at.is_none());
    app.on_mouse_up(MouseButton::Left);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn selections_dim_without_focus_and_hide_the_current_line() {
    let (mut app, dir) = long_file("selection-focus");
    // 选中第 9 到第 11 行的中间一段：选区跨三行，能与当前行的高亮相区分。
    let doc = app.editor.active_mut().unwrap();
    let a = doc.text().line_start(9) + 4;
    let b = doc.text().line_start(11) + 12;
    doc.selection = winc_core::Selection::new(a, b);
    assert!(app.editor_focused());
    shoot(&mut app, "selection-focused");

    app.focus = Focus::Tree;
    assert!(!app.editor_focused());
    shoot(&mut app, "selection-unfocused");
    app.focus = Focus::Editor;
    app.window.focused = false;
    assert!(!app.editor_focused());
    // 窗口不在前台也按无焦点绘制；无焦点时用淡一层的 `selection_inactive`，当前行不高亮。
    app.window.focused = true;
    let p = app.palette;
    assert_ne!(p.selection_inactive, p.selection);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 触控板的滚动只按位移较大的方向生效：竖直滚动时附带的少量横向位移不使长行左右移动。
#[test]
fn a_trackpad_scrolls_along_the_dominant_axis_only() {
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta::PixelDelta;

    let dir = sample_folder("wheel-axis");
    let code = format!("let x = \"{}\";\n", "long ".repeat(400)).repeat(300);
    std::fs::write(dir.join("wide.rs"), code).unwrap();
    let mut app = app_with(&dir.join("wide.rs"), false);
    paint(&mut app);
    let text = app.layout.text;
    app.on_mouse_move(text.x + 100.0, text.y + 100.0);
    let at = |app: &App| {
        let doc = app.editor.active().unwrap();
        (doc.scroll_x, doc.scroll_line)
    };

    app.on_wheel(PixelDelta(PhysicalPosition::new(-9.0, -80.0)));
    let (x, line) = at(&app);
    assert!(x == 0.0 && line > 0.0, "{x} {line}");
    app.on_wheel(PixelDelta(PhysicalPosition::new(-80.0, -9.0)));
    assert!(at(&app).0 > 0.0);
    assert_eq!(at(&app).1, line);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_mouse_wheel_notch_glides_and_a_trackpad_follows_directly() {
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta::{LineDelta, PixelDelta};

    const LENGTH: Duration = Duration::from_millis(100);
    let dir = sample_folder("wheel-glide");
    let code = "let x = 1;\n".repeat(500);
    std::fs::write(dir.join("long.rs"), code).unwrap();
    let mut app = app_with(&dir.join("long.rs"), false);
    paint(&mut app);
    let text = app.layout.text;
    app.on_mouse_move(text.x + 100.0, text.y + 100.0);
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;
    let lh = app.fonts.line_height;

    // 只有鼠标滚轮的整格滚动使用过渡；不足一格、触控板和终端的滚动立即生效。
    assert!(app.wheel_glides(LineDelta(0.0, -1.0)));
    assert!(!app.wheel_glides(LineDelta(0.0, -0.25)));
    assert!(!app.wheel_glides(PixelDelta(PhysicalPosition::new(0.0, -40.0))));
    app.layout.terminal = text;
    assert!(!app.wheel_glides(LineDelta(0.0, -1.0)));
    app.layout.terminal = winc_render::Rect::default();

    // 向下滚动 30 行：时钟过半时位于缓出曲线的对应位置。
    let half = Clock::stopped(LENGTH, LENGTH / 2);
    let progress = half.progress();
    app.start_wheel_glide((0.0, -30.0 * lh), half);
    app.advance_wheel_glide();
    assert!((scroll(&app) - 30.0 * f64::from(progress)).abs() < 0.01);
    assert!(app.wheel.glide.is_some());
    assert!(app.animating());

    // 过渡途中再次滚动：未完成的距离计入新的总距离。
    app.start_wheel_glide((0.0, -10.0 * lh), Clock::stopped(LENGTH, LENGTH));
    app.advance_wheel_glide();
    assert!((scroll(&app) - 40.0).abs() < 0.01);
    assert!(app.wheel.glide.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn quick_wheel_notches_shorten_the_glide() {
    use crate::app::input::mouse::wheel_glide_length;

    let ms = Duration::from_millis;
    // 单独的一格和间隔较长的一格按完整时长过渡。
    assert_eq!(wheel_glide_length(None), SCROLL_GLIDE);
    assert_eq!(wheel_glide_length(Some(ms(400))), SCROLL_GLIDE);
    // 回归：每格都按完整时长重新计时时，快速转动滚轮会使画面越来越落后于滚轮。过渡时长取
    // 两格间隔的两倍，最短 50 毫秒。
    assert_eq!(wheel_glide_length(Some(ms(40))), ms(80));
    assert_eq!(wheel_glide_length(Some(ms(8))), ms(50));
}

#[test]
fn a_fast_spinning_wheel_keeps_the_view_close() {
    use crate::app::input::mouse::wheel_glide_length;

    let (mut app, dir) = long_file("wheel-spin");
    let text = app.layout.text;
    app.on_mouse_move(text.x + 100.0, text.y + 100.0);
    let notch = 3.0 * app.fonts.line_height;
    // 每 10 毫秒一格（滚轮快速转动）：下一格到来时，上一格的过渡进行到 10 毫秒处。
    let interval = Duration::from_millis(10);
    let length = wheel_glide_length(Some(interval));
    for _ in 0..20 {
        app.start_wheel_glide((0.0, -notch), Clock::stopped(length, interval));
        app.advance_wheel_glide();
    }
    // 画面落后于滚轮约一格；按完整时长重新计时时落后三格以上。
    let behind = 20.0 * 3.0 - scroll(&app);
    assert!(behind < 3.0 * 1.2, "{behind}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_mouse_wheel_notch_scrolls_the_platform_distance() {
    let (mut app, dir) = long_file("wheel-step");
    let text = app.layout.text;
    let lines = (app.cell * 3.0, app.fonts.line_height * 3.0);
    app.on_mouse_move(text.x + 20.0, text.y + 20.0);
    // 编辑区中：macOS 上为 3 行；Windows 上随系统设置变化，不检查；其他系统上为 120 像素
    // （`WHEEL_NOTCH`）。
    if cfg!(target_os = "macos") {
        assert_eq!(app.wheel_step(), lines);
    } else if !cfg!(windows) {
        assert_eq!(app.wheel_step(), (120.0, 120.0));
    }
    // 终端中各平台均为 3 行，行高按终端字号计算。
    app.config.terminal_size = Some(app.fonts.code + 4.0);
    app.apply_font_sizes();
    app.layout.terminal = text;
    let lines = (
        app.terminal_cell * 3.0,
        app.fonts.terminal_line_height * 3.0,
    );
    assert!(lines.1 > app.fonts.line_height * 3.0);
    assert_eq!(app.wheel_step(), lines);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回主修饰键：macOS 上为 `⌘`，其他系统上为 `Ctrl`。
fn command_key() -> winit::keyboard::ModifiersState {
    if cfg!(target_os = "macos") {
        winit::keyboard::ModifiersState::SUPER
    } else {
        winit::keyboard::ModifiersState::CONTROL
    }
}

#[test]
fn the_wheel_with_the_command_key_steps_the_font_size() {
    use winit::event::TouchPhase::{Moved, Started};
    use winit::keyboard::ModifiersState;

    let (mut app, dir) = long_file("wheel-font-size");
    let size = |app: &App| ViewFonts::of(&app.config).code;
    let notch =
        |app: &mut App, y: f32| app.on_wheel_event(MouseScrollDelta::LineDelta(0.0, y), Moved);
    let text = app.layout.text;
    app.mouse = (text.x + 40.0, text.y + 40.0);
    let start = size(&app);

    // 不按修饰键：滚动，字号不变。
    notch(&mut app, -1.0);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    assert_eq!(size(&app), start);

    // 只按主修饰键：向上放大，向下缩小，不滚动。
    let top = app.editor.active().unwrap().scroll_line;
    app.modifiers = command_key();
    notch(&mut app, 1.0);
    assert_eq!(size(&app), start + 1.0);
    notch(&mut app, -1.0);
    assert_eq!(size(&app), start);
    assert_eq!(app.editor.active().unwrap().scroll_line, top);
    // 同时按着其他修饰键、鼠标不在编辑区或终端上时照常滚动。
    app.modifiers = command_key() | ModifiersState::SHIFT;
    notch(&mut app, 1.0);
    app.modifiers = command_key();
    app.mouse = (app.layout.sidebar.x + 20.0, app.layout.sidebar.y + 80.0);
    notch(&mut app, 1.0);
    assert_eq!(size(&app), start);

    // 触控板：累计满一档才调整，每个事件至多一档。
    let pixels = |y: f32| MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, f64::from(y)));
    let step = crate::app::input::mouse::ZOOM_WHEEL_PIXELS * SCALE;
    app.mouse = (text.x + 40.0, text.y + 40.0);
    app.on_wheel_event(pixels(step / 2.0), Started);
    assert_eq!(size(&app), start);
    app.on_wheel_event(pixels(step * 5.0), Moved);
    assert_eq!(size(&app), start + 1.0);
    // 调整途中松开修饰键：余下的滚动量丢弃，既不调整也不滚动。
    let top = app.editor.active().unwrap().scroll_line;
    app.modifiers = ModifiersState::empty();
    app.on_wheel_event(pixels(-step * 5.0), Moved);
    assert_eq!(size(&app), start + 1.0);
    assert_eq!(app.editor.active().unwrap().scroll_line, top);
    // 滚动途中按下修饰键：仍是滚动。
    app.on_wheel_event(pixels(-step), Started);
    app.modifiers = command_key();
    app.on_wheel_event(pixels(-step * 5.0), Moved);
    assert_eq!(size(&app), start + 1.0);
    assert!(app.editor.active().unwrap().scroll_line > top);

    std::fs::remove_dir_all(&dir).unwrap();
}
