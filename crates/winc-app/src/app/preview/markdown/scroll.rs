//! Markdown 预览的滚动：滚轮、目录与锚点的跳转、表格的横向滚动，以及与源代码一侧的同步。

use super::*;

/// 标题列表中列出的范围：标题超过可显示的行数时只列出连续的一段，可按行滚动。
#[derive(Debug, Clone, Copy, Default)]
pub struct TocList {
    /// 上一帧卡片的矩形；列表收起时为空。
    pub card: winc_render::Rect,
    /// 上一帧各行所占的矩形，即滚动条的轨道。
    pub track: winc_render::Rect,
    /// 列出的第一行、行数和标题的总数。
    pub first: usize,
    pub rows: usize,
    pub total: usize,
    /// 一行的高度（逻辑像素）。
    pub row: f32,
    /// 列出的第一行相对默认位置的偏移（行）；默认位置使当前所在的一节居中。
    pub shift: isize,
    /// 设置 `shift` 时当前所在的一节；该值变化时 `shift` 归零。
    pub at: Option<usize>,
    /// 滚轮位移中不足一行的部分（逻辑像素）。
    pub wheel: f32,
}

/// 分帧排版的状态，见 [`Later`](super::laid::Later)。
#[derive(Debug, Clone, Copy, Default)]
pub struct CatchUp {
    /// 一帧中排版视口之外的块的时限；`None` 时取
    /// [`LAYOUT_BUDGET`](super::laid::LAYOUT_BUDGET)，测试中不设时限。
    pub budget: Option<std::time::Duration>,
    /// 本帧绘制后是否仍有沿用旧结果的块；为真时逐帧重绘，直到全部排完。
    pub pending: bool,
}

impl CatchUp {
    /// 返回本帧排版视口之外的块的截止时刻。
    pub fn deadline(&self) -> std::time::Instant {
        let default = if cfg!(test) {
            std::time::Duration::from_secs(3600)
        } else {
            super::laid::LAYOUT_BUDGET
        };
        std::time::Instant::now() + self.budget.unwrap_or(default)
    }
}

impl TocList {
    /// 返回本帧列出的第一行：默认位置 `natural` 加上偏移，并使 `rows` 行不超出 `total`。
    pub fn start(
        &mut self,
        natural: usize,
        current: Option<usize>,
        (rows, total): (usize, usize),
    ) -> usize {
        if self.at != current {
            (self.at, self.shift, self.wheel) = (current, 0, 0.0);
        }
        let start = natural.saturating_add_signed(self.shift).min(total - rows);
        self.shift = start as isize - natural as isize;
        (self.first, self.rows, self.total) = (start, rows, total);
        start
    }
}

impl TocList {
    /// 返回滚动条的（滚动位置，内容高度），逻辑像素；全部标题都已列出时为 `None`。
    pub fn scrollbar(&self) -> Option<(f32, f32)> {
        (self.rows < self.total)
            .then_some((self.first as f32 * self.row, self.total as f32 * self.row))
    }
}

impl App {
    /// 拖动标题列表的滚动条：鼠标的纵坐标为 `y`，按下处距滑块上边 `grab`（逻辑像素）。
    pub(crate) fn drag_toc_scrollbar(&mut self, y: f32, grab: f32) {
        let list = &mut self.preview.markdown.toc_list;
        let Some((_, content)) = list.scrollbar() else {
            return;
        };
        let track = list.track;
        if let Some(thumb) = crate::layout::scrollbar_thumb(track, 0.0, content) {
            let max = content - track.h;
            let to = crate::layout::scroll_for_thumb(track.y, track.h, thumb.h, y, grab, max);
            let first = (to / list.row).round() as isize;
            list.shift += first - list.first as isize;
            self.request_redraw();
        }
    }

    /// 按滚轮位移 `dy` 滚动标题列表，返回是否已处理。
    ///
    /// 鼠标 (`x`, `y`) 不在列表上，或全部标题都已列出时不处理。
    pub(crate) fn scroll_toc(&mut self, (x, y): (f32, f32), dy: f32) -> bool {
        let list = &mut self.preview.markdown.toc_list;
        if list.rows >= list.total || list.row <= 0.0 || !list.card.contains(x, y) {
            return false;
        }
        list.wheel -= dy;
        let steps = (list.wheel / list.row).trunc();
        list.wheel -= steps * list.row;
        list.shift += steps as isize;
        self.request_redraw();
        true
    }

    /// 排版完成后，若有属于该文档的待跳转标题（见 [`App::open_markdown_file`]），滚动到该标题。
    pub(crate) fn scroll_to_pending_anchor(&mut self) {
        let md = &mut self.preview.markdown;
        let Some((_, anchor)) = md.pending_anchor.take_if(|(id, _)| md.doc == Some(*id)) else {
            return;
        };
        match crate::app::preview::markdown::laid::anchor_top(self, &anchor) {
            Some(y) => self.scroll_markdown_to(y),
            None => self.flash(t!("preview-markdown-heading-not-found")),
        }
    }

    /// 在预览上滚动 `dy`（逻辑像素，向上为正，方向约定同滚轮事件）。
    /// 可横向滚动的表格 `key`（见 [`TableBar::key`]）当前的横向滚动位置（逻辑像素）。
    pub(crate) fn markdown_table_scroll(&self, key: (usize, usize)) -> f32 {
        (self.preview.markdown.layout.as_ref()).map_or(0.0, |laid| laid.hscroll_of(key.0, key.1))
    }

    /// 拖动表格的水平滚动条：按鼠标横坐标 `x` 和按下时在滑块上的位置 `grab` 设置滚动位置。
    pub(in crate::app) fn drag_markdown_table_scrollbar(&mut self, x: f32, grab: f32) {
        let Some(bar) = self.hits.markdown_table_bar else {
            return;
        };
        let scroll = self.markdown_table_scroll(bar.key);
        let Some(thumb) = crate::layout::hscrollbar_thumb(bar.track, scroll, bar.content) else {
            return;
        };
        let max = bar.content - bar.track.w;
        let to = crate::layout::scroll_for_thumb(bar.track.x, bar.track.w, thumb.w, x, grab, max);
        if let Some(laid) = self.preview.markdown.layout.as_mut() {
            laid.hscroll.insert(bar.key, to.clamp(0.0, max));
        }
        self.preview.markdown.last_table = Some(bar.key);
        self.request_redraw();
    }

    pub(crate) fn scroll_markdown(&mut self, dy: f32) {
        let y = self.preview.markdown.scroll() - dy;
        self.scroll_markdown_to(y);
    }

    /// 处理标题指示条中第 `i` 个标题的点击：预览滚动到该标题。
    ///
    /// 滚动后将其记为当前所在的一节（见 `MarkdownPreview::toc_picked`），并在正文中为其添加
    /// 逐渐淡出的底色，标示跳转的目标。
    pub(in crate::app) fn press_toc(&mut self, target: TocTarget) {
        if target == TocTarget::Pin {
            self.update_state(move |state| state.markdown_toc_pinned = !state.markdown_toc_pinned);
            let pinned = self.state.markdown_toc_pinned;
            let clock = self.clock(crate::app::animation::TOC_PIN);
            self.preview.markdown.toc_pin = clock.map(|clock| (pinned, clock));
            // 钉住状态变化使正文左边缘移动时，从当前位置过渡到新位置。
            let left = self.preview.markdown.layout.as_ref().map(|laid| laid.left);
            self.preview.markdown.toc_slide = left.filter(|_| clock.is_some());
            self.request_redraw();
            return;
        }
        let TocTarget::Heading(i) = target else {
            return;
        };
        // 收起的一节中的标题：展开后跳到该标题。
        let key = (self.hits.markdown_toc_blocks.get(i))
            .zip(self.preview.markdown.parsed.as_ref())
            .and_then(|(&block, parsed)| parsed.heading_keys().into_iter().nth(block).flatten());
        if let Some(key) = key
            && self.unfold_to_anchor(&key)
        {
            return;
        }
        let Some(&top) = self.hits.markdown_toc_tops.get(i) else {
            return;
        };
        self.glide_pane_to(ScrollTarget::Markdown, top);
        self.preview.markdown.toc_picked = Some((i, self.preview.markdown.scroll()));
        let clock = self.clock(crate::app::animation::TOC_FLASH);
        self.preview.markdown.toc_flash = clock.map(|clock| (i, clock));
    }

    /// 跳转到预览中的锚点 `anchor`（标题、脚注、脚注引用）；找不到时提示。
    ///
    /// 标题滚动到预览顶部。脚注和脚注引用滚动到预览上部三分之一处，上方保留一段上下文，并像
    /// 点击标题列表一样铺一层逐渐淡出的底色，标示跳转的目标：脚注集中在文末，滚动到底之后
    /// 仅凭位置看不出是哪一条。
    pub(in crate::app) fn jump_to_anchor(&mut self, anchor: &str) {
        let Some((top, height)) = crate::app::preview::markdown::laid::anchor_box(self, anchor)
        else {
            if !self.unfold_to_anchor(anchor) {
                self.flash(t!("preview-markdown-heading-not-found"));
            }
            return;
        };
        let lower = anchor.to_lowercase();
        if !(lower.starts_with("fn-") || lower.starts_with("fnref-")) {
            // 书中不用过渡：过渡中的滚动会被当成用户滚动，不再对齐。
            if self.hold_book_at(top) {
                self.settle_glide_of(ScrollTarget::Markdown);
                self.scroll_markdown_to(top);
                return;
            }
            self.glide_pane_to(ScrollTarget::Markdown, top);
            return;
        }
        self.glide_pane_to(
            ScrollTarget::Markdown,
            top - self.preview.markdown.area.h / 3.0,
        );
        let clock = self.clock(crate::app::animation::TOC_FLASH);
        self.preview.markdown.anchor_flash = clock.map(|clock| ((top, height), clock));
    }

    /// 将预览滚动到 `y`；分栏时左侧的文字随之滚动到对应位置。
    pub(crate) fn scroll_markdown_to(&mut self, y: f32) {
        self.preview.markdown.scroll_to(y);
        self.request_redraw();
        if shown_area(self).is_none_or(|a| a == self.layout.editor) {
            return;
        }
        let max = self.max_scroll_line();
        let y = self.preview.markdown.scroll();
        let (Some(laid), Some(doc)) = (
            self.preview.markdown.layout.as_ref(),
            self.editor.active_mut(),
        ) else {
            return;
        };
        let line = laid.line_of_y(y, doc.text().line_count());
        set_scroll_line(doc, text_row(doc, line).clamp(0.0, max.max(0.0)));
        let bottom = laid.height() - self.preview.markdown.area.h;
        self.preview.markdown.synced = Some((doc.id, doc.scroll_line, bottom));
    }

    /// 滚动文字，使原文第 `line` 行（小数）位于顶部。
    pub(in crate::app) fn scroll_text_to(&mut self, line: f32) {
        let max = self.max_scroll_line();
        if let Some(doc) = self.editor.active_mut() {
            set_scroll_line(doc, text_row(doc, line).clamp(0.0, max.max(0.0)));
        }
    }

    /// 将预览滚动到文字顶端那一行对应的位置；仅在刚切换为只显示预览后执行一次。
    ///
    /// 位置的计算使用分栏同步滚动的映射。
    ///
    /// 排版完成之后、绘制之前调用。
    pub(crate) fn align_markdown_once(&mut self) {
        let (Some(laid), Some(doc)) = (self.preview.markdown.layout.as_ref(), self.editor.active())
        else {
            return;
        };
        if self.preview.markdown.align != Some(doc.id) {
            return;
        }
        self.preview.markdown.align = None;
        let lines = self.preview.markdown.lines.as_deref();
        let (line, total) = preview_line(doc, lines, doc.scroll_line);
        let y = laid.y_of_line(line, total);
        self.preview.markdown.scroll_to(y);
    }

    /// 分栏时左侧文字滚动或预览重排后，将右侧滚动到对应位置。
    ///
    /// 绘制预览之前调用。
    pub(crate) fn sync_markdown_scroll(&mut self) {
        let (Some(laid), Some(doc)) = (self.preview.markdown.layout.as_ref(), self.editor.active())
        else {
            return;
        };
        // 内容或预览区域的高度变化后同样重新对齐。
        let bottom = laid.height() - self.preview.markdown.area.h;
        let now = (doc.id, doc.scroll_line, bottom);
        if self.preview.markdown.synced == Some(now) {
            return;
        }
        let lines = self.preview.markdown.lines.as_deref();
        let (line, total) = preview_line(doc, lines, doc.scroll_line);
        // 文字滚到底时预览可能还没有到底（预览中折行更多）：余下的距离在文字的最后一屏中
        // 按比例补上，两侧同时到底。
        let max = self.max_scroll_line();
        let screen = f64::from(self.scroll_track().h / self.fonts.line_height).min(max);
        let (end, _) = preview_line(doc, lines, max);
        let rest = (bottom - laid.y_of_line(end, total)).max(0.0);
        let share = match screen > 0.0 {
            true => ((doc.scroll_line - (max - screen)) / screen).clamp(0.0, 1.0),
            false => 0.0,
        };
        let y = laid.y_of_line(line, total) + rest * share as f32;
        self.preview.markdown.scroll_to(y);
        self.preview.markdown.synced = Some(now);
    }
}
