//! 创建仓库和管理远程仓库：初始化当前目录、克隆、添加和修改远程仓库，以及标签的推送和删除。

use std::path::{Path, PathBuf};

use winc_editor::git::{Remote, TagTarget, repository_dir_name};
use winc_i18n::t;

use super::branch::{TagOp, Then};
use super::form::{FormKind, FormPlan};
use super::log::LogConfirmed;
use crate::app::editor::find::ref_name_field;
use crate::app::{App, TextField};
use crate::layout::DialogPart;

/// 返回当前用户的主目录；相应的环境变量未设置时返回 `None`。
fn home_dir() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name).map(PathBuf::from)
}

/// 目录 `dir` 是否已存在且不为空：`git clone` 只接受不存在或为空的目录。
fn occupied(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

impl App {
    /// 在当前目录中初始化仓库，完成后重新查找仓库。
    pub(in crate::app) fn git_init(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        self.git_operate_in(root, t!("git-op-init").into(), |git, dir| {
            git.init(dir).map(|summary| (summary, Then::Rescan))
        });
    }

    /// 打开克隆仓库的对话框。
    pub(in crate::app) fn open_clone_dialog(&mut self) {
        if self.form_dialog.is_none() {
            let field = TextField::default();
            self.show_form_dialog(FormKind::Clone, field, DialogPart::Branch);
        }
    }

    /// 返回克隆默认存放的目录：当前目录的上一级，没有打开目录时是主目录。
    fn default_clone_parent(&self) -> Option<PathBuf> {
        (self.editor.root().and_then(Path::parent))
            .map(Path::to_path_buf)
            .or_else(home_dir)
    }

    /// 按对话框填写的地址 `url` 和选择的位置 `folder`，说明克隆的结果。
    ///
    /// 能确定目录名时 [`FormPlan::dir`] 是目标目录，否则是存放的位置。
    pub(super) fn clone_plan(&self, url: &str, folder: Option<&Path>) -> FormPlan {
        let parent = (folder.map(Path::to_path_buf)).or_else(|| self.default_clone_parent());
        let name = repository_dir_name(url);
        let dir = (parent.as_ref())
            .zip(name.as_ref())
            .map(|(parent, name)| parent.join(name));
        let problem = if url.is_empty() {
            None
        } else if name.is_none() || url.starts_with('-') {
            Some(t!("git-clone-invalid-url"))
        } else if parent.as_ref().is_none_or(|p| !p.is_dir()) {
            Some(t!("git-clone-no-parent"))
        } else if dir.as_deref().is_some_and(occupied) {
            Some(t!("git-clone-occupied"))
        } else {
            None
        };
        FormPlan {
            enabled: !url.is_empty() && problem.is_none(),
            problem: problem.map(str::to_owned),
            confirm: t!("git-clone-button"),
            base: String::new(),
            base_enabled: false,
            dir: dir.or(parent),
        }
    }

    /// 执行克隆，成功后打开新仓库：已打开目录时在新窗口中打开。
    pub(super) fn submit_clone(&mut self) {
        let Some(dialog) = &self.form_dialog else {
            return;
        };
        let url = dialog.name.text().trim().to_owned();
        let plan = self.clone_plan(&url, dialog.folder.as_deref());
        let Some(dir) = plan.dir else {
            return;
        };
        let Some(parent) = dir.parent().map(Path::to_path_buf) else {
            return;
        };
        // `git` 在 `parent` 中运行，目标只需目录名：`parent` 可能与当前仓库不在同一系统中，
        // 绝对路径的写法取决于 `git` 在哪一侧运行。
        let Some(target) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return;
        };
        let new_window = self.editor.root().is_some();
        let name = winc_editor::display_name(&dir).into_owned();
        self.git_operate_in(
            parent,
            t!("git-op-clone", name = name.as_str()),
            move |git, parent| {
                git.clone_repository(parent, &url, &target)
                    .map(|summary| (summary, Then::Open(dir, new_window)))
            },
        );
    }

    /// 打开添加远程仓库（`remote` 为 `None`）或修改其地址的对话框。
    pub(in crate::app) fn open_remote_dialog(&mut self, remote: Option<Remote>) {
        if self.form_dialog.is_some() {
            return;
        }
        match remote {
            Some(remote) => {
                let mut url = TextField::default();
                url.set_text(&remote.url);
                let kind = FormKind::Remote {
                    editing: Some(remote.name),
                };
                self.show_form_dialog(kind, url, DialogPart::Branch);
            }
            None => {
                let kind = FormKind::Remote { editing: None };
                self.show_form_dialog(kind, ref_name_field(), DialogPart::Branch);
                if let Some(dialog) = &mut self.form_dialog {
                    dialog.second = TextField::default();
                }
            }
        }
    }

    /// 按填写的内容，说明添加（`editing` 为 `None`）或修改远程仓库的结果。
    pub(super) fn remote_plan(&self, editing: Option<&str>, first: &str, second: &str) -> FormPlan {
        let (name, url) = match editing {
            Some(name) => (name, first.trim()),
            None => (first.trim(), second.trim()),
        };
        let exists = editing.is_none() && self.git.remote_list.iter().any(|r| r.name == name);
        let problem = if name.starts_with('-') {
            Some(t!("git-form-name-leading-dash"))
        } else if exists {
            Some(t!("git-remote-name-exists"))
        } else if url.starts_with('-') {
            Some(t!("git-remote-url-invalid"))
        } else {
            None
        };
        FormPlan {
            enabled: !name.is_empty() && !url.is_empty() && problem.is_none(),
            problem: problem.map(str::to_owned),
            confirm: if editing.is_some() {
                t!("git-remote-save-button")
            } else {
                t!("git-remote-add-button")
            },
            base: String::new(),
            base_enabled: false,
            dir: None,
        }
    }

    /// 提交远程仓库的对话框。
    pub(super) fn submit_remote(&mut self, editing: Option<String>) {
        let Some(dialog) = &self.form_dialog else {
            return;
        };
        let first = dialog.name.text().trim().to_owned();
        let second = dialog.second.text().trim().to_owned();
        match editing {
            Some(name) => self.git_operate_then(
                t!("git-op-remote-edit", name = name.as_str()),
                move |git, root| {
                    git.operate(root, &["remote", "set-url", "--", &name, &first])
                        .map(|summary| (summary, Then::Refresh))
                },
            ),
            None => self.git_operate_then(
                t!("git-op-remote-add", name = first.as_str()),
                move |git, root| {
                    git.operate(root, &["remote", "add", "--", &first, &second])
                        .map(|summary| (summary, Then::Refresh))
                },
            ),
        }
    }

    /// 确认后移除远程仓库 `name`。
    pub(in crate::app) fn confirm_remove_remote(&mut self, name: String) {
        self.confirm_log(LogConfirmed::RemoveRemote(name));
    }

    /// 执行已确认的移除远程仓库。
    pub(in crate::app) fn remove_remote(&mut self, name: String) {
        self.git_operate_then(
            t!("git-op-remote-remove", name = name.as_str()),
            move |git, root| {
                git.operate(root, &["remote", "remove", "--", &name])
                    .map(|summary| (summary, Then::Refresh))
            },
        );
    }

    /// 对标签 `name` 执行 `op`；删除前先确认。
    pub(in crate::app) fn run_tag_op(&mut self, op: TagOp, name: String) {
        match op {
            TagOp::Push => self.git_operate(
                t!("git-op-tag-push", name = name.as_str()),
                move |git, root| git.push_tag(root, &name),
            ),
            TagOp::Delete => self.confirm_log(LogConfirmed::DeleteTag(name, TagTarget::Local)),
            TagOp::DeleteRemote => {
                self.confirm_log(LogConfirmed::DeleteTag(name, TagTarget::Remote));
            }
        }
    }

    /// 执行已确认的删除标签。
    pub(in crate::app) fn delete_tag(&mut self, name: String, target: TagTarget) {
        match target {
            TagTarget::Local => self.git_operate(
                t!("git-op-tag-delete", name = name.as_str()),
                move |git, root| git.operate(root, &["tag", "--delete", "--end-of-options", &name]),
            ),
            TagTarget::Remote => self.git_operate(
                t!("git-op-tag-delete-remote", name = name.as_str()),
                move |git, root| git.delete_remote_tag(root, &name),
            ),
        }
    }
}
