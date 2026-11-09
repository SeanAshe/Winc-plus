//! 快速打开面板（`⌘P`）：面板的开关、查询、选择，以及最近打开的文件。
//!
//! 输入框内容以 `>` 开头时进入命令面板（见 `palette.rs`），以 `@` 开头时列出当前文件的符号。

use super::*;

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
    /// 返回文件列表第 `i` 行（`picker_hits` 中的下标）路径里命中查询的字符下标，绘制时标出
    /// 这些字符；末尾的 `:行:列` 不参与匹配。
    pub(crate) fn picker_matched(&self, i: usize) -> Vec<usize> {
        let (query, _) = split_goto(self.picker_input.text());
        self.picker_hits
            .get(i)
            .map_or_else(Vec::new, |&index| self.picker.matched(query, index))
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
        self.picker_places = None;
        self.picker_branch_rows = None;
        self.picker_location_rows = None;
        self.picker_folders = false;
        self.picker_open = true;
        self.picker_doc = self.editor.active().map(|d| d.id);
        self.picker_input.set_text("");
        self.picker_selected = 0;
        self.picker_scroll = 0.0;
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
        self.picker.forget_query();
        self.picker_open = false;
        self.picker_branch_rows = None;
        self.picker_location_rows = None;
        self.picker_folders = false;
        self.picker_part_hits.clear();
        self.picker_fold = None;
        self.places_in = None;
        if let Some(places) = self.picker_places.take()
            && let Some(clock) = self.clock(POPUP_OUT)
        {
            self.places_out = Some(PlacesGhost {
                places,
                hits: self.picker_hits.clone(),
                selected: self.picker_selected,
                scroll: self.picker_scroll,
                filter: self.picker_input.text().to_owned(),
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
        if !self.picker_open {
            self.picker_height = None;
            self.picker_motion = None;
            return;
        }

        let target = self.layout.picker.h;
        // 位置浮窗和分支、位置选择器都依附于触发位置，筛选时高度直接到位：卡片边沿的滑动会使
        // 列表看起来在滚动。
        if self.picker_places.is_some() || self.picker_is_git() {
            self.picker_motion = None;
            self.picker_height = Some((target, size));
            return;
        }
        match self.picker_height {
            Some((last, at)) if at == size && last != target => {
                let motion = self.picker_motion;
                self.picker_motion = self
                    .clock(PICKER_ANIMATION)
                    .map(|clock| PickerMotion::toward(motion, last, target, clock));
            }
            Some((_, at)) if at != size => self.picker_motion = None,
            _ => {}
        }
        self.picker_height = Some((target, size));

        if let Some(motion) = self.picker_motion {
            self.layout.picker.h = motion.height();
        }
    }

    /// 列出位置（定义、用法、书签）时，将面板改为位于名字旁边的浮窗：在名字所在行的下方，
    /// 左边缘比名字所在的列靠左 12 逻辑像素，使文件图标落在名字下方附近；下方放不下全部行、
    /// 上方空间更多时放在上方。
    ///
    /// 左右不超出编辑区，名字不在屏幕上时贴着编辑区顶部居中。
    pub(in crate::app) fn place_places(&mut self) {
        self.places_bottom = None;
        let Some(places) = self.picker_places.as_ref().filter(|_| self.picker_open) else {
            return;
        };
        let editor = self.layout.editor;
        let w = PLACES_WIDTH.min(editor.w - 24.0).max(0.0);
        let chrome = BAR_HEIGHT + 2.0 * PICKER_PAD + PLACES_FOOTER;
        let fit = |space: f32| ((space - chrome) / PICKER_ROW).floor().max(0.0) as usize;
        let wanted = self.picker_hits.len().clamp(1, PICKER_MAX_ROWS);
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
            self.places_bottom = Some(top - PLACES_GAP);
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
        self.places_bottom.is_some()
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
        if let Some((height, _)) = self.picker_height {
            rect.h = height;
        }
        rect
    }

    /// 按输入框中的文字重新生成结果列表，每次键入都调用。
    pub(in crate::app) fn picker_refresh(&mut self) {
        self.picker_selected = 0;
        self.picker_scroll = 0.0;
        self.picker_recent = 0;
        if self.picker_places.is_some() {
            self.picker_goto = None;
            self.places_refresh();
            return;
        }
        self.picker_fold = None;
        if self.picker_lists_branches() {
            self.picker_goto = None;
            self.drop_symbols();
            self.branch_picker_refresh();
            return;
        }
        if self.picker_lists_locations() {
            self.picker_goto = None;
            self.drop_symbols();
            self.location_picker_refresh();
            return;
        }
        if self.picker_folders {
            self.picker_goto = None;
            self.drop_symbols();
            self.folder_picker_refresh();
            return;
        }
        if self.picker_lists_symbols() {
            self.picker_goto = None;
            self.symbols_refresh();
            return;
        }
        self.drop_symbols();
        if self.picker_lists_commands() {
            self.picker_goto = None;
            self.palette_refresh();
            return;
        }

        // 没有打开目录（已关闭）时，文件表仍属于旧目录，不能列出。
        if self.editor.root().is_none() {
            self.picker_goto = None;
            self.picker_hits.clear();
            return;
        }
        self.prepare_index();
        let (query, goto) = split_goto(self.picker_input.text());
        self.picker_goto = goto.map(str::to_owned);

        // 只写了 `:行` 时在当前文件内跳转，不列出文件。
        if query.trim().is_empty() && self.picker_goto.is_some() {
            self.picker_hits.clear();
            // 计数行不再显示上一次查询的命中数。
            self.picker.forget_query();
            return;
        }

        self.picker_hits = self.picker.search(query);

        // 没有输入时，最近打开的文件排在最前面；第一条是上一个文件，因此 `⌘P` 加回车即可切回。
        if query.trim().is_empty() {
            let active = self.editor.active().and_then(|d| d.path.as_deref());
            let recent: Vec<usize> = self
                .recent_files
                .iter()
                .filter(|p| Some(p.as_path()) != active)
                .filter_map(|p| self.picker.position_of(p))
                .collect();

            self.picker_hits.retain(|i| !recent.contains(i));
            self.picker_recent = recent.len();
            self.picker_hits.splice(0..0, recent);
            self.picker_hits.truncate(winc_editor::PICKER_MAX_RESULTS);
        }
    }

    /// 是否应显示「正在索引文件…」：首次建立文件表且还没有可列出的文件时为 `true`。
    ///
    /// 重新遍历目录时旧表仍然存在，继续列出旧表中的结果（面板高度也按这些结果计算），
    /// 避免只显示一行文字而下方留出大片空白。
    pub(crate) fn picker_indexing(&self) -> bool {
        !self.picker.is_ready() && self.picker.is_empty()
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
    /// 打开的目录还没有文件表，或窗口失去过焦点（文件可能被增删，`picker_stale`）时，
    /// 在后台建立新表。
    pub(super) fn prepare_index(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };

        if self.picker_stale && self.picker.root() == Some(root.as_path()) {
            self.picker_invalidate();
        }

        if self.picker.root() != Some(root.as_path()) {
            self.picker.cancel();
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
            .picker_hits
            .get(self.picker_selected)
            .and_then(|&i| self.picker.absolute(i));

        if !self.picker.poll() {
            return;
        }

        let files = !self.picker_lists_commands()
            && !self.picker_lists_symbols()
            && !self.picker_lists_branches()
            && !self.picker_lists_locations()
            && !self.picker_folders;
        if self.picker_open && files && self.picker_places.is_none() {
            let scroll = self.picker_scroll;
            self.picker_refresh();

            let at = selected
                .and_then(|path| self.picker.position_of(&path))
                .and_then(|position| self.picker_hits.iter().position(|&i| i == position));
            if let Some(at) = at {
                self.picker_selected = at;
                self.picker_scroll = scroll;
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
        if self.picker_hits.is_empty() {
            return;
        }

        let last = self.picker_hits.len() - 1;
        self.picker_default = None;
        self.picker_selected = if self.picker_lists_branches() {
            self.branch_picker_step(down)
        } else if down {
            (self.picker_selected + 1).min(last)
        } else {
            self.picker_selected.saturating_sub(1)
        };

        self.picker_reveal();
        if self.picker_lists_symbols() {
            self.symbols_preview();
        }
        self.request_redraw();
    }

    /// 滚动结果列表，使选中项进入可见范围。
    pub(in crate::app) fn picker_reveal(&mut self) {
        let height = view::picker_span(self);
        if height <= 0.0 {
            return;
        }

        let top = self.picker_selected as f32 * PICKER_ROW;
        self.picker_scroll = scroll_to_show(self.picker_scroll, top, top + PICKER_ROW, height);
    }

    /// 确认选中项。
    ///
    /// 文件列表中打开选中的文件（已有标签则切换过去，否则新开一个）；命令面板中执行选中的命令；
    /// 符号列表和位置列表中跳转到选中的位置；最近目录列表中打开选中的目录。
    pub(in crate::app) fn picker_accept(&mut self) {
        if self.picker_places.is_some() {
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
        if self.picker_folders {
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
        let goto = self.picker_goto.take();
        let path = self
            .picker_hits
            .get(self.picker_selected)
            .and_then(|&index| self.picker.absolute(index));

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
        self.picker.apply(winc_editor::Changes {
            paths,
            rescan: false,
            git: false,
        });
    }

    /// 让文件表失效并重新遍历整个目录。
    ///
    /// 用户点击刷新目录，或没有监视目录时需要使用 `⌘P`，都会调用。
    pub(super) fn picker_invalidate(&mut self) {
        self.picker_stale = false;
        let Some(root) = self.picker.root().map(Path::to_path_buf) else {
            return;
        };

        self.build_index(&root);

        if self.picker_open {
            self.picker_refresh();
        }
    }

    /// 在后台为 `root` 建立文件表，建好后发送 [`UserEvent::Indexed`]。
    ///
    /// 已有旧表时保留旧表，新表建好之前面板仍可使用，不会变空。测试中没有事件循环，同步建立。
    fn build_index(&mut self, root: &Path) {
        let Some(proxy) = self.proxy.clone() else {
            self.picker = FileIndex::collect_now(root);
            return;
        };
        if self.picker.root() == Some(root) {
            self.picker.refresh();
        } else {
            self.picker = FileIndex::spawn(root, move || {
                let _ = proxy.send_event(UserEvent::Indexed);
            });
        }
    }
}

#[cfg(test)]
mod tests;
