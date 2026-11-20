//! 修正、压缩到历史提交：提交卡片写好 `fixup!`、`squash!` 说明，提交以后变基并入。

use super::*;
use crate::app::git::rewrite::Autosquash;

impl App {
    /// 在提交卡片里准备修正或压缩到 `target`：关掉修改上一次提交，记下原来写的说明，换成
    /// `fixup!`、`squash!` 开头的说明，焦点放进说明框；压缩时另起一段写补充的说明。
    pub(in crate::app::git) fn begin_commit_into(&mut self, target: Autosquash) {
        if self.git.commit.amend {
            self.toggle_amend();
        }
        if self.git.commit.into.is_none() {
            self.git.commit.draft = Some(self.git.commit.message.text().to_owned());
        }
        let message = match target.squash {
            true => format!("{}\n\n", target.subject),
            false => target.subject.clone(),
        };
        self.git.commit.message.set_text(&message);
        self.git.commit.message.set_caret(message.len(), false);
        self.git.commit.into = Some(target);
        self.show_commit();
        self.focus = Focus::CommitMessage;
        self.request_redraw();
    }

    /// 不再修正、压缩到历史提交：恢复原来写的说明。
    pub(in crate::app::git) fn cancel_commit_into(&mut self) {
        if self.git.commit.into.take().is_none() {
            return;
        }
        let draft = self.git.commit.draft.take().unwrap_or_default();
        self.git.commit.message.set_text(&draft);
        self.settle_commit_focus();
        self.request_redraw();
    }

    /// 提交成功以后：说明仍以 `fixup!`、`squash!` 开头时变基并入目标。
    pub(in crate::app::git) fn finish_commit_into(&mut self, message: &str) {
        let Some(target) = self.git.commit.into.take() else {
            return;
        };
        self.git.commit.draft = None;
        let word = target.subject.split(' ').next().unwrap_or_default();
        if message.starts_with(word) {
            self.run_autosquash(target);
        }
    }
}
