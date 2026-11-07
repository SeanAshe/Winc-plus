//! `frame_log` 的测试：未启用时不记录，启用时累计帧数和超过刷新间隔的帧数。

use super::*;

#[test]
fn frames_are_counted_only_when_enabled() {
    let ms = Duration::from_millis;
    let mut off = FrameLog {
        enabled: false,
        ..FrameLog::from_env()
    };
    off.record(ms(30), ms(16), (100, 100));
    assert!(off.times.is_empty());

    let mut on = FrameLog {
        enabled: true,
        ..FrameLog::from_env()
    };
    // 一帧在刷新间隔以内，一帧超过：只有后者计入超时帧。
    on.record(ms(5), ms(16), (100, 100));
    on.record(ms(30), ms(16), (100, 100));
    assert_eq!(on.times.len(), 2);
    assert_eq!(on.slow, 1);
}
