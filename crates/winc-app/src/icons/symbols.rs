//! 大纲中各类符号的图标：线条风格，与文件树的文件图标同宽、同笔宽，按种类着色。

use super::*;

/// 符号图标的图形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolGlyph {
    /// 函数、方法：一个立方体。
    Cube,
    /// 类型（类、结构体、接口……）：一张卡片，上沿下面一道横线隔出表头。
    Card,
    /// 模块、命名空间：一对花括号。
    Braces,
    /// 常量、变量：一个圆，中间一点。
    Field,
    /// Markdown 标题：一段文字，三行横线，最后一行短。
    Text,
    /// 配置文件的小节：一对方括号。
    Brackets,
}

/// 绘制符号图标：左上角在 `(x, y)`，占 `FILE_WIDTH` × 15 的位置，与 [`file()`] 相同，文件树和
/// 大纲里同一列的图标上下、左右都对齐。
pub fn symbol(
    frame: &mut Frame<'_>,
    (x, y): (f32, f32),
    glyph: SymbolGlyph,
    color: Color,
    clip: Rect,
) {
    let (cx, cy) = (x + FILE_WIDTH / 2.0, y + 7.5);
    let k = MARK_SCALE;
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Stroke::Line(cx + x0 * k, cy + y0 * k, cx + x1 * k, cy + y1 * k)
    };
    match glyph {
        SymbolGlyph::Cube => {
            // 正六边形的外框，加上从中心伸向左上、右上和正下方的三条棱。
            let (w, h) = (3.0, 1.75);
            let marks = [
                line(0.0, -2.0 * h, w, -h),
                line(w, -h, w, h),
                line(w, h, 0.0, 2.0 * h),
                line(0.0, 2.0 * h, -w, h),
                line(-w, h, -w, -h),
                line(-w, -h, 0.0, -2.0 * h),
                line(-w, -h, 0.0, 0.0),
                line(0.0, 0.0, w, -h),
                line(0.0, 0.0, 0.0, 2.0 * h),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        SymbolGlyph::Card => {
            let (w, h) = (6.0 * k, 6.0 * k);
            let card = rounded_outline(cx - w / 2.0, cy - h / 2.0, w, h, 1.5);
            frame.stroke(&card, STROKE, color, clip);
            frame.stroke(&[line(-3.0, -1.0, 3.0, -1.0)], STROKE, color, clip);
        }
        SymbolGlyph::Braces => {
            // 两个花括号：竖着的一笔中间向外顶出一个尖，与 JSON 文件的图标相同。
            let brace = |s: f32| {
                [
                    line(s * 1.0, -3.5, s * 2.0, -3.0),
                    line(s * 2.0, -3.0, s * 2.0, -0.75),
                    line(s * 2.0, -0.75, s * 3.0, 0.0),
                    line(s * 3.0, 0.0, s * 2.0, 0.75),
                    line(s * 2.0, 0.75, s * 2.0, 3.0),
                    line(s * 2.0, 3.0, s * 1.0, 3.5),
                ]
            };
            frame.stroke(&brace(-1.0), STROKE, color, clip);
            frame.stroke(&brace(1.0), STROKE, color, clip);
        }
        SymbolGlyph::Field => {
            frame.stroke(&[Stroke::Circle(cx, cy, 3.0 * k)], STROKE, color, clip);
            // 零长度的线段描边后呈圆点，直径等于笔宽。
            frame.stroke(&[Stroke::Line(cx, cy, cx, cy)], DOT, color, clip);
        }
        SymbolGlyph::Text => {
            // 标题文字前已写出 `#`，图标不再重复井号，只表示这是一段文字。
            let marks = [
                line(-3.25, -2.5, 3.25, -2.5),
                line(-3.25, 0.0, 3.25, 0.0),
                line(-3.25, 2.5, 1.0, 2.5),
            ];
            frame.stroke(&marks, STROKE, color, clip);
        }
        SymbolGlyph::Brackets => {
            let bracket = |s: f32| {
                [
                    line(s * 1.25, -3.25, s * 3.0, -3.25),
                    line(s * 3.0, -3.25, s * 3.0, 3.25),
                    line(s * 3.0, 3.25, s * 1.25, 3.25),
                ]
            };
            frame.stroke(&bracket(-1.0), STROKE, color, clip);
            frame.stroke(&bracket(1.0), STROKE, color, clip);
        }
    }
}
