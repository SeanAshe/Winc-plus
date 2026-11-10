//! 动画的测试：缓出曲线、模块显隐动画中途反向、消失时先淡出内容再收起宽度。

use super::*;

impl Clock {
    /// 创建已用时间固定为 `at` 的时钟，仅用于测试。
    pub(in crate::app) fn frozen(at: Duration, duration: Duration) -> Clock {
        Clock {
            frozen: Some(at),
            ..Clock::new(duration)
        }
    }
}

#[test]
fn clocks_ease_out_and_stop_at_the_end() {
    let at = |ms| Clock::frozen(Duration::from_millis(ms), TREE_ANIMATION);
    assert_eq!(at(0).progress(), 0.0);
    assert!(!at(0).done());
    // 先快后慢：经过一半时间时进度已达 7/8。
    assert!((at(80).progress() - 0.875).abs() < 1e-6);
    assert_eq!(at(160).progress(), 1.0);
    assert!(at(160).done());
    assert_eq!(at(500).progress(), 1.0);
    assert_eq!(at(80).between(10.0, 20.0), 18.75);
    assert_eq!(at(80).between(20.0, 10.0), 11.25);
}

#[test]
fn presence_fades_both_ways_and_turns_back_midway() {
    let half = || Clock::frozen(MODULE_FADE / 2, MODULE_FADE);
    let mut p = Presence::default();
    // 没比过上一帧时按这一帧该不该有画。
    assert_eq!(p.share(true), 1.0);
    assert_eq!(p.share(false), 0.0);

    // 出现到一半：缓出曲线走过一半时间是 7/8；不管这一帧问的是什么都按动画走。
    p.track(true, Some(half()));
    assert!(p.moving());
    assert!((p.share(true) - 0.875).abs() < 1e-6);
    assert!((p.share(false) - 0.875).abs() < 1e-6);

    // 出现途中转为消失：内容先淡出，宽度保持 7/8，不跳回 1。
    p.track(false, Some(Clock::frozen(EXIT_FADE / 2, MODULE_FADE)));
    assert!((p.share(false) - 0.875).abs() < 1e-6);
    let opacity = p.opacity(false);
    assert!(opacity > 0.0 && opacity < reveal(0.875), "{opacity}");

    // 内容淡出结束后宽度才开始收起，收起途中内容不显示。
    let mut p = Presence::default();
    p.track(true, None);
    p.track(
        false,
        Some(Clock::frozen(EXIT_FADE + MODULE_FADE / 2, MODULE_FADE)),
    );
    assert!((p.share(false) - 0.125).abs() < 1e-6);
    assert_eq!(p.opacity(false), 0.0);

    // 走完以后下一次比较时清掉动画；没有时钟（没有窗口、动态效果关闭）时直接到终点。
    let mut p = Presence::default();
    p.track(true, Some(Clock::frozen(MODULE_FADE, MODULE_FADE)));
    assert!(p.moving());
    p.track(true, None);
    assert!(!p.moving());
    p.track(false, None);
    assert!(!p.moving());
    assert_eq!(p.share(false), 0.0);
}
