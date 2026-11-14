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

    // 折叠或展开动画进行中，按展开后的位置与高度绘制：临时替换为该状态的折叠与滚动位置，
    // 绘制后恢复。
    let mut motion = app
        .motion
        .fold_motion
        .take()
        .filter(|m| m.applies_to(&app.editor.documents[index]));
    if let Some(m) = &mut motion {
        let doc = &mut app.editor.documents[index];
        doc.swap_folds(&mut m.open);
        crate::app::scroll::swap_scroll_line(doc, &mut m.scroll);
    }
    let motion_ref = motion.as_ref();
    let hidden = motion_ref.map_or(0.0, |m| m.hidden(app.fonts.line_height));

    // `scroll_line` 以显示行计；折叠隐藏的行不显示，因此屏幕上显示的是文件中若干段连续的行。
    let doc = &mut app.editor.documents[index];
    let first_row = doc.scroll_line.floor().max(0.0) as usize;
    // 动画中变化的行仅收起了一部分，因此下方额外绘制与这些行数量相同的行。
    let rows = (layout.text.h / app.fonts.line_height).ceil() as usize
        + 1
        + motion_ref.map_or(0, |m| m.rows.len());
    let segments = doc
        .folds()
        .segments(first_row, rows, doc.text().line_count());
    // CSV 按列对齐时需要每一列的起始位置。
    let tab_width = doc.tab_width(app.editor.tab_width);
    let columns = doc.columns(tab_width);
    let origin_x = layout.text.x + TEXT_PAD - doc.scroll_x;
    let span = (origin_x, layout.text.right(), app.metrics.cell);
    let read_cap = line_read_cap(doc, columns.is_some(), span, (layout.text.x, tab_width));
    let mut pending = false;
    let states: Vec<Option<LineState>> = segments
        .iter()
        .map(|range| {
            let start = doc.highlight_state(range.start, HIGHLIGHT_BUDGET);
            // 行未读全时，下一行的高亮状态无法由该行顺推：先推算到本段最后一行，绘制时逐行取用。
            if read_cap < MAX_LINE_BYTES {
                doc.highlight_state(range.end.saturating_sub(1), HIGHLIGHT_BUDGET);
            }
            // 精确的状态尚未推算到时（远距离跳转之后），先按附近的内容推算的近似状态着色。
            pending |= start.is_none();
            start.or_else(|| Some(doc.approx_highlight_state(range.start)))
        })
        .collect();
    app.caches.highlight_pending |= pending;
    // 鼠标在行号栏上时，所有可折叠的行都绘制箭头（已折叠的行始终绘制）。
    let foldable: Vec<usize> = if app.pointer.hover == Hover::Gutter {
        let tab_width = doc.tab_width(app.editor.tab_width);
        doc.block_starts(segments.iter().flat_map(|r| r.clone()), tab_width)
    } else {
        Vec::new()
    };
    let brace = matching_brace(app, index);

    // 文件中的合并冲突标记行。
    let conflicts = app.conflicts(index);
    let mut merge_hits = Vec::new();

    // 与 `HEAD` 比较得到的改动绘制在行号右侧；改动卡片打开的一处在文字区铺一层底色。
    app.update_line_changes(index);
    let id = app.editor.documents[index].id;

    // 逐行追溯绘制在行号栏左侧额外的一栏中。
    app.update_blame(id);
    let blame_w = app.blame_width();
    let blame = (blame_w > 0.0).then(|| app.blame_of(id)).flatten();
    // 提交时间的范围：最旧的提交色条最淡，最新的最深。
    let ages = blame.as_ref().map(|b| {
        let times = b.commits.iter().filter(|c| !c.uncommitted).map(|c| c.time);
        (times.clone().min().unwrap_or(0), times.max().unwrap_or(0))
    });
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // 当前行追溯：光标所在行的末尾写一行说明。
    let line_blame = app.inline_blame(now);
    let blame_hover = match app.pointer.hover {
        Hover::Blame(line) => blame.as_ref().and_then(|b| b.lines.get(line).copied()),
        _ => None,
    };

    let mut scratch = Scratch::new();

    {
        let app = &*app;
        // 三栏合并中改动由连接带表示，不再标出与 `HEAD` 的差别。
        let hunks = if app.merge_panes_placed() {
            &[][..]
        } else {
            app.git.changes.hunks(id)
        };
        let doc = &app.editor.documents[index];
        let text = doc.text();
        let tab_width = doc.tab_width(app.editor.tab_width);
        let lh = app.fonts.line_height;
        // 查找对象是分栏右侧的预览时，匹配位置属于预览文字，不在左侧标注。
        let show_matches =
            (layout.search.h > 0.0 || app.vim_highlights()) && !app.markdown_focused();

        let selection = doc.selection;
        // Vim 的可视模式下光标画在选区的光标一端所含的字符上。
        let vim_caret = app.vim_caret();
        let caret = vim_caret.map_or(selection.head, |(pos, _)| pos);
        let (caret_line, caret_col) = text.pos_to_line_col(caret);
        // 多光标：主选区以外、位于可见范围内的选区和光标（行，列）。光标可能有上万个，
        // 只取可见的部分。
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
            offset: (doc.scroll_line - first_row as f64) as f32 * lh,
            motion: motion_ref,
            hidden,
            turning: motion_ref.map(|m| (doc.line_of_row(m.rows.start - 1), m.reveal())),
            text_rect: layout.text,
            gutter: layout.gutter,
            editor: layout.editor,
            origin_x,
            read_cap,
            cell: app.metrics.cell,
            lh,
            tab_width,
            // 缩进参考线使用该文件自身的缩进宽度，与按 Tab 键插入的缩进一致。
            indent_unit,
            active_guide: line::active_guide(text, caret_line, &visible, tab_width, indent_unit),
            columns: columns.as_deref(),
            language: doc.highlighter.language(),
            focused: app.focus == Focus::Editor,
            show_matches,
            // 光标所在的词在屏幕上其他位置的出现。查找条打开时只标注查找结果，避免两种底色混杂；
            // 屏幕上只有该词本身时不标注。
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
            relative: app.vim_relative(),
            yanked: app.vim_yanked(id),
            vim_caret: vim_caret.map(|(_, shape)| shape),
            extra_carets: &extra_carets,
            hunks,
            open: app
                .git
                .changes
                .popup
                .filter(|popup| popup.doc == id)
                .map(|popup| popup.hunk),
            blame: blame.as_deref(),
            blame_w,
            ages,
            now,
            blame_hover,
            line_blame,
        };

        screen.draw_conflict_tints(frame, &conflicts);
        screen.draw_merge_tints(frame);
        screen.draw_open_hunk(frame);
        screen.draw_current_lines(frame);
        screen.draw_selections(frame);

        // 动画中变化的行：位置不变，只显示动画已展开的部分；起始行的箭头随进度旋转。
        let curtain = motion_ref.map(|m| {
            let top = screen.top_of(m.rows.start) as f32;
            let h = m.rows.len() as f32 * lh - hidden;
            Rect::new(layout.editor.x, top, layout.editor.w, h.max(0.0))
        });
        // 每一段都从行首的高亮状态开始绘制：精确状态，或尚未推算到时的近似状态。
        for (range, start) in segments.iter().zip(&states) {
            let mut state = start.unwrap_or(LineState::Normal);
            text.visit_lines(range.start, read_cap, |line, line_start, content| {
                if line >= range.end {
                    return ControlFlow::Break(());
                }
                if read_cap < MAX_LINE_BYTES {
                    state = doc.known_highlight_state(line).unwrap_or(state);
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
    app.hits.merge = merge_hits;

    app.hits.folded = scratch.folded_hits;
    app.hits.long_lines = scratch.long_hits;
    draw_sticky(app, frame, index);
    minimap::draw_minimap(app, frame, index);
    // 滚动条绘制在固定行的上层，使滚动条从上到下是完整的一条。
    draw_editor_scrollbar(app, frame, index);

    if let Some(mut m) = motion {
        let doc = &mut app.editor.documents[index];
        doc.swap_folds(&mut m.open);
        crate::app::scroll::swap_scroll_line(doc, &mut m.scroll);
        app.motion.fold_motion = Some(m);
    }

    if scratch.ime_cursor.is_some() {
        app.ime.cursor = scratch.ime_cursor;
    }
}

/// 一帧里绘制文本区时各行共用的数据。
struct Screen<'a> {
    app: &'a App,
    doc: &'a Document,
    /// 屏幕上显示的各段连续的行（文件中的行号）；`visible` 是从第一段开头到最后一段末尾的范围。
    segments: &'a [Range<usize>],
    visible: Range<usize>,
    /// 第一个显示行，以及该行顶边超出文本区上沿的距离（逻辑像素）。
    first_row: usize,
    offset: f32,
    /// 折叠或展开动画、变化的行尚未显示的高度，以及起始行的箭头（行，展开的比例）。
    motion: Option<&'a FoldMotion>,
    hidden: f32,
    turning: Option<(usize, f32)>,
    text_rect: Rect,
    gutter: Rect,
    editor: Rect,
    /// 每行第 0 个单元格的 x 坐标（已减去横向滚动）。
    origin_x: f32,
    /// 逐行读取的字节上限（字节），见 `line_read_cap`。
    read_cap: usize,
    /// 一个单元格的宽度（逻辑像素）。
    cell: f32,
    lh: f32,
    /// 制表符展开的宽度（单元格）。
    tab_width: usize,
    /// 缩进参考线的间隔（单元格）。
    indent_unit: usize,
    /// 光标所在代码块的参考线（所在的格与覆盖的行范围），加深绘制；见 `line::active_guide`。
    active_guide: Option<(usize, Range<usize>)>,
    /// CSV 按列对齐时各列的位置。
    columns: Option<&'a Columns>,
    language: &'static Language,
    /// 是否绘制光标；仅编辑区有焦点时为真。
    focused: bool,
    /// 是否标注查找结果。
    show_matches: bool,
    occurrences: Option<Occurrences>,
    /// 光标旁的括号和与之配对的括号（字符偏移）。
    brace: Option<(usize, usize)>,
    /// 绘制折叠箭头的行（鼠标在行号栏上时）。
    foldable: &'a [usize],
    /// 主选区，以及位于可见范围内的其余选区和光标（行，列）。
    selection: Selection,
    extra: &'a [Selection],
    caret_line: usize,
    caret_col: usize,
    /// 行号栏是否显示相对行号。
    relative: bool,
    /// Vim 模式下刚被复制、仍在高亮的范围。
    yanked: Option<Selection>,
    /// Vim 模式下光标的形状；使用竖线光标时为 `None`。
    vim_caret: Option<crate::app::vim::CaretShape>,
    extra_carets: &'a [(usize, usize)],
    /// 与 `HEAD` 比较得到的改动，以及改动卡片打开的一处。
    hunks: &'a [LineHunk],
    open: Option<usize>,
    /// 逐行追溯、追溯栏的宽度、最旧和最新的提交时间、当前时间（Unix 秒），以及鼠标所指的
    /// 提交（`commits` 中的位置）。
    blame: Option<&'a Blame>,
    blame_w: f32,
    ages: Option<(i64, i64)>,
    now: i64,
    blame_hover: Option<u32>,
    /// 当前行追溯：光标所在的行和要写在行尾的说明。
    line_blame: Option<(usize, String)>,
}

/// 行尾可点击的区域及其所在的行。
type LineHits = Vec<(Rect, usize)>;

/// 逐行绘制时复用的缓冲区，以及从各行收集的结果。
struct Scratch {
    chars: Vec<char>,
    /// 按列对齐时各字符的单元格位置和所属的列。
    cells: Vec<usize>,
    fields: Vec<usize>,
    tokens: Vec<Token>,
    /// 这一行里要断开连字的列（光标和选区边界），已排序、去重。
    splits: Vec<usize>,
    number: String,
    /// 上一个非空行的缩进（单元格）；空行按该缩进绘制缩进参考线。
    guide_indent: usize,
    /// 折叠行之后的小块及其所在的行，点击小块后展开。
    folded_hits: LineHits,
    /// 截断说明中「显示整行」的位置及其所在的行。
    long_hits: LineHits,
    /// 主光标的位置；输入法候选框跟随该位置。
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
            long_hits: Vec::new(),
            ime_cursor: None,
        }
    }
}

/// 正在绘制的一行。
///
/// 按整行断开的长行只取屏幕上可见的若干段（行内窗口，见 `Document::line_window`）：此时
/// `line_start`、`chars`、`breaks`、`y` 等均以窗口开头为准，`origin`、`first_part` 给出窗口在
/// 整行中的位置。不换行时显示整行的超长行只取可见的列（`Document::long_line_span`），`base`
/// 给出这些字符之前的显示列数。其他行的窗口即整行。
struct Row<'r> {
    line: usize,
    /// 整行行首的字符偏移。
    origin: usize,
    /// `chars` 起止处的字符偏移（末尾为截断后的位置）。
    line_start: usize,
    line_end: usize,
    /// `chars` 的第一段在整行中的段号（从 0 起），以及 `chars` 是否到行尾为止。
    first_part: usize,
    ends: bool,
    /// 只有一段时第 0 个单元格的 x 坐标，以及 `chars` 的第一个字符所在的单元格。
    ///
    /// 只取可见列的超长行以 `chars` 之前最近的制表位为第 0 格：制表符的宽度不变，坐标也不必由
    /// 数千万像素之外的行首算起（`f32` 在该量级上只能精确到几个像素）。其他行即行首和 0。
    x0: f32,
    base: usize,
    /// `chars` 第一段的顶边和基线。
    y: f32,
    baseline: f32,
    /// 行号、书签、折叠箭头在行号栏中的顶边和基线（见 `Screen::draw_line`）。
    gutter_y: f32,
    gutter_baseline: f32,
    slice: RopeSlice<'r>,
    /// 整行的开头部分，用于计算缩进。
    head: RopeSlice<'r>,
    /// 绘制的字符（过长的行只取开头部分），以及是否已截断。
    chars: &'r [char],
    truncated: bool,
    /// 与编辑区相交的段（左闭右开）。
    parts: Range<usize>,
    /// 自动换行时的分段。
    breaks: Breaks,
    /// 按列对齐时各字符的单元格位置、所属的列和分隔符。
    aligned: Option<(&'r [usize], &'r [usize], char)>,
    /// 本行是否已折叠。
    collapsed: bool,
    /// 第 `i` 项是 `chars` 的前 `i` 个字符末尾所在的单元格（含 `base`）；首次使用时计算
    /// （见 `Screen::place`），一行中的标记无论多少只计算一次。
    prefix: std::cell::OnceCell<Vec<usize>>,
}

impl Row<'_> {
    /// 可见各段覆盖的列（行内字符序号，左闭右开）；只有一段时为整行。
    ///
    /// 按整行断开的长行可达数 MB，标记和空白只在这些列内计算。
    fn shown_cols(&self) -> Range<usize> {
        let len = self.chars.len();
        let rows = self.breaks.rows();
        if rows == 1 {
            return 0..len;
        }
        let end = match self.parts.end {
            end if end < rows => self.breaks.start_of(end),
            _ => len,
        };
        let start = self.breaks.start_of(self.parts.start.min(rows - 1));
        start.min(len)..end.min(len)
    }
}

impl Screen<'_> {
    /// 行 `line` 是否显示（没有被折叠隐藏）。
    fn shown(&self, line: usize) -> bool {
        self.segments.iter().any(|r| r.contains(&line))
    }

    /// 显示行 `row` 的顶边，不计动画中收起的高度。
    ///
    /// 按 `f64` 计算：显示行号可超过 2²⁴，`f32` 无法逐一表示。
    fn top_of(&self, row: usize) -> f64 {
        let rows = row as f64 - self.first_row as f64;
        f64::from(self.text_rect.y) + rows * f64::from(self.lh) - f64::from(self.offset)
    }

    /// 行 `line` 第一段的顶边。
    ///
    /// 长行的第一段可在屏幕之外数亿像素处，加上段的偏移之后才能转为 `f32`（见 [`Self::part_y`]）。
    fn line_y(&self, line: usize) -> f64 {
        let row = self.doc.row_of(line);
        let shift = match self.motion {
            Some(m) if row >= m.rows.end => self.hidden,
            _ => 0.0,
        };
        self.top_of(row) - f64::from(shift)
    }

    /// [`Self::line_y`] 转为 `f32`，用于第一段在屏幕附近的行。
    fn row_y(&self, line: usize) -> f32 {
        self.line_y(line) as f32
    }

    /// 行 `line` 第 `part` 段的顶边。
    fn part_y(&self, line: usize, part: usize) -> f32 {
        (self.line_y(line) + part as f64 * f64::from(self.lh)) as f32
    }

    /// 行 `line` 是否属于动画中变化的行。
    fn in_curtain(&self, line: usize) -> bool {
        self.motion
            .is_some_and(|m| m.rows.contains(&self.doc.row_of(line)))
    }

    /// 为三栏合并的结果一栏中连接带对应的各段铺底色，从行号栏延伸到右边缘，与连接带连续。
    ///
    /// 展开着的冲突由 [`Screen::draw_conflict_tints`] 按当前、对方分别上色。
    fn draw_merge_tints(&self, frame: &mut Frame<'_>) {
        let Some((_, aligned)) = self
            .app
            .git
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
        // 冲突在两侧各有一段，底色半透明，只按第一侧绘制，避免叠加后颜色加深。
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

    /// 为合并冲突的当前、对方、共同祖先各铺一种底色；标记行在文字绘制后绘制为标题。
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

    /// 标出改动卡片打开的一处：当前内容铺一层比卡片中原内容更浅的改动底色，使大段改动仍不遮挡
    /// 文字，便于与卡片对照；删除处绘制一道线。
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
                    self.part_y(line, self.doc.rows_of(line))
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
            self.doc.part_of(self.caret_line, self.caret_col),
        );
        let drawn = self.selection.is_empty().then_some(primary);
        if let Some(row) = drawn {
            self.draw_current_line(frame, row);
        }
        let mut previous = drawn;
        let text = self.doc.text();
        // 断行结果按行计算一次：同一行上的多个光标在 `extra` 中相邻，屏幕外的行不计算。
        // 有行内索引的长行不计算整行断点，按光标所在的窗口换算段号。
        let mut breaks = None;
        for s in self.extra.iter().filter(|s| s.is_empty()) {
            let (line, col) = text.pos_to_line_col(s.head);
            if !self.visible.contains(&line) {
                continue;
            }
            if breaks.as_ref().is_none_or(|&(l, _)| l != line) {
                let whole = (!self.doc.is_long_line(line)).then(|| self.doc.line_breaks(line));
                breaks = Some((line, whole));
            }
            let part = match &breaks {
                Some((_, Some(line_breaks))) => line_breaks.row_of(col),
                _ => self.doc.part_of(line, col),
            };
            let row = (line, part);
            if Some(row) != drawn && Some(row) != previous {
                self.draw_current_line(frame, row);
            }
            previous = Some(row);
        }
    }

    /// 为行 `line` 的第 `part` 段铺当前行底色。
    fn draw_current_line(&self, frame: &mut Frame<'_>, (line, part): (usize, usize)) {
        if !self.shown(line) {
            return;
        }
        let editor = self.editor;
        let y = self.part_y(line, part);
        let row = Rect::new(editor.x + 6.0, y, editor.w - 12.0, self.lh);
        frame.rect_clipped(row, self.app.palette.current_line, RADIUS_ROW, editor);
    }

    /// 绘制主选区和其余选区。
    fn draw_selections(&self, frame: &mut Frame<'_>) {
        let row_y = |line: usize| self.row_y(line);
        let line_y = |line: usize| self.line_y(line);
        let selections = std::iter::once(&self.selection).chain(self.extra);
        for &selection in selections.chain(&self.yanked) {
            if selection.is_empty() {
                continue;
            }
            let (app, segments, columns, origin_x) =
                (self.app, self.segments, self.columns, self.origin_x);
            if self.doc.is_wrapping() {
                draw_wrapped_selection(app, frame, selection, segments, origin_x, &line_y);
            } else {
                draw_selection(app, frame, selection, segments, columns, origin_x, &row_y);
            }
        }
    }

    /// 把合并冲突的标记行绘制为标题，点击区域记录到 `hits`。
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

/// 返回 `all`（按位置排序、互不重叠的选区）中与位置范围 `shown` 相交的部分。
///
/// 以二分查找确定两端，避免每帧逐个比较：光标可能有上万个，可见的只有数十个。
pub(super) fn visible_selections<'a>(
    all: &'a [Selection],
    shown: &std::ops::RangeInclusive<usize>,
) -> &'a [Selection] {
    let first = all.partition_point(|s| s.to() < *shown.start());
    let last = first + all[first..].partition_point(|s| s.from() <= *shown.end());
    &all[first..last]
}
