//! 竖向终端列表：固定的新建入口、间隔排列的条目与悬停操作。

use super::*;
use crate::app::MenuButton;
use crate::app::terminal::strip::LIST_GAP;
use crate::theme::TREE_ROW;

pub(super) fn draw_list(app: &mut App, frame: &mut Frame<'_>) {
    let Some(list) = app.terminal_tab_list() else {
        return;
    };
    let p = app.palette;
    let clip = list.intersect(app.layout.terminal);
    frame.rect_clipped(
        Rect::new(list.x, list.y, 1.0, list.h),
        p.separator,
        0.0,
        clip,
    );
    draw_new(app, frame, list, clip);
    let Some(rows) = app.terminal_tab_rows() else {
        return;
    };
    let area = rows.inset(TAB_GAP, 0.0);
    let clip = area.intersect(clip);
    if area.w <= 0.0 || area.h <= 0.0 {
        return;
    }
    let count = app.terminal.tabs.len();
    let step = TREE_ROW + LIST_GAP;
    let current = app.terminal.tab().map(|tab| (tab.id, count, area.h));
    let strip = &mut app.terminal.strip;
    strip.max_scroll = (count as f32 * step - LIST_GAP - area.h).max(0.0);
    let mut scroll = strip.scroll;
    if strip.revealed != current {
        strip.revealed = current;
        let top = app.terminal.active as f32 * step;
        scroll = scroll.min(top).max(top + TREE_ROW - area.h);
    }
    strip.scroll = scroll.clamp(0.0, strip.max_scroll);
    strip.sizes.clear();
    strip.sizes.resize(count, TREE_ROW);
    strip.start = area.y - strip.scroll;

    let mut rename = app.terminal.rename.take();
    let dragged = app.dragged_terminal_tab();
    let mut lifted = None;
    for i in 0..count {
        let id = app.terminal.tabs[i].id;
        if let Some((_, y)) = dragged.filter(|(dragged, _)| *dragged == id) {
            lifted = Some((i, y));
            continue;
        }
        let y = app.terminal.strip.start + i as f32 * step;
        let row = Rect::new(area.x, y, area.w, TREE_ROW);
        draw_row(app, frame, i, row, clip, rename.as_mut());
    }
    if let Some((i, y)) = lifted {
        let row = Rect::new(area.x, y, area.w, TREE_ROW);
        frame.rect_clipped(row, p.editor_bg, RADIUS_ROW, clip);
        draw_row(app, frame, i, row, clip, rename.as_mut());
    }
    app.terminal.rename = rename;
}

fn draw_new(app: &mut App, frame: &mut Frame<'_>, list: Rect, clip: Rect) {
    let p = app.palette;
    let row = Rect::new(
        list.x + TAB_GAP,
        list.y + TAB_GAP,
        (list.w - TAB_GAP * 2.0).max(0.0),
        TREE_ROW,
    );
    let shell_menu = app.terminal_shell_list().len() > 1;
    let group = row;
    let menu = shell_menu.then(|| {
        let width = BUTTON.min(group.w);
        Rect::new(group.right() - width, group.y, width, group.h)
    });
    let new = Rect::new(
        group.x,
        group.y,
        menu.map_or(group.w, |menu| menu.x - group.x),
        group.h,
    );
    let hovered = matches!(
        app.pointer.hover,
        Hover::TerminalHeader(HeaderHit::New | HeaderHit::ShellMenu)
    );
    let open = app.overlays.menu_button == Some(MenuButton::TerminalShells);
    if hovered || open {
        frame.rect_clipped(group, p.hover, RADIUS_ROW, clip);
    }
    icons::plus(
        frame,
        (new.x + BUTTON_SMALL / 2.0, row.y + row.h / 2.0),
        8.0,
        p.text_muted,
        new.intersect(clip),
    );
    let text = Rect::new(
        new.x + BUTTON_SMALL,
        row.y,
        (new.w - BUTTON_SMALL - TAB_GAP).max(0.0),
        row.h,
    );
    let label = truncate_end(frame, t!("terminal-new"), app.fonts.ui, text.w);
    ui_text(
        frame,
        app,
        text.x,
        text,
        &label,
        p.text_muted,
        text.intersect(clip),
    );
    app.hits
        .terminal
        .push((new.intersect(clip), HeaderHit::New));
    if let Some(menu) = menu {
        if hovered || open {
            frame.rect_clipped(
                Rect::new(menu.x, menu.y + TAB_GAP, 1.0, menu.h - TAB_GAP * 2.0),
                p.separator,
                0.0,
                clip,
            );
        }
        icons::chevron(
            frame,
            center(menu),
            8.0,
            icons::Direction::Down,
            p.text_muted,
            clip,
        );
        app.hits
            .terminal
            .push((menu.intersect(clip), HeaderHit::ShellMenu));
    }
}

fn draw_row(
    app: &mut App,
    frame: &mut Frame<'_>,
    i: usize,
    row: Rect,
    clip: Rect,
    rename: Option<&mut (u16, TextField)>,
) {
    let hit = row.intersect(clip);
    if hit.w <= 0.0 || hit.h <= 0.0 {
        app.hits.terminal.push((hit, HeaderHit::Tab(i)));
        return;
    }
    let p = app.palette;
    let tab = &app.terminal.tabs[i];
    let active = i == app.terminal.active;
    let hovered = matches!(app.pointer.hover, Hover::TerminalHeader(HeaderHit::Tab(j) | HeaderHit::Close(j)) if i == j);
    if active || hovered {
        frame.rect_clipped(
            row,
            if active { p.selected } else { p.hover },
            RADIUS_ROW,
            clip,
        );
    }
    let text = Rect::new(
        row.x + 12.0,
        row.y,
        (row.w - 12.0 - BUTTON_SMALL).max(0.0),
        row.h,
    );
    if let Some((_, field)) = rename.filter(|(id, _)| *id == tab.id) {
        let field_rect = row.inset(6.0, 2.0);
        let focused = app.focus == Focus::TerminalRename;
        let caret = frame.within(clip, |frame| {
            field_box(app, frame, field_rect, p.editor_bg, focused, false);
            draw_bar_field(app, frame, field_rect.inset(4.0, 0.0), field, "", focused)
        });
        if let Some(caret) = caret
            .map(|caret| caret.intersect(clip))
            .filter(|caret| caret.w > 0.0 && caret.h > 0.0)
        {
            app.ime.cursor = Some(caret);
        }
    } else {
        let color = if tab.ended.is_some() {
            p.text_dim
        } else if active {
            p.text
        } else {
            p.text_muted
        };
        let label = truncate_end(frame, tab.label(), app.fonts.ui, text.w);
        ui_text(
            frame,
            app,
            text.x,
            text,
            &label,
            color,
            text.intersect(clip),
        );
        let dot = match tab.unseen {
            Some(true) => Some(p.error),
            Some(false) => Some(p.accent),
            None => tab.busy().map(|_| p.text_dim),
        };
        if let Some(color) = dot {
            frame.rect_clipped(
                Rect::new(row.x + 5.0, row.y + row.h / 2.0 - 2.0, 4.0, 4.0),
                color,
                2.0,
                clip,
            );
        }
        if hovered {
            let close = Rect::new(
                row.right() - BUTTON_SMALL,
                row.y + (row.h - BUTTON_SMALL) / 2.0,
                BUTTON_SMALL,
                BUTTON_SMALL,
            );
            let on = app.pointer.hover == Hover::TerminalHeader(HeaderHit::Close(i));
            workspace::top_bar::draw_tab_close(frame, p, close, on, clip);
            app.hits
                .terminal
                .push((close.intersect(clip), HeaderHit::Close(i)));
        }
    }
    app.hits.terminal.push((hit, HeaderHit::Tab(i)));
}
