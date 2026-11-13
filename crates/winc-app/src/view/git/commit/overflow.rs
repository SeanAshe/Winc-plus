//! 文件行右侧操作按钮收进「⋯」时的几何与绘制。

use super::*;

/// 计算鼠标停着的改动行写全名所需的溢出区域（见 [`CommitOverflow`]）。
///
/// 文件行的「打开文件」「回滚」两个按钮跟在全名之后。名字放得下、正在拖动时返回 `None`。
/// 展开收起的动画途中，被展开、收起的行及其上方各行位置不变，仍显示全名；下方正在移动的行
/// 返回 `None`。
pub(in crate::view) fn commit_overflow(app: &App, frame: &mut Frame<'_>) -> Option<CommitOverflow> {
    // 刷新以后各行还在移动（见 `draw_shift`），按行号对不上。
    if !app.commit_shown() || !app.idle_pointer() || app.git.commit.shift.is_some() {
        return None;
    }
    let status = app.git.status.as_ref()?;
    let root = app.git.root.as_deref()?;
    let rows = app.commit_rows();
    let row = match app.pointer.hover {
        Hover::Commit(
            CommitTarget::Dir(r) | CommitTarget::DirCheck(r) | CommitTarget::Sticky(r, _),
        ) => r,
        // 鼠标移到跟在全名后面的按钮上时，仍保持全名显示。
        Hover::Commit(
            CommitTarget::File(f)
            | CommitTarget::Check(f)
            | CommitTarget::OpenFile(f)
            | CommitTarget::RevertFile(f),
        ) => rows
            .iter()
            .position(|r| matches!(r, CommitRow::File { index, .. } if *index == f))?,
        _ => return None,
    };
    if app
        .git
        .commit
        .motion
        .as_ref()
        .is_some_and(|m| row > m.parent)
    {
        return None;
    }
    let list = app.commit_parts().list;
    let y = row_top(app, &rows, row, list);
    let text = row_text(app, status, root, rows.get(row)?)?;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    let end = name_end(app, frame, &text, band);
    let window = app.layout.window.right() - 8.0;
    let (right, actions) = if text.folder {
        if end <= band.right() - 6.0 {
            return None;
        }
        ((end + 8.0).min(window), None)
    } else if app.git.commit.browse {
        // 浏览历史提交的文件时没有按钮：名字放不下（状态字母之前）时才写全名。
        if end <= band.right() - END_CENTER * 2.0 {
            return None;
        }
        ((end + 8.0).min(window), None)
    } else {
        // 文件行的按钮在名字之后：名字与按钮在原位放得下时不展开。按钮超出窗口时向左移回，
        // 名字在按钮之前截断。
        let (open, revert) = file_actions(band);
        if end <= open.x - 4.0 {
            return None;
        }
        let shift = (end + 4.0 - open.x).min(window - revert.right());
        let moved = |r: Rect| Rect::new(r.x + shift, r.y, r.w, r.h);
        let (open, revert) = (moved(open), moved(revert));
        (revert.right() + 4.0, Some((open, revert)))
    };
    let spill = (right > list.right()).then(|| {
        Rect::new(
            list.right(),
            y + 1.0,
            right - list.right(),
            COMMIT_ROW - 2.0,
        )
    });
    Some(CommitOverflow {
        row,
        y,
        spill,
        actions,
    })
}

/// 返回第 `row` 行写全名时两个按钮离终点（「打开文件」在 `open`）还差的距离（逻辑像素）。
///
/// 按钮随名字滑出（名字还有 `name_lag` 未显示），同时从原位（[`file_actions`]）滑向终点
/// （[`App::track_commit_actions`]），取两者中较大的距离：名字无需加长、只为按钮让出位置时，
/// 名字不动，按钮仍然滑向终点。
pub(super) fn actions_lag(app: &App, row: usize, band: Rect, open: Rect, name_lag: f32) -> f32 {
    let slide = app.git.commit.actions_slide.filter(|&(r, _)| r == row);
    let rest = slide.map_or(0.0, |(_, clock)| {
        let (home, _) = file_actions(band);
        (open.x - home.x).max(0.0) * (1.0 - clock.progress())
    });
    name_lag.max(rest)
}

/// 返回名字显示出的宽度：内容宽 `width`，截短时可用宽度为 `room`（写全名时为 `None`）。
///
/// 截短后仍然放不下的名字（不能再缩短的文件名、目录名）在 `room` 处淡出，显示出的宽度为
/// `room`。名字的过渡按显示出的宽度进行：写全名时从截断处滑出，按钮随之移动；内容不变时
/// 同样有过渡。
pub(super) fn shown_width(width: f32, room: Option<f32>) -> f32 {
    room.map_or(width, |room| width.min(room))
}

/// 返回 `band` 一行中名字（平铺时连同其后的目录）完整绘制时末尾的 x 坐标（逻辑像素）。
pub(super) fn name_end(app: &App, frame: &mut Frame<'_>, text: &RowText, band: Rect) -> f32 {
    let name_w = if text.folder {
        dir_label_width(app, frame, &text.name)
    } else {
        frame.measure(FontKind::Ui, app.fonts.ui, &text.name)
    };
    let mut end = band.x + text.offset + name_w;
    if let Some(dir) = &text.dir {
        end += 8.0 + frame.measure(FontKind::Ui, app.fonts.ui_small, dir);
    }
    end
}

/// 在编辑区上面继续绘制放不下的改动行名字：底色、文字和列表里的那一行连成一条。
///
/// 命中范围按全名算（见 [`commit_overflow`]），画的时候按名字展开到的宽度露出，和文件树相同：
/// 过渡途中鼠标移过去不会落空。
pub(in crate::view) fn draw_commit_overflow(app: &App, frame: &mut Frame<'_>) {
    let Some(CommitOverflow {
        row,
        y,
        spill: Some(spill),
        actions,
    }) = app.git.commit.overflow
    else {
        return;
    };
    let (Some(status), Some(root)) = (app.git.status.as_ref(), app.git.root.as_deref()) else {
        return;
    };
    let rows = app.commit_rows();
    let Some(text) = rows.get(row).and_then(|r| row_text(app, status, root, r)) else {
        return;
    };
    let Some(view) = app.git.commit.names.peek(&name_key(status, &rows[row])) else {
        return;
    };
    let list = app.commit_parts().list;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    let x = band.x + text.offset;
    // 按钮跟着名字移动（见 `draw_row`），露出的部分到按钮为止；目录行露到名字后面 8。
    let parts = full_parts(&text);
    let width = parts_width(app, frame, &parts, text.folder);
    let lag = match actions {
        Some((open, _)) => actions_lag(app, row, band, open, width - view.width),
        None => width - view.width,
    };
    let actions = actions.map(|(open, revert)| {
        let back = |r: Rect| Rect::new(r.x - lag, r.y, r.w, r.h);
        (back(open), back(revert))
    });
    let right = match actions {
        Some((_, revert)) => revert.right() + 4.0,
        None => x + view.width + 8.0,
    }
    .min(spill.right());
    if right <= spill.x {
        return;
    }
    // 这一行绘制在最上层：底色从滚动条的可点击范围开始，覆盖这一行高度内的滚动条，滚动条不会
    // 穿过全名；滚动条在这一行上下的部分正常显示。
    let left = list.right() - SCROLLBAR_HIT;
    let spill = Rect::new(left, spill.y, right - left, spill.h);
    let p = app.palette;
    let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
    // 先垫一层侧栏底色，再叠半透明的悬停色，与列表里那一行的底色相同。
    frame.rect_corners(spill, p.sidebar_bg, radii, spill);
    if let Some(color) = text.shade {
        frame.rect_corners(spill, color, radii, spill);
    }
    let clip = Rect::new(spill.x, y, spill.w, COMMIT_ROW);
    // 有按钮时名字在按钮之前截断。
    let text_clip = match actions {
        Some((open, _)) => Rect::new(clip.x, clip.y, (open.x - 4.0 - clip.x).max(0.0), clip.h),
        None => clip,
    };
    let style = (x, band, text.folder, text.color);
    draw_name(app, frame, style, (&parts, &view), text_clip);
    if let (Some(actions), CommitRow::File { index, .. }) = (actions, &rows[row]) {
        draw_file_actions(app, frame, actions, *index, clip);
    }
}

/// 绘制鼠标移开后正在收回、还伸在侧栏外面的改动行名字：不响应点击，收回到侧栏边缘时消失。
///
/// 底色和列表里的那一段一起淡出（`retracting_color`），写的是收回前的全名，与文件树相同。
pub(in crate::view) fn draw_commit_overflow_closing(app: &App, frame: &mut Frame<'_>) {
    if !app.commit_shown() || !app.git.commit.names.moving() {
        return;
    }
    let (Some(status), Some(root)) = (app.git.status.as_ref(), app.git.root.as_deref()) else {
        return;
    };
    let p = app.palette;
    let rows = app.commit_rows();
    let list = app.commit_parts().list;
    let open = app.git.commit.overflow.map(|o| o.row);
    let window = app.layout.window.right() - 8.0;
    let focused = app.focus == Focus::CommitList;
    for (key, view) in app.git.commit.names.moving_rows() {
        let Some(row) = (0..rows.len()).find(|&i| name_key(status, &rows[i]) == key) else {
            continue;
        };
        if Some(row) == open
            || app
                .git
                .commit
                .motion
                .as_ref()
                .is_some_and(|m| row > m.parent)
        {
            continue;
        }
        let Some(text) = row_text(app, status, root, &rows[row]) else {
            continue;
        };
        let y = row_top(app, &rows, row, list);
        let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
        let x = band.x + text.offset;
        let right = (x + view.width + 8.0).min(window);
        if right <= list.right() {
            continue;
        }
        let left = list.right() - SCROLLBAR_HIT;
        let rect = Rect::new(left, y + 1.0, right - left, COMMIT_ROW - 2.0);
        let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
        frame.rect_corners(rect, p.sidebar_bg, radii, rect);
        let selected = match &rows[row] {
            CommitRow::File { index, .. } => app.commit_is_selected(&status.entries[*index].path),
            _ => false,
        };
        if let Some(color) = retracting_color(p, selected, focused, view.progress) {
            frame.rect_corners(rect, color, radii, rect);
        }
        let clip = Rect::new(rect.x, y, rect.w, COMMIT_ROW);
        // 收回前的全名：相同的前缀不淡，其余随过渡淡出。内容没变（只是宽度在收）时照写全名。
        let parts: Vec<Cow<'_, str>> = if view.old.is_empty() {
            full_parts(&text)
        } else {
            view.old.iter().map(|s| Cow::Borrowed(s.as_str())).collect()
        };
        let alpha = |k: usize| {
            if view.old.is_empty() || k < view.common {
                1.0
            } else {
                crossfade(&view).0
            }
        };
        if text.folder {
            draw_dir_parts(app, frame, (x, band), &parts, alpha, clip);
        } else {
            draw_file_parts(app, frame, (x, band), &parts, (text.color, alpha), clip);
        }
    }
}
