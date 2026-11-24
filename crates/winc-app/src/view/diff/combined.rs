//! 「未提交的更改」标签的绘制：顶部的工具栏，各段的段头和差异，视口顶边所在那一段的段头
//! 吸顶，以及滚动条上每处更改的位置。
//!
//! 各段的差异按差异标签的方式绘制：把露出来的那一段换进标签（`App::draw_combined_section`），
//! 只绘制正文，不绘制差异的标题栏。

use super::*;
use crate::app::git::combined::{Body, CombinedHit, SECTION_ROW, Section};
use crate::view::workspace::sidebar::git_color;
use winc_render::Stroke;

/// 段与段之间斜线纹的线距（逻辑像素，取自调参）：够疏，不显得杂乱。
const HATCH: f32 = 6.0;
/// 段头上文件图标的左边离段头左边缘的距离（逻辑像素），左边让出折叠箭头。
const ICON_X: f32 = 30.0;
/// 段头上文件名的左边离段头左边缘的距离（逻辑像素）：「正在比较」与它对齐。
const NAME_X: f32 = ICON_X + icons::FILE_WIDTH + 8.0;
/// 段头右端的统计离右边缘的距离（逻辑像素）：让出滚动条和概览标尺。
const END_PAD: f32 = 20.0;
/// 段头上统计之间的间距（逻辑像素）。
const STAT_GAP: f32 = 8.0;
/// 段头上至少留给文件名的宽度（逻辑像素），统计和原因在此之外截短。
const NAME_MIN: f32 = 120.0;

/// 绘制第 `index` 个文档（「未提交的更改」标签）。
pub(in crate::view) fn draw_combined(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let parts = app.combined_parts();
    let p = app.palette;
    let lh = app.fonts.line_height;
    let scroll = app.editor.documents[index].scroll_line.max(0.0);
    let starts = app.combined_starts();
    let header_rows = app.combined_header_rows();
    let mut hits: Vec<(Rect, CombinedHit)> = Vec::new();
    let mut section_hits = Vec::new();
    let count = app.combined_tab().map_or(0, |t| t.sections.len());
    let y_of = |row: f64| parts.body.y + ((row - scroll) * f64::from(lh)) as f32;
    app.note_combined_hover();

    for i in 0..count {
        let (top, bottom) = (y_of(starts[i]), y_of(starts[i + 1]));
        if bottom <= parts.body.y || top >= parts.body.bottom() {
            continue;
        }
        // 先画差异，段头画在上面。段头露出来而正文还在视口外（段头刚从底边上来）时不画正文：
        // 正文没有露出的部分，换进标签时会按整个正文区从这一段的开头画，盖在上一段上。
        let mut diff_hits = Vec::new();
        if app.combined_section_area(i).is_some() {
            app.update_combined_blame(i);
            app.draw_combined_section(i, |app| {
                draw_section_body(app, frame, (index, i), &mut diff_hits)
            });
        }
        section_hits.push((i, diff_hits));
        let Some(section) = app.combined_tab().and_then(|t| t.sections.get(i)) else {
            continue;
        };
        if let (false, Body::Waiting) = (section.folded, &section.body) {
            let row = Rect::new(
                parts.body.x,
                y_of(starts[i] + header_rows),
                parts.body.w,
                lh,
            );
            let text = t!("git-combined-comparing");
            ui_text(
                frame,
                app,
                row.x + NAME_X,
                row,
                text,
                p.text_dim,
                parts.body,
            );
        }
        let header = Rect::new(parts.body.x, top, parts.body.w, app.combined_header());
        draw_section_header(app, frame, i, header, parts.body, false, &mut hits);
    }

    // 视口顶边所在那一段的段头已经滚出去时，贴在顶边，看得出正在看哪个文件。段头的那一行
    // 还没滚过顶边（顶边落在它上面的间隔里）时不贴：贴上的一份会和它本身叠在一起。
    let gap = f64::from(app.combined_gap() / lh);
    let current = (starts.iter().take(count)).rposition(|&s| s + gap < scroll);
    if let Some(i) = current {
        // 下一段的间隔上来时把它顶上去。
        let next = y_of(starts[i + 1]);
        let y = (next - SECTION_ROW).min(parts.body.y);
        let header = Rect::new(parts.body.x, y, parts.body.w, SECTION_ROW);
        draw_section_header(app, frame, i, header, parts.body, true, &mut hits);
    }

    if count == 0 {
        let row = Rect::new(parts.body.x, parts.body.y + 24.0, parts.body.w, lh);
        let text = t!("git-combined-empty");
        let w = frame.measure(FontKind::Ui, app.fonts.ui, text);
        let x = row.x + (row.w - w) / 2.0;
        ui_text(frame, app, x, row, text, p.text_muted, parts.body);
    }

    draw_toolbar(app, frame, parts.toolbar, &mut hits);
    draw_combined_scrollbar(app, frame, index, parts.body, &starts);
    // 段头上的按钮叠在差异上面：先列出的先命中。
    hits.reverse();
    app.hits.combined = hits;
    if let Some(tab) = app.git.combined.as_mut() {
        tab.section_hits = section_hits;
    }
}

/// 绘制换进标签的第 `section` 段差异（第 `index` 个文档）的正文和逐行追溯一栏（见
/// [`draw_diff`]，不含标题栏、滚动条和吸顶的行）。
///
/// 悬停在这一段的部件上时临时换成差异自己的悬停目标，展开按钮、追溯一格的高亮照常绘制。
fn draw_section_body(
    app: &mut App,
    frame: &mut Frame<'_>,
    (index, section): (usize, usize),
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let Some(mut view) = app.editor.documents[index].diff.take() else {
        return;
    };
    let hovered = app.combined_tab().and_then(|t| t.hovered) == Some(section);
    let hover = app.pointer.hover;
    if !hovered {
        app.pointer.hover = Hover::None;
    }
    let parts = diff_parts(app.layout.editor, &view, app.metrics.cell);
    let doc = &app.editor.documents[index];
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    let pos = doc.scroll_line.max(0.0);
    let scroll = Scroll {
        rows: [
            view.side_scroll(LEFT, pos, visible),
            view.side_scroll(RIGHT, pos, visible),
        ],
        x: doc.scroll_x,
    };
    let rows = visible_rows(scroll.rows[RIGHT], parts.body.h, app.fonts.line_height);
    let mut edit = edit_state(app, &view, new_lines(&view, rows));
    // 查找的匹配是按各段接成的快照算的位置，不在这一段的文字里标；当前匹配是这一段的选区。
    edit.searched = None;
    let id = app.editor.documents[index].id;
    let key = app
        .combined_tab()
        .and_then(|t| t.sections.get(section))
        .map(|s| s.id);
    // 这一段正在展开、折叠未更改的内容：同差异标签的过渡。滚动时照常绘制：各段的位置
    // （`App::combined_starts`）一直按过渡中的高度算，两边要一致。
    let shift = (app.motion.diff_fold.as_ref())
        .filter(|m| m.doc == id && m.section == key && view.is_open(m.gap))
        .map(|m| FoldShift {
            rows: m.rows.clone(),
            hidden: m.hidden(app.fonts.line_height),
            line_height: app.fonts.line_height,
        });
    let scroll = (scroll, shift.as_ref());
    let drawn = draw_body(app, frame, &mut view, &parts, scroll, pos, &edit, hits);
    if let Some(key) = key {
        let blame = |side| crate::app::git::blame::BlameKey::Section(id, key, side);
        blame::draw(app, frame, (blame, &view), &parts, &drawn, hits);
    }
    app.pointer.hover = hover;
    app.editor.documents[index].diff = Some(view);
}

/// 绘制第 `i` 段的段头（在 `rect`，裁到 `clip`），排法同提交视图的文件行：左边是折叠箭头、
/// 文件图标、文件名和所在目录，右边是增删行数（不显示内容时是原因）和状态字母；悬停时增删行数
/// 左边出现「打开文件」和「在差异标签中打开」。`pinned` 为真时是吸顶的那一份，分隔线画在下面。
fn draw_section_header(
    app: &App,
    frame: &mut Frame<'_>,
    i: usize,
    rect: Rect,
    clip: Rect,
    pinned: bool,
    hits: &mut Vec<(Rect, CombinedHit)>,
) {
    let Some(section) = app.combined_tab().and_then(|t| t.sections.get(i)) else {
        return;
    };
    let p = app.palette;
    // 不吸顶时上面是与上一段之间的间隔；第一段的间隔在滚动范围之上，看不见，不画。
    let rect = if pinned {
        rect
    } else {
        let gap = Rect::new(rect.x, rect.y, rect.w, app.combined_gap());
        if i > 0 {
            draw_gap(frame, p, gap, clip);
        }
        Rect::new(rect.x, gap.bottom(), rect.w, SECTION_ROW)
    };
    let clip = rect.intersect(clip);
    if clip.h <= 0.0 {
        return;
    }
    let hovered = |hit| app.pointer.hover == Hover::Combined(hit);
    let active = hovered(CombinedHit::Header(i))
        || hovered(CombinedHit::OpenFile(i))
        || hovered(CombinedHit::OpenDiff(i));
    frame.rect(clip, p.editor_bg);
    if active {
        frame.rect(clip, p.hover);
    }
    // 吸顶时下面一条分隔线，与下面滚动的内容分开。
    if pinned {
        let line = Rect::new(rect.x, rect.bottom() - 1.0, rect.w, 1.0);
        frame.rect_clipped(line, p.separator, 0.0, clip);
    }

    let cy = icon_center_y(app, rect, app.fonts.ui);
    // 折叠、展开的过渡中箭头跟着正文转动。
    let angle = app.combined_section_open(i) * std::f32::consts::FRAC_PI_2;
    icons::chevron_turned(frame, (rect.x + 16.0, cy), 8.0, angle, p.text_muted, clip);
    let name = (section.path.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let x = draw_file_icon(frame, p, (rect.x + ICON_X, cy), &name, clip) + 8.0;

    // 右端自右向左：状态字母、删除和新增的行数（为 0 的不写），不显示内容时是原因。
    let small = app.fonts.ui_small;
    let mut right = rect.right() - END_PAD;
    let mut texts = vec![(
        section.change.letter().to_string(),
        git_color(p, section.change),
    )];
    match (&section.body, section.stats()) {
        (Body::Skipped(reason), _) => texts.push((reason.clone(), p.text_dim)),
        (_, Some((added, removed))) => {
            if removed > 0 {
                texts.push((format!("−{removed}"), p.git_deleted));
            }
            if added > 0 {
                texts.push((format!("+{added}"), p.git_added));
            }
        }
        _ => {}
    }
    let room = (right - x - NAME_MIN).max(0.0);
    for (text, color) in &texts {
        let text = ellipsize(frame, text, small, room);
        right -= frame.measure(FontKind::Ui, small, &text);
        ui_text_small(frame, app, right, rect, &text, *color, clip);
        right -= STAT_GAP;
    }
    // 悬停时的按钮在统计左边，统计的位置不随悬停变化。
    if active {
        let y = rect.y + (rect.h - BUTTON) / 2.0;
        for hit in [CombinedHit::OpenDiff(i), CombinedHit::OpenFile(i)] {
            let button = Rect::new(right - BUTTON, y, BUTTON, BUTTON);
            right = button.x - 2.0;
            draw_button(frame, p, button, clip, hovered(hit), false);
            let color = if hovered(hit) { p.text } else { p.text_muted };
            match hit {
                CombinedHit::OpenFile(_) => icons::open_file(frame, center(button), color, clip),
                _ => icons::external(frame, center(button), color, clip),
            }
            hits.push((button.intersect(clip), hit));
        }
    }

    // 文件名，后面是仓库内的目录（淡色、小一号，过长时从中间截短）。
    let name_clip = Rect::new(x, rect.y, (right - 8.0 - x).max(0.0), rect.h).intersect(clip);
    let end = ui_text(frame, app, x, rect, &name, p.text, name_clip);
    let dir = (app.git.root.as_deref())
        .and_then(|root| section.path.parent()?.strip_prefix(root).ok())
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.display().to_string());
    if let Some(dir) = dir {
        let x = end + 6.0;
        let dir = ellipsize(frame, &dir, small, (name_clip.right() - x).max(0.0));
        ui_text_small(frame, app, x, rect, &dir, p.text_dim, name_clip);
    }
    hits.push((rect.intersect(clip), CombinedHit::Header(i)));
}

/// 绘制工具栏：文件数和总的增删行数，右端的按钮与差异标签的标题栏相同。
fn draw_toolbar(app: &App, frame: &mut Frame<'_>, bar: Rect, hits: &mut Vec<(Rect, CombinedHit)>) {
    let p = app.palette;
    frame.rect(bar, p.editor_bg);
    frame.rect(
        Rect::new(bar.x, bar.bottom() - 1.0, bar.w, 1.0),
        p.separator,
    );
    // 右端自右向左，与差异标签的标题栏相同：合并差异视图（开着）、单栏与并排、折叠未更改的
    // 行、下一处、上一处。
    let y = bar.y + (bar.h - BUTTON) / 2.0;
    let mut right = bar.right() - 10.0;
    for hit in [
        CombinedHit::View,
        CombinedHit::Layout,
        CombinedHit::Collapse,
        CombinedHit::Next,
        CombinedHit::Previous,
    ] {
        let rect = Rect::new(right - BUTTON, y, BUTTON, BUTTON);
        right = rect.x - 2.0;
        let hovered = app.pointer.hover == Hover::Combined(hit);
        draw_button(frame, p, rect, bar, hovered, hit == CombinedHit::View);
        let c = center(rect);
        let color = p.text_muted;
        match hit {
            CombinedHit::View => icons::files(frame, c, 1.0, p.text, rect),
            CombinedHit::Layout if app.editor.diff_unified => icons::split(frame, c, color, rect),
            CombinedHit::Layout => icons::single_column(frame, c, color, rect),
            CombinedHit::Collapse if app.combined_collapses() => {
                icons::expand_all(frame, c, color, rect);
            }
            CombinedHit::Collapse => icons::collapse_all(frame, c, color, rect),
            CombinedHit::Next => icons::chevron(frame, c, 8.0, Direction::Down, color, rect),
            _ => icons::chevron(frame, c, 8.0, Direction::Up, color, rect),
        }
        hits.push((rect, hit));
    }

    let sections = app.combined_tab().map_or(&[][..], |t| &t.sections[..]);
    let (added, removed) = sections
        .iter()
        .filter_map(Section::stats)
        .fold((0, 0), |(a, r), (x, y)| (a + x, r + y));
    // 标签上已经写着「未提交的更改」，这里只写文件数和增删行数（为 0 的不写），与段头的箭头
    // 左对齐。
    let files = t!("git-combined-files", count = sections.len());
    let mut x = ui_text(frame, app, bar.x + 10.0, bar, &files, p.text, bar) + 10.0;
    let counts = [
        (added, format!("+{added}"), p.git_added),
        (removed, format!("−{removed}"), p.git_deleted),
    ];
    for (_, text, color) in counts.iter().filter(|(n, ..)| *n > 0) {
        x = ui_text_small(frame, app, x, bar, text, *color, bar) + STAT_GAP;
    }
}

/// 绘制滚动条和概览标尺：每处更改在标尺上标出位置，颜色同差异中这一处的底色。
fn draw_combined_scrollbar(
    app: &mut App,
    frame: &mut Frame<'_>,
    index: usize,
    track: Rect,
    starts: &[f64],
) {
    let doc = &app.editor.documents[index];
    let scroll = (doc.scroll_line * f64::from(app.fonts.line_height)) as f32;
    let owner = doc.id.0;
    let total = app.combined_rows().max(1);
    let content = editor_content_height(total, track.h, app.fonts.line_height);
    let ruler = Ruler::new(track, total);
    let header = app.combined_header_rows();
    let p = app.palette;
    for (i, section) in app
        .combined_tab()
        .map_or(&[][..], |t| &t.sections[..])
        .iter()
        .enumerate()
    {
        let (false, Body::Ready(view)) = (section.folded, &section.body) else {
            continue;
        };
        let base = starts[i] + header;
        for (h, hunk) in view.hunks.iter().enumerate() {
            let span = view.hunk_span(h);
            let rows = (base + span.start).floor() as usize..(base + span.end).ceil() as usize;
            ruler.mark(frame, GIT_LANE, rows, hunk_shade(p, hunk).mark);
        }
    }
    draw_scrollbar(
        app,
        frame,
        ScrollArea::Editor,
        owner,
        track,
        scroll,
        content,
    );
}

/// 在 `gap` 画两个文件之间的间隔：上下各一条分隔线，中间是 45° 的细斜线纹。
///
/// 斜线逐条描、端点对齐到物理像素：每条形状相同，滚动时只取渲染器缓存的蒙版贴上，不逐像素
/// 重新计算整条间隔。
fn draw_gap(frame: &mut Frame<'_>, p: &Palette, gap: Rect, clip: Rect) {
    let clip = gap.intersect(clip);
    if clip.h <= 0.0 {
        return;
    }
    frame.rect(clip, p.editor_bg);
    let scale = frame.scale();
    let snap = |v: f32| (v * scale).round() / scale;
    let (top, bottom) = (snap(gap.y), snap(gap.bottom()));
    let rise = bottom - top;
    let color = p.separator.alpha(p.separator.a * 0.8);
    let mut x = snap(gap.x - rise);
    while x < clip.right() {
        if x + rise > clip.x {
            frame.stroke(&[Stroke::Line(x, bottom, x + rise, top)], 1.0, color, clip);
        }
        x += HATCH;
    }
    for y in [gap.y, gap.bottom() - 1.0] {
        frame.rect_clipped(Rect::new(gap.x, y, gap.w, 1.0), p.separator, 0.0, clip);
    }
}
