//! Git 模块的界面线程部分：决定何时刷新仓库状态，收到结果后更新文件树、状态栏和已打开的差异；
//! 读取文件在 `HEAD` 中的版本并打开差异。
//!
//! 模块关闭或打开的目录不在仓库中时不查找 `git`，也不启动线程。`git status` 在后台线程上运行，
//! 同一时刻最多一次，运行期间又有变化时在完成后补做一次。打开差异时，读取 `HEAD` 与磁盘内容、
//! 判断二进制与大小、解码和逐行比较都在后台完成，界面线程只提供文档内容的快照。

pub(crate) mod blame;
pub(crate) mod branch;
pub(super) mod changes;
pub(crate) mod combined;
pub(super) mod commit;
mod diffs;
pub(super) mod fetch;
pub(crate) mod form;
pub(crate) mod location_picker;
pub(crate) mod log;
pub(crate) mod merge;
pub(crate) mod panel;
pub(crate) mod question;
pub(crate) mod rebase;
mod refresh;
mod repo_list;
mod repository;
pub(super) mod rewrite;
mod stepping;
pub(crate) mod worktree;

use crate::app::NoticeAction;
use crate::app::scroll::set_scroll_line;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ropey::Rope;
use winc_editor::diff_view::{BlameSource, DiffSide, DiffView};
use winc_editor::encoding::{decode_known, decode_unknown};
use winc_editor::format::Cancel;
use winc_editor::git::{self, Branch, Change, Git, Stash, Status, Unavailable, Worktree};
use winc_editor::shell::Shell;
use winc_editor::{DocumentId, Encoding, LARGE_FILE, looks_binary};
use winc_i18n::t;

use super::diff::worktree_title;
pub(crate) use diffs::DiffKind;
// Git 的测试（只在 Unix 上编译）按 `crate::app::git::Target` 打开差异。
use super::{App, Focus, UserEvent};
use crate::app::NoticeKind;
use crate::layout::SidebarView;
#[cfg(all(test, unix))]
pub(super) use diffs::Target;
use diffs::{DiffTab, Prepared};

/// Git 视图可见时补查本地状态的间隔；系统漏发通知时定期重新读取（取自调参）。
const STATUS_RECHECK: Duration = Duration::from_secs(5);

/// 界面线程与各后台任务共用的 [`Git`]。
///
/// 第一个找到 `git` 的任务写入结果，之后的任务直接使用；未找到时不写入，由下一个任务重新查找
/// （用户可能刚安装 `git`）。每个 [`Git`] 各自记录仓库的信任状态，共用同一个实例才能使用户对
/// 仓库配置的信任决定在各任务之间一致。
#[derive(Debug, Clone, Default)]
pub(crate) struct SharedGit(Arc<Mutex<Option<Git>>>);

impl SharedGit {
    /// 返回已找到的 `git`；尚未查找或未找到时返回 `None`。
    pub(crate) fn get(&self) -> Option<Git> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 返回共用的 `git`，尚未找到时就地查找并写入；两个任务同时找到时保留先写入的结果。
    ///
    /// 第一次查找需要等待读取用户 shell 的环境，因此只在后台线程上调用。
    fn resolve(&self, shell: &Shell, program: Option<&str>) -> Result<Git, Unavailable> {
        if let Some(git) = self.get() {
            return Ok(git);
        }
        let found = Git::find_with(shell, program)?;
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(slot.get_or_insert(found).clone())
    }

    /// 记录已找到的 `git`，之后不再查找。
    pub(super) fn keep(&self, git: Git) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(git);
    }

    /// 清除已找到的 `git`（关闭目录或 Git 模块时），下次使用时重新查找。
    pub(super) fn forget(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// 返回后台任务找不到 `git` 时向用户显示的原因。
pub(super) fn unusable(e: &Unavailable) -> String {
    t!("git-unusable", error = e.to_string())
}

/// 一次后台刷新的结果。
struct Outcome {
    /// 刷新的仓库；结果送达时当前仓库已更换则丢弃。
    root: PathBuf,
    /// 读取到的状态；失败时为原因。
    status: Result<Status, String>,
    /// 失败的原因是否为没有可用的 `git`。
    missing: bool,
    /// 分支、储藏与工作树列表。
    ///
    /// 状态读取失败，或本次只刷新状态（[`App::git_refresh_status`]）时不读取。
    refs: Option<Refs>,
}

/// 与状态一起刷新的列表；这些列表只随 `.git` 中的内容变化。
struct Refs {
    /// 本地和远程分支，本地的在前。
    branches: Vec<Branch>,
    /// 各个标签的名字，最近创建的在前。
    tags: Vec<String>,
    /// 仓库配置的远程名。
    remotes: Vec<String>,
    /// 仓库配置的远程仓库和地址。
    remote_list: Vec<winc_editor::git::Remote>,
    /// 发布当前分支时使用的远程（[`winc_editor::git::Git::publish_target`]）；没有远程时为
    /// `None`。
    publish_remote: Option<String>,
    /// 拉取当前分支时是否配置为变基（[`winc_editor::git::Git::pull_rebases`]）。
    pull_rebase: bool,
    /// 仓库的公共 Git 目录；取不到时为 `None`。
    common_dir: Option<PathBuf>,
    /// 储藏，最新的在前。
    stashes: Vec<Stash>,
    /// 最近切换到过的分支名，最近的在前。
    recent_branches: Vec<String>,
    /// 工作树列表；读取失败（`git` 版本过旧、超时）时为 `None`，保留此前的列表，
    /// 否则被占用的分支、窗口标题和新建工作树的存放位置都会出错。
    worktrees: Option<Vec<Worktree>>,
    /// 仓库所在系统中用户的主目录（[`winc_editor::wsl::home_dir`]）。
    home: Option<PathBuf>,
    /// 仓库所在系统中 `git` 的主、次版本号；读不出时为 `None`。
    version: Option<(u32, u32)>,
    /// 最近提交的作者和自己写的提交信息（见 [`Git::recent_history`]）；这次没有读时为 `None`。
    history: Option<winc_editor::git::RecentHistory>,
}

/// 读取分支和改动数的位置数上限（个）。
///
/// 每个位置运行一次 `git status`；该上限避免包含大量仓库的目录在后台占用过多进程。
const SUMMARY_REPOSITORIES: usize = 32;

/// 后台深层查找的结果：（查找的目录，找到的仓库）。
type ScannedRepositories = (PathBuf, Vec<PathBuf>);
/// 等着确认的推送：要做的推送、会送出的提交的标题（最新的在前）和总数。
pub type PushPreview = (commit::PushIntent, Vec<String>, usize);

/// Git 模块的状态。
pub(crate) struct GitState {
    /// 当前仓库的根目录；模块关闭或目录中没有仓库时为 `None`。
    pub root: Option<PathBuf>,
    /// 打开的目录中可用的仓库（见 [`App::git_discover`]）；有多个时由位置选择器列出。
    pub repositories: Vec<PathBuf>,
    /// 上次跟随的标签（[`App::git_follow_active`]）；只在切换标签时查找文件所在的仓库。
    followed: Option<DocumentId>,
    /// 用户对该目录仓库列表的调整：手动添加的仓库和从列表中移除的仓库。
    pub(crate) edits: winc_config::SessionRepositories,
    /// 后台查找手动添加的仓库的结果，见 [`App::git_take_added`]。
    added: Arc<Mutex<Option<repo_list::Added>>>,
    /// 上次查找嵌套仓库时的（状态的代数，各仓库状态的版本），见
    /// [`App::adopt_nested_repositories`]。
    nested_seen: (u64, u64),
    /// 各仓库最近一次读取的状态；位置选择器据此显示分支和改动数，切换仓库时先显示该状态。
    summaries: Arc<Mutex<HashMap<PathBuf, Status>>>,
    /// `summaries` 每改一次加 1（后台读到新状态、切换仓库时放进、取出）：按它缓存由这些状态
    /// 排出来的东西（见 `App::commit_other_rows`）。
    summaries_version: Arc<std::sync::atomic::AtomicU64>,
    /// 上次按其他仓库的状态更新差异标签时 `summaries_version` 的值（见
    /// [`App::refresh_other_diffs`]）。
    diffs_seen: u64,
    /// 各仓库最近一次读取的工作树；位置选择器据此在仓库下列出工作树。
    summary_worktrees: Arc<Mutex<HashMap<PathBuf, Vec<Worktree>>>>,
    /// 离开各仓库时记下的分支、储藏、工作树和上一次提交，切换回去时先显示。
    kept: HashMap<PathBuf, location_picker::KeptRefs>,
    /// 排好的全部位置，见 `App::cached_places`。
    places: std::cell::RefCell<Option<location_picker::PlacesCache>>,
    /// 当前仓库各工作树相对当前 `HEAD` 的差异，与 `summaries` 一起读取。
    summary_divergence: Arc<Mutex<HashMap<PathBuf, location_picker::Divergence>>>,
    /// 正在运行的读取；重新读取时取消。
    summaries_running: Option<Cancel>,
    /// 后台深层查找（[`App::git_scan_repositories`]）的结果。
    scanned: Arc<Mutex<Option<ScannedRepositories>>>,
    /// 正在运行的深层查找；重新查找时取消。
    scan_running: Option<Cancel>,
    /// 共用的 `git`（[`SharedGit`]），第一次使用时在后台查找。
    pub(super) tool: SharedGit,
    /// 最近一次成功的 `git status`。
    pub status: Option<Status>,
    /// 本地和远程分支（本地分支在前），与状态一起刷新。
    pub branches: Vec<Branch>,
    /// 当前仓库的各个标签，最近创建的在前；分支选择器的「标签」一组列出它们。
    pub tags: Vec<String>,
    /// 仓库配置的远程名，与状态一起刷新。
    pub remotes: Vec<String>,
    /// 仓库配置的远程仓库和地址，与状态一起刷新。
    pub remote_list: Vec<winc_editor::git::Remote>,
    /// 发布当前分支时使用的远程，与状态一起刷新；没有远程时为 `None`。
    pub publish_remote: Option<String>,
    /// 拉取当前分支时是否配置为变基，与状态一起刷新；「拉取自…」的「变基」默认按它勾选。
    pub pull_rebase: bool,
    /// 储藏，最新的在前，与状态一起刷新。
    pub stashes: Vec<Stash>,
    /// 最近切换到过的分支名，最近的在前，与状态一起刷新。
    pub recent_branches: Vec<String>,
    /// 仓库的工作树，主工作树在最前，与状态一起刷新；只有一个工作树时为空。
    pub worktrees: Vec<Worktree>,
    /// 仓库所在系统中用户的主目录，与分支一起刷新；`git.worktreeRoot` 开头的 `~/` 展开为该目录。
    pub home: Option<PathBuf>,
    /// 最近一次刷新失败的原因（找不到 `git`、`git` 报错）；刷新成功后清除。
    pub problem: Option<String>,
    /// `problem` 是否为没有可用的 `git`：这时提交视图不写「点击刷新按钮重试」。
    pub missing: bool,
    /// 状态的版本号，每更新一次加 1；提交视图据此判断绘制时记录的行是否仍然有效。
    pub generation: u64,
    /// 文件树使用的各路径状态（[`Status::decorations`]）。
    pub decorations: HashMap<PathBuf, Change>,
    /// 被忽略且未跟踪的文件和目录（[`Status::ignored`]）。
    ///
    /// 文件树以淡色绘制这些路径，其下的变化不触发状态刷新。
    ignored_paths: HashSet<PathBuf>,
    /// 正在运行的刷新；`None` 表示没有在运行。
    running: Option<Cancel>,
    /// 下一次补查本地状态的时刻；Git 视图不可见时不安排。
    recheck_due: Option<Instant>,
    /// 运行期间是否又收到刷新请求；为真时完成后再刷新一次。
    again: bool,
    /// 下一次刷新是否同时读取分支与储藏（由 [`App::git_refresh`] 设置，读取时清除）。
    refs_wanted: bool,
    /// 刚完成的操作可能改动了引用（标签、远程分支）：下一次读到状态时重新读取已打开的日志。
    ///
    /// 状态中只能看出 `HEAD` 和分支的变化，标签的增删看不出。
    logs_outdated: bool,
    /// 已打开的 Git 差异标签；状态刷新后按需原地更新，标签关闭时移除。
    diff_tabs: HashMap<DocumentId, DiffTab>,
    /// 「未提交的更改」标签；没有打开时为 `None`。
    pub(crate) combined: Option<combined::CombinedTab>,
    /// 最近一次切换文件时的（差异标签，切换到的文件）。
    stepped: Option<(DocumentId, PathBuf)>,
    /// 最近一次预览请求的序号。
    pub(super) preview_seq: u64,
    /// 本次运行中已询问过是否信任的仓库配置指纹（[`git::Risks::fingerprint`]）。
    ///
    /// 用户选择不信任后本次运行不再询问，下次启动时重新询问。
    asked_trust: Vec<u64>,
    sender: Sender<Outcome>,
    receiver: Receiver<Outcome>,
    prepared: (Sender<Prepared>, Receiver<Prepared>),
    /// 提交视图：勾选、提交信息。
    pub commit: commit::CommitState,
    /// 行号栏的改动标记和已展开的改动卡片。
    pub changes: changes::LineChanges,
    /// 提交视图上下文一行中各控件宽度的过渡。
    pub context_row: commit::ContextRow,
    /// 日志标签的后台通道和点击区域。
    pub log: log::LogState,
    /// 窗口底部的 Git 面板和其中的日志。
    pub panel: panel::GitPanel,
    /// 逐行追溯（`git blame`）的状态。
    pub blame: blame::BlameState,
    /// 正在进行的分支操作。
    pub branch: branch::BranchState,
    /// 文件内合并冲突的解决状态：识别出的冲突与可点击区域。
    pub merge: merge::MergeState,
    /// 位置选择器是否列出全部仓库和它们的工作树；否则只列当前仓库的工作树。
    pub location_all: bool,
    /// 后台读到的、等着确认的推送：要做的推送、会送出的提交的标题和总数（见
    /// [`App::ask_push`]）。
    pub push_preview: Arc<Mutex<Option<PushPreview>>>,
    /// 推送或拉取对话框改了目标后在后台读到的提交（见 [`App::refresh_transfer_list`]）。
    pub(super) transfer_commits: Arc<Mutex<Option<form::TransferCommits>>>,
    /// 已提示过 `git` 版本过低：每次运行只提示一次。
    pub(super) old_git_warned: bool,
    /// 定时获取的状态（见 [`fetch`]）。
    pub(super) auto_fetch: fetch::AutoFetch,
    /// 当前仓库最近提交的作者（`名字 <邮箱>`），提交按钮的菜单里用来添加共同作者。
    pub(crate) authors: Vec<String>,
    /// 当前仓库里自己最近写的提交信息，最近的在前：在别处（终端、其他工具）提交的也在其中，
    /// 补在本程序记住的提交信息后面。
    pub(crate) own_messages: Vec<String>,
    /// 仓库配置的当前用户的邮箱（见 [`winc_editor::git::RecentHistory::me`]）；日志的作者筛选里
    /// 「我」按它筛选。没有读到时为空。
    pub(crate) my_email: String,
    /// 读作者时的仓库和它的 `HEAD`：换了仓库或 `HEAD` 动过（提交、拉取、切换）时重读。
    authors_for: Option<(PathBuf, Option<String>)>,
    /// 已经读过 `git` 版本的仓库：每个仓库每次运行只读一次，不随每次刷新多运行一条命令。
    version_read: HashSet<PathBuf>,
    /// 开着的 Git 输出标签已经显示到记录的第几个字节（见 `winc_editor::git::output_snapshot`）；
    /// 标签里是「暂无输出」的说明时为 0。
    pub output_shown: u64,
    /// 分支选择器里改变过默认展开状态的目录：（是否远程，目录路径）。
    pub branch_folders: HashSet<(bool, String)>,
}

impl Default for GitState {
    fn default() -> Self {
        let (sender, receiver) = channel();
        GitState {
            root: None,
            repositories: Vec::new(),
            followed: None,
            edits: Default::default(),
            added: Arc::default(),
            nested_seen: (u64::MAX, u64::MAX),
            summaries: Arc::default(),
            summaries_version: Arc::default(),
            diffs_seen: 0,
            summary_worktrees: Arc::default(),
            kept: HashMap::new(),
            places: std::cell::RefCell::new(None),
            summary_divergence: Arc::default(),
            summaries_running: None,
            scanned: Arc::default(),
            scan_running: None,
            tool: SharedGit::default(),
            status: None,
            branches: Vec::new(),
            tags: Vec::new(),
            remotes: Vec::new(),
            remote_list: Vec::new(),
            publish_remote: None,
            pull_rebase: false,
            stashes: Vec::new(),
            recent_branches: Vec::new(),
            worktrees: Vec::new(),
            home: None,
            problem: None,
            missing: false,
            generation: 0,
            decorations: HashMap::new(),
            ignored_paths: HashSet::new(),
            running: None,
            recheck_due: None,
            again: false,
            refs_wanted: false,
            logs_outdated: false,
            diff_tabs: HashMap::new(),
            combined: None,
            stepped: None,
            preview_seq: 0,
            asked_trust: Vec::new(),
            sender,
            receiver,
            commit: Default::default(),
            changes: Default::default(),
            context_row: Default::default(),
            log: Default::default(),
            panel: Default::default(),
            blame: Default::default(),
            branch: Default::default(),
            merge: Default::default(),
            location_all: false,
            output_shown: 0,
            push_preview: Arc::default(),
            transfer_commits: Arc::default(),
            old_git_warned: false,
            version_read: HashSet::new(),
            authors: Vec::new(),
            own_messages: Vec::new(),
            my_email: String::new(),
            authors_for: None,
            auto_fetch: fetch::AutoFetch::default(),
            branch_folders: HashSet::new(),
            prepared: channel(),
        }
    }
}

impl GitState {
    /// 测试用：提供后台读到的仓库状态，不启动 Git 进程。
    #[cfg(test)]
    pub(in crate::app) fn summary_for_test(&self, root: PathBuf, status: Status) {
        self.summaries.lock().unwrap().insert(root, status);
        self.summaries_changed();
    }

    /// 记下各仓库的状态（`summaries`）改过了。
    pub(crate) fn summaries_changed(&self) {
        (self.summaries_version).fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    /// 各仓库状态的版本，见 `summaries_version`。
    pub(crate) fn summaries_version(&self) -> u64 {
        self.summaries_version
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// 返回文件树中 `path` 显示的状态。
    pub fn change_of(&self, path: &Path) -> Option<Change> {
        self.decorations.get(path).copied()
    }

    /// `path` 是否被 Git 忽略：`path` 本身或任一上级目录位于最近一次状态的忽略列表中。
    pub fn is_ignored(&self, path: &Path) -> bool {
        if self.ignored_paths.is_empty() {
            return false;
        }
        let root = self.root.as_deref();
        path.ancestors()
            .take_while(|p| Some(*p) != root)
            .any(|p| self.ignored_paths.contains(p))
    }

    /// 切换到仓库 `next` 之前清空状态。
    ///
    /// `next` 是同一仓库的另一个工作树时保留共用的工作树列表和远程，使徽章、位置行和同步控件
    /// 在后台读取完成之前即显示正确内容，不出现短暂的空白。
    pub(in crate::app) fn clear_for(&mut self, next: Option<&Path>) {
        let same_repository = next.is_some_and(|next| self.shares_repository(next));
        let worktrees = std::mem::take(&mut self.worktrees);
        let remotes = std::mem::take(&mut self.remotes);
        self.clear();
        if same_repository {
            self.worktrees = worktrees;
            self.remotes = remotes;
        }
    }

    /// 停止正在运行的刷新并清空状态。
    ///
    /// 未找到 `git` 的结论同样清除，下次重新查找。
    fn clear(&mut self) {
        if let Some(cancel) = self.running.take() {
            cancel.cancel();
        }
        self.again = false;
        self.recheck_due = None;
        self.refs_wanted = false;
        self.root = None;
        self.tool.forget();
        self.status = None;
        self.branches.clear();
        self.tags.clear();
        self.remotes.clear();
        self.remote_list.clear();
        self.publish_remote = None;
        self.pull_rebase = false;
        self.stashes.clear();
        self.worktrees.clear();
        self.home = None;
        self.authors.clear();
        self.own_messages.clear();
        self.my_email.clear();
        self.authors_for = None;
        self.problem = None;
        self.missing = false;
        self.generation += 1;
        self.decorations.clear();
        self.ignored_paths.clear();
    }
}

impl App {
    /// Git 模块是否已开启。
    pub(crate) fn git_enabled(&self) -> bool {
        self.config.git_enabled.unwrap_or(false)
    }

    /// 在打开的目录下查找仓库的层数（`git.repositoryScanDepth`）。
    pub(crate) fn repository_scan_depth(&self) -> usize {
        let depth = self.config.repository_scan_depth;
        depth.unwrap_or(winc_config::Config::REPOSITORY_SCAN_DEPTH)
    }

    /// 设置的 `git` 程序（`git.path`）变化后重新查找 `git`，并刷新一次。
    pub(super) fn git_program_changed(&mut self) {
        self.git.tool.forget();
        self.git_refresh();
    }

    /// 在打开的目录或模块开关变化后重新确定仓库，并刷新一次。
    ///
    /// 目录本身不是仓库而第一层子目录中有仓库时使用第一个，其余仓库在位置选择器中切换；更深的
    /// 仓库由后台查找（[`App::git_scan_repositories`]）加入。
    pub(super) fn git_folder_changed(&mut self) {
        self.git.repositories.clear();
        self.load_repository_edits();
        self.git.followed = None;
        // 切换到同一仓库的另一个工作树时，已读取的状态仍然有效，切换后可立即显示。
        let same_repository = (self.editor.root()).is_some_and(|r| self.git.shares_repository(r));
        if !same_repository {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            self.git.summaries_changed();
            self.git
                .summary_worktrees
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            self.git.kept.clear();
        }
        self.git_discover();
        let root = self.git.repositories.first().cloned();
        self.git_set_root(root);
        self.git_scan_repositories();
    }

    /// 在后台按配置的层数查找打开的目录下的仓库，结果由 [`App::git_take_scan`] 加入列表。
    pub(in crate::app) fn git_scan_repositories(&mut self) {
        if let Some(cancel) = self.git.scan_running.take() {
            cancel.cancel();
        }
        let Some(dir) = self
            .editor
            .root()
            .filter(|_| self.git_enabled())
            .map(Path::to_path_buf)
        else {
            return;
        };
        let depth = self.repository_scan_depth();
        let cancel = Cancel::default();
        self.git.scan_running = Some(cancel.clone());
        let slot = Arc::clone(&self.git.scanned);
        let extra = self.extra_folders();
        let added = self.git.edits.added.clone();
        self.spawn_blocking_then(UserEvent::GitStatus, move || {
            // 本机的目录已在界面线程上查过它所在的仓库、第一层子目录和手动添加的仓库
            // （`git_discover`）；远端的目录在这里一并查找。
            let mut found = Vec::new();
            if winc_backend::is_remote(&dir) {
                found = git::find_repositories(&dir);
                let markers: Vec<PathBuf> = added.iter().map(|repo| repo.join(".git")).collect();
                let present = winc_backend::fs::metadata_many(&markers);
                let present = added.into_iter().zip(present).filter(|(_, m)| m.is_ok());
                found.extend(present.map(|(repo, _)| repo));
            }
            found.extend(git::scan_repositories(&dir, depth, &cancel));
            for folder in &extra {
                found.extend(git::scan_repositories(folder, depth, &cancel));
            }
            if !cancel.is_cancelled() {
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((dir, found));
            }
        });
    }

    /// 是否正在查找目录里的仓库：从开始查找到 [`App::git_take_scan`] 收到这个目录的结果。
    pub(in crate::app) fn git_scanning(&self) -> bool {
        self.git.scan_running.is_some()
    }

    /// 接收后台查找的结果，新的仓库加在列表末尾；尚无当前仓库时使用找到的第一个。
    ///
    /// 菜单与选择器按下标引用仓库，因此已有仓库的下标不能改变。
    pub(in crate::app) fn git_take_scan(&mut self) {
        let scanned = self
            .git
            .scanned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some((dir, found)) = scanned else {
            return;
        };
        if self.editor.root() != Some(dir.as_path()) || !self.git_enabled() {
            return;
        }
        // 这个目录的查找结束了：一个仓库都没找到时提交视图从「正在读取」换成不是仓库的说明。
        self.git.scan_running = None;
        self.request_redraw();
        if !self.git_adopt_repositories(found) {
            return;
        }
        self.git_read_summaries();
        if self.git.root.is_none() {
            let root = self.git.repositories.first().cloned();
            self.git_set_root(root);
        }
        self.location_picker_refreshed();
        self.request_redraw();
    }

    /// 重新查找打开的目录中的仓库；已加入的仓库（打开过其中的文件）仍然存在时保留。
    pub(in crate::app) fn git_discover(&mut self) {
        let Some(dir) = self.editor.root().filter(|_| self.git_enabled()) else {
            self.git.repositories.clear();
            return;
        };
        // 远端的目录不在界面线程上查找：仓库由后台的查找加入（`git_scan_repositories`），
        // 已有的列表保留。
        if winc_backend::is_remote(dir) {
            return;
        }
        let mut found = git::find_repositories(dir);
        // 附加的目录里的仓库排在后面。
        for folder in self.extra_folders() {
            for repo in git::find_repositories(&folder) {
                if !found.contains(&repo) {
                    found.push(repo);
                }
            }
        }
        // 手动添加的仓库排在查找到的之后；之前加入的仓库仍然存在时保留。移除的仓库不列出。
        let added = self.git.edits.added.iter();
        let earlier = added
            .chain(self.git.repositories.iter())
            .filter(|repo| !found.contains(repo) && winc_backend::fs::exists(repo.join(".git")));
        let earlier: Vec<PathBuf> = earlier.cloned().collect();
        for repo in earlier {
            if !found.contains(&repo) {
                found.push(repo);
            }
        }
        found.retain(|repo| !self.git_repository_removed(repo));
        self.git.repositories = found;
        self.git_read_summaries();
    }

    /// 切换到第 `index` 个仓库。
    pub(in crate::app) fn git_switch_repository(&mut self, index: usize) {
        if let Some(root) = self.git.repositories.get(index).cloned() {
            self.git_set_root(Some(root));
        }
    }

    /// 当前标签切换到其他文件时，将文件所在的仓库加入列表并切换到该仓库。
    ///
    /// 提交视图可见时不切换，以免列表突然换成另一个仓库的改动而提交到错误的仓库；正在提交、
    /// 拉取或推送，或提交信息已有内容时同样不切换，以免丢失这些内容。以上情况只加入列表。
    pub(in crate::app) fn git_follow_active(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        if self.git.followed == Some(doc.id) || !self.git_enabled() {
            return;
        }
        self.git.followed = Some(doc.id);
        // 文件历史跟随它所在的仓库。
        let history = doc.log.as_deref().filter(|v| v.query.path.is_some());
        let dir = match history {
            Some(view) => view.root.as_deref(),
            None => doc.path.as_deref().and_then(Path::parent),
        };
        if let Some(dir) = dir.map(Path::to_path_buf) {
            self.git_follow_dir(&dir);
        }
    }

    /// 文档所在的目录 `dir` 在打开的目录（或附加的目录）中的仓库里时，把当前仓库换成它；提交
    /// 视图开着、正在提交或有没写完的提交信息时不换，只把仓库加入列表。
    pub(in crate::app) fn git_follow_dir(&mut self, dir: &Path) {
        let Some(folder) = self.editor.root().filter(|_| self.git_enabled()) else {
            return;
        };
        // 打开的目录和附加的目录之外的文件不改变当前仓库。
        let inside = dir.starts_with(folder)
            || self
                .extra_folders()
                .iter()
                .any(|extra| dir.starts_with(extra));
        if !inside {
            return;
        }
        let Some(root) = git::find_root(dir) else {
            return;
        };
        // 用户从列表中移除的仓库不因打开其中的文件而回来。
        if self.git_repository_removed(&root) {
            return;
        }
        if self.git_adopt_repositories([root.clone()]) {
            self.git_read_summaries();
        }
        let stay = self.panels.sidebar.view == SidebarView::Commit
            || self.git.commit.busy
            || self.git.branch.running.is_some()
            || !self.git.commit.message.text().is_empty();
        if !stay {
            self.git_set_root(Some(root));
        }
    }

    /// 返回文件 `path` 所在的已知仓库（[`GitState::repositories`] 和当前仓库）的根目录，以及
    /// 文件在仓库中的路径；不在任何已知仓库中时返回 `None`。
    ///
    /// 打开的目录里有几个仓库时不一定是当前仓库，取包含该文件的最里层那个。原路径不在已知仓库中时
    /// 换成文档路径的写法再找一次（见 [`winc_editor::Editor::document_path`]）。
    pub(in crate::app) fn repo_of(&self, path: &Path) -> Option<(PathBuf, PathBuf)> {
        let owner = |path: &Path| {
            let repos = self.git.repositories.iter().chain(&self.git.root);
            let inside = repos.filter(|repo| path.starts_with(repo));
            inside.max_by_key(|repo| repo.as_os_str().len()).cloned()
        };
        if let Some(root) = owner(path) {
            return Some((root, path.to_path_buf()));
        }
        let path = self.editor.document_path(path);
        Some((owner(&path)?, path))
    }

    /// 将当前仓库切换为 `root`；没有变化时不做任何操作。
    pub(in crate::app) fn git_set_root(&mut self, root: Option<PathBuf>) {
        if root == self.git.root {
            return;
        }
        // 未完成的提交信息按位置保存，切换回来时恢复。多仓库时列表里的各个仓库共用一条提交信息、
        // 各文件的勾选也仍然留着：在列表里点另一个仓库只换当前仓库，不换要提交的内容。
        let carried = self
            .commit_shows_others()
            .then(|| self.git.commit.draft_text());
        let carrying = carried.is_some() && self.git.root.is_some();
        self.carry_checks_out();
        if let Some(old) = self.git.root.clone().filter(|_| carried.is_none()) {
            self.git.commit.keep_draft(old);
        }
        // 保存即将离开的仓库的状态，切换回来时可立即显示。
        if let Some(old) = self.git.root.clone() {
            // 分支和储藏随后清空，直接移走；工作树列表仍用于判断是否为同一仓库。
            let kept = location_picker::KeptRefs {
                branches: std::mem::take(&mut self.git.branches),
                stashes: std::mem::take(&mut self.git.stashes),
                worktrees: self.git.worktrees.clone(),
                last: self.git.commit.last.clone(),
            };
            self.git.kept.insert(old, kept);
        }
        if let (Some(old), Some(status)) = (self.git.root.clone(), self.git.status.take()) {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(old, status);
        }
        self.git.summaries_changed();
        // 仅更换仓库时 `git` 不变，无需重新查找，也无需重新检查已检查过的仓库配置。
        let tool = root.as_ref().and(self.git.tool.get());
        self.git.clear_for(root.as_deref());
        if let Some(tool) = tool {
            self.git.tool.keep(tool);
        }
        self.git.changes.forget();
        self.git.blame.clear();
        // 旧仓库中进行的分支操作与提交属于旧仓库：此处不再显示「正在拉取…」，不阻止新仓库的操作，
        // 也不沿用「修改上一次提交」。操作完成时只显示提示（见 `poll_branch_ops`、`poll_commit`）。
        self.git.branch.running = None;
        self.git.branch.finished = None;
        self.reset_commit_view();
        self.git.root = root;
        self.restore_kept_refs();
        // 换到的仓库的状态读到以后（下面已知的，或稍后刷新读到的），把它的勾选带回来。
        self.git.commit.carry_in = carrying;
        match carried {
            Some(text) => self.git.commit.message.set_text(&text),
            None => {
                if let Some(root) = self.git.root.clone() {
                    self.git.commit.restore_draft(&root);
                }
            }
        }
        // 先显示上次读取的状态，后台读取完成后再更新，避免提交视图短暂显示「正在读取」。
        let known = self.git.root.as_ref().and_then(|root| {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(root)
        });
        self.git.summaries_changed();
        if let (Some(status), Some(root)) = (known, self.git.root.as_deref()) {
            self.git.changes.head_is(status.oid.as_deref());
            self.git.commit.head_is(status.oid.as_deref());
            self.git.decorations = status.decorations(root);
            self.git.ignored_paths = status.ignored.iter().cloned().collect();
            self.git.status = Some(status);
            self.git.generation += 1;
            self.carry_checks_in();
        }
        self.retarget_logs();
        // 不再位于仓库中时提交视图不可用，切换回文件树；焦点位于提交视图时同样移出，
        // 否则键入的文字会进入不可见的输入框。
        if self.git.root.is_none() {
            if self.panels.sidebar.view == SidebarView::Commit {
                self.set_sidebar_view(SidebarView::Files);
            }
            if matches!(self.focus, Focus::CommitMessage | Focus::CommitList) {
                self.focus = if self.editor.active.is_some() {
                    Focus::Editor
                } else {
                    Focus::Tree
                };
            }
        }
        self.git_refresh();
        self.request_redraw();
    }

    /// 处理目录监视报告的变化：`rescan` 或 `git` 为真时刷新状态和分支，否则仅当 `paths` 中有
    /// 位于仓库内且未被忽略的路径时刷新状态。
    ///
    /// 是否被忽略以 `git` 上次给出的忽略列表为准（[`GitState::is_ignored`]）。该列表只含未跟踪的
    /// 路径，因此已跟踪的文件即使匹配忽略规则，其改动仍然触发刷新。不在列表中的新文件触发一次
    /// 刷新，之后即出现在列表中；`.gitignore` 自身不被忽略，其改动触发的刷新会得到新的列表。
    pub(super) fn git_paths_changed(&mut self, paths: &[PathBuf], rescan: bool, git: bool) {
        let Some(root) = self.git.root.as_deref() else {
            return;
        };
        if rescan || git {
            self.git_refresh();
        } else if paths
            .iter()
            .any(|p| p.starts_with(root) && !self.git.is_ignored(p))
        {
            self.git_refresh_status();
        }
    }

    /// 刷新仓库状态以及分支、储藏列表。
    ///
    /// 不在仓库中时不做任何操作；正在刷新时，完成后再补做一次。
    pub(super) fn git_refresh(&mut self) {
        self.git.refs_wanted = true;
        self.git_refresh_status();
    }

    /// 只刷新状态，分支与储藏列表保持不变。
    ///
    /// 分支与储藏只随 `.git` 中的内容变化（目录监视报告 `git`，或 winc 自身执行的 Git 操作），
    /// 这些情况都经由 [`App::git_refresh`]。工作区中每保存一次文件只运行一个 `status` 进程，
    /// 不运行 `for-each-ref` 和 `stash list`。
    pub(super) fn git_refresh_status(&mut self) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        if self.git.running.is_some() {
            self.git.again = true;
            return;
        }
        self.git.recheck_due = Some(Instant::now() + STATUS_RECHECK);
        // 尚未读取到状态时（刚打开、上次出错）同样没有分支列表，因此一并读取。
        let refs = std::mem::take(&mut self.git.refs_wanted) || self.git.status.is_none();
        let cancel = Cancel::default();
        self.git.running = Some(cancel.clone());
        let sender = self.git.sender.clone();
        let read_version = !self.git.old_git_warned && self.git.version_read.insert(root.clone());
        let head = self.git.status.as_ref().and_then(|s| s.oid.clone());
        let authors_key = (root.clone(), head);
        let read_authors = self.git.authors_for.as_ref() != Some(&authors_key);
        if read_authors {
            self.git.authors_for = Some(authors_key);
        }
        // 上次未找到 `git` 时本次在后台重新查找（用户可能刚安装）。
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            let usable = |git: Git| git.reaches(&root).then_some(git);
            let tool = tool.and_then(|git| usable(git).ok_or(Unavailable::NotInstalled));
            let missing = tool.is_err();
            let (status, refs) = match &tool {
                Err(e) => (Err(e.to_string()), None),
                Ok(git) if !refs => (git.status(&root, &cancel), None),
                // 各项互不依赖，同时读取：每一项是一条或几条 `git` 命令，远端的仓库上每条命令
                // 要等一次网络往返，依次执行时总耗时是它们的和。
                Ok(git) => std::thread::scope(|scope| {
                    let (root, cancel) = (&root, &cancel);
                    // 先读状态：其中对仓库配置的检查结果会被记住，随后同时进行的各项直接使用，
                    // 不各自再检查一遍。
                    let status = git.status(root, cancel);
                    if status.is_err() {
                        return (status, None);
                    }
                    let branches = scope.spawn(|| git.branches(root, cancel));
                    let tags = scope.spawn(|| git.tags(root, cancel));
                    let remotes = scope.spawn(|| git.remotes(root, cancel));
                    let remote_list = scope.spawn(|| git.remote_list(root, cancel));
                    let publish_remote = scope.spawn(|| git.publish_target(root));
                    let pull_rebase = scope.spawn(|| git.pull_rebases(root) == Some(true));
                    let common_dir = scope.spawn(|| git.common_dir(root, cancel));
                    let stashes = scope.spawn(|| git.stashes(root, cancel));
                    let recent_branches = scope.spawn(|| git.recent_branches(root, cancel));
                    let worktrees = scope.spawn(|| git.worktrees(root, cancel));
                    let version = read_version.then(|| scope.spawn(|| git.version(root, cancel)));
                    let history = read_authors.then(|| {
                        scope.spawn(|| {
                            let (commits, own) = (commit::AUTHOR_COMMITS, commit::OWN_MESSAGES);
                            git.recent_history(root, commits, own)
                        })
                    });
                    // 线程中的函数不会 panic；万一发生，该项按读取失败处理。
                    let refs = Refs {
                        branches: branches
                            .join()
                            .ok()
                            .and_then(Result::ok)
                            .unwrap_or_default(),
                        tags: tags.join().ok().and_then(Result::ok).unwrap_or_default(),
                        remotes: remotes.join().ok().and_then(Result::ok).unwrap_or_default(),
                        remote_list: (remote_list.join().ok())
                            .and_then(Result::ok)
                            .unwrap_or_default(),
                        publish_remote: publish_remote.join().ok().and_then(Result::ok),
                        pull_rebase: pull_rebase.join().unwrap_or(false),
                        common_dir: common_dir.join().ok().and_then(Result::ok),
                        stashes: stashes.join().ok().and_then(Result::ok).unwrap_or_default(),
                        recent_branches: (recent_branches.join().ok())
                            .and_then(Result::ok)
                            .unwrap_or_default(),
                        worktrees: worktrees.join().ok().and_then(Result::ok),
                        home: winc_editor::wsl::home_dir(root),
                        version: version.and_then(|v| v.join().ok().flatten()),
                        history: history.and_then(|h| h.join().ok().and_then(Result::ok)),
                    };
                    (status, Some(refs))
                }),
            };
            let _ = sender.send(Outcome {
                root,
                status,
                missing,
                refs,
            });
        });
    }

    /// `version` 低于 [`winc_editor::git::MINIMUM_VERSION`] 时提示一次：部分命令在旧版本上
    /// 会因不认识的选项而失败。读不出版本时不提示。
    pub(in crate::app) fn warn_old_git(&mut self, version: Option<(u32, u32)>) {
        let minimum = winc_editor::git::MINIMUM_VERSION;
        let Some(found) = version.filter(|v| *v < minimum && !self.git.old_git_warned) else {
            return;
        };
        self.git.old_git_warned = true;
        let (found, minimum) = (
            format!("{}.{}", found.0, found.1),
            format!("{}.{}", minimum.0, minimum.1),
        );
        self.notify(
            NoticeKind::Warning,
            t!(
                "git-version-too-old",
                version = found.as_str(),
                minimum = minimum.as_str()
            ),
        );
    }

    /// 在后台执行一项 Git 任务，完成后以 `done` 唤醒界面线程接收结果。
    ///
    /// 线程的启动与唤醒见 [`App::spawn_blocking_then`]。
    ///
    /// `work` 收到共用的 `git`（尚未找到时在后台查找，找不到时为原因）。`work` 运行 `git` 并等待
    /// 其结束（拉取、推送等待网络，提交等待钩子），因此使用单独的线程，不占用共用的线程池；
    /// 各 Git 功能的后台任务都由此启动。
    pub(super) fn spawn_git(
        &mut self,
        done: UserEvent,
        work: impl FnOnce(Result<Git, Unavailable>) + Send + 'static,
    ) {
        let (tool, shell) = (self.git.tool.clone(), self.format.formatter.shell().clone());
        let program = self.config.git_path.clone();
        self.spawn_blocking_then(done, move || work(tool.resolve(&shell, program.as_deref())));
    }

    /// 接收后台刷新的结果；成功时同时更新已打开的 Git 差异。
    pub(super) fn poll_git(&mut self) {
        self.git_take_scan();
        self.take_push_preview();
        self.take_transfer_commits();
        self.take_auto_fetch();
        // 其他仓库的状态由后台直接写入，读完时同样到达这里：列着其他仓库时重新绘制。
        if self.commit_shows_others() && self.commit_shown() {
            self.prune_other_checked();
            self.request_redraw();
        }
        let mut changed = false;
        let mut refreshed = false;
        let mut reread = false;
        let mut others = false;
        while let Ok(outcome) = self.git.receiver.try_recv() {
            if self.git.root.as_deref() != Some(outcome.root.as_path()) {
                continue;
            }
            self.git.running = None;
            // 这份状态读取于暂存的操作写完之前：不采用，随后重新读取。
            if self.git.commit.outdates_status() {
                self.git.refs_wanted |= outcome.refs.is_some();
                reread = true;
                continue;
            }
            let had_refs = outcome.refs.is_some();
            match outcome.status {
                Ok(status) => {
                    // `HEAD`、分支或标签变化（提交、切换、新建、删除）时，重新读取已打开的日志。
                    let moved = self.git.status.as_ref().is_some_and(|old| {
                        old.oid != status.oid
                            || outcome.refs.as_ref().is_some_and(|refs| {
                                self.git.branches != refs.branches || self.git.tags != refs.tags
                            })
                    });
                    let moved = std::mem::take(&mut self.git.logs_outdated) || moved;
                    self.git.changes.head_is(status.oid.as_deref());
                    self.git.commit.head_is(status.oid.as_deref());
                    self.git.decorations = status.decorations(&outcome.root);
                    self.git.ignored_paths = status.ignored.iter().cloned().collect();
                    // 有变化的文件数超过上限时，只在首次超过时提示，之后的刷新不再重复提示。
                    let was_truncated = self.git.status.as_ref().is_some_and(|s| s.truncated);
                    if status.truncated && !was_truncated {
                        let count = winc_editor::git::STATUS_LIMIT;
                        self.notify(
                            NoticeKind::Warning,
                            t!("git-status-truncated", count = count),
                        );
                    }
                    let rows = self.commit_shown().then(|| self.commit_rows());
                    let old = self.git.status.replace(status);
                    self.carry_checks_in();
                    self.begin_commit_shift(old, rows);
                    if let Some(refs) = outcome.refs {
                        let before = std::mem::replace(&mut self.git.branches, refs.branches);
                        self.git.stashes = refs.stashes;
                        self.git.tags = refs.tags;
                        self.git.recent_branches = refs.recent_branches;
                        self.git.remotes = refs.remotes;
                        self.git.remote_list = refs.remote_list;
                        self.git.publish_remote = refs.publish_remote;
                        self.git.pull_rebase = refs.pull_rebase;
                        self.git.home = refs.home;
                        self.warn_old_git(refs.version);
                        if let Some(history) = refs.history {
                            self.git.authors = history.authors;
                            self.git.own_messages = history.own;
                            self.git.my_email = history.me;
                            self.git.commit.message_history_changed();
                        }
                        if let Some(watcher) = &mut self.disk.watcher {
                            watcher.watch_git_dir(refs.common_dir.as_deref());
                        }
                        self.branch_picker_refreshed(&before);
                        let mut listed = false;
                        if let Some(worktrees) = refs.worktrees
                            && self.git.worktrees != worktrees
                        {
                            self.git.worktrees = worktrees;
                            self.location_picker_refreshed();
                            self.update_window_title();
                            listed = true;
                        }
                        // 只有 `.git` 变化或用户点击刷新时才执行到此处，其他位置也可能已变化。
                        // 这些位置不可见时不读取，切换到提交视图或打开选择器时再读取。
                        if listed || self.commit_shown() || self.picker_lists_locations() {
                            self.git_read_summaries();
                        }
                    }
                    self.git.problem = None;
                    self.git.missing = false;
                    self.fetch_last_commit();
                    refreshed = true;
                    // 只读了状态的这次刷新没有读其他仓库：列着其他仓库时补读，文件保存后各仓库的
                    // 更改一起更新。
                    others |= !had_refs;
                    if had_refs {
                        self.start_waiting_logs();
                    }
                    if moved {
                        self.reload_logs();
                        self.git.commit.history.outdate();
                    }
                }
                Err(e) => {
                    self.git.status = None;
                    self.git.decorations.clear();
                    self.git.problem = Some(e);
                    self.git.missing = outcome.missing;
                    self.start_waiting_logs();
                }
            }
            self.git.generation += 1;
            changed = true;
        }
        // 暂存的操作仍在进行时由其结束后读取（见 `App::staging_finished`）。
        if reread && !self.git.commit.staging_active() {
            self.git_refresh_status();
        }
        if changed {
            self.check_git_trust();
            if std::mem::take(&mut self.git.again) {
                self.git_refresh_status();
            }
            if others && self.commit_shows_others() && self.commit_shown() {
                self.git_read_summaries();
            }
            if refreshed {
                self.refresh_git_diffs();
                self.refresh_combined();
                self.sync_index_texts();
                self.prefill_merge_message();
            }
            // 改动全部提交后，提交卡片随之收起。
            self.settle_commit_focus();
            self.request_redraw();
        }
        self.refresh_other_diffs();
        self.git_take_added();
        self.adopt_nested_repositories();
    }

    /// 检查仓库配置是否会让 `git` 运行命令（[`Git::untrusted`]），必要时询问用户是否信任。
    ///
    /// 信任过相同配置时正常使用，否则询问用户一次。用户回答之前，读取状态时不运行这些命令，
    /// 修改仓库的操作被拒绝；被拒绝的操作的失败提示上可以再次确认（[`App::review_git_trust`]）。
    fn check_git_trust(&mut self) {
        let (Some(root), Some(git)) = (self.git.root.clone(), self.git.tool.get()) else {
            return;
        };
        // 状态读取成功时后台已检查过配置，`untrusted` 只读取检查结果，不在界面线程上运行 `git`。
        if self.git.status.is_none() {
            return;
        }
        let Some(risks) = git.untrusted(&root) else {
            return;
        };
        if self.state.trusted_repo(&root) == Some(risks.fingerprint) {
            git.trust(&root, risks.fingerprint);
            return;
        }
        if self.git.asked_trust.contains(&risks.fingerprint) {
            return;
        }
        // 窗口已有其他提示框时不显示，也不记录指纹，下次刷新时再询问。
        if self.ask_git_trust(&root, &risks) {
            self.git.asked_trust.push(risks.fingerprint);
        }
    }

    /// 询问用户是否信任仓库 `root` 检查出的配置 `risks`，返回是否显示了提示框。
    ///
    /// 没有窗口或窗口已有其他提示框时不显示。回答见 [`App::answer_git_trust`]。
    fn ask_git_trust(&mut self, root: &Path, risks: &winc_editor::git::Risks) -> bool {
        use crate::app::AlertButton;

        if self.window.handle.is_none() {
            return false;
        }
        let name = winc_editor::display_name(root).into_owned();
        let message = t!("git-trust-confirm", name = name);
        let mut detail = t!("git-trust-confirm-detail").to_owned();
        detail.push('\n');
        // 详情中最多列出的配置项数（行）；取自调参：条目更多时详情的高度超出窗口。
        const SHOWN: usize = 8;
        for item in risks.items.iter().take(SHOWN) {
            detail.push_str("\n· ");
            detail.push_str(item);
        }
        if risks.items.len() > SHOWN {
            detail.push('\n');
            detail.push_str(&t!("git-trust-more", count = risks.items.len()));
        }
        let buttons = vec![
            AlertButton::cancel(t!("git-trust-deny")),
            AlertButton::new(t!("git-trust-allow")),
        ];
        let (root, fingerprint) = (root.to_path_buf(), risks.fingerprint);
        self.present_alert(&message, &detail, buttons, None, move |i, _| {
            Some(UserEvent::GitTrust(root.clone(), fingerprint, i == 1))
        })
    }

    /// 操作因仓库 `root` 的配置不可信而被拒绝时，失败提示上的「信任此仓库…」按钮；否则为
    /// `None`。
    ///
    /// 只读取后台检查的结果，不运行 `git`。
    pub(in crate::app) fn git_trust_action(&self, root: Option<&Path>) -> Option<NoticeAction> {
        let root = root?;
        let git = self.git.tool.get()?;
        git.known_untrusted(root)?;
        Some(NoticeAction::TrustGitRepository(root.to_path_buf()))
    }

    /// 再次询问是否信任仓库 `root`：失败提示上的「信任此仓库…」按钮。之前回答过「不信任」时同样
    /// 询问；配置已不需要信任时不询问。
    pub(in crate::app) fn review_git_trust(&mut self, root: &Path) {
        let risks = self
            .git
            .tool
            .get()
            .and_then(|git| git.known_untrusted(root));
        if let Some(risks) = risks {
            self.ask_git_trust(root, &risks);
        }
    }

    /// 处理提示框中对是否信任仓库 `root` 配置的回答。
    pub(super) fn answer_git_trust(&mut self, root: &Path, fingerprint: u64, trusted: bool) {
        if !trusted {
            return;
        }
        let trusted = root.to_path_buf();
        self.update_state(move |state| state.trust_repo(&trusted, fingerprint));
        if let Some(git) = self.git.tool.get() {
            git.trust(root, fingerprint);
        }
        self.git_refresh();
    }

    /// 合并、拣选或还原中断时，将 `git` 生成的提交信息（`MERGE_MSG`）填入提交信息框。
    ///
    /// 每次中断只填入一次；提交信息框已有内容时不修改。
    fn prefill_merge_message(&mut self) {
        use winc_editor::git::Operation;
        let operation = self.git.status.as_ref().and_then(|s| s.operation);
        if !matches!(
            operation,
            Some(Operation::Merge | Operation::CherryPick | Operation::Revert)
        ) {
            self.git.commit.prefilled = false;
            return;
        }
        if self.git.commit.prefilled || !self.git.commit.message.text().trim().is_empty() {
            return;
        }
        let (Some(root), Some(git)) = (self.git.root.as_deref(), self.git.tool.get()) else {
            return;
        };
        if let Some(message) = git.merge_message(root) {
            self.git.commit.message.set_text(&message);
            self.git.commit.prefilled = true;
        }
    }

    /// 在标签 `id`（文件 `path`）关闭后释放其逐行追溯、改动标记、差异标签记录和日志的 `git log`。
    ///
    /// 没有其他标签打开同一文件时，同时释放该文件在 `HEAD` 中的内容（基准）。
    pub(super) fn forget_git_document(&mut self, id: DocumentId, path: Option<&Path>) {
        self.git.blame.forget(id);
        self.git.log.forget(id);
        self.git.changes.forget_document(id);
        self.git.diff_tabs.remove(&id);
        if self.git.combined.as_ref().is_some_and(|t| t.doc == id) {
            self.git.combined = None;
        }
        if let Some(path) = path
            && !self
                .editor
                .documents
                .iter()
                .any(|d| d.path.as_deref() == Some(path))
        {
            self.git.changes.forget_base(path);
        }
    }
}
