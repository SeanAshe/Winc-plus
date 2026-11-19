//! 日志标签所在的仓库：日志跟着当前仓库换，文件历史留在打开时的仓库。

use super::*;

impl App {
    /// 日志标签 `id` 所在仓库的根目录。
    pub(in crate::app) fn log_root(&self, id: DocumentId) -> Option<PathBuf> {
        self.editor.document(id)?.log.as_deref()?.root.clone()
    }

    /// 当前日志标签所在仓库的根目录。
    pub(crate) fn active_log_root(&self) -> Option<&Path> {
        self.editor.active()?.log.as_deref()?.root.as_deref()
    }

    /// 是否有日志标签在仓库 `root` 里。
    pub(super) fn log_open_in(&self, root: &Path) -> bool {
        let mut views = self
            .editor
            .documents
            .iter()
            .filter_map(|d| d.log.as_deref());
        views.any(|v| v.root.as_deref() == Some(root))
    }

    /// 当前仓库换了：各个日志标签（文件历史除外）换成新仓库的日志，从头读取。新仓库的分支还没
    /// 读到时，等读到以后按当前分支读取（见 [`Self::start_waiting_logs`]）。
    pub(in crate::app) fn retarget_logs(&mut self) {
        let root = self.git.root.clone();
        let ids: Vec<DocumentId> = (self.editor.documents.iter())
            .filter(|d| {
                (d.log.as_deref()).is_some_and(|v| v.query.path.is_none() && v.root != root)
            })
            .map(|d| d.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let wait = root.is_some() && self.git.branches.is_empty();
        let branches = self.current_branches();
        for &id in &ids {
            self.git.log.restarted(id);
            let Some(doc) = self.editor.document_mut(id) else {
                continue;
            };
            set_scroll_line(doc, 0.0);
            let Some(view) = doc.log.as_deref_mut() else {
                continue;
            };
            view.restart(root.clone(), branches.clone());
            if root.is_none() {
                view.complete = true;
                view.error = Some(t!("git-not-a-repository").to_owned());
            } else if !wait {
                self.load_log(id);
            }
        }
        let log = &mut self.git.log;
        log.waiting_branches = wait;
        log.list_waiting = Some(Instant::now() + LOADING_DELAY);
        log.back.clear();
        log.forward.clear();
        log.file = None;
        log.open_first = None;
        log.text_sel = None;
        if self.editor.active().is_some_and(|d| ids.contains(&d.id)) {
            self.git.log.filter.set_text("");
        }
        self.request_redraw();
    }

    /// 新仓库的分支读到了（或读取出错）：等着的日志按当前分支开始读取。
    pub(in crate::app) fn start_waiting_logs(&mut self) {
        if !std::mem::take(&mut self.git.log.waiting_branches) {
            return;
        }
        let root = self.git.root.clone();
        let branches = self.current_branches();
        let ids: Vec<DocumentId> = (self.editor.documents.iter())
            .filter(|d| {
                (d.log.as_deref()).is_some_and(|v| v.query.path.is_none() && v.root == root)
            })
            .map(|d| d.id)
            .collect();
        for id in ids {
            if let Some(view) = self
                .editor
                .document_mut(id)
                .and_then(|d| d.log.as_deref_mut())
            {
                view.query.branches = branches.clone();
            }
            self.reload_log(id);
        }
        self.request_redraw();
    }

    /// 日志 `view` 是否正在读取提交，或在等新仓库的分支。
    pub(crate) fn log_loading(&self, view: &winc_editor::git::log::LogView) -> bool {
        view.is_loading() || (self.git.log.waiting_branches && view.query.path.is_none())
    }

    /// 新打开的日志只看的分支：当前分支；分离 `HEAD` 时为空，即全部分支。
    pub(super) fn current_branches(&self) -> Vec<String> {
        let current = self.git.branches.iter().find(|b| b.current);
        current.map(|b| b.name.clone()).into_iter().collect()
    }

    /// 要在当前日志标签的仓库里改动（签出、拣选、重置等）：它不是当前仓库时先换过去。
    pub(super) fn follow_log_root(&mut self) {
        let root = self.active_log_root().map(Path::to_path_buf);
        if root.is_some() && root != self.git.root {
            self.git_set_root(root);
        }
    }
}

impl LogState {
    /// 日志标签 `id` 换了仓库：结束它打开的 `git log`，不再在其中查找提交、查所在分支。
    fn restarted(&mut self, id: DocumentId) {
        self.streams.remove(&id);
        if self.seek.as_ref().is_some_and(|(doc, _)| *doc == id) {
            self.seek = None;
        }
        if self.branches_due.is_some_and(|(doc, _)| doc == id) {
            self.branches_due = None;
        }
        if self.authors_of == Some(id) {
            self.authors_of = None;
        }
    }
}
