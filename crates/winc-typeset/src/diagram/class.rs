//! 类图与实体关系图：节点是分栏的方框，按流程图的分层布局排开；类图的关系线两端画 UML 标记，
//! 实体关系图的关系线两端画基数标记（鸦脚）。
//!
//! 坐标和尺寸都是逻辑像素，`s` 是排版用的字号。

use winc_preview::markdown::mermaid::{Card, Class, Edge, End, Er, Flow, Head, Line, Node, Shape};
use winc_render::{Rect, Stroke};

use super::super::math::polyline;
use super::super::{Cx, Item};
use super::{EDGE_WIDTH, Label, Look, NODE_RADIUS, bounds, closed, dashed, outline};

/// 方框内文字左右的边距与字号之比。
const PAD_X: f32 = 0.8;
/// 方框各栏上下的边距与字号之比。
const PAD_Y: f32 = 0.4;

/// 方框中一栏的若干行文字。
type Rows = Vec<Label>;

impl Cx<'_, '_> {
    /// 排类图，返回宽高。
    pub(crate) fn class(
        &mut self,
        c: &Class,
        s: f32,
        wrap: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) -> (f32, f32) {
        let max = s * 24.0 * wrap;
        let member = |cx: &mut Self, text: &String| cx.label(text, s * 0.9, look.text, max);
        let boxes: Vec<(Option<Label>, Label, Rows, Rows)> = c
            .classes
            .iter()
            .map(|k| {
                let note = k
                    .annotation
                    .as_ref()
                    .map(|a| self.label(&format!("«{a}»"), s * 0.8, look.soft_text, max));
                let name = self.styled_label(&k.label, s, look.text, max, true);
                let attrs = k.attributes.iter().map(|a| member(self, a)).collect();
                let methods = k.methods.iter().map(|m| member(self, m)).collect();
                (note, name, attrs, methods)
            })
            .collect();
        let sizes: Vec<(f32, f32)> = boxes
            .iter()
            .map(|(note, name, attrs, methods)| {
                let w = attrs
                    .iter()
                    .chain(methods)
                    .chain(note)
                    .map(|l| l.w)
                    .fold(name.w, f32::max);
                let mut h = head_height(note.as_ref(), name, s);
                if !attrs.is_empty() || !methods.is_empty() {
                    h += rows_height(attrs, s) + rows_height(methods, s);
                }
                ((w + s * PAD_X * 2.0).max(s * 6.0), h)
            })
            .collect();
        let flow = Flow {
            dir: c.dir,
            nodes: c.classes.iter().map(|_| plain_node()).collect(),
            edges: c
                .relations
                .iter()
                .map(|r| plain_edge(r.from, r.to, &r.label))
                .collect(),
            groups: c.groups.clone(),
        };
        let cards: Vec<[Label; 2]> = c
            .relations
            .iter()
            .map(|r| {
                [&r.from_card, &r.to_card].map(|t| self.label(t, s * 0.8, look.soft_text, max))
            })
            .collect();
        let (rects, w, h) = self.boxed(&flow, &sizes, s, wrap, look, out, |cx, k, points, out| {
            let r = &c.relations[k];
            let mut line = points.to_vec();
            let len = s * 0.9;
            let n = line.len();
            // 标记盖在线上：先算出线该画到哪里，标记在线之后加入。
            let mut marks = Vec::new();
            line[n - 1] = uml_end(r.to_end, line[n - 1], line[n - 2], len, look, &mut marks);
            line[0] = uml_end(r.from_end, line[0], line[1], len, look, &mut marks);
            let strokes = if r.dotted {
                dashed(&line, s * 0.3, s * 0.25)
            } else {
                polyline(&line)
            };
            out.push(Item::path(strokes, EDGE_WIDTH, look.line));
            out.append(&mut marks);
            let [from, to] = &cards[k];
            cx.near_end(from, points[0], points[1], s, out);
            cx.near_end(to, points[n - 1], points[n - 2], s, out);
        });
        for (rect, (note, name, attrs, methods)) in rects.iter().zip(&boxes) {
            let head = head_height(note.as_ref(), name, s);
            let mut cuts = Vec::new();
            if !attrs.is_empty() || !methods.is_empty() {
                cuts.push(head);
                cuts.push(head + rows_height(attrs, s));
            }
            frame(*rect, head, &cuts, look, out);
            let mut y = rect.y + s * PAD_Y;
            let cx = rect.x + rect.w / 2.0;
            if let Some(note) = note {
                self.put_label(note, cx, y + note.h / 2.0, out);
                y += note.h;
            }
            self.put_label(name, cx, y + name.h / 2.0, out);
            for (rows, top) in [(attrs, head), (methods, head + rows_height(attrs, s))] {
                let mut y = rect.y + top + s * PAD_Y;
                for row in rows {
                    self.paste(&row.piece, rect.x + s * PAD_X, y, 0, out);
                    y += row.h;
                }
            }
        }
        bounds(out, w, h)
    }

    /// 排实体关系图，返回宽高。
    pub(crate) fn er(
        &mut self,
        er: &Er,
        s: f32,
        wrap: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) -> (f32, f32) {
        let max = s * 16.0 * wrap;
        // 各实体：名字和属性表（类型、名字、键、说明四列）。
        let tables: Vec<(Label, Vec<[Label; 4]>)> = er
            .entities
            .iter()
            .map(|e| {
                let name = self.styled_label(&e.label, s, look.text, max, true);
                let rows = e
                    .attributes
                    .iter()
                    .map(|a| {
                        [
                            self.label(&a.kind, s * 0.85, look.soft_text, max),
                            self.label(&a.name, s * 0.85, look.text, max),
                            self.label(&a.keys, s * 0.75, look.title, max),
                            self.label(&a.comment, s * 0.8, look.soft_text, max),
                        ]
                    })
                    .collect();
                (name, rows)
            })
            .collect();
        let gap = s * 0.8;
        // 各实体属性表的列宽和行高。
        let grids: Vec<([f32; 4], Vec<f32>)> = tables
            .iter()
            .map(|(_, rows)| {
                let mut cols = [0f32; 4];
                for row in rows {
                    for (c, l) in row.iter().enumerate() {
                        cols[c] = cols[c].max(l.w);
                    }
                }
                let heights = rows
                    .iter()
                    .map(|r| r.iter().map(|l| l.h).fold(0.0, f32::max) + s * PAD_Y * 1.5)
                    .collect();
                (cols, heights)
            })
            .collect();
        let sizes: Vec<(f32, f32)> = tables
            .iter()
            .zip(&grids)
            .map(|((name, _), (cols, heights))| {
                let used = cols.iter().filter(|&&w| w > 0.0);
                let table = used.clone().sum::<f32>() + gap * used.count().saturating_sub(1) as f32;
                let w = name.w.max(table) + s * PAD_X * 2.0;
                let h = name.h + s * PAD_Y * 2.0 + heights.iter().sum::<f32>();
                (w.max(s * 6.0), h)
            })
            .collect();
        let flow = Flow {
            dir: er.dir,
            nodes: er.entities.iter().map(|_| plain_node()).collect(),
            edges: er
                .links
                .iter()
                .map(|l| plain_edge(l.from, l.to, &l.label))
                .collect(),
            groups: Vec::new(),
        };
        let (rects, w, h) = self.boxed(&flow, &sizes, s, wrap, look, out, |_, k, points, out| {
            let link = &er.links[k];
            let strokes = if link.identifying {
                polyline(points)
            } else {
                dashed(points, s * 0.3, s * 0.25)
            };
            out.push(Item::path(strokes, EDGE_WIDTH, look.line));
            let n = points.len();
            let len = s * 1.1;
            er_end(link.from_card, points[0], points[1], len, look, out);
            er_end(link.to_card, points[n - 1], points[n - 2], len, look, out);
        });
        for (rect, ((name, rows), (cols, heights))) in rects.iter().zip(tables.iter().zip(&grids)) {
            let head = name.h + s * PAD_Y * 2.0;
            let cuts: Vec<f32> = if rows.is_empty() {
                Vec::new()
            } else {
                vec![head]
            };
            frame(*rect, head, &cuts, look, out);
            self.put_label(name, rect.x + rect.w / 2.0, rect.y + head / 2.0, out);
            let mut y = rect.y + head;
            let mut lines = Vec::new();
            for (k, (row, &rh)) in rows.iter().zip(heights).enumerate() {
                let mut x = rect.x + s * PAD_X;
                for (c, label) in row.iter().enumerate() {
                    if cols[c] <= 0.0 {
                        continue;
                    }
                    self.paste(&label.piece, x, y + (rh - label.h) / 2.0, 0, out);
                    x += cols[c] + gap;
                }
                if k > 0 {
                    lines.push(Stroke::Line(rect.x, y, rect.right(), y));
                }
                y += rh;
            }
            if !lines.is_empty() {
                out.push(Item::path(lines, 1.0, look.group_border));
            }
        }
        bounds(out, w, h)
    }

    /// 在连线端点 `tip` 附近写多重性 `label`：沿线离开端点一段，再偏到线的一侧，不压住线。
    fn near_end(
        &mut self,
        label: &Label,
        tip: (f32, f32),
        from: (f32, f32),
        s: f32,
        out: &mut Vec<Item>,
    ) {
        if label.w <= 0.0 {
            return;
        }
        let (ux, uy) = unit(tip, from);
        let (nx, ny) = (-uy, ux);
        let along = s * 1.4 + (label.w * ux.abs() + label.h * uy.abs()) / 2.0;
        let side = s * 0.3 + (label.w * nx.abs() + label.h * ny.abs()) / 2.0;
        let cx = tip.0 - ux * along + nx * side;
        let cy = tip.1 - uy * along + ny * side;
        self.put_label(label, cx, cy, out);
    }
}

/// 方框标题栏的高度：上方的标注和类名，加上下边距。
fn head_height(note: Option<&Label>, name: &Label, s: f32) -> f32 {
    note.map_or(0.0, |n| n.h) + name.h + s * PAD_Y * 2.0
}

/// 一栏成员的高度：各行高度之和加上下边距；没有成员时仍留出一段空栏。
fn rows_height(rows: &[Label], s: f32) -> f32 {
    rows.iter().map(|l| l.h).sum::<f32>() + s * PAD_Y * 2.0
}

/// 画方框 `r`：标题栏高 `head`、填色更深，`cuts` 处（相对上边缘）画分栏横线。
fn frame(r: Rect, head: f32, cuts: &[f32], look: Look, out: &mut Vec<Item>) {
    out.push(Item::Fill {
        rect: r,
        color: look.fill,
        radius: NODE_RADIUS,
    });
    let head_rect = Rect::new(r.x, r.y, r.w, head.min(r.h));
    let bottom = if head >= r.h { NODE_RADIUS } else { 0.0 };
    out.push(Item::Corners {
        rect: head_rect,
        color: look.head,
        radii: [NODE_RADIUS, NODE_RADIUS, bottom, bottom],
    });
    let mut strokes = outline(r, NODE_RADIUS);
    for &y in cuts {
        strokes.push(Stroke::Line(r.x, r.y + y, r.right(), r.y + y));
    }
    out.push(Item::path(strokes, 1.0, look.border));
}

/// 只用于分层布局的节点：类图、实体关系图的方框都按矩形处理。
fn plain_node() -> Node {
    Node {
        id: String::new(),
        label: String::new(),
        shape: Shape::Rect,
    }
}

/// 只用于分层布局的连线：起点、终点和文字。
fn plain_edge(from: usize, to: usize, label: &str) -> Edge {
    Edge {
        from,
        to,
        label: label.to_owned(),
        line: Line::Solid,
        head: Head::None,
        tail: Head::None,
        span: 1,
    }
}

/// 从 `from` 指向 `tip` 的单位向量。
fn unit(tip: (f32, f32), from: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let d = (dx * dx + dy * dy).sqrt().max(1e-3);
    (dx / d, dy / d)
}

/// 画 UML 关系标记（尖在 `tip`，线从 `from` 那边来），返回线该画到哪里为止。
///
/// 空心标记先以背景色填充，遮住下面的线。`len` 是标记的长度（逻辑像素）。
fn uml_end(
    end: End,
    tip: (f32, f32),
    from: (f32, f32),
    len: f32,
    look: Look,
    out: &mut Vec<Item>,
) -> (f32, f32) {
    let (ux, uy) = unit(tip, from);
    let (nx, ny) = (-uy, ux);
    let at = |k: f32, side: f32| {
        (
            tip.0 - ux * len * k + nx * len * side,
            tip.1 - uy * len * k + ny * len * side,
        )
    };
    let hollow = |points: Vec<(f32, f32)>, out: &mut Vec<Item>| {
        out.push(Item::polygon(points.clone(), look.label_bg));
        out.push(Item::path(closed(&points), EDGE_WIDTH, look.line));
    };
    match end {
        End::None => tip,
        End::Triangle => {
            hollow(vec![tip, at(1.0, 0.5), at(1.0, -0.5)], out);
            at(1.0, 0.0)
        }
        End::Diamond | End::OpenDiamond => {
            let points = vec![tip, at(0.6, 0.35), at(1.2, 0.0), at(0.6, -0.35)];
            if end == End::Diamond {
                out.push(Item::polygon(points, look.line));
            } else {
                hollow(points, out);
            }
            at(1.2, 0.0)
        }
        End::Arrow => {
            let (a, b) = (at(0.8, 0.4), at(0.8, -0.4));
            out.push(Item::path(
                vec![
                    Stroke::Line(a.0, a.1, tip.0, tip.1),
                    Stroke::Line(tip.0, tip.1, b.0, b.1),
                ],
                EDGE_WIDTH,
                look.line,
            ));
            tip
        }
        End::Lollipop => {
            let r = len * 0.3;
            let c = at(0.3, 0.0);
            out.push(Item::Fill {
                rect: Rect::new(c.0 - r, c.1 - r, r * 2.0, r * 2.0),
                color: look.label_bg,
                radius: r,
            });
            out.push(Item::path(
                vec![Stroke::Circle(c.0, c.1, r)],
                EDGE_WIDTH,
                look.line,
            ));
            at(0.6, 0.0)
        }
    }
}

/// 画实体关系图的基数标记（鸦脚记法）：端点 `tip` 在实体边缘，线从 `from` 那边来。
///
/// 靠近实体的一侧表示上限（竖线为一个，鸦脚为多个），远离实体的一侧表示下限（竖线为至少一个，
/// 圆圈为可以没有）。`len` 是标记的长度（逻辑像素）。
fn er_end(
    card: Card,
    tip: (f32, f32),
    from: (f32, f32),
    len: f32,
    look: Look,
    out: &mut Vec<Item>,
) {
    let (ux, uy) = unit(tip, from);
    let (nx, ny) = (-uy, ux);
    let at = |k: f32, side: f32| {
        (
            tip.0 - ux * len * k + nx * len * side,
            tip.1 - uy * len * k + ny * len * side,
        )
    };
    let bar = |k: f32| {
        let (a, b) = (at(k, 0.4), at(k, -0.4));
        Stroke::Line(a.0, a.1, b.0, b.1)
    };
    let mut strokes = Vec::new();
    let many = matches!(card, Card::ZeroOrMore | Card::OneOrMore);
    if many {
        let root = at(0.65, 0.0);
        for side in [0.45, 0.0, -0.45] {
            let p = at(0.0, side);
            strokes.push(Stroke::Line(root.0, root.1, p.0, p.1));
        }
    } else {
        strokes.push(bar(0.35));
    }
    let far = if many { 0.95 } else { 0.7 };
    let ring = match card {
        Card::ExactlyOne | Card::OneOrMore => {
            strokes.push(bar(far));
            None
        }
        Card::ZeroOrOne | Card::ZeroOrMore => Some(at(far + 0.2, 0.0)),
    };
    out.push(Item::path(strokes, EDGE_WIDTH, look.line));
    if let Some(c) = ring {
        let r = len * 0.2;
        out.push(Item::Fill {
            rect: Rect::new(c.0 - r, c.1 - r, r * 2.0, r * 2.0),
            color: look.label_bg,
            radius: r,
        });
        out.push(Item::path(
            vec![Stroke::Circle(c.0, c.1, r)],
            EDGE_WIDTH,
            look.line,
        ));
    }
}
