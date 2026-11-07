//! 目录的打开、切换和关闭，最近打开的目录，以及每个目录的现场（标签和当前标签）。

use super::*;

/// 开始页（没有打开目录时）上可以点击的项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    /// 打开应用设置（`⌘,`）。
    Settings,
    /// 新建未命名文件（`⌘N`）。
    NewFile,
    /// macOS 上打开文件或目录（`⌘O`），选择器中都能选；其他平台上只打开文件（`Ctrl+O`）。
    Open,
    /// 打开目录（`Ctrl+K Ctrl+O`）；只在选择器不能同时选文件和目录的平台上显示。
    OpenFolder,
    /// 打开最近列表中的目录，参数为列表下标（从 0 起）。
    Recent(usize),
    /// 从最近列表中移除一项（悬停时右端的 ✕），参数为列表下标（从 0 起）。
    Forget(usize),
}

impl App {
    /// 打开目录 `path`（「打开」面板、最近列表、拖进窗口）。
    ///
    /// 窗口里已经开着另一个目录时，按 `editor.openFolderInNewWindow` 在新窗口或此窗口打开；
    /// 没有设置时弹出提示框询问。其余情况同 [`Self::switch_folder`]。
    pub(in crate::app) fn open_folder(&mut self, path: &Path) {
        let target = winc_config::files::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let other = self.editor.root().is_some_and(|root| root != target);
        if !other || self.focus_window_with(&target) {
            self.switch_folder(&target);
            return;
        }
        match self.config.open_folder_in_new_window {
            Some(true) => self.new_window(Some(target)),
            Some(false) => self.switch_folder(&target),
            None => self.ask_open_folder(target),
        }
    }

    /// 询问在新窗口还是此窗口打开目录 `target`。
    ///
    /// 按钮从右向左为「此窗口」（↩）、「新窗口」、「取消」（Esc），上方是「不再询问」勾选框。
    /// 没有窗口（测试中）时直接在此窗口打开。
    fn ask_open_folder(&mut self, target: PathBuf) {
        if self.window.is_some() {
            use crate::app::AlertButton;
            use winc_i18n::t;

            let name = winc_editor::display_name(&target);
            let current = self.editor.root().map(winc_editor::display_name);
            let message = t!("workspace-open-folder-where", name = name);
            let detail = t!(
                "workspace-open-folder-where-detail",
                current = current.unwrap_or_default()
            );
            let buttons = vec![
                AlertButton::new(t!("workspace-open-folder-this-window")),
                AlertButton::new(t!("workspace-open-folder-new-window")),
                AlertButton::cancel(t!("common-cancel")),
            ];
            let asked = Some((t!("workspace-tree-dont-ask"), false));
            self.show_alert(&message, &detail, buttons, asked, move |index, never| {
                (index < 2).then(|| UserEvent::OpenFolderIn(target.clone(), index == 1, never))
            });
            return;
        }

        self.switch_folder(&target);
    }

    /// 处理「要在哪里打开」提示框上的选择：`new_window` 为真时在新窗口打开 `path`，否则在此
    /// 窗口打开。
    ///
    /// `never_ask` 为真（勾选了「不再询问」）时把这次的选择写入配置，之后不再询问。
    pub(in crate::app) fn open_folder_in(
        &mut self,
        path: &Path,
        new_window: bool,
        never_ask: bool,
    ) {
        if never_ask {
            self.config.open_folder_in_new_window = Some(new_window);
            self.save_config();
        }
        if new_window {
            self.new_window(Some(path.to_path_buf()));
        } else {
            self.switch_folder(path);
        }
    }

    /// 打开目录 `path`。
    ///
    /// 已经打开了其他目录时先替换：记录当前目录的现场，关闭该目录的标签（有未保存修改的先询问）
    /// 和终端（有程序在运行的先确认），再打开新目录。`path` 就是当前目录时只显示文件树；已在
    /// 另一个 winc 窗口里打开时切到该窗口。
    pub(in crate::app) fn switch_folder(&mut self, path: &Path) {
        let target = winc_config::files::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // 开在另一个窗口里：切到该窗口，不在当前窗口再打开。
        if self.focus_window_with(&target) {
            return;
        }

        match self.editor.root().map(Path::to_path_buf) {
            Some(root) if root == target => self.show_files(),
            // 终端随原目录一起关掉（标签记入原目录的现场），有程序在运行时先确认。
            Some(_) => {
                self.remember_session();
                let request = CloseRequest::Switch(target);
                if !self.confirm_terminal_quit(request.clone()) {
                    self.request_close(request);
                }
            }
            None => self.open_folder_now(&target),
        }
    }

    /// 立即打开目录 `path` 并恢复该目录上次的标签，侧栏展开。
    ///
    /// 不处理当前已打开的目录和标签，调用方须先完成这些处理。
    pub(crate) fn open_folder_now(&mut self, path: &Path) {
        self.open_folder_restoring(path, true);
    }

    /// 打开文件所在的目录并恢复它上次的标签，侧栏保持收起。
    ///
    /// 单独打开文件时编辑区占满窗口；需要文件树时再打开侧栏。
    pub(crate) fn open_folder_for_files(&mut self, path: &Path) {
        self.open_folder_restoring(path, false);
    }

    /// 立即打开目录 `path` 并恢复该目录上次的标签。`sidebar` 为真时展开侧栏。
    fn open_folder_restoring(&mut self, path: &Path, sidebar: bool) {
        if let Err(e) = self.editor.open_folder(path) {
            let name = winc_editor::display_name(path);
            self.notify(winc_i18n::t!(
                "workspace-open-folder-failed",
                name = name,
                error = e.to_string()
            ));
            return;
        }

        self.apply_compact_folders();
        self.finish_folder_open(sidebar);
        self.focus = if sidebar {
            match self.sidebar_view {
                SidebarView::Files => Focus::Tree,
                SidebarView::Search => Focus::GlobalQuery,
                SidebarView::Commit => self.commit_view_focus(),
            }
        } else {
            Focus::Editor
        };

        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let session = self.state.session_for(&root).cloned();
        let restored = restore_tabs(
            &mut self.editor,
            session.as_ref(),
            self.backup.root.as_deref(),
        );
        self.adopt_restored(restored);
        if session
            .as_ref()
            .is_some_and(|session| session.sidebar_hidden)
        {
            self.set_sidebar(false);
            if self.focus_in_sidebar() {
                self.focus = Focus::Editor;
            }
        }
        // 当前标签在后台读取：上次的当前标签可能是很大的文件或在很慢的磁盘上。
        self.load_active_in_background();
        if self.editor.active.is_some() {
            self.focus = Focus::Editor;
            self.note_recent_file();
        }
        self.switch_terminal(session.as_ref().and_then(|s| s.terminal.as_ref()));
        self.switch_run_configs(session.as_ref().and_then(|s| s.run.clone()));

        self.relayout();
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 在工作区换成新打开的目录之后，完成界面切换。
    ///
    /// 清除与上一个目录相关的状态，按 `sidebar` 展开或收起侧栏，把窗口标题改为目录名，并记入最近列表。
    /// 不恢复标签。
    ///
    /// 侧栏停在原来的视图上（在「搜索」里换目录还在「搜索」，查的内容不变、在新目录里重新搜），
    /// 不跳回文件树；新目录不是 Git 仓库时「提交」视图没有了，由 `git_folder_changed` 回到文件树。
    fn finish_folder_open(&mut self, sidebar: bool) {
        self.reset_folder_views();
        self.watch_folder();
        self.set_sidebar(sidebar);
        if self.sidebar_view == SidebarView::Search && !self.global.query.text().is_empty() {
            self.run_global_search();
        }
        self.update_window_title();
        self.remember_workspace();
    }

    /// 关闭目录并回到开始页；调用方须先关闭所有标签。
    ///
    /// 打开目录时为应对进程异常中止而记下的 `last` 一并清除：之后再中止，下次启动是开始页。
    pub(in crate::app) fn close_folder_now(&mut self) {
        // `last` 是别的 winc 进程写的（指向别的目录）时不动，该进程异常中止后仍需用它恢复。
        if let Some(root) = self.editor.root().map(Path::to_path_buf) {
            self.update_state(|state| {
                if state.last.as_deref() == Some(root.as_path()) {
                    state.last = None;
                }
            });
        }
        self.editor.workspace = None;
        // 标签都关了，没有要备份的：放开这个目录的备份锁（见 `release_folder_lock`）。
        self.release_folder_lock();
        self.reset_folder_views();
        self.watch_folder();
        self.set_sidebar(false);
        self.focus = Focus::Editor;
        self.set_window_title("winc");
        self.relayout();
        self.request_redraw();
    }

    /// 清除与上一个目录相关的状态。
    ///
    /// 包括输入框和浮层、全局搜索结果、`⌘P` 面板的文件表、文件树的滚动位置和动画、
    /// 已关闭标签的记录（`⇧⌘T`）以及后退前进历史。
    fn reset_folder_views(&mut self) {
        // 输入框和浮层中保存的行号、路径属于旧目录，在新目录中无效。
        self.name_input = None;
        self.goto_input = None;
        self.tree_drag = None;
        self.global.engine.clear();
        self.global.collapsed.clear();
        self.global.selected = None;
        self.global.due = None;
        self.global.rebuild_rows();
        // 提交视图的滚动位置、选中的文件、收起的目录是旧仓库的。
        self.commit.scroll = 0.0;
        self.commit.clear_selection();
        self.commit.forget_dirs();
        self.picker.cancel();
        self.picker = FileIndex::default();
        self.definitions = None;
        if self.picker_open {
            self.close_picker();
        }
        self.tree_motion = None;
        self.tree_scroll = 0.0;
        self.tree_scroll_x = 0.0;
        self.closed.clear();
        self.navigation.clear();
    }

    /// 把当前目录移到最近列表的最前面并立即写入状态文件，避免进程异常中止后丢失。
    pub(crate) fn remember_workspace(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };

        // 退出时会再记录一次；此处先设置 `last`，使进程异常中止后下次启动仍能回到该目录，
        // 并找回未保存的内容。还没有现场的目录先记一个空现场：`last` 只认有现场的目录。
        self.update_state(|state| {
            state.remember(&root);
            if state.session_for(&root).is_none() {
                state.set_session(Session {
                    folder: root.clone(),
                    ..Session::default()
                });
            }
            state.last = Some(root);
        });
    }

    /// 目录 `folder` 开在另一个窗口里时请应用层切到该窗口，返回是否存在已打开 `folder` 的窗口。
    pub(in crate::app) fn focus_window_with(&self, folder: &Path) -> bool {
        let key = self.shared.window_with(folder, self.key);
        if let Some(key) = key {
            self.shared.request(crate::winc::Request::Focus(key));
        }
        key.is_some()
    }

    /// 退出前调用：记录当前目录的现场，以及下次启动时要打开的目录。
    ///
    /// 没有打开目录（例如 macOS 上先关闭了窗口），或设置中关掉了「启动时打开上次的目录」时，
    /// 下次启动显示开始页；只是 `last` 指向的目录正开在另一个窗口里时不清，那是它记下的。
    /// 目录的现场仍然记下，以后打开它时恢复标签和未保存的内容。
    pub(in crate::app) fn save_session(&mut self) {
        let reopen = self.config.restore_folder != Some(false);
        let root = self.editor.root().map(Path::to_path_buf);
        let shared = self.shared.clone();
        self.update_state(|state| match root {
            Some(root) if reopen => state.last = Some(root),
            _ if !reopen => state.last = None,
            _ => {
                if state
                    .last
                    .as_deref()
                    .is_some_and(|last| shared.window_with(last, None).is_none())
                {
                    state.last = None;
                }
            }
        });
        self.remember_session();
    }

    /// 记录当前目录的现场。
    ///
    /// 内容包括：有路径且仍在磁盘上的标签、有备份的未保存内容、书签，以及当前标签。
    pub(in crate::app) fn remember_session(&mut self) {
        let Some(folder) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };

        let mut tabs = Vec::new();
        let mut active = None;
        for (i, doc) in self.editor.documents.iter().enumerate() {
            let Some(tab) = self.session_tab(doc) else {
                continue;
            };
            if self.editor.active == Some(i) {
                active = Some(tabs.len());
            }
            tabs.push(tab);
        }

        let terminal = self.terminal_session();
        let run = self.run.selected.clone();
        let sidebar_hidden = !self.panel_visible;
        let bookmarks = self
            .editor
            .all_bookmarks()
            .into_iter()
            .map(|(path, marks)| (path, marks.lines().to_vec()))
            .collect();
        self.update_state(|state| {
            state.set_session(Session {
                folder,
                tabs,
                active,
                bookmarks,
                terminal,
                run,
                sidebar_hidden,
            });
        });
    }

    /// 最近列表中的目录已不存在（删除、移动，或者外接磁盘未挂载）：从列表中移除并提示。
    fn forget_missing_recent(&mut self, folder: &Path) {
        self.update_state(|state| state.forget(folder));
        self.notify(winc_i18n::t!(
            "workspace-recent-missing",
            path = winc_editor::display_path(folder, None)
        ));
    }

    /// 修改状态（最近目录、各目录的现场）并写入状态文件；失败时在标准错误输出原因。
    ///
    /// 先重新读出状态文件，在其上修改再写回（见 [`State::update`]），不会盖掉别的 winc 进程
    /// 刚写入的内容；内存中的状态随之换成最新的。测试和测量时没有状态文件，只改内存。
    pub(in crate::app) fn update_state(&mut self, change: impl FnOnce(&mut State)) {
        let Some(path) = self.state_path.clone() else {
            change(&mut self.state);
            return;
        };
        let (state, result) = State::update(&path, change);
        self.state = state;
        if let Err(e) = result {
            eprintln!("winc: 无法记下最近打开的目录：{e}");
        }
    }

    /// 新建窗口（`⇧⌘N`）；带 `folder` 时在新窗口中打开它（已经开在某个窗口里就切到该窗口）。
    pub(in crate::app) fn new_window(&mut self, folder: Option<PathBuf>) {
        self.shared.request(crate::winc::Request::NewWindow(folder));
    }

    /// 在新窗口中打开最近列表中的第 `index` 个目录（项目菜单里按住 `⌥`、开始页上按住 `⌥` 单击）。
    pub(in crate::app) fn open_recent_in_new_window(&mut self, index: usize) {
        let Some(folder) = self.state.recent.get(index).cloned() else {
            return;
        };
        if folder.is_dir() {
            self.new_window(Some(folder));
        } else {
            self.forget_missing_recent(&folder);
        }
    }

    /// 执行开始页上被点击的项 `action`。
    pub(in crate::app) fn run_start_action(&mut self, action: StartAction) {
        match action {
            StartAction::Settings => self.open_settings(),
            StartAction::NewFile => self.new_file(),
            StartAction::Open => self.pick(Pick::Open),
            StartAction::OpenFolder => self.pick(Pick::OpenFolder),
            StartAction::Recent(index) => {
                let Some(folder) = self.state.recent.get(index).cloned() else {
                    return;
                };

                if folder.is_dir() {
                    self.open_path(&folder);
                } else {
                    self.forget_missing_recent(&folder);
                }
            }
            StartAction::Forget(index) => {
                if let Some(folder) = self.state.recent.get(index).cloned() {
                    self.update_state(|state| state.forget(&folder));
                }
            }
        }

        self.set_hover(Hover::None);
        self.request_redraw();
    }
}
