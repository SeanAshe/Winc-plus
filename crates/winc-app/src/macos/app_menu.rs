//! 挂在 `winit` 应用委托上的扩展：⌘Q 的拦截、应用菜单中的「设置…」、菜单项的动作方法，以及录制
//! 按键期间暂停菜单项的快捷键。
//!
//! 这些方法通过 Objective-C 运行时（`class_addMethod`）添加到 `winit` 的应用委托类上，不替换
//! 已有的实现；找不到委托类时，各功能保持系统或键盘事件的默认行为。

use objc2::Encode;
use objc2::runtime::Bool;

use super::files::ns_string;
use super::*;

/// ⌘Q 时调用的回调，返回是否允许立即退出；只在主线程上设置和调用。
type QuitHandler = Box<dyn Fn() -> bool + Send + Sync>;

static QUIT_HANDLER: OnceLock<QuitHandler> = OnceLock::new();

/// 拦截 ⌘Q（和程序坞中的「退出」）：`allow` 返回 `false` 时取消本次退出。
///
/// 必须在事件循环开始之后（`winit` 已设置应用委托）调用；只有第一次调用生效，之后的调用被忽略。
pub fn intercept_quit(allow: impl Fn() -> bool + Send + Sync + 'static) {
    if QUIT_HANDLER.set(Box::new(allow)).is_err() {
        return;
    }

    /// 实现 `- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender`：
    /// 退出前询问是否允许退出。
    extern "C-unwind" fn should_terminate(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
    ) -> usize {
        // NSTerminateCancel = 0，NSTerminateNow = 1。
        let allow = QUIT_HANDLER.get().is_none_or(|allow| allow());
        usize::from(allow)
    }

    // 委托类在事件循环开始后才注册；找不到时（例如 `winit` 修改了类名），⌘Q 保持系统的默认行为。
    if let Some(delegate) = AnyClass::get(DELEGATE) {
        add_should_terminate(delegate, should_terminate);
    }
}

/// `winit` 的应用委托类名，已对照 `winit` 0.30.13 的源码确认；升级 `winit` 后需要确认类名未变。
pub(super) const DELEGATE: &CStr = c"WinitApplicationDelegate";

type ShouldTerminate = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize;

/// 给 `class` 添加 `applicationShouldTerminate:` 方法，返回是否添加成功；类已有该方法时不做
/// 任何事并返回 `false`。
fn add_should_terminate(class: &AnyClass, imp: ShouldTerminate) -> bool {
    // SAFETY: `ShouldTerminate` 与 `Imp`（`unsafe extern "C-unwind" fn()`）都是函数指针，
    // 两者大小相同，`transmute` 成立。传入的类型编码 `Q@:@` 与实际签名一致：返回 NSUInteger（Q），
    // 参数依次为 self（@）、_cmd（:）和 NSApplication（@）；AppKit 按该编码调用，不会读错参数。
    unsafe {
        let imp = std::mem::transmute::<ShouldTerminate, Imp>(imp);
        add_raw_method(class, sel!(applicationShouldTerminate:), imp, c"Q@:@")
    }
}

/// 没有可见窗口时点击程序坞中的图标调用的回调；只在主线程上设置和调用。
type ReopenHandler = Box<dyn Fn() + Send + Sync>;

static REOPEN_HANDLER: OnceLock<ReopenHandler> = OnceLock::new();

/// 应用没有可见窗口时（窗口已关闭，应用仍留在程序坞中），点击程序坞中的图标或在系统文件
/// 管理器中再次打开 winc，调用 `reopen`。
///
/// 必须在事件循环开始之后（`winit` 已设置应用委托）调用；只有第一次调用生效。
pub fn on_reopen(reopen: impl Fn() + Send + Sync + 'static) {
    if REOPEN_HANDLER.set(Box::new(reopen)).is_err() {
        return;
    }

    /// 实现 `- (BOOL)applicationShouldHandleReopen:(NSApplication *)sender
    /// hasVisibleWindows:(BOOL)flag`：没有可见窗口时点击图标，由系统调用。
    extern "C-unwind" fn should_reopen(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        visible: Bool,
    ) -> Bool {
        if !visible.as_bool()
            && let Some(reopen) = REOPEN_HANDLER.get()
        {
            reopen();
        }
        // 返回 YES：其余情况（例如窗口最小化到程序坞中）交给系统的默认处理。
        Bool::YES
    }

    if let Some(delegate) = AnyClass::get(DELEGATE) {
        add_should_reopen(delegate, should_reopen);
    }
}

type ShouldReopen = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool;

/// 给 `class` 添加 `applicationShouldHandleReopen:hasVisibleWindows:` 方法，返回是否添加成功；
/// 类已有该方法时不做任何事并返回 `false`。
fn add_should_reopen(class: &AnyClass, imp: ShouldReopen) -> bool {
    // BOOL 的类型编码随架构不同（arm64 上是 `B`，x86_64 上是 `c`），按 `Bool::ENCODING` 拼出。
    let types = CString::new(format!("{0}@:@{0}", Bool::ENCODING)).expect("类型编码中没有 NUL");
    // SAFETY: `ShouldReopen` 与 `Imp` 都是函数指针，大小相同，`transmute` 成立。类型编码与
    // 实际签名一致：返回 BOOL，参数依次为 self（@）、_cmd（:）、NSApplication（@）和 BOOL；
    // AppKit 按该编码调用，不会读错参数。
    unsafe {
        let imp = std::mem::transmute::<ShouldReopen, Imp>(imp);
        add_raw_method(
            class,
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            imp,
            &types,
        )
    }
}

/// 系统交来要打开的文件和目录时调用的回调；只在主线程上设置和调用。
type OpenHandler = Box<dyn Fn(Vec<PathBuf>) + Send + Sync>;

static OPEN_HANDLER: OnceLock<OpenHandler> = OnceLock::new();

/// 文件或目录拖到程序坞的图标上、在文件管理器里用「打开方式 → winc」打开时，以它们的路径调用
/// `open`（`application:openURLs:`）；winc 还没运行时，系统先启动它再交来。
///
/// 必须在 `winit` 建好事件循环（已设置应用委托）之后、开始运行之前调用：启动时交来的那一批在
/// `applicationDidFinishLaunching:` 之前就到了。只有第一次调用生效。
pub fn on_open_files(open: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) {
    if OPEN_HANDLER.set(Box::new(open)).is_err() {
        return;
    }

    if let Some(delegate) = AnyClass::get(DELEGATE) {
        add_open_urls(delegate, open_urls);
    }
}

/// 实现 `- (void)application:(NSApplication *)app openURLs:(NSArray<NSURL *> *)urls`：系统交来
/// 要打开的文件和网址。
extern "C-unwind" fn open_urls(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    urls: *mut AnyObject,
) {
    // SAFETY: `urls` 由 AppKit 传入，在整个调用期间有效（所有权归自动释放池），可为空。
    // `count` 与 `objectAtIndex:` 是 NSArray 的公开方法，下标小于 `count`；元素是 NSURL，
    // 由数组持有，在本次调用内有效。数组为空时不取元素；每个元素判空后再转路径，不是文件
    // URL 的（`url_path` 返回 `None`）跳过。
    let paths: Vec<PathBuf> = unsafe {
        let Some(urls) = urls.as_ref() else {
            return;
        };
        let count: usize = msg_send![urls, count];
        (0..count)
            .filter_map(|i| {
                let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                super::files::url_path(url.as_ref()?)
            })
            .collect()
    };
    if let Some(open) = OPEN_HANDLER.get().filter(|_| !paths.is_empty()) {
        open(paths);
    }
}

type OpenUrls = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject);

/// 给 `class` 添加 `application:openURLs:` 方法，返回是否添加成功；类已有该方法时不做任何事
/// 并返回 `false`。
fn add_open_urls(class: &AnyClass, imp: OpenUrls) -> bool {
    // SAFETY: `OpenUrls` 与 `Imp` 都是函数指针，大小相同，`transmute` 成立。类型编码
    // `v@:@@` 与实际签名一致：返回 void（v），参数依次为 self（@）、_cmd（:）、NSApplication（@）
    // 和 NSArray（@）；AppKit 按该编码调用，不会读错参数。
    unsafe {
        let imp = std::mem::transmute::<OpenUrls, Imp>(imp);
        add_raw_method(class, sel!(application:openURLs:), imp, c"v@:@@")
    }
}

/// 菜单栏「设置…」被选中时调用的回调；只在主线程上设置和调用。
type SettingsHandler = Box<dyn Fn() + Send + Sync>;

static SETTINGS_HANDLER: OnceLock<SettingsHandler> = OnceLock::new();

/// 在应用菜单（菜单栏上的「winc」）中添加一项「设置…  ⌘,」，放在「关于」下面，选中时调用 `open`。
///
/// 必须在事件循环开始之后（`winit` 已建好菜单和应用委托）调用；只有第一次调用生效。菜单项绑定
/// `⌘,` 后，该按键先被菜单处理，不再作为键盘事件送到窗口，因此只由本函数注册的回调处理。
pub fn add_settings_menu(open: impl Fn() + Send + Sync + 'static) {
    if SETTINGS_HANDLER.set(Box::new(open)).is_err() {
        return;
    }

    /// 实现 `- (void)wincOpenSettings:(id)sender`：菜单项「设置…」的动作。
    extern "C-unwind" fn open_settings(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
        if let Some(open) = SETTINGS_HANDLER.get() {
            open();
        }
    }

    // 找不到委托类（例如 `winit` 修改了类名）时不添加菜单项，⌘, 退回由键盘事件处理。
    let Some(delegate) = AnyClass::get(DELEGATE) else {
        return;
    };
    add_method(delegate, SETTINGS_ACTION, open_settings);

    // SAFETY: 用到的 `NSApplication`、`NSMenu`、`NSMenuItem`、`NSString` 是公开 API，只在主线程调用
    // （AppKit 的线程要求）。指针先判空再使用；取出的 `app`、`main_menu`、`app_item`、`app_menu` 由
    // AppKit 持有，调用内有效。标题由 `ns_string` 创建并持有到块末尾；`c","` 以 NUL 结尾，
    // `stringWithUTF8String:` 复制其内容。
    // 菜单项 `alloc` 后 +1，与 `separatorItem`（自动释放）加入后都由菜单持有，`release` 抵消。
    unsafe {
        let selector = sel!(wincOpenSettings:);
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let Some(app) = app.as_ref() else {
            return;
        };
        let target: *mut AnyObject = msg_send![app, delegate];
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

        let Some(title) = ns_string(winc_i18n::t!("menu-settings")) else {
            return;
        };
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c",".as_ptr()];
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let item: *mut AnyObject = msg_send![
            item,
            initWithTitle: &*title,
            action: selector,
            keyEquivalent: key
        ];
        let Some(item) = item.as_ref() else {
            return;
        };
        let () = msg_send![item, setTarget: target];

        // 「关于」是第 0 项，其后是分隔线。「设置…」插在分隔线之后（下标 2），并在其后再插一条
        // 分隔线，结果为：关于 | 分隔线 | 设置… | 分隔线 | `winit` 菜单原有的其余项。
        // 下标超出项数时 `AppKit` 会抛出异常，因此取项数与 2 中的较小值作为插入下标
        // （`winit` 的菜单项数不会少到该程度，此处只是兜底）。
        let count: isize = msg_send![app_menu, numberOfItems];
        let index = count.min(2);
        let separator: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
        let () = msg_send![app_menu, insertItem: item, atIndex: index];
        if let Some(separator) = separator.as_ref() {
            let () = msg_send![app_menu, insertItem: separator, atIndex: index + 1];
        }
        let () = msg_send![item, release];
    }
}

/// 菜单项「设置…」的动作名，添加在 `winit` 的应用委托上。
const SETTINGS_ACTION: &CStr = c"wincOpenSettings:";

/// 暂停期间去掉了快捷键的菜单项：菜单项、原来的按键和修饰键。
type Shortcuts = Vec<(Retained<AnyObject>, Retained<AnyObject>, usize)>;

thread_local! {
    /// 录制按键期间暂时去掉快捷键的菜单项；没有暂停时为空。只在主线程上使用。
    static SUSPENDED: std::cell::RefCell<Shortcuts> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 暂停（`suspend` 为 `true`）或恢复菜单栏中各菜单项的快捷键。
///
/// 菜单项的快捷键（如「设置…」的 ⌘,、「退出」的 ⌘Q）先于窗口收到按键，不会作为键盘事件送到
/// 窗口。录制按键时暂停这些快捷键，按键照常送到窗口；恢复时按原样放回。重复暂停或恢复不做任何
/// 事。只能在主线程调用。
pub fn suspend_menu_shortcuts(suspend: bool) {
    SUSPENDED.with_borrow_mut(|saved| {
        if !suspend {
            for (item, key, mask) in saved.drain(..) {
                // SAFETY: `item` 是暂停时持有引用的 `NSMenuItem`，`key` 是它原来的按键
                // （`NSString`），两者都仍然有效；在主线程调用公开的 setter。
                unsafe {
                    let () = msg_send![&*item, setKeyEquivalent: &*key];
                    let () = msg_send![&*item, setKeyEquivalentModifierMask: mask];
                }
            }
            return;
        }
        if !saved.is_empty() {
            return;
        }
        // SAFETY: `sharedApplication`、`mainMenu` 和 `+[NSString string]` 是公开 API，在主线程
        // 调用；返回值先判空再使用，菜单由 `NSApplication` 持有，空字符串是自动释放的常量。
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let Some(app) = app.as_ref() else {
                return;
            };
            let menu: *mut AnyObject = msg_send![app, mainMenu];
            let empty: *mut AnyObject = msg_send![class!(NSString), string];
            if let (Some(menu), Some(empty)) = (menu.as_ref(), empty.as_ref()) {
                clear_shortcuts(menu, empty, saved);
            }
        }
    });
}

/// 去掉 `menu` 及其各级子菜单中菜单项的快捷键（换成空字符串 `empty`），把菜单项、原来的按键
/// 和修饰键记入 `saved`。
///
/// # Safety
///
/// `menu` 必须是有效的 `NSMenu`，`empty` 必须是有效的 `NSString`，只能在主线程调用。
unsafe fn clear_shortcuts(menu: &AnyObject, empty: &AnyObject, saved: &mut Shortcuts) {
    // SAFETY: 由调用方保证 `menu` 有效、在主线程；用到的都是 `NSMenu`、`NSMenuItem` 的公开方法。
    // 取出的菜单项、子菜单和按键先判空再使用；保存的菜单项和按键由 `Retained` 各持有一个引用，
    // 菜单项换掉按键后原来的字符串仍然有效。
    unsafe {
        let count: isize = msg_send![menu, numberOfItems];
        for i in 0..count {
            let item: *mut AnyObject = msg_send![menu, itemAtIndex: i];
            let Some(item) = Retained::retain(item) else {
                continue;
            };
            let submenu: *mut AnyObject = msg_send![&*item, submenu];
            if let Some(submenu) = submenu.as_ref() {
                clear_shortcuts(submenu, empty, saved);
            }
            let key: *mut AnyObject = msg_send![&*item, keyEquivalent];
            let Some(key) = Retained::retain(key) else {
                continue;
            };
            let length: usize = msg_send![&*key, length];
            if length == 0 {
                continue;
            }
            let mask: usize = msg_send![&*item, keyEquivalentModifierMask];
            let () = msg_send![&*item, setKeyEquivalent: empty];
            saved.push((item, key, mask));
        }
    }
}

/// 菜单项动作或委托回调的函数类型，对应 Objective-C 的 `- (void)name:(id)arg`。
pub(super) type Action = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);

/// 给 `class` 添加一个 `- (void)name:(id)arg` 方法，用作菜单项的动作或 `menuNeedsUpdate:`
/// 这样的委托回调；返回是否添加成功，已有同名方法时不做任何事并返回 `false`。
pub(super) fn add_method(class: &AnyClass, name: &CStr, imp: Action) -> bool {
    // SAFETY: `Action` 与 `Imp` 都是函数指针，大小相同，`transmute` 成立。类型编码 `v@:@`
    // 与实际签名一致：返回 void（v），参数依次为 self（@）、_cmd（:）、sender（@）；AppKit
    // 按该编码调用，不会读错参数。
    unsafe {
        let imp = std::mem::transmute::<Action, Imp>(imp);
        add_raw_method(class, Sel::register(name), imp, c"v@:@")
    }
}

/// 返回 `NSApplication` 的委托（`winit` 的 `WinitApplicationDelegate`）；没有委托时返回 `None`。
///
/// 返回的 [`Retained`] 持有一个引用计数，调用方保存它时不依赖事件循环的生命周期。只能在主线程
/// 调用（AppKit 的线程要求）。
pub(super) fn app_delegate() -> Option<Retained<AnyObject>> {
    // SAFETY: `sharedApplication` 和 `delegate` 是公开 API，在主线程调用；两者的返回值在
    // 调用时有效或为空，返回的委托由 `NSApplication` 持有。`Retained::retain` 对空指针返回
    // `None`，对非空指针增加引用计数，之后由 `Retained` 负责释放。
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![app.as_ref()?, delegate];
        Retained::retain(delegate)
    }
}

/// 菜单项被选中时调用的回调，参数是该菜单项的 `tag`；只在主线程上设置和调用。
type MenuHandler = Box<dyn Fn(isize) + Send + Sync>;
static MENU_HANDLER: OnceLock<MenuHandler> = OnceLock::new();

/// 右键菜单项的动作名，添加在 `winit` 的应用委托上。
pub(super) const MENU_ACTION: &CStr = c"wincMenuCommand:";

/// 注册右键菜单的回调：选中一项时以该项的 `tag` 调用 `chosen`。
///
/// 必须在事件循环开始之后调用；只有第一次调用生效。
pub fn on_menu_command(chosen: impl Fn(isize) + Send + Sync + 'static) {
    if MENU_HANDLER.set(Box::new(chosen)).is_err() {
        return;
    }

    /// 实现 `- (void)wincMenuCommand:(NSMenuItem *)sender`：菜单项的动作，`tag` 是菜单项编号。
    extern "C-unwind" fn menu_command(_this: *mut AnyObject, _cmd: Sel, sender: *mut AnyObject) {
        // SAFETY: 发送者是本程序创建的 `NSMenuItem`，由 AppKit 在回调期间持有；`tag` 是其公开
        // 属性，不改变所有权。指针为空时不读取（`as_ref` 返回 `None`）。
        let tag: Option<isize> = unsafe { sender.as_ref().map(|item| msg_send![item, tag]) };
        if let (Some(tag), Some(chosen)) = (tag, MENU_HANDLER.get()) {
            chosen(tag);
        }
    }

    if let Some(delegate) = AnyClass::get(DELEGATE) {
        add_method(delegate, MENU_ACTION, menu_command);
    }
}

#[cfg(test)]
mod tests;
