//! 后台任务共用的线程池：构建 ⌘P 的文件表、更新文件表、全局搜索、格式化前的准备都经 [`spawn`]
//! 提交。
//!
//! 线程按需创建，最多 [`MAX_THREADS`] 个，空闲 [`KEEP_ALIVE`] 后退出，成批到达的任务因此复用同一个
//! 线程；线程数未达上限时任务之间不会互相等待。需要多线程协作的工作用 [`crew`] 或 [`split`]，帮手
//! 也来自本池，调用线程自己参与；阻塞在子进程管道上的工作不用本池，由 [`spawn_blocking`] 分别建
//! 线程。

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

/// 线程池同时存在的最大线程数（个）。
///
/// 不随 CPU 核心数调整，取自调参：线程池的任务以读取若干个小文件为主，单个任务并不占满一个核心，
/// 取值超过常见的核心数（8 个）后，增加线程只会增加调度开销。该上限约束池中同时存在的全部线程，
/// 经 [`spawn_blocking`] 单独创建的不受约束。
pub const MAX_THREADS: usize = 8;
/// 线程空闲后保留的时长（10 秒）。
///
/// 取自调参：成批提交的任务通常在数秒内到达，取值不小于批内的间隔时，同一批任务复用同一个线程；
/// 取值过大时，线程空闲退出的时刻相应推迟，空闲线程占用的内存保留更久。
pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

type Task = Box<dyn FnOnce() + Send>;

/// 执行互不依赖的 `first` 和 `second`，返回两者的结果。
///
/// `together` 为真时两者同时进行（`first` 在另一个线程上），用于各自要等远端应答的操作：
/// 依次执行时总耗时是两次往返之和。为假时在当前线程上依次执行，不启动线程；当前线程不允许
/// 等待远端的应答（界面线程）时也是如此，各操作照常立即返回，不借别的线程去等。
pub(crate) fn both<A: Send, B>(
    together: bool,
    first: impl FnOnce() -> A + Send,
    second: impl FnOnce() -> B,
) -> (A, B) {
    if !together || !winc_backend::remote::may_block() {
        return (first(), second());
    }
    std::thread::scope(|scope| {
        let first = scope.spawn(first);
        let second = second();
        let first = first
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
        (first, second)
    })
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(MAX_THREADS, KEEP_ALIVE))
}

/// 把 `task` 提交给后台线程池执行，不等待其完成。
pub fn spawn(task: impl FnOnce() + Send + 'static) {
    pool().spawn(Box::new(task));
}

/// 在单独创建的线程上执行 `task`，不等待其完成。
///
/// 用于阻塞在子进程管道、等待子进程退出或等待网络的工作：这类工作的耗时没有上限（`git push`
/// 等待网络、提交前的钩子运行测试、格式化工具长时间不返回），放进线程池会占住池中的线程，
/// 读文件、保存、备份等任务只能排队。无法创建线程时（系统资源耗尽）改交线程池执行。
pub fn spawn_blocking(task: impl FnOnce() + Send + 'static) {
    fn take(slot: &Mutex<Option<Task>>) -> Option<Task> {
        slot.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    // `Builder::spawn` 失败时会丢弃传入的闭包：任务放在共享的槽里，失败后取回交给线程池。
    let slot = Arc::new(Mutex::new(Some(Box::new(task) as Task)));
    let theirs = Arc::clone(&slot);
    let spawned = thread::Builder::new()
        .name("winc-blocking".into())
        .spawn(move || {
            if let Some(task) = take(&theirs) {
                task();
            }
        });
    if spawned.is_err()
        && let Some(task) = take(&slot)
    {
        pool().spawn(task);
    }
}

/// 邀请最多 `helpers` 个后台线程与调用线程一起执行 `work`（每个线程调用一次）。
///
/// 调用线程在此之后应自己也执行 `work`，完成后用 [`Crew::wait`] 等待帮手结束。`work` 必须自行
/// 从共享的任务中逐份领取，领完即返回：帮手何时开始、是否开始都不确定。
pub fn crew(helpers: usize, work: impl Fn() + Send + Sync + 'static) -> Crew {
    crew_in(pool(), helpers, Arc::new(work))
}

fn crew_in(pool: &'static Pool, helpers: usize, work: Arc<dyn Fn() + Send + Sync>) -> Crew {
    let shared = Arc::new(CrewState::default());
    for _ in 0..helpers {
        let (shared, work) = (Arc::clone(&shared), Arc::clone(&work));
        pool.spawn(Box::new(move || {
            {
                let mut count = shared.lock();
                if count.closed {
                    return;
                }
                count.started += 1;
            }
            // 帮手发生 panic 时（unwind 构建，如测试）也要计为已完成，否则 `Crew::wait`
            // 会一直等待。
            struct Done(Arc<CrewState>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.lock().finished += 1;
                    self.0.done.notify_all();
                }
            }
            let _done = Done(shared);
            work();
        }));
    }
    Crew { shared }
}

/// 由 [`crew`] 邀请的帮手线程的句柄，用于等待它们结束。
pub struct Crew {
    shared: Arc<CrewState>,
}

#[derive(Default)]
struct CrewState {
    count: Mutex<CrewCount>,
    done: Condvar,
}

#[derive(Default)]
struct CrewCount {
    /// 是否不再需要帮手：为 `true` 时，尚未开始的帮手任务直接退出。
    closed: bool,
    /// 已开始执行 `work` 的帮手数。
    started: usize,
    /// 已经结束（完成或 panic）的帮手数。
    finished: usize,
}

impl CrewState {
    fn lock(&self) -> std::sync::MutexGuard<'_, CrewCount> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Crew {
    /// 等待帮手结束：尚未开始的帮手不再执行 `work`，已经开始的等它们完成。
    ///
    /// 在调用线程自己的那份工作做完之后调用。
    pub fn wait(self) {
        let mut count = self.shared.lock();
        count.closed = true;
        while count.finished < count.started {
            count = self
                .shared
                .done
                .wait(count)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// 把 `0..parts` 各份的工作分给多个线程执行，返回按份的序号排列的结果。
///
/// 参与的线程是调用线程加上最多 `threads - 1` 个帮手；`threads`、`parts` 不大于 1 时由调用线程
/// 依次完成。
///
/// # Panics
///
/// `work` 在帮手线程中 panic 时，对应份没有结果，本函数随之 panic；release 构建设置了
/// `panic = "abort"`，此时 panic 等于进程中止。
pub fn split<R: Send + 'static>(
    parts: usize,
    threads: usize,
    work: impl Fn(usize) -> R + Send + Sync + 'static,
) -> Vec<R> {
    split_in(pool(), parts, threads, work)
}

fn split_in<R: Send + 'static>(
    pool: &'static Pool,
    parts: usize,
    threads: usize,
    work: impl Fn(usize) -> R + Send + Sync + 'static,
) -> Vec<R> {
    if parts <= 1 || threads <= 1 {
        return (0..parts).map(work).collect();
    }
    let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let results: Arc<Mutex<Vec<Option<R>>>> =
        Arc::new(Mutex::new((0..parts).map(|_| None).collect()));
    let take = {
        let (next, results) = (Arc::clone(&next), Arc::clone(&results));
        move || {
            loop {
                let part = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if part >= parts {
                    return;
                }
                let result = work(part);
                results.lock().unwrap_or_else(PoisonError::into_inner)[part] = Some(result);
            }
        }
    };
    let take: Arc<dyn Fn() + Send + Sync> = Arc::new(take);
    let crew = crew_in(pool, (threads - 1).min(parts - 1), Arc::clone(&take));
    take();
    crew.wait();
    let mut results = results.lock().unwrap_or_else(PoisonError::into_inner);
    results
        .iter_mut()
        .map(|r| r.take().expect("每一份都做完了"))
        .collect()
}

struct Pool {
    state: Mutex<State>,
    /// 有新任务时，通过该条件变量唤醒一个空闲线程。
    ready: Condvar,
    /// 线程数上限（个）。
    max: usize,
    /// 线程空闲多久后退出。
    keep_alive: Duration,
}

struct State {
    queue: VecDeque<Task>,
    /// 已创建且未退出的线程数（个），包含空闲线程。
    threads: usize,
    /// 正在等待任务的空闲线程数（个）。
    idle: usize,
}

impl Pool {
    const fn new(max: usize, keep_alive: Duration) -> Pool {
        Pool {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                threads: 0,
                idle: 0,
            }),
            ready: Condvar::new(),
            max,
            keep_alive,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn spawn(&'static self, task: Task) {
        let mut state = self.lock();
        state.queue.push_back(task);

        // 每个空闲线程领走一个任务；队列中超出空闲线程数的任务才需要新建线程。
        if state.queue.len() <= state.idle {
            self.ready.notify_one();
            return;
        }
        if state.threads >= self.max {
            return;
        }
        state.threads += 1;
        drop(state);

        let spawned = thread::Builder::new()
            .name("winc-background".into())
            .spawn(move || self.work());
        if spawned.is_err() {
            // 线程创建失败（系统资源耗尽）：仍有其他线程时，任务留在队列中等它们领取；
            // 一个线程都没有时，在当前线程中依次执行队列里的全部任务。
            let mut state = self.lock();
            state.threads -= 1;
            if state.threads == 0 {
                let queued: Vec<Task> = state.queue.drain(..).collect();
                drop(state);
                queued.into_iter().for_each(|task| task());
            }
        }
    }

    fn work(&self) {
        // 任务发生 panic 时（unwind 构建，如测试）线程随之退出，`threads` 计数也要减 1。
        struct Leave<'a>(&'a Pool);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                self.0.lock().threads -= 1;
            }
        }
        let leave = Leave(self);

        loop {
            let mut state = self.lock();
            let task = loop {
                if let Some(task) = state.queue.pop_front() {
                    break task;
                }
                state.idle += 1;
                let (next, waited) = self
                    .ready
                    .wait_timeout(state, self.keep_alive)
                    .unwrap_or_else(PoisonError::into_inner);
                state = next;
                state.idle -= 1;
                if waited.timed_out() && state.queue.is_empty() {
                    // 在持有同一把锁时把 `threads` 减 1：此后提交的任务看到的线程数已不含本线程，
                    // 会新建线程，不会排给已经退出的线程。
                    state.threads -= 1;
                    let last = state.threads == 0;
                    drop(state);
                    std::mem::forget(leave);
                    if last {
                        release_free_memory();
                    }
                    return;
                }
            };
            drop(state);
            task();
        }
    }
}

/// 把分配器中已释放的空闲页还给系统，只在 Linux 的 glibc 上执行。
///
/// 线程池的最后一个线程空闲退出时调用。glibc 为后台线程建立独立的分配区（arena），其中已释放的
/// 内存不随线程退出归还，`malloc_trim` 遍历全部分配区把空闲页交还系统，耗时与空闲内存的量成正比
/// （毫秒级）。macOS 的 `malloc_zone_pressure_relief` 实测收回 0 字节，Windows 的堆自行解除提交
/// 大块的空闲内存，因此不做。
#[cfg(all(target_os = "linux", target_env = "gnu"))]
// 本函数只有对 `libc::malloc_trim` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
fn release_free_memory() {
    // SAFETY: `malloc_trim` 没有前置条件，可在任何线程调用；参数 0 表示堆顶不保留余量。
    unsafe { libc::malloc_trim(0) };
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_free_memory() {}

#[cfg(test)]
mod tests;
