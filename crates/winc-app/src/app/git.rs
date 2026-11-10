//! Git 模块的界面线程部分：决定何时刷新仓库状态，收到结果后更新文件树、状态栏和已打开的差异；
//! 读取文件在 `HEAD` 中的版本并打开差异。
//!
//! 模块关闭或打开的目录不在仓库中时不查找 `git`，也不启动线程。`git status` 在后台线程上运行，
//! 同一时刻最多一次，运行期间又有变化时在完成后补做一次。打开差异时，读取 `HEAD` 与磁盘内容、
//! 判断二进制与大小、解码和逐行比较都在后台完成，界面线程只提供文档内容的快照。

pub(crate) mod blame;
pub(crate) mod branch;
pub(super) mod changes;
pub(super) mod commit;
pub(crate) mod form;
pub(crate) mod location_picker;
pub(crate) mod log;
pub(crate) mod merge;
pub(crate) mod question;
pub(crate) mod rebase;
mod repository;
pub(crate) mod worktree;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use ropey::Rope;
use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::encoding::{decode_known, decode_unknown};
use winc_editor::format::Cancel;
use winc_editor::git::{self, Branch, Change, Git, Stash, Status, Unavailable, Worktree};
use winc_editor::shell::Shell;
use winc_editor::{DocumentId, Encoding, LARGE_FILE, looks_binary};
use winc_i18n::t;

use super::diff::worktree_title;
use super::{App, Focus, UserEvent};
use crate::layout::SidebarView;

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
    fn resolve(&self, shell: &Shell) -> Result<Git, Unavailable> {
        if let Some(git) = self.get() {
            return Ok(git);
        }
        let found = Git::find(shell)?;
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
    /// 分支、储藏与工作树列表。
    ///
    /// 状态读取失败，或本次只刷新状态（[`App::git_refresh_status`]）时不读取。
    refs: Option<Refs>,
}

/// 与状态一起刷新的列表；这些列表只随 `.git` 中的内容变化。
struct Refs {
    /// 本地和远程分支，本地的在前。
    branches: Vec<Branch>,
    /// 仓库配置的远程名。
    remotes: Vec<String>,
    /// 仓库配置的远程仓库和地址。
    remote_list: Vec<winc_editor::git::Remote>,
    /// 发布当前分支时使用的远程（[`winc_editor::git::Git::publish_target`]）；没有远程时为
    /// `None`。
    publish_remote: Option<String>,
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
}

/// 差异的打开方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    /// 新建差异标签（或替换同名的差异标签）并切换到该标签。
    Open,
    /// 提交视图的预览标签，参数为请求序号。只采用最新的请求，快速切换选择时，
    /// 先发出而后送达的结果被丢弃。
    Preview(u64),
    /// 原地更新已打开的差异标签，不切换标签，不改变滚动位置。
    Refresh(DocumentId),
}

/// 后台准备好的差异。
struct Prepared {
    /// 进行比较的仓库；之后更换了目录时丢弃结果。
    root: PathBuf,
    /// 比较的文件。
    path: PathBuf,
    /// 比较时 `HEAD` 的提交号，左侧内容取自该提交。
    oid: Option<String>,
    target: Target,
    /// 比较结果；无法比较时为原因（二进制、过大、无法读取）。
    view: Result<DiffView, String>,
}

/// 发起比较时编辑器中该文件的快照。
struct Snapshot {
    /// 发起比较时的文本。
    text: Rope,
    /// 发起比较时的文档版本号，记进差异视图（`DiffView::source`）。
    revision: u64,
    /// 文档的编码，左侧内容按该编码解码。
    encoding: Encoding,
    /// 发起比较时文档是否有未保存的修改。
    dirty: bool,
    /// 文档的语言，左右两侧都按该语言着色。
    language: &'static winc_syntax::Language,
}

/// 已打开的 Git 差异标签。
struct DiffTab {
    /// 比较的文件。
    path: PathBuf,
    /// 左侧内容对应的 `HEAD` 提交号；`HEAD` 变化时才重新读取左侧内容。
    oid: Option<String>,
}

/// 读取分支和改动数的位置数上限（个）。
///
/// 每个位置运行一次 `git status`；该上限避免包含大量仓库的目录在后台占用过多进程。
const SUMMARY_REPOSITORIES: usize = 32;

/// 后台深层查找的结果：（查找的目录，找到的仓库）。
type ScannedRepositories = (PathBuf, Vec<PathBuf>);

/// Git 模块的状态。
pub(crate) struct GitState {
    /// 当前仓库的根目录；模块关闭或目录中没有仓库时为 `None`。
    pub root: Option<PathBuf>,
    /// 打开的目录中可用的仓库（见 [`App::git_discover`]）；有多个时由位置选择器列出。
    pub repositories: Vec<PathBuf>,
    /// 上次跟随的标签（[`App::git_follow_active`]）；只在切换标签时查找文件所在的仓库。
    followed: Option<DocumentId>,
    /// 各仓库最近一次读取的状态；位置选择器据此显示分支和改动数，切换仓库时先显示该状态。
    summaries: Arc<Mutex<HashMap<PathBuf, Status>>>,
    /// 各仓库最近一次读取的工作树；位置选择器据此在仓库下列出工作树。
    summary_worktrees: Arc<Mutex<HashMap<PathBuf, Vec<Worktree>>>>,
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
    /// 仓库配置的远程名，与状态一起刷新。
    pub remotes: Vec<String>,
    /// 仓库配置的远程仓库和地址，与状态一起刷新。
    pub remote_list: Vec<winc_editor::git::Remote>,
    /// 发布当前分支时使用的远程，与状态一起刷新；没有远程时为 `None`。
    pub publish_remote: Option<String>,
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
    /// 运行期间是否又收到刷新请求；为真时完成后再刷新一次。
    again: bool,
    /// 下一次刷新是否同时读取分支与储藏（由 [`App::git_refresh`] 设置，读取时清除）。
    refs_wanted: bool,
    /// 已打开的 Git 差异标签；状态刷新后按需原地更新，标签关闭时移除。
    diff_tabs: HashMap<DocumentId, DiffTab>,
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
}

impl Default for GitState {
    fn default() -> Self {
        let (sender, receiver) = channel();
        GitState {
            root: None,
            repositories: Vec::new(),
            followed: None,
            summaries: Arc::default(),
            summary_worktrees: Arc::default(),
            summaries_running: None,
            scanned: Arc::default(),
            scan_running: None,
            tool: SharedGit::default(),
            status: None,
            branches: Vec::new(),
            remotes: Vec::new(),
            remote_list: Vec::new(),
            publish_remote: None,
            stashes: Vec::new(),
            recent_branches: Vec::new(),
            worktrees: Vec::new(),
            home: None,
            problem: None,
            generation: 0,
            decorations: HashMap::new(),
            ignored_paths: HashSet::new(),
            running: None,
            again: false,
            refs_wanted: false,
            diff_tabs: HashMap::new(),
            stepped: None,
            preview_seq: 0,
            asked_trust: Vec::new(),
            sender,
            receiver,
            prepared: channel(),
        }
    }
}

impl GitState {
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
        self.refs_wanted = false;
        self.root = None;
        self.tool.forget();
        self.status = None;
        self.branches.clear();
        self.remotes.clear();
        self.remote_list.clear();
        self.publish_remote = None;
        self.stashes.clear();
        self.worktrees.clear();
        self.home = None;
        self.problem = None;
        self.generation += 1;
        self.decorations.clear();
        self.ignored_paths.clear();
    }
}

/// 在后台准备一个文件的差异：读取 `HEAD` 中的版本，与编辑器中的快照（未打开时读取磁盘）比较。
fn prepare(
    git: &Git,
    root: &Path,
    path: &Path,
    snapshot: Option<Snapshot>,
    oid: Option<&str>,
) -> Result<DiffView, String> {
    let name = winc_editor::display_name(path).into_owned();
    if path.is_dir() {
        return Err(t!("git-diff-submodule", name = name));
    }
    let too_large = |len: usize| len as u64 > LARGE_FILE;
    let head = git
        .head_content(root, path, &Cancel::default())
        .map_err(|e| t!("git-diff-head-unreadable", name = name.as_str(), error = e))?;
    if head.as_ref().is_some_and(|h| too_large(h.len())) {
        return Err(t!("git-diff-too-large", name = name));
    }
    if head.as_deref().is_some_and(looks_binary) {
        return Err(t!("git-diff-binary", name = name));
    }

    // 右侧：文件已在编辑器中打开时使用编辑器中的内容（含未保存的修改），否则读取磁盘。左侧按
    // 右侧的编码解码；右侧不存在（文件已删除）时左侧自行识别编码。
    let revision = snapshot.as_ref().map(|s| s.revision);
    let (new, encoding, title, language) = match snapshot {
        Some(s) => (
            Some(s.text.to_string()),
            Some(s.encoding),
            worktree_title(s.dirty),
            s.language,
        ),
        None => {
            let language = winc_syntax::detect(&name);
            match std::fs::read(path) {
                Ok(bytes) if too_large(bytes.len()) => {
                    return Err(t!("git-diff-too-large", name = name));
                }
                Ok(bytes) if looks_binary(&bytes) => {
                    return Err(t!("git-diff-binary", name = name));
                }
                Ok(bytes) => {
                    let (text, encoding) = decode_unknown(&bytes);
                    (Some(text), Some(encoding), worktree_title(false), language)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    (None, None, t!("git-diff-side-worktree-deleted"), language)
                }
                Err(e) => {
                    let error = e.to_string();
                    return Err(t!("git-read-file-failed", name = name, error = error));
                }
            }
        }
    };
    let old = head.map(|bytes| match encoding {
        Some(encoding) => decode_known(&bytes, encoding),
        None => decode_unknown(&bytes).0,
    });
    let old_title = match (&old, oid) {
        (None, _) => t!("git-diff-side-head-missing").to_owned(),
        (Some(_), Some(oid)) => format!("HEAD · {}", &oid[..oid.len().min(7)]),
        (Some(_), None) => "HEAD".to_owned(),
    };
    // 文件已删除时右侧不可编辑。路径按编辑器的规则规范化，以便与已打开的标签匹配。
    let editable = new.is_some().then(|| winc_editor::canonical_path(path));
    let mut view = DiffView::new(
        DiffSide::new(old_title, old.as_deref().unwrap_or(""), None, language)
            .with_missing(old.is_none()),
        DiffSide::new(title, new.as_deref().unwrap_or(""), editable, language)
            .with_missing(new.is_none()),
    );
    view.source = revision;
    view.repo_path = Some(path.to_path_buf());
    Ok(view)
}

/// 返回差异标签的标题。
pub(super) fn diff_title(path: &Path) -> String {
    t!("git-diff-title", name = winc_editor::display_name(path))
}

impl App {
    /// Git 模块是否已开启。
    pub(crate) fn git_enabled(&self) -> bool {
        self.config.git_enabled.unwrap_or(false)
    }

    /// 在打开的目录或模块开关变化后重新确定仓库，并刷新一次。
    ///
    /// 目录本身不是仓库而第一层子目录中有仓库时使用第一个，其余仓库在位置选择器中切换；更深的
    /// 仓库由后台查找（[`App::git_scan_repositories`]）加入。
    pub(super) fn git_folder_changed(&mut self) {
        self.git.repositories.clear();
        self.git.followed = None;
        // 切换到同一仓库的另一个工作树时，已读取的状态仍然有效，切换后可立即显示。
        let same_repository = (self.editor.root()).is_some_and(|r| self.git.shares_repository(r));
        if !same_repository {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            self.git
                .summary_worktrees
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
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
        let depth = self
            .config
            .repository_scan_depth
            .unwrap_or(winc_config::Config::REPOSITORY_SCAN_DEPTH);
        let cancel = Cancel::default();
        self.git.scan_running = Some(cancel.clone());
        let slot = Arc::clone(&self.git.scanned);
        self.spawn_blocking_then(UserEvent::GitStatus, move || {
            let found = git::scan_repositories(&dir, depth, &cancel);
            if !cancel.is_cancelled() {
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((dir, found));
            }
        });
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
        let known = self.git.repositories.len();
        for repo in found {
            if !self.git.repositories.contains(&repo) {
                self.git.repositories.push(repo);
            }
        }
        if self.git.repositories.len() == known {
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
        let mut found = git::find_repositories(dir);
        let kept = self
            .git
            .repositories
            .iter()
            .filter(|repo| !found.contains(repo) && repo.join(".git").exists());
        let kept: Vec<PathBuf> = kept.cloned().collect();
        found.extend(kept);
        self.git.repositories = found;
        self.git_read_summaries();
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
        let worktrees = Arc::clone(&self.git.summary_worktrees);
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            let Ok(git) = tool else {
                return;
            };
            let mut left = SUMMARY_REPOSITORIES;
            for root in repositories {
                let trees = git.worktrees(&root, &cancel).unwrap_or_default();
                let paths: Vec<PathBuf> = if trees.is_empty() {
                    vec![root.clone()]
                } else {
                    let usable = trees.iter().filter(|w| w.head.is_some() && !w.prunable);
                    usable.map(|w| w.path.clone()).collect()
                };
                worktrees
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(root, trees);
                for path in paths {
                    if cancel.is_cancelled() || left == 0 {
                        return;
                    }
                    left -= 1;
                    if let Ok(status) = git.status(&path, &cancel) {
                        summaries
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(path, status);
                    }
                }
            }
        });
    }

    /// 返回第 `index` 个仓库在菜单中显示的名字：相对打开的目录的路径，目录本身使用目录名。
    pub(crate) fn git_repository_name(&self, index: usize) -> String {
        let Some(repo) = self.git.repositories.get(index) else {
            return String::new();
        };
        let relative = self
            .editor
            .root()
            .and_then(|root| repo.strip_prefix(root).ok());
        match relative {
            Some(path) if !path.as_os_str().is_empty() => path.display().to_string(),
            _ => repo.file_name().map_or_else(
                || repo.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
        }
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
        let (Some(dir), Some(folder)) = (
            doc.path.as_deref().and_then(Path::parent),
            self.editor.root(),
        ) else {
            return;
        };
        // 目录之外的文件不改变当前仓库。
        if !dir.starts_with(folder) {
            return;
        }
        let Some(root) = git::find_root(dir) else {
            return;
        };
        if !self.git.repositories.contains(&root) {
            self.git.repositories.push(root.clone());
            self.git_read_summaries();
        }
        let stay = self.sidebar.view == SidebarView::Commit
            || self.commit.busy
            || self.branch.running.is_some()
            || !self.commit.message.text().is_empty();
        if !stay {
            self.git_set_root(Some(root));
        }
    }

    /// 将当前仓库切换为 `root`；没有变化时不做任何操作。
    fn git_set_root(&mut self, root: Option<PathBuf>) {
        if root == self.git.root {
            return;
        }
        // 未完成的提交信息按位置保存，切换回来时恢复。
        if let Some(old) = self.git.root.clone() {
            self.commit.keep_draft(old);
        }
        // 保存即将离开的仓库的状态，切换回来时可立即显示。
        if let (Some(old), Some(status)) = (self.git.root.clone(), self.git.status.take()) {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(old, status);
        }
        // 仅更换仓库时 `git` 不变，无需重新查找，也无需重新检查已检查过的仓库配置。
        let tool = root.as_ref().and(self.git.tool.get());
        self.git.clear_for(root.as_deref());
        if let Some(tool) = tool {
            self.git.tool.keep(tool);
        }
        self.changes.forget();
        self.blame.clear();
        // 旧仓库中进行的分支操作与提交属于旧仓库：此处不再显示「正在拉取…」，不阻止新仓库的操作，
        // 也不沿用「修改上一次提交」。操作完成时只显示提示（见 `poll_branch_ops`、`poll_commit`）。
        self.branch.running = None;
        self.branch.finished = None;
        self.reset_commit_view();
        self.git.root = root;
        if let Some(root) = self.git.root.clone() {
            self.commit.restore_draft(&root);
        }
        // 先显示上次读取的状态，后台读取完成后再更新，避免提交视图短暂显示「正在读取」。
        let known = self.git.root.as_ref().and_then(|root| {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(root)
        });
        if let (Some(status), Some(root)) = (known, self.git.root.as_deref()) {
            self.commit.head_is(status.oid.as_deref());
            self.changes.head_is(status.oid.as_deref());
            self.git.decorations = status.decorations(root);
            self.git.ignored_paths = status.ignored.iter().cloned().collect();
            self.git.status = Some(status);
            self.git.generation += 1;
        }
        // 不再位于仓库中时提交视图不可用，切换回文件树；焦点位于提交视图时同样移出，
        // 否则键入的文字会进入不可见的输入框。
        if self.git.root.is_none() {
            if self.sidebar.view == SidebarView::Commit {
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
        // 尚未读取到状态时（刚打开、上次出错）同样没有分支列表，因此一并读取。
        let refs = std::mem::take(&mut self.git.refs_wanted) || self.git.status.is_none();
        let cancel = Cancel::default();
        self.git.running = Some(cancel.clone());
        let sender = self.git.sender.clone();
        // 上次未找到 `git` 时本次在后台重新查找（用户可能刚安装）。
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            let status = match &tool {
                Ok(git) => git.status(&root, &cancel),
                Err(e) => Err(e.to_string()),
            };
            let refs = match (&tool, &status) {
                (Ok(git), Ok(_)) if refs => Some(Refs {
                    branches: git.branches(&root, &cancel).unwrap_or_default(),
                    remotes: git.remotes(&root, &cancel).unwrap_or_default(),
                    remote_list: git.remote_list(&root, &cancel).unwrap_or_default(),
                    publish_remote: git.publish_target(&root).ok(),
                    common_dir: git.common_dir(&root, &cancel).ok(),
                    stashes: git.stashes(&root, &cancel).unwrap_or_default(),
                    recent_branches: git.recent_branches(&root, &cancel).unwrap_or_default(),
                    worktrees: git.worktrees(&root, &cancel).ok(),
                    home: winc_editor::wsl::home_dir(&root),
                }),
                _ => None,
            };
            let _ = sender.send(Outcome { root, status, refs });
        });
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
        self.spawn_blocking_then(done, move || work(tool.resolve(&shell)));
    }

    /// 接收后台刷新的结果；成功时同时更新已打开的 Git 差异。
    pub(super) fn poll_git(&mut self) {
        self.git_take_scan();
        let mut changed = false;
        let mut refreshed = false;
        while let Ok(outcome) = self.git.receiver.try_recv() {
            if self.git.root.as_deref() != Some(outcome.root.as_path()) {
                continue;
            }
            self.git.running = None;
            match outcome.status {
                Ok(status) => {
                    // `HEAD` 或分支变化（提交、切换、新建、删除）时，重新读取已打开的日志。
                    let moved = self.git.status.as_ref().is_some_and(|old| {
                        old.oid != status.oid
                            || outcome
                                .refs
                                .as_ref()
                                .is_some_and(|refs| self.git.branches != refs.branches)
                    });
                    self.changes.head_is(status.oid.as_deref());
                    self.commit.head_is(status.oid.as_deref());
                    self.git.decorations = status.decorations(&outcome.root);
                    self.git.ignored_paths = status.ignored.iter().cloned().collect();
                    // 有变化的文件数超过上限时，只在首次超过时提示，之后的刷新不再重复提示。
                    let was_truncated = self.git.status.as_ref().is_some_and(|s| s.truncated);
                    if status.truncated && !was_truncated {
                        let count = winc_editor::git::STATUS_LIMIT;
                        self.notify(t!("git-status-truncated", count = count));
                    }
                    let rows = self.commit_shown().then(|| self.commit_rows());
                    let old = self.git.status.replace(status);
                    self.begin_commit_shift(old, rows);
                    if let Some(refs) = outcome.refs {
                        let before = std::mem::replace(&mut self.git.branches, refs.branches);
                        self.git.stashes = refs.stashes;
                        self.git.recent_branches = refs.recent_branches;
                        self.git.remotes = refs.remotes;
                        self.git.remote_list = refs.remote_list;
                        self.git.publish_remote = refs.publish_remote;
                        self.git.home = refs.home;
                        if let Some(watcher) = &mut self.watcher {
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
                    self.fetch_last_commit();
                    refreshed = true;
                    if moved {
                        self.reload_logs();
                    }
                }
                Err(e) => {
                    self.git.status = None;
                    self.git.decorations.clear();
                    self.git.problem = Some(e);
                }
            }
            self.git.generation += 1;
            changed = true;
        }
        if changed {
            self.check_git_trust();
            if std::mem::take(&mut self.git.again) {
                self.git_refresh_status();
            }
            if refreshed {
                self.refresh_git_diffs();
                self.prefill_merge_message();
            }
            // 改动全部提交后，提交卡片随之收起。
            self.settle_commit_focus();
            self.request_redraw();
        }
    }

    /// 检查仓库配置是否会让 `git` 运行命令（[`Git::untrusted`]），必要时询问用户是否信任。
    ///
    /// 信任过相同配置时正常使用，否则询问用户一次。用户回答之前，读取状态时不运行这些命令，
    /// 修改仓库的操作被拒绝。
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

        if self.window.handle.is_some() {
            use crate::app::AlertButton;

            let name = winc_editor::display_name(&root).into_owned();
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
            let fingerprint = risks.fingerprint;
            let asked = self.present_alert(&message, &detail, buttons, None, {
                let root = root.clone();
                move |i, _| Some(UserEvent::GitTrust(root.clone(), fingerprint, i == 1))
            });
            // 窗口已有其他提示框时不显示，也不记录指纹，下次刷新时再询问。
            if asked {
                self.git.asked_trust.push(fingerprint);
            }
        }
    }

    /// 处理提示框中对是否信任仓库 `root` 配置的回答。
    pub(super) fn answer_git_trust(&mut self, root: &Path, fingerprint: u64, trusted: bool) {
        if !trusted {
            return;
        }
        self.update_state(|state| state.trust_repo(root, fingerprint));
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
            self.commit.prefilled = false;
            return;
        }
        if self.commit.prefilled || !self.commit.message.text().trim().is_empty() {
            return;
        }
        let (Some(root), Some(git)) = (self.git.root.as_deref(), self.git.tool.get()) else {
            return;
        };
        if let Some(message) = git.merge_message(root) {
            self.commit.message.set_text(&message);
            self.commit.prefilled = true;
        }
    }

    /// 打开当前文件的改动（命令「Git：显示当前文件的改动」）。
    pub(super) fn git_show_changes(&mut self) {
        let Some(path) = self.editor.active().and_then(|d| d.path.clone()) else {
            self.flash(t!("git-no-file-in-tab"));
            return;
        };
        self.fetch_changes(&path, Target::Open);
    }

    /// 在提交视图的预览标签中打开 `path` 的改动；依次查看多个文件时只占用一个标签。
    pub(super) fn git_preview_changes(&mut self, path: &Path) {
        self.git.preview_seq += 1;
        self.fetch_changes(path, Target::Preview(self.git.preview_seq));
    }

    /// 标签 `id` 是否为工作区文件的 Git 差异。
    pub(super) fn steps_changed_files(&self, id: DocumentId) -> bool {
        self.git.diff_tabs.contains_key(&id)
    }

    /// 返回提交视图列出的下一个（`forward`）或上一个文件：（路径，序号（从 0 起），文件总数）。
    ///
    /// 当前标签不是工作区文件的差异，或没有更多文件时返回 `None`。从最近一次切换到的文件起算：
    /// 该文件无法打开时（二进制、没有差异）标签保留原内容，再次切换即可跳过该文件。
    pub(super) fn next_changed_file(&self, forward: bool) -> Option<(PathBuf, usize, usize)> {
        let id = self.editor.active()?.id;
        let tab = self.git.diff_tabs.get(&id)?;
        let current = match &self.git.stepped {
            Some((doc, path)) if *doc == id => path,
            _ => &tab.path,
        };
        // 列表中已折叠目录内的文件不在 `commit_files` 中，此时按状态中的顺序。
        let mut files = self.commit_files();
        if !files.contains(current) {
            let entries = &self.git.status.as_ref()?.entries;
            files = entries.iter().map(|e| e.path.clone()).collect();
        }
        let at = files.iter().position(|p| p == current)?;
        let next = if forward { at + 1 } else { at.checked_sub(1)? };
        let path = files.get(next)?.clone();
        Some((path, next, files.len()))
    }

    /// 在工作区文件的差异标签中打开提交视图列出的下一个（`forward`）或上一个文件，返回是否已打开。
    ///
    /// 新文件在当前标签中打开，连续查看时只占用一个标签。
    pub(super) fn step_changed_file(&mut self, forward: bool) -> bool {
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return false;
        };
        let Some((path, next, total)) = self.next_changed_file(forward) else {
            return false;
        };
        let name = winc_editor::display_name(&path).into_owned();
        self.commit.preview = Some(id);
        self.git.stepped = Some((id, path.clone()));
        self.select_commit_file(path);
        let (index, name) = (next + 1, name.as_str());
        self.flash(if forward {
            t!("git-next-file", name = name, index = index, total = total)
        } else {
            t!(
                "git-previous-file",
                name = name,
                index = index,
                total = total
            )
        });
        true
    }

    /// 状态刷新后重新比较已过时的 Git 差异标签，并原地更新内容。
    ///
    /// `HEAD` 变化（提交、切换分支）时重新读取左侧。右侧为磁盘内容（文件未在编辑器中打开）时，
    /// 文件可能已变化，同样重新比较；其余标签的右侧由 [`App::sync_diff_source`] 随编辑器中的
    /// 文档更新，无需再运行 `git`。
    fn refresh_git_diffs(&mut self) {
        let editor = &self.editor;
        self.git
            .diff_tabs
            .retain(|id, _| editor.document(*id).is_some_and(|d| d.diff.is_some()));
        let oid = self.git.status.as_ref().and_then(|s| s.oid.as_deref());
        let follows_editor = |id: DocumentId| {
            editor
                .document(id)
                .and_then(|d| d.diff.as_deref())
                .and_then(|v| v.new.path.as_deref())
                .is_some_and(|path| {
                    editor
                        .documents
                        .iter()
                        .any(|d| d.path.as_deref() == Some(path))
                })
        };
        let stale: Vec<(DocumentId, PathBuf)> = self
            .git
            .diff_tabs
            .iter()
            .filter(|(id, tab)| tab.oid.as_deref() != oid || !follows_editor(**id))
            .map(|(id, tab)| (*id, tab.path.clone()))
            .collect();
        for (id, path) in stale {
            self.fetch_changes(&path, Target::Refresh(id));
        }
    }

    /// 在标签 `id`（文件 `path`）关闭后释放其逐行追溯、改动标记、差异标签记录和日志的 `git log`。
    ///
    /// 没有其他标签打开同一文件时，同时释放该文件在 `HEAD` 中的内容（基准）。
    pub(super) fn forget_git_document(&mut self, id: DocumentId, path: Option<&Path>) {
        self.blame.forget(id);
        self.log.forget(id);
        self.changes.forget_document(id);
        self.git.diff_tabs.remove(&id);
        if let Some(path) = path
            && !self
                .editor
                .documents
                .iter()
                .any(|d| d.path.as_deref() == Some(path))
        {
            self.changes.forget_base(path);
        }
    }

    /// 在后台读取 `path` 的 Git 改动，完成后按 `target` 交给界面线程。
    pub(super) fn fetch_changes(&mut self, path: &Path, target: Target) {
        let Some(root) = self.git.root.clone() else {
            self.flash(t!("git-not-a-repository"));
            return;
        };
        // 路径可能经过符号链接（macOS 的 `/var` 指向 `/private/var`），不在仓库下时规范化后再比较。
        let path = if path.starts_with(&root) {
            path.to_path_buf()
        } else {
            match winc_config::files::canonicalize(path) {
                Ok(real) if real.starts_with(&root) => real,
                _ => {
                    self.flash(t!("git-file-outside-repository"));
                    return;
                }
            }
        };

        // 文件已在编辑器中打开时使用其快照；大文件不比较，整体读入字符串会超出内存预算。
        let canonical = winc_editor::canonical_path(&path);
        let doc = self
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(canonical.as_path()) && !d.is_pending());
        if doc.is_some_and(|d| d.is_large()) {
            if !matches!(target, Target::Refresh(_)) {
                let name = winc_editor::display_name(&path).into_owned();
                self.flash(t!("git-diff-too-large", name = name));
            }
            return;
        }
        let snapshot = doc.and_then(|d| {
            Some(Snapshot {
                text: d.text().rope()?.clone(),
                revision: d.revision,
                encoding: d.encoding,
                dirty: d.is_dirty(),
                language: d.highlighter.language(),
            })
        });

        let oid = self.git.status.as_ref().and_then(|s| s.oid.clone());
        let sender = self.git.prepared.0.clone();
        self.spawn_git(UserEvent::GitContent, move |tool| {
            let view = match &tool {
                Ok(git) => prepare(git, &root, &path, snapshot, oid.as_deref()),
                Err(e) => Err(unusable(e)),
            };
            let _ = sender.send(Prepared {
                root,
                path,
                oid,
                target,
                view,
            });
        });
    }

    /// 接收后台准备好的差异，按 [`Target`] 新建标签、替换预览标签的内容或原地更新已打开的标签。
    ///
    /// 仓库已更换、预览请求不是最新的一次或原地更新失败时丢弃结果。
    pub(super) fn poll_git_content(&mut self) {
        while let Ok(prepared) = self.git.prepared.1.try_recv() {
            // 等待结果期间更换了目录：文件属于旧仓库，不在新目录或开始页中打开标签。
            if self.git.root.as_deref() != Some(prepared.root.as_path()) {
                continue;
            }
            // 预览只采用最新的一次请求。
            if let Target::Preview(seq) = prepared.target
                && seq != self.git.preview_seq
            {
                continue;
            }
            let tab = DiffTab {
                path: prepared.path,
                oid: prepared.oid,
            };
            let view = match prepared.view {
                Ok(view) => view,
                // 原地更新失败（文件变为二进制、无法读取）时不提示，标签保留上一次的内容。
                Err(_) if matches!(prepared.target, Target::Refresh(_)) => continue,
                Err(e) => {
                    self.diff.land = None;
                    self.flash(e);
                    continue;
                }
            };
            let name = winc_editor::display_name(&tab.path).into_owned();
            let title = diff_title(&tab.path);
            let index = match prepared.target {
                Target::Refresh(id) => {
                    // 替换内容，保留滚动位置。
                    if let Some(index) = self.editor.replace_diff(id, &title, view) {
                        self.git.diff_tabs.insert(id, tab);
                        if self.editor.active == Some(index) {
                            self.clamp_scroll();
                        }
                    }
                    self.request_redraw();
                    continue;
                }
                _ if view.is_identical() => {
                    self.diff.land = None;
                    // 逐行比较不计换行符的写法，没有改动时两侧的内容仍可能只差换行符。这类差别
                    // 是否算作修改由 `git` 按 `core.autocrlf` 和文件属性判定，提示只说明差别所在。
                    let text = if view.old.text == view.new.text {
                        t!("git-diff-identical", name = name.as_str())
                    } else {
                        t!("git-diff-line-endings-only", name = name.as_str())
                    };
                    self.flash(text);
                    continue;
                }
                Target::Preview(_) => {
                    let index = self.editor.show_diff_in(self.commit.preview, &title, view);
                    self.commit.preview = Some(self.editor.documents[index].id);
                    index
                }
                Target::Open => self.editor.show_diff(&title, view),
            };
            let id = self.editor.documents[index].id;
            self.git.diff_tabs.insert(id, tab);
            // 打开时定位到第一处改动。
            let doc = &mut self.editor.documents[index];
            if let Some(row) = doc.diff.as_deref().and_then(|v| {
                v.hunk_rows[winc_editor::diff_view::RIGHT]
                    .first()
                    .map(|r| r.start)
            }) {
                doc.scroll_line = row.saturating_sub(3) as f64;
            }
            self.relayout();
            self.clamp_scroll();
            self.land_diff_file();
            self.request_redraw();
        }
    }
}

// 仅在 Unix 上编译测试：相关测试以 sh 脚本作为格式化工具，Git 的测试也只在该平台上运行。
#[cfg(all(test, unix))]
mod tests;
