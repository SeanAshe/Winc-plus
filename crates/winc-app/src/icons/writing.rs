//! Markdown 写作命令的图标：格式工具栏与 `/` 命令列表使用，线条风格同其他图标，约 12×12，
//! 以中心点定位。

use super::*;

/// Markdown 写作命令的图形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritingGlyph {
    Bold,
    Italic,
    Strike,
    Code,
    Highlight,
    Link,
    /// 标题，带级别（1–3）。
    Heading(u8),
    Bullet,
    Ordered,
    Task,
    Quote,
    Callout,
    Details,
    CodeBlock,
    Math,
    InlineMath,
    Diagram,
    Table,
    Divider,
    Image,
    Contents,
    Date,
}

/// 绘制写作命令的图标 `glyph`，中心在 `(cx, cy)`。
pub fn writing(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    glyph: WritingGlyph,
    color: Color,
    clip: Rect,
) {
    let l = |x0: f32, y0: f32, x1: f32, y1: f32| Stroke::Line(cx + x0, cy + y0, cx + x1, cy + y1);
    let dot = |frame: &mut Frame<'_>, x: f32, y: f32| {
        let r = Rect::new(cx + x - 1.0, cy + y - 1.0, 2.0, 2.0);
        frame.rect_clipped(r, color, 1.0, clip);
    };
    let shapes: Vec<Stroke> = match glyph {
        WritingGlyph::Bold => {
            // 竖笔加上下两个半圆，下面的稍宽。
            let (x, top, mid, bottom) = (-3.5, -5.5, -0.5, 5.5);
            let (r1, r2) = ((mid - top) / 2.0, (bottom - mid) / 2.0);
            vec![
                l(x, top, x, bottom),
                l(x, top, 0.0, top),
                Stroke::Arc(cx, cy + top + r1, r1, -FRAC_PI_2, FRAC_PI_2),
                l(x, mid, 0.5, mid),
                Stroke::Arc(cx + 0.5, cy + mid + r2, r2, -FRAC_PI_2, FRAC_PI_2),
                l(x, bottom, 0.5, bottom),
            ]
        }
        WritingGlyph::Italic => vec![
            l(-1.0, -5.5, 4.0, -5.5),
            l(-4.0, 5.5, 1.0, 5.5),
            l(1.5, -5.5, -1.5, 5.5),
        ],
        WritingGlyph::Strike => {
            let r = 2.75;
            vec![
                Stroke::Arc(cx, cy - r, r, FRAC_PI_2, PI * 1.8),
                Stroke::Arc(cx, cy + r, r, -FRAC_PI_2, PI * 0.8),
                l(-6.0, 0.0, 6.0, 0.0),
            ]
        }
        WritingGlyph::Code | WritingGlyph::CodeBlock => vec![
            l(-2.5, -4.0, -6.0, 0.0),
            l(-6.0, 0.0, -2.5, 4.0),
            l(2.5, -4.0, 6.0, 0.0),
            l(6.0, 0.0, 2.5, 4.0),
            l(1.0, -5.5, -1.0, 5.5),
        ],
        WritingGlyph::Callout => {
            // 圆角方框里一个「i」。
            let mut shapes = rounded_outline(cx - 6.0, cy - 6.0, 12.0, 12.0, 3.0).to_vec();
            shapes.push(l(0.0, -0.5, 0.0, 3.0));
            frame.rect_clipped(Rect::new(cx - 0.75, cy - 3.5, 1.5, 1.5), color, 0.75, clip);
            shapes
        }
        WritingGlyph::Highlight => vec![
            // 字母 A 下面一道横线。
            l(-4.0, 2.5, 0.0, -6.0),
            l(0.0, -6.0, 4.0, 2.5),
            l(-2.25, -1.25, 2.25, -1.25),
            l(-6.0, 5.75, 6.0, 5.75),
        ],
        WritingGlyph::Link => {
            let mut shapes = capsule(cx - 2.25, cy + 2.25, 3.5, 2.5);
            shapes.extend(capsule(cx + 2.25, cy - 2.25, 3.5, 2.5));
            shapes
        }
        WritingGlyph::Heading(level) => {
            let mut shapes = vec![
                l(-5.5, -5.5, -5.5, 5.5),
                l(-0.5, -5.5, -0.5, 5.5),
                l(-5.5, 0.0, -0.5, 0.0),
            ];
            shapes.extend(digit(cx + 4.0, cy + 5.5, level, 6.0));
            shapes
        }
        WritingGlyph::Bullet => {
            for y in [-4.5, 0.0, 4.5] {
                dot(frame, -5.0, y);
            }
            vec![
                l(-1.5, -4.5, 6.0, -4.5),
                l(-1.5, 0.0, 6.0, 0.0),
                l(-1.5, 4.5, 6.0, 4.5),
            ]
        }
        WritingGlyph::Ordered => {
            // 左侧上下两个序号「1」「2」，各行一道横线。
            let mut shapes = digit(cx - 4.5, cy - 1.5, 1, 4.5);
            shapes.extend(digit(cx - 4.5, cy + 6.0, 2, 4.5));
            shapes.extend([l(-1.0, -3.75, 6.0, -3.75), l(-1.0, 3.75, 6.0, 3.75)]);
            shapes
        }
        WritingGlyph::Task => {
            let mut shapes = rounded_outline(cx - 6.0, cy - 6.0, 12.0, 12.0, 3.0).to_vec();
            shapes.extend([l(-3.0, 0.25, -1.0, 2.25), l(-1.0, 2.25, 3.0, -2.0)]);
            shapes
        }
        WritingGlyph::Quote => vec![
            l(-5.5, -5.5, -5.5, 5.5),
            l(-2.0, -3.5, 6.0, -3.5),
            l(-2.0, 0.0, 6.0, 0.0),
            l(-2.0, 3.5, 3.5, 3.5),
        ],
        WritingGlyph::Details => vec![
            // 首行一个向右的箭头和一道横线，下面缩进的一道横线。
            l(-5.5, -6.0, -3.0, -3.5),
            l(-3.0, -3.5, -5.5, -1.0),
            l(-0.5, -3.5, 6.0, -3.5),
            l(-0.5, 2.0, 6.0, 2.0),
            l(-0.5, 5.5, 4.0, 5.5),
        ],
        WritingGlyph::Math => vec![
            // Σ。
            l(5.0, -5.5, -4.5, -5.5),
            l(-4.5, -5.5, 1.0, 0.0),
            l(1.0, 0.0, -4.5, 5.5),
            l(-4.5, 5.5, 5.0, 5.5),
        ],
        WritingGlyph::InlineMath => vec![
            // 两个 `$` 之间的 x：一对竖线夹着一个叉。
            l(-5.5, -5.5, -5.5, 5.5),
            l(5.5, -5.5, 5.5, 5.5),
            l(-2.5, -2.5, 2.5, 2.5),
            l(2.5, -2.5, -2.5, 2.5),
        ],
        WritingGlyph::Diagram => {
            let mut shapes = rounded_outline(cx - 6.0, cy - 6.0, 6.5, 5.0, 1.25).to_vec();
            shapes.extend(rounded_outline(cx - 0.5, cy + 1.0, 6.5, 5.0, 1.25));
            shapes.extend([l(-2.75, -1.0, -2.75, 3.5), l(-2.75, 3.5, -0.5, 3.5)]);
            shapes
        }
        WritingGlyph::Table => {
            let mut shapes = rounded_outline(cx - 6.0, cy - 6.0, 12.0, 12.0, 2.0).to_vec();
            shapes.extend([l(-6.0, -2.0, 6.0, -2.0), l(-1.0, -2.0, -1.0, 6.0)]);
            shapes
        }
        WritingGlyph::Divider => vec![
            l(-6.0, 0.0, 6.0, 0.0),
            l(-3.5, -4.5, 3.5, -4.5),
            l(-3.5, 4.5, 3.5, 4.5),
        ],
        WritingGlyph::Image => {
            let mut shapes = rounded_outline(cx - 6.0, cy - 6.0, 12.0, 12.0, 2.0).to_vec();
            shapes.extend([
                l(-6.0, 3.5, -2.0, -0.5),
                l(-2.0, -0.5, 1.5, 3.0),
                l(0.5, 2.0, 2.5, 0.5),
                l(2.5, 0.5, 6.0, 4.0),
                Stroke::Circle(cx + 2.5, cy - 2.75, 1.0),
            ]);
            shapes
        }
        WritingGlyph::Contents => vec![
            // 目录：长短交错的四行，第二、四行缩进。
            l(-6.0, -4.5, 6.0, -4.5),
            l(-2.5, -1.5, 6.0, -1.5),
            l(-6.0, 1.5, 6.0, 1.5),
            l(-2.5, 4.5, 6.0, 4.5),
        ],
        WritingGlyph::Date => {
            // 日历：圆角方框，上沿下一道横线，顶上两个挂环。
            let mut shapes = rounded_outline(cx - 6.0, cy - 4.5, 12.0, 10.5, 2.0).to_vec();
            shapes.extend([
                l(-6.0, -1.0, 6.0, -1.0),
                l(-3.0, -6.0, -3.0, -3.0),
                l(3.0, -6.0, 3.0, -3.0),
            ]);
            shapes
        }
    };
    frame.stroke(&shapes, STROKE, color, clip);
}

/// 返回沿左下到右上方向、两端圆心相距 `len`、半径 `r` 的胶囊形描边，中心在 `(cx, cy)`。
fn capsule(cx: f32, cy: f32, len: f32, r: f32) -> Vec<Stroke> {
    let (dx, dy) = (FRAC_1_SQRT_2 * len / 2.0, -FRAC_1_SQRT_2 * len / 2.0);
    let (nx, ny) = (FRAC_1_SQRT_2 * r, FRAC_1_SQRT_2 * r);
    let angle = -PI / 4.0;
    vec![
        Stroke::Arc(cx + dx, cy + dy, r, angle - FRAC_PI_2, angle + FRAC_PI_2),
        Stroke::Arc(cx - dx, cy - dy, r, angle + FRAC_PI_2, angle + PI * 1.5),
        Stroke::Line(cx - dx + nx, cy - dy + ny, cx + dx + nx, cy + dy + ny),
        Stroke::Line(cx - dx - nx, cy - dy - ny, cx + dx - nx, cy + dy - ny),
    ]
}

/// 返回数字 `n`（1–3）的笔画：高 `h`、宽约 `h / 2`，底边中点在 `(x, bottom)`。
fn digit(x: f32, bottom: f32, n: u8, h: f32) -> Vec<Stroke> {
    let (top, r) = (bottom - h, h / 4.0);
    match n {
        1 => vec![
            Stroke::Line(x, top, x, bottom),
            Stroke::Line(x - r, top + r, x, top),
        ],
        2 => vec![
            Stroke::Arc(x, top + r, r, PI, PI * 2.25),
            Stroke::Line(
                x + r * FRAC_1_SQRT_2,
                top + r + r * FRAC_1_SQRT_2,
                x - r,
                bottom,
            ),
            Stroke::Line(x - r, bottom, x + r + 0.25, bottom),
        ],
        _ => vec![
            Stroke::Arc(x, top + r, r, -PI, FRAC_PI_2),
            Stroke::Arc(x, bottom - r, r, -FRAC_PI_2, PI),
        ],
    }
}
