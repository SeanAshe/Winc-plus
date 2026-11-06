//! `placement` 的测试：显示器变化以后窗口的大小和位置。

use super::*;

const MAIN: Screen = Screen {
    x: 0.0,
    y: 0.0,
    width: 3840.0,
    height: 2160.0,
    scale: 2.0,
};
const LEFT: Screen = Screen {
    x: -1920.0,
    y: 0.0,
    width: 1920.0,
    height: 1080.0,
    scale: 1.0,
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
