//! 应用内更新：运行时加载应用包里的 Sparkle，检查更新交给它，由它显示更新窗口、下载和替换应用。
//!
//! 主程序不链接 Sparkle：从 `Contents/Frameworks/Sparkle.framework` 加载；没有框架（例如
//! `cargo run`）、`Info.plist` 里没有更新通道（`WincUpdateFeeds`）时不启用。检查由应用自己安排
//! （启动后 [`FIRST_CHECK`] 秒、之后每 [`CHECK_INTERVAL`] 秒）；后台检查找到新版本时不显示窗口，
//! 由 `found` 在状态栏上提示。

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};

use super::files::{ns_string, rust_string};
use crate::winc::UpdateState;

/// 启动后第一次检查前等待的时间（秒）：留出打开窗口、恢复标签的时间。
const FIRST_CHECK: f64 = 5.0;
/// 两次检查之间的间隔（秒）：一天。
///
/// 由计时器按该间隔重复触发（`NSTimer` 的 `scheduledTimerWithTimeInterval:`）。
const CHECK_INTERVAL: f64 = 86_400.0;
/// 请求一个通道的 appcast 的等待上限（秒）；超时的通道按没有回应处理。
const FEED_TIMEOUT: f64 = 15.0;

/// 启用后的更新器。
struct Updater {
    /// `SPUStandardUpdaterController`。
    controller: Retained<AnyObject>,
    /// 既是更新器又是用户界面的委托；Sparkle 只是弱引用它，因此由本结构持有。
    delegate: Retained<AnyObject>,
    /// 各通道的 appcast 地址，顺序同 `Info.plist`。
    feeds: Vec<String>,
    /// 上一次比出来的通道；还没比过时用第一个。
    feed: Option<String>,
    /// 正在同时请求各通道。
    racing: bool,
    /// 比完以后由用户发起检查（显示更新窗口）；否则在后台检查。
    user: bool,
}

/// 应用交给更新器的回调。
struct Hooks {
    /// 后台检查找到了新版本（版本号），或者提示可以收起了（`None`）。
    found: Box<dyn Fn(Option<String>)>,
    /// 更新的状态变了（开始检查、检查的结果）。
    state: Box<dyn Fn(UpdateState)>,
    /// 到了定时检查的时候：由应用按设置决定要不要调用 [`check`]。
    tick: Box<dyn Fn()>,
}

thread_local! {
    static UPDATER: RefCell<Option<Updater>> = const { RefCell::new(None) };
    static HOOKS: RefCell<Option<Hooks>> = const { RefCell::new(None) };
}

/// 启用应用内更新，返回是否启用了；已经启用过时什么也不做，返回 `true`。
///
/// 只能在主线程、事件循环开始以后调用。
pub fn start(
    found: impl Fn(Option<String>) + 'static,
    state: impl Fn(UpdateState) + 'static,
    tick: impl Fn() + 'static,
) -> bool {
    if available() {
        return true;
    }
    let Some(feeds) = feeds().filter(|f| !f.is_empty()) else {
        return false;
    };
    if !load_framework() {
        return false;
    }
    let Some(class) = delegate_class() else {
        return false;
    };
    let Some(controller_class) = AnyClass::get(c"SPUStandardUpdaterController") else {
        return false;
    };
    // SAFETY: 都是公开 API，在主线程调用。`new` 返回 +1 的委托，交给 `Retained`；控制器
    // `alloc` + `initWithStartingUpdater:…` 返回 +1 的对象，同样交给 `Retained`。控制器只弱引用
    // 委托，委托由 `Updater` 持有到进程结束。`performSelector:…afterDelay:` 和
    // `scheduledTimerWithTimeInterval:…` 由运行循环持有委托，计时器重复触发、不需要另外保存。
    let (controller, delegate) = unsafe {
        let delegate: *mut AnyObject = msg_send![class, new];
        let Some(delegate) = Retained::from_raw(delegate) else {
            return false;
        };
        let controller: *mut AnyObject = msg_send![controller_class, alloc];
        let controller: *mut AnyObject = msg_send![
            controller,
            initWithStartingUpdater: Bool::YES,
            updaterDelegate: &*delegate,
            userDriverDelegate: &*delegate
        ];
        let Some(controller) = Retained::from_raw(controller) else {
            return false;
        };
        let nil: *mut AnyObject = std::ptr::null_mut();
        let () = msg_send![
            &*delegate,
            performSelector: sel!(wincTick:),
            withObject: nil,
            afterDelay: FIRST_CHECK
        ];
        let _: *mut AnyObject = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: CHECK_INTERVAL,
            target: &*delegate,
            selector: sel!(wincTick:),
            userInfo: nil,
            repeats: Bool::YES
        ];
        (controller, delegate)
    };
    add_menu_item(&delegate);
    HOOKS.set(Some(Hooks {
        found: Box::new(found),
        state: Box::new(state),
        tick: Box::new(tick),
    }));
    UPDATER.set(Some(Updater {
        controller,
        delegate,
        feeds,
        feed: None,
        racing: false,
        user: false,
    }));
    report(UpdateState::Idle);
    true
}

/// 应用内更新是否启用了（菜单里的「检查更新」据此显示）。
pub fn available() -> bool {
    UPDATER.with_borrow(Option::is_some)
}

/// 检查更新：先同时请求各通道的 appcast，比出来以后交给 Sparkle。
///
/// `user` 为真时是用户要求的（菜单、状态栏的提示）：Sparkle 显示更新窗口，没有新版本、出错也
/// 告诉用户；否则在后台检查，找到新版本时由 `found` 提示。正在比的时候再要求，比完一并处理。
pub fn check(user: bool) {
    let start = UPDATER.with_borrow_mut(|updater| {
        let updater = updater.as_mut()?;
        updater.user |= user;
        if updater.racing {
            return None;
        }
        updater.racing = true;
        let delegate = Retained::as_ptr(&updater.delegate) as usize;
        Some((updater.feeds.clone(), delegate))
    });
    if let Some((feeds, delegate)) = start {
        report(UpdateState::Checking);
        race(feeds, delegate);
    }
}

/// 并发请求 `feeds`，第一个成功返回 appcast 的通道送回主线程（`wincRaced:`）；都失败时送回
/// `nil`。
///
/// `delegate` 是委托对象的地址：委托活到进程结束，后台线程上的回调只拿它发消息回主线程。
fn race(feeds: Vec<String>, delegate: usize) {
    /// 还没返回的请求数，以及是否已经有通道胜出。
    struct Race {
        left: usize,
        won: bool,
    }
    let state = Arc::new(Mutex::new(Race {
        left: feeds.len(),
        won: false,
    }));
    // SAFETY: `NSURLSession`、`NSURL`、`NSURLRequest` 是公开 API，可在任意线程使用。请求和任务
    // 由会话持有到完成。完成回调在会话的后台队列上执行：只读 `data`、`response`、`error`（回调
    // 期间有效），再用 `performSelectorOnMainThread:` 把结果交给主线程；送回的 NSString 由该方法
    // 持有到送达。
    unsafe {
        let session: *mut AnyObject = msg_send![class!(NSURLSession), sharedSession];
        let Some(session) = session.as_ref() else {
            return finish(delegate, None);
        };
        for feed in feeds {
            let url = ns_string(&feed)
                .map(|s| -> *mut AnyObject { msg_send![class!(NSURL), URLWithString: &*s] })
                .filter(|url| !url.is_null());
            let Some(url) = url else {
                settle(&state, delegate, None);
                continue;
            };
            // NSURLRequestReloadIgnoringLocalCacheData = 1：每次都取最新的 appcast。
            let request: *mut AnyObject = msg_send![
                class!(NSURLRequest),
                requestWithURL: url,
                cachePolicy: 1usize,
                timeoutInterval: FEED_TIMEOUT
            ];
            let state = Arc::clone(&state);
            let handler = RcBlock::new(
                move |data: *mut AnyObject, response: *mut AnyObject, error: *mut AnyObject| {
                    let ok = error.is_null() && is_appcast(data, response);
                    settle(&state, delegate, ok.then_some(feed.as_str()));
                },
            );
            let task: *mut AnyObject =
                msg_send![session, dataTaskWithRequest: request, completionHandler: &*handler];
            if let Some(task) = task.as_ref() {
                let () = msg_send![task, resume];
            }
        }
    }

    /// 记下一个请求的结果；第一个成功的、或者全部失败时，把结果送回主线程。
    fn settle(state: &Mutex<Race>, delegate: usize, feed: Option<&str>) {
        let Ok(mut race) = state.lock() else {
            return;
        };
        race.left = race.left.saturating_sub(1);
        if race.won {
            return;
        }
        if feed.is_some() {
            race.won = true;
            drop(race);
            finish(delegate, feed);
        } else if race.left == 0 {
            race.won = true;
            drop(race);
            finish(delegate, None);
        }
    }

    /// 把比出来的通道（或 `nil`）送回主线程上的委托。
    fn finish(delegate: usize, feed: Option<&str>) {
        let feed = feed.and_then(ns_string);
        let feed: *const AnyObject = feed.as_deref().map_or(std::ptr::null(), |f| f as *const _);
        // SAFETY: `delegate` 是委托对象的地址，委托活到进程结束；`performSelectorOnMainThread:`
        // 可在任意线程调用，会持有 `feed` 直到在主线程上送达。
        unsafe {
            let delegate = delegate as *mut AnyObject;
            let () = msg_send![
                delegate,
                performSelectorOnMainThread: sel!(wincRaced:),
                withObject: feed,
                waitUntilDone: Bool::NO
            ];
        }
    }
}

/// 一个通道的回应是否是完整的 appcast。
///
/// HTTP 状态为 200（不是 HTTP 时不看状态），内容里有 `<rss`；签名由 Sparkle 校验。
///
/// # Safety
///
/// `data`、`response` 为空或指向回调期间有效的 `NSData`、`NSURLResponse`。
unsafe fn is_appcast(data: *mut AnyObject, response: *mut AnyObject) -> bool {
    // SAFETY: 由调用方保证两个指针为空或有效；`bytes` 指向 `length` 个字节，在 `data` 释放前有效。
    unsafe {
        if let Some(response) = response.as_ref() {
            let http: Bool = msg_send![response, respondsToSelector: sel!(statusCode)];
            if http.as_bool() {
                let status: isize = msg_send![response, statusCode];
                if status != 200 {
                    return false;
                }
            }
        }
        let Some(data) = data.as_ref() else {
            return false;
        };
        let len: usize = msg_send![data, length];
        let bytes: *const u8 = msg_send![data, bytes];
        if bytes.is_null() {
            return false;
        }
        let bytes = std::slice::from_raw_parts(bytes, len);
        bytes.windows(4).any(|w| w == b"<rss")
    }
}

/// 读 `Info.plist` 里的更新通道（`WincUpdateFeeds`）；没有这一项时返回 `None`。
fn feeds() -> Option<Vec<String>> {
    let key = ns_string("WincUpdateFeeds")?;
    // SAFETY: `mainBundle`、`objectForInfoDictionaryKey:`、`count`、`objectAtIndex:` 是公开 API；
    // 取出的数组和字符串由主程序包持有，调用期间有效。先确认是 NSArray、元素是 NSString。
    unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let bundle = bundle.as_ref()?;
        let array: *mut AnyObject = msg_send![bundle, objectForInfoDictionaryKey: &*key];
        let array = array.as_ref()?;
        let is_array: Bool = msg_send![array, isKindOfClass: class!(NSArray)];
        if !is_array.as_bool() {
            return None;
        }
        let count: usize = msg_send![array, count];
        let mut feeds = Vec::with_capacity(count);
        for i in 0..count {
            let item: *mut AnyObject = msg_send![array, objectAtIndex: i];
            let Some(item) = item.as_ref() else {
                continue;
            };
            let is_string: Bool = msg_send![item, isKindOfClass: class!(NSString)];
            if is_string.as_bool()
                && let Some(feed) = rust_string(item)
            {
                feeds.push(feed);
            }
        }
        Some(feeds)
    }
}

/// 加载应用包里的 `Sparkle.framework`，返回是否加载了。
fn load_framework() -> bool {
    // SAFETY: `NSBundle` 的公开 API，在主线程调用；取出的路径字符串由主程序包持有，拷贝后不再
    // 使用。`load` 加载框架里的类，重复调用时直接返回。
    unsafe {
        let main: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let Some(main) = main.as_ref() else {
            return false;
        };
        let frameworks: *mut AnyObject = msg_send![main, privateFrameworksPath];
        let Some(frameworks) = frameworks.as_ref().and_then(|p| rust_string(p)) else {
            return false;
        };
        let Some(path) = ns_string(&format!("{frameworks}/Sparkle.framework")) else {
            return false;
        };
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), bundleWithPath: &*path];
        let Some(bundle) = bundle.as_ref() else {
            return false;
        };
        let loaded: Bool = msg_send![bundle, load];
        loaded.as_bool()
    }
}

/// 委托类 `WincUpdateDelegate`：给 Sparkle 选通道、接管后台检查找到新版本时的提示，以及
/// 本模块自己的定时检查（`wincTick:`）和比通道的结果（`wincRaced:`）。
///
/// Sparkle 的协议在框架加载以后才有，因此加载以后再注册；注册失败时返回 `None`。
fn delegate_class() -> Option<&'static AnyClass> {
    if let Some(class) = AnyClass::get(c"WincUpdateDelegate") {
        return Some(class);
    }
    let mut builder = ClassBuilder::new(c"WincUpdateDelegate", class!(NSObject))?;
    for protocol in [c"SPUUpdaterDelegate", c"SPUStandardUserDriverDelegate"] {
        if let Some(protocol) = AnyProtocol::get(protocol) {
            builder.add_protocol(protocol);
        }
    }
    type Feed = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject;
    type Flag = extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool;
    type Scheduled = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool;
    type WillShow = extern "C-unwind" fn(*mut AnyObject, Sel, Bool, *mut AnyObject, *mut AnyObject);
    type One = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
    type Two = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject);
    type None0 = extern "C-unwind" fn(*mut AnyObject, Sel);
    // SAFETY: 每个 `extern "C-unwind"` 函数的参数依次为 self、_cmd 与对应 Objective-C 方法声明的
    // 其余参数，返回类型也一致（例如 `feed_url` 返回 NSString）；方法都在 `builder.register()`
    // 之前添加，Sparkle 按类型编码调用时不会读错参数。
    unsafe {
        builder.add_method(sel!(feedURLStringForUpdater:), feed_url as Feed);
        builder.add_method(
            sel!(supportsGentleScheduledUpdateReminders),
            gentle_reminders as Flag,
        );
        builder.add_method(
            sel!(standardUserDriverShouldHandleShowingScheduledUpdate:andInImmediateFocus:),
            show_scheduled as Scheduled,
        );
        builder.add_method(
            sel!(standardUserDriverWillHandleShowingUpdate:forUpdate:state:),
            will_show as WillShow,
        );
        builder.add_method(
            sel!(standardUserDriverDidReceiveUserAttentionForUpdate:),
            attended as One,
        );
        builder.add_method(
            sel!(standardUserDriverWillFinishUpdateSession),
            finished as None0,
        );
        builder.add_method(sel!(updater:didFindValidUpdate:), found_update as Two);
        builder.add_method(sel!(updaterDidNotFindUpdate:), no_update as One);
        builder.add_method(sel!(updater:didAbortWithError:), aborted as Two);
        builder.add_method(sel!(wincTick:), tick as One);
        builder.add_method(sel!(wincCheck:), check_from_menu as One);
        builder.add_method(sel!(wincRaced:), raced as One);
    }
    Some(builder.register())
}

/// `- (NSString *)feedURLStringForUpdater:`：比出来的通道，还没比过时是第一个。
extern "C-unwind" fn feed_url(
    _this: *mut AnyObject,
    _cmd: Sel,
    _: *mut AnyObject,
) -> *mut AnyObject {
    let feed = UPDATER.with_borrow(|updater| {
        let updater = updater.as_ref()?;
        updater
            .feed
            .clone()
            .or_else(|| updater.feeds.first().cloned())
    });
    feed.and_then(|f| ns_string(&f))
        .map_or(std::ptr::null_mut(), Retained::autorelease_return)
}

/// `supportsGentleScheduledUpdateReminders`：后台检查找到的新版本由应用自己提示。
extern "C-unwind" fn gentle_reminders(_this: *mut AnyObject, _cmd: Sel) -> Bool {
    Bool::YES
}

/// 后台检查找到新版本时 Sparkle 不显示更新窗口，由应用提示。
extern "C-unwind" fn show_scheduled(
    _this: *mut AnyObject,
    _cmd: Sel,
    _update: *mut AnyObject,
    _immediate: Bool,
) -> Bool {
    Bool::NO
}

/// Sparkle 即将显示一个更新：由应用处理的（后台检查找到的）不显示窗口，只在状态栏上提示版本号。
extern "C-unwind" fn will_show(
    _this: *mut AnyObject,
    _cmd: Sel,
    handled: Bool,
    update: *mut AnyObject,
    _state: *mut AnyObject,
) {
    if handled.as_bool() {
        return;
    }
    notify(Some(version_of(update).unwrap_or_default()));
}

/// 更新项 `item`（`SUAppcastItem`）给用户看的版本号。
fn version_of(item: *mut AnyObject) -> Option<String> {
    // SAFETY: `item` 为空或是回调期间有效的 `SUAppcastItem`，`displayVersionString` 是它的公开
    // 属性（NSString），先判空再拷贝。
    unsafe {
        item.as_ref().and_then(|item| {
            let version: *mut AnyObject = msg_send![item, displayVersionString];
            version.as_ref().and_then(|v| rust_string(v))
        })
    }
}

/// 用户已经看到了更新窗口：收起状态栏上的提示。
extern "C-unwind" fn attended(_this: *mut AnyObject, _cmd: Sel, _update: *mut AnyObject) {
    notify(None);
}

/// 这一轮更新结束（装了、跳过了或关掉了）：收起状态栏上的提示。
extern "C-unwind" fn finished(_this: *mut AnyObject, _cmd: Sel) {
    notify(None);
}

/// 找到了可以装的新版本。
extern "C-unwind" fn found_update(
    _this: *mut AnyObject,
    _cmd: Sel,
    _updater: *mut AnyObject,
    item: *mut AnyObject,
) {
    report(UpdateState::Found(version_of(item).unwrap_or_default()));
}

/// 已是最新版本。
extern "C-unwind" fn no_update(_this: *mut AnyObject, _cmd: Sel, _updater: *mut AnyObject) {
    report(UpdateState::Latest);
}

/// 这一轮检查或更新中止了。
///
/// 没有新版本（`SUNoUpdateError`，1001）也会走到这里，状态已由
/// [`no_update`] 记下；用户取消安装（`SUInstallationCanceledError`，4007）不算失败。
extern "C-unwind" fn aborted(
    _this: *mut AnyObject,
    _cmd: Sel,
    _updater: *mut AnyObject,
    error: *mut AnyObject,
) {
    // SAFETY: `error` 为空或是回调期间有效的 NSError，`code` 是它的公开属性。
    let code = unsafe { error.as_ref().map(|e| -> isize { msg_send![e, code] }) };
    if !matches!(code, Some(1001 | 4007)) {
        report(UpdateState::Failed(None));
    }
}

/// 菜单项「检查更新」：用户要求的检查。
extern "C-unwind" fn check_from_menu(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
    check(true);
}

/// 在应用菜单「关于」下面加「检查更新」，动作交给委托（`wincCheck:`）。
fn add_menu_item(delegate: &AnyObject) {
    let Some(title) = ns_string(winc_i18n::t!("menu-check-updates")) else {
        return;
    };
    let Some(key) = ns_string("") else {
        return;
    };
    // SAFETY: `NSApplication`、`NSMenu`、`NSMenuItem` 的公开 API，在主线程调用；指针先判空再用。
    // 菜单项 `alloc` 后 +1，插入菜单后由菜单持有，`release` 抵消；委托活到进程结束。
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let Some(app) = app.as_ref() else {
            return;
        };
        let main_menu: *mut AnyObject = msg_send![app, mainMenu];
        let Some(main_menu) = main_menu.as_ref() else {
            return;
        };
        let app_item: *mut AnyObject = msg_send![main_menu, itemAtIndex: 0isize];
        let Some(app_item) = app_item.as_ref() else {
            return;
        };
        let app_menu: *mut AnyObject = msg_send![app_item, submenu];
        let Some(app_menu) = app_menu.as_ref() else {
            return;
        };
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let item: *mut AnyObject = msg_send![
            item,
            initWithTitle: &*title,
            action: sel!(wincCheck:),
            keyEquivalent: &*key
        ];
        let Some(item) = item.as_ref() else {
            return;
        };
        let () = msg_send![item, setTarget: delegate];
        // 「关于」是第 0 项，「检查更新」紧跟在它后面。
        let count: isize = msg_send![app_menu, numberOfItems];
        let () = msg_send![app_menu, insertItem: item, atIndex: count.min(1)];
        let () = msg_send![item, release];
    }
}

/// 到了定时检查的时候。
extern "C-unwind" fn tick(_this: *mut AnyObject, _cmd: Sel, _timer: *mut AnyObject) {
    HOOKS.with_borrow(|hooks| {
        if let Some(hooks) = hooks {
            (hooks.tick)();
        }
    });
}

/// 比通道的结果送回主线程：记下胜出的通道，再交给 Sparkle 检查。
///
/// 都失败时后台检查就此作罢；用户要求的仍交给 Sparkle，由它告诉用户检查失败。
extern "C-unwind" fn raced(_this: *mut AnyObject, _cmd: Sel, feed: *mut AnyObject) {
    // SAFETY: `feed` 为空或是送达期间有效的 NSString。
    let feed = unsafe { feed.as_ref().and_then(|f| rust_string(f)) };
    let next = UPDATER.with_borrow_mut(|updater| {
        let updater = updater.as_mut()?;
        updater.racing = false;
        if feed.is_some() {
            updater.feed.clone_from(&feed);
        }
        let user = std::mem::take(&mut updater.user);
        Some((updater.controller.clone(), user))
    });
    let Some((controller, user)) = next else {
        return;
    };
    // 后台检查时各通道都没回应：就此作罢，记为失败。
    if !user && feed.is_none() {
        report(UpdateState::Failed(None));
    }
    // 不在借用 `UPDATER` 时调用 Sparkle：它会同步回调 `feedURLStringForUpdater:`。
    // SAFETY: `controller` 是有效的 `SPUStandardUpdaterController`，在主线程调用它的公开方法。
    unsafe {
        if user {
            let nil: *mut AnyObject = std::ptr::null_mut();
            let () = msg_send![&*controller, checkForUpdates: nil];
        } else if feed.is_some() {
            let updater: *mut AnyObject = msg_send![&*controller, updater];
            if let Some(updater) = updater.as_ref() {
                let () = msg_send![updater, checkForUpdatesInBackground];
            }
        }
    }
}

/// 把更新的状态交给应用。
fn report(state: UpdateState) {
    HOOKS.with_borrow(|hooks| {
        if let Some(hooks) = hooks {
            (hooks.state)(state);
        }
    });
}

/// 把找到的版本（或收起提示）交给应用。
fn notify(version: Option<String>) {
    HOOKS.with_borrow(|hooks| {
        if let Some(hooks) = hooks {
            (hooks.found)(version);
        }
    });
}
