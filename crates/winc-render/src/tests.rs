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

#[test]
fn base_rects_look_the_same_declared_or_not() {
    let new = || Renderer::offscreen(20.0, 10.0, 2.0, Color::hex(0x101010), None, None).unwrap();
    // 声明 `declared`（`None` 时不声明）后绘制底色矩形 `base` 和其上的一个矩形。
    let draw = |declared: Option<&[(Rect, Color)]>, base: &[(Rect, Color)]| {
        let mut renderer = new();
        if let Some(rects) = declared {
            renderer.base_next_frame(rects);
        }
        renderer.draw(|frame| {
            frame.base(base);
            frame.rect(Rect::new(4.0, 4.0, 8.0, 2.0), Color::hex(0xffffff));
        });
        renderer.image().unwrap().pixels.clone()
    };
    let base = [
        (Rect::new(0.0, 0.0, 6.5, 10.0), Color::hex(0x112233)),
        (Rect::new(5.0, 2.0, 15.0, 6.0), Color::hex(0x445566)),
    ];
    let plain = draw(None, &base);
    assert_eq!(plain[0] & 0xff_ffff, 0x112233);
    assert_eq!(plain[2 * 40 + 39] & 0xff_ffff, 0x101010);
    assert_eq!(plain[5 * 40 + 39] & 0xff_ffff, 0x445566);
    // 声明的与绘制的一致。
    assert_eq!(draw(Some(&base), &base), plain);
    // 不一致：画面上不留下声明的矩形。
    let other = [(Rect::new(0.0, 0.0, 20.0, 10.0), Color::hex(0xff0000))];
    assert_eq!(draw(Some(&other), &base), plain);
    // 存在非不透明的颜色：不声明，逐个混合。
    let sheer = [base[0], (base[1].0, base[1].1.alpha(0.5))];
    assert_eq!(draw(Some(&sheer), &sheer), draw(None, &sheer));
}

#[test]
fn declared_base_rects_are_painted_for_one_frame_only() {
    let mut renderer =
        Renderer::offscreen(20.0, 10.0, 2.0, Color::hex(0x101010), None, None).unwrap();
    let colors = |renderer: &Renderer| {
        let px = &renderer.image().unwrap().pixels;
        (px[0] & 0xff_ffff, px[px.len() - 1] & 0xff_ffff)
    };
    // 声明的矩形由渲染器绘制，与 `paint` 是否绘制无关。
    renderer.base_next_frame(&[(Rect::new(0.0, 0.0, 6.0, 10.0), Color::hex(0x112233))]);
    renderer.draw(|_| {});
    assert_eq!(colors(&renderer), (0x112233, 0x101010));
    // 声明只对下一帧有效。
    renderer.draw(|_| {});
    assert_eq!(colors(&renderer), (0x101010, 0x101010));
}

/// 字重对有字重轴的字体、没有字重轴的字体和轮廓由系统提供的字体都生效，字宽不变。
#[cfg(target_os = "macos")]
#[test]
fn font_weight_changes_the_ink_of_every_kind_of_font() {
    let sf = FontSpec::Path("/System/Library/Fonts/SFNSMono.ttf".into());
    let monaco = FontSpec::Path("/System/Library/Fonts/Monaco.ttf".into());
    let ink = |spec: &FontSpec, weight: u16, c: char| {
        let mut renderer =
            Renderer::offscreen(60.0, 40.0, 1.0, Color::hex(0x000000), None, None).unwrap();
        renderer.set_fonts(Some(spec), None, None, weight).unwrap();
        let mut advance = 0.0;
        renderer.draw(|frame| {
            let all = frame.bounds();
            let text = c.to_string();
            advance = frame.text(
                FontKind::Mono,
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
    for (spec, c, has_axis) in [(&sf, 'H', true), (&monaco, 'H', false), (&sf, '中', false)] {
        let (regular, width) = ink(spec, FONT_WEIGHT_NORMAL, c);
        let (bold, bold_width) = ink(spec, 700, c);
        let (light, light_width) = ink(spec, 300, c);
        assert!(
            bold as f32 > regular as f32 * 1.15,
            "{c}：{regular} → {bold}"
        );
        assert_eq!((bold_width, light_width), (width, width), "{c}");
        if has_axis {
            assert!(
                (light as f32) < regular as f32 * 0.95,
                "{c}：{regular} → {light}"
            );
        } else {
            assert_eq!(light, regular, "{c}");
        }
    }
}

/// 缩放绘制位图：每个目标像素取最近的源像素，超出裁剪范围的部分不绘制。
#[test]
fn scaled_pixels_take_the_nearest_source_pixel() {
    // 2 × 2 的位图：红、绿、蓝、白，放大到 8 × 8。
    let rgba = [
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let mut renderer =
        Renderer::offscreen(12.0, 12.0, 1.0, Color::hex(0x000000), None, None).unwrap();
    renderer.draw(|frame| {
        let clip = Rect::new(0.0, 0.0, 12.0, 9.0);
        frame.pixels_scaled(Rect::new(2.0, 2.0, 8.0, 8.0), 2, 2, &rgba, clip);
    });
    let image = renderer.image().unwrap();
    let at = |x: usize, y: usize| image.pixels[y * image.width as usize + x] & 0xff_ffff;
    assert_eq!((at(2, 2), at(5, 5)), (0xff0000, 0xff0000));
    assert_eq!((at(6, 2), at(9, 5)), (0x00ff00, 0x00ff00));
    assert_eq!((at(2, 6), at(5, 8)), (0x0000ff, 0x0000ff));
    assert_eq!(at(9, 8), 0xffffff);
    // 位图之外和裁剪范围之外保持底色。
    assert_eq!((at(1, 2), at(10, 2), at(5, 9)), (0, 0, 0));
}

/// 放大到数万像素宽时，远离左端的源像素边界仍落在准确的位置：
/// 横向步进的舍入误差不累积到可见的偏移。
#[test]
fn scaled_pixels_stay_aligned_far_from_the_left_edge() {
    // 3 × 1 的位图：红、绿、蓝，放大到 90000 像素宽；绿与蓝的边界在第 60000 个像素处。
    let rgba = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255];
    let mut renderer =
        Renderer::offscreen(8.0, 1.0, 1.0, Color::hex(0x000000), None, None).unwrap();
    renderer.draw(|frame| {
        let rect = Rect::new(-59_996.0, 0.0, 90_000.0, 1.0);
        frame.pixels_scaled(rect, 3, 1, &rgba, Rect::new(0.0, 0.0, 8.0, 1.0));
    });
    let image = renderer.image().unwrap();
    let row: Vec<u32> = image.pixels[..8].iter().map(|p| p & 0xff_ffff).collect();
    assert_eq!(row[..4], [0x00ff00; 4]);
    assert_eq!(row[4..], [0x0000ff; 4]);
}

/// 半透明的棋盘格只绘制裁剪范围内的方格：方格的颜色按其在整个矩形中的行列决定，与矩形超出
/// 裁剪范围的部分无关。
#[test]
fn translucent_checkers_draw_only_the_visible_cells() {
    let (first, second) = (Color::hex(0xff0000), Color::hex(0x0000ff));
    let draw = |rect: Rect| {
        let mut renderer =
            Renderer::offscreen(8.0, 8.0, 1.0, Color::hex(0x000000), None, None).unwrap();
        renderer.draw(|frame| {
            let clip = Rect::new(2.0, 2.0, 4.0, 4.0);
            frame.with_opacity(0.5, |frame| frame.checker(rect, 2.0, (first, second), clip));
        });
        renderer.image().unwrap().pixels.clone()
    };
    // 矩形的左上角在裁剪范围之外 2 格（4 像素）处：(2, 2) 位于第 3 行第 3 列，为 `first`。
    let pixels = draw(Rect::new(-2.0, -2.0, 12.0, 12.0));
    let at = |x: usize, y: usize| pixels[y * 8 + x] & 0xff_ffff;
    // `second` 先铺满整个矩形，`first` 再叠在自己的方格上：只有 `first` 的方格带红色。
    let (red, blue) = (at(2, 2), at(4, 2));
    assert!(red >> 16 > 0, "{red:06x}");
    assert!(blue & 0xff > 0 && blue >> 16 == 0, "{blue:06x}");
    assert_eq!((at(4, 4), at(2, 4)), (red, blue));
    // 裁剪范围之外保持底色。
    assert_eq!((at(1, 2), at(6, 2), at(2, 6)), (0, 0, 0));
    // 矩形向四周延伸到约一亿个方格时画面不变，且立即完成。
    assert_eq!(
        draw(Rect::new(-20_002.0, -20_002.0, 40_012.0, 40_012.0)),
        pixels
    );
}

/// 背景图在窗口背景、声明的底色和之后以这些颜色填充的矩形之下透出，颜色为「图片 × 不透明度 +
/// 底色 × (1 − 不透明度)」；其余颜色照常绘制。底色声明与否画面相同。
#[test]
fn the_wallpaper_shows_under_surface_colors_only() {
    let (bg, card, other) = (
        Color::hex(0x202020),
        Color::hex(0x404040),
        Color::hex(0x808080),
    );
    // 2 × 2 的图片铺满 8 × 8 的画布：红、绿、蓝、白各占四分之一。
    let rgba = [
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let draw = |declare: bool| {
        let mut renderer = Renderer::offscreen(8.0, 8.0, 1.0, bg, None, None).unwrap();
        renderer.set_wallpaper(
            Wallpaper::new((2, 2), &rgba, 0.5, WallpaperFit::Fill).map(Arc::new),
            &[bg, card],
        );
        assert_eq!(renderer.wallpaper_size(), Some((2, 2)));
        let rects = [(Rect::new(0.0, 0.0, 4.0, 8.0), card)];
        if declare {
            renderer.base_next_frame(&rects);
        }
        renderer.draw(|frame| {
            frame.base(&rects);
            frame.rect(Rect::new(6.0, 0.0, 2.0, 2.0), other);
            // 再次以背景色填充：仍然透出背景图。
            frame.rect(Rect::new(4.0, 6.0, 2.0, 2.0), bg);
            // 圆角外侧修整为背景色：同样透出背景图。
            frame.round_corners(Rect::new(0.0, 0.0, 4.0, 8.0), 2.0, bg);
        });
        renderer.image().unwrap().pixels.clone()
    };
    let image = draw(true);
    assert_eq!(image, draw(false));
    let at = |x: usize, y: usize| image[y * 8 + x] & 0xff_ffff;
    // 底色矩形内：红色和蓝色的图片加上卡片色各一半。
    assert_eq!((at(2, 2), at(2, 6)), (0x9f2020, 0x20209f));
    // 窗口背景：绿色和白色的图片加上背景色各一半；重复填充的一块与周围相同。
    assert_eq!(
        (at(5, 2), at(7, 7), at(5, 7)),
        (0x108f10, 0x8f8f8f, 0x8f8f8f)
    );
    // 不是底色的颜色照常绘制。
    assert_eq!(at(6, 0), 0x808080);
    // 圆角外侧的像素向背景色之下的图片过渡（弧线上的像素按覆盖比例混合）。
    let corner = at(0, 0);
    assert!((0x8f1010..0x9f2020).contains(&corner), "{corner:06x}");

    // 不透明度为 1 时只剩图片，为 0 时只剩底色；两端各通道都不溢出。
    for (opacity, expected) in [(1.0, 0xff0000), (0.0, 0x202020)] {
        let mut renderer = Renderer::offscreen(8.0, 8.0, 1.0, bg, None, None).unwrap();
        renderer.set_wallpaper(
            Wallpaper::new((2, 2), &rgba, opacity, WallpaperFit::Fill).map(Arc::new),
            &[bg],
        );
        renderer.draw(|_| {});
        assert_eq!(renderer.image().unwrap().pixels[0] & 0xff_ffff, expected);
    }

    // 图片与画布的宽高比不同时等比缩放、居中裁剪：4 × 2 的图片铺到 8 × 8 的画布上，只显示中间
    // 两列，左右两列被裁掉。
    let wide = [
        [9, 9, 9, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [9, 9, 9, 255], //
        [9, 9, 9, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
        [9, 9, 9, 255],
    ]
    .concat();
    let mut renderer = Renderer::offscreen(8.0, 8.0, 1.0, bg, None, None).unwrap();
    renderer.set_wallpaper(
        Wallpaper::new((4, 2), &wide, 1.0, WallpaperFit::Fill).map(Arc::new),
        &[bg],
    );
    renderer.draw(|_| {});
    let covered = renderer.image().unwrap().pixels.clone();
    let at = |x: usize, y: usize| covered[y * 8 + x] & 0xff_ffff;
    assert_eq!(
        (at(0, 0), at(3, 3), at(4, 0), at(7, 3)),
        (0xff0000, 0xff0000, 0x00ff00, 0x00ff00)
    );
    assert_eq!((at(0, 4), at(7, 7)), (0x0000ff, 0xffffff));

    // 取消以后恢复为纯色。
    let mut renderer = Renderer::offscreen(8.0, 8.0, 1.0, bg, None, None).unwrap();
    renderer.set_wallpaper(
        Wallpaper::new((2, 2), &rgba, 0.5, WallpaperFit::Fill).map(Arc::new),
        &[bg],
    );
    renderer.set_wallpaper(None, &[bg]);
    renderer.draw(|_| {});
    assert!(
        renderer
            .image()
            .unwrap()
            .pixels
            .iter()
            .all(|p| p & 0xff_ffff == 0x202020)
    );
    assert!(Wallpaper::new((2, 2), &rgba[..8], 0.5, WallpaperFit::Fill).is_none());
}

/// 各显示方式下图片在画布上的位置；图片铺不满时其余部分只有底色，浮层不透出背景图。
#[test]
fn the_wallpaper_is_placed_by_its_fit() {
    let (bg, card) = (Color::hex(0x202020), Color::hex(0x404040));
    // 2 × 1 的图片：左红右蓝，完全不透明。
    let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
    let draw = |fit, scale: f32, paint: &dyn Fn(&mut Frame<'_>)| {
        let side = 8.0 / scale;
        let mut renderer = Renderer::offscreen(side, side, scale, bg, None, None).unwrap();
        let wallpaper = Wallpaper::new((2, 1), &rgba, 1.0, fit).map(Arc::new);
        renderer.set_wallpaper(wallpaper, &[bg, card]);
        renderer.draw(|frame| paint(frame));
        let image = renderer.image().unwrap();
        assert_eq!((image.width, image.height), (8, 8));
        image
            .pixels
            .iter()
            .map(|p| p & 0xff_ffff)
            .collect::<Vec<_>>()
    };
    let (red, blue) = (0xff0000, 0x0000ff);

    // 适应：图片缩放为 8 × 4，上下各两行只有底色，包括之后以底色填充的部分。
    let left = Rect::new(0.0, 0.0, 4.0, 8.0);
    let image = draw(WallpaperFit::Fit, 1.0, &|frame| {
        frame.base(&[(left, card)]);
        frame.rect(Rect::new(6.0, 0.0, 2.0, 4.0), bg);
    });
    let at = |x: usize, y: usize| image[y * 8 + x];
    assert_eq!(
        (at(0, 1), at(5, 1), at(7, 1)),
        (0x404040, 0x202020, 0x202020)
    );
    assert_eq!(
        (at(0, 2), at(3, 5), at(4, 2), at(7, 3)),
        (red, red, blue, blue)
    );
    assert_eq!((at(0, 6), at(7, 7)), (0x404040, 0x202020));

    // 拉伸：铺满画布，不保持宽高比。
    let image = draw(WallpaperFit::Stretch, 1.0, &|_| {});
    assert_eq!(
        (image[0], image[3], image[4], image[63]),
        (red, red, blue, blue)
    );

    // 平铺：图层的一个像素对应一个逻辑像素，缩放系数为 2 时占 2 × 2 个物理像素。
    let image = draw(WallpaperFit::Tile, 2.0, &|_| {});
    assert_eq!(image[..8], [red, red, blue, blue, red, red, blue, blue]);
    assert_eq!(image[56..], image[..8]);

    // 浮层的底色与界面的底色相同时也不透出背景图。
    let image = draw(WallpaperFit::Fill, 1.0, &|frame| {
        frame.rect(Rect::new(0.0, 0.0, 2.0, 2.0), bg);
        frame.end_wallpaper();
        frame.rect(Rect::new(2.0, 0.0, 2.0, 2.0), bg);
    });
    assert_eq!((image[0], image[2], image[4]), (red, 0x202020, blue));
}
