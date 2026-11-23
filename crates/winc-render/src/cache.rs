//! 渲染层共用的缓存工具：小整数键的快速哈希、按代淘汰的缓存，以及图标描边的覆盖率蒙版缓存。
//!
//! 缓存都以帧为失效周期：字形与描边蒙版在帧首检查占用，超过上限时淘汰旧的一代。

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::{BuildHasherDefault, Hash, Hasher};

use crate::draw::canvas::{IRect, Stroke, StrokeRaster, stroke_area};

/// 适用于小整数键的快速哈希，算法取自 rustc 内部的 `FxHasher`。
///
/// 键都是字形 id、码点、字号这样的小整数，不来自外部输入，不需要标准库默认 `SipHash` 的
/// 防碰撞攻击能力；绘制每个字符都要查几次表，哈希开销直接算在每帧的时间预算里。
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

impl FxHasher {
    /// 每次写入乘的常数，单位无量纲，取自 rustc 的 `FxHasher`（黄金分割比的小数部分左移 64 位）。
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for &chunk in chunks {
            self.add(u64::from_le_bytes(chunk));
        }
        for &b in rest {
            self.add(u64::from(b));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// 用 [`FxHasher`] 的哈希表。
pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// 按代淘汰的缓存：新一代的估算占用超过 `cap` 字节后，新一代整代降为旧的一代，旧的一代释放，
/// 旧一代里再次用到的项搬回新一代。
///
/// 分两代是为了让刚用过的项留在缓存里，最多再搬一次，总占用最多约为 `cap` 的两倍；整体清空的
/// 缓存在一帧要用的项多于上限时，每帧都要重新生成全部项。按帧换代时在帧首换代，帧中间只在
/// 超过 `FRAME_SLACK` 倍上限时强制换代，避免丢弃本帧还要用的项。
pub(crate) struct Generations<K, V> {
    /// 新的一代。
    new: FastMap<K, V>,
    /// 旧的一代：下一次换代时释放。
    old: FastMap<K, V>,
    /// `new` 估算占用的字节数。
    bytes: usize,
    /// 一代的软上限（字节），即 `new` 的估算占用超过该值时在帧首换代。
    cap: usize,
    /// 帧中间（`get_or_insert_with` 里）的换代上限（字节）：不按帧换代时等于 `cap`。
    hard: usize,
}

/// 按帧换代时，一帧之中新一代的估算占用最多达到 `cap` 的倍数。
///
/// 取 4：一帧要用的项通常不超过上限的 2 倍，再留一倍余量；超出时在帧中间强制换代，使一次性
/// 绘制异常多的项时内存仍有上界。
const FRAME_SLACK: usize = 4;

impl<K: Hash + Eq + Copy, V> Generations<K, V> {
    /// 创建一代上限为 `cap` 字节的空缓存。
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            new: FastMap::default(),
            old: FastMap::default(),
            bytes: 0,
            cap,
            hard: cap,
        }
    }

    /// 创建按帧换代的空缓存：一代上限为 `cap` 字节，帧首调用 [`Generations::new_frame`]。
    pub(crate) fn per_frame(cap: usize) -> Self {
        Self {
            hard: cap * FRAME_SLACK,
            ..Self::new(cap)
        }
    }

    /// 开始新的一帧：新一代的估算占用超过 `cap` 时换代。
    pub(crate) fn new_frame(&mut self) {
        if self.bytes > self.cap {
            self.swap();
        }
    }

    fn swap(&mut self) {
        self.old = std::mem::take(&mut self.new);
        self.bytes = 0;
    }

    /// 清空两代，占用估算归零。
    pub(crate) fn clear(&mut self) {
        self.new = FastMap::default();
        self.old = FastMap::default();
        self.bytes = 0;
    }

    /// 返回 `key` 对应的可变引用；两代里都没有时用 `make` 生成并放进新一代。
    ///
    /// `cost` 返回一项占用的字节数，用于估算新一代的占用。新一代超过上限时在本次调用开头换代，
    /// 本次返回的项一定留在缓存里。
    pub(crate) fn get_or_insert_with(
        &mut self,
        key: K,
        cost: impl FnOnce(&V) -> usize,
        make: impl FnOnce() -> V,
    ) -> &mut V {
        if self.bytes > self.hard {
            self.swap();
        }
        match self.new.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let value = self.old.remove(&key).unwrap_or_else(make);
                self.bytes += cost(&value);
                entry.insert(value)
            }
        }
    }
}

/// 描边蒙版缓存一代的软上限（字节，512 KiB）。
///
/// 一个 2× 屏上的文件图标约 30×30 像素，蒙版不到 1 KiB，一屏的图标全部放下仍有余量；每帧角度
/// 都不同的折叠箭头靠上限淘汰。
const STROKE_CACHE_BYTES: usize = 512 << 10;

/// 缓存蒙版的最大面积（像素，64×64）。
///
/// 图标都比这个上限小；长线、大圆这类更大的描边很少重复，缓存占用的内存不划算，改为直接绘制。
const STROKE_CACHE_MAX_PIXELS: usize = 64 * 64;

/// 形状坐标的量化精度（每物理像素的等分数，无量纲）。
///
/// 量化后相同的两组形状共用一张蒙版，覆盖率的差异不超过一两级，肉眼难以分辨。
const QUANTUM: f32 = 64.0;

/// 圆弧角度的量化精度（每弧度的等分数，无量纲）。
///
/// 取 4096 时，半径 64 像素的圆周上相邻角度相差不到 1/64 像素，量化误差可以忽略。
const ANGLE_QUANTUM: f32 = 4096.0;

/// 一组描边形状的覆盖率蒙版。
struct StrokeMask {
    /// 量化后的形状和笔宽（相对蒙版左上角，单位为 1/`QUANTUM` 物理像素），
    /// 用来确认哈希值相同的确实是同一组形状。
    key: Box<[i32]>,
    /// 蒙版的宽高（物理像素）。
    size: (i32, i32),
    /// 每像素 1 字节的覆盖率（0–255），按行排列，每行 `size.0` 字节。
    data: Box<[u8]>,
}

/// 图标描边的覆盖率蒙版缓存。
///
/// 文件树、快速打开的每一行都有文件图标，每帧都要重画；逐像素求到每个形状的距离，比整块蒙版贴
/// 一遍贵一两个数量级。一组形状（量化后、相对整像素位置）的覆盖率只取决于形状本身，与颜色无关，
/// 因此按形状缓存蒙版，绘制时按颜色混合上去，结果与直接描边相同。
pub(crate) struct StrokeCache {
    /// 量化后形状的哈希值 → 蒙版。
    masks: Generations<u64, StrokeMask>,
    /// 存放量化形状的缓冲，复用以免每次调用都分配。
    scratch: Vec<i32>,
}

impl StrokeCache {
    /// 创建空的描边蒙版缓存。
    pub(crate) fn new() -> Self {
        Self {
            masks: Generations::new(STROKE_CACHE_BYTES),
            scratch: Vec::new(),
        }
    }

    /// 返回用笔宽 `width`（逻辑像素）描 `shapes` 得到的蒙版和它在画布上的位置（物理像素）。
    ///
    /// `scale` 是屏幕缩放系数。蒙版面积超过 `STROKE_CACHE_MAX_PIXELS`、不值得缓存时返回 `None`，
    /// 由调用方直接描边。
    pub(crate) fn get(
        &mut self,
        shapes: &[Stroke],
        width: f32,
        scale: f32,
    ) -> Option<(IRect, &[u8])> {
        let area = stroke_area(shapes, width, scale)?;
        let (w, h) = (area.width(), (area.y1 - area.y0) as usize);
        if w * h > STROKE_CACHE_MAX_PIXELS {
            return None;
        }

        let (ox, oy) = (area.x0 as f32, area.y0 as f32);
        let q = |v: f32| (v * QUANTUM).round() as i32;
        let (qx, qy) = (|x: f32| q(x * scale - ox), |y: f32| q(y * scale - oy));
        let key = &mut self.scratch;
        key.clear();
        key.push(q(width * scale));
        for &shape in shapes {
            match shape {
                Stroke::Line(x0, y0, x1, y1) => key.extend([0, qx(x0), qy(y0), qx(x1), qy(y1)]),
                Stroke::Circle(cx, cy, r) => key.extend([1, qx(cx), qy(cy), q(r * scale)]),
                Stroke::Arc(cx, cy, r, from, to) => {
                    let a = |v: f32| (v * ANGLE_QUANTUM).round() as i32;
                    key.extend([2, qx(cx), qy(cy), q(r * scale), a(from), a(to)]);
                }
            }
        }
        let mut hasher = FxHasher::default();
        key.hash(&mut hasher);

        let rasterize = |key: &[i32]| StrokeMask {
            key: key.into(),
            size: (area.x1 - area.x0, area.y1 - area.y0),
            data: StrokeRaster::new(shapes, width, scale)
                .expect("有范围的形状不为空")
                .mask(),
        };
        let cost = |m: &StrokeMask| {
            m.data.len() + m.key.len() * 4 + std::mem::size_of::<(u64, StrokeMask)>()
        };
        let mask = self
            .masks
            .get_or_insert_with(hasher.finish(), cost, || rasterize(key));
        // 哈希值相同而量化后的形状不同（实际不会发生）：重新生成，替换原来的那张蒙版。
        if *mask.key != **key {
            *mask = rasterize(key);
        }
        // 量化后相同的形状，像素范围的取整可能相差一个像素；按蒙版的宽高贴，多出的一圈覆盖率为 0。
        let at = IRect::new(area.x0, area.y0, mask.size.0, mask.size.1);
        Some((at, &mask.data))
    }
}

#[cfg(test)]
mod tests;
