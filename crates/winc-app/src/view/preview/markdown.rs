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
use crate::app::preview::markdown::{ImageSlot, TableBar, TocTarget};
use crate::app::{App, Focus, ScrollArea};
use crate::icons;
use crate::theme::Palette;

mod viewer;

pub(in crate::view) use viewer::draw_diagram_view;
use winc_typeset::Item;

use crate::app::preview::markdown::laid::*;

/// 在 `area`（窗口坐标，逻辑像素）里画当前文档的 Markdown 预览，分栏时是右半边，只看预览时
/// 是整个编辑区。
///
/// 排版结果和光标位置写回 `app.markdown`，可点击范围记入 `app.hits`；需要读取的图片记进请求，下一帧再画。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    app.markdown.area = area;
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
    let live_key = (live.as_ref()).map(|l| live_key(l, app.preedit_at(Focus::Editor)));
    let blank = live.as_ref().is_some_and(|l| l.blank);
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
        let muted = p.text_muted;
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
        // 段落之后的空行上组字使用正文的字体，不沿用上方标题的字体。
        let body = (winc_render::FontKind::Ui, Default::default(), key.base);
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
    scroller: &winc_typeset::Scroller,
    strip: Rect,
    off: f32,
    area: Rect,
) {
    let max = scroller.max();
    if max <= 0.0 {
        return;
    }
    let table = Rect::new(
        strip.x,
        strip.y,
        strip.w,
        strip.h - winc_typeset::SCROLL_BAR_ROOM,
    );
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
    app.hits.markdown_table_bar = chosen.map(|(bar, ..)| bar);
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
/// 位置写入 `app.hits.markdown_toc`。
fn draw_toc(app: &mut App, frame: &mut Frame<'_>, area: Rect, pad: f32) {
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
    app.hits.markdown_toc = hits;
    app.hits.markdown_toc_tops = headings.iter().map(|h| h.2).collect();
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
    let width: f32 = preedit
        .chars()
        .map(|c| frame.advance_faux(font, faux, size, c))
        .sum();
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
