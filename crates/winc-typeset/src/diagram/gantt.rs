//! 甘特图：每项任务一行，横条从开始时间画到结束时间；左侧一栏写分组名，底部是时间轴。
//!
//! 时间轴的刻度间隔按图的宽度和刻度文字的宽度自动选取。任务名能放进横条时写在条内，否则写在
//! 条的右侧（靠近右边缘时写在左侧）。坐标和尺寸都是逻辑像素，`s` 是排版用的字号。

use winc_preview::markdown::mermaid::{Gantt, Task, format_time, month_start};
use winc_render::{Color, Rect, Stroke};

use super::super::{Cx, Item};
use super::{Look, bounds, mix, outline};

/// 关键任务（`crit`）的基准色（红色），与背景混合后得到横条的填充色和边框色。
const CRIT_TINT: Color = Color::hex(0xe5534b);

/// 时间轴的宽度与字号之比：字号 14 逻辑像素时约 560 逻辑像素，通常能放进正文宽度。
const AXIS_WIDTH: f32 = 40.0;

/// 一小时、一天的秒数。
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

/// 时间轴的刻度间隔：固定的秒数，或若干个月。
#[derive(Clone, Copy)]
enum Step {
    Seconds(i64),
    Months(i64),
}

impl Cx<'_, '_> {
    /// 排甘特图，返回宽高。
    pub(crate) fn gantt(
        &mut self,
        g: &Gantt,
        s: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) -> (f32, f32) {
        let t0 = g.tasks.iter().map(|t| t.start).min().unwrap_or(0);
        let t1 = g
            .tasks
            .iter()
            .map(|t| t.end)
            .max()
            .unwrap_or(0)
            .max(t0 + HOUR);
        let sections: Vec<_> = g
            .sections
            .iter()
            .map(|name| self.styled_label(name, s * 0.9, look.text, s * 10.0, true))
            .collect();
        let side = sections.iter().map(|l| l.w).fold(0.0, f32::max);
        let left = if side > 0.0 { side + s * 1.6 } else { 0.0 };
        let width = s * AXIS_WIDTH;
        let x_of = |t: i64| left + (t - t0) as f32 / (t1 - t0) as f32 * width;
        let mut top = 0.0;
        if let Some(title) = &g.title {
            let t = self.styled_label(title, s * 1.1, look.text, s * 40.0, true);
            self.put_label(&t, left + width / 2.0, t.h / 2.0, out);
            top = t.h + s * 0.8;
        }
        let row = s * 2.0;
        let bar = s * 1.3;
        let bottom = top + row * g.tasks.len() as f32;

        // 分组的底色：相邻分组交替加底色，分组名写在左栏，垂直居中。
        let mut k = 0;
        while k < g.tasks.len() {
            let section = g.tasks[k].section;
            let len = g.tasks[k..]
                .iter()
                .take_while(|t| t.section == section)
                .count();
            let y = top + row * k as f32;
            if section.is_some_and(|i| i % 2 == 0) {
                out.push(Item::Fill {
                    rect: Rect::new(0.0, y, left + width, row * len as f32),
                    color: look.group_fill,
                    radius: 0.0,
                });
            }
            if let Some(label) = section.and_then(|i| sections.get(i)) {
                let cy = y + row * len as f32 / 2.0;
                self.paste(&label.piece, s * 0.6, cy - label.h / 2.0, 0, out);
            }
            k += len;
        }

        // 时间轴：网格竖线和底部的刻度文字。
        let step = self.axis_step(g, t0, t1, width, s, look);
        let format = g
            .axis
            .clone()
            .unwrap_or_else(|| default_format(step).to_owned());
        let mut grid = Vec::new();
        let mut t = first_tick(t0, step);
        // 刻度数有上限，避免格式异常时无限循环。
        for _ in 0..200 {
            if t > t1 {
                break;
            }
            let x = x_of(t);
            grid.push(Stroke::Line(x, top, x, bottom + s * 0.3));
            let label = self.label(&format_time(t, &format), s * 0.75, look.soft_text, s * 10.0);
            self.put_label(&label, x, bottom + s * 0.5 + label.h / 2.0, out);
            t = next_tick(t, step);
        }
        out.push(Item::path(grid, 1.0, look.group_border));
        out.push(Item::path(
            vec![Stroke::Line(left, bottom, left + width, bottom)],
            1.0,
            look.border,
        ));

        // 任务。
        for (k, task) in g.tasks.iter().enumerate() {
            let cy = top + row * k as f32 + row / 2.0;
            let name = self.label(&task.name, s * 0.85, look.text, s * 20.0);
            let (fill, border) = colors(task, look);
            if task.milestone {
                let x = x_of((task.start + task.end) / 2);
                let d = bar * 0.45;
                out.push(Item::polygon(
                    vec![(x, cy - d), (x + d, cy), (x, cy + d), (x - d, cy)],
                    border,
                ));
                self.put_label(&name, x + d + s * 0.4 + name.w / 2.0, cy, out);
                continue;
            }
            let (x0, x1) = (x_of(task.start), x_of(task.end).max(x_of(task.start) + 2.0));
            let rect = Rect::new(x0, cy - bar / 2.0, x1 - x0, bar);
            let radius = 3.0f32.min(rect.w / 2.0);
            out.push(Item::Fill {
                rect,
                color: fill,
                radius,
            });
            out.push(Item::path(outline(rect, radius), 1.0, border));
            let pad = s * 0.5;
            let cx = if name.w + pad * 2.0 <= rect.w {
                x0 + rect.w / 2.0
            } else if x1 + pad + name.w <= left + width {
                x1 + pad + name.w / 2.0
            } else {
                x0 - pad - name.w / 2.0
            };
            self.put_label(&name, cx, cy, out);
        }
        bounds(out, left + width, bottom + s * 1.6)
    }

    /// 选取时间轴的刻度间隔：在候选间隔中取刻度文字不重叠的最小一个。
    fn axis_step(&mut self, g: &Gantt, t0: i64, t1: i64, width: f32, s: f32, look: Look) -> Step {
        let candidates = [
            Step::Seconds(HOUR),
            Step::Seconds(3 * HOUR),
            Step::Seconds(6 * HOUR),
            Step::Seconds(12 * HOUR),
            Step::Seconds(DAY),
            Step::Seconds(2 * DAY),
            Step::Seconds(7 * DAY),
            Step::Seconds(14 * DAY),
            Step::Months(1),
            Step::Months(3),
            Step::Months(6),
            Step::Months(12),
        ];
        let span = (t1 - t0) as f32;
        for step in candidates {
            let format = g.axis.as_deref().unwrap_or(default_format(step));
            let sample = self.label(&format_time(t0, format), s * 0.75, look.soft_text, s * 10.0);
            let seconds = match step {
                Step::Seconds(n) => n as f32,
                Step::Months(n) => n as f32 * 30.0 * DAY as f32,
            };
            if seconds / span * width >= sample.w + s * 1.2 {
                return step;
            }
        }
        Step::Months(12)
    }
}

/// 没有指定 `axisFormat` 时刻度文字的格式：按小时的间隔写时刻，按天的写月日，按月的写年月。
fn default_format(step: Step) -> &'static str {
    match step {
        Step::Seconds(n) if n < DAY => "%H:%M",
        Step::Seconds(_) => "%m-%d",
        Step::Months(_) => "%Y-%m",
    }
}

/// 不早于 `t` 的第一个刻度：按天及更短的间隔对齐到整点，按周的对齐到周一，按月的对齐到月初。
fn first_tick(t: i64, step: Step) -> i64 {
    match step {
        Step::Seconds(n) if n >= 7 * DAY => {
            // 1970-01-01 是周四，往后 4 天是周一。
            let monday = 4 * DAY;
            (t - monday).div_euclid(n) * n
                + monday
                + if (t - monday).rem_euclid(n) == 0 {
                    0
                } else {
                    n
                }
        }
        Step::Seconds(n) => t.div_euclid(n) * n + if t.rem_euclid(n) == 0 { 0 } else { n },
        Step::Months(n) => {
            let first = month_start(t, 0);
            let first = if first < t { month_start(t, 1) } else { first };
            // 按季度、半年、年的间隔从对应的月份开始。
            let month: i64 = format_time(first, "%m").parse().unwrap_or(1);
            month_start(first, (n - (month - 1) % n) % n)
        }
    }
}

/// `t` 之后的下一个刻度。
fn next_tick(t: i64, step: Step) -> i64 {
    match step {
        Step::Seconds(n) => t + n,
        Step::Months(n) => month_start(t, n),
    }
}

/// 任务横条的填充色和边框色：已完成为灰色，关键任务为红色，其余为强调色；进行中的填充色较浅、
/// 边框较深。
fn colors(task: &Task, look: Look) -> (Color, Color) {
    let page = look.label_bg;
    let hue = if task.crit { CRIT_TINT } else { look.title };
    let fill = if task.done {
        mix(page, look.text, if look.dark { 0.2 } else { 0.12 })
    } else if task.active {
        mix(page, hue, if look.dark { 0.16 } else { 0.1 })
    } else {
        mix(page, hue, if look.dark { 0.4 } else { 0.28 })
    };
    let border = if task.done && !task.crit {
        mix(page, look.text, 0.35)
    } else if task.active {
        hue
    } else {
        mix(page, hue, 0.7)
    };
    (fill, border)
}
