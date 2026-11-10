//! 动画收尾的测试：动画在最后一帧绘制之前结束，避免画面停在接近终点的中间帧。

use std::time::Duration;

use super::*;

#[test]
fn finished_animations_are_settled_before_the_last_frame() {
    let dir = sample_folder("animation-settle");
    let mut app = app_with(&dir, false);
    let at = |ms, duration| Clock::frozen(Duration::from_millis(ms), duration);

    // 动画尚未结束：保留状态，并继续安排下一帧。
    app.sidebar.motion = Some(SidebarMotion {
        from: 0.0,
        clock: at(100, SIDEBAR_ANIMATION),
    });
    app.picker.motion = Some(PickerMotion {
        from: 100.0,
        to: 50.0,
        clock: at(159, PICKER_ANIMATION),
    });
    app.settle_animations();
    assert!(app.sidebar.motion.is_some() && app.picker.motion.is_some());
    assert!(app.animating());

    // 动画已结束：在绘制前清除状态，使最后一帧画在终点位置，而不是停在接近终点的中间帧。
    app.sidebar.motion = Some(SidebarMotion {
        from: 0.0,
        clock: at(200, SIDEBAR_ANIMATION),
    });
    app.picker.motion = Some(PickerMotion {
        from: 100.0,
        to: 50.0,
        clock: at(160, PICKER_ANIMATION),
    });
    app.tree.motion = Some(TreeMotion {
        parent: 0,
        count: 1,
        expanding: true,
        removed: Vec::new(),
        clock: at(160, TREE_ANIMATION),
    });
    app.settle_animations();
    assert!(app.sidebar.motion.is_none() && app.picker.motion.is_none());
    assert!(app.tree.motion.is_none());
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
    // 像素是否接近颜色 `c`（每个通道相差不超过 2，容许舍入）。
    let near = |px: u32, c: Color| {
        let ch = |shift: u32, v: f32| ((px >> shift) & 0xff).abs_diff((v * 255.0).round() as u32);
        ch(16, c.r) <= 2 && ch(8, c.g) <= 2 && ch(0, c.b) <= 2
    };
    let (popup_bg, editor_bg) = (app.palette.popup_bg, app.palette.editor_bg);
    // 卡片里标题行右侧的一点：没有文字，只有卡片底色。
    let probe = (card.right() - 120.0, card.y + 4.0);
    let pixel = |app: &App, (x, y): (f32, f32)| {
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
        image.pixels[y * image.width as usize + x]
    };
    assert!(near(pixel(&app, probe), popup_bg));

    // 出现到一半：仍然半透明，并且从名字所在的一侧（上方）偏移了若干像素。
    let at = |ms, duration| Clock::frozen(Duration::from_millis(ms), duration);
    app.picker.places_in = Some(at(20, POPUP_IN));
    paint(&mut app);
    assert!(!near(pixel(&app, probe), popup_bg));
    assert!(app.animating());

    // 出现结束：清掉动画，完全不透明。
    app.picker.places_in = Some(at(200, POPUP_IN));
    app.settle_animations();
    assert!(app.picker.places_in.is_none());
    paint(&mut app);
    assert!(near(pixel(&app, probe), popup_bg));

    // 关闭后按关闭时刻的状态淡出：淡出一半时仍然可见，结束后清除。
    let ghost = PlacesGhost {
        places: app.picker.places.clone().unwrap(),
        hits: app.picker.hits.clone(),
        selected: app.picker.selected,
        scroll: app.picker.scroll,
        filter: String::new(),
        rect: app.layout.picker,
        clock: at(10, POPUP_OUT),
    };
    app.close_picker();
    app.picker.places_out = Some(ghost);
    paint(&mut app);
    assert!(!near(pixel(&app, probe), editor_bg));
    assert!(app.animating());
    app.picker.places_out.as_mut().unwrap().clock = at(150, POPUP_OUT);
    app.settle_animations();
    assert!(app.picker.places_out.is_none());
    paint(&mut app);
    assert!(near(pixel(&app, probe), editor_bg));

    // 选中一项并跳转时不淡出。
    app.open_path(&dir.join("lib/util.py"));
    app.select(5, 5);
    app.go_to_definition();
    app.picker_accept();
    assert!(app.picker.places_out.is_none());

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
