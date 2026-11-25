//! 竖向终端列表：固定的新建入口、间隔排列的条目与悬停操作。拆分的标签展开成一组：一行写着
//! 有几个终端，下面按排列顺序每个终端一行，用树形连线连到上面，可以单独切换、关闭和重命名。

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
    // 各条目的高度：拆分的标签多出它的各个终端那几行。
    let sizes: Vec<f32> = (app.terminal.tabs.iter())
        .map(|tab| group_rows(tab) as f32 * TREE_ROW)
        .collect();
    let offsets: Vec<f32> = std::iter::once(0.0)
        .chain(sizes.iter().scan(0.0, |y, h| {
            *y += h + LIST_GAP;
            Some(*y)
        }))
        .collect();
    let total = (offsets[count] - LIST_GAP).max(0.0);
    // 前台终端换了（切换标签、在组里换终端）时把它那一行滚进视野。
    let current = app.terminal.tab().map(|tab| (tab.id, count, area.h));
    let active = app.terminal.active;
    let focused_row = (app.terminal.tab())
        .filter(|tab| tab.pane_count() > 1)
        .map_or(0, |tab| 1 + tab.pane_at());
    let strip = &mut app.terminal.strip;
    strip.max_scroll = (total - area.h).max(0.0);
    let mut scroll = strip.scroll;
    if strip.revealed != current {
        strip.revealed = current;
        let top = offsets.get(active).copied().unwrap_or(0.0) + focused_row as f32 * TREE_ROW;
        scroll = scroll.min(top).max(top + TREE_ROW - area.h);
    }
    strip.scroll = scroll.clamp(0.0, strip.max_scroll);
    strip.sizes.clone_from(&sizes);
    strip.start = area.y - drawn_scroll(strip.scroll);

    let mut rename = app.terminal.rename.take();
    let dragged = app.dragged_terminal_tab();
    let mut lifted = None;
    for (i, offset) in offsets.iter().take(count).enumerate() {
        let id = app.terminal.tabs[i].id;
        if let Some((_, y)) = dragged.filter(|(dragged, _)| *dragged == id) {
            lifted = Some((i, y));
            continue;
        }
        let y = app.terminal.strip.start + offset;
        draw_entry(app, frame, i, (area, y), clip, rename.as_mut());
    }
    if let Some((i, y)) = lifted {
        let group = Rect::new(area.x, y, area.w, sizes[i]);
        frame.rect_clipped(group, p.editor_bg, RADIUS_ROW, clip);
        draw_entry(app, frame, i, (area, y), clip, rename.as_mut());
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

/// 标签 `tab` 在列表里占几行：拆分的标签是一行组名加上每个终端一行。
fn group_rows(tab: &TermTab) -> usize {
    match tab.pane_count() {
        1 => 1,
        n => n + 1,
    }
}

/// 绘制第 `i` 个标签的条目：顶边在 `y`，宽同 `area`。没有拆分时是一行；拆分时是组名
/// 一行和各个终端的行。
fn draw_entry(
    app: &mut App,
    frame: &mut Frame<'_>,
    i: usize,
    (area, y): (Rect, f32),
    clip: Rect,
    mut rename: Option<&mut (u16, TextField)>,
) {
    let head = Rect::new(area.x, y, area.w, TREE_ROW);
    let panes = app.terminal.tabs[i].pane_count();
    if panes == 1 {
        draw_row(app, frame, i, head, clip, rename);
        return;
    }
    draw_group_head(app, frame, i, head, clip);
    let ids: Vec<u16> = (app.terminal.tabs[i].split_order().iter())
        .map(|pane| pane.id)
        .collect();
    for (at, id) in ids.into_iter().enumerate() {
        let row = Rect::new(area.x, y + (1 + at) as f32 * TREE_ROW, area.w, TREE_ROW);
        let last = at + 1 == panes;
        draw_pane_row(
            app,
            frame,
            (i, at, id, last),
            row,
            clip,
            rename.as_deref_mut(),
        );
    }
}

/// 拆分的标签的组名一行：拆分图标和「N 个终端」；悬停时右端是关闭整个标签的按钮。
fn draw_group_head(app: &mut App, frame: &mut Frame<'_>, i: usize, row: Rect, clip: Rect) {
    let hit = row.intersect(clip);
    app.hits.terminal.push((hit, HeaderHit::Tab(i)));
    if hit.w <= 0.0 || hit.h <= 0.0 {
        return;
    }
    let p = app.palette;
    let active = i == app.terminal.active;
    let hovered = matches!(app.pointer.hover, Hover::TerminalHeader(HeaderHit::Tab(j) | HeaderHit::Close(j)) if i == j);
    if hovered {
        frame.rect_clipped(row, p.hover, RADIUS_ROW, clip);
    }
    let cy = row.y + row.h / 2.0;
    let color = if active { p.text } else { p.text_muted };
    icons::split(frame, (row.x + 14.0, cy), p.text_muted, row.intersect(clip));
    let text = Rect::new(
        row.x + 26.0,
        row.y,
        (row.w - 26.0 - BUTTON_SMALL).max(0.0),
        row.h,
    );
    let count = app.terminal.tabs[i].pane_count();
    let label = t!("terminal-split-group", count = count);
    let label = truncate_end(frame, &label, app.fonts.ui, text.w);
    ui_text(
        frame,
        app,
        text.x,
        text,
        &label,
        color,
        text.intersect(clip),
    );
    if hovered {
        let close = Rect::new(
            row.right() - BUTTON_SMALL,
            row.y + (row.h - BUTTON_SMALL) / 2.0,
            BUTTON_SMALL,
            BUTTON_SMALL,
        );
        let on = app.pointer.hover == Hover::TerminalHeader(HeaderHit::Close(i));
        workspace::top_bar::draw_tab_close(frame, p, close, on, clip);
        // 关闭按钮叠在这一行上：先列出的先命中。
        app.hits
            .terminal
            .insert(0, (close.intersect(clip), HeaderHit::Close(i)));
    }
}

/// 拆分的标签下第 `at` 个终端（编号 `id`，`last` 时是最后一个）的一行：树形连线、序号和名字，
/// 正在用的那个铺选中色；悬停时右端是只关掉它的按钮。重命名的就是这一行时画输入框。
fn draw_pane_row(
    app: &mut App,
    frame: &mut Frame<'_>,
    (i, at, id, last): (usize, usize, u16, bool),
    row: Rect,
    clip: Rect,
    rename: Option<&mut (u16, TextField)>,
) {
    let hit = row.intersect(clip);
    app.hits.terminal.push((hit, HeaderHit::Pane(i, at)));
    if hit.w <= 0.0 || hit.h <= 0.0 {
        return;
    }
    let p = app.palette;
    let tab = &app.terminal.tabs[i];
    let focused = i == app.terminal.active && at == tab.pane_at();
    let Some(pane) = tab.split_order().get(at).copied() else {
        return;
    };
    let hovered = matches!(app.pointer.hover, Hover::TerminalHeader(HeaderHit::Pane(j, k) | HeaderHit::ClosePane(j, k)) if (j, k) == (i, at));
    // 连线：竖线从组名下面接到这一行（不是最后一个时穿过这一行），横线接到序号前。
    let trunk = (row.x + 14.0).round();
    let cy = (row.y + row.h / 2.0).round();
    let bottom = if last { cy } else { row.bottom() };
    let guide = p.indent_guide;
    frame.rect_clipped(
        Rect::new(trunk, row.y, 1.0, bottom - row.y),
        guide,
        0.0,
        clip,
    );
    frame.rect_clipped(Rect::new(trunk, cy, 6.0, 1.0), guide, 0.0, clip);
    let body = Rect::new(row.x + 22.0, row.y, (row.w - 22.0).max(0.0), row.h);
    if focused || hovered {
        let color = if focused { p.selected } else { p.hover };
        frame.rect_clipped(body, color, RADIUS_ROW, clip);
    }
    if let Some((_, field)) = rename.filter(|(rid, _)| *rid == id) {
        let field_rect = body.inset(2.0, 2.0);
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
        return;
    }
    let number = (at + 1).to_string();
    let size = app.fonts.ui_small;
    let number_w = frame.measure(FontKind::Ui, size, &number);
    let x = body.x + 8.0;
    ui_text_small(
        frame,
        app,
        x,
        body,
        &number,
        p.text_dim,
        body.intersect(clip),
    );
    let text = Rect::new(
        x + number_w + 6.0,
        row.y,
        (body.right() - BUTTON_SMALL - x - number_w - 6.0).max(0.0),
        row.h,
    );
    let color = if pane.ended.is_some() {
        p.text_dim
    } else if focused {
        p.text
    } else {
        p.text_muted
    };
    let label = truncate_end(frame, pane.label(), app.fonts.ui, text.w);
    ui_text(
        frame,
        app,
        text.x,
        text,
        &label,
        color,
        text.intersect(clip),
    );
    // 有没看过的输出或程序在运行：名字后面一个小圆点，同没有拆分的条目。
    let dot = match pane.unseen {
        Some(true) => Some(p.error),
        Some(false) => Some(p.accent),
        None => pane.busy().map(|_| p.text_dim),
    };
    if let Some(color) = dot.filter(|_| !hovered) {
        let w = frame.measure(FontKind::Ui, app.fonts.ui, &label);
        let dot = Rect::new(text.x + w + 6.0, cy - 2.0, 4.0, 4.0);
        frame.rect_clipped(dot, color, 2.0, text.intersect(clip));
    }
    if hovered {
        let close = Rect::new(
            row.right() - BUTTON_SMALL,
            row.y + (row.h - BUTTON_SMALL) / 2.0,
            BUTTON_SMALL,
            BUTTON_SMALL,
        );
        let on = app.pointer.hover == Hover::TerminalHeader(HeaderHit::ClosePane(i, at));
        workspace::top_bar::draw_tab_close(frame, p, close, on, clip);
        app.hits
            .terminal
            .insert(0, (close.intersect(clip), HeaderHit::ClosePane(i, at)));
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
