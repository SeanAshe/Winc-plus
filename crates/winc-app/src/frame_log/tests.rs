//! `frame_log` 的测试：未启用时不记录，启用时累计帧数和超过刷新间隔的帧数。

use super::*;

#[test]
fn frames_are_counted_only_when_enabled() {
    let ms = Duration::from_millis;
    let mut off = FrameLog {
        enabled: false,
        ..FrameLog::from_env()
    };
    off.record(ms(30), ms(16), (100, 100), 0);
    assert!(off.times.is_empty());

    let mut on = FrameLog {
        enabled: true,
        ..FrameLog::from_env()
    };
    // 一帧在刷新间隔以内，一帧超过：只有后者计入超时帧。
    on.record(ms(5), ms(16), (100, 100), 0);
    on.record(ms(30), ms(16), (100, 100), 0);
    assert_eq!(on.times.len(), 2);
    assert_eq!(on.slow, 1);
}

#[test]
fn pointer_moves_are_timed_until_the_next_frame() {
    let ms = Duration::from_millis;
    let mut log = FrameLog {
        enabled: true,
        ..FrameLog::from_env()
    };
    // 一帧之前的两次移动只记一次，从较早的一次算起；没有移动的帧不记间隔。
    log.pointer_moved();
    let first = log.moved;
    log.pointer_moved();
    assert_eq!(log.moved, first);
    assert_eq!(log.time_query(|| 7), 7);
    log.frame_skipped();
    log.record(ms(5), ms(16), (100, 100), 2);
    log.record(ms(5), ms(16), (100, 100), 0);
    assert_eq!((log.waits.len(), log.moved), (1, None));
    assert_eq!((log.queries.get().0, log.skipped, log.reclaimed), (1, 1, 2));

    // 未启用时都不记录，查询仍然执行。
    let mut off = FrameLog {
        enabled: false,
        ..FrameLog::from_env()
    };
    off.pointer_moved();
    off.frame_skipped();
    assert_eq!(off.time_query(|| 7), 7);
    assert_eq!((off.moved, off.skipped, off.queries.get().0), (None, 0, 0));
}
