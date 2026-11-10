//! 公式排版用到的细节：类别间距、数学字母的替换、高括号和根号的画法、`\color` 的颜色名。
//!
//! 间距和几何量的单位与 `math` 模块相同（逻辑像素，间距按字号的 1/18 计）。

use winc_preview::markdown::math::{Class, Font, Node};
use winc_render::{Color, Stroke};

/// 返回类别 `left` 后面接 `right` 时中间留多少（单位是字号的 1/18）。
///
/// 上下标里只留窄的；返回值 3、4、5 分别对应 TeX 的窄、中、宽三档，负值的类别回来取 0。
pub(crate) fn spacing(left: Class, right: Class, script: bool) -> f32 {
    use Class::{Bin, Close, Op, Open, Ord, Punct, Rel};
    // TeX 的表：1 窄、2 中、3 宽；负的只在不是上下标时才留。
    let space: i8 = match (left, right) {
        (Ord, Op) | (Op, Ord) | (Op, Op) | (Close, Op) => 1,
        (Ord | Close, Bin) | (Bin, Ord | Op | Open) => -2,
        (Ord | Op | Close, Rel) | (Rel, Ord | Op | Open) => -3,
        (Punct, _) => -1,
        _ => 0,
    };
    match space {
        s if s < 0 && script => 0.0,
        1 | -1 => 3.0,
        -2 => 4.0,
        -3 => 5.0,
        _ => 0.0,
    }
}

/// 单个字母（或希腊字母）的符号节点，用斜体：上标、重音往右挪一点。
pub(crate) fn italic_letter(node: &Node) -> bool {
    match node {
        Node::Sym(text, _, Font::Italic | Font::BoldItalic) => text.chars().count() == 1,
        Node::Row(nodes) if nodes.len() == 1 => italic_letter(&nodes[0]),
        _ => false,
    }
}

/// 是否只有一个字（重音按一个字的宽度画，不铺满）。
pub(crate) fn single_glyph(node: &Node) -> bool {
    match node {
        Node::Sym(text, ..) => text.chars().count() == 1,
        Node::Row(nodes) if nodes.len() == 1 => single_glyph(&nodes[0]),
        _ => false,
    }
}

/// 字体 `font` 下的字 `c`：字母换成 Unicode 数学字母区里对应的字（数学字体里有）。
pub(crate) fn letter(c: char, font: Font) -> char {
    let at = |base: u32, offset: u32| char::from_u32(base + offset).unwrap_or(c);
    let upper = c.is_ascii_uppercase().then(|| c as u32 - 'A' as u32);
    let lower = c.is_ascii_lowercase().then(|| c as u32 - 'a' as u32);
    let digit = c.is_ascii_digit().then(|| c as u32 - '0' as u32);
    match font {
        Font::Italic => match (upper, lower) {
            (Some(i), _) => at(0x1D434, i),
            // 数学斜体区里 h 的位置空着，用 U+210E ℎ 代替。
            (_, Some(_)) if c == 'h' => 'ℎ',
            (_, Some(i)) => at(0x1D44E, i),
            _ => match c {
                'α'..='ω' => at(0x1D6FC, c as u32 - 'α' as u32),
                'ϵ' => '𝜖',
                'ϑ' => '𝜗',
                'ϰ' => '𝜘',
                'ϕ' => '𝜙',
                'ϱ' => '𝜚',
                'ϖ' => '𝜛',
                _ => c,
            },
        },
        Font::Bold => match (upper, lower, digit) {
            (Some(i), ..) => at(0x1D400, i),
            (_, Some(i), _) => at(0x1D41A, i),
            (.., Some(i)) => at(0x1D7CE, i),
            _ => c,
        },
        Font::BoldItalic => match (upper, lower, digit) {
            (Some(i), ..) => at(0x1D468, i),
            (_, Some(i), _) => at(0x1D482, i),
            (.., Some(i)) => at(0x1D7CE, i),
            _ => c,
        },
        Font::Mono => match (upper, lower, digit) {
            (Some(i), ..) => at(0x1D670, i),
            (_, Some(i), _) => at(0x1D68A, i),
            (.., Some(i)) => at(0x1D7F6, i),
            _ => c,
        },
        Font::Roman | Font::Text => c,
    }
}

/// 依次连起 `points` 的线段（逻辑像素）。
pub(crate) fn polyline(points: &[(f32, f32)]) -> Vec<Stroke> {
    points
        .windows(2)
        .map(|w| Stroke::Line(w[0].0, w[0].1, w[1].0, w[1].1))
        .collect()
}

/// 二次贝塞尔曲线 `a`→`b`（控制点 `c`）上的 `n + 1` 个点。
fn bezier(a: (f32, f32), c: (f32, f32), b: (f32, f32), n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            let u = 1.0 - t;
            (
                u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
                u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
            )
        })
        .collect()
}

/// 画出高 `top`..`bottom` 的括号 `delim`：返回宽、线段和线宽（逻辑像素）。
///
/// 没有画法的括号返回 `None`，改用放大的字。
pub(crate) fn drawn_delimiter(
    delim: &str,
    top: f32,
    bottom: f32,
    s: f32,
    t: f32,
) -> Option<(f32, Vec<Stroke>, f32)> {
    let mid = (top + bottom) / 2.0;
    let h = bottom - top;
    let w = (s * 0.3 + h * 0.04).min(s * 0.6);
    let flip = |points: Vec<(f32, f32)>| -> Vec<(f32, f32)> {
        points.into_iter().map(|(x, y)| (w - x, y)).collect()
    };
    let strokes = match delim {
        "(" | ")" => {
            let points = bezier((w * 0.85, top), (w * 0.0, mid), (w * 0.85, bottom), 24);
            polyline(&if delim == "(" { points } else { flip(points) })
        }
        "[" | "]" | "⌊" | "⌋" | "⌈" | "⌉" => {
            let (x0, x1) = (w * 0.3, w * 0.8);
            let up = !matches!(delim, "⌊" | "⌋");
            let down = !matches!(delim, "⌈" | "⌉");
            let mut points = Vec::new();
            if up {
                points.push((x1, top));
            }
            points.push((x0, top));
            points.push((x0, bottom));
            if down {
                points.push((x1, bottom));
            }
            let points = if matches!(delim, "]" | "⌋" | "⌉") {
                flip(points)
            } else {
                points
            };
            polyline(&points)
        }
        "{" | "}" => {
            let (x0, x1, x2) = (w * 0.1, w * 0.45, w * 0.85);
            let r = (h * 0.08).min(s * 0.3);
            let mut points = bezier((x2, top), (x1, top), (x1, top + r), 6);
            points.extend(bezier((x1, mid - r), (x1, mid), (x0, mid), 6));
            points.extend(bezier((x0, mid), (x1, mid), (x1, mid + r), 6));
            points.extend(bezier((x1, bottom - r), (x1, bottom), (x2, bottom), 6));
            polyline(&if delim == "{" { points } else { flip(points) })
        }
        "|" | "∣" => vec![Stroke::Line(w / 2.0, top, w / 2.0, bottom)],
        "‖" => vec![
            Stroke::Line(w * 0.35, top, w * 0.35, bottom),
            Stroke::Line(w * 0.65, top, w * 0.65, bottom),
        ],
        "⟨" | "⟩" => {
            let points = vec![(w * 0.8, top), (w * 0.2, mid), (w * 0.8, bottom)];
            polyline(&if delim == "⟨" { points } else { flip(points) })
        }
        "/" => vec![Stroke::Line(w, top, 0.0, bottom)],
        "\\" => vec![Stroke::Line(0.0, top, w, bottom)],
        _ => return None,
    };
    Some((w, strokes, t * 1.2))
}

/// `\color` 写的颜色：常见的名字和 `#rrggbb`。
pub(crate) fn named_color(name: &str) -> Option<Color> {
    let name = name.trim();
    let hex = match name.to_ascii_lowercase().as_str() {
        "red" => 0xff0000,
        "green" => 0x008000,
        "blue" => 0x0000ff,
        "orange" => 0xffa500,
        "purple" => 0x800080,
        "gray" | "grey" => 0x808080,
        "black" => 0x000000,
        "white" => 0xffffff,
        "magenta" => 0xff00ff,
        "cyan" => 0x00ffff,
        "yellow" => 0xffff00,
        "brown" => 0xa52a2a,
        "teal" => 0x008080,
        "pink" => 0xffc0cb,
        "violet" => 0xee82ee,
        "olive" => 0x808000,
        "navy" => 0x000080,
        "maroon" => 0x800000,
        "lime" => 0x00ff00,
        other => {
            let digits = other.trim_start_matches('#');
            if digits.len() != 6 {
                return None;
            }
            u32::from_str_radix(digits, 16).ok()?
        }
    };
    Some(Color::hex(hex))
}
