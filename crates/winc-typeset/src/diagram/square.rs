//! 流程图、类图等分层布局的图中连线的直角走线：把布局给出的各点连成只有横线和竖线的折线。
//!
//! 坐标在布局的方向里计算（见 [`Plan`]），最后按图的方向换成屏幕坐标。

use winc_preview::markdown::mermaid::{Dir, Flow, Shape};

use super::Point;
use super::rank::{Plan, Route};

/// 相邻两条轨道的间距，以字号为单位。取自调参：0.6 倍字号（字号 14 时约 8 逻辑像素）时并排的
/// 线仍能分辨，更小时看起来连成一片。
const PITCH: f32 = 0.6;
/// 轨道与上下两层之间留出的直线段的长度，以字号为单位：不短于箭头的长度。
const LEAD: f32 = 0.6;
/// 接点排开时留出的直线段的长度，以字号为单位：线端的记号比箭头长，取自调参，1.6 倍字号时
/// 类图和 ER 图的记号都画在直线上。
const LEAD_SPREAD: f32 = 1.6;
/// 拐角的圆角半径，以字号为单位。取自调参：再大时相邻轨道上的拐角互相接触。
const RADIUS: f32 = 0.35;
/// 连线经过的点对齐的范围，以字号为单位：等于布局中连线与节点之间的间距，对齐后不与节点接触。
const SNAP: f32 = 0.8;
/// 接点与节点的角之间留出的距离，以字号为单位。取自调参：0.35 倍字号时接点不落在节点的圆角
/// 上，上下两个节点错开不到半个节点宽时连线仍是直线。
const CORNER: f32 = 0.35;

/// 直角连线的排法。
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Ports {
    /// 节点同一侧出发（或到达）的连线共用一个接点，先合为一条线再分开。
    Shared,
    /// 节点同一侧的连线各用一个接点，沿这一侧排开：线的两端有记号时互不遮挡。
    Spread,
}

/// 一条排成直角的连线：屏幕上经过的点和连线文字的中心。
pub(super) type Squared = (Vec<Point>, Option<Point>);

/// 把各条连线排成只有横线和竖线的折线；连到自己的线为 `None`。同时返回各层之后还需要增加的
/// 高度（逻辑像素）：横线段多到排不开时不为零，调用方据此重新布局。
///
/// 连线从节点朝向下一层的一侧出发，在两层之间的空隙里横向移到下一点所在的列。接点在这一侧
/// 可以移动：对面的点在节点的宽度之内时移到它的正上方，连线不必拐弯。同一个空隙里横向重叠的
/// 线段分到不同的轨道，起点或终点相同的线段共用轨道；拐角取小圆角。
pub(super) fn square_routes(
    plan: &Plan,
    flow: &Flow,
    sizes: &[(f32, f32)],
    s: f32,
    ports: Ports,
) -> (Vec<Option<Squared>>, Vec<f32>) {
    let vertical = matches!(flow.dir, Dir::Down | Dir::Up);
    let across = |node: usize| if vertical { sizes[node].0 } else { sizes[node].1 } / 2.0;
    let pointed = |node: usize| {
        let shape = flow.nodes[node].shape;
        let round = matches!(
            shape,
            Shape::Circle | Shape::DoubleCircle | Shape::Start | Shape::End
        );
        ports == Ports::Shared && (round || shape == Shape::Diamond)
    };
    // 形状在离中心横向 `off` 处的半高：菱形和圆形越靠边越矮。
    let reach = |node: usize, off: f32| {
        let along = if vertical {
            sizes[node].1
        } else {
            sizes[node].0
        } / 2.0;
        let t = (off.abs() / across(node).max(1.0)).min(1.0);
        match flow.nodes[node].shape {
            _ if !pointed(node) => along,
            Shape::Diamond => along * (1.0 - t),
            _ => along * (1.0 - t * t).sqrt(),
        }
    };
    // 接点离中心最远的距离：离角留出 [`CORNER`]，菱形和圆形只用中间的一半。
    let slack = |node: usize| {
        let half = across(node);
        if pointed(node) {
            half * 0.5
        } else {
            (half - s * CORNER).max(half * 0.4)
        }
    };
    let routes: Vec<Option<&Route>> = (plan.routes.iter())
        .map(|route| route.as_ref().filter(|r| r.points.len() >= 2))
        .collect();
    // 节点的一侧（`true` 为朝向下一层）上各条连线的端点：连线、是否为终点、对面一点所在的列。
    type Side = Vec<(usize, bool, f32)>;
    let mut sides: std::collections::BTreeMap<(usize, bool), Side> = Default::default();
    for (k, edge) in flow.edges.iter().enumerate() {
        let Some(route) = routes[k] else { continue };
        let (p, n) = (&route.points, route.points.len());
        let far = 2.min(n - 1);
        sides
            .entry((edge.from, p[1].1 > p[0].1))
            .or_default()
            .push((k, false, p[far].0));
        sides
            .entry((edge.to, p[n - 2].1 > p[n - 1].1))
            .or_default()
            .push((k, true, p[n - 1 - far].0));
    }
    // 先定起点的接点，再定终点的：只隔一层的连线，终点对着起点已经定下的接点。
    let mut tail = vec![0f32; flow.edges.len()];
    let mut head = vec![0f32; flow.edges.len()];
    for heads in [false, true] {
        for (&(node, _), ends) in &sides {
            let center = plan.centers[node].0;
            let (low, high) = (center - slack(node), center + slack(node));
            let mut ends: Vec<(usize, f32)> = (ends.iter())
                .filter(|end| end.1 == heads)
                .map(|&(k, _, far)| {
                    let direct = heads && routes[k].is_some_and(|r| r.points.len() == 3);
                    (k, if direct { tail[k] } else { far })
                })
                .collect();
            if ends.is_empty() {
                continue;
            }
            ends.sort_by(|a, b| a.1.total_cmp(&b.1));
            let count = ends.len();
            for (i, &(k, far)) in ends.iter().enumerate() {
                let at = match ports {
                    Ports::Spread if count > 1 => {
                        low + (high - low) * (i as f32 + 0.5) / count as f32
                    }
                    Ports::Spread => far.clamp(low, high),
                    // 共用的接点取对面各点中在宽度之内、离中心最近的一个，没有时取中心。
                    Ports::Shared => (ends.iter())
                        .map(|end| end.1)
                        .filter(|far| (low..=high).contains(far))
                        .min_by(|a, b| (a - center).abs().total_cmp(&(b - center).abs()))
                        .unwrap_or(center),
                };
                if heads { head[k] = at } else { tail[k] = at }
            }
        }
    }
    // 共用接点时，节点的一侧既有连线出发又有连线到达：到达的连线偏到来的那一边，箭头不压在
    // 出发的线上。
    if ports == Ports::Shared {
        for (&(node, _), ends) in &sides {
            let Some(&(out, ..)) = ends.iter().find(|end| !end.1) else {
                continue;
            };
            let center = plan.centers[node].0;
            for &(k, _, far) in ends.iter().filter(|end| end.1) {
                let by = slack(node).min(across(node) / 2.0);
                let at = if far < tail[out] {
                    tail[out] - by
                } else {
                    tail[out] + by
                };
                head[k] = at.clamp(center - slack(node), center + slack(node));
            }
        }
    }
    // 一条横线段：所在的连线和段、两端的列、所在的空隙（上面一层的序号）、可用的纵向范围，
    // 以及分到的轨道。
    struct Jog {
        edge: usize,
        at: usize,
        columns: (f32, f32),
        gap: usize,
        band: (f32, f32),
        track: usize,
    }
    let row_of = |along: f32| {
        let off = |r: &usize| (plan.rows[*r].center - along).abs();
        (0..plan.rows.len()).min_by(|a, b| off(a).total_cmp(&off(b)))
    };
    let mut lines: Vec<Option<Vec<Point>>> = vec![None; flow.edges.len()];
    let mut labels: Vec<Option<Point>> = vec![None; flow.edges.len()];
    let mut jogs: Vec<Jog> = Vec::new();
    for (k, edge) in flow.edges.iter().enumerate() {
        let Some(route) = routes[k] else { continue };
        let mut centers = route.points.clone();
        let n = centers.len();
        centers[0].0 = tail[k];
        centers[n - 1].0 = head[k];
        // 相邻两个偶数层的点横向相差不到 [`SNAP`] 时对齐。
        for i in (2..n - 1).step_by(2) {
            if (centers[i].0 - centers[i - 2].0).abs() < s * SNAP {
                centers[i].0 = centers[i - 2].0;
            }
        }
        // 真节点都在偶数层，奇数层只有连线经过的点：把这些点移到下一点所在的列，连线只在偶数层
        // 之后的空隙里横向移动，不逐层来回。连线文字跟随所在的点移动。
        for i in (1..n - 1).step_by(2) {
            centers[i].0 = centers[i + 1].0;
        }
        let near = |p: Point, q: Point| (p.0 - q.0).abs() < 0.01 && (p.1 - q.1).abs() < 0.01;
        labels[k] = route.label.map(|p| {
            let at = route.points.iter().position(|&q| near(p, q));
            plan.place(at.map_or(p, |i| centers[i]), flow.dir)
        });
        let mut points = centers.clone();
        let toward =
            |from: Point, to: Point, by: f32| (from.0, from.1 + by * (to.1 - from.1).signum());
        let off = |node: usize, at: f32| at - plan.centers[node].0;
        points[0] = toward(
            points[0],
            points[1],
            reach(edge.from, off(edge.from, tail[k])),
        );
        points[n - 1] = toward(
            points[n - 1],
            points[n - 2],
            reach(edge.to, off(edge.to, head[k])),
        );
        for (at, pair) in centers.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            // 相差不到 1.5 逻辑像素的不拐弯，画成一条几乎竖直的线。
            if (a.0 - b.0).abs() < 1.5 {
                continue;
            }
            let Some(gap) = row_of(a.1.min(b.1)) else {
                continue;
            };
            let (above, below) = (
                plan.rows[gap],
                plan.rows[(gap + 1).min(plan.rows.len() - 1)],
            );
            // 横线段走在两层的子图边距之间；没有这样的空隙时用两层之间的全部范围。
            let narrow = (above.bottom + above.clear.1, below.top - below.clear.0);
            let band = if narrow.1 - narrow.0 > s * 0.5 {
                narrow
            } else {
                (above.bottom, below.top)
            };
            jogs.push(Jog {
                edge: k,
                at,
                columns: (a.0, b.0),
                gap,
                band,
                track: 0,
            });
        }
        lines[k] = Some(points);
    }
    // 同一个空隙里的横线段依次取第一条不冲突的轨道：横向不重叠，或者有一端在同一列。
    let same = |a: f32, b: f32| (a - b).abs() < 0.5;
    let left = |j: &Jog| j.columns.0.min(j.columns.1);
    let right = |j: &Jog| j.columns.0.max(j.columns.1);
    let mut order: Vec<usize> = (0..jogs.len()).collect();
    order.sort_by(|&a, &b| left(&jogs[a]).total_cmp(&left(&jogs[b])));
    for (i, &j) in order.iter().enumerate() {
        let fits = |track: usize| {
            order[..i].iter().all(|&o| {
                let (a, b) = (&jogs[o], &jogs[j]);
                let apart = a.gap != b.gap
                    || a.track != track
                    || right(a) + s * 0.4 < left(b)
                    || right(b) + s * 0.4 < left(a);
                apart || same(a.columns.0, b.columns.0) || same(a.columns.1, b.columns.1)
            })
        };
        jogs[j].track = (0..).find(|&t| fits(t)).unwrap_or(0);
    }
    let mut used = vec![0usize; plan.rows.len()];
    for jog in &jogs {
        used[jog.gap] = used[jog.gap].max(jog.track + 1);
    }
    // 轨道与上下两层之间各留一段直线，相邻两条轨道相隔 [`PITCH`]；空隙不够高时报告还需要增加的
    // 高度。
    let lead = s * if ports == Ports::Spread {
        LEAD_SPREAD
    } else {
        LEAD
    };
    let pitch = s * PITCH;
    let mut room = vec![0f32; plan.rows.len()];
    for jog in &jogs {
        let needed = lead * 2.0 + (used[jog.gap] - 1) as f32 * pitch;
        room[jog.gap] = room[jog.gap].max(needed - (jog.band.1 - jog.band.0));
    }
    let mut added = vec![0usize; flow.edges.len()];
    for jog in &jogs {
        // 各轨道在留出两段直线之后的范围内均匀排开；范围不足时都在空隙的正中。
        let inner = (jog.band.1 - jog.band.0 - lead * 2.0).max(0.0);
        let middle = (jog.band.0 + jog.band.1) / 2.0;
        let slot = (jog.track as f32 + 0.5) / used[jog.gap] as f32 - 0.5;
        let along = middle + inner * slot;
        let Some(points) = lines[jog.edge].as_mut() else {
            continue;
        };
        // 每条横线段在原来的两点之间加两个拐点；同一条连线上前面的段已加入的拐点使下标后移。
        let at = jog.at + 1 + added[jog.edge];
        points.splice(at..at, [(jog.columns.0, along), (jog.columns.1, along)]);
        added[jog.edge] += 2;
    }
    let radius = s * RADIUS;
    let squared = (lines.into_iter().zip(labels))
        .map(|(line, label)| {
            let line = rounded(&line?, radius);
            let line = line.into_iter().map(|p| plan.place(p, flow.dir)).collect();
            Some((line, label))
        })
        .collect();
    (squared, room)
}

/// 把折线 `points` 的各个拐角换成半径不超过 `radius` 的圆角（以二次贝塞尔曲线近似，各取 4 点）。
fn rounded(points: &[Point], radius: f32) -> Vec<Point> {
    let mut out = vec![points[0]];
    for w in points.windows(3) {
        let (a, corner, b) = (w[0], w[1], w[2]);
        // 三点共线时不是拐角，中间一点不必保留。
        let turn = (corner.0 - a.0) * (b.1 - corner.1) - (corner.1 - a.1) * (b.0 - corner.0);
        if turn.abs() < 0.01 {
            continue;
        }
        let length = |p: Point, q: Point| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
        let r = radius
            .min(length(a, corner) / 2.0)
            .min(length(corner, b) / 2.0);
        let back = |from: Point| {
            let d = length(from, corner).max(1e-3);
            (
                corner.0 + (from.0 - corner.0) * r / d,
                corner.1 + (from.1 - corner.1) * r / d,
            )
        };
        let (start, end) = (back(a), back(b));
        for k in 0..=4 {
            let t = k as f32 / 4.0;
            let u = 1.0 - t;
            out.push((
                u * u * start.0 + 2.0 * u * t * corner.0 + t * t * end.0,
                u * u * start.1 + 2.0 * u * t * corner.1 + t * t * end.1,
            ));
        }
    }
    out.push(points[points.len() - 1]);
    out.dedup_by(|a, b| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
    out
}

#[cfg(test)]
mod tests;
