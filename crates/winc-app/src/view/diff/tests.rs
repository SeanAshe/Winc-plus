//! 差异绘制的测试：未改动内容展开或折叠的过渡中各显示行的位置。

use super::*;

#[test]
fn a_fold_transition_reveals_whole_rows_and_lifts_the_rows_after_them() {
    // 第 10 至 19 行（共 10 行，行高 20）正在展开，已露出三行半。
    let shift = FoldShift {
        rows: [10..20, 12..22],
        hidden: 130.0,
        line_height: 20.0,
    };
    // 变化的行之前不动，之后上移尚未露出的高度；两侧各按自己的显示行判断。
    assert_eq!(shift.of(LEFT, 9), Some(0.0));
    assert_eq!(shift.of(LEFT, 20), Some(130.0));
    assert_eq!(shift.of(RIGHT, 11), Some(0.0));
    assert_eq!(shift.of(RIGHT, 22), Some(130.0));
    // 完整露出的行原地绘制，只露出一部分的行和尚未露出的行不绘制。
    assert_eq!(shift.of(LEFT, 12), Some(0.0));
    assert_eq!(shift.of(LEFT, 13), None);
    assert_eq!(shift.of(LEFT, 19), None);
    // 底部需要多绘制的行数按上移的高度向上取整。
    assert_eq!(shift.extra_rows(), 7);

    // 过渡结束：没有上移，变化的行全部绘制。
    let settled = FoldShift {
        hidden: 0.0,
        ..shift
    };
    assert_eq!(settled.of(LEFT, 19), Some(0.0));
    assert_eq!(settled.of(LEFT, 20), Some(0.0));
    assert_eq!(settled.extra_rows(), 0);
}
