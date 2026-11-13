//! 位置选择器：从提交视图的位置行就地展开，选在哪个仓库、哪个工作树里工作。
//!
//! 只有一个工作树的仓库占一行；有多个工作树的仓库是一个可以展开的组，默认只展开当前仓库。展开组
//! 只是看一看，选中工作树才真正切换。每一行写着分支、与上游的差异和未提交的更改数，哪里有内容
//! 等着处理一眼能看到。

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use winc_editor::git::{Head, Status, Worktree};
use winc_i18n::t;

use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuLabel, MenuTarget};
use crate::app::input::palette::{needle, rate};
use crate::app::{App, Focus, GitAnchor};

use super::worktree::worktree_name;

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

/// 位置选择器的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationRow {
    /// 有多个工作树的仓库。
    Repository {
        /// 主工作树的目录，收起和展开时用它记。
        path: PathBuf,
        /// 显示的名字。
        label: String,
        /// 工作树数。
        count: usize,
        /// 各工作树未提交的更改数之和。
        changes: usize,
        /// 是否展开。
        open: bool,
    },
    /// 一个位置。
    Place(Place),
}

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

impl App {
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

    /// 从位置行展开位置选择器，选中当前位置，并在后台重新查找一次仓库。
    pub(in crate::app) fn open_location_picker(&mut self) {
        self.git_discover();
        self.git_scan_repositories();
        self.finish_name_input();
        self.overlays.picker.places = None;
        self.overlays.picker.branch_rows = None;
        self.overlays.picker.folders = false;
        self.overlays.picker.vim_list = None;
        self.overlays.picker.remote = None;
        self.overlays.picker.anchor = GitAnchor::Location;
        self.overlays.picker.open = true;
        self.overlays.picker.doc = self.editor.active().map(|d| d.id);
        self.overlays.picker.input.set_text("");
        self.git.location_toggled.clear();
        self.overlays.picker.location_rows = Some(Vec::new());
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 命令「切换仓库」「切换工作树」：打开提交视图，再展开位置选择器。
    pub(in crate::app) fn switch_location(&mut self) {
        if !self.git_has_locations() {
            self.flash(t!("git-location-only-one"));
            return;
        }
        self.show_commit();
        self.paint_now();
        self.open_location_picker();
    }

    /// 按查询重新排列各行，选中当前位置（它不在结果里时是第一个位置）。
    pub(in crate::app) fn location_picker_refresh(&mut self) {
        let rows = self.current_location_rows();
        let current = rows
            .iter()
            .position(|row| matches!(row, LocationRow::Place(place) if place.current));
        let first = rows
            .iter()
            .position(|row| matches!(row, LocationRow::Place(_)));
        self.overlays.picker.selected = current.or(first).unwrap_or(0);
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
    fn location_groups(&self) -> Vec<(String, PathBuf, Vec<Place>)> {
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
    fn current_location_rows(&self) -> Vec<LocationRow> {
        let query = needle(self.overlays.picker.input.text());
        let searching = !query.is_empty();
        let mut rows = Vec::new();
        let groups = self.location_groups();
        // 只有一个仓库时分组标题没有信息量，工作树直接列出来。
        let single = groups.len() == 1;
        for (label, path, mut places) in groups {
            if single {
                places.iter_mut().for_each(|place| place.depth = 0);
            }
            let hit = |place: &Place| {
                !searching
                    || rate(
                        &query,
                        &format!("{label} {} {}", place.label, self.place_head(place)),
                    )
                    .is_some()
            };
            if single || places.len() == 1 {
                rows.extend(
                    places
                        .into_iter()
                        .filter(|p| hit(p))
                        .map(LocationRow::Place),
                );
                continue;
            }
            let shown: Vec<Place> = places.iter().filter(|p| hit(p)).cloned().collect();
            if shown.is_empty() {
                continue;
            }
            let contains_current = places.iter().any(|p| p.current);
            let flipped = self.git.location_toggled.contains(&path);
            let open = searching || contains_current != flipped;
            rows.push(LocationRow::Repository {
                changes: places
                    .iter()
                    .map(|p| self.place_state(&p.path).changes)
                    .sum(),
                count: places.len(),
                path,
                label,
                open,
            });
            if open {
                rows.extend(shown.into_iter().map(LocationRow::Place));
            }
        }
        rows
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

    /// 当前位置以外有未提交的更改的位置数。
    pub(crate) fn git_other_changes(&self) -> usize {
        let groups = self.location_groups();
        let places = groups.iter().flat_map(|(_, _, places)| places);
        places
            .filter(|p| !p.current && !p.missing && self.place_state(&p.path).changes > 0)
            .count()
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

    /// 位置选择器底部「新建工作树」的文字：有多个仓库时写明建在当前仓库。
    pub(crate) fn location_new_worktree_label(&self) -> String {
        match self.git.root.as_deref() {
            Some(root) if self.git.repositories.len() >= 2 => {
                let repository = winc_editor::display_name(root);
                t!("git-location-new-worktree-in", repository = repository)
            }
            _ => t!("git-branch-picker-new-worktree").to_owned(),
        }
    }

    /// 所有位置，顺序和状态栏位置菜单的编号一致（[`App::location_menu`]）。
    pub(in crate::app) fn location_places(&self) -> Vec<Place> {
        (self.location_groups().into_iter())
            .flat_map(|(_, _, places)| places)
            .collect()
    }

    /// 状态栏位置菜单的菜单项：只有一个仓库时直接列工作树，多个仓库时有多个工作树的仓库收进子
    /// 菜单；最后是新建工作树。
    pub(in crate::app) fn location_menu(&self) -> Vec<MenuEntry> {
        let groups = self.location_groups();
        let single = groups.len() == 1;
        let mut next = 0;
        let mut menu = Vec::new();
        for (label, _, places) in groups {
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
                menu.push(MenuEntry::Submenu(label, items));
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

    /// 确认选中的行：组展开或收起，位置切换过去。
    pub(in crate::app) fn location_picker_accept(&mut self) {
        let row = (self.overlays.picker.location_rows.as_ref())
            .and_then(|rows| rows.get(self.overlays.picker.selected))
            .cloned();
        match row {
            Some(LocationRow::Repository { path, .. }) => self.toggle_location_group(path),
            Some(LocationRow::Place(place)) => self.go_to_place(&place),
            None => {}
        }
        self.request_redraw();
    }

    /// 切换到位置 `place`：打开的目录里的仓库只换当前仓库，其余的（别处的工作树）在当前窗口打开
    /// 那个目录，文件树和编辑区跟着换。
    pub(in crate::app) fn go_to_place(&mut self, place: &Place) {
        // 和失效的分支一样：点了弹出它能做的事。
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
            self.git_switch_repository(i);
        } else {
            self.switch_folder(&place.path);
        }
    }

    fn toggle_location_group(&mut self, path: PathBuf) {
        let before = self.picker_fold_before();
        if !self.git.location_toggled.remove(&path) {
            self.git.location_toggled.insert(path);
        }
        let selected = self.overlays.picker.selected;
        self.location_picker_refresh();
        self.overlays.picker.selected =
            selected.min(self.overlays.picker.hits.len().saturating_sub(1));
        self.start_picker_fold(before);
    }

    /// 处理没有查询时的左右键：左键收起组、回到所在的组，右键展开组、打开位置的操作菜单。返回
    /// 是否已处理；有查询时左右键移动输入框的光标。
    pub(in crate::app) fn location_picker_side(&mut self, right: bool) -> bool {
        if !self.picker_lists_locations() || !self.overlays.picker.input.text().is_empty() {
            return false;
        }
        let rows = self
            .overlays
            .picker
            .location_rows
            .as_deref()
            .unwrap_or_default();
        let Some(row) = rows.get(self.overlays.picker.selected).cloned() else {
            return false;
        };
        match (row, right) {
            (LocationRow::Repository { path, open, .. }, _) if open != right => {
                self.toggle_location_group(path);
            }
            (LocationRow::Place(place), true) => {
                self.open_location_row_menu(self.overlays.picker.selected, &place)
            }
            (LocationRow::Place(place), false) if place.depth > 0 => {
                let group = (0..self.overlays.picker.selected)
                    .rev()
                    .find(|&i| matches!(rows[i], LocationRow::Repository { .. }));
                if let Some(group) = group {
                    self.overlays.picker.selected = group;
                    self.picker_reveal();
                }
            }
            _ => {}
        }
        self.request_redraw();
        true
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
