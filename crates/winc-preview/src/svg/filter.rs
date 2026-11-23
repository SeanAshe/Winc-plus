//! SVG 滤镜（`<filter>` 元素和 CSS 滤镜函数）的解析与计算。
//!
//! 支持模糊、偏移、填色、合并、混合、合成、颜色矩阵、分量变换、投影、膨胀腐蚀十种原语。只在滤镜
//! 区域内计算，颜色是 0..1 的预乘浮点值，默认在线性 RGB 中计算（`color-interpolation-filters` 的
//! 初始值为 `linearRGB`）。其他滤镜无法绘制时整个滤镜不生效，元素原样绘制并在状态栏提示。

mod parsing;
mod primitives;

pub(super) use parsing::*;
pub(super) use primitives::*;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use roxmltree::Node;

use super::render::{Area, Layer};
use super::syntax::*;
use super::{Parser, fraction};

/// 滤镜的解析结果。
pub(super) enum Parsed {
    /// 解析成功的滤镜。
    Filter(Filter),
    /// 元素不画：滤镜区域按外框计算而元素没有外框，或者滤镜区域为空。
    Hide,
    /// 含有不支持的滤镜效果，整个滤镜不生效；字符串是状态栏提示的内容。
    Unsupported(&'static str),
}

/// 解析后的滤镜：滤镜区域、坐标变换和效果列表。
#[derive(Debug)]
pub(super) struct Filter {
    /// 滤镜区域（x、y、宽、高，元素的用户坐标）。
    region: [f32; 4],
    /// 元素的用户坐标到 SVG 根坐标的变换。
    ctm: Affine,
    /// 滤镜效果（原语），按执行顺序排列。
    primitives: Vec<Primitive>,
}

/// 一个滤镜效果（原语）及其输入输出设置。
#[derive(Debug)]
struct Primitive {
    kind: Kind,
    /// 两个输入（`in`、`in2`）。
    inputs: [Input; 2],
    /// `result` 属性，供后续原语按名字引用。
    result: Option<String>,
    /// 是否在线性 RGB 中计算。
    linear: bool,
    /// 只在该范围内输出（x、y、宽、高，元素的用户坐标）；`None` 表示整个滤镜区域。
    subregion: Option<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
enum Input {
    /// `SourceGraphic`：滤镜前的图。
    Source,
    /// `SourceAlpha`：滤镜前的图，只保留透明度。
    SourceAlpha,
    /// `Previous`：上一个原语的结果；第一个原语取源图。
    Previous,
    /// 引用前面某个原语的 `result`。
    Named(String),
    /// BackgroundImage、FillPaint 等未实现的输入：当作全透明。
    Empty,
}

#[derive(Debug)]
enum Kind {
    /// 高斯模糊：x、y 两个方向的标准差（像素）。
    Blur(f32, f32),
    /// 平移量（像素）。
    Offset(f32, f32),
    /// 不预乘的 sRGB 颜色；第 4 项是已乘上 `flood-opacity` 的透明度。
    Flood([f32; 4]),
    /// 合并多个输入，依次按 source-over 叠加。
    Merge(Vec<Input>),
    Blend(Blend),
    Composite(Composite),
    /// 4 × 5 的颜色矩阵，作用在不预乘的颜色上。
    ColorMatrix(Box<[f32; 20]>),
    /// R、G、B、A 四个通道的分量变换。
    Transfer(Box<[Transfer; 4]>),
    DropShadow {
        /// 模糊的标准差（像素）。
        blur: (f32, f32),
        /// 平移量（像素）。
        offset: (f32, f32),
        /// 不预乘的 sRGB 颜色。
        color: [f32; 4],
    },
    Morphology {
        /// 为 `true` 时膨胀（取邻域最大值），否则腐蚀（取最小值）。
        dilate: bool,
        /// 半径（像素），超过图像尺寸时按图像尺寸算。
        radius: (f32, f32),
    },
}

/// 混合模式，由 CSS 的 `mix-blend-mode` 与 `feBlend` 的 `mode` 属性指定。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Blend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

/// Porter-Duff 合成算子（`feComposite` 的 `operator` 属性）；`Arithmetic` 带 `k1` 到 `k4` 四个
/// 系数。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Composite {
    /// `over`：上层叠在下层上。
    Over,
    /// `in`：只保留上层与下层重叠的部分。
    In,
    /// `out`：只保留上层不重叠的部分。
    Out,
    /// `atop`：上层叠在下层上，但只保留下层有内容的地方。
    Atop,
    /// `xor`：两层不重叠的部分。
    Xor,
    /// `lighter`：两层相加。
    Lighter,
    Arithmetic([f32; 4]),
}

/// `feFuncR` 等分量变换函数（`feComponentTransfer` 的 `type`）及其参数。
#[derive(Debug, Clone, PartialEq)]
enum Transfer {
    Identity,
    /// 线性插值的查找表；值为 0..1 之间的采样点。
    Table(Vec<f32>),
    /// 阶梯查找表；值为 0..1 之间的采样点。
    Discrete(Vec<f32>),
    /// 线性函数：斜率、截距。
    Linear(f32, f32),
    /// 幂函数：幅度、指数、偏移。
    Gamma(f32, f32, f32),
}

/// 把图像按行分段，交给多个线程并行处理：对每段调用 `f(首行行号, 这几行的像素)`。
///
/// 像素数少于 `64 * 1024` 时在当前线程处理；线程数最多 8 个，避免大图上的调度开销超过计算
/// 本身。两个值取自调参。
fn par_rows<F>(px: &mut [[f32; 4]], w: usize, f: F)
where
    F: Fn(usize, &mut [[f32; 4]]) + Sync,
{
    let rows = px.len() / w.max(1);
    let threads = if px.len() < 64 * 1024 {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
    };
    if threads <= 1 || rows < 2 {
        f(0, px);
        return;
    }
    let each = rows.div_ceil(threads);
    std::thread::scope(|scope| {
        for (i, chunk) in px.chunks_mut(each * w).enumerate() {
            let f = &f;
            scope.spawn(move || f(i * each, chunk));
        }
    });
}

/// 滤镜区域内的一张图：预乘的 0..1 浮点像素；`linear` 表示是否为线性 RGB。
#[derive(Clone)]
struct Image {
    /// 宽度（像素）。
    w: usize,
    /// 高度（像素）。
    h: usize,
    /// 按行排列的像素，每项为预乘的 RGBA。
    px: Vec<[f32; 4]>,
    linear: bool,
}

impl Image {
    fn empty(w: usize, h: usize, linear: bool) -> Image {
        Image {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
            linear,
        }
    }

    /// 转换到 `linear` 指定的颜色空间（线性 RGB 或 sRGB）。
    fn to_space(&self, linear: bool) -> Image {
        if self.linear == linear {
            return self.clone();
        }
        let table = if linear { linear_table() } else { srgb_table() };
        let convert = |v: f32| lookup(table, v);
        let mut out = Image {
            linear,
            ..self.clone()
        };
        par_rows(&mut out.px, self.w, |_, chunk| {
            for p in chunk {
                let [r, g, b, a] = *p;
                *p = if a <= 0.0 {
                    [0.0; 4]
                } else {
                    [
                        convert(r / a) * a,
                        convert(g / a) * a,
                        convert(b / a) * a,
                        a,
                    ]
                };
            }
        });
        out
    }
}

/// sRGB 与线性 RGB 互相换算的查找表的分段数（段）。
///
/// 0..1 分成 4096 段，段内线性插值，避免对每个像素计算一次乘方。
const STEPS: usize = 4096;

/// 返回 sRGB 到线性 RGB 的查找表（`STEPS + 1` 项，进程内只初始化一次）。
fn linear_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=STEPS)
            .map(|i| to_linear(i as f32 / STEPS as f32))
            .collect()
    })
}

/// 返回线性 RGB 到 sRGB 的查找表（`STEPS + 1` 项，进程内只初始化一次）。
fn srgb_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=STEPS)
            .map(|i| to_srgb(i as f32 / STEPS as f32))
            .collect()
    })
}

/// 在 0..1 上按 `table` 分段线性插值，`v` 超出范围时夹取到边界。
fn lookup(table: &[f32], v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0) * STEPS as f32;
    let i = (x as usize).min(STEPS - 1);
    let f = x - i as f32;
    table[i] + (table[i + 1] - table[i]) * f
}

/// 把 sRGB 分量 `c`（0..1）换算为线性 RGB（IEC 61966-2-1 的传递函数）。
fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// 把线性 RGB 分量 `c`（0..1）换算为 sRGB（IEC 61966-2-1 的传递函数）。
fn to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

impl Filter {
    /// 对已绘制的一层 `source` 应用滤镜，返回新的一层。
    ///
    /// `viewport` 是 SVG 根坐标到画布像素的变换。
    pub(super) fn apply(&self, source: Layer, viewport: Affine) -> Layer {
        let device = self.ctm.then(viewport);
        let area = device_area(self.region, device, source.width, source.height);
        let mut out = Layer::new(source.width, source.height);
        if area.is_empty() {
            return out;
        }
        let (w, h) = (area.x1 - area.x0, area.y1 - area.y0);
        let [a, b, c, d, ..] = device.0;
        let (sx, sy) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());

        // 源图：取 `source` 中位于滤镜区域内的部分。
        let mut src = Image::empty(w, h, false);
        for y in 0..h {
            for x in 0..w {
                let p = source.px[(area.y0 + y) * source.width + area.x0 + x];
                src.px[y * w + x] = p.map(|c| f32::from(c) / 255.0);
            }
        }
        let alpha_only = |img: &Image| Image {
            w: img.w,
            h: img.h,
            linear: img.linear,
            px: img.px.iter().map(|p| [0.0, 0.0, 0.0, p[3]]).collect(),
        };

        // 记录每个命名结果最后一次被引用的步骤序号：此后即可丢弃，以减少大图上的内存占用。
        let mut last_use: HashMap<&str, usize> = HashMap::new();
        for (i, p) in self.primitives.iter().enumerate() {
            let inputs = match &p.kind {
                Kind::Merge(inputs) => inputs.iter().collect::<Vec<_>>(),
                _ => p.inputs.iter().collect(),
            };
            for input in inputs {
                if let Input::Named(name) = input {
                    last_use.insert(name.as_str(), i);
                }
            }
        }
        let src = Rc::new(src);
        let mut source_alpha: Option<Rc<Image>> = None;
        let mut converted: HashMap<(usize, bool), Rc<Image>> = HashMap::new();
        let mut results: HashMap<String, Rc<Image>> = HashMap::new();
        let mut previous: Option<Rc<Image>> = None;
        for (step, p) in self.primitives.iter().enumerate() {
            let mut fetch = |input: &Input| -> Rc<Image> {
                let img = match input {
                    Input::Source => Rc::clone(&src),
                    Input::SourceAlpha => {
                        Rc::clone(source_alpha.get_or_insert_with(|| Rc::new(alpha_only(&src))))
                    }
                    Input::Previous => previous.clone().unwrap_or_else(|| Rc::clone(&src)),
                    // 引用了尚未产生的结果：按未指定输入处理，取上一步的结果。
                    Input::Named(name) => results
                        .get(name)
                        .cloned()
                        .or_else(|| previous.clone())
                        .unwrap_or_else(|| Rc::clone(&src)),
                    Input::Empty => return Rc::new(Image::empty(w, h, p.linear)),
                };
                if img.linear == p.linear {
                    return img;
                }
                // 同一张图转换颜色空间只做一次（结果缓存在 `converted` 中）。
                let key = (Rc::as_ptr(&img) as usize, p.linear);
                Rc::clone(
                    converted
                        .entry(key)
                        .or_insert_with(|| Rc::new(img.to_space(p.linear))),
                )
            };
            let mut result = match &p.kind {
                Kind::Blur(x, y) => blur(&fetch(&p.inputs[0]), x * sx, y * sy),
                Kind::Offset(dx, dy) => offset(&fetch(&p.inputs[0]), dx * sx, dy * sy),
                Kind::Flood(color) => flood(w, h, *color, p.linear),
                Kind::Merge(inputs) => {
                    let mut acc = Image::empty(w, h, p.linear);
                    for input in inputs {
                        let top = fetch(input);
                        composite(&mut acc, &top, Composite::Over);
                    }
                    acc
                }
                Kind::Blend(mode) => {
                    let top = fetch(&p.inputs[0]);
                    blend(&top, &fetch(&p.inputs[1]), *mode)
                }
                Kind::Composite(op) => {
                    let top = fetch(&p.inputs[0]);
                    let mut bottom = (*fetch(&p.inputs[1])).clone();
                    composite(&mut bottom, &top, *op);
                    bottom
                }
                Kind::ColorMatrix(m) => color_matrix(&fetch(&p.inputs[0]), m),
                Kind::Transfer(funcs) => transfer(&fetch(&p.inputs[0]), funcs),
                Kind::DropShadow {
                    blur: (bx, by),
                    offset: (dx, dy),
                    color,
                } => {
                    let input = fetch(&p.inputs[0]);
                    let shadow = offset(
                        &blur(&alpha_only(&input), bx * sx, by * sy),
                        dx * sx,
                        dy * sy,
                    );
                    // 用 `color` 给阴影的形状着色，再把原图叠在上面。
                    let tint = flood(1, 1, *color, p.linear).px[0];
                    let mut shade = shadow;
                    for s in &mut shade.px {
                        let a = s[3];
                        *s = tint.map(|c| c * a);
                    }
                    composite(&mut shade, &input, Composite::Over);
                    shade
                }
                Kind::Morphology {
                    dilate,
                    radius: (rx, ry),
                } => morphology(
                    &fetch(&p.inputs[0]),
                    *dilate,
                    (rx * sx).round() as usize,
                    (ry * sy).round() as usize,
                ),
            };
            if let Some(sub) = p.subregion {
                let sub = device_area(sub, device, source.width, source.height);
                for y in 0..h {
                    for x in 0..w {
                        let (gx, gy) = (area.x0 + x, area.y0 + y);
                        if gx < sub.x0 || gx >= sub.x1 || gy < sub.y0 || gy >= sub.y1 {
                            result.px[y * w + x] = [0.0; 4];
                        }
                    }
                }
            }
            let result = Rc::new(result);
            if let Some(name) = &p.result
                && last_use.get(name.as_str()).is_some_and(|&last| last > step)
            {
                results.insert(name.clone(), Rc::clone(&result));
            }
            results.retain(|name, _| last_use.get(name.as_str()).is_some_and(|&last| last > step));
            converted.clear();
            previous = Some(result);
            // 命名结果与滤镜区域一样大，同时保留的数量由文件决定：数百个 `result` 被同一个
            // `feMerge` 引用时合计可达数十 GB。连同源图和上一步的结果，合计像素超过单张图片的
            // 解码上限（`crate::image::PIXELS_MAX`）时不再应用滤镜，按原样绘制。
            let live = (results.len() as u64 + 2) * (w * h) as u64;
            if live > crate::image::PIXELS_MAX {
                return source;
            }
        }

        let last = previous.unwrap_or(src);
        let last = if last.linear {
            Rc::new(last.to_space(false))
        } else {
            last
        };
        for y in 0..h {
            for x in 0..w {
                let p = last.px[y * w + x];
                // 透明度为 NaN（例如矩阵中的 `1e39 × 0`）时，`clamp(0.0, a)` 的上界是 NaN，会
                // panic：按全透明处理。
                let a = if p[3].is_nan() {
                    0.0
                } else {
                    p[3].clamp(0.0, 1.0)
                };
                out.px[(area.y0 + y) * out.width + area.x0 + x] =
                    p.map(|c| (c.clamp(0.0, a) * 255.0).round() as u8);
            }
        }
        out.touched = area;
        out
    }
}

#[cfg(test)]
mod tests;
