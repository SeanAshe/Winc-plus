//! 解析 `<filter>` 元素和 CSS 滤镜函数（`blur()`、`drop-shadow()`、颜色矩阵等）。

use super::*;

/// 返回属性字符串解析出的有限数；`NaN`、`inf` 和无法解析的值返回 `None`。
///
/// `str::parse` 会接受 `NaN`、`inf`，而滤镜计算中一旦出现 NaN 就会扩散到整个结果，
/// 因此拒绝非有限值。
fn finite(value: &str) -> Option<f32> {
    value.trim().parse().ok().filter(|v: &f32| v.is_finite())
}

/// 解析原语的 `in`、`in2` 属性和 `<feMergeNode>` 的 `in` 属性。
fn parse_input(value: Option<&str>) -> Input {
    match value.map(str::trim) {
        None | Some("") => Input::Previous,
        Some("SourceGraphic") => Input::Source,
        Some("SourceAlpha") => Input::SourceAlpha,
        Some("BackgroundImage" | "BackgroundAlpha" | "FillPaint" | "StrokePaint") => Input::Empty,
        Some(name) => Input::Named(name.to_owned()),
    }
}

/// 解析 `<filter>` 元素 `node`，返回滤镜或隐藏、不支持的标记。
///
/// `ctm` 是元素用户坐标到 SVG 根的变换，`bbox` 是元素的外框（x、y、宽、高，用户坐标）。
pub(in crate::svg) fn parse<'a, 'input>(
    parser: &mut Parser<'a, 'input>,
    node: Node<'a, 'input>,
    ctm: Affine,
    bbox: Option<[f32; 4]>,
) -> Parsed {
    let bbox = bbox.filter(|b| b[2] > 0.0 && b[3] > 0.0);
    let (vw, vh) = parser.viewport;
    // 滤镜区域：默认取外框并向四周各扩 10%（x、y 为 -10%，宽、高为 120%）。
    let by_box = node.attribute("filterUnits") != Some("userSpaceOnUse");
    let region = if by_box {
        let Some(b) = bbox else {
            return Parsed::Hide;
        };
        let f =
            |name: &str, default: f32| node.attribute(name).and_then(fraction).unwrap_or(default);
        let (x, y, w, h) = (
            f("x", -0.1),
            f("y", -0.1),
            f("width", 1.2),
            f("height", 1.2),
        );
        [b[0] + x * b[2], b[1] + y * b[3], w * b[2], h * b[3]]
    } else {
        let l = |name: &str, default: f32, reference: f32| {
            node.attribute(name)
                .and_then(|v| length(v, reference))
                .unwrap_or(default * reference)
        };
        [
            l("x", -0.1, vw),
            l("y", -0.1, vh),
            l("width", 1.2, vw),
            l("height", 1.2, vh),
        ]
    };
    if region[2] <= 0.0 || region[3] <= 0.0 {
        return Parsed::Hide;
    }

    // 滤镜中的长度：`primitiveUnits` 为 `objectBoundingBox` 时按外框的宽、高换算。
    let primitive_box = node.attribute("primitiveUnits") == Some("objectBoundingBox");
    if primitive_box && bbox.is_none() {
        return Parsed::Hide;
    }
    let b = bbox.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let scale_x = if primitive_box { b[2] } else { 1.0 };
    let scale_y = if primitive_box { b[3] } else { 1.0 };

    let filter_props = parser.inherited(node);
    let mut primitives = Vec::new();
    for child in node.children().filter(Node::is_element) {
        // 每个原语都要按滤镜区域的大小算一遍：计入上限，超出时整个滤镜不生效。
        if !parser.filter_step() {
            return Parsed::Unsupported(winc_i18n::t!("preview-svg-missing-too-many-effects"));
        }
        let declared = parser.declared(child);
        let props = parser.props_from(&declared, &filter_props);
        let get = |name: &str| {
            declared
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|&(_, v)| v)
        };
        let nums = |name: &str| child.attribute(name).map(numbers).unwrap_or_default();
        let input = |name: &str| parse_input(child.attribute(name));
        let pair = |name: &str, default: f32| {
            let v = nums(name);
            match v.as_slice() {
                [a] => (*a, *a),
                [a, b, ..] => (*a, *b),
                [] => (default, default),
            }
        };
        let flood = |color_name: &str, opacity_name: &str| {
            let c = match get(color_name).map(str::trim) {
                Some("currentColor") => props.color,
                Some(v) => color(v).unwrap_or([0, 0, 0, 255]),
                None => [0, 0, 0, 255],
            };
            let o = get(opacity_name).map_or(1.0, opacity);
            [
                f32::from(c[0]) / 255.0,
                f32::from(c[1]) / 255.0,
                f32::from(c[2]) / 255.0,
                f32::from(c[3]) / 255.0 * o,
            ]
        };
        let kind = match child.tag_name().name() {
            "feGaussianBlur" => {
                let (x, y) = pair("stdDeviation", 0.0);
                Kind::Blur(x * scale_x, y * scale_y)
            }
            "feOffset" => Kind::Offset(
                child.attribute("dx").and_then(finite).unwrap_or(0.0) * scale_x,
                child.attribute("dy").and_then(finite).unwrap_or(0.0) * scale_y,
            ),
            "feFlood" => Kind::Flood(flood("flood-color", "flood-opacity")),
            "feMerge" => Kind::Merge(
                child
                    .children()
                    .filter(|n| n.has_tag_name("feMergeNode"))
                    .map(|n| parse_input(n.attribute("in")))
                    .collect(),
            ),
            "feBlend" => Kind::Blend(blend_mode(child.attribute("mode").unwrap_or("normal"))),
            "feComposite" => Kind::Composite(match child.attribute("operator").unwrap_or("over") {
                "in" => Composite::In,
                "out" => Composite::Out,
                "atop" => Composite::Atop,
                "xor" => Composite::Xor,
                "lighter" => Composite::Lighter,
                "arithmetic" => {
                    let k = |name: &str| child.attribute(name).and_then(finite).unwrap_or(0.0);
                    Composite::Arithmetic([k("k1"), k("k2"), k("k3"), k("k4")])
                }
                _ => Composite::Over,
            }),
            "feColorMatrix" => {
                let values = nums("values");
                let matrix = match child.attribute("type").unwrap_or("matrix") {
                    "saturate" => saturate(values.first().copied().unwrap_or(1.0)),
                    "hueRotate" => hue_rotate(values.first().copied().unwrap_or(0.0)),
                    "luminanceToAlpha" => {
                        // 亮度系数取自 SVG 1.1 第 15.10 节的 `luminanceToAlpha`。
                        let mut m = [0.0; 20];
                        m[15] = 0.2125;
                        m[16] = 0.7154;
                        m[17] = 0.0721;
                        m
                    }
                    _ if values.len() == 20 => std::array::from_fn(|i| values[i]),
                    _ => identity_matrix(),
                };
                Kind::ColorMatrix(Box::new(matrix))
            }
            "feComponentTransfer" => {
                let mut funcs = [
                    Transfer::Identity,
                    Transfer::Identity,
                    Transfer::Identity,
                    Transfer::Identity,
                ];
                for f in child.children().filter(Node::is_element) {
                    let i = match f.tag_name().name() {
                        "feFuncR" => 0,
                        "feFuncG" => 1,
                        "feFuncB" => 2,
                        "feFuncA" => 3,
                        _ => continue,
                    };
                    let num = |name: &str, default: f32| {
                        f.attribute(name).and_then(finite).unwrap_or(default)
                    };
                    let table = f.attribute("tableValues").map(numbers).unwrap_or_default();
                    funcs[i] = match f.attribute("type").unwrap_or("identity") {
                        "table" if !table.is_empty() => Transfer::Table(table),
                        "discrete" if !table.is_empty() => Transfer::Discrete(table),
                        "linear" => Transfer::Linear(num("slope", 1.0), num("intercept", 0.0)),
                        "gamma" => Transfer::Gamma(
                            num("amplitude", 1.0),
                            num("exponent", 1.0),
                            num("offset", 0.0),
                        ),
                        _ => Transfer::Identity,
                    };
                }
                Kind::Transfer(Box::new(funcs))
            }
            "feDropShadow" => {
                let (bx, by) = pair("stdDeviation", 2.0);
                let d = |name: &str| child.attribute(name).and_then(finite).unwrap_or(2.0);
                Kind::DropShadow {
                    blur: (bx * scale_x, by * scale_y),
                    offset: (d("dx") * scale_x, d("dy") * scale_y),
                    color: flood("flood-color", "flood-opacity"),
                }
            }
            "feMorphology" => {
                let (rx, ry) = pair("radius", 0.0);
                Kind::Morphology {
                    dilate: child.attribute("operator") == Some("dilate"),
                    radius: (rx * scale_x, ry * scale_y),
                }
            }
            "feTurbulence" | "feDisplacementMap" | "feConvolveMatrix" | "feDiffuseLighting"
            | "feSpecularLighting" | "feImage" | "feTile" => {
                return Parsed::Unsupported(winc_i18n::t!("preview-svg-missing-filter-effects"));
            }
            _ => continue,
        };
        // 子区域（x、y、宽、高）：未指定的项取滤镜区域的值。
        let sub = |name: &str, reference: f32| -> Option<f32> {
            let v = child.attribute(name)?;
            if primitive_box {
                fraction(v).map(|f| f * reference)
            } else {
                length(v, reference)
            }
        };
        let subregion = if ["x", "y", "width", "height"]
            .iter()
            .any(|n| child.attribute(*n).is_some())
        {
            let x = sub("x", b[2]).map_or(region[0], |v| if primitive_box { b[0] + v } else { v });
            let y = sub("y", b[3]).map_or(region[1], |v| if primitive_box { b[1] + v } else { v });
            let w = sub("width", if primitive_box { b[2] } else { vw }).unwrap_or(region[2]);
            let h = sub("height", if primitive_box { b[3] } else { vh }).unwrap_or(region[3]);
            Some([x, y, w, h])
        } else {
            None
        };
        primitives.push(Primitive {
            kind,
            inputs: [input("in"), input("in2")],
            result: child.attribute("result").map(str::to_owned),
            linear: props.filters_linear,
            subregion,
        });
    }
    // 没有任何滤镜效果：元素整个不画。
    if primitives.is_empty() {
        return Parsed::Hide;
    }
    Parsed::Filter(Filter {
        region,
        ctm,
        primitives,
    })
}

/// 解析 CSS 滤镜函数 `text`，返回滤镜或隐藏、不支持的标记。
///
/// 支持 `grayscale()`、`sepia()`、`saturate()`、`hue-rotate()`、`invert()`、`brightness()`、
/// `contrast()`、`opacity()`、`blur()` 和 `drop-shadow()`，在 sRGB 色彩空间中计算（CSS Filter
/// Effects Module Level 1 第 5 节），其余（含 `url(` 引用）按不支持处理。`bbox` 为空时元素
/// 不绘制；每个函数与 `<filter>` 的原语一样计入工作量上限，超出时整个滤镜不生效。
pub(in crate::svg) fn functions(
    parser: &mut Parser,
    text: &str,
    ctm: Affine,
    bbox: Option<[f32; 4]>,
) -> Parsed {
    let Some(b) = bbox.filter(|b| b[2] > 0.0 && b[3] > 0.0) else {
        return Parsed::Hide;
    };
    let mut primitives = Vec::new();
    let mut margin = 0.0f32;
    let mut rest = text.trim();
    while let Some(open) = rest.find('(') {
        if !parser.filter_step() {
            return Parsed::Unsupported(winc_i18n::t!("preview-svg-missing-too-many-effects"));
        }
        let name = rest[..open].trim().to_ascii_lowercase();
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let args = rest[open + 1..open + close].trim().to_owned();
        rest = rest[open + close + 1..].trim_start();
        // 数值可以写成百分比；没写时取默认值。
        let amount = |default: f32| {
            let a = args.trim();
            if a.is_empty() {
                default
            } else {
                fraction(a).unwrap_or(default)
            }
        };
        let matrix = |m: [f32; 20]| Kind::ColorMatrix(Box::new(m));
        let linear = |slope: f32, intercept: f32| {
            Kind::Transfer(Box::new([
                Transfer::Linear(slope, intercept),
                Transfer::Linear(slope, intercept),
                Transfer::Linear(slope, intercept),
                Transfer::Identity,
            ]))
        };
        let kind = match name.as_str() {
            "grayscale" => matrix(saturate(1.0 - amount(1.0).clamp(0.0, 1.0))),
            "saturate" => matrix(saturate(amount(1.0))),
            "sepia" => {
                // 系数取自 CSS Filter Effects Module Level 1 对 `sepia()` 的定义。
                let a = 1.0 - amount(1.0).clamp(0.0, 1.0);
                let mut m = identity_matrix();
                let rows = [
                    [0.393 + 0.607 * a, 0.769 - 0.769 * a, 0.189 - 0.189 * a],
                    [0.349 - 0.349 * a, 0.686 + 0.314 * a, 0.168 - 0.168 * a],
                    [0.272 - 0.272 * a, 0.534 - 0.534 * a, 0.131 + 0.869 * a],
                ];
                for (r, row) in rows.iter().enumerate() {
                    m[r * 5..r * 5 + 3].copy_from_slice(row);
                }
                matrix(m)
            }
            "hue-rotate" => {
                let a = args.trim();
                let degrees = if let Some(t) = a.strip_suffix("turn") {
                    t.trim().parse::<f32>().unwrap_or(0.0) * 360.0
                } else if let Some(r) = a.strip_suffix("rad") {
                    r.trim().parse::<f32>().unwrap_or(0.0).to_degrees()
                } else {
                    a.trim_end_matches("deg").trim().parse().unwrap_or(0.0)
                };
                matrix(hue_rotate(degrees))
            }
            "invert" => {
                let a = amount(1.0).clamp(0.0, 1.0);
                Kind::Transfer(Box::new([
                    Transfer::Table(vec![a, 1.0 - a]),
                    Transfer::Table(vec![a, 1.0 - a]),
                    Transfer::Table(vec![a, 1.0 - a]),
                    Transfer::Identity,
                ]))
            }
            "brightness" => linear(amount(1.0), 0.0),
            "contrast" => {
                let a = amount(1.0);
                linear(a, 0.5 - 0.5 * a)
            }
            "opacity" => Kind::Transfer(Box::new([
                Transfer::Identity,
                Transfer::Identity,
                Transfer::Identity,
                Transfer::Table(vec![0.0, amount(1.0).clamp(0.0, 1.0)]),
            ])),
            "blur" => {
                let s = length(&args, 0.0).unwrap_or(0.0);
                margin = margin.max(s * 3.0);
                Kind::Blur(s, s)
            }
            "drop-shadow" => {
                // 长度依次为 dx、dy 和可选的模糊半径；颜色可以在长度之前或之后。
                let mut lengths = Vec::new();
                let mut shadow = [0.0, 0.0, 0.0, 1.0];
                let mut words = args.as_str();
                while !words.is_empty() {
                    let end = if words.starts_with(|c: char| c.is_ascii_alphabetic() || c == '#') {
                        // 颜色函数（如 `rgb(...)`）内部含空格：取到右括号为止。
                        words
                            .find(')')
                            .map_or_else(|| words.find(' ').unwrap_or(words.len()), |i| i + 1)
                    } else {
                        words.find(' ').unwrap_or(words.len())
                    };
                    let word = &words[..end];
                    if let Some(l) = length(word, 0.0) {
                        lengths.push(l);
                    } else if let Some(c) = color(word) {
                        shadow = c.map(|v| f32::from(v) / 255.0);
                    }
                    words = words[end..].trim_start();
                }
                let (dx, dy) = (
                    lengths.first().copied().unwrap_or(0.0),
                    lengths.get(1).copied().unwrap_or(0.0),
                );
                let blur = lengths.get(2).copied().unwrap_or(0.0) / 2.0;
                margin = margin.max(blur * 3.0 + dx.abs().max(dy.abs()));
                Kind::DropShadow {
                    blur: (blur, blur),
                    offset: (dx, dy),
                    color: shadow,
                }
            }
            "url" => {
                return Parsed::Unsupported(winc_i18n::t!("preview-svg-missing-filter-effects"));
            }
            _ => continue,
        };
        primitives.push(Primitive {
            kind,
            inputs: [Input::Previous, Input::Previous],
            result: None,
            linear: false,
            subregion: None,
        });
    }
    if primitives.is_empty() {
        return Parsed::Unsupported(winc_i18n::t!("preview-svg-missing-filter-effects"));
    }
    // 区域：CSS 滤镜作用在元素绘制出的整个范围上（描边会伸出外框）。这里取外框四周各扩出
    // 一个外框的大小，模糊和阴影再按 `margin` 继续外扩。
    let (mx, my) = (b[2] + margin, b[3] + margin);
    let region = [b[0] - mx, b[1] - my, b[2] + 2.0 * mx, b[3] + 2.0 * my];
    Parsed::Filter(Filter {
        region,
        ctm,
        primitives,
    })
}

fn identity_matrix() -> [f32; 20] {
    let mut m = [0.0; 20];
    for i in 0..4 {
        m[i * 5 + i] = 1.0;
    }
    m
}

/// 返回饱和度为 `s` 的颜色矩阵。
/// `s` 为 1 时结果不变，为 0 时得到灰度；系数取自 SVG 规范对 `feColorMatrix` 的定义。
fn saturate(s: f32) -> [f32; 20] {
    let mut m = identity_matrix();
    let rows = [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ];
    for (r, row) in rows.iter().enumerate() {
        m[r * 5..r * 5 + 3].copy_from_slice(row);
    }
    m
}

/// 返回色相旋转 `degrees`（度）的颜色矩阵；系数取自 SVG 规范对 `feColorMatrix` 的定义。
fn hue_rotate(degrees: f32) -> [f32; 20] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut m = identity_matrix();
    let rows = [
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
        ],
    ];
    for (r, row) in rows.iter().enumerate() {
        m[r * 5..r * 5 + 3].copy_from_slice(row);
    }
    m
}
