//! 提交视图中的鼠标按下：按命中的目标分发到勾选、展开、按钮和菜单。

use super::*;

impl App {
    /// 点了提交视图里的 `target`。
    pub(in crate::app) fn press_commit(&mut self, target: CommitTarget) {
        // 按下标找文件的目标，状态在绘制以后变过就不处理：下标可能已经指向别的文件。
        let indexed = matches!(
            target,
            CommitTarget::Check(_)
                | CommitTarget::File(_)
                | CommitTarget::OpenFile(_)
                | CommitTarget::RevertFile(_)
                | CommitTarget::Dir(_)
                | CommitTarget::DirCheck(_)
                | CommitTarget::Stash(_)
                | CommitTarget::StashFile(_)
                | CommitTarget::StashApply(_)
                | CommitTarget::StashMenu(_)
        );
        if indexed && self.git.commit.hits_generation != self.git.generation {
            self.request_redraw();
            return;
        }
        // 浏览历史提交的文件时只能展开、收起和查看文件，其余目标（勾选、回滚、储藏、提交等）
        // 都会修改工作区，不处理。
        if self.git.commit.browse {
            match target {
                CommitTarget::File(i) => self.browse_commit_file(i),
                CommitTarget::Group(_) | CommitTarget::Dir(_) | CommitTarget::Sticky(..) => {}
                _ => return,
            }
            if matches!(target, CommitTarget::File(_)) {
                return;
            }
        }
        match target {
            CommitTarget::Stashes
            | CommitTarget::Stash(_)
            | CommitTarget::StashFile(_)
            | CommitTarget::StashApply(_)
            | CommitTarget::StashMenu(_)
            | CommitTarget::StashMore => self.press_commit_stash(target),
            CommitTarget::Group(_)
            | CommitTarget::GroupCheck(_)
            | CommitTarget::GroupRevert(_)
            | CommitTarget::Dir(_)
            | CommitTarget::DirCheck(_)
            | CommitTarget::Check(_)
            | CommitTarget::File(_)
            | CommitTarget::Sticky(..)
            | CommitTarget::OpenFile(_)
            | CommitTarget::RevertFile(_) => self.press_commit_row(target),
            _ => self.press_commit_button(target),
        }
    }

    /// 处理提交视图中按钮类的目标：工具按钮、位置与分支、同步、提交和上一次提交。
    fn press_commit_button(&mut self, target: CommitTarget) {
        match target {
            CommitTarget::Log => self.git_show_log(),
            CommitTarget::Refresh => {
                self.git.commit.refreshed = Some(Instant::now());
                self.git_refresh();
            }
            CommitTarget::More => {
                let rect = self.commit_parts().tools[2].0;
                self.open_button_menu(
                    MenuButton::CommitMore,
                    MenuTarget::CommitMore,
                    menu_below(rect),
                );
            }
            CommitTarget::Location => self.open_location_picker(),
            CommitTarget::Branch => self.open_branch_picker(GitAnchor::Branch),
            CommitTarget::Sync => self.git_sync(),
            // 按住 ⌥ 时按钮写的是「提交并推送」（见 `App::commit_pushes`）。
            CommitTarget::Commit if self.commit_pushes() => self.confirm_commit_and_push(),
            CommitTarget::Commit => self.git_commit(),
            CommitTarget::Amend => self.toggle_amend(),
            CommitTarget::NextStep => self.git_sync(),
            CommitTarget::Abort => self.confirm_abort(),
            CommitTarget::Undo => {
                if let Some(last) = self.commit_undoable().cloned() {
                    self.confirm_undo_commit(&last);
                }
            }
            CommitTarget::LastCommit => {
                let Some((_, _, button)) = self.commit_parts().undo else {
                    return;
                };
                self.open_button_menu(
                    MenuButton::LastCommit,
                    MenuTarget::LastCommit,
                    menu_below(button),
                );
            }
            CommitTarget::CommitMenu => {
                // 菜单挂在「⌄」那一段下方，右端与按钮右端对齐、向左展开，不从整个按钮的左端弹出，
                // 避免菜单看起来与「⌄」无关。
                let rect = self.commit_parts().commit_menu;
                let (_, y) = menu_below(rect);
                self.open_button_menu(
                    MenuButton::CommitButton,
                    MenuTarget::CommitButton,
                    (rect.right(), y),
                );
            }
            // 其余目标由 [`Self::press_commit`] 交给别的函数。
            _ => {}
        }
        self.request_redraw();
    }

    /// 处理文件列表中的目标：分组、目录、文件行及其勾选、打开和回滚。
    fn press_commit_row(&mut self, target: CommitTarget) {
        match target {
            CommitTarget::Group(group) => {
                let rows = self.commit_rows();
                if let Some(row) = rows.iter().position(|r| *r == CommitRow::Group(group)) {
                    self.toggle_commit_row(row);
                }
            }
            CommitTarget::GroupCheck(group) => {
                let Some(status) = self.git.status.as_ref() else {
                    return;
                };
                let on = self.git.commit.group_state(status, group) != Some(true);
                for entry in status.entries.iter().filter(|e| group.holds(e)) {
                    self.git.commit.set_checked(entry, on);
                }
            }
            CommitTarget::GroupRevert(group) => self.ask_rollback(group),
            // ⌘ 单击目录把它下面的文件加入或移出选区，⇧ 单击选到它：只改选区，不收起展开。
            CommitTarget::Dir(row) if self.command_held() => {
                self.focus = Focus::CommitList;
                self.toggle_commit_dir_selected(row);
            }
            CommitTarget::Dir(row) if self.pointer.modifiers.shift_key() => {
                self.focus = Focus::CommitList;
                self.select_commit_rows(row);
            }
            // `⌥` 单击目录：递归收起或展开。
            CommitTarget::Dir(row) if self.pointer.modifiers.alt_key() => {
                self.toggle_commit_dir_recursively(row);
            }
            CommitTarget::Dir(row) => self.toggle_commit_row(row),
            CommitTarget::DirCheck(row) => {
                let rows = self.commit_rows();
                let (Some(CommitRow::Dir { group, dir, .. }), Some(status)) =
                    (rows.get(row), self.git.status.as_ref())
                else {
                    return;
                };
                let members = rows.dir(*group, dir);
                let on = self.git.commit.members_state(status, members) != Some(true);
                for &i in members {
                    self.git.commit.set_checked(&status.entries[i], on);
                }
            }
            CommitTarget::Check(i) => self.toggle_commit_check(i),
            CommitTarget::File(i) => {
                let Some(path) = self.commit_entry(i).map(|e| e.path.clone()) else {
                    return;
                };
                self.focus = Focus::CommitList;
                // ⌘ 单击加选或取消，⇧ 单击选到这一行：只改选区，不换预览。
                if self.command_held() {
                    self.toggle_commit_selected(path);
                    self.request_redraw();
                    return;
                }
                if self.pointer.modifiers.shift_key() {
                    self.select_commit_range(path);
                    self.request_redraw();
                    return;
                }
                // 双击打开文件本身，单击选中并预览差异。
                let (x, y) = self.pointer.mouse;
                if self.click_count(x, y) >= 2 && winc_backend::fs::is_file(&path) {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                } else {
                    self.select_commit_file(path);
                }
            }
            CommitTarget::Sticky(row, level) => {
                // 先滚到那一行正好停在固定的位置，画面不动；再当作点了那一行。
                self.git.commit.scroll =
                    (row as f32 * COMMIT_ROW - level as f32 * COMMIT_ROW).max(0.0);
                let target = match self.commit_rows().get(row) {
                    Some(CommitRow::Group(group)) => CommitTarget::Group(*group),
                    Some(CommitRow::Dir { .. }) => CommitTarget::Dir(row),
                    _ => return,
                };
                return self.press_commit(target);
            }
            CommitTarget::OpenFile(i) => {
                let Some(path) = self.commit_entry(i).map(|e| e.path.clone()) else {
                    return;
                };
                if winc_backend::fs::is_file(&path) {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                } else {
                    self.flash(t!("git-commit-open-deleted"));
                }
            }
            CommitTarget::RevertFile(i) => {
                if let Some(path) = self.commit_entry(i).map(|e| e.path.clone()) {
                    self.confirm_rollback(vec![path]);
                }
            }
            // 其余目标由 [`Self::press_commit`] 交给别的函数。
            _ => {}
        }
        self.request_redraw();
    }

    /// 处理贮藏列表中的目标：展开、查看、应用和菜单。
    fn press_commit_stash(&mut self, target: CommitTarget) {
        match target {
            CommitTarget::Stashes => self.toggle_stashes(),
            CommitTarget::Stash(i) => self.show_stash(i),
            CommitTarget::StashFile(at) => self.open_stash_file(at),
            CommitTarget::StashApply(i) => {
                if let Some(stash) = self.git.stashes.get(i).cloned() {
                    self.run_stash_op(super::super::branch::StashOp::Apply, stash);
                }
            }
            CommitTarget::StashMenu(i) => {
                let Some(&(rect, _)) =
                    (self.hits.commit.iter()).find(|(_, t)| *t == CommitTarget::StashMenu(i))
                else {
                    return;
                };
                self.open_context_menu(MenuTarget::StashRow(i), menu_below(rect));
            }
            CommitTarget::StashMore => {
                let Some(&(rect, _)) =
                    (self.hits.commit.iter()).find(|(_, t)| *t == CommitTarget::StashMore)
                else {
                    return;
                };
                self.open_context_menu(MenuTarget::StashList, (rect.x + 8.0, rect.bottom()));
            }
            // 其余目标由 [`Self::press_commit`] 交给别的函数。
            _ => {}
        }
        self.request_redraw();
    }
}
