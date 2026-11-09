//! 滚动条自动隐藏的测试：滚动与悬停时显示、切换文档后隐藏，以及 `Shift` 只影响滚轮的水平滚动。

use super::*;
use crate::app::ScrollArea;

#[test]
fn scrollbars_show_while_scrolling_or_hovered() {
    let dir = sample_folder("scrollbar-reveal");
    // 400 行远超一屏高度，竖直滚动条有可显示的滑块。
    let long: String = (0..400).map(|i| format!("line {i}\n")).collect();
    let path = dir.join("long.txt");
    std::fs::write(&path, long).unwrap();

    let mut app = app_with(&path, false);
    let editor = app.layout.editor;
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    paint(&mut app);

    // 打开文件后没有滚动过，滚动条不显示。
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 0.0);

    // 滚动后显示。
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -200.0,
    )));
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 1.0);

    // 切换到另一个文档不算滚动：新文档的滚动条不显示。
    app.open_path(&dir.join("a.txt"));
    paint(&mut app);
    std::fs::write(dir.join("b.txt"), "x\n".repeat(400)).unwrap();
    app.open_path(&dir.join("b.txt"));
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 0.0);

    // 鼠标移到编辑区右边缘的滚动条位置时显示，移开后先保持显示，随后淡出。
    app.on_mouse_move(editor.right() - 4.0, editor.y + editor.h / 2.0);
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 1.0);
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 1.0);

    // 内容不超过一屏的文件树没有滑块，鼠标移到文件树的右边缘也不显示。
    let tree = app.layout.tree;
    app.on_mouse_move(tree.right() - 4.0, tree.y + 10.0);
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Tree), 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_editor_scrolls_sideways_by_its_scrollbar_and_by_dragging_past_the_edge() {
    let dir = sample_folder("scrollbar-horizontal");
    // 每行宽 300 列，远超编辑区宽度，横向滚动有可滚动的范围。
    let long: String = (0..20)
        .map(|i| format!("line {i} {}\n", "x".repeat(300)))
        .collect();
    let path = dir.join("long.txt");
    std::fs::write(&path, long).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let track = app.text_hscroll_track();
    assert!(app.text_content_w > track.w);

    // 按住滑块拖到右端：滚动到最右边。
    let y = track.bottom() - 4.0;
    app.on_mouse_move(track.x + 8.0, y);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::EditorHorizontal));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(track.right(), y);
    app.on_mouse_up(MouseButton::Left);
    let max = app.text_content_w - track.w;
    assert!((app.editor.active().unwrap().scroll_x - max).abs() < 1.0);

    // 回到左端后拖选文字，鼠标停在右边缘之外：持续向右滚动，选区随之扩展。
    app.editor.active_mut().unwrap().scroll_x = 0.0;
    paint(&mut app);
    let t = app.layout.text;
    app.on_mouse_move(t.x + 40.0, t.y + 8.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(t.right() + 40.0, t.y + 8.0);
    assert!(app.drag_autoscrolling());
    let first = app.editor.active().unwrap().scroll_x;
    assert!(first > 0.0);
    app.drag_scrolled_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(50));
    app.continue_drag_scroll();
    let doc = app.editor.active().unwrap();
    assert!(doc.scroll_x > first, "{first} → {}", doc.scroll_x);
    let head = doc.selection.head;
    app.drag_scrolled_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(50));
    app.continue_drag_scroll();
    assert!(app.editor.active().unwrap().selection.head > head);

    // 鼠标回到编辑区内时停止滚动。
    app.on_mouse_move(t.x + 40.0, t.y + 8.0);
    assert!(!app.drag_autoscrolling());
    app.on_mouse_up(MouseButton::Left);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_minimap_takes_room_from_the_text_and_scrolls_the_editor() {
    let dir = sample_folder("minimap");
    let long: String = (0..2000)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect();
    let path = dir.join("long.rs");
    std::fs::write(&path, long).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);

    // 默认不显示；开启后占用文字区右侧的宽度，紧邻文字区。
    let before = app.layout.text.w;
    assert_eq!(app.layout.minimap.w, 0.0);
    app.settings_toggle(Setting::Minimap);
    paint(&mut app);
    let map = app.layout.minimap;
    assert!(map.w > 0.0);
    assert!(app.layout.text.w < before);
    assert_eq!(map.x, app.layout.text.right());

    // 点击滑块之外：滑块移到指针下方，靠近底边时滚动到文档末尾。
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;
    app.on_mouse_move(map.x + 8.0, map.bottom() - 1.0);
    assert_eq!(app.hover, Hover::Minimap);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(scroll(&app), app.max_scroll_line());
    paint(&mut app);

    // 按在滑块上时滑块不跳动；拖动时滑块随指针移动，拖到两端时分别滚动到文档的两端。
    let (_, slider, _) = app.minimap_view();
    let (at, y) = (scroll(&app), slider.y + 2.0);
    app.on_mouse_move(map.x + 8.0, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(scroll(&app), at);
    app.on_mouse_move(map.x + 8.0, y - 4.0);
    assert!(scroll(&app) < at);
    app.on_mouse_move(map.x + 8.0, map.bottom() + 40.0);
    assert_eq!(scroll(&app), app.max_scroll_line());
    app.on_mouse_move(map.x + 8.0, map.y - 40.0);
    assert_eq!(scroll(&app), 0.0);
    app.on_mouse_up(MouseButton::Left);

    // 滚轮在缩略图上仍然滚动编辑区。
    app.on_mouse_move(map.x + 8.0, map.y + 40.0);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    paint(&mut app);

    // 关闭后文字区恢复原宽度。
    app.settings_toggle(Setting::Minimap);
    assert_eq!(app.layout.minimap.w, 0.0);
    assert_eq!(app.layout.text.w, before);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 回归：自动换行时长行的每个显示行都绘制，后面不留空白。
#[test]
fn pressing_the_minimap_puts_the_slider_under_the_pointer_once() {
    let dir = sample_folder("minimap-press");
    let row = crate::theme::MINIMAP_ROW;
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;
    let center = |app: &App| {
        let (_, slider, _) = app.minimap_view();
        slider.y + slider.h / 2.0
    };
    // 远长于缩略图和短于缩略图的文档：滑块都位于指针下方。
    for (name, lines) in [("long.rs", 20_000), ("short.rs", 150)] {
        let text: String = (0..lines)
            .map(|i| format!("let row_{i} = {i};\n"))
            .collect();
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        let mut app = app_with(&file, true);
        app.settings_toggle(Setting::Minimap);
        paint(&mut app);
        let map = app.layout.minimap;
        let x = map.x + 8.0;

        let (_, slider, _) = app.minimap_view();
        let y = (slider.bottom() + 120.0)
            .min(map.y + map.h * 0.75)
            .max(slider.bottom() + 1.0);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        let pressed = scroll(&app);
        assert!(pressed > 0.0, "{name}");
        assert!(
            (center(&app) - y).abs() <= row,
            "{name}: {} {y}",
            center(&app)
        );

        // 指针不动：继续移动事件和松开都不改变位置。
        app.on_mouse_move(x, y);
        assert_eq!(scroll(&app), pressed, "{name}");
        app.on_mouse_up(MouseButton::Left);
        assert_eq!(scroll(&app), pressed, "{name}");
        paint(&mut app);
        assert_eq!(scroll(&app), pressed, "{name}");
    }
}

#[test]
fn dragging_the_minimap_slider_follows_the_pointer() {
    let dir = sample_folder("minimap-drag");
    let row = crate::theme::MINIMAP_ROW;
    let text: String = (0..20_000)
        .map(|i| format!("let row_{i} = {i};\n"))
        .collect();
    let file = dir.join("long.rs");
    std::fs::write(&file, text).unwrap();
    let mut app = app_with(&file, true);
    app.settings_toggle(Setting::Minimap);
    paint(&mut app);
    let map = app.layout.minimap;
    let x = map.x + 8.0;
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;

    // 按在滑块上时滑块不跳动，之后滑块的位移等于指针的位移；取整到整行造成的误差不超过一行。
    let (_, slider, _) = app.minimap_view();
    let y = slider.y + 4.0;
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(scroll(&app), 0.0);
    for delta in [1.0, 37.0, 240.0] {
        app.on_mouse_move(x, y + delta);
        let (_, moved, _) = app.minimap_view();
        assert!(
            (moved.y - slider.y - delta).abs() <= row,
            "{delta} {}",
            moved.y
        );
    }

    // 指针拖回按下处时，滚动位置恢复原值。
    app.on_mouse_move(x, y);
    assert_eq!(scroll(&app), 0.0);
    app.on_mouse_up(MouseButton::Left);
}

#[test]
fn wrapped_lines_fill_their_rows_in_the_minimap() {
    let dir = sample_folder("minimap-wrap");
    let path = dir.join("wrap.txt");
    std::fs::write(&path, format!("{}\nend\n", "word ".repeat(2000))).unwrap();
    let mut app = app_with(&path, false);
    app.settings_toggle(Setting::Minimap);
    app.settings_toggle(Setting::WordWrap);
    let mut filled = 0;
    let map = app.layout.minimap;
    paint(&mut app);
    let renderer = app.renderer.take().unwrap();
    let scale = renderer.scale_factor();
    let image = renderer.image().unwrap();
    let pixel = |x: f32, y: f32| {
        let (x, y) = ((x * scale) as usize, (y * scale) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let x = map.x + 2.0;
    let background = pixel(x, map.bottom() - 2.0);
    for row in 2..10 {
        let y = map.y + row as f32 * crate::theme::MINIMAP_ROW + 0.5;
        filled += usize::from(pixel(x, y) != background);
    }
    app.renderer = Some(renderer);
    assert!(filled >= 6, "{filled}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 大文件中按整行断开的长行占数千个显示行；
/// 滚动到这类行的中部时，缩略图同样绘制出该行靠后的段。
#[test]
fn the_minimap_shows_the_later_rows_of_long_wrapped_lines() {
    let dir = sample_folder("minimap-long-wrap");
    let path = dir.join("dump.sql");
    let values: Vec<String> = (0..9_000).map(|n| format!("({n},'订单 {n}')")).collect();
    let line = format!("INSERT INTO t VALUES {};\n", values.join(","));
    let count = winc_editor::LARGE_FILE as usize / line.len() + 2;
    std::fs::write(&path, line.repeat(count)).unwrap();
    let mut app = app_with(&path, false);
    app.settings_toggle(Setting::Minimap);
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(doc.is_large() && doc.is_wrapping());
    // 第 1 行的中部：缩略图的范围完全落在这一行之内。
    let rows = doc.rows_of(1);
    assert!(rows > 1_000, "{rows}");
    let scroll = (doc.row_of(1) + rows / 2) as f64;
    app.editor.active_mut().unwrap().scroll_line = scroll;
    paint(&mut app);

    let map = app.layout.minimap;
    let renderer = app.renderer.take().unwrap();
    let scale = renderer.scale_factor();
    let image = renderer.image().unwrap();
    let pixel = |x: f32, y: f32| {
        let (x, y) = ((x * scale) as usize, (y * scale) as usize);
        image.pixels[y * image.width as usize + x]
    };
    // 小块所在列与左侧留白的颜色不同，表示该显示行有内容。
    let filled = (10..110)
        .step_by(10)
        .filter(|&row| {
            let y = map.y + row as f32 * crate::theme::MINIMAP_ROW + 0.5;
            pixel(map.x + 6.0, y) != pixel(map.x + 1.0, y)
        })
        .count();
    app.renderer = Some(renderer);
    assert!(filled >= 8, "{filled}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_minimap() {
    let mut app = app_with(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/app.rs")
            .as_path(),
        false,
    );
    app.settings_toggle(Setting::Minimap);
    app.editor.active_mut().unwrap().scroll_line = 300.0;
    shoot(&mut app, "minimap");
    app.set_theme_choice(ThemeChoice::Dark);
    shoot(&mut app, "minimap-dark");
}

/// 测量跳转后第一帧的耗时，手动运行：
/// `cargo test --release -p winc-app measure_minimap_jump -- --ignored --nocapture`。
#[test]
#[ignore = "测量，手动运行"]
fn measure_minimap_jump() {
    let dir = sample_folder("minimap-jump");
    let line = |i: usize| {
        format!("    fn value_{i}(x: u32) -> String {{ format!(\"{{x}} and {i}\") }} // note\n")
    };
    for (name, count) in [("rope.rs", 200_000), ("large.rs", 1_000_000)] {
        let path = dir.join(name);
        std::fs::write(&path, (0..count).map(line).collect::<String>()).unwrap();
        for (on, sticky) in [(false, true), (true, true), (false, false)] {
            let mut app = app_with(&path, false);
            app.config.sticky_scroll = Some(sticky);
            if on {
                app.settings_toggle(Setting::Minimap);
            }
            paint(&mut app);
            for target in [0.9, 0.2, 0.6] {
                app.editor.active_mut().unwrap().scroll_line = app.max_scroll_line() * target;
                let mut frames = Vec::new();
                for _ in 0..12 {
                    let started = std::time::Instant::now();
                    paint(&mut app);
                    frames.push(started.elapsed().as_millis());
                }
                println!("{name} 缩略图{on} 粘滞{sticky} 跳到 {target}：{frames:?} ms");
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 测量缩略图的绘制耗时，手动运行：
/// `cargo test --release -p winc-app measure_minimap -- --ignored --nocapture`。
#[test]
#[ignore = "测量，手动运行"]
fn measure_minimap() {
    let dir = sample_folder("minimap-measure");
    let long: String = (0..100_000)
        .map(|i| {
            format!("    fn value_{i}(x: u32) -> String {{ format!(\"{{x}} and {i}\") }} // note\n")
        })
        .collect();
    let path = dir.join("long.rs");
    std::fs::write(&path, long).unwrap();
    let mut app = app_with(&path, false);
    let frames = 60;
    let run = |app: &mut App| {
        let started = std::time::Instant::now();
        for i in 0..frames {
            app.editor.active_mut().unwrap().scroll_line = (i * 500) as f64;
            paint(app);
        }
        started.elapsed() / frames
    };
    let off = run(&mut app);
    app.settings_toggle(Setting::Minimap);
    let on = run(&mut app);
    println!("每帧绘制：关闭 {off:?}，打开 {on:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shift_turns_only_mouse_wheel_scrolling_sideways() {
    let dir = sample_folder("scrollbar-shift");
    // 每行宽 300 列，远超编辑区宽度，横向滚动有可滚动的范围。
    let long: String = (0..400)
        .map(|i| format!("    line {i} {}\n", "x".repeat(300)))
        .collect();
    let path = dir.join("long.txt");
    std::fs::write(&path, long).unwrap();

    let mut app = app_with(&path, false);
    let editor = app.layout.editor;
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;

    // 触控板（含手指离开后的惯性滚动）：按住 `Shift` 仍然竖直滚动，不会横向移动。
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -200.0,
    )));
    let doc = app.editor.active().unwrap();
    assert!(doc.scroll_line > 0.0);
    assert_eq!(doc.scroll_x, 0.0);

    // 鼠标滚轮：按住 `Shift` 时横向滚动。
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    assert!(app.editor.active().unwrap().scroll_x > 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}
