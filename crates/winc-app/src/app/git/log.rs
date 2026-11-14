//! Git 日志标签的交互：打开标签、在后台分批读取提交、读取选中提交的详情（完整说明、改动的
//! 文件）、点击文件打开该提交的并排差异，以及键盘操作。
//!
//! 绘制由 `view/git/log.rs` 完成。日志是一种只读文档（`Document::log`），每个提交占一行，
//! 滚动使用文档的通用逻辑；左侧为提交列表（提交图、说明、作者、时间），右侧为选中提交的详情。

mod arrive;
mod branches;
mod columns;
mod compare;
mod details;
mod filter;
mod keys;
pub(crate) mod menus;
mod time;
mod tree;

pub(crate) use branches::{BRANCH_ROW, BranchRow, PanelFold};
pub(crate) use columns::{AUTHOR_DEFAULT, LogColumn, log_author_width};
pub(crate) use details::LogFileRow;
pub(crate) use filter::{LogPill, since_label};
pub(crate) use time::*;
pub(crate) use tree::LogDiffTree;

use crate::app::scroll::set_scroll_line;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::encoding::decode_unknown;
use winc_editor::format::Cancel;
use winc_editor::git::log::{Arrival, Compare, Details};
use winc_editor::git::{Commit, CommitFile, Git, LogQuery, LogStream, Ref};
use winc_editor::{DocumentId, LARGE_FILE, looks_binary};
use winc_i18n::t;
use winc_render::Rect;
use winit::keyboard::{Key, NamedKey};

use super::unusable;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::app::{App, Focus, UserEvent};
use crate::field::TextField;

/// 返回日志标签的标题。
pub(crate) fn log_title() -> &'static str {
    t!("git-log-title")
}
/// 标题栏的高度（逻辑像素）。
///
/// 等于差异视图的标题栏高度，使两类标签的标题栏对齐。
pub(crate) const LOG_HEADER: f32 = 36.0;
/// 筛选输入框与分支按钮之间的间距（逻辑像素）。
pub(crate) const LOG_FILTER_GAP: f32 = 8.0;
/// 详情一栏的最小宽度与最大宽度（逻辑像素）。
///
/// 取自调参：300 可容纳提交说明和文件名；宽于 520 时提交列表过窄。拖动分隔线后不受上限
/// 限制，只为列表保留 [`LIST_MIN`]。
const DETAILS_MIN: f32 = 300.0;
const DETAILS_MAX: f32 = 520.0;
/// 提交列表的最小宽度（逻辑像素）。
///
/// 取自调参：可容纳提交图、作者名和时间三栏。
const LIST_MIN: f32 = 360.0;
/// 继续读取下一批提交时，可见区域底部与已读取的最后一个提交之间的剩余行数（行）。
///
/// 取自调参：约为两屏的滚动距离，滚动到底之前下一批已读取完成。
const PREFETCH: usize = 100;
/// 详情中文件行的高度（逻辑像素）。
///
/// 由图标按钮的高度决定：容纳 24 像素高的图标按钮，上下各留 1 像素。
pub(crate) const LOG_FILE_ROW: f32 = 26.0;
/// 提交列表第一行上方的留白（逻辑像素）。
///
/// 留白属于滚动内容，使第一行与标题栏的分隔线之间保持间距，滚动时各行仍可移到标题栏下方，
/// 选中行的底色不与分隔线相接。取值等于侧栏各块之间的间距（`SIDEBAR_GAP`）；详情一栏的
/// 第一行与之对齐（[`details_top`]）。
pub(crate) const LOG_LIST_PAD: f32 = crate::theme::SIDEBAR_GAP;

/// 返回详情一栏第一行（高 24 逻辑像素的说明）的上沿到该栏顶部的距离（逻辑像素）。
///
/// 该偏移使左侧列表第一行与右侧说明垂直居中对齐，两栏的第一行文字位于同一基线。
pub(crate) fn details_top(line_height: f32) -> f32 {
    LOG_LIST_PAD + (line_height - 24.0) / 2.0
}
/// 日志后退历史中最多保留的提交数（个）。
///
/// 取自调参：更早的跳转极少再被回退到。
const LOG_HISTORY: usize = 100;
/// 显示「正在读取…」之前等待详情返回的时长。
///
/// 取自调参：详情通常在数十毫秒内返回，等待该时长可避免提示出现后立即被结果替换而闪烁。
const LOADING_DELAY: Duration = Duration::from_millis(300);

/// 左侧分支面板的宽度（逻辑像素）：小号字约 24 个字符，常见的分支名（含远程名）写得下，
/// 更长的截短。
pub(crate) const BRANCHES_W: f32 = 200.0;

/// 日志标签的几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LogParts {
    /// 顶部的标题栏。
    pub header: Rect,
    /// 左侧的分支面板；不显示时宽度为 0。
    pub branches: Rect,
    /// 左侧的提交列表。
    pub list: Rect,
    /// 右侧的详情一栏。
    pub details: Rect,
}

impl LogParts {
    /// 标题栏最左边显示、隐藏分支面板的按钮。
    pub(crate) fn branches_button(&self) -> Rect {
        let header = self.header;
        Rect::new(
            header.x + 8.0,
            header.y + (header.h - 24.0) / 2.0,
            24.0,
            24.0,
        )
    }

    /// 标题栏左侧的提交筛选输入框，在分支面板按钮右边。
    pub(crate) fn search(&self) -> Rect {
        let header = self.header;
        let x = self.branches_button().right() + 6.0;
        Rect::new(x, header.y + 6.0, 220.0, header.h - 12.0)
    }
}

/// 返回日志标签中各部分的位置。
///
/// 标题栏下方左侧为列表，右侧为详情；详情默认约占 38% 的宽度，拖动分隔线后为 `details_w`。
/// 最左边是分支面板，露出 `branches`（0 到 1，打开、关闭途中在两者之间）；放不下（列表和
/// 详情不够最小宽度）时不显示。
pub(crate) fn log_parts(editor: Rect, details_w: Option<f32>, branches: f32) -> LogParts {
    let header = Rect::new(editor.x, editor.y, editor.w, LOG_HEADER);
    let body_h = (editor.h - LOG_HEADER).max(0.0);
    let room = editor.w - BRANCHES_W >= LIST_MIN + DETAILS_MIN;
    let side = if room { BRANCHES_W * branches } else { 0.0 };
    let branches = Rect::new(editor.x, header.bottom(), side, body_h);
    let editor = Rect::new(editor.x + side, editor.y, editor.w - side, editor.h);
    // 窗口较窄时详情最多占一半宽度；较宽时至少为列表保留 `LIST_MIN`。
    let max = (editor.w - LIST_MIN).max(editor.w * 0.5);
    let details_w = match details_w {
        Some(w) => w.max(DETAILS_MIN),
        None => (editor.w * 0.38).clamp(DETAILS_MIN, DETAILS_MAX),
    }
    .min(max);
    let list = Rect::new(editor.x, header.bottom(), editor.w - details_w, body_h);
    let details = Rect::new(list.right(), header.bottom(), details_w, body_h);
    LogParts {
        header,
        branches,
        list,
        details,
    }
}

/// 日志标签中可点击的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTarget {
    /// 一个提交（`commits` 的下标）。
    Row(usize),
    /// 提交行中的作者头像：悬停时提示作者的名字和邮箱（列表较窄、只绘制头像时用于辨认作者），
    /// 点击头像等同于点击该行。
    Avatar(usize),
    /// 详情中改动的一个文件（`files` 的下标），点击时打开该文件在该提交中的差异。
    File(usize),
    /// 详情中的父提交，点击时选中该提交。
    ///
    /// 参数为父提交的序号。
    Parent(usize),
    /// 详情中的提交号，点击时复制完整的提交号。
    CopyOid,
    /// 标题栏的「刷新」。
    Refresh,
    /// 标题栏的筛选输入框。
    Search,
    /// 标题栏的「分支」下拉，点击时弹出分支菜单。
    Branch,
    /// 标题栏的「作者」下拉，点击时弹出作者菜单。
    Author,
    /// 标题栏的「日期」下拉，点击时弹出日期菜单。
    Date,
    /// 标题栏的「路径」下拉，点击时弹出路径菜单。
    Path,
    /// 详情里改动的文件一行右端的按钮：平铺、按目录分组之间切换（与提交视图一起）。
    FilesLayout,
    /// 标题栏最左边的按钮：显示、隐藏分支面板。
    BranchesToggle,
    /// 标题栏右边的「⋯」：选择显示哪几列。
    Columns,
    /// 作者一列的左边缘：左右拖动改宽度，双击恢复默认宽度。
    AuthorEdge,
    /// 分支面板的第几行（[`App::log_branch_rows`] 的下标）。
    BranchRow(usize),
    /// 标题栏的「清除筛选」。
    ClearFilters,
    /// 列表与详情之间的分隔线：左右拖动改变详情一栏的宽度，双击恢复默认宽度。
    Divider,
    /// 提交图中第 `.0` 行的一段线，属于编号为 `.1` 的线（`GraphRow::line`）。
    ///
    /// 悬停时高亮该线并提示其连接的提交，点击时沿该线跳转到该提交（`App::log_line_target`）。
    Line(usize, u32),
    /// 列表最上方的「未提交的更改」，点击时打开提交视图。
    Uncommitted,
}

/// 详情一栏中绘制的、可选中复制的一行文字（说明的一行、作者、邮箱、时间）。
///
/// 绘制时记录，用于鼠标选择文字和复制。
#[derive(Debug, Clone)]
pub(crate) struct TextRun {
    /// 该行占用的整行矩形；选中的底色按其高度绘制。
    pub row: Rect,
    /// 第一个字符的左边界（逻辑像素）。
    pub x: f32,
    /// 绘制的文字。
    pub text: String,
    /// 每个字符的右边界（逻辑像素），与 `text` 的字符一一对应，用于按位置查找字符。
    pub edges: Vec<f32>,
    /// 绘制的文字被截短时（邮箱）的完整内容；整行选中复制时使用。
    pub full: Option<String>,
    /// 是否为上一行自动折行后的续行；复制时与上一行直接连接，不加换行符。
    pub soft: bool,
}

impl TextRun {
    /// 返回横坐标 `x` 之后的第一个字符的序号（0 到字符数）；越过字符宽度的一半即算在其后。
    fn index_at(&self, x: f32) -> usize {
        let mut left = self.x;
        for (i, &right) in self.edges.iter().enumerate() {
            if x < (left + right) / 2.0 {
                return i;
            }
            left = right;
        }
        self.edges.len()
    }
}

/// 详情中文字选区的一端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TextPoint {
    /// 所在行（`LogState::runs` 的下标）。
    pub run: usize,
    /// 行内位置：位于第几个字符之前（从 0 起）。
    pub ch: usize,
}

/// 日志中需要确认后才执行的提交操作。
///
/// 确认框由系统绘制，确认后经 `UserEvent` 返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogConfirmed {
    /// 签出提交本身，进入分离的 `HEAD`。
    ///
    /// 参数是完整的提交号。
    Detach(String),
    /// 签出一个标签指向的提交，进入分离的 `HEAD`；参数是标签名。
    CheckoutTag(String),
    /// 将当前分支硬重置到该提交。
    ResetHard(String),
    /// 删除一条储藏；执行时按提交号查找（[`winc_editor::git::Git::stash`]）。
    ///
    /// 使用 `Box`：储藏包含父提交、时间和说明，直接内联会增大发往事件循环的每个事件的大小。
    DropStash(Box<winc_editor::git::Stash>),
    /// 删除一个本地分支；第二项为真时连同未合并的提交一并删除（[`App::run_delete_branch`]）。
    DeleteBranch(String, bool),
    /// 删除一个标签。
    DeleteTag(String, winc_editor::git::TagTarget),
    /// 移除一个远程仓库。
    RemoveRemote(String),
}

/// 一个提交上的本地分支；签出时优先签出该分支，不进入分离的 `HEAD`。
fn local_branch(commit: &Commit) -> Option<String> {
    commit.refs.iter().find_map(|r| match r {
        Ref::Head(name) | Ref::Branch(name) => Some(name.clone()),
        _ => None,
    })
}

/// 返回文件历史标签的标题。
pub(crate) fn history_title(path: &Path) -> String {
    t!("git-history-title", name = winc_editor::display_name(path))
}

/// 后台返回的结果。
enum Loaded {
    /// 「转到提交号、分支或标签」的解析结果：完整的提交号，以及它是否在不筛选的日志中。
    GoTo {
        revision: String,
        result: Result<(String, bool), String>,
    },
    /// 一批提交，以及用于继续读取的 `git log`（未出错时）。
    Commits {
        doc: DocumentId,
        /// 发出请求时日志的版本号（[`winc_editor::git::log::LogView`]）；不一致时丢弃。
        generation: u64,
        result: Result<Vec<Commit>, String>,
        stream: Option<LogStream>,
    },
    /// 一个提交的详情。
    Details {
        doc: DocumentId,
        oid: String,
        message: Result<String, String>,
        files: Result<Vec<CommitFile>, String>,
        /// 改动的文件对照的版本（见 [`Details::base`]）。
        base: Option<String>,
    },
    /// 一个文件在某个版本里与工作区里的并排差异，打开为新标签。
    LocalDiff {
        title: String,
        view: Result<Box<DiffView>, String>,
    },
    /// 一个提交的补丁（[`Git::commit_patch`]），复制到剪贴板。
    Patch {
        oid: String,
        result: Result<String, String>,
    },
    /// 包含一个提交的分支（[`Git::containing_branches`]）。
    Branches {
        doc: DocumentId,
        oid: String,
        result: Result<Vec<String>, String>,
    },
    /// 比较两个版本时两者之间改动的文件。
    Compare {
        doc: DocumentId,
        from: String,
        to: String,
        files: Result<Vec<CommitFile>, String>,
    },
    /// 一个文件在一次提交中的差异，打开为标签。
    Diff {
        /// 发起读取时的请求序号，与 `LogState::diff_request` 不同时结果已过时。
        request: u64,
        /// 文件所属的提交或比较。
        source: Arc<LogDiffSource>,
        /// 文件在 `source.files` 中的下标。
        file: usize,
        title: String,
        view: Result<Box<DiffView>, String>,
    },
}

/// 从日志打开的差异所属的提交或比较。
#[derive(PartialEq, Eq)]
struct LogDiffSource {
    /// 所在仓库的根目录。
    root: PathBuf,
    /// 旧的一侧的提交；根提交没有父提交，为 `None`。
    old: Option<String>,
    /// 新的一侧的提交。
    new: String,
    /// 标题中显示的版本：提交号，或「旧…新」两个提交号。
    version: String,
    /// 改动的文件。
    files: Vec<CommitFile>,
}

/// 日志标签的状态：与后台之间的通道和点击区域；标签的数据存放在 `Document::log` 中。
pub(crate) struct LogState {
    results: (Sender<Loaded>, Receiver<Loaded>),
    /// 标题栏的筛选输入框（按说明查找）。
    ///
    /// 所有日志标签共用一个输入框，切换标签时换为该标签的筛选文字。
    pub filter: TextField,
    /// 作者菜单中列出的作者，弹出菜单时从已读取的提交中收集；菜单项按下标对应。
    pub authors: Vec<String>,
    /// `authors` 是哪个日志标签的：按作者筛选时只沿用同一个标签上次的名单。
    pub(super) authors_of: Option<DocumentId>,
    /// 路径菜单中列出的路径（相对仓库根目录），弹出菜单时收集；菜单项按下标对应。
    pub paths: Vec<String>,
    /// 打开文件差异的标签；依次查看多个文件时只占用一个标签。
    preview: Option<DocumentId>,
    /// 当前预览对应的提交及文件列表，独立于日志中后来选中的提交。
    tree: Option<LogDiffTree>,
    /// 文件列表的浏览状态（见 [`App::with_log_list`]）。
    list: crate::app::git::commit::CommitState,
    /// 文件列表对应的仓库状态：该提交改动的文件，下标与 `tree` 中的文件相同。
    status: Option<winc_editor::git::Status>,
    /// 只接收最后一次打开文件的后台结果。
    diff_request: u64,
    /// 提交视图与文件列表之间的过渡（见 [`App::track_log_tree`]）：上一帧是否显示文件列表、
    /// 侧栏上一帧是否显示提交视图，以及过渡的时钟。
    header: (bool, bool, Option<crate::app::Clock>),
    /// 键盘焦点位于详情文件列表时选中的文件下标（`→` 进入，`←` / `Esc` 退出）。
    ///
    /// 焦点位于提交列表时为 `None`。
    pub file: Option<usize>,
    /// 双击时改动文件尚未读取完成的提交；读取完成后打开其第一个文件。
    open_first: Option<String>,
    /// 要选中但尚未读取到的提交：日志标签和提交号（由逐行追溯的点击设置）。
    ///
    /// 提交可能位于很靠后的批次；读取到之前继续读取下一批，读取到后选中，全部读取完仍未找到
    /// 时按普通的第一批处理。用户改选其他提交后不再查找。
    seek: Option<(DocumentId, String)>,
    /// 悬停行中延伸到列表右边界仍显示不全的说明；悬停片刻后提示完整说明。
    ///
    /// 绘制时更新。
    pub clipped: Option<usize>,
    /// 详情中目录显示不全或已重命名的文件（文件列表的下标）；悬停时提示完整路径。
    ///
    /// 绘制时更新。
    pub long_files: Vec<usize>,
    /// 拖动分隔线后详情一栏的宽度（逻辑像素）；`None` 表示默认宽度，随窗口宽度变化。
    pub details_w: Option<f32>,
    /// 拖动左边缘后作者一列的宽度（逻辑像素）；`None` 是默认宽度 [`AUTHOR_DEFAULT`]。
    pub author_w: Option<f32>,
    /// 分支面板里名字截短了的行（[`App::log_branch_rows`] 的下标）；悬停时提示全名。
    ///
    /// 绘制时更新。
    pub long_branches: Vec<usize>,
    /// 分支面板滚动的距离（逻辑像素）。
    pub branches_scroll: f32,
    /// 分支面板的最大滚动距离（逻辑像素），绘制时按行数计算。
    pub branches_max: f32,
    /// 分支面板里正在展开、收起的组或目录。
    pub(crate) branch_fold: Option<PanelFold>,
    /// 标题栏各个筛选下拉的宽度过渡：换了筛选条件、文字变长变短时宽度缓动，旧文字淡出、新文字
    /// 淡入，与状态栏右侧各项相同。
    pub(crate) pills: crate::app::StatusMotion<LogPill>,
    /// 分支面板各行的缓存（见 [`App::log_branch_rows`]）：排它们用的分支、最近的分支、收起的
    /// 组和目录、标签，以及排好的各行。
    #[allow(clippy::type_complexity, reason = "缓存键就是这几项，单独起名反而难懂")]
    pub(super) branch_rows: std::cell::RefCell<
        Option<(
            (
                Vec<winc_editor::git::Branch>,
                Vec<String>,
                HashSet<(bool, String)>,
                Vec<String>,
            ),
            Vec<BranchRow>,
        )>,
    >,
    /// 分支面板打开、关闭的过渡：开始时露出的比例与时钟。
    pub(in crate::app) branches_motion: Option<(f32, crate::app::Clock)>,
    /// 显示「正在读取…」的时刻；选中提交的详情（或比较的文件）正在后台读取时设置。
    ///
    /// 详情通常在数十毫秒内返回，立即显示提示再替换为文件列表会产生一次闪烁。
    pub waiting: Option<Instant>,
    /// 显示「正在读取提交…」「正在刷新…」的时刻；从头读取提交列表时设置。
    ///
    /// 在此之前不显示提示，避免刚发出的请求先显示提示随即被结果替换。
    pub list_waiting: Option<Instant>,
    /// 点击了「刷新」的日志标签；第一批提交返回时提示已刷新，没有新提交时列表不变，需要提示。
    pub refreshing: Option<DocumentId>,
    /// 跳转前选中的提交（沿提交图的线或点击父提交跳转后，「后退」回到这些提交），新的在后。
    pub back: Vec<String>,
    /// 后退后可以再前进到的提交，新的在后；发生新的跳转时清空。
    pub forward: Vec<String>,
    /// 详情中可选中复制的文字（绘制时更新）。
    ///
    /// 选中的提交变化或点击其他区域时清空。
    pub runs: Vec<TextRun>,
    /// 详情中的文字选区（按下的一端，拖动到的一端）。
    pub text_sel: Option<(TextPoint, TextPoint)>,
    /// 详情一栏当前绘制的提交（`view::log::shown_oid`）及切换到另一个提交时内容淡入的时钟。
    pub shown: Option<String>,
    /// 详情内容淡入的时钟；不在淡入途中时为 `None`。
    pub fade: Option<crate::app::animation::Clock>,
    /// 详情一栏的最大滚动距离（逻辑像素），绘制时按内容高度计算。
    pub details_max: f32,
    /// 正在后台读取详情的提交（选中的与预取的），同一提交不重复读取。
    fetching: HashSet<String>,
    /// 正在后台查找包含它的分支的提交。
    branches_fetching: HashSet<String>,
    /// 没查出包含它的分支的提交（超时、出错）：详情里不画「所在分支」。
    pub(crate) branches_failed: HashSet<String>,
    /// 选中停下以后才去查所在分支：日志标签和开始查的时刻（见 [`App::log_tick`]）。
    branches_due: Option<(DocumentId, Instant)>,
    /// 各日志标签打开的 `git log`（[`Git::log_stream`]）及打开时的重新读取次数。
    ///
    /// 下一批从该流继续读取。
    ///
    /// 读取期间由后台任务持有，返回后再放回此处；重新读取、读取完毕或关闭标签时移除，
    /// `git` 进程随之结束。
    streams: HashMap<DocumentId, (u64, LogStream)>,
    /// 详情栏中已折行的提交说明，绘制时填入（见 [`Wrapped`]）。
    pub(crate) wrapped: std::cell::RefCell<Option<std::rc::Rc<Wrapped>>>,
}

/// 详情栏中已折行的提交说明：各行在原文中的字节范围。
///
/// 测量宽度需要逐段进行，说明较长时每帧重新折行开销较大，
/// 因此按说明、宽度和字号缓存，滚动和悬停时复用。
pub(crate) struct Wrapped {
    /// 缓存键：说明、宽度、正文字号与小字字号（后三者取 `f32::to_bits`）。
    pub key: (String, u32, u32, u32),
    /// 第一行（去除首尾空白）折成的各行。
    pub subject: Vec<std::ops::Range<usize>>,
    /// 其余部分（去除首尾空白）每一段折成的各行。
    pub body: Vec<Vec<std::ops::Range<usize>>>,
}

impl LogState {
    /// 在日志标签 `id` 关闭后结束其打开的 `git log`，并放弃在该标签中查找的提交。
    ///
    /// 后台正在读取的流返回时文档已不存在，随结果一起丢弃。
    pub(super) fn forget(&mut self, id: DocumentId) {
        self.streams.remove(&id);
        if self.seek.as_ref().is_some_and(|(doc, _)| *doc == id) {
            self.seek = None;
        }
        if self.preview == Some(id) {
            self.preview = None;
            self.tree = None;
            self.status = None;
            self.diff_request += 1;
        }
    }
}

impl Default for LogState {
    fn default() -> Self {
        LogState {
            results: channel(),
            filter: TextField::default(),
            authors: Vec::new(),
            authors_of: None,
            paths: Vec::new(),
            preview: None,
            tree: None,
            list: crate::app::git::commit::CommitState::browsing(&Default::default()),
            status: None,
            diff_request: 0,
            header: (false, false, None),
            file: None,
            open_first: None,
            seek: None,
            clipped: None,
            long_files: Vec::new(),
            details_w: None,
            author_w: None,
            long_branches: Vec::new(),
            branches_scroll: 0.0,
            branches_max: 0.0,
            branch_fold: None,
            branches_motion: None,
            branch_rows: Default::default(),
            pills: Default::default(),
            waiting: None,
            list_waiting: None,
            refreshing: None,
            back: Vec::new(),
            forward: Vec::new(),
            runs: Vec::new(),
            text_sel: None,
            shown: None,
            fade: None,
            details_max: 0.0,
            fetching: HashSet::new(),
            branches_fetching: HashSet::new(),
            branches_failed: HashSet::new(),
            branches_due: None,
            streams: HashMap::new(),
            wrapped: Default::default(),
        }
    }
}

impl App {
    /// 打开日志标签（命令「Git：显示日志」）；已打开时切换到该标签。
    pub(in crate::app) fn git_show_log(&mut self) {
        if self.git.root.is_none() {
            self.flash(t!("git-not-a-repository"));
            return;
        }
        let (index, new) = self.editor.show_log(log_title());
        let id = self.editor.documents[index].id;
        if new {
            // 新打开的日志默认只显示当前分支的提交；分离 `HEAD` 时显示全部分支。
            let current = self.git.branches.iter().find(|b| b.current);
            let branches = current.map(|b| b.name.clone()).into_iter().collect();
            if let Some(view) = self.editor.documents[index].log.as_deref_mut() {
                view.query.branches = branches;
            }
            self.load_log(id);
        }
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 打开日志标签并选中提交 `oid`（由逐行追溯的点击触发）。
    ///
    /// 该提交尚未读取时（被筛选掉或位于后续批次），清除筛选并从头读取，逐批读取直至读到该提交
    /// 后选中（见 `LogState::seek`）。
    pub(super) fn git_show_log_at(&mut self, oid: &str) {
        self.show_log_commit(oid, false);
    }

    /// 打开日志标签并选中提交 `oid`（完整的提交号）：已列出时直接选中，否则清除筛选条件从头
    /// 读取，读取到后选中。
    ///
    /// `pin` 为真时把该提交固定在列表最前面（见 `LogQuery::pin`），用于任何分支都走不到、
    /// 因而不在日志中的提交。
    fn show_log_commit(&mut self, oid: &str, pin: bool) {
        if self.git.root.is_none() {
            return;
        }
        let (index, new) = self.editor.show_log(log_title());
        let id = self.editor.documents[index].id;
        self.focus = Focus::Editor;
        self.clear_log_compare();
        let row = self.editor.documents[index]
            .log
            .as_deref()
            .filter(|view| !view.stale)
            .and_then(|v| v.commits.iter().position(|c| c.oid == oid));
        match row {
            Some(row) if !new => self.select_log_row(row),
            _ => {
                if let Some(view) = self.editor.documents[index].log.as_deref_mut() {
                    view.query = LogQuery {
                        pin: pin.then(|| oid.to_owned()),
                        ..LogQuery::default()
                    };
                    view.selected = None;
                    view.reset();
                    view.details = Some(Details {
                        oid: oid.to_owned(),
                        ..Details::default()
                    });
                }
                self.git.log.filter.set_text("");
                self.git.log.seek = Some((id, oid.to_owned()));
                set_scroll_line(&mut self.editor.documents[index], 0.0);
                self.load_log(id);
            }
        }
        self.relayout();
        self.request_redraw();
    }

    /// 打开「转到提交号、分支或标签」对话框。
    pub(in crate::app) fn open_go_to_revision(&mut self) {
        if self.git.root.is_none() {
            self.flash(t!("git-not-a-repository"));
            return;
        }
        use crate::app::git::form::FormKind;
        self.show_form_dialog(
            FormKind::GoToRevision,
            TextField::default(),
            crate::layout::DialogPart::Branch,
        );
    }

    /// 在日志中转到 `revision`（提交号、分支名或标签名）：在后台解析成提交，之后选中它（见
    /// [`Self::show_log_commit`]）；任何分支都走不到的提交固定在最前面。无法解析时提示原因。
    pub(in crate::app) fn go_to_log_revision(&mut self, revision: String) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let result = tool.map_err(|e| unusable(&e)).and_then(|git| {
                let cancel = Cancel::default();
                let oid = git.resolve_revision(&root, &revision, &cancel)?;
                let listed = git.in_log(&root, &oid, &cancel).unwrap_or(false);
                Ok((oid, listed))
            });
            let _ = sender.send(Loaded::GoTo { revision, result });
        });
    }

    /// 当前标签是否为日志。
    pub(crate) fn log_active(&self) -> bool {
        self.editor.active().is_some_and(|d| d.log.is_some())
    }

    /// 在后台读取文档 `id` 的下一批提交。
    ///
    /// 正在读取或已全部读取时不做任何操作。
    pub(crate) fn load_log(&mut self, id: DocumentId) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let Some(view) = self
            .editor
            .document_mut(id)
            .and_then(|d| d.log.as_deref_mut())
        else {
            return;
        };
        if view.complete || view.is_loading() {
            return;
        }
        // 从头读取（刚打开、刷新、修改筛选）时，`LOADING_DELAY` 内不显示「正在读取」「正在刷新」。
        if view.stale || view.commits.is_empty() {
            self.git.log.list_waiting = Some(Instant::now() + LOADING_DELAY);
        }
        // 后台仍有重新读取之前发出的请求在运行时暂不发出，待其返回后再读取（同一时刻只运行一个）。
        let Some(request) = view.begin() else {
            return;
        };
        let query = view.query.clone();
        let sender = self.git.log.results.0.clone();
        // 从上一批的流继续读取；从头读取（`skip` 为 0）或已重新读取时新建流，旧的流随之结束。
        let held = self
            .git
            .log
            .streams
            .remove(&id)
            .filter(|(generation, _)| request.skip > 0 && *generation == request.generation)
            .map(|(_, stream)| stream);
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let read = |git: Git| {
                let mut stream = match held {
                    Some(stream) => stream,
                    None => {
                        // 继续读取的流通常总会保留；缺失时新建流并跳过已读取的部分。
                        let mut stream = git.log_stream(&root, &query)?;
                        if request.skip > 0 {
                            stream.next(request.skip)?;
                        }
                        stream
                    }
                };
                Ok((stream.next(request.max)?, stream))
            };
            let (result, stream) = match tool.map_err(|e| unusable(&e)).and_then(read) {
                Ok((commits, stream)) => (Ok(commits), Some(stream)),
                Err(e) => (Err(e), None),
            };
            let _ = sender.send(Loaded::Commits {
                doc: id,
                generation: request.generation,
                result,
                stream,
            });
        });
    }

    /// 从头重新读取文档 `id` 的日志（`HEAD` 变化或点击「刷新」时）。
    pub(crate) fn reload_log(&mut self, id: DocumentId) {
        if let Some(view) = self
            .editor
            .document_mut(id)
            .and_then(|d| d.log.as_deref_mut())
        {
            view.reset();
            self.load_log(id);
        }
    }

    /// 在 `HEAD` 或分支变化后重新读取所有已打开的日志标签。
    pub(super) fn reload_logs(&mut self) {
        let ids: Vec<DocumentId> = self
            .editor
            .documents
            .iter()
            .filter(|d| d.log.is_some())
            .map(|d| d.id)
            .collect();
        for id in ids {
            self.reload_log(id);
        }
    }

    /// 绘制时可见区域接近已读取的最后一个提交时，继续读取下一批。
    pub(crate) fn log_near_end(&mut self, id: DocumentId, last_visible: usize) {
        let near = self
            .editor
            .document(id)
            .and_then(|d| d.log.as_deref())
            .is_some_and(|v| !v.complete && !v.stale && last_visible + PREFETCH >= v.commits.len());
        if near {
            self.load_log(id);
        }
    }

    /// 返回日志列表最上方「未提交的更改」一行显示的改动数；不显示该行时为 0。
    ///
    /// 工作区有改动时在 `HEAD` 上方增加一行，使未提交的改动在历史中有明确的位置；
    /// 筛选或查看文件历史时不显示。
    pub(crate) fn log_uncommitted(&self, view: &winc_editor::git::log::LogView) -> usize {
        if view.query.filtered() || view.query.path.is_some() {
            return 0;
        }
        self.git.status.as_ref().map_or(0, |s| s.entries.len())
    }

    /// 返回当前日志标签中第一个提交的上沿到列表顶部的距离（不含滚动）。
    ///
    /// 距离为 [`LOG_LIST_PAD`]，显示「未提交的更改」一行时再加一行的高度。
    pub(crate) fn log_list_top(&self) -> f32 {
        let extra = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref())
            .is_some_and(|v| self.log_uncommitted(v) > 0);
        LOG_LIST_PAD + if extra { self.fonts.line_height } else { 0.0 }
    }

    /// 选中当前日志标签中的第 `row` 个提交，使其在列表中可见，并在后台读取其详情。
    pub(crate) fn select_log_row(&mut self, row: usize) {
        let visible = ((self.log_parts().list.h - self.log_list_top()) / self.fonts.line_height)
            .floor()
            .max(1.0) as f64;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let id = doc.id;
        let Some(view) = doc.log.as_deref_mut() else {
            return;
        };
        let Some(commit) = view.commits.get(row) else {
            return;
        };
        let (oid, parent) = (commit.oid.clone(), commit.parents.first().cloned());
        self.git.log.file = None;
        view.selected = Some(row);
        // 已选中的文字属于上一个提交的详情，因此清除。
        self.git.log.text_sel = None;
        let same = view.details.as_ref().is_some_and(|d| d.oid == oid);
        if !same {
            view.details_scroll = 0.0;
        }
        if view
            .details
            .as_ref()
            .is_none_or(|d| d.oid != oid || d.files.is_none())
        {
            match view.cached(&oid) {
                Some(cached) => view.details = Some(cached.clone()),
                None => {
                    if let Some(old) = view.details.take().filter(|d| d.files.is_some()) {
                        view.previous = Some(old);
                    }
                    view.details = Some(Details {
                        oid: oid.clone(),
                        ..Details::default()
                    });
                    self.git.log.waiting = Some(Instant::now() + LOADING_DELAY);
                    self.fetch_log_details(id, oid, parent);
                }
            }
        }
        self.prefetch_log_neighbours(id, row);
        self.schedule_log_branches(id);
        // 使选中行可见：位于屏幕外时滚动到距顶部三行处，紧邻边缘时滚动一行。
        let row = row as f64;
        if let Some(doc) = self.editor.active_mut() {
            if row < doc.scroll_line {
                set_scroll_line(doc, row);
            } else if row + 1.0 > doc.scroll_line + visible {
                set_scroll_line(doc, row + 1.0 - visible);
            }
        }
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 预取第 `row` 个提交前后各两个提交的详情（缓存中没有且未在读取的）。
    ///
    /// 键盘上下移动到这些提交时详情即可立即显示，不会先出现空白。
    fn prefetch_log_neighbours(&mut self, id: DocumentId, row: usize) {
        let Some(view) = self.editor.document(id).and_then(|d| d.log.as_deref()) else {
            return;
        };
        let wanted: Vec<(String, Option<String>)> = [row.checked_sub(1), Some(row + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| view.commits.get(i))
            .filter(|c| view.cached(&c.oid).is_none() && !self.git.log.fetching.contains(&c.oid))
            .map(|c| (c.oid.clone(), c.parents.first().cloned()))
            .collect();
        for (oid, parent) in wanted {
            self.fetch_log_details(id, oid, parent);
        }
    }

    fn fetch_log_details(&mut self, id: DocumentId, oid: String, parent: Option<String>) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        if !self.git.log.fetching.insert(oid.clone()) {
            return;
        }
        // 按路径筛选时列表上的父提交是改写过的：改动的文件要和真正的第一个父提交比。
        let view = self.editor.document(id).and_then(|d| d.log.as_deref());
        let rewritten = view.is_some_and(|v| v.query.rewrites_parents());
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let cancel = Cancel::default();
            let (message, files, base) = match tool {
                Ok(git) => {
                    let base = match rewritten {
                        true => git
                            .resolve_revision(&root, &format!("{oid}^1"), &cancel)
                            .ok(),
                        false => parent,
                    };
                    (
                        git.commit_message(&root, &oid, &cancel),
                        git.commit_files(&root, base.as_deref(), &oid, &cancel),
                        base,
                    )
                }
                Err(e) => (Err(unusable(&e)), Err(String::new()), None),
            };
            let _ = sender.send(Loaded::Details {
                doc: id,
                oid,
                message,
                files,
                base,
            });
        });
    }

    /// 处理对日志标签中 `target` 的点击。
    pub(in crate::app) fn press_log(&mut self, target: LogTarget) {
        match target {
            LogTarget::Uncommitted => self.show_commit(),
            LogTarget::Row(row) | LogTarget::Avatar(row) => {
                self.focus = Focus::Editor;
                // `⌘` 点击另一个提交时，比较选中的提交与被点击的提交。
                let selected = self
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref()?.selected);
                match selected {
                    Some(at) if self.command_held() && at != row => {
                        self.compare_log_rows(at, row);
                    }
                    _ => {
                        self.clear_log_compare();
                        self.select_log_row(row);
                    }
                }
            }
            LogTarget::File(i) => self.open_log_file(i),
            // 拖动由 `mouse.rs` 开始处理。
            LogTarget::Divider => {}
            LogTarget::Line(row, line) => {
                self.focus = Focus::Editor;
                match self.log_line_target(row, line) {
                    Some(target) => {
                        self.clear_log_compare();
                        self.jump_log_row(target);
                    }
                    None => self.flash(t!("git-log-target-not-loaded")),
                }
            }
            LogTarget::Parent(i) => {
                let row = self.editor.active().and_then(|d| {
                    let view = d.log.as_deref()?;
                    let parent = view.commits.get(view.selected?)?.parents.get(i)?;
                    view.commits.iter().position(|c| &c.oid == parent)
                });
                match row {
                    Some(row) => self.jump_log_row(row),
                    None => self.flash(t!("git-log-parent-not-loaded")),
                }
            }
            LogTarget::CopyOid => {
                let oid = self
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref()?.selected_oid().map(str::to_owned));
                if let Some(oid) = oid {
                    self.flash(t!("git-log-copied-oid", commit = short(&oid)));
                    self.set_clipboard(oid, false);
                }
            }
            LogTarget::Refresh => {
                if let Some(id) = self.editor.active().map(|d| d.id) {
                    self.git.log.refreshing = Some(id);
                    self.reload_log(id);
                }
                // 列表上方的未提交更改和分支筛选来自仓库状态，因此同时刷新仓库状态。
                self.git_refresh();
            }
            LogTarget::Search => {
                self.focus = Focus::LogFilter;
                let (x, y) = self.pointer.mouse;
                self.click_field(x, y);
            }
            LogTarget::Branch => self.open_branch_picker(crate::app::GitAnchor::Log),
            LogTarget::Author => {
                let Some(rect) = self
                    .hits
                    .log
                    .iter()
                    .find(|(_, t)| *t == target)
                    .map(|(r, _)| *r)
                else {
                    return;
                };
                self.git.log.authors = self.log_authors();
                self.git.log.authors_of = self.editor.active().map(|d| d.id);
                self.open_context_menu(MenuTarget::LogAuthor, (rect.x, rect.bottom() + 4.0));
            }
            LogTarget::FilesLayout => self.toggle_commit_layout(),
            LogTarget::BranchesToggle => self.toggle_log_branches(),
            // 拖动由 `mouse.rs` 开始处理。
            LogTarget::AuthorEdge => {}
            LogTarget::Columns => {
                let hit = self.hits.log.iter().find(|(_, t)| *t == target);
                if let Some(&(rect, _)) = hit {
                    let at = (rect.right(), rect.bottom() + 4.0);
                    self.open_context_menu(MenuTarget::LogColumns, at);
                }
            }
            LogTarget::BranchRow(i) => self.press_log_branch_row(i),
            LogTarget::Date | LogTarget::Path => {
                let hit = self.hits.log.iter().find(|(_, t)| *t == target);
                let Some(rect) = hit.map(|(r, _)| *r) else {
                    return;
                };
                let menu = match target {
                    LogTarget::Date => MenuTarget::LogDate,
                    _ => {
                        self.git.log.paths = self.log_path_choices();
                        MenuTarget::LogPath
                    }
                };
                self.open_context_menu(menu, (rect.x, rect.bottom() + 4.0));
            }
            LogTarget::ClearFilters => {
                self.git.log.filter.set_text("");
                self.change_log_query(|q| {
                    q.text.clear();
                    q.branches.clear();
                    q.authors.clear();
                    q.pin = None;
                    q.since_days = None;
                    q.paths.clear();
                });
            }
        }
        self.request_redraw();
    }

    /// 打开选中提交中第 `i` 个改动文件的差异：左侧为第一个父提交中的版本，右侧为该提交中的版本。
    fn open_log_file(&mut self, i: usize) {
        if let Some(log) = self.editor.active().filter(|d| d.log.is_some()) {
            self.open_log_file_of(log.id, i);
        }
    }

    /// 打开日志标签 `log` 中选中提交（或比较）的第 `i` 个改动文件的差异。
    fn open_log_file_of(&mut self, log: DocumentId, i: usize) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        // 比较两个版本时左侧为旧版本、右侧为新版本；否则左侧为第一个父提交（根提交没有父提交）。
        let Some((old, new, version, files)) = self.editor.document(log).and_then(|d| {
            let view = d.log.as_deref()?;
            if let Some(compare) = &view.compare {
                let version = format!("{}…{}", short(&compare.from), short(&compare.to));
                return Some((
                    Some(compare.from.clone()),
                    compare.to.clone(),
                    version,
                    compare.files.as_ref()?,
                ));
            }
            let commit = view.commits.get(view.selected?)?;
            // 和详情里的文件同一个对照版本（按路径筛选时是真正的父提交）。
            let details = view.details.as_ref()?;
            Some((
                details.base.clone(),
                commit.oid.clone(),
                short(&commit.oid).to_owned(),
                details.files.as_ref()?,
            ))
        }) else {
            return;
        };
        self.diff.land = None;
        self.load_log_diff(
            Arc::new(LogDiffSource {
                root,
                old,
                new,
                version,
                files: files.clone(),
            }),
            i,
        );
    }

    fn load_log_diff(&mut self, source: Arc<LogDiffSource>, i: usize) {
        let Some(file) = source.files.get(i) else {
            return;
        };
        let name = winc_editor::display_name(&file.path);
        let title = format!("{name} · {}", source.version);
        self.git.log.diff_request += 1;
        let request = self.git.log.diff_request;
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let view = match tool {
                Ok(git) => commit_diff(
                    &git,
                    &source.root,
                    source.old.as_deref(),
                    &source.new,
                    &source.files[i],
                )
                .map(Box::new),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Loaded::Diff {
                request,
                source,
                file: i,
                title,
                view,
            });
        });
    }

    /// 标签 `id` 是否为当前仓库从日志打开的差异。
    pub(in crate::app) fn steps_log_files(&self, id: DocumentId) -> bool {
        self.git.log.preview == Some(id)
            && self
                .git
                .log
                .tree
                .as_ref()
                .is_some_and(|tree| self.git.root.as_ref() == Some(&tree.source.root))
    }

    /// 返回按文件树显示顺序排列时，同一提交中的下一个（`forward`）或上一个文件。
    ///
    /// 返回（下标，在显示顺序中的位置（从 0 起），文件名，文件总数）；当前标签不是从日志打开的
    /// 差异，或没有更多文件时返回 `None`。
    pub(in crate::app) fn next_log_file(
        &self,
        forward: bool,
    ) -> Option<(usize, usize, String, usize)> {
        let id = self.editor.active()?.id;
        let tree = self
            .git
            .log
            .tree
            .as_ref()
            .filter(|_| self.steps_log_files(id))?;
        let files = &tree.source.files;
        let (next, at) = self.log_diff_neighbor(forward)?;
        let name = winc_editor::display_name(&files.get(next)?.path).into_owned();
        Some((next, at, name, files.len()))
    }

    /// 在同一个差异标签里打开同一提交的上一个或下一个文件。
    pub(in crate::app) fn step_log_file(&mut self, forward: bool) -> bool {
        let Some((next, at, name, total)) = self.next_log_file(forward) else {
            return false;
        };
        self.open_log_diff_file(next);
        let (index, name) = (at + 1, name.as_str());
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

    /// 返回当前日志标签的几何。
    pub(crate) fn log_parts(&self) -> LogParts {
        log_parts(
            self.layout.editor,
            self.git.log.details_w,
            self.log_branches_shown(),
        )
    }

    /// 等待详情超过 `LOADING_DELAY` 仍未返回时重绘，显示「正在读取…」；选中停下
    /// [`branches::BRANCHES_DELAY`] 以后去查所在分支。
    ///
    /// 返回下一次唤醒的时刻。
    pub(in crate::app) fn log_tick(&mut self, now: Instant) -> Option<Instant> {
        let (mut wake, mut due) = (None, false);
        match self.git.log.branches_due {
            Some((_, at)) if now < at => wake = Some(at),
            Some((id, _)) => {
                self.git.log.branches_due = None;
                self.fetch_log_branches(id);
            }
            None => {}
        }
        for slot in [&mut self.git.log.waiting, &mut self.git.log.list_waiting] {
            match *slot {
                Some(at) if now < at => wake = Some(wake.map_or(at, |w: Instant| w.min(at))),
                Some(_) => {
                    *slot = None;
                    due = true;
                }
                None => {}
            }
        }
        if due {
            self.request_redraw();
        }
        wake
    }

    /// 详情一栏是否暂不显示「正在读取…」「正在比较…」（刚开始读取，结果通常很快返回）。
    pub(crate) fn log_quiet(&self) -> bool {
        self.git.log.waiting.is_some_and(|at| Instant::now() < at)
    }

    /// 提交列表是否暂不显示「正在读取提交…」，标题栏是否暂不显示「正在刷新…」。
    pub(crate) fn log_list_quiet(&self) -> bool {
        self.git
            .log
            .list_waiting
            .is_some_and(|at| Instant::now() < at)
    }

    /// 是否正在拖动列表与详情之间的分隔线。
    pub(crate) fn log_dividing(&self) -> bool {
        matches!(self.pointer.drag, crate::app::Drag::LogDivider { .. })
    }
}

/// 在后台准备一个文件在两个版本之间的差异。
///
/// 左侧为 `old` 中的版本（重命名的文件取旧路径；`None` 表示根提交之前，内容为空），
/// 右侧为 `new` 中的版本。
pub(in crate::app) fn commit_diff(
    git: &Git,
    root: &Path,
    old_rev: Option<&str>,
    new_rev: &str,
    file: &CommitFile,
) -> Result<DiffView, String> {
    let name = winc_editor::display_name(&file.path).into_owned();
    let cancel = Cancel::default();
    let read = |rev: &str, path: &Path| -> Result<Option<String>, String> {
        let bytes = git
            .content_at(root, rev, path, &cancel)
            .map_err(|e| t!("git-read-file-failed", name = name.as_str(), error = e))?;
        match bytes {
            Some(b) if b.len() as u64 > LARGE_FILE => {
                Err(t!("git-diff-too-large", name = name.as_str()))
            }
            Some(b) if looks_binary(&b) => Err(t!("git-diff-binary", name = name.as_str())),
            Some(b) => Ok(Some(decode_unknown(&b).0)),
            None => Ok(None),
        }
    };
    let old_path: PathBuf = file.from.clone().unwrap_or_else(|| file.path.clone());
    let old = match old_rev {
        Some(rev) => read(rev, &old_path)?,
        None => None,
    };
    let new = read(new_rev, &file.path)?;
    let language = winc_syntax::detect(&name);
    let side_title = |oid: Option<&str>, present: bool| match (oid, present) {
        (Some(oid), true) => short(oid).to_owned(),
        (Some(oid), false) => t!("git-diff-side-commit-missing", commit = short(oid)),
        (None, _) => t!("git-diff-side-before-root").to_owned(),
    };
    let mut view = DiffView::new(
        DiffSide::new(
            side_title(old_rev, old.is_some()),
            old.as_deref().unwrap_or(""),
            None,
            language,
        )
        .with_missing(old_rev.is_some() && old.is_none()),
        DiffSide::new(
            side_title(Some(new_rev), new.is_some()),
            new.as_deref().unwrap_or(""),
            None,
            language,
        )
        .with_missing(new.is_none()),
    );
    view.snapshot_path = Some(file.path.clone());
    Ok(view)
}

/// 返回提交号的前 7 位；不是提交号（分支名、标签名）时原样返回，名字里有汉字也不会被截断在
/// 一个字的中间。
pub(crate) fn short(oid: &str) -> &str {
    if oid.len() > 7 && oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        &oid[..7]
    } else {
        oid
    }
}

#[cfg(test)]
pub(crate) mod tests;
