//! 图片预览的位置计算的测试：适应区域的大小、滚动范围、缩放时保持不动的位置。

use super::*;
use winc_preview::image::Pixels;

/// 返回一张 `width` × `height` 的位图；位置计算不读取像素。
fn raster(width: u32, height: u32) -> Picture {
    Picture {
        width,
        height,
        format: "PNG",
        file_size: 0,
        source: Source::Raster(Pixels {
            width,
            height,
            rgba: Vec::new(),
        }),
        transparent: false,
    }
}

const AREA: Rect = Rect {
    x: 100.0,
    y: 50.0,
    w: 448.0,
    h: 348.0,
};

#[test]
fn large_images_shrink_to_the_room_and_small_ones_keep_their_size() {
    // 可用区域为 400 × 300：800 × 300 的图片缩小一半，四周留白之外上下居中。
    let view = PictureView::default();
    let rect = view.rect(&raster(800, 300), AREA).unwrap();
    assert_eq!(rect, Rect::new(124.0, 149.0, 400.0, 150.0));
    // 小图按原大小居中，一个图片像素对应一个逻辑像素。
    let rect = view.rect(&raster(8, 8), AREA).unwrap();
    assert_eq!(rect, Rect::new(320.0, 220.0, 8.0, 8.0));
    // 区域容不下留白时不显示。
    assert!(
        view.rect(&raster(8, 8), Rect::new(0.0, 0.0, 48.0, 300.0))
            .is_none()
    );
}

#[test]
fn scrolling_stops_at_the_margins_of_the_content() {
    let picture = raster(800, 300);
    let fit = fitted(&picture, AREA).unwrap();
    let mut view = PictureView::default();
    // 适应区域时内容不超出区域，滚动不改变位置。
    view.scroll_by((-500.0, -500.0), fit, AREA);
    assert_eq!(view.place(fit, AREA), Rect::new(124.0, 149.0, 400.0, 150.0));

    // 2 倍时图片为 800 × 300：横向超出 400，竖直方向加留白后恰好不超出，仍然居中。
    view.zoom_to(2.0, (324.0, 224.0), (fit, AREA), None);
    assert_eq!(range(2.0, fit, AREA), (400.0, 0.0));
    assert_eq!(view.place(fit, AREA), Rect::new(-76.0, 74.0, 800.0, 300.0));
    // 滚动到两端时图片的边缘与区域的边缘相距留白的宽度。
    view.scroll_by((-5000.0, 0.0), fit, AREA);
    assert_eq!(view.place(fit, AREA).right(), AREA.right() - MARGIN);
    view.scroll_by((5000.0, 30.0), fit, AREA);
    assert_eq!(view.place(fit, AREA), Rect::new(124.0, 74.0, 800.0, 300.0));
}

#[test]
fn zooming_keeps_the_anchor_in_place_within_the_content() {
    let picture = raster(800, 600);
    let fit = fitted(&picture, AREA).unwrap();
    let mut view = PictureView::default();
    view.zoom_to(4.0, (324.0, 224.0), (fit, AREA), None);
    // 锚点处是图片中的哪一处（相对图片的宽高）。
    let anchor = (200.0, 300.0);
    let at = |view: &PictureView| {
        let rect = view.place(fit, AREA);
        ((anchor.0 - rect.x) / rect.w, (anchor.1 - rect.y) / rect.h)
    };
    let before = at(&view);
    for zoom in [6.0, 8.0, 12.0, 8.0] {
        view.zoom_to(zoom, anchor, (fit, AREA), None);
        let after = at(&view);
        assert!(
            (after.0 - before.0).abs() < 1e-4 && (after.1 - before.1).abs() < 1e-4,
            "{zoom}：{before:?} → {after:?}"
        );
    }
    // 缩小到内容不超出区域时无法保持锚点，图片回到居中的位置。
    view.zoom_to(1.0, anchor, (fit, AREA), None);
    assert_eq!(view.place(fit, AREA), Rect::new(124.0, 74.0, 400.0, 300.0));
    assert_eq!(view.scroll, (0.0, 0.0));
}

#[test]
fn a_zoom_transition_grows_from_the_shown_size_around_the_anchor() {
    use std::time::Duration;
    let picture = raster(800, 600);
    let fit = fitted(&picture, AREA).unwrap();
    let mut view = PictureView::default();
    view.zoom_to(4.0, (324.0, 224.0), (fit, AREA), None);
    let anchor = (200.0, 300.0);
    let at = |rect: Rect| ((anchor.0 - rect.x) / rect.w, (anchor.1 - rect.y) / rect.h);
    let before = view.place(fit, AREA);

    // 过渡的各个时刻：大小从 4 倍逐渐变为 8 倍，锚点处的内容不动；滚动位置已是结束后的值。
    let length = Duration::from_millis(200);
    let clock = |ms: u64| Some(Clock::stopped(length, Duration::from_millis(ms)));
    view.zoom_to(8.0, anchor, (fit, AREA), clock(0));
    assert_eq!(view.place(fit, AREA), before);
    let settled = placed(view.zoom, view.scroll, fit, AREA);
    assert_eq!(settled.w, fit.0 * 8.0);
    let mut last = before.w;
    for ms in [50, 100, 150] {
        view.zoom_motion.as_mut().unwrap().clock = clock(ms).unwrap();
        let rect = view.place(fit, AREA);
        assert!(rect.w > last && rect.w < settled.w, "{ms}：{}", rect.w);
        let (now, start) = (at(rect), at(before));
        assert!((now.0 - start.0).abs() < 1e-4 && (now.1 - start.1).abs() < 1e-4);
        last = rect.w;
    }
    view.zoom_motion.as_mut().unwrap().clock = clock(200).unwrap();
    assert_eq!(view.place(fit, AREA), settled);

    // 过渡中再次缩放：从这一帧显示的大小开始，不跳回上一次的终点。
    view.zoom_motion.as_mut().unwrap().clock = clock(100).unwrap();
    let shown = view.place(fit, AREA);
    view.zoom_to(4.0, anchor, (fit, AREA), clock(0));
    assert_eq!(view.place(fit, AREA), shown);
    // 滚动立即结束过渡。
    view.scroll_by((10.0, 0.0), fit, AREA);
    assert!(view.zoom_motion.is_none());
    assert_eq!(view.place(fit, AREA).w, fit.0 * 4.0);
}

impl App {
    /// 测试用：让刚完成的缩放从倍数 `from` 起、按 `clock` 的进度重新过渡，锚点为预览区域的中心。
    ///
    /// 测试中没有窗口，缩放立即到位；过渡期间的绘制用它检查。
    pub(crate) fn replay_picture_zoom(&mut self, from: f32, clock: Clock) {
        let (fit, area) = self.picture_fit().expect("正在显示图片");
        let anchor = (area.x + area.w / 2.0, area.y + area.h / 2.0);
        let view = &mut self.picture_view;
        let settled = placed(view.zoom, view.scroll, fit, area);
        view.zoom_motion = Some(ZoomMotion {
            from,
            clock,
            at: (
                (anchor.0 - settled.x) / settled.w,
                (anchor.1 - settled.y) / settled.h,
            ),
            anchor,
        });
    }
}
