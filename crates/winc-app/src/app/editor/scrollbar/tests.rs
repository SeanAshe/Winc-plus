//! 滚动条的测试：过渡从当前值出发、滑块颜色按加深程度插值。

use super::*;

impl App {
    /// 返回区域 `area` 的滚动条此刻的不透明度，不改变记下的状态；仅用于测试。
    pub(crate) fn scrollbar_shown(&self, area: ScrollArea) -> f32 {
        let reveal = &self.scroll.bars.reveals[area as usize];
        if reveal.area.is_some() {
            reveal.opacity.value()
        } else {
            0.0
        }
    }
}

#[test]
fn tweens_ease_from_where_they_are() {
    let at = |ms| Some(Clock::frozen(Duration::from_millis(ms), SCROLLBAR_SHOW));
    let tween = |clock| Tween {
        from: 0.0,
        to: 1.0,
        clock,
    };
    assert_eq!(tween(None).value(), 1.0);
    assert_eq!(tween(at(0)).value(), 0.0);
    // 三次缓出：经过一半时间时已到 7/8。
    assert!((tween(at(75)).value() - 0.875).abs() < 1e-6);
    assert_eq!(tween(at(150)).value(), 1.0);
    assert!(tween(at(75)).running());
    assert!(!tween(at(150)).running());
}

#[test]
fn looks_mix_the_two_colors() {
    let normal = Color::hex(0x000000).alpha(0.2);
    let active = Color::hex(0x000000).alpha(0.4);
    let look = |opacity, heat| ThumbLook { opacity, heat }.color(normal, active).a;
    assert!((look(1.0, 0.0) - 0.2).abs() < 1e-6);
    assert!((look(1.0, 0.5) - 0.3).abs() < 1e-6);
    assert!((look(0.5, 1.0) - 0.2).abs() < 1e-6);
    assert_eq!(look(0.0, 1.0), 0.0);
}
