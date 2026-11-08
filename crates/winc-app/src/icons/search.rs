//! 查找和全局搜索的按钮：勾选框、行内的替换和勾选、筛选、替换、正则、整词。

use super::*;

/// 差异视图中间一栏按钮的悬停底色：与工具栏的图标按钮相同，悬停时才铺一层中性的底，平时
/// 只有图标；按钮叠在改动的底色上，一直铺淡色方块会与背景混同，看不出按钮边界。
fn line_button_hover(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    hover: Option<Color>,
    clip: Rect,
) {
    if let Some(hover) = hover {
        let rect = Rect::new((cx - 10.0).round(), (cy - 10.0).round(), 20.0, 20.0);
        frame.rect_clipped(rect, hover, 4.0, clip);
    }
}

/// 绘制差异视图里的勾选框，线条风格，与旁边的「»」一致：边长 `size` 的圆角细框，勾上时
/// 框和勾都是强调色，不铺底色；没勾时只有一圈淡色的框。
///
/// `hover` 是悬停时的底色。
#[allow(clippy::too_many_arguments)]
pub fn line_checkbox(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    on: bool,
    hover: Option<Color>,
    accent: Color,
    border: Color,
    clip: Rect,
) {
    line_button_hover(frame, center, hover, clip);
    let rect = checkbox_rect(center, size);
    let outline_color = if on { accent } else { border };
    frame.stroke(&checkbox_outline(rect), 1.2, outline_color, clip);
    if on {
        let k = size / 12.0;
        let (x, y) = (rect.x, rect.y);
        let mark = [
            Stroke::Line(x + 3.2 * k, y + 6.2 * k, x + 5.2 * k, y + 8.2 * k),
            Stroke::Line(x + 5.2 * k, y + 8.2 * k, x + 8.9 * k, y + 4.0 * k),
        ];
        frame.stroke(&mark, MARK_STROKE, accent, clip);
    }
}

/// 绘制只包含部分内容的勾选框：样式与 [`line_checkbox`] 勾上时相同，框内是一条横线。
///
/// `hover` 是悬停时的底色。
pub fn line_checkbox_mixed(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    hover: Option<Color>,
    accent: Color,
    clip: Rect,
) {
    line_button_hover(frame, center, hover, clip);
    let rect = checkbox_rect(center, size);
    frame.stroke(&checkbox_outline(rect), 1.2, accent, clip);
    let (inset, y) = (size * 0.25, rect.y + rect.h / 2.0);
    let mark = [Stroke::Line(rect.x + inset, y, rect.right() - inset, y)];
    frame.stroke(&mark, MARK_STROKE, accent, clip);
}

/// 绘制三栏合并右栏的「«」按钮：把右栏这一段采用到中间，样式与 [`line_replace`] 相同，方向相反。
///
/// `hover` 是悬停时的底色。
pub fn line_replace_back(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    hover: Option<Color>,
    color: Color,
    clip: Rect,
) {
    line_button_hover(frame, (cx, cy), hover, clip);
    let (w, h) = (2.2, 4.0);
    for x in [cx - 2.4, cx + 2.4] {
        let chevron = [
            Stroke::Line(x + w, cy - h, x - w, cy),
            Stroke::Line(x - w, cy, x + w, cy + h),
        ];
        frame.stroke(&chevron, MARK_STROKE, color, clip);
    }
}

/// 绘制差异视图里的「»」按钮（用左侧替换这一段）：两道朝右的箭头，强调色，笔宽和勾选框的勾
/// 一样，大小和勾选框的框相当。
///
/// `hover` 是悬停时的底色。
pub fn line_replace(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    hover: Option<Color>,
    color: Color,
    clip: Rect,
) {
    line_button_hover(frame, (cx, cy), hover, clip);
    let (w, h) = (2.2, 4.0);
    let lines = [cx - 2.4, cx + 2.4].map(|x| {
        [
            Stroke::Line(x - w, cy - h, x + w, cy),
            Stroke::Line(x + w, cy, x - w, cy + h),
        ]
    });
    frame.stroke(lines.as_flattened(), MARK_STROKE, color, clip);
}

/// 勾选框（[`checkbox`]、[`line_checkbox`]）的方框：边长 `size`，以
/// `center` 为中心，左上角取整到整像素。
fn checkbox_rect((cx, cy): (f32, f32), size: f32) -> Rect {
    Rect::new(
        (cx - size / 2.0).round(),
        (cy - size / 2.0).round(),
        size,
        size,
    )
}

/// 勾选框的细框：画在方框 `rect` 里面半个像素，圆角 3。
fn checkbox_outline(rect: Rect) -> [Stroke; 8] {
    rounded_outline(rect.x + 0.5, rect.y + 0.5, rect.w - 1.0, rect.h - 1.0, 3.0)
}

/// 12×12 勾选框里的记号：勾上（`full`）时是一个勾，只勾了一部分时是一道横线。
fn checkbox_mark(rect: Rect, full: bool) -> Strokes<2> {
    let (x, y) = (rect.x, rect.y);
    let mut mark = Strokes::new();
    if full {
        mark.extend([
            Stroke::Line(x + 3.0, y + 6.2, x + 5.2, y + 8.4),
            Stroke::Line(x + 5.2, y + 8.4, x + 9.2, y + 3.8),
        ]);
    } else {
        mark.push(Stroke::Line(x + 3.0, y + 6.0, x + 9.0, y + 6.0));
    }
    mark
}

/// 绘制勾选框，12×12，圆角方框：勾上时铺强调色、画白色的勾；`partial` 时（一组里只勾了一部分）
/// 铺强调色、画一道横线；没勾时只有一圈边框。
pub fn checkbox(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    state: Option<bool>,
    accent: Color,
    border: Color,
    on_accent: Color,
    clip: Rect,
) {
    let rect = checkbox_rect(center, 12.0);
    match state {
        None => frame.stroke(&checkbox_outline(rect), 1.0, border, clip),
        Some(full) => {
            frame.rect_clipped(rect, accent, 3.0, clip);
            let mark = checkbox_mark(rect, full);
            frame.stroke(mark.as_slice(), MARK_STROKE, on_accent, clip);
        }
    }
}

/// 绘制快速打开面板中的放大镜图标：线条版本，不带实心底色。
pub fn magnifier(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (ox, oy, r) = (center.0 - 1.5, center.1 - 1.5, 4.75);
    let k = (r + 1.0) * FRAC_1_SQRT_2;
    let shapes = [
        Stroke::Circle(ox, oy, r),
        Stroke::Line(ox + k, oy + k, center.0 + 6.0, center.1 + 6.0),
    ];
    frame.stroke(&shapes, 1.5, color, clip);
}

/// 绘制「清除搜索结果」图标，12×12：三行文字，右下角叠着一个叉。
pub fn clear_results(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (left, top) = (cx - 6.0, cy - 5.0);
    let lines = [
        Stroke::Line(left, top, left + 11.0, top),
        Stroke::Line(left, top + 4.0, left + 7.0, top + 4.0),
        Stroke::Line(left, top + 8.0, left + 4.0, top + 8.0),
    ];
    let (x, y, h) = (cx + 3.0, cy + 4.0, 2.25);
    let cross = [
        Stroke::Line(x - h, y - h, x + h, y + h),
        Stroke::Line(x - h, y + h, x + h, y - h),
    ];
    frame.stroke(&lines, RING_STROKE, color, clip);
    frame.stroke(&cross, RING_STROKE, color, clip);
}

/// 绘制「过滤」图标，12×11：漏斗，上沿一道横线向下收窄为一根短柄。
pub fn filter(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (top, neck, bottom) = (cy - 5.0, cy + 0.5, cy + 5.5);
    let shapes = [
        Stroke::Line(cx - 6.0, top, cx + 6.0, top),
        Stroke::Line(cx - 6.0, top, cx - 1.5, neck),
        Stroke::Line(cx + 6.0, top, cx + 1.5, neck),
        Stroke::Line(cx - 1.5, neck, cx - 1.5, bottom),
        Stroke::Line(cx + 1.5, neck, cx + 1.5, bottom - 1.5),
        Stroke::Line(cx + 1.5, bottom - 1.5, cx - 1.5, bottom),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「被忽略的文件」图标：一个文件，被一条斜线划过。
pub fn ignored_files(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let file = rounded_outline(cx - 4.5, cy - 6.0, 9.0, 12.0, 1.5);
    let slash = [Stroke::Line(cx - 6.5, cy + 5.5, cx + 6.5, cy - 5.5)];
    frame.stroke(&file, RING_STROKE, color, clip);
    frame.stroke(&slash, RING_STROKE, color, clip);
}

/// 绘制「全部展开」图标，与「全部折叠」成对：卡片相同，卡片内是加号。
pub fn expand_all(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let card = rounded_outline(cx - 5.5, cy - 3.0, 11.0, 9.0, 2.0);
    let back = [Stroke::Line(cx - 3.5, cy - 6.0, cx + 3.5, cy - 6.0)];
    frame.stroke(&card, RING_STROKE, color, clip);
    frame.stroke(&back, RING_STROKE, color, clip);
    frame.stroke(&small_plus(cx, cy + 1.5, 2.5), RING_STROKE, color, clip);
}

/// 绘制「替换」和「全部替换」图标，12×12：一个圆角框（表示输入框中的文字），一支箭头从右上方
/// 横向伸来，圆角拐弯后向下指进框内。
///
/// `stacked` 为 `true` 时框较矮，下方多一道线，表示叠放。
fn replace_icon(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    stacked: bool,
    color: Color,
    clip: Rect,
) {
    let (top, bottom) = if stacked {
        (cy - 2.0, cy + 4.0)
    } else {
        (cy - 1.0, cy + 6.0)
    };
    let (r, arm) = (2.0, 2.25);
    let (stem, from, tip) = (cx, cy - 6.0, top + 4.0);
    let [a, b, c, d, e, f, g, h] = rounded_outline(cx - 6.0, top, 12.0, bottom - top, r);
    // 最后一条是叠放效果下方的那道线，`stacked` 为 `false` 时不画。箭头穿过框的上沿，需要和框
    // 一起描边，交叉处才不会叠出更深的颜色。
    let shapes = [
        Stroke::Line(cx + 5.5, from, stem + r, from),
        Stroke::Arc(stem + r, from + r, r, -PI, -FRAC_PI_2),
        Stroke::Line(stem, from + r, stem, tip),
        Stroke::Line(stem - arm, tip - arm, stem, tip),
        Stroke::Line(stem + arm, tip - arm, stem, tip),
        a,
        b,
        c,
        d,
        e,
        f,
        g,
        h,
        Stroke::Line(cx - 4.0, cy + 6.5, cx + 4.0, cy + 6.5),
    ];
    let count = if stacked {
        shapes.len()
    } else {
        shapes.len() - 1
    };

    frame.stroke(&shapes[..count], RING_STROKE, color, clip);
}

/// 绘制「替换」图标，12×12。
pub fn replace_one(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    replace_icon(frame, center, false, color, clip);
}

/// 绘制「全部替换」图标，12×12：与「替换」相同，框下方多一道线。
pub fn replace_all(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    replace_icon(frame, center, true, color, clip);
}

/// 绘制「正则」图标，约 12×12：左下一个点，右上一个星号，即「.*」。
pub fn regex(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (sx, sy, r) = (cx + 2.0, cy - 2.0, 3.5);
    let (dx, dy) = (r * 0.866, r * 0.5);
    let star = [
        Stroke::Line(sx, sy - r, sx, sy + r),
        Stroke::Line(sx - dx, sy - dy, sx + dx, sy + dy),
        Stroke::Line(sx - dx, sy + dy, sx + dx, sy - dy),
    ];
    frame.stroke(&star, GLYPH_STROKE, color, clip);

    // 零长度的线段画出来就是圆点。
    let dot = Stroke::Line(cx - 3.5, cy + 4.0, cx - 3.5, cy + 4.0);
    frame.stroke(&[dot], 2.75, color, clip);
}

/// 绘制全字匹配开关中文字下方的托架：一条横线，两端向上翘，宽度为 `width`。
pub fn word_bracket(
    frame: &mut Frame<'_>,
    (cx, y): (f32, f32),
    width: f32,
    color: Color,
    clip: Rect,
) {
    let (left, right) = (cx - width / 2.0, cx + width / 2.0);
    let lines = [
        Stroke::Line(left, y - 2.5, left, y),
        Stroke::Line(left, y, right, y),
        Stroke::Line(right, y, right, y - 2.5),
    ];
    frame.stroke(&lines, GLYPH_STROKE, color, clip);
}
