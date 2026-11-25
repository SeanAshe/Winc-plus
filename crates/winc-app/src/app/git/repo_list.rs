//! 仓库列表的来源与用户的改动：自动查找到的仓库、读状态时发现的嵌套仓库、手动添加和从列表中
//! 移除的仓库。
//!
//! 自动查找见 [`App::git_discover`] 和 [`App::git_scan_repositories`]。用户的改动按目录记在
//! 现场中（[`winc_config::SessionRepositories`]），再次打开目录时沿用：移除的仓库不再自动加入，
//! 手动添加的仓库即使查找不到也列出。

use std::path::{Component, Path, PathBuf};
use std::sync::PoisonError;

use winc_editor::git;
use winc_i18n::t;

use super::*;

/// 手动添加仓库的结果：（添加时打开的目录，输入的位置，所在仓库的根目录；不在仓库中时为
/// `None`）。
pub(super) type Added = (PathBuf, PathBuf, Option<PathBuf>);

/// 按字面消去绝对路径 `path` 中的 `.` 和 `..`，不访问文件系统：同一个仓库不因输入的写法不同而
/// 记成两个路径。
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => _ = out.pop(),
            other => out.push(other),
        }
    }
    out
}

impl App {
    /// 仓库 `root` 是否被用户从列表中移除了。
    pub(in crate::app) fn git_repository_removed(&self, root: &Path) -> bool {
        self.git.edits.removed.iter().any(|r| r == root)
    }

    /// 把 `found` 中尚未列出、未被移除的仓库加在列表末尾，返回是否加入了仓库。
    ///
    /// 菜单与选择器按下标引用仓库，已有仓库的下标不变。
    pub(in crate::app) fn git_adopt_repositories(
        &mut self,
        found: impl IntoIterator<Item = PathBuf>,
    ) -> bool {
        let known = self.git.repositories.len();
        for repo in found {
            if !self.git.repositories.contains(&repo) && !self.git_repository_removed(&repo) {
                self.git.repositories.push(repo);
            }
        }
        self.git.repositories.len() > known
    }

    /// 读到的各仓库状态里有嵌套的仓库（见 [`git::nested_repositories`]）时加入列表；状态没有
    /// 变化时不做处理。
    pub(in crate::app) fn adopt_nested_repositories(&mut self) {
        let seen = (self.git.generation, self.git.summaries_version());
        if self.git.nested_seen == seen {
            return;
        }
        self.git.nested_seen = seen;
        let mut nested: Vec<PathBuf> = self
            .git
            .status
            .iter()
            .flat_map(|s| s.nested.clone())
            .collect();
        {
            let summaries = self.git.summaries.lock();
            let summaries = summaries.unwrap_or_else(PoisonError::into_inner);
            // 只看列表中的仓库：移除的仓库留下的状态不再带入其中嵌套的仓库。
            let listed = self
                .git
                .repositories
                .iter()
                .filter_map(|r| summaries.get(r));
            nested.extend(listed.flat_map(|s| s.nested.iter().cloned()));
        }
        if self.git_adopt_repositories(nested) {
            self.git_read_summaries();
            self.location_picker_refreshed();
            self.request_redraw();
        }
    }

    /// 读出窗口现场中用户对仓库列表的改动；换目录时调用。
    pub(super) fn load_repository_edits(&mut self) {
        let edits = self.current_session().map(|s| s.repositories.clone());
        self.git.edits = edits.unwrap_or_default();
    }

    /// 调整仓库列表的记录：`change` 作用于本窗口的记录和状态文件中该目录的现场，随即保存。
    ///
    /// 打开同一目录的其他窗口可能已调整过，因此在状态文件中的记录上修改，不整体覆盖。
    fn change_repository_edits(
        &mut self,
        change: impl Fn(&mut winc_config::SessionRepositories) + Clone + Send + 'static,
    ) {
        change(&mut self.git.edits);
        let Some(key) = self.session_key() else {
            return;
        };
        self.update_state(
            move |state| match state.sessions.iter_mut().find(|s| s.folder == key) {
                Some(session) => change(&mut session.repositories),
                None => {
                    let mut repositories = winc_config::SessionRepositories::default();
                    change(&mut repositories);
                    state.set_session(winc_config::Session {
                        folder: key,
                        repositories,
                        ..winc_config::Session::default()
                    });
                }
            },
        );
    }

    /// 打开「添加仓库」对话框：输入仓库中任意一个目录的路径，绝对路径或相对于打开的目录。
    pub(in crate::app) fn open_add_repository_dialog(&mut self) {
        if !self.git_enabled() || self.editor.root().is_none() {
            self.flash(t!("git-repository-add-needs-folder"));
            return;
        }
        self.show_form_dialog(
            super::form::FormKind::AddRepository,
            crate::field::TextField::default(),
            crate::layout::DialogPart::Branch,
        );
    }

    /// 把「添加仓库」对话框中输入的 `text` 换成路径：绝对路径照用，其余相对于打开的目录；远端的
    /// 目录按那台主机上的路径处理。没有打开目录时返回 `None`。
    pub(in crate::app) fn repository_input_path(&self, text: &str) -> Option<PathBuf> {
        let root = self.editor.root()?;
        let text = text.trim();
        if let Some(at) = winc_backend::locate(root) {
            let path = match text.starts_with('/') {
                true => winc_backend::virtual_path(&at.target, text.as_bytes()),
                false => winc_backend::join(root, text),
            };
            // 解析后再写回：消去其中的 `..`，在远端的根目录处不再上移。
            let at = winc_backend::locate(&path)?;
            return Some(winc_backend::virtual_path(&at.target, &at.path));
        }
        let path = Path::new(text);
        Some(lexical(&match path.is_absolute() {
            true => path.to_path_buf(),
            false => root.join(path),
        }))
    }

    /// 在后台查找 `path` 所在的仓库，结果由 [`App::git_take_added`] 加入列表。
    pub(in crate::app) fn add_repository_at(&mut self, path: PathBuf) {
        let Some(folder) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let slot = std::sync::Arc::clone(&self.git.added);
        self.spawn_blocking_then(UserEvent::GitStatus, move || {
            let root = git::find_root(&path);
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((folder, path, root));
        });
    }

    /// 收下手动添加仓库的结果：加入列表并记下，切换到该仓库；不在仓库中时说明原因。
    pub(in crate::app) fn git_take_added(&mut self) {
        let taken = self
            .git
            .added
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // 查找期间换了目录：结果属于原来的目录，不再加入。
        let taken = taken.filter(|(folder, ..)| self.editor.root() == Some(folder.as_path()));
        let Some((_, path, root)) = taken else {
            return;
        };
        let Some(root) = root else {
            let name = winc_editor::display_path(&path, self.editor.root());
            self.notify(
                NoticeKind::Error,
                t!("git-repository-add-not-found", path = name),
            );
            return;
        };
        let repo = root.clone();
        self.change_repository_edits(move |edits| {
            edits.removed.retain(|r| *r != repo);
            if !edits.added.contains(&repo) {
                edits.added.push(repo.clone());
            }
        });
        if self.git_adopt_repositories([root.clone()]) {
            self.git_read_summaries();
        }
        self.git_set_root(Some(root.clone()));
        self.location_picker_refreshed();
        let name = winc_editor::display_name(&root).into_owned();
        self.flash(t!("git-repository-added", name = name));
        self.request_redraw();
    }

    /// 把仓库 `root` 从列表中移除并记下：之后查找、打开其中的文件时不再加入，直到再次手动添加。
    /// 移除的是当前仓库时换到列表中的第一个仓库。
    pub(in crate::app) fn remove_repository(&mut self, root: &Path) {
        let repo = root.to_path_buf();
        self.change_repository_edits(move |edits| {
            edits.added.retain(|r| *r != repo);
            if !edits.removed.contains(&repo) {
                edits.removed.push(repo.clone());
            }
        });
        self.git.repositories.retain(|r| r != root);
        if self.git.root.as_deref() == Some(root) {
            let next = self.git.repositories.first().cloned();
            self.git_set_root(next);
        }
        self.location_picker_refreshed();
        let name = winc_editor::display_name(root).into_owned();
        self.flash(t!("git-repository-removed", name = name));
        self.request_redraw();
    }
}
