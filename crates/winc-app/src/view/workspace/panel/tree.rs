//! 以树形显示的搜索结果：各行按所在目录缩进，每级目录一条缩进参考线，目录一行是箭头、文件夹
//! 图标、名字和右端的匹配总数；排法同文件视图。

use super::*;

/// 结果行 `kind` 的内容向右移的距离（逻辑像素）：每级目录 [`TREE_INDENT`]，匹配和所在的文件
/// 同一列；以列表显示时为 0。
pub(super) fn shift(app: &App, kind: ResultRow) -> f32 {
    let g = &app.panels.global;
    let depth = match kind {
        ResultRow::Match(f, _) => g.row_depth(ResultRow::File(f)),
        _ => g.row_depth(kind),
    };
    depth as f32 * TREE_INDENT
}

/// 以树形显示时画结果行 `kind` 的缩进参考线：每个上一级（目录，匹配还有所在的文件）一条，在
/// 它的箭头正下方，相邻行的参考线连成竖线。
pub(super) fn draw_guides(
    app: &App,
    frame: &mut Frame<'_>,
    kind: ResultRow,
    row: Rect,
    clip: Rect,
) {
    let g = &app.panels.global;
    if g.tree.is_none() {
        return;
    }
    for level in 0..g.row_depth(kind) {
        let x = (row.x + PANEL_CHEVRON + level as f32 * TREE_INDENT).round();
        let guide = Rect::new(x, row.y, 1.0, row.h);
        frame.rect_clipped(guide, app.palette.indent_guide, 0.0, clip);
    }
}

/// 绘制第 `d` 个目录的结果行（`row` 已按层数右移）：箭头、文件夹图标、名字（合成一行的几级），
/// 右端是其中匹配的总数（悬停时换成按钮），与文件行的计数同样排法。
pub(super) fn draw_dir_row(
    app: &App,
    frame: &mut Frame<'_>,
    row: Rect,
    clip: Rect,
    d: usize,
    end: &RowEnd,
) {
    let p = app.palette;
    let g = &app.panels.global;
    let Some(dir) = g.dir(d) else {
        return;
    };
    let collapsed = g.collapsed.contains(&dir.path);
    let cy = icon_center_y(app, row, app.fonts.ui);
    // 折叠、展开途中，箭头按里面的行露出的比例旋转，文件夹在展开过半时换成打开的样子。
    let turned = match &g.motion {
        Some(m) if m.rows.get(m.parent) == Some(&ResultRow::Dir(d)) => m.turned(),
        _ => f32::from(u8::from(!collapsed)),
    };
    let chevron = (row.x + PANEL_CHEVRON, cy);
    icons::chevron_turned(frame, chevron, 6.0, turned * FRAC_PI_2, p.text_dim, clip);
    // 文件夹图标比文件图标宽：左边往外让 1.5，名字与图标的间距和文件行相近，同文件视图。
    let top = (cy - 6.0).round();
    icons::folder(
        frame,
        (row.x + PANEL_LEFT - 1.5, top),
        turned > 0.5,
        p.folder,
        clip,
    );

    let right = draw_count(app, frame, dir.matches, end, clip) - end.pad;
    let x = row.x + PANEL_TEXT;
    let label = ellipsize(frame, &dir.label, app.fonts.ui, (right - x).max(0.0));
    ui_text(frame, app, x, row, &label, p.text, clip);
}

/// 在行的右端画计数 `count`：与按钮同高的胶囊，右边缘与按钮对齐；按钮浮出来时不画。返回胶囊的
/// 左边缘，左边的文字到此为止（悬停时也不变）。
pub(super) fn draw_count(
    app: &App,
    frame: &mut Frame<'_>,
    count: usize,
    end: &RowEnd,
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let count = count.to_string();
    let size = app.fonts.ui_small;
    let count_w = frame.measure(FontKind::Ui, size, &count);
    let slot = end.slot;
    let badge_w = (count_w + 2.0 * end.pad).max(slot.w);
    let badge = Rect::new(slot.right() - badge_w, slot.y, badge_w, slot.h);
    if !end.covered {
        frame.rect_clipped(badge, p.hover, slot.h / 2.0, clip);
        let x = badge.right() - end.pad - count_w;
        caps_label(frame, app, size, (x, badge), &count, p.text_muted, clip);
    }
    badge.x
}
