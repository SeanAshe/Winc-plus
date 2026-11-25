//! 弹出菜单：右键点中的是什么、各项的文字和勾选、交给系统菜单显示。

use winc_editor::git::Ref;
use winc_i18n::t;

use super::*;

/// 菜单中一项需要的数据，由 `Prepared` 持有，弹出菜单时借给系统菜单。
#[cfg(target_os = "macos")]
struct Prepared {
    label: MenuLabel,
    /// 快捷键的按键字符和修饰键位。
    key: (String, usize),
    icon: Option<Image>,
    /// 图标的逻辑尺寸（宽，高）。
    icon_size: (f32, f32),
    enabled: bool,
    checked: bool,
    /// 按住 `⌥` 时代替前一项（[`MenuEntry::Alternate`]）。
    alternate: bool,
    tag: isize,
    children: Vec<Option<Prepared>>,
}

#[cfg(target_os = "macos")]
impl Prepared {
    fn item(&self) -> crate::macos::MenuItem<'_> {
        crate::macos::MenuItem {
            title: &self.label.title,
            subtitle: self.label.subtitle.as_deref(),
            tooltip: self.label.tooltip.as_deref(),
            icon: self.icon.as_ref().map(|image| crate::macos::MenuIcon {
                image,
                size: self.icon_size,
            }),
            key: &self.key.0,
            modifiers: self.key.1,
            enabled: self.enabled,
            checked: self.checked,
            alternate: self.alternate,
            tag: self.tag,
            submenu: self
                .children
                .iter()
                .map(|c| c.as_ref().map(Prepared::item))
                .collect(),
        }
    }
}

/// 标签列表菜单的图标边长（逻辑像素）。
///
/// 文件图标为 12 × 15，无法放入其他菜单 15 × 12 的图标位，因此改用 16 × 16 的图标位，
/// 状态栏路径的目录菜单也使用该尺寸。
const TAB_LIST_ICON: f32 = 16.0;

/// 菜单项左边的图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuIcon {
    /// 最近打开的目录：与文件树中的目录相同的图标和颜色。
    Folder,
    /// 最近打开的一组目录：两个叠放的目录。
    Folders,
    /// 标签列表的第几项：标签上的图标（文件类型图标，日志标签为 Git 图标）。
    Tab(usize),
    /// 目录菜单中的文件、语言菜单中的语言：文件类型的记号和它在深色、浅色配色下的颜色（同
    /// 文件树）。
    File(icons::FileGlyph, [u32; 2]),
}

impl MenuIcon {
    /// 图标位的逻辑尺寸（宽，高）。
    pub(crate) fn size(self) -> (f32, f32) {
        match self {
            MenuIcon::Tab(_) | MenuIcon::File(..) => (TAB_LIST_ICON, TAB_LIST_ICON),
            MenuIcon::Folder | MenuIcon::Folders => MENU_ICON,
        }
    }
}

/// 子菜单 `items` 这一行用谁的图标：第一项打开整个目录组时用它的，其余子菜单没有图标。
pub(super) fn submenu_lead(items: &[MenuEntry]) -> Option<MenuCommand> {
    match items.first() {
        Some(&MenuEntry::Item(command @ MenuCommand::OpenGroup(_), _)) => Some(command),
        _ => None,
    }
}

impl App {
    /// 处理右键点击（macOS 上也包括 `⌃` 单击）：点在标签上弹出标签菜单，点在文件树里弹出
    /// 文件树菜单，点在编辑区里弹出编辑区菜单。
    ///
    /// 文件树中点中的那一行先被选中，点在空白处表示根目录。其他位置没有菜单。设置浮层或
    /// 快速打开面板打开时不处理，点在卡片、补全列表等遮挡下方的部件上时也不处理。
    pub(in crate::app) fn context_click(&mut self, x: f32, y: f32) {
        if self.settings.open
            && let Some(SettingsPart::KeyRow(action)) = self.settings_part_at(x, y)
        {
            self.cancel_key_edit();
            self.open_context_menu(MenuTarget::Shortcut(action), (x, y));
            return;
        }
        if self.settings.open
            || self.overlays.form_dialog.is_some()
            || self.overlays.picker.open
            || self.blocks_context_menu(x, y)
        {
            return;
        }

        // 正在输入名字时点击了右键：先提交名字，与点击其他位置时相同。
        if self.panels.name_input.is_some()
            && !self.name_input_rect().is_some_and(|r| r.contains(x, y))
        {
            self.submit_name_input();
        }

        // 应用绘制标题栏时，顶栏空白处的右键弹出系统的窗口菜单；窗口系统不支持时没有菜单。
        if self.window.controls && self.over_title_bar(x, y) {
            if let Some(window) = &self.window.handle {
                let zoom = f64::from(self.zoom());
                let at = winit::dpi::LogicalPosition::new(f64::from(x) * zoom, f64::from(y) * zoom);
                window.show_window_menu(at);
            }
            return;
        }

        let tab = self
            .tab_at(x, y)
            .and_then(|(index, _)| self.editor.tabs().get(index));
        let target = if let Some(doc) = tab {
            MenuTarget::Tab(doc.id)
        } else if self.panels.sidebar.visible
            && self.panels.sidebar.view == SidebarView::Files
            && (self.layout.tree.contains(x, y) || self.in_tree_overflow(x, y))
            && self.editor.workspace.is_some()
        {
            let row = self.tree_row_at(y);
            let segment = row.and_then(|i| self.tree_segment_at(i, x));
            let Some(ws) = &mut self.editor.workspace else {
                return;
            };
            // 点在选区里的一行上时菜单作用于选中的各项，否则只选这一行。
            if !row.is_some_and(|i| ws.is_selected(i)) {
                ws.select(row);
            }
            // 点在合并显示的一行上时，菜单作用于点中的那一级目录。
            if let Some(i) = row.filter(|_| !ws.multi_selected()) {
                ws.select_segment(i, segment.as_deref());
            }
            self.focus = Focus::Tree;
            MenuTarget::Tree(row.map(|i| segment.unwrap_or_else(|| ws.rows()[i].path.clone())))
        } else if let Some((path, m)) = self.search_context_target(x, y) {
            MenuTarget::SearchResult(path, m)
        } else if let Some(i) = self.stash_context_target(x, y) {
            MenuTarget::StashRow(i)
        } else if let Some(path) = self.commit_context_target(x, y) {
            MenuTarget::CommitFile(path)
        } else if self.in_commit_overflow(x, y) {
            // 改动行伸出侧栏的一段遮挡下方；其中的按钮没有菜单，右键点击不传给下方。
            return;
        } else if let Some((i, path)) = self.log_file_context_target(x, y) {
            MenuTarget::LogFile(i, path)
        } else if let Some(index) = self.log_branch_context_target(x, y) {
            MenuTarget::BranchOps(index)
        } else if let Some(oid) = self.log_context_target(x, y) {
            MenuTarget::LogCommit(oid)
        } else if let Some(id) = self.diff_gutter_at(x, y) {
            MenuTarget::DiffGutter(id)
        } else if let Some((id, right)) = self.diff_context_target(x, y) {
            MenuTarget::Diff(id, right)
        } else if let Some(id) = self.terminal_tab_context_target(x, y) {
            MenuTarget::TerminalTab(id)
        } else if let Some(block) = self.terminal_context_target(x, y) {
            MenuTarget::Terminal(block)
        } else if self.markdown_context_target(x, y) {
            MenuTarget::Markdown
        } else if self.combined_active() && self.layout.editor.contains(x, y) {
            // 「未提交的更改」中段头、工具栏上没有菜单；差异上的菜单由那一段弹出。
            return;
        } else if let Some(id) = self.editor_context_target(x, y) {
            MenuTarget::Editor(id)
        } else {
            return;
        };

        self.open_context_menu(target, (x, y));
    }

    /// 右键点在搜索面板的一个结果行上（含写全名时伸出侧栏的一段）：选中这一行，焦点给结果
    /// 列表，返回它（文件路径，第几处匹配）。
    fn search_context_target(&mut self, x: f32, y: f32) -> Option<(PathBuf, Option<usize>)> {
        if !self.panel_shown() {
            return None;
        }
        let in_list = self.panel_parts().results.contains(x, y) && self.panel_rect().contains(x, y);
        if !in_list && !self.in_result_overflow(x, y) {
            return None;
        }
        let index = self.result_row_at(y)?;
        self.panels.global.select_row(index);
        self.focus = Focus::Results;
        self.panels.global.selected.clone()
    }

    /// 右键点在提交视图里的一条储藏上：返回它是第几条储藏。
    fn stash_context_target(&self, x: f32, y: f32) -> Option<usize> {
        if !self.commit_shown() || self.git.commit.hits_generation != self.git.generation {
            return None;
        }
        match hit(&self.hits.commit, x, y)? {
            CommitTarget::Stash(i) | CommitTarget::StashApply(i) | CommitTarget::StashMenu(i) => {
                Some(i)
            }
            _ => None,
        }
    }

    /// 右键点在提交视图改动列表里的一个文件上：选中它（不打开预览），返回它的路径。
    ///
    /// 点击目录或组标题时选中其下的各个文件，返回第一个文件的路径。
    fn commit_context_target(&mut self, x: f32, y: f32) -> Option<PathBuf> {
        if !self.panels.sidebar.visible
            || self.panels.sidebar.view != SidebarView::Commit
            || self.git.commit.hits_generation != self.git.generation
        {
            return None;
        }
        let path = match hit(&self.hits.commit, x, y)? {
            CommitTarget::File(i) | CommitTarget::Check(i) => {
                self.git.status.as_ref()?.entries.get(i)?.path.clone()
            }
            // 目录：菜单作用于它下面的各个文件（都已在选区里时作用于整个选区）。
            CommitTarget::Dir(row) | CommitTarget::DirCheck(row) => {
                let files = self.commit_dir_files(row);
                let first = files.first()?.clone();
                if !files.iter().all(|f| self.commit_is_selected(f)) {
                    self.select_commit_files(files);
                }
                self.focus = Focus::CommitList;
                return Some(first);
            }
            // 组标题的处理方式同目录：菜单作用于组内的各个文件，组收起时同样适用。
            CommitTarget::Group(group) | CommitTarget::GroupCheck(group) => {
                let files: Vec<PathBuf> = self
                    .git
                    .status
                    .as_ref()?
                    .entries
                    .iter()
                    .filter(|e| group.holds(e, self.git.commit.staging()))
                    .map(|e| e.path.clone())
                    .collect();
                let first = files.first()?.clone();
                if !files.iter().all(|f| self.commit_is_selected(f)) {
                    self.select_commit_files(files);
                }
                self.focus = Focus::CommitList;
                return Some(first);
            }
            _ => return None,
        };
        // 点在选区里的文件上时菜单作用于选中的各个文件，否则只选这一个。
        if !self.commit_is_selected(&path) {
            self.git.commit.clear_selection();
            self.git.commit.selected = Some(path.clone());
        }
        self.focus = Focus::CommitList;
        Some(path)
    }

    /// 返回右键点击处所在的编辑区文档；该位置不应弹出编辑区菜单时返回 `None`。
    ///
    /// 点在选区外时光标先移到点击处；点在选区内时保留选区，菜单作用于该选区。不显示文本的
    /// 文档没有菜单。
    fn editor_context_target(&mut self, x: f32, y: f32) -> Option<DocumentId> {
        let l = self.layout;
        if !l.editor.contains(x, y) {
            return None;
        }

        if self.hex_active() {
            if let Some(at) = self.hex_byte_at(x, y) {
                let inside = (self.editor.active().and_then(|d| d.hex.as_deref()))
                    .is_some_and(|v| v.selection().contains(&at));
                if !inside {
                    self.hex_press(x, y);
                    self.pointer.drag = Drag::None;
                }
            }
            return self.editor.active().map(|d| d.id);
        }
        let pos = self.pos_at(x.max(l.text.x), y)?;
        let doc = self.editor.active_mut()?;
        if !doc.shows_text() {
            return None;
        }
        if !(doc.selection.from()..=doc.selection.to()).contains(&pos) {
            doc.selection = Selection::point(pos);
            doc.goal_cell = None;
        }
        let id = doc.id;

        self.focus = Focus::Editor;
        self.ime.preedit.clear();
        Some(id)
    }

    /// 右键点在终端的标签上：它成为正在看的标签，返回它的编号。
    fn terminal_tab_context_target(&mut self, x: f32, y: f32) -> Option<u16> {
        use crate::app::terminal::HeaderHit;
        let index = match crate::layout::hit(&self.hits.terminal, x, y)? {
            HeaderHit::Tab(i) | HeaderHit::Close(i) => i,
            // 拆分出的终端：先换到前台，菜单作用于它。
            HeaderHit::Pane(i, at) | HeaderHit::ClosePane(i, at) => {
                self.select_terminal_pane(i, at);
                i
            }
            _ => return None,
        };
        self.activate_terminal_tab(index);
        self.focus = Focus::Terminal;
        self.terminal.tabs.get(index).map(|t| t.id)
    }

    /// 右键点在终端面板的网格上（标签栏以下）：焦点给终端，弹终端的菜单；返回点着的命令块。
    ///
    /// 程序要了鼠标时右键已经上报给程序了，到不了这里（按着 `⇧` 时才到）。
    fn terminal_context_target(&mut self, x: f32, y: f32) -> Option<Option<u32>> {
        let card = self.layout.terminal;
        let hit = card.contains(x, y)
            && y >= card.y + crate::app::terminal::header_height(card)
            && self.terminal.tab().is_some();
        if !hit {
            return None;
        }
        self.focus = Focus::Terminal;
        Some(self.terminal_block_under(x, y))
    }

    /// 为 `target` 生成菜单项，并在窗口坐标 `at`（逻辑像素）处弹出系统菜单。
    ///
    /// 目标文档已不存在时不弹出。
    pub(in crate::app) fn open_context_menu(&mut self, target: MenuTarget, at: (f32, f32)) {
        // 提交菜单里的操作都在日志所在的仓库里做：先换到这个仓库，菜单按它的分支列出。
        if matches!(target, MenuTarget::LogCommit(_)) {
            self.follow_log_root();
        }
        let entries = match &target {
            &MenuTarget::AddLanguage(task) => {
                let listed = match task {
                    Task::Format => &self.settings.languages.format,
                    Task::Fix => &self.settings.languages.fix,
                };
                language_menu(task, listed)
            }
            MenuTarget::SkipOnSave => skip_menu(&self.settings.languages.format),
            MenuTarget::Locale => locale_menu(),
            MenuTarget::NamedTheme(dark) => (0..crate::theme::themes(*dark).len())
                .map(|i| MenuEntry::Item(MenuCommand::NamedTheme(*dark, i), true))
                .collect(),
            MenuTarget::Whitespace => whitespace_menu(),
            MenuTarget::VimClipboard => vim_clipboard_menu(),
            MenuTarget::OpenWorktree => open_worktree_menu(),
            MenuTarget::PullMode => (0..winc_config::PullMode::ALL.len())
                .map(|i| MenuEntry::Item(MenuCommand::PullMode(i), true))
                .collect(),
            MenuTarget::SearchStyle => (0..winc_config::SearchStyle::ALL.len())
                .map(|i| MenuEntry::Item(MenuCommand::SearchStyle(i), true))
                .collect(),
            MenuTarget::DropFolder => (0..winc_config::FolderDrop::ALL.len())
                .map(|i| MenuEntry::Item(MenuCommand::DropFolder(i), true))
                .collect(),
            MenuTarget::AutoSave => (0..winc_config::AutoSave::ALL.len())
                .map(|i| MenuEntry::Item(MenuCommand::AutoSave(i), true))
                .collect(),
            MenuTarget::SmoothCaret => (0..winc_config::SmoothCaret::ALL.len())
                .map(|i| MenuEntry::Item(MenuCommand::SmoothCaret(i), true))
                .collect(),
            MenuTarget::DefaultShell => default_shell_menu(self.terminal.shells.len()),
            MenuTarget::Zoom => zoom_menu(),
            MenuTarget::BackgroundImage => background_menu(),
            MenuTarget::BackgroundFit => background_fit_menu(),
            MenuTarget::TextWidth => text_width_menu(),
            MenuTarget::DefaultLanguage => document_language_menu(false, &[]),
            MenuTarget::TerminalPosition => terminal_position_menu(),
            MenuTarget::TerminalTabPosition => terminal_tab_position_menu(),
            MenuTarget::OptionAsMeta => option_as_meta_menu(),
            &MenuTarget::Font(setting) => font_menu(setting, self.font_value(setting).as_deref()),
            &MenuTarget::Tool(task, id) => {
                let row = self.settings.tool_row(tool_setting(task, id));
                let install = row.is_some_and(|r| !r.install.is_empty());
                let removable = row.is_some_and(|r| r.removable);
                tool_menu(task, id, install, removable)
            }
            MenuTarget::Project => {
                let root = self.editor.root();
                let current = root.and_then(|r| self.state.recent.iter().position(|p| p == r));
                let mut menu: Vec<MenuEntry> = Vec::new();
                menu.extend(project_menu(
                    self.state.recent.len().min(winc_config::RECENT_IN_MENU),
                    current,
                    root.is_some(),
                    crate::system::wsl_distributions().len(),
                    // 每次打开菜单重新读取：用户可能刚修改了 `ssh` 配置。
                    (self.remote_enabled()).then(crate::app::remote::refresh_hosts),
                    root.is_some_and(winc_backend::is_remote),
                ));
                self.add_folder_entries(&mut menu);
                menu
            }
            MenuTarget::Tree(Some(_))
                if self
                    .editor
                    .workspace
                    .as_ref()
                    .is_some_and(|w| w.multi_selected()) =>
            {
                let targets = self.tree_targets();
                let remote =
                    !targets.is_empty() && targets.iter().all(|p| winc_backend::is_remote(p));
                tree_selection_menu(self.tree_selected_files().len(), remote)
            }
            MenuTarget::Tree(path) => {
                // 目录自己的一行和空白处一样：不能改名、剪切、删除。
                let folder = path.as_deref().filter(|p| {
                    (self.editor.workspace.as_ref()).is_some_and(|ws| ws.is_folder_row(p))
                });
                let kind = path.as_ref().filter(|_| folder.is_none()).and_then(|p| {
                    let ws = self.editor.workspace.as_ref()?;
                    ws.row_of(p).map(|i| ws.rows()[i].kind)
                });
                let can_paste = self.can_paste_into_tree();
                let remote = path.as_deref().is_some_and(winc_backend::is_remote);
                let mut menu = tree_menu(kind, can_paste, remote);
                if kind.is_none() {
                    menu.extend(self.folder_items(folder));
                }
                // 脚本文件：「在终端中打开」之后加「在终端中运行」。
                let opens = |entry: &MenuEntry| {
                    matches!(entry, MenuEntry::Item(MenuCommand::OpenInTerminal, _))
                };
                if let Some(path) = path.as_deref()
                    && kind == Some(EntryKind::File)
                    && self.can_run_script(path)
                    && let Some(at) = menu.iter().position(opens)
                {
                    menu.insert(at + 1, MenuEntry::Item(MenuCommand::RunScript, true));
                }
                // Git 模块开着、文件在仓库里：最后加「显示文件历史」，有更改的文件再加暂存与回滚。
                if let Some(path) = path.as_deref()
                    && kind == Some(EntryKind::File)
                    && self.in_repo(path)
                {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::FileHistory, true),
                    ]);
                    menu.extend(self.git_change_items(path));
                }
                menu
            }
            MenuTarget::Editor(id) => 'editor: {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                if let Some(view) = doc.hex.as_deref() {
                    break 'editor hex_menu(view.size() > 0);
                }
                let bookmarked = doc
                    .bookmarks()
                    .lines()
                    .contains(&doc.text().line_of(doc.selection.head));
                let markdown = doc.preview_kind() == Some(winc_editor::PreviewKind::Markdown);
                let mut menu = editor_menu(
                    doc.readonly_reason().is_none(),
                    !doc.selection.is_empty(),
                    bookmarked,
                    markdown,
                );
                // 光标在链接上时第一项是「打开链接」。
                if self.link_at_caret().is_some() {
                    menu.insert(0, MenuEntry::Item(MenuCommand::OpenLink, true));
                }
                // Git 模块开着、文件在仓库里：最后是所选代码的历史和「逐行追溯」开关。
                if doc.path.as_deref().is_some_and(|p| self.in_repo(p)) {
                    let history = Action::GitSelectionHistory;
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::GitAction(history), true),
                        MenuEntry::Item(MenuCommand::ToggleBlame, true),
                    ]);
                }
                menu
            }
            // 可编辑的预览：选区即文档的选区，剪切、复制、粘贴按编辑区的方式处理。
            MenuTarget::Markdown if self.markdown_live() => {
                let selected = self
                    .editor
                    .active()
                    .is_some_and(|d| !d.selection.is_empty());
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::Cut, selected),
                    MenuEntry::Item(MenuCommand::Copy, selected),
                    MenuEntry::Item(MenuCommand::Paste, true),
                    MenuEntry::Item(MenuCommand::PastePlain, true),
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuCommand::SelectAll, true),
                ];
                // 右键点击图片时提供「在新标签中打开图片」和「图片大小」，两项同在一组。
                let (picture, image) = (
                    self.preview.markdown.menu_picture.is_some(),
                    self.preview.markdown.menu_image.is_some(),
                );
                if picture || image {
                    menu.push(MenuEntry::Separator);
                }
                if picture {
                    menu.push(MenuEntry::Item(MenuCommand::OpenImage, true));
                }
                if image {
                    use crate::app::preview::markdown::resize::PRESETS;
                    let sizes = PRESETS.map(|p| MenuEntry::Item(MenuCommand::ImageWidth(p), true));
                    menu.push(MenuEntry::Submenu(
                        t!("menu-image-size").into(),
                        sizes.into(),
                    ));
                }
                // 光标位于表格中时提供「表格」子菜单，集中列出行、列的编辑操作。
                if let Some(table) = self.live_table_menu() {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Submenu(t!("menu-table").into(), table),
                    ]);
                }
                menu.extend([
                    MenuEntry::Separator,
                    MenuEntry::Submenu(t!("menu-text-width").into(), text_width_menu()),
                    MenuEntry::Item(MenuCommand::MarkdownLineNumbers, true),
                    MenuEntry::Separator,
                    MenuEntry::Item(
                        MenuCommand::RevealSource,
                        self.preview.markdown.menu_source.is_some(),
                    ),
                ]);
                menu
            }
            MenuTarget::Markdown => {
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::Copy, self.markdown_selected_text().is_some()),
                    MenuEntry::Item(MenuCommand::SelectAll, true),
                ];
                if self.preview.markdown.menu_picture.is_some() {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::OpenImage, true),
                    ]);
                }
                // docx、EPUB 的阅读视图由转换得到，阅读标签只有排版后的内容：都没有可以显示的
                // 源代码。
                let has_source = self
                    .editor
                    .active()
                    .is_some_and(|d| d.unsupported.is_none() && d.reading.is_none());
                menu.extend([
                    MenuEntry::Separator,
                    MenuEntry::Submenu(t!("menu-text-width").into(), text_width_menu()),
                    MenuEntry::Item(MenuCommand::MarkdownLineNumbers, true),
                ]);
                if has_source {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(
                            MenuCommand::RevealSource,
                            self.preview.markdown.menu_source.is_some(),
                        ),
                    ]);
                }
                menu
            }
            MenuTarget::TerminalTab(id) => vec![
                MenuEntry::Item(MenuCommand::RenameTerminal, true),
                MenuEntry::Item(MenuCommand::SplitTerminal, self.terminal_splittable(*id)),
                MenuEntry::Item(
                    move_group_command(self.terminal_right()),
                    self.terminal_movable_to_group(),
                ),
                MenuEntry::Separator,
                MenuEntry::Item(MenuCommand::CloseTerminal, true),
                MenuEntry::Item(
                    MenuCommand::CloseOtherTerminals,
                    self.terminal.tabs.len() > 1,
                ),
            ],
            &MenuTarget::Terminal(block) => {
                let Some(tab) = self.terminal.tab() else {
                    return;
                };
                let running = tab.ended.is_none() && !self.terminal.crashed;
                let block = block.and_then(|id| {
                    let b = tab.blocks.iter().find(|b| b.id == id)?;
                    Some((!b.text.is_empty(), self.terminal_can_rerun(id)))
                });
                let group = (self.terminal_right(), self.terminal_movable_to_group());
                terminal_menu(tab.selection_range().is_some(), running, block, group)
            }
            &MenuTarget::Diff(id, right) => {
                if self.editor.active().map(|d| d.id) != Some(id) {
                    return;
                }
                let blame = self.diff_blamable();
                diff_menu(right, self.diff_caret_hunk().is_some(), blame)
            }
            &MenuTarget::DiffGutter(id) => {
                if self.editor.active().map(|d| d.id) != Some(id) || !self.diff_blamable() {
                    return;
                }
                vec![MenuEntry::Item(MenuCommand::ToggleBlame, true)]
            }
            MenuTarget::Tab(id) => {
                let Some(index) = self
                    .editor
                    .index_of(*id)
                    .filter(|&i| i < self.editor.tab_count())
                else {
                    return;
                };
                let docs = self.editor.tabs();
                let path = docs[index].path.as_deref();
                let span = self.tab_span(*id, index);
                let selection = self.tab_selection();
                let selected = if selection.contains(id) {
                    selection.len()
                } else {
                    1
                };
                let mut menu = tab_menu(
                    span,
                    selected,
                    docs.len(),
                    path.is_some(),
                    path.is_some_and(|p| self.relative_path(p).is_some()),
                    docs.iter().any(|d| !d.is_dirty()),
                );
                let doc = &docs[index];
                if doc.can_lock() && doc.shows_text() {
                    let command = MenuCommand::ReadOnly(!doc.is_locked());
                    menu.extend([MenuEntry::Separator, MenuEntry::Item(command, true)]);
                }
                if let Some(path) = path.filter(|p| self.in_repo(p)) {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::FileHistory, true),
                    ]);
                    menu.extend(self.git_change_items(path));
                }
                menu
            }
            MenuTarget::Indent(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                let editable = doc.readonly_reason().is_none() && !doc.is_large();
                indent_menu(indent_size_title(doc.indent()), editable)
            }
            MenuTarget::LineEnding(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                let editable = doc.readonly_reason().is_none() && !doc.is_large();
                line_ending_menu(doc.line_ending, editable)
            }
            MenuTarget::Encoding(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                // 十六进制视图没有文本，按文件的大小判断是不是大文件。
                let large = doc.is_large()
                    || (doc.hex.as_deref()).is_some_and(|v| v.size() > winc_editor::LARGE_FILE);
                let utf8_only = doc.hex.is_some() && large;
                // 「Unicode 转义」只用于 `.properties` 文件。
                let properties = (doc.path.as_deref())
                    .and_then(std::path::Path::extension)
                    .is_some_and(|e| e.eq_ignore_ascii_case("properties"));
                let reopen = |encoding| {
                    doc.path.is_some()
                        && (!large || utf8_only && encoding == winc_editor::Encoding::Utf8)
                };
                let hex = doc.path.is_some() && doc.hex.is_none();
                encoding_menu(properties, reopen, hex, |encoding| {
                    doc.hex.is_none() && doc.encoding.writable() && doc.can_save_with(encoding)
                })
            }
            MenuTarget::Language(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                document_language_menu(doc.path.is_some(), &self.state.recent_languages)
            }
            MenuTarget::ReadOnly(id) => {
                let own = self.editor.document(*id).is_some_and(|doc| doc.is_locked());
                read_only_menu(own, self.editor.all_locked())
            }
            MenuTarget::Keymap => {
                let customized = !self.config.shortcuts.is_empty();
                keymap_menu(self.settings.import_sources.len(), customized)
            }
            &MenuTarget::Shortcut(action) => {
                let has_keys = !self.current_keys(action).is_empty();
                shortcut_menu(has_keys, self.shortcut_customized(action))
            }
            MenuTarget::CommitFile(_) if self.commit_multi_selected() => {
                let any_exists = (self.commit_selection().iter()).any(|path| self.may_exist(path));
                commit_selection_menu(any_exists)
            }
            MenuTarget::CommitFile(path) => {
                let mut menu = commit_file_menu(
                    self.may_exist(path),
                    self.relative_path(path).is_some(),
                    &self.ignore_kinds(path),
                );
                // 暂存区视图中在「回滚」之前加暂存、取消暂存，按该文件现在的状态列出。
                let stage = self.git_change_items(path);
                let stage = stage.into_iter().filter(|entry| {
                    matches!(entry, MenuEntry::Item(MenuCommand::CommitCheck(_), _))
                });
                let at = (menu.iter())
                    .position(|e| matches!(e, MenuEntry::Item(MenuCommand::GitRollbackFile, _)));
                if let Some(at) = at {
                    menu.splice(at..at, stage);
                }
                menu
            }
            MenuTarget::SearchResult(path, m) => {
                let g = &self.panels.global;
                let dir = m.is_none() && !g.engine.results.iter().any(|f| &f.path == path);
                search_result_menu(m.is_none(), dir, g.replace_open)
            }
            MenuTarget::CommitMore => self.commit_more_menu(),
            MenuTarget::StashRow(i) => {
                crate::app::git::branch::stash_ops(*i, self.git.branch.running.is_some())
            }
            MenuTarget::StashList => {
                let busy = self.git.branch.running.is_some();
                crate::app::git::branch::stash_menu(&self.git.stashes, busy)
            }
            MenuTarget::CommitButton => self.commit_button_menu(),
            MenuTarget::LastCommit => self.last_commit_menu(),
            MenuTarget::TreeTools => self.tree_tools_menu(),
            MenuTarget::TabList => (0..self.editor.tab_count())
                .map(|i| MenuEntry::Item(MenuCommand::ShowTab(i), true))
                .collect(),
            MenuTarget::LatexProblems => {
                let rc = [
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuCommand::LatexRunRc, true),
                ];
                (0..self.latex_problem_count())
                    .map(|i| MenuEntry::Item(MenuCommand::LatexProblem(i), true))
                    .chain(rc.into_iter().filter(|_| self.latex_rc_skipped()))
                    .collect()
            }
            MenuTarget::RunConfigs => (0..self.run.list.len())
                .map(|i| MenuEntry::Item(MenuCommand::RunSelect(i), true))
                .chain([
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuCommand::RunEdit, true),
                ])
                .collect(),
            MenuTarget::TerminalShells => (0..self.terminal_shell_list().len())
                .map(|i| MenuEntry::Item(MenuCommand::NewTerminalShell(i), true))
                .collect(),
            MenuTarget::StatusPlaces(_) => self.location_menu(),
            MenuTarget::RebaseAction(_) => (crate::app::git::rebase::REBASE_ACTIONS.iter())
                .map(|&action| MenuEntry::Item(MenuCommand::RebaseAction(action), true))
                .collect(),
            MenuTarget::FormBase => form_base_menu(&self.git.branches),
            // 以 `-` 开头的远程名不能推送，不列出。
            MenuTarget::FormRemote => (self.git.remotes.iter().enumerate())
                .filter(|(_, remote)| !remote.starts_with('-'))
                .map(|(i, _)| MenuEntry::Item(MenuCommand::FormRemote(i), true))
                .collect(),
            MenuTarget::LogFile(_, path) => {
                log_file_menu(self.may_exist(path), self.relative_path(path).is_some())
            }
            MenuTarget::LogCommit(oid) => {
                let commit = self.log_commit(oid);
                let tags: Vec<&str> = (commit.iter().flat_map(|c| &c.refs))
                    .filter_map(crate::app::git::log::menus::tag_name)
                    .collect();
                // 指向这个提交的分支，各自的操作与分支面板中相同；分支列表属于提交视图所在的
                // 仓库，还没换过去（日志没有仓库）时不列出。
                let (held, stale) = (self.git.held_branches(), self.git.stale_branches());
                let all = &self.git.branches;
                let same = self.active_log_root() == self.git.root.as_deref();
                let branches = (commit.iter().filter(|_| same).flat_map(|c| &c.refs))
                    .filter_map(|r| match r {
                        Ref::Head(name) | Ref::Branch(name) => {
                            all.iter().position(|b| !b.remote && b.name == *name)
                        }
                        Ref::Remote(name) => all.iter().position(|b| b.remote && b.name == *name),
                        _ => None,
                    })
                    .map(|i| {
                        let running = self.git.branch.running.is_some();
                        let ops =
                            crate::app::git::branch::branch_ops(i, all, &held, &stale, running);
                        (all[i].name.clone(), ops)
                    })
                    .collect();
                let actions = self.log_commit_actions(oid);
                log_commit_menu(actions, &self.current_branch_label(), branches, &tags)
            }
            MenuTarget::LogColumns => self.log_columns_menu(),
            MenuTarget::LogDate | MenuTarget::LogPath | MenuTarget::LogAuthor => {
                self.log_filter_menu(&target)
            }
            // 目录菜单不经过这里（`App::open_crumb_menu`）。
            MenuTarget::Folder => return,
            MenuTarget::Location(path, worktree) => self.location_row_menu(path, *worktree),
            MenuTarget::BranchOps(index) => crate::app::git::branch::branch_ops(
                *index,
                &self.git.branches,
                &self.git.held_branches(),
                &self.git.stale_branches(),
                self.git.branch.running.is_some(),
            ),
            MenuTarget::TagOps(index) => {
                crate::app::git::branch::tag_ops(*index, self.git.branch.running.is_some())
            }
        };

        // 菜单所指的文件在远端主机上：不列出只能在本机执行的项。
        let place = match &target {
            MenuTarget::Tree(path) => path.as_deref().or(self.editor.root()),
            MenuTarget::Tab(id) => (self.editor.document(*id)).and_then(|d| d.path.as_deref()),
            MenuTarget::CommitFile(path) | MenuTarget::LogFile(_, path) => Some(path.as_path()),
            _ => None,
        };
        let entries = match place {
            Some(path) if winc_backend::is_remote(path) => {
                let mounted = self.local_folder(path).is_some();
                without_local_only(entries, self.terminal_enabled(), mounted)
            }
            _ => entries,
        };

        // 菜单上显示的快捷键按菜单弹出的位置取：文件树中取文件树的键位，编辑区中取编辑区的键位，
        // 编辑 Markdown 时取 Markdown 的键位（「粘贴为纯文本」只在这里有键）。
        let context = match &target {
            MenuTarget::Tree(_) => Context::Tree,
            MenuTarget::Editor(_) | MenuTarget::Markdown if self.markdown_keys() => {
                Context::Markdown
            }
            MenuTarget::Editor(_)
            | MenuTarget::Diff(..)
            | MenuTarget::DiffGutter(..)
            | MenuTarget::Markdown
            | MenuTarget::LogCommit(_) => Context::Editor,
            MenuTarget::Terminal(_) | MenuTarget::TerminalTab(_) => Context::Terminal,
            MenuTarget::SearchResult(..) => Context::Panel,
            MenuTarget::CommitFile(_) => Context::Changes,
            _ => Context::Other,
        };
        // 状态栏的菜单行为类似弹出式按钮：当前选中的项显示在被点击的位置上。
        let at_checked = matches!(target, MenuTarget::Indent(_) | MenuTarget::LineEnding(_));
        // 拆分按钮的菜单：`at` 是菜单的右上角，菜单往左展开。
        let right_aligned = matches!(
            target,
            MenuTarget::CommitButton
                | MenuTarget::TabList
                | MenuTarget::LatexProblems
                | MenuTarget::Locale
                | MenuTarget::NamedTheme(_)
                | MenuTarget::Whitespace
                | MenuTarget::VimClipboard
                | MenuTarget::OpenWorktree
                | MenuTarget::PullMode
                | MenuTarget::DropFolder
                | MenuTarget::SearchStyle
                | MenuTarget::AutoSave
                | MenuTarget::SmoothCaret
                | MenuTarget::Zoom
                | MenuTarget::BackgroundImage
                | MenuTarget::BackgroundFit
                | MenuTarget::TextWidth
                | MenuTarget::DefaultLanguage
                | MenuTarget::TerminalPosition
                | MenuTarget::TerminalTabPosition
                | MenuTarget::OptionAsMeta
                | MenuTarget::Font(_)
        );
        // 设置中语言、字体等选项的菜单不窄于弹出它的控件。
        let control = match target {
            MenuTarget::Locale => Some(Setting::Language),
            MenuTarget::Whitespace => Some(Setting::RenderWhitespace),
            MenuTarget::VimClipboard => Some(Setting::VimClipboard),
            MenuTarget::OpenWorktree => Some(Setting::GitOpenWorktree),
            MenuTarget::PullMode => Some(Setting::GitPullMode),
            MenuTarget::DropFolder => Some(Setting::DropFolder),
            MenuTarget::SearchStyle => Some(Setting::FindInFiles),
            MenuTarget::AutoSave => Some(Setting::AutoSave),
            MenuTarget::SmoothCaret => Some(Setting::SmoothCaret),
            MenuTarget::Zoom => Some(Setting::Zoom),
            MenuTarget::BackgroundImage => Some(Setting::BackgroundImage),
            MenuTarget::BackgroundFit => Some(Setting::BackgroundFit),
            MenuTarget::TextWidth => Some(Setting::TextWidth),
            MenuTarget::DefaultLanguage => Some(Setting::DefaultLanguage),
            MenuTarget::TerminalPosition => Some(Setting::TerminalPosition),
            MenuTarget::TerminalTabPosition => Some(Setting::TerminalTabPosition),
            MenuTarget::OptionAsMeta => Some(Setting::TerminalOptionAsMeta),
            MenuTarget::Font(setting) => Some(setting),
            _ => None,
        };
        let min_width = control
            .and_then(|setting| self.settings_layout().row(setting).map(|row| row.control.w))
            .unwrap_or(0.0);

        // 菜单项是否打勾、显示什么取决于目标，因此先记录目标，再准备菜单项。
        let (branches, stashes) = match target {
            MenuTarget::BranchOps(_) | MenuTarget::CommitMore => {
                (self.git.branches.clone(), self.git.stashes.clone())
            }
            MenuTarget::LogCommit(_) => (self.git.branches.clone(), Vec::new()),
            MenuTarget::StashRow(_) | MenuTarget::StashList => {
                (Vec::new(), self.git.stashes.clone())
            }
            MenuTarget::FormBase => (self.git.branches.clone(), Vec::new()),
            _ => Default::default(),
        };
        self.overlays.context_menu = Some(ContextMenu {
            target,
            entries: entries.clone(),
            branches,
            stashes,
            popup: None,
        });
        self.set_hover(Hover::None);

        // 没有系统菜单（macOS 以外的平台，以及没有窗口的测试中）：在窗口里画出来，由应用自己
        // 处理悬停、点击和按键，打开期间菜单按钮保持按下的样子。
        if !cfg!(target_os = "macos") || self.window.handle.is_none() {
            let rows = self.popup_rows(&entries, context, &mut 0);
            if let Some(menu) = &mut self.overlays.context_menu {
                let mut popup = PopupMenu::new(rows, at, (at_checked, right_aligned));
                popup.min_width = min_width;
                menu.popup = Some(popup);
            }
            self.update_cursor_icon();
            self.request_redraw();
            return;
        }

        #[cfg(target_os = "macos")]
        let prepared = self.prepare_menu(&entries, context, &mut 0);

        // 菜单打开期间系统在跟踪菜单事件，窗口不会重绘，因此先把选中的行同步绘制一次，
        // 再弹出菜单。
        self.paint_now();

        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.handle.clone() {
            let items: Vec<_> = prepared
                .iter()
                .map(|p| p.as_ref().map(Prepared::item))
                .collect();
            // 界面的逻辑像素乘以界面缩放倍数得到 `AppKit` 的点。
            let zoom = f64::from(self.zoom());
            let at = (f64::from(at.0) * zoom, f64::from(at.1) * zoom);
            let place = crate::macos::MenuPlace {
                at,
                at_checked,
                right_aligned,
                min_width: f64::from(min_width) * zoom,
            };
            crate::macos::popup_menu(&window, &items, place);
        }
    }

    /// 先算好各菜单项需要的数据（标题、快捷键、图标、是否打勾），再借给系统菜单。
    ///
    /// 菜单项按显示顺序从 1 起编号，和 [`MenuEntry::command_at`] 数的一样。
    /// `numbered` 是已经编到的号。
    #[cfg(target_os = "macos")]
    fn prepare_menu(
        &mut self,
        entries: &[MenuEntry],
        context: Context,
        numbered: &mut isize,
    ) -> Vec<Option<Prepared>> {
        use crate::macos::key_equivalent;

        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(command, enabled) | MenuEntry::Alternate(command, enabled) => {
                    Some(Prepared {
                        tag: {
                            *numbered += 1;
                            *numbered
                        },
                        label: self.menu_label(*command),
                        key: command
                            .shortcut(context)
                            .and_then(key_equivalent)
                            .unwrap_or_default(),
                        icon: self.menu_icon(*command),
                        icon_size: self.menu_icon_size(*command),
                        enabled: *enabled,
                        checked: self.menu_checked(*command),
                        alternate: matches!(entry, MenuEntry::Alternate(..)),
                        children: Vec::new(),
                    })
                }
                MenuEntry::Submenu(title, items) => Some(Prepared {
                    label: MenuLabel::plain(title),
                    key: Default::default(),
                    icon: submenu_lead(items).and_then(|c| self.menu_icon(c)),
                    icon_size: submenu_lead(items).map_or(MENU_ICON, |c| self.menu_icon_size(c)),
                    enabled: true,
                    checked: false,
                    alternate: false,
                    tag: 0,
                    children: self.prepare_menu(items, context, numbered),
                }),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    /// 是否给菜单项 `command` 打勾。
    ///
    /// 状态栏缩进、行尾、编码菜单中当前使用的那一项打勾；「排除的语言」菜单中已被排除的语言打勾；
    /// 格式化工具菜单中当前使用的那一项打勾。
    pub(in crate::app) fn menu_checked(&self, command: MenuCommand) -> bool {
        // 设置中的默认语言：当前的设置打勾。
        let target = self.overlays.context_menu.as_ref().map(|menu| &menu.target);
        if let Some(MenuTarget::DefaultLanguage) = target {
            let chosen = self.default_language().map(|language| language.id);
            return match command {
                MenuCommand::UsePlainText => chosen.is_none(),
                MenuCommand::UseLanguage(i) => {
                    chosen.is_some() && winc_syntax::languages().get(i).map(|l| l.id) == chosen
                }
                _ => false,
            };
        }
        if let MenuCommand::ShowTab(i) = command {
            return self.editor.active == Some(i);
        }
        if let MenuCommand::RunSelect(i) = command {
            let current = self.run.current();
            return current.is_some_and(|c| self.run.list.get(i) == Some(c));
        }
        if command == MenuCommand::ToggleBlame {
            let doc = self.editor.active();
            return match doc.and_then(|d| d.diff.as_deref()) {
                Some(view) => view.blamed.contains(&true),
                None => doc.is_some_and(|d| self.git.blame.is_on(d.id)),
            };
        }
        if let MenuCommand::LogColumn(column) = command {
            return self.log_column_shown(column);
        }
        if let Some(on) = self.log_filter_checked(command) {
            return on;
        }
        if command == MenuCommand::CommitAmend {
            return self.git.commit.amend;
        }
        if command == MenuCommand::CommitTree {
            return !self.git.commit.flat();
        }
        if command == MenuCommand::CommitList {
            return self.git.commit.flat();
        }
        if command == MenuCommand::CommitChangeList {
            return !self.git.commit.staging();
        }
        if let MenuCommand::CommitSortBy(sort) = command {
            return self.git.commit.sort() == sort;
        }
        if command == MenuCommand::CommitAllRepositories {
            return self.commit_all_repositories();
        }
        if command == MenuCommand::CommitStagingArea {
            return self.git.commit.staging();
        }
        if command == MenuCommand::CommitSignOff {
            return self.commit_signs_off();
        }
        if command == MenuCommand::CommitNoVerify {
            return self.git.commit.no_verify;
        }
        if command == MenuCommand::CommitCheckTodo {
            return self.state.commit_check_todo;
        }
        if command == MenuCommand::CommitReformat {
            return self.state.commit_reformat;
        }
        if command == MenuCommand::TerminalMaximize {
            return self.terminal.maximized;
        }
        if command == MenuCommand::MarkdownLineNumbers {
            return self.markdown_line_numbers_on();
        }
        // 日志的作者筛选：现在用着的那一项。
        let query = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref())
            .map(|v| &v.query);
        if let MenuCommand::FormBase(i) = command {
            return self.form_base_chosen(i);
        }
        if let MenuCommand::FormRemote(i) = command {
            return self.form_remote_chosen(i);
        }
        if let MenuCommand::StatusPlace(i) = command {
            return self.location_places().get(i).is_some_and(|p| p.current);
        }
        if let (MenuCommand::LogAuthor(i), Some(q)) = (command, query) {
            return match i.checked_sub(1) {
                None => q.authors.is_empty(),
                Some(i) => (self.git.log.authors.get(i)).is_some_and(|a| q.authors.contains(a)),
            };
        }

        // 状态栏的缩进、行尾、编码、语言菜单：当前使用的那一种。
        let status_doc = match &self.overlays.context_menu {
            Some(ContextMenu {
                target:
                    MenuTarget::Indent(id)
                    | MenuTarget::LineEnding(id)
                    | MenuTarget::Encoding(id)
                    | MenuTarget::Language(id),
                ..
            }) => self.editor.document(*id),
            _ => None,
        };
        if let Some(doc) = status_doc {
            let tab_width = self.editor.tab_width;
            return match command {
                MenuCommand::IndentUsingSpaces => matches!(doc.indent(), Indent::Spaces(_)),
                MenuCommand::IndentUsingTabs => doc.indent() == Indent::Tabs,
                MenuCommand::IndentSize(n) => doc.indent_size(tab_width) == n,
                MenuCommand::LineEnding(eol) => doc.line_ending == eol,
                MenuCommand::ReopenEncoding(i) | MenuCommand::SaveEncoding(i) => {
                    doc.hex.is_none()
                        && CHOICES
                            .get(i)
                            .is_some_and(|(e, _)| *e == doc.encoding.choice())
                }
                MenuCommand::ReopenHex => doc.hex.is_some(),
                MenuCommand::UseLanguage(i) => (winc_syntax::languages().get(i))
                    .is_some_and(|l| std::ptr::eq(*l, doc.highlighter.language())),
                MenuCommand::UsePlainText => doc.highlighter.language().id == "text",
                _ => false,
            };
        }

        let tool = match &self.overlays.context_menu {
            Some(ContextMenu {
                target: MenuTarget::Tool(task, id),
                ..
            }) => Some((*task, *id)),
            _ => None,
        };
        let language = tool.map(|(_, id)| id);
        let configured = tool.and_then(|(task, id)| {
            crate::app::editor::format::task_commands(&self.config, task).get(id)
        });
        match command {
            MenuCommand::SkipOnSave(i) => winc_syntax::languages().get(i).is_some_and(|l| {
                self.config
                    .format_on_save_exclude
                    .iter()
                    .any(|id| id == l.id)
            }),
            MenuCommand::Locale(i) => {
                winc_i18n::LocaleChoice::ALL.get(i)
                    == Some(&self.config.language.unwrap_or_default())
            }
            MenuCommand::NamedTheme(dark, i) => self.theme_slot(dark) == i,
            MenuCommand::KeymapPreset(preset) => crate::keymap::current_preset() == preset,
            MenuCommand::Whitespace(i) => {
                winc_config::RenderWhitespace::ALL.get(i)
                    == Some(&self.config.render_whitespace.unwrap_or_default())
            }
            MenuCommand::VimClipboard(i) => {
                winc_config::VimClipboard::ALL.get(i)
                    == Some(&self.config.vim_clipboard.unwrap_or_default())
            }
            MenuCommand::OpenWorktree(i) => {
                winc_config::WorktreeOpen::ALL.get(i)
                    == Some(&self.config.git_open_worktree.unwrap_or_default())
            }
            MenuCommand::PullMode(i) => {
                winc_config::PullMode::ALL.get(i) == Some(&self.config.git_pull.unwrap_or_default())
            }
            MenuCommand::SearchStyle(i) => {
                winc_config::SearchStyle::ALL.get(i) == Some(&self.search_style())
            }
            MenuCommand::DropFolder(i) => {
                winc_config::FolderDrop::ALL.get(i)
                    == Some(&self.config.drop_folder.unwrap_or_default())
            }
            MenuCommand::AutoSave(i) => {
                winc_config::AutoSave::ALL.get(i)
                    == Some(&self.config.auto_save.unwrap_or_default())
            }
            MenuCommand::SmoothCaret(i) => {
                winc_config::SmoothCaret::ALL.get(i)
                    == Some(&self.config.smooth_caret_movement.unwrap_or_default())
            }
            MenuCommand::DefaultShell(i) => {
                let configured = self.configured_shell().map(|shell| shell.command);
                let listed = i.and_then(|i| self.terminal.shells.get(i));
                configured == listed.map(|shell| shell.command.clone())
            }
            MenuCommand::DefaultShellCustom => self.configured_shell().is_some_and(|configured| {
                (self.terminal.shells.iter()).all(|shell| shell.command != configured.command)
            }),
            MenuCommand::TerminalPosition(right) => self.terminal_right() == right,
            MenuCommand::TerminalTabPosition(right) => {
                (self.config.terminal_vertical_tabs == Some(true)) == right
            }
            MenuCommand::Zoom(i) => {
                crate::app::settings::ZOOM_STEPS.get(i) == Some(&self.config.ui_zoom.unwrap_or(100))
            }
            MenuCommand::BackgroundNone => self.config.ui_background_image.is_none(),
            MenuCommand::TextWidth(i) => {
                winc_config::TextWidth::ALL.get(i)
                    == Some(&self.config.markdown_text_width.unwrap_or_default())
            }
            MenuCommand::BackgroundFit(i) => {
                winc_config::BackgroundFit::ALL.get(i)
                    == Some(&self.config.ui_background_fit.unwrap_or_default())
            }
            MenuCommand::OptionAsMeta(i) => {
                let current =
                    (self.config.option_as_meta).unwrap_or(winc_config::OptionAsMeta::Left);
                winc_config::OptionAsMeta::ALL.get(i) == Some(&current)
            }
            MenuCommand::FontDefault | MenuCommand::Font(_) | MenuCommand::FallbackFont(_) => {
                let Some(ContextMenu {
                    target: MenuTarget::Font(setting),
                    ..
                }) = &self.overlays.context_menu
                else {
                    return false;
                };
                // 字体菜单只看列表中的第一个字体，「后备字体」看其余的字体。
                let list = winc_config::font_list(self.font_value(*setting).as_deref());
                let name = |i: usize| winc_render::installed_fonts().get(i).map(|f| &*f.name);
                match command {
                    MenuCommand::Font(i) => name(i).is_some_and(|n| list.first() == Some(&n)),
                    MenuCommand::FallbackFont(i) => {
                        name(i).is_some_and(|n| list.iter().skip(1).any(|&m| m == n))
                    }
                    _ => list.is_empty(),
                }
            }
            MenuCommand::ToolAuto => language.is_some() && configured.is_none(),
            MenuCommand::ToolPreset(i) => {
                configured.is_some_and(|c| PRESETS.get(i).is_some_and(|p| p.command == c))
            }
            MenuCommand::ToolCustom => {
                configured.is_some_and(|c| PRESETS.iter().all(|p| p.command != c))
            }
            _ => false,
        }
    }

    /// 返回菜单项左边的图标：最近的目录用目录图标，工作树用分支图标，标签列表用标签上的图标，
    /// 语言用这种语言的文件的图标；其他命令没有图标。
    pub(in crate::app) fn menu_icon_kind(&self, command: MenuCommand) -> Option<MenuIcon> {
        match command {
            MenuCommand::OpenRecent(i) | MenuCommand::OpenRecentInNewWindow(i) => {
                let group = (self.state.recent.get(i))
                    .is_some_and(|folder| self.recent_group(folder).is_some());
                Some(if group {
                    MenuIcon::Folders
                } else {
                    MenuIcon::Folder
                })
            }
            MenuCommand::OpenGroup(_) => Some(MenuIcon::Folders),
            MenuCommand::GroupFolder(..) => Some(MenuIcon::Folder),
            MenuCommand::ShowTab(i) => self.editor.documents.get(i).map(|_| MenuIcon::Tab(i)),
            MenuCommand::UseLanguage(_) | MenuCommand::UsePlainText => {
                let language = match command {
                    MenuCommand::UseLanguage(i) => winc_syntax::languages().get(i).copied()?,
                    _ => winc_syntax::detect(""),
                };
                let (glyph, colors) = crate::theme::language_kind(language);
                Some(MenuIcon::File(glyph, colors))
            }
            _ => None,
        }
    }

    /// 在 `(x, y)`（图标位的左上角）绘制菜单项图标 `icon`，图标位的尺寸见
    /// [`MenuIcon::size`]。
    ///
    /// 窗口的深浅外观跟随主题，菜单跟随窗口，因此图标颜色与菜单的深浅外观一致。
    pub(crate) fn draw_menu_icon(
        &self,
        frame: &mut winc_render::Frame<'_>,
        icon: MenuIcon,
        (x, y): (f32, f32),
        clip: Rect,
    ) {
        let (w, h) = icon.size();
        match icon {
            MenuIcon::Folder => icons::folder(frame, (x, y), false, self.palette.folder, clip),
            MenuIcon::Folders => icons::folders(frame, (x, y), self.palette.folder, clip),
            MenuIcon::File(glyph, [dark, light]) => {
                let color = winc_render::Color::hex(if self.palette.dark { dark } else { light });
                // 文件图标 12 × 15。
                icons::file(frame, (x + 2.0, y + 0.5), glyph, color, clip);
            }
            MenuIcon::Tab(i) => {
                let Some(doc) = self.editor.documents.get(i) else {
                    return;
                };
                if doc.log.is_some() {
                    icons::git(
                        frame,
                        (x + w / 2.0, y + h / 2.0),
                        1.0,
                        self.palette.accent,
                        clip,
                    );
                } else {
                    let (glyph, color) = self
                        .palette
                        .file_icon(&crate::app::workspace::tab_names::tab_icon_name(doc));
                    // 文件图标 12 × 15。
                    icons::file(frame, (x + 2.0, y + 0.5), glyph, color, clip);
                }
            }
        }
    }

    /// 返回菜单项 `command` 的图标的逻辑尺寸（宽，高）；没有图标时为 [`MENU_ICON`]。
    #[cfg(any(target_os = "macos", test))]
    pub(in crate::app) fn menu_icon_size(&self, command: MenuCommand) -> (f32, f32) {
        (self.menu_icon_kind(command)).map_or(MENU_ICON, MenuIcon::size)
    }

    /// 把菜单项 `command` 的图标画成图片，交给系统菜单；没有图标或没有渲染器时为 `None`。
    #[cfg(any(target_os = "macos", test))]
    pub(in crate::app) fn menu_icon(&mut self, command: MenuCommand) -> Option<Image> {
        let icon = self.menu_icon_kind(command)?;
        let (w, h) = icon.size();
        let clip = Rect::new(0.0, 0.0, w, h);
        let mut renderer = self.frame.renderer.take()?;
        let image = renderer.draw_transparent(w, h, |frame| {
            self.draw_menu_icon(frame, icon, (0.0, 0.0), clip);
        });
        self.frame.renderer = Some(renderer);
        Some(image)
    }

    /// 文件树工具栏「⋯」的菜单：列出收起的各个工具，按工具栏中的顺序排列。
    pub(super) fn tree_tools_menu(&self) -> Vec<MenuEntry> {
        crate::layout::folded_tools(self.hits.tree_tools.len())
            .iter()
            .map(|&tool| {
                let command = match tool {
                    TreeTool::NewFile => MenuCommand::NewFile,
                    TreeTool::NewFolder => MenuCommand::NewFolder,
                    TreeTool::Refresh => MenuCommand::TreeRefresh,
                    TreeTool::CollapseAll => MenuCommand::TreeCollapseAll,
                    TreeTool::Locate => MenuCommand::TreeLocate,
                };
                MenuEntry::Item(command, true)
            })
            .collect()
    }

    /// 处理对项目名的点击：在项目名下方弹出最近项目菜单。
    pub(in crate::app) fn open_project_menu(&mut self) {
        self.git_discover();
        self.git_scan_repositories();
        let hit = self.hits.project;
        self.open_button_menu(MenuButton::Project, MenuTarget::Project, menu_below(hit));
    }

    /// 从按钮 `button` 上弹出 `target` 的菜单（在 `at` 处，见 [`App::open_context_menu`]）：系统
    /// 菜单开着期间按钮画成按下的样子，菜单关掉以后还原。
    pub(in crate::app) fn open_button_menu(
        &mut self,
        button: MenuButton,
        target: MenuTarget,
        at: (f32, f32),
    ) {
        self.overlays.menu_button = Some(button);
        self.open_context_menu(target, at);
        // 系统菜单关掉以后才返回；窗口内的菜单还开着，关掉时再放开按钮。
        if self.popup().is_none() {
            self.overlays.menu_button = None;
        }
        self.request_redraw();
    }

    /// 立即同步绘制一帧，不等待下一轮事件循环；画法与平常的一帧相同（见 `App::prepare_frame`）。
    pub(in crate::app) fn paint_now(&mut self) {
        self.prepare_frame();
        if let Some(mut renderer) = self.frame.renderer.take() {
            self.declare_base(&mut renderer);
            self.draw_scrolled(&mut renderer);
            self.frame.renderer = Some(renderer);
        }
    }
}
