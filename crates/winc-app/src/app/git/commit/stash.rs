//! 提交视图底部的储藏：一组可以收起的行，每条储藏一行。
//!
//! 一组贴在改动列表下面，展开、收起时像抽屉一样往上拉开、往下收回：标题跟着上升，各行从下沿底下
//! 露出来，上面的改动列表随之变矮，节奏和文件树展开一个目录相同（`TREE_ANIMATION`）。
//!
//! 悬停时右端是「应用」和「⋯」（应用、应用并删除、删除），右键同样是这个菜单。点击一条储藏时在
//! 单独的预览标签里打开它的第一个改动文件，同时在这一行下面列出它改动的文件，点击其中一个即在
//! 预览标签里查看；差异标签里的上一个、下一个文件同样在这些文件之间切换。储藏时一起收进去的
//! 未跟踪文件排在后面。直接列出的条数有上限，再多时最后一行是「还有 N 条」，点击后弹出全部储藏
//! 的菜单。

use winc_editor::DocumentId;
use winc_editor::git::{CommitFile, Stash};

use super::*;

/// 储藏一组展开时最多占几行（行），连同「还有 N 条」：再多会把上面的改动列表挤得太矮。
pub(crate) const STASH_ROWS: usize = 5;

/// 正在查看的储藏下面最多列出的文件行数（行），包括「还有 N 个文件」：再多时上方的改动列表
/// 高度过小。列出的一段总包含正在查看的文件，其余文件通过差异标签里的上一个、下一个文件切换。
const STASH_FILE_ROWS: usize = 6;

/// 储藏一组展开时标题下面的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StashLine {
    /// 第几条储藏（`GitState::stashes` 的下标）。
    Stash(usize),
    /// 正在看的储藏里的第几个文件（`StashView::files` 的下标）。
    File(usize),
    /// 正在看的储藏里没有列出的文件个数。
    MoreFiles(usize),
    /// 没有列出的储藏条数。
    MoreStashes(usize),
}

/// 储藏一组上下各留的空隙（逻辑像素）：列表间隔（`SIDEBAR_LIST_GAP`）的一半。这一组上下都是
/// 分隔线，分隔线本身已经隔开了相邻区域，留半个间隔即可。
pub(crate) const STASH_GAP: f32 = SIDEBAR_LIST_GAP / 2.0;
/// 储藏一组收着时的高度（逻辑像素）：标题一行，上下各留 [`STASH_GAP`]。
const STASH_CLOSED: f32 = COMMIT_ROW + 2.0 * STASH_GAP;

/// 正在预览标签里看的一条储藏。
#[derive(Debug, Clone)]
pub(crate) struct StashView {
    /// 储藏的提交号。
    pub oid: String,
    /// 储藏的名字（`stash@{0}`），写在差异标签的标题里。
    pub name: String,
    /// 改动的文件，各自连同比较的两个版本：（文件，左边的版本，右边的版本）；未跟踪的文件左边是
    /// `None`。
    pub files: Vec<(CommitFile, Option<String>, String)>,
    /// 现在看的是第几个文件。
    pub at: usize,
}

/// 后台取回的一条储藏的改动文件，或者失败的原因。
pub(super) type StashFiles = Result<Vec<(CommitFile, Option<String>, String)>, String>;

impl App {
    /// 储藏一组展开时直接列出的储藏：最多 [`STASH_ROWS`] 行；再多时留出最后一行写「还有 N 条」。
    pub(crate) fn shown_stashes(&self) -> &[Stash] {
        let stashes = self.git.stashes.as_slice();
        if stashes.len() > STASH_ROWS {
            &stashes[..STASH_ROWS - 1]
        } else {
            stashes
        }
    }

    /// 储藏一组展开时标题下面的各行：各条储藏，正在看的那条下面是它的文件，最后是「还有 N 条」。
    pub(crate) fn stash_lines(&self) -> Vec<StashLine> {
        let shown = self.shown_stashes();
        let open = self.listed_stash_view();
        let mut lines = Vec::new();
        for (i, stash) in shown.iter().enumerate() {
            lines.push(StashLine::Stash(i));
            let Some(view) = open.filter(|view| view.oid == stash.oid) else {
                continue;
            };
            let count = view.files.len();
            if count <= STASH_FILE_ROWS {
                lines.extend((0..count).map(StashLine::File));
                continue;
            }
            let room = STASH_FILE_ROWS - 1;
            let start = view.at.saturating_sub(room / 2).min(count - room);
            lines.extend((start..start + room).map(StashLine::File));
            lines.push(StashLine::MoreFiles(count - room));
        }
        let rest = self.git.stashes.len() - shown.len();
        if rest > 0 {
            lines.push(StashLine::MoreStashes(rest));
        }
        lines
    }

    /// 储藏一组展开时，标题下面的行数（[`App::stash_lines`]）。
    pub(crate) fn stash_open_rows(&self) -> usize {
        self.stash_lines().len()
    }

    /// 储藏的预览标签还开着、文件没有收起时，在里面看的储藏：它的文件列在储藏一组里。
    pub(crate) fn listed_stash_view(&self) -> Option<&StashView> {
        if self.commit.stash_files_folded {
            return None;
        }
        self.previewed_stash()
    }

    /// 储藏的预览标签还开着时，在里面看的储藏。
    fn previewed_stash(&self) -> Option<&StashView> {
        let view = self.commit.stash_view.as_ref()?;
        let id = self.commit.stash_preview?;
        self.editor
            .documents
            .iter()
            .any(|d| d.id == id)
            .then_some(view)
    }

    /// 储藏一组的终点高度变化（展开、收起一条储藏的文件，或在别处关闭了预览标签，文件不再列出）
    /// 时，从当前显示的高度开始过渡。每帧绘制提交视图之前调用，预览标签无论在何处关闭都能察觉。
    ///
    /// 整个一组出现或消失（新增第一条储藏、删除最后一条）时不过渡。
    pub(crate) fn track_stash_height(&mut self) {
        let settled = self.stash_settled_height();
        let old = std::mem::replace(&mut self.commit.stash_settled, settled);
        if (settled - old).abs() < 0.5 || old == 0.0 || settled == 0.0 {
            return;
        }
        let from = (self.commit.stash_motion).map_or(old, |(from, clock)| clock.between(from, old));
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.commit.stash_motion = clock.map(|clock| (from, clock));
    }

    /// 储藏一组占的高度（逻辑像素）：收着时是 [`STASH_CLOSED`]，展开时再加上
    /// [`App::stash_open_rows`] 行；展开、收起的途中按时钟过渡。没有储藏时为 0。
    pub(crate) fn stash_section_height(&self) -> f32 {
        let settled = self.stash_settled_height();
        (self.commit.stash_motion).map_or(settled, |(from, clock)| clock.between(from, settled))
    }

    /// 储藏一组不在过渡中时的高度（逻辑像素），见 [`App::stash_section_height`]。
    fn stash_settled_height(&self) -> f32 {
        if self.git.stashes.is_empty() {
            return 0.0;
        }
        if self.commit.stash_open {
            STASH_CLOSED + self.stash_open_rows() as f32 * COMMIT_ROW
        } else {
            STASH_CLOSED
        }
    }

    /// 储藏一组标题上的箭头转到哪儿：0 朝右（收着），1 朝下（展开）。
    ///
    /// 按一组现在拉开了多少算，途中再点一下往回收时箭头和高度一起从当时的位置往回走。
    pub(crate) fn stash_chevron(&self) -> f32 {
        let span = self.stash_open_rows() as f32 * COMMIT_ROW;
        if span <= 0.0 {
            return 0.0;
        }
        ((self.stash_section_height() - STASH_CLOSED) / span).clamp(0.0, 1.0)
    }

    /// 点了储藏一组的标题：收起或展开，从现在的高度过渡过去。
    pub(super) fn toggle_stashes(&mut self) {
        let from = self.stash_section_height();
        self.commit.stash_open = !self.commit.stash_open;
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.commit.stash_motion = clock.map(|clock| (from, clock));
        // 过渡已经从现在的高度开始，`track_stash_height` 不必再开始一次。
        self.commit.stash_settled = self.stash_settled_height();
        self.commit.scroll = self.commit.scroll.min(self.commit_max_scroll());
    }

    /// 在预览标签里打开第 `i` 条储藏：先在后台取它改动的文件，取回来以后打开第一个。
    ///
    /// 点击的是正在查看的储藏时不重新打开，只收起或展开它下面的文件。
    pub(super) fn show_stash(&mut self, i: usize) {
        let Some(stash) = self.git.stashes.get(i).cloned() else {
            return;
        };
        if self
            .previewed_stash()
            .is_some_and(|view| view.oid == stash.oid)
        {
            self.commit.stash_files_folded = !self.commit.stash_files_folded;
            return;
        }
        let sender = self.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let files = git.and_then(|git| stash_files(git, root, &stash));
            let _ = sender.send(Outcome::StashFiles(stash.oid, stash.name, files));
        });
    }

    /// 后台取回了储藏 `oid`（名字是 `name`）改动的文件：打开第一个；没有能看的文件时说明。
    pub(super) fn stash_files_arrived(&mut self, oid: String, name: String, files: StashFiles) {
        let files = match files {
            Ok(files) => files,
            Err(e) => return self.flash(e),
        };
        if files.is_empty() {
            return self.flash(t!("git-stash-no-files"));
        }
        self.commit.stash_files_folded = false;
        self.commit.stash_view = Some(StashView {
            oid,
            name,
            files,
            at: 0,
        });
        self.open_stash_file(0);
    }

    /// 在储藏的预览标签里打开正在看的储藏的第 `at` 个文件。
    pub(super) fn open_stash_file(&mut self, at: usize) {
        let Some(view) = self.commit.stash_view.as_mut() else {
            return;
        };
        let Some((file, old, new)) = view.files.get(at).cloned() else {
            return;
        };
        view.at = at;
        let name = winc_editor::display_name(&file.path).into_owned();
        let title = format!("{name} · {}", view.name);
        let sender = self.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let diff = git.and_then(|git| {
                crate::app::git::log::commit_diff(git, root, old.as_deref(), &new, &file)
            });
            let _ = sender.send(Outcome::StashDiff(title, diff.map(Box::new)));
        });
    }

    /// 后台准备好了储藏里一个文件的差异：在储藏的预览标签里换上它。
    pub(super) fn stash_diff_arrived(
        &mut self,
        title: String,
        view: Result<Box<winc_editor::diff_view::DiffView>, String>,
    ) {
        match view {
            Ok(view) => {
                let index = self
                    .editor
                    .show_diff_in(self.commit.stash_preview, &title, *view);
                self.commit.stash_preview = Some(self.editor.documents[index].id);
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

    /// 当前标签是不是储藏的预览标签，而且还在看那条储藏。
    fn viewing_stash(&self) -> Option<&StashView> {
        let active: Option<DocumentId> = self.editor.active().map(|d| d.id);
        let view = self.commit.stash_view.as_ref()?;
        (active.is_some() && active == self.commit.stash_preview).then_some(view)
    }

    /// 储藏的预览标签里，同一条储藏中的下一个（`forward`）或上一个文件：（从 0 起的序号，文件名，
    /// 文件总数）；当前标签不是储藏的预览，或者没有更多文件时为 `None`。
    pub(in crate::app) fn next_stash_file(&self, forward: bool) -> Option<(usize, String, usize)> {
        let view = self.viewing_stash()?;
        let next = if forward {
            view.at + 1
        } else {
            view.at.checked_sub(1)?
        };
        let (file, ..) = view.files.get(next)?;
        let name = winc_editor::display_name(&file.path).into_owned();
        Some((next, name, view.files.len()))
    }

    /// 在储藏的预览标签里打开下一个（`forward`）或上一个文件，返回是否已打开。
    pub(in crate::app) fn step_stash_file(&mut self, forward: bool) -> bool {
        let Some((next, name, total)) = self.next_stash_file(forward) else {
            return false;
        };
        self.open_stash_file(next);
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

/// 储藏 `stash` 改动的文件，连同比较的两个版本。
///
/// 已跟踪的文件比较储藏时的 `HEAD`（第一个父提交）和储藏本身；一起收进去的未跟踪文件在第三个父
/// 提交里，左边为空。
fn stash_files(
    git: &winc_editor::git::Git,
    root: &std::path::Path,
    stash: &Stash,
) -> Result<Vec<(CommitFile, Option<String>, String)>, String> {
    let cancel = winc_editor::format::Cancel::default();
    let base = stash.parents.first().cloned();
    let tracked = git.commit_files(root, base.as_deref(), &stash.oid, &cancel)?;
    let mut files: Vec<_> = (tracked.into_iter())
        .map(|file| (file, base.clone(), stash.oid.clone()))
        .collect();
    if let Some(untracked) = stash.parents.get(2) {
        let added = git.commit_files(root, None, untracked, &cancel)?;
        files.extend(
            added
                .into_iter()
                .map(|file| (file, None, untracked.clone())),
        );
    }
    Ok(files)
}
