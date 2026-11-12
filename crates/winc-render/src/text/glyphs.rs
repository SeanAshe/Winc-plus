//! 字形缓存：按字体、字号和字符缓存光栅化后的字形，以及模拟的粗体和斜体。

use super::shape::Ligature;
use super::*;
use swash::FontRef;
use swash::shape::ShapeContext;
use swash::zeno::{Fill, Join, Placement, Stroke, Style};

/// 字形缓存的键：同一字体槽位、字形、字号和亚像素档位对应同一张位图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    /// 字体槽位（见 `Fonts`）。
    pub(crate) slot: u8,
    /// 字体内的字形 id。
    pub(crate) glyph: u16,
    /// 物理像素字号乘 4 后取整（四分之一像素精度），避免用浮点数做键。
    ///
    /// 用 `u32`：`u16` 在字号超过 16383 物理像素时饱和，不同字号会共用同一个缓存项。
    pub(crate) size: u32,
    /// 横向亚像素偏移，分四档：0、¼、½、¾。
    pub(crate) subpixel: u8,
    /// 是否把正体斜切成斜体（字体没有可用的斜体时）。
    pub(crate) oblique: bool,
    /// 是否模拟粗体（见 [`Faux`]）。
    pub(crate) bold: bool,
    /// 位图的覆盖率是否按深色背景的映射表换算（见 [`GlyphCache::get`]）。
    pub(crate) dark: bool,
}

/// 一个已光栅化的字形位图。
pub(crate) struct Glyph {
    /// 位图宽度（物理像素）。
    pub(crate) width: u16,
    /// 位图高度（物理像素）。
    pub(crate) height: u16,
    /// 位图左边缘相对笔位置的横向偏移（物理像素，向右为正）。
    pub(crate) left: i16,
    /// 位图上边缘相对基线的高度（物理像素，向上为正）。
    pub(crate) top: i16,
    /// 是否为彩色字形（如 emoji）。
    pub(crate) color: bool,
    /// 单色字形每像素 1 字节覆盖率；彩色字形是 RGBA8。
    pub(crate) data: Box<[u8]>,
}

/// 是否按像素网格微调字形轮廓（见 [`GlyphCache::hinting`]）的平台默认值。
///
/// macOS 的系统文字渲染不做微调，此处保持一致；其余平台的系统文字渲染均做微调。实测 1 至 2 倍
/// 缩放下微调后接近全覆盖的像素占比均高于未微调，1 倍时差别最大，因此不按缩放系数区分。
const HINTING: bool = cfg!(not(target_os = "macos"));

/// 字体、字形缓存和前进宽度缓存。
pub(crate) struct GlyphCache {
    pub(crate) fonts: Fonts,
    /// 光栅化时是否按像素网格微调字形轮廓。
    ///
    /// 微调只移动竖直方向的坐标：横画、x 字高和大写字高对齐到像素行，笔画不再跨两行像素。
    /// 横向坐标和前进宽度不变，排版与横向亚像素定位不受影响。位图随该值不同，整个缓存只用
    /// 一个值，因此不计入 [`GlyphKey`]。
    pub(crate) hinting: bool,
    /// swash 的光栅化上下文，复用以避免重复分配。
    scaler: ScaleContext,
    /// 光栅化后的字形，每一代的上限为 [`GLYPH_CACHE_BYTES`] 字节。
    glyphs: Generations<GlyphKey, Glyph>,
    /// （槽位，字形，字号×4）→ 前进宽度，逻辑像素。
    pub(crate) advances: FastMap<(u8, u16, u32), f32>,
    /// 整形上下文，行与行之间复用，避免每次重新分配。
    pub(crate) shaper: ShapeContext,
    /// 最近一次 [`Self::shape_run`] 的结果。下一次整形会清空。
    pub(crate) shaped: Vec<Ligature>,
}

/// 字符解析出的字体槽位、字形 id 和前进宽度，见 [`GlyphCache::lookup`]。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Resolved {
    /// 字体槽位（见 `Fonts`）。
    pub(crate) slot: u8,
    /// 字体内的字形 id。
    pub(crate) glyph: u16,
    /// 前进宽度（逻辑像素）。
    pub(crate) advance: f32,
}

impl GlyphCache {
    /// 创建空的字形缓存，字体取自 `fonts`。
    pub(crate) fn new(fonts: Fonts) -> Self {
        Self {
            fonts,
            hinting: HINTING,
            scaler: ScaleContext::new(),
            glyphs: Generations::per_frame(GLYPH_CACHE_BYTES),
            advances: FastMap::default(),
            shaper: ShapeContext::new(),
            shaped: Vec::new(),
        }
    }

    /// 开始新的一帧：字形缓存在此换代（见 [`Generations::per_frame`]）。
    pub(crate) fn new_frame(&mut self) {
        self.glyphs.new_frame();
    }

    /// 返回字体 `kind` 在字号 `size`（逻辑像素）下的度量，见 [`Renderer::metrics`]。
    ///
    /// 斜体与正体同宽（字宽不同的斜体不会被采用），度量取正体的值。
    pub(crate) fn metrics(&self, kind: FontKind, size: f32) -> Metrics {
        let kind = match kind {
            FontKind::MonoItalic => FontKind::Mono,
            FontKind::TerminalItalic => FontKind::Terminal,
            kind => kind,
        };
        let face = self.fonts.primary_face(kind).expect("主字体总是已加载");
        let font = face.font();
        let metrics = font.metrics(&face.coords).scale(size);
        let cell = font
            .glyph_metrics(&face.coords)
            .scale(size)
            .advance_width(font.charmap().map('0'));

        let (x_height, cap_height) = heights(font, &face.coords, size);
        Metrics {
            ascent: metrics.ascent,
            descent: metrics.descent,
            cell,
            x_height,
            cap_height,
        }
    }

    /// 清空字形缓存（不含前进宽度缓存）。
    pub(crate) fn clear(&mut self) {
        self.glyphs.clear();
    }

    /// 解析字符 `c` 在字体 `kind`、字号 `size`（逻辑像素）下的字体槽位、字形 id 和前进宽度。
    ///
    /// 前进宽度带缓存；绘制一个字符只解析一次，测量宽度和绘制字形都使用该结果。
    pub(crate) fn lookup(&mut self, kind: FontKind, size: f32, c: char) -> Resolved {
        self.lookup_faux(kind, Faux::default(), size, c)
    }

    /// 与 [`Self::lookup`] 相同，但模拟粗体时前进宽度加上笔画加宽的量。
    pub(crate) fn lookup_faux(
        &mut self,
        kind: FontKind,
        faux: Faux,
        size: f32,
        c: char,
    ) -> Resolved {
        let (slot, glyph) = self.fonts.resolve(kind, c);
        let fonts = &self.fonts;
        let key = (slot, glyph, (size * 4.0) as u32);

        if self.advances.len() >= ADVANCE_CACHE_ENTRIES && !self.advances.contains_key(&key) {
            self.advances = FastMap::default();
        }

        let advance = *self.advances.entry(key).or_insert_with(|| {
            fonts.face(slot).map_or(0.0, |face| {
                face.font()
                    .glyph_metrics(&face.coords)
                    .scale(size)
                    .advance_width(glyph)
            })
        });
        Resolved {
            slot,
            glyph,
            advance: if faux.bold && !c.is_whitespace() {
                advance + size * BOLD_RATIO
            } else {
                advance
            },
        }
    }

    /// 返回字符 `c` 在字体 `kind`、字号 `size`（逻辑像素）下的前进宽度（逻辑像素），结果带缓存。
    pub(crate) fn advance(&mut self, kind: FontKind, size: f32, c: char) -> f32 {
        self.lookup(kind, size, c).advance
    }

    /// 返回字形 `glyph`（槽位 `slot`）在字号 `size`（逻辑像素）下的前进宽度，结果带缓存。
    ///
    /// 连字用整形给出的字形 id，不再经过字符映射。
    pub(crate) fn glyph_advance(&mut self, slot: u8, glyph: u16, size: f32) -> f32 {
        let key = (slot, glyph, (size * 4.0) as u32);
        if self.advances.len() >= ADVANCE_CACHE_ENTRIES && !self.advances.contains_key(&key) {
            self.advances = FastMap::default();
        }
        *self.advances.entry(key).or_insert_with(|| {
            self.fonts.face(slot).map_or(0.0, |face| {
                face.font()
                    .glyph_metrics(&face.coords)
                    .scale(size)
                    .advance_width(glyph)
            })
        })
    }

    /// 返回缓存中的字形，没有时先光栅化；没有可见像素的字形（如空白）返回 `None`。
    ///
    /// `size_px` 是物理像素字号，`subpixel` 是 0 至 3 的横向亚像素档位。
    ///
    /// 单色字形的覆盖率已按 `gamma` 换算，绘制时不再换算；`gamma` 必须与 `key.dark` 对应
    /// （深色、浅色背景各有一张映射表），两种背景下的位图因此分开缓存。
    pub(crate) fn get(&mut self, key: GlyphKey, size_px: f32, gamma: &[u8; 256]) -> Option<&Glyph> {
        let (fonts, scaler, hinting) = (&self.fonts, &mut self.scaler, self.hinting);
        // 空字形同样占用表中的一项，按一项的大小计入。
        let cost = |g: &Glyph| g.data.len() + std::mem::size_of::<(GlyphKey, Glyph)>();
        // 光栅化失败的字形（如字体不含可渲染的轮廓或位图）同样记为空字形：同一个键下次仍会
        // 失败，避免每帧重试。
        let make = || {
            let mut glyph = rasterize(fonts, scaler, key, size_px, hinting).unwrap_or(Glyph {
                width: 0,
                height: 0,
                left: 0,
                top: 0,
                color: false,
                data: Box::default(),
            });
            // 光栅化时即按 `gamma` 换算单色字形的覆盖率，绘制时只需混色：
            // 逐像素查表约占字形绘制一半以上的时间。
            if !glyph.color {
                for coverage in &mut glyph.data {
                    *coverage = gamma[usize::from(*coverage)];
                }
            }
            glyph
        };
        let glyph = self.glyphs.get_or_insert_with(key, cost, make);
        (glyph.width > 0 && glyph.height > 0).then_some(&*glyph)
    }
}

/// 返回字体在字号 `size` 下的 x 字高和大写字高（逻辑像素）。
///
/// 旧版 OS/2 表（版本 0、1）没有这两项，此时取字母 `x`、`H` 轮廓的顶点。
pub(crate) fn heights(font: FontRef<'_>, coords: &[i16], size: f32) -> (f32, f32) {
    let metrics = font.metrics(coords).scale(size);
    let top = |stated: f32, c: char| {
        if stated > 0.0 {
            return stated;
        }
        let glyph = font.charmap().map(c);
        let mut context = ScaleContext::new();
        let mut scaler = context
            .builder(font)
            .size(size)
            .normalized_coords(coords)
            .build();
        scaler
            .scale_outline(glyph)
            .map_or(0.0, |outline| outline.bounds().max.y)
    };
    (top(metrics.x_height, 'x'), top(metrics.cap_height, 'H'))
}

/// 用 `fonts` 中的字体光栅化 `key` 对应的字形；字体里没有可用的轮廓或位图时返回 `None`。
///
/// `hinting` 为是否按像素网格微调轮廓；由系统提供轮廓的字体不做微调。
fn rasterize(
    fonts: &Fonts,
    scaler: &mut ScaleContext,
    key: GlyphKey,
    size_px: f32,
    hinting: bool,
) -> Option<Glyph> {
    let face = fonts.face(key.slot)?;
    let dx = key.subpixel as f32 / 4.0;
    // 笔画加宽的总量（物理像素）：补足字重的部分与模拟粗体之和。
    let bold = if key.bold { BOLD_RATIO } else { 0.0 };
    let widen = size_px * (face.embolden + bold);
    if let Some(system) = &face.system {
        let slant = if key.oblique {
            OBLIQUE_DEGREES.to_radians().tan()
        } else {
            0.0
        };
        let mask = system.fill(key.glyph, size_px, dx, slant, widen)?;
        return Some(Glyph {
            width: mask.width as u16,
            height: mask.height as u16,
            left: mask.left as i16,
            top: mask.top as i16,
            color: false,
            data: mask.data.into_boxed_slice(),
        });
    }
    let mut scaler = scaler
        .builder(face.font())
        .size(size_px)
        .hint(hinting)
        .normalized_coords(&face.coords)
        .build();
    if widen > 0.0
        && !scaler.has_color_outlines()
        && !scaler.has_color_bitmaps()
        && let Some(glyph) = rasterize_bold(&mut scaler, key, widen, dx)
    {
        return Some(glyph);
    }
    let image = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ])
    .format(Format::Alpha)
    .offset(Vector::new(dx, 0.0))
    // swash 在两侧各加宽传入的量（FreeType 则先除以 2），总宽度增加该量的两倍。
    .embolden(widen / 2.0)
    .transform(key.oblique.then(|| {
        Transform::skew(
            Angle::from_degrees(OBLIQUE_DEGREES),
            Angle::from_degrees(0.0),
        )
    }))
    .render(&mut scaler, key.glyph)?;

    let p = image.placement;
    Some(Glyph {
        width: p.width as u16,
        height: p.height as u16,
        left: p.left as i16,
        top: p.top as i16,
        color: image.content == Content::Color,
        data: image.data.into_boxed_slice(),
    })
}

/// 合并字形与宽 `widen`（物理像素）的描边，得到加粗的字形：笔画两侧各加宽一半。
///
/// 不沿轮廓方向偏移：组合字形的轮廓方向判断不可靠，判断错误时笔画会变细。描边只会向外扩展。
/// 描边与填充各由 swash 的 [`Render`] 光栅化一次，再逐像素取较大的覆盖率合并。
fn rasterize_bold(
    scaler: &mut swash::scale::Scaler,
    key: GlyphKey,
    widen: f32,
    dx: f32,
) -> Option<Glyph> {
    let mut render = |style: Style<'_>| {
        Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .style(style)
            .offset(Vector::new(dx, 0.0))
            .transform(key.oblique.then(|| {
                Transform::skew(
                    Angle::from_degrees(OBLIQUE_DEGREES),
                    Angle::from_degrees(0.0),
                )
            }))
            .render(scaler, key.glyph)
    };
    let mut stroke = Stroke::new(widen);
    stroke.join(Join::Miter);
    let mut image = render(Style::Stroke(stroke))?;
    let fill = render(Style::Fill(Fill::NonZero))?;
    let placement = image.placement;
    merge_coverage(&mut image.data, placement, &fill.data, fill.placement);
    Some(Glyph {
        width: placement.width as u16,
        height: placement.height as u16,
        left: placement.left as i16,
        top: placement.top as i16,
        color: false,
        data: image.data.into_boxed_slice(),
    })
}

/// 把覆盖率位图 `src` 叠加到 `dst` 上，每个像素取较大的值；超出 `dst` 的部分丢弃。
fn merge_coverage(dst: &mut [u8], at: Placement, src: &[u8], from: Placement) {
    let width = at.width as usize;
    for (row, line) in src.chunks_exact((from.width as usize).max(1)).enumerate() {
        let y = i64::from(at.top) - i64::from(from.top) + row as i64;
        if y < 0 || y >= i64::from(at.height) {
            continue;
        }
        for (column, &coverage) in line.iter().enumerate() {
            let x = i64::from(from.left) - i64::from(at.left) + column as i64;
            if x < 0 || x >= i64::from(at.width) {
                continue;
            }
            let pixel = &mut dst[y as usize * width + x as usize];
            *pixel = (*pixel).max(coverage);
        }
    }
}

#[cfg(test)]
mod tests;
