//! 侧栏顶部的「文件」「搜索」视图切换按钮，以及全局搜索面板的绘制。

use std::f32::consts::FRAC_PI_2;

use winc_i18n::t;
use winc_render::{Frame, Rect};

use super::*;
use crate::app::{Focus, OutlineTarget, ResultRow, SyncState};
use crate::layout::{
    PANEL_CHEVRON, PANEL_LEFT, PANEL_TEXT, PanelButton, PanelParts, PanelTool, RowAction,
    SearchButton, SidebarView, row_actions,
};

/// 视图切换里的图标比原大小略小（约占 13 逻辑像素，放在 22 逻辑像素的一段里四周留出 4.5）。
const VIEW_ICON: f32 = 0.93;

/// 结果行上悬停按钮左侧渐隐区域的宽度（逻辑像素，经验值）。
///
/// 其下的文本在该区域内逐渐淡出，而不是被直接截断。
const COVER_FADE: f32 = 20.0;

/// 绘制顶栏里侧栏开关右边的视图切换（文件、搜索、提交）。
///
/// 各段放在一条浅灰的底槽里，当前视图那一段凸起来：浅色配色里是白底、一圈细边，深色配色里比
/// 底槽亮一档；图标都是线条风格、同一个颜色，选中靠凸起的底分辨。悬停的那一段铺浅色，提示写
/// 名字和快捷键。有未提交的改动时「提交」那一段右上角是一个强调色的小圆点。
/// 侧栏收起、动画途中超出侧栏的部分不绘制。
pub(super) fn draw_view_tabs(app: &App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let clip = app.layout.sidebar;
    let tabs = app.sidebar_tabs();
    let (Some(first), Some(last)) = (tabs.first(), tabs.last()) else {
        return;
    };
    let track = Rect::new(
        first.0.x - 2.0,
        first.0.y - 2.0,
        last.0.right() - first.0.x + 4.0,
        first.0.h + 4.0,
    );
    frame.rect_clipped(track, p.hover, RADIUS_TAB, clip);
    let track_bg = over(p.sidebar_bg, p.hover);
    let (raised, edge) = segment_colors(p, track_bg);

    // 凸起的那一段：切换视图时从上一个视图的位置滑过来（`Sidebar::view_slide`），先绘制，图标压在上面。
    let at = |view: SidebarView| tabs.iter().find(|t| t.1 == view).map(|t| t.0);
    if let Some(to) = at(app.panels.sidebar.view) {
        let slot = match app
            .panels
            .sidebar
            .view_slide
            .and_then(|(from, clock)| Some((at(from)?, clock)))
        {
            Some((from, clock)) => Rect::new(clock.between(from.x, to.x), to.y, to.w, to.h),
            None => to,
        };
        frame.rect_clipped(slot, edge, RADIUS_ROW, clip);
        frame.rect_clipped(slot.inset(0.5, 0.5), raised, RADIUS_ROW - 0.5, clip);
    }

    for &(rect, view) in &tabs {
        let on = app.panels.sidebar.view == view;
        let hovered = app.pointer.hover == Hover::ViewTab(view);
        // 图标外圈（改动小圆点的隔圈）用这一段的底色。
        let base = if on {
            raised
        } else if hovered {
            frame.rect_clipped(rect, p.hover, RADIUS_ROW, clip);
            over(track_bg, p.hover)
        } else {
            track_bg
        };
        // 选中的一段靠凸起的底分辨，图标与其他段一样用淡一档的字色；悬停时才加深。若选中段
        // 用正文色，选中的图标会明显重于相邻的几段。
        let color = if hovered { p.text } else { p.text_muted };
        // 模块的视图出现、消失途中（`App::view_tab_share`）这一段按比例变窄，图标跟着淡入淡出、
        // 从小到大，只绘制在这一段里，不压到旁边。
        let share = app.view_tab_share(view);
        let (color, scale, clip) = if share < 1.0 {
            let opacity = app.view_tab_opacity(view);
            let color = color.alpha(color.a * opacity);
            (color, VIEW_ICON * (0.6 + 0.4 * share), rect.intersect(clip))
        } else {
            (color, VIEW_ICON, clip)
        };
        match view {
            SidebarView::Files => icons::files(frame, center(rect), scale, color, clip),
            SidebarView::Search => icons::search(frame, center(rect), scale, color, clip),
            SidebarView::Commit => icons::git(frame, center(rect), scale, color, clip),
        }
        if view == SidebarView::Commit && share >= 1.0 && app.commit_count_text().is_some() {
            draw_change_dot(frame, p, rect, base, clip);
        }
    }
}

/// 分段控件里凸起那一段的底色和细边：浅色配色是白底、淡黑边；深色配色是比底槽 `track` 亮一档的
/// 灰、淡白边。
///
/// 都是不透明色，叠在底槽上不会透出下面。
pub(in crate::view) fn segment_colors(p: &Palette, track: Color) -> (Color, Color) {
    let white = Color::hex(0xFFFFFF);
    if p.dark {
        let raised = over(track, white.alpha(0.09));
        (raised, over(track, white.alpha(0.14)))
    } else {
        (white, over(track, Color::hex(0x000000).alpha(0.08)))
    }
}

/// 在图标按钮 `button` 的右上角、图标外面绘制一个强调色的小圆点，表示有未提交的改动；外面
/// 一圈按钮底色 `ring`，与按钮底色隔开。
///
/// 具体几个改动写在悬停提示和列表的组标题里。
///
/// 圆点放在图标（中心 ±7 逻辑像素）与按钮边（±14 逻辑像素）之间：压在图标上会盖住 Git 图标
/// 右上的提交标记。
fn draw_change_dot(frame: &mut Frame<'_>, p: &Palette, button: Rect, ring: Color, clip: Rect) {
    let (cx, cy) = center(button);
    let r = 2.5;
    // 段是横着的长方形：圆点放在图标右上、段里面，不伸到底槽外。
    let (x, y) = (cx + 11.5, cy - 6.5);
    let dot = Rect::new(x - r, y - r, 2.0 * r, 2.0 * r);
    frame.rect_clipped(dot.inset(-1.5, -1.5), ring, r + 1.5, clip);
    frame.rect_clipped(dot, p.accent, r, clip);
}

/// 绘制全局搜索面板，返回有焦点的输入框的光标矩形（供输入法候选框定位）。
pub(super) fn draw_panel(app: &mut App, frame: &mut Frame<'_>) -> Option<Rect> {
    app.panels.global.sync_rows();
    // 查找、替换输入框随内容长高：先按框的宽度折好行（宽度和高度无关），框的高度按行数定，
    // 绘制时继续使用折好的行。
    let wraps = {
        let parts = app.panel_parts();
        let width = PanelParts::text_area(parts.query, parts.case_toggle.x).w;
        let (query_focused, replace_focused) = (
            app.focus == Focus::GlobalQuery,
            app.focus == Focus::GlobalReplace,
        );
        let mut query = std::mem::take(&mut app.panels.global.query);
        let mut replace = std::mem::take(&mut app.panels.global.replace);
        let wraps = (
            wrap_area_field(app, frame, width, &mut query, query_focused),
            wrap_area_field(app, frame, width, &mut replace, replace_focused),
        );
        app.panels.global.query = query;
        app.panels.global.replace = replace;
        wraps
    };
    app.set_search_field_lines((wraps.0.lines(), wraps.1.lines()));
    let parts = app.panel_parts();
    draw_panel_header(app, frame, &parts);

    // 绘制输入框时需要更新其中记录的几何信息：先将输入框从 `app` 中取出，绘制后放回。
    let mut fields = [
        std::mem::take(&mut app.panels.global.query),
        std::mem::take(&mut app.panels.global.replace),
        std::mem::take(&mut app.panels.global.include),
        std::mem::take(&mut app.panels.global.exclude),
    ];
    let caret = draw_panel_inputs(app, frame, &parts, &mut fields, wraps);
    let [query, replace, include, exclude] = fields;
    app.panels.global.query = query;
    app.panels.global.replace = replace;
    app.panels.global.include = include;
    app.panels.global.exclude = exclude;

    draw_summary(app, frame, &parts);
    draw_results(app, frame, &parts);

    caret
}

/// 绘制面板第一行右端的三个按钮（左边是共用的项目和分支）。
fn draw_panel_header(app: &App, frame: &mut Frame<'_>, parts: &PanelParts) {
    let p = app.palette;
    let header = parts.header;

    let has_results = !app.panels.global.engine.results.is_empty();

    for (rect, tool) in parts.tools {
        // 没有结果时「清除」「全部折叠」置灰。
        let enabled = tool == PanelTool::Refresh || has_results;
        let hovered = enabled && app.pointer.hover == Hover::Panel(PanelButton::Tool(tool));
        draw_button(frame, p, rect, header, hovered, false);

        let color = match (enabled, hovered) {
            (false, _) => p.text_dim,
            (true, true) => p.text,
            (true, false) => p.text_muted,
        };
        let c = center(rect);

        match tool {
            PanelTool::Refresh => icons::refresh(frame, c, color, header),
            PanelTool::Clear => icons::clear_results(frame, c, color, header),
            PanelTool::CollapseAll if app.panels.global.all_collapsed() => {
                icons::expand_all(frame, c, color, header);
            }
            PanelTool::CollapseAll => icons::collapse_all(frame, c, color, header),
        }
    }
}

/// 侧栏里的输入框，见 [`field_box`]。
fn draw_field_box(app: &App, frame: &mut Frame<'_>, rect: Rect, focused: bool, error: bool) {
    field_box(app, frame, rect, app.palette.sidebar_bg, focused, error);
}

/// 绘制查找、替换、包含、排除四个输入框及其旁边的按钮，返回有焦点的输入框的光标矩形。
fn draw_panel_inputs(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PanelParts,
    [query, replace, include, exclude]: &mut [TextField; 4],
    (query_wrap, replace_wrap): (AreaWrap, AreaWrap),
) -> Option<Rect> {
    let p = app.palette;
    let g = &app.panels.global;
    let panel = app.panel_rect();
    let hovered = |button| app.pointer.hover == Hover::Panel(button);
    let error = g.engine.error().is_some() && g.due.is_none();

    let focused = app.focus == Focus::GlobalQuery;
    draw_field_box(app, frame, parts.query, focused, error);

    // 输入框左端是展开替换的箭头。
    draw_button(
        frame,
        p,
        parts.expand,
        panel,
        hovered(PanelButton::Expand),
        false,
    );
    // 箭头按替换行展开的比例从朝右旋转到朝下。
    let turned = g.replace_share() * FRAC_PI_2;
    icons::chevron_turned(
        frame,
        center(parts.expand),
        6.5,
        turned,
        p.text_muted,
        panel,
    );

    // 过渡途中文字按过渡结束后的高度排版，只绘制在当前的框内。
    let (query_h, replace_h) = g.settled_field_heights();
    let lines = |field: Rect, right: f32, height: f32| {
        let area = PanelParts::text_area(field, right);
        // 文字行在第一行的高度上居中。
        let pad = (crate::layout::PANEL_FIELD - AREA_LINE) / 2.0;
        Rect::new(area.x, field.y + pad, area.w, height - 2.0 * pad)
    };
    let area = lines(parts.query, parts.case_toggle.x, query_h);
    let mut caret = frame.within(parts.query, |frame| {
        draw_area_field(
            app,
            frame,
            area,
            query,
            query_wrap,
            t!("workspace-search-placeholder"),
            focused,
        )
    });

    let toggles = [
        (
            parts.case_toggle,
            g.case_sensitive,
            hovered(PanelButton::Case),
        ),
        (parts.word_toggle, g.whole_word, hovered(PanelButton::Word)),
        (parts.regex_toggle, g.regex, hovered(PanelButton::Regex)),
    ];
    draw_search_options(app, frame, toggles, panel);

    // 输入框最右端是「过滤」：展开了包含、排除，或者包含、排除不为空时用强调色。
    let button = parts.details;
    let active = g.details_open
        || g.include_ignored
        || !g.include.text().is_empty()
        || !g.exclude.text().is_empty();
    draw_button(
        frame,
        p,
        button,
        panel,
        hovered(PanelButton::Details),
        g.details_open,
    );
    let color = if active { p.accent } else { p.text_muted };
    icons::filter(frame, center(button), color, panel);

    // 展开、收起途中只绘制露出的部分（`replace_band`），并按展开比例淡入淡出。
    if parts.replace.h > 0.0 {
        let c = frame.within(reveal(parts.replace_band), |frame| {
            frame.with_opacity(g.replace_share(), |frame| {
                let focused = app.focus == Focus::GlobalReplace;
                draw_field_box(app, frame, parts.replace, focused, false);
                let area = lines(parts.replace, parts.replace_all.x, replace_h);
                let c = frame.within(parts.replace, |frame| {
                    draw_area_field(
                        app,
                        frame,
                        area,
                        replace,
                        replace_wrap,
                        t!("workspace-search-replace-placeholder"),
                        focused,
                    )
                });

                // 全部替换：图标按钮，与上一行的正则开关在水平方向上对齐；没有结果时置灰。
                let button = parts.replace_all;
                let enabled = g.engine.total() > 0;

                draw_button(
                    frame,
                    p,
                    button,
                    panel,
                    enabled && hovered(PanelButton::ReplaceAll),
                    false,
                );

                let color = if enabled { p.text_muted } else { p.text_dim };
                icons::replace_all(frame, center(button), color, panel);
                c
            })
        });
        caret = caret.or(c);
    }

    // 包含、排除：输入框内左侧是淡色的标签，其后才是输入区，标签不另占一行。
    if parts.include.h > 0.0 {
        let fields = [
            (
                parts.include,
                include,
                Focus::GlobalInclude,
                t!("workspace-search-include"),
                t!("workspace-search-include-example"),
            ),
            (
                parts.exclude,
                exclude,
                Focus::GlobalExclude,
                t!("workspace-search-exclude"),
                t!("workspace-search-exclude-example"),
            ),
        ];
        let glob_error = error && g.engine.is_filter_error();

        let c = frame.within(reveal(parts.details_band), |frame| {
            frame.with_opacity(g.details_share(), |frame| {
                let mut caret = None;
                for (rect, field, focus, title, placeholder) in fields {
                    let focused = app.focus == focus;
                    let error = glob_error && !field.text().is_empty();
                    draw_field_box(app, frame, rect, focused, error);
                    let title_x = rect.x - 8.0 + PANEL_LEFT;
                    let end = ui_text_small(frame, app, title_x, rect, title, p.text_muted, rect);
                    // 「排除」的右端让给「也搜索被忽略的文件」开关。
                    let right = if focus == Focus::GlobalExclude {
                        parts.ignored_toggle.x - 2.0
                    } else {
                        rect.right() - 6.0
                    };
                    let area = Rect::new(end + 8.0, rect.y, (right - end - 8.0).max(0.0), rect.h);
                    let c = draw_bar_field(app, frame, area, field, placeholder, focused);
                    caret = caret.or(c);
                }
                let button = parts.ignored_toggle;
                let on = g.include_ignored;
                draw_button(frame, p, button, panel, hovered(PanelButton::Ignored), on);
                let color = if on { p.accent } else { p.text_muted };
                icons::ignored_files(frame, center(button), color, panel);
                caret
            })
        });
        caret = caret.or(c);
    }

    caret
}

/// 返回展开、收起途中输入框可绘制的范围：露出的部分 `band`，下沿多留 1 像素。
///
/// 焦点和出错的外圈由 `field_box` 绘制在输入框外 1 像素处；不留出这 1 像素时，
/// 完全展开后最下面一个输入框外圈的底边会被裁掉。
fn reveal(band: Rect) -> Rect {
    Rect::new(band.x, band.y, band.w, band.h + 1.0)
}

/// 绘制结果统计行：显示「3 个文件中有 12 个结果」等统计，或搜索中、没有结果、出错的状态。
///
/// 没有内容时这一行不占空间（`parts.summary` 高度为 0），不绘制。
fn draw_summary(app: &App, frame: &mut Frame<'_>, parts: &PanelParts) {
    if parts.summary.h <= 0.0 {
        return;
    }
    let p = app.palette;
    let g = &app.panels.global;
    let engine = &g.engine;
    let row = parts.summary;
    let files = engine.results.len();
    let total = engine.total();
    let searching = engine.is_running() || g.due.is_some();

    let (text, color) = if let Some(error) = engine.error().filter(|_| g.due.is_none()) {
        (error.to_owned(), p.error)
    } else if g.query.text().is_empty() {
        (String::new(), p.text_dim)
    } else if searching && total == 0 {
        (t!("workspace-search-searching").to_owned(), p.text_dim)
    } else {
        // 截断了，或有行因正则回溯过多而放弃查找：计数是下限。
        let more = if engine.is_truncated() || engine.gave_up() {
            "+"
        } else {
            ""
        };
        let text = if searching {
            t!(
                "workspace-search-summary-searching",
                files = files,
                total = total,
                more = more
            )
        } else {
            t!(
                "workspace-search-summary",
                files = files,
                total = total,
                more = more
            )
        };
        (text, p.text_muted)
    };

    // 文本与输入框里的文字对齐。
    let x = row.x + PANEL_LEFT;
    let max = (parts.query.right() - x).max(0.0);
    let text = ellipsize(frame, &text, app.fonts.ui_small, max);
    ui_text_small(frame, app, x, row, &text, color, row);
}

/// 绘制结果列表：每个文件一行（箭头、图标、文件名、所在目录、匹配数），其下每处匹配一行。
fn draw_results(app: &mut App, frame: &mut Frame<'_>, parts: &PanelParts) {
    let area = parts.results;
    if app.no_search_results() {
        draw_empty_state(
            app,
            frame,
            (area, sidebar_span(app)),
            t!("workspace-search-no-results"),
            Some(if app.panels.global.include_ignored {
                t!("workspace-search-no-results-hint")
            } else {
                t!("workspace-search-no-results-hint-ignored")
            }),
        );
        return;
    }
    let scroll = app.panels.global.scroll;
    let rows = app.panels.global.rows().len();
    let replace_open = app.panels.global.replace_open;
    // 行号一栏按结果里最大的行号定宽（各文件的匹配按行号排序，最后一处就是最大的），滚动、
    // 悬停时都不变。
    let last_line = app
        .panels
        .global
        .engine
        .results
        .iter()
        .filter_map(|f| f.matches.last())
        .map(|m| m.line + 1)
        .max()
        .unwrap_or(1);
    let digit = frame.measure(FontKind::Ui, app.fonts.ui_small, "0");
    let cx = ResultContext {
        area,
        selected: app
            .panels
            .global
            .selected_row()
            .map(|i| app.panels.global.rows()[i]),
        focused: app.focus == Focus::Results,
        replace_open,
        replacement: replace_open && app.panels.global.preview().is_some(),
        digit,
        numbers: digit * last_line.to_string().len() as f32,
    };

    // 文件正在折叠或展开时按动画中的结果行绘制：匹配行位置不变，只显示已露出的部分，
    // 其下方的行随之上下移动。动画时长很短，期间不显示悬停。
    let shared = &*app;
    if let Some(motion) = &shared.panels.global.motion {
        let (block, hidden) = (motion.block(), motion.hidden());
        for (v, &kind) in motion.rows.iter().enumerate() {
            let (row, clip) = crate::layout::tree_row_slot(area, scroll, v, Some(block), hidden);
            let clip = clip.intersect(area);
            if clip.h <= 0.0 || row.bottom() < area.y || row.y > area.bottom() {
                continue;
            }
            frame.within(clip, |frame| {
                draw_result_row(shared, frame, &cx, None, kind, row)
            });
        }
    } else {
        let first = (scroll / TREE_ROW).floor().max(0.0) as usize;
        let visible = (area.h / TREE_ROW).ceil() as usize + 1;
        for index in first..(first + visible).min(rows) {
            let row = shared.result_row_rect(index);
            let kind = shared.panels.global.rows()[index];
            draw_result_row(shared, frame, &cx, Some(index), kind, row);
        }
    }

    let content = (rows as f32 + 1.0) * TREE_ROW;
    draw_scrollbar(app, frame, ScrollArea::Panel, 0, area, scroll, content);
}

/// 结果列表中各行共用的绘制参数。
struct ResultContext {
    /// 结果列表区域，各行均裁剪在该区域内。
    area: Rect,
    /// 选中的结果行。
    selected: Option<ResultRow>,
    /// 焦点是否在结果列表上。
    focused: bool,
    /// 替换行是否展开；展开时悬停的行多显示一个「替换」按钮。
    replace_open: bool,
    /// 匹配行是否显示替换后的文本。
    replacement: bool,
    /// 单个数字的宽度（逻辑像素）。
    digit: f32,
    /// 行号列的宽度（逻辑像素），见 [`RowEnd::numbers`]。
    numbers: f32,
}

/// 绘制一个结果行：底色、内容，以及悬停或选中时右端浮出的按钮。
///
/// `index` 为该行在当前结果行中的序号，用于判断悬停；
/// 绘制折叠动画中的行时传 `None`，不显示悬停。
fn draw_result_row(
    app: &App,
    frame: &mut Frame<'_>,
    cx: &ResultContext,
    index: Option<usize>,
    kind: ResultRow,
    row: Rect,
) {
    let p = app.palette;
    let area = cx.area;
    let is_selected = cx.selected == Some(kind);
    let hovered = index.is_some_and(
        |index| matches!(app.pointer.hover, Hover::Result(i) | Hover::RowAction(i, _) if i == index),
    );
    let pill = Rect::new(row.x + 8.0, row.y + 1.0, row.w - 16.0, row.h - 2.0);

    let background = row_color(p, is_selected, cx.focused, hovered);
    if let Some(color) = background {
        frame.rect_clipped(pill, color, RADIUS_ROW, area);
    }

    let (replace_rect, dismiss_rect) = row_actions(row, cx.replace_open);
    let end = RowEnd {
        slot: dismiss_rect,
        pad: (dismiss_rect.w - cx.digit) / 2.0,
        numbers: cx.numbers,
        covered: background.is_some(),
    };

    match kind {
        ResultRow::File(f) => draw_file_row(app, frame, row, area, f, &end),
        ResultRow::Match(f, m) => {
            draw_match_row(app, frame, row, area, (f, m), &end, cx.replacement);
        }
    }

    // 悬停或选中时，按钮浮现在右端匹配数、行号所在的位置：先用该行底色遮盖下方文本，
    // 遮盖区左侧带一段渐隐，再绘制按钮。
    if let Some(background) = background {
        let left = replace_rect.map_or(dismiss_rect.x, |r| r.x) - 4.0;
        let solid = over(p.sidebar_bg, background);
        let cover = Rect::new(left, pill.y, pill.right() - left, pill.h);
        let r = RADIUS_ROW;
        frame.rect_corners(cover, solid, [0.0, r, r, 0.0], area);
        fade(
            frame,
            Rect::new(left - COVER_FADE, pill.y, COVER_FADE, pill.h),
            solid,
            false,
        );

        let hover_on =
            |action| index.is_some_and(|i| app.pointer.hover == Hover::RowAction(i, action));

        if let Some(rect) = replace_rect {
            draw_button(frame, p, rect, area, hover_on(RowAction::Replace), false);

            icons::replace_one(frame, center(rect), p.text_muted, area);
        }

        draw_button(
            frame,
            p,
            dismiss_rect,
            area,
            hover_on(RowAction::Dismiss),
            false,
        );

        icons::close(frame, center(dismiss_rect), 6.5, p.text_muted, area);
    }
}

/// 结果行右端那一格：匹配数、行号和悬停时浮出的按钮共用一条右边和一条上下中线，按钮出来时
/// 原地顶替匹配数、行号，左边的文字不动。
struct RowEnd {
    /// 最右一个按钮（「从结果中移除」）的位置（`row_actions`）。
    slot: Rect,
    /// 数字在这一格里左右留的空：一位数正好居中，多一位往左长一位。
    pad: f32,
    /// 行号一栏的宽度：结果里最大的行号有几位就留几位，各匹配行一样宽，右对齐。
    numbers: f32,
    /// 按钮浮出来了（悬停或选中），匹配数、行号不绘制。
    covered: bool,
}

/// 绘制第 `f` 个文件的结果行：箭头、文件图标、文件名、淡色的所在目录，右端是匹配数
/// （悬停时换成按钮）。
fn draw_file_row(app: &App, frame: &mut Frame<'_>, row: Rect, clip: Rect, f: usize, end: &RowEnd) {
    let p = app.palette;
    let file = &app.panels.global.engine.results[f];
    let collapsed = app.panels.global.collapsed.contains(&file.path);
    let cy = icon_center_y(app, row, app.fonts.ui);
    // 折叠、展开途中，箭头按匹配行露出的比例旋转。
    let turned = match &app.panels.global.motion {
        Some(m) if m.rows.get(m.parent) == Some(&ResultRow::File(f)) => m.turned(),
        _ => f32::from(u8::from(!collapsed)),
    };
    let angle = turned * FRAC_PI_2;
    let chevron = (row.x + PANEL_CHEVRON, cy);
    icons::chevron_turned(frame, chevron, 6.0, angle, p.text_dim, clip);

    let (dir, name) = match file.relative.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", file.relative.as_str()),
    };
    draw_file_icon(frame, p, (row.x + PANEL_LEFT, cy), name, clip);
    let x = row.x + PANEL_TEXT;

    // 右端的匹配数绘制为与按钮同高的胶囊，右边缘与按钮对齐；
    // 文字的右边界按胶囊计算，悬停时保持不变。
    let count = file.matches.len().to_string();
    let size = app.fonts.ui_small;
    let count_w = frame.measure(FontKind::Ui, size, &count);
    let slot = end.slot;
    let badge_w = (count_w + 2.0 * end.pad).max(slot.w);
    let badge = Rect::new(slot.right() - badge_w, slot.y, badge_w, slot.h);
    if !end.covered {
        frame.rect_clipped(badge, p.hover, slot.h / 2.0, clip);
        let x = badge.right() - end.pad - count_w;
        caps_label(frame, app, size, (x, badge), &count, p.text_muted, clip);
    }
    let right = badge.x - end.pad;

    let name_max = (right - x).max(0.0);
    let name = ellipsize(frame, name, app.fonts.ui, name_max);
    let x = ui_text(frame, app, x, row, &name, p.text, clip);

    // 所在目录跟在文件名之后，放不下时从中间截短，仍放不下则不绘制。非 UTF-8 的文件不能替换，
    // 此处改为显示说明。
    let (note, color) = if file.lossy {
        (t!("workspace-search-not-utf8"), p.error)
    } else {
        (dir, p.text_dim)
    };
    let note_max = right - x - 8.0;

    if !note.is_empty() && note_max > 24.0 {
        let note = ellipsize(frame, note, app.fonts.ui_small, note_max);
        ui_text_small(frame, app, x + 8.0, row, &note, color, clip);
    }
}

/// 绘制第 `f` 个文件的第 `m` 处匹配的结果行：一小段原文，其中匹配的文本带底色；
/// 展开替换（`replacing`）时，匹配的文本被划掉，其后紧跟替换后的文本。
///
/// 右端是行号（悬停时换成按钮）。展开替换时调用方先用 `GlobalSearch::preview` 更新替换预览，
/// 这里只读缓存好的那份。
fn draw_match_row(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    clip: Rect,
    (f, m): (usize, usize),
    end: &RowEnd,
    replacing: bool,
) {
    let p = app.palette;
    let found = &app.panels.global.engine.results[f].matches[m];
    // 右端的行号（不用打开就知道在哪一行）在定宽的一栏里右对齐，末位和文件行匹配数的末位
    // 对齐；文本让出整栏，行号几位都不影响文本。
    let numbers_right = end.slot.right() - end.pad;
    if !end.covered {
        let number = (found.line + 1).to_string();
        let size = app.fonts.ui_small;
        let x = numbers_right - frame.measure(FontKind::Ui, size, &number);
        caps_label(frame, app, size, (x, end.slot), &number, p.text_muted, clip);
    }
    let right = numbers_right - end.numbers - end.pad;
    let (preview, highlight) = (found.preview.as_str(), found.highlight.clone());
    let replaced = if replacing {
        app.panels
            .global
            .cached_preview()
            .map(|r| r.apply(preview, highlight.clone()))
    } else {
        None
    };

    let x0 = row.x + PANEL_TEXT;
    let (before, matched, after) = (
        &preview[..highlight.start],
        &preview[highlight.clone()],
        &preview[highlight.end..],
    );

    // 匹配位置离行首太远、整段放不下时，匹配之前的文本从左侧截去并换成「…」，使匹配的文本完整露出。
    let room = right - x0;
    let before_w = frame.measure(FontKind::Ui, app.fonts.ui, before);
    let matched_w = frame.measure(FontKind::Ui, app.fonts.ui, matched);
    // 展开替换时，替换后的文本也要露出来，因此计入宽度。
    let replaced_w = replaced
        .as_deref()
        .map_or(0.0, |r| frame.measure(FontKind::Ui, app.fonts.ui, r) + 3.0);
    let overflow = before_w + matched_w + replaced_w + 8.0 - room;
    let mut x = x0;
    let mut before = before;

    // 从前面按字符逐个去掉，直到放得下，使「…」之后不会出现半个字符。
    if overflow > 0.0 && !before.is_empty() {
        x = ui_text(frame, app, x0, row, "…", p.text_dim, clip) + 1.0;
        let need = overflow + (x - x0);
        let mut dropped = 0.0;

        for (i, c) in before.char_indices() {
            if dropped >= need {
                before = &before[i..];
                break;
            }

            dropped += frame.measure(FontKind::Ui, app.fonts.ui, c.encode_utf8(&mut [0; 4]));

            if i + c.len_utf8() == before.len() {
                before = "";
            }
        }
    }

    let clip = Rect::new(clip.x, clip.y, (right - clip.x).max(0.0), clip.h);
    x = ui_text(frame, app, x, row, before, p.text_muted, clip);

    let mark = Rect::new(x - 1.0, row.y + 5.0, matched_w + 2.0, row.h - 10.0);
    let mark_color = if replaced.is_some() {
        p.error.alpha(0.18)
    } else {
        p.search_match
    };
    frame.rect_clipped(mark, mark_color, 3.0, clip);
    let end = ui_text(frame, app, x, row, matched, p.text, clip);

    if let Some(new) = &replaced {
        // 被替换的文本划一道删除线，其后是替换后的文本，底色为绿色调。
        let strike = Rect::new(x, row.y + row.h / 2.0, end - x, 1.0);
        frame.rect_clipped(strike, p.text_muted, 0.0, clip);
        let new_w = frame.measure(FontKind::Ui, app.fonts.ui, new);
        let added = Rect::new(end + 1.0, row.y + 5.0, new_w + 2.0, row.h - 10.0);
        frame.rect_clipped(added, p.syntax_string.alpha(0.20), 3.0, clip);
        x = ui_text(frame, app, end + 2.0, row, new, p.text, clip) + 1.0;
    } else {
        x = end;
    }

    ui_text(frame, app, x, row, after, p.text_muted, clip);
}

/// 返回 `fg` 叠在不透明的 `bg` 上得到的颜色，结果不透明。
///
/// 用于绘制与下层完全相同的底色，以遮盖其上的文本。
pub(in crate::view) fn over(bg: Color, fg: Color) -> Color {
    let mix = |b: f32, f: f32| b + (f - b) * fg.a;
    Color {
        r: mix(bg.r, fg.r),
        g: mix(bg.g, fg.g),
        b: mix(bg.b, fg.b),
        a: 1.0,
    }
}

/// 返回面板内按钮的悬停提示（按钮矩形和提示文本）；没有提示时返回 `None`。
pub(in crate::view) fn tooltip(app: &App) -> Option<(Rect, String)> {
    use crate::app::{CommitTarget, DiffTarget};
    use crate::keymap::Action;
    use crate::layout::with_key;
    match app.pointer.hover {
        Hover::TerminalHeader(hit @ crate::app::terminal::HeaderHit::ShellMenu) => {
            let rect = app.hits.terminal.iter().find(|(_, h)| *h == hit)?.0;
            Some((rect, t!("terminal-shell-select").to_owned()))
        }
        Hover::TerminalHeader(hit @ crate::app::terminal::HeaderHit::Find(part)) => {
            use crate::app::terminal::FindPart;
            let label = match part {
                FindPart::Bar => return None,
                FindPart::Previous => t!("cmd-find-previous"),
                FindPart::Next => t!("cmd-find-next"),
                FindPart::Close => t!("workspace-find-close"),
                FindPart::Case => t!("workspace-find-match-case"),
                FindPart::Word => t!("workspace-find-whole-word"),
                FindPart::Regex => t!("workspace-find-use-regex"),
            };
            let rect = app.hits.terminal.iter().find(|(_, h)| *h == hit)?.0;
            Some((rect, label.to_owned()))
        }
        Hover::TerminalHeader(hit @ crate::app::terminal::HeaderHit::Position) => {
            let rect = app.hits.terminal.iter().find(|(_, h)| *h == hit)?.0;
            let label = if app.terminal_right() {
                t!("terminal-dock-bottom")
            } else {
                t!("terminal-dock-right")
            };
            Some((rect, label.to_owned()))
        }
        Hover::TerminalHeader(hit @ crate::app::terminal::HeaderHit::Hide) => {
            let rect = app.hits.terminal.iter().find(|(_, h)| *h == hit)?.0;
            Some((
                rect,
                with_key(t!("terminal-hide-panel"), Some(Action::ToggleTerminal)),
            ))
        }
        Hover::TerminalHeader(hit @ crate::app::terminal::HeaderHit::Run(button)) => {
            use crate::app::terminal::run::RunButton;
            let rect = app.hits.terminal.iter().find(|(_, h)| *h == hit)?.0;
            let label = match button {
                RunButton::Menu => t!("terminal-run-select").to_owned(),
                RunButton::Start if app.run.list.is_empty() => t!("terminal-run-add").to_owned(),
                RunButton::Start if app.run_config_running() => {
                    with_key(t!("terminal-run-again"), Some(Action::RunConfiguration))
                }
                RunButton::Start => {
                    with_key(t!("cmd-run-configuration"), Some(Action::RunConfiguration))
                }
                RunButton::Stop => with_key(
                    t!("cmd-stop-configuration"),
                    Some(Action::StopConfiguration),
                ),
            };
            Some((rect, label))
        }
        Hover::Status(item @ crate::app::StatusItem::Terminal) => {
            let rect = app.hits.status.iter().find(|(_, i)| *i == item)?.0;
            let label = if app.terminal.open {
                t!("terminal-hide-panel")
            } else {
                t!("terminal-show-panel")
            };
            Some((rect, with_key(label, Some(Action::ToggleTerminal))))
        }
        Hover::Status(item @ crate::app::StatusItem::Remote) => {
            let rect = app.hits.status.iter().find(|(_, i)| *i == item)?.0;
            // 已打开远端目录时说明连着哪台主机。
            let host = (app.editor.root())
                .and_then(winc_backend::locate)
                .map(|location| location.target);
            let label = match host {
                Some(host) if !winc_backend::remote::is_connected(&host) => {
                    t!("workspace-remote-status-lost", host = host.as_str())
                }
                Some(host) if winc_backend::remote::is_stalled(&host) => {
                    t!("workspace-remote-status-stalled", host = host.as_str())
                }
                Some(host) => t!("workspace-remote-status-connected", host = host.as_str()),
                None => t!("workspace-start-remote").to_owned(),
            };
            Some((rect, with_key(&label, Some(Action::RemoteConnect))))
        }
        Hover::Status(item @ crate::app::StatusItem::Update) => {
            let rect = app.hits.status.iter().find(|(_, i)| *i == item)?.0;
            Some((rect, crate::view::status::update_tooltip(app)?))
        }
        Hover::Status(item @ crate::app::StatusItem::Settings) => {
            let rect = app.hits.status.iter().find(|(_, i)| *i == item)?.0;
            Some((rect, with_key(t!("cmd-settings"), Some(Action::Settings))))
        }
        // 分支选择器的拉取、推送、获取：写明会做什么（和谁差几个提交、推到哪儿）；不能用时写原因。
        Hover::PickerPart(
            part @ (crate::app::PickerPart::Pull
            | crate::app::PickerPart::Push
            | crate::app::PickerPart::Fetch),
        ) => {
            let rect = app.hits.picker_parts.iter().find(|(_, p)| *p == part)?.0;
            let command = app.picker_action_command(part);
            let label = app.menu_label(command);
            let tip = if app.git.remotes.is_empty() {
                t!("git-push-no-remote").to_owned()
            } else {
                label.subtitle.unwrap_or(label.title)
            };
            Some((rect, tip))
        }
        // 提交按钮：无法提交时显示原因，可以提交时显示提交目标和快捷键；按住 ⌥ 时是提交并推送。
        Hover::Commit(CommitTarget::Commit) => {
            use winc_editor::git::{Head, Operation};
            let parts = app.commit_parts();
            let status = app.git.status.as_ref()?;
            if app.git.commit.busy {
                return None;
            }
            if let Some(reason) = app.git.commit.blocker(status) {
                return Some((parts.commit, reason));
            }
            let name = match (status.operation, &status.head) {
                (Some(Operation::Rebase), _) => t!("workspace-tip-continue-rebase").to_owned(),
                (Some(op), _) => t!("workspace-tip-finish-operation", operation = op.label()),
                (None, _) if app.git.commit.amend => t!("workspace-tip-amend").to_owned(),
                (None, Some(Head::Branch(b))) if app.commit_pushes() => {
                    t!("workspace-tip-commit-push-to", branch = b.as_str())
                }
                (None, Some(Head::Branch(b))) => t!("workspace-tip-commit-to", branch = b.as_str()),
                (None, _) => t!("workspace-tip-commit").to_owned(),
            };
            let key = match (cfg!(target_os = "macos"), app.commit_pushes()) {
                (true, true) => "⌥⌘↩",
                (true, false) => "⌘↩",
                (false, true) => "Ctrl+Alt+Enter",
                (false, false) => "Ctrl+Enter",
            };
            Some((
                parts.commit,
                format!("{name}{}{key}", crate::layout::TIP_KEY),
            ))
        }
        Hover::Commit(
            target @ (CommitTarget::OpenFile(_)
            | CommitTarget::RevertFile(_)
            | CommitTarget::GroupRevert(_)),
        ) => {
            let rect = app.hits.commit.iter().find(|(_, t)| *t == target)?.0;
            let label = match target {
                CommitTarget::OpenFile(_) => t!("workspace-tip-open-file"),
                CommitTarget::GroupRevert(_) => t!("menu-commit-roll-back"),
                _ => t!("workspace-tip-revert-change"),
            };
            Some((rect, label.to_owned()))
        }
        // 同步控件上没有文字，要做的事写在这里。
        Hover::Commit(target @ CommitTarget::Sync) => {
            let rect = app.hits.commit.iter().find(|(_, t)| *t == target)?.0;
            let upstream = (app.git.branches.iter())
                .find(|b| b.current)
                .and_then(|b| b.upstream.as_deref())
                .unwrap_or_default();
            let label = match (app.git.branch.running.as_deref(), app.git_sync_state()) {
                (Some(task), _) => t!("workspace-status-branch-running", task = task),
                (None, SyncState::Publish) => t!("git-tip-publish").to_owned(),
                (None, SyncState::InSync) => t!("git-tip-in-sync", upstream = upstream),
                (None, SyncState::Push(count)) => {
                    t!("git-tip-push", count = count, upstream = upstream)
                }
                (None, SyncState::Pull(count)) => {
                    t!("git-tip-pull", count = count, upstream = upstream)
                }
                (None, SyncState::Both { ahead, behind }) => t!(
                    "git-tip-both",
                    ahead = ahead,
                    behind = behind,
                    upstream = upstream
                ),
                (None, SyncState::NoRemote | SyncState::Hidden) => t!("git-tip-fetch").to_owned(),
            };
            Some((rect, label))
        }
        Hover::Outline(target @ OutlineTarget::FoldAll) => {
            let rect = app.hits.outline.iter().find(|(_, t)| *t == target)?.0;
            let label = if app.outline_any_expanded() {
                t!("workspace-outline-tool-collapse-all")
            } else {
                t!("workspace-outline-tool-expand-all")
            };
            Some((rect, label.to_owned()))
        }
        // 选择器开着时每一行都写着，不再提示。
        Hover::Commit(target @ CommitTarget::Location) if !app.picker_lists_locations() => {
            let rect = app.hits.commit.iter().find(|(_, t)| *t == target)?.0;
            let count = app.git_other_changes();
            (count > 0).then(|| {
                let label = if app.git.repositories.len() >= 2 {
                    t!("git-tip-other-locations", count = count)
                } else {
                    t!("git-tip-other-worktrees", count = count)
                };
                (rect, label)
            })
        }
        Hover::Commit(CommitTarget::Location) => None,
        // 历史差异文件列表的显示方式按钮：写明点击后切换成哪一种。
        Hover::LogDiffLayout => {
            let label = if app.log_list().flat() {
                t!("workspace-tip-show-as-tree")
            } else {
                t!("workspace-tip-show-as-list")
            };
            Some((app.log_diff_layout_button(), label.to_owned()))
        }
        Hover::Commit(CommitTarget::CommitMenu) => Some((
            app.commit_parts().commit_menu,
            t!("workspace-tip-commit-menu").to_owned(),
        )),
        Hover::Commit(target) => {
            let rect = app
                .commit_parts()
                .tools
                .iter()
                .find(|(_, t)| *t == target)?
                .0;
            if target == CommitTarget::Log {
                return Some((
                    rect,
                    with_key(t!("workspace-tip-git-log"), Some(Action::GitShowLog)),
                ));
            }
            if target == CommitTarget::Refresh {
                return Some((rect, t!("workspace-tip-git-refresh").to_owned()));
            }
            Some((rect, t!("workspace-tip-more-actions").to_owned()))
        }
        Hover::Merge(target) => {
            use crate::app::git::merge::MergeTarget;
            let rect = app.hits.merge.iter().find(|(_, t)| *t == target)?.0;
            let label = match target {
                MergeTarget::Previous => with_key(
                    t!("workspace-tip-previous-conflict"),
                    Some(Action::PreviousDifference),
                ),
                _ => with_key(
                    t!("workspace-tip-next-conflict"),
                    Some(Action::NextDifference),
                ),
            };
            Some((rect, label))
        }
        // 附属工作树：写明是哪个仓库的工作树、在哪里（项目名写的是工作树自己的目录名）。
        Hover::ProjectName => {
            let main = app.git.main_worktree()?;
            let root = app.git.root.as_deref()?;
            let name = crate::app::git::worktree::worktree_name(main);
            let path = winc_editor::display_path(root, None);
            let tip = t!("workspace-tip-worktree-of", name = name, path = path);
            Some((app.hits.project, tip))
        }
        Hover::RulerChange(hunk) => app.ruler_tip(hunk),
        Hover::Blame(line) => app.blame_tip(line),
        Hover::Change(target) => {
            use crate::app::ChangeTarget;
            let rect = app.hits.changes.iter().find(|(_, t)| *t == target)?.0;
            let label = match target {
                ChangeTarget::Previous => with_key(
                    t!("workspace-tip-previous-change"),
                    Some(Action::PreviousDifference),
                ),
                ChangeTarget::Next => with_key(
                    t!("workspace-tip-next-change"),
                    Some(Action::NextDifference),
                ),
                ChangeTarget::Revert => with_key(
                    t!("workspace-tip-revert-change"),
                    Some(Action::GitRevertChange),
                ),
                ChangeTarget::ShowDiff => {
                    with_key(t!("workspace-tip-open-diff"), Some(Action::GitShowChanges))
                }
                ChangeTarget::Close => t!("workspace-tip-close-esc").to_owned(),
            };
            Some((rect, label))
        }
        Hover::Diff(target) => {
            let rect = app.hits.diff.iter().find(|(_, t)| *t == target)?.0;
            let label = match target {
                DiffTarget::Previous => with_key(
                    t!("workspace-tip-previous-change"),
                    Some(Action::PreviousDifference),
                ),
                DiffTarget::Next => with_key(
                    t!("workspace-tip-next-change"),
                    Some(Action::NextDifference),
                ),
                DiffTarget::PreviousFile => with_key(
                    t!("workspace-tip-previous-file"),
                    Some(Action::PreviousDiffFile),
                ),
                DiffTarget::NextFile => {
                    with_key(t!("workspace-tip-next-file"), Some(Action::NextDiffFile))
                }
                DiffTarget::Layout if app.editor.diff_unified => {
                    t!("workspace-tip-diff-side-by-side").to_owned()
                }
                DiffTarget::Layout => t!("workspace-tip-diff-inline").to_owned(),
                DiffTarget::Revert(_) => with_key(
                    t!("workspace-tip-revert-to-left"),
                    Some(Action::GitRevertChange),
                ),
                DiffTarget::Include(_) => t!("workspace-tip-include-change").to_owned(),
                DiffTarget::IncludeLine(..) => t!("workspace-tip-include-line").to_owned(),
                _ if app.editor.active()?.diff.as_deref()?.collapses() => {
                    t!("workspace-tip-expand-unchanged").to_owned()
                }
                _ => t!("workspace-tip-collapse-unchanged").to_owned(),
            };
            Some((rect, label))
        }
        // 标签提示显示完整名称和所在目录，便于确认被截短的文件名和历史差异的版本；
        // 关闭按钮显示快捷键。按住或拖动标签时不显示提示。
        Hover::Tab(index, close) => {
            if app.tabs.drag.is_some() {
                return None;
            }
            let hit = app.hits.tabs.iter().find(|h| h.index == index)?;
            if close {
                return Some((
                    hit.close,
                    with_key(t!("workspace-tip-close-tab"), Some(Action::CloseTab)),
                ));
            }
            let doc = app.editor.documents.get(index)?;
            let title = doc.name();
            let path = (doc.path.as_deref()).or_else(|| {
                doc.diff
                    .as_deref()
                    .and_then(winc_editor::diff_view::DiffView::file_path)
            });
            let label = match path.and_then(std::path::Path::parent) {
                Some(dir) => format!("{title}\n{}", winc_editor::display_path(dir, None)),
                // 没有对应文件的标签（未命名文件、日志等）只在名称被截短时提示完整名称。
                None if hit.elided => title.into_owned(),
                None => return None,
            };
            Some((hit.tab, label))
        }
        Hover::TabList => Some((app.tabs.list?, t!("workspace-tip-all-tabs").to_owned())),
        Hover::ViewTab(view) => {
            let rect = app.sidebar_tabs().into_iter().find(|(_, v)| *v == view)?.0;
            let label = match (view, app.commit_count_text()) {
                (SidebarView::Commit, Some(count)) => {
                    let name = t!("workspace-tip-commit-changes", count = count);
                    with_key(&name, Some(Action::ShowCommit))
                }
                _ => view.label(),
            };
            Some((rect, label))
        }
        Hover::Panel(PanelButton::Tool(tool)) => {
            let parts = app.panel_parts();
            let rect = parts.tools.iter().find(|(_, t)| *t == tool)?.0;
            let label = match tool {
                PanelTool::Refresh => t!("workspace-search-tool-refresh"),
                PanelTool::Clear => t!("workspace-search-tool-clear"),
                PanelTool::CollapseAll if app.panels.global.all_collapsed() => {
                    t!("workspace-search-tool-expand-all")
                }
                PanelTool::CollapseAll => t!("workspace-search-tool-collapse-all"),
            };
            Some((rect, label.to_owned()))
        }
        Hover::Panel(
            button @ (PanelButton::Expand
            | PanelButton::Case
            | PanelButton::Word
            | PanelButton::Regex
            | PanelButton::ReplaceAll),
        ) => {
            // 提示文本沿用查找条上对应按钮的文本。
            let parts = app.panel_parts();
            let (rect, same) = match button {
                PanelButton::Expand => (parts.expand, SearchButton::Expand),
                PanelButton::Case => (parts.case_toggle, SearchButton::Case),
                PanelButton::Word => (parts.word_toggle, SearchButton::Word),
                PanelButton::Regex => (parts.regex_toggle, SearchButton::Regex),
                _ => (parts.replace_all, SearchButton::ReplaceAll),
            };
            Some((rect, same.label()))
        }
        Hover::Panel(PanelButton::Ignored) => Some((
            app.panel_parts().ignored_toggle,
            t!("workspace-search-include-ignored").to_owned(),
        )),
        Hover::Panel(PanelButton::Details) => {
            let label = if app.panels.global.details_open {
                t!("workspace-search-hide-filters")
            } else {
                t!("workspace-search-filter-files")
            };
            Some((app.panel_parts().details, label.to_owned()))
        }
        Hover::RowAction(index, action) => {
            let row = app.result_row_rect(index);
            let (replace, dismiss) = row_actions(row, app.panels.global.replace_open);
            let is_file = matches!(
                app.panels.global.rows().get(index),
                Some(ResultRow::File(_))
            );
            match action {
                RowAction::Replace => {
                    let label = if is_file {
                        t!("workspace-search-row-replace-all")
                    } else {
                        t!("workspace-search-row-replace")
                    };
                    Some((replace?, label.to_owned()))
                }
                RowAction::Dismiss => {
                    Some((dismiss, t!("workspace-search-row-dismiss").to_owned()))
                }
            }
        }
        _ => None,
    }
}
