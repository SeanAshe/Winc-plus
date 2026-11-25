//! 多仓库时改动列表中各仓库的标题行，以及其他仓库的分组行、文件行。

use super::*;
use crate::app::{OtherGroup, OtherRow};

/// 绘制多仓库时当前仓库改动列表以外的各行，记下可点击的位置。
///
/// 各行按 `App::commit_other_rows` 的顺序排，随列表一起滚动。当前仓库标题行之后的各行让出
/// 当前仓库改动列表的高度 `block`（逻辑像素）。仓库标题在最外一层，组、平铺的文件缩进一层，
/// 树形模式中的目录和文件再按层级缩进，与当前仓库的排法相同。
pub(super) fn draw_other_rows(
    app: &App,
    frame: &mut Frame<'_>,
    list: Rect,
    block: f32,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let layout = OtherLayout::new(app, list, block);
    // 展开、收起的动画期间只记下动画所在标题行及其上方各行的命中区域：它们的位置不变。
    let motion = app.git.commit.fold_motion.as_ref();
    let settled = motion.map_or(layout.rows.len(), |m| m.parent + 1);
    for (i, row) in layout.rows.iter().enumerate() {
        let y = layout.top(i);
        let clip = layout.clip(i);
        if y + COMMIT_ROW < clip.y || y > clip.bottom() || clip.h <= 0.0 {
            continue;
        }
        let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
        let mut row_hits = Vec::new();
        frame.within(clip, |frame| {
            let hits = &mut row_hits;
            if let OtherRow::Dir(.., depth) | OtherRow::TreeFile(.., depth) = row {
                for level in 1..=*depth {
                    let x = (band.x + ARROW + (level + 1) as f32 * INDENT).round();
                    frame.rect_clipped(
                        Rect::new(x, band.y, 1.0, COMMIT_ROW),
                        app.palette.indent_guide,
                        0.0,
                        list,
                    );
                }
            }
            match *row {
                OtherRow::Current => draw_current_row(app, frame, (list, band), hits),
                OtherRow::Repository(repo) => {
                    draw_repository_row(app, frame, (list, band), repo, false, hits);
                }
                OtherRow::Quiet(repo) => {
                    draw_repository_row(app, frame, (list, band), repo, true, hits)
                }
                OtherRow::QuietGroup => draw_quiet_group(app, frame, (list, band), hits),
                OtherRow::Group(repo, group) => {
                    draw_other_group(app, frame, (list, band), (repo, group), hits);
                }
                OtherRow::Staged(repo, entry) => {
                    draw_other_file(app, frame, (list, band), (repo, entry, Some(true), 0), hits);
                }
                OtherRow::File(repo, entry) => {
                    let side = app.git.commit.staging().then_some(false);
                    draw_other_file(app, frame, (list, band), (repo, entry, side, 0), hits);
                }
                OtherRow::Dir(repo, group, ref dir, ref label, depth) => {
                    let target = CommitTarget::OtherDir(i);
                    let staged = (app.git.commit.staging()).then_some(group == OtherGroup::Staged);
                    let actions = other_dir_actions(i, staged);
                    let hovered = app.pointer.hover == Hover::Commit(target)
                        || matches!(app.pointer.hover, Hover::Commit(t) if actions.holds(t));
                    if hovered {
                        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
                        frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
                    }
                    let cy = band.y + COMMIT_ROW / 2.0;
                    let arrow = band.x + ARROW + (depth + 1) as f32 * INDENT;
                    let open = !app.other_dir_folded(repo, group, dir);
                    icons::chevron_turned(
                        frame,
                        (arrow, cy),
                        6.0,
                        f32::from(u8::from(open)) * std::f32::consts::FRAC_PI_2,
                        app.palette.text_dim,
                        list,
                    );
                    let icon_x = arrow + check_gap(app) + ICON;
                    icons::folder(
                        frame,
                        (icon_x, (cy - 6.0).round()),
                        open,
                        app.palette.folder,
                        list,
                    );
                    let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
                    let strip = hovered.then(|| actions_home(band, actions.len(), 0.0));
                    let end = strip.map_or(band.right() - 6.0, |s| s.first.x - 4.0);
                    let room = (end - text_x).max(0.0);
                    let parts = dir_parts(app, frame, label, Some(room));
                    draw_dir_parts(app, frame, (text_x, band), &parts, |_| 1.0, list);
                    if let Some(strip) = strip {
                        draw_row_actions(app, frame, (strip, actions), (list, false), Some(hits));
                    }
                    hits.push((band.intersect(list), target));
                }
                OtherRow::TreeFile(repo, entry, group, depth) => {
                    let side = app
                        .git
                        .commit
                        .staging()
                        .then_some(group == OtherGroup::Staged);
                    draw_other_file(app, frame, (list, band), (repo, entry, side, depth), hits);
                }
            }
        });
        if i < settled {
            hits.extend(row_hits);
        }
    }
}

/// 多仓库时当前仓库改动列表以外各行的排法：第几行画在哪里、露出哪一段。
///
/// 当前仓库标题行之后的各行让出它的改动列表的高度。某一节或某一组正在展开、收起时按动画中的
/// 那一版排：子行位置不动、只露出已露出的一段，其后各行随之移动。
pub(super) struct OtherLayout {
    /// 要画的各行：动画期间是行数较多的那一版。
    pub rows: Vec<OtherRow>,
    list: Rect,
    scroll: f32,
    /// 当前仓库标题行及其上方的行数。
    before: usize,
    /// 当前仓库改动列表的高度（逻辑像素）。
    block: f32,
    /// 正在展开、收起的其他仓库的标题行。
    fold: Option<Fold>,
}

/// 其他仓库的一节或一组展开、收起中的一帧。
#[derive(Clone, Copy)]
struct Fold {
    /// 标题行的下标。
    parent: usize,
    /// 子行数。
    count: usize,
    /// 还没露出来的高度（逻辑像素）。
    hidden: f32,
}

impl OtherLayout {
    /// 当前仓库改动列表高 `block`；当前仓库一节正在展开、收起时改按动画中的高度。
    pub(super) fn new(app: &App, list: Rect, block: f32) -> Self {
        let motion = app.git.commit.fold_motion.as_ref();
        let rows = motion.map_or_else(|| app.commit_other_rows(), |m| m.rows.clone());
        let before = rows
            .iter()
            .position(|r| *r == OtherRow::Current)
            .map_or(0, |at| at + 1);
        let (block, fold) = match motion {
            Some(m) if m.count == 0 => (m.height - m.hidden(), None),
            Some(m) => {
                let fold = Fold {
                    parent: m.parent,
                    count: m.count,
                    hidden: m.hidden(),
                };
                (block, Some(fold))
            }
            None => (block, None),
        };
        Self {
            rows,
            list,
            scroll: app.git.commit.scroll,
            before,
            block,
            fold,
        }
    }

    /// 第 `i` 行的上沿。
    pub(super) fn top(&self, i: usize) -> f32 {
        let mut y = self.list.y + i as f32 * COMMIT_ROW - self.scroll;
        if i >= self.before {
            y += self.block;
        }
        if let Some(f) = self.fold
            && i > f.parent + f.count
        {
            y -= f.hidden;
        }
        y
    }

    /// 第 `i` 行能画的范围：正在展开、收起的子行只画已露出的一段。
    fn clip(&self, i: usize) -> Rect {
        match self.fold {
            Some(f) if i > f.parent && i <= f.parent + f.count => {
                let y = self.top(f.parent) + COMMIT_ROW;
                let h = f.count as f32 * COMMIT_ROW - f.hidden;
                Rect::new(self.list.x, y, self.list.w, h.max(0.0)).intersect(self.list)
            }
            _ => self.list,
        }
    }

    /// 当前仓库改动列表的上沿：紧接在它的标题行之下；不分仓库时就是列表内容的顶部。
    pub(super) fn block_top(&self) -> f32 {
        match self.before {
            0 => self.list.y - self.scroll,
            at => self.top(at - 1) + COMMIT_ROW,
        }
    }

    /// 当前仓库改动列表的高度（逻辑像素）：动画中的一帧按动画中的高度。
    pub(super) fn block(&self) -> f32 {
        self.block
    }
}

/// 仓库标题行各部分的横坐标：折叠箭头的中心、勾选框的中心（勾选式下才有）、图标的左边、名字
/// 的左边。与当前仓库改动列表最外一层（组标题）的箭头、勾选框同列。
///
/// `quiet` 时是「没有更改的仓库」一组里的一行：没有箭头和勾选框，图标缩进一层。
fn header_columns(app: &App, band: Rect, quiet: bool) -> (f32, f32, f32, f32) {
    let arrow = band.x + ARROW;
    let check = arrow + check_gap(app);
    let icon = if quiet { arrow + ICON } else { check + ICON };
    (arrow, check, icon, icon + 6.0 + ICON)
}

/// 绘制当前仓库的标题行：折叠箭头、勾选框（勾选式下）、文件夹图标、仓库名、所在的分支和与上游
/// 的差异，右端是更改数；与其他仓库的标题行排法相同，以强调色的图标和整节的底色（见
/// [`draw_sections`]）区分。点击箭头收起或展开整节，点击分支展开分支选择面板。
///
/// 没有更改时排在「没有更改的仓库」一组里：没有箭头和勾选框，右端不写更改数。
fn draw_current_row(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let status = app.git.status.as_ref();
    let total = status.map_or(0, |s| s.entries.len());
    let quiet = total == 0;
    let target = CommitTarget::CurrentRepository;
    let hovered = matches!(
        app.pointer.hover,
        Hover::Commit(CommitTarget::CurrentFold | CommitTarget::CurrentCheck)
    );
    if !quiet {
        draw_header_band(app, frame, (list, band), hovered);
    }
    let cy = band.y + COMMIT_ROW / 2.0;
    let (arrow, check, icon, name_x) = header_columns(app, band, quiet);
    if !quiet {
        let open = !app.current_section_folded();
        let turn = app.other_fold_turn(OtherRow::Current, open) * std::f32::consts::FRAC_PI_2;
        icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, list);
        hits.push((fold_area(band).intersect(list), CommitTarget::CurrentFold));
        if check_gap(app) > 0.0 {
            draw_other_checkbox(app, frame, (check, cy), app.current_check_state(), list);
            let hit = Rect::new(check - 8.0, band.y + 4.0, 16.0, 16.0);
            hits.push((hit.intersect(list), CommitTarget::CurrentCheck));
        }
    }
    let count = if quiet {
        String::new()
    } else {
        total.to_string()
    };
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count) + 2.0 * END_CENTER;
    let clip = Rect::new(list.x, band.y, band.right() - count_w - list.x, COMMIT_ROW);
    let clip = clip.intersect(list);
    icons::folder_small(frame, (icon + 6.0, cy), p.accent, clip);
    // 仓库名用正文字号，比下面小号淡色的组名高一级。
    let name = app.git_current_repository_name();
    let end = ui_text(frame, app, name_x, band, &name, p.text, clip);
    let state = (app.git.root.as_deref()).map(|root| app.place_state(root));
    let state = state.unwrap_or_default();
    let branch = CommitTarget::CurrentBranch;
    draw_branch_sync(app, frame, (band, clip), end, (&state, branch), hits);
    if !quiet {
        draw_count_pill(app, frame, band, &count, list);
    }
    if !quiet {
        hits.push((band.intersect(list), target));
    }
}

/// 绘制多仓库时当前仓库一节的底色：一张圆角卡片，覆盖它的标题行和改动列表，先于各行绘制。
///
/// 当前仓库的改动列表高 `block`（逻辑像素）；它没有更改时卡片只有标题一行。
pub(super) fn draw_sections(app: &App, frame: &mut Frame<'_>, list: Rect, block: f32) {
    let layout = OtherLayout::new(app, list, block);
    let Some(at) = layout.rows.iter().position(|r| *r == OtherRow::Current) else {
        return;
    };
    let clip = layout.clip(at);
    let height = COMMIT_ROW + layout.block();
    let card = Rect::new(
        list.x + 8.0,
        layout.top(at) + 1.0,
        list.w - 16.0,
        height - 2.0,
    );
    if card.bottom() < clip.y || card.y > clip.bottom() || clip.h <= 0.0 {
        return;
    }
    frame.rect_clipped(card, current_fill(app), RADIUS_ROW, clip);
}

/// 当前仓库一节卡片的底色。
pub(super) fn current_fill(app: &App) -> Color {
    app.palette.accent.alpha(0.08)
}

/// 有更改的仓库的标题行悬停时铺悬停的底色。
fn draw_header_band(app: &App, frame: &mut Frame<'_>, (list, band): (Rect, Rect), hovered: bool) {
    if hovered {
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
    }
}

/// 鼠标停在 `target` 上时给这一行铺悬停的底色。
fn shade_hovered(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    target: CommitTarget,
) {
    if app.pointer.hover == Hover::Commit(target) {
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
    }
}

/// 仓库标题行左端折叠箭头的点击区域：从行的左边到箭头右侧 8，与行同高。
fn fold_area(band: Rect) -> Rect {
    Rect::new(band.x, band.y, ARROW + 8.0, band.h)
}

/// 画另一个仓库的标题行：折叠箭头、勾选框（勾选式下）、文件夹图标、仓库名、所在的分支和与
/// 上游的差异，右端是更改数。点这一行切换到该仓库，点箭头折叠或展开。
///
/// `quiet` 时该仓库没有更改：没有箭头和勾选框，名字用淡色，右端不写更改数。
fn draw_repository_row(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    repo: usize,
    quiet: bool,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let target = CommitTarget::OtherRepository(repo);
    let hovered = app.pointer.hover == Hover::Commit(target);
    if quiet {
        shade_hovered(app, frame, (list, band), target);
    } else {
        draw_header_band(app, frame, (list, band), hovered);
    }
    let cy = band.y + COMMIT_ROW / 2.0;
    let state = (app.git.repositories.get(repo)).map(|root| app.place_state(root));
    let state = state.unwrap_or_default();
    // 名字和分支截到右端的更改数之前，不与它重叠。
    let count = if quiet {
        String::new()
    } else {
        state.changes.to_string()
    };
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count) + 2.0 * END_CENTER;
    let clip = Rect::new(list.x, band.y, band.right() - count_w - list.x, COMMIT_ROW);
    let clip = clip.intersect(list);
    let (arrow, check, icon, name_x) = header_columns(app, band, quiet);
    if !quiet {
        let open = !app.other_folded(repo, None);
        let header = OtherRow::Repository(repo);
        let turn = app.other_fold_turn(header, open) * std::f32::consts::FRAC_PI_2;
        icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, list);
        hits.push((
            fold_area(band).intersect(list),
            CommitTarget::OtherFold(repo),
        ));
        // 勾选式下的勾选框和当前仓库各组标题的勾选框同列：勾上或取消这个仓库的全部文件。
        if check_gap(app) > 0.0 {
            let state = app.other_group_state(repo, None);
            draw_other_checkbox(app, frame, (check, cy), state, list);
            let hit = Rect::new(check - 8.0, band.y + 4.0, 16.0, 16.0);
            hits.push((hit.intersect(list), CommitTarget::OtherGroupCheck(repo)));
        }
    }
    let ink = if quiet { p.text_muted } else { p.text };
    icons::folder_small(frame, (icon + 6.0, cy), p.text_muted, clip);
    // 仓库名用正文字号，比下面小号淡色的组名高一级。
    let name = app.git_repository_name(repo);
    let end = ui_text(frame, app, name_x, band, &name, ink, clip);
    let branch = CommitTarget::OtherBranch(repo);
    draw_branch_sync(app, frame, (band, clip), end, (&state, branch), hits);
    if !quiet {
        draw_count_pill(app, frame, band, &count, list);
    }
    // 箭头、勾选框的点击区域先记，整行的后记：点在它们上面只折叠、勾选，不切换仓库。
    hits.push((band.intersect(list), target));
}

/// 在仓库标题行 `band` 中仓库名之后（名字结束于 `x`）画所在的分支和与上游的差异，只画在
/// `clip` 之内。
///
/// 分支是一个可以点的小块，命中目标为 `target`；有提交可推送或拉取时差异用强调色。
fn draw_branch_sync(
    app: &App,
    frame: &mut Frame<'_>,
    (band, clip): (Rect, Rect),
    mut x: f32,
    (state, target): (&crate::app::git::location_picker::PlaceState, CommitTarget),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let cy = band.y + COMMIT_ROW / 2.0;
    if let Some(head) = &state.head {
        let on = app.pointer.hover == Hover::Commit(target);
        let ink = if on { p.text } else { p.text_dim };
        let left = x + BRANCH_LEAD - 8.0;
        let size = app.fonts.ui_small;
        let end = x + BRANCH_LEAD + BRANCH_TEXT + frame.measure(FontKind::Ui, size, head);
        let chip = Rect::new(left, band.y + 3.0, end + 6.0 - left, band.h - 6.0);
        draw_button(frame, p, chip, clip, on, false);
        x += BRANCH_LEAD;
        icons::branch_small(frame, (x, cy), ink, clip);
        x = ui_text_small(frame, app, x + BRANCH_TEXT, band, head, ink, clip);
        hits.push((chip.intersect(clip), target));
    }
    let sync = match (state.behind, state.ahead) {
        (0, 0) => return,
        (0, ahead) => format!("↑{ahead}"),
        (behind, 0) => format!("↓{behind}"),
        (behind, ahead) => format!("↓{behind} ↑{ahead}"),
    };
    ui_text_small(frame, app, x + 8.0, band, &sync, p.accent, clip);
}

/// 画「没有更改的仓库」一组的标题行：折叠箭头、组名，右端是仓库数。点这一行展开或收起。
fn draw_quiet_group(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let target = CommitTarget::QuietRepositories;
    shade_hovered(app, frame, (list, band), target);
    let (count, any_changed) = app.quiet_repositories();
    let cy = band.y + COMMIT_ROW / 2.0;
    let arrow = band.x + ARROW;
    let open = app.quiet_repositories_open(any_changed);
    let turn = app.other_fold_turn(OtherRow::QuietGroup, open) * std::f32::consts::FRAC_PI_2;
    icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, list);
    let title = t!("git-commit-group-quiet");
    ui_text_small(frame, app, arrow + ICON, band, title, p.text_muted, list);
    draw_count_pill(app, frame, band, &count.to_string(), list);
    hits.push((band.intersect(list), target));
}

/// 在 `at` 画其他仓库各行的勾选框，样式与当前仓库各行的相同。
fn draw_other_checkbox(
    app: &App,
    frame: &mut Frame<'_>,
    at: (f32, f32),
    state: Option<bool>,
    list: Rect,
) {
    let p = app.palette;
    let border = p.text_dim.alpha(0.55);
    icons::checkbox(frame, at, state, p.accent, border, p.text_on_accent, list);
}

/// 仓库名的末尾到分支图标中心的距离（逻辑像素）。
const BRANCH_LEAD: f32 = 14.0;
/// 分支图标的中心到分支名的距离（逻辑像素）：图标宽约 10，与文字相隔 3。
const BRANCH_TEXT: f32 = 8.0;

/// 画另一个仓库里一组的标题行：折叠箭头、勾选框（勾选式下）、组名，右端是文件数；悬停时文件数
/// 左边是按钮（见 [`other_group_actions`]）。与当前仓库改动列表的组标题同列。
fn draw_other_group(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    (repo, group): (usize, OtherGroup),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let staging = app.git.commit.staging();
    let count = |status: &Status| {
        let entries = status.entries.iter();
        entries.filter(|e| group.holds(e, staging)).count()
    };
    let count = app.other_status(repo, |_, status| count(status));
    let row = CommitTarget::OtherGroup(repo, group);
    let actions = other_group_actions(repo, group, staging);
    let hovered = app.pointer.hover == Hover::Commit(row)
        || matches!(app.pointer.hover, Hover::Commit(t) if actions.holds(t));
    if hovered {
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
    }
    let count = count.unwrap_or(0).to_string();
    draw_count_pill(app, frame, band, &count, list);
    if hovered {
        let strip = actions_home(band, actions.len(), count_tail(app, frame, &count));
        draw_row_actions(app, frame, (strip, actions), (list, false), Some(hits));
    }
    let cy = band.y + COMMIT_ROW / 2.0;
    let arrow = band.x + ARROW + INDENT;
    let open = !app.other_folded(repo, Some(group));
    let header = OtherRow::Group(repo, group);
    let turn = app.other_fold_turn(header, open) * std::f32::consts::FRAC_PI_2;
    icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, list);
    let check = arrow + check_gap(app);
    if check_gap(app) > 0.0 {
        let state = app.other_group_state(repo, Some(group));
        draw_other_checkbox(app, frame, (check, cy), state, list);
        let hit = Rect::new(check - 8.0, band.y + 4.0, 16.0, 16.0);
        hits.push((
            hit.intersect(list),
            CommitTarget::OtherGroupFiles(repo, group),
        ));
    }
    ui_text_small(
        frame,
        app,
        check + ICON,
        band,
        group.title(),
        p.text_muted,
        list,
    );
    hits.push((band.intersect(list), row));
}

/// 画另一个仓库中的一个文件：图标、名字，平铺时加淡色的所在目录，右端是状态字母；悬停时状态
/// 字母左边是按钮（见 [`other_file_actions`]），选中时铺选中的底色。
///
/// `side` 是暂存区视图中这一行所在的部分：`Some(true)` 为已暂存，`Some(false)` 为未暂存；勾选式
/// 下为 `None`。
fn draw_other_file(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    (repo, entry, side, depth): (usize, usize, Option<bool>, usize),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let read = |root: &Path, status: &Status| {
        let entry = status.entries.get(entry)?;
        let name = entry.path.file_name()?.to_string_lossy().into_owned();
        let dir = entry.path.parent().and_then(|d| d.strip_prefix(root).ok());
        let dir = dir.map(|d| d.to_string_lossy().into_owned());
        let change = match side {
            Some(true) => entry.staged.unwrap_or(entry.shown()),
            Some(false) => entry.unstaged.unwrap_or(entry.shown()),
            None => entry.shown(),
        };
        let fixed = entry.conflicted();
        let selected = (app.git.commit.other_selected.as_ref())
            .is_some_and(|(r, path)| r == root && *path == entry.path);
        Some((
            name,
            dir.unwrap_or_default(),
            change,
            entry.is_dir,
            fixed,
            selected,
        ))
    };
    let Some((name, dir, change, is_dir, fixed, selected)) = app.other_status(repo, read).flatten()
    else {
        return;
    };
    let target = match side {
        Some(true) => CommitTarget::OtherStagedFile(repo, entry),
        _ => CommitTarget::OtherFile(repo, entry),
    };
    let actions = other_file_actions((repo, entry), side, fixed);
    let hover = app.pointer.hover;
    let hovered =
        hover == Hover::Commit(target) || matches!(hover, Hover::Commit(t) if actions.holds(t));
    let focused = app.focus == Focus::CommitList;
    let shade_color = row_color(p, selected, focused, hovered);
    if let Some(color) = shade_color {
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        frame.rect_clipped(shade, color, RADIUS_ROW, list);
    }
    let strip = hovered.then(|| actions_home(band, actions.len(), 0.0));
    if let Some(strip) = strip {
        draw_row_actions(app, frame, (strip, actions), (list, false), Some(hits));
    }
    let cy = band.y + COMMIT_ROW / 2.0;
    // 缩进一层，与当前仓库平铺时的文件行同列：勾选式下前面是勾选框，暂存区视图中那一列没有。
    let check = band.x + ARROW + (depth + 1) as f32 * INDENT + check_gap(app);
    if check_gap(app) > 0.0 {
        let state = app.other_status(repo, |_, status| {
            (status.entries.get(entry)).and_then(|e| app.other_pick_state(e))
        });
        let state = state.flatten();
        draw_other_checkbox(app, frame, (check, cy), state, list);
        let hit = Rect::new(check - 8.0, band.y + 4.0, 16.0, 16.0);
        hits.push((hit.intersect(list), CommitTarget::OtherCheck(repo, entry)));
    }
    let icon_x = check + ICON;
    let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
    if is_dir {
        icons::folder(frame, (icon_x, (cy - 6.0).round()), false, p.folder, list);
    } else {
        let inset = (icons::FOLDER_WIDTH - icons::FILE_WIDTH) / 2.0;
        draw_file_icon(frame, p, (icon_x + inset, cy), &name, list);
    }
    let letter = change.letter().to_string();
    let letter_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &letter);
    let letter_x = band.right() - END_CENTER - letter_w / 2.0;
    ui_text_small(
        frame,
        app,
        letter_x,
        band,
        &letter,
        git_color(p, change),
        list,
    );
    let text_end = strip.map_or(letter_x - 8.0, |s| s.first.x - 4.0);
    let clip = Rect::new(list.x, band.y, text_end - list.x, COMMIT_ROW).intersect(list);
    let room = (text_end - text_x).max(0.0);
    let dir = (app.git.commit.flat() && !dir.is_empty()).then_some(dir.as_str());
    let parts = file_parts(app, frame, (&name, dir), Some(room));
    let end = draw_file_parts(
        app,
        frame,
        (text_x, band),
        &parts,
        (file_color(change, p), |_| 1.0),
        clip,
    );
    // 和当前仓库使用相同的名称排版与截断渐隐，底色随这一行的悬停状态变化。
    if end > text_end + 0.5 {
        let solid = match shade_color {
            Some(color) => crate::view::workspace::panel::over(p.sidebar_bg, color),
            None => p.sidebar_bg,
        };
        let reach = NAME_FADE.min(room);
        let edge = Rect::new(text_end - reach, band.y, reach, COMMIT_ROW);
        frame.within(clip, |frame| fade(frame, edge, solid, false));
    }
    hits.push((band.intersect(list), target));
}
