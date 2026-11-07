//! 文本区：当前行、选区、各行的内容（见 `line`、`gutter`）和合并冲突的标题。
//!
//! 折叠或展开动画进行中按展开后的位置与高度绘制。

mod gutter;
mod line;

use winc_core::diff::LineHunk;
use winc_core::text::Columns;
use winc_core::wrap::Breaks;
use winc_editor::Document;
use winc_editor::git::Blame;
use winc_editor::git::merge::Conflict;
use winc_syntax::{Language, Token};

use super::*;
use crate::app::FoldMotion;
use crate::app::git::merge::MergeTarget;

/// 绘制当前文档的文本区，`index` 是它在 `documents` 中的位置。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_text_area(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let layout = app.layout;

    // 折叠或展开动画进行中，按展开后的位置与高度绘制：临时换上该状态的折叠与滚动位置，
    // 绘制完再恢复。
    let mut motion = app
        .fold_motion
        .take()
        .filter(|m| m.applies_to(&app.editor.documents[index]));
    if let Some(m) = &mut motion {
        let doc = &mut app.editor.documents[index];
        doc.swap_folds(&mut m.open);
        std::mem::swap(&mut doc.scroll_line, &mut m.scroll);
    }
    let motion_ref = motion.as_ref();
    let hidden = motion_ref.map_or(0.0, |m| m.hidden(app.fonts.line_height));

    // `scroll_line` 以显示行计；被折叠隐藏的行不显示，因此屏幕上显示的是文件中的若干段连续的行。
    let doc = &mut app.editor.documents[index];
    let first_row = doc.scroll_line.floor().max(0.0) as usize;
    // 动画中变化的那段行只收起了一部分，因此下方多绘制几行。
    let rows = (layout.text.h / app.fonts.line_height).ceil() as usize
        + 1
        + motion_ref.map_or(0, |m| m.rows.len());
    let segments = doc
        .folds()
        .segments(first_row, rows, doc.text().line_count());
    // CSV 按列对齐：取得每一列的起始位置。
    let columns = doc.columns(doc.tab_width(app.editor.tab_width));
    let states: Vec<Option<LineState>> = segments
        .iter()
        .map(|range| doc.highlight_state(range.start, HIGHLIGHT_BUDGET))
        .collect();
    app.highlight_pending |= states.iter().any(Option::is_none);
    // 鼠标在行号栏上时，所有可折叠的行都绘制箭头（已折叠的行始终绘制）。
    let foldable: Vec<usize> = if app.hover == Hover::Gutter {
        let tab_width = doc.tab_width(app.editor.tab_width);
        doc.block_starts(segments.iter().flat_map(|r| r.clone()), tab_width)
    } else {
        Vec::new()
    };
    let brace = matching_brace(app, index);

    // 文件中的合并冲突标记行。
    let conflicts = app.conflicts(index);
    let mut merge_hits = Vec::new();

    // 与 `HEAD` 比较出的改动绘制在行号右边；卡片打开着的那一处在文字区铺一层底色。
    app.update_line_changes(index);
    let id = app.editor.documents[index].id;

    // 逐行追溯绘制在行号栏左边多出来的一栏里。
    app.update_blame(id);
    let blame_w = app.blame_width();
    let blame = (blame_w > 0.0).then(|| app.blame_of(id)).flatten();
    // 新旧的范围：最旧的提交色条最淡，最新的最深。
    let ages = blame.as_ref().map(|b| {
        let times = b.commits.iter().filter(|c| !c.uncommitted).map(|c| c.time);
        (times.clone().min().unwrap_or(0), times.max().unwrap_or(0))
    });
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let blame_hover = match app.hover {
        Hover::Blame(line) => blame.as_ref().and_then(|b| b.lines.get(line).copied()),
        _ => None,
    };

    let mut scratch = Scratch::new();

    {
        let app = &*app;
        // 三栏合并里改动由连接带表示，不再标出与 `HEAD` 的差别。
        let hunks = if app.merge_panes_placed() {
            &[][..]
        } else {
            app.changes.hunks(id)
        };
        let doc = &app.editor.documents[index];
        let text = doc.text();
        let tab_width = doc.tab_width(app.editor.tab_width);
        let lh = app.fonts.line_height;
        // 查找对着分栏右边的预览时，匹配的位置是预览文字里的，不标在左边。
        let show_matches = layout.search.h > 0.0 && !app.markdown_focused();

        let selection = doc.selection;
        let (caret_line, caret_col) = text.pos_to_line_col(selection.head);
        // 多光标：主选区以外、落在可见范围里的选区和光标（行，列）。光标可能有上万个，
        // 只取看得见的。
        let shown_range = {
            let first = segments.first().map_or(0, |r| r.start);
            let end = segments.last().map_or(0, |r| r.end);
            let to = if end < text.line_count() {
                text.line_start(end)
            } else {
                text.end_pos()
            };
            text.line_start(first.min(text.line_count() - 1))..=to
        };
        let extra: Vec<Selection> =
            visible_selections(doc.extra_selections(), &shown_range).to_vec();
        let extra_carets: Vec<(usize, usize)> = extra
            .iter()
            .filter(|s| shown_range.contains(&s.head))
            .map(|s| text.pos_to_line_col(s.head))
            .collect();

        let indent_unit = match doc.indent() {
            Indent::Tabs => tab_width,
            Indent::Spaces(n) => n,
        };
        let visible = segments.first().map_or(0, |r| r.start)..segments.last().map_or(0, |r| r.end);
        let screen = Screen {
            app,
            doc,
            segments: &segments,
            visible: visible.clone(),
            first_row,
            offset: (doc.scroll_line - first_row as f32) * lh,
            motion: motion_ref,
            hidden,
            turning: motion_ref.map(|m| (doc.line_of_row(m.rows.start - 1), m.reveal())),
            text_rect: layout.text,
            gutter: layout.gutter,
            editor: layout.editor,
            origin_x: layout.text.x + TEXT_PAD - doc.scroll_x,
            cell: app.cell,
            lh,
            tab_width,
            // 缩进参考线使用该文件自身的缩进宽度，与按 Tab 键插入的缩进一致。
            indent_unit,
            active_guide: line::active_guide(text, caret_line, &visible, tab_width, indent_unit),
            columns: columns.as_deref(),
            language: doc.highlighter.language(),
            focused: app.focus == Focus::Editor,
            show_matches,
            // 光标所在的词在屏幕上其他位置的出现。查找条打开时只标注查找结果，避免两种底色混在
            // 一起；屏幕上只有该词自己时不标注。
            occurrences: (!show_matches)
                .then(|| Occurrences::of(doc))
                .flatten()
                .filter(|o| o.count(text, &segments) >= 2),
            brace,
            foldable: &foldable,
            selection,
            extra: &extra,
            caret_line,
            caret_col,
            extra_carets: &extra_carets,
            hunks,
            open: app
                .changes
                .popup
                .filter(|popup| popup.doc == id)
                .map(|popup| popup.hunk),
            blame: blame.as_deref(),
            blame_w,
            ages,
            now,
            blame_hover,
        };

        screen.draw_conflict_tints(frame, &conflicts);
        screen.draw_merge_tints(frame);
        screen.draw_open_hunk(frame);
        screen.draw_current_lines(frame);
        screen.draw_selections(frame);

        // 动画中变化的那段行：停在原位，只露出动画已展开的那一截；起始行的箭头随进度转动。
        let curtain = motion_ref.map(|m| {
            let top = screen.top_of(m.rows.start);
            let h = m.rows.len() as f32 * lh - hidden;
            Rect::new(layout.editor.x, top, layout.editor.w, h.max(0.0))
        });
        // 每一段都从缓存的行首高亮状态开始绘制；状态尚未推算完时按普通文本处理。
        for (range, start) in segments.iter().zip(&states) {
            let mut state = start.unwrap_or(LineState::Normal);
            text.visit_lines(range.start, MAX_LINE_BYTES, |line, line_start, content| {
                if line >= range.end {
                    return ControlFlow::Break(());
                }
                let mut draw = |frame: &mut Frame<'_>| {
                    screen.draw_line(frame, &mut scratch, line, line_start, content, &mut state)
                };
                match curtain.filter(|_| screen.in_curtain(line)) {
                    Some(curtain) => frame.within(curtain, draw),
                    None => draw(frame),
                }
                ControlFlow::Continue(())
            });
        }

        screen.draw_conflict_markers(frame, &conflicts, &mut merge_hits);
    }
    app.merge.hits = merge_hits;

    app.folded_hits = scratch.folded_hits;
    draw_sticky(app, frame, index);
    minimap::draw_minimap(app, frame, index);
    // 滚动条绘制在固定行的上层，使滚动条从上到下是完整的一条。
    draw_editor_scrollbar(app, frame, index);

    if let Some(mut m) = motion {
        let doc = &mut app.editor.documents[index];
        doc.swap_folds(&mut m.open);
        std::mem::swap(&mut doc.scroll_line, &mut m.scroll);
        app.fold_motion = Some(m);
    }

    if scratch.ime_cursor.is_some() {
        app.ime_cursor = scratch.ime_cursor;
    }
}

/// 一帧里绘制文本区时各行共用的数据。
struct Screen<'a> {
    app: &'a App,
    doc: &'a Document,
    /// 屏幕上显示的几段连续的行（文件中的行号）；`visible` 是从第一段开头到最后一段末尾。
    segments: &'a [Range<usize>],
    visible: Range<usize>,
    /// 第一个显示行，以及它的顶边超出文本区上沿的距离（逻辑像素）。
    first_row: usize,
    offset: f32,
    /// 折叠或展开动画、变化的几行尚未露出的高度，以及起始行的箭头（行，展开的比例）。
    motion: Option<&'a FoldMotion>,
    hidden: f32,
    turning: Option<(usize, f32)>,
    text_rect: Rect,
    gutter: Rect,
    editor: Rect,
    /// 每行第 0 个单元格的 x 坐标（已减去横向滚动）。
    origin_x: f32,
    /// 一个单元格的宽度（逻辑像素）。
    cell: f32,
    lh: f32,
    /// 制表符展开为几个空格（格）。
    tab_width: usize,
    /// 缩进参考线的间隔（单元格）。
    indent_unit: usize,
    /// 光标所在代码块的参考线（所在的格与覆盖的行范围），加深绘制；见 `line::active_guide`。
    active_guide: Option<(usize, Range<usize>)>,
    /// CSV 按列对齐时各列的位置。
    columns: Option<&'a Columns>,
    language: &'static Language,
    /// 编辑区有焦点时才绘制光标。
    focused: bool,
    /// 是否标注查找结果。
    show_matches: bool,
    occurrences: Option<Occurrences>,
    /// 光标旁的括号和与它配对的括号（字符偏移）。
    brace: Option<(usize, usize)>,
    /// 绘制折叠箭头的行（鼠标在行号栏上时）。
    foldable: &'a [usize],
    /// 主选区，以及落在可见范围里的其余选区和光标（行，列）。
    selection: Selection,
    extra: &'a [Selection],
    caret_line: usize,
    caret_col: usize,
    extra_carets: &'a [(usize, usize)],
    /// 和 `HEAD` 比较出的改动，以及卡片打开着的那一处。
    hunks: &'a [LineHunk],
    open: Option<usize>,
    /// 逐行追溯、它那一栏的宽度、最旧和最新的提交时间、现在的时间（Unix 秒），以及鼠标指着的
    /// 提交（`commits` 中的位置）。
    blame: Option<&'a Blame>,
    blame_w: f32,
    ages: Option<(i64, i64)>,
    now: i64,
    blame_hover: Option<u32>,
}

/// 逐行绘制时复用的缓冲区，以及从各行收集起来的结果。
struct Scratch {
    chars: Vec<char>,
    /// 按列对齐时各字符的单元格位置和所属的列。
    cells: Vec<usize>,
    fields: Vec<usize>,
    tokens: Vec<Token>,
    /// 这一行里要断开连字的列（光标和选区边界），已排序、去重。
    splits: Vec<usize>,
    number: String,
    /// 上一个非空行的缩进（单元格）；空行沿用它绘制缩进参考线。
    guide_indent: usize,
    /// 折叠行之后的小块和它所在的行，点击后展开。
    folded_hits: Vec<(Rect, usize)>,
    /// 主光标的位置，输入法的候选框跟着它。
    ime_cursor: Option<Rect>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            chars: Vec::with_capacity(256),
            cells: Vec::new(),
            fields: Vec::new(),
            tokens: Vec::with_capacity(32),
            splits: Vec::new(),
            number: String::new(),
            guide_indent: 0,
            folded_hits: Vec::new(),
            ime_cursor: None,
        }
    }
}

/// 正在绘制的一行。
struct Row<'r> {
    line: usize,
    /// 行首和（截断后的）行尾的字符偏移。
    line_start: usize,
    line_end: usize,
    /// 第一段的顶边和基线。
    y: f32,
    baseline: f32,
    /// 行号、书签、折叠箭头在行号栏中的顶边和基线（见 `Screen::draw_line`）。
    gutter_y: f32,
    gutter_baseline: f32,
    slice: RopeSlice<'r>,
    /// 绘制的字符（过长的行只取开头部分），以及是否截断了。
    chars: &'r [char],
    truncated: bool,
    /// 自动换行时的分段。
    breaks: Breaks,
    /// 按列对齐时各字符的单元格位置、所属的列和分隔符。
    aligned: Option<(&'r [usize], &'r [usize], char)>,
    /// 这一行是否已折叠。
    collapsed: bool,
    /// 第 `i` 项是前 `i` 个字符占的格数，用到时才算（见 `Screen::place`），一行里标记再多也只
    /// 数一遍。
    prefix: std::cell::OnceCell<Vec<usize>>,
}

impl Screen<'_> {
    /// 行 `line` 是否显示（没有被折叠隐藏）。
    fn shown(&self, line: usize) -> bool {
        self.segments.iter().any(|r| r.contains(&line))
    }

    /// 显示行 `row` 的顶边，不计动画中收起的高度。
    fn top_of(&self, row: usize) -> f32 {
        self.text_rect.y + (row as f32 - self.first_row as f32) * self.lh - self.offset
    }

    /// 行 `line` 第一段的顶边。
    fn row_y(&self, line: usize) -> f32 {
        let row = self.doc.row_of(line);
        let shift = match self.motion {
            Some(m) if row >= m.rows.end => self.hidden,
            _ => 0.0,
        };
        self.top_of(row) - shift
    }

    /// 行 `line` 是否在动画中变化的那段行里。
    fn in_curtain(&self, line: usize) -> bool {
        self.motion
            .is_some_and(|m| m.rows.contains(&self.doc.row_of(line)))
    }

    /// 三栏合并的结果一栏：连接带连到的各段铺底色，从行号栏一直铺到右边，与连接带连成一片。
    ///
    /// 展开着的冲突由 [`Screen::draw_conflict_tints`] 按当前、对方分别上色。
    fn draw_merge_tints(&self, frame: &mut Frame<'_>) {
        let Some((_, aligned)) = self
            .app
            .merge
            .panes
            .as_ref()
            .and_then(|p| p.aligned.as_ref())
        else {
            return;
        };
        if !self.app.merge_panes_placed() {
            return;
        }
        let p = self.app.palette;
        let editor = self.app.layout.editor;
        let clip = Rect::new(editor.x, self.text_rect.y, editor.w, self.text_rect.h);
        // 冲突在两侧都有一段，底色是半透明的，只取第一侧绘制，否则叠加后颜色加深。
        let segments = (aligned.iter().enumerate()).flat_map(|(side, align)| {
            (align.segments().iter()).filter(move |segment| side == 0 || segment.conflict.is_none())
        });
        for segment in segments {
            let shade = match segment.conflict {
                Some(_) if !self.doc.folds().is_collapsed(segment.file.start) => continue,
                Some(_) => git::merge::conflict_shade(p),
                None => git::merge::merged_shade(p, segment),
            };
            let (color, edge) = (shade.line, shade.edge);
            if segment.file.is_empty() {
                let line = segment.file.start;
                if self.visible.contains(&line) && self.shown(line) {
                    let mark = Rect::new(editor.x, self.row_y(line) - 1.0, editor.w, 2.0);
                    frame.rect_clipped(mark, edge, 0.0, clip);
                }
                continue;
            }
            let start = segment.file.start.max(self.visible.start);
            for line in start..segment.file.end.min(self.visible.end) {
                if self.shown(line) {
                    let h = self.doc.rows_of(line) as f32 * self.lh;
                    let band = Rect::new(editor.x, self.row_y(line), editor.w, h);
                    frame.rect_clipped(band, color, 0.0, clip);
                }
            }
        }
    }

    /// 合并冲突：当前、对方、共同祖先各铺一种底色；标记行等文字绘制完以后绘制成标题。
    fn draw_conflict_tints(&self, frame: &mut Frame<'_>, conflicts: &[Conflict]) {
        let p = self.app.palette;
        let text_rect = self.text_rect;
        for conflict in conflicts {
            let lines = conflict.lines();
            for line in lines.start.max(self.visible.start)..lines.end.min(self.visible.end) {
                if let Some(color) =
                    git::merge::tint(p, conflict, line).filter(|_| self.shown(line))
                {
                    let h = self.doc.rows_of(line) as f32 * self.lh;
                    let band = Rect::new(text_rect.x, self.row_y(line), text_rect.w, h);
                    frame.rect_clipped(band, color, 0.0, text_rect);
                }
            }
        }
    }

    /// 卡片打开着的那一处：现在的几行铺一层浅一些的改动底色（比卡片里原来的内容浅，长的一块
    /// 不至于满屏都是颜色），和卡片对照着看；删掉的地方绘制一道线。
    fn draw_open_hunk(&self, frame: &mut Frame<'_>) {
        let Some(h) = self.open.and_then(|i| self.hunks.get(i)) else {
            return;
        };
        let (text, text_rect) = (self.doc.text(), self.text_rect);
        let shade = diff::hunk_shade(self.app.palette, h);
        if h.new.is_empty() {
            let line = h.new.start.min(text.line_count() - 1);
            if self.shown(line) {
                let y = if h.new.start > line {
                    self.row_y(line) + self.doc.rows_of(line) as f32 * self.lh
                } else {
                    self.row_y(line)
                };
                let mark = Rect::new(text_rect.x, y - 1.0, text_rect.w, 2.0);
                frame.rect_clipped(mark, shade.edge, 0.0, text_rect);
            }
        } else {
            for line in h.new.start.max(self.visible.start)..h.new.end.min(self.visible.end) {
                if self.shown(line) {
                    let h = self.doc.rows_of(line) as f32 * self.lh;
                    let band = Rect::new(text_rect.x, self.row_y(line), text_rect.w, h);
                    frame.rect_clipped(band, shade.line.alpha(0.6), 0.0, text_rect);
                }
            }
        }
    }

    /// 为每个光标所在的显示段（自动换行时的一个显示行）绘制当前行底色，同一段只绘制一次。
    ///
    /// 带选区的光标不绘制：整行底色叠在选区下方时，无法分辨选中的范围。其余光标已按位置排序，
    /// 落在同一段的光标相邻，只需与前一个比较；主光标所在的段已经绘制过。
    fn draw_current_lines(&self, frame: &mut Frame<'_>) {
        let primary = (
            self.caret_line,
            self.doc.line_breaks(self.caret_line).row_of(self.caret_col),
        );
        let drawn = self.selection.is_empty().then_some(primary);
        if let Some(row) = drawn {
            self.draw_current_line(frame, row);
        }
        let mut previous = drawn;
        let text = self.doc.text();
        // 断行结果按行计算一次：同一行上的多个光标在 `extra` 中相邻，屏幕外的行不计算。
        let mut breaks = None;
        for s in self.extra.iter().filter(|s| s.is_empty()) {
            let (line, col) = text.pos_to_line_col(s.head);
            if !self.visible.contains(&line) {
                continue;
            }
            if breaks.as_ref().is_none_or(|&(l, _)| l != line) {
                breaks = Some((line, self.doc.line_breaks(line)));
            }
            let Some((_, line_breaks)) = &breaks else {
                continue;
            };
            let row = (line, line_breaks.row_of(col));
            if Some(row) != drawn && Some(row) != previous {
                self.draw_current_line(frame, row);
            }
            previous = Some(row);
        }
    }

    /// 给行 `line` 的第 `part` 段铺一层当前行的底色。
    fn draw_current_line(&self, frame: &mut Frame<'_>, (line, part): (usize, usize)) {
        if !self.shown(line) {
            return;
        }
        let editor = self.editor;
        let row = Rect::new(
            editor.x + 6.0,
            self.row_y(line) + part as f32 * self.lh,
            editor.w - 12.0,
            self.lh,
        );
        frame.rect_clipped(row, self.app.palette.current_line, RADIUS_ROW, editor);
    }

    /// 主选区和其余选区。
    fn draw_selections(&self, frame: &mut Frame<'_>) {
        let row_y = |line: usize| self.row_y(line);
        for &selection in std::iter::once(&self.selection).chain(self.extra) {
            if selection.is_empty() {
                continue;
            }
            let (app, segments, columns, origin_x) =
                (self.app, self.segments, self.columns, self.origin_x);
            if self.doc.is_wrapping() {
                draw_wrapped_selection(app, frame, selection, segments, origin_x, &row_y);
            } else {
                draw_selection(app, frame, selection, segments, columns, origin_x, &row_y);
            }
        }
    }

    /// 合并冲突的标记行绘制成标题，点击区域收进 `hits`。
    fn draw_conflict_markers(
        &self,
        frame: &mut Frame<'_>,
        conflicts: &[Conflict],
        hits: &mut Vec<(Rect, MergeTarget)>,
    ) {
        let text_rect = self.text_rect;
        for (i, conflict) in conflicts.iter().enumerate() {
            let markers = [Some(conflict.start), conflict.base, Some(conflict.middle)];
            for line in markers.into_iter().flatten().chain([conflict.end]) {
                if self.visible.contains(&line) && self.shown(line) {
                    let band = Rect::new(text_rect.x, self.row_y(line), text_rect.w, self.lh);
                    git::merge::draw_marker(
                        self.app,
                        frame,
                        i,
                        conflicts.len(),
                        conflict,
                        line,
                        band,
                        hits,
                    );
                }
            }
        }
    }
}

/// `all`（按位置排好、互不重叠的选区）中与位置范围 `shown` 相交的一段。
///
/// 二分找出两端，不必每帧逐个比较：光标可能有上万个，看得见的只有数十个。
pub(super) fn visible_selections<'a>(
    all: &'a [Selection],
    shown: &std::ops::RangeInclusive<usize>,
) -> &'a [Selection] {
    let first = all.partition_point(|s| s.to() < *shown.start());
    let last = first + all[first..].partition_point(|s| s.from() <= *shown.end());
    &all[first..last]
}
