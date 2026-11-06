//! Git 模块的界面线程部分：什么时候刷新仓库状态，结果交回来以后更新文件树、状态栏和打开着的
//! 差异；取文件在 `HEAD` 里的版本并打开差异。
//!
//! 模块关着、打开的目录不在仓库里时什么都不做：不找 git，不起线程。`git status` 在后台线程上运行，
//! 同一时刻最多一次，运行期间又有变化时完成后补一次。打开差异时，读 `HEAD` 和磁盘、判断二进制和
//! 大小、解码、逐行比较都在后台做，界面线程只拿文档内容的快照。

pub(crate) mod blame;
pub(crate) mod branch;
pub(super) mod changes;
pub(super) mod commit;
pub(crate) mod form;
pub(crate) mod log;
pub(crate) mod merge;
pub(crate) mod worktree;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use ropey::Rope;
use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::encoding::{decode_known, decode_unknown};
use winc_editor::format::Cancel;
use winc_editor::git::{self, Branch, Change, Git, Ignored, Stash, Status, Unavailable, Worktree};
use winc_editor::shell::Shell;
use winc_editor::{DocumentId, Encoding, LARGE_FILE, looks_binary};
use winc_i18n::t;

use super::diff::worktree_title;
use super::{App, Focus, UserEvent};
use crate::layout::SidebarView;

/// 找到的 git，界面线程和各个后台任务共用同一个 [`Git`]。
///
/// 哪个任务先找到就放进来，之后的任务直接取用；找不到时什么都不放，下一个任务再找（用户可能
/// 刚装好）。每个任务各自 `Git::find` 得到的 [`Git`] 会各自记着仓库信任状态，共用才能让用户对
/// 一个仓库配置的信任决定在各任务之间一致。
#[derive(Debug, Clone, Default)]
pub(crate) struct SharedGit(Arc<Mutex<Option<Git>>>);

impl SharedGit {
    /// 已经找到的 git；还没找到、或者找不到时为 `None`。
    pub(crate) fn get(&self) -> Option<Git> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 取用 git：还没找到时就地找，找到了放进来；两个任务同时找到时保留先放进来的结果。
    ///
    /// 第一次查找要等取到用户 shell 的环境，只在后台线程上调用。
    fn resolve(&self, shell: &Shell) -> Result<Git, Unavailable> {
        if let Some(git) = self.get() {
            return Ok(git);
        }
        let found = Git::find(shell)?;
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(slot.get_or_insert(found).clone())
    }

    /// 记下已经找到的 `git`，不再查找。
    pub(super) fn keep(&self, git: Git) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(git);
    }

    /// 清除找到的 git（关闭目录、关闭 Git 模块时），下次用到时重新查找。
    pub(super) fn forget(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// 后台任务找不到 git 时给用户看的原因。
pub(super) fn unusable(e: &Unavailable) -> String {
    t!("git-unusable", error = e.to_string())
}

/// 一次后台刷新的结果。
struct Outcome {
    /// 刷新的是哪个仓库；交回来时仓库已经换了就丢弃。
    root: PathBuf,
    /// 这次读到的状态；失败时是原因。
    status: Result<Status, String>,
    /// 分支、储藏、工作树列表；状态读不出来，或这次只刷新状态（[`App::git_refresh_status`]）
    /// 时不取。
    refs: Option<Refs>,
}

/// 和状态一起刷新的几张列表：只在 `.git` 里变。
struct Refs {
    /// 本地和远程分支，本地的在前。
    branches: Vec<Branch>,
    /// 储藏，最新的在前。
    stashes: Vec<Stash>,
    /// 取不到（git 太旧、超时）时为 `None`，保留此前的列表：否则被占用的分支、窗口标题、新建
    /// 工作树存放在哪儿都会跟着出错。
    worktrees: Option<Vec<Worktree>>,
}

/// 差异打开在哪里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    /// 新开（或换掉同名的）差异标签，切过去。
    Open,
    /// 提交视图的预览标签，参数是请求的序号：只认最新的一次，快速换选择时先发、后到的结果
    /// 丢弃。
    Preview(u64),
    /// 原地更新已经打开着的差异标签，不切换标签、不动滚动位置。
    Refresh(DocumentId),
}

/// 后台准备好的差异。
struct Prepared {
    /// 在哪个仓库里比较的：之后换了目录就不要了。
    root: PathBuf,
    /// 比较的文件。
    path: PathBuf,
    /// 比较时 `HEAD` 的提交号，左边内容取自它。
    oid: Option<String>,
    target: Target,
    /// 比较的结果；不能比较时是原因（二进制、太大、读不出来）。
    view: Result<DiffView, String>,
}

/// 发起比较时，编辑器里该文件的快照。
struct Snapshot {
    /// 发起比较时的文字。
    text: Rope,
    /// 发起比较时的文档版本号，记进差异视图（`DiffView::source`）。
    revision: u64,
    /// 文档的编码，左边按它解码。
    encoding: Encoding,
    /// 发起比较时文档有未保存的修改。
    dirty: bool,
    /// 文档的语言，左右两边都按它着色。
    language: &'static winc_syntax::Language,
}

/// 一个打开着的 Git 差异标签。
struct DiffTab {
    /// 比较的文件。
    path: PathBuf,
    /// 左边内容对应的 `HEAD` 提交号；`HEAD` 动了才重新取左边。
    oid: Option<String>,
}

/// 项目菜单最多读取分支和改动数的仓库数（个，经验值）：每个要运行一次 `git status`。
const SUMMARY_REPOSITORIES: usize = 32;

/// Git 模块的状态。
pub(crate) struct GitState {
    /// 当前仓库的根目录；模块关着、目录里没有仓库时为 `None`。
    pub root: Option<PathBuf>,
    /// 打开的目录里可用的仓库，见 [`App::git_discover`]；项目菜单在有多个时列出。
    pub repositories: Vec<PathBuf>,
    /// 上次跟随的标签（[`App::git_follow_active`]）：切换标签时才查找文件所在的仓库。
    followed: Option<DocumentId>,
    /// 各仓库最近一次读到的状态：项目菜单据此显示分支和改动数，切换过去时先显示它，不必等待。
    summaries: Arc<Mutex<HashMap<PathBuf, Status>>>,
    /// 正在运行的读取；重新读取时取消。
    summaries_running: Option<Cancel>,
    /// 找到的 git，第一次用到时在后台找，所有后台任务共用（[`SharedGit`]）。
    pub(super) tool: SharedGit,
    /// 最近一次成功的 `git status`。
    pub status: Option<Status>,
    /// 本地和远程分支（本地的在前），和状态一起刷新。
    pub branches: Vec<Branch>,
    /// 储藏，最新的在前，和状态一起刷新。
    pub stashes: Vec<Stash>,
    /// 仓库的工作树，主工作树在最前面，和状态一起刷新；只有一个工作树时为空。
    pub worktrees: Vec<Worktree>,
    /// 最近一次刷新失败的原因（找不到 git、git 报错）；成功以后清掉。
    pub problem: Option<String>,
    /// 状态每更新一次加 1；提交视图据此判断绘制时记下的行是否还对得上。
    pub generation: u64,
    /// 文件树要用的状态（[`Status::decorations`]）。
    pub decorations: HashMap<PathBuf, Change>,
    /// 被忽略且未跟踪的文件和目录（[`Status::ignored`]），文件树以淡色绘制。
    ignored_paths: HashSet<PathBuf>,
    /// 仓库的忽略规则，判断一次变化要不要刷新。
    ignored: Option<Ignored>,
    /// 正在运行的刷新；`None` 表示没有在运行。
    running: Option<Cancel>,
    /// 运行期间又要求刷新：完成以后再来一次。
    again: bool,
    /// 下一次刷新要连分支、储藏一起取（[`App::git_refresh`] 设上，取的时候清掉）。
    refs_wanted: bool,
    /// 打开着的 Git 差异标签；状态刷新以后按需原地更新，标签关闭时去掉。
    diff_tabs: HashMap<DocumentId, DiffTab>,
    /// 最近一次切换文件时的（差异标签，切换到的文件）。
    stepped: Option<(DocumentId, PathBuf)>,
    /// 最近一次预览请求的序号。
    pub(super) preview_seq: u64,
    /// 本次运行中已经问过是否信任的仓库配置指纹（[`git::Risks::fingerprint`]）：选了不信任的
    /// 不再问，下次启动再问。
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
            summaries_running: None,
            tool: SharedGit::default(),
            status: None,
            branches: Vec::new(),
            stashes: Vec::new(),
            worktrees: Vec::new(),
            problem: None,
            generation: 0,
            decorations: HashMap::new(),
            ignored_paths: HashSet::new(),
            ignored: None,
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
    /// 文件树里 `path` 要显示的状态。
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

    /// 停掉正在运行的刷新，清空状态。
    ///
    /// 找不到 git 的结论也清掉，下次重新找。
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
        self.stashes.clear();
        self.worktrees.clear();
        self.problem = None;
        self.ignored = None;
        self.generation += 1;
        self.decorations.clear();
        self.ignored_paths.clear();
    }
}

/// 在后台准备一个文件的差异：取 `HEAD` 里的版本，和编辑器里的快照（没打开时读磁盘）比较。
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

    // 右边：编辑器里开着就用编辑器里的内容（含未保存的修改），否则读磁盘。左边按右边的编码
    // 解码；右边不存在（文件删掉了）时左边自己识别编码。
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
    // 文件删掉了：右侧没有可以编辑的东西。路径按编辑器的规矩规范化，好和打开着的标签对上。
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

/// 差异标签的标题。
pub(super) fn diff_title(path: &Path) -> String {
    t!("git-diff-title", name = winc_editor::display_name(path))
}

impl App {
    /// Git 模块是否开着。
    pub(crate) fn git_enabled(&self) -> bool {
        self.config.git_enabled.unwrap_or(false)
    }

    /// 打开的目录变了、或者模块开关变了：重新确定仓库，并刷新一次。
    ///
    /// 目录本身不是仓库、第一层子目录里有仓库时用第一个，其余的在项目菜单里切换。
    pub(super) fn git_folder_changed(&mut self) {
        self.git.repositories.clear();
        self.git.followed = None;
        self.git
            .summaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.git_discover();
        let root = self.git.repositories.first().cloned();
        self.git_set_root(root);
    }

    /// 重新查找打开的目录里的仓库；已经加入的（打开过其中文件的）只要还在就保留。
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

    /// 在后台读取各仓库的分支和改动数，项目菜单下次打开时显示；只有一个仓库时不读。
    ///
    /// 系统菜单弹出后内容不能再变，因此提前读好。最多读 [`SUMMARY_REPOSITORIES`] 个。
    fn git_read_summaries(&mut self) {
        if let Some(cancel) = self.git.summaries_running.take() {
            cancel.cancel();
        }
        if self.git.repositories.len() < 2 {
            return;
        }
        let cancel = Cancel::default();
        self.git.summaries_running = Some(cancel.clone());
        let repositories = self.git.repositories.clone();
        let summaries = Arc::clone(&self.git.summaries);
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            let Ok(git) = tool else {
                return;
            };
            for root in repositories.into_iter().take(SUMMARY_REPOSITORIES) {
                if cancel.is_cancelled() {
                    return;
                }
                if let Ok(status) = git.status(&root, &cancel) {
                    summaries
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(root, status);
                }
            }
        });
    }

    /// 返回第 `index` 个仓库在项目菜单里的小字：分支和改动数；还没读到时为 `None`。当前仓库用
    /// 最新的状态。
    pub(in crate::app) fn git_repository_detail(&self, index: usize) -> Option<String> {
        let repo = self.git.repositories.get(index)?;
        let (head, changes) = match &self.git.status {
            Some(status) if self.git.root.as_ref() == Some(repo) => {
                (status.head.clone(), status.entries.len())
            }
            _ => {
                let summaries = self
                    .git
                    .summaries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let status = summaries.get(repo)?;
                (status.head.clone(), status.entries.len())
            }
        };
        let head = match head? {
            git::Head::Branch(name) => name,
            git::Head::Detached(oid) => t!("git-worktree-detached", oid = oid.as_str()),
        };
        Some(if changes == 0 {
            head
        } else {
            t!("git-repository-detail", head = head, count = changes)
        })
    }

    /// 返回第 `index` 个仓库在菜单里显示的名字：相对打开的目录的路径，目录本身用目录名。
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

    /// 切换到项目菜单里的第 `index` 个仓库。
    pub(in crate::app) fn git_switch_repository(&mut self, index: usize) {
        if let Some(root) = self.git.repositories.get(index).cloned() {
            self.git_set_root(Some(root));
        }
    }

    /// 当前标签换了文件时，把文件所在的仓库加入列表，并切换过去。
    ///
    /// 正在看提交视图时不切换，否则列表会突然换成另一个仓库的改动，有可能提交错仓库；正在提交、
    /// 拉取推送，或提交信息已写了字时也不切换，切换会丢掉这些内容。这些情况下只加入列表。
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
        let stay = self.sidebar_view == SidebarView::Commit
            || self.commit.busy
            || self.branch.running.is_some()
            || !self.commit.message.text().is_empty();
        if !stay {
            self.git_set_root(Some(root));
        }
    }

    /// 把当前仓库换成 `root`；没有变化时什么也不做。
    fn git_set_root(&mut self, root: Option<PathBuf>) {
        if root == self.git.root {
            return;
        }
        // 留下要离开的仓库的状态，切回来时马上能显示。
        if let (Some(old), Some(status)) = (self.git.root.clone(), self.git.status.take()) {
            self.git
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(old, status);
        }
        // 只是换一个仓库时 git 还是那个 git，不必重新查找，也不必重新检查已经检查过的仓库配置。
        let tool = root.as_ref().and(self.git.tool.get());
        self.git.clear();
        if let Some(tool) = tool {
            self.git.tool.keep(tool);
        }
        self.changes.forget();
        self.blame.clear();
        // 旧仓库还在做的分支操作、提交是那边的：这里不再显示「正在拉取…」、不拦着新仓库的操作，
        // 写了一半的提交信息、「修改上一次提交」也不带过来。做完时只说一声（见 `poll_branch_ops`、
        // `poll_commit`）。
        self.branch.running = None;
        self.reset_commit_view();
        self.git.ignored = root.as_deref().map(Ignored::new);
        self.git.root = root;
        // 先显示上次读到的状态，后台读完以后再更新，提交视图不会闪成「正在读取」。
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
        // 不在仓库里了：提交视图没有了，回到文件树；焦点在提交视图里的要拿出来，否则键入的字
        // 会进到看不见的输入框。
        if self.git.root.is_none() {
            if self.sidebar_view == SidebarView::Commit {
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

    /// 目录里有变化：路径都被忽略、`.git` 也没变时不刷新。
    ///
    /// `.gitignore` 变了先清除已读的规则；需要重新扫描时 `paths` 不完整，无从得知 `.gitignore`
    /// 是否变了，也一并清除。
    pub(super) fn git_paths_changed(&mut self, paths: &[PathBuf], rescan: bool, git: bool) {
        let Some(ignored) = self.git.ignored.as_mut() else {
            return;
        };
        if rescan
            || paths
                .iter()
                .any(|p| p.file_name().is_some_and(|n| n == ".gitignore"))
        {
            ignored.forget();
        }
        if rescan || git {
            self.git_refresh();
        } else if paths.iter().any(|p| !ignored.is_ignored(p)) {
            self.git_refresh_status();
        }
    }

    /// 刷新仓库状态，连同分支、储藏列表。
    ///
    /// 不在仓库里时什么都不做；正在刷新时，完成后再补一次。
    pub(super) fn git_refresh(&mut self) {
        self.git.refs_wanted = true;
        self.git_refresh_status();
    }

    /// 只刷新状态，分支、储藏列表保持不变：它们只在 `.git` 里变（目录监视报 `git`，或是本程序
    /// 自己做的 Git 操作，这些都走 [`App::git_refresh`]）。
    ///
    /// 工作区里每保存一次文件只运行 `status` 一个进程，不像 [`App::git_refresh`] 那样还要运行
    /// `for-each-ref` 和 `stash list`。
    pub(super) fn git_refresh_status(&mut self) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        if self.git.running.is_some() {
            self.git.again = true;
            return;
        }
        // 还没取到过状态时（刚打开、上次出错）分支也没有，一起取。
        let refs = std::mem::take(&mut self.git.refs_wanted) || self.git.status.is_none();
        let cancel = Cancel::default();
        self.git.running = Some(cancel.clone());
        let sender = self.git.sender.clone();
        // 上次没找到 git 时这次在后台重新找（用户可能刚装好）。
        self.spawn_git(UserEvent::GitStatus, move |tool| {
            let status = match &tool {
                Ok(git) => git.status(&root, &cancel),
                Err(e) => Err(e.to_string()),
            };
            let refs = match (&tool, &status) {
                (Ok(git), Ok(_)) if refs => Some(Refs {
                    branches: git.branches(&root, &cancel).unwrap_or_default(),
                    stashes: git.stashes(&root, &cancel).unwrap_or_default(),
                    worktrees: git.worktrees(&root, &cancel).ok(),
                }),
                _ => None,
            };
            let _ = sender.send(Outcome { root, status, refs });
        });
    }

    /// 在后台用 git 做一件事：`work` 拿到找到的 git（还没找到时在后台找，找不到时是原因），做完
    /// 以 `done` 叫醒界面线程收结果（[`App::spawn_blocking_then`]）。
    ///
    /// `work` 运行 `git` 并等它结束（拉取、推送等网络，提交等钩子），因此单独开线程，不占共用的
    /// 线程池；各 Git 功能的后台任务都从这里起。
    pub(super) fn spawn_git(
        &mut self,
        done: UserEvent,
        work: impl FnOnce(Result<Git, Unavailable>) + Send + 'static,
    ) {
        let (tool, shell) = (self.git.tool.clone(), self.format.formatter.shell().clone());
        self.spawn_blocking_then(done, move || work(tool.resolve(&shell)));
    }

    /// 收下后台刷新的结果；成功时顺带更新打开着的 Git 差异。
    pub(super) fn poll_git(&mut self) {
        let mut changed = false;
        let mut refreshed = false;
        while let Ok(outcome) = self.git.receiver.try_recv() {
            if self.git.root.as_deref() != Some(outcome.root.as_path()) {
                continue;
            }
            self.git.running = None;
            match outcome.status {
                Ok(status) => {
                    // `HEAD` 或分支变了（提交、切换、新建、删除）：开着的日志重新取。
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
                    // 有变化的文件超过上限：刚超过时提示一次，之后每次刷新不再重复。
                    let was_truncated = self.git.status.as_ref().is_some_and(|s| s.truncated);
                    if status.truncated && !was_truncated {
                        let count = winc_editor::git::STATUS_LIMIT;
                        self.notify(t!("git-status-truncated", count = count));
                    }
                    self.git.status = Some(status);
                    if let Some(refs) = outcome.refs {
                        self.git.branches = refs.branches;
                        self.git.stashes = refs.stashes;
                        if let Some(worktrees) = refs.worktrees
                            && self.git.worktrees != worktrees
                        {
                            self.git.worktrees = worktrees;
                            self.update_window_title();
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
            self.request_redraw();
        }
    }

    /// 仓库的配置会让 git 运行命令（[`Git::untrusted`]）：信任过同样的配置就照常使用，否则问
    /// 用户一次。
    ///
    /// 回答之前读状态时不运行这些命令，会改仓库的操作被拒绝。
    fn check_git_trust(&mut self) {
        let (Some(root), Some(git)) = (self.git.root.clone(), self.git.tool.get()) else {
            return;
        };
        // 状态读出来了，后台已经检查过配置，`untrusted` 只取检查结果，不在界面线程上运行 git。
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

        if self.window.is_some() {
            use crate::app::AlertButton;

            let name = winc_editor::display_name(&root).into_owned();
            let message = t!("git-trust-confirm", name = name);
            let mut detail = t!("git-trust-confirm-detail").to_owned();
            detail.push('\n');
            // 最多列出 8 条（行，经验值）：再多详情就超出窗口高度。
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
            // 窗口上已有其他提示框时下次刷新再问。
            if asked {
                self.git.asked_trust.push(fingerprint);
            }
        }
    }

    /// 处理提示框上对是否信任仓库 `root` 配置的回答。
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

    /// 合并、拣选、还原停下来时，把 git 准备好的提交信息（`MERGE_MSG`）填进提交信息框，每次
    /// 只填一回；用户已经写了东西时不动。
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

    /// 在提交视图的预览标签里打开 `path` 的改动：一个接一个地看几个文件，只占一个标签。
    pub(super) fn git_preview_changes(&mut self, path: &Path) {
        self.git.preview_seq += 1;
        self.fetch_changes(path, Target::Preview(self.git.preview_seq));
    }

    /// 标签 `id` 是否是工作区文件的 Git 差异。
    pub(super) fn steps_changed_files(&self, id: DocumentId) -> bool {
        self.git.diff_tabs.contains_key(&id)
    }

    /// 工作区文件的差异标签里，提交视图列出的下一个（`forward`）或上一个文件：（路径，从 0 起的
    /// 序号，文件总数）；当前标签不是这种差异，或者没有更多文件时为 `None`。
    ///
    /// 按最近一次切换到的文件算：那个文件打不开时（二进制、没有差异）标签还是原来的内容，这样再切换
    /// 一次能跳过它。
    pub(super) fn next_changed_file(&self, forward: bool) -> Option<(PathBuf, usize, usize)> {
        let id = self.editor.active()?.id;
        let tab = self.git.diff_tabs.get(&id)?;
        let current = match &self.git.stepped {
            Some((doc, path)) if *doc == id => path,
            _ => &tab.path,
        };
        // 列表里收起的目录中的文件不在 `commit_files` 里，这时按状态里的顺序。
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
    /// 新文件在当前标签里打开，连续查看时只占一个标签。
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

    /// 状态刷新以后，重新比较打开着的 Git 差异标签里过时了的，原地换内容：`HEAD` 动了（提交、
    /// 切换分支）的左边要重新取；右边是磁盘上的内容（文件没在编辑器里打开）的，文件可能变了。
    ///
    /// 其余的右边跟着编辑器里的文档（[`App::sync_diff_source`]），不必再运行一次 git。
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

    /// 标签 `id`（文件 `path`）关闭了：放下它的逐行追溯、改动标记、差异标签的记录和日志开着的
    /// `git log`；别的标签没开着同一个文件时，连它在 `HEAD` 里的内容（基准）也放下。
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

    /// 在后台取 `path` 的 Git 改动，完成后按 `target` 交给界面线程。
    pub(super) fn fetch_changes(&mut self, path: &Path, target: Target) {
        let Some(root) = self.git.root.clone() else {
            self.flash(t!("git-not-a-repository"));
            return;
        };
        // 路径可能经过符号链接（macOS 的 `/var` 就是 `/private/var`），不在仓库下时规范化再比。
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

        // 编辑器里开着就拿它的快照；大文件不比较（整个读成字符串会违背内存预算）。
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

    /// 收下后台准备好的差异：按 [`Target`] 新开标签、换掉预览标签的内容，或原地更新打开着的
    /// 标签。
    ///
    /// 仓库已经换了、预览请求不是最新的一次、原地更新失败时丢弃结果。
    pub(super) fn poll_git_content(&mut self) {
        while let Ok(prepared) = self.git.prepared.1.try_recv() {
            // 等结果的时候换了目录：是旧仓库的文件，不在这里（新目录、开始页）开标签。
            if self.git.root.as_deref() != Some(prepared.root.as_path()) {
                continue;
            }
            // 预览只认最新的一次请求。
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
                // 原地更新失败（文件变成了二进制、读不出来）时不打扰，标签留着上一次的内容。
                Err(_) if matches!(prepared.target, Target::Refresh(_)) => continue,
                Err(e) => {
                    self.diff_land = None;
                    self.flash(e);
                    continue;
                }
            };
            let name = winc_editor::display_name(&tab.path).into_owned();
            let title = diff_title(&tab.path);
            let index = match prepared.target {
                Target::Refresh(id) => {
                    // 换掉内容，保留滚动位置。
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
                    self.diff_land = None;
                    self.flash(t!("git-diff-identical", name = name.as_str()));
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
            // 打开时直接停在第一处改动。
            let doc = &mut self.editor.documents[index];
            if let Some(row) = doc.diff.as_deref().and_then(|v| {
                v.hunk_rows[winc_editor::diff_view::RIGHT]
                    .first()
                    .map(|r| r.start)
            }) {
                doc.scroll_line = row.saturating_sub(3) as f32;
            }
            self.relayout();
            self.clamp_scroll();
            self.land_diff_file();
            self.request_redraw();
        }
    }
}

// 只有 Unix 上的测试用到：那些测试用 sh 脚本当作格式化工具，Git 的测试也只在这个平台上运行。
#[cfg(all(test, unix))]
mod tests;
