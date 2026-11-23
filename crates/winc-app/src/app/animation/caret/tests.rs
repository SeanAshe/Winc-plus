//! 光标移动动画的测试：两条时间曲线、滑动、滚动时直接跳到目标，以及被打断的移动。

use std::time::Duration;

use winc_config::SmoothCaret;
use winc_editor::DocumentId;
use winc_render::Rect;

use super::*;

/// 创建一个不绘制界面的应用，并使用 `style`。
fn app(style: SmoothCaret) -> App {
    let mut app = App::new(
        Editor::default(),
        None,
        Bench::from_env(),
        Config::default(),
    );
    app.config.smooth_caret_movement = Some(style);
    app
}

/// 文档 `doc` 中字符偏移 `pos` 处的光标矩形。
fn target(doc: u64, pos: usize, x: f32, y: f32) -> CaretTarget {
    CaretTarget::new(DocumentId(doc), pos, Rect::new(x, y, 2.0, 18.0), false)
}

/// 把目标标记为随跳转的平滑滚动逐帧移动。
fn following(mut target: CaretTarget) -> CaretTarget {
    target.follow_scroll = true;
    target
}

/// 把动画时钟固定在已过 `at` 处：测试不依赖实际经过的时间。
fn freeze(app: &mut App, at: Duration) {
    app.motion.caret_motion.as_mut().expect("有动画").freeze(at);
}

#[test]
fn the_curves_start_and_end_at_the_caret_positions() {
    assert_eq!(ease(SmoothCaret::Off, 0.3), 1.0);

    for style in [SmoothCaret::Snappy, SmoothCaret::Gliding] {
        assert_eq!(ease(style, 0.0), 0.0);
        assert_eq!(ease(style, 1.0), 1.0);
        // 时间增加时进度不减少。
        let mut last = 0.0;
        for step in 0..=20 {
            let value = ease(style, step as f32 / 20.0);
            assert!(value >= last, "{style:?} 在 {step} 处后退");
            last = value;
        }
    }

    // Snappy 在开始时走过大半距离，Gliding 在时间过半时约走八成。
    assert!((ease(SmoothCaret::Snappy, 0.1) - 0.846).abs() < 0.001);
    assert!((ease(SmoothCaret::Gliding, 0.5) - 0.806).abs() < 0.001);
}

#[test]
fn a_move_glides_over_the_configured_duration() {
    let mut app = app(SmoothCaret::Snappy);
    let start = target(1, 0, 100.0, 20.0);
    let end = target(1, 5, 200.0, 120.0);
    assert!(same_spot(app.advance_caret(start), start.rect));
    assert!(!app.animating());

    // 移动的第一帧仍在起点，之后由绘制安排下一帧。
    assert!(same_spot(app.advance_caret(end), start.rect));
    assert!(app.animating());

    // 时间过半：按曲线走了大部分距离，但还没有到目标。
    freeze(&mut app, CARET_MOVE / 2);
    let half = app.advance_caret(end);
    let expected = ease(SmoothCaret::Snappy, 0.5);
    assert!((half.x - (start.rect.x + 100.0 * expected)).abs() < 0.01);
    assert!((half.y - (start.rect.y + 100.0 * expected)).abs() < 0.01);
    assert!(app.animating());

    // 到达时长后停在目标处，不再安排帧。
    freeze(&mut app, CARET_MOVE);
    assert!(same_spot(app.advance_caret(end), end.rect));
    assert!(!app.animating());
}

#[test]
fn scrolling_the_view_moves_the_caret_with_the_text() {
    let mut app = app(SmoothCaret::Gliding);
    let start = target(1, 0, 100.0, 20.0);
    app.advance_caret(start);

    // 光标没动、视口滚动：直接画在新的位置，不播放动画。
    let scrolled = target(1, 0, 100.0, 8.0);
    assert!(same_spot(app.advance_caret(scrolled), scrolled.rect));
    assert!(!app.animating());
}

#[test]
fn a_move_that_scrolls_the_view_still_animates() {
    let mut app = app(SmoothCaret::Gliding);
    let start = target(1, 0, 100.0, 300.0);
    app.advance_caret(start);

    // 光标换到了另一行，视口同时滚动（例如跳到视野之外的行）：仍从旧位置滑过去。
    let moved = target(1, 5, 200.0, 20.0);
    assert!(same_spot(app.advance_caret(moved), start.rect));
    assert!(app.animating());

    freeze(&mut app, CARET_MOVE / 2);
    let half = app.advance_caret(moved);
    assert!(half.y < start.rect.y && half.y > moved.rect.y);

    freeze(&mut app, CARET_MOVE);
    assert!(same_spot(app.advance_caret(moved), moved.rect));
}

#[test]
fn a_moving_target_is_followed_without_restarting() {
    let mut app = app(SmoothCaret::Gliding);
    let start = target(1, 0, 0.0, 0.0);
    let first = target(1, 1, 100.0, 0.0);
    let scrolled = following(target(1, 1, 100.0, 50.0));
    app.advance_caret(start);
    app.advance_caret(first);
    freeze(&mut app, CARET_MOVE / 2);
    app.advance_caret(first);

    // 跳转的平滑滚动把目标带到新位置：保持进度和时钟，不重新开始一次移动。
    app.advance_caret(scrolled);
    let motion = app.motion.caret_motion.unwrap();
    assert!(motion.moving);
    assert!(same_spot(motion.to, scrolled.rect));
    assert_eq!(motion.clock.duration(), CARET_MOVE);

    freeze(&mut app, CARET_MOVE);
    assert!(same_spot(app.advance_caret(scrolled), scrolled.rect));
    assert!(!app.animating());
}

#[test]
fn a_wheel_scroll_mid_flight_snaps_to_the_target() {
    let mut app = app(SmoothCaret::Gliding);
    let start = target(1, 0, 0.0, 0.0);
    let first = target(1, 1, 100.0, 0.0);
    // 滚轮滚动时目标同样会移动，但它不是这次移动引起的：直接画在新位置。
    let wheeled = target(1, 1, 100.0, 50.0);
    app.advance_caret(start);
    app.advance_caret(first);
    freeze(&mut app, CARET_MOVE / 2);
    app.advance_caret(first);

    assert!(same_spot(app.advance_caret(wheeled), wheeled.rect));
    assert!(!app.animating());
}

#[test]
fn a_new_document_snaps_to_its_caret() {
    let mut app = app(SmoothCaret::Snappy);
    app.advance_caret(target(1, 0, 100.0, 20.0));
    let other = target(2, 0, 300.0, 60.0);
    assert!(same_spot(app.advance_caret(other), other.rect));
    assert!(!app.animating());
}

#[test]
fn an_interrupted_move_continues_from_where_it_is_and_gets_shorter() {
    let mut app = app(SmoothCaret::Gliding);
    let start = target(1, 0, 0.0, 0.0);
    let first = target(1, 1, 100.0, 0.0);
    let second = target(1, 2, 100.0, 100.0);
    app.advance_caret(start);
    app.advance_caret(first);

    // 移动途中改到别处：这一帧仍在当前画面位置，不跳。
    freeze(&mut app, CARET_MOVE / 2);
    let painted = app.advance_caret(first);
    assert!(same_spot(app.advance_caret(second), painted));

    // 被打断的移动缩短为完整时长的八成。
    let motion = app.motion.caret_motion.unwrap();
    let duration = motion.clock.duration().as_secs_f32() * 1000.0;
    assert!((duration - 96.0).abs() < 0.01);
    assert!((motion.urgency - 0.8).abs() < 1e-6);

    // 新的时长走完后到达新目标。
    freeze(&mut app, Duration::from_millis(100));
    assert!(same_spot(app.advance_caret(second), second.rect));
    assert!(!app.animating());
}

#[test]
fn moves_in_quick_succession_stay_short_after_one_finishes() {
    let mut app = app(SmoothCaret::Gliding);
    let at = |pos: usize| target(1, pos, pos as f32 * 10.0, 0.0);
    app.advance_caret(at(0));
    app.advance_caret(at(1));
    freeze(&mut app, CARET_MOVE / 2);
    app.advance_caret(at(2));
    let urgency = |app: &App| app.motion.caret_motion.unwrap().urgency;
    assert!((urgency(&app) - 0.8).abs() < 1e-6);

    // 缩短了的移动已经走完，但离它开始还不到一次完整时长：下一次移动继续缩短，不回到完整时长。
    freeze(&mut app, Duration::from_millis(100));
    assert!(same_spot(app.advance_caret(at(2)), at(2).rect));
    assert!(!app.animating());
    app.advance_caret(at(3));
    assert!((urgency(&app) - 0.64).abs() < 1e-6);

    // 隔了一次完整时长之后再移动：恢复完整时长。
    freeze(&mut app, CARET_MOVE);
    app.advance_caret(at(3));
    app.advance_caret(at(4));
    assert!((urgency(&app) - 1.0).abs() < 1e-6);
    assert_eq!(
        app.motion.caret_motion.unwrap().clock.duration(),
        CARET_MOVE
    );
}

#[test]
fn a_move_that_stays_in_place_does_not_animate() {
    let mut app = app(SmoothCaret::Snappy);
    let before = target(1, 10, 100.0, 20.0);
    app.advance_caret(before);
    // 光标之前的文字变了：字符偏移不同，画面上的位置相同。
    let after = target(1, 14, 100.0, 20.0);
    assert!(same_spot(app.advance_caret(after), after.rect));
    assert!(!app.animating());
}

#[test]
fn without_animation_the_caret_jumps() {
    let mut app = app(SmoothCaret::Off);
    let start = target(1, 0, 100.0, 20.0);
    let end = target(1, 5, 200.0, 120.0);
    assert!(same_spot(app.advance_caret(start), start.rect));
    assert!(same_spot(app.advance_caret(end), end.rect));
    assert!(!app.animating());
    assert!(app.motion.caret_motion.is_none());
}

#[test]
fn hiding_the_caret_forgets_where_it_was() {
    let mut app = app(SmoothCaret::Snappy);
    app.advance_caret(target(1, 0, 100.0, 20.0));
    app.caret_hidden();
    assert!(app.motion.caret_motion.is_none());

    // 重新出现时停在原处，不从上一次的位置滑过来。
    let other = target(1, 8, 300.0, 60.0);
    assert!(same_spot(app.advance_caret(other), other.rect));
    assert!(!app.animating());
}
