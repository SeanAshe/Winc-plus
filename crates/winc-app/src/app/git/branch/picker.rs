//! 分支选择器：从提交视图的分支行就地展开，或者从状态栏的分支名往上展开，两处是同一个面板。
//!
//! 搜索框下面是一排拉取、推送、获取（按住 ⌥ 时拉取换成拉取并变基）：分支和同步都在这里，
//! 状态栏不再另有一套分支菜单。
//! 依次是最近使用、本地分支和可以展开的远程分支，分支名按 `/` 分层，只展开当前分支所在的目录。
//! 同一个分支只列一次：本地分支一屏放得下时不单列最近使用，单列时下面不再重复；已有本地分支的
//! 远程分支只在搜索时列出。
//! 输入框既能搜索也能新建：搜索时目录全部展开，没有同名分支时最后一行是「新建分支」。回车切换
//! 到选中的分支，有未提交的更改时用提示框询问带过去还是储藏；右键或行尾的「⋯」打开分支的操作菜单。

use std::collections::{BTreeMap, HashSet};

use winc_editor::git::{Branch, Status};
use winc_i18n::t;
use winc_render::Rect;

use super::local_name;
use crate::app::animation::TREE_ANIMATION;
use crate::app::git::location_picker::LocationRow;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::app::input::palette::{matched, needle, rate};
use crate::app::{AlertButton, App, Clock, Focus, PICKER_ROW, UserEvent};
use crate::layout::{MENU_PICKER_BAR, MENU_PICKER_PAD};

/// 最近使用的分支最多列出几个（个）：加上标题不超过选择面板一半的行数，本地分支仍在首屏。
const RECENT_BRANCHES: usize = 5;
/// [`rate`] 给字符分散命中的最低得分：低于它的是连续命中。
const SCATTERED: u32 = 1000;

/// 分支、位置选择器从哪儿展开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitAnchor {
    /// 提交视图的分支行下方。
    Branch,
    /// 提交视图的位置行下方。
    Location,
    /// 状态栏的分支名上方：侧栏收起、看着文件视图时也能切换分支和同步。
    Status,
    /// Git 日志的分支筛选按钮下方。
    Log,
}

/// 选择面板里除了各行以外可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerPart {
    /// 第几行行尾的「⋯」。
    More(usize),
    /// 搜索框下面的「拉取」；按住 ⌥ 时是拉取并变基。
    Pull,
    /// 搜索框下面的「推送」。
    Push,
    /// 搜索框下面的「获取」。
    Fetch,
    /// 底栏的「新建分支」。
    NewBranch,
    /// 底栏的「新建工作树」。
    NewWorktree,
    /// 最近目录列表第几行行尾的「✕」：从最近列表中移除该目录。
    ForgetFolder(usize),
}

/// 选择面板最小、最大的宽度（逻辑像素）：平时和 Git 上下文的行一样宽，两边对齐。最小宽度放得下
/// 较长的分支名和后面的时间；最大宽度与窗口内绘制的菜单相同，名字和右端的文字不至于隔得太远。
const GIT_PICKER_WIDTH: (f32, f32) = (260.0, 420.0);
/// 选择面板最多同时显示的行数（行）：分支常常成组展开，比文件列表多留一些行。
const GIT_PICKER_ROWS: usize = 16;
/// 选择面板和触发它的按钮之间的间距（逻辑像素）。
const GIT_PICKER_GAP: f32 = 4.0;
/// 选择面板底栏的高度（逻辑像素）：上面留 4，`BUTTON_SMALL` 高的按钮，下面留 6，和顶部的搜索行
/// 对称。
const GIT_PICKER_FOOT: f32 = 34.0;
/// 分支选择器中拉取、推送、获取一行的高度（逻辑像素）：上下各留 4，中间是 `BUTTON_SMALL` 高的
/// 按钮。加上搜索行下方的 4，按钮与搜索框相距 8，不贴近搜索框获得焦点时的外框。
pub const GIT_PICKER_ACTIONS: f32 = 32.0;
/// 拉取、推送、获取按钮上方的留白（逻辑像素）。
pub const GIT_PICKER_ACTIONS_TOP: f32 = 4.0;
/// 从状态栏展开时面板的宽度（逻辑像素）：和默认宽度的侧栏里从分支行展开时一样宽（侧栏 300
/// 减去两边各 8 的边距）。
const STATUS_PICKER_W: f32 = crate::theme::DEFAULT_PANEL_WIDTH - 16.0;

/// 分支选择器的分组标题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchSection {
    /// 最近使用。
    Recent,
    /// 本地分支。
    Local,
    /// 日志分支筛选中的远程分支：有多个远程时按远程分组，带远程名；只有一个远程时为 `None`。
    Remote(Option<String>),
    /// 最近使用以外的本地分支。
    Other,
}

/// 分支选择器的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchRow {
    /// 清除日志的分支筛选。
    All,
    /// 分组标题，不能选中。
    Heading(BranchSection),
    /// 可以展开的「远程分支」一组。
    Remotes {
        /// 这一组的分支数。
        count: usize,
        /// 是否展开。
        open: bool,
    },
    /// 分支名按 `/` 分出的一层目录。
    Folder {
        /// 是否在远程分支一组。
        remote: bool,
        /// 从分支名开头到这一层的路径。
        path: String,
        /// 显示的名字：只有一个子目录的目录与子目录合成一行。
        label: String,
        /// 缩进的层数。
        depth: usize,
        /// 目录下的分支数。
        count: usize,
        /// 是否展开。
        open: bool,
    },
    /// 一个分支。
    Branch {
        /// [`GitState::branches`](crate::app::git::GitState::branches) 的下标。
        index: usize,
        /// 缩进的层数。
        depth: usize,
        /// 显示的名字：去掉上级目录。
        label: String,
    },
    /// 新建名字是查询文字的分支。
    Create(String),
}

impl BranchRow {
    /// 这一行能否选中。
    pub fn selectable(&self) -> bool {
        !matches!(self, BranchRow::Heading(_))
    }

    /// 两行是否是同一项：展开状态变化、列表刷新以后据此留住选中项。
    ///
    /// 分支的下标按各自所在的列表解释：`self` 是 `old` 的，`other` 是 `new` 的。
    fn same(&self, other: &BranchRow, old: &[Branch], new: &[Branch]) -> bool {
        match (self, other) {
            (BranchRow::All, BranchRow::All) => true,
            (BranchRow::Remotes { .. }, BranchRow::Remotes { .. }) => true,
            (
                BranchRow::Folder { remote, path, .. },
                BranchRow::Folder {
                    remote: r, path: p, ..
                },
            ) => remote == r && path == p,
            (
                BranchRow::Branch { index, depth, .. },
                BranchRow::Branch {
                    index: i, depth: d, ..
                },
            ) => depth == d && old.get(*index).map(|b| &b.name) == new.get(*i).map(|b| &b.name),
            _ => false,
        }
    }
}

/// 返回判断分支是否命中查询 `query`（[`needle`] 处理过）的函数；没有查询时全部命中。
///
/// 有连续命中的分支时不计字符分散命中的，否则短查询会带出大量无关分支。
fn branch_hit<'a>(branches: &[Branch], query: &'a [char]) -> impl Fn(&Branch) -> bool + 'a {
    let searching = !query.is_empty();
    let score = move |branch: &Branch| rate(query, &branch.name);
    let contiguous = searching
        && branches
            .iter()
            .any(|b| score(b).is_some_and(|s| s < SCATTERED));
    move |branch| !searching || score(branch).is_some_and(|s| !contiguous || s < SCATTERED)
}

/// 排出日志分支筛选的各行：没有查询时最前面是「全部」，之后是本地分支和远程分支。
///
/// 命中规则与 [`branch_rows`] 相同。远程分支按远程分组，各行省去远程名，由组标题写明；只有
/// 一个远程时只有一组，标题为「远程分支」。
fn log_branch_rows(branches: &[Branch], query: &[char]) -> Vec<BranchRow> {
    let hit = branch_hit(branches, query);
    let mut rows = Vec::new();
    if query.is_empty() {
        rows.push(BranchRow::All);
    }
    let row = |index: usize, label: &str| BranchRow::Branch {
        index,
        depth: 0,
        label: label.to_owned(),
    };
    let mut local = branches
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.remote && hit(b))
        .peekable();
    if local.peek().is_some() {
        rows.push(BranchRow::Heading(BranchSection::Local));
        rows.extend(local.map(|(index, b)| row(index, &b.name)));
    }
    let mut remotes: BTreeMap<&str, Vec<(usize, &str)>> = BTreeMap::new();
    for (index, branch) in branches.iter().enumerate().filter(|(_, b)| b.remote) {
        let (remote, name) = branch.name.split_once('/').unwrap_or(("", &branch.name));
        let group = remotes.entry(remote).or_default();
        if hit(branch) {
            group.push((index, name));
        }
    }
    let single = remotes.len() == 1;
    for (remote, group) in remotes.into_iter().filter(|(_, g)| !g.is_empty()) {
        let title = (!single).then(|| remote.to_owned());
        rows.push(BranchRow::Heading(BranchSection::Remote(title)));
        rows.extend(group.into_iter().map(|(index, name)| row(index, name)));
    }
    rows
}

/// 排列分支选择器时用到的内容。
pub struct BranchList<'a> {
    /// 全部分支。
    pub branches: &'a [Branch],
    /// 最近切换到过的分支名，最近的在前。
    pub recent: &'a [String],
    /// [`needle`] 处理过的查询。
    pub query: &'a [char],
    /// 原样的查询文字，新建分支时用作名字。
    pub text: &'a str,
    /// 改变过默认展开状态的目录：（是否远程，目录路径）；远程分支一组的路径是空的。
    pub toggled: &'a HashSet<(bool, String)>,
}

/// 排出分支选择器的各行。
pub fn branch_rows(list: &BranchList<'_>) -> Vec<BranchRow> {
    let (branches, query) = (list.branches, list.query);
    let searching = !query.is_empty();
    let hit = branch_hit(branches, query);
    let current = branches.iter().find(|b| b.current).map(|b| b.name.as_str());
    let mut rows = Vec::new();
    // 本地分支一屏放得下时都看得见，单列最近使用只是重复。
    let many = branches.iter().filter(|b| !b.remote).count() > GIT_PICKER_ROWS;
    let latest = if many && !searching {
        quick_branches(branches, list.recent)
    } else {
        Vec::new()
    };
    if !latest.is_empty() {
        rows.push(BranchRow::Heading(BranchSection::Recent));
        rows.extend(latest.iter().map(|&index| BranchRow::Branch {
            index,
            depth: 0,
            label: branches[index].name.clone(),
        }));
    }
    let items = |remote: bool| -> Vec<(usize, &str)> {
        branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.remote == remote && hit(b))
            .map(|(i, b)| (i, b.name.as_str()))
            .collect()
    };
    let tree = |remote| Tree {
        branches,
        remote,
        searching,
        current,
        toggled: list.toggled,
    };
    let mut local = items(false);
    local.retain(|(index, _)| !latest.contains(index));
    if !local.is_empty() {
        let section = if latest.is_empty() {
            BranchSection::Local
        } else {
            BranchSection::Other
        };
        rows.push(BranchRow::Heading(section));
        tree(false).push(local, "", 0, &mut rows);
    }
    // 已有本地分支的远程分支只在搜索时列出：平时选本地那一行就是它，搜索时还能拿它比较、合并。
    let mut remote = items(true);
    remote.retain(|&(index, _)| searching || !has_local_branch(branches, &branches[index]));
    if !remote.is_empty() {
        let flipped = list.toggled.contains(&(true, String::new()));
        let open = searching || flipped;
        rows.push(BranchRow::Remotes {
            count: remote.len(),
            open,
        });
        if open {
            // 和本地分支同一层：这一行只是分组标题，名字的列上下对齐。
            let remote = strip_single_remote(remote);
            tree(true).push(remote, "", 0, &mut rows);
        }
    }
    let name = list.text.trim();
    let exists = branches.iter().any(|b| !b.remote && b.name == name);
    if searching && !exists {
        rows.push(BranchRow::Create(
            name.split_whitespace().collect::<Vec<_>>().join("-"),
        ));
    }
    rows
}

/// 远程分支 `remote` 是否已有本地分支：同名，或者有本地分支跟踪它。
pub(in crate::app) fn has_local_branch(branches: &[Branch], remote: &Branch) -> bool {
    let name = local_name(remote);
    (branches.iter().filter(|b| !b.remote))
        .any(|b| b.name == name || b.upstream.as_deref() == Some(remote.name.as_str()))
}

/// 只有一个远程时去掉分支名里的远程名：少一层没有信息量的目录。
fn strip_single_remote(items: Vec<(usize, &str)>) -> Vec<(usize, &str)> {
    let first = items
        .first()
        .and_then(|(_, n)| n.split_once('/'))
        .map(|(r, _)| r);
    let single = first.is_some_and(|remote| {
        (items.iter()).all(|(_, n)| n.split_once('/').is_some_and(|(r, _)| r == remote))
    });
    if !single {
        return items;
    }
    items
        .into_iter()
        .map(|(i, name)| (i, name.split_once('/').map_or(name, |(_, n)| n)))
        .collect()
}

/// 「最近使用」里的本地分支：默认分支（`main`，没有时 `master`）排最前，之后是最近切换到过的；
/// 不含当前分支。
fn quick_branches(branches: &[Branch], recent: &[String]) -> Vec<usize> {
    let default = ["main", "master"]
        .into_iter()
        .find(|name| branches.iter().any(|b| !b.remote && b.name == *name));
    let names = default.into_iter().chain(recent.iter().map(String::as_str));
    let mut out = Vec::new();
    for name in names {
        let found = branches
            .iter()
            .position(|b| !b.remote && !b.current && b.name == name);
        if let Some(index) = found
            && !out.contains(&index)
        {
            out.push(index);
        }
    }
    out.truncate(RECENT_BRANCHES);
    out
}

/// 分支排序用的值，大的在前：当前分支最前，本地分支按最后一次提交的时间，远程分支都一样（再按
/// 名字）。
pub(in crate::app) fn branch_rank(branch: &Branch) -> i64 {
    match (branch.remote, branch.current) {
        (true, _) => 0,
        (false, true) => i64::MAX,
        (false, false) => branch.time,
    }
}

/// 排一组分支的树时不变的条件。
struct Tree<'a> {
    branches: &'a [Branch],
    remote: bool,
    searching: bool,
    current: Option<&'a str>,
    toggled: &'a HashSet<(bool, String)>,
}

impl Tree<'_> {
    /// 把 `items`（下标、去掉 `prefix` 的分支名）排成一层。
    ///
    /// 本地分支里当前分支（或它所在的目录）在最前，其余按最后一次提交的时间排，最近的在前，目录按
    /// 里面最近的分支算；时间相同时目录在前，再按名字。远程分支只按名字。
    fn push(
        &self,
        items: Vec<(usize, &str)>,
        prefix: &str,
        depth: usize,
        rows: &mut Vec<BranchRow>,
    ) {
        let mut folders: BTreeMap<&str, Vec<(usize, &str)>> = BTreeMap::new();
        let mut leaves = Vec::new();
        for (index, name) in items {
            match name.split_once('/') {
                Some((folder, rest)) => folders.entry(folder).or_default().push((index, rest)),
                None => leaves.push((index, name)),
            }
        }
        let time = |index: usize| self.branches.get(index).map_or(0, branch_rank);
        let mut nodes: Vec<_> = (folders.into_iter())
            .map(|(folder, sub)| {
                let newest = sub.iter().map(|&(i, _)| time(i)).max().unwrap_or(0);
                (newest, false, folder, sub)
            })
            .chain(
                leaves
                    .into_iter()
                    .map(|(i, name)| (time(i), true, name, vec![(i, name)])),
            )
            .collect();
        nodes.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
        for (_, leaf, folder, mut sub) in nodes {
            if leaf {
                let (index, name) = sub[0];
                rows.push(BranchRow::Branch {
                    index,
                    depth,
                    label: name.to_owned(),
                });
                continue;
            }
            // 只有一个子目录、没有分支：和子目录合成一行。
            let mut label = folder.to_owned();
            while let Some(next) = only_folder(&sub) {
                label.push('/');
                label.push_str(next);
                for (_, rest) in &mut sub {
                    *rest = &rest[next.len() + 1..];
                }
            }
            let path = format!("{prefix}{label}");
            // 默认只展开当前分支所在的目录，列表短，一眼能看全。
            let inside = !self.remote
                && self
                    .current
                    .is_some_and(|name| name.starts_with(&format!("{path}/")));
            let flipped = self.toggled.contains(&(self.remote, path.clone()));
            let open = self.searching || inside != flipped;
            rows.push(BranchRow::Folder {
                remote: self.remote,
                path: path.clone(),
                label,
                depth,
                count: sub.len(),
                open,
            });
            if open {
                self.push(sub, &format!("{path}/"), depth + 1, rows);
            }
        }
    }
}

/// 这些分支都在同一个子目录里时返回它的名字。
fn only_folder<'a>(items: &[(usize, &'a str)]) -> Option<&'a str> {
    let (first, _) = items.first()?.1.split_once('/')?;
    items
        .iter()
        .all(|(_, name)| {
            name.split_once('/')
                .is_some_and(|(folder, _)| folder == first)
        })
        .then_some(first)
}

/// 已跟踪文件上未提交的更改数：切换分支时只有它们需要考虑，未跟踪的文件不受影响。
fn tracked_changes(status: &Status) -> usize {
    status.entries.iter().filter(|e| !e.untracked()).count()
}

/// 目录、组展开或收起之前的状态。
#[derive(Debug, Clone)]
pub(crate) struct FoldBefore {
    /// 分支选择器的各行；位置选择器里是空的。
    pub branches: Vec<BranchRow>,
    /// 位置选择器的各行；分支选择器里是空的。
    pub locations: Vec<LocationRow>,
    /// 展开或收起的那一行的下标。
    pub parent: usize,
    /// 面板的高度（逻辑像素）。
    height: f32,
    /// 滚动量（逻辑像素）。
    scroll: f32,
}

/// 选择面板里目录、组展开或收起的动画，实现方式与改动列表的 `CommitMotion` 相同：子行从父行下方
/// 逐渐显示或收回，其后的各行随之移动，面板的高度和滚动量同时过渡到新的位置。
#[derive(Debug, Clone)]
pub(crate) struct PickerFold {
    /// 之前的状态：收起时画它的各行（收回的行还在），展开时画现在的各行。
    pub before: FoldBefore,
    /// 露出或收回的行数。
    pub count: usize,
    /// 是展开；`false` 是收起。
    pub expanding: bool,
    /// 这一段动画的时钟。
    pub(in crate::app) clock: Clock,
}

impl PickerFold {
    /// 还没露出来的子行占的高度（逻辑像素）：下面的行往上收该距离。
    pub fn hidden(&self) -> f32 {
        let eased = self.clock.progress();
        let reveal = if self.expanding { eased } else { 1.0 - eased };
        (1.0 - reveal) * self.count as f32 * PICKER_ROW
    }

    /// 展开的箭头转过的比例：0 朝右，1 朝下。
    pub fn turned(&self) -> f32 {
        1.0 - self.hidden() / (self.count as f32 * PICKER_ROW)
    }

    /// 现在的第 `row` 行在动画期间所画各行里的下标：收起时收回的行还在，下面的行要往后数。
    pub fn drawn_index(&self, row: usize) -> usize {
        if !self.expanding && row > self.before.parent {
            row + self.count
        } else {
            row
        }
    }
}

impl App {
    /// 记下选中的行展开或收起之前的状态，改完以后交给 [`App::start_picker_fold`]。
    pub(in crate::app) fn picker_fold_before(&self) -> FoldBefore {
        FoldBefore {
            branches: self.picker.branch_rows.clone().unwrap_or_default(),
            locations: self.picker.location_rows.clone().unwrap_or_default(),
            parent: self.picker.selected,
            height: self.layout.picker.h,
            scroll: self.picker_drawn_scroll(),
        }
    }

    /// 选中的行刚展开或收起：调整滚动量，开始动画。
    ///
    /// 展开时让新露出的行尽量都看得见，展开的那一行不滚出去；收起以后不留空白。打开了「减弱动态
    /// 效果」时直接到位。
    pub(in crate::app) fn start_picker_fold(&mut self, before: FoldBefore) {
        self.picker.fold = None;
        self.relayout();
        let rows = self.picker.hits.len();
        let was = before.branches.len().max(before.locations.len());
        let (expanding, count) = (rows > was, rows.abs_diff(was));
        let span = crate::app::workspace::picker::picker_span(self);
        let mut scroll = self.picker.scroll;
        if expanding {
            let bottom = (before.parent + 1 + count) as f32 * PICKER_ROW;
            let top = before.parent as f32 * PICKER_ROW;
            scroll = scroll.max(bottom - span).min(top);
        }
        let most = (rows as f32 * PICKER_ROW - span).max(0.0);
        self.picker.scroll = scroll.clamp(0.0, most);
        if count > 0 {
            self.picker.fold = self.clock(TREE_ANIMATION).map(|clock| PickerFold {
                before,
                count,
                expanding,
                clock,
            });
        }
        self.relayout();
        self.request_redraw();
    }

    /// 绘制用的滚动量：展开、收起的动画期间从原来的位置过渡到新的位置。
    pub(crate) fn picker_drawn_scroll(&self) -> f32 {
        (self.picker.fold.as_ref()).map_or(self.picker.scroll, |fold| {
            fold.clock.between(fold.before.scroll, self.picker.scroll)
        })
    }

    /// 把面板摆到触发它的按钮下方，左右和提交视图的行对齐。放不下全部行时滚动。
    pub(in crate::app) fn place_git_picker(&mut self) {
        if !self.picker.open || !self.picker_is_git() {
            return;
        }
        let window = self.layout.window;
        let most = (window.w - 16.0).max(0.0);
        let chrome = MENU_PICKER_BAR + MENU_PICKER_PAD + self.git_picker_footer();
        let wanted = self.picker.hits.len().clamp(1, GIT_PICKER_ROWS);
        let chrome = chrome
            + if self.picker_has_actions() {
                GIT_PICKER_ACTIONS
            } else {
                0.0
            };
        let list = |room: f32| {
            let fit = ((room - chrome) / PICKER_ROW).floor() as usize;
            wanted.min(fit.max(1)) as f32 * PICKER_ROW
        };
        // 展开、收起的动画期间高度跟着过渡。
        let height = |h: f32| {
            (self.picker.fold.as_ref()).map_or(h, |fold| fold.clock.between(fold.before.height, h))
        };
        if self.picker.anchor == GitAnchor::Status {
            // 从状态栏往上展开：底边贴着分支名的上沿，右边和它对齐，放不下时往左挪。
            let Some(&(item, _)) =
                (self.hits.status.iter()).find(|(_, i)| *i == crate::app::StatusItem::Branch)
            else {
                return;
            };
            let bottom = item.y - GIT_PICKER_GAP;
            let room = bottom - crate::theme::TOP_BAR_END;
            let w = STATUS_PICKER_W.min(most);
            let h = height(chrome + list(room));
            let x = (item.right() - w).min(window.right() - 8.0 - w).max(8.0);
            self.layout.picker = Rect::new(x, bottom - h, w, h);
            return;
        }
        let (row, bottom) = if self.picker_filters_log() {
            // 从日志标题栏的「分支」按钮往下展开：位置取绘制时记录的按钮区域。底边与日志列表的
            // 底边相距 8，与面板离窗口左右边缘的距离相同。
            let Some(&(button, _)) =
                (self.hits.log.iter()).find(|(_, t)| *t == crate::app::git::log::LogTarget::Branch)
            else {
                return;
            };
            (button, self.log_parts().list.bottom() - 8.0)
        } else {
            let parts = self.commit_parts();
            let row = match self.picker.anchor {
                GitAnchor::Location => parts.location.unwrap_or(parts.branch),
                _ => parts.branch,
            };
            let sidebar = self.layout.sidebar;
            (row, sidebar.bottom() - (row.x - sidebar.x))
        };
        let y = row.bottom() + GIT_PICKER_GAP;
        let room = bottom - y;
        let w = row
            .w
            .clamp(GIT_PICKER_WIDTH.0, GIT_PICKER_WIDTH.1)
            .min(most);
        let h = height(chrome + list(room));
        let x = row.x.min(window.right() - 8.0 - w).max(8.0);
        self.layout.picker = Rect::new(x, y, w, h);
    }

    /// 点了选择面板里的 `part`。
    pub(in crate::app) fn press_picker_part(&mut self, part: PickerPart) {
        match part {
            PickerPart::More(row) => self.press_picker_more(row),
            PickerPart::Pull | PickerPart::Push | PickerPart::Fetch => {
                if !self.picker_action_enabled(part) {
                    return;
                }
                let command = self.picker_action_command(part);
                self.close_picker();
                self.run_branch_command(command);
            }
            // 新建分支使用窗口正中的表单对话框（名称、基于、创建后签出），与命令面板和分支菜单中
            // 的「新建分支」共用。
            PickerPart::NewBranch => {
                self.close_picker();
                self.open_branch_dialog(None, None);
            }
            PickerPart::NewWorktree => {
                self.close_picker();
                self.open_worktree_dialog();
            }
            PickerPart::ForgetFolder(row) => self.forget_picker_folder(row),
        }
        self.request_redraw();
    }

    /// 点了第 `row` 行行尾的「⋯」。
    fn press_picker_more(&mut self, row: usize) {
        self.picker.selected = row;
        let branch = (self.picker.branch_rows.as_ref()).and_then(|rows| match rows.get(row) {
            Some(BranchRow::Branch { index, .. }) => Some(*index),
            _ => None,
        });
        if let Some(index) = branch {
            self.open_branch_row_menu(MenuTarget::BranchOps(index));
            return;
        }
        let place = (self.picker.location_rows.as_ref()).and_then(|rows| match rows.get(row) {
            Some(LocationRow::Place(place)) => Some(place.clone()),
            _ => None,
        });
        if let Some(place) = place {
            self.open_location_row_menu(row, &place);
        }
    }

    /// 选择面板的第 `row` 行能否选中：分组标题不能。
    pub(in crate::app) fn picker_row_selectable(&self, row: usize) -> bool {
        (self.picker.branch_rows.as_ref())
            .is_none_or(|rows| rows.get(row).is_some_and(BranchRow::selectable))
    }

    /// 面板是否正在列出分支。
    pub(crate) fn picker_lists_branches(&self) -> bool {
        self.picker.branch_rows.is_some()
    }

    /// 是否正从 `anchor` 展开着分支选择器：触发它的按钮据此保持按下的样式。
    pub(crate) fn branch_picker_from(&self, anchor: GitAnchor) -> bool {
        self.picker_lists_branches() && self.picker.anchor == anchor
    }

    /// 是否为日志的分支筛选：从日志标题栏展开，选中的分支只修改日志的筛选条件，不切换分支。
    pub(crate) fn picker_filters_log(&self) -> bool {
        self.branch_picker_from(GitAnchor::Log)
    }

    /// 返回选择面板底栏的高度（逻辑像素）：日志的分支筛选没有新建分支、工作树的入口，没有底栏。
    pub(crate) fn git_picker_footer(&self) -> f32 {
        if self.picker_filters_log() {
            0.0
        } else {
            GIT_PICKER_FOOT
        }
    }

    /// 搜索框下面有没有拉取、推送、获取一行：列出分支时有。
    pub(crate) fn picker_has_actions(&self) -> bool {
        self.picker_lists_branches() && !self.picker_filters_log()
    }

    /// 拉取、推送、获取（`part`）对应的 Git 操作：按住 ⌥ 时拉取是拉取并变基。
    pub(crate) fn picker_action_command(&self, part: PickerPart) -> MenuCommand {
        match part {
            PickerPart::Pull if self.modifiers.alt_key() => MenuCommand::GitPullRebase,
            PickerPart::Pull => MenuCommand::GitPull,
            PickerPart::Push => MenuCommand::GitPush,
            _ => MenuCommand::GitFetch,
        }
    }

    /// 拉取、推送、获取（`part`）现在能不能用：有 Git 操作在跑时都不能；没有远程时都不能；拉取要
    /// 当前分支有上游；推送要在分支上（没有上游时推上去并设为上游）。
    pub(crate) fn picker_action_enabled(&self, part: PickerPart) -> bool {
        if self.branch.running.is_some() || self.git.remotes.is_empty() {
            return false;
        }
        let on_branch = (self.git.status.as_ref())
            .is_some_and(|s| matches!(s.head, Some(winc_editor::git::Head::Branch(_))));
        match part {
            PickerPart::Pull => self.has_upstream(),
            PickerPart::Push => on_branch,
            _ => true,
        }
    }

    /// 命令「Git：切换分支…」：提交视图开着时从分支行展开分支选择器，否则从状态栏的分支名；状态栏
    /// 也收着时先打开提交视图。
    pub(in crate::app) fn switch_branch_from_command(&mut self) {
        if self.commit_shown() {
            return self.open_branch_picker(GitAnchor::Branch);
        }
        let on_status =
            (self.hits.status.iter()).any(|(_, i)| *i == crate::app::StatusItem::Branch);
        if on_status {
            return self.open_branch_picker(GitAnchor::Status);
        }
        self.show_commit();
        self.open_branch_picker(GitAnchor::Branch);
    }

    /// 从 `anchor` 展开分支选择器。
    pub(in crate::app) fn open_branch_picker(&mut self, anchor: crate::app::GitAnchor) {
        self.finish_name_input();
        self.picker.places = None;
        self.picker.location_rows = None;
        self.picker.folders = false;
        self.picker.anchor = anchor;
        self.picker.open = true;
        self.picker.doc = self.editor.active().map(|d| d.id);
        self.picker.input.set_text("");
        self.picker.branch_rows = Some(Vec::new());
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 按查询重新排列各行，并选中一行。
    ///
    /// 没有查询（只有空白字符同样视为没有）时：日志的分支筛选选中已应用的筛选；其余情况在没有
    /// 单列最近使用时选中当前分支，选中的行记为自动选中（`Picker::default`）。其他情况选中第一个
    /// 分支，没有分支时选中第一个能选中的行。
    pub(in crate::app) fn branch_picker_refresh(&mut self) {
        let rows = self.current_branch_rows();
        let plain = needle(self.picker.input.text()).is_empty();
        let current = if !plain {
            None
        } else if self.picker_filters_log() {
            self.applied_log_filter_row(&rows)
        } else if rows.first() == Some(&BranchRow::Heading(BranchSection::Recent)) {
            None
        } else {
            rows.iter().position(|row| {
                matches!(row, BranchRow::Branch { index, .. }
                    if self.git.branches.get(*index).is_some_and(|b| b.current))
            })
        };
        self.picker.selected = current
            .or_else(|| {
                rows.iter()
                    .position(|row| matches!(row, BranchRow::Branch { .. }))
            })
            .or_else(|| rows.iter().position(BranchRow::selectable))
            .unwrap_or(0);
        self.picker.default = plain.then_some(self.picker.selected);
        self.set_branch_rows(rows);
        self.picker_reveal();
    }

    /// 返回日志分支筛选中已应用的筛选所在的行；筛选的分支已不在列表中（如已删除）时返回
    /// 「全部」一行，回车即清除该筛选。
    fn applied_log_filter_row(&self, rows: &[BranchRow]) -> Option<usize> {
        let filter = self.log_branch_filter();
        let applied = rows.iter().position(|row| match row {
            BranchRow::Branch { index, .. } => {
                (self.git.branches.get(*index)).is_some_and(|b| filter == Some(b.name.as_str()))
            }
            _ => false,
        });
        applied.or_else(|| rows.iter().position(|row| *row == BranchRow::All))
    }

    /// 分支列表从 `before` 刷新以后重排：下标可能已经指向别的分支，选中项按名字留住。
    pub(in crate::app) fn branch_picker_refreshed(&mut self, before: &[Branch]) {
        if self.picker_lists_branches() {
            self.keep_branch_selection(before);
        }
    }

    /// 查询在名字 `text` 里命中的字符下标：绘制时标出来。
    pub(crate) fn git_picker_matched(&self, text: &str) -> Vec<usize> {
        matched(&needle(self.picker.input.text()), text)
    }

    /// 重新排列各行，留住 `before` 列表下选中的那一项；它不在了时选中第一个能选中的行。
    fn keep_branch_selection(&mut self, before: &[Branch]) {
        let selected = (self.picker.branch_rows.as_ref())
            .and_then(|rows| rows.get(self.picker.selected))
            .cloned();
        let default = self.picker.default == Some(self.picker.selected);
        let rows = self.current_branch_rows();
        let same = |row: &BranchRow| {
            selected
                .as_ref()
                .is_some_and(|s| s.same(row, before, &self.git.branches))
        };
        self.picker.selected = rows
            .iter()
            .position(same)
            .or_else(|| rows.iter().position(BranchRow::selectable))
            .unwrap_or(0);
        // 自动选中的行换了位置仍是自动选中的。
        self.picker.default = default.then_some(self.picker.selected);
        self.set_branch_rows(rows);
        self.relayout();
        self.request_redraw();
    }

    fn current_branch_rows(&self) -> Vec<BranchRow> {
        let query = needle(self.picker.input.text());
        if self.picker_filters_log() {
            return log_branch_rows(&self.git.branches, &query);
        }
        branch_rows(&BranchList {
            branches: &self.git.branches,
            recent: &self.git.recent_branches,
            query: &query,
            text: self.picker.input.text(),
            toggled: &self.branch_folders,
        })
    }

    fn set_branch_rows(&mut self, rows: Vec<BranchRow>) {
        self.picker.hits = (0..rows.len()).collect();
        self.picker.branch_rows = Some(rows);
    }

    /// 返回向下（`down`）或向上移动一步后选中的行，跳过分组标题；到达两端后不动。
    pub(in crate::app) fn branch_picker_step(&self, down: bool) -> usize {
        let rows = self.picker.branch_rows.as_deref().unwrap_or_default();
        let at = self.picker.selected;
        let next = if down {
            (at + 1..rows.len()).find(|&i| rows[i].selectable())
        } else {
            (0..at).rev().find(|&i| rows[i].selectable())
        };
        next.unwrap_or(at)
    }

    /// 确认选中的行。
    pub(in crate::app) fn branch_picker_accept(&mut self) {
        let row = (self.picker.branch_rows.as_ref())
            .and_then(|rows| rows.get(self.picker.selected))
            .cloned();
        if self.picker_filters_log() {
            let branch = match row {
                Some(BranchRow::All) => None,
                Some(BranchRow::Branch { index, .. }) => {
                    let Some(branch) = self.git.branches.get(index) else {
                        return;
                    };
                    Some(branch.name.clone())
                }
                _ => return,
            };
            self.close_picker();
            self.change_log_query(|q| q.branch = branch);
            return;
        }
        match row {
            Some(BranchRow::Remotes { .. }) => self.toggle_branch_folder(true, String::new()),
            Some(BranchRow::Folder { remote, path, .. }) => self.toggle_branch_folder(remote, path),
            Some(BranchRow::Branch { index, .. }) => self.pick_branch(index),
            Some(BranchRow::Create(name)) => self.create_branch_from_picker(name, None),
            Some(BranchRow::All | BranchRow::Heading(_)) | None => {}
        }
    }

    /// 选了第 `index` 个分支：当前分支只关闭选择器；被别的工作树占着时切到那个工作树；不能签出
    /// 的（工作树目录已不在）打开操作菜单，灰掉的菜单项说明原因；有未提交的更改时先询问。
    pub(in crate::app) fn pick_branch(&mut self, index: usize) {
        let Some(branch) = self.git.branches.get(index).cloned() else {
            return;
        };
        let local = local_name(&branch);
        if branch.current {
            self.close_picker();
            return;
        }
        if self.git.stale_branches().contains(&local) {
            self.open_branch_row_menu(MenuTarget::BranchOps(index));
            return;
        }
        if let Some(i) = self.git.worktree_holding(local) {
            self.close_picker();
            self.open_worktree(i, false);
            return;
        }
        let changes = self.git.status.as_ref().map_or(0, tracked_changes);
        self.close_picker();
        // 未跟踪的文件不计入：它们同样可能使 git 拒绝切换，由 `switch_branch` 处理。
        if changes == 0 {
            self.switch_branch(branch, false);
            return;
        }
        let head = (self.git.branches.iter())
            .find(|b| b.current)
            .map_or("HEAD", |b| b.name.as_str());
        let message = t!("git-switch-dirty", branch = local);
        let detail = t!("git-switch-dirty-detail", current = head, count = changes);
        let buttons = vec![
            AlertButton::new(t!("git-switch-carry")),
            AlertButton::new(t!("git-switch-stash")),
            AlertButton::cancel(t!("common-cancel")),
        ];
        self.show_alert(&message, &detail, buttons, None, move |choice, _| {
            (choice < 2).then(|| UserEvent::GitSwitchBranch(branch.clone(), choice == 1))
        });
    }

    /// 切换到 `branch`；`stash` 时先储藏未提交的更改，否则更改留在工作区。
    pub(in crate::app) fn switch_branch(&mut self, branch: Branch, stash: bool) {
        if stash {
            self.stash_then_checkout(branch);
        } else {
            self.carry_then_checkout(branch);
        }
    }

    fn toggle_branch_folder(&mut self, remote: bool, path: String) {
        let before = self.picker_fold_before();
        let key = (remote, path);
        if !self.branch_folders.remove(&key) {
            self.branch_folders.insert(key);
        }
        let branches = self.git.branches.clone();
        self.keep_branch_selection(&branches);
        self.start_picker_fold(before);
    }

    /// 处理没有查询时的左右键：右键展开目录、打开分支的操作菜单，左键收起目录、回到上一级目录。
    /// 返回是否已处理；有查询时左右键移动输入框的光标。
    pub(in crate::app) fn branch_picker_side(&mut self, right: bool) -> bool {
        if !self.picker_lists_branches()
            || self.picker_filters_log()
            || !self.picker.input.text().is_empty()
        {
            return false;
        }
        let Some(row) = (self.picker.branch_rows.as_ref())
            .and_then(|rows| rows.get(self.picker.selected))
            .cloned()
        else {
            return false;
        };
        match (row, right) {
            (BranchRow::Remotes { open, .. }, _) if open != right => {
                self.toggle_branch_folder(true, String::new());
            }
            (
                BranchRow::Folder {
                    remote, path, open, ..
                },
                _,
            ) if open != right => self.toggle_branch_folder(remote, path),
            (BranchRow::Branch { index, .. }, true) => {
                self.open_branch_row_menu(MenuTarget::BranchOps(index));
            }
            (BranchRow::Folder { .. } | BranchRow::Remotes { .. }, true) => {
                self.picker.selected = self.branch_picker_step(true);
                self.picker.default = None;
                self.picker_reveal();
                self.request_redraw();
            }
            (_, false) => self.select_parent_branch_folder(),
            _ => {}
        }
        true
    }

    /// 选中当前行所在的目录；已经在最外层时不动。
    fn select_parent_branch_folder(&mut self) {
        let rows = self.picker.branch_rows.as_deref().unwrap_or_default();
        let depth_of = |row: &BranchRow| match row {
            BranchRow::Folder { depth, .. } | BranchRow::Branch { depth, .. } => Some(*depth),
            _ => None,
        };
        let Some(depth) = rows.get(self.picker.selected).and_then(depth_of) else {
            return;
        };
        // 远程分支一组没有层数，`None` 比任何层数都小：最外层的远程分支回到这一组。
        let parent = (0..self.picker.selected).rev().find(|&i| {
            matches!(
                rows[i],
                BranchRow::Folder { .. } | BranchRow::Remotes { .. }
            ) && depth_of(&rows[i]) < Some(depth)
        });
        if let Some(parent) = parent {
            self.picker.selected = parent;
            self.picker.default = None;
            self.picker_reveal();
            self.request_redraw();
        }
    }

    /// 在选中行的右端弹出菜单 `target`。
    pub(in crate::app) fn open_branch_row_menu(&mut self, target: MenuTarget) {
        let parts = crate::app::workspace::picker::picker_parts(self, self.layout.picker);
        let row = parts.row(self.picker.selected, self.picker.scroll);
        self.open_context_menu(target, (row.right() - 14.0, row.bottom()));
    }

    /// 关闭选择器，从 `from`（`None` 是当前分支）新建分支 `name` 并切换过去。
    fn create_branch_from_picker(&mut self, name: String, from: Option<String>) {
        self.close_picker();
        self.git_new_branch(name, from, true);
    }
}

#[cfg(test)]
mod tests;
