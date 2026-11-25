//! 并行搜索一批文件：一个线程取出要搜索的文件，若干线程读文件并匹配，匹配总数有上限。

use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Condvar, Mutex, PoisonError};
use std::thread;

use winc_core::Lines;

use crate::lines::{LineMatch, search_lines};
use crate::matcher::Matcher;
use crate::query::Filter;
use crate::scan::search_file;
use crate::walk::{
    dependency_folders, is_listed_file, nested_repositories, walker, walker_with_ignored,
};

/// 并行读文件的线程数上限（个，含取文件的线程）。
///
/// 每个线程同一时刻最多处理一个文件；读文件和匹配受磁盘与内存带宽所限，更多线程收益很小。
pub const MAX_THREADS: usize = 4;
/// 队列中待搜索文件的最大数量（个，取自调参）。
///
/// 遍历目录比读文件快，不加限制时整个目录的路径都会堆积在队列里；该值只影响队列的内存占用。
/// 队列已满时的处理见 [`Queue::push`]。
const QUEUE_MAX: usize = 1024;

/// 搜索哪些文件。
pub enum Scope {
    /// 遍历这些目录下的全部文件，按 `filter` 过滤；`ignored` 为是否也搜索被忽略的文件。
    ///
    /// 不搜索被忽略的文件时，其中的嵌套版本库（见 [`nested_repositories`]）按各自的忽略规则搜索。
    Tree {
        /// 依次遍历的目录，各自按所在的仓库判断忽略规则。
        dirs: Vec<PathBuf>,
        /// 「包含」「排除」的规则。
        filter: Filter,
        /// 是否也搜索被 `.gitignore` 等规则忽略的文件。
        ignored: bool,
    },
    /// 只搜索这些目录下的依赖目录（见 [`dependency_folders`]），其中被忽略的文件也搜索。
    Dependencies(Vec<PathBuf>),
    /// 只搜索这些文件。
    Files(Vec<PathBuf>),
}

impl Scope {
    /// 依次给出要搜索的文件；目录边遍历边给出。`stop` 置位后查找嵌套版本库提前结束。
    pub fn paths<'a>(&'a self, stop: &'a AtomicBool) -> Box<dyn Iterator<Item = PathBuf> + 'a> {
        let files = |builder: ignore::WalkBuilder| {
            (builder.build().flatten())
                .filter(|entry| {
                    (entry.file_type()).is_some_and(|kind| is_listed_file(kind, entry.path()))
                })
                .map(ignore::DirEntry::into_path)
        };
        match self {
            Scope::Tree {
                dirs,
                filter,
                ignored,
            } => Box::new(dirs.iter().flat_map(move |dir| {
                let walk = move |root: &Path| {
                    let mut builder = if *ignored {
                        walker_with_ignored(root)
                    } else {
                        walker(root)
                    };
                    builder.overrides(filter.pruning());
                    files(builder).filter(|path| filter.includes(path))
                };
                // 目录走完之后再找嵌套版本库，先给出的结果不必等待查找。
                let nested = std::iter::once(*ignored).flat_map(move |ignored| {
                    let roots = match ignored {
                        true => Vec::new(),
                        false => nested_repositories(dir, stop),
                    };
                    roots.into_iter().flat_map(move |root| walk(&root))
                });
                walk(dir).chain(nested)
            })),
            Scope::Dependencies(dirs) => Box::new(
                (dirs.iter())
                    .flat_map(|dir| dependency_folders(dir))
                    .flat_map(move |dir| files(walker_with_ignored(&dir))),
            ),
            Scope::Files(files) => Box::new(files.iter().cloned()),
        }
    }
}

/// 一次搜索共用的匹配数上限与停止标志。
pub struct Budget {
    limit: usize,
    /// 已记入的匹配数，由多个线程共用。
    count: AtomicUsize,
    /// 是否因达到上限而停止，即结果不全。
    truncated: AtomicBool,
    /// 搜索是否应当停止：被取消，或达到了上限。
    stop: AtomicBool,
}

impl Budget {
    /// 最多记入 `limit` 处匹配。
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            count: AtomicUsize::new(0),
            truncated: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        }
    }

    /// 返回离上限还差的匹配数（处）；已经达到上限时返回 `None`。
    ///
    /// 单个文件收满该数量之后不必再往下找：单个文件（例如压缩后的大文件）可能含远多于上限的匹配，
    /// 每处还带一段预览。
    pub fn remaining(&self) -> Option<usize> {
        let left = self.limit.saturating_sub(self.count.load(Relaxed));
        (left > 0).then_some(left)
    }

    /// 记入一个文件的匹配 `matches`，截去超出上限的部分（可能截空）；达到上限时记下结果不全，
    /// 让各线程停止，返回 `Break`。
    pub fn take(&self, matches: &mut Vec<LineMatch>) -> ControlFlow<()> {
        let before = self.count.fetch_add(matches.len(), Relaxed);
        let left = self.limit.saturating_sub(before);
        if matches.len() < left {
            return ControlFlow::Continue(());
        }
        matches.truncate(left);
        self.truncated.store(true, Relaxed);
        self.stop();
        ControlFlow::Break(())
    }

    /// 记下结果不全：别处（远端主机上）的搜索达到了上限。
    pub fn mark_truncated(&self) {
        self.truncated.store(true, Relaxed);
    }

    /// 是否因达到上限而停止，即结果不全。
    pub fn truncated(&self) -> bool {
        self.truncated.load(Relaxed)
    }

    /// 让搜索尽快停止。
    pub fn stop(&self) {
        self.stop.store(true, Relaxed);
    }

    /// 搜索是否应当停止。
    pub fn stopped(&self) -> bool {
        self.stop.load(Relaxed)
    }

    /// 停止标志本身，交给按块读文件的循环随时查看。
    pub fn stop_flag(&self) -> &AtomicBool {
        &self.stop
    }
}

/// 搜索文件的一个线程：各自持有匹配器的副本和读文件的缓冲。
pub struct Searcher<'a> {
    /// 每个线程各一个：正则的每个副本有自己的缓存，多个线程共用一个时会争用缓存。
    matcher: Matcher,
    /// 读文件的缓冲，同一线程复用。
    buffer: Vec<u8>,
    budget: &'a Budget,
}

impl<'a> Searcher<'a> {
    /// 按 `matcher` 搜索，匹配数计入 `budget`。
    pub fn new(matcher: &Matcher, budget: &'a Budget) -> Self {
        Self {
            matcher: matcher.clone(),
            buffer: Vec::new(),
            budget,
        }
    }

    /// 搜索磁盘上的文件 `path`，最多收到上限为止；没有匹配、二进制或无法读取时返回 `None`。
    /// 第二项表示是否有字节按 U+FFFD 替换字符解码。
    pub fn file(&mut self, path: &Path) -> Option<(Vec<LineMatch>, bool)> {
        let limit = self.budget.remaining()?;
        let (matches, lossy) = search_file(
            &self.matcher,
            &mut self.buffer,
            path,
            limit,
            self.budget.stop_flag(),
        )?;
        (!matches.is_empty()).then_some((matches, lossy))
    }

    /// 搜索内存中的文本（有未保存修改的文档）；没有匹配时返回 `None`。
    pub fn text(&self, text: &impl Lines) -> Option<Vec<LineMatch>> {
        let matches = search_lines(&self.matcher, text, self.budget.remaining()?);
        (!matches.is_empty()).then_some(matches)
    }
}

/// 并行搜索 `paths` 中的文件：当前线程取出文件放进队列，另有至多 [`MAX_THREADS`] − 1 个线程从
/// 队列取文件交给 `visit`，取完后当前线程也参与。`visit` 返回 `Break` 时停止整个搜索。
///
/// 文件读取和匹配是主要开销，因此由多个线程并行处理；搜索被停止（[`Budget::stop`]）后不再
/// 处理新的文件。
pub fn search_parallel(
    paths: impl Iterator<Item = PathBuf>,
    matcher: &Matcher,
    budget: &Budget,
    visit: impl Fn(&mut Searcher<'_>, &Path) -> ControlFlow<()> + Sync,
) {
    let threads = thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(MAX_THREADS);
    let queue = Queue::default();
    let handle = |searcher: &mut Searcher<'_>, path: &Path| {
        if !budget.stopped() && visit(searcher, path).is_break() {
            budget.stop();
        }
    };

    thread::scope(|scope| {
        for _ in 1..threads {
            scope.spawn(|| {
                let mut searcher = Searcher::new(matcher, budget);
                while let Some(path) = queue.pop() {
                    handle(&mut searcher, &path);
                }
            });
        }

        let mut searcher = Searcher::new(matcher, budget);
        for path in paths {
            if budget.stopped() {
                break;
            }
            if let Some(path) = queue.push(path) {
                handle(&mut searcher, &path);
            }
        }
        queue.close();
        // 取完之后当前线程也从队列取文件处理。
        while let Some(path) = queue.pop() {
            handle(&mut searcher, &path);
        }
    });
}

/// 取文件的线程放入、读文件的线程取走的待搜索文件队列。
///
/// 取完文件后关闭队列，取空后读文件的线程退出。
#[derive(Default)]
struct Queue {
    state: Mutex<(VecDeque<PathBuf>, bool)>,
    ready: Condvar,
}

impl Queue {
    /// 把 `path` 放入队列；队列已满时取出最早的一个返回，由取文件的线程自己处理。
    ///
    /// 这样即使别的线程都在忙，取文件的线程也总有文件可处理。
    fn push(&self, path: PathBuf) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let full = (state.0.len() >= QUEUE_MAX)
            .then(|| state.0.pop_front())
            .flatten();
        state.0.push_back(path);
        self.ready.notify_one();
        full
    }

    /// 关闭队列：不再放入新文件，取空后 [`Queue::pop`] 返回 `None`。
    fn close(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).1 = true;
        self.ready.notify_all();
    }

    /// 取出一个文件；队列为空时阻塞等待，队列已关闭且取空时返回 `None`。
    fn pop(&self) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(path) = state.0.pop_front() {
                return Some(path);
            }
            if state.1 {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}
