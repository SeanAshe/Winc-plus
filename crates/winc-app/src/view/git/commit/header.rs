//! 提交视图顶部的绘制：位置与分支、同步状态、横幅和撤销条。

use super::*;

/// 绘制第二行右端的三个按钮（左边是共用的项目和分支）：「日志」「刷新」「⋯」。
///
/// 三个按钮都只有图标，做什么写在悬停提示里。
/// 读取期间「刷新」的图标旋转，至少转满一圈（见 `App::commit_refreshing`）。「⋯」的下拉菜单
/// 里有同步、储藏、远程仓库和显示方式。
pub(super) fn draw_header(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let header = parts.header;
    let p = app.palette;
    for (rect, target) in parts.tools {
        let open = target == CommitTarget::More
            && app.overlays.menu_button == Some(crate::app::MenuButton::CommitMore);
        let hovered = app.pointer.hover == Hover::Commit(target);
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
                icons::refresh_turned(frame, c, icons::spin_turn(), color, header);
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
pub(super) fn draw_context(
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
pub(super) fn draw_context_pick(
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
    let hovered = app.pointer.hover == Hover::Commit(target);
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
    if !app.git_has_locations_button() {
        return None;
    }
    let bar = ContextBar::new(app, frame);
    let right = app.commit_parts().tools[0].0.x - 4.0;
    let width = location_width(app, frame)?;
    let room = (right - bar.left() - LOCATION_GAP).max(0.0);
    let project = bar.project_full();
    if project + width <= room {
        let left = bar.left() + project + LOCATION_GAP;
        return Some((left, left + width));
    }
    Some((right - width.min((room - project).max(room / 2.0)), right))
}

/// 返回位置按钮完整显示时的宽度（逻辑像素）；只有一个位置、没有该按钮时为 `None`。
pub(super) fn location_width(app: &App, frame: &mut Frame<'_>) -> Option<f32> {
    if !app.git_has_locations_button() {
        return None;
    }
    let label = worktree_button_label(app, frame);
    let text = frame.measure(FontKind::Ui, app.fonts.ui_small, &label);
    Some(LOCATION_LEAD.1 + text + CONTEXT_PICK_TAIL)
}

/// 位置按钮上写的名字：打开的目录中有多个仓库时写当前仓库；否则写工作树，主工作树写「主工作树」，
/// 附属工作树写它自己的名字。
///
/// 标题行放不下完整的项目名和「主工作树」时不写后者，按钮只有图标和下拉箭头：项目名优先，
/// 是哪个工作树写在悬停提示里。仓库名、附属工作树的名字始终写出。
fn worktree_button_label(app: &App, frame: &mut Frame<'_>) -> String {
    if app.git.repositories.len() >= 2 {
        return app.git_current_repository_name();
    }
    let label = app.git_worktree_label();
    if label != t!("git-location-main-worktree") {
        return label;
    }
    let bar = ContextBar::new(app, frame);
    let right = app.commit_parts().tools[0].0.x - 4.0;
    let room = (right - bar.left() - LOCATION_GAP).max(0.0);
    let text = frame.measure(FontKind::Ui, app.fonts.ui_small, &label);
    let full = LOCATION_LEAD.1 + text + CONTEXT_PICK_TAIL;
    if bar.project_full() + full <= room {
        label
    } else {
        String::new()
    }
}

/// 绘制项目名后面的位置按钮：有多个仓库或多个工作树时才有，写着现在在哪个仓库或工作树。图标
/// 在当前仓库有多个工作树时是工作树，否则是文件夹。
///
/// 当前仓库的其他工作树有未提交的更改时，图标右上角有一个强调色的小圆点，和视图标签上的圆点
/// 同一个意思；几个写在悬停提示里，是哪几个在选择器里。
pub(super) fn draw_location_row(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    (left, right): (f32, f32),
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let label = worktree_button_label(app, frame);
    let names = [(label.as_str(), p.text_muted)];
    let open = app.picker_lists_locations();
    let lit = open || app.pointer.hover == Hover::Commit(CommitTarget::Location);
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
            if app.git_has_worktrees() {
                icons::worktree_small(frame, (x, y), color, clip);
            } else {
                icons::folder_small(frame, (x, y), color, clip);
            }
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
pub(super) fn draw_branch_row(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    // 不是仓库时没有分支可写：不画一个空的按钮。
    if app.git.root.is_none() {
        return;
    }
    let p = app.palette;
    let mut right = row.right();
    if sync_shown(app) {
        right = draw_sync(app, frame, row, clip, hits) - BUTTON_GAP;
    }
    let head = head_label(app);
    // 当前是哪个仓库写在标题行的位置按钮上，这里只写分支。
    let names = vec![(head.as_str(), p.text)];
    draw_context_pick(
        app,
        frame,
        row,
        (BADGE_CENTER, BADGE_ROOM),
        right,
        CommitTarget::Branch,
        app.branch_picker_from(crate::app::GitAnchor::Branch),
        |frame, center, color| icons::branch_small(frame, center, color, clip),
        &names,
        clip,
        hits,
    );
}

/// 分支按钮上写的名字：当前分支；游离时是括号里的提交号，还没有读到状态时为空。
fn head_label(app: &App) -> String {
    let head = app.git.status.as_ref().and_then(|s| s.head.as_ref());
    head.map_or_else(String::new, |head| match head {
        Head::Branch(name) => name.clone(),
        Head::Detached(oid) => format!("({oid})"),
    })
}

/// 有没有同步控件：没有远程时没有东西可同步，但操作的进度和结果仍然显示在这里。
fn sync_shown(app: &App) -> bool {
    let active = app.git.branch.running.is_some() || app.git.branch.finished.is_some();
    app.git.status.is_some() && (active || app.git_sync_state() != SyncState::NoRemote)
}

/// 同步控件上写的文字：和上游差多少个提交，后面是点了会做的事（推送、拉取、同步、发布分支）；
/// 没有事可做时为空。
///
/// 没有待提交的更改时列表区的空状态已经写着同样的事并有一个按钮：这里只写提交数，不再写一遍。
fn sync_counts(app: &App) -> String {
    let clean = (app.git.status.as_ref()).is_some_and(|s| s.entries.is_empty())
        && app.commit_other_rows().is_empty();
    if clean {
        return match app.git_sync_state() {
            SyncState::Push(ahead) => format!("↑{ahead}"),
            SyncState::Pull(behind) => format!("↓{behind}"),
            SyncState::Both { ahead, behind } => format!("↓{behind} ↑{ahead}"),
            _ => String::new(),
        };
    }
    match app.git_sync_state() {
        SyncState::Push(ahead) => format!("↑{ahead} {}", t!("git-commit-next-push")),
        SyncState::Pull(behind) => format!("↓{behind} {}", t!("git-commit-next-pull")),
        SyncState::Both { ahead, behind } => {
            format!("↓{behind} ↑{ahead} {}", t!("git-commit-next-both"))
        }
        SyncState::Publish => t!("git-commit-next-publish").to_owned(),
        SyncState::InSync | SyncState::NoRemote | SyncState::Hidden => String::new(),
    }
}

/// 同步控件的宽度（逻辑像素）；没有同步控件时为 `None`。
fn sync_width(app: &App, frame: &mut Frame<'_>) -> Option<f32> {
    let counts = sync_counts(app);
    let text = if counts.is_empty() {
        0.0
    } else {
        SYNC_TEXT_LEFT + frame.measure(FontKind::Ui, app.fonts.ui_small, &counts)
    };
    sync_shown(app).then_some(text + BUTTON_SMALL)
}

/// 绘制 `row` 右端的同步控件，返回它的左边缘。
///
/// 图标在行的最右端，左边写和上游差多少个提交和要做的事。有事可做时用强调色，其余时候用
/// 淡色；运行时图标在原位旋转，成功后过渡到对勾，再过渡到当前状态的图标。要做什么写在提示里。
pub(super) fn draw_sync(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> f32 {
    let p = app.palette;
    let state = app.git_sync_state();
    let counts = sync_counts(app);
    let width = sync_width(app, frame).unwrap_or(BUTTON_SMALL);
    let button = Rect::new(row.right() - width, row.y, width, row.h);
    let running = app.git.branch.running.is_some();
    let hovered = !running && app.pointer.hover == Hover::Commit(CommitTarget::Sync);
    draw_button(frame, p, button, clip, hovered, false);
    let done = app.git.branch.finished.filter(|_| !running);
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
    // 图标与要做的事一致：只推送时是上传，只拉取时是下载，两边都有或没有事可做时是同步。
    if leaving {
        icons::sync_turned(frame, icon, icons::spin_turn(), faded, clip);
    } else {
        match state {
            SyncState::Publish | SyncState::Push(_) => icons::upload(frame, icon, faded, clip),
            SyncState::Pull(_) => icons::download(frame, icon, faded, clip),
            _ => icons::sync(frame, icon, faded, clip),
        }
    }
    if check > 0.0 {
        icons::check(frame, icon, 11.0, p.git_added.alpha(check), clip);
    }
    hits.push((button, CommitTarget::Sync));
    button.x
}

/// 「上一次提交」一栏中提交信息离行左边的距离（逻辑像素）：前面是 13 宽的提交记号，左右各留 2
/// 和 6。
const LAST_COMMIT_TEXT: f32 = 21.0;

/// 绘制合并、变基等暂停时提交信息上方的说明条。
///
/// 说明条显示进行中的操作和仍有冲突的文件数，右端为「中止」。仍有冲突时底色和图标使用冲突色；
/// 冲突全部解决后改用淡强调色，提示用户可以提交。
pub(super) fn draw_banner(
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
    // 变基时写明进度：正在处理第几个提交，共几个。
    let op = match status.progress {
        Some((at, total)) => format!("{} {at}/{total}", operation.label()),
        None => operation.label().to_owned(),
    };
    let op = op.as_str();
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
    let hovered = app.pointer.hover == Hover::Commit(CommitTarget::Abort);
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
pub(super) fn draw_undo(
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
    // 前面一个提交的记号：这一行写的是上一次提交，不是一条普通的说明。
    let mark = (row.x + 8.5, row.y + row.h / 2.0);
    icons::ref_icon(
        frame,
        icons::RefMark::Commit,
        mark,
        p.text_dim,
        clip.intersect(room),
    );
    let subject = fit(frame, &last.subject, size, room.w - LAST_COMMIT_TEXT, false);
    let subject_clip = clip.intersect(room);
    ui_text_small(
        frame,
        app,
        row.x + LAST_COMMIT_TEXT,
        row,
        &subject,
        p.text_muted,
        subject_clip,
    );

    // 「撤销」写成字：最常用的一项，放进菜单里看不出能撤销。平时用次要色，不比提交按钮显眼，
    // 悬停时才用强调色。
    let hovered = app.pointer.hover == Hover::Commit(CommitTarget::Undo);
    draw_button(frame, p, undo, clip, hovered, false);
    let text = t!("git-commit-undo");
    let w = frame.measure(FontKind::Ui, size, text);
    let x = (undo.x + (undo.w - w) / 2.0).round();
    let ink = if hovered { p.accent } else { p.text_muted };
    ui_text_small(frame, app, x, undo, text, ink, clip);
    hits.push((undo, CommitTarget::Undo));

    let hovered = app.pointer.hover == Hover::Commit(CommitTarget::LastCommit);
    let open = app.overlays.menu_button == Some(crate::app::MenuButton::LastCommit);
    draw_button(frame, p, more, clip, hovered, open);
    let color = if hovered || open {
        p.text
    } else {
        p.text_muted
    };
    icons::more(frame, center(more), color, clip);
    hits.push((more, CommitTarget::LastCommit));
}
