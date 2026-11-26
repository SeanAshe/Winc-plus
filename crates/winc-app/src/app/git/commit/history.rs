//! 提交视图中的提交历史：改动列表下面一组可以收起的行，列出当前分支与上游最近的提交和提交图。
//!
//! 尚未推送、尚未拉取的提交带有标记。悬停显示提交的详情卡片，点击在预览标签中依次显示该提交的
//! 全部更改，双击在日志中选中该提交，右键弹出提交菜单。行数多于 [`HISTORY_ROWS`] 时在组内滚动；更早
//! 的提交在日志中查看。提交在后台读取，`HEAD`、上游或引用变化后重新读取，提交视图不可见时推迟到
//! 下次显示。

use winc_editor::git::Commit;
use winc_editor::git::recent::{Recent, Side};

use std::collections::HashMap;

use super::*;
use crate::app::Hover;
use crate::app::scroll::ScrollTarget;

mod card;
pub(crate) use card::{CardState, CommitCard};

/// 提交历史一组展开时最多露出的行数（行），更多的行在组内滚动。
///
/// 取自调参：展开时连同标题约 225 逻辑像素，在 800 逻辑像素高的侧栏中不超过三分之一，
/// 改动列表仍占主要的高度。
pub(crate) const HISTORY_ROWS: usize = 8;

/// 提交历史一组给上方的改动列表至少留出的高度（逻辑像素）：四行。
///
/// 侧栏较矮（窗口矮、下方有 Git 面板、改动和储藏多）时提交历史先让出高度，组内露出的行变少；
/// 改动是提交视图的主要内容，取自调参：四行足够看出有哪些改动并滚动查看。
pub(crate) const HISTORY_LIST_KEEP: f32 = 4.0 * COMMIT_ROW;

/// 提交历史一组中可点击的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryTarget {
    /// 标题：收起或展开。
    Title,
    /// 悬停标题时右端的「在日志中显示」：打开日志。
    Log,
    /// 第几个提交（`Recent::commits` 的下标）：查看该提交的全部更改，双击在日志中选中该提交。
    Commit(usize),
    /// 最后一行「在日志中查看更早的提交」。
    More,
}

/// 提交历史一组展开时标题下面的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryLine {
    /// 第几个提交（`Recent::commits` 的下标）。
    Commit(usize),
    /// 还有更早的提交：在日志中查看。
    More,
}

/// 读取提交历史的依据，任何一项变化时重新读取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistoryKey {
    /// 仓库的根目录。
    root: PathBuf,
    /// `HEAD` 指向的提交号。
    head: String,
    /// 当前分支的上游（`origin/main`）和它指向的提交号；没有上游时为 `None`。
    upstream: Option<(String, String)>,
    /// 引用的版本：分支、标签变化时加 1。
    refs: u64,
}

/// 借用自当前仓库状态的读取依据，与 [`HistoryKey`] 的各项对应；每帧比较时不复制。
#[derive(Debug, Clone, Copy)]
struct Basis<'a> {
    root: &'a Path,
    head: &'a str,
    upstream: Option<(&'a str, &'a str)>,
}

impl Basis<'_> {
    /// 是否与按引用版本 `refs` 记下的依据 `key` 相同。
    fn matches(&self, key: &HistoryKey, refs: u64) -> bool {
        let upstream = (key.upstream.as_ref()).map(|(name, oid)| (name.as_str(), oid.as_str()));
        key.root == self.root
            && key.head == self.head
            && upstream == self.upstream
            && key.refs == refs
    }

    /// 返回引用版本为 `refs` 时的依据。
    fn key(&self, refs: u64) -> HistoryKey {
        HistoryKey {
            root: self.root.to_path_buf(),
            head: self.head.to_owned(),
            upstream: (self.upstream).map(|(name, oid)| (name.to_owned(), oid.to_owned())),
            refs,
        }
    }
}

/// 后台读取提交历史的结果。
pub(super) enum HistoryOutcome {
    /// 按 `HistoryKey` 读取到的提交，或失败的原因。
    Loaded(Box<HistoryKey>, Result<Recent, String>),
    /// 读取到的完整提交说明，复制到剪贴板。
    Message(Result<String, String>),
    /// 读取到的提交（提交号）的详情，用于悬停时的卡片。
    Card(String, Result<CommitCard, String>),
}

/// 提交历史一组的状态。
pub(crate) struct History {
    /// 读取到的提交；尚未读取、仓库还没有提交或读取失败时为 `None`，这时不显示这一组。
    pub(crate) recent: Option<Recent>,
    /// 已读取或正在读取的依据；与当前的依据不同时重新读取。
    asked: Option<HistoryKey>,
    /// 是否有一次读取在后台进行；同一时刻只进行一次，结束后按最新的依据补读。
    running: bool,
    /// 引用的版本：分支、标签变化时加 1，提交上的引用随之重新读取。
    refs: u64,
    /// 展开状态和高度的过渡，默认展开。
    pub(crate) drawer: Drawer,
    /// 组内滚动的距离（逻辑像素）。
    pub(crate) scroll: f32,
    /// 这一组是因为显示了 Git 面板才收起的：面板隐藏时重新展开；期间手动展开、收起后清除。
    folded_for_panel: bool,
    /// `recent` 的版本，每换一次加 1。
    version: u64,
    /// 绘制时记下的 `version`：之后 `recent` 换过，记下的下标就对不上了，点击不处理。
    drawn: u64,
    /// 各提交的详情卡片的读取状态，按提交号。
    cards: HashMap<String, CardState>,
    /// 详情卡片换到另一个提交时从原来的行滑过去：开始时相对新位置的纵向偏移（逻辑像素）和
    /// 时钟；不在滑动中时为 `None`。
    pub(crate) card_slide: Option<(f32, crate::app::animation::Clock)>,
}

impl Default for History {
    fn default() -> Self {
        History {
            recent: None,
            asked: None,
            running: false,
            refs: 0,
            drawer: Drawer::new(true),
            scroll: 0.0,
            folded_for_panel: false,
            version: 0,
            drawn: 0,
            cards: HashMap::new(),
            card_slide: None,
        }
    }
}

impl History {
    /// 换了仓库时的新状态：只保留展开状态、高度和是否因 Git 面板而收起，提交按新仓库重新读取。
    /// 此前仓库仍在进行的读取结束时按依据不符丢弃（见 [`App::history_arrived`]）。
    pub(super) fn switched(&self) -> History {
        History {
            drawer: Drawer {
                motion: None,
                ..self.drawer
            },
            folded_for_panel: self.folded_for_panel,
            ..History::default()
        }
    }

    /// 绘制完这一组后调用：记下绘制时的版本。
    pub(crate) fn hits_drawn(&mut self) {
        self.drawn = self.version;
    }

    /// 绘制时记下的各提交的下标是否仍然有效。
    pub(in crate::app) fn hits_current(&self) -> bool {
        self.drawn == self.version
    }

    /// 分支或标签变化了：下次绘制时重新读取。
    pub(in crate::app) fn outdate(&mut self) {
        self.refs += 1;
    }

    /// 返回第 `i` 个提交和它的位置。
    pub(crate) fn commit(&self, i: usize) -> Option<(&Commit, Side)> {
        let recent = self.recent.as_ref()?;
        Some((recent.commits.get(i)?, *recent.sides.get(i)?))
    }
}

impl App {
    /// 返回提交历史一组展开时标题下面的各行：各个提交，还有更早的提交时最后是「在日志中查看更早
    /// 的提交」。
    pub(crate) fn history_lines(&self) -> Vec<HistoryLine> {
        let Some(recent) = self.git.commit.history.recent.as_ref() else {
            return Vec::new();
        };
        let mut lines: Vec<HistoryLine> =
            (0..recent.commits.len()).map(HistoryLine::Commit).collect();
        if recent.more {
            lines.push(HistoryLine::More);
        }
        lines
    }

    /// 鼠标从提交历史的一个提交移到（或随列表滚动换到）另一个提交时，返回详情卡片是否直接换成
    /// 新提交的卡片：卡片已显示时不重新计时，从原来的行滑到新的行；否则返回 `false`。
    pub(crate) fn move_history_card(&mut self, from: Hover, to: Hover) -> bool {
        let commit = |hover| match hover {
            Hover::Commit(CommitTarget::History(HistoryTarget::Commit(i))) => Some(i),
            _ => None,
        };
        let (Some(from), Some(to)) = (commit(from), commit(to)) else {
            return false;
        };
        if !self.overlays.tooltip.shown {
            return false;
        }
        // 各行等高、连续排列，原来那一行与新的一行相距的行数即卡片要移动的距离。
        let offset = (from as f32 - to as f32) * COMMIT_ROW;
        let slide = self.clock(crate::app::animation::HISTORY_CARD_SLIDE);
        self.git.commit.history.card_slide = slide.map(|clock| (offset, clock));
        true
    }

    /// 每帧绘制提交视图之前调用：依据变化时在后台重新读取，终点高度变化时开始过渡，滚动位置
    /// 限制在范围内。
    pub(crate) fn track_history(&mut self) {
        self.load_history();
        self.track_history_card();
        let (settled, clock) = (self.history_settled_height(), self.section_clock());
        self.git.commit.history.drawer.track(settled, clock);
        let scroll = self.git.commit.history.scroll;
        self.set_pane_scroll(ScrollTarget::CommitHistory, scroll);
    }

    /// 当前的读取依据；设置中关闭了提交历史、没有仓库、尚未读到状态或还没有提交时为 `None`。
    /// 每帧调用，只借用不复制。
    fn history_basis(&self) -> Option<Basis<'_>> {
        if self.config.git_commit_history != Some(true) {
            return None;
        }
        let root = self.git.root.as_deref()?;
        let head = self.git.status.as_ref()?.oid.as_deref()?;
        let branch = self.git.branches.iter().find(|b| b.current && !b.remote);
        let upstream = branch.and_then(|b| {
            let name = b.upstream.as_deref()?;
            let remote = (self.git.branches.iter()).find(|r| r.remote && r.name == name)?;
            Some((name, remote.oid.as_str()))
        });
        Some(Basis {
            root,
            head,
            upstream,
        })
    }

    /// 依据与已读取的不同时在后台重新读取；已有一次读取在进行时等它结束。
    fn load_history(&mut self) {
        let basis = self.history_basis();
        let history = &self.git.commit.history;
        let same = match (basis, history.asked.as_ref()) {
            (None, None) => true,
            (Some(basis), Some(key)) => basis.matches(key, history.refs),
            _ => false,
        };
        let key = basis.map(|basis| basis.key(history.refs));
        let history = &mut self.git.commit.history;
        if same || history.running {
            return;
        }
        let Some(key) = key else {
            history.asked = None;
            history.recent = None;
            history.version += 1;
            return;
        };
        history.asked = Some(key.clone());
        history.running = true;
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let upstream = key.upstream.as_ref().map(|(name, _)| name.as_str());
            let recent = git.and_then(|git| git.recent(root, upstream));
            let _ = sender.send(Outcome::History(HistoryOutcome::Loaded(
                Box::new(key),
                recent,
            )));
        });
    }

    /// 后台读取历史的结果到达。
    pub(super) fn history_arrived(&mut self, outcome: HistoryOutcome) {
        match outcome {
            HistoryOutcome::Loaded(key, recent) => {
                let history = &mut self.git.commit.history;
                // 不是正在进行的那次读取（换仓库前发出的）：丢弃，不影响此后发出的读取。读取
                // 失败（例如超时）时保留上一次的结果，直到依据再次变化。
                if history.asked.as_ref() != Some(&*key) {
                    return;
                }
                history.running = false;
                if let Ok(recent) = recent {
                    // 只留仍在列出的提交的详情卡片。
                    let listed = |oid: &String| recent.commits.iter().any(|c| &c.oid == oid);
                    history.cards.retain(|oid, _| listed(oid));
                    history.recent = Some(recent).filter(|r| !r.commits.is_empty());
                    history.version += 1;
                }
            }
            HistoryOutcome::Message(Ok(message)) => {
                self.flash(t!("git-log-copied-message"));
                self.set_clipboard(message, false);
            }
            HistoryOutcome::Message(Err(e)) => self.flash(e),
            HistoryOutcome::Card(oid, card) => self.history_card_arrived(oid, card),
        }
        self.request_redraw();
    }

    /// 返回提交历史一组的高度（逻辑像素），过渡中按时钟插值；没有提交时为 0。
    pub(in crate::app) fn history_section_height(&self) -> f32 {
        let history = &self.git.commit.history;
        history.drawer.height(self.history_settled_height())
    }

    /// 返回提交历史一组展开时露出的行数，不多于 [`HISTORY_ROWS`]。
    fn history_open_rows(&self) -> usize {
        self.history_line_count().min(HISTORY_ROWS)
    }

    /// 返回 [`App::history_lines`] 的行数，不排出各行。
    pub(in crate::app) fn history_line_count(&self) -> usize {
        let Some(recent) = self.git.commit.history.recent.as_ref() else {
            return 0;
        };
        recent.commits.len() + usize::from(recent.more)
    }

    /// 返回提交历史一组不在过渡中时的高度（逻辑像素）。
    fn history_settled_height(&self) -> f32 {
        if self.git.commit.history.recent.is_none() {
            return 0.0;
        }
        if self.git.commit.history.drawer.open {
            DRAWER_CLOSED + self.history_open_rows() as f32 * COMMIT_ROW
        } else {
            DRAWER_CLOSED
        }
    }

    /// 返回提交历史一组标题上箭头的角度比例，见 [`Drawer::chevron`]。
    pub(crate) fn history_chevron(&self) -> f32 {
        let span = self.history_open_rows() as f32 * COMMIT_ROW;
        let history = &self.git.commit.history;
        history.drawer.chevron(self.history_settled_height(), span)
    }

    /// 返回组内最大的滚动距离（逻辑像素）：各行的总高度减去露出的高度，露出的行数受
    /// [`HISTORY_ROWS`] 和侧栏的高度限制。
    ///
    /// 收起和过渡中按展开时露出的行数算：收起的过渡中各行不跳回顶部，再展开时停在原来的位置。
    pub(crate) fn history_max_scroll(&self) -> f32 {
        let lines = self.history_line_count();
        let mut shown = lines.min(HISTORY_ROWS) as f32 * COMMIT_ROW;
        let history = &self.git.commit.history;
        if history.drawer.open && history.drawer.motion.is_none() {
            shown = shown.min(self.history_rows_area().map_or(shown, |rows| rows.h));
        }
        (lines as f32 * COMMIT_ROW - shown).max(0.0)
    }

    /// 返回提交历史一组中各行所在的范围（标题下面、下沿留白上面）；这一组不显示时为 `None`。
    pub(crate) fn history_rows_area(&self) -> Option<Rect> {
        let area = self.commit_parts().history?;
        let top = area.y + DRAWER_TOP + COMMIT_ROW;
        let h = (area.bottom() - STASH_GAP - top).max(0.0);
        Some(Rect::new(area.x, top, area.w, h))
    }

    /// Git 面板显示（`open`）或隐藏了：显示时展开着的提交历史收起（完整的日志已在眼前），隐藏
    /// 时把因此收起的重新展开。手动展开、收起过的不再自动恢复。
    pub(in crate::app) fn fold_history_for_panel(&mut self, open: bool) {
        let history = &self.git.commit.history;
        if history.recent.is_none() {
            return;
        }
        if open && history.drawer.open {
            self.toggle_history();
            self.git.commit.history.folded_for_panel = true;
        } else if !open && history.folded_for_panel {
            if !history.drawer.open {
                self.toggle_history();
            }
            self.git.commit.history.folded_for_panel = false;
        }
    }

    /// 收起或展开提交历史一组。
    fn toggle_history(&mut self) {
        let from = self.history_section_height();
        let clock = self.clock(crate::app::animation::TREE_ANIMATION);
        self.git.commit.history.drawer.toggle(from, clock);
        self.git.commit.history.drawer.settled = self.history_settled_height();
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
    }

    /// 处理提交历史一组中的点击。
    pub(super) fn press_history(&mut self, target: HistoryTarget) {
        match target {
            HistoryTarget::Title => {
                self.git.commit.history.folded_for_panel = false;
                self.toggle_history();
            }
            HistoryTarget::Log => self.git_show_log(),
            HistoryTarget::Commit(i) => {
                let Some((commit, _)) = self.git.commit.history.commit(i) else {
                    return;
                };
                let (oid, subject) = (commit.oid.clone(), commit.subject.clone());
                let parent = commit.parents.first().cloned();
                let (x, y) = self.pointer.mouse;
                if self.click_count(x, y) >= 2 {
                    self.git_show_log_at(&oid);
                } else {
                    self.open_commit_changes(&oid, parent.as_deref(), &subject);
                }
            }
            // 选中列出的最后一个提交：日志从该提交起继续查看。
            HistoryTarget::More => {
                let recent = self.git.commit.history.recent.as_ref();
                match recent.and_then(|r| r.commits.last()).map(|c| c.oid.clone()) {
                    Some(oid) => self.git_show_log_at(&oid),
                    None => self.git_show_log(),
                }
            }
        }
        self.request_redraw();
    }

    /// 右键点在提交历史的一个提交上：返回它的提交号。
    pub(in crate::app) fn history_context_target(&self, x: f32, y: f32) -> Option<String> {
        let current = self.git.commit.hits_generation == self.git.generation;
        if !self.commit_shown() || !current || !self.git.commit.history.hits_current() {
            return None;
        }
        let CommitTarget::History(HistoryTarget::Commit(i)) =
            crate::layout::hit(&self.hits.commit, x, y)?
        else {
            return None;
        };
        let (commit, _) = self.git.commit.history.commit(i)?;
        Some(commit.oid.clone())
    }

    /// 提交历史中提交号为 `oid` 的提交及其位置。
    pub(in crate::app) fn history_commit(&self, oid: &str) -> Option<(Commit, Side)> {
        let recent = self.git.commit.history.recent.as_ref()?;
        let i = recent.commits.iter().position(|c| c.oid == oid)?;
        Some((recent.commits[i].clone(), recent.sides[i]))
    }

    /// 在后台读取提交 `oid` 的完整说明，读取到后复制到剪贴板。
    pub(in crate::app) fn copy_history_message(&mut self, oid: String) {
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let cancel = winc_editor::format::Cancel::default();
            let message = git.and_then(|git| git.commit_message(root, &oid, &cancel));
            let _ = sender.send(Outcome::History(HistoryOutcome::Message(message)));
        });
    }
}
