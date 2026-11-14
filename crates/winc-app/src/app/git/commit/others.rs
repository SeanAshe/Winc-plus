//! 「显示全部仓库」：打开的目录中有多个仓库时，按仓库列表的顺序每个仓库一节。
//!
//! 每节一行标题，当前仓库也有，用 ✓ 标出；当前仓库的改动列表排在它的标题下面，缩进一层。
//! 其他仓库的文件同样分组（勾选式下「更改」「未跟踪的文件」，暂存区视图中「已暂存的更改」
//! 「更改」），可以折叠。没有更改的仓库只有一行。点击其他仓库的标题行或其中的文件即切换到
//! 该仓库，列表的顺序不变。各行的内容取自后台读到的各仓库状态。

use std::sync::PoisonError;

use super::*;

/// 多仓库时其他仓库一节里的一组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OtherGroup {
    /// 暂存区视图：已暂存的更改。
    Staged,
    /// 勾选式：已跟踪文件的更改；暂存区视图：尚未暂存的更改（含未跟踪的文件）。
    Changes,
    /// 勾选式：未跟踪的文件。
    Untracked,
}

impl OtherGroup {
    /// 组标题，与当前仓库同名的组用同一段文字。
    pub(crate) fn title(self) -> &'static str {
        match self {
            OtherGroup::Staged => t!("git-commit-group-staged"),
            OtherGroup::Changes => t!("git-commit-group-changes"),
            OtherGroup::Untracked => t!("git-commit-group-untracked"),
        }
    }

    /// 文件 `entry` 是否在这一组里。
    pub(crate) fn holds(self, entry: &Entry, staging: bool) -> bool {
        match (self, staging) {
            (OtherGroup::Staged, _) => commits_staged(entry),
            (OtherGroup::Changes, true) => entry.unstaged.is_some() || !commits_staged(entry),
            (OtherGroup::Changes, false) => !entry.untracked(),
            (OtherGroup::Untracked, _) => entry.untracked(),
        }
    }
}

/// 多仓库时改动列表中当前仓库那一节以外的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OtherRow {
    /// 当前仓库的标题行；它的改动列表（[`App::commit_rows`]）紧接在下面。
    Current,
    /// 另一个仓库的标题行：该仓库在仓库列表（`GitState::repositories`）中的下标。
    Repository(usize),
    /// 没有更改的另一个仓库：只有这一行，写着它的分支和与上游的差异。
    Quiet(usize),
    /// 另一个仓库里的一组：仓库的下标和组。
    Group(usize, OtherGroup),
    /// 暂存区视图中另一个仓库已暂存的文件：仓库的下标，文件在该仓库状态各项中的下标。
    Staged(usize, usize),
    /// 另一个仓库中有更改的文件：仓库的下标，文件在该仓库状态各项中的下标。暂存区视图中是
    /// 尚未暂存的那一部分。
    File(usize, usize),
}

/// 决定 [`App::commit_other_rows`] 的各项：都没变时各行也不变。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OtherRowsKey {
    /// 各仓库状态的版本。
    version: u64,
    /// 当前仓库。
    root: Option<PathBuf>,
    /// 是否暂存区视图（分组不同）。
    staging: bool,
    /// 仓库列表。
    repositories: Vec<PathBuf>,
    /// 折叠着的仓库和组。
    folds: HashSet<(PathBuf, Option<OtherGroup>)>,
}

/// `entry` 暂存的内容是否随提交写入：暂存区中有内容，并且不在冲突中。暂存区视图中据此把其他
/// 仓库的文件分成「已暂存的更改」和「更改」两组。
fn commits_staged(entry: &Entry) -> bool {
    entry.staged.is_some() && !entry.conflicted()
}

impl App {
    /// 「显示全部仓库」是否开着：没有选过时勾选式开着，暂存区视图关着。
    pub(crate) fn commit_all_repositories(&self) -> bool {
        let chosen = self.state.commit_all_repositories;
        chosen.unwrap_or(!self.git.commit.staging())
    }

    /// 是否在当前仓库的更改之后列出其他仓库：开着「显示全部仓库」，打开的目录中有多个仓库，
    /// 并且不在浏览历史提交的文件。
    pub(crate) fn commit_shows_others(&self) -> bool {
        self.commit_all_repositories()
            && !self.git.commit.browse
            && self.git.repositories.len() >= 2
    }

    /// 返回多仓库时当前仓库那一节以外的各行，按仓库列表的顺序；当前仓库的位置是一行
    /// [`OtherRow::Current`]，它的改动列表画在那一行下面。不列出其他仓库时为空。
    ///
    /// 有更改的仓库一行标题，其下按组列出文件，两部分都有内容的文件在暂存区视图的两组中各列
    /// 一次；折叠的仓库、组只有标题行。没有更改的仓库只有一行，状态尚未读到的仓库只有标题行。
    ///
    /// 每帧要用好几次（排行、铺卡片、算滚动），按决定它的各项缓存（[`OtherRowsKey`]），没变时
    /// 不再锁各仓库的状态重排。
    pub(crate) fn commit_other_rows(&self) -> Vec<OtherRow> {
        if !self.commit_shows_others() {
            return Vec::new();
        }
        let key = OtherRowsKey {
            version: self.git.summaries_version(),
            root: self.git.root.clone(),
            staging: self.git.commit.staging(),
            repositories: self.git.repositories.clone(),
            folds: self.git.commit.other_folds.clone(),
        };
        let mut cache = self.git.commit.other_rows.borrow_mut();
        if let Some((cached, rows)) = cache.as_ref()
            && *cached == key
        {
            return rows.clone();
        }
        let rows = self.build_other_rows();
        *cache = Some((key, rows.clone()));
        rows
    }

    /// 排出 [`App::commit_other_rows`] 的各行。
    fn build_other_rows(&self) -> Vec<OtherRow> {
        let root = self.git.root.as_deref();
        let staging = self.git.commit.staging();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let folded = &self.git.commit.other_folds;
        let mut rows = Vec::new();
        for (i, repo) in self.git.repositories.iter().enumerate() {
            if Some(repo.as_path()) == root {
                rows.push(OtherRow::Current);
                continue;
            }
            let Some(status) = summaries.get(repo) else {
                rows.push(OtherRow::Repository(i));
                continue;
            };
            if status.entries.is_empty() {
                rows.push(OtherRow::Quiet(i));
                continue;
            }
            rows.push(OtherRow::Repository(i));
            if folded.contains(&(repo.clone(), None)) {
                continue;
            }
            let groups = match staging {
                true => [OtherGroup::Staged, OtherGroup::Changes],
                false => [OtherGroup::Changes, OtherGroup::Untracked],
            };
            for group in groups {
                let entries = status.entries.iter().enumerate();
                let files: Vec<usize> = entries
                    .filter(|(_, e)| group.holds(e, staging))
                    .map(|(n, _)| n)
                    .collect();
                if files.is_empty() {
                    continue;
                }
                rows.push(OtherRow::Group(i, group));
                if folded.contains(&(repo.clone(), Some(group))) {
                    continue;
                }
                rows.extend(files.into_iter().map(|n| match group {
                    OtherGroup::Staged => OtherRow::Staged(i, n),
                    _ => OtherRow::File(i, n),
                }));
            }
        }
        // 当前仓库不在仓库列表里（刚打开、还没查找完）时排在最前。
        if !rows.contains(&OtherRow::Current) {
            rows.insert(0, OtherRow::Current);
        }
        rows
    }

    /// 多仓库时当前仓库的改动列表上面有几行（其他仓库的各行和当前仓库的标题行）；不列出其他
    /// 仓库时为 0。
    pub(crate) fn commit_rows_before(&self) -> usize {
        let rows = self.commit_other_rows();
        rows.iter()
            .position(|r| *r == OtherRow::Current)
            .map_or(0, |at| at + 1)
    }

    /// 当前仓库的改动列表相对于整个列表顶部的滚动量：整个列表的滚动量减去它上面那些行的高度。
    /// 改动列表的各行、固定在顶部的行都按它排。
    pub(crate) fn commit_block_scroll(&self) -> f32 {
        self.git.commit.scroll - self.commit_rows_before() as f32 * COMMIT_ROW
    }

    /// 当前仓库的改动列表多缩进几层：多仓库时排在仓库标题下面，缩进一层。
    pub(crate) fn commit_base_level(&self) -> usize {
        usize::from(self.commit_shows_others())
    }

    /// 多仓库时第 `repo` 个仓库（仓库列表中的下标；`None` 是当前仓库）一节的颜色序号：按仓库
    /// 列表的顺序依次取项目徽章的颜色，同一个目录里各仓库的颜色不变、前几个互不相同。
    pub(crate) fn repository_color(&self, repo: Option<usize>) -> usize {
        let root = self.git.root.as_deref();
        let current = || {
            self.git
                .repositories
                .iter()
                .position(|r| Some(r.as_path()) == root)
        };
        repo.or_else(current).unwrap_or(0)
    }

    /// 当前仓库在列表中写的名字，与其他仓库的写法相同（相对打开的目录的路径）。
    pub(crate) fn git_current_repository_name(&self) -> String {
        let root = self.git.root.as_deref();
        let index = self
            .git
            .repositories
            .iter()
            .position(|r| Some(r.as_path()) == root);
        match (index, root) {
            (Some(i), _) => self.git_repository_name(i),
            (None, Some(root)) => winc_editor::display_name(root).into_owned(),
            (None, None) => String::new(),
        }
    }

    /// 多仓库时当前仓库标题行的勾选框：全部勾上为 `Some(true)`，勾了一部分为 `Some(false)`，
    /// 一个都没勾或没有更改时为 `None`。
    pub(crate) fn current_check_state(&self) -> Option<bool> {
        let status = self.git.status.as_ref()?;
        match self.git.commit.picked_entries(status).count() {
            0 => None,
            n => Some(n == status.entries.len()),
        }
    }

    /// 多仓库时当前仓库一节是否收起：收起时只剩它的标题行，改动列表不画。
    pub(crate) fn current_section_folded(&self) -> bool {
        let folded = |root: &Path| {
            let key = (root.to_path_buf(), None);
            self.git.commit.other_folds.contains(&key)
        };
        self.commit_shows_others() && self.git.root.as_deref().is_some_and(folded)
    }

    /// 当前仓库的改动列表占几行：多仓库时它那一节收起后为 0。
    pub(crate) fn commit_block_len(&self) -> usize {
        if self.current_section_folded() {
            0
        } else {
            self.commit_rows().len()
        }
    }

    /// 其他仓库 `repo`（仓库列表中的下标）的一节或其中的组 `group` 是否折叠。
    pub(crate) fn other_folded(&self, repo: usize, group: Option<OtherGroup>) -> bool {
        let Some(root) = self.git.repositories.get(repo) else {
            return false;
        };
        self.git.commit.other_folds.contains(&(root.clone(), group))
    }

    /// 标题行 `header` 的折叠箭头转过的比例：0 指向右（收起），1 指向下（展开）。该行正在展开
    /// 或收起时随动画转动，否则按 `open`。
    pub(crate) fn other_fold_turn(&self, header: OtherRow, open: bool) -> f32 {
        match &self.git.commit.fold_motion {
            Some(m) if m.rows.get(m.parent) == Some(&header) => m.reveal(),
            _ => f32::from(u8::from(open)),
        }
    }

    /// 折叠或展开仓库 `repo`（仓库列表中的下标；`None` 是当前仓库）的一节（`group` 为
    /// `None`）或其他仓库中的一组，并开始展开或收起的动画；动态效果关闭时直接到位。
    pub(super) fn toggle_other_fold(&mut self, repo: Option<usize>, group: Option<OtherGroup>) {
        let current = repo.is_none();
        let root = match repo {
            Some(repo) => self.git.repositories.get(repo).cloned(),
            None => self.git.root.clone(),
        };
        let Some(root) = root else {
            return;
        };
        let (before, block) = (self.commit_other_rows(), self.commit_block_len());
        let folds = &mut self.git.commit.other_folds;
        let expanding = folds.remove(&(root.clone(), group));
        if !expanding {
            folds.insert((root, group));
        }
        let header = match (repo, group) {
            (None, _) => OtherRow::Current,
            (Some(repo), None) => OtherRow::Repository(repo),
            (Some(repo), Some(group)) => OtherRow::Group(repo, group),
        };
        let after = self.commit_other_rows();
        let count = after.len().abs_diff(before.len());
        let block = block.abs_diff(self.commit_block_len());
        // 动画期间按行数较多的状态画：展开时画展开后的行，收起时画收起前的行。
        let rows = if expanding { after } else { before };
        let height = (if current { block } else { count }) as f32 * COMMIT_ROW;
        self.git.commit.motion = None;
        self.git.commit.fold_motion = match rows.iter().position(|r| *r == header) {
            Some(parent) if height > 0.0 => {
                self.clock(crate::app::animation::TREE_ANIMATION)
                    .map(|clock| FoldMotion {
                        rows,
                        parent,
                        count,
                        height,
                        expanding,
                        clock,
                    })
            }
            _ => None,
        };
        // 收起后滚动量可能超出内容：动画结束后由绘制时收回，动画期间不跳。
        self.request_redraw();
    }

    /// 键盘在改动列表里移动选中时：当前仓库一节收起着就先展开，选中的文件露出来。
    pub(super) fn unfold_current_section(&mut self) {
        if self.current_section_folded() {
            self.toggle_other_fold(None, None);
        }
    }

    /// 用第 `repo` 个仓库的根目录和读到的状态调用 `read`；该仓库的状态尚未读到时为 `None`。
    pub(crate) fn other_status<R>(
        &self,
        repo: usize,
        read: impl FnOnce(&Path, &Status) -> R,
    ) -> Option<R> {
        let root = self.git.repositories.get(repo)?;
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        summaries.get(root).map(|status| read(root, status))
    }

    /// 打开或关闭「显示全部仓库」；所有窗口共用，下次启动沿用。
    pub(in crate::app) fn toggle_all_repositories(&mut self) {
        let on = !self.commit_all_repositories();
        self.update_state(|state| state.commit_all_repositories = Some(on));
        if on {
            self.git_read_summaries();
        }
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
        self.request_redraw();
    }

    /// 点了其他仓库的标题行或文件：切换到该仓库，点的是文件时选中它并预览差异。
    pub(super) fn press_other(&mut self, target: CommitTarget) {
        let (repo, entry) = match target {
            CommitTarget::OtherRepository(repo) => (repo, None),
            CommitTarget::OtherFile(repo, entry) | CommitTarget::OtherStagedFile(repo, entry) => {
                (repo, Some(entry))
            }
            _ => return,
        };
        let path = entry.and_then(|entry| {
            let path = |status: &Status| status.entries.get(entry).map(|e| e.path.clone());
            self.other_status(repo, |_, status| path(status)).flatten()
        });
        self.git_switch_repository(repo);
        self.focus = Focus::CommitList;
        if let Some(path) = path {
            self.select_commit_file(path);
        }
        self.request_redraw();
    }

    /// 其他仓库中文件 `path` 是否勾上了。
    pub(crate) fn other_checked(&self, path: &Path) -> bool {
        self.git.commit.other_checked.contains(path)
    }

    /// 第 `repo` 个仓库（`group` 为 `None`）或其中一组标题行的勾选框：全部勾上为 `Some(true)`，
    /// 勾了一部分为 `Some(false)`，一个都没勾或没有文件时为 `None`。
    pub(crate) fn other_group_state(&self, repo: usize, group: Option<OtherGroup>) -> Option<bool> {
        let checked = &self.git.commit.other_checked;
        let count = |status: &Status| {
            let held = |e: &&Entry| group.is_none_or(|g| g.holds(e, false));
            let members: Vec<&Entry> = status.entries.iter().filter(held).collect();
            let on = members.iter().filter(|e| checked.contains(&e.path)).count();
            (on, members.len())
        };
        match self.other_status(repo, |_, status| count(status))? {
            (0, _) => None,
            (on, all) => Some(on == all),
        }
    }

    /// 其他仓库中勾上并且仍然有更改的文件数。
    pub(crate) fn other_checked_count(&self) -> usize {
        self.count_other_checked(false)
    }

    /// 其他仓库中勾上、会随这次提交写进去的文件数：有合并、变基等操作进行中的仓库不一并提交，
    /// 不算在内。提交按钮、确认框上的文件数和能不能提交都按它。
    pub(crate) fn other_committed_count(&self) -> usize {
        self.count_other_checked(true)
    }

    /// 会随这次提交一并提交（「提交并推送」时也一并推送）的其他仓库，写成仓库列表里的名字。
    pub(crate) fn other_commit_names(&self) -> Vec<String> {
        if self.git.commit.amend || !self.commit_shows_others() {
            return Vec::new();
        }
        let roots: Vec<PathBuf> = match self.git.commit.staging() {
            true => self
                .other_staged_roots()
                .into_iter()
                .map(|(r, _)| r)
                .collect(),
            false => {
                let root = self.git.root.as_deref();
                let summaries = self.git.summaries.lock();
                let summaries = summaries.unwrap_or_else(PoisonError::into_inner);
                let others = (self.git.repositories.iter()).filter(|r| Some(r.as_path()) != root);
                let taken = others.filter(|repo| {
                    summaries.get(*repo).is_some_and(|s| {
                        s.operation.is_none()
                            && s.entries.iter().any(|e| self.other_checked(&e.path))
                    })
                });
                taken.cloned().collect()
            }
        };
        let index = |root: &PathBuf| self.git.repositories.iter().position(|r| r == root);
        roots
            .iter()
            .filter_map(index)
            .map(|i| self.git_repository_name(i))
            .collect()
    }

    /// 其他仓库中勾上的文件数；`committable` 时跳过有操作进行中的仓库。
    fn count_other_checked(&self, committable: bool) -> usize {
        if self.git.commit.other_checked.is_empty() || !self.commit_shows_others() {
            return 0;
        }
        let root = self.git.root.as_deref();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let others = (self.git.repositories.iter()).filter(|repo| Some(repo.as_path()) != root);
        let entries = (others.filter_map(|repo| summaries.get(repo)))
            .filter(|s| !committable || s.operation.is_none())
            .flat_map(|s| &s.entries);
        entries.filter(|e| self.other_checked(&e.path)).count()
    }

    /// 其他仓库的状态重读以后，去掉已经没有更改的文件的勾选。
    pub(in crate::app) fn prune_other_checked(&mut self) {
        if self.git.commit.other_checked.is_empty() {
            return;
        }
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let changed = |path: &PathBuf| {
            summaries
                .values()
                .any(|s| s.entries.iter().any(|e| e.path == *path))
        };
        self.git.commit.other_checked.retain(changed);
        // 那个仓库的 `HEAD` 变了（在别处提交、切换了分支）：按处取消勾选的记录对不上了，作废，
        // 这些文件按整个文件勾选（与当前仓库的 `CommitState::head_is` 相同）。
        let commit = &mut self.git.commit;
        let moved: Vec<PathBuf> = (commit.other_heads.iter())
            .filter(|(repo, head)| summaries.get(*repo).is_some_and(|s| s.oid != **head))
            .map(|(repo, _)| repo.clone())
            .collect();
        for repo in moved {
            commit.other_heads.remove(&repo);
            for entry in summaries.get(&repo).iter().flat_map(|s| &s.entries) {
                commit.other_skipped.remove(&entry.path);
            }
        }
        let checked = &commit.other_checked;
        commit
            .other_skipped
            .retain(|path, _| checked.contains(path));
    }

    /// 勾选式下其他仓库中文件 `entry` 的勾选框：`Some(true)` 整个提交，`Some(false)` 只提交一部分
    /// 改动，`None` 不提交。
    pub(crate) fn other_pick_state(&self, entry: &Entry) -> Option<bool> {
        let commit = &self.git.commit;
        let partial = commit.other_skipped.get(&entry.path);
        self.other_checked(&entry.path)
            .then(|| partial.is_none_or(Vec::is_empty))
    }

    /// 多仓库的勾选式下换仓库之前：当前仓库各文件的勾选（连同取消勾选的各处改动）改记成其他仓库
    /// 的勾选，提交的内容和界面上的勾选都不因换仓库而变。
    pub(in crate::app::git) fn carry_checks_out(&mut self) {
        if self.git.commit.staging() || !self.commit_shows_others() {
            return;
        }
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let commit = &mut self.git.commit;
        if let Some(root) = self.git.root.clone() {
            commit.other_heads.insert(root, status.oid.clone());
        }
        for entry in &status.entries {
            let path = &entry.path;
            if !commit.picked(entry) {
                commit.other_checked.remove(path);
                commit.other_skipped.remove(path);
                continue;
            }
            commit.other_checked.insert(path.clone());
            match commit.skipped.remove(path).filter(|s| !s.is_empty()) {
                Some(skipped) => commit.other_skipped.insert(path.clone(), skipped),
                None => commit.other_skipped.remove(path),
            };
        }
    }

    /// 在多仓库的列表里换到的仓库第一次读到状态时：它在其他仓库里的勾选改记成当前仓库的勾选
    /// （与 [`App::carry_checks_out`] 相反）。
    pub(in crate::app::git) fn carry_checks_in(&mut self) {
        if !self.git.commit.carry_in {
            return;
        }
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        self.git.commit.carry_in = false;
        if let Some(root) = &self.git.root {
            self.git.commit.other_heads.remove(root);
        }
        if self.git.commit.staging() || !self.commit_shows_others() {
            return;
        }
        let commit = &mut self.git.commit;
        for entry in &status.entries {
            let path = &entry.path;
            let on = commit.other_checked.remove(path);
            commit.included.remove(path);
            commit.excluded.remove(path);
            match (on, entry.untracked()) {
                (true, true) => _ = commit.included.insert(path.clone()),
                (false, false) => _ = commit.excluded.insert(path.clone()),
                _ => {}
            }
            if let Some(skipped) = commit.other_skipped.remove(path).filter(|_| on) {
                commit.skipped.insert(path.clone(), skipped);
            }
        }
        commit.checks += 1;
    }

    /// 其他仓库中勾上或暂存了、但不随这次提交写入的文件数：`committed` 是一并提交的仓库。
    pub(super) fn others_left(&self, committed: &[PathBuf]) -> usize {
        let staging = self.git.commit.staging();
        let total = match staging {
            true => self.other_staged_count(),
            false => self.other_checked_count(),
        };
        let root = self.git.root.as_deref();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let taken = |e: &Entry| match staging {
            true => commits_staged(e),
            false => self.other_checked(&e.path),
        };
        let going: usize = (committed.iter())
            .filter(|repo| Some(repo.as_path()) != root)
            .filter_map(|repo| summaries.get(repo))
            .map(|s| s.entries.iter().filter(|e| taken(e)).count())
            .sum();
        total.saturating_sub(going)
    }

    /// 提交成功以后：一并提交了的其他仓库里的勾选都已提交，清掉；没有提交的仓库的勾选留着。
    pub(super) fn clear_committed_checks(&mut self) {
        let committed = std::mem::take(&mut self.git.commit.committed_with);
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let commit = &mut self.git.commit;
        for status in committed.iter().filter_map(|repo| summaries.get(repo)) {
            for entry in &status.entries {
                commit.other_checked.remove(&entry.path);
                commit.other_skipped.remove(&entry.path);
            }
        }
    }

    /// 点了其他仓库的勾选框：文件的勾选框只管这一个文件；仓库、组标题的勾选框在其中全部勾上时
    /// 取消全部，否则全部勾上。
    pub(super) fn press_other_check(&mut self, target: CommitTarget) {
        let paths = |status: &Status, entry: Option<usize>| -> Vec<PathBuf> {
            let entries = status.entries.iter().enumerate();
            let wanted = entries.filter(|(i, _)| entry.is_none_or(|entry| entry == *i));
            wanted.map(|(_, e)| e.path.clone()).collect()
        };
        let (repo, entry, group) = match target {
            CommitTarget::OtherGroupCheck(repo) => (repo, None, None),
            CommitTarget::OtherGroupFiles(repo, group) => (repo, None, Some(group)),
            CommitTarget::OtherCheck(repo, entry) => (repo, Some(entry), None),
            _ => return,
        };
        let pick = |status: &Status| {
            let all = paths(status, entry);
            let held = |path: &PathBuf| {
                let entry = status.entries.iter().find(|e| e.path == *path);
                group.is_none_or(|g| entry.is_some_and(|e| g.holds(e, false)))
            };
            all.into_iter().filter(held).collect::<Vec<_>>()
        };
        let Some(paths) = self.other_status(repo, |_, status| pick(status)) else {
            return;
        };
        let commit = &mut self.git.commit;
        // 整个文件勾上或勾掉，之前按处取消勾选的改动作废。
        for path in &paths {
            commit.other_skipped.remove(path);
        }
        let checked = &mut commit.other_checked;
        if paths.iter().all(|path| checked.contains(path)) {
            checked.retain(|path| !paths.contains(path));
        } else {
            checked.extend(paths);
        }
        self.request_redraw();
    }

    /// 暂存区视图中各个其他仓库里随当前仓库一并提交的文件数：仓库的根目录和已暂存的文件数。
    ///
    /// 不列出其他仓库时为空；有合并、变基等操作进行中的仓库不在这里提交。
    fn other_staged(&self) -> Vec<(PathBuf, usize)> {
        if !self.git.commit.staging() || !self.commit_shows_others() {
            return Vec::new();
        }
        let root = self.git.root.as_deref();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let others = (self.git.repositories.iter()).filter(|repo| Some(repo.as_path()) != root);
        let counted = others.filter_map(|repo| {
            let status = summaries.get(repo).filter(|s| s.operation.is_none())?;
            let count = status.entries.iter().filter(|e| commits_staged(e)).count();
            (count > 0).then(|| (repo.clone(), count))
        });
        counted.collect()
    }

    /// 提交前格式化时其他仓库里要格式化的文件和所在仓库：会整个写进这次提交的文件（勾选式下
    /// 勾上、没有按处取消勾选的；暂存区视图中暂存了、工作区里没有另外改动的）。修改上一次提交、
    /// 有合并或变基进行中的仓库不一并提交，也不格式化。
    pub(super) fn other_reformat_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        if !self.commit_shows_others() || self.git.commit.amend {
            return Vec::new();
        }
        let root = self.git.root.as_deref();
        let staging = self.git.commit.staging();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let mut paths = Vec::new();
        for repo in (self.git.repositories.iter()).filter(|r| Some(r.as_path()) != root) {
            let Some(status) = summaries.get(repo).filter(|s| s.operation.is_none()) else {
                continue;
            };
            for e in &status.entries {
                let gone =
                    [e.staged, e.unstaged].contains(&Some(winc_editor::git::Change::Deleted));
                if gone || e.is_dir || e.conflicted() {
                    continue;
                }
                let taken = match staging {
                    true => commits_staged(e) && e.unstaged.is_none(),
                    false => {
                        let partial = self.git.commit.other_skipped.contains_key(&e.path);
                        self.other_checked(&e.path) && !partial
                    }
                };
                if taken {
                    paths.push((e.path.clone(), repo.clone()));
                }
            }
        }
        paths
    }

    /// 暂存区视图中其他仓库里随当前仓库一并提交的文件数。
    pub(crate) fn other_staged_count(&self) -> usize {
        self.other_staged().iter().map(|(_, count)| count).sum()
    }

    /// 暂存区视图中有内容随当前仓库一并提交的其他仓库：根目录，是否附加签署行。
    pub(super) fn other_staged_roots(&self) -> Vec<(PathBuf, bool)> {
        let roots = self.other_staged().into_iter();
        roots
            .map(|(root, _)| {
                let sign_off = self.state.sign_off_repos.contains(&root);
                (root, sign_off)
            })
            .collect()
    }

    /// 当前按下「提交」不会提交的原因（见 [`CommitState::blocker`]），计入其他仓库暂存的内容。
    pub(crate) fn commit_blocker(&self, status: &Status) -> Option<String> {
        let others = match self.git.commit.staging() {
            true => self.other_staged_count(),
            false => self.other_committed_count(),
        };
        self.git.commit.blocker(status, others)
    }

    /// 点了其他仓库文件行或分组行行尾的「＋」「−」：界面立即变化，命令在后台按顺序执行。
    ///
    /// 冲突中的文件不在这里处理：切换到该仓库解决后再暂存。
    pub(super) fn press_other_stage(&mut self, target: CommitTarget) {
        let (repo, entry, on) = match target {
            CommitTarget::OtherStage(repo, entry, on) => (repo, Some(entry), on),
            CommitTarget::OtherGroupStage(repo, on) => (repo, None, on),
            _ => return,
        };
        let Some(root) = self.git.repositories.get(repo).cloned() else {
            return;
        };
        let mut touched = Vec::new();
        {
            let mut summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
            let Some(status) = summaries.get_mut(&root) else {
                return;
            };
            let entries = status.entries.iter_mut().enumerate();
            let wanted = entries.filter(|(i, e)| entry.is_none_or(|n| n == *i) && !e.conflicted());
            for (_, e) in wanted {
                staging::flip_staged(e, on, &mut touched);
            }
        }
        self.git.summaries_changed();
        if touched.is_empty() {
            return;
        }
        (self.git.commit.other_stage_queue).push_back((root, on, touched));
        self.pump_other_staging();
        self.request_redraw();
    }

    /// 队列中有等待的操作、且没有操作正在执行时，在后台执行下一步。
    fn pump_other_staging(&mut self) {
        if self.git.commit.other_stage_running || self.git.root.is_none() {
            return;
        }
        let Some((other, on, paths)) = self.git.commit.other_stage_queue.pop_front() else {
            return;
        };
        self.git.commit.other_stage_running = true;
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, _| {
            let result = git.and_then(|git| match on {
                true => git.stage(&other, &paths).map(drop),
                false => git.unstage(&other, &paths),
            });
            let _ = sender.send(Outcome::OtherStaged(result));
        });
    }

    /// 写入其他仓库暂存区的一步操作结束：继续下一步；全部完成后重读各仓库的状态，并执行
    /// 等待中的提交。
    pub(super) fn other_staging_finished(&mut self, result: Result<(), String>) {
        self.git.commit.other_stage_running = false;
        if let Err(e) = result {
            // 界面已按成功的结果显示：放弃其余的操作，按各仓库的实际状态重新显示。
            self.git.commit.other_stage_queue.clear();
            self.git.commit.commit_queued = false;
            self.notify(t!("git-stage-failed", error = e));
        }
        if !self.git.commit.other_stage_queue.is_empty() {
            self.pump_other_staging();
            return;
        }
        // 这期间可能已经换到了其中一个仓库：当前仓库的状态和其他仓库的一起重读。
        self.git_read_summaries();
        self.git_refresh();
        if std::mem::take(&mut self.git.commit.commit_queued) {
            self.git_commit();
        }
    }
}
