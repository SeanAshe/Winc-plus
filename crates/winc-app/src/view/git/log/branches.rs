//! 日志左侧的分支面板：「全部分支」，其余各行与分支选择器相同，正在看的那一个铺选中的底色。

use super::*;
use crate::app::git::log::{BRANCH_ROW, BRANCHES_W, BranchRow};
use crate::view::git::commit::draw_count_pill_in;

/// 绘制标题栏最左边显示、隐藏分支面板的按钮：面板显示着时按下的样子。
pub(super) fn draw_branches_button(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &LogParts,
    hits: &mut Vec<(Rect, LogTarget)>,
) {
    let p = app.palette;
    let button = parts.branches_button();
    let hovered = app.pointer.hover == Hover::Log(LogTarget::BranchesToggle);
    // 按要切换到的状态画：关闭途中按钮已经弹起，打开途中已经按下。
    let on = app.state.log_branches.unwrap_or(true) && parts.branches.w > 0.0;
    draw_button(frame, p, button, parts.header, hovered, on);
    let ink = if hovered || on { p.text } else { p.text_muted };
    icons::sidebar(frame, center(button), on, ink, parts.header);
    hits.push((button, LogTarget::BranchesToggle));
}

/// 绘制分支面板，返回最大滚动距离（逻辑像素）；名字截短了的行记进 `long`。
///
/// 各行的排法和画法与分支选择器相同：分组标题是折叠箭头、小号淡色的组名和右端的数量徽标，分支
/// 名按 `/` 分出的目录有文件夹图标，当前分支在名字前打勾，其余分支、标签前面是淡色的记号。正在
/// 按它筛选的行铺选中的底色，没有按分支筛选时「全部分支」铺选中的底色；行的底色上下各让出 1，
/// 相邻两行之间留出间隔。组、目录展开、收起时子行逐渐露出（收回），下面的行随之移动。
pub(super) fn draw_branches(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    parts: &LogParts,
    (hits, long): (&mut Vec<(Rect, LogTarget)>, &mut Vec<usize>),
) -> f32 {
    let area = parts.branches;
    if area.w <= 0.0 {
        return 0.0;
    }
    let p = app.palette;
    frame.rect(
        Rect::new(area.right() - 1.0, area.y, 1.0, area.h),
        p.separator,
    );
    let area = Rect::new(area.x, area.y, area.w - 1.0, area.h);
    // 打开、关闭途中面板只露出右边一段：各行按完整的宽度排，跟着右边缘从左边滑进、滑出。
    let slid = Rect::new(
        area.right() - (BRANCHES_W - 1.0),
        area.y,
        BRANCHES_W - 1.0,
        area.h,
    );
    // 展开、收起途中按行数较多的那一版画（见 `PanelFold`）。
    let fold = app.git.log.branch_fold.as_ref();
    let rows = fold.map_or_else(|| app.log_branch_rows(), |f| f.rows.clone());
    let filter = &view.query.branches;
    let top = area.y + LOG_LIST_PAD - app.git.log.branches_scroll;
    for (i, row) in rows.iter().enumerate() {
        let (y, clip) = panel_slot(fold, (top, i), area);
        let band = Rect::new(slid.x + 6.0, y, slid.w - 12.0, BRANCH_ROW);
        if clip.h <= 0.0 || band.bottom() < clip.y || band.y > clip.bottom() {
            continue;
        }
        let target = LogTarget::BranchRow(i);
        // 动画期间行在移动，不画悬停、不记命中范围。
        let hovered = fold.is_none() && app.pointer.hover == Hover::Log(target);
        let named = |name: Option<&String>| name.is_some_and(|n| filter.contains(n));
        let chosen = match row {
            BranchRow::All => filter.is_empty(),
            BranchRow::Branch { index, .. } => named(app.git.branches.get(*index).map(|b| &b.name)),
            BranchRow::Tag(index) => named(app.git.tags.get(*index)),
            _ => false,
        };
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        if let Some(color) = row_color(p, chosen, false, hovered) {
            frame.rect_clipped(shade, color, RADIUS_ROW, clip);
        }
        let cy = band.y + BRANCH_ROW / 2.0;
        // 图标一列和文字一列，每深一层缩进 `TREE_INDENT`（与分支选择器相同）。
        let columns = |depth: usize| {
            let indent = depth as f32 * crate::theme::TREE_INDENT;
            (band.x + 14.0 + indent, band.x + 26.0 + indent)
        };
        let right = band.right() - 8.0;
        let name = |frame: &mut Frame<'_>, x: f32, text: &str, long: &mut Vec<usize>| {
            // 名字和分支选择器一样用正文字号，组名、数量才用小号字。
            let shown = truncate_end(frame, text, app.fonts.ui, (right - x).max(0.0));
            if shown != text {
                long.push(i);
            }
            ui_text(frame, app, x, band, &shown, p.text, clip);
        };
        let turned = |open: bool| match fold {
            Some(f) if f.parent == i => f.reveal(),
            _ => f32::from(u8::from(open)),
        };
        let group = |frame: &mut Frame<'_>, title: &str, count: usize, open: bool| {
            // 和分支选择器一样，每组上方一条分隔线与上一组隔开。
            let line = Rect::new(band.x + 6.0, band.y, band.w - 12.0, 1.0);
            frame.rect_clipped(line, p.separator, 0.0, clip);
            let (arrow, text) = columns(0);
            let turn = turned(open) * FRAC_PI_2;
            icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, clip);
            ui_text_small(frame, app, text, band, title, p.text_dim, clip);
            let pill = Rect::new(band.x, band.y, band.w, band.h);
            draw_count_pill_in(
                app,
                frame,
                pill,
                &count.to_string(),
                (p.hover, p.text_muted),
                clip,
            );
        };
        match row {
            BranchRow::All => {
                let text = columns(0).1;
                ui_text(
                    frame,
                    app,
                    text,
                    band,
                    t!("git-log-all-branches"),
                    p.text,
                    clip,
                );
            }
            BranchRow::Locals {
                recent,
                count,
                open,
            } => {
                let title = match recent {
                    true => t!("git-branch-picker-recent"),
                    false => t!("git-branch-picker-other"),
                };
                group(frame, title, *count, *open);
            }
            BranchRow::Remotes { count, open } => {
                group(frame, t!("git-branch-picker-remote"), *count, *open);
            }
            BranchRow::Tags { count, open } => {
                group(frame, t!("git-branch-picker-tags"), *count, *open);
            }
            // 分组里的目录：箭头在图标左边一层，文件夹图标和同层的分支对齐。
            BranchRow::Folder {
                label, depth, open, ..
            } => {
                let depth = depth + 1;
                let arrow = columns(depth - 1).0;
                let turn = turned(*open) * FRAC_PI_2;
                icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, clip);
                let (icon, text) = columns(depth);
                icons::folder_small(frame, (icon, cy), p.text_dim, clip);
                name(frame, text, label, long);
            }
            // 当前分支打勾，其余分支前面是淡色的分支记号（远程分支用远程的记号）。
            BranchRow::Branch {
                index,
                depth,
                label,
            } => {
                let (icon, text) = columns(depth + 1);
                match app.git.branches.get(*index) {
                    Some(b) if b.current => icons::check(frame, (icon, cy), 9.0, p.text, clip),
                    Some(b) => {
                        let mark = match b.remote {
                            true => icons::RefMark::Remote,
                            false => icons::RefMark::Branch,
                        };
                        icons::ref_icon(frame, mark, (icon, cy), p.text_dim, clip);
                    }
                    None => {}
                }
                name(frame, text, label, long);
            }
            BranchRow::Tag(index) => {
                let (icon, text) = columns(1);
                icons::ref_icon(frame, icons::RefMark::Tag, (icon, cy), p.text_dim, clip);
                let tag = app.git.tags.get(*index).map_or("", String::as_str);
                name(frame, text, tag, long);
            }
            BranchRow::Create(_) => {}
        }
        if fold.is_none() {
            hits.push((band.intersect(area), target));
        }
    }
    let content = LOG_LIST_PAD + rows.len() as f32 * BRANCH_ROW + LOG_LIST_PAD;
    (content - area.h).max(0.0)
}

/// 分支面板第 `i` 行的上沿和能画的范围：展开、收起途中子行位置不动、只露出已露出的一段，其后
/// 的行往上收还没露出的高度（与提交视图里组、目录的动画相同）。
fn panel_slot(
    fold: Option<&crate::app::git::log::PanelFold>,
    (top, i): (f32, usize),
    area: Rect,
) -> (f32, Rect) {
    let y = top + i as f32 * BRANCH_ROW;
    let Some(fold) = fold else {
        return (y, area);
    };
    let (first, last) = (fold.parent + 1, fold.parent + fold.count);
    if i > last {
        return (y - fold.hidden(), area);
    }
    if i < first {
        return (y, area);
    }
    let start = top + first as f32 * BRANCH_ROW;
    let shown = fold.count as f32 * BRANCH_ROW - fold.hidden();
    (
        y,
        Rect::new(area.x, start, area.w, shown.max(0.0)).intersect(area),
    )
}
