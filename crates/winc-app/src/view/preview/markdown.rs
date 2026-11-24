//! Markdown 预览的排版和绘制，字号与间距都按逻辑像素算。
//!
//! 正文是编辑器字号的 16/13 倍，编辑器默认 13 时正文 16；行高 1.5 倍，段落之间 16 逻辑像素，
//! 标题为正文的 2/1.5/1.25/1/0.875/0.85 倍。排版在 `layout` 子模块里做，要量字宽，所以在绘制
//! 时进行，结果按顶层块缓存（[`Laid`]）：文字改了只重排变了的块，宽度、字号、配色或图片大小
//! 变了才整份重排。换行时不在「，。」这类标点前面、「（“」这类标点后面断。

use std::sync::Arc;

use winc_i18n::t;
use winc_render::{Color, FontKind, Frame, Rect, Stroke};

use crate::app::Hover;
use crate::app::preview::markdown::resize::ImageAt;
use crate::app::preview::markdown::{ImageSlot, TableBar, TocTarget};
use crate::app::{App, Focus, ScrollArea};
use crate::icons;
use crate::theme::Palette;

mod resize;
mod table;
mod toolbar;
mod viewer;

use table::{draw_table_fades, draw_table_scrollbar, table_toolbar};

pub(in crate::view) use viewer::draw_diagram_view;
use winc_typeset::Item;

use crate::app::preview::markdown::laid::*;
use crate::layout::drawn_scroll;

/// 以 `(left, top)` 为块的左上角绘制 `items` 中的底色和文字，裁剪到 `clip`。
fn draw_plain(frame: &mut Frame<'_>, items: &[Item], (left, top): (f32, f32), clip: Rect) {
    let at = |r: &Rect| Rect::new(left + r.x, top + r.y, r.w, r.h);
    for item in items {
        match item {
            Item::Fill {
                rect,
                color,
                radius,
            } => frame.rect_clipped(at(rect), *color, *radius, clip),
            Item::Corners { rect, color, radii } => {
                frame.rect_corners(at(rect), *color, *radii, clip);
            }
            Item::Text {
                x,
                baseline,
                text,
                font,
                faux,
                size,
                color,
                ..
            } => {
                frame.text_faux(
                    *font,
                    *faux,
                    *size,
                    left + x,
                    top + baseline,
                    text,
                    *color,
                    clip,
                );
            }
            _ => {}
        }
    }
}

/// 在指向不存在的文件或标题的链接下方绘制错误色的虚线。
fn draw_broken_links(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let color = app.palette.error.alpha(0.7);
    // 检查链接需要可变借用 `app`，检查期间暂时移出本帧的链接。
    let links = std::mem::take(&mut app.hits.markdown_links);
    for (rect, url) in &links {
        // 预览控件的地址以 `\u{1}` 开头，不检查。
        if url.starts_with('\u{1}') || app.check_markdown_link(url).is_none() {
            continue;
        }
        let y = (rect.bottom() - 1.5).round();
        let mut x = rect.x;
        while x < rect.right() {
            let w = 2.0f32.min(rect.right() - x);
            frame.rect_clipped(Rect::new(x, y, w, 1.0), color, 0.0, area);
            x += 4.0;
        }
    }
    app.hits.markdown_links = links;
}

/// 在 `area`（窗口坐标，逻辑像素）里画当前文档的 Markdown 预览，分栏时是右半边，只看预览时
/// 是整个编辑区。
///
/// 排版结果和光标位置写回 `app.preview.markdown`，可点击范围记入 `app.hits`；
/// 需要读取的图片记进请求，下一帧再画。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    app.preview.markdown.area = area;
    // 可编辑的预览在排版之前取得当前文字的解析结果，本帧即按最新的结构显示。
    app.parse_markdown_inline();
    // 第一次解析还没回来：先留空。
    let Some(parsed) = app.preview.markdown.parsed.clone() else {
        return;
    };
    let pad = if area.w >= 600.0 { PAD } else { PAD_NARROW };
    let widest = match app.config.markdown_text_width.unwrap_or_default() {
        winc_config::TextWidth::Standard => MAX_WIDTH,
        winc_config::TextWidth::Wide => WIDE_WIDTH,
        winc_config::TextWidth::Full => f32::MAX,
    };
    let width = (area.w - pad * 2.0).clamp(1.0, widest).floor();
    let left = (area.x + (area.w - width) / 2.0).round();
    // 有标题列表时正文左侧为指示条留出位置，留白不足时正文右移并变窄。钉住的列表占一列，
    // 同样由正文让出；让出后正文窄于 `TOC_PINNED_BODY` 时只保留指示条的位置。
    let numbers = number_room(app, frame);
    let narrowed = |to: f32| (area.right() - pad - to).floor();
    let outline = has_outline(&parsed);
    let bars = area.x + TOC_ROOM + numbers;
    let pushed = area.x + TOC_PINNED_EDGE + TOC_PINNED_COLUMN + numbers;
    let pinned = app.state.markdown_toc_pinned && narrowed(pushed) >= TOC_PINNED_BODY;
    let (left, width) = match (outline, pinned) {
        (true, true) if left < pushed => (pushed, width.min(narrowed(pushed))),
        (true, _) if left < bars => (bars, width.min(narrowed(bars)).max(1.0)),
        _ => (left, width),
    };
    // 钉住状态变化时正文按新宽度排版，左边缘从原位置过渡到新位置，与标题列表的过渡同步。
    app.preview.markdown.toc_left = left;
    let left = match (app.preview.markdown.toc_pin, app.preview.markdown.toc_slide) {
        (Some((_, clock)), Some(from)) => clock.between(from, left),
        _ => {
            app.preview.markdown.toc_slide = None;
            left
        }
    };
    let base = body_size(app);
    let key = Key {
        width,
        base,
        text: p.text,
        background: p.editor_bg,
        images: app.preview.markdown.images_version,
        // 可编辑的预览在左侧按原文编行号，代码块里不再另编。
        numbers: app.markdown_line_numbers_on() && !app.markdown_live(),
        tall: (area.h * TALL_SHARE).round().max(TALL_MIN),
    };
    let source = Arc::as_ptr(&parsed) as usize;
    let live = app.live_editing().then(|| app.live_leaf()).flatten();
    let live_key = (live.as_ref()).map(|l| live_key(l, app.preedit_at(Focus::Editor)));
    let blank = live.as_ref().is_some_and(|l| l.blank);
    let loaded = std::mem::take(&mut app.preview.markdown.loaded);
    let toggled = app.preview.markdown.toggled_now.take();
    let refold = std::mem::take(&mut app.preview.markdown.refold);
    // 重排之前光标在旧排版里的高度：重排后据此保持光标在屏幕上不动（见 [`keep_caret_still`]）。
    let mut still = None;
    // 记录重排之前各块的绘制位置，重排后各块从该位置滑动到新位置（见 `BlockMotion`）。
    let mut before = None;
    // 很长的文档只保留这个范围内的块的排版结果：视口前后各两屏，以及光标、选区所在的块前后。
    let near = {
        let view = drawn_scroll(app.preview.markdown.scroll()) - pad;
        let around = |block: usize| block.saturating_sub(NEAR_BLOCKS)..block + NEAR_BLOCKS + 1;
        let chosen = app.preview.markdown.selection;
        let picked = (app.preview.markdown.layout.as_ref().zip(chosen)).map(|(laid, (a, b))| {
            let block = |at: usize| laid.starts.partition_point(|&s| s <= at).saturating_sub(1);
            block(a.min(b)).saturating_sub(1)..block(a.max(b)) + 2
        });
        let caret = live.as_ref().map(|live| around(live.block));
        Near {
            rows: (view - area.h * 2.0, view + area.h * 3.0),
            blocks: caret.into_iter().chain(picked).collect(),
        }
    };
    let mut laid = match app.preview.markdown.layout.take() {
        Some(laid)
            if laid.key == key
                && laid.source == source
                && laid.live == live_key
                && loaded.is_empty()
                && toggled.is_none()
                && !refold
                && !laid.pending()
                && (laid.near.as_ref()).is_none_or(|kept| kept.covers(&near, area.h)) =>
        {
            laid
        }
        old => {
            let hscroll = old.as_ref().map(|l| l.hscroll.clone()).unwrap_or_default();
            let followed = old
                .as_ref()
                .filter(|l| l.source == source && l.live == live_key)
                .and_then(|l| l.followed);
            // 继续上一帧未完成的排版，或视口移到了保留的范围之外：排版条件和文字都未变，选区、
            // 查找结果和光标位置保留。
            let resumed = old.as_ref().is_some_and(|l| {
                l.key == key
                    && l.source == source
                    && l.live == live_key
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
                still = old_caret(app, old.as_ref(), source, live_key.as_ref(), clicked);
                let slide = std::mem::take(&mut app.preview.markdown.slide);
                // 收起、展开标题时各块同样滑到新位置，不只在可编辑的预览中。
                before = old
                    .as_ref()
                    .filter(|l| l.key == key && (app.markdown_live() || refold))
                    .map(|l| (Before::of(app, l), slide));
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
            // 有图片刚读好时，用到它的块大小变了，需要重排；折叠块开合时，它所在的块需要
            // 重排；其余块沿用上次的结果。
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
                    &parsed,
                    key,
                    source,
                    live.as_ref(),
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
    laid.left = left;
    laid.pad = pad;
    laid.set_upstream(app.live_upstream());
    app.preview.markdown.height = laid.height + pad * 2.0;
    // 可编辑的预览与编辑区相同：内容高于一屏时，最后一行下方可以滚出一段空白。
    if app.markdown_live() && app.preview.markdown.height > area.h {
        app.preview.markdown.height += area.h * crate::layout::OVERSCROLL;
    }
    keep_caret_still(app, still, &laid);
    app.preview.markdown.layout = Some(laid);
    app.follow_live_caret();
    // 分栏时右边跟着左边滚；刚换成只看预览时对齐一次。
    if area != app.layout.editor {
        app.sync_markdown_scroll();
    } else {
        app.align_markdown_once();
    }
    app.scroll_to_pending_anchor();
    app.hold_book_place();
    // `layout` 刚在上面放进去，取出时一定存在。
    let mut laid = app.preview.markdown.layout.take().expect("刚放进去");

    let max = (app.preview.markdown.height - area.h).max(0.0);
    let scroll = app.preview.markdown.scroll().min(max);
    let origin_y = area.y + pad - scroll;
    if let Some((before, slide)) = before {
        let muted = p.text_muted;
        let moves = before.motion(&laid, scroll, muted, slide);
        app.start_markdown_motion(moves);
    }
    let motion = app.preview.markdown.motion.take();
    // 拖着选文字时不出复制按钮。
    let mouse = if !app.dragging() {
        app.mouse()
    } else {
        (f32::NAN, f32::NAN)
    };
    // 分栏时编辑区光标在预览上的位置。
    let mirrored = (area != app.layout.editor)
        .then(|| app.markdown_caret_source())
        .flatten()
        .and_then(|source| laid.caret_point(source));
    // 标出编辑区光标所在的行，便于对照两侧的位置；键盘焦点在预览上时不标出。
    let caret = mirrored.filter(|_| !app.preview.markdown.focused);
    // 预览的文字中同样画出光标；在预览上点击后光标移到点中处，选中文字时不画。
    let picked = app.preview.markdown.selection.is_none_or(|(a, b)| a == b);
    let mirrored = mirrored.filter(|_| !app.preview.markdown.focused || picked);
    // 可编辑的预览：文字光标和选区（原文位置），画在预览上。在预览上点过、移过光标或者打过
    // 字之后才显示光标；刚打开、只是阅读时不显示，光标停在文首也不会落在居中的图片旁边。
    let editing = app.live_caret_shown();
    let live_caret = app
        .markdown_live()
        .then(|| app.live_caret_source())
        .flatten()
        .filter(|_| editing);
    if let Some(source) = live_caret {
        laid.follow_caret(source);
    }
    let live_selection = live_caret
        .and_then(|_| {
            let doc = app.editor.active()?;
            let rope = doc.text().rope()?;
            let (a, b) = (doc.selection.from(), doc.selection.to());
            let byte = |c: usize| rope.char_to_byte(c.min(rope.len_chars()));
            Some((byte(a), byte(b)))
        })
        .filter(|(a, b)| a < b);
    let focus = app.markdown_focus() && app.focus == crate::app::Focus::Editor;
    let resizable = app.markdown_live();
    let md = &mut app.preview.markdown;
    let selection = md
        .selection
        .map(|(a, b)| (a.min(b), a.max(b)))
        .filter(|(a, b)| a < b);
    // 查找条在查预览的文字时，标出匹配。
    let search = &app.bars.search;
    let matches = (app.layout.search.h > 0.0
        && md.find_doc.is_some()
        && search.document() == md.find_doc.as_ref().map(|d| d.id))
    .then(|| (search, search.current_match()));
    // 可编辑的预览中查找对着文档：匹配是原文中的位置（字符偏移），换算到每段文字里的字。
    let live_matches = (live_caret.is_some() && app.layout.search.h > 0.0)
        .then(|| app.editor.active())
        .flatten()
        .filter(|d| search.document() == Some(d.id))
        .and_then(|d| d.text().rope())
        .map(|rope| (rope, search.current_match()));
    let mut wanted = Vec::new();
    // 这一帧绘制范围内的图片。
    let mut images = Vec::new();
    let mut shown = Vec::new();
    let mut toolbar = None;
    // 可横向滚动的表格的滚动条，各块绘制完成后再绘制：（块，表格，显示区域，滚动位置）。
    let mut bars = Vec::new();
    let first = laid
        .tops
        .partition_point(|&t| origin_y + t <= area.y)
        .saturating_sub(1);
    // 专注模式：光标所在的块以外画得淡一些。
    let focused = live_caret.filter(|_| focus).and_then(|source| {
        let i = laid
            .ranges
            .partition_point(|r| r.start <= source)
            .checked_sub(1)?;
        (source <= laid.ranges[i].end).then_some(i)
    });
    for (i, (_, block)) in laid.blocks.iter().enumerate().skip(first) {
        let opacity = match focused {
            Some(b) if b != i => crate::theme::FOCUS_DIM,
            _ => 1.0,
        };
        let offset = |i: usize| motion.as_ref().map_or(0.0, |m| m.offset(i));
        let top = origin_y + laid.tops[i] + offset(i);
        if top > area.bottom() {
            break;
        }
        if top + block.height < area.y {
            continue;
        }
        let whole = area;
        // 滑动过程中每块只绘制到下一块（同样在滑动）的上边缘：变高的块逐渐显露，
        // 变矮的块不与下方滑上来的块重叠。
        let area = match &motion {
            Some(m) if m.moving() => {
                let next = laid
                    .tops
                    .get(i + 1)
                    .map_or(f32::INFINITY, |t| origin_y + t + offset(i + 1));
                let (y0, y1) = (top.max(area.y), next.min(area.bottom()));
                Rect::new(area.x, y0, area.w, (y1 - y0).max(0.0))
            }
            _ => area,
        };
        let fading = motion.as_ref().map_or(&[][..], |m| m.fading(i));
        let fade = motion.as_ref().map_or(1.0, |m| m.clock.progress());
        let fixed = |r: &Rect| Rect::new(left + r.x, top + r.y, r.w, r.h);
        // 刚显示原文的块：渲染结果另有偏移，原文裁剪到渲染结果的上边缘。
        let inner = motion.as_ref().and_then(|m| m.inner(i));
        let reach = inner.map_or(0.0, |(_, d)| d.abs());
        block.visible(
            area.y - top - reach,
            area.bottom() - top + reach,
            &mut shown,
        );
        if let Some(leaving) = motion.as_ref().and_then(|m| m.leaving(i)) {
            let at = origin_y + laid.tops[i] + leaving.offset * (1.0 - fade);
            let clip = Rect::new(whole.x, whole.y, whole.w, (top - whole.y).max(0.0));
            let alpha = 1.0 - motion.as_ref().map_or(1.0, |m| m.clock.linear());
            let source = &leaving.old.items[..leaving.items as usize];
            frame.with_opacity(alpha, |frame| draw_plain(frame, source, (left, at), clip));
        }
        frame.with_fade(p.editor_bg, opacity, |frame| {
            for (k, item) in shown
                .iter()
                .map(|&k| (k as usize, &block.items[k as usize]))
            {
                let (top, area) = match inner {
                    Some((first, d)) if k >= first => (top + d, area),
                    Some((_, d)) => {
                        let edge =
                            (top + block.preview.map_or(0.0, |p| p.1) + d).min(area.bottom());
                        (
                            top,
                            Rect::new(area.x, area.y, area.w, (edge - area.y).max(0.0)),
                        )
                    }
                    None => (top, area),
                };
                // 可横向滚动的表格中的项：按滚动位置左移，裁剪到正文一栏内。
                let (off, strip) = laid.shift(i, k);
                let area = strip.map_or(area, |s| area.intersect(fixed(&s)));
                let left = left - off;
                let at = |r: &Rect| Rect::new(left + r.x, top + r.y, r.w, r.h);
                match item {
                    Item::Text {
                        x,
                        baseline,
                        text,
                        font,
                        faux,
                        size,
                        color,
                        span,
                    } => {
                        if let Some(span) = span {
                            let from = laid.starts[i] + span.start;
                            let to = from + span.advances.len();
                            let mut mark = |a: usize, b: usize, color: Color| {
                                let (a, b) = (a.max(from), b.min(to));
                                if a >= b {
                                    return;
                                }
                                let (a, b) = (a - from, b - from);
                                let x0: f32 = span.advances[..a].iter().sum();
                                let x1: f32 = x0 + span.advances[a..b].iter().sum::<f32>();
                                let (y0, h) = line_box(top + baseline, *size);
                                let r = Rect::new(left + x + x0, y0, x1 - x0, h);
                                frame.rect_clipped(r, color, 2.0, area);
                            };
                            if let Some((search, current)) = matches {
                                for &(a, b) in search.matches_in(from, to) {
                                    let color = if current == Some((a, b)) {
                                        p.search_current
                                    } else {
                                        p.search_match
                                    };
                                    mark(a, b, color);
                                }
                            }
                            if let Some((a, b)) = selection {
                                mark(a, b, p.selection);
                            }
                            let start = laid.ranges.get(i).map_or(0, |r| r.start);
                            let local = |s: usize| span.chars_before(text, s.saturating_sub(start));
                            if let Some((rope, current)) = live_matches
                                && let Some(first) = span.source_of(text, 0)
                                && let Some(last) = span.source_of(text, span.advances.len())
                            {
                                let chars = |b: usize| rope.byte_to_char(b.min(rope.len_bytes()));
                                let byte = |c: usize| rope.char_to_byte(c.min(rope.len_chars()));
                                let (c0, c1) = (chars(start + first), chars(start + last));
                                for &(a, b) in search.matches_in(c0, c1.max(c0 + 1)) {
                                    let color = if current == Some((a, b)) {
                                        p.search_current
                                    } else {
                                        p.search_match
                                    };
                                    mark(from + local(byte(a)), from + local(byte(b)), color);
                                }
                            }
                            if let Some((a, b)) = live_selection
                                && span.source.is_some()
                            {
                                mark(from + local(a), from + local(b), p.selection);
                            }
                        }
                        frame.text_faux(
                            *font,
                            *faux,
                            *size,
                            left + x,
                            top + baseline,
                            text,
                            if fading.contains(&k) {
                                color.alpha(color.a * fade)
                            } else {
                                *color
                            },
                            area,
                        );
                    }
                    Item::Fill {
                        rect,
                        color,
                        radius,
                    } => frame.rect_clipped(at(rect), *color, *radius, area),
                    Item::Corners { rect, color, radii } => {
                        frame.rect_corners(at(rect), *color, *radii, area);
                    }
                    Item::Image {
                        rect,
                        path,
                        at: spot,
                    } => {
                        let r = at(rect);
                        // 可编辑的预览中，原文可以改写的图片可调整大小。
                        if spot.is_some() && resizable {
                            let image = ImageAt {
                                rect: r,
                                block: i,
                                item: k,
                            };
                            app.hits.markdown_images.push(image);
                        }
                        let scale = frame.scale();
                        let size = ((r.w * scale).round() as u32, (r.h * scale).round() as u32);
                        let drawn = match md.images.get(path) {
                            Some(ImageSlot::Ready { pixels, .. }) => {
                                pixels.iter().find(|px| (px.width, px.height) == size)
                            }
                            _ => None,
                        };
                        match drawn {
                            Some(px) => {
                                frame.pixels((r.x, r.y), px.width, px.height, &px.rgba, area)
                            }
                            None => wanted.push((path.clone(), size)),
                        }
                        images.push(path.clone());
                        // 本地图片可从右键菜单在新标签中打开。
                        if md.urls.get(path).is_none_or(|u| !u.contains("://")) {
                            app.hits.markdown_pictures.push((r, path.clone()));
                        }
                    }
                    Item::Link { rect, url } => {
                        let r = at(rect).intersect(area);
                        if r.w > 0.0 && r.h > 0.0 {
                            app.hits.markdown_links.push((r, url.to_string()));
                        }
                    }
                    Item::Check {
                        rect,
                        checked,
                        source,
                    } => {
                        let r = at(rect);
                        draw_check(frame, p, r, *checked, area);
                        // 可点击范围比框向外扩展一圈，较小的框也易于点中。
                        let hit = r.inset(-3.0, -3.0).intersect(area);
                        if hit.w > 0.0 && hit.h > 0.0 {
                            let start = laid.ranges.get(i).map_or(0, |r| r.start);
                            let at = start.saturating_add(*source as usize);
                            app.hits.markdown_links.push((hit, format!("{TASK}{at}")));
                        }
                    }
                    Item::Path {
                        strokes,
                        width,
                        color,
                        ..
                    } => {
                        let moved: Vec<Stroke> = strokes
                            .iter()
                            .map(|&s| winc_typeset::shift(s, left, top))
                            .collect();
                        frame.stroke(&moved, *width, *color, area);
                    }
                    Item::Polygon { points, color, .. } => {
                        let moved: Vec<(f32, f32)> =
                            points.iter().map(|&(x, y)| (x + left, y + top)).collect();
                        frame.polygon(&moved, *color, area);
                    }
                    // 所有块绘制完成后再绘制，使其覆盖在上一块的文字之上。
                    // 工具栏贴着正文一栏的左边，不随表格横向滚动。
                    Item::Table {
                        rect,
                        align: Some(align),
                        ..
                    } => toolbar = Some((fixed(rect), *align)),
                    Item::Table { .. } | Item::Spot { .. } => {}
                    Item::Code {
                        rect,
                        text,
                        diagram,
                    } => {
                        let r = at(rect);
                        app.hits.markdown_codes.push(r);
                        if r.contains(mouse.0, mouse.1) && area.contains(mouse.0, mouse.1) {
                            let button = copy_button(r);
                            code_button(frame, p, button, area, icons::copy);
                            app.hits.markdown_links.push((button, COPY.to_owned()));
                            app.hits.markdown_copy = Some(text.clone());
                            // 图的复制按钮左侧另有全屏查看按钮。
                            if *diagram {
                                let expand = Rect {
                                    x: button.x - button.w - 4.0,
                                    ..button
                                };
                                code_button(frame, p, expand, area, icons::expand);
                                app.hits.markdown_links.push((expand, EXPAND.to_owned()));
                                app.hits.markdown_expand = Some(text.clone());
                            }
                        }
                    }
                }
            }
        });
        for (s, scroller) in block.scrolls.iter().enumerate() {
            bars.push((i, s, fixed(&scroller.rect), laid.hscroll_of(i, s)));
        }
    }
    // 显示滚动条的表格：鼠标所在的表格，其次是最近一次横向滚动的表格。
    let mut chosen = None;
    for &(i, s, strip, off) in &bars {
        let scroller = &laid.blocks[i].1.scrolls[s];
        draw_table_fades(frame, p, scroller, strip, off, area);
        let bar = TableBar {
            key: (i, s),
            track: strip.intersect(area),
            content: scroller.content,
        };
        if strip.contains(mouse.0, mouse.1) {
            chosen = Some((bar, off, true));
        } else if chosen.is_none() && md.last_table == Some((i, s)) {
            chosen = Some((bar, off, false));
        }
    }
    let small = app.fonts.ui_small;
    resize::draw_image_resize(md, &app.hits.markdown_images, p, small, frame, area);
    // 跳转到的脚注或脚注引用铺一层强调色的底色，慢慢淡出（同标题列表点过的标题）。
    if let Some(((y, h), clock)) = md.anchor_flash {
        let alpha = 0.14 * (1.0 - clock.linear());
        let band = Rect::new(left, origin_y + y, key.width, h);
        frame.rect_clipped(
            band,
            p.accent.alpha(p.accent.a * alpha),
            TOC_FLASH_RADIUS,
            area,
        );
    }
    if let Some((table, align)) = toolbar {
        table_toolbar(
            frame,
            p,
            table,
            align,
            area,
            mouse,
            &mut app.hits.markdown_links,
        );
    }
    // 竖条在内容左边的留白正中，宽 2 逻辑像素；画在标题的折叠箭头之后（见 `draw_heading_folds`）。
    let caret_bar =
        caret.map(|(_, y, h)| Rect::new(left - pad / 2.0 - 1.0, area.y - scroll + y, 2.0, h));
    // 光标的绘制位置：找不到时（空文档、光标在第一块之前）位于内容左上角。
    // 滑动过程中随所在的块移动。
    let live_point = live_caret.map(|source| {
        let (x, y, h) =
            laid.caret_point(source)
                .unwrap_or((0.0, pad, (key.base * 0.85 * 1.4).round()));
        let block = laid
            .ranges
            .iter()
            .rposition(|r| r.start <= source && source <= r.end);
        let shift = motion.as_ref().zip(block).map_or(0.0, |(m, b)| {
            // 光标位于刚显示原文的块的渲染结果中时，加上渲染结果的偏移。
            let below = laid.blocks[b]
                .1
                .preview
                .map(|(_, top)| top + laid.tops[b] + pad);
            let inner = m
                .inner(b)
                .filter(|_| below.is_some_and(|top| y + h / 2.0 >= top));
            m.offset(b) + inner.map_or(0.0, |(_, d)| d)
        });
        let point = (x, y + shift, h);
        // 正文中的组字文字已排入所在的行，不再在其上重复绘制。
        let inline = laid.blocks.iter().any(|(_, b)| b.preedit);
        // 段落之后的空行上组字使用正文的字体，不沿用上方标题的字体。
        let body = (winc_render::FontKind::Prose, Default::default(), key.base);
        let font = match blank {
            true => Some(body),
            false => font_at(&laid, source),
        };
        (point, (!inline).then_some(font))
    });
    md.layout = Some(laid);
    md.motion = motion;
    // 与源代码的光标相同，只在编辑区持有焦点时绘制；焦点在终端、查找条等处时不显示，
    // 也不占用输入法候选窗的位置。
    if let Some(((x, y, h), font)) = live_point.filter(|_| app.focus == Focus::Editor) {
        let (x, y) = (left + x, area.y - scroll + y);
        app.ime.cursor = Some(draw_live_caret(app, frame, (x, y, h), font, area));
    }
    let missing: Vec<_> = wanted.iter().map(|(path, _)| path.clone()).collect();
    for (path, size) in wanted {
        app.request_markdown_pixels(&path, size);
    }
    app.release_markdown_images(&images, &missing);
    draw_table_scrollbar(app, frame, chosen, area);
    let (arrows, covered) = draw_heading_folds(app, frame, area, caret_bar);
    if let Some(bar) = caret_bar.filter(|_| !covered) {
        frame.rect_clipped(bar, p.accent.alpha(0.7), 1.0, area);
    }
    // 分栏时在预览的文字中同一位置画出编辑区的光标，与编辑区的光标一同闪烁。
    if let Some((x, y, h)) = mirrored.filter(|_| app.blink.visible) {
        let bar = Rect::new(
            (left + x).round() - 1.0,
            area.y - scroll + y + 1.0,
            2.0,
            h - 2.0,
        );
        frame.rect_clipped(bar, p.caret, 1.0, area);
    }
    draw_source_numbers(app, frame, area, &arrows);
    draw_broken_links(app, frame, area);
    toolbar::draw_selection_toolbar(app, frame, area);
    draw_toc(app, frame, area, pad);

    // 和编辑区的滚动条一样：平时隐藏，滚动、悬停时显示，可以拖动。
    let owner = app.editor.active().map_or(0, |doc| doc.id.0);
    let content = app.preview.markdown.height;
    crate::view::draw_scrollbar(
        app,
        frame,
        ScrollArea::Markdown,
        owner,
        area,
        scroll,
        content,
    );
}

/// 表格被裁掉的一侧渐隐的宽度（逻辑像素），与标签栏两端的渐隐（`TAB_FADE`）相同。
const TABLE_FADE: f32 = 24.0;

/// 原文行号的右边离正文左边的距离（逻辑像素）：与编辑区行号栏和文字之间的间距相近。
const NUMBER_GAP: f32 = 12.0;

/// 在可编辑预览的左侧留白里写原文行号（见 `laid::source_lines`），光标所在的行颜色深一些。
///
/// 关闭行号设置时、只读的预览中不写；留白放不下最大的行号时也不写。
///
/// `skip` 是画了折叠箭头的行，不再写行号。
fn draw_source_numbers(app: &App, frame: &mut Frame<'_>, area: Rect, skip: &[usize]) {
    if !app.markdown_line_numbers_on() || !app.markdown_live() {
        return;
    }
    let (Some(laid), Some(doc)) = (app.preview.markdown.layout.as_ref(), app.editor.active())
    else {
        return;
    };
    let Some(rope) = doc.text().rope() else {
        return;
    };
    let p = app.palette;
    let size = app.fonts.code;
    let cell = frame.measure(FontKind::Mono, size, "0");
    let right = laid.left - NUMBER_GAP;
    if laid.left - number_room(app, frame) < area.x + 4.0 {
        return;
    }
    let lit = selected_rows(laid, rope, &doc.selection);
    let top = drawn_scroll(app.preview.markdown.scroll()) - laid.pad;
    let lines = crate::app::preview::markdown::laid::source_lines(laid, rope, top..top + area.h);
    for (line, y) in lines {
        if skip.contains(&line) {
            continue;
        }
        let number = (line + 1).to_string();
        let x = right - number.len() as f32 * cell;
        // 行号只作定位参考，比正文的次要色更淡；光标或选区所在显示行的行号用次要色。
        let color = match lit.contains(line, y) {
            true => p.text_muted,
            false => p.text_dim,
        };
        let baseline = area.y + y - top;
        frame.text(FontKind::Mono, size, x, baseline, &number, color, area);
    }
}

/// 在标题左边画收起、展开下面一节的箭头：鼠标在标题上时显示，收起的标题一直显示，朝右；
/// 点击的范围记成链接（[`FOLD`]）。
///
/// 有行号时箭头画在这一行的行号上；没有行号时画在留白正中，与分栏时标出光标所在行的竖条
/// `bar` 同一列，竖条落在画了箭头的标题上时箭头改用强调色、不再画竖条。返回画了箭头的行，
/// 以及竖条是否被箭头代替。
fn draw_heading_folds(
    app: &mut App,
    frame: &mut Frame<'_>,
    area: Rect,
    bar: Option<Rect>,
) -> (Vec<usize>, bool) {
    let (mut lines, mut covered) = (Vec::new(), false);
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return (lines, covered);
    };
    let folds = (app.editor.active()).and_then(|d| app.preview.markdown.folds.get(&d.id));
    let scroll = drawn_scroll(app.preview.markdown.scroll());
    let (mx, my) = app.mouse();
    let p = app.palette;
    let numbers = app.markdown_line_numbers_on() && app.markdown_live();
    let x = match numbers {
        true => laid.left - NUMBER_GAP - ARROW / 2.0,
        false => laid.left - laid.pad / 2.0,
    };
    let view = scroll - laid.pad;
    for i in laid.blocks_in(view, view + area.h) {
        let Some(section) = laid.sections.get(i).and_then(Option::as_ref) else {
            continue;
        };
        let top = area.y - scroll + laid.pad + laid.tops[i];
        let height = laid.blocks[i].1.height;
        let folded = folds.is_some_and(|f| f.contains(&section.key));
        let over = area.contains(mx, my) && (top..top + height).contains(&my);
        if !(folded || over) {
            continue;
        }
        let start = laid.ranges[i].start;
        let cy = laid
            .caret_point(start)
            .map_or(top + height / 2.0, |(_, y, h)| {
                area.y - scroll + y + h / 2.0
            });
        let hovered = app.pointer.hover == Hover::MarkdownLink(app.hits.markdown_links.len());
        let marked = bar.is_some_and(|b| (b.y..b.bottom()).contains(&cy));
        covered |= marked;
        let color = match (marked, hovered) {
            (true, _) => p.accent,
            (false, true) => p.text,
            (false, false) => p.text_dim,
        };
        let turned = if folded {
            0.0
        } else {
            std::f32::consts::FRAC_PI_2
        };
        icons::chevron_turned(frame, (x, cy), ARROW, turned, color, area);
        let hit = Rect::new(x - ARROW - 2.0, cy - 10.0, 2.0 * ARROW + 4.0, 20.0);
        let url = format!("{FOLD}{}", section.key);
        app.hits.markdown_links.push((hit, url));
        if let Some(&line) = laid.lines.get(i) {
            lines.push(line);
        }
    }
    (lines, covered)
}

/// 返回第 `i` 块的上沿到下一块上沿的距离（逻辑像素）。
fn room_below(laid: &Laid, i: usize) -> f32 {
    let top = laid.tops[i];
    laid.tops
        .get(i + 1)
        .map_or(laid.blocks[i].1.height, |next| next - top)
}

/// mermaid 图的高度上限占预览区高度的比例。
const TALL_SHARE: f32 = 0.66;
/// mermaid 图高度上限的最小值（逻辑像素）。
const TALL_MIN: f32 = 320.0;

/// 标题折叠箭头的大小（逻辑像素）。
const ARROW: f32 = 8.0;

/// 光标或选区占据的范围，决定哪些行号显示为当前行（见 [`draw_source_numbers`]）。
enum Lit {
    /// 显示行的上下范围（相对内容顶部，与 `laid::source_lines` 的基线同一量法）。
    Rows(f32, f32),
    /// 光标或选区的一端落在没有显示文字的位置（空行、分隔线）：按原文行号的范围判断。
    Lines(usize, usize),
}

impl Lit {
    /// 写在基线 `y` 处的原文第 `line` 行的行号是否在光标或选区所在的显示行上。
    fn contains(&self, line: usize, y: f32) -> bool {
        match *self {
            Lit::Rows(top, bottom) => (top..=bottom).contains(&y),
            Lit::Lines(first, last) => (first..=last).contains(&line),
        }
    }
}

/// 选区 `selection` 在排版结果 `laid` 中占据的显示行；一行原文可能折成几个显示行，几行原文也
/// 可能排在同一显示行，因此按显示行判断，不按原文行号判断。
fn selected_rows(
    laid: &crate::app::preview::markdown::laid::Laid,
    rope: &ropey::Rope,
    selection: &winc_core::Selection,
) -> Lit {
    use crate::app::preview::markdown::laid::point_of;
    let (from, to) = (
        selection.anchor.min(selection.head),
        selection.anchor.max(selection.head),
    );
    let byte = |c: usize| rope.char_to_byte(c.min(rope.len_chars()));
    match (point_of(laid, byte(from)), point_of(laid, byte(to))) {
        (Some((_, a, _)), Some((_, b, h))) => Lit::Rows(a - laid.pad, b + h - laid.pad),
        _ => {
            let line = |c: usize| rope.char_to_line(c.min(rope.len_chars()));
            Lit::Lines(line(from), line(to))
        }
    }
}

/// 原文行号在正文左侧占用的宽度（逻辑像素）：最大行号的宽度加上与正文的间距；不写行号时为 0。
fn number_room(app: &App, frame: &mut Frame<'_>) -> f32 {
    if !app.markdown_line_numbers_on() || !app.markdown_live() {
        return 0.0;
    }
    let Some(rope) = app.editor.active().and_then(|d| d.text().rope()) else {
        return 0.0;
    };
    let cell = frame.measure(FontKind::Mono, app.fonts.code, "0");
    rope.len_lines().to_string().len() as f32 * cell + NUMBER_GAP
}

/// 标题指示条左边离预览区域左边至少的距离（逻辑像素）。
const TOC_X: f32 = 10.0;
/// 标题指示条右端（当前所在一节那一条的右端）离正文左边的距离（逻辑像素）：指示条靠着正文，
/// 随正文居中的位置移动。
const TOC_GAP: f32 = 36.0;
/// 指示条中一条的长度（逻辑像素）；当前所在的一节为 [`TOC_CURRENT`]。
const TOC_DASH: f32 = 14.0;
/// 指示条中当前所在一节那一条的长度（逻辑像素）：比其他各条长，一眼看出读到哪里。
const TOC_CURRENT: f32 = 18.0;
/// 画标题指示条所需的最小左侧留白（逻辑像素）：指示条右边与正文之间至少留 8。
const TOC_ROOM: f32 = TOC_X + TOC_CURRENT + 8.0;
/// 指示条中相邻两条的间距（逻辑像素）；标题过多时缩小，见 [`draw_toc`]。
const TOC_STEP: f32 = 8.0;
/// 指示条中一条的粗细（逻辑像素）；当前所在的一节再粗 1。
const TOC_BAR: f32 = 2.0;
/// 标题列表一行的高度（逻辑像素）：比文件树的行（`TREE_ROW`）紧凑，标题多时一屏列得下更多。
const TOC_ROW: f32 = 20.0;
/// 点过的标题在正文里铺的底色的圆角半径（逻辑像素）：底色与正文同宽，圆角取小，比列表行的
/// 圆角（`RADIUS_ROW`）小，整行看起来不像一个按钮。
const TOC_FLASH_RADIUS: f32 = 3.0;
/// 标题列表的内边距（逻辑像素）：行的底色离卡片边缘的距离。
const TOC_PAD: f32 = 4.0;
/// 标题列表里每深一级标题，文字缩进的宽度（逻辑像素）。
const TOC_INDENT: f32 = 12.0;
/// 标题列表右上角图钉按钮的宽度（逻辑像素）：与一行同高的方块，第一行的标题让出这一段。
const TOC_PIN: f32 = TOC_ROW;
/// 钉住的标题列表最窄的宽度（逻辑像素）：放得下横线、图钉和约六个汉字；左侧留白比它还窄时
/// 收成指示条。
const TOC_PINNED_MIN: f32 = 140.0;

/// 展开的标题列表的最小和最大宽度（逻辑像素）；超出最大宽度的标题文字被省略。
const TOC_WIDTHS: (f32, f32) = (180.0, 320.0);
/// 正文为钉住的标题列表让出的列宽（逻辑像素），等于列表的最小宽度。取固定值，
/// 使正文的位置不随标题长短变化；留白更宽时列表可以更宽，至多为 [`TOC_WIDTHS`] 的上限。
const TOC_PINNED_COLUMN: f32 = TOC_WIDTHS.0;
/// 钉住的标题列表两侧间距之和（逻辑像素）：左侧距预览区域 4，右侧距正文或行号 8，与
/// [`draw_toc`] 的布局一致。
const TOC_PINNED_EDGE: f32 = 12.0;
/// 让出一列后正文的最小宽度（逻辑像素），等于区域宽 600 逻辑像素、使用窄留白时的正文宽度；
/// 不足时不让出，列表显示为指示条。
const TOC_PINNED_BODY: f32 = 600.0 - 2.0 * PAD_NARROW;

/// 文档是否有标题列表：至少两个文字不为空的标题，与 [`draw_toc`] 的条件相同。
fn has_outline(parsed: &winc_preview::markdown::Markdown) -> bool {
    let titled = |block: &winc_preview::markdown::Block| match block {
        winc_preview::markdown::Block::Heading(_, inline) => {
            !heading_title(&inline.text).is_empty()
        }
        _ => false,
    };
    parsed
        .blocks
        .iter()
        .filter(|block| titled(block))
        .take(2)
        .count()
        == 2
}

/// 标题指示条和列表里一条横线的颜色与粗细：已经读过的各节用次要色，当前所在的一节用强调色并
/// 加粗，还没读到的用更淡的颜色，像阅读进度。
fn toc_dash(p: &Palette, i: usize, current: Option<usize>) -> (Color, f32) {
    match current {
        Some(c) if i == c => (p.accent, TOC_BAR + 1.0),
        Some(c) if i < c => (p.text_muted, TOC_BAR),
        _ => (p.text_dim, TOC_BAR),
    }
}

/// 在预览左侧的留白中绘制标题指示条：每个标题一条横线，颜色表示阅读进度（见 [`toc_dash`]）。
///
/// 鼠标停留满 `TOC_HOVER_DELAY` 后标题列表在指示条处展开，点击一项滚动到该标题；
/// 图钉使列表在正文左侧保持展开，放不下时仍显示为指示条（见 [`draw`]）。标题少于两个时不绘制。
/// 可点击的区域写入 `app.hits.markdown_toc`。
fn draw_toc(app: &mut App, frame: &mut Frame<'_>, area: Rect, pad: f32) {
    app.preview.markdown.toc_current = None;
    let (Some(parsed), Some(laid)) = (
        app.preview.markdown.parsed.clone(),
        app.preview.markdown.layout.as_ref(),
    ) else {
        return;
    };
    // 可编辑的预览在正文左侧显示原文行号，指示条位于行号左侧。
    let numbers = number_room(app, frame);
    if laid.left - numbers - area.x < TOC_ROOM {
        return;
    }
    // 各标题：（级别，文字，所在顶层块的上边缘，块高）。
    let headings: Vec<(u8, String, f32, f32, usize)> = parsed
        .blocks
        .iter()
        .zip(laid.tops.iter().zip(&laid.blocks))
        .enumerate()
        .filter_map(|(i, (block, (&top, (_, laid))))| match block {
            winc_preview::markdown::Block::Heading(level, inline) => {
                Some((*level, heading_title(&inline.text), top, laid.height, i))
            }
            _ => None,
        })
        .filter(|(_, title, ..)| !title.is_empty())
        .collect();
    if headings.len() < 2 {
        return;
    }
    let p = app.palette;
    let scroll = drawn_scroll(app.preview.markdown.scroll());
    let max = (app.preview.markdown.height - area.h).max(0.0);
    // 当前所在的一节：顶边已经滚到预览顶上（或以上）的最后一个标题。滚到底时，文末几个标题
    // 滚不到顶上，取屏幕里看得到的最后一个；在标题列表里点过、之后没有再滚动时，取点的那一个。
    let picked = app
        .preview
        .markdown
        .toc_picked
        .filter(|&(i, at)| i < headings.len() && (at - scroll).abs() < 0.5)
        .map(|(i, _)| i);
    if picked.is_none() {
        app.preview.markdown.toc_picked = None;
    }
    let current = picked.or_else(|| {
        if max > 0.0 && scroll >= max - 0.5 {
            headings
                .iter()
                .rposition(|h| h.2 < scroll + area.h - pad * 2.0)
        } else {
            headings.iter().rposition(|h| h.2 <= scroll + 1.0)
        }
    });
    let top_level = headings.iter().map(|h| h.0).min().unwrap_or(1);
    let depth = |level: u8| f32::from(level.saturating_sub(top_level).min(3));

    // 标题很多时缩小间距，放不下的部分不画。
    let room = (area.h - pad * 2.0).max(0.0);
    let step = (room / headings.len() as f32).clamp(TOC_BAR + 2.0, TOC_STEP);
    let shown = ((room / step).floor() as usize).min(headings.len());
    let x0 = (laid.left - numbers - TOC_GAP - TOC_CURRENT).max(area.x + TOC_X);
    let y0 = area.y + pad;
    let left = laid.left;
    let width = laid.key.width;

    // 点过的标题在正文里铺一层强调色的底色，慢慢淡出。底色与正文同宽，左右与表格、代码块的
    // 边缘对齐。
    if let Some((i, clock)) = app.preview.markdown.toc_flash
        && let Some(&(_, _, top, height, _)) = headings.get(i)
    {
        let alpha = 0.14 * (1.0 - clock.linear());
        let band = Rect::new(left, y0 - scroll + top, width, height);
        frame.rect_clipped(
            band,
            p.accent.alpha(p.accent.a * alpha),
            TOC_FLASH_RADIUS,
            area,
        );
    }
    let size = app.fonts.ui;
    // 行里的横线离卡片左边的距离；鼠标停留展开时与指示条在同一列。
    let inner = TOC_PAD + 8.0;
    // 停留满 `TOC_HOVER_DELAY` 后展开；鼠标移过指示条时不展开。
    let rested = (app.preview.markdown.toc_rest)
        .is_some_and(|since| since.elapsed() >= crate::theme::TOC_HOVER_DELAY);
    let on_bar = app.scrollbar_active(crate::layout::Scrollbar::MarkdownToc);
    let hovered = rested && (on_bar || matches!(app.pointer.hover, Hover::MarkdownToc(_)));
    let pinned = app.state.markdown_toc_pinned;
    // 收起过渡中的列表仍然绘制，展开程度从 1 降到 0。
    let closing = (app.preview.markdown.toc_close).map(|clock| 1.0 - clock.progress());
    let widest = (hovered || pinned || closing.is_some()).then(|| {
        (headings.iter())
            .map(|(level, title, ..)| {
                depth(*level) * TOC_INDENT + frame.measure(FontKind::Ui, size, title)
            })
            .fold(0.0_f32, f32::max)
    });
    let natural = widest
        .map(|w| (w + inner + TOC_DASH + 10.0 + 8.0 + TOC_PAD).clamp(TOC_WIDTHS.0, TOC_WIDTHS.1));
    // 钉住的列表靠着正文放在左侧留白里，右边与正文之间留 8，留白不够时变窄（标题省略）；窄于
    // [`TOC_PINNED_MIN`] 时收成指示条，不遮住正文。鼠标停留展开时卡片在指示条处展开、遮住指示条，
    // 不超出预览的左边。可编辑的预览在正文左侧写原文行号：钉住的列表放在行号左边，不遮住行号。
    let edge = left - numbers - 8.0;
    let room = (edge - (area.x + 4.0)).max(0.0);
    // 正文过渡期间按过渡结束后的位置判断能否容纳列表；取消钉住时按过渡开始前的位置。
    let fits_at = |left: f32| left - numbers - 8.0 - (area.x + 4.0) >= TOC_PINNED_MIN;
    let fits = fits_at(app.preview.markdown.toc_left);
    let fitted = app.preview.markdown.toc_slide.is_some_and(fits_at);
    let stays = pinned && fits;
    // 钉住的程度：0 是鼠标停留展开的样子（卡片盖在指示条上），1 是钉住常驻的样子（靠着正文、
    // 没有卡片的底色和边框）。钉住、取消钉住时在两者之间过渡。
    let pinning = match app.preview.markdown.toc_pin {
        Some((to, clock)) if fits || (!to && fitted) => {
            let t = clock.progress();
            if to { t } else { 1.0 - t }
        }
        _ => f32::from(u8::from(stays)),
    };
    let hover_x = (x0 - inner).max(area.x + 4.0);
    let x = hover_x + (edge - natural.unwrap_or(0.0).min(room) - hover_x) * pinning;
    let dash_x = (x0 - hover_x) + (inner - (x0 - hover_x)) * pinning;
    let text_x = dash_x + TOC_DASH + 10.0;
    let card_w = natural.map(|w| {
        let floating = w.min(area.right() - 16.0 - hover_x);
        floating + (w.min(room) - floating) * pinning
    });
    // 列表展开着时，指示条的各条由列表里的横线接替（从原位滑到各行），这里不再画。
    let open = hovered || stays || closing.is_some();
    if !open {
        app.preview.markdown.toc_top = None;
        app.preview.markdown.toc_list = Default::default();
    }
    let bar_y = |i: usize| y0 + i as f32 * step + TOC_BAR / 2.0;
    for i in (0..shown).filter(|_| !open) {
        let (color, thick) = toc_dash(p, i, current);
        let len = if current == Some(i) {
            TOC_CURRENT
        } else {
            TOC_DASH
        };
        let y = y0 + i as f32 * step + (TOC_BAR - thick) / 2.0;
        frame.rect_clipped(Rect::new(x0, y, len, thick), color, thick / 2.0, area);
    }
    let bars_h = (shown.max(1) - 1) as f32 * step + TOC_BAR;
    let bars = Rect::new(x0 - 8.0, y0 - 8.0, TOC_CURRENT + 16.0, bars_h + 16.0);

    let mut hits = Vec::new();
    if let Some(w) = card_w.filter(|_| open) {
        let fit = ((area.h - 16.0 - TOC_PAD * 2.0) / TOC_ROW).floor().max(1.0) as usize;
        let rows = headings.len().min(fit);
        // 标题超过可显示的行数时，列出当前所在一节附近的一段，可按行滚动。
        let natural = current
            .unwrap_or(0)
            .saturating_sub(rows / 2)
            .min(headings.len() - rows);
        let list = &mut app.preview.markdown.toc_list;
        let start = list.start(natural, current, (rows, headings.len()));
        let h = rows as f32 * TOC_ROW + TOC_PAD * 2.0;
        // 列表展开时，当前所在一节那一行的横线落在指示条里同一条的位置上，其余各行向上下展开，
        // 横线看起来没有移动；没有当前所在的一节时以第一行对齐第一条。超出预览时再挪进来。
        let anchor = current
            .filter(|&c| c < shown && (start..start + rows).contains(&c))
            .unwrap_or(start);
        let bar_mid = bar_y(anchor.min(shown.saturating_sub(1)));
        let row_mid = TOC_PAD + (anchor - start) as f32 * TOC_ROW + TOC_ROW / 2.0;
        let fit_in = |y: f32| y.min(area.bottom() - 8.0 - h).max(area.y + 8.0);
        // 悬停展开的列表位置在展开时确定，滚动预览时不变；钉住的列表首行对齐指示条的第一条。
        // 二者之间按钉住的程度过渡。
        let floating = app.preview.markdown.toc_top;
        let floating = floating.map_or(bar_mid - row_mid, |top| area.y + top);
        if !stays {
            app.preview.markdown.toc_top = Some(fit_in(floating) - area.y);
        }
        let docked = bar_y(0) - (TOC_PAD + TOC_ROW / 2.0);
        let y = fit_in(floating + (docked - floating) * pinning);
        let card = Rect::new(x, y, w.max(0.0), h);
        app.preview.markdown.toc_list.card = card;
        app.preview.markdown.toc_list.row = TOC_ROW;
        // 展开时卡片从指示条处展开并淡入，各行的横线从指示条中对应的位置移到所在的行；收起时相反。
        // 钉住的列表始终显示，不参与悬停展开。
        let progress = match (stays, hovered, closing) {
            (true, ..) => 1.0,
            (false, false, Some(left)) => left,
            _ => app.preview.markdown.toc_fade.map_or(1.0, |c| c.progress()),
        };
        // 过渡期间按展开程度裁剪卡片：从指示条所在的位置扩展到整张卡片。
        let grow = |from: f32, to: f32| from + (to - from) * progress;
        let seed = bars.intersect(card);
        let reveal = Rect::new(
            grow(seed.x, card.x),
            grow(seed.y, card.y),
            grow(seed.w, card.w),
            grow(seed.h, card.h),
        );
        // 收起中的列表不再响应鼠标。
        let live = hovered || stays;
        let mut dashes = Vec::with_capacity(rows);
        frame.with_opacity(progress, |frame| {
            // 钉住后列表与预览融为一体：卡片的底色、边框和阴影随钉住的程度淡出。
            frame.with_opacity(1.0 - pinning, |frame| {
                crate::view::draw_card(frame, p, reveal, crate::theme::RADIUS_POPUP, p.popup_bg);
            });
            for (n, i) in (start..start + rows).enumerate() {
                let (level, title, ..) = &headings[i];
                let band = Rect::new(
                    card.x + TOC_PAD,
                    card.y + TOC_PAD + n as f32 * TOC_ROW,
                    card.w - TOC_PAD * 2.0,
                    TOC_ROW,
                );
                if app.pointer.hover == Hover::MarkdownToc(TocTarget::Heading(i)) {
                    frame.rect_clipped(band, p.hover, crate::theme::RADIUS_ROW, reveal);
                }
                let row_mid = band.y + band.h / 2.0;
                let from = if i < shown { bar_y(i) } else { row_mid };
                dashes.push((i, from + (row_mid - from) * progress));
                let tx = card.x + text_x + depth(*level) * TOC_INDENT;
                // 第一行的右端是图钉按钮。
                let pin_room = if n == 0 { TOC_PIN } else { 0.0 };
                let room = (band.right() - 8.0 - pin_room - tx).max(0.0);
                let title = crate::view::ellipsize(frame, title, size, room);
                // 当前所在一节的标题加粗显示。
                let faux = winc_render::Faux {
                    bold: current == Some(i),
                    ..Default::default()
                };
                let baseline = crate::view::ui_baseline(app, band, size);
                frame.text_faux(
                    FontKind::Ui,
                    faux,
                    size,
                    tx,
                    baseline,
                    &title,
                    p.text,
                    reveal,
                );
                if live {
                    hits.push((band, TocTarget::Heading(i)));
                }
            }
            // 图钉按钮盖在第一行的右端：命中时取先记下的一项，按钮排在各行之前。
            let pin = Rect::new(
                card.right() - TOC_PAD - TOC_PIN,
                card.y + TOC_PAD,
                TOC_PIN,
                TOC_ROW,
            );
            let pin_hovered = app.pointer.hover == Hover::MarkdownToc(TocTarget::Pin);
            if pin_hovered {
                frame.rect_clipped(pin, p.hover, crate::theme::RADIUS_ROW, reveal);
            }
            let color = match (pinned, pin_hovered) {
                (true, _) => p.accent,
                (false, true) => p.text,
                (false, false) => p.text_dim,
            };
            let center = (pin.x + pin.w / 2.0, pin.y + pin.h / 2.0);
            crate::icons::pin(frame, center, color, reveal);
            if live {
                hits.insert(0, (pin, TocTarget::Pin));
            }
        });
        // 标题没有全部列出时带滚动条。
        let track = Rect::new(card.x, card.y + TOC_PAD, card.w, rows as f32 * TOC_ROW);
        app.preview.markdown.toc_list.track = track;
        if let Some((scroll, content)) = app.preview.markdown.toc_list.scrollbar().filter(|_| live)
        {
            let owner = app.editor.active().map_or(0, |doc| doc.id.0);
            let area = ScrollArea::MarkdownToc;
            crate::view::draw_scrollbar(app, frame, area, owner, track, scroll, content);
        }
        // 横线不随卡片淡入，始终不透明：展开的第一帧与指示条完全重合。
        for (i, mid) in dashes {
            let (color, thick) = toc_dash(p, i, current);
            let len = if current == Some(i) {
                TOC_CURRENT
            } else {
                TOC_DASH
            };
            let dash = Rect::new(card.x + dash_x, mid - thick / 2.0, len, thick);
            frame.rect_clipped(dash, color, thick / 2.0, area);
        }
        // 卡片的内边距也算在内：鼠标经过行与行之外的地方时列表不收起。
        if live {
            hits.push((card, TocTarget::Bars));
        }
    }
    hits.push((bars, TocTarget::Bars));
    app.hits.markdown_toc = hits;
    app.hits.markdown_toc_tops = headings.iter().map(|h| h.2).collect();
    app.hits.markdown_toc_blocks = headings.iter().map(|h| h.4).collect();
    app.preview.markdown.toc_current = current;
}

/// 标题列表里显示的标题文字：硬换行换成空格，去掉图片占位符和控制字符，首尾不留空白。
fn heading_title(text: &str) -> String {
    let text: String = text
        .chars()
        .filter(|&c| c != winc_preview::markdown::IMAGE_MARK)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 画可编辑预览中的文字光标（左边缘 `x`、所在那一行的上边缘 `y` 和高度 `h`，窗口坐标），
/// 或输入法正在组字的文字；返回输入法候选框应对齐的矩形。
///
/// 正文中的组字文字已排入所在的行（`font` 为 `None`），只返回候选框位置；代码块和表格中组字覆盖在
/// 光标处绘制，字体字号同光标处的文字（`Some(None)`：无文字时用等宽字体，字号见 [`line_box`]）。
fn draw_live_caret(
    app: &App,
    frame: &mut Frame<'_>,
    (x, y, h): Point,
    font: Option<Option<(winc_render::FontKind, winc_render::Faux, f32)>>,
    clip: Rect,
) -> Rect {
    let p = app.palette;
    let preedit = app.preedit_at(Focus::Editor);
    let Some(font) = font.filter(|_| !preedit.is_empty()) else {
        if !preedit.is_empty() {
            return Rect::new(x, y, 2.0, h);
        }
        if app.blink.visible {
            let bar = Rect::new(x.round() - 1.0, y + 1.0, 2.0, h - 2.0);
            frame.rect_clipped(bar, p.caret, 1.0, clip);
        }
        return Rect::new(x, y, 2.0, h);
    };
    let (font, faux, size) = font.unwrap_or((
        winc_render::FontKind::Mono,
        winc_render::Faux::default(),
        h / 1.4,
    ));
    // 光标的高度和基线均按 [`line_box`] 由字号计算。
    let baseline = y + size * 1.05;
    let width = frame.measure_faux(font, faux, size, preedit);
    frame.rect_clipped(Rect::new(x, y, width, h), p.editor_bg, 0.0, clip);
    frame.text_faux(font, faux, size, x, baseline, preedit, p.text, clip);
    frame.rect_clipped(
        Rect::new(x, baseline + 3.0, width, 1.0),
        p.accent,
        0.0,
        clip,
    );
    Rect::new(x + width, y, 2.0, h)
}

/// 绘制代码块或图右上角的一个按钮：不透明底色遮住下方内容，中间绘制图标 `icon`。
fn code_button(
    frame: &mut Frame<'_>,
    p: &Palette,
    button: Rect,
    clip: Rect,
    icon: fn(&mut Frame<'_>, (f32, f32), Color, Rect),
) {
    frame.rect_clipped(button, p.editor_bg, 6.0, clip);
    frame.rect_clipped(button, p.text.alpha(0.08), 6.0, clip);
    let center = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    icon(frame, center, p.text_muted, clip);
}

/// 返回代码块 `code`（屏幕坐标，逻辑像素）右上角的复制按钮。
///
/// 边长 28 逻辑像素，离代码块右边和上边各 6 逻辑像素（取自调参）。
pub(crate) fn copy_button(code: Rect) -> Rect {
    const SIZE: f32 = 28.0;
    Rect::new(code.right() - SIZE - 6.0, code.y + 6.0, SIZE, SIZE)
}

/// 任务列表的勾选框（逻辑像素）：选中时填强调色并画对勾，未选中时只画一个细边框。
fn draw_check(frame: &mut Frame<'_>, p: &Palette, r: Rect, checked: bool, clip: Rect) {
    if checked {
        frame.rect_clipped(r, p.accent, 3.0, clip);
        let (x, y, w, h) = (r.x, r.y, r.w, r.h);
        let mark = [
            Stroke::Line(x + w * 0.26, y + h * 0.52, x + w * 0.43, y + h * 0.69),
            Stroke::Line(x + w * 0.43, y + h * 0.69, x + w * 0.75, y + h * 0.33),
        ];
        frame.stroke(&mark, 1.5, p.text_on_accent, clip);
    } else {
        frame.rect_clipped(r, p.text_muted.alpha(0.6), 3.0, clip);
        frame.rect_clipped(r.inset(1.0, 1.0), p.editor_bg, 2.0, clip);
    }
}

#[cfg(test)]
pub(crate) mod tests;
