//! 设置浮层左侧各页的图标（外观、代码、格式化、终端、模块、快捷键），以及锁、预览的眼睛、齿轮。

use super::*;

/// 绘制锁图标，9×12：线条风格，锁梁加圆角的锁身，锁身中间一道短竖线是锁孔。
pub fn lock(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (r, top) = (2.25, cy - 2.5);
    let h = GLYPH_STROKE / 2.0;
    let shackle = [
        Stroke::Arc(cx, top, r, -PI, 0.0),
        Stroke::Line(cx - r, top, cx - r, cy - 0.5 + h),
        Stroke::Line(cx + r, top, cx + r, cy - 0.5 + h),
        Stroke::Line(cx, cy + 1.75, cx, cy + 3.0),
    ];
    let body = rounded_outline(
        cx - 4.5 + h,
        cy - 0.5 + h,
        9.0 - 2.0 * h,
        6.0 - 2.0 * h,
        1.5,
    );
    let shapes: Strokes<12> = joined(&[&shackle, &body]);
    frame.stroke(shapes.as_slice(), GLYPH_STROKE, color, clip);
}

/// 绘制「外观」图标，12×12：一个圆，右半边实心，表示浅色和深色两套配色。
///
/// 用于设置的分类栏。
pub fn appearance(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    frame.stroke(&[Stroke::Circle(cx, cy, RING)], RING_STROKE, color, clip);
    // 半径 `RING / 2`、笔宽 `RING` 的圆正好铺满整个圆面，裁掉左半边。
    let half = Rect::new(cx, cy - RING, RING, RING * 2.0).intersect(clip);
    frame.stroke(&[Stroke::Circle(cx, cy, RING / 2.0)], RING, color, half);
}

/// 绘制「编辑器」图标，14×11：一对尖括号，中间一道斜线，即「</>」。
///
/// 用于设置的分类栏。
pub fn code(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    code_scaled(frame, center, 1.0, color, clip);
}

/// 按比例 `k` 缩放绘制 [`code`] 图标，线宽不变；用于出现、消失的动画。
pub fn code_scaled(frame: &mut Frame<'_>, (cx, cy): (f32, f32), k: f32, color: Color, clip: Rect) {
    let lines = [
        Stroke::Line(cx - 3.5 * k, cy - 4.0 * k, cx - 7.0 * k, cy),
        Stroke::Line(cx - 7.0 * k, cy, cx - 3.5 * k, cy + 4.0 * k),
        Stroke::Line(cx + 3.5 * k, cy - 4.0 * k, cx + 7.0 * k, cy),
        Stroke::Line(cx + 7.0 * k, cy, cx + 3.5 * k, cy + 4.0 * k),
        Stroke::Line(cx + 1.5 * k, cy - 5.5 * k, cx - 1.5 * k, cy + 5.5 * k),
    ];
    frame.stroke(&lines, RING_STROKE, color, clip);
}

/// 绘制「预览」图标，15×9：一只眼睛，上下两道圆弧围成眼眶，中间一个圆的瞳孔。
///
/// 图形按比例 `k` 缩放，线宽不变；出现、消失的动画中 `k` 逐渐变化。
pub fn eye(frame: &mut Frame<'_>, (cx, cy): (f32, f32), k: f32, color: Color, clip: Rect) {
    // 眼角在 (cx ± 7.5, cy)，眼眶高 4.5：圆弧半径 (7.5² + 4.5²) / (2 × 4.5) = 8.5，圆心离中线 4。
    let corner = (4.0_f32).atan2(7.5);
    let lids = [
        Stroke::Arc(cx, cy + 4.0 * k, 8.5 * k, -PI + corner, -corner),
        Stroke::Arc(cx, cy - 4.0 * k, 8.5 * k, corner, PI - corner),
        Stroke::Circle(cx, cy, 2.0 * k),
    ];
    frame.stroke(&lids, RING_STROKE, color, clip);
}

/// 绘制「在浏览器中打开」图标，13×13：一个地球，外圈一个圆，中间一道竖着的经线（两道圆弧
/// 围成的梭形）和一道横着的赤道。
///
/// 图形按比例 `k` 缩放，线宽不变；出现、消失的动画中 `k` 逐渐变化。
pub fn globe(frame: &mut Frame<'_>, (cx, cy): (f32, f32), k: f32, color: Color, clip: Rect) {
    let r = 6.5 * k;
    // 经线：半宽 2.8、上下顶到外圈的梭形，由两道圆弧围成。圆弧过 (0, ±r) 和 (±2.8, 0)：
    // 半径 (r² + 2.8²) / (2 × 2.8)，圆心离中线 半径 - 2.8。
    let half = 2.8 * k;
    let radius = (r * r + half * half) / (2.0 * half);
    let offset = radius - half;
    let span = r.atan2(offset);
    let lines = [
        Stroke::Circle(cx, cy, r),
        Stroke::Arc(cx - offset, cy, radius, -span, span),
        Stroke::Arc(cx + offset, cy, radius, PI - span, PI + span),
        Stroke::Line(cx - r, cy, cx + r, cy),
    ];
    frame.stroke(&lines, RING_STROKE, color, clip);
}

/// 绘制「格式化」图标，14×10：四道长短、缩进各不相同的横线，表示排版后的代码。
///
/// 用于设置的分类栏。
pub fn format(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let lines = [
        Stroke::Line(cx - 6.5, cy - 4.5, cx + 4.5, cy - 4.5),
        Stroke::Line(cx - 2.5, cy - 1.5, cx + 6.5, cy - 1.5),
        Stroke::Line(cx - 2.5, cy + 1.5, cx + 3.5, cy + 1.5),
        Stroke::Line(cx - 6.5, cy + 4.5, cx + 0.5, cy + 4.5),
    ];
    frame.stroke(&lines, RING_STROKE, color, clip);
}

/// 绘制「终端」图标，15×11：圆角框，内有提示符 `>_`。
///
/// 用于设置的分类栏。
pub fn console(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (w, h) = (15.0, 11.0);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let prompt = [
        Stroke::Line(x + 3.5, y + 3.0, x + 6.0, y + 5.5),
        Stroke::Line(x + 6.0, y + 5.5, x + 3.5, y + 8.0),
        Stroke::Line(x + 7.5, y + 8.0, x + 11.0, y + 8.0),
    ];
    let shapes: Strokes<11> = joined(&[&rounded_outline(x, y, w, h, 2.5), &prompt]);
    frame.stroke(shapes.as_slice(), RING_STROKE, color, clip);
}

/// 绘制「主机」图标，13×12：上下两个圆角的机箱，各有一道短线表示指示灯。
///
/// 用于连接远端主机的入口和主机列表。
pub fn server(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (w, h) = (13.0, 5.0);
    let x = cx - w / 2.0;
    // 两个机箱之间留 2 的空隙，整体在竖直方向居中。
    let (top, bottom) = (cy - 6.0, cy + 1.0);
    let lights = [
        Stroke::Line(x + 3.0, top + 2.5, x + 5.0, top + 2.5),
        Stroke::Line(x + 3.0, bottom + 2.5, x + 5.0, bottom + 2.5),
    ];
    let shapes: Strokes<18> = joined(&[
        &rounded_outline(x, top, w, h, 1.5),
        &rounded_outline(x, bottom, w, h, 1.5),
        &lights,
    ]);
    frame.stroke(shapes.as_slice(), RING_STROKE, color, clip);
}

/// 绘制「模块」图标，13×13：四个圆角小方块排成两行两列，表示可以拼装的部件。
///
/// 用于设置的分类栏。
pub fn modules(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let outlines = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].map(|(dx, dy)| {
        let x = cx + dx * 3.5 - 2.5;
        let y = cy + dy * 3.5 - 2.5;
        rounded_outline(x, y, 5.0, 5.0, 1.2)
    });
    frame.stroke(outlines.as_flattened(), RING_STROKE, color, clip);
}

/// 绘制「关于」图标，13×13：一个圆圈，中间一个小写的「i」（上面一点、下面一竖）。
///
/// 用于设置的分类栏。
pub fn about(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    frame.stroke(&[Stroke::Circle(cx, cy, RING)], RING_STROKE, color, clip);
    frame.rect_clipped(Rect::new(cx - 0.8, cy - 3.6, 1.6, 1.6), color, 0.8, clip);
    let stem = [Stroke::Line(cx, cy - 0.8, cx, cy + 3.4)];
    frame.stroke(&stem, RING_STROKE, color, clip);
}

/// 绘制「快捷键」图标，15×11：圆角键盘外框，内有两排键和一道空格键。
///
/// 用于设置的分类栏。
pub fn keyboard(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (w, h) = (15.0, 11.0);
    let outline = rounded_outline(cx - w / 2.0, cy - h / 2.0, w, h, 2.5);
    frame.stroke(&outline, RING_STROKE, color, clip);
    for row in 0..2 {
        let y = cy - 2.5 + row as f32 * 2.5;
        for col in 0..4 {
            let x = cx - 4.5 + col as f32 * 3.0;
            frame.rect_clipped(Rect::new(x - 0.75, y - 0.75, 1.5, 1.5), color, 0.5, clip);
        }
    }
    let space = [Stroke::Line(cx - 3.0, cy + 3.0, cx + 3.0, cy + 3.0)];
    frame.stroke(&space, RING_STROKE, color, clip);
}

/// 绘制「设置」图标（齿轮），约 14×14：线条风格，一圈齿圈（圆加八个短齿）和中间的轴孔，笔宽
/// `STROKE`。
///
/// 欢迎页的「设置」一行使用它。
pub fn gear(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    frame.stroke(&gear_shape(center, (4.6, 6.4, 1.9)), STROKE, color, clip);
}

/// 齿轮的笔画：齿圈是一条闭合折线，八个梯形的齿，齿根在半径 `root`、齿顶在半径 `tip`，齿顶比
/// 齿根窄；中间半径 `hub` 的轴孔。
///
/// 齿画成梯形而不是径向短线：短线围成一圈时形似放射状的光晕，看不出是齿。
pub(super) fn gear_shape((cx, cy): (f32, f32), (root, tip, hub): (f32, f32, f32)) -> [Stroke; 33] {
    let step = PI / 4.0;
    let (base, top) = (step * 0.3, step * 0.17);
    let at = |angle: f32, r: f32| (cx + r * angle.cos(), cy + r * angle.sin());
    // 每个齿四个点：齿根左、齿顶左、齿顶右、齿根右。
    let points: [(f32, f32); 32] = std::array::from_fn(|j| {
        let a = (j / 4) as f32 * step;
        match j % 4 {
            0 => at(a - base, root),
            1 => at(a - top, tip),
            2 => at(a + top, tip),
            _ => at(a + base, root),
        }
    });
    std::array::from_fn(|i| match points.get(i) {
        Some(&p) => {
            let q = points[(i + 1) % points.len()];
            Stroke::Line(p.0, p.1, q.0, q.1)
        }
        None => Stroke::Circle(cx, cy, hub),
    })
}

/// 运行（三角形），约 10×12。
pub fn play(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (left, right, half) = (cx - 4.0, cx + 5.0, 5.5);
    let lines = [
        Stroke::Line(left, cy - half, right, cy),
        Stroke::Line(right, cy, left, cy + half),
        Stroke::Line(left, cy + half, left, cy - half),
    ];
    frame.stroke(&lines, RING_STROKE, color, clip);
}

/// 停止（方形），10×10。
pub fn stop(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let r = 5.0;
    let lines = [
        Stroke::Line(cx - r, cy - r, cx + r, cy - r),
        Stroke::Line(cx + r, cy - r, cx + r, cy + r),
        Stroke::Line(cx + r, cy + r, cx - r, cy + r),
        Stroke::Line(cx - r, cy + r, cx - r, cy - r),
    ];
    frame.stroke(&lines, RING_STROKE, color, clip);
}
