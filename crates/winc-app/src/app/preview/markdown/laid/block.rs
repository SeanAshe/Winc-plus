//! 排一个顶层块：排版上下文、光标所在顶层块的原文，以及单块排版。整篇排版（[`super::lay_out`]）
//! 与保留范围的就地刷新（[`Laid::refresh_near`](super::Laid::refresh_near)）共用。

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use winc_preview::markdown::{Block, Markdown};
use winc_render::Frame;
use winc_typeset::{Colors, Cx, Drift, Env, LaidBlock, Out, Raw};

use super::Key;
use crate::app::preview::markdown::LiveLeaf;
use crate::app::{App, Focus};

/// 光标所在顶层块现在的原文，以及叶子块、光标在其中的位置（字节偏移，相对顶层块）。
///
/// 原文含解析之后的修改，排版时按 [`Raw`] 显示。
#[derive(Hash)]
pub(super) struct LiveRaw {
    /// 顶层块的序号。
    pub(super) block: usize,
    /// 顶层块现在的原文。
    text: Arc<str>,
    /// 叶子块在 `text` 中的范围。
    leaf: Range<usize>,
    /// 叶子块在解析结果中的终点，见 [`Raw::parsed_end`]。
    parsed_end: usize,
    /// 顶层块内解析之后的改动，见 [`Raw::drift`]。
    drift: Drift,
    /// 光标在 `text` 中的位置。
    caret: usize,
    /// 叶子块是否为正文（段落、标题）。
    prose: bool,
    /// 输入法正在组合的文字；只排入正文和表格。
    preedit: Option<Arc<str>>,
}

/// 读取光标所在叶子块 `live` 的顶层块现在的原文；没有光标或文档不是文本时返回 `None`。
///
/// `parsed` 是排版依据的解析结果。
pub(super) fn live_raw(app: &App, parsed: &Markdown, live: Option<&LiveLeaf>) -> Option<LiveRaw> {
    let live = live?;
    let rope = app.editor.active()?.text().rope()?;
    let snap = |b: usize| rope.char_to_byte(rope.byte_to_char(b.min(rope.len_bytes())));
    let (start, end) = (snap(live.range.start), snap(live.range.end));
    let preedit = app.preedit_at(Focus::Editor);
    // 光标所在的顶层块位于改动之后时，块的起点已换算，块内的相对位置不变。
    let parsed_start = parsed.ranges.get(live.block).map_or(start, |r| r.start);
    Some(LiveRaw {
        block: live.block,
        text: rope.byte_slice(start..end).to_string().into(),
        leaf: snap(live.leaf.start).saturating_sub(start)
            ..snap(live.leaf.end).saturating_sub(start),
        parsed_end: live.parsed_end.saturating_sub(start),
        drift: live.drift.within(parsed_start),
        caret: snap(live.at).saturating_sub(start),
        prose: live.prose,
        preedit: ((live.prose || live.table) && !preedit.is_empty()).then(|| Arc::from(preedit)),
    })
}

/// 块中是否有开合状态（`<details>` 折叠块、mermaid 图）：这样的块按内容相同的块中的序号区分
/// 开合状态（见 `Cx::repeat`）。
pub(super) fn has_toggles(block: &Block) -> bool {
    match block {
        Block::Details { .. } => true,
        Block::Code { lang, .. } => lang == "mermaid",
        Block::Quote(_, inner) | Block::Footnote(_, inner) => inner.iter().any(has_toggles),
        Block::List(_, items) => (items.iter()).any(|item| item.blocks.iter().any(has_toggles)),
        _ => false,
    }
}

/// 按排版条件 `key` 准备排版缓存，以当前的配色、图片和开合状态构造排版上下文，交给 `f`。
///
/// 排版条件与上次不同时缓存全部失效（见 [`winc_typeset::Memo::prepare`]）。
pub(super) fn with_cx<R>(
    app: &mut App,
    frame: &mut Frame<'_>,
    key: Key,
    f: impl FnOnce(&mut Cx<'_, '_>, Env) -> R,
) -> R {
    let markdown = &mut app.preview.markdown;
    markdown.memo.prepare(winc_typeset::memo_key(|h| {
        use std::hash::Hash;
        for v in [key.width, key.base, key.tall] {
            v.to_bits().hash(h);
        }
        for c in [key.text, key.background] {
            [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(h);
        }
        key.images.hash(h);
        key.numbers.hash(h);
    }));
    let p = app.palette;
    let doc = app.editor.active().and_then(|d| d.path.clone());
    let remote = app.config.markdown_remote_images != Some(false);
    let image_path =
        |url: &str| crate::app::preview::markdown::image_path(doc.as_deref(), url, remote);
    let token = |kind| p.token(kind);
    let markdown = &mut app.preview.markdown;
    let mut cx = Cx {
        frame,
        base: key.base,
        token: &token,
        surface: p.editor_bg,
        accent: p.accent,
        colors: Colors::new(p.text, p.text_muted, p.accent, p.dark),
        dark: p.dark,
        images: &markdown.images,
        image_path: &image_path,
        wanted: Vec::new(),
        memo: &mut markdown.memo,
        toggled: &markdown.toggled,
        text: String::new(),
        chars: 0,
        raw: None,
        cell: None,
        preedit: false,
        line_numbers: key.numbers,
        tall: key.tall,
        repeat: 0,
        toggles: HashMap::new(),
        bleed: key.bleed,
    };
    let env = Env {
        color: cx.colors.text,
        depth: 0,
    };
    f(&mut cx, env)
}

/// 排一个宽 `width` 的顶层块 `block`；`raw` 是光标所在的这一块的原文，显示为原文。
///
/// 调用前设好 `cx.repeat`。排出的块记下本次用到的缓存条目。
pub(super) fn lay_block(
    cx: &mut Cx<'_, '_>,
    block: &Block,
    width: f32,
    env: Env,
    raw: Option<&LiveRaw>,
) -> LaidBlock {
    let mut out = Out::default();
    cx.text.clear();
    cx.chars = 0;
    cx.toggles.clear();
    cx.memo.take_touched();
    cx.raw = raw.map(|r| {
        let (text, preedit) = (Arc::clone(&r.text), r.preedit.clone());
        Raw::new(
            text,
            r.leaf.clone(),
            r.parsed_end,
            r.drift,
            r.caret,
            r.prose,
            preedit,
        )
    });
    cx.preedit = false;
    cx.cell = None;
    let mut flow = cx.block(block, 0.0, 0.0, width, env, &mut out);
    // 光标所在的叶子块在块里没有找到（写在列表里的公式这类没有原文对照的块）：整个
    // 顶层块显示原文。
    if let Some(raw) = cx.raw.as_mut().filter(|r| !r.done) {
        raw.leaf = 0..raw.block.len();
        raw.parsed_end = raw.block.len();
        raw.kind = raw.guess();
        out = Out::default();
        cx.text.clear();
        cx.chars = 0;
        flow = cx.raw_with_preview(block, 0.0, 0.0, width, env, &mut out);
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
    laid.preview = out.preview;
    laid
}
