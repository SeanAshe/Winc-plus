//! 工作树（`git worktree`）在界面这边：位置选择器里的工作树，在新窗口或当前窗口打开，分支被
//! 其他工作树签出时切过去，新建、移除、清理失效的，以及附属工作树的窗口标识。
//!
//! 工作树是同一个仓库的另一个目录，一个目录一个窗口（见 `instance.rs`），所以「在新窗口打开」就是
//! 启动 `winc <目录>`，已开着时切到那个窗口。新建、移除都会改动仓库（[`App::git_operate_then`]），
//! 同一时刻只做一件。

use std::path::{Path, PathBuf};

use winc_editor::git::{Branch, Worktree, WorktreeCheck};
use winc_i18n::t;

use super::GitState;
use super::branch::{Then, local_name};
use crate::app::{AlertButton, App, UserEvent};

/// 新建工作树对话框里输入的名字指的是什么。
pub(crate) enum NameTarget {
    /// 还没有这个分支：从当前分支新建。
    New,
    /// 已有的分支（本地的，或者只有远程的同名分支），签出到新工作树。
    Branch(Branch),
    /// 已经在第几个工作树里签出着：切过去。
    Held(usize),
    /// 被目录已经不在的工作树占着：先清理。
    Stale,
    /// 当前分支：不能在另一个工作树里再签出。
    Current,
}

/// 检查完、等着确认移除的工作树。
pub(crate) struct RemoveCheck {
    /// 工作树的目录。
    path: PathBuf,
    /// 签出的分支。
    branch: Option<String>,
    /// 检查结果：没提交的改动数、未跟踪的文件数、游离的提交数，以及分支是否已合并。
    check: WorktreeCheck,
}

impl GitState {
    /// 当前仓库在工作树列表里的下标；只有一个工作树（列表为空）时为 `None`。
    pub fn current_worktree(&self) -> Option<usize> {
        let root = self.root.as_deref()?;
        self.worktrees.iter().position(|w| w.path == root)
    }

    /// `path` 是不是当前仓库的一个工作树。
    pub(in crate::app) fn shares_repository(&self, path: &Path) -> bool {
        self.worktrees.iter().any(|w| w.path == path)
    }

    /// 打开的是附属工作树时的主工作树（列表的第一项）；打开的是主工作树、只有一个工作树时为
    /// `None`。
    pub fn main_worktree(&self) -> Option<&Worktree> {
        match self.current_worktree() {
            Some(1..) => self.worktrees.first(),
            _ => None,
        }
    }

    /// 签出了本地分支 `branch` 的别的工作树（不算当前这个；目录还在的）的下标。
    pub fn worktree_holding(&self, branch: &str) -> Option<usize> {
        self.holders(false)
            .find(|&i| self.worktrees[i].branch() == Some(branch))
    }

    /// 签出了本地分支 `branch`、目录已经不在的工作树的下标：git 仍然不允许签出这个分支，要先
    /// 清理（`git worktree prune`）。
    pub fn stale_holding(&self, branch: &str) -> Option<usize> {
        self.holders(true)
            .find(|&i| self.worktrees[i].branch() == Some(branch))
    }

    /// 被别的工作树签出的本地分支名（选择分支时是切过去，不能删除）。
    pub fn held_branches(&self) -> Vec<&str> {
        let holders = self.holders(false);
        holders.filter_map(|i| self.worktrees[i].branch()).collect()
    }

    /// 被目录已经不在的工作树占着的本地分支名（不能签出、删除）。
    pub fn stale_branches(&self) -> Vec<&str> {
        let holders = self.holders(true);
        holders.filter_map(|i| self.worktrees[i].branch()).collect()
    }

    /// 当前这个以外、目录已经不在（`prunable`）或者还在的工作树的下标。
    fn holders(&self, prunable: bool) -> impl Iterator<Item = usize> + '_ {
        let current = self.current_worktree();
        (0..self.worktrees.len())
            .filter(move |&i| Some(i) != current && self.worktrees[i].prunable == prunable)
    }
}

/// 工作树的名字：目录名；裸仓库的目录去掉 `.git`（`winc.git` → `winc`）。
pub(crate) fn worktree_name(worktree: &Worktree) -> String {
    winc_editor::git::repo_name(&worktree.path)
}

impl App {
    /// 返回传给当前仓库 `git` 的目录参数：仓库位于 WSL 发行版中时使用发行版内的路径。
    pub(in crate::app) fn git_path_arg(&self, path: &Path) -> String {
        let root = self.git.root.as_deref().unwrap_or(path);
        winc_editor::wsl::git_arg(root, path)
    }

    /// 上下文栏上的项目名：打开的目录就是附属工作树时写它所属的仓库，同一个仓库的工作树之间切换时
    /// 不变；工作树的名字写在位置按钮上。打开的目录里有多个仓库时项目就是这个目录。没有打开目录时
    /// 为 `None`。
    pub(crate) fn project_name(&self) -> Option<String> {
        let folder = self.editor.workspace.as_ref()?;
        let main = (self.editor.root() == self.git.root.as_deref())
            .then(|| self.git.main_worktree())
            .flatten();
        Some(main.map_or_else(|| folder.name.clone(), worktree_name))
    }

    /// 打开第 `i` 个工作树：`new_window` 时在新窗口（已经开着就切到那个窗口），否则在当前窗口
    /// 打开；`i` 是当前工作树时什么都不做。
    pub(in crate::app) fn open_worktree(&mut self, i: usize, new_window: bool) {
        if self.git.current_worktree() == Some(i) {
            return;
        }
        let Some(path) = self.git.worktrees.get(i).map(|w| w.path.clone()) else {
            return;
        };
        if new_window {
            self.open_in_new_window(&path);
        } else {
            self.switch_folder(&path);
        }
    }

    /// 在新窗口里打开目录 `path`：已经在某个 winc 窗口里开着就切过去。
    pub(in crate::app) fn open_in_new_window(&mut self, path: &Path) {
        self.new_window(Some(path.to_path_buf()));
    }

    /// 按打开的目录设置窗口标题（程序坞、「窗口」菜单里的名字）：目录名；打开的是附属工作树时
    /// 后面接主工作树的名字（「feat-x — winc」），便于区分同一仓库的各个窗口。
    pub(in crate::app) fn update_window_title(&self) {
        let Some(name) = self.editor.workspace.as_ref().map(|ws| ws.name.as_str()) else {
            return;
        };
        match self.git.main_worktree() {
            Some(main) => self.set_window_title(&format!("{name} — {}", worktree_name(main))),
            None => self.set_window_title(name),
        }
    }

    /// 给分支 `branch` 新建的工作树放在哪儿（[`winc_editor::git::worktree_dir`]）；不在仓库里时
    /// 为 `None`。
    pub(crate) fn new_worktree_dir(&self, branch: &str) -> Option<PathBuf> {
        let main =
            (self.git.worktrees.first().map(|w| w.path.as_path())).or(self.git.root.as_deref())?;
        let folder = self.config.git_worktree_root.as_deref();
        let home = self.git.home.as_deref();
        Some(winc_editor::git::worktree_dir(main, folder, home, branch))
    }

    /// 分支操作菜单、日志菜单里写的新工作树的位置：默认位置写相对于仓库所在目录的
    /// （`winc.worktrees/feat-x`），配置了 `git.worktreeRoot` 时写完整的（主目录写成 `~`）。
    ///
    /// 还没输入名字时最后一级写「…」。
    pub(crate) fn worktree_dir_hint(&self, branch: &str) -> String {
        let Some(dir) = self.new_worktree_dir(if branch.is_empty() { "…" } else { branch })
        else {
            return String::new();
        };
        let main =
            (self.git.worktrees.first().map(|w| w.path.as_path())).or(self.git.root.as_deref());
        let parent = main.and_then(Path::parent);
        match parent {
            Some(parent) if self.config.git_worktree_root.is_none() => {
                winc_editor::display_path(&dir, Some(parent))
            }
            _ => winc_editor::display_path(&dir, None),
        }
    }

    /// 输入条里的名字 `name` 指的是什么：已有的本地分支，只有远程的同名分支（`origin/x` 签出成
    /// `x`），已经在别的工作树里签出的，当前分支，还是要新建的。
    pub(crate) fn worktree_name_target(&self, name: &str) -> NameTarget {
        if let Some(i) = self.git.worktree_holding(name) {
            return NameTarget::Held(i);
        }
        if self.git.stale_holding(name).is_some() {
            return NameTarget::Stale;
        }
        let local = self
            .git
            .branches
            .iter()
            .find(|b| !b.remote && b.name == name);
        // 写的是 `x` 或者 `origin/x`，都找远程分支（`origin/x` 不该新建一个叫这个的本地分支）。
        let remote = || {
            (self.git.branches.iter())
                .find(|b| b.remote && (b.name == name || local_name(b) == name))
        };
        match local.or_else(remote) {
            // 远程分支先找同名的本地分支，所以 `origin/main` 在 `main` 上时也是当前分支。
            Some(branch) if branch.current => NameTarget::Current,
            Some(branch) => NameTarget::Branch(branch.clone()),
            None => NameTarget::New,
        }
    }

    /// 分支操作菜单的「在新工作树中签出」：放在默认位置，按配置复制被忽略的文件，在新窗口打开。
    pub(super) fn new_worktree_for(&mut self, branch: &Branch) {
        let Some(dir) = self.new_worktree_dir(local_name(branch)) else {
            return;
        };
        let globs = self.config.git_worktree_copy.clone();
        self.checkout_in_worktree(branch, dir, globs, true);
    }

    /// 把已有的分支签出到新工作树 `dir`：本地分支直接签出；远程分支签出同名的本地分支，还没有
    /// 时建一个跟踪它的。
    pub(super) fn checkout_in_worktree(
        &mut self,
        branch: &Branch,
        dir: PathBuf,
        globs: Vec<String>,
        new_window: bool,
    ) {
        let local = local_name(branch).to_owned();
        let exists =
            !branch.remote || (self.git.branches.iter()).any(|b| !b.remote && b.name == local);
        let remote = branch.name.clone();
        let target = self.git_path_arg(&dir);
        self.add_worktree(
            t!("git-op-checkout-new-worktree", name = local.as_str()),
            dir,
            globs,
            new_window,
            move |git, root| {
                if exists {
                    git.operate(
                        root,
                        &["worktree", "add", "--end-of-options", &target, &local],
                    )
                } else {
                    let args = [
                        "worktree",
                        "add",
                        "--track",
                        "-b",
                        &local,
                        "--end-of-options",
                        &target,
                        &remote,
                    ];
                    git.operate(root, &args)
                }
            },
        );
    }

    /// 日志里提交的「在新工作树中签出」：有能签出的本地分支指着它（[`App::log_worktree_branch`]）
    /// 时签出那个分支，否则游离地签出这个提交，目录用短提交号。
    pub(in crate::app) fn new_worktree_at(&mut self, oid: String, branch: Option<String>) {
        if let Some(branch) = branch.and_then(|b| self.log_worktree_branch(&b)) {
            return self.new_worktree_for(&branch);
        }
        let short = super::log::short(&oid).to_owned();
        let Some(dir) = self.new_worktree_dir(&short) else {
            return;
        };
        let target = self.git_path_arg(&dir);
        let globs = self.config.git_worktree_copy.clone();
        self.add_worktree(
            t!("git-op-checkout-commit-new-worktree", commit = &short),
            dir,
            globs,
            true,
            move |git, root| {
                let args = [
                    "worktree",
                    "add",
                    "--detach",
                    "--end-of-options",
                    &target,
                    &oid,
                ];
                git.operate(root, &args)
            },
        );
    }

    /// 日志里提交上的本地分支 `name` 能签出到新工作树时返回那个分支；不是当前分支、没有被别的
    /// 工作树签出时才可签出，否则返回 `None`。
    pub(in crate::app) fn log_worktree_branch(&self, name: &str) -> Option<Branch> {
        match self.worktree_name_target(name) {
            NameTarget::Branch(branch) if !branch.remote => Some(branch),
            _ => None,
        }
    }

    /// 在后台用 `add` 建好工作树 `dir`，再复制匹配 `globs`（配置 `git.worktreeCopy`）的被忽略的
    /// 文件，做完打开它：`new_window` 时在新窗口，否则当前窗口换过去。
    pub(super) fn add_worktree(
        &mut self,
        what: String,
        dir: PathBuf,
        globs: Vec<String>,
        new_window: bool,
        add: impl FnOnce(&winc_editor::git::Git, &Path) -> Result<String, String> + Send + 'static,
    ) {
        self.git_operate_then(what, move |git, root| {
            add(git, root)?;
            let cancel = winc_editor::format::Cancel::default();
            let summary = match git.copy_ignored(root, &dir, &globs, &cancel) {
                Ok(0) => String::new(),
                Ok(n) => t!("git-worktree-copied-ignored", count = n),
                // 工作树已经建好：仍然打开，只说明哪些文件没有复制成功。
                Err(e) => e,
            };
            Ok((summary, Then::Open(dir, new_window)))
        });
    }

    /// 操作做完以后（[`Then`]）：工作树变了时重新取列表，新建的打开（当前窗口换过去时不用再取，
    /// 换目录会重新读）。
    pub(super) fn worktree_done(&mut self, then: Then) {
        match then {
            Then::Nothing
            | Then::ConfirmRemove(_)
            | Then::ConfirmForcePush(_)
            | Then::ConfirmForceDelete(_)
            | Then::RebasePlan(_)
            | Then::Warn(_) => return,
            Then::Refresh => {}
            Then::Open(path, true) => self.open_in_new_window(&path),
            Then::Open(path, false) => return self.switch_folder(&path),
            Then::Rescan => return self.git_folder_changed(),
        }
        self.git_refresh();
    }

    /// 位置选择器的「清理」：目录已经不在的，去掉 git 里的记录。
    pub(in crate::app) fn prune_worktrees(&mut self) {
        self.git_operate_then(t!("git-op-prune-worktrees").into(), |git, root| {
            git.operate(root, &["worktree", "prune"])
                .map(|summary| (summary, Then::Refresh))
        });
    }

    /// 位置选择器「移除工作树」的第 `i` 个工作树：在另一个窗口里开着时请用户先关闭，否则在后台检查
    /// 有没有没提交的改动、分支合并了没有，再确认（[`App::confirm_remove_worktree`]）。
    pub(in crate::app) fn remove_worktree(&mut self, i: usize) {
        let Some(worktree) = self.git.worktrees.get(i).cloned() else {
            return;
        };
        if i == 0 || self.git.current_worktree() == Some(i) || worktree.locked {
            return;
        }
        let name = worktree_name(&worktree);
        if self.shared.window_with(&worktree.path, self.key).is_some() {
            self.ask_close_worktree_window(&name, &worktree.path);
            return;
        }
        let what = t!("git-op-check-worktree", name = name.as_str());
        self.git_operate_then(what, move |git, root| {
            let cancel = winc_editor::format::Cancel::default();
            let check = git.check_worktree(root, &worktree, &cancel)?;
            let branch = worktree.branch().map(str::to_owned);
            let path = worktree.path;
            Ok((
                String::new(),
                Then::ConfirmRemove(RemoveCheck {
                    path,
                    branch,
                    check,
                }),
            ))
        });
    }

    /// 要移除的工作树在另一个窗口里开着：请用户先关闭那个窗口，可以直接切过去。
    fn ask_close_worktree_window(&mut self, name: &str, path: &Path) {
        if self.window.handle.is_some() {
            let message = t!("git-worktree-open-elsewhere", name = name);
            let detail = t!("git-worktree-open-elsewhere-detail");
            let buttons = vec![
                AlertButton::new(t!("git-worktree-switch-to-window")),
                AlertButton::cancel(t!("common-cancel")),
            ];
            let path = path.to_path_buf();
            let shown = self.present_alert(&message, detail, buttons, None, move |i, _| {
                (i == 0).then(|| UserEvent::FocusFolder(path.clone()))
            });
            if shown {
                return;
            }
        }
        self.notify(t!("git-worktree-close-window-first", name = name));
    }

    /// 检查完了，确认移除：写明有多少没提交的改动（有改动时没有默认按钮，↩ 不会误删）；分支
    /// 已合并时可以勾选一起删除（默认勾上）。
    ///
    /// 没有窗口时（测试中）直接移除，已合并的分支一起删。
    pub(super) fn confirm_remove_worktree(&mut self, remove: RemoveCheck) {
        let RemoveCheck {
            path,
            branch,
            check,
        } = remove;
        let deletable = branch.filter(|_| check.merged);
        let name = winc_editor::git::repo_name(&path);
        let message = t!("git-worktree-remove-confirm", name = name.as_str());
        if self.window.handle.is_some() {
            let mut detail = t!("git-worktree-remove-confirm-detail").to_owned();
            // 有没提交的改动、游离的提交时没有默认按钮（↩ 不会误删）。
            let dirty = check.changed + check.untracked + check.unreachable > 0;
            if check.unreachable > 0 {
                detail.push_str("\n\n");
                let count = check.unreachable;
                detail.push_str(&t!("git-worktree-remove-unreachable", count = count));
            }
            if check.changed + check.untracked > 0 {
                let mut parts = Vec::new();
                if check.changed > 0 {
                    parts.push(t!("git-worktree-modified-files", count = check.changed));
                }
                if check.untracked > 0 {
                    parts.push(t!("git-worktree-untracked-files", count = check.untracked));
                }
                let files = parts.join(t!("git-worktree-files-separator"));
                detail.push_str("\n\n");
                detail.push_str(&t!("git-worktree-remove-uncommitted", files = files));
            }
            let remove = AlertButton {
                destructive: true,
                ..AlertButton::new(t!("git-worktree-remove-button"))
            };
            let cancel = AlertButton::cancel(t!("common-cancel"));
            // 第一个按钮是默认按钮（↩）；「取消」绑的是 Esc，放在第一个就没有默认按钮。
            let (buttons, removing) = if dirty {
                (vec![cancel, remove], 1)
            } else {
                (vec![remove, cancel], 0)
            };
            let label = deletable
                .as_ref()
                .map(|b| t!("git-worktree-remove-delete-branch", branch = b.as_str()));
            let checkbox = label.as_deref().map(|l| (l, true));
            let shown =
                self.present_alert(&message, &detail, buttons, checkbox, move |i, checked| {
                    let branch = deletable.clone().filter(|_| checked);
                    (i == removing).then(|| UserEvent::GitRemoveWorktree(path.clone(), branch))
                });
            // 窗口上已经挂着别的提示框：不再弹出，改为在状态栏说明。
            if !shown {
                self.flash(t!("git-worktree-close-dialog-first"));
            }
            return;
        }
        // 没有窗口（测试中）时直接移除，已合并的分支与提示框上的默认选择一样一并删除。
        let _ = message;
        self.remove_worktree_now(&path, deletable);
    }

    /// 移除工作树 `path`：目录移到废纸篓（和文件树的删除一样能找回），再在后台清掉 git 里这一个
    /// 工作树的记录（目录已经不在了，`git worktree remove --force` 只删记录；不用 `prune`，它会把
    /// 外接磁盘没插上的别的工作树也一起清掉），`branch` 不为空时随后删除这个分支
    /// （`git branch -d`）。
    pub(in crate::app) fn remove_worktree_now(&mut self, path: &Path, branch: Option<String>) {
        // 确认框弹着的时候列表可能变了：还得是一个能移除的附属工作树。
        let i = self.git.worktrees.iter().position(|w| w.path == path);
        let Some(i) = i.filter(|&i| i > 0 && self.git.current_worktree() != Some(i)) else {
            return;
        };
        if self.branch.running.is_some() {
            self.flash(t!("git-op-busy"));
            return;
        }
        let name = worktree_name(&self.git.worktrees[i]);
        // 确认框弹着的时候在另一个窗口里打开了它。
        if self.shared.window_with(path, self.key).is_some() {
            self.ask_close_worktree_window(&name, path);
            return;
        }
        if let Err(e) = (self.trash)(path) {
            let error = e.to_string();
            self.notify(t!("git-worktree-trash-failed", name = &name, error = error));
            return;
        }
        let target = self.git_path_arg(path);
        let what = t!("git-op-remove-worktree", name = name.as_str());
        self.git_operate_then(what, move |git, root| {
            git.operate(
                root,
                &["worktree", "remove", "--force", "--end-of-options", &target],
            )
            .map_err(|e| t!("git-worktree-remove-record-failed", error = e))?;
            let Some(branch) = branch else {
                return Ok((String::new(), Then::Refresh));
            };
            match git.operate(root, &["branch", "--delete", "--end-of-options", &branch]) {
                Ok(_) => {
                    let summary = t!("git-worktree-branch-deleted", branch = branch.as_str());
                    Ok((summary, Then::Refresh))
                }
                Err(e) => {
                    let b = branch.as_str();
                    let error = t!("git-worktree-branch-delete-failed", branch = b, error = e);
                    Err(error)
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
