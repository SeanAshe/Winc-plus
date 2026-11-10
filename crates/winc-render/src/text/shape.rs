//! 等宽文字的连字：用 OpenType 的 `calt`、`liga` 把 `=>`、`!=` 换成字体里的连字字形。
//!
//! 编辑区仍按字符格子排版。整形只决定画哪个字形；格子的位置不跟字距走，下一簇总是从下一个
//! 字符的格子开始。这样光标、选区和点击都还落在原来的列上。

use super::font::{FontKind, MONO};
use super::glyphs::GlyphCache;

/// 编程字体用来做连字的特性。`calt` 是 Maple Mono、Fira Code 这一类的主体；`liga`、`clig`
/// 是普通连字；`dlig` 是另一些字体把箭头放进去的自由连字。
const LIGATURES: &[(&str, u16)] = &[("calt", 1), ("liga", 1), ("clig", 1), ("dlig", 1)];

/// 整形得到的一个字形，覆盖这段文字里的几个字符。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ligature {
    /// 起点（这段文字里的字符下标，包含）。
    pub start: usize,
    /// 终点（这段文字里的字符下标，不含）。
    pub end: usize,
    /// 为真时从簇的左边缘按 `pen` 排放（一个簇里有多个字形，例如基字加附标）；
    /// 为假时在所占的格子里居中，和逐字绘制一致。
    pub placed: bool,
    /// `placed` 时相对簇左边缘的笔位置（逻辑像素）。
    pub pen: f32,
    /// 相对笔位置的横向偏移（逻辑像素）。单个字形不使用，以免字距把字拉离格子。
    pub x: f32,
    /// 相对基线向上的偏移（逻辑像素）。
    pub y: f32,
    /// 字体槽位。
    pub(crate) slot: u8,
    /// 字体内的字形 id。
    pub(crate) id: u16,
}

impl GlyphCache {
    /// 返回字体 `kind` 整形时使用的槽位。
    ///
    /// 斜体不存在或字宽对不上时，注释改用正体再斜切，槽位因此是正体。
    pub(crate) fn shape_slot(&mut self, kind: FontKind) -> u8 {
        if kind == FontKind::MonoItalic {
            let _ = self.fonts.resolve(FontKind::MonoItalic, '0');
        }
        let primary = self.fonts.primary(kind);
        if self.fonts.face(primary).is_some() {
            primary
        } else if kind == FontKind::MonoItalic {
            MONO
        } else {
            primary
        }
    }

    /// 字符 `c` 是否由字体 `kind` 的主字体绘制。落到后备字体上时不能和前后的字一起连字。
    pub(crate) fn in_shape_slot(&mut self, kind: FontKind, c: char) -> bool {
        let slot = self.shape_slot(kind);
        let (got, glyph) = self.fonts.resolve(kind, c);
        glyph != 0 && got == slot
    }

    /// 把 `chars` 按字体 `kind`、字号 `size`（逻辑像素）整形，结果留在 [`Self::shaped`]。
    ///
    /// 没有可用的字体、或这段文字没有字形时，`shaped` 为空，调用方逐字绘制。
    pub(crate) fn shape_run(&mut self, kind: FontKind, size: f32, chars: &[char]) {
        self.shaped.clear();
        if chars.is_empty() || size <= 0.0 {
            return;
        }
        let mut text = String::new();
        let mut bytes = Vec::with_capacity(chars.len() + 1);
        bytes.push(0);
        for c in chars {
            text.push(*c);
            bytes.push(text.len());
        }

        let slot = self.shape_slot(kind);
        let Some(face) = self.fonts.face(slot) else {
            return;
        };
        let coords = face.coords.clone();
        let font = face.font();
        let mut shaper = self
            .shaper
            .builder(font)
            .size(size)
            .features(LIGATURES)
            .normalized_coords(&coords)
            .build();
        shaper.add_str(&text);

        let mut shaped = Vec::new();
        shaper.shape_with(|cluster| {
            let start = bytes.partition_point(|&b| b < cluster.source.start as usize);
            let end = bytes.partition_point(|&b| b < cluster.source.end as usize);
            if end <= start {
                return;
            }
            let glyphs = cluster.glyphs;
            if glyphs.is_empty() || glyphs.iter().all(|glyph| glyph.id == 0) {
                return;
            }
            let multi = glyphs.len() > 1;
            let mut pen = 0.0;
            for glyph in glyphs {
                if glyph.id == 0 {
                    pen += glyph.advance;
                    continue;
                }
                shaped.push(Ligature {
                    start,
                    end,
                    placed: multi,
                    pen: if multi { pen } else { 0.0 },
                    x: if multi { glyph.x } else { 0.0 },
                    y: glyph.y,
                    slot,
                    id: glyph.id,
                });
                pen += glyph.advance;
            }
        });
        self.shaped = shaped;
    }
}

#[cfg(test)]
mod tests {
    use super::super::font::{FontKind, Fonts, MONO};
    use super::super::font_path::FontSpec;
    use super::super::glyphs::GlyphCache;

    #[test]
    fn a_single_letter_keeps_its_cmap_glyph() {
        let fonts = Fonts::load(None, None, None, 400).expect("系统里有等宽字体");
        let mut cache = GlyphCache::new(fonts);
        cache.shape_run(FontKind::Mono, 16.0, &['A']);
        let face = cache.fonts.primary_face(FontKind::Mono).unwrap();
        let expected = face.font().charmap().map('A');
        assert_eq!(cache.shaped.len(), 1);
        assert_eq!(cache.shaped[0].id, expected);
        assert_eq!((cache.shaped[0].start, cache.shaped[0].end), (0, 1));
    }

    #[test]
    fn coding_ligatures_replace_the_separate_glyphs_when_the_font_has_them() {
        let spec = [
            "Maple Mono NF CN",
            "Maple Mono",
            "Cascadia Code",
            "Fira Code",
            "JetBrains Mono",
        ]
        .into_iter()
        .find_map(|name| {
            let spec = FontSpec::Name(name.into());
            spec.check().is_ok().then_some(spec)
        });
        let Some(spec) = spec else {
            return;
        };
        let fonts = Fonts::load(Some(&spec), None, None, 400).expect("连字字体能打开");
        let mut cache = GlyphCache::new(fonts);
        cache.shape_run(FontKind::Mono, 16.0, &['=', '>']);
        let face = cache.fonts.face(MONO).unwrap();
        let cmap = |c: char| face.font().charmap().map(c);
        let separate = cache.shaped.len() == 2
            && cache.shaped[0].id == cmap('=')
            && cache.shaped[1].id == cmap('>');
        assert!(
            !separate,
            "字体应把 => 连成连字，实际 {:?}",
            cache.shaped.iter().map(|g| g.id).collect::<Vec<_>>()
        );
    }
}
