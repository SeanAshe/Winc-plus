//! 动画收尾的测试：动画在最后一帧绘制之前结束，避免画面停在接近终点的中间帧。

use std::time::Duration;

use winc_config::SmoothCaret;

use super::*;
use crate::app::animation::{CARET_MOVE, ease};

#[test]
fn finished_animations_are_settled_before_the_last_frame() {
    let dir = sample_folder("animation-settle");
    let mut app = app_with(&dir, false);
    let at = |ms, duration| Clock::frozen(Duration::from_millis(ms), duration);

    // 动画尚未结束：保留状态，并继续安排下一帧。
    app.panels.sidebar.motion = Some(SidebarMotion {
        from: 0.0,
        clock: at(100, SIDEBAR_ANIMATION),
    });
    app.overlays.picker.motion = Some(PickerMotion {
        from: 100.0,
        to: 50.0,
        clock: at(159, PICKER_ANIMATION),
    });
    app.settle_animations();
    assert!(app.panels.sidebar.motion.is_some() && app.overlays.picker.motion.is_some());
    assert!(app.animating());

    // 动画已结束：在绘制前清除状态，使最后一帧画在终点位置，而不是停在接近终点的中间帧。
    app.panels.sidebar.motion = Some(SidebarMotion {
        from: 0.0,
        clock: at(200, SIDEBAR_ANIMATION),
    });
    app.overlays.picker.motion = Some(PickerMotion {
        from: 100.0,
        to: 50.0,
        clock: at(160, PICKER_ANIMATION),
    });
    app.panels.tree.motion = Some(TreeMotion {
        parent: 0,
        count: 1,
        expanding: true,
        removed: Vec::new(),
        clock: at(160, TREE_ANIMATION),
    });
    app.settle_animations();
    assert!(app.panels.sidebar.motion.is_none() && app.overlays.picker.motion.is_none());
    assert!(app.panels.tree.motion.is_none());
    assert!(!app.animating());

    // 没有窗口（测试环境）时不启动动画，界面直接到达终点。
    assert!(app.clock(TREE_ANIMATION).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn places_popup_slides_in_and_fades_out() {
    let dir = super::editor::navigation::navigation_folder("places-fade");
    // 深色主题下浮窗底色与编辑区底色差得多，淡入淡出时的像素容易区分。
    let mut app = app_with(&dir, true);
    app.open_path(&dir.join("lib/util.py"));
    app.select(5, 5);
    app.go_to_definition();
    paint(&mut app);
    let card = app.layout.picker;
    let (popup_bg, editor_bg) = (app.palette.popup_bg, app.palette.editor_bg);
    // 卡片里标题行右侧的一点：没有文字，只有卡片底色。
    let probe = (card.right() - 120.0, card.y + 4.0);
    assert!(near(pixel(&app, probe), popup_bg));

    // 出现到一半：仍然半透明，并且从名字所在的一侧（上方）偏移了若干像素。
    let at = |ms, duration| Clock::frozen(Duration::from_millis(ms), duration);
    app.overlays.picker.places_in = Some(at(20, POPUP_IN));
    paint(&mut app);
    assert!(!near(pixel(&app, probe), popup_bg));
    assert!(app.animating());

    // 出现结束：清掉动画，完全不透明。
    app.overlays.picker.places_in = Some(at(200, POPUP_IN));
    app.settle_animations();
    assert!(app.overlays.picker.places_in.is_none());
    paint(&mut app);
    assert!(near(pixel(&app, probe), popup_bg));

    // 关闭后按关闭时刻的状态淡出：淡出一半时仍然可见，结束后清除。
    let ghost = PlacesGhost {
        places: app.overlays.picker.places.clone().unwrap(),
        hits: app.overlays.picker.hits.clone(),
        selected: app.overlays.picker.selected,
        scroll: app.overlays.picker.scroll,
        filter: String::new(),
        rect: app.layout.picker,
        clock: at(10, POPUP_OUT),
    };
    app.close_picker();
    app.overlays.picker.places_out = Some(ghost);
    paint(&mut app);
    assert!(!near(pixel(&app, probe), editor_bg));
    assert!(app.animating());
    app.overlays.picker.places_out.as_mut().unwrap().clock = at(150, POPUP_OUT);
    app.settle_animations();
    assert!(app.overlays.picker.places_out.is_none());
    paint(&mut app);
    assert!(near(pixel(&app, probe), editor_bg));

    // 选中一项并跳转时不淡出。
    app.open_path(&dir.join("lib/util.py"));
    app.select(5, 5);
    app.go_to_definition();
    app.picker_accept();
    assert!(app.overlays.picker.places_out.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_covered_window_does_not_wake_up_to_blink_the_caret() {
    let dir = sample_folder("blink-covered");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.focus = Focus::Editor;
    app.window.focused = true;
    app.reset_blink();
    let wake = app.tick().expect("看得见时按闪烁的节奏唤醒");
    assert!(wake <= Instant::now() + BLINK);

    // 有焦点但被别的窗口整个挡住：不画，也不为闪烁醒来；光标停在亮着的状态。
    app.window.occluded = true;
    assert_eq!(app.tick(), None);
    assert!(app.blink.visible);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unfocused_window_hides_the_caret_and_shows_it_again_on_focus() {
    let dir = sample_folder("blink-unfocused");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.focus = Focus::Editor;
    app.window.focused = true;
    app.reset_blink();
    app.tick();
    assert!(app.blink.visible);

    // 失去焦点：不显示光标，也不为闪烁醒来。
    app.window.focused = false;
    assert_eq!(app.tick(), None);
    assert!(!app.blink.visible);

    // 回到前台：从亮着的一半重新开始闪烁。
    app.window.focused = true;
    app.reset_blink();
    app.tick();
    assert!(app.blink.visible);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 平滑光标移动：移动后的第一帧画在旧位置，途中按曲线前进，到时停在新位置。
#[test]
fn smooth_caret_movement_glides_between_positions() {
    let dir = sample_folder("caret-glide");
    std::fs::write(dir.join("blank.txt"), "a\n\n\n\n\n").unwrap();
    let mut app = app_with(&dir.join("blank.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    app.window.focused = true;
    app.blink.visible = true;
    paint(&mut app);

    // 光标在第 0 行行首；移到第 4 行的空行（字符偏移 5），两处之间隔着三行。
    let start = app.ime.cursor.expect("光标在第 0 行");
    app.select(5, 5);
    paint(&mut app);
    let target = app.ime.cursor.expect("光标在第 4 行");
    assert!(target.y > start.y + app.fonts.line_height * 3.0);

    let caret = app.palette.caret;
    // 第一帧仍在旧位置，新位置还没有光标。
    assert!(
        near(pixel(&app, caret_point(&app, start)), caret),
        "起点应画着光标"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, target)), caret),
        "目标处还不应有光标"
    );

    // 时间过半：Gliding 曲线走过约八成距离，光标画在两者之间。
    app.motion
        .caret_motion
        .as_mut()
        .unwrap()
        .freeze(CARET_MOVE / 2);
    paint(&mut app);
    let eased = ease(SmoothCaret::Gliding, 0.5);
    let middle = Rect::new(
        start.x,
        start.y + (target.y - start.y) * eased,
        start.w,
        start.h,
    );
    assert!(
        near(pixel(&app, caret_point(&app, middle)), caret),
        "中途应画在曲线位置"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, start)), caret),
        "起点应已没有光标"
    );

    // 动画结束：停在新位置，中途不再有光标。
    app.motion.caret_motion.as_mut().unwrap().freeze(CARET_MOVE);
    paint(&mut app);
    assert!(
        near(pixel(&app, caret_point(&app, target)), caret),
        "终点应画着光标"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, middle)), caret),
        "中途应已没有光标"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 方向键换行时播放动画：移动后的第一帧仍在旧行，到时换到新行。
#[test]
fn arrow_keys_animate_line_changes() {
    let dir = sample_folder("caret-lines");
    std::fs::write(dir.join("lines.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    let mut app = app_with(&dir.join("lines.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    app.window.focused = true;
    paint(&mut app);

    let start = app.ime.cursor.expect("光标在第一行");
    app.press_editor_key(winit::keyboard::NamedKey::ArrowDown);
    paint(&mut app);
    let target = app.ime.cursor.expect("光标在第二行");
    assert!(target.y > start.y);

    // 第一帧仍画在旧行，动画在运行。
    assert!(app.animating(), "换行后应播放动画");
    let caret = app.palette.caret;
    assert!(
        near(pixel(&app, caret_point(&app, start)), caret),
        "第一帧应还在旧行"
    );

    // 到时停在新行。
    app.motion.caret_motion.as_mut().unwrap().freeze(CARET_MOVE);
    paint(&mut app);
    assert!(!app.animating());
    assert!(
        near(pixel(&app, caret_point(&app, target)), caret),
        "到时应在新的行"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, start)), caret),
        "旧行应没有光标"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 跳到视野之外的行：视口平滑滚向它的途中光标不在屏幕上，动画状态保留，滚到目标后从原来的
/// 位置滑过去。用滚轮滚出视野的光标不保留，回到视野时直接出现。
#[test]
fn a_jump_out_of_view_keeps_the_caret_animation() {
    use crate::app::animation::SCROLL_GLIDE;
    use crate::app::scroll::ScrollGlide;

    let dir = sample_folder("caret-jump");
    let text: String = (0..120).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("long.txt"), text).unwrap();
    let mut app = app_with(&dir.join("long.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    app.window.focused = true;
    paint(&mut app);

    // 跳到很下面的行，平滑滚动刚开始：这一帧光标还不在屏幕上，动画状态保留。
    let doc = app.editor.active().unwrap();
    let (id, from) = (doc.id, doc.scroll_line);
    let target = doc.text().line_start(100);
    app.editor.active_mut().unwrap().selection = winc_core::Selection::point(target);
    app.ensure_caret_visible();
    app.scroll.glide = Some(ScrollGlide {
        doc: id,
        from,
        clock: Clock::frozen(Duration::ZERO, SCROLL_GLIDE),
    });
    let frame = app.begin_scroll_frame();
    paint(&mut app);
    app.end_scroll_frame(frame);
    assert!(app.ime.cursor.is_none(), "这一帧光标不在屏幕上");
    assert!(app.motion.caret_motion.is_some(), "滚向光标时保留动画状态");

    // 视口滚到目标：光标回到屏幕上，从原来的位置开始滑动。
    app.scroll.glide = None;
    paint(&mut app);
    assert!(app.animating(), "跳转后应播放动画");

    // 用滚轮把光标滚出视野：没有滚向光标的平滑滚动，动画状态清除。
    app.motion.caret_motion = None;
    paint(&mut app);
    assert!(app.motion.caret_motion.is_some());
    app.set_active_scroll(0.0);
    paint(&mut app);
    assert!(app.ime.cursor.is_none() && app.motion.caret_motion.is_none());
    // 这时移动光标并滚回去：直接出现在新位置，不从很久以前的位置滑过来。
    let next = app.editor.active().unwrap().text().line_start(101);
    app.editor.active_mut().unwrap().selection = winc_core::Selection::point(next);
    app.ensure_caret_visible();
    app.scroll.glide = None;
    paint(&mut app);
    assert!(app.ime.cursor.is_some() && !app.animating());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 鼠标单击换行时也播放动画。按下与松开之间常会绘制一帧；这一帧不能把动画状态清掉。
#[test]
fn mouse_clicks_animate_the_caret() {
    let dir = sample_folder("caret-click");
    std::fs::write(dir.join("lines.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    let mut app = app_with(&dir.join("lines.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    app.window.focused = true;
    paint(&mut app);

    // 量出第 2 行行首的位置，再回到第 0 行并把动画走完。
    app.select(8, 8);
    paint(&mut app);
    let target = app.ime.cursor.expect("光标在第 2 行");
    app.select(0, 0);
    paint(&mut app);
    app.motion.caret_motion.as_mut().unwrap().freeze(CARET_MOVE);
    paint(&mut app);
    let start = app.ime.cursor.expect("光标在第 0 行");
    assert!(target.y > start.y);

    // 按下、松开之间绘制一帧，与真实的事件顺序一致。
    let point = caret_point(&app, target);
    app.on_mouse_move(point.0, point.1);
    app.on_mouse_down(winit::event::MouseButton::Left);
    paint(&mut app);
    app.on_mouse_up(winit::event::MouseButton::Left);
    paint(&mut app);
    assert!(app.animating(), "单击后应播放动画");

    // 固定时钟：第一帧在起点，到时在点击处。
    app.motion
        .caret_motion
        .as_mut()
        .unwrap()
        .freeze(Duration::ZERO);
    paint(&mut app);
    let caret = app.palette.caret;
    assert!(
        near(pixel(&app, caret_point(&app, start)), caret),
        "单击后的第一帧应还在起点"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, target)), caret),
        "点击处还不应有光标"
    );

    app.motion.caret_motion.as_mut().unwrap().freeze(CARET_MOVE);
    paint(&mut app);
    assert!(
        near(pixel(&app, caret_point(&app, target)), caret),
        "到时应在点击处"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 关闭「动态效果」时，平滑光标也直接到位。
#[test]
fn the_animations_switch_disables_the_caret_slide() {
    let dir = sample_folder("caret-animations-off");
    std::fs::write(dir.join("lines.txt"), "one\ntwo\nthree\n").unwrap();
    let mut app = app_with(&dir.join("lines.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.config.ui_animations = Some(false);
    app.focus = Focus::Editor;
    paint(&mut app);

    let start = app.ime.cursor.expect("光标在第一行");
    app.press_editor_key(winit::keyboard::NamedKey::ArrowDown);
    paint(&mut app);
    assert!(!app.animating(), "关闭动态效果后不应播放动画");
    let target = app.ime.cursor.expect("光标在第二行");
    let caret = app.palette.caret;
    assert!(
        near(pixel(&app, caret_point(&app, target)), caret),
        "应直接画在新行"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, start)), caret),
        "旧行应没有光标"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 多光标时所有光标一起瞬移；只让主光标滑动会让其余光标显得不同步。
#[test]
fn multiple_carets_move_without_animation() {
    let dir = sample_folder("caret-multi");
    std::fs::write(dir.join("lines.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    let mut app = app_with(&dir.join("lines.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    paint(&mut app);

    let second = app.editor.active().unwrap().text().line_start(2);
    app.toggle_cursor(second);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().extra_selections().is_empty());

    app.press_editor_key(winit::keyboard::NamedKey::ArrowDown);
    paint(&mut app);
    assert!(!app.animating(), "多光标时不应播放动画");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 输入法组字时不播放动画：光标跟着组字走。
#[test]
fn ime_composition_does_not_animate_the_caret() {
    let dir = sample_folder("caret-ime");
    std::fs::write(dir.join("lines.txt"), "one\ntwo\nthree\n").unwrap();
    let mut app = app_with(&dir.join("lines.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    paint(&mut app);

    let pos = app.editor.active().unwrap().text().line_start(2);
    app.select(pos, pos);
    app.ime.preedit = "ni".to_owned();
    paint(&mut app);
    assert!(!app.animating(), "组字时不应播放动画");
    app.ime.preedit.clear();

    let _ = std::fs::remove_dir_all(&dir);
}

/// Shift 单击产生选区：和拖动一样直接到位，不播放动画。
#[test]
fn selecting_with_a_shift_click_does_not_animate() {
    let dir = sample_folder("caret-shift-click");
    std::fs::write(dir.join("lines.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    let mut app = app_with(&dir.join("lines.txt"), false);
    app.config.smooth_caret_movement = Some(SmoothCaret::Gliding);
    app.focus = Focus::Editor;
    paint(&mut app);

    // 量出第 2 行行首的位置，再回到第 0 行。
    app.select(8, 8);
    paint(&mut app);
    let target = app.ime.cursor.expect("光标在第 2 行");
    app.select(0, 0);
    paint(&mut app);
    app.motion.caret_motion.as_mut().unwrap().freeze(CARET_MOVE);
    paint(&mut app);
    let start = app.ime.cursor.expect("光标在第 0 行");

    // Shift 单击第 2 行：按下与松开之间绘制一帧。
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    let point = caret_point(&app, target);
    app.on_mouse_move(point.0, point.1);
    app.on_mouse_down(winit::event::MouseButton::Left);
    paint(&mut app);
    app.on_mouse_up(winit::event::MouseButton::Left);
    paint(&mut app);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();

    assert!(
        !app.editor.active().unwrap().selection.is_empty(),
        "Shift 单击应产生选区"
    );
    assert!(!app.animating(), "产生选区的点击应直接到位");
    let caret = app.palette.caret;
    assert!(
        near(pixel(&app, caret_point(&app, target)), caret),
        "应直接画在点击处"
    );
    assert!(
        !near(pixel(&app, caret_point(&app, start)), caret),
        "起点应没有光标"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
