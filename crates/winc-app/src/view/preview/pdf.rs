//! PDF 预览的绘制：各页从上到下居中排列，按显示宽度渲染，位图与物理像素一一对应。
//!
//! 页面宽度适应编辑区，最宽不超过 [`MAX_PAGE_WIDTH`]；只请求视口内和视口上下各一屏范围内的页
//! （见 `App::request_pdf_pages`），尚未渲染完成的页先显示白色底，渲染失败的页在页面中央显示
//! 原因。右下角显示当前页号和总页数。

use winc_render::{FontKind, Frame, Rect};

use crate::app::App;
use crate::theme::RADIUS_POPUP;

/// 页面列四周的留白（逻辑像素），与 Markdown 预览的留白相同。
const PAD: f32 = 32.0;
/// 相邻两页之间的间距（逻辑像素），为页面列留白 [`PAD`] 的一半。
const GAP: f32 = 16.0;
/// 页面的最大显示宽度（逻辑像素）：A4 页在该宽度下放大约 1.45 倍，10.5 磅的正文约为 15 逻辑
/// 像素，便于阅读；更宽的编辑区不再放大，同时限制了每页位图的大小。
pub(crate) const MAX_PAGE_WIDTH: f32 = 860.0;

/// 各页在内容中的位置（逻辑像素，相对内容左上角）：页面宽度都按最宽的一页缩放到同一比例。
fn page_rects(pages: &[winc_preview::pdf::PageSize], width: f32) -> (Vec<Rect>, f32) {
    let widest = pages.iter().map(|p| p.width).fold(0.0, f64::max).max(1.0);
    let shown = (width - PAD * 2.0).clamp(1.0, MAX_PAGE_WIDTH);
    let zoom = f64::from(shown) / widest;
    let mut y = PAD;
    let rects = pages
        .iter()
        .map(|page| {
            let (w, h) = ((page.width * zoom) as f32, (page.height * zoom) as f32);
            let rect = Rect::new((width - w) / 2.0, y, w, h);
            y += h + GAP;
            rect
        })
        .collect();
    (rects, y - GAP + PAD)
}

/// 在 `area`（窗口坐标）里画当前标签的 PDF。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    app.pdf.area = area;
    let Some(Ok(pages)) = app.pdf.pages.clone() else {
        return;
    };
    let (rects, height) = page_rects(&pages, area.w);
    app.pdf.height = height;
    app.pdf.scroll = app.pdf.scroll.clamp(0.0, app.pdf.max_scroll());
    let scroll = app.pdf.scroll;
    let scale = frame.scale();
    // 位图按物理像素对齐：位置和大小都取整到物理像素，绘制时不经插值，线条和文字保持清晰。
    let snap = |v: f32| (v * scale).round() / scale;

    let mut wanted = Vec::new();
    let mut current = 0;
    for (i, rect) in rects.iter().enumerate() {
        let top = area.y + rect.y - scroll;
        // 视口上下各一屏范围内的页也提前请求，滚动到时通常已渲染完成。
        let reach = area.h;
        if top + rect.h < area.y - reach || top > area.bottom() + reach {
            continue;
        }
        let (w, h) = (
            (rect.w * scale).round().max(1.0) as u32,
            (rect.h * scale).round().max(1.0) as u32,
        );
        wanted.push((i, w, h));
        if top <= area.y + area.h / 2.0 {
            current = i;
        }
        if top + rect.h < area.y || top > area.bottom() {
            continue;
        }
        let page = Rect::new(
            snap(area.x + rect.x),
            snap(top),
            w as f32 / scale,
            h as f32 / scale,
        );
        // 页面四周一圈细边框与背景区分，裁剪到预览区域内：滚出顶部的页不画到标签栏上。
        frame.rect_clipped(page.inset(-1.0, -1.0), p.border, 0.0, area);
        frame.rect_clipped(page, winc_render::Color::hex(0xffffff), 0.0, area);
        if let Some(pixels) = app
            .pdf
            .rendered
            .get(&i)
            .filter(|px| px.width == w && px.height == h)
        {
            frame.pixels((page.x, page.y), w, h, &pixels.rgba, area);
        } else if let Some(reason) = app.pdf.failed.get(&(i, w)) {
            draw_failure(app, frame, page.intersect(area), reason);
        }
    }
    app.request_pdf_pages(&wanted);
    draw_page_number(app, frame, area, current, pages.len());
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

/// 在右下角画「当前页 / 总页数」：当前页是视口中线所在的页。
fn draw_page_number(app: &App, frame: &mut Frame<'_>, area: Rect, current: usize, total: usize) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let text = format!("{} / {total}", current + 1);
    let tw = frame.measure(FontKind::Ui, size, &text);
    let (w, h) = ((tw + 16.0).round(), 24.0);
    let pill = Rect::new(area.right() - w - 16.0, area.bottom() - h - 16.0, w, h);
    frame.rect_clipped(pill, p.popup_border, RADIUS_POPUP, area);
    frame.rect_clipped(pill.inset(1.0, 1.0), p.popup_bg, RADIUS_POPUP - 1.0, area);
    let baseline = crate::view::ui_baseline(app, pill, size);
    frame.text(
        FontKind::Ui,
        size,
        (pill.x + (w - tw) / 2.0).round(),
        baseline,
        &text,
        p.text_muted,
        area,
    );
}
