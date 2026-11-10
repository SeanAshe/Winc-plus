//! PDF 预览的绘制：各页从上到下居中排列，按显示宽度渲染，位图与物理像素一一对应。
//!
//! 页面的位置由 `PdfPreview::lay_out` 排出；只请求视口内和视口上下各半屏范围内的页（见
//! `App::request_pdf_pages`），尚未渲染的页显示白色底，渲染失败的页在页面中央显示原因。缩放后
//! 新尺寸的位图渲染完成之前，先把旧位图缩放显示。左侧是缩略图栏，右下角是工具栏。

use winc_render::{FontKind, Frame, Rect};

use crate::app::preview::pdf::{GAP, PdfTool, ZOOM_RANGE};
use crate::app::{App, Hover};
use crate::theme::RADIUS_POPUP;

/// 缩略图栏的宽度（逻辑像素）。
const STRIP_WIDTH: f32 = 136.0;
/// 缩略图的宽度（逻辑像素）：A4 页在该宽度下高约 136，一屏可显示五六页。
const THUMB_WIDTH: f32 = 96.0;
/// 缩略图下方页码一行的高度（逻辑像素）。
const THUMB_LABEL: f32 = 20.0;
/// 相邻两项之间、以及首尾与栏边缘之间的间距（逻辑像素）。
const THUMB_GAP: f32 = 12.0;
/// 显示缩略图栏所需的预览区域宽度（逻辑像素）：更窄时正文的页面过小。
const STRIP_MIN_AREA: f32 = 560.0;

/// 在 `area`（窗口坐标）里画当前标签的 PDF。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    app.pdf.area = area;
    let Some(Ok(pages)) = app.pdf.pages.clone() else {
        return;
    };
    let scale = frame.scale();
    // 位图按物理像素对齐：位置和大小都取整到物理像素，绘制时不经插值，线条和文字保持清晰。
    let snap = |v: f32| (v * scale).round() / scale;

    // 只有一页，或预览区域过窄时没有缩略图栏。滑入、滑出期间栏的可见宽度逐帧变化；正文按最终
    // 的宽度排版，只随栏平移，页面的大小不在过渡中反复改变。
    let strip_fits = pages.len() > 1 && area.w >= STRIP_MIN_AREA;
    let open = strip_fits && !app.state.pdf_thumbnails_hidden;
    let shown = match app.pdf.strip_motion.filter(|_| strip_fits) {
        Some((true, clock)) => clock.between(0.0, STRIP_WIDTH),
        Some((false, clock)) => clock.between(STRIP_WIDTH, 0.0),
        None if open => STRIP_WIDTH,
        None => 0.0,
    };
    let shown = snap(shown);
    let body = Rect::new(area.x + shown, area.y, area.w - shown, area.h);
    let fixed = if open { STRIP_WIDTH } else { 0.0 };
    let (rects, ratio) = app.pdf.lay_out(&pages, area.w - fixed);
    let (scroll, scroll_x, height) = (app.pdf.scroll, app.pdf.scroll_x, app.pdf.height);
    // 放大后每页的位图更大，只请求视口内的页；否则视口上下各半屏范围内的页也提前请求。
    let reach = if app.pdf.zoom > 1.0 {
        0.0
    } else {
        body.h / 2.0
    };

    let mut wanted = Vec::new();
    let mut current = 0;
    for (i, rect) in rects.iter().enumerate() {
        let top = body.y + rect.y - scroll;
        if top + rect.h < body.y - reach || top > body.bottom() + reach {
            continue;
        }
        let px = |v: f32| (v * scale).round().max(1.0) as u32;
        // 请求的是最终倍数下的大小；缩放过渡期间本帧显示的大小与它不同。
        let (w, h) = (px(rect.w * ratio), px(rect.h * ratio));
        wanted.push((i, w, h));
        if top <= body.y + body.h / 2.0 {
            current = i;
        }
        if top + rect.h < body.y || top > body.bottom() {
            continue;
        }
        let (dw, dh) = (px(rect.w), px(rect.h));
        let page = Rect::new(
            snap(body.x + rect.x - scroll_x),
            snap(top),
            dw as f32 / scale,
            dh as f32 / scale,
        );
        // 页面四周一圈细边框与背景区分，裁剪到正文区域内：滚出顶部的页不画到标签栏上。
        frame.rect_clipped(page.inset(-1.0, -1.0), p.border, 0.0, body);
        frame.rect_clipped(page, winc_render::Color::hex(0xffffff), 0.0, body);
        match app.pdf.rendered.get(&i) {
            Some(pixels) if (pixels.width, pixels.height) == (dw, dh) => {
                frame.pixels((page.x, page.y), dw, dh, &pixels.rgba, body);
            }
            Some(pixels) => {
                frame.pixels_scaled(page, pixels.width, pixels.height, &pixels.rgba, body);
            }
            None => {
                if let Some(reason) = app.pdf.failed.get(&(i, w)) {
                    draw_failure(app, frame, page.intersect(body), reason);
                }
            }
        }
    }
    let thumbs = if shown > 0.0 {
        let strip = Rect::new(body.x - STRIP_WIDTH, area.y, STRIP_WIDTH, area.h);
        let visible = Rect::new(area.x, area.y, shown, area.h);
        draw_strip(app, frame, (strip, visible), &pages, &rects, current)
    } else {
        Vec::new()
    };
    app.request_pdf_pages(&wanted, &thumbs);
    draw_toolbar(
        app,
        frame,
        body,
        (current, pages.len()),
        strip_fits.then_some(open),
    );
    let owner = app.editor.active().map_or(0, |doc| doc.id.0);
    let bar = crate::view::ScrollArea::Pdf;
    crate::view::draw_scrollbar(app, frame, bar, owner, area, scroll, height);
}

/// 画缩略图栏，返回这一帧需要的缩略图：（页号，位图宽度，位图高度）。
///
/// `strip` 是栏的位置（窗口坐标），`visible` 是其中可见的部分：滑入、滑出期间栏有一部分在预览
/// 区域之外。`rects` 是各页在正文中的位置，`current` 是当前页；当前页改变时把它滚入栏内。
fn draw_strip(
    app: &mut App,
    frame: &mut Frame<'_>,
    (strip, visible): (Rect, Rect),
    pages: &[winc_preview::pdf::PageSize],
    rects: &[Rect],
    current: usize,
) -> Vec<(usize, u32, u32)> {
    let p = app.palette;
    frame.rect_clipped(strip, p.sidebar_bg, 0.0, visible);
    let edge = Rect::new(strip.right() - 1.0, strip.y, 1.0, strip.h);
    frame.rect_clipped(edge, p.border, 0.0, visible);

    // 各项在栏内的位置（顶部，缩略图高度），相对内容顶部。
    let mut y = THUMB_GAP;
    let items: Vec<(f32, f32)> = (pages.iter())
        .map(|page| {
            let h = (f64::from(THUMB_WIDTH) * page.height / page.width.max(1.0)) as f32;
            let item = (y, h);
            y += h + THUMB_LABEL + THUMB_GAP;
            item
        })
        .collect();
    let pdf = &mut app.pdf;
    (pdf.strip, pdf.strip_height) = (visible, y);
    if pdf.current != current
        && let Some(&(top, h)) = items.get(current)
    {
        pdf.current = current;
        let below = top + h + THUMB_LABEL + THUMB_GAP - strip.h;
        pdf.strip_scroll = pdf.strip_scroll.max(below).min(top - THUMB_GAP);
    }
    pdf.strip_scroll = pdf.strip_scroll.clamp(0.0, (y - strip.h).max(0.0));
    let scroll = pdf.strip_scroll;

    let scale = frame.scale();
    let snap = |v: f32| (v * scale).round() / scale;
    let x = strip.x + (strip.w - THUMB_WIDTH) / 2.0;
    let size = app.fonts.ui_small;
    let mut wanted = Vec::new();
    for (i, &(top, h)) in items.iter().enumerate() {
        let top = strip.y + top - scroll;
        // 栏外上下各一项的缩略图也提前请求；更远的不保留，缩略图渲染得快。
        let item = h + THUMB_LABEL + THUMB_GAP;
        if top + item < strip.y - item || top > strip.bottom() + item {
            continue;
        }
        let (w, h) = (
            (THUMB_WIDTH * scale).round().max(1.0) as u32,
            (h * scale).round().max(1.0) as u32,
        );
        wanted.push((i, w, h));
        let thumb = Rect::new(snap(x), snap(top), w as f32 / scale, h as f32 / scale);
        if thumb.bottom() + THUMB_LABEL < strip.y || thumb.y > strip.bottom() {
            continue;
        }
        let (line, color, label) = match i == current {
            true => (2.0, p.accent, p.text),
            false => (1.0, p.border, p.text_muted),
        };
        frame.rect_clipped(thumb.inset(-line, -line), color, 0.0, visible);
        frame.rect_clipped(thumb, winc_render::Color::hex(0xffffff), 0.0, visible);
        let ready =
            |px: &&std::sync::Arc<winc_preview::image::Pixels>| px.width == w && px.height == h;
        if let Some(pixels) = app.pdf.thumbs.get(&i).filter(ready) {
            frame.pixels((thumb.x, thumb.y), w, h, &pixels.rgba, visible);
        }
        let text = (i + 1).to_string();
        let tw = frame.measure(FontKind::Ui, size, &text);
        let row = Rect::new(strip.x, thumb.bottom(), strip.w, THUMB_LABEL);
        let baseline = crate::view::ui_baseline(app, row, size);
        let at = (strip.x + (strip.w - tw) / 2.0).round();
        frame.text(FontKind::Ui, size, at, baseline, &text, label, visible);
        let hit = Rect::new(thumb.x, thumb.y, thumb.w, thumb.h + THUMB_LABEL).intersect(visible);
        let to = rects.get(i).map_or(0.0, |rect| (rect.y - GAP).max(0.0));
        app.hits.pdf_thumbs.push((hit, to));
    }
    // 滑入、滑出期间不画滚动条。
    if visible.w >= strip.w {
        let owner = app.editor.active().map_or(0, |doc| doc.id.0);
        let bar = crate::view::ScrollArea::PdfStrip;
        crate::view::draw_scrollbar(app, frame, bar, owner, strip, scroll, y);
    }
    wanted
}

/// 页面背景固定为白色，失败原因用固定的中灰色，在浅色和深色主题下都清晰可读。
const FAILURE_TEXT: u32 = 0x6e6e73;

/// 在渲染失败的页面 `page`（窗口坐标，已裁剪到预览区域）中央画失败的原因，过长时截短。
fn draw_failure(app: &App, frame: &mut Frame<'_>, page: Rect, reason: &str) {
    if page.w <= 0.0 || page.h <= 0.0 {
        return;
    }
    let size = app.fonts.ui_small;
    let room = (page.w - 32.0).max(0.0);
    let text = crate::view::ellipsize(frame, reason, size, room);
    let tw = frame.measure(FontKind::Ui, size, &text);
    let row = Rect::new(page.x, page.y + page.h / 2.0 - 12.0, page.w, 24.0);
    let baseline = crate::view::ui_baseline(app, row, size);
    frame.text(
        FontKind::Ui,
        size,
        (page.x + (page.w - tw) / 2.0).round(),
        baseline,
        &text,
        winc_render::Color::hex(FAILURE_TEXT),
        page,
    );
}

/// 工具栏按钮的边长（逻辑像素），与 Markdown 预览中查看图表的工具栏相同。
const BUTTON: f32 = 28.0;
/// 工具栏中显示倍数的按钮的宽度（逻辑像素）：界面小字号下可容纳「200%」。
const PERCENT: f32 = 48.0;
/// 工具栏与正文区域右边、下边的距离（逻辑像素）。
const INSET: f32 = 16.0;
/// 工具栏按钮与工具栏边缘、按钮与分隔线之间的间距（逻辑像素）。
const TOOL_PAD: f32 = 4.0;

/// 在正文区域 `body` 的右下角画工具栏：缩略图栏的开关、缩小、当前倍数（点击后还原）、放大，
/// 以及「当前页 / 总页数」；按钮的位置记入 `Hits::pdf_tools`。
///
/// `pages` 是（当前页，总页数），当前页是视口中线所在的页。`strip` 为缩略图栏是否显示，没有
/// 缩略图栏可用时为 `None`，此时没有开关。
fn draw_toolbar(
    app: &mut App,
    frame: &mut Frame<'_>,
    body: Rect,
    (current, total): (usize, usize),
    strip: Option<bool>,
) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let label = format!("{} / {total}", current + 1);
    let label_w = (frame.measure(FontKind::Ui, size, &label) + TOOL_PAD * 4.0).round();
    // 各组之间以竖线分隔。
    let gap = TOOL_PAD * 2.0 + 1.0;
    let toggle = if strip.is_some() { BUTTON + gap } else { 0.0 };
    let inner = toggle + BUTTON * 2.0 + PERCENT + gap + label_w;
    let (w, h) = (inner + TOOL_PAD * 2.0, BUTTON + TOOL_PAD * 2.0);
    // 位置取整：图标和文字各自对齐到整像素，外框不在整像素上时两者会上下错开。
    let (x, y) = (body.right() - INSET - w, body.bottom() - INSET - h);
    let bar = Rect::new(x.round(), y.round(), w, h);
    let covered = frame.covered(bar, RADIUS_POPUP);
    frame.shadow(bar, RADIUS_POPUP, 12.0, 4.0, p.shadow, Some(covered));
    frame.rect_clipped(bar, p.popup_border, RADIUS_POPUP, body);
    frame.rect_clipped(bar.inset(1.0, 1.0), p.popup_bg, RADIUS_POPUP - 1.0, body);

    let mut x = bar.x + TOOL_PAD;
    let separator = |frame: &mut Frame<'_>, x: f32| {
        let line = Rect::new(
            x + TOOL_PAD,
            bar.y + TOOL_PAD * 2.0,
            1.0,
            bar.h - TOOL_PAD * 4.0,
        );
        frame.rect_clipped(line, p.separator, 0.0, body);
    };
    let zoom = app.pdf.zoom;
    let tools = [
        (PdfTool::Thumbnails, BUTTON, strip.is_some()),
        (PdfTool::ZoomOut, BUTTON, true),
        (PdfTool::ZoomReset, PERCENT, true),
        (PdfTool::ZoomIn, BUTTON, true),
    ];
    for (tool, w, shown) in tools {
        if !shown {
            continue;
        }
        let r = Rect::new(x, bar.y + TOOL_PAD, w, BUTTON);
        if app.hover == Hover::PdfTool(tool) {
            frame.rect_clipped(r, p.hover, RADIUS_POPUP - TOOL_PAD, body);
        }
        let center = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        // 已到最小、最大倍数的按钮变淡。
        let faded = |end: f32| {
            if zoom == end {
                p.text_muted.alpha(0.4)
            } else {
                p.text_muted
            }
        };
        match tool {
            PdfTool::Thumbnails => {
                crate::icons::sidebar(frame, center, strip == Some(true), p.text_muted, body);
            }
            PdfTool::ZoomOut => crate::icons::minus(frame, center, 10.0, faded(ZOOM_RANGE.0), body),
            PdfTool::ZoomIn => crate::icons::plus(frame, center, 10.0, faded(ZOOM_RANGE.1), body),
            PdfTool::ZoomReset => {
                let text = format!("{:.0}%", zoom * 100.0);
                let tw = frame.measure(FontKind::Ui, size, &text);
                let baseline = crate::view::ui_baseline(app, r, size);
                let at = (center.0 - tw / 2.0).round();
                frame.text(FontKind::Ui, size, at, baseline, &text, p.text, body);
            }
        }
        app.hits.pdf_tools.push((r, tool));
        x += w;
        if tool == PdfTool::Thumbnails {
            separator(frame, x);
            x += gap;
        }
    }
    separator(frame, x);
    x += gap;
    let row = Rect::new(x, bar.y + TOOL_PAD, label_w, BUTTON);
    let tw = frame.measure(FontKind::Ui, size, &label);
    let baseline = crate::view::ui_baseline(app, row, size);
    let at = (row.x + (row.w - tw) / 2.0).round();
    frame.text(FontKind::Ui, size, at, baseline, &label, p.text_muted, body);
}
