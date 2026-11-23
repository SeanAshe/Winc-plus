//! 项目文件（`.winc-project`，见 [`winc_config::read_project`]）：打开、另存为，以及项目目录
//! 变化后写回。
//!
//! 第一个目录作为打开的目录，其余作为附加目录。从窗口外打开项目文件时打开项目，在窗口内打开时
//! 作为文本编辑。现场和最近列表以项目文件为键（见 [`App::session_key`]）。
//!
//! 其他编辑器的项目文件（见 [`winc_config::project_kind`]）只读：打开后按未保存的项目处理，以
//! 文件名为项目名，不写回原文件。

use std::path::{Path, PathBuf};

use winc_config::{PROJECT_EXTENSION, ProjectError, read_project, write_project};
use winc_i18n::t;

use super::files::Pick;
use crate::app::lifecycle::notice::NoticeAction;
use crate::app::{App, NoticeKind};

/// 从项目文件读出的一组目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectFolders {
    /// 项目文件。
    pub file: PathBuf,
    /// 是否为 Winc 的项目文件；只有这种文件会被写回。
    pub native: bool,
    /// 打开的目录：项目文件中的第一个目录。
    pub root: PathBuf,
    /// 附加的目录，按文件中的顺序，不含打开的目录。
    pub extra: Vec<PathBuf>,
}

/// 读取项目文件 `file` 中的目录，并尽量规范化路径。
///
/// # Errors
///
/// 同 [`read_project`]。
pub(crate) fn project_folders(file: &Path) -> Result<ProjectFolders, ProjectError> {
    let canonical = |path: PathBuf| winc_config::files::canonicalize(&path).unwrap_or(path);
    let mut folders = read_project(file)?.into_iter().map(|f| canonical(f.path));
    let root = folders.next().ok_or(ProjectError::Invalid)?;
    let mut extra: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if folder != root && !extra.contains(&folder) {
            extra.push(folder);
        }
    }
    Ok(ProjectFolders {
        native: winc_config::is_project_file(file),
        file: canonical(file.to_path_buf()),
        root,
        extra,
    })
}

/// `path` 是否为项目文件（不是同名的目录）。
pub(crate) fn is_project(path: &Path) -> bool {
    winc_config::project_kind(path).is_some() && !winc_backend::fs::is_dir(path)
}

impl App {
    /// 窗口现场和最近列表的键：有项目文件时为项目文件，否则为打开的目录。
    pub(crate) fn session_key(&self) -> Option<PathBuf> {
        let workspace = self.editor.workspace.as_ref()?;
        let project = workspace.project().map(Path::to_path_buf);
        project.or_else(|| self.editor.root().map(Path::to_path_buf))
    }

    /// 窗口的现场（见 [`Self::session_key`]）；还没有记下时为 `None`。
    pub(crate) fn current_session(&self) -> Option<&winc_config::Session> {
        self.state.session_for(&self.session_key()?)
    }

    /// 当前标签是其他项目的项目文件时常驻提示可打开该项目；切换标签时收起，切回时再次提示。
    /// 用户关闭提示后，切换标签前不再提示。
    pub(in crate::app) fn sync_project_offer(&mut self) {
        let current = self.editor.workspace.as_ref().and_then(|ws| ws.project());
        let wanted = (self.editor.active().and_then(|doc| doc.path.as_deref()))
            .filter(|path| winc_config::project_kind(path).is_some())
            .filter(|path| current != Some(*path))
            .map(Path::to_path_buf);
        if wanted == self.disk.project_offer {
            return;
        }
        self.disk.project_offer.clone_from(&wanted);
        match wanted {
            Some(file) => {
                let name = winc_editor::display_name(&file).into_owned();
                let message = t!("workspace-project-file-opened", name = name);
                self.notify_pinned(message, NoticeAction::OpenProject(file));
            }
            None if matches!(self.notice.action, Some(NoticeAction::OpenProject(_))) => {
                self.dismiss_notice();
            }
            None => {}
        }
    }

    /// 窗口中的项目是否未保存：有不止一个目录，还没有项目文件。
    pub(crate) fn project_unsaved(&self) -> bool {
        let workspace = self.editor.workspace.as_ref();
        workspace.is_some_and(|ws| !ws.extra_folders().is_empty() && ws.project().is_none())
    }

    /// 从窗口外打开 `path`（「打开」面板、拖入编辑区）：项目文件打开项目，其余同
    /// [`Self::open_path`] 并记入最近打开的文件。
    ///
    /// 文件树、快速打开等窗口内的入口将项目文件作为文本打开。
    pub(in crate::app) fn open_from_outside(&mut self, path: &Path) {
        if is_project(path) {
            self.open_folder(path);
            return;
        }
        self.open_path(path);
        self.note_opened_file(path);
    }

    /// 返回打开 `path` 时要打开的目录；`path` 为项目文件时一并返回其目录。读取项目文件失败时
    /// 提示原因并返回 `None`。
    pub(in crate::app) fn folder_target(
        &mut self,
        path: &Path,
    ) -> Option<(PathBuf, Option<ProjectFolders>)> {
        let target = winc_config::files::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !is_project(&target) {
            return Some((target, None));
        }
        match project_folders(&target) {
            Ok(project) => Some((project.root.clone(), Some(project))),
            Err(error) => {
                self.project_failed(&target, &error);
                None
            }
        }
    }

    /// 提示项目文件 `file` 读取失败的原因。
    pub(in crate::app) fn project_failed(&mut self, file: &Path, error: &ProjectError) {
        let name = winc_editor::display_name(file);
        let message = match error {
            ProjectError::Io(e) => t!(
                "workspace-project-read-failed",
                name = name,
                error = e.to_string()
            ),
            ProjectError::Invalid => t!("workspace-project-invalid", name = name),
        };
        self.notify(NoticeKind::Error, message);
    }

    /// 用项目 `project` 中的目录替换附加目录并记录项目文件；不可用的目录跳过并提示。
    ///
    /// 只修改工作区，监视、现场和标题由调用方更新（见 [`Self::apply_project`]）。
    pub(in crate::app) fn load_project(&mut self, project: &ProjectFolders) {
        let Some(workspace) = &mut self.editor.workspace else {
            return;
        };
        let same = project.native
            && workspace.extra_folders() == project.extra.as_slice()
            && workspace.project() == Some(project.file.as_path());
        if same {
            return;
        }
        for extra in workspace.extra_folders().to_vec() {
            workspace.remove_folder(&extra);
        }
        let mut skipped = Vec::new();
        for folder in &project.extra {
            if workspace.add_folder(folder).is_err() {
                skipped.push(winc_editor::display_name(folder).into_owned());
            }
        }
        if project.native {
            workspace.set_project(Some(project.file.clone()));
        } else {
            let stem = project.file.file_stem().map(|stem| stem.to_string_lossy());
            workspace.set_project(None);
            workspace.set_alias(stem.as_deref());
        }
        if !skipped.is_empty() {
            let names = skipped.join(t!("common-list-separator"));
            self.notify(
                NoticeKind::Warning,
                t!("workspace-add-folder-skipped", names = names),
            );
        }
    }

    /// 在已打开项目第一个目录的窗口中就地载入项目 `project`。
    pub(in crate::app) fn apply_project(&mut self, project: &ProjectFolders) {
        self.load_project(project);
        self.folders_changed();
    }

    /// 打开的目录 `root` 是基于目录的 IDEA 项目时，加入在它之外的模块目录（见
    /// [`winc_config::idea_folders`]）；已有附加的目录时不加。
    pub(in crate::app) fn add_idea_modules(&mut self, root: &Path) {
        if winc_backend::is_remote(root) || !self.extra_folders().is_empty() {
            return;
        }
        let modules = winc_config::idea_folders(root);
        let Some(workspace) = &mut self.editor.workspace else {
            return;
        };
        for folder in modules {
            let _ = workspace.add_folder(&folder);
        }
    }

    /// 附加的目录变化后写回所属的项目文件；文件中已是这些目录时不改写。
    pub(in crate::app) fn write_back_project(&mut self) {
        let Some(workspace) = &self.editor.workspace else {
            return;
        };
        let Some(file) = workspace.project().map(Path::to_path_buf) else {
            return;
        };
        let folders: Vec<PathBuf> = workspace.folders().map(Path::to_path_buf).collect();
        if let Err(e) = write_project(&file, &folders) {
            self.project_write_failed(&file, &e);
        }
    }

    /// 打开系统的面板选择项目文件的保存位置（「将项目另存为…」）。
    pub(in crate::app) fn ask_save_project(&mut self) {
        if self.editor.root().is_none() {
            self.flash(t!("workspace-add-folder-none"));
            return;
        }
        self.pick(Pick::SaveProject);
    }

    /// 「将项目另存为…」建议的文件名和起始目录：文件名取打开的目录名，放在该目录中；该目录在远端
    /// 时起始目录由面板决定，项目文件保存在本机。
    pub(in crate::app) fn project_save_place(&self) -> (String, Option<PathBuf>) {
        let Some(root) = self.editor.root() else {
            return (format!(".{PROJECT_EXTENSION}"), None);
        };
        let name = format!("{}.{PROJECT_EXTENSION}", winc_editor::display_name(root));
        let dir = (!winc_backend::is_remote(root)).then(|| root.to_path_buf());
        (name, dir)
    }

    /// 将窗口中的目录写入项目文件 `path`（缺少扩展名时补上），此后由该文件记录项目。
    pub(in crate::app) fn save_project_as(&mut self, path: PathBuf) {
        let file = if winc_config::is_project_file(&path) {
            path
        } else {
            let mut name = path.into_os_string();
            name.push(".");
            name.push(PROJECT_EXTENSION);
            PathBuf::from(name)
        };
        let Some(workspace) = &mut self.editor.workspace else {
            return;
        };
        let folders: Vec<PathBuf> = workspace.folders().map(Path::to_path_buf).collect();
        match write_project(&file, &folders) {
            Ok(_) => {
                let file = winc_config::files::canonicalize(&file).unwrap_or(file);
                let name = winc_editor::display_name(&file).into_owned();
                workspace.set_project(Some(file));
                self.folders_changed();
                self.flash(t!("workspace-project-saved", name = name));
            }
            Err(e) => self.project_write_failed(&file, &e),
        }
    }

    /// 提示项目文件 `file` 写入失败的原因。
    fn project_write_failed(&mut self, file: &Path, error: &std::io::Error) {
        let name = winc_editor::display_name(file);
        let message = if error.kind() == std::io::ErrorKind::InvalidData {
            t!("workspace-project-invalid", name = name)
        } else {
            t!(
                "workspace-project-write-failed",
                name = name,
                error = error.to_string()
            )
        };
        self.notify(NoticeKind::Error, message);
    }
}
