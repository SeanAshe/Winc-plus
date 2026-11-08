//! Git 日志标签的绘制：顶上一行标题栏，左边提交列表（提交图、引用徽章、说明、作者、时间），
//! 右边一栏选中提交的详情（说明、作者、时间、提交号、父提交、改动的文件和增删行数）。
//!
//! 交互在 `app/git/log.rs`。列表一个提交一行，行高是编辑区的行高（逻辑像素），滚动走文档的
//! 通用逻辑；详情一栏自己滚。

mod badges;
mod details;
mod files;
mod graph;

use badges::*;
use details::*;
use files::*;
use graph::*;

use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

use winc_core::graph::GraphRow;
use winc_editor::git::log::LogView;
use winc_editor::git::{Change, Commit, Ref};
use winc_i18n::t;
use winc_render::Stroke;

use crate::app::Focus;
use crate::app::git::log::{
    LOG_LIST_PAD, LogParts, LogTarget, TextRun, details_top, display_time, is_relative_time,
    list_time, short,
};
use crate::icons::Direction;
use crate::keymap::{self, Action};
use crate::layout::editor_content_height;
use crate::view::workspace::sidebar::git_color;
use crate::view::*;

/// 提交图里相邻两条车道的间距（逻辑像素，经验值）。
const LANE: f32 = 12.0;
/// 提交图线条的粗细（逻辑像素，经验值）：几条车道并排时仍能区分，在浅色底上也能看清走向，
/// 又不比说明的文字笔画更重。
const GRAPH_LINE: f32 = 1.5;
/// 换车道的线拐弯处的圆角半径（逻辑像素，经验值）。
const CORNER: f32 = 6.0;
/// 拐弯处的圆弧用几段折线近似（段，经验值）：段数越多越圆滑，绘制开销也越大。
const CORNER_STEPS: usize = 6;
/// 选中一个提交时，不在选中提交那条线上的线、圆点保留的不透明度：仍能看清走向和颜色，
/// 选中的线又比它们突出。
const GRAPH_FADED: f32 = 0.55;
/// 提交图最多画的车道数（条，经验值）：更多的车道挤在最右边，说明不至于被挤没。
const MAX_LANES: usize = 10;
/// 作者一列的宽度（逻辑像素，经验值）。
const AUTHOR_W: f32 = 150.0;
/// 时间一列的宽度（逻辑像素，经验值）。
const DATE_W: f32 = 104.0;
/// 列表窄于这个宽度（逻辑像素，经验值）时作者一列只画头像（`AVATAR_W` 宽），不写名字，
/// 仍能认出提交者；宽度在 `AUTHOR_MIN_LIST - AUTHOR_BLEND` 到 `AUTHOR_MIN_LIST` 之间时，
/// 从只有头像渐渐展开成头像加名字。
const AUTHOR_MIN_LIST: f32 = 620.0;
/// 作者一列从只有头像过渡到头像加名字的宽度范围（逻辑像素，经验值）。
const AUTHOR_BLEND: f32 = 120.0;
/// 头像的边长（逻辑像素，经验值）。
const AVATAR_W: f32 = 26.0;
/// 一个引用徽章最多多宽（逻辑像素，经验值），更长的名字从末尾截短（详情里写全名）。
const BADGE_MAX: f32 = 140.0;
/// 截短的徽章至少多宽（逻辑像素，经验值），再窄就认不出是哪个了，不如收进「+N」。
const BADGE_MIN: f32 = 64.0;
/// 一行里的引用徽章最多占说明那一段的几成（0 到 1）：徽章多的提交（几个分支、远程、标签都
/// 指向它）不至于把说明挤没；放不下的收成「+N」。
const BADGE_SHARE: f32 = 0.45;
/// 相邻两个徽章之间、徽章和说明之间的间隔（逻辑像素，经验值）。
const BADGE_GAP: f32 = 6.0;
/// 提交图第几种颜色取调色板里的哪一种：相邻的颜色色相差得远（蓝、绿、红、紫……），并排的
/// 车道不会是红、橙、黄这样相近的几种；第一种（通常是主线）是蓝色。
const LANE_ORDER: [usize; 10] = [6, 3, 0, 7, 4, 1, 8, 5, 2, 9];
/// 详情一栏的左右留白（逻辑像素，经验值）。
const PAD: f32 = 18.0;
/// 详情里文件一行的高度（逻辑像素）。
const FILE_ROW: f32 = crate::app::git::log::LOG_FILE_ROW;

/// 绘制第 `index` 个文档（日志标签）。
///
/// # Panics
///
/// `index` 超出 `app.editor.documents` 的范围时 panic。
pub(in crate::view) fn draw_log(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    app.log.hits.clear();
    let id = app.editor.documents[index].id;
    let parts = app.log_parts();
    let lh = app.fonts.line_height;
    let (scroll, rows) = {
        let doc = &app.editor.documents[index];
        (doc.scroll_line, doc.row_count())
    };
    let first = scroll.floor().max(0.0) as usize;
    let visible = (parts.list.h / lh).ceil() as usize + 1;
    // 快滚到取回来的最后一个提交了：继续取下一批。
    app.log_near_end(id, first + visible);

    let mut hits = Vec::new();
    let mut clipped = None;
    let mut long_files = Vec::new();
    let mut runs = Vec::new();
    let details_bottom;
    let shown = app.editor.documents[index]
        .log
        .as_deref()
        .and_then(|view| shown_oid(app, view));
    app.note_log_details_shown(shown);
    {
        let p = app.palette;
        frame.rect(app.layout.editor, p.editor_bg);
    }
    // 筛选输入框要改输入框自己的排版状态（点击、拖选用），先单独画。
    app.sync_log_filter();
    let search = draw_search(app, frame, &parts);
    hits.push((search, LogTarget::Search));
    {
        let app = &*app;
        let Some(view) = app.editor.documents[index].log.as_deref() else {
            return;
        };
        draw_header(app, frame, view, &parts, search, &mut hits);
        draw_list(app, frame, view, &parts, scroll, &mut hits, &mut clipped);
        // 换成另一个提交时内容淡入（`App::note_log_details_shown`）。
        draw_details_footer(app, frame, view, &parts);
        let opacity = app.log.fade.map_or(1.0, |c| 0.4 + 0.6 * c.progress());
        details_bottom = frame.with_opacity(opacity, |frame| {
            draw_details(
                app,
                frame,
                view,
                &parts,
                (&mut hits, &mut long_files, &mut runs),
            )
        });
    }
    // 详情一栏最多滚到哪：内容底下留 16 逻辑像素，最下面那一行提示（30 逻辑像素高）不挡着
    // 最后一行。
    let details = parts.details;
    if let Some(view) = app.editor.documents[index].log.as_deref_mut() {
        let content = details_bottom + view.details_scroll - details.y + 16.0;
        let max = (content - (details.h - 30.0)).max(0.0);
        app.log.details_max = max;
        view.details_scroll = view.details_scroll.min(max);
    }
    app.log.hits = hits;
    app.log.clipped = clipped;
    app.log.long_files = long_files;
    app.log.runs = runs;

    // 列表的滚动条贴在列表的右边。
    let top = {
        let view = app.editor.documents[index].log.as_deref();
        let extra = view.is_some_and(|v| app.log_uncommitted(v) > 0);
        LOG_LIST_PAD + if extra { lh } else { 0.0 }
    };
    let content = editor_content_height(rows, parts.list.h, lh) + top;
    draw_scrollbar(
        app,
        frame,
        ScrollArea::Editor,
        id.0,
        parts.list,
        scroll * lh,
        content,
    );
}

/// 绘制标题栏左边的筛选输入框，返回它的位置（窗口坐标，逻辑像素）。
///
/// 与侧栏的搜索框共用 `field_box` 的画法：平时没有边框、铺一层淡底色，有焦点时外圈强调色。
fn draw_search(app: &mut App, frame: &mut Frame<'_>, parts: &LogParts) -> Rect {
    let p = app.palette;
    let header = parts.header;
    let search = Rect::new(header.x + 12.0, header.y + 6.0, 220.0, header.h - 12.0);
    let focused = app.focus == Focus::LogFilter;
    field_box(app, frame, search, p.editor_bg, focused, false);
    icons::magnifier(
        frame,
        (search.x + 14.0, search.y + search.h / 2.0),
        p.text_dim,
        search,
    );
    let area = Rect::new(search.x + 26.0, search.y, search.w - 32.0, search.h);
    let placeholder = match keymap::shortcut(Action::Find) {
        Some(key) => t!("git-log-filter-placeholder-key", key = key),
        None => t!("git-log-filter-placeholder").to_owned(),
    };
    let mut field = std::mem::take(&mut app.log.filter);
    let caret = draw_bar_field(app, frame, area, &mut field, &placeholder, focused);
    app.log.filter = field;
    if focused {
        app.ime_cursor = caret;
    }
    search
}

/// 标题栏：筛选输入框右边是「分支」「作者」两个下拉（筛着的时候铺淡强调色），文件历史还有一个
/// 写着文件名的小签，筛过时最后是「清除筛选」；右端是提交数和刷新按钮。
fn draw_header(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    parts: &LogParts,
    search: Rect,
    hits: &mut Vec<(Rect, LogTarget)>,
) {
    let p = app.palette;
    let header = parts.header;
    frame.rect(
        Rect::new(header.x, header.bottom() - 1.0, header.w, 1.0),
        p.separator,
    );
    let query = &view.query;
    let mut x = search.right() + 8.0;
    let pill_h = search.h;
    let y = search.y;

    // 两个下拉：「分支 全部」「作者 张伟」。
    let branch = query.branch.as_deref().unwrap_or(t!("git-log-filter-all"));
    let author = query.author.as_deref().unwrap_or(t!("git-log-filter-all"));
    for (target, name, value, on) in [
        (
            LogTarget::Branch,
            t!("git-log-filter-branch"),
            branch,
            query.branch.is_some(),
        ),
        (
            LogTarget::Author,
            t!("git-log-filter-author"),
            author,
            query.author.is_some(),
        ),
    ] {
        let name_w = frame.measure(FontKind::Ui, app.fonts.ui_small, name);
        let value_w = frame
            .measure(FontKind::Ui, app.fonts.ui_small, value)
            .min(160.0);
        let rect = Rect::new(x, y, 10.0 + name_w + 6.0 + value_w + 20.0, pill_h);
        let hovered = app.hover == Hover::Log(target);
        if on {
            frame.rounded(rect, p.accent.alpha(p.accent.a * 0.14), RADIUS_ROW);
        } else if hovered {
            frame.rounded(rect, p.hover, RADIUS_ROW);
        }
        let end = ui_text_small(frame, app, rect.x + 10.0, rect, name, p.text_dim, rect);
        let value_clip = Rect::new(end + 6.0, rect.y, value_w, rect.h);
        let ink = if on { p.accent } else { p.text_muted };
        ui_text_small(frame, app, end + 6.0, rect, value, ink, value_clip);
        let chevron = (rect.right() - 10.0, rect.y + rect.h / 2.0);
        icons::chevron(frame, chevron, 6.0, Direction::Down, ink, rect);
        hits.push((rect, target));
        x = rect.right() + 4.0;
    }

    // 文件历史：看的是哪个文件。
    if let Some(path) = &query.path {
        let shown = app
            .git
            .root
            .as_deref()
            .and_then(|r| path.strip_prefix(r).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let text = t!("git-log-filter-file", path = shown);
        let w = frame
            .measure(FontKind::Ui, app.fonts.ui_small, &text)
            .min(320.0)
            + 20.0;
        let rect = Rect::new(x + 4.0, y, w, pill_h);
        frame.rounded(rect, p.hover, RADIUS_ROW);
        ui_text_small(frame, app, rect.x + 10.0, rect, &text, p.text_muted, rect);
        x = rect.right() + 4.0;
    }

    // 右端：刷新按钮（边长 24 逻辑像素，经验值）；左边是提交数。
    let size = 24.0;
    let refresh = Rect::new(
        header.right() - 10.0 - size,
        header.y + (header.h - size) / 2.0,
        size,
        size,
    );
    let hovered = app.hover == Hover::Log(LogTarget::Refresh);
    draw_button(frame, p, refresh, header, hovered, false);
    icons::refresh(frame, center(refresh), p.text_muted, refresh);
    hits.push((refresh, LogTarget::Refresh));
    let count = view.commits.len();
    let more = if view.complete { "" } else { "+" };
    // 重取时列表仍是旧数据（`stale`），超过 `LOADING_DELAY`（300 ms）仍未取回才显示正在刷新；
    // 一般很快就返回，先显示提示再换回提交数会闪烁一次。
    let quiet = app.log_list_quiet();
    let note = if view.stale && !quiet {
        t!("git-log-refreshing").to_owned()
    } else if view.is_loading() && count == 0 {
        if quiet {
            String::new()
        } else {
            t!("git-log-loading").to_owned()
        }
    } else {
        t!("git-log-commit-count", count = count, more = more)
    };
    let note_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &note);
    let note_x = refresh.x - 10.0 - note_w;
    ui_text_small(frame, app, note_x, header, &note, p.text_dim, header);

    // 筛过时在左侧画「清除筛选」，与右端的提交数之间放得下才画。
    if query.filtered() {
        let text = t!("git-log-clear-filter");
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + 20.0;
        let rect = Rect::new(x + 4.0, y, w, pill_h);
        if rect.right() < note_x - 8.0 {
            let hovered = app.hover == Hover::Log(LogTarget::ClearFilters);
            if hovered {
                frame.rounded(rect, p.hover, RADIUS_ROW);
            }
            let ink = if hovered { p.text } else { p.accent };
            ui_text_small(frame, app, rect.x + 10.0, rect, text, ink, rect);
            hits.push((rect, LogTarget::ClearFilters));
        }
    }
}

/// 返回 `HEAD` 提交在提交图中的车道和颜色；该提交尚未读取（不在列表中）时为 `None`。
fn head_lane(view: &LogView) -> Option<(usize, u8)> {
    let graph = view.graph.get(view.head_row()?)?;
    Some((graph.lane, graph.color))
}

/// 绘制列表最上方的「未提交的更改（N）」一行；`y` 为该行上沿，文字从 `text_x` 开始。
///
/// 在提交图中 `HEAD` 的车道上绘制空心圆（`lane` 是提交图左边的 x 坐标与 `HEAD` 的车道、
/// 颜色）；`HEAD` 正是下方第一个提交时，以一段淡色线向下连接。点击该行打开提交视图。
fn draw_uncommitted(
    app: &App,
    frame: &mut Frame<'_>,
    (list, y, text_x): (Rect, f32, f32),
    (lane, head_first): (Option<(f32, usize, u8)>, bool),
    count: usize,
    hits: &mut Vec<(Rect, LogTarget)>,
) {
    let p = app.palette;
    let lh = app.fonts.line_height;
    let row = Rect::new(list.x, y, list.w, lh);
    if row.bottom() <= list.y {
        return;
    }
    let band = Rect::new(list.x + 4.0, y, list.w - 8.0, lh);
    let hovered = app.hover == Hover::Log(LogTarget::Uncommitted);
    if let Some(color) = row_color(p, false, false, hovered) {
        frame.rect_clipped(band, color, RADIUS_ROW, list);
    }
    if let Some((x0, lane, color)) = lane {
        let ink = lane_color(p, color);
        let cx = x0 + lane.min(MAX_LANES - 1) as f32 * LANE + LANE / 2.0;
        let cy = y + lh / 2.0;
        if head_first {
            let line = Rect::new(cx - GRAPH_LINE / 2.0, cy, GRAPH_LINE, lh);
            frame.rect_clipped(line, ink.alpha(ink.a * GRAPH_FADED), 0.0, list);
        }
        let r = 3.75; // 空心圆的半径（逻辑像素，经验值），与 `HEAD` 的圆点相近
        let dot = Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r);
        frame.rect_clipped(dot, ink, r, list);
        let inner = r - GRAPH_LINE;
        let hole = Rect::new(cx - inner, cy - inner, 2.0 * inner, 2.0 * inner);
        let bg = if hovered {
            crate::view::workspace::panel::over(p.editor_bg, p.hover)
        } else {
            p.editor_bg
        };
        frame.rect_clipped(hole, bg, inner, list);
    }
    let end = ui_text(
        frame,
        app,
        text_x,
        row,
        t!("git-log-uncommitted"),
        p.text_muted,
        list,
    );
    ui_text_small(
        frame,
        app,
        end + 6.0,
        row,
        &t!("git-log-uncommitted-files", count = count),
        p.text_dim,
        list,
    );
    hits.push((band.intersect(list), LogTarget::Uncommitted));
}

fn draw_list(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    parts: &LogParts,
    scroll: f32,
    hits: &mut Vec<(Rect, LogTarget)>,
    clipped: &mut Option<usize>,
) {
    let p = app.palette;
    let list = parts.list;
    let lh = app.fonts.line_height;
    if view.commits.is_empty() {
        // 与侧栏的空列表共用 `draw_empty_state`。
        let (title, detail) = match &view.error {
            Some(e) => (t!("git-log-error"), Some(e.as_str())),
            // 刚开始取时先空着，超过 `LOADING_DELAY`（300 ms）仍未返回才显示。
            None if view.is_loading() && app.log_list_quiet() => return,
            None if view.is_loading() => (t!("git-log-loading-commits"), None),
            None if view.query.filtered() => (
                t!("git-log-no-matches"),
                Some(t!("git-log-no-matches-detail")),
            ),
            None => (t!("git-log-empty"), Some(t!("git-log-empty-detail"))),
        };
        // 列表空着时详情一栏也没有内容可看（`draw_details` 不画），提示在整个标签里居中。
        let whole = Rect::new(list.x, list.y, parts.details.right() - list.x, list.h);
        draw_empty_state(app, frame, (whole, whole), title, detail);
        return;
    }

    let first = scroll.floor().max(0.0) as usize;
    let visible = (list.h / lh).ceil() as usize + 1;
    // 从上一行画起：第一行上面有 `LOG_LIST_PAD` 的留白，滚动时上一行的下半截还露在留白里。
    let rows = first.saturating_sub(1)..(first + visible).min(view.commits.len());
    // 提交图那一列的宽度按上下各多一屏的范围取：车道数随滚动变化，说明文字会左右跳，
    // 范围取大一些可以减少跳动。
    let around = first.saturating_sub(visible)..first + 2 * visible;
    let lanes = view.graph_width(around).min(MAX_LANES);
    let graph_x = list.x + 12.0;
    // 筛过说明、作者、文件时不画提交图（父提交大多不在列表里，图连不起来），说明从左边开始。
    let graphless = view.query.graphless();
    let text_x = if graphless {
        list.x + 16.0
    } else {
        graph_x + lanes as f32 * LANE + 8.0
    };
    // 作者一列随列表宽窄连续地变：窄于 `AUTHOR_MIN_LIST` 前后一段（`AUTHOR_BLEND`）里从只有
    // 头像渐渐展宽到头像加名字，名字跟着淡入。拖分隔线、缩窗口时一路跟着变，不在某个宽度上
    // 突然冒出来或消失。
    let show = ((list.w - (AUTHOR_MIN_LIST - AUTHOR_BLEND)) / AUTHOR_BLEND).clamp(0.0, 1.0);
    let date_x = list.right() - SCROLLBAR_HIT - DATE_W;
    let author_x = date_x - (AVATAR_W + (AUTHOR_W - AVATAR_W) * show);
    let subject_end = author_x - 12.0;
    let spill_end = list.right() - SCROLLBAR_HIT - 4.0;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // 键盘在详情的文件列表里时，选中的提交铺不带焦点的底色。
    let focused = app.focus == Focus::Editor && app.log.file.is_none();
    // 选中的提交所在的那条线：高亮这条线，别的线淡下去。比较两个版本时两头不在一条线上，
    // 不高亮。鼠标指着一条线时高亮指着的那条（点击后沿这条线跳转，先让人看清是哪条）。
    let line = match app.hover {
        Hover::Log(LogTarget::Line(_, l)) => Some(l),
        _ => view
            .selected
            .filter(|_| view.compare.is_none())
            .and_then(|i| view.graph.get(i))
            .map(|g| g.line),
    };

    // 工作区有改动时，第一个提交上方增加一行「未提交的更改」。
    let uncommitted = app.log_uncommitted(view);
    let top = LOG_LIST_PAD + if uncommitted > 0 { lh } else { 0.0 };
    if uncommitted > 0 {
        let y = list.y + LOG_LIST_PAD - scroll * lh;
        let lane = (!graphless)
            .then(|| head_lane(view).map(|(lane, color)| (graph_x, lane, color)))
            .flatten();
        let head_first = view.head_row() == Some(0);
        let graph = (lane, head_first);
        draw_uncommitted(app, frame, (list, y, text_x), graph, uncommitted, hits);
    }

    for i in rows {
        let commit = &view.commits[i];
        let y = list.y + top + (i as f32 - scroll) * lh;
        let row = Rect::new(list.x, y, list.w, lh);
        let band = Rect::new(list.x + 4.0, y, list.w - 8.0, lh);
        // 比较两个版本时两头都算选中。
        let selected = match &view.compare {
            Some(c) => rev_matches(commit, &c.from) || rev_matches(commit, &c.to),
            None => view.selected == Some(i),
        };
        let hovered =
            matches!(app.hover, Hover::Log(LogTarget::Row(r) | LogTarget::Avatar(r)) if r == i);
        let fill = row_color(p, selected, focused, hovered);
        if let Some(color) = fill {
            frame.rect_clipped(band, color, RADIUS_ROW, list);
        }

        if let Some(graph) = view.graph.get(i).filter(|_| !graphless) {
            draw_graph_row(
                app,
                frame,
                (graph, line),
                (commit, i),
                (graph_x, y, fill),
                list,
                hits,
            );
        }

        // 说明从提交图右边的同一列开始，引用徽章靠右排在说明那一段的末尾：上下各行的说明
        // 对齐，便于逐行浏览。徽章最多占说明那一段宽度的 `BADGE_SHARE`（45%），放不下的收成
        // 「+N」。
        let x = text_x;
        let room = ((subject_end - text_x) * BADGE_SHARE).max(0.0);
        let fitted = fit_badges(app, frame, &badges(p, commit), room);
        let badges_w = badges_width(app, frame, &fitted);
        let gap = if fitted.is_empty() {
            0.0
        } else {
            BADGE_GAP * 2.0
        };
        let merge = commit.parents.len() > 1;
        let color = if merge { p.text_muted } else { p.text };
        // 悬停在一行上、说明写不下时，说明一直铺到列表右边，盖掉这一行的作者和时间（详情里
        // 还有），徽章跟着挪到右边；还写不下的，悬停片刻后提示整句（`clipped`）。
        let full = frame.measure(FontKind::Ui, app.fonts.ui, &commit.subject);
        let spill = hovered && x + full + gap + badges_w > subject_end;
        let row_end = if spill { spill_end } else { subject_end };
        let badges_x = row_end - badges_w;
        let end = badges_x - gap;
        if spill && x + full > end {
            *clipped = Some(i);
        }
        let badge_clip = Rect::new(list.x, y, row_end - list.x, lh).intersect(list);
        draw_badges(app, frame, (badges_x, row), &fitted, badge_clip);
        let clip = Rect::new(list.x, y, end - list.x, lh).intersect(list);
        let room = (end - x).max(0.0);
        let subject = truncate_end(frame, &commit.subject, app.fonts.ui, room);
        // 按说明筛选时，说明中匹配的部分以查找结果的底色标出。
        for range in matches(&subject, view.query.text.trim()) {
            let font = app.fonts.ui;
            let left = x + frame.measure(FontKind::Ui, font, &subject[..range.start]);
            let w = frame.measure(FontKind::Ui, font, &subject[range]);
            let mark = Rect::new(left - 1.0, row.y + 5.0, w + 2.0, row.h - 10.0);
            frame.rect_clipped(mark, p.search_match, 3.0, clip);
        }
        ui_text(frame, app, x, row, &subject, color, clip);

        if !spill {
            draw_author_and_time(
                app,
                frame,
                commit,
                (author_x, date_x, row, show),
                now,
                list,
                hits,
                i,
            );
        }
        hits.push((band.intersect(list), LogTarget::Row(i)));
    }

    // 列表和详情之间的分隔线，可以左右拖：悬停、拖着的时候换成强调色的粗一点的线。
    // 能抓的范围往详情那边多留 4 逻辑像素：线左边紧挨着的是列表的滚动条。
    let divider = Rect::new(list.right() - 1.0, list.y, 5.0, list.h);
    let active = app.log_dividing() || app.hover == Hover::Log(LogTarget::Divider);
    if active {
        frame.rect(Rect::new(list.right() - 1.5, list.y, 2.0, list.h), p.accent);
    } else {
        frame.rect(
            Rect::new(list.right() - 1.0, list.y, 1.0, list.h),
            p.separator,
        );
    }
    // 命中范围放在命中表最前面，先于各行和详情判定。
    hits.insert(0, (divider, LogTarget::Divider));
}

/// 绘制一行右边的作者（头像加名字，或只有头像）和时间（靠右对齐）。
///
/// `show` 是名字显出来的程度（0 到 1），名字按它淡入；名字一栏宽的时候连名字一起画，
/// 窄的时候只有头像。时间用等宽字体，数字一位对一位，便于扫视。
#[allow(clippy::too_many_arguments)]
fn draw_author_and_time(
    app: &App,
    frame: &mut Frame<'_>,
    commit: &Commit,
    (author_x, date_x, row, show): (f32, f32, Rect, f32),
    now: i64,
    list: Rect,
    hits: &mut Vec<(Rect, LogTarget)>,
    i: usize,
) {
    let p = app.palette;
    let (y, lh) = (row.y, row.h);
    let clip = Rect::new(author_x, y, date_x - author_x - 8.0, lh).intersect(list);
    draw_avatar(app, frame, commit, (author_x + 8.0, y + lh / 2.0), clip);
    // 头像可点击的范围（逻辑像素）：16 宽、整行高，比头像本身大一点。
    let avatar = Rect::new(author_x, y, 16.0, lh).intersect(list);
    hits.push((avatar, LogTarget::Avatar(i)));
    if show > 0.0 {
        // 按展开以后的宽度截短，展开途中被这一列裁掉，不在半路上换「…」的位置。
        let name = truncate_end(frame, &commit.author, app.fonts.ui_small, AUTHOR_W - 32.0);
        let ink = p.text_muted.alpha(p.text_muted.a * show);
        ui_text_small(frame, app, author_x + 22.0, row, &name, ink, clip);
    }
    // 等宽字体、靠右对齐：一列时间数字一位对一位（`list_time` 只用数字和 `-`、`:`）。
    let time = list_time(commit, now);
    let clip = Rect::new(date_x, y, DATE_W, lh).intersect(list);
    let w = frame.measure(FontKind::Mono, app.fonts.ui_small, &time);
    mono(
        app,
        frame,
        (date_x + DATE_W - w, row),
        &time,
        p.text_dim,
        clip,
    );
}

/// 可以点的提交号的颜色：平时强调色，悬停时深一些。
fn link(p: &Palette, hovered: bool) -> Color {
    if hovered { p.text } else { p.accent }
}

/// 用等宽字体写一段文字（提交号），在 `row` 里垂直居中，返回末尾的 x（逻辑像素）。
pub(in crate::view) fn mono(
    app: &App,
    frame: &mut Frame<'_>,
    (x, row): (f32, Rect),
    text: &str,
    color: Color,
    clip: Rect,
) -> f32 {
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let size = app.fonts.ui_small;
    let scale = size / app.fonts.code;
    let baseline = baseline_in(row, ascent * scale, descent * scale);
    frame.text(FontKind::Mono, size, x, baseline, text, color, clip)
}

/// 把一段文字按宽度 `width`（逻辑像素）折成几行，返回各行的文字；折法见
/// [`crate::layout::wrap_text`]。
pub(in crate::view) fn wrap<'a>(
    frame: &mut Frame<'_>,
    size: f32,
    text: &'a str,
    width: f32,
) -> Vec<&'a str> {
    crate::layout::wrap_text(text, width, |s| frame.measure(FontKind::Ui, size, s))
}

#[cfg(test)]
mod tests;

/// 返回 `text` 中不区分大小写匹配 `needle` 的各段（字节范围，互不重叠）；`needle` 为空时返回空。
fn matches(text: &str, needle: &str) -> Vec<Range<usize>> {
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    // 从 `start` 起匹配时返回匹配段的末尾。
    let match_at = |start: usize| {
        let mut chars = text[start..].chars();
        let mut end = start;
        for n in needle.chars() {
            let c = chars.next().filter(|&c| same(c, n))?;
            end += c.len_utf8();
        }
        Some(end)
    };
    let mut found = Vec::new();
    let mut at = 0;
    while !needle.is_empty() && at < text.len() {
        match match_at(at) {
            Some(end) => {
                found.push(at..end);
                at = end;
            }
            None => at += text[at..].chars().next().map_or(1, char::len_utf8),
        }
    }
    found
}

/// 提交图第 `color` 种颜色（[`GraphRow::color`]、[`winc_core::graph::Segment::color`]）的线色。
fn lane_color(p: &Palette, color: u8) -> Color {
    let i = LANE_ORDER[color as usize % LANE_ORDER.len()];
    p.badges[i % p.badges.len()].0
}
