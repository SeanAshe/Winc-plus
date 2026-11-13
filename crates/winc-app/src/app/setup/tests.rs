//! 首次设置启动动画的测试：图标各层的出现顺序、完成时与应用图标一致，图标淡出和内容淡入的
//! 先后。

use super::*;

/// 返回图像中不透明度大于 0 的像素个数。
fn visible(rgba: &[u8]) -> usize {
    let (pixels, _) = rgba.as_chunks::<4>();
    pixels.iter().filter(|px| px[3] > 0).count()
}

/// 返回图像中接近白色（丝带和圆点）的像素个数。
fn white(rgba: &[u8]) -> usize {
    let (pixels, _) = rgba.as_chunks::<4>();
    (pixels.iter())
        .filter(|px| px[3] > 200 && px[0] > 215 && px[1] > 215 && px[2] > 215)
        .count()
}

#[test]
fn the_logo_builds_up_layer_by_layer_and_ends_as_the_app_icon() {
    let size = 96;
    let plain = winc_preview::svg::load(ICON, None, None)
        .unwrap()
        .render(size, size)
        .rgba;
    // 完成时与应用图标相同：分组的不透明度经过取整，各通道至多相差 2。
    assert!(posed(1.0).is_some());
    let last = logo_frame(size, 1.0);
    let differing = (last.iter().zip(&plain))
        .filter(|(a, b)| a.abs_diff(**b) > 2)
        .count();
    assert!(last.len() == plain.len() && differing == 0, "{differing}");
    // 开始时什么都没有；底板先出现，此时还没有白色的丝带。
    assert_eq!(visible(&logo_frame(size, 0.0)), 0);
    let tile = logo_frame(size, 0.26);
    assert!(visible(&tile) > 0 && white(&tile) == 0);
    // 丝带从左到右显露：白色的像素逐渐增多，圆点最后出现。
    let (early, late) = (logo_frame(size, 0.45), logo_frame(size, 0.68));
    assert!(white(&early) > 0 && white(&late) > white(&early));
    assert!(white(&plain) > white(&late));
}

#[test]
fn finishing_fades_the_setup_out_before_the_start_page_fades_in() {
    assert_eq!(outro_opacity(0.0), (1.0, 0.0));
    assert_eq!(outro_opacity(1.0), (0.0, 1.0));
    // 「首次设置」先淡出，开始页随后淡入，两者不同时可见。
    for step in 0..=100 {
        let (leaving, entering) = outro_opacity(step as f32 / 100.0);
        assert!(leaving == 0.0 || entering == 0.0, "{step}");
    }
    let (leaving, _) = outro_opacity(0.2);
    let (_, entering) = outro_opacity(0.7);
    assert!(leaving > 0.0 && leaving < 1.0 && entering > 0.0 && entering < 1.0);
}

#[test]
fn the_logo_fades_out_before_the_content_fades_in_one_item_after_another() {
    // 图标先逐层出现，完成后完整地停留，再向上淡出，最后消失。
    let building = splash(0.25).unwrap();
    assert!(building.build > 0.0 && building.build < 1.0 && building.opacity == 1.0);
    let held = splash(0.56).unwrap();
    assert_eq!((held.build, held.opacity, held.rise), (1.0, 1.0, 0.0));
    let leaving = splash(0.72).unwrap();
    assert!(leaving.opacity < 1.0 && leaving.opacity > 0.0 && leaving.rise > 0.0);
    assert_eq!(splash(0.8), None);
    assert_eq!(splash(1.0), None);

    // 图标停留期间内容都不显示，结束时全部显示。
    for index in 0..9 {
        assert_eq!(content_opacity(CONTENT_START, index), 0.0);
        assert_eq!(content_opacity(1.0, index), 1.0);
    }
    // 同一时刻，排在后面的项更淡。
    let at = 0.80;
    assert!(content_opacity(at, 0) > content_opacity(at, 2));
    assert!(content_opacity(at, 2) > content_opacity(at, 5));
}
