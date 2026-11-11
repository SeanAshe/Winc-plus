//! 快速打开面板（`⌘P`）：面板的开关、查询、选择，以及最近打开的文件。
//!
//! 输入框内容以 `>` 开头时进入命令面板（见 `palette.rs`），以 `@` 开头时列出当前文件的符号。

use super::*;
use crate::layout::PickerParts;

/// 结果列表中一行的高度（逻辑像素）。
///
/// 取 24：容纳一行正文文字并留出上下留白，与文件树的行高相同。
pub const PICKER_ROW: f32 = 24.0;
/// 结果列表上下的留白（逻辑像素）。
///
/// 第一条与输入框下方的分隔线、最后一条与卡片底边之间留出此距离。
pub const PICKER_PAD: f32 = 6.0;
/// 文件图标左边缘到卡片左边缘的距离（逻辑像素）。
///
/// 输入框行的放大镜与结果行的文件图标在竖直方向上对齐。
pub const PICKER_ICON_X: f32 = 14.0;
/// 文字左边缘到卡片左边缘的距离（逻辑像素）。
///
/// 输入框中输入的文字与结果行的名字在竖直方向上对齐。
pub const PICKER_TEXT_X: f32 = 40.0;

/// 结果列表最多同时显示的行数（行）；结果更多时滚动。
pub const PICKER_MAX_ROWS: usize = 10;
/// 位置浮窗（定义、用法、书签）底栏的高度（逻辑像素）：显示选中项的完整路径。
pub const PLACES_FOOTER: f32 = 28.0;
/// 位置浮窗的宽度（逻辑像素）：放得下文件名、行号和一行代码；窄窗口下随编辑区收窄。
const PLACES_WIDTH: f32 = 720.0;
/// 位置浮窗与名字所在行之间的间距（逻辑像素）。
const PLACES_GAP: f32 = 4.0;

/// 位置浮窗关闭时的状态，淡出动画据此绘制（此时面板状态已清空）。
pub struct PlacesGhost {
    /// 列出的位置。
    pub places: editor::definition::Places,
    /// 筛选后留下的条目序号。
    pub hits: Vec<usize>,
    /// 选中项在 `hits` 中的序号。
    pub selected: usize,
    /// 列表的滚动量（逻辑像素）。
    pub scroll: f32,
    /// 筛选文字。
    pub filter: String,
    /// 浮窗的位置。
    pub rect: Rect,
    /// 淡出的动画时钟。
    pub clock: Clock,
}

/// 面板高度的过渡：结果数量变化时，卡片的下边沿从原高度滑向新高度。
///
/// 时长为 [`PICKER_ANIMATION`]，缓动为 [`Clock`] 的三次缓出。
#[derive(Debug, Clone, Copy)]
pub(in crate::app) struct PickerMotion {
    /// 起始高度（逻辑像素）。
    pub(in crate::app) from: f32,
    /// 目标高度（逻辑像素）。
    pub(in crate::app) to: f32,
    /// 过渡的时钟。
    pub(in crate::app) clock: Clock,
}

impl PickerMotion {
    /// 返回此刻卡片的高度（逻辑像素）。
    pub(in crate::app) fn height(&self) -> f32 {
        self.clock.between(self.from, self.to)
    }

    /// 创建从此刻的高度滑向 `to` 的过渡。
    ///
    /// 上一段过渡 `motion` 尚未结束时，从该过渡当前的高度继续，不跳回起点；没有过渡时从
    /// 上一次的高度 `last` 开始。
    pub(in crate::app) fn toward(
        motion: Option<PickerMotion>,
        last: f32,
        to: f32,
        clock: Clock,
    ) -> PickerMotion {
        PickerMotion {
            from: motion.map_or(last, |m| m.height()),
            to,
            clock,
        }
    }
}

/// 把快速打开（`⌘P`）的查询拆成文件名和末尾的 `:行[:列]`。
///
/// 例如 `main.rs:42` 拆成（`main.rs`，`42`），`main.rs:42:7` 拆成（`main.rs`，`42:7`），
/// `:42` 拆成（空，`42`）。冒号后面的部分不是数字时不拆分。
pub(super) fn split_goto(query: &str) -> (&str, Option<&str>) {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let Some((head, last)) = query.rsplit_once(':') else {
        return (query, None);
    };
    if !digits(last) {
        return (query, None);
    }
    match head.rsplit_once(':') {
        Some((file, line)) if digits(line) => (file, Some(&query[file.len() + 1..])),
        _ => (head, Some(last)),
    }
}

impl App {
    /// 返回文件列表第 `i` 行（`Picker::hits` 中的下标）路径里命中查询的字符下标，绘制时标出
    /// 这些字符；末尾的 `:行:列` 不参与匹配。
    pub(crate) fn picker_matched(&self, i: usize) -> Vec<usize> {
        let (query, _) = split_goto(self.picker.input.text());
        self.picker
            .hits
            .get(i)
            .map_or_else(Vec::new, |&index| self.picker.index.matched(query, index))
    }

    /// 打开面板；没有打开目录时在状态栏提示。
    ///
    /// 文件表由 `picker_refresh` 在需要时建立；通常已在打开目录时由 `index_folder` 预先建好。
    pub(in crate::app) fn open_picker(&mut self) {
        if self.editor.root().is_none() {
            self.flash(winc_i18n::t!("workspace-open-folder-first"));
            return;
        }

        self.finish_name_input();
        self.picker.places = None;
        self.picker.branch_rows = None;
        self.picker.location_rows = None;
        self.picker.folders = false;
        self.picker.open = true;
        self.picker.doc = self.editor.active().map(|d| d.id);
        self.picker.input.set_text("");
        self.picker.selected = 0;
        self.picker.scroll = 0.0;
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 关闭面板：释放符号列表和上一次查询的命中列表，清除输入法组字，并将焦点交还编辑区。
    ///
    /// 位置浮窗关闭后淡出：按关闭时的状态再绘制 100 ms（见 [`PlacesGhost`]）。
    pub(in crate::app) fn close_picker(&mut self) {
        self.drop_symbols();
        self.picker.index.forget_query();
        self.picker.open = false;
        self.picker.branch_rows = None;
        self.picker.location_rows = None;
        self.picker.folders = false;
        self.hits.picker_parts.clear();
        self.picker.fold = None;
        self.picker.places_in = None;
        if let Some(places) = self.picker.places.take()
            && let Some(clock) = self.clock(POPUP_OUT)
        {
            self.picker.places_out = Some(PlacesGhost {
                places,
                hits: self.picker.hits.clone(),
                selected: self.picker.selected,
                scroll: self.picker.scroll,
                filter: self.picker.input.text().to_owned(),
                rect: self.layout.picker,
                clock,
            });
        }
        self.field_preedit.clear();

        if self.focus == Focus::Picker {
            self.focus = Focus::Editor;
        }

        self.relayout();
        self.request_redraw();
    }

    /// 布局完成后调用：面板的目标高度变化时，从当前高度滑向新高度；动画期间用此刻的高度
    /// 替换布局中的面板高度。
    ///
    /// `size` 为窗口尺寸。
    ///
    /// 刚打开面板、窗口大小变化时直接到位：拖动窗口时每帧尺寸都在变，动画无法跟随。
    pub(in crate::app) fn animate_picker(&mut self, size: (f32, f32)) {
        if !self.picker.open {
            self.picker.height = None;
            self.picker.motion = None;
            return;
        }

        let target = self.layout.picker.h;
        // 位置浮窗和分支、位置选择器都依附于触发位置，筛选时高度直接到位：卡片边沿的滑动会使
        // 列表看起来在滚动。
        if self.picker.places.is_some() || self.picker_is_git() {
            self.picker.motion = None;
            self.picker.height = Some((target, size));
            return;
        }
        match self.picker.height {
            Some((last, at)) if at == size && last != target => {
                let motion = self.picker.motion;
                self.picker.motion = self
                    .clock(PICKER_ANIMATION)
                    .map(|clock| PickerMotion::toward(motion, last, target, clock));
            }
            Some((_, at)) if at != size => self.picker.motion = None,
            _ => {}
        }
        self.picker.height = Some((target, size));

        if let Some(motion) = self.picker.motion {
            self.layout.picker.h = motion.height();
        }
    }

    /// 列出位置（定义、用法、书签）时，将面板改为位于名字旁边的浮窗：在名字所在行的下方，
    /// 左边缘比名字所在的列靠左 12 逻辑像素，使文件图标落在名字下方附近；下方放不下全部行、
    /// 上方空间更多时放在上方。
    ///
    /// 左右不超出编辑区，名字不在屏幕上时贴着编辑区顶部居中。
    pub(in crate::app) fn place_places(&mut self) {
        self.picker.places_bottom = None;
        let Some(places) = self.picker.places.as_ref().filter(|_| self.picker.open) else {
            return;
        };
        let editor = self.layout.editor;
        let w = PLACES_WIDTH.min(editor.w - 24.0).max(0.0);
        let chrome = BAR_HEIGHT + 2.0 * PICKER_PAD + PLACES_FOOTER;
        let fit = |space: f32| ((space - chrome) / PICKER_ROW).floor().max(0.0) as usize;
        let wanted = self.picker.hits.len().clamp(1, PICKER_MAX_ROWS);
        let line_h = self.fonts.line_height;

        let (x, top) = self
            .anchor_point(places.anchor)
            .filter(|&(_, y)| y >= editor.y && y + line_h <= editor.bottom())
            .unwrap_or((
                editor.x + (editor.w - w) / 2.0 + 12.0,
                editor.y + 8.0 - line_h,
            ));
        let below = editor.bottom() - 8.0 - (top + line_h + PLACES_GAP);
        let above = top - PLACES_GAP - (editor.y + 8.0);
        let up = fit(below) < wanted && above > below;
        let rows = wanted.min(fit(if up { above } else { below })).max(1);
        let h = chrome + rows as f32 * PICKER_ROW;
        let right = editor.right() - SCROLLBAR_HIT - 8.0;
        let x = (x - 12.0).min(right - w).max(editor.x + 8.0).round();
        let y = if up {
            self.picker.places_bottom = Some(top - PLACES_GAP);
            top - PLACES_GAP - h
        } else {
            top + line_h + PLACES_GAP
        };
        self.layout.picker = Rect::new(x, y.round(), w, h);
    }

    /// 面板是否为分支选择器或位置选择器。
    pub(crate) fn picker_is_git(&self) -> bool {
        self.picker_lists_branches() || self.picker_lists_locations()
    }

    /// 位置浮窗是否在名字上方。
    pub(crate) fn places_above(&self) -> bool {
        self.picker.places_bottom.is_some()
    }

    /// 返回当前文档中位置 `pos` 在窗口中的横坐标，以及它所在显示行的顶边纵坐标（逻辑像素）。
    pub(crate) fn anchor_point(&self, pos: usize) -> Option<(f32, f32)> {
        let doc = self.editor.active()?;
        let text = self.layout.text;
        let row = doc.row_at(pos) as f64;
        let y = text.y + (row - doc.scroll_line) as f32 * self.fonts.line_height;
        let (line, col) = doc.text().pos_to_line_col(pos);
        let cell = doc.col_to_cell(line, col, self.editor.active_tab_width());
        let x = text.x + TEXT_PAD - doc.scroll_x + cell as f32 * self.cell;
        Some((x, y))
    }

    /// 返回面板布局完成后的位置；动画期间高度取动画终点的高度。
    ///
    /// 滚动范围按该位置计算，不随动画逐帧变化。
    pub(crate) fn picker_rect(&self) -> Rect {
        let mut rect = self.layout.picker;
        if let Some((height, _)) = self.picker.height {
            rect.h = height;
        }
        rect
    }

    /// 按输入框中的文字重新生成结果列表，每次键入都调用。
    pub(in crate::app) fn picker_refresh(&mut self) {
        self.picker.selected = 0;
        self.picker.scroll = 0.0;
        self.picker.recent = 0;
        if self.picker.places.is_some() {
            self.picker.goto = None;
            self.places_refresh();
            return;
        }
        self.picker.fold = None;
        if self.picker_lists_branches() {
            self.picker.goto = None;
            self.drop_symbols();
            self.branch_picker_refresh();
            return;
        }
        if self.picker_lists_locations() {
            self.picker.goto = None;
            self.drop_symbols();
            self.location_picker_refresh();
            return;
        }
        if self.picker.folders {
            self.picker.goto = None;
            self.drop_symbols();
            self.folder_picker_refresh();
            return;
        }
        if self.picker_lists_symbols() {
            self.picker.goto = None;
            self.symbols_refresh();
            return;
        }
        self.drop_symbols();
        if self.picker_lists_commands() {
            self.picker.goto = None;
            self.palette_refresh();
            return;
        }

        // 没有打开目录（已关闭）时，文件表仍属于旧目录，不能列出。
        if self.editor.root().is_none() {
            self.picker.goto = None;
            self.picker.hits.clear();
            return;
        }
        self.prepare_index();
        let (query, goto) = split_goto(self.picker.input.text());
        self.picker.goto = goto.map(str::to_owned);

        // 只写了 `:行` 时在当前文件内跳转，不列出文件。
        if query.trim().is_empty() && self.picker.goto.is_some() {
            self.picker.hits.clear();
            // 计数行不再显示上一次查询的命中数。
            self.picker.index.forget_query();
            return;
        }

        self.picker.hits = self.picker.index.search(query);

        // 没有输入时，最近打开的文件排在最前面；第一条是上一个文件，因此 `⌘P` 加回车即可切回。
        if query.trim().is_empty() {
            let active = self.editor.active().and_then(|d| d.path.as_deref());
            let recent: Vec<usize> = self
                .recent_files
                .iter()
                .filter(|p| Some(p.as_path()) != active)
                .filter_map(|p| self.picker.index.position_of(p))
                .collect();

            self.picker.hits.retain(|i| !recent.contains(i));
            self.picker.recent = recent.len();
            self.picker.hits.splice(0..0, recent);
            self.picker.hits.truncate(winc_editor::PICKER_MAX_RESULTS);
        }
    }

    /// 是否应显示「正在索引文件…」：首次建立文件表且还没有可列出的文件时为 `true`。
    ///
    /// 重新遍历目录时旧表仍然存在，继续列出旧表中的结果（面板高度也按这些结果计算），
    /// 避免只显示一行文字而下方留出大片空白。
    pub(crate) fn picker_indexing(&self) -> bool {
        !self.picker.index.is_ready() && self.picker.index.is_empty()
    }

    /// 打开目录之后，在后台建立 `⌘P` 面板的文件表，使第一次按 `⌘P` 时即可列出文件。
    ///
    /// 目录处于监视状态时，文件表建好后随变化增量更新，不需要重新遍历。建表耗时：大目录约
    /// 1 至 2 秒，整个主目录约 3 秒以上。基准测试模式和没有事件循环时不建立，
    /// 避免干扰启动耗时和内存的测量。
    pub(super) fn index_folder(&mut self) {
        if !self.bench.silent() && self.proxy.is_some() {
            self.prepare_index();
        }
    }

    /// 在列出文件之前确保文件表可用。
    ///
    /// 打开的目录还没有文件表，或窗口失去过焦点（文件可能被增删，`Picker::stale`）时，
    /// 在后台建立新表。
    pub(super) fn prepare_index(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };

        if self.picker.stale && self.picker.index.root() == Some(root.as_path()) {
            self.picker_invalidate();
        }

        if self.picker.index.root() != Some(root.as_path()) {
            self.picker.index.cancel();
            self.build_index(&root);
        }
    }

    /// 后台的文件表建好后调用：换用新表。
    ///
    /// 面板打开时按新表重新列出结果，并保持原来选中的文件（用户可能已用 `↓` 选好，正要按 `↩`）；
    /// 新表中没有该文件时才回到第一条。
    pub(in crate::app) fn picker_indexed(&mut self) {
        // 结果下标只对旧表有效，换表之前先按旧表取出选中项的路径。
        let selected = self
            .picker
            .hits
            .get(self.picker.selected)
            .and_then(|&i| self.picker.index.absolute(i));

        if !self.picker.index.poll() {
            return;
        }

        let files = !self.picker_lists_commands()
            && !self.picker_lists_symbols()
            && !self.picker_lists_branches()
            && !self.picker_lists_locations()
            && !self.picker.folders;
        if self.picker.open && files && self.picker.places.is_none() {
            let scroll = self.picker.scroll;
            self.picker_refresh();

            let at = selected
                .and_then(|path| self.picker.index.position_of(&path))
                .and_then(|position| self.picker.hits.iter().position(|&i| i == position));
            if let Some(at) = at {
                self.picker.selected = at;
                self.picker.scroll = scroll;
                self.picker_reveal();
            }
        }
        self.request_redraw();
    }

    /// 把当前文件记入最近打开列表，供 `⌘P` 面板的「最近打开」使用。
    pub(super) fn note_recent_file(&mut self) {
        // 最多记录的文件数（个）：取一屏行数 `PICKER_MAX_ROWS` 的 3 倍。
        const MAX_RECENT_FILES: usize = 30;

        let Some(path) = self.editor.active().and_then(|d| d.path.clone()) else {
            return;
        };
        self.recent_files.retain(|p| *p != path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(MAX_RECENT_FILES);
    }

    /// 上下移动选中项，`down` 为 `true` 时向下；到达两端后停止，不绕回。
    pub(in crate::app) fn picker_move(&mut self, down: bool) {
        if self.picker.hits.is_empty() {
            return;
        }

        let last = self.picker.hits.len() - 1;
        self.picker.default = None;
        self.picker.selected = if self.picker_lists_branches() {
            self.branch_picker_step(down)
        } else if down {
            (self.picker.selected + 1).min(last)
        } else {
            self.picker.selected.saturating_sub(1)
        };

        self.picker_reveal();
        if self.picker_lists_symbols() {
            self.symbols_preview();
        }
        self.request_redraw();
    }

    /// 滚动结果列表，使选中项进入可见范围。
    pub(in crate::app) fn picker_reveal(&mut self) {
        let height = crate::app::workspace::picker::picker_span(self);
        if height <= 0.0 {
            return;
        }

        let top = self.picker.selected as f32 * PICKER_ROW;
        self.picker.scroll = scroll_to_show(self.picker.scroll, top, top + PICKER_ROW, height);
    }

    /// 确认选中项。
    ///
    /// 文件列表中打开选中的文件（已有标签则切换过去，否则新开一个）；命令面板中执行选中的命令；
    /// 符号列表和位置列表中跳转到选中的位置；最近目录列表中打开选中的目录。
    pub(in crate::app) fn picker_accept(&mut self) {
        if self.picker.places.is_some() {
            self.places_accept();
            return;
        }
        if self.picker_lists_branches() {
            self.branch_picker_accept();
            return;
        }
        if self.picker_lists_locations() {
            self.location_picker_accept();
            return;
        }
        if self.picker.folders {
            self.folder_picker_accept();
            return;
        }
        if self.picker_lists_commands() {
            self.palette_accept();
            return;
        }
        if self.picker_lists_symbols() {
            self.symbols_accept();
            return;
        }
        let goto = self.picker.goto.take();
        let path = self
            .picker
            .hits
            .get(self.picker.selected)
            .and_then(|&index| self.picker.index.absolute(index));

        match (path, goto) {
            (Some(path), goto) => {
                self.close_picker();
                self.open_path_then(&path, move |app| {
                    if let Some(goto) = goto {
                        app.goto_line(&goto);
                    }
                });
            }
            // 只写了 `:行` 时，在当前文件内跳转。
            (None, Some(goto)) if self.editor.active.is_some() => {
                self.close_picker();
                self.focus = Focus::Editor;
                self.goto_line(&goto);
            }
            (None, _) => return,
        }

        self.request_redraw();
    }

    /// 通知文件表：`paths` 中的路径在本程序内被新建、删除、改名或另存为。
    ///
    /// 文件表在后台只更新这几项，不等待目录监视的通知，也不重新遍历整个目录。
    pub(in crate::app) fn picker_touched(&mut self, paths: Vec<PathBuf>) {
        self.picker.index.apply(winc_editor::Changes {
            paths,
            rescan: false,
            git: false,
        });
    }

    /// 让文件表失效并重新遍历整个目录。
    ///
    /// 用户点击刷新目录，或没有监视目录时需要使用 `⌘P`，都会调用。
    pub(super) fn picker_invalidate(&mut self) {
        self.picker.stale = false;
        let Some(root) = self.picker.index.root().map(Path::to_path_buf) else {
            return;
        };

        self.build_index(&root);

        if self.picker.open {
            self.picker_refresh();
        }
    }

    /// 在后台为 `root` 建立文件表，建好后发送 [`UserEvent::Indexed`]。
    ///
    /// 已有旧表时保留旧表，新表建好之前面板仍可使用，不会变空。测试中没有事件循环，同步建立。
    fn build_index(&mut self, root: &Path) {
        let Some(proxy) = self.proxy.clone() else {
            self.picker.index = FileIndex::collect_now(root);
            return;
        };
        if self.picker.index.root() == Some(root) {
            self.picker.index.refresh();
        } else {
            self.picker.index = FileIndex::spawn(root, move || {
                let _ = proxy.send_event(UserEvent::Indexed);
            });
        }
    }
}

/// 返回快速打开卡片（或位置浮窗）中结果列表可显示区域的高度（逻辑像素）。
///
/// 界面还没有排版（没有窗口）时为 0。
pub(crate) fn picker_span(app: &App) -> f32 {
    picker_parts(app, app.picker_rect()).list.h
}

/// 返回卡片 `card` 的各部件：列出位置（定义、用法、书签）时是带底栏的位置浮窗，否则是快速
/// 打开面板。
///
/// 绘制、点击和滚动都用这里的几何。
pub(crate) fn picker_parts(app: &App, card: Rect) -> PickerParts {
    if app.picker.places.is_some() {
        PickerParts::places(card)
    } else if app.picker_is_git() {
        use crate::app::git::branch::picker::GIT_PICKER_ACTIONS;
        let actions = if app.picker_has_actions() {
            GIT_PICKER_ACTIONS
        } else {
            0.0
        };
        PickerParts::menu(card, actions, app.git_picker_footer())
    } else {
        PickerParts::new(card)
    }
}

/// 快速打开面板与位置浮窗的状态：文件索引、查询、结果列表，以及符号、位置、分支等列表模式。
pub struct Picker {
    /// 工作区中各文件的路径与匹配用的索引。
    pub(crate) index: FileIndex,
    /// 快速打开面板是否打开。
    pub(crate) open: bool,
    /// 面板打开时的当前文档；当前文档变化后关闭面板（见 `relayout`）。
    pub(crate) doc: Option<DocumentId>,
    /// 快速打开面板中的查询输入。
    pub(crate) input: TextField,
    /// 当前查询命中的文件索引下标。
    pub(crate) hits: Vec<usize>,
    /// 当前选中的结果在 `Picker::hits` 中的序号（从 0 起）。
    pub(crate) selected: usize,
    /// 分支选择器打开时自动选中的行（`Picker::selected` 的值）；移动选中项或输入查询后为 `None`。
    ///
    /// 自动选中的行不一定是当前分支：只列出最近使用的分支时，默认选中上一个用过的分支，
    /// 回车即切回。该行因此以浅色底显示，避免强调色底被误认为当前分支；当前分支由行首的对勾标出。
    pub(crate) default: Option<usize>,
    /// 结果开头属于「最近打开」的条数（未输入查询时列在最前面）。
    pub(crate) recent: usize,
    /// 查询末尾的 `:行` 或 `:行:列`，打开文件后跳转到该位置。
    pub(crate) goto: Option<String>,
    /// 是否需要在下次打开 ⌘P 时于后台重新扫描文件表。
    ///
    /// 目录监视不可用时，窗口重新获得焦点后目录内容可能已有增删。
    pub(crate) stale: bool,
    /// 结果列表的滚动位置（像素）。
    pub(crate) scroll: f32,
    /// 在快速打开中输入 `@` 时列出的当前文件的符号。
    pub(crate) symbols: Option<crate::app::editor::symbols::SymbolList>,
    /// 转到定义找到多个位置时，卡片中列出的位置。
    pub(crate) places: Option<crate::app::editor::definition::Places>,
    /// 面板列出分支（分支选择器）时的各行。
    pub(crate) branch_rows: Option<Vec<crate::app::git::branch::picker::BranchRow>>,
    /// 面板列出位置（位置选择器）时的各行。
    pub(crate) location_rows: Option<Vec<crate::app::git::location_picker::LocationRow>>,
    /// 面板是否列出最近打开的目录（`⌃R`）；为真时 `Picker::hits` 是最近列表中的下标。
    pub(crate) folders: bool,
    /// 分支选择器、位置选择器的展开位置。
    pub(crate) anchor: GitAnchor,
    /// ⌘P 面板上次排版时的高度，以及当时的窗口大小。
    pub(crate) height: Option<(f32, (f32, f32))>,
    pub(in crate::app) motion: Option<PickerMotion>,
    /// 分支、位置选择器中目录或分组展开、收起的动画。
    pub(crate) fold: Option<crate::app::git::branch::picker::PickerFold>,
    /// `⌘P` 面板出现、消失的动画。
    pub(crate) card: crate::app::animation::CardFx,
    /// 位置浮窗（定义、用法、书签）位于名字上方时浮窗底边的纵坐标。
    ///
    /// 高度变化时底边保持不动、顶边移动；浮窗位于下方或未打开时为 `None`。
    pub(in crate::app) places_bottom: Option<f32>,
    /// 位置浮窗中尚未累计满一行的滚轮滚动量（逻辑像素）；浮窗按整行滚动。
    pub(in crate::app) places_wheel: f32,
    /// 位置浮窗出现的动画（滑出并淡入）；动画结束后为 `None`。
    pub(crate) places_in: Option<crate::app::animation::Clock>,
    /// 位置浮窗关闭后的淡出状态；不在淡出时为 `None`。
    pub(crate) places_out: Option<PlacesGhost>,
}

impl Default for Picker {
    fn default() -> Self {
        Picker {
            index: FileIndex::default(),
            open: false,
            doc: None,
            input: TextField::default(),
            hits: Vec::new(),
            selected: 0,
            default: None,
            recent: 0,
            goto: None,
            stale: false,
            scroll: 0.0,
            symbols: None,
            places: None,
            branch_rows: None,
            location_rows: None,
            folders: false,
            anchor: GitAnchor::Branch,
            height: None,
            motion: None,
            fold: None,
            card: Default::default(),
            places_bottom: None,
            places_wheel: 0.0,
            places_in: None,
            places_out: None,
        }
    }
}

#[cfg(test)]
mod tests;
