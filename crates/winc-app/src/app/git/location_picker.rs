//! 位置选择器：从提交视图的上下文一行就地展开，选在哪个仓库或哪个工作树里工作。
//!
//! 仓库和工作树各有一个选择器：仓库选择器每个仓库一行，选中即切换到该仓库；
//! 工作树选择器只列当前仓库的工作树，新建工作树的入口在底栏。各行显示分支、
//! 与上游的差异和未提交的更改数。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, PoisonError};

use winc_editor::git::{Branch, Head, LastCommit, Stash, Status, Worktree};
use winc_i18n::t;

use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuLabel, MenuTarget};
use crate::app::input::palette::{needle, rate};
use crate::app::{App, Focus, GitAnchor, UserEvent};

use super::worktree::worktree_name;
use super::{Cancel, SUMMARY_REPOSITORIES};

/// 一个可以工作的位置：只有一个工作树的仓库，或者仓库里的一个工作树。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// 目录。
    pub path: PathBuf,
    /// 显示的名字。
    pub label: String,
    /// 缩进的层数：独立成行的仓库是 0，组里的工作树是 1。
    pub depth: usize,
    /// 在当前仓库的工作树列表（[`GitState::worktrees`](crate::app::git::GitState::worktrees)）
    /// 里的下标；不是当前仓库的工作树时为 `None`。
    pub worktree: Option<usize>,
    /// 目录是否已经不存在。
    pub missing: bool,
    /// 是否是当前的位置。
    pub current: bool,
}

/// 一个仓库的各个位置：仓库的名字、主工作树的目录、位置。
pub(crate) type PlaceGroup = (String, PathBuf, Vec<Place>);
/// 排好的全部位置及其缓存键：Git 状态的代数、各仓库状态的版本、仓库数。
pub(super) type PlacesCache = ((u64, u64, usize), Rc<Vec<PlaceGroup>>, Rc<Vec<Place>>);

/// 一个工作树相对当前 `HEAD` 的差异：据以计算的两个提交号，以及落后和领先的提交数。
pub(super) type Divergence = ((String, String), (u32, u32));

/// 一个位置现在的样子。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaceState {
    /// 分支名，或者游离时的提交号。
    pub head: Option<String>,
    /// 领先上游的提交数。
    pub ahead: u32,
    /// 落后上游的提交数。
    pub behind: u32,
    /// 未提交的更改数。
    pub changes: usize,
}

impl PlaceState {
    fn of(status: &Status) -> PlaceState {
        let head = status.head.as_ref().map(|head| match head {
            Head::Branch(name) => name.clone(),
            Head::Detached(oid) => t!("git-worktree-detached", oid = oid.as_str()),
        });
        PlaceState {
            head,
            ahead: status.ahead,
            behind: status.behind,
            changes: status.entries.len(),
        }
    }
}

/// 离开一个仓库时记下的内容，见 `GitState::kept`。
pub(super) struct KeptRefs {
    pub(super) branches: Vec<Branch>,
    pub(super) stashes: Vec<Stash>,
    pub(super) worktrees: Vec<Worktree>,
    pub(super) last: Option<LastCommit>,
}

impl App {
    /// 切换到一个仓库后恢复离开时记下的分支、储藏、工作树和上一次提交；
    /// 未记录过的仓库使用后台读到的工作树。
    pub(super) fn restore_kept_refs(&mut self) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        // 恢复的内容影响排好的位置（`cached_places`）。
        self.git.generation += 1;
        if let Some(kept) = self.git.kept.remove(&root) {
            self.git.branches = kept.branches;
            self.git.stashes = kept.stashes;
            self.git.worktrees = kept.worktrees;
            self.git.commit.last = kept.last;
            return;
        }
        if self.git.worktrees.is_empty() {
            let known = self.git.summary_worktrees.lock();
            let known = known.unwrap_or_else(PoisonError::into_inner);
            // 仓库是附属工作树时，其工作树列表记在主工作树名下。
            let trees = known
                .values()
                .find(|trees| trees.iter().any(|w| w.path == root));
            self.git.worktrees = trees.cloned().unwrap_or_default();
        }
    }

    /// 面板是否正在列出位置。
    pub(crate) fn picker_lists_locations(&self) -> bool {
        self.overlays.picker.location_rows.is_some()
    }

    /// 目录下有不止一个可以工作的位置时才需要位置行：一个仓库、一个工作树时没有可选的。
    pub(crate) fn git_has_locations(&self) -> bool {
        let usable = |w: &Worktree| w.head.is_some();
        self.git.repositories.len() >= 2
            || self.git.worktrees.iter().filter(|w| usable(w)).count() >= 2
    }

    /// 当前仓库是否有多个可用的工作树。
    pub(crate) fn git_has_worktrees(&self) -> bool {
        let usable = self.git.worktrees.iter().filter(|w| w.head.is_some());
        usable.count() >= 2
    }

    /// 面板是否正在列出仓库。
    pub(crate) fn picker_lists_repositories(&self) -> bool {
        self.picker_lists_locations() && self.git.location_all
    }

    /// 面板是否正在列出当前仓库的工作树。
    pub(crate) fn picker_lists_worktrees(&self) -> bool {
        self.picker_lists_locations() && !self.git.location_all
    }

    /// 展开选择器并选中当前位置，同时在后台重新查找一次仓库；`all` 见
    /// [`GitState::location_all`](crate::app::git::GitState::location_all)。
    pub(in crate::app) fn open_places(&mut self, all: bool) {
        self.git.location_all = all;
        self.git_discover();
        self.git_scan_repositories();
        self.finish_name_input();
        self.overlays.picker.places = None;
        self.overlays.picker.branch_rows = None;
        self.overlays.picker.folders = false;
        self.overlays.picker.languages = false;
        self.overlays.picker.vim_list = None;
        self.overlays.picker.remote = None;
        self.overlays.picker.anchor = GitAnchor::Location;
        self.overlays.picker.open = true;
        self.overlays.picker.doc = self.editor.active().map(|d| d.id);
        self.overlays.picker.input.set_text("");
        self.overlays.picker.location_rows = Some(Vec::new());
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 打开提交视图并展开选择器：`all` 为真时列出各仓库，否则列出当前仓库的工作树。
    pub(in crate::app) fn switch_location(&mut self, all: bool) {
        let any = if all {
            self.git.repositories.len() >= 2
        } else {
            self.git_has_worktrees()
        };
        if !any {
            self.flash(t!("git-location-only-one"));
            return;
        }
        self.show_commit();
        self.paint_now();
        self.open_places(all);
    }

    /// 按查询重新排列各行，选中当前位置（它不在结果里时是第一个位置）。
    pub(in crate::app) fn location_picker_refresh(&mut self) {
        let rows = self.current_location_rows();
        let current = rows.iter().position(|place| place.current);
        self.overlays.picker.selected = current.unwrap_or(0);
        self.overlays.picker.hits = (0..rows.len()).collect();
        self.overlays.picker.location_rows = Some(rows);
    }

    /// 后台找到了新的仓库、工作树：重新排列。
    pub(in crate::app) fn location_picker_refreshed(&mut self) {
        if self.picker_lists_locations() {
            self.location_picker_refresh();
            self.relayout();
        }
    }

    /// 仓库 `repo` 的工作树；不知道时（还没读到）是空的。
    fn worktrees_of(&self, repo: &Path) -> Vec<Worktree> {
        let current = self.git.worktrees.iter().any(|w| w.path == repo);
        if current || self.git.root.as_deref() == Some(repo) {
            return self.git.worktrees.clone();
        }
        (self.git.summary_worktrees.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .get(repo)
            .cloned()
            .unwrap_or_default()
    }

    /// 全部位置，按仓库分组：（仓库的名字，主工作树的目录，位置）。
    ///
    /// 工作树放在打开的目录里时，它本身也会被当作仓库找到：只算在它所属的仓库下面。
    fn location_groups(&self) -> Rc<Vec<PlaceGroup>> {
        self.cached_places().0
    }

    /// 当前仓库的各个工作树，主工作树在前；目录已不在的也在其中（`missing`）。
    pub(crate) fn worktree_places(&self) -> Rc<Vec<Place>> {
        self.cached_places().1
    }

    /// 返回全部位置和其中当前仓库的各个工作树，按 Git 状态和各仓库状态的版本缓存。
    fn cached_places(&self) -> (Rc<Vec<PlaceGroup>>, Rc<Vec<Place>>) {
        let key = (
            self.git.generation,
            self.git.summaries_version(),
            self.git.repositories.len(),
        );
        let mut cache = self.git.places.borrow_mut();
        if let Some((cached, groups, own)) = cache.as_ref()
            && *cached == key
        {
            return (Rc::clone(groups), Rc::clone(own));
        }
        let groups = self.build_location_groups();
        let own = groups
            .iter()
            .find(|(_, _, places)| places.iter().any(|p| p.current));
        let own = Rc::new(own.map(|(_, _, places)| places.clone()).unwrap_or_default());
        let groups = Rc::new(groups);
        *cache = Some((key, Rc::clone(&groups), Rc::clone(&own)));
        (groups, own)
    }

    /// 排出全部位置，见 [`App::location_groups`]。
    fn build_location_groups(&self) -> Vec<PlaceGroup> {
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut groups = Vec::new();
        let root = self.git.root.as_deref();
        let current_trees = self
            .git
            .worktrees
            .iter()
            .map(|w| &w.path)
            .collect::<Vec<_>>();
        let repositories = (self.git.repositories.iter().enumerate())
            .map(|(i, repo)| (self.git_repository_name(i), repo.clone()));
        for (name, repo) in repositories {
            if seen.contains(&repo) {
                continue;
            }
            let trees = self.worktrees_of(&repo);
            let usable: Vec<&Worktree> = trees.iter().filter(|w| w.head.is_some()).collect();
            if usable.len() < 2 {
                seen.push(repo.clone());
                groups.push((
                    name.clone(),
                    repo.clone(),
                    vec![Place {
                        current: root == Some(repo.as_path()),
                        worktree: current_trees.iter().position(|p| **p == repo),
                        path: repo,
                        label: name,
                        depth: 0,
                        missing: false,
                    }],
                ));
                continue;
            }
            seen.extend(usable.iter().map(|w| w.path.clone()));
            // 打开的就是附属工作树时，仓库用主工作树的目录名。
            let main = usable[0];
            let label = if main.path == repo {
                name
            } else {
                worktree_name(main)
            };
            let places = usable
                .iter()
                .map(|w| Place {
                    current: root == Some(w.path.as_path()),
                    worktree: current_trees.iter().position(|p| **p == w.path),
                    path: w.path.clone(),
                    label: if w.path == main.path {
                        t!("git-location-main-worktree").to_owned()
                    } else {
                        worktree_name(w)
                    },
                    depth: 1,
                    missing: w.prunable,
                })
                .collect();
            groups.push((label, main.path.clone(), places));
        }
        groups
    }

    /// 排出位置选择器的各行。
    fn current_location_rows(&self) -> Vec<Place> {
        let query = needle(self.overlays.picker.input.text());
        let searching = !query.is_empty();
        let groups = (*self.location_groups()).clone();
        // 列出仓库时每个仓库一行：当前仓库取所在的工作树，其他仓库取主工作树。
        if self.git.location_all {
            let rows = groups.into_iter().filter_map(|(label, _, places)| {
                let at = places.iter().position(|p| p.current).unwrap_or(0);
                let place = places.into_iter().nth(at)?;
                let head = self.place_head(&place);
                let hit = !searching || rate(&query, &format!("{label} {head}")).is_some();
                hit.then_some(Place {
                    label,
                    depth: 0,
                    ..place
                })
            });
            return rows.collect();
        }
        // 列出工作树时只列当前仓库的，每个工作树一行。
        let own = groups
            .into_iter()
            .find(|(_, _, places)| places.iter().any(|p| p.current));
        let Some((label, _, places)) = own else {
            return Vec::new();
        };
        let hit = |place: &Place| {
            let text = format!("{label} {} {}", place.label, self.place_head(place));
            !searching || rate(&query, &text).is_some()
        };
        let shown = places.into_iter().filter(|place| hit(place));
        shown.map(|place| Place { depth: 0, ..place }).collect()
    }

    fn place_head(&self, place: &Place) -> String {
        self.place_state(&place.path).head.unwrap_or_default()
    }

    /// 目录 `path` 这个位置现在的样子；还没读到时是默认值。当前位置用最新的状态。
    pub(crate) fn place_state(&self, path: &Path) -> PlaceState {
        if self.git.root.as_deref() == Some(path)
            && let Some(status) = &self.git.status
        {
            return PlaceState::of(status);
        }
        (self.git.summaries.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .map(PlaceState::of)
            .unwrap_or_default()
    }

    /// 当前仓库的其他工作树中有未提交的更改的个数。
    pub(crate) fn git_other_changes(&self) -> usize {
        let groups = self.location_groups();
        let own = (groups.iter()).filter(|(_, _, places)| places.iter().any(|p| p.current));
        let places = own.flat_map(|(_, _, places)| places);
        places
            .filter(|p| !p.current && !p.missing && self.place_state(&p.path).changes > 0)
            .count()
    }

    /// 在后台读取各仓库的工作树以及每个工作树的分支和改动数，供位置选择器和位置行显示。
    ///
    /// 只有一个位置时不读取。
    ///
    /// 最多读 [`SUMMARY_REPOSITORIES`] 个位置。
    pub(in crate::app) fn git_read_summaries(&mut self) {
        if let Some(cancel) = self.git.summaries_running.take() {
            cancel.cancel();
        }
        if !self.git_has_locations() {
            return;
        }
        let cancel = Cancel::default();
        self.git.summaries_running = Some(cancel.clone());
        let repositories = self.git.repositories.clone();
        let summaries = Arc::clone(&self.git.summaries);
        let version = Arc::clone(&self.git.summaries_version);
        let worktrees = Arc::clone(&self.git.summary_worktrees);
        let divergence = Arc::clone(&self.git.summary_divergence);
        let current = self.git.root.clone();
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            let Ok(git) = tool else {
                return;
            };
            // 各仓库的状态读完后一次写入，列表只重排一次；中途取消时写入已读到的部分。
            let mut read: Vec<(PathBuf, Status)> = Vec::new();
            let flush = |read: Vec<(PathBuf, Status)>| {
                let mut summaries = summaries.lock().unwrap_or_else(PoisonError::into_inner);
                summaries.extend(read);
                version.fetch_add(1, std::sync::atomic::Ordering::Release);
            };
            let mut left = SUMMARY_REPOSITORIES;
            'repositories: for root in repositories {
                let trees = git.worktrees(&root, &cancel).unwrap_or_default();
                let paths: Vec<PathBuf> = if trees.is_empty() {
                    vec![root.clone()]
                } else {
                    let usable = trees.iter().filter(|w| w.head.is_some() && !w.prunable);
                    usable.map(|w| w.path.clone()).collect()
                };
                // 当前仓库的其他工作树各有多少提交不在当前分支上；提交号未变时沿用上次的结果。
                let here = current
                    .as_deref()
                    .and_then(|c| trees.iter().find(|w| w.path == c));
                if let Some((here, base)) = here.and_then(|w| Some((w, w.oid.clone()?))) {
                    // 计算期间界面读取上次的结果，计算完成后一次替换。
                    let old = divergence
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    let mut fresh = HashMap::new();
                    let others = trees.iter().filter(|w| w.path != here.path && !w.prunable);
                    for (tree, tip) in others.filter_map(|w| Some((w, w.oid.clone()?))) {
                        let basis = (base.clone(), tip);
                        let known = old.get(&tree.path).filter(|(b, _)| *b == basis);
                        let count = match known {
                            Some((_, count)) => Some(*count),
                            None => git.divergence(&here.path, &basis.0, &basis.1, &cancel),
                        };
                        if let Some(count) = count {
                            fresh.insert(tree.path.clone(), (basis, count));
                        }
                    }
                    *divergence.lock().unwrap_or_else(PoisonError::into_inner) = fresh;
                    version.fetch_add(1, std::sync::atomic::Ordering::Release);
                }
                worktrees
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(root, trees);
                for path in paths {
                    if cancel.is_cancelled() || left == 0 {
                        break 'repositories;
                    }
                    left -= 1;
                    if let Ok(status) = git.status(&path, &cancel) {
                        read.push((path, status));
                    }
                }
            }
            flush(read);
        });
    }

    /// 返回工作树 `path` 相对当前 `HEAD` 落后和领先的提交数；未读到时为 `None`。
    pub(crate) fn worktree_divergence(&self, path: &Path) -> Option<(u32, u32)> {
        let counts = self.git.summary_divergence.lock();
        let counts = counts.unwrap_or_else(PoisonError::into_inner);
        counts.get(path).map(|(_, count)| *count)
    }

    /// 工作树 `place` 的分支是否已合并到当前分支：没有不在当前分支上的提交（判断同
    /// `git branch --merged`），并且没有未提交的更改。主工作树、当前工作树和锁定的工作树除外。
    pub(crate) fn worktree_merged(&self, place: &Place) -> bool {
        let linked = place.worktree.filter(|i| *i > 0);
        let tree = linked.and_then(|i| self.git.worktrees.get(i));
        tree.is_some_and(|w| !w.locked)
            && !place.current
            && !place.missing
            && self.place_state(&place.path).changes == 0
            && matches!(self.worktree_divergence(&place.path), Some((_, 0)))
    }

    /// 位置行上写的内容：仓库的名字，以及打开的是附属工作树时工作树的名字。
    ///
    /// 项目名已经写了仓库时位置行只写工作树：主工作树写「主工作树」，附属工作树写它自己的名字。
    pub(crate) fn git_location_label(&self) -> (String, Option<String>) {
        let groups = self.location_groups();
        let current = groups.iter().find_map(|(label, _, places)| {
            let place = places.iter().find(|p| p.current)?;
            let main = self.git.worktrees.first().map(|w| w.path.as_path());
            let project = self.editor.workspace.as_ref().map(|w| w.name.as_str());
            if place.depth > 0 && Some(place.path.as_path()) != main {
                if project == Some(place.label.as_str()) {
                    return Some((place.label.clone(), None));
                }
                return Some((label.clone(), Some(place.label.clone())));
            }
            let name = if places.len() > 1 && project == Some(label.as_str()) {
                &place.label
            } else {
                label
            };
            Some((name.clone(), None))
        });
        current.unwrap_or_default()
    }

    /// 所有位置，顺序和状态栏位置菜单的编号一致（[`App::location_menu`]）。
    pub(in crate::app) fn location_places(&self) -> Vec<Place> {
        let groups = self.location_groups();
        let places = groups.iter().flat_map(|(_, _, places)| places);
        places.cloned().collect()
    }

    /// 状态栏位置菜单的菜单项：只有一个仓库时直接列工作树，多个仓库时有多个工作树的仓库收进子
    /// 菜单；最后是新建工作树。
    pub(in crate::app) fn location_menu(&self) -> Vec<MenuEntry> {
        let groups = self.location_groups();
        let single = groups.len() == 1;
        let mut next = 0;
        let mut menu = Vec::new();
        for (label, _, places) in groups.iter() {
            let items: Vec<MenuEntry> = (places.iter())
                .map(|place| {
                    let item = MenuEntry::Item(MenuCommand::StatusPlace(next), !place.missing);
                    next += 1;
                    item
                })
                .collect();
            if single || items.len() == 1 {
                menu.extend(items);
            } else {
                menu.push(MenuEntry::Submenu(label.clone(), items));
            }
        }
        menu.extend([
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::NewWorktree, true),
        ]);
        menu
    }

    /// 状态栏位置菜单第 `i` 项的文字：名字，下面一行是分支。
    pub(in crate::app) fn location_menu_label(&self, i: usize) -> MenuLabel {
        let places = self.location_places();
        let Some(place) = places.get(i) else {
            return MenuLabel::plain("");
        };
        let subtitle = if place.missing {
            Some(t!("git-worktree-missing").to_owned())
        } else {
            self.place_state(&place.path).head
        };
        MenuLabel {
            subtitle,
            ..MenuLabel::plain(&place.label)
        }
    }

    /// 确认选中的行：切换到这个位置。
    pub(in crate::app) fn location_picker_accept(&mut self) {
        let rows = self.overlays.picker.location_rows.as_ref();
        let place = rows
            .and_then(|rows| rows.get(self.overlays.picker.selected))
            .cloned();
        if let Some(place) = place {
            self.go_to_place(&place);
        }
        self.request_redraw();
    }

    /// 切换到位置 `place`：打开的目录里的仓库只换当前仓库，其余的（别处的工作树）在当前窗口打开
    /// 那个目录，文件树和编辑区跟着换。
    pub(in crate::app) fn go_to_place(&mut self, place: &Place) {
        // 和失效的分支一样：点击弹出它能做的事。
        if place.missing {
            if place.worktree.is_some() {
                self.open_location_row_menu(self.overlays.picker.selected, place);
            } else {
                self.flash(t!("git-worktree-missing"));
            }
            return;
        }
        self.close_picker();
        if place.current {
            return;
        }
        if let Some(i) = self.git.repositories.iter().position(|r| *r == place.path) {
            let scroll = self.git.commit.scroll;
            self.git_switch_repository(i);
            self.keep_commit_scroll(scroll);
        } else if let Some(i) = place.worktree {
            self.open_worktree_asking(i);
        } else {
            self.switch_folder(&place.path);
        }
    }

    /// 处理没有查询时的左右键，返回是否已处理：右键打开选中位置的操作菜单。
    pub(in crate::app) fn location_picker_side(&mut self, right: bool) -> bool {
        if !self.picker_lists_locations() || !self.overlays.picker.input.text().is_empty() {
            return false;
        }
        let rows = self.overlays.picker.location_rows.as_deref();
        let place = rows
            .and_then(|rows| rows.get(self.overlays.picker.selected))
            .cloned();
        let Some(place) = place else {
            return false;
        };
        if right {
            self.open_location_row_menu(self.overlays.picker.selected, &place);
        }
        self.request_redraw();
        true
    }

    /// 返回位置 `path` 的操作菜单；`worktree` 是它在当前仓库工作树列表中的下标。
    pub(in crate::app) fn location_row_menu(
        &self,
        path: &Path,
        worktree: Option<usize>,
    ) -> Vec<MenuEntry> {
        let tree = worktree.and_then(|i| self.git.worktrees.get(i));
        let idle = self.git.branch.running.is_none();
        // 目录已不存在：只能清理。
        if tree.is_some_and(|w| w.prunable) {
            return vec![MenuEntry::Item(MenuCommand::LocationPrune, idle)];
        }
        let mut menu = vec![MenuEntry::Item(MenuCommand::LocationNewWindow, true)];
        // 远端主机上的目录无法在本机的文件管理器中显示。
        if !winc_backend::is_remote(path) {
            menu.push(MenuEntry::Item(MenuCommand::LocationReveal, true));
        }
        // 只有工作树可以在此窗口中打开；当前所在的工作树该项不可用。
        if tree.is_some() {
            let here = self.git.root.as_deref() == Some(path);
            menu.insert(0, MenuEntry::Item(MenuCommand::LocationThisWindow, !here));
        }
        let other = tree.filter(|_| self.git.current_worktree() != worktree);
        // 其他工作树有提交不在当前分支上时可以合并。
        if let Some(other) = other.filter(|w| w.branch().is_some()) {
            let ahead = self.worktree_divergence(&other.path);
            let any = ahead.is_none_or(|(_, ahead)| ahead > 0);
            menu.push(MenuEntry::Separator);
            menu.push(MenuEntry::Item(MenuCommand::LocationMerge, idle && any));
        }
        if worktree.is_some_and(|i| i > 0) {
            let removable = other.is_some_and(|w| !w.locked) && idle;
            menu.push(MenuEntry::Separator);
            menu.push(MenuEntry::Item(MenuCommand::LocationRemove, removable));
        }
        // 仓库选择器中的仓库：可以从列表中移除，列表里至少留一个。
        if self.git.location_all && self.git.repositories.iter().any(|r| r == path) {
            let more = self.git.repositories.len() >= 2;
            menu.push(MenuEntry::Separator);
            menu.push(MenuEntry::Item(MenuCommand::LocationForget, more));
        }
        menu
    }

    /// 在第 `row` 行的右端弹出位置 `place` 的操作菜单。
    pub(in crate::app) fn open_location_row_menu(&mut self, row: usize, place: &Place) {
        let parts = crate::app::workspace::picker::picker_parts(self, self.layout.picker);
        let rect = parts.row(row, self.overlays.picker.scroll);
        let target = MenuTarget::Location(place.path.clone(), place.worktree);
        self.open_context_menu(target, (rect.right() - 14.0, rect.bottom()));
    }
}

#[cfg(test)]
mod tests;
