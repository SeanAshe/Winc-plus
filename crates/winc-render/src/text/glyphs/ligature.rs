//! 连字：按等宽字体整形一段符号，取用字体为其换用的连写字形（OpenType 的 `calt`、`liga`）。
//!
//! 编辑区逐字符绘制，只在开启连字时为相邻的符号整形。结果按字符所在的格排开，列宽不变。

use super::*;

/// 整形时启用的 OpenType 特性：上下文替换与标准连字，编程字体的连字由二者实现。
const FEATURES: [(&str, u16); 2] = [("calt", 1), ("liga", 1)];

/// 一代整形结果：（槽位，一段符号）→ 整形结果；字体没有为该段换用字形时为 `None`。
pub(super) type Cache = FastMap<(u8, Box<str>), Option<Rc<Ligated>>>;

/// 一段符号启用连字整形后的字形，长度以字号为单位（乘以字号得到逻辑像素）。
#[derive(Debug)]
pub(crate) struct Ligated {
    /// 字体槽位。
    pub(crate) slot: u8,
    /// 各字形：（所属字符在该段中的序号，字形 id，横向偏移，纵向偏移）。
    pub(crate) glyphs: Vec<(usize, u16, f32, f32)>,
}

impl GlyphCache {
    /// 返回字体 `kind` 为符号 `run` 启用连字整形的结果，带缓存。
    ///
    /// 各字符都必须由同一个字体绘制；有字符需要回退字体、或字体没有为该段换用任何字形时返回
    /// `None`，调用方逐字符绘制。
    pub(crate) fn ligatures(&mut self, kind: FontKind, run: &str) -> Option<Rc<Ligated>> {
        let mut slot = None;
        let mut plain = Vec::with_capacity(run.len());
        for c in run.chars() {
            let (found, glyph) = self.fonts.resolve(kind, c);
            if glyph == 0 || slot.is_some_and(|slot| slot != found) {
                return None;
            }
            slot = Some(found);
            plain.push(glyph);
        }
        let key = (slot?, Box::<str>::from(run));
        if let Some(found) = self.ligated[0].get(&key) {
            return found.clone();
        }
        if self.ligated[0].len() >= SEQUENCE_ENTRIES {
            self.ligated.swap(0, 1);
            self.ligated[0].clear();
        }
        let ligated = match self.ligated[1].remove(&key) {
            Some(kept) => kept,
            None => self.ligate(key.0, run, &plain).map(Rc::new),
        };
        self.ligated[0].insert(key, ligated.clone());
        ligated
    }

    /// 用槽位 `slot` 的字体为 `run` 整形；结果与逐字符的字形 `plain` 相同时返回 `None`。
    fn ligate(&mut self, slot: u8, run: &str, plain: &[u16]) -> Option<Ligated> {
        let face = self.fonts.face(slot)?;
        // 整形结果以字节标出所属的字符，换算成字符序号。
        let starts: Vec<usize> = run.char_indices().map(|(at, _)| at).collect();
        let mut shaper = self
            .shaper
            .builder(face.font())
            .size(1.0)
            .normalized_coords(&face.coords)
            .features(FEATURES)
            .build();
        shaper.add_str(run);
        let mut glyphs = Vec::with_capacity(plain.len());
        shaper.shape_with(|cluster| {
            let at = starts.partition_point(|&start| start < cluster.source.start as usize);
            glyphs.extend(cluster.glyphs.iter().map(|g| (at, g.id, g.x, g.y)));
        });
        let same = glyphs.iter().map(|g| g.1).eq(plain.iter().copied());
        (!same && !glyphs.is_empty()).then_some(Ligated { slot, glyphs })
    }
}
