//! `touch` 的测试：点按、长按、滚动、多指取消与直接拖动的手势识别，不依赖窗口和真实时钟。

use super::*;

fn event(id: u64, phase: TouchPhase) -> Touch {
    Touch {
        device_id: DeviceId::dummy(),
        phase,
        location: PhysicalPosition::new(0.0, 0.0),
        force: None,
        id,
    }
}

#[test]
fn a_tap_waits_for_release_and_tolerates_jitter() {
    let mut state = TouchState::default();
    let now = Instant::now();
    assert_eq!(
        state.update(&event(1, TouchPhase::Started), (20.0, 30.0), now, false),
        None
    );
    assert_eq!(state.deadline(), Some(now + LONG_PRESS));
    assert_eq!(
        state.update(&event(1, TouchPhase::Moved), (23.0, 32.0), now, false),
        None
    );
    assert_eq!(
        state.update(&event(1, TouchPhase::Ended), (24.0, 33.0), now, false),
        Some(Action::Tap((20.0, 30.0)))
    );
    assert!(!state.active());
    assert_eq!(state.deadline(), None);
}

#[test]
fn scrolling_keeps_the_origin_and_includes_the_threshold_distance() {
    let mut state = TouchState::default();
    let now = Instant::now();
    state.update(&event(1, TouchPhase::Started), (100.0, 100.0), now, false);
    // 4 像素未越过 `SLOP`；越过时的增量从落指位置算起，包含此前未滚动的距离。
    state.update(&event(1, TouchPhase::Moved), (100.0, 96.0), now, false);
    assert_eq!(
        state.update(&event(1, TouchPhase::Moved), (100.0, 80.0), now, false),
        Some(Action::Scroll {
            origin: (100.0, 100.0),
            delta: (0.0, -20.0)
        })
    );
    assert_eq!(state.deadline(), None);
    assert_eq!(state.hold(now + LONG_PRESS), None);
    assert_eq!(
        state.update(&event(1, TouchPhase::Ended), (110.0, 70.0), now, false),
        Some(Action::Scroll {
            origin: (100.0, 100.0),
            delta: (10.0, -10.0)
        })
    );
}

#[test]
fn release_without_a_move_event_can_still_be_a_swipe() {
    let mut state = TouchState::default();
    let now = Instant::now();
    state.update(&event(1, TouchPhase::Started), (0.0, 0.0), now, false);
    assert_eq!(
        state.update(&event(1, TouchPhase::Ended), (0.0, -50.0), now, false),
        Some(Action::Scroll {
            origin: (0.0, 0.0),
            delta: (0.0, -50.0)
        })
    );
}

#[test]
fn a_long_press_fires_once_without_a_move_and_never_clicks_on_release() {
    let mut state = TouchState::default();
    let now = Instant::now();
    state.update(&event(1, TouchPhase::Started), (20.0, 30.0), now, false);
    assert_eq!(
        state.hold(now + LONG_PRESS - Duration::from_millis(1)),
        None
    );
    assert_eq!(
        state.hold(now + LONG_PRESS),
        Some(Action::Context((20.0, 30.0)))
    );
    assert_eq!(state.hold(now + LONG_PRESS), None);
    assert_eq!(
        state.update(
            &event(1, TouchPhase::Ended),
            (20.0, 30.0),
            now + LONG_PRESS,
            false
        ),
        None
    );
}

#[test]
fn release_after_the_deadline_does_not_depend_on_timer_order() {
    let mut state = TouchState::default();
    let now = Instant::now();
    state.update(&event(1, TouchPhase::Started), (20.0, 30.0), now, false);
    assert_eq!(
        state.update(
            &event(1, TouchPhase::Ended),
            (20.0, 30.0),
            now + LONG_PRESS,
            false
        ),
        Some(Action::Context((20.0, 30.0)))
    );
}

#[test]
fn cancelled_contacts_and_late_events_do_not_click() {
    let mut state = TouchState::default();
    let now = Instant::now();
    state.update(&event(1, TouchPhase::Started), (0.0, 0.0), now, false);
    assert_eq!(
        state.update(&event(1, TouchPhase::Cancelled), (0.0, 0.0), now, false),
        None
    );
    assert_eq!(
        state.update(&event(1, TouchPhase::Ended), (0.0, 0.0), now, false),
        None
    );
    assert_eq!(
        state.update(&event(2, TouchPhase::Moved), (0.0, 0.0), now, false),
        None
    );
    assert_eq!(state.hold(now + LONG_PRESS), None);
    assert!(!state.active());
}

#[test]
fn multiple_contacts_cancel_until_every_finger_is_up() {
    let mut state = TouchState::default();
    let now = Instant::now();
    for id in [1, 2] {
        state.update(&event(id, TouchPhase::Started), (0.0, 0.0), now, false);
    }
    assert_eq!(state.deadline(), None);
    state.update(&event(1, TouchPhase::Ended), (0.0, 0.0), now, false);
    assert!(state.active());
    assert_eq!(
        state.update(&event(2, TouchPhase::Moved), (0.0, -50.0), now, false),
        None
    );
    assert_eq!(
        state.update(&event(2, TouchPhase::Ended), (0.0, -50.0), now, false),
        None
    );
    state.update(&event(3, TouchPhase::Started), (20.0, 30.0), now, false);
    assert_eq!(
        state.update(&event(3, TouchPhase::Ended), (20.0, 30.0), now, false),
        Some(Action::Tap((20.0, 30.0)))
    );
}

#[test]
fn direct_drags_release_and_cancel_without_turning_into_taps() {
    let mut state = TouchState::default();
    let now = Instant::now();
    assert_eq!(
        state.update(&event(1, TouchPhase::Started), (10.0, 20.0), now, true),
        Some(Action::Press((10.0, 20.0)))
    );
    assert_eq!(state.deadline(), None);
    assert_eq!(
        state.update(&event(1, TouchPhase::Moved), (10.0, 25.0), now, false),
        Some(Action::Move((10.0, 25.0)))
    );
    assert_eq!(
        state.update(&event(1, TouchPhase::Ended), (10.0, 30.0), now, false),
        Some(Action::Release((10.0, 30.0)))
    );
    state.update(&event(2, TouchPhase::Started), (10.0, 20.0), now, true);
    assert_eq!(
        state.update(&event(3, TouchPhase::Started), (20.0, 20.0), now, true),
        Some(Action::CancelDrag)
    );
    assert_eq!(
        state.update(&event(2, TouchPhase::Ended), (10.0, 20.0), now, false),
        None
    );
}

#[test]
fn duplicate_starts_do_not_reset_a_gesture() {
    let mut state = TouchState::default();
    let now = Instant::now();
    state.update(&event(1, TouchPhase::Started), (20.0, 30.0), now, false);
    state.update(
        &event(1, TouchPhase::Started),
        (30.0, 40.0),
        now + LONG_PRESS,
        false,
    );
    assert_eq!(
        state.hold(now + LONG_PRESS),
        Some(Action::Context((20.0, 30.0)))
    );
}
