//! 滤镜原语的计算：模糊、偏移、填色、合成、混合、颜色矩阵、分量变换、膨胀腐蚀。

use super::*;

/// 返回矩形的像素范围：先按 `device` 变换四个角，取外接矩形后限制在画布内。
pub(super) fn device_area(rect: [f32; 4], device: Affine, width: usize, height: usize) -> Area {
    let [x, y, w, h] = rect;
    let corners =
        [(x, y), (x + w, y), (x, y + h), (x + w, y + h)].map(|(px, py)| device.apply(px, py));
    let min_x = corners.iter().map(|c| c.0).fold(f32::MAX, f32::min);
    let min_y = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min);
    let max_x = corners.iter().map(|c| c.0).fold(f32::MIN, f32::max);
    let max_y = corners.iter().map(|c| c.1).fold(f32::MIN, f32::max);
    Area {
        x0: min_x.floor().clamp(0.0, width as f32) as usize,
        y0: min_y.floor().clamp(0.0, height as f32) as usize,
        x1: max_x.ceil().clamp(0.0, width as f32) as usize,
        y1: max_y.ceil().clamp(0.0, height as f32) as usize,
    }
}

/// 生成 `w` × `h` 像素的纯色图像。
/// 不预乘的 sRGB 颜色在 `linear` 为 `true` 时转换到线性空间，然后预乘透明度。
pub(super) fn flood(w: usize, h: usize, [r, g, b, a]: [f32; 4], linear: bool) -> Image {
    let convert = |c: f32| if linear { to_linear(c) } else { c };
    let color = [convert(r) * a, convert(g) * a, convert(b) * a, a];
    Image {
        w,
        h,
        px: vec![color; w * h],
        linear,
    }
}

/// 返回高斯模糊后的图像，横向和纵向分开计算。
/// `sx`、`sy` 分别是两个方向的标准差（像素）；两者都不大于 0 时返回 `img` 的副本。
///
/// 标准差不小于 2 时用三次方框模糊近似，这是 SVG 滤镜规范给出的做法。
pub(super) fn blur(img: &Image, sx: f32, sy: f32) -> Image {
    if sx <= 0.0 && sy <= 0.0 {
        return img.clone();
    }
    let mut out = img.clone();
    if sx > 0.0 {
        blur_axis(&mut out, sx, true);
    }
    if sy > 0.0 {
        blur_axis(&mut out, sy, false);
    }
    out
}

fn blur_axis(img: &mut Image, sigma: f32, horizontal: bool) {
    // 标准差超过图的尺寸后，每个像素都近似整张图的平均，再增大没有区别，但方框窗口会
    // 随标准差增大，因此夹取到图的尺寸。
    // 增大（`stdDeviation` 为 1e9 时每个像素要遍历十亿量级的格子），因此夹取到图的尺寸。
    let sigma = sigma.min(img.w.max(img.h) as f32);
    // 标准差小于 2 时直接用高斯核卷积；否则按规范用三次方框模糊近似：`d` 为奇数时三次
    // 窗口都居中，为偶数时前两次分别向左、向右偏半格，第三次窗口宽 `d + 1`。
    // 为偶数时前两次分别向左、向右偏半格，第三次窗口宽 `d + 1`。
    let kernel: Option<(isize, Vec<f32>)> = (sigma < 2.0).then(|| {
        let radius = (sigma * 3.0).ceil() as isize;
        let k: Vec<f32> = (-radius..=radius)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let total: f32 = k.iter().sum();
        (radius, k.into_iter().map(|v| v / total).collect())
    });
    let d = (sigma * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0 + 0.5)
        .floor()
        .max(1.0) as usize;
    let passes: [(usize, usize); 3] = if d % 2 == 1 {
        [(d / 2, d / 2); 3]
    } else {
        [(d / 2, d / 2 - 1), (d / 2 - 1, d / 2), (d / 2, d / 2)]
    };
    let (w, h) = (img.w, img.h);
    if horizontal {
        // 每一行的方框累加独立计算，窗口在行尾处按透明像素补齐。
        par_rows(&mut img.px, w, |_, chunk| {
            let mut tmp = vec![[0.0f32; 4]; w];
            for row in chunk.chunks_exact_mut(w) {
                match &kernel {
                    Some((radius, k)) => {
                        for (i, t) in tmp.iter_mut().enumerate() {
                            let mut acc = [0.0f32; 4];
                            for (n, weight) in k.iter().enumerate() {
                                let j = i as isize + n as isize - radius;
                                if j >= 0 && (j as usize) < w {
                                    let p = row[j as usize];
                                    for c in 0..4 {
                                        acc[c] += p[c] * weight;
                                    }
                                }
                            }
                            *t = acc;
                        }
                        row.copy_from_slice(&tmp);
                    }
                    None => {
                        for &(left, right) in &passes {
                            box_line(row, &mut tmp, left, right);
                            row.copy_from_slice(&tmp);
                        }
                    }
                }
            }
        });
        return;
    }
    // 纵向：每一遍从上一遍的结果读取，按行分段并行写入。`(left, right)` 是窗口向上、向下的行数；
    // `weights` 有值时按高斯核加权，为 `None` 时是方框平均。上一遍的结果换到 `scratch` 中读取，
    // 这一遍写回 `img.px`：两种算法都会写满每个像素，`img.px` 里换过来的旧内容不会被读到。
    let mut scratch = vec![[0.0f32; 4]; img.px.len()];
    let mut run = |(left, right): (isize, isize), weights: Option<&[f32]>| {
        std::mem::swap(&mut img.px, &mut scratch);
        let src = &scratch;
        par_rows(&mut img.px, w, |first, chunk| {
            let rows = chunk.len() / w;
            match weights {
                Some(k) => {
                    let lo = -left;
                    for r in 0..rows {
                        let y = first + r;
                        let out = &mut chunk[r * w..(r + 1) * w];
                        out.fill([0.0; 4]);
                        for (n, weight) in k.iter().enumerate() {
                            let sy = y as isize + lo + n as isize;
                            if sy < 0 || sy >= h as isize {
                                continue;
                            }
                            let line = &src[sy as usize * w..(sy as usize + 1) * w];
                            for (o, p) in out.iter_mut().zip(line) {
                                for c in 0..4 {
                                    o[c] += p[c] * weight;
                                }
                            }
                        }
                    }
                }
                None => {
                    let width = (left + right + 1) as f32;
                    // 窗口 [y - left, y + right] 内各列的和，随 y 增大向下滑动。
                    let mut acc = vec![[0.0f32; 4]; w];
                    for sy in first as isize - left..=first as isize + right {
                        if sy >= 0 && sy < h as isize {
                            for (a, p) in acc
                                .iter_mut()
                                .zip(&src[sy as usize * w..(sy as usize + 1) * w])
                            {
                                for c in 0..4 {
                                    a[c] += p[c];
                                }
                            }
                        }
                    }
                    for r in 0..rows {
                        let y = (first + r) as isize;
                        let out = &mut chunk[r * w..(r + 1) * w];
                        for (o, a) in out.iter_mut().zip(&acc) {
                            *o = a.map(|v| v / width);
                        }
                        let (add, sub) = (y + right + 1, y - left);
                        if add < h as isize {
                            for (a, p) in acc
                                .iter_mut()
                                .zip(&src[add as usize * w..(add as usize + 1) * w])
                            {
                                for c in 0..4 {
                                    a[c] += p[c];
                                }
                            }
                        }
                        if sub >= 0 {
                            for (a, p) in acc
                                .iter_mut()
                                .zip(&src[sub as usize * w..(sub as usize + 1) * w])
                            {
                                for c in 0..4 {
                                    a[c] -= p[c];
                                }
                            }
                        }
                    }
                }
            }
        });
    };
    match &kernel {
        Some((radius, k)) => run((*radius, *radius), Some(k)),
        None => {
            for &(left, right) in &passes {
                run((left as isize, right as isize), None);
            }
        }
    }
}

/// 返回一行方框模糊的结果：窗口为 [i - left, i + right]，超出行外的部分按透明像素计。
fn box_line(src: &[[f32; 4]], out: &mut [[f32; 4]], left: usize, right: usize) {
    let len = src.len();
    let width = (left + right + 1) as f32;
    let mut acc = [0.0f32; 4];
    for p in &src[..(right + 1).min(len)] {
        for c in 0..4 {
            acc[c] += p[c];
        }
    }
    for i in 0..len {
        out[i] = acc.map(|v| v / width);
        let add = i + right + 1;
        if add < len {
            for c in 0..4 {
                acc[c] += src[add][c];
            }
        }
        if i >= left {
            for c in 0..4 {
                acc[c] -= src[i - left][c];
            }
        }
    }
}

/// 返回平移后的图像，位移的小数部分按双线性插值。
pub(super) fn offset(img: &Image, dx: f32, dy: f32) -> Image {
    let mut out = Image::empty(img.w, img.h, img.linear);
    // 位移完全移出图外时结果为空：把位移夹取到图的尺寸加 1 以内，后续的 `floor` 转换和
    // 下标加减才不会溢出。
    // 减法才不会溢出。
    let limit = |v: f32, size: usize| {
        let size = size as f32 + 1.0;
        if v.is_nan() {
            0.0
        } else {
            v.clamp(-size, size)
        }
    };
    let (dx, dy) = (limit(dx, img.w), limit(dy, img.h));
    let (fx, fy) = (dx - dx.floor(), dy - dy.floor());
    let (ix, iy) = (dx.floor() as isize, dy.floor() as isize);
    let get = |x: isize, y: isize| -> [f32; 4] {
        if x < 0 || y < 0 || x >= img.w as isize || y >= img.h as isize {
            [0.0; 4]
        } else {
            img.px[y as usize * img.w + x as usize]
        }
    };
    let w = img.w;
    par_rows(&mut out.px, w, |first, chunk| {
        for (r, row) in chunk.chunks_exact_mut(w).enumerate() {
            let y = (first + r) as isize;
            for (x, o) in row.iter_mut().enumerate() {
                let (sx, sy) = (x as isize - ix, y - iy);
                let (p00, p10, p01, p11) = (
                    get(sx, sy),
                    get(sx - 1, sy),
                    get(sx, sy - 1),
                    get(sx - 1, sy - 1),
                );
                *o = std::array::from_fn(|c| {
                    let top = p00[c] * (1.0 - fx) + p10[c] * fx;
                    let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
                    top * (1.0 - fy) + bottom * fy
                });
            }
        }
    });
    out
}

/// 返回把 `top` 按 Porter-Duff 操作 `op` 合成到 `bottom` 之后的图像。
pub(super) fn composite(bottom: &mut Image, top: &Image, op: Composite) {
    let w = bottom.w;
    par_rows(&mut bottom.px, w, |first, chunk| {
        let top = &top.px[first * w..first * w + chunk.len()];
        composite_span(chunk, top, op);
    });
}

fn composite_span(bottom: &mut [[f32; 4]], top: &[[f32; 4]], op: Composite) {
    for (b, t) in bottom.iter_mut().zip(top) {
        let (ab, at) = (b[3], t[3]);
        *b = match op {
            Composite::Over => std::array::from_fn(|c| t[c] + b[c] * (1.0 - at)),
            Composite::In => t.map(|v| v * ab),
            Composite::Out => t.map(|v| v * (1.0 - ab)),
            Composite::Atop => std::array::from_fn(|c| t[c] * ab + b[c] * (1.0 - at)),
            Composite::Xor => std::array::from_fn(|c| t[c] * (1.0 - ab) + b[c] * (1.0 - at)),
            Composite::Lighter => std::array::from_fn(|c| (t[c] + b[c]).min(1.0)),
            Composite::Arithmetic([k1, k2, k3, k4]) => {
                // 系数为 NaN，或算出 `inf * 0` 时，`clamp` 与 `min`/`max` 的结果都是 NaN 或 panic：
                // 此时按全透明处理。
                // `NaN`，或算出 `inf * 0` 时都会出现 NaN。此时按全透明处理。
                let finite = |v: f32| if v.is_nan() { 0.0 } else { v };
                let a = finite(k1 * at * ab + k2 * at + k3 * ab + k4).clamp(0.0, 1.0);
                let mut out = [0.0; 4];
                for c in 0..3 {
                    out[c] = finite(k1 * t[c] * b[c] + k2 * t[c] + k3 * b[c] + k4).clamp(0.0, a);
                }
                out[3] = a;
                out
            }
        };
    }
}

/// 返回 `top` 按混合模式 `mode` 混合到 `bottom` 上的新图像。
pub(super) fn blend(top: &Image, bottom: &Image, mode: Blend) -> Image {
    let mut out = bottom.clone();
    let w = out.w;
    par_rows(&mut out.px, w, |first, chunk| {
        let top = &top.px[first * w..first * w + chunk.len()];
        for (o, t) in chunk.iter_mut().zip(top) {
            *o = blend_pixel(*t, *o, mode);
        }
    });
    out
}

/// 返回 `mix-blend-mode` 的值或 `feBlend` 的 `mode` 属性对应的混合模式。
/// 无法识别的值返回 `Blend::Normal`。
/// `Blend::Normal`。
pub(in crate::svg) fn blend_mode(value: &str) -> Blend {
    match value.trim() {
        "multiply" => Blend::Multiply,
        "screen" => Blend::Screen,
        "overlay" => Blend::Overlay,
        "darken" => Blend::Darken,
        "lighten" => Blend::Lighten,
        "color-dodge" => Blend::ColorDodge,
        "color-burn" => Blend::ColorBurn,
        "hard-light" => Blend::HardLight,
        "soft-light" => Blend::SoftLight,
        "difference" => Blend::Difference,
        "exclusion" => Blend::Exclusion,
        _ => Blend::Normal,
    }
}

/// 返回预乘的 `t`（上层）按 `mode` 混合到预乘的 `b`（下层）上的结果，结果同样是预乘的。
///
/// 公式取自 W3C Compositing and Blending 规范。
pub(in crate::svg) fn blend_pixel(t: [f32; 4], b: [f32; 4], mode: Blend) -> [f32; 4] {
    let (at, ab) = (t[3], b[3]);
    let un = |p: &[f32; 4], c: usize| if p[3] > 0.0 { p[c] / p[3] } else { 0.0 };
    let mut result = [0.0; 4];
    for c in 0..3 {
        let (cs, cb) = (un(&t, c), un(&b, c));
        let mixed = match mode {
            Blend::Normal => cs,
            Blend::Multiply => cs * cb,
            Blend::Screen => cs + cb - cs * cb,
            Blend::Overlay => hard_light(cb, cs),
            Blend::Darken => cs.min(cb),
            Blend::Lighten => cs.max(cb),
            Blend::ColorDodge => {
                if cb == 0.0 {
                    0.0
                } else if cs >= 1.0 {
                    1.0
                } else {
                    (cb / (1.0 - cs)).min(1.0)
                }
            }
            Blend::ColorBurn => {
                if cb >= 1.0 {
                    1.0
                } else if cs <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - cb) / cs).min(1.0)
                }
            }
            Blend::HardLight => hard_light(cs, cb),
            Blend::SoftLight => {
                if cs <= 0.5 {
                    cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
                } else {
                    let d = if cb <= 0.25 {
                        ((16.0 * cb - 12.0) * cb + 4.0) * cb
                    } else {
                        cb.sqrt()
                    };
                    cb + (2.0 * cs - 1.0) * (d - cb)
                }
            }
            Blend::Difference => (cs - cb).abs(),
            Blend::Exclusion => cs + cb - 2.0 * cs * cb,
        };
        result[c] = (1.0 - ab) * t[c] + (1.0 - at) * b[c] + at * ab * mixed;
    }
    result[3] = at + ab - at * ab;
    result
}

fn hard_light(cs: f32, cb: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

/// 返回对每个像素应用颜色矩阵 `m` 之后的图像；矩阵作用在不预乘的颜色上。
pub(super) fn color_matrix(img: &Image, m: &[f32; 20]) -> Image {
    let mut out = img.clone();
    let w = out.w;
    par_rows(&mut out.px, w, |_, chunk| color_matrix_span(chunk, m));
    out
}

fn color_matrix_span(px: &mut [[f32; 4]], m: &[f32; 20]) {
    for p in px {
        let a = p[3];
        let [r, g, b] = if a > 0.0 {
            [p[0] / a, p[1] / a, p[2] / a]
        } else {
            [0.0; 3]
        };
        let row = |i: usize| {
            (m[i * 5] * r + m[i * 5 + 1] * g + m[i * 5 + 2] * b + m[i * 5 + 3] * a + m[i * 5 + 4])
                .clamp(0.0, 1.0)
        };
        let na = row(3);
        *p = [row(0) * na, row(1) * na, row(2) * na, na];
    }
}

/// 返回对每个像素应用分量变换 `funcs` 之后的图像；`funcs` 依次对应 R、G、B、A 四个通道。
pub(super) fn transfer(img: &Image, funcs: &[Transfer; 4]) -> Image {
    let apply = |f: &Transfer, v: f32| -> f32 {
        let v = match f {
            Transfer::Identity => v,
            Transfer::Table(t) => {
                let n = t.len() - 1;
                if n == 0 {
                    t[0]
                } else {
                    let k = ((v * n as f32).floor() as usize).min(n - 1);
                    let local = v * n as f32 - k as f32;
                    t[k] + (t[k + 1] - t[k]) * local
                }
            }
            Transfer::Discrete(t) => t[((v * t.len() as f32).floor() as usize).min(t.len() - 1)],
            Transfer::Linear(slope, intercept) => slope * v + intercept,
            Transfer::Gamma(amplitude, exponent, offset) => amplitude * v.powf(*exponent) + offset,
        };
        v.clamp(0.0, 1.0)
    };
    let mut out = img.clone();
    let w = out.w;
    par_rows(&mut out.px, w, |_, chunk| {
        for p in chunk {
            let a = p[3];
            let un = |c: f32| if a > 0.0 { c / a } else { 0.0 };
            let na = apply(&funcs[3], a);
            *p = [
                apply(&funcs[0], un(p[0])) * na,
                apply(&funcs[1], un(p[1])) * na,
                apply(&funcs[2], un(p[2])) * na,
                na,
            ];
        }
    });
    out
}

/// 返回腐蚀（`dilate` 为 `false`）或膨胀（`dilate` 为 `true`）后的图像。
///
/// 横向和纵向分开计算，窗口为 [x - r, x + r]，图外按透明像素计。采用 van Herk / Gil-Werman
/// 算法：把一行按窗口宽度分段，每段从左到右、从右到左各计算一遍累计最值，任何窗口都恰好跨
/// 两段，取这两个值比较一次即可，每个像素的比较次数与半径无关。
pub(super) fn morphology(img: &Image, dilate: bool, rx: usize, ry: usize) -> Image {
    let pick = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] {
        std::array::from_fn(|c| {
            if dilate {
                a[c].max(b[c])
            } else {
                a[c].min(b[c])
            }
        })
    };
    let (w, h) = (img.w, img.h);
    let mut out = img.clone();
    // 半径超过图的尺寸时窗口总是伸到图外，结果与半径等于图的尺寸时相同。
    for (r, horizontal) in [(rx.min(w), true), (ry.min(h), false)] {
        if r == 0 {
            continue;
        }
        let (lines, len) = if horizontal { (h, w) } else { (w, h) };
        let at = |line: usize, i: usize| {
            if horizontal {
                line * w + i
            } else {
                i * w + line
            }
        };
        // 两端各补 `r` 个透明像素，窗口就不会越界。
        let (k, padded) = (2 * r + 1, len + 2 * r);
        let mut line_px = vec![[0.0f32; 4]; padded];
        let mut forward = vec![[0.0f32; 4]; padded];
        let mut backward = vec![[0.0f32; 4]; padded];
        for line in 0..lines {
            for i in 0..len {
                line_px[r + i] = out.px[at(line, i)];
            }
            for i in 0..padded {
                forward[i] = if i % k == 0 {
                    line_px[i]
                } else {
                    pick(forward[i - 1], line_px[i])
                };
            }
            for i in (0..padded).rev() {
                backward[i] = if i + 1 == padded || (i + 1) % k == 0 {
                    line_px[i]
                } else {
                    pick(backward[i + 1], line_px[i])
                };
            }
            for x in 0..len {
                out.px[at(line, x)] = pick(backward[x], forward[x + 2 * r]);
            }
        }
    }
    out
}
