//! `Renderer` 的测试：整帧绘制的透明度与裁剪、模拟粗体和斜体、多个窗口共用的字形缓存，
//! 以及由系统提供的字形轮廓。

use super::*;

/// 用字体 `font` 的 `kind` 字形画字符 `c`，返回字形上半部分与下半部分墨迹的平均横坐标
/// （物理像素）。
#[cfg(target_os = "macos")]
fn lean(font: &str, kind: FontKind, c: char) -> (f32, f32) {
    let spec = FontSpec::Path(font.into());
    let mut renderer =
        Renderer::offscreen(40.0, 40.0, 1.0, Color::hex(0x000000), Some(&spec), None).unwrap();
    renderer.draw(|frame| {
        let all = frame.bounds();
        frame.cell_char(kind, 30.0, 5.0, 32.0, 30.0, 1, c, Color::hex(0xffffff), all);
    });
    let image = renderer.image().unwrap();
    let w = image.width as usize;
    let mean_x = |rows: std::ops::Range<usize>| {
        let (mut sum, mut n) = (0.0, 0.0);
        for y in rows {
            for x in 0..w {
                let ink = (image.pixels[y * w + x] & 0xff) as f32;
                sum += ink * x as f32;
                n += ink;
            }
        }
        sum / n
    };
    (mean_x(6..18), mean_x(18..30))
}

#[test]
fn opacity_fades_everything_drawn_inside_it() {
    let mut renderer =
        Renderer::offscreen(20.0, 10.0, 1.0, Color::hex(0x000000), None, None).unwrap();
    renderer.draw(|frame| {
        frame.with_opacity(0.5, |frame| {
            frame.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::hex(0xffffff));
            // 嵌套时相乘：0.5 × 0.5。
            frame.with_opacity(0.5, |frame| {
                frame.rect(Rect::new(10.0, 0.0, 10.0, 10.0), Color::hex(0xffffff));
            });
        });
    });
    let image = renderer.image().unwrap();
    let red = |x: usize| image.pixels[5 * 20 + x] & 0xff;
    assert!((126..=129).contains(&red(5)), "{}", red(5));
    assert!((62..=66).contains(&red(15)), "{}", red(15));
}

#[test]
fn zoom_scales_drawing_without_resizing_the_image() {
    let mut renderer =
        Renderer::offscreen(40.0, 20.0, 2.0, Color::hex(0x000000), None, None).unwrap();
    // 物理尺寸不变，逻辑尺寸按倍数变小；反复设置不累积误差。
    renderer.set_zoom(1.25);
    assert_eq!(renderer.scale_factor(), 2.5);
    assert_eq!(renderer.logical_size(), (32.0, 16.0));
    renderer.set_zoom(2.0);
    renderer.set_zoom(1.0);
    assert_eq!(renderer.logical_size(), (40.0, 20.0));

    // 1 逻辑像素宽的竖线在 2 倍缩放下占 4 个物理像素。
    renderer.set_zoom(2.0);
    renderer.draw(|frame| frame.rect(Rect::new(1.0, 0.0, 1.0, 10.0), Color::hex(0xffffff)));
    let image = renderer.image().unwrap();
    let lit = (0..image.width as usize)
        .filter(|&x| image.pixels[x] & 0xff > 0)
        .count();
    assert_eq!((image.width, lit), (80, 4));
}

#[test]
fn faux_bold_is_heavier_and_wider() {
    // 西文取自界面字体，中文落到后备字体（macOS 15 起苹方的轮廓由系统提供）：两种字形都加粗。
    for c in ['H', '中'] {
        let ink = |bold: bool| {
            let mut renderer =
                Renderer::offscreen(60.0, 40.0, 1.0, Color::hex(0x000000), None, None).unwrap();
            let mut advance = 0.0;
            renderer.draw(|frame| {
                let all = frame.bounds();
                let faux = Faux {
                    bold,
                    italic: false,
                };
                advance = frame.advance_faux(FontKind::Ui, faux, 30.0, c);
                let text = c.to_string();
                frame.text_faux(
                    FontKind::Ui,
                    faux,
                    30.0,
                    5.0,
                    32.0,
                    &text,
                    Color::hex(0xffffff),
                    all,
                );
            });
            let image = renderer.image().unwrap();
            let sum: u32 = image.pixels.iter().map(|p| p & 0xff).sum();
            (sum, advance)
        };
        let (regular, regular_advance) = ink(false);
        let (bold, bold_advance) = ink(true);
        assert!(
            bold as f32 > regular as f32 * 1.15,
            "{c}：{regular} → {bold}"
        );
        assert!(bold_advance > regular_advance, "{c}");
    }
}

/// 模拟的粗体不会把任何字母变细：组合字形（`i`、`j`）的轮廓方向不可靠，不能按它偏移。
///
/// 没有装 `JetBrains Mono` 的机器只检查默认字体。
#[test]
fn faux_bold_thickens_every_letter() {
    let check = |spec: Option<&FontSpec>| {
        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            let ink = |bold: bool| {
                let mut renderer =
                    Renderer::offscreen(60.0, 50.0, 2.0, Color::hex(0x000000), None, spec).unwrap();
                renderer.draw(|frame| {
                    let all = frame.bounds();
                    let faux = Faux {
                        bold,
                        italic: false,
                    };
                    let text = c.to_string();
                    let white = Color::hex(0xffffff);
                    frame.text_faux(FontKind::Ui, faux, 30.0, 5.0, 35.0, &text, white, all);
                });
                let image = renderer.image().unwrap();
                image.pixels.iter().map(|p| p & 0xff).sum::<u32>()
            };
            let (regular, bold) = (ink(false), ink(true));
            assert!(
                bold as f32 > regular as f32 * 1.1,
                "{spec:?} {c}：{regular} → {bold}"
            );
        }
    };
    check(None);
    let jetbrains = FontSpec::Name("JetBrains Mono".into());
    if jetbrains.resolve().is_some() {
        check(Some(&jetbrains));
    }
}

#[test]
#[cfg(target_os = "macos")]
fn fonts_without_italics_are_slanted_to_the_right() {
    // Monaco 没有斜体：正体竖直，斜体由正体斜切而成，上半部分比下半部分靠右。
    let monaco = "/System/Library/Fonts/Monaco.ttf";
    let (top, bottom) = lean(monaco, FontKind::Mono, '|');
    assert!((top - bottom).abs() < 0.5, "正体是竖直的：{top} {bottom}");
    let (top, bottom) = lean(monaco, FontKind::MonoItalic, '|');
    assert!(top > bottom + 1.0, "斜体向右倾：{top} {bottom}");
    // 注释里的中文落到后备字体（苹方，由系统提供轮廓）上，同样斜切。
    let (upright_top, upright_bottom) = lean(monaco, FontKind::Mono, '中');
    let (top, bottom) = lean(monaco, FontKind::MonoItalic, '中');
    assert!(
        top - bottom > upright_top - upright_bottom + 1.0,
        "中文也向右倾：{top} {bottom}"
    );
}

#[test]
fn cached_strokes_look_the_same_as_direct_ones() {
    use std::f32::consts::PI;
    // 同一个图标画在相差整数个物理像素的两处：第二次使用第一次存下的蒙版。结果与直接逐像素
    // 描边完全相同，半透明的颜色也相同（蒙版不经过文字的覆盖率映射）。
    let icon = |x: f32| {
        [
            Stroke::Circle(x + 6.0, 7.3, 4.5),
            Stroke::Arc(x + 6.0, 7.3, 2.0, 0.0, 1.75 * PI),
            Stroke::Line(x + 1.0, 13.3, x + 11.0, 13.3),
        ]
    };
    let color = Color::hex(0xffcc00).alpha(0.8);
    let background = Color::hex(0x202020);
    let mut renderer = Renderer::offscreen(40.0, 20.0, 2.0, background, None, None).unwrap();
    for _ in 0..2 {
        renderer.draw(|frame| {
            let all = frame.bounds();
            frame.stroke(&icon(2.25), 1.25, color, all);
            frame.stroke(&icon(22.25), 1.25, color, all);
        });
    }
    let image = renderer.image().unwrap();

    let (w, h) = (image.width as usize, image.height as usize);
    let mut px = vec![0; w * h];
    let mut canvas = Canvas {
        px: &mut px,
        width: w,
        height: h,
        stride: w,
    };
    canvas.fill(background.argb());
    let all = IRect::new(0, 0, w as i32, h as i32);
    for x in [2.25, 22.25] {
        canvas.stroke(&icon(x), 1.25, 2.0, color.argb(), all);
    }
    assert!(image.pixels == px);
}

#[test]
fn nothing_is_drawn_outside_within() {
    let mut renderer = Renderer::offscreen(40.0, 40.0, 1.0, Color::hex(0x000000), None, None)
        .expect("测试机上有系统字体");
    let clip = Rect::new(10.0, 10.0, 20.0, 20.0);
    renderer.draw(|frame| {
        frame.within(clip, |frame| {
            // 每种绘制调用都传入一个比 `within` 范围更大的裁剪框。
            let all = frame.bounds();
            frame.rect_clipped(all, Color::hex(0xffffff), 0.0, all);
            frame.rect_clipped(all, Color::hex(0xffffff), 4.0, all);
            let line = [Stroke::Line(0.0, 20.0, 40.0, 20.0)];
            frame.stroke(&line, 2.0, Color::hex(0xffffff), all);
            frame.text(
                FontKind::Ui,
                30.0,
                0.0,
                35.0,
                "WWW",
                Color::hex(0xffffff),
                all,
            );
            // 嵌套的 `within` 不会比外层更大。
            frame.within(all, |frame| frame.rect(all, Color::hex(0xffffff)));
        });
        // 离开 `within` 后恢复不受限制的绘制。
        frame.rect(Rect::new(0.0, 0.0, 2.0, 2.0), Color::hex(0xff0000));
    });

    let image = renderer.image().unwrap();
    let at = |x: u32, y: u32| image.pixels[(y * image.width + x) as usize] & 0xff_ffff;
    assert_eq!(at(0, 0), 0xff0000);
    for (x, y) in [(5, 20), (35, 20), (20, 5), (20, 35), (9, 9), (30, 30)] {
        assert_eq!(at(x, y), 0, "({x}, {y}) 在限制外面");
    }
    assert_eq!(at(20, 20), 0xffffff);
}

#[cfg(target_os = "macos")]
#[test]
fn system_outline_glyphs_sit_on_the_baseline() {
    // 由系统提供轮廓的字形与 swash 光栅化的字形在同一条基线上：`top` 是位图上边缘在基线以上
    // 的高度。「中」上至约 0.82 em、下至约 -0.11 em；「一」在 0.33–0.40 em 之间。
    let file = FontSpec::Name("苹方-简".into()).resolve().unwrap();
    let face = text::font::Face::open(&file).unwrap();
    let Some(system) = &face.system else {
        return; // 旧版 macOS 的苹方由 swash 读取。
    };
    let charmap = face.font().charmap();
    let zhong = system
        .fill(charmap.map('中'), 100.0, 0.0, 0.0, 0.0)
        .unwrap();
    assert!((80..=84).contains(&zhong.top), "{}", zhong.top);
    assert!((92..=96).contains(&zhong.height), "{}", zhong.height);
    let yi = system
        .fill(charmap.map('一'), 100.0, 0.0, 0.0, 0.0)
        .unwrap();
    assert!((40..=41).contains(&yi.top), "{}", yi.top);
    assert!(
        system
            .fill(charmap.map(' '), 100.0, 0.0, 0.0, 0.0)
            .is_none()
    );
    // 有覆盖率：笔画中间是实心的（覆盖率接近 255）。
    assert!(zhong.data.iter().any(|&a| a > 250));
}

#[cfg(target_os = "macos")]
#[test]
fn pingfang_draws_as_the_ui_font() {
    // 苹方的字形可能只放在 `hvgl` 表中，由系统提供轮廓；绘制出来的字必须有像素。
    let ui = FontSpec::Name("苹方-简".into());
    assert_eq!(ui.check(), Ok(()));
    let mut renderer = Renderer::offscreen(80.0, 40.0, 1.0, Color::hex(0), None, Some(&ui))
        .expect("测试机上有系统字体");
    renderer.draw(|frame| {
        let all = frame.bounds();
        frame.text(
            FontKind::Ui,
            24.0,
            4.0,
            30.0,
            "中文",
            Color::hex(0xffffff),
            all,
        );
    });
    let image = renderer.image().unwrap();
    let lit = image.pixels.iter().filter(|&&p| p & 0xff_ffff != 0).count();
    assert!(lit > 100, "只有 {lit} 个像素有颜色");
}

#[test]
fn windows_sharing_glyphs_draw_the_same_and_rasterize_once() {
    let draw = |renderer: &mut Renderer| {
        renderer.draw(|frame| {
            let all = frame.bounds();
            frame.text(
                FontKind::Ui,
                20.0,
                2.0,
                24.0,
                "Wg中",
                Color::hex(0xffffff),
                all,
            );
        });
        renderer.image().unwrap().pixels.clone()
    };
    let new = || Renderer::offscreen(60.0, 30.0, 2.0, Color::hex(0), None, None).unwrap();
    let mut first = new();
    let alone = draw(&mut first);
    let mut second = new();
    second.use_glyphs(first.shared_glyphs());
    assert!(Rc::ptr_eq(&first.glyphs, &second.glyphs));

    // 第二个窗口绘制同样的字：缓存里已经有了，不再多出字形，绘制结果相同。
    let before = first.glyphs.borrow().cached();
    assert_eq!(draw(&mut second), alone);
    assert_eq!(first.glyphs.borrow().cached(), before);
}
