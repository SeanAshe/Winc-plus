//! 侧栏的提交视图：标题行与工具按钮、Git 上下文（位置、分支和同步）、说明条（合并、变基暂停时）、
//! 分组并按目录排成树的改动列表，以及底部的提交卡片（多行提交信息、「修改上次提交」、写明提交
//! 范围的「提交 ⌄」拆分按钮）；没有更改时空状态下面是下一步，底部是还没推送的上一次提交。
//!
//! 每个文件占一行：勾选框、文件图标、文件名（平铺时其后以淡色显示所在目录）、右端着色的
//! 状态字母；悬停时状态字母替换为「打开文件」「回滚」两个按钮。改动行不显示行数。
//! 坐标均为逻辑像素。

use std::borrow::Cow;
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::path::Path;

use winc_editor::git::{Change, Head, Operation, Status};
use winc_i18n::t;

use crate::app::animation::stage;
use crate::app::git::log::display_local;
use crate::app::{
    COMMIT_ROW, CommitGroup, CommitLabels, CommitName, CommitOverflow, CommitParts, CommitRow,
    CommitRows, CommitTarget, Focus, LabelCause, LabelView, MESSAGE_LINE, MESSAGE_PAD, StashLine,
    StashView, SyncState,
};
use crate::icons::Direction;
use crate::view::workspace::sidebar::{
    BADGE_CENTER, BADGE_ROOM, CHAIN_FOLD, CHAIN_GAP, CHAIN_SEPARATOR, ContextBar, FADE_FROM,
    NAME_FADE, fold_chain, git_color, retracting_color,
};
use crate::view::*;

/// Git 上下文按钮里文字右边到按钮右边的宽度（逻辑像素）：空隙加下拉箭头。
const CONTEXT_PICK_TAIL: f32 = 18.0;
/// 位置按钮里图标中心和文字左边离按钮左边的距离（逻辑像素）：图标 12 宽，左右各留 6。
const LOCATION_LEAD: (f32, f32) = (12.0, 24.0);
/// 项目按钮和位置按钮之间的距离（逻辑像素）。
const LOCATION_GAP: f32 = 4.0;
/// 同步控件里提交数左边的留白（逻辑像素）。
const SYNC_TEXT_LEFT: f32 = 8.0;
/// 「修改上次提交」的勾选框中心离那一段左边的距离（逻辑像素）：边长 12 的勾选框左边留 1，和
/// 悬停底色的边缘分开。
const AMEND_CHECK: f32 = 7.0;
/// 「修改上次提交」的文字离该区域左边的距离（逻辑像素）：勾选框右边再隔 8，与设置里勾选项中
/// 勾选框与文字的间隔相同。
const AMEND_TEXT: f32 = 21.0;
/// 「修改上次提交」文字右边留到悬停底色边缘的距离（逻辑像素），与左边勾选框外的空隙相当。
const AMEND_END: f32 = 6.0;
/// 没有更改时空状态和下面「推送」「拉取」按钮之间的距离（逻辑像素）：比说明的两行之间宽，
/// 看得出按钮不是说明的一部分。
const NEXT_GAP: f32 = 14.0;
/// 「推送」「拉取」按钮文字两侧的留白（逻辑像素），和三栏合并底部操作栏的按钮一样。
const NEXT_PAD: f32 = 12.0;

/// 绘制提交视图，返回提交信息里光标的矩形（窗口坐标，逻辑像素），供输入法定位候选框。
///
/// `tools` 是标题行右端工具按钮的不透明度：与历史差异文件列表切换时，按钮随之淡入。
pub(in crate::view) fn draw_commit(
    app: &mut App,
    frame: &mut Frame<'_>,
    tools: f32,
) -> Option<Rect> {
    // 提交信息框随内容长高：先按框的宽度折好行（框的宽度和高度无关，卡片和分支行同宽，卡片
    // 收起时也照样折），框的高度按行数定，绘制时继续使用折好的行。
    app.track_stash_height();
    let row = app.commit_parts().branch;
    let width = message_area(app, Rect::new(row.x, 0.0, row.w, 0.0)).w;
    let focused = app.focus == Focus::CommitMessage;
    let mut field = std::mem::take(&mut app.commit.message);
    let wrap = wrap_area_field(app, frame, width, &mut field, focused);
    app.commit.message = field;
    app.set_commit_message_lines(wrap.lines());
    // 底行的勾选框、按钮和「撤销」按文字宽度排：先量好，排版时就用上了。
    let size = app.fonts.ui_small;
    let (text, icon_w) = commit_button_text(app);
    app.set_commit_labels(CommitLabels {
        amend: AMEND_TEXT + frame.measure(FontKind::Ui, size, t!("git-commit-amend")) + AMEND_END,
        button: icon_w + frame.measure(FontKind::Ui, size, &text),
        undo: frame.measure(FontKind::Ui, size, t!("git-commit-undo")),
    });
    let parts = app.commit_parts();
    // 改动变少（提交、外部的 `git` 操作）或列表变高后，滚动位置可能超出内容，所有行都画不出来。
    // 展开、收起的动画中列表按动画结束时的行计算，由动画自己处理。
    if app.commit.motion.is_none() && app.git.status.is_some() {
        app.commit.scroll = app.commit.scroll.min(app.commit_max_scroll());
    }
    let mut hits = Vec::new();
    frame.with_opacity(tools, |frame| draw_header(app, frame, &parts, &mut hits));
    let animate = app.animate_commit_names();
    app.commit.names.begin_frame(animate);
    match (&app.git.status, &app.git.problem) {
        (Some(status), _) if app.commit.shift.is_some() => {
            draw_shift(app, frame, &parts, status, &mut hits);
        }
        (Some(status), _) if !status.entries.is_empty() => {
            draw_list(app, frame, &parts, status, &mut hits);
        }
        (Some(_), _) => draw_empty(app, frame, &parts, &mut hits),
        (None, Some(problem)) => {
            let text = t!("git-commit-view-problem", problem = problem.as_str());
            draw_note(app, frame, &parts, &text, true);
        }
        (None, None) => draw_note(app, frame, &parts, t!("git-commit-view-loading"), false),
    }
    app.commit.names.end_frame();
    // 列表的滚动条贴在列表右边，盖在固定的行上面；放得下时没有。
    let content = app.commit_max_scroll() + parts.list.h;
    let scroll = app.commit.scroll;
    draw_scrollbar(
        app,
        frame,
        ScrollArea::Commit,
        0,
        parts.list,
        scroll,
        content,
    );
    draw_context(app, frame, &parts, &mut hits);
    draw_banner(app, frame, &parts, &mut hits);
    draw_stashes(app, frame, &parts, &mut hits);
    draw_undo(app, frame, &parts, &mut hits);
    let caret = draw_compose(app, frame, &parts, wrap, &mut hits);
    app.hits.commit = hits;
    app.commit.hits_generation = app.git.generation;
    caret
}

/// 绘制浏览历史提交的文件列表（见 `App::with_log_list`）：只有改动列表，标题行由调用方绘制。
///
/// 借用提交视图时调用：列表的排列、写全名、固定在顶部的目录和各种过渡与提交视图相同。
pub(in crate::view) fn draw_commit_browse(app: &mut App, frame: &mut Frame<'_>) {
    let parts = app.commit_parts();
    if app.commit.motion.is_none() && app.git.status.is_some() {
        app.commit.scroll = app.commit.scroll.min(app.commit_max_scroll());
    }
    let mut hits = Vec::new();
    let animate = app.animate_commit_names();
    app.commit.names.begin_frame(animate);
    if let Some(status) = &app.git.status {
        if app.commit.shift.is_some() {
            draw_shift(app, frame, &parts, status, &mut hits);
        } else {
            draw_list(app, frame, &parts, status, &mut hits);
        }
    }
    app.commit.names.end_frame();
    let content = app.commit_max_scroll() + parts.list.h;
    let scroll = app.commit.scroll;
    let owner = app.editor.active().map_or(0, |doc| doc.id.0);
    draw_scrollbar(
        app,
        frame,
        ScrollArea::Commit,
        owner,
        parts.list,
        scroll,
        content,
    );
    app.hits.commit = hits;
    app.commit.hits_generation = app.git.generation;
}

/// 只绘制标题行右端的工具按钮，不透明度为 `opacity`，不记录点击区域：切换到历史差异文件列表
/// 时，按钮在原位淡出。
pub(in crate::view) fn draw_commit_tools(app: &mut App, frame: &mut Frame<'_>, opacity: f32) {
    let parts = app.commit_parts();
    let mut ignored = Vec::new();
    frame.with_opacity(opacity, |frame| {
        draw_header(app, frame, &parts, &mut ignored)
    });
}

/// 绘制第二行右端的三个按钮（左边是共用的项目和分支）：「Git 日志」「刷新」「⋯」。
///
/// 读取期间「刷新」的图标旋转，至少转满一圈（见 `App::commit_refreshing`）。「⋯」的下拉菜单
/// 里有同步、储藏、远程仓库和显示方式。
fn draw_header(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let header = parts.header;
    let p = app.palette;
    for (rect, target) in parts.tools {
        let open = target == CommitTarget::More
            && app.menu_button == Some(crate::app::MenuButton::CommitMore);
        let hovered = app.hover == Hover::Commit(target);
        draw_button(frame, p, rect, header, hovered, open);
        let color = if hovered || open {
            p.text
        } else {
            p.text_muted
        };
        let c = center(rect);
        match target {
            CommitTarget::Log => icons::history(frame, c, color, header),
            CommitTarget::Refresh if app.commit_refreshing() => {
                icons::refresh_turned(frame, c, spin_turn(), color, header);
            }
            CommitTarget::Refresh => icons::refresh(frame, c, color, header),
            _ => icons::more(frame, c, color, header),
        }
        hits.push((rect, target));
    }
}

/// 绘制 Git 上下文：项目名后面的位置按钮（只有一个位置时没有）和分支行。
///
/// 提交之前要确认的事都在这里：写在哪个仓库和工作树、在哪个分支、和上游差多少。切换用的面板从
/// 下面展开，下一步要做的同步就在分支行右端。分支行图标对着项目徽章，文字对着项目名，比项目名
/// 小一号；分支是提交前最要确认的，用正文色，位置淡一档。
fn draw_context(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let clip = app.layout.sidebar;
    if let Some(span) = location_span(app, frame) {
        draw_location_row(app, frame, parts, span, clip, hits);
    }
    draw_branch_row(app, frame, parts.branch, clip, hits);
}

/// 绘制 Git 上下文一行左边的按钮：图标、名字和下拉箭头，宽度贴合文字。
///
/// `(center, room)` 是图标中心离按钮左边的距离和文字的左边距。`names` 是名字的各段和各自的颜色；
/// 不超过 `right`，放不下时从末尾截短。
#[allow(clippy::too_many_arguments)]
fn draw_context_pick(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    (icon_at, room_left): (f32, f32),
    right: f32,
    target: CommitTarget,
    open: bool,
    icon: impl FnOnce(&mut Frame<'_>, (f32, f32), Color),
    names: &[(&str, Color)],
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let room = (right - row.x - room_left - CONTEXT_PICK_TAIL).max(0.0);
    let full: f32 = names
        .iter()
        .map(|(text, _)| frame.measure(FontKind::Ui, size, text))
        .sum();
    let pick = Rect::new(
        row.x,
        row.y,
        room_left + full.min(room) + CONTEXT_PICK_TAIL,
        row.h,
    );
    let hovered = app.hover == Hover::Commit(target);
    draw_button(frame, p, pick, clip, hovered, open);
    hits.push((pick, target));
    let lit = hovered || open;
    let cy = icon_center_y(app, row, size);
    icon(
        frame,
        (pick.x + icon_at, cy),
        if lit { p.text } else { p.text_muted },
    );
    let mut x = pick.x + room_left;
    let limit = x + room;
    for (text, color) in names {
        let shown = fit(frame, text, size, (limit - x).max(0.0), false);
        let color = if lit { p.text } else { *color };
        x = label(frame, app, size, (x, row), &shown, color, clip);
        // 截短了就到此为止：后面的段再写只会多一个省略号。
        if shown.len() != text.len() {
            break;
        }
    }
    icons::chevron(frame, (x + 9.0, cy), 5.5, Direction::Down, p.text_dim, clip);
}

/// 项目名后面的位置按钮的左右边：有多个位置时才有，放在项目按钮和工具按钮之间。
///
/// 放得下时紧贴项目按钮；放不下时位置按钮最多让到一半，再由项目名截短。
pub(in crate::view) fn location_span(app: &App, frame: &mut Frame<'_>) -> Option<(f32, f32)> {
    if !app.git_has_locations() {
        return None;
    }
    let bar = ContextBar::new(app, frame);
    let right = app.commit_parts().tools[0].0.x - 4.0;
    let (repository, worktree) = app.git_location_label();
    let mut text = frame.measure(FontKind::Ui, app.fonts.ui_small, &repository);
    if let Some(worktree) = worktree.as_deref() {
        text += frame.measure(FontKind::Ui, app.fonts.ui_small, &format!(" › {worktree}"));
    }
    let width = LOCATION_LEAD.1 + text + CONTEXT_PICK_TAIL;
    let room = (right - bar.left() - LOCATION_GAP).max(0.0);
    let project = bar.project_full();
    if project + width <= room {
        let left = bar.left() + project + LOCATION_GAP;
        return Some((left, left + width));
    }
    Some((right - width.min((room - project).max(room / 2.0)), right))
}

/// 绘制项目名后面的位置按钮：仓库，打开的是附属工作树时后面写工作树。
///
/// 其他位置有未提交的更改时，文件夹图标右上角有一个强调色的小圆点，和视图标签上的圆点同一个
/// 意思；几个写在悬停提示里，是哪几个在位置选择器里。
fn draw_location_row(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    (left, right): (f32, f32),
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let (repository, worktree) = app.git_location_label();
    let mut names = vec![(repository.as_str(), p.text_muted)];
    if let Some(worktree) = worktree.as_deref() {
        names.push((" › ", p.text_dim));
        names.push((worktree, p.text_muted));
    }
    let open = app.picker_lists_locations();
    let lit = open || app.hover == Hover::Commit(CommitTarget::Location);
    let dotted = app.git_other_changes() > 0;
    let tool = parts.tools[0].0;
    let row = Rect::new(left, tool.y, (right - left).max(0.0), tool.h);
    draw_context_pick(
        app,
        frame,
        row,
        LOCATION_LEAD,
        right,
        CommitTarget::Location,
        open,
        |frame, (x, y), color| {
            icons::folder_small(frame, (x, y), color, clip);
            if dotted {
                let dot = Rect::new(x + 3.5, y - 7.0, 5.0, 5.0);
                let ring = dot.inset(-1.0, -1.0);
                frame.rect_clipped(ring, p.sidebar_bg, 3.5, clip);
                if lit {
                    frame.rect_clipped(ring, p.hover, 3.5, clip);
                }
                frame.rect_clipped(dot, p.accent, 2.5, clip);
            }
        },
        &names,
        clip,
        hits,
    );
}

/// 绘制分支行：分支名，右端是同步控件。
fn draw_branch_row(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let mut right = row.right();
    // 没有远程时没有东西可同步；但操作的进度和结果仍然显示在这里。
    let active = app.branch.running.is_some() || app.branch.finished.is_some();
    if app.git.status.is_some() && (active || app.git_sync_state() != SyncState::NoRemote) {
        right = draw_sync(app, frame, row, clip, hits) - BUTTON_GAP;
    }
    let head = app
        .git
        .status
        .as_ref()
        .and_then(|s| s.head.as_ref())
        .map(|head| match head {
            Head::Branch(name) => name.clone(),
            Head::Detached(oid) => format!("({oid})"),
        });
    let head = head.unwrap_or_default();
    draw_context_pick(
        app,
        frame,
        row,
        (BADGE_CENTER, BADGE_ROOM),
        right,
        CommitTarget::Branch,
        app.branch_picker_from(crate::app::GitAnchor::Branch),
        |frame, center, color| icons::branch_small(frame, center, color, clip),
        &[(head.as_str(), p.text)],
        clip,
        hits,
    );
}

/// 绘制分支行右端的同步控件，返回它的左边缘。
///
/// 图标和标题行的工具按钮在同一列，左边写和上游差多少个提交。有事可做时用强调色，其余时候用
/// 淡色；运行时图标在原位旋转，成功后过渡到对勾，再过渡到当前状态的图标。要做什么写在提示里。
fn draw_sync(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> f32 {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let state = app.git_sync_state();
    let counts = match state {
        SyncState::Push(ahead) => format!("↑{ahead}"),
        SyncState::Pull(behind) => format!("↓{behind}"),
        SyncState::Both { ahead, behind } => format!("↓{behind} ↑{ahead}"),
        SyncState::Publish | SyncState::InSync | SyncState::NoRemote | SyncState::Hidden => {
            String::new()
        }
    };
    let width = if counts.is_empty() {
        BUTTON_SMALL
    } else {
        SYNC_TEXT_LEFT + frame.measure(FontKind::Ui, size, &counts) + BUTTON_SMALL
    };
    let button = Rect::new(row.right() - width, row.y, width, row.h);
    let running = app.branch.running.is_some();
    let hovered = !running && app.hover == Hover::Commit(CommitTarget::Sync);
    draw_button(frame, p, button, clip, hovered, false);
    let done = app.branch.finished.filter(|_| !running);
    // 前半段换下的是刚停下的旋转图标，它接着转；后半段换上的是当前状态的图标。
    let leaving = running || done.is_some_and(|clock| clock.linear() < 0.5);
    let check = if running { 0.0 } else { app.sync_check() };
    let ink = if leaving {
        p.text_muted
    } else if !counts.is_empty() || state == SyncState::Publish {
        p.accent
    } else if hovered {
        p.text
    } else {
        p.text_muted
    };
    if !counts.is_empty() {
        let x = button.x + SYNC_TEXT_LEFT;
        ui_text_small(frame, app, x, button, &counts, ink, clip);
    }
    let icon = (
        button.right() - BUTTON_SMALL / 2.0,
        button.y + button.h / 2.0,
    );
    let faded = ink.alpha(ink.a * (1.0 - check));
    // 同步与远程交换提交，图标与标题行只重读本地的「刷新」不同。
    if leaving {
        icons::sync_turned(frame, icon, spin_turn(), faded, clip);
    } else if state == SyncState::Publish {
        icons::upload(frame, icon, faded, clip);
    } else {
        icons::sync(frame, icon, faded, clip);
    }
    if check > 0.0 {
        icons::check(frame, icon, 11.0, p.git_added.alpha(check), clip);
    }
    hits.push((button, CommitTarget::Sync));
    button.x
}

/// 绘制合并、变基等暂停时提交信息上方的说明条。
///
/// 说明条显示进行中的操作和仍有冲突的文件数，右端为「中止」。仍有冲突时底色和图标使用冲突色；
/// 冲突全部解决后改用淡强调色，提示用户可以提交。
fn draw_banner(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let (Some((rect, abort)), Some(status)) = (parts.banner, app.git.status.as_ref()) else {
        return;
    };
    let Some(operation) = status.operation else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(rect);
    let unresolved = status.unresolved.len();
    let conflicted = status
        .entries
        .iter()
        .any(winc_editor::git::Entry::conflicted);
    let tint = if unresolved > 0 {
        p.git_conflict
    } else {
        p.accent
    };
    frame.rounded(rect, tint.alpha(0.12), RADIUS_ROW);
    let op = operation.label();
    let text = match (unresolved, conflicted) {
        (0, true) => t!("git-operation-banner-resolved", operation = op),
        (0, false) => t!("git-operation-banner", operation = op),
        (n, _) => t!("git-operation-banner-conflicts", operation = op, count = n),
    };
    let size = app.fonts.ui_small;
    let mut x = rect.x + 8.0;
    if unresolved > 0 {
        icons::warning(frame, (x + 6.0, icon_center_y(app, rect, size)), tint, clip);
        x += 18.0;
    }
    let room = Rect::new(rect.x, rect.y, abort.x - 4.0 - rect.x, rect.h);
    ui_text_small(frame, app, x, rect, &text, p.text, clip.intersect(room));
    let hovered = app.hover == Hover::Commit(CommitTarget::Abort);
    draw_button(frame, p, abort, clip, hovered, false);
    let w = frame.measure(FontKind::Ui, size, t!("git-operation-abort"));
    let color = if hovered { p.text } else { p.text_muted };
    let text_x = (abort.x + (abort.w - w) / 2.0).round();
    ui_text_small(
        frame,
        app,
        text_x,
        abort,
        t!("git-operation-abort"),
        color,
        clip,
    );
    hits.push((abort, CommitTarget::Abort));
}

/// 上一次提交（视图底部，有提交卡片时在卡片上面）：上沿一条分隔线，左边是提交信息的第一行，
/// 右边淡色写提交时间，最右端是「撤销」和「⋯」（修改上一次提交、复制提交号）。
///
/// 只在上一次提交还没推送时出现（见 `App::commit_undoable`）；拉开、收起的途中按栏露出的高度
/// 裁掉。
fn draw_undo(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let (Some((bar, undo, more)), Some(last)) = (parts.undo, app.commit_undo_shown()) else {
        return;
    };
    // 收起途中（已经推送、撤销）不能再撤销，不响应点击。
    let mut leaving = Vec::new();
    let hits = if app.commit_undoable().is_some() {
        hits
    } else {
        &mut leaving
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(bar);
    frame.rect_clipped(Rect::new(bar.x, bar.y, bar.w, 1.0), p.separator, 0.0, clip);
    let left = parts.branch.x;
    let row = Rect::new(left, more.y, more.right() - left, more.h);
    let size = app.fonts.ui_small;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let time = display_local(&last.local, last.time, now);
    let time_w = frame.measure(FontKind::Ui, size, &time);
    let time_x = (undo.x - 6.0 - time_w).max(row.x);
    ui_text_small(frame, app, time_x, row, &time, p.text_dim, clip);
    let room = Rect::new(row.x, row.y, (time_x - 8.0 - row.x).max(0.0), row.h);
    let subject = fit(frame, &last.subject, size, room.w - 2.0, false);
    let subject_clip = clip.intersect(room);
    ui_text_small(
        frame,
        app,
        row.x + 2.0,
        row,
        &subject,
        p.text_muted,
        subject_clip,
    );

    // 「撤销」写成字：最常用的一项，放进菜单里看不出能撤销。
    let hovered = app.hover == Hover::Commit(CommitTarget::Undo);
    draw_button(frame, p, undo, clip, hovered, false);
    let text = t!("git-commit-undo");
    let w = frame.measure(FontKind::Ui, size, text);
    let x = (undo.x + (undo.w - w) / 2.0).round();
    ui_text_small(frame, app, x, undo, text, p.accent, clip);
    hits.push((undo, CommitTarget::Undo));

    let hovered = app.hover == Hover::Commit(CommitTarget::LastCommit);
    let open = app.menu_button == Some(crate::app::MenuButton::LastCommit);
    draw_button(frame, p, more, clip, hovered, open);
    let color = if hovered || open {
        p.text
    } else {
        p.text_muted
    };
    icons::more(frame, center(more), color, clip);
    hits.push((more, CommitTarget::LastCommit));
}

/// 储藏一组的标题和各条储藏的名字离底色左边的距离（逻辑像素）：标题紧跟在箭头后面，和改动列表组
/// 标题的箭头同一列。各条储藏和标题的文字对齐：行前面没有勾选框和图标，再缩进一层只会留下
/// 一块空白。
const STASH_TITLE: f32 = ARROW + 10.0;
/// 正在查看的储藏下面各个文件的图标离行左边的距离（逻辑像素）：比储藏名再缩进一级。
const STASH_FILE: f32 = STASH_TITLE + 12.0;
/// 储藏那一行悬停时「应用」文字两侧的留白（逻辑像素），和「上一次提交」那一栏的「撤销」相同。
const STASH_APPLY_PAD: f32 = 6.0;

/// 绘制改动列表下面的储藏一组：上沿一条分隔线；标题是箭头、「储藏」和数量；展开时下面每条储藏一行，
/// 右端淡色写储藏时的分支和时间，悬停时换成「应用」和「⋯」；正在预览标签里看的那一条底色同选中。
/// 储藏太多时最后一行是「还有 N 条」。
fn draw_stashes(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let Some(area) = parts.stashes else {
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
    let size = app.fonts.ui_small;
    let band = |row: usize| {
        let y = area.y + crate::app::STASH_GAP + row as f32 * COMMIT_ROW;
        Rect::new(area.x + 8.0, y, area.w - 16.0, COMMIT_ROW)
    };
    let shade = |band: Rect| Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);

    // 标题：和改动列表的组标题一样，小一号的次要色字，右端一个写数量的徽标。
    let head = band(0);
    if app.hover == Hover::Commit(CommitTarget::Stashes) {
        frame.rect_clipped(shade(head), p.hover, RADIUS_ROW, clip);
    }
    let cy = head.y + head.h / 2.0;
    let turned = app.stash_chevron() * FRAC_PI_2;
    icons::chevron_turned(frame, (head.x + ARROW, cy), 6.0, turned, p.text_dim, clip);
    let title = t!("git-stash-group");
    label(
        frame,
        app,
        size,
        (head.x + STASH_TITLE, head),
        title,
        p.text_muted,
        clip,
    );
    draw_count_pill(app, frame, head, &app.git.stashes.len().to_string(), clip);
    hits.push((head, CommitTarget::Stashes));
    // 展开、收起的途中各行跟着标题移动，露出多少由一组的范围裁掉；这时行还没停稳，不响应点击。
    let moving = app.commit.stash_motion.is_some();
    if !app.commit.stash_open && !moving {
        return;
    }
    let mut still = Vec::new();
    let hits = if moving { &mut still } else { hits };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let viewing = (app.commit.stash_view.as_ref())
        .filter(|_| app.editor.active().map(|d| d.id) == app.commit.stash_preview)
        .map(|view| view.oid.as_str());
    let listed = app.listed_stash_view();
    for (n, line) in app.stash_lines().into_iter().enumerate() {
        let row = band(n + 1);
        let i = match line {
            StashLine::Stash(i) => i,
            StashLine::File(at) => {
                if let Some(view) = listed {
                    draw_stash_file(app, frame, row, view, at, clip);
                    hits.push((row, CommitTarget::StashFile(at)));
                }
                continue;
            }
            StashLine::MoreFiles(count) => {
                let text = t!("git-stash-more-files", count = count);
                let x = row.x + STASH_FILE;
                ui_text_small(frame, app, x, row, &text, p.text_dim, clip);
                continue;
            }
            StashLine::MoreStashes(rest) => {
                if app.hover == Hover::Commit(CommitTarget::StashMore) {
                    frame.rect_clipped(shade(row), p.hover, RADIUS_ROW, clip);
                }
                let text = t!("git-stash-more", count = rest);
                let x = row.x + STASH_TITLE;
                ui_text_small(frame, app, x, row, &text, p.text_muted, clip);
                hits.push((row, CommitTarget::StashMore));
                continue;
            }
        };
        let stash = &app.git.stashes[i];
        let hovered = matches!(
            app.hover,
            Hover::Commit(
                CommitTarget::Stash(h) | CommitTarget::StashApply(h) | CommitTarget::StashMenu(h)
            ) if h == i
        );
        let selected = viewing == Some(stash.oid.as_str());
        if let Some(color) = row_color(p, selected, false, hovered) {
            frame.rect_clipped(shade(row), color, RADIUS_ROW, clip);
        }
        // 右端：平时是分支和时间，悬停时换成「应用」和「⋯」。
        let mut right = row.right() - END_PAD;
        if hovered {
            let (_, more) = file_actions(row);
            let on = app.hover == Hover::Commit(CommitTarget::StashMenu(i));
            draw_button(frame, p, more, clip, on, false);
            let color = if on { p.text } else { p.text_muted };
            icons::more(frame, center(more), color, clip);
            hits.push((more, CommitTarget::StashMenu(i)));
            let text = t!("git-stash-apply");
            let w = frame.measure(FontKind::Ui, size, text) + 2.0 * STASH_APPLY_PAD;
            let apply = Rect::new(more.x - BUTTON_GAP - w, more.y, w, more.h);
            let on = app.hover == Hover::Commit(CommitTarget::StashApply(i));
            draw_button(frame, p, apply, clip, on, false);
            ui_text_small(
                frame,
                app,
                apply.x + STASH_APPLY_PAD,
                apply,
                text,
                p.accent,
                clip,
            );
            hits.push((apply, CommitTarget::StashApply(i)));
            right = apply.x - 6.0;
        } else {
            let time = display_local(&stash.local, stash.time, now);
            let meta = match stash.branch() {
                Some(branch) => format!("{branch} · {time}"),
                None => time,
            };
            let room = (right - row.x - STASH_TITLE) / 2.0;
            let meta = fit(frame, &meta, size, room.max(0.0), false);
            let w = frame.measure(FontKind::Ui, size, &meta);
            ui_text_small(frame, app, right - w, row, &meta, p.text_dim, clip);
            right -= w + 8.0;
        }
        let x = row.x + STASH_TITLE;
        let name = crate::app::git::branch::stash_title(stash);
        let name = fit(frame, &name, app.fonts.ui, (right - x).max(0.0), false);
        let room = clip.intersect(Rect::new(row.x, row.y, (right - row.x).max(0.0), row.h));
        label(frame, app, app.fonts.ui, (x, row), &name, p.text, room);
        hits.push((row, CommitTarget::Stash(i)));
    }
}

/// 绘制正在查看的储藏下面的一个文件：比储藏名再缩进一级，依次是文件图标、文件名、淡色的所在
/// 目录，右端是状态字母；正在查看的文件铺选中的底色。
fn draw_stash_file(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    view: &StashView,
    at: usize,
    clip: Rect,
) {
    let p = app.palette;
    let Some((file, ..)) = view.files.get(at) else {
        return;
    };
    let hovered = app.hover == Hover::Commit(CommitTarget::StashFile(at));
    let shade = Rect::new(row.x, row.y + 1.0, row.w, row.h - 2.0);
    if let Some(color) = row_color(p, at == view.at, false, hovered) {
        frame.rect_clipped(shade, color, RADIUS_ROW, clip);
    }
    let size = app.fonts.ui_small;
    let cy = row.y + row.h / 2.0;
    let name = winc_editor::display_name(&file.path).into_owned();
    let x = row.x + STASH_FILE;
    draw_file_icon(frame, p, (x, cy), &name, clip);
    let letter = file.change.letter().to_string();
    let letter_w = frame.measure(FontKind::Ui, size, &letter);
    let right = row.right() - END_PAD;
    let ink = git_color(p, file.change);
    ui_text_small(frame, app, right - letter_w, row, &letter, ink, clip);
    let text_x = x + icons::FILE_WIDTH + 6.0;
    let room = (right - letter_w - 8.0 - text_x).max(0.0);
    let name_w = frame.measure(FontKind::Ui, app.fonts.ui, &name).min(room);
    let shown = fit(frame, &name, app.fonts.ui, room, false);
    let color = file_color(file.change, p);
    let area = clip.intersect(Rect::new(text_x, row.y, room, row.h));
    label(frame, app, app.fonts.ui, (text_x, row), &shown, color, area);
    // 所在目录相对仓库根目录，写在名字后面，放不下时截短。
    let dir = app.editor.root().and_then(|root| {
        let parent = file.path.parent()?.strip_prefix(root).ok()?;
        let dir = parent.to_string_lossy().into_owned();
        (!dir.is_empty()).then_some(dir)
    });
    if let Some(dir) = dir {
        let dir_x = text_x + name_w + 6.0;
        let dir_room = (text_x + room - dir_x).max(0.0);
        let dir = fit(frame, &dir, size, dir_room, false);
        ui_text_small(frame, app, dir_x, row, &dir, p.text_dim, clip);
    }
}

/// 没有改动时列表区的空状态：「没有要提交的更改」，下面是当前分支与上游的
/// 关系（已同步、领先、落后、还没发布），有上游时再一行淡色写上游的名字（很长时截短，不和关系
/// 挤在一行里折断）；有事可做时最下面是一个强调色的按钮（推送、拉取、同步或发布），只写动作。
///
/// 正在获取、拉取、推送时按钮灰掉，进度在分支行右端的同步控件上。
fn draw_empty(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let list = parts.list;
    let (label, detail, upstream) = next_step(app);
    let bottom = draw_empty_state(
        app,
        frame,
        (list, sidebar_span(app)),
        t!("git-commit-view-empty"),
        Some(&detail),
    );
    let mut bottom = bottom;
    if let Some(upstream) = upstream {
        let size = app.fonts.ui_small;
        let row = Rect::new(list.x, bottom, list.w, 22.0);
        // 两边和上面的说明一样各留 16，放不下时从中间截短，两端的远程名和分支名都留着。
        let shown = fit(frame, upstream, size, (list.w - 32.0).max(0.0), true);
        let w = frame.measure(FontKind::Ui, size, &shown);
        let x = list.x + ((list.w - w) / 2.0).max(16.0);
        ui_text_small(frame, app, x, row, &shown, p.text_dim, list);
        bottom = row.bottom();
    }
    let Some(label) = label else {
        return;
    };
    let w = (frame.measure(FontKind::Ui, app.fonts.ui, &label) + 2.0 * NEXT_PAD).min(list.w);
    let rect = Rect::new(
        (list.x + (list.w - w) / 2.0).round(),
        bottom + NEXT_GAP,
        w,
        SETTINGS_CONTROL_H,
    );
    if rect.bottom() > list.bottom() {
        return;
    }
    let enabled = app.branch.running.is_none();
    let hovered = app.hover == Hover::Commit(CommitTarget::NextStep);
    draw_action_button(
        app,
        frame,
        rect,
        &label,
        ButtonRole::Primary,
        enabled,
        hovered,
    );
    if enabled {
        hits.push((rect, CommitTarget::NextStep));
    }
}

/// 没有改动时的下一步：按钮上的动作（和上游一致、没有远程或这时不同步时为 `None`）、写在标题
/// 下面的当前分支与上游的关系，以及上游的名字（没有上游时为 `None`）。
fn next_step(app: &App) -> (Option<String>, String, Option<&str>) {
    let upstream = (app.git.branches.iter())
        .find(|b| b.current)
        .and_then(|b| b.upstream.as_deref());
    let (action, relation) = match app.git_sync_state() {
        SyncState::Push(count) => (
            Some(t!("git-commit-next-push")),
            t!("git-commit-empty-ahead", count = count),
        ),
        SyncState::Pull(count) => (
            Some(t!("git-commit-next-pull")),
            t!("git-commit-empty-behind", count = count),
        ),
        SyncState::Both { ahead, behind } => (
            Some(t!("git-commit-next-both")),
            t!("git-commit-empty-diverged", ahead = ahead, behind = behind),
        ),
        SyncState::Publish => (
            Some(t!("git-commit-next-publish")),
            t!("git-commit-empty-unpublished").to_owned(),
        ),
        SyncState::InSync => (None, t!("git-commit-empty-in-sync").to_owned()),
        SyncState::NoRemote | SyncState::Hidden => {
            return (None, t!("git-commit-empty-local").to_owned(), None);
        }
    };
    (action.map(str::to_owned), relation, upstream)
}

/// 列表的位置上写一行说明（没有改动、正在读取、出错的原因）。
///
/// 出错时用警告色，前面一个警告图标，放不下时折成几行。
fn draw_note(app: &App, frame: &mut Frame<'_>, parts: &CommitParts, text: &str, warn: bool) {
    let p = app.palette;
    let list = parts.list;
    let x = list.x + 16.0;
    let room = (list.right() - 16.0 - x - if warn { 18.0 } else { 0.0 }).max(40.0);
    let color = if warn { p.warning } else { p.text_dim };
    // 按宽度折行：逐个字符量，放不下就换行。
    let mut lines = vec![String::new()];
    let mut width = 0.0;
    for c in text.chars() {
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, c.encode_utf8(&mut [0; 4]));
        if width + w > room && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
            width = 0.0;
        }
        width += w;
        if let Some(line) = lines.last_mut() {
            line.push(c);
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let row = Rect::new(list.x, list.y + 4.0 + i as f32 * 20.0, list.w, 20.0);
        let mut text_x = x;
        if warn && i == 0 {
            icons::warning(frame, (x + 6.0, row.y + row.h / 2.0), p.warning, list);
            text_x += 18.0;
        } else if warn {
            text_x += 18.0;
        }
        label(
            frame,
            app,
            app.fonts.ui_small,
            (text_x, row),
            line,
            color,
            list,
        );
    }
}

/// 画列表各行时共用的状态：每一行都要读取，一帧只取一次。
struct ListContext<'a> {
    list: Rect,
    status: &'a Status,
    /// 仓库的根目录。
    root: &'a Path,
    /// 画的是哪一版列表行（展开收起的动画期间是动画中的那一版）。
    rows: &'a CommitRows,
    /// 是否有多个文件被选中：目录行下面的文件全部选中时，目录行才画成选中状态。
    ///
    /// 每行是否选中按 `App::commit_is_selected` 查集合：几千个改动、多选时，逐项线性查找
    /// 会在一帧里做上千万次比较。
    multi: bool,
}

/// 绘制改动列表：按目录排成树（或者平铺），滚动时所在的组、目录固定在顶部。
fn draw_list(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    status: &Status,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let list = parts.list;
    let root = app.git.root.as_deref().unwrap_or(Path::new(""));
    let multi = app.commit_multi_selected();

    // 组、目录正在展开或收起：按动画中的那一版绘制。子行的位置不动，只露出已经露出来的那一段，
    // 下面的行随之上下移动。被展开、收起的行及其上方各行位置不变，行号在动画前后相同，仍记录
    // 命中区域，悬停底色不因动画中断；其余各行在动画期间不记录。
    if let Some(motion) = &app.commit.motion {
        let cx = ListContext {
            list,
            status,
            root,
            rows: &motion.rows,
            multi,
        };
        let (block, hidden) = (motion.block(), motion.hidden());
        for v in 0..motion.rows.len() {
            let (rect, clip) =
                crate::layout::tree_row_slot(list, app.commit.scroll, v, Some(block), hidden);
            let clip = clip.intersect(list);
            if clip.h <= 0.0 || rect.bottom() < list.y || rect.y > list.bottom() {
                continue;
            }
            let mut row_hits = Vec::new();
            frame.within(clip, |frame| {
                draw_row(app, frame, &cx, v, rect.y, &mut row_hits);
            });
            if v <= motion.parent {
                hits.extend(row_hits);
            }
        }
        // 固定的行仍然盖在上面（展开收起的是它们下面的行），否则动画过程中会先消失再出现。
        let mut ignored = Vec::new();
        draw_sticky(app, frame, &cx, &mut ignored);
        return;
    }

    let rows = app.commit_rows();
    let cx = ListContext {
        list,
        status,
        root,
        rows: &rows,
        multi,
    };
    let first = (app.commit.scroll / COMMIT_ROW).floor() as usize;
    let visible = (list.h / COMMIT_ROW).ceil() as usize + 1;

    for i in (first..rows.len()).take(visible) {
        let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll;
        draw_row(app, frame, &cx, i, y, hits);
    }

    draw_sticky(app, frame, &cx, hits);
}

/// 刷新以后各行过渡中的一帧（见 `CommitShift`）：离开的行按刷新前的状态画，和收起目录一样高度
/// 收到 0、同时淡出；新出现的行像展开一样长出来、淡入；留下的行跟着上下移动。列表空了、或从空
/// 变成有时，空状态同时淡入、淡出。
///
/// 过渡只有一百多毫秒，其间各行都在移动，不记录命中区域，不画固定在顶部的行。
fn draw_shift(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    status: &Status,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let Some(shift) = &app.commit.shift else {
        return;
    };
    let list = parts.list;
    let root = app.git.root.as_deref().unwrap_or(Path::new(""));
    let multi = app.commit_multi_selected();
    let rows = app.commit_rows();
    let old = ListContext {
        list,
        status: &shift.status,
        root,
        rows: &shift.rows,
        multi,
    };
    let new = ListContext {
        rows: &rows,
        status,
        ..old
    };
    let t = shift.clock.progress();
    let after: HashMap<RowId<'_>, usize> = (0..rows.len())
        .map(|j| (row_id(status, &rows[j]), j))
        .collect();
    let before: HashMap<RowId<'_>, usize> = (0..shift.rows.len())
        .map(|i| (row_id(&shift.status, &shift.rows[i]), i))
        .collect();

    // 两个列表为空时空状态一个淡出、一个淡入，另一边的行同时出现或离开。
    let mut ignored = Vec::new();
    match (shift.rows.is_empty(), rows.is_empty()) {
        (true, false) => frame.with_opacity(1.0 - t, |frame| {
            draw_empty(app, frame, parts, &mut ignored);
        }),
        (false, true) => frame.with_opacity(t, |frame| draw_empty(app, frame, parts, hits)),
        _ => {}
    }

    // 把两版的行按顺序并成一列：留下的行按新的顺序；离开的行排在它在旧列表里后面那一个留下的
    // 行之前。离开的行高度从一行收到 0，新出现的从 0 长到一行，留下的行跟着上下移动，彼此不重叠。
    let mut slots: Vec<(Slot, f32)> = Vec::with_capacity(rows.len() + shift.rows.len());
    let mut next_old = 0;
    let mut leave_until = |slots: &mut Vec<(Slot, f32)>, until: usize| {
        while next_old < until.min(shift.rows.len()) {
            let id = row_id(&shift.status, &shift.rows[next_old]);
            if !after.contains_key(&id) {
                slots.push((Slot::Old(next_old), 1.0 - t));
            }
            next_old += 1;
        }
    };
    for j in 0..rows.len() {
        match before.get(&row_id(status, &rows[j])) {
            Some(&i) => {
                leave_until(&mut slots, i);
                slots.push((Slot::New(j), 1.0));
            }
            None => slots.push((Slot::New(j), t)),
        }
    }
    leave_until(&mut slots, shift.rows.len());

    let scroll = shift.scroll + (app.commit.scroll - shift.scroll) * t;
    let mut y = list.y - scroll;
    for (slot, share) in slots {
        let h = COMMIT_ROW * share;
        let room = Rect::new(list.x, y, list.w, h).intersect(list);
        if room.h > 0.0 {
            let (cx, i) = match slot {
                Slot::Old(i) => (&old, i),
                Slot::New(j) => (&new, j),
            };
            frame.within(room, |frame| {
                frame.with_opacity(share, |frame| draw_row(app, frame, cx, i, y, &mut ignored));
            });
        }
        y += h;
    }
}

/// [`draw_shift`] 里并起来的一格：刷新前的第几行（离开的），或者刷新后的第几行。
#[derive(Clone, Copy)]
enum Slot {
    Old(usize),
    New(usize),
}

/// 改动列表里一行是谁，刷新前后用它对上同一行：组、目录（连同所在的组）、文件。文件换了组
/// （暂存以后）还算同一行，从原来的位置移过去。
#[derive(PartialEq, Eq, Hash)]
enum RowId<'a> {
    Group(CommitGroup),
    Dir(CommitGroup, &'a Path),
    File(&'a Path),
}

/// 状态 `status` 下第 `row` 行的 [`RowId`]。
fn row_id<'a>(status: &'a Status, row: &'a CommitRow) -> RowId<'a> {
    match row {
        CommitRow::Group(group) => RowId::Group(*group),
        CommitRow::Dir { group, dir, .. } => RowId::Dir(*group, dir),
        CommitRow::File { index, .. } => RowId::File(
            status
                .entries
                .get(*index)
                .map_or(Path::new(""), |e| &e.path),
        ),
    }
}

/// 固定在顶部的组、目录：盖住下面滚过去的行，下沿一道细线。
///
/// 点击时先滚到那一行再收起或展开，勾选框仍可点击；命中范围记在 `hits` 最前面，先于
/// 下面的行判定。被下一个组、目录往上推的那一行只画在它自己那一格里，从上一层固定的行
/// 下面滑走，不盖住上一层，否则滚动时上下两行会叠在一起。
fn draw_sticky(
    app: &App,
    frame: &mut Frame<'_>,
    cx: &ListContext<'_>,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let list = cx.list;
    let sticky = app.commit_sticky(cx.rows);
    let mut pinned = Vec::new();
    for s in &sticky {
        let slot = Rect::new(
            list.x,
            list.y + s.level as f32 * COMMIT_ROW,
            list.w,
            COMMIT_ROW,
        )
        .intersect(list);
        let mut row_hits = Vec::new();
        frame.within(slot, |frame| {
            frame.rect_clipped(s.rect, p.sidebar_bg, 0.0, list);
            if app.hover == Hover::Commit(CommitTarget::Sticky(s.row, s.level)) {
                let band = Rect::new(
                    list.x + 8.0,
                    s.rect.y + 1.0,
                    list.w - 16.0,
                    COMMIT_ROW - 2.0,
                );
                frame.rect_clipped(band, p.hover, RADIUS_ROW, list);
            }
            draw_row(app, frame, cx, s.row, s.rect.y, &mut row_hits);
        });
        for (rect, target) in row_hits {
            let target = match target {
                CommitTarget::Group(_) | CommitTarget::Dir(_) => {
                    CommitTarget::Sticky(s.row, s.level)
                }
                other => other,
            };
            let rect = rect.intersect(slot);
            if rect.h > 0.0 {
                pinned.push((rect, target));
            }
        }
    }
    if let Some(bottom) = sticky.iter().map(|s| s.rect.bottom()).reduce(f32::max) {
        let line = Rect::new(list.x, bottom - 1.0, list.w, 1.0);
        frame.rect_clipped(line, p.separator, 0.0, list);
        // 固定的各行下面被盖住的行点不到。
        let covered = Rect::new(list.x, list.y, list.w, bottom - list.y);
        hits.retain(|(rect, _)| !covered.contains(rect.x + 1.0, rect.y + rect.h / 2.0));
    }
    hits.splice(0..0, pinned);
}

/// 画改动列表里的第 `i` 行，上沿在 `y`（逻辑像素）；记下可点击的位置。
fn draw_row(
    app: &App,
    frame: &mut Frame<'_>,
    cx: &ListContext<'_>,
    i: usize,
    y: f32,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let ListContext {
        list,
        status,
        root,
        rows,
        multi,
    } = *cx;
    let row = &rows[i];
    let focused = app.focus == Focus::CommitList;
    // 浏览历史提交的文件时没有勾选框，也没有回滚、打开文件等按钮。
    let browse = app.commit.browse;
    let checkbox = |frame: &mut Frame<'_>, x: f32, cy: f32, state: Option<bool>| {
        if browse {
            return;
        }
        let border = p.text_dim.alpha(0.55);
        let on_accent = p.text_on_accent;
        icons::checkbox(frame, (x, cy), state, p.accent, border, on_accent, list);
    };
    // 勾选框的命中区域；浏览时没有勾选框，不记录。
    let check_hit = |hits: &mut Vec<(Rect, CommitTarget)>, rect: Rect, target: CommitTarget| {
        if !browse {
            hits.push((rect.intersect(list), target));
        }
    };
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    // 鼠标停着的这一行名字放不下：写全名，状态字母让开；伸出侧栏时底色铺到侧栏边缘、右边是
    // 直角，和伸出去的那一段（`draw_commit_overflow`）连成一条。
    let full = app
        .commit
        .overflow
        .filter(|o| o.row == i && (o.y - y).abs() < 0.5);
    let spill = full.and_then(|o| o.spill);
    // 名字写全名还是截短，按行判断，不看位置：固定在顶部的行和列表里被它盖住的那一行一样。
    let wanted = app.commit.overflow.is_some_and(|o| o.row == i);
    let cause = LabelCause {
        hovered: wanted,
        levels: 0,
        current: 0,
    };
    // 底色上下各让出 1 逻辑像素：相邻两行都有底色（选中的和悬停的）时中间留一道缝。
    let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
    // 名字收回途中还伸在侧栏外面时，底色同样铺到侧栏边缘。
    let fill = |frame: &mut Frame<'_>, color: Color, wide: bool| {
        if spill.is_some() || wide {
            let r = RADIUS_ROW;
            let wide = Rect::new(shade.x, shade.y, list.right() - shade.x, shade.h);
            frame.rect_corners(wide, color, [r, 0.0, 0.0, r], list);
        } else {
            frame.rect_clipped(shade, color, RADIUS_ROW, list);
        }
    };
    let cy = y + COMMIT_ROW / 2.0;
    // 箭头、勾选框的列（勾选框按中心）：每深一层往右缩进 `INDENT`，目录的箭头在勾选框左边。
    let arrow_x = |level: usize| band.x + ARROW + level as f32 * INDENT;
    let check_x = |level: usize| arrow_x(level) + check_gap(app);
    let text_clip = Rect::new(list.x, y, band.right() - 6.0 - list.x, COMMIT_ROW).intersect(list);
    // 缩进参考线：每一层上级目录一条，位于该目录的箭头正下方，相邻行连成一条竖线，
    // 目录层级深时能看出每个文件属于哪个目录。
    if let CommitRow::Dir { depth, .. } | CommitRow::File { depth, .. } = row {
        for level in 1..=*depth {
            let x = arrow_x(level).round();
            frame.rect_clipped(Rect::new(x, y, 1.0, COMMIT_ROW), p.indent_guide, 0.0, list);
        }
    }

    match row {
        CommitRow::Group(group) => {
            let group = *group;
            let hovered = matches!(
                app.hover,
                Hover::Commit(
                    CommitTarget::Group(g) | CommitTarget::GroupCheck(g) | CommitTarget::GroupRevert(g)
                ) if g == group
            );
            if hovered {
                frame.rect_clipped(shade, p.hover, RADIUS_ROW, list);
            }
            let open = !app.commit.group_collapsed(group);
            chevron(app, frame, (arrow_x(0), cy), (i, open), p.text_dim, list);
            let group_check = check_x(0);
            let members = rows.group(group);
            let checks = app.commit.checks();
            let state = rows.members_state(members, checks, || {
                app.commit.members_state(status, members)
            });
            checkbox(frame, group_check, cy, state);
            // 组标题：小一号的次要色字，右端一个写文件数的小圆角徽标。
            label(
                frame,
                app,
                app.fonts.ui_small,
                (group_check + ICON, band),
                group.title(),
                p.text_muted,
                list,
            );
            // 悬停时右端的文件数换成「回滚」，回滚这一组里勾上的文件，位置和文件行悬停时的按钮
            // 相同。冲突组没有：回滚冲突的文件会丢掉解决到一半的内容。
            let revert = (hovered && !browse && group != CommitGroup::Conflicts && state.is_some())
                .then(|| file_actions(band).1);
            match revert {
                Some(rect) => {
                    let target = CommitTarget::GroupRevert(group);
                    let on = app.hover == Hover::Commit(target);
                    draw_button(frame, p, rect, list, on, false);
                    let color = if on { p.text } else { p.text_muted };
                    icons::rollback(frame, center(rect), color, list);
                    hits.push((rect.intersect(list), target));
                }
                None => draw_count_pill(app, frame, band, &members.len().to_string(), list),
            }
            let check = Rect::new(group_check - 8.0, y + 4.0, 16.0, 16.0);
            check_hit(hits, check, CommitTarget::GroupCheck(group));
            hits.push((band.intersect(list), CommitTarget::Group(group)));
        }
        CommitRow::Dir {
            group,
            dir,
            label: name,
            depth,
        } => {
            // 组标题已经写了文件数，展开后每个文件都看得见，每层再写一遍是重复信息。
            let level = depth + 1;
            let hovered = matches!(
                app.hover,
                Hover::Commit(CommitTarget::Dir(r) | CommitTarget::DirCheck(r)) if r == i
            );
            // 多选时下面的文件都选中了：画成选中状态。
            let members = rows.dir(*group, dir);
            // 选中的文件比目录下面的少时不可能都选中，不必逐个查。
            let is_selected = multi
                && !members.is_empty()
                && app.commit_marked_count() >= members.len()
                && members
                    .iter()
                    .all(|&m| app.commit_is_selected(&status.entries[m].path));
            let icon_x = check_x(level) + ICON;
            let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
            let room = (!wanted).then(|| (text_clip.right() - text_x).max(0.0));
            let parts = dir_parts(app, frame, name, room);
            let width = parts_width(app, frame, &parts, true);
            let key = CommitName::Dir(*group, dir.clone());
            let view = app
                .commit
                .names
                .sync(&key, cause, &parts, shown_width(width, room));
            let retracting =
                !wanted && view.progress < 1.0 && text_x + view.width > text_clip.right();
            let color = if retracting {
                retracting_color(p, is_selected, focused, view.progress)
            } else {
                row_color(p, is_selected, focused, hovered || full.is_some())
            };
            if let Some(color) = color {
                fill(frame, color, retracting);
            }
            let open = !app.commit.dir_collapsed(*group, dir);
            chevron(
                app,
                frame,
                (arrow_x(level), cy),
                (i, open),
                p.text_dim,
                list,
            );
            let dir_check = check_x(level);
            let checks = app.commit.checks();
            let state = rows.members_state(members, checks, || {
                app.commit.members_state(status, members)
            });
            checkbox(frame, dir_check, cy, state);
            // 目录和文件的图标在同一列（宽 `FOLDER_WIDTH`）里居中，名字从同一处开始，
            // 两种行排得整齐。
            icons::folder(frame, (icon_x, (cy - 6.0).round()), open, p.folder, list);
            // 写全名、收回途中名字画到过渡中的宽度为止，侧栏外的部分由 `draw_commit_overflow`
            // 接着画。
            let name_clip = if wanted || retracting {
                let w = (text_x + view.width - list.x).max(0.0);
                Rect::new(list.x, y, w, COMMIT_ROW).intersect(list)
            } else {
                text_clip
            };
            draw_name(
                app,
                frame,
                (text_x, band, true, p.text),
                (&parts, &view),
                name_clip,
            );
            if let Some(spill) = spill {
                // 显示全名的这一行覆盖列表右边的滚动条（见 `draw_commit_overflow`）：行底色右边到
                // 侧栏边缘的区域同样属于这一行，鼠标移入时全名保持显示。
                let edge = Rect::new(band.right(), y, list.right() - band.right(), COMMIT_ROW);
                hits.push((edge, CommitTarget::Dir(i)));
                hits.push((spill, CommitTarget::Dir(i)));
            }
            let check = Rect::new(dir_check - 8.0, y + 4.0, 16.0, 16.0);
            check_hit(hits, check, CommitTarget::DirCheck(i));
            hits.push((band.intersect(list), CommitTarget::Dir(i)));
        }
        CommitRow::File { index, depth } => {
            let index = *index;
            let entry = &status.entries[index];
            let is_selected = app.commit_is_selected(&entry.path);
            let hovered = matches!(
                app.hover,
                Hover::Commit(
                    CommitTarget::File(i)
                        | CommitTarget::Check(i)
                        | CommitTarget::OpenFile(i)
                        | CommitTarget::RevertFile(i)
                ) if i == index
            );
            // 悬停时右端的状态字母替换为「打开文件」「回滚」两个按钮；文件名放不下而显示全名时，
            // 按钮跟在全名之后（见 `commit_overflow`）。
            let actions = match full {
                Some(overflow) => overflow.actions,
                None => (hovered && !browse).then(|| file_actions(band)),
            };
            // 文件没有箭头：勾选框和同一层目录的勾选框对齐；平铺时没有层级，和组标题的勾选框
            // 对齐。
            let file_check = check_x(file_column(app, *depth));
            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let icon_x = file_check + ICON;
            let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
            let dir = (app.commit.flat())
                .then(|| entry.path.parent().and_then(|p| p.strip_prefix(root).ok()))
                .flatten()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_string_lossy());
            // 名字用正文色（删除的文件用次要色），状态是右端一个上色的字母（M、U、D 等）；
            // 整行名字上色后颜色过多，不易分辨。
            let change = entry.shown();
            let letter = change.letter().to_string();
            let letter_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &letter);
            let letter_x = band.right() - END_CENTER - letter_w / 2.0;
            // 文件名及平铺时的目录截短到 `text_end` 为止。写全名时按钮跟着名字展开的宽度移动。
            let text_end = actions.map_or(letter_x - 8.0, |(open, _)| open.x - 4.0);
            let room = (!wanted).then(|| (text_end - text_x).max(0.0));
            let parts = file_parts(app, frame, (&name, dir.as_deref()), room);
            let width = parts_width(app, frame, &parts, false);
            let key = CommitName::File(entry.path.clone());
            let view = app
                .commit
                .names
                .sync(&key, cause, &parts, shown_width(width, room));
            let lag = match actions {
                Some((open, _)) if wanted => actions_lag(app, i, band, open, width - view.width),
                _ => 0.0,
            };
            let actions = actions.map(|(open, revert)| {
                let back = |r: Rect| Rect::new(r.x - lag, r.y, r.w, r.h);
                (back(open), back(revert))
            });
            let retracting = !wanted && view.progress < 1.0 && text_x + view.width > text_end;
            let shade_color = if retracting {
                retracting_color(p, is_selected, focused, view.progress)
            } else {
                row_color(p, is_selected, focused, hovered || full.is_some())
            };
            if let Some(color) = shade_color {
                fill(
                    frame,
                    color,
                    retracting && text_x + view.width > text_clip.right(),
                );
            }
            checkbox(frame, file_check, cy, app.commit.file_state(entry));
            // 未跟踪的整个目录在 Git 的输出里只有一项，画目录图标。文件图标在图标那一列里居中。
            if entry.is_dir {
                icons::folder(frame, (icon_x, (cy - 6.0).round()), false, p.folder, list);
            } else {
                let inset = (icons::FOLDER_WIDTH - icons::FILE_WIDTH) / 2.0;
                draw_file_icon(frame, p, (icon_x + inset, cy), &name, list);
            }
            if let Some(actions) = actions {
                draw_file_actions(app, frame, actions, index, list);
                // 伸出侧栏的按钮同样可以点击，命中区域不裁到列表内。
                let (open, revert) = actions;
                for (rect, target) in [
                    (open, CommitTarget::OpenFile(index)),
                    (revert, CommitTarget::RevertFile(index)),
                ] {
                    let rect = if full.is_some() {
                        rect
                    } else {
                        rect.intersect(list)
                    };
                    hits.push((rect, target));
                }
            } else if full.is_none() {
                // 收回途中名字还盖着状态字母那一块：字母随过渡淡入，不和名字叠在一起。
                let ink = git_color(p, change);
                let ink = if retracting {
                    ink.alpha(ink.a * view.progress)
                } else {
                    ink
                };
                ui_text_small(frame, app, letter_x, band, &letter, ink, list);
            }
            let color = file_color(change, p);
            // 名字先于按钮绘制：按钮滑向名字后面的过程中，名字随按钮移开而逐渐显示。
            let name_end = if wanted || retracting {
                text_x + view.width
            } else {
                text_end
            };
            let name_end = actions.map_or(name_end, |(open, _)| name_end.min(open.x - 4.0));
            let name_clip =
                Rect::new(list.x, y, (name_end - list.x).max(0.0), COMMIT_ROW).intersect(list);
            draw_name(
                app,
                frame,
                (text_x, band, false, color),
                (&parts, &view),
                name_clip,
            );
            // 放不下的名字和文件树一样在截断处淡出，不在半个字处直接切断；收回途中名字还连着侧栏
            // 外面那一段，到最后才淡出。
            let content = parts_width(app, frame, &parts, false);
            let reach = NAME_FADE * stage(view.progress, FADE_FROM, 1.0);
            if !wanted && text_x + content > name_end + 0.5 && reach > 0.0 {
                let solid = shade_color.map_or(p.sidebar_bg, |c| {
                    crate::view::workspace::panel::over(p.sidebar_bg, c)
                });
                let edge = Rect::new(name_end - reach, y, reach, COMMIT_ROW);
                frame.within(name_clip, |frame| fade(frame, edge, solid, false));
            }
            if let Some(spill) = spill {
                // 显示全名的这一行覆盖列表右边的滚动条（见 `draw_commit_overflow`）：行底色右边到
                // 侧栏边缘的区域同样属于这一行，鼠标移入时全名保持显示。
                let edge = Rect::new(band.right(), y, list.right() - band.right(), COMMIT_ROW);
                hits.push((edge, CommitTarget::File(index)));
                hits.push((spill, CommitTarget::File(index)));
            }
            let check = Rect::new(file_check - 8.0, y + 4.0, 16.0, 16.0);
            check_hit(hits, check, CommitTarget::Check(index));
            hits.push((band.intersect(list), CommitTarget::File(index)));
        }
    }
}

/// 在行底色 `row` 右端画一个写着 `count` 的小圆角徽标，右边和行右端的悬停按钮对齐。
pub(in crate::view) fn draw_count_pill(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    count: &str,
    clip: Rect,
) {
    let p = app.palette;
    // 徽标右边和悬停按钮的右边缘对齐（`draw_count_pill_in` 留 4）。
    let row = Rect::new(row.x, row.y, row.w + 4.0 - END_PAD, row.h);
    draw_count_pill_in(app, frame, row, count, (p.hover, p.text_muted), clip);
}

/// 与 [`draw_count_pill`] 相同，但徽标的底色和字色是 `colors`。
pub(in crate::view) fn draw_count_pill_in(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    count: &str,
    (fill, ink): (Color, Color),
    clip: Rect,
) {
    let size = app.fonts.ui_small;
    let w = frame.measure(FontKind::Ui, size, count) + 12.0;
    let pill = Rect::new(row.right() - 4.0 - w, row.y + 5.0, w, row.h - 10.0);
    frame.rect_clipped(pill, fill, pill.h / 2.0, clip);
    caps_label(frame, app, size, (pill.x + 6.0, pill), count, ink, clip);
}

/// 组、目录行（第 `i` 行）前面的箭头：展开时朝下，收起时朝右；这一行正在展开或收起时
/// 跟着动画转到对应角度。
fn chevron(
    app: &App,
    frame: &mut Frame<'_>,
    at: (f32, f32),
    (i, open): (usize, bool),
    color: Color,
    clip: Rect,
) {
    let turned = match &app.commit.motion {
        Some(m) if m.parent == i => 1.0 - m.hidden() / (m.count as f32 * COMMIT_ROW),
        _ => f32::from(u8::from(open)),
    };
    icons::chevron_turned(frame, at, 6.0, turned * FRAC_PI_2, color, clip);
}

/// 改动行悬停时右端按钮的边长（逻辑像素，经验值）。
const ACTION: f32 = 20.0;

/// 行右端那一列的中心离行底色右边的距离（逻辑像素）：悬停的按钮、状态字母都以它为中心。
///
/// 行底色离侧栏右边 8，加上 12 正好是 20：标题行的工具按钮、分支行的同步图标（离侧栏右边 8，
/// 宽 24）的中心也在这里，从上到下是一列。
const END_CENTER: f32 = 12.0;
/// 行右端靠右排的东西（数量徽标、储藏的时间）离行底色右边的距离（逻辑像素）：和悬停按钮的右边缘
/// 对齐。
const END_PAD: f32 = END_CENTER - ACTION / 2.0;

/// 返回改动行 `band` 悬停时右端「打开文件」「回滚」两个按钮的位置。
fn file_actions(band: Rect) -> (Rect, Rect) {
    let y = band.y + (band.h - ACTION) / 2.0;
    let revert = Rect::new(band.right() - END_PAD - ACTION, y, ACTION, ACTION);
    let open = Rect::new(revert.x - 2.0 - ACTION, y, ACTION, ACTION);
    (open, revert)
}

/// 绘制第 `index` 个文件的「打开文件」「回滚」两个按钮（位置为 `open`、`revert`），只绘制在
/// `clip` 之内；悬停的按钮铺底色，图标用正文色。
fn draw_file_actions(
    app: &App,
    frame: &mut Frame<'_>,
    (open, revert): (Rect, Rect),
    index: usize,
    clip: Rect,
) {
    let p = app.palette;
    for (rect, target) in [
        (open, CommitTarget::OpenFile(index)),
        (revert, CommitTarget::RevertFile(index)),
    ] {
        let on = app.hover == Hover::Commit(target);
        draw_button(frame, p, rect, clip, on, false);
        let color = if on { p.text } else { p.text_muted };
        let (cx, cy) = center(rect);
        match target {
            CommitTarget::OpenFile(_) => {
                let (w, h) = (10.0, 12.0);
                let at = ((cx - w / 2.0).round(), (cy - h / 2.0).round());
                icons::document(frame, at, (w, h), color, clip);
            }
            _ => icons::rollback(frame, (cx, cy), color, clip),
        }
    }
}

/// 改动列表里一行的各列（逻辑像素）：箭头的中心离底色左边 `ARROW`，每深一层往右缩进
/// `INDENT`（与文件视图的树共用 `theme::TREE_INDENT`，16），图标和名字之间隔 `LABEL_GAP`。
///
/// 箭头和图标之间多一列勾选框：勾选框的中心在箭头右边 `CHECK`，图标从勾选框中心往右
/// `ICON` 开始。间距均为经验值。
const ARROW: f32 = 10.0;
const INDENT: f32 = crate::theme::TREE_INDENT;
const CHECK: f32 = 16.0;
const ICON: f32 = 12.0;
const LABEL_GAP: f32 = 5.0;

/// 返回提交信息框 `rect` 内放置文字行的区域。
///
/// 上下留 [`MESSAGE_PAD`]；行高中字形上下各有 `(MESSAGE_LINE - 字号) / 2` 的空白，左右的留白
/// 加上同样的宽度，使文字与边框的可见间距四边相等。
fn message_area(app: &App, rect: Rect) -> Rect {
    let side = MESSAGE_PAD + (MESSAGE_LINE - app.fonts.ui).max(0.0) / 2.0;
    rect.inset(side, MESSAGE_PAD)
}

/// 绘制底部的提交卡片：提交信息（折好的行是 `wrap`），底行左边是「修改上次提交」，右边是
/// 「提交 ⌄」拆分按钮。不显示卡片时什么也不画。
///
/// 整张卡片就是提交信息的输入框：平时没有边框，铺一层比侧栏深一点的中性底色；有焦点时换成
/// 编辑区的底色，外圈画强调色，和搜索框一样。
///
/// 卡片滑进、滑出时有一部分在视图底部下面，整张裁在侧栏里；滑出途中已经不该再提交，不响应
/// 点击。
fn draw_compose(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    wrap: AreaWrap,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> Option<Rect> {
    let panel = app.layout.sidebar;
    let mut leaving = Vec::new();
    let hits = if app.commit_compose_shown() {
        hits
    } else {
        &mut leaving
    };
    frame.within(panel, |frame| draw_card(app, frame, parts, wrap, hits))
}

/// 绘制提交卡片本身，见 [`draw_compose`]。
fn draw_card(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    wrap: AreaWrap,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> Option<Rect> {
    let card = parts.card?;
    let p = app.palette;
    let panel = app.layout.sidebar;
    let focused = app.focus == Focus::CommitMessage;
    field_box(app, frame, card, p.sidebar_bg, focused, false);
    // 框高过渡途中文字按过渡结束后的高度排版，只绘制在当前的框内，行的位置和滚动量不随框高变化。
    let rect = parts.message;
    let area = message_area(app, parts.message_settled);
    let mut field = std::mem::take(&mut app.commit.message);
    // 占位文字只写快捷键：分支就在上面一行，长分支名写进来会被截掉。
    let key = if cfg!(target_os = "macos") {
        "⌘↩"
    } else {
        "Ctrl+Enter"
    };
    let placeholder = if app.commit.amend {
        t!("git-commit-message-amend-placeholder").to_owned()
    } else {
        t!("git-commit-message-placeholder", key = key)
    };
    let caret = frame.within(rect, |frame| {
        draw_area_field(app, frame, area, &mut field, wrap, &placeholder, focused)
    });
    app.commit.message = field;
    let clip = panel.intersect(card);

    // 「修改上次提交」：勾选框连同文字整段可点击，悬停时铺底色。随提交按钮变宽而隐去、变窄而
    // 出现时按进度淡出、淡入，过渡中不响应鼠标。
    if let Some(amend) = parts.amend {
        let (shown, fading) = match app.commit.amend_motion {
            Some((true, clock)) => (clock.progress(), true),
            Some((false, clock)) => (1.0 - clock.progress(), true),
            None => (1.0, false),
        };
        let fade = |c: Color| c.alpha(c.a * shown);
        let hovered = !fading && app.hover == Hover::Commit(CommitTarget::Amend);
        draw_button(frame, p, amend, clip, hovered, false);
        let cy = amend.y + amend.h / 2.0;
        let state = app.commit.amend.then_some(true);
        let border = fade(p.text_dim.alpha(0.55));
        let check = (amend.x + AMEND_CHECK, cy);
        icons::checkbox(
            frame,
            check,
            state,
            fade(p.accent),
            border,
            fade(p.text_on_accent),
            clip,
        );
        let x = amend.x + AMEND_TEXT;
        ui_text_small(
            frame,
            app,
            x,
            amend,
            t!("git-commit-amend"),
            fade(p.text_muted),
            clip,
        );
        if !fading {
            hits.push((amend, CommitTarget::Amend));
        }
    }

    // 「提交 ⌄」：强调色的拆分按钮，左边「✓ 提交」，右边一道细竖线隔开下拉箭头。
    // 无法提交（见 `CommitState::blocker`：未勾选文件、未填写说明、仍有冲突标记）时按钮整体
    // 变淡且悬停不变色，表示按下不会生效；变基暂停时无需勾选即可继续。正在提交时保持强调色，
    // 只是不响应悬停，按钮不在提交前后闪成灰色。
    let (main, menu) = (parts.commit, parts.commit_menu);
    let whole = Rect::new(main.x, main.y, menu.right() - main.x, main.h);
    let busy = app.commit.busy;
    let ready = !busy
        && app
            .git
            .status
            .as_ref()
            .is_some_and(|s| app.commit.blocker(s).is_none());
    // 刚提交成功时提交信息已清空，按钮本应变淡；显示「已提交」期间仍使用强调色。
    let done = app.commit.committed.as_ref().filter(|_| !busy);
    // 不能提交时用灰底、淡色字，表示不可用；把蓝底和白字一起调淡时，浅色主题里的对比度不足。
    let (fill, ink, shade) = if ready || busy || done.is_some() {
        (
            p.accent_fill,
            p.text_on_accent,
            p.text_on_accent.alpha(0.12),
        )
    } else {
        (p.hover, p.text_dim, p.hover)
    };
    frame.rounded(whole, fill, RADIUS_ROW);
    let open = app.menu_button == Some(crate::app::MenuButton::CommitButton);
    for (rect, target) in [
        (main, CommitTarget::Commit),
        (menu, CommitTarget::CommitMenu),
    ] {
        let hovered = !busy && app.hover == Hover::Commit(target);
        let lit = match target {
            CommitTarget::Commit => hovered && ready,
            _ => hovered || open,
        };
        if lit {
            frame.rect_clipped(rect, shade, RADIUS_ROW, whole);
        }
        hits.push((rect, target));
    }
    let divider = Rect::new(menu.x, menu.y + 6.0, 1.0, menu.h - 12.0);
    frame.rect_clipped(divider, ink.alpha(ink.a * 0.3), 0.0, whole);
    icons::chevron(frame, center(menu), 6.0, Direction::Down, ink, whole);
    let size = app.fonts.ui_small;
    let (text, icon_w) = commit_button_text(app);
    let text_w = frame.measure(FontKind::Ui, size, &text);
    let x = (main.x + (main.w - icon_w - text_w) / 2.0).round();
    let clip = panel.intersect(main);
    if icon_w > 0.0 {
        let cy = icon_center_y(app, main, size);
        icons::check(frame, (x + 5.0, cy), 10.0, ink, clip);
    }
    ui_text_small(frame, app, x + icon_w, main, &text, ink, clip);

    caret
}

/// 提交按钮上的文字和前面对勾占的宽度（不画对勾时为 0）。
///
/// 合并、变基暂停时按钮用于完成当前操作；刚提交成功时短暂显示新提交的提交号；平时写明提交几个
/// 文件，分支写在上方的分支行里。
fn commit_button_text(app: &App) -> (String, f32) {
    let busy = app.commit.busy;
    let done = app.commit.committed.as_ref().filter(|_| !busy);
    let operation = app.git.status.as_ref().and_then(|s| s.operation);
    let clean = (app.git.status.as_ref()).is_some_and(|s| s.entries.is_empty());
    let text = match (busy, operation, app.commit.amend) {
        _ if let Some((oid, _)) = done => t!("git-commit-button-committed", oid = oid.as_str()),
        _ if app.commit_pushes() => return (t!("menu-commit-and-push").to_owned(), 16.0),
        (true, ..) => t!("git-commit-button-committing").to_owned(),
        (false, Some(Operation::Rebase), _) => t!("git-commit-button-continue-rebase").to_owned(),
        (false, Some(Operation::Merge), _) => t!("git-commit-button-finish-merge").to_owned(),
        (false, Some(Operation::CherryPick), _) => {
            t!("git-commit-button-finish-cherry-pick").to_owned()
        }
        (false, Some(Operation::Revert), _) => t!("git-commit-button-finish-revert").to_owned(),
        (false, None, true) => t!("git-commit-button-amend").to_owned(),
        (false, None, false) if clean => t!("git-commit-button-nothing").to_owned(),
        (false, None, false) => t!("git-commit-button").to_owned(),
    };
    let scope = app.commit_scope().filter(|_| !busy && done.is_none());
    let text = scope.map_or(text, |count| t!("git-commit-button-files", count = count));
    let icon_w = if busy || clean { 0.0 } else { 16.0 };
    (text, icon_w)
}

/// 返回加载图标当前旋转到的圈数，按进程启动以来的时间计算，各处的加载图标转速一致。
fn spin_turn() -> f32 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f32()
        * SPIN_SPEED
}

/// 文件名的颜色：正文色，删掉的用次要色。
fn file_color(change: Change, p: &Palette) -> Color {
    if change == Change::Deleted {
        p.text_muted
    } else {
        p.text
    }
}

/// 改动列表中一行文件名的位置与内容。
///
/// 包括文件名起点（相对行底色左边）、文件名、平铺时其后以淡色显示的目录、文件名的颜色，
/// 以及悬停时该行的底色。
struct RowText {
    /// 文件名的起点，相对行底色左边（逻辑像素）。
    offset: f32,
    name: String,
    dir: Option<String>,
    /// 名字是并起来的几级目录（`a/b/c`），由 [`draw_dir_parts`] 绘制。
    folder: bool,
    color: Color,
    /// 悬停时该行的底色；没有悬停时也是 `Some` 的行同样铺色。
    shade: Option<Color>,
}

/// 返回深 `depth` 层的文件行的勾选框在第几列缩进。
///
/// 按目录排成树时比所在目录深一层，平铺时不缩进。
/// 返回箭头到勾选框中心的距离（逻辑像素）：浏览历史提交的文件时没有勾选框，为 0。
fn check_gap(app: &App) -> f32 {
    if app.commit.browse { 0.0 } else { CHECK }
}

fn file_column(app: &App, depth: usize) -> usize {
    if app.commit.flat() { 0 } else { depth + 1 }
}

/// 返回第 `row` 行名字的排法（组标题行返回 `None`）；列位置与 [`draw_row`] 使用同一组常量。
fn row_text(app: &App, status: &Status, root: &Path, row: &CommitRow) -> Option<RowText> {
    let p = app.palette;
    // 箭头、勾选框、图标、图标那一列宽、间隔（见 `draw_row` 和 `ARROW` 那一组常量）。
    let offset = |level: usize| {
        ARROW + level as f32 * INDENT + check_gap(app) + ICON + icons::FOLDER_WIDTH + LABEL_GAP
    };
    match row {
        CommitRow::Group(_) => None,
        CommitRow::Dir { label, depth, .. } => Some(RowText {
            offset: offset(depth + 1),
            name: label.clone(),
            dir: None,
            folder: true,
            color: p.text,
            shade: Some(p.hover),
        }),
        CommitRow::File { index, depth } => {
            let entry = status.entries.get(*index)?;
            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dir = entry
                .path
                .parent()
                .and_then(|p| p.strip_prefix(root).ok())
                .filter(|p| app.commit.flat() && !p.as_os_str().is_empty())
                .map(|p| p.to_string_lossy().into_owned());
            let change = entry.shown();
            let selected = app.commit_is_selected(&entry.path);
            Some(RowText {
                offset: offset(file_column(app, *depth)),
                name,
                dir,
                folder: false,
                color: file_color(change, p),
                shade: row_color(p, selected, app.focus == Focus::CommitList, true),
            })
        }
    }
}

/// 计算鼠标停着的改动行写全名所需的溢出区域（见 [`CommitOverflow`]）。
///
/// 文件行的「打开文件」「回滚」两个按钮跟在全名之后。名字放得下、正在拖动时返回 `None`。
/// 展开收起的动画途中，被展开、收起的行及其上方各行位置不变，仍显示全名；下方正在移动的行
/// 返回 `None`。
pub(in crate::view) fn commit_overflow(app: &App, frame: &mut Frame<'_>) -> Option<CommitOverflow> {
    // 刷新以后各行还在移动（见 `draw_shift`），按行号对不上。
    if !app.commit_shown() || !app.idle_pointer() || app.commit.shift.is_some() {
        return None;
    }
    let status = app.git.status.as_ref()?;
    let root = app.git.root.as_deref()?;
    let rows = app.commit_rows();
    let row = match app.hover {
        Hover::Commit(
            CommitTarget::Dir(r) | CommitTarget::DirCheck(r) | CommitTarget::Sticky(r, _),
        ) => r,
        // 鼠标移到跟在全名后面的按钮上时，仍保持全名显示。
        Hover::Commit(
            CommitTarget::File(f)
            | CommitTarget::Check(f)
            | CommitTarget::OpenFile(f)
            | CommitTarget::RevertFile(f),
        ) => rows
            .iter()
            .position(|r| matches!(r, CommitRow::File { index, .. } if *index == f))?,
        _ => return None,
    };
    if app.commit.motion.as_ref().is_some_and(|m| row > m.parent) {
        return None;
    }
    let list = app.commit_parts().list;
    let y = row_top(app, &rows, row, list);
    let text = row_text(app, status, root, rows.get(row)?)?;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    let end = name_end(app, frame, &text, band);
    let window = app.layout.window.right() - 8.0;
    let (right, actions) = if text.folder {
        if end <= band.right() - 6.0 {
            return None;
        }
        ((end + 8.0).min(window), None)
    } else if app.commit.browse {
        // 浏览历史提交的文件时没有按钮：名字放不下（状态字母之前）时才写全名。
        if end <= band.right() - END_CENTER * 2.0 {
            return None;
        }
        ((end + 8.0).min(window), None)
    } else {
        // 文件行的按钮在名字之后：名字与按钮在原位放得下时不展开。按钮超出窗口时向左移回，
        // 名字在按钮之前截断。
        let (open, revert) = file_actions(band);
        if end <= open.x - 4.0 {
            return None;
        }
        let shift = (end + 4.0 - open.x).min(window - revert.right());
        let moved = |r: Rect| Rect::new(r.x + shift, r.y, r.w, r.h);
        let (open, revert) = (moved(open), moved(revert));
        (revert.right() + 4.0, Some((open, revert)))
    };
    let spill = (right > list.right()).then(|| {
        Rect::new(
            list.right(),
            y + 1.0,
            right - list.right(),
            COMMIT_ROW - 2.0,
        )
    });
    Some(CommitOverflow {
        row,
        y,
        spill,
        actions,
    })
}

/// 返回第 `row` 行写全名时两个按钮离终点（「打开文件」在 `open`）还差的距离（逻辑像素）。
///
/// 按钮随名字滑出（名字还有 `name_lag` 未显示），同时从原位（[`file_actions`]）滑向终点
/// （[`App::track_commit_actions`]），取两者中较大的距离：名字无需加长、只为按钮让出位置时，
/// 名字不动，按钮仍然滑向终点。
fn actions_lag(app: &App, row: usize, band: Rect, open: Rect, name_lag: f32) -> f32 {
    let slide = app.commit.actions_slide.filter(|&(r, _)| r == row);
    let rest = slide.map_or(0.0, |(_, clock)| {
        let (home, _) = file_actions(band);
        (open.x - home.x).max(0.0) * (1.0 - clock.progress())
    });
    name_lag.max(rest)
}

/// 返回名字显示出的宽度：内容宽 `width`，截短时可用宽度为 `room`（写全名时为 `None`）。
///
/// 截短后仍然放不下的名字（不能再缩短的文件名、目录名）在 `room` 处淡出，显示出的宽度为
/// `room`。名字的过渡按显示出的宽度进行：写全名时从截断处滑出，按钮随之移动；内容不变时
/// 同样有过渡。
fn shown_width(width: f32, room: Option<f32>) -> f32 {
    room.map_or(width, |room| width.min(room))
}

/// 返回 `band` 一行中名字（平铺时连同其后的目录）完整绘制时末尾的 x 坐标（逻辑像素）。
fn name_end(app: &App, frame: &mut Frame<'_>, text: &RowText, band: Rect) -> f32 {
    let name_w = if text.folder {
        dir_label_width(app, frame, &text.name)
    } else {
        frame.measure(FontKind::Ui, app.fonts.ui, &text.name)
    };
    let mut end = band.x + text.offset + name_w;
    if let Some(dir) = &text.dir {
        end += 8.0 + frame.measure(FontKind::Ui, app.fonts.ui_small, dir);
    }
    end
}

/// 在编辑区上面继续绘制放不下的改动行名字：底色、文字和列表里的那一行连成一条。
///
/// 命中范围按全名算（见 [`commit_overflow`]），画的时候按名字展开到的宽度露出，和文件树相同：
/// 过渡途中鼠标移过去不会落空。
pub(in crate::view) fn draw_commit_overflow(app: &App, frame: &mut Frame<'_>) {
    let Some(CommitOverflow {
        row,
        y,
        spill: Some(spill),
        actions,
    }) = app.commit.overflow
    else {
        return;
    };
    let (Some(status), Some(root)) = (app.git.status.as_ref(), app.git.root.as_deref()) else {
        return;
    };
    let rows = app.commit_rows();
    let Some(text) = rows.get(row).and_then(|r| row_text(app, status, root, r)) else {
        return;
    };
    let Some(view) = app.commit.names.peek(&name_key(status, &rows[row])) else {
        return;
    };
    let list = app.commit_parts().list;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    let x = band.x + text.offset;
    // 按钮跟着名字移动（见 `draw_row`），露出的部分到按钮为止；目录行露到名字后面 8。
    let parts = full_parts(&text);
    let width = parts_width(app, frame, &parts, text.folder);
    let lag = match actions {
        Some((open, _)) => actions_lag(app, row, band, open, width - view.width),
        None => width - view.width,
    };
    let actions = actions.map(|(open, revert)| {
        let back = |r: Rect| Rect::new(r.x - lag, r.y, r.w, r.h);
        (back(open), back(revert))
    });
    let right = match actions {
        Some((_, revert)) => revert.right() + 4.0,
        None => x + view.width + 8.0,
    }
    .min(spill.right());
    if right <= spill.x {
        return;
    }
    // 这一行绘制在最上层：底色从滚动条的可点击范围开始，覆盖这一行高度内的滚动条，滚动条不会
    // 穿过全名；滚动条在这一行上下的部分正常显示。
    let left = list.right() - SCROLLBAR_HIT;
    let spill = Rect::new(left, spill.y, right - left, spill.h);
    let p = app.palette;
    let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
    // 先垫一层侧栏底色，再叠半透明的悬停色，与列表里那一行的底色相同。
    frame.rect_corners(spill, p.sidebar_bg, radii, spill);
    if let Some(color) = text.shade {
        frame.rect_corners(spill, color, radii, spill);
    }
    let clip = Rect::new(spill.x, y, spill.w, COMMIT_ROW);
    // 有按钮时名字在按钮之前截断。
    let text_clip = match actions {
        Some((open, _)) => Rect::new(clip.x, clip.y, (open.x - 4.0 - clip.x).max(0.0), clip.h),
        None => clip,
    };
    let style = (x, band, text.folder, text.color);
    draw_name(app, frame, style, (&parts, &view), text_clip);
    if let (Some(actions), CommitRow::File { index, .. }) = (actions, &rows[row]) {
        draw_file_actions(app, frame, actions, *index, clip);
    }
}

/// 绘制鼠标移开后正在收回、还伸在侧栏外面的改动行名字：不响应点击，收回到侧栏边缘时消失。
///
/// 底色和列表里的那一段一起淡出（`retracting_color`），写的是收回前的全名，与文件树相同。
pub(in crate::view) fn draw_commit_overflow_closing(app: &App, frame: &mut Frame<'_>) {
    if !app.commit_shown() || !app.commit.names.moving() {
        return;
    }
    let (Some(status), Some(root)) = (app.git.status.as_ref(), app.git.root.as_deref()) else {
        return;
    };
    let p = app.palette;
    let rows = app.commit_rows();
    let list = app.commit_parts().list;
    let open = app.commit.overflow.map(|o| o.row);
    let window = app.layout.window.right() - 8.0;
    let focused = app.focus == Focus::CommitList;
    for (key, view) in app.commit.names.moving_rows() {
        let Some(row) = (0..rows.len()).find(|&i| name_key(status, &rows[i]) == key) else {
            continue;
        };
        if Some(row) == open || app.commit.motion.as_ref().is_some_and(|m| row > m.parent) {
            continue;
        }
        let Some(text) = row_text(app, status, root, &rows[row]) else {
            continue;
        };
        let y = row_top(app, &rows, row, list);
        let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
        let x = band.x + text.offset;
        let right = (x + view.width + 8.0).min(window);
        if right <= list.right() {
            continue;
        }
        let left = list.right() - SCROLLBAR_HIT;
        let rect = Rect::new(left, y + 1.0, right - left, COMMIT_ROW - 2.0);
        let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
        frame.rect_corners(rect, p.sidebar_bg, radii, rect);
        let selected = match &rows[row] {
            CommitRow::File { index, .. } => app.commit_is_selected(&status.entries[*index].path),
            _ => false,
        };
        if let Some(color) = retracting_color(p, selected, focused, view.progress) {
            frame.rect_corners(rect, color, radii, rect);
        }
        let clip = Rect::new(rect.x, y, rect.w, COMMIT_ROW);
        // 收回前的全名：相同的前缀不淡，其余随过渡淡出。内容没变（只是宽度在收）时照写全名。
        let parts: Vec<Cow<'_, str>> = if view.old.is_empty() {
            full_parts(&text)
        } else {
            view.old.iter().map(|s| Cow::Borrowed(s.as_str())).collect()
        };
        let alpha = |k: usize| {
            if view.old.is_empty() || k < view.common {
                1.0
            } else {
                crossfade(&view).0
            }
        };
        if text.folder {
            draw_dir_parts(app, frame, (x, band), &parts, alpha, clip);
        } else {
            draw_file_parts(app, frame, (x, band), &parts, (text.color, alpha), clip);
        }
    }
}

/// 改动行 `row` 名字过渡状态的键（见 [`CommitName`]）；组标题行没有名字，不会用到。
fn name_key(status: &Status, row: &CommitRow) -> CommitName {
    match row {
        CommitRow::Dir { group, dir, .. } => CommitName::Dir(*group, dir.clone()),
        CommitRow::File { index, .. } => CommitName::File(
            status
                .entries
                .get(*index)
                .map(|e| e.path.clone())
                .unwrap_or_default(),
        ),
        CommitRow::Group(group) => CommitName::Dir(*group, Default::default()),
    }
}

/// 改动行全名的各部分（见 [`dir_parts`]、[`file_parts`]），不截短。
fn full_parts(text: &RowText) -> Vec<Cow<'_, str>> {
    if text.folder {
        text.name.split('/').map(Cow::Borrowed).collect()
    } else {
        let mut parts = vec![Cow::Borrowed(text.name.as_str())];
        parts.extend(text.dir.as_deref().map(Cow::Borrowed));
        parts
    }
}

/// 第 `row` 行的上沿（逻辑像素）：固定在顶部的行按固定的位置，其余按滚动的偏移。
fn row_top(app: &App, rows: &CommitRows, row: usize, list: Rect) -> f32 {
    match app.commit_sticky(rows).iter().find(|s| s.row == row) {
        Some(s) => s.rect.y,
        None => list.y + row as f32 * COMMIT_ROW - app.commit.scroll,
    }
}

/// 返回并起来的几级目录（`Sources/DEEIXPET`）画出来的宽度（逻辑像素），见 [`draw_dir_parts`]。
fn dir_label_width(app: &App, frame: &mut Frame<'_>, label: &str) -> f32 {
    let size = app.fonts.ui;
    let parts = label.split('/').count() as f32;
    let names: f32 = label
        .split('/')
        .map(|part| frame.measure(FontKind::Ui, size, part))
        .sum();
    let slash = frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP;
    names + (parts - 1.0) * slash
}

/// 目录行名字的各部分：并起来的几级目录各是一部分；放不下 `room`（逻辑像素）时折叠中间的各级
/// （见 [`fold_chain`]），折掉的几级换成一个 `CHAIN_FOLD`，目录名不会被截成半个。
fn dir_parts<'a>(
    app: &App,
    frame: &mut Frame<'_>,
    label: &'a str,
    room: Option<f32>,
) -> Vec<Cow<'a, str>> {
    let names: Vec<&str> = label.split('/').collect();
    let Some(room) = room else {
        return names.into_iter().map(Cow::Borrowed).collect();
    };
    let size = app.fonts.ui;
    let widths: Vec<f32> = (names.iter())
        .map(|name| frame.measure(FontKind::Ui, size, name))
        .collect();
    let step = frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP;
    let fold = frame.measure(FontKind::Ui, size, CHAIN_FOLD);
    let (parts, _) = fold_chain(&widths, step, fold, names.len() - 1, room);
    (parts.into_iter())
        .map(|part| Cow::Borrowed(part.map_or(CHAIN_FOLD, |i| names[i])))
        .collect()
}

/// 画目录行名字的各部分 `parts`（见 [`dir_parts`]），之间是「/」，样式与文件树中合并显示的目录
/// 相同。第 k 部分的不透明度是 `alpha(k)`，它前面的「/」取两边较淡的一个，为 0 的不画；返回
/// 名字末尾的 x 坐标（逻辑像素）。
fn draw_dir_parts<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band): (f32, Rect),
    parts: &[S],
    alpha: impl Fn(usize) -> f32,
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let size = app.fonts.ui;
    let separator = p.text.alpha(p.text.a * CHAIN_SEPARATOR);
    let slash = frame.measure(FontKind::Ui, size, "/");
    let mut x = x;
    for (k, part) in parts.iter().enumerate() {
        let part = part.as_ref();
        if k > 0 {
            let a = alpha(k - 1).min(alpha(k));
            if a > 0.0 {
                let color = separator.alpha(separator.a * a);
                ui_text(frame, app, x + CHAIN_GAP, band, "/", color, clip);
            }
            x += slash + 2.0 * CHAIN_GAP;
        }
        let color = if part == CHAIN_FOLD {
            separator
        } else {
            p.text
        };
        let a = alpha(k);
        x = if a > 0.0 {
            ui_text(frame, app, x, band, part, color.alpha(color.a * a), clip)
        } else {
            x + frame.measure(FontKind::Ui, size, part)
        };
    }
    x
}

/// 文件行名字的各部分：文件名，平铺时再加所在目录 `dir`。
///
/// 和文件树一样，文件名不截短：放不下 `room`（逻辑像素）时画到 `room` 为止，在截断处淡出。目录
/// 只用文件名剩下的地方，从前面按级去掉，留下离文件最近的几级（`…/app/git`）：分辨同名文件靠的
/// 就是它们。
fn file_parts<'a>(
    app: &App,
    frame: &mut Frame<'_>,
    (name, dir): (&'a str, Option<&'a str>),
    room: Option<f32>,
) -> Vec<Cow<'a, str>> {
    let mut parts = vec![Cow::Borrowed(name)];
    if let Some(dir) = dir {
        let dir = match room {
            Some(room) => {
                let used = frame.measure(FontKind::Ui, app.fonts.ui, name) + DIR_GAP;
                nearest_dirs(frame, dir, app.fonts.ui_small, room - used)
            }
            None => Cow::Borrowed(dir),
        };
        parts.push(dir);
    }
    parts
}

/// 把目录 `dir` 从前面按级去掉，直到放得进 `room`（逻辑像素）：`crates/app/git` 依次试
/// `…/app/git`、`…/git`。只剩一级还放不下时就是它，由绘制时截断淡出。
fn nearest_dirs<'a>(frame: &mut Frame<'_>, dir: &'a str, size: f32, room: f32) -> Cow<'a, str> {
    if frame.measure(FontKind::Ui, size, dir) <= room {
        return Cow::Borrowed(dir);
    }
    let mut rest = dir;
    while let Some((_, tail)) = rest.split_once('/') {
        rest = tail;
        let shown = format!("{CHAIN_FOLD}/{rest}");
        if frame.measure(FontKind::Ui, size, &shown) <= room || !rest.contains('/') {
            return Cow::Owned(shown);
        }
    }
    Cow::Borrowed(dir)
}

/// 画文件行名字的各部分 `parts`（见 [`file_parts`]）：文件名用 `color`，后面的目录用小一号的
/// 淡色；不透明度和 [`draw_dir_parts`] 一样由 `alpha` 给出。返回名字末尾的 x 坐标（逻辑像素）。
fn draw_file_parts<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band): (f32, Rect),
    parts: &[S],
    (color, alpha): (Color, impl Fn(usize) -> f32),
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let mut x = x;
    for (k, part) in parts.iter().enumerate() {
        let (size, ink, start) = if k == 0 {
            (app.fonts.ui, color, x)
        } else {
            (app.fonts.ui_small, p.text_dim, x + DIR_GAP)
        };
        let a = alpha(k);
        let ink = ink.alpha(ink.a * a);
        x = match (k, a > 0.0) {
            (0, true) => ui_text(frame, app, start, band, part.as_ref(), ink, clip),
            (_, true) => ui_text_small(frame, app, start, band, part.as_ref(), ink, clip),
            (_, false) => start + frame.measure(FontKind::Ui, size, part.as_ref()),
        };
    }
    x
}

/// 平铺时文件名和后面的目录之间的距离（逻辑像素）。
const DIR_GAP: f32 = 8.0;

/// 改动行名字各部分 `parts` 画出来的宽度（逻辑像素）；`folder` 为真时是目录行。
fn parts_width<S: AsRef<str>>(app: &App, frame: &mut Frame<'_>, parts: &[S], folder: bool) -> f32 {
    let size = app.fonts.ui;
    let measure = |frame: &mut Frame<'_>, k: usize, part: &str| {
        let size = if k == 0 || folder {
            size
        } else {
            app.fonts.ui_small
        };
        frame.measure(FontKind::Ui, size, part)
    };
    let gap = if folder {
        frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP
    } else {
        DIR_GAP
    };
    let mut width = 0.0;
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            width += gap;
        }
        width += measure(frame, k, part.as_ref());
    }
    width
}

/// 名字过渡中旧内容和新内容的不透明度：随进度此消彼长，两者之和始终为 1。
///
/// 改动行截短的目录与全名的写法不同（`crates/winc-app/…` 与 `…/app/git`），先淡出旧内容、再淡入
/// 新内容（`LabelView` 的 `old_alpha`、`alpha`）时，中间有一段两者都接近透明，文字会短暂消失；
/// 交叉淡化使任意时刻都有可见的文字。
fn crossfade(view: &LabelView) -> (f32, f32) {
    (1.0 - view.progress, view.progress)
}

/// 画改动行名字过渡中的一帧：旧内容淡出、新内容 `parts` 同时淡入（见 [`crossfade`]），相同的
/// 前缀只由新内容画一次。返回新内容末尾的 x 坐标（逻辑像素）。
fn draw_name<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band, folder, color): (f32, Rect, bool, Color),
    (parts, view): (&[S], &LabelView),
    clip: Rect,
) -> f32 {
    let common = view.common;
    let (old_alpha, new_alpha) = crossfade(view);
    let old = |k: usize| if k < common { 0.0 } else { old_alpha };
    let new = |k: usize| if k < common { 1.0 } else { new_alpha };
    if folder {
        draw_dir_parts(app, frame, (x, band), &view.old, old, clip);
        draw_dir_parts(app, frame, (x, band), parts, new, clip)
    } else {
        draw_file_parts(app, frame, (x, band), &view.old, (color, old), clip);
        draw_file_parts(app, frame, (x, band), parts, (color, new), clip)
    }
}
