//! 在文件里解决合并冲突的绘制：冲突各部分的底色、标记行上的标题和按钮、右上角的卡片。
//!
//! 当前这一侧铺蓝、对方铺绿、共同祖先铺灰，和差异视图用同一组柔和色。标记行不显示原来的
//! `<<<<<<<`，换成一条标题：`<<<<<<<` 那条写「当前 · HEAD」，后面是「采用当前」「采用对方」
//! 「两者都要」「对比」，右端是第几处；`|||||||` 写「共同祖先」，`=======` 写「对方 · 分支名」，
//! `>>>>>>>` 只是一道收尾的细线。

use winc_editor::git::merge::{AlignSegment, Conflict, Pick};
use winc_i18n::t;

use crate::app::git::merge::{MergePanes, MergeTarget, PANE_HEADER};
use crate::theme::DiffShade;
use crate::view::diff::draw_ribbon;
use crate::view::*;

/// 标记行上的按钮，从左到右；冲突序号用 `usize::MAX` 占位，绘制时换成当前冲突的序号。
const ACTIONS: [MergeTarget; 4] = [
    MergeTarget::Pick(usize::MAX, Pick::Ours),
    MergeTarget::Pick(usize::MAX, Pick::Theirs),
    MergeTarget::Pick(usize::MAX, Pick::Both),
    MergeTarget::Compare(usize::MAX),
];

/// 把 `ACTIONS` 里的占位换成第 `i` 段冲突。
fn for_conflict(target: MergeTarget, i: usize) -> MergeTarget {
    match target {
        MergeTarget::Pick(_, pick) => MergeTarget::Pick(i, pick),
        MergeTarget::Compare(_) => MergeTarget::Compare(i),
        other => other,
    }
}

fn label(target: MergeTarget) -> &'static str {
    match target {
        MergeTarget::Pick(_, pick) | MergeTarget::All(pick) => pick.label(),
        MergeTarget::Compare(_) => t!("git-merge-compare"),
        MergeTarget::Previous => t!("git-merge-previous"),
        MergeTarget::Next => t!("git-merge-next"),
        MergeTarget::MarkResolved => t!("git-merge-mark-resolved"),
        MergeTarget::ThreePane => t!("git-merge-three-pane"),
        MergeTarget::EditConflict(_) => t!("git-merge-edit-conflict"),
    }
}

/// 冲突里一行（不是标记行）的底色；`line` 不在冲突的内容里时为 `None`。
pub(in crate::view) fn tint(p: &Palette, conflict: &Conflict, line: usize) -> Option<Color> {
    use crate::app::git::merge::{Part, part_of};
    match part_of(conflict, line)? {
        Part::Ours => Some(p.diff_modified.line),
        Part::Theirs => Some(p.diff_added.line),
        Part::Base => Some(p.diff_deleted.line),
        Part::Marker => None,
    }
}

/// 绘制第 `i` 段冲突（共 `count` 段）的标记行 `line`，画在 `band`（文字区里这一行，逻辑像素）上。
#[allow(clippy::too_many_arguments)]
pub(in crate::view) fn draw_marker(
    app: &App,
    frame: &mut Frame<'_>,
    i: usize,
    count: usize,
    conflict: &Conflict,
    line: usize,
    band: Rect,
    hits: &mut Vec<(Rect, MergeTarget)>,
) {
    if app.merge_panes_placed() && line == conflict.start && conflict_folded(app, line) {
        return draw_folded_conflict(app, frame, i, count, band, hits);
    }
    let p = app.palette;
    let side = |who: &str, name: &str| {
        if name.is_empty() {
            who.to_owned()
        } else {
            format!("{who} · {name}")
        }
    };
    let (color, title) = if line == conflict.start {
        (
            p.diff_modified.word,
            side(t!("git-merge-side-current"), &conflict.ours_label),
        )
    } else if Some(line) == conflict.base {
        (p.diff_deleted.word, t!("git-merge-side-base").to_owned())
    } else if line == conflict.middle {
        (
            p.diff_added.word,
            side(t!("git-merge-side-incoming"), &conflict.theirs_label),
        )
    } else {
        // `>>>>>>>`：盖住标记，只留一道收尾的细线。
        frame.rect_clipped(band, p.editor_bg, 0.0, band);
        let rule = Rect::new(band.x, (band.y + band.h / 2.0 - 1.0).round(), band.w, 2.0);
        frame.rect_clipped(rule, p.diff_added.word, 0.0, band);
        return;
    };
    frame.rect_clipped(band, color, 0.0, band);
    let mut x = ui_text_small(frame, app, band.x + TEXT_PAD, band, &title, p.text, band) + 14.0;
    if line != conflict.start {
        return;
    }

    // 「采用当前」「采用对方」「两者都要」「对比」，只有文字，悬停时铺底色。
    let h = (band.h - 4.0).max(12.0);
    for action in ACTIONS {
        let target = for_conflict(action, i);
        let text = label(target);
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + 12.0;
        let rect = Rect::new(x, band.y + (band.h - h) / 2.0, w, h);
        if app.hover == Hover::Merge(target) {
            frame.rect_clipped(rect, p.hover, RADIUS_ROW, band);
        }
        ui_text_small(frame, app, rect.x + 6.0, band, text, p.accent, band);
        hits.push((rect.intersect(band), target));
        x = rect.right() + 2.0;
    }

    // 右端：第几处。
    let counter = format!("{} / {count}", i + 1);
    let w = frame.measure(FontKind::Ui, app.fonts.ui_small, &counter);
    let right = band.right() - 14.0 - w;
    if right > x + 8.0 {
        ui_text_small(frame, app, right, band, &counter, p.text_muted, band);
    }
}

/// 绘制右上角的冲突卡片：还剩几处冲突、上一处下一处、全部采用一边；冲突都去掉了但还没标记为
/// 已解决时是「冲突都已解决」和「标记为已解决」。
///
/// 返回卡片的矩形（窗口坐标，逻辑像素），不需要卡片时为 `None`。
pub(in crate::view) fn draw_merge_card(
    app: &mut App,
    frame: &mut Frame<'_>,
    top: f32,
    hits: &mut Vec<(Rect, MergeTarget)>,
) -> Option<Rect> {
    let index = app.editor.active?;
    let count = app.conflicts(index).len();
    // 三栏合并的操作都在标题行和底部操作栏上。
    if app.merge_panes_placed() || (count == 0 && !app.active_unmerged()) {
        return None;
    }
    let p = app.palette;
    let editor = app.layout.editor;
    let font = app.fonts.ui;
    let (message, buttons): (String, Vec<MergeTarget>) = if count > 0 {
        (
            t!("git-merge-card-conflicts", count = count),
            vec![
                MergeTarget::Previous,
                MergeTarget::Next,
                MergeTarget::ThreePane,
                MergeTarget::All(Pick::Ours),
                MergeTarget::All(Pick::Theirs),
            ],
        )
    } else {
        (
            t!("git-merge-card-resolved").to_owned(),
            vec![MergeTarget::MarkResolved],
        )
    };
    let widths: Vec<f32> = buttons
        .iter()
        .map(|&b| match b {
            MergeTarget::Previous | MergeTarget::Next => BUTTON_SMALL,
            b => frame.measure(FontKind::Ui, font, all_label(b)) + 16.0,
        })
        .collect();
    let buttons_w: f32 = widths.iter().sum::<f32>() + 4.0 * (widths.len() - 1) as f32;
    let text_w = frame.measure(FontKind::Ui, font, &message);
    let width = 34.0 + text_w + 16.0 + buttons_w + 8.0;
    // 卡片高 36 逻辑像素（经验值）：比一行文字高，按钮上下各留一点空隙。
    let card = Rect::new(editor.right() - 8.0 - width, top, width, 36.0);

    draw_card(frame, p, card, RADIUS_BAR, p.popup_bg);
    let icon = (card.x + 16.0, icon_center_y(app, card, font));
    if count > 0 {
        icons::warning(frame, icon, p.warning, card);
    } else {
        icons::checkbox(
            frame,
            icon,
            Some(true),
            p.git_added,
            p.git_added,
            p.popup_bg,
            card,
        );
    }
    ui_text(frame, app, card.x + 34.0, card, &message, p.text, card);

    let mut x = card.right() - 8.0 - buttons_w;
    let y = card.y + (card.h - BUTTON_SMALL) / 2.0;
    for (&target, w) in buttons.iter().zip(widths) {
        let rect = Rect::new(x, y, w, BUTTON_SMALL);
        draw_button(
            frame,
            p,
            rect,
            card,
            app.hover == Hover::Merge(target),
            false,
        );
        match target {
            MergeTarget::Previous => {
                icons::chevron(
                    frame,
                    center(rect),
                    8.0,
                    icons::Direction::Up,
                    p.text_muted,
                    card,
                );
            }
            MergeTarget::Next => {
                icons::chevron(
                    frame,
                    center(rect),
                    8.0,
                    icons::Direction::Down,
                    p.text_muted,
                    card,
                );
            }
            _ => {
                ui_text(
                    frame,
                    app,
                    rect.x + 8.0,
                    rect,
                    all_label(target),
                    p.accent,
                    card,
                );
            }
        }
        hits.push((rect, target));
        x += w + 4.0;
    }
    Some(card)
}

fn all_label(target: MergeTarget) -> &'static str {
    match target {
        MergeTarget::All(Pick::Ours) => t!("git-merge-accept-all-current"),
        MergeTarget::All(Pick::Theirs) => t!("git-merge-accept-all-incoming"),
        other => label(other),
    }
}

/// 三栏合并两侧行号与文字的间距（逻辑像素）。
const PANE_PAD: f32 = 8.0;
/// 底部操作栏按钮文字两侧的留白（逻辑像素）。
const FOOTER_BUTTON_PAD: f32 = 12.0;

/// 第 `line` 行所在的冲突是否折叠着。
fn conflict_folded(app: &App, line: usize) -> bool {
    (app.editor.active()).is_some_and(|doc| doc.folds().is_collapsed(line))
}

/// 冲突使用的颜色：红色只用于还没解决的地方，与差异视图的蓝、绿、灰区分。
pub(in crate::view) fn conflict_shade(p: &Palette) -> DiffShade {
    DiffShade {
        line: p.error.alpha(0.12),
        word: p.error.alpha(0.24),
        edge: p.error.alpha(0.45),
        mark: p.error,
    }
}

/// 返回一段已自动合并的改动使用的颜色：只在结果里有的绿、只在这一侧有的灰、两边都有的蓝。
pub(in crate::view) fn merged_shade(p: &Palette, segment: &AlignSegment) -> DiffShade {
    if segment.side.is_empty() {
        p.diff_added
    } else if segment.file.is_empty() {
        p.diff_deleted
    } else {
        p.diff_modified
    }
}

/// 绘制结果一栏里折叠成一行的第 `i` 段冲突：序号和「保留两者」「手动编辑」。
fn draw_folded_conflict(
    app: &App,
    frame: &mut Frame<'_>,
    i: usize,
    count: usize,
    band: Rect,
    hits: &mut Vec<(Rect, MergeTarget)>,
) {
    let p = app.palette;
    // 盖住折叠行原来的文字，再铺上与行号栏、连接带相同的底色。
    frame.rect_clipped(band, p.editor_bg, 0.0, band);
    frame.rect_clipped(band, conflict_shade(p).line, 0.0, band);
    let title = t!("git-merge-conflict-title", index = i + 1, count = count);
    let mut x = ui_text_small(frame, app, band.x + TEXT_PAD, band, &title, p.text, band) + 14.0;
    let h = (band.h - 4.0).max(12.0);
    for target in [
        MergeTarget::Pick(i, Pick::Both),
        MergeTarget::EditConflict(i),
    ] {
        let text = label(target);
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + 12.0;
        let rect = Rect::new(x, band.y + (band.h - h) / 2.0, w, h);
        if app.hover == Hover::Merge(target) {
            frame.rect_clipped(rect, p.hover, RADIUS_ROW, band);
        }
        ui_text_small(frame, app, rect.x + 6.0, band, text, p.accent, band);
        hits.push((rect.intersect(band), target));
        x = rect.right() + 2.0;
    }
}

/// 绘制三栏合并的两侧、连接带、标题行和底部操作栏。
///
/// 两侧随结果一栏滚动：同一高度的行相对应，结果一栏的一行对应这一侧多行时，这一侧滚动得更快。
pub(in crate::view) fn draw_side_panes(app: &mut App, frame: &mut Frame<'_>) {
    if !app.merge_panes_placed() {
        return;
    }
    let Some(index) = app.editor.active else {
        return;
    };
    let count = app.conflicts(index).len();
    let Some(mut panes) = app.merge.panes.take() else {
        return;
    };
    let lh = app.fonts.line_height;
    let text_y = app.layout.text.y;
    let doc = &app.editor.documents[index];
    let revision = doc.revision;
    let scroll = doc.scroll_line.max(0.0);
    let row = scroll.floor() as usize;
    let fraction = scroll - row as f32;
    let (line, next) = (doc.line_of_row(row), doc.line_of_row(row + 1));
    let file_y = |line: usize| text_y + (doc.row_of(line) as f32 - scroll) * lh;

    let mut hits = Vec::new();
    if let Some((aligned_at, aligned)) = &panes.aligned {
        // 先算好结果一栏里各段的位置，之后绘制时不再借用文档。
        let spans: [Vec<(&AlignSegment, (f32, f32))>; 2] = [0, 1].map(|side| {
            (aligned[side].segments().iter())
                .map(|segment| {
                    let y0 = file_y(segment.file.start);
                    let y1 = match segment.file.end.checked_sub(1) {
                        Some(last) if !segment.file.is_empty() => {
                            // 自动换行时一行占多个显示行；折叠着的行不占显示行，按一行计。
                            file_y(last) + doc.rows_of(last).max(1) as f32 * lh
                        }
                        _ => y0,
                    };
                    (segment, (y0, y1))
                })
                .collect()
        });
        let tops = [0, 1].map(|side| {
            let (start, end) = (aligned[side].side_line(line), aligned[side].side_line(next));
            start as f32 + fraction * (end as f32 - start as f32)
        });
        for side in 0..2 {
            draw_side_pane(
                app,
                frame,
                side,
                &mut panes.sides[side],
                tops[side],
                &spans[side],
                &mut hits,
            );
        }
        // 文档改过而新的对齐还没送达时，连接带上的冲突序号已经过期，点击会解决另一段冲突。
        if *aligned_at != revision {
            hits.clear();
        }
    }
    draw_merge_chrome(app, frame, &panes, count, &mut hits);
    app.merge.hits.extend(hits);
    app.merge.panes = Some(panes);
}

/// 绘制三栏合并的一侧和它与结果一栏之间的连接带：`top` 是第一行的行号（可以是小数），`spans` 是要
/// 连接的各段和它们在结果一栏的上下边。
fn draw_side_pane(
    app: &mut App,
    frame: &mut Frame<'_>,
    side: usize,
    source: &mut winc_editor::diff_view::DiffSide,
    top: f32,
    spans: &[(&AlignSegment, (f32, f32))],
    hits: &mut Vec<(Rect, MergeTarget)>,
) {
    let p = app.palette;
    let rect = app.merge.pane_rects[side];
    let ribbon = app.merge.ribbons[side];
    frame.rect(rect, p.editor_bg);
    frame.rect(ribbon, p.editor_bg);
    let body_y = rect.y + PANE_HEADER;
    let body = Rect::new(rect.x, body_y, rect.w, (rect.h - PANE_HEADER).max(0.0));
    let clip = Rect::new(ribbon.x, body_y, ribbon.w, body.h);
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let lh = app.fonts.line_height;
    let total = source.text.len_lines();
    let gutter_w = pane_gutter_width(app, total);
    // 行号挨着中间：左栏在右边，右栏在左边。
    let (gutter, text) = if side == 0 {
        let gutter = Rect::new(rect.right() - gutter_w, body.y, gutter_w, body.h);
        (
            gutter,
            Rect::new(rect.x, body.y, (gutter.x - rect.x).max(0.0), body.h),
        )
    } else {
        let gutter = Rect::new(rect.x, body.y, gutter_w, body.h);
        let x = gutter.right();
        (
            gutter,
            Rect::new(x, body.y, (rect.right() - x).max(0.0), body.h),
        )
    };
    let row_y = |line: usize| body.y + (line as f32 - top) * lh;

    // 各段的底色和连接带：左栏的带子从这一侧连到结果，右栏的带子从结果连到这一侧。
    for (segment, file) in spans {
        let shade = match segment.conflict {
            Some(_) => conflict_shade(p),
            None => merged_shade(p, segment),
        };
        let this = (row_y(segment.side.start), row_y(segment.side.end));
        if segment.side.is_empty() {
            let mark = Rect::new(rect.x, this.0 - 1.0, rect.w, 2.0);
            frame.rect_clipped(mark, shade.edge, 0.0, body);
        } else {
            let band = Rect::new(rect.x, this.0, rect.w, this.1 - this.0);
            frame.rect_clipped(band, shade.line, 0.0, body);
        }
        let (left, right) = if side == 0 {
            (this, *file)
        } else {
            (*file, this)
        };
        // 两端同高时是一条直带，与两边的底色连成一片，不需要边线勾勒。
        let straight = (left.0 - right.0).abs() < 0.5 && (left.1 - right.1).abs() < 0.5;
        let shade = if straight {
            DiffShade {
                edge: shade.line,
                ..shade
            }
        } else {
            shade
        };
        draw_ribbon(frame, ribbon, left, right, shade, clip);
    }

    let first = top.floor().max(0.0) as usize;
    let visible = (body.h / lh).ceil() as usize + 1;
    let (mut chars, mut tokens) = (Vec::with_capacity(128), Vec::with_capacity(16));
    let tab_width = app.editor.tab_width;
    for line in first..(first + visible).min(total) {
        let band = Rect::new(rect.x, row_y(line), rect.w, lh);
        let baseline = baseline_in(band, ascent, descent);
        let number = (line + 1).to_string();
        let number_x = gutter.right() - PANE_PAD - number.chars().count() as f32 * app.cell;
        frame.text(
            FontKind::Mono,
            app.fonts.code,
            number_x,
            baseline,
            &number,
            p.gutter_text,
            gutter,
        );
        crate::view::diff::line_chars(source, line, &mut chars);
        let state =
            source
                .highlighter
                .state_at(&source.text, line, crate::view::editor::HIGHLIGHT_BUDGET);
        app.highlight_pending |= state.is_none();
        tokens.clear();
        let _ = winc_syntax::highlight_line(
            source.highlighter.language(),
            &chars,
            state.unwrap_or_default(),
            &mut tokens,
        );
        crate::view::editor::draw_glyphs(
            app,
            frame,
            &chars,
            &tokens,
            text.x + PANE_PAD,
            baseline,
            tab_width,
            text,
            &[],
        );
    }

    // 连接带上的采用按钮，与这一侧冲突的第一行同高。
    let pick = if side == 0 { Pick::Ours } else { Pick::Theirs };
    for (segment, _) in spans {
        let Some(i) = segment.conflict else {
            continue;
        };
        let mid = row_y(segment.side.start)
            + if segment.side.is_empty() {
                0.0
            } else {
                lh / 2.0
            };
        if mid < clip.y || mid > clip.bottom() {
            continue;
        }
        let c = (ribbon.x + ribbon.w / 2.0, mid);
        let target = MergeTarget::Pick(i, pick);
        let hover = (app.hover == Hover::Merge(target)).then_some(p.hover);
        if side == 0 {
            icons::line_replace(frame, c, hover, p.accent, clip);
        } else {
            icons::line_replace_back(frame, c, hover, p.accent, clip);
        }
        let rect = Rect::new(c.0 - 10.0, c.1 - 10.0, 20.0, 20.0).intersect(clip);
        hits.push((rect, target));
    }

    // 文字和行号之间的分隔线，与差异视图相同。
    let rule_x = if side == 0 {
        text.right()
    } else {
        gutter.right()
    };
    let rule = p.separator.alpha(p.separator.a * 0.7);
    frame.rect(Rect::new(rule_x, body.y, 1.0, body.h), rule);
}

/// 返回行数为 `total` 的一侧行号栏的宽度（逻辑像素）。
fn pane_gutter_width(app: &App, total: usize) -> f32 {
    let digits = total.to_string().len().max(2) as f32;
    (digits * app.cell + PANE_PAD * 2.0).round()
}

/// 绘制三栏合并的标题行和底部操作栏。
fn draw_merge_chrome(
    app: &App,
    frame: &mut Frame<'_>,
    panes: &MergePanes,
    count: usize,
    hits: &mut Vec<(Rect, MergeTarget)>,
) {
    let p = app.palette;
    let [left, right] = app.merge.pane_rects;
    let whole = Rect::new(left.x, left.y, right.right() - left.x, PANE_HEADER);
    frame.rect(whole, p.editor_bg);
    frame.rect(
        Rect::new(whole.x, whole.bottom() - 1.0, whole.w, 1.0),
        p.separator,
    );

    // 两侧的标题与代码左对齐，前面的锁表示只读。
    let code_x = [
        left.x + PANE_PAD,
        right.x + pane_gutter_width(app, panes.sides[1].text.len_lines()) + PANE_PAD,
    ];
    for (side, x) in code_x.into_iter().enumerate() {
        let rect = app.merge.pane_rects[side];
        icons::lock(frame, (x + 4.0, whole.y + whole.h / 2.0), p.text_dim, whole);
        let title_x = x + 14.0;
        let room = (rect.right() - 12.0 - title_x).max(0.0);
        let title = ellipsize(frame, &panes.sides[side].title, app.fonts.ui, room).into_owned();
        ui_text(frame, app, title_x, whole, &title, p.text, whole);
    }

    // 结果一栏：标题、还剩几处、上一处、下一处。
    let result = app.layout.editor;
    let bar = Rect::new(result.x, whole.y, result.w, whole.h);
    let title = t!("git-merge-side-result");
    ui_text(frame, app, app.layout.text.x, bar, title, p.text, bar);
    let mut x = bar.right() - 8.0;
    for (target, direction) in [
        (MergeTarget::Next, icons::Direction::Down),
        (MergeTarget::Previous, icons::Direction::Up),
    ] {
        x -= BUTTON_SMALL;
        let rect = Rect::new(
            x,
            bar.y + (bar.h - BUTTON_SMALL) / 2.0,
            BUTTON_SMALL,
            BUTTON_SMALL,
        );
        draw_button(
            frame,
            p,
            rect,
            bar,
            app.hover == Hover::Merge(target),
            false,
        );
        icons::chevron(frame, center(rect), 8.0, direction, p.text_muted, bar);
        hits.push((rect, target));
        x -= 2.0;
    }
    let status = if count > 0 {
        t!("git-merge-card-conflicts", count = count)
    } else {
        t!("git-merge-card-resolved").to_owned()
    };
    let color = if count > 0 { p.error } else { p.git_added };
    let w = frame.measure(FontKind::Ui, app.fonts.ui_small, &status);
    ui_text_small(frame, app, x - 6.0 - w, bar, &status, color, bar);

    // 底部操作栏：左边是整个文件采用一侧，右边是退出和完成。
    let footer = app.merge.footer;
    frame.rect(footer, p.editor_bg);
    frame.rect(Rect::new(footer.x, footer.y, footer.w, 1.0), p.separator);
    let y = footer.y + (footer.h - SETTINGS_CONTROL_H) / 2.0;
    let width = |frame: &mut Frame<'_>, text: &str| {
        frame.measure(FontKind::Ui, app.fonts.ui, text) + FOOTER_BUTTON_PAD * 2.0
    };
    let mut button =
        |frame: &mut Frame<'_>, x: f32, text: &str, role, enabled, target: MergeTarget| {
            let w = width(frame, text);
            let rect = Rect::new(x, y, w, SETTINGS_CONTROL_H);
            let hovered = app.hover == Hover::Merge(target);
            draw_action_button(app, frame, rect, text, role, enabled, hovered);
            if enabled {
                hits.push((rect, target));
            }
            w
        };
    let mut x = footer.x + 12.0;
    for (target, text) in [
        (
            MergeTarget::All(Pick::Ours),
            t!("git-merge-accept-all-current"),
        ),
        (
            MergeTarget::All(Pick::Theirs),
            t!("git-merge-accept-all-incoming"),
        ),
    ] {
        x += button(frame, x, text, ButtonRole::Normal, count > 0, target) + 8.0;
    }
    let done = t!("git-merge-mark-resolved");
    let exit = t!("git-merge-close-three-pane");
    let done_w = width(frame, done);
    let exit_w = width(frame, exit);
    let done_x = footer.right() - 12.0 - done_w;
    let primary = ButtonRole::Primary;
    button(
        frame,
        done_x,
        done,
        primary,
        count == 0,
        MergeTarget::MarkResolved,
    );
    let exit_x = done_x - 8.0 - exit_w;
    button(
        frame,
        exit_x,
        exit,
        ButtonRole::Normal,
        true,
        MergeTarget::ThreePane,
    );
}
