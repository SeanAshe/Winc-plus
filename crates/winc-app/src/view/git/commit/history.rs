//! 提交历史一组的绘制：提交图、说明、引用徽章、时间，以及尚未推送、尚未拉取的标记。

use winc_editor::git::recent::{Recent, Side};

use super::*;
use crate::app::{HistoryLine, HistoryTarget};
use crate::view::git::log::{
    BADGE_GAP, GraphStyle, LANE, badges, badges_width, draw_badges, draw_graph_row, fit_badges,
};

/// 提交图最多画的车道数（条）：侧栏较窄，更多的车道压在最右边一条上，给说明留出宽度。
const HISTORY_LANES: usize = 4;
/// 提交图第一条车道的中心与标题的箭头对齐：车道的左边离行左边的距离（逻辑像素）。
const GRAPH_LEFT: f32 = ARROW - LANE / 2.0;
/// 说明离提交图右边的距离（逻辑像素）：只有一条车道时说明与标题的文字对齐。
const SUBJECT_GAP: f32 = STASH_TITLE - GRAPH_LEFT - LANE;
/// 行右端推送、拉取标记占的宽度（逻辑像素）：图标约 10 宽，左右各留 2。
const MARK_ROOM: f32 = 14.0;
/// 时间离说明或徽章的最小距离（逻辑像素）。
const TIME_GAP: f32 = 8.0;
/// 引用徽章最多占说明区宽度的比例：侧栏较窄，说明优先，多的徽章收成「+N」。取自调参：三成时
/// 常见的分支名截短后仍可辨认，说明保留一半以上的宽度。
const HISTORY_BADGE_SHARE: f32 = 0.3;
/// 说明区窄于该宽度（逻辑像素）时不显示时间，先保证说明可读。取自调参：约六个汉字。
const TIME_MIN_ROOM: f32 = 140.0;

/// 绘制提交历史一组：上沿一条分隔线，标题是箭头和「提交历史」，悬停时右端是「在日志中显示」；
/// 展开时每个提交一行，正在查看的提交铺选中的底色，还有更早的提交时最后一行在日志中查看。行多于
/// 显示的行数时在组内滚动，上下两端有内容未显示时渐隐。
pub(super) fn draw_history(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let (Some(area), Some(recent)) = (parts.history, app.git.commit.history.recent.as_ref()) else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(area);
    frame.rect_clipped(
        Rect::new(area.x, area.y, area.w, 1.0),
        p.separator,
        0.0,
        clip,
    );
    let band = |y: f32| Rect::new(area.x + 8.0, y, area.w - 16.0, COMMIT_ROW);
    let shade = |band: Rect| Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);

    let head = band(area.y + crate::app::DRAWER_TOP);
    let title_target = CommitTarget::History(HistoryTarget::Title);
    let log_target = CommitTarget::History(HistoryTarget::Log);
    let on_title = matches!(
        app.pointer.hover,
        Hover::Commit(t) if t == title_target || t == log_target
    );
    if on_title {
        frame.rect_clipped(shade(head), p.hover, RADIUS_ROW, clip);
    }
    let cy = head.y + head.h / 2.0;
    let turned = app.history_chevron() * FRAC_PI_2;
    icons::chevron_turned(frame, (head.x + ARROW, cy), 6.0, turned, p.text_dim, clip);
    let size = app.fonts.ui_small;
    let at = (head.x + STASH_TITLE, head);
    label(
        frame,
        app,
        size,
        at,
        t!("git-history-group"),
        p.text_muted,
        clip,
    );
    if on_title {
        let button = end_slot(head);
        let on = app.pointer.hover == Hover::Commit(log_target);
        draw_button(frame, p, button, clip, on, false);
        let color = if on { p.text } else { p.text_muted };
        icons::history(frame, center(button), color, clip);
        hits.push((button, log_target));
    }
    hits.push((head, title_target));
    let moving = app.git.commit.history.drawer.motion.is_some();
    if !app.git.commit.history.drawer.open && !moving {
        return;
    }
    // 展开、收起的途中各行还没停稳，不响应点击。
    let mut still = Vec::new();
    let hits = if moving { &mut still } else { hits };
    let Some(rows) = app.history_rows_area() else {
        return;
    };
    let rows_clip = clip.intersect(rows);
    let style = GraphStyle {
        height: COMMIT_ROW,
        bg: p.sidebar_bg,
        lanes: HISTORY_LANES,
    };
    let lanes = (recent.graph.iter().map(|g| g.width).max().unwrap_or(1)).clamp(1, HISTORY_LANES);
    let graph_x = head.x + GRAPH_LEFT;
    let text_x = graph_x + lanes as f32 * LANE + SUBJECT_GAP;
    let marks = recent.sides.iter().any(|side| *side != Side::Both);
    let viewing = app.combined_commit().map(|c| c.oid.as_str());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    for (n, line) in app.history_lines().into_iter().enumerate() {
        let row = band(rows.y + n as f32 * COMMIT_ROW - app.git.commit.history.scroll);
        if row.bottom() <= rows_clip.y || row.y >= rows_clip.bottom() {
            continue;
        }
        let grab = row.intersect(rows_clip);
        match line {
            HistoryLine::Commit(i) => {
                let target = CommitTarget::History(HistoryTarget::Commit(i));
                let hovered = app.pointer.hover == Hover::Commit(target);
                let selected = viewing == Some(recent.commits[i].oid.as_str());
                let fill = row_color(p, selected, false, hovered);
                if let Some(color) = fill {
                    frame.rect_clipped(shade(row), color, RADIUS_ROW, rows_clip);
                }
                let cells = (graph_x, text_x, marks, now);
                draw_commit_row(
                    app,
                    frame,
                    (row, cells),
                    (recent, i),
                    (style, fill),
                    rows_clip,
                );
                hits.push((grab, target));
            }
            HistoryLine::More => {
                let target = CommitTarget::History(HistoryTarget::More);
                if app.pointer.hover == Hover::Commit(target) {
                    frame.rect_clipped(shade(row), p.hover, RADIUS_ROW, rows_clip);
                }
                let text = t!("git-history-more");
                ui_text_small(frame, app, text_x, row, text, p.text_muted, rows_clip);
                hits.push((grab, target));
            }
        }
    }
    draw_history_fades(app, frame, rows_clip);
}

/// 组内的行上下有内容未露出时，在 `rows` 的上沿、下沿画渐隐，表明可以滚动。
fn draw_history_fades(app: &App, frame: &mut Frame<'_>, rows: Rect) {
    use crate::view::workspace::top_bar::TAB_FADE;
    let height = TAB_FADE.min(rows.h / 2.0);
    if height <= 0.0 {
        return;
    }
    let scroll = app.git.commit.history.scroll;
    let below = app.history_max_scroll() - scroll;
    let color = app.palette.sidebar_bg;
    frame.within(rows, |frame| {
        let top = Rect::new(rows.x, rows.y, rows.w, height);
        let strength = (scroll / TAB_FADE).min(1.0);
        if strength > 0.0 {
            frame.with_opacity(strength, |frame| fade_down(frame, top, color));
        }
        let bottom = Rect::new(rows.x, rows.bottom() - height, rows.w, height);
        let strength = (below / TAB_FADE).min(1.0);
        if strength > 0.0 {
            frame.with_opacity(strength, |frame| fade_up(frame, bottom, color));
        }
    });
}

/// 绘制一个提交的一行：提交图、说明和引用徽章，右端是时间和推送、拉取标记。
///
/// `cells` 是提交图的左边、说明的左边、是否为标记留出一列，以及现在的 Unix 秒；`fill` 是行的
/// 底色（选中、悬停时），提交图中淡化的线与它混色。
fn draw_commit_row(
    app: &App,
    frame: &mut Frame<'_>,
    (row, (graph_x, text_x, marks, now)): (Rect, (f32, f32, bool, i64)),
    (recent, i): (&Recent, usize),
    (style, fill): (GraphStyle, Option<Color>),
    clip: Rect,
) {
    let p = app.palette;
    let commit = &recent.commits[i];
    let side = recent.sides.get(i).copied().unwrap_or(Side::Both);
    if let Some(graph) = recent.graph.get(i) {
        let mut ignored = Vec::new();
        let at = (style, graph_x, row.y, fill);
        draw_graph_row(
            app,
            frame,
            (graph, None, &[]),
            (commit, i),
            at,
            clip,
            &mut ignored,
        );
    }
    let mut right = row.right() - END_PAD;
    if marks {
        let cx = right - MARK_ROOM / 2.0;
        let cy = row.y + row.h / 2.0;
        match side {
            Side::Outgoing => icons::push_small(frame, (cx, cy), p.accent, clip),
            Side::Incoming => icons::pull_small(frame, (cx, cy), p.text_muted, clip),
            Side::Both => {}
        }
        right -= MARK_ROOM;
    }
    let size = app.fonts.ui_small;
    if right - text_x >= TIME_MIN_ROOM {
        let time = display_local(&commit.local, commit.time, now);
        let w = frame.measure(FontKind::Ui, size, &time);
        ui_text_small(frame, app, right - w, row, &time, p.text_dim, clip);
        right -= w + TIME_GAP;
    }
    let room = (right - text_x).max(0.0);
    let fitted = fit_badges(app, frame, &badges(p, commit), room * HISTORY_BADGE_SHARE);
    let badges_w = badges_width(app, frame, &fitted);
    let gap = if fitted.is_empty() { 0.0 } else { BADGE_GAP };
    let subject_room = (room - badges_w - gap).max(0.0);
    let subject = fit(frame, &commit.subject, app.fonts.ui, subject_room, false);
    // 尚未拉取的提交还不在工作区中，合并提交只是汇合：两者的说明都用次要色。
    let quiet = side == Side::Incoming || commit.parents.len() > 1;
    let color = if quiet { p.text_muted } else { p.text };
    let area = clip.intersect(Rect::new(text_x, row.y, subject_room, row.h));
    let end = label(
        frame,
        app,
        app.fonts.ui,
        (text_x, row),
        &subject,
        color,
        area,
    );
    if !fitted.is_empty() {
        draw_badges(app, frame, (end + gap, row), &fitted, clip);
    }
}

/// 详情卡片的宽度（逻辑像素）。取自调参：一行约 28 个汉字，说明的段落不至于折得过碎。
const CARD_W: f32 = 380.0;
/// 详情卡片的内边距（逻辑像素）。
const CARD_PAD: f32 = 12.0;
/// 详情卡片中一行文字的高度（逻辑像素），与悬停提示相同。
const CARD_LINE: f32 = 18.0;
/// 详情卡片中分隔线上下的留白（逻辑像素）。
const CARD_GAP: f32 = 8.0;
/// 说明最多显示的行数（行），更长时截断；完整的说明在日志中查看。取自调参：常见的说明不截断，
/// 卡片不超过侧栏高度的一半。
const CARD_MESSAGE_LINES: usize = 12;

/// 绘制鼠标停在第 `i` 个提交上时的详情卡片：作者和时间、完整的说明、改动的统计和短提交号，
/// 宽度随内容、不超过 [`CARD_W`]，从鼠标处开始，放在该行下方（下方空间不足时在上方）。说明和统计
/// 尚未读取完时先写说明的首行。
pub(in crate::view) fn draw_history_card(app: &App, frame: &mut Frame<'_>, i: usize, opacity: f32) {
    let Some((commit, _)) = app.git.commit.history.commit(i) else {
        return;
    };
    let target = CommitTarget::History(HistoryTarget::Commit(i));
    let Some(&(row, _)) = app.hits.commit.iter().find(|(_, t)| *t == target) else {
        return;
    };
    let p = app.palette;
    let size = app.fonts.ui_small;
    let window = app.layout.window;
    let room = CARD_W.min(window.w - 8.0) - 2.0 * CARD_PAD;
    let state = app.history_card(i);
    let card = match state {
        Some(crate::app::CardState::Ready(card)) => Some(card),
        _ => None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // 作者，相对时间（本机时间）；较早的提交只写本机时间。
    let relative = display_local(&commit.local, commit.time, now);
    let time = match relative == commit.local {
        true => relative,
        false => t!(
            "git-history-card-time",
            relative = relative,
            absolute = commit.local.as_str()
        ),
    };
    let byline = t!("git-history-card-byline", time = time);
    let message = card.map_or(commit.subject.as_str(), |c| c.message.as_str());
    let lines = card_lines(frame, size, message, room);
    let stats = card_stats(app, state);
    let short = crate::app::git::log::short(&commit.oid);
    let mut measure = |text: &str| frame.measure(FontKind::Ui, size, text);
    let mut content = measure(&commit.author) + measure(&byline);
    content = content.max(stats.iter().map(|(text, _)| measure(text)).sum());
    content = content.max(measure(short));
    for line in &lines {
        content = content.max(measure(line));
    }
    let width = content.min(room) + 2.0 * CARD_PAD;
    let message_h = CARD_LINE * lines.len() as f32;
    let height = 2.0 * CARD_PAD + 3.0 * CARD_LINE + message_h + 4.0 * CARD_GAP + 2.0;
    let below = row.bottom() + 6.0;
    let y = match below + height > window.bottom() {
        true => row.y - 6.0 - height,
        false => below,
    };
    let x = app.overlays.tooltip.pointer_x - 10.0;
    let right = (window.right() - width - 4.0).max(4.0);
    let rect = Rect::new(x.clamp(4.0, right), y.max(4.0), width, height);
    frame.with_opacity(opacity, |frame| {
        crate::view::draw_card(frame, p, rect, RADIUS_POPUP, p.popup_bg);
        let left = rect.x + CARD_PAD;
        let inner = width - 2.0 * CARD_PAD;
        let mut y = rect.y + CARD_PAD;
        let line = |y: f32| Rect::new(rect.x, y, rect.w, CARD_LINE);
        let x = ui_text_small(frame, app, left, line(y), &commit.author, p.text, rect);
        ui_text_small(frame, app, x, line(y), &byline, p.text_muted, rect);
        y += CARD_LINE + CARD_GAP;
        for text in &lines {
            ui_text_small(frame, app, left, line(y), text, p.text, rect);
            y += CARD_LINE;
        }
        y += CARD_GAP;
        let divider = |y: f32| Rect::new(left, y, inner, 1.0);
        frame.rect_clipped(divider(y), p.separator, 0.0, rect);
        y += 1.0 + CARD_GAP;
        let mut x = left;
        for (text, color) in &stats {
            x = ui_text_small(frame, app, x, line(y), text, *color, rect);
        }
        y += CARD_LINE + CARD_GAP;
        frame.rect_clipped(divider(y), p.separator, 0.0, rect);
        y += 1.0 + CARD_GAP;
        ui_text_small(frame, app, left, line(y), short, p.text_muted, rect);
    });
}

/// 把说明 `message` 按宽度 `room`（逻辑像素）折行；超过 [`CARD_MESSAGE_LINES`] 行时截断，
/// 最后一行末尾加省略号。空行保留，分开段落。
fn card_lines<'a>(
    frame: &mut Frame<'_>,
    size: f32,
    message: &'a str,
    room: f32,
) -> Vec<Cow<'a, str>> {
    let mut lines: Vec<Cow<'a, str>> = Vec::new();
    for paragraph in message.lines() {
        let measure = |text: &str| frame.measure(FontKind::Ui, size, text);
        let wrapped = crate::layout::wrap_text(paragraph, room, measure);
        match wrapped.is_empty() {
            true => lines.push(Cow::Borrowed("")),
            false => lines.extend(wrapped.into_iter().map(Cow::Borrowed)),
        }
    }
    if lines.len() > CARD_MESSAGE_LINES {
        lines.truncate(CARD_MESSAGE_LINES);
        let last = lines.pop().unwrap_or_default();
        lines.push(Cow::Owned(format!("{last}…")));
    }
    lines
}

/// 返回详情卡片中改动统计的各段文字和颜色：文件数、增加和删除的行数，后两项用差异的颜色；
/// 尚未读取完时是「正在读取」，读取失败时写明。
fn card_stats(app: &App, state: Option<&crate::app::CardState>) -> Vec<(String, Color)> {
    use crate::app::CardState;
    let p = app.palette;
    let card = match state {
        Some(CardState::Ready(card)) => card,
        Some(CardState::Failed) => {
            return vec![(t!("git-history-card-failed").to_owned(), p.text_muted)];
        }
        _ => {
            let text = t!("git-log-details-loading-files").to_owned();
            return vec![(text, p.text_muted)];
        }
    };
    let separator = t!("git-history-card-separator");
    vec![
        (t!("git-history-card-files", count = card.files), p.text),
        (separator.to_owned(), p.text_muted),
        (
            t!("git-history-card-added", count = card.added),
            p.git_added,
        ),
        (separator.to_owned(), p.text_muted),
        (
            t!("git-history-card-removed", count = card.removed),
            p.git_deleted,
        ),
    ]
}
