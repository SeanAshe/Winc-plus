//! 上一次提交：读取、修改（amend）的开关、菜单，以及撤销。

use super::*;

impl App {
    /// 切换「修改上一次提交」：打开时把上一次提交的信息填进来，关闭时恢复原来写的。
    pub(in crate::app::git) fn toggle_amend(&mut self) {
        self.git.commit.amend = !self.git.commit.amend;
        if !self.git.commit.amend {
            let draft = self.git.commit.draft.take().unwrap_or_default();
            self.git.commit.message.set_text(&draft);
            // 没有改动时关掉以后提交卡片也收起。
            self.settle_commit_focus();
            return;
        }
        self.git.commit.draft = Some(self.git.commit.message.text().to_owned());
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let message = match git {
                Ok(git) => git.last_message(root),
                Err(_) => String::new(),
            };
            let _ = sender.send(Outcome::LastMessage(message));
        });
    }

    /// `HEAD` 变了（或还没取过）时在后台取它指向的提交，「上一次提交」那一行用。
    pub(in crate::app::git) fn fetch_last_commit(&mut self) {
        let oid = self.git.status.as_ref().and_then(|s| s.oid.clone());
        if self.git.commit.last_for.as_ref() == Some(&oid) {
            return;
        }
        self.git.commit.last_for = Some(oid.clone());
        if self.git.commit.last.as_ref().map(|l| &l.oid) != oid.as_ref() {
            self.git.commit.last = None;
        }
        if oid.is_none() {
            return;
        }
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let last = git.ok().and_then(|git| git.last_commit(root));
            let _ = sender.send(Outcome::Last(last, root.to_path_buf()));
        });
    }

    /// 「上一次提交」那一栏的「⋯」菜单：撤销提交、修改上一次提交（打开提交卡片，填上它的信息）、
    /// 复制提交号。
    pub(in crate::app) fn last_commit_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        vec![
            MenuEntry::Item(MenuCommand::CommitUndo, self.commit_undoable().is_some()),
            MenuEntry::Item(MenuCommand::CommitAmend, self.commit_amendable()),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::CommitCopyOid, true),
        ]
    }

    /// 执行「上一次提交」菜单中的命令。
    pub(in crate::app) fn run_last_commit_command(
        &mut self,
        command: crate::app::input::context_menu::MenuCommand,
    ) {
        use crate::app::input::context_menu::MenuCommand;
        let Some(last) = self.commit_undoable().cloned() else {
            return;
        };
        match command {
            MenuCommand::CommitUndo => self.confirm_undo_commit(&last),
            MenuCommand::CommitAmend => {
                self.toggle_amend();
                self.focus = Focus::CommitMessage;
            }
            MenuCommand::CommitCopyOid => {
                let short = last.oid.chars().take(7).collect::<String>();
                self.flash(t!("git-log-copied-oid", commit = short.as_str()));
                self.set_clipboard(last.oid, false);
            }
            _ => {}
        }
    }

    /// 用提示框询问，确认后撤销上一次提交。更改还在，确认按钮不标记为危险操作。
    pub(super) fn confirm_undo_commit(&mut self, last: &LastCommit) {
        let message = t!("git-undo-commit-confirm");
        let detail = t!(
            "git-undo-commit-confirm-detail",
            subject = last.subject.as_str()
        );
        let buttons = vec![
            AlertButton::new(t!("git-undo-commit-confirm-button")),
            AlertButton::cancel(t!("common-cancel")),
        ];
        self.show_alert(message, &detail, buttons, None, |choice, _| {
            (choice == 0).then_some(UserEvent::GitUndoCommit)
        });
    }

    /// 撤销上一次提交：分支退回上一个提交，改动留在暂存区并保持勾选，提交信息填回输入框（输入框里
    /// 已经写了别的时不覆盖）。
    pub(in crate::app) fn undo_last_commit(&mut self) {
        let Some(last) = self.commit_undoable().cloned() else {
            return;
        };
        // 那一行先收起，不会再按一次；失败时重新取。
        self.git.commit.last = None;
        self.git.commit.committed = None;
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let result = git.and_then(|git| git.undo_commit(root, &last.oid));
            let _ = sender.send(Outcome::Undone(result, last, root.to_path_buf()));
        });
    }
}
