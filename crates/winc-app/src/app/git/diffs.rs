//! Git 差异标签：比较哪两份内容（[`DiffKind`]）、在后台准备差异、打开与预览、状态刷新后重新
//! 比较，以及收下后台准备好的结果。

use super::*;

/// 提交视图打开的差异比较哪两份内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DiffKind {
    /// 上次提交 ↔ 工作区：勾选式，以及暂存区视图中未跟踪、有冲突的文件。
    #[default]
    Whole,
    /// 暂存区 ↔ 工作区：暂存区视图「更改」一组中的文件。
    Unstaged,
    /// 上次提交 ↔ 暂存区：暂存区视图「暂存的更改」一组中的文件，右侧只读。
    Staged,
}

/// 差异的打开方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Target {
    /// 新建差异标签（或替换同名的差异标签）并切换到该标签。
    Open,
    /// 提交视图的预览标签，参数为请求序号。只采用最新的请求，快速切换选择时，
    /// 先发出而后送达的结果被丢弃。
    Preview(u64),
    /// 原地更新已打开的差异标签，不切换标签，不改变滚动位置。
    Refresh(DocumentId),
}

/// 后台准备好的差异。
pub(super) struct Prepared {
    /// 进行比较的仓库；之后更换了目录时丢弃结果。
    root: PathBuf,
    /// 比较的文件。
    path: PathBuf,
    /// 比较时 `HEAD` 的提交号，左侧内容取自该提交。
    oid: Option<String>,
    /// 比较的两份内容。
    kind: DiffKind,
    target: Target,
    /// 比较结果；无法比较时为原因（二进制、过大、无法读取）。
    view: Result<DiffView, String>,
}

/// 发起比较时编辑器中该文件的快照。
pub(super) struct Snapshot {
    /// 发起比较时的文本。
    text: Rope,
    /// 发起比较时的文档版本号，记进差异视图（`DiffView::source`）。
    pub(super) revision: u64,
    /// 文档的编码，左侧内容按该编码解码。
    encoding: Encoding,
    /// 发起比较时文档是否有未保存的修改。
    dirty: bool,
    /// 文档的语言，左右两侧都按该语言着色。
    language: &'static winc_syntax::Language,
}

/// 已打开的 Git 差异标签。
pub(super) struct DiffTab {
    /// 比较的文件。
    pub(super) path: PathBuf,
    /// 左侧内容对应的 `HEAD` 提交号；`HEAD` 变化时才重新读取左侧内容。
    oid: Option<String>,
    /// 比较的两份内容。
    pub(super) kind: DiffKind,
}

impl DiffTab {
    /// 文件 `path` 的差异标签：左侧取自提交 `oid`，比较 `kind`。
    pub(super) fn new(path: PathBuf, oid: Option<String>, kind: DiffKind) -> DiffTab {
        DiffTab { path, oid, kind }
    }
}

/// 在后台准备一个文件的差异 `kind`：左侧是 `HEAD`（「更改」时是暂存区）中的版本，右侧是编辑器
/// 中的快照，未打开时读取磁盘（「暂存的更改」时是暂存区中的版本，只读）。
pub(super) fn prepare(
    git: &Git,
    root: &Path,
    path: &Path,
    snapshot: Option<Snapshot>,
    oid: Option<&str>,
    kind: DiffKind,
) -> Result<DiffView, String> {
    let name = winc_editor::display_name(path).into_owned();
    if winc_backend::fs::is_dir(path) {
        return Err(t!("git-diff-submodule", name = name));
    }
    let too_large = |len: usize| len as u64 > LARGE_FILE;
    let usable = |bytes: &Option<Vec<u8>>| match bytes {
        Some(b) if too_large(b.len()) => Err(t!("git-diff-too-large", name = name.as_str())),
        Some(b) if looks_binary(b) => Err(t!("git-diff-binary", name = name.as_str())),
        _ => Ok(()),
    };
    let staged = || {
        let bytes = git
            .index_content(root, path, &Cancel::default())
            .map_err(|e| t!("git-diff-index-unreadable", name = name.as_str(), error = e))?;
        usable(&bytes).map(|()| bytes)
    };
    let left = match kind {
        DiffKind::Unstaged => staged()?,
        DiffKind::Whole | DiffKind::Staged => {
            let head = git
                .head_content(root, path, &Cancel::default())
                .map_err(|e| t!("git-diff-head-unreadable", name = name.as_str(), error = e))?;
            usable(&head)?;
            head
        }
    };

    // 右侧：文件已在编辑器中打开时使用编辑器中的内容（含未保存的修改），否则读取磁盘；「暂存的
    // 更改」取暂存区中的版本。左侧按右侧的编码解码；右侧不存在（文件已删除）时左侧自行识别编码。
    let revision = snapshot.as_ref().map(|s| s.revision);
    let (new, encoding, title, language) = match (kind, snapshot) {
        (DiffKind::Staged, snapshot) => {
            let bytes = staged()?;
            let language = snapshot
                .as_ref()
                .map_or_else(|| winc_syntax::detect(&name), |s| s.language);
            let known = snapshot.map(|s| s.encoding);
            let decoded = bytes.map(|b| match known {
                Some(encoding) => (decode_known(&b, encoding), encoding),
                None => decode_unknown(&b),
            });
            let title = match decoded {
                Some(_) => t!("git-diff-side-index"),
                None => t!("git-diff-side-index-missing"),
            };
            let (text, encoding) = decoded.unzip();
            (text, encoding.or(known), title, language)
        }
        (_, Some(s)) => (
            Some(s.text.to_string()),
            Some(s.encoding),
            worktree_title(s.dirty),
            s.language,
        ),
        (_, None) => {
            let language = winc_syntax::detect(&name);
            match winc_backend::fs::read(path) {
                Ok(bytes) if too_large(bytes.len()) => {
                    return Err(t!("git-diff-too-large", name = name));
                }
                Ok(bytes) if looks_binary(&bytes) => {
                    return Err(t!("git-diff-binary", name = name));
                }
                Ok(bytes) => {
                    let (text, encoding) = decode_unknown(&bytes);
                    (Some(text), Some(encoding), worktree_title(false), language)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    (None, None, t!("git-diff-side-worktree-deleted"), language)
                }
                Err(e) => {
                    let error = e.to_string();
                    return Err(t!("git-read-file-failed", name = name, error = error));
                }
            }
        }
    };
    let old = left.map(|bytes| match encoding {
        Some(encoding) => decode_known(&bytes, encoding),
        None => decode_unknown(&bytes).0,
    });
    let old_title = match (kind, &old, oid) {
        (DiffKind::Unstaged, None, _) => t!("git-diff-side-index-missing").to_owned(),
        (DiffKind::Unstaged, Some(_), _) => t!("git-diff-side-index").to_owned(),
        (_, None, _) => t!("git-diff-side-head-missing").to_owned(),
        (_, Some(_), Some(oid)) => format!("HEAD · {}", &oid[..oid.len().min(7)]),
        (_, Some(_), None) => "HEAD".to_owned(),
    };
    // 右侧是工作区且文件存在时才可编辑。路径按编辑器的规则规范化，以便与已打开的标签匹配。
    let worktree = kind != DiffKind::Staged;
    let editable = (worktree && new.is_some()).then(|| winc_editor::canonical_path(path));
    let blame = |rev: Option<&str>| BlameSource {
        root: root.to_path_buf(),
        path: path.to_path_buf(),
        rev: rev.map(str::to_owned),
    };
    // 暂存区中的内容没有对应的提交，这一侧不提供逐行追溯。
    let mut left = DiffSide::new(old_title, old.as_deref().unwrap_or(""), None, language)
        .with_missing(old.is_none());
    if kind != DiffKind::Unstaged {
        left = left.with_blame(blame(Some(oid.unwrap_or("HEAD"))));
    }
    let mut right = DiffSide::new(title, new.as_deref().unwrap_or(""), editable, language)
        .with_missing(new.is_none());
    if worktree {
        right = right.with_blame(blame(None));
    }
    let mut view = DiffView::new(left, right);
    view.source = revision.filter(|_| worktree);
    view.repo_path = Some(path.to_path_buf());
    Ok(view)
}

/// 返回差异标签的标题：分开显示的两种差异按所在的组注明，与侧栏的组名相同。
pub(super) fn diff_title(path: &Path, kind: DiffKind) -> String {
    let name = winc_editor::display_name(path);
    match kind {
        DiffKind::Whole => t!("git-diff-title", name = name),
        DiffKind::Unstaged => t!("git-diff-title-changes", name = name),
        DiffKind::Staged => t!("git-diff-title-staged", name = name),
    }
}

impl App {
    /// 编辑器中打开着的 `path` 的快照，比较时右侧用它（含未保存的修改）；没有打开时为
    /// `Ok(None)`，读磁盘。
    ///
    /// 打开的是大文件时为 `Err`：不比较，整体读入字符串会超出内存预算。
    pub(super) fn editor_snapshot(&self, path: &Path) -> Result<Option<Snapshot>, ()> {
        let canonical = winc_editor::canonical_path(path);
        let doc = self
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(canonical.as_path()) && d.holds_file_text());
        if doc.is_some_and(|d| d.is_large()) {
            return Err(());
        }
        Ok(doc.and_then(|d| {
            Some(Snapshot {
                text: d.text().rope()?.clone(),
                revision: d.revision,
                encoding: d.encoding,
                dirty: d.is_dirty(),
                language: d.highlighter.language(),
            })
        }))
    }

    /// 当前标签是 Git 差异时比较的两份内容；其他标签为 [`DiffKind::Whole`]。
    pub(crate) fn active_diff_kind(&self) -> DiffKind {
        let tab = (self.editor.active()).and_then(|d| self.git.diff_tabs.get(&d.id));
        tab.map_or(DiffKind::Whole, |tab| tab.kind)
    }

    /// 打开当前文件的改动（命令「Git：显示当前文件的改动」）。
    pub(in crate::app) fn git_show_changes(&mut self) {
        let Some(path) = self.editor.active().and_then(|d| d.path.clone()) else {
            self.flash(t!("git-no-file-in-tab"));
            return;
        };
        self.fetch_changes(&path, Target::Open);
    }

    /// 在提交视图的预览标签中打开 `path` 的改动；依次查看多个文件时只占用一个标签。开着「合并
    /// 差异视图」时改为滚到「未提交的更改」标签中这个文件那一段。
    ///
    /// 暂存区视图中按选中的一行所在的组，分别显示暂存的更改和未暂存的更改（见 [`DiffKind`]）。
    pub(in crate::app) fn git_preview_changes(&mut self, path: &Path) {
        let kind = self.commit_diff_kind(path);
        // 合并差异视图（`git.combinedDiff`）：滚到这个文件那一段，焦点留在列表里。
        if self.config.git_combined_diff == Some(true) {
            let focus = self.focus;
            self.open_combined(Some((path.to_path_buf(), kind)));
            self.focus = focus;
            return;
        }
        self.git.preview_seq += 1;
        self.fetch_diff(path, Target::Preview(self.git.preview_seq), kind);
    }

    /// 状态刷新后重新比较已过时的 Git 差异标签，并原地更新内容。
    ///
    /// `HEAD` 变化（提交、切换分支）时重新读取左侧。右侧为磁盘内容（文件未在编辑器中打开）时，
    /// 文件可能已变化，同样重新比较；其余标签的右侧由 [`App::sync_diff_source`] 随编辑器中的
    /// 文档更新，无需再运行 `git`。有一侧是暂存区的差异总是重新比较：暂存区可能在别处改变。
    pub(super) fn refresh_git_diffs(&mut self) {
        self.refresh_diff_tabs(false);
    }

    /// 其他仓库的状态读到了新的一版：按 [`App::refresh_git_diffs`] 的规则重新比较这些仓库中文件
    /// 的差异标签；没有变化时不做处理。
    pub(super) fn refresh_other_diffs(&mut self) {
        let version = self.git.summaries_version();
        if version != self.git.diffs_seen {
            self.git.diffs_seen = version;
            self.refresh_diff_tabs(true);
        }
    }

    /// 重新比较已过时的差异标签：`others` 为真时只看其他仓库中的文件，否则只看当前仓库中的文件。
    fn refresh_diff_tabs(&mut self, others: bool) {
        let editor = &self.editor;
        self.git
            .diff_tabs
            .retain(|id, _| editor.document(*id).is_some_and(|d| d.diff.is_some()));
        // 各标签按文件所在仓库的提交号比较；其他仓库还没有读到状态时没有提交号。
        let current = self.git.root.clone();
        let heads: Vec<(DocumentId, Option<PathBuf>, Option<String>)> = (self.git.diff_tabs.iter())
            .map(|(id, tab)| {
                let root = self.repo_of(&tab.path).map(|(root, _)| root);
                let oid = (root.as_deref())
                    .and_then(|root| self.status_in(root, |s| s.oid.clone()))
                    .flatten();
                (*id, root, oid)
            })
            .collect();
        let head_of = |id: &DocumentId| heads.iter().find(|(i, ..)| i == id);
        let editor = &self.editor;
        let follows_editor = |id: DocumentId| {
            editor
                .document(id)
                .and_then(|d| d.diff.as_deref())
                .and_then(|v| v.new.path.as_deref())
                .is_some_and(|path| {
                    editor
                        .documents
                        .iter()
                        .any(|d| d.path.as_deref() == Some(path))
                })
        };
        let stale: Vec<(DocumentId, PathBuf, DiffKind)> = self
            .git
            .diff_tabs
            .iter()
            .filter(|(id, tab)| {
                let Some((_, root, oid)) = head_of(id) else {
                    return false;
                };
                if root.is_none() || (*root != current) != others {
                    return false;
                }
                tab.kind != DiffKind::Whole || tab.oid != *oid || !follows_editor(**id)
            })
            .map(|(id, tab)| (*id, tab.path.clone(), tab.kind))
            .collect();
        for (id, path, kind) in stale {
            self.fetch_diff(&path, Target::Refresh(id), kind);
        }
    }

    /// 在后台读取 `path` 的 Git 改动（上次提交 ↔ 工作区），完成后按 `target` 交给界面线程。
    pub(in crate::app) fn fetch_changes(&mut self, path: &Path, target: Target) {
        self.fetch_diff(path, target, DiffKind::Whole);
    }

    /// 在后台准备 `path` 的差异 `kind`，完成后按 `target` 交给界面线程。
    ///
    /// 文件在所在的已知仓库中比较（见 [`App::repo_of`]），可以不是当前仓库。
    pub(in crate::app) fn fetch_diff(&mut self, path: &Path, target: Target, kind: DiffKind) {
        let Some((root, path)) = self.repo_of(path) else {
            self.flash(match self.git.root {
                Some(_) => t!("git-file-outside-repository"),
                None => t!("git-not-a-repository"),
            });
            return;
        };

        let Ok(snapshot) = self.editor_snapshot(&path) else {
            if !matches!(target, Target::Refresh(_)) {
                let name = winc_editor::display_name(&path).into_owned();
                self.flash(t!("git-diff-too-large", name = name));
            }
            return;
        };

        let oid = self.status_in(&root, |s| s.oid.clone()).flatten();
        let sender = self.git.prepared.0.clone();
        self.spawn_git(UserEvent::GitContent, move |tool| {
            let view = match &tool {
                Ok(git) => prepare(git, &root, &path, snapshot, oid.as_deref(), kind),
                Err(e) => Err(unusable(e)),
            };
            let _ = sender.send(Prepared {
                root,
                path,
                oid,
                kind,
                target,
                view,
            });
        });
    }

    /// 接收后台准备好的差异，按 [`Target`] 新建标签、替换预览标签的内容或原地更新已打开的标签。
    ///
    /// 仓库已更换、预览请求不是最新的一次或原地更新失败时丢弃结果。
    pub(in crate::app) fn poll_git_content(&mut self) {
        while let Ok(prepared) = self.git.prepared.1.try_recv() {
            // 等待结果期间更换了目录：文件属于旧目录中的仓库，不在新目录或开始页中打开标签。
            let known = self.git.root.as_ref() == Some(&prepared.root)
                || self.git.repositories.contains(&prepared.root);
            if !known {
                continue;
            }
            // 预览只采用最新的一次请求。
            if let Target::Preview(seq) = prepared.target
                && seq != self.git.preview_seq
            {
                continue;
            }
            let tab = DiffTab {
                path: prepared.path,
                oid: prepared.oid,
                kind: prepared.kind,
            };
            let view = match prepared.view {
                Ok(view) => view,
                // 原地更新失败（文件变为二进制、无法读取）时不提示，标签保留上一次的内容。
                Err(_) if matches!(prepared.target, Target::Refresh(_)) => continue,
                Err(e) => {
                    self.diff.land = None;
                    self.flash(e);
                    continue;
                }
            };
            let name = winc_editor::display_name(&tab.path).into_owned();
            let title = diff_title(&tab.path, tab.kind);
            let index = match prepared.target {
                Target::Refresh(id) => {
                    // 替换内容，保留滚动位置。
                    if let Some(index) = self.editor.replace_diff(id, &title, view) {
                        self.git.diff_tabs.insert(id, tab);
                        if self.editor.active == Some(index) {
                            self.clamp_scroll();
                        }
                    }
                    self.request_redraw();
                    continue;
                }
                _ if view.is_identical() => {
                    self.diff.land = None;
                    // 逐行比较不计换行符的写法，没有改动时两侧的内容仍可能只差换行符。这类差别
                    // 是否算作修改由 `git` 按 `core.autocrlf` 和文件属性判定，提示只说明差别所在。
                    let text = if view.old.text == view.new.text {
                        t!("git-diff-identical", name = name.as_str())
                    } else {
                        t!("git-diff-line-endings-only", name = name.as_str())
                    };
                    self.flash(text);
                    continue;
                }
                Target::Preview(_) => {
                    let index = self
                        .editor
                        .show_diff_in(self.git.commit.preview, &title, view);
                    self.git.commit.preview = Some(self.editor.documents[index].id);
                    index
                }
                Target::Open => self.editor.show_diff(&title, view),
            };
            let id = self.editor.documents[index].id;
            self.git.diff_tabs.insert(id, tab);
            // 打开时定位到第一处改动。
            let doc = &mut self.editor.documents[index];
            if let Some(row) = doc.diff.as_deref().and_then(|v| {
                v.hunk_rows[winc_editor::diff_view::RIGHT]
                    .first()
                    .map(|r| r.start)
            }) {
                set_scroll_line(doc, row.saturating_sub(3) as f64);
            }
            self.relayout();
            self.clamp_scroll();
            self.land_diff_file();
            self.sync_index_texts();
            self.request_redraw();
        }
    }
}

// 仅在 Unix 上编译测试：Git 的测试只在该平台上运行。
#[cfg(all(test, unix))]
mod tests;
