//! Git 面板：窗口底部、侧栏和编辑区下方的一张卡片，显示日志和文件历史；标题行列出各个日志，
//! 右端是「移至编辑区」、最大化和隐藏。
//!
//! 日志文档只在一处：面板模式（默认）下全部在面板中，编辑区没有日志标签；编辑区模式下全部是编辑区
//! 的标签。两种模式之间整体移动，同一时刻只绘制一个日志，日志视图的状态（`LogState`）因此共用一份。
//! 日志的各个功能经 [`App::log_view`]、[`App::log_area`] 等取得所在的文档和区域，不区分模式。

use winc_config::LogPlacement;
use winc_editor::git::log::LogView;
use winc_editor::{Document, DocumentId, Editor};
use winc_render::Rect;

use crate::app::scroll::set_scroll_line;
use crate::app::{App, Clock, Focus};
use crate::theme::{ISLAND_GAP, ISLAND_MARGIN};

/// 面板标题行的高度（逻辑像素）：与终端面板的标题行相同，容纳 24 像素高的按钮。
pub(crate) const PANEL_HEADER: f32 = 32.0;
/// 面板默认的高度（逻辑像素）。
///
/// 取自调参：提交列表露出约十二行，详情一栏能显示说明和前几个改动的文件。
const DEFAULT_HEIGHT: f32 = 340.0;
/// 面板最矮的高度（逻辑像素）：标题行、日志的标题栏再加约五行提交。
pub(crate) const MIN_HEIGHT: f32 = 200.0;
/// 面板打开时编辑区至少保留的高度（逻辑像素），与终端面板相同。
const EDITOR_MIN: f32 = crate::app::terminal::EDITOR_MIN;

/// Git 面板中可点击的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelHit {
    /// 标题行中第几个日志（`GitPanel::docs` 的下标）：切换到它。
    Tab(usize),
    /// 标题行中一个日志上的关闭按钮。
    Close(usize),
    /// 「移至编辑区」：全部日志改为编辑区的标签。
    ToEditor,
    /// 最大化或还原。
    Maximize,
    /// 隐藏面板。
    Hide,
    /// 面板上沿与编辑区之间的间距：上下拖动改变高度，双击恢复默认高度。
    Divider,
    /// 面板中没有点到其他东西的位置：焦点放进面板。
    Body,
    /// 编辑区模式下日志标签栏右端的「移至面板」：全部日志移到面板。
    ToPanel,
}

/// Git 面板的状态。
pub(crate) struct GitPanel {
    /// 日志是否放在面板中（面板模式）；否则是编辑区的标签。
    pub(crate) docked: bool,
    /// 面板中的日志文档：日志和文件历史，按标题行中的顺序。
    pub(crate) docs: Vec<Document>,
    /// 显示的是第几个日志（`docs` 的下标）。
    pub(crate) at: usize,
    /// 面板是否显示。
    pub(crate) open: bool,
    /// 拖动后的高度（逻辑像素）；`None` 是默认高度。
    pub(crate) height: Option<f32>,
    /// 是否最大化：铺满顶栏以下的部分，盖在侧栏和编辑区上面。
    pub(crate) full: bool,
    /// 显示、隐藏的过渡：开始时露出的比例（0 到 1）和时钟；不在过渡中时为 `None`。
    pub(crate) motion: Option<(f32, Clock)>,
}

impl Default for GitPanel {
    fn default() -> Self {
        GitPanel {
            docked: true,
            docs: Vec::new(),
            at: 0,
            open: false,
            height: None,
            full: false,
            motion: None,
        }
    }
}

impl GitPanel {
    /// 面板露出的比例：0 是隐藏，1 是完全显示，过渡中按时钟插值。
    pub(crate) fn share(&self) -> f32 {
        let target = f32::from(u8::from(self.open));
        self.motion
            .map_or(target, |(from, clock)| clock.between(from, target))
    }

    /// 显示的日志文档。
    fn current(&self) -> Option<&Document> {
        self.docs.get(self.at)
    }

    /// 编号为 `id` 的日志文档，在面板或编辑区 `editor` 中查找。只借用面板和编辑区，调用方可以
    /// 同时修改 `LogState` 等其他状态；[`App::log_document`] 的拆分借用版本。
    pub(crate) fn doc<'a>(&'a self, editor: &'a Editor, id: DocumentId) -> Option<&'a Document> {
        (self.docs.iter())
            .find(|d| d.id == id)
            .or_else(|| editor.document(id))
    }

    /// 同 [`Self::doc`]，可修改；[`App::log_document_mut`] 的拆分借用版本。
    pub(crate) fn doc_mut<'a>(
        &'a mut self,
        editor: &'a mut Editor,
        id: DocumentId,
    ) -> Option<&'a mut Document> {
        match self.docs.iter().position(|d| d.id == id) {
            Some(i) => self.docs.get_mut(i),
            None => editor.document_mut(id),
        }
    }

    /// 正在显示的日志文档，可修改；[`App::log_doc_mut`] 的拆分借用版本。
    pub(in crate::app) fn shown_mut<'a>(
        &'a mut self,
        editor: &'a mut Editor,
    ) -> Option<&'a mut Document> {
        if self.docked {
            return self.docs.get_mut(self.at);
        }
        editor.active_mut().filter(|d| d.log.is_some())
    }
}

impl App {
    /// 面板是否显示着日志：面板模式、面板打开且有日志。
    pub(crate) fn git_panel_shown(&self) -> bool {
        let panel = &self.git.panel;
        panel.docked && panel.open && !panel.docs.is_empty() && self.layout.git_panel.h > 0.0
    }

    /// (`x`, `y`) 是否在显示着的 Git 面板中（不含上沿的间距）。
    pub(in crate::app) fn in_git_panel(&self, x: f32, y: f32) -> bool {
        self.git_panel_shown() && self.layout.git_panel.contains(x, y)
    }

    /// 正在显示的日志文档：面板模式下是面板中显示的日志（面板隐藏时同样返回），编辑区模式下是
    /// 当前标签（是日志时）。
    pub(crate) fn log_doc(&self) -> Option<&Document> {
        if self.git.panel.docked {
            return self.git.panel.current();
        }
        self.editor.active().filter(|d| d.log.is_some())
    }

    /// 同 [`Self::log_doc`]，可修改。
    pub(in crate::app) fn log_doc_mut(&mut self) -> Option<&mut Document> {
        self.git.panel.shown_mut(&mut self.editor)
    }

    /// 正在显示的日志（见 [`Self::log_doc`]）。
    pub(crate) fn log_view(&self) -> Option<&LogView> {
        self.log_doc()?.log.as_deref()
    }

    /// 同 [`Self::log_view`]，可修改。
    pub(in crate::app) fn log_view_mut(&mut self) -> Option<&mut LogView> {
        self.log_doc_mut()?.log.as_deref_mut()
    }

    /// 编号为 `id` 的日志文档，在面板或编辑区中查找。
    pub(crate) fn log_document(&self, id: DocumentId) -> Option<&Document> {
        self.git.panel.doc(&self.editor, id)
    }

    /// 同 [`Self::log_document`]，可修改。
    pub(in crate::app) fn log_document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.git.panel.doc_mut(&mut self.editor, id)
    }

    /// 全部日志文档：面板中的和编辑区的标签。
    pub(in crate::app) fn log_documents(&self) -> impl Iterator<Item = &Document> {
        let docs = self.git.panel.docs.iter().chain(&self.editor.documents);
        docs.filter(|d| d.log.is_some())
    }

    /// 日志所在的区域：面板模式下是面板中标题行以下的部分，编辑区模式下是编辑区。
    pub(crate) fn log_area(&self) -> Rect {
        if !self.git.panel.docked {
            return self.layout.editor;
        }
        let panel = self.layout.git_panel;
        let h = (panel.h - PANEL_HEADER).max(0.0);
        Rect::new(panel.x, panel.y + PANEL_HEADER, panel.w, h)
    }

    /// 日志获得焦点时的焦点：面板模式下是面板，编辑区模式下是编辑区。
    pub(crate) fn log_focus(&self) -> Focus {
        match self.git.panel.docked {
            true => Focus::GitPanel,
            false => Focus::Editor,
        }
    }

    /// 键盘焦点是否在 Git 面板中（面板中的日志或它的筛选输入框）。
    pub(in crate::app) fn git_panel_focused(&self) -> bool {
        self.git_panel_shown() && matches!(self.focus, Focus::GitPanel | Focus::LogFilter)
    }

    /// 键盘焦点是否在日志上（不含筛选输入框）。
    pub(in crate::app) fn log_focused(&self) -> bool {
        self.focus == self.log_focus() && self.log_active()
    }

    /// 打开标题为 `title` 的日志文档：面板模式下在面板中（并显示面板），编辑区模式下是编辑区的
    /// 标签。已打开时换到它。返回文档的编号和是否新建。
    pub(in crate::app) fn open_log_document(&mut self, title: &str) -> (DocumentId, bool) {
        if !self.git.panel.docked {
            let (index, new) = self.editor.show_log(title);
            return (self.editor.documents[index].id, new);
        }
        self.set_git_panel_open(true);
        let panel = &mut self.git.panel;
        let found =
            (panel.docs.iter()).position(|d| d.log.is_some() && d.title.as_deref() == Some(title));
        if let Some(i) = found {
            panel.at = i;
            return (panel.docs[i].id, false);
        }
        let id = self.editor.allocate_id();
        let doc = Document::log(id, title, LogView::new());
        let panel = &mut self.git.panel;
        panel.docs.push(doc);
        panel.at = panel.docs.len() - 1;
        (id, true)
    }

    /// 面板模式下的「Git：显示日志」：面板没有日志时打开日志；焦点已在面板上时隐藏面板，否则
    /// 显示面板并把焦点放进去。
    pub(in crate::app) fn toggle_git_panel(&mut self) {
        if self.git.panel.open && self.focus == Focus::GitPanel {
            self.hide_git_panel();
            return;
        }
        if self.git.panel.docs.is_empty() {
            self.git_show_log();
            return;
        }
        self.set_git_panel_open(true);
        self.focus = Focus::GitPanel;
        self.relayout();
        self.leave_unshown_panel();
        self.request_redraw();
    }

    /// 窗口太矮、面板排不下（见 [`App::place_git_panel`]）时，焦点不停在看不见的面板上。
    pub(in crate::app) fn leave_unshown_panel(&mut self) {
        if self.focus == Focus::GitPanel && !self.git_panel_shown() {
            self.focus = Focus::Editor;
        }
    }

    /// 显示或隐藏面板，从现在露出的比例开始过渡（时长与侧栏、终端面板的开合相同）。
    ///
    /// 显示时提交视图中的提交历史一组随之收起（完整的日志已在眼前），隐藏时恢复，见
    /// [`App::fold_history_for_panel`]。
    fn set_git_panel_open(&mut self, open: bool) {
        if open == self.git.panel.open {
            return;
        }
        let from = self.git.panel.share();
        self.git.panel.open = open;
        let clock = self.clock(crate::app::animation::SIDEBAR_ANIMATION);
        self.git.panel.motion = clock.map(|clock| (from, clock));
        self.fold_history_for_panel(open);
    }

    /// 隐藏面板；日志仍然保留，再次显示时原样出现。焦点在面板上时回到编辑区。
    pub(in crate::app) fn hide_git_panel(&mut self) {
        self.set_git_panel_open(false);
        // 最大化的面板直接收起：先还原再滑下去会多一次跳动。
        if std::mem::take(&mut self.git.panel.full) {
            self.git.panel.motion = None;
        }
        if matches!(self.focus, Focus::GitPanel | Focus::LogFilter) {
            self.focus = Focus::Editor;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 关闭面板中的第 `i` 个日志；关掉最后一个时隐藏面板。
    pub(in crate::app) fn close_panel_log(&mut self, i: usize) {
        if i >= self.git.panel.docs.len() {
            return;
        }
        let shown = i == self.git.panel.at;
        let doc = self.git.panel.docs.remove(i);
        self.git.log.forget(doc.id);
        let panel = &mut self.git.panel;
        if panel.at > i || panel.at >= panel.docs.len() {
            panel.at = panel.at.saturating_sub(1);
        }
        if panel.docs.is_empty() {
            self.hide_git_panel();
        }
        // 关掉的是显示着的日志：筛选输入框换成之后显示的日志的筛选文字。
        if shown {
            if self.focus == Focus::LogFilter {
                self.focus = Focus::GitPanel;
            }
            self.git.log.file = None;
            self.git.log.text_sel = None;
            self.sync_log_filter();
        }
        self.request_redraw();
    }

    /// 面板中显示的是文件历史时，当前仓库跟随它所在的仓库，与编辑区中切换到文件历史的标签相同
    /// （见 [`App::git_follow_dir`]）。
    pub(in crate::app) fn follow_panel_history(&mut self) {
        let view = self
            .log_view()
            .filter(|v| self.git.panel.docked && v.query.path.is_some());
        if let Some(root) = view.and_then(|v| v.root.clone()) {
            self.git_follow_dir(&root);
        }
    }

    /// 关闭面板中的全部日志并隐藏面板（关闭目录时）。
    pub(in crate::app) fn close_panel_logs(&mut self) {
        for doc in std::mem::take(&mut self.git.panel.docs) {
            self.git.log.forget(doc.id);
        }
        self.git.panel.at = 0;
        self.git.panel.open = false;
        self.git.panel.motion = None;
        self.git.panel.full = false;
    }

    /// 换到面板中的第 `i` 个日志。
    pub(in crate::app) fn select_panel_log(&mut self, i: usize) {
        if i < self.git.panel.docs.len() && i != self.git.panel.at {
            self.git.panel.at = i;
            self.git.log.file = None;
            self.git.log.text_sel = None;
            self.sync_log_filter();
            self.follow_panel_history();
        }
        self.focus = Focus::GitPanel;
        self.request_redraw();
    }

    /// 全部日志移到面板（面板模式）：编辑区的日志标签依次移入面板，显示当前标签的那个日志。
    /// `focus` 为真时焦点放进面板（本窗口中的操作）；随另一个窗口改了配置而移动时不抢焦点。
    pub(in crate::app) fn move_logs_to_panel(&mut self, focus: bool) {
        let active = self
            .editor
            .active()
            .filter(|d| d.log.is_some())
            .map(|d| d.id);
        let ids: Vec<DocumentId> = (self.editor.documents.iter())
            .filter(|d| d.log.is_some())
            .map(|d| d.id)
            .collect();
        for id in ids {
            if let Some(doc) = self.editor.close_everywhere(id) {
                self.git.panel.docs.push(doc);
            }
            self.tabs.marked.retain(|marked| *marked != id);
        }
        // 日志是某个窗格仅有的标签时，该窗格随之去掉，与关闭标签相同。
        self.drop_empty_pane();
        self.forget_closed_panes();
        let panel = &mut self.git.panel;
        panel.docked = true;
        if let Some(at) = active.and_then(|id| panel.docs.iter().position(|d| d.id == id)) {
            panel.at = at;
        }
        let open = !panel.docs.is_empty();
        self.set_git_panel_open(open);
        if open && focus {
            self.focus = Focus::GitPanel;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 全部日志移到编辑区（编辑区模式）：面板中的日志依次成为编辑区的标签，面板中显示的那个成为
    /// 当前标签，面板随之关闭。
    pub(in crate::app) fn move_logs_to_editor(&mut self) {
        let shown = self.git.panel.current().map(|d| d.id);
        let docs = std::mem::take(&mut self.git.panel.docs);
        self.set_git_panel_open(false);
        // 日志已经移走，面板没有东西可画：不做收起的过渡。
        let panel = &mut self.git.panel;
        panel.docked = false;
        panel.motion = None;
        panel.full = false;
        panel.at = 0;
        for doc in docs {
            self.editor.insert_document(doc);
        }
        if let Some(index) = shown.and_then(|id| self.editor.index_of(id)) {
            self.editor.active = Some(index);
        }
        if self.focus == Focus::GitPanel {
            self.focus = Focus::Editor;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 日志列表的最大滚动位置（行），按日志所在区域中列表的高度算。
    pub(in crate::app) fn log_max_scroll(&self) -> f64 {
        let rows = self.log_doc().map_or(0, Document::row_count);
        let list = self.log_parts().list.h;
        crate::layout::editor_max_scroll(rows, list, self.fonts.line_height)
    }

    /// 把日志列表的滚动位置限制在范围内；编辑区模式下同编辑区的文档（`App::clamp_scroll`）。
    pub(in crate::app) fn clamp_log_scroll(&mut self) {
        if !self.git.panel.docked {
            return self.clamp_scroll();
        }
        let max = self.log_max_scroll();
        if let Some(doc) = self.log_doc_mut() {
            set_scroll_line(doc, doc.scroll_line.clamp(0.0, max));
        }
    }

    /// 滚轮、触控板在面板中的日志列表上：按 `dy` 逻辑像素（向下滚动为负）滚动。
    pub(in crate::app) fn scroll_panel_log(&mut self, dy: f32) {
        let lh = f64::from(self.fonts.line_height);
        if let Some(doc) = self.log_doc_mut() {
            set_scroll_line(doc, doc.scroll_line - f64::from(dy) / lh);
        }
        self.clamp_log_scroll();
    }

    /// 按配置的日志位置（`git.logPlacement`）放置日志：与现在的模式不同时，已打开的日志整体移过去。
    /// `focus` 见 [`Self::move_logs_to_panel`]。
    pub(in crate::app) fn apply_log_placement(&mut self, focus: bool) {
        let docked = self.config.git_log_placement.unwrap_or_default() == LogPlacement::Panel;
        match (docked, self.git.panel.docked) {
            (true, false) => self.move_logs_to_panel(focus),
            (false, true) => self.move_logs_to_editor(),
            _ => {}
        }
    }

    /// 状态栏是否有 Git 面板开关：Git 模块开着、当前目录有仓库，且日志在 Git 面板中。
    pub(crate) fn git_panel_button(&self) -> bool {
        self.git_enabled() && self.git.root.is_some() && self.git.panel.docked
    }

    /// 状态栏的 Git 面板开关：面板显示着时隐藏，否则显示并把焦点放进去（没有日志时打开日志）。
    pub(in crate::app) fn click_git_panel_button(&mut self) {
        if self.git.panel.open {
            self.hide_git_panel();
        } else {
            self.focus = Focus::GitPanel;
            self.toggle_git_panel();
        }
    }

    /// 处理面板标题行中的点击（上沿的拖动由鼠标的拖动处理）。
    pub(in crate::app) fn press_git_panel(&mut self, hit: PanelHit) {
        match hit {
            PanelHit::Tab(i) => self.select_panel_log(i),
            PanelHit::Close(i) => self.close_panel_log(i),
            PanelHit::ToEditor => self.set_log_placement(LogPlacement::Editor),
            PanelHit::ToPanel => self.set_log_placement(LogPlacement::Panel),
            PanelHit::Maximize => self.toggle_git_panel_full(),
            PanelHit::Hide => self.hide_git_panel(),
            PanelHit::Body => self.focus = Focus::GitPanel,
            PanelHit::Divider => {}
        }
        self.request_redraw();
    }

    /// 面板中日志提交列表的滚动位置和内容高度（逻辑像素），与绘制的滚动条一致。
    pub(in crate::app) fn panel_log_extent(&self) -> (f32, f32) {
        let lh = self.fonts.line_height;
        let list = self.log_parts().list;
        let rows = self.log_doc().map_or(0, Document::row_count);
        let scroll = self.log_doc().map_or(0.0, |d| d.scroll_line);
        let content = crate::layout::editor_content_height(rows, list.h, lh) + self.log_list_top();
        ((scroll * f64::from(lh)) as f32, content)
    }

    /// 把面板中日志提交列表滚动到 `px` 逻辑像素处（拖动滚动条）。
    pub(in crate::app) fn scroll_panel_log_to(&mut self, px: f32) {
        let lh = f64::from(self.fonts.line_height);
        if let Some(doc) = self.log_doc_mut() {
            set_scroll_line(doc, f64::from(px) / lh);
        }
        self.clamp_log_scroll();
    }

    /// 最大化或还原面板。
    pub(in crate::app) fn toggle_git_panel_full(&mut self) {
        self.git.panel.full = !self.git.panel.full;
        self.relayout();
        self.request_redraw();
    }

    /// 拖动面板上沿：把面板的高度改为 `wanted`（逻辑像素），限制在最矮和编辑区最少保留的高度
    /// 之间；记录的高度与排出的高度一致。
    pub(in crate::app) fn drag_git_panel(&mut self, wanted: f32) {
        let panel = self.layout.git_panel;
        let room = panel.bottom() - self.layout.editor.y - EDITOR_MIN - ISLAND_GAP;
        let h = wanted.clamp(MIN_HEIGHT, room.max(MIN_HEIGHT));
        self.git.panel.height = Some(h.round());
        self.relayout();
        self.request_redraw();
    }

    /// 恢复面板的默认高度。
    pub(in crate::app) fn reset_git_panel_height(&mut self) {
        self.git.panel.height = None;
        self.relayout();
        self.request_redraw();
    }

    /// 排版时放置面板：占窗口底部的整个宽度（侧栏和编辑区下方），侧栏和编辑区随之变矮；最大化
    /// 时铺满顶栏以下的部分，盖在侧栏和编辑区上面。编辑区放不下最矮的面板时不显示。显示、隐藏的
    /// 过渡中面板整个从状态栏后面滑上来、滑下去，侧栏和编辑区随露出的高度伸缩。
    ///
    /// 在排好侧栏和编辑区之后、放置终端面板之前调用：终端面板从变矮的编辑区中划出。
    pub(in crate::app) fn place_git_panel(&mut self) {
        let share = self.git.panel.share();
        let shown = {
            let panel = &self.git.panel;
            panel.docked && share > 0.0 && !panel.docs.is_empty()
        };
        let full = self.git.panel.full;
        let height = self.git.panel.height.unwrap_or(DEFAULT_HEIGHT);
        let l = &mut self.layout;
        l.git_panel = Rect::default();
        l.git_divider = Rect::default();
        if !shown {
            return;
        }
        // 左右与侧栏、编辑区的卡片对齐：窗口两侧各留 `ISLAND_MARGIN`。
        let (left, right) = (ISLAND_MARGIN, l.window.right() - ISLAND_MARGIN);
        let bottom = l.editor.bottom();
        if full {
            let top = l.top_bar.bottom();
            l.git_panel = Rect::new(left, top, right - left, bottom - top);
            return;
        }
        let room = l.editor.h - EDITOR_MIN - ISLAND_GAP;
        if room < MIN_HEIGHT {
            return;
        }
        let h = height.clamp(MIN_HEIGHT, room);
        let cut = ((h + ISLAND_GAP) * share).round();
        for rect in [
            &mut l.editor,
            &mut l.gutter,
            &mut l.text,
            &mut l.sidebar,
            &mut l.tree,
            &mut l.splitter,
        ] {
            rect.h = (rect.h - cut).max(0.0);
        }
        l.git_panel = Rect::new(left, bottom - cut + ISLAND_GAP, right - left, h);
        // 过渡中上沿不能拖。
        if share >= 1.0 {
            l.git_divider = Rect::new(left, bottom - cut, right - left, ISLAND_GAP);
        }
    }
}
