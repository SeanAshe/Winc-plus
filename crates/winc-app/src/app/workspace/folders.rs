//! 窗口的附加目录：在打开的目录之外再显示的目录，见 [`winc_editor::Workspace::add_folder`]。
//!
//! 打开的目录决定窗口的环境（终端的起始目录、运行配置、会话）；附加的目录只参与浏览和查找，
//! 随打开的目录的会话保存。

use std::path::{Path, PathBuf};

use winc_i18n::t;

use super::files::Pick;
use crate::app::App;
use crate::app::NoticeKind;
use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuLabel};

impl App {
    /// 是否可以向项目添加目录：已打开目录。
    pub(in crate::app) fn can_add_folder(&self) -> bool {
        self.editor.root().is_some()
    }

    /// 返回文件树菜单中管理附加目录的项；`folder` 是右键点中的目录行，空白处为 `None`。
    pub(in crate::app) fn folder_items(&self, folder: Option<&Path>) -> Vec<MenuEntry> {
        let mut items = Vec::new();
        if self.can_add_folder() {
            items.push(MenuEntry::Item(MenuCommand::AddFolder, true));
        }
        let extra = self.editor.workspace.as_ref().zip(folder);
        if extra.is_some_and(|(ws, folder)| ws.extra_folders().iter().any(|f| f == folder)) {
            items.push(MenuEntry::Item(MenuCommand::RemoveFolder, true));
        }
        if !items.is_empty() {
            items.insert(0, MenuEntry::Separator);
        }
        items
    }

    /// 选择要添加的目录：本机目录用系统面板，远端目录在同一主机上从打开目录的上级开始浏览。
    pub(in crate::app) fn ask_add_folder(&mut self) {
        let Some(root) = self.editor.root() else {
            self.flash(t!("workspace-add-folder-none"));
            return;
        };
        match winc_backend::locate(root) {
            Some(at) => {
                let path = String::from_utf8_lossy(&at.path).into_owned();
                let parent = path.rsplit_once('/').map_or("/", |(up, _)| up);
                let dir = if parent.is_empty() { "/" } else { parent };
                self.browse_remote_to_add(at.target, dir.to_owned());
            }
            None => self.pick(Pick::AddFolder),
        }
    }

    /// 把目录 `path` 加为附加的目录；不能添加时说明原因。
    ///
    /// 单独打开着一组中的打开的目录时，先恢复这一组，再加入新的目录。
    pub(in crate::app) fn add_folder(&mut self, path: &Path) {
        if self.disk.solo {
            self.attach_group();
        }
        let Some(workspace) = &mut self.editor.workspace else {
            return;
        };
        match workspace.add_folder(path) {
            Ok(_) => self.folders_changed(),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                self.notify(NoticeKind::Warning, t!("workspace-add-folder-overlaps"));
            }
            Err(e) => {
                let name = winc_editor::display_name(path);
                self.notify(
                    NoticeKind::Error,
                    t!(
                        "workspace-add-folder-failed",
                        name = name,
                        error = e.to_string()
                    ),
                );
            }
        }
    }

    /// 从窗口中移除附加的目录 `path`。
    pub(in crate::app) fn remove_folder(&mut self, path: &Path) {
        let removed = (self.editor.workspace.as_mut()).is_some_and(|ws| ws.remove_folder(path));
        if !removed {
            return;
        }
        // 其中的仓库不再属于这个窗口；正在看的是其中之一时换到剩下的第一个。
        self.git.repositories.retain(|repo| !repo.starts_with(path));
        if (self.git.root.as_deref()).is_some_and(|root| root.starts_with(path)) {
            let next = self.git.repositories.first().cloned();
            self.git_set_root(next);
        }
        self.folders_changed();
    }

    /// 返回 `path` 所在的目录（打开的目录或附加的目录）；不在其中时返回打开的目录。
    ///
    /// 新建、改名、移动以它为界，不越出所在的目录。
    pub(in crate::app) fn folder_root(&self, path: &Path) -> Option<PathBuf> {
        let workspace = self.editor.workspace.as_ref()?;
        let folder = workspace.folder_of(path).unwrap_or(&workspace.root);
        Some(folder.to_path_buf())
    }

    /// 打开对话框给这组目录起名，输入框中是现在的名字。
    pub(in crate::app) fn ask_rename_group(&mut self) {
        let Some(workspace) = self.editor.workspace.as_ref() else {
            return;
        };
        if workspace.extra_folders().is_empty() {
            return;
        }
        let mut name = crate::app::TextField::default();
        name.set_text(workspace.alias().unwrap_or_default());
        let kind = crate::app::git::form::FormKind::RenameGroup;
        self.show_form_dialog(kind, name, crate::layout::DialogPart::Branch);
    }

    /// 把这组目录的名字改为 `name`；空白时恢复为打开的目录的名字。
    pub(in crate::app) fn rename_group(&mut self, name: &str) {
        if let Some(workspace) = &mut self.editor.workspace {
            workspace.set_alias(Some(name));
            self.folders_changed();
        }
    }

    /// 返回最近列表项 `folder` 的附加目录数和名称；不是项目（既非项目文件，也没有附加目录）时
    /// 返回 `None`。
    pub(crate) fn recent_group(&self, folder: &Path) -> Option<(usize, Option<&str>)> {
        let session = self.state.session_for(folder);
        if super::project::is_project(folder) {
            let count = session.map_or(0, |session| session.folders.len());
            return Some((count, session.and_then(|session| session.name.as_deref())));
        }
        let session = session?;
        (!session.folders.is_empty()).then_some((session.folders.len(), session.name.as_deref()))
    }

    /// 最近列表项 `path` 为项目文件时，名称后的淡色标注；否则为 `None`。仅按扩展名判断，不访问
    /// 磁盘。
    pub(crate) fn recent_tag(&self, path: &Path) -> Option<&'static str> {
        winc_config::project_kind(path).map(|_| t!("workspace-recent-project-tag"))
    }

    /// 返回最近列表项 `folder` 的显示名：项目文件取文件名（不含扩展名），已命名的取名称，有附加
    /// 目录的在目录名后注明个数。
    pub(crate) fn recent_title(&self, folder: &Path) -> String {
        if super::project::is_project(folder)
            && let Some(stem) = folder.file_stem()
        {
            return stem.to_string_lossy().into_owned();
        }
        let name = winc_editor::display_name(folder);
        match self.recent_group(folder) {
            Some((_, Some(alias))) => alias.to_owned(),
            Some((count, None)) => t!("workspace-group-count", name = name, count = count),
            None => name.into_owned(),
        }
    }

    /// 返回附加的目录。
    pub(in crate::app) fn extra_folders(&self) -> Vec<PathBuf> {
        (self.editor.workspace.as_ref()).map_or_else(Vec::new, |ws| ws.extra_folders().to_vec())
    }

    /// 返回文件树中展开了的、指向本机目录的符号链接。
    pub(in crate::app) fn expanded_links(&self) -> Vec<PathBuf> {
        let rows = self.editor.workspace.as_ref().map(|ws| ws.rows());
        let links = rows
            .into_iter()
            .flatten()
            .filter(|row| row.expanded && row.symlink && !winc_backend::is_remote(&row.path));
        links.map(|row| row.path.clone()).collect()
    }

    /// 附加目录或项目名称变化后：重新监视，写回项目文件，记录现场和最近列表，更新窗口标题并重新
    /// 布局。
    pub(in crate::app) fn folders_changed(&mut self) {
        self.watch_extra_folders();
        self.write_back_project();
        self.remember_workspace();
        self.git_discover();
        self.git_scan_repositories();
        self.index_folder();
        self.remember_session();
        self.update_window_title();
        self.relayout();
        self.request_redraw();
    }

    /// 恢复会话 `session` 中记下的附加目录和这一组的名字；已不可用的目录不恢复，并说明是哪些。
    ///
    /// 会话属于某个项目文件、且该文件的第一个目录仍是这个目录时，按项目文件恢复。
    pub(in crate::app) fn restore_group(&mut self, session: Option<&winc_config::Session>) {
        let Some(session) = session else {
            return;
        };
        let project = (session.project.as_deref())
            .and_then(|file| super::project::project_folders(file).ok())
            .filter(|project| project.root == session.folder);
        if let Some(project) = project {
            self.load_project(&project);
            return;
        }
        let mut skipped = Vec::new();
        for folder in &session.folders {
            if let Some(workspace) = &mut self.editor.workspace
                && workspace.add_folder(folder).is_err()
            {
                skipped.push(winc_editor::display_name(folder).into_owned());
            }
        }
        if !skipped.is_empty() {
            let names = skipped.join(t!("common-list-separator"));
            self.notify(
                NoticeKind::Warning,
                t!("workspace-add-folder-skipped", names = names),
            );
        }
        if let Some(workspace) = &mut self.editor.workspace
            && !workspace.extra_folders().is_empty()
        {
            workspace.set_alias(session.name.as_deref());
        }
    }

    /// 单独打开着一组中的打开的目录时，恢复这一组的其他目录。
    pub(in crate::app) fn attach_group(&mut self) {
        if !std::mem::take(&mut self.disk.solo) {
            return;
        }
        let session = (self.editor.root()).and_then(|root| self.state.session_for(root).cloned());
        self.restore_group(session.as_ref());
        self.folders_changed();
    }

    /// 打开最近列表第 `i` 项这一组：正单独打开着其中打开的目录时就地恢复其他目录。
    pub(in crate::app) fn open_group(&mut self, i: usize) {
        let here =
            (self.state.recent.get(i)).is_some_and(|f| Some(f.as_path()) == self.editor.root());
        if here {
            self.attach_group();
        } else {
            self.run_start_action(crate::app::StartAction::Recent(i));
        }
    }

    /// 单独打开最近列表第 `i` 项这一组中的第 `j` 个目录，不带这一组的其他目录。
    pub(in crate::app) fn open_group_folder(&mut self, i: usize, j: usize) {
        let folders = self.group_folders(i);
        let Some(folder) = folders.get(j).cloned() else {
            return;
        };
        if j > 0 {
            self.open_path(&folder);
            return;
        }
        // 这一组正开在这个窗口里：移除其他目录，这一组仍记在会话里。
        if Some(folder.as_path()) == self.editor.root() {
            self.disk.solo = true;
            for extra in self.extra_folders() {
                self.remove_folder(&extra);
            }
            return;
        }
        self.disk.open_alone = Some(folder);
        self.run_start_action(crate::app::StartAction::Recent(i));
    }

    /// 整理项目菜单 `menu` 中与项目有关的项。
    ///
    /// 最近列表中没有项目文件的一组目录展开为子菜单。「打开目录…」后加入「打开项目…」；「关闭」
    /// 前另起一组：添加目录、保存项目，以及重命名项目（仅无项目文件且有附加目录时）。窗口中是
    /// 项目时，「关闭目录」改为「关闭项目」。
    pub(in crate::app) fn add_folder_entries(&self, menu: &mut Vec<MenuEntry>) {
        let mut at = 0;
        while at < menu.len() {
            let MenuEntry::Item(MenuCommand::OpenRecent(i), _) = menu[at] else {
                at += 1;
                continue;
            };
            let folders = self.group_folders(i);
            // 项目文件不展开子菜单。
            if folders.len() > 1 && !super::project::is_project(&folders[0]) {
                let mut items = vec![
                    MenuEntry::Item(MenuCommand::OpenGroup(i), true),
                    MenuEntry::Separator,
                ];
                items.extend(
                    (0..folders.len())
                        .map(|j| MenuEntry::Item(MenuCommand::GroupFolder(i, j), true)),
                );
                let title = self.recent_title(&folders[0]);
                // 子菜单取代这一项和紧随其后的「在新窗口打开」。
                let alternate = matches!(menu.get(at + 1), Some(MenuEntry::Alternate(..)));
                menu.splice(
                    at..at + 1 + usize::from(alternate),
                    [MenuEntry::Submenu(title, items)],
                );
            }
            at += 1;
        }
        // 窗口中是项目（有项目文件或附加目录）时改为「关闭项目」。
        let workspace = self.editor.workspace.as_ref();
        if workspace.is_some_and(|ws| ws.project().is_some() || !ws.extra_folders().is_empty()) {
            for entry in menu.iter_mut() {
                if let MenuEntry::Item(command @ MenuCommand::CloseFolder, _) = entry {
                    *command = MenuCommand::CloseProject;
                }
            }
        }
        let open = MenuEntry::Item(MenuCommand::OpenFolder, true);
        let Some(at) = menu.iter().position(|entry| *entry == open) else {
            return;
        };
        menu.insert(at + 1, MenuEntry::Item(MenuCommand::OpenProject, true));
        // 「关闭」前插入管理当前项目的一组。
        let closes = |entry: &MenuEntry| {
            matches!(
                entry,
                MenuEntry::Item(MenuCommand::CloseFolder | MenuCommand::CloseProject, _)
            )
        };
        let Some(close) = menu.iter().position(closes) else {
            return;
        };
        let mut group = Vec::new();
        if self.can_add_folder() {
            group.push(MenuEntry::Item(MenuCommand::AddFolder, true));
            // 单独打开着一组中的打开的目录：可以就地恢复这一组。
            let current = (self.editor.root())
                .and_then(|root| self.state.recent.iter().position(|folder| folder == root));
            if let Some(i) = current.filter(|_| self.disk.solo) {
                group.push(MenuEntry::Item(MenuCommand::OpenGroup(i), true));
            }
            // 有项目文件时名称取自文件，不提供重命名；没有时为「保存项目…」。
            let project = (self.editor.workspace.as_ref()).is_some_and(|ws| ws.project().is_some());
            let save = if project {
                MenuCommand::SaveProjectAs
            } else {
                MenuCommand::SaveProject
            };
            group.push(MenuEntry::Item(save, true));
            if !self.extra_folders().is_empty() && !project {
                group.push(MenuEntry::Item(MenuCommand::RenameGroup, true));
            }
            group.push(MenuEntry::Separator);
        }
        group.insert(0, MenuEntry::Separator);
        menu.splice(close..close, group);
    }

    /// 返回最近列表第 `i` 项这一组的各个目录：打开的目录在前，其后是上次附加的目录；该项不存在
    /// 时为空。
    pub(in crate::app) fn group_folders(&self, i: usize) -> Vec<PathBuf> {
        let Some(folder) = self.state.recent.get(i) else {
            return Vec::new();
        };
        let extra = (self.state.session_for(folder).into_iter()).flat_map(|s| s.folders.iter());
        std::iter::once(folder).chain(extra).cloned().collect()
    }

    /// 返回最近列表第 `i` 项这一组中第 `j` 个目录的菜单文字：目录名，完整路径在悬停提示中。
    pub(in crate::app) fn group_folder_label(&self, i: usize, j: usize) -> MenuLabel {
        match self.group_folders(i).get(j) {
            Some(folder) => MenuLabel {
                title: winc_editor::display_name(folder).into_owned(),
                subtitle: None,
                tooltip: Some(winc_editor::display_path(folder, None)),
            },
            None => MenuLabel::plain(""),
        }
    }

    /// 返回最近目录第 `i` 项的菜单文字：标题只写目录名，完整路径放在悬停提示中（主目录显示为
    /// `~`）。
    ///
    /// 只有最近列表中另有同名目录时，才在标题下方用一行小字写明所在的上级目录；否则每一项
    /// 都会多出一行相同的上级目录，既占空间又没有信息量。
    pub(in crate::app) fn recent_label(&self, i: usize) -> MenuLabel {
        let Some(folder) = self.state.recent.get(i) else {
            return MenuLabel::plain("");
        };
        let name = winc_editor::display_name(folder);
        let same_name = |other: &PathBuf| winc_editor::display_name(other) == name;
        let ambiguous = self.state.recent.iter().filter(|p| same_name(p)).count() > 1;
        // 菜单文字不能分色，标注写在括号中。
        let title = match self.recent_tag(folder) {
            Some(_) => t!("workspace-recent-project", name = self.recent_title(folder)),
            None => self.recent_title(folder),
        };
        MenuLabel {
            subtitle: folder
                .parent()
                .filter(|_| ambiguous)
                .map(|parent| winc_editor::display_path(parent, None)),
            tooltip: Some(winc_editor::display_path(folder, None)),
            title,
        }
    }

    /// 返回第 `index` 个仓库在菜单中显示的名字：相对打开的目录的路径，目录本身使用目录名；
    /// 附加的目录里的仓库是目录名加其中的相对路径。
    pub(crate) fn git_repository_name(&self, index: usize) -> String {
        let Some(repo) = self.git.repositories.get(index) else {
            return String::new();
        };
        let relative = self
            .editor
            .root()
            .and_then(|root| repo.strip_prefix(root).ok());
        let own = || {
            repo.file_name().map_or_else(
                || repo.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        };
        match relative {
            Some(path) if !path.as_os_str().is_empty() => path.display().to_string(),
            Some(_) => own(),
            // 附加的目录里的仓库：目录名加其中的相对路径。
            None => {
                let folder = (self.editor.workspace.as_ref())
                    .and_then(|ws| ws.folder_of(repo))
                    .filter(|folder| *folder != repo.as_path());
                match folder.and_then(|folder| Some((folder, repo.strip_prefix(folder).ok()?))) {
                    Some((folder, inner)) => {
                        format!("{}/{}", winc_editor::display_name(folder), inner.display())
                    }
                    None => own(),
                }
            }
        }
    }

    /// 处理拖进窗口的 `path`：可以加为附加目录的目录按设置询问、添加或按落点处理，其余的按落点
    /// 处理（见 [`App::drop_plain`]）。`into` 是文件树上的目标目录，落在编辑区时为 `None`。
    ///
    /// 询问推迟到拖放结束之后（[`UserEvent::FolderDropAsk`]）：系统的拖放回调中不弹出对话框。
    pub(in crate::app) fn drop_folder(&mut self, path: &Path, into: Option<PathBuf>) {
        use winc_config::FolderDrop;

        let addable = self.can_add_folder()
            && (self.editor.workspace.as_ref()).is_some_and(|ws| ws.can_add_folder(path));
        let choice = self.config.drop_folder.unwrap_or_default();
        match (addable, choice, self.link.proxy.clone()) {
            (true, FolderDrop::Add, _) => self.add_folder(path),
            // 没有窗口（测试中）时不询问，按落点处理。
            (true, FolderDrop::Ask, Some(proxy)) if self.window.handle.is_some() => {
                let ask = crate::app::UserEvent::FolderDropAsk(path.to_path_buf(), into);
                let _ = proxy.send_event(ask);
            }
            _ => self.drop_plain(path, into),
        }
    }

    /// 询问拖入的目录 `path` 是添加到窗口，还是按落点处理（打开，或复制进 `into`）。
    pub(in crate::app) fn ask_folder_drop(&mut self, path: &Path, into: Option<PathBuf>) {
        let name = winc_editor::display_name(path);
        let (message, detail, other) = match &into {
            Some(dir) => {
                let dir = winc_editor::display_name(dir);
                (
                    t!("workspace-drop-folder-ask-copy", name = name, folder = dir),
                    t!("workspace-drop-folder-copy-detail"),
                    t!("workspace-drop-folder-copy"),
                )
            }
            None => (
                t!("workspace-drop-folder-ask", name = name),
                t!("workspace-drop-folder-ask-detail"),
                t!("workspace-drop-folder-open"),
            ),
        };
        let buttons = vec![
            crate::app::AlertButton::new(t!("workspace-drop-folder-add")),
            crate::app::AlertButton::new(other),
            crate::app::AlertButton::cancel(t!("common-cancel")),
        ];
        let remember = Some((t!("common-dont-ask"), false));
        let target = path.to_path_buf();
        self.present_alert(&message, detail, buttons, remember, move |i, kept| {
            let (path, into) = (target.clone(), into.clone());
            (i < 2).then(|| crate::app::UserEvent::FolderDropped(path, into, i == 0, kept))
        });
    }

    /// 按询问中的选择处理拖入的目录 `path`；`remember` 时把该方式写入设置。
    pub(in crate::app) fn folder_drop_chosen(
        &mut self,
        path: &Path,
        into: Option<PathBuf>,
        add: bool,
        remember: bool,
    ) {
        use winc_config::FolderDrop;

        if remember {
            self.set_drop_folder(if add {
                FolderDrop::Add
            } else {
                FolderDrop::Open
            });
        }
        if add {
            self.add_folder(path);
        } else {
            self.drop_plain(path, into);
        }
    }

    /// 设置把目录拖进已打开目录的窗口时怎样处理。
    pub(in crate::app) fn set_drop_folder(&mut self, choice: winc_config::FolderDrop) {
        self.config.drop_folder = Some(choice);
        self.save_config();
        self.request_redraw();
    }
}
