//! 编辑区的滚动：滚动位置的写入、限制范围、跳转过渡，以及使光标保持可见的滚动。
//!
//! `Document::scroll_line` 只经 [`set_scroll_line`] 和 [`swap_scroll_line`] 写入，由 `xtask` 的
//! 测试检查。跳转过渡期间字段保存目标位置，绘制时换入插值位置（见 [`super::glide`]）。

use super::*;

use winc_editor::Document;

/// 触发平滑滚动的最小滚动距离（显示行）。
///
/// 键入或上下移动光标通常只滚动一两行，此时直接到达；否则连续按方向键时画面会滞后于光标。
pub(crate) const GLIDE_MIN_ROWS: f64 = 2.0;
/// 打字机模式中平滑滚动的最短距离（行）：更近时直接到达，几乎不可见的移动不启动过渡。
pub(crate) const TYPEWRITER_MIN: f64 = 0.05;

/// 编辑区平滑滚动的状态。
///
/// 文档的滚动位置已经是目标，绘制时按时钟从 `from` 插值到目标。
#[derive(Debug, Clone)]
pub struct ScrollGlide {
    /// 正在滚动的文档；切换标签页后停止平滑滚动。
    pub doc: DocumentId,
    /// 开始时屏幕上的滚动位置（显示行）。
    pub from: f64,
    /// 本次平滑滚动的时钟。
    pub clock: Clock,
}

impl ScrollGlide {
    /// 返回目标位置为 `to`（显示行）时此刻屏幕上的滚动位置。
    pub(crate) fn shown(&self, to: f64) -> f64 {
        self.clock.between_rows(self.from, to)
    }
}

/// 把 `doc` 的滚动位置设为 `row`（显示行，可为小数），不限制范围。
///
/// 当前文档需要限制范围时之后调用 [`App::clamp_scroll`]；需要平滑滚动时，写入前记下原位置，
/// 写入后调用 [`App::glide_scroll`]。
pub(crate) fn set_scroll_line(doc: &mut Document, row: f64) {
    doc.scroll_line = row;
}

/// 交换 `doc` 的滚动位置与 `row`（显示行）。
///
/// 供绘制时临时换入另一状态下的滚动位置，绘制后再次调用以换回。
pub(crate) fn swap_scroll_line(doc: &mut Document, row: &mut f64) {
    std::mem::swap(&mut doc.scroll_line, row);
}

/// 断行变化前屏幕顶部的位置：（行号，段号，显示行内的小数部分）。
///
/// 重新断行后按同一行、同一段恢复滚动位置，顶部文字不随上方各行段数的变化而移动。
pub(crate) struct TopRow(usize, usize, f64);

impl TopRow {
    /// 记录 `doc` 当前的顶部位置。
    pub(crate) fn of(doc: &Document) -> Self {
        let (line, part) = doc.line_and_part(doc.scroll_line.max(0.0) as usize);
        TopRow(line, part, doc.scroll_line.fract())
    }

    /// 按记录的位置恢复 `doc` 的滚动位置；`wrapping` 为 `true` 时横向滚动归零。
    pub(crate) fn restore(&self, doc: &mut Document, wrapping: bool) {
        let TopRow(line, part, fraction) = *self;
        let part = part.min(doc.rows_of(line).saturating_sub(1));
        set_scroll_line(doc, (doc.row_of(line) + part) as f64 + fraction);
        if wrapping {
            doc.scroll_x = 0.0;
        }
    }
}

impl App {
    /// 返回 `scroll_line` 的最大值（显示行）：内容能完整显示时不滚动；否则滚动到底时，
    /// 最后一行下方留出四成屏高的空白（[`OVERSCROLL`](crate::layout::OVERSCROLL)）。
    pub(in crate::app) fn max_scroll_line(&self) -> f64 {
        crate::layout::editor_max_scroll(
            self.row_count(),
            self.scroll_track().h,
            self.fonts.line_height,
        )
    }

    /// 返回当前文档纵向滚动位置的上限（显示行）。
    ///
    /// 打字机模式下文末的行同样居中：上限放宽到光标所在行居中时的位置。
    fn scroll_limit(&self) -> f64 {
        let max = self.max_scroll_line();
        let Some(doc) = self.editor.active().filter(|_| self.markdown_typewriter()) else {
            return max;
        };
        let row = doc.row_at(doc.selection.head) as f64;
        max.max(row + 0.5 - self.visible_lines() as f64 / 2.0)
    }

    /// 把当前文档的滚动位置设为 `row`（显示行），不限制范围，不做平滑滚动。
    ///
    /// 进行中的平滑滚动继续滚向新的目标。
    pub(in crate::app) fn set_active_scroll(&mut self, row: f64) {
        if let Some(doc) = self.editor.active_mut() {
            set_scroll_line(doc, row);
        }
    }

    /// 按滚轮或触控板的增量 `dx`、`dy`（逻辑像素，向右、向下滚动为负）滚动文档 `id`；`id` 不是
    /// 当前文档时不滚动。
    pub(in crate::app) fn scroll_editor_by_pixels(&mut self, id: DocumentId, dx: f32, dy: f32) {
        let lh = self.fonts.line_height;
        let Some(doc) = self.editor.active_mut().filter(|d| d.id == id) else {
            return;
        };
        set_scroll_line(doc, doc.scroll_line - f64::from(dy / lh));
        // 展开的超长行可达数千万像素宽，`f32` 在该处的间隔大于一次滚动的距离：没有体现在滚动
        // 位置上的部分留到下一次，不丢弃。
        let x = f64::from(doc.scroll_x) - f64::from(dx) + self.scroll.wheel.carry_x;
        doc.scroll_x = x as f32;
        self.scroll.wheel.carry_x = x - f64::from(doc.scroll_x);
        let wanted = doc.scroll_x;

        self.clamp_scroll();
        // 到达两端后不再累计。
        if self
            .editor
            .active()
            .is_some_and(|doc| doc.scroll_x != wanted)
        {
            self.scroll.wheel.carry_x = 0.0;
        }
    }

    /// 视口以上的内容变高、变矮，文档 `doc` 的滚动位置随之移动了 `delta` 行后调用：正在进行的
    /// 平滑滚动的起点一起移动，屏幕上的内容不跳。
    pub(in crate::app) fn shift_scroll_glide(&mut self, doc: DocumentId, delta: f64) {
        if let Some(glide) = self.scroll.glide.as_mut().filter(|g| g.doc == doc) {
            glide.from += delta;
        }
    }

    /// 在编辑区纵向滚动位置从 `from` 变为当前值后调用，距离较远时启动平滑滚动。
    ///
    /// 距离不小于 [`GLIDE_MIN_ROWS`] 行时按 [`ScrollGlide`] 平滑滚动，否则直接到达目标。
    /// 已在滚动时从屏幕上的当前位置继续滚动，不跳回起点。
    /// 动态效果关闭或没有窗口时不播放动画。
    pub(in crate::app) fn glide_scroll(&mut self, from: f64) {
        self.glide_scroll_over(from, GLIDE_MIN_ROWS);
    }

    /// 同 [`Self::glide_scroll`]，距离不小于 `min` 行时平滑滚动。
    fn glide_scroll_over(&mut self, from: f64, min: f64) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (id, to) = (doc.id, doc.scroll_line);
        let shown = match &self.scroll.glide {
            Some(g) if g.doc == id => g.shown(from),
            _ => from,
        };
        if (to - shown).abs() < min {
            // 距离较近时直接到达；同一文档正在进行的平滑滚动保留，继续滚向新目标。
            if self.scroll.glide.as_ref().is_some_and(|g| g.doc != id) {
                self.scroll.glide = None;
            }
            return;
        }
        self.scroll.glide = self
            .clock(crate::app::animation::SCROLL_GLIDE)
            .map(|clock| ScrollGlide {
                doc: id,
                from: shown,
                clock,
            });
    }

    /// 视口是否正在平滑滚动到当前文档的某一位置（跳转、居中光标）。
    ///
    /// 这种滚动由移动光标引起，光标的目标会随之一帧帧移动；光标动画应跟着目标走，而不是把
    /// 它当成滚轮滚动而直接跳到目标。
    pub(crate) fn caret_scrolls(&self) -> bool {
        self.scroll
            .glide
            .as_ref()
            .is_some_and(|glide| self.editor.active().is_some_and(|doc| doc.id == glide.doc))
    }

    /// 结束编辑区的平滑滚动，滚动位置停在此刻屏幕上显示的位置。
    ///
    /// 用于用户直接控制滚动（按下鼠标、滚轮、触摸、拖动滚动条或缩略图）时：此后的命中测试和
    /// 滚动都从屏幕上看到的位置开始，画面不跳到目标位置。
    pub(in crate::app) fn settle_editor_glide(&mut self) {
        let Some(glide) = self.scroll.glide.take() else {
            return;
        };
        if let Some(doc) = self.editor.active_mut().filter(|d| d.id == glide.doc) {
            let shown = glide.shown(doc.scroll_line);
            set_scroll_line(doc, shown);
        }
    }

    /// 滚动编辑区，使光标保持在可见范围内。
    ///
    /// 上下各留一行余量（顶部有固定行时，留出固定行占用的行数），左右各留 4 个单元格。
    /// 选区的任一端处在折叠的块内（查找、转到行、撤销都可能把光标移到折叠处）时，先展开。
    ///
    /// 打字机模式下光标所在行停在中间，每换一行都平滑滚动，不论距离远近。
    pub(in crate::app) fn ensure_caret_visible(&mut self) {
        let before = self.editor.active().map(|d| d.scroll_line);
        let typewriter = self.markdown_typewriter() && self.scroll.keyed;
        self.reveal_caret();
        if let Some(before) = before {
            let min = if typewriter {
                TYPEWRITER_MIN
            } else {
                GLIDE_MIN_ROWS
            };
            self.glide_scroll_over(before, min);
        }
    }

    /// 把光标所在行滚到编辑区的中间，光标位置不变；光标在折叠的块内时先展开。
    pub(in crate::app) fn recenter_caret(&mut self) {
        let before = self.editor.active().map(|d| d.scroll_line);
        self.reveal_caret();
        // 先按不可变借用算好目标位置，再写入滚动位置（同 `Self::reveal_caret`）。
        let visible = self.visible_lines() as f64;
        let row = self
            .editor
            .active()
            .map_or(0.0, |d| d.row_at(d.selection.head) as f64);
        let max_scroll = self.max_scroll_line();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        set_scroll_line(doc, (row + 0.5 - visible / 2.0).clamp(0.0, max_scroll));
        self.clamp_scroll();
        if let Some(before) = before {
            self.glide_scroll(before);
        }
        self.request_redraw();
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
        let cell = self.metrics.cell;
        let tab_width = self.editor.active_tab_width();
        let max_scroll = self.max_scroll_line();
        let centered = self.markdown_typewriter() && self.scroll.keyed;

        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
        // 自动换行时按光标所在的显示行计算，且无需横向滚动。
        let row = doc.row_at(doc.selection.head) as f64;

        let mut scroll = doc.scroll_line;
        if centered {
            scroll = (row + 0.5 - visible / 2.0).max(0.0);
        } else if row < scroll + top {
            scroll = (row - top).max(0.0);
        } else if row > scroll + visible - 2.0 {
            scroll = (row - visible + 2.0).max(0.0);
        }
        set_scroll_line(doc, scroll.clamp(0.0, max_scroll.max(row)));

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

    /// 把当前文档的纵向和横向滚动位置限制在有效范围内；纵向上限见 [`Self::scroll_limit`]。
    pub(in crate::app) fn clamp_scroll(&mut self) {
        let max_scroll = self.scroll_limit();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        set_scroll_line(doc, doc.scroll_line.clamp(0.0, max_scroll));
        if doc.diff.is_some() {
            // 差异的两侧共用一个横向滚动位置。
            let max_x = self.diff_hscroll().map_or(0.0, |h| h.max);
            if let Some(doc) = self.editor.active_mut() {
                doc.scroll_x = doc.scroll_x.clamp(0.0, max_x);
            }
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
}
