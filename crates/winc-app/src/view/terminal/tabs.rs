//! 横向终端标签栏的排版与绘制。

use super::*;
use crate::app::terminal::strip::{fit_named_tabs, tabs_narrowed};

pub(super) fn draw_header_tabs(
    app: &mut App,
    frame: &mut Frame<'_>,
    area: Rect,
    y: f32,
) -> (f32, Vec<(Rect, HeaderHit)>) {
    if app.terminal_tab_list().is_some() {
        return (area.x, Vec::new());
    }
    draw_horizontal(app, frame, area, y)
}

fn draw_horizontal(
    app: &mut App,
    frame: &mut Frame<'_>,
    area: Rect,
    y: f32,
) -> (f32, Vec<(Rect, HeaderHit)>) {
    let p = app.palette;
    let (left, room) = (area.x, area.w);
    let names: Vec<f32> = (app.terminal.tabs.iter())
        .map(|tab| {
            // 正在改名字的标签跟着输入的字变宽。
            let label = match &app.terminal.rename {
                Some((id, field)) if *id == tab.id => field.text(),
                _ => tab.label(),
            };
            frame.measure(FontKind::Ui, app.fonts.ui, label) + 36.0
        })
        .collect();
    let widths = fit_named_tabs(&names, room);
    // 最窄时仍放不下：标签栏可以横向滚动，切换标签或增减标签时滚到正在看的标签；超出的部分裁剪，
    // 「+」始终显示。还没有标签（刚打开、shell 尚未启动）时不滚动。
    let span = |n: usize| {
        let n = n.min(widths.len());
        widths[..n].iter().sum::<f32>() + TAB_GAP * n.saturating_sub(1) as f32
    };
    let total = span(widths.len());
    let max_scroll = (total - room).max(0.0);
    let active = app.terminal.active;
    let current = app
        .terminal
        .tabs
        .get(active)
        .map(|tab| (tab.id, widths.len(), room));
    let strip = &mut app.terminal.strip;
    let mut scroll = strip.scroll;
    if strip.revealed != current {
        strip.revealed = current;
        let start = (span(active + 1) - widths.get(active).copied().unwrap_or(0.0)).max(0.0);
        scroll = scroll.min(start).max(span(active + 1) - room);
    }
    let scroll = scroll.clamp(0.0, max_scroll);
    strip.scroll = scroll;
    strip.max_scroll = max_scroll;
    app.terminal.strip.sizes.clone_from(&widths);
    app.terminal.strip.start = left - scroll;
    let dragged = app.dragged_terminal_tab();
    let mut rename = app.terminal.rename.take();
    let mut caret = None;
    let strip = &app.terminal.strip;
    let mut hits = Vec::new();
    let mut x = left - scroll;
    let mut lifted = None;
    // 正在看的、悬停的、拖着的标签有自己的底色，两边不绘制竖线。
    let hovered = match app.pointer.hover {
        Hover::TerminalHeader(HeaderHit::Tab(i) | HeaderHit::Close(i)) => Some(i),
        _ => None,
    };
    let plain = |i: usize| {
        i != app.terminal.active
            && Some(i) != hovered
            && dragged.is_none_or(|(id, _)| id != app.terminal.tabs[i].id)
    };

    // 每个位置先绘制在该处关闭的标签（内容淡出后宽度收起），再绘制该处的标签：新建的标签从零宽
    // 展开，内容随后淡入；拖动中的标签最后绘制，位于其他标签之上。
    for slot in 0..=widths.len() {
        for ghost in strip.closing.iter().filter(|g| g.at == slot) {
            let t = ghost.clock.progress();
            let rect = Rect::new(x, y, ghost.size, TAB_H);
            let clip = rect.intersect(area);
            frame.with_opacity(ghost.fade.fade_out(), |frame| {
                let color = if ghost.active { p.text } else { p.text_muted };
                let state = (ghost.active, false);
                draw_tab(app, frame, rect, clip, &ghost.label, state, color);
            });
            x += (ghost.size + TAB_GAP) * (1.0 - t);
        }
        let Some(&w) = widths.get(slot) else {
            break;
        };
        let tab = &app.terminal.tabs[slot];
        let grow = strip.grow(tab.id);
        match dragged {
            Some((id, at)) if id == tab.id => lifted = Some((slot, at)),
            _ => {
                let rect = Rect::new(x + strip.slide(tab.id), y, w, TAB_H);
                let clip = Rect::new(rect.x, y, w * grow, TAB_H).intersect(area);
                frame.with_opacity(reveal(grow), |frame| {
                    // 竖线绘制在左边的间距正中，跟着标签一起滑。
                    if slot > 0 && plain(slot) && plain(slot - 1) {
                        let h = TAB_H / 2.0;
                        let line = Rect::new(rect.x - TAB_GAP / 2.0 - 0.5, y + h / 2.0, 1.0, h);
                        frame.rect_clipped(line, p.tab_divider, 0.0, area);
                    }
                    let field = rename.as_mut();
                    caret = caret.or(draw_terminal_tab(
                        app, frame, slot, rect, clip, &mut hits, field,
                    ));
                });
            }
        }
        x += (w + TAB_GAP) * grow;
    }
    if let Some((slot, at)) = lifted {
        let rect = Rect::new(at, y, widths[slot], TAB_H);
        let field = rename.as_mut();
        caret = caret.or(draw_terminal_tab(
            app, frame, slot, rect, area, &mut hits, field,
        ));
    }
    app.terminal.rename = rename;
    if caret.is_some() {
        app.ime.cursor = caret;
    }
    // 放不下时被裁掉的一端渐隐，强度随离两端的距离变化，与编辑区标签栏相同。
    let edge = workspace::top_bar::TAB_FADE;
    for (hidden, x, opaque_left) in [
        (scroll, area.x, true),
        (max_scroll - scroll, area.right() - edge, false),
    ] {
        if hidden > 0.0 {
            let rect = Rect::new(x, y, edge, TAB_H);
            frame.with_opacity((hidden / edge).min(1.0), |frame| {
                super::fade(frame, rect, p.editor_bg, opaque_left);
            });
        }
    }

    // 标签已经收窄时「+」固定在最右端：新建标签时其他标签立即收窄、新标签从零宽展开，按当前宽度
    // 计算会使「+」先左移，再随动画右移。
    let end = area.right() + TAB_GAP;
    let new_x = if tabs_narrowed(&names, &widths) {
        end
    } else {
        x.min(end)
    };
    (new_x, hits)
}
