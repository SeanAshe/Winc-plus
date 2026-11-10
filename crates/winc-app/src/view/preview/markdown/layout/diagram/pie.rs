//! 饼图：从正上方顺时针一块块画，块里写百分比，右边是图例。
//!
//! 各块的颜色按 [`super::PALETTE`] 轮流取。坐标和尺寸都是逻辑像素，
//! `s` 是排版用的字号。

use std::f32::consts::{FRAC_PI_2, TAU};

use winc_preview::markdown::mermaid::Pie;
use winc_render::{Color, Rect, Stroke};

use super::super::{Cx, Item};
use super::{Look, bounds, ellipse, palette};

impl Cx<'_, '_> {
    /// 排饼图，返回宽高。
    pub(super) fn pie(&mut self, pie: &Pie, s: f32, look: Look, out: &mut Vec<Item>) -> (f32, f32) {
        let total: f64 = pie.slices.iter().map(|(_, v)| v.max(0.0)).sum();
        let mut top = 0.0;
        // 饼的半径（逻辑像素，经验值）：字号的 7 倍，加上图例后仍能排进正文宽度。
        let r = s * 7.0;
        if let Some(title) = &pie.title {
            let t = self.label(title, s * 1.1, look.text, s * 30.0);
            self.put_label(&t, r, t.h / 2.0, out);
            top = t.h + s;
        }
        let (cx, cy) = (r, top + r);
        let ink = Color::hex(0x1f1f1f);
        let mut at = -FRAC_PI_2;
        let mut seams = Vec::new();
        let mut percents = Vec::new();
        for (k, (_, value)) in pie.slices.iter().enumerate() {
            if total <= 0.0 {
                break;
            }
            let share = (value.max(0.0) / total) as f32;
            let sweep = share * TAU;
            let color = palette(k);
            // 多边形要凸：每块切成不超过四分之一圈的小扇形。
            let chunks = (sweep / FRAC_PI_2).ceil().max(1.0) as usize;
            for c in 0..chunks {
                let a0 = at + sweep * c as f32 / chunks as f32;
                // 同一块里相邻的小扇形稍微叠一点，接缝处不会因为抗锯齿露出一道浅线。
                let overlap = if c + 1 < chunks { 0.02 } else { 0.0 };
                let a1 = at + sweep * (c + 1) as f32 / chunks as f32 + overlap;
                // 每圈至少切 96 段（经验值），小扇形连起来看不出棱角。
                let steps = ((a1 - a0) / TAU * 96.0).ceil().max(2.0) as usize;
                let mut points = vec![(cx, cy)];
                points.extend(ellipse(cx, cy, r, r, a0, a1, steps));
                out.push(Item::polygon(points, color));
            }
            if pie.slices.len() > 1 {
                seams.push(Stroke::Line(cx, cy, cx + r * at.cos(), cy + r * at.sin()));
            }
            // 占比小于 3%（经验值）的块里写不下百分比，只在图例里列出。
            if share >= 0.03 {
                let mid = at + sweep / 2.0;
                percents.push((format!("{:.0}%", share * 100.0), mid));
            }
            at += sweep;
        }
        seams.push(Stroke::Circle(cx, cy, r));
        out.push(Item::path(seams, 1.5, look.label_bg));
        for (text, mid) in percents {
            let label = self.label(&text, s * 0.85, ink, s * 6.0);
            // 百分比写在半径 68% 处（经验值），靠近外沿、又不会压到接缝。
            let d = r * 0.68;
            self.put_label(&label, cx + d * mid.cos(), cy + d * mid.sin(), out);
        }

        // 图例：色块加名字，竖着一列，和饼的中间对齐。
        let rows: Vec<_> = pie
            .slices
            .iter()
            .map(|(name, value)| {
                let text = if pie.show_data {
                    format!("{name} [{}]", number(*value))
                } else {
                    name.clone()
                };
                self.label(&text, s * 0.9, look.text, s * 16.0)
            })
            .collect();
        let row_h = rows.iter().map(|l| l.h).fold(s * 1.1, f32::max) + s * 0.4;
        let left = cx + r + s * 2.0;
        let mut y = cy - row_h * rows.len() as f32 / 2.0;
        for (k, label) in rows.iter().enumerate() {
            let sq = s * 0.9;
            out.push(Item::Fill {
                rect: Rect::new(left, y + (row_h - sq) / 2.0, sq, sq),
                color: palette(k),
                radius: 2.0,
            });
            self.put_label(
                label,
                left + sq + s * 0.5 + label.w / 2.0,
                y + row_h / 2.0,
                out,
            );
            y += row_h;
        }
        bounds(out, 0.0, cy + r)
    }
}

/// 图例里的数：整数不带小数点。
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
