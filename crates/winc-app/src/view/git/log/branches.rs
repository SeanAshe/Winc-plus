//! 日志左侧的分支面板：「全部分支」和分支选择器里的各组，按文件列表的样子排成树，正在看的那一个
//! 铺选中的底色。

use super::files::{
    LABEL, Label, Lane, ROW_PAD, arrow_x, draw_laid, label_clip, lay_label, row_base,
};
use super::*;
use crate::app::git::log::RowName;
use crate::app::git::log::{BRANCH_ROW, BranchRow};
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

/// 绘制分支面板，返回最大滚动距离（逻辑像素）；名字展开后仍写不下的行记进 `long`。
///
/// 各行按文件列表的样子排成树（见 `files::draw_file_list`）：分组标题是折叠箭头、小号淡色的
/// 组名和右端的数量徽标，组里的分支、标签和按 `/` 分出的目录缩进一层，上级各层有缩进参考线；
/// 当前分支打勾，其余分支、标签前面是淡色的记号。正在按它筛选的行铺选中的底色，没有按分支筛选
/// 时「全部分支」铺选中的底色。放不下的名字在截断处淡出，鼠标停着时展开成全名，可以伸到提交
/// 列表上面。组、目录展开、收起时子行逐渐露出（收回），下面的行随之移动。
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
    // 右边缘可以左右拖动，悬停或拖动时改为较粗的强调色线条（与列表、详情之间的分隔线相同）。
    let resizing = app.log_branches_resizing();
    let edge_hovered = app.pointer.hover == Hover::Log(LogTarget::BranchesEdge);
    if resizing || edge_hovered {
        frame.rect(Rect::new(area.right() - 1.5, area.y, 2.0, area.h), p.accent);
    } else {
        frame.rect(
            Rect::new(area.right() - 1.0, area.y, 1.0, area.h),
            p.separator,
        );
    }
    let area = Rect::new(area.x, area.y, area.w - 1.0, area.h);
    // 打开、关闭途中面板只露出右边一段：各行按完整的宽度排，跟着右边缘从左边滑进、滑出。
    let full = parts.branches_w - 1.0;
    let slid = Rect::new(area.right() - full, area.y, full, area.h);
    // 展开、收起途中按行数较多的那一版画（见 `PanelFold`）。
    let fold = app.git.log.branch_fold.as_ref();
    let rows = fold.map_or_else(|| app.log_branch_rows(), |f| f.rows.clone());
    let opened = parts.branches.w >= parts.branches_w;
    let lane = Lane {
        names: &app.git.log.branch_names,
        content: (slid.x + 6.0 + ROW_PAD, slid.w - 12.0 - 2.0 * ROW_PAD),
        clip: area,
        // 全名可以伸到提交列表上面，右边留 8 逻辑像素；打开、关闭途中不伸出。
        spill: opened.then_some(parts.list.right() - 8.0),
    };
    let filter = &view.query.branches;
    let top = area.y + LOG_LIST_PAD - drawn_scroll(app.git.log.branches_scroll);
    let idle = app.idle_pointer();
    let mut spills = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let (y, clip) = fold_slot(fold, (top, i), area);
        let band = Rect::new(slid.x + 6.0, y, slid.w - 12.0, BRANCH_ROW);
        if clip.h <= 0.0 || band.bottom() < clip.y || band.y > clip.bottom() {
            continue;
        }
        let target = LogTarget::BranchRow(i);
        // 动画期间行在移动，不画悬停、不记命中范围。
        let hovered = fold.is_none() && idle && app.pointer.hover == Hover::Log(target);
        let named = |name: Option<&String>| name.is_some_and(|n| filter.contains(n));
        let chosen = match row {
            BranchRow::All => filter.is_empty(),
            BranchRow::Branch { index, .. } => named(app.git.branches.get(*index).map(|b| &b.name)),
            BranchRow::Tag(index) => named(app.git.tags.get(*index)),
            _ => false,
        };
        let shown = BranchLine {
            lane: &lane,
            row: band,
            hovered,
            color: row_color(p, chosen, false, hovered),
            clip,
            turned: match fold {
                Some(f) if f.parent == i => Some(f.reveal()),
                _ => None,
            },
        };
        let out = draw_branch_row(app, frame, row, &shown);
        if out.cut {
            long.push(i);
        }
        if fold.is_none() {
            if let Some(spill) = out.spill.filter(|_| hovered) {
                spills.push((spill, target));
            }
            hits.push((band.intersect(area), target));
        }
    }
    // 右边缘的命中范围向两侧各扩展 3 逻辑像素，放在命中表最前面，先于各行判定；打开、关闭
    // 途中不能拖动。
    if parts.branches.w >= parts.branches_w {
        let edge = Rect::new(area.right() - 3.0, area.y, 6.0, area.h);
        hits.insert(0, (edge, LogTarget::BranchesEdge));
    }
    // 伸到提交列表上面的名字盖住了右边缘和下面的提交，先判定。
    for spill in spills {
        hits.insert(0, spill);
    }
    let content = LOG_LIST_PAD + rows.len() as f32 * BRANCH_ROW + LOG_LIST_PAD;
    (content - area.h).max(0.0)
}

/// 分支面板里一行的位置和状态。
struct BranchLine<'a> {
    lane: &'a Lane<'a>,
    /// 行的范围（底色铺满它）。
    row: Rect,
    hovered: bool,
    /// 行的底色；没有时为 `None`。
    color: Option<Color>,
    /// 能画的范围：展开、收起途中只到已露出的一段。
    clip: Rect,
    /// 这一行正在展开、收起时箭头转过的比例；没有时按是否展开画。
    turned: Option<f32>,
}

/// 一行画完的结果：名字伸出面板的那一段，名字展开后是否仍写不下。
struct BranchOut {
    spill: Option<Rect>,
    cut: bool,
}

/// 画分支面板的一行 `row`。
fn draw_branch_row(
    app: &App,
    frame: &mut Frame<'_>,
    row: &BranchRow,
    line: &BranchLine<'_>,
) -> BranchOut {
    let p = app.palette;
    let (x, _) = line.lane.content;
    let band = line.row;
    let cy = band.y + BRANCH_ROW / 2.0;
    let chevron = |frame: &mut Frame<'_>, depth: usize, open: bool| {
        let turned = line.turned.unwrap_or(f32::from(u8::from(open)));
        let at = (arrow_x(x, depth), cy);
        icons::chevron_turned(frame, at, 6.0, turned * FRAC_PI_2, p.text_dim, line.clip);
    };
    let plain = |frame: &mut Frame<'_>, depth: usize| {
        row_base(
            app,
            frame,
            line.lane,
            (band, depth, band.x),
            line.color,
            line.clip,
        );
    };
    let group = |frame: &mut Frame<'_>, title: &str, count: usize, open: bool| {
        plain(frame, 0);
        chevron(frame, 0, open);
        let text = arrow_x(x, 0) + 10.0;
        ui_text_small(frame, app, text, band, title, p.text_dim, line.clip);
        let colors = (p.hover, p.text_muted);
        draw_count_pill_in(app, frame, band, &count.to_string(), colors, line.clip);
        BranchOut {
            spill: None,
            cut: false,
        }
    };
    match row {
        BranchRow::All => {
            plain(frame, 0);
            let text = arrow_x(x, 0) + 10.0;
            let title = t!("git-log-all-branches");
            ui_text(frame, app, text, band, title, p.text, line.clip);
            BranchOut {
                spill: None,
                cut: false,
            }
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
            group(frame, title, *count, *open)
        }
        BranchRow::Remotes { count, open } => {
            group(frame, t!("git-branch-picker-remote"), *count, *open)
        }
        BranchRow::Tags { count, open } => {
            group(frame, t!("git-branch-picker-tags"), *count, *open)
        }
        // 分组里的目录：同文件列表的目录行，箭头、打开或合上的文件夹和名字。
        BranchRow::Folder {
            remote,
            path,
            label,
            depth,
            open,
            ..
        } => {
            let depth = depth + 1;
            let icon = arrow_x(x, depth) + 10.0;
            let key = RowName::Ref(format!("{remote}/{path}/"));
            let text = (label.as_str(), None);
            let out = draw_ref_name(app, frame, line, (key, true, text), (icon, depth));
            chevron(frame, depth, *open);
            icons::folder(
                frame,
                (icon, (cy - 6.0).round()),
                *open,
                p.folder,
                line.clip,
            );
            out
        }
        // 当前分支打勾，其余分支前面是淡色的分支记号（远程分支用远程的记号）。
        BranchRow::Branch {
            index,
            depth,
            label,
        } => {
            let depth = depth + 1;
            let icon = arrow_x(x, depth) + 10.0;
            let branch = app.git.branches.get(*index);
            let key = RowName::Ref(branch.map_or_else(String::new, |b| b.name.clone()));
            let text = (label.as_str(), None);
            let out = draw_ref_name(app, frame, line, (key, false, text), (icon, depth));
            let at = (icon + icons::FOLDER_WIDTH / 2.0, cy);
            match branch {
                Some(b) if b.current => icons::check(frame, at, 9.0, p.text, line.clip),
                Some(b) => {
                    let mark = match b.remote {
                        true => icons::RefMark::Remote,
                        false => icons::RefMark::Branch,
                    };
                    icons::ref_icon(frame, mark, at, p.text_dim, line.clip);
                }
                None => {}
            }
            out
        }
        BranchRow::Tag(index) => {
            let icon = arrow_x(x, 1) + 10.0;
            let tag = app.git.tags.get(*index).map_or("", String::as_str);
            let key = RowName::Ref(format!("refs/tags/{tag}"));
            let out = draw_ref_name(app, frame, line, (key, false, (tag, None)), (icon, 1));
            let at = (icon + icons::FOLDER_WIDTH / 2.0, cy);
            icons::ref_icon(frame, icons::RefMark::Tag, at, p.text_dim, line.clip);
            out
        }
        BranchRow::Create(_) => BranchOut {
            spill: None,
            cut: false,
        },
    }
}

/// 画一行的底色、缩进参考线和名字（`key`、是否目录、文字）：图标从 `icon` 开始，在树里的第
/// `depth` 层。名字放不下时同文件列表在截断处淡出，鼠标停着时展开成全名。
fn draw_ref_name(
    app: &App,
    frame: &mut Frame<'_>,
    line: &BranchLine<'_>,
    (key, folder, text): (RowName, bool, (&str, Option<&str>)),
    (icon, depth): (f32, usize),
) -> BranchOut {
    let band = line.row;
    let label = Label {
        key,
        folder,
        text,
        color: app.palette.text,
        span: (icon + LABEL, band.right() - ROW_PAD),
    };
    let laid = lay_label(app, frame, line.lane, (&label, band, line.hovered));
    let place = (band, depth, laid.end);
    let spill = row_base(app, frame, line.lane, place, line.color, line.clip);
    let clip = label_clip(line.lane, line.clip);
    draw_laid(app, frame, (&label, &laid, band), line.color, clip);
    BranchOut {
        spill,
        cut: laid.cut,
    }
}
