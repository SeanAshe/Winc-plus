//! Markdown 预览每一帧的排版：沿用上一帧的结果、就地刷新保留的范围（[`Near`]），或重新排版。

use std::sync::Arc;

use winc_render::{Frame, Rect};

use crate::app::App;
use crate::app::preview::markdown::LiveLeaf;
use crate::app::preview::markdown::laid::*;
use crate::layout::drawn_scroll;

use super::room_below;

/// 本帧排版的依据。
pub(super) struct LayoutInput<'a> {
    /// 解析结果。
    pub(super) parsed: &'a Arc<winc_preview::markdown::Markdown>,
    /// 排版条件。
    pub(super) key: Key,
    /// 解析结果的标识（`Arc` 的地址），见 [`Laid::source`]。
    pub(super) source: usize,
    /// 光标所在的叶子块，不能编辑时为 `None`。
    pub(super) live: Option<&'a LiveLeaf>,
    /// `live` 的标识，见 [`LiveKey`]。
    pub(super) live_key: Option<&'a LiveKey>,
    /// 需要保留排版结果的范围。
    pub(super) near: Near,
    /// 预览区域。
    pub(super) area: Rect,
    /// 内容四周的留白（逻辑像素）。
    pub(super) pad: f32,
    /// 刚读好的图片。
    pub(super) loaded: Vec<std::path::PathBuf>,
    /// 刚开合的折叠块的链接地址。
    pub(super) toggled: Option<String>,
    /// 是否刚收起或展开了标题。
    pub(super) refold: bool,
}

/// 本帧的排版结果。
pub(super) struct LayoutOutput {
    /// 排版结果。
    pub(super) laid: Laid,
    /// 重排之前光标在旧排版里的高度，见 [`old_caret`]。
    pub(super) still: Option<(usize, f32)>,
    /// 重排之前各块的绘制位置和是否滑动，见 [`Before`]。
    pub(super) before: Option<(Before, bool)>,
}

/// 按 `input` 排出本帧的 Markdown 文档。
///
/// 排版条件和文字都未变时沿用上一帧的结果；视口移到保留的范围之外时就地刷新该范围
/// （[`Laid::refresh_near`]）；其余情况重新排版，内容未变的块沿用旧结果。
pub(super) fn resolve_layout(
    app: &mut App,
    frame: &mut Frame<'_>,
    input: LayoutInput<'_>,
) -> LayoutOutput {
    let LayoutInput {
        parsed,
        key,
        source,
        live,
        live_key,
        near,
        area,
        pad,
        loaded,
        toggled,
        refold,
    } = input;
    let mut still = None;
    let mut before = None;
    // 排版条件、文字和光标所在的叶子块都未变，也没有图片读好、块开合或未排完的块。
    let unchanged = |laid: &Laid| {
        laid.key == key
            && laid.source == source
            && laid.live.as_ref() == live_key
            && loaded.is_empty()
            && toggled.is_none()
            && !refold
            && !laid.pending()
    };
    let laid = match app.preview.markdown.layout.take() {
        Some(laid)
            if unchanged(&laid)
                && (laid.near.as_ref()).is_none_or(|kept| kept.covers(&near, area.h)) =>
        {
            laid
        }
        // 只滚动或跳转到了保留的范围之外：就地刷新，选区、查找结果和光标位置保留。
        Some(mut laid) if unchanged(&laid) => {
            laid.refresh_near(app, frame, parsed, live, near);
            laid
        }
        old => {
            let hscroll = old.as_ref().map(|l| l.hscroll.clone()).unwrap_or_default();
            let followed = old
                .as_ref()
                .filter(|l| l.source == source && l.live.as_ref() == live_key)
                .and_then(|l| l.followed);
            // 继续上一帧未完成的排版：排版条件和文字都未变，选区、查找结果和光标位置保留。
            let resumed = old.as_ref().is_some_and(|l| {
                l.key == key
                    && l.source == source
                    && l.live.as_ref() == live_key
                    && loaded.is_empty()
                    && toggled.is_none()
                    && !refold
            });
            // 文字变了：选区、查找用的文字作废。只是重排（宽度、图片变了）时选区留着。
            if old.as_ref().is_none_or(|l| l.source != source) {
                app.preview.markdown.selection = None;
            }
            if !resumed {
                app.preview.markdown.find_doc = None;
                let clicked = app.preview.markdown.clicked.take();
                still = old_caret(app, old.as_ref(), source, live_key, clicked);
                let slide = std::mem::take(&mut app.preview.markdown.slide);
                // 在预览中输入了文字：各块直接到位（见 `Before::motion`），不记录各块的位置。
                let revision = app.editor.active().map(|d| d.revision);
                let typed = (old.as_ref())
                    .and_then(|l| l.live.as_ref())
                    .is_some_and(|k| Some(k.2) != revision);
                if typed && !slide && !refold {
                    app.preview.markdown.motion = None;
                } else {
                    // 收起、展开标题时各块同样滑到新位置，不只在可编辑的预览中。
                    before = old
                        .as_ref()
                        .filter(|l| l.key == key && (app.markdown_live() || refold))
                        .map(|l| (Before::of(app, l), slide));
                }
            }
            // 文字未变时保留旧的各块供沿用，并记录视口顶边所在的块，排版期间保持其屏幕位置。
            let same = (old.as_ref())
                .filter(|l| l.source == source && l.blocks.len() == parsed.blocks.len());
            let top = drawn_scroll(app.preview.markdown.scroll());
            // 顶边在该块里所占的比例：块的高度变了，顶边仍在同一块的相同比例处。
            let held = same.and_then(|l| {
                let (i, off) = l.place_at(top - l.pad)?;
                Some((i, off / room_below(l, i).max(1.0)))
            });
            let mut window = same.map(|l| l.blocks_in(top - l.pad - area.h, top + area.h * 2.0));
            let mut kept = same.map(|l| l.blocks.clone());
            let was_pending = same.is_some_and(Laid::pending);
            // 有图片刚读好时，用到它的块大小变了，需要重排；折叠块开合时，它所在的块需要重排；
            // 其余块沿用上次的结果。
            let mut reuse = old.filter(|l| l.key == key).map(|l| {
                l.fresh()
                    .into_iter()
                    .filter(|(_, block)| {
                        !block.uses_any(&loaded)
                            && toggled.as_deref().is_none_or(|url| !block.links_to(url))
                    })
                    .collect()
            });
            let deadline = app.preview.markdown.catch_up.deadline();
            // 视口上方的块高度变化后，视口内的块可能与排版前估计的不同：仍有未排的块时再排一轮。
            let mut laid = loop {
                let later = (kept.as_deref().zip(window.clone())).map(|(old, window)| Later {
                    old,
                    window,
                    deadline,
                });
                let fresh = reuse.take().unwrap_or_default();
                let laid = lay_out(
                    app,
                    frame,
                    parsed,
                    key,
                    source,
                    live,
                    fresh,
                    later,
                    Some(near.clone()),
                );
                let y = held.map_or(top - pad, |(i, share)| {
                    laid.tops[i] + share.min(1.0) * room_below(&laid, i)
                });
                let seen = laid.blocks_in(y, y + area.h);
                let waits = |i: usize| laid.stale.get(i).copied().unwrap_or(false);
                if window.as_ref() == Some(&seen) || !seen.clone().any(waits) {
                    break laid;
                }
                (reuse, window) = (Some(laid.fresh()), Some(seen));
                kept = Some(laid.blocks);
            };
            // 分帧排版、图片加载完成、收起或展开标题后，视口顶边所在块的屏幕位置不变。
            let anchored = was_pending || laid.pending() || !loaded.is_empty() || refold;
            if let Some((i, share)) = held.filter(|_| anchored) {
                let y = laid.tops[i] + share.min(1.0) * room_below(&laid, i) + pad;
                app.preview.markdown.set_scroll(y);
                still = None;
            }
            app.preview.markdown.catch_up.pending = laid.pending();
            // 释放了要画的项的块不再使用排版缓存，缓存只保留其余的块用到的条目。
            if laid.near.is_some() {
                let used = laid.blocks.iter().map(|(_, block)| &*block.memo);
                app.preview.markdown.memo.keep_only(used);
            }
            laid.hscroll = hscroll;
            laid.followed = followed;
            laid
        }
    };
    LayoutOutput {
        laid,
        still,
        before,
    }
}
