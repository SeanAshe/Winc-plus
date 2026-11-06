//! 并排差异标签的绘制：标题栏（两侧标题、改动处数、增删行数、上一处、下一处、折叠开关），
//! 左右两栏的代码和行号，中间连接对应改动的带子，滚动条上每处改动的位置。
//!
//! 两侧各绘制各的行，行号挨着中间的带子。每段改动按类型上色（新增绿、删除灰、修改蓝，都是主题里
//! 配好的柔和色），两侧的底色与带子同色，一侧没有行时那一侧在插入的位置绘制一道线；改了的字再
//! 深一层。「»」在左侧行号的左边，把左侧的一段换到右侧。

use std::fmt::Write as _;
use std::ops::Range;

use winc_config::RenderWhitespace;
use winc_core::text::{self, MAX_LINE_CHARS};
use winc_core::wrap::{self, Breaks};
use winc_core::{Lines, Selection};
use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT, SideRow};
use winc_i18n::t;

use super::editor::{
    HIGHLIGHT_BUDGET, caret_bar, draw_caret, draw_glyphs, draw_whitespace_mark,
    trailing_whitespace, whitespace_shown,
};
use super::*;
use crate::app::{
    DIFF_GUTTER_PAD, DIFF_TEXT_PAD, DiffParts, DiffTarget, Focus, HunkKind, diff_parts,
};
use crate::icons::Direction;
use crate::layout::{GIT_LANE, Ruler, editor_content_height};

/// 中间一栏勾选框的边长（逻辑像素）；「»」按钮的图标和它一样大，左右成对。
const DIFF_BUTTON: f32 = 13.0;
/// 工具栏按钮的边长（逻辑像素）。
const BUTTON: f32 = 24.0;
/// 返回一处改动使用的颜色：只在右侧新增的绿、只在左侧删掉的灰、两侧都有（修改）的蓝。
pub(super) fn hunk_shade(p: &Palette, hunk: &winc_core::diff::LineHunk) -> DiffShade {
    kind_shade(p, HunkKind::of(hunk))
}

fn kind_shade(p: &Palette, kind: HunkKind) -> DiffShade {
    match kind {
        HunkKind::Added => p.diff_added,
        HunkKind::Deleted => p.diff_deleted,
        HunkKind::Modified => p.diff_modified,
    }
}

/// 「»」换掉的那段改动在原处淡出：两侧换完以后对应的行铺上原来的颜色，中间连一条带子，
/// 透明度随进度降到 0。
///
/// 先绘制，文字压在上面。
fn draw_fade(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    row_y: &dyn Fn(usize, usize) -> f32,
) {
    let Some(fade) = &app.diff_fade else {
        return;
    };
    if app.editor.active().map(|d| d.id) != Some(fade.doc) {
        return;
    }
    let left = 1.0 - fade.clock.linear();
    let alpha = left * left;
    let shade = kind_shade(app.palette, fade.kind);
    let shade = DiffShade {
        line: shade.line.alpha(alpha),
        word: shade.word.alpha(alpha),
        edge: shade.edge.alpha(alpha),
        mark: shade.mark.alpha(alpha),
    };
    let span = |side: usize, lines: &Range<usize>| {
        (
            row_y(side, view.row_of(side, lines.start)),
            row_y(side, view.row_of(side, lines.end)),
        )
    };
    let spans = [span(LEFT, &fade.left), span(RIGHT, &fade.right)];
    for (side, &(y0, y1)) in spans.iter().enumerate() {
        let pane = parts.panes[side].rect;
        if y1 > y0 {
            frame.rect_clipped(
                Rect::new(pane.x, y0, pane.w, y1 - y0),
                shade.line,
                0.0,
                parts.body,
            );
        } else {
            let line = Rect::new(pane.x, y0 - 0.5, pane.w, 1.0);
            frame.rect_clipped(line, shade.edge, 0.0, parts.body);
        }
    }
    draw_ribbon(frame, parts.ribbon, spans[0], spans[1], shade, parts.body);
}

/// 标出跳到的那一段改动：一侧的边上一条强调色的竖线，这一侧没有行时是插入位置上的横线；刚跳到时
/// 整段再叠一层渐隐的强调色。
///
/// `(y0, y1)` 是这一段在这一侧的上下边，`clip` 是差异的文字区域。
fn draw_hunk_focus(app: &App, frame: &mut Frame<'_>, pane: Rect, (y0, y1): (f32, f32), clip: Rect) {
    /// 当前标记的粗细（逻辑像素）：比 1 像素的改动边线粗，能和普通改动区分开。
    const MARK: f32 = 3.0;
    /// 高亮开始时强调色的不透明度：能看出落点，又不遮挡文字。
    const PULSE_ALPHA: f32 = 0.28;
    let accent = app.palette.accent;
    if y1 > y0 {
        if let Some(pulse) = app.diff_pulse {
            let band = Rect::new(pane.x, y0, pane.w, y1 - y0);
            frame.rect_clipped(
                band,
                accent.alpha(PULSE_ALPHA * pulse.fade_out()),
                0.0,
                clip,
            );
        }
        frame.rect_clipped(Rect::new(pane.x, y0, MARK, y1 - y0), accent, 0.0, clip);
    } else {
        let line = Rect::new(pane.x, y0 - MARK / 2.0, pane.w, MARK);
        frame.rect_clipped(line, accent, 0.0, clip);
    }
}

/// 两侧的滚动位置（显示行）。
#[derive(Clone, Copy)]
struct Scroll {
    /// 左侧、右侧。
    rows: [f32; 2],
    /// 两侧共用的横向滚动（逻辑像素）。
    x: f32,
}

/// 绘制当前标签（第 `index` 个）的差异。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_diff(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    app.sync_diff_source(index);
    // 绘制时要推进两侧的着色状态，同时要读 `app` 的字体和配色：先把差异从文档里取出来，绘制完放回。
    let Some(mut view) = app.editor.documents[index].diff.take() else {
        return;
    };
    let parts = diff_parts(app.layout.editor, &view, app.cell);
    let doc = &app.editor.documents[index];
    let visible = parts.body.h / app.fonts.line_height;
    // 滚动位置是两侧共用的一把尺，两侧各自换算成自己滚到的行（见 `DiffView::row_at`）。
    let pos = doc.scroll_line.max(0.0);
    let scroll = Scroll {
        rows: [
            view.side_scroll(LEFT, pos, visible),
            view.side_scroll(RIGHT, pos, visible),
        ],
        x: doc.scroll_x,
    };
    let rows = visible_rows(scroll.rows[RIGHT], parts.body.h, app.fonts.line_height);
    let edit = edit_state(app, &view, new_lines(&view, rows));
    let mut hits = Vec::new();

    draw_body(app, frame, &mut view, &parts, scroll, pos, &edit, &mut hits);
    draw_header(app, frame, &view, &parts, pos, &mut hits);
    draw_overview(app, frame, &view, &parts, index);

    app.diff_hits = hits;
    app.editor.documents[index].diff = Some(view);
}

/// 绘制标题栏：两侧标题、改动处数与增删行数，右侧是折叠开关、上一处、下一处三个按钮。
fn draw_header(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    scroll: f32,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let p = app.palette;
    let header = parts.header;
    frame.rect(header, p.editor_bg);
    frame.rect(
        Rect::new(header.x, header.bottom() - 1.0, header.w, 1.0),
        p.separator,
    );

    // 右端：折叠开关、下一处、上一处（从右往左排）。折叠开关按状态换图标，不绘制按下状态。
    let y = header.y + (header.h - BUTTON) / 2.0;
    let mut right = header.right() - 10.0;
    let files = [DiffTarget::NextFile, DiffTarget::PreviousFile];
    let files = files.iter().copied().filter(|_| app.diff_steps_files());
    let targets = [DiffTarget::Layout, DiffTarget::Collapse]
        .into_iter()
        .chain(files);
    for target in targets.chain([DiffTarget::Next, DiffTarget::Previous]) {
        let rect = Rect::new(right - BUTTON, y, BUTTON, BUTTON);
        right = rect.x - 2.0;
        draw_button(
            frame,
            p,
            rect,
            header,
            app.hover == Hover::Diff(target),
            false,
        );
        let c = center(rect);
        match target {
            DiffTarget::Previous => {
                icons::chevron(frame, c, 8.0, Direction::Up, p.text_muted, rect)
            }
            DiffTarget::Next => icons::chevron(frame, c, 8.0, Direction::Down, p.text_muted, rect),
            DiffTarget::PreviousFile => {
                icons::chevron(frame, c, 8.0, Direction::Left, p.text_muted, rect)
            }
            DiffTarget::NextFile => {
                icons::chevron(frame, c, 8.0, Direction::Right, p.text_muted, rect)
            }
            // 图标画的是点下去以后的样子。
            DiffTarget::Layout if view.is_unified() => {
                icons::split(frame, c, p.text_muted, rect);
            }
            DiffTarget::Layout => icons::single_column(frame, c, p.text_muted, rect),
            _ if view.collapses() => icons::expand_all(frame, c, p.text_muted, rect),
            _ => icons::collapse_all(frame, c, p.text_muted, rect),
        }
        hits.push((rect, target));
    }

    // 按钮左边：增删行数，再左边第几处、一共几处。
    right -= 10.0;
    if view.is_identical() {
        let text = t!("editor-diff-identical");
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text);
        right -= w;
        ui_text_small(frame, app, right, header, text, p.text_muted, header);
    } else {
        let (added, removed) = view.stats();
        let at = view.current_hunk(scroll).unwrap_or(0);
        let parts_text = [
            (format!("−{removed}"), p.git_deleted),
            (format!("+{added}"), p.git_added),
            (format!("{} / {}", at + 1, view.hunks.len()), p.text_muted),
        ];
        for (text, color) in &parts_text {
            let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text);
            right -= w;
            ui_text_small(frame, app, right, header, text, *color, header);
            right -= 8.0;
        }
    }
    let stats_left = right - 8.0;

    // 两侧的标题各自从文字区的左边开始；右侧的不盖住统计。不能改的一侧前面一把锁。单栏只有一个
    // 文字区，两个标题前后排开，中间一个箭头。
    let mut after = None;
    for (side, pane) in parts.panes.iter().enumerate() {
        let title = if side == LEFT { &view.old } else { &view.new };
        let end = pane.text.right().min(stats_left);
        let clip = Rect::new(
            pane.text.x,
            header.y,
            (end - pane.text.x).max(0.0),
            header.h,
        );
        let mut x = match after {
            Some(end) => ui_text(frame, app, end + 8.0, header, "→", p.text_dim, clip) + 8.0,
            None => pane.text.x + DIFF_TEXT_PAD,
        };
        if title.path.is_none() {
            icons::lock(
                frame,
                (x + 4.0, header.y + header.h / 2.0),
                p.text_dim,
                clip,
            );
            x += 15.0;
        }
        // 「HEAD · 7ca683c」「工作区 · 未保存」：前一半是哪个版本，正常字色；后一半是补充，淡色。
        let (main, rest) = match title.title.split_once(" · ") {
            Some((main, rest)) => (main, Some(rest)),
            None => (title.title.as_str(), None),
        };
        let mut end = ui_text(frame, app, x, header, main, p.text, clip);
        if let Some(rest) = rest {
            let rest = format!(" · {rest}");
            end = ui_text(frame, app, end, header, &rest, p.text_dim, clip);
        }
        after = view.is_unified().then_some(end);
    }
}

/// 一个选区的起点和终点（行，列），起点在前。
type Span = ((usize, usize), (usize, usize));

/// 右侧的光标和选区，按右侧的行号和列（字符）算。
#[derive(Default)]
struct EditState {
    /// 光标在右侧时为 `true`。
    editing: bool,
    /// 主光标所在的（行，列）：输入法的候选窗跟着它。
    caret: (usize, usize),
    /// 其余光标所在的（行，列）（多光标），只有落在看得见的几行里的，按位置排好。
    carets: Vec<(usize, usize)>,
    /// 各个选区，按位置排好；只有和看得见的几行相交的，没有选中内容的选区不在里面。
    selections: Vec<Span>,
    /// 只读一侧的选区：（哪一侧，选区）；没有选中内容时为 `None`。
    read: Option<(usize, Span)>,
    /// 只读一侧的光标：（哪一侧，（行，列））。
    read_caret: Option<(usize, (usize, usize))>,
    /// 查找条开着、查的是右侧的文档时为它的标识：右侧要标出匹配。
    searched: Option<winc_editor::DocumentId>,
}

/// 读出右侧对应文档的光标和选区；多光标只取右侧文档第 `lines` 行（左闭右开）里看得见的。
fn edit_state(app: &App, view: &DiffView, lines: Range<usize>) -> EditState {
    // 只读一侧的选区和光标按这一侧的文本换成（行，列）。
    let at = |side: usize, pos: usize| {
        let text = &view.side(side).text;
        let pos = pos.min(text.len_chars());
        let line = text.char_to_line(pos);
        (line, pos - text.line_to_char(line))
    };
    let read = view.read_selection.and_then(|selection| {
        let (side, (a, b)) = (selection.side, selection.range);
        (a != b).then(|| (side, (at(side, a.min(b)), at(side, a.max(b)))))
    });
    let read_caret = view.read_caret().map(|(side, pos)| (side, at(side, pos)));
    let Some(source) = view.new.path.as_deref().and_then(|path| {
        app.editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path))
    }) else {
        return EditState {
            read,
            read_caret,
            ..EditState::default()
        };
    };
    let text = source.text();
    // 光标可能有上万个，只取看得见的：其余选区按位置排好、互不重叠，二分找到第一个够得着的。
    let count = text.line_count();
    let from = text.line_start(lines.start.min(count - 1));
    let to = if lines.end < count {
        text.line_start(lines.end)
    } else {
        text.end_pos()
    };
    let extra = source.extra_selections();
    let skip = extra.partition_point(|s| s.to() < from);
    let extra = extra[skip..].iter().take_while(|s| s.from() <= to);
    let primary = source.selection;
    let span = |s: &Selection| (text.pos_to_line_col(s.from()), text.pos_to_line_col(s.to()));
    let mut selections: Vec<_> = extra.clone().filter(|s| !s.is_empty()).map(span).collect();
    if !primary.is_empty() && primary.to() >= from && primary.from() <= to {
        let primary = span(&primary);
        let at = selections.partition_point(|s| s.0 < primary.0);
        selections.insert(at, primary);
    }
    EditState {
        editing: view.editing,
        caret: text.pos_to_line_col(primary.head),
        carets: extra
            .filter(|s| (from..=to).contains(&s.head))
            .map(|s| text.pos_to_line_col(s.head))
            .collect(),
        selections,
        read,
        read_caret,
        searched: (app.layout.search.h > 0.0 && app.search.document() == Some(source.id))
            .then_some(source.id),
    }
}

/// 右侧显示行 `rows` 覆盖的文档行（左闭右开）；折起来的一段算它折起来的全部行。
fn new_lines(view: &DiffView, rows: Range<usize>) -> Range<usize> {
    let all = &view.rows[RIGHT];
    let rows = &all[rows.start.min(all.len())..rows.end.min(all.len())];
    let lines = |row: &SideRow| match row {
        SideRow::Line { line, .. } => *line..*line + 1,
        SideRow::Fold { lines, .. } => lines.clone(),
    };
    match (rows.first(), rows.last()) {
        (Some(first), Some(last)) => lines(first).start..lines(last).end,
        _ => 0..0,
    }
}

/// 滚到第 `scroll` 显示行（可以带小数）时，高 `height` 的区域里露出的显示行。
fn visible_rows(scroll: f32, height: f32, line_height: f32) -> Range<usize> {
    let first = scroll.floor() as usize;
    first..first + (height / line_height).ceil() as usize + 1
}

/// 要绘制的一个显示行。
struct DrawnRow {
    /// 哪一侧。
    side: usize,
    /// 这一侧的第几个显示行。
    row: usize,
    /// 顶边。
    y: f32,
    /// 单栏时没改的行还要显示左侧的行号：左侧的行号（从 0 起）。
    old_line: Option<usize>,
}

/// 排出这一帧要绘制的显示行：并排时两侧各自滚到自己的位置，单栏时一列。
fn drawn_rows(
    view: &DiffView,
    body: Rect,
    scroll: Scroll,
    pos: f32,
    line_height: f32,
) -> Vec<DrawnRow> {
    if !view.is_unified() {
        let side_rows = [LEFT, RIGHT].into_iter().flat_map(|side| {
            let rows = visible_rows(scroll.rows[side], body.h, line_height);
            (rows.start..rows.end.min(view.rows[side].len())).map(move |row| DrawnRow {
                side,
                row,
                y: body.y + (row as f32 - scroll.rows[side]) * line_height,
                old_line: None,
            })
        });
        return side_rows.collect();
    }
    visible_rows(pos, body.h, line_height)
        .filter_map(|index| {
            let (side, row) = view.unified_row(index)?;
            let unchanged = matches!(view.rows[side][row], SideRow::Line { hunk: None, .. });
            let old_line = unchanged
                .then(|| view.line_at(LEFT, view.row_at(LEFT, index as f32) as usize))
                .flatten();
            Some(DrawnRow {
                side,
                row,
                y: body.y + (index as f32 - pos) * line_height,
                old_line,
            })
        })
        .collect()
}

/// 绘制两侧的代码与行号、改动的底色和带子、查找结果与选区、光标，以及勾选框和「»」按钮。
#[allow(clippy::too_many_arguments)]
fn draw_body(
    app: &mut App,
    frame: &mut Frame<'_>,
    view: &mut DiffView,
    parts: &DiffParts,
    scroll: Scroll,
    pos: f32,
    edit: &EditState,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let p = app.palette;
    let lh = app.fonts.line_height;
    let body = parts.body;
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let tab_width = app.editor.tab_width;
    // 光标只在编辑区有焦点时绘制，和编辑器一样。
    let carets_shown = edit.editing && app.focus == Focus::Editor;
    let read_caret = edit.read_caret.filter(|_| app.focus == Focus::Editor);
    let unified = view.is_unified();
    // 单栏时的换算表从差异里取出来：绘制途中要修改差异两侧的着色状态。
    let map = view.unified_map();
    // 第 `side` 侧显示行 `row` 的顶边。
    let row_y = |side: usize, row: usize| {
        if unified {
            body.y + (map.pos_of(side, row as f32) - pos) * lh
        } else {
            body.y + (row as f32 - scroll.rows[side]) * lh
        }
    };
    // 这一段改动在单栏里的第一行的顶边，并排时为 `None`。
    let hunk_y =
        |view: &DiffView, h: usize| unified.then(|| body.y + (view.hunk_pos(h) - pos) * lh);
    let drawn = drawn_rows(view, body, scroll, pos, lh);
    let mut chars: Vec<char> = Vec::with_capacity(256);
    let mut tokens = Vec::with_capacity(32);
    let mut number = String::new();
    let view_wrap = view.wrap();

    draw_fade(app, frame, view, parts, &row_y);

    // 文字区和中间一栏之间的分隔线，先绘制：改动的几行铺的底色盖住它，一段改动左右连成一片，
    // 不被竖线切开。
    // 各自绘制到这一侧最后一行的底下，不在下面的空白里一直拉到底。
    let rule = p.separator.alpha(p.separator.a * 0.7);
    for (side, x) in [
        (LEFT, parts.panes[LEFT].text.right()),
        (RIGHT, parts.panes[RIGHT].text.x - 1.0),
    ] {
        if unified && side == LEFT {
            continue;
        }
        let end = row_y(side, view.rows[side].len()).clamp(body.y, body.bottom());
        frame.rect(Rect::new(x, body.y, 1.0, end - body.y), rule);
    }

    // 同一行的各段共用字符、分段和着色，换到另一行时才重新取。
    let mut loaded = None;
    let mut breaks = Breaks::default();
    let mut prefix = std::cell::OnceCell::new();
    for &DrawnRow {
        side,
        row,
        y,
        old_line,
    } in &drawn
    {
        let pane = parts.panes[side];
        let band = Rect::new(pane.rect.x, y, pane.rect.w, lh);
        let baseline = baseline_in(band, ascent, descent);
        let (line, hunk, part) = match view.rows[side][row] {
            SideRow::Fold { gap, ref lines } => {
                // 折起来的一段：一条淡底（不绘制上下两道细线：这些线在中间一栏会断开），行号
                // 那一栏一对朝外的小箭头表示能展开；悬停时底色加深。
                let hovered = app.hover == Hover::Diff(DiffTarget::Fold(gap));
                let fill = if hovered {
                    p.hover
                } else {
                    p.hover.alpha(p.hover.a * 0.6)
                };
                frame.rect_clipped(band, fill, 0.0, body);
                let icon = (
                    pane.gutter.right() - DIFF_GUTTER_PAD - app.cell / 2.0,
                    y + lh / 2.0,
                );
                let color = if hovered { p.text_muted } else { p.text_dim };
                icons::up_down(frame, icon, color, pane.gutter);
                let label = t!("editor-diff-unchanged-lines", count = lines.len());
                let x = pane.text.x + DIFF_TEXT_PAD;
                ui_text_small(frame, app, x, band, &label, color, pane.text);
                hits.push((band.intersect(body), DiffTarget::Fold(gap)));
                continue;
            }
            SideRow::Line { line, hunk, part } => (line, hunk, part),
        };

        // 单栏时左右两侧的行上下排开，要靠颜色区分：删掉的和新增的各用各的颜色。
        let row_shade = |view: &DiffView, h: usize| {
            if unified {
                kind_shade(
                    p,
                    if side == LEFT {
                        HunkKind::Deleted
                    } else {
                        HunkKind::Added
                    },
                )
            } else {
                hunk_shade(p, &view.hunks[h])
            }
        };
        let shade = hunk.map(|h| row_shade(view, h));
        if let Some(shade) = shade {
            frame.rect_clipped(band, shade.line, 0.0, body);
            if unified && part == 0 {
                let sign = if side == LEFT { "−" } else { "+" };
                frame.text(
                    FontKind::Mono,
                    app.fonts.code,
                    parts.checks.right(),
                    baseline,
                    sign,
                    p.gutter_text,
                    band,
                );
            }
        }
        if let Some(line) = old_line.filter(|_| part == 0) {
            number.clear();
            let _ = write!(number, "{}", line + 1);
            let gutter = parts.panes[LEFT].gutter;
            let number_x =
                gutter.right() - DIFF_GUTTER_PAD - number.chars().count() as f32 * app.cell;
            frame.text(
                FontKind::Mono,
                app.fonts.code,
                number_x,
                baseline,
                &number,
                p.gutter_text,
                gutter,
            );
        }

        // 行号靠中间的带子一边右对齐；后续各段不绘制行号。
        if part == 0 {
            number.clear();
            let _ = write!(number, "{}", line + 1);
            let number_x =
                pane.gutter.right() - DIFF_GUTTER_PAD - number.chars().count() as f32 * app.cell;
            frame.text(
                FontKind::Mono,
                app.fonts.code,
                number_x,
                baseline,
                &number,
                p.gutter_text,
                pane.gutter,
            );
        }

        let text_side = if side == LEFT {
            &mut view.old
        } else {
            &mut view.new
        };
        if loaded != Some((side, line)) {
            loaded = Some((side, line));
            line_chars(text_side, line, &mut chars);
            breaks = match view_wrap {
                Some((width, tab)) => wrap::breaks(&chars, width, tab),
                None => Breaks::default(),
            };
            prefix = std::cell::OnceCell::new();
            let state = text_side
                .highlighter
                .state_at(&text_side.text, line, HIGHLIGHT_BUDGET);
            app.highlight_pending |= state.is_none();
            let language = text_side.highlighter.language();
            tokens.clear();
            let _ = winc_syntax::highlight_line(
                language,
                &chars,
                state.unwrap_or_default(),
                &mut tokens,
            );
        }
        // 这一行各字符之前的格数用到时算一次：改了的字、查找结果、选区、光标都按列查表。
        // 这一段之外的列夹到这一段的两端，绘制出来宽度为 0。
        let range = breaks.range(part.min(breaks.rows() - 1), chars.len());
        let last = range.end == chars.len();
        let cells = |col: usize| {
            prefix.get_or_init(|| text::prefix_cells(&chars, tab_width))[col.min(chars.len())]
        };
        let lead = breaks.indent_of(part) as f32 - cells(range.start) as f32;
        let at = |col: usize| cells(col.clamp(range.start, range.end)) as f32 + lead;
        let origin_x = pane.text.x + DIFF_TEXT_PAD - scroll.x;

        // 改了的字：行首的缩进和只有空白的地方不标。
        if let (Some(h), Some(shade)) = (hunk, shade)
            && let Some(marks) = view.inline[h].as_ref()
        {
            let hunk = &view.hunks[h];
            let (lines, start) = if side == LEFT {
                (&marks.old, hunk.old.start)
            } else {
                (&marks.new, hunk.new.start)
            };
            let word = shade.word;
            for mark in lines.get(line - start).into_iter().flatten() {
                let end = mark.end.min(chars.len());
                let from = (mark.start..end)
                    .find(|&c| !chars[c].is_whitespace())
                    .unwrap_or(end);
                let to = (from..end)
                    .rev()
                    .find(|&c| !chars[c].is_whitespace())
                    .map_or(from, |c| c + 1);
                if from >= to || to <= range.start || from >= range.end {
                    continue;
                }
                let x0 = origin_x + at(from) * app.cell;
                let x1 = origin_x + at(to) * app.cell;
                let rect = Rect::new(x0, y + 2.0, (x1 - x0).max(2.0), lh - 4.0);
                frame.rect_clipped(rect, word, 3.0, pane.text);
            }
        }

        // 右侧：查找条开着时标出右侧文档里的匹配。
        if side == RIGHT && edit.searched.is_some() {
            let start = text_side.text.line_to_char(line);
            let end = start + chars.len();
            let current = app.search.current_match();
            for &(from, to) in app.search.matches_in(start, end.max(start + 1)) {
                let (a, b) = (from.max(start) - start, to.min(end).max(from) - start);
                if b <= range.start || a >= range.end {
                    continue;
                }
                let x0 = origin_x + at(a) * app.cell;
                let x1 = origin_x + at(b) * app.cell;
                let color = if current == Some((from, to)) {
                    p.search_current
                } else {
                    p.search_match
                };
                let rect = Rect::new(x0, y + 1.0, (x1 - x0).max(2.0), lh - 2.0);
                frame.rect_clipped(rect, color, 3.0, pane.text);
            }
        }

        // 选区垫在字下面：光标在右侧时是右侧文档的各个选区，只读的一侧是它自己的选区。
        // 选区按位置排好、互不重叠，二分找到第一个够得着这一行的。
        let read = edit.read.as_ref().filter(|read| read.0 == side);
        let selections = match (side, read) {
            (RIGHT, _) if edit.editing => &edit.selections[..],
            (_, Some(read)) => std::slice::from_ref(&read.1),
            _ => &[],
        };
        let skip = selections.partition_point(|s| s.1.0 < line);
        for &((l0, c0), (l1, c1)) in selections[skip..].iter().take_while(|s| s.0.0 <= line) {
            let from = if line == l0 { c0 } else { 0 };
            let to = if line == l1 { c1 } else { chars.len() + 1 };
            let x0 = origin_x + at(from) * app.cell;
            let x1 = origin_x
                + at(to) * app.cell
                + if to > chars.len() && last {
                    app.cell * 0.5
                } else {
                    0.0
                };
            if x1 > x0 {
                let rect = Rect::new(x0, y, x1 - x0, lh);
                frame.rect_clipped(rect, p.selection, 0.0, pane.text);
            }
        }

        // 整行从行首绘制（Tab 的宽度与它在行内的位置有关），只露出这一段的范围。
        let clip = if breaks.starts.is_empty() {
            pane.text
        } else {
            let from = origin_x + at(range.start) * app.cell;
            let to = origin_x + at(range.end) * app.cell;
            Rect::new(from, pane.text.y, (to - from).max(0.0), pane.text.h).intersect(pane.text)
        };

        // 空白符垫在字下面，和编辑区用同一个设置。
        let whitespace = app.config.render_whitespace.unwrap_or_default();
        if whitespace != RenderWhitespace::None {
            let trailing = trailing_whitespace(&chars);
            for col in range.clone() {
                let Some(&c) = chars.get(col).filter(|c| matches!(c, ' ' | '\t')) else {
                    continue;
                };
                let in_selection = || {
                    let mut near = selections[skip..].iter().take_while(|s| s.0.0 <= line);
                    near.any(|&(start, end)| start <= (line, col) && (line, col) < end)
                };
                if whitespace_shown(whitespace, col, trailing, in_selection) {
                    let x = origin_x + at(col) * app.cell;
                    if x > clip.right() {
                        break;
                    }
                    let width = (at(col + 1) - at(col)) * app.cell;
                    draw_whitespace_mark(app, frame, c, (x, y), width, clip);
                }
            }
        }

        draw_glyphs(
            app,
            frame,
            &chars,
            &tokens,
            origin_x + lead * app.cell,
            baseline,
            tab_width,
            clip,
            &[],
        );

        // 其余光标与主光标一起闪烁；输入法组字只显示在主光标处，候选窗也跟着它。
        if side == RIGHT && carets_shown {
            let skip = edit.carets.partition_point(|c| c.0 < line);
            for &(_, col) in edit.carets[skip..].iter().take_while(|c| c.0 == line) {
                if breaks.row_of(col) == part {
                    let x = origin_x + at(col) * app.cell;
                    caret_bar(app, frame, x, y, pane.text);
                }
            }
            if edit.caret.0 == line && breaks.row_of(edit.caret.1) == part {
                let x = origin_x + at(edit.caret.1) * app.cell;
                app.ime_cursor = Some(draw_caret(app, frame, x, y, baseline, pane.text));
            }
        }
        // 只读的一侧不能输入，不需要输入法的候选窗跟着。
        if let Some((caret_side, (caret_line, col))) = read_caret
            && caret_side == side
            && caret_line == line
            && breaks.row_of(col) == part
        {
            let x = origin_x + at(col) * app.cell;
            caret_bar(app, frame, x, y, pane.text);
        }
    }

    // 只读一侧的光标在文件末尾换行符之后的空行上：绘制在最后一行下面。
    if let Some((side, (line, _))) = read_caret
        && view.row_of(side, line) == view.rows[side].len()
    {
        let y = row_y(side, view.rows[side].len());
        if y >= body.y && y < body.bottom() {
            let pane = parts.panes[side];
            caret_bar(
                app,
                frame,
                pane.text.x + DIFF_TEXT_PAD - scroll.x,
                y,
                pane.text,
            );
        }
    }

    // 光标在文件末尾换行符之后的空行上：那一行没有显示行，绘制在最后一行下面。
    if carets_shown && view.row_of_new(edit.caret.0) == view.row_count() {
        let y = row_y(RIGHT, view.row_count());
        if y >= body.y && y < body.bottom() {
            let pane = parts.panes[RIGHT];
            let band = Rect::new(pane.text.x, y, pane.text.w, lh);
            let x = pane.text.x + DIFF_TEXT_PAD - scroll.x;
            let baseline = baseline_in(band, ascent, descent);
            app.ime_cursor = Some(draw_caret(app, frame, x, y, baseline, pane.text));
        }
    }

    // 一侧整个是空的（新文件、删掉的文件）：整个文件是一处改动，带子没有形状可绘制（一头是
    // 一个点，另一头是整个文件，绘制出来整块填满颜色），空的那一侧也不绘制插入线。文件不存在、
    // 已删除时标题已经写明，留空即可；版本存在但内容为空时标题看不出这一点，中间写「空文件」。
    let empty_side = [
        view.old.text.len_chars() == 0,
        view.new.text.len_chars() == 0,
    ];
    let sides = [&view.old, &view.new];
    for side in [LEFT, RIGHT]
        .into_iter()
        .filter(|&s| !unified && empty_side[s] && !sides[s].missing)
    {
        let note = t!("editor-diff-empty-file");
        let text = parts.panes[side].text;
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, note);
        let row = Rect::new(text.x, body.y + 2.0 * lh, text.w, lh);
        let x = text.x + ((text.w - w) / 2.0).max(DIFF_TEXT_PAD);
        ui_text_small(frame, app, x, row, note, p.text_dim, text);
    }

    // 带子：每段改动从左侧的改动行连到右侧的改动行；一侧没有行时那一侧绘制一道线。
    let ribbon = parts.ribbon;
    let hunks = if view.one_side_empty() {
        &[][..]
    } else {
        &view.hunks[..]
    };
    for (h, hunk) in hunks.iter().enumerate() {
        let (l, r) = (&view.hunk_rows[LEFT][h], &view.hunk_rows[RIGHT][h]);
        let (l0, l1) = (row_y(LEFT, l.start), row_y(LEFT, l.end));
        let (r0, r1) = (row_y(RIGHT, r.start), row_y(RIGHT, r.end));
        if l1.max(r1) < body.y || l0.min(r0) > body.bottom() {
            continue;
        }
        let shade = hunk_shade(p, hunk);
        if !unified {
            draw_ribbon(frame, ribbon, (l0, l1), (r0, r1), shade, body);
        }
        for (side, (y0, y1)) in [(LEFT, (l0, l1)), (RIGHT, (r0, r1))] {
            if y0 == y1 && !unified {
                let pane = parts.panes[side];
                let line = Rect::new(pane.rect.x, y0 - 0.5, pane.rect.w, 1.0);
                frame.rect_clipped(line, shade.edge, 0.0, body);
            }
        }
        if view.focus.is_some_and(|(focus, _)| focus == h) {
            for (side, span) in [(LEFT, (l0, l1)), (RIGHT, (r0, r1))] {
                // 单栏时没有行的一侧不必画插入线，另一侧的行就在那里。
                if !unified || span.1 > span.0 {
                    draw_hunk_focus(app, frame, parts.panes[side].rect, span, body);
                }
            }
        }
    }

    // 勾选框：每段改动右侧第一行（右侧没有行时是插入的那道线）旁边，提交时包含这一处改动。
    // 文件不在提交视图里时不绘制。
    if let Some(entry) = app.diff_entry(view) {
        let checks = parts.checks;
        for (h, rows) in view.hunk_rows[RIGHT].iter().enumerate() {
            let mid = if let Some(y) = hunk_y(view, h) {
                y + lh / 2.0
            } else if rows.is_empty() {
                row_y(RIGHT, rows.start)
            } else {
                row_y(RIGHT, rows.start) + lh / 2.0
            };
            if mid < body.y || mid > body.bottom() {
                continue;
            }
            let mid = mid.max(body.y + 10.0);
            let on = view
                .keys
                .get(h)
                .is_some_and(|&k| app.commit.hunk_on(entry, k, &view.hunks[h].old));
            let c = (checks.x + checks.w / 2.0, mid);
            // 和左边的「»」一套线条图标（`icons::line_checkbox`），悬停时才有底色。
            let hovered = app.hover == Hover::Diff(DiffTarget::Include(h));
            icons::line_checkbox(
                frame,
                c,
                DIFF_BUTTON,
                on,
                hovered.then_some(p.hover),
                p.accent,
                p.text_dim.alpha(0.7),
                body,
            );
            let rect = Rect::new(c.0 - 10.0, c.1 - 10.0, 20.0, 20.0);
            hits.push((rect.intersect(body), DiffTarget::Include(h)));
        }
    }

    // 「»」：每段改动左侧第一行（左侧没有行时是插入的那道线）旁边，用左侧替换右侧这一段（回滚
    // 这一处改动）。右侧不是工作区的文件时不绘制。
    // 左侧整个是空的（新文件）时也不绘制：用左侧替换就是把整个文件清空，这不是一处改动该有的
    // 操作；新文件的回滚在提交视图里进行。
    if view.new.path.is_some() && !empty_side[LEFT] {
        let arrows = parts.arrows;
        for (h, rows) in view.hunk_rows[LEFT].iter().enumerate() {
            let mid = if let Some(y) = hunk_y(view, h) {
                y + lh / 2.0
            } else if rows.is_empty() {
                row_y(LEFT, rows.start)
            } else {
                row_y(LEFT, rows.start) + lh / 2.0
            };
            if mid < body.y || mid > body.bottom() {
                continue;
            }
            // 和右边的勾选框一套线条图标：强调色、一样的笔宽，悬停时才有底色。
            // 贴着顶边的（文件开头插入的）下移半个按钮高度，避免被标题栏裁掉。
            let mid = mid.max(body.y + 10.0);
            let c = (arrows.x + arrows.w / 2.0, mid);
            let hovered = app.hover == Hover::Diff(DiffTarget::Revert(h));
            icons::line_replace(frame, c, hovered.then_some(p.hover), p.accent, body);
            let rect = Rect::new(c.0 - 10.0, c.1 - 10.0, 20.0, 20.0);
            hits.push((rect.intersect(body), DiffTarget::Revert(h)));
        }
    }
}

/// 在 `area` 里绘制一条带子：左边从 `left` 的上下沿、右边到 `right` 的上下沿，中间按 S 形曲线
/// 过渡。
///
/// 逐列绘制：每列一段底色，上下沿各一段和前一列相连的边线。不用抗锯齿的笔画（按像素
/// 算距离，大块曲线每帧要一两毫秒），逐列的矩形在滚动时每帧绘制开销也较小。
fn draw_ribbon(
    frame: &mut Frame<'_>,
    area: Rect,
    left: (f32, f32),
    right: (f32, f32),
    shade: DiffShade,
    clip: Rect,
) {
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let at = |t: f32| {
        let s = ease(t);
        (
            left.0 + (right.0 - left.0) * s,
            left.1 + (right.1 - left.1) * s,
        )
    };
    // 每列半个逻辑像素：高分屏上正好一个物理像素，边线的台阶看不出来。
    const STEP: f32 = 0.5;
    let columns = (area.w / STEP).max(1.0) as usize;
    let mut previous = at(0.0);
    for k in 0..columns {
        let x = area.x + k as f32 * STEP;
        let (top, bottom) = at((k as f32 + 1.0) / columns as f32);
        if bottom > top {
            frame.rect_clipped(Rect::new(x, top, STEP, bottom - top), shade.line, 0.0, clip);
        }
        // 边线：从前一列的位置连到这一列，至少 1 像素高。
        for (from, to) in [(previous.0, top), (previous.1, bottom)] {
            let (y0, y1) = (from.min(to), from.max(to));
            let rect = Rect::new(x, y0 - 0.5, STEP, (y1 - y0).max(1.0));
            frame.rect_clipped(rect, shade.edge, 0.0, clip);
        }
        previous = (top, bottom);
    }
}

/// 在编辑区的滚动条上标出每处改动在右侧的位置（概览标尺的 Git 那一道）。
fn draw_overview(
    app: &mut App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    index: usize,
) {
    let p = app.palette;
    let doc = &app.editor.documents[index];
    let (owner, scroll) = (doc.id.0, doc.scroll_line * app.fonts.line_height);
    let total = view.scroll_rows().max(1);
    // 让出标题栏：轨道从正文上沿开始（绘制时差异已从文档中取出，`App::scroll_track` 认不出）。
    let track = Rect::new(
        app.layout.editor.x,
        parts.body.y,
        app.layout.editor.w,
        parts.body.h,
    );
    let content = editor_content_height(total, track.h, app.fonts.line_height);
    // 与普通文件的概览标尺同一道、同样的颜色；半透明的滑块绘制在上面。一侧整个是空的（新文件、
    // 删掉的文件）时整个文件是一处改动，从头到底的色条说明不了位置，不绘制。
    let ruler = Ruler::new(track, total);
    if !view.one_side_empty() {
        // 按滚动位置标：和滑块用同一把尺。
        for (i, hunk) in view.hunks.iter().enumerate() {
            let span = view.hunk_span(i);
            let rows = span.start.floor() as usize..span.end.ceil() as usize;
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

/// 把这一侧第 `line` 行去掉换行符的内容取进 `chars`，最多 [`MAX_LINE_CHARS`] 个字符。
fn line_chars(side: &DiffSide, line: usize, chars: &mut Vec<char>) {
    chars.clear();
    if line >= side.text.len_lines() {
        return;
    }
    chars.extend(
        side.text
            .line(line)
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .take(MAX_LINE_CHARS),
    );
}
