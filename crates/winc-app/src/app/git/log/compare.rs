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
        let Some(root) = self.active_log_root().map(Path::to_path_buf) else {
            return;
        };
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let id = doc.id;
        let Some(view) = doc.log.as_deref_mut() else {
            return;
        };
        view.compare_request += 1;
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

    /// 打开「与指定版本比较」对话框：输入要与提交 `oid` 比较的提交号、分支名或标签名。
    pub(super) fn open_compare_revision(&mut self, oid: String) {
        use crate::app::git::form::FormKind;
        self.show_form_dialog(
            FormKind::CompareRevision { commit: oid },
            TextField::default(),
            crate::layout::DialogPart::Branch,
        );
    }

    /// 在当前日志标签里比较提交 `commit` 与输入的 `revision`（提交号、分支名或标签名）。
    ///
    /// 在后台把 `revision` 解析成提交号并取回改了的文件，成功后才进入比较，文件列表和之后打开
    /// 的差异因此指同一个提交；无法解析时提示原因，日志保持原样。`revision` 是 `commit` 的祖先
    /// 时放在旧的一侧，否则 `commit` 在旧的一侧。
    pub(in crate::app::git) fn compare_log_with_revision(
        &mut self,
        commit: String,
        revision: String,
    ) {
        let Some(root) = self.active_log_root().map(Path::to_path_buf) else {
            return;
        };
        let Some((id, request)) = self.editor.active().and_then(|doc| {
            let view = doc.log.as_deref()?;
            Some((doc.id, view.compare_request))
        }) else {
            return;
        };
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let result = tool.map_err(|e| unusable(&e)).and_then(|git| {
                let cancel = Cancel::default();
                let oid = git.resolve_revision(&root, &revision, &cancel)?;
                // 输入的不是提交号（或其缩写）时为分支名或标签名，显示时保留。
                let typed = revision.to_ascii_lowercase();
                let name = (!oid.starts_with(&typed)).then(|| revision.clone());
                let older = oid != commit && git.is_ancestor(&root, &oid, &commit, &cancel);
                let (from, to, from_name, to_name) = match older {
                    true => (oid, commit, name, None),
                    false => (commit, oid, None, name),
                };
                let files = git.commit_files(&root, Some(&from), &to, &cancel)?;
                Ok(Compare {
                    from,
                    to,
                    from_name,
                    to_name,
                    files: Some(files),
                    error: None,
                })
            });
            let _ = sender.send(Loaded::CompareRevision {
                doc: id,
                request,
                revision,
                result,
            });
        });
    }

    /// 退出比较，详情一栏回到选着的提交。
    pub(in crate::app) fn clear_log_compare(&mut self) {
        self.git.log.file = None;
        if let Some(view) = self.editor.active_mut().and_then(|d| d.log.as_deref_mut()) {
            // 尚未返回的「与指定版本比较」随之作废。
            view.compare_request += 1;
            if view.compare.take().is_none() {
                return;
            }
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
