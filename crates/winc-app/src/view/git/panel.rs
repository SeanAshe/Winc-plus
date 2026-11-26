//! Git 面板的绘制：卡片、标题行（各个日志的标签，「移至编辑区」、最大化和隐藏），以及其中的
//! 日志。

use crate::app::git::panel::{PANEL_HEADER, PanelHit};
use crate::view::*;

/// 标题行中按钮的边长（逻辑像素），与终端面板标题行的按钮相同。
const BUTTON: f32 = 24.0;
/// 标题行按钮之间、标签之间的间距（逻辑像素）。
const GAP: f32 = 4.0;
/// 标签中文字两侧的留白（逻辑像素）。
const TAB_PAD: f32 = 10.0;
/// 标签上关闭按钮的宽度（逻辑像素）：图标约 7 宽，左右各留约 4。
const TAB_CLOSE: f32 = 16.0;
/// 文字与关闭按钮之间、关闭按钮与标签右边之间的空隙（逻辑像素）。
const TAB_INNER: f32 = 4.0;
/// 一个标签最宽的宽度（逻辑像素）：文件历史的标题较长，放不下时截短。
const TAB_MAX: f32 = 220.0;

/// 绘制 Git 面板；面板模式下面板显示着或在显示、隐藏的过渡中时绘制。
pub(in crate::view) fn draw_git_panel(app: &mut App, frame: &mut Frame<'_>) {
    let panel = &app.git.panel;
    if !panel.docked || panel.docs.is_empty() || app.layout.git_panel.h <= 0.0 {
        return;
    }
    let card = app.layout.git_panel;
    let p = app.palette;
    frame.rect(card, p.editor_bg);
    let header = Rect::new(card.x, card.y, card.w, PANEL_HEADER);
    let mut hits = Vec::new();
    let buttons = draw_buttons(app, frame, header, &mut hits);
    draw_tabs(
        app,
        frame,
        Rect::new(header.x, header.y, buttons - header.x, header.h),
        &mut hits,
    );
    frame.rect(
        Rect::new(card.x, header.bottom() - 1.0, card.w, 1.0),
        p.separator,
    );
    // 正在隐藏时画出来的位置不记下：面板正在滑走，点击落到下面的部件上。
    let shown = app.git_panel_shown();
    if shown {
        app.hits.git_panel = hits;
    }
    let kept = (!shown).then(|| std::mem::take(&mut app.hits.log));
    if let Some(id) = app.log_doc().map(|d| d.id) {
        crate::view::git::log::draw_log(app, frame, id);
    }
    for (rect, _) in &mut app.hits.log {
        *rect = rect.intersect(card);
    }
    if let Some(kept) = kept {
        app.hits.log = kept;
    }
}

/// 绘制标题行右端的按钮：「移至编辑区」、最大化或还原、隐藏；返回最左一个按钮左边的横坐标。
fn draw_buttons(
    app: &App,
    frame: &mut Frame<'_>,
    header: Rect,
    hits: &mut Vec<(Rect, PanelHit)>,
) -> f32 {
    let p = app.palette;
    let y = header.y + (header.h - BUTTON) / 2.0;
    let slot = |i: usize| {
        let x = header.right() - 8.0 - (i + 1) as f32 * BUTTON - i as f32 * GAP;
        Rect::new(x, y, BUTTON, BUTTON)
    };
    let items = [PanelHit::Hide, PanelHit::Maximize, PanelHit::ToEditor];
    for (i, hit) in items.into_iter().enumerate() {
        let rect = slot(i);
        let on = app.pointer.hover == Hover::GitPanel(hit);
        draw_button(frame, p, rect, header, on, false);
        let color = if on { p.text } else { p.text_muted };
        let c = center(rect);
        match hit {
            PanelHit::Hide => icons::collapse(frame, c, icons::Direction::Down, color, header),
            PanelHit::Maximize if app.git.panel.full => icons::shrink(frame, c, color, header),
            PanelHit::Maximize => icons::expand(frame, c, color, header),
            _ => icons::external(frame, c, color, header),
        }
        hits.push((rect, hit));
    }
    slot(items.len() - 1).x - GAP
}

/// 绘制标题行左侧各个日志的标签：显示的那个铺选中的底色，悬停时右端出现关闭按钮。放不下全部
/// 标签时从显示的那个往前留出能放下的几个，显示的标签总在标题行中。
fn draw_tabs(app: &App, frame: &mut Frame<'_>, area: Rect, hits: &mut Vec<(Rect, PanelHit)>) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let y = area.y + (area.h - BUTTON) / 2.0;
    let chrome = TAB_PAD + 2.0 * TAB_INNER + TAB_CLOSE;
    let labels: Vec<(std::borrow::Cow<'_, str>, f32)> = (app.git.panel.docs.iter())
        .map(|doc| {
            let title = doc.title.as_deref().unwrap_or_default();
            let text = ellipsize(frame, title, size, TAB_MAX - chrome);
            let w = frame.measure(FontKind::Ui, size, &text) + chrome;
            (text, w)
        })
        .collect();
    let shown = app.git.panel.at;
    let room = area.w - 8.0;
    let mut start = shown.min(labels.len().saturating_sub(1));
    let mut used = labels.get(start).map_or(0.0, |(_, w)| *w);
    while start > 0 && used + GAP + labels[start - 1].1 <= room {
        start -= 1;
        used += GAP + labels[start].1;
    }
    let mut x = area.x + 8.0;
    for (i, (text, w)) in labels.iter().enumerate().skip(start) {
        if x + w > area.right() && i != shown {
            break;
        }
        let tab = Rect::new(x, y, *w, BUTTON);
        let close_x = tab.right() - TAB_INNER - TAB_CLOSE;
        let close = Rect::new(close_x, y + 4.0, TAB_CLOSE, BUTTON - 8.0);
        let hovered = matches!(
            app.pointer.hover,
            Hover::GitPanel(PanelHit::Tab(t) | PanelHit::Close(t)) if t == i
        );
        let fill = if i == shown {
            Some(p.selected_inactive)
        } else if hovered {
            Some(p.hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            frame.rect_clipped(tab, fill, RADIUS_ROW, area);
        }
        let color = if i == shown { p.text } else { p.text_muted };
        ui_text_small(frame, app, tab.x + TAB_PAD, tab, text, color, area);
        if hovered || i == shown {
            let on = app.pointer.hover == Hover::GitPanel(PanelHit::Close(i));
            if on {
                frame.rect_clipped(close, p.hover, RADIUS_ROW, area);
            }
            let color = if on { p.text } else { p.text_dim };
            icons::close(frame, center(close), 6.0, color, area);
            hits.push((close.intersect(area), PanelHit::Close(i)));
        }
        hits.push((tab.intersect(area), PanelHit::Tab(i)));
        x = tab.right() + GAP;
    }
}
