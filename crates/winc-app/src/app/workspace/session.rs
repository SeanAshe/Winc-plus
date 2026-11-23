//! 目录的打开、切换和关闭，最近打开的目录，以及每个目录的现场（标签和当前标签）。

use std::sync::PoisonError;

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
    /// 连接到 SSH 主机；只在远程模块开着时显示。
    Remote,
    /// 打开最近列表中的目录，参数为列表下标（从 0 起）。
    Recent(usize),
    /// 从最近列表中移除一项（悬停时右端的 ✕），参数为列表下标（从 0 起）。
    Forget(usize),
    /// 「首次设置」：弹出「语言」菜单。
    Language,
    /// 「首次设置」：改用外观（跟随系统、浅色或深色）。
    Appearance(winc_config::ThemeChoice),
    /// 「首次设置」：弹出当前外观的「主题」菜单。
    Theme,
    /// 「首次设置」：开启或关闭一个可选模块。
    Module(crate::keymap::Module),
    /// 「首次设置」第一页底部的「继续」：进入快捷键一页。
    SetupNext,
    /// 「首次设置」快捷键一页底部的「返回」。
    SetupBack,
    /// 「首次设置」快捷键一页中的一项，参数见 `SetupKeys::choice`。
    SetupKeymap(usize),
    /// 「首次设置」快捷键一页底部的「完成」：结束首次设置。
    FinishSetup,
}

impl App {
    /// 打开目录 `path`（「打开」面板、最近列表、拖进窗口）。
    ///
    /// 窗口里已经开着另一个目录时，按 `editor.openFolderInNewWindow` 在新窗口或此窗口打开；
    /// 没有设置时弹出提示框询问。其余情况同 [`Self::switch_folder`]。
    /// `path` 可以是项目文件（见 [`super::project`]），按其第一个目录判断；该目录位于未连接的
    /// 远端主机时先连接，再打开项目。
    pub(in crate::app) fn open_folder(&mut self, path: &Path) {
        let Some((root, project)) = self.folder_target(path) else {
            return;
        };
        // 连接成功后远端模块打开的是项目的第一个目录，此处改为打开项目。
        if project.is_none()
            && let Some(file) = self.remote.project.take()
            && super::project::project_folders(&file).is_ok_and(|p| p.root == root)
        {
            self.open_folder(&file);
            return;
        }
        if let Some(project) = &project
            && let Some(at) = winc_backend::locate(&root)
            && !winc_backend::remote::is_connected(&at.target)
        {
            self.remote.project = Some(project.file.clone());
            self.reopen_remote(&root);
            return;
        }
        let target = project.map_or(root.clone(), |project| project.file);
        let other = self.editor.root().is_some_and(|open| open != root);
        if !other || self.focus_window_with(&root) {
            self.switch_folder(&target);
            return;
        }
        match self.config.open_folder_in_new_window {
            Some(true) => self.new_window(Some(target)),
            Some(false) => self.switch_folder(&target),
            None => self.ask_open_folder(target),
        }
    }

    /// 询问在新窗口还是此窗口打开目录 `target`，或者把它加入当前项目。
    ///
    /// 按钮从右向左为「此窗口」（↩）、「新窗口」、「添加到此项目」、「取消」（Esc），上方是
    /// 「不再询问」勾选框；「添加到此项目」仅在 `target` 可加入当前项目时出现，选择它不记录
    /// 「不再询问」。
    /// 没有窗口（测试中）时直接在此窗口打开。
    fn ask_open_folder(&mut self, target: PathBuf) {
        if self.window.handle.is_some() {
            use crate::app::AlertButton;
            use winc_i18n::t;

            let name = winc_editor::display_name(&target);
            let current = self.editor.root().map(winc_editor::display_name);
            let message = t!("workspace-open-folder-where", name = name);
            let detail = t!(
                "workspace-open-folder-where-detail",
                current = current.unwrap_or_default()
            );
            let attach =
                (self.editor.workspace.as_ref()).is_some_and(|ws| ws.can_add_folder(&target));
            let mut buttons = vec![
                AlertButton::new(t!("workspace-open-folder-this-window")),
                AlertButton::new(t!("workspace-open-folder-new-window")),
            ];
            if attach {
                buttons.push(AlertButton::new(t!("workspace-open-folder-attach")));
            }
            buttons.push(AlertButton::cancel(t!("common-cancel")));
            let asked = Some((t!("workspace-tree-dont-ask"), false));
            self.show_alert(
                &message,
                &detail,
                buttons,
                asked,
                move |index, never| match index {
                    0 | 1 => Some(UserEvent::OpenFolderIn(target.clone(), index == 1, never)),
                    2 if attach => Some(UserEvent::AttachFolder(target.clone())),
                    _ => None,
                },
            );
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
    ///
    /// `path` 为项目文件时打开项目；其第一个目录已在当前窗口打开时就地载入项目。
    pub(in crate::app) fn switch_folder(&mut self, path: &Path) {
        let Some((folder, project)) = self.folder_target(path) else {
            return;
        };
        let target = project.as_ref().map_or(folder.clone(), |p| p.file.clone());

        // 开在另一个窗口里：切到该窗口，不在当前窗口再打开。
        if self.focus_window_with(&folder) {
            return;
        }

        match self.editor.root().map(Path::to_path_buf) {
            Some(root) if root == folder => {
                if let Some(project) = &project {
                    self.apply_project(project);
                }
                self.show_files();
            }
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

    /// 立即打开目录 `path` 并恢复该目录上次的标签，启动时新建的文件尚未修改时先关掉。
    ///
    /// 不处理当前已打开的目录和其他标签，调用方须先完成这些处理。`path` 为项目文件时打开项目。
    pub(crate) fn open_folder_now(&mut self, path: &Path) {
        let Some((folder, project)) = self.folder_target(path) else {
            return;
        };
        if let Some(project) = &project
            && let Some(at) = winc_backend::locate(&folder)
            && !winc_backend::remote::is_connected(&at.target)
        {
            self.remote.project = Some(project.file.clone());
            self.reopen_remote(&folder);
            return;
        }
        let path = folder.as_path();
        if let Some(index) = self.startup_file_index() {
            self.discard_tab(self.editor.documents[index].id);
        }
        // 有事件循环时目录交给后台读：它可能在很慢的磁盘或网络盘上。测试中就地读。
        let opened = match self.link.proxy {
            Some(_) => self.editor.open_folder_later(path),
            None => self.editor.open_folder(path),
        };
        if let Err(e) = opened {
            let name = winc_editor::display_name(path);
            self.notify(
                NoticeKind::Error,
                winc_i18n::t!(
                    "workspace-open-folder-failed",
                    name = name,
                    error = e.to_string()
                ),
            );
            return;
        }

        self.apply_compact_folders();
        if let Some(project) = &project {
            self.load_project(project);
        }
        self.folder_opened();
        self.focus = match self.panels.sidebar.view {
            SidebarView::Files => Focus::Tree,
            SidebarView::Search => Focus::GlobalQuery,
            SidebarView::Commit => self.commit_view_focus(),
        };

        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        // 项目的现场以项目文件为键；兼容旧版本以第一个目录为键、属于同一项目的现场。
        let session = match &project {
            Some(project) => (self.state.session_for(&project.file).cloned()).or_else(|| {
                let old = self.state.session_for(&root)?;
                (old.project.as_ref() == Some(&project.file)).then(|| old.clone())
            }),
            None => self.state.session_for(&root).cloned(),
        };
        // 单独打开一组中的打开的目录时不恢复附加的目录，这一组仍记在会话里。
        let alone = self.disk.open_alone.take().is_some_and(|path| path == root);
        self.disk.solo = alone && session.as_ref().is_some_and(|s| !s.folders.is_empty());
        match &project {
            Some(_) => {}
            None if !self.disk.solo => {
                self.restore_group(session.as_ref());
                self.add_idea_modules(&root);
            }
            None => {}
        }
        self.watch_extra_folders();
        // 备份和各标签的文件在后台读：要等备份锁，目录可能在很慢的磁盘上。
        self.disk.restoring = Some(root.clone());
        let backups = self.backup.root.clone();
        let slot = Arc::clone(&self.disk.prepared);
        self.spawn_blocking_then(UserEvent::TabsPrepared, move || {
            let prepared = prepare_restore(Some(&root), session.as_ref(), backups.as_deref());
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            slot.push((root, session, prepared));
        });
        self.request_redraw();
    }

    /// 是否正在后台读取备份、还没有恢复打开的目录的标签；这时编辑区显示正在恢复。
    pub(crate) fn restoring_tabs(&self) -> bool {
        self.disk.restoring.is_some()
    }

    /// 后台读好了恢复标签要用的内容：恢复该目录上次的标签、终端和运行配置。期间换了目录时丢弃。
    ///
    /// 期间打开的文件仍是当前标签，不被现场里当时的当前标签取代。
    pub(in crate::app) fn poll_prepared_tabs(&mut self) {
        // 几个目录的结果可能先后到达：只取正在等的那个目录的，其余的丢弃。
        let prepared = {
            let mut slot = (self.disk.prepared.lock()).unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *slot)
        };
        let wanted = |root: &PathBuf| {
            self.disk.restoring.as_ref() == Some(root) && self.editor.root() == Some(root)
        };
        let prepared = prepared.into_iter().rfind(|(root, ..)| wanted(root));
        let Some((_, session, prepared)) = prepared else {
            return;
        };
        self.disk.restoring = None;
        let opened = self.editor.active().map(|doc| doc.id);
        let restored = restore_prepared(&mut self.editor, session.as_ref(), prepared);
        if let Some(index) = opened.and_then(|id| self.editor.index_of(id)) {
            self.editor.active = Some(index);
        }
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
    /// 清除与上一个目录相关的状态，展开侧栏，把窗口标题改为目录名，并记入最近列表。不恢复标签。
    ///
    /// 侧栏停在原来的视图上（在「搜索」里换目录还在「搜索」，查的内容不变、在新目录里重新搜），
    /// 不跳回文件树；新目录不是 Git 仓库时「提交」视图没有了，由 `git_folder_changed` 回到文件树。
    pub(super) fn folder_opened(&mut self) {
        self.reset_folder_views();
        self.watch_folder();
        self.set_sidebar(true);
        if self.panels.sidebar.view == SidebarView::Search
            && !self.panels.global.query.text().is_empty()
        {
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
        if let Some(root) = self.session_key() {
            self.update_state(move |state| {
                if state.last.as_deref() == Some(root.as_path()) {
                    state.last = None;
                }
            });
        }
        self.editor.workspace = None;
        self.disk.restoring = None;
        // 标签都关了，没有要备份的：放开这个目录的备份锁（见 `release_folder_lock`）。
        self.release_folder_lock();
        self.reset_folder_views();
        self.watch_folder();
        self.set_sidebar(false);
        self.focus = Focus::Editor;
        self.set_window_title(crate::layout::PRODUCT_NAME);
        self.relayout();
        self.request_redraw();
    }

    /// 清除与上一个目录相关的状态。
    ///
    /// 包括输入框和浮层、全局搜索结果、`⌘P` 面板的文件表、文件树的滚动位置和动画、
    /// 已关闭标签的记录（`⇧⌘T`）以及后退前进历史。
    fn reset_folder_views(&mut self) {
        // 等文件树读好后要做的事（选中某一行等）属于旧目录。
        self.panels.tree.scans.forget_follow_ups();
        // 输入框和浮层中保存的行号、路径属于旧目录，在新目录中无效。
        self.panels.name_input = None;
        self.bars.goto_input = None;
        self.panels.tree.drag = None;
        self.panels.global.engine.clear();
        self.panels.global.collapsed.clear();
        self.panels.global.selected = None;
        self.panels.global.due = None;
        self.panels.global.rebuild_rows();
        // 提交视图的滚动位置、选中的文件、收起的目录是旧仓库的。
        self.git.commit.scroll = 0.0;
        self.git.commit.clear_selection();
        self.git.commit.forget_dirs();
        self.overlays.picker.index.cancel();
        self.overlays.picker.index = FileIndex::default();
        self.jumps.definitions = None;
        if self.overlays.picker.open {
            self.close_picker();
        }
        self.panels.tree.motion = None;
        self.panels.tree.scroll = 0.0;
        self.panels.tree.scroll_x = 0.0;
        self.history.closed.clear();
        self.jumps.navigation.clear();
    }

    /// 把当前项目（见 [`App::session_key`]）移到最近列表的最前面并立即写入状态文件，避免进程
    /// 异常中止后丢失。
    pub(crate) fn remember_workspace(&mut self) {
        let Some(root) = self.session_key() else {
            return;
        };

        // 退出时会再记录一次；此处先设置 `last`，使进程异常中止后下次启动仍能回到该目录，
        // 并找回未保存的内容。还没有现场的目录先记一个空现场：`last` 只认有现场的目录。
        self.update_state(move |state| {
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
        let key = self.link.shared.window_with(folder, self.link.key);
        if let Some(key) = key {
            self.link.shared.request(crate::winc::Request::Focus(key));
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
        let root = self.session_key();
        let open = self.link.shared.open_roots();
        self.update_state(move |state| match root {
            Some(root) if reopen => state.last = Some(root),
            _ if !reopen => state.last = None,
            _ => {
                if (state.last.as_ref()).is_some_and(|last| !open.contains(last)) {
                    state.last = None;
                }
            }
        });
        self.remember_session();
    }

    /// 记录当前目录的现场，以及各文件此刻的位置（见 `places`）；没有打开目录时只记后者。
    ///
    /// 现场的内容包括：有路径且仍在磁盘上的标签、有备份的未保存内容、书签、折叠的块、大纲里
    /// 收起的符号，以及当前标签。
    pub(in crate::app) fn remember_session(&mut self) {
        // 上次的标签还没有恢复：这时记下的现场是空的，会盖掉要恢复的那一份。
        if self.restoring_tabs() {
            return;
        }
        let places = self.places_now();
        let Some(folder) = self.session_key() else {
            if !places.is_empty() {
                self.update_state(move |state| places::record(state, &places));
            }
            return;
        };

        let mut tabs = Vec::new();
        let mut active = None;
        let mut slots = Vec::new();
        for (i, doc) in self.editor.documents.iter().enumerate() {
            let Some(tab) = self.session_tab(doc) else {
                continue;
            };
            if self.editor.active == Some(i) {
                active = Some(tabs.len());
            }
            slots.push((doc.id, tabs.len()));
            tabs.push(tab);
        }
        let panes = self.session_panes(&slots);

        let terminal = self.terminal_session();
        let run = self.run.selected.clone();
        let mut folders = self.extra_folders();
        let workspace = self.editor.workspace.as_ref();
        let mut name = workspace.and_then(|workspace| workspace.alias().map(str::to_owned));
        let mut project = workspace.and_then(|ws| ws.project().map(Path::to_path_buf));
        // 单独打开着一组中的打开的目录：这一组保持原样。
        if self.disk.solo
            && folders.is_empty()
            && let Some(session) = self.state.session_for(&folder)
        {
            folders = session.folders.clone();
            name = session.name.clone();
            project = session.project.clone();
        }
        let sidebar_hidden = !self.panels.sidebar.visible;
        let locked = (self.editor.documents.iter()).filter(|doc| doc.is_locked());
        let readonly = locked.filter_map(|doc| doc.path.clone()).collect();
        let readonly_all = self.editor.all_locked();
        let bookmarks = self
            .editor
            .all_bookmarks()
            .into_iter()
            .map(|(path, marks)| (path, marks.lines().to_vec()))
            .collect();
        let outline = self.outline_session(&folder);
        let folds = self.editor.all_folds().into_iter().collect();
        let earlier = self.state.session_for(&folder).and_then(|s| s.window);
        let window = self.window_frame_now().or(earlier);
        self.update_state(move |state| {
            places::record(state, &places);
            state.set_session(Session {
                folder,
                tabs,
                active,
                bookmarks,
                outline,
                folds,
                terminal,
                run,
                folders,
                name,
                project,
                readonly,
                readonly_all,
                sidebar_hidden,
                window,
                panes,
            });
        });
    }

    /// 最近列表中的目录已不存在（删除、移动，或者外接磁盘未挂载）：从列表中移除并提示。
    pub(in crate::app) fn forget_missing_recent(&mut self, folder: &Path) {
        let gone = folder.to_path_buf();
        self.update_state(move |state| state.forget(&gone));
        self.notify(
            NoticeKind::Warning,
            winc_i18n::t!(
                "workspace-recent-missing",
                path = winc_editor::display_path(folder, None)
            ),
        );
    }

    /// 修改状态（最近目录、各目录的现场）并写入状态文件；失败时在标准错误输出原因。
    ///
    /// 内存中的状态立即修改；文件由后台重新读出、在其上做同一个修改再写回（见
    /// [`State::update`]），不会盖掉别的 winc 进程刚写入的内容，写完后内存中的状态换成文件里
    /// 的（[`App::state_stored`]）。测试和测量时没有状态文件，只改内存；没有事件循环时当场写。
    pub(in crate::app) fn update_state(
        &mut self,
        change: impl FnOnce(&mut State) + Clone + Send + 'static,
    ) {
        let Some(path) = self.store.state_path.clone() else {
            change(&mut self.state);
            return;
        };
        let Some(proxy) = self.link.proxy.clone() else {
            let (state, result) = State::update(&path, &self.state, change);
            self.state = state;
            report_state_error(&result);
            self.sync_system_recents();
            return;
        };
        let current = self.state.clone();
        change.clone()(&mut self.state);
        self.sync_system_recents();
        self.store.state_requests += 1;
        let request = self.store.state_requests;
        winc_config::state_writer::update(path, current, change, move |state, result| {
            report_state_error(result);
            let stored = UserEvent::StateStored(request, Box::new(state.clone()));
            let _ = proxy.send_event(stored);
        });
    }

    /// 在后台重新读出状态文件：其他 winc 进程可能修改了最近列表。读不出来时保留内存中的状态。
    pub(in crate::app) fn reload_state(&mut self) {
        let Some(path) = self.store.state_path.clone() else {
            return;
        };
        let Some(proxy) = self.link.proxy.clone() else {
            if let Ok(Some(state)) = State::read(&path) {
                self.state = state;
            }
            return;
        };
        self.store.state_requests += 1;
        let request = self.store.state_requests;
        winc_config::state_writer::reload(path, move |state| {
            let _ = proxy.send_event(UserEvent::StateStored(request, Box::new(state)));
        });
    }

    /// 后台读写完了状态文件：内存中的状态换成文件里的，其中有其他 winc 进程写入的内容。
    ///
    /// 之后又提交过读写时不换：文件里还没有那些修改，等最后一次的结果。
    pub(in crate::app) fn state_stored(&mut self, request: u64, state: State) {
        if request != self.store.state_requests {
            return;
        }
        let recents = |state: &State| (state.recent.clone(), state.recent_files.clone());
        let changed = recents(&state) != recents(&self.state);
        self.state = state;
        if changed {
            self.sync_system_recents();
            // 开始页、菜单和选择面板按序号记着最近列表里的项：列表变了，画面上的还是旧的。
            // 清掉可点击的区域，重绘之前的点击不会落到另一项上。
            self.hits.clear();
            if self.overlays.picker.open && self.overlays.picker.folders {
                self.picker_refresh();
            }
        }
        self.request_redraw();
    }

    /// 把最近打开的目录和文件交给系统：macOS 的最近文档列表（程序坞菜单）、Windows 的任务栏
    /// 跳转列表。没有状态文件（测试、测量）时不做任何事，其他平台没有对应的入口。
    pub(in crate::app) fn sync_system_recents(&self) {
        // 远端主机上的目录和文件不交给系统：系统的最近列表只能打开本机路径。
        #[cfg(any(target_os = "macos", windows))]
        let local = |paths: &[PathBuf]| -> Vec<PathBuf> {
            let local = paths.iter().filter(|path| !winc_backend::is_remote(path));
            local.cloned().collect()
        };
        #[cfg(any(target_os = "macos", windows))]
        let (folders, files) = (local(&self.state.recent), local(&self.state.recent_files));
        #[cfg(target_os = "macos")]
        if self.store.state_path.is_some() {
            let exists = |path: &Path| winc_backend::fs::exists(path);
            let (folders, files) = crate::recents::dock_entries(&folders, &files, exists);
            crate::macos::sync_recent_documents(&folders, &files);
        }
        #[cfg(windows)]
        if let Some(path) = &self.store.state_path {
            crate::windows::sync_jump_list(&folders, &files, path);
        }
    }

    /// 新建窗口（`⇧⌘N`）；带 `folder` 时在新窗口中打开它（已经开在某个窗口里就切到该窗口）。
    pub(in crate::app) fn new_window(&mut self, folder: Option<PathBuf>) {
        self.link
            .shared
            .request(crate::winc::Request::NewWindow(folder));
    }

    /// 在新窗口中打开最近列表中的第 `index` 个目录（项目菜单里按住 `⌥`、开始页上按住 `⌥` 单击）。
    pub(in crate::app) fn open_recent_in_new_window(&mut self, index: usize) {
        let Some(folder) = self.state.recent.get(index).cloned() else {
            return;
        };
        if self.reopen_remote(&folder) {
            return;
        }
        if winc_backend::fs::is_dir(&folder) || super::project::is_project(&folder) {
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
            StartAction::Remote => self.open_remote_picker(),
            StartAction::Language | StartAction::Theme => {
                use crate::app::input::context_menu::{MenuTarget, menu_below};
                // 菜单从被点击的控件下方弹出，右端与控件对齐。
                let Some(&(control, _)) = self.hits.start.iter().find(|(_, a)| *a == action) else {
                    return;
                };
                let target = match action {
                    StartAction::Language => MenuTarget::Locale,
                    _ => MenuTarget::NamedTheme(self.palette.dark),
                };
                let (_, y) = menu_below(control);
                self.open_context_menu(target, (control.right(), y));
            }
            StartAction::Appearance(choice) => self.set_theme_choice(choice),
            StartAction::SetupNext => self.setup_next(),
            StartAction::SetupBack => self.setup_back(),
            StartAction::SetupKeymap(choice) => self.setup_choose(choice),
            StartAction::FinishSetup => self.finish_setup(),
            StartAction::Module(module) => {
                use crate::keymap::Module;
                self.settings_toggle(match module {
                    Module::Git => Setting::GitModule,
                    Module::Terminal => Setting::TerminalModule,
                    Module::Remote => Setting::RemoteModule,
                });
            }
            StartAction::Recent(index) => {
                let Some(folder) = self.state.recent.get(index).cloned() else {
                    return;
                };
                if self.reopen_remote(&folder) {
                    return;
                }
                if winc_backend::fs::is_dir(&folder) {
                    self.open_path(&folder);
                } else if super::project::is_project(&folder) {
                    self.open_folder(&folder);
                } else {
                    self.forget_missing_recent(&folder);
                }
            }
            StartAction::Forget(index) => {
                if let Some(folder) = self.state.recent.get(index).cloned() {
                    self.update_state(move |state| state.forget(&folder));
                }
            }
        }

        self.set_hover(Hover::None);
        self.request_redraw();
    }
}

/// 状态文件没能写入时在标准错误输出原因。
fn report_state_error(result: &std::io::Result<()>) {
    if let Err(e) = result {
        eprintln!("winc: 无法记下最近打开的目录：{e}");
    }
}

/// 后台读好的、恢复标签要用的内容：目录、它的现场和读到的备份。
pub(in crate::app) type PreparedTabs = Arc<Mutex<Vec<(PathBuf, Option<Session>, Prepared)>>>;
