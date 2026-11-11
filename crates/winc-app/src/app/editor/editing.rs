//! 执行编辑命令，处理剪贴板和光标定位，以及使光标保持在可见范围内的滚动。

use std::collections::HashMap;

use winc_i18n::t;

use super::*;

/// 括号配对缓存的键：文档、版本号和光标位置都不变时，配对结果不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BraceKey {
    /// 缓存对应的文档。
    pub doc: DocumentId,
    /// 缓存对应的文档版本号（`revision`）。
    pub revision: u64,
    /// 缓存对应的光标位置（字符偏移）。
    pub head: usize,
}

/// 返回一行开头 [`MAX_LINE_CHARS`] 个字符占的显示列数。
///
/// 开头全是不含制表符的 ASCII 字符时直接取字节数，无需逐字符测量；
/// 实测一屏数十条超长行逐字符测量约需 1.5 ms。
fn head_cells(line: ropey::RopeSlice<'_>, tab_width: usize) -> usize {
    let Some(text) = line.as_str() else {
        return text::col_to_cell(line, MAX_LINE_CHARS, tab_width);
    };
    let head = &text.as_bytes()[..text.len().min(MAX_LINE_CHARS)];
    if head.is_ascii() && !head.contains(&b'\t') {
        return head.len();
    }
    (text.chars().take(MAX_LINE_CHARS))
        .fold(0, |cell, c| cell + text::char_cells(c, cell, tab_width))
}

impl App {
    /// 返回行号栏的宽度（逻辑像素）：随行数的位数变化，至少按 2 位计算。
    ///
    /// 不显示行号时只保留两侧供折叠箭头、书签和改动标记使用的空白。
    pub(in crate::app) fn gutter_width(&self) -> f32 {
        let digits = self.line_count().max(1).ilog10() as usize + 1;
        let numbers = if self.config.line_numbers == Some(false) {
            0.0
        } else {
            digits.max(2) as f32 * self.cell
        };
        numbers + 2.0 * GUTTER_PADDING + self.blame_width()
    }

    /// 返回自动换行时一个显示行可容纳的单元格数，至少 10 格。
    ///
    /// 可用宽度为文本区宽度减去左侧留白 `TEXT_PAD` 和右侧滚动条 `SCROLLBAR_HIT`。
    /// 下限取自调参：窗口极窄时每个显示行仍能显示若干字符。
    fn wrap_cells(&self) -> usize {
        let room = self.layout.text.w - TEXT_PAD - SCROLLBAR_HIT;
        ((room / self.cell).floor() as usize).max(10)
    }

    /// 文本区宽度是否正在逐帧变化：侧栏或终端面板的动画进行中，或正在拖动两者的分隔条。
    pub(in crate::app) fn text_width_changing(&self) -> bool {
        self.sidebar.motion.is_some()
            || self.terminal.motion.is_some()
            || self.terminal.grow.is_some()
            || matches!(
                self.drag,
                Drag::Splitter { .. } | Drag::TerminalDivider { .. } | Drag::SplitDivider { .. }
            )
    }

    /// 使当前文档按当前设置和宽度断行；仅在宽度、内容或开关变化时才重新计算。
    ///
    /// 重新断行后，屏幕最上方的文字行保持在原位，不随其上方各行段数的变化而移动。
    pub(in crate::app) fn sync_wrap(&mut self) {
        if self.sync_diff_wrap() {
            return;
        }
        let width = (self.config.word_wrap == Some(true)).then(|| self.wrap_cells());
        let tab_width = self.editor.active_tab_width();
        let resizing = self.text_width_changing();
        let visible = self.visible_lines();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        if !doc.shows_text() {
            return;
        }
        let before = doc.wrap_width();
        // 整篇断行的耗时与行数成正比（20 万行约 115 ms），不能在宽度逐帧变化时每帧执行。此时沿用
        // 已有的断行宽度，动画结束或松开分隔条后按最终宽度重新断行一次。
        let width = match (width, before) {
            (Some(_), Some((cells, _))) if resizing => Some(cells),
            _ => width,
        };
        let place = TopRow::of(doc);
        // 设置「超长行完整显示」的变化在此生效：各行的宽度和段数随之变化。
        let whole = doc.show_whole_lines(self.config.whole_lines == Some(true));
        if whole {
            self.line_widths.1.clear();
            self.max_scroll_x = None;
        }
        doc.ensure_wrap(width, tab_width);
        let job = doc.take_wrap_job().map(|job| (doc.id, job));
        // 大文件在后台断行；完成前先断开屏幕附近两屏的显示行，使换行立即生效。
        if doc.wrap_pending() {
            let top = doc.line_of_row(doc.scroll_line.max(0.0) as usize);
            doc.preview_wrap(top, 2 * visible);
        }
        let wrapping = doc.is_wrapping();
        if doc.wrap_width() != before || whole {
            place.restore(doc, wrapping);
            self.sticky_cache = None;
            self.fold_motion = None;
            self.clamp_scroll();
        }
        if let Some((id, job)) = job {
            let wraps = self.wraps.clone();
            self.spawn_blocking_then(UserEvent::Wrapped, move || {
                let result = job.run();
                wraps
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((id, result));
            });
        }
    }

    /// 展开第 `line` 行，此后显示整行；屏幕顶部的文字位置保持不变。
    pub(in crate::app) fn show_whole_line(&mut self, line: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let place = TopRow::of(doc);
        doc.show_whole_line(line);
        let wrapping = doc.is_wrapping();
        place.restore(doc, wrapping);
        self.line_widths.1.remove(&line);
        self.max_scroll_x = None;
        self.sticky_cache = None;
        self.fold_motion = None;
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 采用后台完成的大文件断行结果，屏幕顶部的文字位置保持不变。
    pub(in crate::app) fn poll_wraps(&mut self) {
        let results = std::mem::take(
            &mut *self
                .wraps
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let active = self.editor.active().map(|d| d.id);
        let mut changed = false;
        for (id, result) in results {
            let Some(doc) = self.editor.documents.iter_mut().find(|d| d.id == id) else {
                continue;
            };
            let place = TopRow::of(doc);
            if doc.finish_wrap(result) {
                place.restore(doc, true);
                changed |= Some(id) == active;
            }
        }
        if changed {
            self.scroll_glide = None;
            self.sticky_cache = None;
            self.fold_motion = None;
            self.clamp_scroll();
            self.request_redraw();
        }
    }

    /// 处理自动换行开关的变化：重新断行，视图顶部的文本行保持不动，并在状态栏提示结果。
    pub(in crate::app) fn word_wrap_changed(&mut self) {
        // 正在进行的平滑滚动按断行前的显示行计算，断行后已不再对应原先的内容。
        self.scroll_glide = None;
        self.relayout();
        let message = match (self.config.word_wrap == Some(true), self.editor.active()) {
            (true, Some(doc)) if doc.diff.as_deref().is_some_and(|v| !v.is_wrapping()) => {
                t!("editor-word-wrap-on-large")
            }
            (true, Some(doc)) if doc.diff.is_some() => t!("editor-word-wrap-on"),
            (true, Some(doc)) if doc.wrap_pending() => t!("editor-word-wrap-on-pending"),
            (true, Some(doc)) if !doc.is_wrapping() => t!("editor-word-wrap-on-table"),
            (true, _) => t!("editor-word-wrap-on"),
            (false, _) => t!("editor-word-wrap-off"),
        };
        self.flash(message);
    }

    /// 返回缩略图最上方显示的行和滑块的位置。
    fn minimap_place(&self) -> (usize, Rect) {
        let area = self.layout.minimap;
        let scroll = self.editor.active().map_or(0.0, |doc| doc.scroll_line);
        let shown = (area.h / MINIMAP_ROW).ceil() as usize;
        let first = crate::layout::minimap_first_row(
            self.row_count(),
            shown,
            scroll,
            self.max_scroll_line(),
        );
        let slider = Rect::new(
            area.x,
            area.y + (scroll - first as f64) as f32 * MINIMAP_ROW,
            area.w,
            self.visible_lines() as f32 * MINIMAP_ROW,
        );
        (first, slider)
    }

    /// 返回绘制缩略图所需的状态：最上方显示的行、滑块的位置，以及滑块是否加深。
    pub(crate) fn minimap_view(&self) -> (usize, Rect, bool) {
        let active = self.hover == Hover::Minimap || matches!(self.drag, Drag::Minimap { .. });
        let (first, slider) = self.minimap_place();
        (first, slider, active)
    }

    /// 在缩略图上按下，开始拖动滑块。
    ///
    /// 点在滑块外时视为按在滑块中心：滑块移到指针下方，之后按指针的位移继续拖动。
    pub(in crate::app) fn press_minimap(&mut self, y: f32) {
        let (_, slider) = self.minimap_place();
        let scroll = self.editor.active().map_or(0.0, |doc| doc.scroll_line);
        let from = if y < slider.y || y >= slider.bottom() {
            slider.y + slider.h / 2.0
        } else {
            y
        };
        self.drag = Drag::Minimap { from, scroll };
        self.drag_minimap(y, from, scroll);
    }

    /// 处理缩略图滑块的拖动：滚动位置从按下时的 `scroll` 起，按指针相对按下点 `from` 的位移变化，
    /// 指针不动时滚动位置不变。
    ///
    /// 滑块的可移动距离等于滚动到底时滑块的位置，因此滑块与指针同步移动。
    pub(in crate::app) fn drag_minimap(&mut self, y: f32, from: f32, scroll: f64) {
        let total = self.row_count();
        let shown = (self.layout.minimap.h / MINIMAP_ROW).ceil() as usize;
        let max = self.max_scroll_line();
        let first = crate::layout::minimap_first_row(total, shown, max, max);
        let room = (max - first as f64) as f32 * MINIMAP_ROW;
        if room > 0.0 {
            self.set_minimap_scroll(scroll + f64::from((y - from) / room) * max);
        }
    }

    /// 把当前文档滚动到第 `line` 行（可为小数），不播放平滑滚动。
    fn set_minimap_scroll(&mut self, line: f64) {
        let max = self.max_scroll_line();
        self.scroll_glide = None;
        if let Some(doc) = self.editor.active_mut() {
            doc.scroll_line = line.clamp(0.0, max);
        }
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 返回当前文档的文件行数；没有打开文档时为 0。
    pub(super) fn line_count(&self) -> usize {
        self.editor.active().map_or(0, |d| d.text().line_count())
    }

    /// 返回当前文档的显示行数；已折叠的行不计入。
    pub(in crate::app) fn row_count(&self) -> usize {
        self.editor.active().map_or(0, |d| d.row_count())
    }

    /// 返回文本区可容纳的行数，至少为 1。
    pub(in crate::app) fn visible_lines(&self) -> usize {
        (self.layout.text.h / self.fonts.line_height)
            .floor()
            .max(1.0) as usize
    }

    /// 返回翻页时移动的行数：可见行数减 1，至少为 1。
    fn page_lines(&self) -> usize {
        self.visible_lines().saturating_sub(1).max(1)
    }

    /// 返回 `scroll_line` 的最大值（显示行）：内容能完整显示时不滚动；否则滚动到底时，
    /// 最后一行下方留出四成屏高的空白（[`OVERSCROLL`](crate::layout::OVERSCROLL)）。
    pub(in crate::app) fn max_scroll_line(&self) -> f64 {
        crate::layout::editor_max_scroll(
            self.row_count(),
            self.scroll_track().h,
            self.fonts.line_height,
        )
    }

    /// 返回编辑区内容的总高度（逻辑像素），供滚动条使用：可滚动的距离加一屏高度。
    pub(in crate::app) fn content_height(&self) -> f32 {
        editor_content_height(
            self.row_count(),
            self.scroll_track().h,
            self.fonts.line_height,
        )
    }

    /// 返回编辑区竖直滚动条的轨道，供绘制和点击共用。
    ///
    /// 差异标签让出顶部的标题栏，滑块、概览标尺和点击范围都不延伸到标题栏中的按钮旁边。
    pub(crate) fn scroll_track(&self) -> Rect {
        let editor = self.layout.editor;
        // 日志标签：滚动条位于左侧提交列表的右边缘，详情栏不使用该滚动条。
        if self.log_active() {
            return self.log_parts().list;
        }
        if self.editor.active().is_some_and(|d| d.diff.is_some()) {
            let header = crate::app::diff::DIFF_HEADER;
            Rect::new(
                editor.x,
                editor.y + header,
                editor.w,
                (editor.h - header).max(0.0),
            )
        } else {
            editor
        }
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）对应的文档位置（字符偏移）。
    ///
    /// 位置在最后一个显示行之下时返回文档末尾；没有打开文档时返回 `None`。
    pub(in crate::app) fn pos_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        let t = self.layout.text;
        let row = (f64::from((y - t.y) / self.fonts.line_height) + doc.scroll_line)
            .floor()
            .max(0.0) as usize;

        if row >= doc.row_count() {
            return Some(doc.text().end_pos());
        }
        let cell = ((x - (t.x + TEXT_PAD - doc.scroll_x)) / self.cell)
            .round()
            .max(0.0) as usize;
        if doc.is_wrapping() {
            return Some(doc.pos_in_row(row, cell, self.editor.active_tab_width()));
        }
        let line = doc.line_of_row(row);
        let col = doc.cell_to_col(line, cell, self.editor.active_tab_width());
        let pos = doc.text().line_col_to_pos(line, col);
        Some(winc_core::movement::snap_to_grapheme(doc.text(), pos))
    }

    /// 在编辑区纵向滚动位置从 `from` 变为当前值后调用，距离较远时启动平滑滚动。
    ///
    /// 距离不小于 [`GLIDE_MIN_ROWS`] 行时按 [`ScrollGlide`] 平滑滚动，否则直接到达目标。
    /// 已在滚动时从屏幕上的当前位置继续滚动，不跳回起点。
    /// 动态效果关闭或没有窗口时不播放动画。
    pub(in crate::app) fn glide_scroll(&mut self, from: f64) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (id, to) = (doc.id, doc.scroll_line);
        let shown = match &self.scroll_glide {
            Some(g) if g.doc == id => g.clock.between_rows(g.from, from),
            _ => from,
        };
        if (to - shown).abs() < GLIDE_MIN_ROWS {
            // 距离较近时直接到达；同一文档正在进行的平滑滚动保留，继续滚向新目标。
            if self.scroll_glide.as_ref().is_some_and(|g| g.doc != id) {
                self.scroll_glide = None;
            }
            return;
        }
        self.scroll_glide = self
            .clock(crate::app::animation::SCROLL_GLIDE)
            .map(|clock| ScrollGlide {
                doc: id,
                from: shown,
                clock,
            });
    }

    /// 绘制前调用：正在平滑滚动时，将当前文档的滚动位置替换为本帧的插值位置，并返回目标位置。
    ///
    /// 绘制完成后由 [`Self::end_glide_frame`] 恢复目标位置。其余时刻滚动位置始终等于目标，
    /// 翻页、跳转都按目标计算，连续触发时结果仍然正确。正在折叠或展开时取消平滑滚动，
    /// 因为折叠动画假定滚动位置不变。
    pub(in crate::app) fn begin_glide_frame(&mut self) -> Option<f64> {
        let glide = self.scroll_glide.as_ref()?;
        if self.fold_motion.is_some() {
            self.scroll_glide = None;
            return None;
        }
        let (doc_id, from, clock) = (glide.doc, glide.from, glide.clock);
        let doc = self.editor.active_mut().filter(|d| d.id == doc_id)?;
        let target = doc.scroll_line;
        doc.scroll_line = clock.between_rows(from, target);
        Some(target)
    }

    /// 绘制完一帧后将滚动位置恢复为目标位置，见 [`Self::begin_glide_frame`]。
    pub(in crate::app) fn end_glide_frame(&mut self, target: Option<f64>) {
        if let Some(target) = target
            && let Some(doc) = self.editor.active_mut()
        {
            doc.scroll_line = target;
        }
    }

    /// 是否将键入送往编辑区：焦点位于编辑区，且窗口在前台。
    ///
    /// 返回 `false` 时选区按 `Palette::selection_inactive` 绘制。
    pub(crate) fn editor_focused(&self) -> bool {
        self.focus == Focus::Editor && self.window.focused
    }

    /// 滚动编辑区，使光标保持在可见范围内。
    ///
    /// 上下各留一行余量（顶部有固定行时，留出固定行占用的行数），左右各留 4 个单元格。
    /// 选区的任一端处在折叠的块内（查找、转到行、撤销都可能把光标移到折叠处）时，先展开。
    pub(in crate::app) fn ensure_caret_visible(&mut self) {
        let before = self.editor.active().map(|d| d.scroll_line);
        self.reveal_caret();
        if let Some(before) = before {
            self.glide_scroll(before);
        }
    }

    /// 滚动编辑区使光标可见，不做平滑滚动；供 [`Self::ensure_caret_visible`] 调用。
    fn reveal_caret(&mut self) {
        if let Some(doc) = self.editor.active_mut() {
            let selection = doc.selection;
            doc.reveal(selection.anchor);
            doc.reveal(selection.head);
        }
        let visible = self.visible_lines() as f64;
        let caret_line = self
            .editor
            .active()
            .map_or(0, |d| d.text().line_of(d.selection.head));
        let top = (self.sticky_depth(caret_line) as f64).max(1.0);
        let width = self.layout.text.w;
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();
        let max_scroll = self.max_scroll_line();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
        // 自动换行时按光标所在的显示行计算，且无需横向滚动。
        let row = doc.row_at(doc.selection.head) as f64;

        if row < doc.scroll_line + top {
            doc.scroll_line = (row - top).max(0.0);
        } else if row > doc.scroll_line + visible - 2.0 {
            doc.scroll_line = (row - visible + 2.0).max(0.0);
        }

        doc.scroll_line = doc.scroll_line.clamp(0.0, max_scroll.max(row));

        if doc.is_wrapping() {
            doc.scroll_x = 0.0;
            return;
        }
        // 按列对齐的 CSV 先计算并记录当前版本的列信息，之后的换算和绘制都使用该结果。
        doc.columns(tab_width);
        // 不换行时每行只显示前 `MAX_LINE_CHARS` 个字符；光标超出时滚动到显示部分的末尾，
        // 避免文本区滚动到没有内容的位置。
        let col = col.min(doc.line_limit(line));
        let caret_x = doc.col_to_cell(line, col, tab_width) as f32 * cell;
        let margin = cell * 4.0;
        let right = width - TEXT_PAD - margin - SCROLLBAR_HIT;

        if caret_x < doc.scroll_x + margin {
            doc.scroll_x = (caret_x - margin).max(0.0);
        } else if caret_x > doc.scroll_x + right {
            doc.scroll_x = caret_x - right;
        }
    }

    /// 把当前文档的纵向和横向滚动位置限制在有效范围内。
    pub(in crate::app) fn clamp_scroll(&mut self) {
        let max_scroll = self.max_scroll_line();
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        doc.scroll_line = doc.scroll_line.clamp(0.0, max_scroll);
        if let Some(view) = doc.diff.as_deref() {
            // 差异的两侧共用一个横向滚动位置，按两侧最长的一行、半个编辑区宽的文字区限制。
            let max_x = crate::app::diff_max_scroll_x(self.layout.editor, view, cell, tab_width);
            doc.scroll_x = doc.scroll_x.clamp(0.0, max_x);
            return;
        }
        if doc.is_wrapping() {
            doc.scroll_x = 0.0;
            return;
        }

        let max_x = self.text_max_scroll_x();
        if let Some(doc) = self.editor.active_mut() {
            doc.scroll_x = doc.scroll_x.clamp(0.0, max_x);
        }
    }

    /// 返回当前文档水平滚动条的轨道。
    ///
    /// 差异视图是标题栏以下的正文，两侧共用这一根，右端让出竖直滚动条。其余情况与
    /// [`Self::text_hscroll_track`] 相同。
    pub(crate) fn hscroll_track(&self) -> Rect {
        if self.editor.active().is_some_and(|d| d.diff.is_some()) {
            let body = self.scroll_track();
            return Rect::new(body.x, body.y, (body.w - SCROLLBAR_HIT).max(0.0), body.h);
        }
        self.text_hscroll_track()
    }

    /// 返回编辑区水平滚动条的轨道：文字区，右端让出竖直滚动条。
    pub(crate) fn text_hscroll_track(&self) -> Rect {
        let t = self.layout.text;
        Rect::new(t.x, t.y, (t.w - SCROLLBAR_HIT).max(0.0), t.h)
    }

    /// 返回当前文档文字的最大横向滚动量（逻辑像素）；差异视图和自动换行时为 0。
    ///
    /// 范围只按可见行中最长的一行计算。
    pub(crate) fn text_max_scroll_x(&mut self) -> f32 {
        let width = self.layout.text.w;
        let visible = self.visible_lines() + 1;
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active_mut() else {
            return 0.0;
        };
        if doc.diff.is_some() || doc.is_wrapping() {
            return 0.0;
        }

        let segments =
            doc.folds()
                .segments(doc.scroll_line as usize, visible, doc.text().line_count());
        // 绘制滚动条与限制滚动位置每帧都会调用；可见行和宽度不变时复用上次的结果。
        // 每条超长行需测量前 `MAX_LINE_CHARS` 个字符，逐帧重算开销过大。
        let key = MaxScrollKey {
            doc: doc.id,
            revision: doc.revision,
            segments: segments.clone(),
            tab_width,
            width: width.to_bits(),
            cell: cell.to_bits(),
            epoch: doc.long_lines_epoch(),
        };
        if let Some((cached, max_x)) = &self.max_scroll_x
            && *cached == key
        {
            return *max_x;
        }
        let mut widest = 0;
        // CSV 按列对齐时，行宽按最后一列的起始位置加上该行自身的宽度估算，结果可能偏大，
        // 仅用于限制滚动范围。
        let columns = doc.columns(tab_width);
        let shift = columns
            .as_ref()
            .map_or(0, |c| c.starts.last().copied().unwrap_or(0));

        // 行宽按行缓存：滚动时每帧通常只有少数行新进入可见范围。
        let widths = &mut self.line_widths;
        let widths_key = (doc.id, doc.revision, tab_width);
        if widths.0 != widths_key {
            // 编辑不影响改动之上的行，这些行宽仍然有效：在超长行中输入时无需每次按键重新测量整屏。
            let (id, revision, tab) = widths.0;
            let kept = match (id, tab) == (doc.id, tab_width) {
                true => doc.unchanged_lines_since(revision),
                false => 0,
            };
            widths.1.retain(|&line, _| line < kept);
            widths.0 = widths_key;
        }
        if widths.1.len() > LINE_WIDTHS_MAX {
            widths.1.clear();
        }
        // 截断的行之后有一句说明和「显示整行」，滚动范围为它们留出位置；遇到截断的行时才测量。
        let (renderer, size) = (&self.renderer, self.fonts.ui_small);
        let mut note = None;
        let mut note_cells = || {
            *note.get_or_insert_with(|| {
                let note = t!("editor-line-truncated", count = MAX_LINE_CHARS);
                let text = format!("{note}{}", t!("editor-line-show-whole"));
                let estimate = text.chars().count() as f32 * size;
                let measured = renderer
                    .as_ref()
                    .map(|r| r.measure(FontKind::Ui, size, &text));
                ((measured.unwrap_or(estimate) + 24.0) / cell).ceil() as usize
            })
        };
        for line in segments.into_iter().flatten() {
            let cells = match widths.1.get(&line) {
                Some(&cells) => cells,
                None => {
                    let (cells, exact) = match doc.whole_line_cells(line, tab_width) {
                        Some(width) => width,
                        None => {
                            let content = doc.text().line_prefix(line, MAX_LINE_BYTES);
                            let slice = content.slice();
                            let cut = slice.len_chars() > MAX_LINE_CHARS;
                            let note = if cut { note_cells() } else { 0 };
                            (head_cells(slice, tab_width) + note, true)
                        }
                    };
                    // 展开的行在列索引延伸到行尾之前只有估计的宽度，不缓存。
                    if exact {
                        widths.1.insert(line, cells);
                    }
                    cells
                }
            };
            widest = widest.max(shift + cells);
        }
        let max_x = (widest as f32 * cell - width + TEXT_PAD + cell * 8.0).max(0.0);
        self.max_scroll_x = Some((key, max_x));
        max_x
    }

    /// 把 `text` 写入剪贴板。
    ///
    /// `whole_line` 为 `true` 表示复制的是整行，粘贴到编辑区时插入当前行上方。
    pub(in crate::app) fn set_clipboard(&mut self, text: String, whole_line: bool) {
        self.line_clipboard = whole_line.then(|| text.clone());

        #[cfg(test)]
        {
            self.copied = Some(text);
            self.copied_files.clear();
            self.copied_image = None;
        }
        #[cfg(not(test))]
        if let Some(clipboard) = self.system_clipboard() {
            let _ = clipboard.set_text(text);
        }
    }

    /// 返回剪贴板中的文本；测试中返回 `copied` 的内容，没有时返回 `None`。
    pub(in crate::app) fn clipboard_text(&mut self) -> Option<String> {
        #[cfg(test)]
        {
            self.copied.clone()
        }
        #[cfg(not(test))]
        {
            self.system_clipboard()?.get_text().ok()
        }
    }

    /// 返回系统剪贴板，首次调用时创建；创建失败时返回 `None`。
    #[cfg(not(test))]
    pub(super) fn system_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// 复制选区（`cut` 为 `true` 时剪切）；没有选区时操作光标所在的整行。
    pub(in crate::app) fn copy(&mut self, cut: bool) {
        // 日志标签：详情中有选中文字时复制选中的文字，否则复制选中提交的完整提交号，
        // 效果同点击详情中的提交号。
        if self.log_active() {
            if cut {
                return;
            }
            match self.log_selected_text() {
                Some(text) => {
                    self.set_clipboard(text, false);
                    self.flash(t!("editor-selection-copied"));
                }
                None => self.press_log(crate::app::git::log::LogTarget::CopyOid),
            }
            return;
        }
        // 表格：复制选中的单元格（表格只读，剪切也只复制）。
        if self.table_focused() {
            self.copy_table();
            return;
        }
        // 预览中只复制选中的文字（预览只读，剪切也只复制）。
        if self.markdown_focused() {
            if let Some(text) = self.markdown_selected_text() {
                self.set_clipboard(text, false);
                self.flash(t!("editor-selection-copied"));
            }
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        if doc.has_cursors() {
            self.copy_each(cut);
            return;
        }
        let selection = doc.selection;
        let whole_line = selection.is_empty();
        let range = if whole_line {
            doc.text().line_span(doc.text().line_of(selection.head))
        } else {
            selection.range()
        };

        let Some(mut content) = doc.text_range(range.start, range.end) else {
            self.flash(t!(
                "editor-selection-too-large-to-copy",
                size = COPY_LIMIT >> 20
            ));
            return;
        };

        // 整行复制到文件最后一行（末尾没有换行符）时补上行尾换行符，使粘贴时仍是完整的一行。
        if whole_line && range.end == doc.text().end_pos() {
            content.push_str(doc.line_ending.as_str());
        }

        self.set_clipboard(content, whole_line);

        if cut {
            self.run(if whole_line {
                Command::DeleteLine
            } else {
                Command::Delete { word: false }
            });
        }
    }

    /// 多光标时复制（`cut` 为 `true` 时剪切）：各选区的文字按位置顺序以换行符连接；
    /// 全部为光标时复制各光标所在的整行（同一行只复制一次）。
    ///
    /// 粘贴时若剪贴板的行数等于光标个数，每个光标各插入一行，见 `Document::execute`。
    fn copy_each(&mut self, cut: bool) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (all, _) = doc.selections();
        let text = doc.text();
        let ending = doc.line_ending.as_str();
        let whole_line = all.iter().all(|s| s.is_empty());
        let mut parts = Vec::with_capacity(all.len());
        let mut copied = 0;
        let mut last_line = None;
        for selection in &all {
            let range = if whole_line {
                let line = text.line_of(selection.head);
                if last_line.replace(line) == Some(line) {
                    continue;
                }
                text.line_span(line)
            } else {
                selection.range()
            };
            copied += text.byte_len(range.start, range.end);
            let part = (copied <= COPY_LIMIT)
                .then(|| doc.text_range(range.start, range.end))
                .flatten();
            let Some(mut part) = part else {
                self.flash(t!(
                    "editor-selection-too-large-to-copy",
                    size = COPY_LIMIT >> 20
                ));
                return;
            };
            if whole_line && range.end == text.end_pos() {
                part.push_str(ending);
            }
            parts.push(part);
        }

        // 整行各自带有行尾换行符，直接相连；选中的文字之间补充换行符。
        let content = parts.join(if whole_line { "" } else { ending });
        self.set_clipboard(content, whole_line);

        if cut {
            // 剪切只删除选中的文字：空选区（光标）替换为空串不算修改。
            self.run(if whole_line {
                Command::DeleteLine
            } else {
                Command::Insert(String::new())
            });
        }
    }

    /// 粘贴剪贴板内容。
    ///
    /// 焦点在输入框时插入文本；在编辑区时粘贴文件、文本或图片，在文件树中粘贴文件或图片
    /// （见 `paste` 模块）。终端里没有文本、有图片时，把图片存成临时文件并粘贴路径。
    pub(in crate::app) fn paste(&mut self) {
        match self.focus {
            focus if focus.is_field() => {
                if let Some(content) = self.clipboard_text() {
                    self.edit_field(|field| field.insert(&content));
                }
            }
            Focus::Editor => self.paste_into_editor(),
            Focus::Tree => self.paste_into_tree(),
            Focus::Terminal => self.terminal_paste(),
            _ => {}
        }

        self.request_redraw();
    }

    /// 在编辑区粘贴文本 `content`。
    ///
    /// 没有选区且 `content` 等于最近一次整行复制的内容时，插入到当前行上方；否则在光标处插入。
    /// Markdown 中选中文字后粘贴网址时生成链接，见 [`App::markdown_link_paste`]。
    pub(super) fn paste_text(&mut self, content: String) {
        if let Some(link) = self.markdown_link_paste(&content) {
            self.run(Command::Insert(link));
            return;
        }
        let empty_selection = self
            .editor
            .active()
            .is_some_and(|d| d.selections().0.iter().all(|s| s.is_empty()));
        let whole_line =
            empty_selection && self.line_clipboard.as_deref() == Some(content.as_str());
        self.run(if whole_line {
            Command::InsertLinesAbove(content)
        } else {
            Command::Paste(content)
        });
    }

    /// 对当前文档执行编辑命令 `command`，并更新布局、滚动位置和光标闪烁状态。
    ///
    /// 文档不可编辑时在状态栏提示原因，不执行命令。
    pub(in crate::app) fn run(&mut self, command: Command) {
        let page = self.page_lines();
        let tab_width = self.editor.active_tab_width();
        let lines_before = self.line_count();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };

        // 没有可编辑文本的文档（无法打开的文件、图片、SVG 的预览）不处理编辑命令；Markdown 的
        // 预览可以直接编辑。
        if !doc.edits_text() {
            return;
        }

        if let Some(reason) = doc.edit_blocked(&command) {
            self.flash(reason);
            return;
        }

        // Vim 的替换模式下键入的文字覆盖已有的字符，由 Vim 层执行。
        if self.vim.is_some() && self.vim_overtype(&command) {
            return;
        }
        // Vim 的插入模式记录执行的输入，供次数与点重复重放。
        let input = self.vim_input(&command);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        doc.execute(command, page, tab_width);
        if let Some(input) = input {
            self.vim_typed(input);
        }

        // 行数变化可能改变行号的位数，因此需要重新布局行号栏。
        if self.line_count() != lines_before {
            self.relayout();
        }

        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 把当前文档的选区设为从 `from` 到 `to`（字符偏移），并滚动到光标。
    pub(in crate::app) fn select(&mut self, from: usize, to: usize) {
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = Selection::new(from, to);
            doc.goal_cell = None;
        }

        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }
}

/// 触发平滑滚动的最小滚动距离（显示行）。
///
/// 键入或上下移动光标通常只滚动一两行，此时直接到达；否则连续按方向键时画面会滞后于光标。
const GLIDE_MIN_ROWS: f64 = 2.0;

/// 编辑区平滑滚动的状态。
///
/// 文档的滚动位置已经是目标，绘制时按时钟从 `from` 插值到目标。
#[derive(Debug, Clone)]
pub struct ScrollGlide {
    /// 正在滚动的文档；切换标签后停止平滑滚动。
    pub doc: DocumentId,
    /// 开始时屏幕上的滚动位置（显示行）。
    pub from: f64,
    /// 本次平滑滚动的时钟。
    pub clock: Clock,
}

/// 断行变化前屏幕顶部的位置：（行号，段号，显示行内的小数部分）。
///
/// 重新断行后按同一行、同一段恢复滚动位置，顶部文字不随上方各行段数的变化而移动。
struct TopRow(usize, usize, f64);

impl TopRow {
    /// 记录 `doc` 当前的顶部位置。
    fn of(doc: &winc_editor::Document) -> Self {
        let (line, part) = doc.line_and_part(doc.scroll_line.max(0.0) as usize);
        TopRow(line, part, doc.scroll_line.fract())
    }

    /// 按记录的位置恢复 `doc` 的滚动位置；`wrapping` 为 `true` 时横向滚动归零。
    fn restore(&self, doc: &mut winc_editor::Document, wrapping: bool) {
        let TopRow(line, part, fraction) = *self;
        let part = part.min(doc.rows_of(line).saturating_sub(1));
        doc.scroll_line = (doc.row_of(line) + part) as f64 + fraction;
        if wrapping {
            doc.scroll_x = 0.0;
        }
    }
}

/// [`App::text_max_scroll_x`] 的结果所依据的条件；条件相同时结果不变。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaxScrollKey {
    doc: DocumentId,
    revision: u64,
    /// 可见的各段行（行号，左闭右开）。
    segments: Vec<std::ops::Range<usize>>,
    tab_width: usize,
    /// 文字区宽度和单元格宽度（逻辑像素）的位表示。
    width: u32,
    cell: u32,
    /// 超长行列索引的计数：展开的行的宽度随之变化，见 `Document::long_lines_epoch`。
    epoch: u64,
}

/// [`App::text_max_scroll_x`] 缓存行宽的最大行数（行）；超出时清空。
///
/// 只需覆盖近期滚动经过的行，取值远大于一屏的行数。
const LINE_WIDTHS_MAX: usize = 8192;

/// 各行的显示宽度（显示列），以行号为键；第一项为（文档，版本号，Tab 宽度），变化时清空。
pub(crate) type LineWidths = ((DocumentId, u64, usize), HashMap<usize, usize>);
