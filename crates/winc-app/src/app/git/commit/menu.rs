//! 提交视图的菜单：「提交」按钮旁的菜单、「⋯」菜单和文件行的菜单，以及各项对应的命令。

use super::*;

impl App {
    /// 提交视图标题行「⋯」的菜单：Git 操作（继续和中止、储藏、远程仓库，见 [`branch_menu`]）；
    /// 以树形显示（排成树时打勾）、全部展开、全部折叠（已经是那样的灰显）；显示 Git 输出。
    ///
    /// 刷新是标题行上的按钮，拉取、推送、获取在分支选择器里，回滚勾选的文件在组标题上，这里不重复。
    pub(in crate::app) fn commit_more_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::MenuEntry;
        // 没有文件的组不显示，不算在内。
        let shown = || {
            let status = self.git.status.as_ref();
            CommitGroup::ALL.into_iter().filter(move |group| {
                status.is_some_and(|s| s.entries.iter().any(|e| group.holds(e)))
            })
        };
        let all_collapsed = shown().all(|group| self.git.commit.group_collapsed(group));
        let all_expanded = shown().all(|group| !self.git.commit.group_collapsed(group))
            && self.git.commit.collapsed_dirs.iter().all(HashSet::is_empty);
        let mut entries = branch_menu(
            &self.git.remote_list,
            self.git.status.as_ref().and_then(|s| s.operation),
            self.git.branch.running.is_some(),
        );
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::CommitTree, true),
            MenuEntry::Item(MenuCommand::CommitExpandAll, !all_expanded),
            MenuEntry::Item(MenuCommand::CommitCollapseAll, !all_collapsed),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::GitShowOutput, true),
        ]);
        entries
    }

    /// 「提交 ⌄」拆分按钮右边箭头的菜单：提交并推送（合并、变基停着时没有推送这回事，没有要提交的
    /// 更改时与主按钮一样不能提交，都灰显）；卡片太窄、底行放不下勾选框时还有修改上一次提交（开着
    /// 时打勾）。
    pub(in crate::app) fn commit_button_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        let status = self.git.status.as_ref();
        let idle = !self.git.commit.busy && status.is_none_or(|s| s.operation.is_none());
        let clean = status.is_some_and(|s| s.entries.is_empty());
        let mut entries = vec![MenuEntry::Item(
            MenuCommand::CommitPush,
            idle && (!clean || self.git.commit.amend),
        )];
        // 正在淡出的「修改上次提交」视为已移入菜单。
        let leaving = matches!(self.git.commit.amend_motion, Some((false, _)));
        if self.commit_amendable() && (self.commit_parts().amend.is_none() || leaving) {
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::Item(MenuCommand::CommitAmend, idle));
        }
        entries
    }

    /// 「提交 ⌄」的菜单选了 `command`。
    pub(in crate::app) fn run_commit_button_command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::CommitPush => self.confirm_commit_and_push(),
            MenuCommand::CommitAmend => self.toggle_amend(),
            _ => {}
        }
        self.request_redraw();
    }

    /// 提交勾上的文件，成功以后继续推送。
    pub(in crate::app) fn git_commit_and_push(&mut self) {
        self.git.commit.push_after = true;
        self.git_commit();
        // 没有真的开始提交（没写信息、没勾文件），就不留着「之后推送」。
        if !self.git.commit.busy {
            self.git.commit.push_after = false;
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
            MenuCommand::CommitTree => self.toggle_commit_layout(),
            MenuCommand::GitShowOutput => self.show_git_output(),
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
                MenuCommand::CommitCheck(on) => {
                    let Some(status) = self.git.status.as_ref() else {
                        return;
                    };
                    let entries: Vec<_> = status
                        .entries
                        .iter()
                        .filter(|e| paths.contains(&e.path))
                        .cloned()
                        .collect();
                    for entry in &entries {
                        self.git.commit.set_checked(entry, on);
                    }
                }
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
