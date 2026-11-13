//! Git 的提交视图（侧栏）：改动列表、勾选、回滚、储藏、提交信息、提交和修改上一次提交。
//!
//! 视图自上而下是标题行、分支行、说明条（合并、变基暂停时才有）、改动列表、储藏（有储藏时）、
//! 上一次提交（未推送时可撤销），最下面是提交卡片（提交信息 2 至 8 行，「修改上次提交」和「提交
//! ⌄」）；列表分「冲突」「更改」「未跟踪的文件」三组。提交只含勾选的文件（`git commit --only`），
//! 冲突未解决时拒绝提交；选中文件时在预览标签里打开差异。绘制在 `view/git/commit.rs`，点击区域见
//! `commit_hits`。

mod actions;
pub(in crate::app) use actions::COMMITTED_SHOWN;
pub(crate) use actions::{PushIntent, SyncState};
mod list;
pub(in crate::app::git) use list::rows_of;
mod stash;
pub(crate) use stash::{STASH_GAP, StashLine, StashView};
mod last;
mod menu;
mod message;
mod press;
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
use crate::theme::{BUTTON_GAP, SIDEBAR_GAP, SIDEBAR_HEADER, SIDEBAR_LIST_GAP, TOP_BAR_END};

/// 列表一行的高度（逻辑像素），取侧栏列表统一的行高。
pub(crate) const COMMIT_ROW: f32 = 24.0;
/// 标题行上工具按钮的边长（逻辑像素），与 `BUTTON_SMALL` 同高。
const TOOL: f32 = 24.0;
/// 提交卡片底行的高度（逻辑像素）：勾选框和提交按钮一行，与 `BUTTON_SMALL` 同高。
const FOOTER_H: f32 = 24.0;
/// 提交卡片底行与卡片左、右、下边缘的距离（逻辑像素）。
///
/// 取 4：按钮外沿到卡片边缘的空隙和文字到卡片左边的空隙（`MESSAGE_PAD` 加行高里字形左右的
/// 空白，约 5.5）相近。
const CARD_INSET: f32 = 4.0;
/// 提交按钮文字两侧的留白（逻辑像素）。
///
/// 按钮 24 高，比对话框里 28 高的按钮（文字两侧留 14）小一号，留白随之取 10。
const BUTTON_PAD: f32 = 10.0;
/// 底行里勾选框的文字和提交按钮之间至少留出的距离（逻辑像素）；放不下时勾选框收进按钮的菜单。
///
/// 取 12：比按钮之间的间隔（`BUTTON_GAP`，2）宽得多，两样东西不会看成一组按钮。
const FOOTER_GAP: f32 = 12.0;
/// 提交信息框最少显示的行数（行）。
///
/// 取 2：只有一行时底行的按钮比信息本身还高，看起来像一个按钮而不是写信息的地方。
const MESSAGE_MIN_LINES: usize = 2;
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
/// 「上一次提交」那一栏里「撤销」文字两侧的留白（逻辑像素），与悬停底色的圆角相当。
const UNDO_PAD: f32 = 6.0;
/// 提交卡片、说明条左右的边距（逻辑像素），和标题行的项目名、右端按钮对齐；卡片离视图底边
/// 也是这么多。
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

/// 绘制时量好的几段文字的宽度（逻辑像素），供 [`App::commit_parts`] 排版：布局时不量文字。
///
/// 第一次绘制之前都是 0，按钮按最窄排；绘制时先量好再排版，同一帧里就用上了。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct CommitLabels {
    /// 「修改上次提交」：勾选框连同后面的文字。
    pub amend: f32,
    /// 提交按钮上的文字，连同前面的对勾。
    pub button: f32,
    /// 「上一次提交」那一栏的「撤销」。
    pub undo: f32,
}

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

/// 改动列表里一行名字的过渡状态的键（见 `TreeLabels`）：滚动、展开收起前后不变。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CommitName {
    /// 一个目录：同一个目录可以同时出现在几组里，连同组区分。
    Dir(CommitGroup, PathBuf),
    /// 一个文件（`Status::entries` 里的路径）。
    File(PathBuf),
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
/// 刷新前后的状态 `old`、`new` 里改动的文件或者它们的状态（改了、新增、删除……）是否不同：只是
/// 文件内容变了（保存）时相同。
pub(crate) fn entries_changed(old: &Status, new: &Status) -> bool {
    old.entries.len() != new.entries.len()
        || (old.entries.iter().zip(&new.entries))
            .any(|(a, b)| a.path != b.path || a.shown() != b.shown())
}

/// 刷新以后改动列表的过渡：离开的行（提交了、回滚了）原地淡出，新出现的行淡入，留下的行从原来
/// 的位置移到新位置；列表空了或者从空变成有时，空状态同时淡入、淡出。
///
/// 离开的行在新的状态里已经没有了，按刷新前的状态和各行绘制。
pub(crate) struct CommitShift {
    /// 刷新前的仓库状态。
    pub status: Status,
    /// 刷新前的各行。
    pub rows: Rc<CommitRows>,
    /// 刷新前列表滚动的距离（逻辑像素）。
    pub scroll: f32,
    pub clock: crate::app::animation::Clock,
}

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
    /// 标题行的「刷新」：重新读取状态和分支，读取期间图标旋转。
    Refresh,
    /// 标题行的「⋯」：弹出同步、储藏、远程仓库和显示方式的菜单。
    More,
    /// 组标题行：收起或展开。
    Group(CommitGroup),
    /// 组标题的勾选框：整组勾上或取消。
    GroupCheck(CommitGroup),
    /// 悬停组标题时右端的「回滚」按钮：回滚这一组里勾上的文件（先确认）。
    GroupRevert(CommitGroup),
    /// 储藏一组的标题：收起或展开。
    Stashes,
    /// 第几条储藏（`GitState::stashes` 的下标）那一行：在预览标签里看它的改动。
    Stash(usize),
    /// 正在看的储藏下面列出的一个文件（`StashView::files` 的下标）：在预览标签里看它。
    StashFile(usize),
    /// 悬停储藏那一行时右端的「应用」。
    StashApply(usize),
    /// 悬停储藏那一行时右端的「⋯」：应用、应用并删除、删除。
    StashMenu(usize),
    /// 储藏太多时最后一行的「还有 N 条」：弹出全部储藏的菜单。
    StashMore,
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
    /// 拆分按钮右边的下拉箭头：弹「提交并推送」；卡片太窄、放不下勾选框时还有「修改上一次
    /// 提交」。
    CommitMenu,
    /// 提交卡片底行左边的「修改上次提交」勾选框和文字。
    Amend,
    /// 没有更改时空状态下面的按钮：推送、拉取或发布，见 [`App::git_sync`]。
    NextStep,
    /// 「上一次提交」那一栏的「撤销」。
    Undo,
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
    /// 标题行右端的工具按钮，从左到右：「Git 日志」「刷新」「⋯」。
    pub tools: [(Rect, CommitTarget); 3],
    /// 合并、变基等操作暂停时改动列表上方的说明条及其右端的「中止」按钮。
    ///
    /// 没有进行中的操作时为 `None`。
    pub banner: Option<(Rect, Rect)>,
    /// 视图底部的提交卡片：提交信息和底行的勾选框、按钮都在里面。
    ///
    /// 不显示（见 [`App::commit_compose_shown`]）时为 `None`，下面几项都是空矩形。
    pub card: Option<Rect>,
    /// 提交信息区的外框，在卡片的上部。
    pub message: Rect,
    /// 框高过渡结束后提交信息区的外框；文字按它排版，过渡途中只绘制在 `message` 之内。
    pub message_settled: Rect,
    /// 卡片底行左边的「修改上次提交」勾选框和文字；卡片太窄时收进按钮的菜单，为 `None`。
    pub amend: Option<Rect>,
    /// 拆分按钮左边的「提交」。
    pub commit: Rect,
    /// 拆分按钮右边的下拉箭头。
    pub commit_menu: Rect,
    /// 位置选择面板对着的区域：标题行里内容的范围；只有一个位置时没有。
    pub location: Option<Rect>,
    /// 分支行。
    pub branch: Rect,
    /// 「上一次提交」那一栏（整个侧栏宽，在视图底部或提交卡片上面）及其右端的「撤销」「⋯」两个
    /// 按钮。
    ///
    /// 上一次提交不能撤销（已经推送、不在分支上、有进行中的操作）时为 `None`。
    pub undo: Option<(Rect, Rect, Rect)>,
    /// 储藏一组（整个侧栏宽，上沿一条分隔线），在改动列表下面；没有储藏时为 `None`。
    pub stashes: Option<Rect>,
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
    /// 取回了一条储藏（提交号、名字）改动的文件。
    StashFiles(String, String, stash::StashFiles),
    /// 准备好了储藏里一个文件的差异（标签的标题），或者失败的原因。
    StashDiff(
        String,
        Result<Box<winc_editor::diff_view::DiffView>, String>,
    ),
}

/// 排好的各行的缓存键：排它时的（`GitState::generation`、`CommitState::layout`）。
type RowsKey = (u64, u64);

/// 提交视图底部随内容出现、消失的一块（提交卡片、上一次提交）：上一帧的状态和过渡。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Slide {
    /// 上一帧是否显示；还没读到仓库状态时为 `None`。
    shown: Option<bool>,
    /// 起始露出比例（0–1）与时钟；不在过渡中时为 `None`。
    pub(crate) motion: Option<(f32, crate::app::animation::Clock)>,
}

impl Slide {
    /// 露出的比例：0 表示完全收起，1 表示完全露出；`present` 是本帧该不该显示，过渡途中跟着时钟。
    pub(crate) fn share(&self, present: bool) -> f32 {
        let target = f32::from(u8::from(present));
        (self.motion).map_or(target, |(from, clock)| clock.between(from, target))
    }

    /// 本帧该不该显示（`present`，`None` 表示还没读到仓库状态）是否和上一帧不同。
    fn changes(&self, present: Option<bool>) -> bool {
        matches!((self.shown, present), (Some(last), Some(now)) if last != now)
    }

    /// 与上一帧比较，变了时从当时露出的比例往另一端过渡：中途反向时不跳。`present` 为 `None`
    /// 时只记下；`clock` 为 `None` 时直接到终态。
    fn track(&mut self, present: Option<bool>, clock: Option<crate::app::animation::Clock>) {
        if !self.changes(present) {
            self.shown = present;
            return;
        }
        let before = f32::from(u8::from(self.shown == Some(true)));
        let from = (self.motion).map_or(before, |(from, c)| c.between(from, before));
        self.shown = present;
        self.motion = clock.map(|clock| (from, clock));
    }
}

/// 提交视图的状态。
pub(crate) struct CommitState {
    /// 提交信息，可以多行。
    pub message: TextField,
    /// 提交信息折行以后有几行（绘制前按框的宽度量出来），框的高度跟着它长。
    pub message_lines: usize,
    /// 行数变化后框高过渡的起始高度（逻辑像素）与时钟；不在过渡中时为 `None`。
    pub message_motion: Option<(f32, crate::app::animation::Clock)>,
    /// 提交按钮宽度变化（文字变了，例如按住 ⌥ 换成「提交并推送」）时过渡的起始宽度（逻辑像素，
    /// 连同下拉箭头）与时钟；不在过渡中时为 `None`。
    pub button_motion: Option<(f32, crate::app::animation::Clock)>,
    /// 「修改上次提交」随按钮变宽隐去、变窄出现的过渡：终点是否显示与时钟；不在过渡中时为
    /// `None`。
    pub amend_motion: Option<(bool, crate::app::animation::Clock)>,
    /// 提交卡片滑进、滑出（见 [`App::track_commit_bars`]）。
    pub(crate) card: Slide,
    /// 「上一次提交」那一栏拉开、收起。
    pub(crate) undo: Slide,
    /// 最近一帧画的上一次提交：栏收起途中（已经推送、撤销）照样写着它。
    undo_drawn: Option<LastCommit>,
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
    /// 是否把只有一个子目录、没有文件的各级目录合成一行（「src/core」），跟随设置
    /// `compactFolders`。
    compact: bool,
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
    /// 刷新以后各行的过渡；不在过渡中时为 `None`。
    pub(crate) shift: Option<CommitShift>,
    /// 鼠标停着的那一行名字放不下时，这一行写全名（每帧绘制前算，见
    /// `view::commit::commit_overflow`）。
    pub(crate) overflow: Option<CommitOverflow>,
    /// 文件行写全名时「打开文件」「回滚」两个按钮从原位滑到全名后面的过渡：那一行（`overflow`
    /// 的 `row`）和时钟。名字无需加长、只为按钮让出位置时，名字不动，过渡表现为按钮的移动。
    pub(crate) actions_slide: Option<(usize, crate::app::Clock)>,
    /// 各行名字随鼠标展开、收回的过渡，和文件树的名字一样。
    pub(crate) names: crate::app::TreeLabels<CommitName>,
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
    /// 最近一次点标题行「刷新」的时刻：读取结束、图标至少转满一圈之前图标一直转（见
    /// [`App::commit_refreshing`]）。
    pub(crate) refreshed: Option<Instant>,
    /// 绘制时量好的文字宽度，见 [`CommitLabels`]。
    pub(crate) labels: CommitLabels,
    /// 储藏一组是否展开（默认收起：储藏是暂时放到一边的东西，平时不必占地方）。
    pub(crate) stash_open: bool,
    /// 储藏一组展开、收起的过渡：起始高度（逻辑像素）和时钟；不在过渡中时为 `None`。
    pub(crate) stash_motion: Option<(f32, crate::app::animation::Clock)>,
    /// 正在储藏的预览标签里看的一条储藏。
    pub(crate) stash_view: Option<StashView>,
    /// 正在查看的储藏下面的文件是否已收起：再次点击该储藏时在收起与展开之间切换。
    pub(crate) stash_files_folded: bool,
    /// 上一帧储藏一组的终点高度（逻辑像素），见 [`App::track_stash_height`]。
    pub(crate) stash_settled: f32,
    /// 储藏的预览标签：和改动列表的预览标签分开，那个标签跟着工作区里的文件刷新。
    pub(crate) stash_preview: Option<DocumentId>,
    /// 是否为只读的浏览列表：列出历史提交中改动的文件（见 [`App::with_log_list`]），没有勾选框、
    /// 回滚等修改工作区的操作，点击文件在差异标签中打开该文件。
    pub(crate) browse: bool,
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
    /// 创建只读的浏览列表（见 [`CommitState::browse`]），平铺和合并目录的设置同 `like`。
    pub(crate) fn browsing(like: &CommitState) -> Self {
        CommitState {
            browse: true,
            flat: like.flat,
            compact: like.compact,
            ..CommitState::default()
        }
    }

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

    /// 只选中 `path`，清掉多选。
    pub(crate) fn select_only(&mut self, path: PathBuf) {
        self.marked.clear();
        self.anchor = Some(path.clone());
        self.selected = Some(path);
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
            button_motion: None,
            amend_motion: None,
            card: Slide::default(),
            undo: Slide::default(),
            undo_drawn: None,
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
            compact: true,
            collapsed_dirs: Default::default(),
            layout: 0,
            checks: 0,
            rows: RefCell::new(None),
            scroll: 0.0,
            motion: None,
            overflow: None,
            actions_slide: None,
            names: Default::default(),
            shift: None,
            selected: None,
            marked: HashSet::new(),
            anchor: None,
            preview: None,
            busy: false,
            committed: None,
            refreshed: None,
            labels: CommitLabels::default(),
            stash_open: false,
            stash_motion: None,
            stash_view: None,
            stash_files_folded: false,
            stash_settled: 0.0,
            stash_preview: None,
            browse: false,
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
            crate::layout::SidebarView::Commit => self
                .motion
                .module_fx
                .commit_tab
                .share(self.git.root.is_some()),
            _ => 1.0,
        }
    }

    /// 侧栏视图切换中 `view` 一段图标的不透明度，0 到 1；只有「提交」一段会出现、消失。
    pub(crate) fn view_tab_opacity(&self, view: crate::layout::SidebarView) -> f32 {
        match view {
            crate::layout::SidebarView::Commit => self
                .motion
                .module_fx
                .commit_tab
                .opacity(self.git.root.is_some()),
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

    /// 提交视图是否正在显示；借用提交视图显示历史提交的文件时（`commit.browse`）同样为
    /// `true`。
    pub(crate) fn commit_shown(&self) -> bool {
        self.panels.sidebar.view == crate::layout::SidebarView::Commit
            && self.layout.sidebar.right() > 0.0
            && (self.git.commit.browse || self.log_diff_tree().is_none())
    }

    /// 改动列表或历史提交的文件列表是否正在显示。
    pub(in crate::app) fn commit_list_shown(&self) -> bool {
        self.commit_shown() || self.log_diff_tree().is_some()
    }

    /// 返回正在显示的列表的状态：显示历史提交的文件时为其浏览列表，否则为提交视图。
    pub(in crate::app) fn shown_commit_list(&self) -> &CommitState {
        match self.log_diff_tree() {
            Some(_) if !self.git.commit.browse => self.log_list(),
            _ => &self.git.commit,
        }
    }

    /// 计算提交视图的布局。
    ///
    /// 自上而下依次为标题行、分支行、说明条（有进行中的操作时）、改动列表、还没推送的上一次提交，
    /// 最下面是提交卡片（提交信息在上，「修改上次提交」和「提交 ⌄」一行在下）。先看改了什么、再写
    /// 信息、最后提交，从上往下就是这个顺序。
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
            (tool(1), CommitTarget::Refresh),
            (tool(2), CommitTarget::More),
        ];
        // 浏览历史提交的文件时只有标题行和列表：列表从标题行下面一直到视图底部。
        if self.shown_commit_list().browse {
            let list_y = header.bottom() + SIDEBAR_LIST_GAP;
            let list = Rect::new(panel.x, list_y, panel.w, (panel.bottom() - list_y).max(0.0));
            return CommitParts {
                header,
                tools,
                location: None,
                branch: Rect::new(panel.x, header.bottom(), panel.w, 0.0),
                banner: None,
                card: None,
                message: Rect::default(),
                message_settled: Rect::default(),
                amend: None,
                commit: Rect::default(),
                commit_menu: Rect::default(),
                undo: None,
                stashes: None,
                list,
            };
        }
        let inner_x = panel.x + MARGIN;
        let inner_w = (panel.w - 2.0 * MARGIN).max(0.0);

        // Git 上下文：写在哪、在哪个分支、和上游差多少，提交前一眼能看到。
        let context_y = header.bottom();
        let branch = Rect::new(inner_x, context_y, inner_w, CONTEXT_ROW);
        // 位置按钮在标题行上，选择面板从标题行下面展开，和各行同宽。
        let location =
            (self.git_has_locations()).then(|| Rect::new(inner_x, header.y, inner_w, header.h));
        let banner = (self.commit_banner_height() > 0.0).then(|| {
            let rect = Rect::new(inner_x, branch.bottom() + SIDEBAR_GAP, inner_w, BANNER_H);
            let w = BANNER_ABORT_W.min(rect.w);
            let abort = Rect::new(rect.right() - 2.0 - w, rect.y + 2.0, w, rect.h - 4.0);
            (rect, abort)
        });

        let share = self.commit_card_share();
        let card_share = share;
        let (card, message, message_settled, amend, commit, commit_menu) = if share > 0.0 {
            self.commit_card_parts(panel, inner_x, inner_w, share)
        } else {
            Default::default()
        };
        // 「上一次提交」在视图底部，有提交卡片时紧接在卡片上沿：栏内上下各留 4，文字到上方分隔线
        // 和到卡片的距离相等，两者之间没有 `SIDEBAR_LIST_GAP`。改动列表在它上面结束。提交以后
        // 仍有未勾选的文件（未跟踪的文件默认不勾选）时卡片保持显示，栏同样显示。
        // 拉开、收起的途中栏按露出的高度算，里面的东西贴着上沿，从下沿底下露出来。
        let share = self.git.commit.undo.share(self.commit_undoable().is_some());
        let undo = (share > 0.0).then(|| {
            let bottom = card.map_or(panel.bottom(), |card| card.y);
            let bar = Rect::new(panel.x, bottom - UNDO_H * share, panel.w, UNDO_H * share);
            // 「⋯」比标题行的工具按钮小，中心和它们对齐，从上到下是一列。
            let h = UNDO_H - 8.0;
            let more = Rect::new(inner_x + inner_w - (TOOL + h) / 2.0, bar.y + 4.0, h, h);
            let w = self.git.commit.labels.undo + 2.0 * UNDO_PAD;
            let undo = Rect::new(more.x - BUTTON_GAP - w, more.y, w, h);
            (bar, undo, more)
        });
        let list_y = banner.map_or(branch.bottom(), |(rect, _)| rect.bottom()) + SIDEBAR_LIST_GAP;
        let above = match (undo, card) {
            (Some((bar, ..)), _) => bar.y,
            // 卡片滑入、滑出时这段间隔按露出比例伸缩，卡片完全收起时列表的下沿与视图底部重合。
            (None, Some(card)) => card.y - SIDEBAR_LIST_GAP * card_share,
            (None, None) => panel.bottom(),
        };
        // 储藏一组在改动列表下面，标题和各条储藏各占一行，上下各留 `STASH_GAP`。一组的下沿紧贴
        // 卡片：`SIDEBAR_LIST_GAP` 是改动列表和卡片之间的间隔，再加上它时下方空隙远大于上方。
        let h = self.stash_section_height();
        let stash_bottom = match (undo, card) {
            (None, Some(card)) => card.y,
            _ => above,
        };
        let stashes = (h > 0.0).then(|| Rect::new(panel.x, stash_bottom - h, panel.w, h));
        let list_end = stashes.map_or(above, |rect| rect.y);
        let list = Rect::new(panel.x, list_y, panel.w, (list_end - list_y).max(0.0));
        CommitParts {
            header,
            tools,
            location,
            branch,
            banner,
            card,
            message,
            message_settled,
            amend,
            commit,
            commit_menu,
            undo,
            stashes,
            list,
        }
    }

    /// 提交卡片的各部分：卡片、提交信息（当前的和过渡结束后的）、「修改上次提交」、提交按钮的
    /// 左右两段。
    ///
    /// 卡片贴着视图底部，信息变长时往上长。底行左边是「修改上次提交」，提交按钮按文字宽度靠右；
    /// 一行放不下两样时按钮占满整行，「修改上次提交」收进按钮的菜单。进行中的合并、变基不能修改
    /// 上一次提交，没有这个勾选框。
    ///
    /// `share` 是卡片露出的比例（见 [`App::commit_card_share`]）：不到 1 时整张卡片往下移，没露
    /// 出的部分在视图底部下面。
    fn commit_card_parts(
        &self,
        panel: Rect,
        inner_x: f32,
        inner_w: f32,
        share: f32,
    ) -> (Option<Rect>, Rect, Rect, Option<Rect>, Rect, Rect) {
        let settled_h = self.commit_message_target(panel.h);
        let message_h = self
            .git
            .commit
            .message_motion
            .map_or(settled_h, |(from, clock)| clock.between(from, settled_h));
        let card_h = message_h + FOOTER_H + CARD_INSET;
        let sunk = (1.0 - share) * (card_h + MARGIN);
        let card = Rect::new(
            inner_x,
            panel.bottom() - MARGIN - card_h + sunk,
            inner_w,
            card_h,
        );
        let message = Rect::new(card.x, card.y, card.w, message_h);
        let message_settled = Rect::new(card.x, card.y, card.w, settled_h);
        let footer = Rect::new(
            card.x + CARD_INSET,
            message.bottom(),
            (card.w - 2.0 * CARD_INSET).max(0.0),
            FOOTER_H,
        );
        let labels = self.git.commit.labels;
        let fitted = labels.button + 2.0 * BUTTON_PAD + BUTTON_MENU_W;
        let amendable = self.commit_amendable();
        let beside = if amendable {
            labels.amend + FOOTER_GAP
        } else {
            0.0
        };
        let compact = beside + fitted <= footer.w;
        let settled_w = if compact { fitted } else { footer.w };
        let button_w = self
            .git
            .commit
            .button_motion
            .map_or(settled_w, |(from, clock)| clock.between(from, settled_w));
        let whole = Rect::new(footer.right() - button_w, footer.y, button_w, FOOTER_H);
        let menu_w = BUTTON_MENU_W.min(whole.w);
        let commit = Rect::new(whole.x, whole.y, whole.w - menu_w, FOOTER_H);
        let commit_menu = Rect::new(commit.right(), whole.y, menu_w, FOOTER_H);
        // 隐去的过渡中仍排出「修改上次提交」，由绘制按进度淡出。
        let leaving = matches!(self.git.commit.amend_motion, Some((false, _)));
        let amend = (amendable && (compact || leaving))
            .then(|| Rect::new(footer.x, footer.y, labels.amend, FOOTER_H));
        (
            Some(card),
            message,
            message_settled,
            amend,
            commit,
            commit_menu,
        )
    }

    /// 是否显示提交卡片：有改动、有进行中的合并或变基、开着「修改上一次提交」或正在提交时显示。
    ///
    /// 没有可提交的东西时不显示：卡片里只会是一个灰掉的按钮，空状态下面改为写下一步（推送、
    /// 拉取或发布）。
    pub(crate) fn commit_compose_shown(&self) -> bool {
        let Some(status) = self.git.status.as_ref() else {
            return false;
        };
        !status.entries.is_empty()
            || status.operation.is_some()
            || self.git.commit.amend
            || self.git.commit.busy
    }

    /// 提交按钮现在是不是「提交并推送」：按住 ⌥ 时是（和项目菜单里 ⌥ 换出备选项一样），合并、变基
    /// 停着时、修改上一次提交时、正在提交时不是：前两种推送没有意义或要强制推送。
    pub(crate) fn commit_pushes(&self) -> bool {
        self.pointer.modifiers.alt_key()
            && !self.git.commit.busy
            && !self.git.commit.amend
            && (self.git.status.as_ref()).is_some_and(|s| s.operation.is_none())
    }

    /// 提交卡片露出的比例：0 表示完全收在视图底部下面，1 表示完全露出；滑进、滑出途中跟着时钟。
    pub(crate) fn commit_card_share(&self) -> f32 {
        self.git.commit.card.share(self.commit_compose_shown())
    }

    /// 栏里写的上一次提交：能撤销时是它，收起途中是最近一帧画的那个。
    pub(crate) fn commit_undo_shown(&self) -> Option<&LastCommit> {
        (self.commit_undoable()).or(self.git.commit.undo_drawn.as_ref())
    }

    /// 每帧绘制前调用：与上一帧比较是否显示提交卡片（见 [`App::commit_compose_shown`]）和
    /// 「上一次提交」（见 [`App::commit_undoable`]）。
    ///
    /// 卡片出现时从视图底部滑上来，消失时（提交完、改动都没了）滑下去；上一次提交在提交以后拉开，
    /// 推送、撤销以后收起。上面的改动列表和储藏跟着移动。刚读到仓库状态（启动、换仓库）时和提交
    /// 视图不在屏幕上时直接到终态：它们本来就在那里，切到提交视图时也不补放动画。
    pub(in crate::app) fn track_commit_bars(&mut self) {
        let known = self.git.status.is_some();
        let card = known.then(|| self.commit_compose_shown());
        let undoable = self.commit_undoable();
        let undo = known.then_some(undoable.is_some());
        if let Some(last) = undoable
            && self.git.commit.undo_drawn.as_ref() != Some(last)
        {
            self.git.commit.undo_drawn = Some(last.clone());
        }
        let changing = self.git.commit.card.changes(card) || self.git.commit.undo.changes(undo);
        let clock = if changing && self.commit_shown() {
            self.clock(crate::app::animation::TREE_ANIMATION)
        } else {
            None
        };
        self.git.commit.card.track(card, clock);
        self.git.commit.undo.track(undo, clock);
    }

    /// 能不能修改上一次提交：已经有提交，没有进行中的合并、变基。
    pub(crate) fn commit_amendable(&self) -> bool {
        self.git
            .status
            .as_ref()
            .is_some_and(|s| s.operation.is_none() && s.oid.is_some())
    }

    /// 打开提交视图时焦点落在哪：显示提交卡片时在提交信息里，否则在改动列表上。
    pub(crate) fn commit_view_focus(&self) -> Focus {
        if self.log_diff_tree().is_some() {
            Focus::CommitList
        } else if self.commit_compose_shown() {
            Focus::CommitMessage
        } else {
            Focus::CommitList
        }
    }

    /// 提交卡片收起以后（提交完、改动都没了）把焦点从提交信息移到改动列表上，否则键入的字会
    /// 进到看不见的输入框。
    pub(in crate::app) fn settle_commit_focus(&mut self) {
        if self.focus == Focus::CommitMessage && !self.commit_compose_shown() {
            self.focus = Focus::CommitList;
        }
    }

    /// 标题行的「刷新」是否在转：读取还没结束，或者点下去以后还没转满一圈。
    ///
    /// 读取通常几十毫秒就结束，只按读取的时长转，图标还没动就停了，看不出点过。
    pub(crate) fn commit_refreshing(&self) -> bool {
        self.git.commit.refreshed.is_some_and(|at| {
            self.git.running.is_some()
                || at.elapsed().as_secs_f32() < 1.0 / crate::theme::SPIN_SPEED
        })
    }

    /// 提交按钮要写明的范围：勾上的文件数；分支就写在上方的分支行里。
    ///
    /// 修改提交、合并或变基暂停时按钮另有说法，没有勾选时也不写范围，都返回 `None`。
    pub(crate) fn commit_scope(&self) -> Option<usize> {
        let status = self.git.status.as_ref()?;
        if self.git.commit.amend || status.operation.is_some() {
            return None;
        }
        let count = self.git.commit.checked_entries(status).count();
        (count > 0).then_some(count)
    }

    /// 可以撤销的上一次提交：`HEAD` 在分支上，指向的提交还没推送到上游（没有上游，或者本地
    /// 领先上游），没有进行中的合并、变基，也没在提交或修改上一次提交。
    pub(crate) fn commit_undoable(&self) -> Option<&LastCommit> {
        let status = self.git.status.as_ref()?;
        let last = self.git.commit.last.as_ref()?;
        if self.git.commit.busy || self.git.commit.amend || status.operation.is_some() {
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

    /// 返回说明条及其上方间隔的总高度；没有暂停的合并、变基等操作时为 0。
    fn commit_banner_height(&self) -> f32 {
        let operation = self.git.status.as_ref().and_then(|s| s.operation);
        if operation.is_some() {
            BANNER_H + SIDEBAR_GAP
        } else {
            0.0
        }
    }

    /// 返回提交信息框按行数应有的高度（逻辑像素）：[`MESSAGE_MIN_LINES`] 行起，最多
    /// [`MESSAGE_MAX_LINES`] 行；窗口较矮时缩小，为列表留出 [`LIST_MIN`]，但不少于一行。
    fn commit_message_target(&self, panel_h: f32) -> f32 {
        let lines = (self.git.commit.message_lines).clamp(MESSAGE_MIN_LINES, MESSAGE_MAX_LINES);
        let want = lines as f32 * MESSAGE_LINE + 2.0 * MESSAGE_PAD;
        let fixed = SIDEBAR_HEADER
            + CONTEXT_ROW
            + self.commit_banner_height()
            + 2.0 * SIDEBAR_LIST_GAP
            + FOOTER_H
            + CARD_INSET
            + MARGIN;
        let min = MESSAGE_LINE + 2.0 * MESSAGE_PAD;
        want.min(panel_h - fixed - LIST_MIN).max(min)
    }

    /// 记下提交信息折行后的行数 `lines`；行数变化时框高从当前高度过渡到新高度。
    pub(crate) fn set_commit_message_lines(&mut self, lines: usize) {
        if lines == self.git.commit.message_lines {
            return;
        }
        let from = self.commit_parts().message.h;
        self.git.commit.message_lines = lines;
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.git.commit.message_motion = clock.map(|c| (from, c));
    }

    /// 记下绘制时量好的文字宽度（见 [`CommitLabels`]）。提交按钮因此变宽、变窄时宽度随之过渡，
    /// 左边的「修改上次提交」因按钮放不下而隐去、因腾出位置而出现时同时淡出、淡入。
    ///
    /// 第一次量出宽度时直接到位，不做过渡。
    pub(crate) fn set_commit_labels(&mut self, labels: CommitLabels) {
        if labels == self.git.commit.labels {
            return;
        }
        let first = self.git.commit.labels == CommitLabels::default();
        let before = self.commit_parts();
        // 「修改上次提交」原本是否显示：过渡中取过渡的终点，否则取当前布局中是否排出。
        let motion = self.git.commit.amend_motion.take();
        let was = motion.map_or(before.amend.is_some(), |(shown, _)| shown);
        self.git.commit.labels = labels;
        if first {
            return;
        }
        let after = self.commit_parts();
        let width = |parts: &CommitParts| parts.commit.w + parts.commit_menu.w;
        let from = width(&before);
        if (from - width(&after)).abs() >= 0.5 {
            let clock = self.clock(crate::app::animation::TREE_ANIMATION);
            self.git.commit.button_motion = clock.map(|c| (from, c));
        }
        let now = after.amend.is_some();
        self.git.commit.amend_motion = if was == now {
            motion
        } else {
            self.clock(crate::app::animation::TREE_ANIMATION)
                .map(|c| (now, c))
        };
    }

    /// 鼠标没在拖东西、侧栏也没在开合：可以在悬停的行上展开写不下的名字。
    pub(crate) fn idle_pointer(&self) -> bool {
        self.panels.sidebar.motion.is_none() && matches!(self.pointer.drag, crate::app::Drag::None)
    }

    /// 仓库状态刷新了，`old` 是刷新前的状态，`rows` 是刷新前的各行（提交视图不在屏幕上时为
    /// `None`）：改动的文件变了时开始 [`CommitShift`]。
    ///
    /// 只是内容变了（保存文件）时文件和各自的状态都不变，不做过渡，列表照常响应点击。组、目录
    /// 正在展开或收起时也不做，两种动画不叠在一起。
    pub(in crate::app) fn begin_commit_shift(
        &mut self,
        old: Option<Status>,
        rows: Option<Rc<CommitRows>>,
    ) {
        let (Some(old), Some(rows)) = (old, rows) else {
            return;
        };
        let Some(new) = self.git.status.as_ref() else {
            return;
        };
        if !entries_changed(&old, new) || self.git.commit.motion.is_some() {
            return;
        }
        let scroll = self.git.commit.scroll;
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.git.commit.shift = clock.map(|clock| CommitShift {
            status: old,
            rows,
            scroll,
            clock,
        });
    }

    /// 窗口坐标的 `y` 是否落在写全名的那一改动行上：鼠标在这一行上时不唤出列表的滚动条，
    /// 和文件树相同。
    pub(in crate::app) fn over_commit_overflow_row(&self, y: f32) -> bool {
        self.commit_list_shown()
            && (self.shown_commit_list().overflow)
                .is_some_and(|o| (o.y..o.y + COMMIT_ROW).contains(&y))
    }

    /// 每帧算出 `overflow` 之后调用：换了写全名的文件行时，按钮从原位开始滑到全名后面；
    /// `before` 是上一帧写全名的那一行。
    pub(crate) fn track_commit_actions(&mut self, before: Option<usize>) {
        let now = (self.git.commit.overflow)
            .filter(|o| o.actions.is_some())
            .map(|o| o.row);
        if now == before {
            return;
        }
        let animate = self.animate_commit_names();
        self.git.commit.actions_slide = now
            .filter(|_| animate)
            .map(|row| (row, self.git.commit.names.clock()));
    }

    /// 窗口坐标 (`x`, `y`) 是否在改动行伸出侧栏的那一段名字上。
    pub(in crate::app) fn in_commit_overflow(&self, x: f32, y: f32) -> bool {
        self.commit_list_shown()
            && (self.shown_commit_list().overflow)
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
    ///
    /// 当前标签是从日志打开的差异时，提交视图已经可见则在该差异的文件树与工作区改动之间
    /// 切换，否则显示文件树（见 [`App::toggle_log_diff_tree`]）。
    pub(in crate::app) fn show_commit(&mut self) {
        if self.git.root.is_none() {
            self.flash(t!("git-not-a-repository"));
            return;
        }
        // 从日志打开的差异标签中，提交视图在文件树与工作区改动之间切换。
        self.toggle_log_diff_tree();
        if !self.panels.sidebar.visible {
            self.set_sidebar(true);
        }
        self.set_sidebar_view(crate::layout::SidebarView::Commit);
        if self.log_diff_tree().is_some() {
            self.reveal_log_diff_file();
        } else {
            self.git_refresh();
        }
        self.focus = self.commit_view_focus();
        self.request_redraw();
    }
}
