//! 把文件拖出窗口，交给系统的拖放：放进其他程序时复制，拖回 Winc 自己的窗口时不放下。
//!
//! 窗口的内容视图兼作拖动来源：为其类添加 `NSDraggingSource` 的两个方法。

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::runtime::Bool;

use super::files::{file_url, ns_string};
use super::*;

/// `NSDragOperationCopy`。
const COPY: usize = 1;
/// `NSDraggingContextOutsideApplication`。
const OUTSIDE: isize = 0;
/// `NSEventTypeLeftMouseDown` 和 `NSEventTypeLeftMouseDragged`：开始拖放必须用这两种事件。
const MOUSE_EVENTS: [usize; 2] = [1, 6];
/// 拖动时图标的边长（点）。
const ICON: f64 = 32.0;

/// 拖出的文件是否尚未放下。
static DRAGGING: AtomicBool = AtomicBool::new(false);

/// 拖出的文件是否尚未放下：此时经过 Winc 的窗口不标出落点。
pub fn dragging_out() -> bool {
    DRAGGING.load(Ordering::Relaxed)
}

/// 把 `paths` 中的各项交给系统的拖放，立即返回；系统没有开始拖放时返回 `false`。
///
/// 必须在处理按住左键的移动事件时调用。
pub fn drag_files(window: &Window, paths: &[PathBuf]) -> bool {
    let Some(view) = ns_view(window) else {
        return false;
    };
    if !make_source(view) {
        return false;
    }

    // SAFETY: 在主线程调用；`currentEvent` 可能为空，判空并确认是左键事件后才建立各项、开始
    // 拖放。`items` 是元素均为 NSDraggingItem 的 NSArray，`view` 已实现拖动来源所需的方法。
    let session: *mut AnyObject = unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let event: *mut AnyObject = msg_send![app, currentEvent];
        let Some(event) = event.as_ref() else {
            return false;
        };
        let kind: usize = msg_send![event, type];
        if !MOUSE_EVENTS.contains(&kind) {
            return false;
        }
        let Some(items) = dragging_items(view, paths) else {
            return false;
        };
        msg_send![view, beginDraggingSessionWithItems: &*items, event: event, source: view]
    };
    let started = !session.is_null();
    DRAGGING.store(started, Ordering::Relaxed);
    started
}

/// 为 `view` 的类添加拖动来源所需的方法，返回 `view` 能否作为拖动来源。
fn make_source(view: &AnyObject) -> bool {
    type Mask = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, isize) -> usize;
    type Ended = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, CGPoint, usize);
    static SOURCE: OnceLock<bool> = OnceLock::new();
    // SAFETY: 在主线程调用；`Mask`、`Ended` 与两个方法的声明及类型编码一致，与 `Imp` 同为
    // `extern "C-unwind"` 函数指针，大小相同。`objc_getProtocol` 找不到协议时返回空指针，判空后
    // 再使用。
    *SOURCE.get_or_init(|| unsafe {
        let class = view.class();
        let mask = std::mem::transmute::<Mask, Imp>(operation_mask);
        let ended = std::mem::transmute::<Ended, Imp>(session_ended);
        add_raw_method(
            class,
            sel!(draggingSession:sourceOperationMaskForDraggingContext:),
            mask,
            c"Q@:@q",
        );
        add_raw_method(
            class,
            sel!(draggingSession:endedAtPoint:operation:),
            ended,
            c"v@:@{CGPoint=dd}Q",
        );
        let protocol = ffi::objc_getProtocol(c"NSDraggingSource".as_ptr());
        if !protocol.is_null() {
            ffi::class_addProtocol((class as *const AnyClass).cast_mut(), protocol);
        }
        let responds: Bool = msg_send![
            view,
            respondsToSelector: sel!(draggingSession:sourceOperationMaskForDraggingContext:)
        ];
        responds.as_bool()
    })
}

/// 返回 `paths` 中各项的 `NSDraggingItem` 组成的 `NSArray`，图标位于鼠标处；某一项无法表示时
/// 返回 `None`。
#[allow(clippy::disallowed_methods, reason = "只访问本机：拖出的都是本机文件")]
fn dragging_items(view: &AnyObject, paths: &[PathBuf]) -> Option<Retained<AnyObject>> {
    // SAFETY: 在主线程调用。`mouseLocationOutsideOfEventStream` 和 `convertPoint:fromView:` 只读，
    // 返回值类型的点；`alloc` 加 `initWithPasteboardWriter:` 返回 +1 的对象，交给 `Retained`；
    // `iconForFile:` 返回自动释放的图片，`setDraggingFrame:contents:` 会持有它。`arrayWithObjects:
    // count:` 复制指针并持有各元素，`objects` 在调用期间有效。
    unsafe {
        let window: *mut AnyObject = msg_send![view, window];
        let at: CGPoint = msg_send![window.as_ref()?, mouseLocationOutsideOfEventStream];
        let at: CGPoint =
            msg_send![view, convertPoint: at, fromView: std::ptr::null_mut::<AnyObject>()];
        let frame = CGRect::new(
            CGPoint::new(at.x - ICON / 2.0, at.y - ICON / 2.0),
            CGSize::new(ICON, ICON),
        );
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let mut items = Vec::with_capacity(paths.len());
        for path in paths {
            let url = file_url(path, path.is_dir())?;
            let name = ns_string(path.to_str()?)?;
            let icon: *mut AnyObject = msg_send![workspace, iconForFile: &*name];
            let item: *mut AnyObject = msg_send![class!(NSDraggingItem), alloc];
            let item: *mut AnyObject = msg_send![item, initWithPasteboardWriter: &*url];
            let item = Retained::from_raw(item)?;
            let _: () = msg_send![&*item, setDraggingFrame: frame, contents: icon];
            items.push(item);
        }
        let objects: Vec<*const AnyObject> = items.iter().map(Retained::as_ptr).collect();
        let array: *mut AnyObject = msg_send![
            class!(NSArray),
            arrayWithObjects: objects.as_ptr(),
            count: objects.len()
        ];
        Retained::retain(array)
    }
}

/// 实现 `- draggingSession:sourceOperationMaskForDraggingContext:`：只在拖到其他程序时复制。
extern "C-unwind" fn operation_mask(
    _: *mut AnyObject,
    _: Sel,
    _: *mut AnyObject,
    context: isize,
) -> usize {
    if context == OUTSIDE { COPY } else { 0 }
}

/// 实现 `- draggingSession:endedAtPoint:operation:`：拖出的文件已放下或取消。
extern "C-unwind" fn session_ended(
    _: *mut AnyObject,
    _: Sel,
    _: *mut AnyObject,
    _: CGPoint,
    _: usize,
) {
    DRAGGING.store(false, Ordering::Relaxed);
}
