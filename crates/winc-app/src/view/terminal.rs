//! 终端面板的绘制：标签栏、网格（看回滚时显示的是回滚缓冲中的行）、选区和查找的匹配、
//! 光标、输入法组字、查找条；有 shell 集成时还有命令块：色带、时长和退出码、折起的输出、
//! 吸顶的命令行。
//!
//! 网格按终端字体等宽排版（每格宽 `app.terminal_cell`、每行高 `terminal_line_height`，字形缓存
//! 与编辑区共用）：一行先画所有样式段的底色，再画选区和查找的底色，最后画字和下划线，宽字符不会
//! 被后一段的底色盖住；对比度不够时按 `terminal.minimumContrast` 调整文字的颜色，以颜色拼接
//! 图形的字符（制表符、方块元素、Powerline 分隔符）除外。

mod grid;

use grid::*;

use winc_i18n::t;
use winc_pty::protocol::{Attrs, Block, Color as TermColor, Line, Modes, Style, Underline};
use winc_pty::width::cells;
use winc_render::{Color, FontKind, Frame, Rect};

use super::*;
use crate::app::Focus;
use crate::app::MARK_W;
use crate::app::terminal::strip::{TAB_GAP, fit_named_tabs, tabs_narrowed};
use crate::app::terminal::{
    BAND_X, BlockHit, FIND_OPTIONS_MIN, FindPart, HeaderHit, TabEnd, TermTab, ViewRow,
    format_duration, header_height, tab_inset,
};
use crate::theme::{BUTTON, BUTTON_GAP, BUTTON_SMALL, RADIUS_TAB, TOGGLE_W, readable};

/// 标签栏上一个标签的高度（逻辑像素），与编辑区的标签相同。
const TAB_H: f32 = crate::theme::TAB_HEIGHT;
/// 最右端的按钮离面板右边的距离（逻辑像素），与编辑区标签栏相同。
const BUTTONS_END: f32 = 8.0;
/// 成功命令色带的不透明度（0–1）：大多数命令都成功，满色的长条会比失败的红色更显眼。
///
/// 取自调参：对照深色和浅色主题，长输出旁的色带可辨认且不抢正文。鼠标悬停时恢复满色。
const SUCCESS_BAND_ALPHA: f32 = 0.45;
/// 「重新启动」按钮上回车图标占的宽度，含与文字的间距（逻辑像素）。
const RESTART_ICON: f32 = icons::RETURN_KEY_WIDTH + 5.0;
/// 显示命令时长的下限（毫秒）：时长保留一位小数（秒），不足该值时会显示为 0.0 s。
const MIN_SHOWN_DURATION: u32 = 50;

/// 绘制终端面板。
///
/// 面板关着时什么也不绘制。
pub(super) fn draw_terminal(app: &mut App, frame: &mut Frame<'_>) {
    draw_panel(app, frame);
    draw_find(app, frame);
}

/// 绘制标签栏和内容区，查找条之外的部分。
fn draw_panel(app: &mut App, frame: &mut Frame<'_>) {
    let card = app.layout.terminal;
    if card.h <= 0.0 || card.w <= 0.0 {
        return;
    }
    let p = app.palette;
    let focused = app.focus == Focus::Terminal;
    frame.rect(card, p.editor_bg);
    draw_header(app, frame, card);

    let grid = app.terminal_grid();
    if let Some(error) = &app.terminal.error {
        let row = Rect::new(grid.x, grid.y, grid.w, app.fonts.terminal_line_height);
        ui_text(frame, app, grid.x, row, error, p.error, grid);
        return;
    }
    // 滚动条：先取显示状态（同时记下滚动位置），滑块最后绘制，压在吸顶的命令行上面。
    let bar = app.terminal_scrollbar().map(|(track, scroll, content)| {
        let thumb = scrollbar_thumb(track, scroll, content);
        let id = app.terminal.tab().map_or(0, |t| u64::from(t.id));
        let look = app.scrollbar_look(ScrollArea::Terminal, id, scroll, thumb.map(|_| track));
        (thumb, look)
    });
    let Some(tab) = app.terminal.tab() else {
        return;
    };

    let (cell, lh) = (app.terminal_cell, app.fonts.terminal_line_height);
    let (ascent, descent) = (app.terminal_metrics.ascent, app.terminal_metrics.descent);
    let colors = Colors {
        p,
        reverse: tab.modes.has(Modes::REVERSE),
        min: app.terminal_contrast(),
        bright: app.config.terminal_bold_is_bright == Some(true),
    };
    // 整屏反色时默认底色同样反转；标签栏属于面板，不反转。
    if colors.reverse {
        let top = card.y + header_height(card);
        let content = Rect::new(card.x, top, card.w, card.bottom() - top);
        frame.rect(content, colors.default_bg());
    }
    let find = app.terminal.find.as_ref().filter(|f| f.tab == tab.id);
    let jumped = tab
        .jump
        .and_then(|id| tab.blocks.iter().find(|b| b.id == id));
    let mut cursor_y = None;
    // 色带和文字裁到内容区域：滚动时最上面和最下面的行只露出一部分。
    let view = app.terminal_view();
    let band_clip = Rect::new(card.x, view.y, card.w, view.h);
    let (first, origin) = app.terminal_origin();
    let count = tab.grid_rows() as u64 + 2;
    for (r, view_row) in tab.rows_from(first, count).into_iter().enumerate() {
        let y = origin + r as f32 * lh;
        if y >= view.bottom() {
            break;
        }
        let row = Rect::new(grid.x, y, grid.w, lh);
        draw_band(app, frame, tab, view_row, row, band_clip);
        let number = match view_row {
            ViewRow::Line(number) => number,
            ViewRow::Fold(block, hidden) => {
                draw_fold(app, frame, row, (block, hidden), view);
                continue;
            }
        };
        if number == tab.history.1 + u64::from(tab.cursor.row) {
            cursor_y = Some(y);
        }
        let Some(line) = tab.line(number) else {
            continue;
        };
        // ⌘↑ ⌘↓ 跳到的命令行、选区和查找的匹配：绘制在样式的底色之上、字的下面。
        let mut marks = Vec::new();
        if jumped.is_some_and(|b| b.command == number) {
            marks.push((0, usize::from(tab.size.cols), p.current_line));
        }
        if let Some((start, end)) = tab.selected_cols(number) {
            let selection = if focused {
                p.selection
            } else {
                p.selected_inactive
            };
            marks.push((start, end.min(usize::from(tab.size.cols)), selection));
        }
        // 匹配绘制在选区之上：当前匹配同时也是选区，否则无法与其他匹配区分。
        if let Some(find) = find {
            // 匹配按位置升序排列：二分查找与这一行相交的第一处，每帧开销与匹配总数无关。软换行的
            // 匹配可能跨多行。
            let first = find.matches.partition_point(|(_, end)| end.line < number);
            let here = find.matches[first..]
                .iter()
                .take_while(|(start, _)| start.line <= number);
            for (i, &(start, end)) in (first..).zip(here) {
                let color = if find.current == Some(i) {
                    p.search_current
                } else {
                    p.search_match
                };
                let from = if start.line == number { start.col } else { 0 };
                let to = if end.line == number {
                    end.col
                } else {
                    usize::from(tab.size.cols)
                };
                marks.push((from, to, color));
            }
        }
        let baseline = baseline_in(row, ascent, descent);
        draw_line(app, frame, &colors, line, row, baseline, &marks, view);
        // 按住 ⌘ 时鼠标下的链接：强调色的下划线。
        if let Some(link) = app
            .terminal
            .link
            .as_ref()
            .filter(|l| l.tab == tab.id && l.line == number)
        {
            let under = Rect::new(
                grid.x + link.start as f32 * cell,
                baseline + 2.0,
                (link.end - link.start) as f32 * cell,
                1.0,
            );
            frame.rect_clipped(under, p.accent, 0.0, view);
        }
        if let Some(block) = tab.block_at(number).map(|i| &tab.blocks[i])
            && block.command == number
        {
            draw_outcome(app, frame, block, line, row, view);
        }
    }
    if let Some(block) = tab.sticky() {
        draw_sticky(app, frame, tab, &colors, block, card);
    }
    if let Some(find) = find {
        draw_find_marks(frame, app, tab, find, card, view);
    }
    if let Some((Some(thumb), look)) = bar {
        draw_thumb(frame, p, thumb, look);
    }
    if tab.ended.is_some() || app.terminal.crashed {
        // 往上翻着时看不到最后一行，也不写说明；↩ 仍然可以重新启动。
        if let Some(y) = cursor_y {
            let blank = (tab.rows.get(usize::from(tab.cursor.row)))
                .is_none_or(|line| line.trimmed().is_empty());
            let y = (y + if blank { 0.0 } else { lh }).min(view.bottom() - lh);
            let row = Rect::new(grid.x, y, grid.w, lh);
            draw_end_notice(app, frame, row, view);
        }
        return;
    }

    // 往上翻着时光标不在可见范围里。
    let Some(y) = cursor_y else {
        return;
    };
    let cursor = tab.cursor;
    let Some(line) = tab.rows.get(usize::from(cursor.row)) else {
        return;
    };
    let x = grid.x + f32::from(cursor.col) * cell;
    let at = Rect::new(
        x,
        y,
        cell * width_at(line, usize::from(cursor.col)) as f32,
        lh,
    );
    let preedit = app.preedit_at(Focus::Terminal);
    let start = preedit_x(x, preedit, cell, view.right());
    let composing = !preedit.is_empty();
    if focused {
        // 候选窗对齐组字文字的左边；组字文字超出左边界时对齐可见范围。
        app.ime.cursor = Some(Rect::new(start.max(view.x), y, cell, lh));
    }
    if composing {
        let baseline = baseline_in(at, ascent, descent);
        draw_preedit(app, frame, (start, y), baseline, view);
    } else if cursor.visible && tab.settle.is_none() {
        draw_cursor(app, frame, &colors, tab, at, focused, view);
    }
}

/// 命令块的颜色：成功绿、失败红、还在运行灰；还没运行（在敲命令、空回车）的不绘制。
fn band_color(p: &Palette, block: &Block) -> Option<Color> {
    block.output?;
    Some(match block.exit {
        _ if block.running() => p.text_dim,
        Some(0) => p.git_added.alpha(SUCCESS_BAND_ALPHA),
        Some(_) => p.error,
        None => p.text_dim,
    })
}

/// 网格左边这一行的色带。
///
/// 一块的第一行上面、最后一行下面各缩进 2 逻辑像素，相邻的两块不连成一条。
fn draw_band(app: &App, frame: &mut Frame<'_>, tab: &TermTab, row: ViewRow, at: Rect, card: Rect) {
    let index = match row {
        ViewRow::Fold(block, _) => Some(block),
        ViewRow::Line(line) => tab.block_at(line),
    };
    let Some(block) = index.map(|i| &tab.blocks[i]) else {
        return;
    };
    let Some(color) = band_color(app.palette, block) else {
        return;
    };
    let line = row.line(tab);
    let last = match row {
        ViewRow::Fold(..) => true,
        ViewRow::Line(line) => line + 1 == tab.block_end(block),
    };
    let top = if line == block.prompt { 2.0 } else { 0.0 };
    let bottom = if last { 2.0 } else { 0.0 };
    let lit = index.is_some_and(|i| app.hover == Hover::TerminalBlock(BlockHit::Band(i)));
    let (w, color) = if lit {
        (MARK_W + 1.0, color.alpha(1.0))
    } else {
        (MARK_W, color)
    };
    let bar = Rect::new(card.x + BAND_X, at.y + top, w, at.h - top - bottom);
    frame.rect_clipped(bar, color, 0.0, card);
}

/// 折起来的输出：一行「⋯ 1 284 行」，鼠标悬停（点击可展开）时颜色更亮。
///
/// 界面字体的空格很窄，三段分开绘制、中间留固定空隙，避免挤在一起。
fn draw_fold(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    (block, hidden): (usize, u64),
    clip: Rect,
) {
    let p = app.palette;
    let lit = app.hover == Hover::TerminalBlock(BlockHit::Band(block));
    let (fill, color) = if lit {
        (p.selected, p.text)
    } else {
        (p.hover, p.text_muted)
    };
    let count = group_digits(hidden);
    let unit = t!("terminal-folded-lines-unit", count = hidden);
    let parts = ["⋯", count.as_str(), unit.as_str()];
    // 三段之间的空隙（逻辑像素），最后一段后面不留。
    let gaps = [6.0, 3.0, 0.0];
    let widths = parts.map(|part| frame.measure(FontKind::Ui, app.fonts.ui, part));
    let pad = 8.0;
    let w = pad * 2.0 + widths.iter().sum::<f32>() + gaps.iter().sum::<f32>();
    let pill = Rect::new(row.x, row.y + 2.0, w, row.h - 4.0);
    frame.rect_clipped(pill, fill, RADIUS_ROW, clip);
    let mut x = row.x + pad;
    for ((part, width), gap) in parts.iter().zip(widths).zip(gaps) {
        ui_text(frame, app, x, row, part, color, clip);
        x += width + gap;
    }
}

/// 数字按三位一组用空格分开：`1 284`。
fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// 命令行右边淡淡的结果：`✓ 12.4 s`、`✗ 1  0.8 s`。
///
/// 成功又不到一秒的不写；右边放不下（提示符
/// 本身写到了右边）时也不写。
fn draw_outcome(
    app: &App,
    frame: &mut Frame<'_>,
    block: &Block,
    line: &Line,
    row: Rect,
    clip: Rect,
) {
    let p = app.palette;
    let Some(exit) = block.exit.filter(|_| block.output.is_some()) else {
        return;
    };
    let duration = (block.duration)
        .filter(|&ms| ms >= MIN_SHOWN_DURATION)
        .map(format_duration);
    if exit == 0 && block.duration.is_none_or(|d| d < 1000) {
        return;
    }
    let code = (exit != 0).then(|| exit.to_string());
    let measure = |frame: &mut Frame<'_>, s: &str| frame.measure(FontKind::Ui, app.fonts.ui, s);
    let icon = 10.0;
    let code_w = code.as_deref().map_or(0.0, |c| measure(frame, c) + 4.0);
    let time_w = duration.as_deref().map_or(0.0, |d| measure(frame, d) + 8.0);
    let w = icon + code_w + time_w;
    let x = row.right() - w - 4.0;
    let used = cells(line.trimmed()).map(|(_, w)| w).sum::<usize>() as f32 * app.terminal_cell;
    if row.x + used + 2.0 * app.terminal_cell > x {
        return;
    }
    let center = (x + icon / 2.0, row.y + row.h / 2.0);
    let color = if exit == 0 { p.git_added } else { p.error };
    if exit == 0 {
        icons::check(frame, center, 8.0, color, clip);
    } else {
        icons::close(frame, center, 6.0, color, clip);
    }
    let mut tx = x + icon + 4.0;
    if let Some(code) = &code {
        ui_text(frame, app, tx, row, code, color, clip);
        tx += code_w;
    }
    if let Some(duration) = &duration {
        ui_text(frame, app, tx + 4.0, row, duration, p.text_dim, clip);
    }
}

/// 吸顶的命令行：网格第一行在一条命令的输出里时，把那条命令那一行盖在最上面，下面带阴影。
///
/// 那一行还没取回来时写 shell 报的命令行。
fn draw_sticky(
    app: &App,
    frame: &mut Frame<'_>,
    tab: &TermTab,
    colors: &Colors,
    index: usize,
    card: Rect,
) {
    let p = app.palette;
    let grid = app.terminal_grid();
    let lh = app.fonts.terminal_line_height;
    let block = &tab.blocks[index];
    // 紧贴标签栏；下一条命令的提示符上来时文字随底色的下沿一起往上走。
    let band = app.terminal_sticky_band();
    let covered = frame.covered(band, 0.0);
    // 阴影只落在内容上：扩散到标签栏会使标签下方的留白看起来比上方窄。
    let content = Rect::new(card.x, band.y, card.w, card.bottom() - band.y);
    frame.within(content, |frame| {
        frame.shadow(band, 0.0, 6.0, 0.0, p.shadow, Some(covered));
    });
    frame.rect_clipped(band, colors.default_bg(), 0.0, card);
    let row = Rect::new(grid.x, band.bottom() - lh, grid.w, lh);
    let clip = Rect::new(grid.x, band.y, grid.w, band.h);
    if app.hover == Hover::TerminalBlock(BlockHit::Sticky) {
        let hover = Rect::new(grid.x - 4.0, row.y, grid.w + 4.0, lh);
        frame.rect_clipped(hover, p.hover, RADIUS_ROW, band);
    }
    if let Some(color) = band_color(p, block) {
        let bar = Rect::new(card.x + BAND_X, row.y + 2.0, MARK_W, lh - 4.0);
        frame.rect_clipped(bar, color, 0.0, band);
    }
    let baseline = baseline_in(
        row,
        app.terminal_metrics.ascent,
        app.terminal_metrics.descent,
    );
    match tab.sticky_line(index) {
        Some(line) => {
            draw_line(app, frame, colors, line, row, baseline, &[], clip);
            draw_outcome(app, frame, block, line, row, clip);
        }
        None => {
            let text = format!("$ {}", block.text);
            ui_text(frame, app, grid.x, row, &text, p.text_muted, clip);
        }
    }
}

/// 标签栏：各个标签（当前标签凸起，悬停或正在看时露出关闭按钮）、「+」；右边是运行配置的按钮，
/// 最右端是切换面板位置、收起面板两个按钮。
fn draw_header(app: &mut App, frame: &mut Frame<'_>, card: Rect) {
    app.note_terminal_tabs();
    app.note_stop_button();
    let p = app.palette;
    let header = Rect::new(card.x, card.y, card.w, header_height(card));
    let y = card.y + (header.h - TAB_H) / 2.0;
    let left = card.x + tab_inset(card);
    // 面板在右边时标签栏在顶栏那一行，右端让给窗口按钮（由应用绘制标题栏时）。
    let controls = app.layout.controls;
    let end = if controls.w > 0.0 && controls.y < header.bottom() && controls.x < header.right() {
        controls.x - TAB_GAP
    } else {
        header.right()
    };
    // 按钮的尺寸和间距与编辑区标签栏相同；标签放不下时「+」紧贴右边的按钮。
    let button_y = card.y + (header.h - BUTTON) / 2.0;
    let hide = Rect::new(end - BUTTONS_END - BUTTON, button_y, BUTTON, BUTTON);
    let position = Rect::new(hide.x - TAB_GAP - BUTTON, button_y, BUTTON, BUTTON);
    let run = run_buttons(app, frame, left, position.x - TAB_GAP, button_y);
    // 标签和「+」放得下的宽度。
    let rest = run.map_or(position.x, |[menu, ..]| menu.x) - TAB_GAP;
    // 能选的 shell 不止一个时，「+」右边是选 shell 的按钮。
    let shell_menu = app.terminal_shell_list().len() > 1;
    let room = rest - left - TAB_GAP - BUTTON;
    let room = if shell_menu {
        room - TAB_GAP - BUTTON
    } else {
        room
    };
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
        .map(|tab| (tab.id, widths.len()));
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
    let area = Rect::new(left, header.y, room.max(0.0), header.h).intersect(header);
    app.terminal.strip.widths.clone_from(&widths);
    app.terminal.strip.left = left - scroll;
    let dragged = app.dragged_terminal_tab();
    let mut rename = app.terminal.rename.take();
    let mut caret = None;
    let strip = &app.terminal.strip;
    let mut hits = Vec::new();
    let mut x = left - scroll;
    let mut lifted = None;
    // 正在看的、悬停的、拖着的标签有自己的底色，两边不绘制竖线。
    let hovered = match app.hover {
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
            let rect = Rect::new(x, y, ghost.width, TAB_H);
            let clip = rect.intersect(area);
            frame.with_opacity(ghost.fade.fade_out(), |frame| {
                let color = if ghost.active { p.text } else { p.text_muted };
                let state = (ghost.active, false);
                draw_tab(app, frame, rect, clip, &ghost.label, state, color);
            });
            x += (ghost.width + TAB_GAP) * (1.0 - t);
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
    let new = Rect::new(new_x, button_y, BUTTON, BUTTON);
    if app.hover == Hover::TerminalHeader(HeaderHit::New) {
        frame.rect_clipped(new, p.hover, RADIUS_ROW, header);
    }
    icons::plus(
        frame,
        (new.x + BUTTON / 2.0, new.y + BUTTON / 2.0),
        9.0,
        p.text_muted,
        header,
    );
    hits.push((new, HeaderHit::New));
    if shell_menu {
        use crate::app::MenuButton;
        let menu = Rect::new(new.right() + TAB_GAP, button_y, BUTTON, BUTTON);
        let on = app.hover == Hover::TerminalHeader(HeaderHit::ShellMenu);
        let open = app.menu_button == Some(MenuButton::TerminalShells);
        draw_button(frame, p, menu, header, on, open);
        let direction = icons::Direction::Down;
        icons::chevron(frame, center(menu), 8.0, direction, p.text_muted, header);
        hits.push((menu, HeaderHit::ShellMenu));
    }

    // 右端：切换面板的位置（编辑区右边或下面），图标绘制的是点下去以后的位置。
    if app.hover == Hover::TerminalHeader(HeaderHit::Position) {
        frame.rect_clipped(position, p.hover, RADIUS_ROW, header);
    }
    let center = (position.x + BUTTON / 2.0, position.y + BUTTON / 2.0);
    icons::panel(frame, center, !app.terminal_right(), p.text_muted, header);
    hits.push((position, HeaderHit::Position));

    // 最右端：收起面板，箭头朝面板收进去的那一边。
    if app.hover == Hover::TerminalHeader(HeaderHit::Hide) {
        frame.rect_clipped(hide, p.hover, RADIUS_ROW, header);
    }
    let toward = if app.terminal_right() {
        icons::Direction::Right
    } else {
        icons::Direction::Down
    };
    let center = (hide.x + BUTTON / 2.0, hide.y + BUTTON / 2.0);
    icons::collapse(frame, center, toward, p.text_muted, header);
    hits.push((hide, HeaderHit::Hide));
    if let Some(run) = run {
        draw_run_buttons(app, frame, run, header, &mut hits);
    }
    app.hits.terminal = hits;
}

/// 配置名按钮最宽多少（逻辑像素），约容纳 8 个汉字；更长的名字截短。
const RUN_NAME_MAX: f32 = 104.0;

/// 配置名按钮中文字之外的宽度（逻辑像素）：左边距 8、下拉箭头及其边距 20。
pub(super) const RUN_NAME_CHROME: f32 = 28.0;

/// 留给标签和「+」的最小宽度（逻辑像素），约容纳两个标签；放不下配置名时只保留下拉箭头。
const RUN_TABS_MIN: f32 = 240.0;

/// 返回配置名按钮的宽度：`name` 是名字的宽度，`room` 是标签栏可用的总宽度。
///
/// 面板窄时只保留下拉箭头，配置名可在菜单中看到；再窄时返回 `None`，用命令面板运行。
pub(super) fn run_menu_width(name: f32, room: f32) -> Option<f32> {
    let buttons = (BUTTON + TAB_GAP) * 2.0;
    // 向上取整：绘制时从按钮宽度减回文字宽度，浮点误差不能让放得下的名字被截短。
    let full = name.min(RUN_NAME_MAX).ceil() + RUN_NAME_CHROME;
    [full, BUTTON]
        .into_iter()
        .find(|menu| room - menu - buttons >= RUN_TABS_MIN)
}

/// 运行配置的按钮在标签栏上的位置（配置名，运行，停止）。
///
/// 还没有配置时只有运行按钮，点击后生成配置文件；没有打开目录或太窄时没有按钮。停止按钮只在运行时
/// 出现，其余时候运行按钮贴着右边。
fn run_buttons(app: &App, frame: &mut Frame<'_>, left: f32, end: f32, y: f32) -> Option<[Rect; 3]> {
    app.editor.root()?;
    let last = Rect::new(end - BUTTON, y, BUTTON, BUTTON);
    let Some(config) = app.run.current() else {
        return (end - left - BUTTON >= RUN_TABS_MIN).then_some([last; 3]);
    };
    // 停止按钮出现、消失时，运行按钮、配置名和标签跟着滑动。
    let share = app.terminal.strip.stop_share(app.run_config_running());
    let start = Rect::new(last.x - share * (TAB_GAP + BUTTON), y, BUTTON, BUTTON);
    let name = frame.measure(FontKind::Ui, app.fonts.ui, &config.name);
    // `run_menu_width` 按运行、停止两个按钮算：没有停止按钮时多出一个的位置。
    let spare = (1.0 - share) * (BUTTON + TAB_GAP);
    let menu = run_menu_width(name, end - left + spare)?;
    let menu = Rect::new(start.x - TAB_GAP - menu, y, menu, BUTTON);
    Some([menu, start, last])
}

/// 绘制运行配置的按钮：配置名和下拉箭头，运行（运行中是重新运行），停止（运行中才有）。
fn draw_run_buttons(
    app: &App,
    frame: &mut Frame<'_>,
    [menu, start, stop]: [Rect; 3],
    header: Rect,
    hits: &mut Vec<(Rect, HeaderHit)>,
) {
    use crate::app::MenuButton;
    use crate::app::terminal::run::RunButton;
    let p = app.palette;
    let hovered = |button| app.hover == Hover::TerminalHeader(HeaderHit::Run(button));
    let center = |rect: Rect| (rect.x + BUTTON / 2.0, rect.y + BUTTON / 2.0);
    let Some(config) = app.run.current() else {
        let on = hovered(RunButton::Start);
        draw_button(frame, p, start, header, on, false);
        let color = if on { p.text } else { p.text_muted };
        icons::play(frame, center(start), color, header);
        hits.push((start, HeaderHit::Run(RunButton::Start)));
        return;
    };
    let running = app.run_config_running();
    let open = app.menu_button == Some(MenuButton::RunConfigs);

    draw_button(frame, p, menu, header, hovered(RunButton::Menu), open);
    let color = if open || hovered(RunButton::Menu) {
        p.text
    } else {
        p.text_muted
    };
    let arrow = if menu.w > BUTTON {
        let room = menu.w - RUN_NAME_CHROME;
        let name = ellipsize(frame, &config.name, app.fonts.ui, room);
        ui_text(frame, app, menu.x + 8.0, menu, &name, color, menu);
        (menu.right() - 12.0, menu.y + menu.h / 2.0)
    } else {
        center(menu)
    };
    icons::chevron(frame, arrow, 8.0, icons::Direction::Down, color, menu);
    hits.push((menu, HeaderHit::Run(RunButton::Menu)));

    draw_button(frame, p, start, header, hovered(RunButton::Start), false);
    if running {
        icons::refresh(frame, center(start), p.git_added, header);
    } else {
        icons::play(frame, center(start), p.git_added, header);
    }
    hits.push((start, HeaderHit::Run(RunButton::Start)));

    // 淡出期间不再响应点击。
    let share = app.terminal.strip.stop_share(running);
    frame.with_opacity(share, |frame| {
        let on = running && hovered(RunButton::Stop);
        draw_button(frame, p, stop, header, on, false);
        icons::stop(frame, center(stop), p.error, header);
    });
    if running {
        hits.push((stop, HeaderHit::Run(RunButton::Stop)));
    }
}

/// 第 `i` 个终端标签：底色、名字、未查看的命令结束小点、关闭按钮，记录各处的命中区域。
///
/// 正在改这个标签的名字（`rename`）时名字换成输入框，返回输入框的光标。
fn draw_terminal_tab(
    app: &App,
    frame: &mut Frame<'_>,
    i: usize,
    rect: Rect,
    clip: Rect,
    hits: &mut Vec<(Rect, HeaderHit)>,
    rename: Option<&mut (u16, TextField)>,
) -> Option<Rect> {
    let p = app.palette;
    let tab = &app.terminal.tabs[i];
    if let Some((_, field)) = rename.filter(|(id, _)| *id == tab.id) {
        workspace::top_bar::draw_tab_fill(frame, p, rect, true, false, clip);
        let focused = app.focus == Focus::TerminalRename;
        let field_rect = rect.inset(3.0, 3.0);
        field_box(app, frame, field_rect, p.tab_fill, focused, false);
        let area = field_rect.inset(6.0, 0.0);
        hits.push((rect.intersect(clip), HeaderHit::Tab(i)));
        return draw_bar_field(app, frame, area, field, "", focused).filter(|_| focused);
    }
    let active = i == app.terminal.active;
    let hovered = matches!(app.hover, Hover::TerminalHeader(HeaderHit::Tab(j) | HeaderHit::Close(j)) if j == i);
    // 字色和编辑区的标签一样：正在看的用正文色，不管焦点在不在面板上。
    let color = match (active, tab.ended.is_some()) {
        (_, true) => p.text_dim,
        (true, _) => p.text,
        _ => p.text_muted,
    };
    let state = (active, hovered);
    draw_tab(app, frame, rect, clip, tab.label(), state, color);
    // 左上角的圆点：未查看的命令结束时为强调色，命令失败时为红色；否则标签内有程序运行时为灰色。
    let dot = match tab.unseen {
        Some(true) => Some(p.error),
        Some(false) => Some(p.accent),
        None => tab.busy().map(|_| p.text_dim),
    };
    if let Some(dot) = dot {
        let at = Rect::new(rect.x + 3.0, rect.y + 3.0, 5.0, 5.0);
        frame.rect_clipped(at, dot, 2.5, clip);
    }
    let close = workspace::top_bar::tab_close(rect);
    if active || hovered {
        let on = app.hover == Hover::TerminalHeader(HeaderHit::Close(i));
        workspace::top_bar::draw_tab_close(frame, p, close, on, clip);
    }
    // 移出标签栏、被裁掉的部分无法点击。
    hits.push((close.intersect(clip), HeaderHit::Close(i)));
    hits.push((rect.intersect(clip), HeaderHit::Tab(i)));
    None
}

/// 一个标签的底（与编辑区的标签相同）和名字，名字放不下时省略。
///
/// 参数 `(active, hovered)` 为真时分别是正在看的标签、鼠标悬停的标签。
fn draw_tab(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    clip: Rect,
    label: &str,
    (active, hovered): (bool, bool),
    color: Color,
) {
    workspace::top_bar::draw_tab_fill(frame, app.palette, rect, active, hovered, clip);
    let text = Rect::new(rect.x + 10.0, rect.y, rect.w - 32.0, TAB_H);
    // 路径的两端都有用；命令和进程名从头读，开头足以辨认。
    let label = if label.contains(['/', '\\']) {
        ellipsize(frame, label, app.fonts.ui, text.w)
    } else {
        truncate_end(frame, label, app.fonts.ui, text.w)
    };
    let clip = text.intersect(clip);
    ui_text(frame, app, text.x, text, &label, color, clip);
}

/// 终端结束时的说明和颜色：shell 退出了、伪终端断开了、或者辅助进程退出了；还能用时为 `None`。
fn end_note(app: &App) -> Option<(String, Color)> {
    let p = app.palette;
    if app.terminal.crashed {
        return Some((t!("terminal-helper-exited").to_string(), p.error));
    }
    let tab = app.terminal.tab()?;
    Some(match tab.ended? {
        TabEnd::Exited(_) if tab.run.is_some() && tab.stopped => {
            (t!("terminal-run-stopped").to_string(), p.text_dim)
        }
        TabEnd::Exited(0) if tab.run.is_some() => {
            (t!("terminal-run-finished").to_string(), p.text_dim)
        }
        TabEnd::Exited(code) => (t!("terminal-process-exited", code = code), p.text_dim),
        TabEnd::Disconnected => (t!("terminal-disconnected").to_string(), p.error),
    })
}

/// 终端结束后写在最后一行下面的一行：说明和「重新启动」按钮（↩ 也行）。
fn draw_end_notice(app: &mut App, frame: &mut Frame<'_>, row: Rect, clip: Rect) {
    let p = app.palette;
    let Some((note, color)) = end_note(app) else {
        return;
    };
    // 屏幕写满时写在最后一行上，盖住那一行。
    frame.rect_clipped(row, p.editor_bg, 0.0, clip);
    ui_text(frame, app, row.x, row, &note, color, clip);
    // 运行配置的标签再次运行的是配置，不是打开 shell。
    let label = match app.terminal.tab() {
        Some(tab) if tab.run.is_some() && !app.terminal.crashed => t!("terminal-run-again"),
        _ => t!("terminal-restart"),
    };
    let note_w = frame.measure(FontKind::Ui, app.fonts.ui, &note);
    let label_w = frame.measure(FontKind::Ui, app.fonts.ui, label);
    let button = Rect::new(
        row.x + note_w + 10.0,
        row.y,
        label_w + RESTART_ICON + 12.0,
        row.h,
    );
    let hovered = app.hover == Hover::TerminalHeader(HeaderHit::Restart);
    let fill = if hovered { p.hover } else { p.tab_fill };
    frame.rect_clipped(button, fill, RADIUS_ROW, clip);
    let icon = (
        button.x + 6.0 + icons::RETURN_KEY_WIDTH / 2.0,
        button.y + button.h / 2.0,
    );
    icons::return_key(frame, icon, p.text, clip);
    let text = button.x + 6.0 + RESTART_ICON;
    ui_text(frame, app, text, row, label, p.text, clip);
    app.hits.terminal.push((button, HeaderHit::Restart));
}

/// 在滚动条轨道上标出匹配的位置，当前匹配用强调色。匹配超过上限时等间隔取样。
fn draw_find_marks(
    frame: &mut Frame<'_>,
    app: &App,
    tab: &TermTab,
    find: &crate::app::terminal::TermFind,
    card: Rect,
    view: Rect,
) {
    /// 绘制的标记数量上限（个）：轨道高度的物理像素数远小于该值，超出的标记会落在相同像素上。
    const MAX_MARKS: usize = 2000;
    /// 标记的宽和高（逻辑像素）：宽度小于滚动条滑块，高度保证 1× 屏幕上至少 2 个物理像素。
    const MARK_SIZE: (f32, f32) = (4.0, 2.0);
    let p = app.palette;
    let lh = app.fonts.terminal_line_height;
    let content = (tab.display_len() as f32 * lh).max(view.h).max(1.0);
    let x = card.right() - 2.0 - MARK_SIZE.0;
    let mark = |frame: &mut Frame<'_>, at: usize, color: Color| {
        let display = tab.display_of(find.matches[at].0.line) as f32;
        let y = view.y + display * lh / content * view.h;
        let rect = Rect::new(x, y, MARK_SIZE.0, MARK_SIZE.1);
        frame.rect_clipped(rect, color, 0.0, view_clip(card, view));
    };
    let step = (find.matches.len() / MAX_MARKS).max(1);
    for at in (0..find.matches.len()).step_by(step) {
        mark(frame, at, p.search_current);
    }
    if let Some(current) = find.current.filter(|&i| i < find.matches.len()) {
        mark(frame, current, p.accent);
    }
}

/// 标记的裁剪范围：纵向为内容区，横向为面板，标记位于内容区右侧的留白内。
fn view_clip(card: Rect, view: Rect) -> Rect {
    Rect::new(card.x, view.y, card.w, view.h)
}

/// 查找条：浮在内容区右上角，与编辑区的查找条样式相同。
///
/// 从左到右是输入框、第几处和共几处、三个选项、上一处、下一处、关闭；没有匹配时上一处、下一处置灰。
fn draw_find(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let Some(bar) = app.terminal_find_bar() else {
        return;
    };
    let Some(mut find) = app.terminal.find.take() else {
        return;
    };
    let focused = app.focus == Focus::TerminalFind;
    // 只绘制细边框：查找条紧邻标签栏，阴影会投到标签栏上。
    let covered = frame.covered(bar, RADIUS_TAB);
    let edge = bar.inset(-1.0, -1.0);
    frame.rect_except(edge, p.popup_border, RADIUS_TAB + 1.0, covered);
    frame.rounded(bar, p.popup_bg, RADIUS_TAB);
    let inset = (bar.h - BUTTON_SMALL) / 2.0;
    let button = |i: f32| {
        let x = bar.right() - inset - BUTTON_SMALL - i * (BUTTON_SMALL + BUTTON_GAP);
        Rect::new(x, bar.y + inset, BUTTON_SMALL, BUTTON_SMALL)
    };
    let (close, next, previous) = (button(0.0), button(1.0), button(2.0));
    let hovered = |part| app.hover == Hover::TerminalHeader(HeaderHit::Find(part));
    let can_step = !find.matches.is_empty();
    let step = if can_step { p.text_muted } else { p.text_dim };
    for (rect, part) in [(previous, FindPart::Previous), (next, FindPart::Next)] {
        draw_button(frame, p, rect, bar, can_step && hovered(part), false);
    }
    draw_button(frame, p, close, bar, hovered(FindPart::Close), false);
    icons::chevron(
        frame,
        center(previous),
        8.0,
        icons::Direction::Up,
        step,
        bar,
    );
    icons::chevron(frame, center(next), 8.0, icons::Direction::Down, step, bar);
    icons::close(frame, center(close), 7.0, p.text_muted, bar);

    // 选项位于按钮左侧，查找条窄于 `FIND_OPTIONS_MIN` 时不显示。
    let option = |i: f32| {
        let x = previous.x - 6.0 - (3.0 - i) * (TOGGLE_W + BUTTON_GAP) + BUTTON_GAP;
        Rect::new(x, previous.y, TOGGLE_W, BUTTON_SMALL)
    };
    let options = [
        (option(0.0), find.options.case_sensitive, FindPart::Case),
        (option(1.0), find.options.whole_word, FindPart::Word),
        (option(2.0), find.options.regex, FindPart::Regex),
    ];
    let show_options = bar.w >= FIND_OPTIONS_MIN;
    if show_options {
        draw_search_options(
            app,
            frame,
            options.map(|(rect, on, part)| (rect, on, hovered(part))),
            bar,
        );
    }
    let count = match (
        find.field.text().is_empty(),
        find.matches.len(),
        find.current,
    ) {
        (true, ..) => String::new(),
        _ if find.invalid => t!("workspace-find-regex-invalid").into(),
        (false, 0, _) if !find.loaded => t!("terminal-find-searching").into(),
        (false, 0, _) => t!("terminal-find-no-results").into(),
        (false, n, Some(i)) => format!("{}/{n}", i + 1),
        (false, n, None) => format!("{n}"),
    };
    let failed = find.invalid || (!count.is_empty() && find.loaded && !can_step);
    let count_color = if failed { p.error } else { p.text_dim };
    // 计数靠选项（没有选项时是按钮）左边，输入框用剩下的宽度。
    let right = if show_options {
        options[0].0.x - 6.0
    } else {
        previous.x - 6.0
    };
    let count_x = right - frame.measure(FontKind::Ui, app.fonts.ui_small, &count);
    ui_text_small(frame, app, count_x, bar, &count, count_color, bar);
    let text_right = if count.is_empty() {
        right
    } else {
        count_x - 10.0
    };
    let left = bar.x + 12.0;
    let field = Rect::new(left, bar.y, (text_right - left).max(0.0), bar.h);
    let caret = draw_bar_field(
        app,
        frame,
        field,
        &mut find.field,
        t!("terminal-find-placeholder"),
        focused,
    );
    if focused && caret.is_some() {
        app.ime.cursor = caret;
    }
    app.terminal.find = Some(find);
    // 排在命中列表最前：查找条覆盖下方内容，优先响应点击。
    let shown = options.into_iter().filter(|_| show_options);
    let hits = shown
        .map(|(rect, _, part)| (rect, HeaderHit::Find(part)))
        .chain([
            (previous, HeaderHit::Find(FindPart::Previous)),
            (next, HeaderHit::Find(FindPart::Next)),
            (close, HeaderHit::Find(FindPart::Close)),
            (bar, HeaderHit::Find(FindPart::Bar)),
        ]);
    let hits: Vec<_> = hits.collect();
    app.hits.terminal.splice(0..0, hits);
}

#[cfg(test)]
mod tests;
