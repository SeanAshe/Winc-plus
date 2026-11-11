//! 像素画布：所有绘制最终写入一块不透明的 32 位像素缓冲。
//!
//! 像素为 `0xAARRGGBB`，绘制到画布后 alpha 恒为 `0xff`。[`Canvas`] 负责矩形、圆角矩形、阴影、
//! 描边、蒙版和位图的混合绘制，坐标和尺寸都是物理像素；[`Stroke`] 的形状坐标是逻辑像素。

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::ops::Range;

mod wide;

/// 物理像素下的矩形，左闭右开：包含 `(x0, y0)`，不含 `(x1, y1)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IRect {
    /// 左边界（含）。
    pub x0: i32,
    /// 上边界（含）。
    pub y0: i32,
    /// 右边界（不含）。
    pub x1: i32,
    /// 下边界（不含）。
    pub y1: i32,
}

impl IRect {
    /// 用左上角坐标 `(x, y)` 和宽高 `w`、`h` 创建矩形。
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        }
    }

    /// 返回两个矩形的交集；不相交时结果为空矩形（[`IRect::is_empty`] 为真）。
    fn intersect(self, other: IRect) -> IRect {
        IRect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    /// 返回宽度（像素）；右边界不大于左边界时为 0。
    pub(crate) fn width(self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }

    /// 是否宽度或高度为 0（或为负）。
    fn is_empty(self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
}

/// 背景图层：窗口背景图按不透明度预乘后的像素，以及画布各列、各行对应的图层列、行。
#[derive(Clone, Copy)]
pub(crate) struct Wall<'a> {
    pub px: &'a [u32],
    pub width: usize,
    pub xmap: &'a [u32],
    pub ymap: &'a [u32],
    /// 画布上显示图片的范围；之外只有底色。
    pub span: IRect,
}

/// 透出背景图的一种底色：图层的像素加上 `k`（底色按 1 减去图片的不透明度预乘）即最终的颜色。
///
/// 两者各通道之和不超过 255（见 `Wallpaper::new`），打包在一个整数中相加不会进位。
#[derive(Clone, Copy)]
pub(crate) struct Tinted<'a> {
    pub wall: Wall<'a>,
    pub k: u32,
    /// 底色本身（不透明的 `0xAARRGGBB`），用在图片的范围之外。
    pub plain: u32,
}

impl Tinted<'_> {
    /// 返回画布上 `(x, y)` 处的颜色。
    fn at(&self, x: i32, y: i32) -> u32 {
        let wall = &self.wall;
        let span = wall.span;
        if x < span.x0 || x >= span.x1 || y < span.y0 || y >= span.y1 {
            return self.plain;
        }
        let row = wall.ymap[y as usize] as usize * wall.width;
        wall.px[row + wall.xmap[x as usize] as usize] + self.k
    }

    /// 把画布第 `y` 行自 `x0` 起的颜色写入 `row`。
    fn write(&self, y: i32, x0: i32, row: &mut [u32]) {
        let wall = &self.wall;
        let span = wall.span;
        if y < span.y0 || y >= span.y1 {
            return row.fill(self.plain);
        }
        let x1 = x0 + row.len() as i32;
        let (a, b) = (span.x0.clamp(x0, x1), span.x1.clamp(x0, x1));
        let (left, rest) = row.split_at_mut((a - x0) as usize);
        let (inside, right) = rest.split_at_mut((b - a) as usize);
        left.fill(self.plain);
        right.fill(self.plain);
        let line = &wall.px[wall.ymap[y as usize] as usize * wall.width..][..wall.width];
        for (p, &x) in inside.iter_mut().zip(&wall.xmap[a as usize..]) {
            *p = line[x as usize] + self.k;
        }
    }
}

/// 一块可变像素缓冲。
pub(crate) struct Canvas<'a> {
    /// 像素数据，按行优先排列，每个像素为 `0xAARRGGBB`。
    pub px: &'a mut [u32],
    /// 可绘制区域的宽度（物理像素）。
    pub width: usize,
    /// 可绘制区域的高度（物理像素）。
    pub height: usize,
    /// 一行占用的像素数，可能因为行对齐而大于 `width`；多出的填充部分不会被绘制修改。
    pub stride: usize,
}

/// 按权重 `a`（单位 1/256，取值 0–256）从不透明像素 `d` 过渡到 `s`。
///
/// 红、蓝两个通道存放在同一个 32 位整数中相隔 16 位的两个字段，一次乘法同时计算两个通道；每个
/// 字段最大 255 × 256 = 65280，不超过 16 位，字段之间不会进位，结果与逐通道计算完全相同（见测试
/// `lerp_matches_per_channel`），循环中也没有分支。
#[inline(always)]
fn lerp(d: u32, s: u32, a: u32) -> u32 {
    let da = 256 - a;
    let rb = ((d & 0x00ff_00ff) * da + (s & 0x00ff_00ff) * a) >> 8;
    let g = ((d & 0x0000_ff00) * da + (s & 0x0000_ff00) * a) >> 8;
    0xff00_0000 | (rb & 0x00ff_00ff) | (g & 0x0000_ff00)
}

/// 返回 `x / 255` 四舍五入后的整数，对 `0..=65025` 精确（见测试 `div255_exact`）。
#[inline(always)]
fn div255(x: u32) -> u32 {
    (x + 128 + ((x + 128) >> 8)) >> 8
}

/// 返回按覆盖率 `cov`（0 至 255）把 `src` 叠加到 `dst` 上得到的不透明像素。
///
/// `src` 是非预乘的 `0xAARRGGBB`。
#[inline(always)]
fn blend(dst: u32, src: u32, cov: u32) -> u32 {
    let a = (src >> 24) & 0xff;

    if a == 0 || cov == 0 {
        return dst;
    }

    let sa = div255(a * cov);

    if sa >= 255 {
        return src | 0xff00_0000;
    }
    mix(dst, src, sa)
}

/// 返回按透明度 `sa`（0 至 255）把 `src` 的颜色叠加到 `dst` 上得到的不透明像素。
///
/// 忽略 `src` 自带的透明度。红、蓝两个通道存放在同一个 32 位整数中相隔 16 位的两个字段，一次乘法
/// 同时计算两个通道；每个字段最大 255 × 255 = 65025，加上 [`div255`] 的进位也不超过 16 位，字段
/// 之间不会进位，结果与逐通道计算 [`div255`] 完全相同（见测试 `mix_matches_per_channel`），
/// 循环中也没有分支。
#[inline(always)]
fn mix(dst: u32, src: u32, sa: u32) -> u32 {
    let da = 255 - sa;
    let rb = (src & 0x00ff_00ff) * sa + (dst & 0x00ff_00ff) * da + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let g = div255(((src >> 8) & 0xff) * sa + ((dst >> 8) & 0xff) * da);
    0xff00_0000 | rb | (g << 8)
}

/// 把同一位置分别绘制在黑底和白底上得到的两个像素，还原为透明底上的像素（已预乘透明度）。
///
/// 白底像素比黑底像素多出的亮度等于透出的底色，因此用两者绿色通道的差得到透明度；黑底上的颜色
/// 即为颜色乘以透明度。
pub(crate) fn unblend(on_black: u32, on_white: u32) -> u32 {
    let shown = ((on_white >> 8) & 0xff).saturating_sub((on_black >> 8) & 0xff);
    let alpha = 255 - shown;
    // 舍入可能使某个分量比透明度大 1，而预乘颜色的分量不能超过透明度。
    let channel = |shift: u32| ((on_black >> shift) & 0xff).min(alpha) << shift;
    (alpha << 24) | channel(16) | channel(8) | channel(0)
}

/// 返回所在象限对应的圆角半径；`radii` 依次是左上、右上、右下、左下。
#[inline(always)]
fn corner_radius(radii: [f32; 4], left: bool, top: bool) -> f32 {
    match (left, top) {
        (true, true) => radii[0],
        (false, true) => radii[1],
        (false, false) => radii[2],
        (true, false) => radii[3],
    }
}

/// 返回点 `(px, py)` 到圆角矩形 `area` 边缘的有向距离（像素）：内部为负，外部为正。
///
/// `r` 是圆角半径。
#[inline(always)]
fn rounded_distance(area: IRect, r: f32, px: f32, py: f32) -> f32 {
    let half_w = (area.x1 - area.x0) as f32 / 2.0;
    let half_h = (area.y1 - area.y0) as f32 / 2.0;
    let qx = (px - (area.x0 as f32 + half_w)).abs() - (half_w - r);
    let qy = (py - (area.y0 as f32 + half_h)).abs() - (half_h - r);
    let outside = (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// 描边的基本形状，坐标是逻辑像素。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stroke {
    /// 从 `(x0, y0)` 到 `(x1, y1)` 的线段，两端是圆头。
    Line(f32, f32, f32, f32),
    /// 圆心 `(x, y)`、半径 `r` 的圆周。
    Circle(f32, f32, f32),
    /// 圆心 `(x, y)`、半径 `r` 的一段圆弧，从角度 `from` 到 `to`（弧度，`from < to`）。
    ///
    /// 角度 0 指向右侧；屏幕坐标 y 向下，因此 -π 到 0 是上半圆。两端是圆头。
    Arc(f32, f32, f32, f32, f32),
}

impl Stroke {
    /// 返回外接矩形的（左，上，右，下）（逻辑像素），不含笔宽。
    fn bounds(self) -> (f32, f32, f32, f32) {
        match self {
            Stroke::Line(x0, y0, x1, y1) => (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)),
            Stroke::Circle(cx, cy, r) | Stroke::Arc(cx, cy, r, ..) => {
                (cx - r, cy - r, cx + r, cy + r)
            }
        }
    }

    /// 返回用半笔宽 `half`（物理像素）描边该形状时可能有覆盖率的像素范围：外接矩形按 `scale`
    /// 换算成物理像素，再向外扩展半个笔宽和 1 像素的抗锯齿边。
    fn pixel_area(self, half: f32, scale: f32) -> IRect {
        let (left, top, right, bottom) = self.bounds();
        IRect {
            x0: (left * scale - half - 1.0).floor() as i32,
            y0: (top * scale - half - 1.0).floor() as i32,
            x1: (right * scale + half + 1.0).ceil() as i32,
            y1: (bottom * scale + half + 1.0).ceil() as i32,
        }
    }
}

/// 返回圆角矩形描边的八段笔画：四条边加四段圆角弧线。
///
/// 矩形左上角为 `(x, y)`，宽 `w`、高 `h`，圆角半径 `r`，单位均为逻辑像素；`r` 不超过
/// `w / 2` 与 `h / 2` 中的较小值。
pub fn rounded_outline(x: f32, y: f32, w: f32, h: f32, r: f32) -> [Stroke; 8] {
    let (right, bottom) = (x + w, y + h);
    [
        Stroke::Line(x + r, y, right - r, y),
        Stroke::Arc(right - r, y + r, r, -FRAC_PI_2, 0.0),
        Stroke::Line(right, y + r, right, bottom - r),
        Stroke::Arc(right - r, bottom - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(right - r, bottom, x + r, bottom),
        Stroke::Arc(x + r, bottom - r, r, FRAC_PI_2, PI),
        Stroke::Line(x, bottom - r, x, y + r),
        Stroke::Arc(x + r, y + r, r, -PI, -FRAC_PI_2),
    ]
}

/// 返回用笔宽 `width`（逻辑像素）描边 `shapes` 时可能有覆盖率的像素范围（物理像素，按 `scale`
/// 换算）；`shapes` 为空时返回 `None`。
pub(crate) fn stroke_area(shapes: &[Stroke], width: f32, scale: f32) -> Option<IRect> {
    let half = width * scale / 2.0;
    shapes
        .iter()
        .map(|s| s.pixel_area(half, scale))
        .reduce(|a, b| IRect {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        })
}

/// 预处理后用于逐像素求距离的形状：预先计算圆弧两端相对圆心的位置，避免每个像素都计算三角函数。
struct Prepared {
    shape: Stroke,
    /// 圆弧起点、终点相对圆心的偏移（逻辑像素）；不是圆弧时不使用。
    ends: [(f32, f32); 2],
    /// 可能有覆盖率的像素范围，见 `Stroke::pixel_area`。
    ///
    /// 范围外的像素到形状的距离超过半个笔宽加 1 像素，覆盖率必为 0。
    area: IRect,
}

impl Prepared {
    fn new(shape: Stroke, half: f32, scale: f32) -> Self {
        let ends = match shape {
            Stroke::Arc(_, _, r, from, to) => [from, to].map(|a| (r * a.cos(), r * a.sin())),
            _ => [(0.0, 0.0); 2],
        };
        Prepared {
            shape,
            ends,
            area: shape.pixel_area(half, scale),
        }
    }

    /// 把第 `y` 行中可能有覆盖率的像素段（物理像素，左含右不含）追加到 `out`；`half` 为半个
    /// 笔宽（物理像素）。
    ///
    /// 只取到中心线的距离不超过半个笔宽加 1 像素的部分，矩形边框的内部、大圆的内部不再逐像素
    /// 求距离：线段取该行上下该距离内的部分，圆取圆环与该行相交的一段或两段，圆弧取外接矩形与
    /// 该行相交的一段。
    fn row_spans(&self, y: i32, half: f32, scale: f32, out: &mut Vec<(i32, i32)>) {
        let area = self.area;
        let reach = half + 1.0;
        let yc = y as f32 + 0.5;
        let mut push = |left: f32, right: f32| {
            let (a, b) = ((left - 0.5).floor() as i32, (right + 0.5).ceil() as i32);
            let (a, b) = (a.max(area.x0), b.min(area.x1));
            if a < b {
                out.push((a, b));
            }
        };
        match self.shape {
            Stroke::Line(x0, y0, x1, y1) => {
                let (ax, ay, bx, by) = (x0 * scale, y0 * scale, x1 * scale, y1 * scale);
                let dy = by - ay;
                let (lo, hi) = if dy.abs() < 1e-3 {
                    (0.0, 1.0)
                } else {
                    let (t0, t1) = ((yc - reach - ay) / dy, (yc + reach - ay) / dy);
                    (t0.min(t1).max(0.0), t0.max(t1).min(1.0))
                };
                if lo <= hi {
                    let (xa, xb) = (ax + (bx - ax) * lo, ax + (bx - ax) * hi);
                    push(xa.min(xb) - reach, xa.max(xb) + reach);
                }
            }
            Stroke::Circle(cx, cy, r) => {
                let (cx, r) = (cx * scale, r * scale);
                let dy = (yc - cy * scale).abs();
                let outer = r + reach;
                if dy > outer {
                    return;
                }
                let wide = (outer * outer - dy * dy).sqrt();
                let inner = r - reach;
                if inner > 0.0 && dy < inner {
                    let narrow = (inner * inner - dy * dy).sqrt();
                    push(cx - wide, cx - narrow);
                    push(cx + narrow, cx + wide);
                } else {
                    push(cx - wide, cx + wide);
                }
            }
            Stroke::Arc(..) => out.push((area.x0, area.x1)),
        }
    }

    /// 返回点 `(px, py)` 到该形状中心线的距离（逻辑像素）。
    fn distance(&self, px: f32, py: f32) -> f32 {
        match self.shape {
            Stroke::Line(x0, y0, x1, y1) => {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 0.0 {
                    (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (px - x0 - t * dx).hypot(py - y0 - t * dy)
            }
            Stroke::Circle(cx, cy, r) => ((px - cx).hypot(py - cy) - r).abs(),
            Stroke::Arc(cx, cy, r, from, to) => {
                let angle = (py - cy).atan2(px - cx);
                // `atan2` 的结果在 -π 到 π 之间，圆弧的角度范围可能超出该区间（如 0 到 1.75π），
                // 因此分别加减 2π 各检查一次。
                let on_arc = [angle - TAU, angle, angle + TAU]
                    .iter()
                    .any(|a| (from..=to).contains(a));

                if on_arc {
                    ((px - cx).hypot(py - cy) - r).abs()
                } else {
                    let end = |(ex, ey): (f32, f32)| (px - cx - ex).hypot(py - cy - ey);
                    end(self.ends[0]).min(end(self.ends[1]))
                }
            }
        }
    }
}

/// 一组描边形状的逐行覆盖率：[`Canvas::stroke`] 按它直接混合，描边缓存用它生成蒙版。
///
/// 每行先选出范围跨越该行的形状，只计算它们在该行上可能有覆盖率的像素段（见
/// `Prepared::row_spans`）；每个像素只对范围覆盖该像素的形状求距离。覆盖率只取决于最近的
/// 形状，未选中的形状距离更远，因此结果与对每个像素求全部形状的距离相同。
pub(crate) struct StrokeRaster {
    shapes: Vec<Prepared>,
    /// 当前行用到的形状在 `shapes` 中的下标，跨行复用以避免每行分配内存。
    active: Vec<usize>,
    /// 当前行需要计算的像素段（物理像素，左含右不含），跨行复用以避免每行分配内存。
    spans: Vec<(i32, i32)>,
    /// 半个笔宽（物理像素）。
    half: f32,
    /// 屏幕缩放系数。
    scale: f32,
    /// 全部形状可能有覆盖率的范围（物理像素），即 [`stroke_area`](Self::stroke_area)。
    pub area: IRect,
}

impl StrokeRaster {
    /// 准备用笔宽 `width`（逻辑像素）描边 `shapes`，按 `scale` 换算成物理像素；
    /// `shapes` 为空时返回 `None`。
    pub(crate) fn new(shapes: &[Stroke], width: f32, scale: f32) -> Option<Self> {
        let area = stroke_area(shapes, width, scale)?;
        let half = width * scale / 2.0;
        Some(StrokeRaster {
            shapes: shapes
                .iter()
                .map(|&s| Prepared::new(s, half, scale))
                .collect(),
            active: Vec::with_capacity(shapes.len()),
            spans: Vec::new(),
            half,
            scale,
            area,
        })
    }

    /// 计算第 `y` 行中 `x0..x1` 这一段像素的覆盖率（0 至 255）。
    ///
    /// 对覆盖率大于 0 的像素调用 `put(x, 覆盖率)`。
    fn row(&mut self, y: i32, (x0, x1): (i32, i32), mut put: impl FnMut(i32, u32)) {
        let StrokeRaster {
            shapes,
            active,
            spans,
            half,
            scale,
            ..
        } = self;
        let (half, scale) = (*half, *scale);
        active.clear();
        active.extend((0..shapes.len()).filter(|&i| {
            let a = shapes[i].area;
            a.y0 <= y && y < a.y1 && a.x0 < x1 && x0 < a.x1
        }));
        // 各形状的像素段裁剪到 `x0..x1`，排序后合并重叠或相接的段：段外的像素不计算，
        // 段内每个像素只计算一次。
        spans.clear();
        for &i in active.iter() {
            shapes[i].row_spans(y, half, scale, spans);
        }
        spans.retain_mut(|s| {
            *s = (s.0.max(x0), s.1.min(x1));
            s.0 < s.1
        });
        spans.sort_unstable();
        let mut merged = 0;
        for k in 0..spans.len() {
            if merged > 0 && spans[k].0 <= spans[merged - 1].1 {
                spans[merged - 1].1 = spans[merged - 1].1.max(spans[k].1);
            } else {
                spans[merged] = spans[k];
                merged += 1;
            }
        }
        spans.truncate(merged);
        let py = (y as f32 + 0.5) / scale;

        for &(lo, hi) in spans.iter() {
            for x in lo..hi {
                let px = (x as f32 + 0.5) / scale;
                let d = active
                    .iter()
                    .map(|&i| &shapes[i])
                    .filter(|s| s.area.x0 <= x && x < s.area.x1)
                    .map(|s| s.distance(px, py))
                    .fold(f32::MAX, f32::min);
                let cov = ((half + 0.5 - d * scale).clamp(0.0, 1.0) * 255.0) as u32;

                if cov > 0 {
                    put(x, cov);
                }
            }
        }
    }

    /// 返回整个 [`Self::area`] 的覆盖率蒙版：每像素 1 字节（0–255），按行排列。
    pub(crate) fn mask(mut self) -> Box<[u8]> {
        let area = self.area;
        let w = area.width();
        let mut data = vec![0u8; w * (area.y1 - area.y0) as usize];
        for (r, y) in (area.y0..area.y1).enumerate() {
            let row = &mut data[r * w..(r + 1) * w];
            self.row(y, (area.x0, area.x1), |x, cov| {
                row[(x - area.x0) as usize] = cov as u8;
            });
        }
        data.into_boxed_slice()
    }
}

impl Canvas<'_> {
    /// 返回 `area` 裁剪到画布之内后的范围，以及其中像素的副本。
    ///
    /// 像素按行排列，每行 `area` 宽。
    pub fn copy(&self, area: IRect) -> (IRect, Vec<u32>) {
        let mut out = Vec::new();
        let area = self.copy_into(area, &mut out);
        (area, out)
    }

    /// 与 [`Self::copy`] 相同，但复制到 `out`（先清空，保留已有容量），返回裁剪后的 `area`。
    ///
    /// 用于每帧都要复制的场景：避免每帧分配并释放数 MB 内存。
    pub fn copy_into(&self, area: IRect, out: &mut Vec<u32>) -> IRect {
        out.clear();
        let area = area.intersect(IRect::new(0, 0, self.width as i32, self.height as i32));
        if area.is_empty() {
            return area;
        }
        let w = area.width();
        out.reserve(w * (area.y1 - area.y0) as usize);
        for y in area.y0..area.y1 {
            let start = y as usize * self.stride + area.x0 as usize;
            out.extend_from_slice(&self.px[start..start + w]);
        }
        area
    }

    /// 把 [`Canvas::copy`] 取出的像素 `px` 写回 `area`；画布之外的部分忽略。
    pub fn paste(&mut self, area: IRect, px: &[u32]) {
        for (dst, src) in self.paste_rows(area, px) {
            self.px[dst.clone()].copy_from_slice(&px[src]);
        }
    }

    /// 返回写回 `area`（`px` 按 `area` 的宽排列）时每一行的下标范围：画布中的范围与 `px` 中的范围。
    ///
    /// 两个范围都已裁剪到画布内；`px` 的长度与 `area` 不符时不返回任何行。先裁剪再计算下标，
    /// 因此 `area` 超出画布（窗口缩小、坐标为负）时不会越界或写入下一行。
    fn paste_rows(
        &self,
        area: IRect,
        px: &[u32],
    ) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + use<> {
        let w = area.width();
        let clipped = area.intersect(IRect::new(0, 0, self.width as i32, self.height as i32));
        let valid =
            !area.is_empty() && !clipped.is_empty() && px.len() == w * (area.y1 - area.y0) as usize;
        let rows = if valid { clipped.y0..clipped.y1 } else { 0..0 };
        let (stride, cw) = (self.stride, clipped.width());
        let dx = (clipped.x0 - area.x0).max(0) as usize;
        rows.map(move |y| {
            let dst = y as usize * stride + clipped.x0 as usize;
            let src = (y - area.y0) as usize * w + dx;
            (dst..dst + cw, src..src + cw)
        })
    }

    /// 与 [`Canvas::paste`] 相同，写回时按 `tint`（非预乘的 `0xAARRGGBB`）的 alpha 叠加一层颜色。
    ///
    /// 一次遍历同时完成写回与压暗，无需写回后再整块绘制一层半透明遮罩。
    pub fn paste_tinted(&mut self, area: IRect, px: &[u32], tint: u32) {
        let a = (tint >> 24) * 256 / 255;
        if a == 0 {
            self.paste(area, px);
            return;
        }
        for (dst, src) in self.paste_rows(area, px) {
            wide::lerp_to_color(&mut self.px[dst], &px[src], tint, a);
        }
    }

    /// 把图层 `layer`（`area` 大小的像素，按行排列）叠加到画布现有的像素上。
    ///
    /// 图层是 `area` 中圆角半径为 `radius`（物理像素）的圆角矩形，以中心为原点缩放 `scale`，再按
    /// 不透明度 `opacity`（0–1）叠加。整层绘制完成后一次性叠加，半透明的卡片才不会透出自身下方的
    /// 边框和底色；圆角之外、缩小后空出的区域保持不变，边缘按覆盖率柔化。缩放按双线性取样：每行先
    /// 在纵向混合出一行像素，再在横向取样。每行内部的覆盖率相同，只有边缘和圆角逐像素计算覆盖率。
    pub fn blend_layer(
        &mut self,
        area: IRect,
        layer: &[u32],
        opacity: f32,
        scale: f32,
        radius: f32,
    ) {
        let area_c = area.intersect(IRect::new(0, 0, self.width as i32, self.height as i32));
        let (w, h) = (area.width(), (area.y1 - area.y0).max(0) as usize);
        if area_c != area || area.is_empty() || layer.len() != w * h {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let scale = scale.max(0.01);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        // 缩放后的圆角矩形：中心不变，半宽、半高和圆角半径都乘以 `scale`。
        let (hw, hh, r) = (cx * scale, cy * scale, radius * scale);
        // 每列、每行在图层中的取样位置：左（上）侧像素和相邻像素的下标，
        // 以及后者的权重（单位 1/256）。
        let samples = |n: usize, c: f32| -> Vec<(usize, usize, u32)> {
            (0..n)
                .map(|i| {
                    let s = (c + (i as f32 + 0.5 - c) / scale - 0.5).clamp(0.0, (n - 1) as f32);
                    let i0 = s.floor() as usize;
                    (i0, (i0 + 1).min(n - 1), ((s - i0 as f32) * 256.0) as u32)
                })
                .collect()
        };
        let (xs, ys) = (samples(w, cx), samples(h, cy));
        let scaled = (scale - 1.0).abs() >= 1e-4;
        let full = (opacity * 256.0).round() as u32;
        // 边缘和圆角上的覆盖率（单位 1/256）：按像素中心到圆角矩形的距离计算。
        let edge = |px: f32, py: f32| {
            let qx = px - (hw - r);
            let qy = py - (hh - r);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            (opacity * (0.5 - d).clamp(0.0, 1.0) * 256.0).round() as u32
        };
        let columns = if scaled { w } else { 0 };
        let (mut blended, mut before, mut after) = (
            vec![0u32; columns],
            vec![0u32; columns],
            vec![0u32; columns],
        );
        let weights: Vec<u32> = xs.iter().map(|&(_, _, fx)| fx).collect();

        for (y, &(y0, y1, fy)) in ys.iter().enumerate() {
            let py = (y as f32 + 0.5 - cy).abs();
            if py > hh + 1.0 {
                continue;
            }
            // 该行中距边缘超过一个像素且不在圆角区域内的列：`inner.0..inner.1`。
            let half = if py >= hh - 1.0 {
                0.0
            } else if py < hh - r - 1.0 {
                hw - 1.0
            } else {
                hw - r - 1.0
            };
            let inner = if half > 0.0 {
                let from = ((cx - half - 0.5).floor() as isize + 1).clamp(0, w as isize) as usize;
                let to = ((cx + half - 0.5).ceil() as isize).clamp(0, w as isize) as usize;
                (from, to.max(from))
            } else {
                (w, w)
            };
            let source: &[u32] = if scaled {
                let (top, bottom) = (&layer[y0 * w..][..w], &layer[y1 * w..][..w]);
                wide::lerp_rows(&mut blended, top, bottom, fy);
                &blended
            } else {
                &layer[y * w..][..w]
            };
            let sample = |x: usize| {
                if scaled {
                    let (x0, x1, fx) = xs[x];
                    lerp(source[x0], source[x1], fx)
                } else {
                    source[x]
                }
            };
            let row = &mut self.px[(area.y0 as usize + y) * self.stride + area.x0 as usize..][..w];
            let (left, rest) = row.split_at_mut(inner.0.min(w));
            let (middle, right) = rest.split_at_mut(inner.1.min(w) - inner.0.min(w));

            if scaled {
                // 先取出每列的两个相邻像素，使混合循环只读取连续内存，便于向量化。
                let (span, fxs) = (&xs[inner.0..inner.1], &weights[inner.0..inner.1]);
                let (first, second) = (&mut before[..span.len()], &mut after[..span.len()]);
                for ((a, b), &(x0, x1, _)) in first.iter_mut().zip(&mut *second).zip(span) {
                    (*a, *b) = (source[x0], source[x1]);
                }
                wide::lerp_pairs_onto(middle, first, second, fxs, full);
            } else {
                wide::lerp_onto(middle, &source[inner.0..inner.1], full);
            }
            for (part, start) in [(left, 0), (right, inner.1)] {
                for (p, x) in part.iter_mut().zip(start..) {
                    let px = (x as f32 + 0.5 - cx).abs();
                    if px > hw + 1.0 {
                        continue;
                    }
                    let a = edge(px, py);
                    if a > 0 {
                        *p = lerp(*p, sample(x), a);
                    }
                }
            }
        }
    }

    /// 用 `color` 填充整个画布；alpha 强制为不透明。
    pub fn fill(&mut self, color: u32) {
        let n = (self.stride * self.height).min(self.px.len());
        self.px[..n].fill(color | 0xff00_0000);
    }

    /// 与 [`Self::fill`] 相同，但跳过 `covered` 中的像素（这些像素随后会被不透明的内容覆盖）。
    pub fn fill_except(&mut self, color: u32, covered: Option<Covered>) {
        let Some(covered) = covered else {
            return self.fill(color);
        };
        let all = IRect {
            x0: 0,
            y0: 0,
            x1: self.width as i32,
            y1: self.height as i32,
        };
        for y in 0..self.height as i32 {
            for (a, b) in uncovered(all, Some(covered), y) {
                self.span(y, a, b).fill(color | 0xff00_0000);
            }
        }
    }

    /// 用 `background` 填充整个画布，并依次绘制 `rects` 中的矩形，后者覆盖前者；alpha 强制为
    /// 不透明。
    ///
    /// 结果与先 [`Self::fill`] 再逐个 [`Self::rect`] 相同，但缓冲按地址顺序只写一遍：矩形的
    /// 上下边将画布分为若干横带，带内各行相同，在 `row` 中生成一行后逐行复制。逐个填充时写入
    /// 地址不连续，实测（M4 Pro，2560 × 1640，三个矩形）耗时 0.95 ms，本方法 0.15 ms。
    pub fn fill_layered(&mut self, background: u32, rects: &[(IRect, u32)], row: &mut Vec<u32>) {
        if self.stride == 0 {
            return;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let n = (self.stride * self.height).min(self.px.len());
        row.clear();
        row.resize(self.stride, 0);

        let mut y = 0;
        while y < h {
            // 横带止于下一条矩形边界。
            let end = rects
                .iter()
                .flat_map(|(r, _)| [r.y0, r.y1])
                .filter(|&edge| edge > y)
                .fold(h, i32::min);
            row.fill(background | 0xff00_0000);
            for &(r, color) in rects {
                let (a, b) = (r.x0.clamp(0, w) as usize, r.x1.clamp(0, w) as usize);
                if r.y0 <= y && y < r.y1 && a < b {
                    row[a..b].fill(color | 0xff00_0000);
                }
            }
            let (from, to) = (y as usize * self.stride, end as usize * self.stride);
            for line in self.px[from.min(n)..to.min(n)].chunks_mut(self.stride) {
                line.copy_from_slice(&row[..line.len()]);
            }
            y = end;
        }
    }

    /// 与 [`Self::fill_layered`] 相同，但各处透出背景图：`background` 和 `rects` 中的颜色都是
    /// 加在图层像素上的值（见 [`Tinted::k`]）。只写图片的范围（[`Wall::span`]）以内的像素。
    ///
    /// 图层的分辨率低于画布时，相邻的若干行取自图层的同一行：只计算第一行，其余的逐行复制。
    pub fn wall_layered(
        &mut self,
        wall: Wall<'_>,
        background: u32,
        rects: &[(IRect, u32)],
        row: &mut Vec<u32>,
    ) {
        if self.stride == 0 {
            return;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let (width, stride) = (self.width, self.stride);
        row.clear();
        row.resize(width, 0);
        let span = wall.span;
        let (left, right) = (span.x0.clamp(0, w) as usize, span.x1.clamp(0, w) as usize);
        let h = span.y1.clamp(0, h);

        let mut y = span.y0.clamp(0, h);
        while y < h {
            let end = (rects.iter())
                .flat_map(|(r, _)| [r.y0, r.y1])
                .filter(|&edge| edge > y)
                .fold(h, i32::min);
            row.fill(background);
            for &(r, k) in rects {
                let (a, b) = (r.x0.clamp(0, w) as usize, r.x1.clamp(0, w) as usize);
                if r.y0 <= y && y < r.y1 && a < b {
                    row[a..b].fill(k);
                }
            }
            for line in y..end {
                let at = line as usize * stride;
                let source = wall.ymap[line as usize];
                if line > y && wall.ymap[line as usize - 1] == source {
                    (self.px).copy_within(at - stride + left..at - stride + right, at + left);
                    continue;
                }
                let from = &wall.px[source as usize * wall.width..][..wall.width];
                let to = &mut self.px[at + left..at + right];
                let (xmap, row) = (&wall.xmap[left..right], &row[left..right]);
                for ((p, &x), &k) in to.iter_mut().zip(xmap).zip(row) {
                    *p = from[x as usize] + k;
                }
            }
            y = end;
        }
    }

    /// 绘制实心矩形，跳过 `covered` 中的像素。
    ///
    /// `color` 不透明时直接覆盖，否则与底色混合；`tint` 不为 `None` 时写入的是透出背景图的颜色。
    pub fn rect(
        &mut self,
        area: IRect,
        color: u32,
        clip: IRect,
        covered: Option<Covered>,
        tint: Option<Tinted<'_>>,
    ) {
        let Some(v) = self.visible(area, clip) else {
            return;
        };
        let opaque = color >> 24 == 0xff;

        for y in v.y0..v.y1 {
            for (a, b) in uncovered(v, covered, y) {
                let row = self.span(y, a, b);

                if let Some(tint) = tint {
                    tint.write(y, a, row);
                } else if opaque {
                    row.fill(color);
                } else {
                    wide::mix_span(row, color, color >> 24);
                }
            }
        }
    }

    /// 在 `area` 中绘制棋盘格：边长 `cell`（物理像素）的方格，左上角的方格为 `first`，
    /// 与其相邻的为 `second`。
    ///
    /// 两种颜色都必须不透明（alpha 为 `0xff`）。每行按方格切分为多段直接填色：逐格按矩形绘制时
    /// 每格都要单独裁剪和换算坐标，大图上可达上万格。
    pub fn checker(&mut self, area: IRect, cell: usize, (first, second): (u32, u32), clip: IRect) {
        let Some(v) = self.visible(area, clip) else {
            return;
        };
        let cell = cell.max(1);
        for y in v.y0..v.y1 {
            let parity = (y - area.y0) as usize / cell;
            let row = self.span(y, v.x0, v.x1);
            let mut x = v.x0;
            while x < v.x1 {
                let col = (x - area.x0) as usize / cell;
                let end = (area.x0 + ((col + 1) * cell) as i32).min(v.x1);
                let color = if (col + parity).is_multiple_of(2) {
                    first
                } else {
                    second
                };
                row[(x - v.x0) as usize..(end - v.x0) as usize].fill(color);
                x = end;
            }
        }
    }

    /// 绘制圆角矩形，四个角的半径依次是左上、右上、右下、左下。
    ///
    /// 半径超过短边的一半时按一半计。圆角边缘按像素中心到圆弧的距离做抗锯齿。
    /// 跳过 `covered` 中的像素。
    pub fn rounded_rect(
        &mut self,
        area: IRect,
        radii: [f32; 4],
        color: u32,
        clip: IRect,
        covered: Option<Covered>,
        tint: Option<Tinted<'_>>,
    ) {
        let limit = ((area.x1 - area.x0).min(area.y1 - area.y0) as f32 / 2.0).max(0.0);
        let radii = radii.map(|r| r.clamp(0.0, limit));

        if radii.iter().all(|&r| r <= 0.5) {
            self.rect(area, color, clip, covered, tint);
            return;
        }

        let Some(v) = self.visible(area, clip) else {
            return;
        };
        let center = (
            (area.x0 + area.x1) as f32 / 2.0,
            (area.y0 + area.y1) as f32 / 2.0,
        );
        let opaque = color >> 24 == 0xff;
        // 距上下边都不小于最大半径的行整行为实心，其余行中距左右边都不小于最大半径的像素也为
        // 实心；只有四个角需要计算到圆弧的距离。
        let r_max = radii.iter().copied().fold(0.0, f32::max).ceil() as i32;

        for y in v.y0..v.y1 {
            let py = y as f32 + 0.5;
            let (solid0, solid1) = if y >= area.y0 + r_max && y < area.y1 - r_max {
                (i32::MIN, i32::MAX)
            } else {
                (area.x0 + r_max, area.x1 - r_max)
            };

            for (a, b) in uncovered(v, covered, y) {
                // 每段分为三部分：左角、实心部分、右角。矩形宽度小于两个半径之和时实心部分为空。
                let mid0 = solid0.clamp(a, b);
                let mid1 = solid1.clamp(mid0, b);
                let row = self.span(y, a, b);
                let (left, rest) = row.split_at_mut((mid0 - a) as usize);
                let (mid, right) = rest.split_at_mut((mid1 - mid0) as usize);

                if let Some(tint) = tint {
                    tint.write(y, mid0, mid);
                } else if opaque {
                    mid.fill(color);
                } else {
                    wide::mix_span(mid, color, color >> 24);
                }

                for (start, part) in [(a, left), (mid1, right)] {
                    for (i, p) in part.iter_mut().enumerate() {
                        let px = (start + i as i32) as f32 + 0.5;
                        let r = corner_radius(radii, px < center.0, py < center.1);
                        let d = rounded_distance(area, r, px, py);
                        let cov = ((0.5 - d).clamp(0.0, 1.0) * 255.0) as u32;

                        if cov > 0 {
                            let color = tint.map_or(color, |t| t.at(start + i as i32, y));
                            *p = blend(*p, color, cov);
                        }
                    }
                }
            }
        }
    }

    /// 把圆角矩形 `area` 四个角圆弧外侧的像素填充为 `color`，圆弧上的像素按覆盖比例混合。
    ///
    /// 半径超过短边的一半时按一半计。区域内的内容先按直角矩形绘制，再由本方法修整为圆角，
    /// 内容无需各自按圆角裁剪。
    pub fn round_corners(
        &mut self,
        area: IRect,
        radius: f32,
        color: u32,
        clip: IRect,
        tint: Option<Tinted<'_>>,
    ) {
        let limit = ((area.x1 - area.x0).min(area.y1 - area.y0) as f32 / 2.0).max(0.0);
        let r = radius.clamp(0.0, limit);
        if r <= 0.5 {
            return;
        }

        let n = r.ceil() as i32;
        let corners = [
            IRect::new(area.x0, area.y0, n, n),
            IRect::new(area.x1 - n, area.y0, n, n),
            IRect::new(area.x1 - n, area.y1 - n, n, n),
            IRect::new(area.x0, area.y1 - n, n, n),
        ];

        for corner in corners {
            let Some(v) = self.visible(corner, clip) else {
                continue;
            };

            for y in v.y0..v.y1 {
                let py = y as f32 + 0.5;

                for (i, p) in self.span(y, v.x0, v.x1).iter_mut().enumerate() {
                    let px = (v.x0 + i as i32) as f32 + 0.5;
                    let d = rounded_distance(area, r, px, py);
                    let cov = ((0.5 + d).clamp(0.0, 1.0) * 255.0) as u32;

                    if cov > 0 {
                        let color = tint.map_or(color, |t| t.at(v.x0 + i as i32, y));
                        *p = blend(*p, color, cov);
                    }
                }
            }
        }
    }

    /// 绘制圆角矩形的柔和阴影：边缘内外各 `blur` 像素按平滑曲线过渡，近似高斯模糊。
    ///
    /// 跳过 `covered` 中的像素。
    pub fn shadow(
        &mut self,
        area: IRect,
        radius: f32,
        blur: f32,
        color: u32,
        clip: IRect,
        covered: Option<Covered>,
    ) {
        let spread = blur.ceil() as i32;
        let outer = IRect {
            x0: area.x0 - spread,
            y0: area.y0 - spread,
            x1: area.x1 + spread,
            y1: area.y1 + spread,
        };
        let Some(v) = self.visible(outer, clip) else {
            return;
        };
        let r = radius
            .min((area.x1 - area.x0).min(area.y1 - area.y0) as f32 / 2.0)
            .max(0.0);
        let alpha = ((color >> 24) & 0xff) as f32;

        // 每个像素都参与计算、不提前跳过，使循环没有分支、便于向量化：距边缘超过 `blur`
        // 的像素透明度为 0，[`mix`] 按 0 叠加后像素不变。
        for y in v.y0..v.y1 {
            let py = y as f32 + 0.5;

            for (start, end) in uncovered(v, covered, y) {
                for (i, p) in self.span(y, start, end).iter_mut().enumerate() {
                    let d = rounded_distance(area, r, (start + i as i32) as f32 + 0.5, py);

                    // smoothstep 曲线：边缘处为一半浓度，向内 `blur` 处浓度最高，
                    // 向外 `blur` 处降为 0。
                    let u = ((d / blur + 1.0) / 2.0).clamp(0.0, 1.0);
                    let a = (alpha * (1.0 - u * u * (3.0 - 2.0 * u))) as u32;
                    *p = mix(*p, color, a);
                }
            }
        }
    }

    /// 绘制一组线段、圆周和圆弧。
    ///
    /// 形状坐标和笔宽 `width` 是逻辑像素，按 `scale` 换算为物理像素。
    ///
    /// 每个像素只取到最近一个形状的距离，因此交叉处只叠加一次颜色，半透明的图标不会在交点处变深。
    pub fn stroke(&mut self, shapes: &[Stroke], width: f32, scale: f32, color: u32, clip: IRect) {
        let Some(mut raster) = StrokeRaster::new(shapes, width, scale) else {
            return;
        };
        let Some(v) = self.visible(raster.area, clip) else {
            return;
        };

        for y in v.y0..v.y1 {
            let row = self.row(y, v);
            raster.row(y, (v.x0, v.x1), |x, cov| {
                let p = &mut row[(x - v.x0) as usize];
                *p = blend(*p, color, cov);
            });
        }
    }

    /// 绘制 8 位覆盖率蒙版（单色字形），按 `color` 着色后叠加，覆盖率经 `gamma` 映射。
    ///
    /// `area` 的尺寸即为蒙版 `data` 的尺寸。
    ///
    /// # Panics
    ///
    /// `data` 的长度小于 `area` 宽高之积，且可见部分超出 `data` 时 panic。
    pub fn mask(&mut self, area: IRect, data: &[u8], color: u32, gamma: &[u8; 256], clip: IRect) {
        let Some(v) = self.visible(area, clip) else {
            return;
        };
        let (w, cols) = (area.width(), v.width());

        for y in v.y0..v.y1 {
            let src = (y - area.y0) as usize * w + (v.x0 - area.x0) as usize;

            for (p, &m) in self.row(y, v).iter_mut().zip(&data[src..src + cols]) {
                let cov = gamma[m as usize] as u32;

                if cov > 0 {
                    *p = blend(*p, color, cov);
                }
            }
        }
    }

    /// 绘制 8 位覆盖率蒙版（单色字形），按 `color` 着色后叠加；覆盖率已是最终值，不再映射。
    ///
    /// `area` 的尺寸即蒙版 `data` 的尺寸，结果与以恒等映射表调用 [`Canvas::mask`] 相同。
    /// 循环内无分支、无查表：在覆盖率随机的 12×20 像素蒙版上实测每像素约 0.64 ns，
    /// 逐像素查表并判断覆盖率时约 1.48 ns。字形混色是每帧绘制中开销最大的一项。
    ///
    /// # Panics
    ///
    /// `data` 的长度小于 `area` 宽高之积，且可见部分超出 `data` 时 panic。
    pub fn coverage(&mut self, area: IRect, data: &[u8], color: u32, clip: IRect) {
        let alpha = (color >> 24) & 0xff;
        let Some(v) = self.visible(area, clip).filter(|_| alpha > 0) else {
            return;
        };
        let (w, cols) = (area.width(), v.width());

        for y in v.y0..v.y1 {
            let src = (y - area.y0) as usize * w + (v.x0 - area.x0) as usize;
            let pixels = self.row(y, v).iter_mut().zip(&data[src..src + cols]);

            if alpha == 255 {
                for (p, &m) in pixels {
                    *p = mix(*p, color, u32::from(m));
                }
            } else {
                for (p, &m) in pixels {
                    *p = mix(*p, color, div255(alpha * u32::from(m)));
                }
            }
        }
    }

    /// 绘制非预乘的 RGBA8 位图（彩色 emoji），整体乘以不透明度 `opacity`（0 至 255）。
    ///
    /// `area` 的尺寸即为位图 `data` 的尺寸。
    ///
    /// # Panics
    ///
    /// `data` 的长度小于 `area` 宽高之积的 4 倍，且可见部分超出 `data` 时 panic。
    pub fn rgba(&mut self, area: IRect, data: &[u8], opacity: u32, clip: IRect) {
        let Some(v) = self.visible(area, clip) else {
            return;
        };
        let (w, cols) = (area.width(), v.width());

        for y in v.y0..v.y1 {
            let src = ((y - area.y0) as usize * w + (v.x0 - area.x0) as usize) * 4;

            for (p, rgba) in self
                .row(y, v)
                .iter_mut()
                .zip(data[src..src + cols * 4].as_chunks::<4>().0)
            {
                let [r, g, b, a] = rgba.map(u32::from);
                *p = blend(*p, (a << 24) | (r << 16) | (g << 8) | b, opacity);
            }
        }
    }

    /// 把 `size`（宽，高）的非预乘 RGBA 位图 `data` 缩放到 `area` 混合到画布上，取最近的像素。
    ///
    /// # Panics
    ///
    /// `data` 的长度小于 `size` 宽高之积的 4 倍时 panic。
    pub fn rgba_scaled(
        &mut self,
        area: IRect,
        (width, height): (usize, usize),
        data: &[u8],
        opacity: u32,
        clip: IRect,
    ) {
        let Some(v) = self.visible(area, clip) else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }
        let (w, h) = (area.width(), (area.y1 - area.y0) as usize);
        // 横向按定点数步进，避免逐像素做除法。小数部分取 32 位：步进的舍入误差逐像素累积，
        // 16 位时放大到数万像素宽的位图在右端偏移可达一个源像素以上。步长向上取整：向下取整
        // 时，位于源像素边界上的目标像素会取到前一个源像素，与竖直方向的整数除法不一致。
        let step = ((width as u64) << 32).div_ceil(w as u64);
        let first = (v.x0 - area.x0) as u64 * step;
        for y in v.y0..v.y1 {
            let row = ((y - area.y0) as usize * height / h).min(height - 1);
            let line = &data[row * width * 4..(row + 1) * width * 4];
            let mut at = first;
            for p in self.row(y, v) {
                let x = ((at >> 32) as usize).min(width - 1) * 4;
                let [r, g, b, a] = [line[x], line[x + 1], line[x + 2], line[x + 3]].map(u32::from);
                *p = blend(*p, (a << 24) | (r << 16) | (g << 8) | b, opacity);
                at += step;
            }
        }
    }

    /// 填充凸多边形 `points`（物理像素，顺序不限方向）。
    ///
    /// 边缘按覆盖率抗锯齿：每行取四条扫描线，分别计算多边形在各扫描线上的左右端点，按横向
    /// 覆盖的长度累加。
    pub fn polygon(&mut self, points: &[(f32, f32)], color: u32, clip: IRect) {
        if points.len() < 3 {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let area = IRect {
            x0: x0.floor() as i32,
            y0: y0.floor() as i32,
            x1: x1.ceil() as i32,
            y1: y1.ceil() as i32,
        };
        let Some(v) = self.visible(area, clip) else {
            return;
        };
        let mut cover = vec![0f32; v.width()];
        for y in v.y0..v.y1 {
            cover.fill(0.0);
            for k in 0..4 {
                let sy = y as f32 + 0.125 + k as f32 * 0.25;
                // 凸多边形与扫描线的交点中，最左和最右的交点之间即为多边形在该扫描线上的范围。
                let (mut l, mut r) = (f32::MAX, f32::MIN);
                for (i, &(ax, ay)) in points.iter().enumerate() {
                    let (bx, by) = points[(i + 1) % points.len()];
                    if (ay <= sy) == (by <= sy) {
                        continue;
                    }
                    let x = ax + (sy - ay) / (by - ay) * (bx - ax);
                    (l, r) = (l.min(x), r.max(x));
                }
                if l >= r {
                    continue;
                }
                let (from, to) = ((l.floor() as i32).max(v.x0), (r.ceil() as i32).min(v.x1));
                for x in from..to {
                    let part = (r.min(x as f32 + 1.0) - l.max(x as f32)).clamp(0.0, 1.0);
                    cover[(x - v.x0) as usize] += part * 0.25;
                }
            }
            for (p, &c) in self.row(y, v).iter_mut().zip(&cover) {
                let cov = (c.min(1.0) * 255.0).round() as u32;
                if cov > 0 {
                    *p = blend(*p, color, cov);
                }
            }
        }
    }

    /// 返回 `area` 落在裁剪框和画布内的部分；为空时返回 `None`。
    fn visible(&self, area: IRect, clip: IRect) -> Option<IRect> {
        let canvas = IRect::new(0, 0, self.width as i32, self.height as i32);
        let v = area.intersect(clip).intersect(canvas);
        (!v.is_empty()).then_some(v)
    }

    fn row(&mut self, y: i32, v: IRect) -> &mut [u32] {
        self.span(y, v.x0, v.x1)
    }

    /// 返回第 `y` 行中 `x0..x1` 这一段像素。
    fn span(&mut self, y: i32, x0: i32, x1: i32) -> &mut [u32] {
        let start = y as usize * self.stride;
        &mut self.px[start + x0 as usize..start + x1 as usize]
    }
}

/// 不透明圆角矩形必然完全覆盖的像素范围。
///
/// 距上下边都不小于半径的行整行被覆盖，其余行去掉左右各一个半径，范围与 `Canvas::rounded_rect`
/// 绘制为实心的像素相同。先于该矩形绘制在其下方的阴影、边框和遮罩可以跳过这些像素，因为这些
/// 像素随后会被覆盖；大卡片上可以省去这部分像素的混合计算。
#[derive(Debug, Clone, Copy)]
pub struct Covered {
    /// 圆角矩形的范围（物理像素）。
    area: IRect,
    /// 圆角半径向上取整后的像素数，已夹取到短边的一半以内。
    r: i32,
}

impl Covered {
    /// 创建覆盖范围：`area` 是矩形，`radius` 是圆角半径（物理像素）。
    pub(crate) fn new(area: IRect, radius: f32) -> Self {
        let limit = ((area.x1 - area.x0).min(area.y1 - area.y0) as f32 / 2.0).max(0.0);
        Covered {
            area,
            r: radius.clamp(0.0, limit).ceil() as i32,
        }
    }

    /// 返回第 `y` 行被覆盖的列区间 `(x0, x1)`，左闭右开；没有时为空区间。
    fn span(self, y: i32) -> (i32, i32) {
        let a = self.area;
        if y < a.y0 || y >= a.y1 {
            (0, 0)
        } else if y >= a.y0 + self.r && y < a.y1 - self.r {
            (a.x0, a.x1)
        } else {
            (a.x0 + self.r, a.x1 - self.r)
        }
    }
}

/// 返回可见范围 `v` 的第 `y` 行中未被 `covered` 覆盖的部分：最多左右两段，空的段起止相同。
fn uncovered(v: IRect, covered: Option<Covered>, y: i32) -> [(i32, i32); 2] {
    let Some((c0, c1)) = covered.map(|c| c.span(y)).filter(|(c0, c1)| c0 < c1) else {
        return [(v.x0, v.x1), (v.x1, v.x1)];
    };
    let left_end = c0.clamp(v.x0, v.x1);
    let right_start = c1.clamp(left_end, v.x1);
    [(v.x0, left_end), (right_start, v.x1)]
}

#[cfg(test)]
mod tests;
