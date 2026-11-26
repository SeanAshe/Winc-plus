//! 提交视图底部的储藏：一组可以收起的行，每条储藏一行。
//!
//! 一组贴在改动列表下面，展开、收起时往上拉开、往下收回，改动列表随之变矮（`TREE_ANIMATION`）。
//!
//! 悬停时右端是「应用」和「⋯」（应用、应用并删除、删除），右键弹出同一菜单。点击一条储藏时
//! 在这一行下面列出它改动的文件并预览第一个（见 [`PeekView`]），储藏时一起收进去的未跟踪文件排在
//! 后面。直接列出的条数有上限，再多时最后一行是「还有 N 条」，点击后弹出全部储藏的菜单。

use winc_editor::git::Stash;

use super::*;
use peek::PeekFiles;

/// 储藏一组展开时最多占几行（行），连同「还有 N 条」：再多会把上面的改动列表挤得太矮。
pub(crate) const STASH_ROWS: usize = 5;

/// 储藏一组展开时标题下面的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StashLine {
    /// 第几条储藏（`GitState::stashes` 的下标）。
    Stash(usize),
    /// 正在查看的储藏里的第几个文件（`PeekView::files` 的下标）。
    File(usize),
    /// 正在查看的储藏里没有列出的文件个数。
    MoreFiles(usize),
    /// 没有列出的储藏条数。
    MoreStashes(usize),
}

/// 储藏一组上下各留的空隙（逻辑像素）：列表间隔（`SIDEBAR_LIST_GAP`）的一半。这一组上下都是
/// 分隔线，分隔线本身已经隔开了相邻区域，留半个间隔即可。
pub(crate) const STASH_GAP: f32 = SIDEBAR_LIST_GAP / 2.0;

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
        let open = self.listed_peek();
        let mut lines = Vec::new();
        for (i, stash) in shown.iter().enumerate() {
            lines.push(StashLine::Stash(i));
            let Some(view) = open.filter(|view| view.oid == stash.oid) else {
                continue;
            };
            let (files, rest) = view.listed();
            lines.extend(files.map(StashLine::File));
            if rest > 0 {
                lines.push(StashLine::MoreFiles(rest));
            }
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

    /// 储藏一组的终点高度变化（展开、收起一条储藏的文件，或在别处关闭了预览标签，文件不再列出）
    /// 时，从当前显示的高度开始过渡。每帧绘制提交视图之前调用，预览标签无论在何处关闭都能察觉。
    ///
    /// 整组出现或消失时同样过渡；仓库的状态刚读到时直接到位，见 [`App::section_clock`]。
    pub(crate) fn track_stash_height(&mut self) {
        let (settled, clock) = (self.stash_settled_height(), self.section_clock());
        self.git.commit.stashes.track(settled, clock);
    }

    /// 返回储藏一组的高度（逻辑像素），过渡中按时钟插值；没有储藏时为 0。
    pub(crate) fn stash_section_height(&self) -> f32 {
        (self.git.commit.stashes).height(self.stash_settled_height())
    }

    /// 储藏一组不在过渡中时的高度（逻辑像素），见 [`App::stash_section_height`]。
    fn stash_settled_height(&self) -> f32 {
        if self.git.stashes.is_empty() {
            return 0.0;
        }
        if self.git.commit.stashes.open {
            DRAWER_CLOSED + self.stash_open_rows() as f32 * COMMIT_ROW
        } else {
            DRAWER_CLOSED
        }
    }

    /// 返回储藏一组标题上箭头的角度比例，见 [`Drawer::chevron`]。
    pub(crate) fn stash_chevron(&self) -> f32 {
        let span = self.stash_open_rows() as f32 * COMMIT_ROW;
        (self.git.commit.stashes).chevron(self.stash_settled_height(), span)
    }

    /// 点击储藏一组的标题：收起或展开，从现在的高度过渡过去。
    pub(super) fn toggle_stashes(&mut self) {
        let from = self.stash_section_height();
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.git.commit.stashes.toggle(from, clock);
        self.git.commit.stashes.settled = self.stash_settled_height();
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
    }

    /// 在预览标签里打开第 `i` 条储藏：先在后台取它改动的文件，取回来以后打开第一个。
    ///
    /// 点击的是正在查看的储藏时不重新打开，只收起或展开它下面的文件。
    pub(super) fn show_stash(&mut self, i: usize) {
        let Some(stash) = self.git.stashes.get(i).cloned() else {
            return;
        };
        if self.refold_peek(&stash.oid) {
            return;
        }
        let request = self.next_peek_request();
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let files = git.and_then(|git| stash_files(git, root, &stash));
            let _ = sender.send(Outcome::PeekFiles(request, stash.oid, stash.name, files));
        });
    }
}

/// 储藏 `stash` 改动的文件，连同比较的两个版本。
///
/// 已跟踪的文件比较储藏时的 `HEAD`（第一个父提交）和储藏本身；一起收进去的未跟踪文件在第三个父
/// 提交里，左边为空。
fn stash_files(git: &winc_editor::git::Git, root: &std::path::Path, stash: &Stash) -> PeekFiles {
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
