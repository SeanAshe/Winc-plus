//! 公式的排版：把 [`math::parse`] 解析出的树排成一块（宽、基线上方的高、下方的深和要画的
//! 项）。
//!
//! 按 TeX 的排版规则简化：符号按类别（普通、运算、关系……）在中间留不同宽的空，上下标、
//! 分式按「数学轴」（减号的高度）定位置，括号、根号按内容的墨迹高度变高。字用数学字体
//! （`FontKind::Math`，由系统提供），单个字母换成 Unicode 数学斜体字母。
//!
//! 坐标和尺寸都是逻辑像素。

use winc_preview::markdown::math::{self, Accent, Class, Font, Grid, Node};
use winc_render::{Color, Faux, FontKind, Rect, Stroke};

use super::{Cx, Item};

mod support;

pub(crate) use support::polyline;
use support::{drawn_delimiter, italic_letter, letter, named_color, single_glyph, spacing};

/// 括号用放大的字绘制时的最大放大倍数（无量纲，相对于字的自然高度）；更高的括号画出来。
const GLYPH_STRETCH: f32 = 2.2;

/// 排好的一块公式：项的坐标以左端、基线为原点（y 向下）。
#[derive(Default)]
pub(crate) struct MathBox {
    /// 宽度（逻辑像素）。
    pub(crate) w: f32,
    /// 基线上方多高。
    pub(crate) asc: f32,
    /// 基线下方多深。
    pub(crate) desc: f32,
    /// 要画的项，坐标以左端、基线为原点。
    pub(crate) items: Vec<Item>,
}

/// 公式字号的放大系数（无量纲，经验值）：数学字体的字面比界面字体小，放大后小写字母的高度
/// 与正文接近。
const SCALE: f32 = 1.1;

/// 公式里的大小层级（TeX 的 display、text、script、scriptscript）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Display,
    Text,
    Script,
    Tiny,
}

impl Level {
    fn scale(self) -> f32 {
        match self {
            Level::Display | Level::Text => 1.0,
            Level::Script => 0.7,
            Level::Tiny => 0.5,
        }
    }

    /// 上下标用的层级。
    fn script(self) -> Level {
        match self {
            Level::Display | Level::Text => Level::Script,
            _ => Level::Tiny,
        }
    }

    /// 分子、分母用的层级。
    fn frac(self) -> Level {
        match self {
            Level::Display => Level::Text,
            Level::Text => Level::Script,
            _ => Level::Tiny,
        }
    }
}

/// 排一个节点时继承的条件：大小层级和文字颜色。
#[derive(Clone, Copy)]
struct Env {
    /// 当前的大小层级。
    level: Level,
    /// 当前文字的颜色。
    color: Color,
}

/// 排好的一个节点和它左右两端算作什么类别（定和旁边留多宽）。
struct Laid {
    /// 排好的项与宽高。
    b: MathBox,
    /// 左右两端当作的类别。
    class: Class,
}

impl Laid {
    fn new(b: MathBox, class: Class) -> Laid {
        Laid { b, class }
    }
}

impl MathBox {
    /// 把 `other` 放到 `(x, y)`（它的基线左端），并撑大自己。
    fn put(&mut self, other: MathBox, x: f32, y: f32) {
        self.asc = self.asc.max(other.asc - y);
        self.desc = self.desc.max(other.desc + y);
        self.w = self.w.max(x + other.w);
        self.items
            .extend(other.items.iter().map(|item| item.moved(x, y, 0)));
    }
}

impl Cx<'_, '_> {
    /// 排公式 `tex`：正文字号 `size`，`display` 为真时是单独成行的（大运算符更大，上下限放在
    /// 正上下方）。
    pub(crate) fn math(&mut self, tex: &str, size: f32, display: bool, color: Color) -> MathBox {
        let node = math::parse(tex);
        let level = if display { Level::Display } else { Level::Text };
        let mut m = Math {
            cx: self,
            size: size * SCALE,
        };
        m.node(&node, Env { level, color }).b
    }
}

struct Math<'c, 'a, 'f> {
    cx: &'c mut Cx<'a, 'f>,
    /// 正文层级的字号（逻辑像素）。
    size: f32,
}

impl Math<'_, '_, '_> {
    /// 当前层级的字号（逻辑像素）。
    fn size(&self, level: Level) -> f32 {
        self.size * level.scale()
    }

    /// 数学轴（分数线、减号）在基线上方多高（逻辑像素）。
    ///
    /// 取字号（TeX 的 axis height 约 0.25 em）的 0.25 倍。
    fn axis(&self, level: Level) -> f32 {
        self.size(level) * 0.25
    }

    /// 分数线、根号的粗细（逻辑像素）：字号的 0.045 倍，至少 0.8（经验值，再细会发虚）。
    fn rule(&self, level: Level) -> f32 {
        (self.size(level) * 0.045).max(0.8)
    }

    /// 排一个节点。
    fn node(&mut self, node: &Node, env: Env) -> Laid {
        match node {
            Node::Sym(text, class, font) => Laid::new(self.glyphs(text, *font, env), *class),
            Node::Row(nodes) => Laid::new(self.hlist(nodes, env, false), Class::Ord),
            Node::Frac(num, den, bar, force) => self.frac(num, den, *bar, *force, env),
            Node::Sqrt(index, body) => self.sqrt(index.as_deref(), body, env),
            Node::Scripts(base, sup, sub) => {
                self.scripts(base, sup.as_deref(), sub.as_deref(), env)
            }
            Node::Op(text, big, _) => self.op(text, *big, env),
            Node::Fenced(left, body, right) => {
                let body = self.node(body, env).b;
                let target = self.fence_height(&body, env.level);
                let left = self.delimiter(left, target, env);
                let right = self.delimiter(right, target, env);
                let mut out = MathBox::default();
                let (lw, bw) = (left.w, body.w);
                out.put(left, 0.0, 0.0);
                out.put(body, lw, 0.0);
                out.put(right, lw + bw, 0.0);
                Laid::new(out, Class::Ord)
            }
            Node::Big(delim, level, class) => {
                let target = self.size(env.level) * [1.2, 1.8, 2.4, 3.0][*level as usize - 1];
                Laid::new(self.delimiter(delim, target, env), *class)
            }
            Node::Accent(accent, body) => self.accent(*accent, body, env),
            Node::Stack(base, over, under) => {
                let base = self.node(base, env).b;
                let s = self.size(env.level);
                Laid::new(
                    self.limits(base, over.as_deref(), under.as_deref(), s * 0.1, env),
                    Class::Ord,
                )
            }
            Node::Space(n) => {
                // `\!` 记成 0：往回退一点。
                let mu = if *n == 0 { -3.0 } else { f32::from(*n) };
                let b = MathBox {
                    w: self.size(env.level) * mu / 18.0,
                    ..MathBox::default()
                };
                Laid::new(b, Class::Ord)
            }
            Node::Grid(grid) => Laid::new(self.grid(grid, env), Class::Ord),
            Node::Style(display, body) => {
                let level = match (display, env.level) {
                    (true, _) => Level::Display,
                    (false, Level::Display) => Level::Text,
                    (false, level) => level,
                };
                self.node(body, Env { level, ..env })
            }
            Node::Color(name, body) => {
                let color = named_color(name).unwrap_or(env.color);
                self.node(body, Env { color, ..env })
            }
            Node::Class(class, body) => Laid::new(self.node(body, env).b, *class),
            Node::Boxed(body) => {
                let b = self.node(body, env).b;
                let s = self.size(env.level);
                let (pad, t) = (s * 0.25, self.rule(env.level));
                let (w, top, bottom) = (b.w + pad * 2.0, -(b.asc + pad), b.desc + pad);
                let mut out = MathBox::default();
                out.put(b, pad, 0.0);
                out.items.push(Item::path(
                    vec![
                        Stroke::Line(0.0, top, w, top),
                        Stroke::Line(w, top, w, bottom),
                        Stroke::Line(w, bottom, 0.0, bottom),
                        Stroke::Line(0.0, bottom, 0.0, top),
                    ],
                    t,
                    env.color,
                ));
                out.asc = -top + t;
                out.desc = bottom + t;
                Laid::new(out, Class::Ord)
            }
            Node::Phantom(body) => {
                let mut b = self.node(body, env).b;
                b.items.clear();
                Laid::new(b, Class::Ord)
            }
            Node::Error(text) => {
                let red = Color::hex(if self.cx.dark { 0xff7b72 } else { 0xcf222e });
                let env = Env { color: red, ..env };
                Laid::new(self.glyphs(text, Font::Text, env), Class::Ord)
            }
        }
    }

    /// 一串字：量宽度和墨迹的高低，成一项文字。
    fn glyphs(&mut self, text: &str, font: Font, env: Env) -> MathBox {
        let size = self.size(env.level);
        let kind = if font == Font::Text {
            FontKind::Ui
        } else {
            FontKind::Math
        };
        let shown: String = text.chars().map(|c| letter(c, font)).collect();
        // 换不成数学粗体字母的（中文）模拟加粗。
        let faux = Faux {
            bold: matches!(font, Font::Bold | Font::BoldItalic) && shown == text,
            italic: false,
        };
        let frame = &mut *self.cx.frame;
        let (mut w, mut asc, mut desc) = (0f32, 0f32, 0f32);
        for c in shown.chars() {
            w += frame.advance_faux(kind, faux, size, c);
            if let Some((top, bottom)) = frame.ink(kind, faux, size, c) {
                asc = asc.max(top);
                desc = desc.max(bottom);
            }
        }
        let item = Item::Text {
            x: 0.0,
            baseline: 0.0,
            text: shown.into(),
            font: kind,
            faux,
            size,
            color: env.color,
            span: None,
        };
        MathBox {
            w,
            asc,
            desc,
            items: vec![item],
        }
    }

    /// 横着排一串，按类别留空（TeX 的规则：关系两边宽、运算两边中、大运算符旁边窄，上下标里
    /// 只留窄的）。
    ///
    /// `lead` 为真时开头当作接在一个普通符号后面（`aligned` 里 `&=` 的 `=` 左边也要留空）。
    fn hlist(&mut self, nodes: &[Node], env: Env, lead: bool) -> MathBox {
        let mut laid: Vec<Laid> = nodes.iter().map(|n| self.node(n, env)).collect();
        // 前面没有东西、跟在运算或括号后面的「+」「−」是正负号，不是二元运算。
        for i in 0..laid.len() {
            if laid[i].class != Class::Bin {
                continue;
            }
            let prev = if i == 0 {
                (!lead).then_some(Class::Bin)
            } else {
                Some(laid[i - 1].class)
            };
            let next = laid.get(i + 1).map(|l| l.class);
            let unary = prev.is_some_and(|p| {
                matches!(
                    p,
                    Class::Bin | Class::Op | Class::Rel | Class::Open | Class::Punct
                )
            });
            if unary || next.is_none_or(|n| matches!(n, Class::Rel | Class::Close | Class::Punct)) {
                laid[i].class = Class::Ord;
            }
        }
        let script = matches!(env.level, Level::Script | Level::Tiny);
        let mu = self.size(env.level) / 18.0;
        let mut out = MathBox::default();
        let mut x = 0.0;
        let mut prev = lead.then_some(Class::Ord);
        for item in laid {
            if let Some(p) = prev {
                x += spacing(p, item.class, script) * mu;
            }
            prev = Some(item.class);
            let w = item.b.w;
            out.put(item.b, x, 0.0);
            x += w;
        }
        out.w = out.w.max(x);
        out
    }

    /// 排分式 `num / den`；`bar` 为假时排成 `\binom` 的无横线形式（两边加圆括号）。
    fn frac(&mut self, num: &Node, den: &Node, bar: bool, force: Option<bool>, env: Env) -> Laid {
        let level = match (force, env.level) {
            (Some(true), _) => Level::Display,
            (Some(false), Level::Display) => Level::Text,
            (_, level) => level,
        };
        let inner = Env {
            level: level.frac(),
            ..env
        };
        let n = self.node(num, inner).b;
        let d = self.node(den, inner).b;
        let (s, t, axis) = (self.size(level), self.rule(level), self.axis(level));
        let display = level == Level::Display;
        let pad = s * 0.12;
        // 上下留白取自 TeX 的分数参数（单位 em）：display 样式上 0.677、下 0.686，其余样式
        // 上 0.394、下 0.345。
        let w = n.w.max(d.w) + pad * 2.0;
        let (mut u, mut v) = if display {
            (s * 0.677, s * 0.686)
        } else {
            (s * 0.394, s * 0.345)
        };
        if bar {
            let gap = if display { t * 3.0 } else { t };
            u = u.max(axis + t / 2.0 + gap + n.desc);
            v = v.max(d.asc - axis + t / 2.0 + gap);
        } else {
            let clear = if display { t * 7.0 } else { t * 3.0 };
            let gap = (u - n.desc) - (d.asc - v);
            if gap < clear {
                u += (clear - gap) / 2.0;
                v += (clear - gap) / 2.0;
            }
        }
        let mut out = MathBox::default();
        let (nw, dw) = (n.w, d.w);
        out.put(n, (w - nw) / 2.0, -u);
        out.put(d, (w - dw) / 2.0, v);
        out.w = w;
        if bar {
            out.items.push(Item::Fill {
                rect: Rect::new(pad / 2.0, -axis - t / 2.0, w - pad, t),
                color: env.color,
                radius: 0.0,
            });
            out.asc = out.asc.max(axis + t);
        }
        if !bar {
            // `\binom`：两边加圆括号。
            let target = self.fence_height(&out, level);
            let env = Env { level, ..env };
            let left = self.delimiter("(", target, env);
            let right = self.delimiter(")", target, env);
            let mut wrapped = MathBox::default();
            let lw = left.w;
            wrapped.put(left, 0.0, 0.0);
            wrapped.put(out, lw, 0.0);
            let x = wrapped.w;
            wrapped.put(right, x, 0.0);
            return Laid::new(wrapped, Class::Ord);
        }
        Laid::new(out, Class::Ord)
    }

    /// 排根号 `body`；`index` 是开方次数（`\sqrt[3]` 里的 `3`）。
    fn sqrt(&mut self, index: Option<&Node>, body: &Node, env: Env) -> Laid {
        let b = self.node(body, env).b;
        let (s, t) = (self.size(env.level), self.rule(env.level));
        let phi = if env.level == Level::Display {
            s * 0.25
        } else {
            t
        };
        let clearance = t + phi / 4.0;
        let body_top = b.asc.max(s * 0.45);
        let top = -(body_top + clearance) - t / 2.0;
        let bottom = b.desc.max(s * 0.05) + s * 0.05;
        let h = bottom - top;
        let rw = (s * 0.42 + h * 0.12).min(s * 0.9);
        // 开方次数：小两级，放在根号左上，宽了就把根号往右挪。
        let index = index.map(|i| {
            self.node(
                i,
                Env {
                    level: Level::Tiny,
                    ..env
                },
            )
            .b
        });
        let shift = index.as_ref().map_or(0.0, |i| (i.w - rw * 0.55).max(0.0));
        let x = |v: f32| v + shift;
        let p0 = (x(0.0), top + h * 0.62);
        let p1 = (x(rw * 0.22), top + h * 0.54);
        let p2 = (x(rw * 0.5), bottom);
        let p3 = (x(rw), top);
        let end = x(rw) + s * 0.08 + b.w + s * 0.06;
        let mut out = MathBox::default();
        let body_x = x(rw) + s * 0.08;
        out.put(b, body_x, 0.0);
        out.items.push(Item::path(
            vec![
                Stroke::Line(p0.0, p0.1, p1.0, p1.1),
                Stroke::Line(p2.0, p2.1, p3.0, p3.1),
                Stroke::Line(p3.0, p3.1, end, p3.1),
            ],
            t,
            env.color,
        ));
        out.items.push(Item::path(
            vec![Stroke::Line(p1.0, p1.1, p2.0, p2.1)],
            t * 1.9,
            env.color,
        ));
        if let Some(i) = index {
            let y = top + h * 0.5 - i.desc;
            let ix = x(rw * 0.55) - i.w;
            out.put(i, ix, y);
        }
        out.w = out.w.max(end);
        out.asc = out.asc.max(-top + t);
        out.desc = out.desc.max(bottom);
        Laid::new(out, Class::Ord)
    }

    /// 排上下标：`base` 后面接上标 `sup`、下标 `sub`。
    fn scripts(&mut self, base: &Node, sup: Option<&Node>, sub: Option<&Node>, env: Env) -> Laid {
        // 大运算符、`\overbrace` 这类的上下标放在正上下方。
        let stacked = match base {
            Node::Op(_, _, limits) => limits.unwrap_or(env.level == Level::Display),
            Node::Accent(Accent::OverBrace | Accent::UnderBrace, _) => true,
            _ => false,
        };
        let laid = self.node(base, env);
        let s = self.size(env.level);
        if stacked {
            let gap = s * 0.15;
            return Laid::new(self.limits(laid.b, sup, sub, gap, env), laid.class);
        }
        let b = laid.b;
        let script = Env {
            level: env.level.script(),
            ..env
        };
        let ss = self.size(script.level);
        let t = self.rule(env.level);
        let up = sup.map(|n| self.node(n, script).b);
        let down = sub.map(|n| self.node(n, script).b);
        let mut u = 0.0;
        let mut v = 0.0;
        // 上标至少抬起、下标至少下沉的比例（相对字号）取自 TeX 的上下标参数。
        if let Some(up) = &up {
            let min = if env.level == Level::Display {
                0.413
            } else {
                0.363
            };
            u = (b.asc - ss * 0.38).max(s * min).max(up.desc + s * 0.11);
        }
        if let Some(down) = &down {
            v = (b.desc + ss * 0.05).max(s * 0.15).max(down.asc - s * 0.36);
        }
        if let (Some(up), Some(down)) = (&up, &down) {
            v = v.max(s * 0.247);
            let gap = (u - up.desc) - (down.asc - v);
            if gap < t * 4.0 {
                v += t * 4.0 - gap;
                let psi = s * 0.36 - (u - up.desc);
                if psi > 0.0 {
                    u += psi;
                    v -= psi;
                }
            }
        }
        // 斜体字母右上角往外伸，上标往右挪一点。
        let kern = if italic_letter(base) { s * 0.05 } else { 0.0 };
        let bw = b.w;
        let mut out = MathBox::default();
        out.put(b, 0.0, 0.0);
        let mut right: f32 = bw;
        if let Some(up) = up {
            right = right.max(bw + kern + up.w);
            out.put(up, bw + kern, -u);
        }
        if let Some(down) = down {
            right = right.max(bw + down.w);
            out.put(down, bw, v);
        }
        out.w = right + s * 0.05;
        Laid::new(out, laid.class)
    }

    /// 上下限放在 `base` 的正上方、正下方（`\sum_{i=1}^n`、`\overset`）。
    fn limits(
        &mut self,
        base: MathBox,
        over: Option<&Node>,
        under: Option<&Node>,
        gap: f32,
        env: Env,
    ) -> MathBox {
        let script = Env {
            level: env.level.script(),
            ..env
        };
        let s = self.size(env.level);
        let over = over.map(|n| self.node(n, script).b);
        let under = under.map(|n| self.node(n, script).b);
        let w = base
            .w
            .max(over.as_ref().map_or(0.0, |b| b.w))
            .max(under.as_ref().map_or(0.0, |b| b.w));
        let (base_asc, base_desc, bw) = (base.asc, base.desc, base.w);
        let mut out = MathBox::default();
        out.put(base, (w - bw) / 2.0, 0.0);
        if let Some(o) = over {
            let y = -(base_asc + gap.max(s * 0.2 - o.desc) + o.desc);
            let ow = o.w;
            out.put(o, (w - ow) / 2.0, y);
            out.asc += s * 0.05;
        }
        if let Some(u) = under {
            let y = base_desc + gap.max(s * 0.6 - u.asc).min(s * 0.4) + u.asc;
            let uw = u.w;
            out.put(u, (w - uw) / 2.0, y);
            out.desc += s * 0.05;
        }
        out.w = w;
        out
    }

    /// 排运算符 `text`；`big` 为真时按层级放大（`\sum`、`\int` 这类大运算符）。
    fn op(&mut self, text: &str, big: bool, env: Env) -> Laid {
        if !big {
            return Laid::new(self.glyphs(text, Font::Roman, env), Class::Op);
        }
        let integral = matches!(text, "∫" | "∬" | "∭" | "∮" | "∯");
        let factor = match (env.level == Level::Display, integral) {
            (true, true) => 2.0,
            (true, false) => 1.45,
            (false, true) => 1.2,
            (false, false) => 1.05,
        };
        let s = self.size(env.level);
        let mut b = self.glyphs(text, Font::Roman, env);
        // 换成大字号，墨迹的中间对着数学轴。
        if let Some(Item::Text { size, .. }) = b.items.first_mut() {
            *size = s * factor;
        }
        let w = self.cx.frame.measure(FontKind::Math, s * factor, text);
        let (mut asc, mut desc) = (0f32, 0f32);
        for c in text.chars() {
            if let Some((top, bottom)) =
                self.cx
                    .frame
                    .ink(FontKind::Math, Faux::default(), s * factor, c)
            {
                asc = asc.max(top);
                desc = desc.max(bottom);
            }
        }
        let dy = (asc - desc) / 2.0 - self.axis(env.level);
        let mut out = MathBox::default();
        out.put(
            MathBox {
                w,
                asc,
                desc,
                items: std::mem::take(&mut b.items),
            },
            0.0,
            dy,
        );
        out.w = w + if integral { s * 0.1 } else { 0.0 };
        Laid::new(out, Class::Op)
    }

    /// 在 `body` 上方或下方画重音 `accent`（横线、帽、波浪号、箭头、点、花括号等）。
    fn accent(&mut self, accent: Accent, body: &Node, env: Env) -> Laid {
        let b = self.node(body, env).b;
        let (s, t) = (self.size(env.level), self.rule(env.level));
        let single = single_glyph(body);
        let skew = if italic_letter(body) { s * 0.06 } else { 0.0 };
        let cx = b.w / 2.0 + skew;
        let (asc, desc, bw) = (b.asc.max(s * 0.45), b.desc, b.w);
        let mut out = MathBox::default();
        out.put(b, 0.0, 0.0);
        let color = env.color;
        let top = -(asc + s * 0.1);
        let narrow = |w: f32| if single { s * 0.45 } else { w.max(s * 0.45) };
        match accent {
            Accent::Bar => {
                let w = if single { s * 0.45 } else { bw };
                let x = if single { cx - w / 2.0 } else { 0.0 };
                out.items.push(Item::Fill {
                    rect: Rect::new(x, top - t, w, t),
                    color,
                    radius: 0.0,
                });
                out.asc = out.asc.max(-top + t);
            }
            Accent::Underline => {
                let y = desc + s * 0.1;
                out.items.push(Item::Fill {
                    rect: Rect::new(0.0, y, bw, t),
                    color,
                    radius: 0.0,
                });
                out.desc = out.desc.max(y + t);
            }
            Accent::Hat | Accent::Check => {
                let w = narrow(bw.min(s * 3.0));
                let h = s * 0.13 + w * 0.04;
                let (base, peak) = if accent == Accent::Hat {
                    (top, top - h)
                } else {
                    (top - h, top)
                };
                out.items.push(Item::path(
                    vec![
                        Stroke::Line(cx - w / 2.0, base, cx, peak),
                        Stroke::Line(cx, peak, cx + w / 2.0, base),
                    ],
                    t,
                    color,
                ));
                out.asc = out.asc.max(-top + h + t);
            }
            Accent::Tilde => {
                let w = narrow(bw.min(s * 3.0));
                let h = s * 0.07;
                let points: Vec<(f32, f32)> = (0..=12)
                    .map(|k| {
                        let f = k as f32 / 12.0;
                        let y = top - h - (f * std::f32::consts::TAU).sin() * h;
                        (cx - w / 2.0 + f * w, y)
                    })
                    .collect();
                out.items
                    .push(Item::path(polyline(&points), t * 1.1, color));
                out.asc = out.asc.max(-top + h * 2.0 + t);
            }
            Accent::Vec | Accent::LeftArrow => {
                let w = if single { s * 0.5 } else { bw };
                let (x0, x1) = if single {
                    (cx - w / 2.0, cx + w / 2.0)
                } else {
                    (0.0, bw)
                };
                let y = top - s * 0.08;
                let head = s * 0.13;
                let (tip, dir) = if accent == Accent::Vec {
                    (x1, -1.0)
                } else {
                    (x0, 1.0)
                };
                out.items.push(Item::path(
                    vec![
                        Stroke::Line(x0, y, x1, y),
                        Stroke::Line(tip, y, tip + dir * head, y - head * 0.7),
                        Stroke::Line(tip, y, tip + dir * head, y + head * 0.7),
                    ],
                    t,
                    color,
                ));
                out.asc = out.asc.max(-y + head);
            }
            Accent::Dot | Accent::DDot => {
                let r = s * 0.055;
                let y = top - r * 2.0;
                let xs: &[f32] = if accent == Accent::Dot {
                    &[0.0]
                } else {
                    &[-s * 0.12, s * 0.12]
                };
                for dx in xs {
                    out.items.push(Item::Fill {
                        rect: Rect::new(cx + dx - r, y - r, r * 2.0, r * 2.0),
                        color,
                        radius: r,
                    });
                }
                out.asc = out.asc.max(-y + r);
            }
            Accent::OverBrace | Accent::UnderBrace => {
                let h = s * 0.25;
                let (y0, dir) = if accent == Accent::OverBrace {
                    (top, -1.0)
                } else {
                    (desc + s * 0.1, 1.0)
                };
                let mid = bw / 2.0;
                let points = [
                    (0.0, y0),
                    (h * 0.5, y0 + dir * h * 0.5),
                    (mid - h * 0.5, y0 + dir * h * 0.5),
                    (mid, y0 + dir * h),
                    (mid + h * 0.5, y0 + dir * h * 0.5),
                    (bw - h * 0.5, y0 + dir * h * 0.5),
                    (bw, y0),
                ];
                out.items.push(Item::path(polyline(&points), t, color));
                if dir < 0.0 {
                    out.asc = out.asc.max(-(y0 - h) + t);
                } else {
                    out.desc = out.desc.max(y0 + h + t);
                }
            }
        }
        Laid::new(out, Class::Ord)
    }

    /// 返回包住 `body` 的括号应有多高（逻辑像素）。
    ///
    /// 以数学轴为中心，允许比内容略矮（0.901 取自 TeX 的 delimiter factor）；矮的部分不超过
    /// 半个字号。
    fn fence_height(&self, body: &MathBox, level: Level) -> f32 {
        let axis = self.axis(level);
        let delta = (body.asc - axis).max(body.desc + axis);
        (delta * 2.0 * 0.901).max(delta * 2.0 - self.size(level) * 0.5)
    }

    /// 高 `target` 的括号 `delim`：不高时用字，高一点的把字放大，再高的画出来。
    ///
    /// 字最多放大到 [`GLYPH_STRETCH`] 倍。没有画法的符号（`\left a`）不再继续放大：按内容高度
    /// 放大时字号没有上限，数千行的矩阵会要求光栅化数万像素高的字形，占用数 GB 内存。
    fn delimiter(&mut self, delim: &str, target: f32, env: Env) -> MathBox {
        let s = self.size(env.level);
        if delim.is_empty() {
            return MathBox {
                w: s * 0.12,
                ..MathBox::default()
            };
        }
        let glyph = self.glyphs(delim, Font::Roman, env);
        let natural = glyph.asc + glyph.desc;
        if target <= natural * 1.1 || natural <= 0.0 {
            return glyph;
        }
        let axis = self.axis(env.level);
        let (top, bottom) = (-axis - target / 2.0, -axis + target / 2.0);
        let t = self.rule(env.level);
        let drawn = drawn_delimiter(delim, top, bottom, s, t);
        let Some((w, strokes, width)) = drawn.filter(|_| target > natural * GLYPH_STRETCH) else {
            // 放大字号，墨迹的中间对着数学轴。
            let f = (target / natural).min(GLYPH_STRETCH);
            let mut big = self.glyphs(
                delim,
                Font::Roman,
                Env {
                    level: env.level,
                    ..env
                },
            );
            let size = s * f;
            if let Some(Item::Text { size: sz, .. }) = big.items.first_mut() {
                *sz = size;
            }
            let w = self.cx.frame.measure(FontKind::Math, size, delim);
            let (asc, desc) = (glyph.asc * f, glyph.desc * f);
            let dy = (asc - desc) / 2.0 - axis;
            let mut out = MathBox::default();
            out.put(
                MathBox {
                    w,
                    asc,
                    desc,
                    items: big.items,
                },
                0.0,
                dy,
            );
            return out;
        };
        MathBox {
            w,
            asc: -top,
            desc: bottom,
            items: vec![Item::path(strokes, width, env.color)],
        }
    }

    /// 排 `\begin{matrix}` 这类网格，按 `grid.align` 对齐各列。
    fn grid(&mut self, grid: &Grid, env: Env) -> MathBox {
        let level = if grid.display {
            env.level
        } else {
            match env.level {
                Level::Display => Level::Text,
                level => level,
            }
        };
        let cell_env = Env { level, ..env };
        let s = self.size(level);
        let aligned = !grid.gap;
        let cells: Vec<Vec<MathBox>> = grid
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(j, cell)| {
                        let nodes = match cell {
                            Node::Row(nodes) => nodes.as_slice(),
                            other => std::slice::from_ref(other),
                        };
                        // `aligned` 的第二、四……列接在 `&` 后面，开头的 `=` 左边也留空。
                        self.hlist(nodes, cell_env, aligned && j % 2 == 1)
                    })
                    .collect()
            })
            .collect();
        let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0f32; cols];
        for row in &cells {
            for (j, cell) in row.iter().enumerate() {
                widths[j] = widths[j].max(cell.w);
            }
        }
        let gap = |j: usize| {
            if j == 0 {
                0.0
            } else if !aligned {
                s
            } else if j.is_multiple_of(2) {
                s * 2.0
            } else {
                0.0
            }
        };
        let align = |j: usize| {
            if aligned {
                grid.align[j % grid.align.len().max(1)]
            } else {
                grid.align
                    .get(j)
                    .or(grid.align.last())
                    .copied()
                    .unwrap_or('c')
            }
        };
        let row_gap = if grid.display { s * 0.25 } else { s * 0.15 };
        let heights: Vec<(f32, f32)> = cells
            .iter()
            .map(|row| {
                row.iter().fold((s * 0.72, s * 0.3), |(a, d), c| {
                    (a.max(c.asc), d.max(c.desc))
                })
            })
            .collect();
        let total: f32 = heights.iter().map(|(a, d)| a + d).sum::<f32>()
            + row_gap * heights.len().saturating_sub(1) as f32;
        let axis = self.axis(level);
        let mut body = MathBox::default();
        let mut y = -axis - total / 2.0;
        for (row, &(a, d)) in cells.into_iter().zip(&heights) {
            let baseline = y + a;
            let mut x = 0.0;
            for (j, cell) in row.into_iter().enumerate() {
                x += gap(j);
                let room = widths[j] - cell.w;
                let dx = match align(j) {
                    'l' => 0.0,
                    'r' => room,
                    _ => room / 2.0,
                };
                body.put(cell, x + dx, baseline);
                x += widths[j];
            }
            y = baseline + d + row_gap;
        }
        body.w = (0..cols).map(|j| gap(j) + widths[j]).sum();
        body.asc = body.asc.max(axis + total / 2.0);
        body.desc = body.desc.max(total / 2.0 - axis);
        if grid.left.is_empty() && grid.right.is_empty() {
            return body;
        }
        let target = total + s * 0.2;
        let pad = s * 0.15;
        let left = if grid.left.is_empty() {
            MathBox::default()
        } else {
            self.delimiter(&grid.left, target, env)
        };
        let right = if grid.right.is_empty() {
            MathBox::default()
        } else {
            self.delimiter(&grid.right, target, env)
        };
        let mut out = MathBox::default();
        let lw = left.w;
        out.put(left, 0.0, 0.0);
        let bw = body.w;
        out.put(body, lw + pad, 0.0);
        out.put(right, lw + pad * 2.0 + bw, 0.0);
        out
    }
}
