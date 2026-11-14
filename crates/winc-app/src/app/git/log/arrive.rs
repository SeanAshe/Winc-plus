//! 日志标签接收后台返回的结果：一批提交、详情、所在分支、比较的文件、差异和补丁。

use super::*;

impl App {
    /// 接收后台返回的结果。
    pub(in crate::app) fn poll_log(&mut self) {
        while let Ok(loaded) = self.git.log.results.1.try_recv() {
            match loaded {
                Loaded::GoTo { revision, result } => match result {
                    Ok((oid, listed)) => self.show_log_commit(&oid, !listed),
                    Err(error) => {
                        self.notify(t!("git-go-to-failed", revision = revision, error = error));
                    }
                },
                Loaded::Commits {
                    doc,
                    generation,
                    result,
                    stream,
                } => self.log_commits_arrived(doc, generation, result, stream),
                Loaded::Details {
                    doc,
                    oid,
                    message,
                    files,
                    base,
                } => {
                    self.git.log.fetching.remove(&oid);
                    let Some(view) = self
                        .editor
                        .document_mut(doc)
                        .and_then(|d| d.log.as_deref_mut())
                    else {
                        continue;
                    };
                    let error = message.as_ref().err().or(files.as_ref().err()).cloned();
                    let mut files = files.unwrap_or_default();
                    // 文件历史中查看的文件排在最前，回车即打开该文件；重命名过的文件按文件名匹配。
                    if let Some(path) = &view.query.path {
                        let name = path.file_name();
                        let at = files
                            .iter()
                            .position(|f| &f.path == path)
                            .or_else(|| files.iter().position(|f| f.path.file_name() == name));
                        if let Some(at) = at {
                            let file = files.remove(at);
                            files.insert(0, file);
                        }
                    }
                    // 包含它的分支可能先查到了，记在占位的详情上：换上取回的详情时留着。
                    let branches = (view.details.iter().chain(view.cached(&oid)))
                        .find(|d| d.oid == oid)
                        .and_then(|d| d.branches.clone());
                    let details = Details {
                        oid: oid.clone(),
                        message: message.ok(),
                        files: Some(files),
                        error,
                        branches,
                        base,
                    };
                    // 出错的详情不缓存，下次选中时重新读取。
                    if details.error.is_none() {
                        view.remember(details.clone());
                    }
                    // 预取的详情（不属于选中的提交）只写入缓存。
                    let Some(shown) = view.details.as_mut().filter(|d| d.oid == oid) else {
                        continue;
                    };
                    *shown = details;
                    self.git.log.waiting = None;
                    if self.git.log.open_first.as_deref() == Some(oid.as_str()) {
                        self.git.log.open_first = None;
                        if let Some(&first) = self.log_file_order().first() {
                            self.open_log_file(first);
                        }
                    }
                }
                Loaded::LocalDiff { title, view } => match view {
                    Ok(view) => {
                        self.editor.show_diff(&title, *view);
                        self.focus = Focus::Editor;
                        self.relayout();
                    }
                    Err(e) => self.notify(e),
                },
                Loaded::Patch { oid, result } => match result {
                    Ok(patch) => {
                        self.set_clipboard(patch, false);
                        self.flash(t!("git-log-copied-patch", commit = short(&oid)));
                    }
                    Err(e) => self.notify(t!("git-log-patch-failed", error = e)),
                },
                Loaded::Branches { doc, oid, result } => {
                    self.git.log.branches_fetching.remove(&oid);
                    let view = self.editor.document_mut(doc);
                    let Some(view) = view.and_then(|d| d.log.as_deref_mut()) else {
                        continue;
                    };
                    // 查不出来（超时等）时写「不在任何分支上」会误导：这一行不画，下次选中时再查。
                    match result {
                        Ok(branches) => view.set_branches(&oid, &branches),
                        Err(_) => _ = self.git.log.branches_failed.insert(oid),
                    }
                }
                Loaded::Compare {
                    doc,
                    from,
                    to,
                    files,
                } => {
                    let Some(compare) = self
                        .editor
                        .document_mut(doc)
                        .and_then(|d| d.log.as_deref_mut()?.compare.as_mut())
                        .filter(|c| c.from == from && c.to == to)
                    else {
                        continue;
                    };
                    self.git.log.waiting = None;
                    match files {
                        Ok(files) => compare.files = Some(files),
                        Err(e) => {
                            compare.files = Some(Vec::new());
                            compare.error = Some(e);
                        }
                    }
                }
                Loaded::Diff { request, .. } if request != self.git.log.diff_request => {}
                // 等待结果期间更换了目录：结果属于旧仓库，不在此处打开标签。
                Loaded::Diff { source, .. } if self.git.root.as_ref() != Some(&source.root) => {}
                Loaded::Diff {
                    source,
                    file,
                    title,
                    view,
                    ..
                } => match view {
                    Ok(view) => {
                        // 只在新建差异标签时打开侧栏的文件树，之后保留用户对侧栏的选择。
                        let new_tab = (self.git.log.preview)
                            .and_then(|id| self.editor.document(id))
                            .is_none();
                        let index = self
                            .editor
                            .show_diff_in(self.git.log.preview, &title, *view);
                        self.git.log.preview = Some(self.editor.documents[index].id);
                        self.select_log_diff(source, file);
                        if new_tab {
                            self.set_sidebar_view(crate::layout::SidebarView::Commit);
                            self.set_sidebar(true);
                        }
                        self.relayout();
                        self.reveal_log_diff_file();
                        self.clamp_scroll();
                        self.land_diff_file();
                    }
                    Err(e) => {
                        self.diff.land = None;
                        self.flash(e);
                    }
                },
            }
        }
        self.request_redraw();
    }

    fn log_commits_arrived(
        &mut self,
        doc: DocumentId,
        generation: u64,
        result: Result<Vec<Commit>, String>,
        stream: Option<LogStream>,
    ) {
        let Some(view) = self
            .editor
            .document_mut(doc)
            .and_then(|d| d.log.as_deref_mut())
        else {
            return;
        };
        let arrival = view.finish(generation, result);
        // 结果已应用且尚未读取完毕时保留流，用于读取下一批；其余情况（作废、读取完毕、出错）移除。
        if let (Arrival::Applied { .. }, false, Some(stream)) = (arrival, view.complete, stream) {
            self.git.log.streams.insert(doc, (generation, stream));
        }
        let first = match arrival {
            Arrival::Applied { first } => first,
            Arrival::Dropped => return,
            Arrival::Refetch => {
                self.load_log(doc);
                return;
            }
        };
        // 查找逐行追溯点击的提交：已改选其他提交时不再查找；尚未读取到时继续读取下一批，
        // 读取到后选中，读取完仍未找到时按第一批处理（选中 `HEAD`）。
        let seek = match self.git.log.seek.take() {
            Some((id, oid)) if id == doc => {
                Some(oid).filter(|oid| view.details.as_ref().is_some_and(|d| d.oid == *oid))
            }
            other => {
                self.git.log.seek = other;
                None
            }
        };
        if let Some(oid) = &seek {
            if let Some(row) = view.commits.iter().position(|c| c.oid == *oid) {
                if self.editor.active().is_some_and(|d| d.id == doc) {
                    self.select_log_row(row);
                }
                return;
            }
            if !view.complete {
                self.git.log.seek = Some((doc, oid.clone()));
                self.load_log(doc);
                return;
            }
        }
        // 查找落空时（已读取完）仍需选中，此时到达的不一定是第一批。
        if view.selected.is_some() || !(first || seek.is_some()) {
            return;
        }
        // 第一批到达时选中重新读取前选中的提交；不存在时选中 `HEAD`，仍不存在时选中第一个。
        let keep = view.details.as_ref().map(|d| d.oid.clone());
        let at = |pred: &dyn Fn(&Commit) -> bool| view.commits.iter().position(pred);
        let row = keep
            .and_then(|oid| at(&|c| c.oid == oid))
            .or_else(|| {
                at(&|c| {
                    c.refs
                        .iter()
                        .any(|r| matches!(r, Ref::Head(_) | Ref::Detached))
                })
            })
            .or((!view.commits.is_empty()).then_some(0));
        // 保留详情：仍是同一提交时（刷新、`HEAD` 变化）无需重新读取，详情一栏不闪烁。
        let failed = view.error.is_some();
        let active = self.editor.active().is_some_and(|d| d.id == doc);
        if let Some(row) = row
            && active
        {
            self.select_log_row(row);
        }
        if self.git.log.refreshing == Some(doc) {
            self.git.log.refreshing = None;
            if !failed {
                self.flash(t!("git-log-refreshed"));
            }
        }
    }
}
