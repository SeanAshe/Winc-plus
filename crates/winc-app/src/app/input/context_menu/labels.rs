//! 菜单项的文字：标题、标题下方的小字和悬停提示。

use winc_i18n::t;

use super::*;

impl App {
    /// 返回命令 `command` 的菜单项文字。
    pub(crate) fn menu_label(&self, command: MenuCommand) -> MenuLabel {
        match command {
            MenuCommand::OpenRecent(i) => self.recent_label(i),
            MenuCommand::GroupFolder(i, j) => self.group_folder_label(i, j),
            MenuCommand::ShowTab(i) => self.tab_list_label(i),
            MenuCommand::LatexProblem(i) => self.latex_problem_label(i),
            MenuCommand::ImportKeymap(i) => {
                let source = self.settings.import_sources.get(i);
                let name = source.map_or("", |s| s.name.as_str());
                MenuLabel {
                    title: t!("settings-keys-import-from", name = name),
                    subtitle: None,
                    tooltip: source.map(|s| winc_editor::display_path(&s.dir, None)),
                }
            }
            MenuCommand::RunSelect(i) => {
                let config = self.run.list.get(i);
                // 名字就是命令时不重复显示。
                let subtitle = config.and_then(|c| {
                    if self.config_running(c) {
                        Some(t!(
                            "terminal-run-menu-running",
                            command = c.command.as_str()
                        ))
                    } else {
                        (c.command != c.name).then(|| c.command.clone())
                    }
                });
                MenuLabel {
                    title: config.map(|c| c.name.clone()).unwrap_or_default(),
                    subtitle,
                    tooltip: None,
                }
            }
            // 暂存区视图中这两项是暂存、取消暂存；勾选式下是勾选、取消勾选。
            MenuCommand::CommitCheck(on) if self.git.commit.staging() => MenuLabel::plain(if on {
                t!("menu-git-stage")
            } else {
                t!("menu-git-unstage")
            }),
            MenuCommand::TerminalDock => {
                let title = if self.terminal_right() {
                    t!("terminal-dock-bottom")
                } else {
                    t!("terminal-dock-right")
                };
                MenuLabel {
                    title: title.to_owned(),
                    subtitle: None,
                    tooltip: None,
                }
            }
            MenuCommand::NewTerminalShell(i) => {
                let shell = self.terminal_shell_list().into_iter().nth(i);
                MenuLabel {
                    title: shell.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
                    subtitle: shell.map(|s| s.command),
                    tooltip: None,
                }
            }
            MenuCommand::Zoom(i) => MenuLabel {
                title: (crate::app::settings::ZOOM_STEPS.get(i))
                    .map(|zoom| format!("{zoom}%"))
                    .unwrap_or_default(),
                subtitle: None,
                tooltip: None,
            },
            MenuCommand::DefaultShell(Some(i)) => {
                let shell = self.terminal.shells.get(i);
                MenuLabel {
                    title: shell.map(|s| s.name.clone()).unwrap_or_default(),
                    subtitle: shell.map(|s| s.command.clone()),
                    tooltip: None,
                }
            }
            MenuCommand::FontDefault
                if matches!(
                    &self.overlays.context_menu,
                    Some(ContextMenu {
                        target: MenuTarget::Font(Setting::TerminalFont),
                        ..
                    })
                ) =>
            {
                MenuLabel {
                    title: t!("settings-terminal-font-default").to_owned(),
                    subtitle: None,
                    tooltip: None,
                }
            }
            MenuCommand::FontDefault
                if matches!(
                    &self.overlays.context_menu,
                    Some(ContextMenu {
                        target: MenuTarget::Font(Setting::MarkdownFont),
                        ..
                    })
                ) =>
            {
                MenuLabel {
                    title: t!("settings-markdown-font-default").to_owned(),
                    subtitle: None,
                    tooltip: None,
                }
            }
            MenuCommand::OpenRecentInNewWindow(i) => {
                let label = self.recent_label(i);
                MenuLabel {
                    title: t!("menu-open-recent-in-new-window", name = &label.title),
                    ..label
                }
            }
            MenuCommand::ToolAuto | MenuCommand::ToolPreset(_) => self.tool_label(command),
            MenuCommand::LogCheckout | MenuCommand::LogCherryPick | MenuCommand::LogNewWorktree => {
                self.log_commit_label(command)
            }
            // 三种重置：名字是标题，各自对改动做什么写在下面一行小字里。
            MenuCommand::LogResetSoft | MenuCommand::LogResetMixed | MenuCommand::LogResetHard => {
                let detail = match command {
                    MenuCommand::LogResetSoft => t!("menu-log-reset-soft-desc"),
                    MenuCommand::LogResetMixed => t!("menu-log-reset-mixed-desc"),
                    _ => t!("menu-log-reset-hard-desc"),
                };
                MenuLabel {
                    subtitle: Some(detail.to_owned()),
                    ..MenuLabel::plain(command.label())
                }
            }
            MenuCommand::StatusPlace(i) => self.location_menu_label(i),
            MenuCommand::CommitMessage(_) | MenuCommand::CommitCoAuthor(_) => self
                .compose_menu_label(command)
                .unwrap_or_else(|| MenuLabel::plain("")),
            MenuCommand::FormBase(0) => match self.git.branches.iter().find(|b| b.current) {
                Some(b) => MenuLabel::plain(&t!("menu-form-base-current-branch", branch = &b.name)),
                None => MenuLabel::plain(t!("menu-form-base-current-commit")),
            },
            MenuCommand::FormBase(i) => {
                MenuLabel::plain(self.git.branches.get(i - 1).map_or("", |b| b.name.as_str()))
            }
            MenuCommand::FormRemote(i) => {
                MenuLabel::plain(self.git.remotes.get(i).map_or("", String::as_str))
            }
            MenuCommand::LogAuthor(1..)
            | MenuCommand::LogSince(1..)
            | MenuCommand::LogPath(1..) => MenuLabel::plain(&self.log_filter_label(command)),
            MenuCommand::GitBranch(..)
            | MenuCommand::GitFetch
            | MenuCommand::GitPush
            | MenuCommand::GitPull
            | MenuCommand::GitPullRebase
            | MenuCommand::GitPullMerge
            | MenuCommand::GitContinue
            | MenuCommand::GitAbort => self.git_label(command),
            // 编辑器字体的「默认」是 Maple Mono NF CN；界面字体仍是系统默认。
            MenuCommand::FontDefault => {
                let title = match self.overlays.context_menu.as_ref().map(|menu| &menu.target) {
                    Some(MenuTarget::Font(Setting::EditorFont)) => winc_render::DEFAULT_EDITOR_FONT,
                    _ => t!("settings-font-default"),
                };
                MenuLabel::plain(title)
            }
            _ => MenuLabel::plain(command.label()),
        }
    }

    /// 日志里提交菜单的文字：有本地分支指着这个提交时「签出「dev」」；拣选写明拣到哪个分支。
    fn log_commit_label(&self, command: MenuCommand) -> MenuLabel {
        let Some(ContextMenu {
            target: MenuTarget::LogCommit(oid),
            ..
        }) = &self.overlays.context_menu
        else {
            return MenuLabel::plain(command.label());
        };
        match command {
            MenuCommand::LogCheckout => match self.log_checkout_branch(oid) {
                Some(branch) => MenuLabel::plain(&t!("menu-log-checkout-branch", branch = branch)),
                None => MenuLabel::plain(command.label()),
            },
            // 有能签出的本地分支指着它时签出那个分支，否则游离地签出提交；建在哪里写在悬停提示里。
            MenuCommand::LogNewWorktree => {
                let branch = self.log_checkout_branch(oid);
                let branch = branch.and_then(|b| self.log_worktree_branch(&b));
                let (title, dir) = match branch {
                    Some(b) => (t!("menu-log-new-worktree-branch", branch = &b.name), b.name),
                    None => (
                        command.label().to_owned(),
                        crate::app::git::log::short(oid).into(),
                    ),
                };
                MenuLabel {
                    title,
                    subtitle: None,
                    tooltip: Some(self.worktree_dir_hint(&dir)),
                }
            }
            _ => {
                let branch = self.current_branch_label();
                MenuLabel::plain(&t!("menu-log-cherry-pick-onto", branch = branch))
            }
        }
    }

    /// 分支操作菜单和「⋯」里的文字：推送、拉取下方写明和哪个上游、差几个提交。
    fn git_label(&self, command: MenuCommand) -> MenuLabel {
        use crate::app::git::branch::BranchOp;
        let current = self.git.branches.iter().find(|b| b.current);
        let here = current.map_or(t!("menu-git-current-branch"), |b| b.name.as_str());
        let upstream = current.and_then(|b| b.upstream.as_deref());
        let (ahead, behind) = self
            .git
            .status
            .as_ref()
            .map_or((0, 0), |s| (s.ahead, s.behind));
        let title = match command {
            // 已经在别的工作树里签出（远程分支看同名的本地分支）：签出就是切过去。
            MenuCommand::GitBranch(BranchOp::Checkout, i) => {
                let holding = self.git.branches.get(i).and_then(|b| {
                    let name = crate::app::git::branch::local_name(b);
                    self.git.worktree_holding(name)
                });
                match holding {
                    Some(w) => {
                        let worktree = &self.git.worktrees[w];
                        let name = crate::app::git::worktree::worktree_name(worktree);
                        t!("menu-git-switch-to-worktree", name = name)
                    }
                    None => command.label().to_owned(),
                }
            }
            MenuCommand::GitContinue | MenuCommand::GitAbort => {
                use winc_editor::git::Operation;
                let operation = self.git.status.as_ref().and_then(|s| s.operation);
                let label = operation.map_or("", Operation::label);
                match (command, operation) {
                    (MenuCommand::GitContinue, Some(Operation::Merge)) => {
                        t!("menu-git-finish-operation", operation = label)
                    }
                    (MenuCommand::GitContinue, _) => {
                        t!("menu-git-continue-operation", operation = label)
                    }
                    _ => t!("menu-git-abort-operation", operation = label),
                }
            }
            _ => command.label().to_owned(),
        };
        let subtitle = match (command, upstream) {
            // 被目录已经不在的工作树占着：git 不让签出，写明要先清理。
            (MenuCommand::GitBranch(BranchOp::Checkout, i), _)
                if self.git.branches.get(i).is_some_and(|b| {
                    let name = crate::app::git::branch::local_name(b);
                    self.git.stale_holding(name).is_some()
                }) =>
            {
                Some(t!("menu-git-checkout-stale-worktree").to_owned())
            }
            (MenuCommand::GitFetch, _) => Some(t!("menu-git-fetch-desc").to_owned()),
            (MenuCommand::GitContinue, _) => Some(t!("menu-git-continue-desc").to_owned()),
            (MenuCommand::GitSkip, _) => Some(t!("menu-git-skip-desc").to_owned()),
            (MenuCommand::GitAbort, _) => Some(t!("menu-git-abort-desc").to_owned()),
            (MenuCommand::GitPush, Some(up)) if ahead > 0 => {
                Some(t!("menu-git-push-ahead", count = ahead, upstream = up))
            }
            (MenuCommand::GitPush, Some(up)) => Some(t!("menu-git-push-up-to-date", upstream = up)),
            (MenuCommand::GitPush, None) => Some(t!("menu-git-push-set-upstream", branch = here)),
            // 平时的拉取写明这次是合并还是变基（设置 `git.pull`）。
            (MenuCommand::GitPull, Some(up)) => {
                let detail = match behind {
                    0 => t!("menu-git-pull-from", upstream = up),
                    _ => t!("menu-git-pull-behind", count = behind, upstream = up),
                };
                let mode = match self.pulls_with_rebase() {
                    true => t!("menu-git-pull-mode-rebase"),
                    false => t!("menu-git-pull-mode-merge"),
                };
                Some(t!("menu-git-pull-mode", detail = detail, mode = mode))
            }
            (
                MenuCommand::GitPull | MenuCommand::GitPullRebase | MenuCommand::GitPullMerge,
                None,
            ) => Some(t!("menu-git-pull-no-upstream").to_owned()),
            _ => None,
        };
        // 新工作树建在哪里写在悬停提示里，与日志菜单相同。
        let tooltip = match command {
            MenuCommand::GitBranch(BranchOp::NewWorktree, i) => (self.git.branches.get(i))
                .map(|b| self.worktree_dir_hint(crate::app::git::branch::local_name(b))),
            _ => None,
        };
        MenuLabel {
            title,
            subtitle,
            tooltip,
        }
    }

    /// 返回格式化（修复）工具菜单中一项的文字：「自动」下方显示当前自动选中的工具；
    /// 预设下方显示该预设的命令，没装的在名字后面注明「（未安装）」。
    fn tool_label(&self, command: MenuCommand) -> MenuLabel {
        let preset = match command {
            MenuCommand::ToolPreset(i) => PRESETS.get(i),
            _ => None,
        };
        let Some(preset) = preset else {
            let (task, auto) = match &self.overlays.context_menu {
                Some(ContextMenu {
                    target: MenuTarget::Tool(task, id),
                    ..
                }) => {
                    let row = self.settings.tool_row(tool_setting(*task, id));
                    (*task, row.and_then(|r| r.auto.clone()))
                }
                _ => (Task::Format, None),
            };
            let subtitle = auto.unwrap_or_else(|| {
                match task {
                    Task::Format => t!("menu-tool-auto-format-desc"),
                    Task::Fix => t!("menu-tool-auto-fix-desc"),
                }
                .to_owned()
            });
            return MenuLabel {
                title: command.label().to_owned(),
                subtitle: Some(subtitle),
                tooltip: None,
            };
        };
        let title = match self.format.formatter.has(preset.program()) {
            Some(false) => t!("menu-tool-not-installed", name = preset.name),
            _ => preset.name.to_owned(),
        };
        MenuLabel {
            title,
            subtitle: Some(preset.command.to_owned()),
            tooltip: None,
        }
    }

    /// 返回标签列表第 `i` 项的菜单文字。
    ///
    /// 标题为标签上的名字；存在同名标签时，标题下方以小字显示标签栏所标的上级目录，
    /// 用于区分同名文件。完整路径显示在悬停提示中（主目录显示为 `~`）。
    fn tab_list_label(&self, i: usize) -> MenuLabel {
        let docs = &self.editor.documents;
        let Some(doc) = docs.get(i) else {
            return MenuLabel::plain("");
        };
        MenuLabel {
            title: doc.name().into_owned(),
            subtitle: crate::app::workspace::tab_names::tab_folder(docs, i),
            tooltip: doc
                .path
                .as_deref()
                .map(|p| winc_editor::display_path(p, None)),
        }
    }
}
