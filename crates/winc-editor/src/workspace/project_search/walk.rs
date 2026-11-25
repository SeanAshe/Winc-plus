//! 后台搜索：本机的目录由多个线程并行遍历和读取，远端主机上的目录交给远端的服务进程，结果
//! 一并送回界面线程。

use std::ops::ControlFlow;
use std::sync::mpsc::Sender;
use std::thread;

use winc_backend::remote::Stopper;
use winc_search::Scope;

use super::*;

impl ProjectSearch {
    /// 按当前的匹配器在后台搜索 `source` 中的文件，取代正在进行的搜索（旧的随之停止）。
    ///
    /// `refresh` 为 `None` 时是整体搜索，否则是重新搜索一批文件。搜索在单独的线程上进行：远端的
    /// 目录要等待网络，本机的目录由搜索自己的线程并行读取，都不占用公共的线程池。
    pub(super) fn launch(
        &mut self,
        source: Source,
        dirty: Arc<HashMap<PathBuf, Rope>>,
        refresh: Option<Refresh>,
    ) {
        let (Some(matcher), Some(wake), Some(query)) = (&self.matcher, &self.wake, &self.query)
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let run = Arc::new(Run::default());
        let walk = Walk {
            root: self.root.clone(),
            extra: Arc::clone(&self.labelled),
            query: query.clone(),
            matcher: matcher.clone(),
            source,
            dirty,
            run: Arc::clone(&run),
            wake: Arc::clone(wake),
            last_wake: Mutex::new(None),
        };
        crate::background::spawn_blocking(move || walk.run(&sender));
        self.job = Some(Job {
            run,
            receiver,
            refresh,
        });
    }
}

/// 后台搜索哪些文件。
pub(super) enum Source {
    /// 遍历根目录和附加目录下的全部文件，按「包含」「排除」过滤（整体搜索）；第二项为是否也
    /// 搜索被忽略的文件。
    Tree(Filter, bool),
    /// 只搜索这些文件（重新搜索一批文件）。
    Files(Vec<PathBuf>),
    /// 只搜索根目录和附加目录下的依赖目录，其中被忽略的文件也搜索。
    Dependencies,
}

/// 一次搜索的上限、停止开关和结局，由界面线程（[`Job`]）和后台线程共用。
pub(super) struct Run {
    /// 匹配数的上限，以及停止标志。
    pub(super) budget: Budget,
    /// 有远端主机上的搜索没能完成（连接中断、远端出错），结果不全。
    pub(super) remote_failed: AtomicBool,
    /// 进行中的远端搜索各自的结束句柄；停止后放进来的立即结束。
    remote: Mutex<Vec<Stopper>>,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            budget: Budget::new(MAX_RESULTS),
            remote_failed: AtomicBool::new(false),
            remote: Mutex::default(),
        }
    }
}

impl Run {
    /// 让搜索尽快停止：本机的线程看到停止标志后退出，远端的搜索由服务进程结束。
    pub(super) fn stop(&self) {
        self.budget.stop();
        let remote =
            std::mem::take(&mut *self.remote.lock().unwrap_or_else(PoisonError::into_inner));
        remote.iter().for_each(Stopper::stop);
    }

    /// 登记一个进行中的远端搜索；搜索已经停止时立即结束它。
    pub(super) fn watch(&self, stopper: Stopper) {
        let mut remote = self.remote.lock().unwrap_or_else(PoisonError::into_inner);
        if self.budget.stopped() {
            stopper.stop();
        } else {
            remote.push(stopper);
        }
    }
}

/// 一次搜索的后台部分。
pub(super) struct Walk {
    pub(super) root: PathBuf,
    /// 一并搜索的附加目录：相对路径中的前缀和目录。
    pub(super) extra: Arc<[(String, PathBuf)]>,
    pub(super) query: Query,
    pub(super) matcher: Matcher,
    pub(super) source: Source,
    pub(super) dirty: Arc<HashMap<PathBuf, Rope>>,
    pub(super) run: Arc<Run>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// 上一次唤醒界面线程的时刻；为 `None`（第一批结果）时立即唤醒。
    last_wake: Mutex<Option<Instant>>,
}

impl Walk {
    /// 搜索全部来源，搜完后告诉界面线程。
    ///
    /// 每个远端主机上的部分各用一个线程收取远端送回的结果，本机的部分在当前线程上并行搜索。
    fn run(&self, sender: &Sender<Message>) {
        let dirs = || std::iter::once(&self.root).chain(self.extra.iter().map(|(_, dir)| dir));
        let (local, remote): (Vec<PathBuf>, Vec<PathBuf>) = dirs()
            .cloned()
            .partition(|dir| !winc_backend::is_remote(dir));
        let (scope, requests) = match &self.source {
            Source::Tree(filter, ignored) => (
                Scope::Tree {
                    dirs: local,
                    filter: filter.clone(),
                    ignored: *ignored,
                },
                self.tree_requests(&remote, *ignored),
            ),
            Source::Dependencies => (
                Scope::Dependencies(local),
                self.tree_requests(&remote, false)
                    .into_iter()
                    .map(|request| request.dependencies())
                    .collect(),
            ),
            Source::Files(files) => {
                // 有未保存修改的远端文件按内存中的内容搜索，和本机的文件一起处理。
                let (here, there): (Vec<PathBuf>, Vec<PathBuf>) = (files.iter().cloned())
                    .partition(|file| {
                        !winc_backend::is_remote(file) || self.dirty.contains_key(file)
                    });
                (Scope::Files(here), self.file_requests(there))
            }
        };

        thread::scope(|threads| {
            for request in &requests {
                threads.spawn(|| self.search_remote(request, sender));
            }
            winc_search::search_parallel(scope.paths(), &self.matcher, &self.run.budget, {
                |searcher, path| {
                    let found = match self.dirty.get(path) {
                        Some(rope) => searcher.text(rope).map(|matches| (matches, false)),
                        None => searcher.file(path),
                    };
                    match found {
                        Some((matches, lossy)) => {
                            self.emit(path.to_path_buf(), matches, lossy, sender)
                        }
                        None => ControlFlow::Continue(()),
                    }
                }
            });
            // 本机的部分达到上限而停止时，远端的部分也不必再找。
            if self.run.budget.stopped() {
                self.run.stop();
            }
        });

        let _ = sender.send(Message::Done);
        (self.wake)();
    }

    /// 记入一个文件的匹配并送给界面线程；达到上限、或界面线程已不再接收时返回 `Break`。
    pub(super) fn emit(
        &self,
        path: PathBuf,
        mut matches: Vec<LineMatch>,
        lossy: bool,
        sender: &Sender<Message>,
    ) -> ControlFlow<()> {
        let flow = self.run.budget.take(&mut matches);
        if !matches.is_empty() {
            let relative = crate::workspace::tree::labelled_path(&self.root, &self.extra, &path);
            let file = FileResult {
                path,
                relative,
                matches,
                lossy,
            };
            if sender.send(Message::File(file)).is_err() {
                return ControlFlow::Break(());
            }
            self.maybe_wake();
        }
        flow
    }

    /// 结果送出后调用：距上次唤醒超过 [`WAKE_INTERVAL`] 时才唤醒界面线程。
    fn maybe_wake(&self) {
        let mut last = self
            .last_wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if last.is_none_or(|at| at.elapsed() >= WAKE_INTERVAL) {
            *last = Some(Instant::now());
            drop(last);
            (self.wake)();
        }
    }
}
