//! 补查 Git 状态：`.git` 不在可靠的监视范围内时，Git 视图可见期间定时重读工作区状态。

use std::time::Instant;

use super::STATUS_RECHECK;
use crate::app::App;

impl App {
    /// `.git` 不在可靠的监视范围内、Git 视图可见且窗口有焦点时，定时补查本地状态。
    ///
    /// 监视可靠时变化都由通知即时刷新（通知过多时监视改报整体重扫），不定时运行 `git`。补查只
    /// 读取状态，不读取分支、储藏，也不访问远程仓库。
    pub(in crate::app) fn git_status_tick(&mut self, now: Instant) -> Option<Instant> {
        let visible = self.commit_shown()
            || self.git_panel_shown()
            || self
                .editor
                .active()
                .is_some_and(|d| d.log.is_some() || d.diff.is_some());
        let local = self
            .git
            .root
            .as_deref()
            .is_some_and(|root| !winc_backend::is_remote(root));
        let idle = !self.window.focused || self.unseen();
        if !self.git_enabled() || !local || !visible || idle || self.git_watched() {
            self.git.recheck_due = None;
            return None;
        }
        let due = *self.git.recheck_due.get_or_insert(now + STATUS_RECHECK);
        if now >= due {
            self.git.recheck_due = Some(now + STATUS_RECHECK);
            if self.git.running.is_none()
                && self.git.branch.running.is_none()
                && !self.git.commit.staging_active()
            {
                self.git_refresh_status();
            }
        }
        self.git.recheck_due
    }

    /// 当前仓库的 `.git` 是否在可靠的目录监视范围内：仓库根位于监视的目录中，`.git` 是目录。
    ///
    /// 工作树、子模块的 `.git` 是文件，真正的目录在其他位置；`.git` 的位置取自上一次读到的
    /// 状态，不访问文件系统。
    pub(in crate::app) fn git_watched(&self) -> bool {
        let watcher = (self.disk.watcher.as_ref()).filter(|w| w.reliable());
        let git_dir = (self.git.status.as_ref()).and_then(|status| status.git_dir.as_deref());
        watcher
            .zip(self.git.root.as_deref())
            .is_some_and(|(w, root)| {
                root.starts_with(w.root()) && git_dir == Some(root.join(".git").as_path())
            })
    }
}
