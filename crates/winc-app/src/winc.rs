//! 应用这一层：进程里的所有窗口。
//!
//! 每个窗口一个 [`App`]（打开的目录、文档、界面状态等），本层实现 `winit` 的
//! [`ApplicationHandler`]，按窗口把事件分下去：窗口事件按 [`WindowId`]，后台任务和系统回调
//! 送回的按发出它的窗口（[`WindowKey`]），属于整个应用的事件交给当前窗口。每处理完一个事件，
//! 本层统一办理跨窗口的事情：新建窗口、切到前台（[`Request`]）、同步设置、去掉关掉的窗口。

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopClosed, EventLoopProxy};
use winit::window::WindowId;

use winc_config::State;
use winc_editor::Editor;

use crate::app::{App, UserEvent};
use crate::bench::Bench;
use crate::cli::Target;
use crate::instance::{AppLock, Listener};

/// 窗口在本进程里的编号：建 [`App`] 时就定下（那时系统窗口还没建，没有 [`WindowId`]），
/// 后台任务送回的事件靠它找回自己的窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowKey(u32);

/// 事件交给谁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipient {
    /// 发出它的那个窗口。
    Window(WindowKey),
    /// 当前窗口：菜单栏、⌘Q、程序坞这些属于整个应用的。
    Active,
}

/// 送回事件循环的事件和它的收件窗口。
#[derive(Debug)]
pub struct Envelope {
    /// 交给哪个窗口。
    pub to: Recipient,
    /// 事件本身。
    pub event: UserEvent,
}

/// 一个窗口用的事件循环代理：发出的事件都带着收件窗口，用法和 `EventLoopProxy` 一样。
#[derive(Debug, Clone)]
pub struct WindowProxy {
    to: Recipient,
    inner: EventLoopProxy<Envelope>,
}

impl WindowProxy {
    /// 把 `event` 送回事件循环，交给这个代理的收件窗口。
    ///
    /// 事件循环已经结束时原样还回来。
    ///
    /// # Errors
    ///
    /// 事件循环已经结束时返回 [`EventLoopClosed`]，里面是没送出去的事件。
    pub fn send_event(&self, event: UserEvent) -> Result<(), EventLoopClosed<UserEvent>> {
        let envelope = Envelope { to: self.to, event };
        self.inner
            .send_event(envelope)
            .map_err(|EventLoopClosed(envelope)| EventLoopClosed(envelope.event))
    }

    /// 同一个事件循环、交给当前窗口的代理：菜单栏、⌘Q、程序坞的回调用它。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn active(&self) -> WindowProxy {
        WindowProxy {
            to: Recipient::Active,
            inner: self.inner.clone(),
        }
    }
}

/// 窗口要本层代做的事：新建窗口、切换到其他窗口等窗口自身无法完成的操作，先记录下来，
/// 由 [`Winc`] 在处理完当前事件后执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// 新建一个窗口；带目录时在里面打开它，目录已经开在某个窗口里时切到那个窗口。
    NewWindow(Option<PathBuf>),
    /// 把这个窗口切到前台（它开着要打开的目录）。
    Focus(WindowKey),
    /// 文件或目录从 `.0` 移到了 `.1`（在一个窗口剪切、到另一个窗口粘贴）：各窗口里打开的
    /// 文件、最近的文件、文件树跟着更新。
    Moved(PathBuf, PathBuf),
}

/// 应用内更新的状态，设置的「关于」页显示。
///
/// macOS 上由 Sparkle 更新（`macos/updates.rs`），其他平台以 AppImage 运行或由安装包安装时由
/// 应用自己更新（`updates.rs`），此外总是 [`UpdateState::Off`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateState {
    /// 当前版本不检查更新：没有配置更新通道、应用包中没有 Sparkle（例如 `cargo run`）、不是以
    /// AppImage 运行（由包管理器更新）、不是由安装包安装，或者平台不支持。
    #[default]
    Off,
    /// 可以检查，这次运行还没有检查过。
    Idle,
    /// 正在检查。
    Checking,
    /// 上次检查时已是最新版本。
    Latest,
    /// 上次检查找到了新版本（版本号）。
    Found(String),
    /// 上次检查失败（各通道都连不上、appcast 签名不对……）。
    Failed,
    /// 正在下载找到的新版本（版本号，已下载的百分比 0 到 100）；macOS 上这一段交给 Sparkle，由它
    /// 显示进度。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Downloading(String, u8),
    /// 新版本（版本号）已经装好（Windows 上为已下载，退出时安装），重新启动后生效。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Ready(String),
    /// 下载或安装失败（原因）；状态栏上仍提示有新版本，点击后重新下载。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    InstallFailed(String),
}

/// 各窗口共用的一点状态：窗口通过它知道别的窗口开着什么、请本层做事、共用剪切的文件。
///
/// 只在界面线程上用。配置、主题、最近列表不在这里：每个窗口各存一个副本，由 [`Winc`]
/// 同步（见 [`Winc::sync_config`]）。
#[derive(Debug, Default)]
pub struct Shared {
    /// 各窗口打开的目录；每处理完一个事件刷新一次。
    roots: RefCell<Vec<(WindowKey, PathBuf)>>,
    /// 窗口的个数（含关着、等着从程序坞叫回来的窗口）。
    count: Cell<usize>,
    requests: RefCell<Vec<Request>>,
    /// 文件树里剪切的各项：剪贴板里还是这些文件时，粘贴到文件树是把它们移过去（之后清掉）。
    ///
    /// 在一个窗口里剪切、到另一个窗口里粘贴也是移动，所以各窗口共用。
    pub cut_files: RefCell<Vec<PathBuf>>,
    /// 退出前是否需要询问用户：有窗口存在未保存的文档，或者终端中有程序在运行。
    ///
    /// `AppKit` 的 `⌘Q` 回调读取该值，决定直接退出还是回到事件
    /// 循环询问。
    pub ask_on_quit: Arc<AtomicBool>,
    /// 后台检查更新找到的新版本号；各窗口的状态栏都提示该版本号，点击提示后打开更新窗口。
    pub update: RefCell<Option<String>>,
    /// 应用内更新的状态。
    pub update_state: RefCell<UpdateState>,
    /// 最近生成的窗口背景图层：各窗口共用同一张，没有窗口使用时释放。
    pub wallpaper: RefCell<std::rc::Weak<crate::app::WallpaperLayer>>,
}

impl Shared {
    /// 除 `except` 以外开着目录 `folder`（规范化过的路径）的窗口。
    ///
    /// 窗口记着的是打开时给的路径，比较时也规范化（经过符号链接打开的、macOS 上 `/var` 和
    /// `/private/var`）。
    pub fn window_with(&self, folder: &Path, except: Option<WindowKey>) -> Option<WindowKey> {
        let roots = self.roots.borrow();
        let same = |root: &Path| {
            root == folder || winc_config::files::canonicalize(root).is_ok_and(|r| r == folder)
        };
        let found = roots
            .iter()
            .find(|(key, root)| Some(*key) != except && same(root));
        found.map(|(key, _)| *key)
    }

    /// 开着包含 `path` 的目录的窗口；有几个时（嵌套的目录）取最里面的一个。
    pub fn window_containing(&self, path: &Path) -> Option<WindowKey> {
        let roots = self.roots.borrow();
        let inside = roots.iter().filter(|(_, root)| {
            path.starts_with(root)
                || winc_config::files::canonicalize(root).is_ok_and(|r| path.starts_with(r))
        });
        let deepest = inside.max_by_key(|(_, root)| root.components().count());
        deepest.map(|(key, _)| *key)
    }

    /// 是否还有别的窗口。
    pub fn others(&self) -> bool {
        self.count.get() > 1
    }

    /// 记录请求 `request`，由 [`Winc`] 在处理完当前事件后执行。
    pub fn request(&self, request: Request) {
        self.requests.borrow_mut().push(request);
    }
}

/// 新建窗口用的、整个进程一样的东西：配置和状态文件的位置、备份目录。
///
/// 临时打开（`--wait`、`-`）和测量时没有，不能新建窗口。
#[derive(Debug, Clone)]
pub struct Template {
    /// 配置文件。
    pub config_path: Option<PathBuf>,
    /// 状态文件（最近目录、各目录的现场）。
    pub state_path: Option<PathBuf>,
    /// 未保存内容的备份目录。
    pub backup_root: Option<PathBuf>,
}

/// 一个窗口和本层为它记下的东西。
struct Slot {
    key: WindowKey,
    app: App,
    /// 上次看到的配置版本（[`App::config_rev`]）：变了就是这个窗口改了设置，发给其他窗口。
    config_rev: u64,
}

/// ⌘Q 退出全部窗口的进度：正在问的窗口，和还没问的。
#[derive(Debug, Default)]
struct Quitting {
    asking: Option<Asking>,
    rest: Vec<WindowKey>,
}

/// 正在请其退出的窗口。
#[derive(Debug, Clone, Copy)]
struct Asking {
    key: WindowKey,
    /// 请求该窗口退出时它正在进行另一项询问（例如关闭标签时的保存确认），退出请求被推迟：
    /// 该询问结束后再次请求，不视为用户取消了退出。
    deferred: bool,
}

/// 整个应用：所有窗口，和当前是哪个（最近获得焦点或按下鼠标的）。
pub struct Winc {
    /// 事件循环的代理；测试中没有事件循环，为 `None`。
    proxy: Option<EventLoopProxy<Envelope>>,
    /// 所有窗口，以及本层为它们记下的东西。
    windows: Vec<Slot>,
    /// 最近一个获得焦点或按下鼠标的窗口；属于整个应用的事件交给它。
    active: Option<WindowKey>,
    /// 下一个窗口的编号（[`WindowKey`] 的序号，从 0 起递增）。
    next_key: u32,
    /// 各窗口共用的状态，每个 [`App`] 都持有同一个副本的引用。
    shared: Rc<Shared>,
    /// 新建窗口用的配置和文件位置；不能新建窗口时为 `None`。
    template: Option<Template>,
    /// 正在进行的退出：当前在问的窗口和还没问的窗口；没有在退出时为 `None`。
    quitting: Option<Quitting>,
    /// 在整个应用的套接字上等之后启动的 `winc` 交来的目录和文件（见 `instance`）；另一个
    /// winc 已经在等、或者不能新建窗口时没有。
    launches: Option<(Listener, AppLock)>,
}

impl Winc {
    /// 创建还没有窗口的应用；`proxy` 是事件循环的代理（测试中为 `None`），`template` 是新建
    /// 窗口用的（没有时不能新建窗口）。
    pub fn new(proxy: Option<EventLoopProxy<Envelope>>, template: Option<Template>) -> Winc {
        Winc {
            proxy,
            windows: Vec::new(),
            active: None,
            next_key: 0,
            shared: Rc::default(),
            template,
            quitting: None,
            launches: None,
        }
    }

    /// 开始在整个应用的套接字上等之后启动的 `winc`（见 `instance`）。
    ///
    /// 另一个 winc 已经拿着锁、不能新建窗口（临时打开、测量）或没有事件循环时不等，各自开
    /// 各自的。
    pub fn listen_for_launches(&mut self) {
        let (Some(_), Some(proxy)) = (&self.template, self.proxy.clone()) else {
            return;
        };
        let Some(lock) = AppLock::acquire() else {
            return;
        };
        let listener = Listener::bind_app(move |launch, answer| {
            let event = UserEvent::Launched(launch, answer);
            let _ = proxy.send_event(Envelope {
                to: Recipient::Active,
                event,
            });
        });
        self.launches = listener.map(|listener| (listener, lock));
    }

    /// 启动时把上次退出时其他窗口开着的目录 `folders` 也各开一个窗口（还在的），最后把第一个
    /// 窗口（上次的当前窗口）切回前台。
    ///
    /// 第一个窗口在开始页上时（上次的当前窗口没开目录），第一个目录就开在它里面，不多留一个空
    /// 窗口。新窗口在事件循环开始以后建。
    pub fn restore_windows(&mut self, folders: &[PathBuf]) {
        let Some(slot) = self.windows.first_mut() else {
            return;
        };
        let first = slot.key;
        let root = slot.app.editor.root().map(Path::to_path_buf);
        let mut folders = folders
            .iter()
            .filter(|f| f.is_dir() && Some(*f) != root.as_ref())
            .peekable();
        if root.is_none()
            && let Some(folder) = folders.next()
        {
            slot.app.open_folder_now(folder);
        }
        if folders.peek().is_none() {
            return;
        }
        for folder in folders {
            self.shared
                .request(Request::NewWindow(Some(folder.clone())));
        }
        self.shared.request(Request::Focus(first));
    }

    /// 为下一个窗口取一个编号和发事件用的代理；建好 [`App`] 以后用 [`Winc::add`] 加进来。
    ///
    /// 没有事件循环（测试中）时没有代理。
    pub fn reserve(&mut self) -> (WindowKey, Option<WindowProxy>) {
        let key = WindowKey(self.next_key);
        self.next_key += 1;
        let proxy = self.proxy.as_ref().map(|inner| WindowProxy {
            to: Recipient::Window(key),
            inner: inner.clone(),
        });
        (key, proxy)
    }

    /// 加入编号为 `key` 的窗口，接上共用的状态；第一个窗口同时是当前窗口。
    pub fn add(&mut self, key: WindowKey, mut app: App) {
        app.key = Some(key);
        app.shared = Rc::clone(&self.shared);
        app.ask_on_quit = Arc::clone(&self.shared.ask_on_quit);
        self.active.get_or_insert(key);
        let config_rev = app.config_rev;
        self.windows.push(Slot {
            key,
            app,
            config_rev,
        });
        self.refresh_shared();
    }

    /// 编号为 `key` 的窗口。
    fn by_key(&mut self, key: WindowKey) -> Option<&mut App> {
        let found = self.windows.iter_mut().find(|slot| slot.key == key);
        found.map(|slot| &mut slot.app)
    }

    /// 系统窗口为 `id` 的窗口的编号。
    fn key_of(&self, id: WindowId) -> Option<WindowKey> {
        let found = self
            .windows
            .iter()
            .find(|slot| slot.app.window_id() == Some(id));
        found.map(|slot| slot.key)
    }

    /// 事件 `to` 指的窗口：当前窗口不在了（关掉了）时交给第一个。
    fn recipient(&mut self, to: Recipient) -> Option<&mut App> {
        let key = match to {
            Recipient::Window(key) => key,
            Recipient::Active => self.active_key()?,
        };
        self.by_key(key)
    }

    /// 当前窗口；已经关掉时取第一个窗口。
    fn active_key(&self) -> Option<WindowKey> {
        let alive = |key: &WindowKey| self.windows.iter().any(|slot| slot.key == *key);
        self.active
            .filter(alive)
            .or(self.windows.first().map(|slot| slot.key))
    }

    /// 处理完一个事件后的收尾：同步设置、移除已关闭的窗口、`--wait` 等待结束时开始退出、推进
    /// 退出流程，最后刷新共用的状态。
    ///
    /// 返回窗口要本层做的事，和是否该结束事件循环。
    fn settle(&mut self) -> (Vec<Request>, bool) {
        self.sync_config();
        self.drop_closed();
        for slot in &mut self.windows {
            slot.app.finish_waiting();
        }
        let exit = self.advance_quit();
        self.refresh_shared();
        let mut requests = std::mem::take(&mut *self.shared.requests.borrow_mut());
        // 移动不用事件循环，在这里就告诉各窗口；发起的窗口已经更新过，再做一次没有变化。
        requests.retain(|request| match request {
            Request::Moved(from, to) => {
                for slot in &mut self.windows {
                    slot.app.moved_elsewhere(from, to);
                }
                false
            }
            _ => true,
        });
        (requests, exit)
    }

    /// 某个窗口改了设置（[`App::config_rev`] 变了）：把它的配置发给其他窗口。
    fn sync_config(&mut self) {
        let changed = self
            .windows
            .iter()
            .position(|s| s.app.config_rev != s.config_rev);
        let Some(from) = changed else {
            return;
        };
        let config = self.windows[from].app.config.clone();
        // 一并同步配置文件的内容：该窗口写入失败时，未写入的项在之后任一窗口写回时仍算作修改。
        let saved = self.windows[from].app.saved_config.clone();
        for (i, slot) in self.windows.iter_mut().enumerate() {
            if i != from {
                slot.app.saved_config = saved.clone();
                slot.app.adopt_config(config.clone());
            }
            slot.config_rev = slot.app.config_rev;
        }
    }

    /// 关掉（隐藏）的窗口在还有别的窗口时去掉：macOS 上只有最后一个窗口关掉时留着，点程序坞
    /// 时带着开始页回来。
    ///
    /// 关掉之前目录的现场、未保存内容的备份都已记下。
    fn drop_closed(&mut self) {
        if self.windows.iter().all(|slot| slot.app.window.hidden) {
            return;
        }
        self.windows.retain(|slot| !slot.app.window.hidden);
    }

    /// 推进退出流程：有窗口请求退出（⌘Q，`Closing::quit_requested`）时逐个请求其他窗口退出，
    /// 由各窗口自行完成保存询问；全部窗口均已退出时返回 `true`，此时应结束事件循环。
    ///
    /// 任一窗口在询问期间选择「取消」时终止整个退出流程，已经同意退出的窗口保持打开。窗口正在
    /// 进行其他询问（关闭标签时的保存确认）时，等该询问结束后再请求它退出。
    fn advance_quit(&mut self) -> bool {
        if self.quitting.is_none() {
            if !self
                .windows
                .iter()
                .any(|slot| slot.app.closing.quit_requested)
            {
                return false;
            }
            // 当前窗口最后退：下次启动打开的是它的目录（`State::last`）。从末尾往前问，当前窗口
            // 排在最前。
            let active = self.active_key();
            let mut rest: Vec<WindowKey> = (self.windows.iter())
                .filter(|slot| !slot.app.closing.quit_requested)
                .map(|slot| slot.key)
                .collect();
            rest.sort_by_key(|key| Some(*key) != active);
            self.quitting = Some(Quitting { asking: None, rest });
        }
        loop {
            let Some(quitting) = &mut self.quitting else {
                return false;
            };
            if let Some(asking) = quitting.asking {
                let Some(app) = self.windows.iter().find(|s| s.key == asking.key) else {
                    quitting.asking = None;
                    continue;
                };
                let app = &app.app;
                if app.closing.quit_requested {
                    quitting.asking = None;
                    continue;
                }
                if app.closing_pending() {
                    return false;
                }
                if !asking.deferred {
                    self.cancel_quit();
                    return false;
                }
                // 推迟退出请求的那次询问已经结束：再次请求退出。
                self.ask_to_quit(asking.key);
                continue;
            }
            let Some(key) = quitting.rest.pop() else {
                return true;
            };
            self.ask_to_quit(key);
        }
    }

    /// 请窗口 `key` 退出，记为正在问的窗口。
    ///
    /// 该窗口正在进行其他询问（例如关闭标签时的保存确认）时，退出请求不会打断该询问，
    /// 记为推迟。
    fn ask_to_quit(&mut self, key: WindowKey) {
        let deferred = self.by_key(key).is_some_and(|app| {
            app.request_quit();
            !app.closing.quit_requested && app.closing_pending() && !app.quit_pending()
        });
        if let Some(quitting) = &mut self.quitting {
            quitting.asking = Some(Asking { key, deferred });
        }
    }

    /// 终止退出流程：已经同意退出的窗口保持打开，标签不关闭，备份继续写入。
    fn cancel_quit(&mut self) {
        self.quitting = None;
        // 为完成更新而发起的退出（`updates::restart`）被取消后，之后的退出不再启动新版本。
        #[cfg(not(target_os = "macos"))]
        crate::updates::cancel_restart();
        for slot in &mut self.windows {
            slot.app.closing.quit_requested = false;
        }
    }

    /// 刷新共用的状态里各窗口的目录和窗口个数。
    ///
    /// 每个事件之后都调用（包括鼠标移动），目录没变时不重建列表。
    fn refresh_shared(&mut self) {
        self.shared.count.set(self.windows.len());
        let roots = || {
            self.windows.iter().filter_map(|slot| {
                let root = slot.app.editor.root()?;
                Some((slot.key, root))
            })
        };
        let same = {
            let known = self.shared.roots.borrow();
            known.len() == roots().count()
                && known
                    .iter()
                    .zip(roots())
                    .all(|((k, r), (key, root))| *k == key && r == root)
        };
        if !same {
            let fresh = roots().map(|(key, root)| (key, root.to_path_buf()));
            *self.shared.roots.borrow_mut() = fresh.collect();
        }
    }

    /// 在处理完事件后执行窗口记录的请求；返回 `true` 时结束事件循环。
    ///
    /// 执行请求期间（如在新建窗口中打开目录）产生的请求进入下一轮处理。
    fn after_event(&mut self, event_loop: &ActiveEventLoop) {
        loop {
            let (requests, exit) = self.settle();
            if exit {
                event_loop.exit();
                return;
            }
            if requests.is_empty() {
                return;
            }
            for request in requests {
                match request {
                    Request::NewWindow(folder) => self.new_window(event_loop, folder, None),
                    Request::Focus(key) => {
                        if let Some(app) = self.by_key(key) {
                            app.raise();
                        }
                        self.active = Some(key);
                    }
                    // 已在 `settle` 里处理。
                    Request::Moved(..) => {}
                }
            }
        }
    }

    /// 新启动的 `winc` 把 `targets` 交给了整个应用，按 [`Self::open_targets`] 打开。
    ///
    /// 正在退出时不收，新进程自己打开。
    fn launched(
        &mut self,
        event_loop: &ActiveEventLoop,
        launch: crate::instance::Launch,
        answer: std::sync::mpsc::Sender<bool>,
    ) {
        // 答复发不出去：新进程等不及，已经自己打开了。
        if self.quitting.is_some() || answer.send(true).is_err() {
            let _ = answer.send(false);
            return;
        }
        // 不带目标再次启动：macOS 上切到当前窗口，与点击程序坞图标相同；其他系统上新建窗口。
        if launch.targets.is_empty() && !cfg!(target_os = "macos") {
            self.new_window(event_loop, None, launch.token);
            if let Some(app) = self.active_key().and_then(|key| self.by_key(key)) {
                app.raise();
            }
            return;
        }
        self.open_targets(event_loop, &launch.targets, launch.token);
    }

    /// 系统交来了要打开的 `paths`（拖到程序坞的图标上、文件管理器里的「打开方式」），和之后启动的
    /// `winc` 交来的一样按 [`Self::open_targets`] 打开。
    ///
    /// 正在退出时不理会。
    fn opened(&mut self, event_loop: &ActiveEventLoop, paths: &[PathBuf]) {
        if self.quitting.is_some() {
            return;
        }
        let targets = paths.iter().map(|path| Target {
            path: crate::instance::absolute(path),
            line: None,
            col: None,
        });
        self.open_targets(event_loop, &targets.collect::<Vec<_>>(), None);
    }

    /// 把 `targets` 开在合适的窗口里（窗口见 [`Self::window_for`]），并把它切到前台。
    ///
    /// `token` 是新启动的 winc 交来的激活令牌（见 [`App::set_activation`]）。
    fn open_targets(
        &mut self,
        event_loop: &ActiveEventLoop,
        targets: &[Target],
        mut token: Option<String>,
    ) {
        let key = match self.window_for(targets) {
            Ok(key) => key,
            Err(folder) => {
                self.new_window(event_loop, folder, token.take());
                let Some(key) = self.active_key() else {
                    return;
                };
                key
            }
        };
        self.active = Some(key);
        if let Some(app) = self.by_key(key) {
            app.set_activation(token);
            app.open_files(targets);
        }
    }

    /// 选出打开 `targets` 的窗口：有目录的切到开着它的窗口；只有文件的，交给开着它所在目录的
    /// 窗口，没有时交给当前窗口，不打开文件所在的目录；没有目标时是当前窗口。
    ///
    /// 都没有时，有停在开始页上的空窗口就用它，在里面打开目录；否则返回 `Err(目录)`，由调用方
    /// 新建窗口。
    fn window_for(&mut self, targets: &[Target]) -> Result<WindowKey, Option<PathBuf>> {
        let folder = targets
            .iter()
            .find(|t| t.path.is_dir())
            .map(|t| t.path.clone());
        let file = targets
            .iter()
            .find(|t| !t.path.is_dir())
            .map(|t| t.path.clone());
        let key = match (&folder, &file) {
            (Some(folder), _) => self.shared.window_with(folder, None),
            (None, Some(file)) => (self.shared.window_containing(file)).or(self.active_key()),
            (None, None) => self.active_key(),
        };
        if let Some(key) = key {
            return Ok(key);
        }
        let Some(key) = self.blank_window() else {
            return Err(folder);
        };
        if let (Some(folder), Some(app)) = (&folder, self.by_key(key)) {
            app.open_folder_now(folder);
        }
        Ok(key)
    }

    /// 停在开始页上、什么都没开（没有目录也没有标签）的窗口，当前窗口优先；没有时返回 `None`。
    fn blank_window(&self) -> Option<WindowKey> {
        let active = self.active_key();
        let blank =
            |slot: &&Slot| slot.app.editor.root().is_none() && slot.app.editor.documents.is_empty();
        let blanks = self.windows.iter().filter(blank);
        blanks
            .min_by_key(|slot| Some(slot.key) != active)
            .map(|slot| slot.key)
    }

    /// 新建一个窗口；带 `folder` 时在里面打开它。
    ///
    /// 目录已经开在某个窗口里时切到那个窗口；关着的窗口（macOS 上关掉了最后一个窗口）先拿来用。
    /// 当前窗口处于全屏时，新窗口同样进入全屏。
    /// `token` 同 [`Self::open_targets`]。
    fn new_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        folder: Option<PathBuf>,
        token: Option<String>,
    ) {
        let folder = folder.map(|f| winc_config::files::canonicalize(&f).unwrap_or(f));
        if let Some(key) = folder
            .as_ref()
            .and_then(|f| self.shared.window_with(f, None))
        {
            if let Some(app) = self.by_key(key) {
                app.set_activation(token);
                app.raise();
            }
            return;
        }
        let Some(template) = self.template.clone() else {
            if let Some(app) = self.active_key().and_then(|key| self.by_key(key)) {
                app.flash(winc_i18n::t!("cmd-one-off-no-new-window"));
            }
            return;
        };
        // 当前窗口处于全屏时，新窗口同样进入全屏。
        let fullscreen = (self.windows.iter())
            .find(|slot| Some(slot.key) == self.active_key())
            .is_some_and(|slot| slot.app.is_fullscreen());
        // 关着的窗口在开始页上，拿来用，不再建一个。
        let hidden = self
            .windows
            .iter()
            .find(|s| s.app.window.hidden)
            .map(|s| s.key);
        if let Some(key) = hidden {
            if let Some(app) = self.by_key(key) {
                if let Some(folder) = &folder {
                    app.open_folder_now(folder);
                }
                app.set_activation(token);
                app.raise();
                if fullscreen {
                    app.enter_fullscreen();
                }
            }
            self.active = Some(key);
            self.refresh_shared();
            return;
        }

        // 配置用当前窗口的（和配置文件一样，各窗口之间已同步），最近列表重新读。
        let from = self.active_key().and_then(|key| {
            let slot = self.windows.iter().find(|s| s.key == key)?;
            let app = &slot.app;
            Some((app.config.clone(), app.cascade_from(), app.shared_glyphs()))
        });
        let (config, cascade, glyphs) = from.unwrap_or_default();
        let mut editor = Editor::default();
        editor.tab_width = config.tab_width.unwrap_or(editor.tab_width);
        let (key, proxy) = self.reserve();
        let mut app = App::new(editor, proxy, Bench::off(), config);
        app.config_path = template.config_path;
        app.state = (template.state_path.as_deref())
            .map(State::load_from)
            .unwrap_or_default();
        app.commit.set_flat(!app.state.commit_tree);
        app.restore_panel_width();
        app.editor.diff_unified = app.config.diff_inline == Some(true);
        app.apply_preview_state();
        app.state_path = template.state_path;
        app.backup.root = template.backup_root;
        app.cascade = cascade;
        app.open_fullscreen = fullscreen;
        app.glyphs_from = glyphs;
        self.add(key, app);
        self.active = Some(key);
        let Some(app) = self.by_key(key) else {
            return;
        };
        if let Some(folder) = &folder {
            app.open_folder_now(folder);
        }
        app.set_activation(token);
        app.resumed(event_loop);
        self.refresh_shared();
    }

    /// 退出时记下其他窗口开着的目录（`State::others`），下次启动时各开一个窗口；当前窗口的是
    /// `State::last`，由它自己记。
    ///
    /// 设置里关掉了「启动时打开上次的目录」时不记。
    fn remember_windows(&mut self) {
        let Some(path) = self.template.as_ref().and_then(|t| t.state_path.clone()) else {
            return;
        };
        let active = self.active_key();
        let reopen = (self.windows.iter())
            .find(|slot| Some(slot.key) == active)
            .is_none_or(|slot| slot.app.config.restore_folder != Some(false));
        let others: Vec<PathBuf> = (self.windows.iter())
            .filter(|slot| reopen && Some(slot.key) != active)
            .filter_map(|slot| slot.app.editor.root().map(Path::to_path_buf))
            .collect();
        let (_, result) = State::update(&path, |state| state.others = others);
        if let Err(e) = result {
            eprintln!("winc: 无法记下开着的窗口：{e}");
        }
    }

    /// 各窗口下一次要醒来的时刻里最早的一个。
    fn next_wake(&mut self) -> Option<Instant> {
        let wakes = self.windows.iter_mut().filter_map(|slot| slot.app.tick());
        wakes.min()
    }
}

impl ApplicationHandler<Envelope> for Winc {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        for slot in &mut self.windows {
            slot.app.resumed(event_loop);
        }
        self.after_event(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, envelope: Envelope) {
        match envelope.event {
            UserEvent::Launched(launch, answer) => self.launched(event_loop, launch, answer),
            UserEvent::Opened(paths) => self.opened(event_loop, &paths),
            UserEvent::UpdateFound(version) => {
                *self.shared.update.borrow_mut() = version;
                for slot in &mut self.windows {
                    slot.app.request_redraw();
                }
            }
            UserEvent::UpdateState(state) => {
                *self.shared.update_state.borrow_mut() = state;
                for slot in &mut self.windows {
                    slot.app.request_redraw();
                }
            }
            // 设置是各窗口同步的，看哪个窗口都一样。
            UserEvent::UpdateTick => {
                let on = self
                    .windows
                    .first()
                    .is_some_and(|slot| slot.app.config.update_auto_check != Some(false));
                #[cfg(target_os = "macos")]
                if on {
                    crate::macos::check_for_updates(false);
                }
                #[cfg(not(target_os = "macos"))]
                if on {
                    crate::updates::check();
                }
            }
            event => {
                // 发出事件的窗口已经关掉了：后台任务的结果没有窗口接收，直接丢弃。
                if let Some(app) = self.recipient(envelope.to) {
                    app.user_event(event_loop, event);
                }
            }
        }
        self.after_event(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(key) = self.key_of(id) else {
            return;
        };
        // 右键菜单可以在没有焦点的窗口上弹出，菜单项选中的结果交给当前窗口：按下鼠标的窗口
        // 也算当前窗口，菜单的结果才回到弹出它的那个窗口。
        if matches!(
            event,
            WindowEvent::Focused(true)
                | WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                }
        ) {
            self.active = Some(key);
        }
        if let Some(app) = self.by_key(key) {
            app.window_event(event_loop, id, event);
        }
        self.after_event(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let wake = self.next_wake();
        let ask = self.windows.iter().any(|slot| slot.app.asks_before_quit());
        self.shared.ask_on_quit.store(ask, Ordering::Relaxed);
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        // 当前窗口最后记现场：下次启动打开的是它的目录（`State::last`）。
        let active = self.active_key();
        self.windows.sort_by_key(|slot| Some(slot.key) == active);
        for slot in &mut self.windows {
            slot.app.exiting(event_loop);
        }
        self.remember_windows();
    }
}

#[cfg(test)]
mod tests;
