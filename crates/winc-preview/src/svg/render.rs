//! 把解析好的图形绘制为像素：画布为预乘 alpha 的 RGBA，按覆盖度叠加纯色、渐变和图案。
//!
//! 带有不透明度、裁剪、蒙版或滤镜的组先绘制在单独的一层上，处理完成后再叠加回去。每一层
//! 记录已绘制的区域（`touched`），叠加、裁剪、蒙版只处理这一块。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use zeno::{Command, Fill, Mask, Stroke};

use super::syntax::Affine;
use super::*;

/// 图案图块及其变换：图块坐标到画布像素的 [`Affine`] 六个系数。
type Tile = ([f32; 6], Rc<Layer>);

thread_local! {
 /// 当前这次 [`Drawing::render`] 中各图案最近绘制的图块，按图案地址查找，记录图块对应的
 /// 变换（图块坐标到画布像素）。
    /// （图块坐标到画布像素）。
    ///
 /// 一个图案常被数千个形状引用，变换相同时图块只绘制一次。每个图案只保留最近绘制的图块，
 /// 渲染结束时清空，不随绘图常驻内存。
    /// 渲染结束时清空，不随绘图常驻内存。
    static TILES: RefCell<HashMap<*const Pattern, Tile>> = RefCell::new(HashMap::new());
}

/// 画布上的一块矩形区域（像素，右边界和下边界不含）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Area {
    /// 左边界（像素列号，含）。
    pub x0: usize,
    /// 上边界（像素行号，含）。
    pub y0: usize,
    /// 右边界（像素列号，不含）。
    pub x1: usize,
    /// 下边界（像素行号，不含）。
    pub y1: usize,
}

impl Area {
    /// 空区域，`x0`、`y0` 取最大值，`x1`、`y1` 为 0；与任何区域求并集时结果等于对方。
    const EMPTY: Area = Area {
        x0: usize::MAX,
        y0: usize::MAX,
        x1: 0,
        y1: 0,
    };

    /// 是否为空区域（宽或高为 0）。
    pub(super) fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    /// 返回同时包含 `self` 和 `o` 的最小矩形；`o` 为空时返回 `self`。
    fn union(self, o: Area) -> Area {
        if o.is_empty() {
            return self;
        }
        Area {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    /// 返回 `self` 与 `o` 的交集；二者不相交时返回空区域。
    pub(super) fn intersect(self, o: Area) -> Area {
        Area {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }
}

/// 覆盖度缓冲的像素数相对画布像素数的最大倍数（倍）。
///
/// 路径外框对应的缓冲像素数超过画布像素数的该倍数（且超过 2^20 像素）时，只渲染画布内
/// 的部分。
/// 按外框直接渲染的缓冲可能极大：`<rect width="20000" height="20000">` 的覆盖度缓冲
/// 为 4 × 10^8 字节（约 400 MB）；宽高之积超过 `u32` 范围时，zeno 内部的 `u32` 乘法在 debug
/// 构建中会 panic。该值是经验值，作用范围为覆盖度缓冲的分配上限。
const MAX_AREA_SHARE: usize = 4;

/// 把路径渲染为覆盖度缓冲，返回缓冲及其在画布坐标中的位置。
///
/// 路径外框的像素数超过上限时，只渲染画布范围内的部分（范围之外本来也不可见）。上限取画布
/// 路径外框的像素数超过上限时只渲染画布内的部分，上限取画布像素数的 [`MAX_AREA_SHARE`]
/// 倍与 2^20 像素（1 048 576 像素）中的较大者；
/// 未超过上限时仍按外框渲染，像素结果不变。
fn render_mask<'a>(
    mask: &mut Mask<'a, '_, &'a Vec<Command>>,
    width: usize,
    height: usize,
) -> (Vec<u8>, zeno::Placement) {
    let mut natural = (0u32, 0u32);
    mask.inspect(|_, w, h| natural = (w, h));
    let area = (natural.0 as usize).saturating_mul(natural.1 as usize);
    if area
        > width
            .saturating_mul(height)
            .saturating_mul(MAX_AREA_SHARE)
            .max(1 << 20)
    {
        // 显式指定画布大小且不偏移：缓冲的第 (i, j) 个像素对应画布上的第 (i, j) 个像素，
        // 返回的 `placement.left`、`placement.top` 均为 0。
        // 返回的 `placement.left`、`placement.top` 均为 0。
        mask.size(width as u32, height as u32)
            .offset(zeno::Vector::ZERO);
    }
    mask.render()
}

/// 画布坐标的范围上限（像素），`device_path` 把路径坐标限制在 `-LIMIT` 到 `LIMIT` 之间。
///
/// zeno 的光栅化使用 24.8 定点数（8 位小数），`dx * fy1` 这类 `i32` 乘积在坐标达到
/// 2^15 = 32768 像素时会溢出（release 回绕，debug 构建 panic），描边还会向外扩出线宽。
/// 8192 像素是经验值：预览按编辑区的物理像素尺寸渲染，一般小于该值，被限制掉的部分
/// 本来就在画布之外。
/// 本来就在画布之外。
const LIMIT: f32 = 8192.0;

/// 光栅化时整张图长边的上限（像素），等于画布坐标的范围上限。
///
/// 不超过时，图内的坐标都在范围之内，按原样绘制；超过时，坐标超出范围的形状被限制到范围的
/// 边缘而变形。按显示大小光栅化的调用方据此限制大小，更大的显示大小改为缩放位图。
pub const MAX_EDGE: u32 = LIMIT as u32;

/// 一条描边最多绘制的虚线段数（段），超过时按实线描边。
///
/// 依据 Skia 的 `kMaxDashCount`。zeno 把短于 1 像素的虚线画成实线，但更长的虚线在由大量
/// 线段组成的路径上仍可以产生数百万段，每段单独生成轮廓：120 万段实测约 30 秒（release
/// 构建）。
const MAX_DASHES: f32 = 1_000_000.0;

/// 路径长度的上限估计（与坐标同单位），取各段起点、控制点、终点依次相连的折线长度，
/// 不小于曲线的实际长度。
/// 不小于曲线的实际长度。
fn path_length(path: &[Command]) -> f32 {
    let (mut length, mut at, mut start) = (0.0, zeno::Point::ZERO, zeno::Point::ZERO);
    let mut go = |to: zeno::Point, at: &mut zeno::Point| {
        length += (to - *at).length();
        *at = to;
    };
    for c in path {
        match *c {
            Command::MoveTo(p) => (at, start) = (p, p),
            Command::LineTo(p) => go(p, &mut at),
            Command::QuadTo(a, p) => [a, p].into_iter().for_each(|q| go(q, &mut at)),
            Command::CurveTo(a, b, p) => [a, b, p].into_iter().for_each(|q| go(q, &mut at)),
            Command::Close => go(start, &mut at),
        }
    }
    length
}

/// 返回变换到画布坐标后的路径，每个坐标限制在 `-LIMIT` 到 `LIMIT` 之间。
fn device_path(path: &[Command], t: Affine) -> Vec<Command> {
    map_points(path, |p| {
        let (x, y) = t.apply(p.x, p.y);
        zeno::Point::new(x.clamp(-LIMIT, LIMIT), y.clamp(-LIMIT, LIMIT))
    })
}

/// 预乘 alpha 的 RGBA 画布。
pub(super) struct Layer {
    /// 宽度（像素）。
    pub width: usize,
    /// 高度（像素）。
    pub height: usize,
    /// 按行排列的像素，每个像素是预乘的 `[R, G, B, A]`，长度为 `width * height`。
    pub px: Vec<[u8; 4]>,
    /// 已绘制过的区域；叠加、裁剪、蒙版只处理这一块。
    pub touched: Area,
}

impl Layer {
    /// 创建 `width` × `height` 像素的全透明画布。
    pub(super) fn new(width: usize, height: usize) -> Layer {
        Layer {
            width,
            height,
            // 全零内存通过 `alloc_zeroed` 申请，操作系统按需提交物理页；大画布上只绘制一小块时，
            // 不会立即占满内存。
            // 不会立即占满内存。
            px: vec![[0; 4]; width * height],
            touched: Area::EMPTY,
        }
    }

    /// 返回覆盖整个画布的区域。
    pub(super) fn full(&self) -> Area {
        Area {
            x0: 0,
            y0: 0,
            x1: self.width,
            y1: self.height,
        }
    }

    /// 返回覆盖度缓冲与画布相交的区域，以及 `placement` 的左、上偏移和缓冲每行的宽度
    ///（像素）；二者不相交时返回 `None`。
    /// 和缓冲每行的宽度（像素）；二者不相交时返回 `None`。
    fn clip_placement(&self, placement: zeno::Placement) -> Option<(Area, i32, i32, usize)> {
        let (left, top) = (placement.left, placement.top);
        let (mw, mh) = (placement.width as i32, placement.height as i32);
        let area = Area {
            x0: left.max(0) as usize,
            y0: top.max(0) as usize,
            x1: (left + mw).clamp(0, self.width as i32) as usize,
            y1: (top + mh).clamp(0, self.height as i32) as usize,
        };
        (!area.is_empty()).then_some((area, left, top, mw as usize))
    }

    /// 按覆盖度缓冲 `mask`（位置为 `placement`）把 `paint` 叠加到画布上。
    ///
    /// `to_pixels` 是从形状自身坐标到画布像素的变换。
    fn paint(&mut self, mask: &[u8], placement: zeno::Placement, paint: &Paint, to_pixels: Affine) {
        let Some((area, left, top, mw)) = self.clip_placement(placement) else {
            return;
        };
        let sampler = Sampler::new(paint, to_pixels);
        for y in area.y0..area.y1 {
            let row = (y as i32 - top) as usize * mw;
            for x in area.x0..area.x1 {
                let coverage = mask[row + (x as i32 - left) as usize];
                if coverage == 0 {
                    continue;
                }
                let src = sampler.at(x as i32, y as i32);
                let dst = &mut self.px[y * self.width + x];
                *dst = over(*dst, src, u32::from(coverage));
            }
        }
        self.touched = self.touched.union(area);
    }

    /// 把 `layer` 按不透明度 `alpha`（0 到 1）叠加到本层上。
    fn merge(&mut self, layer: &Layer, alpha: f32) {
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
        let area = layer.touched.intersect(self.full());
        if area.is_empty() || a == 0 {
            return;
        }
        for y in area.y0..area.y1 {
            let row = y * self.width;
            for x in area.x0..area.x1 {
                let src = layer.px[row + x];
                if src[3] > 0 {
                    let dst = &mut self.px[row + x];
                    *dst = over(*dst, src, a);
                }
            }
        }
        self.touched = self.touched.union(area);
    }

    /// 把 `layer` 按不透明度 `alpha` 和混合模式 `mode` 叠加到本层上。
    fn blend(&mut self, layer: &Layer, alpha: f32, mode: super::filter::Blend) {
        let area = layer.touched.intersect(self.full());
        if area.is_empty() {
            return;
        }
        let a = alpha.clamp(0.0, 1.0);
        for y in area.y0..area.y1 {
            let row = y * self.width;
            for x in area.x0..area.x1 {
                let src = layer.px[row + x];
                if src[3] == 0 {
                    continue;
                }
                let t = src.map(|c| f32::from(c) / 255.0 * a);
                let b = self.px[row + x].map(|c| f32::from(c) / 255.0);
                let out = super::filter::blend_pixel(t, b, mode);
                self.px[row + x] = out.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        self.touched = self.touched.union(area);
    }

    /// 把每个像素乘以 `coverage` 中对应的覆盖度（0 到 255，尺寸与画布相同）。
    fn multiply(&mut self, coverage: &[u8]) {
        let area = self.touched;
        if area.is_empty() {
            return;
        }
        for y in area.y0..area.y1 {
            let row = y * self.width;
            for x in area.x0..area.x1 {
                let k = u32::from(coverage[row + x]);
                if k < 255 {
                    let p = &mut self.px[row + x];
                    *p = p.map(|c| ((u32::from(c) * k + 127) / 255) as u8);
                }
            }
        }
    }
}

/// 返回 `src` 按 source-over 叠加到 `dst` 上的结果。
/// `src` 为预乘颜色，且先按 `coverage`/255 缩放；结果是预乘颜色。
fn over(dst: [u8; 4], src: [u8; 4], coverage: u32) -> [u8; 4] {
    let s = |c: u8| (u32::from(c) * coverage + 127) / 255;
    let sa = s(src[3]);
    let keep = 255 - sa;
    std::array::from_fn(|i| (s(src[i]) + (u32::from(dst[i]) * keep + 127) / 255).min(255) as u8)
}

enum Sampler<'a> {
    /// 纯色（预乘）。
    Solid([u8; 4]),
    /// 渐变，以及从画布像素坐标到渐变坐标的逆变换。
    Gradient(&'a Gradient, Affine),
    /// 已绘制好的图案图块，以及从画布像素到图块像素的变换。
    Pattern(Rc<Layer>, Affine),
}

impl<'a> Sampler<'a> {
    /// 创建 `paint` 的取色器。
    /// `to_pixels` 是从形状自身坐标到画布像素的变换。
    fn new(paint: &'a Paint, to_pixels: Affine) -> Sampler<'a> {
        match paint {
            Paint::Solid(c) => Sampler::Solid(premultiply(*c)),
            Paint::Gradient(g) => match g.transform.then(to_pixels).invert() {
                Some(inverse) => Sampler::Gradient(g, inverse),
                None => Sampler::Solid(g.table[255]),
            },
            Paint::Pattern(p) => pattern_sampler(p, to_pixels),
        }
    }

    /// 返回画布像素 (x, y) 处的预乘颜色。
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let (g, inverse) = match self {
            Sampler::Solid(c) => return *c,
            Sampler::Gradient(g, inverse) => (g, inverse),
            Sampler::Pattern(tile, inverse) => {
                let (u, v) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
                let tx = (u.floor() as isize).rem_euclid(tile.width as isize) as usize;
                let ty = (v.floor() as isize).rem_euclid(tile.height as isize) as usize;
                return tile.px[ty * tile.width + tx];
            }
        };
        let (px, py) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
        let t = if g.radial {
            radial_t(g.geometry, px, py)
        } else {
            let [x1, y1, x2, y2, ..] = g.geometry;
            let (dx, dy) = (x2 - x1, y2 - y1);
            let len = dx * dx + dy * dy;
            if len <= 0.0 {
                1.0
            } else {
                ((px - x1) * dx + (py - y1) * dy) / len
            }
        };
        let t = match g.spread {
            Spread::Pad => t.clamp(0.0, 1.0),
            Spread::Repeat => t.rem_euclid(1.0),
            Spread::Reflect => {
                let r = t.rem_euclid(2.0);
                if r > 1.0 { 2.0 - r } else { r }
            }
        };
        g.table[(t * 255.0).round() as usize]
    }
}

/// 创建图案的取色器：按图块在画布上的大小把图块绘制出来，之后按坐标对图块尺寸取模。
/// 每边尺寸限制在 1 到 2048 像素（经验值），避免图块过大时内存占用失控。
/// 内存占用失控）把图块绘制出来，之后按坐标对图块尺寸取模取色。
fn pattern_sampler(p: &Pattern, to_pixels: Affine) -> Sampler<'static> {
    let device = p.transform.then(to_pixels);
    let [a, b, c, d, ..] = device.0;
    let (sx, sy) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
    let tw = (p.size.0 * sx).round().clamp(1.0, 2048.0);
    let th = (p.size.1 * sy).round().clamp(1.0, 2048.0);
    let to_tile = Affine::scale(tw / p.size.0, th / p.size.1);
    let key = std::ptr::from_ref(p);
    let cached = TILES.with_borrow(|tiles| {
        tiles
            .get(&key)
            .filter(|(at, _)| *at == device.0)
            .map(|(_, tile)| Rc::clone(tile))
    });
    let tile = cached.unwrap_or_else(|| {
        let mut tile = Layer::new(tw as usize, th as usize);
        draw_items(&p.items, &mut tile, to_tile);
        if p.opacity < 1.0 {
            let k = (p.opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
            for px in &mut tile.px {
                *px = px.map(|v| ((u32::from(v) * k + 127) / 255) as u8);
            }
        }
        let tile = Rc::new(tile);
        TILES.with_borrow_mut(|tiles| tiles.insert(key, (device.0, Rc::clone(&tile))));
        tile
    });
    match to_tile
        .invert()
        .map(|inv| inv.then(device))
        .and_then(Affine::invert)
    {
        Some(inverse) => Sampler::Pattern(tile, inverse),
        None => Sampler::Solid([0; 4]),
    }
}

/// 返回点 (px, py) 在径向渐变中的参数 t。
///
/// 渐变由焦点圆 (fx, fy, fr) 过渡到外圆 (cx, cy, r)；点可能落在多个圆上时，取半径非负的
/// 圆中最大的 t。
fn radial_t([cx, cy, r, fx, fy, fr]: [f32; 6], px: f32, py: f32) -> f32 {
    let (cdx, cdy, dr) = (cx - fx, cy - fy, r - fr);
    let (pdx, pdy) = (px - fx, py - fy);
    let a = cdx * cdx + cdy * cdy - dr * dr;
    let b = pdx * cdx + pdy * cdy + fr * dr;
    let c = pdx * pdx + pdy * pdy - fr * fr;
    if a.abs() < 1e-6 {
        return if b.abs() < 1e-9 { 0.0 } else { c / (2.0 * b) };
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return 0.0;
    }
    let root = disc.sqrt();
    let (t1, t2) = ((b + root) / a, (b - root) / a);
    let ok = |t: f32| fr + t * dr >= 0.0;
    match (ok(t1.max(t2)), ok(t1.min(t2))) {
        (true, _) => t1.max(t2),
        (false, true) => t1.min(t2),
        _ => 0.0,
    }
}

impl Drawing {
    /// 把图形绘制为 `width` × `height` 像素的位图（非预乘的 RGBA）。
    ///
    /// `width` 或 `height` 为 0 时按 1 处理。
    pub fn render(&self, width: u32, height: u32) -> Pixels {
        let (width, height) = (width.max(1), height.max(1));
        self.render_region((width, height), [0, 0, width, height])
    }

    /// 绘制整张图为 `size`（宽、高，像素）时其中 `region` 部分的位图（非预乘的 RGBA）。
    ///
    /// `region` 依次为该部分左上角的横、纵坐标和宽、高（像素），结果的大小为其宽、高；`size`
    /// 与 `region` 的宽、高为 0 时按 1 处理。内存取决于 `region` 的大小，与 `size` 无关：放大后
    /// 只需要可见的部分时使用。`size` 的长边不应超过 [`MAX_EDGE`]。
    ///
    /// 滤镜只处理 `region` 内的像素：模糊等滤镜在 `region` 的边缘取不到外侧的内容，这些位置的
    /// 结果与绘制整张图后裁剪略有差别。
    pub fn render_region(&self, size: (u32, u32), [x, y, width, height]: [u32; 4]) -> Pixels {
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        let whole = (size.0.max(1) as f32, size.1.max(1) as f32);
        let viewport = fit(self.view_box, self.aspect, whole.0, whole.1)
            .then(Affine::translate(-(x as f32), -(y as f32)));
        let mut layer = Layer::new(w, h);
        draw_items(&self.items, &mut layer, viewport);
        TILES.with_borrow_mut(HashMap::clear);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for [r, g, b, a] in layer.px {
            if a == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let un =
                    |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
                rgba.extend_from_slice(&[un(r), un(g), un(b), a]);
            }
        }
        Pixels {
            width: w as u32,
            height: h as u32,
            rgba,
        }
    }
}

/// 按顺序把 `items` 绘制到 `layer` 上；`viewport` 是从图形坐标到画布像素的变换。
pub(super) fn draw_items(items: &[Item], layer: &mut Layer, viewport: Affine) {
    for item in items {
        match item {
            Item::Shape(shape) => draw_shape(shape, layer, viewport),
            Item::Image(image) => draw_image(image, layer, viewport),
            Item::Group(group) => draw_group(group, layer, viewport),
        }
    }
}

/// 绘制一个组：先画在单独的一层上，依次应用滤镜、裁剪和蒙版，再按不透明度和混合模式
/// 叠加回 `layer`。
fn draw_group(group: &Group, layer: &mut Layer, viewport: Affine) {
    let mut inner = Layer::new(layer.width, layer.height);
    draw_items(&group.items, &mut inner, viewport);
    if let Some(filter) = &group.filter {
        inner = filter.apply(inner, viewport);
    }
    if let Some(clip) = &group.clip {
        if inner.touched.is_empty() {
            return;
        }
        let coverage = clip_coverage(clip, viewport, layer.width, layer.height);
        inner.multiply(&coverage);
    }
    if let Some(mask) = &group.mask {
        if inner.touched.is_empty() {
            return;
        }
        let coverage = mask_coverage(mask, viewport, layer.width, layer.height);
        inner.multiply(&coverage);
    }
    if group.blend == super::filter::Blend::Normal {
        layer.merge(&inner, group.opacity);
    } else {
        layer.blend(&inner, group.opacity, group.blend);
    }
}

/// 返回裁剪路径对画布的覆盖度（0 到 255，尺寸与画布相同）。
///
/// 各形状的覆盖度按 `a + b - a·b` 合并，再乘以嵌套的 `clip.clip`（如有）。
fn clip_coverage(clip: &Clip, viewport: Affine, width: usize, height: usize) -> Vec<u8> {
    let mut coverage = vec![0u8; width * height];
    for (path, transform, rule) in &clip.shapes {
        let to_pixels = transform.then(viewport);
        let device = device_path(path, to_pixels);
        let (mask, placement) = render_mask(Mask::new(&device).style(*rule), width, height);
        add_coverage(&mut coverage, width, height, &mask, placement);
    }
    if let Some(inner) = &clip.clip {
        let other = clip_coverage(inner, viewport, width, height);
        for (c, o) in coverage.iter_mut().zip(other) {
            *c = ((u32::from(*c) * u32::from(o) + 127) / 255) as u8;
        }
    }
    coverage
}

/// 把 `mask` 的覆盖度并入 `coverage`：结果为 a + b - a·b（a、b 按 0 到 1 归一化）。
fn add_coverage(
    coverage: &mut [u8],
    width: usize,
    height: usize,
    mask: &[u8],
    placement: zeno::Placement,
) {
    let (left, top) = (placement.left, placement.top);
    let mw = placement.width as usize;
    for my in 0..placement.height as usize {
        let y = top + my as i32;
        if y < 0 || y >= height as i32 {
            continue;
        }
        for mx in 0..mw {
            let x = left + mx as i32;
            if x < 0 || x >= width as i32 {
                continue;
            }
            let m = u32::from(mask[my * mw + mx]);
            if m == 0 {
                continue;
            }
            let c = &mut coverage[y as usize * width + x as usize];
            let a = u32::from(*c);
            *c = (a + m - (a * m + 127) / 255).min(255) as u8;
        }
    }
}

/// 返回蒙版对画布的保留程度（0 到 255，尺寸与画布相同）。
///
/// 先绘制蒙版内容，按亮度（`luminance` 为 `true`）或透明度计算，再乘以蒙版范围 `region`
/// 的覆盖度；
/// 最后乘以嵌套的 `mask.mask`（如有）。
fn mask_coverage(mask: &MaskRef, viewport: Affine, width: usize, height: usize) -> Vec<u8> {
    let mut content = Layer::new(width, height);
    draw_items(&mask.items, &mut content, viewport);
    let region = clip_coverage(&mask.region, viewport, width, height);
    let mut coverage = vec![0u8; width * height];
    let area = content.touched;
    if !area.is_empty() {
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let i = y * width + x;
                let [r, g, b, a] = content.px[i];
                // 画布像素是预乘的，由预乘分量算出的亮度已经包含 alpha。亮度系数取自
                // SVG 1.1 第 15.10 节的 `luminanceToAlpha`，蒙版按第 14.4 节套用；不处理蒙版的
                // `color-interpolation: linearRGB`，按默认的 sRGB 计算。
                let value = if mask.luminance {
                    0.2125 * f32::from(r) + 0.7154 * f32::from(g) + 0.0721 * f32::from(b)
                } else {
                    f32::from(a)
                };
                let v = value.round().clamp(0.0, 255.0) as u32;
                coverage[i] = ((v * u32::from(region[i]) + 127) / 255) as u8;
            }
        }
    }
    if let Some(inner) = &mask.mask {
        let other = mask_coverage(inner, viewport, width, height);
        for (c, o) in coverage.iter_mut().zip(other) {
            *c = ((u32::from(*c) * u32::from(o) + 127) / 255) as u8;
        }
    }
    coverage
}

/// 绘制一个形状：填充与描边分别光栅化，按 `paint-order` 决定的先后叠加。
fn draw_shape(shape: &Shape, layer: &mut Layer, viewport: Affine) {
    let to_pixels = shape.transform.then(viewport);
    // 形状至少有填充或描边之一，两者共用变换到画布坐标的路径。
    let device = device_path(&shape.path, to_pixels);
    let fill = |layer: &mut Layer| {
        if let Some((paint, rule)) = &shape.fill {
            let (mask, placement) =
                render_mask(Mask::new(&device).style(*rule), layer.width, layer.height);
            layer.paint(&mask, placement, paint, to_pixels);
        }
    };
    let stroke = |layer: &mut Layer| {
        if let Some((paint, style)) = &shape.stroke {
            // 先把路径变换到画布坐标再描边，使坐标能限制在 zeno 可处理的范围内（见 `LIMIT`）。
            // 线宽需同步缩放；`vector-effect: non-scaling-stroke` 时线宽按画布像素计，
            // 不乘缩放系数。
            let [a, b, c, d, ..] = to_pixels.0;
            let scale = (a * a + b * b).sqrt().max((c * c + d * d).sqrt()).max(1e-6);
            let factor = if style.non_scaling { 1.0 } else { scale };
            // 虚线长度同样以形状自身坐标表示，与线宽一样换算到画布像素。
            let dashes: Vec<f32> = style.dashes.iter().map(|d| d * factor).collect();
            let mut stroke = Stroke::new(style.width * factor);
            stroke
                .cap(style.cap)
                .join(style.join)
                .miter_limit(style.miter);
            let period: f32 = dashes.iter().sum();
            if !dashes.is_empty() && path_length(&device) / period <= MAX_DASHES {
                stroke.dash(&dashes, style.offset * factor);
            }
            let (mask, placement) =
                render_mask(Mask::new(&device).style(stroke), layer.width, layer.height);
            layer.paint(&mask, placement, paint, to_pixels);
        }
    };
    if shape.stroke_first {
        stroke(layer);
        fill(layer);
    } else {
        fill(layer);
        stroke(layer);
    }
}

/// 绘制位图：边缘按图片矩形计算覆盖度，内部双线性采样。
///
/// 缩放系数小于 0.5 且不是 `pixelated` 时，先把图片缩小到目标大小再采样，避免只采到几个
/// 稀疏的像素。
fn draw_image(image: &ImageItem, layer: &mut Layer, viewport: Affine) {
    let mut to_pixels = image.transform.then(viewport);
    let (iw, ih) = (image.pixels.width as f32, image.pixels.height as f32);
    let [a, b, c, d, ..] = to_pixels.0;
    let (sx, sy) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
    let mut source = std::borrow::Cow::Borrowed(&*image.pixels);
    if (sx < 0.5 || sy < 0.5) && !image.pixelated {
        let (tw, th) = (
            (iw * sx).ceil().max(1.0) as u32,
            (ih * sy).ceil().max(1.0) as u32,
        );
        let small = crate::image::resize(&image.pixels, tw, th);
        let (kx, ky) = (iw / tw as f32, ih / th as f32);
        to_pixels = Affine::scale(kx, ky).then(to_pixels);
        source = std::borrow::Cow::Owned(small);
    }
    let Some(inverse) = to_pixels.invert() else {
        return;
    };
    let (w, h) = (source.width as usize, source.height as usize);
    let mut rect = Vec::new();
    rounded_rect(&mut rect, 0.0, 0.0, w as f32, h as f32, 0.0, 0.0);
    let device = device_path(&rect, to_pixels);
    let (mask, placement) = render_mask(
        Mask::new(&device).style(Fill::NonZero),
        layer.width,
        layer.height,
    );
    let Some((area, left, top, mw)) = layer.clip_placement(placement) else {
        return;
    };
    let texel = |x: isize, y: isize| -> [f32; 4] {
        let (x, y) = (
            x.clamp(0, w as isize - 1) as usize,
            y.clamp(0, h as isize - 1) as usize,
        );
        let p = &source.rgba[(y * w + x) * 4..][..4];
        let alpha = f32::from(p[3]) / 255.0;
        [
            f32::from(p[0]) * alpha,
            f32::from(p[1]) * alpha,
            f32::from(p[2]) * alpha,
            f32::from(p[3]),
        ]
    };
    for y in area.y0..area.y1 {
        let row = (y as i32 - top) as usize * mw;
        for x in area.x0..area.x1 {
            let coverage = mask[row + (x as i32 - left) as usize];
            if coverage == 0 {
                continue;
            }
            let (u, v) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let px = if image.pixelated {
                texel(u.floor() as isize, v.floor() as isize)
            } else {
                let (u, v) = (u - 0.5, v - 0.5);
                let (x0, y0) = (u.floor(), v.floor());
                let (fx, fy) = (u - x0, v - y0);
                let (x0, y0) = (x0 as isize, y0 as isize);
                let (p00, p10, p01, p11) = (
                    texel(x0, y0),
                    texel(x0 + 1, y0),
                    texel(x0, y0 + 1),
                    texel(x0 + 1, y0 + 1),
                );
                std::array::from_fn(|i| {
                    let top = p00[i] + (p10[i] - p00[i]) * fx;
                    let bottom = p01[i] + (p11[i] - p01[i]) * fx;
                    top + (bottom - top) * fy
                })
            };
            let src = px.map(|c| c.round().clamp(0.0, 255.0) as u8);
            let dst = &mut layer.px[y * layer.width + x];
            *dst = over(*dst, src, u32::from(coverage));
        }
    }
    layer.touched = layer.touched.union(area);
}
