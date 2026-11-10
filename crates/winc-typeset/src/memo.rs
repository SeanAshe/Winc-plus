//! 排版缓存：段落、单元格（不含图片的行内文字）、代码块的一行和单元格的测量宽度，按内容和
//! 排版条件的哈希保存排版结果，跨次排版保留。
//!
//! 长列表、长表格、长代码块各是一个顶层块，修改一个字符即需重排整块；缓存使未修改的项、行和单元格
//! 直接移到新位置，只有修改处重新测量字宽、断行。[`Memo::prepare`] 的签名变化时全部失效；各顶层块
//! 记录其使用的条目（[`Memo::take_touched`]），修改前的旧结果由 [`Memo::sweep`] 清理。

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use winc_preview::markdown::Inline;
use winc_render::{Rect, Stroke};

use super::{Cx, Flow, Item, Para, Span};

/// 清理前允许保留的旧条目数（条）。
///
/// 缓存条目超过当前排版使用的条目数加上该值时清理。每条平均约 0.6 KB（经验值），旧条目
/// 最多占用约 0.6 MB。
const SLACK: usize = 1024;

/// 跨次排版留着的排版缓存，见模块说明。
#[derive(Default)]
pub struct Memo {
    /// 排出这些结果时的条件签名。
    key: u64,
    /// 段落的排版结果。
    pub(crate) flows: HashMap<u64, Arc<Piece>>,
    /// 代码块每一行的排版结果和行末的词法状态。
    pub(crate) lines: HashMap<u64, (Arc<Piece>, winc_syntax::LineState)>,
    /// 量出的（最长词宽，整段宽），都是逻辑像素。
    pub(crate) measures: HashMap<u64, (f32, f32)>,
    /// 上次调用 [`Memo::take_touched`] 之后读取或写入的条目。
    touched: Vec<u64>,
}

impl Memo {
    /// 返回三项缓存的条目总数。
    pub fn len(&self) -> usize {
        self.flows.len() + self.lines.len() + self.measures.len()
    }

    /// 是否没有任何缓存条目。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 开始一次排版，`key` 是排版条件的签名；签名与上次不同时清空缓存。
    pub fn prepare(&mut self, key: u64) {
        if key != self.key {
            *self = Memo {
                key,
                ..Memo::default()
            };
        }
    }

    /// 记录读取或写入的条目 `key`。
    pub fn touch(&mut self, key: u64) {
        self.touched.push(key);
    }

    /// 返回上次调用之后读取或写入的条目（已去重）。
    ///
    /// 排版一个顶层块之前调用一次以清空记录，排版完成后再调用一次，得到该块使用的条目。
    pub fn take_touched(&mut self) -> Box<[u64]> {
        let mut keys = std::mem::take(&mut self.touched);
        keys.sort_unstable();
        keys.dedup();
        keys.into()
    }

    /// 完成一次排版后清理缓存：条目数比 `live` 多出 `SLACK` 条以上时，只保留 `live` 中的条目。
    ///
    /// `live` 是当前排版使用的条目（各顶层块的 [`Memo::take_touched`]），多出的都是修改前的旧结果。
    /// 旧结果累积到上限才清理，每次输入无需逐条检查。
    pub fn sweep<'a>(&mut self, live: impl Iterator<Item = &'a [u64]> + Clone) {
        let count: usize = live.clone().map(<[u64]>::len).sum();
        if self.len() <= count * 2 + SLACK {
            return;
        }
        let live: HashSet<u64> = live.flatten().copied().collect();
        self.flows.retain(|k, _| live.contains(k));
        self.lines.retain(|k, _| live.contains(k));
        self.measures.retain(|k, _| live.contains(k));
        self.flows.shrink_to_fit();
        self.lines.shrink_to_fit();
        self.measures.shrink_to_fit();
    }
}

/// 单独排好的一段：坐标从 0 起，文字的位置从块文字的第 0 个字符起。
pub struct Piece {
    items: Vec<Item>,
    text: String,
    chars: usize,
    /// 这段的高度（逻辑像素）。
    pub(crate) height: f32,
    /// 第一行的基线（逻辑像素，从这段的顶部算起）；没有文字时为 `None`。
    pub(crate) first_baseline: Option<f32>,
    /// 排这段时它在原文中的位置（相对顶层块）；移动到其他位置时，文字的原文位置按差值平移。
    pub(crate) source: u32,
}

/// 用 `f` 写入的内容算出一个哈希值。
pub fn key(f: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut hasher = DefaultHasher::new();
    f(&mut hasher);
    hasher.finish()
}

impl Para {
    fn hash_into(&self, h: &mut DefaultHasher) {
        let c = self.color;
        for v in [self.size, self.line, c.r, c.g, c.b, c.a] {
            v.to_bits().hash(h);
        }
        self.bold.hash(h);
        self.align.hash(h);
    }
}

impl Item {
    /// 挪到 `(dx, dy)`，文字的位置往后挪 `dc` 个字符。
    pub(crate) fn moved(&self, dx: f32, dy: f32, dc: usize) -> Item {
        let rect = |r: &Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        match self {
            Item::Text {
                x,
                baseline,
                text,
                font,
                faux,
                size,
                color,
                span,
            } => Item::Text {
                x: x + dx,
                baseline: baseline + dy,
                text: Arc::clone(text),
                font: *font,
                faux: *faux,
                size: *size,
                color: *color,
                span: span.as_ref().map(|s| Span {
                    start: s.start + dc,
                    advances: Arc::clone(&s.advances),
                    source: s.source,
                    jumps: s.jumps.clone(),
                }),
            },
            Item::Fill {
                rect: r,
                color,
                radius,
            } => Item::Fill {
                rect: rect(r),
                color: *color,
                radius: *radius,
            },
            Item::Corners {
                rect: r,
                color,
                radii,
            } => Item::Corners {
                rect: rect(r),
                color: *color,
                radii: *radii,
            },
            Item::Image { rect: r, path, at } => Item::Image {
                rect: rect(r),
                path: path.clone(),
                at: at.map(|(c, s)| (c + dc, s)),
            },
            Item::Link { rect: r, url } => Item::Link {
                rect: rect(r),
                url: Arc::clone(url),
            },
            Item::Spot { rect: r, at } => Item::Spot {
                rect: rect(r),
                at: (at.0 + dc, at.1),
            },
            Item::Check {
                rect: r,
                checked,
                source,
            } => Item::Check {
                rect: rect(r),
                checked: *checked,
                source: *source,
            },
            Item::Table {
                rect: r,
                align,
                cols,
                rows,
            } => Item::Table {
                rect: rect(r),
                align: *align,
                cols: Arc::clone(cols),
                rows: Arc::clone(rows),
            },
            Item::Code {
                rect: r,
                text,
                diagram,
            } => Item::Code {
                rect: rect(r),
                text: Arc::clone(text),
                diagram: *diagram,
            },
            Item::Path {
                rect: r,
                strokes,
                width,
                color,
            } => Item::Path {
                rect: rect(r),
                strokes: strokes.iter().map(|&s| shift(s, dx, dy)).collect(),
                width: *width,
                color: *color,
            },
            Item::Polygon {
                rect: r,
                points,
                color,
            } => Item::Polygon {
                rect: rect(r),
                points: points.iter().map(|&(x, y)| (x + dx, y + dy)).collect(),
                color: *color,
            },
        }
    }
}

/// 线 `s` 挪 `(dx, dy)`。
pub fn shift(s: Stroke, dx: f32, dy: f32) -> Stroke {
    match s {
        Stroke::Line(ax, ay, bx, by) => Stroke::Line(ax + dx, ay + dy, bx + dx, by + dy),
        Stroke::Circle(cx, cy, r) => Stroke::Circle(cx + dx, cy + dy, r),
        Stroke::Arc(cx, cy, r, from, to) => Stroke::Arc(cx + dx, cy + dy, r, from, to),
    }
}

impl Cx<'_, '_> {
    /// 单独排一段：`f` 往空的项表、空的块文字里排，坐标从 0 起。
    pub(crate) fn isolated(&mut self, f: impl FnOnce(&mut Self, &mut Vec<Item>) -> Flow) -> Piece {
        let text = std::mem::take(&mut self.text);
        let chars = std::mem::replace(&mut self.chars, 0);
        let mut items = Vec::new();
        let flow = f(self, &mut items);
        Piece {
            items,
            text: std::mem::replace(&mut self.text, text),
            chars: std::mem::replace(&mut self.chars, chars),
            height: flow.height,
            first_baseline: flow.first_baseline,
            source: 0,
        }
    }

    /// 把单独排好的 `piece` 放到 `(x, y)`，文字接在块文字后面；它现在在原文中的位置是
    /// `source`（相对顶层块）。
    pub(crate) fn paste(
        &mut self,
        piece: &Piece,
        x: f32,
        y: f32,
        source: u32,
        out: &mut Vec<Item>,
    ) {
        let dc = self.chars;
        let ds = i64::from(source) - i64::from(piece.source);
        out.extend(piece.items.iter().map(|item| {
            let mut item = item.moved(x, y, dc);
            let shifted = |s: u32| u32::try_from(i64::from(s) + ds).unwrap_or(0);
            match &mut item {
                Item::Text {
                    span: Some(span), ..
                } if ds != 0 => span.source = span.source.map(shifted),
                Item::Image { at: Some(at), .. } if ds != 0 => at.1 = shifted(at.1),
                _ => {}
            }
            item
        }));
        self.text.push_str(&piece.text);
        self.chars += piece.chars;
    }

    /// 排一段行内文字：左上角在 `(x, y)`（逻辑像素），最宽 `width`（逻辑像素）。
    ///
    /// 没有图片的按内容缓存：图片的大小会变，带图片的每次重新排。
    pub(crate) fn flow(
        &mut self,
        inline: &Inline,
        x: f32,
        y: f32,
        width: f32,
        para: Para,
        out: &mut Vec<Item>,
    ) -> Flow {
        if !inline.images.is_empty() {
            return self.flow_at(inline, x, y, width, para, out);
        }
        let key = key(|h| {
            inline.hash(h);
            width.to_bits().hash(h);
            para.hash_into(h);
        });
        let source = u32::try_from(inline.source).unwrap_or(0);
        self.memo.touch(key);
        let piece = match self.memo.flows.get(&key) {
            Some(piece) => Arc::clone(piece),
            None => {
                let mut piece =
                    self.isolated(|cx, out| cx.flow_at(inline, 0.0, 0.0, width, para, out));
                piece.source = source;
                let piece = Arc::new(piece);
                self.memo.flows.insert(key, Arc::clone(&piece));
                piece
            }
        };
        self.paste(&piece, x, y, source, out);
        Flow {
            height: piece.height,
            first_baseline: piece.first_baseline.map(|b| b + y),
        }
    }

    /// 量一段行内文字（见 [`Cx::measure_raw`]），没有图片的按内容缓存。
    pub(crate) fn measure(&mut self, inline: &Inline, para: Para) -> (f32, f32) {
        if !inline.images.is_empty() {
            return self.measure_raw(inline, para);
        }
        let key = key(|h| {
            inline.hash(h);
            para.hash_into(h);
        });
        self.memo.touch(key);
        if let Some(&hit) = self.memo.measures.get(&key) {
            return hit;
        }
        let measured = self.measure_raw(inline, para);
        self.memo.measures.insert(key, measured);
        measured
    }
}

#[cfg(test)]
mod tests;
