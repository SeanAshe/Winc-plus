//! 图片预览绘制的测试：图片位于预览区域内的部分，以及测试辅助。

use super::*;

#[test]
fn only_the_part_inside_the_preview_area_is_seen() {
    // 预览区域为物理像素 (200, 100) 到 (1000, 700)；图片 4000 × 3000，左上角在 (-500, 50)。
    let place = Placed {
        origin: (-500, 50),
        size: (4000, 3000),
        scale: 2.0,
        clip: Rect::new(100.0, 50.0, 400.0, 300.0),
    };
    assert_eq!(place.seen(), Some([700, 50, 1500, 650]));
    assert_eq!(
        place.rect((700, 50), (800, 600)),
        Rect::new(100.0, 50.0, 400.0, 300.0)
    );
    // 图片完全在预览区域内时整张可见；完全在区域外时没有可见的部分。
    let inside = Placed {
        origin: (300, 200),
        size: (100, 80),
        ..place
    };
    assert_eq!(inside.seen(), Some([0, 0, 100, 80]));
    let outside = Placed {
        origin: (1000, 200),
        ..inside
    };
    assert_eq!(outside.seen(), None);
}

#[test]
fn sizes_shrink_in_proportion_to_the_longest_edge_limit() {
    assert_eq!(bounded((1824, 1216), 8192), (1824, 1216));
    assert_eq!(bounded((8192, 100), 8192), (8192, 100));
    // 超过上限时按比例缩小，长边等于上限，短边至少为 1。
    assert_eq!(bounded((16_384, 8192), 8192), (8192, 4096));
    assert_eq!(bounded((3000, 12_288), 8192), (2000, 8192));
    assert_eq!(bounded((131_072, 1), 8192), (8192, 1));
    assert_eq!(bounded((7296, 4864), 1024), (1024, 682));
}

#[test]
fn a_raster_made_for_another_size_lands_on_the_same_part_of_the_image() {
    let place = Placed {
        origin: (100, 40),
        size: (4000, 3000),
        scale: 2.0,
        clip: Rect::new(0.0, 0.0, 1000.0, 1000.0),
    };
    let raster = |size, origin, (width, height)| Raster {
        pixels: Pixels {
            width,
            height,
            rgba: Vec::new(),
        },
        size,
        origin,
    };
    // 大小相同：逐像素对应。
    let exact = raster((4000, 3000), (500, 300), (800, 600));
    assert_eq!(place.part(&exact), Rect::new(300.0, 170.0, 400.0, 300.0));
    // 按一半的大小光栅化的同一部分落在相同的位置。
    let half = raster((2000, 1500), (250, 150), (400, 300));
    assert_eq!(place.part(&half), place.part(&exact));
}

#[test]
fn the_base_is_drawn_only_outside_the_sharp_raster() {
    let area = Rect::new(0.0, 0.0, 100.0, 80.0);
    assert_eq!(outside(area, None), [area]);
    assert_eq!(
        outside(area, Some(Rect::new(200.0, 0.0, 10.0, 10.0))),
        [area]
    );
    // 清晰的位图盖住整个区域时不绘制。
    assert!(outside(area, Some(Rect::new(-10.0, -10.0, 200.0, 200.0))).is_empty());
    // 位图在区域中间：上、下各一条，左、右各一条，互不重叠。
    let strips = outside(area, Some(Rect::new(20.0, 10.0, 50.0, 40.0)));
    assert_eq!(
        strips,
        [
            Rect::new(0.0, 0.0, 100.0, 10.0),
            Rect::new(0.0, 50.0, 100.0, 30.0),
            Rect::new(0.0, 10.0, 20.0, 40.0),
            Rect::new(70.0, 10.0, 30.0, 40.0),
        ]
    );
    // 位图贴着区域的左上角：只剩下方和右侧。
    let strips = outside(area, Some(Rect::new(-5.0, -5.0, 65.0, 45.0)));
    assert_eq!(
        strips,
        [
            Rect::new(0.0, 40.0, 100.0, 40.0),
            Rect::new(60.0, 0.0, 40.0, 40.0),
        ]
    );
}

impl PictureCache {
    /// 返回按显示大小光栅化的部分。
    ///
    /// # Panics
    ///
    /// 尚未光栅化时 panic。
    pub(crate) fn sharp(&self) -> &Raster {
        self.sharp.as_ref().expect("已光栅化")
    }

    /// 返回缓存的位图的宽度（物理像素）。
    pub(crate) fn width(&self) -> u32 {
        self.sharp().pixels.width
    }

    /// 返回缓存的位图的高度（物理像素）。
    pub(crate) fn height(&self) -> u32 {
        self.sharp().pixels.height
    }
}
