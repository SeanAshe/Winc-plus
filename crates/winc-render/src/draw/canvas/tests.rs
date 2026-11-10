//! `Canvas` 的测试：混合与覆盖率的取整、圆角、阴影、描边的裁剪，
//! 以及跳过被不透明卡片覆盖的像素。

use super::*;

const ALL: IRect = IRect {
    x0: i32::MIN,
    y0: i32::MIN,
    x1: i32::MAX,
    y1: i32::MAX,
};

fn canvas(px: &mut [u32], w: usize, h: usize) -> Canvas<'_> {
    Canvas {
        px,
        width: w,
        height: h,
        stride: w,
    }
}

#[test]
fn composite_fades_and_scales_the_whole_layer() {
    // 在黑底上以不透明度 0.5 叠加整块白色图层：每个像素都是相同的中灰，没有任何位置透出更多底色。
    let mut px = vec![0xff00_0000; 8 * 8];
    let mut c = canvas(&mut px, 8, 8);
    let area = IRect::new(0, 0, 8, 8);
    let layer = vec![0xffff_ffff; 64];
    c.blend_layer(area, &layer, 0.5, 1.0, 0.0);
    assert!(px.iter().all(|&p| p == px[0]), "{px:x?}");
    assert!(((px[0] >> 16) & 0xff).abs_diff(128) <= 1, "{:x}", px[0]);

    // 以中心缩小到 0.5 倍：中间仍为白色，四周空出的区域保持原有的黑色。
    let mut px = vec![0xff00_0000; 8 * 8];
    let mut c = canvas(&mut px, 8, 8);
    let area = IRect::new(0, 0, 8, 8);
    let layer = vec![0xffff_ffff; 64];
    c.blend_layer(area, &layer, 1.0, 0.5, 0.0);
    assert_eq!(px[0], 0xff00_0000);
    assert_eq!(px[3 * 8 + 3], 0xffff_ffff);
    assert_eq!(px[7 * 8 + 7], 0xff00_0000);

    // 超出画布的部分先被裁剪：面积和像素数都按裁剪后的 2 × 2 计。
    let mut px = vec![0xff00_0000; 4];
    let c = canvas(&mut px, 2, 2);
    let (area, under) = c.copy(IRect::new(-5, -5, 20, 20));
    assert_eq!((area, under.len()), (IRect::new(0, 0, 2, 2), 4));
}

#[test]
fn blend_endpoints() {
    assert_eq!(blend(0xff00_0000, 0xffff_ffff, 255), 0xffff_ffff);
    assert_eq!(blend(0xff12_3456, 0xffff_ffff, 0), 0xff12_3456);
    assert_eq!(blend(0xff12_3456, 0x00ff_ffff, 255), 0xff12_3456);

    // 不透明度 0x80 的白色叠加在黑色上，结果约为中灰（与 128 相差不超过 1）。
    let mid = blend(0xff00_0000, 0x80ff_ffff, 255);
    assert!(((mid >> 16) & 0xff).abs_diff(128) <= 1);
}

#[test]
fn unblend_recovers_what_was_painted() {
    for src in [
        0xff7d_8ba3,
        0x737d_8ba3,
        0x80ff_ffff,
        0x2000_0000,
        0x00ff_0000,
    ] {
        let a = src >> 24;
        let pixel = unblend(blend(0xff00_0000, src, 255), blend(0xffff_ffff, src, 255));
        assert_eq!(pixel >> 24, a, "{src:08x}");
        for shift in [0, 8, 16] {
            let want = div255(((src >> shift) & 0xff) * a);
            assert!(((pixel >> shift) & 0xff).abs_diff(want) <= 1, "{src:08x}");
        }
    }
}

#[test]
fn mix_matches_per_channel() {
    // 三个通道取不同的值：红通道由 `s` 决定、绿通道由 `255 - s` 决定、蓝通道由 `d` 决定，
    // 三个通道合计覆盖 `(src, dst, sa)` 的全部组合。
    for sa in 0..=255 {
        for s in 0..=255 {
            for d in 0..=255 {
                let src = s << 16 | (255 - s) << 8 | s;
                let dst = 0xff00_0000 | d << 16 | d << 8 | (255 - d);
                let want = |shift: u32| {
                    div255(((src >> shift) & 0xff) * sa + ((dst >> shift) & 0xff) * (255 - sa))
                        << shift
                };
                let want = 0xff00_0000 | want(16) | want(8) | want(0);
                assert_eq!(
                    mix(dst, src, sa),
                    want,
                    "src={src:06x} dst={dst:08x} sa={sa}"
                );
            }
        }
    }
    // 按 0 叠加不改变不透明的 `dst`，按 255 叠加得到 `src` 的颜色。
    assert_eq!(mix(0xff12_3456, 0x00ab_cdef, 0), 0xff12_3456);
    assert_eq!(mix(0xff12_3456, 0x00ab_cdef, 255), 0xffab_cdef);
}

#[test]
fn lerp_matches_per_channel() {
    let per_channel = |d: u32, s: u32, a: u32| {
        let ch = |shift: u32| {
            let (d, s) = ((d >> shift) & 0xff, (s >> shift) & 0xff);
            ((d * (256 - a) + s * a) >> 8) << shift
        };
        0xff00_0000 | ch(16) | ch(8) | ch(0)
    };
    let values = [0x00u32, 0x01, 0x7f, 0x80, 0xfe, 0xff];
    for &d in &values {
        for &s in &values {
            let (d, s) = (d << 16 | (255 - d) << 8 | d, s << 16 | s << 8 | (255 - s));
            for a in 0..=256 {
                assert_eq!(lerp(d, s, a), per_channel(d, s, a), "{d:x} {s:x} {a}");
            }
        }
    }
}

#[test]
fn span_loops_match_the_single_pixel_functions() {
    let row: Vec<u32> = (0..1031u32)
        .map(|i| 0xff00_0000 | ((i * 89) & 0xff) << 16 | ((i * 3) & 0xff) << 8 | ((i * 17) & 0xff))
        .collect();
    let other: Vec<u32> = row.iter().rev().copied().collect();
    let weights: Vec<u32> = (0..row.len() as u32).map(|i| i % 257).collect();
    let run = |f: &dyn Fn(&mut [u32])| {
        let mut out = row.clone();
        f(&mut out);
        out
    };
    for (color, a) in [(0x00c8_4010, 0), (0x80c8_4010, 128), (0xfe01_ff7f, 256)] {
        let sa = a.min(255);

        // 半透明矩形按颜色自带的透明度填满，结果与逐像素调用 `blend` 相同。
        let want: Vec<u32> = row.iter().map(|&p| blend(p, color, 255)).collect();
        assert_eq!(run(&|o| wide::mix_span(o, color, color >> 24)), want);
        let want: Vec<u32> = row.iter().map(|&p| mix(p, color, sa)).collect();
        assert_eq!(run(&|o| wide::mix_span(o, color, sa)), want);

        let want: Vec<u32> = other.iter().map(|&s| lerp(s, color, a)).collect();
        assert_eq!(run(&|o| wide::lerp_to_color(o, &other, color, a)), want);

        let want: Vec<u32> = (0..row.len()).map(|i| lerp(other[i], row[i], a)).collect();
        assert_eq!(run(&|o| wide::lerp_rows(o, &other, &row, a)), want);

        let want: Vec<u32> = (0..row.len()).map(|i| lerp(row[i], other[i], a)).collect();
        assert_eq!(run(&|o| wide::lerp_onto(o, &other, a)), want);

        let want: Vec<u32> = (0..row.len())
            .map(|i| lerp(row[i], lerp(row[i], other[i], weights[i]), a))
            .collect();
        assert_eq!(
            run(&|o| wide::lerp_pairs_onto(o, &row, &other, &weights, a)),
            want
        );
    }
}

/// 返回逐像素判断覆盖率、按四个相邻像素取样的合成结果；实现独立于 `Canvas::blend_layer`。
fn reference_layer(
    (w, h): (usize, usize),
    under: u32,
    layer: &[u32],
    (opacity, scale, radius): (f32, f32, f32),
) -> Vec<u32> {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (hw, hh, r) = (cx * scale, cy * scale, radius * scale);
    let channel = |p: u32, shift: u32| (p >> shift) & 0xff;
    let mut out = vec![under; w * h];
    for y in 0..h {
        let py = (y as f32 + 0.5 - cy).abs();
        let sy = (cy + (y as f32 + 0.5 - cy) / scale - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, fy) = (sy.floor() as usize, ((sy - sy.floor()) * 256.0) as u32);
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..w {
            let px = (x as f32 + 0.5 - cx).abs();
            if px > hw + 1.0 || py > hh + 1.0 {
                continue;
            }
            let qx = px - (hw - r);
            let qy = py - (hh - r);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            let a = (opacity * (0.5 - d).clamp(0.0, 1.0) * 256.0).round() as u32;
            let sx = (cx + (x as f32 + 0.5 - cx) / scale - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, fx) = (sx.floor() as usize, ((sx - sx.floor()) * 256.0) as u32);
            let x1 = (x0 + 1).min(w - 1);
            let ch = |shift: u32| {
                let c = |px: usize, py: usize| channel(layer[py * w + px], shift);
                let top = c(x0, y0) * (256 - fx) + c(x1, y0) * fx;
                let bottom = c(x0, y1) * (256 - fx) + c(x1, y1) * fx;
                let s = (top * (256 - fy) + bottom * fy) >> 16;
                let d = channel(under, shift);
                ((d * (256 - a) + s * a) >> 8) << shift
            };
            out[y * w + x] = 0xff00_0000 | ch(16) | ch(8) | ch(0);
        }
    }
    out
}

#[test]
fn composite_matches_the_pixel_by_pixel_result() {
    let (w, h) = (41usize, 29usize);
    let under = 0xff20_3040u32;
    let layer: Vec<u32> = (0..w * h)
        .map(|i| 0xff00_0000 | ((i as u32 * 7) & 0xff) << 16 | ((i as u32 * 13) & 0xff) << 8 | 0x55)
        .collect();
    let area = IRect::new(0, 0, w as i32, h as i32);
    for opacity in [0.0, 0.25, 0.6, 1.0] {
        for scale in [0.5, 0.9, 0.97, 1.0] {
            for radius in [0.0, 3.0, 8.0, 14.0] {
                let mut px = vec![under; w * h];
                canvas(&mut px, w, h).blend_layer(area, &layer, opacity, scale, radius);
                let expected = reference_layer((w, h), under, &layer, (opacity, scale, radius));
                for (i, (&got, &want)) in px.iter().zip(&expected).enumerate() {
                    // 缩放时纵向、横向分两步取整，每个通道最多相差 2。
                    let tolerance = if scale == 1.0 { 0 } else { 2 };
                    for shift in [16, 8, 0] {
                        let (g, e) = ((got >> shift) & 0xff, (want >> shift) & 0xff);
                        assert!(
                            g.abs_diff(e) <= tolerance,
                            "pixel {i}: {got:x} vs {want:x}, {opacity} {scale} {radius}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn div255_exact() {
    for x in 0..=255 * 255 {
        assert_eq!(div255(x), (x as f32 / 255.0).round() as u32, "x={x}");
    }
}

#[test]
fn rect_is_clipped_to_canvas_and_clip() {
    let mut px = vec![0u32; 8 * 4];
    let mut c = canvas(&mut px, 8, 4);
    c.rect(IRect::new(-2, -2, 6, 4), 0xffff_0000, ALL, None, None);
    c.rect(
        IRect::new(4, 0, 10, 10),
        0xff00_ff00,
        IRect::new(5, 1, 2, 2),
        None,
        None,
    );

    assert_eq!(px[0], 0xffff_0000);
    assert_eq!(px[3], 0xffff_0000);
    assert_eq!(px[4], 0);
    assert_eq!(px[8 + 3], 0xffff_0000);
    assert_eq!(px[2 * 8], 0);
    assert_eq!(px[8 + 5], 0xff00_ff00);
    assert_eq!(px[2 * 8 + 6], 0xff00_ff00);
    assert_eq!(px[8 + 7], 0);
    assert_eq!(px[3 * 8 + 5], 0);
}

#[test]
fn stride_padding_is_untouched() {
    let mut px = vec![7u32; 6 * 2];
    let mut c = Canvas {
        px: &mut px,
        width: 4,
        height: 2,
        stride: 6,
    };
    c.rect(IRect::new(0, 0, 100, 100), 0xff00_00ff, ALL, None, None);
    assert_eq!(&px[4..6], &[7, 7]);
    assert_eq!(px[6], 0xff00_00ff);
}

#[test]
fn rounded_corners_are_soft() {
    let mut px = vec![0xff00_0000u32; 20 * 20];
    let mut c = canvas(&mut px, 20, 20);
    c.rounded_rect(
        IRect::new(0, 0, 20, 20),
        [8.0; 4],
        0xffff_ffff,
        ALL,
        None,
        None,
    );
    assert_eq!(px[0], 0xff00_0000);
    assert_eq!(px[10 * 20 + 10], 0xffff_ffff);
    assert_ne!(px[2 * 20 + 2], 0xffff_ffff);
}

#[test]
fn per_corner_radii_and_shadow() {
    let mut px = vec![0xff00_0000u32; 20 * 20];
    let mut c = canvas(&mut px, 20, 20);
    c.rounded_rect(
        IRect::new(0, 0, 20, 20),
        [8.0, 0.0, 0.0, 0.0],
        0xffff_ffff,
        ALL,
        None,
        None,
    );
    // 只有左上角为圆角：左上角像素不变，右上角、左下角仍被填充。
    assert_eq!(px[0], 0xff00_0000);
    assert_eq!(px[19], 0xffff_ffff);
    assert_eq!(px[19 * 20], 0xffff_ffff);

    let mut px = vec![0xffff_ffffu32; 20 * 20];
    let mut c = canvas(&mut px, 20, 20);
    c.shadow(IRect::new(5, 5, 10, 10), 2.0, 4.0, 0x8000_0000, ALL, None);
    // 阴影中心浓度最高，向外越过边缘后逐渐变淡，`blur` 之外不绘制。
    let at = |x: usize| px[10 * 20 + x] & 0xff;
    assert!(at(10) < at(15) && at(15) < at(16) && at(16) < 0xff);
    assert_eq!(at(19), 0xff);
}

#[test]
fn skipping_what_an_opaque_card_covers_changes_nothing_you_can_see() {
    // 依次绘制阴影、半透明边框和遮罩，最后绘制不透明的圆角卡片：无论是否跳过卡片覆盖的像素，
    // 最终像素都必须完全相同。
    let draw = |covered: bool| {
        let mut px = vec![0xff20_4060u32; 40 * 40];
        let mut c = canvas(&mut px, 40, 40);
        let card = IRect::new(8, 6, 24, 26);
        let skip = covered.then(|| Covered::new(card, 5.0));
        c.rect(IRect::new(0, 0, 40, 40), 0x5900_0000, ALL, skip, None);
        c.shadow(IRect::new(8, 9, 24, 26), 5.0, 6.0, 0x8000_0000, ALL, skip);
        c.rounded_rect(
            IRect::new(7, 5, 26, 28),
            [6.0; 4],
            0x14ff_ffff,
            ALL,
            skip,
            None,
        );
        c.rounded_rect(card, [5.0; 4], 0xffee_eeee, ALL, None, None);
        px
    };
    assert_eq!(draw(true), draw(false));
}

#[test]
fn strokes_blend_once_where_they_cross() {
    let mut px = vec![0xff00_0000u32; 20 * 20];
    let mut c = canvas(&mut px, 20, 20);
    let cross = [
        Stroke::Line(2.0, 10.0, 18.0, 10.0),
        Stroke::Line(10.0, 2.0, 10.0, 18.0),
    ];
    c.stroke(&cross, 2.0, 1.0, 0x80ff_ffff, ALL);
    // 交点与只有一条线经过的像素颜色相同（只叠加一次颜色），距线较远的像素不变。
    assert_eq!(px[10 * 20 + 10], px[10 * 20 + 4]);
    assert_ne!(px[10 * 20 + 4], 0xff00_0000);
    assert_eq!(px[4 * 20 + 4], 0xff00_0000);

    let mut px = vec![0xff00_0000u32; 20 * 20];
    let mut c = canvas(&mut px, 20, 20);
    c.stroke(
        &[Stroke::Circle(10.0, 10.0, 6.0)],
        2.0,
        1.0,
        0xffff_ffff,
        ALL,
    );
    // 圆周上有颜色，圆心处没有颜色。
    assert_eq!(px[10 * 20 + 15], 0xffff_ffff);
    assert_eq!(px[10 * 20 + 10], 0xff00_0000);

    let mut px = vec![0xff00_0000u32; 20 * 20];
    let mut c = canvas(&mut px, 20, 20);
    let upper = Stroke::Arc(10.0, 10.0, 6.0, -std::f32::consts::PI, 0.0);
    c.stroke(&[upper], 2.0, 1.0, 0xffff_ffff, ALL);
    // 上半圆有颜色，下半圆没有。
    assert_eq!(px[4 * 20 + 10], 0xffff_ffff);
    assert_eq!(px[16 * 20 + 10], 0xff00_0000);
}

/// 返回作为参照的覆盖率：逐像素求到每个形状的距离，不筛选形状，每个像素都计算三角函数。
fn reference_coverage(shapes: &[Stroke], width: f32, scale: f32, x: i32, y: i32) -> u32 {
    let (px, py) = ((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
    let distance = |s: Stroke| match s {
        Stroke::Arc(cx, cy, r, from, to) => {
            let angle = (py - cy).atan2(px - cx);
            let on_arc = [angle - TAU, angle, angle + TAU]
                .iter()
                .any(|a| (from..=to).contains(a));
            if on_arc {
                ((px - cx).hypot(py - cy) - r).abs()
            } else {
                let end = |a: f32| (px - cx - r * a.cos()).hypot(py - cy - r * a.sin());
                end(from).min(end(to))
            }
        }
        s => Prepared::new(s, 0.0, scale).distance(px, py),
    };
    let d = shapes.iter().map(|&s| distance(s)).fold(f32::MAX, f32::min);
    ((width * scale / 2.0 + 0.5 - d * scale).clamp(0.0, 1.0) * 255.0) as u32
}

#[test]
fn culled_strokes_match_testing_every_shape() {
    use std::f32::consts::{FRAC_PI_2, PI};
    // 两组样本：图标式的小形状（圆角框、超过半圈的圆弧、零长度的点），以及图表式的大而空的形状
    // （矩形的四条边、斜向长线、折线、大小不同的圆环）。坐标均为非整数，按 1×、2× 和非整数倍
    // 各绘制一次，与逐像素求全部形状距离的结果逐点比较。
    let (x, y) = (3.3, 2.6);
    let icon = [
        Stroke::Line(x + 1.5, y, x + 10.5, y),
        Stroke::Arc(x + 10.5, y + 1.5, 1.5, -FRAC_PI_2, 0.0),
        Stroke::Line(x + 12.0, y + 1.5, x + 12.0, y + 9.5),
        Stroke::Arc(x + 6.0, y + 6.0, 4.0, -0.75 * PI, PI),
        Stroke::Arc(x + 6.0, y + 6.0, 2.5, 0.0, 1.75 * PI),
        Stroke::Circle(x + 3.0, y + 9.0, 1.2),
        Stroke::Line(x + 8.0, y + 9.0, x + 8.0, y + 9.0),
    ];
    let diagram = [
        Stroke::Line(x, y, x + 60.0, y),
        Stroke::Line(x + 60.0, y, x + 60.0, y + 30.0),
        Stroke::Line(x + 60.0, y + 30.0, x, y + 30.0),
        Stroke::Line(x, y + 30.0, x, y),
        Stroke::Line(x + 5.0, y + 40.0, x + 55.0, y + 33.0),
        Stroke::Line(x + 10.0, y + 31.0, x + 13.5, y + 52.0),
        Stroke::Line(x + 13.5, y + 52.0, x + 20.2, y + 55.1),
        Stroke::Circle(x + 30.0, y + 15.0, 9.5),
        Stroke::Circle(x + 45.0, y + 45.0, 0.6),
    ];
    for (shapes, scale) in [&icon[..], &diagram[..]]
        .into_iter()
        .flat_map(|s| [1.0, 2.0, 1.5].map(|k| (s, k)))
    {
        let area = stroke_area(shapes, 1.25, scale).unwrap();
        let mask = StrokeRaster::new(shapes, 1.25, scale).unwrap().mask();
        let w = area.width();
        for yy in area.y0..area.y1 {
            for xx in area.x0..area.x1 {
                let i = (yy - area.y0) as usize * w + (xx - area.x0) as usize;
                let want = reference_coverage(shapes, 1.25, scale, xx, yy);
                assert_eq!(u32::from(mask[i]), want, "({xx}, {yy}) ×{scale}");
            }
        }
    }
}

#[test]
fn rounded_rect_spans_match_pixel_by_pixel() {
    // 分为左角、实心部分、右角三部分绘制，结果与逐像素判断相同：奇数宽度（两个半径之和大于
    // 宽度）、半透明、被卡片覆盖一部分的情况都包括在内。
    let reference = |area: IRect, radii: [f32; 4], color: u32, covered: Option<Covered>| {
        let mut px = vec![0xff20_4060u32; 30 * 30];
        let limit = ((area.x1 - area.x0).min(area.y1 - area.y0) as f32 / 2.0).max(0.0);
        let radii = radii.map(|r| r.clamp(0.0, limit));
        let r_max = radii.iter().copied().fold(0.0, f32::max).ceil() as i32;
        let center = (
            (area.x0 + area.x1) as f32 / 2.0,
            (area.y0 + area.y1) as f32 / 2.0,
        );
        let v = area.intersect(IRect::new(0, 0, 30, 30));
        for y in v.y0..v.y1 {
            let py = y as f32 + 0.5;
            let solid = if y >= area.y0 + r_max && y < area.y1 - r_max {
                i32::MIN..i32::MAX
            } else {
                area.x0 + r_max..area.x1 - r_max
            };
            for (a, b) in uncovered(v, covered, y) {
                for x in a..b {
                    let p = &mut px[y as usize * 30 + x as usize];
                    if solid.contains(&x) {
                        *p = blend(*p, color, 255);
                        continue;
                    }
                    let px = x as f32 + 0.5;
                    let r = corner_radius(radii, px < center.0, py < center.1);
                    let d = rounded_distance(area, r, px, py);
                    let cov = ((0.5 - d).clamp(0.0, 1.0) * 255.0) as u32;
                    if cov > 0 {
                        *p = blend(*p, color, cov);
                    }
                }
            }
        }
        px
    };
    let cases = [
        (IRect::new(2, 3, 21, 17), [6.0, 2.0, 0.0, 4.5]),
        (IRect::new(4, 4, 5, 12), [4.0; 4]),
        (IRect::new(-3, 1, 30, 25), [7.0; 4]),
    ];
    for (area, radii) in cases {
        for color in [0xffee_eeee, 0x80ff_ffff] {
            for covered in [None, Some(Covered::new(IRect::new(6, 6, 12, 14), 3.0))] {
                let mut px = vec![0xff20_4060u32; 30 * 30];
                canvas(&mut px, 30, 30).rounded_rect(area, radii, color, ALL, covered, None);
                let want = reference(area, radii, color, covered);
                assert!(px == want, "{area:?} {radii:?} {color:08x}");
            }
        }
    }
}

#[test]
fn mask_is_clipped_against_its_own_origin() {
    let mut px = vec![0xff00_0000u32; 4 * 4];
    let mut c = canvas(&mut px, 4, 4);
    let gamma: [u8; 256] = std::array::from_fn(|i| i as u8);
    let data = [255, 0, 0, 255];
    c.mask(IRect::new(-1, -1, 2, 2), &data, 0xffff_ffff, &gamma, ALL);
    assert_eq!(px[0], 0xffff_ffff);
    assert_eq!(px[1], 0xff00_0000);
}

/// 覆盖率已是最终值的蒙版，绘制结果与按恒等映射表逐像素叠加相同，包括覆盖率为 0 和 255、
/// 半透明的颜色，以及裁剪后只显示一部分的情况。
#[test]
fn coverage_matches_mask_with_an_identity_gamma() {
    let identity: [u8; 256] = std::array::from_fn(|i| i as u8);
    let (w, h) = (37, 9);
    let data: Vec<u8> = (0..w * h).map(|i| (i * 7 % 256) as u8).collect();
    let dst: Vec<u32> = (0..w * h)
        .map(|i| 0xff00_0000 | (i as u32).wrapping_mul(2_654_435_761) >> 8)
        .collect();
    let clips = [ALL, IRect::new(3, 2, 30, 7)];
    for color in [
        0xffff_ffff,
        0xff12_3456,
        0x8040_c0ff,
        0x01ff_0000,
        0x00ff_ffff,
    ] {
        for (area, clip) in [IRect::new(0, 0, 37, 9), IRect::new(-5, -2, 32, 7)]
            .into_iter()
            .flat_map(|area| clips.map(|clip| (area, clip)))
        {
            let (mut a, mut b) = (dst.clone(), dst.clone());
            canvas(&mut a, w, h).mask(area, &data, color, &identity, clip);
            canvas(&mut b, w, h).coverage(area, &data, color, clip);
            assert!(a == b, "{color:08x} {area:?} {clip:?}");
        }
    }
}

mod composite_bench {
    use super::*;

    /// 测量设置卡片大小（2× 屏幕，1440×1040 个物理像素）的图层合成一次的耗时。
    #[test]
    #[ignore]
    fn measure_composite() {
        let (w, h) = (1440usize, 1040usize);
        let mut px = vec![0xff20_2020u32; w * h];
        let mut c = Canvas {
            px: &mut px,
            width: w,
            height: h,
            stride: w,
        };
        let area = IRect::new(0, 0, w as i32, h as i32);
        let (area, layer) = c.copy(area);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            c.blend_layer(area, &layer, 0.6, 0.97, 24.0);
        }
        println!("缩放合成 {:?}", start.elapsed() / 10);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            c.blend_layer(area, &layer, 0.6, 1.0, 24.0);
        }
        println!("不缩放合成 {:?}", start.elapsed() / 10);
    }
}

#[test]
fn fill_except_leaves_the_covered_block_alone() {
    let (w, h) = (8, 6);
    let mut px = vec![7; w * h];
    let area = IRect {
        x0: 2,
        y0: 1,
        x1: 6,
        y1: 4,
    };
    canvas(&mut px, w, h).fill_except(0x123456, Some(Covered::new(area, 0.0)));
    for y in 0..h {
        for x in 0..w {
            let inside = (2..6).contains(&x) && (1..4).contains(&y);
            let want = if inside { 7 } else { 0xff12_3456 };
            assert_eq!(px[y * w + x], want, "({x}, {y})");
        }
    }
    // 没有需要跳过的像素时，结果与 `fill` 相同，填满整个画布。
    canvas(&mut px, w, h).fill_except(0x123456, None);
    assert!(px.iter().all(|&p| p == 0xff12_3456));
}

#[test]
fn layered_fill_matches_filling_then_drawing_each_rect() {
    let rect = |x0, y0, x1, y1| IRect { x0, y0, x1, y1 };
    let rects = [
        (rect(1, 1, 4, 6), 0xff11_1111),
        // 与上一个重叠；alpha 不是 `0xff`。
        (rect(3, 0, 9, 3), 0x0022_2222),
        // 超出画布。
        (rect(-5, 5, 30, 40), 0xff33_3333),
        // 空矩形、完全位于画布之外的矩形。
        (rect(4, 4, 4, 6), 0xff44_4444),
        (rect(-8, 0, 0, 7), 0xff55_5555),
    ];
    // 行宽大于画布宽度；`short` 为真时最后一行没有行尾的填充部分。
    let (w, h, stride) = (9, 7, 12);
    let mut row = Vec::new();
    for (count, short) in [(5, false), (5, true), (1, false), (0, false)] {
        let len = stride * h - if short { stride - w } else { 0 };
        let rects = &rects[..count];
        let mut want = vec![7; len];
        let mut c = Canvas {
            px: &mut want,
            width: w,
            height: h,
            stride,
        };
        c.fill(0x123456);
        for &(area, color) in rects {
            c.rect(area, color | 0xff00_0000, ALL, None, None);
        }
        let mut got = vec![7; len];
        let mut c = Canvas {
            px: &mut got,
            width: w,
            height: h,
            stride,
        };
        c.fill_layered(0x123456, rects, &mut row);
        assert_eq!(got, want, "{count} 个矩形，short = {short}");
    }
    // 宽度为 0 的画布不 panic。
    canvas(&mut [], 0, 4).fill_layered(0x123456, &rects, &mut row);
}

/// 回归：`paste` 不先裁剪到画布内时，`area` 超出画布会导致越界 panic 或写入下一行。
#[test]
fn pasting_clips_to_the_canvas() {
    // 把 3 × 2 的像素写回左上角位于 `(-1, 1)` 的区域：只有中间、右侧两列的第一行落在 2 × 2 的
    // 画布内，画布上的值应为 `[0, 0, 2, 3]`。
    let mut px = vec![0; 4];
    let mut c = canvas(&mut px, 2, 2);
    let area = IRect::new(-1, 1, 3, 2);
    c.paste(area, &[1, 2, 3, 4, 5, 6]);
    assert_eq!(px, [0, 0, 2, 3]);
    // 叠加颜色的写回同样先裁剪；完全位于画布之外的区域不写入。
    let mut px = vec![0; 4];
    let mut c = canvas(&mut px, 2, 2);
    c.paste_tinted(IRect::new(1, -1, 2, 2), &[0xff00_0000; 4], 0x80ff_ffff);
    c.paste(IRect::new(5, 5, 1, 1), &[9]);
    assert_eq!(px[0], 0);
    assert_ne!(px[1], 0);
    assert_eq!(px[2..], [0, 0]);
}

/// 回归：透明图片的棋盘格逐格按矩形绘制时，大图上每帧要绘制上万个矩形。
///
/// 按方格一次填色：左上角的方格为第一种颜色，相邻方格为第二种；画布之外、裁剪框之外不绘制。
#[test]
fn checkers_alternate_cell_by_cell() {
    let (a, b) = (0xff11_1111, 0xff22_2222);
    let mut px = vec![0; 6 * 4];
    let mut c = canvas(&mut px, 6, 4);
    c.checker(IRect::new(-1, 0, 7, 4), 2, (a, b), IRect::new(0, 0, 5, 4));
    // 方格从 x = -1 起算：第 0 列属于第 0 格的后半部分，因此第一行从第一种颜色开始。
    let row = |y: usize| px[y * 6..y * 6 + 6].to_vec();
    assert_eq!(row(0), [a, b, b, a, a, 0]);
    assert_eq!(row(1), [a, b, b, a, a, 0]);
    assert_eq!(row(2), [b, a, a, b, b, 0]);
    assert_eq!(row(3), [b, a, a, b, b, 0]);
}
