//! 详情栏里改动的文件和增删的行数。
//!
//! 平时由 [`draw_files`] 在详情一栏里逐个文件画出，每个文件一行，点击打开差异；
//! 截短的路径记进 `long_files`，鼠标悬停时提示完整路径。

use super::*;

/// 绘制改动的文件：标题行「N 个文件 +A −D」，下面一个文件一行（图标、按变化着色的名字、
/// 淡色的目录、右端的增删统计），点击后打开差异。
///
/// 从 `(x, y)` 开始画，宽 `width`，返回画完后的 y 坐标；坐标都是逻辑像素。目录写不下而截短、
/// 或改过名的文件记进 `long_files`：悬停时提示完整的路径。
pub(super) fn draw_files(
    app: &App,
    frame: &mut Frame<'_>,
    files: &[winc_editor::git::CommitFile],
    (x, mut y, width): (f32, f32, f32),
    area: Rect,
    (hits, long_files): (&mut Vec<(Rect, LogTarget)>, &mut Vec<usize>),
) -> f32 {
    let p = app.palette;
    let (added, removed) = files
        .iter()
        .filter_map(|f| f.lines)
        .fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
    // 标题行高 22 逻辑像素（经验值），下面留 4 逻辑像素再开始画文件。
    let row = Rect::new(x, y, width, 22.0);
    let end = ui_text_small(
        frame,
        app,
        x,
        row,
        &t!("git-log-files-changed", count = files.len()),
        p.text_muted,
        area,
    );
    let end = ui_text_small(
        frame,
        app,
        end + 10.0,
        row,
        &format!("+{added}"),
        p.git_added,
        area,
    );
    ui_text_small(
        frame,
        app,
        end + 6.0,
        row,
        &format!("−{removed}"),
        p.git_deleted,
        area,
    );
    y += 26.0;

    let root = app.git.root.as_deref();
    for (i, file) in files.iter().enumerate() {
        let row = Rect::new(area.x + 6.0, y, area.w - 12.0, FILE_ROW);
        y += FILE_ROW;
        if row.bottom() < area.y || row.y > area.bottom() {
            continue;
        }
        let hovered = app.pointer.hover == Hover::Log(LogTarget::File(i));
        // 键盘选中的文件（按 `→` 进入文件列表以后）铺选中的底色。
        let chosen = app.git.log.file == Some(i);
        if let Some(color) = row_color(p, chosen, app.focus == Focus::Editor, hovered) {
            frame.rect_clipped(row, color, RADIUS_ROW, area);
        }
        let cy = row.y + row.h / 2.0;
        let name = winc_editor::display_name(&file.path).into_owned();
        // 增删统计占行右端 96 逻辑像素（经验值）。
        let stat_w = 96.0;
        let clip = Rect::new(row.x, row.y, row.w - stat_w, row.h).intersect(area);
        let icon_end = draw_file_icon(frame, p, (x, cy), &name, clip);
        let color = git_color(p, file.change);
        let end = ui_text(frame, app, icon_end + 6.0, row, &name, color, clip);
        let dir = file
            .path
            .parent()
            .and_then(|d| root.and_then(|r| d.strip_prefix(r).ok()))
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = match (&file.from, file.change) {
            (Some(from), Change::Renamed) => format!("← {}", winc_editor::display_name(from)),
            _ => dir,
        };
        let room = (clip.right() - end - 12.0).max(0.0);
        let full = frame.measure(FontKind::Ui, app.fonts.ui_small, &dir);
        if full > room || file.from.is_some() {
            long_files.push(i);
        }
        if !dir.is_empty() {
            let dir = truncate_end(frame, &dir, app.fonts.ui_small, room);
            ui_text_small(frame, app, end + 8.0, row, &dir, p.text_dim, clip);
        }
        draw_stat(app, frame, file.lines, (row.right() - stat_w, row), area);
        hits.push((row.intersect(area), LogTarget::File(i)));
    }
    y
}

/// 绘制文件一行右边的增删统计：「+12 −3」，后面五格小方块按增删的比例上色。
///
/// 每格 6 逻辑像素见方，间距 1 逻辑像素，整组靠行右端对齐。
fn draw_stat(
    app: &App,
    frame: &mut Frame<'_>,
    lines: Option<(usize, usize)>,
    (x, row): (f32, Rect),
    clip: Rect,
) {
    let p = app.palette;
    let Some((added, removed)) = lines else {
        ui_text_small(
            frame,
            app,
            x + 30.0,
            row,
            t!("git-log-file-binary"),
            p.text_dim,
            clip,
        );
        return;
    };
    let text = format!("+{added} −{removed}");
    let w = frame.measure(FontKind::Ui, app.fonts.ui_small, &text);
    let blocks_x = row.right() - 5.0 * 7.0 - 6.0;
    let end = ui_text_small(
        frame,
        app,
        blocks_x - 8.0 - w,
        row,
        &format!("+{added}"),
        p.git_added,
        clip,
    );
    ui_text_small(
        frame,
        app,
        end + 4.0,
        row,
        &format!("−{removed}"),
        p.git_deleted,
        clip,
    );
    let total = added + removed;
    let green = if total == 0 {
        0
    } else {
        ((added as f32 / total as f32) * 5.0).round() as usize
    };
    let red = if total == 0 { 0 } else { 5 - green };
    let cy = row.y + row.h / 2.0;
    for k in 0..5 {
        let color = if k < green {
            p.git_added
        } else if k < green + red {
            p.git_deleted
        } else {
            p.separator
        };
        let block = Rect::new(blocks_x + k as f32 * 7.0, cy - 3.0, 6.0, 6.0);
        frame.rect_clipped(block, color, 1.5, clip);
    }
}
