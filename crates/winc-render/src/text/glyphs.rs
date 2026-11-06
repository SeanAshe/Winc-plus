//! 字形缓存：按字体、字号和字符缓存光栅化后的字形，以及模拟的粗体和斜体。

use super::shape::Ligature;
use super::*;
use swash::FontRef;
use swash::shape::ShapeContext;

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
}

/// 光栅化好的一个字形位图。
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

/// 字体、字形缓存和前进宽度缓存。
pub(crate) struct GlyphCache {
    pub(crate) fonts: Fonts,
    /// swash 的光栅化上下文，复用以避免重复分配。
    scaler: ScaleContext,
    /// 光栅化后的字形，一代上限 [`GLYPH_CACHE_BYTES`] 字节。
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
            scaler: ScaleContext::new(),
            glyphs: Generations::per_frame(GLYPH_CACHE_BYTES),
            advances: FastMap::default(),
            shaper: ShapeContext::new(),
            shaped: Vec::new(),
        }
    }

    /// 一帧开始时调用：字形缓存在这里换代（见 [`Generations::per_frame`]）。
    pub(crate) fn new_frame(&mut self) {
        self.glyphs.new_frame();
    }

    /// 返回字体 `kind` 在字号 `size`（逻辑像素）下的度量，见 [`Renderer::metrics`]。
    ///
    /// 斜体与正体同宽（字宽不同的斜体不会被采用），度量取正体的。
    pub(crate) fn metrics(&self, kind: FontKind, size: f32) -> Metrics {
        let kind = match kind {
            FontKind::MonoItalic => FontKind::Mono,
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
    /// 前进宽度带缓存；绘制一个字符只解析这一次，量宽度和画字形都用这次的结果。
    pub(crate) fn lookup(&mut self, kind: FontKind, size: f32, c: char) -> Resolved {
        self.lookup_faux(kind, Faux::default(), size, c)
    }

    /// 同 [`Self::lookup`]，模拟粗体时前进宽度加上笔画加宽的量。
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
    pub(crate) fn get(&mut self, key: GlyphKey, size_px: f32) -> Option<&Glyph> {
        let (fonts, scaler) = (&self.fonts, &mut self.scaler);
        // 空字形同样占用表中的一项，按一项的大小计入。
        let cost = |g: &Glyph| g.data.len() + std::mem::size_of::<(GlyphKey, Glyph)>();
        // 光栅化失败的字形（如字体不含可渲染的轮廓或位图）同样记为空字形：同一个键下次仍会
        // 失败，避免每帧重试。
        let make = || {
            rasterize(fonts, scaler, key, size_px).unwrap_or(Glyph {
                width: 0,
                height: 0,
                left: 0,
                top: 0,
                color: false,
                data: Box::default(),
            })
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
fn rasterize(
    fonts: &Fonts,
    scaler: &mut ScaleContext,
    key: GlyphKey,
    size_px: f32,
) -> Option<Glyph> {
    let face = fonts.face(key.slot)?;
    let dx = key.subpixel as f32 / 4.0;
    if let Some(system) = &face.system {
        let slant = if key.oblique {
            OBLIQUE_DEGREES.to_radians().tan()
        } else {
            0.0
        };
        let bold = if key.bold { size_px * BOLD_RATIO } else { 0.0 };
        let mask = system.fill(key.glyph, size_px, dx, slant, bold)?;
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
        .hint(false)
        .normalized_coords(&face.coords)
        .build();
    let image = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ])
    .format(Format::Alpha)
    .offset(Vector::new(dx, 0.0))
    // swash 把两侧各加宽传入的量（FreeType 先除以 2），总宽度多出它的两倍。
    .embolden(if key.bold {
        size_px * BOLD_RATIO / 2.0
    } else {
        0.0
    })
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

#[cfg(test)]
mod tests;
