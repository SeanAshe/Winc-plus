//! Markdown 预览的排版结果与位置查询。
//!
//! [`Laid`] 是整篇文档的排版结果，由 [`lay_out`] 按顶层块生成并缓存：文字变化时只重排变化的块，
//! 宽度、字号、配色或图片大小变化时整篇重排。位置查询在预览坐标与原文位置之间换算，
//! 供点击、选择、滚动同步和可编辑的预览使用。绘制在 `view::preview::markdown` 中。

use std::collections::HashMap;
use std::sync::Arc;

use winc_render::{Color, Frame, Rect};
use winc_typeset::{Colors, Cx, Env, Item, LaidBlock, Out, Raw, Span, margins};

pub(crate) use winc_typeset::{DETAILS, Memo};

use crate::app::App;

/// 正文字号与编辑器字号之比：编辑器字号 13 逻辑像素时，正文为 16 逻辑像素。
pub(crate) const BASE_RATIO: f32 = 16.0 / 13.0;
/// 正文最大宽度（逻辑像素，经验值）：更宽时居中、两侧留白。
pub(crate) const MAX_WIDTH: f32 = 880.0;
/// 区域不窄于 600 逻辑像素时文档四周的留白（逻辑像素，经验值）。
pub(crate) const PAD: f32 = 32.0;
/// 窄的时候（分栏，区域宽不足 600 逻辑像素）两边的留白（逻辑像素，经验值）。
pub(crate) const PAD_NARROW: f32 = 20.0;

/// 排版完成的文档。
pub(crate) struct Laid {
    pub(crate) key: Key,
    /// 排版依据的解析结果（`Arc` 的地址），换了说明文字改了。
    pub(crate) source: usize,
    /// 各顶层块：内容哈希和排版结果。
    pub(crate) blocks: Vec<(u64, Arc<LaidBlock>)>,
    /// 各顶层块的上边缘（相对内容顶部）。
    pub(crate) tops: Vec<f32>,
    /// 各顶层块的文字在整份文字（各块的 [`LaidBlock::text`] 连起来）里从第几个字符开始。
    pub(crate) starts: Vec<usize>,
    /// 整份文字的字符数。
    pub(crate) chars: usize,
    /// 各顶层块在原文里从第几行开始（见 `Markdown::lines`），分栏对齐滚动用。
    pub(crate) lines: Vec<usize>,
    /// 各顶层块在原文中的字节范围（见 `Markdown::ranges`），预览与原文互相定位用。
    pub(crate) ranges: Vec<std::ops::Range<usize>>,
    /// 排版时光标所在的叶子块，见 [`LiveKey`]。
    ///
    /// 不能编辑时为 `None`；光标换了叶子块（表格里换了一格）、或者打了字，这一块要重排。
    pub(crate) live: Option<LiveKey>,
    /// 上一次找的光标位置和结果（见 [`point_of`]）：滚动时光标不动，不必每帧在大块里再找。
    pub(crate) caret: std::cell::Cell<Option<(usize, Option<Point>)>>,
    /// 应画在上一显示行末尾的光标位置（原文字节偏移），见 [`Laid::set_upstream`]。
    pub(crate) upstream: std::cell::Cell<Option<usize>>,
    /// 内容总高度。
    pub(crate) height: f32,
    /// 上一次画的时候内容的左边缘（窗口坐标，逻辑像素），鼠标定位用。
    pub(crate) left: f32,
    /// 上一次画的时候内容四周的留白（逻辑像素），鼠标定位用。
    pub(crate) pad: f32,
    /// 比正文宽的表格的横向滚动位置（逻辑像素）：键为（顶层块序号，块中表格的序号，见
    /// [`LaidBlock::scrolls`]）。重排后按序号沿用。
    pub(crate) hscroll: HashMap<(usize, usize), f32>,
    /// 上一次横向滚动使其可见的光标位置（原文字节偏移），见 [`Laid::follow_caret`]。
    pub(crate) followed: Option<usize>,
}

/// 预览上的一个位置：左边缘（相对内容左边）、所在那一行铺底色的上边缘（和滚动位置同一个
/// 量法）和高度，均为逻辑像素；见 [`point_of`]。
pub(crate) type Point = (f32, f32, f32);

/// 决定排版结果的外部条件：任何一项变了都要整份重排。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Key {
    /// 正文宽度（逻辑像素，已减去两侧留白）。
    pub(crate) width: f32,
    /// 正文字号（逻辑像素）。
    pub(crate) base: f32,
    /// 正文颜色。
    pub(crate) text: Color,
    /// 底色，决定差异色、强调色混在白底还是黑底上。
    pub(crate) background: Color,
    /// 图片的版本号，有图片读好后就加一。
    pub(crate) images: u64,
}

/// 重排之前各顶层块的排版和绘制位置，重排后据此计算过渡动画（见 `BlockMotion`）。
pub(crate) struct Before {
    /// 各块的内容哈希、排版结果和绘制时的上边缘（相对内容顶部、已减去滚动位置，含滑动偏移）。
    pub(crate) blocks: Vec<(u64, Arc<LaidBlock>, f32)>,
    /// 排版时的文档版本（见 [`LiveKey`]）；版本未变表示只移动了光标。
    pub(crate) revision: Option<u64>,
}

impl Before {
    pub(crate) fn of(app: &App, laid: &Laid) -> Before {
        let scroll = app.markdown.scroll();
        let motion = app.markdown.motion.as_ref();
        let blocks = laid
            .blocks
            .iter()
            .zip(&laid.tops)
            .enumerate()
            .map(|(i, ((hash, block), top))| {
                let offset = motion.map_or(0.0, |m| m.offset(i));
                (*hash, Arc::clone(block), top + offset - scroll)
            })
            .collect();
        Before {
            blocks,
            revision: laid.live.as_ref().map(|k| k.2),
        }
    }

    /// 计算重排为 `laid`（滚动位置 `scroll`）后各块的起始偏移（见 `BlockMotion::from`）和淡入的项。
    ///
    /// 块数不变时按序号对应；块数变化（删除了一块、解析出新的块）时，按开头和末尾哈希相同的块
    /// 对应，中间新增的块随前一块移动。仅移动光标时，外观变化的块中新增的标记（次要色 `muted`
    /// 的文字中比原来多出的段）淡入。
    ///
    /// 文字已修改时各块直接到位、偏移全为 0，输入的结果在同一帧显示在最终位置；`slide` 为真
    /// （表格工具增删行列）时仍然滑动。
    pub(crate) fn motion(
        &self,
        laid: &Laid,
        scroll: f32,
        muted: Color,
        slide: bool,
    ) -> (Vec<f32>, crate::app::preview::markdown::Fading) {
        let (old, new) = (&self.blocks, &laid.blocks);
        let matched: Vec<Option<usize>> = if old.len() == new.len() {
            (0..new.len()).map(Some).collect()
        } else {
            let same = |i: usize, j: usize| new[i].0 == old[j].0;
            let short = old.len().min(new.len());
            let head = (0..short).take_while(|&i| same(i, i)).count();
            let tail = (0..short - head)
                .take_while(|&k| same(new.len() - 1 - k, old.len() - 1 - k))
                .count();
            (0..new.len())
                .map(|i| {
                    if i < head {
                        Some(i)
                    } else if i >= new.len() - tail {
                        Some(i + old.len() - new.len())
                    } else {
                        None
                    }
                })
                .collect()
        };
        let mut from = Vec::with_capacity(new.len());
        let mut last = 0.0;
        for (i, j) in matched.iter().enumerate() {
            if let Some(j) = j {
                last = old[*j].2 - (laid.tops[i] - scroll);
            }
            from.push(last);
        }
        let moved = laid.live.as_ref().map(|k| k.2) == self.revision && self.revision.is_some();
        if !moved && !slide {
            from.fill(0.0);
        }
        let mut fading = Vec::new();
        for (i, j) in matched.iter().enumerate() {
            let Some(j) = *j else { continue };
            if !moved || new[i].0 == old[j].0 {
                continue;
            }
            let marks = |block: &LaidBlock| {
                block
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(k, item)| match item {
                        Item::Text { text, color, .. }
                            if *color == muted && !text.trim().is_empty() =>
                        {
                            Some((k, Arc::clone(text)))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let mut seen: HashMap<Arc<str>, usize> = HashMap::new();
            for (_, text) in marks(&old[j].1) {
                *seen.entry(text).or_default() += 1;
            }
            let fresh: Box<[usize]> = marks(&new[i].1)
                .into_iter()
                .filter(|(_, text)| match seen.get_mut(text) {
                    Some(n) if *n > 0 => {
                        *n -= 1;
                        false
                    }
                    _ => true,
                })
                .map(|(k, _)| k)
                .collect();
            if !fresh.is_empty() {
                fading.push((i, fresh));
            }
        }
        (from, fading)
    }
}

/// 可编辑的预览中显示原文的块的缓存键；各字段均不变时沿用上一次的排版。
///
/// 包含顶层块、叶子块和文档版本；位于表格或正文中时还包含光标位置（切换单元格需要重排，
/// 正文中光标接触的标记需要显示）；位于代码块的首尾几行时包含所在的行（围栏行需要显示，
/// 见 `LiveLeaf::edge`）。
pub(crate) type LiveKey = (usize, std::ops::Range<usize>, u64, Option<usize>, String);

/// `preedit` 是输入法正在组合的文字，在正文中排入光标所在的行，因此其变化同样需要重排。
pub(crate) fn live_key(live: &crate::app::preview::markdown::LiveLeaf, preedit: &str) -> LiveKey {
    (
        live.block,
        live.leaf.clone(),
        live.revision,
        (live.table || live.prose).then_some(live.at).or(live.edge),
        if live.prose || live.table {
            preedit.to_owned()
        } else {
            String::new()
        },
    )
}

/// 可编辑的预览重排之前，光标（原文位置）在旧排版里的高度（不含上方留白）。
///
/// 文字已修改（版本或解析结果不同）时返回 `None`：修改文字只影响光标所在块及其以下的块，
/// 上方的块位置不变；此时按光标补偿反而会使页面随光标上下跳动（插入或删除表格行时光标换行，
/// 解析结果返回前后各排版一次）。
pub(crate) fn old_caret(
    app: &App,
    old: Option<&Laid>,
    source: usize,
    live: Option<&LiveKey>,
) -> Option<(usize, f32)> {
    let old = old.filter(|old| {
        old.source == source && old.live.as_ref().map(|k| k.2) == live.map(|k| k.2)
    })?;
    let source = app
        .markdown_live()
        .then(|| app.live_caret_source())
        .flatten()?;
    old.caret_point(source)
        .map(|(_, y, _)| (source, y - old.pad))
}

/// 重排后光标所在的那一行在屏幕上不动：光标换了叶子块时，上面的块从原文换回排版（或反过来），
/// 高度变了，内容整体上下错开；按光标的新旧高度之差滚动。
pub(crate) fn keep_caret_still(app: &mut App, still: Option<(usize, f32)>, laid: &Laid) {
    let Some((source, before)) = still else {
        return;
    };
    let Some((_, y, _)) = laid.caret_point(source) else {
        return;
    };
    let dy = y - laid.pad - before;
    if dy.abs() >= 0.5 {
        let scroll = app.markdown.scroll();
        app.markdown.scroll_to(scroll + dy);
    }
}

/// 按 `key` 排整份文档，`reuse` 里有同样内容的块直接沿用。
///
/// 可编辑的预览中光标所在的叶子块（`live`）显示原文，这一块不沿用，排出的结果也不留给其他
/// 时候用。
#[allow(clippy::too_many_arguments)]
pub(crate) fn lay_out(
    app: &mut App,
    frame: &mut Frame<'_>,
    parsed: &winc_preview::markdown::Markdown,
    key: Key,
    source: usize,
    live: Option<&crate::app::preview::markdown::LiveLeaf>,
    reuse: Vec<(u64, Arc<LaidBlock>)>,
) -> Laid {
    let reuse: HashMap<u64, Arc<LaidBlock>> = reuse.into_iter().collect();
    // 光标所在顶层块现在的原文，和叶子块在其中的范围。
    let raw = live.and_then(|live| {
        let rope = app.editor.active()?.text().rope()?;
        let snap = |b: usize| rope.char_to_byte(rope.byte_to_char(b.min(rope.len_bytes())));
        let (start, end) = (snap(live.range.start), snap(live.range.end));
        let text: Arc<str> = rope.byte_slice(start..end).to_string().into();
        let leaf =
            snap(live.leaf.start).saturating_sub(start)..snap(live.leaf.end).saturating_sub(start);
        let caret = snap(live.at).saturating_sub(start);
        let preedit = ((live.prose || live.table) && !app.preedit.is_empty())
            .then(|| Arc::from(&*app.preedit));
        let parsed_end = live.parsed_end.saturating_sub(start);
        Some((
            live.block, text, leaf, parsed_end, caret, live.prose, preedit,
        ))
    });
    let p = app.palette;
    app.markdown.memo.prepare(winc_typeset::memo_key(|h| {
        use std::hash::Hash;
        for v in [key.width, key.base] {
            v.to_bits().hash(h);
        }
        for c in [key.text, key.background] {
            [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(h);
        }
        key.images.hash(h);
    }));
    let doc = app.editor.active().and_then(|d| d.path.clone());
    let remote = app.config.markdown_remote_images != Some(false);
    let image_path =
        |url: &str| crate::app::preview::markdown::image_path(doc.as_deref(), url, remote);
    let token = |kind| p.token(kind);
    let mut cx = Cx {
        frame,
        base: key.base,
        token: &token,
        surface: p.editor_bg,
        accent: p.accent,
        colors: Colors::new(p.text, p.text_muted, p.accent, p.dark),
        dark: p.dark,
        images: &app.markdown.images,
        image_path: &image_path,
        wanted: Vec::new(),
        memo: &mut app.markdown.memo,
        toggled: &app.markdown.toggled,
        text: String::new(),
        chars: 0,
        raw: None,
        cell: None,
        preedit: false,
    };
    let env = Env {
        color: cx.colors.text,
        depth: 0,
    };
    let mut blocks = Vec::with_capacity(parsed.blocks.len());
    let mut tops = Vec::with_capacity(parsed.blocks.len());
    let mut starts = Vec::with_capacity(parsed.blocks.len());
    let (mut y, mut below, mut chars) = (0.0, None, 0);
    for (i, (block, &hash)) in parsed.blocks.iter().zip(&parsed.hashes).enumerate() {
        let raw = raw.as_ref().filter(|r| r.0 == i);
        // 显示原文的块换一个键存：光标离开后同样内容的块要重新按排版显示。
        let hash = match raw {
            Some((_, text, leaf, parsed_end, caret, prose, preedit)) => {
                winc_typeset::memo_key(|h| {
                    use std::hash::Hash;
                    (hash, &**text, leaf, parsed_end, caret, prose, preedit).hash(h);
                })
            }
            None => hash,
        };
        let laid = reuse.get(&hash).cloned().unwrap_or_else(|| {
            let mut out = Out::default();
            cx.text.clear();
            cx.chars = 0;
            cx.memo.take_touched();
            cx.raw = raw.map(|(_, text, leaf, parsed_end, caret, prose, preedit)| {
                Raw::new(
                    Arc::clone(text),
                    leaf.clone(),
                    *parsed_end,
                    *caret,
                    *prose,
                    preedit.clone(),
                )
            });
            cx.preedit = false;
            cx.cell = None;
            let mut flow = cx.block(block, 0.0, 0.0, key.width, env, &mut out);
            // 光标所在的叶子块在块里没有找到（写在列表里的公式这类没有原文对照的块）：整个
            // 顶层块显示原文。
            if let Some(raw) = cx.raw.as_mut().filter(|r| !r.done) {
                raw.leaf = 0..raw.block.len();
                raw.parsed_end = raw.block.len();
                raw.kind = raw.guess();
                out = Out::default();
                cx.text.clear();
                cx.chars = 0;
                flow = cx.raw_with_preview(block, 0.0, 0.0, key.width, env, &mut out);
            }
            cx.raw = None;
            let mut laid = LaidBlock::new(
                flow.height,
                out.items,
                out.anchors,
                cx.text.as_str().into(),
                cx.chars,
                cx.memo.take_touched(),
            );
            laid.preedit = cx.preedit;
            laid.scrolls = out.scrolls;
            Arc::new(laid)
        });
        starts.push(chars);
        chars += laid.chars;
        let (margin_top, margin_bottom) = margins(block, key.base, false);
        if let Some(below) = below {
            y += f32::max(below, margin_top);
        }
        tops.push(y);
        y += laid.height;
        below = Some(margin_bottom);
        blocks.push((hash, laid));
    }
    cx.memo.sweep(blocks.iter().map(|(_, b)| &*b.memo));
    let wanted = std::mem::take(&mut cx.wanted);
    for (path, url) in wanted {
        app.request_markdown_image(&path, &url);
    }
    Laid {
        key,
        source,
        blocks,
        tops,
        starts,
        chars,
        lines: parsed.lines.clone(),
        ranges: {
            // 解析之后打的字都在光标所在的块里：这一块换成现在的范围，原文在它后面的块按文字
            // 长度的变化挪。
            let mut ranges = parsed.ranges.clone();
            if let Some(live) = live
                && let Some(start) = ranges.get(live.block).map(|r| r.start)
            {
                let len = app
                    .editor
                    .active()
                    .and_then(|d| d.text().rope())
                    .map(|r| r.len_bytes());
                let delta = len.map_or(0, |len| len as i64 - parsed.len as i64);
                let shift = |at: usize| usize::try_from(at as i64 + delta).unwrap_or(0);
                for range in ranges.iter_mut().filter(|r| r.start > start) {
                    *range = shift(range.start)..shift(range.end);
                }
                ranges[live.block] = live.range.clone();
            }
            ranges
        },
        live: live.map(|l| live_key(l, &app.preedit)),
        caret: std::cell::Cell::new(None),
        upstream: std::cell::Cell::new(None),
        height: y,
        left: 0.0,
        pad: 0.0,
        hscroll: HashMap::new(),
        followed: None,
    }
}

/// 返回基线在 `baseline`、字号 `size` 的一行字在选中、查找时铺底色的范围：上边缘和高度
/// （逻辑像素）。
///
/// 上边缘在基线之上 1.05 倍字号处，高度取 1.4 倍字号（经验值，且能盖住上下标）。
pub(crate) fn line_box(baseline: f32, size: f32) -> (f32, f32) {
    (baseline - size * 1.05, size * 1.4)
}

impl Laid {
    /// `blocks` 范围里的各顶层块中能选中的文字，逐段返回：所在块的上边缘（相对内容顶部）、在
    /// 整份文字里的起点、横坐标、基线、字号、各字宽度和原文位置，以及所在的块和文字本身。
    pub(crate) fn spans(
        &self,
        blocks: std::ops::Range<usize>,
    ) -> impl Iterator<Item = (f32, usize, f32, f32, f32, &Span, usize, &str)> {
        let blocks = blocks.start.min(self.blocks.len())..blocks.end.min(self.blocks.len());
        self.blocks[blocks.clone()]
            .iter()
            .zip(blocks)
            .flat_map(move |((_, block), i)| {
                block
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(move |(k, item)| match item {
                        Item::Text {
                            x,
                            baseline,
                            size,
                            text,
                            span: Some(span),
                            ..
                        } => Some((
                            self.tops[i],
                            self.starts[i] + span.start,
                            *x - self.shift(i, k).0,
                            *baseline,
                            *size,
                            span,
                            i,
                            &**text,
                        )),
                        _ => None,
                    })
            })
    }

    /// 第 `block` 块第 `item` 项的横向滚动位置（逻辑像素）和所在表格的显示区域（块里的坐标）；
    /// 不在可横向滚动的表格中时为 `(0.0, None)`。
    pub(crate) fn shift(&self, block: usize, item: usize) -> (f32, Option<Rect>) {
        let Some((_, laid)) = self.blocks.get(block) else {
            return (0.0, None);
        };
        laid.scroller_of(item).map_or((0.0, None), |s| {
            (self.hscroll_of(block, s), Some(laid.scrolls[s].rect))
        })
    }

    /// 第 `block` 块第 `scroller` 个可横向滚动的表格当前的滚动位置，限制在可滚动的范围内。
    pub(crate) fn hscroll_of(&self, block: usize, scroller: usize) -> f32 {
        let max = self
            .blocks
            .get(block)
            .and_then(|(_, b)| b.scrolls.get(scroller))
            .map_or(0.0, |s| s.max());
        self.hscroll
            .get(&(block, scroller))
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, max)
    }

    /// 光标（原文位置 `source`）落在可横向滚动的表格中、位于显示区域之外时，横向滚动该表格使
    /// 光标可见。只在光标移动后对齐一次，不干扰用户自行滚动。
    pub(crate) fn follow_caret(&mut self, source: usize) {
        if self.followed == Some(source) {
            return;
        }
        self.followed = Some(source);
        let Some((x, y, _)) = self.caret_point(source) else {
            return;
        };
        let Some(block) = self
            .ranges
            .iter()
            .rposition(|r| r.start <= source && source <= r.end)
        else {
            return;
        };
        let y = y - self.pad - self.tops[block];
        let Some(s) = self.blocks[block]
            .1
            .scrolls
            .iter()
            .position(|s| (s.rect.y..s.rect.bottom()).contains(&y))
        else {
            return;
        };
        let rect = self.blocks[block].1.scrolls[s].rect;
        let max = self.blocks[block].1.scrolls[s].max();
        // 光标两侧至少留出的距离（逻辑像素），与单元格的内边距相近。
        let margin = (self.key.base * 0.8125).round();
        let now = self.hscroll_of(block, s);
        let next = if x < rect.x + margin {
            now - (rect.x + margin - x)
        } else if x > rect.right() - margin {
            now + (x - (rect.right() - margin))
        } else {
            return;
        };
        self.hscroll.insert((block, s), next.clamp(0.0, max));
        // 光标位置按滚动位置算过，滚动后重新计算。
        self.caret.set(None);
    }

    /// 内容坐标 `(x, y)`（相对内容左上角）处可横向滚动的表格：（块序号，表格序号，最大滚动距离）。
    pub(crate) fn scroller_at(&self, x: f32, y: f32) -> Option<(usize, usize, f32)> {
        let i = self.tops.partition_point(|&t| t <= y).checked_sub(1)?;
        let (_, block) = self.blocks.get(i)?;
        let y = y - self.tops[i];
        block
            .scrolls
            .iter()
            .position(|s| s.rect.contains(x, y))
            .map(|s| (i, s, block.scrolls[s].max()))
    }

    /// 返回整份文字里第 `from` 到 `to` 个字符。
    pub(crate) fn text(&self, from: usize, to: usize) -> String {
        let mut out = String::new();
        let first = self
            .starts
            .partition_point(|&s| s <= from)
            .saturating_sub(1);
        for (i, (_, block)) in self.blocks.iter().enumerate().skip(first) {
            let start = self.starts[i];
            if start >= to {
                break;
            }
            let (a, b) = (from.saturating_sub(start), (to - start).min(block.chars));
            out.extend(block.text.chars().skip(a).take(b.saturating_sub(a)));
        }
        out
    }

    /// 返回整份文字的字符数。
    pub(crate) fn chars(&self) -> usize {
        self.chars
    }

    /// 原文的行和预览滚动位置的对应点：文首、各块的开头、文末（原文共 `total` 行），行号和
    /// 位置都递增。
    ///
    /// 两点之间按比例算。
    pub(crate) fn sync_points(&self, total: usize) -> Vec<(f32, f32)> {
        let mut points = vec![(0.0, 0.0)];
        for (line, top) in self.lines.iter().zip(&self.tops) {
            let (line, y) = (*line as f32, self.pad + top);
            if points.last().is_some_and(|&(l, py)| line > l && y >= py) {
                points.push((line, y));
            }
        }
        let end = (total.max(1) as f32, self.height + self.pad * 2.0);
        if points.last().is_some_and(|&(l, y)| end.0 > l && end.1 >= y) {
            points.push(end);
        }
        points
    }

    /// 原文第 `line` 行（可以是小数）对应的预览滚动位置。
    pub(crate) fn y_of_line(&self, line: f32, total: usize) -> f32 {
        interpolate(&self.sync_points(total), line, false)
    }

    /// 预览滚到 `y` 时对应原文的第几行（小数）。
    pub(crate) fn line_of_y(&self, y: f32, total: usize) -> f32 {
        interpolate(&self.sync_points(total), y, true)
    }

    /// 原文第 `source` 个字节在预览上的位置（见 [`point_of`]），按上一次的结果缓存。
    pub(crate) fn caret_point(&self, source: usize) -> Option<Point> {
        if let Some((at, point)) = self.caret.get()
            && at == source
        {
            return point;
        }
        let point = point_of(self, source);
        self.caret.set(Some((source, point)));
        point
    }

    /// 指定应画在上一显示行末尾的光标位置（原文字节偏移，没有时为 `None`）。
    ///
    /// 折行处的原文位置同时是上一行的末尾和下一行的开头，[`point_of`] 默认取下一行的开头；
    /// 按 End 移到行尾后光标应留在本行。
    pub(crate) fn set_upstream(&self, source: Option<usize>) {
        if self.upstream.replace(source) != source {
            self.caret.set(None);
        }
    }

    /// 正文一栏的宽度（逻辑像素）。
    pub(crate) fn width(&self) -> f32 {
        self.key.width
    }

    /// 内容总高度（含四周留白）。
    ///
    /// 分栏对齐滚动时判断排版变了没有。
    pub(crate) fn height(&self) -> f32 {
        self.height + self.pad * 2.0
    }
}

/// 在递增的对应点 `(行, 位置)` 之间按比例换算：`inverse` 为假时由行算位置，为真时反过来。
pub(crate) fn interpolate(points: &[(f32, f32)], at: f32, inverse: bool) -> f32 {
    let key = |p: &(f32, f32)| if inverse { p.1 } else { p.0 };
    let value = |p: &(f32, f32)| if inverse { p.0 } else { p.1 };
    let i = points.partition_point(|p| key(p) <= at);
    match (i.checked_sub(1).map(|i| &points[i]), points.get(i)) {
        (Some(a), Some(b)) if key(b) > key(a) => {
            value(a) + (at - key(a)) / (key(b) - key(a)) * (value(b) - value(a))
        }
        (Some(a), _) => value(a),
        (None, _) => 0.0,
    }
}

/// 预览上窗口坐标 `(x, y)`（逻辑像素）处是整份文字里的第几个字符（落在两个字中间时取
/// 近的那边）；不在字上时取同一行最近的字，不在任何一行上时取上下最近的一行。
pub(crate) fn offset_at(app: &App, x: f32, y: f32) -> Option<usize> {
    hit(app, x, y).map(|hit| hit.offset)
}

/// 窗口横坐标 `x` 是否落在预览的内容一栏内（左右各放宽 [`CONTENT_SLACK`]）。
///
/// 内容一栏两侧的空白不对应任何文字：在那里按下鼠标不放光标、不开始选字。
pub(crate) fn in_content(app: &App, x: f32) -> bool {
    app.markdown.layout.as_ref().is_none_or(|laid| {
        let left = laid.left - CONTENT_SLACK;
        let right = laid.left + laid.key.width + CONTENT_SLACK;
        (left..=right).contains(&x)
    })
}

/// 内容一栏两侧仍按点在文字上处理的宽度（逻辑像素，经验值）：点在行首稍左处仍能把光标放到
/// 行首。
pub(crate) const CONTENT_SLACK: f32 = 12.0;

/// 预览上窗口坐标 `(x, y)` 处的字（取法同 [`offset_at`]）在原文中的位置（字节偏移）。
///
/// 由原文生成、没有对应原文的文字（提示块的标题）取所在顶层块的开头。
pub(crate) fn source_at(app: &App, x: f32, y: f32) -> Option<usize> {
    let hit = hit(app, x, y)?;
    let laid = app.markdown.layout.as_ref()?;
    let start = laid.ranges.get(hit.block)?.start;
    Some(start + hit.source.unwrap_or(0))
}

/// 把窗口坐标 `(x, y)` 处可横向滚动的表格横向滚动 `dx`（逻辑像素，向右滚动为负），返回是否
/// 滚动了；该处没有可滚动的表格时返回 `false`，由调用方按竖直方向滚动预览。
pub(crate) fn scroll_table(app: &mut App, x: f32, y: f32, dx: f32) -> bool {
    let (area, scroll) = (app.markdown.area, app.markdown.scroll());
    let Some(laid) = app.markdown.layout.as_mut() else {
        return false;
    };
    let (cx, cy) = (x - laid.left, y - (area.y + laid.pad - scroll));
    let Some((block, s, max)) = laid.scroller_at(cx, cy).filter(|&(.., max)| max > 0.0) else {
        return false;
    };
    let now = laid.hscroll_of(block, s);
    laid.hscroll.insert((block, s), (now - dx).clamp(0.0, max));
    app.markdown.last_table = Some((block, s));
    app.request_redraw();
    true
}

/// 鼠标落在的字：在整份文字里的位置、所在的顶层块、在原文中的位置（相对顶层块）。
pub(crate) struct Hit {
    pub(crate) offset: usize,
    pub(crate) block: usize,
    pub(crate) source: Option<usize>,
}

/// 返回鼠标位置（在块 `blocks` 中，内容坐标）下的表格单元格：所在的块及单元格范围（块内坐标）。
pub(crate) fn table_cell(
    laid: &Laid,
    blocks: std::ops::Range<usize>,
    x: f32,
    y: f32,
) -> Option<(usize, Rect)> {
    for block in blocks {
        let y = y - laid.tops[block];
        for (k, item) in laid.blocks[block].1.items.iter().enumerate() {
            let Item::Table {
                rect, cols, rows, ..
            } = item
            else {
                continue;
            };
            // 可横向滚动的表格：只认显示区域内的点，单元格按滚动位置左移。
            let (off, strip) = laid.shift(block, k);
            if strip.is_some_and(|s| !s.contains(x, y)) {
                continue;
            }
            let rect = &Rect::new(rect.x - off, rect.y, rect.w, rect.h);
            if !rect.contains(x, y) {
                continue;
            }
            let (mut right, mut bottom) = (rect.x, rect.y);
            let c = cols.iter().position(|w| {
                right += w;
                x < right
            })?;
            let r = rows.iter().position(|h| {
                bottom += h;
                y < bottom
            })?;
            let cell = Rect::new(right - cols[c], bottom - rows[r], cols[c], rows[r]);
            return Some((block, cell));
        }
    }
    None
}

pub(crate) fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    let laid = app.markdown.layout.as_ref()?;
    let origin = app.markdown.area.y + laid.pad - app.markdown.scroll();
    let (x, y) = (x - laid.left, y - origin);
    let mut best: Option<((f32, f32), Hit)> = None;
    // 只看鼠标所在的块和上下各一块。
    let i = laid.tops.partition_point(|&t| t <= y).saturating_sub(1);
    let blocks = i.saturating_sub(1)..(i + 2).min(laid.blocks.len());
    // 点击表格单元格时只在该单元格内查找文字，使光标不会进入相邻单元格；
    // 空单元格中有一段空的原文，可供定位。
    let table = table_cell(laid, blocks.clone(), x, y);
    let mut inside: Option<((f32, f32), Hit)> = None;
    for (top, start, ix, baseline, size, span, block, text) in laid.spans(blocks.clone()) {
        let (y0, h) = line_box(top + baseline, size);
        let width: f32 = span.advances.iter().sum();
        let dy = (y0 - y).max(y - (y0 + h)).max(0.0);
        let dx = (ix - x).max(x - (ix + width)).max(0.0);
        let here = table.as_ref().is_some_and(|(b, cell)| {
            *b == block && cell.contains(ix, y0 + h / 2.0 - laid.tops[*b])
        });
        let slot = if here { &mut inside } else { &mut best };
        if slot.as_ref().is_some_and(|(d, _)| *d <= (dy, dx)) {
            continue;
        }
        // 找离 `x` 最近的字缝。
        let mut pen = ix;
        let mut at = span.advances.len();
        for (k, w) in span.advances.iter().enumerate() {
            if x < pen + w / 2.0 {
                at = k;
                break;
            }
            pen += w;
        }
        let hit = Hit {
            offset: start + at,
            block,
            source: span.source_of(text, at),
        };
        *slot = Some(((dy, dx), hit));
    }
    if inside.is_some() {
        return inside.map(|(_, hit)| hit);
    }
    // 点击图片时光标移到图片的原文处，因为图片中没有可定位的文字。
    for block in blocks {
        let top = laid.tops[block];
        for (k, item) in laid.blocks[block].1.items.iter().enumerate() {
            let off = laid.shift(block, k).0;
            let spot = match item {
                Item::Image {
                    rect, at: Some(at), ..
                }
                | Item::Spot { rect, at } => Some((rect, at)),
                _ => None,
            };
            if let Some((rect, (start, source))) = spot
                && rect.contains(x + off, y - top)
            {
                return Some(Hit {
                    offset: laid.starts[block] + start,
                    block,
                    source: Some(*source as usize),
                });
            }
        }
    }
    best.map(|(_, hit)| hit)
}

/// 原文第 `source` 个字节在预览上的位置：左边缘（相对内容左边）、所在那一行铺底色的上边缘
/// （和滚动位置同一个量法）和高度；落在没有文字的块（分隔线、图）里时为 `None`。
///
/// 取原文位置不超过 `source` 的最后一个字之前；落在标记里（`**`、链接地址）时就是标记前面
/// 那个字之后。
pub(crate) fn point_of(laid: &Laid, source: usize) -> Option<Point> {
    // 脚注排在最后，原文范围不按先后，从后往前找第一个包含它的块。
    let block = laid
        .ranges
        .iter()
        .rposition(|r| r.start <= source && source <= r.end)?;
    let upstream = laid.upstream.get() == Some(source);
    let source = source - laid.ranges[block].start;
    // 光标应画在上一显示行的末尾：取原文位置在它之前的最后一段，该段正好在此结束时用其末尾。
    if upstream
        && let Some((top, x, baseline, size, span, text)) = laid
            .spans(block..block + 1)
            .filter(|(.., span, _, _)| span.source.is_some_and(|s| (s as usize) < source))
            .max_by_key(|(.., span, _, _)| span.source)
            .map(|(top, _, x, baseline, size, span, _, text)| (top, x, baseline, size, span, text))
        && span.char_at_source(text, source) == Some(span.advances.len())
    {
        let x = x + span.advances.iter().sum::<f32>();
        let (y, h) = line_box(top + baseline, size);
        return Some((x, laid.pad + y, h));
    }
    // 先按每段第一个字找（每段只看一次），再在找到的那段里逐字找：长列表、长代码块是一个
    // 顶层块，每帧都要找，不逐字看整块。
    let (top, x, baseline, size, span, text) = laid
        .spans(block..block + 1)
        .filter(|(.., span, _, _)| span.source.is_some_and(|s| s as usize <= source))
        .max_by_key(|(.., span, _, _)| span.source)
        .map(|(top, _, x, baseline, size, span, _, text)| (top, x, baseline, size, span, text))?;
    let k = span.char_at_source(text, source)?;
    let x = x + span.advances[..k].iter().sum::<f32>();
    let (y, h) = line_box(top + baseline, size);
    Some((x, laid.pad + y, h))
}

/// 返回原文位置 `source` 处（取法同 [`point_of`]）文字的字体、模拟粗斜体和字号。
///
/// 输入法组字的文字按此样式绘制。
pub(crate) fn font_at(
    laid: &Laid,
    source: usize,
) -> Option<(winc_render::FontKind, winc_render::Faux, f32)> {
    let block = laid
        .ranges
        .iter()
        .rposition(|r| r.start <= source && source <= r.end)?;
    let source = source - laid.ranges[block].start;
    laid.blocks[block]
        .1
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text {
                font,
                faux,
                size,
                span: Some(span),
                ..
            } => span
                .source
                .filter(|&s| s as usize <= source)
                .map(|s| (s, (*font, *faux, *size))),
            _ => None,
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, font)| font)
}

/// 整份文字里第 `offset` 个字符所在那一行的上边缘（和滚动位置同一个量法）。
pub(crate) fn offset_top(app: &App, offset: usize) -> Option<f32> {
    let laid = app.markdown.layout.as_ref()?;
    let i = laid
        .starts
        .partition_point(|&s| s <= offset)
        .saturating_sub(1);
    laid.spans(i..i + 1)
        .find(|(_, start, _, _, _, span, _, _)| offset < start + span.advances.len())
        .map(|(top, _, _, baseline, size, ..)| laid.pad + line_box(top + baseline, size).0)
}

/// 整份文字（各块的文字连起来），预览里查找用。
pub(crate) fn full_text(app: &App) -> Option<String> {
    let laid = app.markdown.layout.as_ref()?;
    Some(laid.blocks.iter().map(|(_, b)| &*b.text).collect())
}

/// 复制按钮的「链接」地址前缀，后面接代码：点击时把代码放进剪贴板。
///
/// 开头的 `\u{1}` 不是合法的 URL 字符，用来和文档里的真实链接区分。
pub(crate) const COPY: &str = "\u{1}copy:";

/// 图的全屏按钮的「链接」地址：点击后全屏查看 `MarkdownPreview::expand` 中的图。
pub(crate) const EXPAND: &str = "\u{1}expand";

/// 勾选框链接的地址前缀，其后为 `[ ]` 在原文中的位置（字节偏移）；点击时切换勾选状态。
pub(crate) const TASK: &str = "\u{1}task:";
/// 表格工具栏按钮链接的地址前缀，其后为操作名（见 [`table_tools`]）。
pub(crate) const TABLE: &str = "\u{1}table:";

/// 标题锚点 `anchor`（`[跳转](#标题)` 里 `#` 后面的部分）在内容里的高度（逻辑像素）；
/// 找不到该标题时返回 `None`。
///
/// `anchor` 先做百分号解码再转小写；以 `fnref-` 开头的锚点指向引用脚注的编号。
pub(crate) fn anchor_top(app: &App, anchor: &str) -> Option<f32> {
    anchor_box(app, anchor).map(|(top, _)| top)
}

/// 锚点 `anchor` 的范围：上边缘（取法同 [`anchor_top`]）和高度。脚注引用是编号所在那一行，脚注是
/// 整条脚注，标题是标题所在的块。
pub(crate) fn anchor_box(app: &App, anchor: &str) -> Option<(f32, f32)> {
    let laid = app.markdown.layout.as_ref()?;
    let anchor = crate::app::preview::markdown::percent_decode(anchor).to_lowercase();
    // 脚注末尾的 ↩ 跳回正文里第一次引用该脚注的位置，即链到脚注的编号。
    let reference = anchor.strip_prefix("fnref-").map(|n| format!("#fn-{n}"));
    laid.blocks
        .iter()
        .zip(&laid.tops)
        .find_map(|((_, block), top)| {
            let (y, h) = match &reference {
                Some(url) => block.items.iter().find_map(|item| match item {
                    Item::Link { rect, url: u } if **u == **url => Some((rect.y, rect.h)),
                    _ => None,
                }),
                None => block
                    .anchors
                    .iter()
                    .find(|(slug, _)| *slug == anchor)
                    .map(|(_, y)| (*y, block.height - y)),
            }?;
            Some((top + y, h))
        })
}
