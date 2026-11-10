//! 侧栏的全局搜索面板：条件输入、后台搜索、结果列表和跨文件替换。
//!
//! [`GlobalSearch`] 保存输入框、选项、折叠状态和选中的行；搜索交给 [`ProjectSearch`] 在后台
//! 进行，结果按版本号增量更新到列表。替换前先在 [`ReplacePreview`] 中确认，再改写文件或已打开
//! 的文档。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use winc_core::text::line_slice;
use winc_editor::{
    Document, LineMatch, ProjectSearch, Query, ReplacePreview, replace_in_document, replace_in_file,
};
use winc_i18n::t;
use winc_render::Rect;

use super::*;
use crate::app::animation::Presence;
use crate::app::input::context_menu::MenuCommand;
use crate::layout::{PanelButton, PanelParts, PanelTool, RowAction, SidebarView};

/// 搜索条件变化之后到开始搜索之间的等待时间。
///
/// 取 300 ms（经验值）：连续键入时只在停顿之后搜索一次，避免每输入一个字符就重新搜索整个目录。
pub const SEARCH_DELAY: Duration = Duration::from_millis(300);

/// 结果列表中单个文件折叠或展开的动画，实现方式与改动列表的 `CommitMotion` 相同。
#[derive(Debug)]
pub struct ResultMotion {
    /// 动画期间绘制的结果行：展开时为展开后的行，折叠时为折叠前的行。
    pub rows: Vec<ResultRow>,
    /// 正在折叠或展开的文件行在 `rows` 中的下标。
    pub parent: usize,
    /// 露出或收回的匹配行数。
    pub count: usize,
    /// 是否为展开；`false` 表示折叠。
    pub expanding: bool,
    /// 本次动画的时钟。
    pub(in crate::app) clock: Clock,
}

impl ResultMotion {
    /// 返回匹配行在 `rows` 中的下标范围 `[起, 止)`。
    pub fn block(&self) -> (usize, usize) {
        (self.parent + 1, self.parent + 1 + self.count)
    }

    /// 返回尚未露出的匹配行的总高度（逻辑像素），其下方的行按该距离上移。
    pub fn hidden(&self) -> f32 {
        let eased = self.clock.progress();
        let reveal = if self.expanding { eased } else { 1.0 - eased };
        (1.0 - reveal) * self.count as f32 * TREE_ROW
    }

    /// 返回文件行箭头的旋转比例：0 朝右（折叠），1 朝下（展开）。
    pub fn turned(&self) -> f32 {
        1.0 - self.hidden() / (self.count as f32 * TREE_ROW)
    }
}

/// 结果列表中的一行：文件，或文件中的一处匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultRow {
    /// 文件行，参数为文件在结果中的序号（从 0 起）。
    File(usize),
    /// 文件中的一处匹配，参数依次为文件序号和该文件内的匹配序号（均从 0 起）。
    Match(usize, usize),
}

impl ResultRow {
    /// 返回这一行所属文件在结果中的序号。
    pub fn file(self) -> usize {
        match self {
            ResultRow::File(f) | ResultRow::Match(f, _) => f,
        }
    }
}

/// 跨文件替换的统计。
#[derive(Debug, Default)]
struct ReplaceTally {
    /// 有替换的文件数。
    files: usize,
    /// 替换的总处数。
    count: usize,
    /// 有替换且已打开的文件数：替换在文档中进行，尚未保存。
    opened: usize,
    /// 替换失败的文件，每项是文件名和原因（显示给用户的文字）。
    failed: Vec<String>,
}

impl ReplaceTally {
    /// 记录文件 `path` 的替换结果：成功时为替换的处数，失败时为原因；`open` 表示文件已打开。
    fn add(&mut self, path: &Path, result: Result<usize, String>, open: bool) {
        match result {
            Ok(n) => {
                self.files += usize::from(n > 0);
                self.count += n;
                self.opened += usize::from(open && n > 0);
            }
            Err(e) => {
                let name = winc_editor::display_name(path);
                let message = t!("workspace-replace-failed-file", name = name, error = e);
                self.failed.push(message);
            }
        }
    }
}

/// 全局搜索面板的状态。
#[derive(Default)]
pub struct GlobalSearch {
    /// 后台项目搜索引擎，同时保存搜索结果。
    pub engine: ProjectSearch,
    /// 查找输入框。
    pub query: TextField,
    /// 替换输入框。
    pub replace: TextField,
    /// 「包含」文件的 glob 输入框。
    pub include: TextField,
    /// 「排除」文件的 glob 输入框。
    pub exclude: TextField,
    /// 是否区分大小写。
    pub case_sensitive: bool,
    /// 是否全字匹配。
    pub whole_word: bool,
    /// 是否按正则表达式搜索。
    pub regex: bool,
    /// 是否也搜索被忽略的文件。
    pub include_ignored: bool,
    /// 查找、替换输入框的文字折成的行数。
    pub field_lines: (usize, usize),
    /// 输入框高度的过渡：开始时查找、替换输入框的高度和时钟；不在过渡时为 `None`。
    pub field_motion: Option<(f32, f32, crate::app::animation::Clock)>,
    /// 替换行是否展开。
    pub replace_open: bool,
    /// 「包含」「排除」输入框是否展开。
    pub details_open: bool,
    /// 替换行展开、收起的动画。
    pub(in crate::app) replace_fx: Presence,
    /// 「包含」「排除」两行展开、收起的动画。
    pub(in crate::app) details_fx: Presence,
    /// 是否已记录初始的展开状态。
    ///
    /// 第一帧只记录两行是否展开，不播放动画：恢复上次会话时两行已处于展开状态。
    fx_primed: bool,
    /// 文件折叠或展开的动画；不在动画中时为 `None`。
    pub motion: Option<ResultMotion>,
    /// 已折叠的文件。
    pub collapsed: HashSet<PathBuf>,
    /// 结果列表的纵向滚动位置（逻辑像素）。
    pub scroll: f32,
    /// 选中的一行：文件路径和该文件中第几处匹配（`None` 表示文件行）。
    ///
    /// 按路径记录，搜索过程中陆续插入的文件不会使选中的行变成别的。
    pub selected: Option<(PathBuf, Option<usize>)>,
    /// 条件变化后开始搜索的时刻。
    pub due: Option<Instant>,
    /// 正在后台写回磁盘的跨文件替换：接收统计结果的通道，以及开始替换时结果列表中的总处数。
    replacing: Option<(mpsc::Receiver<ReplaceTally>, usize)>,
    /// 正在后台替换的单个未打开文件：每个文件一个通道，接收路径和替换的处数（或失败的原因）。
    row_replacing: Vec<mpsc::Receiver<(PathBuf, Result<usize, String>)>>,
    /// 展开后的结果行；结果或折叠状态变化时重新生成。
    rows: Vec<ResultRow>,
    /// 生成 `rows` 时结果的版本号。
    rows_version: Option<u64>,
    /// 替换预览用的正则，按搜索条件和替换文字缓存。
    preview: Option<(Query, String, ReplacePreview)>,
}

/// 查找、替换输入框最多显示的行数（经验值）；更长的文字在框内滚动。
const FIELD_MAX_LINES: usize = 6;
/// 查找、替换输入框每多一行增加的高度（逻辑像素），与折行的行高相同。
const FIELD_LINE: f32 = 20.0;

impl GlobalSearch {
    /// 返回查找、替换输入框当前的高度，过渡途中是过渡到的位置。
    pub(crate) fn field_heights(&self) -> (f32, f32) {
        let (query, replace) = self.settled_field_heights();
        self.field_motion
            .map_or((query, replace), |(from_q, from_r, clock)| {
                (clock.between(from_q, query), clock.between(from_r, replace))
            })
    }

    /// 返回查找、替换输入框按行数应有的高度。
    pub(crate) fn settled_field_heights(&self) -> (f32, f32) {
        let height = |lines: usize| {
            let extra = lines.clamp(1, FIELD_MAX_LINES) - 1;
            crate::layout::PANEL_FIELD + extra as f32 * FIELD_LINE
        };
        (height(self.field_lines.0), height(self.field_lines.1))
    }

    /// 返回由输入框和选项组成的当前搜索条件。
    pub fn current_query(&self) -> Query {
        Query {
            text: self.query.text().to_owned(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            include_ignored: self.include_ignored,
            include: self.include.text().to_owned(),
            exclude: self.exclude.text().to_owned(),
            required: String::new(),
        }
    }

    /// 返回展开后的结果行。
    pub fn rows(&self) -> &[ResultRow] {
        &self.rows
    }

    /// 结果变化（后台新增文件、替换、移除）时重新生成结果行。
    pub fn sync_rows(&mut self) {
        if self.rows_version != Some(self.engine.version()) {
            self.rebuild_rows();
        }
    }

    /// 返回替换行展开的比例：0 为收起，1 为完全展开，介于两者之间表示动画进行中。
    pub fn replace_share(&self) -> f32 {
        self.replace_fx.share(self.replace_open)
    }

    /// 返回「包含」「排除」两行展开的比例：0 为收起，1 为完全展开，介于两者之间表示动画进行中；
    /// 比例的计算方式与 [`Self::replace_share`] 相同。
    pub fn details_share(&self) -> f32 {
        self.details_fx.share(self.details_open)
    }

    /// 替换行或「包含」「排除」两行是否正在展开或收起。
    pub fn rows_moving(&self) -> bool {
        self.replace_fx.moving() || self.details_fx.moving()
    }

    /// 折叠状态变化后重新生成结果行。
    ///
    /// 进行中的折叠动画按旧的结果行绘制，因此重新生成结果行时同时取消该动画。
    pub fn rebuild_rows(&mut self) {
        self.motion = None;
        self.rows_version = Some(self.engine.version());
        self.rows.clear();

        for (f, file) in self.engine.results.iter().enumerate() {
            self.rows.push(ResultRow::File(f));

            if !self.collapsed.contains(&file.path) {
                self.rows
                    .extend((0..file.matches.len()).map(|m| ResultRow::Match(f, m)));
            }
        }
        // 结果大幅减少（清空、更换查找内容）后不保留上一次的容量；结果陆续送达、行数增长时
        // 不收缩。
        if self.rows.len() * 4 < self.rows.capacity() {
            self.rows.shrink_to_fit();
        }
    }

    /// 返回选中的那一行在结果行中的序号；没有选中时返回 `None`。
    pub fn selected_row(&self) -> Option<usize> {
        let (path, m) = self.selected.as_ref()?;
        let f = self.engine.results.iter().position(|r| &r.path == path)?;
        let row = match m {
            Some(m) => ResultRow::Match(f, *m),
            None => ResultRow::File(f),
        };
        self.rows.iter().position(|&r| r == row)
    }

    /// 选中第 `index` 个结果行；`index` 超出范围时清除选中。
    pub fn select_row(&mut self, index: usize) {
        self.selected = self.rows.get(index).map(|&row| match row {
            ResultRow::File(f) => (self.engine.results[f].path.clone(), None),
            ResultRow::Match(f, m) => (self.engine.results[f].path.clone(), Some(m)),
        });
    }

    /// 是否所有文件都已折叠；没有结果时为 `false`。
    pub fn all_collapsed(&self) -> bool {
        !self.engine.results.is_empty()
            && self
                .engine
                .results
                .iter()
                .all(|f| self.collapsed.contains(&f.path))
    }

    /// 返回替换预览；搜索条件或替换文字变化时才重新编译正则。
    ///
    /// 还没有搜索过（没有查询条件）时返回 `None`。
    pub fn preview(&mut self) -> Option<&ReplacePreview> {
        let query = self.engine.query()?;
        let replacement = self.replace.text();
        let stale = self
            .preview
            .as_ref()
            .is_none_or(|(q, r, _)| q != query || r != replacement);

        if stale {
            let preview = ReplacePreview::new(query, replacement);
            self.preview = Some((query.clone(), replacement.to_owned(), preview));
        }

        self.preview.as_ref().map(|(_, _, p)| p)
    }

    /// 返回上一次 [`GlobalSearch::preview`] 算好的替换预览，不检查是否过时。
    ///
    /// 绘制结果列表时每帧先调用一次 `preview` 更新，各行再用它读出，不必每行比较一遍条件。
    pub fn cached_preview(&self) -> Option<&ReplacePreview> {
        self.preview.as_ref().map(|(_, _, p)| p)
    }
}

impl App {
    /// 返回侧栏中顶栏以下的区域，搜索面板绘制在这里。
    pub(crate) fn panel_rect(&self) -> Rect {
        let s = self.layout.sidebar;
        Rect::new(s.x, TOP_BAR_END, s.w, (s.bottom() - TOP_BAR_END).max(0.0))
    }

    /// 返回搜索面板各部分的布局。
    pub(crate) fn panel_parts(&self) -> PanelParts {
        let g = &self.global;
        // 搜完了没有结果时不写统计行，结果列表里居中写「没有找到结果」（`draw_empty_state`）。
        let summary =
            (!g.query.text().is_empty() && !self.no_search_results()) || g.engine.error().is_some();
        let (replace, details) = (g.replace_share(), g.details_share());
        PanelParts::new(
            self.panel_rect(),
            replace,
            details,
            summary,
            g.field_heights(),
        )
    }

    /// 记下查找、替换输入框折成的行数；变化时框高从当前高度过渡到新高度。
    pub(crate) fn set_search_field_lines(&mut self, lines: (usize, usize)) {
        if lines == self.global.field_lines {
            return;
        }
        let settled = self.global.settled_field_heights();
        let (from_q, from_r) = self.global.field_heights();
        self.global.field_lines = lines;
        // 行数变了、高度不变（超过最多行数之后）时不用过渡。
        if self.global.settled_field_heights() == settled {
            return;
        }
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.global.field_motion = clock.map(|c| (from_q, from_r, c));
        self.clamp_results_scroll();
    }

    /// 在每帧绘制前检查替换行与「包含」「排除」两行的展开状态，变化时开始展开或收起的动画。
    ///
    /// 与 [`App::track_modules`] 相同，按上一帧的状态比较，切换开关的代码无需各自记录。
    pub(in crate::app) fn track_search_rows(&mut self) {
        let g = &self.global;
        let changed = g.replace_fx.changed(g.replace_open) || g.details_fx.changed(g.details_open);
        let primed = std::mem::replace(&mut self.global.fx_primed, true);
        let clock = if primed && changed {
            self.clock(TREE_ANIMATION)
        } else {
            None
        };
        let g = &mut self.global;
        g.replace_fx.track(g.replace_open, clock);
        g.details_fx.track(g.details_open, clock);
    }

    /// 折叠（`collapse` 为 `true`）或展开第 `f` 个文件的结果，并播放动画。
    ///
    /// 匹配行从文件行下方逐渐露出或收回，其下方的行随之移动。
    pub(in crate::app) fn set_result_collapsed(&mut self, f: usize, collapse: bool) {
        let g = &mut self.global;
        let Some(file) = g.engine.results.get(f) else {
            return;
        };
        let (path, count) = (file.path.clone(), file.matches.len());
        let changed = if collapse {
            g.collapsed.insert(path)
        } else {
            g.collapsed.remove(&path)
        };
        if !changed {
            return;
        }

        let before = std::mem::take(&mut g.rows);
        g.rebuild_rows();
        let parent = g.rows.iter().position(|&r| r == ResultRow::File(f));
        let clock = self.clock(TREE_ANIMATION);
        let g = &mut self.global;
        // 动画期间按行数较多的状态绘制：展开时绘制展开后的行，折叠时绘制折叠前的行。
        let rows = if collapse { before } else { g.rows.clone() };
        g.motion = match (parent, clock) {
            (Some(parent), Some(clock)) if count > 0 => Some(ResultMotion {
                rows,
                parent,
                count,
                expanding: !collapse,
                clock,
            }),
            _ => None,
        };
        self.clamp_results_scroll();
    }

    /// 是否已搜索完成但没有任何结果，且没有出错。
    pub(crate) fn no_search_results(&self) -> bool {
        let g = &self.global;
        !g.query.text().is_empty()
            && g.engine.total() == 0
            && !g.engine.is_running()
            && g.due.is_none()
            && g.engine.error().is_none()
    }

    /// 搜索面板是否正在显示：侧栏当前是搜索视图，且侧栏可见。
    pub(in crate::app) fn panel_shown(&self) -> bool {
        self.sidebar.view == SidebarView::Search && self.layout.sidebar.right() > 0.0
    }

    /// 把侧栏切换到视图 `view`（文件树或搜索）。
    pub(in crate::app) fn set_sidebar_view(&mut self, view: SidebarView) {
        if self.sidebar.view == view {
            return;
        }

        let from = std::mem::replace(&mut self.sidebar.view, view);
        self.sidebar.view_slide = self
            .clock(crate::app::animation::VIEW_SLIDE)
            .map(|clock| (from, clock));
        self.finish_name_input();
        self.relayout();

        // 切回文件树时，在搜索中打开过的文件已在树中展开并选中，滚动文件树使其可见。
        if view == SidebarView::Files {
            self.clamp_tree_scroll();
            self.reveal_active();
        }
        // 其他位置的状态只在提交视图里看得见，看不见的时候没有读。
        if view == SidebarView::Commit {
            self.git_read_summaries();
        }

        self.request_redraw();
    }

    /// 把侧栏切换到文件树，并把焦点交给文件树（`⇧⌘E`）。
    pub(in crate::app) fn show_files(&mut self) {
        self.set_sidebar_view(SidebarView::Files);

        if !self.sidebar.visible {
            self.set_sidebar(true);
        }

        self.focus = Focus::Tree;
        self.request_redraw();
    }

    /// 把侧栏切换到搜索面板，并把焦点放进查找输入框（`⇧⌘F` / `⇧⌘H`）。
    ///
    /// 有合适的选区（没有选区时为光标所在的词）时，用它作为查找内容。`replace` 为 `true` 时
    /// 同时展开替换行；此时如果已有查找内容，焦点放进替换输入框。没有打开目录时在状态栏提示。
    pub(in crate::app) fn open_global_search(&mut self, replace: bool) {
        if self.editor.workspace.is_none() {
            self.flash(t!("workspace-open-folder-first"));
            return;
        }

        self.set_sidebar_view(SidebarView::Search);

        if !self.sidebar.visible {
            self.set_sidebar(true);
        }

        if let Some(seed) = self.search_seed()
            && seed != self.global.query.text()
        {
            self.global.query.remember();
            self.global.query.set_text(&seed);
            self.global_changed(true);
        } else {
            self.global.query.select_all();
        }

        if replace {
            self.global.replace_open = true;
        }

        self.focus = if replace && !self.global.query.text().is_empty() {
            self.global.replace.select_all();
            Focus::GlobalReplace
        } else {
            Focus::GlobalQuery
        };
        self.field_preedit.clear();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 处理查找条件的变化：`now` 为 `true` 时立即搜索，否则等待 [`SEARCH_DELAY`] 之后再搜索。
    ///
    /// 查找内容删空时立即搜索：统计行随输入框清空而消失，结果列表须在同一帧清除，
    /// 避免延迟期间仍显示旧结果。
    pub(in crate::app) fn global_changed(&mut self, now: bool) {
        if now || self.global.query.text().is_empty() {
            self.run_global_search();
        } else {
            self.global.due = Some(Instant::now() + SEARCH_DELAY);
        }

        self.request_redraw();
    }

    /// 按输入框中的条件从头搜索整个目录。
    pub(in crate::app) fn run_global_search(&mut self) {
        self.global.due = None;

        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let query = self.global.current_query();

        // 有未保存修改的文档按内存中的内容搜索。
        let dirty = self.dirty_ropes();

        let proxy = self.proxy.clone();
        let wake = move || {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::Searched);
            }
        };

        self.global.engine.start(&root, &query, dirty, wake);
        self.global.scroll = 0.0;
        self.global.selected = None;
        self.global.rebuild_rows();
        self.request_redraw();
    }

    /// 文件被改名、移动或删除之后调用，有结果时按当前条件重新搜索一遍。
    ///
    /// 结果中仍记录着旧路径：点击会打开不存在的文件，替换也会作用到旧路径上。
    pub(in crate::app) fn files_changed_under_results(&mut self) {
        if self.global.engine.query().is_some() {
            self.run_global_search();
        }
    }

    /// 处理后台搜索的进展：有新结果时重新生成结果行并重绘。
    ///
    /// 后台重新搜索一批文件后，选中的那一处可能已经不在结果中：改为选中原位置上的行，与替换、
    /// 移除结果之后相同（见 [`Self::after_results_changed`]）。
    pub(in crate::app) fn poll_global(&mut self) {
        let selected = self.global.selected_row();
        if self.global.engine.poll() {
            self.global.sync_rows();
            if let Some(index) = selected
                && self.global.selected_row().is_none()
            {
                self.after_results_changed(index);
            }
            self.clamp_results_scroll();
            self.request_redraw();
        }
    }

    /// 返回结果列表的最大滚动位置（逻辑像素）；末尾多留一行（`TREE_ROW`）空白。
    pub(in crate::app) fn results_max_scroll(&self) -> f32 {
        let rows = self.global.rows().len() as f32;
        ((rows + 1.0) * TREE_ROW - self.panel_parts().results.h).max(0.0)
    }

    fn clamp_results_scroll(&mut self) {
        self.global.scroll = self.global.scroll.clamp(0.0, self.results_max_scroll());
    }

    /// 返回窗口纵坐标 `y`（逻辑像素）处的结果行序号；没有对应的行时返回 `None`。
    pub(in crate::app) fn result_row_at(&self, y: f32) -> Option<usize> {
        let results = self.panel_parts().results;
        let index = ((y - results.y + self.global.scroll) / TREE_ROW).floor();
        (index >= 0.0 && (index as usize) < self.global.rows().len()).then_some(index as usize)
    }

    /// 返回第 `index` 个结果行在窗口中的矩形。
    pub(crate) fn result_row_rect(&self, index: usize) -> Rect {
        let results = self.panel_parts().results;
        let y = results.y + index as f32 * TREE_ROW - self.global.scroll;
        Rect::new(results.x, y, results.w, TREE_ROW)
    }

    /// 滚动结果列表，使第 `index` 个结果行进入可见范围。
    fn scroll_result_into_view(&mut self, index: usize) {
        let h = self.panel_parts().results.h;
        let y = index as f32 * TREE_ROW;
        self.global.scroll = scroll_to_show(self.global.scroll, y, y + TREE_ROW, h);
        self.clamp_results_scroll();
    }

    /// 处理搜索面板上按钮 `button` 的点击。
    pub(in crate::app) fn press_panel_button(&mut self, button: PanelButton) {
        let g = &mut self.global;

        match button {
            PanelButton::Tool(PanelTool::Refresh) => self.run_global_search(),
            PanelButton::Tool(PanelTool::Clear) => {
                g.engine.clear();
                g.query.remember();
                g.query.set_text("");
                g.rebuild_rows();
                g.selected = None;
                g.scroll = 0.0;
                g.due = None;
                self.focus = Focus::GlobalQuery;
            }
            PanelButton::Tool(PanelTool::CollapseAll) => {
                if g.all_collapsed() {
                    g.collapsed.clear();
                } else {
                    g.collapsed = g.engine.results.iter().map(|f| f.path.clone()).collect();
                }

                if let Some((_, m)) = &mut g.selected {
                    *m = None;
                }

                g.rebuild_rows();
                g.scroll = 0.0;
            }
            PanelButton::Expand => {
                g.replace_open = !g.replace_open;
                self.focus = if g.replace_open {
                    Focus::GlobalReplace
                } else {
                    Focus::GlobalQuery
                };
            }
            PanelButton::Case => {
                g.case_sensitive = !g.case_sensitive;
                self.global_changed(true);
            }
            PanelButton::Word => {
                g.whole_word = !g.whole_word;
                self.global_changed(true);
            }
            PanelButton::Regex => {
                g.regex = !g.regex;
                self.global_changed(true);
            }
            PanelButton::Ignored => {
                g.include_ignored = !g.include_ignored;
                self.global_changed(true);
            }
            PanelButton::Details => {
                g.details_open = !g.details_open;

                if g.details_open {
                    self.focus = Focus::GlobalInclude;
                }
            }
            PanelButton::ReplaceAll => self.ask_replace_all(),
        }

        self.clamp_results_scroll();
        self.reset_blink();
        self.request_redraw();
    }

    /// 弹出确认框，用户确认后执行全部替换。
    ///
    /// 未打开的文件会直接写回磁盘且无法撤销，因此须先确认；确认后经事件循环进入
    /// [`App::replace_all_confirmed`]。搜索未完成、结果被截断或不全、没有结果时不弹出确认框，
    /// 改为在状态栏提示原因。
    pub(in crate::app) fn ask_replace_all(&mut self) {
        let g = &self.global;

        if g.replacing.is_some() {
            self.flash(t!("workspace-replace-all-busy"));
        } else if g.due.is_some() || g.engine.is_running() {
            self.flash(t!("workspace-replace-all-searching"));
        } else if g.engine.is_truncated() {
            self.flash(t!(
                "workspace-replace-all-too-many",
                max = winc_editor::MAX_RESULTS
            ));
        } else if g.engine.gave_up() {
            self.flash(t!("editor-replace-all-gave-up"));
        } else if g.engine.total() == 0 {
            self.flash(t!("workspace-replace-all-nothing"));
        } else {
            self.confirm_replace_all();
        }

        self.request_redraw();
    }

    /// 弹出全部替换的确认框。
    ///
    /// 主文本写明文件数、匹配数和替换文字，说明文本提示操作无法撤销。
    /// 「替换」为默认按钮（`↩`），「取消」对应 `Esc`。
    fn confirm_replace_all(&mut self) {
        let total = self.global.engine.total();
        if self.window.handle.is_some() {
            use crate::app::AlertButton;
            let files = self.global.engine.results.len();
            let replacement = self.global.replace.text();
            let message = if replacement.is_empty() {
                t!(
                    "workspace-replace-all-confirm-delete",
                    files = files,
                    total = total
                )
            } else {
                // 替换文字完整显示、不截短，便于用户核对；过长时提示框自动折行。
                t!(
                    "workspace-replace-all-confirm",
                    files = files,
                    total = total,
                    replacement = replacement
                )
            };
            let buttons = vec![
                AlertButton::new(t!("workspace-replace-all-button")),
                AlertButton::cancel(t!("common-cancel")),
            ];
            let warning = t!("workspace-replace-all-warning");
            self.show_alert(&message, warning, buttons, None, move |index, _| {
                (index == 0).then_some(UserEvent::ReplaceAllConfirmed(total))
            });
            return;
        }

        // 没有窗口（测试中）时直接替换。
        self.replace_all_confirmed(total);
    }

    /// 处理确认框上的全部替换确认。
    ///
    /// 仅当结果与弹出确认框时相同（期间没有重新搜索、总处数不变）时替换；
    /// 否则在状态栏说明原因，不替换用户未确认的结果。
    pub(in crate::app) fn replace_all_confirmed(&mut self, total: usize) {
        let g = &self.global;
        if g.due.is_some() || g.engine.is_running() || g.engine.total() != total {
            self.flash(t!("workspace-replace-all-changed"));
            return;
        }
        self.replace_all_global();
    }

    /// 替换结果列表中剩余的全部匹配，在状态栏提示结果，之后重新搜索。
    ///
    /// 已打开的文件在文档中替换，在界面线程完成；没有打开的文件需要逐个读取、写回磁盘，文件数可达
    /// 数千个，在后台完成，结束后由 [`App::poll_replace_all`] 提示结果。
    pub(in crate::app) fn replace_all_global(&mut self) {
        let Some(query) = self.global.engine.query().cloned() else {
            return;
        };
        // 只替换结果列表中剩余的匹配；用户已从结果中移除的不替换。结果替换完就作废（最后重新
        // 搜索），直接取走，不复制；引擎随即清空，结果行跟着重排，即使之后没能重新搜索也不会
        // 指向已取走的结果。
        let expected = self.global.engine.total();
        let files_left = std::mem::take(&mut self.global.engine.results);
        self.global.engine.clear();
        self.global.rebuild_rows();
        let replacement = self.global.replace.text().to_owned();
        self.global.replace.remember();

        let mut tally = ReplaceTally::default();
        let mut closed = Vec::new();
        for file in files_left {
            match self.document_at(&file.path) {
                Some(doc) => {
                    let result = replace_in_document(doc, &query, &replacement, &file.matches)
                        .map_err(str::to_owned);
                    tally.add(&file.path, result, true);
                }
                None => closed.push(file),
            }
        }
        if closed.is_empty() {
            self.finish_replace_all(tally, expected);
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.global.replacing = Some((receiver, expected));
        self.spawn_then(UserEvent::Replaced, move || {
            for file in &closed {
                let result = replace_in_file(&file.path, &query, &replacement, &file.matches);
                tally.add(&file.path, result, false);
            }
            let _ = sender.send(tally);
        });
    }

    /// 收取后台完成的替换，提示结果：单个文件的替换（[`App::replace_row`]）和跨文件替换。
    pub(in crate::app) fn poll_replace_all(&mut self) {
        let mut rows = Vec::new();
        self.global
            .row_replacing
            .retain(|receiver| match receiver.try_recv() {
                Ok(done) => {
                    rows.push(done);
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => false,
            });
        for (path, result) in rows {
            self.row_replaced(path, result);
        }

        let Some((receiver, expected)) = &self.global.replacing else {
            return;
        };
        let Ok(tally) = receiver.try_recv() else {
            return;
        };
        let expected = *expected;
        self.global.replacing = None;
        self.finish_replace_all(tally, expected);
    }

    /// 在状态栏提示跨文件替换的结果 `tally`，之后重新搜索；`expected` 是替换前结果列表中的总处数。
    fn finish_replace_all(&mut self, tally: ReplaceTally, expected: usize) {
        let ReplaceTally {
            files,
            count,
            opened,
            failed,
        } = tally;
        let mut message = t!("workspace-replace-all-done", files = files, count = count);

        if opened > 0 {
            message.push_str(&t!("workspace-replace-all-unsaved", opened = opened));
        }

        let stale = expected.saturating_sub(count);
        if failed.is_empty() && stale > 0 {
            message.push_str(&t!("workspace-replace-all-stale", stale = stale));
        }

        if let Some(first) = failed.first() {
            message.push_str(&t!(
                "workspace-replace-all-failed",
                count = failed.len(),
                error = first.as_str()
            ));
        }

        self.flash(message);
        self.run_global_search();
    }

    /// 替换文件 `path` 中 `matches` 对应的匹配：已打开的文件改在文档里，没有打开的写回磁盘。
    ///
    /// 成功时返回替换的处数，以及文件是否已打开。
    ///
    /// # Errors
    ///
    /// 替换失败时（例如文件无法读写）返回失败原因。
    fn replace_in_path(
        &mut self,
        path: &Path,
        query: &Query,
        replacement: &str,
        matches: &[LineMatch],
    ) -> Result<(usize, bool), String> {
        match self.document_at(path) {
            Some(doc) => replace_in_document(doc, query, replacement, matches)
                .map(|n| (n, true))
                .map_err(str::to_owned),
            None => replace_in_file(path, query, replacement, matches).map(|n| (n, false)),
        }
    }

    /// 返回路径为 `path` 的已打开文档；尚未读取的不算，它的内容以磁盘为准。
    fn document_at(&mut self, path: &Path) -> Option<&mut Document> {
        self.editor
            .documents
            .iter_mut()
            .find(|d| d.path.as_deref() == Some(path) && !d.is_pending())
    }

    /// 返回有未保存修改的文档的路径和当前内容（rope 共享持有，不复制），项目搜索据此按内存中的
    /// 内容搜索这些文件。
    ///
    /// 未命名的文档不在其中。
    pub(in crate::app) fn dirty_ropes(&self) -> Vec<(PathBuf, ropey::Rope)> {
        self.editor
            .documents
            .iter()
            .filter(|d| d.is_dirty())
            .filter_map(|d| Some((d.path.clone()?, d.text().rope()?.clone())))
            .collect()
    }

    /// 返回路径为 `path`、有未保存修改的文档的当前内容；没有这样的文档时返回 `None`。
    fn dirty_rope(&self, path: &Path) -> Option<ropey::Rope> {
        self.editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path))
            .filter(|d| d.is_dirty())
            .and_then(|d| d.text().rope().cloned())
    }

    /// 替换第 `index` 个结果行：文件行替换该文件中剩余的全部匹配，匹配行只替换这一处。
    ///
    /// 已打开的文件在文档中替换。没有打开的文件需要读取、替换并写回磁盘（最大可达 4 GB），
    /// 在后台进行：这些匹配先从结果中移除，完成后由 [`App::row_replaced`] 提示结果，
    /// 并在后台重新搜索该文件。
    fn replace_row(&mut self, index: usize) {
        let Some(&row) = self.global.rows().get(index) else {
            return;
        };
        let Some(query) = self.global.engine.query().cloned() else {
            return;
        };
        let file = &self.global.engine.results[row.file()];
        let path = file.path.clone();
        let matches = match row {
            ResultRow::File(_) => file.matches.clone(),
            ResultRow::Match(_, m) => vec![file.matches[m].clone()],
        };
        let replacement = self.global.replace.text().to_owned();

        if self.document_at(&path).is_none() {
            match row {
                ResultRow::File(_) => self.global.engine.remove_file(&path),
                ResultRow::Match(_, m) => self.global.engine.remove_match(&path, m),
            }
            self.after_results_changed(index);
            let (sender, receiver) = mpsc::channel();
            self.global.row_replacing.push(receiver);
            self.spawn_then(UserEvent::Replaced, move || {
                let result = replace_in_file(&path, &query, &replacement, &matches);
                let _ = sender.send((path, result));
            });
            return;
        }
        let result = self
            .replace_in_path(&path, &query, &replacement, &matches)
            .map(|(n, _)| n > 0);

        match result {
            Ok(true) => {}
            Ok(false) => self.flash(t!("workspace-replace-stale")),
            Err(e) => self.notify(t!("workspace-replace-failed", error = e.to_string())),
        }

        // 只重新搜索这一个文件，避免结果列表被整个清空再重建。
        let rope = self.dirty_rope(&path);
        self.global.engine.refresh_file(&path, rope.as_ref());
        self.after_results_changed(index);
    }

    /// 后台替换完没有打开的文件 `path`：替换失败或匹配已经过时时提示，之后在后台重新搜索该文件。
    fn row_replaced(&mut self, path: PathBuf, result: Result<usize, String>) {
        match result {
            Ok(0) => self.flash(t!("workspace-replace-stale")),
            Ok(_) => {}
            Err(e) => self.notify(t!("workspace-replace-failed", error = e)),
        }
        self.global.engine.refresh_files([(path, None)]);
        self.request_redraw();
    }

    /// 把第 `index` 个结果行从结果中移除（不改文件），用于全部替换前排除不想修改的位置。
    pub(in crate::app) fn dismiss_row(&mut self, index: usize) {
        let Some(&row) = self.global.rows().get(index) else {
            return;
        };
        let path = self.global.engine.results[row.file()].path.clone();

        match row {
            ResultRow::File(_) => self.global.engine.remove_file(&path),
            ResultRow::Match(_, m) => self.global.engine.remove_match(&path, m),
        }

        self.after_results_changed(index);
    }

    /// 处理 `paths` 中的文件在磁盘上的变化（被其他程序修改或删除）。
    ///
    /// 只重新搜索结果中包含的文件，已删除的从结果中移除。不在结果中的文件（包括新建的文件
    /// 和内容改变后才匹配的文件）不处理，需要时由用户手动重新搜索。重新搜索在后台进行，搜完
    /// 由 [`Self::poll_global`] 更新结果列表；整体搜索进行中时不处理，规则见
    /// [`winc_editor::ProjectSearch::refresh_files`]。
    pub(super) fn results_touched(&mut self, paths: &[PathBuf]) {
        // 切换分支时一次可能变化上千个文件，结果中也可能有上千个文件：用集合查找，避免逐对比较。
        let in_results: HashSet<&Path> = self
            .global
            .engine
            .results
            .iter()
            .map(|f| f.path.as_path())
            .collect();
        let touched: Vec<PathBuf> = paths
            .iter()
            .filter(|p| in_results.contains(p.as_path()))
            .cloned()
            .collect();
        if touched.is_empty() {
            return;
        }
        let files: Vec<_> = touched
            .into_iter()
            .map(|path| {
                let rope = self.dirty_rope(&path);
                (path, rope)
            })
            .collect();
        self.global.engine.refresh_files(files);
        self.request_redraw();
    }

    /// 结果行数变化之后：重新生成结果行，并选中原位置（序号 `index`，超出时取最后一行）上的行。
    fn after_results_changed(&mut self, index: usize) {
        self.global.rebuild_rows();
        let rows = self.global.rows().len();

        if rows == 0 {
            self.global.selected = None;
        } else {
            self.global.select_row(index.min(rows - 1));
        }

        self.clamp_results_scroll();
        self.request_redraw();
    }

    /// 处理对第 `index` 个结果行的单击：文件行折叠或展开，匹配行打开文件并选中该处。
    pub(in crate::app) fn click_result(&mut self, index: usize) {
        self.focus = Focus::Results;
        self.global.select_row(index);

        match self.global.rows().get(index) {
            Some(&ResultRow::File(f)) => {
                let path = &self.global.engine.results[f].path;
                let collapse = !self.global.collapsed.contains(path);
                self.set_result_collapsed(f, collapse);
            }
            Some(&ResultRow::Match(..)) => self.open_result(index, false),
            None => {}
        }

        self.request_redraw();
    }

    /// 打开第 `index` 个结果行所在的文件；是匹配行时选中该处并滚动到中间。
    ///
    /// `focus_editor` 为 `false` 时焦点留在结果列表。
    pub(in crate::app) fn open_result(&mut self, index: usize, focus_editor: bool) {
        let Some(&row) = self.global.rows().get(index) else {
            return;
        };
        let file = &self.global.engine.results[row.file()];
        let path = file.path.clone();
        let target = match row {
            ResultRow::Match(_, m) => file.matches.get(m).map(|m| (m.line, m.cols.clone())),
            ResultRow::File(_) => None,
        };

        self.open_path_then(&path, move |app| {
            let Some((line, cols)) = target else {
                return;
            };
            app.center_line(line);

            if let Some(doc) = app.editor.active() {
                let text = doc.text();
                let line = line.min(text.line_count().saturating_sub(1));
                let start = text.line_start(line);
                let len = text.line_len(line);
                // 跨行匹配的结尾超出所在行，因此结尾位置不按行长截断。
                let end = (start + cols.end).min(text.end_pos());
                app.select(start + cols.start.min(len), end);
            }
        });

        self.focus = if focus_editor {
            Focus::Editor
        } else {
            Focus::Results
        };
        self.request_redraw();
    }

    /// 第 `line` 行不在可见范围内时，滚动到使它位于中间。
    fn center_line(&mut self, line: usize) {
        self.scroll_to_line(line, 0.5, true);
    }

    /// 在结果列表中移动选中的行，`delta` 为移动的行数（正数向下，负数向上），到两端后绕回。
    ///
    /// `open` 为 `true` 时同时打开选中的匹配。
    pub(in crate::app) fn step_result(&mut self, delta: isize, open: bool) {
        let rows = self.global.rows().len();

        if rows == 0 {
            return;
        }

        let index = match self.global.selected_row() {
            Some(i) => (i as isize + delta).rem_euclid(rows as isize) as usize,
            None if delta < 0 => rows - 1,
            None => 0,
        };

        self.global.select_row(index);
        self.scroll_result_into_view(index);

        if open && matches!(self.global.rows()[index], ResultRow::Match(..)) {
            self.open_result(index, false);
        }

        self.request_redraw();
    }

    /// 选中第 `index` 个结果行并滚动到可见；超出范围时停在最后一行，不绕回。
    ///
    /// 供 `Home`、`End`、`PageUp`、`PageDown` 使用，行为与文件树相同。
    pub(in crate::app) fn jump_result(&mut self, index: usize) {
        let rows = self.global.rows().len();
        if rows == 0 {
            return;
        }
        let index = index.min(rows - 1);
        self.global.select_row(index);
        self.scroll_result_into_view(index);
        self.request_redraw();
    }

    /// 返回结果列表翻一页移动的行数。
    ///
    /// 取可见整行数减一，使上一页的最后一行在翻页后仍然可见。
    pub(in crate::app) fn result_page(&self) -> usize {
        let visible = (self.panel_parts().results.h / TREE_ROW).floor() as usize;
        visible.saturating_sub(1).max(1)
    }

    /// 跳到下一处匹配（`backwards` 为 `true` 时为上一处），跳过文件行（`F4` / `⇧F4`）。
    ///
    /// 已折叠的文件会先全部展开。
    pub(in crate::app) fn next_result(&mut self, backwards: bool) {
        if self.global.engine.results.is_empty() {
            return;
        }

        // 已折叠的文件中的匹配也必须能跳到，因此先全部展开。
        if !self.global.collapsed.is_empty() {
            self.global.collapsed.clear();
            self.global.rebuild_rows();
        }

        let delta = if backwards { -1 } else { 1 };
        self.step_result(delta, false);

        if matches!(
            self.global.selected_row().map(|i| self.global.rows()[i]),
            Some(ResultRow::File(_))
        ) {
            self.step_result(delta, false);
        }

        if let Some(index) = self.global.selected_row() {
            self.open_result(index, false);
        }
    }

    /// 输入框中的条件是否与当前结果不一致（条件已修改但尚未搜索）。
    fn global_stale(&self) -> bool {
        let g = &self.global;
        g.due.is_some() || g.engine.query() != Some(&g.current_query())
    }

    /// 处理查找输入框中的 `↩` / `⇧↩`：跳到下一处、上一处，焦点留在输入框。
    ///
    /// 条件改了还没有搜索时，先搜索。
    pub(in crate::app) fn step_from_field(&mut self, backwards: bool) {
        if self.global_stale() {
            self.run_global_search();
            return;
        }

        let focus = self.focus;
        self.next_result(backwards);
        self.focus = focus;
    }

    /// 处理替换输入框中的 `↩`：选中的是一处匹配时替换它并跳到下一处；没有选中匹配时只选中下一处，
    /// 再次执行才替换，使用户先看到再替换。
    ///
    /// 焦点留在替换输入框。
    pub(in crate::app) fn replace_from_field(&mut self) {
        if self.global_stale() {
            self.run_global_search();
            return;
        }

        let focus = self.focus;
        let selected = self.global.selected_row();

        match selected.map(|i| (i, self.global.rows()[i])) {
            Some((index, ResultRow::Match(..))) => {
                self.replace_row(index);

                // 替换后原位置上是下一处匹配；该文件的匹配都已替换时，
                // 原位置可能是下一个文件的文件行。
                match self.global.selected_row() {
                    Some(i) if matches!(self.global.rows()[i], ResultRow::Match(..)) => {
                        self.open_result(i, false);
                    }
                    Some(_) => self.next_result(false),
                    None => {}
                }
            }
            _ => self.next_result(false),
        }

        self.focus = focus;
    }

    /// 执行结果行菜单里的命令 `command`：作用于文件 `path` 的文件行（`m` 为 `None`），或其中
    /// 第 `m` 处匹配。
    ///
    /// 弹出菜单以后结果变了、这一行已经不在时什么也不做。
    pub(in crate::app) fn run_search_result_command(
        &mut self,
        command: MenuCommand,
        path: PathBuf,
        m: Option<usize>,
    ) {
        self.global.selected = Some((path.clone(), m));
        let Some(index) = self.global.selected_row() else {
            return;
        };
        match command {
            MenuCommand::OpenFiles => self.open_result(index, true),
            MenuCommand::CopyPath | MenuCommand::CopyRelativePath => {
                self.copy_path(&path, command == MenuCommand::CopyRelativePath);
            }
            MenuCommand::RevealInTree => {
                self.show_files();
                self.reveal_path(&path);
            }
            MenuCommand::Copy => self.copy_result_line(index),
            MenuCommand::ReplaceResult(_) => self.replace_row(index),
            MenuCommand::DismissResult => self.dismiss_row(index),
            _ => {}
        }
    }

    /// 把第 `index` 个结果行（匹配行）所在那一行的文字复制到剪贴板，不带行尾。
    ///
    /// 预览去掉了行首空白、可能截去两端，所以取原文：文件打开着时取文档里的（可能还没保存），
    /// 否则读磁盘上的文件；都取不到（超大文件、刚被删掉）时才用预览。
    fn copy_result_line(&mut self, index: usize) {
        let Some(&ResultRow::Match(f, m)) = self.global.rows().get(index) else {
            return;
        };
        let file = &self.global.engine.results[f];
        let found = &file.matches[m];
        let text = self
            .line_text(&file.path, found.line)
            .unwrap_or_else(|| found.preview.clone());
        self.set_clipboard(text, false);
        self.flash(t!("workspace-search-line-copied"));
    }

    /// 返回文件 `path` 第 `line` 行（从 0 起）的文字，不带行尾；见 [`Self::copy_result_line`]。
    fn line_text(&self, path: &Path, line: usize) -> Option<String> {
        let docs = &self.editor.documents;
        if let Some(doc) = docs
            .iter()
            .find(|d| d.path.as_deref() == Some(path) && !d.is_pending())
        {
            let rope = doc.text().rope()?;
            return (line < rope.len_lines()).then(|| line_slice(rope, line).to_string());
        }
        let bytes = std::fs::read(path).ok()?;
        let (text, _) = winc_editor::encoding::decode_unknown(&bytes);
        text.lines().nth(line).map(str::to_owned)
    }

    /// 处理第 `index` 个结果行上悬停时出现的按钮 `action` 的点击。
    pub(in crate::app) fn press_row_action(&mut self, index: usize, action: RowAction) {
        match action {
            RowAction::Replace => self.replace_row(index),
            RowAction::Dismiss => self.dismiss_row(index),
        }
    }

    /// 返回侧栏中可依次用 `Tab` 切换的焦点：输入框，以及有结果时的结果列表。
    fn panel_fields(&self) -> Vec<Focus> {
        let g = &self.global;
        let mut fields = vec![Focus::GlobalQuery];

        if g.replace_open {
            fields.push(Focus::GlobalReplace);
        }

        if g.details_open {
            fields.extend([Focus::GlobalInclude, Focus::GlobalExclude]);
        }

        if !g.rows().is_empty() {
            fields.push(Focus::Results);
        }

        fields
    }

    /// 处理 `Tab` / `⇧Tab`：在面板的输入框和结果列表之间切换焦点。
    pub(in crate::app) fn cycle_panel_focus(&mut self, backwards: bool) {
        let fields = self.panel_fields();
        let at = fields.iter().position(|&f| f == self.focus).unwrap_or(0);
        let n = fields.len() as isize;
        let next = (at as isize + if backwards { -1 } else { 1 }).rem_euclid(n);
        self.focus = fields[next as usize];

        if self.focus == Focus::Results && self.global.selected.is_none() {
            self.global.select_row(0);
        }

        self.field_preedit.clear();
        self.reset_blink();
        self.request_redraw();
    }
}
