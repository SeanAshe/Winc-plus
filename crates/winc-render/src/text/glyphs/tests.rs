//! `glyphs` 的测试辅助：读取字形缓存中的字形个数，供检查缓存复用的测试使用。
//!
//! 只在测试构建下编译。

use super::*;

impl GlyphCache {
    /// 返回缓存中的字形个数。
    pub(crate) fn cached(&self) -> usize {
        self.glyphs.len()
    }
}

#[test]
fn emoji_sequences_are_composed_by_the_emoji_font() {
    let fonts = Fonts::load(&[], &[], &[], 400).expect("系统里有等宽字体");
    let mut cache = GlyphCache::new(fonts);
    // 系统中没有彩色 emoji 字体时不做比较。
    if cache.fonts.emoji_face().is_none() || cache.shaped("😀\u{fe0f}").is_none() {
        return;
    }
    let size = 20.0;
    let single = cache.lookup(FontKind::Ui, size, '😀').advance;
    assert!(single > 0.0);
    for unit in [
        "❤\u{fe0f}",
        "👍\u{1f3fd}",
        "👨\u{200d}👩\u{200d}👧",
        "🇨🇳",
        "1\u{fe0f}\u{20e3}",
    ] {
        let Some(shaped) = cache.shaped(unit) else {
            continue;
        };
        // 这两种序列在各平台的 emoji 字体中都有合成的字形。国旗、家庭组合和键帽依字体而定：
        // 字体没有合成的字形时结果是几个字形，键帽可以由数字与边框两个字形叠成。
        if matches!(unit, "❤\u{fe0f}" | "👍\u{1f3fd}") {
            assert_eq!(shaped.glyphs.len(), 1, "{unit:?}");
        }
        assert!(shaped.advance() > 0.0, "{unit:?}");
        let width = cache.measure(FontKind::Ui, Faux::default(), size, unit);
        assert!((width - shaped.advance() * size).abs() < 0.01);
        // 前后有文字时宽度是各部分之和：测量和绘制用同一种切分。
        let around = cache.measure(FontKind::Ui, Faux::default(), size, &format!("a{unit}b"));
        let ends = cache.measure(FontKind::Ui, Faux::default(), size, "ab");
        assert!((around - ends - width).abs() < 0.01, "{unit:?}");
    }
    // 由两层叠成的图形（第二层向左退回）只占一层的宽度。
    let layered = Shaped {
        slot: 0,
        glyphs: vec![(1, 0.0, 0.0, 1.0), (2, -1.0, 0.0, 1.0)],
    };
    assert!((layered.advance() - 1.0).abs() < 1e-6);
    // 按文字显示的序列不由 emoji 字体合成，宽度是其中可见字符的宽度。
    assert!(cache.shaped("a\u{fe0f}").is_none());
    let plain = cache.measure(FontKind::Ui, Faux::default(), size, "a");
    let marked = cache.measure(FontKind::Ui, Faux::default(), size, "a\u{fe0f}");
    assert!((plain - marked).abs() < 0.01);
}

#[test]
fn the_sequence_cache_keeps_entries_in_use_across_generations() {
    let fonts = Fonts::load(&[], &[], &[], 400).expect("系统里有等宽字体");
    let mut cache = GlyphCache::new(fonts);
    let kept = "a\u{fe0f}";
    cache.shaped(kept);
    // 写入两代容量的不同序列，其间一直使用 `kept`：它随每次换代搬回新一代，总项数不超过两代。
    for code in 0..2 * SEQUENCE_ENTRIES as u32 {
        let c = char::from_u32(0x4e00 + code).unwrap();
        cache.shaped(&format!("{c}\u{fe0f}"));
        cache.shaped(kept);
        assert!(cache.sequences[0].contains_key(kept));
    }
    assert!(cache.sequences[0].len() <= SEQUENCE_ENTRIES);
    assert!(cache.sequences[1].len() <= SEQUENCE_ENTRIES);
}
