//! 比较两个提交，或者一个分支和当前分支。

use super::*;

impl App {
    /// 比较当前日志里第 `a`、`b` 两行的提交：旧的（下面那行）在左，新的在右。
    ///
    /// 固定在最前面的提交（见 `LogQuery::pin`）不在按时间排列的位置上，总是作为旧的一侧：
    /// 它通常是转到的、已不在任何分支上的较早版本。
    pub(super) fn compare_log_rows(&mut self, a: usize, b: usize) {
        let Some(view) = self.editor.active().and_then(|d| d.log.as_deref()) else {
            return;
        };
        let pinned = |row: usize| {
            let oid = view.commits.get(row).map(|c| c.oid.as_str());
            oid.is_some() && oid == view.query.pin.as_deref()
        };
        let (newer, older) = match (pinned(a), pinned(b)) {
            (true, false) => (b, a),
            (false, true) => (a, b),
            _ => (a.min(b), a.max(b)),
        };
        let (Some(to), Some(from)) = (view.commits.get(newer), view.commits.get(older)) else {
            return;
        };
        let (from, to) = (from.oid.clone(), to.oid.clone());
        self.start_log_compare(from, to);
    }

    /// 在当前日志标签里比较 `from`（旧）和 `to`（新），在后台取两者之间改了的文件。
    pub(super) fn start_log_compare(&mut self, from: String, to: String) {
        self.git.log.file = None;
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let id = doc.id;
        let Some(view) = doc.log.as_deref_mut() else {
            return;
        };
        view.compare = Some(Compare {
            from: from.clone(),
            to: to.clone(),
            ..Compare::default()
        });
        view.details_scroll = 0.0;
        self.git.log.waiting = Some(Instant::now() + LOADING_DELAY);
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let files = match tool {
                Ok(git) => git.commit_files(&root, Some(&from), &to, &Cancel::default()),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Loaded::Compare {
                doc: id,
                from,
                to,
                files,
            });
        });
        self.request_redraw();
    }

    /// 退出比较，详情一栏回到选着的提交。
    pub(in crate::app) fn clear_log_compare(&mut self) {
        self.git.log.file = None;
        if let Some(view) = self.editor.active_mut().and_then(|d| d.log.as_deref_mut())
            && view.compare.take().is_some()
        {
            view.details_scroll = 0.0;
            self.request_redraw();
        }
    }

    /// 把分支 `branch` 和当前分支比较（分支菜单「与当前分支比较」）：打开日志，详情一栏是从
    /// 当前分支到 `branch` 改了的文件。
    pub(in crate::app::git) fn git_compare_branch(&mut self, branch: &str) {
        let current = self.current_branch_label();
        self.git_show_log();
        if self.log_active() {
            self.start_log_compare(current, branch.to_owned());
        }
    }
}
