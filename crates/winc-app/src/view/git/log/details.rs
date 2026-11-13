//! 详情栏：选中提交的说明、比较、底部的提示，以及可以选中的文字。
//!
//! [`draw_details`] 从选中提交的内容自上而下画出说明、引用徽章、作者与时间、提交号与父提交、
//! 改动的文件；选中与取回不同步时（`held_details`）先继续显示上一个提交。可选中复制的文字
//! 记进 `runs`，由 `App::log_text_at` 反查。

use std::ops::Range;
use std::rc::Rc;

use winc_i18n::t;

use super::*;
use crate::app::git::log::Wrapped;

/// 提示中「单击比较」所按修饰键的写法（见 `App::command_held`）。
const COMMAND: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl"
};

/// 说明 `message` 按宽度 `width`（逻辑像素）折好的行；与上次的结果相同时直接复用缓存。
fn wrapped(app: &App, frame: &mut Frame<'_>, message: &str, width: f32) -> Rc<Wrapped> {
    let (ui, small) = (app.fonts.ui, app.fonts.ui_small);
    let key = (width.to_bits(), ui.to_bits(), small.to_bits());
    let mut cache = app.git.log.wrapped.borrow_mut();
    if let Some(w) = cache.as_ref()
        && w.key.0 == message
        && (w.key.1, w.key.2, w.key.3) == key
    {
        return Rc::clone(w);
    }
    let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
    let (subject, body) = (subject.trim(), body.trim());
    let ranges = |base: &str, lines: Vec<&str>| -> Vec<Range<usize>> {
        let at = |s: &str| s.as_ptr() as usize - base.as_ptr() as usize;
        lines.into_iter().map(|l| at(l)..at(l) + l.len()).collect()
    };
    let wrapped = Rc::new(Wrapped {
        key: (message.to_owned(), key.0, key.1, key.2),
        subject: ranges(subject, wrap(frame, ui, subject, width)),
        body: if body.is_empty() {
            Vec::new()
        } else {
            (body.lines())
                .map(|paragraph| ranges(paragraph, wrap(frame, small, paragraph, width)))
                .collect()
        },
    });
    *cache = Some(Rc::clone(&wrapped));
    wrapped
}

/// 绘制详情一栏：选中提交的说明、引用徽章、作者与时间、提交号与父提交、改动的文件。
///
/// 返回内容底部的位置（逻辑像素）。
pub(super) fn draw_details(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    parts: &LogParts,
    (hits, long_files, runs): (
        &mut Vec<(Rect, LogTarget)>,
        &mut Vec<usize>,
        &mut Vec<TextRun>,
    ),
) -> f32 {
    let p = app.palette;
    let area = parts.details;
    // 内容的上沿（随滚动往上走）：没有内容时返回它，内容高度就是 0。
    let top = area.y - view.details_scroll;
    // 列表空着（筛完没有、还在读、出错）：提示在 `draw_list` 里整个标签居中写，这里不画。
    if view.commits.is_empty() {
        return top;
    }
    if let Some(compare) = &view.compare {
        return draw_compare(app, frame, view, compare, area, (hits, long_files));
    }
    // 最下面一行快捷键提示在 `draw_details_footer` 里画（不跟着内容淡入）。
    let area = Rect::new(area.x, area.y, area.w, area.h - 30.0);
    let Some(selected) = view.selected.and_then(|i| view.commits.get(i)) else {
        // 和其他空列表共用 `draw_empty_state`。
        draw_empty_state(
            app,
            frame,
            (area, area),
            t!("git-log-no-selection"),
            Some(t!("git-log-no-selection-detail")),
        );
        return top;
    };
    // 新选的提交的详情还没取齐、又刚开始取：继续显示上一个提交的详情（`previous`），取回来
    // 一次换掉。画着的提交与选中的提交不同时，它的文件、提交号点击无效，命中范围先收集到
    // 一边再丢弃。
    let current = view.details.as_ref().filter(|d| d.oid == selected.oid);
    let held = held_details(app, view, selected);
    let mut ignored = Vec::new();
    let (commit, details, hits) = match held {
        Some((commit, d)) => (commit, Some(d), &mut ignored),
        None => (selected, current, hits),
    };
    let x = area.x + PAD;
    let width = area.w - 2.0 * PAD;
    let mut y = area.y + details_top(app.fonts.line_height) - view.details_scroll;

    // 说明：第一行用正文字号，其余淡色小字，按宽度折行。
    let message = details
        .and_then(|d| d.message.as_deref())
        .unwrap_or(&commit.subject);
    let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
    let (subject, body) = (subject.trim(), body.trim());
    let wrapped = wrapped(app, frame, message, width);
    // 说明、作者、邮箱、时间都可以用鼠标选中、⌘C 复制（`selectable`）。
    for (i, line) in wrapped.subject.iter().enumerate() {
        let row = Rect::new(x, y, width, 24.0);
        let style = (app.fonts.ui, p.text, i > 0, None);
        selectable(
            app,
            frame,
            runs,
            (x, row),
            &subject[line.clone()],
            style,
            area,
        );
        y += 24.0;
    }
    if !body.is_empty() {
        y += 4.0;
        for (paragraph, lines) in body.lines().zip(&wrapped.body) {
            for (i, line) in lines.iter().enumerate() {
                let row = Rect::new(x, y, width, 20.0);
                let style = (app.fonts.ui_small, p.text_muted, i > 0, None);
                let line = &paragraph[line.clone()];
                selectable(app, frame, runs, (x, row), line, style, area);
                y += 20.0;
            }
        }
    }

    // 指向这个提交的分支、标签：列表里放不下时截短、收成「+N」，这里写全名，放不下换行。
    let refs = badges(p, commit);
    if !refs.is_empty() {
        y += 8.0;
        let mut bx = x;
        for (text, fill, ink) in &refs {
            let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + 12.0;
            if bx > x && bx + w > x + width {
                bx = x;
                y += 24.0;
            }
            let text = truncate_end(frame, text, app.fonts.ui_small, width - 12.0);
            let row = Rect::new(x, y, width, 22.0);
            bx = draw_badge(app, frame, (bx, row), &text, *fill, *ink, area) + 6.0;
        }
        y += 22.0;
    }

    // 作者、时间、提交号、父提交。
    y += 12.0;
    let row = Rect::new(x, y, width, 24.0);
    draw_avatar(app, frame, commit, (x + 7.0, y + 12.0), area);
    let style = (app.fonts.ui_small, p.text, false, None);
    let end = selectable(
        app,
        frame,
        runs,
        (x + 22.0, row),
        &commit.author,
        style,
        area,
    );
    // 邮箱很长（自动生成的长地址）时从末尾截短，不伸出详情一栏。
    let room = (x + width - end - 6.0).max(0.0);
    let email = truncate_end(frame, &commit.email, app.fonts.ui_small, room);
    let full = (email != commit.email).then_some(commit.email.as_str());
    let style = (app.fonts.ui_small, p.text_dim, false, full);
    selectable(app, frame, runs, (end + 6.0, row), &email, style, area);
    y += 24.0;
    let row = Rect::new(x, y, width, 22.0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let style = (app.fonts.ui_small, p.text_muted, false, None);
    let end = selectable(app, frame, runs, (x, row), &commit.local, style, area);
    // 「今天」「昨天」「N 分钟前」比日期多给出信息时才写；只有日期就不重复。
    let relative = display_time(commit, now);
    if is_relative_time(commit, now) {
        ui_text_small(frame, app, end + 8.0, row, &relative, p.text_dim, area);
    }
    y += 22.0;

    // 提交号可以点（复制），父提交可以点（选中该提交）。
    let row = Rect::new(x, y, width, 22.0);
    let end = ui_text_small(
        frame,
        app,
        x,
        row,
        t!("git-log-details-commit"),
        p.text_dim,
        area,
    );
    let oid_x = end + 8.0;
    let hovered = app.pointer.hover == Hover::Log(LogTarget::CopyOid);
    let oid_end = mono(
        app,
        frame,
        (oid_x, row),
        short(&commit.oid),
        link(p, hovered),
        area,
    );
    hits.push((
        Rect::new(oid_x - 2.0, y, oid_end - oid_x + 4.0, 22.0).intersect(area),
        LogTarget::CopyOid,
    ));
    let mut px = oid_end + 16.0;
    if !commit.parents.is_empty() {
        let label = t!("git-log-details-parents", count = commit.parents.len());
        px = ui_text_small(frame, app, px, row, &label, p.text_dim, area) + 8.0;
        for (i, parent) in commit.parents.iter().enumerate() {
            let hovered = app.pointer.hover == Hover::Log(LogTarget::Parent(i));
            let end = mono(app, frame, (px, row), short(parent), link(p, hovered), area);
            hits.push((
                Rect::new(px - 2.0, y, end - px + 4.0, 22.0).intersect(area),
                LogTarget::Parent(i),
            ));
            px = end + 8.0;
        }
    }
    y += 22.0;

    // 改动的文件。
    y += 12.0;
    frame.rect_clipped(Rect::new(x, y, width, 1.0), p.separator, 0.0, area);
    y += 10.0;
    let Some(details) = details else {
        return y;
    };
    if let Some(error) = &details.error {
        let row = Rect::new(x, y, width, 22.0);
        ui_text_small(frame, app, x, row, error, p.warning, area);
        y += 26.0;
    }
    let Some(files) = &details.files else {
        // 刚开始取时先空着（`log_quiet`），一般马上就有，不会先显示提示再换掉。
        if !app.log_quiet() {
            let row = Rect::new(x, y, width, 22.0);
            let text = t!("git-log-details-loading-files");
            ui_text_small(frame, app, x, row, text, p.text_dim, area);
        }
        return y + 22.0;
    };
    draw_files(app, frame, files, (x, y, width), area, (hits, long_files))
}

/// 绘制比较两个版本时的详情：「比较」，旧的一边、新的一边（提交号或分支名，后面是说明），
/// 下面是两者之间改了的文件。
///
/// 返回内容底部的位置（逻辑像素）。
fn draw_compare(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    compare: &winc_editor::git::log::Compare,
    area: Rect,
    (hits, long_files): (&mut Vec<(Rect, LogTarget)>, &mut Vec<usize>),
) -> f32 {
    let p = app.palette;
    let x = area.x + PAD;
    let width = area.w - 2.0 * PAD;
    let mut y = area.y + details_top(app.fonts.line_height) - view.details_scroll;
    let row = Rect::new(x, y, width, 24.0);
    ui_text(frame, app, x, row, t!("git-compare-title"), p.text, area);
    y += 30.0;
    let sides = [
        (t!("git-compare-old"), &compare.from),
        (t!("git-compare-new"), &compare.to),
    ];
    for (label, rev) in sides {
        let row = Rect::new(x, y, width, 22.0);
        let end = ui_text_small(frame, app, x, row, label, p.text_dim, area) + 10.0;
        let commit = view.commits.iter().find(|c| rev_matches(c, rev));
        let is_oid = matches!(rev.len(), 40 | 64) && rev.bytes().all(|b| b.is_ascii_hexdigit());
        let end = if is_oid {
            mono(app, frame, (end, row), short(rev), p.accent, area)
        } else {
            draw_badge(
                app,
                frame,
                (end, row),
                rev,
                p.accent.alpha(p.accent.a * 0.16),
                p.accent,
                area,
            )
        };
        if let Some(commit) = commit {
            // 放不下的从末尾截短，不把半个字切在栏的右边缘。
            let clip = Rect::new(x, y, width, 22.0).intersect(area);
            let room = (x + width - end - 8.0).max(0.0);
            let subject = truncate_end(frame, &commit.subject, app.fonts.ui_small, room);
            ui_text_small(frame, app, end + 8.0, row, &subject, p.text_muted, clip);
        }
        y += 24.0;
    }
    let row = Rect::new(x, y, width, 20.0);
    let items: [(String, &str); 3] = if app.git.log.file.is_some() {
        [
            ("↑↓".into(), t!("git-log-hint-select-file")),
            (t!("common-key-enter").into(), t!("git-log-hint-open-diff")),
            ("←".into(), t!("git-log-hint-return")),
        ]
    } else {
        [
            ("→".into(), t!("git-log-hint-file-list")),
            ("Esc".into(), t!("git-log-hint-exit-compare")),
            (
                t!("git-log-hint-click").into(),
                t!("git-log-hint-view-commit"),
            ),
        ]
    };
    draw_hints(app, frame, row, &items, area);
    y += 30.0;
    frame.rect_clipped(Rect::new(x, y, width, 1.0), p.separator, 0.0, area);
    y += 10.0;
    if let Some(error) = &compare.error {
        let row = Rect::new(x, y, width, 22.0);
        ui_text_small(frame, app, x, row, error, p.warning, area);
        return y + 22.0;
    }
    match &compare.files {
        Some(files) if files.is_empty() => {
            let row = Rect::new(x, y, width, 22.0);
            ui_text_small(
                frame,
                app,
                x,
                row,
                t!("git-compare-no-differences"),
                p.text_dim,
                area,
            );
            y + 22.0
        }
        Some(files) => draw_files(app, frame, files, (x, y, width), area, (hits, long_files)),
        None if app.log_quiet() => y,
        None => {
            let row = Rect::new(x, y, width, 22.0);
            ui_text_small(
                frame,
                app,
                x,
                row,
                t!("git-compare-comparing"),
                p.text_dim,
                area,
            );
            y + 22.0
        }
    }
}

/// 绘制详情一栏最下面一行快捷键提示：如何用键盘进入文件列表，以及 `⌘` 单击另一个提交进入
/// 比较。
///
/// 不写这一行时，`⌘` 单击这一操作不容易被发现。不画在 `draw_details` 里：换选中的提交时详情
/// 的内容淡入（`DETAILS_FADE`），这一行和提交无关，跟着一起淡入会使每次点击都闪烁一次。
/// 比较两个版本时提示在比较的内容里，这里不画。
pub(super) fn draw_details_footer(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    parts: &LogParts,
) {
    let p = app.palette;
    let area = parts.details;
    if view.commits.is_empty() || view.compare.is_some() || view.selected.is_none() {
        return;
    }
    let row = Rect::new(area.x, area.bottom() - 30.0, area.w, 30.0);
    frame.rect(Rect::new(area.x, row.y, area.w, 1.0), p.separator);
    let mut items: Vec<(String, &str)> = if app.git.log.file.is_some() {
        vec![
            ("↑↓".into(), t!("git-log-hint-select-file")),
            (t!("common-key-enter").into(), t!("git-log-hint-open-diff")),
            ("←".into(), t!("git-log-hint-return")),
        ]
    } else {
        vec![
            ("→".into(), t!("git-log-hint-file-list")),
            (
                t!("git-log-hint-double-click").into(),
                t!("git-log-hint-open-diff"),
            ),
            (
                t!("git-log-hint-cmd-click", key = COMMAND),
                t!("git-log-hint-compare"),
            ),
        ]
    };
    // 顺着线、父提交跳过以后：显示如何返回。
    if !app.git.log.back.is_empty()
        && let Some(key) = keymap::shortcut(Action::GoBack)
    {
        items.insert(0, (key, t!("git-log-hint-go-back")));
    }
    let hints = Rect::new(area.x + PAD, row.y, area.w - 2.0 * PAD, row.h);
    draw_hints(app, frame, hints, &items, row);
}

/// 绘制一行快捷键提示：每一项是一个键帽加淡色的动作，项与项之间隔开 `GAP` 逻辑像素；放不下
/// 时从后往前去掉整项，不截半句。
///
/// 键和说明分开画：键在键帽里，说明在右侧，扫视时容易找到按键。
pub(super) fn draw_hints(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    items: &[(String, &str)],
    clip: Rect,
) {
    /// 键帽的高度（逻辑像素，经验值）。
    const CAP_H: f32 = 18.0;
    /// 键帽里文字左右的留白（逻辑像素，经验值）。
    const CAP_PAD: f32 = 6.0;
    /// 相邻两项之间的间隔（逻辑像素，经验值）。
    const GAP: f32 = 14.0;
    let p = app.palette;
    let size = app.fonts.ui_small;
    let widths: Vec<(f32, f32)> = items
        .iter()
        .map(|(key, label)| {
            let cap = frame.measure(FontKind::Ui, size, key) + 2.0 * CAP_PAD;
            (cap, frame.measure(FontKind::Ui, size, label))
        })
        .collect();
    let mut count = items.len();
    let total = |n: usize| -> f32 {
        widths[..n].iter().map(|(c, l)| c + 5.0 + l).sum::<f32>() + GAP * n.saturating_sub(1) as f32
    };
    while count > 0 && total(count) > row.w {
        count -= 1;
    }
    let mut x = row.x;
    for ((key, label), (cap_w, _)) in items.iter().zip(&widths).take(count) {
        let cap = Rect::new(x, (row.y + (row.h - CAP_H) / 2.0).round(), *cap_w, CAP_H);
        frame.rect_clipped(cap, p.selected_inactive, 4.0, clip);
        caps_label(
            frame,
            app,
            size,
            (cap.x + CAP_PAD, cap),
            key,
            p.text_muted,
            clip,
        );
        x = ui_text_small(frame, app, cap.right() + 5.0, row, label, p.text_dim, clip) + GAP;
    }
}

/// 新选的提交的详情还没取齐、又刚开始取（`log_quiet`）时继续显示上一个提交和它的详情。
fn held_details<'a>(
    app: &App,
    view: &'a LogView,
    selected: &Commit,
) -> Option<(&'a Commit, &'a winc_editor::git::log::Details)> {
    let current = view.details.as_ref().filter(|d| d.oid == selected.oid);
    let ready = current.is_some_and(|d| d.files.is_some() || d.error.is_some());
    view.previous
        .as_ref()
        .filter(|_| !ready && app.log_quiet())
        .and_then(|d| Some((view.commits.iter().find(|c| c.oid == d.oid)?, d)))
}

/// 详情一栏这一帧画的是哪个提交：比较两个版本时、没选中时为 `None`。
///
/// 返回值与上一帧不同时，详情内容淡入。
pub(super) fn shown_oid(app: &App, view: &LogView) -> Option<String> {
    if view.compare.is_some() {
        return None;
    }
    let selected = view.selected.and_then(|i| view.commits.get(i))?;
    let commit = held_details(app, view, selected).map_or(selected, |(c, _)| c);
    Some(commit.oid.clone())
}

/// 在详情里画一行可以选中复制的字，记进 `runs`（`App::log_text_at` 按它找字）。
///
/// 这一行有选中的部分时先铺上选区的底色（与编辑区的选区同一个颜色）。坐标是逻辑像素，
/// 返回字的右端 x。`style` 是（字号，颜色，是不是上一行折下来的，截短时完整的原文）。
fn selectable(
    app: &App,
    frame: &mut Frame<'_>,
    runs: &mut Vec<TextRun>,
    (x, row): (f32, Rect),
    text: &str,
    (size, color, soft, full): (f32, Color, bool, Option<&str>),
    clip: Rect,
) -> f32 {
    let mut edges = Vec::with_capacity(text.len());
    let (mut right, mut buf) = (x, [0u8; 4]);
    for c in text.chars() {
        right += frame.measure(FontKind::Ui, size, c.encode_utf8(&mut buf));
        edges.push(right);
    }
    if let Some((from, to)) = app.log_selected_chars(runs.len(), edges.len()) {
        let left = if from == 0 { x } else { edges[from - 1] };
        let band = Rect::new(left, row.y + 1.0, edges[to - 1] - left, row.h - 2.0);
        frame.rect_clipped(band, app.palette.selection, 2.0, clip);
    }
    runs.push(TextRun {
        row,
        x,
        text: text.to_owned(),
        edges,
        full: full.map(str::to_owned),
        soft,
    });
    if size == app.fonts.ui {
        ui_text(frame, app, x, row, text, color, clip)
    } else {
        ui_text_small(frame, app, x, row, text, color, clip)
    }
}

/// 提交 `commit` 是不是版本 `rev`：提交号相同，或者 `rev` 是指着它的分支、标签、`HEAD`。
pub(super) fn rev_matches(commit: &Commit, rev: &str) -> bool {
    commit.oid == rev
        || commit.refs.iter().any(|r| match r {
            Ref::Head(name) | Ref::Branch(name) | Ref::Remote(name) | Ref::Tag(name) => name == rev,
            Ref::Detached => rev == "HEAD",
        })
        || (rev == "HEAD" && commit.refs.iter().any(|r| matches!(r, Ref::Head(_))))
}
