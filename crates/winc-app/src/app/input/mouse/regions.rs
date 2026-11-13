//! 各区域的点击：全局搜索面板、设置浮层、快速打开、输入框、文件树、编辑区。

use super::*;

impl App {
    /// 返回侧栏顶部切换按钮和搜索面板中的悬停目标；不在这些位置时返回 `None`。
    pub(super) fn panel_hover_at(&self, x: f32, y: f32) -> Option<Hover> {
        let sidebar = self.layout.sidebar;

        if sidebar.right() <= 0.0 || !(sidebar.contains(x, y) || self.in_commit_overflow(x, y)) {
            return None;
        }

        if let Some(view) = hit(&self.sidebar_tabs(), x, y) {
            return Some(Hover::ViewTab(view));
        }
        // 第二行的项目和分支，三个视图都一样。
        if self.hits.project.contains(x, y) {
            return Some(Hover::ProjectName);
        }

        // 历史提交的文件列表借用提交视图的列表，按绘制时记下的位置找。
        if self.log_diff_tree().is_some() && y >= self.panel_rect().y {
            if self.log_diff_layout_button().contains(x, y) {
                return Some(Hover::LogDiffLayout);
            }
            if self.on_splitter_outside_commit_overflow(x, y) {
                return None;
            }
            return Some(hit(&self.hits.commit, x, y).map_or(Hover::None, Hover::Commit));
        }

        // 文件视图底部的大纲：按绘制时记下的位置找；分隔条由 `Target::Splitter` 处理。
        if self.layout.outline.contains(x, y) {
            if self.panels.sidebar.visible && self.layout.splitter.contains(x, y) {
                return None;
            }
            return Some(hit(&self.hits.outline, x, y).map_or(Hover::None, Hover::Outline));
        }

        // 提交视图：输入框之外按绘制时记下的位置找。
        if self.commit_shown() && y >= self.panel_rect().y {
            // 分隔条由 `Target::Splitter` 处理，写全名的那一行除外：该行覆盖在分隔条之上。
            if self.on_splitter_outside_commit_overflow(x, y) {
                return None;
            }
            return Some(hit(&self.hits.commit, x, y).map_or(Hover::None, Hover::Commit));
        }

        // 面板只负责顶栏以下、分隔条以左的区域；侧栏开关和分隔条由后面的判断处理。
        let panel = self.panel_rect();
        let on_splitter = self.panels.sidebar.visible && self.layout.splitter.contains(x, y);

        if !self.panel_shown() || !panel.contains(x, y) || on_splitter {
            return None;
        }

        let parts = self.panel_parts();
        let g = &self.panels.global;

        if let Some(button) = parts.button_at(x, y, g.replace_open) {
            return Some(Hover::Panel(button));
        }

        if parts.results.contains(x, y) {
            let index = self.result_row_at(y)?;
            let row = self.result_row_rect(index);
            let (replace, dismiss) = row_actions(row, g.replace_open);

            return Some(if dismiss.contains(x, y) {
                Hover::RowAction(index, RowAction::Dismiss)
            } else if replace.is_some_and(|r| r.contains(x, y)) {
                Hover::RowAction(index, RowAction::Replace)
            } else {
                Hover::Result(index)
            });
        }

        Some(Hover::None)
    }

    /// 返回在设置页滚动条上 (`x`, `y`) 处按下时抓住滑块的位置（相对滑块上沿）；点在轨道上滑块
    /// 以外时为滑块长度的一半，使滑块跳到鼠标处。不在滚动条上或内容放得下时返回 `None`。
    ///
    /// 可点击的范围与唤出滚动条的范围相同：滚动区域右边 [`SCROLLBAR_HIT`] 以内。
    pub(super) fn settings_scrollbar_at(&self, x: f32, y: f32) -> Option<f32> {
        let (area, max) = self.settings_scroll_area();
        let thumb = settings_thumb(area, self.settings.scroll, max)?;
        if !area.contains(x, y) || x < area.right() - SCROLLBAR_HIT {
            return None;
        }
        Some(if (thumb.y..thumb.bottom()).contains(&y) {
            y - thumb.y
        } else {
            thumb.h / 2.0
        })
    }

    /// 拖动设置页滚动条的滑块到纵坐标 `y`；`grab` 为按下点相对滑块上沿的距离。
    pub(super) fn drag_settings_scrollbar(&mut self, y: f32, grab: f32) {
        let (area, max) = self.settings_scroll_area();
        let Some(thumb) = settings_thumb(area, 0.0, max) else {
            return;
        };
        let track = settings_track(area);
        self.settings.scroll = scroll_for_thumb(track.y, track.h, thumb.h, y, grab, max);
        self.settings.scrolled_at = Some(Instant::now());
        self.request_redraw();
    }

    /// 处理设置浮层打开时的点击。
    ///
    /// 点在卡片外面（遮罩上）时关闭设置，不会同时点到下方的内容。正在编辑时，点到输入框
    /// 以外的地方先离开输入框（见 [`App::settings_leave_field`]），再处理这次点击。
    pub(super) fn click_settings(&mut self, x: f32, y: f32) {
        let layout = self.settings_layout();
        let part = self.settings_part_at(x, y);

        if !layout.card.contains(x, y) {
            self.close_settings();
            return;
        }

        let editing = self.settings.editing.as_ref().map(|(s, _)| *s);
        if editing.is_some() && part != editing.map(SettingsPart::Field) {
            self.settings_leave_field();
        }

        // 鼠标点击之后不再显示键盘选中的底色。
        self.settings.keyboard = false;

        // 右边缘的滚动条：没有点到控件时按下即抓住滑块。
        if part.is_none()
            && let Some(grab) = self.settings_scrollbar_at(x, y)
        {
            self.grab_scrollbar(Scrollbar::Settings, grab);
            return;
        }

        match part {
            Some(SettingsPart::Close) => self.close_settings(),
            Some(SettingsPart::Page(page)) => self.settings_show_page(page),
            Some(SettingsPart::Field(setting)) => {
                self.settings.selected = setting;
                if editing == Some(setting) {
                    // 已经在编辑这一项时：点击定位光标，拖动选择文字。
                    self.focus = Focus::Setting;
                    self.click_field(x, y);
                } else if setting.control() == SettingControl::Command {
                    self.settings_open_menu(setting);
                } else {
                    self.settings_start_editing(setting);
                }
            }
            Some(SettingsPart::FontMenu(setting)) => self.settings_open_menu(setting),
            Some(SettingsPart::Decrease(setting)) => {
                self.settings.selected = setting;
                self.settings_adjust(setting, false);
            }
            Some(SettingsPart::Increase(setting)) => {
                self.settings.selected = setting;
                self.settings_adjust(setting, true);
            }
            Some(SettingsPart::Theme(choice)) => {
                self.settings.selected = crate::layout::Setting::Theme;
                self.set_theme_choice(choice);
            }
            Some(SettingsPart::NamedTheme(dark, index)) => {
                self.settings.selected = if dark {
                    crate::layout::Setting::DarkTheme
                } else {
                    crate::layout::Setting::LightTheme
                };
                self.set_named_theme(dark, index);
            }
            Some(SettingsPart::Switch(setting)) => {
                self.settings.selected = setting;
                self.settings_toggle(setting);
            }
            Some(SettingsPart::Action(setting)) => self.settings_open_menu(setting),
            Some(SettingsPart::Record) => self.toggle_recording(),
            Some(SettingsPart::KeySearch) => {
                self.set_recording(false);
                self.focus = Focus::KeySearch;
                self.click_field(x, y);
            }
            Some(SettingsPart::SearchField) => {
                self.settings_leave_field();
                self.set_recording(false);
                self.focus = Focus::SettingsSearch;
                self.click_field(x, y);
            }
            Some(SettingsPart::Copy(setting)) => self.settings_copy_value(setting),
            Some(SettingsPart::Link(setting)) => self.settings_open_link(setting),
            Some(SettingsPart::Install(id)) => {
                self.settings_choose_tool(id, MenuCommand::ToolInstall);
            }
            None => {
                // 点在卡片内的空白处：落在某一行上时选中该行，焦点回到浮层。
                if let Some(row) = layout.rows.iter().find(|r| r.row.contains(x, y)) {
                    self.settings.selected = row.setting;
                }
                if self.focus == Focus::Setting {
                    self.focus = Focus::Settings;
                }
            }
        }

        self.request_redraw();
    }

    /// 是否已处理对快速打开面板的点击。
    ///
    /// 点在面板内交给面板处理；点在面板外则关闭面板，不会同时点到下方的内容。
    /// 面板没有打开时返回 `false`。
    pub(super) fn click_picker(&mut self, x: f32, y: f32) -> bool {
        if !self.overlays.picker.open {
            return false;
        }

        let rect = self.layout.picker;

        if !rect.contains(x, y) {
            self.close_picker();
            return true;
        }

        if let Some(&(_, part)) = self
            .hits
            .picker_parts
            .iter()
            .find(|(r, _)| r.contains(x, y))
        {
            self.focus = Focus::Picker;
            self.press_picker_part(part);
            return true;
        }

        let parts = crate::app::workspace::picker::picker_parts(self, rect);

        if parts.bar.contains(x, y) {
            self.focus = Focus::Picker;
            // 位置浮窗的标题行没有输入框：筛选文字直接打字输入。
            if self.overlays.picker.places.is_none() {
                self.click_field(x, y);
            }
            return true;
        }

        self.focus = Focus::Picker;

        if let Some(index) = parts.row_at(y, self.overlays.picker.scroll)
            && index < self.overlays.picker.hits.len()
        {
            // 分组标题点了没有反应。
            if self.picker_row_selectable(index) {
                self.overlays.picker.selected = index;
                self.picker_accept();
            }
        }

        true
    }

    /// (`x`, `y`) 是否在分隔条上、且不在提交视图里写全名的那一行上：该行覆盖在分隔条之上，与文件
    /// 树的处理相同（见 [`App::target_at`]）。
    fn on_splitter_outside_commit_overflow(&self, x: f32, y: f32) -> bool {
        self.panels.sidebar.visible
            && self.layout.splitter.contains(x, y)
            && !self.over_commit_overflow_row(y)
    }

    /// 是否已处理对侧栏顶部切换按钮或搜索面板的点击。
    pub(super) fn click_panel(&mut self, x: f32, y: f32) -> bool {
        let sidebar = self.layout.sidebar;

        if sidebar.right() <= 0.0 || !(sidebar.contains(x, y) || self.in_commit_overflow(x, y)) {
            return false;
        }

        // 点击切换按钮：已在该页时也把焦点交给它（文件树或搜索输入框）。
        if let Some(view) = hit(&self.sidebar_tabs(), x, y) {
            match view {
                SidebarView::Files => self.show_files(),
                SidebarView::Search => self.open_global_search(false),
                SidebarView::Commit => self.show_commit(),
            }

            return true;
        }
        if self.hits.project.contains(x, y) {
            self.open_project_menu();
            return true;
        }

        if self.log_diff_tree().is_some() && y >= self.panel_rect().y {
            if self.log_diff_layout_button().contains(x, y) {
                self.toggle_log_diff_layout();
                return true;
            }
            if self.on_splitter_outside_commit_overflow(x, y) {
                return false;
            }
            if let Some(target) = hit(&self.hits.commit, x, y) {
                self.with_log_list(|app| app.press_commit(target));
            }
            self.request_redraw();
            return true;
        }

        if self.layout.outline.contains(x, y) {
            if self.panels.sidebar.visible && self.layout.splitter.contains(x, y) {
                return false;
            }
            if let Some(target) = hit(&self.hits.outline, x, y) {
                self.press_outline(target);
            } else if self.outline_list().contains(x, y) {
                self.focus = Focus::Outline;
            }
            self.request_redraw();
            return true;
        }

        if self.commit_shown() && y >= self.panel_rect().y {
            if self.on_splitter_outside_commit_overflow(x, y) {
                return false;
            }
            if let Some(focus) = self.field_at(x, y) {
                self.focus = focus;
                self.click_field(x, y);
            } else if let Some(target) = hit(&self.hits.commit, x, y) {
                self.press_commit(target);
            }
            self.request_redraw();
            return true;
        }

        if !self.panel_shown() || (y < self.panel_rect().y) {
            return false;
        }

        // 分隔条在侧栏右边缘，由后面的分支处理。
        if self.panels.sidebar.visible && self.layout.splitter.contains(x, y) {
            return false;
        }

        match self.panel_hover_at(x, y) {
            Some(Hover::Panel(button)) => self.press_panel_button(button),
            Some(Hover::RowAction(index, action)) => self.press_row_action(index, action),
            Some(Hover::Result(index)) => {
                let double = self.click_count(x, y) == 2;
                self.click_result(index, double);
            }
            _ => {
                if let Some(focus) = self.field_at(x, y) {
                    self.focus = focus;
                    self.click_field(x, y);
                }
            }
        }

        self.request_redraw();
        true
    }

    /// 返回鼠标所在输入框对应的焦点；只考虑正在显示的输入框。
    pub(in crate::app) fn field_at(&self, x: f32, y: f32) -> Option<Focus> {
        let visible = [
            (Focus::Search, self.bars.find.open, &self.bars.find.input),
            (
                Focus::Replace,
                self.bars.find.open && self.bars.find.replace_open,
                &self.bars.find.replace_input,
            ),
        ];
        let prompt = self
            .bars
            .goto_input
            .as_ref()
            .map(|f| (Focus::Prompt, true, f));
        let rename = (self.terminal.rename.as_ref()).map(|(_, f)| (Focus::TerminalRename, true, f));
        let terminal_find =
            (self.terminal.find.as_ref()).map(|f| (Focus::TerminalFind, true, &f.field));
        let shown = self.panel_shown();
        let g = &self.panels.global;
        let panel = [
            (Focus::GlobalQuery, shown, &g.query),
            (Focus::GlobalReplace, shown && g.replace_open, &g.replace),
            (Focus::GlobalInclude, shown && g.details_open, &g.include),
            (Focus::GlobalExclude, shown && g.details_open, &g.exclude),
            (
                Focus::CommitMessage,
                self.commit_shown() && self.commit_compose_shown(),
                &self.git.commit.message,
            ),
        ];

        visible
            .into_iter()
            .chain(prompt)
            .chain(rename)
            .chain(terminal_find)
            .chain(panel)
            .find(|(_, shown, f)| *shown && f.view.area.contains(x, y))
            .map(|(focus, ..)| focus)
    }

    /// 处理对当前输入框的点击：单击放置光标（按住 `Shift` 时扩展选区），双击选词，三击全选。
    pub(in crate::app) fn click_field(&mut self, x: f32, y: f32) {
        let count = self.click_count(x, y);
        let shift = self.pointer.modifiers.shift_key();

        let Some(field) = self.focused_field() else {
            return;
        };
        let pos = field.view.position_at_point(x, y);

        match count {
            2 => field.select_word_at(pos),
            3 => field.select_all(),
            _ => {
                field.set_caret(pos, shift);
                self.pointer.drag = Drag::Field;
            }
        }

        self.reset_blink();
    }

    /// 处理在文件树一行上的按下：先选中该行。
    ///
    /// 松开时没有拖动才算单击，拖动则表示移动选中的各项（见 `TreeDrag`）。
    ///
    /// `⌘` 单击把这一行加入或移出选区，`⇧` 单击选中从上次单击的行到这一行；这两种只改选区，
    /// 不打开文件、不拖动。按在已经多选的一行上时先保留选区，好拖动全部；松开时没有拖动才只选
    /// 这一行。
    pub(super) fn click_tree(&mut self, x: f32, y: f32) {
        self.focus = Focus::Tree;
        let (command, shift) = (self.command_held(), self.pointer.modifiers.shift_key());

        // 点在最后一行下面的空白处：不选中任何一项，此时新建的内容放在根目录；
        // 双击则在根目录新建文件。
        if self.tree_blank_at(y) {
            let count = self.click_count(x, y);
            if let Some(ws) = &mut self.editor.workspace {
                if command || shift {
                    return;
                }
                ws.select(None);
                if count == 2 {
                    self.start_new_entry(false);
                }
            }
            return;
        }

        let Some(index) = self.tree_row_at(y) else {
            return;
        };
        let count = self.click_count(x, y);
        let sticky = self.tree_sticky_at(y).map(|s| s.level);
        let segment = self.tree_segment_at(index, x);
        let Some(ws) = &mut self.editor.workspace else {
            return;
        };
        if command {
            ws.toggle_selected(index);
            return;
        }
        if shift {
            ws.select_range(index);
            return;
        }
        if !(ws.multi_selected() && ws.is_selected(index)) {
            ws.select_segment(index, segment.as_deref());
        }
        let mut drag = TreeDrag::new(ws, index, (x, y));
        drag.sticky = sticky;
        drag.keep = count == 2;
        self.panels.tree.drag = Some(drag);
    }

    /// 处理对编辑区窗口坐标 (`x`, `y`)（逻辑像素）处的点击：放置光标，双击选词，三击或点击行号栏
    /// 选中整行。
    ///
    /// `⌘` 单击转到定义，其他平台上为 `Ctrl` 单击。
    pub(super) fn click_editor(&mut self, x: f32, y: f32) {
        let l = self.layout;

        if self.click_diff(x, y) {
            return;
        }
        self.focus = Focus::Editor;
        self.ime.preedit.clear();

        // 差异、图片、表格这类不显示文本的标签：点击只拿焦点，表格底部的工作表标签例外，
        // 点它会换工作表。
        // 文档中的文本看不见，在它上面放光标、转到定义都没有意义，放了光标还会把视图滚到看不见
        // 的光标那里。
        if !self.active_shows_text() {
            self.click_table(x, y);
            return;
        }

        let Some(pos) = self.pos_at(x.max(l.text.x), y) else {
            return;
        };

        let count = self.click_count(x, y);

        // `⌘` 单击转到定义，其他平台上为 `Ctrl` 单击。
        let jump = if cfg!(target_os = "macos") {
            self.pointer.modifiers.super_key()
        } else {
            self.pointer.modifiers.control_key()
        };
        if jump && count == 1 && !l.gutter.contains(x, y) {
            self.select(pos, pos);
            self.go_to_definition();
            return;
        }

        // `⇧⌥` 按下拖动框出一列；`⌥` 单击添加或移除一个光标，按住拖动则添加一个选区。
        let (shift, alt) = (
            self.pointer.modifiers.shift_key(),
            self.pointer.modifiers.alt_key(),
        );
        if alt && !l.gutter.contains(x, y) {
            if shift {
                self.start_column_drag(x, y);
            } else if count == 1 {
                let before = self.editor.active().map(|d| d.selections().0.len());
                self.toggle_cursor(pos);
                // 点在已有的光标上是移除，不继续拖动。
                let added = self.editor.active().map(|d| d.selections().0.len()) > before;
                if added {
                    self.pointer.drag = Drag::AddSelect { origin: pos };
                }
            }
            return;
        }

        // 点在行号栏上选中整行。
        let count = if l.gutter.contains(x, y) { 3 } else { count };
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        doc.goal_cell = None;
        // 普通的点击回到单光标（`⇧` 单击扩展的也只是主选区）；点之前的选区可以用 `⌘U` 退回。
        doc.record_cursors();
        doc.collapse_selections();

        match count {
            2 => {
                let (a, b) = movement::word_at(doc.text(), pos);
                doc.selection = Selection::new(a, b);
                self.pointer.drag = Drag::Select {
                    by_word: true,
                    origin: (a, b),
                };
            }
            3 => {
                let (line, _) = doc.text().pos_to_line_col(pos);
                let span = doc.text().line_span(line);
                doc.selection = Selection::new(span.start, span.end);
                self.pointer.drag = Drag::SelectLine { line };
            }
            _ => {
                let anchor = if shift { doc.selection.anchor } else { pos };
                doc.selection = Selection::new(anchor, pos);
                self.pointer.drag = Drag::Select {
                    by_word: false,
                    origin: (anchor, anchor),
                };
            }
        }
    }

    /// 在窗口坐标 (`x`, `y`)（逻辑像素）处开始列选择：先在这里放一个光标，拖动时框出一列。
    pub(super) fn start_column_drag(&mut self, x: f32, y: f32) {
        self.focus = Focus::Editor;
        self.ime.preedit.clear();
        let x = x.max(self.layout.text.x);
        if let Some(anchor) = self.line_cell_at(x, y) {
            if let Some(doc) = self.editor.active_mut() {
                doc.record_cursors();
            }
            self.pointer.drag = Drag::Columns { anchor };
            self.drag_columns(x, y, anchor);
        }
    }
}
