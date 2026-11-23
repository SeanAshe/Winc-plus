//! 状态文件的后台读写：读改写（含落盘）在一个线程里按提交的顺序进行，调用方不等待。
//!
//! 调用方先改自己内存中的状态，再把同一个修改交到这里；文件里的结果经回调送回。排着队的
//! 同一个文件的修改合并成一次读改写。进程结束、要让别的进程读到最新内容之前调用 [`flush`]。

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use super::State;

/// [`flush`] 最长的等待时间（5 秒）：磁盘失去响应时不让退出一直等下去。
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// 对状态的一个修改。
type Change = Box<dyn FnOnce(&mut State) + Send>;

/// 读改写完成后的回调：文件里现在的状态和写入的结果。
type Done = Box<dyn FnOnce(&State, &std::io::Result<()>) + Send>;

/// 交给后台线程的一件事。
enum Job {
    /// 见 [`update`]。
    Update {
        path: PathBuf,
        current: Box<State>,
        change: Change,
        done: Done,
    },
    /// 见 [`reload`]。
    Reload {
        path: PathBuf,
        done: Box<dyn FnOnce(State) + Send>,
    },
    /// 见 [`flush`]。
    Flush(Sender<()>),
}

/// 返回通往后台线程的队列，第一次调用时启动线程；线程起不来时返回 `None`。
fn queue() -> Option<&'static Sender<Job>> {
    static QUEUE: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    let start = || {
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new().name("winc-state".into());
        thread.spawn(move || run(&receiver)).ok().map(|_| sender)
    };
    QUEUE.get_or_init(start).as_ref()
}

/// 在后台读出 `path` 中现在的状态，用 `change` 修改后写回（见 [`State::update`]），完成后在
/// 后台线程上调用 `done`。
///
/// `current` 是调用方修改之前内存中的状态，文件读不出来时在它之上修改。先提交的先写入。
pub fn update(
    path: PathBuf,
    current: State,
    change: impl FnOnce(&mut State) + Send + 'static,
    done: impl FnOnce(&State, &std::io::Result<()>) + Send + 'static,
) {
    let job = Job::Update {
        path,
        current: Box::new(current),
        change: Box::new(change),
        done: Box::new(done),
    };
    // 没有后台线程时在调用方的线程上完成，修改不会丢。
    if let Some(job) = submit(job) {
        work(vec![job]);
    }
}

/// 在后台读出 `path` 中现在的状态，读到时在后台线程上调用 `done`；读不出来时不调用。
///
/// 排在此前提交的修改之后，读到的内容包含它们。
pub fn reload(path: PathBuf, done: impl FnOnce(State) + Send + 'static) {
    let done = Box::new(done);
    if let Some(job) = submit(Job::Reload { path, done }) {
        work(vec![job]);
    }
}

/// 等此前提交的修改都写入文件，最长 5 秒（`FLUSH_TIMEOUT`）。
pub fn flush() {
    let (sender, receiver) = mpsc::channel();
    if submit(Job::Flush(sender)).is_none() {
        let _ = receiver.recv_timeout(FLUSH_TIMEOUT);
    }
}

/// 把 `job` 放进队列；没有后台线程时原样还回来。
fn submit(job: Job) -> Option<Job> {
    match queue() {
        Some(queue) => queue.send(job).err().map(|mpsc::SendError(job)| job),
        None => Some(job),
    }
}

/// 后台线程：取出排着队的全部工作，一批一批地做。
fn run(receiver: &Receiver<Job>) {
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        batch.extend(receiver.try_iter());
        work(batch);
    }
}

/// 按顺序完成 `batch`：相邻的、同一个文件的修改合并成一次读改写。
fn work(batch: Vec<Job>) {
    let mut batch = batch.into_iter().peekable();
    while let Some(job) = batch.next() {
        match job {
            Job::Update {
                path,
                current,
                change,
                done,
            } => {
                let (mut changes, mut waiting) = (vec![change], vec![done]);
                while let Some(Job::Update { path: next, .. }) = batch.peek()
                    && *next == path
                    && let Some(Job::Update { change, done, .. }) = batch.next()
                {
                    changes.push(change);
                    waiting.push(done);
                }
                let (state, result) = State::update(&path, &current, |state| {
                    changes.into_iter().for_each(|change| change(state));
                });
                waiting.into_iter().for_each(|done| done(&state, &result));
            }
            Job::Reload { path, done } => {
                if let Ok(Some(state)) = State::read(&path) {
                    done(state);
                }
            }
            Job::Flush(reply) => {
                let _ = reply.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests;
