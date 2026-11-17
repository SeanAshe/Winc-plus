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

/// 绘制标题行下面的 Git 上下文一行。
///
/// 左边是仓库和分支，组成一条路径；右边是同步控件。选择面板从这一行下面展开。
/// 分支用正文色，仓库用次要色。
pub(super) fn draw_context(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    widths: (f32, f32),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let clip = app.layout.sidebar;
    draw_branch_row(app, frame, parts.branch, widths, clip, hits);
}

/// 返回上下文一行 `row` 中仓库按钮和同步控件各自要求的宽度（逻辑像素）；没有该控件时为 0。
///
/// 放不下时仓库按钮让出宽度，但保留到整行的 [`LOCATION_SHARE`]（名字更短时到名字的宽度），
/// 不少于 [`LOCATION_MIN_TEXT`]；其余由分支名截短。
pub(super) fn context_widths(app: &App, frame: &mut Frame<'_>, row: Rect) -> (f32, f32) {
    if app.git.root.is_none() {
        return (0.0, 0.0);
    }
    let sync = if sync_shown(app) {
        sync_width(app, frame).unwrap_or(BUTTON_SMALL)
    } else {
        0.0
    };
    if app.git.repositories.len() < 2 {
        return (0.0, sync);
    }
    let size = app.fonts.ui_small;
    let right = row.right() - if sync > 0.0 { sync + BUTTON_GAP } else { 0.0 };
    let head = frame.measure(FontKind::Ui, size, &head_label(app));
    let branch = LOCATION_LEAD.1 + head + CONTEXT_PICK_TAIL;
    let button = |text: f32| LOCATION_PICK_LEAD.1 + text + CONTEXT_PICK_PAD;
    let full = button(frame.measure(FontKind::Ui, size, &app.git_current_repository_name()));
    let kept = full
        .min((right - row.x) * LOCATION_SHARE)
        .max(button(LOCATION_MIN_TEXT));
    let room = (right - PATH_SEPARATOR - branch - row.x)
        .max(kept)
        .min(right - row.x);
    (full.min(room).max(0.0), sync)
}

/// 绘制 Git 上下文一行左边的按钮：图标、名字，宽度贴合文字；`chevron` 为真时名字后面有下拉箭头。
///
/// `(center, room)` 是图标中心离按钮左边的距离和文字的左边距。`names` 是名字的各段和各自的颜色；
/// 不超过 `right`，放不下时从末尾截短。返回按钮的右边缘。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_context_pick(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    (icon_at, room_left): (f32, f32),
    (right, chevron): (f32, bool),
    target: CommitTarget,
    open: bool,
    icon: impl FnOnce(&mut Frame<'_>, (f32, f32), Color),
    names: &[(&str, Color)],
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> f32 {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let tail = if chevron {
        CONTEXT_PICK_TAIL
    } else {
        CONTEXT_PICK_PAD
    };
    let room = (right - row.x - room_left - tail).max(0.0);
    let full: f32 = names
        .iter()
        .map(|(text, _)| frame.measure(FontKind::Ui, size, text))
        .sum();
    let pick = Rect::new(row.x, row.y, room_left + full.min(room) + tail, row.h);
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
    if chevron {
        icons::chevron(frame, (x + 9.0, cy), 5.5, Direction::Down, p.text_dim, clip);
    }
    pick.right()
}

/// 仓库按钮与分支按钮之间分隔符所占的宽度（逻辑像素）：一个「/」，两侧紧贴两个按钮。
const PATH_SEPARATOR: f32 = 8.0;
/// 没有下拉箭头的上下文按钮里文字右边到按钮右边的留白（逻辑像素）。
const CONTEXT_PICK_PAD: f32 = 6.0;
/// 仓库按钮里图标中心和文字左边离按钮左边的距离（逻辑像素）：图标与项目徽章的中心对齐，文字
/// 紧跟图标，相隔 6。
const LOCATION_PICK_LEAD: (f32, f32) = (BADGE_CENTER, BADGE_CENTER + 12.0);
/// 上下文一行放不下时仓库名至少保留的宽度（逻辑像素）：约三个字符。
const LOCATION_MIN_TEXT: f32 = 24.0;
/// 上下文一行放不下时仓库按钮最多占的份额（0 到 1），其余留给分支。
///
/// 取自调参：两个名字都要截短时各自仍能辨认；分支名是这一行的主体，占多数。
const LOCATION_SHARE: f32 = 0.4;

/// 绘制上下文一行：左边是仓库按钮（有多个仓库时）、分隔符和分支按钮，右端是同步控件。
///
/// `widths` 是仓库按钮和同步控件此刻的宽度（见 [`context_widths`]、`App::track_context_row`）。
pub(super) fn draw_branch_row(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    (location, sync): (f32, f32),
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
        right = draw_sync(app, frame, row, sync, clip, hits) - BUTTON_GAP;
    }
    let head = head_label(app);
    let names = vec![(head.as_str(), p.text)];
    let size = app.fonts.ui_small;
    let name = app.git_current_repository_name();
    let (mut row, mut lead) = (row, (BADGE_CENTER, BADGE_ROOM));
    if app.git.repositories.len() >= 2 {
        draw_context_pick(
            app,
            frame,
            row,
            LOCATION_PICK_LEAD,
            ((row.x + location).min(right), false),
            CommitTarget::Location,
            app.picker_lists_repositories(),
            |frame, center, color| icons::folder_small(frame, center, color, clip),
            &[(name.as_str(), p.text_muted)],
            clip,
            hits,
        );
        // 分隔符和分支按钮跟着过渡中的宽度走，名字比它短时也不提前靠拢。
        let end = (row.x + location).min(right);
        let slash = frame.measure(FontKind::Ui, size, "/");
        let at = end + (PATH_SEPARATOR - slash) / 2.0;
        label(frame, app, size, (at, row), "/", p.text_dim, clip);
        let left = end + PATH_SEPARATOR;
        row = Rect::new(left, row.y, (row.right() - left).max(0.0), row.h);
        lead = LOCATION_LEAD;
    }
    draw_context_pick(
        app,
        frame,
        row,
        lead,
        (right, true),
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

/// 绘制 `row` 右端宽 `width` 的同步控件，返回它的左边缘。
///
/// 图标在行的最右端，左边写和上游差多少个提交和要做的事。有事可做时用强调色，其余时候用
/// 淡色；运行时图标在原位旋转，成功后过渡到对勾，再过渡到当前状态的图标。要做什么写在提示里。
pub(super) fn draw_sync(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    width: f32,
    clip: Rect,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> f32 {
    let p = app.palette;
    let state = app.git_sync_state();
    let counts = sync_counts(app);
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

/// 绘制「上一次提交」一栏：提交信息的第一行、提交时间，右端是「撤销」和「⋯」。
///
/// 位于视图的最底部；没有提交卡片时上沿有一条分隔线。
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
    if parts.card.is_none() {
        frame.rect_clipped(Rect::new(bar.x, bar.y, bar.w, 1.0), p.separator, 0.0, clip);
    }
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
