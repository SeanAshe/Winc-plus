//! SVG 解析与绘制的测试：属性值、形状、裁剪与蒙版、滤镜、图案、文字排版、资源上限。

use super::*;

/// 按 `w` × `h` 绘制，取坐标 (x, y) 处的像素。
fn pixel(svg: &str, w: u32, h: u32, x: u32, y: u32) -> [u8; 4] {
    let drawing = Drawing::parse(svg, None, Some(Arc::new(BoxGlyphs))).unwrap();
    let pixels = drawing.render(w, h);
    let i = ((y * w + x) * 4) as usize;
    pixels.rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn colors_lengths_and_transforms_parse() {
    assert_eq!(color("#f00"), Some([255, 0, 0, 255]));
    assert_eq!(color("#11223380"), Some([0x11, 0x22, 0x33, 0x80]));
    assert_eq!(color("rgb(0, 128, 255)"), Some([0, 128, 255, 255]));
    assert_eq!(color("rgba(100%,0%,0%,.5)"), Some([255, 0, 0, 128]));
    assert_eq!(color("hsl(120, 100%, 50%)"), Some([0, 255, 0, 255]));
    assert_eq!(color("RebeccaPurple"), Some([0x66, 0x33, 0x99, 255]));
    assert_eq!(color("nope"), None);
    assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0), "颜色名要排好序");

    assert_eq!(numbers("1-2.5.5e1,3 4"), vec![1.0, -2.5, 5.0, 3.0, 4.0]);
    assert_eq!(length("2in", 0.0), Some(192.0));
    assert_eq!(length("50%", 40.0), Some(20.0));
    assert_eq!(length("1em", 0.0), Some(16.0));
    assert_eq!(length("1e1", 0.0), Some(10.0));

    let t = transform("translate(10 20) scale(2)");
    assert_eq!(t.apply(1.0, 1.0), (12.0, 22.0));
    let (x, y) = transform("rotate(90 5 5)").apply(10.0, 5.0);
    assert!((x - 5.0).abs() < 1e-4 && (y - 10.0).abs() < 1e-4);
    let inverse = t.invert().unwrap();
    assert_eq!(inverse.apply(12.0, 22.0), (1.0, 1.0));
}

#[test]
fn extreme_filters_finish_quickly() {
    // 标准差比图还大时按图的尺寸封顶，否则 1e9 的标准差要遍历数十亿格。
    // 系数会算成 NaN，而 `clamp` 的上界是 NaN 时会 panic。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <defs><filter id="f"><feGaussianBlur stdDeviation="1e9"/></filter></defs>
            <rect width="8" height="8" fill="red" filter="url(#f)"/>
        </svg>"##;
    let start = std::time::Instant::now();
    let center = pixel(svg, 8, 8, 4, 4);
    assert!(center[0] > 200 && center[3] > 0, "{center:?}");
    assert!(start.elapsed().as_secs() < 5, "模糊画得太慢");

    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <defs><filter id="f"><feComposite in="SourceGraphic" in2="SourceGraphic" operator="arithmetic" k1="NaN"/></filter></defs>
            <rect width="8" height="8" fill="red" filter="url(#f)"/>
        </svg>"##;
    let _ = pixel(svg, 8, 8, 4, 4);
}

#[test]
fn odd_filter_numbers_do_not_panic_or_hang() {
    let svg = |primitive: &str| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
                <defs><filter id="f">{primitive}</filter></defs>
                <rect width="8" height="8" fill="red" filter="url(#f)"/></svg>"##
        )
    };
    // 偏移极大：`floor() as isize` 再相减会溢出；偏移出画布后什么都不剩。
    assert_eq!(alpha(&svg(r#"<feOffset dx="-1e30"/>"#), 8, 8, 4, 4), 0);
    // `NaN` 不是有效数值，按未写（0）处理。
    assert_eq!(
        pixel(&svg(r#"<feOffset dx="NaN"/>"#), 8, 8, 4, 4),
        [255, 0, 0, 255]
    );
    // 矩阵里的 1e39 在 `f32` 中是无穷大，乘以 0 得 NaN：透明度为 NaN 时按全透明处理。
    let matrix = r#"<feColorMatrix values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 1e39 0 0 0"/>"#;
    assert_eq!(alpha(&svg(matrix), 8, 8, 4, 4), 0);
    // 半径极大的膨胀：每个像素扫描一遍窗口要执行数十亿次。
    let start = std::time::Instant::now();
    let dilate = r#"<feMorphology operator="dilate" radius="1e9"/>"#;
    assert_eq!(pixel(&svg(dilate), 8, 8, 4, 4), [255, 0, 0, 255]);
    let erode = r#"<feMorphology radius="1e9"/>"#;
    assert_eq!(alpha(&svg(erode), 8, 8, 4, 4), 0);
    assert!(start.elapsed().as_secs() < 5, "腐蚀膨胀画得太慢");
}

#[test]
fn huge_shapes_are_only_rasterized_where_they_are_seen() {
    // 外框 20000×20000 对应 400 MB 的覆盖度缓冲（每像素 1 字节）；宽高乘积超过 `u32` 时
    // zeno 自己还会 panic。画布只有 8×8，只光栅化看得见的那一块，结果相同。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <rect width="20000" height="20000" fill="red"/>
        </svg>"##;
    let start = std::time::Instant::now();
    assert_eq!(pixel(svg, 8, 8, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(svg, 8, 8, 7, 7), [255, 0, 0, 255]);
    assert!(start.elapsed().as_secs() < 5, "画得太慢");

    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <rect width="65536" height="65537" fill="red" stroke="blue" stroke-width="2"/>
            <path d="M0 0 L70000 70000" stroke="blue" stroke-width="3"/>
        </svg>"##;
    assert_eq!(pixel(svg, 8, 8, 3, 0)[3], 255, "粗线照样画得出来");
}

#[test]
fn deep_nesting_is_rejected_before_parsing() {
    // 几千层 `<g>` 会让 XML 解析把后台线程（2 MB 栈）的栈用完，整个程序退出。
    let deep = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg">{}{}</svg>"#,
        "<g>".repeat(5000),
        "</g>".repeat(5000)
    );
    let parsed = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || Drawing::parse(&deep, None, None).map(|_| ()))
        .unwrap()
        .join()
        .unwrap();
    assert!(parsed.unwrap_err().contains("嵌套"));

    // 计算嵌套层数时不计入注释、CDATA、DOCTYPE、自闭合标签和属性值中的大于号。
    assert!(!nesting_exceeds(
        r#"<!DOCTYPE svg [<!ENTITY a "<g>">]><svg><!-- <g><g> --><![CDATA[<g>]]><?x <g>?><g a="<>"/><g></g></svg>"#,
        2
    ));
    assert!(nesting_exceeds("<svg><g><g></g></g></svg>", 2));
    assert!(!nesting_exceeds("<svg><g/><g></g><g></g></svg>", 2));
}

#[test]
fn doctype_entities_expand_as_plain_text_only() {
    // 命名空间、样式写在 DOCTYPE 实体中的 SVG；值中的引号不会提前结束属性。
    let svg = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator -->
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
	<!ENTITY ns_svg "http://www.w3.org/2000/svg">
	<!ENTITY st0 'fill:#ff0000;font-family:"A B"'>
]>
<svg xmlns="&ns_svg;" width="4" height="4"><rect width="4" height="4" style="&st0;"/></svg>"#;
    assert_eq!(pixel(svg, 4, 4, 1, 1), [255, 0, 0, 255]);

    // 行号与原文一致；注释、CDATA 中的引用不展开。预定义实体、参数实体、值为标签的实体
    // 和引用其他实体的实体都不支持。
    let text = "<!DOCTYPE svg [\n<!ENTITY a \"x\">\n<!ENTITY lt \"y\">\n<!ENTITY % p \"z\">\n\
                <!ENTITY g \"<g>\">\n<!ENTITY r \"&a;\">\n]>\n\
                <svg>&a;&lt;<!-- &a; --><![CDATA[&a;]]>&g;&r;&p;&a</svg>";
    let expanded = dtd::expand(text).unwrap();
    assert_eq!(expanded.matches('\n').count(), text.matches('\n').count());
    assert!(
        expanded.ends_with("\n<svg>x&lt;<!-- &a; --><![CDATA[&a;]]>&g;&r;&p;&a</svg>"),
        "{expanded}"
    );
    assert!(matches!(dtd::expand("<svg/>"), Ok(Cow::Borrowed(_))));
    // 格式错误的 DOCTYPE 原样交给 XML 解析，由 XML 解析报错。
    let broken = "<!DOCTYPE svg [<!ENTITY a \"x\"><svg>&a;</svg>";
    assert!(matches!(dtd::expand(broken), Ok(Cow::Borrowed(_))));
    assert!(Drawing::parse(broken, None, None).is_err());
}

#[test]
fn doctype_entities_cannot_nest_deeply_or_blow_up() {
    let parse = |svg: String| {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || Drawing::parse(&svg, None, None).map(|_| ()))
            .unwrap()
            .join()
            .unwrap()
    };
    // 回归：实体中带标签，嵌套层数在展开后相乘，使后台线程栈溢出。
    let mut entities = String::from("<!ENTITY e0 \"\">");
    for i in 1..=8 {
        let g = 1500;
        entities += &format!(
            "<!ENTITY e{i} \"{}&e{};{}\">",
            "<g>".repeat(g),
            i - 1,
            "</g>".repeat(g)
        );
    }
    let svg = |entities: &str, body: &str| {
        format!(
            r#"<!DOCTYPE svg [{entities}]><svg xmlns="http://www.w3.org/2000/svg">{body}</svg>"#
        )
    };
    assert!(parse(svg(&entities, "&e8;")).is_err());
    // 实体引用其他实体：每层放大 100 倍。
    let laughs = format!(
        "<!ENTITY a \"{}\"><!ENTITY b \"{}\"><!ENTITY c \"{}\">",
        "x".repeat(100),
        "&a;".repeat(100),
        "&b;".repeat(100)
    );
    assert!(parse(svg(&laughs, "<text>&c;</text>")).is_err());
    // 纯文本实体被引用多次：展开的总量受上限约束。
    let many = format!("<!ENTITY a \"{}\">", "x".repeat(10_000));
    assert_eq!(
        parse(svg(&many, &format!("<text>{}</text>", "&a;".repeat(1_000)))).unwrap_err(),
        winc_i18n::t!("preview-svg-entities-too-large")
    );
    assert!(parse(svg(&many, &format!("<text>{}</text>", "&a;".repeat(50)))).is_ok());
}

#[test]
fn repeated_effects_stay_bounded() {
    let start = std::time::Instant::now();
    // 图案层层嵌套，每层用上一层填十个矩形：图块数按层数指数增长。
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><defs><pattern id="p0" width="10" height="10" patternUnits="userSpaceOnUse"><rect width="5" height="5" fill="red"/></pattern>"#,
    );
    for k in 1..=6 {
        svg.push_str(&format!(
            r#"<pattern id="p{k}" width="200" height="200" patternUnits="userSpaceOnUse">"#
        ));
        for _ in 0..10 {
            svg.push_str(&format!(
                r##"<rect width="200" height="200" fill="url(#p{})"/>"##,
                k - 1
            ));
        }
        svg.push_str("</pattern>");
    }
    svg.push_str(r##"</defs><rect width="200" height="200" fill="url(#p6)"/></svg>"##);
    let _ = pixel(&svg, 200, 200, 1, 1);

    // 一个有一千个形状的蒙版、一个有一千个形状的裁剪，各被一千个元素引用。
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><defs><mask id="m">"#,
    );
    svg.push_str(&r#"<rect width="100" height="100" fill="white"/>"#.repeat(1000));
    svg.push_str(r#"</mask><clipPath id="c">"#);
    svg.push_str(&r#"<rect width="100" height="100"/>"#.repeat(1000));
    svg.push_str("</clipPath></defs>");
    svg.push_str(&r##"<rect width="100" height="100" fill="red" mask="url(#m)"/>"##.repeat(1000));
    svg.push_str(
        &r##"<rect width="100" height="100" fill="red" clip-path="url(#c)"/>"##.repeat(1000),
    );
    svg.push_str("</svg>");
    let drawing = Drawing::parse(&svg, None, None).unwrap();
    assert!(drawing.missing.iter().any(|m| m.contains("过多")));
    drawing.render(40, 40);

    // 有五十个模糊的滤镜被一千个元素引用。
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50"><defs><filter id="f">"#,
    );
    svg.push_str(&r#"<feGaussianBlur stdDeviation="30"/>"#.repeat(50));
    svg.push_str("</filter></defs>");
    svg.push_str(&r##"<rect width="50" height="50" fill="red" filter="url(#f)"/>"##.repeat(1000));
    svg.push_str("</svg>");
    let _ = pixel(&svg, 50, 50, 1, 1);

    assert!(
        start.elapsed().as_secs() < 20,
        "画得太慢：{:?}",
        start.elapsed()
    );
}

#[test]
fn use_bombs_and_self_references_stay_bounded() {
    // 每一层引用上一层十次，最内层是空的：展开后有 10 亿个元素，形状数却达不到上限。
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/><defs><g id="l0"/>"#,
    );
    for i in 1..=9 {
        svg.push_str(&format!(r#"<g id="l{i}">"#));
        for _ in 0..10 {
            svg.push_str(&format!(r##"<use href="#l{}"/>"##, i - 1));
        }
        svg.push_str("</g>");
    }
    svg.push_str(r##"</defs><use href="#l9"/></svg>"##);
    let start = std::time::Instant::now();
    assert_eq!(pixel(&svg, 4, 4, 1, 1), [255, 0, 0, 255]);
    assert!(start.elapsed().as_secs() < 30, "展开得太久");

    // 引用自己所在的组：不绘制，也不会多出一个向右偏移的副本。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <g id="a"><rect width="2" height="4"/><use href="#a" x="2"/></g></svg>"##;
    assert_eq!((alpha(svg, 4, 4, 1, 1), alpha(svg, 4, 4, 3, 1)), (255, 0));

    // 同一张图被多次引用：只解码一次，各处共用同一份像素。
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image id="i" width="2" height="2" href="data:image/png;base64,{png}"/><use href="#i" x="2"/></svg>"##
    );
    let drawing = Drawing::parse(&svg, None, None).unwrap();
    let images: Vec<&Arc<Pixels>> = drawing
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Image(image) => Some(&image.pixels),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 2);
    assert!(Arc::ptr_eq(images[0], images[1]));
}

#[test]
fn malformed_numbers_and_colors_do_not_panic() {
    // 含非 ASCII 字符时按字节扫描可能停在字符中间：切分字符串、按字节切十六进制数字都不能
    // 落在非字符边界上。
    assert_eq!(numbers("0 0 24 24\u{a0}"), vec![0.0, 0.0, 24.0, 24.0]);
    assert_eq!(numbers("中 1"), vec![1.0]);
    assert_eq!(numbers("1，2"), vec![1.0, 2.0]);
    assert_eq!(color("#éa"), None);
    assert_eq!(color("#中文"), None);
    // `preserveAspectRatio` 的对齐方式按前 4 个字节分成横、竖两半，第 4 个字节可能落在字符
    // 中间（`é` 占第 3、4 个字节）：认不出时居中。
    for value in ["xMié", "中文字"] {
        assert_eq!(aspect(Some(value)).align, (0.5, 0.5), "{value}");
    }
    assert_eq!(aspect(Some("xMinYMié")).align, (0.0, 0.5));
    assert_eq!(aspect(Some("xMaxYMin slice")).align, (1.0, 0.0));
    // `e` 后面没有数字时不是指数：`1em` 读出 1，与 `length` 的规则相同。
    assert_eq!(numbers("1em 2"), vec![1.0, 2.0]);
    assert_eq!(numbers("3e,4"), vec![3.0, 4.0]);
}

#[test]
fn shapes_share_one_gradient_table() {
    // 引用同一个渐变、透明度相同的形状共用一张颜色表；透明度不同时各有一张。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient>
            <rect width="5" height="5" fill="url(#g)"/><rect x="5" width="5" height="5" fill="url(#g)"/>
            <rect y="5" width="5" height="5" fill="url(#g)" fill-opacity="0.5"/></svg>"##;
    let drawing = Drawing::parse(svg, None, None).unwrap();
    let tables: Vec<&Arc<[[u8; 4]; 256]>> = drawing
        .items
        .iter()
        .map(|item| match item {
            Item::Shape(Shape {
                fill: Some((Paint::Gradient(g), _)),
                ..
            }) => &g.table,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(Arc::ptr_eq(tables[0], tables[1]));
    assert!(!Arc::ptr_eq(tables[0], tables[2]));
    assert_eq!(tables[2][0][3], 128);
}

#[test]
fn root_size_ignores_zero_width_and_height() {
    // 宽为 0 时按没写处理：宽按 viewBox 的宽高比由高推算，
    // 而不是把 viewBox 的宽当成写明的宽。
    let size = |attrs: &str| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" {attrs}/>"#);
        let drawing = Drawing::parse(&svg, None, None).unwrap();
        (drawing.width, drawing.height)
    };
    assert_eq!(
        size(r#"width="0" height="40" viewBox="0 0 10 20""#),
        (20.0, 40.0)
    );
    assert_eq!(size(r#"width="30" viewBox="0 0 10 20""#), (30.0, 60.0));
    assert_eq!(size(r#"height="50%" viewBox="0 0 10 20""#), (10.0, 20.0));
    assert_eq!(size(r#"width="-5""#), DEFAULT_SIZE);
    assert_eq!(size(r#"width="12" height="34""#), (12.0, 34.0));
}

#[test]
fn shapes_fill_stroke_and_view_box() {
    // viewBox 缩放：10×10 的坐标画在 20×20 上，红方块占左上角一半。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <rect width="5" height="5" fill="red"/>
            <circle cx="7.5" cy="7.5" r="2" fill="none" stroke="#00f" stroke-width="1"/>
        </svg>"##;
    let drawing = Drawing::parse(svg, None, None).unwrap();
    assert_eq!((drawing.width, drawing.height), (10.0, 10.0));
    assert!(drawing.missing.is_empty());
    assert_eq!(pixel(svg, 20, 20, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(svg, 20, 20, 12, 2)[3], 0);
    // 圆的描边在半径 2（画布上 4 像素）那一圈，圆心是空的。
    assert_eq!(pixel(svg, 20, 20, 15, 15)[3], 0);
    let ring = pixel(svg, 20, 20, 15, 11);
    assert!(ring[2] > 200 && ring[3] > 200, "{ring:?}");

    // 虚线随 viewBox 一起放大：「1 1」放大十倍后每段 10 像素。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 1">
            <path d="M0 0.5H10" stroke="black" stroke-dasharray="1 1"/></svg>"#;
    let dashes: Vec<u8> = [5, 15, 25, 35].map(|x| alpha(svg, 100, 10, x, 5)).into();
    assert_eq!(dashes, [255, 0, 255, 0]);

    // 覆盖变换、`<use>` 引用、样式表、继承和 `currentColor`。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20" height="20">
            <style>.on { fill: #0f0 } #x { fill: blue }</style>
            <defs><rect id="r" width="4" height="4"/></defs>
            <g fill="red" color="#ff00ff" transform="translate(10 0)">
                <use xlink:href="#r"/>
                <use href="#r" y="10" class="on"/>
                <path d="M0 16h4v4h-4z" fill="currentColor"/>
            </g>
            <rect id="x" class="on" x="0" y="0" width="4" height="4"/>
        </svg>"##;
    assert_eq!(pixel(svg, 20, 20, 11, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(svg, 20, 20, 11, 11), [0, 255, 0, 255]);
    assert_eq!(pixel(svg, 20, 20, 11, 17), [255, 0, 255, 255]);
    // id 选择器优先于类选择器。
    assert_eq!(pixel(svg, 20, 20, 1, 1), [0, 0, 255, 255]);
}

#[test]
fn opacity_gradients_and_what_is_missing() {
    // 整组半透明：两个重叠的方块先画在同一层上再整体半透明，重叠处不会更深。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <g opacity="0.5"><rect width="6" height="10"/><rect x="4" width="6" height="10"/></g>
        </svg>"#;
    assert_eq!(pixel(svg, 10, 10, 5, 5)[3], pixel(svg, 10, 10, 1, 5)[3]);
    assert!((120..=135).contains(&pixel(svg, 10, 10, 5, 5)[3]));

    // 只有一个纯色形状的半透明不再单独分层，透明度直接乘进颜色；与分层绘制的结果至多差 1。
    let single = |attrs: &str| {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <linearGradient id="g"><stop stop-color="red"/><stop offset="1"/></linearGradient>
                <rect width="10" height="10" fill="#08f"/>
                <path d="M1 1h8v8h-8z" opacity="0.3" {attrs}/></svg>"##
        );
        let drawing = Drawing::parse(&svg, None, None).unwrap();
        let groups = drawing.items.iter().filter(|i| matches!(i, Item::Group(_)));
        (groups.count(), pixel(&svg, 10, 10, 5, 5))
    };
    let layered = pixel(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect width="10" height="10" fill="#08f"/>
            <g opacity="0.3"><path d="M1 1h8v8h-8z" fill="#f80c"/><rect x="20" width="1" height="1"/></g>
            </svg>"##,
        10,
        10,
        5,
        5,
    );
    let (groups, folded) = single(r##"fill="#f80c""##);
    assert_eq!(groups, 0);
    assert!(
        folded.iter().zip(layered).all(|(a, b)| a.abs_diff(b) <= 1),
        "{folded:?} {layered:?}"
    );
    let (groups, stroked) = single(r##"fill="none" stroke="#f80c" stroke-width="20""##);
    assert_eq!(groups, 0);
    assert_eq!(stroked, folded);
    // 填充和描边都有时二者相交处会叠两次色，渐变的颜色表由多个形状共用：仍要分层。
    assert_eq!(single(r#"fill="red" stroke="blue""#).0, 1);
    assert_eq!(single(r#"fill="url(#g)""#).0, 1);

    // 线性渐变从左红到右蓝，单位取外框比例。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="10">
            <linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient>
            <rect width="100" height="10" fill="url(#g)"/>
        </svg>"#;
    let left = pixel(svg, 100, 10, 0, 5);
    let right = pixel(svg, 100, 10, 99, 5);
    let middle = pixel(svg, 100, 10, 50, 5);
    assert!(left[0] > 240 && left[2] < 15, "{left:?}");
    assert!(right[2] > 240 && right[0] < 15, "{right:?}");
    assert!((110..=145).contains(&middle[0]), "{middle:?}");

    // 径向渐变：中心白，边上黑。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">
            <radialGradient id="g"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient>
            <rect width="20" height="20" fill="url(#g)"/>
        </svg>"##;
    assert!(pixel(svg, 20, 20, 10, 10)[0] > 230);
    assert!(pixel(svg, 20, 20, 0, 10)[0] < 30);

    // 没有字形时的文字、少见的滤镜无法绘制，需记录到 `missing`。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <filter id="f"><feTurbulence/></filter>
            <text>hi</text><rect width="1" height="1" filter="url(#f)"/>
        </svg>"#;
    assert_eq!(
        Drawing::parse(svg, None, None).unwrap().missing,
        vec!["文字", "部分滤镜效果"]
    );

    // 根元素不是 `<svg>`、XML 有语法错误：返回原因。
    assert!(
        Drawing::parse("<html/>", None, None)
            .unwrap_err()
            .contains("不是 SVG 文件")
    );
    assert!(
        Drawing::parse("<svg", None, None)
            .unwrap_err()
            .contains("XML")
    );
    // 没写大小时用 viewBox 的；都没有时取默认的 300 × 150。
    let picture = load(r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#, None, None).unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.format),
        (300, 150, "SVG")
    );
}

/// 测试用的字形提供者：每个字符是一个宽 0.5 em（粗体 0.6 em）、从基线向上高 0.7 em 的方块。
///
/// 使排版测试不依赖系统字体。
struct BoxGlyphs;

impl Glyphs for BoxGlyphs {
    fn glyph(&self, _: &[String], weight: u16, _: bool, c: char) -> Option<Glyph> {
        let p = |x: f32, y: f32| zeno::Point::new(x, y);
        let w = if weight >= 600 { 0.6 } else { 0.5 };
        let outline = if c == ' ' {
            Vec::new()
        } else {
            vec![
                Command::MoveTo(p(0.0, -0.7)),
                Command::LineTo(p(w, -0.7)),
                Command::LineTo(p(w, 0.0)),
                Command::LineTo(p(0.0, 0.0)),
                Command::Close,
            ]
        };
        Some(Glyph {
            outline: outline.into(),
            advance: w,
        })
    }

    fn metrics(&self, _: &[String], _: u16, _: bool) -> FontMetrics {
        FontMetrics {
            ascent: 0.8,
            descent: 0.2,
            x_height: 0.5,
        }
    }
}

fn alpha(svg: &str, w: u32, h: u32, x: u32, y: u32) -> u8 {
    pixel(svg, w, h, x, y)[3]
}

#[test]
fn path_data_arcs_and_sloppy_numbers() {
    // 圆弧：从 (0,5) 到 (10,5) 的半圆（上半）。
    let path = path_data("M0 5A5 5 0 0 1 10 5");
    let b = path_bounds(&path);
    assert!(
        (b[1] - 0.0).abs() < 0.05 && (b[2] - 10.0).abs() < 0.05,
        "{b:?}"
    );
    // 标志位紧挨着写、命令后接多组参数、相对坐标。
    let path = path_data("m1 1 2 0 0 2a1 1 0 011 1z");
    assert_eq!(path.len(), 5);
    // 路径数据出错时，出错位置之前的部分仍然保留。
    assert_eq!(path_data("M0 0L1 1L2").len(), 2);
}

#[test]
fn clip_paths_masks_and_nested_viewports() {
    // 裁剪成左半边。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <clipPath id="c"><rect width="5" height="10"/></clipPath>
            <rect width="10" height="10" fill="red" clip-path="url(#c)"/></svg>"#;
    assert_eq!(alpha(svg, 10, 10, 2, 5), 255);
    assert_eq!(alpha(svg, 10, 10, 7, 5), 0);
    // 单位取外框比例的裁剪。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <clipPath id="c" clipPathUnits="objectBoundingBox"><rect y="0.5" width="1" height="0.5"/></clipPath>
            <rect width="10" height="10" clip-path="url(#c)"/></svg>"#;
    assert_eq!(
        (alpha(svg, 10, 10, 5, 2), alpha(svg, 10, 10, 5, 7)),
        (0, 255)
    );
    // 亮度蒙版：白色处保留、黑色处去掉；透明度蒙版按透明度决定。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <mask id="m"><rect width="10" height="10" fill="#fff"/><rect width="5" height="10" fill="#000"/></mask>
            <rect width="10" height="10" fill="blue" mask="url(#m)"/></svg>"##;
    assert_eq!(
        (alpha(svg, 10, 10, 2, 5), alpha(svg, 10, 10, 7, 5)),
        (0, 255)
    );
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <mask id="m" mask-type="alpha"><rect width="10" height="10" fill="#000"/></mask>
            <rect width="10" height="10" fill="blue" mask="url(#m)"/></svg>"##;
    assert_eq!(alpha(svg, 10, 10, 5, 5), 255);
    // 嵌套的 `<svg>` 在视口外的部分裁掉；`overflow: visible` 时不裁。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <svg width="5" height="5"><rect width="10" height="10"/></svg></svg>"#;
    assert_eq!(
        (alpha(svg, 10, 10, 2, 2), alpha(svg, 10, 10, 7, 7)),
        (255, 0)
    );
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <svg width="5" height="5" style="overflow:visible"><rect width="10" height="10"/></svg></svg>"#;
    assert_eq!(alpha(svg, 10, 10, 7, 7), 255);
    // `<switch>` 只绘制第一个满足条件的子元素；`display:none` 的元素不绘制。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <switch><rect width="10" height="10" requiredExtensions="x"/><rect width="5" height="10"/></switch>
            <rect x="5" width="5" height="10" style="display:none"/></svg>"#;
    assert_eq!(
        (alpha(svg, 10, 10, 2, 5), alpha(svg, 10, 10, 7, 5)),
        (255, 0)
    );
}

#[test]
fn filters_blend_modes_and_patterns() {
    // 模糊：边缘变淡，中间仍是完全不透明。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
            <filter id="b"><feGaussianBlur stdDeviation="3"/></filter>
            <rect x="10" y="10" width="20" height="20" filter="url(#b)"/></svg>"#;
    let edge = alpha(svg, 40, 40, 10, 20);
    assert!((90..170).contains(&edge), "{edge}");
    assert!(alpha(svg, 40, 40, 20, 20) > 250);
    // 形状外面、滤镜区域（外框向四周各扩大 10%）之内也有模糊的扩散；区域之外没有。
    assert!(alpha(svg, 40, 40, 8, 20) > 0);
    assert_eq!(alpha(svg, 40, 40, 5, 20), 0);
    // 投影：形状向右下偏移出一块半透明的黑色。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
            <filter id="s" x="-1" y="-1" width="3" height="3"><feDropShadow dx="10" dy="10" stdDeviation="0.1" flood-opacity="0.5"/></filter>
            <rect x="5" y="5" width="10" height="10" fill="white" filter="url(#s)"/></svg>"#;
    let shadow = pixel(svg, 40, 40, 22, 22);
    assert!(
        shadow[0] < 10 && (110..145).contains(&shadow[3]),
        "{shadow:?}"
    );
    assert_eq!(pixel(svg, 40, 40, 10, 10), [255, 255, 255, 255]);
    // 颜色矩阵（在 sRGB 中计算）：红变成绿。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <filter id="m" color-interpolation-filters="sRGB"><feColorMatrix values="0 0 0 0 0  1 0 0 0 0  0 0 0 0 0  0 0 0 1 0"/></filter>
            <rect width="4" height="4" fill="red" filter="url(#m)"/></svg>"#;
    assert_eq!(pixel(svg, 4, 4, 2, 2), [0, 255, 0, 255]);
    // `<feMergeNode in="BackgroundImage"/>` 与原语的 `in` 一样按全透明处理，
    // 不会取到上一步的结果。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <filter id="m"><feFlood flood-color="red"/><feMerge><feMergeNode in="BackgroundImage"/></feMerge></filter>
            <rect width="4" height="4" filter="url(#m)"/></svg>"#;
    assert_eq!(pixel(svg, 4, 4, 2, 2)[3], 0);
    // CSS 的滤镜函数。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <rect width="4" height="4" fill="red" style="filter: grayscale(100%)"/></svg>"#;
    let [r, g, b, _] = pixel(svg, 4, 4, 2, 2);
    assert!(r == g && g == b, "{r} {g} {b}");
    // 混合：`difference` 模式下，白色减白色得到黑色。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <rect width="4" height="4" fill="white"/><rect width="4" height="4" fill="white" style="mix-blend-mode:difference"/></svg>"#;
    assert_eq!(pixel(svg, 4, 4, 2, 2), [0, 0, 0, 255]);
    // 图案：2×2 的图块左上角有一个 1×1 的点，平铺整个形状。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <pattern id="p" width="2" height="2" patternUnits="userSpaceOnUse"><rect width="1" height="1"/></pattern>
            <rect width="8" height="8" fill="url(#p)"/></svg>"#;
    assert_eq!(
        (
            alpha(svg, 8, 8, 4, 4),
            alpha(svg, 8, 8, 5, 4),
            alpha(svg, 8, 8, 6, 6)
        ),
        (255, 0, 255)
    );
}

#[test]
fn images_inline_and_beside_the_file() {
    // 1×1 的红色 PNG，以 base64 内嵌。
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
            <image width="10" height="10" xlink:href="data:image/png;base64,{png}" style="image-rendering:pixelated"/></svg>"#
    );
    assert_eq!(pixel(&svg, 10, 10, 5, 5), [255, 0, 0, 255]);
    // 同目录的文件：相对路径从 SVG 所在的目录查找；找不到的记入 `missing`。
    let dir = std::env::temp_dir().join(format!("winc-preview-svg-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("dot.png"),
        crate::svg::syntax::base64(png).unwrap(),
    )
    .unwrap();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="dot.png" width="10" height="10"/><image href="nope.png"/></svg>"#;
    let drawing = Drawing::parse(svg, Some(&dir), None).unwrap();
    assert_eq!(drawing.render(10, 10).rgba[..4], [255, 0, 0, 255]);
    assert_eq!(drawing.missing, ["无法找到的图片"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_lays_out_with_anchors_spans_and_positions() {
    let glyphs: Option<Arc<dyn Glyphs>> = Some(Arc::new(BoxGlyphs));
    let bounds = |svg: &str| {
        let d = Drawing::parse(svg, None, glyphs.clone()).unwrap();
        assert!(d.missing.is_empty(), "{:?}", d.missing);
        let inverse = Affine::IDENTITY;
        bbox_of(&d.items, inverse).unwrap()
    };
    // 两个字符，中间的换行和缩进合并成一个空格，字号 10：每个字符宽 5，三者共宽 15，
    // 基线在 y=20，高 7。
    let b = bounds(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="10" y="20" font-size="10">a
              b</text></svg>"#,
    );
    assert_eq!(b, [10.0, 13.0, 15.0, 7.0]);
    // 居中：整段文字的中点在 x 处。
    let b = bounds(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="50" y="20" font-size="10" text-anchor="middle">ab</text></svg>"#,
    );
    assert_eq!(b[0] + b[2] / 2.0, 50.0);
    // `<tspan>` 换成粗体（更宽），`dx` 向右偏移。
    let b = bounds(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text y="10" font-size="10">a<tspan font-weight="bold" dx="2">b</tspan></text></svg>"#,
    );
    assert_eq!(b[2], 5.0 + 2.0 + 6.0);
    // 每个字符各有一个 x 坐标。
    let b = bounds(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0 30" y="10" font-size="10">ab</text></svg>"#,
    );
    assert_eq!(b[2], 35.0);
    // `font` 简写属性的解析。
    let mut font = text::Font::default();
    let parent = text::Font::default();
    text::apply_font(
        &mut font,
        &parent,
        "font",
        "italic bold 2em/1.5 'Helvetica Neue', sans-serif",
        0.0,
    );
    let debug = format!("{font:?}");
    for part in [
        "\"Helvetica Neue\", \"sans-serif\"",
        "size: 32.0",
        "weight: 700",
        "italic: true",
    ] {
        assert!(debug.contains(part), "{debug}");
    }
}

/// 测量 2560×1600 的画布上 5000 个纯色路径的绘制耗时（毫秒），各自写了 `opacity` 与
/// 不透明时各测一遍。
///
/// 运行 `cargo test --release -p winc-preview --lib svg_translucent -- --ignored --nocapture`
/// 打印耗时。
#[test]
#[ignore = "测量耗时，手动运行"]
fn svg_translucent_paths_timing() {
    for opacity in ["0.8", "1"] {
        let mut svg =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="2560" height="1600">"#);
        for i in 0..5000 {
            let (x, y) = ((i * 37) % 2500, (i * 53) % 1550);
            svg.push_str(&format!(
                r#"<path d="M{x} {y}h30v30h-30z" fill="red" opacity="{opacity}"/>"#
            ));
        }
        svg.push_str("</svg>");
        let drawing = Drawing::parse(&svg, None, None).unwrap();
        let start = std::time::Instant::now();
        let pixels = drawing.render(2560, 1600);
        eprintln!("opacity {opacity}: {:?}", start.elapsed());
        assert_eq!(pixels.rgba.len(), 2560 * 1600 * 4);
    }
}

#[test]
fn embedded_images_share_one_memory_budget() {
    // 画布 2560 × 2560 的 GIF：解码后约 26 MB。各张的帧位置不同，内容不同、不共用缓存；前 15 张
    // 合计不超过上限，之后的不再解码。
    let gif = |left: u16| {
        let mut bytes = Vec::new();
        let palette = [0, 0, 0, 255, 255, 255];
        let mut encoder = gif::Encoder::new(&mut bytes, 2560, 2560, &palette).unwrap();
        let mut frame = gif::Frame::default();
        (frame.left, frame.width, frame.height) = (left, 1, 1);
        frame.buffer = std::borrow::Cow::Owned(vec![1]);
        encoder.write_frame(&frame).unwrap();
        drop(encoder);
        bytes
    };
    let images: String = (0..20)
        .map(|i| {
            let data: String = gif(i).iter().map(|b| format!("%{b:02X}")).collect();
            format!(r#"<image href="data:image/gif,{data}" width="10" height="10"/>"#)
        })
        .collect();
    let svg =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{images}</svg>"#);
    let drawing = Drawing::parse(&svg, None, None).unwrap();
    assert!(
        drawing.missing.iter().any(|m| m.contains("内嵌图片")),
        "{:?}",
        drawing.missing
    );
}

#[test]
fn css_filter_functions_count_toward_the_filter_budget() {
    // 一个元素上写数千个滤镜函数：与 `<filter>` 的原语一样计入上限，超出时整个滤镜不生效。
    let functions = "blur(1px) ".repeat(1000);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
        <rect width="10" height="10" style="filter: {functions}"/></svg>"#
    );
    let drawing = Drawing::parse(&svg, None, None).unwrap();
    assert!(
        drawing.missing.iter().any(|m| m.contains("过多")),
        "{:?}",
        drawing.missing
    );
}

#[test]
fn too_many_dashes_stroke_solid() {
    // 300 条重合的子路径，每条长 8000 像素，按 1 像素一段画虚线约 120 万段，超过上限，按实线
    // 描边：本该是空隙的第 1 列也画上了。逐段生成轮廓时这一张图要画数十秒。
    let lines = "M0 5 H8000 ".repeat(300);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="10">
        <path d="{lines}" stroke="black" fill="none" stroke-width="2"
            stroke-dasharray="1"/></svg>"#
    );
    assert_eq!(pixel(&svg, 100, 10, 1, 5)[3], 255);
    // 段数没有超过上限：正常绘制虚线，空隙处透明。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="10">
        <path d="M0 5 H100" stroke="black" stroke-width="4" stroke-dasharray="10"/></svg>"#;
    assert_eq!(pixel(svg, 100, 10, 15, 5)[3], 0);
}

/// 按区域绘制的结果与绘制整张图后裁剪出的同一部分相同（形状、渐变、裁剪和图案；滤镜除外，
/// 见 [`Drawing::render_region`]）。
#[test]
fn a_rendered_region_equals_the_same_part_of_the_whole() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 30">
<defs>
<linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
<pattern id="p" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="2" height="2" fill="#0a0"/></pattern>
<clipPath id="c"><circle cx="20" cy="15" r="12"/></clipPath>
</defs>
<rect width="40" height="30" fill="url(#g)"/>
<g clip-path="url(#c)" opacity="0.8"><rect width="40" height="30" fill="url(#p)"/></g>
<path d="M2 28L38 2" stroke="#fc0" stroke-width="1.5"/>
</svg>"##;
    let drawing = Drawing::parse(svg, None, None).unwrap();
    let size = (160, 120);
    let whole = drawing.render(size.0, size.1);
    for region in [[0, 0, 160, 120], [37, 21, 64, 48], [150, 110, 10, 10]] {
        let [x, y, w, h] = region;
        let part = drawing.render_region(size, region);
        assert_eq!((part.width, part.height), (w, h));
        for row in 0..h {
            let at = (((y + row) * size.0 + x) * 4) as usize;
            let line = (row * w * 4) as usize;
            let len = (w * 4) as usize;
            assert_eq!(
                part.rgba[line..line + len],
                whole.rgba[at..at + len],
                "区域 {region:?} 的第 {row} 行"
            );
        }
    }
}
