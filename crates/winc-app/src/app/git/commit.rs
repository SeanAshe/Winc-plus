//! Git 的提交视图（侧栏）：改动列表、勾选、回滚、提交信息、提交和修改上一次提交。
//!
//! 视图自上而下是标题行、说明条（合并、变基暂停时才有）、提交信息（1 至 8 行）、「提交 ⌄」
//! 拆分按钮和改动列表，底部是上一次提交（未推送时可撤销）；列表分「冲突」「更改」两组，默认
//! 都勾选。提交只含勾选的文件（`git commit --only`），冲突未解决时拒绝提交；选中文件时在预览
//! 标签里打开差异。绘制在 `view/git/commit.rs`，点击区域见 `commit_hits`。

mod actions;
pub(in crate::app) use actions::COMMITTED_SHOWN;
pub(crate) use actions::SyncState;
mod list;
mod state;

use state::*;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::ops::{Deref, Range};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use winc_editor::DocumentId;
use winc_editor::git::{Entry, Git, Head, LastCommit, Operation, Partial, Status};
use winc_i18n::t;
use winc_render::Rect;
use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use super::branch::branch_menu;
use super::unusable;
use crate::app::input::context_menu::{MenuButton, MenuCommand, MenuTarget, menu_below};
use crate::app::{AlertButton, App, Focus, GitAnchor, UserEvent};
use crate::field::TextField;
use crate::theme::{SIDEBAR_GAP, SIDEBAR_HEADER, SIDEBAR_LIST_GAP, TOP_BAR_END};

/// 列表一行的高度（逻辑像素），取侧栏列表统一的行高。
pub(crate) const COMMIT_ROW: f32 = 24.0;
/// 标题行上工具按钮的边长（逻辑像素），与 `BUTTON_SMALL` 同高。
const TOOL: f32 = 24.0;
/// 「提交」按钮的高度（逻辑像素），与 `BUTTON` 同高；比工具按钮高，按下时更明显。
const BUTTON_H: f32 = 28.0;
/// 说明条的高度（逻辑像素）；说明条在合并、变基暂停时显示于提交信息上方。
const BANNER_H: f32 = 28.0;
/// 视图底部「上一次提交」那一栏的高度（逻辑像素），上沿是一条分隔线。
///
/// 取 `BANNER_H`，两处单行栏等高。
const UNDO_H: f32 = 28.0;
/// Git 上下文一行的高度（逻辑像素），与行内的按钮同高（`BUTTON_SMALL`）。
const CONTEXT_ROW: f32 = 24.0;
/// 说明条右端「中止」按钮的宽度（逻辑像素）。
///
/// 40 是经验值：「中止」两个字加左右各 8 像素的留白。
const BANNER_ABORT_W: f32 = 40.0;
/// 拆分按钮右边下拉箭头那一段的宽度（逻辑像素），与 `TOGGLE_W` 同宽：两个字符宽的图标。
const BUTTON_MENU_W: f32 = 26.0;
/// 提交信息、按钮左右的边距（逻辑像素），和标题行的项目名、右端按钮对齐。
const MARGIN: f32 = 8.0;
/// 提交信息中一行的高度（逻辑像素），取多行输入框统一的行高。
pub(crate) const MESSAGE_LINE: f32 = 20.0;
/// 提交信息框内文字行与上下边框之间的留白（逻辑像素），与行数无关。
///
/// 2 是经验值：单行时框高 24，与搜索框同高。左右留白另加行高中字形上下的空白，使文字
/// 与边框的可见间距四边相等。
pub(crate) const MESSAGE_PAD: f32 = 2.0;
/// 提交信息框最多显示的行数（行），再多就在框内滚动。
///
/// 8 是经验值：再多的说明就在框内滚动，不再挤占下面列表的高度。
const MESSAGE_MAX_LINES: usize = 8;
/// 窗口很矮时，列表至少留下的高度（逻辑像素）。
///
/// 72 是经验值：三行列表各 24，仍能看出列表里有几项。
const LIST_MIN: f32 = 72.0;

/// 改动列表中悬停行的文件名溢出信息。
///
/// 文件名被截短或与右端的状态字母重叠时，悬停时显示全名并隐藏状态字母；超出侧栏的部分绘制
/// 在编辑区之上。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CommitOverflow {
    /// 列表里的第几行（[`App::commit_rows`] 的下标）。
    pub row: usize,
    /// 这一行的上沿（逻辑像素；固定在顶部的行用固定的位置）。
    pub y: f32,
    /// 伸出侧栏的那一段；全名在侧栏里写得下（只是要让开状态字母）时为 `None`。
    pub spill: Option<Rect>,
    /// 文件行跟在全名后面的「打开文件」「回滚」两个按钮；目录行为 `None`。
    pub actions: Option<(Rect, Rect)>,
}

/// 改动列表里的一组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitGroup {
    /// 已跟踪文件的改动和已暂存的新文件，默认勾上。
    Changes,
    /// 合并、拣选、变基时有冲突的文件。
    ///
    /// 冲突须先于其他改动处理，因此单独成组并排在最上方，不与普通改动混排。
    Conflicts,
    /// 未跟踪的文件（状态字母 U）：默认不勾，只有勾上才随提交加入仓库。
    Untracked,
}

impl CommitGroup {
    /// 各组在列表中自上而下的顺序。
    pub const ALL: [CommitGroup; CommitGroup::COUNT] = [
        CommitGroup::Conflicts,
        CommitGroup::Changes,
        CommitGroup::Untracked,
    ];
    /// 组的个数。
    pub const COUNT: usize = 3;
}

impl CommitGroup {
    /// 组的标题。
    pub fn title(self) -> &'static str {
        match self {
            CommitGroup::Changes => t!("git-commit-group-changes"),
            CommitGroup::Conflicts => t!("git-commit-group-conflicts"),
            CommitGroup::Untracked => t!("git-commit-group-untracked"),
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// `entry` 是否属于这一组。
    pub fn holds(self, entry: &Entry) -> bool {
        match self {
            CommitGroup::Conflicts => entry.conflicted(),
            CommitGroup::Changes => !entry.conflicted() && !entry.untracked(),
            CommitGroup::Untracked => !entry.conflicted() && entry.untracked(),
        }
    }
}

/// 列表里的一行。
///
/// 组下面按目录排成树：先是目录，再是文件，各自按名字排；只有一个子目录、没有文件的几级合成
/// 一行（「src/core」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitRow {
    /// 组的标题行。
    Group(CommitGroup),
    /// 一个目录：所在的组、目录（相对仓库根目录）、显示的名字（合起来的几级）、缩进的层数
    /// （组下面第一层是 0）。
    Dir {
        group: CommitGroup,
        dir: PathBuf,
        label: String,
        depth: usize,
    },
    /// 一个文件：它在 `Status::entries` 里的下标、缩进的层数。
    File { index: usize, depth: usize },
}

impl CommitRow {
    /// 这一行在树里的层级：组是 0，组下面第一层（目录、文件）是 1，依此类推。
    ///
    /// 固定在列表顶部的行按它排：第 n 层固定在顶部往下第 n 行。
    pub fn level(&self) -> usize {
        match self {
            CommitRow::Group(_) => 0,
            CommitRow::Dir { depth, .. } | CommitRow::File { depth, .. } => depth + 1,
        }
    }

    /// 是不是能装别的行的（组、目录）。
    fn is_container(&self) -> bool {
        !matches!(self, CommitRow::File { .. })
    }
}

/// 排好的改动列表：各行，和每组、每个目录下面有哪些文件（画勾选框、点勾选框时用，不必每行
/// 把所有改动翻一遍）。
///
/// 按状态和收起、平铺的样子排一次，缓存到它们变了为止（见 [`App::commit_rows`]）；解引用成
/// 各行。
#[derive(Debug, Default)]
pub(crate) struct CommitRows {
    rows: Vec<CommitRow>,
    /// 各组、各目录下面的文件（`Status::entries` 的下标）：一组、一个目录（收起的也算）下面的
    /// 文件在这里连成一段，它里面的目录是其中的一小段。
    members: Vec<usize>,
    /// 每组的文件在 `members` 里的那一段（下标是 `CommitGroup as usize`）。
    groups: [Range<usize>; CommitGroup::COUNT],
    /// 列表里每个目录行的文件在 `members` 里的那一段：下标同上，键是相对仓库根目录的目录。
    dirs: [HashMap<PathBuf, Range<usize>>; CommitGroup::COUNT],
    /// 组、目录行的勾选框（见 [`CommitRows::members_state`]）：（勾选的版本，按那一段在
    /// `members` 里的位置记下的结果）。
    states: RefCell<MemberStates>,
}

/// [`CommitRows::members_state`] 的缓存：勾选的版本，以及按那一段在 `members` 里的
/// （起点，长度）记下的勾选状态。
type MemberStates = (u64, HashMap<(usize, usize), Option<bool>>);

impl CommitRows {
    /// 组 `group` 里的文件（`Status::entries` 的下标）。
    pub fn group(&self, group: CommitGroup) -> &[usize] {
        &self.members[self.groups[group.index()].clone()]
    }

    /// 组 `group` 里目录 `dir`（相对仓库根目录）下面的文件；列表里没有这个目录行时为空。
    pub fn dir(&self, group: CommitGroup, dir: &Path) -> &[usize] {
        self.dirs[group.index()]
            .get(dir)
            .map_or(&[], |r| &self.members[r.clone()])
    }

    /// `members`（本表的 [`Self::group`]、[`Self::dir`] 给出的一段）的勾选状态，含义同
    /// [`CommitState::members_state`]；`checks` 是勾选的版本（[`CommitState::checks`]）。
    ///
    /// 组、目录下面可能有上万个文件，每帧都要画它们的勾选框：按位置记下结果，勾选变了、或者
    /// 重新排了各行（换成另一张表）时才重新算。
    pub fn members_state(
        &self,
        members: &[usize],
        checks: u64,
        compute: impl FnOnce() -> Option<bool>,
    ) -> Option<bool> {
        if members.is_empty() {
            return compute();
        }
        let offset = (members.as_ptr() as usize - self.members.as_ptr() as usize)
            / std::mem::size_of::<usize>();
        let mut states = self.states.borrow_mut();
        if states.0 != checks {
            *states = (checks, HashMap::new());
        }
        *states
            .1
            .entry((offset, members.len()))
            .or_insert_with(compute)
    }

    /// 排下一层已经排好序的文件 `files`（下标、相对路径）：每个一行，深 `depth` 层。
    fn push_files(&mut self, files: &[(usize, &Path)], depth: usize) {
        self.members.extend(files.iter().map(|&(index, _)| index));
        self.rows.extend(
            files
                .iter()
                .map(|&(index, _)| CommitRow::File { index, depth }),
        );
    }
}

impl Deref for CommitRows {
    type Target = [CommitRow];

    fn deref(&self) -> &[CommitRow] {
        &self.rows
    }
}

/// 列表滚动时固定在顶部的一行。
///
/// 已滚出视口的行所属的组和各级目录逐层固定在顶部，下一组或下一个目录到达时将其向上推出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CommitSticky {
    /// 它在列表各行（[`App::commit_rows`]）里的下标。
    pub row: usize,
    /// 第几层，固定在列表顶部往下第几行。
    pub level: usize,
    /// 画在哪里（被推上去时比固定的位置高）。
    pub rect: Rect,
}

/// 固定在列表顶部的最大层数（层）。
///
/// 5 是经验值：目录再深时固定的行已经占掉大半个列表。
const STICKY_MAX: usize = 5;

/// 组、目录展开或收起的动画。
///
/// 子行从父行下方逐渐显示或收回，其后的各行随之移动。
#[derive(Debug, Clone)]
pub(crate) struct CommitMotion {
    /// 动画期间画的各行：展开时是展开以后的，收起时是收起之前的。
    pub rows: Rc<CommitRows>,
    /// 展开或收起的组、目录在 `rows` 里的下标。
    pub parent: usize,
    /// 露出或收回的行数。
    pub count: usize,
    /// 是展开；`false` 是收起。
    pub expanding: bool,
    /// 这一段动画的时钟。
    pub(in crate::app) clock: crate::app::Clock,
}

impl CommitMotion {
    /// 子行在 `rows` 里的范围 `[起, 止)`。
    pub fn block(&self) -> (usize, usize) {
        (self.parent + 1, self.parent + 1 + self.count)
    }

    /// 还没露出来的子行占的高度（逻辑像素）：下面的行往上收该距离。
    pub fn hidden(&self) -> f32 {
        let eased = self.clock.progress();
        let reveal = if self.expanding { eased } else { 1.0 - eased };
        (1.0 - reveal) * self.count as f32 * COMMIT_ROW
    }
}

/// 提交视图里可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitTarget {
    /// 标题行的「Git 日志」：打开日志标签。
    Log,
    /// 标题行的「以树形显示」「以列表显示」：在按目录排成树和平铺之间切换。
    Layout,
    /// 标题行的「⋯」：弹出刷新、全部展开、全部折叠、回滚勾选的文件的菜单。
    More,
    /// 组标题行：收起或展开。
    Group(CommitGroup),
    /// 组标题的勾选框：整组勾上或取消。
    GroupCheck(CommitGroup),
    /// 目录那一行：收起或展开。
    ///
    /// 参数是它在绘制时的行号（[`App::commit_rows`] 的下标）。
    Dir(usize),
    /// 目录的勾选框：目录下的文件全部勾上或取消。
    ///
    /// 参数同 [`CommitTarget::Dir`]。
    DirCheck(usize),
    /// 文件的勾选框，参数是 `Status::entries` 的下标。
    Check(usize),
    /// 文件那一行：选中并预览差异；双击打开文件。
    File(usize),
    /// 悬停文件行时右端的「打开文件」按钮，参数同 [`CommitTarget::File`]。
    OpenFile(usize),
    /// 悬停文件行时右端的「回滚」按钮，参数同 [`CommitTarget::File`]。
    RevertFile(usize),
    /// Git 上下文的位置行：展开位置选择器。
    Location,
    /// Git 上下文的分支行：展开分支选择器。
    Branch,
    /// 分支行右端的同步控件：同步、发布或获取，见 [`App::git_sync`]。
    Sync,
    /// 拆分按钮左边的「提交」。
    Commit,
    /// 拆分按钮右边的下拉箭头：弹「提交并推送」「修改上一次提交」。
    CommitMenu,
    /// 「上一次提交」那一栏的「⋯」。
    LastCommit,
    /// 合并、变基暂停时说明条右端的「中止」按钮。
    Abort,
    /// 固定在列表顶部的组或目录：列表先滚到它真正那一行停在固定的位置（画面不动），再收起或
    /// 展开。
    ///
    /// 参数是它在各行里的下标和固定在第几层。
    Sticky(usize, usize),
}

/// 提交视图的几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CommitParts {
    /// 标题行（左边是项目和分支的上下文栏，右端是工具按钮）。
    pub header: Rect,
    /// 标题行右端的工具按钮，从左到右：「Git 日志」、树形和列表的切换、「⋯」。
    pub tools: [(Rect, CommitTarget); 3],
    /// 合并、变基等操作暂停时提交信息上方的说明条及其右端的「中止」按钮。
    ///
    /// 没有进行中的操作时为 `None`。
    pub banner: Option<(Rect, Rect)>,
    /// 提交信息区的外框。
    pub message: Rect,
    /// 框高过渡结束后提交信息区的外框；文字按它排版，过渡途中只绘制在 `message` 之内。
    pub message_settled: Rect,
    /// 拆分按钮左边的「提交」。
    pub commit: Rect,
    /// 拆分按钮右边的下拉箭头。
    pub commit_menu: Rect,
    /// 位置选择面板对着的区域：标题行里内容的范围；只有一个位置时没有。
    pub location: Option<Rect>,
    /// 分支行。
    pub branch: Rect,
    /// 视图底部「上一次提交」那一栏（整个侧栏宽）及其右端的「⋯」按钮。
    ///
    /// 上一次提交不能撤销（已经推送、不在分支上、有进行中的操作）时为 `None`。
    pub undo: Option<(Rect, Rect)>,
    /// 改动列表。
    pub list: Rect,
}

/// 后台交回来的结果。
enum Outcome {
    /// 提交完成：新提交的提交号，或者失败的原因。
    ///
    /// 第二项是提交的信息，第三项是在哪个仓库里提交的。
    Committed(Result<String, String>, String, PathBuf),
    /// 取到了上一次提交的信息（打开「修改上一次提交」时）。
    LastMessage(String),
    /// 取到了 `HEAD` 指向的提交（`HEAD` 变了以后），第二项是在哪个仓库里取的。
    Last(Option<LastCommit>, PathBuf),
    /// 撤销上一次提交完成：成功或失败的原因、撤销的提交、在哪个仓库里撤销的。
    Undone(Result<(), String>, LastCommit, PathBuf),
    /// 回滚完成：回滚的文件数，或者失败的原因；在哪个仓库里回滚的。
    RolledBack(Result<usize, String>, PathBuf),
}

/// 排好的各行的缓存键：排它时的（`GitState::generation`、`CommitState::layout`）。
type RowsKey = (u64, u64);

/// 提交视图的状态。
pub(crate) struct CommitState {
    /// 提交信息，可以多行。
    pub message: TextField,
    /// 提交信息折行以后有几行（绘制前按框的宽度量出来），框的高度跟着它长。
    pub message_lines: usize,
    /// 行数变化后框高过渡的起始高度（逻辑像素）与时钟；不在过渡中时为 `None`。
    pub message_motion: Option<(f32, crate::app::animation::Clock)>,
    /// 是否修改上一次提交。
    pub amend: bool,
    /// 打开「修改上一次提交」之前由用户填写的提交信息，关闭该模式时恢复。
    draft: Option<String>,
    /// 各位置写了一半的提交信息：切换仓库、工作树时不丢。
    drafts: HashMap<PathBuf, String>,
    /// 勾选（`excluded`）每变一次加 1，见 [`CommitRows::members_state`]。
    checks: u64,
    /// 用户取消勾选的文件；其余已跟踪的文件都勾着。
    excluded: HashSet<PathBuf>,
    /// 用户勾上的未跟踪文件；其余未跟踪的文件都不勾。
    included: HashSet<PathBuf>,
    /// 这次合并（拣选、还原）的提交信息草稿已经填进去过了：用户清空以后不再填。
    pub(super) prefilled: bool,
    /// 勾上的文件里取消勾选的各块改动（[`winc_core::diff::hunk_key`]）：这些文件只提交其余的改动。
    skipped: HashMap<PathBuf, Vec<Skip>>,
    /// 上一次见到的 `HEAD`：变了（切换分支、在其他窗口提交）时按块的勾选作废，记下的行号对不上了。
    head: Option<String>,
    /// 收起的组（下标是 `CommitGroup as usize`）。
    collapsed: [bool; CommitGroup::COUNT],
    /// 平铺显示（不按目录排成树）：每个文件一行，名字后面淡色写所在目录。默认平铺；用标题行的
    /// 切换按钮改成树形。
    flat: bool,
    /// 每一组里收起的目录（相对仓库根目录；下标是 `CommitGroup as usize`）。
    collapsed_dirs: [HashSet<PathBuf>; CommitGroup::COUNT],
    /// 收起、平铺的样子每变一次加 1。
    ///
    /// 这些字段只经下面的方法改，和 `GitState::generation` 一起作为排好的各行（`rows`）的
    /// 缓存键。
    layout: u64,
    /// 排好的各行和它的缓存键，见 [`App::commit_rows`]。
    rows: RefCell<Option<(RowsKey, Rc<CommitRows>)>>,
    /// 列表滚动的距离（逻辑像素）。
    pub scroll: f32,
    /// 正在展开或收起的组、目录的动画。
    pub(crate) motion: Option<CommitMotion>,
    /// 鼠标停着的那一行名字放不下时，这一行写全名（每帧绘制前算，见
    /// `view::commit::commit_overflow`）。
    pub(crate) overflow: Option<CommitOverflow>,
    /// 选中的文件；多选时是当前行（键盘从这里移动，预览的是它）。
    pub selected: Option<PathBuf>,
    /// 多选时选中的各个文件；为空时只有 `selected` 算选中。
    marked: HashSet<PathBuf>,
    /// 按 ⇧ 扩展选区的起点。
    anchor: Option<PathBuf>,
    /// 预览差异的标签；选中别的文件时在该标签里换内容。
    pub(super) preview: Option<DocumentId>,
    /// 是否正在提交或回滚。
    pub busy: bool,
    /// 最近一次成功提交的短提交号及其提示的截止时刻。
    ///
    /// 截止前按钮显示「✓ 已提交 abc1234」。
    pub committed: Option<(String, Instant)>,
    /// 提交成功以后继续推送（「提交并推送」）。
    push_after: bool,
    /// `HEAD` 指向的提交，「上一次提交」那一行用。
    pub last: Option<LastCommit>,
    /// `last` 是按哪个 `HEAD` 取的（外层为 `None` 表示还没取过）；`HEAD` 变了重新取。
    last_for: Option<Option<String>>,
    /// 绘制 `commit_hits` 时的状态版本（`GitState::generation`）；之后状态变了，记下的行号就
    /// 对不上了，点击不处理，等下一帧。
    pub(crate) hits_generation: u64,
    results: (Sender<Outcome>, Receiver<Outcome>),
}

impl CommitState {
    /// 勾选的版本：勾选每变一次加 1。
    pub(crate) fn checks(&self) -> u64 {
        self.checks
    }

    /// 清掉所有勾选，回到默认（提交完成以后）。
    pub(super) fn reset_checks(&mut self) {
        self.excluded.clear();
        self.included.clear();
        self.checks += 1;
    }

    /// 清掉选中的文件和多选（换目录时）。
    pub(crate) fn clear_selection(&mut self) {
        self.selected = None;
        self.marked.clear();
        self.anchor = None;
    }
}

impl Default for CommitState {
    fn default() -> Self {
        CommitState {
            message: TextField::multiline(),
            message_lines: 1,
            message_motion: None,
            amend: false,
            draft: None,
            drafts: HashMap::new(),
            excluded: HashSet::new(),
            included: HashSet::new(),
            skipped: HashMap::new(),
            head: None,
            prefilled: false,
            collapsed: [false; CommitGroup::COUNT],
            flat: true,
            collapsed_dirs: Default::default(),
            layout: 0,
            checks: 0,
            rows: RefCell::new(None),
            scroll: 0.0,
            motion: None,
            overflow: None,
            selected: None,
            marked: HashSet::new(),
            anchor: None,
            preview: None,
            busy: false,
            committed: None,
            push_after: false,
            last: None,
            last_for: None,
            hits_generation: 0,
            results: channel(),
        }
    }
}

impl App {
    /// 当前能用的侧栏视图：文件、搜索，Git 模块开着且目录在仓库里时再加提交。
    pub(crate) fn sidebar_views(&self) -> Vec<crate::layout::SidebarView> {
        let mut views = crate::layout::BASE_VIEWS.to_vec();
        if self.git.root.is_some() {
            views.push(crate::layout::SidebarView::Commit);
        }
        views
    }

    /// 顶栏里各视图切换图标的位置。
    ///
    /// 「提交」一段出现、消失途中按比例变窄，消失途中也还在。
    pub(crate) fn sidebar_tabs(&self) -> Vec<(Rect, crate::layout::SidebarView)> {
        use crate::layout::SidebarView;
        let mut views = self.sidebar_views();
        if !views.contains(&SidebarView::Commit) && self.view_tab_share(SidebarView::Commit) > 0.0 {
            views.push(SidebarView::Commit);
        }
        let views: Vec<_> = views
            .into_iter()
            .map(|v| (v, self.view_tab_share(v)))
            .collect();
        crate::layout::view_tabs(self.layout.sidebar_button, self.layout.sidebar, &views)
    }

    /// 视图 `view` 那一段的显示比例：「提交」一段出现、消失途中小于 1。
    pub(crate) fn view_tab_share(&self, view: crate::layout::SidebarView) -> f32 {
        match view {
            crate::layout::SidebarView::Commit => {
                self.module_fx.commit_tab.share(self.git.root.is_some())
            }
            _ => 1.0,
        }
    }

    /// 侧栏视图切换中 `view` 一段图标的不透明度，0 到 1；只有「提交」一段会出现、消失。
    pub(crate) fn view_tab_opacity(&self, view: crate::layout::SidebarView) -> f32 {
        match view {
            crate::layout::SidebarView::Commit => {
                self.module_fx.commit_tab.opacity(self.git.root.is_some())
            }
            _ => 1.0,
        }
    }

    /// 「提交」视图图标角上的改动文件数；没有改动、还没读到状态时为 `None`。
    pub(crate) fn commit_count_text(&self) -> Option<String> {
        let n = self.git.status.as_ref()?.entries.len();
        match n {
            0 => None,
            1..=99 => Some(n.to_string()),
            _ => Some("99+".to_owned()),
        }
    }

    /// 提交视图是否正在显示。
    pub(crate) fn commit_shown(&self) -> bool {
        self.sidebar_view == crate::layout::SidebarView::Commit && self.layout.sidebar.right() > 0.0
    }

    /// 计算提交视图的布局。
    ///
    /// 自上而下依次为标题行、说明条（有进行中的操作时）、提交信息和按钮，剩余高度分配给列表。
    pub(crate) fn commit_parts(&self) -> CommitParts {
        let s = self.layout.sidebar;
        let panel = Rect::new(s.x, TOP_BAR_END, s.w, (s.bottom() - TOP_BAR_END).max(0.0));
        let header = Rect::new(panel.x, panel.y, panel.w, SIDEBAR_HEADER);
        let tool = |i: usize| {
            let x = header.right() - 8.0 - (3 - i) as f32 * TOOL;
            Rect::new(x, header.y + (header.h - TOOL) / 2.0, TOOL, TOOL)
        };
        let tools = [
            (tool(0), CommitTarget::Log),
            (tool(1), CommitTarget::Layout),
            (tool(2), CommitTarget::More),
        ];
        let inner_x = panel.x + MARGIN;
        let inner_w = (panel.w - 2.0 * MARGIN).max(0.0);

        // Git 上下文：写在哪、在哪个分支、和上游差多少，提交前一眼能看到。
        let context_h = self.commit_context_height();
        let context_y = header.bottom();
        let branch = Rect::new(inner_x, context_y, inner_w, CONTEXT_ROW);
        // 位置按钮在标题行上，选择面板从标题行下面展开，和各行同宽。
        let location =
            (self.git_has_locations()).then(|| Rect::new(inner_x, header.y, inner_w, header.h));
        let top = context_y + context_h;
        let banner_h = self.commit_banner_height();
        let banner = (banner_h > 0.0).then(|| {
            let rect = Rect::new(inner_x, top, inner_w, BANNER_H);
            let w = BANNER_ABORT_W.min(rect.w);
            let abort = Rect::new(rect.right() - 2.0 - w, rect.y + 2.0, w, rect.h - 4.0);
            (rect, abort)
        });
        let settled_h = self.commit_message_target(panel.h);
        let message_h = self
            .commit
            .message_motion
            .map_or(settled_h, |(from, clock)| clock.between(from, settled_h));
        let message_y = top + banner_h;
        let message = Rect::new(inner_x, message_y, inner_w, message_h);
        let message_settled = Rect::new(inner_x, message_y, inner_w, settled_h);
        let button_y = message.bottom() + SIDEBAR_GAP;
        let menu_w = BUTTON_MENU_W.min(inner_w);
        let commit = Rect::new(inner_x, button_y, inner_w - menu_w, BUTTON_H);
        let commit_menu = Rect::new(commit.right(), button_y, menu_w, BUTTON_H);
        // 「上一次提交」固定在视图底部，改动列表在它上面结束。
        let undo = self.commit_undoable().is_some().then(|| {
            let bar = Rect::new(panel.x, panel.bottom() - UNDO_H, panel.w, UNDO_H);
            let h = bar.h - 8.0;
            let button = Rect::new(inner_x + inner_w - h, bar.y + 4.0, h, h);
            (bar, button)
        });
        let list_y = commit.bottom() + SIDEBAR_LIST_GAP;
        let list_end = undo.map_or(panel.bottom(), |(bar, _)| bar.y);
        let list = Rect::new(panel.x, list_y, panel.w, (list_end - list_y).max(0.0));
        CommitParts {
            header,
            tools,
            location,
            branch,
            banner,
            message,
            message_settled,
            commit,
            commit_menu,
            undo,
            list,
        }
    }

    /// 提交按钮要写明的范围：勾上的文件数；分支就写在上方的分支行里。
    ///
    /// 修改提交、合并或变基暂停时按钮另有说法，没有勾选时也不写范围，都返回 `None`。
    pub(crate) fn commit_scope(&self) -> Option<usize> {
        let status = self.git.status.as_ref()?;
        if self.commit.amend || status.operation.is_some() {
            return None;
        }
        let count = self.commit.checked_entries(status).count();
        (count > 0).then_some(count)
    }

    /// Git 上下文及其下方间隔的总高度（逻辑像素）。
    fn commit_context_height(&self) -> f32 {
        CONTEXT_ROW + SIDEBAR_GAP
    }

    /// 可以撤销的上一次提交：`HEAD` 在分支上，指向的提交还没推送到上游（没有上游，或者本地
    /// 领先上游），没有进行中的合并、变基，也没在提交或修改上一次提交。
    pub(crate) fn commit_undoable(&self) -> Option<&LastCommit> {
        let status = self.git.status.as_ref()?;
        let last = self.commit.last.as_ref()?;
        if self.commit.busy || self.commit.amend || status.operation.is_some() {
            return None;
        }
        if status.oid.as_deref() != Some(last.oid.as_str()) {
            return None;
        }
        let Some(Head::Branch(_)) = &status.head else {
            return None;
        };
        let branch = self.git.branches.iter().find(|b| b.current && !b.remote)?;
        // 上游已不存在的分支推送过，分不清这个提交发布了没有，按发布过算。
        ((branch.upstream.is_none() && !branch.gone) || status.ahead > 0).then_some(last)
    }

    /// 返回说明条及其下方间隔的总高度；没有暂停的合并、变基等操作时为 0。
    fn commit_banner_height(&self) -> f32 {
        let operation = self.git.status.as_ref().and_then(|s| s.operation);
        if operation.is_some() {
            BANNER_H + SIDEBAR_GAP
        } else {
            0.0
        }
    }

    /// 返回提交信息框按行数应有的高度（逻辑像素）：一行起，最多 [`MESSAGE_MAX_LINES`] 行；窗口
    /// 较矮时缩小，为列表留出 [`LIST_MIN`]。
    fn commit_message_target(&self, panel_h: f32) -> f32 {
        let lines = self.commit.message_lines.clamp(1, MESSAGE_MAX_LINES);
        let want = lines as f32 * MESSAGE_LINE + 2.0 * MESSAGE_PAD;
        let fixed = SIDEBAR_HEADER
            + self.commit_context_height()
            + self.commit_banner_height()
            + SIDEBAR_GAP
            + BUTTON_H
            + self.commit_undoable().map_or(0.0, |_| UNDO_H)
            + SIDEBAR_LIST_GAP;
        let min = MESSAGE_LINE + 2.0 * MESSAGE_PAD;
        want.min(panel_h - fixed - LIST_MIN).max(min)
    }

    /// 记下提交信息折行后的行数 `lines`；行数变化时框高从当前高度过渡到新高度。
    pub(crate) fn set_commit_message_lines(&mut self, lines: usize) {
        if lines == self.commit.message_lines {
            return;
        }
        let from = self.commit_parts().message.h;
        self.commit.message_lines = lines;
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.commit.message_motion = clock.map(|c| (from, c));
    }

    /// 鼠标没在拖东西、侧栏也没在开合：可以在悬停的行上展开写不下的名字。
    pub(crate) fn idle_pointer(&self) -> bool {
        self.sidebar_motion.is_none() && matches!(self.drag, crate::app::Drag::None)
    }

    /// 窗口坐标 (`x`, `y`) 是否在改动行伸出侧栏的那一段名字上。
    pub(in crate::app) fn in_commit_overflow(&self, x: f32, y: f32) -> bool {
        self.commit_shown()
            && self
                .commit
                .overflow
                .and_then(|o| o.spill)
                .is_some_and(|r| r.contains(x, y))
    }

    /// 返回提交列表的最大滚动距离；内容超出视口时末尾留出一行空白，表示已到列表末尾。
    pub(crate) fn commit_max_scroll(&self) -> f32 {
        let rows = self.commit_rows().len() as f32;
        let list = self.commit_parts().list.h;
        if rows * COMMIT_ROW <= list {
            return 0.0;
        }
        ((rows + 1.0) * COMMIT_ROW - list).max(0.0)
    }

    /// 打开提交视图（⌃⇧G），焦点放进提交信息。
    pub(in crate::app) fn show_commit(&mut self) {
        if self.git.root.is_none() {
            self.flash(t!("git-not-a-repository"));
            return;
        }
        if !self.panel_visible {
            self.set_sidebar(true);
        }
        self.set_sidebar_view(crate::layout::SidebarView::Commit);
        self.git_refresh();
        self.focus = Focus::CommitMessage;
        self.request_redraw();
    }

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
        );
        if indexed && self.commit.hits_generation != self.git.generation {
            self.request_redraw();
            return;
        }
        match target {
            CommitTarget::Log => self.git_show_log(),
            CommitTarget::Layout => self.toggle_commit_layout(),
            CommitTarget::More => {
                let rect = self.commit_parts().tools[2].0;
                self.open_button_menu(
                    MenuButton::CommitMore,
                    MenuTarget::CommitMore,
                    menu_below(rect),
                );
            }
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
                let on = self.commit.group_state(status, group) != Some(true);
                for entry in status.entries.iter().filter(|e| group.holds(e)) {
                    self.commit.set_checked(entry, on);
                }
            }
            // ⌘ 单击目录把它下面的文件加入或移出选区，⇧ 单击选到它：只改选区，不收起展开。
            CommitTarget::Dir(row) if self.command_held() => {
                self.focus = Focus::CommitList;
                self.toggle_commit_dir_selected(row);
            }
            CommitTarget::Dir(row) if self.modifiers.shift_key() => {
                self.focus = Focus::CommitList;
                self.select_commit_rows(row);
            }
            // `⌥` 单击目录：递归收起或展开。
            CommitTarget::Dir(row) if self.modifiers.alt_key() => {
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
                let on = self.commit.members_state(status, members) != Some(true);
                for &i in members {
                    self.commit.set_checked(&status.entries[i], on);
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
                if self.modifiers.shift_key() {
                    self.select_commit_range(path);
                    self.request_redraw();
                    return;
                }
                // 双击打开文件本身，单击选中并预览差异。
                let (x, y) = self.mouse;
                if self.click_count(x, y) >= 2 && path.is_file() {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                } else {
                    self.select_commit_file(path);
                }
            }
            CommitTarget::Sticky(row, level) => {
                // 先滚到那一行正好停在固定的位置，画面不动；再当作点了那一行。
                self.commit.scroll = (row as f32 * COMMIT_ROW - level as f32 * COMMIT_ROW).max(0.0);
                let target = match self.commit_rows().get(row) {
                    Some(CommitRow::Group(group)) => CommitTarget::Group(*group),
                    Some(CommitRow::Dir { .. }) => CommitTarget::Dir(row),
                    _ => return,
                };
                self.press_commit(target);
                return;
            }
            CommitTarget::OpenFile(i) => {
                let Some(path) = self.commit_entry(i).map(|e| e.path.clone()) else {
                    return;
                };
                if path.is_file() {
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
            CommitTarget::Location => self.open_location_picker(),
            CommitTarget::Branch => self.open_branch_picker(GitAnchor::Branch),
            CommitTarget::Sync => self.git_sync(),
            CommitTarget::Commit => self.git_commit(),
            CommitTarget::Abort => self.confirm_abort(),
            CommitTarget::LastCommit => {
                let Some((_, button)) = self.commit_parts().undo else {
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
        }
        self.request_redraw();
    }

    /// 提交视图标题行「⋯」的菜单：刷新；全部展开、全部折叠（已经是那样的灰显）；回滚勾选的
    /// 文件（没勾文件、正在提交时灰显）。
    pub(in crate::app) fn commit_more_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::MenuEntry;
        let anything_checked = self
            .git
            .status
            .as_ref()
            .is_some_and(|s| s.entries.iter().any(|e| self.commit.checked(e)));
        // 没有文件的组不显示，不算在内。
        let shown = || {
            let status = self.git.status.as_ref();
            CommitGroup::ALL.into_iter().filter(move |group| {
                status.is_some_and(|s| s.entries.iter().any(|e| group.holds(e)))
            })
        };
        let all_collapsed = shown().all(|group| self.commit.group_collapsed(group));
        let all_expanded = shown().all(|group| !self.commit.group_collapsed(group))
            && self.commit.collapsed_dirs.iter().all(HashSet::is_empty);
        vec![
            MenuEntry::Item(MenuCommand::CommitRefresh, true),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::CommitExpandAll, !all_expanded),
            MenuEntry::Item(MenuCommand::CommitCollapseAll, !all_collapsed),
            MenuEntry::Separator,
        ]
        .into_iter()
        .chain(branch_menu(
            &self.git.branches,
            &self.git.stashes,
            self.git.status.as_ref().and_then(|s| s.operation),
            self.branch.running.is_some(),
        ))
        .chain([
            MenuEntry::Separator,
            MenuEntry::Item(
                MenuCommand::CommitRollback,
                anything_checked && !self.commit.busy,
            ),
        ])
        .collect()
    }

    /// 「提交 ⌄」拆分按钮右边箭头的菜单：提交并推送（合并、变基停着时没有推送这回事，灰显），
    /// 修改上一次提交（开着时打勾）。
    pub(in crate::app) fn commit_button_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        let operation = self.git.status.as_ref().and_then(|s| s.operation);
        let idle = !self.commit.busy;
        vec![
            MenuEntry::Item(MenuCommand::CommitPush, idle && operation.is_none()),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::CommitAmend, idle && operation.is_none()),
        ]
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
        self.commit.push_after = true;
        self.git_commit();
        // 没有真的开始提交（没写信息、没勾文件），就不留着「之后推送」。
        if !self.commit.busy {
            self.commit.push_after = false;
        }
    }

    /// 在按目录排成树和平铺之间切换更改列表。
    ///
    /// 记进状态文件：以后打开的窗口、下次启动都用同一种显示方式。
    pub(in crate::app) fn toggle_commit_layout(&mut self) {
        let flat = !self.commit.flat();
        self.commit.set_flat(flat);
        self.update_state(|state| state.commit_tree = !flat);
        self.commit.scroll = self.commit.scroll.min(self.commit_max_scroll());
        self.reveal_commit_selection();
        self.request_redraw();
    }

    /// 提交视图标题行「⋯」的菜单选了 `command`。
    pub(in crate::app) fn run_commit_more_command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::CommitRefresh => self.git_refresh(),
            MenuCommand::CommitRollback => self.ask_rollback(),
            MenuCommand::CommitExpandAll | MenuCommand::CommitCollapseAll => {
                // 「全部展开」连收起的目录一起展开。
                self.commit
                    .collapse_all(command == MenuCommand::CommitCollapseAll);
                self.commit.scroll = self.commit.scroll.min(self.commit_max_scroll());
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
                    for path in paths.iter().filter(|p| p.is_file()) {
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
                        self.commit.set_checked(entry, on);
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
                if path.is_file() {
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

    /// 提交信息里的按键：`⌘↩` 提交；`↑` `↓` 在行之间移动，已在第一行（最后一行）时翻阅此前的
    /// 提交信息；`Tab` 移到改动列表；`Esc` 回到编辑区。
    ///
    /// 其余按键交给输入框。
    pub(in crate::app) fn on_key_commit_message(
        &mut self,
        event: &KeyEvent,
        base: &str,
        cmd: bool,
    ) {
        let shift = self.modifiers.shift_key();
        if !self.commit_message_key(&event.logical_key, cmd, shift) {
            self.on_key_field(event, base, cmd);
        }
        self.request_redraw();
    }

    /// 提交信息里自己处理的键；不是这些键时返回 `false`，交给输入框的通用处理。
    pub(in crate::app) fn commit_message_key(&mut self, key: &Key, cmd: bool, shift: bool) -> bool {
        match key {
            Key::Named(NamedKey::Enter) if cmd => self.git_commit(),
            Key::Named(NamedKey::Enter) => self.edit_field(|f| f.insert("\n")),
            // 提交信息不接受制表符，因此 `Tab` 移到改动列表；列表中按 `Tab` 返回提交信息。
            Key::Named(NamedKey::Tab) if !cmd => {
                self.field_preedit.clear();
                self.focus = Focus::CommitList;
                if self.commit.selected.is_none()
                    && let Some(first) = self.commit_files().into_iter().next()
                {
                    self.select_commit_file(first);
                }
            }
            Key::Named(NamedKey::Escape) => {
                self.field_preedit.clear();
                if self.editor.active.is_some() {
                    self.focus = Focus::Editor;
                }
            }
            // ↑ ↓ 在行之间移动。信息为空、或者正在翻阅此前的信息时，↑ ↓ 翻阅提交过的信息；
            // 写了一半的信息不会因为在第一行按了 ↑ 就被换掉。
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if !cmd => {
                let down = *key == Key::Named(NamedKey::ArrowDown);
                let message = &self.commit.message;
                let browse = !shift && (message.text().is_empty() || message.recalling());
                if browse {
                    self.edit_field(|f| f.recall(!down));
                } else {
                    self.commit.message.move_vertical(down, shift);
                }
                self.reset_blink();
            }
            _ => return false,
        }
        true
    }

    /// 切换「修改上一次提交」：打开时把上一次提交的信息填进来，关闭时恢复原来写的。
    pub(super) fn toggle_amend(&mut self) {
        self.commit.amend = !self.commit.amend;
        if !self.commit.amend {
            let draft = self.commit.draft.take().unwrap_or_default();
            self.commit.message.set_text(&draft);
            return;
        }
        self.commit.draft = Some(self.commit.message.text().to_owned());
        let sender = self.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let message = match git {
                Ok(git) => git.last_message(root),
                Err(_) => String::new(),
            };
            let _ = sender.send(Outcome::LastMessage(message));
        });
    }

    /// `HEAD` 变了（或还没取过）时在后台取它指向的提交，「上一次提交」那一行用。
    pub(super) fn fetch_last_commit(&mut self) {
        let oid = self.git.status.as_ref().and_then(|s| s.oid.clone());
        if self.commit.last_for.as_ref() == Some(&oid) {
            return;
        }
        self.commit.last_for = Some(oid.clone());
        if self.commit.last.as_ref().map(|l| &l.oid) != oid.as_ref() {
            self.commit.last = None;
        }
        if oid.is_none() {
            return;
        }
        let sender = self.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let last = git.ok().and_then(|git| git.last_commit(root));
            let _ = sender.send(Outcome::Last(last, root.to_path_buf()));
        });
    }

    /// 「上一次提交」那一栏的「⋯」菜单。
    pub(in crate::app) fn last_commit_menu(
        &self,
    ) -> Vec<crate::app::input::context_menu::MenuEntry> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        vec![
            MenuEntry::Item(MenuCommand::CommitUndo, self.commit_undoable().is_some()),
            MenuEntry::Item(MenuCommand::CommitCopyOid, true),
        ]
    }

    /// 执行「上一次提交」菜单中的命令。
    pub(in crate::app) fn run_last_commit_command(
        &mut self,
        command: crate::app::input::context_menu::MenuCommand,
    ) {
        use crate::app::input::context_menu::MenuCommand;
        let Some(last) = self.commit_undoable().cloned() else {
            return;
        };
        match command {
            MenuCommand::CommitUndo => self.confirm_undo_commit(&last),
            MenuCommand::CommitCopyOid => {
                let short = last.oid.chars().take(7).collect::<String>();
                self.flash(t!("git-log-copied-oid", commit = short.as_str()));
                self.set_clipboard(last.oid, false);
            }
            _ => {}
        }
    }

    /// 用提示框询问，确认后撤销上一次提交。更改还在，确认按钮不标记为危险操作。
    fn confirm_undo_commit(&mut self, last: &LastCommit) {
        let message = t!("git-undo-commit-confirm");
        let detail = t!(
            "git-undo-commit-confirm-detail",
            subject = last.subject.as_str()
        );
        let buttons = vec![
            AlertButton::new(t!("git-undo-commit-confirm-button")),
            AlertButton::cancel(t!("common-cancel")),
        ];
        self.show_alert(message, &detail, buttons, None, |choice, _| {
            (choice == 0).then_some(UserEvent::GitUndoCommit)
        });
    }

    /// 撤销上一次提交：分支退回上一个提交，改动留在暂存区并保持勾选，提交信息填回输入框（输入框里
    /// 已经写了别的时不覆盖）。
    pub(in crate::app) fn undo_last_commit(&mut self) {
        let Some(last) = self.commit_undoable().cloned() else {
            return;
        };
        // 那一行先收起，不会再按一次；失败时重新取。
        self.commit.last = None;
        self.commit.committed = None;
        let sender = self.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let result = git.and_then(|git| git.undo_commit(root, &last.oid));
            let _ = sender.send(Outcome::Undone(result, last, root.to_path_buf()));
        });
    }
}
