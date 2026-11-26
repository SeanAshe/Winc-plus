//! 提交视图底部一组中展开查看的一条储藏或一个提交：它改动的文件列在这一行下面，点击其中一个
//! 即在单独的预览标签里查看差异；差异标签里的上一个、下一个文件同样在这些文件之间切换。
//!
//! 储藏和提交历史共用一个预览标签，同一时刻只查看一条储藏或一个提交。

use winc_editor::DocumentId;
use winc_editor::git::CommitFile;

use super::*;

/// 正在查看的储藏或提交下面最多列出的文件行数（行），包括「还有 N 个文件」：再多时上方的
/// 改动列表高度过小。列出的一段总包含正在查看的文件，其余文件通过差异标签里的上一个、下一个
/// 文件切换。
const PEEK_FILE_ROWS: usize = 6;

/// 正在预览标签里查看的一条储藏或一个提交。
#[derive(Debug, Clone)]
pub(crate) struct PeekView {
    /// 储藏或提交的提交号。
    pub oid: String,
    /// 写在差异标签标题里的名字：储藏的名字（`stash@{0}`）或短提交号。
    pub name: String,
    /// 改动的文件，各自连同比较的两个版本：（文件，左边的版本，右边的版本）；左边为 `None`
    /// 时是新增的文件（储藏的未跟踪文件、根提交的文件）。
    pub files: Vec<(CommitFile, Option<String>, String)>,
    /// 现在查看的是第几个文件。
    pub at: usize,
}

impl PeekView {
    /// 返回这一行下面列出的文件（`files` 的下标范围）和没有列出的文件个数。
    pub(crate) fn listed(&self) -> (Range<usize>, usize) {
        let count = self.files.len();
        if count <= PEEK_FILE_ROWS {
            return (0..count, 0);
        }
        let room = PEEK_FILE_ROWS - 1;
        let start = self.at.saturating_sub(room / 2).min(count - room);
        (start..start + room, count - room)
    }
}

/// 后台取回的储藏或提交改动的文件，或者失败的原因。
pub(super) type PeekFiles = Result<Vec<(CommitFile, Option<String>, String)>, String>;

impl App {
    /// 预览标签还开着、文件没有收起时查看的储藏或提交：它的文件列在所在的一行下面。
    pub(crate) fn listed_peek(&self) -> Option<&PeekView> {
        if self.git.commit.peek_folded {
            return None;
        }
        self.previewed_peek()
    }

    /// 预览标签还开着时查看的储藏或提交。
    fn previewed_peek(&self) -> Option<&PeekView> {
        let view = self.git.commit.peek.as_ref()?;
        let id = self.git.commit.peek_preview?;
        self.editor
            .documents
            .iter()
            .any(|d| d.id == id)
            .then_some(view)
    }

    /// 当前标签是预览标签、并且仍在查看时，返回所查看的储藏或提交号；绘制时据此铺选中的底色。
    pub(crate) fn viewed_peek(&self) -> Option<&str> {
        self.viewing_peek().map(|view| view.oid.as_str())
    }

    /// 点击了提交号为 `oid` 的储藏或提交：正在查看它时收起或展开它下面的文件，返回 `true`；
    /// 否则返回 `false`，由调用方在后台读取它的文件。
    pub(super) fn refold_peek(&mut self, oid: &str) -> bool {
        if !self.previewed_peek().is_some_and(|view| view.oid == oid) {
            return false;
        }
        self.git.commit.peek_folded = !self.git.commit.peek_folded;
        true
    }

    /// 开始一次新的查看请求，返回它的序号；此前尚未到达的请求随之作废。
    pub(super) fn next_peek_request(&mut self) -> u64 {
        self.git.commit.peek_request += 1;
        self.git.commit.peek_request
    }

    /// 后台取回了储藏或提交 `oid`（名字是 `name`）改动的文件：打开第一个；没有能看的文件时
    /// 说明。`request` 不是最近的请求时丢弃。
    pub(in crate::app) fn peek_files_arrived(
        &mut self,
        request: u64,
        oid: String,
        name: String,
        files: PeekFiles,
    ) {
        if request != self.git.commit.peek_request {
            return;
        }
        let files = match files {
            Ok(files) => files,
            Err(e) => return self.flash(e),
        };
        if files.is_empty() {
            return self.flash(t!("git-peek-no-files"));
        }
        self.git.commit.peek_folded = false;
        self.git.commit.peek = Some(PeekView {
            oid,
            name,
            files,
            at: 0,
        });
        self.open_peek_file(0);
    }

    /// 在预览标签里打开正在查看的储藏或提交的第 `at` 个文件。
    pub(super) fn open_peek_file(&mut self, at: usize) {
        let Some(view) = self.git.commit.peek.as_mut() else {
            return;
        };
        let Some((file, old, new)) = view.files.get(at).cloned() else {
            return;
        };
        view.at = at;
        let name = winc_editor::display_name(&file.path).into_owned();
        let title = format!("{name} · {}", view.name);
        let request = self.next_peek_request();
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let diff = git.and_then(|git| {
                crate::app::git::log::commit_diff(git, root, old.as_deref(), &new, &file)
            });
            let _ = sender.send(Outcome::PeekDiff(request, title, diff.map(Box::new)));
        });
    }

    /// 后台准备好了一个文件的差异：在预览标签里换上它。`request` 不是最近的请求时丢弃，
    /// 其间打开的其他标签不被预览标签换下。
    pub(super) fn peek_diff_arrived(
        &mut self,
        request: u64,
        title: String,
        view: Result<Box<winc_editor::diff_view::DiffView>, String>,
    ) {
        if request != self.git.commit.peek_request {
            return;
        }
        match view {
            Ok(view) => {
                let index = self
                    .editor
                    .show_diff_in(self.git.commit.peek_preview, &title, *view);
                self.git.commit.peek_preview = Some(self.editor.documents[index].id);
                self.relayout();
                self.clamp_scroll();
                self.land_diff_file();
            }
            Err(e) => {
                self.diff.land = None;
                self.flash(e);
            }
        }
    }

    /// 当前标签是预览标签、并且仍在查看时，返回所查看的储藏或提交。
    fn viewing_peek(&self) -> Option<&PeekView> {
        let active: Option<DocumentId> = self.editor.active().map(|d| d.id);
        let view = self.git.commit.peek.as_ref()?;
        (active.is_some() && active == self.git.commit.peek_preview).then_some(view)
    }

    /// 预览标签里，同一条储藏或同一个提交中的下一个（`forward`）或上一个文件：（从 0 起的
    /// 序号，文件名，文件总数）；当前标签不是预览标签，或者没有更多文件时为 `None`。
    pub(in crate::app) fn next_peek_file(&self, forward: bool) -> Option<(usize, String, usize)> {
        let view = self.viewing_peek()?;
        let next = if forward {
            view.at + 1
        } else {
            view.at.checked_sub(1)?
        };
        let (file, ..) = view.files.get(next)?;
        let name = winc_editor::display_name(&file.path).into_owned();
        Some((next, name, view.files.len()))
    }

    /// 在预览标签里打开下一个（`forward`）或上一个文件，返回是否已打开。
    pub(in crate::app) fn step_peek_file(&mut self, forward: bool) -> bool {
        let Some((next, name, total)) = self.next_peek_file(forward) else {
            return false;
        };
        self.open_peek_file(next);
        let (index, name) = (next + 1, name.as_str());
        self.flash(if forward {
            t!("git-next-file", name = name, index = index, total = total)
        } else {
            t!(
                "git-previous-file",
                name = name,
                index = index,
                total = total
            )
        });
        true
    }
}
