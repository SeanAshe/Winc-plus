//! Git 提交图：每个分支一条泳道，提交按时间先后等距排开；分出与合并的连线在拐角处取圆角。
//!
//! 布局先在「时间、泳道」坐标中进行，再按图的方向换成屏幕坐标：从左往右时时间为横坐标，从上
//! 往下（或从下往上）时时间为纵坐标。坐标和尺寸都是逻辑像素，`s` 是排版用的字号。

use winc_preview::markdown::mermaid::{CommitKind, Dir, Git};
use winc_render::{Color, Rect, Stroke};

use super::super::math::polyline;
use super::super::{Cx, Item};
use super::{Label, Look, bounds, dashed, mix, palette};

impl Cx<'_, '_> {
    /// 排 Git 提交图，返回宽高。
    pub(super) fn git(&mut self, g: &Git, s: f32, look: Look, out: &mut Vec<Item>) -> (f32, f32) {
        let across = g.dir == Dir::Right;
        let names: Vec<Label> = g
            .branches
            .iter()
            .map(|b| self.styled_label(b, s * 0.85, look.text, s * 12.0, true))
            .collect();
        let ids: Vec<Option<Label>> = g
            .commits
            .iter()
            .map(|c| {
                let id = match c.picked {
                    Some(p) => g.commits[p].id.as_ref(),
                    None => c.id.as_ref(),
                };
                id.map(|id| self.label(id, s * 0.75, look.soft_text, s * 10.0))
            })
            .collect();
        let tags: Vec<Option<Label>> = g
            .commits
            .iter()
            .map(|c| {
                c.tag
                    .as_ref()
                    .map(|t| self.label(t, s * 0.75, look.text, s * 10.0))
            })
            .collect();
        let r = s * 0.6;
        let step = s * 3.2;
        let pill = |l: &Label| (l.w + s * 1.2, l.h + s * 0.5);
        // 分支名写在泳道起点的圆角标签里；横排时各标签左对齐成一列，竖排时写在泳道上方。
        let widest = names.iter().map(|l| pill(l).0).fold(0.0, f32::max);
        let lane_gap = if across {
            s * 3.4
        } else {
            let id_w = ids.iter().flatten().map(|l| l.w).fold(0.0, f32::max);
            (widest + s).max(r * 2.0 + id_w + s * 1.5).max(s * 3.4)
        };
        let head = if across {
            widest + s * 1.5
        } else {
            names[0].h + s * 2.0
        };
        let n = g.commits.len();
        let along = |k: usize| {
            let k = if g.dir == Dir::Up { n - 1 - k } else { k };
            head + s * 1.2 + k as f32 * step
        };
        let end = head + s * 2.4 + (n.saturating_sub(1)) as f32 * step;
        let lane = |b: usize| s * 1.0 + b as f32 * lane_gap;
        let at = |a: f32, c: f32| if across { (a, c) } else { (c, a) };
        let colors: Vec<Color> = (0..g.branches.len()).map(palette).collect();

        // 泳道：从分支名标签一直画到最后一次提交之后。
        for (b, name) in names.iter().enumerate() {
            let (pw, ph) = pill(name);
            let c = lane(b);
            let start = if across { pw } else { head - s * 0.4 };
            let line = [at(start, c), at(end, c)];
            out.push(Item::path(
                dashed(&line, s * 0.3, s * 0.25),
                1.0,
                mix(look.label_bg, colors[b], 0.55),
            ));
            let rect = if across {
                Rect::new(0.0, c - ph / 2.0, pw, ph)
            } else {
                Rect::new(c - pw / 2.0, head - s * 0.8 - ph, pw, ph)
            };
            out.push(Item::Fill {
                rect,
                color: mix(look.label_bg, colors[b], if look.dark { 0.3 } else { 0.18 }),
                radius: ph / 2.0,
            });
            out.push(Item::path(
                super::outline(rect, ph / 2.0),
                1.0,
                mix(look.label_bg, colors[b], 0.7),
            ));
            self.put_label(name, rect.x + pw / 2.0, rect.y + ph / 2.0, out);
        }

        // 父提交到子提交的连线：分出时先拐到新泳道再沿时间方向走，合并时先沿时间方向走再
        // 拐到目标泳道；颜色取泳道靠后的一方。
        let place = |k: usize| (along(k), lane(g.commits[k].branch));
        for (k, c) in g.commits.iter().enumerate() {
            for &p in &c.parents {
                let (pa, pc) = place(p);
                let (ka, kc) = place(k);
                let corner = if c.branch > g.commits[p].branch {
                    (pa, kc)
                } else {
                    (ka, pc)
                };
                let points: Vec<(f32, f32)> = elbow((pa, pc), corner, (ka, kc), s * 0.9)
                    .into_iter()
                    .map(|(a, c)| at(a, c))
                    .collect();
                let color = colors[c.branch.max(g.commits[p].branch)];
                out.push(Item::path(polyline(&points), s * 0.18, color));
            }
        }

        // 提交点、提交名与标签。
        for (k, c) in g.commits.iter().enumerate() {
            let (x, y) = at(place(k).0, place(k).1);
            let color = colors[c.branch];
            let bg = look.label_bg;
            let dot = |radius: f32, color: Color| Item::Fill {
                rect: Rect::new(x - radius, y - radius, radius * 2.0, radius * 2.0),
                color,
                radius,
            };
            match c.kind {
                CommitKind::Normal => out.push(dot(r, color)),
                CommitKind::Merge => {
                    out.push(dot(r, color));
                    out.push(dot(r * 0.45, bg));
                }
                CommitKind::CherryPick => {
                    out.push(dot(r, bg));
                    out.push(Item::path(
                        vec![Stroke::Circle(x, y, r * 0.85)],
                        s * 0.18,
                        color,
                    ));
                }
                CommitKind::Reverse => {
                    out.push(dot(r, color));
                    let d = r * 0.45;
                    out.push(Item::path(
                        vec![
                            Stroke::Line(x - d, y - d, x + d, y + d),
                            Stroke::Line(x - d, y + d, x + d, y - d),
                        ],
                        s * 0.12,
                        bg,
                    ));
                }
                CommitKind::Highlight => out.push(Item::Fill {
                    rect: Rect::new(x - r, y - r, r * 2.0, r * 2.0),
                    color,
                    radius: 2.0,
                }),
            }
            if let Some(id) = &ids[k] {
                let (cx, cy) = if across {
                    (x, y + r + s * 0.3 + id.h / 2.0)
                } else {
                    (x + r + s * 0.5 + id.w / 2.0, y)
                };
                // 底色遮住从提交名下方经过的分出连线。
                out.push(Item::Fill {
                    rect: Rect::new(cx - id.w / 2.0 - 2.0, cy - id.h / 2.0, id.w + 4.0, id.h),
                    color: bg,
                    radius: 2.0,
                });
                self.put_label(id, cx, cy, out);
            }
            if let Some(tag) = &tags[k] {
                let (tw, th) = (tag.w + s * 0.8, tag.h + s * 0.2);
                let rect = if across {
                    Rect::new(x - tw / 2.0, y - r - s * 0.4 - th, tw, th)
                } else {
                    Rect::new(x - r - s * 0.5 - tw, y - th / 2.0, tw, th)
                };
                out.push(Item::Fill {
                    rect,
                    color: look.fill,
                    radius: 3.0,
                });
                out.push(Item::path(super::outline(rect, 3.0), 1.0, look.border));
                self.put_label(tag, rect.x + tw / 2.0, rect.y + th / 2.0, out);
            }
        }
        bounds(out, 0.0, 0.0)
    }
}

/// 从 `a` 经拐角 `corner` 到 `b` 的折线，拐角处是半径不超过 `radius` 的圆角；三点共线时为直线。
fn elbow(a: (f32, f32), corner: (f32, f32), b: (f32, f32), radius: f32) -> Vec<(f32, f32)> {
    let len = |p: (f32, f32), q: (f32, f32)| ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
    let (la, lb) = (len(a, corner), len(corner, b));
    if la < 0.01 || lb < 0.01 {
        return vec![a, b];
    }
    let rad = radius.min(la).min(lb);
    let toward = |p: (f32, f32), q: (f32, f32), d: f32| {
        let l = len(p, q);
        (p.0 + (q.0 - p.0) * d / l, p.1 + (q.1 - p.1) * d / l)
    };
    let (p0, p1) = (toward(corner, a, rad), toward(corner, b, rad));
    let mut points = vec![a, p0];
    // 以拐角为控制点的二次贝塞尔曲线，取 8 段。
    for k in 1..=8 {
        let t = k as f32 / 8.0;
        let u = 1.0 - t;
        points.push((
            u * u * p0.0 + 2.0 * u * t * corner.0 + t * t * p1.0,
            u * u * p0.1 + 2.0 * u * t * corner.1 + t * t * p1.1,
        ));
    }
    points.push(b);
    points
}
