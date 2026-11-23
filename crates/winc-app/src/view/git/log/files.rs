//! 详情栏里改动的文件和增删的行数。
//!
//! 详情一栏（[`draw_files`]）和差异的文件栏共用 [`draw_file_list`]：平铺时一个文件一行，否则同
//! 文件视图排成树（见 `App::log_file_rows_of`），点击文件打开差异，点击目录收起或展开，展开、
//! 收起时下面的行逐渐露出、收回。放不下的名字在鼠标停留时展开成全名。

use super::*;
use std::borrow::Cow;
use std::path::Path;
use std::rc::Rc;

use crate::app::animation::stage;
use crate::app::git::log::{LogFileRow, PanelFold, RowName};
use crate::app::{LabelCause, LabelView, TreeLabels};
use crate::theme::TREE_INDENT;
use crate::view::git::commit::{dir_parts, draw_name, file_parts, parts_width, shown_width};
use crate::view::workspace::panel::over;
use crate::view::workspace::sidebar::{FADE_FROM, NAME_FADE};
use winc_editor::git::CommitFile;

/// 文件行的底色比内容左右各宽该值（逻辑像素）。
pub(in crate::view) const ROW_PAD: f32 = 6.0;

/// 绘制改动的文件：标题行「N 个文件 +A −D」，下面是 [`draw_file_list`] 画的各行。
///
/// 从 `(x, y)` 开始画，宽 `width`，返回画完后的 y 坐标；坐标都是逻辑像素。名字展开后仍写不下、
/// 或改过名的文件记进 `long_files`：悬停时提示完整的路径。
pub(super) fn draw_files(
    app: &App,
    frame: &mut Frame<'_>,
    files: &[CommitFile],
    (x, mut y, width): (f32, f32, f32),
    area: Rect,
    (hits, long_files): (&mut Vec<(Rect, LogTarget)>, &mut Vec<usize>),
) -> f32 {
    let flat = app.log_details_flat();
    let hovered = app.pointer.hover == Hover::Log(LogTarget::FilesLayout);
    let button = draw_files_title(app, frame, files, (x, y, width, flat), area, hovered);
    hits.push((button.intersect(area), LogTarget::FilesLayout));
    y += 26.0;

    let fold = app.git.log.details_fold.as_ref();
    let rows = fold.map_or_else(|| app.log_file_rows_of(files), |f| Rc::clone(&f.rows));
    let hover = match app.pointer.hover {
        Hover::Log(LogTarget::FilesDir(n)) => Some(FileHit::Dir(n)),
        Hover::Log(LogTarget::File(i)) => Some(FileHit::File(i)),
        _ => None,
    };
    let list = FileList {
        files,
        rows: &rows,
        root: app.git.root.as_deref(),
        flat,
        fold,
        top: y,
        lane: Lane {
            names: &app.git.log.details_names,
            content: (x, width),
            clip: area,
            spill: None,
        },
    };
    // 键盘选中的文件（按 `Tab` 进入文件列表以后）铺选中的底色。
    let chosen = (app.git.log.file, app.focus == Focus::Editor);
    let out = draw_file_list(app, frame, &list, hover, chosen);
    for (rect, hit) in out.hits {
        hits.push((rect, hit.log()));
    }
    long_files.extend(out.long);
    y + out.height
}

/// 文件列表里一行的命中目标：目录（各行的下标）或文件（文件列表的下标）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::view) enum FileHit {
    Dir(usize),
    File(usize),
}

impl FileHit {
    /// 详情一栏里的命中目标。
    fn log(self) -> LogTarget {
        match self {
            FileHit::Dir(n) => LogTarget::FilesDir(n),
            FileHit::File(i) => LogTarget::File(i),
        }
    }
}

/// [`draw_file_list`] 要画的列表。
pub(in crate::view) struct FileList<'a> {
    pub files: &'a [CommitFile],
    pub rows: &'a [LogFileRow],
    /// 仓库的根目录，平铺时文件后面的目录按它算。
    pub root: Option<&'a Path>,
    pub flat: bool,
    /// 目录正在展开、收起时的一帧，`rows` 就是它的各行。
    pub fold: Option<&'a PanelFold<Rc<[LogFileRow]>>>,
    /// 第一行的上沿（逻辑像素）。
    pub top: f32,
    pub lane: Lane<'a>,
}

/// 按文件树的样子排的一列行（文件列表、日志的分支面板）共用的：名字的过渡、内容的范围、能画
/// 的范围和名字最远能伸到哪里。
pub(in crate::view) struct Lane<'a> {
    pub names: &'a TreeLabels<RowName>,
    /// 内容的左边和宽度（逻辑像素）：标题行的文字、按钮也在这一段里。
    pub content: (f32, f32),
    /// 能画的范围。
    pub clip: Rect,
    /// 鼠标停着的名字最远能伸到哪里（逻辑像素）：文件栏、分支面板可以伸到旁边的内容上面；
    /// `None` 时只到行尾。
    pub spill: Option<f32>,
}

/// [`draw_file_list`] 画完的结果。
pub(in crate::view) struct ListOut {
    /// 各行的命中范围。
    pub hits: Vec<(Rect, FileHit)>,
    /// 名字伸出列表的那一段的命中范围：要先于下面的内容判定。
    pub spills: Vec<(Rect, FileHit)>,
    /// 名字展开后仍写不下、或改过名的文件：悬停时提示完整路径。
    pub long: Vec<usize>,
    /// 各行占的高度（逻辑像素），展开、收起途中不算还没露出的。
    pub height: f32,
}

/// 画文件列表的各行。`hover` 是鼠标停着的行，`chosen` 是（选中的文件，是否有焦点）。展开、
/// 收起途中各行在移动，不画悬停、不记命中范围。
pub(in crate::view) fn draw_file_list(
    app: &App,
    frame: &mut Frame<'_>,
    list: &FileList<'_>,
    hover: Option<FileHit>,
    (chosen, focused): (Option<usize>, bool),
) -> ListOut {
    let mut out = ListOut {
        hits: Vec::new(),
        spills: Vec::new(),
        long: Vec::new(),
        height: list.rows.len() as f32 * FILE_ROW - list.fold.map_or(0.0, |f| f.hidden()),
    };
    let (x, width) = list.lane.content;
    let moving = list.fold.is_some();
    // 拖动中（选文字、拖边缘）不跟着鼠标展开名字。
    let idle = app.idle_pointer();
    for (n, entry) in list.rows.iter().enumerate() {
        let (y, clip) = fold_slot(list.fold, (list.top, n), list.lane.clip);
        let row = Rect::new(x - ROW_PAD, y, width + 2.0 * ROW_PAD, FILE_ROW);
        if clip.h <= 0.0 || row.bottom() < clip.y || row.y > clip.bottom() {
            continue;
        }
        let hit = match entry {
            LogFileRow::Dir { .. } => FileHit::Dir(n),
            LogFileRow::File { index, .. } => FileHit::File(*index),
        };
        let hovered = !moving && idle && hover == Some(hit);
        let row_out = match entry {
            LogFileRow::Dir { .. } => {
                draw_dir_row(app, frame, list, (n, entry, row), hovered, clip)
            }
            LogFileRow::File { index, depth } => {
                let file = &list.files[*index];
                let depth = (!list.flat).then_some(*depth);
                let state = (chosen == Some(*index), focused, hovered);
                draw_file_row(app, frame, list, (file, depth, row), state, clip)
            }
        };
        if row_out.long
            && let FileHit::File(i) = hit
        {
            out.long.push(i);
        }
        if moving {
            continue;
        }
        if let Some(spill) = row_out.spill {
            out.spills.push((spill, hit));
        }
        out.hits.push((row.intersect(clip), hit));
    }
    out
}

/// 一行画完的结果：伸出列表的那一段（命中范围），名字是否仍写不下或改过名。
struct RowOut {
    spill: Option<Rect>,
    long: bool,
}

/// 目录、文件正在展开、收起时第 `i` 行的上沿和能画的范围：露出的各行位置不动、只画已露出的
/// 一段，其后的行往上收还没露出的高度（与提交视图里组、目录的动画相同）。
pub(in crate::view) fn fold_slot<R>(
    fold: Option<&PanelFold<R>>,
    (top, i): (f32, usize),
    area: Rect,
) -> (f32, Rect) {
    let y = top + i as f32 * FILE_ROW;
    let Some(fold) = fold else {
        return (y, area);
    };
    let (first, last) = (fold.parent + 1, fold.parent + fold.count);
    if i > last {
        return (y - fold.hidden(), area);
    }
    if i < first {
        return (y, area);
    }
    let start = top + first as f32 * FILE_ROW;
    let shown = fold.count as f32 * FILE_ROW - fold.hidden();
    (
        y,
        Rect::new(area.x, start, area.w, shown.max(0.0)).intersect(area),
    )
}

/// 文件列表的标题行：「N 个文件 +A −D」，右端是平铺与树的切换按钮（图标是切换以后的样子；
/// `flat` 是现在是否平铺），返回按钮的位置。详情一栏和差异的文件栏共用。
pub(in crate::view) fn draw_files_title(
    app: &App,
    frame: &mut Frame<'_>,
    files: &[CommitFile],
    (x, y, width, flat): (f32, f32, f32, bool),
    area: Rect,
    hovered: bool,
) -> Rect {
    let p = app.palette;
    let (added, removed) = files
        .iter()
        .filter_map(|f| f.lines)
        .fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
    // 标题行高 22 逻辑像素（取自调参），下面留 4 逻辑像素再开始画文件。
    let row = Rect::new(x, y, width, 22.0);
    let count = t!("git-log-files-changed", count = files.len());
    let end = ui_text_small(frame, app, x, row, &count, p.text_muted, area);
    let added = format!("+{added}");
    let end = ui_text_small(frame, app, end + 10.0, row, &added, p.git_added, area);
    let removed = format!("−{removed}");
    ui_text_small(frame, app, end + 6.0, row, &removed, p.git_deleted, area);
    // 图标（宽 13）的右端与文件行增删方块的右端对齐。
    let cx = x + width + ROW_PAD - BLOCKS_INSET - 6.5;
    let button = Rect::new(cx - 12.0, y - 1.0, 24.0, 24.0);
    draw_button(frame, p, button, area, hovered, false);
    let ink = if hovered { p.text } else { p.text_muted };
    let at = (cx, button.y + button.h / 2.0);
    match flat {
        true => icons::tree_view(frame, at, ink, area),
        false => icons::list_view(frame, at, ink, area),
    }
    button
}

/// 树里第 `depth` 层箭头的中心：箭头的左端与标题的文字对齐，每深一层缩进 `TREE_INDENT`；缩进
/// 参考线也在这一列。
pub(super) fn arrow_x(x: f32, depth: usize) -> f32 {
    x + 3.0 + depth as f32 * TREE_INDENT
}

/// 图标左端到名字的距离（逻辑像素），同文件视图。
pub(super) const LABEL: f32 = 20.0;

/// 一行的名字：鼠标停着、原处放不下时写全名，宽度随过渡展开、收回。
pub(super) struct Label<'a> {
    pub key: RowName,
    /// 是目录（几级目录之间画「/」）。
    pub folder: bool,
    /// 文件名和后面淡色的一段（平铺时的目录、改名前的名字）；目录是合起来的几级。
    pub text: (&'a str, Option<&'a str>),
    pub color: Color,
    /// 名字的左边和平时能写到的右端（逻辑像素）。
    pub span: (f32, f32),
}

/// 排好的名字：各部分和本帧的过渡状态。
pub(super) struct Laid<'a> {
    parts: Vec<Cow<'a, str>>,
    view: LabelView,
    /// 名字现在画到的右端（逻辑像素）。
    pub end: f32,
    /// 名字盖过了平时的右端：行尾的增删统计让开。
    pub over: bool,
    /// 全名仍然写不下。
    pub cut: bool,
    /// 在写全名。
    wanted: bool,
}

/// 排一行的名字 `label`：平时写到右端为止（目录折掉中间几级，文件名在截断处淡出）；鼠标停着
/// （`hovered`）且放不下时写全名，最远到 `lane.spill`（没有时到行尾），再放不下才截断。
pub(super) fn lay_label<'a>(
    app: &App,
    frame: &mut Frame<'_>,
    lane: &Lane<'_>,
    (label, row, hovered): (&Label<'a>, Rect, bool),
) -> Laid<'a> {
    let (left, right) = label.span;
    let full_parts = parts_of(app, frame, label, None);
    let full = parts_width(app, frame, &full_parts, label.folder);
    let room = (right - left).max(0.0);
    let wanted = hovered && full > room;
    let limit = lane.spill.unwrap_or(row.right() - ROW_PAD);
    let room = match wanted {
        true => (full > limit - left).then_some((limit - left).max(0.0)),
        false => Some(room),
    };
    let parts = parts_of(app, frame, label, room);
    let width = parts_width(app, frame, &parts, label.folder);
    let cause = LabelCause {
        hovered: wanted,
        levels: 0,
        current: 0,
    };
    let view = lane
        .names
        .sync(&label.key, cause, &parts, shown_width(width, room));
    let end = left + view.width;
    Laid {
        parts,
        view,
        end,
        over: end > right + 0.5,
        cut: full > limit - left,
        wanted,
    }
}

/// 画排好的名字 `laid`，画到它现在的右端为止；放不下的名字同文件视图在截断处淡出成行的底色
/// `shade`（没有底色时为 `None`）。
pub(super) fn draw_laid(
    app: &App,
    frame: &mut Frame<'_>,
    (label, laid, row): (&Label<'_>, &Laid<'_>, Rect),
    shade: Option<Color>,
    clip: Rect,
) {
    let p = app.palette;
    let (left, _) = label.span;
    let width = (laid.end - clip.x).max(0.0);
    let clip = Rect::new(clip.x, clip.y, width, clip.h).intersect(clip);
    let place = (left, row, label.folder, label.color);
    draw_name(app, frame, place, (&laid.parts, &laid.view), clip);
    let content = parts_width(app, frame, &laid.parts, label.folder);
    let reach = NAME_FADE * stage(laid.view.progress, FADE_FROM, 1.0);
    if !laid.wanted && left + content > laid.end + 0.5 && reach > 0.0 {
        let solid = shade.map_or(p.editor_bg, |c| over(p.editor_bg, c));
        let edge = Rect::new(laid.end - reach, row.y, reach, row.h);
        frame.within(clip, |frame| fade(frame, edge, solid, false));
    }
}

/// 名字各部分：放不下 `room` 时目录折掉中间几级、文件后面的目录从前面按级去掉。
fn parts_of<'a>(
    app: &App,
    frame: &mut Frame<'_>,
    label: &Label<'a>,
    room: Option<f32>,
) -> Vec<Cow<'a, str>> {
    match label.folder {
        true => dir_parts(app, frame, label.text.0, room),
        false => file_parts(app, frame, label.text, room),
    }
}

/// 铺一行的底色 `color`，画上级各层的缩进参考线；名字伸出列表（右端是 `end`）时底色跟着铺到
/// 名字后面，返回伸出的那一段。
pub(super) fn row_base(
    app: &App,
    frame: &mut Frame<'_>,
    lane: &Lane<'_>,
    (row, depth, end): (Rect, usize, f32),
    color: Option<Color>,
    clip: Rect,
) -> Option<Rect> {
    let p = app.palette;
    let (x, _) = lane.content;
    let right = end + 8.0;
    let spill = (lane.spill.is_some() && right > row.right())
        .then(|| Rect::new(row.right(), row.y, right - row.right(), row.h));
    if let Some(color) = color {
        match spill {
            // 伸到差异上面的一段先和底色混成不透明的颜色，遮住下面的内容。
            Some(_) => {
                let pill = Rect::new(row.x, row.y, right - row.x, row.h);
                let color = over(p.editor_bg, color);
                frame.rect_clipped(pill, color, RADIUS_ROW, label_clip(lane, clip));
            }
            None => frame.rect_clipped(row, color, RADIUS_ROW, clip),
        }
    }
    for level in 0..depth {
        let guide = Rect::new(arrow_x(x, level).round(), row.y, 1.0, row.h);
        frame.rect_clipped(guide, p.indent_guide, 0.0, clip);
    }
    spill
}

/// 名字可以画到的范围：伸出列表时到 `lane.spill` 为止。
pub(super) fn label_clip(lane: &Lane<'_>, clip: Rect) -> Rect {
    match lane.spill {
        Some(spill) => Rect::new(clip.x, clip.y, (spill - clip.x).max(0.0), clip.h),
        None => clip,
    }
}

/// 画第 `n` 行的目录 `entry`：箭头、文件夹图标和名字（合起来的几级），同文件视图。
fn draw_dir_row(
    app: &App,
    frame: &mut Frame<'_>,
    list: &FileList<'_>,
    (n, entry, row): (usize, &LogFileRow, Rect),
    hovered: bool,
    clip: Rect,
) -> RowOut {
    let LogFileRow::Dir {
        dir,
        label,
        depth,
        open,
    } = entry
    else {
        return RowOut {
            spill: None,
            long: false,
        };
    };
    let p = app.palette;
    let (x, _) = list.lane.content;
    let cy = row.y + row.h / 2.0;
    let arrow = arrow_x(x, *depth);
    let icon = arrow + 10.0;
    let left = icon + LABEL;
    // 底色要铺到名字后面：先排好名字，再画底色和名字。
    let text = Label {
        key: RowName::Dir(dir.clone()),
        folder: true,
        text: (label, None),
        color: p.text,
        span: (left, row.right() - ROW_PAD),
    };
    let laid = lay_label(app, frame, &list.lane, (&text, row, hovered));
    let color = row_color(p, false, false, hovered);
    let spill = row_base(app, frame, &list.lane, (row, *depth, laid.end), color, clip);
    let turned = match list.fold {
        Some(f) if f.parent == n => f.reveal(),
        _ => f32::from(u8::from(*open)),
    };
    let angle = turned * std::f32::consts::FRAC_PI_2;
    icons::chevron_turned(frame, (arrow, cy), 6.0, angle, p.text_dim, clip);
    icons::folder(frame, (icon, (cy - 6.0).round()), *open, p.folder, clip);
    draw_laid(
        app,
        frame,
        (&text, &laid, row),
        color,
        label_clip(&list.lane, clip),
    );
    RowOut {
        spill: spill.filter(|_| hovered),
        long: false,
    }
}

/// 画文件 `file` 一行：图标、按变化着色的名字、淡色的目录（平铺时）或改名前的名字、右端的增删
/// 统计；名字展开盖过统计时统计让开。`depth` 是在树里的层数，平铺时为 `None`；`state` 是
/// （选中、有焦点、悬停）。
fn draw_file_row(
    app: &App,
    frame: &mut Frame<'_>,
    list: &FileList<'_>,
    (file, depth, row): (&CommitFile, Option<usize>, Rect),
    (chosen, focused, hovered): (bool, bool, bool),
    clip: Rect,
) -> RowOut {
    let p = app.palette;
    let (x, _) = list.lane.content;
    let cy = row.y + row.h / 2.0;
    let name = winc_editor::display_name(&file.path).into_owned();
    let dir = file
        .path
        .parent()
        .and_then(|d| list.root.and_then(|r| d.strip_prefix(r).ok()))
        .map(|d| d.to_string_lossy().into_owned())
        .filter(|d| !d.is_empty());
    // 树里所在的目录已经是上级的目录行，不再写；改过名时写原来的名字。
    let after = match (&file.from, file.change) {
        (Some(from), Change::Renamed) => Some(format!("← {}", winc_editor::display_name(from))),
        _ if depth.is_some() => None,
        _ => dir,
    };
    // 树里的文件和目录的图标在同一列，名字从同一处开始；平铺时图标从内容左边开始。
    let (icon, left) = match depth {
        Some(depth) => {
            let icon = arrow_x(x, depth) + 10.0;
            (icon + 1.5, icon + LABEL)
        }
        None => (x, x + icons::FILE_WIDTH + 6.0),
    };
    // 增删统计占行右端 96 逻辑像素（取自调参），名字平时写到它左边 12 逻辑像素为止。
    let stat_x = row.right() - STAT_W;
    let text = Label {
        key: RowName::File(file.path.clone()),
        folder: false,
        text: (&name, after.as_deref()),
        color: git_color(p, file.change),
        span: (left, stat_x - 12.0),
    };
    let laid = lay_label(app, frame, &list.lane, (&text, row, hovered));
    let color = row_color(p, chosen, focused, hovered);
    let place = (row, depth.unwrap_or(0), laid.end);
    let spill = row_base(app, frame, &list.lane, place, color, clip);
    let label_clip = label_clip(&list.lane, clip);
    draw_file_icon(frame, p, (icon, cy), &name, label_clip);
    draw_laid(app, frame, (&text, &laid, row), color, label_clip);
    if !laid.over {
        draw_stat(app, frame, file.lines, (stat_x, row), clip);
    }
    RowOut {
        spill: spill.filter(|_| hovered),
        long: laid.cut || file.from.is_some(),
    }
}

/// 文件行右端增删统计的宽度（逻辑像素，取自调参）。
const STAT_W: f32 = 96.0;

/// 增删方块的右端离行底色右边的距离（逻辑像素）。
const BLOCKS_INSET: f32 = 7.0;

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
    let blocks_x = row.right() - BLOCKS_INSET - 5.0 * 7.0 + 1.0;
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
