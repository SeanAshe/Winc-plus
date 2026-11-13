//! 遍历目录：多个线程并行取文件并搜索，结果送回界面线程。

use super::*;

impl ProjectSearch {
    /// 按当前的匹配器在后台搜索 `source` 中的文件，取代正在进行的搜索（旧的随之停止）。
    ///
    /// `refresh` 为 `None` 时是整体搜索，否则是重新搜索一批文件。
    pub(super) fn launch(
        &mut self,
        source: Source,
        dirty: Arc<HashMap<PathBuf, Rope>>,
        refresh: Option<Refresh>,
    ) {
        let (Some(matcher), Some(wake)) = (&self.matcher, &self.wake) else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let job = Job {
            cancel: Arc::new(AtomicBool::new(false)),
            truncated: Arc::new(AtomicBool::new(false)),
            receiver,
            refresh,
        };
        let walk = Walk {
            root: self.root.clone(),
            literal: self.query.as_ref().and_then(literal),
            matcher: matcher.clone(),
            source,
            dirty,
            cancel: Arc::clone(&job.cancel),
            truncated: Arc::clone(&job.truncated),
            count: Arc::new(AtomicUsize::new(0)),
            wake: Arc::clone(wake),
            last_wake: Arc::new(Mutex::new(None)),
        };
        crate::background::spawn(move || Arc::new(walk).run(sender));
        self.job = Some(job);
    }
}

/// 返回 `query` 的每处匹配都含有的一段文字，以及比较时是否区分大小写；无法确定时返回 `None`。
///
/// 按字面查找时就是查找的文本；正则查找时用调用方给出的，没有给出时从表达式里取（见
/// [`regex_literal`]）。文字跨行、或不区分大小写而含有 ASCII 以外的字符时不用：远端的 `grep`
/// 按行比较，大小写的规则也随它的语言环境变化，可能漏掉文件。
fn literal(query: &Query) -> Option<(String, bool)> {
    let found;
    let text = if !query.required.is_empty() {
        &query.required
    } else if !query.regex {
        &query.text
    } else {
        found = regex_literal(&query.text)?;
        &found
    };
    let single_line = !text.contains(['\n', '\r', '\0']);
    let comparable = query.case_sensitive || text.is_ascii();
    (single_line && comparable).then(|| (text.clone(), query.case_sensitive))
}

/// 返回正则表达式 `pattern` 的每处匹配都含有的一段文字：各种匹配共同的开头或结尾，取较长
/// 的一个；没有、或表达式用到了这里不解析的写法（环视、反向引用）时返回 `None`。
///
/// 表达式按区分大小写解析：不区分大小写的查找由调用方按不区分大小写比较这段文字。表达式
/// 自己用 `(?i)` 关掉大小写的部分没有共同的文字，不会被取出。
fn regex_literal(pattern: &str) -> Option<String> {
    use regex_syntax::hir::literal::{ExtractKind, Extractor};

    let hir = regex_syntax::parse(pattern).ok()?;
    let common = |suffix: bool| {
        let mut extractor = Extractor::new();
        extractor.kind(match suffix {
            true => ExtractKind::Suffix,
            false => ExtractKind::Prefix,
        });
        let found = extractor.extract(&hir);
        let bytes = match suffix {
            true => found.longest_common_suffix(),
            false => found.longest_common_prefix(),
        };
        String::from_utf8(bytes?.to_vec()).ok()
    };
    let (start, end) = (common(false), common(true));
    let longer = match (start, end) {
        (Some(start), Some(end)) if end.len() > start.len() => end,
        (Some(start), _) => start,
        (None, end) => end?,
    };
    (!longer.is_empty()).then_some(longer)
}

/// 后台搜索哪些文件。
pub(super) enum Source {
    /// 遍历根目录下的全部文件，按「包含」「排除」规则过滤（整体搜索）；第二项为是否也搜索
    /// 被忽略的文件。
    Tree(Override, bool),
    /// 只搜索这些文件（重新搜索一批文件）。
    Files(Vec<PathBuf>),
}

/// 一次搜索的后台部分。
pub(super) struct Walk {
    root: PathBuf,
    /// 每处匹配都含有的一段文字及是否区分大小写（见 [`literal`]）；远端的目录用它先筛出
    /// 候选的文件。
    literal: Option<(String, bool)>,
    matcher: Matcher,
    source: Source,
    dirty: Arc<HashMap<PathBuf, Rope>>,
    cancel: Arc<AtomicBool>,
    truncated: Arc<AtomicBool>,
    /// 已记录的匹配数，由多个线程共用。
    count: Arc<AtomicUsize>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// 上一次唤醒界面线程的时刻；为 `None`（第一批结果）时立即唤醒。
    last_wake: Arc<Mutex<Option<Instant>>>,
}

impl Walk {
    /// 一个线程遍历目录（或给定的文件列表），把要搜索的文件放进队列；另有若干后台线程从队列取
    /// 文件读取并匹配，遍历结束后遍历线程也参与。
    ///
    /// 文件读取和匹配是主要开销，因此由多个线程并行处理。
    pub(super) fn run(self: Arc<Self>, sender: Sender<Message>) {
        let threads = thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(MAX_THREADS);
        let queue = Arc::new(Queue::default());

        let helpers = {
            let (walk, queue, sender) = (Arc::clone(&self), Arc::clone(&queue), sender.clone());
            crate::background::crew(threads - 1, move || {
                let mut worker = Worker::new(&walk, sender.clone());
                while let Some(path) = queue.pop() {
                    if worker.visit(&path) == WalkState::Quit {
                        walk.cancel.store(true, Relaxed);
                    }
                }
            })
        };

        let paths: Box<dyn Iterator<Item = PathBuf> + '_> = match &self.source {
            // 远端的目录：由远端的一条命令给出要搜索的文件。能确定每处匹配都含有某段文字时
            // 只取含有它的文件，匹配仍由下面的工作线程按相同的规则完成，结果与本机一致。
            Source::Tree(overrides, ignored) if winc_backend::is_remote(&self.root) => {
                let containing = self.literal.as_ref().map(|(text, case_sensitive)| {
                    crate::workspace::remote_files::Containing {
                        text,
                        case_sensitive: *case_sensitive,
                    }
                });
                let stop = || self.cancel.load(Relaxed);
                let found = crate::workspace::remote_files::list(
                    &self.root,
                    *ignored,
                    containing.as_ref(),
                    &stop,
                );
                Box::new(
                    (found.into_iter())
                        .map(|relative| winc_backend::join(&self.root, &relative))
                        .filter(|path| !overrides.matched(path, false).is_ignore()),
                )
            }
            Source::Tree(overrides, ignored) => Box::new(
                if *ignored {
                    walker_with_ignored(&self.root)
                } else {
                    walker(&self.root)
                }
                .overrides(overrides.clone())
                .build()
                .flatten()
                .filter(|entry| {
                    entry
                        .file_type()
                        .is_some_and(|t| is_listed_file(t, entry.path()))
                })
                .map(ignore::DirEntry::into_path),
            ),
            Source::Files(files) => Box::new(files.iter().cloned()),
        };
        let mut worker = Worker::new(&self, sender.clone());
        for path in paths {
            if self.cancel.load(Relaxed) {
                break;
            }
            if let Some(path) = queue.push(path)
                && worker.visit(&path) == WalkState::Quit
            {
                self.cancel.store(true, Relaxed);
            }
        }
        queue.close();

        // 遍历结束后，当前线程也从队列取文件处理。
        while let Some(path) = queue.pop() {
            if worker.visit(&path) == WalkState::Quit {
                self.cancel.store(true, Relaxed);
            }
        }
        drop(worker);
        helpers.wait();

        let _ = sender.send(Message::Done);
        (self.wake)();
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

/// 遍历线程放入、读取线程取走的待搜索文件队列。
///
/// 遍历结束后关闭队列，取空后读取线程退出。
#[derive(Default)]
struct Queue {
    state: Mutex<(std::collections::VecDeque<PathBuf>, bool)>,
    ready: std::sync::Condvar,
}

/// 队列中待搜索文件的最大数量（个，经验值）。
///
/// 遍历目录比读文件快，不加限制时整个目录的路径都会堆积在队列里；该值只影响队列的内存占用。
/// 队列已满时的处理见 [`Queue::push`]。
const QUEUE_MAX: usize = 1024;

impl Queue {
    /// 把 `path` 放入队列；队列已满时取出最早的一个返回，由遍历线程自己处理。
    ///
    /// 这样即使帮手线程都在忙，遍历线程也总有文件可处理。
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

/// 读文件的线程之一：读取文件、匹配，有结果就发回界面线程。
struct Worker<'a> {
    walk: &'a Walk,
    sender: Sender<Message>,
    /// 每个线程各一个：正则的每个副本有自己的缓存，多个线程共用一个时会争用缓存。
    matcher: Matcher,
    /// 读文件的缓冲，同一线程复用。
    buffer: Vec<u8>,
}

impl<'a> Worker<'a> {
    fn new(walk: &'a Walk, sender: Sender<Message>) -> Self {
        Worker {
            walk,
            sender,
            matcher: walk.matcher.clone(),
            buffer: Vec::new(),
        }
    }

    fn visit(&mut self, path: &Path) -> WalkState {
        if self.walk.cancel.load(Relaxed) {
            return WalkState::Quit;
        }

        let file = match self.walk.dirty.get(path) {
            Some(rope) => self.search_rope(path, rope),
            None => self.search_file(path),
        };

        let Some(mut file) = file else {
            return WalkState::Continue;
        };

        // 多个线程共同计数：使总数超过上限的线程截去多出的部分，并通知所有线程停止。
        let before = self.walk.count.fetch_add(file.matches.len(), Relaxed);

        if before >= MAX_RESULTS {
            self.walk.truncated.store(true, Relaxed);
            return WalkState::Quit;
        }

        let full = before + file.matches.len() >= MAX_RESULTS;
        file.matches.truncate(MAX_RESULTS - before);

        if self.sender.send(Message::File(file)).is_err() {
            return WalkState::Quit;
        }

        self.walk.maybe_wake();

        if full {
            self.walk.truncated.store(true, Relaxed);
            return WalkState::Quit;
        }

        WalkState::Continue
    }

    fn search_file(&mut self, path: &Path) -> Option<FileResult> {
        let limit = self.remaining()?;
        let (matches, lossy) = search_disk(
            &self.matcher,
            &mut self.buffer,
            path,
            limit,
            &self.walk.cancel,
        )?;
        file_result(&self.walk.root, path, matches, lossy)
    }

    fn search_rope(&mut self, path: &Path, rope: &Rope) -> Option<FileResult> {
        let limit = self.remaining()?;
        let matches = search_lines(&self.matcher, rope, limit);
        file_result(&self.walk.root, path, matches, false)
    }

    /// 返回离 [`MAX_RESULTS`] 还差的匹配数（处）；已经达到上限时返回 `None`。
    ///
    /// 单个文件收满该数量之后不必再往下找：单个文件（例如压缩后的大文件）可能含远多于上限的匹配，
    /// 每处还带一段预览。
    fn remaining(&self) -> Option<usize> {
        let left = MAX_RESULTS.saturating_sub(self.walk.count.load(Relaxed));
        (left > 0).then_some(left)
    }
}

#[cfg(test)]
mod tests {
    use super::regex_literal;

    #[test]
    fn a_regex_gives_the_text_every_match_contains() {
        let found = |pattern| regex_literal(pattern);
        assert_eq!(found(r"fn \w+").as_deref(), Some("fn "));
        assert_eq!(found(r"\w+_test\(\)").as_deref(), Some("_test()"));
        assert_eq!(found(r"\bTODO\b").as_deref(), Some("TODO"));
        // 开头和结尾都有时取较长的。
        assert_eq!(found(r"ab\d+cdef").as_deref(), Some("cdef"));
        // 各种匹配没有共同的文字，或表达式自己关掉了大小写。
        assert_eq!(found(r"foo|bar"), None);
        assert_eq!(found(r"\d+"), None);
        assert_eq!(found(r"(?i)todo"), None);
        // 共同的开头仍然可用。
        assert_eq!(found(r"foo(bar|baz)").as_deref(), Some("fooba"));
        // 这里不解析的写法。
        assert_eq!(found(r"(?<=x)y"), None);
        assert_eq!(found(r"(a)\1"), None);
    }
}
