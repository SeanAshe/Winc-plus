//! 差异两侧的逐行追溯一栏（见 `App::toggle_diff_blame`）：画法同编辑区的一栏，色条、作者和时间，
//! 提交说明在悬停提示中。

use super::*;
use crate::app::git::blame::BlameKey;
use crate::view::editor::{blame_ages, draw_blame_cell};

/// 绘制开着逐行追溯的一侧的追溯一栏（单栏时两侧共用一栏，每行取所在一侧的结果，未改动的行
/// 右侧没开时取左侧），记下各格：悬停提示提交，点击在日志中选中它。`key` 给出每一侧追溯结果的
/// 键，`drawn` 是正文画出的显示行。
pub(super) fn draw(
    app: &App,
    frame: &mut Frame<'_>,
    (key, view): (impl Fn(usize) -> BlameKey, &DiffView),
    parts: &DiffParts,
    drawn: &[DrawnRow],
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    if !view.blamed.contains(&true) {
        return;
    }
    let lh = app.fonts.line_height;
    let blames = [LEFT, RIGHT].map(|side| {
        let on = view.blamed[side];
        on.then(|| app.blame_of_key(key(side))).flatten()
    });
    let ages = blames.each_ref().map(|b| b.as_deref().map(blame_ages));
    let hovered = match app.pointer.hover {
        Hover::Diff(DiffTarget::Blame(side, line)) => Some((side, line)),
        _ => None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let unified = view.is_unified();
    for &DrawnRow {
        side,
        row,
        y,
        old_line,
    } in drawn
    {
        let SideRow::Line { line, part, .. } = view.rows[side][row] else {
            continue;
        };
        // 单栏中未改动的行在右侧没开追溯时取左侧的结果。
        let (owner, owned) = match (unified && !view.blamed[side], old_line) {
            (false, _) => (side, line),
            (true, Some(old)) => (LEFT, old),
            (true, None) => continue,
        };
        let (Some(blame), Some(ages)) = (&blames[owner], ages[owner]) else {
            continue;
        };
        let column = parts.panes[side].blame;
        let hover = hovered
            .filter(|&(s, _)| s == owner)
            .and_then(|(_, l)| blame.lines.get(l).copied());
        // 上方紧挨着显示的是前一行时，同一次提交只在第一行写作者和时间。
        let above = row.checked_sub(1).and_then(|r| match view.rows[side][r] {
            SideRow::Line { line: l, .. } if l + 1 == line => owned.checked_sub(1),
            _ => None,
        });
        let label = (part == 0).then_some((y, above));
        let cx = (&**blame, ages, hover, now);
        draw_blame_cell(app, frame, cx, owned, column, (y, lh, label));
        if part == 0 {
            let cell = Rect::new(column.x, y, column.w, lh).intersect(parts.body);
            hits.push((cell, DiffTarget::Blame(owner, owned)));
        }
    }
}
