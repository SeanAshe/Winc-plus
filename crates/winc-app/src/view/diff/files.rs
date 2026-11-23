//! 从日志打开的差异左侧的文件栏：标题行和文件列表的画法与日志详情一栏相同。

use super::*;
use crate::app::git::log::{FILES_TITLE, LogFileRow};
use crate::view::git::log::files::{FileHit, FileList, Lane, draw_file_list, draw_files_title};
use std::rc::Rc;

/// 文件栏里一行的高度（逻辑像素），与日志详情一栏相同。
const ROW: f32 = crate::app::git::log::LOG_FILE_ROW;

/// 绘制文件栏：标题行（文件数、增删行数、平铺与树的切换），下面一个文件一行，正在看的文件
/// 铺选中的底色；右边缘可以拖动。没有文件栏时不画。
pub(super) fn draw(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &DiffParts,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let area = parts.files;
    if area.w <= 0.0 || app.log_diff_tree_source().is_none() {
        return;
    }
    // 文件列表在标题行下面滚动；先按行数算好最大滚动距离。
    let title_y = area.y + 10.0;
    let list_top = title_y + FILES_TITLE;
    let list = Rect::new(
        area.x,
        list_top,
        area.w,
        (area.bottom() - list_top).max(0.0),
    );
    let rows = match &app.git.log.panel_fold {
        Some(fold) => Rc::clone(&fold.rows),
        None => app.log_files_rows(),
    };
    let panel = &mut app.git.log.files;
    panel.max = (rows.len() as f32 * ROW + 8.0 - list.h).max(0.0);
    panel.scroll = panel.scroll.min(panel.max);
    let scroll = panel.scroll;
    let animate = app.animate_log_names();
    app.git.log.panel_names.begin_frame(animate);
    // 展开、收起途中只露出右边一段：各行按展开时的宽度排，跟着右边缘从左边滑进、滑出。
    let slid = Rect::new(area.right() - parts.files_w, area.y, parts.files_w, area.h);
    let place = (area, slid, title_y, list);
    let long = draw_list(app, frame, place, (&rows, scroll), hits);
    app.git.log.panel_names.end_frame();
    app.git.log.files.long = long;
    // 右边缘：拖动改变宽度，命中范围比分隔线宽一些，容易抓住；展开、收起途中不能拖动。
    if area.w >= parts.files_w {
        let edge = Rect::new(area.right() - 3.0, area.y, 6.0, area.h);
        hits.push((edge, DiffTarget::FilesEdge));
    }
}

/// 画文件栏的底色、标题行和各行，返回目录截短或改过名的文件：悬停时提示完整路径。
fn draw_list(
    app: &App,
    frame: &mut Frame<'_>,
    (area, slid, title_y, list): (Rect, Rect, f32, Rect),
    (rows, scroll): (&[LogFileRow], f32),
    hits: &mut Vec<(Rect, DiffTarget)>,
) -> Vec<usize> {
    let mut long = Vec::new();
    let Some((files, root, selected)) = app.log_diff_tree_source() else {
        return long;
    };
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    // 右边缘可以左右拖动，悬停或拖动时改为较粗的强调色线条（与日志里的分隔线相同）。
    if app.log_files_resizing() || app.pointer.hover == Hover::Diff(DiffTarget::FilesEdge) {
        frame.rect(Rect::new(area.right() - 1.5, area.y, 2.0, area.h), p.accent);
    } else {
        frame.rect(
            Rect::new(area.right() - 1.0, area.y, 1.0, area.h),
            p.separator,
        );
    }
    let x = slid.x + 12.0;
    let width = slid.w - 24.0;
    let hovered = app.pointer.hover == Hover::Diff(DiffTarget::FilesLayout);
    let flat = app.diff_files_flat();
    let button = draw_files_title(app, frame, files, (x, title_y, width, flat), area, hovered);
    hits.push((button.intersect(area), DiffTarget::FilesLayout));

    let hover = match app.pointer.hover {
        Hover::Diff(DiffTarget::FilesDir(n)) => Some(FileHit::Dir(n)),
        Hover::Diff(DiffTarget::File(i)) => Some(FileHit::File(i)),
        _ => None,
    };
    let list = FileList {
        files,
        rows,
        root: Some(root),
        flat,
        fold: app.git.log.panel_fold.as_ref(),
        top: list.y - scroll,
        lane: Lane {
            names: &app.git.log.panel_names,
            content: (x, width),
            clip: list,
            // 放不下的名字可以伸到差异上面，右边留 8 逻辑像素。
            spill: Some(app.layout.editor.right() - 8.0),
        },
    };
    let out = draw_file_list(app, frame, &list, hover, (Some(selected), false));
    let target = |hit| match hit {
        FileHit::Dir(n) => DiffTarget::FilesDir(n),
        FileHit::File(i) => DiffTarget::File(i),
    };
    hits.extend(out.hits.into_iter().map(|(rect, hit)| (rect, target(hit))));
    // 伸到差异上面的名字遮住了那里的按钮，先判定。
    for (rect, hit) in out.spills {
        hits.insert(0, (rect, target(hit)));
    }
    long.extend(out.long);
    long
}
