//! 思维导图：根节点居中，第一层分支按子树高度分到左右两侧，各子树向外逐层展开。
//!
//! 每个第一层分支取一种颜色（见 [`super::PALETTE`]），分支内的连线与节点按该颜色着色。坐标和
//! 尺寸都是逻辑像素，`s` 是排版用的字号。

use winc_preview::markdown::mermaid::{MindShape, Mindmap, Shape};
use winc_render::{Color, Rect, Stroke};

use super::super::math::polyline;
use super::super::{Cx, Item};
use super::{Label, Look, bounds, draw_shape, mix, palette};

/// 相邻两层节点之间的水平距离与字号之比。
const LEVEL_GAP: f32 = 2.6;

/// 排好的节点：文字、大小、所属分支、在根节点的哪一侧。
struct Spot {
    label: Label,
    size: (f32, f32),
    /// 第一层分支的序号，决定颜色；根节点为 `None`。
    branch: Option<usize>,
    /// 是否在根节点左侧。
    left: bool,
    /// 左上角。
    at: (f32, f32),
}

impl Cx<'_, '_> {
    /// 排思维导图，返回宽高。
    pub(super) fn mindmap(
        &mut self,
        m: &Mindmap,
        s: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) -> (f32, f32) {
        let n = m.nodes.len();
        let mut children = vec![Vec::new(); n];
        let mut depth = vec![0usize; n];
        for (i, node) in m.nodes.iter().enumerate() {
            if let Some(p) = node.parent {
                children[p].push(i);
                depth[i] = depth[p] + 1;
            }
        }
        let mut spots: Vec<Spot> = m
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let (size, bold) = match depth[i] {
                    0 => (s * 1.15, true),
                    1 => (s, false),
                    _ => (s * 0.9, false),
                };
                let label = self.styled_label(&node.label, size, look.text, s * 14.0, bold);
                let size = box_size(shape_at(node.shape, depth[i]), label.w, label.h, s);
                Spot {
                    label,
                    size,
                    branch: None,
                    left: false,
                    at: (0.0, 0.0),
                }
            })
            .collect();
        // 父节点总在子节点之前，按顺序即可把分支与左右传下去。
        for (k, &c) in children[0].iter().enumerate() {
            spots[c].branch = Some(k);
        }
        for i in 1..n {
            if let Some(p) = m.nodes[i].parent.filter(|&p| p != 0) {
                spots[i].branch = spots[p].branch;
            }
        }
        let gap = |d: usize| if d <= 1 { s * 1.0 } else { s * 0.5 };
        // 各子树的高度，从后往前算：子节点总在父节点之后。
        let mut extent = vec![0f32; n];
        for i in (0..n).rev() {
            let kids = &children[i];
            let stacked = kids.iter().map(|&c| extent[c]).sum::<f32>()
                + gap(depth[i] + 1) * kids.len().saturating_sub(1) as f32;
            extent[i] = spots[i].size.1.max(stacked);
        }
        // 第一层分支按顺序分到当前较矮的一侧，先右后左。
        let (mut right, mut left) = (Vec::new(), Vec::new());
        let (mut rh, mut lh) = (0f32, 0f32);
        for &c in &children[0] {
            if rh <= lh {
                rh += extent[c] + gap(1);
                right.push(c);
            } else {
                lh += extent[c] + gap(1);
                left.push(c);
            }
        }
        let root = spots[0].size;
        spots[0].at = (-root.0 / 2.0, -root.1 / 2.0);
        for (side, is_left) in [(&right, false), (&left, true)] {
            let total = side.iter().map(|&c| extent[c]).sum::<f32>()
                + gap(1) * side.len().saturating_sub(1) as f32;
            let mut top = -total / 2.0;
            let near = if is_left { -root.0 / 2.0 } else { root.0 / 2.0 };
            for &c in side.iter() {
                place(
                    c, near, top, is_left, s, &children, &extent, &depth, &mut spots,
                );
                top += extent[c] + gap(1);
            }
        }

        // 连线：从父节点的外侧边缘连到子节点的内侧边缘，两端水平出入。
        for i in 1..n {
            let p = m.nodes[i].parent.unwrap_or(0);
            let color = branch_color(spots[i].branch, look);
            let from = attach(
                &spots[p],
                shape_at(m.nodes[p].shape, depth[p]),
                !spots[i].left,
            );
            let to = attach(
                &spots[i],
                shape_at(m.nodes[i].shape, depth[i]),
                spots[i].left,
            );
            let width = if depth[i] == 1 { s * 0.2 } else { s * 0.12 };
            out.push(Item::path(polyline(&bend(from, to)), width, color));
        }
        for (i, spot) in spots.iter().enumerate() {
            let shape = shape_at(m.nodes[i].shape, depth[i]);
            let r = Rect::new(spot.at.0, spot.at.1, spot.size.0, spot.size.1);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let color = branch_color(spot.branch, look);
            match shape {
                None => {
                    // 没有括号的节点只写文字，下方一道分支颜色的横线，连线从横线两端接出。
                    let y = r.bottom();
                    out.push(Item::path(
                        vec![Stroke::Line(r.x, y, r.right(), y)],
                        s * 0.12,
                        color,
                    ));
                    self.put_label(&spot.label, cx, r.y + (r.h - s * 0.2) / 2.0, out);
                }
                Some(shape) => {
                    let strength = match (spot.branch, look.dark) {
                        (None, false) => 0.16,
                        (None, true) => 0.3,
                        (Some(_), false) if depth[i] == 1 => 0.18,
                        (Some(_), true) if depth[i] == 1 => 0.3,
                        (_, false) => 0.08,
                        (_, true) => 0.16,
                    };
                    let tinted = Look {
                        fill: mix(look.label_bg, color, strength),
                        border: mix(look.label_bg, color, 0.6),
                        ..look
                    };
                    draw_shape(shape, r, tinted, out);
                    self.put_label(&spot.label, cx, cy, out);
                }
            }
        }
        bounds(out, 0.0, 0.0)
    }
}

/// 把以 `i` 为根的子树放在 `near`（左侧时为右边缘，右侧时为左边缘）外侧、上边缘为 `top` 的
/// 一段里，节点在子树的高度范围内垂直居中。
#[allow(clippy::too_many_arguments)]
fn place(
    i: usize,
    near: f32,
    top: f32,
    left: bool,
    s: f32,
    children: &[Vec<usize>],
    extent: &[f32],
    depth: &[usize],
    spots: &mut [Spot],
) {
    let (w, h) = spots[i].size;
    let x = if left {
        near - s * LEVEL_GAP - w
    } else {
        near + s * LEVEL_GAP
    };
    spots[i].at = (x, top + (extent[i] - h) / 2.0);
    spots[i].left = left;
    let kids = &children[i];
    let gap = if depth[i] == 0 { s } else { s * 0.5 };
    let stacked =
        kids.iter().map(|&c| extent[c]).sum::<f32>() + gap * kids.len().saturating_sub(1) as f32;
    let mut y = top + (extent[i] - stacked) / 2.0;
    let outer = if left { x } else { x + w };
    for &c in kids {
        place(c, outer, y, left, s, children, extent, depth, spots);
        y += extent[c] + gap;
    }
}

/// 节点画成的形状：根节点和第一层没有括号时也画成圆角框，更深的层只写文字（`None`）。
fn shape_at(shape: MindShape, depth: usize) -> Option<Shape> {
    Some(match shape {
        MindShape::Plain if depth <= 1 => Shape::Round,
        MindShape::Plain => return None,
        MindShape::Rect => Shape::Rect,
        MindShape::Round | MindShape::Cloud => Shape::Round,
        MindShape::Circle => Shape::Circle,
        MindShape::Bang => Shape::Stadium,
        MindShape::Hexagon => Shape::Hexagon,
    })
}

/// 装下 `tw × th` 的文字所需的节点大小。
fn box_size(shape: Option<Shape>, tw: f32, th: f32, s: f32) -> (f32, f32) {
    let (w, h) = (tw + s * 1.6, th + s * 0.9);
    match shape {
        None => (tw + s * 0.4, th + s * 0.5),
        Some(Shape::Circle) => {
            let d = tw.max(th) + s * 1.2;
            (d, d)
        }
        Some(Shape::Stadium | Shape::Hexagon) => (w + h / 2.0, h),
        Some(_) => (w, h),
    }
}

/// 分支的颜色；根节点使用强调色。
fn branch_color(branch: Option<usize>, look: Look) -> Color {
    branch.map_or(look.title, palette)
}

/// 连线在节点上的接点：`outward` 为真时取右边缘，否则取左边缘；只写文字的节点接在横线的端点。
fn attach(spot: &Spot, shape: Option<Shape>, outward: bool) -> (f32, f32) {
    let (x, y) = spot.at;
    let (w, h) = spot.size;
    let ex = if outward { x + w } else { x };
    match shape {
        None => (ex, y + h),
        Some(_) => (ex, y + h / 2.0),
    }
}

/// 从 `a` 到 `b` 的平滑曲线：两端水平出入的三次贝塞尔曲线，取 16 个点。
fn bend(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let mid = (a.0 + b.0) / 2.0;
    let (c1, c2) = ((mid, a.1), (mid, b.1));
    (0..=16)
        .map(|k| {
            let t = k as f32 / 16.0;
            let u = 1.0 - t;
            let (p, q, r, v) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (
                p * a.0 + q * c1.0 + r * c2.0 + v * b.0,
                p * a.1 + q * c1.1 + r * c2.1 + v * b.1,
            )
        })
        .collect()
}
