//! 定时获取（`git.autoFetch`）：每隔 [`FETCH_PERIOD`] 在后台运行一次 `git fetch --all --prune`，
//! 分支行上与上游的差异随之更新，不用等用户手动获取。
//!
//! 后台获取不经过输入请求（没有 askpass）：需要密码或口令的远程直接失败，不弹出询问，也不提示。
//! 窗口完全看不见、有其他操作在进行时不获取，下一个周期再试。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::{App, UserEvent};

/// 两次获取之间的间隔，与常见编辑器的默认周期相同。
pub(in crate::app) const FETCH_PERIOD: Duration = Duration::from_secs(180);

/// 定时获取的状态。
#[derive(Default)]
pub(crate) struct AutoFetch {
    /// 下一次获取的时刻；尚未安排时为 `None`。
    next: Option<Instant>,
    /// 是否有一次获取正在后台进行。
    running: bool,
    /// 后台的获取结束时置位，界面线程据此重新读取分支（见 [`App::take_auto_fetch`]）。
    done: Arc<AtomicBool>,
}

impl App {
    /// 到时刻就在后台获取一次，返回下一次要醒来的时刻；没有开启定时获取时为 `None`。
    ///
    /// 开启后的第一个周期立即获取一次：打开仓库时分支行上的差异就是新的。
    pub(in crate::app) fn auto_fetch_tick(&mut self, now: Instant) -> Option<Instant> {
        let on = self.config.git_auto_fetch == Some(true) && self.git_enabled();
        let Some(root) = self
            .git
            .root
            .clone()
            .filter(|_| on && !self.git.remotes.is_empty())
        else {
            self.git.auto_fetch.next = None;
            return None;
        };
        let next = *self.git.auto_fetch.next.get_or_insert(now);
        if now < next {
            return Some(next);
        }
        self.git.auto_fetch.next = Some(now + FETCH_PERIOD);
        let busy = self.git.auto_fetch.running || self.git.branch.running.is_some();
        if busy || self.unseen() {
            return self.git.auto_fetch.next;
        }
        self.git.auto_fetch.running = true;
        let done = Arc::clone(&self.git.auto_fetch.done);
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            if let Ok(git) = tool {
                let _ = git.fetch_unattended(&root);
            }
            done.store(true, Ordering::Release);
        });
        self.take_auto_fetch();
        self.git.auto_fetch.next
    }

    /// 后台的获取结束了：重新读取状态和分支，与上游的差异随之更新。
    pub(in crate::app) fn take_auto_fetch(&mut self) {
        if !self.git.auto_fetch.done.swap(false, Ordering::AcqRel) {
            return;
        }
        self.git.auto_fetch.running = false;
        self.git_refresh();
    }
}
