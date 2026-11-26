//! 选日韩文字体用的语言的测试：按系统语言选字形，应用指定的语言优先。

use super::*;

#[test]
fn chinese_japanese_and_korean_locales_pick_their_own_glyph_forms() {
    for (locale, language) in [
        ("zh_TW.UTF-8", "zh-tw"),
        ("zh_HK.UTF-8", "zh-hk"),
        ("zh_CN.UTF-8", "zh-cn"),
        ("ja_JP.UTF-8", "ja-jp"),
        ("ko_KR.UTF-8", "ko-kr"),
        ("zh", "zh-cn"),
        ("en_US.UTF-8", "zh-cn"),
        ("C", "zh-cn"),
        ("", "zh-cn"),
    ] {
        assert_eq!(cjk_language_of(locale), language, "{locale}");
    }
}

#[test]
fn the_app_language_overrides_the_system_language_for_cjk_glyphs() {
    assert!(set_cjk_language(Some("zh-cn")));
    assert_eq!(cjk_language(), "zh-cn");
    // 同一种语言再设一次不算变化，调用方不必重新加载字体。
    assert!(!set_cjk_language(Some("zh-cn")));
    // 不再指定时回到按系统语言选。
    assert!(set_cjk_language(None));
    assert_eq!(
        cjk_language(),
        cjk_language_of(
            &["LC_ALL", "LC_CTYPE", "LANG"]
                .iter()
                .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
                .unwrap_or_default()
        )
    );
}
