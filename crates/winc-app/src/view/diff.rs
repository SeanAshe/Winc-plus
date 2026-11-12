//! 并排差异标签的绘制：标题栏（两侧标题、改动处数、增删行数、上一处、下一处、折叠开关），
//! 左右两栏的代码和行号，中间连接对应改动的带子，滚动条上每处改动的位置，以及两侧文字区底部的横向滚动条。
//!
//! 两侧分别绘制各自的行，行号靠近中间的带子。每段改动按类型使用主题中的柔和色（新增为绿、删除为灰、
//! 修改为蓝），两侧底色与带子同色；一侧没有行时，在该侧的插入位置绘制一道线；改动的字符再加深
//! 一层。并排时「»」和改动勾选框位于带子上，「»」用左侧的一段替换右侧；行勾选框位于行号栏靠近
//! 文字的一侧。折叠的一段同样用带子连接。

mod sticky;

use std::fmt::Write as _;
use std::ops::Range;

use winc_config::RenderWhitespace;
use winc_core::diff::Pick;
use winc_core::text::{self, MAX_LINE_CHARS};
use winc_core::wrap::{self, Breaks};
use winc_core::{Lines, Selection};
use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT, SideRow};
use winc_i18n::t;

use super::editor::{
    HIGHLIGHT_BUDGET, caret_bar, draw_caret, draw_glyphs, draw_modal_caret, draw_whitespace_mark,
    trailing_whitespace, whitespace_shown,
};
use super::*;
use crate::app::diff::{DIFF_HSCROLL, DiffHScroll, DiffPane};
use crate::app::vim::CaretShape;
use crate::app::{DIFF_TEXT_PAD, DiffParts, DiffTarget, Focus, HunkKind, diff_parts};
use crate::icons::Direction;
use crate::layout::{GIT_LANE, Ruler, editor_content_height};

/// 中间一栏勾选框的边长（逻辑像素）；「»」按钮的图标尺寸相同，左右成对。
const DIFF_BUTTON: f32 = 13.0;
/// 行勾选框的边长（逻辑像素）：小于整段改动的勾选框，以示层级。
const LINE_CHECK_SIZE: f32 = 11.0;
/// 不提交的行的底色相对正常底色的不透明度（0 到 1）：仍可辨认为改动，又能与提交的行区分。
const SKIPPED_LINE_ALPHA: f32 = 0.35;
/// 工具栏按钮的边长（逻辑像素）。
const BUTTON: f32 = 24.0;
/// 返回一处改动使用的颜色：仅右侧有的新增为绿，仅左侧有的删除为灰，两侧都有的修改为蓝。
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

/// 绘制「»」替换后的改动在原处的淡出效果：两侧对应的行铺上原来的颜色，中间以带子连接，
/// 不透明度随进度降到 0。
///
/// 该效果先于文字绘制，文字位于其上方。
fn draw_fade(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    row_y: &dyn Fn(usize, usize) -> f32,
) {
    let Some(fade) = &app.diff.fade else {
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

/// 标出跳转到的改动：在该侧边缘绘制强调色竖线，该侧没有行时在插入位置绘制横线；刚跳转时
/// 整段再叠加一层渐隐的强调色。
///
/// `(y0, y1)` 是该段在该侧的上下边，`clip` 是差异的文字区域。
fn draw_hunk_focus(app: &App, frame: &mut Frame<'_>, pane: Rect, (y0, y1): (f32, f32), clip: Rect) {
    /// 当前标记的粗细（逻辑像素）：粗于 1 逻辑像素的改动边线，以便与普通改动区分。
    const MARK: f32 = 3.0;
    /// 高亮开始时强调色的不透明度（0 到 1）：可辨认跳转位置，又不遮挡文字。
    const PULSE_ALPHA: f32 = 0.28;
    let accent = app.palette.accent;
    if y1 > y0 {
        if let Some(pulse) = app.diff.pulse {
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
    /// 左侧、右侧的滚动位置。
    rows: [f64; 2],
    /// 两侧共用的横向滚动（逻辑像素）。
    x: f32,
}

/// 一段未改动内容展开或折叠的过渡在本帧的状态（见 `DiffFoldMotion`）。
#[derive(Clone)]
struct FoldShift {
    /// 该段折叠后隐藏的行在左侧、右侧占用的显示行。
    rows: [Range<usize>; 2],
    /// 该段尚未露出的高度（逻辑像素）。
    hidden: f32,
    /// 行高（逻辑像素）。
    line_height: f32,
}

impl FoldShift {
    /// 返回第 `side` 侧显示行 `row` 向上收起的高度（逻辑像素）；该行尚未完整露出时返回 `None`，
    /// 本帧不绘制。
    fn of(&self, side: usize, row: usize) -> Option<f32> {
        let rows = &self.rows[side];
        if row >= rows.end {
            return Some(self.hidden);
        }
        if row < rows.start {
            return Some(0.0);
        }
        let bottom = (row + 1 - rows.start) as f32 * self.line_height;
        let shown = rows.len() as f32 * self.line_height - self.hidden;
        (bottom <= shown + 0.5).then_some(0.0)
    }

    /// 尚未露出的高度相当于多少个显示行（向上取整）：其后的行上移，底部需要多绘制这些行。
    fn extra_rows(&self) -> usize {
        (self.hidden / self.line_height).ceil() as usize
    }
}

/// 绘制当前标签（第 `index` 个）的差异。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_diff(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    app.sync_diff_source(index);
    // 绘制时需要推进两侧的着色状态，同时读取 `app` 的字体和配色：先从文档中取出差异，绘制后恢复。
    let Some(mut view) = app.editor.documents[index].diff.take() else {
        return;
    };
    let parts = diff_parts(app.layout.editor, &view, app.cell);
    let doc = &app.editor.documents[index];
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    // 滚动位置由两侧共用，各侧分别换算为自己的显示行（见 `DiffView::row_at`）。
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
    // 展开或折叠的过渡：滚动位置改变、该段已不处于展开状态时不再绘制过渡。
    let shift = (app.diff_fold.as_ref())
        .filter(|m| m.doc == doc.id && m.scroll == doc.scroll_line && view.is_open(m.gap))
        .map(|m| FoldShift {
            rows: m.rows.clone(),
            hidden: m.hidden(app.fonts.line_height),
            line_height: app.fonts.line_height,
        });
    let scroll = (scroll, shift.as_ref());

    draw_body(app, frame, &mut view, &parts, scroll, pos, &edit, &mut hits);
    let (scroll, _) = scroll;
    sticky::draw(app, frame, &mut view, &parts, scroll, pos, &mut hits);
    draw_header(app, frame, &view, &parts, pos, &mut hits);
    draw_overview(app, frame, &view, &parts, index);

    app.hits.diff = hits;
    app.editor.documents[index].diff = Some(view);
}

/// 绘制标题栏：两侧标题、改动处数与增删行数，右端是布局切换、折叠开关和导航按钮。
fn draw_header(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    scroll: f64,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let p = app.palette;
    let header = parts.header;
    frame.rect(header, p.editor_bg);
    frame.rect(
        Rect::new(header.x, header.bottom() - 1.0, header.w, 1.0),
        p.separator,
    );

    // 右端从右往左依次为：布局切换、折叠开关、下一个文件与上一个文件（可跨文件导航时）、下一处、
    // 上一处。布局切换和折叠开关按状态更换图标，不绘制按下状态。
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
            // 图标表示点击后切换到的布局。
            DiffTarget::Layout if view.is_unified() => {
                icons::split(frame, c, p.text_muted, rect);
            }
            DiffTarget::Layout => icons::single_column(frame, c, p.text_muted, rect),
            _ if view.collapses() => icons::expand_all(frame, c, p.text_muted, rect),
            _ => icons::collapse_all(frame, c, p.text_muted, rect),
        }
        hits.push((rect, target));
    }

    // 按钮左侧为增删行数，再往左为当前处序号和总处数。
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

    // 两侧的标题分别从各自文字区的左边开始，右侧标题不遮挡统计；只读一侧的标题前显示锁图标。
    // 单栏只有一个文字区，两个标题依次排列，中间以箭头分隔。
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
        // 「HEAD · 7ca683c」「工作区 · 未保存」：前半部分表示版本，用正常字色；
        // 后半部分为补充说明，用淡色。
        let (main, rest) = match title.title.split_once(" · ") {
            Some((main, rest)) => (main, Some(rest)),
            None => (title.title.as_str(), None),
        };
        let mut end = ui_text(frame, app, x, header, main, p.text, clip);
        if let Some(rest) = rest {
            let rest = format!(" · {rest}");
            end = ui_text(frame, app, end, header, &rest, p.text_dim, clip);
        }
        // 文件路径显示在第一个标题之后（单栏显示在最后），过长时从中间截短。
        if side == if view.is_unified() { RIGHT } else { LEFT } {
            let path = view.repo_path.as_deref().map(|path| {
                let root = app.git.root.as_deref();
                let relative = root
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path);
                relative.display().to_string()
            });
            if let Some(path) = path {
                let x = end + 12.0;
                let path = ellipsize(
                    frame,
                    &path,
                    app.fonts.ui_small,
                    (clip.right() - x).max(0.0),
                );
                ui_text_small(frame, app, x, header, &path, p.text_dim, clip);
            }
        }
        after = view.is_unified().then_some(end);
    }
}

/// 一个选区的起点和终点（行，列），起点在前。
type Span = ((usize, usize), (usize, usize));

/// 右侧的光标和选区，以右侧的行号和列（字符）表示。
#[derive(Default)]
struct EditState {
    /// 光标是否在右侧。
    editing: bool,
    /// 主光标所在的（行，列）；输入法候选窗跟随主光标。
    caret: (usize, usize),
    /// 其余光标所在的（行，列），仅含位于可见行内的光标，按位置排序。
    carets: Vec<(usize, usize)>,
    /// 与可见行相交的非空选区，按位置排序。
    selections: Vec<Span>,
    /// 只读一侧的选区：（所在侧，选区）；没有选中内容时为 `None`。
    read: Option<(usize, Span)>,
    /// 只读一侧的光标：（所在侧，（行，列））。
    read_caret: Option<(usize, (usize, usize))>,
    /// Vim 模式的光标：（所在侧，（行，列），形状）；有时该侧不画竖线光标。
    modal: Option<(usize, (usize, usize), CaretShape)>,
    /// 查找条打开时需要标出匹配的一侧（`LEFT` 或 `RIGHT`）；无需标出时为 `None`。
    searched: Option<usize>,
}

/// 读取右侧对应文档的光标和选区；多光标只取右侧文档第 `lines` 行（左闭右开）内的部分。
fn edit_state(app: &App, view: &DiffView, lines: Range<usize>) -> EditState {
    // 只读一侧的选区和光标按该侧的文本换算为（行，列）。
    let at = |side: usize, pos: usize| {
        let text = &view.side(side).text;
        let pos = pos.min(text.len_chars());
        let line = text.char_to_line(pos);
        (line, pos - text.line_to_char(line))
    };
    let snapshot = app
        .editor
        .active()
        .filter(|_| app.layout.search.h > 0.0)
        .and_then(|doc| app.search.snapshot_side(doc));
    let read = view.read_selection.and_then(|selection| {
        let (side, (a, b)) = (selection.side, selection.range);
        if snapshot == Some(side) && app.search.current_match() == Some((a, b)) {
            return None;
        }
        (a != b).then(|| (side, (at(side, a.min(b)), at(side, a.max(b)))))
    });
    let read_caret = view.read_caret().map(|(side, pos)| (side, at(side, pos)));
    let modal = app.vim_diff_caret(view);
    let Some(source) = view.new.path.as_deref().and_then(|path| {
        app.editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path))
    }) else {
        return EditState {
            read,
            read_caret,
            modal: modal.map(|(side, pos, shape)| (side, at(side, pos), shape)),
            searched: snapshot,
            ..EditState::default()
        };
    };
    let text = source.text();
    // 可编辑一侧的光标按该侧对应的文档换算。
    let modal = modal.map(|(side, pos, shape)| match view.editing {
        true => (side, text.pos_to_line_col(pos.min(text.end_pos())), shape),
        false => (side, at(side, pos), shape),
    });
    // 光标可能有上万个，只取可见范围内的：其余选区按位置排序、互不重叠，二分查找第一个相交的选区。
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
        modal,
        searched: (app.layout.search.h > 0.0 && app.search.document() == Some(source.id))
            .then_some(RIGHT),
    }
}

/// 返回右侧显示行 `rows` 覆盖的文档行（左闭右开）；折叠的一段计入其包含的全部行。
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

/// 返回滚动到第 `scroll` 显示行（可以是小数）时，高 `height` 的区域内可见的显示行。
fn visible_rows(scroll: f64, height: f32, line_height: f32) -> Range<usize> {
    let first = scroll.floor() as usize;
    first..first + (height / line_height).ceil() as usize + 1
}

/// 要绘制的一个显示行。
struct DrawnRow {
    /// 所在侧。
    side: usize,
    /// 在该侧的显示行序号（从 0 起）。
    row: usize,
    /// 顶边的 y 坐标（逻辑像素）。
    y: f32,
    /// 左侧的行号（从 0 起）；单栏时未改动的行同时显示左侧行号。
    old_line: Option<usize>,
}

/// 返回本帧要绘制的显示行：并排时两侧分别按各自的滚动位置排列，单栏时排为一列。
///
/// `shift` 为进行中的展开或折叠过渡：尚未露出的行不绘制，其后的行上移，底部相应多绘制几行。
fn drawn_rows(
    view: &DiffView,
    body: Rect,
    (scroll, shift): (Scroll, Option<&FoldShift>),
    pos: f64,
    line_height: f32,
) -> Vec<DrawnRow> {
    let extra = shift.map_or(0, FoldShift::extra_rows);
    let lift = |side: usize, row: usize| shift.map_or(Some(0.0), |s| s.of(side, row));
    if !view.is_unified() {
        let side_rows = [LEFT, RIGHT].into_iter().flat_map(|side| {
            let rows = visible_rows(scroll.rows[side], body.h, line_height);
            let end = (rows.end + extra).min(view.rows[side].len());
            (rows.start..end).filter_map(move |row| {
                let y = body.y + (row as f64 - scroll.rows[side]) as f32 * line_height;
                Some(DrawnRow {
                    side,
                    row,
                    y: y - lift(side, row)?,
                    old_line: None,
                })
            })
        });
        return side_rows.collect();
    }
    let rows = visible_rows(pos, body.h, line_height);
    (rows.start..rows.end + extra)
        .filter_map(|index| {
            let (side, row) = view.unified_row(index)?;
            let old_line = unified_old_line(view, side, row, index as f64);
            Some(DrawnRow {
                side,
                row,
                y: body.y + (index as f64 - pos) as f32 * line_height - lift(side, row)?,
                old_line,
            })
        })
        .collect()
}

/// 单栏中第 `side` 侧显示行 `row`（位于单栏位置 `pos`）未改动时，返回同时显示的左侧行号；
/// 改动行和并排时为 `None`。
fn unified_old_line(view: &DiffView, side: usize, row: usize, pos: f64) -> Option<usize> {
    let unchanged = matches!(
        view.rows[side].get(row),
        Some(SideRow::Line { hunk: None, .. })
    );
    (view.is_unified() && unchanged)
        .then(|| view.line_at(LEFT, view.row_at(LEFT, pos) as usize))
        .flatten()
}

/// 绘制文字区与中间一栏之间的竖向分隔线：每侧从正文顶部画到 `ends` 中该侧的下端（窗口
/// 纵坐标），不超出正文区域。单栏只有右侧一条。
fn draw_pane_rules(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    parts: &DiffParts,
    ends: [f32; 2],
) {
    let p = app.palette;
    let body = parts.body;
    let rule = p.separator.alpha(p.separator.a * 0.7);
    for (side, x) in [
        (LEFT, parts.panes[LEFT].text.right()),
        (RIGHT, parts.panes[RIGHT].text.x - 1.0),
    ] {
        if view.is_unified() && side == LEFT {
            continue;
        }
        let end = ends[side].clamp(body.y, body.bottom());
        frame.rect(Rect::new(x, body.y, 1.0, end - body.y), rule);
    }
}

/// 在 `pane` 的行号栏中靠文字一侧右对齐绘制行号（`line` 从 0 起，显示时加 1）。
fn draw_line_number(
    app: &App,
    frame: &mut Frame<'_>,
    pane: &DiffPane,
    line: usize,
    baseline: f32,
    number: &mut String,
) {
    number.clear();
    let _ = write!(number, "{}", line + 1);
    let x = pane.number_end - number.chars().count() as f32 * app.cell;
    let p = app.palette;
    frame.text(
        FontKind::Mono,
        app.fonts.code,
        x,
        baseline,
        number,
        p.gutter_text,
        pane.gutter,
    );
}

/// 绘制两侧的代码与行号、改动的底色和带子、查找结果与选区、光标，以及勾选框和「»」按钮。
#[allow(clippy::too_many_arguments)]
fn draw_body(
    app: &mut App,
    frame: &mut Frame<'_>,
    view: &mut DiffView,
    parts: &DiffParts,
    (scroll, shift): (Scroll, Option<&FoldShift>),
    pos: f64,
    edit: &EditState,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let p = app.palette;
    let lh = app.fonts.line_height;
    let body = parts.body;
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let tab_width = app.editor.tab_width;
    // 光标只在编辑区有焦点时绘制。
    let carets_shown = edit.editing && app.focus == Focus::Editor;
    // Vim 模式的光标所在的一侧不画竖线光标；空心的光标在编辑区没有焦点时仍然绘制。
    let modal = (edit.modal).filter(|m| app.focus == Focus::Editor || m.2 == CaretShape::Hollow);
    let barred = |side: usize| edit.modal.is_none_or(|m| m.0 != side);
    let read_caret = (edit.read_caret).filter(|c| app.focus == Focus::Editor && barred(c.0));
    let unified = view.is_unified();
    // 预先取出单栏时的换算表：绘制过程中需要修改差异两侧的着色状态。
    let map = view.unified_map();
    // 第 `side` 侧显示行 `row` 的顶边；展开或折叠的过渡期间，变化的行之后的行上移。
    let row_y = |side: usize, row: usize| {
        let y = if unified {
            body.y + (map.pos_of(side, row as f64) - pos) as f32 * lh
        } else {
            body.y + (row as f64 - scroll.rows[side]) as f32 * lh
        };
        y - shift.map_or(0.0, |s| s.of(side, row).unwrap_or(s.hidden))
    };
    // 第 `h` 段改动在单栏中第一行的顶边；并排时为 `None`。
    let hunk_y =
        |view: &DiffView, h: usize| unified.then(|| body.y + (view.hunk_pos(h) - pos) as f32 * lh);
    let drawn = drawn_rows(view, body, (scroll, shift), pos, lh);
    // 各段改动的提交方式；文件不在提交视图里时为空，不绘制任何勾选框。
    let inclusions: Vec<Pick> = app.diff_entry(view).map_or_else(Vec::new, |entry| {
        let all: Vec<(u64, std::ops::Range<usize>)> = (view.keys.iter().zip(&view.hunks))
            .map(|(&key, hunk)| (key, hunk.old.clone()))
            .collect();
        (app.commit).hunk_inclusions(entry, &all)
    });
    let mut chars: Vec<char> = Vec::with_capacity(256);
    let mut tokens = Vec::with_capacity(32);
    let mut number = String::new();
    let view_wrap = view.wrap();

    draw_fade(app, frame, view, parts, &row_y);

    // 先绘制文字区和中间一栏之间的分隔线，使改动行的底色覆盖分隔线，一段改动左右连续，
    // 不被竖线切开。分隔线只绘制到该侧最后一行的下沿，不延伸到下方的空白。
    let ends = [LEFT, RIGHT].map(|side| row_y(side, view.rows[side].len()));
    draw_pane_rules(app, frame, view, parts, ends);

    // 同一行的各段共用字符、分段和着色，切换到另一行时才重新读取。
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
                // 折叠的一段：绘制一条淡色底（不绘制上下边线，边线在中间一栏处会断开），行号栏中
                // 一对朝外的小箭头表示可以展开；悬停时底色加深。
                let hovered = app.hover == Hover::Diff(DiffTarget::Fold(gap));
                let fill = if hovered {
                    p.hover
                } else {
                    p.hover.alpha(p.hover.a * 0.6)
                };
                frame.rect_clipped(band, fill, 0.0, body);
                let icon = (pane.number_end - app.cell / 2.0, y + lh / 2.0);
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

        // 单栏时左右两侧的行上下排列，需要以颜色区分：删除行与新增行使用不同的颜色。
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
        // 只提交部分行的改动中，不提交的行底色变淡。
        let line_skipped = hunk.is_some_and(|h| {
            let start = if side == LEFT {
                view.hunks[h].old.start
            } else {
                view.hunks[h].new.start
            };
            matches!(inclusions.get(h), Some(Pick::Lines(set))
                if (if side == LEFT { &set.old } else { &set.new }).contains(&(line - start)))
        });
        if let Some(shade) = shade {
            let fill = if line_skipped {
                shade.line.alpha(shade.line.a * SKIPPED_LINE_ALPHA)
            } else {
                shade.line
            };
            frame.rect_clipped(band, fill, 0.0, body);
            // 单栏时行勾选框显示在加减号的位置。
            let line_check = hunk
                .and_then(|h| inclusions.get(h))
                .is_some_and(|inclusion| {
                    matches!(inclusion, Pick::Lines(_))
                        || app.hover == Hover::Diff(DiffTarget::IncludeLine(side, line))
                });
            if unified && part == 0 && !line_check {
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
        // 行号在靠近中间带子的一侧右对齐；后续各段不绘制行号。
        if part == 0 {
            if let Some(line) = old_line {
                draw_line_number(app, frame, &parts.panes[LEFT], line, baseline, &mut number);
            }
            draw_line_number(app, frame, &pane, line, baseline, &mut number);
        }
        // 已展开的未改动段：在重新折叠后第一个隐藏的行上绘制收起标记，位于右侧行号左边的留白
        // 中；悬停时颜色加深，点击后重新折叠该段。
        if part == 0
            && side == RIGHT
            && let Ok(index) = view.open_folds.binary_search_by_key(&row, |&(_, at)| at)
        {
            let target = DiffTarget::Refold(view.open_folds[index].0);
            let pad = crate::app::diff::DIFF_GUTTER_PAD;
            let mark = Rect::new(pane.gutter.x, y, pad, lh);
            let hovered = app.hover == Hover::Diff(target);
            let color = if hovered { p.text_muted } else { p.text_dim };
            // 标记略小于留白并偏向左侧，与行号之间留出间隙。
            let center = (mark.x + mark.w / 2.0 - 1.0, y + lh / 2.0);
            icons::chevron(frame, center, 5.0, Direction::Down, color, pane.gutter);
            hits.push((mark.intersect(body), target));
        }

        // 行勾选框：只提交部分行的改动始终显示；其他改动在鼠标移到该格时显示。
        if let (0, Some(inclusion)) = (part, hunk.and_then(|h| inclusions.get(h))) {
            let cell = parts.line_checks[side];
            let rect = Rect::new(cell.x, y, cell.w, lh);
            let hovered = app.hover == Hover::Diff(DiffTarget::IncludeLine(side, line));
            if hovered || matches!(inclusion, Pick::Lines(_)) {
                let on = !line_skipped && !matches!(inclusion, Pick::Skip);
                icons::line_checkbox(
                    frame,
                    (rect.x + rect.w / 2.0, y + lh / 2.0),
                    LINE_CHECK_SIZE,
                    on,
                    None,
                    p.accent,
                    p.text_dim.alpha(0.7),
                    body,
                );
            }
            hits.push((rect.intersect(body), DiffTarget::IncludeLine(side, line)));
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
        // 本行各字符之前的格数在首次使用时计算一次，改动的字符、查找结果、选区、光标均按列查表。
        // 本段之外的列限制到本段的两端，绘制宽度为 0。
        let range = breaks.range(part.min(breaks.rows() - 1), chars.len());
        let last = range.end == chars.len();
        let cells = |col: usize| {
            prefix.get_or_init(|| text::prefix_cells(&chars, tab_width))[col.min(chars.len())]
        };
        let lead = breaks.indent_of(part) as f32 - cells(range.start) as f32;
        let at = |col: usize| cells(col.clamp(range.start, range.end)) as f32 + lead;
        let origin_x = pane.text.x + DIFF_TEXT_PAD - scroll.x;

        // 改动的字符：行首缩进和仅含空白的部分不标记。
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
            let marks = lines.get(line - start).map_or(&[][..], Vec::as_slice);
            let marks = if winc_editor::diff_view::marks_cover_line(&chars, marks) {
                &[][..]
            } else {
                marks
            };
            for mark in marks {
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

        // 单栏布局中未改动的行只按右侧绘制一次；查找左侧时按左侧行号换算字符偏移。
        let search_start = match edit.searched {
            Some(searched) if searched == side => Some(text_side.text.line_to_char(line)),
            Some(LEFT) => old_line.map(|line| view.old.text.line_to_char(line)),
            _ => None,
        };
        if let Some(start) = search_start {
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

        // 选区绘制在文字下方：光标在右侧时绘制右侧文档的各个选区，只读一侧绘制该侧自身的选区。
        // 选区按位置排序、互不重叠，二分查找第一个与本行相交的选区。
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

        // 固定高亮画在选区底色上方，选中的文字仍能看到颜色。不折行时 `range` 是整行，只取
        // 窗口中可见的列：超长的行不必逐帧整行查找。
        let shown = if app.word_highlights.is_empty() {
            0..0
        } else {
            let prefix = prefix.get_or_init(|| text::prefix_cells(&chars, tab_width));
            let cell_at = |x: f32| ((x - origin_x) / app.cell - lead).max(0.0) as usize;
            let (left, right) = (cell_at(pane.text.x), cell_at(pane.text.right()) + 1);
            let first = prefix.partition_point(|&c| c <= left).saturating_sub(1);
            let last = prefix.partition_point(|&c| c <= right);
            first.max(range.start)..last.min(range.end).max(first.max(range.start))
        };
        for (from, to, color) in marked_occurrences(app, &chars, shown) {
            let x0 = origin_x + at(from) * app.cell;
            let x1 = origin_x + at(to) * app.cell;
            let rect = Rect::new(x0, y + 1.0, (x1 - x0).max(2.0), lh - 2.0);
            frame.rect_clipped(rect, color, 3.0, pane.text);
        }

        // 整行从行首绘制（制表符的宽度取决于其在行内的位置），只显示本段的范围。
        let clip = if breaks.starts.is_empty() {
            pane.text
        } else {
            let from = origin_x + at(range.start) * app.cell;
            let to = origin_x + at(range.end) * app.cell;
            Rect::new(from, pane.text.y, (to - from).max(0.0), pane.text.h).intersect(pane.text)
        };

        // 空白符绘制在文字下方，使用与编辑区相同的设置。
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

        // 其余光标与主光标同步闪烁；输入法组字只显示在主光标处，候选窗同样跟随主光标。
        if side == RIGHT && carets_shown {
            let skip = edit.carets.partition_point(|c| c.0 < line);
            for &(_, col) in edit.carets[skip..].iter().take_while(|c| c.0 == line) {
                if breaks.row_of(col) == part {
                    let x = origin_x + at(col) * app.cell;
                    caret_bar(app, frame, x, y, pane.text);
                }
            }
            if edit.caret.0 == line && breaks.row_of(edit.caret.1) == part && barred(RIGHT) {
                let x = origin_x + at(edit.caret.1) * app.cell;
                app.ime.cursor = Some(draw_caret(app, frame, x, y, baseline, pane.text));
            }
        }
        if let Some((caret_side, (caret_line, col), shape)) = modal
            && (caret_side, caret_line, breaks.row_of(col)) == (side, line, part)
        {
            // 行尾与空行上的光标占一格。
            let width = Some((at(col + 1) - at(col)) * app.cell).filter(|width| *width > 0.0);
            let place = (origin_x + at(col) * app.cell, y, width.unwrap_or(app.cell));
            let c = chars.get(col).copied();
            draw_modal_caret(app, frame, place, c, baseline, shape, pane.text);
        }
        // 只读一侧不接受输入，输入法候选窗不跟随该侧的光标。
        if let Some((caret_side, (caret_line, col))) = read_caret
            && caret_side == side
            && caret_line == line
            && breaks.row_of(col) == part
        {
            let x = origin_x + at(col) * app.cell;
            caret_bar(app, frame, x, y, pane.text);
        }
    }

    // 只读一侧的光标位于文件末尾换行符之后的空行时，绘制在最后一行下方。
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

    // 光标位于文件末尾换行符之后的空行时，该行没有对应的显示行，绘制在最后一行下方。
    if carets_shown && barred(RIGHT) && view.row_of_new(edit.caret.0) == view.row_count() {
        let y = row_y(RIGHT, view.row_count());
        if y >= body.y && y < body.bottom() {
            let pane = parts.panes[RIGHT];
            let band = Rect::new(pane.text.x, y, pane.text.w, lh);
            let x = pane.text.x + DIFF_TEXT_PAD - scroll.x;
            let baseline = baseline_in(band, ascent, descent);
            app.ime.cursor = Some(draw_caret(app, frame, x, y, baseline, pane.text));
        }
    }

    // 一侧为空（新文件、已删除的文件）时整个文件是一处改动，带子一端为点、另一端为整个文件，
    // 绘制结果是整块颜色，因此不绘制带子，空的一侧也不绘制插入线。文件不存在或已删除时标题已注明；
    // 版本存在但内容为空时标题无法体现，中间显示「空文件」。
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

    // 带子：每段改动从左侧的改动行连接到右侧的改动行；一侧没有行时该侧绘制一道线。
    let ribbon = parts.ribbon;
    // 折叠的一段在两侧的位置不一定相同：用与折叠行相同的淡色底连接，表示两侧为同一段。
    if !unified {
        let fill = p.hover.alpha(p.hover.a * 0.6);
        let shade = DiffShade {
            line: fill,
            word: fill,
            edge: fill,
            mark: fill,
        };
        for &(left, right) in &view.fold_rows {
            let (l0, r0) = (row_y(LEFT, left), row_y(RIGHT, right));
            if (l0 + lh).max(r0 + lh) < body.y || l0.min(r0) > body.bottom() {
                continue;
            }
            draw_ribbon(frame, ribbon, (l0, l0 + lh), (r0, r0 + lh), shade, body);
        }
    }
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
                // 单栏时另一侧的行显示在同一位置，没有行的一侧无需绘制插入线。
                if !unified || span.1 > span.0 {
                    draw_hunk_focus(app, frame, parts.panes[side].rect, span, body);
                }
            }
        }
    }

    // 改动勾选框与每段改动右侧的第一行（右侧没有行时为插入线）同高，勾选表示提交时包含该处改动。
    // 文件不在提交视图中时不绘制。
    if !inclusions.is_empty() {
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
            let c = (parts.include_x, mid);
            // 与左侧的「»」使用同一套线条图标（`icons::line_checkbox`），仅悬停时显示底色。
            let hovered = app.hover == Hover::Diff(DiffTarget::Include(h));
            match inclusions.get(h) {
                Some(Pick::Lines(_)) => icons::line_checkbox_mixed(
                    frame,
                    c,
                    DIFF_BUTTON,
                    hovered.then_some(p.hover),
                    p.accent,
                    body,
                ),
                inclusion => icons::line_checkbox(
                    frame,
                    c,
                    DIFF_BUTTON,
                    matches!(inclusion, Some(Pick::Apply)),
                    hovered.then_some(p.hover),
                    p.accent,
                    p.text_dim.alpha(0.7),
                    body,
                ),
            }
            let rect = Rect::new(c.0 - 10.0, c.1 - 10.0, 20.0, 20.0);
            hits.push((rect.intersect(body), DiffTarget::Include(h)));
        }
    }

    // 「»」与每段改动左侧的第一行（左侧没有行时为插入线）同高，用左侧替换右侧的该段，即回滚
    // 该处改动。右侧不是工作区的文件时不绘制。左侧为空（新文件）时同样不绘制：替换会清空整个
    // 文件，不属于单处改动的操作；新文件的回滚在提交视图中进行。
    if view.new.path.is_some() && !empty_side[LEFT] {
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
            // 与右侧的勾选框使用同一套线条图标：强调色、相同的笔宽，仅悬停时显示底色。
            // 紧贴顶边的按钮（文件开头的插入）下移半个按钮高度，避免被标题栏裁切。
            let mid = mid.max(body.y + 10.0);
            let c = (parts.revert_x, mid);
            let hovered = app.hover == Hover::Diff(DiffTarget::Revert(h));
            icons::line_replace(frame, c, hovered.then_some(p.hover), p.accent, body);
            let rect = Rect::new(c.0 - 10.0, c.1 - 10.0, 20.0, 20.0);
            hits.push((rect.intersect(body), DiffTarget::Revert(h)));
        }
    }
}

/// 在 `area` 内绘制一条带子：左端为 `left` 的上下沿，右端为 `right` 的上下沿，中间按 S 形曲线
/// 过渡。
///
/// 逐列绘制：每列一段底色，上下沿各一段与前一列相连的边线。抗锯齿笔画按像素计算距离，
/// 大块曲线每帧需要一到两毫秒；逐列矩形在滚动时每帧的绘制开销较小。
pub(in crate::view) fn draw_ribbon(
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
    // 每列宽半个逻辑像素：缩放倍数为 2 时恰为一个物理像素，边线不出现可见的台阶。
    const STEP: f32 = 0.5;
    let columns = (area.w / STEP).max(1.0) as usize;
    let mut previous = at(0.0);
    for k in 0..columns {
        let x = area.x + k as f32 * STEP;
        let (top, bottom) = at((k as f32 + 1.0) / columns as f32);
        if bottom > top {
            frame.rect_clipped(Rect::new(x, top, STEP, bottom - top), shade.line, 0.0, clip);
        }
        // 边线：从前一列的位置连接到本列，高度至少 1 逻辑像素。
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
    let scroll = (doc.scroll_line * f64::from(app.fonts.line_height)) as f32;
    let owner = doc.id.0;
    let total = view.scroll_rows().max(1);
    // 轨道从正文上沿开始，不含标题栏；绘制时差异已从文档中取出，`App::scroll_track` 无法识别。
    let track = Rect::new(
        app.layout.editor.x,
        parts.body.y,
        app.layout.editor.w,
        parts.body.h,
    );
    let content = editor_content_height(total, track.h, app.fonts.line_height);
    // 使用普通文件概览标尺的同一列和颜色，半透明的滑块绘制在上方。一侧为空（新文件、已删除的
    // 文件）时整个文件是一处改动，贯穿全长的色条不提供位置信息，因此不绘制。
    let ruler = Ruler::new(track, total);
    if !view.one_side_empty() {
        // 按滚动位置标记，与滑块使用相同的换算。
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

    // 两侧各一条水平滚动条，共用一个滚动位置。
    let scroll_x = app.editor.documents[index].scroll_x;
    let tab_width = app.editor.active_tab_width();
    let bars = DiffHScroll::of(parts, view, app.cell, tab_width);
    for (side, (area, _)) in DIFF_HSCROLL.into_iter().enumerate() {
        let bar = bars.thumb(side, scroll_x);
        let look = app.scrollbar_look(area, owner, scroll_x, bar.map(|(track, _)| track));
        if let Some((_, thumb)) = bar {
            draw_thumb(frame, p, thumb, look);
        }
    }
}

/// 把该侧第 `line` 行去掉换行符的内容读入 `chars`，最多 [`MAX_LINE_CHARS`] 个字符。
pub(in crate::view) fn line_chars(side: &DiffSide, line: usize, chars: &mut Vec<char>) {
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

#[cfg(test)]
mod tests;
