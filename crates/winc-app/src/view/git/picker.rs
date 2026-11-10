//! 分支、位置选择器：从 Git 上下文或状态栏展开的菜单式浮层。
//!
//! 卡片、行高和选中行的强调色实底都与窗口内绘制的菜单相同；顶部多一个内嵌的搜索框（分支选择器在
//! 它下面还有一排拉取、推送、获取），底部多一栏新建的入口。选中项跟着鼠标走，同一时刻只有一行
//! 高亮，行尾的「⋯」也只在这一行。
//!
//! 卡片里的竖线从上到下是同几条：搜索框、选中行的底色、分隔线和底栏的按钮两边对着 `GUTTER`，它们
//! 里面的内容对着 `CONTENT`；放大镜、勾、箭头和底栏的加号是一列，搜索框的字、分组标题、各行的名字
//! 和底栏第一个入口的字是一列。

use std::f32::consts::FRAC_PI_2;
use std::ops::Range;

use winc_i18n::t;

use super::super::popup_menu::band_ink;
use super::commit::draw_count_pill_in;
use crate::app::git::branch::local_name;
use crate::app::git::branch::picker::{BranchRow, BranchSection, GIT_PICKER_ACTIONS_TOP};
use crate::app::git::location_picker::{LocationRow, Place};
use crate::app::git::log::display_local;
use crate::app::{Focus, PICKER_ROW, PickerPart};
use crate::layout::PickerParts;
use crate::view::*;

/// 搜索框、选中行的底色、分隔线和底栏的按钮离卡片边的距离（逻辑像素）。
const GUTTER: f32 = 6.0;
/// 内容离卡片左右边的距离（逻辑像素）：`GUTTER` 里面再留 8。
const CONTENT: f32 = 14.0;
/// 图标一列的中线离卡片左边的距离（逻辑像素）：12 宽的图标从 `CONTENT` 开始。
const ICON_CENTER: f32 = 20.0;
/// 文字一列离卡片左边的距离（逻辑像素）：12 宽的图标右边再隔 6。
const TEXT_LEFT: f32 = 32.0;
/// 行尾按钮右边和选中行底色右边的距离（逻辑像素）：按钮的中线和图标一列左右对称。
const ROW_BUTTON_GAP: f32 = 2.0;
/// 名字右边到右端文字的最小间距（逻辑像素）。
const NAME_GAP: f32 = 8.0;
/// 行右端文字（时间、上游）的右边比「⋯」按钮的右边缘让进的距离（逻辑像素）：和按钮里图标的
/// 右侧差不多齐，不贴着底色的圆角。
const META_PAD: f32 = 6.0;
/// 名字后面的小圆点占的宽度（逻辑像素）：6 的间距加 5 的直径。
const NAME_DOT_ROOM: f32 = 11.0;
/// 选中行上徽标底色和按钮悬停底色的不透明度：比次要文字弱得多，只从实底上浮出一层。
const ON_BAND: f32 = 0.2;

/// 绘制选择器：卡片、搜索框、列表和底栏，并记下列表以外可以点的地方。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let rect = app.layout.picker;
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let parts = picker_parts(app, rect);
    draw_card(frame, app.palette, rect, RADIUS_POPUP, app.palette.popup_bg);
    draw_search(app, frame, parts.bar);

    let mut hits = Vec::new();
    draw_actions(app, frame, &parts, &mut hits);
    if app.picker_lists_locations() {
        draw_location_rows(app, frame, &parts, &mut hits);
    } else {
        draw_branch_rows(app, frame, &parts, &mut hits);
    }
    draw_footer(app, frame, &parts, &mut hits);
    app.hits.picker_parts = hits;
}

/// 绘制内嵌的搜索框：左边是放大镜。
fn draw_search(app: &mut App, frame: &mut Frame<'_>, bar: Rect) {
    let p = app.palette;
    let field = Rect::new(
        bar.x + GUTTER,
        bar.y + GUTTER,
        bar.w - 2.0 * GUTTER,
        BUTTON_SMALL,
    );
    let focused = app.focus == Focus::Picker;
    field_box(app, frame, field, p.popup_bg, focused, false);
    let icon = (bar.x + ICON_CENTER, field.y + field.h / 2.0);
    icons::magnifier(frame, icon, p.text_dim, field);
    let placeholder = if app.picker_lists_locations() {
        t!("git-location-picker-placeholder")
    } else {
        t!("git-branch-picker-placeholder")
    };
    let x = bar.x + TEXT_LEFT;
    let area = Rect::new(x, field.y, (bar.right() - CONTENT - x).max(0.0), field.h);
    let mut input = std::mem::take(&mut app.picker.input);
    let caret = draw_bar_field(app, frame, area, &mut input, placeholder, focused);
    app.picker.input = input;
    if caret.is_some() {
        app.ime.cursor = caret;
    }
}

/// 绘制搜索框下面的拉取、推送、获取：三个一样宽的按钮，和上游差的提交数写在名字后面（「拉取
/// ↓2」「推送 ↑1」）；按住 ⌥ 时拉取写成拉取并变基。不能用的灰掉，原因写在悬停提示里。
fn draw_actions(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
    hits: &mut Vec<(Rect, PickerPart)>,
) {
    let area = parts.actions;
    if area.h <= 0.0 {
        return;
    }
    let p = app.palette;
    let (ahead, behind) = (app.git.status.as_ref()).map_or((0, 0), |s| (s.ahead, s.behind));
    let size = app.fonts.ui_small;
    let inner = area.w - 2.0 * GUTTER;
    // 按钮之间的间距与标签之间的相同（`TAB_GAP`）。
    let w = ((inner - 2.0 * TAB_GAP) / 3.0).floor();
    let actions = [PickerPart::Pull, PickerPart::Push, PickerPart::Fetch];
    for (i, part) in actions.into_iter().enumerate() {
        let x = area.x + GUTTER + i as f32 * (w + TAB_GAP);
        let rect = Rect::new(x, area.y + GIT_PICKER_ACTIONS_TOP, w, BUTTON_SMALL);
        let enabled = app.picker_action_enabled(part);
        let hovered = enabled && app.hover == Hover::PickerPart(part);
        let fill = if hovered {
            p.selected_inactive
        } else {
            p.hover
        };
        frame.rounded(rect, fill, RADIUS_ROW);
        let name = app.picker_action_command(part).label();
        let text = match part {
            PickerPart::Pull if behind > 0 => format!("{name} ↓{behind}"),
            PickerPart::Push if ahead > 0 => format!("{name} ↑{ahead}"),
            _ => name.to_owned(),
        };
        let text = fit(frame, &text, size, (w - 8.0).max(0.0), false);
        let text_w = frame.measure(FontKind::Ui, size, &text);
        let text_x = (rect.x + (rect.w - text_w) / 2.0).round();
        let color = if enabled { p.text } else { p.text_dim };
        ui_text_small(frame, app, text_x, rect, &text, color, area);
        // 不能用时也记下：悬停提示写明原因，点了不做事。
        hits.push((rect, part));
    }
}

/// 这一帧要画的行：展开、收起的动画期间下面的行往上收，视口里能多露出几行。
fn drawn_rows(app: &App, parts: &PickerParts, count: usize) -> Range<usize> {
    let scroll = app.picker_drawn_scroll();
    let hidden = app.picker.fold.as_ref().map_or(0.0, |fold| fold.hidden());
    let first = ((scroll / PICKER_ROW).floor().max(0.0) as usize).min(count);
    let last = (((scroll + parts.list.h + hidden) / PICKER_ROW).ceil() as usize).min(count);
    first..last.max(first)
}

/// 第 `i` 行这一帧的矩形和裁剪范围：展开、收起的动画期间父行下面的行往上收，子行只露出父行下沿
/// 以下的部分。
fn placed_row(app: &App, parts: &PickerParts, i: usize) -> (Rect, Rect) {
    let scroll = app.picker_drawn_scroll();
    let mut row = parts.row(i, scroll);
    let mut clip = parts.list;
    if let Some(fold) = app.picker.fold.as_ref().filter(|f| i > f.before.parent) {
        row.y -= fold.hidden();
        if i <= fold.before.parent + fold.count {
            let top = parts.row(fold.before.parent, scroll).bottom();
            let room = (clip.bottom() - top).max(0.0);
            clip = clip.intersect(Rect::new(clip.x, top, clip.w, room));
        }
    }
    (row, row.intersect(clip))
}

/// 正在绘制的一行。
struct Line {
    row: Rect,
    clip: Rect,
    /// 键盘选中的行：强调色底，回车作用在它上面。分支选择器打开时自动选中的行不算，见
    /// [`Line::begin`]。
    selected: bool,
    /// 分支选择器打开时自动选中的行：浅色底，回车作用在它上面。
    default: bool,
    /// 鼠标所在的行：浅色底。与选中的行相互独立，鼠标移动不改变选中项。
    hovered: bool,
    /// 名字的颜色。
    text: Color,
    /// 次要文字和箭头的颜色。
    dim: Color,
    /// 图标的竖直中线。
    cy: f32,
}

impl Line {
    /// 开始绘制第 `i` 行：选中时先铺强调色底，文字改用底色上的颜色；鼠标所在的行铺浅色底。
    ///
    /// 分支选择器打开时自动选中的行（`Picker::default`）铺比悬停稍深的浅色底，文字颜色不变：
    /// 回车仍作用在它上面，但不与行首对勾标出的当前分支混淆。移动选中项或输入查询后改为强调色底。
    fn begin(app: &App, frame: &mut Frame<'_>, parts: &PickerParts, i: usize) -> Line {
        let p = app.palette;
        let (row, clip) = placed_row(app, parts, i);
        let chosen = (app.picker.fold.as_ref()).map_or(app.picker.selected, |fold| {
            fold.drawn_index(app.picker.selected)
        }) == i;
        let default = chosen && app.picker.default == Some(app.picker.selected);
        let selected = chosen && !default;
        // 展开、收起的动画期间各行的位置在变化，悬停记录的行号与绘制位置不对应，因此不画悬停。
        let hovered = app.picker.fold.is_none()
            && matches!(app.hover,
                Hover::PickerRow(r) | Hover::PickerPart(PickerPart::More(r)) if r == i);
        let band = row.inset(GUTTER, 1.0);
        let (text, dim) = if selected {
            frame.rect_clipped(band, p.accent_fill, RADIUS_ROW, clip);
            band_ink(p)
        } else {
            if default {
                frame.rect_clipped(band, p.selected_inactive, RADIUS_ROW, clip);
            } else if hovered {
                frame.rect_clipped(band, p.hover, RADIUS_ROW, clip);
            }
            (p.text, p.text_dim)
        };
        let cy = icon_center_y(app, row, app.fonts.ui);
        Line {
            row,
            clip,
            selected,
            default,
            hovered,
            text,
            dim,
            cy,
        }
    }

    /// 图标一列的中线和文字一列的左边，每深一层缩进 `TREE_INDENT`。
    fn columns(&self, depth: usize) -> (f32, f32) {
        let indent = depth as f32 * crate::theme::TREE_INDENT;
        (
            self.row.x + ICON_CENTER + indent,
            self.row.x + TEXT_LEFT + indent,
        )
    }

    /// 行右端那一列的右边：选中行「⋯」按钮的右边缘，数量徽标也对齐到这里。
    fn end(&self) -> f32 {
        self.row.right() - GUTTER - ROW_BUTTON_GAP
    }

    /// 行右端文字（时间、上游）的右边：比按钮的右边缘让进 [`META_PAD`]，和按钮里的图标差不多齐。
    fn right(&self) -> f32 {
        self.end() - META_PAD
    }

    /// 在图标一列画第 `i` 行展开、收起的箭头：这一行正在展开或收起时跟着动画转到对应角度。
    fn draw_fold(&self, app: &App, frame: &mut Frame<'_>, (i, depth, open): (usize, usize, bool)) {
        let turned = match &app.picker.fold {
            Some(fold) if fold.before.parent == i => fold.turned(),
            _ => f32::from(u8::from(open)),
        };
        let at = (self.columns(depth).0, self.cy);
        icons::chevron_turned(frame, at, 6.0, turned * FRAC_PI_2, self.dim, self.clip);
    }

    /// 从 `x` 起绘制名字，到 `right` 为止，放不下时截短。查询命中的字用强调色标出；选中行的底色
    /// 就是强调色，截短了的名字下标对不上，都不标。
    fn draw_name(
        &self,
        app: &App,
        frame: &mut Frame<'_>,
        (x, right): (f32, f32),
        name: &str,
        color: Color,
    ) {
        let size = app.fonts.ui;
        let shown = ellipsize(frame, name, size, (right - x).max(0.0));
        let marked = if self.selected || shown.len() != name.len() {
            Vec::new()
        } else {
            app.git_picker_matched(name)
        };
        let at = (x, self.row);
        marked_label(frame, app, size, at, &shown, &marked, color, self.clip);
    }

    /// 同 [`Line::draw_name`]，`dotted` 时名字后面有一个小圆点，表示有未提交的更改。
    fn draw_dotted_name(
        &self,
        app: &App,
        frame: &mut Frame<'_>,
        (x, right): (f32, f32),
        name: &str,
        color: Color,
        dotted: bool,
    ) {
        if !dotted {
            self.draw_name(app, frame, (x, right), name, color);
            return;
        }
        let room = (right - NAME_DOT_ROOM).max(x);
        self.draw_name(app, frame, (x, room), name, color);
        let width = frame
            .measure(FontKind::Ui, app.fonts.ui, name)
            .min(room - x);
        let ink = if self.selected {
            self.text
        } else {
            app.palette.accent
        };
        let dot = Rect::new(x + width + 6.0, self.cy - 2.5, 5.0, 5.0);
        frame.rect_clipped(dot, ink, 2.5, self.clip);
    }

    /// 在 `right` 左边绘制次要文字，把 `right` 移到文字左边。
    fn draw_meta(&self, app: &App, frame: &mut Frame<'_>, right: &mut f32, text: &str) {
        let size = app.fonts.ui_small;
        *right -= frame.measure(FontKind::Ui, size, text);
        ui_text_small(frame, app, *right, self.row, text, self.dim, self.clip);
    }

    /// 在行右端绘制数量徽标，把 `right` 移到徽标左边。
    fn draw_pill(&self, app: &App, frame: &mut Frame<'_>, right: &mut f32, count: usize) {
        let p = app.palette;
        let colors = if self.selected {
            (p.text_on_accent.alpha(ON_BAND), p.text_on_accent)
        } else {
            (p.hover, p.text_muted)
        };
        let text = count.to_string();
        let row = self.row;
        // 徽标右边和「⋯」按钮的右边缘对齐（`draw_count_pill_in` 留 4）。
        let band = Rect::new(row.x, row.y, self.end() + 4.0 - row.x, row.h);
        draw_count_pill_in(app, frame, band, &text, colors, self.clip);
        *right = self.end() - frame.measure(FontKind::Ui, app.fonts.ui_small, &text) - 12.0 - 6.0;
    }

    /// 绘制第 `i` 行行尾的「⋯」，记下它的位置，返回按钮左边可以写名字的右边；选中的行（含自动
    /// 选中的行）和鼠标所在的行才有，其余行返回 `None`。按钮盖在行右端文字的位置上，这几行不写
    /// 那段文字。
    fn draw_more(
        &self,
        app: &App,
        frame: &mut Frame<'_>,
        i: usize,
        hits: &mut Vec<(Rect, PickerPart)>,
    ) -> Option<f32> {
        if !self.selected && !self.default && !self.hovered {
            return None;
        }
        let part = PickerPart::More(i);
        let row = self.row;
        let button = Rect::new(
            self.end() - BUTTON_SMALL,
            row.y + 2.0,
            BUTTON_SMALL,
            row.h - 4.0,
        );
        self.draw_row_button(app, frame, button, part);
        icons::more(frame, center(button), self.text, self.clip);
        hits.push((button, part));
        Some(button.x - ROW_BUTTON_GAP)
    }

    /// 绘制行内按钮悬停时的底色。
    fn draw_row_button(&self, app: &App, frame: &mut Frame<'_>, button: Rect, part: PickerPart) {
        let p = app.palette;
        let hovered = app.hover == Hover::PickerPart(part);
        if hovered && self.selected {
            let fill = p.text_on_accent.alpha(ON_BAND);
            frame.rect_clipped(button, fill, RADIUS_ROW, self.clip);
        } else {
            draw_button(frame, p, button, self.clip, hovered, false);
        }
    }
}

/// 列表为空时在第一行写明。
fn draw_empty(app: &App, frame: &mut Frame<'_>, list: Rect, text: &str) {
    let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
    let x = list.x + TEXT_LEFT;
    ui_text(frame, app, x, row, text, app.palette.text_dim, list);
}

/// 绘制位置选择器的各行：可以展开的仓库和各个位置。
fn draw_location_rows(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
    hits: &mut Vec<(Rect, PickerPart)>,
) {
    let rows = match &app.picker.fold {
        Some(fold) if !fold.expanding => fold.before.locations.as_slice(),
        _ => app.picker.location_rows.as_deref().unwrap_or_default(),
    };
    if rows.is_empty() {
        draw_empty(app, frame, parts.list, t!("git-location-picker-empty"));
        return;
    }
    for i in drawn_rows(app, parts, rows.len()) {
        let line = Line::begin(app, frame, parts, i);
        match &rows[i] {
            LocationRow::Repository {
                label,
                count,
                changes,
                open,
                ..
            } => {
                line.draw_fold(app, frame, (i, 0, *open));
                let mut right = line.right();
                let note = t!("git-location-count", count = *count);
                line.draw_meta(app, frame, &mut right, &note);
                let at = (line.columns(0).1, right - NAME_GAP);
                // 展开时各行有自己的圆点。
                let dotted = !*open && *changes > 0;
                line.draw_dotted_name(app, frame, at, label, line.text, dotted);
            }
            LocationRow::Place(place) => draw_place_row(app, frame, &line, i, place, hits),
        }
    }
}

/// 绘制第 `i` 行的一个位置，和分支行的结构相同：名字、右端一段淡色文字，操作都在选中行的「⋯」里。
///
/// 当前位置打勾，右端写分支和与上游的差异，目录已不在的写明；有未提交的更改时名字后面有小圆点。
fn draw_place_row(
    app: &App,
    frame: &mut Frame<'_>,
    line: &Line,
    i: usize,
    place: &Place,
    hits: &mut Vec<(Rect, PickerPart)>,
) {
    let (check, text) = line.columns(place.depth);
    if place.current {
        icons::check(frame, (check, line.cy), 9.0, line.text, line.clip);
    }
    let mut right = line.right();
    let more = (place.worktree.is_some() || !place.missing)
        .then(|| line.draw_more(app, frame, i, hits))
        .flatten();
    let state = app.place_state(&place.path);
    if let Some(at) = more {
        right = at + NAME_GAP;
    } else if place.missing {
        line.draw_meta(app, frame, &mut right, t!("git-worktree-missing"));
    } else {
        let mut meta = state.head.clone().unwrap_or_default();
        if state.behind > 0 {
            meta.push_str(&format!(" ↓{}", state.behind));
        }
        if state.ahead > 0 {
            meta.push_str(&format!(" ↑{}", state.ahead));
        }
        let shown = ellipsize(frame, &meta, app.fonts.ui_small, line.row.w * 0.45);
        line.draw_meta(app, frame, &mut right, &shown);
    }
    let color = if place.missing { line.dim } else { line.text };
    let dotted = !place.missing && state.changes > 0;
    line.draw_dotted_name(
        app,
        frame,
        (text, right - NAME_GAP),
        &place.label,
        color,
        dotted,
    );
}

/// 绘制分支选择器的各行：分组标题、按 `/` 分层的目录和分支，以及「新建分支」。
fn draw_branch_rows(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
    hits: &mut Vec<(Rect, PickerPart)>,
) {
    let rows = match &app.picker.fold {
        Some(fold) if !fold.expanding => fold.before.branches.as_slice(),
        _ => app.picker.branch_rows.as_deref().unwrap_or_default(),
    };
    if rows.is_empty() {
        draw_empty(app, frame, parts.list, t!("git-branch-picker-empty"));
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    for i in drawn_rows(app, parts, rows.len()) {
        if let BranchRow::Heading(section) = &rows[i] {
            draw_heading(app, frame, parts, i, *section);
            continue;
        }
        // 展开的远程分支一组和其他分组一样，上沿有分隔线；选中时底色盖住它。收起时只有这一行，
        // 下面紧接着底栏的分隔线，再画一条就太密了。
        if matches!(rows[i], BranchRow::Remotes { open: true, .. }) && i > 0 {
            let (row, clip) = placed_row(app, parts, i);
            draw_divider(frame, app.palette, row, row.y + 1.0, clip);
        }
        let line = Line::begin(app, frame, parts, i);
        match &rows[i] {
            BranchRow::Heading(_) => {}
            // 和提交视图的组标题一样：小一号的次要色字，右端一个写数量的小圆角徽标。
            BranchRow::Remotes { count, open } => {
                line.draw_fold(app, frame, (i, 0, *open));
                let title = t!("menu-git-remote-branches");
                let x = line.columns(0).1;
                ui_text_small(frame, app, x, line.row, title, line.dim, line.clip);
                let mut right = line.right();
                line.draw_pill(app, frame, &mut right, *count);
            }
            // 目录不写分支数：展开就看得见，每层写一遍是重复信息。
            BranchRow::Folder {
                label, depth, open, ..
            } => {
                line.draw_fold(app, frame, (i, *depth, *open));
                let at = (line.columns(*depth).1, line.right());
                line.draw_name(app, frame, at, label, line.text);
            }
            BranchRow::Branch {
                index,
                depth,
                label,
            } => {
                let row = BranchLine {
                    index: *index,
                    depth: *depth,
                    label,
                    now,
                };
                draw_branch_row(app, frame, &line, i, &row, hits);
            }
            BranchRow::Create(name) => {
                let (icon, text) = line.columns(0);
                icons::plus(frame, (icon, line.cy), 8.0, line.dim, line.clip);
                let label = t!("git-branch-picker-create-row", name = name.as_str());
                let shown = ellipsize(frame, &label, app.fonts.ui, line.right() - text);
                ui_text(frame, app, text, line.row, &shown, line.text, line.clip);
            }
        }
    }
}

/// 在纵坐标 `y` 画一条分隔线，两端对着 `GUTTER`。
fn draw_divider(frame: &mut Frame<'_>, p: &Palette, rect: Rect, y: f32, clip: Rect) {
    let line = Rect::new(rect.x + GUTTER, y, rect.w - 2.0 * GUTTER, 1.0);
    frame.rect_clipped(line, p.separator, 0.0, clip);
}

/// 绘制第 `i` 行的分组标题：不是第一行时上沿有一条分隔线，和菜单里分组的写法一样。
fn draw_heading(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
    i: usize,
    section: BranchSection,
) {
    let (row, clip) = placed_row(app, parts, i);
    let title = match section {
        BranchSection::Recent => t!("git-branch-picker-recent"),
        BranchSection::Local => t!("menu-git-local-branches"),
        BranchSection::Other => t!("git-branch-picker-other"),
    };
    let band = if i > 0 {
        draw_divider(frame, app.palette, row, row.y + 2.0, clip);
        Rect::new(row.x, row.y + 4.0, row.w, row.h - 4.0)
    } else {
        row
    };
    let x = row.x + TEXT_LEFT;
    ui_text_small(frame, app, x, band, title, app.palette.text_dim, clip);
}

/// 一个分支行的内容。
struct BranchLine<'a> {
    /// [`GitState::branches`](crate::app::git::GitState::branches) 的下标。
    index: usize,
    depth: usize,
    label: &'a str,
    /// 此刻的 Unix 时间（秒）：写最后一次提交离现在多久。
    now: i64,
}

/// 绘制第 `i` 行的一个分支：当前分支打勾，右端淡色写它的上游（没有上游时写「当前」），被别的
/// 工作树占着的写那个工作树，其余写最后一次提交的时间，都靠右对齐；选中的行和鼠标所在的行右端
/// 换成「⋯」。
fn draw_branch_row(
    app: &App,
    frame: &mut Frame<'_>,
    line: &Line,
    i: usize,
    row: &BranchLine<'_>,
    hits: &mut Vec<(Rect, PickerPart)>,
) {
    let (check, text) = line.columns(row.depth);
    let branch = app.git.branches.get(row.index);
    let current = branch.is_some_and(|b| b.current);
    if current {
        icons::check(frame, (check, line.cy), 9.0, line.text, line.clip);
    }
    let mut right = line.right();
    let more = line.draw_more(app, frame, i, hits);
    let held = branch
        .filter(|b| !b.remote)
        .and_then(|b| app.git.worktree_holding(local_name(b)))
        .and_then(|i| app.git.worktrees.get(i))
        .map(crate::app::git::worktree::worktree_name);
    let meta = match (held, current, branch) {
        (Some(name), ..) => Some(name),
        (_, true, Some(b)) => {
            Some((b.upstream.clone()).unwrap_or_else(|| t!("git-branch-picker-current").to_owned()))
        }
        (_, _, Some(b)) if !b.remote => Some(display_local(&b.local, b.time, row.now)),
        _ => None,
    };
    match (more, meta) {
        (Some(at), _) => right = at + NAME_GAP,
        (None, Some(meta)) => line.draw_meta(app, frame, &mut right, &meta),
        (None, None) => {}
    }
    let at = (text, right - NAME_GAP);
    line.draw_name(app, frame, at, row.label, line.text);
}

/// 绘制列表下方的底栏：上沿一条分隔线，里面是新建的入口，图标和字的位置与各行相同。
fn draw_footer(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
    hits: &mut Vec<(Rect, PickerPart)>,
) {
    let p = app.palette;
    let foot = parts.footer;
    draw_divider(frame, p, foot, foot.y, foot);

    let size = app.fonts.ui_small;
    let new_branch = (PickerPart::NewBranch, t!("git-branch-picker-new"));
    let new_worktree = (
        PickerPart::NewWorktree,
        t!("git-branch-picker-new-worktree"),
    );
    let in_repository = app.location_new_worktree_label();
    let entries: &[(PickerPart, &str)] = if app.picker_lists_locations() {
        &[(PickerPart::NewWorktree, in_repository.as_str())]
    } else {
        &[new_branch, new_worktree]
    };
    let mut x = foot.x + GUTTER;
    let y = foot.bottom() - GUTTER - BUTTON_SMALL;
    for &(part, text) in entries {
        let room = foot.w - 2.0 * GUTTER - (TEXT_LEFT - GUTTER) - 8.0;
        let text = fit(frame, text, size, room, false);
        let w = TEXT_LEFT - GUTTER + frame.measure(FontKind::Ui, size, &text) + 8.0;
        let button = Rect::new(x, y, w, BUTTON_SMALL);
        let hovered = app.hover == Hover::PickerPart(part);
        draw_button(frame, p, button, foot, hovered, false);
        let color = if hovered { p.text } else { p.text_muted };
        // 图标与文字按同一字号垂直居中。
        let icon = (x + ICON_CENTER - GUTTER, icon_center_y(app, button, size));
        icons::plus(frame, icon, 8.0, color, foot);
        ui_text_small(
            frame,
            app,
            x + TEXT_LEFT - GUTTER,
            button,
            &text,
            color,
            foot,
        );
        hits.push((button, part));
        x += w + BUTTON_GAP;
    }
}
