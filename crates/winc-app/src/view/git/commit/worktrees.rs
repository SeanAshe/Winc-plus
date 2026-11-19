//! 工作树一组的绘制。

use super::*;

/// 工作树图标的中心离行左边的距离（逻辑像素）：图标 12 宽，左边缘与标题的文字对齐。
const WORKTREE_ICON: f32 = STASH_TITLE + 6.0;
/// 工作树的名字离行左边的距离（逻辑像素）：图标之后相隔 6。
const WORKTREE_NAME: f32 = STASH_TITLE + 18.0;

/// 标题行「新建工作树」的加号的边长（逻辑像素）：与设置页列表里的加号相同。
const PLUS: f32 = 8.0;
/// 行右端的文字离悬停按钮右边缘的距离（逻辑像素）：与标题行个数徽标里文字的右边距相同。
const COUNT_INSET: f32 = 6.0;

/// 工作树名字的末尾到分支图标中心的距离（逻辑像素），与仓库行相同。
const BRANCH_ICON: f32 = 14.0;
/// 工作树名字的末尾到分支名的距离（逻辑像素），与仓库行相同：图标宽约 10，与分支名相隔 3。
const BRANCH_TEXT: f32 = 22.0;
/// 分支名至少需要的宽度（逻辑像素）：约三个字符，不足时不显示分支。
const BRANCH_MIN: f32 = 24.0;
/// 锁定标记占的宽度（逻辑像素）：图标宽 9，左右各留约 5。
const LOCK_ROOM: f32 = 20.0;

/// 绘制工作树一组：上沿一条分隔线，标题是箭头、「工作树」和个数，展开时每个工作树一行。
///
/// 一行依次是图标、名字、「当前」标记或所在的分支和锁定标记，
/// 右端是不在当前分支上的提交数（已合并时显示「已合并」）和未提交的更改数；悬停时最右是「⋯」。
pub(super) fn draw_worktrees(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let Some(area) = parts.worktrees else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(area);
    let line = Rect::new(area.x, area.y, area.w, 1.0);
    frame.rect_clipped(line, p.separator, 0.0, clip);
    let size = app.fonts.ui_small;
    let band = |row: usize| {
        let y = area.y + crate::app::DRAWER_TOP + row as f32 * COMMIT_ROW;
        Rect::new(area.x + 8.0, y, area.w - 16.0, COMMIT_ROW)
    };
    let shade = |band: Rect| Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
    let places = app.worktree_places();

    let head = band(0);
    let on_title = matches!(
        app.pointer.hover,
        Hover::Commit(CommitTarget::Worktrees | CommitTarget::WorktreeNew)
    );
    if on_title {
        frame.rect_clipped(shade(head), p.hover, RADIUS_ROW, clip);
    }
    let cy = head.y + head.h / 2.0;
    let turned = app.worktree_chevron() * FRAC_PI_2;
    icons::chevron_turned(frame, (head.x + ARROW, cy), 6.0, turned, p.text_dim, clip);
    let title = t!("git-worktree-group");
    let at = (head.x + STASH_TITLE, head);
    let end = label(frame, app, size, at, title, p.text_muted, clip);
    let moving = app.git.commit.worktrees.motion.is_some();
    let open = app.git.commit.worktrees.open;
    if !open && !moving && app.git_other_changes() > 0 {
        let dot = Rect::new(end + 6.0, cy - 2.5, 5.0, 5.0);
        frame.rect_clipped(dot, p.accent, 2.5, clip);
    }
    if on_title {
        let (_, add) = file_actions(head);
        let on = app.pointer.hover == Hover::Commit(CommitTarget::WorktreeNew);
        draw_button(frame, p, add, clip, on, false);
        let color = if on { p.text } else { p.text_muted };
        icons::plus(frame, center(add), PLUS, color, clip);
        hits.push((add, CommitTarget::WorktreeNew));
    } else {
        draw_count_pill(app, frame, head, &places.len().to_string(), clip);
    }
    hits.push((head, CommitTarget::Worktrees));
    // 过渡中各行的位置尚未确定，不响应点击。
    if !open && !moving {
        return;
    }
    let mut still = Vec::new();
    let hits = if moving { &mut still } else { hits };

    for (n, line) in app.worktree_lines().into_iter().enumerate() {
        let row = band(n + 1);
        let i = match line {
            WorktreeLine::Place(i) => i,
            WorktreeLine::More(rest) => {
                if app.pointer.hover == Hover::Commit(CommitTarget::WorktreeMore) {
                    frame.rect_clipped(shade(row), p.hover, RADIUS_ROW, clip);
                }
                let text = t!("git-worktree-more", count = rest);
                let x = row.x + STASH_TITLE;
                ui_text_small(frame, app, x, row, &text, p.text_muted, clip);
                hits.push((row, CommitTarget::WorktreeMore));
                continue;
            }
        };
        let Some(place) = places.get(i) else {
            continue;
        };
        let hovered = matches!(
            app.pointer.hover,
            Hover::Commit(CommitTarget::WorktreeRow(h) | CommitTarget::WorktreeMenu(h)) if h == i
        );
        if hovered {
            frame.rect_clipped(shade(row), p.hover, RADIUS_ROW, clip);
        }
        let cy = row.y + row.h / 2.0;
        let state = app.place_state(&place.path);
        let locked = (place.worktree)
            .and_then(|w| app.git.worktrees.get(w))
            .is_some_and(|w| w.locked);
        // 右端是提交数和更改数，与标题行的个数右对齐；悬停时「⋯」在最右，二者移到它的左边。
        let mut right = row.right() - END_PAD - COUNT_INSET;
        if hovered {
            let (_, more) = file_actions(row);
            let on = app.pointer.hover == Hover::Commit(CommitTarget::WorktreeMenu(i));
            draw_button(frame, p, more, clip, on, false);
            let color = if on { p.text } else { p.text_muted };
            icons::more(frame, center(more), color, clip);
            hits.push((more, CommitTarget::WorktreeMenu(i)));
            right = more.x - 6.0;
        }
        // 不在当前分支上的提交数；已合并时显示「已合并」。
        let ahead = app.worktree_divergence(&place.path);
        let (sync, ink) = match ahead.map_or(0, |(_, ahead)| ahead) {
            0 if app.worktree_merged(place) => (t!("git-worktree-merged").to_owned(), p.text_dim),
            0 => (String::new(), p.text_dim),
            ahead => (format!("↑{ahead}"), p.accent),
        };
        let count = match (place.missing, state.changes) {
            (true, _) => t!("git-worktree-missing").to_owned(),
            (false, 0) => String::new(),
            (false, changes) => changes.to_string(),
        };
        for (text, ink) in [(count, p.text_dim), (sync, ink)] {
            if text.is_empty() {
                continue;
            }
            let w = frame.measure(FontKind::Ui, size, &text);
            ui_text_small(frame, app, right - w, row, &text, ink, clip);
            right -= w + 8.0;
        }
        let room = clip.intersect(Rect::new(row.x, row.y, (right - row.x).max(0.0), row.h));
        let ink = if place.current {
            p.accent
        } else {
            p.text_muted
        };
        icons::worktree_small(frame, (row.x + WORKTREE_ICON, cy), ink, room);
        // 名字放不下时截短；分支使用剩余的宽度，不足 [`BRANCH_MIN`] 时不显示。
        let ink = if place.missing { p.text_dim } else { p.text };
        let left = row.x + WORKTREE_NAME;
        let name = fit(
            frame,
            &place.label,
            app.fonts.ui,
            (right - left).max(0.0),
            false,
        );
        let x = ui_text(frame, app, left, row, &name, ink, room);
        if place.current {
            let text = t!("git-commit-repository-current");
            ui_text_small(frame, app, x + 8.0, row, text, p.accent, room);
        } else if let Some(head) = &state.head {
            let lock = if locked { LOCK_ROOM } else { 0.0 };
            let width = right - lock - (x + BRANCH_TEXT);
            if width >= BRANCH_MIN {
                icons::branch_small(frame, (x + BRANCH_ICON, cy), p.text_dim, room);
                let head = fit(frame, head, size, width, false);
                let end = ui_text_small(frame, app, x + BRANCH_TEXT, row, &head, p.text_dim, room);
                // 锁定标记跟在分支后面，表示该工作树的状态。
                if locked {
                    icons::lock(frame, (end + LOCK_ROOM / 2.0 + 2.0, cy), p.text_dim, room);
                }
            } else if locked {
                icons::lock(frame, (x + LOCK_ROOM / 2.0 + 2.0, cy), p.text_dim, room);
            }
        }
        hits.push((row, CommitTarget::WorktreeRow(i)));
    }
}
