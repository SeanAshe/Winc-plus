//! 粘滞滚动：滚过函数、类、标题的开头后，把该行固定在编辑区顶部。
//!
//! 块即将结束时，固定的行随下方的代码上移；单击固定的行跳转到该行。块的识别由
//! `winc_editor::scopes` 完成；固定行数的上限和各行的位置（[`sticky_limit`]、
//! [`place_sticky_rows`]）由编辑区与差异视图共用。

use winc_editor::DocumentId;
use winc_editor::scopes::Scope;

use super::*;

/// 固定行数的上限（行）。
///
/// 取自调参：限制深层嵌套时固定区域占用的高度。实际行数还受 [`sticky_limit`] 限制，
/// 不超过可见行数的 40%。
const STICKY_MAX: usize = 5;

/// 返回可见 `visible` 行时最多固定的行数（行）：不超过可见行数的 40%，也不超过
/// [`STICKY_MAX`]。
///
/// 40% 取自调参：区域较矮时固定行仍给滚动的代码留出多数空间。
pub(crate) fn sticky_limit(visible: usize) -> usize {
    (visible * 2 / 5).min(STICKY_MAX)
}

/// 返回固定的块 `scopes`（从外层到内层）在固定区域中的各行，已完全移出顶部的行不包含在内。
///
/// 第 `depth` 层通常位于第 `depth` 行（从 0 起）；块在屏幕上的最后一行的顶边高于该位置时，
/// 固定行随这一行上移，直至移出顶部。`last_top` 返回 [`Scope::last_row`] 所指的行的顶边
/// （逻辑像素，相对固定区域的顶部），`line_height` 是行高（逻辑像素）。
pub(crate) fn place_sticky_rows(
    scopes: &[Scope],
    line_height: f32,
    last_top: impl Fn(usize) -> f32,
) -> Vec<StickyRow> {
    scopes
        .iter()
        .enumerate()
        .map(|(depth, scope)| {
            let natural = depth as f32 * line_height;
            StickyRow {
                line: scope.start,
                depth,
                y: scope
                    .last_row
                    .map_or(natural, |last| natural.min(last_top(last))),
            }
        })
        .filter(|row| row.y + line_height > 0.0)
        .collect()
}

/// 已计算的块的缓存。
///
/// 文档、版本号和顶部显示的行都不变时无需重新计算。语法高亮尚未推算到所需位置时（例如刚跳到
/// 长文件的后部），字符串内的行可能被误判为代码，此时 `settled` 为 `false`，下一帧重新计算。
pub struct StickyCache {
    /// 缓存对应的文档。
    id: DocumentId,
    /// 缓存对应的文档版本号（`revision`）。
    revision: u64,
    /// 顶部显示的文件行号（从 0 起）；有折叠时不连续。
    rows: Vec<usize>,
    /// 顶部第一行是否只有下半部分可见，即滚动位置带小数。
    partial: bool,
    /// 计算时使用的固定行数上限（行）。
    max: usize,
    /// 计算时使用的 Tab 宽度（列）。
    tab_width: usize,
    /// 语法高亮是否已推算到足以确定 `scopes`。
    settled: bool,
    /// 固定的块，从外层到内层。
    scopes: Vec<Scope>,
}

/// 固定在编辑区顶部的一行。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickyRow {
    /// 显示的文件行号（从 0 起）。
    pub line: usize,
    /// 嵌套层级，最外层为 0。
    pub depth: usize,
    /// 这一行顶边的纵坐标（逻辑像素），从编辑区文字区的顶部起算；上移超出顶部时为负。
    pub y: f32,
}

impl App {
    /// 返回当前最多固定的行数：不超过可见行数的 40%，也不超过 [`STICKY_MAX`]。
    ///
    /// 编辑区较矮时固定的行数相应减少。
    fn sticky_max(&self) -> usize {
        sticky_limit(self.visible_lines())
    }

    /// 返回当前固定在编辑区顶部的行，从外层到内层。
    ///
    /// 已被推出编辑区而不可见的行不包含在内。
    pub(crate) fn sticky_rows(&mut self) -> Vec<StickyRow> {
        if self.config.sticky_scroll == Some(false) {
            return Vec::new();
        }
        let max = self.sticky_max();
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active_mut().filter(|d| d.shows_text()) else {
            return Vec::new();
        };
        // 顶部显示的文件行：有折叠时行号不连续；自动换行时一行的每个显示行各产生一项，
        // 同一个行号会重复，因此之后去重。
        let scroll = doc.scroll_line.max(0.0);
        let first = scroll.floor() as usize;
        let mut rows: Vec<usize> = doc
            .folds()
            .segments(first, max + 1, doc.text().line_count())
            .into_iter()
            .flatten()
            .collect();
        rows.dedup();

        // `scroll_line` 带小数时，顶部第一行的上半部分已滚到固定区域之下。
        let partial = scroll.fract() > 0.0;

        let fresh = self.sticky_cache.as_ref().is_some_and(|c| {
            c.settled
                && (c.id, c.revision, &c.rows, c.partial, c.max, c.tab_width)
                    == (doc.id, doc.revision, &rows, partial, max, tab_width)
        });
        if !fresh {
            let (scopes, settled) = doc.sticky_scopes(&rows, partial, max, tab_width);
            self.sticky_cache = Some(StickyCache {
                id: doc.id,
                revision: doc.revision,
                rows,
                partial,
                max,
                tab_width,
                settled,
                scopes,
            });
        }
        let Some(cache) = &self.sticky_cache else {
            return Vec::new();
        };

        // 位置按屏幕上的显示行计算，已折叠的行不占位置。
        let line_height = self.fonts.line_height;
        let first_row = scroll.floor();
        place_sticky_rows(&cache.scopes, line_height, |last| {
            (first_row + last as f64 - scroll) as f32 * line_height
        })
    }

    /// 返回光标在 `line` 行时顶部需要固定的层数（不超过 `sticky_max`）。
    ///
    /// 光标移到被固定区域遮住的位置时，滚动距离需要增加相同的行数。
    pub(in crate::app) fn sticky_depth(&mut self, line: usize) -> usize {
        if self.config.sticky_scroll == Some(false) {
            return 0;
        }
        let max = self.sticky_max();
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active_mut().filter(|d| d.shows_text()) else {
            return 0;
        };
        doc.scope_depth(line, tab_width).min(max)
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处固定的行；该处不是固定行时返回 `None`。
    ///
    /// 固定行的位置取自上一次绘制时记录的 `self.hits.sticky`。
    pub(in crate::app) fn sticky_at(&self, x: f32, y: f32) -> Option<StickyRow> {
        let text = self.layout.text;
        let editor = self.layout.editor;
        if !editor.contains(x, y)
            || x >= editor.right() - SCROLLBAR_HIT
            || self.layout.search.contains(x, y)
        {
            return None;
        }
        let line_height = self.fonts.line_height;
        // 上移的内层与外层重叠时外层绘制在上面，因此从外层到内层查找，先匹配的即为最上层。
        self.hits
            .sticky
            .iter()
            .find(|row| {
                let top = text.y + row.y;
                (top.max(text.y)..top + line_height).contains(&y)
            })
            .copied()
    }

    /// 处理对固定行 `row` 的点击，`x` 为点击处的窗口横坐标（逻辑像素）。
    ///
    /// 光标移到该行中被点击的列，并滚动到该行的原位置，使该行正好位于外层固定行之下。
    pub(in crate::app) fn click_sticky(&mut self, row: StickyRow, x: f32) {
        let tab_width = self.editor.active_tab_width();
        let origin = self.layout.text.x + TEXT_PAD;
        let cell = self.cell;
        let max = self.max_scroll_line();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let content = doc.text().line_content(row.line);
        let cells = ((x - origin + doc.scroll_x) / cell).round().max(0.0) as usize;
        let col = text::cell_to_col(content.slice(), cells, tab_width);
        let pos = doc.text().line_col_to_pos(row.line, col);
        doc.selection = Selection::point(winc_core::movement::snap_to_grapheme(doc.text(), pos));
        doc.goal_cell = None;
        let before = doc.scroll_line;
        doc.scroll_line = (doc.row_of(row.line) as f64 - row.depth as f64).clamp(0.0, max.max(0.0));
        self.glide_scroll(before);
        self.focus = Focus::Editor;
        self.reset_blink();
        self.request_redraw();
    }
}
