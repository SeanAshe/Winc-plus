//! 提交的菜单，以及执行前要确认的操作。

use winc_editor::git::TagTarget;
use winc_i18n::t;

use super::*;

/// 返回引用 `reference` 的标签名；不是标签时返回 `None`。
pub(in crate::app) fn tag_name(reference: &Ref) -> Option<&str> {
    match reference {
        Ref::Tag(name) => Some(name),
        _ => None,
    }
}

impl App {
    /// 右键点在日志的一个提交上：选中它，返回它的提交号。
    pub(in crate::app) fn log_context_target(&mut self, x: f32, y: f32) -> Option<String> {
        if !self.log_active() {
            return None;
        }
        let (LogTarget::Row(row) | LogTarget::Avatar(row)) =
            crate::layout::hit(&self.hits.log, x, y)?
        else {
            return None;
        };
        self.focus = Focus::Editor;
        self.select_log_row(row);
        let view = self.editor.active()?.log.as_deref()?;
        Some(view.commits.get(row)?.oid.clone())
    }

    /// 当前日志标签里提交号是 `oid` 的提交。
    pub(in crate::app) fn log_commit(&self, oid: &str) -> Option<Commit> {
        let view = self.editor.active()?.log.as_deref()?;
        view.commits.iter().find(|c| c.oid == oid).cloned()
    }

    /// 当前分支名；分离的 `HEAD` 时是「HEAD」。
    pub(in crate::app) fn current_branch_label(&self) -> String {
        self.git
            .branches
            .iter()
            .find(|b| b.current)
            .map_or_else(|| "HEAD".to_owned(), |b| b.name.clone())
    }

    /// 签出提交 `commit` 时签出的本地分支（菜单里写「签出「dev」」用）。
    pub(in crate::app) fn log_checkout_branch(&self, oid: &str) -> Option<String> {
        local_branch(&self.log_commit(oid)?)
    }

    /// 日志里提交 `oid` 的右键菜单选了 `command`。
    ///
    /// 会改仓库的事走分支操作同一套：后台运行、同一时刻只做一件、状态栏说结论、失败右上角说
    /// 原因；做完仓库状态一刷新，日志自己重取。
    pub(in crate::app) fn run_log_commit_command(&mut self, command: MenuCommand, oid: &str) {
        let Some(commit) = self.log_commit(oid) else {
            return;
        };
        let oid = oid.to_owned();
        let tag = short(&oid).to_owned();
        // 合并提交拣选、撤销时按第一个父提交算（`-m 1`）。
        let merge = commit.parents.len() > 1;
        match command {
            MenuCommand::LogCheckout => match local_branch(&commit) {
                Some(branch) => self.git_operate(
                    t!("git-op-checkout", name = branch.as_str()),
                    move |git, root| git.operate(root, &["switch", "--end-of-options", &branch]),
                ),
                None => self.confirm_log(LogConfirmed::Detach(oid)),
            },
            // 从完整的提交号建：仓库大了，前 7 位可能认不出是哪个提交。
            MenuCommand::LogNewBranch => self.open_branch_dialog(None, Some(oid)),
            MenuCommand::LogNewWorktree => self.new_worktree_at(oid, local_branch(&commit)),
            MenuCommand::LogNewTag => self.open_tag_prompt(oid),
            MenuCommand::LogInteractiveRebase => self.start_interactive_rebase(oid),
            MenuCommand::LogTag(op, i) => {
                let tag = commit.refs.iter().filter_map(tag_name).nth(i);
                if let Some(name) = tag {
                    self.run_tag_op(op, name.to_owned());
                }
            }
            MenuCommand::LogCherryPick => self.git_operate(
                t!("git-op-cherry-pick", commit = tag.as_str()),
                move |git, root| {
                    let mut args = vec!["cherry-pick"];
                    if merge {
                        args.extend(["-m", "1"]);
                    }
                    args.extend(["--end-of-options", &oid]);
                    git.operate(root, &args)
                },
            ),
            MenuCommand::LogRevert => self.git_operate(
                t!("git-op-revert", commit = tag.as_str()),
                move |git, root| {
                    let mut args = vec!["revert", "--no-edit"];
                    if merge {
                        args.extend(["-m", "1"]);
                    }
                    args.extend(["--end-of-options", &oid]);
                    git.operate(root, &args)
                },
            ),
            MenuCommand::LogResetSoft | MenuCommand::LogResetMixed => {
                let mode = if command == MenuCommand::LogResetSoft {
                    "--soft"
                } else {
                    "--mixed"
                };
                let branch = self.current_branch_label();
                self.git_operate(
                    t!("git-op-reset", branch = branch, commit = tag.as_str()),
                    // 提交号在 `--` 之前、不会被当成选项（完整的十六进制提交号不以 `-` 开头）；
                    // 较旧的 git 2.39 的 `reset` 不支持 `--end-of-options`。
                    move |git, root| git.operate(root, &["reset", mode, &oid, "--"]),
                );
            }
            MenuCommand::LogResetHard => self.confirm_log(LogConfirmed::ResetHard(oid)),
            MenuCommand::LogCopyOid => {
                self.flash(t!("git-log-copied-oid", commit = tag.as_str()));
                self.set_clipboard(oid, false);
            }
            MenuCommand::LogCopyShortOid => {
                self.flash(t!("git-log-copied-short-oid", commit = tag.as_str()));
                self.set_clipboard(tag, false);
            }
            MenuCommand::LogCopyMessage => {
                let message = self
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref()?.details.clone())
                    .filter(|d| d.oid == oid)
                    .and_then(|d| d.message)
                    .unwrap_or(commit.subject);
                self.flash(t!("git-log-copied-message"));
                self.set_clipboard(message, false);
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// 先请用户确认（[`App::show_confirm`]），确认后执行 `op`；没有窗口时（测试中）直接执行。
    pub(in crate::app) fn confirm_log(&mut self, op: LogConfirmed) {
        let branch = self.current_branch_label();
        let (message, detail, action) = match &op {
            LogConfirmed::Detach(oid) => (
                t!("git-log-detach-confirm", commit = short(oid)),
                t!("git-log-detach-confirm-detail").to_owned(),
                t!("git-log-detach-confirm-button"),
            ),
            LogConfirmed::ResetHard(oid) => {
                let (b, c) = (branch.as_str(), short(oid));
                (
                    t!("git-log-reset-hard-confirm", branch = b, commit = c),
                    t!("git-log-reset-hard-confirm-detail", branch = b),
                    t!("git-log-reset-hard-confirm-button"),
                )
            }
            LogConfirmed::DeleteTag(name, target) => {
                let name = name.as_str();
                match target {
                    TagTarget::Local => (
                        t!("git-tag-delete-confirm", name = name),
                        t!("git-tag-delete-confirm-detail").to_owned(),
                        t!("git-tag-delete-confirm-button"),
                    ),
                    TagTarget::Remote => (
                        t!("git-tag-delete-remote-confirm", name = name),
                        t!("git-tag-delete-remote-confirm-detail").to_owned(),
                        t!("git-tag-delete-confirm-button"),
                    ),
                }
            }
            LogConfirmed::RemoveRemote(name) => (
                t!("git-remote-remove-confirm", name = name.as_str()),
                t!("git-remote-remove-confirm-detail").to_owned(),
                t!("git-remote-remove-confirm-button"),
            ),
            // 删除储藏、分支的确认在分支菜单中进行，不经过这里。
            LogConfirmed::DropStash(_) | LogConfirmed::DeleteBranch(..) => {
                return self.run_log_confirmed(op);
            }
        };
        if self.window.handle.is_some() {
            self.show_confirm(&message, &detail, action, move |ok| {
                ok.then(|| UserEvent::GitLogConfirmed(op.clone()))
            });
            return;
        }
        self.run_log_confirmed(op);
    }

    /// 确认框上确认了 `op`。
    pub(in crate::app) fn run_log_confirmed(&mut self, op: LogConfirmed) {
        match op {
            LogConfirmed::Detach(oid) => {
                let tag = short(&oid).to_owned();
                self.git_operate(
                    t!("git-op-checkout-commit", commit = tag.as_str()),
                    move |git, root| {
                        git.operate(root, &["switch", "--detach", "--end-of-options", &oid])
                    },
                );
            }
            LogConfirmed::ResetHard(oid) => {
                let (tag, branch) = (short(&oid).to_owned(), self.current_branch_label());
                self.git_operate(
                    t!("git-op-reset-hard", branch = branch, commit = tag.as_str()),
                    move |git, root| {
                        // 提交号同样放在 `--` 之前，理由见软重置、混合重置的注释。
                        git.operate(root, &["reset", "--hard", &oid, "--"])
                    },
                );
            }
            LogConfirmed::DropStash(stash) => {
                self.git_operate(
                    t!("git-op-stash-drop", name = stash.name.as_str()),
                    move |git, root| git.stash(root, "drop", &stash.oid),
                );
            }
            LogConfirmed::DeleteBranch(name, force) => self.run_delete_branch(name, force),
            LogConfirmed::DeleteTag(name, target) => self.delete_tag(name, target),
            LogConfirmed::RemoveRemote(name) => self.remove_remote(name),
        }
    }
}
