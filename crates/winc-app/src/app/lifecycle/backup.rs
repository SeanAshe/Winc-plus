//! 热退出：未保存的内容在修改后约 [`BACKUP_DELAY`] 内（内容大时更久，见 [`backup_delay`]）
//! 在后台写入备份，退出时不再逐个询问是否保存，下次启动时原样恢复。
//!
//! 只有退出时不询问；关闭标签、切换目录、关闭目录仍然询问。备份始终在写，关闭热退出
//! （`editor.hotExit` 为 `false`）时也写，使进程中止或断电后仍能找回内容；关闭热退出时退出
//! 仍然询问，询问完成（保存或放弃）后删除备份。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use winc_config::{Session, Tab};
use winc_editor::backup::{self, Backups, Snapshot};
use winc_editor::{Bookmarks, Document};

use super::*;

/// 文档修改后到写入备份之间的等待时间（内容不超过 [`BACKUP_SMALL`] 时）。
///
/// 从第一次出现待写备份起计时，持续输入时不会被推迟，因此约每隔该时长写一次。内容大时等得
/// 更久，见 [`backup_delay`]。
const BACKUP_DELAY: Duration = Duration::from_secs(1);

/// 内容不超过该大小（字节）时按 [`BACKUP_DELAY`] 的间隔写一次。
const BACKUP_SMALL: usize = 4 << 20;

/// 超过 [`BACKUP_SMALL`] 的部分每多 [`BACKUP_BYTES_PER_SECOND`] 字节（8 MiB）多等 1 秒：
/// 64 MiB 的内容约 8.5 秒写一次。
const BACKUP_BYTES_PER_SECOND: usize = 8 << 20;

/// 备份最多等多久（秒）：内容再大也至少按该间隔写一次，进程中止、断电时丢失的不超过这段时间的
/// 修改。
const BACKUP_MAX_DELAY: Duration = Duration::from_secs(30);

/// 待写的备份里最大的内容有 `bytes` 字节时，修改后等多久再写。
///
/// 每次备份都把整份内容写出：几百 KB 的内容 1 秒写一次，开销可以忽略；几十、几百 MB 的
/// 内容每秒重写一遍，磁盘写入和 CPU 开销都很高。超过 [`BACKUP_SMALL`] 以后按大小线性
/// 放慢，封顶 [`BACKUP_MAX_DELAY`]。
fn backup_delay(bytes: usize) -> Duration {
    let over = bytes.saturating_sub(BACKUP_SMALL);
    let extra = Duration::from_secs_f64(over as f64 / BACKUP_BYTES_PER_SECOND as f64);
    (BACKUP_DELAY + extra).min(BACKUP_MAX_DELAY)
}

/// 热退出备份的运行状态。
#[derive(Default)]
pub struct BackupState {
    /// 存放备份的目录（状态文件旁边的 `backups` 目录）。
    ///
    /// 基准测量和测试中为 `None`：不备份，退出时仍然询问。
    pub root: Option<PathBuf>,
    /// 各文档已写入的备份。
    written: HashMap<DocumentId, Written>,
    /// 未命名文档的备份文件名。
    names: HashMap<DocumentId, String>,
    /// 有待写、待删的备份时，统一处理的时刻。
    due: Option<Instant>,
    /// 后台正在写的一批备份；写完后返回写入失败的文档及原因。
    pub(crate) job: Option<mpsc::Receiver<Vec<(DocumentId, String)>>>,
    /// 是否已经提示过写入失败，避免每秒重复提示。
    warned: bool,
    /// 退出过程中已备份好的文档，其备份留到下次启动时恢复。
    ///
    /// 其余文档的备份在询问保存与否之后删除。
    kept: Option<HashSet<DocumentId>>,
    /// 当前占用的备份目录及其锁：（目录，锁）。
    ///
    /// 锁为 `None` 表示该目录已被另一个 winc 进程打开：本进程不读、不写、不删那里的备份，
    /// 退出时仍然询问。
    lock: Option<(PathBuf, Option<backup::Lock>)>,
}

/// [`App::backup_plan`] 的结果：要写的备份（文档、版本号、快照）、要删除的备份文件（所属的
/// 文档、文件），以及无需重写、原样保留的备份所属的文档。
type BackupPlan = (
    Vec<(DocumentId, u64, Snapshot)>,
    Vec<(DocumentId, PathBuf)>,
    Vec<DocumentId>,
);

/// 一个文档已写入的备份。
struct Written {
    /// 备份文件。
    file: PathBuf,
    /// 写入时文档对应的文件；未命名文档为 `None`。
    ///
    /// 备份文件名由该路径算出，路径变了（另存为、改名）时就要换到新的备份文件。
    source: Option<PathBuf>,
    /// 写入时的文档版本号（`revision`）。
    revision: u64,
    /// 写入的头（光标位置、标签序号等，见 [`Snapshot::header`]）；启动时恢复的备份为 `None`。
    header: Option<String>,
}

impl Written {
    /// 这份备份是否仍是 `doc` 在备份目录 `dir` 中的最新备份：版本号、文档路径和所在目录都没变。
    ///
    /// 只比较已有的值，不计算备份文件名的哈希，也不分配内存，每次事件之后都可以调用。
    fn current(&self, doc: &Document, dir: &Path) -> bool {
        self.revision == doc.revision && self.source == doc.path && self.file.parent() == Some(dir)
    }
}

/// 按现场重新打开标签、恢复未保存的内容之后，需要交给界面记录的信息。
#[derive(Default)]
pub(crate) struct Restored {
    /// 已恢复的文档及其备份文件。
    backups: Vec<(DocumentId, PathBuf)>,
    /// 磁盘上的文件在此期间被其他程序修改过的文档名。
    conflicts: Vec<String>,
    /// 占用的备份目录及其锁（见 `BackupState::lock`）。
    lock: Option<(PathBuf, Option<backup::Lock>)>,
}

/// 按上次的现场恢复标签，并把备份中未保存的内容放回文档。
///
/// 标签按原来的顺序重新打开，激活当时的标签，文件树展开到该文件所在的层级；现场中没有记录
/// 的备份（进程中止前来不及记录现场）也会恢复，排在已记录的标签之后。`root` 为备份目录，
/// 为 `None` 时只打开标签；现场中已不存在的文件跳过，除非它还有未保存的内容。
pub(crate) fn restore_tabs(
    editor: &mut Editor,
    session: Option<&Session>,
    root: Option<&Path>,
) -> Restored {
    let backups = root.map(|root| Backups::new(root, editor.root()));
    let lock = backups
        .as_ref()
        .map(|b| (b.dir().to_owned(), b.lock_within(RESTORE_LOCK_WAIT)));
    // 目录被另一个 winc 进程打开时，那里的备份仍由该进程使用，不恢复。
    let mut saved = match (&backups, &lock) {
        (Some(backups), Some((_, Some(_)))) => backups.read_all(),
        _ => Vec::new(),
    };
    let mut restored = Restored {
        lock,
        ..Default::default()
    };
    let mut active = None;
    // 先恢复书签和折叠的块，之后打开各文件时由文件自行取用。
    if let Some(session) = session {
        editor.bookmarks = session
            .bookmarks
            .iter()
            .map(|(path, lines)| (path.clone(), Bookmarks::from_lines(lines.clone())))
            .collect();
        editor.folds = session.folds.iter().cloned().collect();
    }

    // 正文读不出来的备份不恢复（和头坏了的一样跳过），文件原样留在磁盘上。
    let mut put_back = |editor: &mut Editor, backup: backup::Backup| -> Option<usize> {
        let (index, conflict) = editor.restore_backup(&backup)?;
        let doc = &editor.documents[index];
        if conflict {
            restored.conflicts.push(doc.name().into_owned());
        }
        restored.backups.push((doc.id, backup.file));
        Some(index)
    };

    let active_tab = session.and_then(|s| s.active);
    for (i, tab) in session.map_or(&[][..], |s| &s.tabs).iter().enumerate() {
        let index = match tab {
            Tab::File(path) => take(&mut saved, |b| b.path.as_deref() == Some(path.as_path()))
                .and_then(|backup| put_back(editor, backup))
                .or_else(|| path.is_file().then(|| editor.open_pending(path))),
            Tab::Untitled(name) => take(&mut saved, |b| {
                b.path.is_none() && b.file.file_name().is_some_and(|n| *n == **name)
            })
            .and_then(|backup| put_back(editor, backup)),
        };
        if active_tab == Some(i) {
            active = index.map(|i| editor.documents[i].id);
        }
    }
    for backup in saved {
        put_back(editor, backup);
    }

    // 当时的当前标签已不存在时，改为最后一个标签（它可能还没读）。
    editor.active = active
        .and_then(|id| editor.index_of(id))
        .or(editor.documents.len().checked_sub(1));
    let current = editor.active().and_then(|d| d.path.clone());
    if let (Some(file), Some(workspace)) = (current, &mut editor.workspace) {
        workspace.reveal(&file);
    }
    restored
}

/// 启动时恢复上次的现场：同 [`restore_tabs`]，并立即读取当前标签，使第一帧即显示其内容。
pub(crate) fn restore_at_launch(
    editor: &mut Editor,
    session: Option<&Session>,
    root: Option<&Path>,
) -> Restored {
    let restored = restore_tabs(editor, session, root);
    editor.load_active();
    restored
}

/// 恢复标签时等待备份锁的最长时间（经验值）。
///
/// 本进程启动外部程序时，子进程在 fork 之后、exec 之前短暂持有锁文件的描述符，锁在此期间
/// 取不到；等待超过该时间仍取不到时，才认为目录由另一个 winc 进程使用。
const RESTORE_LOCK_WAIT: Duration = Duration::from_millis(200);

/// 移除并返回 `items` 中第一个满足 `wanted` 的元素；没有时返回 `None`。
fn take<T>(items: &mut Vec<T>, wanted: impl Fn(&T) -> bool) -> Option<T> {
    let index = items.iter().position(wanted)?;
    Some(items.remove(index))
}

impl App {
    /// 返回当前目录（没有打开目录时为空窗口）的备份位置。
    ///
    /// 目录改变时改为占用新目录的备份锁。没有备份目录，或另一个 winc 进程占用该目录时返回
    /// `None`，该目录暂不备份；之后每次到写备份的时刻再试一次，对方退出后由本进程接手。
    /// 加锁要建目录、打开锁文件，只在真正要写备份时调用。
    fn backups(&mut self) -> Option<Backups> {
        let backups = Backups::new(self.backup.root.as_deref()?, self.editor.root());
        match &self.backup.lock {
            Some((dir, held)) if dir == backups.dir() => {
                if held.is_none()
                    && let Some(lock) = backups.lock()
                {
                    self.adopt_lock((backups.dir().to_owned(), Some(lock)));
                }
            }
            _ => {
                let lock = (backups.dir().to_owned(), backups.lock());
                self.adopt_lock(lock);
            }
        }
        self.backup.lock.as_ref()?.1.as_ref()?;
        Some(backups)
    }

    /// 记录占用备份目录的锁；打开的目录没有取到锁（`lock.1` 为 `None`）时提示用户。
    ///
    /// 没打开目录的窗口共用一个「空窗口」的备份目录，那里取不到锁只是不备份未命名的新文件，
    /// 不提示：「目录已在另一个窗口中打开」的提示是错误的。
    fn adopt_lock(&mut self, lock: (PathBuf, Option<backup::Lock>)) {
        if lock.1.is_none() && self.editor.root().is_some() {
            self.notify(winc_i18n::t!("workspace-backup-locked"));
        }
        self.backup.lock = Some(lock);
    }

    /// 记录刚恢复的文档已有备份，不需要立即重写；对磁盘上的文件在此期间被修改过的文档，提示用户。
    pub(crate) fn adopt_restored(&mut self, restored: Restored) {
        if let Some(lock) = restored.lock {
            self.adopt_lock(lock);
        }
        for (id, file) in restored.backups {
            let Some(doc) = self.editor.document(id) else {
                continue;
            };
            if doc.path.is_none()
                && let Some(name) = file.file_name()
            {
                let name = name.to_string_lossy().into_owned();
                self.backup.names.insert(id, name);
            }
            let revision = doc.revision;
            self.mark_written(id, file, revision, None);
        }
        if !restored.conflicts.is_empty() {
            let names: Vec<String> = restored
                .conflicts
                .iter()
                .map(|n| winc_i18n::t!("workspace-quoted-name", name = n.as_str()))
                .collect();
            let names = names.join(winc_i18n::t!("workspace-list-separator"));
            self.notify(winc_i18n::t!("workspace-disk-conflicts", names = names));
        }
    }

    /// 返回第 `index` 个文档的备份文件路径；未命名文档在第一次调用时分配备份文件名。
    ///
    /// # Panics
    ///
    /// `index` 超出 `documents` 的范围时 panic。
    fn backup_file(&mut self, backups: &Backups, index: usize) -> PathBuf {
        let doc = &self.editor.documents[index];
        let name = if doc.path.is_none() {
            self.backup
                .names
                .entry(doc.id)
                .or_insert_with(backup::untitled_name)
                .as_str()
        } else {
            ""
        };
        backups.file_for(doc, name)
    }

    /// 记下文档 `id` 已按版本号 `revision` 备份到 `file`，同时记下文档此刻的路径（见
    /// [`Written`]）。
    fn mark_written(
        &mut self,
        id: DocumentId,
        file: PathBuf,
        revision: u64,
        header: Option<String>,
    ) {
        let source = self.editor.document(id).and_then(|d| d.path.clone());
        let written = Written {
            file,
            source,
            revision,
            header,
        };
        self.backup.written.insert(id, written);
    }

    /// 返回要写的备份、要删除的备份文件，以及无需重写、原样保留的备份所属的文档。
    ///
    /// `force` 为 `true` 时（退出、关闭窗口），内容没有修改的文档在光标位置、标签顺序等头部信息
    /// 变化时也重写；头部也相同时原样保留，不重写正文（正文可能很大，写入耗时与大小成正比）。
    fn backup_plan(&mut self, force: bool) -> BackupPlan {
        let mut writes = Vec::new();
        let mut removes = Vec::new();
        let mut unchanged = Vec::new();
        let Some(backups) = self.backups() else {
            return (writes, removes, unchanged);
        };

        for index in 0..self.editor.documents.len() {
            let doc = &self.editor.documents[index];
            let id = doc.id;
            if !Backups::wanted(doc) {
                if let Some(old) = self.backup.written.remove(&id) {
                    removes.push((id, old.file));
                }
                continue;
            }
            let current =
                (self.backup.written.get(&id)).is_some_and(|w| w.current(doc, backups.dir()));
            if !force && current {
                continue;
            }
            let file = self.backup_file(&backups, index);
            let doc = &self.editor.documents[index];
            // 路径变化（另存为、改名、切换目录）时，删除旧路径下的备份。
            if let Some(old) = self.backup.written.get(&id)
                && old.file != file
            {
                removes.push((id, old.file.clone()));
            }
            let name = self.backup.names.get(&id).map_or("", String::as_str);
            let Some(snapshot) = backups.snapshot(doc, name, index) else {
                continue;
            };
            let header = self
                .backup
                .written
                .get(&id)
                .and_then(|w| w.header.as_deref());
            if current && header == Some(snapshot.header()) {
                unchanged.push(id);
            } else {
                writes.push((id, doc.revision, snapshot));
            }
        }

        // 已关闭的标签：删除其备份，并清除名称记录。
        let open: HashSet<DocumentId> = self.editor.documents.iter().map(|d| d.id).collect();
        let closed: Vec<DocumentId> = self
            .backup
            .written
            .keys()
            .filter(|id| !open.contains(id))
            .copied()
            .collect();
        for id in closed {
            if let Some(old) = self.backup.written.remove(&id) {
                removes.push((id, old.file));
            }
            self.backup.names.remove(&id);
        }
        (writes, removes, unchanged)
    }

    /// 是否有待写或待删的备份。
    ///
    /// 每次事件之后都会调用，只比较版本号、文档路径和备份目录（见 [`Written::current`]），不加锁、
    /// 不碰磁盘。备份目录被另一个 winc 进程占用时同样按此判断：到了写的时刻 [`App::backup_plan`]
    /// 再试着加锁，取不到就什么也不写。
    pub(super) fn backup_pending(&self) -> bool {
        let Some(root) = self.backup.root.as_deref() else {
            return false;
        };
        let backups = Backups::new(root, self.editor.root());
        let mut tracked = 0;
        let changed = self.editor.documents.iter().any(|doc| {
            let wanted = Backups::wanted(doc);
            match self.backup.written.get(&doc.id) {
                None => wanted,
                Some(written) => {
                    tracked += 1;
                    !wanted || !written.current(doc, backups.dir())
                }
            }
        });
        // 文档的标识各不相同：有备份的文档少于备份记录，说明已关闭标签的备份尚未删除。
        changed || tracked < self.backup.written.len()
    }

    /// 待写的备份里最大的那份内容的字节数（等多久再写按它算，见 [`backup_delay`]）。
    fn largest_pending_backup(&self) -> usize {
        self.editor
            .documents
            .iter()
            .filter(|doc| Backups::wanted(doc))
            .filter(|doc| {
                self.backup
                    .written
                    .get(&doc.id)
                    .is_none_or(|written| written.revision != doc.revision)
            })
            .filter_map(|doc| doc.text().rope().map(|rope| rope.len_bytes()))
            .max()
            .unwrap_or(0)
    }

    /// 事件循环空闲时调用：文档修改后等待一段时间（[`backup_delay`]，内容越大等待越久），再在后台
    /// 写备份。
    ///
    /// 返回下一次需要调用的时刻；无需再调用时返回 `None`。
    pub(crate) fn backup_tick(&mut self, now: Instant) -> Option<Instant> {
        self.collect_backups(false);
        // 已经提出退出请求时，备份已在退出流程中写好，不再改动。
        if self.closing.quit_requested {
            return None;
        }
        if !self.backup_pending() {
            self.backup.due = None;
            return None;
        }
        // 后台仍在写上一批：写完时 `UserEvent::BackedUp` 唤醒界面，收下结果以后再来这里。
        if self.backup.job.is_some() {
            return None;
        }
        let delay = backup_delay(self.largest_pending_backup());
        let due = *self.backup.due.get_or_insert(now + delay);
        if now < due {
            return Some(due);
        }
        self.backup.due = None;

        let (writes, removes, _) = self.backup_plan(false);
        for (id, revision, snapshot) in &writes {
            let header = Some(snapshot.header().to_owned());
            self.mark_written(*id, snapshot.file.clone(), *revision, header);
        }
        let (send, receive) = mpsc::channel();
        self.backup.job = Some(receive);
        self.spawn_then(UserEvent::BackedUp, move || {
            let _ = send.send(write_backups(writes, removes));
        });
        None
    }

    /// 收取后台写完的一批备份的结果；`wait` 为 `true` 时阻塞等待写完。
    ///
    /// 写入失败的文档下次重写，并在第一次失败时提示用户。
    pub(crate) fn collect_backups(&mut self, wait: bool) {
        let Some(job) = &self.backup.job else {
            return;
        };
        let failed = if wait {
            job.recv().ok()
        } else {
            match job.try_recv() {
                Ok(failed) => Some(failed),
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => None,
            }
        };
        self.backup.job = None;
        self.backup_failed(failed.unwrap_or_default());
    }

    /// 处理写入失败的备份 `failed`：清除这些文档的已写记录以便下次重写，并只在第一次失败时
    /// 提示用户。
    fn backup_failed(&mut self, failed: Vec<(DocumentId, String)>) {
        let Some((_, reason)) = failed.first().cloned() else {
            return;
        };
        for (id, _) in &failed {
            self.backup.written.remove(id);
        }
        if !self.backup.warned {
            self.backup.warned = true;
            self.notify(winc_i18n::t!(
                "workspace-backup-failed",
                error = reason.to_string()
            ));
        }
    }

    /// 文档 `doc` 退出时是否会写进备份、不必询问：开着热退出，文档可以备份，且之前已为它写过
    /// 备份（备份目录可用，写入没有失败）。
    ///
    /// 只看内存里的状态，不碰磁盘；每处理完一批事件都调用。
    pub(crate) fn kept_on_quit(&self, doc: &winc_editor::Document) -> bool {
        self.config.hot_exit != Some(false)
            && Backups::wanted(doc)
            && self.backup.written.contains_key(&doc.id)
    }

    /// 退出前备份全部未保存的内容：等待后台正在写的一批完成，再同步写入内容或头部有变化的文档
    /// （见 [`App::backup_plan`]）。
    ///
    /// 返回已备份、退出时无需再询问的文档。没有备份目录，或关闭了热退出时返回空集合，
    /// 所有未保存的文档仍然询问。
    pub(super) fn back_up_for_quit(&mut self) -> HashSet<DocumentId> {
        self.collect_backups(true);
        if self.config.hot_exit == Some(false) {
            self.backup.kept = Some(HashSet::new());
            return HashSet::new();
        }
        let (writes, removes, unchanged) = self.backup_plan(true);
        let mut done: HashSet<DocumentId> = unchanged.into_iter().collect();
        let mut failed = Vec::new();
        for (id, revision, snapshot) in writes {
            match snapshot.write() {
                Ok(()) => {
                    done.insert(id);
                    let header = Some(snapshot.header().to_owned());
                    self.mark_written(id, snapshot.file, revision, header);
                }
                Err(e) => failed.push((id, e.to_string())),
            }
        }
        let wrote: Vec<&Path> = self
            .backup
            .written
            .values()
            .map(|w| w.file.as_path())
            .collect();
        remove_stale(removes, &wrote, &failed);
        self.backup_failed(failed);
        self.backup.kept = Some(done.clone());
        done
    }

    /// 确认退出时调用（询问过的文档都已保存或放弃）：删除除 `kept` 之外的所有备份。
    ///
    /// `kept` 中的备份留到下次启动时恢复。
    pub(super) fn quit_backups(&mut self) {
        let kept = self.backup.kept.take().unwrap_or_default();
        let gone: Vec<DocumentId> = self
            .backup
            .written
            .keys()
            .filter(|id| !kept.contains(id))
            .copied()
            .collect();
        for id in gone {
            if let Some(old) = self.backup.written.remove(&id) {
                let _ = backup::remove(&old.file);
            }
        }
    }

    /// 把已写好的备份交给磁盘保管，同时放开备份目录的锁。
    ///
    /// 本进程不再记着这些备份，之后关闭标签时不会删除，以后打开该目录时由 `restore_tabs` 读回；
    /// 锁放开以后，在本进程中重新打开该目录时也能再取到锁。在已记下现场、准备关闭所有标签时
    /// 调用（macOS 上关闭窗口）。
    pub(super) fn leave_backups_on_disk(&mut self) {
        self.collect_backups(true);
        self.backup.written.clear();
        self.backup.names.clear();
        self.backup.kept = None;
        self.backup.due = None;
        self.release_folder_lock();
    }

    /// 放开目录的备份锁。
    ///
    /// 关闭目录、关闭窗口时调用。如果保留锁，之后在本窗口重新打开该目录时自己的锁就无法取得
    /// （`flock` 在同一个进程里也互斥），会提示「已在另一个 winc 窗口中打开」，上次的备份也
    /// 不恢复。
    pub(in crate::app) fn release_folder_lock(&mut self) {
        self.backup.lock = None;
    }

    /// 返回 `doc` 在现场中的记录方式；不需要记录时返回 `None`。
    ///
    /// 有路径的文档记录路径（文件已不在磁盘上但仍有未保存内容的也记录）；从未保存过的新文档
    /// 只有存在备份时才记录为未命名标签。
    pub(in crate::app) fn session_tab(&self, doc: &winc_editor::Document) -> Option<Tab> {
        let backed_up = self.backup.written.contains_key(&doc.id);
        match &doc.path {
            // 发行版暂时无法访问时无法判断文件是否存在，保留标签。
            Some(path) if backed_up || path.is_file() || !winc_editor::wsl::reachable(path) => {
                Some(Tab::File(path.clone()))
            }
            Some(_) => None,
            None if backed_up => self.backup.names.get(&doc.id).cloned().map(Tab::Untitled),
            None => None,
        }
    }
}

/// 先写入 `writes` 中的备份，再删除 `removes` 中过时的备份文件（见 [`remove_stale`]），返回
/// 写入失败的文档及原因。
///
/// 先写后删：路径变化（另存为、改名）的文档在两步之间中止时，新旧两份备份都保留在磁盘上。
/// 在后台线程中调用。
fn write_backups(
    writes: Vec<(DocumentId, u64, Snapshot)>,
    removes: Vec<(DocumentId, PathBuf)>,
) -> Vec<(DocumentId, String)> {
    let failed: Vec<(DocumentId, String)> = writes
        .iter()
        .filter_map(|(id, _, snapshot)| snapshot.write().err().map(|e| (*id, e.to_string())))
        .collect();
    let wrote: Vec<&Path> = writes.iter().map(|(_, _, s)| s.file.as_path()).collect();
    remove_stale(removes, &wrote, &failed);
    failed
}

/// 删除过时的备份文件 `removes`，跳过以下两种：
///
/// - 文件就是刚写的备份之一（`wrote`）：关闭的标签和新打开的标签对应同一个
///   文件时，删掉的是新写的备份。
/// - 所属文档这一次写新备份失败（`failed`）：旧备份是该文档仅有的备份。
fn remove_stale(
    removes: Vec<(DocumentId, PathBuf)>,
    wrote: &[&Path],
    failed: &[(DocumentId, String)],
) {
    for (id, file) in removes {
        if wrote.contains(&file.as_path()) || failed.iter().any(|(f, _)| *f == id) {
            continue;
        }
        let _ = backup::remove(&file);
    }
}

#[cfg(test)]
mod tests;
