//! 监视已打开的目录：其他程序新建、删除、改名或改写文件时，增量更新界面。
//!
//! 受影响的部分包括文件树、`⌘P` 面板的文件表、已打开的文档，以及全局搜索结果中涉及的文件。
//! 各部分只处理变化涉及的路径，不需要切换窗口，也不需要重新遍历整个目录。
//!
//! 收到系统通知后立即唤醒界面，但等待 [`SETTLE`] 之后才处理：切换分支、解压、格式化整个
//! 项目等操作产生的密集改动合并为一批，只更新一次。

use std::sync::{PoisonError, mpsc};
use std::time::{Duration, Instant};

use winc_editor::{
    Changes, DirListing, DiskChange, DiskRead, DiskSync, DiskTaken, EncodingRead, Watcher,
};

use super::*;
use crate::app::tickets::Tickets;

/// 收到文件变化通知后到开始处理之前的等待时间。
///
/// 取 100 ms：该值越大，合并的连续通知越多，界面反映变化的时间越晚。
const SETTLE: Duration = Duration::from_millis(100);

/// 后台返回的一次结果：文档、读取代号、结果。
type ReadBack = (DocumentId, u64, DiskBack);

/// 后台对一个文档完成的一步磁盘检查。
pub(crate) enum DiskBack {
    /// 读到的文件元数据（文件不存在或无法访问时为 `None`），交给 `Document::check_disk_with`。
    Checked(Option<winc_backend::fs::Metadata>),
    /// 读到的文件内容（末尾追加的部分或整个文件），交给 `Document::apply_reread`。
    Read(DiskRead),
    /// 按用户选择的编码读到的文件内容，交给 `Document::finish_reopen_with_encoding`。
    Reencoded(EncodingRead),
    /// 「使用磁盘版本」读到的内容和比较结果，交给 `Document::finish_take_disk_version`。
    Taken(DiskTaken),
}

/// 在后台检查、重读被其他程序修改过的文件。
///
/// 界面线程不访问磁盘：网络盘、外接硬盘刚唤醒时，读取元数据都可能阻塞很久；整个重读（读文件、
/// 解码、建 rope）和读取末尾追加的大段内容都与文件大小成正比。先在后台读元数据，界面线程据此
/// 判断文件没有变化、存在冲突还是需要读取；需要读取时再交给后台，读好之后核对文档期间没有变化
/// 才换上。
pub(crate) struct Rereads {
    /// 各文档最近一次发出的检查或重读：同一个文件连续被修改时只采用最后一次。
    pub(in crate::app) tickets: Tickets<DocumentId>,
    /// 各文档最近一次由用户发起的读取：「通过编码重新打开」和「使用磁盘版本」。
    ///
    /// 与 `tickets` 分开：确认框关闭后窗口重新获得焦点，会对所有文档发出检查，共用读取代号时
    /// 这次检查会使用户发起的读取作废。
    pub(in crate::app) reencodes: Tickets<DocumentId>,
    /// 后台检查、重读结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    pub(in crate::app) results: (mpsc::Sender<ReadBack>, mpsc::Receiver<ReadBack>),
}

impl Default for Rereads {
    fn default() -> Self {
        Self {
            tickets: Tickets::default(),
            reencodes: Tickets::default(),
            results: mpsc::channel(),
        }
    }
}

/// 后台启动好的监视：目录和监视器，启动失败时为 `None`。
pub(in crate::app) type Started = Arc<Mutex<Vec<(PathBuf, Option<Watcher>)>>>;

/// 后台重读的一批目录：各目录的读取代号（按读取顺序）、目录内容，和读好后要继续做的事的编号。
type ScanBack = (Vec<u64>, DirListing, Option<u64>);

/// 文件树重读完成后在界面线程上继续做的事（选中、展开到某一行）。
type AfterScan = Box<dyn FnOnce(&mut App)>;

/// 在后台重读的、文件树中被其他程序修改过的目录。
///
/// 界面线程只挑出需要读取的目录，读好后再换入缓存。
pub(crate) struct TreeScans {
    /// 各目录最近一次发出的重读：同一个目录连续被修改时只采用最后一次，不同目录互不影响。
    pub(in crate::app) tickets: Tickets<PathBuf>,
    /// 后台重读结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    pub(in crate::app) results: (mpsc::Sender<ScanBack>, mpsc::Receiver<ScanBack>),
    /// 等某一批重读读好后要做的事：编号、是否改为等下一批（这一批里有目录被更新的重读取代）。
    after: Vec<(u64, bool, AfterScan)>,
    /// 已经发出过几件要继续做的事，用作编号。
    issued: u64,
    /// 后台为展开、定位读好的目录的通道，和各自读好后要做的事（按编号）。
    fetched: (mpsc::Sender<FetchBack>, mpsc::Receiver<FetchBack>),
    then: Vec<(u64, AfterFetch)>,
    /// 最近一次定位的编号：更早发出的读好后不再定位。
    reveals: u64,
    /// 定位的路径还在等目录读好，读好选中后要滚到那里。
    scroll_to_revealed: bool,
}

/// 后台为展开、定位读好的目录：要继续做的事的编号和读到的内容。
type FetchBack = (u64, winc_editor::DirFetched);

/// 目录读好并放进文件树的缓存后要做的事，参数是递归展开的范围。
type AfterFetch = Box<dyn FnOnce(&mut App, winc_editor::Expansion)>;

impl TreeScans {
    /// 丢弃等目录读好后要做的事：换了打开的目录，它们说的行和路径属于原来的目录。
    pub(in crate::app) fn forget_follow_ups(&mut self) {
        self.after.clear();
        self.then.clear();
    }
}

impl Default for TreeScans {
    fn default() -> Self {
        Self {
            tickets: Tickets::default(),
            results: mpsc::channel(),
            after: Vec::new(),
            issued: 0,
            fetched: mpsc::channel(),
            then: Vec::new(),
            reveals: 0,
            scroll_to_revealed: false,
        }
    }
}

impl App {
    /// 开始监视已打开的目录；目录改变时改为监视新目录。
    ///
    /// 没有事件循环（测试中）时不监视，测试直接把变化传给 [`App::apply_changes`]。
    pub(in crate::app) fn watch_folder(&mut self) {
        let root = self.editor.root().map(Path::to_path_buf);
        if self.disk.watcher.as_ref().map(Watcher::root) == root.as_deref() {
            return;
        }
        self.disk.watcher = None;
        self.disk.changes_due = None;
        // 换了目录或关闭了目录：原来附加的目录不再监视。
        self.watch_extra_folders();
        self.git_folder_changed();
        let Some(root) = root.filter(|_| self.link.proxy.is_some()) else {
            return;
        };
        // 启动在后台进行：远端要发出请求，本机在有的系统上要走遍整棵目录树。启动失败（例如超出
        // 系统允许的监视数量）时没有监视，由 `check_disk` 在窗口重新获得焦点时检查。
        self.start_watcher(root);
        self.index_folder();
    }

    /// 发行版内的监视程序监视过后退出了、发行版已恢复时重新启动监视；开始监视后程序要求重新扫描。
    pub(in crate::app) fn restart_exited_watcher(&mut self) {
        let root = (self.disk.watcher.as_ref())
            .filter(|watcher| watcher.exited())
            .map(|watcher| watcher.root().to_path_buf());
        let Some(root) = root.filter(|_| self.link.proxy.is_some()) else {
            return;
        };
        // 远端的监视随连接中断而结束：连接恢复后重新启动。
        let reachable = match winc_backend::locate(&root) {
            Some(at) => winc_backend::remote::is_connected(&at.target),
            None => winc_editor::wsl::reachable(&root),
        };
        if reachable {
            self.disk.watcher = None;
            self.start_watcher(root);
        }
    }

    /// 在后台启动对目录 `folder` 的监视，启动好后由 [`App::poll_watchers`] 接收；已经在启动
    /// 的不再启动一次。
    ///
    /// 启动失败时没有监视，变化由 `check_disk` 在窗口重新获得焦点时检查。
    fn start_watcher(&mut self, folder: PathBuf) {
        let Some(proxy) = self.link.proxy.clone() else {
            return;
        };
        if self.disk.starting.contains(&folder) {
            return;
        }
        self.disk.starting.push(folder.clone());
        let started = Arc::clone(&self.disk.started);
        self.spawn_blocking_then(UserEvent::WatcherStarted, move || {
            let wake = move || {
                let _ = proxy.send_event(UserEvent::FilesChanged);
            };
            let watcher = Watcher::start(&folder, wake).ok();
            let mut started = started.lock().unwrap_or_else(PoisonError::into_inner);
            started.push((folder, watcher));
        });
    }

    /// 接收后台启动好的监视：打开的目录、附加的目录和展开的链接目录各归其位；期间不再需要
    /// 监视的目录（换了目录、收起了链接）丢弃，监视随之结束。
    ///
    /// 启动期间磁盘上的变化没有通知：启动好后重读文件树，检查打开的文档。
    pub(in crate::app) fn poll_watchers(&mut self) {
        let started = {
            let mut started = (self.disk.started.lock()).unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *started)
        };
        let mut extra = self.extra_folders();
        extra.extend(self.expanded_links());
        let mut watching = false;
        for (folder, watcher) in started {
            self.disk.starting.retain(|starting| *starting != folder);
            let Some(watcher) = watcher else {
                continue;
            };
            let watched = |w: &Watcher| w.root() == folder;
            if Some(folder.as_path()) == self.editor.root() {
                self.disk.watcher = Some(watcher);
                watching = true;
            } else if extra.contains(&folder) && !self.disk.extra_watchers.iter().any(watched) {
                self.disk.extra_watchers.push(watcher);
                watching = true;
            }
        }
        if watching {
            self.rescan_tree(|_| {});
            self.sync_documents(|_| true);
        }
    }

    /// 记录系统发来的目录变化通知，并在 [`SETTLE`] 之后统一处理。
    ///
    /// 已有等待中的批次时不重新计时。
    pub(in crate::app) fn files_changed(&mut self) {
        if self.disk.changes_due.is_none() {
            self.disk.changes_due = Some(Instant::now() + SETTLE);
        }
    }

    /// 到达处理时间后处理已累积的变化，返回下一次需要检查的时刻；没有待处理的批次时返回 `None`。
    pub(in crate::app) fn changes_tick(&mut self, now: Instant) -> Option<Instant> {
        let due = self.disk.changes_due?;
        if now < due {
            return Some(due);
        }
        self.disk.changes_due = None;
        self.preview.markdown.links.clear();
        if self.disk.watcher.as_ref().is_some_and(Watcher::take_limit) {
            let name = self.editor.root().map(winc_editor::display_name);
            let name = name.as_deref().unwrap_or_default();
            self.notify(
                NoticeKind::Warning,
                winc_i18n::t!("workspace-watch-limit", name = name),
            );
        }
        if let Some(changes) = self.disk.watcher.as_ref().map(Watcher::take) {
            self.apply_changes(changes);
        }
        let limited: Vec<PathBuf> = (self.disk.extra_watchers.iter())
            .filter(|watcher| watcher.take_limit())
            .map(|watcher| watcher.root().to_path_buf())
            .collect();
        for folder in limited {
            let name = winc_editor::display_name(&folder);
            self.notify(
                NoticeKind::Warning,
                winc_i18n::t!("workspace-watch-limit", name = name),
            );
        }
        let extra: Vec<Changes> = self.disk.extra_watchers.iter().map(Watcher::take).collect();
        for changes in extra {
            self.apply_changes(changes);
        }
        None
    }

    /// 使附加目录的监视与工作区一致：新添加的开始监视，已移除的停止。
    ///
    /// 文件树中展开了的链接目录同样各自监视：打开的目录的监视不跟随符号链接，链接指向的目录
    /// 只在用户展开它时才监视。没有事件循环（测试中）时不监视。监视启动失败的目录在窗口重新
    /// 获得焦点时由 `check_disk` 检查。
    pub(in crate::app) fn watch_extra_folders(&mut self) {
        let mut folders = self.extra_folders();
        folders.extend(self.expanded_links());
        (self.disk.extra_watchers).retain(|watcher| folders.iter().any(|f| f == watcher.root()));
        for folder in folders {
            if !self.disk.extra_watchers.iter().any(|w| w.root() == folder) {
                self.start_watcher(folder);
            }
        }
    }

    /// 按 `changes` 更新界面。
    ///
    /// - 文件树：重读受影响的目录。
    /// - `⌘P` 面板：在后台更新文件表。
    /// - 已打开的文档：从磁盘重新读取；有未保存修改的文档提示冲突。
    /// - 全局搜索结果：重新搜索变化涉及的文件。
    pub(in crate::app) fn apply_changes(&mut self, changes: Changes) {
        if changes.is_empty() {
            return;
        }

        if let Some(ws) = &self.editor.workspace {
            let scan = ws.scan((!changes.rescan).then_some(changes.paths.as_slice()));
            if !scan.dirs().is_empty() {
                self.spawn_scan(scan, None);
            }
        }

        self.run_configs_changed(changes.rescan, &changes.paths);
        self.sync_documents(|path| changes.rescan || changes.paths.iter().any(|p| p == path));
        // 全局搜索只重搜变化涉及的文件：打开的目录较大（例如整个主目录）时，其他程序持续写入，
        // 每次都全部重搜的开销过大。只有系统报告丢失通知（`rescan`）时才全部重搜。
        if changes.rescan {
            self.files_changed_under_results();
        } else {
            self.results_touched(&changes.paths);
        }
        self.git_paths_changed(&changes.paths, changes.rescan, changes.git);
        self.overlays.picker.index.apply(changes);

        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 把已打开的文档中 `touched` 返回 `true` 的那些与磁盘内容同步。
    ///
    /// 只在后台读取元数据（见 [`Rereads`]），读好后由 [`App::poll_rereads`] 判断：没有未保存修改
    /// 的文档在后台重新读取；有未保存修改的文档标记为冲突，当前文档由冲突卡片提示（见
    /// `conflict.rs`），其余标签在右上角提示一次。窗口重新获得焦点时，文件可能在未被监视的位置
    /// 被修改，因此对所有文档调用。
    pub(in crate::app) fn sync_documents(&mut self, touched: impl Fn(&Path) -> bool) {
        let mut checks = Vec::new();
        for doc in &self.editor.documents {
            // 正在后台保存的文档不同步：写出的文件来自本程序，保存完成后再检查
            // （见 `background_save.rs`）。
            let Some(path) = doc.path.as_deref().filter(|p| touched(p)) else {
                continue;
            };
            if !self.disk.saves.running.contains(&doc.id) {
                let ticket = self.disk.rereads.tickets.issue(doc.id);
                checks.push((doc.id, ticket, path.to_path_buf()));
            }
        }
        if checks.is_empty() {
            return;
        }
        let sender = self.disk.rereads.results.0.clone();
        self.spawn_then(UserEvent::Reread, move || {
            // 各文档的文件信息一次取回：远端路径由一次请求完成。
            let paths: Vec<&Path> = checks.iter().map(|(_, _, path)| path.as_path()).collect();
            let found = winc_backend::fs::metadata_many(&paths);
            for ((id, ticket, _), metadata) in checks.iter().zip(found) {
                let _ = sender.send((*id, *ticket, DiskBack::Checked(metadata.ok())));
            }
        });
    }

    /// 在后台重读文件树中全部已缓存的目录，读好换入后在界面线程上调用 `after`。
    ///
    /// 用于本程序刚改过磁盘（删除、复制、粘贴）或用户要求刷新时；没有打开目录时不调用 `after`。
    pub(in crate::app) fn rescan_tree(&mut self, after: impl FnOnce(&mut App) + 'static) {
        let Some(scan) = self.editor.workspace.as_ref().map(|ws| ws.scan(None)) else {
            return;
        };
        let scans = &mut self.panels.tree.scans;
        scans.issued += 1;
        let batch = scans.issued;
        scans.after.push((batch, false, Box::new(after)));
        self.spawn_scan(scan, Some(batch));
    }

    /// 在后台执行 `scan`，读好后由 [`App::poll_tree_scans`] 换入；`batch` 是读好后要做的事的编号。
    fn spawn_scan(&mut self, scan: winc_editor::DirScan, batch: Option<u64>) {
        let tickets = (scan.dirs().iter())
            .map(|dir| self.panels.tree.scans.tickets.issue(dir.clone()))
            .collect();
        let sender = self.panels.tree.scans.results.0.clone();
        self.spawn_then(UserEvent::TreeScanned, move || {
            let _ = sender.send((tickets, scan.run(), batch));
        });
    }

    /// 在后台执行 `fetch`，读到的目录放进文件树的缓存后在界面线程上调用 `after`。
    ///
    /// 用于展开目录、定位文件之前；期间换了打开的目录时不调用 `after`。
    pub(in crate::app) fn fetch_dirs_then(
        &mut self,
        fetch: winc_editor::DirFetch,
        after: impl FnOnce(&mut App, winc_editor::Expansion) + 'static,
    ) {
        let scans = &mut self.panels.tree.scans;
        scans.issued += 1;
        let id = scans.issued;
        scans.then.push((id, Box::new(after)));
        let sender = scans.fetched.0.clone();
        self.spawn_then(UserEvent::TreeScanned, move || {
            let _ = sender.send((id, fetch.run()));
        });
    }

    /// 收取后台为展开、定位读好的目录（见 [`App::fetch_dirs_then`]），放进缓存后继续做该做的事。
    fn poll_tree_fetches(&mut self) {
        while let Ok((id, fetched)) = self.panels.tree.scans.fetched.1.try_recv() {
            let then = &mut self.panels.tree.scans.then;
            let Some(at) = then.iter().position(|(issued, _)| *issued == id) else {
                continue;
            };
            let (_, after) = then.remove(at);
            let workspace = self.editor.workspace.as_mut();
            if let Some(expansion) = workspace.and_then(|ws| ws.adopt(fetched)) {
                after(self, expansion);
            }
        }
    }

    /// 记下一次新的定位并返回它的编号；见 [`App::is_latest_reveal`]。
    pub(in crate::app) fn next_reveal(&mut self) -> u64 {
        let scans = &mut self.panels.tree.scans;
        scans.reveals += 1;
        scans.scroll_to_revealed = false;
        scans.reveals
    }

    /// 编号为 `reveal` 的定位之后是否没有再发出别的定位。
    pub(in crate::app) fn is_latest_reveal(&self, reveal: u64) -> bool {
        self.panels.tree.scans.reveals == reveal
    }

    /// 定位的路径所在的目录读好、选中它之后滚到那里。
    pub(in crate::app) fn scroll_once_revealed(&mut self) {
        self.panels.tree.scans.scroll_to_revealed = true;
    }

    /// 收取后台重读完成的目录（见 [`TreeScans`]）并换入文件树；之后又发出过重读的目录不换。
    pub(in crate::app) fn poll_tree_scans(&mut self) {
        let mut changed = false;
        let mut revealed = false;
        while let Ok((tickets, mut listing, batch)) = self.panels.tree.scans.results.1.try_recv() {
            let mut tickets = tickets.into_iter();
            let scans = &mut self.panels.tree.scans.tickets;
            let mut superseded = false;
            listing.retain(|dir| {
                let current = tickets.next().is_some_and(|t| scans.redeem(dir, t));
                superseded |= !current;
                current
            });
            let mut unreadable = None;
            if let Some(ws) = &mut self.editor.workspace {
                let waiting = ws.awaits_reveal();
                changed |= ws.apply_scan(listing);
                revealed |= waiting && !ws.awaits_reveal();
                unreadable = ws.take_open_failure().then(|| ws.name.clone());
            }
            // 刚打开的目录读不出来（没有权限等）：文件树是空的，说明原因。
            if let Some(name) = unreadable {
                let message = winc_i18n::t!("workspace-open-folder-unreadable", name = name);
                self.notify(NoticeKind::Error, message);
            }
            // 这一批要继续做的事，连同之前改为等下一批的；有目录被取代时内容还不是最新的，再等。
            let after = std::mem::take(&mut self.panels.tree.scans.after);
            for (id, late, run) in after {
                if (late || Some(id) == batch) && !superseded {
                    run(self);
                } else {
                    let late = late || Some(id) == batch;
                    self.panels.tree.scans.after.push((id, late, run));
                }
            }
        }
        if changed {
            self.panels.tree.motion = None;
            self.clamp_tree_scroll();
            // 换入的目录中可能又露出尚未读取的目录，随重绘的请求一并读取。
            self.request_redraw();
        }
        if revealed && std::mem::take(&mut self.panels.tree.scans.scroll_to_revealed) {
            self.scroll_to_selected_row();
        }
        self.poll_tree_fetches();
    }

    /// 在后台读取文件树中需要显示但尚未读取的目录（界面线程不等待远端的目录），读好后由
    /// [`App::poll_tree_scans`] 换入。
    pub(in crate::app) fn fetch_tree(&mut self) {
        let Some(scan) = (self.editor.workspace.as_mut()).and_then(|ws| ws.take_missing()) else {
            return;
        };
        self.spawn_scan(scan, None);
    }

    /// 收取后台检查、重读的结果（见 [`Rereads`]）。
    ///
    /// 只采用各文档最近一次发出的读取。读元数据后得知需要读取文件内容时再交给后台；读到的内容
    /// 换入文档，期间文档被编辑过（或者换了路径）就不换，改为再检查一次磁盘，此时有未保存修改的
    /// 文档会得出冲突。
    pub(in crate::app) fn poll_rereads(&mut self) {
        let mut report = DiskReport::default();
        let mut recheck = Vec::new();
        let mut rereads = Vec::new();
        let mut reencoded = Vec::new();
        let mut taken = Vec::new();
        let active = self.editor.active().map(|d| d.id);

        while let Ok((id, ticket, back)) = self.disk.rereads.results.1.try_recv() {
            let tickets = match back {
                DiskBack::Reencoded(_) | DiskBack::Taken(_) => &mut self.disk.rereads.reencodes,
                _ => &mut self.disk.rereads.tickets,
            };
            if !tickets.redeem(&id, ticket) || self.disk.saves.running.contains(&id) {
                continue;
            }
            let Some(doc) = self.editor.document_mut(id) else {
                continue;
            };
            let read = match back {
                DiskBack::Checked(metadata) => {
                    match doc.check_disk_with(metadata.as_ref()) {
                        DiskSync::Reread(reread) => rereads.push((id, reread)),
                        DiskSync::Done(DiskChange::Reloaded) => {
                            report.reloaded.push(doc.name().into_owned());
                        }
                        DiskSync::Done(DiskChange::Conflict) => {
                            report.marked = true;
                            if active != Some(id) {
                                report.conflicts.push(doc.name().into_owned());
                            }
                        }
                        DiskSync::Done(_) => {}
                    }
                    continue;
                }
                DiskBack::Read(read) => read,
                DiskBack::Reencoded(read) => {
                    reencoded.push((id, read));
                    continue;
                }
                DiskBack::Taken(read) => {
                    taken.push((id, read));
                    continue;
                }
            };
            if !read.applies_to(doc) {
                recheck.extend(doc.path.clone());
                continue;
            }
            // 光标在最后一行且没有选区时，文件末尾追加内容后光标跟随到新的末尾（跟随日志）；
            // 光标在其他位置时不移动。
            let at_end = doc.selection.is_empty()
                && doc.text().line_of(doc.selection.head) + 1 == doc.text().line_count();
            match doc.apply_reread(read) {
                Ok(DiskChange::Reloaded) => report.reloaded.push(doc.name().into_owned()),
                Ok(DiskChange::Appended) => {
                    report.changed = true;
                    if at_end {
                        doc.selection = Selection::point(doc.text().end_pos());
                        report.follow |= active == Some(id);
                    }
                }
                _ => {}
            }
        }

        for (id, reread) in rereads {
            let ticket = self.disk.rereads.tickets.issue(id);
            let sender = self.disk.rereads.results.0.clone();
            self.spawn_then(UserEvent::Reread, move || {
                let _ = sender.send((id, ticket, DiskBack::Read(reread.read())));
            });
        }
        for (id, read) in reencoded {
            self.finish_reopen_with_encoding(id, read);
        }
        for (id, read) in taken {
            self.finish_take_disk_version(id, read);
        }
        self.report_disk(report);
        if !recheck.is_empty() {
            self.sync_documents(|path| recheck.iter().any(|p| p == path));
        }
    }

    /// 按 `report` 重画、跟随到末尾，并提示重新加载或冲突的文档。
    fn report_disk(&mut self, report: DiskReport) {
        if !report.reloaded.is_empty() || report.marked || report.changed || report.follow {
            self.request_redraw();
            self.caches.sticky_cache = None;
        }
        if report.follow {
            self.relayout();
            self.ensure_caret_visible();
        }
        if !report.conflicts.is_empty() {
            let names: Vec<String> = report
                .conflicts
                .iter()
                .map(|n| winc_i18n::t!("workspace-quoted-name", name = n.as_str()))
                .collect();
            let names = names.join(winc_i18n::t!("workspace-list-separator"));
            self.notify(
                NoticeKind::Warning,
                winc_i18n::t!("workspace-disk-conflicts", names = names),
            );
        } else if !report.reloaded.is_empty() {
            let names = report
                .reloaded
                .join(winc_i18n::t!("workspace-list-separator"));
            self.flash(winc_i18n::t!("workspace-reloaded", names = names));
        }
    }
}

/// 一批检查、重读结果中需要告知用户的内容。
#[derive(Default)]
struct DiskReport {
    /// 重新加载了的文档名。
    reloaded: Vec<String>,
    /// 标记为冲突的非当前文档名。
    conflicts: Vec<String>,
    /// 有文档标记为冲突（包括当前文档）。
    marked: bool,
    /// 有文档末尾接上了内容。
    changed: bool,
    /// 当前文档的光标跟随到了新的末尾。
    follow: bool,
}
