//! 时序图：参与者从左往右排开，消息、注释、框从上往下一行行排。
//!
//! 列之间的距离由跨过它的消息、旁边的注释的文字宽度撑开。坐标和尺寸都是逻辑像素，`s` 是
//! 排版用的字号，图中的距离和粗细都按字号的比例确定。

use winc_preview::markdown::mermaid::{Arrow, Event, Head, Sequence, Side};
use winc_render::{Color, Rect, Stroke};

use super::super::math::polyline;
use super::super::{Cx, Item};
use super::{Label, Look, arrow_head, bounds, dashed, outline};

/// `loop`、`alt` 这类框：从哪开始、种类、说明、中间的分隔线，碰到的参与者的范围。
struct Frame {
    /// 上边缘（逻辑像素）。
    top: f32,
    /// 框的种类（`loop`、`alt` 等），写在左上角的小标签里。
    kind: String,
    /// 种类后面的说明，写在标签右边。
    label: String,
    /// 中间分隔线的位置（逻辑像素）和说明。
    dividers: Vec<(f32, String)>,
    /// 碰到的参与者范围（序号），画框时按它取左右边界。
    span: Option<(usize, usize)>,
}

impl Cx<'_, '_> {
    /// 排时序图，返回宽高。
    pub(crate) fn sequence(
        &mut self,
        seq: &Sequence,
        s: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) -> (f32, f32) {
        let n = seq.actors.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        let names: Vec<Label> = seq
            .actors
            .iter()
            .map(|a| self.label(&a.label, s, look.text, s * 10.0))
            .collect();
        let boxes: Vec<(f32, f32)> = seq
            .actors
            .iter()
            .zip(&names)
            .map(|(a, l)| {
                if a.person {
                    (l.w.max(s * 3.0), s * 2.6 + l.h)
                } else {
                    ((l.w + s * 2.0).max(s * 6.0), l.h + s * 1.3)
                }
            })
            .collect();
        let box_h = boxes.iter().map(|b| b.1).fold(0.0, f32::max);

        // 先排好所有文字，按它们的宽度定列距。
        let texts: Vec<Option<Label>> = seq
            .events
            .iter()
            .map(|e| match e {
                Event::Message { text, .. } => Some(self.label(text, s * 0.9, look.text, s * 18.0)),
                Event::Note { text, .. } => Some(self.label(text, s * 0.9, look.text, s * 14.0)),
                Event::Begin(_, label) | Event::Divide(label) if !label.is_empty() => {
                    Some(self.label(&format!("[{label}]"), s * 0.85, look.text, s * 20.0))
                }
                _ => None,
            })
            .collect();
        let mut need: Vec<(usize, usize, f32)> = (1..n)
            .map(|i| (i - 1, i, (boxes[i - 1].0 + boxes[i].0) / 2.0 + s * 2.5))
            .collect();
        for (e, t) in seq.events.iter().zip(&texts) {
            let w = t.as_ref().map_or(0.0, |t| t.w);
            match *e {
                Event::Message { from, to, .. } if from != to => {
                    need.push((from.min(to), from.max(to), w + s * 2.0));
                }
                Event::Message { from, .. } if from + 1 < n => {
                    need.push((from, from + 1, w.max(s * 2.0) + s * 2.5));
                }
                Event::Note {
                    side: Side::Right,
                    actors: (a, _),
                    ..
                } if a + 1 < n => need.push((a, a + 1, w + s * 2.5)),
                Event::Note {
                    side: Side::Left,
                    actors: (a, _),
                    ..
                } if a > 0 => need.push((a - 1, a, w + s * 2.5)),
                _ => {}
            }
        }
        need.sort_by_key(|&(a, b, _)| (b, a));
        let mut x = vec![0f32; n];
        x[0] = boxes[0].0 / 2.0;
        for i in 1..n {
            x[i] = x[i - 1];
        }
        for &(a, b, d) in &need {
            let short = d - (x[b] - x[a]);
            if short > 0.0 {
                for v in &mut x[b..] {
                    *v += short;
                }
            }
        }

        let mut top = 0.0;
        let mut title_out = Vec::new();
        if let Some(title) = &seq.title {
            let t = self.label(title, s * 1.1, look.text, s * 40.0);
            let mid = (x[0] + x[n - 1]) / 2.0;
            self.put_label(&t, mid, t.h / 2.0, &mut title_out);
            top = t.h + s;
        }

        let mut back = Vec::new();
        let mut front = Vec::new();
        let mut y = top + box_h + s * 1.2;
        // 激活条的宽度（逻辑像素，经验值）：字号的 0.4 倍，方块的边线能看清。
        let bar = s * 0.4;
        let mut active: Vec<Vec<f32>> = vec![Vec::new(); n];
        let mut frames: Vec<Frame> = Vec::new();
        let mut number = 0;
        let touch = |frames: &mut Vec<Frame>, a: usize, b: usize| {
            for f in frames.iter_mut() {
                f.span = Some(match f.span {
                    Some((l, r)) => (l.min(a), r.max(b)),
                    None => (a, b),
                });
            }
        };
        // 消息线头离生命线多远：有激活条时就从激活条的边上出发。
        let edge = |active: &Vec<Vec<f32>>, i: usize, toward_right: bool| {
            let depth = active[i].len() as f32;
            let off = if depth > 0.0 {
                bar / 2.0 + (depth - 1.0) * bar / 2.0
            } else {
                0.0
            };
            if toward_right { x[i] + off } else { x[i] - off }
        };
        for (e, t) in seq.events.iter().zip(&texts) {
            match e {
                &Event::Message {
                    from,
                    to,
                    dotted,
                    arrow,
                    both,
                    activate,
                    ..
                } => {
                    touch(&mut frames, from.min(to), from.max(to));
                    let t = t.as_ref().expect("消息都排了文字");
                    let head = match arrow {
                        Arrow::Filled => Head::Arrow,
                        Arrow::Cross => Head::Cross,
                        Arrow::Plain | Arrow::Open => Head::None,
                    };
                    let mut points;
                    if from == to {
                        let x0 = edge(&active, from, true);
                        y += t.h.max(s * 0.2);
                        if t.w > 0.0 {
                            self.put_label(
                                t,
                                x0 + s * 0.6 + t.w / 2.0,
                                y - t.h / 2.0 - s * 0.2,
                                &mut front,
                            );
                        }
                        let w = s * 2.0;
                        points = vec![
                            (x0, y),
                            (x0 + w, y),
                            (x0 + w, y + s * 1.2),
                            (x0, y + s * 1.2),
                        ];
                        y += s * 1.2;
                    } else {
                        y += t.h + s * 0.3;
                        let right = to > from;
                        let a = edge(&active, from, right);
                        let b = edge(&active, to, !right);
                        let mid = (x[from] + x[to]) / 2.0;
                        if t.w > 0.0 {
                            self.put_label(t, mid, y - s * 0.3 - t.h / 2.0, &mut front);
                        }
                        points = vec![(a, y), (b, y)];
                    }
                    let len = s * 0.6;
                    for at_end in [true, false] {
                        if !at_end && !both {
                            continue;
                        }
                        let k = points.len();
                        let (tip, back_to) = if at_end {
                            (points[k - 1], points[k - 2])
                        } else {
                            (points[0], points[1])
                        };
                        let stop = if arrow == Arrow::Open {
                            open_arrow(tip, back_to, len, look.line, &mut front);
                            tip
                        } else {
                            arrow_head(head, tip, back_to, len, look.line, &mut front)
                        };
                        if at_end {
                            points[k - 1] = stop;
                        } else {
                            points[0] = stop;
                        }
                    }
                    let strokes = if dotted {
                        dashed(&points, s * 0.25, s * 0.2)
                    } else {
                        polyline(&points)
                    };
                    front.push(Item::path(strokes, s * 0.1, look.line));
                    if seq.numbered {
                        number += 1;
                        let r = s * 0.55;
                        let (cx, cy) = points[0];
                        front.push(Item::Fill {
                            rect: Rect::new(cx - r, cy - r, r * 2.0, r * 2.0),
                            color: look.line,
                            radius: r,
                        });
                        let ink = look.label_bg;
                        let label = self.label(&number.to_string(), s * 0.7, ink, s * 4.0);
                        self.put_label(&label, cx, cy, &mut front);
                    }
                    match activate {
                        Some(true) => active[to].push(y),
                        Some(false) => {
                            if let Some(start) = active[from].pop() {
                                activation(
                                    x[from],
                                    start,
                                    y,
                                    active[from].len(),
                                    bar,
                                    look,
                                    &mut back,
                                );
                            }
                        }
                        None => {}
                    }
                    y += s * 0.9;
                }
                &Event::Note {
                    side,
                    actors: (a, b),
                    ..
                } => {
                    let t = t.as_ref().expect("注释都排了文字");
                    let (l, r) = (a.min(b), a.max(b));
                    touch(&mut frames, l, r);
                    let h = t.h + s * 0.8;
                    let w = t.w + s * 1.2;
                    let (left, w) = match side {
                        Side::Left => (x[l] - s * 0.6 - w, w),
                        Side::Right => (x[l] + s * 0.6, w),
                        Side::Over => {
                            let span = x[r] - x[l] + s * 2.0;
                            let w = w.max(span);
                            ((x[l] + x[r]) / 2.0 - w / 2.0, w)
                        }
                    };
                    let rect = Rect::new(left, y, w, h);
                    front.push(Item::Fill {
                        rect,
                        color: look.note_fill,
                        radius: 0.0,
                    });
                    front.push(Item::path(outline(rect, 0.0), 1.0, look.note_border));
                    self.put_label(t, left + w / 2.0, y + h / 2.0, &mut front);
                    y += h + s * 0.8;
                }
                Event::Begin(kind, label) => {
                    frames.push(Frame {
                        top: y,
                        kind: kind.clone(),
                        label: label.clone(),
                        dividers: Vec::new(),
                        span: None,
                    });
                    let h = t.as_ref().map_or(0.0, |t| t.h).max(s * 1.2);
                    y += h + s * 1.0;
                }
                Event::Divide(label) => {
                    if let Some(f) = frames.last_mut() {
                        f.dividers.push((y, label.clone()));
                    }
                    let h = t.as_ref().map_or(0.0, |t| t.h).max(s * 0.6);
                    y += h + s * 0.8;
                }
                Event::End => {
                    if let Some(f) = frames.pop() {
                        let (l, r) = f.span.unwrap_or((0, n - 1));
                        let pad = s * 1.2;
                        let left = x[l] - boxes[l].0 / 2.0 - pad * 0.3;
                        let right = x[r] + boxes[r].0 / 2.0 + pad * 0.3;
                        let rect = Rect::new(left, f.top, right - left, y - f.top + s * 0.2);
                        self.frame(&f, rect, s, look, &mut back);
                        if let Some(outer) = frames.last_mut() {
                            outer.span = Some(match outer.span {
                                Some((a, b)) => (a.min(l), b.max(r)),
                                None => (l, r),
                            });
                        }
                    }
                    y += s * 0.8;
                }
                &Event::Activate(i, on) => {
                    if on {
                        active[i].push(y - s * 0.4);
                    } else if let Some(start) = active[i].pop() {
                        activation(
                            x[i],
                            start,
                            y - s * 0.4,
                            active[i].len(),
                            bar,
                            look,
                            &mut back,
                        );
                    }
                }
            }
        }
        for (i, starts) in active.iter().enumerate() {
            for (depth, &start) in starts.iter().enumerate() {
                activation(x[i], start, y, depth, bar, look, &mut back);
            }
        }
        y += s * 0.4;

        // 生命线、参与者（上下各一个）。
        let lifeline_top = top + box_h;
        for &at in &x {
            let line = dashed(&[(at, lifeline_top), (at, y)], s * 0.25, s * 0.25);
            out.push(Item::path(line, 1.0, look.line.alpha(0.6)));
        }
        out.append(&mut back);
        out.append(&mut front);
        for (i, actor) in seq.actors.iter().enumerate() {
            for at in [top, y] {
                let (w, h) = boxes[i];
                let rect = Rect::new(x[i] - w / 2.0, at + box_h - h, w, h);
                if actor.person {
                    stick_figure(x[i], rect.y, s, look, out);
                    self.put_label(&names[i], x[i], rect.bottom() - names[i].h / 2.0, out);
                } else {
                    out.push(Item::Fill {
                        rect,
                        color: look.fill,
                        radius: 3.0,
                    });
                    out.push(Item::path(outline(rect, 3.0), 1.2, look.border));
                    self.put_label(&names[i], x[i], rect.y + h / 2.0, out);
                }
            }
        }
        out.append(&mut title_out);
        bounds(out, 0.0, y + box_h)
    }

    /// 画 `loop`、`alt` 这类框：外框、左上角写种类的小标签、说明、分隔线。
    fn frame(&mut self, f: &Frame, rect: Rect, s: f32, look: Look, out: &mut Vec<Item>) {
        out.push(Item::path(outline(rect, 0.0), 1.0, look.line.alpha(0.7)));
        let tag = self.label(&f.kind, s * 0.85, look.text, s * 10.0);
        let (tw, th) = (tag.w + s * 0.8, tag.h + s * 0.3);
        let (x0, y0) = (rect.x, rect.y);
        let cut = s * 0.4;
        out.push(Item::polygon(
            vec![
                (x0, y0),
                (x0 + tw, y0),
                (x0 + tw, y0 + th - cut),
                (x0 + tw - cut, y0 + th),
                (x0, y0 + th),
            ],
            look.group_fill,
        ));
        out.push(Item::path(
            polyline(&[
                (x0 + tw, y0),
                (x0 + tw, y0 + th - cut),
                (x0 + tw - cut, y0 + th),
                (x0, y0 + th),
            ]),
            1.0,
            look.line.alpha(0.7),
        ));
        self.put_label(&tag, x0 + tw / 2.0, y0 + th / 2.0, out);
        let mid = rect.x + (rect.w + tw) / 2.0;
        if !f.label.is_empty() {
            let label = self.label(&format!("[{}]", f.label), s * 0.85, look.text, s * 20.0);
            self.put_label(
                &label,
                mid.max(x0 + tw + label.w / 2.0 + s * 0.3),
                y0 + th / 2.0,
                out,
            );
        }
        for (y, text) in &f.dividers {
            let line = dashed(&[(rect.x, *y), (rect.right(), *y)], s * 0.3, s * 0.25);
            out.push(Item::path(line, 1.0, look.line.alpha(0.7)));
            if !text.is_empty() {
                let label = self.label(&format!("[{text}]"), s * 0.85, look.text, s * 20.0);
                self.put_label(
                    &label,
                    rect.x + rect.w / 2.0,
                    y + label.h / 2.0 + s * 0.2,
                    out,
                );
            }
        }
    }
}

/// 激活条：生命线上从 `top` 到 `bottom` 的窄方块，`depth` 层叠着往右错开。
fn activation(
    x: f32,
    top: f32,
    bottom: f32,
    depth: usize,
    bar: f32,
    look: Look,
    out: &mut Vec<Item>,
) {
    let rect = Rect::new(
        x - bar / 2.0 + depth as f32 * bar / 2.0,
        top,
        bar,
        (bottom - top).max(bar),
    );
    out.push(Item::Fill {
        rect,
        color: look.fill,
        radius: 0.0,
    });
    out.push(Item::path(outline(rect, 0.0), 1.0, look.border));
}

/// 开口箭头（异步消息）：两条斜线。
fn open_arrow(tip: (f32, f32), from: (f32, f32), len: f32, color: Color, out: &mut Vec<Item>) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let d = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / d, dy / d);
    let base = (tip.0 - ux * len, tip.1 - uy * len);
    let (px, py) = (-uy * len * 0.45, ux * len * 0.45);
    out.push(Item::path(
        vec![
            Stroke::Line(base.0 + px, base.1 + py, tip.0, tip.1),
            Stroke::Line(base.0 - px, base.1 - py, tip.0, tip.1),
        ],
        len * 0.15,
        color,
    ));
}

/// `actor` 画的小人，头顶在 `(cx, top)`。
fn stick_figure(cx: f32, top: f32, s: f32, look: Look, out: &mut Vec<Item>) {
    let r = s * 0.4;
    let neck = top + r * 2.0;
    let hip = neck + s * 0.9;
    let foot = hip + s * 0.8;
    out.push(Item::path(
        vec![
            Stroke::Circle(cx, top + r, r),
            Stroke::Line(cx, neck, cx, hip),
            Stroke::Line(cx - s * 0.7, neck + s * 0.35, cx + s * 0.7, neck + s * 0.35),
            Stroke::Line(cx, hip, cx - s * 0.55, foot),
            Stroke::Line(cx, hip, cx + s * 0.55, foot),
        ],
        1.4,
        look.border,
    ));
}
