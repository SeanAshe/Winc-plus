//! `<foreignObject>` 中的文字：读出 XHTML 内容里的文字和基本样式，排成若干行。
//!
//! 位置和对齐取自各层元素的外边距、内边距、`width`、`max-width`、`height`、`text-align`、
//! `justify-content`、`align-items`，另支持 `color`、`background-color`、字体属性、`white-space`、
//! `line-height`、`<br>`、`<b>`、`<i>`；样式取自元素的属性、样式表和 `style`。其余的 HTML 排版
//! 不支持。

use roxmltree::Node;

use zeno::{Command, Fill, Point};

use super::super::syntax::{color, length};
use super::super::{Item, MAX_DEPTH, Own, PaintSpec, Parser, Props, Shape};
use super::{Anchor, Char, Font, apply_font};
use crate::svg::syntax::Affine;

/// 未写 `line-height` 时的行高与字号之比：CSS 的 `normal` 约为 1.2。
const LEADING: f32 = 1.2;

/// 文字在盒子里沿一个方向的对齐。
#[derive(Clone, Copy, PartialEq)]
enum Align {
    Start,
    Center,
    End,
}

impl Align {
    /// 解析 `text-align`、`justify-content`、`align-items` 的值；不认识时返回 `None`。
    fn parse(value: &str) -> Option<Align> {
        let has = |word: &str| value.contains(word);
        if has("center") {
            Some(Align::Center)
        } else if has("end") || has("right") {
            Some(Align::End)
        } else if has("start") || has("left") {
            Some(Align::Start)
        } else {
            None
        }
    }
}

/// 文字所在的盒子：左上角、宽高和折行的宽度上限（用户坐标单位）、底色、对齐、是否折行、行高
/// 与字号之比。
#[derive(Clone)]
struct Frame {
    x: f32,
    y: f32,
    width: Option<f32>,
    limit: Option<f32>,
    height: f32,
    fill: Option<[u8; 4]>,
    across: Align,
    down: Align,
    wrap: bool,
    leading: f32,
}

/// 读出的文字：各行的字符及其样式的下标、各样式，以及第一个字所在的盒子。
struct Label {
    lines: Vec<Vec<(char, usize)>>,
    styles: Vec<(Props, Own)>,
    frame: Option<Frame>,
}

impl Label {
    /// 当前行不为空时另起一行。
    fn end_line(&mut self) {
        if self.lines.last().is_some_and(|line| !line.is_empty()) {
            self.lines.push(Vec::new());
        }
    }
}

/// 本模块能排版的 HTML 元素；含有其他元素（表格、列表、图片、上下标等）的内容排不出原样。
const PLAIN: [&str; 18] = [
    "div",
    "span",
    "p",
    "br",
    "b",
    "strong",
    "i",
    "em",
    "font",
    "blockquote",
    "a",
    "center",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];

impl<'a, 'input> Parser<'a, 'input> {
    /// `<foreignObject>` 元素 `node` 的内容是否都能排版：有字形来源，且只含 [`PLAIN`] 中的元素。
    pub(in crate::svg) fn foreign_plain(&self, node: Node<'a, 'input>) -> bool {
        let known = |n: Node| PLAIN.contains(&n.tag_name().name().to_ascii_lowercase().as_str());
        let mut inside = node.descendants().skip(1).filter(Node::is_element);
        self.glyphs.is_some() && inside.all(known)
    }

    /// 绘制 `<foreignObject>` 元素 `node` 中的文字，返回是否有文字可画。
    ///
    /// `(props, own)` 是该元素的样式，`ctm` 是当前变换。
    pub(in crate::svg) fn foreign_text(
        &mut self,
        node: Node<'a, 'input>,
        (props, own): (&Props, &Own),
        ctm: Affine,
        out: &mut Vec<Item>,
    ) -> bool {
        let Some(glyphs) = self.glyphs.clone() else {
            return false;
        };
        // HTML 文字的颜色取自 `color`，默认为黑色，不使用 SVG 的填充和描边。
        let mut base = props.clone();
        base.fill = PaintSpec::Color([0, 0, 0, 255]);
        base.stroke = PaintSpec::None;
        // 百分比的宽高相对视口，不是文字所在盒子的大小，不采用。
        let side = |name: &str| {
            let value = node
                .attribute(name)
                .filter(|v| !v.trim_end().ends_with('%'));
            value.and_then(|v| length(v, 0.0))
        };
        let frame = Frame {
            x: side("x").unwrap_or(0.0),
            y: side("y").unwrap_or(0.0),
            width: side("width"),
            limit: None,
            height: side("height").unwrap_or(0.0),
            fill: None,
            across: Align::Start,
            down: Align::Start,
            wrap: true,
            leading: LEADING,
        };
        let mut label = Label {
            lines: vec![Vec::new()],
            styles: vec![(base, own.clone())],
            frame: None,
        };
        self.gather(node, 0, &frame, &mut label, 0);
        while label.lines.last().is_some_and(Vec::is_empty) {
            label.lines.pop();
        }
        let Some(frame) = label.frame.take().filter(|_| !label.lines.is_empty()) else {
            return false;
        };

        let font_of = |style: usize| label.styles[style].0.font.clone();
        let advance = |c: char, style: usize| {
            let font = font_of(style);
            let glyph = glyphs.glyph(&font.families, font.weight, font.italic, c);
            glyph.map_or(font.size * 0.5, |g| g.advance * font.size)
        };
        let room = match (frame.width, frame.limit) {
            (Some(width), Some(limit)) => Some(width.min(limit)),
            (width, limit) => width.or(limit),
        };
        let lines: Vec<Vec<(char, usize)>> = (label.lines.into_iter())
            .flat_map(|line| match room.filter(|w| frame.wrap && *w > 1.0) {
                Some(room) => wrapped(line, room, &advance),
                None => vec![line],
            })
            .collect();
        // 各行的高度：行内最大的字号乘以行高之比；空行按第一种样式的字号。
        let size_of = |line: &[(char, usize)]| {
            let sizes = line.iter().map(|&(_, style)| font_of(style).size);
            sizes.fold(0.0f32, f32::max)
        };
        let heights: Vec<f32> = lines
            .iter()
            .map(|line| {
                frame.leading
                    * if line.is_empty() {
                        font_of(0).size
                    } else {
                        size_of(line)
                    }
            })
            .collect();
        let total: f32 = heights.iter().sum();
        let mut top = match frame.down {
            Align::Start => frame.y,
            Align::Center => frame.y + (frame.height - total) / 2.0,
            Align::End => frame.y + frame.height - total,
        };
        let width = frame.width.unwrap_or(0.0);
        let (x, anchor) = match frame.across {
            Align::Start => (frame.x, Anchor::Start),
            Align::Center => (frame.x + width / 2.0, Anchor::Middle),
            Align::End => (frame.x + width, Anchor::End),
        };
        // 底色铺在各行文字共同的外框下。
        let widest = lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|&(c, style)| advance(c, style))
                    .sum::<f32>()
            })
            .fold(0.0f32, f32::max);
        if let Some(fill) = frame.fill.filter(|fill| fill[3] > 0 && widest > 0.0) {
            let left = match anchor {
                Anchor::Start => x,
                Anchor::Middle => x - widest / 2.0,
                Anchor::End => x - widest,
            };
            let bounds = [left, top, widest, total];
            let at = |x: f32, y: f32| Point::new(x, y);
            let path = vec![
                Command::MoveTo(at(left, top)),
                Command::LineTo(at(left + widest, top)),
                Command::LineTo(at(left + widest, top + total)),
                Command::LineTo(at(left, top + total)),
                Command::Close,
            ];
            let paint = self.paint(&PaintSpec::Color(fill), 1.0, &label.styles[0].0, bounds);
            out.extend(paint.map(|paint| {
                Item::Shape(Shape {
                    bounds,
                    path,
                    transform: ctm,
                    fill: Some((paint, Fill::NonZero)),
                    stroke: None,
                    stroke_first: false,
                })
            }));
        }
        let mut chars = Vec::new();
        for (line, height) in lines.iter().zip(&heights) {
            if let Some(&(_, first)) = line.first() {
                // 字的上下留白相等：基线在行内文字的上沿之下一个上伸部的高度处。
                let font = font_of(first);
                let metrics = glyphs.metrics(&font.families, font.weight, font.italic);
                let size = size_of(line);
                let inked = (metrics.ascent + metrics.descent) * size;
                let baseline = top + (height - inked) / 2.0 + metrics.ascent * size;
                for (k, &(c, style)) in line.iter().enumerate() {
                    chars.push(Char {
                        c,
                        style,
                        x: (k == 0).then_some(x),
                        y: (k == 0).then_some(baseline),
                        dx: 0.0,
                        dy: 0.0,
                    });
                }
            }
            top += height;
        }
        for (props, _) in &mut label.styles {
            props.font.anchor = anchor;
        }
        self.place(glyphs.as_ref(), &chars, &label.styles, ctm, out);
        !chars.is_empty()
    }

    /// 收集 `node` 内的文字：合并空白，`<br>` 和块级元素分行，元素的 `style` 更新样式和盒子。
    fn gather(
        &mut self,
        node: Node<'a, 'input>,
        style: usize,
        frame: &Frame,
        label: &mut Label,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for child in node.children() {
            if child.is_text() {
                for c in child.text().unwrap_or_default().chars() {
                    let line = label.lines.last_mut().expect("至少有一行");
                    let space = c.is_whitespace();
                    // 行首的空白不要，连续的空白合并为一个空格。
                    if space && line.last().is_none_or(|&(last, _)| last == ' ') {
                        continue;
                    }
                    line.push((if space { ' ' } else { c }, style));
                    label.frame.get_or_insert_with(|| frame.clone());
                }
                continue;
            }
            if !child.is_element() {
                continue;
            }
            let tag = child.tag_name().name().to_ascii_lowercase();
            match tag.as_str() {
                "br" => {
                    label.lines.push(Vec::new());
                    continue;
                }
                "style" | "script" | "head" | "title" => continue,
                _ => {}
            }
            let (mut props, own) = label.styles[style].clone();
            let parent: Font = props.font.clone();
            let mut frame = frame.clone();
            match tag.as_str() {
                "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    props.font.weight = 700;
                }
                "i" | "em" => props.font.italic = true,
                _ => {}
            }
            if let Some(c) = child.attribute("color").and_then(color) {
                props.fill = PaintSpec::Color(c);
            }
            let diagonal = self.diagonal();
            let mut hidden = false;
            for (name, value) in self.declared(child) {
                let value = value.trim();
                let px = length(value, 0.0);
                // 简写的外边距、内边距：一到四个值依次是上、右、下、左，缺少的取对边或第一个值。
                let sides = || {
                    let all: Vec<f32> = (value.split_whitespace())
                        .map(|part| length(part, 0.0).unwrap_or(0.0))
                        .collect();
                    let top = all.first().copied().unwrap_or(0.0);
                    let right = all.get(1).copied().unwrap_or(top);
                    (top, all.get(3).copied().unwrap_or(right))
                };
                match &*name {
                    "margin" | "padding" => {
                        let (top, left) = sides();
                        (frame.x, frame.y) = (frame.x + left, frame.y + top);
                    }
                    "background-color" | "background" => frame.fill = color(value).or(frame.fill),
                    "max-width" => {
                        frame.limit = px.filter(|_| !value.ends_with('%')).or(frame.limit);
                    }
                    "color" => {
                        if let Some(c) = color(value) {
                            props.fill = PaintSpec::Color(c);
                        }
                    }
                    "margin-left" | "padding-left" => frame.x += px.unwrap_or(0.0),
                    "margin-top" | "padding-top" => frame.y += px.unwrap_or(0.0),
                    "width" => frame.width = px.filter(|_| !value.ends_with('%')).or(frame.width),
                    "height" => frame.height = px.filter(|_| !value.ends_with('%')).unwrap_or(0.0),
                    "text-align" | "justify-content" => {
                        frame.across = Align::parse(value).unwrap_or(frame.across);
                    }
                    "align-items" => frame.down = Align::parse(value).unwrap_or(frame.down),
                    "white-space" => frame.wrap = !(value.contains("nowrap") || value == "pre"),
                    "line-height" => {
                        frame.leading = value.parse().unwrap_or(frame.leading);
                    }
                    "display" => hidden = value == "none",
                    _ => apply_font(&mut props.font, &parent, &name, value, diagonal),
                }
            }
            if hidden {
                continue;
            }
            label.styles.push((props, own));
            let block = matches!(
                tag.as_str(),
                "div" | "p" | "li" | "ul" | "ol" | "tr" | "table" | "blockquote" | "pre"
            ) || tag.starts_with('h') && tag.len() == 2;
            if block {
                label.end_line();
            }
            self.gather(child, label.styles.len() - 1, &frame, label, depth + 1);
            if block {
                label.end_line();
            }
        }
    }
}

/// 把一行按宽度 `width` 折成若干行：在空格处、以及不以空格分词的文字（CJK）之间折行。
fn wrapped(
    line: Vec<(char, usize)>,
    width: f32,
    advance: &impl Fn(char, usize) -> f32,
) -> Vec<Vec<(char, usize)>> {
    let mut lines: Vec<Vec<(char, usize)>> = vec![Vec::new()];
    // 当前行的宽度，以及最近一个可以折行的位置（行内下标，折行后是否丢弃该处的空格）。
    let (mut used, mut gap) = (0.0f32, None::<(usize, bool)>);
    let wide = |c: char| c as u32 >= 0x2E80;
    for (c, style) in line {
        let current = lines.last_mut().expect("至少有一行");
        if c == ' ' {
            gap = Some((current.len(), true));
        } else if wide(c) && !current.is_empty() {
            gap = Some((current.len(), false));
        }
        let w = advance(c, style);
        if used + w > width
            && c != ' '
            && let Some((at, space)) = gap.take()
        {
            let mut rest = current.split_off(at);
            if space && rest.first().is_some_and(|&(c, _)| c == ' ') {
                rest.remove(0);
            }
            used = rest.iter().map(|&(c, style)| advance(c, style)).sum();
            lines.push(rest);
        }
        lines.last_mut().expect("至少有一行").push((c, style));
        used += w;
    }
    for line in &mut lines {
        while line.last().is_some_and(|&(c, _)| c == ' ') {
            line.pop();
        }
    }
    lines
}
