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

mod header;
mod names;
mod overflow;
mod row;
mod stash;

pub(in crate::view) use header::location_span;
use header::*;
use names::*;
pub(in crate::view) use overflow::commit_overflow;
pub(in crate::view) use overflow::draw_commit_overflow;
pub(in crate::view) use overflow::draw_commit_overflow_closing;
use overflow::*;
use row::*;
use stash::*;

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
    let mut field = std::mem::take(&mut app.git.commit.message);
    let wrap = wrap_area_field(app, frame, width, &mut field, focused);
    app.git.commit.message = field;
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
    if app.git.commit.motion.is_none() && app.git.status.is_some() {
        app.git.commit.scroll = app.git.commit.scroll.min(app.commit_max_scroll());
    }
    let mut hits = Vec::new();
    frame.with_opacity(tools, |frame| draw_header(app, frame, &parts, &mut hits));
    let animate = app.animate_commit_names();
    app.git.commit.names.begin_frame(animate);
    match (&app.git.status, &app.git.problem) {
        (Some(status), _) if app.git.commit.shift.is_some() => {
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
    app.git.commit.names.end_frame();
    // 列表的滚动条贴在列表右边，盖在固定的行上面；放得下时没有。
    let content = app.commit_max_scroll() + parts.list.h;
    let scroll = app.git.commit.scroll;
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
    app.git.commit.hits_generation = app.git.generation;
    caret
}

/// 绘制浏览历史提交的文件列表（见 `App::with_log_list`）：只有改动列表，标题行由调用方绘制。
///
/// 借用提交视图时调用：列表的排列、写全名、固定在顶部的目录和各种过渡与提交视图相同。
pub(in crate::view) fn draw_commit_browse(app: &mut App, frame: &mut Frame<'_>) {
    let parts = app.commit_parts();
    if app.git.commit.motion.is_none() && app.git.status.is_some() {
        app.git.commit.scroll = app.git.commit.scroll.min(app.commit_max_scroll());
    }
    let mut hits = Vec::new();
    let animate = app.animate_commit_names();
    app.git.commit.names.begin_frame(animate);
    if let Some(status) = &app.git.status {
        if app.git.commit.shift.is_some() {
            draw_shift(app, frame, &parts, status, &mut hits);
        } else {
            draw_list(app, frame, &parts, status, &mut hits);
        }
    }
    app.git.commit.names.end_frame();
    let content = app.commit_max_scroll() + parts.list.h;
    let scroll = app.git.commit.scroll;
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
    app.git.commit.hits_generation = app.git.generation;
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

/// 储藏一组的标题和各条储藏的名字离底色左边的距离（逻辑像素）：标题紧跟在箭头后面，和改动列表组
/// 标题的箭头同一列。各条储藏和标题的文字对齐：行前面没有勾选框和图标，再缩进一层只会留下
/// 一块空白。
const STASH_TITLE: f32 = ARROW + 10.0;
/// 正在查看的储藏下面各个文件的图标离行左边的距离（逻辑像素）：比储藏名再缩进一级。
const STASH_FILE: f32 = STASH_TITLE + 12.0;
/// 储藏那一行悬停时「应用」文字两侧的留白（逻辑像素），和「上一次提交」那一栏的「撤销」相同。
const STASH_APPLY_PAD: f32 = 6.0;

/// 没有改动时列表区的空状态：「暂无待提交的更改」，下面是当前分支与上游的
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
    let enabled = app.git.branch.running.is_none();
    let hovered = app.pointer.hover == Hover::Commit(CommitTarget::NextStep);
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
    if let Some(motion) = &app.git.commit.motion {
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
                crate::layout::tree_row_slot(list, app.git.commit.scroll, v, Some(block), hidden);
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
    let first = (app.git.commit.scroll / COMMIT_ROW).floor() as usize;
    let visible = (list.h / COMMIT_ROW).ceil() as usize + 1;

    for i in (first..rows.len()).take(visible) {
        let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll;
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
    let Some(shift) = &app.git.commit.shift else {
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

    let scroll = shift.scroll + (app.git.commit.scroll - shift.scroll) * t;
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
            if app.pointer.hover == Hover::Commit(CommitTarget::Sticky(s.row, s.level)) {
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
    let turned = match &app.git.commit.motion {
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
        let on = app.pointer.hover == Hover::Commit(target);
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
    let mut field = std::mem::take(&mut app.git.commit.message);
    // 占位文字只写快捷键：分支就在上面一行，长分支名写进来会被截掉。
    let key = if cfg!(target_os = "macos") {
        "⌘↩"
    } else {
        "Ctrl+Enter"
    };
    let placeholder = if app.git.commit.amend {
        t!("git-commit-message-amend-placeholder").to_owned()
    } else {
        t!("git-commit-message-placeholder", key = key)
    };
    let caret = frame.within(rect, |frame| {
        draw_area_field(app, frame, area, &mut field, wrap, &placeholder, focused)
    });
    app.git.commit.message = field;
    let clip = panel.intersect(card);

    // 「修改上次提交」：勾选框连同文字整段可点击，悬停时铺底色。随提交按钮变宽而隐去、变窄而
    // 出现时按进度淡出、淡入，过渡中不响应鼠标。
    if let Some(amend) = parts.amend {
        let (shown, fading) = match app.git.commit.amend_motion {
            Some((true, clock)) => (clock.progress(), true),
            Some((false, clock)) => (1.0 - clock.progress(), true),
            None => (1.0, false),
        };
        let fade = |c: Color| c.alpha(c.a * shown);
        let hovered = !fading && app.pointer.hover == Hover::Commit(CommitTarget::Amend);
        draw_button(frame, p, amend, clip, hovered, false);
        let cy = amend.y + amend.h / 2.0;
        let state = app.git.commit.amend.then_some(true);
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
    let busy = app.git.commit.busy;
    let ready = !busy
        && app
            .git
            .status
            .as_ref()
            .is_some_and(|s| app.git.commit.blocker(s).is_none());
    // 刚提交成功时提交信息已清空，按钮本应变淡；显示「已提交」期间仍使用强调色。
    let done = app.git.commit.committed.as_ref().filter(|_| !busy);
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
    let open = app.overlays.menu_button == Some(crate::app::MenuButton::CommitButton);
    for (rect, target) in [
        (main, CommitTarget::Commit),
        (menu, CommitTarget::CommitMenu),
    ] {
        let hovered = !busy && app.pointer.hover == Hover::Commit(target);
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
    let busy = app.git.commit.busy;
    let done = app.git.commit.committed.as_ref().filter(|_| !busy);
    let operation = app.git.status.as_ref().and_then(|s| s.operation);
    let clean = (app.git.status.as_ref()).is_some_and(|s| s.entries.is_empty());
    let text = match (busy, operation, app.git.commit.amend) {
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
    if app.git.commit.browse { 0.0 } else { CHECK }
}

fn file_column(app: &App, depth: usize) -> usize {
    if app.git.commit.flat() { 0 } else { depth + 1 }
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
                .filter(|p| app.git.commit.flat() && !p.as_os_str().is_empty())
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

/// 平铺时文件名和后面的目录之间的距离（逻辑像素）。
const DIR_GAP: f32 = 8.0;
