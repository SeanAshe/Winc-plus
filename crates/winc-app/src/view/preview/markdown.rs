//! Markdown 预览的排版和绘制，字号与间距都按逻辑像素算。
//!
//! 正文是编辑器字号的 16/13 倍，编辑器默认 13 时正文 16；行高 1.5 倍，段落之间 16 逻辑像素，
//! 标题为正文的 2/1.5/1.25/1/0.875/0.85 倍。排版在 `layout` 子模块里做，要量字宽，所以在绘制
//! 时进行，结果按顶层块缓存（[`Laid`]）：文字改了只重排变了的块，宽度、字号、配色或图片大小
//! 变了才整份重排。换行时不在「，。」这类标点前面、「（“」这类标点后面断。

use std::collections::HashMap;
use std::sync::Arc;

use winc_i18n::t;
use winc_render::{Color, FontKind, Frame, Rect, Stroke};

use crate::app::Hover;
use crate::app::preview::markdown::{ImageSlot, TableBar, TocTarget};
use crate::app::{App, ScrollArea};
use crate::icons;
use crate::theme::Palette;

mod layout;
mod viewer;

use layout::{Colors, Cx, Env, Item, LaidBlock, Out, Raw, Span, margins};
pub(crate) use layout::{DETAILS, Memo};
pub(crate) use viewer::DiagramLaid;
pub(in crate::view) use viewer::draw_diagram_view;

/// 正文字号与编辑器字号之比：编辑器字号 13 逻辑像素时，正文为 16 逻辑像素。
const BASE_RATIO: f32 = 16.0 / 13.0;
/// 正文最大宽度（逻辑像素，经验值）：更宽时居中、两侧留白。
const MAX_WIDTH: f32 = 880.0;
/// 区域不窄于 600 逻辑像素时文档四周的留白（逻辑像素，经验值）。
const PAD: f32 = 32.0;
/// 窄的时候（分栏，区域宽不足 600 逻辑像素）两边的留白（逻辑像素，经验值）。
const PAD_NARROW: f32 = 20.0;

/// 排版完成的文档。
pub(crate) struct Laid {
    key: Key,
    /// 排版依据的解析结果（`Arc` 的地址），换了说明文字改了。
    source: usize,
    /// 各顶层块：内容哈希和排版结果。
    blocks: Vec<(u64, Arc<LaidBlock>)>,
    /// 各顶层块的上边缘（相对内容顶部）。
    tops: Vec<f32>,
    /// 各顶层块的文字在整份文字（各块的 [`LaidBlock::text`] 连起来）里从第几个字符开始。
    starts: Vec<usize>,
    /// 整份文字的字符数。
    chars: usize,
    /// 各顶层块在原文里从第几行开始（见 `Markdown::lines`），分栏对齐滚动用。
    lines: Vec<usize>,
    /// 各顶层块在原文中的字节范围（见 `Markdown::ranges`），预览与原文互相定位用。
    ranges: Vec<std::ops::Range<usize>>,
    /// 排版时光标所在的叶子块，见 [`LiveKey`]。
    ///
    /// 不能编辑时为 `None`；光标换了叶子块（表格里换了一格）、或者打了字，这一块要重排。
    live: Option<LiveKey>,
    /// 上一次找的光标位置和结果（见 [`point_of`]）：滚动时光标不动，不必每帧在大块里再找。
    caret: std::cell::Cell<Option<(usize, Option<Point>)>>,
    /// 应画在上一显示行末尾的光标位置（原文字节偏移），见 [`Laid::set_upstream`]。
    upstream: std::cell::Cell<Option<usize>>,
    /// 内容总高度。
    height: f32,
    /// 上一次画的时候内容的左边缘（窗口坐标，逻辑像素），鼠标定位用。
    pub(crate) left: f32,
    /// 上一次画的时候内容四周的留白（逻辑像素），鼠标定位用。
    pad: f32,
    /// 比正文宽的表格的横向滚动位置（逻辑像素）：键为（顶层块序号，块中表格的序号，见
    /// [`LaidBlock::scrolls`]）。重排后按序号沿用。
    pub(crate) hscroll: HashMap<(usize, usize), f32>,
    /// 上一次横向滚动使其可见的光标位置（原文字节偏移），见 [`Laid::follow_caret`]。
    followed: Option<usize>,
}

/// 预览上的一个位置：左边缘（相对内容左边）、所在那一行铺底色的上边缘（和滚动位置同一个
/// 量法）和高度，均为逻辑像素；见 [`point_of`]。
type Point = (f32, f32, f32);

/// 决定排版结果的外部条件：任何一项变了都要整份重排。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    /// 正文宽度（逻辑像素，已减去两侧留白）。
    width: f32,
    /// 正文字号（逻辑像素）。
    base: f32,
    /// 正文颜色。
    text: Color,
    /// 底色，决定差异色、强调色混在白底还是黑底上。
    background: Color,
    /// 图片的版本号，有图片读好后就加一。
    images: u64,
}

/// 在 `area`（窗口坐标，逻辑像素）里画当前文档的 Markdown 预览，分栏时是右半边，只看预览时
/// 是整个编辑区。
///
/// 排版结果、可点击范围和光标位置写回 `app.markdown`；需要读取的图片记进请求，下一帧再画。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    app.markdown.area = area;
    app.markdown.links.clear();
    app.markdown.codes.clear();
    app.markdown.copy = None;
    app.markdown.expand = None;
    // 可编辑的预览在排版之前取得当前文字的解析结果，本帧即按最新的结构显示。
    app.parse_markdown_inline();
    // 第一次解析还没回来：先留空。
    let Some(parsed) = app.markdown.parsed.clone() else {
        return;
    };
    let pad = if area.w >= 600.0 { PAD } else { PAD_NARROW };
    let width = (area.w - pad * 2.0).clamp(1.0, MAX_WIDTH).floor();
    let left = (area.x + (area.w - width) / 2.0).round();
    let base = (app.fonts.code * BASE_RATIO).round();
    let key = Key {
        width,
        base,
        text: p.text,
        background: p.editor_bg,
        images: app.markdown.images_version,
    };
    let source = Arc::as_ptr(&parsed) as usize;
    let live = app.live_editing().then(|| app.live_leaf()).flatten();
    let live_key = live.as_ref().map(|l| live_key(l, &app.preedit));
    let loaded = std::mem::take(&mut app.markdown.loaded);
    let toggled = app.markdown.toggled_now.take();
    // 重排之前光标在旧排版里的高度：重排后据此保持光标在屏幕上不动（见 [`keep_caret_still`]）。
    let mut still = None;
    // 记录重排之前各块的绘制位置，重排后各块从该位置滑动到新位置（见 `BlockMotion`）。
    let mut before = None;
    let mut laid = match app.markdown.layout.take() {
        Some(laid)
            if laid.key == key
                && laid.source == source
                && laid.live == live_key
                && loaded.is_empty()
                && toggled.is_none() =>
        {
            laid
        }
        old => {
            let hscroll = old.as_ref().map(|l| l.hscroll.clone()).unwrap_or_default();
            // 文字变了：选区、查找用的文字作废。只是重排（宽度、图片变了）时选区留着。
            if old.as_ref().is_none_or(|l| l.source != source) {
                app.markdown.selection = None;
            }
            app.markdown.find_doc = None;
            still = old_caret(app, old.as_ref(), source, live_key.as_ref());
            let slide = std::mem::take(&mut app.markdown.slide);
            before = old
                .as_ref()
                .filter(|l| l.key == key && app.markdown_live())
                .map(|l| (Before::of(app, l), slide));
            // 有图片刚读好时，用到它的块大小变了，需要重排；折叠块开合时，它所在的块需要
            // 重排；其余块沿用上次的结果。
            let reuse = old.filter(|l| l.key == key).map(|l| {
                l.blocks
                    .into_iter()
                    .filter(|(_, block)| {
                        !block.uses_any(&loaded)
                            && toggled.as_deref().is_none_or(|url| !block.links_to(url))
                    })
                    .collect()
            });
            let mut laid = lay_out(
                app,
                frame,
                &parsed,
                key,
                source,
                live.as_ref(),
                reuse.unwrap_or_default(),
            );
            laid.hscroll = hscroll;
            laid
        }
    };
    laid.left = left;
    laid.pad = pad;
    laid.set_upstream(app.live_upstream());
    app.markdown.height = laid.height + pad * 2.0;
    // 可编辑的预览与编辑区相同：内容高于一屏时，最后一行下方可以滚出一段空白。
    if app.markdown_live() && app.markdown.height > area.h {
        app.markdown.height += area.h * crate::layout::OVERSCROLL;
    }
    keep_caret_still(app, still, &laid);
    app.markdown.layout = Some(laid);
    app.follow_live_caret();
    // 分栏时右边跟着左边滚；刚换成只看预览时对齐一次。
    if area != app.layout.editor {
        app.sync_markdown_scroll();
    } else {
        app.align_markdown_once();
    }
    app.scroll_to_pending_anchor();
    // `layout` 刚在上面放进去，取出时一定存在。
    let mut laid = app.markdown.layout.take().expect("刚放进去");

    let max = (app.markdown.height - area.h).max(0.0);
    let scroll = app.markdown.scroll().min(max);
    let origin_y = area.y + pad - scroll;
    if let Some((before, slide)) = before {
        let muted = Colors::new(p).muted;
        let (from, fading) = before.motion(&laid, scroll, muted, slide);
        app.start_markdown_motion(from, fading);
    }
    let motion = app.markdown.motion.take();
    // 拖着选文字时不出复制按钮。
    let mouse = if !app.dragging() {
        app.mouse()
    } else {
        (f32::NAN, f32::NAN)
    };
    // 分栏时标出编辑区光标所在的行，便于对照两侧的位置；键盘焦点在预览上时不标出。
    let caret = (area != app.layout.editor && !app.markdown.focused)
        .then(|| app.markdown_caret_source())
        .flatten()
        .and_then(|source| laid.caret_point(source));
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
    let md = &mut app.markdown;
    let selection = md
        .selection
        .map(|(a, b)| (a.min(b), a.max(b)))
        .filter(|(a, b)| a < b);
    // 查找条在查预览的文字时，标出匹配。
    let search = &app.search;
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
    let mut shown = Vec::new();
    let mut toolbar = None;
    // 可横向滚动的表格的滚动条，各块绘制完成后再绘制：（块，表格，显示区域，滚动位置）。
    let mut bars = Vec::new();
    let first = laid
        .tops
        .partition_point(|&t| origin_y + t <= area.y)
        .saturating_sub(1);
    for (i, (_, block)) in laid.blocks.iter().enumerate().skip(first) {
        let offset = |i: usize| motion.as_ref().map_or(0.0, |m| m.offset(i));
        let top = origin_y + laid.tops[i] + offset(i);
        if top > area.bottom() {
            break;
        }
        if top + block.height < area.y {
            continue;
        }
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
        block.visible(area.y - top, area.bottom() - top, &mut shown);
        for (k, item) in shown
            .iter()
            .map(|&k| (k as usize, &block.items[k as usize]))
        {
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
                Item::Image { rect, path, .. } => {
                    let r = at(rect);
                    let scale = frame.scale();
                    let size = ((r.w * scale).round() as u32, (r.h * scale).round() as u32);
                    let drawn = match md.images.get(path) {
                        Some(ImageSlot::Ready { pixels, .. }) => {
                            pixels.iter().find(|px| (px.width, px.height) == size)
                        }
                        _ => None,
                    };
                    match drawn {
                        Some(px) => frame.pixels((r.x, r.y), px.width, px.height, &px.rgba, area),
                        None => wanted.push((path.clone(), size)),
                    }
                }
                Item::Link { rect, url } => {
                    let r = at(rect).intersect(area);
                    if r.w > 0.0 && r.h > 0.0 {
                        md.links.push((r, url.to_string()));
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
                        md.links.push((hit, format!("{TASK}{at}")));
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
                        .map(|&s| layout::shift(s, left, top))
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
                    md.codes.push(r);
                    if r.contains(mouse.0, mouse.1) && area.contains(mouse.0, mouse.1) {
                        let button = copy_button(r);
                        code_button(frame, p, button, area, icons::copy);
                        md.links.push((button, COPY.to_owned()));
                        md.copy = Some(text.clone());
                        // 图的复制按钮左侧另有全屏查看按钮。
                        if *diagram {
                            let expand = Rect {
                                x: button.x - button.w - 4.0,
                                ..button
                            };
                            code_button(frame, p, expand, area, icons::expand);
                            md.links.push((expand, EXPAND.to_owned()));
                            md.expand = Some(text.clone());
                        }
                    }
                }
            }
        }
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
        table_toolbar(frame, p, table, align, area, mouse, &mut md.links);
    }
    if let Some((_, y, h)) = caret {
        // 竖条在内容左边的留白正中，宽 2 逻辑像素。
        let bar = Rect::new(left - pad / 2.0 - 1.0, area.y - scroll + y, 2.0, h);
        frame.rect_clipped(bar, p.accent.alpha(0.7), 1.0, area);
    }
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
        let shift = motion.as_ref().zip(block).map_or(0.0, |(m, b)| m.offset(b));
        let point = (x, y + shift, h);
        // 正文中的组字文字已排入所在的行，不再在其上重复绘制。
        let inline = laid.blocks.iter().any(|(_, b)| b.preedit);
        (point, (!inline).then(|| font_at(&laid, source)))
    });
    md.layout = Some(laid);
    md.motion = motion;
    if let Some(((x, y, h), font)) = live_point {
        let (x, y) = (left + x, area.y - scroll + y);
        app.ime_cursor = Some(draw_live_caret(app, frame, (x, y, h), font, area));
    }
    for (path, size) in wanted {
        app.request_markdown_pixels(&path, size);
    }
    draw_table_scrollbar(app, frame, chosen, area);
    draw_toc(app, frame, area, pad);

    // 和编辑区的滚动条一样：平时隐藏，滚动、悬停时显示，可以拖动。
    let owner = app.editor.active().map_or(0, |doc| doc.id.0);
    let content = app.markdown.height;
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

/// 画可横向滚动的表格被裁掉一侧的渐隐，提示可以横向滚动：`strip` 是显示区域（窗口坐标），
/// `off` 是滚动位置。渐隐的强度随离该侧尽头的距离在 [`TABLE_FADE`] 内线性变化，裁剪到 `area` 内。
fn draw_table_fades(
    frame: &mut Frame<'_>,
    p: &Palette,
    scroller: &layout::Scroller,
    strip: Rect,
    off: f32,
    area: Rect,
) {
    let max = scroller.max();
    if max <= 0.0 {
        return;
    }
    let table = Rect::new(strip.x, strip.y, strip.w, strip.h - layout::SCROLL_BAR_ROOM);
    let left = (off / TABLE_FADE).min(1.0);
    let right = ((max - off) / TABLE_FADE).min(1.0);
    frame.within(table.intersect(area), |frame| {
        if left > 0.0 {
            let edge = Rect::new(table.x, table.y, TABLE_FADE, table.h);
            frame.with_opacity(left, |f| crate::view::fade(f, edge, p.editor_bg, true));
        }
        if right > 0.0 {
            let edge = Rect::new(table.right() - TABLE_FADE, table.y, TABLE_FADE, table.h);
            frame.with_opacity(right, |f| crate::view::fade(f, edge, p.editor_bg, false));
        }
    });
}

/// 画表格 `chosen`（表格、滚动位置、是否因鼠标所在而选中）的水平滚动条，与其他区域的滚动条
/// 一致：平时隐藏，滚动、鼠标移到底边或拖动时显示（见 `App::scrollbar_look`），可以拖动。
fn draw_table_scrollbar(
    app: &mut App,
    frame: &mut Frame<'_>,
    chosen: Option<(TableBar, f32, bool)>,
    area: Rect,
) {
    app.markdown.table_bar = chosen.map(|(bar, ..)| bar);
    let Some((bar, off, hovered)) = chosen else {
        return;
    };
    if hovered {
        app.markdown.last_table = Some(bar.key);
    }
    let thumb = crate::layout::hscrollbar_thumb(bar.track, off, bar.content);
    // 滚动条状态的所属对象按表格区分：换到另一个表格时立即隐藏上一个表格的滚动条。
    let owner = ((bar.key.0 as u64) << 32) | bar.key.1 as u64;
    let look = app.scrollbar_look(
        ScrollArea::MarkdownTable,
        owner,
        off,
        thumb.map(|_| bar.track),
    );
    if let Some(thumb) = thumb {
        let p = app.palette;
        frame.within(area, |frame| crate::view::draw_thumb(frame, p, thumb, look));
    }
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

/// 标题指示条和列表里一条横线的颜色与粗细：已经读过的各节用次要色，当前所在的一节用强调色并
/// 加粗，还没读到的用更淡的颜色，像阅读进度。
fn toc_dash(p: &Palette, i: usize, current: Option<usize>) -> (Color, f32) {
    match current {
        Some(c) if i == c => (p.accent, TOC_BAR + 1.0),
        Some(c) if i < c => (p.text_muted, TOC_BAR),
        _ => (p.text_dim, TOC_BAR),
    }
}

/// 在预览左侧的留白里画标题指示条：每个标题一条横线，颜色表示读到哪里（见 [`toc_dash`]），
/// 当前所在的一节略长。鼠标停在指示条上时，标题列表在原处展开、盖住指示条：每行前面是同样的
/// 横线，后面是标题文字，越深的标题缩进越多；点击一项滚到该标题。
///
/// 左侧留白不小于 [`TOC_ROOM`] 时才画（分栏时预览较窄，通常不画）；标题少于两个时不画。可点击的
/// 位置写入 `app.markdown.toc`。
fn draw_toc(app: &mut App, frame: &mut Frame<'_>, area: Rect, pad: f32) {
    app.markdown.toc.clear();
    app.markdown.toc_tops.clear();
    app.markdown.toc_current = None;
    let (Some(parsed), Some(laid)) = (app.markdown.parsed.clone(), app.markdown.layout.as_ref())
    else {
        return;
    };
    if laid.left - area.x < TOC_ROOM {
        return;
    }
    // 各标题：（级别，文字，所在顶层块的上边缘，块高）。
    let headings: Vec<(u8, String, f32, f32)> = parsed
        .blocks
        .iter()
        .zip(laid.tops.iter().zip(&laid.blocks))
        .filter_map(|(block, (&top, (_, laid)))| match block {
            winc_preview::markdown::Block::Heading(level, inline) => {
                Some((*level, heading_title(&inline.text), top, laid.height))
            }
            _ => None,
        })
        .filter(|(_, title, ..)| !title.is_empty())
        .collect();
    if headings.len() < 2 {
        return;
    }
    let p = app.palette;
    let scroll = app.markdown.scroll();
    let max = (app.markdown.height - area.h).max(0.0);
    // 当前所在的一节：顶边已经滚到预览顶上（或以上）的最后一个标题。滚到底时，文末几个标题
    // 滚不到顶上，取屏幕里看得到的最后一个；在标题列表里点过、之后没有再滚动时，取点的那一个。
    let picked = app
        .markdown
        .toc_picked
        .filter(|&(i, at)| i < headings.len() && (at - scroll).abs() < 0.5)
        .map(|(i, _)| i);
    if picked.is_none() {
        app.markdown.toc_picked = None;
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
    let x0 = (laid.left - TOC_GAP - TOC_CURRENT).max(area.x + TOC_X);
    let y0 = area.y + pad;
    let left = laid.left;
    let width = laid.key.width;

    // 点过的标题在正文里铺一层强调色的底色，慢慢淡出。底色与正文同宽，左右与表格、代码块的
    // 边缘对齐。
    if let Some((i, clock)) = app.markdown.toc_flash
        && let Some(&(_, _, top, height)) = headings.get(i)
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
    // 列表展开着时，指示条的各条由列表里的横线接替（从原位滑到各行），这里不再画。
    let open = matches!(app.hover, Hover::MarkdownToc(_));
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
    if open {
        let size = app.fonts.ui;
        // 卡片在指示条处展开、盖住指示条，行里的横线与指示条在同一列；卡片不超出预览的左边。
        let x = (x0 - TOC_PAD - 8.0).max(area.x + 4.0);
        let dash_x = x0 - x;
        let text_x = dash_x + TOC_DASH + 10.0;
        let widest = headings
            .iter()
            .map(|(level, title, ..)| {
                depth(*level) * TOC_INDENT + frame.measure(FontKind::Ui, size, title)
            })
            .fold(0.0_f32, f32::max);
        let w = (widest + text_x + 8.0 + TOC_PAD)
            .clamp(180.0, 320.0)
            .min(area.right() - 16.0 - x);
        let fit = ((area.h - 16.0 - TOC_PAD * 2.0) / TOC_ROW).floor().max(1.0) as usize;
        let rows = headings.len().min(fit);
        // 放不下全部标题时，列出当前那一节附近的一段。
        let start = current
            .unwrap_or(0)
            .saturating_sub(rows / 2)
            .min(headings.len() - rows);
        let h = rows as f32 * TOC_ROW + TOC_PAD * 2.0;
        // 列表展开时，当前所在一节那一行的横线落在指示条里同一条的位置上，其余各行向上下展开，
        // 横线看起来没有移动；没有当前所在的一节时以第一行对齐第一条。超出预览时再挪进来。
        let anchor = current
            .filter(|&c| c < shown && (start..start + rows).contains(&c))
            .unwrap_or(start);
        let bar_mid = bar_y(anchor.min(shown.saturating_sub(1)));
        let row_mid = TOC_PAD + (anchor - start) as f32 * TOC_ROW + TOC_ROW / 2.0;
        let y = (bar_mid - row_mid)
            .min(area.bottom() - 8.0 - h)
            .max(area.y + 8.0);
        let card = Rect::new(x, y, w.max(0.0), h);
        // 展开时卡片和文字原地淡入；各行的横线从指示条里同一条的位置滑到所在的行，像指示条
        // 展开成列表，不会一下子跳到别处。
        let progress = app.markdown.toc_fade.map_or(1.0, |c| c.progress());
        let mut dashes = Vec::with_capacity(rows);
        frame.with_opacity(progress, |frame| {
            crate::view::draw_card(frame, p, card, crate::theme::RADIUS_POPUP, p.popup_bg);
            for (n, i) in (start..start + rows).enumerate() {
                let (level, title, ..) = &headings[i];
                let band = Rect::new(
                    card.x + TOC_PAD,
                    card.y + TOC_PAD + n as f32 * TOC_ROW,
                    card.w - TOC_PAD * 2.0,
                    TOC_ROW,
                );
                if app.hover == Hover::MarkdownToc(TocTarget::Heading(i)) {
                    frame.rect_clipped(band, p.hover, crate::theme::RADIUS_ROW, card);
                }
                let row_mid = band.y + band.h / 2.0;
                let from = if i < shown { bar_y(i) } else { row_mid };
                dashes.push((i, from + (row_mid - from) * progress));
                let tx = card.x + text_x + depth(*level) * TOC_INDENT;
                let room = (band.right() - 8.0 - tx).max(0.0);
                let title = crate::view::ellipsize(frame, title, size, room);
                // 当前所在一节的标题加粗显示。
                let faux = winc_render::Faux {
                    bold: current == Some(i),
                    ..Default::default()
                };
                let baseline = crate::view::ui_baseline(app, band, size);
                frame.text_faux(FontKind::Ui, faux, size, tx, baseline, &title, p.text, card);
                hits.push((band, TocTarget::Heading(i)));
            }
        });
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
        hits.push((card, TocTarget::Bars));
    }
    hits.push((bars, TocTarget::Bars));
    app.markdown.toc = hits;
    app.markdown.toc_tops = headings.iter().map(|h| h.2).collect();
    app.markdown.toc_current = current;
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

/// 重排之前各顶层块的排版和绘制位置，重排后据此计算过渡动画（见 `BlockMotion`）。
struct Before {
    /// 各块的内容哈希、排版结果和绘制时的上边缘（相对内容顶部、已减去滚动位置，含滑动偏移）。
    blocks: Vec<(u64, Arc<LaidBlock>, f32)>,
    /// 排版时的文档版本（见 [`LiveKey`]）；版本未变表示只移动了光标。
    revision: Option<u64>,
}

impl Before {
    fn of(app: &App, laid: &Laid) -> Before {
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
    fn motion(
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
type LiveKey = (usize, std::ops::Range<usize>, u64, Option<usize>, String);

/// `preedit` 是输入法正在组合的文字，在正文中排入光标所在的行，因此其变化同样需要重排。
fn live_key(live: &crate::app::preview::markdown::LiveLeaf, preedit: &str) -> LiveKey {
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
fn old_caret(
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
fn keep_caret_still(app: &mut App, still: Option<(usize, f32)>, laid: &Laid) {
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
fn lay_out(
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
    app.markdown.memo.prepare(layout::memo_key(|h| {
        use std::hash::Hash;
        for v in [key.width, key.base] {
            v.to_bits().hash(h);
        }
        for c in [key.text, key.background] {
            [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(h);
        }
        key.images.hash(h);
    }));
    let mut cx = Cx {
        frame,
        base: key.base,
        palette: p,
        colors: Colors::new(p),
        dark: p.dark,
        images: &app.markdown.images,
        doc: app.editor.active().and_then(|d| d.path.clone()),
        wanted: Vec::new(),
        remote: app.config.markdown_remote_images != Some(false),
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
            Some((_, text, leaf, parsed_end, caret, prose, preedit)) => layout::memo_key(|h| {
                use std::hash::Hash;
                (hash, &**text, leaf, parsed_end, caret, prose, preedit).hash(h);
            }),
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
fn line_box(baseline: f32, size: f32) -> (f32, f32) {
    (baseline - size * 1.05, size * 1.4)
}

impl Laid {
    /// `blocks` 范围里的各顶层块中能选中的文字，逐段返回：所在块的上边缘（相对内容顶部）、在
    /// 整份文字里的起点、横坐标、基线、字号、各字宽度和原文位置，以及所在的块和文字本身。
    fn spans(
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
    fn follow_caret(&mut self, source: usize) {
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
    fn sync_points(&self, total: usize) -> Vec<(f32, f32)> {
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
fn interpolate(points: &[(f32, f32)], at: f32, inverse: bool) -> f32 {
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
const CONTENT_SLACK: f32 = 12.0;

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
struct Hit {
    offset: usize,
    block: usize,
    source: Option<usize>,
}

/// 返回鼠标位置（在块 `blocks` 中，内容坐标）下的表格单元格：所在的块及单元格范围（块内坐标）。
fn table_cell(
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

fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
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
fn font_at(laid: &Laid, source: usize) -> Option<(winc_render::FontKind, winc_render::Faux, f32)> {
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
    let Some(font) = font.filter(|_| !app.preedit.is_empty()) else {
        if !app.preedit.is_empty() {
            return Rect::new(x, y, 2.0, h);
        }
        if app.caret_visible {
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
    let width: f32 = app
        .preedit
        .chars()
        .map(|c| frame.advance_faux(font, faux, size, c))
        .sum();
    frame.rect_clipped(Rect::new(x, y, width, h), p.editor_bg, 0.0, clip);
    frame.text_faux(font, faux, size, x, baseline, &app.preedit, p.text, clip);
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
/// 边长 28 逻辑像素，离代码块右边和上边各 6 逻辑像素（经验值）。
pub(crate) fn copy_button(code: Rect) -> Rect {
    const SIZE: f32 = 28.0;
    Rect::new(code.right() - SIZE - 6.0, code.y + 6.0, SIZE, SIZE)
}

/// 表格工具栏的按钮：链接中 [`TABLE`] 之后的操作名和按钮文字（三个对齐按钮绘制为图标）。
fn table_tools() -> [(&'static str, &'static str); 8] {
    [
        ("left", ""),
        ("center", ""),
        ("right", ""),
        ("row", t!("preview-table-insert-row")),
        ("column", t!("preview-table-insert-column")),
        ("delete-row", t!("preview-table-delete-row")),
        ("delete-column", t!("preview-table-delete-column")),
        ("delete", t!("preview-table-delete")),
    ]
}

/// 在光标所在的表格 `table`（窗口坐标）上方绘制工具栏。
///
/// 按钮包括光标所在列的对齐方式（`align` 对应的按钮显示为按下）、插入行和列、删除当前行和列、
/// 删除整张表，并作为链接加入 `links`。上方空间不足时工具栏贴紧预览上边缘，覆盖在表格上。
#[allow(clippy::too_many_arguments)]
fn table_toolbar(
    frame: &mut Frame<'_>,
    p: &Palette,
    table: Rect,
    align: winc_preview::markdown::Align,
    area: Rect,
    mouse: (f32, f32),
    links: &mut Vec<(Rect, String)>,
) {
    use winc_preview::markdown::Align;
    // 工具栏高 26、内边距 3、按钮字号 12（逻辑像素，经验值）。
    let (h, pad, size) = (26.0, 3.0, 12.0);
    let tools = table_tools();
    let widths: Vec<f32> = tools
        .iter()
        .map(|(_, label)| {
            if label.is_empty() {
                h - pad * 2.0
            } else {
                frame.measure(winc_render::FontKind::Ui, size, label) + 16.0
            }
        })
        .collect();
    // 对齐与插入之间、插入与删除之间、删除整张表之前各绘制一条竖线分隔。
    let gaps = [3, 5, 7];
    let w = widths.iter().sum::<f32>() + pad * 2.0 + gaps.len() as f32 * 9.0;
    let y = (table.y - h - 6.0).max(area.y + 2.0);
    let bar = Rect::new(table.x, y, w, h);
    frame.shadow(bar, 6.0, 8.0, 2.0, p.shadow, None);
    frame.rect_clipped(bar, p.popup_bg, 6.0, area);
    let mut x = bar.x + pad;
    for (i, ((op, label), bw)) in tools.iter().zip(&widths).enumerate() {
        if gaps.contains(&i) {
            let line = Rect::new(x + 4.0, y + 7.0, 1.0, h - 14.0);
            frame.rect_clipped(line, p.border, 0.0, area);
            x += 9.0;
        }
        let button = Rect::new(x, y + pad, *bw, h - pad * 2.0);
        let on = match *op {
            "left" => align == Align::Left,
            "center" => align == Align::Center,
            "right" => align == Align::Right,
            _ => false,
        };
        if on {
            frame.rect_clipped(button, p.accent.alpha(0.18), 4.0, area);
        } else if button.contains(mouse.0, mouse.1) {
            frame.rect_clipped(button, p.text.alpha(0.08), 4.0, area);
        }
        let color = if on { p.accent } else { p.text_muted };
        if label.is_empty() {
            // 对齐图标：四条长短交错的横线，按对齐方式靠左、居中或靠右排列。
            let (cx, cy) = (button.x + button.w / 2.0, button.y + button.h / 2.0);
            for (k, len) in [12.0, 8.0, 12.0, 8.0].into_iter().enumerate() {
                let lx = match *op {
                    "left" => cx - 6.0,
                    "center" => cx - len / 2.0,
                    _ => cx + 6.0 - len,
                };
                let ly = cy - 5.5 + k as f32 * 3.5;
                frame.rect_clipped(Rect::new(lx, ly, len, 1.5), color, 0.0, area);
            }
        } else {
            let baseline = button.y + button.h / 2.0 + size * 0.35;
            frame.text(
                winc_render::FontKind::Ui,
                size,
                x + 8.0,
                baseline,
                label,
                color,
                area,
            );
        }
        // 再次点击已选中的对齐方式时取消对齐，使对齐按钮可以切换开关。
        let op = if on { "none" } else { op };
        links.push((button, format!("{TABLE}{op}")));
        x += bw;
    }
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

#[cfg(test)]
pub(crate) mod tests;
