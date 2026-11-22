//! Powerline 分隔符（U+E0B0–E0BF）的绘制：按格子的大小绘制，边缘与相邻格子的色块对齐。

use super::*;
use winc_render::Stroke;

/// 细线分隔符的线宽（逻辑像素），与下划线相同。
const LINE: f32 = 1.0;

/// 细线的半圆拆成的线段数：16 段时相邻线段的夹角约 11°，在一个格子的大小下看不出折点。
const ARC_STEPS: usize = 16;

/// 实心分隔符每行像素取边界的次数：边缘的覆盖率取 4 处的平均，接近按面积计算的结果。
const SAMPLES: usize = 4;

/// 半圆在高度 `t`（0 为格子上沿，1 为下沿）处的宽度，按格子宽度的比例。
fn round(t: f32) -> f32 {
    (1.0 - (2.0 * t - 1.0).powi(2)).max(0.0).sqrt()
}

/// 返回实心分隔符 `c` 在高度 `t` 处填充的左右边界，按格子宽度的比例；其他字符返回 `None`。
fn filled_span(c: char, t: f32) -> Option<(f32, f32)> {
    let wedge = 1.0 - (2.0 * t - 1.0).abs();
    Some(match c {
        '\u{e0b0}' => (0.0, wedge),
        '\u{e0b2}' => (1.0 - wedge, 1.0),
        '\u{e0b4}' => (0.0, round(t)),
        '\u{e0b6}' => (1.0 - round(t), 1.0),
        '\u{e0b8}' => (0.0, t),
        '\u{e0ba}' => (1.0 - t, 1.0),
        '\u{e0bc}' => (0.0, 1.0 - t),
        '\u{e0be}' => (t, 1.0),
        _ => return None,
    })
}

/// 返回细线分隔符 `c` 的折线顶点（x，y），按格子的比例；其他字符返回 `None`。
fn outline(c: char) -> Option<Vec<(f32, f32)>> {
    let arc = |x: fn(f32) -> f32| {
        let steps = (0..=ARC_STEPS).map(|i| i as f32 / ARC_STEPS as f32);
        steps.map(|t| (x(round(t)), t)).collect()
    };
    Some(match c {
        '\u{e0b1}' => vec![(0.0, 0.0), (1.0, 0.5), (0.0, 1.0)],
        '\u{e0b3}' => vec![(1.0, 0.0), (0.0, 0.5), (1.0, 1.0)],
        '\u{e0b5}' => arc(|w| w),
        '\u{e0b7}' => arc(|w| 1.0 - w),
        '\u{e0b9}' | '\u{e0bf}' => vec![(0.0, 0.0), (1.0, 1.0)],
        '\u{e0bb}' | '\u{e0bd}' => vec![(1.0, 0.0), (0.0, 1.0)],
        _ => return None,
    })
}

/// 在格子 `at` 内绘制 Powerline 分隔符 `c`；其他字符不绘制并返回假。
///
/// 字体的字形通常低于行高，直接绘制时相邻格子的色块在分隔符的上下露出。
pub(super) fn draw_powerline(
    frame: &mut Frame<'_>,
    c: char,
    at: Rect,
    color: Color,
    clip: Rect,
) -> bool {
    if let Some(points) = outline(c) {
        let point = |(x, y): (f32, f32)| (at.x + x * at.w, at.y + y * at.h);
        let lines = points.windows(2).map(|pair| {
            let ((x0, y0), (x1, y1)) = (point(pair[0]), point(pair[1]));
            Stroke::Line(x0, y0, x1, y1)
        });
        let lines: Vec<Stroke> = lines.collect();
        frame.stroke(&lines, LINE, color, clip.intersect(at));
        return true;
    }
    if filled_span(c, 0.5).is_none() {
        return false;
    }
    // 逐行物理像素填充：整像素不透明，两端不足一个像素的部分按覆盖的比例。格子的四条边与
    // 色块一样各自取整到物理像素，平边与相邻的色块之间没有缝。
    let scale = frame.scale().max(1.0);
    let edge = |v: f32| (v * scale).round();
    let (x0, y0) = (edge(at.x), edge(at.y));
    let (width, rows) = (edge(at.right()) - x0, (edge(at.bottom()) - y0) as usize);
    for row in 0..rows {
        let (mut left, mut right) = (0.0, 0.0);
        for sample in 0..SAMPLES {
            let t = (row as f32 + (sample as f32 + 0.5) / SAMPLES as f32) / rows as f32;
            let (l, r) = filled_span(c, t).unwrap_or_default();
            left += l;
            right += r;
        }
        let (left, right) = (
            left * width / SAMPLES as f32,
            right * width / SAMPLES as f32,
        );
        let y = (y0 + row as f32) / scale;
        let mut fill = |from: f32, to: f32, opacity: f32| {
            if to > from && opacity > 0.0 {
                let rect = Rect::new((x0 + from) / scale, y, (to - from) / scale, 1.0 / scale);
                frame.rect_clipped(rect, color.alpha(opacity), 0.0, clip);
            }
        };
        let (inner_left, inner_right) = (left.ceil(), right.floor());
        if inner_left > inner_right {
            fill(left.floor(), left.floor() + 1.0, right - left);
        } else {
            fill(left.floor(), inner_left, inner_left - left);
            fill(inner_left, inner_right, 1.0);
            fill(inner_right, inner_right + 1.0, right - inner_right);
        }
    }
    true
}
