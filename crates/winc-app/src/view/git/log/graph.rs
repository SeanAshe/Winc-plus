//! 提交图：每行的点和连线，连线可以点的范围。
//!
//! 车道位置、车道间距、线的粗细来自 `log` 模块的常量；颜色按车道序号从调色板里取，与徽章
//! 同一组色。整行由 [`draw_graph_row`] 画出，可点击的范围一并写进 `hits`；跨很多行直着穿过的
//! 长连线只画两头（见 [`long_cuts`]）。

use super::*;

/// 直着穿过至少这么多行的一段线算长连线，中间不画（行）。
///
/// 约为一屏的行数：短于一屏的线两头同时看得见，整段照画；更长的线把提交图分得很碎，又很少
/// 需要逐行看清。
const LONG_EDGE: usize = 30;
/// 长连线两头各画几行（行）：连着提交的那一截看得出线往哪去，末端画箭头。
const LONG_STUB: usize = 2;

/// 一行里一条车道上的长连线怎么画（见 [`long_cuts`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cut {
    /// 在中间：不画。
    Hidden,
    /// 上面那一截的最后一行：下半行画成朝下的箭头。
    Down,
    /// 下面那一截的第一行：上半行画成朝上的箭头。
    Up,
}

/// 第 `row` 行里属于长连线、要截掉或画箭头的车道。
///
/// 一条线在某条车道上连续直着穿过的几行（不经过圆点、不换车道）是一段；一段至少
/// [`LONG_EDGE`] 行时只画两头各 [`LONG_STUB`] 行。没取完的日志里，到已取回的最后一行仍在
/// 穿过的当作还要继续往下。`shown` 所在的线（选中的提交那条）照常画全。只往上下各查
/// `LONG_EDGE` 行，每帧的开销与可见的行数成正比。
pub(super) fn long_cuts(view: &LogView, row: usize, shown: Option<u32>) -> Vec<(usize, Cut)> {
    let Some(graph) = view.graph.get(row) else {
        return Vec::new();
    };
    // 第 `r` 行在车道 `lane` 上有没有直着穿过的线 `line`。
    let through = |r: usize, lane: usize, line: u32| {
        view.graph.get(r).is_some_and(|g| {
            let straight =
                |s: &winc_core::graph::Segment| s.from == lane && s.to == lane && s.line == line;
            g.lane != lane && g.up.iter().any(straight) && g.down.iter().any(straight)
        })
    };
    let mut cuts = Vec::new();
    for s in graph
        .up
        .iter()
        .filter(|s| s.from == s.to && s.to != graph.lane)
    {
        if shown == Some(s.line) || !through(row, s.to, s.line) {
            continue;
        }
        let (lane, line) = (s.to, s.line);
        let above = (1..=LONG_EDGE)
            .take_while(|&d| row >= d && through(row - d, lane, line))
            .count();
        let open = !view.complete;
        let below = (1..=LONG_EDGE)
            .take_while(|&d| {
                let r = row + d;
                through(r, lane, line) || (open && r >= view.graph.len())
            })
            .count();
        if above + below + 1 < LONG_EDGE {
            continue;
        }
        let cut = match (above, below) {
            (a, _) if a == LONG_STUB - 1 => Cut::Down,
            (_, b) if b == LONG_STUB - 1 => Cut::Up,
            (a, b) if a >= LONG_STUB && b >= LONG_STUB => Cut::Hidden,
            _ => continue,
        };
        cuts.push((lane, cut));
    }
    cuts
}

/// 一行的提交图：上半行、下半行的线，再在中间画圆点（普通提交实心、合并提交小一圈、`HEAD` 是圈）。
///
/// 直着穿过的是竖线，换车道的线在圆点那一高度横着走，到那条车道时用半径 `CORNER` 圆角拐下去。
/// `line` 是选中提交所在的线，其余线的不透明度降到 `GRAPH_FADED`：淡色事先与行底色混好，否则
/// 半透明的线每行各画一截，圆头会在行的交界处叠出深色。
///
/// 坐标都是逻辑像素。圆点和线都可以点击，命中范围进 `hits` 时排在整行前面。
pub(super) fn draw_graph_row(
    app: &App,
    frame: &mut Frame<'_>,
    (graph, line, cuts): (&GraphRow, Option<u32>, &[(usize, Cut)]),
    (commit, row): (&Commit, usize),
    (x0, y, band): (f32, f32, Option<Color>),
    clip: Rect,
    hits: &mut Vec<(Rect, LogTarget)>,
) {
    let p = app.palette;
    let lh = app.fonts.line_height;
    let lane_x = |lane: usize| x0 + lane.min(MAX_LANES - 1) as f32 * LANE + LANE / 2.0;
    let faded = |l: u32| line.is_some_and(|f| f != l);
    let row_bg = band.map_or(p.editor_bg, |band| {
        crate::view::workspace::panel::over(p.editor_bg, band)
    });
    let color = |c: u8, l: u32| {
        let ink = lane_color(p, c);
        if faded(l) {
            crate::view::workspace::panel::over(row_bg, ink.alpha(ink.a * GRAPH_FADED))
        } else {
            ink
        }
    };
    let mid = y + lh / 2.0;
    // 两头都在最右边那条车道以外的线挤在一起看不出来，不画：车道很多的仓库每行少画几十笔。
    let beyond = |s: &&winc_core::graph::Segment| s.from >= MAX_LANES && s.to >= MAX_LANES;
    let halves = [(&graph.up, y, mid), (&graph.down, mid, y + lh)];
    let c = (lane_x(graph.lane), mid);
    let row_rect = Rect::new(clip.x, y, clip.w, lh).intersect(clip);
    // 圆点的命中范围（逻辑像素，经验值）：比画出来的圆点大一圈，容易点中。
    let grab = Rect::new(c.0 - 6.0, c.1 - 6.0, 12.0, 12.0).intersect(row_rect);
    hits.push((grab, LogTarget::Row(row)));
    let mut line_hits = Vec::new();
    for pass_faded in [true, false] {
        for (segments, top, bottom) in halves {
            let this_pass = |s: &&winc_core::graph::Segment| faded(s.line) == pass_faded;
            for s in segments.iter().filter(|s| !beyond(s)).filter(this_pass) {
                // 长连线：中间不画，两头的末端画箭头（箭头这半行的线短一截，不碰到箭尖）。
                let cut = (s.from == s.to)
                    .then(|| cuts.iter().find(|(lane, _)| *lane == s.to).map(|c| c.1))
                    .flatten();
                let upper = top == y;
                let (top, bottom) = match cut {
                    Some(Cut::Hidden) => continue,
                    Some(Cut::Down) if !upper => {
                        let tip = (lane_x(s.to), bottom - 2.0);
                        arrow(frame, tip, true, color(s.color, s.line), clip);
                        (top, bottom - 4.0)
                    }
                    Some(Cut::Up) if upper => {
                        let tip = (lane_x(s.to), top + 2.0);
                        arrow(frame, tip, false, color(s.color, s.line), clip);
                        (top + 4.0, bottom)
                    }
                    _ => (top, bottom),
                };
                let edge = lane_edge((lane_x(s.from), top), (lane_x(s.to), bottom), upper);
                frame.stroke(&edge, GRAPH_LINE, color(s.color, s.line), clip);
                for rect in edge_hit_rects(&edge) {
                    line_hits.push((rect.intersect(row_rect), LogTarget::Line(row, s.line)));
                }
            }
        }
    }
    // 后画的（高亮的）压在上面，先进命中表。
    hits.extend(
        line_hits
            .into_iter()
            .rev()
            .filter(|(r, _)| r.w > 0.0 && r.h > 0.0),
    );
    let head = commit
        .refs
        .iter()
        .any(|r| matches!(r, Ref::Head(_) | Ref::Detached));
    let dot = |frame: &mut Frame<'_>, r: f32, fill| {
        frame.rect_clipped(Rect::new(c.0 - r, c.1 - r, 2.0 * r, 2.0 * r), fill, r, clip);
    };
    let ink = color(graph.color, graph.line);
    // 圆点的半径（逻辑像素，经验值）：`HEAD` 最大，合并提交小一圈，普通提交居中。
    let r = if head {
        4.0
    } else if commit.parents.len() > 1 {
        2.75
    } else {
        3.25
    };
    dot(frame, r, ink);
    // `HEAD` 是空心圆，圆环与线条同样粗细。
    if head {
        let hole = r - GRAPH_LINE;
        dot(frame, hole, p.editor_bg);
        if let Some(band) = band {
            dot(frame, hole, band);
        }
    }
}

/// 在 `tip` 画长连线末端的箭头：朝下（`down`）或朝上，两笔与线同样粗细。
fn arrow(frame: &mut Frame<'_>, tip: (f32, f32), down: bool, color: Color, clip: Rect) {
    // 箭头两翼的横、竖跨度（逻辑像素，经验值）：比车道间距小，不碰到相邻车道。
    let (w, h) = (3.5, 3.5);
    let back = if down { tip.1 - h } else { tip.1 + h };
    let wings = [
        Stroke::Line(tip.0 - w, back, tip.0, tip.1),
        Stroke::Line(tip.0 + w, back, tip.0, tip.1),
    ];
    frame.stroke(&wings, GRAPH_LINE, color, clip);
}

/// 从 `a` 连到 `b` 的一段提交图的线（半行高）。
///
/// 同一条车道是一条竖线；换车道的那段一头连着圆点（行的中间），在圆点那一高度横着走，到
/// 另一条车道时用半径 `CORNER` 的圆角拐成竖的：`into_dot` 为 `true` 是上半行合进圆点（先竖
/// 着下来、拐弯、横着进圆点），否则是下半行从圆点分出去（先横着出去、拐弯、竖着下去）。
pub(super) fn lane_edge(a: (f32, f32), b: (f32, f32), into_dot: bool) -> Vec<Stroke> {
    let dx = b.0 - a.0;
    if dx.abs() < 0.5 {
        return vec![Stroke::Line(a.0, a.1, b.0, b.1)];
    }
    let r = CORNER.min(dx.abs()).min((b.1 - a.1).abs());
    let side = dx.signum();
    // 拐角的圆心和圆弧两头的角度（屏幕坐标 y 向下）。
    let (corner, from, to) = if into_dot {
        // 竖着下来到 (a.0, b.1 - r)，拐到 (a.0 + side·r, b.1)。
        let c = (a.0 + side * r, b.1 - r);
        let start = if side > 0.0 { PI } else { 0.0 };
        (c, start, FRAC_PI_2)
    } else {
        // 横着出去到 (b.0 - side·r, a.1)，拐到 (b.0, a.1 + r)。
        let c = (b.0 - side * r, a.1 + r);
        let end = if side > 0.0 { 0.0 } else { -PI };
        (c, -FRAC_PI_2, end)
    };
    let at = |t: f32| {
        let angle = from + (to - from) * t;
        (corner.0 + r * angle.cos(), corner.1 + r * angle.sin())
    };
    let first = at(0.0);
    let last = at(1.0);
    let mut out = vec![Stroke::Line(a.0, a.1, first.0, first.1)];
    out.extend((0..CORNER_STEPS).map(|i| {
        let (x0, y0) = at(i as f32 / CORNER_STEPS as f32);
        let (x1, y1) = at((i + 1) as f32 / CORNER_STEPS as f32);
        Stroke::Line(x0, y0, x1, y1)
    }));
    out.push(Stroke::Line(last.0, last.1, b.0, b.1));
    out
}

/// 返回一段线可以点的范围：线两边各放宽 `SLACK` 逻辑像素。
///
/// 竖线、横线各一个框，拐角的几截合成一个框，一段线最多三个框。
fn edge_hit_rects(edge: &[Stroke]) -> Vec<Rect> {
    // 命中范围比线本身宽出的量（逻辑像素，经验值）：线只有 `GRAPH_LINE`（1.5 逻辑像素）宽，
    // 放宽后才能点中，可点的总宽度约 8 逻辑像素。
    const SLACK: f32 = 3.25;
    let bounds = |pieces: &[Stroke]| {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for piece in pieces {
            if let Stroke::Line(a, b, c, d) = *piece {
                (x0, y0, x1, y1) = (
                    x0.min(a).min(c),
                    y0.min(b).min(d),
                    x1.max(a).max(c),
                    y1.max(b).max(d),
                );
            }
        }
        Rect::new(
            x0 - SLACK,
            y0 - SLACK,
            x1 - x0 + 2.0 * SLACK,
            y1 - y0 + 2.0 * SLACK,
        )
    };
    match edge {
        [] => Vec::new(),
        [only] => vec![bounds(std::slice::from_ref(only))],
        [first, corner @ .., last] => {
            let mut out = vec![bounds(std::slice::from_ref(first))];
            if !corner.is_empty() {
                out.push(bounds(corner));
            }
            out.push(bounds(std::slice::from_ref(last)));
            out
        }
    }
}
