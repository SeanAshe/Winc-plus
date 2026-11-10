//! Git 的分支操作：点提交视图的分支行、状态栏的分支名打开分支选择器（见 `picker`）。
//!
//! 提交视图标题行的「⋯」里有获取、拉取、推送、新建分支、新建工作树和储藏；每个分支另有操作菜单：
//! 签出、在新工作树中签出、从这里新建分支、合并到当前分支、变基、删除。操作都在后台运行，同一时刻
//! 只做一件，运行期间状态栏的分支名后面写着在做什么；做完在状态栏说结论（失败时右上角说原因）、
//! 刷新仓库状态，并重新读被改动过的文件。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use winc_editor::git::{Branch, BranchDeleted, Carried, Git, Operation, Remote, Stash};
use winc_i18n::t;

use super::commit::{COMMITTED_SHOWN, PushIntent};
use super::question;
use super::unusable;
use super::worktree::RemoveCheck;
use crate::app::animation::{Clock, TREE_ANIMATION};
use crate::app::input::context_menu::{MenuCommand, MenuEntry};
use crate::app::{App, UserEvent};
use crate::askpass::{Listener, Question};

/// 对一个分支做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchOp {
    /// 签出该分支；远程分支签出成同名的本地分支并跟踪它。
    Checkout,
    /// 在新工作树中签出（`App::new_worktree_for`），建好在新窗口打开。
    NewWorktree,
    /// 从该分支新建分支（输入名字）。
    NewFrom,
    /// 合并到当前分支。
    Merge,
    /// 把当前分支变基到该分支上。
    Rebase,
    /// 重命名（输入新名字），只用于本地分支。
    Rename,
    /// 删除（先确认；未合并的再确认一次才强制删除）。
    Delete,
    /// 和当前分支比较：打开日志，列出从当前分支到它改了的文件。
    Compare,
}

impl BranchOp {
    /// 操作菜单的各组，顺序就是菜单里的顺序：切换和新建、比较、合到当前分支、删除。
    const GROUPS: [&[BranchOp]; 4] = [
        &[BranchOp::Checkout, BranchOp::NewWorktree, BranchOp::NewFrom],
        &[BranchOp::Compare],
        &[BranchOp::Merge, BranchOp::Rebase],
        &[BranchOp::Rename, BranchOp::Delete],
    ];

    /// 菜单里的文字：不写分支名。菜单是从这个分支的那一行弹出来的，另一边总是当前分支。
    pub fn label(self) -> &'static str {
        match self {
            BranchOp::Checkout => t!("git-branch-checkout"),
            BranchOp::NewWorktree => t!("git-branch-checkout-new-worktree"),
            BranchOp::NewFrom => t!("git-branch-new-from"),
            BranchOp::Merge => t!("git-branch-merge-into-current"),
            BranchOp::Rebase => t!("git-branch-rebase-current-onto"),
            BranchOp::Rename => t!("git-branch-rename"),
            BranchOp::Delete => t!("git-branch-delete"),
            BranchOp::Compare => t!("git-branch-compare-with-current"),
        }
    }
}

/// 对日志里一个标签做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagOp {
    /// 推送到远程仓库。
    Push,
    /// 删除本地标签（先确认）。
    Delete,
    /// 从远程仓库删除（先确认）。
    DeleteRemote,
}

impl TagOp {
    /// 全部操作，顺序就是子菜单里的顺序。
    pub const ALL: [TagOp; 3] = [TagOp::Push, TagOp::Delete, TagOp::DeleteRemote];

    /// 子菜单里的文字。
    pub fn label(self) -> &'static str {
        match self {
            TagOp::Push => t!("git-tag-push"),
            TagOp::Delete => t!("git-tag-delete"),
            TagOp::DeleteRemote => t!("git-tag-delete-remote"),
        }
    }
}

/// 对一个远程仓库做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteOp {
    /// 修改地址。
    Edit,
    /// 移除（先确认）。
    Remove,
}

impl RemoteOp {
    /// 子菜单里的文字。
    pub fn label(self) -> &'static str {
        match self {
            RemoteOp::Edit => t!("git-remote-edit"),
            RemoteOp::Remove => t!("git-remote-remove"),
        }
    }
}

/// 对一条储藏做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StashOp {
    /// 应用到工作区，储藏留着（`stash apply`）。
    Apply,
    /// 应用到工作区并删掉这条储藏（`stash pop`）。
    Pop,
    /// 删掉这条储藏（`stash drop`，先确认）。
    Drop,
}

impl StashOp {
    /// 全部操作，顺序就是子菜单里的顺序。
    pub const ALL: [StashOp; 3] = [StashOp::Apply, StashOp::Pop, StashOp::Drop];

    /// 子菜单里的文字。
    pub fn label(self) -> &'static str {
        match self {
            StashOp::Apply => t!("git-stash-apply"),
            StashOp::Pop => t!("git-stash-pop"),
            StashOp::Drop => t!("git-stash-drop"),
        }
    }
}

/// 一次操作成功之后要做的事。
pub(super) enum Then {
    /// 没有了：说一声做完了。
    Nothing,
    /// 工作树变了：重新取工作树列表。
    Refresh,
    /// 新建了工作树或克隆了仓库：重新取列表，打开它；`true` 是在新窗口打开。
    Open(PathBuf, bool),
    /// 当前目录成为了仓库：重新查找仓库。
    Rescan,
    /// 读取好了交互式变基的范围：打开对话框，不说做完了。
    RebasePlan(Box<winc_editor::git::RebasePlan>),
    /// 检查完要移除的工作树：确认以后移除（[`App::confirm_remove_worktree`]），不说做完了。
    ConfirmRemove(RemoveCheck),
    /// 同步需要强制推送：确认以后推送（[`App::confirm_force_push`]），不说做完了；内容是上游将被
    /// 替换的提交数。
    ConfirmForcePush(usize),
    /// 操作已完成，但有需要用户处理的情况：显示这条提示，不说做完了。
    Warn(String),
    /// 要删除的分支未完全合并：确认以后强制删除（[`App::confirm_force_delete_branch`]），不说
    /// 做完了；内容是分支名。
    ConfirmForceDelete(String),
}

/// 一次操作的结果。
struct Done {
    /// 第几个操作（[`BranchState::started`]）：换目录以后又起了新操作时，认得出这是之前的。
    id: u64,
    /// 在哪个仓库里做的；操作开始时当前目录不是仓库时为 `None`。
    root: Option<PathBuf>,
    /// 做的是什么（「推送」「签出「dev」」），用在提示里。
    what: String,
    result: Result<(String, Then), String>,
}

/// 输入请求从后台线程到界面线程的通道，每项是所属操作的编号和请求。
type QuestionChannel = (Sender<(u64, Question)>, Receiver<(u64, Question)>);

/// 分支操作的状态。
pub(crate) struct BranchState {
    /// 正在做的事；状态栏的分支名后面写着它。
    pub running: Option<String>,
    /// 上一个操作成功后，同步控件显示对勾的时钟。
    pub finished: Option<Clock>,
    /// 起过几个操作；正在做的是第几个。
    pub(in crate::app) started: u64,
    results: (Sender<Done>, Receiver<Done>),
    /// 后台送达的输入请求及其所属操作的编号。
    pub(super) questions: QuestionChannel,
    /// 对话框正在处理的请求：所属操作的编号和答复通道；丢弃通道表示取消。
    pub(in crate::app) asking: Option<(u64, Sender<String>)>,
    /// 用户是否取消了当前操作的输入请求；为真时该操作的失败不按错误提示。
    pub(in crate::app) declined: bool,
    /// 登录对话框中填写的密码及其远程地址，用于答复随后的密码请求，操作结束时清除。
    pub(in crate::app) password: Option<(String, String)>,
}

impl Default for BranchState {
    fn default() -> Self {
        BranchState {
            running: None,
            finished: None,
            started: 0,
            results: channel(),
            questions: channel(),
            asking: None,
            declined: false,
            password: None,
        }
    }
}

/// 提交视图「⋯」里的 Git 操作：进行中的合并、变基的继续和中止，储藏更改，远程仓库。
///
/// 拉取、推送、获取和新建分支、工作树都在分支选择器里，各条储藏在改动列表下面的储藏一组里，回滚
/// 勾选的文件在组标题上，这里不再列出。
pub(crate) fn branch_menu(
    remotes: &[Remote],
    operation: Option<Operation>,
    busy: bool,
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    let mut entries = Vec::new();
    // 合并、变基停下来等着处理冲突：最上面是继续和中止。
    if operation.is_some() {
        entries.push(MenuEntry::Item(GitContinue, !busy));
        entries.push(MenuEntry::Item(GitAbort, !busy));
        entries.push(MenuEntry::Separator);
    }
    entries.push(MenuEntry::Item(GitStash, !busy));
    entries.push(MenuEntry::Separator);
    // 远程仓库：每个一个子菜单（修改地址、移除），最后是添加。
    let mut items: Vec<MenuEntry> = (remotes.iter().enumerate())
        .map(|(i, remote)| {
            let ops = [RemoteOp::Edit, RemoteOp::Remove]
                .into_iter()
                .map(|op| MenuEntry::Item(GitRemote(op, i), !busy))
                .collect();
            MenuEntry::Submenu(remote.name.clone(), ops)
        })
        .collect();
    if !items.is_empty() {
        items.push(MenuEntry::Separator);
    }
    items.push(MenuEntry::Item(GitAddRemote, !busy));
    entries.push(MenuEntry::Submenu(t!("git-remotes-menu").to_owned(), items));
    entries
}

/// 第 `index` 条储藏的菜单：应用、应用并删除，分隔线，删除（删除先确认）。
pub(crate) fn stash_ops(index: usize, busy: bool) -> Vec<MenuEntry> {
    let mut ops: Vec<MenuEntry> = (StashOp::ALL.into_iter())
        .map(|op| MenuEntry::Item(MenuCommand::GitStashOp(op, index), !busy))
        .collect();
    ops.insert(2, MenuEntry::Separator);
    ops
}

/// 全部储藏的菜单，每条一个子菜单（见 [`stash_ops`]）：提交视图里储藏多到列不下时弹出。
pub(crate) fn stash_menu(stashes: &[Stash], busy: bool) -> Vec<MenuEntry> {
    (stashes.iter().enumerate())
        .map(|(i, stash)| MenuEntry::Submenu(stash_title(stash), stash_ops(i, busy)))
        .collect()
}

/// 分支选择器里第 `index` 个分支的操作菜单：按 `BranchOp::GROUPS` 分组，组之间有分隔线。
///
/// `branches` 是本地分支在前、远程分支在后的列表，当前分支只能重命名：从当前分支新建分支、工作树
/// 在分支选择器的底部；`held` 是被别的工作树
/// 签出的本地分支，它们的「签出」是切到那个工作树，不能删除；`stale` 是被目录已经不在的工作树
/// 占着的分支，git 仍然不允许签出、删除，要先清理。
pub(crate) fn branch_ops(
    index: usize,
    branches: &[Branch],
    held: &[&str],
    stale: &[&str],
    busy: bool,
) -> Vec<MenuEntry> {
    use MenuCommand::GitBranch;
    let Some(branch) = branches.get(index) else {
        return Vec::new();
    };
    let current = branches.iter().find(|b| b.current).map(|b| b.name.as_str());
    let items = |group: &[BranchOp]| -> Vec<MenuEntry> {
        group
            .iter()
            .copied()
            .filter(|op| match op {
                // 当前分支不能签出、合并、变基、删除自己。
                _ if branch.current => matches!(op, BranchOp::Rename),
                // 已经在其他工作树里签出着、又是当前分支（`origin/main` 在 `main` 上）：
                // 不能再签出该分支。
                BranchOp::NewWorktree => {
                    let local = local_name(branch);
                    !held.contains(&local) && !stale.contains(&local) && Some(local) != current
                }
                // 远程分支不在这里删除、重命名。
                BranchOp::Delete | BranchOp::Rename => !branch.remote,
                _ => true,
            })
            .map(|op| {
                // 被别的工作树签出的分支 git 不允许删除；签出就是切到那个工作树，不改仓库，忙的时候
                // 也能点。远程分支看签出后的本地分支。
                let elsewhere = held.contains(&local_name(branch));
                let gone = stale.contains(&local_name(branch));
                let enabled = match op {
                    BranchOp::Delete => !busy && !elsewhere && !gone,
                    BranchOp::Checkout if elsewhere => true,
                    BranchOp::Checkout if gone => false,
                    _ => !busy,
                };
                MenuEntry::Item(GitBranch(op, index), enabled)
            })
            .collect()
    };
    let mut entries = Vec::new();
    for group in BranchOp::GROUPS {
        let group = items(group);
        if !group.is_empty() && !entries.is_empty() {
            entries.push(MenuEntry::Separator);
        }
        entries.extend(group);
    }
    entries
}

/// 签出分支 `branch` 得到的本地分支名：远程分支去掉远程名（`origin/dev` → `dev`）。
pub(crate) fn local_name(branch: &Branch) -> &str {
    if branch.remote {
        branch
            .name
            .split_once('/')
            .map_or(branch.name.as_str(), |(_, l)| l)
    } else {
        &branch.name
    }
}

/// 储藏在菜单里的名字：git 记下的说明去掉「On main: 」这样的开头。
///
/// 超过 40 个字符的截短并加省略号（经验值：菜单一行放得下的长度）。
pub(crate) fn stash_title(stash: &Stash) -> String {
    let message = stash
        .message
        .split_once(": ")
        .map_or(stash.message.as_str(), |(_, rest)| rest);
    let mut title: String = message.chars().take(40).collect();
    if message.chars().count() > 40 {
        title.push('…');
    }
    title
}

impl App {
    /// 推送当前分支要做的事：有上游时推送，没有时发布到 `publish_remote`（状态还没读到时按
    /// `origin`、第一个远程的顺序选择）；没有远程时提示原因并返回 `None`。
    pub(in crate::app) fn push_intent(&mut self) -> Option<PushIntent> {
        if self.has_upstream() {
            return Some(PushIntent::Push);
        }
        let fallback = (self.git.remotes.iter())
            .find(|r| *r == "origin")
            .or(self.git.remotes.first());
        let remote = self.git.publish_remote.clone().or(fallback.cloned());
        if remote.is_none() {
            self.notify(t!("git-push-no-remote"));
        }
        remote.map(PushIntent::Publish)
    }

    pub(in crate::app) fn has_upstream(&self) -> bool {
        self.git
            .branches
            .iter()
            .any(|b| b.current && b.upstream.is_some())
    }

    /// 执行提交视图「⋯」里的 Git 操作 `command`。
    pub(in crate::app) fn run_branch_command(&mut self, command: MenuCommand) {
        use MenuCommand::*;
        match command {
            GitFetch => self.git_operate(t!("git-op-fetch").into(), |git, root| {
                git.operate(root, &["fetch", "--all", "--prune"])
            }),
            GitPull => self.git_operate_then(t!("git-op-pull").into(), |git, root| {
                keep_changes(git, root, t!("git-op-pull"), |git| git.pull(root))
            }),
            GitPullRebase => self.git_operate_then(t!("git-op-pull-rebase").into(), |git, root| {
                keep_changes(git, root, t!("git-op-pull-rebase"), |git| {
                    git.pull_rebase(root)
                })
            }),
            GitAddRemote => self.open_remote_dialog(None),
            GitRemote(op, i) => {
                let Some(remote) = self.git.remote_list.get(i).cloned() else {
                    return;
                };
                match op {
                    RemoteOp::Edit => self.open_remote_dialog(Some(remote)),
                    RemoteOp::Remove => self.confirm_remove_remote(remote.name),
                }
            }
            // 推送前确认目标远程（[`App::ask_push`]）。
            GitPush => {
                if let Some(intent) = self.push_intent() {
                    self.ask_push(intent);
                }
            }
            GitStash => self.open_stash_prompt(),
            GitContinue | GitAbort => {
                let Some(operation) = self.git.status.as_ref().and_then(|s| s.operation) else {
                    return;
                };
                match command {
                    // 合并在提交视图里完成：写好（已经填上的）提交信息再提交。
                    GitContinue if operation == Operation::Merge => self.show_commit(),
                    GitContinue => self
                        .git_operate(continue_what(operation).into(), move |git, root| {
                            git.continue_operation(root, operation)
                        }),
                    _ => self.git_operate(abort_what(operation).into(), move |git, root| {
                        git.abort_operation(root, operation)
                    }),
                }
            }
            _ => {}
        }
    }

    /// 对储藏 `stash` 执行 `op`。
    ///
    /// 储藏按提交号指定（[`Git::stash`]），确认和执行期间其他储藏的增删不影响目标。
    pub(in crate::app) fn run_stash_op(&mut self, op: StashOp, stash: Stash) {
        let (name, oid) = (stash.name.clone(), stash.oid.clone());
        match op {
            StashOp::Apply => self.git_operate(
                t!("git-op-stash-apply", name = name.as_str()),
                move |git, root| git.stash(root, "apply", &oid),
            ),
            StashOp::Pop => self.git_operate(
                t!("git-op-stash-pop", name = name.as_str()),
                move |git, root| git.stash(root, "pop", &oid),
            ),
            StashOp::Drop => self.confirm_drop_stash(stash),
        }
    }

    /// 对分支 `branch` 执行操作菜单中的 `op`。
    pub(in crate::app) fn run_branch_op(&mut self, op: BranchOp, branch: Branch) {
        let name = branch.name.clone();
        match op {
            // 分支已经在其他工作树里签出：git 不允许重复签出，改为切到该工作树。
            BranchOp::Checkout if let Some(i) = self.git.worktree_holding(local_name(&branch)) => {
                self.open_worktree(i, false);
            }
            BranchOp::NewWorktree => self.new_worktree_for(&branch),
            BranchOp::Checkout if branch.remote => {
                // 已经有同名的本地分支就切过去，没有就建一个跟踪它的。
                let local = local_name(&branch);
                let exists = self
                    .git
                    .branches
                    .iter()
                    .any(|b| !b.remote && b.name == local);
                let local = local.to_owned();
                self.git_operate(
                    t!("git-op-checkout", name = local.as_str()),
                    move |git, root| {
                        if exists {
                            git.operate(root, &["switch", "--end-of-options", &local])
                        } else {
                            git.operate(root, &["switch", "--track", "--end-of-options", &name])
                        }
                    },
                );
            }
            BranchOp::Checkout => self.git_operate(
                t!("git-op-checkout", name = name.as_str()),
                move |git, root| git.operate(root, &["switch", "--end-of-options", &name]),
            ),
            BranchOp::NewFrom => {
                self.open_branch_dialog(Some(name), None);
            }
            BranchOp::Merge => self.git_operate(
                t!("git-op-merge", name = name.as_str()),
                move |git, root| {
                    git.operate(root, &["merge", "--no-edit", "--end-of-options", &name])
                },
            ),
            BranchOp::Rebase => self.git_operate(
                t!("git-op-rebase-onto", name = name.as_str()),
                move |git, root| git.operate(root, &["rebase", "--end-of-options", &name]),
            ),
            BranchOp::Rename => self.open_rename_prompt(name),
            BranchOp::Delete => self.confirm_delete_branch(name),
            BranchOp::Compare => self.git_compare_branch(&name),
        }
    }

    /// 储藏工作区和暂存区里的改动（含未跟踪的文件），再切换到 `branch`。
    ///
    /// 切换失败时把储藏恢复回来，失败的原因里写明更改的位置。
    pub(in crate::app) fn stash_then_checkout(&mut self, branch: Branch) {
        let (local, track, message) = self.switch_plan(&branch);
        self.git_operate(
            t!("git-op-checkout", name = local.as_str()),
            move |git, root| {
                git.operate(root, &stash_args(&message))?;
                run_switch(git, root, &branch.name, &local, track)
                    .map_err(|error| restored_error(git, root, &error))
            },
        );
    }

    /// 切换到 `branch`，未提交的更改留在工作区。
    ///
    /// 改动的文件在两个分支上内容不同时 git 拒绝切换：这时储藏更改、切换，再把更改应用到新分支。
    /// 不依赖错误文字判断原因：错误文字随系统语言变化。更改无法完整应用时储藏保留，并提示用户。
    pub(in crate::app) fn carry_then_checkout(&mut self, branch: Branch) {
        let (local, track, message) = self.switch_plan(&branch);
        let what = t!("git-op-checkout", name = local.as_str());
        self.git_operate_then(what, move |git, root| {
            let Err(refused) = run_switch(git, root, &branch.name, &local, track) else {
                return Ok((String::new(), Then::Nothing));
            };
            // 没有可储藏的内容时 `stash push` 同样成功：比较储藏的顶端，避免恢复无关的旧储藏。
            let tip = |git: &Git| git.operate(root, &["rev-parse", "-q", "--verify", "refs/stash"]);
            let before = tip(git).ok();
            // 储藏失败时（例如合并未完成）报告的仍是切换失败的原因。
            if git.operate(root, &stash_args(&message)).is_err() || tip(git).ok() == before {
                return Err(refused);
            }
            run_switch(git, root, &branch.name, &local, track)
                .map_err(|error| restored_error(git, root, &error))?;
            let Err(error) = git.operate(root, &["stash", "pop"]) else {
                return Ok((String::new(), Then::Nothing));
            };
            // 恢复失败不一定有冲突：未跟踪的文件在新分支上已存在时，git 不写冲突标记。
            let unmerged = ["diff", "--name-only", "--diff-filter=U"];
            let conflicted = git.operate(root, &unmerged).is_ok_and(|f| !f.is_empty());
            let branch = local.as_str();
            let warning = if conflicted {
                t!("git-switch-carry-conflict", branch = branch)
            } else {
                t!("git-switch-carry-unapplied", branch = branch, error = error)
            };
            Ok((String::new(), Then::Warn(warning)))
        });
    }

    /// 返回切换到 `branch` 需要的（本地分支名，是否新建跟踪分支，储藏说明）。
    fn switch_plan(&self, branch: &Branch) -> (String, bool, String) {
        let local = local_name(branch).to_owned();
        let track =
            branch.remote && !(self.git.branches.iter()).any(|b| !b.remote && b.name == local);
        let current = (self.git.branches.iter())
            .find(|b| b.current)
            .map_or_else(String::new, |b| b.name.clone());
        let message = t!("git-switch-stash-message", branch = current.as_str());
        (local, track, message)
    }

    /// 储藏工作区和暂存区里的改动（含未跟踪的文件），说明是 `message`（空着时 git 自己写）。
    pub(in crate::app) fn git_stash(&mut self, message: &str) {
        let message = message.trim().to_owned();
        self.git_operate(t!("git-op-stash").into(), move |git, root| {
            let mut args = vec!["stash", "push", "--include-untracked"];
            if !message.is_empty() {
                args.extend(["--message", message.as_str()]);
            }
            git.operate(root, &args)
        });
    }

    /// 删除储藏前先确认（删掉就找不回来了）；没有窗口时（测试中）直接删。
    fn confirm_drop_stash(&mut self, stash: Stash) {
        let message = t!("git-stash-drop-confirm", name = stash_title(&stash));
        if self.window.handle.is_some() {
            let detail = t!("git-stash-drop-confirm-detail");
            let button = t!("git-stash-drop-confirm-button");
            self.show_confirm(&message, detail, button, move |ok| {
                let op = super::log::LogConfirmed::DropStash(Box::new(stash.clone()));
                ok.then_some(UserEvent::GitLogConfirmed(op))
            });
            return;
        }
        // 没有窗口（测试中）时直接删除。
        self.run_log_confirmed(super::log::LogConfirmed::DropStash(Box::new(stash)));
    }

    /// 删除分支前先确认。
    fn confirm_delete_branch(&mut self, name: String) {
        let message = t!("git-branch-delete-confirm", name = name.as_str());
        let detail = t!("git-branch-delete-confirm-detail");
        let button = t!("git-branch-delete-confirm-button");
        let op = super::log::LogConfirmed::DeleteBranch(name, false);
        self.show_confirm(&message, detail, button, move |ok| {
            ok.then(|| UserEvent::GitLogConfirmed(op.clone()))
        });
    }

    /// 要删除的分支未完全合并：用提示框询问，确认后才强制删除。
    fn confirm_force_delete_branch(&mut self, name: String) {
        let message = t!("git-branch-force-delete-confirm", name = name.as_str());
        let detail = t!("git-branch-force-delete-confirm-detail");
        let button = t!("git-branch-force-delete-confirm-button");
        let op = super::log::LogConfirmed::DeleteBranch(name, true);
        self.show_confirm(&message, detail, button, move |ok| {
            ok.then(|| UserEvent::GitLogConfirmed(op.clone()))
        });
    }

    /// 在后台删除分支 `name`（[`Git::delete_branch`]）；未完全合并而 `force` 为假时转为询问。
    pub(in crate::app) fn run_delete_branch(&mut self, name: String, force: bool) {
        self.git_operate_then(
            t!("git-op-delete-branch", name = name.as_str()),
            move |git, root| {
                let deleted = git.delete_branch(root, &name, force)?;
                Ok(match deleted {
                    BranchDeleted::Done(summary) => (summary, Then::Nothing),
                    BranchDeleted::NeedsForce => (String::new(), Then::ConfirmForceDelete(name)),
                })
            },
        );
    }

    /// 新建分支 `name`（从 `from` 开始，可以是分支或提交，`None` 是当前分支），`checkout` 为真时
    /// 切过去。
    pub(in crate::app) fn git_new_branch(
        &mut self,
        name: String,
        from: Option<String>,
        checkout: bool,
    ) {
        if name.is_empty() {
            return;
        }
        self.git_operate(
            t!("git-op-new-branch", name = name.as_str()),
            move |git, root| {
                // `--create` 把下一个参数当作它的值，`--end-of-options` 要放在名字之后。
                let mut args = if checkout {
                    vec!["switch", "--create", name.as_str(), "--end-of-options"]
                } else {
                    vec!["branch", "--end-of-options", name.as_str()]
                };
                if let Some(from) = from.as_deref() {
                    args.push(from);
                }
                git.operate(root, &args)
            },
        );
    }

    /// 在提交 `at` 上打一个轻量标签 `name`（`git tag name at`）；名字已经有了时 git 说原因。
    pub(in crate::app) fn git_new_tag(&mut self, name: &str, at: String) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.git_operate(
            t!("git-op-new-tag", name = name.as_str()),
            move |git, root| {
                git.operate(
                    root,
                    &["tag", "--end-of-options", name.as_str(), at.as_str()],
                )
            },
        );
    }

    /// 把本地分支 `old` 重命名为 `name`；名字不合法或已被占用时由 `git` 说明原因。
    pub(in crate::app) fn git_rename_branch(&mut self, old: String, name: &str) {
        let name = name.trim().to_owned();
        if name.is_empty() || name == old {
            return;
        }
        self.git_operate(
            t!("git-op-rename-branch", name = old.as_str()),
            move |git, root| {
                git.operate(root, &["branch", "--move", "--end-of-options", &old, &name])
            },
        );
    }

    /// 在后台做一件会改仓库的事 `what`，同一时刻只做一件。
    pub(super) fn git_operate(
        &mut self,
        what: String,
        work: impl FnOnce(&Git, &Path) -> Result<String, String> + Send + 'static,
    ) {
        self.git_operate_then(what, move |git, root| {
            work(git, root).map(|summary| (summary, Then::Nothing))
        });
    }

    /// 和 [`App::git_operate`] 一样，成功之后还要做 `work` 交回来的 [`Then`]。
    pub(super) fn git_operate_then(
        &mut self,
        what: String,
        work: impl FnOnce(&Git, &Path) -> Result<(String, Then), String> + Send + 'static,
    ) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        self.git_operate_in(root, what, work);
    }

    /// 和 [`App::git_operate_then`] 一样，但在目录 `dir` 中进行，`dir` 可以不是当前仓库。
    pub(super) fn git_operate_in(
        &mut self,
        dir: PathBuf,
        what: String,
        work: impl FnOnce(&Git, &Path) -> Result<(String, Then), String> + Send + 'static,
    ) {
        if self.branch.running.is_some() {
            self.flash(t!("git-op-busy"));
            return;
        }
        let root = self.git.root.clone();
        let sender = self.branch.results.0.clone();
        self.branch.running = Some(what.clone());
        self.branch.finished = None;
        self.branch.declined = false;
        self.branch.started += 1;
        let id = self.branch.started;
        let wake = self.proxy.clone();
        let questions = self.branch.questions.0.clone();
        self.spawn_git(UserEvent::GitOperated, move |tool| {
            let result = match tool {
                Ok(git) => {
                    // 端点只在操作期间存在，`listener` 在本分支结束时被丢弃。
                    let listener = wake.and_then(|proxy| question::listen(proxy, questions, id));
                    let askpass = listener.as_ref().and_then(Listener::askpass);
                    work(&git.with_askpass(askpass), &dir)
                }
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Done {
                id,
                root,
                what,
                result,
            });
        });
        self.request_redraw();
    }

    /// 同步控件上对勾的不透明度：过渡和侧栏里展开、收起的时长相同，没有对勾时为 0。
    pub(crate) fn sync_check(&self) -> f32 {
        self.branch
            .finished
            .map_or(0.0, |clock| clock.pulse(TREE_ANIMATION))
    }

    /// 收下后台做完的分支操作。
    pub(in crate::app) fn poll_branch_ops(&mut self) {
        while let Ok(done) = self.branch.results.1.try_recv() {
            self.end_git_questions(done.id);
            // 做的时候换了目录（换的时候 `running` 已经清掉，见 `git_folder_changed`）：只说一声在
            // 哪个仓库里做完了；新建的工作树仍然打开，检查完的不再移除。换回原来的目录、又起了新
            // 操作时根目录对得上但不是正在做的这个，同样按旧结果处理，不清掉新操作的 `running`。
            let stale = self.branch.running.is_none() || self.branch.started != done.id;
            if stale || self.git.root != done.root {
                let root = done.root.unwrap_or_default();
                let name = winc_editor::display_name(&root);
                match done.result {
                    Ok((
                        _,
                        Then::ConfirmRemove(_)
                        | Then::ConfirmForcePush(_)
                        | Then::ConfirmForceDelete(_)
                        | Then::RebasePlan(_),
                    )) => {}
                    Ok((_, Then::Warn(warning))) => self.notify(warning),
                    Ok((_, then)) => {
                        self.flash(t!("git-op-done-in", name = name, what = done.what));
                        // 已经换了目录：当前窗口不再换过去，在新窗口打开。
                        if let Then::Open(path, _) = then {
                            self.open_in_new_window(&path);
                        }
                    }
                    Err(e) => {
                        let what = done.what;
                        let message = t!("git-op-failed-in", name = name, what = &what, error = e);
                        self.report_git_failure(&what, message);
                    }
                }
                continue;
            }
            self.branch.running = None;
            // 操作由表单对话框发起时：成功则关闭对话框，失败则在对话框中显示原因。
            let error = done.result.as_ref().err().map(String::as_str);
            if self.form_operation_done(done.id, error) && error.is_some() {
                continue;
            }
            match done.result {
                Ok((_, Then::ConfirmRemove(check))) => self.confirm_remove_worktree(check),
                Ok((_, Then::ConfirmForcePush(count))) => self.confirm_force_push(count),
                Ok((_, Then::Warn(warning))) => self.notify(warning),
                Ok((_, Then::RebasePlan(plan))) => self.open_rebase_dialog(*plan),
                Ok((_, Then::ConfirmForceDelete(name))) => self.confirm_force_delete_branch(name),
                Ok((summary, then)) => {
                    if summary.is_empty() {
                        self.flash(t!("git-op-done", what = done.what));
                    } else {
                        let what = done.what;
                        self.flash(t!("git-op-done-summary", what = what, summary = summary));
                    }
                    self.worktree_done(then);
                    // 只更新帧间隔：对勾是操作结果，动态效果关闭时也要显示。
                    let _ = self.clock(COMMITTED_SHOWN);
                    self.branch.finished = Some(Clock::new(COMMITTED_SHOWN));
                }
                Err(e) => {
                    let what = done.what;
                    let message = t!("git-op-failed", what = &what, error = e);
                    self.report_git_failure(&what, message);
                }
            }
            // 切换分支、拉取、合并都会改工作区里的文件。
            self.check_disk();
        }
        self.request_redraw();
    }
}

/// 返回储藏工作区和暂存区改动（含未跟踪的文件）的 git 参数。
fn stash_args(message: &str) -> [&str; 5] {
    ["stash", "push", "--include-untracked", "--message", message]
}

/// 切换到本地分支 `local`；`track` 时新建跟踪远程分支 `name` 的本地分支。
fn run_switch(
    git: &Git,
    root: &Path,
    name: &str,
    local: &str,
    track: bool,
) -> Result<String, String> {
    if track {
        git.operate(root, &["switch", "--track", "--end-of-options", name])
    } else {
        git.operate(root, &["switch", "--end-of-options", local])
    }
}

/// 执行包含拉取的操作 `pull`（拉取、同步），完成后检查未提交的更改是否已恢复（见
/// [`Git::carried`]），返回操作的结果和后续动作：未能完整恢复时提示更改所在的位置。`what` 是
/// 操作的名称。
pub(super) fn keep_changes<T>(
    git: &Git,
    root: &Path,
    what: &str,
    pull: impl FnOnce(&Git) -> Result<T, String>,
) -> Result<(T, Then), String> {
    let before = git.stash_tip(root);
    let summary = pull(git)?;
    let then = match git.carried(root, before.as_deref()) {
        Carried::Restored => Then::Nothing,
        Carried::Conflicted => Then::Warn(t!("git-carry-conflict", what = what)),
        Carried::Kept => Then::Warn(t!("git-carry-kept", what = what)),
    };
    Ok((summary, then))
}

/// 切换失败后恢复储藏，返回写明更改位置的错误。
fn restored_error(git: &Git, root: &Path, error: &str) -> String {
    match git.operate(root, &["stash", "pop"]) {
        Ok(_) => t!("git-switch-failed-restored", error = error),
        Err(_) => t!("git-switch-failed-stashed", error = error),
    }
}

/// 继续 `operation` 时提示中「做的是什么」的文字。
fn continue_what(operation: Operation) -> &'static str {
    match operation {
        Operation::Merge => t!("git-op-continue-merge"),
        Operation::Rebase => t!("git-op-continue-rebase"),
        Operation::CherryPick => t!("git-op-continue-cherry-pick"),
        Operation::Revert => t!("git-op-continue-revert"),
    }
}

/// 中止 `operation` 时提示中「做的是什么」的文字。
fn abort_what(operation: Operation) -> &'static str {
    match operation {
        Operation::Merge => t!("git-op-abort-merge"),
        Operation::Rebase => t!("git-op-abort-rebase"),
        Operation::CherryPick => t!("git-op-abort-cherry-pick"),
        Operation::Revert => t!("git-op-abort-revert"),
    }
}

pub(crate) mod picker;

#[cfg(test)]
mod tests;
