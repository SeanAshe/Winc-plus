//! 贮藏列表及其中文件行的绘制。

use super::*;

/// 绘制改动列表下面的储藏一组：上沿一条分隔线；标题是箭头、「储藏」和数量；展开时下面每条储藏一行，
/// 右端淡色写储藏时的分支和时间，悬停时换成「应用」和「⋯」；正在预览标签里看的那一条底色同选中。
/// 储藏太多时最后一行是「还有 N 条」。
pub(super) fn draw_stashes(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let Some(area) = parts.stashes else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(area);
    frame.rect_clipped(
        Rect::new(area.x, area.y, area.w, 1.0),
        p.separator,
        0.0,
        clip,
    );
    let size = app.fonts.ui_small;
    let band = |row: usize| {
        let y = area.y + crate::app::STASH_GAP + row as f32 * COMMIT_ROW;
        Rect::new(area.x + 8.0, y, area.w - 16.0, COMMIT_ROW)
    };
    let shade = |band: Rect| Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);

    // 标题：和改动列表的组标题一样，小一号的次要色字，右端一个写数量的徽标。
    let head = band(0);
    if app.pointer.hover == Hover::Commit(CommitTarget::Stashes) {
        frame.rect_clipped(shade(head), p.hover, RADIUS_ROW, clip);
    }
    let cy = head.y + head.h / 2.0;
    let turned = app.stash_chevron() * FRAC_PI_2;
    icons::chevron_turned(frame, (head.x + ARROW, cy), 6.0, turned, p.text_dim, clip);
    let title = t!("git-stash-group");
    label(
        frame,
        app,
        size,
        (head.x + STASH_TITLE, head),
        title,
        p.text_muted,
        clip,
    );
    draw_count_pill(app, frame, head, &app.git.stashes.len().to_string(), clip);
    hits.push((head, CommitTarget::Stashes));
    // 展开、收起的途中各行跟着标题移动，露出多少由一组的范围裁掉；这时行还没停稳，不响应点击。
    let moving = app.git.commit.stash_motion.is_some();
    if !app.git.commit.stash_open && !moving {
        return;
    }
    let mut still = Vec::new();
    let hits = if moving { &mut still } else { hits };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let viewing = (app.git.commit.stash_view.as_ref())
        .filter(|_| app.editor.active().map(|d| d.id) == app.git.commit.stash_preview)
        .map(|view| view.oid.as_str());
    let listed = app.listed_stash_view();
    for (n, line) in app.stash_lines().into_iter().enumerate() {
        let row = band(n + 1);
        let i = match line {
            StashLine::Stash(i) => i,
            StashLine::File(at) => {
                if let Some(view) = listed {
                    draw_stash_file(app, frame, row, view, at, clip);
                    hits.push((row, CommitTarget::StashFile(at)));
                }
                continue;
            }
            StashLine::MoreFiles(count) => {
                let text = t!("git-stash-more-files", count = count);
                let x = row.x + STASH_FILE;
                ui_text_small(frame, app, x, row, &text, p.text_dim, clip);
                continue;
            }
            StashLine::MoreStashes(rest) => {
                if app.pointer.hover == Hover::Commit(CommitTarget::StashMore) {
                    frame.rect_clipped(shade(row), p.hover, RADIUS_ROW, clip);
                }
                let text = t!("git-stash-more", count = rest);
                let x = row.x + STASH_TITLE;
                ui_text_small(frame, app, x, row, &text, p.text_muted, clip);
                hits.push((row, CommitTarget::StashMore));
                continue;
            }
        };
        let stash = &app.git.stashes[i];
        let hovered = matches!(
            app.pointer.hover,
            Hover::Commit(
                CommitTarget::Stash(h) | CommitTarget::StashApply(h) | CommitTarget::StashMenu(h)
            ) if h == i
        );
        let selected = viewing == Some(stash.oid.as_str());
        if let Some(color) = row_color(p, selected, false, hovered) {
            frame.rect_clipped(shade(row), color, RADIUS_ROW, clip);
        }
        // 右端：平时是分支和时间，悬停时换成「应用」和「⋯」。
        let mut right = row.right() - END_PAD;
        if hovered {
            let (_, more) = file_actions(row);
            let on = app.pointer.hover == Hover::Commit(CommitTarget::StashMenu(i));
            draw_button(frame, p, more, clip, on, false);
            let color = if on { p.text } else { p.text_muted };
            icons::more(frame, center(more), color, clip);
            hits.push((more, CommitTarget::StashMenu(i)));
            let text = t!("git-stash-apply");
            let w = frame.measure(FontKind::Ui, size, text) + 2.0 * STASH_APPLY_PAD;
            let apply = Rect::new(more.x - BUTTON_GAP - w, more.y, w, more.h);
            let on = app.pointer.hover == Hover::Commit(CommitTarget::StashApply(i));
            draw_button(frame, p, apply, clip, on, false);
            ui_text_small(
                frame,
                app,
                apply.x + STASH_APPLY_PAD,
                apply,
                text,
                p.accent,
                clip,
            );
            hits.push((apply, CommitTarget::StashApply(i)));
            right = apply.x - 6.0;
        } else {
            let time = display_local(&stash.local, stash.time, now);
            let meta = match stash.branch() {
                Some(branch) => format!("{branch} · {time}"),
                None => time,
            };
            let room = (right - row.x - STASH_TITLE) / 2.0;
            let meta = fit(frame, &meta, size, room.max(0.0), false);
            let w = frame.measure(FontKind::Ui, size, &meta);
            ui_text_small(frame, app, right - w, row, &meta, p.text_dim, clip);
            right -= w + 8.0;
        }
        let x = row.x + STASH_TITLE;
        let name = crate::app::git::branch::stash_title(stash);
        let name = fit(frame, &name, app.fonts.ui, (right - x).max(0.0), false);
        let room = clip.intersect(Rect::new(row.x, row.y, (right - row.x).max(0.0), row.h));
        label(frame, app, app.fonts.ui, (x, row), &name, p.text, room);
        hits.push((row, CommitTarget::Stash(i)));
    }
}

/// 绘制正在查看的储藏下面的一个文件：比储藏名再缩进一级，依次是文件图标、文件名、淡色的所在
/// 目录，右端是状态字母；正在查看的文件铺选中的底色。
pub(super) fn draw_stash_file(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    view: &StashView,
    at: usize,
    clip: Rect,
) {
    let p = app.palette;
    let Some((file, ..)) = view.files.get(at) else {
        return;
    };
    let hovered = app.pointer.hover == Hover::Commit(CommitTarget::StashFile(at));
    let shade = Rect::new(row.x, row.y + 1.0, row.w, row.h - 2.0);
    if let Some(color) = row_color(p, at == view.at, false, hovered) {
        frame.rect_clipped(shade, color, RADIUS_ROW, clip);
    }
    let size = app.fonts.ui_small;
    let cy = row.y + row.h / 2.0;
    let name = winc_editor::display_name(&file.path).into_owned();
    let x = row.x + STASH_FILE;
    draw_file_icon(frame, p, (x, cy), &name, clip);
    let letter = file.change.letter().to_string();
    let letter_w = frame.measure(FontKind::Ui, size, &letter);
    let right = row.right() - END_PAD;
    let ink = git_color(p, file.change);
    ui_text_small(frame, app, right - letter_w, row, &letter, ink, clip);
    let text_x = x + icons::FILE_WIDTH + 6.0;
    let room = (right - letter_w - 8.0 - text_x).max(0.0);
    let name_w = frame.measure(FontKind::Ui, app.fonts.ui, &name).min(room);
    let shown = fit(frame, &name, app.fonts.ui, room, false);
    let color = file_color(file.change, p);
    let area = clip.intersect(Rect::new(text_x, row.y, room, row.h));
    label(frame, app, app.fonts.ui, (text_x, row), &shown, color, area);
    // 所在目录相对仓库根目录，写在名字后面，放不下时截短。
    let dir = app.editor.root().and_then(|root| {
        let parent = file.path.parent()?.strip_prefix(root).ok()?;
        let dir = parent.to_string_lossy().into_owned();
        (!dir.is_empty()).then_some(dir)
    });
    if let Some(dir) = dir {
        let dir_x = text_x + name_w + 6.0;
        let dir_room = (text_x + room - dir_x).max(0.0);
        let dir = fit(frame, &dir, size, dir_room, false);
        ui_text_small(frame, app, dir_x, row, &dir, p.text_dim, clip);
    }
}
