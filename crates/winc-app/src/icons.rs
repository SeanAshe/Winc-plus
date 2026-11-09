//! 界面图标：用圆角矩形和描边在运行时绘制，不依赖图片资源。
//!
//! 所有图标共用一套线条风格，与顶栏的分段控件、文件树的折叠箭头相同：圆头描边，笔宽
//! `STROKE`，小图标里的记号用 `GLYPH_STROKE`；只有行号栏的书签（标记）与「外观」的半个圆面
//! 为实心。
//!
//! 图标尺寸以「宽×高」标注，单位为逻辑像素。多数图标以中心点定位；文件、目录、新建文件与文档
//! 图标以左上角定位，具体见各函数的参数说明。

mod files;
mod search;
mod settings;
mod symbols;

pub use files::*;
pub use search::*;
pub use settings::*;
pub use symbols::*;

use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI};

use winc_render::{Color, Frame, Rect, Stroke};

/// 线条图标的笔宽（逻辑像素）。
///
/// 顶栏、目录、新建文件、工具栏图标与文件内的记号共用该笔宽，并排放在同一行时笔画粗细一致。
/// 取值为调参结果：笔宽在 1.25 逻辑像素时，圆头描边在 1 倍与 2 倍像素密度下都不出现断点。
const STROKE: f32 = 1.25;
/// 文件图标纸面上记号的笔宽（逻辑像素），等于 `STROKE`。
const GLYPH_STROKE: f32 = STROKE;
/// 文件图标的记号相对设计稿放大的倍数（无单位）。
///
/// 设计稿的格子为 ±3.5 逻辑像素；按该倍数放大后记号约 11 逻辑像素见方，与目录图标的外形尺寸
/// 相同。取值为调参结果：放大到该倍数时记号在图标纸面内留出约 1 逻辑像素的边距。
const MARK_SCALE: f32 = 1.55;
/// 文件图标记号里圆点的直径（逻辑像素）。
///
/// 约为 `STROKE` 的两倍：记号按 `MARK_SCALE` 放大后仍能分辨出一个实心圆点。
const DOT: f32 = 2.4;
/// 箭头（折叠展开、路径分隔、下拉）的笔宽（逻辑像素）。
///
/// 箭头只有两笔，按 `STROKE` 绘制时视觉重量高于同一行的文字，因此单独取更细的笔宽；取值为
/// 调参结果，作用范围限于上述三类箭头。
const CHEVRON_STROKE: f32 = 1.2;
/// 勾选框内的勾与差异视图「»」按钮的笔宽（逻辑像素）。
///
/// 两者在差异视图里并排显示，取同一个值以保证笔画粗细一致；取值为调参结果。
const MARK_STROKE: f32 = 1.5;
/// 文件图标的宽度（逻辑像素）。
pub const FILE_WIDTH: f32 = 12.0;
/// 目录图标的宽度（逻辑像素），也是文件树中目录图标与文件图标所占列的列宽。
///
/// 目录图标与文件图标同在一列时，列宽取该值，文件图标在列内居中向右偏移
/// `(FOLDER_WIDTH - FILE_WIDTH) / 2`，两种图标的中心对齐，名称从同一位置起始。取值由
/// 目录图标的设计稿宽度决定。
pub const FOLDER_WIDTH: f32 = 15.0;
/// 侧栏开关图标（`sidebar`）的宽度（逻辑像素）。取值为调参结果：宽度为偶数，取整后左右两侧
/// 的留白对称。
pub const SIDEBAR_WIDTH: f32 = 16.0;

/// 箭头 `V` 形折线的开口朝向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 朝上。
    Up,
    /// 朝下。
    Down,
    /// 朝右。
    Right,
    /// 朝左。
    Left,
}

/// 按给定朝向绘制 `V` 形折线箭头。
///
/// `size` 是箭头两端尖点的连线长度（逻辑像素）：朝上或朝下时为宽度，朝右时为高度。
pub fn chevron(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    direction: Direction,
    color: Color,
    clip: Rect,
) {
    let angle = match direction {
        Direction::Up => -FRAC_PI_2,
        Direction::Down => FRAC_PI_2,
        Direction::Right => 0.0,
        Direction::Left => PI,
    };
    chevron_turned(frame, center, size, angle, color, clip);
}

/// 绘制自朝右状态顺时针转过 `angle` 弧度（`f32`，弧度制）的 `V` 形折线箭头。
///
/// 目录展开、收起时，箭头在朝右与朝下之间按动画进度转过中间的弧度。
pub fn chevron_turned(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    size: f32,
    angle: f32,
    color: Color,
    clip: Rect,
) {
    let (a, b) = (size / 2.0, size / 4.0);
    let (sin, cos) = angle.sin_cos();
    // 屏幕坐标的 `y` 轴向下，该旋转在屏幕上表现为顺时针。
    let turn = |(x, y): (f32, f32)| (cx + x * cos - y * sin, cy + x * sin + y * cos);
    let [p0, p1, p2] = [(-b, -a), (b, 0.0), (-b, a)].map(turn);
    let lines = [
        Stroke::Line(p0.0, p0.1, p1.0, p1.1),
        Stroke::Line(p1.0, p1.1, p2.0, p2.1),
    ];
    frame.stroke(&lines, CHEVRON_STROKE, color, clip);
}

/// 绘制收起图标：朝向 `direction` 的箭头，箭头尖端外侧一条边线，表示面板收入的方向。
///
/// 单独的箭头是下拉菜单的图标，边线用于区分两者。
pub fn collapse(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    direction: Direction,
    color: Color,
    clip: Rect,
) {
    let (dx, dy) = match direction {
        Direction::Up => (0.0, -1.0),
        Direction::Down => (0.0, 1.0),
        Direction::Right => (1.0, 0.0),
        Direction::Left => (-1.0, 0.0),
    };
    chevron(
        frame,
        (cx - 2.0 * dx, cy - 2.0 * dy),
        8.0,
        direction,
        color,
        clip,
    );
    let (ex, ey) = (cx + 3.0 * dx, cy + 3.0 * dy);
    // 边线和箭头同宽，垂直于箭头的朝向。
    let (hx, hy) = (4.0 * dy.abs(), 4.0 * dx.abs());
    let edge = [Stroke::Line(ex - hx, ey - hy, ex + hx, ey + hy)];
    frame.stroke(&edge, CHEVRON_STROKE, color, clip);
}

/// 绘制弹出菜单按钮右端上下排列的一对小箭头，整体宽 6、高 8 逻辑像素。
pub fn up_down(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (w, h, gap) = (3.0, 2.5, 1.5);
    let lines = [
        Stroke::Line(cx - w, cy - gap, cx, cy - gap - h),
        Stroke::Line(cx, cy - gap - h, cx + w, cy - gap),
        Stroke::Line(cx - w, cy + gap, cx, cy + gap + h),
        Stroke::Line(cx, cy + gap + h, cx + w, cy + gap),
    ];
    frame.stroke(&lines, CHEVRON_STROKE, color, clip);
}

/// 绘制外链箭头，9×9：指向右上方的箭头，表示在浏览器中打开。
pub fn external(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let r = 3.5;
    let lines = [
        Stroke::Line(cx - r, cy + r, cx + r, cy - r),
        Stroke::Line(cx - 1.0, cy - r, cx + r, cy - r),
        Stroke::Line(cx + r, cy - r, cx + r, cy + 1.0),
    ];
    frame.stroke(&lines, CHEVRON_STROKE, color, clip);
}

/// 绘制提醒图标，12×11：三角形内一个感叹号。
pub fn warning(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (top, bottom, half) = (cy - 5.0, cy + 4.5, 5.5);
    let edges = [
        Stroke::Line(cx, top, cx + half, bottom),
        Stroke::Line(cx + half, bottom, cx - half, bottom),
        Stroke::Line(cx - half, bottom, cx, top),
        Stroke::Line(cx, cy - 1.5, cx, cy + 1.0),
    ];
    frame.stroke(&edges, GLYPH_STROKE, color, clip);
    frame.rect_clipped(Rect::new(cx - 0.75, cy + 2.25, 1.5, 1.5), color, 0.75, clip);
}

/// 绘制关闭图标（×），`size` 是叉的边长。
pub fn close(frame: &mut Frame<'_>, (cx, cy): (f32, f32), size: f32, color: Color, clip: Rect) {
    let h = size / 2.0;
    let lines = [
        Stroke::Line(cx - h, cy - h, cx + h, cy + h),
        Stroke::Line(cx - h, cy + h, cx + h, cy - h),
    ];
    frame.stroke(&lines, STROKE, color, clip);
}

/// 绘制窗口的最小化图标：一条横线，`size` 是线长。
pub fn minimize(frame: &mut Frame<'_>, (cx, cy): (f32, f32), size: f32, color: Color, clip: Rect) {
    let h = size / 2.0;
    frame.stroke(&[Stroke::Line(cx - h, cy, cx + h, cy)], STROKE, color, clip);
}

/// 绘制窗口的最大化图标：一个方框，`size` 为边长；`restore` 为 `true` 时绘制还原图标，即前后
/// 错开 2 逻辑像素的两个方框。
pub fn maximize(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    size: f32,
    restore: bool,
    color: Color,
    clip: Rect,
) {
    let square = |x0: f32, y0: f32, s: f32| {
        [
            Stroke::Line(x0, y0, x0 + s, y0),
            Stroke::Line(x0 + s, y0, x0 + s, y0 + s),
            Stroke::Line(x0 + s, y0 + s, x0, y0 + s),
            Stroke::Line(x0, y0 + s, x0, y0),
        ]
    };
    let h = size / 2.0;
    if !restore {
        frame.stroke(&square(cx - h, cy - h, size), STROKE, color, clip);
        return;
    }
    // 前面的方框在左下，后面的方框只露出右上的两条边。
    let s = size - 2.0;
    frame.stroke(&square(cx - h, cy - h + 2.0, s), STROKE, color, clip);
    let back = [
        Stroke::Line(cx - h + 2.0, cy - h + 2.0, cx - h + 2.0, cy - h),
        Stroke::Line(cx - h + 2.0, cy - h, cx + h, cy - h),
        Stroke::Line(cx + h, cy - h, cx + h, cy + h - 2.0),
        Stroke::Line(cx + h, cy + h - 2.0, cx + h - 2.0, cy + h - 2.0),
    ];
    frame.stroke(&back, STROKE, color, clip);
}

/// 绘制加号，`size` 是边长。
pub fn plus(frame: &mut Frame<'_>, (cx, cy): (f32, f32), size: f32, color: Color, clip: Rect) {
    let h = size / 2.0;
    let lines = [
        Stroke::Line(cx - h, cy, cx + h, cy),
        Stroke::Line(cx, cy - h, cx, cy + h),
    ];
    frame.stroke(&lines, STROKE, color, clip);
}

/// 绘制减号，笔宽和长度与加号相同。
pub fn minus(frame: &mut Frame<'_>, (cx, cy): (f32, f32), size: f32, color: Color, clip: Rect) {
    let h = size / 2.0;
    frame.stroke(&[Stroke::Line(cx - h, cy, cx + h, cy)], STROKE, color, clip);
}

/// 至多 `N` 个笔画的定长缓冲，用于在栈上拼接一个图标的笔画。
///
/// 图标每帧都绘制，因此不为笔画在堆上分配内存。超出容量的笔画被丢弃；容量按各图标实际用到的
/// 笔画数取，超出容量只在调试构建中由断言检查。
struct Strokes<const N: usize> {
    items: [Stroke; N],
    len: usize,
}

impl<const N: usize> Strokes<N> {
    fn new() -> Self {
        Self {
            items: [Stroke::Line(0.0, 0.0, 0.0, 0.0); N],
            len: 0,
        }
    }

    fn push(&mut self, stroke: Stroke) {
        debug_assert!(self.len < N, "图标的笔画超过 {N} 个");
        if let Some(slot) = self.items.get_mut(self.len) {
            *slot = stroke;
            self.len += 1;
        }
    }

    fn extend(&mut self, strokes: impl IntoIterator<Item = Stroke>) {
        strokes.into_iter().for_each(|s| self.push(s));
    }

    fn as_slice(&self) -> &[Stroke] {
        &self.items[..self.len]
    }
}

/// 把若干组笔画依次接成一组 `N` 个笔画的缓冲。
fn joined<const N: usize>(parts: &[&[Stroke]]) -> Strokes<N> {
    let mut out = Strokes::new();
    for part in parts {
        out.extend(part.iter().copied());
    }
    out
}

// 文件树工具栏的五个图标都是线条图标：笔宽相同，外形约 12×12，并排放置时粗细和大小一致。

/// 工具栏图标的笔宽（逻辑像素），等于 `STROKE`。
const RING_STROKE: f32 = STROKE;
/// 刷新与定位图标中圆的半径（逻辑像素）。
///
/// 工具栏其余图标的外形按该圆的直径（11 逻辑像素）确定大小，使并排显示的工具栏图标尺寸一致；
/// 刷新与历史图标在同一个圆上绘制，路径与箭头按该半径计算。
const RING: f32 = 5.5;

/// 返回圆角矩形描边的八段笔画：四条边加四段圆角弧线。
///
/// 矩形左上角为 `(x, y)`，宽 `w`、高 `h`，圆角半径 `r`，单位均为逻辑像素；`r` 不超过
/// `w / 2` 与 `h / 2` 中的较小值。
pub(crate) fn rounded_outline(x: f32, y: f32, w: f32, h: f32, r: f32) -> [Stroke; 8] {
    let (right, bottom) = (x + w, y + h);
    [
        Stroke::Line(x + r, y, right - r, y),
        Stroke::Arc(right - r, y + r, r, -FRAC_PI_2, 0.0),
        Stroke::Line(right, y + r, right, bottom - r),
        Stroke::Arc(right - r, bottom - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(right - r, bottom, x + r, bottom),
        Stroke::Arc(x + r, bottom - r, r, FRAC_PI_2, PI),
        Stroke::Line(x, bottom - r, x, y + r),
        Stroke::Arc(x + r, y + r, r, -PI, -FRAC_PI_2),
    ]
}

/// 返回以 `(cx, cy)` 为中心、单臂长 `h`（逻辑像素）的加号笔画，用于图标的内部记号。
fn small_plus(cx: f32, cy: f32, h: f32) -> [Stroke; 2] {
    [
        Stroke::Line(cx - h, cy, cx + h, cy),
        Stroke::Line(cx, cy - h, cx, cy + h),
    ]
}

/// 绘制「新建文件」图标，10×12：右上角折起的纸，纸上一个加号。
pub fn add_file(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (left, top, right, bottom) = (cx - 5.0, cy - 6.0, cx + 5.0, cy + 6.0);
    let (r, fold) = (1.5, 3.5);
    let [h, v] = small_plus(cx, cy + 1.0, 2.25);
    let shapes = [
        Stroke::Line(left + r, top, right - fold, top),
        Stroke::Line(right - fold, top, right, top + fold),
        Stroke::Line(right, top + fold, right, bottom - r),
        Stroke::Arc(right - r, bottom - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(right - r, bottom, left + r, bottom),
        Stroke::Arc(left + r, bottom - r, r, FRAC_PI_2, PI),
        Stroke::Line(left, bottom - r, left, top + r),
        Stroke::Arc(left + r, top + r, r, -PI, -FRAC_PI_2),
        h,
        v,
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「新建目录」图标，13×11：左上角带标签的目录，中部一个加号。
pub fn add_folder(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (left, top, right, bottom) = (cx - 6.5, cy - 5.5, cx + 6.5, cy + 5.5);
    let (r, shoulder) = (1.5, top + 2.0);
    let [h, v] = small_plus(cx, cy + 1.0, 2.25);
    let shapes = [
        // 目录标签的轮廓：沿顶边画到一半，斜向下落到肩线上，再向右延伸。
        Stroke::Arc(left + r, top + r, r, -PI, -FRAC_PI_2),
        Stroke::Line(left + r, top, left + 4.5, top),
        Stroke::Line(left + 4.5, top, left + 6.5, shoulder),
        Stroke::Line(left + 6.5, shoulder, right - r, shoulder),
        Stroke::Arc(right - r, shoulder + r, r, -FRAC_PI_2, 0.0),
        Stroke::Line(right, shoulder + r, right, bottom - r),
        Stroke::Arc(right - r, bottom - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(right - r, bottom, left + r, bottom),
        Stroke::Arc(left + r, bottom - r, r, FRAC_PI_2, PI),
        Stroke::Line(left, bottom - r, left, top + r),
        h,
        v,
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「刷新」图标，约 13×13：从 3 点钟方向起顺时针绘制 7/8 圆周，在右上方收尾，末端为直角
/// 箭头。
pub fn refresh(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    refresh_turned(frame, center, 0.0, color, clip);
}

/// 回车图标的宽度（逻辑像素）。
pub const RETURN_KEY_WIDTH: f32 = 10.0;

/// 绘制回车键（↩）图标，约 10×9：从右上方向下，再折向左，末端为箭头。
pub fn return_key(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let half = RETURN_KEY_WIDTH / 2.0;
    let (right, left, top, bottom) = (cx + half, cx - half, cy - 4.5, cy + 2.0);
    let arm = 2.5;
    let shapes = [
        Stroke::Line(right, top, right, bottom),
        Stroke::Line(right, bottom, left, bottom),
        Stroke::Line(left, bottom, left + arm, bottom - arm),
        Stroke::Line(left, bottom, left + arm, bottom + arm),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「发布」图标，约 10×11，和「刷新」图标等高：向上的箭头，下面是托盘。
pub fn upload(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (top, bottom) = (cy - RING, cy + RING);
    let arm = 3.0;
    let shapes = [
        Stroke::Line(cx, top, cx, cy + 2.5),
        Stroke::Line(cx, top, cx - arm, top + arm),
        Stroke::Line(cx, top, cx + arm, top + arm),
        Stroke::Line(cx - 5.0, bottom, cx + 5.0, bottom),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「下载」图标，约 10×11，和「发布」图标上下对称：向下的箭头，下面是托盘。
pub fn download(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (top, bottom) = (cy - RING, cy + RING);
    let tip = cy + 2.5;
    let arm = 3.0;
    let shapes = [
        Stroke::Line(cx, top, cx, tip),
        Stroke::Line(cx, tip, cx - arm, tip - arm),
        Stroke::Line(cx, tip, cx + arm, tip - arm),
        Stroke::Line(cx - 5.0, bottom, cx + 5.0, bottom),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制进度环，直径 11，和「刷新」图标的圆一样大：淡色 `track` 的整圆上，从 12 点钟起顺时针
/// 画出 `fraction`（0 到 1）那么长的一段 `color`。
pub fn progress_ring(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    fraction: f32,
    color: Color,
    track: Color,
    clip: Rect,
) {
    frame.stroke(
        &[Stroke::Arc(cx, cy, RING, -PI, PI)],
        RING_STROKE,
        track,
        clip,
    );
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let start = -FRAC_PI_2;
        let arc = Stroke::Arc(cx, cy, RING, start, start + fraction * 2.0 * PI);
        frame.stroke(&[arc], RING_STROKE, color, clip);
    }
}

/// 绘制按 `turn`（圈数，可为任意实数）顺时针旋转后的「刷新」图标，表示操作正在进行。
pub fn refresh_turned(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    turn: f32,
    color: Color,
    clip: Rect,
) {
    let r = RING;
    // 起点映射到 -π 至 π 之间：渲染圆弧时按起止角的 `atan2` 结果加或减一圈来确定角度范围，
    // 起点落在该区间才能保证终点不超过 3π；终点超过 3π 时圆弧末段不绘制。
    let mut angle = turn.rem_euclid(1.0) * 2.0 * PI;
    if angle >= PI {
        angle -= 2.0 * PI;
    }
    let (sin, cos) = angle.sin_cos();
    let at = |x: f32, y: f32| (cx + x * cos - y * sin, cy + x * sin + y * cos);
    // 屏幕坐标的 `y` 轴向下，角度增大即顺时针方向：圆弧从 3 点钟转到 1 点半。
    let (ex, ey) = at(r * FRAC_1_SQRT_2, -r * FRAC_1_SQRT_2);
    // 箭头尖位于 3 点钟方向稍上方，圆弧末端沿切线方向斜向连接箭头尖，两臂分别朝上和朝左。
    let (tip_x, tip_y) = (r, -r * 0.45);
    let arm = 3.0;
    let (tx, ty) = at(tip_x, tip_y);
    let (ux, uy) = at(tip_x, tip_y - arm);
    let (lx, ly) = at(tip_x - arm, tip_y);
    let shapes = [
        Stroke::Arc(cx, cy, r, angle, angle + 1.75 * PI),
        Stroke::Line(ex, ey, tx, ty),
        Stroke::Line(tx, ty, ux, uy),
        Stroke::Line(tx, ty, lx, ly),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「同步」图标，约 13×13：同一个圆上相对的两段圆弧，各自顺时针画到直角箭头为止。
///
/// 用于与远程交换提交的操作；只重读本地状态的操作用 [`refresh`]。
pub fn sync(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    sync_turned(frame, center, 0.0, color, clip);
}

/// 绘制按 `turn`（圈数，可为任意实数）顺时针旋转后的「同步」图标，表示同步正在进行。
pub fn sync_turned(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    turn: f32,
    color: Color,
    clip: Rect,
) {
    let r = RING;
    // 两段圆弧之间的空隙（弧度）。取 0.5：箭头落在空隙里，不碰到另一段圆弧。
    let gap = 0.5;
    // 箭头臂长（逻辑像素），与「刷新」图标相同。
    let arm = 3.0;
    let base = turn.rem_euclid(1.0) * 2.0 * PI;
    // 从 `start` 起顺时针画半圈减去两个空隙的圆弧，末端加箭头。
    let half = |start: f32| {
        // 起点映射到 -π 至 π 之间，理由同 `refresh_turned`。
        let mut from = start + base;
        while from >= PI {
            from -= 2.0 * PI;
        }
        let to = from + PI - 2.0 * gap;
        // 箭头两臂与切线各成 45°，分在圆弧内外两侧。
        let (sin, cos) = to.sin_cos();
        let (tx, ty) = (cx + r * cos, cy + r * sin);
        let k = FRAC_1_SQRT_2 * arm;
        let (bx, by) = (sin, -cos);
        [
            Stroke::Arc(cx, cy, r, from, to),
            Stroke::Line(tx, ty, tx + (bx - cos) * k, ty + (by - sin) * k),
            Stroke::Line(tx, ty, tx + (bx + cos) * k, ty + (by + sin) * k),
        ]
    };
    // 上半段从 9 点钟附近开始，下半段从 3 点钟附近开始。
    let [a, b, c] = half(-PI + gap);
    let [d, e, f] = half(gap);
    let shapes = [a, b, c, d, e, f];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「全部折叠」图标，11×12：圆角卡片内一道横线，卡片上方露出后面一张较窄卡片的上沿，表示
/// 卡片叠放。
pub fn collapse_all(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let card = rounded_outline(cx - 5.5, cy - 3.0, 11.0, 9.0, 2.0);
    let (back, mid) = (cy - 6.0, cy + 1.5);
    let marks = [
        Stroke::Line(cx - 3.5, back, cx + 3.5, back),
        Stroke::Line(cx - 2.5, mid, cx + 2.5, mid),
    ];
    frame.stroke(&card, RING_STROKE, color, clip);
    frame.stroke(&marks, RING_STROKE, color, clip);
}

/// 绘制「定位当前文件」图标，13×13：一个圆加四条由圆周向内伸出的刻度，中心留空。
pub fn locate(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (r, inner) = (RING, RING - 3.0);
    let shapes = [
        Stroke::Circle(cx, cy, r),
        Stroke::Line(cx, cy - r, cx, cy - inner),
        Stroke::Line(cx, cy + r, cx, cy + inner),
        Stroke::Line(cx - r, cy, cx - inner, cy),
        Stroke::Line(cx + r, cy, cx + inner, cy),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「以列表显示」图标，约 12×10：左侧三个圆点，右侧三道等长的横线，表示平铺的各行。
pub fn list_view(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let rows = [cy - 4.0, cy, cy + 4.0];
    // 零长度的线段描边后呈圆点。
    let dots = rows.map(|y| Stroke::Line(cx - 5.0, y, cx - 5.0, y));
    let lines = rows.map(|y| Stroke::Line(cx - 2.0, y, cx + 6.0, y));
    frame.stroke(&dots, 2.0, color, clip);
    frame.stroke(&lines, RING_STROKE, color, clip);
}

/// 绘制「以树形显示」图标，约 12×10：顶格一道横线，其下两道缩进的横线，左侧的竖线和短横线
/// 把它们连到上一行，表示目录与其中的文件。
pub fn tree_view(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (trunk, child) = (cx - 4.0, cx - 1.0);
    let shapes = [
        Stroke::Line(cx - 6.0, cy - 4.0, cx + 6.0, cy - 4.0),
        Stroke::Line(trunk, cy - 4.0, trunk, cy + 4.0),
        Stroke::Line(trunk, cy, child, cy),
        Stroke::Line(trunk, cy + 4.0, child, cy + 4.0),
        Stroke::Line(child + 1.5, cy, cx + 6.0, cy),
        Stroke::Line(child + 1.5, cy + 4.0, cx + 6.0, cy + 4.0),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「更多操作」图标，12×2：横排三个圆点，相邻圆心间距 4.5 逻辑像素，点径 2 逻辑像素。
pub fn more(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    // 零长度的线段描边后呈圆点，直径等于笔宽。
    let dots = [-4.5, 0.0, 4.5].map(|dx| Stroke::Line(cx + dx, cy, cx + dx, cy));
    frame.stroke(&dots, 2.0, color, clip);
}

/// 绘制侧栏「文件」视图图标，约 13×14：线条风格（笔宽 `STROKE`，与顶栏其他图标一致），图形为
/// 前后叠放的两张纸。前面一张是完整的圆角框，内部两道较短的水平线表示文字行；后面一张错开在
/// 左上方，只露出上边和左边一段，两端为圆头。
///
/// 两张纸相距 2 逻辑像素，文字行短于纸边，因此该图标与笔画数同为两三笔的 `search`、`git` 并排
/// 时视觉重量相当。
///
/// `scale` 是相对原大小的缩放倍数（视图切换中的小号图标传入小于 1 的值），笔宽不变。
pub fn files(frame: &mut Frame<'_>, (cx, cy): (f32, f32), scale: f32, color: Color, clip: Rect) {
    let k = scale;
    let (x, y) = (cx - 6.25 * k, cy - 6.75 * k);
    let r = 2.0 * k;
    let (fx, fy, fw, fh) = (x + 4.0 * k, y + 3.5 * k, 8.5 * k, 10.0 * k);
    let marks = [
        // 两道长度不同的文字行。
        Stroke::Line(
            fx + 2.75 * k,
            fy + 3.75 * k,
            fx + fw - 2.75 * k,
            fy + 3.75 * k,
        ),
        Stroke::Line(fx + 2.75 * k, fy + 6.5 * k, fx + fw - 4.0 * k, fy + 6.5 * k),
        // 后面一张纸：左上角为圆角，上边延伸到前面一张纸的右半部分，左边延伸到其下半部分。
        Stroke::Arc(x + r, y + r, r, -PI, -FRAC_PI_2),
        Stroke::Line(x + r, y, fx + fw - 3.5 * k, y),
        Stroke::Line(x, y + r, x, fy + fh - 3.5 * k),
    ];
    let shapes: Strokes<13> = joined(&[&rounded_outline(fx, fy, fw, fh, r), &marks]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制放大镜图标，约 13×13：线条风格，一个圆框加右下的手柄，笔宽 `STROKE`。
///
/// 顶栏的查找按钮、空编辑区的「在文件中查找」与侧栏的「搜索」切换都使用该图标。`scale` 是相对
/// 原大小的缩放倍数，笔宽不变。
pub fn search(frame: &mut Frame<'_>, (cx, cy): (f32, f32), scale: f32, color: Color, clip: Rect) {
    let s = scale;
    let (ox, oy, r) = (cx - s, cy - s, 4.75 * s);
    let k = r * FRAC_1_SQRT_2;
    let shapes = [
        Stroke::Circle(ox, oy, r),
        Stroke::Line(ox + k + 0.5, oy + k + 0.5, cx + 5.75 * s, cy + 5.75 * s),
    ];
    frame.stroke(&shapes, STROKE, color, clip);
}

/// 绘制 Git 图标，约 14×14：线条风格，图形由 `branch_shape` 给出（一条主干、两个提交、一道
/// 四分之一圆弧），笔宽 `STROKE`，提交圆环内不铺底色，与 `files`、`search` 等顶栏图标风格
/// 一致。
///
/// `scale` 是相对原大小的缩放倍数，笔宽不变。
pub fn git(frame: &mut Frame<'_>, center: (f32, f32), scale: f32, color: Color, clip: Rect) {
    branch_shape(frame, center, (0.7 * scale, STROKE), color, None, clip);
}

/// 分支图形：在 24×24 的设计稿坐标系上绘制，按 `unit` 缩放（实际尺寸约 18·`unit` 逻辑像素），
/// 中心在 `(cx, cy)`，笔宽 `width`。
///
/// 左侧一条主干自上而下，下端为一个提交（圆环）；右上方为另一个提交，由它的下沿起一道四分之一
/// 圆弧向下弯折，收在主干下端提交的右沿，表示分出的分支重新合并。`fill` 为圆环内铺的底色，
/// 为 `None` 时环内不铺底色。
fn branch_shape(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    (unit, width): (f32, f32),
    color: Color,
    fill: Option<Color>,
    clip: Rect,
) {
    let at = |x: f32, y: f32| (cx + (x - 12.0) * unit, cy + (y - 12.0) * unit);
    let r = 3.0 * unit;
    let tip = at(18.0, 6.0);
    let base = at(6.0, 18.0);
    let (trunk_x, trunk_top) = at(6.0, 3.0);
    let bend = at(9.0, 9.0);
    if let Some(fill) = fill {
        let inner = r - width / 2.0;
        for (x, y) in [tip, base] {
            let disk = Rect::new(x - inner, y - inner, 2.0 * inner, 2.0 * inner);
            frame.rect_clipped(disk, fill, inner, clip);
        }
    }
    let shapes = [
        Stroke::Line(trunk_x, trunk_top, trunk_x, base.1 - r),
        Stroke::Circle(tip.0, tip.1, r),
        Stroke::Circle(base.0, base.1, r),
        Stroke::Arc(bend.0, bend.1, 9.0 * unit, 0.0, FRAC_PI_2),
    ];
    frame.stroke(&shapes, width, color, clip);
}

/// 绘制「回滚」图标，12×10：一道折返后指向左侧的箭头，表示撤回改动。
pub fn rollback(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let lines = [
        // 箭杆：自右下向上，再折向左侧。
        Stroke::Line(cx + 5.0, cy + 5.0, cx + 5.0, cy),
        Stroke::Line(cx + 5.0, cy, cx + 4.0, cy - 1.5),
        Stroke::Line(cx + 4.0, cy - 1.5, cx + 2.5, cy - 2.0),
        Stroke::Line(cx + 2.5, cy - 2.0, cx - 5.0, cy - 2.0),
        // 箭头。
        Stroke::Line(cx - 5.0, cy - 2.0, cx - 2.0, cy - 5.0),
        Stroke::Line(cx - 5.0, cy - 2.0, cx - 2.0, cy + 1.0),
    ];
    frame.stroke(&lines, STROKE, color, clip);
}

/// 绘制对勾图标，约 `size` 宽：短的一笔向右下，长的一笔向右上，两笔比例与勾选框内的勾一致。
pub fn check(frame: &mut Frame<'_>, (cx, cy): (f32, f32), size: f32, color: Color, clip: Rect) {
    let h = size / 2.0;
    let lines = [
        Stroke::Line(cx - h, cy, cx - h * 0.3, cy + h * 0.65),
        Stroke::Line(cx - h * 0.3, cy + h * 0.65, cx + h, cy - h * 0.7),
    ];
    frame.stroke(&lines, STROKE, color, clip);
}

/// 绘制「历史」图标，约 13×13：与 `refresh` 相同的圆、笔宽和箭头，方向相反（逆时针），圆内
/// 增加时针与分针，表示查看提交记录。
///
/// 视觉重量与标题行的 `refresh`、`collapse_all` 等按钮一致。
pub fn history(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let r = RING;
    // 圆弧自 10 点半顺时针画到 9 点钟：缺口位于左上，箭头尖位于 9 点钟方向稍上方，两臂分别
    // 朝上和朝右。
    let (ex, ey) = (cx - r * FRAC_1_SQRT_2, cy - r * FRAC_1_SQRT_2);
    let (tx, ty) = (cx - r, cy - r * 0.45);
    let arm = 3.0;
    let shapes = [
        Stroke::Arc(cx, cy, r, -0.75 * PI, PI),
        Stroke::Line(ex, ey, tx, ty),
        Stroke::Line(tx, ty, tx, ty - arm),
        Stroke::Line(tx, ty, tx + arm, ty),
        // 指针：时针朝上，分针朝右下。
        Stroke::Line(cx, cy - r * 0.55, cx, cy),
        Stroke::Line(cx, cy, cx + r * 0.45, cy + r * 0.3),
    ];
    frame.stroke(&shapes, RING_STROKE, color, clip);
}

/// 绘制「并排比较」图标，约 12×11：一个圆角框，中央一道竖线分成左右两栏，两栏内各一道短横。
pub fn compare(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (l, r, t, b) = (cx - 6.0, cx + 6.0, cy - 5.5, cy + 5.5);
    let marks = [
        Stroke::Line(cx, t, cx, b),
        Stroke::Line(l + 2.5, cy, cx - 2.5, cy),
        Stroke::Line(cx + 2.5, cy, r - 2.5, cy),
    ];
    let lines: Strokes<11> = joined(&[&rounded_outline(l, t, 12.0, 11.0, 1.5), &marks]);
    frame.stroke(lines.as_slice(), STROKE, color, clip);
}

/// 绘制小号分支图标，约 10×10：图形与 `git` 相同（`branch_shape`），线条更细，圆环内不铺
/// 底色，用于状态栏与侧栏分支名旁的小号文字。
pub fn branch_small(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    branch_shape(frame, center, (0.56, 1.25), color, None, clip);
}

/// 绘制项目徽章右下角的工作树记号，约 9×9：一个底色为 `fill` 的圆，外圈为 `ring`（与徽章底色
/// 隔开），内部为缩小的分支图形（`branch_shape`）。
pub fn worktree_mark(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    (ring, fill, color): (Color, Color, Color),
    clip: Rect,
) {
    for (r, c) in [(6.0, ring), (4.75, fill)] {
        frame.rect_clipped(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), c, r, clip);
    }
    branch_shape(frame, (cx, cy), (0.27, 1.0), color, None, clip);
}

/// 绘制侧栏开关图标，16×14：线条风格，一个圆角窗框，左侧一道短竖线表示侧栏。
///
/// `open` 为 `true`（侧栏显示）时竖线自上而下贯通，把窗框分成左右两栏；为 `false` 时竖线只剩中
/// 间一段。
///
/// 高度取 14 逻辑像素，与窗口按钮、`files`、`search` 图标的高度一致；该值为偶数，坐标取整后
/// 正好以 `cy` 为中心。
pub fn sidebar(frame: &mut Frame<'_>, (cx, cy): (f32, f32), open: bool, color: Color, clip: Rect) {
    let (x, y) = (
        (cx - SIDEBAR_WIDTH / 2.0).round() + 0.75,
        (cy - 7.0).round() + 1.0,
    );
    let (w, h) = (SIDEBAR_WIDTH - 1.5, 12.0);
    let bar = x + 5.0;
    let bar = if open {
        Stroke::Line(bar, y, bar, y + h)
    } else {
        Stroke::Line(bar - 1.0, y + 3.5, bar - 1.0, y + h - 3.5)
    };
    let shapes: Strokes<9> = joined(&[&rounded_outline(x, y, w, h, 3.0), &[bar]]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制终端面板位置按钮的图标：与侧栏开关尺寸相同的圆角框；`right` 为 `true` 时在右侧隔出一
/// 栏，否则在下方隔出一条。
///
/// 调用方传入的是切换后的位置，图标表示按下后终端面板将放置的位置。
pub fn panel(frame: &mut Frame<'_>, (cx, cy): (f32, f32), right: bool, color: Color, clip: Rect) {
    let (x, y) = (
        (cx - SIDEBAR_WIDTH / 2.0).round() + 0.75,
        (cy - 7.0).round() + 1.0,
    );
    let (w, h) = (SIDEBAR_WIDTH - 1.5, 12.0);
    let bar = if right {
        Stroke::Line(x + w - 5.0, y, x + w - 5.0, y + h)
    } else {
        Stroke::Line(x, y + h - 4.0, x + w, y + h - 4.0)
    };
    let shapes: Strokes<9> = joined(&[&rounded_outline(x, y, w, h, 3.0), &[bar]]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制「分栏」图标：与侧栏开关尺寸相同的圆角框，中央一道竖线分成左右两半（左侧为代码，右侧
/// 为预览）。
pub fn split(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (x, y) = (
        (cx - SIDEBAR_WIDTH / 2.0).round() + 0.75,
        (cy - 7.0).round() + 1.0,
    );
    let (w, h) = (SIDEBAR_WIDTH - 1.5, 12.0);
    let mid = (x + w / 2.0).round() - 0.25;
    let shapes: Strokes<9> = joined(&[
        &rounded_outline(x, y, w, h, 3.0),
        &[Stroke::Line(mid, y, mid, y + h)],
    ]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制「单栏」图标：与 [`split`] 尺寸相同的圆角框，中间一道横线分成上下两半（上面是改之前，
/// 下面是改之后）。
pub fn single_column(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (x, y) = (
        (cx - SIDEBAR_WIDTH / 2.0).round() + 0.75,
        (cy - 7.0).round() + 1.0,
    );
    let (w, h) = (SIDEBAR_WIDTH - 1.5, 12.0);
    let mid = (y + h / 2.0).round() - 0.25;
    let shapes: Strokes<9> = joined(&[
        &rounded_outline(x, y, w, h, 3.0),
        &[Stroke::Line(x, mid, x + w, mid)],
    ]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 按缩放倍数 `k` 绘制 `split` 图标，用于出现与消失的动画。
///
/// 线宽不变，坐标不取整；`k` 不小于 1 时直接调用 `split` 绘制原尺寸图标。
pub fn split_scaled(frame: &mut Frame<'_>, (cx, cy): (f32, f32), k: f32, color: Color, clip: Rect) {
    if k >= 1.0 {
        return split(frame, (cx, cy), color, clip);
    }
    let (w, h) = ((SIDEBAR_WIDTH - 1.5) * k, 12.0 * k);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let shapes: Strokes<9> = joined(&[
        &rounded_outline(x, y, w, h, 3.0 * k),
        &[Stroke::Line(cx, y, cx, y + h)],
    ]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制终端图标：与侧栏开关尺寸相同的圆角框，内部为提示符 `>_`。
pub fn terminal(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (x, y) = (
        (cx - SIDEBAR_WIDTH / 2.0).round() + 0.75,
        (cy - 7.0).round() + 1.0,
    );
    let (w, h) = (SIDEBAR_WIDTH - 1.5, 12.0);
    let prompt = [
        Stroke::Line(x + 3.5, y + 3.5, x + 6.0, y + 6.0),
        Stroke::Line(x + 6.0, y + 6.0, x + 3.5, y + 8.5),
        Stroke::Line(x + 7.5, y + 8.5, x + 11.0, y + 8.5),
    ];
    let shapes: Strokes<11> = joined(&[&rounded_outline(x, y, w, h, 3.0), &prompt]);
    frame.stroke(shapes.as_slice(), STROKE, color, clip);
}

/// 绘制书签图标，7×11：实心竖条，下端中央有一个宽 2、高 3 逻辑像素的缺口。
///
/// 绘制在行号栏左侧，标记设置了书签的行。
pub fn bookmark(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (w, h, notch) = (7.0, 11.0, 3.0);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let tail = (w - 2.0) / 2.0;
    frame.rect_corners(
        Rect::new(x, y, w, h - notch),
        color,
        [1.5, 1.5, 0.0, 0.0],
        clip,
    );
    frame.rect_clipped(Rect::new(x, y + h - notch, tail, notch), color, 0.0, clip);
    frame.rect_clipped(
        Rect::new(x + w - tail, y + h - notch, tail, notch),
        color,
        0.0,
        clip,
    );
}

/// 绘制「全屏」图标，约 12×12 逻辑像素：从中心分别指向右上角和左下角的两个箭头。
pub fn expand(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (near, far, head) = (1.5, 5.0, 3.0);
    let arrow = |s: f32| {
        let (tx, ty) = (cx + s * far, cy - s * far);
        [
            Stroke::Line(cx + s * near, cy - s * near, tx, ty),
            Stroke::Line(tx - s * head, ty, tx, ty),
            Stroke::Line(tx, ty, tx, ty + s * head),
        ]
    };
    frame.stroke(&arrow(1.0), STROKE, color, clip);
    frame.stroke(&arrow(-1.0), STROKE, color, clip);
}

/// 绘制「复制」图标，约 12×12：两张错开叠放的圆角方块，前面一张完整，后面一张只露出左上角。
pub fn copy(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (size, r, shift) = (8.0, 1.5, 3.5);
    let (x, y) = (cx - size / 2.0 + shift / 2.0, cy - size / 2.0 + shift / 2.0);
    frame.stroke(&rounded_outline(x, y, size, size, r), STROKE, color, clip);
    let (bx, by) = (x - shift, y - shift);
    let back = [
        Stroke::Line(bx, y, bx, by + r),
        Stroke::Arc(bx + r, by + r, r, -PI, -FRAC_PI_2),
        Stroke::Line(bx + r, by, x, by),
    ];
    frame.stroke(&back, STROKE, color, clip);
}
