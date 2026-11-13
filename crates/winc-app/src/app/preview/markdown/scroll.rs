//! Markdown 预览的滚动：滚轮、目录与锚点的跳转、表格的横向滚动，以及与源代码一侧的同步。

use super::*;

impl App {
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
        let TocTarget::Heading(i) = target else {
            return;
        };
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
            self.flash(t!("preview-markdown-heading-not-found"));
            return;
        };
        let lower = anchor.to_lowercase();
        if !(lower.starts_with("fn-") || lower.starts_with("fnref-")) {
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
        self.preview.markdown.synced = Some((doc.id, doc.scroll_line, laid.height()));
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
        let y = laid.y_of_line(text_line(doc), doc.text().line_count());
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
        let now = (doc.id, doc.scroll_line, laid.height());
        if self.preview.markdown.synced == Some(now) {
            return;
        }
        let y = laid.y_of_line(text_line(doc), doc.text().line_count());
        self.preview.markdown.scroll_to(y);
        self.preview.markdown.synced = Some(now);
    }
}
