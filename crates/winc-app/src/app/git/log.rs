//! Git 日志标签的交互：打开、分批在后台取提交、选中提交取详情（完整说明、改动的文件）、点文件
//! 打开那次提交的并排差异、键盘。
//!
//! 绘制在 `view/git/log.rs`。日志是一种只读文档（`Document::log`），一个提交一行，滚动走文档的
//! 通用逻辑；左边是提交列表（提交图、说明、作者、时间），右边一栏是选中提交的详情。

mod compare;
mod details;
mod filter;
mod keys;
mod menus;
mod time;

pub(crate) use time::*;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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

/// 日志标签的标题。
pub(crate) fn log_title() -> &'static str {
    t!("git-log-title")
}
/// 标题栏的高度（逻辑像素），取差异视图的标题栏高度，使两类标签的标题栏对齐。
pub(crate) const LOG_HEADER: f32 = 36.0;
/// 详情一栏最窄、最宽（逻辑像素）。
///
/// 300 是经验值：还放得下一次提交的说明和文件名；520 是再宽就挤掉了提交列表。拖过分隔线以后
/// 不受上限限制，只给列表留出 [`LIST_MIN`]。
const DETAILS_MIN: f32 = 300.0;
const DETAILS_MAX: f32 = 520.0;
/// 提交列表至少要留下的宽度（逻辑像素）。
///
/// 360 是经验值：容得下提交图、作者名和时间三栏。
const LIST_MIN: f32 = 360.0;
/// 列表末尾离已取回的最后一个提交还剩多少行（行）时继续取下一批。
///
/// 100 是经验值：约两屏的滚动距离，滚到底之前下一批已经取回来。
const PREFETCH: usize = 100;
/// 详情里文件一行的高度（逻辑像素）。
///
/// 26 是经验值：容得下 24 像素高的图标按钮，上下各留 1 像素。
pub(crate) const LOG_FILE_ROW: f32 = 26.0;
/// 提交列表第一行上面的留白（逻辑像素）：在滚动的内容里，第一行和标题栏的分隔线之间空出来，
/// 滚动时各行仍然滑到标题栏下面，选中的底色也不会顶到分隔线上。
///
/// 值与侧栏里各块之间的间距（`SIDEBAR_GAP`）相同；详情一栏的第一行和它对齐
/// （[`details_top`]）。
pub(crate) const LOG_LIST_PAD: f32 = crate::theme::SIDEBAR_GAP;

/// 详情一栏第一行（高 24 逻辑像素的说明）的上沿离一栏顶上多远。
///
/// 这个偏移量让两栏的第一行字落在同一条线上：左边列表第一行与右边说明垂直居中对齐。
pub(crate) fn details_top(line_height: f32) -> f32 {
    LOG_LIST_PAD + (line_height - 24.0) / 2.0
}
/// 日志里后退最多记住的提交数（个）。
///
/// 100 是经验值：再往前翻的跳转已经很少，留着也不再有用。
const LOG_HISTORY: usize = 100;
/// 详情超过这个时长还没取回来才写「正在读取…」（300 毫秒，经验值）。
const LOADING_DELAY: Duration = Duration::from_millis(300);

/// 日志标签的几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LogParts {
    /// 顶上的标题栏。
    pub header: Rect,
    /// 左边的提交列表。
    pub list: Rect,
    /// 右边的详情一栏。
    pub details: Rect,
}

/// 日志标签里各部分的位置：标题栏下面左边是列表，右边是详情（约占四成宽；拖过分隔线的是
/// `details_w`）。
pub(crate) fn log_parts(editor: Rect, details_w: Option<f32>) -> LogParts {
    let header = Rect::new(editor.x, editor.y, editor.w, LOG_HEADER);
    let body_h = (editor.h - LOG_HEADER).max(0.0);
    // 窗口窄时详情最多占一半；宽时最多给列表留 `LIST_MIN`。
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
        list,
        details,
    }
}

/// 日志标签里可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTarget {
    /// 一个提交（`commits` 的下标）。
    Row(usize),
    /// 一个提交那一行的作者头像：悬停提示作者的名字和邮箱（列表窄、只画头像时看得到是谁），
    /// 点击头像等同于点击该行。
    Avatar(usize),
    /// 详情里改动的一个文件（`files` 的下标）：打开它在这次提交里的差异。
    File(usize),
    /// 详情里的父提交：选中它。
    ///
    /// 参数是第几个父提交。
    Parent(usize),
    /// 详情里的提交号：复制完整的提交号。
    CopyOid,
    /// 标题栏的「刷新」。
    Refresh,
    /// 标题栏的筛选输入框。
    Search,
    /// 标题栏的「分支」下拉：弹分支菜单。
    Branch,
    /// 标题栏的「作者」下拉：弹作者菜单。
    Author,
    /// 标题栏的「清除筛选」。
    ClearFilters,
    /// 列表和详情之间的分隔线：左右拖改详情一栏的宽度，双击回到默认宽度。
    Divider,
    /// 提交图里第 `.0` 行的一段线，属于编号 `.1` 的那条线（`GraphRow::line`）：悬停时高亮这条
    /// 线、提示连到哪个提交，点了沿着它往下跳到那个提交（`App::log_line_target`）。
    Line(usize, u32),
    /// 列表最上方的「未提交的更改」，点击时打开提交视图。
    Uncommitted,
}

/// 详情一栏里画出来、可以选中复制的一行字（说明的一行、作者、邮箱、时间）。
///
/// 绘制时记下，鼠标选字、复制时用。
#[derive(Debug, Clone)]
pub(crate) struct TextRun {
    /// 这一行占的整行矩形（选中的底色按它的高度画）。
    pub row: Rect,
    /// 第一个字的左边（逻辑像素）。
    pub x: f32,
    /// 画出来的字。
    pub text: String,
    /// 每个字的右边界（和 `text` 的字一一对应），按位置找第几个字用。
    pub edges: Vec<f32>,
    /// 画出来的是截短后的文字（邮箱）：整行选中复制时给完整内容。
    pub full: Option<String>,
    /// 是上一行折下来的（同一段话的后半截）：复制时和上一行直接连起来，不加换行。
    pub soft: bool,
}

impl TextRun {
    /// 横坐标 `x` 落在第几个字的前面（0 到字数）：过了一个字的一半就算它后面。
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

/// 详情里选中的字的一头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TextPoint {
    /// 第几行（`LogState::runs` 的下标）。
    pub run: usize,
    /// 这一行里第几个字的前面（从 0 起）。
    pub ch: usize,
}

/// 日志里需要先确认才能执行的提交操作。
///
/// 确认框由系统绘制，确认以后经 `UserEvent` 回来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogConfirmed {
    /// 签出提交本身，进入分离的 `HEAD`。
    ///
    /// 参数是完整的提交号。
    Detach(String),
    /// 把当前分支硬重置到这个提交。
    ResetHard(String),
    /// 删除一条储藏；执行时按提交号查找（[`winc_editor::git::Git::stash`]）。
    DropStash(winc_editor::git::Stash),
    /// 删除一个本地分支；第二项为真时连同未合并的提交一起删除（[`App::run_delete_branch`]）。
    DeleteBranch(String, bool),
}

/// 一个提交上的本地分支：签出时优先签出它，不进入分离的 `HEAD`。
fn local_branch(commit: &Commit) -> Option<String> {
    commit.refs.iter().find_map(|r| match r {
        Ref::Head(name) | Ref::Branch(name) => Some(name.clone()),
        _ => None,
    })
}

/// 文件历史标签的标题。
pub(crate) fn history_title(path: &Path) -> String {
    t!("git-history-title", name = winc_editor::display_name(path))
}

/// 后台交回来的结果。
enum Loaded {
    /// 一批提交，以及继续往下读取的 `git log`（没有出错时）。
    Commits {
        doc: DocumentId,
        /// 发出请求时日志的版本号（[`winc_editor::git::log::LogView`]）：对不上就丢弃。
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
    },
    /// 比较两个版本时两者之间改了的文件。
    Compare {
        doc: DocumentId,
        from: String,
        to: String,
        files: Result<Vec<CommitFile>, String>,
    },
    /// 一个文件在一次提交里的差异，打开成标签。
    Diff {
        /// 在哪个仓库里比较的：之后换了目录就不要了。
        root: PathBuf,
        title: String,
        view: Result<Box<DiffView>, String>,
    },
}

/// 日志标签的状态（标签里的数据在 `Document::log` 里，这里是和后台之间的通道、点击区域）。
pub(crate) struct LogState {
    results: (Sender<Loaded>, Receiver<Loaded>),
    /// 标题栏的筛选输入框（按说明找）。
    ///
    /// 几个日志标签共用一个，切过去时换成那个标签的文字。
    pub filter: TextField,
    /// 作者菜单里列出的作者（弹菜单时从已经取回来的提交里收集），菜单项按下标对应。
    pub authors: Vec<String>,
    /// 可以点的地方，绘制时更新。
    pub hits: Vec<(Rect, LogTarget)>,
    /// 打开文件差异的标签：一个接一个地看几个文件，只占一个标签。
    preview: Option<DocumentId>,
    /// 当前差异标签对应的日志标签及文件下标。
    ///
    /// 在差异中越过最后一处改动时，据此打开同一提交中的下一个文件。
    preview_source: Option<(DocumentId, usize)>,
    /// 键盘在详情的文件列表里时，选着第几个文件（`→` 进去、`←` / `Esc` 出来）；在提交列表里
    /// 时为 `None`。
    pub file: Option<usize>,
    /// 双击了这个提交、它改动的文件还没取回来：取回来以后打开第一个文件。
    open_first: Option<String>,
    /// 悬停的那一行说明铺到列表右边也写不下：悬停片刻后提示整句。
    ///
    /// 绘制时更新。
    pub clipped: Option<usize>,
    /// 详情里目录写不下、改过名的文件（文件列表的下标）：悬停提示完整的路径。
    ///
    /// 绘制时更新。
    pub long_files: Vec<usize>,
    /// 拖过分隔线以后详情一栏的宽度（逻辑像素）；`None` 是默认宽度（随窗口宽窄变）。
    pub details_w: Option<f32>,
    /// 选中提交的详情（或比较的文件）正在后台获取：到这个时刻还没取回来才写「正在读取…」。
    ///
    /// 一般几十毫秒就返回，先显示提示再换成文件列表会闪烁一次。
    pub waiting: Option<Instant>,
    /// 提交列表从头取的时候：到这个时刻还没取回来才写「正在读取提交…」「正在刷新…」。
    ///
    /// 先留一段安静的时间，刚发出的请求不先写提示再换成结果。
    pub list_waiting: Option<Instant>,
    /// 跳转之前选中的提交（顺着提交图的线、点父提交跳过去以后，「后退」回到它们），新的在后面。
    pub back: Vec<String>,
    /// 后退之后可以再前进到的提交，新的在后面；发生新的跳转时清空。
    pub forward: Vec<String>,
    /// 详情里可以选中复制的字（绘制时更新）。
    ///
    /// 换了选中的提交、点了别的区域就清空。
    pub runs: Vec<TextRun>,
    /// 详情里选中的范围（按下的那头、拖到的那头）。
    pub text_sel: Option<(TextPoint, TextPoint)>,
    /// 详情一栏正在画的是哪个提交（`view::log::shown_oid`），和换成另一个提交时内容淡入的时钟。
    pub shown: Option<String>,
    /// 详情内容淡入的时钟；不在淡入途中时为 `None`。
    pub fade: Option<crate::app::animation::Clock>,
    /// 详情一栏最多滚多远（逻辑像素，绘制时按内容高度算）。
    pub details_max: f32,
    /// 后台正在取详情的提交（选着的和预取的），同一个提交不重复取。
    fetching: HashSet<String>,
    /// 各日志标签开着的 `git log`（[`Git::log_stream`]），以及它是在第几次重取时开的：下一批
    /// 继续往下读取。
    ///
    /// 取着的时候在后台任务手里，交回来再放回这里；重取、读完、关闭标签时移除，
    /// `git` 随之结束。
    streams: HashMap<DocumentId, (u64, LogStream)>,
    /// 详情栏里折好行的提交说明，绘制时填（见 [`Wrapped`]）。
    pub(crate) wrapped: std::cell::RefCell<Option<std::rc::Rc<Wrapped>>>,
}

/// 详情栏里折好行的提交说明：各行在原文里的字节范围。
///
/// 量宽度要一段一段量，说明长的时候每帧重折一遍不划算，按说明、宽度、字号记下来，滚动、悬停
/// 时照用。
pub(crate) struct Wrapped {
    /// 缓存键：说明，加上宽度、正文字号、小字字号（后三个取 `f32::to_bits`）。
    pub key: (String, u32, u32, u32),
    /// 第一行（去掉首尾空白）折成的各行。
    pub subject: Vec<std::ops::Range<usize>>,
    /// 其余部分（去掉首尾空白）的每一段折成的各行。
    pub body: Vec<Vec<std::ops::Range<usize>>>,
}

impl LogState {
    /// 日志标签 `id` 关闭了：结束它开着的 `git log`。
    ///
    /// 后台正在读取的流交回来时文档已经不在，随结果一起丢弃。
    pub(super) fn forget(&mut self, id: DocumentId) {
        self.streams.remove(&id);
    }
}

impl Default for LogState {
    fn default() -> Self {
        LogState {
            results: channel(),
            filter: TextField::default(),
            authors: Vec::new(),
            hits: Vec::new(),
            preview: None,
            preview_source: None,
            file: None,
            open_first: None,
            clipped: None,
            long_files: Vec::new(),
            details_w: None,
            waiting: None,
            list_waiting: None,
            back: Vec::new(),
            forward: Vec::new(),
            runs: Vec::new(),
            text_sel: None,
            shown: None,
            fade: None,
            details_max: 0.0,
            fetching: HashSet::new(),
            streams: HashMap::new(),
            wrapped: Default::default(),
        }
    }
}

impl App {
    /// 打开日志标签（命令「Git：显示日志」）；已经开着就切过去。
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
            let branch = current.map(|b| b.name.clone());
            if let Some(view) = self.editor.documents[index].log.as_deref_mut() {
                view.query.branch = branch;
            }
            self.load_log(id);
        }
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 打开日志标签并选中提交 `oid`（从逐行追溯点过来）。
    ///
    /// 还没取回来的（筛着、或者在后面的批次里）：清掉筛选从头取，取回来以后选上它。
    pub(super) fn git_show_log_at(&mut self, oid: &str) {
        if self.git.root.is_none() {
            return;
        }
        let (index, new) = self.editor.show_log(log_title());
        let id = self.editor.documents[index].id;
        self.focus = Focus::Editor;
        let row = self.editor.documents[index]
            .log
            .as_deref()
            .and_then(|v| v.commits.iter().position(|c| c.oid == oid));
        match row {
            Some(row) if !new => self.select_log_row(row),
            _ => {
                if let Some(view) = self.editor.documents[index].log.as_deref_mut() {
                    view.query = LogQuery::default();
                    view.selected = None;
                    view.reset();
                    view.details = Some(Details {
                        oid: oid.to_owned(),
                        ..Details::default()
                    });
                }
                self.log.filter.set_text("");
                self.editor.documents[index].scroll_line = 0.0;
                self.load_log(id);
            }
        }
        self.relayout();
        self.request_redraw();
    }

    /// 当前标签是不是日志。
    pub(crate) fn log_active(&self) -> bool {
        self.editor.active().is_some_and(|d| d.log.is_some())
    }

    /// 在后台取文档 `id` 的下一批提交。
    ///
    /// 正在取、已经取完时什么也不做。
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
        // 从头取（刚打开、刷新、改筛选）：一小会儿里不写「正在读取」「正在刷新」。
        if view.stale || view.commits.is_empty() {
            self.log.list_waiting = Some(Instant::now() + LOADING_DELAY);
        }
        // 后台还有一个重取之前发出的请求在运行：先不发出，等它交回来以后再取（同一时刻只运行
        // 一个）。
        let Some(request) = view.begin() else {
            return;
        };
        let query = view.query.clone();
        let sender = self.log.results.0.clone();
        // 继续上一批往下读取；从头取（`skip` 为 0）或重取过时新建一个，旧的随之结束。
        let held = self
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
                        // 正常不会走到这里（继续读取的流总是保留）；万一没有，跳过已经取回的部分。
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

    /// 文档 `id` 的日志从头再取（`HEAD` 变了、点了「刷新」）。
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

    /// `HEAD` 或分支变了：开着的日志标签都重新取。
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

    /// 绘制时看到快滚到已经取回来的最后一个提交了：继续取下一批。
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

    /// 选中当前日志标签里的第 `row` 个提交，让它露在列表里，并在后台取它的详情。
    pub(crate) fn select_log_row(&mut self, row: usize) {
        let visible = ((self.log_parts().list.h - self.log_list_top()) / self.fonts.line_height)
            .floor()
            .max(1.0);
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
        self.log.file = None;
        view.selected = Some(row);
        // 选中的字是上一个提交的详情里的。
        self.log.text_sel = None;
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
                    self.log.waiting = Some(Instant::now() + LOADING_DELAY);
                    self.fetch_log_details(id, oid, parent);
                }
            }
        }
        self.prefetch_log_neighbours(id, row);
        // 露出来：在屏幕外时滚到它离顶上三行的地方，贴着边时挪一行。
        let row = row as f32;
        if let Some(doc) = self.editor.active_mut() {
            if row < doc.scroll_line {
                doc.scroll_line = row;
            } else if row + 1.0 > doc.scroll_line + visible {
                doc.scroll_line = row + 1.0 - visible;
            }
        }
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 预取第 `row` 个提交上下两个的详情（缓存里没有、也没在取的）：键盘上下走到它们时详情
    /// 马上就有，不会先留空再出现。
    fn prefetch_log_neighbours(&mut self, id: DocumentId, row: usize) {
        let Some(view) = self.editor.document(id).and_then(|d| d.log.as_deref()) else {
            return;
        };
        let wanted: Vec<(String, Option<String>)> = [row.checked_sub(1), Some(row + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| view.commits.get(i))
            .filter(|c| view.cached(&c.oid).is_none() && !self.log.fetching.contains(&c.oid))
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
        if !self.log.fetching.insert(oid.clone()) {
            return;
        }
        let sender = self.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let cancel = Cancel::default();
            let (message, files) = match tool {
                Ok(git) => (
                    git.commit_message(&root, &oid, &cancel),
                    git.commit_files(&root, parent.as_deref(), &oid, &cancel),
                ),
                Err(e) => (Err(unusable(&e)), Err(String::new())),
            };
            let _ = sender.send(Loaded::Details {
                doc: id,
                oid,
                message,
                files,
            });
        });
    }

    /// 点了日志标签里的 `target`。
    pub(in crate::app) fn press_log(&mut self, target: LogTarget) {
        match target {
            LogTarget::Uncommitted => self.show_commit(),
            LogTarget::Row(row) | LogTarget::Avatar(row) => {
                self.focus = Focus::Editor;
                // ⌘ 点另一个提交：比较选着的那个和点到的这个提交。
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
            // 拖动在 `mouse.rs` 里开始。
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
                    self.reload_log(id);
                }
            }
            LogTarget::Search => {
                self.focus = Focus::LogFilter;
                let (x, y) = self.mouse;
                self.click_field(x, y);
            }
            LogTarget::Branch | LogTarget::Author => {
                let Some(rect) = self
                    .log
                    .hits
                    .iter()
                    .find(|(_, t)| *t == target)
                    .map(|(r, _)| *r)
                else {
                    return;
                };
                let menu = if target == LogTarget::Branch {
                    MenuTarget::LogBranch
                } else {
                    self.log.authors = self.log_authors();
                    MenuTarget::LogAuthor
                };
                self.open_context_menu(menu, (rect.x, rect.bottom() + 4.0));
            }
            LogTarget::ClearFilters => {
                self.log.filter.set_text("");
                self.change_log_query(|q| {
                    q.text.clear();
                    q.branch = None;
                    q.author = None;
                });
            }
        }
        self.request_redraw();
    }

    /// 打开选中提交里第 `i` 个改动文件的差异：左边是第一个父提交里的版本，右边是这次提交里的。
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
        // 比较两个版本时：左边是旧的一边，右边是新的一边；否则左边是第一个父提交（根提交没有）。
        let Some((old, new, file, title)) = self.editor.document(log).and_then(|d| {
            let view = d.log.as_deref()?;
            if let Some(compare) = &view.compare {
                let file = compare.files.as_ref()?.get(i)?.clone();
                let name = winc_editor::display_name(&file.path).into_owned();
                let title = format!("{name} · {}…{}", short(&compare.from), short(&compare.to));
                return Some((Some(compare.from.clone()), compare.to.clone(), file, title));
            }
            let commit = view.commits.get(view.selected?)?;
            let file = view.details.as_ref()?.files.as_ref()?.get(i)?.clone();
            let name = winc_editor::display_name(&file.path).into_owned();
            let title = format!("{name} · {}", short(&commit.oid));
            Some((
                commit.parents.first().cloned(),
                commit.oid.clone(),
                file,
                title,
            ))
        }) else {
            return;
        };
        self.log.preview_source = Some((log, i));
        let sender = self.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let view = match tool {
                Ok(git) => commit_diff(&git, &root, old.as_deref(), &new, &file).map(Box::new),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Loaded::Diff { root, title, view });
        });
    }

    /// 标签 `id` 是否是从日志打开的差异。
    pub(in crate::app) fn steps_log_files(&self, id: DocumentId) -> bool {
        self.log.preview == Some(id) && self.log.preview_source.is_some()
    }

    /// 从日志打开的差异标签里，同一提交中的下一个（`forward`）或上一个文件：（日志标签，从 0 起的
    /// 序号，文件名，文件总数）；当前标签不是这种差异，或者没有更多文件时为 `None`。
    pub(in crate::app) fn next_log_file(
        &self,
        forward: bool,
    ) -> Option<(DocumentId, usize, String, usize)> {
        let active = self.editor.active().map(|d| d.id);
        let (log, i) = self
            .log
            .preview_source
            .filter(|_| active == self.log.preview)?;
        let view = self.editor.document(log)?.log.as_deref()?;
        let files = match &view.compare {
            Some(compare) => compare.files.as_ref()?,
            None => view.details.as_ref()?.files.as_ref()?,
        };
        let next = if forward { i + 1 } else { i.checked_sub(1)? };
        let name = winc_editor::display_name(&files.get(next)?.path).into_owned();
        Some((log, next, name, files.len()))
    }

    /// 在从日志打开的差异标签中打开同一提交中的下一个（`forward`）或上一个文件，返回是否已打开。
    pub(in crate::app) fn step_log_file(&mut self, forward: bool) -> bool {
        let Some((log, next, name, total)) = self.next_log_file(forward) else {
            return false;
        };
        self.open_log_file_of(log, next);
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

    /// 收下后台的结果。
    pub(in crate::app) fn poll_log(&mut self) {
        while let Ok(loaded) = self.log.results.1.try_recv() {
            match loaded {
                Loaded::Commits {
                    doc,
                    generation,
                    result,
                    stream,
                } => self.log_commits_arrived(doc, generation, result, stream),
                Loaded::Details {
                    doc,
                    oid,
                    message,
                    files,
                } => {
                    self.log.fetching.remove(&oid);
                    let Some(view) = self
                        .editor
                        .document_mut(doc)
                        .and_then(|d| d.log.as_deref_mut())
                    else {
                        continue;
                    };
                    let error = message.as_ref().err().or(files.as_ref().err()).cloned();
                    let mut files = files.unwrap_or_default();
                    // 文件历史：看的那个文件排在最前，回车打开的就是这个文件；改过名的按文件名认。
                    if let Some(path) = &view.query.path {
                        let name = path.file_name();
                        let at = files
                            .iter()
                            .position(|f| &f.path == path)
                            .or_else(|| files.iter().position(|f| f.path.file_name() == name));
                        if let Some(at) = at {
                            let file = files.remove(at);
                            files.insert(0, file);
                        }
                    }
                    let details = Details {
                        oid: oid.clone(),
                        message: message.ok(),
                        files: Some(files),
                        error,
                    };
                    // 出错的不缓存：下次选到它再试。
                    if details.error.is_none() {
                        view.remember(details.clone());
                    }
                    // 预取的（不是选着的那个）只进缓存。
                    let Some(shown) = view.details.as_mut().filter(|d| d.oid == oid) else {
                        continue;
                    };
                    *shown = details;
                    self.log.waiting = None;
                    if self.log.open_first.as_deref() == Some(oid.as_str()) {
                        self.log.open_first = None;
                        self.open_log_file(0);
                    }
                }
                Loaded::Compare {
                    doc,
                    from,
                    to,
                    files,
                } => {
                    let Some(compare) = self
                        .editor
                        .document_mut(doc)
                        .and_then(|d| d.log.as_deref_mut()?.compare.as_mut())
                        .filter(|c| c.from == from && c.to == to)
                    else {
                        continue;
                    };
                    self.log.waiting = None;
                    match files {
                        Ok(files) => compare.files = Some(files),
                        Err(e) => {
                            compare.files = Some(Vec::new());
                            compare.error = Some(e);
                        }
                    }
                }
                // 等结果的时候换了目录：是旧仓库的，不在这里开标签。
                Loaded::Diff { root, .. } if self.git.root.as_deref() != Some(root.as_path()) => {}
                Loaded::Diff { title, view, .. } => match view {
                    Ok(view) => {
                        let index = self.editor.show_diff_in(self.log.preview, &title, *view);
                        self.log.preview = Some(self.editor.documents[index].id);
                        self.relayout();
                        self.clamp_scroll();
                        self.land_diff_file();
                    }
                    Err(e) => {
                        self.diff_land = None;
                        self.flash(e);
                    }
                },
            }
        }
        self.request_redraw();
    }

    fn log_commits_arrived(
        &mut self,
        doc: DocumentId,
        generation: u64,
        result: Result<Vec<Commit>, String>,
        stream: Option<LogStream>,
    ) {
        let Some(view) = self
            .editor
            .document_mut(doc)
            .and_then(|d| d.log.as_deref_mut())
        else {
            return;
        };
        let arrival = view.finish(generation, result);
        // 接上了、还没取完：保留下来读下一批。其余情况（作废、取完、出错）移除。
        if let (Arrival::Applied { .. }, false, Some(stream)) = (arrival, view.complete, stream) {
            self.log.streams.insert(doc, (generation, stream));
        }
        match arrival {
            Arrival::Applied { first: true } if view.selected.is_none() => {}
            Arrival::Applied { .. } | Arrival::Dropped => return,
            Arrival::Refetch => {
                self.load_log(doc);
                return;
            }
        }
        // 第一批到了：选上重取之前选着的那个提交，没有就选 `HEAD`，再没有就选第一个。
        let keep = view.details.as_ref().map(|d| d.oid.clone());
        let at = |pred: &dyn Fn(&Commit) -> bool| view.commits.iter().position(pred);
        let row = keep
            .and_then(|oid| at(&|c| c.oid == oid))
            .or_else(|| {
                at(&|c| {
                    c.refs
                        .iter()
                        .any(|r| matches!(r, Ref::Head(_) | Ref::Detached))
                })
            })
            .or((!view.commits.is_empty()).then_some(0));
        // 详情留着：还是同一个提交时（刷新、`HEAD` 动了）不用再取，详情一栏不闪。
        let active = self.editor.active().is_some_and(|d| d.id == doc);
        if let Some(row) = row
            && active
        {
            self.select_log_row(row);
        }
    }

    /// 当前日志标签的几何。
    pub(crate) fn log_parts(&self) -> LogParts {
        log_parts(self.layout.editor, self.log.details_w)
    }

    /// 等详情的时候到了 `LOADING_DELAY` 还没回来：重画一遍，写上「正在读取…」。
    ///
    /// 返回下一次要醒来的时刻。
    pub(in crate::app) fn log_tick(&mut self, now: Instant) -> Option<Instant> {
        let (mut wake, mut due) = (None, false);
        for slot in [&mut self.log.waiting, &mut self.log.list_waiting] {
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

    /// 详情一栏的「正在读取…」「正在比较…」还不该写（刚开始取，一般马上就回来）。
    pub(crate) fn log_quiet(&self) -> bool {
        self.log.waiting.is_some_and(|at| Instant::now() < at)
    }

    /// 提交列表的「正在读取提交…」、标题栏的「正在刷新…」还不该写。
    pub(crate) fn log_list_quiet(&self) -> bool {
        self.log.list_waiting.is_some_and(|at| Instant::now() < at)
    }

    /// 正在拖列表和详情之间的分隔线。
    pub(crate) fn log_dividing(&self) -> bool {
        matches!(self.drag, crate::app::Drag::LogDivider { .. })
    }
}

/// 在后台准备一个文件在两个版本之间的差异：左边是 `old` 里的版本（改名的取旧路径；`None` 是根
/// 提交之前，什么都没有），右边是 `new` 里的。
fn commit_diff(
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
    Ok(DiffView::new(
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
    ))
}

/// 提交号的前 7 位。
pub(crate) fn short(oid: &str) -> &str {
    &oid[..oid.len().min(7)]
}

#[cfg(test)]
pub(crate) mod tests;
