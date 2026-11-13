//! 文件行中目录与文件名的排版和绘制：过长时的省略，以及展开、收起时的过渡。

use super::*;

/// 改动行 `row` 名字过渡状态的键（见 [`CommitName`]）；组标题行没有名字，不会用到。
pub(super) fn name_key(status: &Status, row: &CommitRow) -> CommitName {
    match row {
        CommitRow::Dir { group, dir, .. } => CommitName::Dir(*group, dir.clone()),
        CommitRow::File { index, .. } => CommitName::File(
            status
                .entries
                .get(*index)
                .map(|e| e.path.clone())
                .unwrap_or_default(),
        ),
        CommitRow::Group(group) => CommitName::Dir(*group, Default::default()),
    }
}

/// 改动行全名的各部分（见 [`dir_parts`]、[`file_parts`]），不截短。
pub(super) fn full_parts(text: &RowText) -> Vec<Cow<'_, str>> {
    if text.folder {
        text.name.split('/').map(Cow::Borrowed).collect()
    } else {
        let mut parts = vec![Cow::Borrowed(text.name.as_str())];
        parts.extend(text.dir.as_deref().map(Cow::Borrowed));
        parts
    }
}

/// 第 `row` 行的上沿（逻辑像素）：固定在顶部的行按固定的位置，其余按滚动的偏移。
pub(super) fn row_top(app: &App, rows: &CommitRows, row: usize, list: Rect) -> f32 {
    match app.commit_sticky(rows).iter().find(|s| s.row == row) {
        Some(s) => s.rect.y,
        None => list.y + row as f32 * COMMIT_ROW - app.git.commit.scroll,
    }
}

/// 返回并起来的几级目录（`Sources/DEEIXPET`）画出来的宽度（逻辑像素），见 [`draw_dir_parts`]。
pub(super) fn dir_label_width(app: &App, frame: &mut Frame<'_>, label: &str) -> f32 {
    let size = app.fonts.ui;
    let parts = label.split('/').count() as f32;
    let names: f32 = label
        .split('/')
        .map(|part| frame.measure(FontKind::Ui, size, part))
        .sum();
    let slash = frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP;
    names + (parts - 1.0) * slash
}

/// 目录行名字的各部分：并起来的几级目录各是一部分；放不下 `room`（逻辑像素）时折叠中间的各级
/// （见 [`fold_chain`]），折掉的几级换成一个 `CHAIN_FOLD`，目录名不会被截成半个。
pub(super) fn dir_parts<'a>(
    app: &App,
    frame: &mut Frame<'_>,
    label: &'a str,
    room: Option<f32>,
) -> Vec<Cow<'a, str>> {
    let names: Vec<&str> = label.split('/').collect();
    let Some(room) = room else {
        return names.into_iter().map(Cow::Borrowed).collect();
    };
    let size = app.fonts.ui;
    let widths: Vec<f32> = (names.iter())
        .map(|name| frame.measure(FontKind::Ui, size, name))
        .collect();
    let step = frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP;
    let fold = frame.measure(FontKind::Ui, size, CHAIN_FOLD);
    let (parts, _) = fold_chain(&widths, step, fold, names.len() - 1, room);
    (parts.into_iter())
        .map(|part| Cow::Borrowed(part.map_or(CHAIN_FOLD, |i| names[i])))
        .collect()
}

/// 画目录行名字的各部分 `parts`（见 [`dir_parts`]），之间是「/」，样式与文件树中合并显示的目录
/// 相同。第 k 部分的不透明度是 `alpha(k)`，它前面的「/」取两边较淡的一个，为 0 的不画；返回
/// 名字末尾的 x 坐标（逻辑像素）。
pub(super) fn draw_dir_parts<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band): (f32, Rect),
    parts: &[S],
    alpha: impl Fn(usize) -> f32,
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let size = app.fonts.ui;
    let separator = p.text.alpha(p.text.a * CHAIN_SEPARATOR);
    let slash = frame.measure(FontKind::Ui, size, "/");
    let mut x = x;
    for (k, part) in parts.iter().enumerate() {
        let part = part.as_ref();
        if k > 0 {
            let a = alpha(k - 1).min(alpha(k));
            if a > 0.0 {
                let color = separator.alpha(separator.a * a);
                ui_text(frame, app, x + CHAIN_GAP, band, "/", color, clip);
            }
            x += slash + 2.0 * CHAIN_GAP;
        }
        let color = if part == CHAIN_FOLD {
            separator
        } else {
            p.text
        };
        let a = alpha(k);
        x = if a > 0.0 {
            ui_text(frame, app, x, band, part, color.alpha(color.a * a), clip)
        } else {
            x + frame.measure(FontKind::Ui, size, part)
        };
    }
    x
}

/// 文件行名字的各部分：文件名，平铺时再加所在目录 `dir`。
///
/// 和文件树一样，文件名不截短：放不下 `room`（逻辑像素）时画到 `room` 为止，在截断处淡出。目录
/// 只用文件名剩下的地方，从前面按级去掉，留下离文件最近的几级（`…/app/git`）：分辨同名文件靠的
/// 就是它们。
pub(super) fn file_parts<'a>(
    app: &App,
    frame: &mut Frame<'_>,
    (name, dir): (&'a str, Option<&'a str>),
    room: Option<f32>,
) -> Vec<Cow<'a, str>> {
    let mut parts = vec![Cow::Borrowed(name)];
    if let Some(dir) = dir {
        let dir = match room {
            Some(room) => {
                let used = frame.measure(FontKind::Ui, app.fonts.ui, name) + DIR_GAP;
                nearest_dirs(frame, dir, app.fonts.ui_small, room - used)
            }
            None => Cow::Borrowed(dir),
        };
        parts.push(dir);
    }
    parts
}

/// 把目录 `dir` 从前面按级去掉，直到放得进 `room`（逻辑像素）：`crates/app/git` 依次试
/// `…/app/git`、`…/git`。只剩一级还放不下时就是它，由绘制时截断淡出。
pub(super) fn nearest_dirs<'a>(
    frame: &mut Frame<'_>,
    dir: &'a str,
    size: f32,
    room: f32,
) -> Cow<'a, str> {
    if frame.measure(FontKind::Ui, size, dir) <= room {
        return Cow::Borrowed(dir);
    }
    let mut rest = dir;
    while let Some((_, tail)) = rest.split_once('/') {
        rest = tail;
        let shown = format!("{CHAIN_FOLD}/{rest}");
        if frame.measure(FontKind::Ui, size, &shown) <= room || !rest.contains('/') {
            return Cow::Owned(shown);
        }
    }
    Cow::Borrowed(dir)
}

/// 画文件行名字的各部分 `parts`（见 [`file_parts`]）：文件名用 `color`，后面的目录用小一号的
/// 淡色；不透明度和 [`draw_dir_parts`] 一样由 `alpha` 给出。返回名字末尾的 x 坐标（逻辑像素）。
pub(super) fn draw_file_parts<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band): (f32, Rect),
    parts: &[S],
    (color, alpha): (Color, impl Fn(usize) -> f32),
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let mut x = x;
    for (k, part) in parts.iter().enumerate() {
        let (size, ink, start) = if k == 0 {
            (app.fonts.ui, color, x)
        } else {
            (app.fonts.ui_small, p.text_dim, x + DIR_GAP)
        };
        let a = alpha(k);
        let ink = ink.alpha(ink.a * a);
        x = match (k, a > 0.0) {
            (0, true) => ui_text(frame, app, start, band, part.as_ref(), ink, clip),
            (_, true) => ui_text_small(frame, app, start, band, part.as_ref(), ink, clip),
            (_, false) => start + frame.measure(FontKind::Ui, size, part.as_ref()),
        };
    }
    x
}

/// 改动行名字各部分 `parts` 画出来的宽度（逻辑像素）；`folder` 为真时是目录行。
pub(super) fn parts_width<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &[S],
    folder: bool,
) -> f32 {
    let size = app.fonts.ui;
    let measure = |frame: &mut Frame<'_>, k: usize, part: &str| {
        let size = if k == 0 || folder {
            size
        } else {
            app.fonts.ui_small
        };
        frame.measure(FontKind::Ui, size, part)
    };
    let gap = if folder {
        frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP
    } else {
        DIR_GAP
    };
    let mut width = 0.0;
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            width += gap;
        }
        width += measure(frame, k, part.as_ref());
    }
    width
}

/// 名字过渡中旧内容和新内容的不透明度：随进度此消彼长，两者之和始终为 1。
///
/// 改动行截短的目录与全名的写法不同（`crates/winc-app/…` 与 `…/app/git`），先淡出旧内容、再淡入
/// 新内容（`LabelView` 的 `old_alpha`、`alpha`）时，中间有一段两者都接近透明，文字会短暂消失；
/// 交叉淡化使任意时刻都有可见的文字。
pub(super) fn crossfade(view: &LabelView) -> (f32, f32) {
    (1.0 - view.progress, view.progress)
}

/// 画改动行名字过渡中的一帧：旧内容淡出、新内容 `parts` 同时淡入（见 [`crossfade`]），相同的
/// 前缀只由新内容画一次。返回新内容末尾的 x 坐标（逻辑像素）。
pub(super) fn draw_name<S: AsRef<str>>(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band, folder, color): (f32, Rect, bool, Color),
    (parts, view): (&[S], &LabelView),
    clip: Rect,
) -> f32 {
    let common = view.common;
    let (old_alpha, new_alpha) = crossfade(view);
    let old = |k: usize| if k < common { 0.0 } else { old_alpha };
    let new = |k: usize| if k < common { 1.0 } else { new_alpha };
    if folder {
        draw_dir_parts(app, frame, (x, band), &view.old, old, clip);
        draw_dir_parts(app, frame, (x, band), parts, new, clip)
    } else {
        draw_file_parts(app, frame, (x, band), &view.old, (color, old), clip);
        draw_file_parts(app, frame, (x, band), parts, (color, new), clip)
    }
}
