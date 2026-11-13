//! 改动列表中一行的绘制：分组标题、目录和文件三类行，以及各行共用的几何。

use super::*;

/// 改动列表中一行的几何和各类行共用的状态。
struct RowBox<'a> {
    /// 列表各行共用的状态。
    cx: &'a ListContext<'a>,
    /// 行在列表中的下标。
    i: usize,
    /// 行的上沿（逻辑像素）。
    y: f32,
    /// 行的范围：左右各比列表窄 8 逻辑像素。
    band: Rect,
    /// 行底色的范围：上下各比 `band` 少 1 逻辑像素，相邻两行都有底色时中间留一道缝。
    shade: Rect,
    /// 名字截短时的裁剪范围。
    text_clip: Rect,
    /// 这一行正在显示全名时的状态；鼠标不在这一行或名字放得下时为 `None`。
    full: Option<CommitOverflow>,
    /// 全名伸出侧栏的那一段；没有伸出时为 `None`。
    spill: Option<Rect>,
    /// 这一行的名字是否显示全名。按行判断，不看位置：固定在顶部的行与列表中被它盖住的
    /// 那一行相同。
    wanted: bool,
    /// 名字的显示方式变化的原因，交给名字的过渡状态。
    cause: LabelCause,
    /// 键盘焦点是否在改动列表上。
    focused: bool,
    /// 是否在浏览历史提交的文件：此时没有勾选框，也没有回滚、打开文件等按钮。
    browse: bool,
}

impl RowBox<'_> {
    /// 返回第 `level` 层箭头的横坐标：每深一层向右缩进 `INDENT`。
    fn arrow_x(&self, level: usize) -> f32 {
        self.band.x + ARROW + level as f32 * INDENT
    }

    /// 返回第 `level` 层勾选框中心的横坐标：目录的箭头在勾选框左边。
    fn check_x(&self, app: &App, level: usize) -> f32 {
        self.arrow_x(level) + check_gap(app)
    }

    /// 在 `(x, cy)` 画勾选框；浏览历史提交时不画。
    fn checkbox(&self, app: &App, frame: &mut Frame<'_>, x: f32, cy: f32, state: Option<bool>) {
        if self.browse {
            return;
        }
        let p = app.palette;
        let border = p.text_dim.alpha(0.55);
        let list = self.cx.list;
        icons::checkbox(
            frame,
            (x, cy),
            state,
            p.accent,
            border,
            p.text_on_accent,
            list,
        );
    }

    /// 记下勾选框的命中区域；浏览历史提交时没有勾选框，不记录。
    fn check_hit(&self, hits: &mut Vec<(Rect, CommitTarget)>, rect: Rect, target: CommitTarget) {
        if !self.browse {
            hits.push((rect.intersect(self.cx.list), target));
        }
    }

    /// 铺行的底色。名字伸出侧栏或 `wide` 为真时铺到侧栏边缘、右边是直角，与伸出去的那一段
    /// 连成一条。
    fn fill(&self, frame: &mut Frame<'_>, color: Color, wide: bool) {
        let (shade, list) = (self.shade, self.cx.list);
        if self.spill.is_some() || wide {
            let r = RADIUS_ROW;
            let wide = Rect::new(shade.x, shade.y, list.right() - shade.x, shade.h);
            frame.rect_corners(wide, color, [r, 0.0, 0.0, r], list);
        } else {
            frame.rect_clipped(shade, color, RADIUS_ROW, list);
        }
    }
}

/// 画改动列表里的第 `i` 行，上沿在 `y`（逻辑像素）；记下可点击的位置。
pub(super) fn draw_row(
    app: &App,
    frame: &mut Frame<'_>,
    cx: &ListContext<'_>,
    i: usize,
    y: f32,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let list = cx.list;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    // 鼠标停着的这一行名字放不下：写全名，状态字母让开；伸出侧栏时底色铺到侧栏边缘。
    let full = (app.git.commit.overflow).filter(|o| o.row == i && (o.y - y).abs() < 0.5);
    let wanted = app.git.commit.overflow.is_some_and(|o| o.row == i);
    let r = RowBox {
        cx,
        i,
        y,
        band,
        shade: Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0),
        text_clip: Rect::new(list.x, y, band.right() - 6.0 - list.x, COMMIT_ROW).intersect(list),
        full,
        spill: full.and_then(|o| o.spill),
        wanted,
        cause: LabelCause {
            hovered: wanted,
            levels: 0,
            current: 0,
        },
        focused: app.focus == Focus::CommitList,
        browse: app.git.commit.browse,
    };
    let row = &cx.rows[i];
    // 缩进参考线：每一层上级目录一条，位于该目录的箭头正下方，相邻行连成一条竖线，
    // 目录层级深时能看出每个文件属于哪个目录。
    if let CommitRow::Dir { depth, .. } | CommitRow::File { depth, .. } = row {
        for level in 1..=*depth {
            let x = r.arrow_x(level).round();
            let guide = Rect::new(x, y, 1.0, COMMIT_ROW);
            frame.rect_clipped(guide, app.palette.indent_guide, 0.0, list);
        }
    }
    match row {
        CommitRow::Group(group) => draw_group_row(app, frame, &r, *group, hits),
        CommitRow::Dir {
            group,
            dir,
            label,
            depth,
        } => draw_dir_row(app, frame, &r, (*group, dir, label, *depth), hits),
        CommitRow::File { index, depth } => draw_file_row(app, frame, &r, (*index, *depth), hits),
    }
}

/// 画分组的标题行：箭头、勾选框、标题，右端是文件数，悬停时换成「回滚」按钮。
fn draw_group_row(
    app: &App,
    frame: &mut Frame<'_>,
    r: &RowBox<'_>,
    group: CommitGroup,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let ListContext {
        list, status, rows, ..
    } = *r.cx;
    let RowBox {
        i,
        y,
        band,
        shade,
        browse,
        ..
    } = *r;
    let cy = y + COMMIT_ROW / 2.0;
    let hovered = matches!(
        app.pointer.hover,
        Hover::Commit(
            CommitTarget::Group(g) | CommitTarget::GroupCheck(g) | CommitTarget::GroupRevert(g)
        ) if g == group
    );
    if hovered {
        frame.rect_clipped(shade, p.hover, RADIUS_ROW, list);
    }
    let open = !app.git.commit.group_collapsed(group);
    chevron(app, frame, (r.arrow_x(0), cy), (i, open), p.text_dim, list);
    let group_check = r.check_x(app, 0);
    let members = rows.group(group);
    let checks = app.git.commit.checks();
    let state = rows.members_state(members, checks, || {
        app.git.commit.members_state(status, members)
    });
    r.checkbox(app, frame, group_check, cy, state);
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
            let on = app.pointer.hover == Hover::Commit(target);
            draw_button(frame, p, rect, list, on, false);
            let color = if on { p.text } else { p.text_muted };
            icons::rollback(frame, center(rect), color, list);
            hits.push((rect.intersect(list), target));
        }
        None => draw_count_pill(app, frame, band, &members.len().to_string(), list),
    }
    let check = Rect::new(group_check - 8.0, y + 4.0, 16.0, 16.0);
    r.check_hit(hits, check, CommitTarget::GroupCheck(group));
    hits.push((band.intersect(list), CommitTarget::Group(group)));
}

/// 画目录行：`(所在的组, 目录, 显示的名字, 缩进的层数)`。
fn draw_dir_row(
    app: &App,
    frame: &mut Frame<'_>,
    r: &RowBox<'_>,
    (group, dir, name, depth): (CommitGroup, &Path, &str, usize),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let ListContext {
        list,
        status,
        rows,
        multi,
        ..
    } = *r.cx;
    let RowBox {
        i,
        y,
        band,
        text_clip,
        full,
        spill,
        wanted,
        cause,
        focused,
        ..
    } = *r;
    let cy = y + COMMIT_ROW / 2.0;
    // 组标题已经写了文件数，展开后每个文件都看得见，每层再写一遍是重复信息。
    let level = depth + 1;
    let hovered = matches!(
        app.pointer.hover,
        Hover::Commit(CommitTarget::Dir(r) | CommitTarget::DirCheck(r)) if r == i
    );
    // 多选时下面的文件都选中了：画成选中状态。
    let members = rows.dir(group, dir);
    // 选中的文件比目录下面的少时不可能都选中，不必逐个查。
    let is_selected = multi
        && !members.is_empty()
        && app.commit_marked_count() >= members.len()
        && members
            .iter()
            .all(|&m| app.commit_is_selected(&status.entries[m].path));
    let icon_x = r.check_x(app, level) + ICON;
    let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
    let room = (!wanted).then(|| (text_clip.right() - text_x).max(0.0));
    let parts = dir_parts(app, frame, name, room);
    let width = parts_width(app, frame, &parts, true);
    let key = CommitName::Dir(group, dir.to_path_buf());
    let view = app
        .git
        .commit
        .names
        .sync(&key, cause, &parts, shown_width(width, room));
    let retracting = !wanted && view.progress < 1.0 && text_x + view.width > text_clip.right();
    let color = if retracting {
        retracting_color(p, is_selected, focused, view.progress)
    } else {
        row_color(p, is_selected, focused, hovered || full.is_some())
    };
    if let Some(color) = color {
        r.fill(frame, color, retracting);
    }
    let open = !app.git.commit.dir_collapsed(group, dir);
    chevron(
        app,
        frame,
        (r.arrow_x(level), cy),
        (i, open),
        p.text_dim,
        list,
    );
    let dir_check = r.check_x(app, level);
    let checks = app.git.commit.checks();
    let state = rows.members_state(members, checks, || {
        app.git.commit.members_state(status, members)
    });
    r.checkbox(app, frame, dir_check, cy, state);
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
    r.check_hit(hits, check, CommitTarget::DirCheck(i));
    hits.push((band.intersect(list), CommitTarget::Dir(i)));
}

/// 画文件行：`(文件在状态中的下标, 缩进的层数)`。
fn draw_file_row(
    app: &App,
    frame: &mut Frame<'_>,
    r: &RowBox<'_>,
    (index, depth): (usize, usize),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let ListContext {
        list, status, root, ..
    } = *r.cx;
    let RowBox {
        i,
        y,
        band,
        text_clip,
        full,
        spill,
        wanted,
        cause,
        focused,
        browse,
        ..
    } = *r;
    let cy = y + COMMIT_ROW / 2.0;
    let entry = &status.entries[index];
    let is_selected = app.commit_is_selected(&entry.path);
    let hovered = matches!(
        app.pointer.hover,
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
    let file_check = r.check_x(app, file_column(app, depth));
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let icon_x = file_check + ICON;
    let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
    let dir = (app.git.commit.flat())
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
        .git
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
        r.fill(
            frame,
            color,
            retracting && text_x + view.width > text_clip.right(),
        );
    }
    r.checkbox(app, frame, file_check, cy, app.git.commit.file_state(entry));
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
    let name_clip = Rect::new(list.x, y, (name_end - list.x).max(0.0), COMMIT_ROW).intersect(list);
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
    r.check_hit(hits, check, CommitTarget::Check(index));
    hits.push((band.intersect(list), CommitTarget::File(index)));
}
