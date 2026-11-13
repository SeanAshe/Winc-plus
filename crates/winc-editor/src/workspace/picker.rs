//! 快速打开（⌘P）的文件索引和模糊匹配。
//!
//! [`FileIndex`] 在后台线程中遍历工作区，把相对路径存入紧凑的文件表，并按目录中的变化增量更新；
//! [`FileIndex::search`] 对文件表做子序列匹配和打分。

mod fuzzy;
mod scan;

use fuzzy::*;
use scan::*;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::background;
use crate::workspace::tree::{HIDDEN, is_listed_file, walker};
use crate::workspace::watch::Changes;

/// 索引收录的最大文件数（个，经验值）。
///
/// 文件再多用户也无法逐个浏览，而且打分要遍历整张文件表。
pub const MAX_FILES: usize = 200_000;
/// 结果列表显示的最大条数（条）。
pub const MAX_RESULTS: usize = 200;
/// 单个查询词的最大长度（字符，经验值）；超过时截去多余的部分，正常输入不会达到。
const MAX_QUERY_CHARS: usize = 128;
/// 后台忙碌时最多暂存的变化路径数（个），与目录监视一批变化的上限（`watch::MAX_PATHS`）相同。
///
/// 超过该值时不再逐个记录，改为后台完成后整个重新遍历。
const MAX_QUEUED: usize = 4096;
/// 并行打分的文件数下限（个，经验值）。
///
/// 待打分的文件少于该值时用一个线程算完，因为启动线程的开销更大。
const PARALLEL_MIN: usize = 16_384;
/// 打分使用的最大线程数（个，经验值）。
///
/// 查询的开销主要在遍历文件表上，线程再多也不会明显变快，反而增加每次查询的调度开销。
const MAX_THREADS: usize = 8;

/// 一个被索引的文件，文本借用自文件表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedFile<'a> {
    /// 相对工作区根目录的路径，分隔符统一是 `/`。
    pub path: &'a str,
    /// 文件名（路径最后一段）。
    pub name: &'a str,
}

/// 整张文件表：所有路径首尾相接存放在一个字符串里。
///
/// 20 万个文件只需要三块内存（路径文本、各条的字节范围、文件名的偏移），比每个文件分配两个
/// `String` 约省一半，打分时顺序读内存也更快。
#[derive(Debug, Default)]
struct Files {
    /// 所有相对路径（以 `/` 分隔），每个文件一条，按 [`path_order`] 排序。
    paths: Packed,
    /// 与 `paths` 一一对应：文件名在路径中的起始字节偏移。
    names: Vec<u32>,
    /// 文件太多、只收录了浅层目录时，收录的层数（从 1 起）；全部收录时为 `None`。
    layers: Option<usize>,
    /// 遍历过的目录（相对路径，不含根目录）。
    ///
    /// 被忽略的、因太深而未收录的目录不在其中，
    /// 这些目录内的变化与文件表无关。
    dirs: DirSet,
}

/// 一组字符串首尾相接存放在一个 `String` 里，每条记下字节范围。
///
/// 遍历目录时用它收集路径：20 万个路径只占两块内存，而不是 20 万个各自分配的 `String`。大量
/// 小块内存即使用完释放，macOS 的分配器也不会还给系统，进程占用的内存会一直停在高位。
#[derive(Debug, Default, Clone)]
struct Packed {
    text: String,
    /// 每条在 `text` 中的（起始，结束）字节偏移。
    spans: Vec<(u32, u32)>,
}

impl Packed {
    /// 按两块内存的容量上限（`text` 的字节数、条数）创建。
    fn with_capacity(bytes: usize, len: usize) -> Packed {
        Packed {
            text: String::with_capacity(bytes),
            spans: Vec::with_capacity(len),
        }
    }

    fn push(&mut self, s: &str) {
        let start = self.text.len() as u32;
        self.text.push_str(s);
        self.spans.push((start, self.text.len() as u32));
    }

    fn len(&self) -> usize {
        self.spans.len()
    }

    fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn get(&self, index: usize) -> &str {
        let (start, end) = self.spans[index];
        &self.text[start as usize..end as usize]
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        self.spans
            .iter()
            .map(|&(start, end)| &self.text[start as usize..end as usize])
    }

    /// 只留前 `len` 条。
    fn truncate(&mut self, len: usize) {
        if let Some(&(start, _)) = self.spans.get(len) {
            self.text.truncate(start as usize);
            self.spans.truncate(len);
        }
    }

    /// 在按 `f` 的顺序排好的各条中二分查找，返回值同 [`slice::binary_search_by`]。
    fn binary_search_by(
        &self,
        mut f: impl FnMut(&str) -> std::cmp::Ordering,
    ) -> Result<usize, usize> {
        self.spans
            .binary_search_by(|&(start, end)| f(&self.text[start as usize..end as usize]))
    }

    fn shrink_to_fit(&mut self) {
        self.text.shrink_to_fit();
        self.spans.shrink_to_fit();
    }
}

impl<'a> FromIterator<&'a str> for Packed {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        let mut packed = Packed::default();
        for s in iter {
            packed.push(s);
        }
        packed
    }
}

/// 遍历过的目录（相对路径）：按字节序排好、去重后放在一个 [`Packed`] 里，用二分查找判断是否在其中。
#[derive(Debug, Default)]
struct DirSet(Packed);

impl DirSet {
    fn new<'a>(dirs: impl IntoIterator<Item = &'a str>) -> DirSet {
        let mut list: Vec<&str> = dirs.into_iter().collect();
        list.sort_unstable();
        list.dedup();
        let mut packed = Packed::with_capacity(list.iter().map(|d| d.len()).sum(), list.len());
        for dir in list {
            packed.push(dir);
        }
        DirSet(packed)
    }

    fn contains(&self, dir: &str) -> bool {
        self.0.binary_search_by(|item| item.cmp(dir)).is_ok()
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter()
    }
}

impl Files {
    /// 把 `paths` 按 [`path_order`] 排序，只留前 `limit` 个，放进文件表。
    ///
    /// 排序后，不输入查询时列出的结果整齐，按路径查找某个文件也可以二分。只给下标排序，不为每个
    /// 路径另外生成小写形式：那样要多出 20 万个小块内存。
    fn sorted(paths: &Packed, limit: usize) -> Files {
        let mut order: Vec<u32> = (0..paths.len() as u32).collect();
        order.sort_unstable_by(|&a, &b| path_order(paths.get(a as usize), paths.get(b as usize)));
        // 先截断再分配：第 1 层的文件数就超过 `limit` 时，文件表只按收录的文件占用内存，而不是
        // 按列出的全部。
        order.truncate(limit);

        let bytes = order.iter().map(|&i| paths.get(i as usize).len()).sum();
        let mut files = Files::with_capacity(bytes, order.len());
        for index in order {
            files.push(paths.get(index as usize));
        }
        files
    }

    /// 按容量上限（路径的总字节数、文件数）创建空表。
    fn with_capacity(bytes: usize, len: usize) -> Files {
        Files {
            paths: Packed::with_capacity(bytes, len),
            names: Vec::with_capacity(len),
            ..Files::default()
        }
    }

    fn push(&mut self, path: &str) {
        self.names.push(path.rfind('/').map_or(0, |i| i + 1) as u32);
        self.paths.push(path);
    }

    fn len(&self) -> usize {
        self.paths.len()
    }

    fn get(&self, index: usize) -> Option<IndexedFile<'_>> {
        let name = *self.names.get(index)?;
        let path = self.paths.get(index);
        Some(IndexedFile {
            path,
            name: &path[name as usize..],
        })
    }

    /// 返回相对路径 `relative`（`/` 分隔）在表中的下标；不在表中时返回 `None`。
    fn position(&self, relative: &str) -> Option<usize> {
        self.paths
            .binary_search_by(|path| path_order(path, relative))
            .ok()
    }

    /// 是否遍历过目录 `dir`（相对路径；根目录 `""` 总是遍历过）。
    fn walked(&self, dir: &str) -> bool {
        dir.is_empty() || self.dirs.contains(dir)
    }
}

/// 上一次查询及其命中的全部文件。
///
/// 查询继续追加字符时，新查询命中的文件一定在其中（`ab` 是某个路径的子序列，则 `a` 也是），
/// 因此只需重新计算这些文件。
#[derive(Debug, Default)]
struct Narrowing {
    /// 上一次查询的字符（已归一化）。
    needle: Vec<char>,
    /// 上一次命中的文件下标，按下标顺序。
    hits: Vec<u32>,
}

/// 后台每完成一项工作时调用的回调，用来唤醒界面线程。
type Wake = Arc<dyn Fn() + Send + Sync>;

/// 后台要做的事。
enum Job {
    /// 遍历整个目录并重建文件表。
    Walk,
    /// 按这些发生变化的路径更新文件表。
    Patch(Vec<PathBuf>),
}

/// 快速打开的文件索引。
#[derive(Default)]
pub struct FileIndex {
    root: Option<PathBuf>,
    /// 后台更新时先取得可变引用再修改，改好后整张表替换。
    files: Arc<Files>,
    narrowing: Narrowing,
    /// 后台线程检查该标志，决定是否提前结束（换了工作区、关闭窗口、重新遍历时置位）。
    cancel: Option<Arc<AtomicBool>>,
    /// 后台送回已建好、已更新的文件表的通道；表没有变化时送回 `None`。
    receiver: Option<Receiver<Option<Files>>>,
    /// 后台忙碌时收到的变化，后台完成后再处理。
    queued: Vec<PathBuf>,
    /// 后台忙碌时收到的变化超过 [`MAX_QUEUED`]：不再逐个记录，后台完成后整个重新遍历。
    rewalk: bool,
    /// 后台每完成一项工作调用一次，唤醒界面线程来取结果。
    wake: Option<Wake>,
}

impl FileIndex {
    /// 开始为 `root` 建索引。
    ///
    /// 索引先为空；后台遍历完目录后把文件表放进通道，界面线程在 [`FileIndex::poll`] 中取出。
    /// 后台每完成一项工作（建好、按变化更新好）调用一次 `wake`，唤醒事件循环。
    pub fn spawn(root: &Path, wake: impl Fn() + Send + Sync + 'static) -> FileIndex {
        let mut index = FileIndex {
            root: Some(root.to_path_buf()),
            wake: Some(Arc::new(wake)),
            ..FileIndex::default()
        };
        index.start(Job::Walk);
        index
    }

    /// 在当前线程同步建索引（测试用）。
    pub fn collect_now(root: &Path) -> FileIndex {
        let cancel = AtomicBool::new(false);
        let files = collect(root, MAX_FILES, &cancel);
        FileIndex {
            root: Some(root.to_path_buf()),
            files: Arc::new(files),
            ..FileIndex::default()
        }
    }

    /// 把 `job` 交给后台线程执行，并取消正在进行的任务。
    ///
    /// 调用方只在上一项任务完成，或需要整个重新遍历时才调用。
    fn start(&mut self, job: Job) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.cancel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let worker_cancel = Arc::clone(&cancel);
        let base = Arc::clone(&self.files);
        let wake = self.wake.clone();

        // 远端的目录要等待网络，不占线程池中的线程。
        let spawn = if winc_backend::is_remote(&root) {
            background::spawn_blocking
        } else {
            background::spawn
        };
        spawn(Box::new(move || {
            let files = match job {
                Job::Walk => Some(collect(&root, MAX_FILES, &worker_cancel)),
                Job::Patch(paths) => patch(&root, &base, &paths, &worker_cancel),
            };
            // 发送失败说明界面一侧已不再需要结果，直接退出。
            if !worker_cancel.load(Ordering::Relaxed)
                && sender.send(files).is_ok()
                && let Some(wake) = wake
            {
                wake();
            }
        }));

        self.cancel = Some(cancel);
        self.receiver = Some(receiver);
    }

    /// 返回文件表所属的工作区根目录；没有索引时返回 `None`。
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// 返回文件表中的文件数（个）。
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// 上一次查询一共命中了几个文件，是截断到 [`MAX_RESULTS`] 条之前的数；查询为空或已清空时为
    /// `None`。
    pub fn matches(&self) -> Option<usize> {
        let narrowing = &self.narrowing;
        (!narrowing.needle.is_empty()).then_some(narrowing.hits.len())
    }

    /// 文件表是否为空。
    pub fn is_empty(&self) -> bool {
        self.files.len() == 0
    }

    /// 取出后台的结果，并把暂存的变化交给后台处理；文件表被替换时返回 `true`，调用方据此
    /// 重新列出结果。
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };

        let changed = match receiver.try_recv() {
            Ok(Some(files)) => {
                self.files = Arc::new(files);
                self.narrowing = Narrowing::default();
                true
            }
            Ok(None) => false,
            Err(TryRecvError::Empty) => return false,
            // 后台任务已被取消并退出，不会再有结果。
            Err(TryRecvError::Disconnected) => false,
        };
        self.receiver = None;

        if std::mem::take(&mut self.rewalk) {
            self.start(Job::Walk);
        } else if !self.queued.is_empty() {
            let paths = std::mem::take(&mut self.queued);
            self.start(Job::Patch(paths));
        }
        changed
    }

    /// 处理目录中的变化（由 [`crate::Watcher`] 送来）：在后台按变化的路径更新文件表，不整个
    /// 重新遍历。
    ///
    /// 更新完成后与建好时一样唤醒界面来取。系统丢失通知（`changes.rescan`）时整个重新遍历。
    /// 没有根目录（还没有为任何目录建过索引）时什么也不做，下次使用时索引自然是新的。
    pub fn apply(&mut self, changes: Changes) {
        if self.root.is_none() {
            return;
        }
        if changes.rescan {
            // 后台正在遍历或更新时不打断它，完成后再整个重新遍历：编译、安装依赖持续写入时每批
            // 变化都可能要求重新扫描，每次都从头开始时遍历永远无法完成，文件表一直是旧的。
            if self.receiver.is_some() {
                self.queued.clear();
                self.rewalk = true;
            } else {
                self.refresh();
            }
            return;
        }
        if changes.paths.is_empty() {
            return;
        }
        if self.receiver.is_some() {
            // 后台正在遍历或更新：等它完成后再处理这些变化，因为它开始之后发生的变化不一定被看到。
            // 遍历大目录需要数秒，期间持续有程序写文件（安装依赖、编译）时暂存的路径数会
            // 迅速超过 `MAX_QUEUED`，不如完成后整个重新遍历。
            if !self.rewalk {
                self.queued.extend(changes.paths);
                if self.queued.len() > MAX_QUEUED {
                    self.queued = Vec::new();
                    self.rewalk = true;
                }
            }
        } else {
            self.start(Job::Patch(changes.paths));
        }
    }

    /// 索引是否已经就绪（要么建好了，要么本来就不需要建）。
    pub fn is_ready(&self) -> bool {
        self.receiver.is_none()
    }

    /// 停止后台任务（换了工作区或关闭窗口时调用）。
    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// 重新遍历目录。
    ///
    /// 新表建好之前仍使用旧的文件表，面板不会突然变空；暂存的变化不必再处理，重新遍历会包含它们。
    pub fn refresh(&mut self) {
        self.queued.clear();
        self.rewalk = false;
        self.start(Job::Walk);
    }

    /// 清空上一次查询命中的文件列表（每个文件最多 4 字节）；面板关闭后调用，下次输入从头计算。
    pub fn forget_query(&mut self) {
        self.narrowing = Narrowing::default();
    }

    /// 按查询打分排序，返回最多 [`MAX_RESULTS`] 条，每条是文件表下标。
    ///
    /// 查询为空时按路径顺序返回前 [`MAX_RESULTS`] 条：刚打开面板、还没输入时显示整齐的文件列表。
    pub fn search(&mut self, query: &str) -> Vec<usize> {
        let query = normalize(query);

        if query.is_empty() {
            self.narrowing = Narrowing::default();
            // 只取前 `MAX_RESULTS` 条：文件表最多有 `MAX_FILES` 个文件，不必先全部列出再截取。
            return (0..self.files.len().min(MAX_RESULTS)).collect();
        }

        let needle: Vec<char> = query.chars().collect();
        let narrowing = std::mem::take(&mut self.narrowing);
        let candidates = (!narrowing.needle.is_empty() && needle.starts_with(&narrowing.needle))
            .then_some(narrowing.hits.as_slice());
        let mut scored = score_all(&self.files, &needle, candidates);
        self.narrowing = Narrowing {
            hits: scored.iter().map(|&(_, i)| i).collect(),
            needle,
        };

        // 分数高的在前；同分时路径短的在前，仍相同时按路径字典序，使结果稳定。
        let files = &self.files;
        let path = |i: u32| files.get(i as usize).map_or("", |f| f.path);
        let order = |a: &(u32, u32), b: &(u32, u32)| {
            b.0.cmp(&a.0)
                .then_with(|| path(a.1).len().cmp(&path(b.1).len()))
                .then_with(|| path(a.1).cmp(path(b.1)))
        };

        // 只需前 `MAX_RESULTS` 条：先用 O(n) 的选择算法选出，再只排这一小段。短查询（例如一个字母）
        // 在大项目里几乎每条路径都命中，全排序的耗时随文件数增长，会使每次按键都有可感知的延迟。
        if scored.len() > MAX_RESULTS {
            scored.select_nth_unstable_by(MAX_RESULTS - 1, order);
            scored.truncate(MAX_RESULTS);
        }
        scored.sort_by(order);
        scored.into_iter().map(|(_, i)| i as usize).collect()
    }

    /// 返回查询 `query` 在第 `index` 个文件的路径里命中的字符下标（对齐方式与打分时相同），
    /// 面板据此标出命中的字符；不命中或下标越界时返回空 `Vec`。
    pub fn matched(&self, query: &str, index: usize) -> Vec<usize> {
        let needle: Vec<char> = normalize(query).chars().collect();
        self.files
            .get(index)
            .and_then(|file| matched(&needle, file))
            .unwrap_or_default()
    }

    /// 返回绝对路径 `path` 在文件表中的下标；不在打开的目录里或不在表中时返回 `None`。
    pub fn position_of(&self, path: &Path) -> Option<usize> {
        let relative = relative_path(path.strip_prefix(self.root.as_ref()?).ok()?)?;
        self.files.position(&relative)
    }

    /// 返回文件表中第 `index` 个文件（[`FileIndex::search`] 返回的就是这种下标）；越界时
    /// 返回 `None`。
    pub fn file(&self, index: usize) -> Option<IndexedFile<'_>> {
        self.files.get(index)
    }

    /// 返回文件表中第 `index` 个文件的绝对路径；越界或没有根目录时返回 `None`。
    pub fn absolute(&self, index: usize) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let file = self.files.get(index)?;
        Some(root.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR)))
    }
}

#[cfg(test)]
mod tests;
