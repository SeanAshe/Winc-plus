//! 全局搜索：在工作区的所有文件中查找，以及跨文件替换。
//!
//! 主要类型是 [`ProjectSearch`]（搜索状态与后台任务）、[`Query`]（搜索条件）和
//! [`FileResult`]（单个文件的匹配）。搜索在后台线程中遍历目录、按块读取文件并匹配，结果
//! 陆续送回界面线程。匹配规则与 [`crate::Search`] 共用同一个匹配器；二进制判断、BOM 与
//! 编码解码复用打开文件时的同一套逻辑，使匹配位置与打开后的文本、行号一致。

mod matches;
mod replace;
mod scan;
mod walk;

use matches::*;
pub use replace::*;
use scan::*;
use walk::*;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::ops::{ControlFlow, Range};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, atomic::Ordering::Relaxed};
use std::thread;
use std::time::{Duration, Instant};

use ignore::WalkState;
use ignore::overrides::{Override, OverrideBuilder};
use memchr::{memchr_iter, memchr2_iter};
use ropey::Rope;
use winc_core::Lines;

use crate::document::encoding::{UTF8_BOM, decode, utf16_bom};
use crate::document::large::{LINE_CAP, visible_len};
use crate::document::{
    BINARY_SNIFF, Document, DocumentId, LARGE_FILE, OPEN_LIMIT, UTF16_INLINE, looks_binary,
};
use crate::editing::search::{Matcher, Options, Pattern, Search, count_chars, joined, unescape};
use crate::workspace::tree::{is_listed_file, natural_cmp, walker, walker_with_ignored};

/// 最多记录的匹配数（处）；达到后停止搜索，并提示结果不全。
///
/// 经验值：每处匹配只保存行号、列和最多 `PREVIEW_CHARS` 个字符的预览（按实际长度分配），
/// 该上限使结果占用的内存保持在数 MB 以内。
pub const MAX_RESULTS: usize = 20_000;
/// 预览中保留匹配之前的最大字符数（字符）；更长时前面用「…」代替。
const PREVIEW_BEFORE: usize = 24;
/// 预览的最大字符数（个，经验值，够显示一行的大部分内容）。
const PREVIEW_CHARS: usize = 160;
/// 搜索过程中唤醒界面线程的最短间隔（50 毫秒），避免大量结果使事件队列积压。
const WAKE_INTERVAL: Duration = Duration::from_millis(50);
/// 并行读文件的线程数上限（个，含遍历目录的线程）。
///
/// 每个线程同一时刻最多处理一个文件。
pub const MAX_THREADS: usize = 4;
/// 读文件时每块的大小（字节，256 KiB；测试中为 7 字节）。
///
/// 文件按块读取、按块搜索，内存中只有当前块加上尚未读完的那一行，与文件大小无关。测试中取很小的值，
/// 使换行符、`\r\n` 和多字节字符落在块的边界上。
const CHUNK: usize = if cfg!(test) { 7 } else { 256 << 10 };
/// 读文件的缓冲容量上限（字节，1 MiB，经验值）。
///
/// 超长行使缓冲容量超过该值时，搜索完当前文件后释放缓冲，避免每个工作线程长期持有大块内存。
const BUFFER_MAX: usize = 1 << 20;

/// 全局搜索的条件：查询文本、匹配选项和路径过滤规则。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// 查找的文本；`regex` 为 `true` 时是正则表达式。
    pub text: String,
    /// 是否区分大小写。
    pub case_sensitive: bool,
    /// 全字匹配：匹配的两端不能紧挨着字母、数字或下划线。
    pub whole_word: bool,
    /// 是否按正则匹配。
    pub regex: bool,
    /// 只搜索匹配的文件：逗号分隔的 glob，例如 `src, *.rs`；为空时不限制。
    pub include: String,
    /// 是否也搜索被 `.gitignore`、`.ignore` 和全局规则忽略的文件。
    pub include_ignored: bool,
    /// 不搜索匹配的文件：逗号分隔的 glob。
    pub exclude: String,
    /// 每处匹配都含有的一段文字，大小写规则与 `case_sensitive` 相同，用于跳过不含它的内容；
    /// 为空时没有这项信息。
    pub required: String,
}

impl Query {
    fn options(&self) -> Options {
        Options {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
        }
    }

    /// 按相同条件构造文件内查找，用于替换，使替换结果与文件内的「全部替换」一致。
    fn in_file_search(&self, replacement: &str) -> Search {
        let mut search = Search::default();
        search.query = self.text.clone();
        search.replacement = replacement.to_owned();
        search.case_sensitive = self.case_sensitive;
        search.whole_word = self.whole_word;
        search.regex = self.regex;
        search
    }
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

/// 搜索结果中的一处匹配：行号、列范围、预览文本和匹配在预览中的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// 行号（从 0 起），与打开文件后文档中的行号一致。
    pub line: usize,
    /// 匹配在该行内的列范围（字符偏移，从 0 起）。
    pub cols: Range<usize>,
    /// 该行的预览文本：已去掉行首空白，过长时截去两端，制表符换成空格。
    pub preview: String,
    /// 匹配在 `preview` 中的字节范围（UTF-8 字节偏移）。
    pub highlight: Range<usize>,
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
    cancel: Arc<AtomicBool>,
    truncated: Arc<AtomicBool>,
    receiver: Receiver<Message>,
    /// 后台重新搜索一批文件时的请求；为 `None` 时是从头开始的整体搜索。
    refresh: Option<Refresh>,
}

/// 后台重新搜索一批文件的请求（见 [`ProjectSearch::refresh_files`]），以及已经送回的新结果。
///
/// 新结果在这批文件全部搜完时才一次换进 `results`：在此之前界面继续显示旧结果，不会出现一部分
/// 文件的旧结果已经移除、新结果还没送到的中间状态。
struct Refresh {
    /// 要重新搜索的文件；结果换进 `results` 时，这些文件原有的结果被替换。
    files: HashSet<PathBuf>,
    /// 其中已打开且有未保存修改的文件的当前内容，与后台线程共用。
    dirty: Arc<HashMap<PathBuf, Rope>>,
    /// 后台已送回的新结果。
    found: Vec<FileResult>,
}

impl Drop for Job {
    fn drop(&mut self) {
        // 查询变化或关闭搜索时，通知后台线程尽快停止。
        self.cancel.store(true, Relaxed);
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
    /// 按 `query` 编译好的匹配器，重新搜索个别文件时复用；查询为空或无法编译时为 `None`。
    matcher: Option<Matcher>,
    /// [`Self::start`] 收到的唤醒回调，后台重新搜索一批文件时同样使用；与 `matcher` 同时设置。
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    total: usize,
    truncated: bool,
    error: Option<SearchError>,
    job: Option<Job>,
    /// 结果每变化一次加 1，界面据此判断是否需要重新排列结果列表。
    version: u64,
}

impl ProjectSearch {
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
        self.clear();
        self.query = Some(query.clone());
        self.root = root.to_path_buf();

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
        let overrides = match globs(root, &query.include, &query.exclude) {
            Ok(overrides) => overrides,
            Err(error) => {
                self.error = Some(SearchError::Filter(error));
                return;
            }
        };

        self.matcher = Some(matcher);
        self.wake = Some(Arc::new(wake));
        self.launch(
            Source::Tree(overrides, query.include_ignored),
            Arc::new(dirty.into_iter().collect()),
            None,
        );
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
            self.truncated |= job.truncated.load(Relaxed);
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
            self.truncated = job.truncated.load(Relaxed);
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
    /// 为 `None` 时在当前线程读取磁盘文件。后台正在重新搜索的一批文件中也有它时，以这里的结果
    /// 为准。
    pub fn refresh_file(&mut self, path: &Path, rope: Option<&Rope>) {
        let Some(matcher) = &self.matcher else {
            return;
        };
        let found = match rope {
            Some(rope) => Some((search_lines(matcher, rope, MAX_RESULTS), false)),
            None => search_disk(
                matcher,
                &mut Vec::new(),
                path,
                MAX_RESULTS,
                &AtomicBool::new(false),
            ),
        };
        let fresh =
            found.and_then(|(matches, lossy)| file_result(&self.root, path, matches, lossy));

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

/// 把「包含」「排除」两个输入框中的 glob 编译为规则。
///
/// 各模式以逗号分隔：不含 `/` 的模式在任意一级目录生效（`src` 匹配所有名为 `src` 的目录和文件）；
/// 以 `./` 或 `/` 开头的模式只从根目录起算；模式指向目录时，目录内的所有文件都匹配。
///
/// # Errors
///
/// 某个 glob 语法有误时，返回带「包含」或「排除」前缀的说明。
fn globs(root: &Path, include: &str, exclude: &str) -> Result<Override, String> {
    let mut builder = OverrideBuilder::new(root);

    for (text, prefix, include) in [(include, "", true), (exclude, "!", false)] {
        for pattern in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let pattern = pattern.trim_end_matches('/');
            let anchored = pattern.starts_with("./") || pattern.starts_with('/');
            let body = pattern.trim_start_matches("./").trim_start_matches('/');
            let base = if anchored || body.contains('/') {
                format!("/{body}")
            } else {
                format!("**/{body}")
            };

            for glob in [base.clone(), format!("{base}/**")] {
                builder.add(&format!("{prefix}{glob}")).map_err(|e| {
                    let error = e.to_string();
                    if include {
                        winc_i18n::t!("workspace-search-include-error", error = error)
                    } else {
                        winc_i18n::t!("workspace-search-exclude-error", error = error)
                    }
                })?;
            }
        }
    }

    builder.build().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
