//! 全局搜索：在工作区的所有文件中查找，以及跨文件替换。
//!
//! 主要类型是 [`ProjectSearch`]（搜索状态与后台任务）、[`Query`]（搜索条件）和
//! [`FileResult`]（单个文件的匹配）。搜索在后台线程中遍历目录、按块读取文件并匹配，结果
//! 陆续送回界面线程；远端主机上的目录由远端的服务进程搜索，只传回结果。匹配、二进制判断、
//! 解码和分行都由 `winc_search` 完成，与文件内查找、打开文件的规则相同，搜出的位置与打开后的
//! 文本和行号一致。

mod remote;
mod replace;
mod walk;

pub use replace::*;
use walk::*;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, atomic::Ordering::Relaxed};
use std::time::{Duration, Instant};

use ropey::Rope;
use winc_search::{Budget, Filter, Matcher, Searcher};
pub use winc_search::{LineMatch, MAX_THREADS, Query};

use crate::document::{Document, DocumentId};
use crate::editing::search::Search;
use crate::workspace::tree::natural_cmp;

/// 最多记录的匹配数（处）；达到后停止搜索，并提示结果不全。
///
/// 取自调参：每处匹配只保存行号、列和一段有长度上限的预览（按实际长度分配），该上限使结果
/// 占用的内存保持在数 MB 以内。
pub const MAX_RESULTS: usize = 20_000;
/// 搜索过程中唤醒界面线程的最短间隔（50 毫秒），避免大量结果使事件队列积压。
const WAKE_INTERVAL: Duration = Duration::from_millis(50);

/// 按相同条件构造文件内查找，用于替换，使替换结果与文件内的「全部替换」一致。
fn in_file_search(query: &Query, replacement: &str) -> Search {
    let mut search = Search::default();
    search.query = query.text.clone();
    search.replacement = replacement.to_owned();
    search.case_sensitive = query.case_sensitive;
    search.whole_word = query.whole_word;
    search.regex = query.regex;
    search
}

/// 一个文件的搜索结果：路径、全部匹配，以及是否含无法解码的字节。
#[derive(Debug, Clone)]
pub struct FileResult {
    /// 文件的绝对路径。
    pub path: PathBuf,
    /// 相对工作区根目录的路径，用 `/` 分隔。
    pub relative: String,
    /// 该文件中的全部匹配，按位置排序。
    pub matches: Vec<LineMatch>,
    /// 文件中有无法解码的字节，这些字节按 U+FFFD 替换字符解码后搜索；这类文件不能替换。
    pub lossy: bool,
}

/// 搜索条件有误的原因，各带一句给用户看的说明（[`ProjectSearch::error`] 返回其中的说明）。
enum SearchError {
    /// 搜索的文字（或正则表达式）有误。
    Pattern(String),
    /// 「包含」或「排除」里的 glob 有误。
    Filter(String),
}

/// 后台线程发给界面线程的消息。
enum Message {
    File(FileResult),
    Done,
}

/// 正在进行的一次搜索。
struct Job {
    run: Arc<Run>,
    receiver: Receiver<Message>,
    /// 后台重新搜索一批文件时的请求；为 `None` 时是从头开始的整体搜索。
    refresh: Option<Refresh>,
}

/// 后台重新搜索一批文件的请求（见 [`ProjectSearch::refresh_files`]），以及已经送回的新结果。
///
/// 新结果在这批文件全部搜完时才一次换入 `results`：在此之前界面继续显示旧结果，不会出现一部分
/// 文件的旧结果已经移除、新结果还没送到的中间状态。
struct Refresh {
    /// 要重新搜索的文件；结果换入 `results` 时，这些文件原有的结果被替换。
    files: HashSet<PathBuf>,
    /// 其中已打开且有未保存修改的文件的当前内容，与后台线程共用。
    dirty: Arc<HashMap<PathBuf, Rope>>,
    /// 后台已送回的新结果。
    found: Vec<FileResult>,
}

impl Drop for Job {
    fn drop(&mut self) {
        // 查询变化或关闭搜索时，让后台线程和远端的搜索尽快停止。
        self.run.stop();
    }
}

/// 全局搜索的状态：条件、结果和后台任务。
#[derive(Default)]
pub struct ProjectSearch {
    /// 按路径排序；搜索过程中陆续插入新文件的结果。
    pub results: Vec<FileResult>,
    /// 当前结果对应的条件和工作区根目录。
    query: Option<Query>,
    root: PathBuf,
    /// 一并搜索的附加目录，见 [`Self::set_extra_folders`]。
    extra: Vec<PathBuf>,
    /// 各个附加目录在相对路径中的前缀和目录，每次 [`Self::start`] 时确定。
    labelled: Arc<[(String, PathBuf)]>,
    /// 按 `query` 编译好的匹配器，重新搜索个别文件时复用；查询为空或无法编译时为 `None`。
    matcher: Option<Matcher>,
    /// [`Self::start`] 收到的唤醒回调，后台重新搜索一批文件时同样使用；与 `matcher` 同时设置。
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    total: usize,
    truncated: bool,
    /// 见 [`Self::remote_failed`]。
    remote_failed: bool,
    error: Option<SearchError>,
    job: Option<Job>,
    /// 结果每变化一次加 1，界面据此判断是否需要重新排列结果列表。
    version: u64,
}

impl ProjectSearch {
    /// 设置与工作区根目录一并搜索的附加目录，下一次 [`Self::start`] 起生效。
    ///
    /// 其中的文件的相对路径以该目录的前缀开头（与快速打开的文件表相同）。包含、排除的通配相对于
    /// 工作区根目录，对附加的目录按文件名和完整路径匹配。
    pub fn set_extra_folders(&mut self, folders: Vec<PathBuf>) {
        self.extra = folders;
    }

    /// 按新条件从头搜索，旧的搜索作废。
    ///
    /// `dirty` 是有未保存修改的文档：路径和当前内容。后台线程有新结果（最多每 50 ms 一次）和
    /// 搜完时调用 `wake`。条件有误时不返回错误，而是记在 [`Self::error`] 里。
    pub fn start(
        &mut self,
        root: &Path,
        query: &Query,
        dirty: Vec<(PathBuf, Rope)>,
        wake: impl Fn() + Send + Sync + 'static,
    ) {
        self.begin(root, query, dirty, Arc::new(wake), false);
    }

    /// 和 [`Self::start`] 一样，但只在根目录和附加目录下的依赖目录（`node_modules`、`.venv`
    /// 等）里找，其中被 `.gitignore` 忽略的文件也搜索；「包含」「排除」不生效。
    pub fn start_in_dependencies(
        &mut self,
        root: &Path,
        query: &Query,
        dirty: Vec<(PathBuf, Rope)>,
        wake: impl Fn() + Send + Sync + 'static,
    ) {
        self.begin(root, query, dirty, Arc::new(wake), true);
    }

    fn begin(
        &mut self,
        root: &Path,
        query: &Query,
        dirty: Vec<(PathBuf, Rope)>,
        wake: Arc<dyn Fn() + Send + Sync>,
        dependencies: bool,
    ) {
        self.clear();
        self.query = Some(query.clone());
        self.root = root.to_path_buf();
        let labels = crate::workspace::tree::folder_prefixes(root, &self.extra);
        self.labelled = labels.into_iter().zip(self.extra.iter().cloned()).collect();

        if query.text.is_empty() {
            return;
        }

        let matcher = match Matcher::requiring(&query.text, query.options(), &query.required) {
            Ok(matcher) => matcher,
            Err(error) => {
                let error = error.to_string();
                let message = winc_i18n::t!("workspace-search-regex-error", error = error);
                self.error = Some(SearchError::Pattern(message));
                return;
            }
        };
        let filter = match Filter::new(root, &query.include, &query.exclude) {
            Ok(filter) => filter,
            Err(error) => {
                let message = error.message;
                self.error = Some(SearchError::Filter(match error.exclude {
                    false => winc_i18n::t!("workspace-search-include-error", error = message),
                    true => winc_i18n::t!("workspace-search-exclude-error", error = message),
                }));
                return;
            }
        };

        self.matcher = Some(matcher);
        self.wake = Some(wake);
        let source = match dependencies {
            true => Source::Dependencies,
            false => Source::Tree(filter, query.include_ignored),
        };
        self.launch(source, Arc::new(dirty.into_iter().collect()), None);
    }

    /// 清空结果并作废正在进行的搜索：版本号递增，之后到达的旧结果会被丢弃。
    ///
    /// 结果占用的内存一并释放。
    pub fn clear(&mut self) {
        self.version += 1;
        self.job = None;
        self.results = Vec::new();
        self.query = None;
        self.matcher = None;
        self.wake = None;
        self.total = 0;
        self.truncated = false;
        self.remote_failed = false;
        self.error = None;
    }

    /// 接收后台线程送来的结果，返回结果是否有变化。
    pub fn poll(&mut self) -> bool {
        let Some(job) = &self.job else {
            return false;
        };
        let mut batch = Vec::new();
        let mut done = false;

        loop {
            match job.receiver.try_recv() {
                Ok(Message::File(file)) => batch.push(file),
                Ok(Message::Done) | Err(TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }

        self.finish(batch, done)
    }

    /// 阻塞到搜索（包括后台重新搜索一批文件）结束；供测试和替换之前使用。
    pub fn wait(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let batch = job
            .receiver
            .iter()
            .map_while(|message| match message {
                Message::File(file) => Some(file),
                Message::Done => None,
            })
            .collect();
        self.finish(batch, true);
    }

    /// 把后台送来的一批结果合并进 `results`；`done` 为 `true` 表示搜索已结束。
    ///
    /// 返回结果是否有变化。
    ///
    /// 重新搜索一批文件时，送来的结果先存起来，搜完时才替换这些文件原有的结果（见 [`Refresh`]）。
    fn finish(&mut self, batch: Vec<FileResult>, done: bool) -> bool {
        if let Some(job) = &mut self.job
            && let Some(refresh) = &mut job.refresh
        {
            refresh.found.extend(batch);
            if !done {
                return false;
            }
            let Refresh {
                files, mut found, ..
            } = job.refresh.take().expect("刚刚检查过");
            self.truncated |= job.run.budget.truncated();
            self.remote_failed |= job.run.remote_failed.load(Relaxed);
            self.job = None;
            // 搜索期间已经单独重新搜索过的文件（见 `refresh_file`）不在 `files` 中，丢弃它们
            // 在后台读到的、可能已经过时的结果。
            found.retain(|file| files.contains(&file.path));
            self.replace_results(&files, found);
            return true;
        }

        let mut changed = !batch.is_empty();
        if changed {
            merge_sorted(&mut self.results, &mut self.total, batch);
            self.version += 1;
        }

        if done && let Some(job) = self.job.take() {
            self.truncated = job.run.budget.truncated();
            self.remote_failed = job.run.remote_failed.load(Relaxed);
            self.version += 1;
            changed = true;
        }

        changed
    }

    /// 返回结果的版本号；结果有任何变化时递增。
    pub fn version(&self) -> u64 {
        self.version
    }

    /// 是否还有搜索在进行中：整体搜索，或后台重新搜索一批文件（见 [`Self::refresh_files`]）。
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// 返回当前结果对应的条件；尚未搜索时返回 `None`。
    pub fn query(&self) -> Option<&Query> {
        self.query.as_ref()
    }

    /// 返回匹配的总数（处）。
    pub fn total(&self) -> usize {
        self.total
    }

    /// 是否因结果达到 [`MAX_RESULTS`] 而停止搜索，即结果不全。
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// 是否有远端主机上的搜索没能完成（连接中断、远端出错），其中的文件没有全部搜索到。
    pub fn remote_failed(&self) -> bool {
        self.remote_failed
    }

    /// 是否有行因正则回溯过多而放弃查找，此时结果可能不全。
    ///
    /// 按当前查询计：查询变化后重新编译匹配器，标记随之清除。
    pub fn gave_up(&self) -> bool {
        self.matcher.as_ref().is_some_and(Matcher::gave_up)
    }

    /// 返回正则表达式或 glob 有误时的说明；没有错误时返回 `None`。
    pub fn error(&self) -> Option<&str> {
        self.error.as_ref().map(|error| match error {
            SearchError::Pattern(message) | SearchError::Filter(message) => message.as_str(),
        })
    }

    /// 错误是否出在「包含」或「排除」的 glob 上（而不是搜索的文字上）。
    pub fn is_filter_error(&self) -> bool {
        matches!(self.error, Some(SearchError::Filter(_)))
    }

    /// 从结果中移除一个文件，例如替换完成之后。
    pub fn remove_file(&mut self, path: &Path) {
        if let Some(index) = self.results.iter().position(|f| f.path == path) {
            self.total -= self.results.remove(index).matches.len();
            self.version += 1;
        }
    }

    /// 从结果中移除一处匹配；文件没有其他匹配时一并移除该文件。
    pub fn remove_match(&mut self, path: &Path, index: usize) {
        let Some(at) = self.results.iter().position(|f| f.path == path) else {
            return;
        };
        let file = &mut self.results[at];

        if index < file.matches.len() {
            file.matches.remove(index);
            self.total -= 1;
            self.version += 1;
        }

        if file.matches.is_empty() {
            self.results.remove(at);
        }
    }

    /// 按当前条件立即重新搜索一个文件，并用新结果替换该文件原有的结果。
    ///
    /// 替换了其中几处之后调用，无需从头搜索。`rope` 是文档已打开且有未保存修改时的内容；
    /// 为 `None` 时在当前线程读取磁盘文件，远端主机上的文件改为在后台重新搜索（见
    /// [`Self::refresh_files`]），不在当前线程等待网络。后台正在重新搜索的一批文件中也有它时，
    /// 以这里的结果为准。
    pub fn refresh_file(&mut self, path: &Path, rope: Option<&Rope>) {
        let Some(matcher) = &self.matcher else {
            return;
        };
        if rope.is_none() && winc_backend::is_remote(path) {
            self.refresh_files([(path.to_path_buf(), None)]);
            return;
        }
        let budget = Budget::new(MAX_RESULTS);
        let mut searcher = Searcher::new(matcher, &budget);
        let found = match rope {
            Some(rope) => searcher.text(rope).map(|matches| (matches, false)),
            None => searcher.file(path),
        };
        let fresh = found.map(|(matches, lossy)| FileResult {
            path: path.to_path_buf(),
            relative: crate::workspace::tree::labelled_path(&self.root, &self.labelled, path),
            matches,
            lossy,
        });

        if let Some(refresh) = self.job.as_mut().and_then(|job| job.refresh.as_mut()) {
            refresh.files.remove(path);
        }
        let paths = HashSet::from([path.to_path_buf()]);
        self.replace_results(&paths, fresh.into_iter().collect());
    }

    /// 按当前条件在后台重新搜索一批文件，搜完后用新结果替换这些文件原有的结果；`rope` 的含义
    /// 同 [`Self::refresh_file`]。
    ///
    /// 切换分支等情况下可能一次有上千个文件变化，因此在后台搜索，结果由 [`Self::poll`] 收取。整体
    /// 搜索进行中时不处理（它会读到文件的新内容）；上一批还没搜完时与这一批合并、从头重新搜索；
    /// 这一批文件合计最多收 [`MAX_RESULTS`] 处。
    pub fn refresh_files(&mut self, files: impl IntoIterator<Item = (PathBuf, Option<Rope>)>) {
        let files: Vec<_> = files.into_iter().collect();
        if self.matcher.is_none() || files.is_empty() {
            return;
        }
        let (mut paths, mut dirty) = match &mut self.job {
            None => (HashSet::new(), HashMap::new()),
            Some(job) => match job.refresh.take() {
                None => return,
                Some(pending) => (pending.files, Arc::unwrap_or_clone(pending.dirty)),
            },
        };

        for (path, rope) in files {
            match rope {
                Some(rope) => dirty.insert(path.clone(), rope),
                None => dirty.remove(&path),
            };
            paths.insert(path);
        }

        let dirty = Arc::new(dirty);
        let source = Source::Files(paths.iter().cloned().collect());
        let refresh = Refresh {
            files: paths,
            dirty: Arc::clone(&dirty),
            found: Vec::new(),
        };
        self.launch(source, dirty, Some(refresh));
    }

    /// 用 `fresh` 替换 `paths` 中各文件原有的结果：旧结果一遍移除，新结果一遍归并，耗时与结果数
    /// 成线性关系。
    fn replace_results(&mut self, paths: &HashSet<PathBuf>, fresh: Vec<FileResult>) {
        let (before, total) = (self.results.len(), &mut self.total);
        self.results.retain(|file| {
            let keep = !paths.contains(&file.path);
            if !keep {
                *total -= file.matches.len();
            }
            keep
        });

        if self.results.len() != before || !fresh.is_empty() {
            merge_sorted(&mut self.results, &mut self.total, fresh);
            self.version += 1;
        }
    }
}

/// 把一批文件的结果按路径合并进 `results`；与已有结果路径相同时，新结果排在前面。
///
/// 逐个插入时每次都要移动后面的元素，总的移动次数与文件数的平方成正比，一次搜出上万个文件时
/// 会占用界面线程。归并只遍历一遍。
fn merge_sorted(results: &mut Vec<FileResult>, total: &mut usize, mut batch: Vec<FileResult>) {
    batch.sort_by(|a, b| path_cmp(&a.relative, &b.relative));
    *total += batch.iter().map(|f| f.matches.len()).sum::<usize>();
    let capacity = results.len() + batch.len();
    let mut old = std::mem::replace(results, Vec::with_capacity(capacity))
        .into_iter()
        .peekable();
    for file in batch {
        while let Some(prev) =
            old.next_if(|f| path_cmp(&f.relative, &file.relative) == Ordering::Less)
        {
            results.push(prev);
        }
        results.push(file);
    }
    results.extend(old);
}

/// 按路径排序：逐级比较，每一级用自然排序。
fn path_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.split('/'), b.split('/'));

    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) => match natural_cmp(x, y) {
                Ordering::Equal => {}
                other => return other,
            },
            (x, y) => return x.is_some().cmp(&y.is_some()),
        }
    }
}

#[cfg(test)]
mod tests;
