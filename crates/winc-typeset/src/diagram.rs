//! mermaid 图的布局与绘制；解析见 `winc_preview::markdown::mermaid`。
//!
//! 配色由预览的背景色和正文颜色按比例混合得出（见 `Look`）。流程图采用分层布局：去除环后按最长
//! 路径分层，跨层的连线插入虚节点，连线文字占用其中一个虚节点；逐层按邻居的平均位置排序以减少
//! 交叉，再按邻居对齐确定横坐标。时序图、饼图见子模块。坐标单位均为逻辑像素。

use winc_preview::markdown::mermaid::{self, Diagram, Dir, Edge, Flow, Head, Line, Shape};
use winc_preview::markdown::{Align, Inline, Run, Style};
use winc_render::{Color, Rect, Stroke};

use super::math::polyline;
use super::memo::Piece;
use super::{Cx, Env, Flow as Laid, Item, Para};
use winc_render::rounded_outline;

mod class;
mod gantt;
mod git;
mod mindmap;
mod pie;
mod rank;
mod sequence;

use rank::{Plan, Route, layout};

/// 流程图宽于可用宽度时，节点与连线文字的折行宽度相对原宽度的比例。
///
/// 先按该比例折行重排，仍然过宽才缩小字号：多折一行对可读性的影响小于整体缩小字号。取自
/// 调参：0.7 时常见的横向架构图宽度可减少约四分之一，节点高度增加不超过一行。
const NARROW_WRAP: f32 = 0.7;

/// 连线的线宽（逻辑像素）：不随缩小的字号变细，图缩小后线条仍保持清晰。
const EDGE_WIDTH: f32 = 1.25;
/// 粗连线（`==>`）的线宽（逻辑像素），为普通连线的 2 倍。
const EDGE_THICK: f32 = 2.5;

/// 图中文字的字号与正文字号之比：正文 16 逻辑像素时为 14 逻辑像素。
const TEXT_RATIO: f32 = 0.875;

/// 解析一张图；无法识别或不支持绘制的图返回 `None`。
fn supported(source: &str) -> Option<Diagram> {
    mermaid::parse(source).filter(|d| !matches!(d, Diagram::Unsupported(_)))
}

/// 节点矩形的圆角半径（逻辑像素）。
const NODE_RADIUS: f32 = 4.0;
/// 圆角矩形节点（`(…)`）的圆角半径（逻辑像素）。
const ROUND_RADIUS: f32 = 10.0;
/// 子图框的圆角半径（逻辑像素）。
const GROUP_RADIUS: f32 = 8.0;
/// 时序图注释框的基准色（琥珀色），与预览背景色混合后得到注释框的填充色和边框色。
const NOTE_TINT: Color = Color::hex(0xe8b339);

/// 饼图各块、思维导图各分支、Git 提交图各分支的颜色（`0xRRGGBB`），按顺序轮流使用，共 10 种，
/// 更多时从头重复。色值取自 mermaid 默认主题。
const PALETTE: [u32; 10] = [
    0x4e79a7, 0xf28e2b, 0xe15759, 0x76b7b2, 0x59a14f, 0xedc948, 0xb07aa1, 0xff9da7, 0x9c755f,
    0xbab0ac,
];

/// 第 `k` 种轮流使用的颜色，见 [`PALETTE`]。
fn palette(k: usize) -> Color {
    Color::hex(PALETTE[k % PALETTE.len()])
}

/// 图的配色。
///
/// 各颜色由预览的背景色和正文颜色按比例混合得出，不另设色板：深浅主题使用同一组比例，图与
/// 正文的明暗、色调保持一致；子图标题使用强调色。
#[derive(Clone, Copy)]
pub(crate) struct Look {
    /// 节点上的文字颜色。
    text: Color,
    /// 连线上的文字颜色：比节点上的文字浅，区分连线说明与节点内容的主次。
    soft_text: Color,
    /// 节点的填充色。
    fill: Color,
    /// 节点的边框色。
    border: Color,
    /// 连线和箭头的颜色。
    line: Color,
    /// 子图外的连线文字的底色，即预览的背景色；穿过文字的连线在文字处被底色遮住。
    ///
    /// 子图内的连线文字使用子图的填充色 `group_fill`。
    label_bg: Color,
    /// 子图框的填充色；不透明，子图内连线文字的底色与之相同。
    group_fill: Color,
    /// 子图框的边框色。
    group_border: Color,
    /// 子图标题的颜色。
    title: Color,
    /// 时序图注释框的填充色。
    note_fill: Color,
    /// 时序图注释框的边框色。
    note_border: Color,
    /// 类图、实体关系图方框标题栏的填充色，比节点的填充色略深。
    head: Color,
    /// 是否为深色主题：图中按色相区分的颜色（分支、任务状态）与背景混合时据此调整比例。
    dark: bool,
}

impl Look {
    /// 由预览的背景色 `page`、正文颜色 `text`、强调色 `accent` 生成一组颜色；`dark` 表示深色主题。
    ///
    /// 混合比例取自调参：节点底色略深于背景，边框、连线依次加深；深色主题下同样比例的节点底色
    /// 与背景几乎无法区分，因此节点底色和子图填充色混入更多正文颜色。
    fn new(text: Color, page: Color, accent: Color, dark: bool) -> Look {
        let ink = |t: f32| mix(page, text, t);
        Look {
            text,
            soft_text: ink(0.72),
            fill: ink(if dark { 0.07 } else { 0.045 }),
            border: ink(0.2),
            line: ink(0.48),
            label_bg: page,
            group_fill: ink(if dark { 0.03 } else { 0.018 }),
            group_border: ink(0.14),
            title: accent,
            note_fill: mix(page, NOTE_TINT, if dark { 0.16 } else { 0.2 }),
            note_border: mix(page, NOTE_TINT, 0.55),
            head: ink(if dark { 0.13 } else { 0.09 }),
            dark,
        }
    }
}

impl Look {
    /// 连线文字的底色：文字在子图内时为子图的填充色，否则为预览的背景色。
    fn behind(&self, route: &Route) -> Color {
        if route.group.is_some() {
            self.group_fill
        } else {
            self.label_bg
        }
    }
}

/// 返回 `a` 与 `b` 按 `t`（0–1，0 为 `a`）线性混合的不透明颜色。
fn mix(a: Color, b: Color, t: f32) -> Color {
    let at = |x: f32, y: f32| x + (y - x) * t;
    Color {
        r: at(a.r, b.r),
        g: at(a.g, b.g),
        b: at(a.b, b.b),
        a: 1.0,
    }
}

/// 排好的一段图里的文字：单独排好的段和它的宽高（逻辑像素）。
pub(crate) struct Label {
    /// 单独排好的这一段（坐标从 0 起）。
    piece: Piece,
    /// 段的宽度（逻辑像素）。
    w: f32,
    /// 段的高度（逻辑像素）。
    h: f32,
}

impl Cx<'_, '_> {
    /// 排一张 mermaid 图，左上角在 `(x, y)`、最宽 `width`（逻辑像素），水平居中。
    ///
    /// 认不出、不会画的图返回 `None`（按代码块显示）。
    pub(crate) fn diagram(
        &mut self,
        source: &str,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Vec<Item>,
    ) -> Option<Laid> {
        let diagram = supported(source)?;
        let look = self.look(env);
        let (text_len, chars) = (self.text.len(), self.chars);
        let mut size = self.base * TEXT_RATIO;
        let mut wrap = 1.0;
        let mut items = Vec::new();
        // 图宽于可用宽度时，先按 `NARROW_WRAP` 折窄文字重排，仍然过宽再按超出的比例缩小字号
        // 重排，使整张图无需横向滚动即可完整显示。文字折行不随字号严格按比例变化，一次缩小后
        // 可能仍然过宽。宽度按实际绘制范围计算：子图、连线避让处可能使图左侧留有空白，放置时
        // 去掉这段空白。
        let mut bounds = (0.0, 0.0);
        let mut h = 0.0;
        // 最多折窄 1 次、缩小字号 5 次：通常 1 至 2 次即可放下；5 次后仍然过宽时不再缩小，超出的
        // 部分按原样绘制，避免字号缩小到无法辨认。
        for _ in 0..6 {
            items.clear();
            self.text.truncate(text_len);
            self.chars = chars;
            (bounds, h) = self.diagram_pass(&diagram, size, wrap, look, &mut items);
            let w = bounds.1 - bounds.0;
            if w <= width {
                break;
            }
            if wrap > NARROW_WRAP
                && matches!(
                    diagram,
                    Diagram::Flow(_) | Diagram::Class(_) | Diagram::Er(_)
                )
            {
                wrap = NARROW_WRAP;
                continue;
            }
            let next = size * width / w * 0.98;
            if next >= size {
                break;
            }
            size = next;
        }
        let w = bounds.1 - bounds.0;
        let dx = x + ((width - w) / 2.0).max(0.0) - bounds.0;
        out.extend(items.iter().map(|item| item.moved(dx, y, 0)));
        // 记录图所在的一栏：鼠标悬停时，栏的右上角显示全屏按钮和复制源码按钮。
        out.push(Item::Code {
            rect: Rect::new(x, y, width.max(w), h),
            text: source.into(),
            diagram: true,
        });
        self.end_paragraph();
        Some(Laid {
            height: h,
            first_baseline: None,
        })
    }

    /// 为全屏查看排版一张图：字号为正文中图的默认字号的 `zoom` 倍，`narrow` 为真时按
    /// `NARROW_WRAP` 折窄文字，不再缩小字号。
    ///
    /// 返回各项（左上角在原点）和宽高（逻辑像素）；无法识别或不支持绘制的图返回 `None`。
    pub fn diagram_alone(
        &mut self,
        source: &str,
        zoom: f32,
        narrow: bool,
        env: Env,
    ) -> Option<(Vec<Item>, f32, f32)> {
        let diagram = supported(source)?;
        let look = self.look(env);
        let mut items = Vec::new();
        let size = self.base * TEXT_RATIO * zoom;
        let wrap = if narrow { NARROW_WRAP } else { 1.0 };
        let ((left, right), h) = self.diagram_pass(&diagram, size, wrap, look, &mut items);
        let items = items.iter().map(|item| item.moved(-left, 0.0, 0)).collect();
        Some((items, right - left, h))
    }

    /// 按字号 `size`、折行比例 `wrap` 排版一次，各项写入 `items`；返回实际绘制的左右边界和
    /// 高度（逻辑像素）。
    fn diagram_pass(
        &mut self,
        diagram: &Diagram,
        size: f32,
        wrap: f32,
        look: Look,
        items: &mut Vec<Item>,
    ) -> ((f32, f32), f32) {
        let laid = match diagram {
            Diagram::Flow(flow) => self.flowchart(flow, size, wrap, look, items),
            Diagram::Sequence(seq) => self.sequence(seq, size, look, items),
            Diagram::Pie(pie) => self.pie(pie, size, look, items),
            Diagram::Class(class) => self.class(class, size, wrap, look, items),
            Diagram::Er(er) => self.er(er, size, wrap, look, items),
            Diagram::Mindmap(map) => self.mindmap(map, size, look, items),
            Diagram::Git(git) => self.git(git, size, look, items),
            Diagram::Gantt(gantt) => self.gantt(gantt, size, look, items),
            Diagram::Unsupported(_) => (0.0, 0.0),
        };
        let bounds = items
            .iter()
            .map(Item::span_x)
            .fold((f32::MAX, f32::MIN), |(a, b), (l, r)| (a.min(l), b.max(r)));
        let bounds = if bounds.0 > bounds.1 {
            (0.0, laid.0)
        } else {
            bounds
        };
        (bounds, laid.1)
    }

    /// 由正文颜色 `env.color` 和当前配色生成图的颜色。
    fn look(&self, env: Env) -> Look {
        Look::new(env.color, self.surface, self.accent, self.dark)
    }

    /// 单独排一段图里的文字（`\n` 分行），字号 `size`，最宽 `max`，各行居中。
    fn label(&mut self, text: &str, size: f32, color: Color, max: f32) -> Label {
        self.styled_label(text, size, color, max, false)
    }

    /// 同 [`Cx::label`]，`bold` 为真时使用粗体。
    fn styled_label(&mut self, text: &str, size: f32, color: Color, max: f32, bold: bool) -> Label {
        let para = Para {
            size,
            bold,
            color,
            line: 1.35,
            align: Align::Center,
        };
        let inline = Inline {
            text: text.to_owned(),
            runs: vec![Run {
                end: text.len(),
                style: Style::default(),
            }],
            ..Inline::default()
        };
        if text.is_empty() {
            let piece = self.isolated(|_, _| Laid {
                height: 0.0,
                first_baseline: None,
            });
            return Label {
                piece,
                w: 0.0,
                h: 0.0,
            };
        }
        let (word, line) = self.measure(&inline, para);
        let w = line.min(max).max(word.min(max)).ceil();
        let piece = self.isolated(|cx, out| cx.flow_at(&inline, 0.0, 0.0, w, para, out));
        let h = piece.height;
        Label { piece, w, h }
    }

    /// 把排好的文字放在中心 `(cx, cy)`。
    fn put_label(&mut self, label: &Label, cx: f32, cy: f32, out: &mut Vec<Item>) {
        self.paste(&label.piece, cx - label.w / 2.0, cy - label.h / 2.0, 0, out);
    }

    /// 在 `(cx, cy)` 写连线上的文字 `label`，文字下面垫一块不透明的底色 `bg`：遮住从文字下方
    /// 穿过的连线，底色左右各比文字宽出一段，连线在文字两侧断开。
    fn edge_label(
        &mut self,
        label: &Label,
        (cx, cy): (f32, f32),
        s: f32,
        bg: Color,
        out: &mut Vec<Item>,
    ) {
        let (px, py) = (s * 0.3, s * 0.1);
        out.push(Item::Fill {
            rect: Rect::new(
                cx - label.w / 2.0 - px,
                cy - label.h / 2.0 - py,
                label.w + px * 2.0,
                label.h + py * 2.0,
            ),
            color: bg,
            radius: 2.0,
        });
        self.put_label(label, cx, cy, out);
    }

    /// 在子图框的左上角写标题 `titles`；`boxes`、`order` 同 [`group_frames`] 的返回值。
    fn group_titles(
        &mut self,
        boxes: &[Option<Rect>],
        order: &[usize],
        titles: &[Label],
        s: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) {
        for &g in order {
            let (Some(rect), title) = (boxes[g], &titles[g]) else {
                continue;
            };
            if title.w > 0.0 {
                // 标题放在框的左上角：连线多从框边的中部进出，放在角上可避免与箭头重叠；
                // 框的宽度不足以容纳标题和两侧边距时居中。
                let inset = s * 0.6;
                let cx = if title.w + inset * 2.0 <= rect.w {
                    rect.x + inset + title.w / 2.0
                } else {
                    rect.x + rect.w / 2.0
                };
                let cy = rect.y + s * 0.2 + title.h / 2.0;
                out.push(Item::Fill {
                    rect: Rect::new(
                        cx - title.w / 2.0 - s * 0.2,
                        cy - title.h / 2.0,
                        title.w + s * 0.4,
                        title.h,
                    ),
                    color: look.group_fill,
                    radius: 2.0,
                });
                self.put_label(title, cx, cy, out);
            }
        }
    }

    /// 类图、实体关系图共用的排布：按分层布局排开宽高为 `sizes` 的方框，画出分组的框、连线和
    /// 连线文字，返回各方框的位置和图的宽高（逻辑像素）。
    ///
    /// `flow` 只用到连线的起止、文字和分组，节点的形状均按矩形处理；连线本身由 `draw` 画，参数
    /// 为连线的序号和平滑后经过的点。方框由调用方在之后画出，盖住连线的端点。
    #[allow(clippy::too_many_arguments)]
    fn boxed(
        &mut self,
        flow: &Flow,
        sizes: &[(f32, f32)],
        s: f32,
        wrap: f32,
        look: Look,
        out: &mut Vec<Item>,
        mut draw: impl FnMut(&mut Self, usize, &[(f32, f32)], &mut Vec<Item>),
    ) -> (Vec<Rect>, f32, f32) {
        let edge_labels: Vec<Label> = flow
            .edges
            .iter()
            .map(|e| self.label(&e.label, s * 0.85, look.soft_text, s * 10.0 * wrap.max(0.8)))
            .collect();
        let titles: Vec<Label> = flow
            .groups
            .iter()
            .map(|g| self.label(&g.title, s * 0.85, look.title, s * 16.0))
            .collect();
        let heights: Vec<f32> = titles.iter().map(|t| t.h).collect();
        let plan = layout(flow, sizes, &edge_labels, &heights, s, flow.dir);
        let rects: Vec<Rect> = plan
            .centers
            .iter()
            .zip(sizes)
            .map(|(&c, &(w, h))| {
                let (cx, cy) = plan.place(c, flow.dir);
                Rect::new(cx - w / 2.0, cy - h / 2.0, w, h)
            })
            .collect();
        let (boxes, outer_first) = group_frames(&plan, flow.dir, look, out);
        for (k, edge) in flow.edges.iter().enumerate() {
            let Some(route) = plan.routes[k].as_ref() else {
                continue;
            };
            let ends = [edge.from, edge.to].map(|i| (Shape::Rect, rects[i]));
            let (points, label_at) = curve(&plan, route, flow.dir, ends, edge.from == edge.to, s);
            if points.len() < 2 {
                continue;
            }
            draw(self, k, &points, out);
            if !edge.label.is_empty() {
                let at = label_at.unwrap_or(points[points.len() / 2]);
                self.edge_label(&edge_labels[k], at, s, look.behind(route), out);
            }
        }
        self.group_titles(&boxes, &outer_first, &titles, s, look, out);
        let (w, h) = if matches!(flow.dir, Dir::Down | Dir::Up) {
            (plan.cross, plan.along)
        } else {
            (plan.along, plan.cross)
        };
        (rects, w, h)
    }

    /// 排版流程图，返回宽高（逻辑像素）；`wrap` 为文字折行宽度相对默认宽度的比例（见
    /// `NARROW_WRAP`）。
    fn flowchart(
        &mut self,
        f: &Flow,
        s: f32,
        wrap: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) -> (f32, f32) {
        let vertical = matches!(f.dir, Dir::Down | Dir::Up);
        let labels: Vec<Label> = f
            .nodes
            .iter()
            .map(|n| self.label(&n.label, s, look.text, s * 14.0 * wrap))
            .collect();
        let sizes: Vec<(f32, f32)> = f
            .nodes
            .iter()
            .zip(&labels)
            .map(|(n, l)| shape_size(n.shape, l.w, l.h, s, vertical))
            .collect();
        let edge_labels: Vec<Label> = f
            .edges
            .iter()
            .map(|e| self.label(&e.label, s * 0.85, look.soft_text, s * 10.0 * wrap.max(0.8)))
            .collect();
        let titles: Vec<Label> = f
            .groups
            .iter()
            .map(|g| self.label(&g.title, s * 0.85, look.title, s * 16.0))
            .collect();
        let heights: Vec<f32> = titles.iter().map(|t| t.h).collect();
        let plan = layout(f, &sizes, &edge_labels, &heights, s, f.dir);
        let place = |p: (f32, f32)| plan.place(p, f.dir);
        let (w, h) = if vertical {
            (plan.cross, plan.along)
        } else {
            (plan.along, plan.cross)
        };
        let rect_of = |i: usize| {
            let (cx, cy) = place(plan.centers[i]);
            let (nw, nh) = sizes[i];
            Rect::new(cx - nw / 2.0, cy - nh / 2.0, nw, nh)
        };

        // 子图的框，外面的先画；标题等连线画完再写，不被线压住。
        let (boxes, outer_first) = group_frames(&plan, f.dir, look, out);
        for (k, edge) in f.edges.iter().enumerate() {
            let Some(route) = plan.routes[k].as_ref() else {
                continue;
            };
            let ends = [edge.from, edge.to].map(|i| (f.nodes[i].shape, rect_of(i)));
            let (points, label_at) = curve(&plan, route, f.dir, ends, edge.from == edge.to, s);
            self.edge(edge, &points, s, look, out);
            if !edge.label.is_empty() {
                let at = label_at.unwrap_or(points[points.len() / 2]);
                self.edge_label(&edge_labels[k], at, s, look.behind(route), out);
            }
        }
        self.group_titles(&boxes, &outer_first, &titles, s, look, out);

        // 节点画在连线上面。
        for (i, node) in f.nodes.iter().enumerate() {
            let r = rect_of(i);
            draw_shape(node.shape, r, look, out);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let dy = if node.shape == Shape::Cylinder {
                cylinder_ry(r.w) / 2.0
            } else {
                0.0
            };
            self.put_label(&labels[i], cx, cy + dy, out);
        }
        bounds(out, w, h)
    }

    /// 画一条连线 `points`（已经平滑过）和两头的线头。
    fn edge(
        &mut self,
        edge: &Edge,
        points: &[(f32, f32)],
        s: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) {
        if edge.line == Line::Hidden || points.len() < 2 {
            return;
        }
        let width = match edge.line {
            Line::Thick => EDGE_THICK,
            _ => EDGE_WIDTH,
        };
        let mut points = points.to_vec();
        let len = s * 0.6;
        for (head, at_end) in [(edge.head, true), (edge.tail, false)] {
            if head == Head::None {
                continue;
            }
            let n = points.len();
            let (tip, from) = if at_end {
                (points[n - 1], points[n - 2])
            } else {
                (points[0], points[1])
            };
            let base = arrow_head(head, tip, from, len, look.line, out);
            if at_end {
                points[n - 1] = base;
            } else {
                points[0] = base;
            }
        }
        let strokes = if edge.line == Line::Dotted {
            dashed(&points, s * 0.25, s * 0.25)
        } else {
            polyline(&points)
        };
        if !strokes.is_empty() {
            out.push(Item::path(strokes, width, look.line));
        }
    }
}

/// 画分层布局中各子图的框（填色和边框），外面的先画。
///
/// 返回各框的位置（框内没有节点时为 `None`，与 [`Plan::boxes`] 一一对应）和由外到内的顺序。
fn group_frames(
    plan: &Plan,
    dir: Dir,
    look: Look,
    out: &mut Vec<Item>,
) -> (Vec<Option<Rect>>, Vec<usize>) {
    let boxes: Vec<Option<Rect>> = plan
        .boxes
        .iter()
        .map(|b| {
            (b[0] <= b[2]).then(|| {
                let (x0, y0) = plan.place((b[0], b[1]), dir);
                let (x1, y1) = plan.place((b[2], b[3]), dir);
                Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
            })
        })
        .collect();
    let mut outer_first: Vec<usize> = (0..boxes.len()).collect();
    outer_first.sort_by(|&a, &b| {
        let area = |g: usize| boxes[g].map_or(0.0, |r| r.w * r.h);
        area(b).total_cmp(&area(a))
    });
    for &g in &outer_first {
        let Some(rect) = boxes[g] else { continue };
        out.push(Item::Fill {
            rect,
            color: look.group_fill,
            radius: GROUP_RADIUS,
        });
        out.push(Item::path(
            outline(rect, GROUP_RADIUS),
            1.0,
            look.group_border,
        ));
    }
    (boxes, outer_first)
}

/// 屏幕上的一点（逻辑像素）。
type Point = (f32, f32);

/// 把布局排好的一条连线换成屏幕上的平滑曲线，返回曲线经过的点和连线文字的中心（布局没有为
/// 文字留位置时为 `None`）。
///
/// `ends` 是起点、终点节点的形状和位置，曲线两端裁到形状的边缘；`to_self` 为真时画成从节点
/// 右侧绕出又绕回的环。
fn curve(
    plan: &Plan,
    route: &Route,
    dir: Dir,
    ends: [(Shape, Rect); 2],
    to_self: bool,
    s: f32,
) -> (Vec<Point>, Option<Point>) {
    let mut points: Vec<(f32, f32)> = route.points.iter().map(|&p| plan.place(p, dir)).collect();
    if to_self {
        points = self_loop(ends[0].1, s);
    } else if points.len() >= 2 {
        let n = points.len();
        points[0] = clip(ends[0].0, ends[0].1, points[1]);
        points[n - 1] = clip(ends[1].0, ends[1].1, points[n - 2]);
    }
    // 布局为连线文字预留了位置，曲线必须经过该点：在该点分成两段分别平滑，两段的接缝被文字的
    // 底色遮住。
    let label_at = route.label.map(|p| plan.place(p, dir));
    let split = label_at.and_then(|p| {
        let near = |q: &(f32, f32)| (q.0 - p.0).abs() < 0.01 && (q.1 - p.1).abs() < 0.01;
        (1..points.len().saturating_sub(1)).find(|&i| near(&points[i]))
    });
    let points = match split {
        Some(i) => {
            let mut curve = smooth(&points[..=i]);
            curve.extend(smooth(&points[i..]).into_iter().skip(1));
            curve
        }
        None => smooth(&points),
    };
    (points, label_at)
}

/// 所有项的外框（不小于 `(w, h)`），左上角挪到原点，返回宽高（逻辑像素）。
///
/// 外框四周各留 2 逻辑像素，避免线宽被裁掉一半。
fn bounds(items: &mut [Item], w: f32, h: f32) -> (f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (0f32, 0f32, w, h);
    for item in items.iter() {
        let r = match item {
            Item::Fill { rect, .. }
            | Item::Path { rect, .. }
            | Item::Polygon { rect, .. }
            | Item::Link { rect, .. } => *rect,
            Item::Text {
                x,
                baseline,
                size,
                span,
                ..
            } => {
                let w = span.as_ref().map_or(0.0, |s| s.advances.iter().sum());
                Rect::new(*x, baseline - size, w, size * 1.3)
            }
            _ => continue,
        };
        (x0, y0, x1, y1) = (
            x0.min(r.x),
            y0.min(r.y),
            x1.max(r.right()),
            y1.max(r.bottom()),
        );
    }
    let pad = 2.0;
    for item in items.iter_mut() {
        *item = item.moved(pad - x0, pad - y0, 0);
    }
    (x1 - x0 + pad * 2.0, y1 - y0 + pad * 2.0)
}

/// 返回装下 `tw × th` 的文字（逻辑像素）所需的形状大小（宽、高）。
///
/// `vertical` 是上下走向的图（横条横着放）。
fn shape_size(shape: Shape, tw: f32, th: f32, s: f32, vertical: bool) -> (f32, f32) {
    let (px, py) = (s * 1.0, s * 0.6);
    let (w, h) = (tw + px * 2.0, th + py * 2.0);
    match shape {
        Shape::Rect | Shape::Round => (w, h),
        Shape::Stadium => (w + h / 2.0, h),
        Shape::Subroutine => (w + s, h),
        Shape::Asym => (w + h / 4.0, h),
        Shape::Cylinder => (w, h + cylinder_ry(w) * 2.0),
        Shape::Circle => {
            let d = tw.max(th) + px * 1.2;
            (d, d)
        }
        Shape::DoubleCircle => {
            let d = tw.max(th) + px * 1.2 + s * 0.6;
            (d, d)
        }
        Shape::Diamond => {
            let side = (tw + th) * 0.8 + px * 2.0;
            (side.max(w * 1.3), (side * 0.75).max(h * 1.5))
        }
        Shape::Hexagon => (w + h / 2.0, h),
        Shape::Lean | Shape::LeanLeft | Shape::Trapezoid | Shape::TrapezoidDown => (w + h * 0.6, h),
        Shape::Start | Shape::End => (s, s),
        Shape::Bar if vertical => (s * 5.0, s * 0.5),
        Shape::Bar => (s * 0.5, s * 5.0),
    }
}

/// 圆柱上下椭圆的半高（逻辑像素）：按宽度的 6% 取，限制在 4–10 之间，窄节点不会太扁、
/// 宽节点不会太鼓。
fn cylinder_ry(w: f32) -> f32 {
    (w * 0.06).clamp(4.0, 10.0)
}

/// 矩形 `r` 的四条边（`radius` 为圆角）。
fn outline(r: Rect, radius: f32) -> Vec<Stroke> {
    if radius > 0.0 {
        rounded_outline(r.x, r.y, r.w, r.h, radius).to_vec()
    } else {
        closed(&[
            (r.x, r.y),
            (r.right(), r.y),
            (r.right(), r.bottom()),
            (r.x, r.bottom()),
        ])
    }
}

/// 首尾相连的折线。
fn closed(points: &[(f32, f32)]) -> Vec<Stroke> {
    let mut all = points.to_vec();
    all.extend(points.first());
    polyline(&all)
}

/// 中心 `(cx, cy)`、半轴 `rx`、`ry` 的椭圆上从角度 `from` 到 `to` 的点。
fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|k| {
            let a = from + (to - from) * k as f32 / n as f32;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// 画节点的形状（填色和边框）。
///
/// 边框线宽为 1 逻辑像素。
fn draw_shape(shape: Shape, r: Rect, look: Look, out: &mut Vec<Item>) {
    let (fill, border) = (look.fill, look.border);
    let width = 1.0;
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let (cx, cy) = (x0 + r.w / 2.0, y0 + r.h / 2.0);
    let polygon = |points: Vec<(f32, f32)>, out: &mut Vec<Item>| {
        out.push(Item::path(closed(&points), width, border));
        out.insert(out.len() - 1, Item::polygon(points, fill));
    };
    match shape {
        Shape::Rect | Shape::Round | Shape::Subroutine | Shape::Stadium => {
            let radius = match shape {
                Shape::Round => ROUND_RADIUS.min(r.h / 2.0),
                Shape::Stadium => r.h / 2.0,
                _ => NODE_RADIUS,
            };
            out.push(Item::Fill {
                rect: r,
                color: fill,
                radius,
            });
            out.push(Item::path(outline(r, radius), width, border));
            if shape == Shape::Subroutine {
                let inset = r.h.min(16.0) / 2.0;
                out.push(Item::path(
                    vec![
                        Stroke::Line(x0 + inset, y0, x0 + inset, y1),
                        Stroke::Line(x1 - inset, y0, x1 - inset, y1),
                    ],
                    width,
                    border,
                ));
            }
        }
        Shape::Circle | Shape::DoubleCircle => {
            let rad = r.w / 2.0;
            out.push(Item::Fill {
                rect: r,
                color: fill,
                radius: rad,
            });
            let mut strokes = vec![Stroke::Circle(cx, cy, rad)];
            if shape == Shape::DoubleCircle {
                strokes.push(Stroke::Circle(cx, cy, rad - 4.0));
            }
            out.push(Item::path(strokes, width, border));
        }
        Shape::Diamond => polygon(vec![(cx, y0), (x1, cy), (cx, y1), (x0, cy)], out),
        Shape::Hexagon => {
            let d = r.h / 4.0;
            polygon(
                vec![
                    (x0 + d, y0),
                    (x1 - d, y0),
                    (x1, cy),
                    (x1 - d, y1),
                    (x0 + d, y1),
                    (x0, cy),
                ],
                out,
            );
        }
        Shape::Lean | Shape::LeanLeft | Shape::Trapezoid | Shape::TrapezoidDown => {
            let d = r.h * 0.3;
            let (tl, tr, br, bl) = match shape {
                Shape::Lean => (d, 0.0, -d, 0.0),
                Shape::LeanLeft => (0.0, -d, 0.0, d),
                Shape::Trapezoid => (d, -d, 0.0, 0.0),
                _ => (0.0, 0.0, -d, d),
            };
            // 每个角往里收多少：负的是右边往左收。
            let (tl, tr, br, bl) = (
                (x0 + tl.max(0.0), y0),
                (x1 + tr.min(0.0), y0),
                (x1 + br.min(0.0), y1),
                (x0 + bl.max(0.0), y1),
            );
            polygon(vec![tl, tr, br, bl], out);
        }
        Shape::Asym => {
            let n = r.h / 4.0;
            // 左边有个往里的缺口，不是凸的：分成一个矩形和两个三角形填色。
            out.push(Item::Fill {
                rect: Rect::new(x0 + n, y0, r.w - n, r.h),
                color: fill,
                radius: 0.0,
            });
            out.push(Item::polygon(
                vec![(x0, y0), (x0 + n, y0), (x0 + n, cy)],
                fill,
            ));
            out.push(Item::polygon(
                vec![(x0, y1), (x0 + n, cy), (x0 + n, y1)],
                fill,
            ));
            out.push(Item::path(
                closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0 + n, cy)]),
                width,
                border,
            ));
        }
        Shape::Cylinder => {
            let ry = cylinder_ry(r.w);
            let rx = r.w / 2.0;
            let (top, bottom) = (y0 + ry, y1 - ry);
            use std::f32::consts::{PI, TAU};
            out.push(Item::Fill {
                rect: Rect::new(x0, top, r.w, bottom - top),
                color: fill,
                radius: 0.0,
            });
            out.push(Item::polygon(ellipse(cx, top, rx, ry, 0.0, TAU, 32), fill));
            out.push(Item::polygon(
                ellipse(cx, bottom, rx, ry, 0.0, TAU, 32),
                fill,
            ));
            let mut strokes = polyline(&ellipse(cx, top, rx, ry, 0.0, TAU, 32));
            strokes.extend(polyline(&ellipse(cx, bottom, rx, ry, 0.0, PI, 16)));
            strokes.push(Stroke::Line(x0, top, x0, bottom));
            strokes.push(Stroke::Line(x1, top, x1, bottom));
            out.push(Item::path(strokes, width, border));
        }
        Shape::Start => out.push(Item::Fill {
            rect: r,
            color: look.line,
            radius: r.w / 2.0,
        }),
        Shape::End => {
            out.push(Item::path(
                vec![Stroke::Circle(cx, cy, r.w / 2.0)],
                width,
                look.line,
            ));
            let inner = r.inset(r.w * 0.22, r.h * 0.22);
            out.push(Item::Fill {
                rect: inner,
                color: look.line,
                radius: inner.w / 2.0,
            });
        }
        Shape::Bar => out.push(Item::Fill {
            rect: r,
            color: look.line,
            radius: 1.0,
        }),
    }
}

/// 从形状中心往 `toward` 的方向，和形状边缘的交点。
fn clip(shape: Shape, r: Rect, toward: (f32, f32)) -> (f32, f32) {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return (cx, cy);
    }
    let t = match shape {
        Shape::Diamond => 1.0 / (dx.abs() / hw + dy.abs() / hh),
        Shape::Circle | Shape::DoubleCircle | Shape::Start | Shape::End => {
            hw / (dx * dx + dy * dy).sqrt()
        }
        _ => {
            let tx = if dx.abs() > 1e-3 {
                hw / dx.abs()
            } else {
                f32::MAX
            };
            let ty = if dy.abs() > 1e-3 {
                hh / dy.abs()
            } else {
                f32::MAX
            };
            tx.min(ty)
        }
    };
    let t = t.min(1.0);
    (cx + dx * t, cy + dy * t)
}

/// 连到自己的线：从右边出去绕一圈回来。
fn self_loop(r: Rect, s: f32) -> Vec<(f32, f32)> {
    let (x, cy) = (r.right(), r.y + r.h / 2.0);
    let d = r.h / 4.0;
    vec![
        (x, cy - d),
        (x + s * 1.5, cy - d),
        (x + s * 1.5, cy + d),
        (x, cy + d),
    ]
}

/// 把连线经过的折线点转换为平滑曲线：以各点为控制点的均匀三次 B 样条，首尾两点位于曲线上。
///
/// 曲线不经过中间的控制点，经过多个点的长连线因此不会在每个点处急转。每段曲线转换为三次
/// 贝塞尔曲线后取 8 个点，以折线绘制。
fn smooth(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    type P = (f32, f32);
    fn cubic(out: &mut Vec<P>, p0: P, c1: P, c2: P, p1: P) {
        for k in 1..=8 {
            let t = k as f32 / 8.0;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push((
                a * p0.0 + b * c1.0 + c * c2.0 + d * p1.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * p1.1,
            ));
        }
    }
    let mut out = vec![points[0]];
    let lerp = |a: (f32, f32), b: (f32, f32), wa: f32, wb: f32| {
        let w = wa + wb;
        ((a.0 * wa + b.0 * wb) / w, (a.1 * wa + b.1 * wb) / w)
    };
    // 以相邻三点 x0、x1、x 确定一段：起点为上一段的终点，控制点位于 x0 与 x1 的三等分处，终点为
    // (x0 + 4·x1 + x) / 6。第一段之前以直线连到 (5·p0 + p1) / 6，最后一段的终点为最后一点。
    let first = lerp(points[0], points[1], 5.0, 1.0);
    out.push(first);
    let mut at = first;
    for w in points.windows(3) {
        let (x0, x1, x) = (w[0], w[1], w[2]);
        let end = (
            (x0.0 + 4.0 * x1.0 + x.0) / 6.0,
            (x0.1 + 4.0 * x1.1 + x.1) / 6.0,
        );
        cubic(
            &mut out,
            at,
            lerp(x0, x1, 2.0, 1.0),
            lerp(x0, x1, 1.0, 2.0),
            end,
        );
        at = end;
    }
    let n = points.len();
    let (x0, x1) = (points[n - 2], points[n - 1]);
    cubic(
        &mut out,
        at,
        lerp(x0, x1, 2.0, 1.0),
        lerp(x0, x1, 1.0, 2.0),
        x1,
    );
    // 去除重复的点：箭头的方向由最后两点确定，两点重合时方向无法确定。
    out.dedup_by(|a, b| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
    out
}

/// 画线头（尖在 `tip`，线从 `from` 那边来），返回线该画到哪里为止。
///
/// `len` 是线头的长度（逻辑像素）。
pub(crate) fn arrow_head(
    head: Head,
    tip: (f32, f32),
    from: (f32, f32),
    len: f32,
    color: Color,
    out: &mut Vec<Item>,
) -> (f32, f32) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let d = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / d, dy / d);
    let back = |k: f32| (tip.0 - ux * len * k, tip.1 - uy * len * k);
    match head {
        Head::Arrow => {
            let base = back(1.0);
            let (px, py) = (-uy * len * 0.45, ux * len * 0.45);
            out.push(Item::polygon(
                vec![tip, (base.0 + px, base.1 + py), (base.0 - px, base.1 - py)],
                color,
            ));
            back(0.8)
        }
        Head::Circle => {
            let r = len * 0.35;
            let c = back(0.35);
            out.push(Item::Fill {
                rect: Rect::new(c.0 - r, c.1 - r, r * 2.0, r * 2.0),
                color,
                radius: r,
            });
            back(0.7)
        }
        Head::Cross => {
            let c = back(0.5);
            let k = len * 0.35;
            out.push(Item::path(
                vec![
                    Stroke::Line(c.0 - k, c.1 - k, c.0 + k, c.1 + k),
                    Stroke::Line(c.0 - k, c.1 + k, c.0 + k, c.1 - k),
                ],
                len * 0.15,
                color,
            ));
            c
        }
        Head::None => tip,
    }
}

/// 虚线：沿折线画 `on` 长、空 `off` 长（都是逻辑像素）。
pub(crate) fn dashed(points: &[(f32, f32)], on: f32, off: f32) -> Vec<Stroke> {
    let mut out = Vec::new();
    let mut drawing = true;
    let mut left = on;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let mut at = 0.0;
        while at < len {
            let step = left.min(len - at);
            if drawing {
                let p = |t: f32| (a.0 + (b.0 - a.0) * t / len, a.1 + (b.1 - a.1) * t / len);
                let (p0, p1) = (p(at), p(at + step));
                out.push(Stroke::Line(p0.0, p0.1, p1.0, p1.1));
            }
            at += step;
            left -= step;
            if left <= 1e-3 {
                drawing = !drawing;
                left = if drawing { on } else { off };
            }
        }
    }
    out
}
