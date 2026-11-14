//! Git 日志标签的数据：取回来的提交、每一行的提交图、选中的提交和它的详情。
//!
//! 日志与并排差异都是只读文档（[`crate::Document::log`]），滚动、标签栏都走文档的通用逻辑：
//! 一个提交一行，`scroll_line` 就是滚到第几个提交。提交分批取回（[`LogView::begin`] 要一批，
//! [`LogView::finish`] 接上交回来的），提交图从上一批继续往下算。取提交、取详情都在后台线程
//! 里完成，本模块只保存结果和取提交的进度。

use winc_core::graph::{Graph, GraphRow};

use crate::git::{Commit, CommitFile, LogQuery, Ref};

/// 第一批取回的提交数（个）。
///
/// 之后每批取已经取回的条数（最多 [`MAX_BATCH`] 个），各批从同一个开着的 `git log` 继续读
/// （[`crate::git::Git::log_stream`]）；批量随之翻倍，翻很深的历史时后台往返次数更少。
pub const BATCH: usize = 500;
/// 一批最多取的提交数（个）。
///
/// 一条记录几百字节，两万条离一次输出的上限（64 MiB）还很远。
pub const MAX_BATCH: usize = 20_000;

/// 日志标签的数据。
#[derive(Debug, Default)]
pub struct LogView {
    /// 已经取回来的提交，孩子在前。
    pub commits: Vec<Commit>,
    /// 每个提交那一行的提交图，和 `commits` 一一对应。
    pub graph: Vec<GraphRow>,
    /// `HEAD` 所在的提交在 `commits` 里的下标，取回来时记下（见 [`Self::head_row`]）。
    head: Option<usize>,
    /// 提交图的泳道状态，跨批次保留。
    lanes: Graph,
    /// 已经取完了（最后一批不满所要的个数）。
    pub complete: bool,
    /// 后台取提交取到哪一步了。
    fetch: Fetch,
    /// 取提交失败的原因（比如仓库还没有提交）。
    pub error: Option<String>,
    /// 选中的提交（`commits` 的下标）。
    pub selected: Option<usize>,
    /// 选中的提交的详情；取回来之前只有提交号。
    pub details: Option<Details>,
    /// 换选中项之前已经取齐的详情。
    ///
    /// 新的详情取回来之前，详情栏继续显示之前已取齐的详情，避免先换说明、再留空、再分几步显示
    /// 文件列表。
    pub previous: Option<Details>,
    /// 详情一栏滚动的距离（逻辑像素）。
    pub details_scroll: f32,
    /// 重取的次数；后台请求交回来时该值不匹配就丢弃。
    generation: u64,
    /// 筛选条件。
    pub query: LogQuery,
    /// 正在比较两个版本（两个提交，或者两个分支）：详情一栏换成两者之间改了的文件。
    pub compare: Option<Compare>,
    /// 重取了、新的第一批还没到。
    ///
    /// `commits`、`graph` 还是重取之前的内容，仍然可以显示和点击，第一批到达时整体替换
    /// （`LogView::append`）。
    pub stale: bool,
    /// 最近看过或预取过的提交详情，新的在后面，最多 [`DETAILS_CACHE`] 个；来回选择、用键盘
    /// 上下移动时不必再等后台。
    cache: Vec<Details>,
}

/// 详情缓存最多保留的提交数（个）。
///
/// 一屏上下翻动、用键盘连续经过的提交数远小于该值；每条详情含提交说明和文件列表，不宜无上限
/// 地积累。
pub const DETAILS_CACHE: usize = 64;

/// 后台取提交取到哪一步了。
///
/// 同一时刻只运行一个 `git log`，作废的请求也算在内：输入筛选文字时不会每个字都启动一个
/// 进程。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Fetch {
    /// 没有在取。
    #[default]
    Idle,
    /// 后台有一个请求在跑。
    Running {
        /// 发出请求时是第几次重取（[`LogView::reset`]）。
        generation: u64,
        /// 这一批要几个：返回的条数少于该数量时表示已取完。
        max: usize,
        /// 运行期间又重取了、需要按新的条件取：这一批结果交回来之后丢弃，再取一次。
        refetch: bool,
    },
}

/// 要后台取的一批提交：前 `skip` 个已经取回来了，本次再取最多 `max` 个。
///
/// 交回结果时连同 `generation` 一并交给 [`LogView::finish`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// 已经取回来几个提交；为 0 时从头开一个 `git log`。
    pub skip: usize,
    /// 最多取几个。
    pub max: usize,
    /// 发出请求时是第几次重取。
    pub generation: u64,
}

/// 一批提交交回来的结果，见 [`LogView::finish`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// 接上了（或者记下了失败的原因）。
    ///
    /// `first`：这是重取之后、或者刚打开时的第一批。
    Applied {
        /// 是不是第一批。
        first: bool,
    },
    /// 这一批作废了，运行期间又重取过：按现在的条件再取一次（[`LogView::begin`]）。
    Refetch,
    /// 这一批作废了，不再使用。
    Dropped,
}

/// 比较两个版本：从 `from`（旧的）到 `to`（新的）改了哪些文件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compare {
    /// 旧的一边：提交号或分支名。
    pub from: String,
    /// 新的一边。
    pub to: String,
    /// 改了的文件；取回来之前为 `None`。
    pub files: Option<Vec<CommitFile>>,
    /// 取文件列表失败的原因。
    pub error: Option<String>,
}

/// 选中的提交的详情。
#[derive(Debug, Clone, Default)]
pub struct Details {
    /// 提交号。
    pub oid: String,
    /// 完整的说明。
    pub message: Option<String>,
    /// 改动的文件（和 `base` 比）。
    pub files: Option<Vec<CommitFile>>,
    /// 改动的文件对照的版本：第一个父提交，根提交为 `None`。按路径筛选的日志里列表上的父提交
    /// 是改写过的（[`crate::git::LogQuery::rewrites_parents`]），这里是真正的父提交。
    pub base: Option<String>,
    /// 取详情失败的原因。
    pub error: Option<String>,
    /// 包含该提交的本地分支和远程分支（[`crate::git::Git::containing_branches`]），本地的在前；
    /// 取回来之前为 `None`。分支随时会变，重取日志时作废。
    pub branches: Option<Vec<String>>,
}

impl LogView {
    /// 空的日志，等着取第一批。
    pub fn new() -> Self {
        LogView::default()
    }

    /// 返回是否正在后台取当前条件下的提交（已作废的请求仍在运行、正排队等再取时也算）。
    pub fn is_loading(&self) -> bool {
        matches!(self.fetch, Fetch::Running { generation, refetch, .. }
            if refetch || generation == self.generation)
    }

    /// 要取下一批时调用：返回要后台取的那一批，后台把结果交给 [`LogView::finish`]。
    ///
    /// 已经取完或正在取时返回 `None`；后台跑着重取之前的请求时也返回 `None`，并记下跑完之后再取。
    pub fn begin(&mut self) -> Option<Request> {
        if self.complete || self.is_loading() {
            return None;
        }
        if let Fetch::Running { refetch, .. } = &mut self.fetch {
            *refetch = true;
            return None;
        }
        let skip = self.next_skip();
        let max = skip.clamp(BATCH, MAX_BATCH);
        self.fetch = Fetch::Running {
            generation: self.generation,
            max,
            refetch: false,
        };
        Some(Request {
            skip,
            max,
            generation: self.generation,
        })
    }

    /// 后台交回了第 `generation` 次重取时要的那一批：没有作废时接上（失败时记下原因，不再向下取）；
    /// 作废了时丢弃；运行期间又重取过的再取一次。
    pub fn finish(&mut self, generation: u64, result: Result<Vec<Commit>, String>) -> Arrival {
        let Fetch::Running { max, refetch, .. } = std::mem::take(&mut self.fetch) else {
            return Arrival::Dropped;
        };
        if generation != self.generation {
            return if refetch {
                Arrival::Refetch
            } else {
                Arrival::Dropped
            };
        }
        let first = self.stale || self.commits.is_empty();
        match result {
            Ok(commits) => self.append(commits, max),
            Err(e) => {
                self.drop_stale();
                self.complete = true;
                self.error = Some(e);
            }
        }
        Arrival::Applied { first }
    }

    /// 接上取回来的一批提交，算好它们的提交图（按说明、作者或文件筛选时不画图）。
    ///
    /// 取回的条数不满 `max` 个时说明已经取完了。
    fn append(&mut self, commits: Vec<Commit>, max: usize) {
        if std::mem::take(&mut self.stale) {
            self.drop_stale();
        }
        self.complete = commits.len() < max;
        let graphless = self.query.graphless();
        for (i, commit) in commits.iter().enumerate() {
            if self.head.is_none()
                && commit
                    .refs
                    .iter()
                    .any(|r| matches!(r, Ref::Head(_) | Ref::Detached))
            {
                self.head = Some(self.commits.len() + i);
            }
            let row = if graphless {
                GraphRow::default()
            } else {
                self.lanes.push(&commit.oid, &commit.parents)
            };
            self.graph.push(row);
        }
        self.commits.extend(commits);
    }

    /// 从头再取（`HEAD`、分支、筛选条件变了）。
    ///
    /// 已经取回来的提交先留着画（`stale`），新的第一批到了再换掉；选中的提交和它的详情留着，
    /// 取回来之后按提交号重新选上它。后台还在跑的请求就此作废，交回来时丢弃。
    pub fn reset(&mut self) {
        self.generation += 1;
        self.stale = !self.commits.is_empty();
        self.complete = false;
        self.error = None;
        if self.selected.is_none() {
            self.details = None;
        }
        // 重取多半是分支变了（提交、拉取、切换）：包含各提交的分支重新查。
        let details = self.details.iter_mut().chain(&mut self.previous);
        for d in details.chain(&mut self.cache) {
            d.branches = None;
        }
    }

    /// 记下包含提交 `oid` 的分支：选中的、之前显示的和缓存里的详情一并更新。
    pub fn set_branches(&mut self, oid: &str, branches: &[String]) {
        let details = self.details.iter_mut().chain(&mut self.previous);
        for d in details.chain(&mut self.cache).filter(|d| d.oid == oid) {
            d.branches = Some(branches.to_vec());
        }
    }

    /// 丢弃重取之前的提交（新的第一批到了，或者重取失败了）。
    fn drop_stale(&mut self) {
        self.stale = false;
        self.commits.clear();
        self.graph.clear();
        self.head = None;
        self.lanes = Graph::default();
        self.selected = None;
    }

    /// 下一批从第几个提交继续取；重取时从头。
    fn next_skip(&self) -> usize {
        if self.stale { 0 } else { self.commits.len() }
    }

    /// `HEAD` 所在的提交在 `commits` 里的下标；还没取到时返回 `None`。
    ///
    /// 日志每帧都要知道 `HEAD` 在哪一行（画它的泳道、判断它是不是第一行），取回来时记下，
    /// 不必每帧在上万个提交里找。
    pub fn head_row(&self) -> Option<usize> {
        self.head
    }

    /// 返回缓存里已取齐的提交 `oid` 的详情；没有缓存时返回 `None`。
    pub fn cached(&self, oid: &str) -> Option<&Details> {
        self.cache.iter().rev().find(|d| d.oid == oid)
    }

    /// 记下一个取回来的详情；同一个提交的旧记录换掉，超过 [`DETAILS_CACHE`] 个时丢弃最早的。
    pub fn remember(&mut self, details: Details) {
        self.cache.retain(|d| d.oid != details.oid);
        if self.cache.len() >= DETAILS_CACHE {
            self.cache.remove(0);
        }
        self.cache.push(details);
    }

    /// 返回显示行数：一个提交一行，没有提交时也占一行（写提示）。
    pub fn row_count(&self) -> usize {
        self.commits.len().max(1)
    }

    /// 选中的提交号。
    pub fn selected_oid(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.commits.get(i))
            .map(|c| c.oid.as_str())
    }

    /// 返回第 `from..to` 行里提交图最宽需要的车道数。
    pub fn graph_width(&self, rows: std::ops::Range<usize>) -> usize {
        let end = rows.end.min(self.graph.len());
        self.graph[rows.start.min(end)..end]
            .iter()
            .map(|r| r.width)
            .max()
            .unwrap_or(1)
    }
}

#[cfg(test)]
mod tests;
