//! `app_menu` 的测试：向一个公开的 `NSObject` 子类添加方法后只生效一次，并用类型编码调用
//! 验证参数与返回值；文件、目录和网址交给回调时只保留文件路径；应用菜单的项改用显示名。

use super::*;

#[test]
fn should_terminate_is_added_once_and_answers() {
    extern "C-unwind" fn cancel(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> usize {
        0
    }

    // 测试中没有 `winit` 的委托，改用不实现该方法的 `NSObject` 子类代替。
    let class = AnyClass::get(c"NSValueTransformer").unwrap();
    let selector = sel!(applicationShouldTerminate:);
    assert!(class.instance_method(selector).is_none());
    assert!(add_should_terminate(class, cancel));
    assert!(class.instance_method(selector).is_some());
    // 第二次不会替换已有的方法。
    assert!(!add_should_terminate(class, cancel));

    // SAFETY: `class` 的实例已有刚添加的方法，按其类型编码 `Q@:@` 调用；NSApplication
    // 参数传 nil，实现不会读取该参数；`new` 创建的对象（引用计数 +1）用完后 `release`。
    let reply: usize = unsafe {
        let object: *mut AnyObject = msg_send![class, new];
        let null = std::ptr::null_mut::<AnyObject>();
        let reply: usize = msg_send![object, applicationShouldTerminate: null];
        let () = msg_send![object, release];
        reply
    };
    assert_eq!(reply, 0);
}

#[test]
fn should_reopen_is_added_once_and_answers() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static HIDDEN: AtomicBool = AtomicBool::new(false);

    extern "C-unwind" fn record(_: *mut AnyObject, _: Sel, _: *mut AnyObject, v: Bool) -> Bool {
        HIDDEN.store(!v.as_bool(), Ordering::Relaxed);
        Bool::YES
    }

    // 测试中没有 `winit` 的委托：用公开的 `NSObject` 子类代替。
    let class = AnyClass::get(c"NSDateFormatter").unwrap();
    let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
    assert!(class.instance_method(selector).is_none());
    assert!(add_should_reopen(class, record));
    assert!(!add_should_reopen(class, record));

    // SAFETY: `class` 的实例已有刚添加的方法，按其类型编码调用；NSApplication 参数传 nil，
    // 实现不会读取该参数；`new` 创建的对象（引用计数 +1）用完后 `release`。
    let answer: Bool = unsafe {
        let object: *mut AnyObject = msg_send![class, new];
        let null = std::ptr::null_mut::<AnyObject>();
        let answer: Bool =
            msg_send![object, applicationShouldHandleReopen: null, hasVisibleWindows: Bool::NO];
        let () = msg_send![object, release];
        answer
    };
    assert!(answer.as_bool() && HIDDEN.load(Ordering::Relaxed));
}

#[test]
fn settings_action_is_added_once_and_runs() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    extern "C-unwind" fn count(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
        CALLS.fetch_add(1, Ordering::Relaxed);
    }

    // 测试中没有 `winit` 的委托：用公开的 `NSObject` 子类代替；换用另一个类，使两个测试互不
    // 影响。
    let class = AnyClass::get(c"NSFormatter").unwrap();
    let selector = Sel::register(SETTINGS_ACTION);
    assert!(class.instance_method(selector).is_none());
    assert!(add_method(class, SETTINGS_ACTION, count));
    assert!(!add_method(class, SETTINGS_ACTION, count));

    // SAFETY: `class` 的实例已有刚添加的方法，按其类型编码 `v@:@` 调用，sender 传 nil；
    // `new` 创建的对象（引用计数 +1）用完后 `release`。
    unsafe {
        let object: *mut AnyObject = msg_send![class, new];
        let null = std::ptr::null_mut::<AnyObject>();
        let () = msg_send![object, wincOpenSettings: null];
        let () = msg_send![object, release];
    }
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn opened_urls_reach_the_handler_as_paths() {
    use std::sync::Mutex;
    static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    // 测试中没有 `winit` 的委托：只登记回调，方法加在另一个公开的 `NSObject` 子类上。
    on_open_files(|paths| OPENED.lock().unwrap().extend(paths));
    let class = AnyClass::get(c"NSByteCountFormatter").unwrap();
    let selector = sel!(application:openURLs:);
    assert!(class.instance_method(selector).is_none());
    assert!(add_open_urls(class, open_urls));
    assert!(!add_open_urls(class, open_urls));

    // 一个文件、一个带空格的目录、一个不是文件的网址（跳过）。
    let file = super::super::files::file_url(Path::new("/tmp/a.txt"), false).unwrap();
    let dir = super::super::files::file_url(Path::new("/tmp/b c"), true).unwrap();
    // SAFETY: `class` 的实例已有刚添加的方法，按其类型编码 `v@:@@` 调用，NSApplication 传 nil；
    // 网址和数组由 `URLWithString:`、`arrayWithObjects:count:` 返回自动释放的对象，在本块内
    // 有效；`objects` 的指针和长度描述三个有效的对象。`new` 创建的对象用完后 `release`。
    unsafe {
        let web = super::super::files::ns_string("https://example.com").unwrap();
        let web: *mut AnyObject = msg_send![class!(NSURL), URLWithString: &*web];
        let objects: [*mut AnyObject; 3] = [
            Retained::as_ptr(&file).cast_mut(),
            Retained::as_ptr(&dir).cast_mut(),
            web,
        ];
        let urls: *mut AnyObject =
            msg_send![class!(NSArray), arrayWithObjects: objects.as_ptr(), count: 3usize];
        let object: *mut AnyObject = msg_send![class, new];
        let null = std::ptr::null_mut::<AnyObject>();
        let () = msg_send![object, application: null, openURLs: urls];
        let () = msg_send![object, release];
    }
    assert_eq!(
        *OPENED.lock().unwrap(),
        [PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b c")]
    );
}

#[test]
fn open_service_is_added_once_and_reads_only_file_urls() {
    // 测试中没有 `winit` 的委托：用公开的 `NSObject` 子类代替。
    let class = AnyClass::get(c"NSNumberFormatter").unwrap();
    let selector = sel!(openInWinc:userData:error:);
    assert!(class.instance_method(selector).is_none());
    assert!(add_open_service(class, open_service));
    assert!(!add_open_service(class, open_service));

    // 服务剪贴板中有一个文件、一个带空格的目录和一个网址；只读出前两个的路径。
    let file = super::super::files::file_url(Path::new("/tmp/a.txt"), false).unwrap();
    let dir = super::super::files::file_url(Path::new("/tmp/b c"), true).unwrap();
    // SAFETY: `pasteboardWithUniqueName` 返回自动释放的独立剪贴板，不影响系统剪贴板，用完后
    // `releaseGlobally` 释放。网址和数组由 `URLWithString:`、`arrayWithObjects:count:` 返回
    // 自动释放的对象，在本块内有效；`objects` 的指针和长度描述三个有效的对象。
    let paths = unsafe {
        let pasteboard: *mut AnyObject = msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
        let pasteboard = pasteboard.as_ref().unwrap();
        let web = super::super::files::ns_string("https://example.com").unwrap();
        let web: *mut AnyObject = msg_send![class!(NSURL), URLWithString: &*web];
        let objects: [*mut AnyObject; 3] = [
            Retained::as_ptr(&file).cast_mut(),
            Retained::as_ptr(&dir).cast_mut(),
            web,
        ];
        let urls: *mut AnyObject =
            msg_send![class!(NSArray), arrayWithObjects: objects.as_ptr(), count: 3usize];
        let _: isize = msg_send![pasteboard, clearContents];
        let written: Bool = msg_send![pasteboard, writeObjects: urls];
        assert!(written.as_bool());
        let paths = service_paths(pasteboard);
        let () = msg_send![pasteboard, releaseGlobally];
        paths
    };
    assert_eq!(
        paths,
        [PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b c")]
    );
}

#[test]
fn install_item_goes_right_below_settings() {
    use super::super::files::rust_string;
    // SAFETY: `NSMenu`、`NSMenuItem`、`NSObject` 是公开 API；测试中只有这一份菜单，不与其他线程
    // 共享。`new`、`alloc` 加 `init…` 创建的对象（+1）加入菜单后由菜单持有，`release` 抵消；
    // `target` 和菜单用完后 `release`。取出的菜单项和字符串由菜单持有，在本块内有效。
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), new];
        let menu = menu.as_ref().unwrap();
        let target: *mut AnyObject = msg_send![class!(NSObject), new];
        let target = target.as_ref().unwrap();
        let empty = super::super::files::ns_string("").unwrap();
        let add = |title: &str, action: Option<Sel>| {
            let title = super::super::files::ns_string(title).unwrap();
            let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
            let item: *mut AnyObject = msg_send![
                item,
                initWithTitle: &*title,
                action: action,
                keyEquivalent: &*empty
            ];
            let () = msg_send![item, setTarget: target];
            let () = msg_send![menu, addItem: item];
            let () = msg_send![item, release];
        };
        let separator = || {
            let item: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
            let () = msg_send![menu, addItem: item];
        };

        // 没有「设置…」时不插入。
        add("About Winc", None);
        assert!(!insert_install_item(menu, target));

        separator();
        add("Settings…", Some(Sel::register(SETTINGS_ACTION)));
        separator();
        add("Quit Winc", None);
        assert!(insert_install_item(menu, target));

        // 关于 | 分隔线 | 设置… | 安装 winc 命令… | 分隔线 | 退出
        let count: isize = msg_send![menu, numberOfItems];
        assert_eq!(count, 6);
        let item: *mut AnyObject = msg_send![menu, itemAtIndex: 3isize];
        let item = item.as_ref().unwrap();
        let action: Option<Sel> = msg_send![item, action];
        assert_eq!(action, Some(Sel::register(INSTALL_COMMAND_ACTION)));
        let title: *mut AnyObject = msg_send![item, title];
        assert_eq!(
            rust_string(title.as_ref().unwrap()).as_deref(),
            Some(winc_i18n::t!("menu-install-command"))
        );
        let next: *mut AnyObject = msg_send![menu, itemAtIndex: 4isize];
        let separated: Bool = msg_send![next, isSeparatorItem];
        assert!(separated.as_bool());

        let () = msg_send![target, release];
        let () = msg_send![menu, release];
    }
}

#[test]
fn menu_items_take_the_display_name() {
    // `winit` 按进程名写的项换成显示名。
    assert_eq!(
        renamed_title("About winc", "winc", "Winc").as_deref(),
        Some("About Winc")
    );
    assert_eq!(
        renamed_title("Quit winc", "winc", "Winc").as_deref(),
        Some("Quit Winc")
    );
    // 不以进程名结尾的项、只是以这几个字母结尾的词，以及显示名与进程名相同（不在应用包中运行）
    // 时都不改。
    assert_eq!(renamed_title("Hide Others", "winc", "Winc"), None);
    assert_eq!(renamed_title("Services", "winc", "Winc"), None);
    assert_eq!(renamed_title("Quit xwinc", "winc", "Winc"), None);
    assert_eq!(renamed_title("Quit winc", "winc", "winc"), None);
}
