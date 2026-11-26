//! 合并差异标签显示一个提交的更改：各文件依次排在一个标签里，比较第一个父提交与该提交，只读。
//!
//! 与「未提交的更改」共用预览标签和全部绘制、折叠、查找：改动的文件在后台读取，各段按需比较，
//! 不随工作区和 Git 状态的变化刷新。

use winc_editor::git::CommitFile;

use super::*;

/// 合并差异标签显示的内容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Source {
    /// 提交视图列出的未提交的更改。
    #[default]
    Changes,
    /// 一个提交的更改。
    Commit(CommitSource),
}

/// 合并差异标签显示的提交。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitSource {
    /// 提交所在的仓库：之后换了仓库，该标签仍在此仓库中读取和比较。
    pub(super) root: PathBuf,
    /// 提交号。
    pub(crate) oid: String,
    /// 比较的父提交；根提交为 `None`。
    parent: Option<String>,
    /// 标签的标题：短提交号和说明的首行。
    title: String,
    /// 是否正在读取改动的文件。
    pub(crate) loading: bool,
}

/// 后台交回合并差异标签的结果。
pub(super) enum Arrival {
    /// 比较完的一段。
    Compared(Compared),
    /// 读到了提交（提交号）改动的文件，或者失败的原因。
    Files(String, Result<Vec<CommitFile>, String>),
    /// 准备好了要在单独的差异标签中打开的一个文件（标签的标题），或者失败的原因。
    Opened(String, Result<DiffView, String>),
}

impl Source {
    /// 显示的是提交时返回该提交。
    pub(crate) fn commit(&self) -> Option<&CommitSource> {
        match self {
            Source::Commit(commit) => Some(commit),
            Source::Changes => None,
        }
    }
}

impl App {
    /// 合并差异标签正在显示的提交；没有打开、或者显示的是未提交的更改时为 `None`。
    pub(crate) fn combined_commit(&self) -> Option<&CommitSource> {
        self.combined_index()?;
        self.combined_tab()?.source.commit()
    }

    /// 在预览标签中依次显示提交 `oid`（说明的首行为 `subject`）相对第一个父提交 `parent` 的
    /// 全部更改；已经在显示该提交时切换过去。
    pub(in crate::app) fn open_commit_changes(
        &mut self,
        oid: &str,
        parent: Option<&str>,
        subject: &str,
    ) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        if self.combined_commit().is_some_and(|c| c.oid == oid) {
            if let Some(index) = self.combined_index() {
                self.activate_tab(index);
            }
            return;
        }
        let index = match self.combined_index() {
            Some(index) => index,
            None => {
                let preview = (self.git.commit.preview)
                    .and_then(|id| self.editor.documents.iter().position(|d| d.id == id));
                preview.unwrap_or_else(|| self.editor.show_output(subject, ""))
            }
        };
        self.activate_tab(index);
        self.start_combined_in(index);
        let source = CommitSource {
            root: root.clone(),
            oid: oid.to_owned(),
            parent: parent.map(str::to_owned),
            title: format!("{} - {subject}", crate::app::git::log::short(oid)),
            loading: true,
        };
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        tab.source = Source::Commit(source);
        let sender = tab.results.0.clone();
        let (oid, parent) = (oid.to_owned(), parent.map(str::to_owned));
        self.spawn_git(UserEvent::GitCombined, move |tool| {
            let cancel = winc_editor::format::Cancel::default();
            let files = match &tool {
                Ok(git) => git.commit_files(&root, parent.as_deref(), &oid, &cancel),
                Err(e) => Err(unusable(e)),
            };
            let _ = sender.send(Arrival::Files(oid, files));
        });
        self.sync_combined_title();
        self.relayout();
        self.request_redraw();
    }

    /// 处理后台交回的提交改动的文件和要单独打开的文件；比较完的段由 `poll_combined` 处理。
    pub(super) fn commit_arrived(&mut self, arrivals: Vec<Arrival>) {
        for arrival in arrivals {
            match arrival {
                Arrival::Files(oid, files) => self.commit_files_arrived(&oid, files),
                Arrival::Opened(title, view) => self.commit_section_opened(&title, view),
                Arrival::Compared(_) => {}
            }
        }
    }

    /// 收到提交 `oid` 改动的文件：每个文件列为一段，由后台比较。期间换了内容时丢弃。
    fn commit_files_arrived(&mut self, oid: &str, files: Result<Vec<CommitFile>, String>) {
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let Source::Commit(source) = &mut tab.source else {
            return;
        };
        if source.oid != oid {
            return;
        }
        source.loading = false;
        let files = match files {
            Ok(files) => files,
            Err(e) => {
                self.flash(e);
                return;
            }
        };
        for file in files {
            tab.next_id += 1;
            tab.sections.push(Section {
                id: tab.next_id,
                path: file.path,
                from: file.from,
                kind: DiffKind::Whole,
                change: file.change,
                body: Body::Waiting,
                folded: false,
                stale: true,
                running: false,
                revision: None,
                stamp: None,
            });
        }
        tab.generation += 1;
        self.pump_combined();
        self.request_redraw();
    }

    /// 在后台比较提交的第 `i` 段：该文件在第一个父提交与该提交中的内容。
    pub(super) fn compare_commit_section(&mut self, i: usize) {
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let Some(source) = tab.source.commit().cloned() else {
            return;
        };
        let root = source.root.clone();
        let Some(section) = tab.sections.get_mut(i) else {
            return;
        };
        section.stale = false;
        section.running = true;
        let file = CommitFile {
            path: section.path.clone(),
            from: section.from.clone(),
            change: section.change,
            lines: None,
        };
        let sender = tab.results.0.clone();
        self.spawn_git(UserEvent::GitCombined, move |tool| {
            let view = match &tool {
                Ok(git) => crate::app::git::log::commit_diff(
                    git,
                    &root,
                    source.parent.as_deref(),
                    &source.oid,
                    &file,
                ),
                Err(e) => Err(unusable(e)),
            };
            let _ = sender.send(Arrival::Compared(Compared {
                root,
                path: file.path,
                kind: DiffKind::Whole,
                revision: None,
                stamp: None,
                view,
            }));
        });
    }

    /// 在单独的差异标签中打开提交的第 `i` 段（段头上的按钮）：在后台准备，到达后新开标签。
    pub(super) fn open_commit_section(&mut self, i: usize) {
        let Some(tab) = self.combined_tab() else {
            return;
        };
        let (Some(source), Some(section)) = (tab.source.commit(), tab.sections.get(i)) else {
            return;
        };
        let root = source.root.clone();
        let name = winc_editor::display_name(&section.path).into_owned();
        let title = format!("{name} · {}", crate::app::git::log::short(&source.oid));
        let file = CommitFile {
            path: section.path.clone(),
            from: section.from.clone(),
            change: section.change,
            lines: None,
        };
        let (oid, parent) = (source.oid.clone(), source.parent.clone());
        let sender = tab.results.0.clone();
        self.spawn_git(UserEvent::GitCombined, move |tool| {
            let view = match &tool {
                Ok(git) => {
                    crate::app::git::log::commit_diff(git, &root, parent.as_deref(), &oid, &file)
                }
                Err(e) => Err(unusable(e)),
            };
            let _ = sender.send(Arrival::Opened(title, view));
        });
    }

    /// 收到要单独打开的一个文件的差异：新开差异标签。
    fn commit_section_opened(&mut self, title: &str, view: Result<DiffView, String>) {
        match view {
            Ok(view) => {
                self.editor.show_diff_in(None, title, view);
                self.relayout();
                self.request_redraw();
            }
            Err(e) => self.flash(e),
        }
    }

    /// 合并差异标签的标题：显示提交时是短提交号和说明的首行。
    pub(super) fn commit_title(&self) -> Option<String> {
        Some(self.combined_tab()?.source.commit()?.title.clone())
    }
}
