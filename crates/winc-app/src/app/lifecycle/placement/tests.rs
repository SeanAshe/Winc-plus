//! `placement` 的测试：显示器变化以后窗口的大小和位置，以及新窗口的层叠位置。

use super::*;

const MAIN: Screen = Screen {
    x: 0.0,
    y: 0.0,
    width: 3840.0,
    height: 2160.0,
    scale: 2.0,
    work: None,
};
const LEFT: Screen = Screen {
    x: -1920.0,
    y: 0.0,
    width: 1920.0,
    height: 1080.0,
    scale: 1.0,
    work: None,
};

fn frame(position: Option<(f64, f64)>, width: f64, height: f64, scale: f64) -> WindowFrame {
    WindowFrame {
        position,
        width,
        height,
        scale,
        maximized: false,
    }
}

#[test]
fn a_window_on_a_connected_screen_keeps_its_place() {
    let placed = place(&frame(Some((100.0, 80.0)), 900.0, 600.0, 2.0), &[MAIN]);
    assert_eq!(
        placed,
        Placement {
            width: 900.0,
            height: 600.0,
            position: Some((100.0, 80.0)),
        }
    );
    // 另一台显示器在左边，坐标是负的。
    let placed = place(
        &frame(Some((-800.0, 0.0)), 700.0, 500.0, 1.0),
        &[MAIN, LEFT],
    );
    assert_eq!(placed.position, Some((-800.0, 0.0)));
}

#[test]
fn a_window_on_a_removed_screen_gets_the_default_place() {
    let placed = place(&frame(Some((-800.0, 0.0)), 700.0, 500.0, 1.0), &[MAIN]);
    assert_eq!(placed.position, None);
    assert_eq!((placed.width, placed.height), (700.0, 500.0));
    // 没有显示器的信息时也不恢复位置。
    assert_eq!(
        place(&frame(Some((0.0, 0.0)), 700.0, 500.0, 1.0), &[]).position,
        None
    );
}

#[test]
fn a_window_must_stay_within_reach() {
    let at = |x: f64, y: f64| place(&frame(Some((x, y)), 800.0, 500.0, 1.0), &[LEFT]).position;
    // 留在显示器内的刚好 128 像素，还能拖回来。
    assert_eq!(at(-1920.0 - 672.0, 0.0), Some((-2592.0, 0.0)));
    assert_eq!(at(-128.0, 952.0), Some((-128.0, 952.0)));
    // 少一个像素、顶边跑到上面或者太靠下就够不着了。
    assert_eq!(at(-127.0, 0.0), None);
    assert_eq!(at(-1920.0 - 673.0, 0.0), None);
    assert_eq!(at(-1000.0, -1.0), None);
    assert_eq!(at(-1000.0, 953.0), None);
}

#[test]
fn a_window_fits_its_screen_and_keeps_the_minimum_size() {
    // 显示器的逻辑尺寸是 1920 × 1080。
    let placed = place(&frame(Some((0.0, 0.0)), 3000.0, 2000.0, 2.0), &[MAIN]);
    assert_eq!((placed.width, placed.height), (1920.0, 1080.0));
    let placed = place(&frame(None, 200.0, 100.0, 1.0), &[MAIN]);
    assert_eq!((placed.width, placed.height), (MIN_WIDTH, MIN_HEIGHT));
    assert_eq!(placed.position, None);
    // 没有显示器的信息时大小照旧。
    let placed = place(&frame(None, 3000.0, 2000.0, 1.0), &[]);
    assert_eq!((placed.width, placed.height), (3000.0, 2000.0));
}

#[test]
fn a_new_window_cascades_from_the_one_that_opened_it() {
    // 错开 28 逻辑像素，缩放比例为 2 时为 56 物理像素；错开后仍在显示器上，位置保留。
    let at = cascaded((100.0, 200.0), 2.0);
    assert_eq!(at, (156.0, 256.0));
    let placed = place(&frame(Some(at), 1280.0, 820.0, 2.0), &[MAIN, LEFT]);
    assert_eq!(placed.position, Some(at));
}

/// 回归 #95：从已最小化的窗口打开新窗口时，Windows 报告的位置为 (-32000, -32000)，错开后的位置
/// 不在任何显示器上，校验时不予采用，由系统决定。
#[test]
fn a_window_cascaded_from_a_minimized_one_is_not_placed_off_screen() {
    let at = cascaded((-32000.0, -32000.0), 2.0);
    assert_eq!(at, (-31944.0, -31944.0));
    let placed = place(&frame(Some(at), 1280.0, 820.0, 2.0), &[MAIN, LEFT]);
    assert_eq!(placed.position, None);
    assert_eq!((placed.width, placed.height), (1280.0, 820.0));
}

#[test]
fn a_window_covering_the_usable_area_is_neither_restored_nor_recorded() {
    // 2560 × 1440、缩放 1.5 的显示器，底部的任务栏占 72 物理像素。
    let screen = Screen {
        x: 0.0,
        y: 0.0,
        width: 2560.0,
        height: 1440.0,
        scale: 1.5,
        work: Some((2560.0, 1368.0)),
    };
    // 最大化窗口的矩形：四周各超出可用区域一圈边框。
    let placed = place(&frame(Some((-10.0, 0.0)), 1721.0, 927.0, 1.5), &[screen]);
    assert_eq!(
        (placed.width, placed.height, placed.position),
        (DEFAULT_WIDTH, DEFAULT_HEIGHT, None)
    );
    // 只在一个方向上占满可用区域（贴靠在半边的窗口）、或略小于可用区域的窗口仍然恢复。
    let half = place(&frame(Some((0.0, 0.0)), 853.0, 912.0, 1.5), &[screen]);
    assert_eq!((half.width, half.height), (853.0, 912.0));
    assert_eq!(half.position, Some((0.0, 0.0)));
    let large = place(&frame(Some((30.0, 30.0)), 1600.0, 900.0, 1.5), &[screen]);
    assert_eq!((large.width, large.height), (1600.0, 900.0));
    // 可用区域未知的系统不作此判断，只把大小限制在显示器以内。
    let unknown = Screen {
        work: None,
        ..screen
    };
    let placed = place(&frame(Some((-10.0, 0.0)), 1721.0, 927.0, 1.5), &[unknown]);
    assert_eq!((placed.width, placed.height), (2560.0 / 1.5, 927.0));
    // 记录窗口大小时使用同一判断，单位为物理像素。
    assert!(covers((2581.0, 1390.0), screen.work));
    assert!(!covers((1280.0, 1368.0), screen.work));
    assert!(!covers((2581.0, 1390.0), None));
}
