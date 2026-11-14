//! 多仓库时改动列表中各仓库的标题行，以及其他仓库的分组行、文件行。

use super::*;
use crate::app::{OtherGroup, OtherRow};

/// 绘制多仓库时当前仓库改动列表以外的各行，记下可点击的位置。
///
/// 各行按仓库列表的顺序排，随列表一起滚动。当前仓库标题行之后的各行让出当前仓库改动列表的高度
/// `block`（逻辑像素）。仓库标题在最外一层，组、文件缩进一层，与当前仓库改动列表的组、平铺
/// 的文件同列。
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
            match *row {
                OtherRow::Current => draw_current_row(app, frame, (list, band), hits),
                OtherRow::Repository(repo) => {
                    draw_repository_row(app, frame, (list, band), repo, false, hits);
                }
                OtherRow::Quiet(repo) => {
                    draw_repository_row(app, frame, (list, band), repo, true, hits)
                }
                OtherRow::Group(repo, group) => {
                    draw_other_group(app, frame, (list, band), (repo, group), hits);
                }
                OtherRow::Staged(repo, entry) => {
                    draw_other_file(app, frame, (list, band), (repo, entry, Some(true)), hits);
                }
                OtherRow::File(repo, entry) => {
                    let side = app.git.commit.staging().then_some(false);
                    draw_other_file(app, frame, (list, band), (repo, entry, side), hits);
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

    /// 最后一行之下（含当前仓库排在最后时它的改动列表）的位置。
    fn bottom(&self) -> f32 {
        let last = self.rows.len().saturating_sub(1);
        let mut y = self.top(last) + COMMIT_ROW;
        if self.rows.get(last) == Some(&OtherRow::Current) {
            y += self.block;
        }
        if let Some(f) = self.fold
            && last == f.parent + f.count
        {
            y -= f.hidden;
        }
        y
    }
}

/// 仓库标题行各部分的横坐标：折叠箭头的中心、勾选框的中心（勾选式下才有）、图标的左边、名字
/// 的左边。与当前仓库改动列表最外一层（组标题）的箭头、勾选框同列。
fn header_columns(app: &App, band: Rect) -> (f32, f32, f32, f32) {
    let arrow = band.x + ARROW;
    let check = arrow + check_gap(app);
    let icon = check + ICON;
    (arrow, check, icon, icon + 6.0 + ICON)
}

/// 画当前仓库的标题行：折叠箭头、勾选框（勾选式下，勾上或取消它的全部文件）、✓、仓库名，
/// 右端是更改数。当前仓库的分支写在上面的分支行里，这里不再写。点这一行收起或展开整个一节。
///
/// 没有更改时和其他没有更改的仓库一样：没有箭头和勾选框，名字用淡色，右端写「无更改」。
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
        Hover::Commit(CommitTarget::CurrentRepository | CommitTarget::CurrentCheck)
    );
    if !quiet {
        draw_header_band(app, frame, (list, band), hovered);
    }
    let cy = band.y + COMMIT_ROW / 2.0;
    let (arrow, check, icon, name_x) = header_columns(app, band);
    if !quiet {
        let open = !app.current_section_folded();
        let turn = app.other_fold_turn(OtherRow::Current, open) * std::f32::consts::FRAC_PI_2;
        icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, list);
        if check_gap(app) > 0.0 {
            draw_other_checkbox(app, frame, (check, cy), app.current_check_state(), list);
            let hit = Rect::new(check - 8.0, band.y + 4.0, 16.0, 16.0);
            hits.push((hit.intersect(list), CommitTarget::CurrentCheck));
        }
    }
    // 当前仓库用 ✓ 标出，与分支、工作树选择器里当前的那一项相同。
    icons::check(frame, (icon + 6.0, cy), 10.0, p.accent, list);
    let count = match quiet {
        true => t!("git-commit-repository-clean").to_owned(),
        false => total.to_string(),
    };
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count) + 2.0 * END_CENTER;
    let clip = Rect::new(list.x, band.y, band.right() - count_w - list.x, COMMIT_ROW);
    // 仓库名用正文字号，比下面小号淡色的组名高一级。
    let name = app.git_current_repository_name();
    let ink = if quiet { p.text_muted } else { p.text };
    ui_text(frame, app, name_x, band, &name, ink, clip.intersect(list));
    draw_count_pill(app, frame, band, &count, list);
    if !quiet {
        hits.push((band.intersect(list), target));
    }
}

/// 相邻两个仓库一节之间的空隙（逻辑像素）：每节上下各让出一半，行的位置不变。
const SECTION_GAP: f32 = 4.0;

/// 先于各行铺好多仓库时每个仓库一节的底：一张圆角的卡片，按序号依次取 [`SECTION_HUES`] 里
/// 的项目徽章颜色，铺得很淡；相邻两节之间隔 [`SECTION_GAP`]。标题行下面的各行在仓库箭头正
/// 下方有一条同色的竖线。行的悬停、选中底色画在它上面。
///
/// 一节从仓库的标题行起，到下一个仓库的标题行之前为止；当前仓库那一节还包括它的改动列表，
/// 高 `block`（逻辑像素）。只用来分开各个仓库，不表示状态。
pub(super) fn draw_sections(app: &App, frame: &mut Frame<'_>, list: Rect, block: f32) {
    let layout = OtherLayout::new(app, list, block);
    if layout.rows.is_empty() {
        return;
    }
    let mut start: Option<(f32, usize)> = None;
    let mut sections = Vec::new();
    for (i, row) in layout.rows.iter().enumerate() {
        let color = match row {
            OtherRow::Current => Some(app.repository_color(None)),
            OtherRow::Repository(r) | OtherRow::Quiet(r) => Some(app.repository_color(Some(*r))),
            _ => None,
        };
        if let Some(color) = color {
            let top = layout.top(i);
            if let Some((y, c)) = start.take() {
                sections.push((y, top, c));
            }
            start = Some((top, color));
        }
    }
    if let Some((y, c)) = start {
        sections.push((y, layout.bottom(), c));
    }
    let (x, w) = (list.x + 8.0, list.w - 16.0);
    for (y0, y1, color) in sections {
        if y1 < list.y || y0 > list.bottom() {
            continue;
        }
        let card = Rect::new(x, y0 + SECTION_GAP / 2.0, w, y1 - y0 - SECTION_GAP);
        frame.rect_clipped(card, section_fill(app, color), RADIUS_ROW, list);
        // 标题行下面的组、文件：仓库箭头正下方一条仓库色的竖线，到卡片底边前收住。
        let (from, to) = (y0 + COMMIT_ROW, card.bottom() - RADIUS_ROW);
        if to > from {
            let base = app.palette.badges[SECTION_HUES[color % SECTION_HUES.len()]].0;
            let guide = Rect::new((x + ARROW).round(), from, 1.0, to - from);
            frame.rect_clipped(guide, base.alpha(0.45), 0.0, list);
        }
    }
}

/// 第 `color` 种仓库色的卡片底色。
pub(super) fn section_fill(app: &App, color: usize) -> Color {
    let base = app.palette.badges[SECTION_HUES[color % SECTION_HUES.len()]].0;
    base.alpha(0.08)
}

/// 仓库一节可用的项目徽章颜色（徽章配色的下标）：青、紫、品红、靛蓝、绿、石板灰、蓝。不用红、
/// 橙、黄：它们在 Git 视图里是删除、冲突、警告这类状态的颜色；蓝排在最后，和选中行的强调色
/// 拉开。
const SECTION_HUES: [usize; 7] = [4, 7, 8, 6, 3, 9, 5];

/// 有更改的仓库的标题行悬停时铺悬停的底色（仓库色的底在 [`draw_sections`] 里铺好了）。
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

/// 画另一个仓库的标题行：折叠箭头、勾选框（勾选式下）、文件夹图标、仓库名、所在的分支和与
/// 上游的差异，右端是更改数。点这一行切换到该仓库，点箭头折叠或展开。
///
/// `quiet` 时这个仓库没有更改：没有箭头和勾选框，名字用淡色，右端写「无更改」。
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
    let count = match quiet {
        true => t!("git-commit-repository-clean").to_owned(),
        false => state.changes.to_string(),
    };
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count) + 2.0 * END_CENTER;
    let clip = Rect::new(list.x, band.y, band.right() - count_w - list.x, COMMIT_ROW);
    let clip = clip.intersect(list);
    let (arrow, check, icon, name_x) = header_columns(app, band);
    if !quiet {
        let open = !app.other_folded(repo, None);
        let header = OtherRow::Repository(repo);
        let turn = app.other_fold_turn(header, open) * std::f32::consts::FRAC_PI_2;
        icons::chevron_turned(frame, (arrow, cy), 6.0, turn, p.text_dim, list);
        let fold = Rect::new(arrow - 8.0, band.y + 4.0, 16.0, 16.0);
        hits.push((fold.intersect(list), CommitTarget::OtherFold(repo)));
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
    let mut x = ui_text(frame, app, name_x, band, &name, ink, clip);
    if let Some(head) = &state.head {
        // 分支是一个可以点的小块：点了切换到这个仓库并展开它的分支选择面板。
        let branch = CommitTarget::OtherBranch(repo);
        let on = app.pointer.hover == Hover::Commit(branch);
        let ink = if on { p.text } else { p.text_dim };
        let left = x + BRANCH_LEAD - 8.0;
        let size = app.fonts.ui_small;
        let end = x + BRANCH_LEAD + BRANCH_TEXT + frame.measure(FontKind::Ui, size, head);
        let chip = Rect::new(left, band.y + 3.0, end + 6.0 - left, band.h - 6.0);
        draw_button(frame, p, chip, clip, on, false);
        x += BRANCH_LEAD;
        icons::branch_small(frame, (x, cy), ink, clip);
        x = ui_text_small(frame, app, x + BRANCH_TEXT, band, head, ink, clip);
        hits.push((chip.intersect(clip), branch));
    }
    // 与上游的差异跟在分支名后面：有提交可推送或拉取时用强调色。
    let sync = match (state.behind, state.ahead) {
        (0, 0) => String::new(),
        (0, ahead) => format!("↑{ahead}"),
        (behind, 0) => format!("↓{behind}"),
        (behind, ahead) => format!("↓{behind} ↑{ahead}"),
    };
    if !sync.is_empty() {
        ui_text_small(frame, app, x + 8.0, band, &sync, p.accent, clip);
    }
    draw_count_pill(app, frame, band, &count, list);
    // 箭头、勾选框的点击区域先记，整行的后记：点在它们上面只折叠、勾选，不切换仓库。
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

/// 画行尾的暂存（`on` 为真，加号）或取消暂存（减号）按钮，位置与当前仓库各行悬停时的按钮相同；
/// 返回它的左边缘。
fn draw_stage_button(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    target: CommitTarget,
    on: bool,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> f32 {
    let p = app.palette;
    let rect = file_actions(band).1;
    let hovered = app.pointer.hover == Hover::Commit(target);
    draw_button(frame, p, rect, list, hovered, false);
    let color = if hovered { p.text } else { p.text_muted };
    match on {
        true => icons::plus(frame, center(rect), 8.0, color, list),
        false => icons::minus(frame, center(rect), 8.0, color, list),
    }
    hits.push((rect.intersect(list), target));
    rect.x
}

/// 画另一个仓库里一组的标题行：折叠箭头、勾选框（勾选式下）、组名，右端是文件数；暂存区视图
/// 中悬停时文件数换成暂存或取消暂存整组的按钮。与当前仓库改动列表的组标题同列。
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
    let staged = group == OtherGroup::Staged;
    let stage = CommitTarget::OtherGroupStage(repo, !staged);
    let hover = app.pointer.hover;
    let hovered = hover == Hover::Commit(row) || hover == Hover::Commit(stage);
    shade_hovered(app, frame, (list, band), row);
    if staging && hovered {
        draw_stage_button(app, frame, (list, band), stage, !staged, hits);
    } else {
        draw_count_pill(app, frame, band, &count.unwrap_or(0).to_string(), list);
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

/// 画另一个仓库中的一个文件：图标、名字、淡色的所在目录，右端是状态字母。
///
/// `side` 是暂存区视图中这一行所在的部分：`Some(true)` 为已暂存，`Some(false)` 为未暂存；
/// 此时行尾是取消暂存或暂存的按钮，状态字母在按钮左边。勾选式下为 `None`。
fn draw_other_file(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    (repo, entry, side): (usize, usize, Option<bool>),
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
        Some((name, dir.unwrap_or_default(), change, entry.is_dir, fixed))
    };
    let Some((name, dir, change, is_dir, fixed)) = app.other_status(repo, read).flatten() else {
        return;
    };
    let target = match side {
        Some(true) => CommitTarget::OtherStagedFile(repo, entry),
        _ => CommitTarget::OtherFile(repo, entry),
    };
    shade_hovered(app, frame, (list, band), target);
    // 悬停时右端的状态字母换成暂存或取消暂存的按钮。冲突中的文件在这里不能暂存：切换到该
    // 仓库解决。
    let stage = side
        .filter(|_| !fixed)
        .map(|staged| (CommitTarget::OtherStage(repo, entry, !staged), !staged));
    let hover = app.pointer.hover;
    let stage =
        stage.filter(|(stage, _)| hover == Hover::Commit(target) || hover == Hover::Commit(*stage));
    let button = stage.map(|(stage, on)| {
        // 悬停在按钮上时整行同样铺底色。
        if hover == Hover::Commit(stage) {
            let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
            frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
        }
        draw_stage_button(app, frame, (list, band), stage, on, hits)
    });
    let cy = band.y + COMMIT_ROW / 2.0;
    // 缩进一层，与当前仓库平铺时的文件行同列：勾选式下前面是勾选框，暂存区视图中那一列没有。
    let check = band.x + ARROW + INDENT + check_gap(app);
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
    if button.is_none() {
        let color = git_color(p, change);
        ui_text_small(frame, app, letter_x, band, &letter, color, list);
    }
    let text_end = button.map_or(letter_x - 8.0, |x| x - 4.0);
    let clip = Rect::new(list.x, band.y, text_end - list.x, COMMIT_ROW).intersect(list);
    let end = ui_text(frame, app, text_x, band, &name, file_color(change, p), clip);
    if !dir.is_empty() {
        ui_text_small(frame, app, end + 8.0, band, &dir, p.text_dim, clip);
    }
    hits.push((band.intersect(list), target));
}
