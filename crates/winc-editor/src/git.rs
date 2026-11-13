//! Git 模块的底层：找到系统里的 `git`，运行 `git status` 并解析结果，取文件在暂存区或提交里的内容。
//!
//! 只调用 `git` 命令，不链接 Git 的库，用户的 hooks、全局配置、凭据助手都仍然生效；界面线程不调用这
//! 里的函数。仓库自己的配置（`.git/config`、hooks）可能来自不可信的压缩包或共享盘，可以指定 `git`
//! 要运行的命令：用户信任该仓库（[`Git::trust`]）之前不运行 fsmonitor 和过滤器，会改动仓库的操作一
//! 律拒绝，依据见 [`Risks`]。

pub mod log;
pub mod merge;

mod commit;
mod output;
mod parse;
mod rebase;
mod remote;
mod sync;

pub use output::output_text;
pub use rebase::{RebaseAction, RebaseCommit, RebasePlan, RebaseStep, rebase_todo};
pub use remote::{Remote, TagTarget, repository_dir_name};
pub use sync::{Carried, Synced};
mod history;
mod trust;
mod worktree;

use parse::*;
pub use trust::*;
pub use worktree::*;

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use winc_core::hash::Fnv;

use crate::tools::shell::Shell;
use crate::tools::shell::process::{self, Cancel, RunError};

/// `git status` 最长的运行时间（30 秒）。
///
/// 几十万个文件的仓库第一次运行需要数秒；超过该时间视为 `git` 失去响应。
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);
/// 日志中日期的格式参数。
///
/// 本机时区的换算由 `git` 完成（`format-local`），不需要读取时区数据库。
const LOG_DATE: &str = "--date=format-local:%Y-%m-%d %H:%M";
/// 日志中单个提交的输出格式，由 [`parse_log`] 解析。
const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%ad%x1f%D%x1f%s%x1e";
/// 取文件内容最长的运行时间（10 秒）。
///
/// 读取大仓库里的对象通常在毫秒级；超过该时间视为 `git` 失去响应。
const SHOW_TIMEOUT: Duration = Duration::from_secs(10);
/// 提交最长的运行时间（10 分钟）：提交前的 hooks 可能要运行测试、格式检查。
const COMMIT_TIMEOUT: Duration = Duration::from_secs(600);
/// 获取、拉取、推送、合并、变基这类操作最长的运行时间（10 分钟）。
///
/// 网络慢、hooks 需要运行测试或格式检查时耗时较长。
const OPERATION_TIMEOUT: Duration = Duration::from_secs(600);
/// 决定 `git` 作用于哪个仓库、暂存区和对象库的环境变量，运行 `git` 时不从环境继承。
///
/// 列表取自 `git rev-parse --local-env-vars`，其中的 `GIT_CONFIG_PARAMETERS`、`GIT_CONFIG_COUNT`
/// 除外：二者只携带配置项，`git` 进入子模块时同样保留。本程序由 `git` 启动时（作为编辑器或在钩子
/// 中运行）继承这些变量；例如 `git commit` 传给编辑器的 `GIT_INDEX_FILE` 指向一个临时暂存区，
/// 提交结束后该文件即被删除。
const REPOSITORY_ENV: [&str; 13] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// `HEAD` 指向的提交（提交视图里「撤销上一次提交」使用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastCommit {
    /// 完整的提交号。
    pub oid: String,
    /// 提交信息的第一行。
    pub subject: String,
    /// 完整的提交信息。
    pub message: String,
    /// 提交时间（Unix 秒）。
    pub time: i64,
    /// 提交时间在本机时区的写法「2026-10-26 14:32」。
    pub local: String,
}

/// 只提交一部分改动的文件：提交成 `content`，工作区里的文件不变。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partial {
    /// 文件的路径。
    pub path: PathBuf,
    /// 要提交成的内容（工作区里的形式，换行符等与工作区文件相同）。
    pub content: Vec<u8>,
}

/// 一个分支。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// 分支名；远程分支带着远程名（`origin/main`）。
    pub name: String,
    /// 是否是远程分支。
    pub remote: bool,
    /// 是否是当前分支。
    pub current: bool,
    /// 跟踪的上游分支（`origin/main`）；没有，或者上游引用已不存在时为 `None`。
    pub upstream: Option<String>,
    /// 配了上游，但上游引用已不存在（远程分支被删除后获取时清掉了）。
    ///
    /// 这样的分支推送过，不能和从未推送过的分支一样看待。
    pub gone: bool,
    /// 该分支指向的完整提交号。
    ///
    /// 提交、推送、获取使分支移动时随之变化，据此判断开着的日志是否需要重新读取。
    pub oid: String,
    /// 分支指向的提交的提交时间（Unix 秒）。
    pub time: i64,
    /// 提交时间在本机时区的写法「2026-10-26 14:32」。
    pub local: String,
}
/// 日志里的一个提交。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// 完整的提交号。
    pub oid: String,
    /// 父提交的提交号，第一个是主线；根提交没有，合并提交有两个以上。
    pub parents: Vec<String>,
    /// 作者的名字。
    pub author: String,
    /// 作者的邮箱。
    pub email: String,
    /// 作者写下这次提交的时间（Unix 秒）。
    pub time: i64,
    /// 同一时间换成本机时区的写法：「2026-10-26 14:32」。
    pub local: String,
    /// 指向该提交的引用：分支、远程分支、标签、`HEAD`。
    pub refs: Vec<Ref>,
    /// 说明的第一行。
    pub subject: String,
}

/// 指着一个提交的引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    /// `HEAD` 指着的当前分支。
    Head(String),
    /// `HEAD` 直接指向该提交（分离的 `HEAD`）。
    Detached,
    /// 本地分支。
    Branch(String),
    /// 远程分支（带远程名，`origin/main`）。
    Remote(String),
    /// 标签。
    Tag(String),
}

/// 一次提交改动的一个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    /// 文件的绝对路径（改之后的路径）。
    pub path: PathBuf,
    /// 改名前的绝对路径（只有改名时才有）。
    pub from: Option<PathBuf>,
    /// 怎么变的：新增、修改、删除、改名。
    pub change: Change,
    /// 加了几行、删了几行；二进制文件为 `None`。
    pub lines: Option<(usize, usize)>,
}

/// 一条储藏（`git stash`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// 名字：`stash@{0}`，最新的是 0。
    ///
    /// 新建或删除储藏后，其余储藏的编号随之改变；要指定某一条储藏时用 `oid`。
    pub name: String,
    /// 储藏对应的提交号（完整的对象号），不随其他储藏的增删改变。
    pub oid: String,
    /// 储藏提交的各个父提交：第一个是储藏时的 `HEAD`，第二个是暂存区，连同未跟踪的文件一起储藏时
    /// 第三个是那些文件。
    pub parents: Vec<String>,
    /// 储藏的时间（Unix 秒）。
    pub time: i64,
    /// 储藏时的本地时间，「2026-11-07 14:32」。
    pub local: String,
    /// `git` 记下的说明：「On main: 说明」或「WIP on main: 提交号 说明」。
    pub message: String,
}

impl Stash {
    /// 储藏时所在的分支：说明开头「On 分支:」「WIP on 分支:」里的分支；分离 `HEAD` 时 `git` 写
    /// 「(no branch)」，原样返回。说明不是这两种写法时为 `None`。
    pub fn branch(&self) -> Option<&str> {
        let rest =
            (self.message.strip_prefix("WIP on ")).or_else(|| self.message.strip_prefix("On "))?;
        rest.split_once(": ").map(|(branch, _)| branch)
    }
}

/// 逐行追溯（`git blame`）的结果：每一行最后一次由哪次提交改动。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blame {
    /// 出现过的提交，按第一次出现的顺序。
    pub commits: Vec<BlameCommit>,
    /// 每一行（从 0 起）是 `commits` 里的第几个。
    pub lines: Vec<u32>,
}

/// 逐行追溯里的一次提交。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlameCommit {
    /// 完整的提交号；尚未提交的行全是 0。
    pub oid: String,
    /// 作者的名字。
    pub author: String,
    /// 作者写下这次提交的时间（Unix 秒）。
    pub time: i64,
    /// 作者所在时区与 UTC 的差（秒），东八区是 28800。
    pub offset: i64,
    /// 说明的第一行。
    pub summary: String,
    /// 这些行还没有提交（在当前工作区里改过、尚未提交的行）。
    pub uncommitted: bool,
}

/// 日志的筛选条件。
///
/// 都不填时是所有分支的全部提交。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogQuery {
    /// 说明里含有这段文字（不分大小写，按字面找，不是正则）。
    pub text: String,
    /// 只看该分支（本地或远程，如 `origin/main`）能走到的提交。
    pub branch: Option<String>,
    /// 作者名字或邮箱里含有这段文字（不分大小写，按字面找）。
    pub author: Option<String>,
    /// 只看改过该文件的提交（文件历史），越过改名继续往前查找。
    pub path: Option<PathBuf>,
    /// 固定在最前面的提交（完整的提交号），见 [`Git::log_stream`]；「转到提交号、分支或标签」
    /// 找到的提交不在已列出的提交中时使用。
    pub pin: Option<String>,
}

impl LogQuery {
    /// 是否因为按说明、作者或文件筛选而画不出提交图。
    ///
    /// 这些条件下结果里的父提交大多不在列表里，提交图连不起来；只按分支筛选时提交仍然相连，
    /// 可以画图。
    pub fn graphless(&self) -> bool {
        !self.text.trim().is_empty()
            || self.author.is_some()
            || self.path.is_some()
            || self.pin.is_some()
    }

    /// 是否按说明、分支或作者筛选过，或固定了一个提交。
    ///
    /// 文件历史的文件不算：那本来就是该标签展示的内容。
    pub fn filtered(&self) -> bool {
        !self.text.trim().is_empty()
            || self.branch.is_some()
            || self.author.is_some()
            || self.pin.is_some()
    }
}

/// 日志最长的运行时间（30 秒，经验值）。
///
/// 几十万个提交的仓库取一批也只要零点几秒。
const LOG_TIMEOUT: Duration = Duration::from_secs(30);

/// 检查 macOS 开发者工具是否已安装的最长时间（5 秒，经验值）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 一个文件相对于某个版本的变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Change {
    /// 被删除。
    Deleted,
    /// 未跟踪的新文件。
    Untracked,
    /// 新加入版本管理。
    Added,
    /// 被改名（可能同时被修改）。
    Renamed,
    /// 内容或类型被修改。
    Modified,
    /// 合并时有冲突。
    Conflicted,
}

impl Change {
    /// 状态的单字母写法，用在文件树和变更列表里。
    pub fn letter(self) -> char {
        match self {
            Change::Deleted => 'D',
            Change::Untracked => 'U',
            Change::Added => 'A',
            Change::Renamed => 'R',
            Change::Modified => 'M',
            Change::Conflicted => '!',
        }
    }

    /// 把 `git status --porcelain=v2` 中 `XY` 的一位字母转换成变化类型，`.` 表示没有变化。
    fn from_code(code: u8) -> Option<Change> {
        match code {
            b'M' | b'T' => Some(Change::Modified),
            b'A' | b'C' => Some(Change::Added),
            b'D' => Some(Change::Deleted),
            b'R' => Some(Change::Renamed),
            b'U' => Some(Change::Conflicted),
            _ => None,
        }
    }
}

/// 状态里的一个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 文件的绝对路径。
    pub path: PathBuf,
    /// 改名前的绝对路径（只有改名时才有）。
    pub from: Option<PathBuf>,
    /// 已暂存的变化：暂存区相对于 `HEAD`。
    pub staged: Option<Change>,
    /// 未暂存的变化：工作区相对于暂存区。
    ///
    /// 未跟踪、冲突的文件只有这一项。
    pub unstaged: Option<Change>,
    /// 该条目是整个未被跟踪的目录（`git` 只报一项，路径以 `/` 结尾）；读状态时记下，绘制时
    /// 不再查询文件系统。
    pub is_dir: bool,
}

impl Entry {
    /// 是否是未跟踪的文件（或目录）。
    pub fn untracked(&self) -> bool {
        self.unstaged == Some(Change::Untracked)
    }

    /// 是否为合并时有冲突的文件。
    pub fn conflicted(&self) -> bool {
        self.staged == Some(Change::Conflicted) || self.unstaged == Some(Change::Conflicted)
    }

    /// 在文件树里显示的状态：两项中优先级更高的一项。
    pub fn shown(&self) -> Change {
        match (self.staged, self.unstaged) {
            (Some(a), Some(b)) => a.max(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => Change::Modified,
        }
    }
}

/// `HEAD` 指向哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// 在某个分支上。
    Branch(String),
    /// 分离的 `HEAD`，值是提交号的前 7 位。
    Detached(String),
}

/// `git status` 最多读取的有变化的文件数（个）。
///
/// 超过时只保留前 `STATUS_LIMIT` 个，并标记 [`Status::truncated`]。
///
/// 没被忽略的数据目录、虚拟环境可以有数十万个文件，每次刷新都重建全部条目会占用数百 MB 内存。
pub const STATUS_LIMIT: usize = 10_000;

/// 一次 `git status` 的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// 当前的 `HEAD`；仓库状态异常、读不出时为 `None`。
    pub head: Option<Head>,
    /// `HEAD` 指向的完整提交号；还没有提交过时为 `None`。
    ///
    /// `HEAD` 变化时（提交、切换分支），提交号随之变化。
    /// 据此判断各文件在 `HEAD` 里的内容是否需要重新取。
    pub oid: Option<String>,
    /// 有上游分支时，本地领先的提交数。
    pub ahead: u32,
    /// 有上游分支时，本地落后的提交数。
    pub behind: u32,
    /// 有变化的文件，按 `git` 输出的顺序（路径排序）。
    pub entries: Vec<Entry>,
    /// 被忽略规则（各级 `.gitignore`、`.git/info/exclude`、全局忽略文件）忽略且未跟踪的文件和目录。
    ///
    /// 整个目录被忽略时只包含目录本身，不列出其中的文件（`--ignored=matching`）。
    pub ignored: Vec<PathBuf>,
    /// 正在进行、停下来等用户处理冲突的操作（合并、变基……）；没有时为 `None`。
    pub operation: Option<Operation>,
    /// 有变化的文件超过 [`STATUS_LIMIT`] 个、`entries` 只含前一部分时为 `true`。
    pub truncated: bool,
    /// 仍含冲突标记（`<<<<<<<` … `>>>>>>>`）的冲突文件。
    ///
    /// 提交这些文件会将冲突标记一并提交。读取状态时在后台同时读取这些文件。
    pub unresolved: Vec<PathBuf>,
    /// 仓库的 `.git` 目录；工作树和子模块里是 `.git` 文件所指的目录。读不到时为 `None`。
    pub git_dir: Option<PathBuf>,
}

/// 停下来等用户处理冲突的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// `git merge`（还有 `pull` 时的合并）。
    Merge,
    /// `git rebase`。
    Rebase,
    /// `git cherry-pick`。
    CherryPick,
    /// `git revert`。
    Revert,
}

impl Operation {
    /// 命令的名字，例如「合并」「变基」。
    pub fn label(self) -> &'static str {
        match self {
            Operation::Merge => winc_i18n::t!("git-operation-merge"),
            Operation::Rebase => winc_i18n::t!("git-operation-rebase"),
            Operation::CherryPick => winc_i18n::t!("git-operation-cherry-pick"),
            Operation::Revert => winc_i18n::t!("git-operation-revert"),
        }
    }

    fn command(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        }
    }

    /// 从仓库目录 `git_dir` 里的标记文件看出正在进行的操作。
    fn detect(git_dir: &Path) -> Option<Operation> {
        // 各标记的文件信息一次取回：远端的仓库只需一次往返。先列出的优先。
        let markers = [
            ("rebase-merge", true, Operation::Rebase),
            ("rebase-apply", true, Operation::Rebase),
            ("MERGE_HEAD", false, Operation::Merge),
            ("CHERRY_PICK_HEAD", false, Operation::CherryPick),
            ("REVERT_HEAD", false, Operation::Revert),
        ];
        let paths = markers.map(|(name, _, _)| git_dir.join(name));
        let found = winc_backend::fs::metadata_many(&paths);
        (markers.into_iter().zip(found)).find_map(|((_, dir, operation), metadata)| {
            let metadata = metadata.ok()?;
            let present = if dir {
                metadata.is_dir()
            } else {
                metadata.is_file()
            };
            present.then_some(operation)
        })
    }
}

/// 仓库 `root` 的 `.git` 目录：工作树和子模块里 `.git` 是一个写着 `gitdir: <路径>` 的文件。
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if winc_backend::fs::is_dir(&dot) {
        return Some(dot);
    }
    let text = winc_backend::fs::read_to_string(&dot).ok()?;
    let target = text.trim().strip_prefix("gitdir:")?.trim();
    Some(crate::host::resolve(root, target))
}

/// 仓库共用的 `git` 目录：工作树（`git worktree`）的 `.git` 文件指向它自己的目录，hooks、
/// 各工作树的记录（`worktrees/`）在那里的 `commondir` 所指的目录里。
fn git_common_dir(root: &Path) -> Option<PathBuf> {
    let dir = git_dir(root)?;
    match winc_backend::fs::read_to_string(dir.join("commondir")) {
        Ok(common) => Some(dir.join(common.trim())),
        Err(_) => Some(dir),
    }
}

impl Status {
    /// 文件树要用的状态：每个有变化的文件，以及它们所在的各级目录（直到 `root`，不含 `root`）。
    ///
    /// 目录取子项里最显眼的状态，但删除不传给目录：目录里删掉一个文件，目录本身不显示异常。
    pub fn decorations(&self, root: &Path) -> HashMap<PathBuf, Change> {
        let mut map: HashMap<PathBuf, Change> = HashMap::new();
        for entry in &self.entries {
            let shown = entry.shown();
            map.insert(entry.path.clone(), shown);
            if shown == Change::Deleted {
                continue;
            }
            for dir in entry.path.ancestors().skip(1) {
                if dir == root || !dir.starts_with(root) {
                    break;
                }
                match map.entry(dir.to_path_buf()) {
                    // 已经有同样显眼或更显眼的状态：再往上的各级目录也已经有了。
                    std::collections::hash_map::Entry::Occupied(slot) if *slot.get() >= shown => {
                        break;
                    }
                    std::collections::hash_map::Entry::Occupied(mut slot) => {
                        slot.insert(shown);
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(shown);
                    }
                }
            }
        }
        map
    }
}

/// 找不到可用的 `git` 的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// `PATH` 里没有 `git`。
    NotInstalled,
    /// macOS 上没有安装命令行开发者工具：`/usr/bin/git` 是占位程序，运行后只会弹出系统的
    /// 安装对话框。
    NoDeveloperTools,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unavailable::NotInstalled => {
                f.write_str(winc_i18n::t!("git-unavailable-not-installed"))
            }
            Unavailable::NoDeveloperTools => {
                f.write_str(winc_i18n::t!("git-unavailable-no-developer-tools"))
            }
        }
    }
}

/// [`Git::delete_branch`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchDeleted {
    /// 已删除，内容是 `git` 的结论。
    Done(String),
    /// 分支未完全合并，需要强制删除，尚未删除。
    NeedsForce,
}

/// 系统里可用的 `git`。
#[derive(Debug, Clone)]
pub struct Git {
    /// `git` 可执行文件的路径。
    program: PathBuf,
    /// 用户 shell 的环境：`git` 命令都在这个环境里运行。
    shell: Shell,
    /// 各仓库的配置检查结果和信任状态，[`Git`] 的各个副本共用。
    trust: Arc<Mutex<Trust>>,
    /// 处理输入请求的 askpass 程序，见 [`Git::with_askpass`]。
    askpass: Option<Askpass>,
}

/// 传递 [`Askpass::endpoint`] 的环境变量名。
pub const ASKPASS_ENDPOINT: &str = "WINC_GIT_ASKPASS";

/// `git`、`ssh` 需要用户输入时运行的 askpass 程序。
#[derive(Debug, Clone)]
pub struct Askpass {
    /// 程序路径：程序以提示文字为参数启动，把答复写到标准输出。
    pub program: PathBuf,
    /// 程序连接的本地端点，由环境变量 [`ASKPASS_ENDPOINT`] 传递。
    pub endpoint: OsString,
}

impl Git {
    /// 在用户 shell 的 `PATH` 里找 `git`，确认它能用。
    ///
    /// 第一次调用要等取到环境。
    ///
    /// # Errors
    ///
    /// 找不到，或者 macOS 上找到的是没装开发者工具时的占位程序，返回 [`Unavailable`]。
    pub fn find(shell: &Shell) -> Result<Git, Unavailable> {
        // 仅在 WSL 中安装了 `git` 时仍然可用：发行版中的仓库使用发行版内的 `git`（`Git::at`）。
        let program = shell
            .find("git")
            .or_else(|| crate::wsl::available().then(|| PathBuf::from("git.exe")))
            .ok_or(Unavailable::NotInstalled)?;
        if cfg!(target_os = "macos")
            && program == Path::new("/usr/bin/git")
            && !developer_tools_installed()
        {
            return Err(Unavailable::NoDeveloperTools);
        }
        Ok(Git {
            program,
            shell: shell.clone(),
            trust: Arc::default(),
            askpass: None,
        })
    }

    /// 在仓库 `root` 里运行 `git status`。
    ///
    /// # Errors
    ///
    /// `git` 运行失败、超时、被取消或者输出读不懂时返回原因。
    pub fn status(&self, root: &Path, cancel: &Cancel) -> Result<Status, String> {
        self.inspect(root)?;
        let mut command = self.command(root);
        command.args([
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
            // 文件树以淡色绘制被忽略的项。`matching` 只列出匹配忽略规则的那一层，不列出被忽略
            // 目录中的内容（`target/`、`node_modules/` 下可能有数万个文件）。
            "--ignored=matching",
            "--ignore-submodules=dirty",
        ]);
        // 正在进行的操作从 `.git` 里的标记文件看出，与 `git status` 互不依赖：远端的仓库上
        // 同时进行。
        let marks = || {
            let dir = git_dir(root);
            let operation = dir.as_deref().and_then(Operation::detect);
            (dir, operation)
        };
        let run = || self.run(&mut command, STATUS_TIMEOUT, cancel);
        let remote = winc_backend::is_remote(root);
        let ((dir, operation), stdout) = crate::background::both(remote, marks, run);
        let mut status = parse_status(&stdout?, root)
            .ok_or_else(|| winc_i18n::t!("git-status-unreadable").to_owned())?;
        (status.git_dir, status.operation) = (dir, operation);
        status.unresolved = status
            .entries
            .iter()
            .filter(|e| e.conflicted() && merge::file_has_conflicts(&e.path))
            .map(|e| e.path.clone())
            .collect();
        Ok(status)
    }

    /// 处理完冲突以后继续 `operation`（例如 `git rebase --continue`）。
    ///
    /// 合并的继续由提交视图提交完成，不经过该函数。
    ///
    /// # Errors
    ///
    /// 还有未解决的冲突、hooks 拒绝时返回 `git` 给出的原因。
    pub fn continue_operation(&self, root: &Path, operation: Operation) -> Result<String, String> {
        self.operate(root, &[operation.command(), "--continue"])
    }

    /// 放弃 `operation`，回到开始之前的状态（例如 `git merge --abort`）。
    ///
    /// # Errors
    ///
    /// `git` 报错时返回它的说明。
    pub fn abort_operation(&self, root: &Path, operation: Operation) -> Result<String, String> {
        self.operate(root, &[operation.command(), "--abort"])
    }

    /// 合并的提交信息草稿（`.git/MERGE_MSG`，去掉 `#` 开头的说明行）；不在合并中时为 `None`。
    pub fn merge_message(&self, root: &Path) -> Option<String> {
        let text = winc_backend::fs::read_to_string(git_dir(root)?.join("MERGE_MSG")).ok()?;
        let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        Some(kept.join("\n").trim().to_owned())
    }

    /// 本地和远程分支，本地的在前，各自按名字排列。
    ///
    /// 远程的 `HEAD` 指针不列出。
    ///
    /// # Errors
    ///
    /// `git` 运行失败时返回原因。
    pub fn branches(&self, root: &Path, cancel: &Cancel) -> Result<Vec<Branch>, String> {
        let mut command = self.command(root);
        command.args([
            "for-each-ref",
            "--format=%(refname)%00%(HEAD)%00%(upstream:short)%00%(objectname)\
             %00%(committerdate:unix)%00%(committerdate:format-local:%Y-%m-%d %H:%M)",
            "refs/heads",
            "refs/remotes",
        ]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(parse_branches(&String::from_utf8_lossy(&stdout)))
    }

    /// 返回仓库配置的远程名。
    ///
    /// # Errors
    ///
    /// `git` 运行失败或超时时返回原因。
    pub fn remotes(&self, root: &Path, cancel: &Cancel) -> Result<Vec<String>, String> {
        let mut command = self.command(root);
        command.arg("remote");
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(String::from_utf8_lossy(&stdout)
            .lines()
            .map(str::trim)
            .filter(|remote| !remote.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// 返回 `root` 所在仓库的公共 Git 目录：提交、分支都记在这里，附属工作树也用它。
    ///
    /// # Errors
    ///
    /// `git` 运行失败或超时时返回原因。
    pub fn common_dir(&self, root: &Path, cancel: &Cancel) -> Result<PathBuf, String> {
        let mut command = self.command(root);
        command.args(["rev-parse", "--git-common-dir"]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        // 在主工作树里是相对路径，相对于运行的目录。
        Ok(crate::host::resolve(
            root,
            String::from_utf8_lossy(&stdout).trim(),
        ))
    }

    /// 返回最近切换到过的分支名，最近的在前，不重复（来自 `HEAD` 的引用日志）。
    ///
    /// # Errors
    ///
    /// `git` 运行失败或超时时返回原因。
    pub fn recent_branches(&self, root: &Path, cancel: &Cancel) -> Result<Vec<String>, String> {
        let mut command = self.command(root);
        command.args([
            "log",
            "--walk-reflogs",
            "--format=%gs",
            "--grep-reflog=checkout: moving from",
            "--max-count=200",
            "HEAD",
        ]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(parse_recent_branches(&String::from_utf8_lossy(&stdout)))
    }

    /// 在仓库 `root` 里运行会改动仓库的 `git` 操作 `args`（如 `["pull"]`），返回 `git` 的结论。
    ///
    /// 分支名、提交号、储藏名等来自仓库或用户的参数之前必须加 `--end-of-options`，否则仓库里名为
    /// `--output=…` 的引用会被当作选项。
    ///
    /// # Errors
    ///
    /// 失败时返回 `git` 给出的原因：优先取 `error::`、`fatal::`、`! [rejected]` 这样的行。
    pub fn operate<S: AsRef<OsStr>>(&self, root: &Path, args: &[S]) -> Result<String, String> {
        if let Err(refused) = self.require_trust(root) {
            output::record(root, args, None, refused.as_bytes());
            return Err(refused);
        }
        self.run_operation(root, args, OPERATION_TIMEOUT)
    }

    /// 把 `paths` 的当前状态加入暂存区（`git add --all`），已删除的文件记为删除。
    ///
    /// 路径以 `/` 分隔并按字面匹配：`[`、`*`、`?` 不作为通配符。
    ///
    /// # Errors
    ///
    /// 同 [`Git::operate`]。
    pub fn stage(&self, root: &Path, paths: &[PathBuf]) -> Result<String, String> {
        let mut args: Vec<OsString> = ["add", "--all", "--"].map(OsString::from).into();
        args.extend(relative_all(root, paths));
        self.operate(root, &args)
    }

    /// 在目录 `root` 里运行 `git args`，记录输出，返回 `git` 的结论；调用方负责信任检查。
    fn run_operation<S: AsRef<OsStr>>(
        &self,
        root: &Path,
        args: &[S],
        timeout: Duration,
    ) -> Result<String, String> {
        self.run_prepared(root, args, timeout, |_| {})
    }

    /// 和 [`Git::run_operation`] 相同，运行前由 `prepare` 调整命令，例如设置环境变量。
    fn run_prepared<S: AsRef<OsStr>>(
        &self,
        root: &Path,
        args: &[S],
        timeout: Duration,
        prepare: impl FnOnce(&mut Command),
    ) -> Result<String, String> {
        let mut command = self.command(root);
        self.ask_through(root, &mut command);
        prepare(&mut command);
        command.args(args).stderr(Stdio::piped());
        match process::run(&mut command, Vec::new(), timeout, &Cancel::default()) {
            Ok(output) => {
                let mut text = output.stderr;
                text.extend_from_slice(&output.stdout);
                output::record(root, args, output.status.code(), &text);
                if output.status.success() {
                    Ok(summary(&text))
                } else {
                    Err(failure(&text))
                }
            }
            Err(e) => {
                let reason = describe(&e);
                output::record(root, args, None, reason.as_bytes());
                Err(reason)
            }
        }
    }

    /// 返回使用 `askpass` 处理输入请求的副本；`None` 表示需要输入时直接失败。
    #[must_use]
    pub fn with_askpass(mut self, askpass: Option<Askpass>) -> Self {
        self.askpass = askpass;
        self
    }

    /// 为 `command` 设置 [`Git::with_askpass`] 指定的 askpass 程序。
    ///
    /// `git` 优先使用凭据助手，没有结果时才运行该程序。`SSH_ASKPASS_REQUIRE` 使 `ssh` 不依赖
    /// `DISPLAY` 和控制终端。WSL 发行版内的 `git` 经互操作运行该程序，`WSLENV` 另外列出
    /// [`crate::wsl::ASKPASS_ENV`]；发行版禁用了互操作时程序无法运行，需要输入时 `git` 报错。
    fn ask_through(&self, root: &Path, command: &mut Command) {
        let Some(askpass) = &self.askpass else {
            return;
        };
        // 远端的仓库：`git` 在那台主机上运行，它的输入请求由服务进程转回本机，再交给这里的
        // askpass 程序（见 `process::run`），这些变量本身不发到远端。
        command
            .env("GIT_ASKPASS", &askpass.program)
            .env("SSH_ASKPASS", &askpass.program)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ASKPASS_ENDPOINT, &askpass.endpoint);
        if crate::wsl::locate_here(root).is_some() {
            let list = format!("{}:{}", crate::wsl::GIT_ENV, crate::wsl::ASKPASS_ENV);
            command.env("WSLENV", list);
        }
    }

    /// 删除本地分支 `name`。
    ///
    /// `force` 时用 `--force` 连同未合并的提交一起删除；否则分支未完全合并时返回
    /// [`BranchDeleted::NeedsForce`]，不改动仓库。
    ///
    /// # Errors
    ///
    /// 分支不存在、被某个工作树签出等其他失败时返回 `git` 给出的原因。
    pub fn delete_branch(
        &self,
        root: &Path,
        name: &str,
        force: bool,
    ) -> Result<BranchDeleted, String> {
        let mut args = vec!["branch", "--delete"];
        if force {
            args.push("--force");
        }
        args.extend(["--end-of-options", name]);
        match self.operate(root, &args) {
            Ok(summary) => Ok(BranchDeleted::Done(summary)),
            // 报错固定为英文（`LC_ALL=C`），分支名不含空格，不会和这句话混淆。
            Err(e) if !force && e.contains("is not fully merged") => Ok(BranchDeleted::NeedsForce),
            Err(e) => Err(e),
        }
    }

    /// 推送当前分支。
    ///
    /// 没有上游时发布到 [`Git::publish_target`] 选出的远程（见 [`Git::publish`]）。
    ///
    /// # Errors
    ///
    /// 没有任何远程、`git` 报错（被拒绝、需要认证）时返回原因。
    pub fn push(&self, root: &Path, has_upstream: bool) -> Result<String, String> {
        self.require_trust(root)?;
        if has_upstream {
            return self.operate(root, &["push"]);
        }
        let remote = self.publish_target(root)?;
        self.publish(root, &remote)
    }

    /// 发布当前分支：推到远程 `remote` 上同名的分支，并设为上游（`push -u`）。
    ///
    /// # Errors
    ///
    /// `remote` 以 `-` 开头、`git` 报错（被拒绝、需要认证）时返回原因。
    pub fn publish(&self, root: &Path, remote: &str) -> Result<String, String> {
        self.require_trust(root)?;
        // 远程名来自仓库的配置，`git` 允许以 `-` 开头的远程名（`--receive-pack=…` 会运行命令）：
        // 拒绝这样的远程，推送时另外加 `--end-of-options`。
        if remote.starts_with('-') {
            return Err(winc_i18n::t!("git-push-no-remote").to_owned());
        }
        self.operate(
            root,
            &["push", "--set-upstream", "--end-of-options", remote, "HEAD"],
        )
    }

    /// 返回发布当前分支（第一次推送）时使用的远程：优先取分支和仓库配置的推送目标，其次是
    /// `origin`，都没有时取第一个；跳过以 `-` 开头的远程名。
    ///
    /// # Errors
    ///
    /// 没有可用的远程或 `git` 运行失败时返回原因。
    pub fn publish_target(&self, root: &Path) -> Result<String, String> {
        let remotes = self.remotes(root, &Cancel::default())?;
        let usable: Vec<&String> = remotes.iter().filter(|r| !r.starts_with('-')).collect();
        let preferred = self.preferred_remote(root);
        publish_remote(&usable, &preferred)
            .cloned()
            .ok_or_else(|| winc_i18n::t!("git-push-no-remote").to_owned())
    }

    /// 当前分支配置的推送目标，优先的在前，顺序和 `git push` 选择远程时相同；上游已不存在的分支
    /// 仍然留着 `branch.<名字>.remote`。
    fn preferred_remote(&self, root: &Path) -> Vec<String> {
        let cancel = Cancel::default();
        let mut command = self.command(root);
        command.args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let branch = self.run(&mut command, STATUS_TIMEOUT, &cancel);
        let branch = branch.map(|out| String::from_utf8_lossy(&out).trim().to_owned());
        let branch = branch.unwrap_or_default();
        [
            format!("branch.{branch}.pushRemote"),
            "remote.pushDefault".to_owned(),
            format!("branch.{branch}.remote"),
        ]
        .iter()
        .filter_map(|key| self.config(root, key, &cancel))
        .collect()
    }

    fn command(&self, root: &Path) -> Command {
        let mut command = self.at(root);
        command
            // 不获取可选的锁（例如刷新索引）：后台运行的 `git` 不能和用户在同一仓库里运行的
            // `git` 抢同一把锁。
            .env("GIT_OPTIONAL_LOCKS", "0")
            // 禁用终端提示：没有 askpass 程序（见 `Git::ask_through`）时直接失败，不阻塞进程。
            .env("GIT_TERMINAL_PROMPT", "0")
            // 不打开编辑器：`rebase --continue` 等操作会打开它修改提交信息，这里沿用原来的信息。
            .env("GIT_EDITOR", ":")
            // 不验证签名：验证需要运行 `gpg.program`，仓库配置不可信时它可以是任意命令；验证结果
            // 还会混入 `log`、`show`、`stash list` 的输出，破坏解析。
            .args(["-c", "log.showSignature=false"]);
        // 配置不可信：fsmonitor 和仓库里定义的过滤器都不运行；必需的过滤器被清空命令后 `git` 会
        // 报错，不会把没经过过滤的内容当成结果。会改仓库的操作在此之前已经被拒绝，剩下的只读命令
        // 中只有 `diff.*.textconv` 会运行仓库配置的命令，`blame` 总是带 `--no-textconv`。
        if let Some(risks) = self.untrusted(root) {
            command.args(["-c", "core.fsmonitor=false"]);
            for name in &risks.filters {
                for key in ["clean", "smudge", "process"] {
                    command.arg("-c").arg(format!("filter.{name}.{key}="));
                }
            }
        }
        command
    }

    /// 返回在仓库 `root` 中运行 `git` 的命令，调用方继续追加 `git` 的参数。
    ///
    /// 发行版中的仓库使用发行版内的 `git`：跨文件系统读取仓库的速度较低，文件属主不同时 `git` 还会
    /// 按 `safe.directory` 拒绝访问。
    ///
    /// 环境中的 [`REPOSITORY_ENV`] 不传给 `git`：仓库由 `root` 决定，调用方需要时（例如临时暂存区）
    /// 在返回的命令上另行设置。
    pub(super) fn at(&self, root: &Path) -> Command {
        // 远端主机上的仓库：以仓库为工作目录运行远端的 `git`，按名字在远端的 `PATH` 中查找
        // （本机 `git` 的路径和本机 shell 的环境在远端没有意义）。命令经 `process::run` 在该
        // 目录所在的主机上运行。
        if winc_backend::is_remote(root) {
            let mut command = Command::new("git");
            command.current_dir(root);
            return command;
        }
        let mut command = match crate::wsl::locate_here(root) {
            None => {
                let mut command = self.shell.command(&self.program);
                command.arg("-C").arg(root);
                command
            }
            Some(place) => {
                let mut command = self.shell.command(Path::new("wsl.exe"));
                crate::wsl::enter(&mut command, &place.distro);
                command
                    .args(["--exec", "git", "-C", &place.path])
                    .env("WSLENV", crate::wsl::GIT_ENV);
                command
            }
        };
        for name in REPOSITORY_ENV {
            command.env_remove(name);
        }
        command
    }

    /// 返回文件在 `git` 中的模式：可执行文件为 `100755`，其余为 `100644`。
    ///
    /// Windows 无法读取发行版中文件的可执行位，因此在发行版内查询。查询失败时返回错误：按不可执行
    /// 处理会使提交移除文件的可执行位。
    fn file_mode(&self, path: &Path, cancel: &Cancel) -> Result<&'static str, String> {
        if let Some(place) = crate::wsl::locate_here(path) {
            let mut command = self.shell.command(Path::new("wsl.exe"));
            crate::wsl::enter(&mut command, &place.distro);
            command
                .args(["--exec", "test", "-x", &place.path])
                .stderr(Stdio::piped());
            let output = process::run(&mut command, Vec::new(), SHOW_TIMEOUT, cancel)
                .map_err(|e| describe(&e))?;
            return mode_from_test(output.status.code()).ok_or_else(|| failure(&output.stderr));
        }
        #[cfg(unix)]
        {
            let executable =
                |m: winc_backend::fs::Metadata| m.unix.is_some_and(|u| u.mode & 0o111 != 0);
            if winc_backend::fs::metadata(path).is_ok_and(executable) {
                return Ok("100755");
            }
        }
        Ok("100644")
    }

    fn run(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancel: &Cancel,
    ) -> Result<Vec<u8>, String> {
        command.stderr(Stdio::piped());
        match process::run(command, Vec::new(), timeout, cancel) {
            Ok(output) if output.status.success() => Ok(output.stdout),
            Ok(output) => Err(failure(&output.stderr)),
            Err(e) => Err(describe(&e)),
        }
    }
}

/// 字符串是否符合完整或缩写提交号的格式。
pub fn is_commit_hash(text: &str) -> bool {
    (4..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 开着的 `git log` 进程，见 [`Git::log_stream`]。
///
/// 丢弃时连同 `git` 进程一起结束。
pub struct LogStream {
    stream: process::Stream,
    /// 按提交号找到的提交（[`Git::commit_by_hash`]），在第一批中排在最前面。
    pinned: Option<Box<Commit>>,
    /// 排在最前面的提交号；说明同样匹配时 `git log` 会再次输出该提交，读取时跳过。
    skip: Option<String>,
}

impl LogStream {
    /// 继续读最多 `max` 个提交；不满 `max` 个说明读完了。
    ///
    /// 每一批最多等 `LOG_TIMEOUT`。
    ///
    /// # Errors
    ///
    /// `git` 报错（还没有提交、分支不存在）、超时或读输出出错时返回原因。读取出错后该日志
    /// 不能再继续读。
    pub fn next(&mut self, max: usize) -> Result<Vec<Commit>, String> {
        let mut commits = self.read(max)?;
        if let Some(at) = commits
            .iter()
            .position(|c| Some(&c.oid) == self.skip.as_ref())
        {
            commits.remove(at);
            self.skip = None;
            // 本批数量少于 `max` 会被视为已读完，因此再读取一个补足。
            if commits.len() + 1 == max {
                commits.extend(self.read(1)?);
            }
        }
        if let Some(pinned) = self.pinned.take() {
            match commits.iter().position(|c| c.oid == pinned.oid) {
                Some(at) => {
                    commits.remove(at);
                }
                None => self.skip = Some(pinned.oid.clone()),
            }
            commits.insert(0, *pinned);
        }
        Ok(commits)
    }

    fn read(&mut self, max: usize) -> Result<Vec<Commit>, String> {
        let (bytes, _) = self
            .stream
            .read_records(0x1e, max, LOG_TIMEOUT)
            .map_err(|e| match e {
                process::StreamError::Run(e) => describe(&e),
                process::StreamError::Exited(stderr) => failure(&stderr),
            })?;
        Ok(parse_log(&String::from_utf8_lossy(&bytes)))
    }
}

/// `path` 相对仓库根目录 `root` 的路径，用 `/` 分隔（`git` 的路径分隔符总是 `/`）。
///
/// 各段按原样的字节拼接，不经有损转换：Unix 上文件名可以不是 UTF-8，替换成 U+FFFD 以后指向的
/// 是另一个（通常不存在的）文件。
fn slash_path(root: &Path, path: &Path) -> Result<OsString, String> {
    // 远端主机上的仓库：按远端的原始字节计算，虚拟路径中为本机所做的换算不能传给远端的 `git`。
    if let (Some(root), Some(path)) = (winc_backend::locate(root), winc_backend::locate(path)) {
        let relative = path
            .relative_to(&root)
            .ok_or_else(|| winc_i18n::t!("git-file-outside-repository"))?;
        #[cfg(unix)]
        return Ok(std::os::unix::ffi::OsStringExt::from_vec(relative.to_vec()));
        // Windows 的字符串无法表示不是有效 UTF-8 的文件名，这样的文件在远端的 `git` 中找不到。
        #[cfg(not(unix))]
        return Ok(String::from_utf8_lossy(relative).into_owned().into());
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| winc_i18n::t!("git-file-outside-repository"))?;
    Ok(slashed(relative))
}

/// 把相对路径 `relative` 的各段用 `/` 连接。
///
/// Windows 上的路径以 `\` 分隔，发行版内的 `git` 把 `\` 当作文件名中的普通字符，
/// `src\main.rs` 匹配不到 `src/main.rs`。
fn slashed(relative: &Path) -> OsString {
    let mut out = OsString::new();
    for (i, part) in relative.components().enumerate() {
        if i > 0 {
            out.push("/");
        }
        out.push(part.as_os_str());
    }
    out
}

/// 把 `parts` 依次接成一个参数，路径部分保持原样的字节（见 [`slash_path`]）。
fn joined_arg(parts: &[&OsStr]) -> OsString {
    parts.iter().fold(OsString::new(), |mut out, part| {
        out.push(part);
        out
    })
}

/// 把 `test -x` 的退出码转换为文件在 `git` 中的模式；其他退出码表示查询失败，返回 `None`。
fn mode_from_test(code: Option<i32>) -> Option<&'static str> {
    match code {
        Some(0) => Some("100755"),
        Some(1) => Some("100644"),
        _ => None,
    }
}

/// 返回 `paths` 对应的 `git` 路径参数（用于 `--` 之后）：位于 `root` 之下的路径换算为相对路径，
/// 各段以 `/` 连接（[`slashed`]）。
///
/// 每个都加上 `:(literal)`，按字面匹配：`app/[slug]/page.tsx` 里的 `[`、`*`、`?` 不当通配符，
/// 回滚、提交时不会连带改动名字碰巧匹配上的其他文件。不用 `GIT_LITERAL_PATHSPECS`：
/// `git stash push -u` 自己也要用路径匹配，开启它以后不再收录未跟踪的文件。
fn relative_all<'a>(
    root: &'a Path,
    paths: &'a [PathBuf],
) -> impl Iterator<Item = std::ffi::OsString> + 'a {
    paths.iter().map(move |p| {
        let mut spec = std::ffi::OsString::from(":(literal)");
        match p.strip_prefix(root) {
            Ok(relative) => spec.push(slashed(relative)),
            Err(_) => spec.push(p),
        }
        spec
    })
}

/// [`scan_repositories`] 最多查看的目录项数（个）：在后台运行，比 [`SCAN_ENTRIES`] 宽松，又不至于
/// 在放着海量文件的目录里长时间读磁盘。
const SCAN_DEEP_ENTRIES: usize = 65_536;
/// [`scan_repositories`] 不进入的目录：依赖和构建产物，里面不会有用户自己的仓库，文件却很多。
const SCAN_SKIPPED: [&str; 4] = ["node_modules", "target", "vendor", "__pycache__"];
/// [`find_repositories`] 最多查看的目录项数（个，经验值）：放着大量文件的目录不拖慢打开。
const SCAN_ENTRIES: usize = 4096;

/// 从 `dir` 往上找仓库的根目录：第一个含有 `.git`（目录，或工作树和子模块里的文件）的目录。
///
/// 只查文件系统，不运行 `git`。
pub fn find_root(dir: &Path) -> Option<PathBuf> {
    // 远端的目录只在那台主机的根目录以内向上查找：再往上就是本机的文件系统。
    let remote = winc_backend::is_remote(dir);
    let ancestors: Vec<&Path> = dir
        .ancestors()
        .take_while(|d| !remote || winc_backend::is_remote(d))
        .collect();
    // 各级的 `.git` 一次查完：远端的目录只需一次往返。
    let markers: Vec<PathBuf> = ancestors.iter().map(|d| d.join(".git")).collect();
    let found = winc_backend::fs::metadata_many(&markers);
    (ancestors.into_iter().zip(found))
        .find(|(_, metadata)| metadata.is_ok())
        .map(|(dir, _)| dir.to_path_buf())
}

/// 返回打开目录 `dir` 时可用的仓库：`dir` 所在的仓库（如果有，排在最前面），之后是第一层子目录里的
/// 仓库，按名字排序。
///
/// 只查文件系统，不运行 `git`。目录项很多时只看前一部分（`SCAN_ENTRIES`）。
pub fn find_repositories(dir: &Path) -> Vec<PathBuf> {
    // 只有目录（和可能指向目录的符号链接）才可能是仓库；各项的 `.git` 一次查完。
    let candidates: Vec<PathBuf> = winc_backend::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .take(SCAN_ENTRIES)
        .filter(|entry| entry.kind.is_dir() || entry.kind.is_symlink())
        .map(|entry| entry.path())
        .collect();
    let markers: Vec<PathBuf> = candidates.iter().map(|path| path.join(".git")).collect();
    let found = winc_backend::fs::metadata_many(&markers);
    let mut children: Vec<PathBuf> = (candidates.into_iter().zip(found))
        .filter(|(_, metadata)| metadata.is_ok())
        .map(|(path, _)| path)
        .collect();
    children.sort_by_cached_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    });
    find_root(dir).into_iter().chain(children).collect()
}

/// 在 `dir` 下面最多 `depth` 层里查找仓库，按路径排序，不含 `dir` 本身。
///
/// 按层往下找，较浅的目录优先用完额度（`SCAN_DEEP_ENTRIES`）。找到的仓库不再往里找；跳过隐藏
/// 目录、依赖和构建产物目录（`SCAN_SKIPPED`），不跟随符号链接。取消时返回已找到的部分。
pub fn scan_repositories(dir: &Path, depth: usize, cancel: &Cancel) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![dir.to_path_buf()];
    let mut budget = SCAN_DEEP_ENTRIES;
    for _ in 0..depth {
        let mut next = Vec::new();
        for parent in &level {
            if cancel.is_cancelled() || budget == 0 {
                found.sort();
                return found;
            }
            let Ok(entries) = winc_backend::fs::read_dir(parent) else {
                continue;
            };
            let mut candidates = Vec::new();
            for entry in entries.flatten().take(budget) {
                budget -= 1;
                let name = entry.name.to_string_lossy();
                let skipped = name.starts_with('.') || SCAN_SKIPPED.contains(&name.as_ref());
                if !skipped && entry.kind.is_dir() {
                    candidates.push(entry.path());
                }
            }
            // 这一个目录下各项的 `.git` 一次查完。
            let markers: Vec<PathBuf> = candidates.iter().map(|path| path.join(".git")).collect();
            let repositories = winc_backend::fs::metadata_many(&markers);
            for (path, repository) in candidates.into_iter().zip(repositories) {
                if repository.is_ok() {
                    found.push(path);
                } else {
                    next.push(path);
                }
            }
        }
        level = next;
    }
    found.sort();
    found
}

/// macOS 上命令行开发者工具是否已安装（`xcode-select -p` 成功）。
fn developer_tools_installed() -> bool {
    let mut command = Command::new("/usr/bin/xcode-select");
    command.arg("-p").stderr(Stdio::null());
    matches!(
        process::run(&mut command, Vec::new(), PROBE_TIMEOUT, &Cancel::default()),
        Ok(output) if output.status.success()
    )
}

/// 选出发布分支（第一次推送）的远程：先按 `preferred`（当前分支和仓库配置的推送目标，优先的在前），
/// 其次是 `origin`，都没有时取 `remotes` 中的第一个；没有远程时返回 `None`。
///
/// 与 `git push` 一致：没有配置推送目标时推到 `origin`。远程按名字排列，审阅他人的拉取请求时
/// 添加的远程可能排在 `origin` 之前，不能直接取第一个。
fn publish_remote<'a>(remotes: &[&'a String], preferred: &[String]) -> Option<&'a String> {
    let named = |name: &str| {
        remotes
            .iter()
            .find(|remote| remote.as_str() == name)
            .copied()
    };
    preferred
        .iter()
        .find_map(|name| named(name))
        .or_else(|| named("origin"))
        .or_else(|| remotes.first().copied())
}

#[cfg(test)]
mod tests;
