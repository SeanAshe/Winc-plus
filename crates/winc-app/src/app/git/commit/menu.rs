//! 提交视图的菜单：「提交」按钮旁的菜单、「⋯」菜单和文件行的菜单，以及各项对应的命令。

use super::*;
use crate::app::PickerPart;
use crate::keymap::Action;

impl App {
    /// 提交视图标题行「⋯」的菜单，自上而下：进行中的操作的继续、跳过和中止；拉取、推送、获取；
    /// 储藏更改、远程仓库（见 [`branch_menu`]），提交、更改、分支、标签、工作树、仓库几个子菜单
    /// （更改一项的内容随视图不同）；以树形显示（排成
    /// 树时打勾）、显示暂存区、显示全部仓库（有多个仓库时才有）、全部展开、全部折叠（已经是那样
    /// 的灰显）；显示 Git 日志、显示 Git 输出。
    ///
    /// 三个子菜单里是命令面板中已有的命令，快捷键相同。刷新是标题行上的按钮，回滚勾选的文件在
    /// 组标题上，这里不重复。
    pub(in crate::app) fn commit_more_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::MenuEntry;
        // 没有文件的组不显示，不算在内。
        let shown = || {
            let status = self.git.status.as_ref();
            CommitGroup::ALL.into_iter().filter(move |group| {
                status.is_some_and(|s| {
                    s.entries
                        .iter()
                        .any(|e| group.holds(e, self.git.commit.staging()))
                })
            })
        };
        let all_collapsed = shown().all(|group| self.git.commit.group_collapsed(group));
        let all_expanded = shown().all(|group| !self.git.commit.group_collapsed(group))
            && self.git.commit.collapsed_dirs.iter().all(HashSet::is_empty);
        let busy = self.git.branch.running.is_some();
        let mut entries = branch_menu(
            &self.git.remote_list,
            self.git.status.as_ref().and_then(|s| s.operation),
            busy,
        );
        // 与远程交换提交的三项排在最前（进行中的操作之后）：不用先展开分支选择面板。
        let sync = [
            MenuCommand::GitPull,
            MenuCommand::GitPush,
            MenuCommand::GitFetch,
        ];
        let at = (entries.iter())
            .position(|entry| *entry == MenuEntry::Item(MenuCommand::GitStash, !busy))
            .unwrap_or(0);
        let enabled = |command| match command {
            MenuCommand::GitPull => self.picker_action_enabled(PickerPart::Pull),
            MenuCommand::GitPush => self.picker_action_enabled(PickerPart::Push),
            _ => self.picker_action_enabled(PickerPart::Fetch),
        };
        let quick = sync.map(|command| MenuEntry::Item(command, enabled(command)));
        entries.splice(at..at, quick.into_iter().chain([MenuEntry::Separator]));
        // 分支、工作树、仓库各一个子菜单，里面是命令面板中已有的命令。
        let item = |action: Action| MenuEntry::Item(MenuCommand::GitAction(action), true);
        let branches = vec![
            item(Action::GitSwitchBranch),
            item(Action::GitNewBranch),
            item(Action::GitGoToRevision),
        ];
        let mut worktrees = vec![item(Action::GitNewWorktree)];
        if self.git_has_worktrees() {
            worktrees.insert(0, item(Action::GitSwitchWorktree));
        }
        let mut repositories = vec![item(Action::GitClone)];
        if self.git.repositories.len() >= 2 {
            repositories.insert(0, item(Action::GitSwitchRepository));
        }
        // 提交：提交和提交并推送，修改与撤销上次提交，签署和跳过钩子，两种视图都有。
        let status = self.git.status.as_ref();
        let ready = status.is_some_and(|s| self.commit_blocker(s).is_none());
        let mut commits = vec![MenuEntry::Item(MenuCommand::CommitRun, ready && !busy)];
        // 这里是完整的命令列表：卡片里有并排的推送按钮时也列出「提交并推送」。
        commits.extend(self.commit_menu_items(true));
        let amend = MenuEntry::Item(MenuCommand::CommitAmend, self.commit_amendable());
        if !commits
            .iter()
            .any(|e| matches!(e, MenuEntry::Item(MenuCommand::CommitAmend, _)))
        {
            commits.extend([MenuEntry::Separator, amend]);
        }
        let undo = MenuCommand::CommitUndo;
        commits.push(MenuEntry::Item(undo, self.commit_undoable().is_some()));
        // 更改：暂存视图中是全部暂存、全部取消暂存，变更列表中是全部勾选、全部取消勾选；放弃
        // 所有更改两种视图都有。
        let changed = status.is_some_and(|s| !s.entries.is_empty());
        let tracked = status.is_some_and(|s| s.entries.iter().any(|e| !e.untracked()));
        let mut changes = if self.git.commit.staging() {
            vec![item(Action::GitStageAll), item(Action::GitUnstageAll)]
        } else {
            vec![
                MenuEntry::Item(MenuCommand::CommitCheckAll(true), changed),
                MenuEntry::Item(MenuCommand::CommitCheckAll(false), changed),
            ]
        };
        changes.extend([
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::CommitDiscardAll, tracked && !busy),
        ]);
        let head = status.is_some_and(|s| s.oid.is_some());
        let tags = vec![MenuEntry::Item(MenuCommand::LogNewTag, head && !busy)];
        entries.extend([
            MenuEntry::Submenu(t!("menu-commit-commits").to_owned(), commits),
            MenuEntry::Submenu(t!("menu-commit-changes").to_owned(), changes),
            MenuEntry::Submenu(t!("menu-git-branches").to_owned(), branches),
            MenuEntry::Submenu(t!("menu-git-tags").to_owned(), tags),
            MenuEntry::Submenu(t!("menu-git-worktrees").to_owned(), worktrees),
            MenuEntry::Submenu(t!("menu-git-repositories").to_owned(), repositories),
        ]);
        // 显示与排序：三组单选（列表或树形、变更列表或暂存区、排序方式），下面是只在这种显示下
        // 有用的几项。
        let mut view = vec![
            MenuEntry::Item(MenuCommand::CommitList, true),
            MenuEntry::Item(MenuCommand::CommitTree, true),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::CommitChangeList, true),
            MenuEntry::Item(MenuCommand::CommitStagingArea, true),
            MenuEntry::Separator,
        ];
        let sorts = CommitSort::ALL.map(MenuCommand::CommitSortBy);
        view.extend(sorts.map(|command| MenuEntry::Item(command, true)));
        view.push(MenuEntry::Separator);
        // 打开的目录中有多个仓库时才有：只有一个仓库时没有可以列出的其他仓库。
        if self.git.repositories.len() >= 2 {
            let all = MenuCommand::CommitAllRepositories;
            view.push(MenuEntry::Item(all, true));
        }
        view.extend([
            MenuEntry::Item(MenuCommand::CommitExpandAll, !all_expanded),
            MenuEntry::Item(MenuCommand::CommitCollapseAll, !all_collapsed),
        ]);
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::Submenu(t!("menu-commit-view").to_owned(), view),
            MenuEntry::Separator,
            item(Action::GitShowLog),
            MenuEntry::Item(MenuCommand::GitShowOutput, true),
        ]);
        entries
    }

    /// 「提交 ⌄」拆分按钮右边箭头的菜单：提交并推送（只在卡片里没有并排的「提交并推送…」按钮时
    /// 列出，不重复；合并、变基停着时没有推送这回事，没有要提交的更改时与主按钮一样不能提交，
    /// 都灰显）；随后是签署和本次跳过钩子两个开关。
    pub(in crate::app) fn commit_button_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        self.commit_menu_items(self.commit_parts().push.is_none())
    }

    /// 提交按钮菜单的各项；`push` 为假时不列「提交并推送」。
    fn commit_menu_items(&self, push: bool) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        let status = self.git.status.as_ref();
        let idle = !self.git.commit.busy && status.is_none_or(|s| s.operation.is_none());
        let clean = status.is_some_and(|s| s.entries.is_empty());
        let mut entries = Vec::new();
        let enabled = idle && (!clean || self.git.commit.amend);
        if push {
            entries.push(MenuEntry::Item(MenuCommand::CommitPush, enabled));
        }
        entries.extend([
            MenuEntry::Item(MenuCommand::CommitSync, enabled),
            MenuEntry::Separator,
        ]);
        entries.extend([
            MenuEntry::Item(MenuCommand::CommitSignOff, true),
            MenuEntry::Item(MenuCommand::CommitNoVerify, idle),
        ]);
        entries.extend(self.compose_menu_entries());
        entries
    }

    /// 当前仓库提交时是否附加签署行（在提交按钮的菜单中打开，按仓库记在状态文件里）。
    pub(in crate::app) fn commit_signs_off(&self) -> bool {
        (self.git.root.as_ref()).is_some_and(|root| self.state.sign_off_repos.contains(root))
    }

    /// 打开或关闭当前仓库提交时的签署行。
    fn toggle_sign_off(&mut self) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        self.update_state(|state| {
            let repos = &mut state.sign_off_repos;
            match repos.iter().position(|repo| *repo == root) {
                Some(at) => drop(repos.remove(at)),
                None => repos.push(root),
            }
        });
    }

    /// 「提交 ⌄」的菜单选了 `command`。
    pub(in crate::app) fn run_commit_button_command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::CommitPush => self.confirm_commit_and_push(),
            MenuCommand::CommitSync => self.confirm_commit_and_sync(),
            MenuCommand::CommitAmend => self.toggle_amend(),
            MenuCommand::CommitSignOff => self.toggle_sign_off(),
            MenuCommand::CommitCheckTodo => self.toggle_check_todo(),
            MenuCommand::CommitReformat => self.toggle_reformat(),
            MenuCommand::CommitMessage(i) => self.use_commit_message(i),
            MenuCommand::CommitCoAuthor(i) => self.add_co_author(i),
            MenuCommand::CommitNoVerify => self.git.commit.no_verify = !self.git.commit.no_verify,
            _ => {}
        }
        self.request_redraw();
    }

    /// 提交勾上的文件，成功以后继续推送。
    pub(in crate::app) fn git_commit_and_push(&mut self) {
        self.git_commit_then(false);
    }

    /// 提交勾上的文件，成功以后与上游同步（先拉取再推送）。
    pub(in crate::app) fn git_commit_and_sync(&mut self) {
        self.git_commit_then(true);
    }

    /// 提交，成功以后推送（`sync` 时同步）。
    pub(super) fn git_commit_then(&mut self, sync: bool) {
        self.git.commit.push_after = true;
        self.git.commit.sync_after = sync;
        self.git_commit();
        // 没有真的开始提交（没写信息、没勾文件），就不留着「之后推送」。
        if !self.git.commit.busy {
            self.git.commit.push_after = false;
            self.git.commit.sync_after = false;
        }
    }

    /// 在按目录排成树和平铺之间切换更改列表。
    ///
    /// 记进状态文件：以后打开的窗口、下次启动都用同一种显示方式。
    pub(in crate::app) fn toggle_commit_layout(&mut self) {
        let flat = !self.git.commit.flat();
        self.git.commit.set_flat(flat);
        self.update_state(|state| state.commit_tree = !flat);
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
        self.reveal_commit_selection();
        self.sync_log_diff_layout();
        self.request_redraw();
    }

    /// 提交视图标题行「⋯」的菜单选了 `command`。
    pub(in crate::app) fn run_commit_more_command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::CommitTree | MenuCommand::CommitList => {
                if self.git.commit.flat() != (command == MenuCommand::CommitList) {
                    self.toggle_commit_layout();
                }
            }
            MenuCommand::CommitAllRepositories => self.toggle_all_repositories(),
            MenuCommand::CommitStagingArea | MenuCommand::CommitChangeList => {
                if self.git.commit.staging() != (command == MenuCommand::CommitStagingArea) {
                    self.toggle_staging_area();
                }
            }
            // 中止和跳过都会丢弃内容：从菜单里选也先确认，与说明条上的「中止」按钮相同。
            MenuCommand::GitAbort => self.confirm_abort(),
            MenuCommand::GitSkip => self.confirm_skip(),
            MenuCommand::CommitRun => self.git_commit(),
            MenuCommand::CommitPush
            | MenuCommand::CommitSync
            | MenuCommand::CommitAmend
            | MenuCommand::CommitSignOff
            | MenuCommand::CommitNoVerify => self.run_commit_button_command(command),
            MenuCommand::CommitUndo => self.run_last_commit_command(command),
            MenuCommand::CommitCheckAll(on) => {
                let entries = self.git.status.iter().flat_map(|s| &s.entries);
                let paths: Vec<PathBuf> = entries.map(|e| e.path.clone()).collect();
                self.set_commit_checked(&paths, on);
            }
            MenuCommand::CommitDiscardAll => {
                let entries = self.git.status.iter().flat_map(|s| &s.entries);
                let tracked = entries.filter(|e| !e.untracked() && !e.conflicted());
                let paths: Vec<PathBuf> = tracked.map(|e| e.path.clone()).collect();
                if !paths.is_empty() {
                    self.confirm_rollback(paths);
                }
            }
            MenuCommand::LogNewTag => {
                let oid = self.git.status.as_ref().and_then(|s| s.oid.clone());
                if let Some(oid) = oid {
                    self.open_tag_prompt(oid);
                }
            }
            MenuCommand::CommitSortBy(sort) => {
                self.git.commit.set_sort(sort);
                self.update_state(|state| state.commit_sort = sort.key().to_owned());
                self.reveal_commit_selection();
            }
            MenuCommand::GitShowOutput => self.show_git_output(),
            MenuCommand::GitAction(action) => {
                self.run_action(action);
            }
            MenuCommand::CommitExpandAll | MenuCommand::CommitCollapseAll => {
                // 「全部展开」连收起的目录一起展开。
                self.git
                    .commit
                    .collapse_all(command == MenuCommand::CommitCollapseAll);
                self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
            }
            _ => self.run_branch_command(command),
        }
        self.request_redraw();
    }

    /// 提交视图里一个文件的右键菜单选了 `command`。
    pub(in crate::app) fn run_commit_file_command(
        &mut self,
        command: crate::app::input::context_menu::MenuCommand,
        path: PathBuf,
    ) {
        use crate::app::input::context_menu::MenuCommand;
        // 选中了多个文件时（菜单只有这几项）作用于选中的各个文件。
        if self.commit_multi_selected() {
            let paths = self.commit_selection();
            match command {
                MenuCommand::GitOpenFile => {
                    for path in paths.iter().filter(|p| winc_backend::fs::is_file(p)) {
                        self.open_path(path);
                    }
                    self.focus = Focus::Editor;
                }
                MenuCommand::GitRollbackFile => self.confirm_rollback(paths),
                MenuCommand::GitStashFiles => self.ask_stash_files(paths),
                MenuCommand::CommitCheck(on) => self.set_commit_checked(&paths, on),
                MenuCommand::CopyPath => self.copy_paths(&paths, false),
                MenuCommand::CopyRelativePath => self.copy_paths(&paths, true),
                _ => {}
            }
            return;
        }
        match command {
            MenuCommand::GitShowDiff => self.select_commit_file(path),
            MenuCommand::GitOpenFile => {
                if winc_backend::fs::is_file(&path) {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                }
            }
            MenuCommand::GitRollbackFile => self.confirm_rollback(vec![path]),
            MenuCommand::GitStashFiles => self.ask_stash_files(vec![path]),
            MenuCommand::CommitCheck(on) => self.set_commit_checked(&[path], on),
            MenuCommand::GitIgnore(kind) => self.git_ignore(&path, kind),
            MenuCommand::CopyPath | MenuCommand::CopyRelativePath => {
                self.copy_path(&path, command == MenuCommand::CopyRelativePath);
            }
            MenuCommand::Reveal => self.reveal_in_file_manager(&path),
            MenuCommand::RevealInTree => {
                self.show_files();
                self.reveal_path(&path);
            }
            _ => {}
        }
        self.request_redraw();
    }
}
